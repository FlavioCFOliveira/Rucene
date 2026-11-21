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

//! Port of `org.apache.lucene.tests.mockfile.FilterAsynchronousFileChannel`.

use std::io;
use std::sync::Arc;

use rucene::store::java_nio_file::{
    AsynchronousFileChannel, CompletionHandler, FileLock, JavaFuture,
};

use super::identity_string;

/// The methods a subclass of `FilterAsynchronousFileChannel` overrides.
pub trait AsynchronousFileChannelOverrides: Send + Sync {
    /// `close()`: `delegate.close()`.
    fn close(&self, delegate: &dyn AsynchronousFileChannel) -> io::Result<()> {
        delegate.close()
    }

    /// `force(metaData)`: `delegate.force(metaData)`.
    fn force(&self, delegate: &dyn AsynchronousFileChannel, meta_data: bool) -> io::Result<()> {
        delegate.force(meta_data)
    }

    /// `toString()`, `None` for `Object.toString()`.
    fn to_java_string(&self) -> Option<String> {
        None
    }
}

/// The overrides of `FilterAsynchronousFileChannel` itself: none.
pub struct NoAsynchronousFileChannelOverrides;

impl AsynchronousFileChannelOverrides for NoAsynchronousFileChannelOverrides {}

/// Port of `org.apache.lucene.tests.mockfile.FilterAsynchronousFileChannel`:
/// an `AsynchronousFileChannel` that contains another, which it uses as its
/// basic source of data.
pub struct FilterAsynchronousFileChannel {
    /// `delegate`: the underlying `AsynchronousFileChannel` instance.
    delegate: Arc<dyn AsynchronousFileChannel>,
    overrides: Box<dyn AsynchronousFileChannelOverrides>,
    class_name: &'static str,
}

impl FilterAsynchronousFileChannel {
    /// `FilterAsynchronousFileChannel(delegate)`.
    pub fn new(delegate: Arc<dyn AsynchronousFileChannel>) -> Self {
        Self::with_overrides(
            delegate,
            Box::new(NoAsynchronousFileChannelOverrides),
            "org.apache.lucene.tests.mockfile.FilterAsynchronousFileChannel",
        )
    }

    /// A subclass named `class_name` with `overrides`.
    pub fn with_overrides(
        delegate: Arc<dyn AsynchronousFileChannel>,
        overrides: Box<dyn AsynchronousFileChannelOverrides>,
        class_name: &'static str,
    ) -> Self {
        FilterAsynchronousFileChannel {
            delegate,
            overrides,
            class_name,
        }
    }
}

impl AsynchronousFileChannel for FilterAsynchronousFileChannel {
    fn size(&self) -> io::Result<i64> {
        self.delegate.size()
    }

    fn truncate(&self, size: i64) -> io::Result<()> {
        self.delegate.truncate(size)
    }

    fn force(&self, meta_data: bool) -> io::Result<()> {
        self.overrides.force(&*self.delegate, meta_data)
    }

    fn lock(&self, position: i64, size: i64, shared: bool) -> JavaFuture<Box<dyn FileLock>> {
        self.delegate.lock(position, size, shared)
    }

    fn lock_with_handler(
        &self,
        position: i64,
        size: i64,
        shared: bool,
        handler: CompletionHandler<Box<dyn FileLock>>,
    ) {
        self.delegate
            .lock_with_handler(position, size, shared, handler)
    }

    fn try_lock(
        &self,
        position: i64,
        size: i64,
        shared: bool,
    ) -> io::Result<Option<Box<dyn FileLock>>> {
        self.delegate.try_lock(position, size, shared)
    }

    fn read(&self, dst: &mut [u8], position: i64) -> JavaFuture<i32> {
        self.delegate.read(dst, position)
    }

    fn read_with_handler(&self, dst: &mut [u8], position: i64, handler: CompletionHandler<i32>) {
        self.delegate.read_with_handler(dst, position, handler)
    }

    fn write(&self, src: &[u8], position: i64) -> JavaFuture<i32> {
        self.delegate.write(src, position)
    }

    fn write_with_handler(&self, src: &[u8], position: i64, handler: CompletionHandler<i32>) {
        self.delegate.write_with_handler(src, position, handler)
    }

    fn is_open(&self) -> bool {
        self.delegate.is_open()
    }

    fn close(&self) -> io::Result<()> {
        self.overrides.close(&*self.delegate)
    }

    fn channel_to_string(&self) -> String {
        self.overrides
            .to_java_string()
            .unwrap_or_else(|| identity_string(self.class_name, self))
    }
}
