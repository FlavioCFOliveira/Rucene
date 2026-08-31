//! Doc-values range confirmation, ported from
//! `org.apache.lucene.search.DocValuesRangeIterator`.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::error::Result;
use crate::index::{
    DocValuesSkipper, NumericDocValues, SortedDocValues, SortedNumericDocValues,
    SortedSetDocValues, TermsEnum,
};
use crate::search::doc_id_set_iterator::{empty, DocIdSetIterator, EmptyDocIdSetIterator};
use crate::search::doc_values_iteration::{
    NumericDocValuesIterator, SortedDocValuesIterator, SortedNumericDocValuesIterator,
    SortedSetDocValuesIterator,
};
use crate::search::skip_block_range_iterator::{Match, SkipBlockRangeIterator};
use crate::search::two_phase_iterator::TwoPhaseIterator;
use crate::util::{FixedBitSet, LongBitSet};

/// The doc values a [`DocValuesRangeIterator`] confirms matches against.
///
/// Equivalent to the `disi` field of `DocValuesRangeIterator`'s subclasses,
/// which in Java is the very doc-values instance the predicate reads.
enum RangeValues {
    Numeric(NumericDocValuesIterator),
    SortedNumeric(SortedNumericDocValuesIterator),
    Sorted(SortedDocValuesIterator),
    SortedSet(SortedSetDocValuesIterator),
    /// The approximation of `EmptyRangeIterator`.
    Empty(EmptyDocIdSetIterator),
}

impl RangeValues {
    /// Returns the values as the [`DocIdSetIterator`] Java treats them as.
    fn as_iterator(&mut self) -> &mut dyn DocIdSetIterator {
        match self {
            RangeValues::Numeric(values) => values,
            RangeValues::SortedNumeric(values) => values,
            RangeValues::Sorted(values) => values,
            RangeValues::SortedSet(values) => values,
            RangeValues::Empty(values) => values,
        }
    }

    /// The shared-borrow sibling of [`as_iterator`](Self::as_iterator).
    fn as_iterator_ref(&self) -> &dyn DocIdSetIterator {
        match self {
            RangeValues::Numeric(values) => values,
            RangeValues::SortedNumeric(values) => values,
            RangeValues::Sorted(values) => values,
            RangeValues::SortedSet(values) => values,
            RangeValues::Empty(values) => values,
        }
    }
}

/// The confirmation each factory installs.
///
/// Equivalent to the `IOBooleanSupplier check` that every
/// `DocValuesRangeIterator` factory builds as a lambda over the doc values.
enum RangePredicate {
    /// `values.longValue()` is within `[min, max]`.
    NumericRange { min: i64, max: i64 },
    /// The first value that is `>= min` is also `<= max`.
    SortedNumericRange { min: i64, max: i64 },
    /// `values.ordValue()` is within `[min, max]`.
    OrdinalRange { min: i64, max: i64 },
    /// The first ordinal that is `>= min` is also `<= max`.
    SortedSetOrdinalRange { min: i64, max: i64 },
    /// `ords` holds `values.ordValue()`.
    SortedOrdinalSet { ords: Arc<LongBitSet> },
    /// Some ordinal of the document is in `[min, max]` and is set in `ords`.
    SortedSetOrdinalSet {
        min: i64,
        max: i64,
        ords: Arc<LongBitSet>,
    },
    /// Nothing ever matches, as in `EmptyRangeIterator`.
    Never,
}

/// A set of ordinals a query matches, with the bounds that frame it.
///
/// Equivalent to the private record `DocValuesRangeIterator.OrdinalSet`.
struct OrdinalSet {
    min: i64,
    max: i64,
    ords: Arc<LongBitSet>,
    contiguous: bool,
}

impl OrdinalSet {
    /// Whether the set cannot intersect the values a skipper covers.
    ///
    /// Equivalent to `OrdinalSet.disjoint(DocValuesSkipper)`.
    fn disjoint(&self, skipper: Option<&dyn DocValuesSkipper>) -> bool {
        match skipper {
            None => false,
            Some(skipper) => {
                self.min > skipper.global_max_value() || self.max < skipper.global_min_value()
            }
        }
    }
}

