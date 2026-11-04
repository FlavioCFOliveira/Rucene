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
use rucene::store::java_nio_file::{Files, Path};

use super::filter_path::FilterPath;
use crate::util::lucene_test_case::LuceneTestCase;

/// Port of `org.apache.lucene.tests.mockfile.MockFileSystemTestCase`: the base
/// class for testing mock filesystems. This tests things that really need to
/// work: `Path` `equals()`/`hashCode()`, directory listing glob and
/// filtering, URI conversion, etc. (`@SuppressFileSystems("*")`: the random
/// filesystems are suppressed and the tests are done explicitly.)
///
/// **Divergence.** Java's test methods start from `createTempDir()`, which
/// `LuceneTestCase`'s `TestRuleTemporaryFilesCleanup` serves (task #348);
/// until that rule is ported, an implementation supplies the directory
/// through [`create_temp_dir`](Self::create_temp_dir).
pub trait MockFileSystemTestCase {
    /// `wrap(path)`: wraps `path` with custom behavior.
    fn wrap(&self, path: &Path) -> Path;

    /// `createTempDir()`: a new, empty temporary directory.
    fn create_temp_dir(&self) -> Path;

    /// `testHashCodeEquals()`: `Path.hashCode`/`equals` are sane
    /// (`@SuppressWarnings("SelfAssertion")`). Java's `assertFalse(f1.equals(null))`
    /// has no counterpart: a Rust path is never null.
    #[allow(clippy::eq_op)]
    fn test_hash_code_equals(&self) {
        let dir = self.wrap(&self.create_temp_dir());

        let f1 = dir.resolve_str("file1");
        let f1_again = dir.resolve_str("file1");
        let f2 = dir.resolve_str("file2");

        Assert::assert_true(f1 == f1);
        Assert::assert_true(f1 == f1_again);
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
        let dir = self.wrap(&self.create_temp_dir());

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
        Assert::assert_true(f1 == f2);

        close_file_system(&dir);
    }

    /// `testDirectoryStreamFiltered()`: `newDirectoryStream` with a filter
    /// works correctly.
    fn test_directory_stream_filtered(&self) {
        let dir = self.wrap(&self.create_temp_dir());

        let mut file =
            Files::new_output_stream(&dir.resolve_str("file1"), &[]).unwrap_or_else(rethrow);
        file.write_byte(5).unwrap_or_else(rethrow);
        file.close().unwrap_or_else(rethrow);
        let mut stream = Files::new_directory_stream(&dir).unwrap_or_else(rethrow);
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
        stream.close().unwrap_or_else(rethrow);
        close_file_system(&dir);
    }

    /// `testDirectoryStreamGlobFiltered()`: `newDirectoryStream` with globbing
    /// works correctly.
    fn test_directory_stream_glob_filtered(&self) {
        let dir = self.wrap(&self.create_temp_dir());

        let mut file =
            Files::new_output_stream(&dir.resolve_str("foo"), &[]).unwrap_or_else(rethrow);
        file.write_byte(5).unwrap_or_else(rethrow);
        file.close().unwrap_or_else(rethrow);
        let mut file =
            Files::new_output_stream(&dir.resolve_str("bar"), &[]).unwrap_or_else(rethrow);
        file.write_byte(5).unwrap_or_else(rethrow);
        file.close().unwrap_or_else(rethrow);
        let mut stream = Files::new_directory_stream_glob(&dir, "f*").unwrap_or_else(rethrow);
        let mut count = 0;
        for path in stream.iterator() {
            let path = path.unwrap_or_else(rethrow);
            Assert::assert_true(FilterPath::is_filter_path(&path));
            count += 1;
        }
        Assert::assert_equals(1, count);
        stream.close().unwrap_or_else(rethrow);
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
    Throwable::new(
        rucene::store::java_nio_file::java_class_of(&error).to_string(),
        rucene::store::java_nio_file::java_message_of(&error).unwrap_or_default(),
    )
    .throw()
}
