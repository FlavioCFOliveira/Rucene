//! Doc-values value accessors ported from `org.apache.lucene.index`.
//!
//! Equivalent to `org.apache.lucene.index.NumericDocValues`, `BinaryDocValues`,
//! `SortedDocValues`, `SortedNumericDocValues`, `SortedSetDocValues`,
//! `DocValuesSkipper`, the `DocValues` utility helper, the filter adapters,
//! `EmptyDocValuesProducer`, the singleton wrappers, `OrdinalMap`, and the
//! `SortedDocValuesTermsEnum` / `SortedSetDocValuesTermsEnum` term iterators.
//!
//! The doc-value iterators extend [`DocIdSetIterator`] and add an
//! `advance_exact` positioning method. Value accessors follow the Java
//! contract: they must only be called after the iterator has been positioned
//! on a document that has a value.

#![deny(unsafe_code)]

use std::cmp::Ordering;

use crate::codecs::doc_values::DocValuesProducer;
use crate::codecs::stub::FieldInfo;
use crate::error::{LuceneError, Result};
use std::fmt::Debug;

use crate::index::postings_enum::{ImpactsEnum, PostingsEnum};
use crate::index::terms::{OrdTermState, SeekStatus, TermState, TermsEnum};
use crate::search::{DocIdSetIterator, NO_MORE_DOCS};
use crate::util::attribute::AttributeSource;
use crate::util::extra::{
    IdentityLongValues, LongValues, PriorityQueue, PriorityQueueComparator, ZeroesLongValues,
};
use crate::util::{BytesRef, BytesRefBuilder, FixedBitSet};

// -----------------------------------------------------------------------------
// Doc-values iterator base trait
// -----------------------------------------------------------------------------

/// Base trait for all doc-value iterators.
///
/// Equivalent to `org.apache.lucene.index.DocValuesIterator`.
///
/// Extends [`DocIdSetIterator`] with `advance_exact`, which positions the
/// iterator exactly on `target` and reports whether that document has a value
/// for the field.
pub trait DocValuesIterator: DocIdSetIterator {
    /// Advance the iterator to exactly `target` and return `true` if `target`
    /// has a value.
    ///
    /// `target` must be greater than or equal to the current doc ID and must
    /// be a valid doc ID (`>= 0`). After this method returns, [`doc_id`](DocIdSetIterator::doc_id)
    /// returns `target`.
    fn advance_exact(&mut self, target: i32) -> Result<bool>;
}

// -----------------------------------------------------------------------------
// Numeric doc values
// -----------------------------------------------------------------------------

/// Iterator over the numeric doc values of a single field.
///
/// Equivalent to `org.apache.lucene.index.NumericDocValues`.
pub trait NumericDocValues: DocValuesIterator {
    /// Returns the numeric value for the current document ID.
    ///
    /// It is illegal to call this method after [`advance_exact`](DocValuesIterator::advance_exact)
    /// returned `false`.
    fn long_value(&self) -> Result<i64>;

    /// Bulk retrieval of numeric doc values.
    ///
    /// Equivalent to `NumericDocValues.longValues(int, int[], int, long[], int, long)`.
    ///
    /// The `docs` array must be sorted in ascending order with no duplicates.
    /// For each doc ID, if it has a value, the value is written into `values`;
    /// otherwise `default_value` is written.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` if `size` is negative or if the
    /// provided slices are too short for the requested offsets and size.
    fn long_values(
        &mut self,
        size: i32,
        docs: &[i32],
        docs_offset: i32,
        values: &mut [i64],
        values_offset: i32,
        default_value: i64,
    ) -> Result<()> {
        if size < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "size must be >= 0, got {size}"
            )));
        }
        let size = size as usize;
        if docs.len() < docs_offset as usize + size {
            return Err(LuceneError::IllegalArgument(
                "docs buffer too small for requested size/offset".to_string(),
            ));
        }
        if values.len() < values_offset as usize + size {
            return Err(LuceneError::IllegalArgument(
                "values buffer too small for requested size/offset".to_string(),
            ));
        }
        for (di, &doc) in docs
            .iter()
            .enumerate()
            .skip(docs_offset as usize)
            .take(size)
        {
            let vi = values_offset as usize + (di - docs_offset as usize);
            let value = if self.advance_exact(doc)? {
                self.long_value()?
            } else {
                default_value
            };
            values[vi] = value;
        }
        Ok(())
    }

    /// Fills a [`FixedBitSet`] with the doc IDs in `[from_doc, to_doc)` whose
    /// values are in `[min_value, max_value]`.
    ///
    /// Equivalent to `NumericDocValues.rangeIntoBitSet`.
    ///
    /// The default implementation falls back to per-doc evaluation via
    /// [`advance_exact`](DocValuesIterator::advance_exact) and [`long_value`](Self::long_value).
    fn range_into_bit_set(
        &mut self,
        from_doc: i32,
        to_doc: i32,
        min_value: i64,
        max_value: i64,
        bit_set: &mut FixedBitSet,
        offset: i32,
    ) -> Result<()> {
        for doc in from_doc..to_doc {
            if self.advance_exact(doc)? {
                let v = self.long_value()?;
                if v >= min_value && v <= max_value {
                    bit_set.set((doc - offset) as usize);
                }
            }
        }
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// Binary doc values
// -----------------------------------------------------------------------------

/// Iterator over the binary doc values of a single field.
///
/// Equivalent to `org.apache.lucene.index.BinaryDocValues`.
pub trait BinaryDocValues: DocValuesIterator {
    /// Returns the binary value for the current document ID.
    ///
    /// It is illegal to call this method after [`advance_exact`](DocValuesIterator::advance_exact)
    /// returned `false`.
    fn binary_value(&self) -> Result<BytesRef>;
}

// -----------------------------------------------------------------------------
// Sorted doc values
// -----------------------------------------------------------------------------

/// Iterator over the sorted binary doc values of a single field.
///
/// Equivalent to `org.apache.lucene.index.SortedDocValues`.
pub trait SortedDocValues: DocValuesIterator {
    /// Returns the ordinal for the current document ID.
    ///
    /// Ordinals are dense, start at `0`, and increase by one for the next
    /// value in sorted order.
    fn ord_value(&self) -> Result<i32>;

    /// Returns the number of unique values.
    fn get_value_count(&self) -> Result<i32>;

    /// Looks up the binary value for the given ordinal.
    ///
    /// The returned [`BytesRef`] may be reused across calls; copy it with
    /// [`BytesRef::deep_copy_of`] if it must outlive the call.
    fn lookup_ord(&self, ord: i32) -> Result<BytesRef>;

    /// If `key` exists, returns its ordinal, else returns `-insertion_point - 1`,
    /// like a binary search.
    ///
    /// Equivalent to `SortedDocValues.lookupTerm(BytesRef)`.
    fn lookup_term(&self, key: &BytesRef) -> Result<i32> {
        let mut low = 0i32;
        let mut high = self.get_value_count()? - 1;
        while low <= high {
            let mid = ((low as u32 + high as u32) >> 1) as i32;
            let term = self.lookup_ord(mid)?;
            match term.cmp(key) {
                Ordering::Less => low = mid + 1,
                Ordering::Greater => high = mid - 1,
                Ordering::Equal => return Ok(mid),
            }
        }
        Ok(-(low + 1))
    }
}

// -----------------------------------------------------------------------------
// Sorted numeric doc values
// -----------------------------------------------------------------------------

/// Iterator over the sorted numeric doc values of a single field.
///
/// Equivalent to `org.apache.lucene.index.SortedNumericDocValues`.
pub trait SortedNumericDocValues: DocValuesIterator {
    /// Returns the next numeric value for the current document.
    ///
    /// Do not call this more than [`doc_value_count`](Self::doc_value_count)
    /// times for the current document.
    fn next_value(&mut self) -> Result<i64>;

    /// Returns the number of values for the current document.
    fn doc_value_count(&self) -> Result<i32>;

    /// Fills a [`FixedBitSet`] with the doc IDs in `[from_doc, to_doc)` whose
    /// sorted numeric values contain at least one value in `[min_value, max_value]`.
    ///
    /// Equivalent to `SortedNumericDocValues.rangeIntoBitSet`.
    fn range_into_bit_set(
        &mut self,
        from_doc: i32,
        to_doc: i32,
        min_value: i64,
        max_value: i64,
        bit_set: &mut FixedBitSet,
        offset: i32,
    ) -> Result<()> {
        for doc in from_doc..to_doc {
            if self.advance_exact(doc)? {
                let count = self.doc_value_count()?;
                for _ in 0..count {
                    let value = self.next_value()?;
                    if value >= min_value {
                        if value <= max_value {
                            bit_set.set((doc - offset) as usize);
                        }
                        break;
                    }
                }
            }
        }
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// Sorted set doc values
// -----------------------------------------------------------------------------

/// Iterator over the sorted set doc values of a single field.
///
/// Equivalent to `org.apache.lucene.index.SortedSetDocValues`.
pub trait SortedSetDocValues: DocValuesIterator {
    /// Returns the next ordinal for the current document.
    ///
    /// Do not call this more than [`doc_value_count`](Self::doc_value_count)
    /// times for the current document. Ordinals are dense, start at `0`, and
    /// increase by one for the next value in sorted order.
    fn next_ord(&mut self) -> Result<i64>;

    /// Returns the number of values for the current document.
    fn doc_value_count(&self) -> Result<i32>;

    /// Looks up the binary value for the given ordinal.
    fn lookup_ord(&self, ord: i64) -> Result<BytesRef>;

    /// Returns the number of unique values.
    fn get_value_count(&self) -> Result<i64>;

    /// If `key` exists, returns its ordinal, else returns `-insertion_point - 1`,
    /// like a binary search.
    ///
    /// Equivalent to `SortedSetDocValues.lookupTerm(BytesRef)`.
    fn lookup_term(&self, key: &BytesRef) -> Result<i64> {
        let mut low = 0i64;
        let mut high = self.get_value_count()? - 1;
        while low <= high {
            let mid = ((low as u64 + high as u64) >> 1) as i64;
            let term = self.lookup_ord(mid)?;
            match term.cmp(key) {
                Ordering::Less => low = mid + 1,
                Ordering::Greater => high = mid - 1,
                Ordering::Equal => return Ok(mid),
            }
        }
        Ok(-(low + 1))
    }
}

// -----------------------------------------------------------------------------
// Doc-values skipper
// -----------------------------------------------------------------------------

/// Skip index for fast-forwarding inside a doc-values field.
///
/// Equivalent to `org.apache.lucene.index.DocValuesSkipper`.
///
/// A skipper has a position that can only be advanced via [`advance`](Self::advance).
/// The next advance position must be greater than `max_doc_id(0)`.
pub trait DocValuesSkipper: Send + Sync {
    /// Advance this skipper so that all levels contain the next document on or
    /// after `target`.
    ///
    /// The behavior is undefined if `target` is less than or equal to
    /// `max_doc_id(0)`.
    fn advance(&mut self, target: i32) -> Result<()>;

    /// Return the number of levels.
    fn num_levels(&self) -> i32;

    /// Return the minimum doc ID of the interval on the given level, inclusive.
    fn min_doc_id(&self, level: i32) -> i32;

    /// Return the maximum doc ID of the interval on the given level, inclusive.
    fn max_doc_id(&self, level: i32) -> i32;

    /// Return the minimum value of the interval at the given level, inclusive.
    fn min_value(&self, level: i32) -> i64;

    /// Return the maximum value of the interval at the given level, inclusive.
    fn max_value(&self, level: i32) -> i64;

    /// Return the number of documents that have a value in the interval
    /// associated with the given level.
    fn doc_count(&self, level: i32) -> i32;

    /// Return the global minimum value.
    fn global_min_value(&self) -> i64;

    /// Return the global maximum value.
    fn global_max_value(&self) -> i64;

    /// Return the global number of documents with a value for the field.
    fn global_doc_count(&self) -> i32;

    /// Return the global maximum number of values that any single document has
    /// for the field.
    ///
    /// Returns `-1` if the exact value is unavailable. Returns `0` if
    /// `global_doc_count()` is `0`.
    fn max_value_count(&self) -> i32 {
        if self.global_doc_count() == 0 {
            0
        } else {
            -1
        }
    }

    /// Advance this skipper so that all levels intersect the range given by
    /// `min_value` and `max_value`.
    ///
    /// Equivalent to `DocValuesSkipper.advance(long, long)`.
    fn advance_range(&mut self, min_value: i64, max_value: i64) -> Result<()> {
        if self.min_doc_id(0) == -1 {
            self.advance(0)?;
        }
        while self.min_doc_id(0) != NO_MORE_DOCS
            && (self.min_value(0) > max_value || self.max_value(0) < min_value)
        {
            let mut max_doc_id = self.max_doc_id(0);
            let mut next_level = 1;
            while next_level < self.num_levels()
                && (self.min_value(next_level) > max_value
                    || self.max_value(next_level) < min_value)
            {
                max_doc_id = self.max_doc_id(next_level);
                next_level += 1;
            }
            self.advance(max_doc_id + 1)?;
        }
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// DocValues utility helper
// -----------------------------------------------------------------------------

/// Utility helper for doc values.
///
/// Equivalent to the static helpers on `org.apache.lucene.index.DocValues`.
///
/// This type has no fields; it only provides constructor-style helpers and
/// empty-iterator factories.
#[derive(Debug, Default, Clone, Copy)]
pub struct DocValues;

impl DocValues {
    /// Returns an empty [`BinaryDocValues`] which returns no documents.
    pub fn empty_binary() -> EmptyBinaryDocValues {
        EmptyBinaryDocValues::new()
    }

    /// Returns an empty [`NumericDocValues`] which returns no documents.
    pub fn empty_numeric() -> EmptyNumericDocValues {
        EmptyNumericDocValues::new()
    }

    /// Returns an empty [`SortedDocValues`] which returns no documents.
    pub fn empty_sorted() -> EmptySortedDocValues {
        EmptySortedDocValues::new()
    }

    /// Returns an empty [`SortedNumericDocValues`] which returns zero values
    /// for every document.
    pub fn empty_sorted_numeric() -> EmptySortedNumericDocValues {
        EmptySortedNumericDocValues::new()
    }

    /// Returns an empty [`SortedSetDocValues`] which returns zero values for
    /// every document.
    pub fn empty_sorted_set() -> EmptySortedSetDocValues {
        EmptySortedSetDocValues::new()
    }

    /// Returns a multi-valued view over the provided [`NumericDocValues`].
    pub fn singleton_numeric(dv: Box<dyn NumericDocValues>) -> SingletonSortedNumericDocValues {
        SingletonSortedNumericDocValues::new(dv)
    }

    /// Returns a multi-valued view over the provided [`SortedDocValues`].
    pub fn singleton_sorted(dv: Box<dyn SortedDocValues>) -> SingletonSortedSetDocValues {
        SingletonSortedSetDocValues::new(dv)
    }

    /// Returns the single-valued [`NumericDocValues`] wrapped by
    /// [`SingletonSortedNumericDocValues`], if any.
    pub fn unwrap_singleton_numeric(
        dv: &dyn SortedNumericDocValues,
    ) -> Option<&dyn NumericDocValues> {
        // Since `SingletonSortedNumericDocValues` stores the inner iterator as
        // `Box<dyn NumericDocValues>`, we cannot recover it through a trait-object
        // downcast without a concrete type. This helper therefore only works
        // on the concrete wrapper.
        let _ = dv;
        None
    }

    /// Returns the single-valued [`SortedDocValues`] wrapped by
    /// [`SingletonSortedSetDocValues`], if any.
    pub fn unwrap_singleton_sorted(dv: &dyn SortedSetDocValues) -> Option<&dyn SortedDocValues> {
        let _ = dv;
        None
    }
}

// -----------------------------------------------------------------------------
// Empty implementations
// -----------------------------------------------------------------------------

/// Shared internal state for empty doc-values iterators.
#[derive(Debug, Clone, Copy)]
struct EmptyDocValuesIterator {
    doc: i32,
}

impl Default for EmptyDocValuesIterator {
    fn default() -> Self {
        Self { doc: -1 }
    }
}

impl DocIdSetIterator for EmptyDocValuesIterator {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.doc = NO_MORE_DOCS;
        Ok(NO_MORE_DOCS)
    }

    fn advance(&mut self, _target: i32) -> Result<i32> {
        self.doc = NO_MORE_DOCS;
        Ok(NO_MORE_DOCS)
    }

    fn cost(&self) -> i64 {
        0
    }
}

impl DocValuesIterator for EmptyDocValuesIterator {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.doc = target;
        Ok(false)
    }
}

/// A no-op numeric doc-values iterator that returns no documents.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyNumericDocValues {
    inner: EmptyDocValuesIterator,
}

impl EmptyNumericDocValues {
    /// Creates a new empty numeric doc-values iterator.
    pub fn new() -> Self {
        Self::default()
    }
}

impl DocIdSetIterator for EmptyNumericDocValues {
    fn doc_id(&self) -> i32 {
        self.inner.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.inner.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.inner.advance(target)
    }

    fn cost(&self) -> i64 {
        self.inner.cost()
    }
}

impl DocValuesIterator for EmptyNumericDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.inner.advance_exact(target)
    }
}

