//! `DocIdSet` implementations ported from `org.apache.lucene.util`.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`BitDocIdSet`] | `BitDocIdSet` |
//! | [`IntArrayDocIdSet`] | `IntArrayDocIdSet` |
//! | [`NotDocIdSet`] | `NotDocIdSet` |
//! | [`DocIdSetBuilder`] | `DocIdSetBuilder` |
//!
//! The base abstraction lives in [`crate::search::doc_id_set`], mirroring
//! Lucene's split between `org.apache.lucene.search.DocIdSet` and these
//! implementations in `org.apache.lucene.util`.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::point_values::PointValues;
use crate::index::terms::Terms;
use crate::search::doc_id_set::DocIdSet;
use crate::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use crate::util::bit_set::{BitSet, BitSetIterator};
use crate::util::packed::PackedInts;
use crate::util::sorter::LSBRadixSorter;
use crate::util::string_helper::IntsRef;
use crate::util::{Accountable, Bits, FixedBitSet, RamUsageEstimator};

/// Shallow size of a small wrapper object, standing in for Java's
/// `RamUsageEstimator.shallowSizeOfInstance(SomeClass.class)`.
///
/// Rust has no object header to measure; this reproduces the JVM shape the rest
/// of [`RamUsageEstimator`] already assumes — an object header plus one
/// reference field and one `long` field, aligned.
fn base_ram_bytes_used() -> i64 {
    RamUsageEstimator::align_object_size(
        RamUsageEstimator::NUM_BYTES_OBJECT_HEADER + RamUsageEstimator::NUM_BYTES_OBJECT_REF + 8,
    )
}

// ---------------------------------------------------------------------------
// BitDocIdSet
// ---------------------------------------------------------------------------

/// A [`DocIdSet`] on top of a [`BitSet`].
///
/// Port of `org.apache.lucene.util.BitDocIdSet`.
///
/// **Divergence from Lucene 10.5.0.** Java holds a bare reference to the bit
/// set and documents that it must not be modified afterwards; this port holds
/// an [`Arc`], which enforces that by construction and lets
/// [`DocIdSet::iterator`] hand out iterators without copying.
#[derive(Clone)]
pub struct BitDocIdSet {
    set: Arc<dyn BitSet>,
    cost: i64,
}

impl std::fmt::Debug for BitDocIdSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "BitDocIdSet(set={:?},cost={})", self.set, self.cost)
    }
}

impl BitDocIdSet {
    /// Wraps `set` as a [`DocIdSet`] with the given cost.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `cost` is negative.
    pub fn new(set: Arc<dyn BitSet>, cost: i64) -> Result<Self> {
        if cost < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "cost must be >= 0, got {cost}"
            )));
        }
        Ok(Self { set, cost })
    }

    /// Wraps `set`, using its approximate cardinality as the cost.
    ///
    /// Equivalent to `new BitDocIdSet(BitSet)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] only if the approximate
    /// cardinality is somehow negative, which it cannot be.
    pub fn from_set(set: Arc<dyn BitSet>) -> Result<Self> {
        let cost = set.approximate_cardinality() as i64;
        Self::new(set, cost)
    }

    /// Returns the wrapped bit set.
    ///
    /// Equivalent to the deprecated `BitDocIdSet.bits()`.
    pub fn set(&self) -> &Arc<dyn BitSet> {
        &self.set
    }
}

impl Accountable for BitDocIdSet {
    fn ram_bytes_used(&self) -> i64 {
        base_ram_bytes_used() + self.set.ram_bytes_used()
    }
}

impl DocIdSet for BitDocIdSet {
    fn iterator(&self) -> Result<Box<dyn DocIdSetIterator>> {
        Ok(Box::new(BitSetIterator::new(
            Arc::clone(&self.set),
            self.cost,
        )?))
    }
}

// ---------------------------------------------------------------------------
// IntArrayDocIdSet
// ---------------------------------------------------------------------------

/// A [`DocIdSet`] backed by a sorted `i32` array.
///
/// Port of `org.apache.lucene.util.IntArrayDocIdSet`, which is package-private
/// in Java (only [`DocIdSetBuilder`] constructs it) and public here because
/// Rust has no package visibility between sibling modules.
#[derive(Debug, Clone)]
pub struct IntArrayDocIdSet {
    docs: Arc<Vec<i32>>,
    length: usize,
}

