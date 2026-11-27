//! Int sorting, ported from
//! `org.apache.lucene.search.comparators.IntComparator`.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::index::LeafReaderContext;
use crate::search::comparators::numeric_comparator::{
    NumericComparator, NumericDocValuesSource, NumericLeafState,
};
use crate::search::field_comparator::{
    java_int_compare, SharedLeafComparator, SlotComparator, SortValue,
};
use crate::search::leaf_field_comparator::LeafFieldComparator;
use crate::search::pruning::Pruning;
use crate::search::scorable::Scorable;
use crate::util::NumericUtils;

/// Comparator based on `Integer#compare` for `num_hits`.
///
/// Equivalent to `org.apache.lucene.search.comparators.IntComparator`. It provides a
/// skipping functionality — an iterator that can skip over non-competitive
/// documents — inherited from
/// [`NumericComparator`], which
/// this type embeds; see that type for the shape of the port.
#[derive(Debug)]
pub struct IntComparator {
    base: NumericComparator,
    values: Vec<i32>,
    /// The top value, as set by
    /// [`FieldComparator::set_top_value`](crate::search::FieldComparator::set_top_value).
    ///
    /// Equivalent to the `protected Integer topValue` field.
    top_value: i32,
    /// The value of the bottom slot of the queue.
    ///
    /// Equivalent to the `protected Integer bottom` field.
    pub bottom: i32,
    /// The value substituted for documents that have none.
    ///
    /// Equivalent to the `protected final T missingValue` field of
    /// `NumericComparator`, narrowed to this comparator's value type.
    missing_value: i32,
}

impl IntComparator {
    /// The `NumericComparator` state this comparator extends in Java, whose
    /// protected fields the package's tests set.
    pub fn numeric_comparator_mut(&mut self) -> &mut NumericComparator {
        &mut self.base
    }

    /// Creates a comparator over `num_hits` slots for `field`.
    ///
    /// Equivalent to
    /// `new IntComparator(int, String, Integer, boolean, Pruning)`; a `missing_value` of
    /// `None` is Java's `null`, which the constructor replaces with
    /// `0`.
    pub fn new(
        num_hits: usize,
        field: impl Into<String>,
        missing_value: Option<i32>,
        reverse: bool,
        pruning: Pruning,
    ) -> Self {
        let missing_value = missing_value.unwrap_or(0);
        let mut comparator = Self {
            base: NumericComparator::new(
                field,
                reverse,
                pruning,
                4, // `Integer.BYTES`
                Self::sortable_bytes_to_long,
                "org.apache.lucene.search.comparators.IntComparator",
            ),
            values: vec![0; num_hits],
            top_value: 0,
            bottom: 0,
            missing_value,
        };
        // `this.missingValueAsLong = missingValueAsComparableLong()`
        // (`NumericComparator.java:68`), which Java's constructor dispatches to
        // this class's override.
        comparator.base.missing_value_as_long = comparator.missing_value_as_comparable_long();
        comparator
    }

    /// Decodes the packed point bytes of this comparator's field into the
    /// comparable `long` its competitive iterator ranges over.
    ///
    /// Equivalent to `IntComparator#sortableBytesToLong`
    /// (`IntComparator.java:62-65`), which is
    /// `NumericUtils.sortableBytesToInt(bytes, 0)` — widened to `long` with sign
    /// extension, as Java's implicit `int`-to-`long` conversion does. It is an
    /// associated function because the override reads no state;
    /// [`NumericComparator`] calls it through the pointer [`new`](Self::new) hands
    /// over.
    pub fn sortable_bytes_to_long(bytes: &[u8]) -> i64 {
        i64::from(NumericUtils::sortable_bytes_to_int(bytes, 0))
    }

    /// Returns this comparator's missing value in the comparable long
    /// encoding the numeric comparator uses.
    ///
    /// Equivalent to `IntComparator.missingValueAsComparableLong()`
    /// (`IntComparator.java:57-60`), which returns the missing value itself,
    /// widened to `long`.
    pub fn missing_value_as_comparable_long(&self) -> i64 {
        let missing_value = self.missing_value;
        i64::from(missing_value)
    }

