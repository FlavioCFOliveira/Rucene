//! Error types used across Rucene.
//!
//! This module maps the most important Java Lucene exceptions to a Rust
//! [`Result`]-based error hierarchy. Where Java uses checked exceptions for
//! recoverable I/O or index corruption, Rucene returns [`LuceneError`].
//!
//! A [`LuceneError`] stands for one Java `Throwable`: its [`kind`] is the
//! exception class with that class's fields (message, cause, …), and it
//! holds the exceptions `Throwable.addSuppressed` attached to it, which
//! `getSuppressed()` returns. Suppressing an exception into an error never
//! changes its kind, as `addSuppressed` never changes a throwable's class.
//!
//! [`kind`]: LuceneError::kind

use std::sync::Arc;

use thiserror::Error;

/// The Java exception class a [`LuceneError`] stands for, with the fields
/// of that class.
#[derive(Error, Debug)]
pub enum LuceneErrorKind {
    /// I/O failure analogous to `java.io.IOException`.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// An index file is corrupt or inconsistent.
    ///
    /// Equivalent to `org.apache.lucene.index.CorruptIndexException`
    /// (`CorruptIndexException.java:25-70`), which keeps the composed
    /// `Throwable` message, the original message, the resource description
    /// and the cause. Build one with [`LuceneError::corrupt_index`] or
    /// [`LuceneError::corrupt_index_with_cause`];
    /// [`LuceneErrorKind::corrupt_index_message`] builds one from a message
    /// the caller composed, with no separate resource description.
    #[error("corrupt index: {message}")]
    CorruptIndex {
        /// The message, Java's `getMessage()`: `message + " (resource=" +
        /// resourceDescription + ")"` (`CorruptIndexException.java:58`).
        message: String,
        /// The message without the resource description, Java's
        /// `getOriginalMessage()` (`CorruptIndexException.java:67-70`).
        original_message: String,
        /// The description of the corrupted file, Java's
        /// `getResourceDescription()` (`CorruptIndexException.java:62-65`);
        /// `None` where the caller composed the message itself.
        resource_description: Option<String>,
        /// The cause, Java's `getCause()`.
        #[source]
        cause: Option<Arc<LuceneError>>,
    },

    /// An index file is too old, too new, or otherwise unsupported.
    #[error("index format not supported: {0}")]
    IndexFormatNotSupported(String),

    /// An index was written by a Lucene release this version can no longer read.
    ///
    /// Equivalent to `org.apache.lucene.index.IndexFormatTooOldException`. The two
    /// message shapes match the two Java constructors: one carries a free-form
    /// `reason` used when the version could not be read at all, the other carries
    /// the version triple.
    #[error("{}", format_too_old(.resource_description, .reason, .version, .min_version, .max_version))]
    IndexFormatTooOld {
        /// Describes the file that was too old.
        resource_description: String,
        /// Reason, when the version itself could not be read. Mutually exclusive
        /// with `version`.
        reason: Option<String>,
        /// The version found in the file.
        version: Option<i32>,
        /// The minimum version accepted.
        min_version: Option<i32>,
        /// The maximum version accepted.
        max_version: Option<i32>,
    },

    /// An index was written by a Lucene release newer than this one.
    ///
    /// Equivalent to `org.apache.lucene.index.IndexFormatTooNewException`.
    #[error("Format version is not supported (resource {resource_description}): {version} (needs to be between {min_version} and {max_version})")]
    IndexFormatTooNew {
        /// Describes the file that was too new.
        resource_description: String,
        /// The version found in the file.
        version: i32,
        /// The minimum version accepted.
        min_version: i32,
        /// The maximum version accepted.
        max_version: i32,
    },

    /// No index was found in the directory.
    ///
    /// Equivalent to `org.apache.lucene.index.IndexNotFoundException`, which Java
    /// derives from `FileNotFoundException`: the directory may simply be empty, but
    /// it may equally indicate corruption.
    #[error("{0}")]
    IndexNotFound(String),

    /// A requested document, field, or term was not found.
    #[error("not found: {0}")]
    NotFound(String),

    /// An invalid argument was supplied.
    ///
    /// Equivalent to `java.lang.IllegalArgumentException`, whose optional
    /// cause — for example the `IOException` reading the commit that the
    /// stale-reader check `IndexWriter` runs on an `IndexCommit` raised
    /// (`IndexWriter.java:1059-1064`) — is `cause`, which
    /// [`std::error::Error::source`] returns as Java's `getCause()` does.
    /// Build one with [`LuceneError::illegal_argument`] or
    /// [`LuceneError::illegal_argument_with_cause`].
    #[error("illegal argument: {message}")]
    IllegalArgument {
        /// The message, Java's `getMessage()`.
        message: String,
        /// The cause, Java's `getCause()`.
        #[source]
        cause: Option<Arc<LuceneError>>,
    },

    /// An illegal state was encountered (e.g., writing to a closed writer).
    ///
    /// Equivalent to `java.lang.IllegalStateException`, whose optional cause —
    /// for example the tragedy of `new IllegalStateException("this writer hit
    /// an unrecoverable error; cannot commit", tragedy.get())`
    /// (`IndexWriter.java:3760-3761`) — is `cause`, which
    /// [`std::error::Error::source`] returns as Java's `getCause()` does.
    /// Build one with [`LuceneError::illegal_state`] or
    /// [`LuceneError::illegal_state_with_cause`].
    #[error("illegal state: {message}")]
    IllegalState {
        /// The message, Java's `getMessage()`.
        message: String,
        /// The cause, Java's `getCause()`.
        #[source]
        cause: Option<Arc<LuceneError>>,
    },

    /// A query has more clauses than `IndexSearcher.getMaxClauseCount()`.
    ///
    /// Equivalent to `org.apache.lucene.search.IndexSearcher.TooManyClauses`,
    /// a `RuntimeException`. It displays exactly its message, Java's
    /// `getMessage()`.
    #[error("{0}")]
    TooManyClauses(crate::search::TooManyClauses),

    /// A query has more clauses than `IndexSearcher.getMaxClauseCount()`
    /// cumulatively across its children.
    ///
    /// Equivalent to
    /// `org.apache.lucene.search.IndexSearcher.TooManyNestedClauses`, which
    /// `extends TooManyClauses`; the Java class mapping keeps that
    /// relationship. It displays exactly its message.
    #[error("{0}")]
    TooManyNestedClauses(crate::search::TooManyNestedClauses),

    /// A `QueryTimeout` fired inside an `ExitableDirectoryReader`.
    ///
    /// Equivalent to
    /// `org.apache.lucene.index.ExitableDirectoryReader.ExitingReaderException`,
    /// a `RuntimeException`. It displays exactly its message.
    #[error("{0}")]
    ExitingReader(String),

    /// `CheckIndex` detected an integrity failure.
    ///
    /// Equivalent to `org.apache.lucene.index.CheckIndex.CheckIndexException`,
    /// a `RuntimeException` whose optional cause — the failure of a per-area
    /// check, `new CheckIndexException("<area> test failed", cause)` — is
    /// `cause`, which [`std::error::Error::source`] returns as Java's
    /// `getCause()` does. It displays exactly its message.
    #[error("{message}")]
    CheckIndex {
        /// The message, Java's `getMessage()`.
        message: String,
        /// The cause, Java's `getCause()`.
        #[source]
        cause: Option<Arc<LuceneError>>,
    },

    /// Determinizing an automaton would exceed its work limit.
    ///
    /// Equivalent to
    /// `org.apache.lucene.util.automaton.TooComplexToDeterminizeException`, a
    /// `RuntimeException`. It displays exactly its message.
    #[error("{0}")]
    TooComplexToDeterminize(Box<crate::util::automaton::TooComplexToDeterminizeException>),

