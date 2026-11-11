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

//! Port of `org.apache.lucene.tests.mockfile.HandleTrackingFS`.
//!
//! **Divergence.** Java's abstract `HandleTrackingFS` overrides six provider
//! methods and declares two hooks its subclasses implement. Rust has no
//! inheritance: the hooks are the trait [`HandleTrackingFS`], the six
//! overrides are the free functions of this module, and
//! [`handle_tracking_fs_methods!`](crate::handle_tracking_fs_methods) puts
//! them in a subclass's [`FilterFileSystemProvider`] implementation. Java
//! identifies a handle by the stream object itself (identity `hashCode` and
//! `equals`); here a [`Handle`] carries a unique identity and the same
//! `toString()`.

use std::fmt;
use std::io;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, Weak};

use rucene::store::java_nio_file::{
    AsynchronousFileChannel, DirectoryStream, DirectoryStreamFilter, FileChannel, InputStream,
    OpenOption, OutputStream, Path, SeekableByteChannel,
};

use super::filter_asynchronous_file_channel::{
    AsynchronousFileChannelOverrides, FilterAsynchronousFileChannel,
};
use super::filter_directory_stream::{DirectoryStreamOverrides, FilterDirectoryStream};
use super::filter_file_channel::{FileChannelOverrides, FilterFileChannel};
use super::filter_file_system_provider::{self as ffsp, FilterFileSystemProvider};
use super::filter_input_stream2::{FilterInputStream2, InputStreamOverrides};
use super::filter_output_stream2::{FilterOutputStream2, OutputStreamOverrides};
use super::filter_seekable_byte_channel::{
    FilterSeekableByteChannel, SeekableByteChannelOverrides,
};

static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);

/// A stream or channel opened against a path, as the hooks see it: its
/// identity and its `toString()`.
#[derive(Debug, Clone)]
pub struct Handle {
    id: u64,
    description: String,
}

impl Handle {
    fn new(kind: &str, path: &Path) -> Handle {
        Handle {
            id: NEXT_HANDLE.fetch_add(1, Ordering::Relaxed),
            description: format!("{kind}({path})"),
        }
    }

    /// The identity of the handle (Java's object identity).
    pub fn id(&self) -> u64 {
        self.id
    }
}

impl fmt::Display for Handle {
    /// `toString()`: `InputStream(path)`, `OutputStream(path)`,
    /// `FileChannel(path)`, `AsynchronousFileChannel(path)`,
    /// `SeekableByteChannel(path)` or `DirectoryStream(dir)`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.description)
    }
}

impl PartialEq for Handle {
    /// `equals(obj)`: `this == obj`.
    fn eq(&self, other: &Handle) -> bool {
        self.id == other.id
    }
}

impl Eq for Handle {}

/// The state a [`HandleTrackingFS`] keeps: a reference to itself, so that
/// the handles it hands out can reach its hooks when they close.
#[derive(Default)]
pub struct HandleTrackingState {
    me: OnceLock<Weak<dyn HandleTrackingFS>>,
}

impl HandleTrackingState {
    /// A new state.
    pub fn new() -> HandleTrackingState {
        HandleTrackingState::default()
    }

    fn owner(&self) -> Arc<dyn HandleTrackingFS> {
        self.me
            .get()
            .and_then(Weak::upgrade)
            .expect("INVARIANT: install_tracking ran in the constructor")
    }
}

/// Registers `this` with its own tracking state; every constructor of a
/// [`HandleTrackingFS`] calls it.
pub fn install_tracking<T: HandleTrackingFS + 'static>(this: &Arc<T>) {
    let me: Arc<dyn HandleTrackingFS> = Arc::clone(this) as Arc<dyn HandleTrackingFS>;
    let _ = this.tracking().me.set(Arc::downgrade(&me));
}

/// Port of `org.apache.lucene.tests.mockfile.HandleTrackingFS`: the base
/// class for tracking file handles. It adds tracking to all streams and
/// channels and provides two hooks to handle file management: `onOpen(Path,
/// Object)` and `onClose(Path, Object)`.
pub trait HandleTrackingFS: FilterFileSystemProvider {
    /// The tracking state.
    fn tracking(&self) -> &HandleTrackingState;

    /// `onOpen(path, stream)`: called when `path` is opened via `stream`.
    ///
    /// # Errors
    ///
    /// An `IOException` that refuses the open; the stream is then closed.
    fn on_open(&self, path: &Path, stream: &Handle) -> io::Result<()>;

    /// `onClose(path, stream)`: called when `path` is closed via `stream`
    /// (Java's overload of `onClose`, named apart from the filesystem's
    /// `onClose()`).
    ///
    /// # Errors
    ///
    /// An `IOException` the close reports.
    fn on_close_handle(&self, path: &Path, stream: &Handle) -> io::Result<()>;
}

