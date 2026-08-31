//! Bit-set abstractions ported from `org.apache.lucene.util`.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`BitSet`] | `BitSet` |
//! | [`bit_set_of`] | `BitSet.of(DocIdSetIterator, int)` |
//! | [`BitSetIterator`] | `BitSetIterator` |
//! | [`DocBaseBitSetIterator`] | `DocBaseBitSetIterator` |
//! | [`FixedBits`] | `FixedBits` |
//! | [`LiveDocs`] | `LiveDocs` |
//!
//! The two concrete bit sets, [`FixedBitSet`] and [`SparseFixedBitSet`], are
//! already ported in [`crate::util`] and [`crate::util::bit_sets`]; this module
//! adds the `BitSet` abstraction on top of them, implementing it for both.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use crate::util::bit_sets::SparseFixedBitSet;
use crate::util::{Accountable, Bits, FixedBitSet, RamUsageEstimator};

// ---------------------------------------------------------------------------
// BitSet
// ---------------------------------------------------------------------------

/// Base implementation for a bit set.
///
/// Port of `org.apache.lucene.util.BitSet`.
pub trait BitSet: Bits + Accountable {
    /// Clears every bit of the set.
    ///
    /// Depending on the implementation this may be significantly faster than
    /// `clear_range(0, length())`.
    fn clear_all(&mut self) {
        // Default implementation, for compatibility, exactly as Java's.
        self.clear_range(0, self.length());
    }

    /// Sets the bit at `i`.
    fn set(&mut self, i: usize);

    /// Sets the bit at `i`, returning whether it was already set.
    fn get_and_set(&mut self, i: usize) -> bool;

    /// Clears the bit at `i`.
    fn clear(&mut self, i: usize);

    /// Clears the bits in `[start_index, end_index)`.
    fn clear_range(&mut self, start_index: usize, end_index: usize);

    /// Returns the number of set bits. Likely to run in linear time.
    fn cardinality(&self) -> usize;

    /// Returns an approximation of the cardinality of this set.
    ///
    /// Implementations may trade accuracy for speed.
    fn approximate_cardinality(&self) -> usize;

    /// Returns the index of the last set bit at or before `index`, or `-1` when
    /// there is none.
    fn prev_set_bit(&self, index: i32) -> i32;

    /// Returns the index of the first set bit at or after `index`, or
    /// [`NO_MORE_DOCS`] when there is none.
    fn next_set_bit(&self, index: i32) -> i32 {
        self.next_set_bit_in_range(index, self.length() as i32)
    }

    /// Returns the index of the first set bit in `[start, end)`, or
    /// [`NO_MORE_DOCS`] when there is none.
    fn next_set_bit_in_range(&self, start: i32, end: i32) -> i32;

    /// Returns the index of the first unset bit at or after `index`, or
    /// [`NO_MORE_DOCS`] when every bit in `[index, length())` is set.
    fn next_clear_bit(&self, index: i32) -> i32 {
        self.next_clear_bit_in_range(index, self.length() as i32)
    }

    /// Returns the first unset bit in `[start, upper_bound)`, or
    /// [`NO_MORE_DOCS`] when there is none.
    fn next_clear_bit_in_range(&self, start: i32, upper_bound: i32) -> i32;

    /// ORs the bits produced by `iter` into this set in place.
    ///
    /// The state of the iterator afterwards is undefined.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the iterator is already
    /// positioned, and propagates iteration errors.
    fn or(&mut self, iter: &mut dyn DocIdSetIterator) -> Result<()> {
        check_unpositioned(iter)?;
        let mut doc = iter.next_doc()?;
        while doc != NO_MORE_DOCS {
            self.set(doc as usize);
            doc = iter.next_doc()?;
        }
        Ok(())
    }

    /// Returns this set as a [`FixedBitSet`] when it is one.
    ///
    /// **Divergence from Lucene 10.5.0.** Java performs this test with
    /// `Class.isInstance`; Rust has no such reflection, so the downcast is an
    /// explicit hook that [`FixedBitSet`] overrides. It backs
    /// [`BitSetIterator::get_fixed_bit_set_or_null`].
    fn as_fixed_bit_set(&self) -> Option<&FixedBitSet> {
        None
    }

