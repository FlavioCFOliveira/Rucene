//! Additional bitset and live-docs variants ported from `org.apache.lucene.util`.
//!
//! This module provides space-efficient alternatives to [`FixedBitSet`] for
//! sparse and structured doc-id sets, together with the live-docs wrappers used
//! by the default codec:
//!
//! * [`SparseFixedBitSet`] – stores only non-zero 64-bit words, ideal for very
//!   sparse bit patterns.
//! * [`RoaringDocIdSet`] – a roaring-bitmap-inspired doc-id set that selects
//!   a compact encoding per 64 Ki doc block.
//! * [`DenseLiveDocs`] – traditional live docs backed by a [`FixedBitSet`].
//! * [`SparseLiveDocs`] – live docs optimized for sparse deletions by storing
//!   the deleted docs in a [`SparseFixedBitSet`] with inverted semantics.
//! * `FilteredDocIdSetIterator` – the package-private `O(max_doc)` predicate
//!   scan that Lucene's two live-docs classes iterate with.

#![deny(unsafe_code)]

use std::fmt;
use std::sync::Arc;

use crate::search::{DocIdSetIterator, NO_MORE_DOCS};
use crate::util::bit_set::{BitSet, BitSetIterator, LiveDocs};
use crate::util::java_fmt;
use crate::util::{Accountable, FixedBitSet, RamUsageEstimator};

use super::Bits;

// -----------------------------------------------------------------------------
// SparseFixedBitSet
// -----------------------------------------------------------------------------

/// A sparse bit set that stores only 64-bit words that have at least one bit set.
///
/// The bit space is divided into blocks of 4096 bits (64 `u64` words). For each
/// block a *block index* `u64` records which of the 64 word positions are
/// non-zero, and a compact `Vec<u64>` stores only those non-zero words in the
/// order implied by the index. This matches the design of Lucene's
/// `SparseFixedBitSet`.
///
/// Equivalent to `org.apache.lucene.util.SparseFixedBitSet`.
#[derive(Clone, Debug)]
pub struct SparseFixedBitSet {
    /// One index word per 4096-bit block. Bit *i* set means word *i* of the block
    /// is non-null; its offset in `bits[block]` is the pop-count of index bits
    /// to the right of *i*.
    indices: Vec<u64>,
    /// Per-block compact storage, or `None` for empty blocks.
    bits: Vec<Option<Vec<u64>>>,
    /// Total number of bits represented by the set (exclusive upper bound).
    length: usize,
    /// Cached number of non-zero longs across all blocks.
    non_zero_long_count: usize,
}

const SPARSE_BLOCK_BITS: usize = 1 << 12; // 4096
const SPARSE_LONGS_PER_BLOCK: usize = SPARSE_BLOCK_BITS / 64; // 64

fn sparse_block_count(length: usize) -> usize {
    length.div_ceil(SPARSE_BLOCK_BITS)
}

/// Grow an array by ~50%, capping at 64, mirroring Lucene's `ArrayUtil.oversize`
/// behavior used by `SparseFixedBitSet`.
fn oversize(s: usize) -> usize {
    let mut new_size = s + (s >> 1);
    if new_size > 50 {
        new_size = 64;
    }
    new_size
}

impl SparseFixedBitSet {
    /// Creates a sparse bit set that can hold bits in `[0, length)`.
    ///
    /// # Panics
    ///
    /// Panics if `length` is zero.
    pub fn new(length: usize) -> Self {
        assert!(length >= 1, "length needs to be >= 1");
        let block_count = sparse_block_count(length);
        Self {
            indices: vec![0u64; block_count],
            bits: vec![None; block_count],
            length,
            non_zero_long_count: 0,
        }
    }

    /// Clears every bit in the set.
    pub fn clear_all(&mut self) {
        for block in &mut self.bits {
            *block = None;
        }
        self.indices.fill(0);
        self.non_zero_long_count = 0;
    }

    /// Returns the number of bits the set can hold.
    /// Returns whether `index` is inside this set's range.
    ///
    /// Equivalent to the private `SparseFixedBitSet.consistent(int)`, which
    /// Java only evaluates inside an `assert` at the head of each accessor.
    fn consistent(&self, index: usize) -> bool {
        debug_assert!(index < self.length, "index={index},length={}", self.length);
        true
    }

    pub fn length(&self) -> usize {
        self.length
    }

    /// Returns the value of the bit at `index`.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of bounds.
    pub fn get(&self, index: usize) -> bool {
        debug_assert!(self.consistent(index));
        assert!(index < self.length, "index={index} out of bounds");
        let i4096 = index >> 12;
        let i64 = index >> 6;
        let index_word = self.indices[i4096];
        let i64bit = 1u64 << (i64 & 0x3f);
        if (index_word & i64bit) == 0 {
            return false;
        }
        let o = (index_word & (i64bit - 1)).count_ones() as usize;
        let word = self.bits[i4096]
            .as_ref()
            .expect("INVARIANT: index bit implies block")[o];
        (word & (1u64 << (index & 0x3f))) != 0
    }

    /// Sets the bit at `index`.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of bounds.
    pub fn set(&mut self, index: usize) {
        debug_assert!(self.consistent(index));
        assert!(index < self.length, "index={index} out of bounds");
        let i4096 = index >> 12;
        let i64 = index >> 6;
        let i64bit = 1u64 << (i64 & 0x3f);
        let index_word = self.indices[i4096];
        if (index_word & i64bit) != 0 {
            let o = (index_word & (i64bit - 1)).count_ones() as usize;
            self.bits[i4096].as_mut().unwrap()[o] |= 1u64 << (index & 0x3f);
        } else if index_word == 0 {
            self.insert_block(i4096, i64bit, index);
        } else {
            self.insert_long(i4096, i64bit, index, index_word);
        }
    }

    /// Clears the bit at `index`.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of bounds.
    pub fn clear(&mut self, index: usize) {
        debug_assert!(self.consistent(index));
        assert!(index < self.length, "index={index} out of bounds");
        let i4096 = index >> 12;
        let i64 = index >> 6;
        let mask = !(1u64 << (index & 0x3f));
        self.and(i4096, i64, mask);
    }

    /// Returns the number of set bits.
    pub fn cardinality(&self) -> usize {
        self.bits
            .iter()
            .filter_map(|b| b.as_ref())
            .flat_map(|b| b.iter())
            .map(|w| w.count_ones() as usize)
            .sum()
    }

    /// Returns an approximate cardinality using the linear-counting estimator.
    pub fn approximate_cardinality(&self) -> usize {
        let total_longs = self.length.div_ceil(64);
        assert!(total_longs >= self.non_zero_long_count);
        let zero_longs = total_longs - self.non_zero_long_count;
        if zero_longs == 0 {
            return self.length;
        }
        let estimate =
            (total_longs as f64 * ((total_longs as f64) / (zero_longs as f64)).ln()).round();
        self.length.min(estimate as usize)
    }

    /// Returns the first document that occurs on or after the block index
    /// `i4096`, or [`NO_MORE_DOCS`] when no block up to `i4096upper` holds one.
    ///
    /// Equivalent to the private `SparseFixedBitSet.firstDoc(int, int)`. Because
    /// a block stores only its non-zero words, the first word of the block that
    /// the index word points at (`bits[i4096][0]`) is by construction the
    /// lowest-addressed non-zero word, so the answer needs no scan within the
    /// block.
    fn first_doc(&self, mut i4096: usize, i4096upper: usize) -> i32 {
        debug_assert!(
            i4096upper <= self.indices.len(),
            "i4096upper={i4096upper}, indices.len()={}",
            self.indices.len()
        );
        while i4096 < i4096upper {
            let index = self.indices[i4096];
            if index != 0 {
                let i64w = index.trailing_zeros() as usize;
                let block = self.bits[i4096]
                    .as_ref()
                    .expect("INVARIANT: a non-zero index word implies an allocated block");
                return ((i4096 << 12) | (i64w << 6) | block[0].trailing_zeros() as usize) as i32;
            }
            i4096 += 1;
        }
        NO_MORE_DOCS
    }

    /// Returns the next set bit in `[start, upper_bound)`, treating
    /// `upper_bound` as a best-effort hint rather than a hard requirement: the
    /// result may be `>= upper_bound`, so a caller for which the bound is hard
    /// must check it.
    ///
    /// Equivalent to the private `SparseFixedBitSet.nextSetBitInRange(int, int)`.
    ///
    /// The name carries a suffix Java's does not, because Java's *public*
    /// `nextSetBit(int, int)` — which applies the bound strictly — is already
    /// spelled `next_set_bit_in_range` on the [`BitSet`] trait, Rust having no
    /// overloading to separate it from `nextSetBit(int)`. The suffix keeps the
    /// two apart and names the difference between them.
    fn next_set_bit_in_range_best_effort(&self, start: usize, upper_bound: usize) -> i32 {
        debug_assert!(start < self.length);
        debug_assert!(
            upper_bound > start && upper_bound <= self.length,
            "upper_bound={upper_bound}, start={start}, length={}",
            self.length
        );
        let i4096 = start >> 12;
        let index = self.indices[i4096];
        let block = self.bits[i4096].as_ref();
        // Java keeps `i64` as a *global* word index and relies on the JVM's
        // shift-by-modulo-64 to use it as a block-local one; the `& 0x3f` below
        // is that modulo made explicit, since Rust's `>>` would panic instead.
        let mut i64w = start >> 6;
        let i64bit = 1u64 << (i64w & 0x3f);
        let mut o = (index & (i64bit - 1)).count_ones() as usize;
        if (index & i64bit) != 0 {
            // At least one bit is set in the current word; check whether one of
            // them is at or after `start`.
            let bits = block.expect("INVARIANT: a non-zero index word implies an allocated block")
                [o]
                >> (start & 0x3f);
            if bits != 0 {
                return (start + bits.trailing_zeros() as usize) as i32;
            }
            o += 1;
        }
        let index_bits = (index >> (i64w & 0x3f)) >> 1;
        if index_bits == 0 {
            // No more bits are set in this block of 4096; go to the next one.
            let i4096upper = if upper_bound == self.length {
                self.indices.len()
            } else {
                sparse_block_count(upper_bound)
            };
            return self.first_doc(i4096 + 1, i4096upper);
        }
        // There are still set bits in this block.
        i64w += 1 + index_bits.trailing_zeros() as usize;
        let bits = block.expect("INVARIANT: a non-zero index word implies an allocated block")[o];
        ((i64w << 6) | bits.trailing_zeros() as usize) as i32
    }

    /// Returns the first set bit at or after `start`, or `None` if there is none.
    ///
    /// Equivalent to `SparseFixedBitSet.nextSetBit(int)`, which delegates to
    /// `nextSetBitInRange(i, length)` and skips the bound check on the result
    /// because no bit at or past `length` can be set. Java signals "no more" with
    /// `DocIdSetIterator.NO_MORE_DOCS`; this port keeps the `Option` its callers
    /// already use.
    pub fn next_set_bit(&self, start: usize) -> Option<usize> {
        if start >= self.length {
            return None;
        }
        match self.next_set_bit_in_range_best_effort(start, self.length) {
            NO_MORE_DOCS => None,
            doc => Some(doc as usize),
        }
    }

    /// Returns an iterator over the set bits as a [`DocIdSetIterator`].
    ///
    /// Returns the mask of the bits in `[from, to]`, both inclusive, within a
    /// 64-bit word.
    ///
    /// Equivalent to `SparseFixedBitSet.mask(int, int)`. The double shift is
    /// deliberate: `1 << 64` is undefined, so the mask for a full word is built
    /// as two shifts of 63 and 1.
    fn mask(from: usize, to: usize) -> u64 {
        ((1u64 << (to - from) << 1).wrapping_sub(1)) << (from & 0x3f)
    }

