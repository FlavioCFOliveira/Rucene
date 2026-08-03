//! Additional small utilities used by codecs, ported from `org.apache.lucene.util`.
//!
//! Contains `LongValues`, `LongBitSet`, `PriorityQueue`, `MergedIterator`, and
//! `Version`.

#![deny(unsafe_code)]

use std::{
    cmp::Ordering,
    collections::BinaryHeap,
    fmt::{self, Debug, Display, Formatter},
    iter::Iterator,
    vec::Vec,
};

use crate::{
    error::LuceneError,
    util::{Accountable, ArrayUtil},
};

// ---------------------------------------------------------------------------
// LongValues
// ---------------------------------------------------------------------------

/// Abstraction over an array of longs, equivalent to Lucene's `LongValues`.
///
/// Implementations return the value at the given index. The `IDENTITY` and
/// `ZEROES` instances are provided for convenience.
pub trait LongValues {
    /// Returns the value at `index`.
    fn get(&self, index: i64) -> i64;
}

/// A `LongValues` implementation that returns `index`.
#[derive(Debug, Copy, Clone, Default)]
pub struct IdentityLongValues;

impl LongValues for IdentityLongValues {
    fn get(&self, index: i64) -> i64 {
        index
    }
}

/// A `LongValues` implementation that always returns zero.
#[derive(Debug, Copy, Clone, Default)]
pub struct ZeroesLongValues;

impl LongValues for ZeroesLongValues {
    fn get(&self, _index: i64) -> i64 {
        0
    }
}

// ---------------------------------------------------------------------------
// LongBitSet
// ---------------------------------------------------------------------------

/// BitSet of fixed length backed by a `Vec<i64>`, indexed with a `long`,
/// equivalent to Lucene's `LongBitSet`.
///
/// Use this only when you need to store more than 2.1B bits; otherwise prefer
/// [`FixedBitSet`](crate::util::FixedBitSet).
#[derive(Clone, Debug)]
pub struct LongBitSet {
    bits: Vec<i64>,
    num_bits: i64,
    num_words: i32,
}

impl LongBitSet {
    /// Maximum number of bits supported by this bit set.
    pub const MAX_NUM_BITS: i64 = 64 * (ArrayUtil::MAX_ARRAY_LENGTH as i64);

    /// Returns the number of 64-bit words needed to hold `num_bits`.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` if `num_bits` is negative or
    /// exceeds [`Self::MAX_NUM_BITS`].
    pub fn bits2words(num_bits: i64) -> Result<i32, LuceneError> {
        if !(0..=Self::MAX_NUM_BITS).contains(&num_bits) {
            return Err(LuceneError::IllegalArgument(format!(
                "numBits must be 0 .. {}; got: {}",
                Self::MAX_NUM_BITS,
                num_bits
            )));
        }
        Ok(((num_bits - 1) >> 6) as i32 + 1)
    }

    /// Creates a new bit set large enough to hold `num_bits`.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` if `num_bits` is out of range.
    pub fn new(num_bits: i64) -> Result<Self, LuceneError> {
        let num_words = Self::bits2words(num_bits)?;
        Ok(Self {
            bits: vec![0i64; num_words as usize],
            num_bits,
            num_words,
        })
    }

    /// Creates a bit set from an existing backing buffer.
    ///
    /// Any "ghost" bits past `num_bits` must be clear.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` if the buffer is too small.
    pub fn from_bits(stored_bits: Vec<i64>, num_bits: i64) -> Result<Self, LuceneError> {
        let num_words = Self::bits2words(num_bits)?;
        if (num_words as usize) > stored_bits.len() {
            return Err(LuceneError::IllegalArgument(format!(
                "The given long array is too small to hold {} bits",
                num_bits
            )));
        }
        let set = Self {
            bits: stored_bits,
            num_bits,
            num_words,
        };
        if !set.verify_ghost_bits_clear() {
            return Err(LuceneError::IllegalArgument(
                "ghost bits past numBits are not clear".to_string(),
            ));
        }
        Ok(set)
    }

    fn verify_ghost_bits_clear(&self) -> bool {
        for i in (self.num_words as usize)..self.bits.len() {
            if self.bits[i] != 0 {
                return false;
            }
        }
        if (self.num_bits & 0x3f) == 0 {
            return true;
        }
        let mask = (-1i64 as u64) << self.num_bits;
        let last = self.bits[(self.num_words - 1) as usize] as u64;
        (last & mask) == 0
    }

    /// Returns the number of bits stored in this bitset.
    pub fn length(&self) -> i64 {
        self.num_bits
    }

    /// Returns a reference to the backing `i64` buffer.
    pub fn get_bits(&self) -> &[i64] {
        &self.bits
    }

    /// Returns the number of set bits.
    pub fn cardinality(&self) -> i64 {
        let mut total = 0i64;
        for i in 0..(self.num_words as usize) {
            total += (self.bits[i] as u64).count_ones() as i64;
        }
        total
    }

    /// Returns the value of the bit at `index`.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of bounds.
    pub fn get(&self, index: i64) -> bool {
        assert!(index >= 0 && index < self.num_bits);
        let word_num = (index >> 6) as usize;
        let bitmask = 1i64.wrapping_shl(index as u32);
        ((self.bits[word_num] as u64) & (bitmask as u64)) != 0
    }