    /// A value is longer than a `BytesRefHash` block can hold.
    ///
    /// Equivalent to
    /// `org.apache.lucene.util.BytesRefHash.MaxBytesLengthExceededException`,
    /// a `RuntimeException` (`bytes can be at most 32766 in length; got N`).
    /// It displays exactly its message.
    #[error("{0}")]
    MaxBytesLengthExceeded(String),

    /// A resource limit or security boundary was violated.
    #[error("resource limit exceeded: {0}")]
    ResourceLimit(String),

    /// An operation was cancelled by the caller or runtime.
    #[error("cancelled")]
    Cancelled,

    /// A lock could not be obtained.
    #[error("lock obtain failed: {message}")]
    LockObtainFailed {
        /// Human-readable reason the lock could not be obtained.
        message: String,
        /// Underlying I/O error, if any.
        #[source]
        source: Option<std::io::Error>,
    },

    /// A lock could not be released cleanly.
    #[error("lock release failed: {message}")]
    LockReleaseFailed {
        /// Human-readable reason the lock could not be released.
        message: String,
        /// Underlying I/O error, if any.
        #[source]
        source: Option<std::io::Error>,
    },

    /// A resource was closed when it was expected to remain open.
    ///
    /// Equivalent to `org.apache.lucene.store.AlreadyClosedException`, whose
    /// optional cause — for example the tragedy `IndexWriter.ensureOpen`
    /// attaches (`IndexWriter.java:917-921`) — is `cause`, which
    /// [`std::error::Error::source`] returns as Java's `getCause()` does.
    /// Build one with [`LuceneError::already_closed`] or
    /// [`LuceneError::already_closed_with_cause`].
    #[error("already closed: {message}")]
    AlreadyClosed {
        /// The message, Java's `getMessage()`.
        message: String,
        /// The cause, Java's `getCause()`.
        #[source]
        cause: Option<Arc<LuceneError>>,
    },

    /// An operation is not supported in the current context.
    ///
    /// Equivalent to `java.lang.UnsupportedOperationException`, whose
    /// optional cause — for example the `NoSuchMethodException`
    /// `getConstructor` threw, which `AnalysisSPILoader.newFactoryClassInstance`
    /// attaches for a factory class without a `Map<String,String>`
    /// constructor (`AnalysisSPILoader.java:206-211`) — is `cause`, which
    /// [`std::error::Error::source`] returns as Java's `getCause()` does.
    /// Build one with [`LuceneError::unsupported_operation`] or
    /// [`LuceneError::unsupported_operation_with_cause`].
    #[error("unsupported operation: {message}")]
    UnsupportedOperation {
        /// The message, Java's `getMessage()`.
        message: String,
        /// The cause, Java's `getCause()`.
        #[source]
        cause: Option<Arc<LuceneError>>,
    },

    /// An index into a string lies outside it.
    ///
    /// Equivalent to `java.lang.StringIndexOutOfBoundsException`, which
    /// `String.charAt` throws — for example `AbstractAnalysisFactory.requireChar`
    /// on an empty value (`AbstractAnalysisFactory.java:227-229`), whose
    /// message on JDK 25.0.4 is `Index 0 out of bounds for length 0`. The
    /// payload is the message, Java's `getMessage()`.
    #[error("string index out of bounds: {0}")]
    StringIndexOutOfBounds(String),

    /// A class has no method or constructor of the signature looked up.
    ///
    /// Equivalent to `java.lang.NoSuchMethodException`, which
    /// `Class.getConstructor` throws — for example the cause
    /// `AnalysisSPILoader.newFactoryClassInstance` attaches for a factory
    /// class without a `Map<String,String>` constructor
    /// (`AnalysisSPILoader.java:206-211`), whose message on JDK 25.0.4 is
    /// `<class>.<init>(java.util.Map)`. The payload is the message, Java's
    /// `getMessage()`.
    #[error("no such method: {0}")]
    NoSuchMethod(String),

    /// An arithmetic result does not fit the type it is computed in.
    ///
    /// Equivalent to `java.lang.ArithmeticException`. Lucene raises it through
    /// `Math.toIntExact(long)` — and `GroupVIntUtil.toInt(long)`, which throws
    /// the same exception explicitly — with the message `integer overflow`;
    /// build that one with [`LuceneError::integer_overflow`], or convert with
    /// [`to_int_exact`].
    #[error("arithmetic error: {0}")]
    Arithmetic(String),

    /// A merge was aborted before it completed.
    ///
    /// Equivalent to `org.apache.lucene.index.MergePolicy.MergeAbortedException`,
    /// which extends `IOException` and is thrown by `OneMerge.checkAborted()`
    /// once `IndexWriter.rollback` or `deleteAll` has stopped the merges. It
    /// displays exactly its message, Java's `getMessage()`.
    #[error("{0}")]
    MergeAborted(String),

    /// A merge failed.
    ///
    /// Equivalent to `org.apache.lucene.index.MergePolicy.MergeException`,
    /// which Lucene throws when a merge policy selects a segment the writer
    /// does not hold, or when a background merge fails.
    #[error("merge failed: {0}")]
    Merge(String),

    /// A failed Lucene `assert`, raised only in builds with
    /// `debug_assertions`, where this port runs the checks Java runs under
    /// `-ea`.
    ///
    /// Equivalent to `java.lang.AssertionError`. Java's `AssertionError` is an
    /// `Error`, which `IndexWriter` records as its tragic exception and closes
    /// on (for example `IndexWriter.java:3858-3863`); the writer does the same
    /// with this variant. A release build never raises it, as Java without
    /// `-ea` never throws `AssertionError` from an `assert`.
    #[error("assertion failed: {0}")]
    AssertionError(String),

    /// A wait was interrupted by `Thread.interrupt()`.
    ///
    /// Equivalent to `org.apache.lucene.util.ThreadInterruptedException`, the
    /// `RuntimeException` Lucene's waits throw when the `Object.wait` they
    /// block in throws `InterruptedException` (for example
    /// `IndexWriter.doWait()`, `IndexWriter.java:5647-5651`). It displays
    /// exactly its message, Java's `getMessage()`; see
    /// [`InterruptHandle`](crate::util::InterruptHandle) for how a thread is
    /// interrupted.
    #[error("{0}")]
    ThreadInterrupted(crate::util::ThreadInterruptedException),

    /// A generic wrapper for errors that do not yet have a dedicated variant.
    #[error("{0}")]
    Other(String),

    /// An unchecked Java exception that no other variant stands for, carried
    /// with its Java class: `class` is the binary name
    /// (`java.lang.IndexOutOfBoundsException`), `message` Java's
    /// `getMessage()` (`None` for `null`) and `cause` Java's `getCause()`.
    ///
    /// Equivalent to throwing `new <class>(message[, cause])`; it displays as
    /// Java's `Throwable.toString()`, the class then `": " + message`.
    #[error("{}", unchecked_to_string(class, message))]
    Unchecked {
        /// The Java binary class name.
        class: &'static str,
        /// `getMessage()`, `None` for `null`.
        message: Option<String>,
        /// `getCause()`.
        #[source]
        cause: Option<Arc<LuceneError>>,
    },
}

/// The binary name of `java.lang.NumberFormatException`, a subclass of
/// `IllegalArgumentException`.
pub const NUMBER_FORMAT_EXCEPTION: &str = "java.lang.NumberFormatException";

