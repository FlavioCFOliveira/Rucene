//! Base doc-id iteration abstractions ported from `org.apache.lucene.search`.
//!
//! This module provides [`DocIdSetIterator`], the fundamental iterator over a
//! non-decreasing sequence of document identifiers, and [`AcceptDocs`], the
//! higher-level filtering abstraction used by codec producers to respect live
//! documents.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::util::{Bits, FixedBitSet};

/// Sentinel returned by [`DocIdSetIterator`] methods when no more documents
/// are available.
///
/// Equivalent to `DocIdSetIterator.NO_MORE_DOCS` in Java Lucene.
pub const NO_MORE_DOCS: i32 = i32::MAX;

/// Iterator over a non-decreasing set of doc IDs.
///
/// The contract mirrors `org.apache.lucene.search.DocIdSetIterator`:
/// - On construction the iterator is unpositioned and [`doc_id`](Self::doc_id)
///   returns `-1`.
/// - [`next_doc`](Self::next_doc) and [`advance`](Self::advance) move forward
///   and return the new current doc ID or [`NO_MORE_DOCS`].
/// - Once exhausted, the iterator must not be used again.
pub trait DocIdSetIterator {
    /// Returns the current doc ID.
    ///
    /// Returns `-1` if the iterator has not yet been positioned, and
    /// [`NO_MORE_DOCS`] if it has been exhausted.
    fn doc_id(&self) -> i32;

    /// Advances to the next document in the set and returns it.
    ///
    /// Returns [`NO_MORE_DOCS`] if there are no more documents.
    fn next_doc(&mut self) -> Result<i32>;

    /// Advances to the first document whose doc ID is greater than or equal to
    /// `target` and returns it.
    ///
    /// Returns [`NO_MORE_DOCS`] if `target` is past the highest matching doc.
    fn advance(&mut self, target: i32) -> Result<i32>;

    /// Returns an estimate of the number of documents this iterator might
    /// match.
    fn cost(&self) -> i64;

    /// Slow (linear) implementation of [`Self::advance`] relying on
    /// [`Self::next_doc`].
    fn slow_advance(&mut self, target: i32) -> Result<i32> {
        debug_assert!(self.doc_id() < target);
        let mut doc = self.doc_id();
        while doc < target {
            doc = self.next_doc()?;
        }
        Ok(doc)
    }

    /// Loads doc IDs in `[doc_id(), up_to)` into `bit_set` at indices
    /// `doc - offset`.
    ///
    /// The default implementation is functionally equivalent to:
    ///
    /// ```text
    /// for (int doc = docID(); doc < upTo; doc = nextDoc()) {
    ///   bitSet.set(doc - offset);
    /// }
    /// ```
    ///
    /// The iterator is advanced to the first doc ID that is `>= up_to`.
    #[allow(clippy::wrong_self_convention)]
    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        let mut doc = self.doc_id();
        if doc == -1 {
            doc = self.next_doc()?;
        }
        debug_assert!(offset <= doc);
        while doc < up_to {
            bit_set.set((doc - offset) as usize);
            doc = self.next_doc()?;
        }
        Ok(())
    }

    /// Returns one past the end of the current run of consecutive matching doc
    /// IDs.
    ///
    /// The returned value is greater than [`Self::doc_id`], and every doc ID in
    /// `[doc_id(), doc_id_run_end())` matches the iterator.
    fn doc_id_run_end(&self) -> Result<i32> {
        debug_assert!(self.doc_id() != NO_MORE_DOCS);
        Ok(self.doc_id() + 1)
    }
}

/// An empty [`DocIdSetIterator`] instance.
///
/// Equivalent to `DocIdSetIterator.empty()`.
#[derive(Debug, Clone)]
pub struct EmptyDocIdSetIterator {
    range: RangeDocIdSetIterator,
}

impl Default for EmptyDocIdSetIterator {
    fn default() -> Self {
        Self::new()
    }
}