/// Builds the set of ordinals a terms enumeration covers.
///
/// Equivalent to the private static
/// `DocValuesRangeIterator.buildOrdinalSet(TermsEnum, long)`, which returns
/// `null` for an empty enumeration.
///
/// # Errors
///
/// Propagates any I/O error raised while enumerating, and the [`LongBitSet`]
/// construction error for an invalid ordinal count.
fn build_ordinal_set(terms_enum: &mut dyn TermsEnum, ord_count: i64) -> Result<Option<OrdinalSet>> {
    if terms_enum.next()?.is_none() {
        return Ok(None);
    }
    let mut ords = LongBitSet::new(ord_count)?;
    let min = terms_enum.ord()?;
    ords.set(min);
    let mut max = min;
    // Distinct ordinals are counted through `get_and_set`, so that a terms
    // enumeration yielding a duplicate ordinal does not fool the contiguity
    // check below. The first set bit — the minimum — is always new on a fresh
    // bit set.
    let mut distinct_count: i64 = 1;
    while terms_enum.next()?.is_some() {
        max = terms_enum.ord()?;
        if !ords.get_and_set(max) {
            distinct_count += 1;
        }
    }
    // If every ordinal in `[min, max]` is set, the set is equivalent to an
    // ordinal range and can use the cheaper range check plus the block-level
    // "YES" short circuit.
    Ok(Some(OrdinalSet {
        min,
        max,
        ords: Arc::new(ords),
        contiguous: distinct_count == max - min + 1,
    }))
}

/// A [`TwoPhaseIterator`] that confirms doc-values matches, using a
/// [`DocValuesSkipper`] to skip whole blocks of documents when it can.
///
/// Equivalent to the sealed abstract class
/// `org.apache.lucene.search.DocValuesRangeIterator` and its six
/// implementations, which differ only in which doc values they read, whether a
/// skipper is present, and how a partially-matching block is evaluated in bulk.
/// Rust has no sealed hierarchies, so the variation is carried by the fields
/// below; every branch runs the same code Java's corresponding subclass does.
pub struct DocValuesRangeIterator {
    values: RangeValues,
    /// The block iterator over the skipper, which is the approximation when it
    /// is present.
    ///
    /// Equivalent to `DocValuesBlockRangeIterator.blockIterator`; `None` is the
    /// `DocValuesValueRangeIterator` shape, whose approximation is the doc
    /// values themselves.
    block_iterator: Option<SkipBlockRangeIterator>,
    predicate: RangePredicate,
    match_cost: f32,
    /// Whether block-level short circuits apply.
    ///
    /// Equivalent to being a `BulkBlockRangeIterator` rather than a plain
    /// `DocValuesBlockRangeIterator`.
    bulk: bool,
    /// The value bounds a numeric bulk block evaluates with.
    ///
    /// Equivalent to the `minValue`/`maxValue` fields of
    /// `BulkNumericRangeIterator` and `BulkSortedNumericRangeIterator`.
    bulk_bounds: (i64, i64),
}

impl std::fmt::Debug for DocValuesRangeIterator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocValuesRangeIterator")
            .field("has_block_iterator", &self.block_iterator.is_some())
            .field("match_cost", &self.match_cost)
            .field("bulk", &self.bulk)
            .finish_non_exhaustive()
    }
}

impl DocValuesRangeIterator {
    /// Confirms that a document's numeric value is within `[min, max]`.
    ///
    /// Equivalent to
    /// `DocValuesRangeIterator.forRange(NumericDocValues, DocValuesSkipper, long, long)`.
    pub fn for_range(
        values: Box<dyn NumericDocValues>,
        skipper: Option<Box<dyn DocValuesSkipper>>,
        min: i64,
        max: i64,
    ) -> Self {
        let predicate = RangePredicate::NumericRange { min, max };
        match skipper {
            None => Self {
                values: RangeValues::Numeric(NumericDocValuesIterator::new(values)),
                block_iterator: None,
                predicate,
                match_cost: 2.0,
                bulk: false,
                bulk_bounds: (min, max),
            },
            Some(skipper) => Self {
                values: RangeValues::Numeric(NumericDocValuesIterator::new(values)),
                block_iterator: Some(SkipBlockRangeIterator::new(skipper, min, max)),
                predicate,
                match_cost: 2.0,
                bulk: true,
                bulk_bounds: (min, max),
            },
        }
    }

