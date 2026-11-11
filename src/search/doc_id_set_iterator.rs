//! Base doc-id iteration abstractions ported from `org.apache.lucene.search`.
//!
//! This module provides [`DocIdSetIterator`], the fundamental iterator over a
//! non-decreasing sequence of document identifiers, and [`AcceptDocs`], the
//! higher-level filtering abstraction used by codec producers to respect live
//! documents.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::error::{LuceneError, LuceneErrorKind, Result};
use crate::util::{bit_set_of, BitSet, BitSetIterator, Bits, FixedBitSet};

/// Sentinel returned by [`DocIdSetIterator`] methods when no more documents
/// are available.
///
/// Equivalent to `DocIdSetIterator.NO_MORE_DOCS` in Java Lucene.
pub const NO_MORE_DOCS: i32 = i32::MAX;

/// Port of `org.apache.lucene.search.DocIdSetIterator`.
///
/// Iterator over a non-decreasing set of doc IDs.
///
/// The contract mirrors `org.apache.lucene.search.DocIdSetIterator`:
/// - On construction the iterator is unpositioned and [`doc_id`](Self::doc_id)
///   returns `-1`.
/// - [`next_doc`](Self::next_doc) and [`advance`](Self::advance) move forward
///   and return the new current doc ID or [`NO_MORE_DOCS`].
/// - Once exhausted, the iterator must not be used again.
pub trait DocIdSetIterator {
    /// Whether this iterator is the view of a two-phase iterator that
    /// `TwoPhaseIterator.asDocIdSetIterator` returns, which is when
    /// `TwoPhaseIterator.unwrap(DocIdSetIterator)` (`TwoPhaseIterator.java`)
    /// returns non-null.
    ///
    /// Java tests the iterator's class with `instanceof`; the wrapper declares
    /// itself here. The default answers `false`.
    fn wraps_two_phase_iterator(&self) -> bool {
        false
    }

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
        debug_assert!(offset <= doc, "offset={offset} docID()={doc} upTo={up_to}");
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
    fn doc_id_run_end(&mut self) -> Result<i32> {
        debug_assert!(self.doc_id() != NO_MORE_DOCS);
        Ok(self.doc_id() + 1)
    }

    /// Returns this iterator as [`std::any::Any`], so that a caller can test
    /// it for a concrete type and reach that type's members.
    ///
    /// This hook stands for Java's `instanceof` and cast on a
    /// `DocIdSetIterator`, which Lucene uses to recognise a bit-set-backed
    /// iterator: `BitSetIterator.getBitSet(DocIdSetIterator, Class)`
    /// (`BitSetIterator.java:30-40`), `FixedBitSet.andNot(DocIdSetIterator)`
    /// (`FixedBitSet.java:670`), `AcceptDocs.createBitSet`
    /// (`AcceptDocs.java:95`) and `ConjunctionDISI.createConjunction`
    /// (`ConjunctionDISI.java:116`). Rust cannot narrow a
    /// `dyn DocIdSetIterator`, so the question is asked of the iterator itself,
    /// in the shape of
    /// [`PostingsEnum::as_any_mut`](crate::index::PostingsEnum::as_any_mut).
    /// The name differs from that hook's so that a `PostingsEnum`, which is
    /// also a `DocIdSetIterator`, has no two methods of one name.
    ///
    /// The default answers `None`, the `false` branch of every `instanceof`;
    /// only the types some Lucene `instanceof` tests for override it — and a
    /// wrapper Java does not have forwards it.
    fn disi_as_any(&self) -> Option<&dyn std::any::Any> {
        None
    }

    /// The mutable form of [`disi_as_any`](Self::disi_as_any), for the
    /// callers that move a narrowed iterator: `BitSetConjunctionDISI` sets the
    /// current doc of the `BitSetIterator`s it narrowed
    /// (`ConjunctionDISI.java:280-282`).
    fn disi_as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        None
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

    fn doc_id_run_end(&mut self) -> Result<i32> {
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
    /// Returns `LuceneErrorKind::IllegalArgument` if `max_doc` is negative.
    pub fn new(max_doc: i32) -> Result<Self> {
        if max_doc < 0 {
            return Err(LuceneError::illegal_argument(format!(
                "maxDoc must be >= 0, but got maxDoc={max_doc}"
            )));
        }
        Ok(Self {
            range: RangeDocIdSetIterator::new(0, max_doc),
        })
    }
}

impl DocIdSetIterator for AllDocIdSetIterator {
    /// Java tests the class of this iterator (`TestPointQueries.java:2219`).
    fn disi_as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

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