impl EmptyDocIdSetIterator {
    /// Creates an empty iterator.
    pub fn new() -> Self {
        Self {
            range: RangeDocIdSetIterator::new(0, 0),
        }
    }
}

impl DocIdSetIterator for EmptyDocIdSetIterator {
    fn doc_id(&self) -> i32 {
        self.range.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.range.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.range.advance(target)
    }

    fn cost(&self) -> i64 {
        self.range.cost()
    }

    fn doc_id_run_end(&self) -> Result<i32> {
        self.range.doc_id_run_end()
    }
}

/// A [`DocIdSetIterator`] that matches all documents in `[0, max_doc)`.
///
/// Equivalent to `DocIdSetIterator.all(int)`.
#[derive(Debug, Clone)]
pub struct AllDocIdSetIterator {
    range: RangeDocIdSetIterator,
}

impl AllDocIdSetIterator {
    /// Creates an iterator matching all docs in `[0, max_doc)`.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` if `max_doc` is negative.
    pub fn new(max_doc: i32) -> Result<Self> {
        if max_doc < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "maxDoc must be >= 0, but got maxDoc={max_doc}"
            )));
        }
        Ok(Self {
            range: RangeDocIdSetIterator::new(0, max_doc),
        })
    }
}

impl DocIdSetIterator for AllDocIdSetIterator {
    fn doc_id(&self) -> i32 {
        self.range.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.range.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.range.advance(target)
    }

    fn cost(&self) -> i64 {
        self.range.cost()
    }

    fn doc_id_run_end(&self) -> Result<i32> {
        self.range.doc_id_run_end()
    }
}

/// A [`DocIdSetIterator`] that matches documents in `[min_doc, max_doc)`.
///
/// Equivalent to `DocIdSetIterator.range(int, int)`.
#[derive(Debug, Clone)]
pub struct RangeDocIdSetIterator {
    min_doc: i32,
    max_doc: i32,
    doc: i32,
}

impl RangeDocIdSetIterator {
    fn new(min_doc: i32, max_doc: i32) -> Self {
        debug_assert!(min_doc <= max_doc);
        Self {
            min_doc,
            max_doc,
            doc: -1,
        }
    }

    /// Creates an iterator over `[min_doc, max_doc)`.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` if `min_doc` is negative or
    /// `min_doc >= max_doc`.
    pub fn range(min_doc: i32, max_doc: i32) -> Result<Self> {
        if min_doc >= max_doc {
            return Err(LuceneError::IllegalArgument(format!(
                "minDoc must be < maxDoc but got minDoc={min_doc} maxDoc={max_doc}"
            )));
        }
        if min_doc < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "minDoc must be >= 0 but got minDoc={min_doc}"
            )));
        }
        Ok(Self::new(min_doc, max_doc))
    }
}

impl DocIdSetIterator for RangeDocIdSetIterator {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.advance(self.doc + 1)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        if target >= self.max_doc {
            self.doc = NO_MORE_DOCS;
        } else if target < self.min_doc {
            self.doc = self.min_doc;
        } else {
            self.doc = target;
        }
        Ok(self.doc)
    }

    fn cost(&self) -> i64 {
        (self.max_doc - self.min_doc) as i64
    }

    fn doc_id_run_end(&self) -> Result<i32> {
        debug_assert!(self.doc != NO_MORE_DOCS);
        Ok(self.max_doc)
    }
}

/// Returns an empty [`DocIdSetIterator`].
///
/// Convenience equivalent to `DocIdSetIterator.empty()`.
pub fn empty() -> EmptyDocIdSetIterator {
    EmptyDocIdSetIterator::new()
}

/// Returns a [`DocIdSetIterator`] matching all documents in `[0, max_doc)`.
///
/// Convenience equivalent to `DocIdSetIterator.all(int)`.
pub fn all(max_doc: i32) -> Result<AllDocIdSetIterator> {
    AllDocIdSetIterator::new(max_doc)
}

