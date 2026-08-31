//! Primitive priority queues over `i64` ported from
//! `org.apache.lucene.util`.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`LongHeap`] | `LongHeap` (2-ary) |
//! | [`TernaryLongHeap`] | `TernaryLongHeap` (3-ary) |
//!
//! Both are min heaps: the top element is the lowest value. Storage is 1-based,
//! `heap[0]` being unused, exactly as in Java.

#![deny(unsafe_code)]

use crate::error::{LuceneError, Result};
use crate::util::ArrayUtil;

// ---------------------------------------------------------------------------
// LongHeap
// ---------------------------------------------------------------------------

/// A binary min heap of `i64` values.
///
/// Port of `org.apache.lucene.util.LongHeap`. It grows without bound through
/// [`LongHeap::push`] and stays bounded by its initial capacity through
/// [`LongHeap::insert_with_overflow`].
#[derive(Debug, Clone)]
pub struct LongHeap {
    initial_capacity: usize,
    heap: Vec<i64>,
    size: usize,
}

impl LongHeap {
    /// Creates an empty heap with the given initial capacity.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `initial_capacity` is zero
    /// or at least `ArrayUtil::MAX_ARRAY_LENGTH`, mirroring the Java guard that
    /// exists to prevent a confusing `OutOfMemoryError`.
    pub fn new(initial_capacity: usize) -> Result<Self> {
        if !(1..ArrayUtil::MAX_ARRAY_LENGTH).contains(&initial_capacity) {
            return Err(LuceneError::IllegalArgument(format!(
                "initialCapacity must be > 0 and < {}; got: {}",
                ArrayUtil::MAX_ARRAY_LENGTH - 1,
                initial_capacity
            )));
        }
        // The `+ 1` is because all access to the heap is 1-based, not 0-based.
        Ok(Self {
            initial_capacity,
            heap: vec![0; initial_capacity + 1],
            size: 0,
        })
    }

    /// Creates a heap of `size` elements all initialised to `initial_value`.
    ///
    /// Equivalent to `new LongHeap(int, long)`.
    ///
    /// # Errors
    ///
    /// As [`LongHeap::new`].
    pub fn filled(size: usize, initial_value: i64) -> Result<Self> {
        let mut heap = Self::new(size)?;
        for slot in heap.heap[1..=size].iter_mut() {
            *slot = initial_value;
        }
        heap.size = size;
        Ok(heap)
    }

    /// Adds a value in `log(size)` time, growing as needed, and returns the new
    /// top of the heap.
    pub fn push(&mut self, element: i64) -> i64 {
        self.size += 1;
        if self.size == self.heap.len() {
            // Java grows to `(size * 3 + 1) / 2`, which for a non-negative
            // size is exactly `ceil(size * 3 / 2)`.
            let want = (self.size * 3).div_ceil(2);
            let target = ArrayUtil::oversize(want, 8).max(want);
            self.heap.resize(target, 0);
        }
        self.heap[self.size] = element;
        self.up_heap(self.size);
        self.heap[1]
    }

    /// Adds a value, discarding the least value when the heap is already at its
    /// initial capacity.
    ///
    /// Returns whether the value was added.
    pub fn insert_with_overflow(&mut self, value: i64) -> bool {
        if self.size >= self.initial_capacity {
            if value < self.heap[1] {
                return false;
            }
            self.update_top(value);
            return true;
        }
        self.push(value);
        true
    }

    /// Returns the least element in constant time.
    ///
    /// It is up to the caller to check that the heap is not empty; no check is
    /// done, and `0` is returned when nothing was added.
    pub fn top(&self) -> i64 {
        self.heap[1]
    }

    /// Removes and returns the least element in `log(size)` time.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the heap is empty, which is
    /// Java's `IllegalStateException("The heap is empty")`.
    pub fn pop(&mut self) -> Result<i64> {
        if self.size > 0 {
            let result = self.heap[1];
            self.heap[1] = self.heap[self.size];
            self.size -= 1;
            self.down_heap(1);
            Ok(result)
        } else {
            Err(LuceneError::IllegalState("The heap is empty".to_string()))
        }
    }