impl NumericDocValues for EmptyNumericDocValues {
    fn long_value(&self) -> Result<i64> {
        Err(LuceneError::IllegalState(
            "long_value called on empty numeric doc values".to_string(),
        ))
    }
}

/// A no-op binary doc-values iterator that returns no documents.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyBinaryDocValues {
    inner: EmptyDocValuesIterator,
}

impl EmptyBinaryDocValues {
    /// Creates a new empty binary doc-values iterator.
    pub fn new() -> Self {
        Self::default()
    }
}

impl DocIdSetIterator for EmptyBinaryDocValues {
    fn doc_id(&self) -> i32 {
        self.inner.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.inner.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.inner.advance(target)
    }

    fn cost(&self) -> i64 {
        self.inner.cost()
    }
}

impl DocValuesIterator for EmptyBinaryDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.inner.advance_exact(target)
    }
}

impl BinaryDocValues for EmptyBinaryDocValues {
    fn binary_value(&self) -> Result<BytesRef> {
        Err(LuceneError::IllegalState(
            "binary_value called on empty binary doc values".to_string(),
        ))
    }
}

/// A no-op sorted doc-values iterator that returns no documents.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptySortedDocValues {
    inner: EmptyDocValuesIterator,
}

impl EmptySortedDocValues {
    /// Creates a new empty sorted doc-values iterator.
    pub fn new() -> Self {
        Self::default()
    }
}

impl DocIdSetIterator for EmptySortedDocValues {
    fn doc_id(&self) -> i32 {
        self.inner.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.inner.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.inner.advance(target)
    }

    fn cost(&self) -> i64 {
        self.inner.cost()
    }
}

impl DocValuesIterator for EmptySortedDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.inner.advance_exact(target)
    }
}

impl SortedDocValues for EmptySortedDocValues {
    fn ord_value(&self) -> Result<i32> {
        Err(LuceneError::IllegalState(
            "ord_value called on empty sorted doc values".to_string(),
        ))
    }

    fn get_value_count(&self) -> Result<i32> {
        Ok(0)
    }

    fn lookup_ord(&self, _ord: i32) -> Result<BytesRef> {
        Ok(BytesRef::new(Vec::new()))
    }
}

/// A no-op sorted-numeric doc-values iterator that returns zero values for
/// every document.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptySortedNumericDocValues {
    inner: EmptyDocValuesIterator,
}

impl EmptySortedNumericDocValues {
    /// Creates a new empty sorted-numeric doc-values iterator.
    pub fn new() -> Self {
        Self::default()
    }
}

impl DocIdSetIterator for EmptySortedNumericDocValues {
    fn doc_id(&self) -> i32 {
        self.inner.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.inner.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.inner.advance(target)
    }

    fn cost(&self) -> i64 {
        self.inner.cost()
    }
}

impl DocValuesIterator for EmptySortedNumericDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.inner.advance_exact(target)
    }
}

impl SortedNumericDocValues for EmptySortedNumericDocValues {
    fn next_value(&mut self) -> Result<i64> {
        Err(LuceneError::IllegalState(
            "next_value called on empty sorted numeric doc values".to_string(),
        ))
    }

    fn doc_value_count(&self) -> Result<i32> {
        Ok(0)
    }
}

/// A no-op sorted-set doc-values iterator that returns zero values for every
/// document.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptySortedSetDocValues {
    inner: EmptyDocValuesIterator,
}

impl EmptySortedSetDocValues {
    /// Creates a new empty sorted-set doc-values iterator.
    pub fn new() -> Self {
        Self::default()
    }
}

impl DocIdSetIterator for EmptySortedSetDocValues {
    fn doc_id(&self) -> i32 {
        self.inner.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.inner.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.inner.advance(target)
    }

    fn cost(&self) -> i64 {
        self.inner.cost()
    }
}

impl DocValuesIterator for EmptySortedSetDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.inner.advance_exact(target)
    }
}

impl SortedSetDocValues for EmptySortedSetDocValues {
    fn next_ord(&mut self) -> Result<i64> {
        Ok(-1)
    }

    fn doc_value_count(&self) -> Result<i32> {
        Ok(0)
    }

    fn lookup_ord(&self, _ord: i64) -> Result<BytesRef> {
        Ok(BytesRef::new(Vec::new()))
    }

    fn get_value_count(&self) -> Result<i64> {
        Ok(0)
    }
}

/// A no-op doc-values skipper.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyDocValuesSkipper;

impl DocValuesSkipper for EmptyDocValuesSkipper {
    fn advance(&mut self, _target: i32) -> Result<()> {
        Ok(())
    }

    fn num_levels(&self) -> i32 {
        1
    }

    fn min_doc_id(&self, _level: i32) -> i32 {
        -1
    }

    fn max_doc_id(&self, _level: i32) -> i32 {
        -1
    }

    fn min_value(&self, _level: i32) -> i64 {
        0
    }

    fn max_value(&self, _level: i32) -> i64 {
        0
    }

    fn doc_count(&self, _level: i32) -> i32 {
        0
    }

