//! Store-layer exception types ported from `org.apache.lucene.store`.
//!
//! Java signals these three conditions with dedicated exception classes:
//!
//! * `org.apache.lucene.store.AlreadyClosedException` (extends
//!   `IllegalStateException`),
//! * `org.apache.lucene.store.LockObtainFailedException` (extends
//!   `IOException`),
//! * `org.apache.lucene.store.LockReleaseFailedException` (extends
//!   `IOException`).
//!
//! Rucene has a single crate-wide error enum, [`LuceneError`], which already
//! carries one variant per condition: [`LuceneError::AlreadyClosed`],
//! [`LuceneError::LockObtainFailed`] and [`LuceneError::LockReleaseFailed`].
//! Rather than introducing a second, parallel error hierarchy, each Java class
//! is ported here as a zero-sized *namespace* type carrying the two operations
//! the Java class provides:
//!
//! * construction — the `throw new …` side, via [`new`](AlreadyClosedException::new)
//!   and `with_cause`;
//! * classification — the `catch (… e)` side, via `is`, because Rust cannot
//!   match on an exception class.
//!
//! # Divergence from Lucene 10.5.0
//!
//! Java's three classes are distinct types, so a `catch` clause can select one
//! of them and a caller can hold, say, a `LockObtainFailedException` in a
//! variable. Here they are all [`LuceneError`] values distinguished by variant,
//! so the classification predicates take the place of `catch`. This follows the
//! convention already established by [`crate::error`], which maps Java's
//! exception hierarchy onto a single enum; adding real Rust error types for
//! these three would have created two ways to express the same failure.
//!
//! [`AlreadyClosedException::with_cause`] additionally folds the cause into the
//! message, because [`LuceneError::AlreadyClosed`] carries only a `String`,
//! whereas Java keeps the cause in a separate `Throwable` field. The two
//! lock exceptions do have a `source` field, so their causes are preserved.

use std::fmt::Display;
use std::io;

use crate::error::LuceneError;

/// Raised when something that has already been closed is accessed.
///
/// Equivalent to `org.apache.lucene.store.AlreadyClosedException`, which
/// extends `IllegalStateException`. Values are produced as
/// [`LuceneError::AlreadyClosed`]; this type only groups the constructors and
/// the classification predicate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct AlreadyClosedException;

impl AlreadyClosedException {
    /// Builds the error for `message`.
    ///
    /// Equivalent to `new AlreadyClosedException(String)`.
    // This type is a namespace for a Java exception class, not a value type:
    // `new` names the Java constructor and necessarily yields a `LuceneError`.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(message: impl Into<String>) -> LuceneError {
        LuceneError::AlreadyClosed(message.into())
    }

    /// Builds the error for `message`, recording `cause` in the message.
    ///
    /// Equivalent to `new AlreadyClosedException(String, Throwable)`.
    ///
    /// # Divergence from Lucene 10.5.0
    ///
    /// Java keeps the cause in `Throwable.getCause()`, separate from the
    /// message. [`LuceneError::AlreadyClosed`] holds only a message, so the
    /// cause is appended as ` (caused by: …)`.
    pub fn with_cause(message: impl Display, cause: impl Display) -> LuceneError {
        LuceneError::AlreadyClosed(format!("{message} (caused by: {cause})"))
    }

    /// Returns `true` if `error` is the equivalent of an
    /// `AlreadyClosedException`.
    ///
    /// This is the Rust counterpart of `catch (AlreadyClosedException e)`.
    pub fn is(error: &LuceneError) -> bool {
        matches!(error, LuceneError::AlreadyClosed(_))
    }
}

/// Raised when the `write.lock` could not be acquired, because another writer
/// already holds it.
///
/// Equivalent to `org.apache.lucene.store.LockObtainFailedException`, which
/// extends `IOException`. Values are produced as
/// [`LuceneError::LockObtainFailed`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct LockObtainFailedException;

impl LockObtainFailedException {
    /// Builds the error for `message`.
    ///
    /// Equivalent to `new LockObtainFailedException(String)`.
    // See `AlreadyClosedException::new` for why this does not return `Self`.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(message: impl Into<String>) -> LuceneError {
        LuceneError::lock_obtain_failed(message)
    }

    /// Builds the error for `message`, keeping `cause` as the error source.
    ///
    /// Equivalent to `new LockObtainFailedException(String, Throwable)`.
    pub fn with_cause(message: impl Into<String>, cause: io::Error) -> LuceneError {
        LuceneError::lock_obtain_failed_with_source(message, cause)
    }

    /// Returns `true` if `error` is the equivalent of a
    /// `LockObtainFailedException`.
    ///
    /// This is the Rust counterpart of `catch (LockObtainFailedException e)`,
    /// which [`SleepingLockWrapper`](super::SleepingLockWrapper) and
    /// [`LockStressTest`](super::LockStressTest) both rely on.
    pub fn is(error: &LuceneError) -> bool {
        matches!(error, LuceneError::LockObtainFailed { .. })
    }
}

/// Raised when the `write.lock` could not be released.
///
/// Equivalent to `org.apache.lucene.store.LockReleaseFailedException`, which
/// extends `IOException`. Values are produced as
/// [`LuceneError::LockReleaseFailed`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct LockReleaseFailedException;

impl LockReleaseFailedException {
    /// Builds the error for `message`.
    ///
    /// Equivalent to `new LockReleaseFailedException(String)`.
    // See `AlreadyClosedException::new` for why this does not return `Self`.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(message: impl Into<String>) -> LuceneError {
        LuceneError::lock_release_failed(message)
    }

    /// Builds the error for `message`, keeping `cause` as the error source.
    ///
    /// Equivalent to `new LockReleaseFailedException(String, Throwable)`.
    pub fn with_cause(message: impl Into<String>, cause: io::Error) -> LuceneError {
        LuceneError::lock_release_failed_with_source(message, cause)
    }

    /// Returns `true` if `error` is the equivalent of a
    /// `LockReleaseFailedException`.
    ///
    /// This is the Rust counterpart of `catch (LockReleaseFailedException e)`.
    pub fn is(error: &LuceneError) -> bool {
        matches!(error, LuceneError::LockReleaseFailed { .. })
    }
}