/// `callOpenHook(path, stream)`: `onOpen`, and on any failure
/// `IOUtils.closeWhileHandlingException(stream)` before the failure
/// propagates.
fn call_open_hook(
    owner: &Arc<dyn HandleTrackingFS>,
    path: &Path,
    handle: &Handle,
    close: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    match catch_unwind(AssertUnwindSafe(|| owner.on_open(path, handle))) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => {
            // `closeWhileHandlingException`: an `IOException` is swallowed;
            // an `Error` (a panic here) propagates and replaces the failure.
            let _ = close();
            Err(error)
        }
        Err(payload) => {
            let _ = close();
            resume_unwind(payload)
        }
    }
}

/// `try { if (!closed) { closed = true; onClose(path, this); } } finally {
/// super.close(); }`: the hook runs on the first close, the delegate is
/// closed every time, and a failure to close the delegate replaces the
/// hook's.
fn close_tracked(
    owner: &Arc<dyn HandleTrackingFS>,
    path: &Path,
    handle: &Handle,
    closed: &AtomicBool,
    super_close: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    let hook = catch_unwind(AssertUnwindSafe(|| {
        if !closed.swap(true, Ordering::SeqCst) {
            owner.on_close_handle(path, handle)
        } else {
            Ok(())
        }
    }));
    let closing = super_close();
    match (hook, closing) {
        (_, Err(error)) => Err(error),
        (Err(payload), Ok(())) => resume_unwind(payload),
        (Ok(result), Ok(())) => result,
    }
}

struct TrackedInputStream {
    owner: Arc<dyn HandleTrackingFS>,
    path: Path,
    handle: Handle,
    closed: AtomicBool,
}

impl InputStreamOverrides for TrackedInputStream {
    fn close(&mut self, delegate: &mut dyn InputStream) -> io::Result<()> {
        close_tracked(&self.owner, &self.path, &self.handle, &self.closed, || {
            delegate.close()
        })
    }

    fn to_java_string(&self) -> Option<String> {
        Some(self.handle.to_string())
    }
}

/// `HandleTrackingFS.newInputStream(path, options...)`.
///
/// # Errors
///
/// The failures of the delegate and of `onOpen`.
pub fn new_input_stream<P: HandleTrackingFS + ?Sized>(
    this: &P,
    path: &Path,
    options: &[OpenOption],
) -> io::Result<Box<dyn InputStream>> {
    let owner = this.tracking().owner();
    let handle = Handle::new("InputStream", path);
    let mut stream = FilterInputStream2::with_overrides(
        ffsp::new_input_stream(this, path, options)?,
        Box::new(TrackedInputStream {
            owner: Arc::clone(&owner),
            path: path.clone(),
            handle: handle.clone(),
            closed: AtomicBool::new(false),
        }),
        "org.apache.lucene.tests.mockfile.HandleTrackingFS$1",
    );
    call_open_hook(&owner, path, &handle, || stream.close())?;
    Ok(Box::new(stream))
}

struct TrackedOutputStream {
    owner: Arc<dyn HandleTrackingFS>,
    path: Path,
    handle: Handle,
    closed: AtomicBool,
}

impl OutputStreamOverrides for TrackedOutputStream {
    fn close(&mut self, delegate: &mut dyn OutputStream) -> io::Result<()> {
        close_tracked(&self.owner, &self.path, &self.handle, &self.closed, || {
            delegate.close()
        })
    }

    fn to_java_string(&self) -> Option<String> {
        Some(self.handle.to_string())
    }
}

/// `HandleTrackingFS.newOutputStream(path, options...)`.
///
/// # Errors
///
/// The failures of the delegate and of `onOpen`.
pub fn new_output_stream<P: HandleTrackingFS + ?Sized>(
    this: &P,
    path: &Path,
    options: &[OpenOption],
) -> io::Result<Box<dyn OutputStream>> {
    let owner = this.tracking().owner();
    let f = this.filter();
    let delegate = f
        .delegate()
        .new_output_stream(&f.to_delegate(path), options)?;
    let handle = Handle::new("OutputStream", path);
    let mut stream = FilterOutputStream2::with_overrides(
        delegate,
        Box::new(TrackedOutputStream {
            owner: Arc::clone(&owner),
            path: path.clone(),
            handle: handle.clone(),
            closed: AtomicBool::new(false),
        }),
        "org.apache.lucene.tests.mockfile.HandleTrackingFS$2",
    );
    call_open_hook(&owner, path, &handle, || stream.close())?;
    Ok(Box::new(stream))
}

struct TrackedFileChannel {
    owner: Arc<dyn HandleTrackingFS>,
    path: Path,
    handle: Handle,
    closed: AtomicBool,
}

