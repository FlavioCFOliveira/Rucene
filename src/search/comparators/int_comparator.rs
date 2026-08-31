//! Int sorting, ported from
//! `org.apache.lucene.search.comparators.IntComparator`.

#![deny(unsafe_code)]

use std::any::Any;

use crate::error::Result;
use crate::index::LeafReaderContext;
use crate::search::comparators::numeric_comparator::{
    NumericComparator, NumericDocValuesSource, SortableBytes,
};
use crate::search::doc_id_set_iterator::DocIdSetIterator;
use crate::search::field_comparator::{java_int_compare, FieldComparator, SortValue};
use crate::search::leaf_field_comparator::LeafFieldComparator;
use crate::search::pruning::Pruning;
use crate::search::scorable::Scorable;

/// Comparator based on `Integer#compare` for `num_hits`.
///
/// Equivalent to `org.apache.lucene.search.comparators.IntComparator`. It provides a
/// skipping functionality — an iterator that can skip over non-competitive
/// documents — inherited from
/// [`NumericComparator`](crate::search::comparators::NumericComparator), which
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
    bottom: i32,
    /// The value substituted for documents that have none.
    ///
    /// Equivalent to the `protected final T missingValue` field of
    /// `NumericComparator`, narrowed to this comparator's value type.
    missing_value: i32,
}

impl IntComparator {
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
        Self {
            base: NumericComparator::new(
                field,
                i64::from(missing_value),
                reverse,
                pruning,
                SortableBytes::Int,
            ),
            values: vec![0; num_hits],
            top_value: 0,
            bottom: 0,
            missing_value,
        }
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
    fn get_value_for_doc(&mut self, doc: i32) -> Result<i32> {
        let missing_value = self.missing_value;
        let Some(doc_values) = self.base.doc_values() else {
            return Ok(missing_value);
        };
        if doc_values.advance_exact(doc)? {
            let raw = doc_values.long_value()?;
            Ok(raw as i32)
        } else {
            Ok(missing_value)
        }
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

impl LeafFieldComparator for IntComparator {
    fn set_bottom(&mut self, slot: i32) -> Result<()> {
        self.bottom = self.values[slot as usize];
        self.base.set_bottom(
            self.bottom_as_comparable_long(),
            self.top_as_comparable_long(),
        )
    }

    fn compare_bottom(&mut self, doc: i32, _scorer: &mut dyn Scorable) -> Result<i32> {
        let value = self.get_value_for_doc(doc)?;
        Ok(java_int_compare(self.bottom, value))
    }

    fn compare_top(&mut self, doc: i32, _scorer: &mut dyn Scorable) -> Result<i32> {
        let value = self.get_value_for_doc(doc)?;
        Ok(java_int_compare(self.top_value, value))
    }

    fn copy(&mut self, slot: i32, doc: i32, _scorer: &mut dyn Scorable) -> Result<()> {
        self.values[slot as usize] = self.get_value_for_doc(doc)?;
        self.base.copy(doc);
        Ok(())
    }

    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        self.base.set_scorer(
            self.bottom_as_comparable_long(),
            self.top_as_comparable_long(),
        )
    }

    fn competitive_iterator(&mut self) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        Ok(self.base.competitive_iterator())
    }

    fn set_hits_threshold_reached(&mut self) -> Result<()> {
        self.base.set_hits_threshold_reached(
            self.bottom_as_comparable_long(),
            self.top_as_comparable_long(),
        )
    }
}

impl FieldComparator for IntComparator {
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

    fn get_leaf_comparator(&mut self, context: &LeafReaderContext) -> Result<()> {
        let top = self.top_as_comparable_long();
        self.base.set_next_leaf(context, top)
    }

    fn as_leaf_comparator(&mut self) -> &mut dyn LeafFieldComparator {
        self
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn set_single_sort(&mut self) {
        self.base.set_single_sort();
    }

    fn disable_skipping(&mut self) {
        self.base.disable_skipping();
    }
}
