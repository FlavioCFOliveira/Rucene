//! Error types used across Rucene.
//!
//! This module maps the most important Java Lucene exceptions to a Rust
//! [`Result`]-based error hierarchy. Where Java uses checked exceptions for
//! recoverable I/O or index corruption, Rucene returns [`LuceneError`].

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

    /// A requested document, field, or term was not found.
    #[error("not found: {0}")]
    NotFound(String),

    /// An invalid argument was supplied.
    #[error("illegal argument: {0}")]
    IllegalArgument(String),

    /// An illegal state was encountered (e.g., writing to a closed writer).
    #[error("illegal state: {0}")]
    IllegalState(String),

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
    #[error("already closed: {0}")]
    AlreadyClosed(String),

    /// An operation is not supported in the current context.
    #[error("unsupported operation: {0}")]
    UnsupportedOperation(String),

    /// A generic wrapper for errors that do not yet have a dedicated variant.
    #[error("{0}")]
    Other(String),
}

impl LuceneError {
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