impl IntArrayDocIdSet {
    /// Wraps the first `length` entries of `docs`.
    ///
    /// `docs[length]` must be [`NO_MORE_DOCS`], as Lucene requires, and
    /// `docs[..length]` must be sorted.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the sentinel is missing,
    /// which is Java's bare `IllegalArgumentException`.
    pub fn new(docs: Vec<i32>, length: usize) -> Result<Self> {
        if docs[length] != NO_MORE_DOCS {
            return Err(LuceneError::IllegalArgument(
                "IntArrayDocIdSet requires docs[length] == NO_MORE_DOCS".to_string(),
            ));
        }
        debug_assert!(
            docs[..length].windows(2).all(|w| w[0] <= w[1]),
            "IntArrayDocIdSet need docs to be sorted"
        );
        Ok(Self {
            docs: Arc::new(docs),
            length,
        })
    }

    /// Returns the number of documents in this set.
    pub fn len(&self) -> usize {
        self.length
    }

    /// Returns whether this set is empty.
    pub fn is_empty(&self) -> bool {
        self.length == 0
    }
}

impl Accountable for IntArrayDocIdSet {
    fn ram_bytes_used(&self) -> i64 {
        base_ram_bytes_used() + RamUsageEstimator::size_of_int(&self.docs)
    }
}

impl DocIdSet for IntArrayDocIdSet {
    fn iterator(&self) -> Result<Box<dyn DocIdSetIterator>> {
        Ok(Box::new(IntArrayDocIdSetIterator::new(
            Arc::clone(&self.docs),
            self.length,
        )))
    }
}

/// The iterator of [`IntArrayDocIdSet`].
///
/// Port of the nested `IntArrayDocIdSet.IntArrayDocIdSetIterator`.
#[derive(Debug, Clone)]
pub struct IntArrayDocIdSetIterator {
    docs: Arc<Vec<i32>>,
    length: usize,
    i: usize,
    doc: i32,
}

impl IntArrayDocIdSetIterator {
    /// Creates an iterator over the first `length` entries of `docs`.
    pub fn new(docs: Arc<Vec<i32>>, length: usize) -> Self {
        Self {
            docs,
            length,
            i: 0,
            doc: -1,
        }
    }
}

/// Returns the first index in `[from, to)` whose value is `>= target`, or `to`.
///
/// **Divergence from Lucene 10.5.0.** Java calls `VectorUtil.findNextGEQ`,
/// which Rucene's `VectorUtil` port does not expose; this is the same scan,
/// written out.
fn find_next_geq(buffer: &[i32], target: i32, from: usize, to: usize) -> usize {
    let mut i = from;
    while i < to {
        if buffer[i] >= target {
            return i;
        }
        i += 1;
    }
    to
}

impl DocIdSetIterator for IntArrayDocIdSetIterator {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.doc = self.docs[self.i];
        self.i += 1;
        Ok(self.doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let mut bound = 1usize;
        // Given that this is used for small arrays only, overflow is very
        // unlikely.
        while self.i + bound < self.length && self.docs[self.i + bound] < target {
            bound *= 2;
        }
        let lo = self.i + bound / 2;
        let hi = (self.i + bound + 1).min(self.length);
        self.i = match self.docs[lo..hi].binary_search(&target) {
            Ok(pos) => lo + pos,
            Err(pos) => lo + pos,
        };
        self.doc = self.docs[self.i];
        self.i += 1;
        Ok(self.doc)
    }

    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        if self.doc >= up_to {
            return Ok(());
        }

        // Java computes `i - 1` unconditionally, which is only in range once the
        // iterator has been positioned; the contract is the same here.
        debug_assert!(
            self.i > 0,
            "into_bit_set requires a positioned iterator, got i=0"
        );
        let from = self.i - 1;
        let to = find_next_geq(&self.docs, up_to, from, self.length);
        for i in from..to {
            bit_set.set((self.docs[i] - offset) as usize);
        }
        self.doc = self.docs[to];
        self.i = to + 1;
        Ok(())
    }

    fn cost(&self) -> i64 {
        self.length as i64
    }
}

// ---------------------------------------------------------------------------
// NotDocIdSet
// ---------------------------------------------------------------------------