    /// Returns this set as a [`SparseFixedBitSet`] when it is one. See
    /// [`BitSet::as_fixed_bit_set`] for why this hook exists.
    fn as_sparse_fixed_bit_set(&self) -> Option<&SparseFixedBitSet> {
        None
    }
}

/// Asserts that `iter` has not been positioned yet.
///
/// Port of the protected `BitSet.checkUnpositioned`.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalState`] when `iter.doc_id() != -1`, which is
/// Java's `IllegalStateException`.
pub fn check_unpositioned(iter: &dyn DocIdSetIterator) -> Result<()> {
    if iter.doc_id() != -1 {
        return Err(LuceneError::IllegalState(format!(
            "This operation only works with an unpositioned iterator, got current position = {}",
            iter.doc_id()
        )));
    }
    Ok(())
}

/// Builds a [`BitSet`] from the content of `it`, fully consuming it.
///
/// Port of `BitSet.of(DocIdSetIterator, int)`: a [`SparseFixedBitSet`] is used
/// when the iterator's cost is below `max_doc >> 7`, a [`FixedBitSet`]
/// otherwise.
///
/// # Errors
///
/// Propagates iteration errors and rejects an already-positioned iterator.
pub fn bit_set_of(it: &mut dyn DocIdSetIterator, max_doc: i32) -> Result<Box<dyn BitSet>> {
    let cost = it.cost();
    let threshold = (max_doc as i64) >> 7;
    let mut set: Box<dyn BitSet> = if cost < threshold {
        Box::new(SparseFixedBitSet::new(max_doc.max(0) as usize))
    } else {
        Box::new(FixedBitSet::new(max_doc.max(0) as usize))
    };
    set.or(it)?;
    Ok(set)
}

// ---------------------------------------------------------------------------
// BitSet for FixedBitSet
// ---------------------------------------------------------------------------

impl BitSet for FixedBitSet {
    fn clear_all(&mut self) {
        FixedBitSet::clear_all(self);
    }

    fn set(&mut self, i: usize) {
        FixedBitSet::set(self, i);
    }

    fn get_and_set(&mut self, i: usize) -> bool {
        FixedBitSet::get_and_set(self, i)
    }

    fn clear(&mut self, i: usize) {
        FixedBitSet::clear(self, i);
    }

    /// **Divergence from Lucene 10.5.0.** Java clears whole words with two edge
    /// masks and an `Arrays.fill`. Rucene's [`FixedBitSet`] does not expose its
    /// backing words mutably, so the range is cleared bit by bit. The resulting
    /// set is identical; only the constant factor differs.
    fn clear_range(&mut self, start_index: usize, end_index: usize) {
        debug_assert!(start_index <= self.length());
        debug_assert!(end_index <= self.length());
        if end_index <= start_index {
            return;
        }
        for i in start_index..end_index {
            FixedBitSet::clear(self, i);
        }
    }

    fn cardinality(&self) -> usize {
        FixedBitSet::cardinality(self)
    }

    /// Naive sampling: counts the bits set in the first 16 words of every 1024
    /// words and scales by `1024 / 16`. Port of
    /// `FixedBitSet.approximateCardinality`.
    fn approximate_cardinality(&self) -> usize {
        const RANGE_LENGTH: usize = 16;
        const INTERVAL: usize = 1024;

        let num_words = FixedBitSet::bits2words(self.length());
        if num_words <= INTERVAL {
            return FixedBitSet::cardinality(self);
        }

        let bits = self.get_bits();
        let mut pop_count: u64 = 0;
        let mut max_word = 0usize;
        while max_word + INTERVAL < num_words {
            for i in 0..RANGE_LENGTH {
                pop_count += bits[max_word + i].count_ones() as u64;
            }
            max_word += INTERVAL;
        }

        pop_count *= ((INTERVAL / RANGE_LENGTH) * num_words / max_word) as u64;
        pop_count as usize
    }

