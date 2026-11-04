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

//! Port of `org.apache.lucene.tests.mockfile.VirusCheckingFS`.

use std::io;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::system;
use rucene::index::WRITE_LOCK_NAME;
use rucene::store::java_nio_file::{access_denied_exception, FileSystem, Files, Path};

use super::filter_file_system_provider::{
    self as ffsp, install, FilterFileSystemProvider, FilterState,
};
use crate::util::lucene_test_case::VERBOSE;

/// Port of `org.apache.lucene.tests.mockfile.VirusCheckingFS`: acts like a
/// virus checker on Windows, where random programs may open the files you
/// just wrote in an unfriendly way preventing deletion (e.g. not passing
/// `FILE_SHARE_DELETE`) or renaming or overwriting etc. This is more evil than
/// `WindowsFS`, which just prevents deletion of files you still hold open.
pub struct VirusCheckingFS {
    filter: FilterState,
    /// `enabled`.
    enabled: AtomicBool,
    /// `state`.
    state: AtomicI64,
}

impl VirusCheckingFS {
    /// `VirusCheckingFS(delegate, salt)`, with the scheme `viruschecking://`.
    pub fn new(delegate: Arc<dyn FileSystem>, salt: i64) -> Arc<VirusCheckingFS> {
        let this = Arc::new(VirusCheckingFS {
            filter: FilterState::new("viruschecking://", &delegate),
            enabled: AtomicBool::new(true),
            state: AtomicI64::new(salt),
        });
        install(&this, delegate);
        this
    }

    /// `enable()`.
    pub fn enable(&self) {
        self.enabled.store(true, Ordering::SeqCst);
    }

    /// `isEnabled()`.
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    /// `disable()`.
    pub fn disable(&self) {
        self.enabled.store(false, Ordering::SeqCst);
    }
}

impl FilterFileSystemProvider for VirusCheckingFS {
    fn filter(&self) -> &FilterState {
        &self.filter
    }

    fn class_name(&self) -> &'static str {
        "org.apache.lucene.tests.mockfile.VirusCheckingFS"
    }

    /// `delete(path)`: "Fake but deterministic and hopefully portable
    /// like-randomness" — `state.incrementAndGet() *
    /// path.getFileName().hashCode()`; when enabled, the file exists ("important
    /// that we NOT delay a NoSuchFileException until later"), it is not the
    /// lock file ("life is particularly difficult if the virus checker hits
    /// our lock file") and `hash % 5 == 1`, the delete is refused with
    /// `AccessDeniedException`.
    fn delete(&self, path: &Path) -> io::Result<()> {
        let name = path.get_file_name();
        let hash_code = name.as_ref().map_or(0, Path::hash_code);
        let hash = (self.state.fetch_add(1, Ordering::SeqCst).wrapping_add(1))
            .wrapping_mul(i64::from(hash_code));
        if self.enabled.load(Ordering::SeqCst)
            && Files::exists(path, &[])
            && name.map(|n| n.to_string()).as_deref() != Some(WRITE_LOCK_NAME)
            && (hash % 5) == 1
        {
            if *VERBOSE {
                system::out_println(&format!(
                    "NOTE: VirusCheckingFS now refusing to delete {path}"
                ));
            }
            return Err(access_denied_exception(format!(
                "VirusCheckingFS is randomly refusing to delete file \"{path}\""
            )));
        }
        ffsp::delete(self, path)
    }
}

crate::impl_file_system_provider!(VirusCheckingFS);