/// Encodes the negation of another [`DocIdSet`].
///
/// Port of `org.apache.lucene.util.NotDocIdSet`. It is cacheable and supports
/// random access when the wrapped set does.
pub struct NotDocIdSet {
    max_doc: i32,
    inner: Arc<dyn DocIdSet>,
}

impl NotDocIdSet {
    /// Negates `inner` over `[0, max_doc)`.
    pub fn new(max_doc: i32, inner: Arc<dyn DocIdSet>) -> Self {
        Self { max_doc, inner }
    }
}

impl Accountable for NotDocIdSet {
    fn ram_bytes_used(&self) -> i64 {
        base_ram_bytes_used() + self.inner.ram_bytes_used()
    }
}

/// The `Bits` view of a [`NotDocIdSet`]: the negation of the wrapped view.
#[derive(Debug)]
struct NotBits {
    inner: Box<dyn Bits>,
}

impl Bits for NotBits {
    fn get(&self, index: usize) -> bool {
        !self.inner.get(index)
    }

    fn length(&self) -> usize {
        self.inner.length()
    }
}

impl DocIdSet for NotDocIdSet {
    fn bits(&self) -> Result<Option<Box<dyn Bits>>> {
        match self.inner.bits()? {
            None => Ok(None),
            Some(inner) => Ok(Some(Box::new(NotBits { inner }))),
        }
    }

    fn iterator(&self) -> Result<Box<dyn DocIdSetIterator>> {
        Ok(Box::new(NotDocIdSetIterator {
            inner: self.inner.iterator()?,
            max_doc: self.max_doc,
            next_skipped_doc: -1,
            doc: -1,
        }))
    }
}

/// The iterator of [`NotDocIdSet`], Lucene's anonymous
/// `AbstractDocIdSetIterator`.
struct NotDocIdSetIterator {
    inner: Box<dyn DocIdSetIterator>,
    max_doc: i32,
    next_skipped_doc: i32,
    doc: i32,
}

impl DocIdSetIterator for NotDocIdSetIterator {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.advance(self.doc + 1)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.doc = target;
        if self.doc > self.next_skipped_doc {
            self.next_skipped_doc = self.inner.advance(self.doc)?;
        }
        loop {
            if self.doc >= self.max_doc {
                self.doc = NO_MORE_DOCS;
                return Ok(self.doc);
            }
            debug_assert!(self.doc <= self.next_skipped_doc);
            if self.doc != self.next_skipped_doc {
                return Ok(self.doc);
            }
            self.doc += 1;
            self.next_skipped_doc = self.inner.next_doc()?;
        }
    }

    fn doc_id_run_end(&self) -> Result<i32> {
        debug_assert!(self.doc != NO_MORE_DOCS);
        debug_assert!(self.next_skipped_doc > self.doc);
        Ok(self.next_skipped_doc.min(self.max_doc))
    }

    fn cost(&self) -> i64 {
        // Even if there are few docs in this set, iterating over all documents
        // costs O(maxDoc) in all cases.
        self.max_doc as i64
    }
}

// ---------------------------------------------------------------------------
// DocIdSetBuilder
// ---------------------------------------------------------------------------

/// One of the sparse buffers a [`DocIdSetBuilder`] accumulates into.
#[derive(Debug)]
struct Buffer {
    array: Vec<i32>,
    length: usize,
}

impl Buffer {
    fn with_len(length: usize) -> Self {
        Self {
            array: vec![0; length],
            length: 0,
        }
    }

    fn from_parts(array: Vec<i32>, length: usize) -> Self {
        Self { array, length }
    }
}

/// A builder of [`DocIdSet`]s: sparse at first, upgrading to a bit set once
/// enough hits match.
///
/// Port of `org.apache.lucene.util.DocIdSetBuilder`.
///
/// **Divergence from Lucene 10.5.0.** Java's `grow(int)` returns a `BulkAdder`
/// object, a sealed interface with one implementation per storage mode, so that
/// the mode is resolved once per batch instead of once per document. Rust
/// cannot hand out such an object without borrowing the builder for the whole
/// batch, which would forbid the interleaved `grow(1).add(doc)` calls Java
/// itself makes, so [`DocIdSetBuilder::grow`] reserves the space and returns a
/// [`BulkAdder`] that borrows the builder for the duration of one batch. The
/// dispatch it performs is the same; only its lifetime is explicit.
pub struct DocIdSetBuilder {
    max_doc: i32,
    threshold: i64,
    /// Whether the source may produce several values per document.
    multivalued: bool,
    /// Average number of values per document, used to estimate the cost.
    num_values_per_doc: f64,

