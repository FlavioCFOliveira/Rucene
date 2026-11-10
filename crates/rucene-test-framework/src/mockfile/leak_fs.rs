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
use rucene::store::java_nio_file::{
    AsynchronousFileChannel, DirectoryStream, DirectoryStreamFilter, FileChannel, FileSystem,
    InputStream, OpenOption, OutputStream, Path, SeekableByteChannel,
};

use super::filter_file_system_provider::{install, FilterFileSystemProvider, FilterState};
use super::handle_tracking_fs::{
    self, install_tracking, Handle, HandleTrackingFS, HandleTrackingState,
};

/// The methods an anonymous subclass of `LeakFS` overrides; each defaults to
/// the class's body.
///
/// **Divergence.** Java subclasses `LeakFS` anonymously (as
/// `TestNIOFSDirectory.testHandleExceptionInConstructor` does, overriding
/// `newFileChannel`); Rust has no inheritance, so a subclass supplies these
/// overrides instead, and reaches `super` through `this`.
pub trait LeakFSOverrides: Send + Sync {
    /// `newFileChannel(path, options, attrs...)`; `super.newFileChannel` is
    /// `handle_tracking_fs::new_file_channel(this, path, options)`.
    fn new_file_channel(
        &self,
        this: &LeakFS,
        path: &Path,
        options: &[OpenOption],
    ) -> io::Result<Arc<dyn FileChannel>> {
        handle_tracking_fs::new_file_channel(this, path, options)
    }
}

/// The overrides of `LeakFS` itself: none.
pub struct NoLeakFSOverrides;

impl LeakFSOverrides for NoLeakFSOverrides {}

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
    /// The overrides of an anonymous subclass.
    overrides: Box<dyn LeakFSOverrides>,
}

impl LeakFS {
    /// `LeakFS(delegate)`: tracks file handle leaks for the delegate
    /// filesystem, with the scheme `leakfs://`.
    pub fn new(delegate: Arc<dyn FileSystem>) -> Arc<LeakFS> {
        Self::with_overrides(delegate, Box::new(NoLeakFSOverrides))
    }

    /// An anonymous subclass of `LeakFS(delegate)` with `overrides`.
    pub fn with_overrides(
        delegate: Arc<dyn FileSystem>,
        overrides: Box<dyn LeakFSOverrides>,
    ) -> Arc<LeakFS> {
        let this = Arc::new(LeakFS {
            filter: FilterState::new("leakfs://", &delegate),
            tracking: HandleTrackingState::new(),
            open_handles: Mutex::new(BTreeMap::new()),
            overrides,
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

    // The six provider methods `HandleTrackingFS` overrides, with
    // `newFileChannel` going through the subclass's overrides.
    fn new_input_stream(
        &self,
        path: &Path,
        options: &[OpenOption],
    ) -> io::Result<Box<dyn InputStream>> {
        handle_tracking_fs::new_input_stream(self, path, options)
    }

    fn new_output_stream(
        &self,
        path: &Path,
        options: &[OpenOption],
    ) -> io::Result<Box<dyn OutputStream>> {
        handle_tracking_fs::new_output_stream(self, path, options)
    }

    fn new_file_channel(
        &self,
        path: &Path,
        options: &[OpenOption],
    ) -> io::Result<Arc<dyn FileChannel>> {
        self.overrides.new_file_channel(self, path, options)
    }

    fn new_asynchronous_file_channel(
        &self,
        path: &Path,
        options: &[OpenOption],
    ) -> io::Result<Arc<dyn AsynchronousFileChannel>> {
        handle_tracking_fs::new_asynchronous_file_channel(self, path, options)
    }

    fn new_byte_channel(
        &self,
        path: &Path,
        options: &[OpenOption],
    ) -> io::Result<Box<dyn SeekableByteChannel>> {
        handle_tracking_fs::new_byte_channel(self, path, options)
    }

    fn new_directory_stream(
        &self,
        dir: &Path,
        filter: DirectoryStreamFilter,
    ) -> io::Result<Box<dyn DirectoryStream>> {
        handle_tracking_fs::new_directory_stream(self, dir, filter)
    }
}

crate::impl_file_system_provider!(LeakFS);
