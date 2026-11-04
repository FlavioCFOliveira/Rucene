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

//! Port of `org.apache.lucene.tests.mockfile.LeakFS`.

use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::exception::{Throwable, EXCEPTION, RUNTIME_EXCEPTION};
use rucene::store::java_nio_file::{FileSystem, Path};

use super::filter_file_system_provider::{install, FilterFileSystemProvider, FilterState};
use super::handle_tracking_fs::{install_tracking, Handle, HandleTrackingFS, HandleTrackingState};

/// Port of `org.apache.lucene.tests.mockfile.LeakFS`: a filesystem that
/// tracks open handles. When `FileSystem.close()` is called, it throws an
/// exception if any file handles are still open.
pub struct LeakFS {
    filter: FilterState,
    tracking: HandleTrackingState,
    /// `openHandles`: "we explicitly use reference hashcode/equality in our
    /// keys" — each handle with the `new Exception()` recorded when it opened.
    ///
    /// **Divergence.** Java's `ConcurrentHashMap` iterates in an order its
    /// identity hash codes decide, different on every run; this map iterates
    /// in opening order.
    open_handles: Mutex<BTreeMap<u64, (Handle, Throwable)>>,
}

impl LeakFS {
    /// `LeakFS(delegate)`: tracks file handle leaks for the delegate
    /// filesystem, with the scheme `leakfs://`.
    pub fn new(delegate: Arc<dyn FileSystem>) -> Arc<LeakFS> {
        let this = Arc::new(LeakFS {
            filter: FilterState::new("leakfs://", &delegate),
            tracking: HandleTrackingState::new(),
            open_handles: Mutex::new(BTreeMap::new()),
        });
        install(&this, delegate);
        install_tracking(&this);
        this
    }
}

impl HandleTrackingFS for LeakFS {
    fn tracking(&self) -> &HandleTrackingState {
        &self.tracking
    }

    /// `onOpen`: `openHandles.put(stream, new Exception())`.
    fn on_open(&self, _path: &Path, stream: &Handle) -> io::Result<()> {
        self.open_handles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                stream.id(),
                (stream.clone(), Throwable::with_message(EXCEPTION, None)),
            );
        Ok(())
    }

    /// `onClose(path, stream)`: `openHandles.remove(stream)`.
    fn on_close_handle(&self, _path: &Path, stream: &Handle) -> io::Result<()> {
        self.open_handles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&stream.id());
        Ok(())
    }
}

impl FilterFileSystemProvider for LeakFS {
    fn filter(&self) -> &FilterState {
        &self.filter
    }

    fn class_name(&self) -> &'static str {
        "org.apache.lucene.tests.mockfile.LeakFS"
    }

    /// `synchronized onClose()`: `RuntimeException("file handle leaks: " +
    /// openHandles.keySet(), cause)`, the cause being the first handle's
    /// `Exception` ("print the first one as it's very verbose otherwise").
    fn on_close(&self) {
        let handles = self
            .open_handles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !handles.is_empty() {
            let keys: Vec<String> = handles.values().map(|(h, _)| h.to_string()).collect();
            let cause = handles
                .values()
                .next()
                .map(|(_, e)| e.clone())
                .expect("non-empty");
            drop(handles);
            Throwable::with_cause(
                RUNTIME_EXCEPTION,
                Some(format!("file handle leaks: [{}]", keys.join(", "))),
                cause,
            )
            .throw()
        }
    }

    crate::handle_tracking_fs_methods!();
}

crate::impl_file_system_provider!(LeakFS);