    /// Returns the shared numeric-comparator state.
    ///
    /// Equivalent to reaching the inherited `NumericComparator` fields.
    pub fn base(&self) -> &NumericComparator {
        &self.base
    }

    /// Installs a replacement for the default doc-values lookup.
    ///
    /// Equivalent to overriding
    /// `NumericLeafComparator.getNumericDocValues(LeafReaderContext, String)`,
    /// which is what `SortedNumericSortField.getComparator` does; see
    /// [`NumericDocValuesSource`].
    pub fn set_numeric_doc_values_source(&mut self, source: NumericDocValuesSource) {
        self.base.set_numeric_doc_values_source(source);
    }

    /// Equivalent to the private `IntComparator.IntLeafComparator.getValueForDoc(int)`.
    fn get_value_for_doc(&self, leaf: &mut NumericLeafState, doc: i32) -> Result<i32> {
        Ok(leaf
            .exact_long_value(doc)?
            .map_or(self.missing_value, |raw| raw as i32))
    }

    /// Equivalent to `IntComparator.IntLeafComparator.bottomAsComparableLong()`.
    fn bottom_as_comparable_long(&self) -> i64 {
        i64::from(self.bottom)
    }

    /// Equivalent to `IntComparator.IntLeafComparator.topAsComparableLong()`.
    fn top_as_comparable_long(&self) -> i64 {
        i64::from(self.top_value)
    }
}

impl SlotComparator for IntComparator {
    /// `IntComparator.IntLeafComparator` adds no field to
    /// `NumericLeafComparator`'s.
    type Leaf = NumericLeafState;

    fn compare(&self, slot1: i32, slot2: i32) -> i32 {
        java_int_compare(self.values[slot1 as usize], self.values[slot2 as usize])
    }

    fn set_top_value(&mut self, value: SortValue) {
        self.base.set_top_value();
        if let SortValue::Int(value) = value {
            self.top_value = value;
        }
    }

    fn value(&self, slot: i32) -> SortValue {
        SortValue::Int(self.values[slot as usize])
    }

    /// Equivalent to `IntComparator.getLeafComparator(LeafReaderContext)`,
    /// which returns `new IntLeafComparator(context)`.
    fn new_leaf(&mut self, context: &LeafReaderContext) -> Result<NumericLeafState> {
        let top = self.top_as_comparable_long();
        self.base.new_leaf(context, top)
    }

    fn set_single_sort(&mut self) {
        self.base.set_single_sort();
    }

    fn disable_skipping(&mut self) {
        self.base.disable_skipping();
    }

    fn set_bottom(&mut self, leaf: &mut NumericLeafState, slot: i32) -> Result<()> {
        self.bottom = self.values[slot as usize];
        self.base.set_bottom(
            leaf,
            self.bottom_as_comparable_long(),
            self.top_as_comparable_long(),
        )
    }

    fn compare_bottom(
        &mut self,
        leaf: &mut NumericLeafState,
        doc: i32,
        _scorer: &mut dyn Scorable,
    ) -> Result<i32> {
        let value = self.get_value_for_doc(leaf, doc)?;
        Ok(java_int_compare(self.bottom, value))
    }

    fn compare_top(
        &mut self,
        leaf: &mut NumericLeafState,
        doc: i32,
        _scorer: &mut dyn Scorable,
    ) -> Result<i32> {
        let value = self.get_value_for_doc(leaf, doc)?;
        Ok(java_int_compare(self.top_value, value))
    }

    fn copy(
        &mut self,
        leaf: &mut NumericLeafState,
        slot: i32,
        doc: i32,
        _scorer: &mut dyn Scorable,
    ) -> Result<()> {
        self.values[slot as usize] = self.get_value_for_doc(leaf, doc)?;
        self.base.copy(leaf, doc);
        Ok(())
    }

