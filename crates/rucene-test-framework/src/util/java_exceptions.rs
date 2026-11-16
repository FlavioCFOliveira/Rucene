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
    // `java.util.IllformedLocaleException extends RuntimeException`: what
    // `Locale.Builder` throws for an ill-formed tag (`DateRecognizerFilterFactory`).
    (
        "java.util.IllformedLocaleException",
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
        LuceneErrorKind::CorruptIndex { .. } => {
            "org.apache.lucene.index.CorruptIndexException".into()
        }
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
        LuceneErrorKind::Throwable(throwable) => throwable.java_class().to_string(),
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
    // `getMessage()`: `null` stays `null` (`Throwable.toString()` then
    // prints the class name alone).
    let message = crate::analysis::base_token_stream_test_case::java_message(e);
    let mut throwable = Throwable::with_message(java_class_name(e), message);
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
    // `getCause()`: for an `IOException`, the cause the `io::Error` carries
    // (`new IOException(message, cause)`, read back with `java_cause_of`);
    // for any other kind, the kind's cause, held as an `Arc<LuceneError>`.
    let cause = match e.kind() {
        LuceneErrorKind::Io(io) => rucene::store::java_nio_file::java_cause_of(io),
        // A throwable of another crate (`ParserException(message, cause)`,
        // `ParseException`, ...) carries its cause behind `getCause()`.
        LuceneErrorKind::Throwable(t) => t.get_cause().map(|cause| &**cause),
        _ => std::error::Error::source(e).and_then(|source| {
            source
                .downcast_ref::<std::sync::Arc<LuceneError>>()
                .map(|cause| &**cause)
                .or_else(|| source.downcast_ref::<LuceneError>())
        }),
    };
    if let Some(cause) = cause {
        throwable.set_cause(java_throwable(cause));
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

/// Java's try-with-resources outcome over one resource: the body's
/// exception, with the exception closing the resource added to it as
/// suppressed (`Throwable.addSuppressed`); else the close's exception.
///
/// # Errors
///
/// The body's exception, else the close's.
pub fn try_with_resource<T>(body: Result<T>, closed: Result<()>) -> Result<T> {
    match body {
        Err(mut e) => {
            if let Err(close_error) = closed {
                e.add_suppressed(close_error);
            }
            Err(e)
        }
        Ok(value) => closed.map(|()| value),
    }
}

/// Java's `try (R r = resource) { body }`, panic-safe: `close` runs on every
/// exit of `body` (an `Err`, a panic, or success). A close failure is added
/// as suppressed to the body's throwable (`Throwable.addSuppressed`, on the
/// `Err` or on the panic's throwable, which is then rethrown); when the body
/// succeeded, the close failure alone is thrown.
///
/// `try (A a = ...; B b = ...) { body }` is `try (A a) { try (B b) { body }
/// }` in Java (resources close in reverse declaration order, each close
/// failure suppressed in that order), so N resources nest N calls.
///
/// # Errors
///
/// The body's error, else the close's.
pub fn try_with<R, T>(
    mut resource: R,
    close: impl FnOnce(&mut R) -> Result<()>,
    body: impl FnOnce(&mut R) -> Result<T>,
) -> Result<T> {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| body(&mut resource)));
    let closed = close(&mut resource);
    match outcome {
        Ok(result) => try_with_resource(result, closed),
        Err(payload) => match closed {
            Ok(()) => std::panic::resume_unwind(payload),
            Err(close_error) => {
                let mut t = Throwable::from_panic(payload);
                t.add_suppressed(java_throwable(&close_error));
                t.throw()
            }
        },
    }
}

/// `IOUtils.close(Closeable...)` over the outcomes of closing each object,
/// in argument order: every object is closed; the first exception is
/// rethrown with the later ones added to it as suppressed.
///
/// # Errors
///
/// The first close's exception.
pub fn io_utils_close<const N: usize>(closes: [Result<()>; N]) -> Result<()> {
    let mut th: Option<LuceneError> = None;
    for closed in closes {
        if let Err(t) = closed {
            match th.as_mut() {
                None => th = Some(t),
                Some(first) => first.add_suppressed(t),
            }
        }
    }
    th.map_or(Ok(()), Err)
}

/// The `Exception` a `catch (Exception e)` around a body catches, given the
/// body's outcome: its `Err`, or a panic whose throwable is not an `Error`;
/// a panic carrying an `Error` is rethrown unchanged.
pub fn caught_exception(outcome: std::thread::Result<Result<()>>) -> Option<Throwable> {
    match outcome {
        Ok(Ok(())) => None,
        Ok(Err(e)) => Some(java_throwable(&e)),
        Err(payload) => {
            if crate::mockfile::is_java_error(&*payload) {
                std::panic::resume_unwind(payload)
            }
            Some(crate::mockfile::panic_throwable(&*payload))
        }
    }
}