    /// Replaces the top of the heap and returns the new top.
    ///
    /// Calling this on an empty heap has no visible effect.
    pub fn update_top(&mut self, value: i64) -> i64 {
        self.heap[1] = value;
        self.down_heap(1);
        self.heap[1]
    }

    /// Returns the number of elements currently stored.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Returns whether the heap holds no element.
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    /// Removes all entries.
    pub fn clear(&mut self) {
        self.size = 0;
    }

    /// Pushes every element of `other` onto this heap.
    pub fn push_all(&mut self, other: &LongHeap) {
        for i in 1..=other.size {
            self.push(other.heap[i]);
        }
    }

    /// Returns the element at the `i`-th location of the heap array, for
    /// iterating when the order does not matter. Valid arguments are `1..=size`.
    pub fn get(&self, i: usize) -> i64 {
        self.heap[i]
    }

    /// Returns the internal heap array.
    pub fn get_heap_array(&self) -> &[i64] {
        &self.heap
    }

    fn up_heap(&mut self, orig_pos: usize) {
        let mut i = orig_pos;
        let value = self.heap[i];
        let mut j = i >> 1;
        while j > 0 && value < self.heap[j] {
            self.heap[i] = self.heap[j];
            i = j;
            j >>= 1;
        }
        self.heap[i] = value;
    }

    fn down_heap(&mut self, i: usize) {
        let mut i = i;
        let value = self.heap[i];
        let mut j = i << 1;
        let mut k = j + 1;
        if k <= self.size && self.heap[k] < self.heap[j] {
            j = k;
        }
        while j <= self.size && self.heap[j] < value {
            self.heap[i] = self.heap[j];
            i = j;
            j = i << 1;
            k = j + 1;
            if k <= self.size && self.heap[k] < self.heap[j] {
                j = k;
            }
        }
        self.heap[i] = value;
    }
}

// ---------------------------------------------------------------------------
// TernaryLongHeap
// ---------------------------------------------------------------------------

/// Number of children per node. `TernaryLongHeap.ARITY`.
const ARITY: usize = 3;

/// A ternary min heap of `i64` values.
///
/// Port of `org.apache.lucene.util.TernaryLongHeap`.
#[derive(Debug, Clone)]
pub struct TernaryLongHeap {
    initial_capacity: usize,
    heap: Vec<i64>,
    size: usize,
}

impl TernaryLongHeap {
    /// Creates an empty heap with the given initial capacity.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `initial_capacity` is zero
    /// or at least `ArrayUtil::MAX_ARRAY_LENGTH`.
    pub fn new(initial_capacity: usize) -> Result<Self> {
        if !(1..ArrayUtil::MAX_ARRAY_LENGTH).contains(&initial_capacity) {
            return Err(LuceneError::IllegalArgument(format!(
                "initialCapacity must be > 0 and < {}; got: {}",
                ArrayUtil::MAX_ARRAY_LENGTH - 1,
                initial_capacity
            )));
        }
        Ok(Self {
            initial_capacity,
            heap: vec![0; initial_capacity + 1],
            size: 0,
        })
    }

    /// Creates a heap of `size` elements all initialised to `initial_value`.
    ///
    /// Equivalent to `new TernaryLongHeap(int, long)`, including its
    /// `size <= 0 ? 1 : size` capacity guard.
    ///
    /// # Errors
    ///
    /// As [`TernaryLongHeap::new`].
    pub fn filled(size: usize, initial_value: i64) -> Result<Self> {
        let mut heap = Self::new(if size == 0 { 1 } else { size })?;
        for slot in heap.heap[1..=size].iter_mut() {
            *slot = initial_value;
        }
        heap.size = size;
        Ok(heap)
    }