/// The JDK classes carried as [`LuceneErrorKind::Unchecked`] that extend
/// `java.lang.IllegalArgumentException`.
const ILLEGAL_ARGUMENT_SUBCLASSES: [&str; 6] = [
    NUMBER_FORMAT_EXCEPTION,
    "java.util.regex.PatternSyntaxException",
    "java.nio.charset.IllegalCharsetNameException",
    "java.nio.charset.UnsupportedCharsetException",
    "java.nio.file.InvalidPathException",
    "java.security.InvalidParameterException",
];

/// The top-level error type for Rucene operations: a Java `Throwable`.
///
/// [`kind`](Self::kind) is the Java exception class with its fields;
/// [`get_suppressed`](Self::get_suppressed) lists the exceptions
/// `Throwable.addSuppressed` attached to it. Build one from its kind with
/// [`LuceneError::from`] or with a named constructor
/// ([`LuceneError::illegal_state`], [`LuceneError::null_pointer`], …);
/// match on its kind with [`kind`](Self::kind), [`into_kind`](Self::into_kind)
/// or a struct pattern `LuceneError { kind: LuceneErrorKind::…, .. }`.
pub struct LuceneError {
    /// The Java exception class this error stands for, with its fields.
    pub kind: LuceneErrorKind,
    /// `getSuppressed()`, in the order the exceptions were added.
    suppressed: Vec<Arc<LuceneError>>,
    /// The stack at creation, Java's `fillInStackTrace()`: captured with
    /// [`Backtrace::capture`](std::backtrace::Backtrace::capture), which costs
    /// nothing unless `RUST_LIB_BACKTRACE` (or `RUST_BACKTRACE`) enables it.
    /// `None` when capture is disabled, so that no allocation is made then.
    backtrace: Option<Arc<std::backtrace::Backtrace>>,
}

/// One frame of [`LuceneError::get_stack_trace`]: Java's
/// `StackTraceElement`, with the Rust path in place of the class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackFrame {
    /// The function's path without its last segment, as the backtrace
    /// resolves it (`rucene::search::scoring_rewrite::...`).
    pub path: String,
    /// The function's own name, the last path segment
    /// (`check_max_clause_count`).
    pub function: String,
    /// The source file, when the backtrace resolved it.
    pub file: Option<String>,
    /// The line number, or `-1` when unknown, as `getLineNumber()` answers.
    pub line: i32,
}

impl From<LuceneErrorKind> for LuceneError {
    fn from(kind: LuceneErrorKind) -> Self {
        Self {
            kind,
            suppressed: Vec::new(),
            backtrace: {
                let backtrace = std::backtrace::Backtrace::capture();
                (backtrace.status() == std::backtrace::BacktraceStatus::Captured)
                    .then(|| Arc::new(backtrace))
            },
        }
    }
}

impl From<std::io::Error> for LuceneError {
    /// The `java.io.IOException` an I/O failure stands for:
    /// [`LuceneErrorKind::Io`].
    fn from(error: std::io::Error) -> Self {
        Self::from(LuceneErrorKind::Io(error))
    }
}

impl std::fmt::Display for LuceneError {
    /// The kind's display: suppressed exceptions do not change the message.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.kind, f)
    }
}

impl std::fmt::Debug for LuceneError {
    /// The kind's debug form, followed by the suppressed exceptions when
    /// there are any.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.suppressed.is_empty() {
            std::fmt::Debug::fmt(&self.kind, f)
        } else {
            f.debug_struct("LuceneError")
                .field("kind", &self.kind)
                .field("suppressed", &self.suppressed)
                .finish()
        }
    }
}

impl std::error::Error for LuceneError {
    /// The kind's cause, Java's `getCause()`: suppression does not change it.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        std::error::Error::source(&self.kind)
    }
}

impl LuceneError {
    /// The Java exception class this error stands for, with its fields.
    pub fn kind(&self) -> &LuceneErrorKind {
        &self.kind
    }

    /// `getStackTrace()`: the frames of the stack captured when the error was
    /// created, innermost first, empty when capture is disabled.
    ///
    /// Java's `fillInStackTrace` leaves out the frames of the throwable's own
    /// construction (`Throwable.<init>`, the subclass constructors); the
    /// counterpart here drops the leading frames of the backtrace machinery,
    /// of this module, and of the constructors and conversions that built the
    /// error (functions named `new`, `from` or `into`, and `with_*` builders).
    pub fn get_stack_trace(&self) -> Vec<StackFrame> {
        let Some(backtrace) = self.backtrace.as_ref() else {
            return Vec::new();
        };
        let frames = parse_backtrace(&format!("{backtrace:?}"));
        let construction = |frame: &StackFrame| {
            let path = frame.path.as_str();
            path.starts_with("std::")
                || path.starts_with("core::")
                || path.starts_with("alloc::")
                || path.starts_with("rucene::error")
                || path.starts_with("<rucene::error")
                || matches!(frame.function.as_str(), "new" | "from" | "into" | "capture")
                || frame.function.starts_with("with_")
        };
        frames.into_iter().skip_while(construction).collect()
    }

    /// The kind, by value; the suppressed exceptions are dropped.
    pub fn into_kind(self) -> LuceneErrorKind {
        self.kind
    }

    /// `Throwable.addSuppressed(exception)`: appends `exception` to the
    /// exceptions suppressed in order to deliver this one. The kind is
    /// unchanged.
    ///
    /// Java refuses to suppress a throwable into itself
    /// (`IllegalArgumentException("Self-suppression not permitted")`); a
    /// Rust value moved in here can never be `self`, so that check has
    /// nothing to refuse.
    pub fn add_suppressed(&mut self, exception: LuceneError) {
        self.add_suppressed_shared(Arc::new(exception));
    }

    /// [`add_suppressed`](Self::add_suppressed) of an exception that is
    /// shared, as Java suppresses the same object it may also keep.
    pub fn add_suppressed_shared(&mut self, exception: Arc<LuceneError>) {
        self.suppressed.push(exception);
    }

    /// `Throwable.getSuppressed()`: the exceptions suppressed into this one,
    /// in the order they were added; empty when there are none.
    pub fn get_suppressed(&self) -> &[Arc<LuceneError>] {
        &self.suppressed
    }
}

/// `Throwable.toString()` of an [`LuceneErrorKind::Unchecked`].
fn unchecked_to_string(class: &str, message: &Option<String>) -> String {
    match message {
        Some(message) => format!("{class}: {message}"),
        None => class.to_string(),
    }
}

