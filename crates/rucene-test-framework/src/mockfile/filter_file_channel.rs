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

//! Port of `org.apache.lucene.tests.mockfile.FilterFileChannel`.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use rucene::store::java_nio_file::{FileChannel, FileLock, SeekableByteChannel};

use super::identity_string;

/// The methods an anonymous subclass of `FilterFileChannel` overrides; each
/// defaults to the class's body, which delegates.
///
/// **Divergence.** Java subclasses `FilterFileChannel` anonymously; Rust has
/// no inheritance, so a subclass supplies these overrides instead.
pub trait FileChannelOverrides: Send + Sync {
    /// `force(metaData)`.
    fn force(&self, delegate: &dyn FileChannel, meta_data: bool) -> io::Result<()> {
        delegate.force(meta_data)
    }

    /// `implCloseChannel()`: "we can't call implCloseChannel, but calling this
    /// instead is \"ok\"": `delegate.close()`.
    fn impl_close_channel(&self, delegate: &dyn FileChannel) -> io::Result<()> {
        delegate.close()
    }

    /// `toString()`, `None` for `Object.toString()`.
    fn to_java_string(&self) -> Option<String> {
        None
    }
}

/// The overrides of `FilterFileChannel` itself: none.
pub struct NoFileChannelOverrides;

impl FileChannelOverrides for NoFileChannelOverrides {}

/// Port of `org.apache.lucene.tests.mockfile.FilterFileChannel`: a
/// `FileChannel` that contains another, which it uses as its basic source of
/// data, possibly transforming the data along the way or providing additional
/// functionality.
pub struct FilterFileChannel {
    /// `delegate`: the underlying `FileChannel` instance.
    delegate: Arc<dyn FileChannel>,
    /// `AbstractInterruptibleChannel`'s `open` flag: `close()` runs
    /// `implCloseChannel()` once.
    open: AtomicBool,
    overrides: Box<dyn FileChannelOverrides>,
    class_name: &'static str,
}

impl FilterFileChannel {
    /// `FilterFileChannel(delegate)`.
    pub fn new(delegate: Arc<dyn FileChannel>) -> FilterFileChannel {
        Self::with_overrides(
            delegate,
            Box::new(NoFileChannelOverrides),
            "org.apache.lucene.tests.mockfile.FilterFileChannel",
        )
    }

    /// A subclass of `FilterFileChannel` named `class_name` with `overrides`.
    pub fn with_overrides(
        delegate: Arc<dyn FileChannel>,
        overrides: Box<dyn FileChannelOverrides>,
        class_name: &'static str,
    ) -> FilterFileChannel {
        FilterFileChannel {
            delegate,
            open: AtomicBool::new(true),
            overrides,
            class_name,
        }
    }

    /// `delegate`.
    pub fn delegate(&self) -> &Arc<dyn FileChannel> {
        &self.delegate
    }
}

impl SeekableByteChannel for FilterFileChannel {
    fn read(&self, dst: &mut [u8]) -> io::Result<i32> {
        self.delegate.read(dst)
    }

    fn write(&self, src: &[u8]) -> io::Result<i32> {
        self.delegate.write(src)
    }

    fn position(&self) -> io::Result<i64> {
        self.delegate.position()
    }

    fn set_position(&self, new_position: i64) -> io::Result<()> {
        self.delegate.set_position(new_position)
    }

    fn size(&self) -> io::Result<i64> {
        self.delegate.size()
    }

    fn truncate(&self, size: i64) -> io::Result<()> {
        self.delegate.truncate(size)
    }

    fn is_open(&self) -> bool {
        self.open.load(Ordering::SeqCst)
    }

    fn close(&self) -> io::Result<()> {
        if self.open.swap(false, Ordering::SeqCst) {
            self.overrides.impl_close_channel(&*self.delegate)
        } else {
            Ok(())
        }
    }

    fn channel_to_string(&self) -> String {
        self.overrides
            .to_java_string()
            .unwrap_or_else(|| identity_string(self.class_name, self))
    }
}

impl FileChannel for FilterFileChannel {
    fn read_scattering(&self, dsts: &mut [&mut [u8]]) -> io::Result<i64> {
        self.delegate.read_scattering(dsts)
    }

    fn write_gathering(&self, srcs: &[&[u8]]) -> io::Result<i64> {
        self.delegate.write_gathering(srcs)
    }

    fn force(&self, meta_data: bool) -> io::Result<()> {
        self.overrides.force(&*self.delegate, meta_data)
    }

    fn transfer_to(
        &self,
        position: i64,
        count: i64,
        target: &dyn SeekableByteChannel,
    ) -> io::Result<i64> {
        self.delegate.transfer_to(position, count, target)
    }

    fn transfer_from(
        &self,
        src: &dyn SeekableByteChannel,
        position: i64,
        count: i64,
    ) -> io::Result<i64> {
        self.delegate.transfer_from(src, position, count)
    }

    fn read_at(&self, dst: &mut [u8], position: i64) -> io::Result<i32> {
        self.delegate.read_at(dst, position)
    }

    fn write_at(&self, src: &[u8], position: i64) -> io::Result<i32> {
        self.delegate.write_at(src, position)
    }

    fn lock(&self, position: i64, size: i64, shared: bool) -> io::Result<Box<dyn FileLock>> {
        self.delegate.lock(position, size, shared)
    }

    fn try_lock(
        &self,
        position: i64,
        size: i64,
        shared: bool,
    ) -> io::Result<Option<Box<dyn FileLock>>> {
        self.delegate.try_lock(position, size, shared)
    }
}
