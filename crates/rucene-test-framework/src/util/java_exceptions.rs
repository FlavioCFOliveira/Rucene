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

//! The Java exception a Rucene error stands for, as a [`Throwable`] whose
//! class (a binary name) and superclasses are Java's, so that
//! `expectThrows(X.class, …)` and `catch (X e)` answer as on the JVM.
//!
//! Used by the suites of rmp #350, #356 and #358
//! (`BaseDirectoryTestCase`, `BaseLockFactoryTestCase`, the search and geo
//! suites), whose bodies call `expectThrows` on Lucene calls.

use std::sync::Once;

use randomizedtesting::jdk::exception::{register_class, Throwable};
use rucene::error::{LuceneError, Result};
use rucene::store::java_nio_file;

use crate::util::LuceneTestCase;

/// The superclass of each exception class the mapping produces.
const HIERARCHY: &[(&str, &str)] = &[
    ("java.lang.Exception", "java.lang.Throwable"),
    ("java.lang.RuntimeException", "java.lang.Exception"),
    ("java.io.IOException", "java.lang.Exception"),
    ("java.io.EOFException", "java.io.IOException"),
    ("java.io.FileNotFoundException", "java.io.IOException"),
    ("java.io.UncheckedIOException", "java.lang.RuntimeException"),
    ("java.nio.file.FileSystemException", "java.io.IOException"),
    (
        "java.nio.file.NoSuchFileException",
        "java.nio.file.FileSystemException",
    ),
    (
        "java.nio.file.FileAlreadyExistsException",
        "java.nio.file.FileSystemException",
    ),
    (
        "java.nio.file.AccessDeniedException",
        "java.nio.file.FileSystemException",
    ),
    (
        "java.lang.IllegalArgumentException",
        "java.lang.RuntimeException",
    ),
    (
        "java.lang.IllegalStateException",
        "java.lang.RuntimeException",
    ),
    (
        "java.lang.UnsupportedOperationException",
        "java.lang.RuntimeException",
    ),
    (
        "java.lang.ArithmeticException",
        "java.lang.RuntimeException",
    ),
    (
        "java.lang.IndexOutOfBoundsException",
        "java.lang.RuntimeException",
    ),
    (
        "java.lang.StringIndexOutOfBoundsException",
        "java.lang.IndexOutOfBoundsException",
    ),
    (
        "java.lang.NullPointerException",
        "java.lang.RuntimeException",
    ),
    ("java.lang.Error", "java.lang.Throwable"),
    ("java.lang.AssertionError", "java.lang.Error"),
    (
        "org.apache.lucene.store.AlreadyClosedException",
        "java.lang.IllegalStateException",
    ),
    (
        "org.apache.lucene.store.LockObtainFailedException",
        "java.io.IOException",
    ),
    (
        "org.apache.lucene.store.LockReleaseFailedException",
        "java.io.IOException",
    ),
    (
        "org.apache.lucene.index.CorruptIndexException",
        "java.io.IOException",
    ),
    (
        "org.apache.lucene.index.IndexFormatTooOldException",
        "org.apache.lucene.index.CorruptIndexException",
    ),
    (
        "org.apache.lucene.index.IndexFormatTooNewException",
        "org.apache.lucene.index.CorruptIndexException",
    ),
    (
        "org.apache.lucene.index.IndexNotFoundException",
        "java.io.FileNotFoundException",
    ),
    (
        "org.apache.lucene.search.IndexSearcher$TooManyClauses",
        "java.lang.RuntimeException",
    ),
    (
        "java.util.concurrent.RejectedExecutionException",
        "java.lang.RuntimeException",
    ),
    (
        "java.util.concurrent.ExecutionException",
        "java.lang.Exception",
    ),
];

/// Registers [`HIERARCHY`] once.
fn register_hierarchy() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        for (class, superclass) in HIERARCHY {
            register_class(class, superclass);
        }
    });
}