    /// Sets the bit at `index`.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of bounds.
    pub fn set(&mut self, index: i64) {
        assert!(index >= 0 && index < self.num_bits);
        let word_num = (index >> 6) as usize;
        let bitmask = 1i64.wrapping_shl(index as u32);
        self.bits[word_num] |= bitmask;
    }

    /// Sets the bit at `index` and returns its previous value.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of bounds.
    pub fn get_and_set(&mut self, index: i64) -> bool {
        assert!(index >= 0 && index < self.num_bits);
        let word_num = (index >> 6) as usize;
        let bitmask = 1i64.wrapping_shl(index as u32);
        let val = (self.bits[word_num] as u64 & bitmask as u64) != 0;
        self.bits[word_num] |= bitmask;
        val
    }

    /// Clears the bit at `index`.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of bounds.
    pub fn clear(&mut self, index: i64) {
        assert!(index >= 0 && index < self.num_bits);
        let word_num = (index >> 6) as usize;
        let bitmask = 1i64.wrapping_shl(index as u32);
        self.bits[word_num] &= !bitmask;
    }

    /// Clears the bit at `index` and returns its previous value.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of bounds.
    pub fn get_and_clear(&mut self, index: i64) -> bool {
        assert!(index >= 0 && index < self.num_bits);
        let word_num = (index >> 6) as usize;
        let bitmask = 1i64.wrapping_shl(index as u32);
        let val = (self.bits[word_num] as u64 & bitmask as u64) != 0;
        self.bits[word_num] &= !bitmask;
        val
    }

    /// Returns the index of the first set bit at or after `index`, or `-1`.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of bounds.
    pub fn next_set_bit(&self, index: i64) -> i64 {
        assert!(index >= 0 && index < self.num_bits);
        let mut i = (index >> 6) as usize;
        let word = (self.bits[i] as u64) >> (index & 0x3f);
        if word != 0 {
            return index + word.trailing_zeros() as i64;
        }
        i += 1;
        while (i as i32) < self.num_words {
            let word = self.bits[i] as u64;
            if word != 0 {
                return ((i as i64) << 6) + word.trailing_zeros() as i64;
            }
            i += 1;
        }
        -1
    }

    /// Returns the index of the last set bit at or before `index`, or `-1`.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of bounds.
    pub fn prev_set_bit(&self, index: i64) -> i64 {
        assert!(index >= 0 && index < self.num_bits);
        let i = (index >> 6) as usize;
        let sub_index = (index & 0x3f) as i32;
        let word = (self.bits[i] as u64) << (63 - sub_index);
        if word != 0 {
            return ((i as i64) << 6) + sub_index as i64 - word.leading_zeros() as i64;
        }
        let mut j = i as i32 - 1;
        while j >= 0 {
            let word = self.bits[j as usize] as u64;
            if word != 0 {
                return ((j as i64) << 6) + 63 - word.leading_zeros() as i64;
            }
            j -= 1;
        }
        -1
    }

    /// `self = self OR other`
    ///
    /// # Panics
    ///
    /// Panics if `other` has more words than `self`.
    pub fn or(&mut self, other: &LongBitSet) {
        assert!(other.num_words <= self.num_words);
        let pos = self.num_words.min(other.num_words) as usize;
        for i in 0..pos {
            self.bits[i] |= other.bits[i];
        }
    }

    /// `self = self XOR other`
    ///
    /// # Panics
    ///
    /// Panics if `other` has more words than `self`.
    pub fn xor(&mut self, other: &LongBitSet) {
        assert!(other.num_words <= self.num_words);
        let pos = self.num_words.min(other.num_words) as usize;
        for i in 0..pos {
            self.bits[i] ^= other.bits[i];
        }
    }

    /// Returns true if this set and `other` share any set bit.
    pub fn intersects(&self, other: &LongBitSet) -> bool {
        let pos = self.num_words.min(other.num_words) as usize;
        for i in 0..pos {
            if (self.bits[i] as u64 & other.bits[i] as u64) != 0 {
                return true;
            }
        }
        false
    }

    /// `self = self AND other`
    pub fn and(&mut self, other: &LongBitSet) {
        let pos = self.num_words.min(other.num_words) as usize;
        for i in 0..pos {
            self.bits[i] &= other.bits[i];
        }
        if self.num_words > other.num_words {
            for i in pos..(self.num_words as usize) {
                self.bits[i] = 0;
            }
        }
    }

    /// `self = self AND NOT other`
    pub fn and_not(&mut self, other: &LongBitSet) {
        let pos = self.num_words.min(other.num_words) as usize;
        for i in 0..pos {
            self.bits[i] &= !other.bits[i];
        }
    }

    /// Scans the backing store to check if all bits are clear.
    pub fn scan_is_empty(&self) -> bool {
        for i in 0..(self.num_words as usize) {
            if self.bits[i] != 0 {
                return false;
            }
        }
        true
    }