    buffers: Vec<Buffer>,
    /// Accumulated size of the allocated buffers.
    total_allocated: i64,

    bit_set: Option<FixedBitSet>,

    counter: i64,
}

impl DocIdSetBuilder {
    /// Creates a builder for doc ids in `[0, max_doc)`.
    pub fn new(max_doc: i32) -> Self {
        Self::with_stats(max_doc, -1, -1)
    }

    /// Creates a builder optimised for accumulating docs matching `terms`.
    ///
    /// Equivalent to `new DocIdSetBuilder(int, Terms)`.
    pub fn from_terms(max_doc: i32, terms: &dyn Terms) -> Self {
        Self::with_stats(max_doc, terms.doc_count(), terms.sum_doc_freq())
    }

    /// Creates a builder optimised for accumulating docs matching `values`.
    ///
    /// Equivalent to `new DocIdSetBuilder(int, PointValues)`.
    pub fn from_point_values(max_doc: i32, values: &dyn PointValues) -> Self {
        Self::with_stats(max_doc, values.doc_count(), values.size())
    }

    /// Creates a builder from raw index statistics.
    ///
    /// Equivalent to the package-private `DocIdSetBuilder(int, int, long)`.
    pub fn with_stats(max_doc: i32, doc_count: i32, value_count: i64) -> Self {
        let multivalued = doc_count < 0 || doc_count as i64 != value_count;
        let num_values_per_doc = if doc_count <= 0 || value_count < 0 {
            // Assume one value per doc; the cost is then overestimated when the
            // docs are in fact multi-valued.
            1.0
        } else {
            value_count as f64 / doc_count as f64
        };
        debug_assert!(num_values_per_doc >= 1.0);

        Self {
            max_doc,
            // `maxDoc >>> 7` is a good value if you want to save memory; lower
            // values such as `maxDoc >>> 11` build faster at the expense of a
            // full bit set even for quite sparse data.
            threshold: (max_doc as i64) >> 7,
            multivalued,
            num_values_per_doc,
            buffers: Vec::new(),
            total_allocated: 0,
            bit_set: None,
            counter: -1,
        }
    }

    /// Whether the source may produce several values per document.
    pub fn multivalued(&self) -> bool {
        self.multivalued
    }

    /// Average number of values per document.
    pub fn num_values_per_doc(&self) -> f64 {
        self.num_values_per_doc
    }

    /// Adds the content of `iter` to this builder.
    ///
    /// When a [`DocIdSet`] must be built out of a single iterator, prefer
    /// `RoaringDocIdSet::builder` instead, exactly as Lucene advises.
    ///
    /// # Errors
    ///
    /// Propagates iteration errors.
    pub fn add(&mut self, iter: &mut dyn DocIdSetIterator) -> Result<()> {
        let cost = iter.cost().min(i32::MAX as i64) as i32;
        self.grow(cost);
        if let Some(bit_set) = self.bit_set.as_mut() {
            return BitSet::or(bit_set, iter);
        }
        for _ in 0..cost {
            let doc = iter.next_doc()?;
            if doc == NO_MORE_DOCS {
                return Ok(());
            }
            self.adder_add(doc);
        }
        let mut doc = iter.next_doc()?;
        while doc != NO_MORE_DOCS {
            self.grow(1);
            self.adder_add(doc);
            doc = iter.next_doc()?;
        }
        Ok(())
    }

