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

//! Port of `org.apache.lucene.tests.mockfile.FilterFileSystem`.

use std::any::Any;
use std::io;
use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use rucene::error::LuceneError;

use rucene::store::java_nio_file::{
    default_file_system, same_file_system, FileStore, FileSystem, FileSystemProvider, Path,
    PathMatcher,
};

use super::filter_file_store::FilterFileStore;
use super::filter_file_system_provider::FilterFileSystemProvider;
use crate::util::java_exceptions::java_throwable;

/// Port of `org.apache.lucene.tests.mockfile.FilterFileSystem`: a filesystem
/// that contains another filesystem, which it uses as its basic source of
/// data, possibly transforming the data along the way or providing additional
/// functionality.
///
/// **Divergence.** Java's provider and filesystem reference each other and
/// the garbage collector reclaims the pair; here the provider holds the
/// filesystem and the filesystem holds the provider, a reference cycle that
/// keeps both alive for the life of the process. Neither holds a resource
/// beyond memory.
pub struct FilterFileSystem {
    /// `parent`: the `FileSystemProvider` that created this filesystem.
    parent: Arc<dyn FilterFileSystemProvider>,
    /// `delegate`: the underlying `FileSystem` instance.
    delegate: Arc<dyn FileSystem>,
}

impl FilterFileSystem {
    /// `FilterFileSystem(parent, delegate)`. Note that the base filesystem is
    /// closed if this filesystem is closed; however the default filesystem
    /// provider will never be closed, it doesn't support that.
    pub fn new(parent: Arc<dyn FilterFileSystemProvider>, delegate: Arc<dyn FileSystem>) -> Self {
        FilterFileSystem { parent, delegate }
    }

    /// `getDelegate()`: the filesystem this one wraps.
    pub fn get_delegate(&self) -> &Arc<dyn FileSystem> {
        &self.delegate
    }

    /// `parent`, as the filter provider it is.
    pub fn parent(&self) -> &Arc<dyn FilterFileSystemProvider> {
        &self.parent
    }

    /// `parent.wrapPath(path)`.
    pub fn wrap_path(&self, path: &Path) -> Path {
        self.parent.wrap_path(path)
    }
}

impl FileSystem for FilterFileSystem {
    fn provider(&self) -> Arc<dyn FileSystemProvider> {
        let parent: Arc<dyn FileSystemProvider> =
            Arc::clone(&self.parent) as Arc<dyn FileSystemProvider>;
        parent
    }