    /// Adds a value in `log_3(size)` time, growing as needed, and returns the
    /// new top of the heap.
    pub fn push(&mut self, element: i64) -> i64 {
        self.size += 1;
        if self.size == self.heap.len() {
            // Java grows to `(size * 3 + 1) / 2`, which for a non-negative
            // size is exactly `ceil(size * 3 / 2)`.
            let want = (self.size * 3).div_ceil(2);
            let target = ArrayUtil::oversize(want, 8).max(want);
            self.heap.resize(target, 0);
        }
        self.heap[self.size] = element;
        Self::up_heap(&mut self.heap, self.size, ARITY);
        self.heap[1]
    }

    /// Adds a value, discarding the least value when the heap is already at its
    /// initial capacity.
    ///
    /// Returns whether the value was added.
    pub fn insert_with_overflow(&mut self, value: i64) -> bool {
        if self.size >= self.initial_capacity {
            if value < self.heap[1] {
                return false;
            }
            self.update_top(value);
            return true;
        }
        self.push(value);
        true
    }

    /// Returns the least element in constant time.
    pub fn top(&self) -> i64 {
        self.heap[1]
    }

    /// Removes and returns the least element.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the heap is empty.
    pub fn pop(&mut self) -> Result<i64> {
        if self.size > 0 {
            let result = self.heap[1];
            self.heap[1] = self.heap[self.size];
            self.size -= 1;
            Self::down_heap(&mut self.heap, 1, self.size, ARITY);
            Ok(result)
        } else {
            Err(LuceneError::IllegalState("The heap is empty".to_string()))
        }
    }

    /// Replaces the top of the heap and returns the new top.
    pub fn update_top(&mut self, value: i64) -> i64 {
        self.heap[1] = value;
        Self::down_heap(&mut self.heap, 1, self.size, ARITY);
        self.heap[1]
    }

    /// Returns the number of elements currently stored.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Returns whether the heap holds no element.
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    /// Removes all entries.
    pub fn clear(&mut self) {
        self.size = 0;
    }

    /// Pushes every element of `other` onto this heap.
    pub fn push_all(&mut self, other: &TernaryLongHeap) {
        for i in 1..=other.size {
            self.push(other.heap[i]);
        }
    }

    /// Returns the element at the `i`-th location of the heap array. Valid
    /// arguments are `1..=size`.
    pub fn get(&self, i: usize) -> i64 {
        self.heap[i]
    }

    /// Returns the internal heap array.
    pub fn get_heap_array(&self) -> &[i64] {
        &self.heap
    }

    /// Moves the element at `i` up until it finds its place, for a heap of any
    /// arity. Equivalent to the static `TernaryLongHeap.upHeap`.
    pub fn up_heap(heap: &mut [i64], i: usize, arity: usize) {
        let mut i = i;
        let value = heap[i];
        while i > 1 {
            // Parent formula for 1-based indexing.
            let parent = ((i - 2) / arity) + 1;
            let parent_val = heap[parent];
            if value >= parent_val {
                break;
            }
            heap[i] = parent_val;
            i = parent;
        }
        heap[i] = value;
    }

    /// Moves the element at `i` down until it finds its place, for a heap of
    /// any arity. Equivalent to the static `TernaryLongHeap.downHeap`.
    pub fn down_heap(heap: &mut [i64], i: usize, size: usize, arity: usize) {
        let mut i = i;
        let value = heap[i];
        loop {
            // First-child formula for 1-based indexing.
            let first_child = arity * (i - 1) + 2;
            if first_child > size {
                // `i` is a leaf.
                break;
            }

            let last_child = (first_child + arity - 1).min(size);

            // Find the smallest child in `[first_child, last_child]`.
            let mut best = first_child;
            let mut best_val = heap[first_child];

            for (offset, &v) in heap[(first_child + 1)..=last_child].iter().enumerate() {
                if v < best_val {
                    best_val = v;
                    best = first_child + 1 + offset;
                }
            }

            if best_val >= value {
                break;
            }

            heap[i] = best_val;
            i = best;
        }
        heap[i] = value;
    }
}
