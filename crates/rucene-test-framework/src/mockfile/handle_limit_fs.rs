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

//! Port of `org.apache.lucene.tests.mockfile.HandleLimitFS`.

use std::io;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use rucene::store::java_nio_file::{file_system_exception, FileSystem, Path};

use super::filter_file_system_provider::{install, FilterFileSystemProvider, FilterState};
use super::handle_tracking_fs::{install_tracking, Handle, HandleTrackingFS, HandleTrackingState};

/// Port of the annotation `HandleLimitFS.MaxOpenHandles`: the maximum number
/// of open handles a suite allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaxOpenHandles {
    /// `limit()`.
    pub limit: i32,
}

impl MaxOpenHandles {
    /// `MAX_OPEN_FILES`: "TODO: can we make the default even lower?"
    pub const MAX_OPEN_FILES: i32 = 2048;
}

/// Port of `org.apache.lucene.tests.mockfile.HandleLimitFS`: a filesystem
/// that throws an exception if the file handles in use exceed a specified
/// limit.
pub struct HandleLimitFS {
    filter: FilterState,
    tracking: HandleTrackingState,
    /// `limit`.
    limit: i32,
    /// `count`.
    count: AtomicI32,
}

impl HandleLimitFS {
    /// `HandleLimitFS(delegate, limit)`, with the scheme `handlelimit://`.
    pub fn new(delegate: Arc<dyn FileSystem>, limit: i32) -> Arc<HandleLimitFS> {
        let this = Arc::new(HandleLimitFS {
            filter: FilterState::new("handlelimit://", &delegate),
            tracking: HandleTrackingState::new(),
            limit,
            count: AtomicI32::new(0),
        });
        install(&this, delegate);
        install_tracking(&this);
        this
    }

    /// `count`, the handles the filesystem believes open.
    pub fn count(&self) -> i32 {
        self.count.load(Ordering::SeqCst)
    }
}

impl HandleTrackingFS for HandleLimitFS {
    fn tracking(&self) -> &HandleTrackingState {
        &self.tracking
    }

    /// `onOpen`: `FileSystemException(path, null, "Too many open files")`
    /// beyond the limit.
    fn on_open(&self, path: &Path, _stream: &Handle) -> io::Result<()> {
        if self.count.fetch_add(1, Ordering::SeqCst).wrapping_add(1) > self.limit {
            self.count.fetch_sub(1, Ordering::SeqCst);
            return Err(file_system_exception(
                Some(path.to_string()),
                None,
                Some("Too many open files".to_string()),
            ));
        }
        Ok(())
    }

    /// `onClose(path, stream)`: `count.decrementAndGet()`.
    fn on_close_handle(&self, _path: &Path, _stream: &Handle) -> io::Result<()> {
        self.count.fetch_sub(1, Ordering::SeqCst);
        Ok(())
    }
}

impl FilterFileSystemProvider for HandleLimitFS {
    fn filter(&self) -> &FilterState {
        &self.filter
    }

    fn class_name(&self) -> &'static str {
        "org.apache.lucene.tests.mockfile.HandleLimitFS"
    }

    crate::handle_tracking_fs_methods!();
}

crate::impl_file_system_provider!(HandleLimitFS);