impl LuceneError {
    /// The simple name of the Java exception class this error stands for:
    /// [`LuceneErrorKind::java_simple_name`] of its kind.
    pub fn java_simple_name(&self) -> &'static str {
        self.kind.java_simple_name()
    }

    /// Whether the Java class this error stands for is a `java.io.IOException`:
    /// [`LuceneErrorKind::is_io_exception`] of its kind.
    pub fn is_io_exception(&self) -> bool {
        self.kind.is_io_exception()
    }

    /// Whether the Java class this error stands for is a
    /// `java.lang.IllegalArgumentException`: what a `catch
    /// (IllegalArgumentException e)` clause catches.
    /// [`LuceneErrorKind::is_illegal_argument_exception`] of its kind.
    pub fn is_illegal_argument_exception(&self) -> bool {
        self.kind.is_illegal_argument_exception()
    }

    /// Whether this error is a `java.lang.NumberFormatException`: what a
    /// `catch (NumberFormatException e)` clause catches.
    pub fn is_number_format_exception(&self) -> bool {
        matches!(
            &self.kind,
            LuceneErrorKind::Unchecked { class, .. } if *class == NUMBER_FORMAT_EXCEPTION
        )
    }

    /// `new NumberFormatException(message)`: a [`LuceneErrorKind::Unchecked`]
    /// of class `java.lang.NumberFormatException`, which extends
    /// `IllegalArgumentException`, so
    /// [`is_illegal_argument_exception`](Self::is_illegal_argument_exception)
    /// answers `true` for it.
    pub fn number_format(message: impl Into<String>) -> Self {
        Self::unchecked(NUMBER_FORMAT_EXCEPTION, message)
    }

    /// `NumberFormatException.forInputString(s, radix)`, which
    /// `Integer.parseInt` and `Long.parseLong` throw: `For input string:
    /// "<s>"`, followed by ` under radix <radix>` when the radix is not 10.
    pub fn number_format_for_input_string(s: &str, radix: u32) -> Self {
        Self::number_format(if radix == 10 {
            format!("For input string: \"{s}\"")
        } else {
            format!("For input string: \"{s}\" under radix {radix}")
        })
    }

    /// Returns an equal error: [`LuceneErrorKind::duplicate`] of its kind,
    /// with the same suppressed exceptions.
    pub fn duplicate(&self) -> Self {
        Self {
            kind: self.kind.duplicate(),
            suppressed: self.suppressed.clone(),
            backtrace: self.backtrace.clone(),
        }
    }

    /// `new IllegalStateException(String)`.
    pub fn illegal_state(message: impl Into<String>) -> Self {
        Self::from(LuceneErrorKind::IllegalState {
            message: message.into(),
            cause: None,
        })
    }

    /// `new IllegalStateException(String, Throwable)`.
    pub fn illegal_state_with_cause(message: impl Into<String>, cause: Arc<LuceneError>) -> Self {
        Self::from(LuceneErrorKind::IllegalState {
            message: message.into(),
            cause: Some(cause),
        })
    }

    /// `new IllegalArgumentException(String)`.
    pub fn illegal_argument(message: impl Into<String>) -> Self {
        Self::from(LuceneErrorKind::IllegalArgument {
            message: message.into(),
            cause: None,
        })
    }

    /// `new IllegalArgumentException(String, Throwable)`.
    pub fn illegal_argument_with_cause(
        message: impl Into<String>,
        cause: Arc<LuceneError>,
    ) -> Self {
        Self::from(LuceneErrorKind::IllegalArgument {
            message: message.into(),
            cause: Some(cause),
        })
    }

    /// `new UnsupportedOperationException(String)`.
    pub fn unsupported_operation(message: impl Into<String>) -> Self {
        Self::from(LuceneErrorKind::UnsupportedOperation {
            message: message.into(),
            cause: None,
        })
    }

    /// `new UnsupportedOperationException(String, Throwable)`.
    pub fn unsupported_operation_with_cause(
        message: impl Into<String>,
        cause: Arc<LuceneError>,
    ) -> Self {
        Self::from(LuceneErrorKind::UnsupportedOperation {
            message: message.into(),
            cause: Some(cause),
        })
    }

    /// `new ArithmeticException("integer overflow")`, what `Math.toIntExact`
    /// throws for a `long` outside the `int` range.
    pub fn integer_overflow() -> Self {
        Self::from(LuceneErrorKind::Arithmetic("integer overflow".to_string()))
    }

    /// `new ThreadInterruptedException(ie)`, what Lucene's waits throw when
    /// `Object.wait` throws `InterruptedException`.
    pub fn thread_interrupted(ie: crate::util::InterruptedException) -> Self {
        Self::from(LuceneErrorKind::ThreadInterrupted(
            crate::util::ThreadInterruptedException::new(ie),
        ))
    }

    /// `new AlreadyClosedException(String)`.
    pub fn already_closed(message: impl Into<String>) -> Self {
        Self::from(LuceneErrorKind::AlreadyClosed {
            message: message.into(),
            cause: None,
        })
    }

    /// `new AlreadyClosedException(String, Throwable)`.
    pub fn already_closed_with_cause(message: impl Into<String>, cause: Arc<LuceneError>) -> Self {
        Self::from(LuceneErrorKind::AlreadyClosed {
            message: message.into(),
            cause: Some(cause),
        })
    }
}

