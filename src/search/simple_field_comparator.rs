//! Single-object comparators, ported from
//! `org.apache.lucene.search.SimpleFieldComparator`.

#![deny(unsafe_code)]

use std::any::Any;
use std::fmt::Debug;

use crate::error::Result;
use crate::index::LeafReaderContext;
use crate::search::field_comparator::{FieldComparator, SortValue};
use crate::search::leaf_field_comparator::LeafFieldComparator;

/// The behaviour a [`SimpleFieldComparator`] is built from.
///
/// Equivalent to what a Java subclass of
/// `org.apache.lucene.search.SimpleFieldComparator` overrides. Java expresses
/// "comparator and leaf comparator in one object" by having the abstract class
/// implement both interfaces and return `this` from `getLeafComparator`; Rust
/// has no implementation inheritance, so the comparing half is this trait and
/// [`SimpleFieldComparator`] supplies the [`FieldComparator`] half around it.
pub trait SimpleFieldComparatorImpl: LeafFieldComparator + Debug + 'static {
    /// Compares the hit at `slot1` with the hit at `slot2`.
    ///
    /// Equivalent to `FieldComparator.compare(int, int)`, which
    /// `SimpleFieldComparator` leaves abstract.
    fn compare(&self, slot1: i32, slot2: i32) -> i32;

    /// Records the top value.
    ///
    /// Equivalent to `FieldComparator.setTopValue(T)`, which
    /// `SimpleFieldComparator` leaves abstract.
    fn set_top_value(&mut self, value: SortValue);

    /// Returns the actual value in `slot`.
    ///
    /// Equivalent to `FieldComparator.value(int)`, which
    /// `SimpleFieldComparator` leaves abstract.
    fn value(&self, slot: i32) -> SortValue;

    /// Called before collecting `context`.
    ///
    /// Equivalent to `SimpleFieldComparator.doSetNextReader(LeafReaderContext)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while opening the segment's values.
    fn do_set_next_reader(&mut self, context: &LeafReaderContext) -> Result<()>;

    /// Compares two slot values.
    ///
    /// Equivalent to `FieldComparator.compareValues(T, T)`; the default matches
    /// the inherited one.
    fn compare_values(&self, first: &SortValue, second: &SortValue) -> i32 {
        first.compare_to(second)
    }

    /// Informs the comparator that the sort is done on this single field.
    ///
    /// Equivalent to `FieldComparator.setSingleSort()`.
    fn set_single_sort(&mut self) {}

    /// Informs the comparator that skipping documents should be disabled.
    ///
    /// Equivalent to `FieldComparator.disableSkipping()`.
    fn disable_skipping(&mut self) {}
}

/// Base [`FieldComparator`] implementation that is used for all contexts: it is
/// its own leaf comparator.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.search.SimpleFieldComparator<T>`. Supply the comparing
/// behaviour as a [`SimpleFieldComparatorImpl`] and wrap it here.
#[derive(Debug, Clone, Copy, Default)]
pub struct SimpleFieldComparator<T: SimpleFieldComparatorImpl> {
    inner: T,
}

impl<T: SimpleFieldComparatorImpl> SimpleFieldComparator<T> {
    /// Wraps the given comparing behaviour.
    pub fn new(inner: T) -> Self {
        Self { inner }
    }

    /// Returns the wrapped behaviour.
    pub fn inner(&self) -> &T {
        &self.inner
    }

    /// Returns the wrapped behaviour for mutation.
    pub fn inner_mut(&mut self) -> &mut T {
        &mut self.inner
    }

    /// Unwraps this comparator, returning the behaviour it was built from.
    pub fn into_inner(self) -> T {
        self.inner
    }
}

impl<T: SimpleFieldComparatorImpl> LeafFieldComparator for SimpleFieldComparator<T> {
    fn set_bottom(&mut self, slot: i32) -> Result<()> {
        self.inner.set_bottom(slot)
    }

    fn compare_bottom(
        &mut self,
        doc: i32,
        scorer: &mut dyn crate::search::scorable::Scorable,
    ) -> Result<i32> {
        self.inner.compare_bottom(doc, scorer)
    }

    fn compare_top(
        &mut self,
        doc: i32,
        scorer: &mut dyn crate::search::scorable::Scorable,
    ) -> Result<i32> {
        self.inner.compare_top(doc, scorer)
    }

    fn copy(
        &mut self,
        slot: i32,
        doc: i32,
        scorer: &mut dyn crate::search::scorable::Scorable,
    ) -> Result<()> {
        self.inner.copy(slot, doc, scorer)
    }

    /// Equivalent to `SimpleFieldComparator.setScorer(Scorable)`, which is a
    /// no-op unless the wrapped behaviour overrides it.
    fn set_scorer(&mut self, scorer: &mut dyn crate::search::scorable::Scorable) -> Result<()> {
        self.inner.set_scorer(scorer)
    }

    fn competitive_iterator(
        &mut self,
    ) -> Result<Option<Box<dyn crate::search::doc_id_set_iterator::DocIdSetIterator>>> {
        self.inner.competitive_iterator()
    }

    fn set_hits_threshold_reached(&mut self) -> Result<()> {
        self.inner.set_hits_threshold_reached()
    }
}

impl<T: SimpleFieldComparatorImpl> FieldComparator for SimpleFieldComparator<T> {
    fn compare(&self, slot1: i32, slot2: i32) -> i32 {
        self.inner.compare(slot1, slot2)
    }

    fn set_top_value(&mut self, value: SortValue) {
        self.inner.set_top_value(value);
    }

    fn value(&self, slot: i32) -> SortValue {
        self.inner.value(slot)
    }

    /// Equivalent to the `final SimpleFieldComparator.getLeafComparator`, which
    /// calls `doSetNextReader(context)` and returns `this`.
    fn get_leaf_comparator(&mut self, context: &LeafReaderContext) -> Result<()> {
        self.inner.do_set_next_reader(context)
    }

    fn as_leaf_comparator(&mut self) -> &mut dyn LeafFieldComparator {
        self
    }

    fn as_any(&self) -> &dyn Any {
        &self.inner
    }

    fn compare_values(&self, first: &SortValue, second: &SortValue) -> i32 {
        self.inner.compare_values(first, second)
    }

    fn set_single_sort(&mut self) {
        self.inner.set_single_sort();
    }

    fn disable_skipping(&mut self) {
        self.inner.disable_skipping();
    }
}