    /// Clears the bits in `[from, to]` of block `i4096`, both offsets relative
    /// to the block.
    ///
    /// Equivalent to `SparseFixedBitSet.clearWithinBlock(int, int, int)`.
    fn clear_within_block(&mut self, i4096: usize, from: usize, to: usize) {
        let first_long = from >> 6;
        let last_long = to >> 6;
        if first_long == last_long {
            self.and(i4096, first_long, !Self::mask(from, to));
        } else {
            debug_assert!(first_long < last_long);
            self.and(i4096, last_long, !Self::mask(0, to));
            for i in (first_long + 1..last_long).rev() {
                self.and(i4096, i, 0);
            }
            self.and(i4096, first_long, !Self::mask(from, 63));
        }
    }

    /// Sets the bits in `[from, to]` of block `i4096`, both offsets relative to
    /// the block.
    ///
    /// Equivalent to `SparseFixedBitSet.setWithinBlock(int, int, int)`.
    fn set_within_block(&mut self, i4096: usize, from: usize, to: usize) {
        let first_long = from >> 6;
        let last_long = to >> 6;
        if first_long == last_long {
            self.or_long(i4096, first_long, Self::mask(from, to));
        } else {
            debug_assert!(first_long < last_long);
            self.or_long(i4096, last_long, Self::mask(0, to));
            for i in first_long + 1..last_long {
                self.or_long(i4096, i, u64::MAX);
            }
            self.or_long(i4096, first_long, Self::mask(from, 63));
        }
    }

    /// Sets every bit in `[start_index, end_index)`.
    ///
    /// Equivalent to `SparseFixedBitSet.set(int, int)`.
    ///
    /// # Panics
    ///
    /// Panics when the range falls outside the set.
    pub fn set_range(&mut self, start_index: usize, end_index: usize) {
        assert!(
            start_index < self.length,
            "startIndex={start_index} out of bounds"
        );
        assert!(
            end_index <= self.length,
            "endIndex={end_index} out of bounds"
        );
        if end_index <= start_index {
            return;
        }
        let first_block = start_index >> 12;
        let last_block = (end_index - 1) >> 12;
        const MASK_4096: usize = SPARSE_BLOCK_BITS - 1;
        if first_block == last_block {
            self.set_within_block(
                first_block,
                start_index & MASK_4096,
                (end_index - 1) & MASK_4096,
            );
        } else {
            self.set_within_block(first_block, start_index & MASK_4096, MASK_4096);
            for i in first_block + 1..last_block {
                self.set_within_block(i, 0, MASK_4096);
            }
            self.set_within_block(last_block, 0, (end_index - 1) & MASK_4096);
        }
    }

    /// Clears every bit in `[from, to)`.
    ///
    /// Equivalent to `SparseFixedBitSet.clear(int, int)`. A block that falls
    /// wholly inside the range is dropped rather than cleared word by word.
    pub fn clear_range(&mut self, from: usize, to: usize) {
        assert!(to <= self.length, "to={to} out of bounds");
        if from >= to {
            return;
        }
        let first_block = from >> 12;
        let last_block = (to - 1) >> 12;
        const MASK_4096: usize = SPARSE_BLOCK_BITS - 1;
        if first_block == last_block {
            self.clear_within_block(first_block, from & MASK_4096, (to - 1) & MASK_4096);
        } else {
            self.clear_within_block(first_block, from & MASK_4096, MASK_4096);
            for i in first_block + 1..last_block {
                self.non_zero_long_count -= self.indices[i].count_ones() as usize;
                self.indices[i] = 0;
                self.bits[i] = None;
            }
            self.clear_within_block(last_block, 0, (to - 1) & MASK_4096);
        }
    }

    /// Returns the bits stored at word `i64` of a block, or zero when that word
    /// is absent.
    ///
    /// Equivalent to `SparseFixedBitSet.longBits(long, long[], int)`.
    fn long_bits(index_word: u64, bits: &[u64], i64: usize) -> u64 {
        let i64bit = 1u64 << (i64 & 0x3f);
        if (index_word & i64bit) == 0 {
            0
        } else {
            bits[(index_word & (i64bit - 1)).count_ones() as usize]
        }
    }

    /// Ors `new_bits` into word `i64` of block `i4096`.
    ///
    /// Equivalent to `SparseFixedBitSet.orLong(int, int, long)`.
    fn or_long(&mut self, i4096: usize, i64: usize, new_bits: u64) {
        if new_bits == 0 {
            return;
        }
        let index_word = self.indices[i4096];
        let i64bit = 1u64 << (i64 & 0x3f);
        if (index_word & i64bit) != 0 {
            let o = (index_word & (i64bit - 1)).count_ones() as usize;
            self.bits[i4096]
                .as_mut()
                .expect("INVARIANT: index bit implies block")[o] |= new_bits;
        } else if index_word == 0 {
            self.indices[i4096] = i64bit;
            debug_assert!(self.bits[i4096].is_none());
            self.bits[i4096] = Some(vec![new_bits]);
            self.non_zero_long_count += 1;
        } else {
            self.insert_long_value(i4096, i64bit, new_bits, index_word);
        }
    }

    /// Returns the last set bit at or below `i`, or `-1` when there is none.
    ///
    /// Equivalent to `SparseFixedBitSet.prevSetBit(int)`.
    pub fn prev_set_bit(&self, i: usize) -> i32 {
        let i4096 = i >> 12;
        let index_word = self.indices[i4096];
        let mut i64 = i >> 6;
        let index_bits = index_word & ((1u64 << (i64 & 0x3f)) - 1);
        let o = index_bits.count_ones() as usize;
        if (index_word & (1u64 << (i64 & 0x3f))) != 0 {
            // At least one bit is set in the same word: look for one below `i`.
            let bit_array = self.bits[i4096]
                .as_ref()
                .expect("INVARIANT: index bit implies block");
            let bits = bit_array[o] & ((1u64 << (i & 0x3f) << 1).wrapping_sub(1));
            if bits != 0 {
                return (((i64 << 6) | (63 - bits.leading_zeros() as usize)) & i32::MAX as usize)
                    as i32;
            }
        }
        if index_bits == 0 {
            // Nothing else is set in this block: find the last bit of the
            // previous one.
            return if i4096 == 0 {
                -1
            } else {
                self.last_doc(i4096 - 1)
            };
        }
        // Go to the previous word.
        i64 = 63 - index_bits.leading_zeros() as usize;
        let bit_array = self.bits[i4096]
            .as_ref()
            .expect("INVARIANT: index_bits non-zero implies block");
        let bits = bit_array[o - 1];
        ((i4096 << 12) | (i64 << 6) | (63 - bits.leading_zeros() as usize)) as i32
    }

    /// Returns the last set bit in block `i4096` or any block below it, or `-1`.
    ///
    /// Equivalent to `SparseFixedBitSet.lastDoc(int)`.
    fn last_doc(&self, i4096: usize) -> i32 {
        let mut block = i4096 as i64;
        while block >= 0 {
            let index_word = self.indices[block as usize];
            if index_word != 0 {
                let i64 = 63 - index_word.leading_zeros() as usize;
                let bit_array = self.bits[block as usize]
                    .as_ref()
                    .expect("INVARIANT: index word non-zero implies block");
                let bits = bit_array[index_word.count_ones() as usize - 1];
                return (((block as usize) << 12)
                    | (i64 << 6)
                    | (63 - bits.leading_zeros() as usize)) as i32;
            }
            block -= 1;
        }
        -1
    }

    /// Returns the first unset bit in `[from_global, upper_bound)` within block
    /// `i4096`, or `None`.
    ///
    /// Equivalent to `SparseFixedBitSet.firstClearInBlock(int, int, int)`.
    fn first_clear_in_block(
        &self,
        i4096: usize,
        from_global: usize,
        upper_bound: usize,
    ) -> Option<usize> {
        let block_base = i4096 << 12;
        let block_end = (block_base + SPARSE_BLOCK_BITS).min(self.length);
        let from = from_global.max(block_base);
        let block_cap = upper_bound.min(block_end);
        if from >= block_cap {
            return None;
        }
        let index_word = self.indices[i4096];
        if index_word == 0 {
            return Some(from);
        }
        let bit_array = self.bits[i4096]
            .as_ref()
            .expect("INVARIANT: index word non-zero implies block");
        let mut pos = from;
        while pos < block_cap {
            let i64 = pos >> 6;
            let i64bit = 1u64 << (i64 & 0x3f);
            let w_start = i64 << 6;
            if w_start >= block_end {
                break;
            }
            if (index_word & i64bit) == 0 {
                if pos < block_cap {
                    return Some(pos);
                }
                pos = w_start + 64;
                continue;
            }
            let o = (index_word & (i64bit - 1)).count_ones() as usize;
            let word_bits = bit_array[o];
            let sub = pos - w_start;
            let tail = word_bits >> sub;
            let span = 64 - sub;
            let span_mask = if sub == 0 {
                u64::MAX
            } else {
                (1u64 << span) - 1
            };
            let inv = (!tail) & span_mask;
            if inv != 0 {
                let cand = pos + inv.trailing_zeros() as usize;
                if cand < block_cap {
                    return Some(cand);
                }
            }
            pos = w_start + 64;
        }
        None
    }

    /// Returns the first unset bit in `[start, upper_bound)`, or `None`.
    ///
    /// Equivalent to `SparseFixedBitSet.nextClearBitInRange(int, int)`.
    fn next_clear_bit_in_range(&self, start: usize, upper_bound: usize) -> Option<usize> {
        debug_assert!(start < self.length);
        debug_assert!(upper_bound > start && upper_bound <= self.length);
        let mut i4096 = start >> 12;
        let i4096_upper = if upper_bound == self.length {
            self.indices.len()
        } else {
            sparse_block_count(upper_bound)
        };
        let mut from = start;
        while i4096 < i4096_upper {
            if let Some(res) = self.first_clear_in_block(i4096, from, upper_bound) {
                return Some(res);
            }
            i4096 += 1;
            from = i4096 << 12;
        }
        None
    }

    /// Returns the first unset bit at or after `index`, or `None`.
    ///
    /// Equivalent to `SparseFixedBitSet.nextClearBit(int)`.
    pub fn next_clear_bit(&self, index: usize) -> Option<usize> {
        self.next_clear_bit_in_range(index, self.length)
            .filter(|&res| res < self.length)
    }

    /// Returns the first unset bit in `[start, upper_bound)`, or `None`.
    ///
    /// Equivalent to `SparseFixedBitSet.nextClearBit(int, int)`.
    pub fn next_clear_bit_up_to(&self, start: usize, upper_bound: usize) -> Option<usize> {
        self.next_clear_bit_in_range(start, upper_bound)
            .filter(|&res| res < upper_bound)
    }

    /// Ors the whole block `i4096` with `bits`, whose set words are described by
    /// `index_word`.
    ///
    /// Equivalent to `SparseFixedBitSet.or(int, long, long[], int)`, the bulk
    /// path `or_dense` flushes each block through.
    fn or_block(
        &mut self,
        i4096: usize,
        index_word: u64,
        words: &[u64],
        non_zero_long_count: usize,
    ) {
        debug_assert_eq!(index_word.count_ones() as usize, non_zero_long_count);
        let current_index = self.indices[i4096];
        if current_index == 0 {
            // Fast path: nothing in the block yet, so just copy the data in.
            self.indices[i4096] = index_word;
            self.bits[i4096] = Some(words[..non_zero_long_count].to_vec());
            self.non_zero_long_count += non_zero_long_count;
            return;
        }
        let new_index = current_index | index_word;
        let new_count = new_index.count_ones() as usize;
        let mut new_bits = vec![0u64; new_count];
        let current_bits = self.bits[i4096]
            .as_ref()
            .expect("INVARIANT: index word non-zero implies block");
        let mut remaining = new_index;
        let mut new_o = 0usize;
        while remaining != 0 {
            let bit_index = remaining.trailing_zeros() as usize;
            new_bits[new_o] = Self::long_bits(current_index, current_bits, bit_index)
                | Self::long_bits(index_word, words, bit_index);
            new_o += 1;
            remaining &= remaining - 1;
        }
        self.indices[i4096] = new_index;
        self.bits[i4096] = Some(new_bits);
        self.non_zero_long_count +=
            non_zero_long_count - (current_index & index_word).count_ones() as usize;
    }