impl LuceneErrorKind {
    /// The simple name of the Java exception class this error stands for, what
    /// `getClass().getSimpleName()` prints.
    ///
    /// **Divergence from Lucene 10.5.0.** [`LuceneErrorKind::IndexFormatNotSupported`],
    /// [`LuceneErrorKind::NotFound`], [`LuceneErrorKind::ResourceLimit`],
    /// [`LuceneErrorKind::Cancelled`] and [`LuceneErrorKind::Other`] are this port's
    /// own variants, each standing for several Java classes, so no single
    /// Java name exists for them; they report the variant's name.
    pub fn java_simple_name(&self) -> &'static str {
        match self {
            Self::Io(_) => "IOException",
            Self::CorruptIndex { .. } => "CorruptIndexException",
            Self::IndexFormatTooOld { .. } => "IndexFormatTooOldException",
            Self::IndexFormatTooNew { .. } => "IndexFormatTooNewException",
            Self::IndexNotFound(_) => "IndexNotFoundException",
            Self::IllegalArgument { .. } => "IllegalArgumentException",
            Self::IllegalState { .. } => "IllegalStateException",
            Self::LockObtainFailed { .. } => "LockObtainFailedException",
            Self::LockReleaseFailed { .. } => "LockReleaseFailedException",
            Self::AlreadyClosed { .. } => "AlreadyClosedException",
            Self::UnsupportedOperation { .. } => "UnsupportedOperationException",
            Self::StringIndexOutOfBounds(_) => "StringIndexOutOfBoundsException",
            Self::NoSuchMethod(_) => "NoSuchMethodException",
            Self::Arithmetic(_) => "ArithmeticException",
            Self::MergeAborted(_) => "MergeAbortedException",
            Self::Merge(_) => "MergeException",
            Self::AssertionError(_) => "AssertionError",
            Self::ThreadInterrupted(_) => "ThreadInterruptedException",
            Self::IndexFormatNotSupported(_) => "IndexFormatNotSupported",
            Self::NotFound(_) => "NotFound",
            Self::ResourceLimit(_) => "ResourceLimit",
            Self::TooManyClauses(_) => "TooManyClauses",
            Self::TooManyNestedClauses(_) => "TooManyNestedClauses",
            Self::ExitingReader(_) => "ExitingReaderException",
            Self::CheckIndex { .. } => "CheckIndexException",
            Self::TooComplexToDeterminize(_) => "TooComplexToDeterminizeException",
            Self::MaxBytesLengthExceeded(_) => "MaxBytesLengthExceededException",
            Self::Cancelled => "Cancelled",
            Self::Other(_) => "Other",
            Self::Unchecked { class, .. } => class.rsplit('.').next().unwrap_or(class),
        }
    }

    /// Whether the Java class this error stands for is a `java.io.IOException`
    /// — what a `catch (IOException e)` clause catches.
    ///
    /// True for [`LuceneErrorKind::Io`] and for the variants whose Java classes
    /// extend `IOException`: `CorruptIndexException` and its
    /// `IndexFormatTooOldException` and `IndexFormatTooNewException`,
    /// `IndexNotFoundException` (a `FileNotFoundException`),
    /// [`LuceneErrorKind::NotFound`] (`FileNotFoundException`),
    /// `LockObtainFailedException`, `LockReleaseFailedException` and
    /// `MergePolicy.MergeAbortedException`.
    pub fn is_io_exception(&self) -> bool {
        matches!(
            self,
            Self::Io(_)
                | Self::CorruptIndex { .. }
                | Self::IndexFormatTooOld { .. }
                | Self::IndexFormatTooNew { .. }
                | Self::IndexNotFound(_)
                | Self::NotFound(_)
                | Self::LockObtainFailed { .. }
                | Self::LockReleaseFailed { .. }
                | Self::MergeAborted(_)
        )
    }

    /// Whether the Java class this error stands for is a
    /// `java.lang.IllegalArgumentException` — what a `catch
    /// (IllegalArgumentException e)` clause catches.
    ///
    /// True for [`LuceneErrorKind::IllegalArgument`] and for the
    /// [`LuceneErrorKind::Unchecked`] classes of the JDK that extend it:
    /// `NumberFormatException`, `java.util.regex.PatternSyntaxException`,
    /// `java.nio.charset.IllegalCharsetNameException`,
    /// `java.nio.charset.UnsupportedCharsetException`,
    /// `java.nio.file.InvalidPathException` and
    /// `java.security.InvalidParameterException`.
    pub fn is_illegal_argument_exception(&self) -> bool {
        match self {
            Self::IllegalArgument { .. } => true,
            Self::Unchecked { class, .. } => ILLEGAL_ARGUMENT_SUBCLASSES.contains(class),
            _ => false,
        }
    }

    /// Returns an equal error: the same variant, message and fields, and the
    /// same cause.
    ///
    /// Used where Java keeps a reference to a `Throwable` that is also thrown,
    /// such as `IndexWriter.tragedy` (`IndexWriter.java:5847-5859`): a
    /// [`LuceneError`] has a single owner, so the writer keeps this copy.
    /// [`LuceneErrorKind::Io`] and the lock variants' I/O sources are rebuilt from
    /// their kind and rendered message, which is all a `java.io.IOException`
    /// carries besides its own cause.
    pub fn duplicate(&self) -> Self {
        fn io(error: &std::io::Error) -> std::io::Error {
            std::io::Error::new(error.kind(), error.to_string())
        }
        match self {
            Self::Io(error) => Self::Io(io(error)),
            Self::CorruptIndex {
                message,
                original_message,
                resource_description,
                cause,
            } => Self::CorruptIndex {
                message: message.clone(),
                original_message: original_message.clone(),
                resource_description: resource_description.clone(),
                cause: cause.clone(),
            },
            Self::IndexFormatNotSupported(m) => Self::IndexFormatNotSupported(m.clone()),
            Self::IndexFormatTooOld {
                resource_description,
                reason,
                version,
                min_version,
                max_version,
            } => Self::IndexFormatTooOld {
                resource_description: resource_description.clone(),
                reason: reason.clone(),
                version: *version,
                min_version: *min_version,
                max_version: *max_version,
            },
            Self::IndexFormatTooNew {
                resource_description,
                version,
                min_version,
                max_version,
            } => Self::IndexFormatTooNew {
                resource_description: resource_description.clone(),
                version: *version,
                min_version: *min_version,
                max_version: *max_version,
            },
            Self::IndexNotFound(m) => Self::IndexNotFound(m.clone()),
            Self::NotFound(m) => Self::NotFound(m.clone()),
            Self::IllegalArgument { message, cause } => Self::IllegalArgument {
                message: message.clone(),
                cause: cause.clone(),
            },
            Self::IllegalState { message, cause } => Self::IllegalState {
                message: message.clone(),
                cause: cause.clone(),
            },
            Self::ResourceLimit(m) => Self::ResourceLimit(m.clone()),
            Self::TooManyClauses(e) => Self::TooManyClauses(e.clone()),
            Self::TooManyNestedClauses(e) => Self::TooManyNestedClauses(e.clone()),
            Self::ExitingReader(m) => Self::ExitingReader(m.clone()),
            Self::CheckIndex { message, cause } => Self::CheckIndex {
                message: message.clone(),
                cause: cause.clone(),
            },
            Self::TooComplexToDeterminize(e) => Self::TooComplexToDeterminize(e.clone()),
            Self::MaxBytesLengthExceeded(m) => Self::MaxBytesLengthExceeded(m.clone()),
            Self::Cancelled => Self::Cancelled,
            Self::LockObtainFailed { message, source } => Self::LockObtainFailed {
                message: message.clone(),
                source: source.as_ref().map(io),
            },
            Self::LockReleaseFailed { message, source } => Self::LockReleaseFailed {
                message: message.clone(),
                source: source.as_ref().map(io),
            },
            Self::AlreadyClosed { message, cause } => Self::AlreadyClosed {
                message: message.clone(),
                cause: cause.clone(),
            },
            Self::UnsupportedOperation { message, cause } => Self::UnsupportedOperation {
                message: message.clone(),
                cause: cause.clone(),
            },
            Self::StringIndexOutOfBounds(m) => Self::StringIndexOutOfBounds(m.clone()),
            Self::NoSuchMethod(m) => Self::NoSuchMethod(m.clone()),
            Self::Arithmetic(m) => Self::Arithmetic(m.clone()),
            Self::MergeAborted(m) => Self::MergeAborted(m.clone()),
            Self::Merge(m) => Self::Merge(m.clone()),
            Self::AssertionError(m) => Self::AssertionError(m.clone()),
            Self::ThreadInterrupted(e) => Self::ThreadInterrupted(e.clone()),
            Self::Other(m) => Self::Other(m.clone()),
            Self::Unchecked {
                class,
                message,
                cause,
            } => Self::Unchecked {
                class,
                message: message.clone(),
                cause: cause.clone(),
            },
        }
    }

    /// A `CorruptIndex` whose message the caller composed itself: the message
    /// is both `getMessage()` and `getOriginalMessage()`, with no resource
    /// description and no cause.
    pub fn corrupt_index_message(message: impl Into<String>) -> Self {
        let message = message.into();
        Self::CorruptIndex {
            original_message: message.clone(),
            message,
            resource_description: None,
            cause: None,
        }
    }

    /// `Throwable.toString()` of the `IOException` this error stands for.
    fn io_exception_to_string(&self) -> String {
        match self {
            Self::Io(e) => crate::store::java_nio_file::java_to_string(e),
            Self::CorruptIndex { message, .. } => {
                format!("org.apache.lucene.index.CorruptIndexException: {message}")
            }
            Self::IndexFormatTooOld { .. } => {
                format!("org.apache.lucene.index.IndexFormatTooOldException: {self}")
            }
            Self::IndexFormatTooNew { .. } => {
                format!("org.apache.lucene.index.IndexFormatTooNewException: {self}")
            }
            Self::IndexNotFound(m) => {
                format!("org.apache.lucene.index.IndexNotFoundException: {m}")
            }
            Self::LockObtainFailed { message, .. } => {
                format!("org.apache.lucene.store.LockObtainFailedException: {message}")
            }
            Self::LockReleaseFailed { message, .. } => {
                format!("org.apache.lucene.store.LockReleaseFailedException: {message}")
            }
            Self::MergeAborted(m) => {
                format!("org.apache.lucene.index.MergePolicy$MergeAbortedException: {m}")
            }
            Self::Unchecked { class, message, .. } => unchecked_to_string(class, message),
            other => format!("{}: {other}", other.java_simple_name()),
        }
    }
}

/// Renders `IndexFormatTooOld` exactly as Lucene 10.5.0 does, across both of its
/// constructor shapes.
fn format_too_old(
    resource_description: &str,
    reason: &Option<String>,
    version: &Option<i32>,
    min_version: &Option<i32>,
    max_version: &Option<i32>,
) -> String {
    match reason {
        Some(reason) => format!(
            "Format version is not supported (resource {resource_description}): {reason}. \
             This version of Lucene only supports indexes created with release {}.0 and later by default.",
            crate::util::Version::MIN_SUPPORTED_MAJOR
        ),
        None => format!(
            "Format version is not supported (resource {resource_description}): {} \
             (needs to be between {} and {}). This version of Lucene only supports indexes \
             created with release {}.0 and later.",
            version.unwrap_or_default(),
            min_version.unwrap_or_default(),
            max_version.unwrap_or_default(),
            crate::util::Version::MIN_SUPPORTED_MAJOR
        ),
    }
}

