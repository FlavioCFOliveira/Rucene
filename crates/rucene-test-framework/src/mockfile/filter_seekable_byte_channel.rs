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

//! Port of `org.apache.lucene.tests.mockfile.FilterSeekableByteChannel`.

use std::io;

use rucene::store::java_nio_file::SeekableByteChannel;

use super::identity_string;

/// The methods a subclass of `FilterSeekableByteChannel` overrides.
pub trait SeekableByteChannelOverrides: Send + Sync {
    /// `close()`: `delegate.close()`.
    fn close(&self, delegate: &dyn SeekableByteChannel) -> io::Result<()> {
        delegate.close()
    }

    /// `toString()`, `None` for `Object.toString()`.
    fn to_java_string(&self) -> Option<String> {
        None
    }
}

/// The overrides of `FilterSeekableByteChannel` itself: none.
pub struct NoSeekableByteChannelOverrides;

impl SeekableByteChannelOverrides for NoSeekableByteChannelOverrides {}

/// Port of `org.apache.lucene.tests.mockfile.FilterSeekableByteChannel`: a
/// `SeekableByteChannel` that contains another, which it uses as its basic
/// source of data.
pub struct FilterSeekableByteChannel {
    /// `delegate`: the underlying `SeekableByteChannel` instance.
    delegate: Box<dyn SeekableByteChannel>,
    overrides: Box<dyn SeekableByteChannelOverrides>,
    class_name: &'static str,
}

impl FilterSeekableByteChannel {
    /// `FilterSeekableByteChannel(delegate)`.
    pub fn new(delegate: Box<dyn SeekableByteChannel>) -> FilterSeekableByteChannel {
        Self::with_overrides(
            delegate,
            Box::new(NoSeekableByteChannelOverrides),
            "org.apache.lucene.tests.mockfile.FilterSeekableByteChannel",
        )
    }

    /// A subclass named `class_name` with `overrides`.
    pub fn with_overrides(
        delegate: Box<dyn SeekableByteChannel>,
        overrides: Box<dyn SeekableByteChannelOverrides>,
        class_name: &'static str,
    ) -> FilterSeekableByteChannel {
        FilterSeekableByteChannel {
            delegate,
            overrides,
            class_name,
        }
    }
}

impl SeekableByteChannel for FilterSeekableByteChannel {
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

    /// `isOpen()`: `delegate.isOpen()`.
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