    /// Reserves space for up to `num_docs` documents and returns the adder that
    /// writes them.
    ///
    /// Equivalent to `DocIdSetBuilder.grow(int)`.
    pub fn grow(&mut self, num_docs: i32) -> BulkAdder<'_> {
        let num_docs = num_docs.max(0) as i64;
        if self.bit_set.is_none() {
            if self.total_allocated + num_docs <= self.threshold {
                self.ensure_buffer_capacity(num_docs);
            } else {
                self.upgrade_to_bit_set();
                self.counter += num_docs;
            }
        } else {
            self.counter += num_docs;
        }
        BulkAdder { builder: self }
    }

    /// Appends one document to whichever storage is currently active.
    ///
    /// This is the body of Java's two `BulkAdder.add(int)` implementations.
    fn adder_add(&mut self, doc: i32) {
        match self.bit_set.as_mut() {
            Some(bit_set) => bit_set.set(doc as usize),
            None => {
                let buffer = self
                    .buffers
                    .last_mut()
                    .expect("INVARIANT: grow() allocates a buffer before any add");
                let len = buffer.length;
                buffer.array[len] = doc;
                buffer.length += 1;
            }
        }
    }

    /// Equivalent to `DocIdSetBuilder.ensureBufferCapacity`.
    fn ensure_buffer_capacity(&mut self, num_docs: i64) {
        if self.buffers.is_empty() {
            let cap = self.additional_capacity(num_docs);
            self.add_buffer(cap);
            return;
        }

        let (current_capacity, current_length) = {
            let current = self
                .buffers
                .last()
                .expect("INVARIANT: buffers is not empty here");
            (current.array.len(), current.length)
        };
        if (current_capacity - current_length) as i64 >= num_docs {
            // The current buffer is large enough.
            return;
        }
        if current_length < current_capacity - (current_capacity >> 3) {
            // The current buffer is less than 7/8 full: resize rather than
            // waste the space.
            let additional = self.additional_capacity(num_docs);
            self.grow_buffer(additional);
        } else {
            let cap = self.additional_capacity(num_docs);
            self.add_buffer(cap);
        }
    }

    /// Equivalent to `DocIdSetBuilder.additionalCapacity`.
    fn additional_capacity(&self, num_docs: i64) -> i64 {
        // Exponential growth: the new array is as large as everything allocated
        // so far ...
        let mut c = self.total_allocated;
        // ... but at least `num_docs + 1`, so that the next batch fits (plus an
        // empty slot, which makes the array more likely to be reused in build())
        c = c.max(num_docs + 1);
        // ... avoiding cold starts ...
        c = c.max(32);
        // ... and never going beyond the threshold.
        c.min(self.threshold - self.total_allocated)
    }

    /// Equivalent to `DocIdSetBuilder.addBuffer`.
    fn add_buffer(&mut self, len: i64) {
        let buffer = Buffer::with_len(len.max(0) as usize);
        self.total_allocated += buffer.array.len() as i64;
        self.buffers.push(buffer);
    }

    /// Equivalent to `DocIdSetBuilder.growBuffer`.
    fn grow_buffer(&mut self, additional_capacity: i64) {
        let buffer = self
            .buffers
            .last_mut()
            .expect("INVARIANT: buffers is not empty here");
        let new_len = buffer.array.len() + additional_capacity.max(0) as usize;
        buffer.array.resize(new_len, 0);
        self.total_allocated += additional_capacity;
    }

    /// Equivalent to `DocIdSetBuilder.upgradeToBitSet`.
    fn upgrade_to_bit_set(&mut self) {
        debug_assert!(self.bit_set.is_none());
        let mut bit_set = FixedBitSet::new(self.max_doc.max(0) as usize);
        let mut counter: i64 = 0;
        for buffer in &self.buffers {
            counter += buffer.length as i64;
            for &doc in &buffer.array[..buffer.length] {
                bit_set.set(doc as usize);
            }
        }
        self.bit_set = Some(bit_set);
        self.counter = counter;
        self.buffers = Vec::new();
    }

    /// Builds a [`DocIdSet`] from the accumulated doc ids.
    ///
    /// Java's `build()` releases the builder's storage but leaves the object
    /// alive; this port consumes the builder, which is the Rust expression of
    /// the same contract.
    ///
    /// # Errors
    ///
    /// Propagates the errors of the underlying set constructors.
    pub fn build(self) -> Result<Box<dyn DocIdSet>> {
        if let Some(bit_set) = self.bit_set {
            debug_assert!(self.counter >= 0);
            let cost = (self.counter as f64 / self.num_values_per_doc).round() as i64;
            let set: Arc<dyn BitSet> = Arc::new(bit_set);
            return Ok(Box::new(BitDocIdSet::new(set, cost)?));
        }

        let mut concatenated = concat(self.buffers);
        // Java passes `maxDoc - 1` straight through, so a `max_doc` of zero
        // reaches `PackedInts.bitsRequired(-1)` and raises
        // `IllegalArgumentException`; that behaviour is preserved.
        let bits = PackedInts::bits_required(self.max_doc as i64 - 1)?;
        let mut sorter = LSBRadixSorter::new();
        sorter.sort(bits as u32, &mut concatenated.array, concatenated.length);
        let l = if self.multivalued {
            dedup(&mut concatenated.array, concatenated.length)
        } else {
            debug_assert!(
                concatenated.array[..concatenated.length]
                    .windows(2)
                    .all(|w| w[0] < w[1]),
                "duplicate doc ids in a single-valued source"
            );
            concatenated.length
        };
        debug_assert!(l <= concatenated.length);
        concatenated.array[l] = NO_MORE_DOCS;
        Ok(Box::new(IntArrayDocIdSet::new(concatenated.array, l)?))
    }
}