    /// Flips bits in the range `[start_index, end_index)`.
    ///
    /// # Panics
    ///
    /// Panics if the range is out of bounds or reversed.
    pub fn flip_range(&mut self, start_index: i64, end_index: i64) {
        assert!(start_index >= 0 && start_index < self.num_bits);
        assert!(end_index >= 0 && end_index <= self.num_bits);
        if end_index <= start_index {
            return;
        }
        let start_word = (start_index >> 6) as usize;
        let end_word = ((end_index - 1) >> 6) as usize;
        let start_mask = (-1i64).wrapping_shl(start_index as u32) as u64;
        let end_mask = u64::MAX.wrapping_shr(((-end_index) as u64) as u32);
        if start_word == end_word {
            self.bits[start_word] ^= (start_mask & end_mask) as i64;
            return;
        }
        self.bits[start_word] ^= start_mask as i64;
        for i in (start_word + 1)..end_word {
            self.bits[i] = !self.bits[i];
        }
        self.bits[end_word] ^= end_mask as i64;
    }

    /// Flips the bit at `index`.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of bounds.
    pub fn flip(&mut self, index: i64) {
        assert!(index >= 0 && index < self.num_bits);
        let word_num = (index >> 6) as usize;
        let bitmask = 1i64.wrapping_shl(index as u32);
        self.bits[word_num] ^= bitmask;
    }

    /// Sets bits in the range `[start_index, end_index)`.
    ///
    /// # Panics
    ///
    /// Panics if the range is out of bounds or reversed.
    pub fn set_range(&mut self, start_index: i64, end_index: i64) {
        assert!(start_index >= 0 && start_index < self.num_bits);
        assert!(end_index >= 0 && end_index <= self.num_bits);
        if end_index <= start_index {
            return;
        }
        let start_word = (start_index >> 6) as usize;
        let end_word = ((end_index - 1) >> 6) as usize;
        let start_mask = (-1i64).wrapping_shl(start_index as u32) as u64;
        let end_mask = u64::MAX.wrapping_shr(((-end_index) as u64) as u32);
        if start_word == end_word {
            self.bits[start_word] |= (start_mask & end_mask) as i64;
            return;
        }
        self.bits[start_word] |= start_mask as i64;
        for i in (start_word + 1)..end_word {
            self.bits[i] = !0i64;
        }
        self.bits[end_word] |= end_mask as i64;
    }

    /// Clears bits in the range `[start_index, end_index)`.
    ///
    /// # Panics
    ///
    /// Panics if the range is out of bounds or reversed.
    pub fn clear_range(&mut self, start_index: i64, end_index: i64) {
        assert!(start_index >= 0 && start_index < self.num_bits);
        assert!(end_index >= 0 && end_index <= self.num_bits);
        if end_index <= start_index {
            return;
        }
        let start_word = (start_index >> 6) as usize;
        let end_word = ((end_index - 1) >> 6) as usize;
        let mut start_mask = (-1i64).wrapping_shl(start_index as u32) as u64;
        let mut end_mask = u64::MAX.wrapping_shr(((-end_index) as u64) as u32);
        start_mask = !start_mask;
        end_mask = !end_mask;
        if start_word == end_word {
            self.bits[start_word] &= (start_mask | end_mask) as i64;
            return;
        }
        self.bits[start_word] &= start_mask as i64;
        for i in (start_word + 1)..end_word {
            self.bits[i] = 0;
        }
        self.bits[end_word] &= end_mask as i64;
    }
}

impl PartialEq for LongBitSet {
    fn eq(&self, other: &Self) -> bool {
        self.num_bits == other.num_bits && self.bits == other.bits
    }
}

impl Eq for LongBitSet {}

impl Accountable for LongBitSet {
    fn ram_bytes_used(&self) -> i64 {
        // Base object estimate (8-byte header + 3 refs) plus the long array.
        8 + 3 * 4 + 16 + 8 * self.bits.len() as i64
    }
}

// ---------------------------------------------------------------------------
// PriorityQueue
// ---------------------------------------------------------------------------

/// Priority ordering callback for [`PriorityQueue`].
///
/// Equivalent to overriding `lessThan` in Lucene's abstract `PriorityQueue`.
pub trait PriorityQueueComparator<T> {
    /// Returns true iff `a` is less than `b`.
    fn less_than(&self, a: &T, b: &T) -> bool;
}

/// A priority queue that maintains a partial ordering so that the least element
/// is always available in constant time, equivalent to Lucene's `PriorityQueue`.
///
/// The heap is 1-based; index 0 is unused. Insertion and removal take
/// `O(log size)` time; removal of an arbitrary element takes linear time.
pub struct PriorityQueue<T, C: PriorityQueueComparator<T>> {
    heap: Vec<Option<T>>,
    size: usize,
    max_size: usize,
    comparator: C,
}

