//! Per-segment comparison, ported from
//! `org.apache.lucene.search.LeafFieldComparator`.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::search::doc_id_set_iterator::DocIdSetIterator;
use crate::search::scorable::Scorable;

/// Expert: comparator that gets instantiated on each leaf from a top-level
/// [`FieldComparator`](crate::search::FieldComparator) instance.
///
/// Equivalent to the interface `org.apache.lucene.search.LeafFieldComparator`.
///
/// A leaf comparator must define these functions:
///
/// * [`set_bottom`](Self::set_bottom) — called by
///   [`FieldValueHitQueue`](crate::search::FieldValueHitQueue) to notify the
///   comparator of the current weakest ("bottom") slot. Note that this slot may
///   not hold the weakest value according to this comparator, in cases where it
///   is not the primary one (that is, is only used to break ties from the
///   comparators before it).
/// * [`compare_bottom`](Self::compare_bottom) — compare a new hit (doc ID)
///   against the "weakest" (bottom) entry in the queue.
/// * [`compare_top`](Self::compare_top) — compare a new hit (doc ID) against
///   the top value previously set by a call to
///   [`FieldComparator::set_top_value`](crate::search::FieldComparator::set_top_value).
/// * [`copy`](Self::copy) — install a new hit into the priority queue. The
///   [`FieldValueHitQueue`](crate::search::FieldValueHitQueue) calls this method
///   when a new hit is competitive.
///
/// # Adaptation: where the [`Scorable`] lives
///
/// **Divergence from Lucene 10.5.0.** In Java a leaf comparator keeps the
/// [`Scorable`] handed to `setScorer(Scorable)` in a field and reads it back
/// from `compareBottom`, `copy` and `compareTop`; `FieldComparator.RelevanceComparator`
/// is exactly such a comparator. That is an alias: the bulk scorer driving
/// iteration and the comparator reading scores hold the same live object and
/// both mutate it. Rust forbids that aliasing, so — following the same rule
/// [`LeafCollector`](crate::search::LeafCollector) already applies — the
/// scorable is *passed* to every call that could read it instead of being
/// stored. [`set_scorer`](Self::set_scorer) still exists and is still called
/// once before collection, exactly where Java calls it, so a comparator that
/// reacts to the scorer at that point behaves identically.
pub trait LeafFieldComparator {
    /// Sets the bottom slot, that is the "weakest" (sorted last) entry in the
    /// queue.
    ///
    /// Equivalent to `LeafFieldComparator.setBottom(int)`. When
    /// [`compare_bottom`](Self::compare_bottom) is called, the comparison
    /// should be against this slot. This is always called before
    /// [`compare_bottom`](Self::compare_bottom).
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the slot's value.
    fn set_bottom(&mut self, slot: i32) -> Result<()>;

    /// Compares the bottom of the queue with this doc.
    ///
    /// Equivalent to `LeafFieldComparator.compareBottom(int)`. It is only
    /// invoked after [`set_bottom`](Self::set_bottom) has been called, and
    /// returns the same result as
    /// [`FieldComparator::compare`](crate::search::FieldComparator::compare)
    /// would as if bottom were `slot1` and the new document were `slot2`.
    ///
    /// For a search that hits many results, this method is the hotspot —
    /// invoked by far the most frequently.
    ///
    /// Returns any `N < 0` if the doc's value is sorted after the bottom entry
    /// (not competitive), any `N > 0` if the doc's value is sorted before the
    /// bottom entry, and `0` if they are equal.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the document's value.
    fn compare_bottom(&mut self, doc: i32, scorer: &mut dyn Scorable) -> Result<i32>;

    /// Compares the top value with this doc.
    ///
    /// Equivalent to `LeafFieldComparator.compareTop(int)`. It is only invoked
    /// after
    /// [`FieldComparator::set_top_value`](crate::search::FieldComparator::set_top_value)
    /// has been called, and returns the same result as
    /// [`FieldComparator::compare`](crate::search::FieldComparator::compare)
    /// would as if the top value were `slot1` and the new document were
    /// `slot2`. This is only called for searches that use `search_after` (deep
    /// paging).
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the document's value.
    fn compare_top(&mut self, doc: i32, scorer: &mut dyn Scorable) -> Result<i32>;

    /// Called when a new hit is competitive: copies any state associated with
    /// the document that is required for future comparisons into `slot`.
    ///
    /// Equivalent to `LeafFieldComparator.copy(int, int)`. `doc` is relative to
    /// the current reader.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the document's value.
    fn copy(&mut self, slot: i32, doc: i32, scorer: &mut dyn Scorable) -> Result<()>;

    /// Sets the scorer to use in case a document's score is needed.
    ///
    /// Equivalent to `LeafFieldComparator.setScorer(Scorable)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while preparing for collection.
    fn set_scorer(&mut self, scorer: &mut dyn Scorable) -> Result<()>;

    /// Returns an iterator over competitive documents — those that are stronger
    /// than the already collected ones — or `None` when such an iterator is not
    /// available for this comparator or segment.
    ///
    /// Equivalent to `LeafFieldComparator.competitiveIterator()`, which returns
    /// `null` by default.
    ///
    /// **Divergence from Lucene 10.5.0.** Java hands out the very object the
    /// comparator keeps updating during collection. Rust cannot give out that
    /// alias, so the comparators that provide one return a shared handle onto
    /// the same state; see
    /// [`UpdateableDocIdSetIterator`](crate::search::comparators::UpdateableDocIdSetIterator).
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while building the iterator.
    fn competitive_iterator(&mut self) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        Ok(None)
    }

    /// Informs this leaf comparator that the hits threshold has been reached.
    ///
    /// Equivalent to `LeafFieldComparator.setHitsThresholdReached()`, a no-op
    /// by default. It is called from a collector when the hits threshold is
    /// reached.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reacting to the notification.
    fn set_hits_threshold_reached(&mut self) -> Result<()> {
        Ok(())
    }
}

impl<T: LeafFieldComparator + ?Sized> LeafFieldComparator for &mut T {
    fn set_bottom(&mut self, slot: i32) -> Result<()> {
        (**self).set_bottom(slot)
    }

    fn compare_bottom(&mut self, doc: i32, scorer: &mut dyn Scorable) -> Result<i32> {
        (**self).compare_bottom(doc, scorer)
    }

    fn compare_top(&mut self, doc: i32, scorer: &mut dyn Scorable) -> Result<i32> {
        (**self).compare_top(doc, scorer)
    }

    fn copy(&mut self, slot: i32, doc: i32, scorer: &mut dyn Scorable) -> Result<()> {
        (**self).copy(slot, doc, scorer)
    }

    fn set_scorer(&mut self, scorer: &mut dyn Scorable) -> Result<()> {
        (**self).set_scorer(scorer)
    }

    fn competitive_iterator(&mut self) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        (**self).competitive_iterator()
    }

    fn set_hits_threshold_reached(&mut self) -> Result<()> {
        (**self).set_hits_threshold_reached()
    }
}
