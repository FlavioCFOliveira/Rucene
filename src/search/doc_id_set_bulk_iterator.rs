//! Bulk iteration over a doc ID set, ported from
//! `org.apache.lucene.search.DocIdSetBulkIterator`.

#![deny(unsafe_code)]

use crate::search::collection_terminated_exception::CollectionResult;
use crate::search::collector::LeafCollector;
use crate::search::scorable::Scorable;
use crate::util::Bits;

/// Bulk iterator over a [`DocIdSetIterator`](crate::search::DocIdSetIterator).
///
/// Equivalent to `org.apache.lucene.search.DocIdSetBulkIterator`.
///
/// **Divergence from Lucene 10.5.0.** Java's `iterate` reaches the scorable
/// through the collector, which stored it in `setScorer`. This port passes it
/// explicitly, for the reason given in the
/// [collector module documentation](crate::search::collector).
pub trait DocIdSetBulkIterator {
    /// Iterates over the documents contained in this iterator and calls
    /// [`LeafCollector::collect`] on them.
    ///
    /// Equivalent to `DocIdSetBulkIterator.iterate(LeafCollector, Bits, int,
    /// int)`.
    ///
    /// # Errors
    ///
    /// Returns
    /// [`CollectionTerminated`](crate::search::CollectionError::CollectionTerminated)
    /// when the collector ends collection of this leaf early, and propagates
    /// any I/O error otherwise.
    fn iterate(
        &mut self,
        collector: &mut dyn LeafCollector,
        accept_docs: Option<&dyn Bits>,
        min: i32,
        max: i32,
        scorer: &mut dyn Scorable,
    ) -> CollectionResult<()>;
}
