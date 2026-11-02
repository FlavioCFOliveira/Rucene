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

use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
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
///
/// [`std::fmt::Display`] is a supertrait because Lucene's `BitDocIdSet.toString()`
/// concatenates the set it wraps, so every `BitSet` has to be renderable. In Java
/// that comes free from `Object.toString()`; here each implementation supplies it,
/// and the one Lucene class that overrides `toString()` — `SparseFixedBitSet` —
/// reproduces its text exactly.
pub trait BitSet: Bits + Accountable + std::fmt::Display {
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

    /// Returns this set as a mutable [`std::any::Any`], so that a caller can
    /// cast it to its concrete type and replace it.
    ///
    /// This hook stands for Java's cast on a `BitSet`:
    /// `HnswGraphSearcher.prepareScratchState` casts its `visited` set with
    /// `(FixedBitSet) visited` to grow it (`HnswGraphSearcher.java:372`). It
    /// follows [`PostingsEnum::as_any_mut`](crate::index::PostingsEnum::as_any_mut);
    /// the default answers `None`, and the two concrete sets override it.
    fn bit_set_as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
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
        return Err(LuceneError::illegal_state(format!(
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
        Box::new(SparseFixedBitSet::new(max_doc.max(0) as usize)?)
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

    /// Equivalent to `FixedBitSet.clear(int, int)`.
    fn clear_range(&mut self, start_index: usize, end_index: usize) {
        FixedBitSet::clear_range(self, start_index, end_index);
    }

    fn cardinality(&self) -> usize {
        FixedBitSet::cardinality(self)
    }

    /// Equivalent to `FixedBitSet.approximateCardinality()`.
    fn approximate_cardinality(&self) -> usize {
        FixedBitSet::approximate_cardinality(self)
    }

    /// Port of `FixedBitSet.prevSetBit`.
    fn prev_set_bit(&self, index: i32) -> i32 {
        debug_assert!(index >= 0 && (index as usize) < self.length());
        let bits = self.get_bits();
        let mut i = (index >> 6) as isize;
        let sub_index = (index & 0x3f) as u32;
        let word = bits[i as usize].load(Relaxed) << (63 - sub_index);

        if word != 0 {
            return ((i << 6) as i32) + sub_index as i32 - word.leading_zeros() as i32;
        }

        i -= 1;
        while i >= 0 {
            let word = bits[i as usize].load(Relaxed);
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

    fn bit_set_as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

/// Port of the private `FixedBitSet.nextSetBitInRange`.
///
/// Depends on the ghost bits past `num_bits` being clear.
fn next_set_bit_in_range_words(
    bits: &[AtomicU64],
    num_bits: usize,
    start: i32,
    upper_bound: i32,
) -> i32 {
    debug_assert!(start >= 0 && (start as usize) < num_bits);
    debug_assert!(start < upper_bound);
    debug_assert!(upper_bound as usize <= num_bits);
    let mut i = (start >> 6) as usize;
    // Java's `>>` on a long shifts by `start & 63`.
    let word = bits[i].load(Relaxed) >> (start & 63);

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
        let word = bits[i].load(Relaxed);
        if word != 0 {
            return ((i << 6) as i32) + word.trailing_zeros() as i32;
        }
        i += 1;
    }

    NO_MORE_DOCS
}

/// Port of the private `FixedBitSet.nextClearBitInRange`.
fn next_clear_bit_in_range_words(
    bits: &[AtomicU64],
    num_bits: usize,
    start: i32,
    upper_bound: i32,
) -> i32 {
    debug_assert!(start >= 0 && (start as usize) < num_bits);
    debug_assert!(start < upper_bound);
    debug_assert!(upper_bound as usize <= num_bits);
    let mut i = (start >> 6) as usize;
    let word = !(bits[i].load(Relaxed) >> (start & 63));

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
        let word = !bits[i].load(Relaxed);
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

    /// Equivalent to `SparseFixedBitSet.clear(int, int)`
    /// (`SparseFixedBitSet.java:330`).
    fn clear_range(&mut self, start_index: usize, end_index: usize) {
        SparseFixedBitSet::clear_range(self, start_index, end_index);
    }

    fn cardinality(&self) -> usize {
        SparseFixedBitSet::cardinality(self)
    }

    fn approximate_cardinality(&self) -> usize {
        SparseFixedBitSet::approximate_cardinality(self)
    }

    /// Equivalent to `SparseFixedBitSet.prevSetBit(int)`
    /// (`SparseFixedBitSet.java:553`).
    fn prev_set_bit(&self, index: i32) -> i32 {
        debug_assert!(index >= 0);
        SparseFixedBitSet::prev_set_bit(self, index as usize)
    }

    fn next_set_bit_in_range(&self, start: i32, end: i32) -> i32 {
        match SparseFixedBitSet::next_set_bit(self, start.max(0) as usize) {
            Some(next) if (next as i32) < end => next as i32,
            _ => NO_MORE_DOCS,
        }
    }

    /// Equivalent to `SparseFixedBitSet.nextClearBit(int)`
    /// (`SparseFixedBitSet.java:418-421`).
    fn next_clear_bit(&self, index: i32) -> i32 {
        SparseFixedBitSet::next_clear_bit(self, index as usize)
            .map_or(NO_MORE_DOCS, |bit| bit as i32)
    }

    /// Equivalent to `SparseFixedBitSet.nextClearBit(int, int)`
    /// (`SparseFixedBitSet.java:424-427`).
    fn next_clear_bit_in_range(&self, start: i32, upper_bound: i32) -> i32 {
        SparseFixedBitSet::next_clear_bit_up_to(self, start as usize, upper_bound as usize)
            .map_or(NO_MORE_DOCS, |bit| bit as i32)
    }

    /// Equivalent to `SparseFixedBitSet.or(DocIdSetIterator)`
    /// (`SparseFixedBitSet.java:691-713`): an iterator over another
    /// `SparseFixedBitSet` is unioned set to set with `or(SparseFixedBitSet)`,
    /// a sparse iterator goes through the default bit-by-bit union, and a
    /// dense one through [`or_dense`](SparseFixedBitSet::or_dense).
    fn or(&mut self, iter: &mut dyn DocIdSetIterator) -> Result<()> {
        // Specialize union with another SparseFixedBitSet.
        if let Some(other) = BitSetIterator::get_sparse_fixed_bit_set_or_null(iter) {
            check_unpositioned(iter)?;
            self.or_sparse(other);
            return Ok(());
        }
        // FixedBitSets are not specialised: they are meant for dense data and
        // sparse fixed bit sets for sparse data, so a sparse set would likely
        // be upgraded by DocIdSetBuilder before being or'ed with a FixedBitSet.
        check_unpositioned(iter)?;
        if iter.cost() < crate::util::bit_sets::sparse_block_count(self.length()) as i64 {
            // the default impl is good for sparse iterators
            let mut doc = iter.next_doc()?;
            while doc != NO_MORE_DOCS {
                SparseFixedBitSet::set(self, doc as usize);
                doc = iter.next_doc()?;
            }
            Ok(())
        } else {
            self.or_dense(iter)
        }
    }

    fn as_sparse_fixed_bit_set(&self) -> Option<&SparseFixedBitSet> {
        Some(self)
    }

    fn bit_set_as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
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

    /// Returns the set `iterator` wraps when it is a [`BitSetIterator`] over a
    /// [`FixedBitSet`], and `None` otherwise.
    ///
    /// Equivalent to the static
    /// `BitSetIterator.getFixedBitSetOrNull(DocIdSetIterator)`
    /// (`BitSetIterator.java:42-45`), through the private `getBitSet`, which
    /// tests `iterator instanceof BitSetIterator`
    /// ([`DocIdSetIterator::disi_as_any`]) and then the set's class
    /// ([`BitSet::as_fixed_bit_set`]).
    pub fn get_fixed_bit_set_or_null(iterator: &dyn DocIdSetIterator) -> Option<&FixedBitSet> {
        Self::narrow(iterator).and_then(|it| it.bits.as_fixed_bit_set())
    }

    /// Returns the set `iterator` wraps when it is a [`BitSetIterator`] over a
    /// [`SparseFixedBitSet`], and `None` otherwise.
    ///
    /// Equivalent to the static
    /// `BitSetIterator.getSparseFixedBitSetOrNull(DocIdSetIterator)`
    /// (`BitSetIterator.java:47-52`).
    pub fn get_sparse_fixed_bit_set_or_null(
        iterator: &dyn DocIdSetIterator,
    ) -> Option<&SparseFixedBitSet> {
        Self::narrow(iterator).and_then(|it| it.bits.as_sparse_fixed_bit_set())
    }

    /// `iterator instanceof BitSetIterator`, with the cast.
    fn narrow(iterator: &dyn DocIdSetIterator) -> Option<&BitSetIterator> {
        iterator
            .disi_as_any()
            .and_then(|any| any.downcast_ref::<BitSetIterator>())
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

    /// A `BitSetIterator` is the class Lucene's `instanceof` tests recognise.
    fn disi_as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn disi_as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
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

    /// `FixedBitSet.andNot(DocIdSetIterator)` tests
    /// `iter instanceof DocBaseBitSetIterator` (`FixedBitSet.java:670`).
    fn disi_as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn disi_as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

// ---------------------------------------------------------------------------
// FixedBits
// ---------------------------------------------------------------------------

/// An immutable twin of [`FixedBitSet`].
///
/// Port of `org.apache.lucene.util.FixedBits`.
#[derive(Debug, Clone)]
pub struct FixedBits {
    bit_set: FixedBitSet,
}

impl FixedBits {
    /// Creates a `FixedBits` over the given words and bit length.
    ///
    /// Equivalent to `new FixedBits(long[], int)`, which builds its
    /// `FixedBitSet` with `new FixedBitSet(bits, length)`.
    ///
    /// # Errors
    ///
    /// Returns the [`LuceneError::IllegalArgument`] of
    /// [`FixedBitSet::from_bits`] when `bits` is too short for `length`.
    pub fn new(bits: Vec<u64>, length: usize) -> Result<Self> {
        Ok(Self::from_bit_set(FixedBitSet::from_bits(bits, length)?))
    }

    /// Wraps `bit_set`, the `FixedBitSet` Java's constructor builds over the
    /// words it is given.
    ///
    /// [`FixedBitSet::as_read_only_bits`] passes a set over its own words, so
    /// the view shares them as `new FixedBits(bits, numBits)` shares the
    /// `long[]` (`FixedBitSet.java:894`).
    pub(crate) fn from_bit_set(bit_set: FixedBitSet) -> Self {
        Self { bit_set }
    }

    /// Returns the underlying bit set.
    ///
    /// Equivalent to the package-private field `FixedBits.bitSet`, which
    /// `FixedBitSet.copyOf(Bits)` reads to restore the original set; crate
    /// visibility is Rust's nearest equivalent of package-private.
    pub(crate) fn bit_set(&self) -> &FixedBitSet {
        &self.bit_set
    }

    /// Clears the bits of `dest` whose corresponding bit, offset by `offset`,
    /// is clear in this instance.
    ///
    /// Equivalent to `FixedBits.applyMask(FixedBitSet, int)`, which forwards to
    /// `FixedBitSet.applyMask` (`FixedBits.java:33-36`).
    ///
    /// # Errors
    ///
    /// Returns the `IllegalArgumentException` of [`FixedBitSet::apply_mask`]
    /// when `dest` has bits set beyond the end of this instance.
    pub fn apply_mask(&self, dest: &mut FixedBitSet, offset: usize) -> Result<()> {
        self.bit_set.apply_mask(dest, offset)
    }
}

impl Bits for FixedBits {
    fn get(&self, index: usize) -> bool {
        self.bit_set.get(index)
    }

    fn length(&self) -> usize {
        self.bit_set.length()
    }

    fn apply_mask(&self, dest: &mut FixedBitSet, offset: usize) -> Result<()> {
        FixedBits::apply_mask(self, dest, offset)
    }

    /// `FixedBitSet.copyOf(Bits)` tests `bits instanceof FixedBits`
    /// (`FixedBitSet.java:867`).
    fn bits_as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

impl Accountable for FixedBits {
    fn ram_bytes_used(&self) -> i64 {
        RamUsageEstimator::align_object_size(
            RamUsageEstimator::NUM_BYTES_ARRAY_HEADER + 8 * self.bit_set.get_bits().len() as i64,
        )
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

#[cfg(test)]
mod fixed_bit_set_trait_tests {
    use super::*;

    /// The `BitSet` view of a `FixedBitSet` answers `approximateCardinality`
    /// with `FixedBitSet.approximateCardinality()` (`FixedBitSet.java:261`),
    /// which samples once the set spans more than 1024 words. Measured
    /// against `lucene-core-10.5.0.jar` (`oracle/fix/MiscFix.java`, item 31)
    /// over every third of `64 * 4096` bits: `approximateCardinality=87040
    /// cardinality=87382`.
    #[test]
    fn approximate_cardinality_is_lucenes_sample() {
        let mut bits = FixedBitSet::new(64 * 4096);
        for i in (0..64 * 4096).step_by(3) {
            bits.set(i);
        }
        assert_eq!(
            (BitSet::approximate_cardinality(&bits), bits.cardinality()),
            (87_040, 87_382)
        );
    }

    /// The `BitSet` view clears a range with `FixedBitSet.clear(int, int)`.
    #[test]
    fn clear_range_is_the_fixed_bit_set_clear() {
        let mut bits = FixedBitSet::new(300);
        bits.set_range(0, 300);
        BitSet::clear_range(&mut bits, 10, 290);
        assert_eq!(bits.cardinality(), 20);
        assert!(bits.get(9) && !bits.get(10) && !bits.get(289) && bits.get(290));
    }
}

#[cfg(test)]
mod sparse_fixed_bit_set_trait_tests {
    use super::*;

    fn sparse() -> SparseFixedBitSet {
        let mut bits = SparseFixedBitSet::new(10_000).unwrap();
        for i in [5usize, 6, 7, 4100, 4101, 9000] {
            bits.set(i);
        }
        bits
    }

    /// The `BitSet` view of a `SparseFixedBitSet` answers with the class's own
    /// overrides: `clear(int, int)`, `prevSetBit`, `nextClearBit` and
    /// `nextClearBit(int, int)` (`SparseFixedBitSet.java:330, 553, 418, 424`).
    #[test]
    fn the_bit_set_view_answers_with_the_sparse_overrides() {
        let mut bits = sparse();
        assert_eq!(BitSet::prev_set_bit(&bits, 4099), 7);
        assert_eq!(BitSet::prev_set_bit(&bits, 4), -1);
        assert_eq!(BitSet::next_clear_bit(&bits, 5), 8);
        assert_eq!(
            BitSet::next_clear_bit_in_range(&bits, 4100, 4102),
            NO_MORE_DOCS
        );
        assert_eq!(BitSet::next_clear_bit_in_range(&bits, 4100, 4103), 4102);
        BitSet::clear_range(&mut bits, 6, 4101);
        assert_eq!(
            (0..10_000).filter(|&i| bits.get(i)).collect::<Vec<_>>(),
            vec![5, 4101, 9000]
        );
    }

    /// `SparseFixedBitSet.set(int, int)` and `clear(int, int)` over ranges that
    /// span several words and blocks: `mask(0, to)` and `mask(from, 63)` take
    /// their shift distances modulo 64, as Java's `long` shifts do
    /// (`SparseFixedBitSet.java:251-268, 330-376`).
    #[test]
    fn ranges_across_words_and_blocks_set_and_clear_every_bit() {
        let mut bits = SparseFixedBitSet::new(10_000).unwrap();
        bits.set_range(10, 8200);
        assert_eq!(BitSet::cardinality(&bits), 8190);
        assert!(!bits.get(9) && bits.get(10) && bits.get(8199) && !bits.get(8200));
        bits.clear_range(70, 8130);
        assert_eq!(
            (0..10_000).filter(|&i| bits.get(i)).count(),
            (10..70).count() + (8130..8200).count()
        );
    }

    /// `SparseFixedBitSet.or(DocIdSetIterator)` unions a dense iterator (cost at
    /// least the block count) through `orDense` (`SparseFixedBitSet.java:707-712`)
    /// and a sparse one bit by bit; both give the union.
    #[test]
    fn or_unions_dense_iterators_through_or_dense() {
        let mut dense = FixedBitSet::new(10_000);
        for i in (0..10_000).step_by(7) {
            dense.set(i);
        }
        let mut expected = dense.clone();
        expected.set(4100);
        let mut bits = SparseFixedBitSet::new(10_000).unwrap();
        bits.set(4100);
        let cost = dense.cardinality() as i64;
        assert!(cost >= crate::util::bit_sets::sparse_block_count(10_000) as i64);
        let mut iterator = BitSetIterator::new(Arc::new(dense), cost).unwrap();
        BitSet::or(&mut bits, &mut iterator).unwrap();
        assert_eq!(BitSet::cardinality(&bits), expected.cardinality());
        assert!((0..10_000).all(|i| bits.get(i) == expected.get(i)));
    }
}

#[cfg(test)]
mod narrowing_tests {
    use super::*;

    fn fixed(len: usize, docs: &[usize]) -> FixedBitSet {
        let mut b = FixedBitSet::new(len);
        for &d in docs {
            b.set(d);
        }
        b
    }

    fn ones(b: &FixedBitSet) -> Vec<usize> {
        (0..b.length()).filter(|&i| b.get(i)).collect()
    }

    /// A plain iterator over `docs`, which no `instanceof` recognises.
    struct Plain {
        docs: Vec<i32>,
        i: usize,
        doc: i32,
    }

    impl Plain {
        fn new(docs: &[i32]) -> Self {
            Self {
                docs: docs.to_vec(),
                i: 0,
                doc: -1,
            }
        }
    }

    impl DocIdSetIterator for Plain {
        fn doc_id(&self) -> i32 {
            self.doc
        }
        fn next_doc(&mut self) -> Result<i32> {
            self.doc = self.docs.get(self.i).copied().unwrap_or(NO_MORE_DOCS);
            self.i += 1;
            Ok(self.doc)
        }
        fn advance(&mut self, target: i32) -> Result<i32> {
            while self.doc < target {
                self.next_doc()?;
            }
            Ok(self.doc)
        }
        fn cost(&self) -> i64 {
            self.docs.len() as i64
        }
    }

    /// rmp #297: `FixedBitSet.xor(DocIdSetIterator)` XORs the set behind a
    /// `BitSetIterator` word by word without iterating it, and flips each doc
    /// of any other iterator (`FixedBitSet.java:616-627`).
    #[test]
    fn xor_narrows_a_bit_set_iterator() {
        let mut bits = fixed(130, &[1, 64, 129]);
        let mut it = BitSetIterator::new(Arc::new(fixed(130, &[1, 2, 129])), 3).unwrap();
        bits.xor_iterator(&mut it).unwrap();
        assert_eq!(ones(&bits), vec![2, 64]);
        // The narrowed branch leaves the iterator where it was.
        assert_eq!(it.doc_id(), -1);

        let mut plain = Plain::new(&[2, 3, 500]);
        bits.xor_iterator(&mut plain).unwrap();
        assert_eq!(ones(&bits), vec![3, 64]);
        assert_eq!(plain.doc_id(), 500);
    }

    /// rmp #297: `FixedBitSet.andNot(DocIdSetIterator)` masks out the set of a
    /// `BitSetIterator`, and that of a `DocBaseBitSetIterator` from its doc
    /// base, word by word; any other iterator clears doc by doc
    /// (`FixedBitSet.java:664-679`).
    #[test]
    fn and_not_narrows_bit_set_iterators() {
        let mut bits = fixed(256, &[1, 2, 70, 130, 200]);
        let mut it = BitSetIterator::new(Arc::new(fixed(256, &[2, 70])), 2).unwrap();
        bits.and_not_iterator(&mut it).unwrap();
        assert_eq!(ones(&bits), vec![1, 130, 200]);
        assert_eq!(it.doc_id(), -1);

        let mut base = DocBaseBitSetIterator::new(Arc::new(fixed(64, &[2, 8])), 2, 128).unwrap();
        bits.and_not_iterator(&mut base).unwrap();
        assert_eq!(ones(&bits), vec![1, 200]);
        assert_eq!(base.doc_id(), -1);

        let mut plain = Plain::new(&[1]);
        bits.and_not_iterator(&mut plain).unwrap();
        assert_eq!(ones(&bits), vec![200]);
    }

    /// rmp #297: `FixedBitSet.copyOf(Bits)` restores the `FixedBitSet` behind
    /// a `FixedBits`, and clones a `FixedBitSet`, keeping its array
    /// (`FixedBitSet.java:865-874`); other `Bits` are copied bit by bit into a
    /// set of `bits2words(length)` words.
    #[test]
    fn copy_of_narrows_fixed_bits_and_fixed_bit_sets() {
        let wide = FixedBitSet::from_bits(vec![0b101, 0, 0, 0], 70).unwrap();
        let from_fixed_bits =
            FixedBitSet::copy_of(&FixedBits::new(vec![0b101, 0, 0, 0], 70).unwrap());
        assert_eq!(from_fixed_bits.get_bits().len(), 4);
        assert_eq!(FixedBitSet::copy_of(&wide).get_bits().len(), 4);
        assert_eq!(ones(&from_fixed_bits), vec![0, 2]);
        let generic = FixedBitSet::copy_of(&crate::util::MatchAllBits::new(70));
        assert_eq!((generic.get_bits().len(), generic.cardinality()), (2, 70));
    }

    /// rmp #297: `SparseFixedBitSet.or(DocIdSetIterator)` unions the set behind
    /// a `BitSetIterator` over another `SparseFixedBitSet` set to set, without
    /// iterating it (`SparseFixedBitSet.java:691-700`).
    #[test]
    fn sparse_or_narrows_a_sparse_bit_set_iterator() {
        let mut other = SparseFixedBitSet::new(10_000).unwrap();
        for d in [3usize, 4100, 9000] {
            other.set(d);
        }
        let mut bits = SparseFixedBitSet::new(10_000).unwrap();
        bits.set(5);
        let mut it = BitSetIterator::new(Arc::new(other), 1).unwrap();
        BitSet::or(&mut bits, &mut it).unwrap();
        assert_eq!(it.doc_id(), -1);
        assert_eq!(
            (0..10_000).filter(|&i| bits.get(i)).collect::<Vec<_>>(),
            vec![3, 5, 4100, 9000]
        );
        assert_eq!(BitSet::cardinality(&bits), 4);
    }
}