/// The binary name of the Java class `e` stands for.
pub fn java_class_name(e: &LuceneError) -> String {
    match e.unwrap_suppressed() {
        LuceneError::Io(io) => java_nio_file::java_class_of(io).to_string(),
        LuceneError::CorruptIndex(_) => "org.apache.lucene.index.CorruptIndexException".into(),
        LuceneError::IndexFormatTooOld { .. } => {
            "org.apache.lucene.index.IndexFormatTooOldException".into()
        }
        LuceneError::IndexFormatTooNew { .. } => {
            "org.apache.lucene.index.IndexFormatTooNewException".into()
        }
        LuceneError::IndexNotFound(_) => "org.apache.lucene.index.IndexNotFoundException".into(),
        LuceneError::NotFound(_) => "java.io.FileNotFoundException".into(),
        LuceneError::IllegalArgument(_) | LuceneError::IllegalArgumentWithCause { .. } => {
            "java.lang.IllegalArgumentException".into()
        }
        LuceneError::IllegalState { .. } => "java.lang.IllegalStateException".into(),
        LuceneError::LockObtainFailed { .. } => {
            "org.apache.lucene.store.LockObtainFailedException".into()
        }
        LuceneError::LockReleaseFailed { .. } => {
            "org.apache.lucene.store.LockReleaseFailedException".into()
        }
        LuceneError::AlreadyClosed { .. } => {
            "org.apache.lucene.store.AlreadyClosedException".into()
        }
        LuceneError::UnsupportedOperation(_)
        | LuceneError::UnsupportedOperationWithCause { .. } => {
            "java.lang.UnsupportedOperationException".into()
        }
        LuceneError::StringIndexOutOfBounds(_) => {
            "java.lang.StringIndexOutOfBoundsException".into()
        }
        LuceneError::Arithmetic(_) => "java.lang.ArithmeticException".into(),
        LuceneError::AssertionError(_) => "java.lang.AssertionError".into(),
        LuceneError::Unchecked { class, .. } => class.to_string(),
        other => other.java_simple_name().to_string(),
    }
}

/// The Java throwable `e` stands for: its class, its message, and its
/// suppressed exceptions (`getSuppressed()`).
pub fn java_throwable(e: &LuceneError) -> Throwable {
    register_hierarchy();
    let (_, message) = crate::analysis::base_token_stream_test_case::java_class_and_message(e);
    let mut throwable = Throwable::new(java_class_name(e), message);
    // `getSuppressed()` of an error that `addSuppressed` was called on.
    for suppressed in e.get_suppressed() {
        throwable.add_suppressed(java_throwable(suppressed));
    }
    // `getSuppressed()` of an `IOException` that carries suppressed ones.
    if let LuceneError::Io(io) = e.unwrap_suppressed() {
        if let Some(exception) = rucene::store::java_nio_file::java_exception_of(io) {
            for suppressed in exception.get_suppressed() {
                throwable.add_suppressed(java_io_throwable(suppressed));
            }
        }
    }
    throwable
}

/// The Java throwable a [`JavaIoException`] stands for, with its suppressed
/// exceptions.
///
/// [`JavaIoException`]: rucene::store::java_nio_file::JavaIoException
fn java_io_throwable(exception: &rucene::store::java_nio_file::JavaIoException) -> Throwable {
    let mut throwable = Throwable::with_message(exception.class_name(), exception.get_message());
    for suppressed in exception.get_suppressed() {
        throwable.add_suppressed(java_io_throwable(suppressed));
    }
    throwable
}

/// Throws `e` out of a test method as the Java exception it stands for.
pub fn rethrow_java(e: LuceneError) -> ! {
    java_throwable(&e).throw()
}

/// Whether Java's `catch (class e)` catches `e`.
pub fn is_instance(e: &LuceneError, class: &str) -> bool {
    java_throwable(e).is_instance(class)
}

/// `expectThrows(class, runnable)` over a Rucene call: the error the call
/// returns (or the throwable it panics with) must be an instance of
/// `class`; returns the throwable.
pub fn expect_throws<T>(class: &str, runnable: impl FnOnce() -> Result<T>) -> Throwable {
    register_hierarchy();
    LuceneTestCase::expect_throws(class, || {
        runnable().map(|_| ()).map_err(|e| java_throwable(&e))
    })
}

/// `expectThrowsAnyOf(classes, runnable)` over a Rucene call.
pub fn expect_throws_any_of<T>(
    classes: &[&str],
    runnable: impl FnOnce() -> Result<T>,
) -> Throwable {
    register_hierarchy();
    LuceneTestCase::expect_throws_any_of(classes, || {
        runnable().map(|_| ()).map_err(|e| java_throwable(&e))
    })
}
