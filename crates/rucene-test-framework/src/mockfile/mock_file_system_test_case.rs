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

//! Port of `org.apache.lucene.tests.mockfile.MockFileSystemTestCase`.

use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::junit::Assert;
use rucene::error::LuceneError;
use rucene::store::java_nio_file::{DirectoryStream, Files, Path};

use super::filter_path::FilterPath;
use crate::util::java_exceptions::java_throwable;
use crate::util::lucene_test_case::LuceneTestCase;

/// The declaration of `org.apache.lucene.tests.mockfile.MockFileSystemTestCase`
/// (`@SuppressFileSystems("*") public abstract class MockFileSystemTestCase
/// extends LuceneTestCase`) for the suites that extend it; the tests it
/// declares are the methods of [`MockFileSystemTestCase`].
pub fn class() -> std::sync::Arc<randomizedtesting::jdk::reflect::Class> {
    use randomizedtesting::jdk::reflect::{modifier, Class};
    use std::sync::{Arc, LazyLock};
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        Class::builder("org.apache.lucene.tests.mockfile.MockFileSystemTestCase")
            .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
            // we suppress random filesystems and do tests explicitly.
            .annotate(crate::util::lucene_test_case::SuppressFileSystems {
                value: vec!["*".to_string()],
            })
            .extends(&LuceneTestCase::class())
            .build()
    });
    Arc::clone(&CLASS)
}

/// Port of `org.apache.lucene.tests.mockfile.MockFileSystemTestCase`: the base
/// class for testing mock filesystems. This tests things that really need to
/// work: `Path` `equals()`/`hashCode()`, directory listing glob and
/// filtering, URI conversion, etc. Java's class is
/// `@SuppressFileSystems("*")` ("we suppress random filesystems and do tests
/// explicitly"): a suite running these tests carries
/// [`SuppressFileSystems`](crate::util::lucene_test_case::SuppressFileSystems)
/// with `*` and `LuceneTestCase`'s temporary-files rule, whose
/// `createTempDir()` each test starts from.
pub trait MockFileSystemTestCase {
    /// `wrap(path)`: wraps `path` with custom behavior.
    fn wrap(&self, path: &Path) -> Path;

    /// `testHashCodeEquals()`: `Path.hashCode`/`equals` are sane
    /// (`@SuppressWarnings("SelfAssertion")`). Java's `assertFalse(f1.equals(null))`
    /// has no counterpart: a Rust path is never null.
    #[allow(clippy::eq_op)]
    fn test_hash_code_equals(&self) {
        let dir = self.wrap(&LuceneTestCase::create_temp_dir());

        let f1 = dir.resolve_str("file1");
        let f1_again = dir.resolve_str("file1");
        let f2 = dir.resolve_str("file2");

        assert_equals_paths(&f1, &f1);
        assert_equals_paths(&f1, &f1_again);
        Assert::assert_equals(f1.hash_code(), f1_again.hash_code());
        Assert::assert_false(f1 == f2);
        close_file_system(&dir);
    }

    /// `testURI()`: URIs are not corrupted (plain ASCII).
    fn test_uri(&self) {
        self.impl_test_uri("file1");
    }

    /// `testURIumlaute()`: umlauts and s-zet.
    fn test_uri_umlaute(&self) {
        self.impl_test_uri("\u{e4}\u{c4}\u{f6}\u{d6}\u{fc}\u{dc}\u{df}");
    }

    /// `testURIchinese()`.
    fn test_uri_chinese(&self) {
        self.impl_test_uri("\u{4e2d}\u{56fd}");
    }

    /// `implTestURI(fileName)`.
    fn impl_test_uri(&self, file_name: &str) {
        let dir = self.wrap(&LuceneTestCase::create_temp_dir());

        let f1 = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            dir.resolve_str(file_name)
        })) {
            Ok(path) => path,
            Err(payload) => {
                let t = Throwable::from_panic(payload);
                if t.is_instance("java.nio.file.InvalidPathException") {
                    LuceneTestCase::assume_no_exception(
                        &format!("couldn't resolve '{file_name}'"),
                        Some(&t),
                    );
                }
                t.throw()
            }
        };

        let uri = f1.to_uri();
        let f2 = dir.get_file_system().provider().get_path(&uri);
        assert_equals_paths(&f1, &f2);

        close_file_system(&dir);
    }

    /// `testDirectoryStreamFiltered()`: `newDirectoryStream` with a filter
    /// works correctly.
    fn test_directory_stream_filtered(&self) {
        let dir = self.wrap(&LuceneTestCase::create_temp_dir());

        let mut file =
            Files::new_output_stream(&dir.resolve_str("file1"), &[]).unwrap_or_else(rethrow);
        file.write_byte(5).unwrap_or_else(rethrow);
        file.close().unwrap_or_else(rethrow);
        let stream = Files::new_directory_stream(&dir).unwrap_or_else(rethrow);
        with_resource(stream, |stream| {
            let mut count = 0;
            for path in stream.iterator() {
                let path = path.unwrap_or_else(rethrow);
                Assert::assert_true(FilterPath::is_filter_path(&path));
                if !path
                    .get_file_name()
                    .map(|n| n.to_string())
                    .unwrap_or_default()
                    .starts_with("extra")
                {
                    count += 1;
                }
            }
            Assert::assert_equals(1, count);
        });
        close_file_system(&dir);
    }

    /// `testDirectoryStreamGlobFiltered()`: `newDirectoryStream` with globbing
    /// works correctly.
    fn test_directory_stream_glob_filtered(&self) {
        let dir = self.wrap(&LuceneTestCase::create_temp_dir());

        let mut file =
            Files::new_output_stream(&dir.resolve_str("foo"), &[]).unwrap_or_else(rethrow);
        file.write_byte(5).unwrap_or_else(rethrow);
        file.close().unwrap_or_else(rethrow);
        let mut file =
            Files::new_output_stream(&dir.resolve_str("bar"), &[]).unwrap_or_else(rethrow);
        file.write_byte(5).unwrap_or_else(rethrow);
        file.close().unwrap_or_else(rethrow);
        let stream = Files::new_directory_stream_glob(&dir, "f*").unwrap_or_else(rethrow);
        with_resource(stream, |stream| {
            let mut count = 0;
            for path in stream.iterator() {
                let path = path.unwrap_or_else(rethrow);
                Assert::assert_true(FilterPath::is_filter_path(&path));
                count += 1;
            }
            Assert::assert_equals(1, count);
        });
        close_file_system(&dir);
    }
}