    /// Ors every document of `it` into this set, taking advantage of the fact
    /// that they arrive in order.
    ///
    /// Equivalent to `SparseFixedBitSet.orDense(DocIdSetIterator)`. Words are
    /// buffered per block and flushed once, which is what makes it worth using
    /// over the one-bit-at-a-time path for a dense iterator.
    pub fn or_dense(&mut self, it: &mut dyn DocIdSetIterator) -> crate::error::Result<()> {
        // NOTE: this relies on shifts being taken modulo 64.
        let first_doc = it.next_doc()?;
        if first_doc == NO_MORE_DOCS {
            return Ok(());
        }
        let mut i4096 = (first_doc >> 12) as usize;
        let mut i64 = (first_doc >> 6) as usize;
        let mut index_word = 1u64 << (i64 & 0x3f);
        let mut current_long = 1u64 << (first_doc & 0x3f);
        // At most 64 words per block, so this never needs to grow.
        let mut words = [0u64; SPARSE_LONGS_PER_BLOCK];
        let mut num_longs = 0usize;

        let mut doc = it.next_doc()?;
        while doc != NO_MORE_DOCS {
            let doc64 = (doc >> 6) as usize;
            if doc64 == i64 {
                // Still in the same word: just set the bit.
                current_long |= 1u64 << (doc & 0x3f);
            } else {
                words[num_longs] = current_long;
                num_longs += 1;

                let doc4096 = (doc >> 12) as usize;
                if doc4096 == i4096 {
                    index_word |= 1u64 << (doc64 & 0x3f);
                } else {
                    // A new block: flush what was buffered.
                    self.or_block(i4096, index_word, &words, num_longs);
                    i4096 = doc4096;
                    index_word = 1u64 << (doc64 & 0x3f);
                    num_longs = 0;
                }
                i64 = doc64;
                current_long = 1u64 << (doc & 0x3f);
            }
            doc = it.next_doc()?;
        }

        words[num_longs] = current_long;
        num_longs += 1;
        self.or_block(i4096, index_word, &words, num_longs);
        Ok(())
    }

    /// The iterator owns a clone of the bit set; mutating the original set after
    /// creating an iterator does not affect the iterator.
    pub fn iter(&self) -> SparseFixedBitSetIterator {
        SparseFixedBitSetIterator::new(self.clone())
    }

    fn insert_block(&mut self, i4096: usize, i64bit: u64, index: usize) {
        self.indices[i4096] = i64bit;
        assert!(self.bits[i4096].is_none());
        self.bits[i4096] = Some(vec![1u64 << (index & 0x3f)]);
        self.non_zero_long_count += 1;
    }

    fn insert_long(&mut self, i4096: usize, i64bit: u64, index: usize, index_word: u64) {
        self.insert_long_value(i4096, i64bit, 1u64 << (index & 0x3f), index_word);
    }

    fn insert_long_value(&mut self, i4096: usize, i64bit: u64, value: u64, index_word: u64) {
        self.indices[i4096] |= i64bit;
        let o = (index_word & (i64bit - 1)).count_ones() as usize;
        let bit_array = self.bits[i4096].as_mut().unwrap();
        let has_trailing_zero = bit_array.last().copied().unwrap_or(0) == 0;
        let bit_array_len = bit_array.len();
        if has_trailing_zero {
            bit_array.copy_within(o..bit_array_len - 1, o + 1);
            bit_array[o] = value;
        } else {
            let new_size = oversize(bit_array.len() + 1);
            let mut new_array = Vec::with_capacity(new_size);
            new_array.extend_from_slice(&bit_array[..o]);
            new_array.push(value);
            new_array.extend_from_slice(&bit_array[o..]);
            *bit_array = new_array;
        }
        self.non_zero_long_count += 1;
    }

    fn and(&mut self, i4096: usize, i64: usize, mask: u64) {
        let index_word = self.indices[i4096];
        let i64bit = 1u64 << (i64 & 0x3f);
        if (index_word & i64bit) != 0 {
            let o = (index_word & (i64bit - 1)).count_ones() as usize;
            let remaining = self.bits[i4096].as_mut().unwrap()[o] & mask;
            if remaining == 0 {
                self.remove_long(i4096, i64, index_word, o);
            } else {
                self.bits[i4096].as_mut().unwrap()[o] = remaining;
            }
        }
    }

    fn remove_long(&mut self, i4096: usize, i64: usize, mut index_word: u64, o: usize) {
        let i64bit = 1u64 << (i64 & 0x3f);
        index_word &= !i64bit;
        self.indices[i4096] = index_word;
        if index_word == 0 {
            self.bits[i4096] = None;
        } else {
            let new_len = index_word.count_ones() as usize;
            let bit_array = self.bits[i4096].as_mut().unwrap();
            bit_array.copy_within(o + 1..new_len + 1, o);
            bit_array[new_len] = 0;
        }
        self.non_zero_long_count -= 1;
    }
}

impl Bits for SparseFixedBitSet {
    fn get(&self, index: usize) -> bool {
        self.get(index)
    }

    fn length(&self) -> usize {
        self.length()
    }
}

impl SparseFixedBitSet {
    /// Shallow size of the Java class, the fixed part of
    /// [`SparseFixedBitSet::ram_bytes_used`](Accountable::ram_bytes_used).
    ///
    /// Equivalent to the private `SparseFixedBitSet.BASE_RAM_BYTES_USED`, which
    /// Java computes as
    /// `RamUsageEstimator.shallowSizeOfInstance(SparseFixedBitSet.class)`: an
    /// object header, the `indices` and `bits` references, the `int` fields
    /// `length` and `nonZeroLongCount`, and the `long ramBytesUsed` cache.
    /// Measured against `lucene-core-10.5.0.jar` on a JVM with compressed
    /// references, that is 40 bytes.
    pub const BASE_RAM_BYTES_USED: i64 = RamUsageEstimator::align_object_size(
        RamUsageEstimator::NUM_BYTES_OBJECT_HEADER
            + 2 * RamUsageEstimator::NUM_BYTES_OBJECT_REF
            + 2 * 4
            + 8,
    );
}

impl Accountable for SparseFixedBitSet {
    /// Returns an estimate of the heap this bit set occupies.
    ///
    /// Equivalent to `org.apache.lucene.util.SparseFixedBitSet#ramBytesUsed`,
    /// which answers a `ramBytesUsed` field kept up to date as blocks are
    /// allocated. Java seeds that field with
    /// `BASE_RAM_BYTES_USED + sizeOf(indices) + shallowSizeOf(bits)` in the
    /// constructor (`SparseFixedBitSet.java:71-74`) and adds `sizeOf(block)`
    /// for each block it allocates; this port recomputes the same sum on
    /// demand, which is the same value.
    fn ram_bytes_used(&self) -> i64 {
        let mut bytes = Self::BASE_RAM_BYTES_USED
            + RamUsageEstimator::size_of_u64(&self.indices)
            + RamUsageEstimator::shallow_size_of(&self.bits);
        for arr in self.bits.iter().flatten() {
            bytes += RamUsageEstimator::size_of_u64(arr);
        }
        bytes
    }
}

/// Iterator over the set bits of a [`SparseFixedBitSet`].
#[derive(Clone, Debug)]
pub struct SparseFixedBitSetIterator {
    bit_set: SparseFixedBitSet,
    doc: i32,
}

impl SparseFixedBitSetIterator {
    fn new(bit_set: SparseFixedBitSet) -> Self {
        Self { bit_set, doc: -1 }
    }
}

impl DocIdSetIterator for SparseFixedBitSetIterator {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> crate::error::Result<i32> {
        let candidate = if self.doc == -1 {
            self.bit_set.next_set_bit(0)
        } else {
            self.bit_set.next_set_bit((self.doc as usize) + 1)
        };
        self.doc = candidate.map(|c| c as i32).unwrap_or(NO_MORE_DOCS);
        Ok(self.doc)
    }

    fn advance(&mut self, target: i32) -> crate::error::Result<i32> {
        let start = target.max(0) as usize;
        self.doc = self
            .bit_set
            .next_set_bit(start)
            .map(|c| c as i32)
            .unwrap_or(NO_MORE_DOCS);
        Ok(self.doc)
    }

    fn cost(&self) -> i64 {
        self.bit_set.cardinality() as i64
    }
}

// -----------------------------------------------------------------------------
// RoaringDocIdSet
// -----------------------------------------------------------------------------

const ROARING_BLOCK_SIZE: usize = 1 << 16; // 65536
const ROARING_MAX_ARRAY_LENGTH: usize = 1 << 12; // 4096

/// Port of `org.apache.lucene.util.RoaringDocIdSet`.
///
/// A roaring-bitmap-inspired doc-id set, equivalent to Lucene's
/// `RoaringDocIdSet`.
///
/// The doc-id space is split into 64 Ki blocks. Each block is encoded with the
/// most compact representation available: an all-set sentinel, a contiguous
/// range, a sorted short array, a dense bit set, or (for nearly-full blocks) the
/// complement of a short exclusion array.
///
/// Instances are immutable after construction. Use [`RoaringDocIdSet::builder`]
/// to create one.
#[derive(Clone, Debug)]
pub struct RoaringDocIdSet {
    max_doc: usize,
    blocks: Vec<Option<RoaringBlock>>,
    cardinality: usize,
}

#[derive(Clone, Debug)]
enum RoaringBlock {
    /// Every doc in the block is present.
    All { block_len: usize },
    /// Docs in `[min, max]` (inclusive, relative to block base) are present.
    Range { min: u16, max: u16 },
    /// Sorted list of present doc offsets.
    Array(Vec<u16>),
    /// Dense representation as a bit set.
    BitSet(FixedBitSet),
    /// Nearly-full block: stores the *excluded* doc offsets.
    Not(Vec<u16>),
}

impl RoaringBlock {
    fn contains(&self, offset: u16) -> bool {
        match self {
            RoaringBlock::All { .. } => true,
            RoaringBlock::Range { min, max } => offset >= *min && offset <= *max,
            RoaringBlock::Array(docs) => docs.binary_search(&offset).is_ok(),
            RoaringBlock::BitSet(bits) => {
                let idx = offset as usize;
                idx < bits.length() && bits.get(idx)
            }
            RoaringBlock::Not(excluded) => excluded.binary_search(&offset).is_err(),
        }
    }

    fn iterator(&self, _block_base: usize) -> Box<dyn DocIdSetIterator> {
        match self {
            RoaringBlock::All { block_len } => Box::new(RoaringBlockIter::Range {
                doc: -1,
                min: 0,
                max: (*block_len - 1) as u16,
            }),
            RoaringBlock::Range { min, max } => Box::new(RoaringBlockIter::Range {
                doc: -1,
                min: *min,
                max: *max,
            }),
            RoaringBlock::Array(docs) => Box::new(RoaringBlockIter::Array {
                docs: docs.clone(),
                idx: -1,
                doc: -1,
            }),
            RoaringBlock::BitSet(bits) => Box::new(RoaringBitSetIter::new(bits.clone())),
            RoaringBlock::Not(excluded) => Box::new(RoaringBlockIter::Not {
                excluded: excluded.clone(),
                doc: -1,
                block_len: ROARING_BLOCK_SIZE,
                next_excluded_idx: 0,
            }),
        }
    }
}