    /// Port of `FixedBitSet.prevSetBit`.
    fn prev_set_bit(&self, index: i32) -> i32 {
        debug_assert!(index >= 0 && (index as usize) < self.length());
        let bits = self.get_bits();
        let mut i = (index >> 6) as isize;
        let sub_index = (index & 0x3f) as u32;
        let word = bits[i as usize] << (63 - sub_index);

        if word != 0 {
            return ((i << 6) as i32) + sub_index as i32 - word.leading_zeros() as i32;
        }

        i -= 1;
        while i >= 0 {
            let word = bits[i as usize];
            if word != 0 {
                return ((i << 6) as i32) + 63 - word.leading_zeros() as i32;
            }
            i -= 1;
        }

        -1
    }

    fn next_set_bit(&self, index: i32) -> i32 {
        // Java skips the bound check because the result cannot go out of bounds.
        next_set_bit_in_range_words(self.get_bits(), self.length(), index, self.length() as i32)
    }

    /// Port of `FixedBitSet.nextSetBit(int,int)`.
    fn next_set_bit_in_range(&self, start: i32, end: i32) -> i32 {
        let res = next_set_bit_in_range_words(self.get_bits(), self.length(), start, end);
        if res < end {
            res
        } else {
            NO_MORE_DOCS
        }
    }

    /// Port of `FixedBitSet.nextClearBit(int)`.
    fn next_clear_bit(&self, index: i32) -> i32 {
        let num_bits = self.length() as i32;
        let res = next_clear_bit_in_range_words(self.get_bits(), self.length(), index, num_bits);
        // Ghost bits past `length()` read as clear; cap the result.
        if res < num_bits {
            res
        } else {
            NO_MORE_DOCS
        }
    }

    /// Port of `FixedBitSet.nextClearBit(int,int)`.
    fn next_clear_bit_in_range(&self, start: i32, upper_bound: i32) -> i32 {
        let res = next_clear_bit_in_range_words(self.get_bits(), self.length(), start, upper_bound);
        if res < upper_bound {
            res
        } else {
            NO_MORE_DOCS
        }
    }

    fn as_fixed_bit_set(&self) -> Option<&FixedBitSet> {
        Some(self)
    }
}

/// Port of the private `FixedBitSet.nextSetBitInRange`.
///
/// Depends on the ghost bits past `num_bits` being clear.
fn next_set_bit_in_range_words(bits: &[u64], num_bits: usize, start: i32, upper_bound: i32) -> i32 {
    debug_assert!(start >= 0 && (start as usize) < num_bits);
    debug_assert!(start < upper_bound);
    debug_assert!(upper_bound as usize <= num_bits);
    let mut i = (start >> 6) as usize;
    // Java's `>>` on a long shifts by `start & 63`.
    let word = bits[i] >> (start & 63);

    if word != 0 {
        return start + word.trailing_zeros() as i32;
    }

    let num_words = FixedBitSet::bits2words(num_bits);
    let limit = if upper_bound as usize == num_bits {
        num_words
    } else {
        FixedBitSet::bits2words(upper_bound as usize)
    };
    i += 1;
    while i < limit {
        let word = bits[i];
        if word != 0 {
            return ((i << 6) as i32) + word.trailing_zeros() as i32;
        }
        i += 1;
    }

    NO_MORE_DOCS
}

/// Port of the private `FixedBitSet.nextClearBitInRange`.
fn next_clear_bit_in_range_words(
    bits: &[u64],
    num_bits: usize,
    start: i32,
    upper_bound: i32,
) -> i32 {
    debug_assert!(start >= 0 && (start as usize) < num_bits);
    debug_assert!(start < upper_bound);
    debug_assert!(upper_bound as usize <= num_bits);
    let mut i = (start >> 6) as usize;
    let word = !(bits[i] >> (start & 63));

    if word != 0 {
        return start + word.trailing_zeros() as i32;
    }

    let num_words = FixedBitSet::bits2words(num_bits);
    let limit = if upper_bound as usize == num_bits {
        num_words
    } else {
        FixedBitSet::bits2words(upper_bound as usize)
    };
    i += 1;
    while i < limit {
        let word = !bits[i];
        if word != 0 {
            return ((i << 6) as i32) + word.trailing_zeros() as i32;
        }
        i += 1;
    }

    NO_MORE_DOCS
}