impl<T, C: PriorityQueueComparator<T>> PriorityQueue<T, C> {
    /// Creates an empty priority queue of the configured size.
    pub fn new(max_size: usize, comparator: C) -> Result<Self, LuceneError> {
        if max_size >= ArrayUtil::MAX_ARRAY_LENGTH {
            return Err(LuceneError::IllegalArgument(format!(
                "maxSize must be >= 0 and < {}; got: {}",
                ArrayUtil::MAX_ARRAY_LENGTH,
                max_size
            )));
        }
        // heap[0] is unused; allocate max_size + 1 slots. When max_size is 0
        // we still allocate a small array so top() can always return heap[1].
        let heap_size = if max_size == 0 { 2 } else { max_size + 1 };
        Ok(Self {
            heap: (0..heap_size).map(|_| None).collect(),
            size: 0,
            max_size,
            comparator,
        })
    }

    /// Adds all elements to the queue in bulk, building the heap in a single
    /// pass.
    pub fn add_all<I: IntoIterator<Item = T>>(&mut self, elements: I) {
        for element in elements {
            self.heap[self.size + 1] = Some(element);
            self.size += 1;
        }
        if self.size > 0 {
            for i in (1..=(self.size / 2)).rev() {
                self.down_heap(i);
            }
        }
    }

    /// Adds an element in `O(log size)` time and returns the new top.
    ///
    /// # Panics
    ///
    /// Panics if the queue is already full.
    pub fn add(&mut self, element: T) -> Option<&T> {
        let index = self.size + 1;
        self.heap[index] = Some(element);
        self.size = index;
        self.up_heap(index);
        self.heap[1].as_ref()
    }

    /// Adds an element, returning the element that was dropped if the queue is
    /// full and the new element is not smaller than the current top.
    pub fn insert_with_overflow(&mut self, element: T) -> Option<T> {
        if self.size < self.max_size {
            self.add(element);
            None
        } else if self.size > 0
            && self
                .comparator
                .less_than(self.heap[1].as_ref().unwrap(), &element)
        {
            let ret = self.heap[1].take().unwrap();
            self.heap[1] = Some(element);
            self.update_top();
            Some(ret)
        } else {
            Some(element)
        }
    }

    /// Returns the least element without removing it.
    pub fn top(&self) -> Option<&T> {
        self.heap[1].as_ref()
    }

    /// Removes and returns the least element in `O(log size)` time.
    pub fn pop(&mut self) -> Option<T> {
        if self.size == 0 {
            return None;
        }
        let result = self.heap[1].take();
        if self.size > 1 {
            self.heap[1] = self.heap[self.size].take();
            self.size -= 1;
            self.down_heap(1);
        } else {
            self.size = 0;
        }
        result
    }

    /// Re-establishes the heap after the top element has been mutated in place.
    pub fn update_top(&mut self) -> Option<&T> {
        self.down_heap(1);
        self.heap[1].as_ref()
    }

    /// Replaces the top with `new_top` and re-establishes the heap.
    pub fn update_top_with(&mut self, new_top: T) -> Option<&T> {
        self.heap[1] = Some(new_top);
        self.update_top()
    }

    /// Returns the number of elements currently stored.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Removes all elements from the queue.
    pub fn clear(&mut self) {
        for i in 0..=self.size {
            self.heap[i] = None;
        }
        self.size = 0;
    }

    /// Removes an existing element by identity, returning true if found.
    pub fn remove(&mut self, element: &T) -> bool
    where
        T: PartialEq,
    {
        for i in 1..=self.size {
            if self.heap[i].as_ref() == Some(element) {
                self.heap[i] = self.heap[self.size].take();
                self.heap[self.size] = None;
                self.size -= 1;
                if i <= self.size && !self.up_heap(i) {
                    self.down_heap(i);
                }
                return true;
            }
        }
        false
    }

    fn up_heap(&mut self, orig_pos: usize) -> bool {
        let mut i = orig_pos;
        let node = self.heap[i].take().unwrap();
        let mut j = i >> 1;
        while j > 0
            && self
                .comparator
                .less_than(&node, self.heap[j].as_ref().unwrap())
        {
            self.heap[i] = self.heap[j].take();
            i = j;
            j >>= 1;
        }
        self.heap[i] = Some(node);
        i != orig_pos
    }

    fn down_heap(&mut self, mut i: usize) {
        let node = self.heap[i].take().unwrap();
        let mut j = i << 1;
        let mut k = j + 1;
        if k <= self.size
            && self.comparator.less_than(
                self.heap[k].as_ref().unwrap(),
                self.heap[j].as_ref().unwrap(),
            )
        {
            j = k;
        }
        while j <= self.size
            && self
                .comparator
                .less_than(self.heap[j].as_ref().unwrap(), &node)
        {
            self.heap[i] = self.heap[j].take();
            i = j;
            j = i << 1;
            k = j + 1;
            if k <= self.size
                && self.comparator.less_than(
                    self.heap[k].as_ref().unwrap(),
                    self.heap[j].as_ref().unwrap(),
                )
            {
                j = k;
            }
        }
        self.heap[i] = Some(node);
    }
}

impl<T, C: PriorityQueueComparator<T>> IntoIterator for PriorityQueue<T, C> {
    type Item = T;
    type IntoIter = IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        let cap = self.size;
        IntoIter {
            heap: self.heap,
            size: cap,
            index: 1,
        }
    }
}

/// Iterator over the internal heap of a [`PriorityQueue`].
///
/// Iteration order is unspecified.
pub struct IntoIter<T> {
    heap: Vec<Option<T>>,
    size: usize,
    index: usize,
}