#[derive(Clone, Debug)]
enum RoaringBlockIter {
    Range {
        doc: i32,
        min: u16,
        max: u16,
    },
    Array {
        docs: Vec<u16>,
        idx: i32,
        doc: i32,
    },
    Not {
        excluded: Vec<u16>,
        doc: i32,
        block_len: usize,
        next_excluded_idx: usize,
    },
}

impl DocIdSetIterator for RoaringBlockIter {
    fn doc_id(&self) -> i32 {
        self.doc()
    }

    fn next_doc(&mut self) -> crate::error::Result<i32> {
        match self {
            RoaringBlockIter::Range { doc, min, max } => {
                if *doc == -1 {
                    *doc = *min as i32;
                } else if (*doc as u16) < *max {
                    *doc += 1;
                } else {
                    *doc = NO_MORE_DOCS;
                }
                Ok(*doc)
            }
            RoaringBlockIter::Array { docs, idx, doc } => {
                *idx += 1;
                if (*idx as usize) < docs.len() {
                    *doc = docs[*idx as usize] as i32;
                } else {
                    *doc = NO_MORE_DOCS;
                }
                Ok(*doc)
            }
            RoaringBlockIter::Not {
                excluded,
                doc,
                block_len,
                next_excluded_idx,
            } => {
                let mut next = if *doc == -1 { 0 } else { (*doc as usize) + 1 };
                while next < *block_len {
                    if *next_excluded_idx < excluded.len()
                        && excluded[*next_excluded_idx] == next as u16
                    {
                        *next_excluded_idx += 1;
                        next += 1;
                        continue;
                    }
                    break;
                }
                *doc = if next < *block_len {
                    next as i32
                } else {
                    NO_MORE_DOCS
                };
                Ok(*doc)
            }
        }
    }

    fn advance(&mut self, target: i32) -> crate::error::Result<i32> {
        match self {
            RoaringBlockIter::Range { doc, min, max } => {
                if target <= *min as i32 {
                    *doc = *min as i32;
                } else if target <= *max as i32 {
                    *doc = target;
                } else {
                    *doc = NO_MORE_DOCS;
                }
                Ok(*doc)
            }
            RoaringBlockIter::Array { docs, idx, doc } => {
                let start = (*idx + 1).max(0) as usize;
                let pos = docs[start..].binary_search(&(target as u16));
                *idx = (start + pos.unwrap_or_else(|e| e)) as i32;
                if (*idx as usize) < docs.len() {
                    *doc = docs[*idx as usize] as i32;
                } else {
                    *doc = NO_MORE_DOCS;
                }
                Ok(*doc)
            }
            RoaringBlockIter::Not {
                excluded,
                doc,
                block_len,
                next_excluded_idx,
            } => {
                let mut next = target.max(0) as usize;
                while next < *block_len {
                    if *next_excluded_idx < excluded.len()
                        && excluded[*next_excluded_idx] < next as u16
                    {
                        *next_excluded_idx += 1;
                        continue;
                    }
                    if *next_excluded_idx < excluded.len()
                        && excluded[*next_excluded_idx] == next as u16
                    {
                        *next_excluded_idx += 1;
                        next += 1;
                        continue;
                    }
                    break;
                }
                *doc = if next < *block_len {
                    next as i32
                } else {
                    NO_MORE_DOCS
                };
                Ok(*doc)
            }
        }
    }

    fn cost(&self) -> i64 {
        match self {
            RoaringBlockIter::Range { min, max, .. } => (*max as i64 - *min as i64) + 1,
            RoaringBlockIter::Array { docs, .. } => docs.len() as i64,
            RoaringBlockIter::Not {
                block_len,
                excluded,
                ..
            } => (*block_len - excluded.len()) as i64,
        }
    }
}

impl RoaringBlockIter {
    fn doc(&self) -> i32 {
        match self {
            RoaringBlockIter::Range { doc, .. } => *doc,
            RoaringBlockIter::Array { doc, .. } => *doc,
            RoaringBlockIter::Not { doc, .. } => *doc,
        }
    }
}

/// Iterator over a [`FixedBitSet`] owned by a roaring block.
#[derive(Clone, Debug)]
struct RoaringBitSetIter {
    bits: FixedBitSet,
    doc: i32,
}

impl RoaringBitSetIter {
    fn new(bits: FixedBitSet) -> Self {
        Self { bits, doc: -1 }
    }
}

impl DocIdSetIterator for RoaringBitSetIter {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> crate::error::Result<i32> {
        self.advance(self.doc + 1)
    }

    fn advance(&mut self, target: i32) -> crate::error::Result<i32> {
        let start = target.max(0) as usize;
        let len = self.bits.length();
        if start >= len {
            self.doc = NO_MORE_DOCS;
            return Ok(NO_MORE_DOCS);
        }
        let words = self.bits.get_bits();
        let mut word_idx = start >> 6;
        let mut shift = start & 0x3f;
        let mut word = words[word_idx] >> shift;
        while word == 0 {
            word_idx += 1;
            if word_idx >= words.len() {
                self.doc = NO_MORE_DOCS;
                return Ok(NO_MORE_DOCS);
            }
            word = words[word_idx];
            shift = 0;
        }
        let candidate = (word_idx << 6) + shift + word.trailing_zeros() as usize;
        if candidate >= len {
            self.doc = NO_MORE_DOCS;
        } else {
            self.doc = candidate as i32;
        }
        Ok(self.doc)
    }

    fn cost(&self) -> i64 {
        self.bits.cardinality() as i64
    }
}

impl RoaringDocIdSet {
    /// Creates a builder for a `RoaringDocIdSet` that can hold doc ids in
    /// `[0, max_doc)`.
    pub fn builder(max_doc: usize) -> RoaringDocIdSetBuilder {
        RoaringDocIdSetBuilder::new(max_doc)
    }

    /// Returns the number of documents in the set.
    pub fn cardinality(&self) -> usize {
        self.cardinality
    }

    /// Returns the exclusive upper bound of doc ids.
    pub fn max_doc(&self) -> usize {
        self.max_doc
    }

    /// Returns the length of the bit space, equal to `max_doc()`.
    pub fn length(&self) -> usize {
        self.max_doc
    }

    /// Returns whether doc `index` is contained in this set.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of bounds.
    pub fn get(&self, index: usize) -> bool {
        assert!(index < self.max_doc, "index={index} out of bounds");
        let block = index >> 16;
        match &self.blocks[block] {
            None => false,
            Some(b) => b.contains((index & 0xffff) as u16),
        }
    }

    /// Returns an iterator over the doc ids in this set.
    pub fn iter(&self) -> RoaringDocIdSetIterator {
        RoaringDocIdSetIterator::new(self.clone())
    }
}

impl Bits for RoaringDocIdSet {
    fn get(&self, index: usize) -> bool {
        self.get(index)
    }

    fn length(&self) -> usize {
        self.length()
    }
}

impl Accountable for RoaringDocIdSet {
    fn ram_bytes_used(&self) -> i64 {
        let mut bytes = RamUsageEstimator::shallow_size_of(&self.blocks);
        for b in self.blocks.iter().flatten() {
            bytes += match b {
                RoaringBlock::All { .. } => 0,
                RoaringBlock::Range { .. } => 0,
                RoaringBlock::Array(arr) => {
                    RamUsageEstimator::size_of_u64(&vec![0u64; arr.len().div_ceil(4)])
                }
                RoaringBlock::BitSet(bits) => bits.ram_bytes_used(),
                RoaringBlock::Not(arr) => {
                    RamUsageEstimator::size_of_u64(&vec![0u64; arr.len().div_ceil(4)])
                }
            };
        }
        bytes
    }
}

/// Builder for [`RoaringDocIdSet`].
#[derive(Debug)]
pub struct RoaringDocIdSetBuilder {
    max_doc: usize,
    blocks: Vec<Option<RoaringBlock>>,
    cardinality: usize,
    first_doc_id: i32,
    last_doc_id: i32,
    current_block: i32,
    current_block_cardinality: usize,
    buffer: Vec<u16>,
    dense_buffer: Option<FixedBitSet>,
}

impl RoaringDocIdSetBuilder {
    fn new(max_doc: usize) -> Self {
        let num_blocks = max_doc.div_ceil(ROARING_BLOCK_SIZE);
        Self {
            max_doc,
            blocks: vec![None; num_blocks],
            cardinality: 0,
            first_doc_id: -1,
            last_doc_id: -1,
            current_block: -1,
            current_block_cardinality: 0,
            buffer: Vec::with_capacity(ROARING_MAX_ARRAY_LENGTH),
            dense_buffer: None,
        }
    }

    /// Adds a single doc id. Doc ids must be added in strictly increasing order.
    ///
    /// # Errors
    ///
    /// Returns an error if `doc_id` is out of order or out of bounds.
    pub fn add(&mut self, doc_id: usize) -> crate::error::Result<()> {
        if doc_id as i32 <= self.last_doc_id {
            return Err(crate::error::LuceneError::IllegalArgument(format!(
                "doc ids must be added in-order, got {doc_id} which is <= lastDocId={}",
                self.last_doc_id
            )));
        }
        if doc_id >= self.max_doc {
            return Err(crate::error::LuceneError::IllegalArgument(format!(
                "doc_id={doc_id} is >= maxDoc={}",
                self.max_doc
            )));
        }
        let block = (doc_id >> 16) as i32;
        if block != self.current_block {
            self.flush();
            self.current_block = block;
            self.first_doc_id = doc_id as i32;
        }
        self.append_doc_in_current_block(doc_id, block as usize);
        Ok(())
    }

    /// Adds all doc ids in the half-open range `[min, max)`.
    ///
    /// # Errors
    ///
    /// Returns an error if the range is invalid or out of order.
    pub fn add_range(&mut self, min: usize, max: usize) -> crate::error::Result<()> {
        if min > max {
            return Err(crate::error::LuceneError::IllegalArgument(format!(
                "min must be <= max, got min={min} max={max}"
            )));
        }
        if min == max {
            return Ok(());
        }
        if min as i32 <= self.last_doc_id {
            return Err(crate::error::LuceneError::IllegalArgument(format!(
                "doc ids must be added in-order, got range starting at {min} which is <= lastDocId={}",
                self.last_doc_id
            )));
        }
        if max > self.max_doc {
            return Err(crate::error::LuceneError::IllegalArgument(format!(
                "max={max} exceeds maxDoc={}",
                self.max_doc
            )));
        }
        let mut doc = min;
        while doc < max {
            let block = doc >> 16;
            let block_end = max.min((block + 1) << 16);
            if block as i32 != self.current_block {
                self.flush();
                self.current_block = block as i32;
                self.first_doc_id = doc as i32;
            }
            self.append_range_in_current_block(doc, block_end, block);
            doc = block_end;
        }
        Ok(())
    }

    /// Builds the immutable `RoaringDocIdSet`.
    pub fn build(mut self) -> RoaringDocIdSet {
        self.flush();
        RoaringDocIdSet {
            max_doc: self.max_doc,
            blocks: self.blocks,
            cardinality: self.cardinality,
        }
    }