// ---------------------------------------------------------------------------
// BitSet for SparseFixedBitSet
// ---------------------------------------------------------------------------

/// **Divergence from Lucene 10.5.0.** `SparseFixedBitSet` keeps its `indices`,
/// `bits` and `nonZeroLongCount` fields private, and this port may not widen
/// them, so `get_and_set`, `clear_range`, `prev_set_bit` and the two
/// clear-bit scans are expressed through the public `get`/`set`/`clear`/
/// `next_set_bit` API instead of Java's word-level arithmetic. Results are
/// identical; only the constant factor differs. `cardinality` and
/// `approximate_cardinality` already exist on the type and are Lucene's own.
impl BitSet for SparseFixedBitSet {
    fn clear_all(&mut self) {
        SparseFixedBitSet::clear_all(self);
    }

    fn set(&mut self, i: usize) {
        SparseFixedBitSet::set(self, i);
    }

    fn get_and_set(&mut self, i: usize) -> bool {
        let previous = SparseFixedBitSet::get(self, i);
        SparseFixedBitSet::set(self, i);
        previous
    }

    fn clear(&mut self, i: usize) {
        SparseFixedBitSet::clear(self, i);
    }

    fn clear_range(&mut self, start_index: usize, end_index: usize) {
        if end_index <= start_index {
            return;
        }
        for i in start_index..end_index.min(self.length()) {
            SparseFixedBitSet::clear(self, i);
        }
    }

    fn cardinality(&self) -> usize {
        SparseFixedBitSet::cardinality(self)
    }

    fn approximate_cardinality(&self) -> usize {
        SparseFixedBitSet::approximate_cardinality(self)
    }

    fn prev_set_bit(&self, index: i32) -> i32 {
        debug_assert!(index >= 0);
        let mut i = index;
        while i >= 0 {
            if SparseFixedBitSet::get(self, i as usize) {
                return i;
            }
            i -= 1;
        }
        -1
    }

    fn next_set_bit_in_range(&self, start: i32, end: i32) -> i32 {
        match SparseFixedBitSet::next_set_bit(self, start.max(0) as usize) {
            Some(next) if (next as i32) < end => next as i32,
            _ => NO_MORE_DOCS,
        }
    }

    fn next_clear_bit_in_range(&self, start: i32, upper_bound: i32) -> i32 {
        let mut i = start.max(0);
        while i < upper_bound {
            if !SparseFixedBitSet::get(self, i as usize) {
                return i;
            }
            i += 1;
        }
        NO_MORE_DOCS
    }

    fn as_sparse_fixed_bit_set(&self) -> Option<&SparseFixedBitSet> {
        Some(self)
    }
}

// ---------------------------------------------------------------------------
// BitSetIterator
// ---------------------------------------------------------------------------

/// A [`DocIdSetIterator`] over the set bits of a [`BitSet`].
///
/// Port of `org.apache.lucene.util.BitSetIterator`.
///
/// **Divergence from Lucene 10.5.0.** Java holds a bare reference to the bit
/// set, which the JVM keeps alive for it. This port holds an [`Arc`], the
/// crate's idiom for a shared, immutable-after-construction structure, so that
/// `BitDocIdSet::iterator` can hand out iterators without copying the set.
/// The static helpers `getFixedBitSetOrNull(DocIdSetIterator)` and
/// `getSparseFixedBitSetOrNull(DocIdSetIterator)` become the instance methods
/// [`Self::get_fixed_bit_set_or_null`] and
/// [`Self::get_sparse_fixed_bit_set_or_null`]: their Java form first tests
/// `iterator instanceof BitSetIterator`, and `DocIdSetIterator` — which is
/// outside this module — carries no downcast hook.
pub struct BitSetIterator {
    bits: Arc<dyn BitSet>,
    length: usize,
    cost: i64,
    doc: i32,
}