/// Returns a [`DocIdSetIterator`] matching documents in `[min_doc, max_doc)`.
///
/// Convenience equivalent to `DocIdSetIterator.range(int, int)`.
pub fn range(min_doc: i32, max_doc: i32) -> Result<RangeDocIdSetIterator> {
    RangeDocIdSetIterator::range(min_doc, max_doc)
}

/// Supplier of [`DocIdSetIterator`] instances.
///
/// Equivalent to Lucene's `IOSupplier<DocIdSetIterator>`.
pub trait DocIdSetIteratorSupplier {
    /// Returns a new iterator.
    fn get(&mut self) -> Result<Box<dyn DocIdSetIterator>>;
}

impl<F> DocIdSetIteratorSupplier for F
where
    F: FnMut() -> Result<Box<dyn DocIdSetIterator>>,
{
    fn get(&mut self) -> Result<Box<dyn DocIdSetIterator>> {
        (self)()
    }
}

/// Higher-level abstraction for document acceptance filtering.
///
/// Equivalent to `org.apache.lucene.search.AcceptDocs`.
pub trait AcceptDocs {
    /// Random access to accepted docs, or `None` if all documents are accepted.
    fn bits(&mut self) -> Result<Option<&dyn Bits>>;

    /// Sequential access to accepted docs.
    fn iterator(&mut self) -> Result<Box<dyn DocIdSetIterator>>;

    /// Approximate number of accepted docs.
    fn cost(&mut self) -> Result<i64>;
}

/// AcceptDocs backed by a [`Bits`] instance representing live documents.
///
/// Equivalent to `AcceptDocs.BitsAcceptDocs`.
#[derive(Clone)]
pub struct BitsAcceptDocs {
    bits: Option<Arc<dyn Bits>>,
    max_doc: i32,
}

impl std::fmt::Debug for BitsAcceptDocs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BitsAcceptDocs")
            .field("max_doc", &self.max_doc)
            .field("has_bits", &self.bits.is_some())
            .finish()
    }
}

impl BitsAcceptDocs {
    /// Creates [`AcceptDocs`] from a [`Bits`] instance.
    ///
    /// `bits == None` is interpreted as matching all documents, like
    /// `LeafReader.getLiveDocs()` returning `null`.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` if the provided bits have a
    /// length different from `max_doc`.
    pub fn from_live_docs(bits: Option<Box<dyn Bits>>, max_doc: i32) -> Result<Self> {
        if let Some(bits) = bits.as_ref() {
            if bits.length() != max_doc as usize {
                return Err(LuceneError::IllegalArgument(format!(
                    "Bits length = {} != maxDoc = {max_doc}",
                    bits.length()
                )));
            }
        }
        Ok(Self {
            bits: bits.map(Arc::from),
            max_doc,
        })
    }

    fn predicate(&self) -> Option<Box<dyn Fn(i32) -> bool>> {
        self.bits.as_ref().map(|bits| {
            let bits = Arc::clone(bits);
            Box::new(move |doc: i32| bits.get(doc as usize)) as Box<dyn Fn(i32) -> bool>
        })
    }
}

impl AcceptDocs for BitsAcceptDocs {
    fn bits(&mut self) -> Result<Option<&dyn Bits>> {
        Ok(self.bits.as_ref().map(|b| b.as_ref()))
    }

    fn iterator(&mut self) -> Result<Box<dyn DocIdSetIterator>> {
        let base = Box::new(AllDocIdSetIterator::new(self.max_doc)?);
        match self.predicate() {
            None => Ok(base),
            Some(predicate) => Ok(Box::new(FilteredDocIdSetIterator::new(base, predicate))),
        }
    }

    fn cost(&mut self) -> Result<i64> {
        // Java estimates maxDoc for the general Bits path. We use the same
        // conservative estimate regardless of whether the bits are dense or
        // sparse.
        Ok(self.max_doc as i64)
    }
}