impl FileChannelOverrides for TrackedFileChannel {
    /// `implCloseChannel()`: `if (!closed) { closed = true; try {
    /// onClose(path, this); } finally { super.implCloseChannel(); } }`.
    fn impl_close_channel(&self, delegate: &dyn FileChannel) -> io::Result<()> {
        if self.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let hook = catch_unwind(AssertUnwindSafe(|| {
            self.owner.on_close_handle(&self.path, &self.handle)
        }));
        let closing = delegate.close();
        match (hook, closing) {
            (_, Err(error)) => Err(error),
            (Err(payload), Ok(())) => resume_unwind(payload),
            (Ok(result), Ok(())) => result,
        }
    }

    fn to_java_string(&self) -> Option<String> {
        Some(self.handle.to_string())
    }
}

/// `HandleTrackingFS.newFileChannel(path, options, attrs...)`.
///
/// # Errors
///
/// The failures of the delegate and of `onOpen`.
pub fn new_file_channel<P: HandleTrackingFS + ?Sized>(
    this: &P,
    path: &Path,
    options: &[OpenOption],
) -> io::Result<Arc<dyn FileChannel>> {
    let owner = this.tracking().owner();
    let f = this.filter();
    let delegate = f
        .delegate()
        .new_file_channel(&f.to_delegate(path), options)?;
    let handle = Handle::new("FileChannel", path);
    let channel: Arc<dyn FileChannel> = Arc::new(FilterFileChannel::with_overrides(
        delegate,
        Box::new(TrackedFileChannel {
            owner: Arc::clone(&owner),
            path: path.clone(),
            handle: handle.clone(),
            closed: AtomicBool::new(false),
        }),
        "org.apache.lucene.tests.mockfile.HandleTrackingFS$3",
    ));
    call_open_hook(&owner, path, &handle, || channel.close())?;
    Ok(channel)
}

struct TrackedAsynchronousFileChannel {
    owner: Arc<dyn HandleTrackingFS>,
    path: Path,
    handle: Handle,
    closed: AtomicBool,
}

impl AsynchronousFileChannelOverrides for TrackedAsynchronousFileChannel {
    fn close(&self, delegate: &dyn AsynchronousFileChannel) -> io::Result<()> {
        close_tracked(&self.owner, &self.path, &self.handle, &self.closed, || {
            delegate.close()
        })
    }

    fn to_java_string(&self) -> Option<String> {
        Some(self.handle.to_string())
    }
}

/// `HandleTrackingFS.newAsynchronousFileChannel(path, options, executor,
/// attrs...)`.
///
/// # Errors
///
/// The failures of the delegate and of `onOpen`.
pub fn new_asynchronous_file_channel<P: HandleTrackingFS + ?Sized>(
    this: &P,
    path: &Path,
    options: &[OpenOption],
) -> io::Result<Arc<dyn AsynchronousFileChannel>> {
    let owner = this.tracking().owner();
    let handle = Handle::new("AsynchronousFileChannel", path);
    let channel: Arc<dyn AsynchronousFileChannel> =
        Arc::new(FilterAsynchronousFileChannel::with_overrides(
            ffsp::new_asynchronous_file_channel(this, path, options)?,
            Box::new(TrackedAsynchronousFileChannel {
                owner: Arc::clone(&owner),
                path: path.clone(),
                handle: handle.clone(),
                closed: AtomicBool::new(false),
            }),
            "org.apache.lucene.tests.mockfile.HandleTrackingFS$4",
        ));
    call_open_hook(&owner, path, &handle, || channel.close())?;
    Ok(channel)
}

struct TrackedSeekableByteChannel {
    owner: Arc<dyn HandleTrackingFS>,
    path: Path,
    handle: Handle,
    closed: AtomicBool,
}

impl SeekableByteChannelOverrides for TrackedSeekableByteChannel {
    fn close(&self, delegate: &dyn SeekableByteChannel) -> io::Result<()> {
        close_tracked(&self.owner, &self.path, &self.handle, &self.closed, || {
            delegate.close()
        })
    }

    fn to_java_string(&self) -> Option<String> {
        Some(self.handle.to_string())
    }
}

/// `HandleTrackingFS.newByteChannel(path, options, attrs...)`.
///
/// # Errors
///
/// The failures of the delegate and of `onOpen`.
pub fn new_byte_channel<P: HandleTrackingFS + ?Sized>(
    this: &P,
    path: &Path,
    options: &[OpenOption],
) -> io::Result<Box<dyn SeekableByteChannel>> {
    let owner = this.tracking().owner();
    let handle = Handle::new("SeekableByteChannel", path);
    let channel = FilterSeekableByteChannel::with_overrides(
        ffsp::new_byte_channel(this, path, options)?,
        Box::new(TrackedSeekableByteChannel {
            owner: Arc::clone(&owner),
            path: path.clone(),
            handle: handle.clone(),
            closed: AtomicBool::new(false),
        }),
        "org.apache.lucene.tests.mockfile.HandleTrackingFS$5",
    );
    call_open_hook(&owner, path, &handle, || channel.close())?;
    Ok(Box::new(channel))
}