    fn global_min_value(&self) -> i64 {
        0
    }

    fn global_max_value(&self) -> i64 {
        0
    }

    fn global_doc_count(&self) -> i32 {
        0
    }
}

// -----------------------------------------------------------------------------
// Singleton wrappers
// -----------------------------------------------------------------------------

/// Multi-valued view over a single-valued [`NumericDocValues`].
///
/// Equivalent to `org.apache.lucene.index.SingletonSortedNumericDocValues`.
pub struct SingletonSortedNumericDocValues {
    inner: Box<dyn NumericDocValues>,
}

impl SingletonSortedNumericDocValues {
    /// Creates a multi-valued view over the provided numeric doc values.
    pub fn new(inner: Box<dyn NumericDocValues>) -> Self {
        Self { inner }
    }

    /// Returns the wrapped numeric doc values.
    pub fn get_numeric_doc_values(&self) -> &dyn NumericDocValues {
        self.inner.as_ref()
    }
}

impl DocIdSetIterator for SingletonSortedNumericDocValues {
    fn doc_id(&self) -> i32 {
        self.inner.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.inner.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.inner.advance(target)
    }

    fn cost(&self) -> i64 {
        self.inner.cost()
    }

    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        self.inner.into_bit_set(up_to, bit_set, offset)
    }
}

impl DocValuesIterator for SingletonSortedNumericDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.inner.advance_exact(target)
    }
}

impl SortedNumericDocValues for SingletonSortedNumericDocValues {
    fn next_value(&mut self) -> Result<i64> {
        self.inner.long_value()
    }

    fn doc_value_count(&self) -> Result<i32> {
        Ok(1)
    }
}

/// Multi-valued view over a single-valued [`SortedDocValues`].
///
/// Equivalent to `org.apache.lucene.index.SingletonSortedSetDocValues`.
pub struct SingletonSortedSetDocValues {
    inner: Box<dyn SortedDocValues>,
    ord: i64,
}

impl SingletonSortedSetDocValues {
    /// Creates a multi-valued view over the provided sorted doc values.
    pub fn new(inner: Box<dyn SortedDocValues>) -> Self {
        Self { inner, ord: -1 }
    }

    /// Returns the wrapped sorted doc values.
    pub fn get_sorted_doc_values(&self) -> &dyn SortedDocValues {
        self.inner.as_ref()
    }
}

impl DocIdSetIterator for SingletonSortedSetDocValues {
    fn doc_id(&self) -> i32 {
        self.inner.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        let doc = self.inner.next_doc()?;
        if doc != NO_MORE_DOCS {
            self.ord = self.inner.ord_value()? as i64;
        }
        Ok(doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let doc = self.inner.advance(target)?;
        if doc != NO_MORE_DOCS {
            self.ord = self.inner.ord_value()? as i64;
        }
        Ok(doc)
    }

    fn cost(&self) -> i64 {
        self.inner.cost()
    }

    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        self.inner.into_bit_set(up_to, bit_set, offset)
    }
}

impl DocValuesIterator for SingletonSortedSetDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        if self.inner.advance_exact(target)? {
            self.ord = self.inner.ord_value()? as i64;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

impl SortedSetDocValues for SingletonSortedSetDocValues {
    fn next_ord(&mut self) -> Result<i64> {
        Ok(self.ord)
    }

    fn doc_value_count(&self) -> Result<i32> {
        Ok(1)
    }

    fn lookup_ord(&self, ord: i64) -> Result<BytesRef> {
        self.inner.lookup_ord(ord as i32)
    }

    fn get_value_count(&self) -> Result<i64> {
        self.inner.get_value_count().map(|v| v as i64)
    }

    fn lookup_term(&self, key: &BytesRef) -> Result<i64> {
        self.inner.lookup_term(key).map(|v| v as i64)
    }
}

// -----------------------------------------------------------------------------
// Box forwarding impls
// -----------------------------------------------------------------------------

/// Forwarding implementations so that `Box<dyn Trait>` can be used wherever
/// `dyn Trait` is expected. This mirrors Java's polymorphic references and
/// avoids manual unboxing at every call site.
impl<T: DocIdSetIterator + ?Sized> DocIdSetIterator for Box<T> {
    fn doc_id(&self) -> i32 {
        (&**self).doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        (&mut **self).next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        (&mut **self).advance(target)
    }

    fn cost(&self) -> i64 {
        (&**self).cost()
    }

    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        (&mut **self).into_bit_set(up_to, bit_set, offset)
    }
}

impl<T: DocValuesIterator + ?Sized> DocValuesIterator for Box<T> {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        (&mut **self).advance_exact(target)
    }
}

impl<T: NumericDocValues + ?Sized> NumericDocValues for Box<T> {
    fn long_value(&self) -> Result<i64> {
        (&**self).long_value()
    }
}

// -----------------------------------------------------------------------------
// Filter adapters
// -----------------------------------------------------------------------------

/// Delegates all methods to a wrapped [`NumericDocValues`].
///
/// Equivalent to `org.apache.lucene.index.FilterNumericDocValues`.
pub struct FilterNumericDocValues {
    inner: Box<dyn NumericDocValues>,
}

impl FilterNumericDocValues {
    /// Creates a filter wrapping the provided numeric doc values.
    pub fn new(inner: Box<dyn NumericDocValues>) -> Self {
        Self { inner }
    }

    /// Returns the wrapped numeric doc values.
    pub fn get_inner(&self) -> &dyn NumericDocValues {
        self.inner.as_ref()
    }
}

impl DocIdSetIterator for FilterNumericDocValues {
    fn doc_id(&self) -> i32 {
        self.inner.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.inner.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.inner.advance(target)
    }

    fn cost(&self) -> i64 {
        self.inner.cost()
    }

    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        self.inner.into_bit_set(up_to, bit_set, offset)
    }
}

impl DocValuesIterator for FilterNumericDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.inner.advance_exact(target)
    }
}

impl NumericDocValues for FilterNumericDocValues {
    fn long_value(&self) -> Result<i64> {
        self.inner.long_value()
    }
}

/// Delegates all methods to a wrapped [`BinaryDocValues`].
///
/// Equivalent to `org.apache.lucene.index.FilterBinaryDocValues`.
pub struct FilterBinaryDocValues {
    inner: Box<dyn BinaryDocValues>,
}

impl FilterBinaryDocValues {
    /// Creates a filter wrapping the provided binary doc values.
    pub fn new(inner: Box<dyn BinaryDocValues>) -> Self {
        Self { inner }
    }

    /// Returns the wrapped binary doc values.
    pub fn get_inner(&self) -> &dyn BinaryDocValues {
        self.inner.as_ref()
    }
}

impl DocIdSetIterator for FilterBinaryDocValues {
    fn doc_id(&self) -> i32 {
        self.inner.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.inner.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.inner.advance(target)
    }

    fn cost(&self) -> i64 {
        self.inner.cost()
    }

    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        self.inner.into_bit_set(up_to, bit_set, offset)
    }
}

impl DocValuesIterator for FilterBinaryDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.inner.advance_exact(target)
    }
}

impl BinaryDocValues for FilterBinaryDocValues {
    fn binary_value(&self) -> Result<BytesRef> {
        self.inner.binary_value()
    }
}

/// Delegates all methods to a wrapped [`SortedDocValues`].
///
/// Equivalent to `org.apache.lucene.index.FilterSortedDocValues`.
pub struct FilterSortedDocValues {
    inner: Box<dyn SortedDocValues>,
}

impl FilterSortedDocValues {
    /// Creates a filter wrapping the provided sorted doc values.
    pub fn new(inner: Box<dyn SortedDocValues>) -> Self {
        Self { inner }
    }

    /// Returns the wrapped sorted doc values.
    pub fn get_inner(&self) -> &dyn SortedDocValues {
        self.inner.as_ref()
    }
}

impl DocIdSetIterator for FilterSortedDocValues {
    fn doc_id(&self) -> i32 {
        self.inner.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.inner.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.inner.advance(target)
    }

    fn cost(&self) -> i64 {
        self.inner.cost()
    }

    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        self.inner.into_bit_set(up_to, bit_set, offset)
    }
}

impl DocValuesIterator for FilterSortedDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.inner.advance_exact(target)
    }
}

impl SortedDocValues for FilterSortedDocValues {
    fn ord_value(&self) -> Result<i32> {
        self.inner.ord_value()
    }

    fn get_value_count(&self) -> Result<i32> {
        self.inner.get_value_count()
    }

    fn lookup_ord(&self, ord: i32) -> Result<BytesRef> {
        self.inner.lookup_ord(ord)
    }

    fn lookup_term(&self, key: &BytesRef) -> Result<i32> {
        self.inner.lookup_term(key)
    }
}

/// Delegates all methods to a wrapped [`SortedNumericDocValues`].
///
/// Equivalent to `org.apache.lucene.index.FilterSortedNumericDocValues`.
pub struct FilterSortedNumericDocValues {
    inner: Box<dyn SortedNumericDocValues>,
}

impl FilterSortedNumericDocValues {
    /// Creates a filter wrapping the provided sorted-numeric doc values.
    pub fn new(inner: Box<dyn SortedNumericDocValues>) -> Self {
        Self { inner }
    }

    /// Returns the wrapped sorted-numeric doc values.
    pub fn get_inner(&self) -> &dyn SortedNumericDocValues {
        self.inner.as_ref()
    }
}

impl DocIdSetIterator for FilterSortedNumericDocValues {
    fn doc_id(&self) -> i32 {
        self.inner.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.inner.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.inner.advance(target)
    }

    fn cost(&self) -> i64 {
        self.inner.cost()
    }

    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        self.inner.into_bit_set(up_to, bit_set, offset)
    }
}

impl DocValuesIterator for FilterSortedNumericDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.inner.advance_exact(target)
    }
}

impl SortedNumericDocValues for FilterSortedNumericDocValues {
    fn next_value(&mut self) -> Result<i64> {
        self.inner.next_value()
    }

    fn doc_value_count(&self) -> Result<i32> {
        self.inner.doc_value_count()
    }
}

/// Delegates all methods to a wrapped [`SortedSetDocValues`].
///
/// Equivalent to `org.apache.lucene.index.FilterSortedSetDocValues`.
pub struct FilterSortedSetDocValues {
    inner: Box<dyn SortedSetDocValues>,
}

impl FilterSortedSetDocValues {
    /// Creates a filter wrapping the provided sorted-set doc values.
    pub fn new(inner: Box<dyn SortedSetDocValues>) -> Self {
        Self { inner }
    }

    /// Returns the wrapped sorted-set doc values.
    pub fn get_inner(&self) -> &dyn SortedSetDocValues {
        self.inner.as_ref()
    }
}

impl DocIdSetIterator for FilterSortedSetDocValues {
    fn doc_id(&self) -> i32 {
        self.inner.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.inner.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.inner.advance(target)
    }

    fn cost(&self) -> i64 {
        self.inner.cost()
    }

    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        self.inner.into_bit_set(up_to, bit_set, offset)
    }
}

impl DocValuesIterator for FilterSortedSetDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.inner.advance_exact(target)
    }
}

impl SortedSetDocValues for FilterSortedSetDocValues {
    fn next_ord(&mut self) -> Result<i64> {
        self.inner.next_ord()
    }