    /// Confirms that one of a document's numeric values is within
    /// `[min, max]`.
    ///
    /// Equivalent to
    /// `DocValuesRangeIterator.forRange(SortedNumericDocValues, DocValuesSkipper, long, long)`.
    pub fn for_sorted_numeric_range(
        values: Box<dyn SortedNumericDocValues>,
        skipper: Option<Box<dyn DocValuesSkipper>>,
        min: i64,
        max: i64,
    ) -> Self {
        let predicate = RangePredicate::SortedNumericRange { min, max };
        match skipper {
            None => Self {
                values: RangeValues::SortedNumeric(SortedNumericDocValuesIterator::new(values)),
                block_iterator: None,
                predicate,
                match_cost: 5.0,
                bulk: false,
                bulk_bounds: (min, max),
            },
            Some(skipper) => Self {
                values: RangeValues::SortedNumeric(SortedNumericDocValuesIterator::new(values)),
                block_iterator: Some(SkipBlockRangeIterator::new(skipper, min, max)),
                predicate,
                // Java passes a match cost of 2 to the bulk sorted-numeric
                // iterator, not the 5 of the non-bulk one.
                match_cost: 2.0,
                bulk: true,
                bulk_bounds: (min, max),
            },
        }
    }

    /// Confirms that a document's ordinal is within `[min, max]`.
    ///
    /// Equivalent to
    /// `DocValuesRangeIterator.forOrdinalRange(SortedDocValues, DocValuesSkipper, long, long)`.
    pub fn for_ordinal_range(
        values: Box<dyn SortedDocValues>,
        skipper: Option<Box<dyn DocValuesSkipper>>,
        min: i64,
        max: i64,
    ) -> Self {
        let predicate = RangePredicate::OrdinalRange { min, max };
        Self {
            values: RangeValues::Sorted(SortedDocValuesIterator::new(values)),
            block_iterator: skipper.map(|skipper| SkipBlockRangeIterator::new(skipper, min, max)),
            predicate,
            match_cost: 2.0,
            bulk: true,
            bulk_bounds: (min, max),
        }
        .with_bulk_only_when_skipping()
    }

    /// Confirms that one of a document's ordinals is within `[min, max]`.
    ///
    /// Equivalent to
    /// `DocValuesRangeIterator.forOrdinalRange(SortedSetDocValues, DocValuesSkipper, long, long)`.
    pub fn for_sorted_set_ordinal_range(
        values: Box<dyn SortedSetDocValues>,
        skipper: Option<Box<dyn DocValuesSkipper>>,
        min: i64,
        max: i64,
    ) -> Self {
        let predicate = RangePredicate::SortedSetOrdinalRange { min, max };
        Self {
            values: RangeValues::SortedSet(SortedSetDocValuesIterator::new(values)),
            block_iterator: skipper.map(|skipper| SkipBlockRangeIterator::new(skipper, min, max)),
            predicate,
            match_cost: 5.0,
            bulk: true,
            bulk_bounds: (min, max),
        }
        .with_bulk_only_when_skipping()
    }

    /// Turns off the block short circuits when there is no block iterator,
    /// which is the `DocValuesValueRangeIterator` shape.
    fn with_bulk_only_when_skipping(mut self) -> Self {
        if self.block_iterator.is_none() {
            self.bulk = false;
        }
        self
    }

    /// Confirms that a document's ordinal belongs to the set the terms
    /// enumeration covers.
    ///
    /// Equivalent to
    /// `DocValuesRangeIterator.forOrdinalSet(SortedDocValues, DocValuesSkipper, TermsEnum)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while enumerating the terms.
    pub fn for_ordinal_set(
        values: Box<dyn SortedDocValues>,
        skipper: Option<Box<dyn DocValuesSkipper>>,
        terms: &mut dyn TermsEnum,
    ) -> Result<Self> {
        let ord_count = i64::from(values.get_value_count()?);
        let ordinal_set = build_ordinal_set(terms, ord_count)?;
        let Some(ordinal_set) = ordinal_set else {
            return Ok(Self::empty());
        };
        if ordinal_set.disjoint(skipper.as_deref()) {
            return Ok(Self::empty());
        }
        if ordinal_set.contiguous {
            return Ok(Self::for_ordinal_range(
                values,
                skipper,
                ordinal_set.min,
                ordinal_set.max,
            ));
        }
        let predicate = RangePredicate::SortedOrdinalSet {
            ords: Arc::clone(&ordinal_set.ords),
        };
        Ok(Self {
            values: RangeValues::Sorted(SortedDocValuesIterator::new(values)),
            block_iterator: skipper.map(|skipper| {
                SkipBlockRangeIterator::new(skipper, ordinal_set.min, ordinal_set.max)
            }),
            predicate,
            match_cost: 2.0,
            // A non-contiguous ordinal set uses the plain block iterator, which
            // has no block-level short circuit.
            bulk: false,
            bulk_bounds: (ordinal_set.min, ordinal_set.max),
        })
    }

