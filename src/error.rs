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

    /// A merge was aborted before it completed.
    ///
    /// Equivalent to `org.apache.lucene.index.MergePolicy.MergeAbortedException`,
    /// which extends `IOException` and is thrown by `OneMerge.checkAborted()`
    /// once `IndexWriter.rollback` or `deleteAll` has stopped the merges.
    #[error("merge aborted: {0}")]
    MergeAborted(String),

    /// A merge failed.
    ///
    /// Equivalent to `org.apache.lucene.index.MergePolicy.MergeException`,
    /// which Lucene throws when a merge policy selects a segment the writer
    /// does not hold, or when a background merge fails.
    #[error("merge failed: {0}")]
    Merge(String),

    /// A generic wrapper for errors that do not yet have a dedicated variant.
    #[error("{0}")]
    Other(String),
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