/// `dir.getFileSystem().close()`.
fn close_file_system(dir: &Path) {
    let fs = Arc::clone(dir.get_file_system());
    fs.close().unwrap_or_else(rethrow);
}

/// Throws an `IOException` out of a test method, as `throws IOException`.
fn rethrow<T>(error: std::io::Error) -> T {
    java_throwable(&LuceneError::from(error)).throw()
}

/// `try (DirectoryStream<Path> stream = ...) { body }`: the stream is closed
/// whether `body` completes or throws; a close failure after a throwing body
/// is suppressed by the body's throwable.
fn with_resource(
    mut stream: Box<dyn DirectoryStream>,
    body: impl FnOnce(&mut Box<dyn DirectoryStream>),
) {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| body(&mut stream)));
    let closed = stream.close();
    match (outcome, closed) {
        (Ok(()), closed) => {
            closed.unwrap_or_else(rethrow);
        }
        (Err(payload), Err(error)) => {
            let mut primary = Throwable::from_panic(payload);
            primary.add_suppressed(java_throwable(&LuceneError::from(error)));
            primary.throw()
        }
        (Err(payload), Ok(())) => std::panic::resume_unwind(payload),
    }
}

/// JUnit's `assertEquals(Object expected, Object actual)` of two paths:
/// `expected:<x> but was:<y>`, or, when both print alike, each with its
/// class (`expected: C<x> but was: C<y>`).
fn assert_equals_paths(expected: &Path, actual: &Path) {
    if expected != actual {
        let (e, a) = (expected.to_string(), actual.to_string());
        let message = if e == a {
            format!(
                "expected: {}<{e}> but was: {}<{a}>",
                path_class(expected),
                path_class(actual)
            )
        } else {
            format!("expected:<{e}> but was:<{a}>")
        };
        Assert::fail(&message)
    }
}

/// `path.getClass().getName()`: a `WindowsPath` for `WindowsFS`'s paths, a
/// `FilterPath` for another filter filesystem's, the JDK's `UnixPath`
/// otherwise.
fn path_class(path: &Path) -> &'static str {
    match path
        .get_file_system()
        .as_any()
        .downcast_ref::<super::FilterFileSystem>()
    {
        Some(fs) if fs.parent().class_name() == "org.apache.lucene.tests.mockfile.WindowsFS" => {
            "org.apache.lucene.tests.mockfile.WindowsPath"
        }
        Some(_) => "org.apache.lucene.tests.mockfile.FilterPath",
        None => "sun.nio.fs.UnixPath",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mockfile::{DisableFsyncFS, FilterFileSystemProvider};
    use rucene::store::java_nio_file::default_file_system;

    /// JUnit's `assertEquals(Object, Object)`: two unequal paths that print
    /// alike fail with each value's class.
    #[test]
    fn unequal_paths_printing_alike_fail_with_their_classes() {
        let plain = default_file_system().get_path("/tmp/x", &[]);
        let fs = DisableFsyncFS::new(default_file_system());
        let filtered = FilterFileSystemProvider::get_file_system(&*fs).get_path("/tmp/x", &[]);
        assert_ne!(plain, filtered);
        let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert_equals_paths(&plain, &filtered)
        }))
        .unwrap_err();
        let t = Throwable::from_panic(payload);
        assert_eq!(t.class_name(), "java.lang.AssertionError");
        assert_eq!(
            t.message(),
            Some(
                "expected: sun.nio.fs.UnixPath</tmp/x> but was: \
                 org.apache.lucene.tests.mockfile.FilterPath</tmp/x>"
            )
        );
    }
}