impl<T> Iterator for IntoIter<T> {
    type Item = T;

    fn next(&mut self) -> Option<T> {
        if self.index <= self.size {
            let item = self.heap[self.index].take();
            self.index += 1;
            item
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// MergedIterator
// ---------------------------------------------------------------------------

/// Provides a merged sorted view from several sorted iterators, equivalent to
/// Lucene's `MergedIterator`.
///
/// With `remove_duplicates` enabled, equal elements from different iterators
/// are deduplicated; with it disabled, all elements are returned. The input
/// iterators must be sorted and must not yield `None` as a "real" element.
pub struct MergedIterator<T: Ord> {
    current: Option<T>,
    queue: BinaryHeap<SubIterator<T>>,
    top: Vec<TopState<T>>,
    remove_duplicates: bool,
}

struct SubIterator<T: Ord> {
    current: T,
    source: Box<dyn Iterator<Item = T>>,
    index: usize,
}

struct TopState<T> {
    source: Box<dyn Iterator<Item = T>>,
    index: usize,
}

impl<T: Ord> PartialEq for SubIterator<T> {
    fn eq(&self, other: &Self) -> bool {
        self.current == other.current && self.index == other.index
    }
}

impl<T: Ord> Eq for SubIterator<T> {}

impl<T: Ord> PartialOrd for SubIterator<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T: Ord> Ord for SubIterator<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        // BinaryHeap is a max-heap, so we reverse comparisons to get a min-heap.
        let cmp = self.current.cmp(&other.current);
        if cmp != Ordering::Equal {
            return cmp.reverse();
        }
        self.index.cmp(&other.index).reverse()
    }
}

impl<T: Ord> MergedIterator<T> {
    /// Creates a merged iterator with duplicate removal enabled.
    pub fn new<I>(iterators: I) -> Self
    where
        I: IntoIterator<Item = Box<dyn Iterator<Item = T>>>,
    {
        Self::with_remove_duplicates(true, iterators)
    }

    /// Creates a merged iterator, optionally removing duplicates.
    pub fn with_remove_duplicates<I>(remove_duplicates: bool, iterators: I) -> Self
    where
        I: IntoIterator<Item = Box<dyn Iterator<Item = T>>>,
    {
        let mut queue = BinaryHeap::new();
        for (index, mut source) in iterators.into_iter().enumerate() {
            if let Some(current) = source.next() {
                queue.push(SubIterator {
                    current,
                    source,
                    index,
                });
            }
        }
        Self {
            current: None,
            queue,
            top: Vec::with_capacity(4),
            remove_duplicates,
        }
    }

    fn pull_top(&mut self) {
        assert!(self.top.is_empty());
        if let Some(top) = self.queue.pop() {
            let value = top.current;
            self.top.push(TopState {
                source: top.source,
                index: top.index,
            });
            if self.remove_duplicates {
                while let Some(next) = self.queue.peek() {
                    if next.current == value {
                        let next = self.queue.pop().unwrap();
                        self.top.push(TopState {
                            source: next.source,
                            index: next.index,
                        });
                    } else {
                        break;
                    }
                }
            }
            self.current = Some(value);
        } else {
            self.current = None;
        }
    }

    fn push_top(&mut self) {
        for mut state in self.top.drain(..) {
            if let Some(current) = state.source.next() {
                self.queue.push(SubIterator {
                    current,
                    source: state.source,
                    index: state.index,
                });
            }
        }
    }
}

impl<T: Ord> Iterator for MergedIterator<T> {
    type Item = T;

    fn next(&mut self) -> Option<T> {
        self.push_top();
        self.pull_top();
        self.current.take()
    }
}

// ---------------------------------------------------------------------------
// Version
// ---------------------------------------------------------------------------

/// Lucene version compatibility marker, equivalent to Lucene's `Version`.
///
/// Encodes `major.minor.bugfix.prerelease` into a single integer so that
/// ordering comparisons are simple bit comparisons.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct Version {
    /// Major version.
    pub major: u8,
    /// Minor version.
    pub minor: u8,
    /// Bugfix number.
    pub bugfix: u8,
    /// Prerelease version: 0 (final), 1, or 2.
    pub prerelease: u8,
    encoded: i32,
}

