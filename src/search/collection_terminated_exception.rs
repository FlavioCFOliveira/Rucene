//! Early-termination signalling, ported from
//! `org.apache.lucene.search.CollectionTerminatedException`.
//!
//! # Adaptation: an exception used as control flow
//!
//! Java throws `CollectionTerminatedException` from
//! `LeafCollector.collect(int)` (and from
//! `Collector.getLeafCollector(LeafReaderContext)`) to end the collection of
//! the current leaf early; `IndexSearcher` swallows it and moves on to the next
//! leaf. It is a `RuntimeException` whose `fillInStackTrace` is overridden to
//! do nothing, precisely because it is control flow rather than a failure.
//!
//! Rust has no exceptions, so the signal travels in the error channel of a
//! dedicated result type: [`CollectionError`] is either the
//! [`CollectionTerminatedException`] signal, the
//! [`TimeExceededException`](CollectionError::TimeExceeded) signal that
//! `TimeLimitingBulkScorer` raises, or a genuine [`LuceneError`]. Everything on
//! the collection path returns [`CollectionResult`], and
//! [`IndexSearcher`](crate::search::IndexSearcher) recognises the two signals
//! exactly where Java catches them.

#![deny(unsafe_code)]

use std::fmt;

use crate::error::LuceneError;

/// Signals that collection of the current leaf must stop early.
///
/// Equivalent to `org.apache.lucene.search.CollectionTerminatedException`.
/// Return it from
/// [`LeafCollector::collect`](crate::search::LeafCollector::collect) — as
/// [`CollectionError::CollectionTerminated`] — to prematurely terminate
/// collection of the current leaf. The last docs of the current
/// [`LeafReaderContext`](crate::index::LeafReaderContext) are skipped and
/// [`IndexSearcher`](crate::search::IndexSearcher) continues with the next
/// leaf.
///
/// As in Java, `IndexSearcher` never re-raises this signal, so callers of the
/// search methods should not attempt to handle it.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CollectionTerminatedException;

impl CollectionTerminatedException {
    /// Creates the signal.
    ///
    /// Equivalent to `new CollectionTerminatedException()`.
    pub fn new() -> Self {
        Self
    }
}

impl fmt::Display for CollectionTerminatedException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("collection terminated")
    }
}

impl std::error::Error for CollectionTerminatedException {}

/// Signals that the elapsed search time exceeded the allowed search time.
///
/// Equivalent to the package-private
/// `TimeLimitingBulkScorer.TimeExceededException`. Like
/// [`CollectionTerminatedException`], it is control flow rather than a failure:
/// [`IndexSearcher`](crate::search::IndexSearcher) catches it and records a
/// partial result.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TimeExceededException;

impl fmt::Display for TimeExceededException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TimeLimit Exceeded")
    }
}

impl std::error::Error for TimeExceededException {}

/// Everything that can interrupt the collection of a leaf: the two control-flow
/// signals Java raises as unchecked exceptions, and a genuine failure.
///
/// See the module documentation for why this type exists.
#[derive(Debug)]
pub enum CollectionError {
    /// Collection of the current leaf ended early.
    ///
    /// Equivalent to a thrown `CollectionTerminatedException`.
    CollectionTerminated,

    /// The search ran past its allowed time.
    ///
    /// Equivalent to a thrown `TimeLimitingBulkScorer.TimeExceededException`.
    TimeExceeded,

    /// A genuine failure, equivalent to Java's `IOException` and the unchecked
    /// exceptions Lucene lets propagate.
    Lucene(LuceneError),
}

impl fmt::Display for CollectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CollectionTerminated => write!(f, "{CollectionTerminatedException}"),
            Self::TimeExceeded => write!(f, "{TimeExceededException}"),
            Self::Lucene(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for CollectionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Lucene(err) => Some(err),
            _ => None,
        }
    }
}

impl From<LuceneError> for CollectionError {
    fn from(err: LuceneError) -> Self {
        Self::Lucene(err)
    }
}

impl From<CollectionTerminatedException> for CollectionError {
    fn from(_: CollectionTerminatedException) -> Self {
        Self::CollectionTerminated
    }
}

impl From<TimeExceededException> for CollectionError {
    fn from(_: TimeExceededException) -> Self {
        Self::TimeExceeded
    }
}

impl From<std::io::Error> for CollectionError {
    fn from(err: std::io::Error) -> Self {
        Self::Lucene(LuceneError::Io(err))
    }
}

impl CollectionError {
    /// Converts this error into a [`LuceneError`], turning the two control-flow
    /// signals into `IllegalState`.
    ///
    /// This is only for boundaries that cannot propagate the signals; the
    /// search loop must match on the variants instead, as Java's `catch` blocks
    /// do.
    pub fn into_lucene_error(self) -> LuceneError {
        match self {
            Self::CollectionTerminated => {
                LuceneError::IllegalState(CollectionTerminatedException.to_string())
            }
            Self::TimeExceeded => LuceneError::IllegalState(TimeExceededException.to_string()),
            Self::Lucene(err) => err,
        }
    }
}

/// The result type of everything on the collection path.
///
/// See the module documentation.
pub type CollectionResult<T> = std::result::Result<T, CollectionError>;