impl LuceneError {
    /// `new RuntimeException(cause)`: an [`LuceneErrorKind::Unchecked`]
    /// `java.lang.RuntimeException` whose message is the cause's
    /// `Throwable.toString()`, as `RuntimeException(Throwable)` sets it.
    ///
    /// Lucene builds it where a method that cannot throw a checked exception
    /// catches an `IOException` (for example `SimpleTextDocValuesReader`'s
    /// value functions), so `cause` is one of the variants
    /// [`is_io_exception`](Self::is_io_exception) accepts; any other is
    /// described by [`java_simple_name`](Self::java_simple_name) and its
    /// display.
    pub fn runtime_exception(cause: LuceneError) -> Self {
        let message = cause.kind.io_exception_to_string();
        Self::from(LuceneErrorKind::Unchecked {
            class: "java.lang.RuntimeException",
            message: Some(message),
            cause: Some(Arc::new(cause)),
        })
    }

    /// `new UncheckedIOException(cause)`: an [`LuceneErrorKind::Unchecked`]
    /// `java.io.UncheckedIOException` whose message is the cause's
    /// `Throwable.toString()`, as `UncheckedIOException(IOException)` sets it
    /// through `RuntimeException(Throwable)`.
    ///
    /// `cause` is one of the variants [`is_io_exception`](Self::is_io_exception)
    /// accepts.
    pub fn unchecked_io_exception(cause: LuceneError) -> Self {
        let message = cause.kind.io_exception_to_string();
        Self::from(LuceneErrorKind::Unchecked {
            class: "java.io.UncheckedIOException",
            message: Some(message),
            cause: Some(Arc::new(cause)),
        })
    }

    /// The `NullPointerException` the JVM raises when Java code dereferences
    /// `null` (or `Objects.requireNonNull` rejects it): a
    /// [`LuceneErrorKind::Unchecked`] of class `java.lang.NullPointerException`.
    /// The JVM computes a helpful message; the port keeps none.
    pub fn null_pointer() -> Self {
        Self::from(LuceneErrorKind::Unchecked {
            class: "java.lang.NullPointerException",
            message: None,
            cause: None,
        })
    }

    /// The `NullPointerException` the JVM raises with a message: its helpful
    /// message for a dereference of `null` (`Cannot invoke "…" because "…" is
    /// null`), or the message `Objects.requireNonNull(obj, message)` was
    /// given. A [`LuceneErrorKind::Unchecked`] of class
    /// `java.lang.NullPointerException`, the one representation of Java's
    /// `NullPointerException` in this crate.
    pub fn null_pointer_with_message(message: impl Into<String>) -> Self {
        Self::from(LuceneErrorKind::Unchecked {
            class: "java.lang.NullPointerException",
            message: Some(message.into()),
            cause: None,
        })
    }

    /// `new <class>(message)` for an unchecked Java exception no other
    /// variant stands for: [`LuceneErrorKind::Unchecked`] without a cause.
    pub fn unchecked(class: &'static str, message: impl Into<String>) -> Self {
        Self::from(LuceneErrorKind::Unchecked {
            class,
            message: Some(message.into()),
            cause: None,
        })
    }

    /// Creates a `CorruptIndex` error composing the message the way
    /// `org.apache.lucene.index.CorruptIndexException` does: the caller's message
    /// followed by ` (resource=<description>)`.
    ///
    /// Equivalent to `new CorruptIndexException(String, String)` and, with
    /// `Objects.toString(input)` as the description, to the `DataInput` and
    /// `DataOutput` forms (`CorruptIndexException.java:31-54`).
    pub fn corrupt_index(message: impl AsRef<str>, resource_description: impl AsRef<str>) -> Self {
        Self::from(LuceneErrorKind::CorruptIndex {
            message: format!(
                "{} (resource={})",
                message.as_ref(),
                resource_description.as_ref()
            ),
            original_message: message.as_ref().to_string(),
            resource_description: Some(resource_description.as_ref().to_string()),
            cause: None,
        })
    }

    /// Creates a `CorruptIndex` error with a cause.
    ///
    /// Equivalent to `new CorruptIndexException(String, String, Throwable)`
    /// and, with `Objects.toString(input)` as the description, to the
    /// `DataInput` and `DataOutput` forms with a cause
    /// (`CorruptIndexException.java:41-60`).
    pub fn corrupt_index_with_cause(
        message: impl AsRef<str>,
        resource_description: impl AsRef<str>,
        cause: LuceneError,
    ) -> Self {
        Self::from(LuceneErrorKind::CorruptIndex {
            message: format!(
                "{} (resource={})",
                message.as_ref(),
                resource_description.as_ref()
            ),
            original_message: message.as_ref().to_string(),
            resource_description: Some(resource_description.as_ref().to_string()),
            cause: Some(Arc::new(cause)),
        })
    }

    /// Returns whether Java would throw `self` as an `EOFException`, a
    /// `NoSuchFileException` or a `FileNotFoundException` (or a subclass):
    /// what a `catch (EOFException | NoSuchFileException |
    /// FileNotFoundException e)` clause catches.
    ///
    /// Those are an [`LuceneErrorKind::Io`] of kind
    /// [`std::io::ErrorKind::UnexpectedEof`] or [`std::io::ErrorKind::NotFound`]
    /// (the kind `NoSuchFileException` maps to), the `FileNotFoundException`
    /// [`LuceneErrorKind::NotFound`], and its subclass
    /// `IndexNotFoundException`.
    pub fn is_eof_or_missing_file(&self) -> bool {
        match &self.kind {
            LuceneErrorKind::Io(error) => matches!(
                error.kind(),
                std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::NotFound
            ),
            LuceneErrorKind::NotFound(_) | LuceneErrorKind::IndexNotFound(_) => true,
            _ => false,
        }
    }

    /// Returns the message of a `CorruptIndex` without its resource
    /// description.
    ///
    /// Equivalent to `org.apache.lucene.index.CorruptIndexException#getOriginalMessage`
    /// (`CorruptIndexException.java:67-70`); `None` for every other error.
    pub fn original_message(&self) -> Option<&str> {
        match &self.kind {
            LuceneErrorKind::CorruptIndex {
                original_message, ..
            } => Some(original_message),
            _ => None,
        }
    }

    /// Creates an `IndexFormatTooOld` error from a free-form reason, for when the
    /// version could not be read from the file at all.
    pub fn index_format_too_old_reason(
        resource_description: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self::from(LuceneErrorKind::IndexFormatTooOld {
            resource_description: resource_description.into(),
            reason: Some(reason.into()),
            version: None,
            min_version: None,
            max_version: None,
        })
    }

    /// Creates an `IndexFormatTooOld` error from the version triple read off the file.
    pub fn index_format_too_old(
        resource_description: impl Into<String>,
        version: i32,
        min_version: i32,
        max_version: i32,
    ) -> Self {
        Self::from(LuceneErrorKind::IndexFormatTooOld {
            resource_description: resource_description.into(),
            reason: None,
            version: Some(version),
            min_version: Some(min_version),
            max_version: Some(max_version),
        })
    }