/// Concatenates the buffers in any order, leaving at least one empty slot at
/// the end. Reuses the largest buffer's array.
///
/// Equivalent to the private static `DocIdSetBuilder.concat`.
fn concat(buffers: Vec<Buffer>) -> Buffer {
    let mut total_length = 0usize;
    let mut largest: Option<usize> = None;
    for (i, buffer) in buffers.iter().enumerate() {
        total_length += buffer.length;
        match largest {
            None => largest = Some(i),
            Some(l) if buffer.array.len() > buffers[l].array.len() => largest = Some(i),
            _ => {}
        }
    }
    let Some(largest) = largest else {
        return Buffer::with_len(1);
    };

    let mut buffers = buffers;
    let largest_buffer = buffers.swap_remove(largest);
    let mut docs = largest_buffer.array;
    if docs.len() < total_length + 1 {
        docs.resize(total_length + 1, 0);
    }
    let mut total_length = largest_buffer.length;
    for buffer in &buffers {
        docs[total_length..total_length + buffer.length]
            .copy_from_slice(&buffer.array[..buffer.length]);
        total_length += buffer.length;
    }
    Buffer::from_parts(docs, total_length)
}

/// Removes consecutive duplicates from `arr[..length]`, returning the new
/// length. Equivalent to the private static `DocIdSetBuilder.dedup`.
fn dedup(arr: &mut [i32], length: usize) -> usize {
    if length == 0 {
        return 0;
    }
    let mut l = 1usize;
    let mut previous = arr[0];
    for i in 1..length {
        let value = arr[i];
        debug_assert!(value >= previous);
        if value != previous {
            arr[l] = value;
            l += 1;
            previous = value;
        }
    }
    l
}

/// Adds many documents to a [`DocIdSetBuilder`] in one go.
///
/// Port of the sealed interface `DocIdSetBuilder.BulkAdder`; see the note on
/// [`DocIdSetBuilder`] for how the two Java implementations collapse into one
/// Rust type.
pub struct BulkAdder<'a> {
    builder: &'a mut DocIdSetBuilder,
}

impl BulkAdder<'_> {
    /// Adds one document.
    pub fn add(&mut self, doc: i32) {
        self.builder.adder_add(doc);
    }

    /// Adds every document in `docs`.
    pub fn add_ints(&mut self, docs: &IntsRef) {
        for i in docs.offset..docs.offset + docs.length {
            self.builder.adder_add(docs.ints[i]);
        }
    }

    /// Adds every document in `docs` that is `>= doc_lower_bound_inclusive`.
    pub fn add_ints_from(&mut self, docs: &IntsRef, doc_lower_bound_inclusive: i32) {
        for i in docs.offset..docs.offset + docs.length {
            let doc = docs.ints[i];
            if doc >= doc_lower_bound_inclusive {
                self.builder.adder_add(doc);
            }
        }
    }

    /// Adds every document produced by `iterator`.
    ///
    /// # Errors
    ///
    /// Propagates iteration errors.
    pub fn add_iterator(&mut self, iterator: &mut dyn DocIdSetIterator) -> Result<()> {
        match self.builder.bit_set.as_mut() {
            Some(bit_set) => {
                iterator.next_doc()?;
                iterator.into_bit_set(NO_MORE_DOCS, bit_set, 0)
            }
            None => {
                let mut doc = iterator.next_doc()?;
                while doc != NO_MORE_DOCS {
                    self.builder.adder_add(doc);
                    doc = iterator.next_doc()?;
                }
                Ok(())
            }
        }
    }
}