    fn doc_id_run_end(&mut self) -> Result<i32> {
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
    /// Returns `LuceneErrorKind::IllegalArgument` if `min_doc` is negative or
    /// `min_doc >= max_doc`.
    pub fn range(min_doc: i32, max_doc: i32) -> Result<Self> {
        if min_doc >= max_doc {
            return Err(LuceneError::illegal_argument(format!(
                "minDoc must be < maxDoc but got minDoc={min_doc} maxDoc={max_doc}"
            )));
        }
        if min_doc < 0 {
            return Err(LuceneError::illegal_argument(format!(
                "minDoc must be >= 0 but got minDoc={min_doc}"
            )));
        }
        Ok(Self::new(min_doc, max_doc))
    }
}

impl DocIdSetIterator for RangeDocIdSetIterator {
    /// Java tests the class of this iterator (`TestPointQueries.java:2219`).
    fn disi_as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

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

    fn doc_id_run_end(&mut self) -> Result<i32> {
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
    /// Returns a new iterator, or `None` for Java's `null`, which
    /// `AcceptDocs` rejects with a `NullPointerException`
    /// (`AcceptDocs.java:201`).
    fn get(&mut self) -> Result<Option<Box<dyn DocIdSetIterator>>>;
}

impl<F> DocIdSetIteratorSupplier for F
where
    F: FnMut() -> Result<Box<dyn DocIdSetIterator>>,
{
    fn get(&mut self) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        (self)().map(Some)
    }
}

/// `Objects.requireNonNull(value)`: a `NullPointerException` with a `null`
/// message for `None`.
fn require_non_null<T>(value: Option<T>) -> Result<T> {
    value.ok_or(LuceneError::from(LuceneErrorKind::Unchecked {
        class: "java.lang.NullPointerException",
        message: None,
        cause: None,
    }))
}

/// Higher-level abstraction for document acceptance filtering.
///
/// Equivalent to `org.apache.lucene.search.AcceptDocs`.
pub trait AcceptDocs {
    /// Random access to accepted docs, or `None` if all documents are accepted.
    ///
    /// The bits are shared, as Java's `AcceptDocs.bits()` hands out a reference
    /// that callers may keep and wrap.
    fn bits(&mut self) -> Result<Option<Arc<dyn Bits>>>;

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
    /// Returns `LuceneErrorKind::IllegalArgument` if the provided bits have a
    /// length different from `max_doc`.
    pub fn from_live_docs(bits: Option<Box<dyn Bits>>, max_doc: i32) -> Result<Self> {
        if let Some(bits) = bits.as_ref() {
            if bits.length() != max_doc as usize {
                return Err(LuceneError::illegal_argument(format!(
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
    fn bits(&mut self) -> Result<Option<Arc<dyn Bits>>> {
        Ok(self.bits.as_ref().map(Arc::clone))
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
    /// The lazily built accept bit set; `None` until `bits()` or `cost()` is
    /// first called. Java's `acceptBitSet`.
    accept_bit_set: Option<Arc<dyn BitSet>>,
    /// The cardinality of `accept_bit_set` once built. Java's `cardinality`.
    cardinality: usize,
}

impl<S: DocIdSetIteratorSupplier + std::fmt::Debug> std::fmt::Debug for IteratorAcceptDocs<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IteratorAcceptDocs")
            .field("supplier", &self.supplier)
            .field("max_doc", &self.max_doc)
            .field("has_live_docs", &self.live_docs.is_some())
            .field("has_accept_bit_set", &self.accept_bit_set.is_some())
            .field("cardinality", &self.cardinality)
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
    /// Returns the `NullPointerException` of `Objects.requireNonNull` when
    /// `supplier` is `None` (`AcceptDocs.java:172`).
    pub fn from_iterator_supplier(
        supplier: Option<S>,
        live_docs: Option<Box<dyn Bits>>,
        max_doc: i32,
    ) -> Result<Self> {
        // `Objects.requireNonNull(iteratorSupplier)` (`AcceptDocs.java:172`).
        let supplier = require_non_null(supplier)?;
        // Java checks no live-docs length here (`AcceptDocs.java:170-175`);
        // only `BitsAcceptDocs` compares it with `maxDoc`.
        Ok(Self {
            supplier,
            live_docs: live_docs.map(Arc::from),
            max_doc,
            accept_bit_set: None,
            cardinality: 0,
        })
    }

    fn filtered_iterator(&mut self) -> Result<Box<dyn DocIdSetIterator>> {
        // `Objects.requireNonNull(iteratorSupplier.get())`
        // (`AcceptDocs.java:201`).
        let base = require_non_null(self.supplier.get()?)?;
        Ok(get_filtered_doc_id_set_iterator(
            base,
            self.live_docs.as_ref().map(Arc::clone),
        ))
    }

    /// Equivalent to `DocIdSetIteratorAcceptDocs.createBitSetAcceptDocsIfNecessary()`.
    fn ensure_cached_bits(&mut self) -> Result<()> {
        if self.accept_bit_set.is_none() {
            // Java passes `iterator()`, which while the bit set is absent is
            // exactly the live-docs-filtered iterator built here.
            let iterator = self.filtered_iterator()?;
            let bit_set: Arc<dyn BitSet> = create_bit_set(
                iterator,
                self.live_docs.as_ref().map(Arc::clone),
                self.max_doc,
            )?;
            self.cardinality = bit_set.cardinality();
            self.accept_bit_set = Some(bit_set);
        }
        Ok(())
    }
}

impl<S: DocIdSetIteratorSupplier> AcceptDocs for IteratorAcceptDocs<S> {
    fn bits(&mut self) -> Result<Option<Arc<dyn Bits>>> {
        self.ensure_cached_bits()?;
        // Java returns the `BitSet` itself, which is a `Bits`.
        Ok(self
            .accept_bit_set
            .as_ref()
            .map(|bit_set| Arc::clone(bit_set) as Arc<dyn Bits>))
    }

    fn iterator(&mut self) -> Result<Box<dyn DocIdSetIterator>> {
        if let Some(bit_set) = self.accept_bit_set.as_ref() {
            Ok(Box::new(BitSetIterator::new(
                Arc::clone(bit_set),
                self.cardinality as i64,
            )?))
        } else {
            self.filtered_iterator()
        }
    }

    fn cost(&mut self) -> Result<i64> {
        self.ensure_cached_bits()?;
        Ok(self.cardinality as i64)
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
    supplier: Option<S>,
    live_docs: Option<Box<dyn Bits>>,
    max_doc: i32,
) -> Result<IteratorAcceptDocs<S>> {
    IteratorAcceptDocs::from_iterator_supplier(supplier, live_docs, max_doc)
}

/// Builds the bit set of the documents `iterator` accepts, honouring
/// `live_docs`.
///
/// Equivalent to the private static `AcceptDocs.createBitSet(DocIdSetIterator,
/// Bits, int)` (`AcceptDocs.java:93-114`): when there are no deletions and
/// the iterator is a [`BitSetIterator`](crate::util::BitSetIterator), its own
/// bit set is handed back; otherwise a dense [`FixedBitSet`] is filled through
/// [`BitSet::or`](crate::util::BitSet::or) when the iterator's cost reaches
/// `max_doc >> 7` — the same threshold as `BitSet.of` — and a sparse set is
/// built with [`bit_set_of`] otherwise.
///
/// # Errors
///
/// Propagates iteration errors, and rejects an already-positioned iterator.
fn create_bit_set(
    iterator: Box<dyn DocIdSetIterator>,
    live_docs: Option<Arc<dyn Bits>>,
    max_doc: i32,
) -> Result<Arc<dyn BitSet>> {
    if live_docs.is_none() {
        if let Some(bit_set_iterator) = iterator
            .disi_as_any()
            .and_then(|any| any.downcast_ref::<crate::util::BitSetIterator>())
        {
            // If we already have a BitSet and no deletions, reuse the BitSet.
            return Ok(Arc::clone(bit_set_iterator.get_bit_set()));
        }
    }
    let threshold = i64::from(max_doc >> 7); // same as BitSet#of
    if iterator.cost() >= threshold {
        // take advantage of DocIdSetIterator#into_bit_set and Bits#apply_mask
        let mut iterator = iterator;
        let mut bit_set = FixedBitSet::new(max_doc.max(0) as usize);
        BitSet::or(&mut bit_set, iterator.as_mut())?;
        if let Some(live_docs) = live_docs.as_ref() {
            live_docs.apply_mask(&mut bit_set, 0)?;
        }
        Ok(Arc::new(bit_set))
    } else {
        // create a sparse bitset
        let mut filtered = get_filtered_doc_id_set_iterator(iterator, live_docs);
        Ok(Arc::from(bit_set_of(filtered.as_mut(), max_doc)?))
    }
}

// -----------------------------------------------------------------------------
// Internal helpers
// -----------------------------------------------------------------------------

/// Port of `org.apache.lucene.search.FilteredDocIdSetIterator`.
///
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

    /// Returns the wrapped [`DocIdSetIterator`].
    ///
    /// Equivalent to `FilteredDocIdSetIterator.getDelegate()`.
    #[allow(dead_code)]
    fn get_delegate(&self) -> &dyn DocIdSetIterator {
        self.inner.as_ref()
    }

    /// Validation method determining whether `doc` should be in the result set.
    ///
    /// Equivalent to the `protected abstract`
    /// `FilteredDocIdSetIterator.match(int)`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java leaves the method abstract and
    /// expects a subclass to implement it. Rust has no inheritance, so this
    /// port takes the test as the `predicate` the iterator is built with and
    /// this method applies it, which is what every subclass body amounts to.
    fn matches(&self, doc: i32) -> bool {
        (self.predicate)(doc)
    }

    fn do_next(&mut self, mut doc: i32) -> Result<i32> {
        loop {
            if doc == NO_MORE_DOCS {
                return Ok(NO_MORE_DOCS);
            }
            if self.matches(doc) {
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

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// `AcceptDocs.createBitSet` builds a dense iterator into a `FixedBitSet`
    /// and applies the live docs with `liveDocs.applyMask(bitSet, 0)`
    /// (`AcceptDocs.java:101-105`).
    #[test]
    fn create_bit_set_applies_the_live_docs_mask() {
        #[derive(Debug)]
        struct Even;
        impl Bits for Even {
            fn get(&self, index: usize) -> bool {
                index.is_multiple_of(2)
            }
            fn length(&self) -> usize {
                1024
            }
        }
        let all: Vec<i32> = (0..1024).collect();
        let bit_set = create_bit_set(
            Box::new(VecDocIdSetIterator::new(all)),
            Some(Arc::new(Even)),
            1024,
        )
        .unwrap();
        assert_eq!(bit_set.cardinality(), 512);
        assert!(bit_set.get(2) && !bit_set.get(3));
    }

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
    #[derive(Debug)]
    struct EvenBits {
        max_doc: i32,
    }

    impl Bits for EvenBits {
        fn get(&self, index: usize) -> bool {
            index.is_multiple_of(2)
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
        let mut accept = from_iterator_supplier(Some(supplier), None, 10).unwrap();
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
        let mut accept = from_iterator_supplier(Some(supplier), Some(live), 10).unwrap();
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

    /// Java's `DocIdSetIteratorAcceptDocs` does not compare the live-docs
    /// length with `maxDoc` (`AcceptDocs.java:170-175`).
    #[test]
    fn iterator_accept_docs_accepts_length_mismatch() {
        let supplier = || -> Result<Box<dyn DocIdSetIterator>> {
            Ok(Box::new(VecDocIdSetIterator::new(vec![])))
        };
        let live: Box<dyn Bits> = Box::new(EvenBits { max_doc: 5 });
        assert!(from_iterator_supplier(Some(supplier), Some(live), 8).is_ok());
    }
}

/// Wraps `iterator` so that it only produces documents `live_docs` accepts, or
/// returns it unchanged when there are none.
///
/// Equivalent to the private static
/// `AcceptDocs.getFilteredDocIdSetIterator(DocIdSetIterator, Bits)`.
pub fn get_filtered_doc_id_set_iterator(
    iterator: Box<dyn DocIdSetIterator>,
    live_docs: Option<Arc<dyn Bits>>,
) -> Box<dyn DocIdSetIterator> {
    match live_docs {
        None => iterator,
        Some(bits) => {
            let predicate = Box::new(move |doc: i32| bits.get(doc as usize));
            Box::new(FilteredDocIdSetIterator::new(iterator, predicate))
        }
    }
}

#[cfg(test)]
mod accept_docs_reuse_tests {
    use super::*;
    use crate::util::{BitSetIterator, FixedBitSet};

    /// rmp #297: with no deletions, `AcceptDocs.createBitSet` hands back the
    /// bit set of a `BitSetIterator` instead of building one
    /// (`AcceptDocs.java:95-97`).
    #[test]
    fn a_bit_set_iterator_lends_its_bit_set() {
        let mut set = FixedBitSet::new(100);
        set.set(3);
        set.set(64);
        let set: Arc<dyn BitSet> = Arc::new(set);
        let shared = Arc::clone(&set);
        let supplier = move || -> Result<Box<dyn DocIdSetIterator>> {
            Ok(Box::new(BitSetIterator::new(Arc::clone(&shared), 2)?))
        };
        let mut accept = from_iterator_supplier(Some(supplier), None, 100).unwrap();
        let bits = accept.bits().unwrap().unwrap();
        assert_eq!(
            Arc::as_ptr(&bits) as *const (),
            Arc::as_ptr(&set) as *const ()
        );
        assert_eq!(accept.cost().unwrap(), 2);
    }
}
