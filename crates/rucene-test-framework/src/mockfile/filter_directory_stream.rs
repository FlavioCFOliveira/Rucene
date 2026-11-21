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

//! Port of `org.apache.lucene.tests.mockfile.FilterDirectoryStream`.

use std::io;
use std::sync::Arc;

use rucene::store::java_nio_file::{DirectoryStream, Path};

use super::filter_file_system::FilterFileSystem;
use super::identity_string;

/// The methods a subclass of `FilterDirectoryStream` overrides.
pub trait DirectoryStreamOverrides: Send + Sync {
    /// `close()`: `delegate.close()`.
    fn close(&mut self, delegate: &mut dyn DirectoryStream) -> io::Result<()> {
        delegate.close()
    }

    /// `toString()`, `None` for `Object.toString()`.
    fn to_java_string(&self) -> Option<String> {
        None
    }
}

/// The overrides of `FilterDirectoryStream` itself: none.
pub struct NoDirectoryStreamOverrides;

impl DirectoryStreamOverrides for NoDirectoryStreamOverrides {}

/// Port of `org.apache.lucene.tests.mockfile.FilterDirectoryStream`: a
/// `DirectoryStream` that contains another, which it uses as its basic source
/// of data; the entries are wrapped by the filesystem's provider.
pub struct FilterDirectoryStream {
    /// `delegate`: the underlying `DirectoryStream` instance.
    delegate: Box<dyn DirectoryStream>,
    /// `fileSystem`: the parent filesystem of the wrapped paths.
    file_system: Arc<FilterFileSystem>,
    overrides: Box<dyn DirectoryStreamOverrides>,
    class_name: &'static str,
}

impl FilterDirectoryStream {
    /// `FilterDirectoryStream(delegate, fileSystem)`.
    pub fn new(delegate: Box<dyn DirectoryStream>, file_system: Arc<FilterFileSystem>) -> Self {
        Self::with_overrides(
            delegate,
            file_system,
            Box::new(NoDirectoryStreamOverrides),
            "org.apache.lucene.tests.mockfile.FilterDirectoryStream",
        )
    }

    /// A subclass named `class_name` with `overrides`.
    pub fn with_overrides(
        delegate: Box<dyn DirectoryStream>,
        file_system: Arc<FilterFileSystem>,
        overrides: Box<dyn DirectoryStreamOverrides>,
        class_name: &'static str,
    ) -> Self {
        FilterDirectoryStream {
            delegate,
            file_system,
            overrides,
            class_name,
        }
    }
}

impl DirectoryStream for FilterDirectoryStream {
    /// `iterator()`: the delegate's entries, each `fileSystem.parent.wrapPath(next)`.
    fn iterator(&mut self) -> Box<dyn Iterator<Item = io::Result<Path>> + Send + '_> {
        let fs = Arc::clone(&self.file_system);
        Box::new(
            self.delegate
                .iterator()
                .map(move |entry| entry.map(|path| fs.wrap_path(&path))),
        )
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