    /// Confirms that one of a document's ordinals belongs to the set the terms
    /// enumeration covers.
    ///
    /// Equivalent to
    /// `DocValuesRangeIterator.forOrdinalSet(SortedSetDocValues, DocValuesSkipper, TermsEnum)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while enumerating the terms.
    pub fn for_sorted_set_ordinal_set(
        values: Box<dyn SortedSetDocValues>,
        skipper: Option<Box<dyn DocValuesSkipper>>,
        terms: &mut dyn TermsEnum,
    ) -> Result<Self> {
        let ord_count = values.get_value_count()?;
        let ordinal_set = build_ordinal_set(terms, ord_count)?;
        Ok(Self::for_sorted_set_ordinal_set_inner(
            values,
            skipper,
            ordinal_set,
        ))
    }

    /// Confirms that one of a document's ordinals is set in `ords` and lies
    /// within `[min_ord, max_ord]`.
    ///
    /// Equivalent to
    /// `DocValuesRangeIterator.forOrdinalSet(SortedSetDocValues, DocValuesSkipper, long, long, LongBitSet)`.
    /// Contiguity is not inferred here: callers of this form are not required
    /// to keep every set bit within `[min_ord, max_ord]`, so it cannot be
    /// derived from the cardinality alone.
    pub fn for_ordinal_set_bits(
        values: Box<dyn SortedSetDocValues>,
        skipper: Option<Box<dyn DocValuesSkipper>>,
        min_ord: i64,
        max_ord: i64,
        ords: Arc<LongBitSet>,
    ) -> Self {
        Self::for_sorted_set_ordinal_set_inner(
            values,
            skipper,
            Some(OrdinalSet {
                min: min_ord,
                max: max_ord,
                ords,
                contiguous: false,
            }),
        )
    }

    /// Equivalent to the private
    /// `forOrdinalSet(SortedSetDocValues, DocValuesSkipper, OrdinalSet)`.
    fn for_sorted_set_ordinal_set_inner(
        values: Box<dyn SortedSetDocValues>,
        skipper: Option<Box<dyn DocValuesSkipper>>,
        ordinal_set: Option<OrdinalSet>,
    ) -> Self {
        let Some(ordinal_set) = ordinal_set else {
            return Self::empty();
        };
        if ordinal_set.disjoint(skipper.as_deref()) {
            return Self::empty();
        }
        if ordinal_set.contiguous {
            return Self::for_sorted_set_ordinal_range(
                values,
                skipper,
                ordinal_set.min,
                ordinal_set.max,
            );
        }
        let predicate = RangePredicate::SortedSetOrdinalSet {
            min: ordinal_set.min,
            max: ordinal_set.max,
            ords: Arc::clone(&ordinal_set.ords),
        };
        Self {
            values: RangeValues::SortedSet(SortedSetDocValuesIterator::new(values)),
            block_iterator: skipper.map(|skipper| {
                SkipBlockRangeIterator::new(skipper, ordinal_set.min, ordinal_set.max)
            }),
            predicate,
            match_cost: 5.0,
            bulk: false,
            bulk_bounds: (ordinal_set.min, ordinal_set.max),
        }
    }

    /// An iterator that matches nothing.
    ///
    /// Equivalent to the private `DocValuesRangeIterator.EmptyRangeIterator`.
    pub fn empty() -> Self {
        Self {
            values: RangeValues::Empty(empty()),
            block_iterator: None,
            predicate: RangePredicate::Never,
            match_cost: 0.0,
            bulk: false,
            bulk_bounds: (0, 0),
        }
    }

