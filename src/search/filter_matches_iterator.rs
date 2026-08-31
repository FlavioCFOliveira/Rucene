//! Delegating match iteration, ported from
//! `org.apache.lucene.search.FilterMatchesIterator`.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::error::Result;
use crate::search::matches::MatchesIterator;
use crate::search::query::Query;

/// A [`MatchesIterator`] that delegates all calls to another
/// [`MatchesIterator`].
///
/// Equivalent to `org.apache.lucene.search.FilterMatchesIterator`. Java leaves
/// the class abstract so that a subclass must exist to change something; Rust
/// composition makes the wrapper concrete, and a "subclass" is a type that
/// holds one and overrides the methods it cares about.
pub struct FilterMatchesIterator {
    /// The delegate.
    ///
    /// Equivalent to the `protected final MatchesIterator in` field.
    pub inner: Box<dyn MatchesIterator>,
}

impl std::fmt::Debug for FilterMatchesIterator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FilterMatchesIterator")
    }
}

impl FilterMatchesIterator {
    /// Creates a new filtering iterator over `inner`.
    ///
    /// Equivalent to `FilterMatchesIterator(MatchesIterator)`.
    pub fn new(inner: Box<dyn MatchesIterator>) -> Self {
        Self { inner }
    }

    /// Unwraps this iterator, returning the delegate.
    pub fn into_inner(self) -> Box<dyn MatchesIterator> {
        self.inner
    }
}

impl MatchesIterator for FilterMatchesIterator {
    fn next(&mut self) -> Result<bool> {
        self.inner.next()
    }

    fn start_position(&self) -> i32 {
        self.inner.start_position()
    }

    fn end_position(&self) -> i32 {
        self.inner.end_position()
    }

    fn start_offset(&self) -> Result<i32> {
        self.inner.start_offset()
    }

    fn end_offset(&self) -> Result<i32> {
        self.inner.end_offset()
    }

    fn get_sub_matches(&self) -> Result<Option<Box<dyn MatchesIterator>>> {
        self.inner.get_sub_matches()
    }

    fn get_query(&self) -> Arc<dyn Query> {
        self.inner.get_query()
    }
}