impl Version {
    /// Lucene 9.0.0.
    pub const LUCENE_9_0_0: Version = Version::new_const(9, 0, 0, 0);
    /// Lucene 9.1.0.
    pub const LUCENE_9_1_0: Version = Version::new_const(9, 1, 0, 0);
    /// Lucene 9.2.0.
    pub const LUCENE_9_2_0: Version = Version::new_const(9, 2, 0, 0);
    /// Lucene 9.3.0.
    pub const LUCENE_9_3_0: Version = Version::new_const(9, 3, 0, 0);
    /// Lucene 9.4.0.
    pub const LUCENE_9_4_0: Version = Version::new_const(9, 4, 0, 0);
    /// Lucene 9.4.1.
    pub const LUCENE_9_4_1: Version = Version::new_const(9, 4, 1, 0);
    /// Lucene 9.4.2.
    pub const LUCENE_9_4_2: Version = Version::new_const(9, 4, 2, 0);
    /// Lucene 9.5.0.
    pub const LUCENE_9_5_0: Version = Version::new_const(9, 5, 0, 0);
    /// Lucene 9.6.0.
    pub const LUCENE_9_6_0: Version = Version::new_const(9, 6, 0, 0);
    /// Lucene 9.7.0.
    pub const LUCENE_9_7_0: Version = Version::new_const(9, 7, 0, 0);
    /// Lucene 9.8.0.
    pub const LUCENE_9_8_0: Version = Version::new_const(9, 8, 0, 0);
    /// Lucene 9.9.0.
    pub const LUCENE_9_9_0: Version = Version::new_const(9, 9, 0, 0);
    /// Lucene 9.9.1.
    pub const LUCENE_9_9_1: Version = Version::new_const(9, 9, 1, 0);
    /// Lucene 9.9.2.
    pub const LUCENE_9_9_2: Version = Version::new_const(9, 9, 2, 0);
    /// Lucene 9.10.0.
    pub const LUCENE_9_10_0: Version = Version::new_const(9, 10, 0, 0);
    /// Lucene 9.11.0.
    pub const LUCENE_9_11_0: Version = Version::new_const(9, 11, 0, 0);
    /// Lucene 9.11.1.
    pub const LUCENE_9_11_1: Version = Version::new_const(9, 11, 1, 0);
    /// Lucene 9.12.0.
    pub const LUCENE_9_12_0: Version = Version::new_const(9, 12, 0, 0);
    /// Lucene 9.12.1.
    pub const LUCENE_9_12_1: Version = Version::new_const(9, 12, 1, 0);
    /// Lucene 9.12.2.
    pub const LUCENE_9_12_2: Version = Version::new_const(9, 12, 2, 0);
    /// Lucene 9.12.3.
    pub const LUCENE_9_12_3: Version = Version::new_const(9, 12, 3, 0);
    /// Lucene 9.12.4.
    pub const LUCENE_9_12_4: Version = Version::new_const(9, 12, 4, 0);
    /// Lucene 10.0.0.
    pub const LUCENE_10_0_0: Version = Version::new_const(10, 0, 0, 0);
    /// Lucene 10.1.0.
    pub const LUCENE_10_1_0: Version = Version::new_const(10, 1, 0, 0);
    /// Lucene 10.2.0.
    pub const LUCENE_10_2_0: Version = Version::new_const(10, 2, 0, 0);
    /// Lucene 10.2.1.
    pub const LUCENE_10_2_1: Version = Version::new_const(10, 2, 1, 0);
    /// Lucene 10.2.2.
    pub const LUCENE_10_2_2: Version = Version::new_const(10, 2, 2, 0);
    /// Lucene 10.3.0.
    pub const LUCENE_10_3_0: Version = Version::new_const(10, 3, 0, 0);
    /// Lucene 10.3.1.
    pub const LUCENE_10_3_1: Version = Version::new_const(10, 3, 1, 0);
    /// Lucene 10.3.2.
    pub const LUCENE_10_3_2: Version = Version::new_const(10, 3, 2, 0);
    /// Lucene 10.4.0.
    pub const LUCENE_10_4_0: Version = Version::new_const(10, 4, 0, 0);
    /// Lucene 10.5.0.
    pub const LUCENE_10_5_0: Version = Version::new_const(10, 5, 0, 0);
    /// Latest supported version.
    pub const LATEST: Version = Version::LUCENE_10_5_0;
    /// Constant for backwards compatibility.
    pub const LUCENE_CURRENT: Version = Version::LATEST;
    /// Minimal supported major version of an index.
    pub const MIN_SUPPORTED_MAJOR: i32 = Version::LATEST.major as i32 - 1;

    const fn new_const(major: u8, minor: u8, bugfix: u8, prerelease: u8) -> Self {
        let encoded = ((major as i32) << 18)
            | ((minor as i32) << 10)
            | ((bugfix as i32) << 2)
            | prerelease as i32;
        Self {
            major,
            minor,
            bugfix,
            prerelease,
            encoded,
        }
    }

    fn new(major: u8, minor: u8, bugfix: u8, prerelease: u8) -> Result<Self, LuceneError> {
        // major/minor/bugfix are u8, so the Java >255 check is enforced by the type system.
        if prerelease > 2 {
            return Err(LuceneError::IllegalArgument(format!(
                "Illegal prerelease version: {}",
                prerelease
            )));
        }
        if prerelease != 0 && (minor != 0 || bugfix != 0) {
            return Err(LuceneError::IllegalArgument(format!(
                "Prerelease version only supported with major release (got prerelease: {}, minor: {}, bugfix: {})",
                prerelease, minor, bugfix
            )));
        }
        Ok(Self::new_const(major, minor, bugfix, prerelease))
    }

