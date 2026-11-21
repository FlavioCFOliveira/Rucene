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

//! Port of `org.apache.lucene.tests.mockfile.FilterInputStream2`.

use std::io;

use rucene::store::java_nio_file::InputStream;

use super::identity_string;

/// The methods a subclass of `FilterInputStream2` overrides.
pub trait InputStreamOverrides: Send + Sync {
    /// `close()`: `delegate.close()`.
    fn close(&mut self, delegate: &mut dyn InputStream) -> io::Result<()> {
        delegate.close()
    }

    /// `toString()`, `None` for `Object.toString()`.
    fn to_java_string(&self) -> Option<String> {
        None
    }
}

/// The overrides of `FilterInputStream2` itself: none.
pub struct NoInputStreamOverrides;

impl InputStreamOverrides for NoInputStreamOverrides {}

/// Port of `org.apache.lucene.tests.mockfile.FilterInputStream2`: a
/// `FilterInputStream2` contains another `InputStream`, which it uses as its
/// basic source of data. Unlike `java.io.FilterInputStream`, every method
/// delegates, so the delegate's own optimizations are kept.
pub struct FilterInputStream2 {
    /// `delegate`: the underlying `InputStream` instance.
    delegate: Box<dyn InputStream>,
    overrides: Box<dyn InputStreamOverrides>,
    class_name: &'static str,
}

impl FilterInputStream2 {
    /// `FilterInputStream2(delegate)`.
    pub fn new(delegate: Box<dyn InputStream>) -> FilterInputStream2 {
        Self::with_overrides(
            delegate,
            Box::new(NoInputStreamOverrides),
            "org.apache.lucene.tests.mockfile.FilterInputStream2",
        )
    }

    /// A subclass named `class_name` with `overrides`.
    pub fn with_overrides(
        delegate: Box<dyn InputStream>,
        overrides: Box<dyn InputStreamOverrides>,
        class_name: &'static str,
    ) -> FilterInputStream2 {
        FilterInputStream2 {
            delegate,
            overrides,
            class_name,
        }
    }
}

impl InputStream for FilterInputStream2 {
    fn read_byte(&mut self) -> io::Result<i32> {
        self.delegate.read_byte()
    }

    fn read(&mut self, b: &mut [u8], off: usize, len: usize) -> io::Result<i32> {
        self.delegate.read(b, off, len)
    }

    fn skip(&mut self, n: i64) -> io::Result<i64> {
        self.delegate.skip(n)
    }

    fn available(&mut self) -> io::Result<i32> {
        self.delegate.available()
    }

    fn close(&mut self) -> io::Result<()> {
        self.overrides.close(&mut *self.delegate)
    }

    fn mark(&mut self, readlimit: i32) {
        self.delegate.mark(readlimit);
    }

    fn reset(&mut self) -> io::Result<()> {
        self.delegate.reset()
    }

    fn mark_supported(&self) -> bool {
        self.delegate.mark_supported()
    }

    fn stream_to_string(&self) -> String {
        self.overrides
            .to_java_string()
            .unwrap_or_else(|| identity_string(self.class_name, self))
    }
}