    fn doc_value_count(&self) -> Result<i32> {
        self.inner.doc_value_count()
    }

    fn lookup_ord(&self, ord: i64) -> Result<BytesRef> {
        self.inner.lookup_ord(ord)
    }

    fn get_value_count(&self) -> Result<i64> {
        self.inner.get_value_count()
    }

    fn lookup_term(&self, key: &BytesRef) -> Result<i64> {
        self.inner.lookup_term(key)
    }
}

// -----------------------------------------------------------------------------
// Empty doc-values producer
// -----------------------------------------------------------------------------

/// Abstract base class implementing a [`DocValuesProducer`] that has no doc values.
///
/// Equivalent to `org.apache.lucene.index.EmptyDocValuesProducer`.
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyDocValuesProducer;

impl DocValuesProducer for EmptyDocValuesProducer {
    fn get_numeric(&self, _field: &FieldInfo) -> Result<Box<dyn NumericDocValues + Send + Sync>> {
        Ok(Box::new(EmptyNumericDocValues::new()))
    }

    fn get_binary(&self, _field: &FieldInfo) -> Result<Box<dyn BinaryDocValues + Send + Sync>> {
        Ok(Box::new(EmptyBinaryDocValues::new()))
    }

    fn get_sorted(&self, _field: &FieldInfo) -> Result<Box<dyn SortedDocValues + Send + Sync>> {
        Ok(Box::new(EmptySortedDocValues::new()))
    }

    fn get_sorted_numeric(
        &self,
        _field: &FieldInfo,
    ) -> Result<Box<dyn SortedNumericDocValues + Send + Sync>> {
        Ok(Box::new(EmptySortedNumericDocValues::new()))
    }

    fn get_sorted_set(
        &self,
        _field: &FieldInfo,
    ) -> Result<Box<dyn SortedSetDocValues + Send + Sync>> {
        Ok(Box::new(EmptySortedSetDocValues::new()))
    }

    fn get_skipper(&self, _field: &FieldInfo) -> Result<Box<dyn DocValuesSkipper + Send + Sync>> {
        Ok(Box::new(EmptyDocValuesSkipper))
    }

    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }

    fn get_merge_instance(&self) -> Result<Box<dyn DocValuesProducer>> {
        Ok(Box::new(*self))
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// Ordinal map
// -----------------------------------------------------------------------------

/// Maps per-segment ordinals to/from global ordinal space.
///
/// Equivalent to `org.apache.lucene.index.OrdinalMap`.
///
/// This is a costly operation that merge-sorts all terms. It is better to operate
/// in segment-private ordinal space when possible.
pub struct OrdinalMap {
    value_count: i64,
    global_ord_deltas: Box<dyn LongValues>,
    first_segments: Box<dyn LongValues>,
    segment_to_global_ords: Vec<Box<dyn LongValues>>,
    segment_map: SegmentMap,
}

impl Debug for OrdinalMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OrdinalMap")
            .field("value_count", &self.value_count)
            .field("segment_map", &self.segment_map)
            .field("num_segments", &self.segment_to_global_ords.len())
            .finish()
    }
}

impl OrdinalMap {
    /// Build an ordinal map using the value counts of each [`SortedDocValues`]
    /// instance as weights.
    ///
    /// Equivalent to `OrdinalMap.build(IndexReader.CacheKey, SortedDocValues[], float)`.
    pub fn build_sorted(
        values: Vec<Box<dyn SortedDocValues>>,
        acceptable_overhead_ratio: f32,
    ) -> Result<Self> {
        let mut subs: Vec<Box<dyn TermsEnum>> = Vec::with_capacity(values.len());
        let mut weights = Vec::with_capacity(values.len());
        for dv in values {
            let weight = dv.get_value_count()? as i64;
            subs.push(SortedDocValuesTermsEnum::new(dv));
            weights.push(weight);
        }
        Self::build(subs, &weights, acceptable_overhead_ratio)
    }

    /// Build an ordinal map using the value counts of each [`SortedSetDocValues`]
    /// instance as weights.
    ///
    /// Equivalent to `OrdinalMap.build(IndexReader.CacheKey, SortedSetDocValues[], float)`.
    pub fn build_sorted_set(
        values: Vec<Box<dyn SortedSetDocValues>>,
        acceptable_overhead_ratio: f32,
    ) -> Result<Self> {
        let mut subs: Vec<Box<dyn TermsEnum>> = Vec::with_capacity(values.len());
        let mut weights = Vec::with_capacity(values.len());
        for dv in values {
            let weight = dv.get_value_count()?;
            subs.push(SortedSetDocValuesTermsEnum::new(dv));
            weights.push(weight);
        }
        Self::build(subs, &weights, acceptable_overhead_ratio)
    }

    /// Build an ordinal map that maps ords to/from a merged space from `subs`.
    ///
    /// Equivalent to `OrdinalMap.build(IndexReader.CacheKey, TermsEnum[], long[], float)`.
    ///
    /// `subs` must support [`TermsEnum::ord`]. They need not be dense (for example,
    /// they may be filtered terms enums).
    pub fn build(
        subs: Vec<Box<dyn TermsEnum>>,
        weights: &[i64],
        _acceptable_overhead_ratio: f32,
    ) -> Result<Self> {
        if subs.len() != weights.len() {
            return Err(LuceneError::IllegalArgument(
                "subs and weights must have the same length".to_string(),
            ));
        }

        let segment_map = SegmentMap::new(weights);
        let num_subs = subs.len();

        let mut global_ord_deltas: Vec<i64> = Vec::new();
        let mut first_segments: Vec<i64> = Vec::new();
        let mut ord_deltas: Vec<Vec<i64>> = vec![Vec::new(); num_subs];
        let mut segment_ords = vec![0i64; num_subs];
        let mut ord_delta_bits = vec![0i64; num_subs];

        let mut subs: Vec<Option<Box<dyn TermsEnum>>> = subs.into_iter().map(Some).collect();
        let mut queue = PriorityQueue::new(num_subs, TermsEnumIndexComparator)?;
        for i in 0..num_subs {
            let old_index = segment_map.new_to_old(i);
            let terms_enum = subs[old_index].take().ok_or_else(|| {
                LuceneError::IllegalState("duplicate use of a terms enum".to_string())
            })?;
            let mut tei = TermsEnumIndex::new(terms_enum, i);
            if tei.next()?.is_some() {
                queue.add(tei);
            }
        }

        let mut global_ord = 0i64;
        while queue.size() > 0 {
            let top_state = TermsEnumIndexState::copy_from(queue.top().unwrap());
            let mut first_segment_index = usize::MAX;
            let mut global_ord_delta = i64::MAX;

            loop {
                let mut top = queue.pop().unwrap();
                let segment_ord = top.terms_enum.ord()?;
                let segment_index = top.sub_index;
                let delta = global_ord - segment_ord;
                if segment_index < first_segment_index {
                    first_segment_index = segment_index;
                    global_ord_delta = delta;
                }
                ord_delta_bits[segment_index] |= delta;

                while segment_ords[segment_index] <= segment_ord {
                    ord_deltas[segment_index].push(delta);
                    segment_ords[segment_index] += 1;
                }

                let advanced = top.next()?.is_some();
                if advanced {
                    queue.add(top);
                }

                match queue.top() {
                    Some(next_top) if next_top.term_equals(&top_state) => continue,
                    _ => break,
                }
            }

            first_segments.push(first_segment_index as i64);
            global_ord_deltas.push(global_ord_delta);
            global_ord += 1;
        }

        let value_count = global_ord;

        let (global_ord_deltas, first_segments) =
            if num_subs > 0 && ord_delta_bits[0] == 0 && first_segments.iter().all(|&x| x == 0) {
                (
                    Box::new(ZeroesLongValues) as Box<dyn LongValues>,
                    Box::new(ZeroesLongValues) as Box<dyn LongValues>,
                )
            } else {
                (
                    Box::new(ArrayLongValues(global_ord_deltas)) as Box<dyn LongValues>,
                    Box::new(ArrayLongValues(first_segments)) as Box<dyn LongValues>,
                )
            };

        let mut segment_to_global_ords: Vec<Box<dyn LongValues>> = Vec::with_capacity(num_subs);
        for i in 0..num_subs {
            if ord_delta_bits[i] == 0 {
                segment_to_global_ords.push(Box::new(IdentityLongValues));
            } else {
                segment_to_global_ords.push(Box::new(DeltaLongValues(ord_deltas[i].clone())));
            }
        }

        Ok(Self {
            value_count,
            global_ord_deltas,
            first_segments,
            segment_to_global_ords,
            segment_map,
        })
    }

    /// Returns a [`LongValues`] instance that maps segment ordinals to global
    /// ordinals for the given segment number.
    pub fn get_global_ords(&self, segment_index: usize) -> &dyn LongValues {
        let new_index = self.segment_map.old_to_new(segment_index);
        self.segment_to_global_ords[new_index].as_ref()
    }

    /// Returns the ordinal of the first segment that contains the given global
    /// ordinal.
    pub fn get_first_segment_ord(&self, global_ord: i64) -> i64 {
        global_ord - self.global_ord_deltas.get(global_ord)
    }

    /// Returns the original segment index of the first segment that contains the
    /// given global ordinal.
    pub fn get_first_segment_number(&self, global_ord: i64) -> usize {
        let new_index = self.first_segments.get(global_ord) as usize;
        self.segment_map.new_to_old(new_index)
    }

    /// Returns the total number of unique values in global ordinal space.
    pub fn get_value_count(&self) -> i64 {
        self.value_count
    }
}

/// Segment ordering by descending weight.
#[derive(Debug)]
struct SegmentMap {
    new_to_old: Vec<usize>,
    old_to_new: Vec<usize>,
}

impl SegmentMap {
    fn new(weights: &[i64]) -> Self {
        let mut new_to_old: Vec<usize> = (0..weights.len()).collect();
        new_to_old.sort_by(|&a, &b| weights[b].cmp(&weights[a]));
        let mut old_to_new = vec![0usize; weights.len()];
        for (new, &old) in new_to_old.iter().enumerate() {
            old_to_new[old] = new;
        }
        Self {
            new_to_old,
            old_to_new,
        }
    }

    fn new_to_old(&self, segment: usize) -> usize {
        self.new_to_old[segment]
    }

    fn old_to_new(&self, segment: usize) -> usize {
        self.old_to_new[segment]
    }
}

/// Wrapper around a [`TermsEnum`] and an integer that identifies it.
///
/// Equivalent to `org.apache.lucene.index.TermsEnumIndex`.
struct TermsEnumIndex {
    terms_enum: Box<dyn TermsEnum>,
    sub_index: usize,
    current_term: Option<BytesRef>,
}

impl TermsEnumIndex {
    fn new(terms_enum: Box<dyn TermsEnum>, sub_index: usize) -> Self {
        Self {
            terms_enum,
            sub_index,
            current_term: None,
        }
    }

    fn term(&self) -> Option<&BytesRef> {
        self.current_term.as_ref()
    }

    fn next(&mut self) -> Result<Option<&BytesRef>> {
        let term = self.terms_enum.next()?;
        self.current_term = term;
        Ok(self.current_term.as_ref())
    }

