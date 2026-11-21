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

//! Port of `org.apache.lucene.tests.mockfile.FilterOutputStream2`.

use std::io;

use rucene::store::java_nio_file::OutputStream;

use super::identity_string;

/// The methods a subclass of `FilterOutputStream2` overrides.
pub trait OutputStreamOverrides: Send + Sync {
    /// `close()`: `delegate.close()`.
    fn close(&mut self, delegate: &mut dyn OutputStream) -> io::Result<()> {
        delegate.close()
    }

    /// `toString()`, `None` for `Object.toString()`.
    fn to_java_string(&self) -> Option<String> {
        None
    }
}

/// The overrides of `FilterOutputStream2` itself: none.
pub struct NoOutputStreamOverrides;

impl OutputStreamOverrides for NoOutputStreamOverrides {}

/// Port of `org.apache.lucene.tests.mockfile.FilterOutputStream2`: a
/// `FilterOutputStream2` contains another `OutputStream`, which it uses as its
/// basic source of data. Unlike `java.io.FilterOutputStream`, every method
/// delegates, so the delegate's own optimizations are kept.
pub struct FilterOutputStream2 {
    /// `delegate`: the underlying `OutputStream` instance.
    delegate: Box<dyn OutputStream>,
    overrides: Box<dyn OutputStreamOverrides>,
    class_name: &'static str,
}

impl FilterOutputStream2 {
    /// A subclass of the abstract `FilterOutputStream2` named `class_name`
    /// with `overrides`.
    pub fn with_overrides(
        delegate: Box<dyn OutputStream>,
        overrides: Box<dyn OutputStreamOverrides>,
        class_name: &'static str,
    ) -> FilterOutputStream2 {
        FilterOutputStream2 {
            delegate,
            overrides,
            class_name,
        }
    }
}

impl OutputStream for FilterOutputStream2 {
    fn write_byte(&mut self, b: i32) -> io::Result<()> {
        self.delegate.write_byte(b)
    }

    fn write(&mut self, b: &[u8], off: usize, len: usize) -> io::Result<()> {
        self.delegate.write(b, off, len)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.delegate.flush()
    }

    fn close(&mut self) -> io::Result<()> {
        self.overrides.close(&mut *self.delegate)
    }

    fn stream_to_string(&self) -> String {
        self.overrides
            .to_java_string()
            .unwrap_or_else(|| identity_string(self.class_name, self))
    }
}