    fn flush(&mut self) {
        if self.current_block < 0 || self.current_block_cardinality == 0 {
            self.reset_current_block();
            return;
        }
        let block = self.current_block as usize;
        let block_len = self.block_len(block);
        let block_data = if self.current_block_cardinality == block_len {
            Some(RoaringBlock::All { block_len })
        } else if self.current_block_cardinality
            == ((self.last_doc_id - self.first_doc_id) as usize) + 1
        {
            Some(RoaringBlock::Range {
                min: (self.first_doc_id & 0xffff) as u16,
                max: (self.last_doc_id & 0xffff) as u16,
            })
        } else if self.current_block_cardinality <= ROARING_MAX_ARRAY_LENGTH {
            assert!(self.dense_buffer.is_none());
            let mut docs = Vec::with_capacity(self.current_block_cardinality);
            docs.extend_from_slice(&self.buffer[..self.current_block_cardinality]);
            docs.sort_unstable();
            Some(RoaringBlock::Array(docs))
        } else {
            let dense = self
                .dense_buffer
                .take()
                .expect("INVARIANT: dense buffer exists");
            assert_eq!(dense.cardinality(), self.current_block_cardinality);
            if dense.length() == ROARING_BLOCK_SIZE
                && ROARING_BLOCK_SIZE - self.current_block_cardinality < ROARING_MAX_ARRAY_LENGTH
            {
                let mut excluded =
                    Vec::with_capacity(ROARING_BLOCK_SIZE - self.current_block_cardinality);
                for offset in 0..ROARING_BLOCK_SIZE {
                    if !dense.get(offset) {
                        excluded.push(offset as u16);
                    }
                }
                Some(RoaringBlock::Not(excluded))
            } else {
                Some(RoaringBlock::BitSet(dense))
            }
        };
        self.blocks[block] = block_data;
        self.reset_current_block();
    }

    fn reset_current_block(&mut self) {
        self.cardinality += self.current_block_cardinality;
        self.dense_buffer = None;
        self.buffer.clear();
        self.current_block_cardinality = 0;
    }

    fn append_doc_in_current_block(&mut self, doc_id: usize, block: usize) {
        let offset = doc_id - (block << 16);
        if self.current_block_cardinality < ROARING_MAX_ARRAY_LENGTH {
            self.buffer.push(offset as u16);
        } else {
            if self.dense_buffer.is_none() {
                let num_bits = ((block + 1) * ROARING_BLOCK_SIZE).min(self.max_doc)
                    - (block * ROARING_BLOCK_SIZE);
                let mut dense = FixedBitSet::new(num_bits);
                for &d in &self.buffer {
                    dense.set(d as usize);
                }
                self.dense_buffer = Some(dense);
            }
            self.dense_buffer.as_mut().unwrap().set(offset);
        }
        self.last_doc_id = doc_id as i32;
        self.current_block_cardinality += 1;
    }

    fn append_range_in_current_block(
        &mut self,
        from_doc: usize,
        to_doc_exclusive: usize,
        block: usize,
    ) {
        let offset = block << 16;
        let span = to_doc_exclusive - from_doc;
        if self.current_block_cardinality + span <= ROARING_MAX_ARRAY_LENGTH {
            for d in from_doc..to_doc_exclusive {
                self.buffer.push((d & 0xffff) as u16);
            }
        } else {
            if self.dense_buffer.is_none() {
                let num_bits = ((block + 1) << 16).min(self.max_doc) - offset;
                let mut dense = FixedBitSet::new(num_bits);
                for &d in &self.buffer {
                    dense.set(d as usize);
                }
                self.dense_buffer = Some(dense);
            }
            let start = from_doc - offset;
            let end = to_doc_exclusive - offset;
            for d in start..end {
                self.dense_buffer.as_mut().unwrap().set(d);
            }
        }
        self.last_doc_id = (to_doc_exclusive - 1) as i32;
        self.current_block_cardinality += span;
    }

    fn block_len(&self, block: usize) -> usize {
        let end = ((block + 1) * ROARING_BLOCK_SIZE).min(self.max_doc);
        end - (block * ROARING_BLOCK_SIZE)
    }
}

/// Iterator over the doc ids of a [`RoaringDocIdSet`].
pub struct RoaringDocIdSetIterator {
    set: RoaringDocIdSet,
    block: i32,
    sub: Option<Box<dyn DocIdSetIterator>>,
    doc: i32,
}

impl fmt::Debug for RoaringDocIdSetIterator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RoaringDocIdSetIterator")
            .field("max_doc", &self.set.max_doc)
            .field("block", &self.block)
            .field("doc", &self.doc)
            .finish()
    }
}

impl RoaringDocIdSetIterator {
    fn new(set: RoaringDocIdSet) -> Self {
        Self {
            set,
            block: -1,
            sub: None,
            doc: -1,
        }
    }

    fn first_doc_from_next_block(&mut self) -> crate::error::Result<i32> {
        loop {
            self.block += 1;
            if self.block as usize >= self.set.blocks.len() {
                self.sub = None;
                self.doc = NO_MORE_DOCS;
                return Ok(NO_MORE_DOCS);
            }
            if let Some(b) = &self.set.blocks[self.block as usize] {
                let block_base = (self.block as usize) << 16;
                let mut sub = b.iterator(block_base);
                let first = sub.next_doc()?;
                assert_ne!(first, NO_MORE_DOCS);
                self.sub = Some(sub);
                self.doc = (block_base as i32) | first;
                return Ok(self.doc);
            }
        }
    }
}

impl DocIdSetIterator for RoaringDocIdSetIterator {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> crate::error::Result<i32> {
        if let Some(sub) = self.sub.as_mut() {
            let sub_next = sub.next_doc()?;
            if sub_next != NO_MORE_DOCS {
                self.doc = ((self.block as usize) << 16) as i32 | sub_next;
                return Ok(self.doc);
            }
        }
        self.first_doc_from_next_block()
    }

    fn advance(&mut self, target: i32) -> crate::error::Result<i32> {
        let target_block = (target.max(0) as usize) >> 16;
        if target_block as i32 != self.block {
            self.block = target_block as i32;
            if self.block as usize >= self.set.blocks.len() {
                self.sub = None;
                self.doc = NO_MORE_DOCS;
                return Ok(NO_MORE_DOCS);
            }
            if self.set.blocks[self.block as usize].is_none() {
                return self.first_doc_from_next_block();
            }
            let block_base = (self.block as usize) << 16;
            self.sub = Some(
                self.set.blocks[self.block as usize]
                    .as_ref()
                    .unwrap()
                    .iterator(block_base),
            );
        }
        if let Some(sub) = self.sub.as_mut() {
            let sub_target = (target as usize) & 0xffff;
            let sub_next = sub.advance(sub_target as i32)?;
            if sub_next != NO_MORE_DOCS {
                self.doc = ((self.block as usize) << 16) as i32 | sub_next;
                return Ok(self.doc);
            }
        }
        self.first_doc_from_next_block()
    }

    fn cost(&self) -> i64 {
        self.set.cardinality as i64
    }
}

// -----------------------------------------------------------------------------
// FilteredDocIdSetIterator
// -----------------------------------------------------------------------------

/// Port of `org.apache.lucene.util.FilteredDocIdSetIterator`.
///
/// A [`DocIdSetIterator`] that returns the documents matching a predicate by
/// scanning every document position. It wraps no inner iterator, so iteration
/// has `O(max_doc)` complexity, and it can walk either the live or the deleted
/// documents depending on the predicate it is given.
///
/// Not to be confused with `org.apache.lucene.search.FilteredDocIdSetIterator`,
/// an unrelated class of the same simple name: that one is a public abstract
/// decorator over an *inner* iterator, with `getDelegate()`/`match(int)` and a
/// delegated `cost()`, and is ported separately in
/// [`crate::search::doc_id_set_iterator`].
///
/// Lucene declares the Java class package-private within
/// `org.apache.lucene.util` and marks it `@lucene.internal`. `pub(in
/// crate::util)` is the closest Rust equivalent, confining it to the port of
/// that same package.
pub(in crate::util) struct FilteredDocIdSetIterator {
    max_doc: i32,
    cost: i32,
    predicate: Box<dyn Fn(i32) -> bool>,
    doc: i32,
}

impl FilteredDocIdSetIterator {
    /// Creates a filtered iterator over the documents in `[0, max_doc)`.
    ///
    /// `cost` is the expected number of documents to return, reported verbatim
    /// by [`DocIdSetIterator::cost`], and `predicate` returns `true` for the
    /// documents to include in the iteration.
    ///
    /// Equivalent to
    /// `FilteredDocIdSetIterator(int maxDoc, int cost, IntPredicate predicate)`.
    ///
    /// In Lucene 10.5.0 this type has exactly two callers, both in the same
    /// package: `DenseLiveDocs.deletedDocsIterator()` (`DenseLiveDocs.java:139`,
    /// predicate `doc -> !liveDocs.get(doc)`) and
    /// `SparseLiveDocs.liveDocsIterator()` (`SparseLiveDocs.java:133`,
    /// predicate `doc -> !deletedDocs.get(doc)`). Both are reachable here:
    /// [`DenseLiveDocs`] and [`SparseLiveDocs`] hold their bit set behind an
    /// [`Arc`], so the predicate closure captures a reference-counted handle
    /// instead of borrowing out of `&self`. That satisfies the implicit
    /// `'static` bound of the `Box<dyn DocIdSetIterator>` that
    /// [`crate::util::bit_set::LiveDocs`] returns, and costs one atomic
    /// increment per call rather than a copy of the bit set — Java's lambda
    /// capture of the field reference costs the same.
    pub(in crate::util) fn new(
        max_doc: i32,
        cost: i32,
        predicate: Box<dyn Fn(i32) -> bool>,
    ) -> Self {
        Self {
            max_doc,
            cost,
            predicate,
            doc: -1,
        }
    }
}

impl DocIdSetIterator for FilteredDocIdSetIterator {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> crate::error::Result<i32> {
        self.advance(self.doc + 1)
    }

    fn advance(&mut self, target: i32) -> crate::error::Result<i32> {
        if target >= self.max_doc {
            self.doc = NO_MORE_DOCS;
            return Ok(self.doc);
        }

        self.doc = target;
        while self.doc < self.max_doc {
            if (self.predicate)(self.doc) {
                return Ok(self.doc);
            }
            self.doc += 1;
        }
        self.doc = NO_MORE_DOCS;
        Ok(self.doc)
    }

    fn cost(&self) -> i64 {
        i64::from(self.cost)
    }
}

// -----------------------------------------------------------------------------
// DenseLiveDocs
// -----------------------------------------------------------------------------

/// Live-docs representation optimized for dense deletions.
///
/// Set bits in the wrapped [`FixedBitSet`] represent **live** documents, which is
/// the traditional Lucene semantics. Instances are immutable after construction.
/// Equivalent to `org.apache.lucene.util.DenseLiveDocs`.
#[derive(Clone, Debug)]
pub struct DenseLiveDocs {
    /// Java holds a plain `FixedBitSet` field and lets `liveDocsIterator()` and
    /// the `deletedDocsIterator()` lambda capture a reference to it. Rust's
    /// [`LiveDocs`] contract hands back a `Box<dyn DocIdSetIterator>`, whose
    /// implicit `'static` bound forbids borrowing out of `&self`, so the set is
    /// held behind an [`Arc`]: the iterators capture a reference-counted handle
    /// in `O(1)`, which is what Java's reference capture costs too.
    live_docs: Arc<FixedBitSet>,
    max_doc: usize,
    deleted_count: usize,
}

impl DenseLiveDocs {
    /// Creates a builder for constructing `DenseLiveDocs` instances.
    pub fn builder(live_docs: FixedBitSet, max_doc: usize) -> DenseLiveDocsBuilder {
        DenseLiveDocsBuilder::new(live_docs, max_doc)
    }

    /// Returns the live-docs bit set.
    pub fn live_docs(&self) -> &FixedBitSet {
        &self.live_docs
    }

    /// Returns the number of deleted documents.
    pub fn deleted_count(&self) -> usize {
        self.deleted_count
    }

