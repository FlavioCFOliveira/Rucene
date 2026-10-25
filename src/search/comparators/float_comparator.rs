//! Float sorting, ported from
//! `org.apache.lucene.search.comparators.FloatComparator`.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::index::LeafReaderContext;
use crate::search::comparators::numeric_comparator::{
    NumericComparator, NumericDocValuesSource, NumericLeafState,
};
use crate::search::doc_id_set_iterator::DocIdSetIterator;
use crate::search::field_comparator::{java_float_compare, SlotComparator, SortValue};
use crate::search::pruning::Pruning;
use crate::search::scorable::Scorable;
use crate::util::NumericUtils;

/// Comparator based on `Float#compare` for `num_hits`.
///
/// Equivalent to `org.apache.lucene.search.comparators.FloatComparator`. It provides a
/// skipping functionality — an iterator that can skip over non-competitive
/// documents — inherited from
/// [`NumericComparator`](crate::search::comparators::NumericComparator), which
/// this type embeds; see that type for the shape of the port.
#[derive(Debug)]
pub struct FloatComparator {
    base: NumericComparator,
    values: Vec<f32>,
    /// The top value, as set by
    /// [`FieldComparator::set_top_value`](crate::search::FieldComparator::set_top_value).
    ///
    /// Equivalent to the `protected Float topValue` field.
    top_value: f32,
    /// The value of the bottom slot of the queue.
    ///
    /// Equivalent to the `protected Float bottom` field.
    bottom: f32,
    /// The value substituted for documents that have none.
    ///
    /// Equivalent to the `protected final T missingValue` field of
    /// `NumericComparator`, narrowed to this comparator's value type.
    missing_value: f32,
}

impl FloatComparator {
    /// Creates a comparator over `num_hits` slots for `field`.
    ///
    /// Equivalent to
    /// `new FloatComparator(int, String, Float, boolean, Pruning)`; a `missing_value` of
    /// `None` is Java's `null`, which the constructor replaces with
    /// `0.0f`.
    pub fn new(
        num_hits: usize,
        field: impl Into<String>,
        missing_value: Option<f32>,
        reverse: bool,
        pruning: Pruning,
    ) -> Self {
        let missing_value = missing_value.unwrap_or(0.0);
        Self {
            base: NumericComparator::new(
                field,
                i64::from(NumericUtils::float_to_sortable_int(missing_value)),
                reverse,
                pruning,
                4, // `Float.BYTES`
                Self::sortable_bytes_to_long,
            ),
            values: vec![0.0; num_hits],
            top_value: 0.0,
            bottom: 0.0,
            missing_value,
        }
    }

    /// Decodes the packed point bytes of this comparator's field into the
    /// comparable `long` its competitive iterator ranges over.
    ///
    /// Equivalent to `FloatComparator#sortableBytesToLong`
    /// (`FloatComparator.java:62-65`), which is
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
    /// Equivalent to `FloatComparator.missingValueAsComparableLong()`, which is
    /// `NumericUtils.floatToSortableInt` of the missing value.
    pub fn missing_value_as_comparable_long(&self) -> i64 {
        let missing_value = self.missing_value;
        i64::from(NumericUtils::float_to_sortable_int(missing_value))
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

    /// Equivalent to the private `FloatComparator.FloatLeafComparator.getValueForDoc(int)`.
    fn get_value_for_doc(&self, leaf: &mut NumericLeafState, doc: i32) -> Result<f32> {
        let missing_value = self.missing_value;
        let doc_values = leaf.doc_values();
        if doc_values.advance_exact(doc)? {
            let raw = doc_values.long_value()?;
            Ok(f32::from_bits(raw as i32 as u32))
        } else {
            Ok(missing_value)
        }
    }

    /// Equivalent to `FloatComparator.FloatLeafComparator.bottomAsComparableLong()`.
    fn bottom_as_comparable_long(&self) -> i64 {
        i64::from(NumericUtils::float_to_sortable_int(self.bottom))
    }

    /// Equivalent to `FloatComparator.FloatLeafComparator.topAsComparableLong()`.
    fn top_as_comparable_long(&self) -> i64 {
        i64::from(NumericUtils::float_to_sortable_int(self.top_value))
    }
}

impl SlotComparator for FloatComparator {
    /// `FloatComparator.FloatLeafComparator` adds no field to
    /// `NumericLeafComparator`'s.
    type Leaf = NumericLeafState;

    fn compare(&self, slot1: i32, slot2: i32) -> i32 {
        java_float_compare(self.values[slot1 as usize], self.values[slot2 as usize])
    }

    fn set_top_value(&mut self, value: SortValue) {
        self.base.set_top_value();
        if let SortValue::Float(value) = value {
            self.top_value = value;
        }
    }

    fn value(&self, slot: i32) -> SortValue {
        SortValue::Float(self.values[slot as usize])
    }

    /// Equivalent to `FloatComparator.getLeafComparator(LeafReaderContext)`,
    /// which returns `new FloatLeafComparator(context)`.
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
        Ok(java_float_compare(self.bottom, value))
    }

    fn compare_top(
        &mut self,
        leaf: &mut NumericLeafState,
        doc: i32,
        _scorer: &mut dyn Scorable,
    ) -> Result<i32> {
        let value = self.get_value_for_doc(leaf, doc)?;
        Ok(java_float_compare(self.top_value, value))
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

    fn set_scorer(
        &mut self,
        leaf: &mut NumericLeafState,
        _scorer: &mut dyn Scorable,
    ) -> Result<()> {
        self.base.set_scorer(
            leaf,
            self.bottom_as_comparable_long(),
            self.top_as_comparable_long(),
        )
    }

    fn competitive_iterator(
        &mut self,
        leaf: &mut NumericLeafState,
    ) -> Result<Option<Box<dyn DocIdSetIterator>>> {
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