/// `try { value = body; } catch (Exception e)` over the body's outcome:
/// the value, or the `Exception` caught (an `Err`, or a panic whose
/// throwable is not an `Error`); an `Error`, as an `Err` or a panic, is
/// rethrown.
pub fn caught_value<T>(
    outcome: std::thread::Result<Result<T>>,
) -> std::result::Result<T, Throwable> {
    match outcome {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(e)) => {
            let t = java_throwable(&e);
            if t.is_instance(randomizedtesting::jdk::exception::ERROR) {
                t.throw()
            }
            Err(t)
        }
        Err(payload) => {
            if crate::mockfile::is_java_error(&*payload) {
                std::panic::resume_unwind(payload)
            }
            Err(crate::mockfile::panic_throwable(&*payload))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `new IOException(message, cause).getCause()` is `cause`
    /// (`IndexWriter.java:2259-2260` throws one for a failed background
    /// merge); an `IOException` without one has a `null` cause.
    #[test]
    fn an_io_exception_reports_the_cause_it_carries() {
        let cause = LuceneError::from(LuceneErrorKind::MergeAborted("merge is aborted: _0".into()));
        let e = LuceneError::from(LuceneErrorKind::Io(
            rucene::store::java_nio_file::io_exception_with_cause(
                "background merge hit exception: _0",
                Some(cause),
            ),
        ));
        let t = java_throwable(&e);
        assert_eq!(t.class_name(), "java.io.IOException");
        assert_eq!(t.message(), Some("background merge hit exception: _0"));
        let cause = t.cause().expect("getCause()");
        assert!(cause.is_instance("org.apache.lucene.index.MergePolicy$MergeAbortedException"));
        assert_eq!(cause.message(), Some("merge is aborted: _0"));

        let plain = LuceneError::from(LuceneErrorKind::Io(
            rucene::store::java_nio_file::io_exception("no cause"),
        ));
        assert!(java_throwable(&plain).cause().is_none());
    }

    /// The cause of any other kind is still its `Arc<LuceneError>` cause.
    #[test]
    fn an_unchecked_exception_reports_its_cause() {
        let e = LuceneError::illegal_state_with_cause(
            "outer",
            std::sync::Arc::new(LuceneError::illegal_argument("inner")),
        );
        let cause = java_throwable(&e).cause().cloned().expect("getCause()");
        assert_eq!(cause.class_name(), "java.lang.IllegalArgumentException");
    }

    /// A `null` message stays `null`: `Throwable.toString()` is the class
    /// name alone (`Throwable.java`, JDK 25 spec). The port gave the
    /// throwable the literal message `<null>`.
    #[test]
    fn a_null_message_stays_null() {
        let e = LuceneError::from(LuceneErrorKind::Unchecked {
            class: "java.lang.RuntimeException",
            message: None,
            cause: None,
        });
        let t = java_throwable(&e);
        assert_eq!(t.message(), None);
        assert_eq!(t.to_string(), "java.lang.RuntimeException");
    }

    /// An `assert` without a message (`java_assert!(cond)`, the core's
    /// `AssertionError(String::new())`) is `new AssertionError()`:
    /// `getMessage()` is `null`, not `""`; a message stays a message.
    #[test]
    fn a_bare_assertion_has_a_null_message() {
        let e = LuceneError::from(LuceneErrorKind::AssertionError(String::new()));
        assert_eq!(
            crate::analysis::base_token_stream_test_case::java_message(&e),
            None
        );
        let t = java_throwable(&e);
        assert_eq!(t.message(), None);
        assert_eq!(t.to_string(), "java.lang.AssertionError");
        let e = LuceneError::from(LuceneErrorKind::AssertionError("m".into()));
        assert_eq!(java_throwable(&e).message(), Some("m"));
    }

    /// `try { body } catch (Exception e)` over a value: an `Err` or a
    /// non-`Error` panic is caught; an `Error` (as an `Err` or a panic)
    /// propagates, as `catch (Exception)` never catches it.
    #[test]
    fn caught_value_lets_errors_through() {
        use std::panic::{catch_unwind, AssertUnwindSafe};
        let caught = caught_value::<()>(Ok(Err(LuceneError::illegal_state("ise"))));
        assert_eq!(
            caught.unwrap_err().class_name(),
            "java.lang.IllegalStateException"
        );
        let caught = caught_value::<()>(catch_unwind(|| {
            Throwable::new("java.lang.RuntimeException", "boom").throw()
        }));
        assert_eq!(caught.unwrap_err().message(), Some("boom"));
        let error = catch_unwind(AssertUnwindSafe(|| {
            caught_value::<()>(Ok(Err(LuceneError::from(LuceneErrorKind::AssertionError(
                "bad".to_string(),
            )))))
        }));
        let t = Throwable::from_panic(error.expect_err("an Error propagates"));
        assert_eq!(t.class_name(), "java.lang.AssertionError");
    }

    /// `try (R r = ...) { body }`: the resource closes when the body panics,
    /// and its close failure rides on the body's throwable as suppressed.
    #[test]
    fn try_with_closes_on_a_panic_and_suppresses_the_close_failure() {
        use std::panic::{catch_unwind, AssertUnwindSafe};
        let closed = std::sync::atomic::AtomicBool::new(false);
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            try_with(
                (),
                |_| {
                    closed.store(true, std::sync::atomic::Ordering::SeqCst);
                    Err(LuceneError::illegal_state("close"))
                },
                |_| -> Result<()> { Throwable::new("java.lang.RuntimeException", "boom").throw() },
            )
        }));
        assert!(closed.load(std::sync::atomic::Ordering::SeqCst));
        let t = Throwable::from_panic(outcome.expect_err("the body's throwable"));
        assert_eq!(t.message(), Some("boom"));
        assert_eq!(
            t.suppressed()[0].class_name(),
            "java.lang.IllegalStateException"
        );
    }
}