/// AcceptDocs backed by a [`DocIdSetIterator`] supplier, optionally filtered
/// by live documents.
///
/// Equivalent to `AcceptDocs.DocIdSetIteratorAcceptDocs`.
pub struct IteratorAcceptDocs<S: DocIdSetIteratorSupplier> {
    supplier: S,
    live_docs: Option<Arc<dyn Bits>>,
    max_doc: i32,
    cached_bits: Option<FixedBitSet>,
}

impl<S: DocIdSetIteratorSupplier + std::fmt::Debug> std::fmt::Debug for IteratorAcceptDocs<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IteratorAcceptDocs")
            .field("supplier", &self.supplier)
            .field("max_doc", &self.max_doc)
            .field("has_live_docs", &self.live_docs.is_some())
            .field("cached_bits", &self.cached_bits.as_ref().map(|_| "..."))
            .finish()
    }
}

impl<S: DocIdSetIteratorSupplier> IteratorAcceptDocs<S> {
    /// Creates [`AcceptDocs`] from an iterator supplier.
    ///
    /// `live_docs == None` means there are no deleted documents.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` if the provided live docs have a
    /// length different from `max_doc`.
    pub fn from_iterator_supplier(
        supplier: S,
        live_docs: Option<Box<dyn Bits>>,
        max_doc: i32,
    ) -> Result<Self> {
        if let Some(bits) = live_docs.as_ref() {
            if bits.length() != max_doc as usize {
                return Err(LuceneError::IllegalArgument(format!(
                    "Bits length = {} != maxDoc = {max_doc}",
                    bits.length()
                )));
            }
        }
        Ok(Self {
            supplier,
            live_docs: live_docs.map(Arc::from),
            max_doc,
            cached_bits: None,
        })
    }

    fn filtered_iterator(&mut self) -> Result<Box<dyn DocIdSetIterator>> {
        let base = self.supplier.get()?;
        match self.live_docs.as_ref() {
            None => Ok(base),
            Some(bits) => {
                let bits = Arc::clone(bits);
                let predicate = Box::new(move |doc: i32| bits.get(doc as usize));
                Ok(Box::new(FilteredDocIdSetIterator::new(base, predicate)))
            }
        }
    }

    fn ensure_cached_bits(&mut self) -> Result<()> {
        if self.cached_bits.is_none() {
            let mut bit_set = FixedBitSet::new(self.max_doc as usize);
            {
                let mut iter = self.filtered_iterator()?;
                iter.into_bit_set(self.max_doc, &mut bit_set, 0)?;
            }
            self.cached_bits = Some(bit_set);
        }
        Ok(())
    }
}

impl<S: DocIdSetIteratorSupplier> AcceptDocs for IteratorAcceptDocs<S> {
    fn bits(&mut self) -> Result<Option<&dyn Bits>> {
        self.ensure_cached_bits()?;
        Ok(self.cached_bits.as_ref().map(|b| b as &dyn Bits))
    }

    fn iterator(&mut self) -> Result<Box<dyn DocIdSetIterator>> {
        if let Some(bits) = self.cached_bits.as_ref() {
            let cardinality = bits.cardinality();
            Ok(Box::new(BitSetDocIdSetIterator::new(
                bits.clone(),
                cardinality as i64,
            )))
        } else {
            self.filtered_iterator()
        }
    }

    fn cost(&mut self) -> Result<i64> {
        self.ensure_cached_bits()?;
        let cardinality = self
            .cached_bits
            .as_ref()
            .expect("INVARIANT: ensure_cached_bits always populates cached_bits")
            .cardinality();
        Ok(cardinality as i64)
    }
}

/// Free function creating [`AcceptDocs`] from live docs.
///
/// Equivalent to `AcceptDocs.fromLiveDocs(Bits, int)`.
pub fn from_live_docs(bits: Option<Box<dyn Bits>>, max_doc: i32) -> Result<BitsAcceptDocs> {
    BitsAcceptDocs::from_live_docs(bits, max_doc)
}

