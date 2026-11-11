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

//! Port of `org.apache.lucene.tests.mockfile.FilterPath`.

use std::sync::Arc;

use rucene::store::java_nio_file::{FileSystem, Path};

use super::filter_file_system::FilterFileSystem;
use super::filter_file_system_provider::to_delegate_of;

/// Port of `org.apache.lucene.tests.mockfile.FilterPath`: a path that
/// contains another path, which it uses as its basic source of data.
///
/// **Divergence.** Java's `FilterPath` is an object wrapping the delegate
/// path; this port's [`Path`] carries its path string and the filesystem it
/// belongs to, so a filter path is a [`Path`] of a [`FilterFileSystem`], and
/// this type holds the class's constructor and static members. Every
/// `FilterPath` method delegates to the wrapped path and wraps the result, so
/// the path string, `equals`, `hashCode` and `compareTo` are the delegate's,
/// as here; `resolve` reaches the overriding filesystem through
/// [`FileSystemProvider::resolve`](rucene::store::java_nio_file::FileSystemProvider::resolve).
pub struct FilterPath;

impl FilterPath {
    /// `new FilterPath(delegate, fileSystem)`: `delegate` as a path of
    /// `file_system`. It returns a [`Path`], the type a filter path is here
    /// (see the type's divergence), hence the lint allowance.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(delegate: &Path, file_system: &Arc<FilterFileSystem>) -> Path {
        delegate.with_file_system(Arc::clone(file_system) as Arc<dyn FileSystem>)
    }

    /// `path instanceof FilterPath`: whether `path` belongs to a filter
    /// filesystem.
    pub fn is_filter_path(path: &Path) -> bool {
        path.get_file_system()
            .as_any()
            .downcast_ref::<FilterFileSystem>()
            .is_some()
    }

    /// `getDelegate()` / `unwrap()`: the wrapped path.
    ///
    /// # Panics
    ///
    /// `ClassCastException` when `path` is not a filter path.
    pub fn get_delegate(path: &Path) -> Path {
        match path
            .get_file_system()
            .as_any()
            .downcast_ref::<FilterFileSystem>()
        {
            Some(fs) => path.with_file_system(Arc::clone(fs.get_delegate())),
            None => rucene::store::java_nio_file::throw_unchecked(
                "java.lang.ClassCastException",
                Some("class sun.nio.fs.UnixPath cannot be cast to class org.apache.lucene.tests.mockfile.FilterPath"),
            ),
        }
    }

    /// The static `FilterPath.unwrap(path)`: unwraps all filter paths,
    /// returning the innermost path ("WARNING: this is exposed for testing
    /// only!").
    pub fn unwrap(path: &Path) -> Path {
        path.unwrap_all()
    }

    /// `toDelegate(path)` of a filter path of `file_system`.
    ///
    /// # Panics
    ///
    /// `ProviderMismatchException`, as `FilterPath.toDelegate`.
    pub fn to_delegate(file_system: &Arc<FilterFileSystem>, path: &Path) -> Path {
        to_delegate_of(file_system, path)
    }
}
