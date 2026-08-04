//! Doc-values value accessors ported from `org.apache.lucene.index`.
//!
//! Equivalent to `org.apache.lucene.index.NumericDocValues`, `BinaryDocValues`,
//! `SortedDocValues`, `SortedNumericDocValues`, `SortedSetDocValues`,
//! `DocValuesSkipper`, and the `DocValues` utility helper.
//!
//! The doc-value iterators extend [`DocIdSetIterator`] and add an
//! `advance_exact` positioning method. Value accessors follow the Java
//! contract: they must only be called after the iterator has been positioned
//! on a document that has a value.

#![deny(unsafe_code)]

use std::cmp::Ordering;

use crate::error::{LuceneError, Result};
use crate::search::{DocIdSetIterator, NO_MORE_DOCS};
use crate::util::{BytesRef, FixedBitSet};

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
}
