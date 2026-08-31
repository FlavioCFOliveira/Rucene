//! Float sorting, ported from
//! `org.apache.lucene.search.comparators.FloatComparator`.

#![deny(unsafe_code)]

use std::any::Any;

use crate::error::Result;
use crate::index::LeafReaderContext;
use crate::search::comparators::numeric_comparator::{
    NumericComparator, NumericDocValuesSource, SortableBytes,
};
use crate::search::doc_id_set_iterator::DocIdSetIterator;
use crate::search::field_comparator::{java_float_compare, FieldComparator, SortValue};
use crate::search::leaf_field_comparator::LeafFieldComparator;
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
                SortableBytes::Int,
            ),
            values: vec![0.0; num_hits],
            top_value: 0.0,
            bottom: 0.0,
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

    /// Equivalent to the private `FloatComparator.FloatLeafComparator.getValueForDoc(int)`.
    fn get_value_for_doc(&mut self, doc: i32) -> Result<f32> {
        let missing_value = self.missing_value;
        let Some(doc_values) = self.base.doc_values() else {
            return Ok(missing_value);
        };
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

impl LeafFieldComparator for FloatComparator {
    fn set_bottom(&mut self, slot: i32) -> Result<()> {
        self.bottom = self.values[slot as usize];
        self.base.set_bottom(
            self.bottom_as_comparable_long(),
            self.top_as_comparable_long(),
        )
    }

    fn compare_bottom(&mut self, doc: i32, _scorer: &mut dyn Scorable) -> Result<i32> {
        let value = self.get_value_for_doc(doc)?;
        Ok(java_float_compare(self.bottom, value))
    }

    fn compare_top(&mut self, doc: i32, _scorer: &mut dyn Scorable) -> Result<i32> {
        let value = self.get_value_for_doc(doc)?;
        Ok(java_float_compare(self.top_value, value))
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

impl FieldComparator for FloatComparator {
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