/// Free function creating [`AcceptDocs`] from an iterator supplier.
///
/// Equivalent to `AcceptDocs.fromIteratorSupplier(IOSupplier<DocIdSetIterator>, Bits, int)`.
pub fn from_iterator_supplier<S: DocIdSetIteratorSupplier>(
    supplier: S,
    live_docs: Option<Box<dyn Bits>>,
    max_doc: i32,
) -> Result<IteratorAcceptDocs<S>> {
    IteratorAcceptDocs::from_iterator_supplier(supplier, live_docs, max_doc)
}

// -----------------------------------------------------------------------------
// Internal helpers
// -----------------------------------------------------------------------------

/// Iterator that wraps another iterator and only returns doc IDs satisfying a
/// predicate.
struct FilteredDocIdSetIterator {
    inner: Box<dyn DocIdSetIterator>,
    predicate: Box<dyn Fn(i32) -> bool>,
}

impl FilteredDocIdSetIterator {
    fn new(inner: Box<dyn DocIdSetIterator>, predicate: Box<dyn Fn(i32) -> bool>) -> Self {
        Self { inner, predicate }
    }

    fn do_next(&mut self, mut doc: i32) -> Result<i32> {
        loop {
            if doc == NO_MORE_DOCS {
                return Ok(NO_MORE_DOCS);
            }
            if (self.predicate)(doc) {
                return Ok(doc);
            }
            doc = self.inner.next_doc()?;
        }
    }
}

impl DocIdSetIterator for FilteredDocIdSetIterator {
    fn doc_id(&self) -> i32 {
        self.inner.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        let doc = self.inner.next_doc()?;
        self.do_next(doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let doc = self.inner.advance(target)?;
        self.do_next(doc)
    }

    fn cost(&self) -> i64 {
        self.inner.cost()
    }
}

/// Iterator over the set bits of a [`FixedBitSet`].
struct BitSetDocIdSetIterator {
    bits: FixedBitSet,
    doc: i32,
    cost: i64,
}

impl BitSetDocIdSetIterator {
    fn new(bits: FixedBitSet, cost: i64) -> Self {
        Self {
            bits,
            doc: -1,
            cost,
        }
    }
}

impl DocIdSetIterator for BitSetDocIdSetIterator {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        let next = next_set_bit(&self.bits, self.doc + 1);
        self.doc = next.unwrap_or(NO_MORE_DOCS);
        Ok(self.doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let next = next_set_bit(&self.bits, target);
        self.doc = next.unwrap_or(NO_MORE_DOCS);
        Ok(self.doc)
    }