    /// Returns the memory usage in bytes.
    pub fn ram_bytes_used(&self) -> i64 {
        self.live_docs.ram_bytes_used()
    }
}

impl Bits for DenseLiveDocs {
    fn get(&self, index: usize) -> bool {
        assert!(index < self.max_doc, "index={index} out of bounds");
        self.live_docs.get(index)
    }

    fn length(&self) -> usize {
        self.max_doc
    }

    fn into_live_docs(self: Box<Self>) -> Option<Box<dyn LiveDocs>> {
        Some(self)
    }
}

impl LiveDocs for DenseLiveDocs {
    /// Iterates the live documents by walking the set bits of the live-docs
    /// bit set directly.
    ///
    /// Equivalent to `DenseLiveDocs.liveDocsIterator()`, which builds a
    /// `BitSetIterator(liveDocs, maxDoc - deletedCount)`.
    fn live_docs_iterator(&self) -> crate::error::Result<Box<dyn DocIdSetIterator>> {
        let bits: Arc<dyn BitSet> = self.live_docs.clone();
        Ok(Box::new(BitSetIterator::new(
            bits,
            (self.max_doc - self.deleted_count) as i64,
        )?))
    }

    /// Iterates the deleted documents by scanning `[0, max_doc)` and keeping
    /// the documents whose live bit is clear.
    ///
    /// Equivalent to `DenseLiveDocs.deletedDocsIterator()`, which builds a
    /// `FilteredDocIdSetIterator(maxDoc, deletedCount, doc -> !liveDocs.get(doc))`.
    fn deleted_docs_iterator(&self) -> crate::error::Result<Box<dyn DocIdSetIterator>> {
        let live_docs = Arc::clone(&self.live_docs);
        Ok(Box::new(FilteredDocIdSetIterator::new(
            self.max_doc as i32,
            self.deleted_count as i32,
            Box::new(move |doc| !live_docs.get(doc as usize)),
        )))
    }

    fn deleted_count(&self) -> i32 {
        self.deleted_count as i32
    }
}

/// Builder for [`DenseLiveDocs`].
#[derive(Debug)]
pub struct DenseLiveDocsBuilder {
    live_docs: FixedBitSet,
    max_doc: usize,
    deleted_count: Option<usize>,
}

impl DenseLiveDocsBuilder {
    fn new(live_docs: FixedBitSet, max_doc: usize) -> Self {
        Self {
            live_docs,
            max_doc,
            deleted_count: None,
        }
    }

    /// Sets the pre-computed deleted document count.
    pub fn with_deleted_count(mut self, deleted_count: usize) -> Self {
        self.deleted_count = Some(deleted_count);
        self
    }

    /// Builds the `DenseLiveDocs` instance.
    ///
    /// # Panics
    ///
    /// Panics if the deleted count is outside `[0, max_doc]` or does not match
    /// `max_doc - live_docs.cardinality()`.
    pub fn build(self) -> DenseLiveDocs {
        let count = self
            .deleted_count
            .unwrap_or_else(|| self.max_doc - self.live_docs.cardinality());
        assert!(
            count <= self.max_doc,
            "deletedCount={count} is outside valid range [0, {}]",
            self.max_doc
        );
        assert_eq!(
            count,
            self.max_doc - self.live_docs.cardinality(),
            "deletedCount does not match maxDoc - liveDocs.cardinality()"
        );
        DenseLiveDocs {
            live_docs: Arc::new(self.live_docs),
            max_doc: self.max_doc,
            deleted_count: count,
        }
    }
}

// -----------------------------------------------------------------------------
// SparseLiveDocs
// -----------------------------------------------------------------------------

/// Live-docs representation optimized for sparse deletions.
///
/// Set bits in the wrapped [`SparseFixedBitSet`] represent **deleted**
/// documents, so `get(doc)` returns `true` when the document is live. Instances
/// are immutable after construction. Equivalent to
/// `org.apache.lucene.util.SparseLiveDocs`.
#[derive(Clone, Debug)]
pub struct SparseLiveDocs {
    /// Held behind an [`Arc`] for the same reason as
    /// [`DenseLiveDocs::live_docs`]: the [`LiveDocs`] iterators must own what
    /// they read.
    deleted_docs: Arc<SparseFixedBitSet>,
    max_doc: usize,
    deleted_count: usize,
}

impl SparseLiveDocs {
    /// Creates a builder for constructing `SparseLiveDocs` instances.
    pub fn builder(deleted_docs: SparseFixedBitSet, max_doc: usize) -> SparseLiveDocsBuilder {
        SparseLiveDocsBuilder::new(deleted_docs, max_doc)
    }

    /// Returns the deleted-docs bit set.
    pub fn deleted_docs(&self) -> &SparseFixedBitSet {
        &self.deleted_docs
    }

    /// Returns the number of deleted documents.
    pub fn deleted_count(&self) -> usize {
        self.deleted_count
    }

    /// Returns the memory usage in bytes.
    pub fn ram_bytes_used(&self) -> i64 {
        self.deleted_docs.ram_bytes_used()
    }
}

impl Bits for SparseLiveDocs {
    fn get(&self, index: usize) -> bool {
        assert!(index < self.max_doc, "index={index} out of bounds");
        !self.deleted_docs.get(index)
    }

    fn length(&self) -> usize {
        self.max_doc
    }

    fn into_live_docs(self: Box<Self>) -> Option<Box<dyn LiveDocs>> {
        Some(self)
    }
}

impl LiveDocs for SparseLiveDocs {
    /// Iterates the live documents by scanning `[0, max_doc)` and keeping the
    /// documents whose deleted bit is clear.
    ///
    /// Equivalent to `SparseLiveDocs.liveDocsIterator()`, which builds a
    /// `FilteredDocIdSetIterator(maxDoc, maxDoc - deletedCount, doc -> !deletedDocs.get(doc))`.
    fn live_docs_iterator(&self) -> crate::error::Result<Box<dyn DocIdSetIterator>> {
        let deleted_docs = Arc::clone(&self.deleted_docs);
        Ok(Box::new(FilteredDocIdSetIterator::new(
            self.max_doc as i32,
            (self.max_doc - self.deleted_count) as i32,
            Box::new(move |doc| !deleted_docs.get(doc as usize)),
        )))
    }

    /// Iterates the deleted documents by walking the set bits of the
    /// deleted-docs bit set directly.
    ///
    /// Equivalent to `SparseLiveDocs.deletedDocsIterator()`, which builds a
    /// `BitSetIterator(deletedDocs, deletedCount)`.
    fn deleted_docs_iterator(&self) -> crate::error::Result<Box<dyn DocIdSetIterator>> {
        let bits: Arc<dyn BitSet> = self.deleted_docs.clone();
        Ok(Box::new(BitSetIterator::new(
            bits,
            self.deleted_count as i64,
        )?))
    }

    fn deleted_count(&self) -> i32 {
        self.deleted_count as i32
    }
}

/// Builder for [`SparseLiveDocs`].
#[derive(Debug)]
pub struct SparseLiveDocsBuilder {
    deleted_docs: SparseFixedBitSet,
    max_doc: usize,
    deleted_count: Option<usize>,
}

impl SparseLiveDocsBuilder {
    fn new(deleted_docs: SparseFixedBitSet, max_doc: usize) -> Self {
        Self {
            deleted_docs,
            max_doc,
            deleted_count: None,
        }
    }

    /// Sets the pre-computed deleted document count.
    pub fn with_deleted_count(mut self, deleted_count: usize) -> Self {
        self.deleted_count = Some(deleted_count);
        self
    }