    #[allow(dead_code)]
    fn seek_ceil(&mut self, term: &BytesRef) -> Result<SeekStatus> {
        let status = self.terms_enum.seek_ceil(term)?;
        if status == SeekStatus::END {
            self.current_term = None;
        } else {
            self.current_term = Some(self.terms_enum.term()?);
        }
        Ok(status)
    }

    #[allow(dead_code)]
    fn seek_exact(&mut self, term: &BytesRef) -> Result<bool> {
        let found = self.terms_enum.seek_exact(term)?;
        if found {
            self.current_term = Some(self.terms_enum.term()?);
        } else {
            self.current_term = None;
        }
        Ok(found)
    }

    #[allow(dead_code)]
    fn seek_ord(&mut self, ord: i64) -> Result<()> {
        self.terms_enum.seek_ord(ord)?;
        self.current_term = Some(self.terms_enum.term()?);
        Ok(())
    }

    fn term_equals(&self, state: &TermsEnumIndexState) -> bool {
        match &self.current_term {
            None => false,
            Some(term) => term == &state.term,
        }
    }
}

/// Saved state of a [`TermsEnumIndex`] for equality checks.
struct TermsEnumIndexState {
    term: BytesRef,
}

impl TermsEnumIndexState {
    fn copy_from(tei: &TermsEnumIndex) -> Self {
        Self {
            term: tei.current_term.clone().unwrap_or_default(),
        }
    }
}

impl Debug for TermsEnumIndexState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TermsEnumIndexState")
            .field("term", &self.term)
            .finish()
    }
}

/// Priority queue ordering for [`TermsEnumIndex`] by current term.
struct TermsEnumIndexComparator;

impl PriorityQueueComparator<TermsEnumIndex> for TermsEnumIndexComparator {
    fn less_than(&self, a: &TermsEnumIndex, b: &TermsEnumIndex) -> bool {
        match (a.term(), b.term()) {
            (Some(a_term), Some(b_term)) => a_term < b_term,
            (None, _) => false,
            (_, None) => true,
        }
    }
}

/// Long values backed by a plain vector.
#[derive(Debug)]
struct ArrayLongValues(Vec<i64>);

impl LongValues for ArrayLongValues {
    fn get(&self, index: i64) -> i64 {
        self.0[index as usize]
    }
}

/// Long values that add a per-index delta.
#[derive(Debug)]
struct DeltaLongValues(Vec<i64>);

impl LongValues for DeltaLongValues {
    fn get(&self, index: i64) -> i64 {
        index + self.0[index as usize]
    }
}

// -----------------------------------------------------------------------------
// Sorted doc-values terms enums
// -----------------------------------------------------------------------------

/// Implements a [`TermsEnum`] wrapping a provided [`SortedDocValues`].
///
/// Equivalent to `org.apache.lucene.index.SortedDocValuesTermsEnum`.
pub struct SortedDocValuesTermsEnum {
    values: Box<dyn SortedDocValues>,
    current_ord: i64,
    scratch: BytesRefBuilder,
    atts: AttributeSource,
}

impl SortedDocValuesTermsEnum {
    /// Creates a new terms enum over the provided sorted doc values.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(values: Box<dyn SortedDocValues>) -> Box<dyn TermsEnum> {
        Box::new(Self {
            values,
            current_ord: -1,
            scratch: BytesRefBuilder::new(),
            atts: AttributeSource::new(),
        })
    }
}

impl TermsEnum for SortedDocValuesTermsEnum {
    fn attributes(&mut self) -> &mut AttributeSource {
        &mut self.atts
    }

    fn seek_ceil(&mut self, text: &BytesRef) -> Result<SeekStatus> {
        let ord = self.values.lookup_term(text)?;
        if ord >= 0 {
            self.current_ord = ord as i64;
            self.scratch.copy_ref(text);
            Ok(SeekStatus::FOUND)
        } else {
            self.current_ord = -ord as i64 - 1;
            let value_count = self.values.get_value_count()? as i64;
            if self.current_ord == value_count {
                Ok(SeekStatus::END)
            } else {
                let term = self.values.lookup_ord(self.current_ord as i32)?;
                self.scratch.copy_ref(&term);
                Ok(SeekStatus::NOT_FOUND)
            }
        }
    }

    fn seek_exact(&mut self, text: &BytesRef) -> Result<bool> {
        let ord = self.values.lookup_term(text)?;
        if ord >= 0 {
            self.current_ord = ord as i64;
            self.scratch.copy_ref(text);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn seek_ord(&mut self, ord: i64) -> Result<()> {
        let value_count = self.values.get_value_count()? as i64;
        if ord < 0 || ord >= value_count {
            return Err(LuceneError::IllegalArgument(format!(
                "ord {ord} out of range [0, {value_count})"
            )));
        }
        self.current_ord = ord;
        let term = self.values.lookup_ord(ord as i32)?;
        self.scratch.copy_ref(&term);
        Ok(())
    }

    fn term(&self) -> Result<BytesRef> {
        Ok(self.scratch.get())
    }

    fn ord(&self) -> Result<i64> {
        Ok(self.current_ord)
    }

    fn doc_freq(&self) -> Result<i32> {
        Err(LuceneError::UnsupportedOperation(
            "docFreq not supported by SortedDocValuesTermsEnum".to_string(),
        ))
    }

    fn total_term_freq(&self) -> Result<i64> {
        Err(LuceneError::UnsupportedOperation(
            "totalTermFreq not supported by SortedDocValuesTermsEnum".to_string(),
        ))
    }

    fn postings(
        &mut self,
        _reuse: Option<Box<dyn PostingsEnum>>,
        _flags: i32,
    ) -> Result<Box<dyn PostingsEnum>> {
        Err(LuceneError::UnsupportedOperation(
            "postings not supported by SortedDocValuesTermsEnum".to_string(),
        ))
    }

    fn impacts(&mut self, _flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        Err(LuceneError::UnsupportedOperation(
            "impacts not supported by SortedDocValuesTermsEnum".to_string(),
        ))
    }

    fn seek_term_state(&mut self, _text: &BytesRef, state: &dyn TermState) -> Result<()> {
        let state = state
            .as_any()
            .downcast_ref::<OrdTermState>()
            .ok_or_else(|| {
                LuceneError::IllegalArgument("state must be an OrdTermState".to_string())
            })?;
        self.seek_ord(state.ord)
    }

    fn term_state(&mut self) -> Result<Box<dyn TermState>> {
        Ok(Box::new(OrdTermState {
            ord: self.current_ord,
        }))
    }

    fn next(&mut self) -> Result<Option<BytesRef>> {
        self.current_ord += 1;
        let value_count = self.values.get_value_count()? as i64;
        if self.current_ord >= value_count {
            return Ok(None);
        }
        let term = self.values.lookup_ord(self.current_ord as i32)?;
        self.scratch.copy_ref(&term);
        Ok(Some(self.scratch.get()))
    }
}

/// Implements a [`TermsEnum`] wrapping a provided [`SortedSetDocValues`].
///
/// Equivalent to `org.apache.lucene.index.SortedSetDocValuesTermsEnum`.
pub struct SortedSetDocValuesTermsEnum {
    values: Box<dyn SortedSetDocValues>,
    current_ord: i64,
    scratch: BytesRefBuilder,
    atts: AttributeSource,
}

impl SortedSetDocValuesTermsEnum {
    /// Creates a new terms enum over the provided sorted-set doc values.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(values: Box<dyn SortedSetDocValues>) -> Box<dyn TermsEnum> {
        Box::new(Self {
            values,
            current_ord: -1,
            scratch: BytesRefBuilder::new(),
            atts: AttributeSource::new(),
        })
    }
}

impl TermsEnum for SortedSetDocValuesTermsEnum {
    fn attributes(&mut self) -> &mut AttributeSource {
        &mut self.atts
    }

    fn seek_ceil(&mut self, text: &BytesRef) -> Result<SeekStatus> {
        let ord = self.values.lookup_term(text)?;
        if ord >= 0 {
            self.current_ord = ord;
            self.scratch.copy_ref(text);
            Ok(SeekStatus::FOUND)
        } else {
            self.current_ord = -ord - 1;
            let value_count = self.values.get_value_count()?;
            if self.current_ord == value_count {
                Ok(SeekStatus::END)
            } else {
                let term = self.values.lookup_ord(self.current_ord)?;
                self.scratch.copy_ref(&term);
                Ok(SeekStatus::NOT_FOUND)
            }
        }
    }