    /// Creates an `IndexFormatTooNew` error.
    pub fn index_format_too_new(
        resource_description: impl Into<String>,
        version: i32,
        min_version: i32,
        max_version: i32,
    ) -> Self {
        Self::from(LuceneErrorKind::IndexFormatTooNew {
            resource_description: resource_description.into(),
            version,
            min_version,
            max_version,
        })
    }

    /// Returns a description of the file that was corrupt, too old or too
    /// new, when this error is a `CorruptIndex`, an `IndexFormatTooOld` or an
    /// `IndexFormatTooNew`.
    ///
    /// Equivalent to `org.apache.lucene.index.CorruptIndexException#getResourceDescription`
    /// (`CorruptIndexException.java:62-65`),
    /// `org.apache.lucene.index.IndexFormatTooOldException#getResourceDescription`
    /// and `org.apache.lucene.index.IndexFormatTooNewException#getResourceDescription`.
    /// A `CorruptIndex` whose message the caller composed itself
    /// ([`LuceneErrorKind::corrupt_index_message`]) has no separate
    /// description and returns `None`.
    pub fn resource_description(&self) -> Option<&str> {
        match &self.kind {
            LuceneErrorKind::CorruptIndex {
                resource_description,
                ..
            } => resource_description.as_deref(),
            LuceneErrorKind::IndexFormatTooOld {
                resource_description,
                ..
            }
            | LuceneErrorKind::IndexFormatTooNew {
                resource_description,
                ..
            } => Some(resource_description),
            _ => None,
        }
    }

    /// Returns the reason an `IndexFormatTooOld` carries when the version
    /// itself could not be read off the file.
    ///
    /// Equivalent to `org.apache.lucene.index.IndexFormatTooOldException#getReason`,
    /// which returns `null` when the version *was* available; this returns
    /// `None` in that case, and `None` for every other error.
    pub fn reason(&self) -> Option<&str> {
        match &self.kind {
            LuceneErrorKind::IndexFormatTooOld { reason, .. } => reason.as_deref(),
            _ => None,
        }
    }

    /// Returns the version of the file that was too old or too new.
    ///
    /// For `IndexFormatTooOld`, returns `None` when only a `reason` is
    /// available, matching Java's `Integer` (nullable) return; for
    /// `IndexFormatTooNew`, whose Java field is a primitive `int`, this is
    /// always `Some`.
    ///
    /// Equivalent to `org.apache.lucene.index.IndexFormatTooOldException#getVersion`
    /// and `org.apache.lucene.index.IndexFormatTooNewException#getVersion`.
    pub fn version(&self) -> Option<i32> {
        match &self.kind {
            LuceneErrorKind::IndexFormatTooOld { version, .. } => *version,
            LuceneErrorKind::IndexFormatTooNew { version, .. } => Some(*version),
            _ => None,
        }
    }

    /// Returns the minimum version accepted, when this error is an
    /// `IndexFormatTooOld` or `IndexFormatTooNew`.
    ///
    /// See [`version`](Self::version) for the nullability of the two variants.
    ///
    /// Equivalent to `org.apache.lucene.index.IndexFormatTooOldException#getMinVersion`
    /// and `org.apache.lucene.index.IndexFormatTooNewException#getMinVersion`.
    pub fn min_version(&self) -> Option<i32> {
        match &self.kind {
            LuceneErrorKind::IndexFormatTooOld { min_version, .. } => *min_version,
            LuceneErrorKind::IndexFormatTooNew { min_version, .. } => Some(*min_version),
            _ => None,
        }
    }

    /// Returns the maximum version accepted, when this error is an
    /// `IndexFormatTooOld` or `IndexFormatTooNew`.
    ///
    /// See [`version`](Self::version) for the nullability of the two variants.
    ///
    /// Equivalent to `org.apache.lucene.index.IndexFormatTooOldException#getMaxVersion`
    /// and `org.apache.lucene.index.IndexFormatTooNewException#getMaxVersion`.
    pub fn max_version(&self) -> Option<i32> {
        match &self.kind {
            LuceneErrorKind::IndexFormatTooOld { max_version, .. } => *max_version,
            LuceneErrorKind::IndexFormatTooNew { max_version, .. } => Some(*max_version),
            _ => None,
        }
    }

    /// Creates an `IndexNotFound` error.
    pub fn index_not_found(message: impl Into<String>) -> Self {
        Self::from(LuceneErrorKind::IndexNotFound(message.into()))
    }

    /// Creates a `LockObtainFailed` error without an underlying I/O cause.
    pub fn lock_obtain_failed(message: impl Into<String>) -> Self {
        Self::from(LuceneErrorKind::LockObtainFailed {
            message: message.into(),
            source: None,
        })
    }

    /// Creates a `LockObtainFailed` error with an underlying I/O cause.
    pub fn lock_obtain_failed_with_source(
        message: impl Into<String>,
        source: std::io::Error,
    ) -> Self {
        Self::from(LuceneErrorKind::LockObtainFailed {
            message: message.into(),
            source: Some(source),
        })
    }

    /// Creates a `LockReleaseFailed` error without an underlying I/O cause.
    pub fn lock_release_failed(message: impl Into<String>) -> Self {
        Self::from(LuceneErrorKind::LockReleaseFailed {
            message: message.into(),
            source: None,
        })
    }

    /// Creates a `LockReleaseFailed` error with an underlying I/O cause.
    pub fn lock_release_failed_with_source(
        message: impl Into<String>,
        source: std::io::Error,
    ) -> Self {
        Self::from(LuceneErrorKind::LockReleaseFailed {
            message: message.into(),
            source: Some(source),
        })
    }
}

/// Convenience alias for `Result<T, LuceneError>`.
pub type Result<T> = std::result::Result<T, LuceneError>;

/// Narrows a `long` to an `int`, failing when it does not fit.
///
/// Equivalent to `java.lang.Math.toIntExact(long)`, which Lucene calls to
/// narrow its `long` arithmetic back to an `int`.
///
/// # Errors
///
/// [`LuceneErrorKind::Arithmetic`] with the message `integer overflow`, built by
/// [`LuceneError::integer_overflow`], when `value` is outside the `i32` range:
/// Java's `ArithmeticException("integer overflow")`.
pub fn to_int_exact(value: i64) -> Result<i32> {
    i32::try_from(value).map_err(|_| LuceneError::integer_overflow())
}

/// Parses the `Debug` form of a captured [`std::backtrace::Backtrace`], a list
/// of `{ fn: "...", file: "...", line: N }` entries; the frames API is not
/// stable, and the `Debug` form is its documented textual rendering.
fn parse_backtrace(text: &str) -> Vec<StackFrame> {
    fn quoted_after<'a>(entry: &'a str, key: &str) -> Option<&'a str> {
        let start = entry.find(key)? + key.len();
        let rest = &entry[start..];
        let rest = rest.strip_prefix('"')?;
        let end = rest.find('"')?;
        Some(&rest[..end])
    }
    let mut frames = Vec::new();
    for entry in text.split("{ fn: ").skip(1) {
        let entry_text = format!("fn: {entry}");
        let Some(function_path) = quoted_after(&entry_text, "fn: ") else {
            continue;
        };
        let file = quoted_after(&entry_text, "file: ").map(str::to_string);
        let line = entry_text
            .find("line: ")
            .map(|i| &entry_text[i + 6..])
            .and_then(|rest| {
                let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
                digits.parse::<i32>().ok()
            })
            .unwrap_or(-1);
        let (path, function) = split_function_path(function_path);
        frames.push(StackFrame {
            path,
            function,
            file,
            line,
        });
    }
    frames
}

