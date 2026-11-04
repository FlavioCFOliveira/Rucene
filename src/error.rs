//! Error types used across Rucene.
//!
//! This module maps the most important Java Lucene exceptions to a Rust
//! [`Result`]-based error hierarchy. Where Java uses checked exceptions for
//! recoverable I/O or index corruption, Rucene returns [`LuceneError`].

use std::sync::Arc;

use thiserror::Error;

/// The top-level error type for Rucene operations.
#[derive(Error, Debug)]
pub enum LuceneError {
    /// I/O failure analogous to `java.io.IOException`.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// An index file is corrupt or inconsistent.
    #[error("corrupt index: {0}")]
    CorruptIndex(String),

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
    /// Equivalent to `java.lang.IllegalArgumentException` built without a
    /// cause, `new IllegalArgumentException(String)`; one built with a cause
    /// is [`LuceneError::IllegalArgumentWithCause`].
    #[error("illegal argument: {0}")]
    IllegalArgument(String),

    /// An invalid argument was supplied, and the failure that revealed it is
    /// kept as its cause.
    ///
    /// Equivalent to `java.lang.IllegalArgumentException` built with a cause,
    /// `new IllegalArgumentException(String, Throwable)` — for example the
    /// stale-reader check `IndexWriter` runs on an `IndexCommit` it is opened
    /// from (`IndexWriter.java:1059-1064`), whose cause is the `IOException`
    /// reading the commit raised. `cause` is that cause, which
    /// [`std::error::Error::source`] returns as Java's `getCause()` does.
    /// Build one with [`LuceneError::illegal_argument_with_cause`].
    ///
    /// **Divergence from Lucene 10.5.0.** Java has one class, and the cause is
    /// an optional field of it. [`LuceneError::IllegalArgument`] is a tuple
    /// variant built at about 1250 sites, none of which passes a cause, so the
    /// cause cannot become a field of it without rewriting every one of them;
    /// the ported sites that pass a cause build this variant instead. Both
    /// report [`java_simple_name`](LuceneError::java_simple_name)
    /// `IllegalArgumentException` and display alike, and every match the port
    /// makes on Java's class names both.
    #[error("illegal argument: {message}")]
    IllegalArgumentWithCause {
        /// The message, Java's `getMessage()`.
        message: String,
        /// The cause, Java's `getCause()`.
        #[source]
        cause: Arc<LuceneError>,
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
    #[error("unsupported operation: {0}")]
    UnsupportedOperation(String),

    /// An operation is not supported, with the error that made it so.
    ///
    /// Equivalent to `java.lang.UnsupportedOperationException` built with a
    /// cause, `new UnsupportedOperationException(String, Throwable)` — for
    /// example `AnalysisSPILoader.newFactoryClassInstance` for a factory
    /// class without a `Map<String,String>` constructor
    /// (`AnalysisSPILoader.java:206-211`), whose cause is the
    /// `NoSuchMethodException` `getConstructor` threw. `cause` is that cause,
    /// which [`std::error::Error::source`] returns as Java's `getCause()`
    /// does. Build one with [`LuceneError::unsupported_operation_with_cause`].
    ///
    /// **Divergence from Lucene 10.5.0.** Java has one class, and the cause is
    /// an optional field of it; [`LuceneError::UnsupportedOperation`] is a
    /// tuple variant built at several hundred sites, none of which passes a
    /// cause, so the sites that pass one build this variant, as
    /// [`LuceneError::IllegalArgumentWithCause`] does for
    /// `IllegalArgumentException`. Both report
    /// [`java_simple_name`](LuceneError::java_simple_name)
    /// `UnsupportedOperationException` and display alike.
    #[error("unsupported operation: {message}")]
    UnsupportedOperationWithCause {
        /// The message, Java's `getMessage()`.
        message: String,
        /// The cause, Java's `getCause()`.
        #[source]
        cause: Arc<LuceneError>,
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

/// `Throwable.toString()` of an [`LuceneError::Unchecked`].
fn unchecked_to_string(class: &str, message: &Option<String>) -> String {
    match message {
        Some(message) => format!("{class}: {message}"),
        None => class.to_string(),
    }
}

impl LuceneError {
    /// The simple name of the Java exception class this error stands for, what
    /// `getClass().getSimpleName()` prints.
    ///
    /// **Divergence from Lucene 10.5.0.** [`LuceneError::IndexFormatNotSupported`],
    /// [`LuceneError::NotFound`], [`LuceneError::ResourceLimit`],
    /// [`LuceneError::Cancelled`] and [`LuceneError::Other`] are this port's
    /// own variants, each standing for several Java classes, so no single
    /// Java name exists for them; they report the variant's name.
    pub fn java_simple_name(&self) -> &'static str {
        match self {
            Self::Io(_) => "IOException",
            Self::CorruptIndex(_) => "CorruptIndexException",
            Self::IndexFormatTooOld { .. } => "IndexFormatTooOldException",
            Self::IndexFormatTooNew { .. } => "IndexFormatTooNewException",
            Self::IndexNotFound(_) => "IndexNotFoundException",
            Self::IllegalArgument(_) | Self::IllegalArgumentWithCause { .. } => {
                "IllegalArgumentException"
            }
            Self::IllegalState { .. } => "IllegalStateException",
            Self::LockObtainFailed { .. } => "LockObtainFailedException",
            Self::LockReleaseFailed { .. } => "LockReleaseFailedException",
            Self::AlreadyClosed { .. } => "AlreadyClosedException",
            Self::UnsupportedOperation(_) | Self::UnsupportedOperationWithCause { .. } => {
                "UnsupportedOperationException"
            }
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
            Self::Cancelled => "Cancelled",
            Self::Other(_) => "Other",
            Self::Unchecked { class, .. } => class.rsplit('.').next().unwrap_or(class),
        }
    }

    /// Whether the Java class this error stands for is a `java.io.IOException`
    /// — what a `catch (IOException e)` clause catches.
    ///
    /// True for [`LuceneError::Io`] and for the variants whose Java classes
    /// extend `IOException`: `CorruptIndexException` and its
    /// `IndexFormatTooOldException` and `IndexFormatTooNewException`,
    /// `IndexNotFoundException` (a `FileNotFoundException`),
    /// `LockObtainFailedException`, `LockReleaseFailedException` and
    /// `MergePolicy.MergeAbortedException`.
    pub fn is_io_exception(&self) -> bool {
        matches!(
            self,
            Self::Io(_)
                | Self::CorruptIndex(_)
                | Self::IndexFormatTooOld { .. }
                | Self::IndexFormatTooNew { .. }
                | Self::IndexNotFound(_)
                | Self::LockObtainFailed { .. }
                | Self::LockReleaseFailed { .. }
                | Self::MergeAborted(_)
        )
    }

    /// `new IllegalStateException(String)`.
    pub fn illegal_state(message: impl Into<String>) -> Self {
        Self::IllegalState {
            message: message.into(),
            cause: None,
        }
    }

    /// `new IllegalStateException(String, Throwable)`.
    pub fn illegal_state_with_cause(message: impl Into<String>, cause: Arc<LuceneError>) -> Self {
        Self::IllegalState {
            message: message.into(),
            cause: Some(cause),
        }
    }

    /// `new IllegalArgumentException(String, Throwable)`.
    pub fn illegal_argument_with_cause(
        message: impl Into<String>,
        cause: Arc<LuceneError>,
    ) -> Self {
        Self::IllegalArgumentWithCause {
            message: message.into(),
            cause,
        }
    }

    /// `new UnsupportedOperationException(String, Throwable)`.
    pub fn unsupported_operation_with_cause(
        message: impl Into<String>,
        cause: Arc<LuceneError>,
    ) -> Self {
        Self::UnsupportedOperationWithCause {
            message: message.into(),
            cause,
        }
    }

    /// `new ArithmeticException("integer overflow")`, what `Math.toIntExact`
    /// throws for a `long` outside the `int` range.
    pub fn integer_overflow() -> Self {
        Self::Arithmetic("integer overflow".to_string())
    }

    /// `new ThreadInterruptedException(ie)`, what Lucene's waits throw when
    /// `Object.wait` throws `InterruptedException`.
    pub fn thread_interrupted(ie: crate::util::InterruptedException) -> Self {
        Self::ThreadInterrupted(crate::util::ThreadInterruptedException::new(ie))
    }

    /// `new AlreadyClosedException(String)`.
    pub fn already_closed(message: impl Into<String>) -> Self {
        Self::AlreadyClosed {
            message: message.into(),
            cause: None,
        }
    }

    /// `new AlreadyClosedException(String, Throwable)`.
    pub fn already_closed_with_cause(message: impl Into<String>, cause: Arc<LuceneError>) -> Self {
        Self::AlreadyClosed {
            message: message.into(),
            cause: Some(cause),
        }
    }

    /// Returns an equal error: the same variant, message and fields, and the
    /// same cause.
    ///
    /// Used where Java keeps a reference to a `Throwable` that is also thrown,
    /// such as `IndexWriter.tragedy` (`IndexWriter.java:5847-5859`): a
    /// [`LuceneError`] has a single owner, so the writer keeps this copy.
    /// [`LuceneError::Io`] and the lock variants' I/O sources are rebuilt from
    /// their kind and rendered message, which is all a `java.io.IOException`
    /// carries besides its own cause.
    pub fn duplicate(&self) -> Self {
        fn io(error: &std::io::Error) -> std::io::Error {
            std::io::Error::new(error.kind(), error.to_string())
        }
        match self {
            Self::Io(error) => Self::Io(io(error)),
            Self::CorruptIndex(m) => Self::CorruptIndex(m.clone()),
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
            Self::IllegalArgument(m) => Self::IllegalArgument(m.clone()),
            Self::IllegalArgumentWithCause { message, cause } => Self::IllegalArgumentWithCause {
                message: message.clone(),
                cause: Arc::clone(cause),
            },
            Self::IllegalState { message, cause } => Self::IllegalState {
                message: message.clone(),
                cause: cause.clone(),
            },
            Self::ResourceLimit(m) => Self::ResourceLimit(m.clone()),
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
            Self::UnsupportedOperation(m) => Self::UnsupportedOperation(m.clone()),
            Self::UnsupportedOperationWithCause { message, cause } => {
                Self::UnsupportedOperationWithCause {
                    message: message.clone(),
                    cause: Arc::clone(cause),
                }
            }
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
    /// `new RuntimeException(cause)`: an [`LuceneError::Unchecked`]
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
        let message = cause.io_exception_to_string();
        Self::Unchecked {
            class: "java.lang.RuntimeException",
            message: Some(message),
            cause: Some(Arc::new(cause)),
        }
    }

    /// `Throwable.toString()` of the `IOException` this error stands for.
    fn io_exception_to_string(&self) -> String {
        match self {
            Self::Io(e) => crate::store::java_nio_file::java_to_string(e),
            Self::CorruptIndex(m) => format!("org.apache.lucene.index.CorruptIndexException: {m}"),
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

    /// `new <class>(message)` for an unchecked Java exception no other
    /// variant stands for: [`LuceneError::Unchecked`] without a cause.
    pub fn unchecked(class: &'static str, message: impl Into<String>) -> Self {
        Self::Unchecked {
            class,
            message: Some(message.into()),
            cause: None,
        }
    }

    /// Creates a `CorruptIndex` error composing the message the way
    /// `org.apache.lucene.index.CorruptIndexException` does: the caller's message
    /// followed by ` (resource=<description>)`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java keeps `message` and
    /// `resourceDescription` as separate fields, reachable through
    /// `getOriginalMessage()` and `getResourceDescription()`
    /// (`CorruptIndexException.java:62-70`). This variant carries the composed
    /// string only, because 232 call sites in the crate construct
    /// `LuceneError::CorruptIndex` directly with an already-composed message
    /// (only 29 of them go through this constructor); splitting the variant
    /// into separate fields would require rewriting every one of those sites,
    /// most of which live outside this module. `getOriginalMessage()` and
    /// `getResourceDescription()` are therefore not ported: recovering the two
    /// parts from the composed string is not reliable, since most call sites
    /// never route through the `" (resource=...)"` suffix this constructor
    /// appends. The rendered message is identical to Java's.
    pub fn corrupt_index(message: impl AsRef<str>, resource_description: impl AsRef<str>) -> Self {
        Self::CorruptIndex(format!(
            "{} (resource={})",
            message.as_ref(),
            resource_description.as_ref()
        ))
    }

    /// Creates an `IndexFormatTooOld` error from a free-form reason, for when the
    /// version could not be read from the file at all.
    pub fn index_format_too_old_reason(
        resource_description: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self::IndexFormatTooOld {
            resource_description: resource_description.into(),
            reason: Some(reason.into()),
            version: None,
            min_version: None,
            max_version: None,
        }
    }

    /// Creates an `IndexFormatTooOld` error from the version triple read off the file.
    pub fn index_format_too_old(
        resource_description: impl Into<String>,
        version: i32,
        min_version: i32,
        max_version: i32,
    ) -> Self {
        Self::IndexFormatTooOld {
            resource_description: resource_description.into(),
            reason: None,
            version: Some(version),
            min_version: Some(min_version),
            max_version: Some(max_version),
        }
    }

    /// Creates an `IndexFormatTooNew` error.
    pub fn index_format_too_new(
        resource_description: impl Into<String>,
        version: i32,
        min_version: i32,
        max_version: i32,
    ) -> Self {
        Self::IndexFormatTooNew {
            resource_description: resource_description.into(),
            version,
            min_version,
            max_version,
        }
    }

    /// Returns a description of the file that was too old or too new, when
    /// this error is an `IndexFormatTooOld` or `IndexFormatTooNew`.
    ///
    /// Equivalent to `org.apache.lucene.index.IndexFormatTooOldException#getResourceDescription`
    /// and `org.apache.lucene.index.IndexFormatTooNewException#getResourceDescription`.
    pub fn resource_description(&self) -> Option<&str> {
        match self {
            Self::IndexFormatTooOld {
                resource_description,
                ..
            }
            | Self::IndexFormatTooNew {
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
        match self {
            Self::IndexFormatTooOld { reason, .. } => reason.as_deref(),
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
        match self {
            Self::IndexFormatTooOld { version, .. } => *version,
            Self::IndexFormatTooNew { version, .. } => Some(*version),
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
        match self {
            Self::IndexFormatTooOld { min_version, .. } => *min_version,
            Self::IndexFormatTooNew { min_version, .. } => Some(*min_version),
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
        match self {
            Self::IndexFormatTooOld { max_version, .. } => *max_version,
            Self::IndexFormatTooNew { max_version, .. } => Some(*max_version),
            _ => None,
        }
    }

    /// Creates an `IndexNotFound` error.
    pub fn index_not_found(message: impl Into<String>) -> Self {
        Self::IndexNotFound(message.into())
    }

    /// Creates a `LockObtainFailed` error without an underlying I/O cause.
    pub fn lock_obtain_failed(message: impl Into<String>) -> Self {
        Self::LockObtainFailed {
            message: message.into(),
            source: None,
        }
    }

    /// Creates a `LockObtainFailed` error with an underlying I/O cause.
    pub fn lock_obtain_failed_with_source(
        message: impl Into<String>,
        source: std::io::Error,
    ) -> Self {
        Self::LockObtainFailed {
            message: message.into(),
            source: Some(source),
        }
    }

    /// Creates a `LockReleaseFailed` error without an underlying I/O cause.
    pub fn lock_release_failed(message: impl Into<String>) -> Self {
        Self::LockReleaseFailed {
            message: message.into(),
            source: None,
        }
    }

    /// Creates a `LockReleaseFailed` error with an underlying I/O cause.
    pub fn lock_release_failed_with_source(
        message: impl Into<String>,
        source: std::io::Error,
    ) -> Self {
        Self::LockReleaseFailed {
            message: message.into(),
            source: Some(source),
        }
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
/// [`LuceneError::Arithmetic`] with the message `integer overflow`, built by
/// [`LuceneError::integer_overflow`], when `value` is outside the `i32` range:
/// Java's `ArithmeticException("integer overflow")`.
pub fn to_int_exact(value: i64) -> Result<i32> {
    i32::try_from(value).map_err(|_| LuceneError::integer_overflow())
}

#[cfg(test)]
mod tests {
    use super::LuceneError;

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
        assert_eq!(LuceneError::CorruptIndex("x".to_string()).reason(), None);
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

        let tragedy = Arc::new(LuceneError::AssertionError("boom".to_string()));
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
            matches!(closed.duplicate(), LuceneError::AlreadyClosed { cause: Some(c), .. } if Arc::ptr_eq(&c, &tragedy))
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
            matches!(&error, LuceneError::Arithmetic(m) if m == "integer overflow"),
            "{error:?}"
        );
        assert_eq!(error.java_simple_name(), "ArithmeticException");
        assert!(matches!(
            to_int_exact(i64::from(i32::MIN) - 1),
            Err(LuceneError::Arithmetic(_))
        ));
        assert_eq!(to_int_exact(i64::from(i32::MAX)).unwrap(), i32::MAX);
        assert_eq!(to_int_exact(i64::from(i32::MIN)).unwrap(), i32::MIN);
        assert!(matches!(
            LuceneError::integer_overflow().duplicate(),
            LuceneError::Arithmetic(m) if m == "integer overflow"
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
            LuceneError::IllegalArgument("too many tokens for field \"f\"".to_string()).to_string()
        );
        let source = error
            .source()
            .and_then(|source| source.downcast_ref::<Arc<LuceneError>>())
            .expect("the cause is the source");
        assert!(Arc::ptr_eq(source, &cause));
        assert!(matches!(
            error.duplicate(),
            LuceneError::IllegalArgumentWithCause { cause: c, .. } if Arc::ptr_eq(&c, &cause)
        ));
    }
}
