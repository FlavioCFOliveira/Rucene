/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `org.apache.lucene.tests.mockfile.WindowsFS`.

use std::collections::HashMap;
use std::fmt;
use std::io;
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::exception::java_assert;
use rucene::store::java_nio_file::{
    io_exception, throw_unchecked, CopyOption, FileKey, FileSystem, Files, Path,
};

use super::filter_file_system_provider::{
    self as ffsp, install, FilterFileSystemProvider, FilterState,
};
use super::handle_tracking_fs::{install_tracking, Handle, HandleTrackingFS, HandleTrackingState};
use super::windows_path::WindowsPath;

/// The key `WindowsFS` tracks a file by: its file key (the inode), or the
/// path when the platform has no file key ("the key may be null, e.g. on real
/// Windows!").
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum OpenFileKey {
    /// `BasicFileAttributes.fileKey()`.
    FileKey(FileKey),
    /// The path itself.
    Path(Path),
}

impl fmt::Display for OpenFileKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OpenFileKey::FileKey(key) => write!(f, "{key}"),
            OpenFileKey::Path(path) => write!(f, "{path}"),
        }
    }
}

/// Port of `org.apache.lucene.tests.mockfile.WindowsFS`: a filesystem that
/// (imperfectly) acts like Windows. Currently it only prevents the deletion of
/// open files.
pub struct WindowsFS {
    filter: FilterState,
    tracking: HandleTrackingState,
    /// `openFiles`: "This map also supports fileKey -> Path -> counts which is
    /// important to effectively support renames etc." The mutex is the
    /// monitor Java synchronizes on.
    open_files: Mutex<HashMap<OpenFileKey, HashMap<Path, i32>>>,
}

impl WindowsFS {
    /// `WindowsFS(delegate)`, with the scheme `windows://`.
    pub fn new(delegate: Arc<dyn FileSystem>) -> Arc<WindowsFS> {
        let this = Arc::new(WindowsFS {
            filter: FilterState::new("windows://", &delegate),
            tracking: HandleTrackingState::new(),
            open_files: Mutex::new(HashMap::new()),
        });
        install(&this, delegate);
        install_tracking(&this);
        this
    }

    /// `openFiles.size()`: the number of open file keys. Java's field is
    /// package-private, read by the same-package `TestWindowsFS`; the port
    /// exposes its size for the crate's tests.
    pub fn open_files_size(&self) -> usize {
        self.lock().len()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<OpenFileKey, HashMap<Path, i32>>> {
        self.open_files
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// `getKey(existing)`: the file key (e.g. inode) of the path, through
    /// `Files.readAttributes`, or the path when there is none.
    fn get_key(existing: &Path) -> io::Result<OpenFileKey> {
        let attributes = Files::read_attributes(existing, &[])?;
        Ok(match attributes.file_key {
            Some(key) => OpenFileKey::FileKey(key),
            None => OpenFileKey::Path(existing.clone()),
        })
    }

    /// `getKeyOrNull(path)`: "we don't care if the file doesn't exist".
    fn get_key_or_null(path: &Path) -> Option<OpenFileKey> {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| Self::get_key(path).ok()))
            .ok()
            .flatten()
    }

    /// `checkDeleteAccess(path)`, under the monitor: `IOException("access
    /// denied: " + path)` while the file is open.
    fn check_delete_access(
        open_files: &HashMap<OpenFileKey, HashMap<Path, i32>>,
        path: &Path,
    ) -> io::Result<()> {
        if let Some(key) = Self::get_key_or_null(path) {
            if open_files.contains_key(&key) {
                return Err(io_exception(format!("access denied: {path}")));
            }
        }
        Ok(())
    }
}

impl HandleTrackingFS for WindowsFS {
    fn tracking(&self) -> &HandleTrackingState {
        &self.tracking
    }

    /// `onOpen`: "we have to read the key under the lock otherwise me might
    /// leak the openFile handle if we concurrently delete or move this file."
    fn on_open(&self, path: &Path, _stream: &Handle) -> io::Result<()> {
        let mut open_files = self.lock();
        let key = Self::get_key(path)?;
        let path_map = open_files.entry(key).or_default();
        *path_map.entry(path.clone()).or_insert(0) += 1;
        Ok(())
    }

    /// `onClose(path, stream)`: "we have to read the key under the lock;
    /// otherwise concurrent move might change the inode of the path".
    fn on_close_handle(&self, path: &Path, _stream: &Handle) -> io::Result<()> {
        let mut open_files = self.lock();
        let key = Self::get_key(path)?;
        let present = open_files.contains_key(&key);
        java_assert(present, || {
            format!("no open file for key {key} on path {path}")
        });
        let Some(path_map) = open_files.get_mut(&key) else {
            throw_unchecked("java.lang.NullPointerException", None)
        };
        java_assert(path_map.contains_key(path), String::new);
        if let Some(v) = path_map.get(path).copied() {
            if v == 1 {
                path_map.remove(path);
            } else {
                path_map.insert(path.clone(), v - 1);
            }
        }
        if path_map.is_empty() {
            open_files.remove(&key);
        }
        Ok(())
    }
}

impl FilterFileSystemProvider for WindowsFS {
    fn filter(&self) -> &FilterState {
        &self.filter
    }

    fn class_name(&self) -> &'static str {
        "org.apache.lucene.tests.mockfile.WindowsFS"
    }

    /// `wrapPath(path)`: a `WindowsPath`.
    fn wrap_path(&self, path: &Path) -> Path {
        super::FilterPath::new(path, self.filter.require_file_system())
    }

    /// `WindowsPath.resolve(Path)`: `checkInvalidPath(other)`, then
    /// `wrap(delegate.resolve(toDelegate(other)))`.
    fn resolve(&self, base: &Path, other: &Path) -> Path {
        WindowsPath::check_invalid_path(other);
        ffsp::resolve(self, base, other)
    }

    /// `delete(path)`, under the monitor: refused while the file is open.
    fn delete(&self, path: &Path) -> io::Result<()> {
        let open_files = self.lock();
        Self::check_delete_access(&open_files, path)?;
        ffsp::delete(self, path)
    }

    /// `move(source, target, options...)`, under the monitor: refused while
    /// the source is open; "we need to transfer ownership here if we have
    /// open files on this file since the getKey() method will return a
    /// different i-node next time we call it with the target path".
    fn r#move(&self, source: &Path, target: &Path, options: &[CopyOption]) -> io::Result<()> {
        let mut open_files = self.lock();
        Self::check_delete_access(&open_files, source)?;
        let key = Self::get_key_or_null(target);
        ffsp::r#move(self, source, target, options)?;
        if let Some(key) = key {
            let new_key = Self::get_key(target)?;
            if new_key != key {
                if let Some(map) = open_files.get_mut(&key) {
                    let moved = map.remove(target);
                    let empty = map.is_empty();
                    if let Some(v) = moved {
                        let path_integer_map = open_files.entry(new_key).or_default();
                        let existing_value = path_integer_map.get(target).copied().unwrap_or(0);
                        path_integer_map.insert(target.clone(), existing_value + v);
                    }
                    if empty {
                        open_files.remove(&key);
                    }
                }
            }
        }
        Ok(())
    }

    /// `deleteIfExists(path)`, under the monitor: refused while the file is
    /// open.
    fn delete_if_exists(&self, path: &Path) -> io::Result<bool> {
        let open_files = self.lock();
        Self::check_delete_access(&open_files, path)?;
        ffsp::delete_if_exists(self, path)
    }

    crate::handle_tracking_fs_methods!();
}

crate::impl_file_system_provider!(WindowsFS);
