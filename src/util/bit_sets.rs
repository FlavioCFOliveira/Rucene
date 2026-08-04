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

#![deny(unsafe_code)]

use std::fmt;

use crate::search::{DocIdSetIterator, NO_MORE_DOCS};
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
    pub fn length(&self) -> usize {
        self.length
    }

    /// Returns the value of the bit at `index`.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of bounds.
    pub fn get(&self, index: usize) -> bool {
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

    /// Returns the first set bit at or after `start`, or `None` if there is none.
    pub fn next_set_bit(&self, start: usize) -> Option<usize> {
        if start >= self.length {
            return None;
        }
        let start_i4096 = start >> 12;
        let start_i64 = (start >> 6) & 0x3f;
        let start_sub = start & 0x3f;

        for (block_idx, index_word) in self.indices.iter().enumerate().skip(start_i4096) {
            if *index_word == 0 {
                continue;
            }
            let first_i64 = if block_idx == start_i4096 {
                start_i64
            } else {
                0
            };
            for i64 in first_i64..SPARSE_LONGS_PER_BLOCK {
                let i64bit = 1u64 << i64;
                if (index_word & i64bit) == 0 {
                    continue;
                }
                let o = (index_word & (i64bit - 1)).count_ones() as usize;
                let word = self.bits[block_idx].as_ref().unwrap()[o];
                let sub = if block_idx == start_i4096 && i64 == start_i64 {
                    start_sub
                } else {
                    0
                };
                let shifted = word >> sub;
                if shifted != 0 {
                    let candidate =
                        (block_idx << 12) | (i64 << 6) | (sub + shifted.trailing_zeros() as usize);
                    if candidate < self.length {
                        return Some(candidate);
                    }
                }
            }
        }
        None
    }

    /// Returns an iterator over the set bits as a [`DocIdSetIterator`].
    ///
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

impl Accountable for SparseFixedBitSet {
    fn ram_bytes_used(&self) -> i64 {
        let mut bytes = RamUsageEstimator::size_of_u64(&self.indices)
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
// DenseLiveDocs
// -----------------------------------------------------------------------------

/// Live-docs representation optimized for dense deletions.
///
/// Set bits in the wrapped [`FixedBitSet`] represent **live** documents, which is
/// the traditional Lucene semantics. Instances are immutable after construction.
/// Equivalent to `org.apache.lucene.util.DenseLiveDocs`.
#[derive(Clone, Debug)]
pub struct DenseLiveDocs {
    live_docs: FixedBitSet,
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
            live_docs: self.live_docs,
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
    deleted_docs: SparseFixedBitSet,
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
            deleted_docs: self.deleted_docs,
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
}