/// Splits a demangled function path into its prefix and its last segment,
/// ignoring the `::` inside `<...>` and the trailing `{{closure}}` segments.
fn split_function_path(function_path: &str) -> (String, String) {
    let mut segments: Vec<String> = Vec::new();
    let mut depth = 0i32;
    let mut current = String::new();
    let mut chars = function_path.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '<' => {
                depth += 1;
                current.push(c);
            }
            '>' => {
                depth -= 1;
                current.push(c);
            }
            ':' if depth == 0 && chars.peek() == Some(&':') => {
                chars.next();
                segments.push(std::mem::take(&mut current));
            }
            _ => current.push(c),
        }
    }
    segments.push(current);
    while segments.len() > 1
        && segments
            .last()
            .is_some_and(|s| s.starts_with("{{") || s.starts_with('{'))
    {
        segments.pop();
    }
    let function = segments.pop().unwrap_or_default();
    (segments.join("::"), function)
}

#[cfg(test)]
mod tests {

    /// The `Debug` rendering of a captured backtrace is parsed into frames,
    /// the last path segment outside `<...>` being the function and the
    /// trailing closure segments being dropped.
    #[test]
    fn a_backtrace_debug_rendering_is_parsed_into_frames() {
        let text = r#"Backtrace [{ fn: "std::backtrace::Backtrace::capture", file: "/rustc/x/library/std/src/backtrace.rs", line: 296 }, { fn: "rucene::error::<impl core::convert::From<rucene::error::LuceneErrorKind> for rucene::error::LuceneError>::from", file: "./src/error.rs", line: 380 }, { fn: "<rucene::search::scoring_rewrite::ScoringRewrite as rucene::search::scoring_rewrite::Check>::check_max_clause_count", file: "./src/search/scoring_rewrite.rs", line: 252 }, { fn: "rucene::search::Foo::bar::{{closure}}", file: "./src/search.rs", line: 7 }]"#;
        let frames = parse_backtrace(text);
        assert_eq!(frames.len(), 4);
        assert_eq!(frames[0].function, "capture");
        assert_eq!(frames[1].function, "from");
        assert_eq!(frames[2].function, "check_max_clause_count");
        assert_eq!(frames[2].line, 252);
        assert_eq!(
            frames[2].file.as_deref(),
            Some("./src/search/scoring_rewrite.rs")
        );
        assert_eq!(frames[3].function, "bar");
        assert_eq!(frames[3].path, "rucene::search::Foo");
    }

    /// Without `RUST_LIB_BACKTRACE`/`RUST_BACKTRACE` the capture is disabled and
    /// the stack trace is empty; with it, the first frame is the function that
    /// created the error, past the constructors.
    #[test]
    fn the_stack_trace_starts_at_the_creating_function() {
        fn creating_function() -> LuceneError {
            LuceneError::illegal_state("boom")
        }
        let error = creating_function();
        let frames = error.get_stack_trace();
        match error.backtrace {
            Some(_) => assert_eq!(frames[0].function, "creating_function", "{frames:?}"),
            None => assert!(frames.is_empty()),
        }
    }
    use super::{parse_backtrace, LuceneError, LuceneErrorKind};

    /// `IndexFormatTooOldException.getReason()` returns the free-form reason
    /// when the version could not be read off the file, and `null` when the
    /// version *was* available — the two constructors are mutually exclusive.
    #[test]
    fn index_format_too_old_reports_its_reason_only_when_it_has_one() {
        let with_reason = LuceneError::index_format_too_old_reason("segments_1", "unreadable");
        assert_eq!(with_reason.reason(), Some("unreadable"));
        assert_eq!(with_reason.version(), None);
        assert_eq!(with_reason.resource_description(), Some("segments_1"));

        let with_version = LuceneError::index_format_too_old("_0.si", 5, 9, 10);
        assert_eq!(with_version.reason(), None);
        assert_eq!(with_version.version(), Some(5));

        // Every other error carries no reason at all.
        assert_eq!(
            LuceneError::index_format_too_new("_0.si", 12, 9, 10).reason(),
            None
        );
        assert_eq!(
            LuceneError::from(LuceneErrorKind::corrupt_index_message("x")).reason(),
            None
        );
    }

    /// `new AlreadyClosedException(String, Throwable)` keeps the cause apart
    /// from the message and `getCause()` returns it; here
    /// [`std::error::Error::source`] does.
    #[test]
    fn already_closed_keeps_its_cause_as_its_source() {
        use std::error::Error;
        use std::sync::Arc;

        let plain = LuceneError::already_closed("this IndexWriter is closed");
        assert!(plain.source().is_none());

        let tragedy = Arc::new(LuceneError::from(LuceneErrorKind::AssertionError(
            "boom".to_string(),
        )));
        let closed = LuceneError::already_closed_with_cause(
            "this IndexWriter is closed",
            Arc::clone(&tragedy),
        );
        assert_eq!(
            closed.to_string(),
            "already closed: this IndexWriter is closed"
        );
        let cause = closed
            .source()
            .and_then(|source| source.downcast_ref::<Arc<LuceneError>>())
            .expect("the cause is the source");
        assert!(Arc::ptr_eq(cause, &tragedy));
        assert!(
            matches!(closed.duplicate(), LuceneError { kind: LuceneErrorKind::AlreadyClosed { cause: Some(c), .. }, .. } if Arc::ptr_eq(&c, &tragedy))
        );
    }

    /// `Math.toIntExact(1L << 40)` throws
    /// `java.lang.ArithmeticException: integer overflow` (observed with the
    /// 10.5.0 oracle); an in-range value converts unchanged, at both ends of
    /// the `int` range.
    #[test]
    fn to_int_exact_is_javas_arithmetic_exception() {
        use super::to_int_exact;

        let error = to_int_exact(1i64 << 40).unwrap_err();
        assert!(
            matches!(&error, LuceneError { kind: LuceneErrorKind::Arithmetic(m), .. } if m == "integer overflow"),
            "{error:?}"
        );
        assert_eq!(error.java_simple_name(), "ArithmeticException");
        assert!(matches!(
            to_int_exact(i64::from(i32::MIN) - 1),
            Err(LuceneError {
                kind: LuceneErrorKind::Arithmetic(_),
                ..
            })
        ));
        assert_eq!(to_int_exact(i64::from(i32::MAX)).unwrap(), i32::MAX);
        assert_eq!(to_int_exact(i64::from(i32::MIN)).unwrap(), i32::MIN);
        assert!(matches!(
            LuceneError::integer_overflow().duplicate(),
            LuceneError { kind: LuceneErrorKind::Arithmetic(m), .. } if m == "integer overflow"
        ));
    }

    /// `new IllegalArgumentException(String, Throwable)` keeps the cause apart
    /// from the message and `getCause()` returns it; it is the same Java class
    /// as the cause-less form.
    #[test]
    fn illegal_argument_with_cause_keeps_its_cause_as_its_source() {
        use std::error::Error;
        use std::sync::Arc;

        let cause = Arc::new(LuceneError::integer_overflow());
        let error = LuceneError::illegal_argument_with_cause(
            "too many tokens for field \"f\"",
            Arc::clone(&cause),
        );
        assert_eq!(error.java_simple_name(), "IllegalArgumentException");
        assert_eq!(
            error.to_string(),
            LuceneError::illegal_argument("too many tokens for field \"f\"").to_string()
        );
        let source = error
            .source()
            .and_then(|source| source.downcast_ref::<Arc<LuceneError>>())
            .expect("the cause is the source");
        assert!(Arc::ptr_eq(source, &cause));
        assert!(matches!(
            error.duplicate(),
            LuceneError { kind: LuceneErrorKind::IllegalArgument { cause: Some(c), .. }, .. } if Arc::ptr_eq(&c, &cause)
        ));
    }
}