    /// Parses a dot-separated version of the form `major.minor.bugfix.prerelease`.
    ///
    /// The bugfix and prerelease components are optional. The parsed version does
    /// not need to exist as a predefined constant.
    pub fn parse(version: &str) -> Result<Self, LuceneError> {
        let mut tokens = version.split('.');
        let major = parse_version_component(tokens.next(), "major", version)?;
        let minor = parse_version_component(tokens.next(), "minor", version)?;
        let mut bugfix = 0u8;
        let mut prerelease = 0u8;
        if let Some(token) = tokens.next() {
            bugfix = parse_version_component(Some(token), "bugfix", version)?;
            if let Some(token) = tokens.next() {
                prerelease = parse_version_component(Some(token), "prerelease", version)?;
                if prerelease == 0 {
                    return Err(LuceneError::IllegalArgument(format!(
                        "Invalid value {} for prerelease; should be 1 or 2 (got: {})",
                        prerelease, version
                    )));
                }
                if tokens.next().is_some() {
                    return Err(LuceneError::IllegalArgument(format!(
                        "Version is not in form major.minor.bugfix(.prerelease) (got: {})",
                        version
                    )));
                }
            }
        }
        Self::new(major, minor, bugfix, prerelease)
    }

    /// Parses a version number leniently, accepting constant names such as
    /// `LATEST`, `LUCENE_CURRENT`, `LUCENE_X_Y`, `LUCENE_X_Y_Z`, or `LUCENE_XY`.
    pub fn parse_leniently(version: &str) -> Result<Self, LuceneError> {
        let original = version;
        let version = version.to_ascii_uppercase();
        match version.as_str() {
            "LATEST" | "LUCENE_CURRENT" => Ok(Self::LATEST),
            _ => {
                let numeric = Self::normalize_lenient_version(&version);
                match numeric {
                    Some(n) => Self::parse(&n),
                    None => Err(LuceneError::IllegalArgument(format!(
                        "failed to parse lenient version string \"{}\"",
                        original
                    ))),
                }
            }
        }
    }

    fn normalize_lenient_version(version: &str) -> Option<String> {
        if let Some(rest) = version.strip_prefix("LUCENE_") {
            let parts: Vec<&str> = rest.split('_').collect();
            match parts.len() {
                3 => Some(parts[0].to_string() + "." + parts[1] + "." + parts[2]),
                2 => {
                    let a = parts[0];
                    let b = parts[1];
                    // LUCENE_X_Y or LUCENE_XX_YY both map to X.Y.0.
                    Some(a.to_string() + "." + b + ".0")
                }
                _ => None,
            }
        } else {
            None
        }
    }

    /// Creates a version from raw numeric components.
    pub fn from_bits(major: u8, minor: u8, bugfix: u8) -> Result<Self, LuceneError> {
        Self::new(major, minor, bugfix, 0)
    }

    /// Returns true if this version is the same or after `other`.
    pub fn on_or_after(&self, other: &Version) -> bool {
        self.encoded >= other.encoded
    }
}

impl Display for Version {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        if self.prerelease == 0 {
            write!(f, "{}.{}.{}", self.major, self.minor, self.bugfix)
        } else {
            write!(
                f,
                "{}.{}.{}.{}",
                self.major, self.minor, self.bugfix, self.prerelease
            )
        }
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.encoded.cmp(&other.encoded)
    }
}

