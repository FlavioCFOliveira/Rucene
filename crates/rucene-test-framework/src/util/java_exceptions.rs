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
use rucene::error::{LuceneError, LuceneErrorKind, Result};
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
        "org.apache.lucene.search.IndexSearcher$TooManyNestedClauses",
        "org.apache.lucene.search.IndexSearcher$TooManyClauses",
    ),
    (
        "org.apache.lucene.index.ExitableDirectoryReader$ExitingReaderException",
        "java.lang.RuntimeException",
    ),
    (
        "org.apache.lucene.index.CheckIndex$CheckIndexException",
        "java.lang.RuntimeException",
    ),
    (
        "org.apache.lucene.util.automaton.TooComplexToDeterminizeException",
        "java.lang.RuntimeException",
    ),
    (
        "org.apache.lucene.util.BytesRefHash$MaxBytesLengthExceededException",
        "java.lang.RuntimeException",
    ),
    (
        "org.apache.lucene.index.MergePolicy$MergeException",
        "java.lang.RuntimeException",
    ),
    (
        "org.apache.lucene.index.MergePolicy$MergeAbortedException",
        "java.io.IOException",
    ),
    (
        "org.apache.lucene.util.ThreadInterruptedException",
        "java.lang.RuntimeException",
    ),
    (
        "java.lang.ReflectiveOperationException",
        "java.lang.Exception",
    ),
    (
        "java.lang.NoSuchMethodException",
        "java.lang.ReflectiveOperationException",
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
    match e.kind() {
        LuceneErrorKind::Io(io) => java_nio_file::java_class_of(io).to_string(),
        LuceneErrorKind::CorruptIndex(_) => "org.apache.lucene.index.CorruptIndexException".into(),
        LuceneErrorKind::IndexFormatTooOld { .. } => {
            "org.apache.lucene.index.IndexFormatTooOldException".into()
        }
        LuceneErrorKind::IndexFormatTooNew { .. } => {
            "org.apache.lucene.index.IndexFormatTooNewException".into()
        }
        LuceneErrorKind::IndexNotFound(_) => {
            "org.apache.lucene.index.IndexNotFoundException".into()
        }
        LuceneErrorKind::NotFound(_) => "java.io.FileNotFoundException".into(),
        LuceneErrorKind::IllegalArgument { .. } => "java.lang.IllegalArgumentException".into(),
        LuceneErrorKind::IllegalState { .. } => "java.lang.IllegalStateException".into(),
        LuceneErrorKind::LockObtainFailed { .. } => {
            "org.apache.lucene.store.LockObtainFailedException".into()
        }
        LuceneErrorKind::LockReleaseFailed { .. } => {
            "org.apache.lucene.store.LockReleaseFailedException".into()
        }
        LuceneErrorKind::AlreadyClosed { .. } => {
            "org.apache.lucene.store.AlreadyClosedException".into()
        }
        LuceneErrorKind::UnsupportedOperation { .. } => {
            "java.lang.UnsupportedOperationException".into()
        }
        LuceneErrorKind::StringIndexOutOfBounds(_) => {
            "java.lang.StringIndexOutOfBoundsException".into()
        }
        LuceneErrorKind::Arithmetic(_) => "java.lang.ArithmeticException".into(),
        LuceneErrorKind::AssertionError(_) => "java.lang.AssertionError".into(),
        LuceneErrorKind::Unchecked { class, .. } => class.to_string(),
        LuceneErrorKind::TooManyClauses(_) => {
            "org.apache.lucene.search.IndexSearcher$TooManyClauses".into()
        }
        LuceneErrorKind::TooManyNestedClauses(_) => {
            "org.apache.lucene.search.IndexSearcher$TooManyNestedClauses".into()
        }
        LuceneErrorKind::ExitingReader(_) => {
            "org.apache.lucene.index.ExitableDirectoryReader$ExitingReaderException".into()
        }
        LuceneErrorKind::CheckIndex { .. } => {
            "org.apache.lucene.index.CheckIndex$CheckIndexException".into()
        }
        LuceneErrorKind::TooComplexToDeterminize(_) => {
            "org.apache.lucene.util.automaton.TooComplexToDeterminizeException".into()
        }
        LuceneErrorKind::MaxBytesLengthExceeded(_) => {
            "org.apache.lucene.util.BytesRefHash$MaxBytesLengthExceededException".into()
        }
        LuceneErrorKind::Merge(_) => "org.apache.lucene.index.MergePolicy$MergeException".into(),
        LuceneErrorKind::MergeAborted(_) => {
            "org.apache.lucene.index.MergePolicy$MergeAbortedException".into()
        }
        LuceneErrorKind::NoSuchMethod(_) => "java.lang.NoSuchMethodException".into(),
        LuceneErrorKind::ThreadInterrupted(_) => {
            "org.apache.lucene.util.ThreadInterruptedException".into()
        }
        other => other.java_simple_name().to_string(),
    }
}

/// The Java throwable `e` stands for: its class, its message, and its
/// suppressed exceptions (`getSuppressed()`).
pub fn java_throwable(e: &LuceneError) -> Throwable {
    register_hierarchy();
    let (_, message) = crate::analysis::base_token_stream_test_case::java_class_and_message(e);
    let mut throwable = Throwable::new(java_class_name(e), message);
    // `getStackTrace()`: the frames captured when the error was created, each
    // function named as Java names the method (`check_max_clause_count` is
    // `checkMaxClauseCount`).
    let frames = e.get_stack_trace();
    if !frames.is_empty() {
        throwable.set_stack_trace(
            frames
                .iter()
                .map(
                    |frame| randomizedtesting::jdk::exception::StackTraceElement {
                        declaring_class: frame.path.clone(),
                        method_name: java_method_name(&frame.function),
                        file_name: frame.file.clone(),
                        line_number: frame.line,
                    },
                )
                .collect(),
        );
    }
    // `getCause()`: the kind's cause, held as an `Arc<LuceneError>`.
    if let Some(source) = std::error::Error::source(e) {
        let cause = source
            .downcast_ref::<std::sync::Arc<LuceneError>>()
            .map(|cause| &**cause)
            .or_else(|| source.downcast_ref::<LuceneError>());
        if let Some(cause) = cause {
            throwable.set_cause(java_throwable(cause));
        }
    }
    // `getSuppressed()` of an error that `addSuppressed` was called on.
    for suppressed in e.get_suppressed() {
        throwable.add_suppressed(java_throwable(suppressed));
    }
    // `getSuppressed()` of an `IOException` that carries suppressed ones.
    if let LuceneErrorKind::Io(io) = e.kind() {
        if let Some(exception) = rucene::store::java_nio_file::java_exception_of(io) {
            for suppressed in exception.get_suppressed() {
                throwable.add_suppressed(java_io_throwable(suppressed));
            }
        }
    }
    throwable
}

/// The Java method name of the Rust function `function`: snake_case becomes
/// lowerCamelCase (`check_max_clause_count` → `checkMaxClauseCount`).
pub fn java_method_name(function: &str) -> String {
    let mut name = String::with_capacity(function.len());
    let mut upper = false;
    for c in function.chars() {
        if c == '_' && !name.is_empty() {
            upper = true;
        } else if upper {
            name.extend(c.to_uppercase());
            upper = false;
        } else {
            name.push(c);
        }
    }
    name
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