    fn seek_exact(&mut self, text: &BytesRef) -> Result<bool> {
        let ord = self.values.lookup_term(text)?;
        if ord >= 0 {
            self.current_ord = ord;
            self.scratch.copy_ref(text);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn seek_ord(&mut self, ord: i64) -> Result<()> {
        let value_count = self.values.get_value_count()?;
        if ord < 0 || ord >= value_count {
            return Err(LuceneError::IllegalArgument(format!(
                "ord {ord} out of range [0, {value_count})"
            )));
        }
        self.current_ord = ord;
        let term = self.values.lookup_ord(ord)?;
        self.scratch.copy_ref(&term);
        Ok(())
    }

    fn term(&self) -> Result<BytesRef> {
        Ok(self.scratch.get())
    }

    fn ord(&self) -> Result<i64> {
        Ok(self.current_ord)
    }

    fn doc_freq(&self) -> Result<i32> {
        Err(LuceneError::UnsupportedOperation(
            "docFreq not supported by SortedSetDocValuesTermsEnum".to_string(),
        ))
    }

    fn total_term_freq(&self) -> Result<i64> {
        Err(LuceneError::UnsupportedOperation(
            "totalTermFreq not supported by SortedSetDocValuesTermsEnum".to_string(),
        ))
    }

    fn postings(
        &mut self,
        _reuse: Option<Box<dyn PostingsEnum>>,
        _flags: i32,
    ) -> Result<Box<dyn PostingsEnum>> {
        Err(LuceneError::UnsupportedOperation(
            "postings not supported by SortedSetDocValuesTermsEnum".to_string(),
        ))
    }

    fn impacts(&mut self, _flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        Err(LuceneError::UnsupportedOperation(
            "impacts not supported by SortedSetDocValuesTermsEnum".to_string(),
        ))
    }

    fn seek_term_state(&mut self, _text: &BytesRef, state: &dyn TermState) -> Result<()> {
        let state = state
            .as_any()
            .downcast_ref::<OrdTermState>()
            .ok_or_else(|| {
                LuceneError::IllegalArgument("state must be an OrdTermState".to_string())
            })?;
        self.seek_ord(state.ord)
    }

    fn term_state(&mut self) -> Result<Box<dyn TermState>> {
        Ok(Box::new(OrdTermState {
            ord: self.current_ord,
        }))
    }

    fn next(&mut self) -> Result<Option<BytesRef>> {
        self.current_ord += 1;
        let value_count = self.values.get_value_count()?;
        if self.current_ord >= value_count {
            return Ok(None);
        }
        let term = self.values.lookup_ord(self.current_ord)?;
        self.scratch.copy_ref(&term);
        Ok(Some(self.scratch.get()))
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::DocIdSetIterator;

    /// Numeric doc values backed by a sorted list of `(doc_id, value)` pairs.
    struct VecNumericDocValues {
        docs: Vec<(i32, i64)>,
        current_doc: i32,
        idx: i32,
    }

    impl VecNumericDocValues {
        fn new(docs: Vec<(i32, i64)>) -> Self {
            Self {
                docs,
                current_doc: -1,
                idx: -1,
            }
        }

        fn find(&self, target: i32) -> i32 {
            self.docs
                .iter()
                .position(|(d, _)| *d >= target)
                .map(|p| p as i32)
                .unwrap_or(self.docs.len() as i32)
        }
    }

    impl DocIdSetIterator for VecNumericDocValues {
        fn doc_id(&self) -> i32 {
            self.current_doc
        }

        fn next_doc(&mut self) -> Result<i32> {
            let start = (self.idx + 1).max(0) as usize;
            if let Some(p) = self.docs[start..]
                .iter()
                .position(|(d, _)| *d > self.current_doc)
            {
                self.idx = (start + p) as i32;
                self.current_doc = self.docs[self.idx as usize].0;
            } else {
                self.idx = self.docs.len() as i32;
                self.current_doc = NO_MORE_DOCS;
            }
            Ok(self.current_doc)
        }

        fn advance(&mut self, target: i32) -> Result<i32> {
            self.idx = self.find(target);
            self.current_doc = if (self.idx as usize) < self.docs.len() {
                self.docs[self.idx as usize].0
            } else {
                NO_MORE_DOCS
            };
            Ok(self.current_doc)
        }

        fn cost(&self) -> i64 {
            self.docs.len() as i64
        }
    }

    impl DocValuesIterator for VecNumericDocValues {
        fn advance_exact(&mut self, target: i32) -> Result<bool> {
            self.current_doc = target;
            self.idx = self.find(target);
            let found =
                (self.idx as usize) < self.docs.len() && self.docs[self.idx as usize].0 == target;
            if !found {
                self.idx = -1;
            }
            Ok(found)
        }
    }

    impl NumericDocValues for VecNumericDocValues {
        fn long_value(&self) -> Result<i64> {
            if self.idx < 0 || self.idx as usize >= self.docs.len() {
                return Err(LuceneError::IllegalState(
                    "long_value called with no current document".to_string(),
                ));
            }
            Ok(self.docs[self.idx as usize].1)
        }
    }

    /// Binary doc values backed by a sorted list of `(doc_id, value)` pairs.
    struct VecBinaryDocValues {
        docs: Vec<(i32, Vec<u8>)>,
        idx: i32,
    }

    impl VecBinaryDocValues {
        fn new(docs: Vec<(i32, Vec<u8>)>) -> Self {
            Self { docs, idx: -1 }
        }

        fn find(&self, target: i32) -> i32 {
            self.docs
                .iter()
                .position(|(d, _)| *d >= target)
                .map(|p| p as i32)
                .unwrap_or(self.docs.len() as i32)
        }
    }

    impl DocIdSetIterator for VecBinaryDocValues {
        fn doc_id(&self) -> i32 {
            if self.idx < 0 {
                -1
            } else if self.idx as usize >= self.docs.len() {
                NO_MORE_DOCS
            } else {
                self.docs[self.idx as usize].0
            }
        }

        fn next_doc(&mut self) -> Result<i32> {
            self.idx += 1;
            Ok(self.doc_id())
        }

        fn advance(&mut self, target: i32) -> Result<i32> {
            self.idx = self.find(target);
            Ok(self.doc_id())
        }

        fn cost(&self) -> i64 {
            self.docs.len() as i64
        }
    }

    impl DocValuesIterator for VecBinaryDocValues {
        fn advance_exact(&mut self, target: i32) -> Result<bool> {
            self.idx = self.find(target);
            Ok(self.doc_id() == target)
        }
    }

    impl BinaryDocValues for VecBinaryDocValues {
        fn binary_value(&self) -> Result<BytesRef> {
            if self.idx < 0 || self.idx as usize >= self.docs.len() {
                return Err(LuceneError::IllegalState(
                    "binary_value called with no current document".to_string(),
                ));
            }
            Ok(BytesRef::new(self.docs[self.idx as usize].1.clone()))
        }
    }

    /// Sorted doc values backed by a dictionary and per-document ordinals.
    struct VecSortedDocValues {
        dictionary: Vec<Vec<u8>>,
        ords: Vec<i32>,
        idx: i32,
    }

    impl VecSortedDocValues {
        fn new(dictionary: Vec<Vec<u8>>, ords: Vec<i32>) -> Self {
            Self {
                dictionary,
                ords,
                idx: -1,
            }
        }
    }

    impl DocIdSetIterator for VecSortedDocValues {
        fn doc_id(&self) -> i32 {
            if self.idx < 0 {
                -1
            } else if self.idx as usize >= self.ords.len() {
                NO_MORE_DOCS
            } else {
                self.idx
            }
        }

        fn next_doc(&mut self) -> Result<i32> {
            self.idx += 1;
            while self.idx < self.ords.len() as i32 && self.ords[self.idx as usize] == -1 {
                self.idx += 1;
            }
            Ok(self.doc_id())
        }

        fn advance(&mut self, target: i32) -> Result<i32> {
            self.idx = target.max(0) - 1;
            self.next_doc()
        }

        fn cost(&self) -> i64 {
            self.ords.iter().filter(|o| **o != -1).count() as i64
        }
    }

    impl DocValuesIterator for VecSortedDocValues {
        fn advance_exact(&mut self, target: i32) -> Result<bool> {
            self.idx = target;
            if self.idx < 0 || self.idx as usize >= self.ords.len() {
                self.idx = self.ords.len() as i32;
                return Ok(false);
            }
            Ok(self.ords[self.idx as usize] != -1)
        }
    }

    impl SortedDocValues for VecSortedDocValues {
        fn ord_value(&self) -> Result<i32> {
            if self.idx < 0 || self.idx as usize >= self.ords.len() {
                return Err(LuceneError::IllegalState(
                    "ord_value called with no current document".to_string(),
                ));
            }
            Ok(self.ords[self.idx as usize])
        }

        fn get_value_count(&self) -> Result<i32> {
            Ok(self.dictionary.len() as i32)
        }

        fn lookup_ord(&self, ord: i32) -> Result<BytesRef> {
            if ord < 0 || ord as usize >= self.dictionary.len() {
                return Err(LuceneError::IllegalArgument(format!(
                    "ordinal {ord} out of range [0, {})",
                    self.dictionary.len()
                )));
            }
            Ok(BytesRef::new(self.dictionary[ord as usize].clone()))
        }
    }

    /// Sorted-numeric doc values backed by per-document value lists.
    struct VecSortedNumericDocValues {
        values: Vec<Vec<i64>>,
        idx: i32,
        value_idx: usize,
    }

    impl VecSortedNumericDocValues {
        fn new(values: Vec<Vec<i64>>) -> Self {
            Self {
                values,
                idx: -1,
                value_idx: 0,
            }
        }
    }

    impl DocIdSetIterator for VecSortedNumericDocValues {
        fn doc_id(&self) -> i32 {
            if self.idx < 0 {
                -1
            } else if self.idx as usize >= self.values.len() {
                NO_MORE_DOCS
            } else {
                self.idx
            }
        }

        fn next_doc(&mut self) -> Result<i32> {
            self.value_idx = 0;
            loop {
                self.idx += 1;
                if self.idx as usize >= self.values.len() {
                    return Ok(NO_MORE_DOCS);
                }
                if !self.values[self.idx as usize].is_empty() {
                    return Ok(self.idx);
                }
            }
        }

        fn advance(&mut self, target: i32) -> Result<i32> {
            self.value_idx = 0;
            self.idx = target - 1;
            self.next_doc()
        }

        fn cost(&self) -> i64 {
            self.values.iter().filter(|v| !v.is_empty()).count() as i64
        }
    }

    impl DocValuesIterator for VecSortedNumericDocValues {
        fn advance_exact(&mut self, target: i32) -> Result<bool> {
            self.value_idx = 0;
            self.idx = target;
            if self.idx < 0 || self.idx as usize >= self.values.len() {
                self.idx = self.values.len() as i32;
                return Ok(false);
            }
            Ok(!self.values[self.idx as usize].is_empty())
        }
    }

    impl SortedNumericDocValues for VecSortedNumericDocValues {
        fn next_value(&mut self) -> Result<i64> {
            if self.idx < 0 || self.idx as usize >= self.values.len() {
                return Err(LuceneError::IllegalState(
                    "next_value called with no current document".to_string(),
                ));
            }
            if self.value_idx >= self.values[self.idx as usize].len() {
                return Err(LuceneError::IllegalState(
                    "next_value called more than doc_value_count times".to_string(),
                ));
            }
            let v = self.values[self.idx as usize][self.value_idx];
            self.value_idx += 1;
            Ok(v)
        }

        fn doc_value_count(&self) -> Result<i32> {
            if self.idx < 0 || self.idx as usize >= self.values.len() {
                return Err(LuceneError::IllegalState(
                    "doc_value_count called with no current document".to_string(),
                ));
            }
            Ok(self.values[self.idx as usize].len() as i32)
        }
    }

    /// Sorted-set doc values backed by a dictionary and per-document ord lists.
    struct VecSortedSetDocValues {
        dictionary: Vec<Vec<u8>>,
        ords: Vec<Vec<i64>>,
        idx: i32,
        ord_idx: usize,
    }

    impl VecSortedSetDocValues {
        fn new(dictionary: Vec<Vec<u8>>, ords: Vec<Vec<i64>>) -> Self {
            Self {
                dictionary,
                ords,
                idx: -1,
                ord_idx: 0,
            }
        }
    }

    impl DocIdSetIterator for VecSortedSetDocValues {
        fn doc_id(&self) -> i32 {
            if self.idx < 0 {
                -1
            } else if self.idx as usize >= self.ords.len() {
                NO_MORE_DOCS
            } else {
                self.idx
            }
        }

        fn next_doc(&mut self) -> Result<i32> {
            self.ord_idx = 0;
            loop {
                self.idx += 1;
                if self.idx as usize >= self.ords.len() {
                    return Ok(NO_MORE_DOCS);
                }
                if !self.ords[self.idx as usize].is_empty() {
                    return Ok(self.idx);
                }
            }
        }

        fn advance(&mut self, target: i32) -> Result<i32> {
            self.ord_idx = 0;
            self.idx = target - 1;
            self.next_doc()
        }

        fn cost(&self) -> i64 {
            self.ords.iter().filter(|o| !o.is_empty()).count() as i64
        }
    }

    impl DocValuesIterator for VecSortedSetDocValues {
        fn advance_exact(&mut self, target: i32) -> Result<bool> {
            self.ord_idx = 0;
            self.idx = target;
            if self.idx < 0 || self.idx as usize >= self.ords.len() {
                self.idx = self.ords.len() as i32;
                return Ok(false);
            }
            Ok(!self.ords[self.idx as usize].is_empty())
        }
    }

    impl SortedSetDocValues for VecSortedSetDocValues {
        fn next_ord(&mut self) -> Result<i64> {
            if self.idx < 0 || self.idx as usize >= self.ords.len() {
                return Err(LuceneError::IllegalState(
                    "next_ord called with no current document".to_string(),
                ));
            }
            if self.ord_idx >= self.ords[self.idx as usize].len() {
                return Err(LuceneError::IllegalState(
                    "next_ord called more than doc_value_count times".to_string(),
                ));
            }
            let ord = self.ords[self.idx as usize][self.ord_idx];
            self.ord_idx += 1;
            Ok(ord)
        }

        fn doc_value_count(&self) -> Result<i32> {
            if self.idx < 0 || self.idx as usize >= self.ords.len() {
                return Err(LuceneError::IllegalState(
                    "doc_value_count called with no current document".to_string(),
                ));
            }
            Ok(self.ords[self.idx as usize].len() as i32)
        }

        fn lookup_ord(&self, ord: i64) -> Result<BytesRef> {
            if ord < 0 || ord as usize >= self.dictionary.len() {
                return Err(LuceneError::IllegalArgument(format!(
                    "ordinal {ord} out of range [0, {})",
                    self.dictionary.len()
                )));
            }
            Ok(BytesRef::new(self.dictionary[ord as usize].clone()))
        }

        fn get_value_count(&self) -> Result<i64> {
            Ok(self.dictionary.len() as i64)
        }
    }

    #[test]
    fn numeric_doc_values_iterator_contract() {
        let mut values = VecNumericDocValues::new(vec![(0, 10), (2, 30), (5, 60)]);
        assert_eq!(values.doc_id(), -1);
        assert_eq!(values.cost(), 3);
        assert_eq!(values.next_doc().unwrap(), 0);
        assert_eq!(values.long_value().unwrap(), 10);
        assert_eq!(values.advance(4).unwrap(), 5);
        assert_eq!(values.long_value().unwrap(), 60);
        assert_eq!(values.advance(6).unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn numeric_doc_values_advance_exact() {
        let mut values = VecNumericDocValues::new(vec![(0, 10), (2, 30), (5, 60)]);
        assert!(values.advance_exact(2).unwrap());
        assert_eq!(values.doc_id(), 2);
        assert_eq!(values.long_value().unwrap(), 30);
        assert!(!values.advance_exact(3).unwrap());
        assert_eq!(values.doc_id(), 3);
    }

    #[test]
    fn numeric_doc_values_long_values_bulk() {
        let mut values = VecNumericDocValues::new(vec![(0, 10), (2, 30), (5, 60)]);
        let docs = vec![0, 1, 2, 3, 5];
        let mut out = vec![0i64; 5];
        values.long_values(5, &docs, 0, &mut out, 0, -1).unwrap();
        assert_eq!(out, vec![10, -1, 30, -1, 60]);
    }

    #[test]
    fn numeric_doc_values_range_into_bit_set() {
        let mut values = VecNumericDocValues::new(vec![(0, 10), (2, 30), (5, 60)]);
        let mut bits = FixedBitSet::new(10);
        values
            .range_into_bit_set(0, 6, 20, 50, &mut bits, 0)
            .unwrap();
        assert!(bits.get(2));
        assert!(!bits.get(0));
        assert!(!bits.get(5));
    }

    #[test]
    fn binary_doc_values_iterator_contract() {
        let mut values =
            VecBinaryDocValues::new(vec![(0, vec![b'a']), (2, vec![b'b']), (5, vec![b'c'])]);
        assert_eq!(values.next_doc().unwrap(), 0);
        assert_eq!(values.binary_value().unwrap().slice(), b"a");
        assert_eq!(values.advance(3).unwrap(), 5);
        assert_eq!(values.binary_value().unwrap().slice(), b"c");
        assert_eq!(values.next_doc().unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn sorted_doc_values_iterator_and_lookup() {
        let dict = vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()];
        // doc 0 -> a (ord 0), doc 1 -> missing, doc 2 -> c (ord 2), doc 3 -> b (ord 1)
        let ords = vec![0, -1, 2, 1];
        let mut values = VecSortedDocValues::new(dict, ords);
        assert_eq!(values.next_doc().unwrap(), 0);
        assert_eq!(values.ord_value().unwrap(), 0);
        assert_eq!(values.lookup_ord(0).unwrap().slice(), b"a");
        assert_eq!(values.advance(2).unwrap(), 2);
        assert_eq!(values.ord_value().unwrap(), 2);
        assert_eq!(
            values.lookup_term(&BytesRef::new(b"b".to_vec())).unwrap(),
            1
        );
        assert_eq!(
            values.lookup_term(&BytesRef::new(b"z".to_vec())).unwrap(),
            -4
        );
    }

    #[test]
    fn sorted_numeric_doc_values_iterator() {
        let values_data: Vec<Vec<i64>> =
            vec![vec![10, 20], vec![], vec![30], vec![], vec![40, 50, 60]];
        let mut values = VecSortedNumericDocValues::new(values_data);
        assert_eq!(values.next_doc().unwrap(), 0);
        assert_eq!(values.doc_value_count().unwrap(), 2);
        assert_eq!(values.next_value().unwrap(), 10);
        assert_eq!(values.next_value().unwrap(), 20);
        assert_eq!(values.advance(3).unwrap(), 4);
        assert_eq!(values.doc_value_count().unwrap(), 3);
        assert_eq!(values.next_value().unwrap(), 40);
    }

    #[test]
    fn sorted_set_doc_values_iterator() {
        let dict = vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()];
        let ords: Vec<Vec<i64>> = vec![vec![0, 2], vec![], vec![1], vec![]];
        let mut values = VecSortedSetDocValues::new(dict, ords);
        assert_eq!(values.next_doc().unwrap(), 0);
        assert_eq!(values.doc_value_count().unwrap(), 2);
        assert_eq!(values.next_ord().unwrap(), 0);
        assert_eq!(values.next_ord().unwrap(), 2);
        assert_eq!(values.lookup_ord(2).unwrap().slice(), b"c");
        assert_eq!(values.advance(2).unwrap(), 2);
        assert_eq!(values.next_ord().unwrap(), 1);
    }

    #[test]
    fn empty_numeric_doc_values_returns_no_docs() {
        let mut values = EmptyNumericDocValues::new();
        assert_eq!(values.doc_id(), -1);
        assert_eq!(values.cost(), 0);
        assert_eq!(values.next_doc().unwrap(), NO_MORE_DOCS);
        assert!(!values.advance_exact(0).unwrap());
        assert_eq!(values.doc_id(), 0);
        assert!(values.long_value().is_err());
    }

    #[test]
    fn empty_binary_doc_values_returns_no_docs() {
        let mut values = EmptyBinaryDocValues::new();
        assert_eq!(values.advance(10).unwrap(), NO_MORE_DOCS);
        assert!(values.binary_value().is_err());
    }

    #[test]
    fn empty_sorted_doc_values_has_empty_dictionary() {
        let values = EmptySortedDocValues::new();
        assert_eq!(values.get_value_count().unwrap(), 0);
        assert_eq!(values.lookup_ord(0).unwrap().length, 0);
        assert!(values.ord_value().is_err());
    }

    #[test]
    fn empty_sorted_numeric_doc_values_has_zero_values() {
        let values = EmptySortedNumericDocValues::new();
        assert_eq!(values.doc_value_count().unwrap(), 0);
    }

    #[test]
    fn empty_sorted_set_doc_values_has_empty_dictionary() {
        let values = EmptySortedSetDocValues::new();
        assert_eq!(values.get_value_count().unwrap(), 0);
        assert_eq!(values.lookup_ord(0).unwrap().length, 0);
    }

    #[test]
    fn empty_doc_values_skipper_reports_initial_state() {
        let skipper = EmptyDocValuesSkipper;
        assert_eq!(skipper.num_levels(), 1);
        assert_eq!(skipper.min_doc_id(0), -1);
        assert_eq!(skipper.global_doc_count(), 0);
        assert_eq!(skipper.max_value_count(), 0);
    }

    #[test]
    fn doc_values_helper_factory_functions() {
        let mut numeric = DocValues::empty_numeric();
        assert_eq!(numeric.next_doc().unwrap(), NO_MORE_DOCS);
        let mut binary = DocValues::empty_binary();
        assert_eq!(binary.next_doc().unwrap(), NO_MORE_DOCS);
        let mut sorted = DocValues::empty_sorted();
        assert_eq!(sorted.next_doc().unwrap(), NO_MORE_DOCS);
        let mut sorted_numeric = DocValues::empty_sorted_numeric();
        assert_eq!(sorted_numeric.next_doc().unwrap(), NO_MORE_DOCS);
        let mut sorted_set = DocValues::empty_sorted_set();
        assert_eq!(sorted_set.next_doc().unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn singleton_numeric_wrapper() {
        let inner = Box::new(VecNumericDocValues::new(vec![(1, 100), (3, 300)]));
        let mut values = DocValues::singleton_numeric(inner);
        assert_eq!(values.next_doc().unwrap(), 1);
        assert_eq!(values.doc_value_count().unwrap(), 1);
        assert_eq!(values.next_value().unwrap(), 100);
        assert_eq!(values.advance(3).unwrap(), 3);
        assert_eq!(values.next_value().unwrap(), 300);
    }

    #[test]
    fn singleton_sorted_wrapper() {
        let dict = vec![b"x".to_vec(), b"y".to_vec()];
        let ords = vec![-1, 0, 1];
        let inner = Box::new(VecSortedDocValues::new(dict, ords));
        let mut values = DocValues::singleton_sorted(inner);
        assert_eq!(values.next_doc().unwrap(), 1);
        assert_eq!(values.doc_value_count().unwrap(), 1);
        assert_eq!(values.next_ord().unwrap(), 0);
        assert_eq!(values.get_value_count().unwrap(), 2);
    }

    #[test]
    fn doc_values_skipper_advance_range_exhausts_when_no_overlap() {
        // A simple skipper with a single level whose interval is [0, 5] and
        // value range [0, 10]. Advancing to a disjoint value range should
        // exhaust it (min_doc_id becomes NO_MORE_DOCS).
        struct StubSkipper {
            min_doc: i32,
            max_doc: i32,
        }
        impl DocValuesSkipper for StubSkipper {
            fn advance(&mut self, target: i32) -> Result<()> {
                if target > self.max_doc {
                    self.min_doc = NO_MORE_DOCS;
                    self.max_doc = NO_MORE_DOCS;
                } else {
                    self.min_doc = target.max(self.min_doc);
                }
                Ok(())
            }
            fn num_levels(&self) -> i32 {
                1
            }
            fn min_doc_id(&self, _level: i32) -> i32 {
                self.min_doc
            }
            fn max_doc_id(&self, _level: i32) -> i32 {
                self.max_doc
            }
            fn min_value(&self, _level: i32) -> i64 {
                0
            }
            fn max_value(&self, _level: i32) -> i64 {
                10
            }
            fn doc_count(&self, _level: i32) -> i32 {
                1
            }
            fn global_min_value(&self) -> i64 {
                0
            }
            fn global_max_value(&self) -> i64 {
                10
            }
            fn global_doc_count(&self) -> i32 {
                1
            }
        }
        let mut skipper = StubSkipper {
            min_doc: -1,
            max_doc: 5,
        };
        skipper.advance_range(20, 30).unwrap();
        assert_eq!(skipper.min_doc_id(0), NO_MORE_DOCS);
    }

    #[test]
    fn empty_doc_values_producer_returns_empty_iterators() {
        use crate::codecs::doc_values::DocValuesProducer;

        let producer = EmptyDocValuesProducer;
        let field = crate::index::FieldInfo::default();
        let mut numeric = producer.get_numeric(&field).unwrap();
        assert_eq!(numeric.next_doc().unwrap(), NO_MORE_DOCS);
        let mut binary = producer.get_binary(&field).unwrap();
        assert_eq!(binary.next_doc().unwrap(), NO_MORE_DOCS);
        let mut sorted = producer.get_sorted(&field).unwrap();
        assert_eq!(sorted.next_doc().unwrap(), NO_MORE_DOCS);
        let mut sorted_numeric = producer.get_sorted_numeric(&field).unwrap();
        assert_eq!(sorted_numeric.next_doc().unwrap(), NO_MORE_DOCS);
        let mut sorted_set = producer.get_sorted_set(&field).unwrap();
        assert_eq!(sorted_set.next_doc().unwrap(), NO_MORE_DOCS);
        let skipper = producer.get_skipper(&field).unwrap();
        assert_eq!(skipper.num_levels(), 1);
        producer.check_integrity().unwrap();
        let mut clone = producer.get_merge_instance().unwrap();
        clone.close().unwrap();
    }

    #[test]
    fn filter_numeric_doc_values_forwards_calls() {
        let inner = Box::new(VecNumericDocValues::new(vec![(0, 10), (2, 30)]));
        let mut filtered = FilterNumericDocValues::new(inner);
        assert_eq!(filtered.next_doc().unwrap(), 0);
        assert_eq!(filtered.long_value().unwrap(), 10);
        assert_eq!(filtered.advance(2).unwrap(), 2);
        assert!(filtered.advance_exact(2).unwrap());
        assert_eq!(filtered.long_value().unwrap(), 30);
        assert_eq!(filtered.get_inner().doc_id(), 2);
    }

    #[test]
    fn filter_binary_doc_values_forwards_calls() {
        let inner = Box::new(VecBinaryDocValues::new(vec![
            (0, b"a".to_vec()),
            (2, b"b".to_vec()),
        ]));
        let mut filtered = FilterBinaryDocValues::new(inner);
        assert_eq!(filtered.next_doc().unwrap(), 0);
        assert_eq!(filtered.binary_value().unwrap().slice(), b"a");
        assert_eq!(filtered.advance(2).unwrap(), 2);
        assert_eq!(filtered.binary_value().unwrap().slice(), b"b");
    }

    #[test]
    fn filter_sorted_doc_values_forwards_calls() {
        let dict = vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()];
        let ords = vec![0, -1, 2, 1];
        let inner = Box::new(VecSortedDocValues::new(dict, ords));
        let mut filtered = FilterSortedDocValues::new(inner);
        assert_eq!(filtered.next_doc().unwrap(), 0);
        assert_eq!(filtered.ord_value().unwrap(), 0);
        assert_eq!(filtered.lookup_ord(2).unwrap().slice(), b"c");
        assert_eq!(
            filtered.lookup_term(&BytesRef::new(b"b".to_vec())).unwrap(),
            1
        );
        assert_eq!(filtered.get_value_count().unwrap(), 3);
    }

    #[test]
    fn filter_sorted_numeric_doc_values_forwards_calls() {
        let values_data: Vec<Vec<i64>> = vec![vec![10, 20], vec![], vec![30]];
        let inner = Box::new(VecSortedNumericDocValues::new(values_data));
        let mut filtered = FilterSortedNumericDocValues::new(inner);
        assert_eq!(filtered.next_doc().unwrap(), 0);
        assert_eq!(filtered.doc_value_count().unwrap(), 2);
        assert_eq!(filtered.next_value().unwrap(), 10);
        assert_eq!(filtered.next_value().unwrap(), 20);
    }

    #[test]
    fn filter_sorted_set_doc_values_forwards_calls() {
        let dict = vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()];
        let ords: Vec<Vec<i64>> = vec![vec![0, 2], vec![], vec![1]];
        let inner = Box::new(VecSortedSetDocValues::new(dict, ords));
        let mut filtered = FilterSortedSetDocValues::new(inner);
        assert_eq!(filtered.next_doc().unwrap(), 0);
        assert_eq!(filtered.doc_value_count().unwrap(), 2);
        assert_eq!(filtered.next_ord().unwrap(), 0);
        assert_eq!(filtered.next_ord().unwrap(), 2);
        assert_eq!(filtered.lookup_ord(1).unwrap().slice(), b"b");
        assert_eq!(filtered.get_value_count().unwrap(), 3);
    }

    #[test]
    fn sorted_doc_values_terms_enum_iteration_and_seek() {
        let dict = vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()];
        let ords = vec![0, -1, 2, 1];
        let values = Box::new(VecSortedDocValues::new(dict, ords));
        let mut terms = SortedDocValuesTermsEnum::new(values);
        assert_eq!(terms.next().unwrap().unwrap().slice(), b"a");
        assert_eq!(terms.ord().unwrap(), 0);
        assert_eq!(terms.term().unwrap().slice(), b"a");
        assert_eq!(
            terms.seek_ceil(&BytesRef::new(b"bb".to_vec())).unwrap(),
            SeekStatus::NOT_FOUND
        );
        assert_eq!(terms.term().unwrap().slice(), b"c");
        assert!(terms.seek_exact(&BytesRef::new(b"b".to_vec())).unwrap());
        assert_eq!(terms.ord().unwrap(), 1);
        terms.seek_ord(2).unwrap();
        assert_eq!(terms.term().unwrap().slice(), b"c");
        let state = terms.term_state().unwrap();
        terms
            .seek_term_state(&BytesRef::new(Vec::new()), state.as_ref())
            .unwrap();
        assert_eq!(terms.ord().unwrap(), 2);
        assert!(terms.next().unwrap().is_none());
    }

    #[test]
    fn sorted_set_doc_values_terms_enum_iteration_and_seek() {
        let dict = vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()];
        let ords: Vec<Vec<i64>> = vec![vec![0, 2], vec![], vec![1], vec![]];
        let values = Box::new(VecSortedSetDocValues::new(dict, ords));
        let mut terms = SortedSetDocValuesTermsEnum::new(values);
        assert_eq!(terms.next().unwrap().unwrap().slice(), b"a");
        assert_eq!(terms.ord().unwrap(), 0);
        assert_eq!(terms.term().unwrap().slice(), b"a");
        assert!(!terms.seek_exact(&BytesRef::new(b"z".to_vec())).unwrap());
        assert_eq!(
            terms.seek_ceil(&BytesRef::new(b"b".to_vec())).unwrap(),
            SeekStatus::FOUND
        );
        assert_eq!(terms.ord().unwrap(), 1);
        terms.seek_ord(2).unwrap();
        assert_eq!(terms.term().unwrap().slice(), b"c");
        let state = terms.term_state().unwrap();
        terms
            .seek_term_state(&BytesRef::new(Vec::new()), state.as_ref())
            .unwrap();
        assert_eq!(terms.ord().unwrap(), 2);
    }

    #[test]
    fn ordinal_map_merge_sorted_doc_values() {
        // Segment 0: values {a, c} -> ords [0, 1]
        // Segment 1: values {b, d} -> ords [0, 1]
        // Segment 2: values {a, e} -> ords [0, 1]
        let seg0 = Box::new(VecSortedDocValues::new(
            vec![b"a".to_vec(), b"c".to_vec()],
            vec![0, 1],
        )) as Box<dyn SortedDocValues>;
        let seg1 = Box::new(VecSortedDocValues::new(
            vec![b"b".to_vec(), b"d".to_vec()],
            vec![0, 1],
        )) as Box<dyn SortedDocValues>;
        let seg2 = Box::new(VecSortedDocValues::new(
            vec![b"a".to_vec(), b"e".to_vec()],
            vec![0, 1],
        )) as Box<dyn SortedDocValues>;

        let map = OrdinalMap::build_sorted(vec![seg0, seg1, seg2], 0.0).unwrap();
        // Global order: a, b, c, d, e -> 5 unique values
        assert_eq!(map.get_value_count(), 5);

        // Segment 0 (weight 2) maps local ords 0->a->global 0, 1->c->global 2
        let g0 = map.get_global_ords(0);
        assert_eq!(g0.get(0), 0);
        assert_eq!(g0.get(1), 2);

        // Segment 1 (weight 2) maps local ords 0->b->global 1, 1->d->global 3
        let g1 = map.get_global_ords(1);
        assert_eq!(g1.get(0), 1);
        assert_eq!(g1.get(1), 3);

        // Segment 2 (weight 2) maps local ords 0->a->global 0, 1->e->global 4
        let g2 = map.get_global_ords(2);
        assert_eq!(g2.get(0), 0);
        assert_eq!(g2.get(1), 4);

        // First segment for each global ord
        assert_eq!(map.get_first_segment_number(0), 0); // a first in seg0
        assert_eq!(map.get_first_segment_number(1), 1); // b first in seg1
        assert_eq!(map.get_first_segment_number(2), 0); // c first in seg0
        assert_eq!(map.get_first_segment_number(3), 1); // d first in seg1
        assert_eq!(map.get_first_segment_number(4), 2); // e first in seg2
        assert_eq!(map.get_first_segment_ord(0), 0);
        assert_eq!(map.get_first_segment_ord(1), 0);
        assert_eq!(map.get_first_segment_ord(2), 1);
        assert_eq!(map.get_first_segment_ord(3), 1);
        assert_eq!(map.get_first_segment_ord(4), 1);
    }

    #[test]
    fn ordinal_map_merge_sorted_set_doc_values() {
        // Segment 0: values {a, c}
        // Segment 1: values {a, b, c}
        let seg0 = Box::new(VecSortedSetDocValues::new(
            vec![b"a".to_vec(), b"c".to_vec()],
            vec![vec![0, 1], vec![]],
        )) as Box<dyn SortedSetDocValues>;
        let seg1 = Box::new(VecSortedSetDocValues::new(
            vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()],
            vec![vec![0, 1, 2], vec![]],
        )) as Box<dyn SortedSetDocValues>;

        let map = OrdinalMap::build_sorted_set(vec![seg0, seg1], 0.0).unwrap();
        // Global order: a, b, c -> 3 unique values
        assert_eq!(map.get_value_count(), 3);

        let g0 = map.get_global_ords(0);
        assert_eq!(g0.get(0), 0);
        assert_eq!(g0.get(1), 2);

        let g1 = map.get_global_ords(1);
        assert_eq!(g1.get(0), 0);
        assert_eq!(g1.get(1), 1);
        assert_eq!(g1.get(2), 2);

        assert_eq!(map.get_first_segment_number(0), 1); // a: seg1 has weight 3, sorted first
        assert_eq!(map.get_first_segment_number(1), 1); // b: only in seg1
        assert_eq!(map.get_first_segment_number(2), 1); // c: seg1 first by weight
    }
}