    /// `close()`: "you can't close the default provider!" — only `onClose()`
    /// runs for a filesystem over the default one; any other delegate is
    /// closed after `onClose()`, in a try-with-resources whose close failure
    /// is suppressed by a failure of `onClose()`.
    fn close(&self) -> io::Result<()> {
        if same_file_system(&self.delegate, &default_file_system()) {
            self.parent.on_close();
            Ok(())
        } else {
            let closing = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.parent.on_close();
            }));
            let closed = self.delegate.close();
            match (closing, closed) {
                (Ok(()), closed) => closed,
                // `Throwable.addSuppressed` of the try-with-resources: the
                // delegate's close failure rides on `onClose()`'s.
                (Err(payload), Err(error)) => {
                    let mut primary = Throwable::from_panic(payload);
                    primary.add_suppressed(java_throwable(&LuceneError::from(error)));
                    primary.throw()
                }
                (Err(payload), Ok(())) => std::panic::resume_unwind(payload),
            }
        }
    }

    fn is_open(&self) -> bool {
        self.delegate.is_open()
    }

    fn is_read_only(&self) -> bool {
        self.delegate.is_read_only()
    }

    fn get_separator(&self) -> String {
        self.delegate.get_separator()
    }

    /// `getRootDirectories()`: the delegate's, wrapped.
    fn get_root_directories(self: Arc<Self>) -> Vec<Path> {
        Arc::clone(&self.delegate)
            .get_root_directories()
            .iter()
            .map(|p| self.parent.wrap_path(p))
            .collect()
    }

    /// `getFileStores()`: the delegate's, each in a `FilterFileStore` of the
    /// parent's scheme.
    fn get_file_stores(&self) -> Vec<Arc<dyn FileStore>> {
        self.delegate
            .get_file_stores()
            .into_iter()
            .map(|store| -> Arc<dyn FileStore> {
                Arc::new(FilterFileStore::new(store, self.parent.filter().scheme()))
            })
            .collect()
    }

    fn supported_file_attribute_views(&self) -> Vec<String> {
        self.delegate.supported_file_attribute_views()
    }

    /// `getPath(first, more...)`: `parent.wrapPath(delegate.getPath(first, more))`.
    fn get_path(self: Arc<Self>, first: &str, more: &[&str]) -> Path {
        let path = Arc::clone(&self.delegate).get_path(first, more);
        self.parent.wrap_path(&path)
    }

    /// `getPathMatcher(syntaxAndPattern)`: the delegate's matcher, applied to
    /// the delegate of a filter path; any other path does not match.
    fn get_path_matcher(&self, syntax_and_pattern: &str) -> Box<dyn PathMatcher> {
        let matcher = self.delegate.get_path_matcher(syntax_and_pattern);
        Box::new(FilterPathMatcher { matcher })
    }

    /// `FilterPath.unwrap()`: the delegate path.
    fn unwrap_path(&self, path: &Path) -> Option<Path> {
        Some(path.with_file_system(Arc::clone(&self.delegate)))
    }

    fn file_system_to_string(&self) -> String {
        format!(
            "org.apache.lucene.tests.mockfile.FilterFileSystem@{:x}",
            (std::ptr::from_ref(self).cast::<()>() as usize >> 3) as u32
        )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The matcher `getPathMatcher` returns.
struct FilterPathMatcher {
    matcher: Box<dyn PathMatcher>,
}

impl PathMatcher for FilterPathMatcher {
    fn matches(&self, path: &Path) -> bool {
        match path
            .get_file_system()
            .as_any()
            .downcast_ref::<FilterFileSystem>()
        {
            Some(fs) => self
                .matcher
                .matches(&path.with_file_system(Arc::clone(&fs.delegate))),
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mockfile::filter_file_system_provider::{install, FilterState};

    /// The default filesystem, except that `close()` fails.
    struct FailingClose(Arc<dyn FileSystem>);

    impl FileSystem for FailingClose {
        fn provider(&self) -> Arc<dyn FileSystemProvider> {
            self.0.provider()
        }
        fn close(&self) -> io::Result<()> {
            Err(io::Error::other("delegate close failed"))
        }
        fn is_open(&self) -> bool {
            true
        }
        fn is_read_only(&self) -> bool {
            self.0.is_read_only()
        }
        fn get_separator(&self) -> String {
            self.0.get_separator()
        }
        fn get_root_directories(self: Arc<Self>) -> Vec<Path> {
            Arc::clone(&self.0).get_root_directories()
        }
        fn get_file_stores(&self) -> Vec<Arc<dyn FileStore>> {
            self.0.get_file_stores()
        }
        fn supported_file_attribute_views(&self) -> Vec<String> {
            self.0.supported_file_attribute_views()
        }
        fn get_path(self: Arc<Self>, first: &str, more: &[&str]) -> Path {
            Arc::clone(&self.0).get_path(first, more)
        }
        fn get_path_matcher(&self, syntax_and_pattern: &str) -> Box<dyn PathMatcher> {
            self.0.get_path_matcher(syntax_and_pattern)
        }
        fn file_system_to_string(&self) -> String {
            "FailingClose".to_string()
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    /// A filter provider whose `onClose()` throws.
    struct ThrowingOnClose {
        filter: FilterState,
    }

    impl FilterFileSystemProvider for ThrowingOnClose {
        fn filter(&self) -> &FilterState {
            &self.filter
        }
        fn class_name(&self) -> &'static str {
            "ThrowingOnClose"
        }
        fn on_close(&self) {
            randomizedtesting::jdk::exception::illegal_state("onClose failed")
        }
    }

    crate::impl_file_system_provider!(ThrowingOnClose);

    /// `try (FileSystem d = delegate) { parent.onClose(); }`
    /// (`FilterFileSystem.java:67-70`): when both throw, `onClose()`'s
    /// exception propagates with the delegate's close failure suppressed.
    #[test]
    fn delegate_close_failure_is_suppressed_by_on_close_failure() {
        let delegate: Arc<dyn FileSystem> = Arc::new(FailingClose(default_file_system()));
        let provider = Arc::new(ThrowingOnClose {
            filter: FilterState::new("throwing://", &delegate),
        });
        install(&provider, delegate);
        let fs = FilterFileSystemProvider::get_file_system(&*provider);
        let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = fs.close();
        }))
        .unwrap_err();
        let t = Throwable::from_panic(payload);
        assert_eq!(t.class_name(), "java.lang.IllegalStateException");
        assert_eq!(t.message(), Some("onClose failed"));
        assert_eq!(t.suppressed().len(), 1, "{t:?}");
        assert_eq!(
            t.suppressed()[0].message(),
            Some("delegate close failed"),
            "{t:?}"
        );
    }
}