impl std::fmt::Debug for BitSetIterator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BitSetIterator")
            .field("length", &self.length)
            .field("cost", &self.cost)
            .field("doc", &self.doc)
            .finish()
    }
}

impl BitSetIterator {
    /// Creates an iterator over `bits` advertising `cost` as its cost.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `cost` is negative.
    pub fn new(bits: Arc<dyn BitSet>, cost: i64) -> Result<Self> {
        if cost < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "cost must be >= 0, got {cost}"
            )));
        }
        let length = bits.length();
        Ok(Self {
            bits,
            length,
            cost,
            doc: -1,
        })
    }

    /// Returns the wrapped [`BitSet`].
    pub fn get_bit_set(&self) -> &Arc<dyn BitSet> {
        &self.bits
    }

    /// Returns the wrapped set when it is a [`FixedBitSet`].
    ///
    /// Equivalent to `BitSetIterator.getFixedBitSetOrNull`.
    pub fn get_fixed_bit_set_or_null(&self) -> Option<&FixedBitSet> {
        self.bits.as_fixed_bit_set()
    }

    /// Returns the wrapped set when it is a [`SparseFixedBitSet`].
    ///
    /// Equivalent to `BitSetIterator.getSparseFixedBitSetOrNull`.
    pub fn get_sparse_fixed_bit_set_or_null(&self) -> Option<&SparseFixedBitSet> {
        self.bits.as_sparse_fixed_bit_set()
    }

    /// Sets the current doc id this iterator is on.
    ///
    /// Equivalent to `BitSetIterator.setDocId`.
    pub fn set_doc_id(&mut self, doc_id: i32) {
        self.doc = doc_id;
    }
}

impl DocIdSetIterator for BitSetIterator {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.advance(self.doc + 1)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        if target >= self.length as i32 {
            self.doc = NO_MORE_DOCS;
            return Ok(self.doc);
        }
        self.doc = self.bits.next_set_bit(target);
        Ok(self.doc)
    }

    fn cost(&self) -> i64 {
        self.cost
    }

    fn doc_id_run_end(&self) -> Result<i32> {
        debug_assert!(self.doc != NO_MORE_DOCS);
        let next = self.doc + 1;
        if next >= self.length as i32 {
            return Ok(self.length as i32);
        }
        let end = self.bits.next_clear_bit(next);
        Ok(if end == NO_MORE_DOCS {
            self.length as i32
        } else {
            end
        })
    }
}

// ---------------------------------------------------------------------------
// DocBaseBitSetIterator
// ---------------------------------------------------------------------------

/// A [`BitSetIterator`]-like iterator with a doc base, so that leading zeroes
/// need not be stored.
///
/// Port of `org.apache.lucene.util.DocBaseBitSetIterator`.
pub struct DocBaseBitSetIterator {
    bits: Arc<FixedBitSet>,
    length: usize,
    cost: i64,
    doc_base: i32,
    doc: i32,
}

impl std::fmt::Debug for DocBaseBitSetIterator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocBaseBitSetIterator")
            .field("length", &self.length)
            .field("cost", &self.cost)
            .field("doc_base", &self.doc_base)
            .field("doc", &self.doc)
            .finish()
    }
}

impl DocBaseBitSetIterator {
    /// Creates an iterator over `bits` offset by `doc_base`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `cost` is negative or
    /// `doc_base` is not a multiple of 64.
    pub fn new(bits: Arc<FixedBitSet>, cost: i64, doc_base: i32) -> Result<Self> {
        if cost < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "cost must be >= 0, got {cost}"
            )));
        }
        if (doc_base & 63) != 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "docBase need to be a multiple of 64, got {doc_base}"
            )));
        }
        let length = bits.length() + doc_base as usize;
        Ok(Self {
            bits,
            length,
            cost,
            doc_base,
            doc: -1,
        })
    }

    /// Returns the offset bit set: a doc id is in this iterator when the bit set
    /// contains `doc_id - doc_base`.
    pub fn get_bit_set(&self) -> &Arc<FixedBitSet> {
        &self.bits
    }

    /// Returns the doc base, guaranteed to be a multiple of 64.
    pub fn get_doc_base(&self) -> i32 {
        self.doc_base
    }
}