    /// Equivalent to the `final DocValuesBlockRangeIterator.advanceDisi(int)`.
    fn advance_disi(&mut self, target: i32) -> Result<bool> {
        let disi = self.values.as_iterator();
        if disi.doc_id() >= target {
            return Ok(disi.doc_id() == target);
        }
        Ok(disi.advance(target)? == target)
    }

    /// Evaluates the confirmation the factory installed.
    ///
    /// Equivalent to calling the `IOBooleanSupplier check`.
    fn predicate(&mut self) -> Result<bool> {
        match &self.predicate {
            RangePredicate::Never => Ok(false),
            RangePredicate::NumericRange { min, max } => {
                let (min, max) = (*min, *max);
                let RangeValues::Numeric(values) = &mut self.values else {
                    return Ok(false);
                };
                let v = values.values().long_value()?;
                Ok(v >= min && v <= max)
            }
            RangePredicate::SortedNumericRange { min, max } => {
                let (min, max) = (*min, *max);
                let RangeValues::SortedNumeric(values) = &mut self.values else {
                    return Ok(false);
                };
                let values = values.values();
                let count = values.doc_value_count()?;
                for _ in 0..count {
                    let v = values.next_value()?;
                    if v >= min {
                        return Ok(v <= max);
                    }
                }
                Ok(false)
            }
            RangePredicate::OrdinalRange { min, max } => {
                let (min, max) = (*min, *max);
                let RangeValues::Sorted(values) = &mut self.values else {
                    return Ok(false);
                };
                let ord = i64::from(values.values().ord_value()?);
                Ok(ord >= min && ord <= max)
            }
            RangePredicate::SortedSetOrdinalRange { min, max } => {
                let (min, max) = (*min, *max);
                let RangeValues::SortedSet(values) = &mut self.values else {
                    return Ok(false);
                };
                let values = values.values();
                let count = values.doc_value_count()?;
                for _ in 0..count {
                    let v = values.next_ord()?;
                    if v >= min {
                        return Ok(v <= max);
                    }
                }
                Ok(false)
            }
            RangePredicate::SortedOrdinalSet { ords } => {
                let ords = Arc::clone(ords);
                let RangeValues::Sorted(values) = &mut self.values else {
                    return Ok(false);
                };
                let ord = i64::from(values.values().ord_value()?);
                Ok(ords.get(ord))
            }
            RangePredicate::SortedSetOrdinalSet { min, max, ords } => {
                let (min, max) = (*min, *max);
                let ords = Arc::clone(ords);
                let RangeValues::SortedSet(values) = &mut self.values else {
                    return Ok(false);
                };
                let values = values.values();
                let count = values.doc_value_count()?;
                for _ in 0..count {
                    let v = values.next_ord()?;
                    if v > max {
                        return Ok(false);
                    }
                    if v >= min && ords.get(v) {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
        }
    }

    /// Evaluates a partially-matching block in bulk.
    ///
    /// Equivalent to the three `BulkBlockRangeIterator.intoMaybeBlock`
    /// implementations, dispatched on the kind of doc values.
    fn into_maybe_block(
        &mut self,
        block_start: i32,
        block_end: i32,
        bit_set: &mut FixedBitSet,
        offset: i32,
    ) -> Result<()> {
        let (min_value, max_value) = self.bulk_bounds;
        match &mut self.values {
            RangeValues::Numeric(values) => {
                // The doc values are the same instance as the approximation, so
                // a preceding `matches()` call may have moved them beyond
                // `block_start`. The starting point is adjusted to keep
                // `range_into_bit_set`'s `advance_exact` calls forward-only.
                let from = block_start.max(values.doc_id());
                values
                    .values()
                    .range_into_bit_set(from, block_end, min_value, max_value, bit_set, offset)
            }
            RangeValues::SortedNumeric(values) => {
                let from = block_start.max(values.doc_id());
                values
                    .values()
                    .range_into_bit_set(from, block_end, min_value, max_value, bit_set, offset)
            }
            _ => {
                // Equivalent to `BulkOrdinalRangeIterator.intoMaybeBlock`.
                if self.values.as_iterator().doc_id() < block_start {
                    self.values.as_iterator().advance(block_start)?;
                }
                let mut doc = self.values.as_iterator().doc_id();
                while doc < block_end {
                    if self.predicate()? {
                        bit_set.set((doc - offset) as usize);
                    }
                    doc = self.values.as_iterator().next_doc()?;
                }
                Ok(())
            }
        }
    }
}

impl TwoPhaseIterator for DocValuesRangeIterator {
    fn approximation(&mut self) -> &mut dyn DocIdSetIterator {
        match self.block_iterator.as_mut() {
            Some(block_iterator) => block_iterator,
            None => self.values.as_iterator(),
        }
    }

    fn approximation_ref(&self) -> &dyn DocIdSetIterator {
        match self.block_iterator.as_ref() {
            Some(block_iterator) => block_iterator,
            None => self.values.as_iterator_ref(),
        }
    }

    fn matches(&mut self) -> Result<bool> {
        let Some(block_iterator) = self.block_iterator.as_ref() else {
            // The `DocValuesValueRangeIterator` and `EmptyRangeIterator`
            // shapes, which confirm with the predicate alone.
            return self.predicate();
        };
        let doc = block_iterator.doc_id();
        if !self.bulk {
            // Equivalent to `DocValuesBlockRangeIterator.matches()`.
            return Ok(self.advance_disi(doc)? && self.predicate()?);
        }
        // Equivalent to `BulkBlockRangeIterator.matches()`.
        match block_iterator.get_match() {
            Match::Yes => Ok(true),
            Match::YesIfPresent => self.advance_disi(doc),
            Match::Maybe => Ok(self.advance_disi(doc)? && self.predicate()?),
        }
    }

    fn match_cost(&self) -> f32 {
        self.match_cost
    }

    fn doc_id_run_end(&mut self) -> Result<i32> {
        match self.block_iterator.as_ref() {
            None => Ok(self.approximation_ref().doc_id()),
            Some(block_iterator) => {
                if self.bulk {
                    block_iterator.doc_id_run_end()
                } else {
                    Ok(block_iterator.doc_id() + 1)
                }
            }
        }
    }

    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        if !self.bulk || self.block_iterator.is_none() {
            // The default confirmation walk, which Java inherits from
            // `TwoPhaseIterator`.
            let mut doc = self.approximation_ref().doc_id();
            while doc < up_to {
                if self.matches()? {
                    bit_set.set((doc - offset) as usize);
                }
                doc = self.approximation().next_doc()?;
            }
            return Ok(());
        }

        // Equivalent to `BulkBlockRangeIterator.intoBitSet`.
        loop {
            let (block_start, r#match, block_end) = {
                let block_iterator = self
                    .block_iterator
                    .as_ref()
                    .expect("INVARIANT: the block iterator was just observed to be present");
                let block_start = block_iterator.doc_id();
                if block_start >= up_to {
                    break;
                }
                let r#match = block_iterator.get_match();
                // For MAYBE blocks `doc_id_run_end()` is conservative — one past
                // the current doc — so the full block boundary is used to
                // evaluate the whole block at once.
                let block_end = if r#match == Match::Maybe {
                    up_to.min(block_iterator.block_end())
                } else {
                    up_to.min(block_iterator.doc_id_run_end()?)
                };
                (block_start, r#match, block_end)
            };

            match r#match {
                Match::Yes => bit_set.set_range(
                    (block_start - offset) as usize,
                    (block_end - offset) as usize,
                ),
                Match::YesIfPresent => {
                    // Every present value is in range, so every doc that has a
                    // value is marked. Delegating to `into_bit_set` lets a dense
                    // codec bulk-set the run rather than probing one doc at a
                    // time. Only advance forward: a preceding YES block leaves
                    // the doc values behind `block_start`, while
                    // MAYBE/YES_IF_PRESENT blocks leave them at or past it.
                    if self.values.as_iterator().doc_id() < block_start {
                        self.values.as_iterator().advance(block_start)?;
                    }
                    self.values
                        .as_iterator()
                        .into_bit_set(block_end, bit_set, offset)?;
                }
                Match::Maybe => self.into_maybe_block(block_start, block_end, bit_set, offset)?,
            }

            self.block_iterator
                .as_mut()
                .expect("INVARIANT: the block iterator was just observed to be present")
                .advance(block_end)?;
        }
        Ok(())
    }
}