struct TrackedDirectoryStream {
    owner: Arc<dyn HandleTrackingFS>,
    dir: Path,
    handle: Handle,
    closed: AtomicBool,
}

impl DirectoryStreamOverrides for TrackedDirectoryStream {
    fn close(&mut self, delegate: &mut dyn DirectoryStream) -> io::Result<()> {
        close_tracked(&self.owner, &self.dir, &self.handle, &self.closed, || {
            delegate.close()
        })
    }

    fn to_java_string(&self) -> Option<String> {
        Some(self.handle.to_string())
    }
}

/// `HandleTrackingFS.newDirectoryStream(dir, filter)`.
///
/// # Errors
///
/// The failures of the delegate and of `onOpen`.
pub fn new_directory_stream<P: HandleTrackingFS + ?Sized>(
    this: &P,
    dir: &Path,
    filter: DirectoryStreamFilter,
) -> io::Result<Box<dyn DirectoryStream>> {
    let owner = this.tracking().owner();
    let f = this.filter();
    let wrapped = ffsp::wrapped_filter(ffsp::wrapper_of(this), filter);
    let stream = f
        .delegate()
        .new_directory_stream(&f.to_delegate(dir), wrapped)?;
    let handle = Handle::new("DirectoryStream", dir);
    let mut stream = FilterDirectoryStream::with_overrides(
        stream,
        Arc::clone(f.require_file_system()),
        Box::new(TrackedDirectoryStream {
            owner: Arc::clone(&owner),
            dir: dir.clone(),
            handle: handle.clone(),
            closed: AtomicBool::new(false),
        }),
        "org.apache.lucene.tests.mockfile.HandleTrackingFS$6",
    );
    call_open_hook(&owner, dir, &handle, || stream.close())?;
    Ok(Box::new(stream))
}

/// The six provider methods `HandleTrackingFS` overrides, for a subclass's
/// [`FilterFileSystemProvider`] implementation.
#[macro_export]
macro_rules! handle_tracking_fs_methods {
    () => {
        fn new_input_stream(
            &self,
            path: &::rucene::store::java_nio_file::Path,
            options: &[::rucene::store::java_nio_file::OpenOption],
        ) -> ::std::io::Result<Box<dyn ::rucene::store::java_nio_file::InputStream>> {
            $crate::mockfile::handle_tracking_fs::new_input_stream(self, path, options)
        }
        fn new_output_stream(
            &self,
            path: &::rucene::store::java_nio_file::Path,
            options: &[::rucene::store::java_nio_file::OpenOption],
        ) -> ::std::io::Result<Box<dyn ::rucene::store::java_nio_file::OutputStream>> {
            $crate::mockfile::handle_tracking_fs::new_output_stream(self, path, options)
        }
        fn new_file_channel(
            &self,
            path: &::rucene::store::java_nio_file::Path,
            options: &[::rucene::store::java_nio_file::OpenOption],
        ) -> ::std::io::Result<::std::sync::Arc<dyn ::rucene::store::java_nio_file::FileChannel>> {
            $crate::mockfile::handle_tracking_fs::new_file_channel(self, path, options)
        }
        fn new_asynchronous_file_channel(
            &self,
            path: &::rucene::store::java_nio_file::Path,
            options: &[::rucene::store::java_nio_file::OpenOption],
        ) -> ::std::io::Result<
            ::std::sync::Arc<dyn ::rucene::store::java_nio_file::AsynchronousFileChannel>,
        > {
            $crate::mockfile::handle_tracking_fs::new_asynchronous_file_channel(self, path, options)
        }
        fn new_byte_channel(
            &self,
            path: &::rucene::store::java_nio_file::Path,
            options: &[::rucene::store::java_nio_file::OpenOption],
        ) -> ::std::io::Result<Box<dyn ::rucene::store::java_nio_file::SeekableByteChannel>> {
            $crate::mockfile::handle_tracking_fs::new_byte_channel(self, path, options)
        }
        fn new_directory_stream(
            &self,
            dir: &::rucene::store::java_nio_file::Path,
            filter: ::rucene::store::java_nio_file::DirectoryStreamFilter,
        ) -> ::std::io::Result<Box<dyn ::rucene::store::java_nio_file::DirectoryStream>> {
            $crate::mockfile::handle_tracking_fs::new_directory_stream(self, dir, filter)
        }
    };
}