fn parse_version_component(
    token: Option<&str>,
    name: &str,
    version: &str,
) -> Result<u8, LuceneError> {
    let token = token.ok_or_else(|| {
        LuceneError::IllegalArgument(format!(
            "Version is not in form major.minor.bugfix(.prerelease) (got: {})",
            version
        ))
    })?;
    if token.is_empty() {
        return Err(LuceneError::IllegalArgument(format!(
            "Version is not in form major.minor.bugfix(.prerelease) (got: {})",
            version
        )));
    }
    token.parse::<u8>().map_err(|e| {
        LuceneError::IllegalArgument(format!(
            "Failed to parse {} version from \"{}\" (got: {}): {}",
            name, token, version, e
        ))
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_values_identity_and_zeroes() {
        assert_eq!(IdentityLongValues.get(42), 42);
        assert_eq!(ZeroesLongValues.get(42), 0);
    }

    #[test]
    fn long_bit_set_basic() {
        let mut bs = LongBitSet::new(200).unwrap();
        assert_eq!(bs.length(), 200);
        assert!(!bs.get(50));

        bs.set(50);
        assert!(bs.get(50));
        assert_eq!(bs.cardinality(), 1);

        bs.clear(50);
        assert!(!bs.get(50));

        bs.set_range(10, 20);
        assert_eq!(bs.cardinality(), 10);
        assert!(bs.get(15));
        assert!(!bs.get(20));

        bs.clear_range(15, 18);
        assert!(!bs.get(15));
        assert!(bs.get(14));
        assert!(bs.get(18));

        bs.flip(100);
        assert!(bs.get(100));
        bs.flip(100);
        assert!(!bs.get(100));
    }

    #[test]
    fn long_bit_set_next_and_prev() {
        let mut bs = LongBitSet::new(1000).unwrap();
        bs.set(10);
        bs.set(100);
        bs.set(500);

        assert_eq!(bs.next_set_bit(0), 10);
        assert_eq!(bs.next_set_bit(11), 100);
        assert_eq!(bs.next_set_bit(101), 500);
        assert_eq!(bs.next_set_bit(501), -1);

        assert_eq!(bs.prev_set_bit(500), 500);
        assert_eq!(bs.prev_set_bit(499), 100);
        assert_eq!(bs.prev_set_bit(99), 10);
        assert_eq!(bs.prev_set_bit(9), -1);
    }

    #[test]
    fn long_bit_set_logical_ops() {
        let mut a = LongBitSet::new(64).unwrap();
        let mut b = LongBitSet::new(64).unwrap();
        a.set(1);
        a.set(2);
        b.set(2);
        b.set(3);

        let mut or = LongBitSet::new(64).unwrap();
        or.or(&a);
        or.or(&b);
        assert!(or.get(1) && or.get(2) && or.get(3));

        let mut and = a.clone();
        and.and(&b);
        assert!(!and.get(1));
        assert!(and.get(2));
        assert!(!and.get(3));

        let mut not_b = a.clone();
        not_b.and_not(&b);
        assert!(not_b.get(1));
        assert!(!not_b.get(2));
    }

    #[test]
    fn priority_queue_basic() {
        struct IntComparator;
        impl PriorityQueueComparator<i32> for IntComparator {
            fn less_than(&self, a: &i32, b: &i32) -> bool {
                a < b
            }
        }

        let mut pq = PriorityQueue::new(10, IntComparator).unwrap();
        for &v in [5, 3, 8, 1, 2].iter() {
            pq.add(v);
        }
        assert_eq!(pq.size(), 5);
        assert_eq!(pq.top(), Some(&1));

        let mut sorted = Vec::new();
        while let Some(v) = pq.pop() {
            sorted.push(v);
        }
        assert_eq!(sorted, vec![1, 2, 3, 5, 8]);
    }

    #[test]
    fn priority_queue_insert_with_overflow() {
        struct RevComparator;
        impl PriorityQueueComparator<i32> for RevComparator {
            fn less_than(&self, a: &i32, b: &i32) -> bool {
                a > b
            }
        }

        // Max-heap of size 3
        let mut pq = PriorityQueue::new(3, RevComparator).unwrap();
        for v in [1, 2, 3, 4, 0] {
            pq.insert_with_overflow(v);
        }
        let mut sorted = Vec::new();
        while let Some(v) = pq.pop() {
            sorted.push(v);
        }
        // Java insertWithOverflow on a max-heap keeps the smallest three values seen.
        assert_eq!(sorted, vec![2, 1, 0]);
    }

    #[test]
    fn priority_queue_remove() {
        struct IntComparator;
        impl PriorityQueueComparator<i32> for IntComparator {
            fn less_than(&self, a: &i32, b: &i32) -> bool {
                a < b
            }
        }

        let mut pq = PriorityQueue::new(10, IntComparator).unwrap();
        pq.add_all([3, 1, 4, 1, 5].iter().copied());
        // Remove one of the 1s
        let to_remove = 1;
        assert!(pq.remove(&to_remove));
        let mut sorted = Vec::new();
        while let Some(v) = pq.pop() {
            sorted.push(v);
        }
        assert_eq!(sorted, vec![1, 3, 4, 5]);
    }

    #[test]
    fn merged_iterator_dedup_and_all() {
        let a: Box<dyn Iterator<Item = i32>> = Box::new([1, 2, 3].into_iter());
        let b: Box<dyn Iterator<Item = i32>> = Box::new([2, 3, 4].into_iter());
        let merged = MergedIterator::with_remove_duplicates(true, [a, b]);
        assert_eq!(merged.collect::<Vec<_>>(), vec![1, 2, 3, 4]);

        let a: Box<dyn Iterator<Item = i32>> = Box::new([1, 2, 3].into_iter());
        let b: Box<dyn Iterator<Item = i32>> = Box::new([2, 3, 4].into_iter());
        let merged = MergedIterator::with_remove_duplicates(false, [a, b]);
        assert_eq!(merged.collect::<Vec<_>>(), vec![1, 2, 2, 3, 3, 4]);
    }

    #[test]
    fn version_constants_and_parse() {
        assert_eq!(Version::LATEST, Version::LUCENE_10_5_0);
        let v = Version::parse("10.5.0").unwrap();
        assert_eq!(v, Version::LUCENE_10_5_0);
        assert!(v.on_or_after(&Version::LUCENE_9_0_0));
        assert!(!Version::LUCENE_9_0_0.on_or_after(&v));

        let v2 = Version::parse_leniently("LUCENE_10_5_0").unwrap();
        assert_eq!(v2, Version::LUCENE_10_5_0);

        let v3 = Version::parse_leniently("LATEST").unwrap();
        assert_eq!(v3, Version::LATEST);
    }

    #[test]
    fn version_rejects_invalid() {
        assert!(Version::parse("10").is_err());
        assert!(Version::parse("10.5.0.0").is_err());
        // Components are u8, so 256 is rejected by the parser.
        assert!(Version::parse("256.0.0").is_err());
        assert!(Version::parse_leniently("LUCENE_BAD").is_err());
    }

    #[test]
    fn version_accepts_optional_components() {
        let v = Version::parse("10.5").unwrap();
        assert_eq!(v, Version::LUCENE_10_5_0);
    }
}