impl DocIdSetIterator for DocBaseBitSetIterator {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.advance(self.doc + 1)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        if target >= self.length as i32 {
            self.doc = NO_MORE_DOCS;
            return Ok(self.doc);
        }
        let next = BitSet::next_set_bit(&*self.bits, (target - self.doc_base).max(0));
        self.doc = if next == NO_MORE_DOCS {
            NO_MORE_DOCS
        } else {
            next + self.doc_base
        };
        Ok(self.doc)
    }

    fn cost(&self) -> i64 {
        self.cost
    }
}

// ---------------------------------------------------------------------------
// FixedBits
// ---------------------------------------------------------------------------

/// An immutable twin of [`FixedBitSet`].
///
/// Port of `org.apache.lucene.util.FixedBits`.
///
/// **Divergence from Lucene 10.5.0.** Java's `FixedBits` also overrides
/// `Bits.applyMask(FixedBitSet, int)`. Rucene's [`Bits`] trait has no
/// `applyMask` hook, so the operation is offered as the inherent method
/// [`FixedBits::apply_mask`] instead of a trait override.
#[derive(Debug, Clone)]
pub struct FixedBits {
    bit_set: FixedBitSet,
}

impl FixedBits {
    /// Creates a `FixedBits` over the given words and bit length.
    ///
    /// Equivalent to `new FixedBits(long[], int)`.
    pub fn new(bits: Vec<u64>, length: usize) -> Self {
        Self {
            bit_set: FixedBitSet::from_bits(bits, length),
        }
    }

    /// Returns the underlying bit set.
    pub fn bit_set(&self) -> &FixedBitSet {
        &self.bit_set
    }

    /// Clears the bits of `dest` whose corresponding bit, offset by `offset`,
    /// is clear in this instance.
    ///
    /// Equivalent to `FixedBits.applyMask(FixedBitSet, int)`, which forwards to
    /// `FixedBitSet.applyMask`. As in Lucene, the caller must guarantee that
    /// `offset + dest.length()` does not exceed this instance's length;
    /// [`FixedBitSet::get`] enforces it the way Java's assertion does.
    ///
    /// # Panics
    ///
    /// Panics when a bit of `dest` maps past the end of this instance.
    pub fn apply_mask(&self, dest: &mut FixedBitSet, offset: usize) {
        for i in 0..dest.length() {
            if dest.get(i) && !self.bit_set.get(offset + i) {
                dest.clear(i);
            }
        }
    }
}

impl Bits for FixedBits {
    fn get(&self, index: usize) -> bool {
        self.bit_set.get(index)
    }

    fn length(&self) -> usize {
        self.bit_set.length()
    }
}

impl Accountable for FixedBits {
    fn ram_bytes_used(&self) -> i64 {
        RamUsageEstimator::size_of_u64(self.bit_set.get_bits())
    }
}

// ---------------------------------------------------------------------------
// LiveDocs
// ---------------------------------------------------------------------------

/// Extension of [`Bits`] providing efficient iteration over deleted documents.
///
/// Port of `org.apache.lucene.util.LiveDocs`, which Lucene 10.5.0 marks
/// `@lucene.experimental`.
pub trait LiveDocs: Bits {
    /// Returns an iterator over live document ids, in ascending order.
    ///
    /// # Errors
    ///
    /// Propagates errors raised while building the iterator.
    fn live_docs_iterator(&self) -> Result<Box<dyn DocIdSetIterator>>;

    /// Returns an iterator over deleted document ids, in ascending order, or an
    /// empty iterator when nothing is deleted.
    ///
    /// # Errors
    ///
    /// Propagates errors raised while building the iterator.
    fn deleted_docs_iterator(&self) -> Result<Box<dyn DocIdSetIterator>>;

    /// Returns the number of deleted documents.
    fn deleted_count(&self) -> i32;
}