    fn cost(&self) -> i64 {
        self.cost
    }
}

/// Returns the index of the first set bit at or after `from_index`, or `None`
/// if there is no such bit.
fn next_set_bit(bits: &FixedBitSet, from_index: i32) -> Option<i32> {
    let len = bits.length() as i32;
    if from_index >= len {
        return None;
    }
    let from_index = from_index.max(0) as usize;
    let words = bits.get_bits();
    let num_words = words.len();
    let mut word_index = from_index >> 6;
    if word_index >= num_words {
        return None;
    }
    let shift = from_index & 0x3f;
    let mut word = words[word_index] & (!0u64 << shift);
    while word == 0 {
        word_index += 1;
        if word_index >= num_words {
            return None;
        }
        word = words[word_index];
    }
    let bit_index = word.trailing_zeros() as usize;
    let candidate = (word_index << 6) + bit_index;
    if candidate >= bits.length() {
        return None;
    }
    Some(candidate as i32)
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Sparse iterator backed by a sorted list of doc IDs.
    struct VecDocIdSetIterator {
        docs: Vec<i32>,
        idx: i32,
    }

    impl VecDocIdSetIterator {
        fn new(docs: Vec<i32>) -> Self {
            Self { docs, idx: -1 }
        }
    }

    impl DocIdSetIterator for VecDocIdSetIterator {
        fn doc_id(&self) -> i32 {
            if self.idx < 0 {
                -1
            } else if self.idx as usize >= self.docs.len() {
                NO_MORE_DOCS
            } else {
                self.docs[self.idx as usize]
            }
        }

        fn next_doc(&mut self) -> Result<i32> {
            self.idx += 1;
            if self.idx as usize >= self.docs.len() {
                Ok(NO_MORE_DOCS)
            } else {
                Ok(self.docs[self.idx as usize])
            }
        }

        fn advance(&mut self, target: i32) -> Result<i32> {
            let start = (self.idx + 1).max(0) as usize;
            match self.docs[start..].iter().position(|&d| d >= target) {
                Some(p) => {
                    self.idx = (start + p) as i32;
                    Ok(self.docs[start + p])
                }
                None => {
                    self.idx = self.docs.len() as i32;
                    Ok(NO_MORE_DOCS)
                }
            }
        }

        fn cost(&self) -> i64 {
            self.docs.len() as i64
        }
    }

    /// Bits implementation accepting only even doc IDs.
    struct EvenBits {
        max_doc: i32,
    }

    impl Bits for EvenBits {
        fn get(&self, index: usize) -> bool {
            index % 2 == 0
        }

        fn length(&self) -> usize {
            self.max_doc as usize
        }
    }

    #[test]
    fn empty_iterator_is_immediately_exhausted() {
        let mut it = empty();
        assert_eq!(it.doc_id(), -1);
        assert_eq!(it.next_doc().unwrap(), NO_MORE_DOCS);
        assert_eq!(it.doc_id(), NO_MORE_DOCS);
        assert_eq!(it.cost(), 0);
    }

    #[test]
    fn all_iterator_visits_every_doc() {
        let mut it = all(5).unwrap();
        assert_eq!(it.doc_id(), -1);
        for expected in 0..5 {
            assert_eq!(it.next_doc().unwrap(), expected);
        }
        assert_eq!(it.next_doc().unwrap(), NO_MORE_DOCS);
        assert_eq!(it.cost(), 5);
    }

    #[test]
    fn range_iterator_visits_subrange() {
        let mut it = range(2, 7).unwrap();
        for expected in 2..7 {
            assert_eq!(it.next_doc().unwrap(), expected);
        }
        assert_eq!(it.next_doc().unwrap(), NO_MORE_DOCS);
        assert_eq!(it.cost(), 5);
    }

    #[test]
    fn range_advance_matches_java_semantics() {
        let mut it = range(2, 10).unwrap();
        assert_eq!(it.advance(5).unwrap(), 5);
        assert_eq!(it.advance(6).unwrap(), 6);
        assert_eq!(it.advance(9).unwrap(), 9);
        assert_eq!(it.advance(10).unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn advance_before_min_positions_at_min() {
        let mut it = range(3, 8).unwrap();
        assert_eq!(it.advance(0).unwrap(), 3);
    }

    #[test]
    fn sparse_advance_jumps_to_target() {
        let mut it = VecDocIdSetIterator::new(vec![1, 5, 10, 20]);
        assert_eq!(it.advance(6).unwrap(), 10);
        assert_eq!(it.advance(20).unwrap(), 20);
        assert_eq!(it.advance(21).unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn slow_advance_default_matches_next_doc_loop() {
        let mut it = VecDocIdSetIterator::new(vec![2, 5, 8]);
        assert_eq!(it.next_doc().unwrap(), 2);
        assert_eq!(it.slow_advance(6).unwrap(), 8);
    }

    #[test]
    fn into_bit_set_loads_matching_docs() {
        let mut it = VecDocIdSetIterator::new(vec![0, 3, 4, 7]);
        let mut bit_set = FixedBitSet::new(10);
        it.into_bit_set(10, &mut bit_set, 0).unwrap();
        assert!(bit_set.get(0));
        assert!(!bit_set.get(1));
        assert!(!bit_set.get(2));
        assert!(bit_set.get(3));
        assert!(bit_set.get(4));
        assert!(!bit_set.get(5));
        assert!(!bit_set.get(6));
        assert!(bit_set.get(7));
        assert!(!bit_set.get(8));
        assert!(!bit_set.get(9));
    }

    #[test]
    fn doc_id_run_end_for_range_is_max_doc() {
        let mut it = range(2, 10).unwrap();
        it.next_doc().unwrap();
        assert_eq!(it.doc_id_run_end().unwrap(), 10);
    }

    #[test]
    fn range_constructor_rejects_invalid_arguments() {
        assert!(all(-1).is_err());
        assert!(range(5, 5).is_err());
        assert!(range(-1, 5).is_err());
    }

    #[test]
    fn accept_docs_match_all_iterates_everything() {
        let mut accept = from_live_docs(None, 5).unwrap();
        assert!(accept.bits().unwrap().is_none());
        let mut it = accept.iterator().unwrap();
        for d in 0..5 {
            assert_eq!(it.next_doc().unwrap(), d);
        }
        assert_eq!(it.next_doc().unwrap(), NO_MORE_DOCS);
        assert_eq!(accept.cost().unwrap(), 5);
    }

    #[test]
    fn accept_docs_from_bits_filters_iterator() {
        let bits: Box<dyn Bits> = Box::new(EvenBits { max_doc: 8 });
        let mut accept = from_live_docs(Some(bits), 8).unwrap();
        let bits_ref = accept.bits().unwrap().unwrap();
        assert_eq!(bits_ref.length(), 8);
        let mut it = accept.iterator().unwrap();
        for expected in [0, 2, 4, 6] {
            assert_eq!(it.next_doc().unwrap(), expected);
        }
        assert_eq!(it.next_doc().unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn accept_docs_from_iterator_supplier_builds_bits_and_cost() {
        let supplier = || -> Result<Box<dyn DocIdSetIterator>> {
            Ok(Box::new(VecDocIdSetIterator::new(vec![1, 3, 5, 7])))
        };
        let mut accept = from_iterator_supplier(supplier, None, 10).unwrap();
        assert_eq!(accept.cost().unwrap(), 4);

        let bits = accept.bits().unwrap().unwrap();
        assert!(bits.get(1));
        assert!(bits.get(3));
        assert!(bits.get(5));
        assert!(bits.get(7));
        assert!(!bits.get(0));
        assert!(!bits.get(2));

        let mut it = accept.iterator().unwrap();
        for expected in [1, 3, 5, 7] {
            assert_eq!(it.next_doc().unwrap(), expected);
        }
        assert_eq!(it.next_doc().unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn accept_docs_from_iterator_with_live_docs_combines_filters() {
        let supplier = || -> Result<Box<dyn DocIdSetIterator>> {
            Ok(Box::new(VecDocIdSetIterator::new(vec![0, 2, 4, 6, 8])))
        };
        let live: Box<dyn Bits> = Box::new(EvenBits { max_doc: 10 });
        let mut accept = from_iterator_supplier(supplier, Some(live), 10).unwrap();
        let mut it = accept.iterator().unwrap();
        for expected in [0, 2, 4, 6, 8] {
            assert_eq!(it.next_doc().unwrap(), expected);
        }
        assert_eq!(it.next_doc().unwrap(), NO_MORE_DOCS);

        let bits = accept.bits().unwrap().unwrap();
        assert!(bits.get(0));
        assert!(!bits.get(1));
        assert!(bits.get(2));
        assert!(bits.get(4));
        assert!(bits.get(6));
        assert!(bits.get(8));
        assert_eq!(accept.cost().unwrap(), 5);
    }

    #[test]
    fn bits_accept_docs_rejects_length_mismatch() {
        let bits: Box<dyn Bits> = Box::new(EvenBits { max_doc: 5 });
        assert!(from_live_docs(Some(bits), 8).is_err());
    }

    #[test]
    fn iterator_accept_docs_rejects_length_mismatch() {
        let supplier = || -> Result<Box<dyn DocIdSetIterator>> {
            Ok(Box::new(VecDocIdSetIterator::new(vec![])))
        };
        let live: Box<dyn Bits> = Box::new(EvenBits { max_doc: 5 });
        assert!(from_iterator_supplier(supplier, Some(live), 8).is_err());
    }
}