    fn set_scorer(&mut self, leaf: &mut NumericLeafState, scorer: &mut dyn Scorable) -> Result<()> {
        self.base.set_scorer(
            leaf,
            scorer,
            self.bottom_as_comparable_long(),
            self.top_as_comparable_long(),
        )
    }

    fn competitive_iterator(
        &mut self,
        leaf: &mut NumericLeafState,
    ) -> Result<Option<crate::search::CompetitiveIterator>> {
        Ok(self.base.competitive_iterator(leaf))
    }

    fn set_hits_threshold_reached(&mut self, leaf: &mut NumericLeafState) -> Result<()> {
        self.base.set_hits_threshold_reached(
            leaf,
            self.bottom_as_comparable_long(),
            self.top_as_comparable_long(),
        )
    }
}

/// Equivalent to `IntComparator.IntLeafComparator`: the leaf comparator of
/// [`IntComparator`] for one segment.
///
/// Java's leaf reads `bottom` and `missingValue`, fields of the enclosing
/// comparator, directly; here the enclosing comparator is shared state behind
/// an `AtomicRefCell` (see [`SharedLeafComparator`]), so the leaf keeps its own
/// copies: `missingValue` is final, and `bottom` is taken when the leaf is
/// created (the value Java's leaf would read before its first `setBottom`)
/// and after each `setBottom(slot)` (`bottom = values[slot]`,
/// `IntComparator.java` `IntLeafComparator.setBottom`), the only writer of the
/// field while the leaf collects.
///
/// PERF: `compareBottom`, called for every collected document, then borrows
/// nothing (rmp #680). Every other method goes through the shared state as
/// [`SharedLeafComparator`] does.
pub(crate) struct IntLeafComparator {
    inner: SharedLeafComparator<IntComparator>,
    bottom: i32,
    missing_value: i32,
}

impl IntLeafComparator {
    /// Wraps the shared leaf comparator `IntComparator.getLeafComparator`
    /// built.
    pub(crate) fn new(mut inner: SharedLeafComparator<IntComparator>) -> Self {
        let (bottom, missing_value) = {
            let (shared, _) = inner.parts_mut();
            let comparator = shared.borrow();
            (comparator.bottom, comparator.missing_value)
        };
        Self {
            inner,
            bottom,
            missing_value,
        }
    }
}

impl LeafFieldComparator for IntLeafComparator {
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    #[inline]
    fn set_bottom(&mut self, slot: i32) -> Result<()> {
        let (shared, leaf) = self.inner.parts_mut();
        let mut comparator = shared.borrow_mut();
        comparator.set_bottom(leaf, slot)?;
        self.bottom = comparator.bottom;
        Ok(())
    }

    /// `Integer.compare(bottom, getValueForDoc(doc))`.
    #[inline]
    fn compare_bottom(&mut self, doc: i32, _scorer: &mut dyn Scorable) -> Result<i32> {
        let (_, leaf) = self.inner.parts_mut();
        let value = leaf
            .exact_long_value(doc)?
            .map_or(self.missing_value, |raw| raw as i32);
        Ok(java_int_compare(self.bottom, value))
    }

    fn compare_top(&mut self, doc: i32, scorer: &mut dyn Scorable) -> Result<i32> {
        self.inner.compare_top(doc, scorer)
    }

    fn copy(&mut self, slot: i32, doc: i32, scorer: &mut dyn Scorable) -> Result<()> {
        self.inner.copy(slot, doc, scorer)
    }

    fn set_scorer(&mut self, scorer: &mut dyn Scorable) -> Result<()> {
        self.inner.set_scorer(scorer)
    }

    fn competitive_iterator(&mut self) -> Result<Option<crate::search::CompetitiveIterator>> {
        self.inner.competitive_iterator()
    }

    fn set_hits_threshold_reached(&mut self) -> Result<()> {
        self.inner.set_hits_threshold_reached()
    }
}