    /// Builds the `SparseLiveDocs` instance.
    ///
    /// # Panics
    ///
    /// Panics if the deleted count is outside `[0, max_doc]` or does not match
    /// `deleted_docs.cardinality()`.
    pub fn build(self) -> SparseLiveDocs {
        let count = self
            .deleted_count
            .unwrap_or_else(|| self.deleted_docs.cardinality());
        assert!(
            count <= self.max_doc,
            "deletedCount={count} is outside valid range [0, {}]",
            self.max_doc
        );
        assert_eq!(
            count,
            self.deleted_docs.cardinality(),
            "deletedCount does not match deletedDocs.cardinality()"
        );
        SparseLiveDocs {
            deleted_docs: Arc::new(self.deleted_docs),
            max_doc: self.max_doc,
            deleted_count: count,
        }
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::NO_MORE_DOCS;

    /// A tiny deterministic LCG for reproducible pseudo-random tests without a
    /// dependency on an external random crate.
    struct TestRng {
        state: u64,
    }

    impl TestRng {
        fn new(seed: u64) -> Self {
            Self { state: seed }
        }

        fn next(&mut self) -> u64 {
            self.state = self.state.wrapping_mul(6364136223846793005).wrapping_add(1);
            self.state
        }

        fn next_in_range(&mut self, max: usize) -> usize {
            (self.next() as usize) % max.max(1)
        }
    }

    fn collect_iterator(mut it: impl DocIdSetIterator) -> Vec<i32> {
        let mut docs = Vec::new();
        loop {
            let doc = it.next_doc().unwrap();
            if doc == NO_MORE_DOCS {
                break;
            }
            docs.push(doc);
        }
        docs
    }

    #[test]
    fn sparse_fixed_bit_set_basic() {
        let mut s = SparseFixedBitSet::new(10_000);
        assert_eq!(s.length(), 10_000);
        assert_eq!(s.cardinality(), 0);

        s.set(5);
        s.set(7);
        s.set(63);
        s.set(64);
        s.set(4095);
        s.set(4096);
        s.set(8191);

        assert!(s.get(5));
        assert!(s.get(7));
        assert!(s.get(63));
        assert!(s.get(64));
        assert!(!s.get(62));
        assert!(s.get(4095));
        assert!(s.get(4096));
        assert!(s.get(8191));
        assert!(!s.get(8192));

        s.clear(7);
        assert!(!s.get(7));
        assert_eq!(s.cardinality(), 6);

        s.clear_all();
        assert_eq!(s.cardinality(), 0);
        assert!(!s.get(5));
    }

    #[test]
    fn sparse_fixed_bit_set_next_set_bit() {
        let mut s = SparseFixedBitSet::new(200);
        s.set(5);
        s.set(67);
        s.set(128);
        s.set(199);

        assert_eq!(s.next_set_bit(0), Some(5));
        assert_eq!(s.next_set_bit(5), Some(5));
        assert_eq!(s.next_set_bit(6), Some(67));
        assert_eq!(s.next_set_bit(68), Some(128));
        assert_eq!(s.next_set_bit(129), Some(199));
        assert_eq!(s.next_set_bit(200), None);
    }

    #[test]
    fn sparse_fixed_bit_set_iterator() {
        let mut s = SparseFixedBitSet::new(100);
        s.set(3);
        s.set(5);
        s.set(64);
        s.set(99);

        let docs = collect_iterator(s.iter());
        assert_eq!(docs, vec![3, 5, 64, 99]);
    }

    #[test]
    fn sparse_fixed_bit_set_matches_fixed_bit_set_random() {
        let mut rng = TestRng::new(12345);
        let max_doc = 100_000;
        let num_docs = 5_000;

        let mut reference = FixedBitSet::new(max_doc);
        let mut sparse = SparseFixedBitSet::new(max_doc);

        let mut docs = Vec::with_capacity(num_docs);
        for _ in 0..num_docs {
            let d = rng.next_in_range(max_doc);
            docs.push(d);
        }
        docs.sort_unstable();
        docs.dedup();

        for &d in &docs {
            reference.set(d);
            sparse.set(d);
        }

        assert_eq!(sparse.cardinality(), reference.cardinality());
        for d in 0..max_doc {
            assert_eq!(sparse.get(d), reference.get(d), "mismatch at {d}");
        }

        // Compare iterator output.
        let mut reference_docs = Vec::new();
        for d in 0..max_doc {
            if reference.get(d) {
                reference_docs.push(d as i32);
            }
        }
        assert_eq!(collect_iterator(sparse.iter()), reference_docs);

        // Compare after clearing half of the docs.
        for &d in docs.iter().take(docs.len() / 2) {
            reference.clear(d);
            sparse.clear(d);
        }
        assert_eq!(sparse.cardinality(), reference.cardinality());
        for d in 0..max_doc {
            assert_eq!(
                sparse.get(d),
                reference.get(d),
                "mismatch after clear at {d}"
            );
        }
    }

    #[test]
    fn roaring_doc_id_set_basic() {
        let mut builder = RoaringDocIdSet::builder(100);
        builder.add(0).unwrap();
        builder.add(5).unwrap();
        builder.add(63).unwrap();
        builder.add(64).unwrap();
        builder.add(99).unwrap();
        let set = builder.build();

        assert!(set.get(0));
        assert!(set.get(5));
        assert!(set.get(63));
        assert!(set.get(64));
        assert!(!set.get(62));
        assert!(set.get(99));
        assert!(!set.get(98));
        assert_eq!(set.cardinality(), 5);

        let docs = collect_iterator(set.iter());
        assert_eq!(docs, vec![0, 5, 63, 64, 99]);
    }

    #[test]
    fn roaring_doc_id_set_range_add() {
        let mut builder = RoaringDocIdSet::builder(200);
        builder.add_range(10, 20).unwrap();
        builder.add_range(50, 55).unwrap();
        builder.add(70).unwrap();
        let set = builder.build();

        for d in 10..20 {
            assert!(set.get(d));
        }
        for d in 50..55 {
            assert!(set.get(d));
        }
        assert!(set.get(70));
        assert!(!set.get(0));
        assert!(!set.get(20));
        assert_eq!(set.cardinality(), 10 + 5 + 1);
    }

    #[test]
    fn roaring_doc_id_set_matches_fixed_bit_set_random() {
        let mut rng = TestRng::new(54321);
        let max_doc = 200_000;
        let num_docs = 20_000;

        let mut reference = FixedBitSet::new(max_doc);
        let mut builder = RoaringDocIdSet::builder(max_doc);

        let mut docs = Vec::with_capacity(num_docs);
        for _ in 0..num_docs {
            let d = rng.next_in_range(max_doc);
            docs.push(d);
        }
        docs.sort_unstable();
        docs.dedup();

        for &d in &docs {
            reference.set(d);
            builder.add(d).unwrap();
        }

        let set = builder.build();
        assert_eq!(set.cardinality(), reference.cardinality());
        for d in 0..max_doc {
            assert_eq!(set.get(d), reference.get(d), "mismatch at {d}");
        }

        // Iterator matches reference.
        let mut reference_docs = Vec::new();
        for d in 0..max_doc {
            if reference.get(d) {
                reference_docs.push(d as i32);
            }
        }
        assert_eq!(collect_iterator(set.iter()), reference_docs);

        // advance() matches.
        let mut it = set.iter();
        assert_eq!(it.advance(0).unwrap(), reference_docs[0]);
        if reference_docs.len() > 5 {
            let mid = reference_docs[reference_docs.len() / 2];
            assert_eq!(it.advance(mid).unwrap(), mid);
        }
        assert_eq!(it.advance(max_doc as i32).unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn dense_live_docs_basic() {
        let mut bits = FixedBitSet::new(10);
        bits.set(0);
        bits.set(2);
        bits.set(4);
        bits.set(6);
        bits.set(8);

        let live = DenseLiveDocs::builder(bits, 10).build();
        assert_eq!(live.length(), 10);
        assert_eq!(live.deleted_count(), 5);
        assert!(live.get(0));
        assert!(!live.get(1));
        assert!(live.get(8));
        assert!(!live.get(9));
    }

    #[test]
    fn sparse_live_docs_basic() {
        let mut deleted = SparseFixedBitSet::new(10);
        deleted.set(1);
        deleted.set(3);
        deleted.set(5);
        deleted.set(7);
        deleted.set(9);

        let live = SparseLiveDocs::builder(deleted, 10).build();
        assert_eq!(live.length(), 10);
        assert_eq!(live.deleted_count(), 5);
        assert!(live.get(0));
        assert!(!live.get(1));
        assert!(live.get(8));
        assert!(!live.get(9));
    }

    #[test]
    fn dense_and_sparse_live_docs_match_fixed_bit_set_random() {
        let mut rng = TestRng::new(98765);
        let max_doc = 50_000;

        // Build a reference of live docs (set = live).
        let mut reference_live = FixedBitSet::new(max_doc);
        let mut deleted_count = 0;
        for d in 0..max_doc {
            if rng.next() % 5 == 0 {
                deleted_count += 1;
            } else {
                reference_live.set(d);
            }
        }

        // DenseLiveDocs mirrors reference directly.
        let dense = DenseLiveDocs::builder(reference_live.clone(), max_doc)
            .with_deleted_count(deleted_count)
            .build();

        // SparseLiveDocs stores deleted docs.
        let mut sparse_deleted = SparseFixedBitSet::new(max_doc);
        for d in 0..max_doc {
            if !reference_live.get(d) {
                sparse_deleted.set(d);
            }
        }
        let sparse = SparseLiveDocs::builder(sparse_deleted, max_doc)
            .with_deleted_count(deleted_count)
            .build();

        assert_eq!(dense.length(), max_doc);
        assert_eq!(sparse.length(), max_doc);
        assert_eq!(dense.deleted_count(), deleted_count);
        assert_eq!(sparse.deleted_count(), deleted_count);

        for d in 0..max_doc {
            assert_eq!(dense.get(d), reference_live.get(d), "dense mismatch at {d}");
            assert_eq!(
                sparse.get(d),
                reference_live.get(d),
                "sparse mismatch at {d}"
            );
        }
    }

    // -------------------------------------------------------------------------
    // FilteredDocIdSetIterator (org.apache.lucene.util)
    // -------------------------------------------------------------------------

    #[test]
    fn filtered_doc_id_set_iterator_next_doc_and_terminal_values() {
        let mut it = FilteredDocIdSetIterator::new(8, 4, Box::new(|doc| doc % 2 == 0));
        assert_eq!(it.doc_id(), -1, "docID must be -1 before the first call");

        let mut seen = Vec::new();
        loop {
            let doc = it.next_doc().unwrap();
            if doc == NO_MORE_DOCS {
                break;
            }
            seen.push(doc);
        }

        assert_eq!(seen, vec![0, 2, 4, 6]);
        assert_eq!(
            it.doc_id(),
            NO_MORE_DOCS,
            "docID must stick at NO_MORE_DOCS"
        );
    }

    #[test]
    fn filtered_doc_id_set_iterator_advance() {
        let matches = |doc: i32| matches!(doc, 2 | 5 | 6);

        // advance() is inclusive: it lands on the target when the target matches.
        let mut on_target = FilteredDocIdSetIterator::new(10, 3, Box::new(matches));
        assert_eq!(on_target.advance(2).unwrap(), 2);
        assert_eq!(on_target.doc_id(), 2);
        // ... and otherwise scans forward to the next matching document.
        assert_eq!(on_target.advance(3).unwrap(), 5);
        assert_eq!(on_target.next_doc().unwrap(), 6);

        let mut from_zero = FilteredDocIdSetIterator::new(10, 3, Box::new(matches));
        assert_eq!(from_zero.advance(0).unwrap(), 2);

        // A target at or beyond max_doc exhausts the iterator immediately.
        let mut past_end = FilteredDocIdSetIterator::new(10, 3, Box::new(matches));
        assert_eq!(past_end.advance(10).unwrap(), NO_MORE_DOCS);
        assert_eq!(past_end.doc_id(), NO_MORE_DOCS);

        // So does a target with no match at or after it.
        let mut no_match_after = FilteredDocIdSetIterator::new(10, 3, Box::new(matches));
        assert_eq!(no_match_after.advance(7).unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn filtered_doc_id_set_iterator_rejects_ignoring_the_predicate() {
        // Regression guard (CLAUDE.md §9): every assertion here fails if the
        // predicate is dropped and the iterator degenerates into a plain scan
        // of [0, max_doc).
        let max_doc = 9;

        // Doc 0 deliberately does not match, so an unfiltered scan would answer
        // 0 where Lucene answers 1.
        let mut it = FilteredDocIdSetIterator::new(max_doc, 3, Box::new(|doc| doc % 3 == 1));
        assert_eq!(it.next_doc().unwrap(), 1);
        assert_eq!(it.next_doc().unwrap(), 4);
        assert_eq!(it.advance(5).unwrap(), 7);
        assert_eq!(it.next_doc().unwrap(), NO_MORE_DOCS);

        // The sharpest case: a predicate that never matches must consume the
        // whole range and report NO_MORE_DOCS, not return doc 0.
        let mut never = FilteredDocIdSetIterator::new(max_doc, 0, Box::new(|_| false));
        assert_eq!(never.next_doc().unwrap(), NO_MORE_DOCS);
        assert_eq!(never.doc_id(), NO_MORE_DOCS);
        let mut never_advance = FilteredDocIdSetIterator::new(max_doc, 0, Box::new(|_| false));
        assert_eq!(never_advance.advance(0).unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn filtered_doc_id_set_iterator_cost_is_reported_verbatim() {
        // Java stores `cost` and widens it to long; it is an estimate supplied
        // by the caller and is never recomputed from the predicate.
        let generous = FilteredDocIdSetIterator::new(1_000, 7, Box::new(|_| true));
        assert_eq!(generous.cost(), 7);

        let empty = FilteredDocIdSetIterator::new(0, 0, Box::new(|_| true));
        assert_eq!(empty.cost(), 0);
    }

    #[test]
    fn filtered_doc_id_set_iterator_range_edges() {
        // An empty range is exhausted straight away.
        let mut empty = FilteredDocIdSetIterator::new(0, 0, Box::new(|_| true));
        assert_eq!(empty.doc_id(), -1);
        assert_eq!(empty.next_doc().unwrap(), NO_MORE_DOCS);

        // The last document in range is reachable; max_doc itself is not.
        let mut last = FilteredDocIdSetIterator::new(4, 1, Box::new(|doc| doc == 3));
        assert_eq!(last.next_doc().unwrap(), 3);
        assert_eq!(last.next_doc().unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn filtered_doc_id_set_iterator_reproduces_the_two_lucene_call_sites() {
        // Portability test (CLAUDE.md §14.3) for the only two callers in
        // Lucene 10.5.0:
        //   DenseLiveDocs.java:139
        //     new FilteredDocIdSetIterator(maxDoc, deletedCount, doc -> !liveDocs.get(doc))
        //   SparseLiveDocs.java:133
        //     new FilteredDocIdSetIterator(maxDoc, maxDoc - deletedCount, doc -> !deletedDocs.get(doc))
        let max_doc = 10usize;
        let deleted = [1usize, 3, 7];
        let deleted_count = deleted.len();

        let mut live_bits = FixedBitSet::new(max_doc);
        for d in 0..max_doc {
            if !deleted.contains(&d) {
                live_bits.set(d);
            }
        }

        // Dense side: the bit set stores LIVE docs, so the scan yields the
        // deleted ones.
        let dense_probe = live_bits.clone();
        let mut deleted_iter = FilteredDocIdSetIterator::new(
            max_doc as i32,
            deleted_count as i32,
            Box::new(move |doc| !dense_probe.get(doc as usize)),
        );
        assert_eq!(deleted_iter.cost(), deleted_count as i64);
        let mut got_deleted = Vec::new();
        loop {
            let doc = deleted_iter.next_doc().unwrap();
            if doc == NO_MORE_DOCS {
                break;
            }
            got_deleted.push(doc as usize);
        }
        assert_eq!(got_deleted, deleted.to_vec());

        // Sparse side: the bit set stores DELETED docs, so the scan yields the
        // live ones.
        let mut deleted_bits = SparseFixedBitSet::new(max_doc);
        for &d in deleted.iter() {
            deleted_bits.set(d);
        }
        let mut live_iter = FilteredDocIdSetIterator::new(
            max_doc as i32,
            (max_doc - deleted_count) as i32,
            Box::new(move |doc| !deleted_bits.get(doc as usize)),
        );
        assert_eq!(live_iter.cost(), (max_doc - deleted_count) as i64);
        let mut got_live = Vec::new();
        loop {
            let doc = live_iter.next_doc().unwrap();
            if doc == NO_MORE_DOCS {
                break;
            }
            got_live.push(doc as usize);
        }
        let expected_live: Vec<usize> = (0..max_doc).filter(|d| !deleted.contains(d)).collect();
        assert_eq!(got_live, expected_live);

        // The two iterators partition [0, max_doc) exactly.
        let mut union = got_deleted;
        union.extend(got_live);
        union.sort_unstable();
        assert_eq!(union, (0..max_doc).collect::<Vec<_>>());
    }

    fn drain(it: &mut dyn DocIdSetIterator) -> Vec<i32> {
        let mut out = Vec::new();
        loop {
            let doc = it.next_doc().expect("next_doc");
            if doc == NO_MORE_DOCS {
                break;
            }
            out.push(doc);
        }
        out
    }

    /// The four live-docs iterators, against the same deletions expressed both
    /// ways. Equivalent to the four Java call sites:
    /// `DenseLiveDocs.liveDocsIterator()` / `deletedDocsIterator()` and
    /// `SparseLiveDocs.liveDocsIterator()` / `deletedDocsIterator()`.
    #[test]
    fn live_docs_iterators_agree_with_the_bits_view() {
        let max_doc = 300usize;
        let deleted: Vec<usize> = vec![0, 1, 7, 63, 64, 65, 128, 299];

        let mut live_bits = FixedBitSet::new(max_doc);
        live_bits.set_range(0, max_doc);
        let mut deleted_bits = SparseFixedBitSet::new(max_doc);
        for &d in &deleted {
            live_bits.clear(d);
            deleted_bits.set(d);
        }

        let dense = DenseLiveDocs::builder(live_bits, max_doc)
            .with_deleted_count(deleted.len())
            .build();
        let sparse = SparseLiveDocs::builder(deleted_bits, max_doc)
            .with_deleted_count(deleted.len())
            .build();

        let expected_deleted: Vec<i32> = deleted.iter().map(|&d| d as i32).collect();
        let expected_live: Vec<i32> = (0..max_doc)
            .filter(|d| !deleted.contains(d))
            .map(|d| d as i32)
            .collect();

        for (name, docs) in [
            ("dense", &dense as &dyn LiveDocs),
            ("sparse", &sparse as &dyn LiveDocs),
        ] {
            assert_eq!(docs.deleted_count(), deleted.len() as i32, "{name}");

            let mut live_it = docs.live_docs_iterator().expect("live iterator");
            assert_eq!(
                live_it.cost(),
                (max_doc - deleted.len()) as i64,
                "{name}: live cost is maxDoc - deletedCount"
            );
            assert_eq!(drain(live_it.as_mut()), expected_live, "{name}: live docs");

            let mut del_it = docs.deleted_docs_iterator().expect("deleted iterator");
            assert_eq!(
                del_it.cost(),
                deleted.len() as i64,
                "{name}: deleted cost is deletedCount"
            );
            assert_eq!(
                drain(del_it.as_mut()),
                expected_deleted,
                "{name}: deleted docs"
            );

            // Every document is live exactly when `Bits::get` says so.
            for doc in 0..max_doc {
                assert_eq!(
                    Bits::get(docs, doc),
                    !deleted.contains(&doc),
                    "{name}: get({doc})"
                );
            }
        }
    }

    /// `Bits::into_live_docs` is Rucene's stand-in for Java's
    /// `bits instanceof LiveDocs` in `LeafReader.getLiveDocsWithDeletedIterator`,
    /// so both live-docs types must answer it.
    #[test]
    fn live_docs_narrow_from_bits() {
        let max_doc = 64usize;
        let mut live_bits = FixedBitSet::new(max_doc);
        live_bits.set_range(0, max_doc);
        live_bits.clear(3);
        let dense: Box<dyn Bits> = Box::new(
            DenseLiveDocs::builder(live_bits, max_doc)
                .with_deleted_count(1)
                .build(),
        );
        let narrowed = dense.into_live_docs().expect("DenseLiveDocs is LiveDocs");
        assert_eq!(narrowed.deleted_count(), 1);
        assert_eq!(
            drain(narrowed.deleted_docs_iterator().unwrap().as_mut()),
            vec![3]
        );

        let mut deleted_bits = SparseFixedBitSet::new(max_doc);
        deleted_bits.set(3);
        let sparse: Box<dyn Bits> = Box::new(
            SparseLiveDocs::builder(deleted_bits, max_doc)
                .with_deleted_count(1)
                .build(),
        );
        let narrowed = sparse.into_live_docs().expect("SparseLiveDocs is LiveDocs");
        assert_eq!(narrowed.deleted_count(), 1);
        assert_eq!(
            drain(narrowed.deleted_docs_iterator().unwrap().as_mut()),
            vec![3]
        );
    }

    /// `first_doc` answers the first set bit at or after a block, which is what
    /// `next_set_bit_in_range` falls back to once the current block is
    /// exhausted. Crossing a 4096-bit block boundary is the case that exercises
    /// it.
    #[test]
    fn first_doc_finds_the_next_non_empty_block() {
        let mut set = SparseFixedBitSet::new(20_000);
        set.set(5);
        set.set(4096 * 3 + 77);

        // Block 0 holds doc 5.
        assert_eq!(set.first_doc(0, 5), 5);
        // Blocks 1 and 2 are empty, so the answer is the doc in block 3.
        assert_eq!(set.first_doc(1, 5), 4096 * 3 + 77);
        assert_eq!(set.first_doc(3, 5), 4096 * 3 + 77);
        // Past the last non-empty block there is nothing.
        assert_eq!(set.first_doc(4, 5), NO_MORE_DOCS);
        // An empty upper bound is an immediate miss.
        assert_eq!(set.first_doc(0, 0), NO_MORE_DOCS);
    }

    /// `next_set_bit` now delegates to `next_set_bit_in_range`, which delegates
    /// to `first_doc`; the whole chain must agree with a straight scan.
    #[test]
    fn next_set_bit_matches_a_linear_scan_across_blocks() {
        let length = 30_000usize;
        let mut set = SparseFixedBitSet::new(length);
        let mut rng = TestRng::new(0x51ED_2701);
        let mut expected = Vec::new();
        for _ in 0..400 {
            let bit = rng.next_in_range(length);
            if !set.get(bit) {
                set.set(bit);
                expected.push(bit);
            }
        }
        expected.sort_unstable();

        // Walk the set with next_set_bit and compare against the known bits.
        let mut walked = Vec::new();
        let mut cursor = 0usize;
        while let Some(bit) = set.next_set_bit(cursor) {
            walked.push(bit);
            cursor = bit + 1;
            if cursor >= length {
                break;
            }
        }
        assert_eq!(walked, expected);

        // And from every start position, the answer is the first expected bit
        // at or after it.
        for start in 0..length {
            let want = expected.iter().copied().find(|&b| b >= start);
            assert_eq!(set.next_set_bit(start), want, "start={start}");
        }
    }

    /// Regression: `FixedBitSet.ramBytesUsed()` is
    /// `BASE_RAM_BYTES_USED + sizeOf(bits)`. Rucene used to omit the base term
    /// and report only the words, which understated every estimate built on it
    /// — `RunAutomaton.ramBytesUsed()` among them.
    #[test]
    fn fixed_bit_set_ram_bytes_used_includes_the_shallow_size() {
        // Measured against lucene-core-10.5.0.jar on a JVM with compressed
        // references: shallowSizeOfInstance(FixedBitSet.class) == 24, and
        // new FixedBitSet(2).ramBytesUsed() == 48.
        assert_eq!(FixedBitSet::BASE_RAM_BYTES_USED, 24);
        assert_eq!(FixedBitSet::new(2).ram_bytes_used(), 24 + (16 + 8));
        assert_eq!(FixedBitSet::new(2).ram_bytes_used(), 48);
        // new FixedBitSet(1000).ramBytesUsed() == 168: 16 words of 8 bytes
        // under a 16-byte array header, aligned, plus the 24-byte base.
        assert_eq!(FixedBitSet::new(1000).ram_bytes_used(), 168);
    }

    /// Regression, same defect as above:
    /// `SparseFixedBitSet.ramBytesUsed()` is seeded with
    /// `BASE_RAM_BYTES_USED + sizeOf(indices) + shallowSizeOf(bits)`.
    #[test]
    fn sparse_fixed_bit_set_ram_bytes_used_includes_the_shallow_size() {
        // Measured against lucene-core-10.5.0.jar:
        // shallowSizeOfInstance(SparseFixedBitSet.class) == 40, and
        // new SparseFixedBitSet(10000).ramBytesUsed() == 112 while empty.
        assert_eq!(SparseFixedBitSet::BASE_RAM_BYTES_USED, 40);
        let mut set = SparseFixedBitSet::new(10_000);
        // 3 blocks: indices is a long[3] (40 bytes) and bits an Object[3]
        // (32 bytes).
        assert_eq!(set.ram_bytes_used(), 40 + 40 + 32);
        assert_eq!(set.ram_bytes_used(), 112);
        // Two bits in two different blocks allocate one word in each, and Java
        // reports 160 for exactly that.
        set.set(5);
        set.set(9000);
        assert_eq!(set.ram_bytes_used(), 160);
    }
}

// -----------------------------------------------------------------------------
// Object contracts: the `toString()` bodies of this module's four types
// -----------------------------------------------------------------------------

impl fmt::Display for SparseFixedBitSet {
    /// Port of `SparseFixedBitSet.toString()`.
    ///
    /// Java builds `"SparseFixedBitSet(size=" + length + ",cardinality=~" +
    /// approximateCardinality()` — the closing parenthesis is missing in
    /// `SparseFixedBitSet.java:369` of Lucene 10.5.0, and this port reproduces
    /// the string as Lucene emits it rather than repairing it, because the text
    /// is the observable contract.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "SparseFixedBitSet(size={},cardinality=~{}",
            self.length,
            self.approximate_cardinality()
        )
    }
}

impl fmt::Display for RoaringDocIdSet {
    /// Port of `RoaringDocIdSet.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RoaringDocIdSet(cardinality={})", self.cardinality)
    }
}

impl fmt::Display for DenseLiveDocs {
    /// Port of `DenseLiveDocs.toString()`.
    ///
    /// The deletion rate goes through `String.format(Locale.ROOT, "%.2f%%",
    /// ...)`, whose ties round away from zero; see
    /// [`java_fmt::format_fixed`].
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "DenseLiveDocs(maxDoc={}, deleted={}, deletionRate={}%)",
            self.max_doc,
            self.deleted_count,
            java_fmt::format_fixed(100.0 * self.deleted_count as f64 / self.max_doc as f64, 2)
        )
    }
}

impl fmt::Display for SparseLiveDocs {
    /// Port of `SparseLiveDocs.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "SparseLiveDocs(maxDoc={}, deleted={}, deletionRate={}%)",
            self.max_doc,
            self.deleted_count,
            java_fmt::format_fixed(100.0 * self.deleted_count as f64 / self.max_doc as f64, 2)
        )
    }
}
