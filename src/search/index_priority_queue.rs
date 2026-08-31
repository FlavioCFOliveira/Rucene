//! An index-addressed rendering of `org.apache.lucene.util.PriorityQueue`, used
//! by the boolean and disjunction scorers.
//!
//! # Why this exists
//!
//! Several scorers in this package build a
//! [`PriorityQueue`](crate::util::PriorityQueue) whose ordering reads a field
//! that the surrounding code mutates while the element sits in the heap — the
//! current doc ID of a [`DisiWrapper`](crate::search::DisiWrapper), or the next
//! doc a sub bulk scorer will produce. In Java the heap holds references, so the
//! comparison always sees the mutated value. Rust forbids that aliasing, so the
//! elements live in an array owned by the scorer and the heap holds
//! **positions** into it; every operation that compares receives the array.
//!
//! The heap layout is Lucene's: 1-based, with slot `0` unused, and the
//! `upHeap`/`downHeap` loops, `add`, `insertWithOverflow`, `pop`, `updateTop`
//! and `clear` are transcribed unchanged, so the resulting heap array — and
//! therefore the order in which equal elements come out — is identical.

#![deny(unsafe_code)]

use std::marker::PhantomData;

/// The ordering a [`IndexPriorityQueue`] sorts by.
///
/// Equivalent to the `protected abstract boolean lessThan(T a, T b)` a Java
/// subclass of `PriorityQueue` overrides.
pub trait IndexOrder<T> {
    /// Returns `true` if and only if `a` is less than `b`.
    fn less_than(a: &T, b: &T) -> bool;
}

/// A priority queue over positions into a caller-owned array.
///
/// See the [module documentation](self) for why the elements are addressed by
/// position.
#[derive(Debug)]
pub struct IndexPriorityQueue<T, O: IndexOrder<T>> {
    /// 1-based heap; slot `0` is unused, exactly as in Java.
    heap: Vec<usize>,
    size: usize,
    max_size: usize,
    marker: PhantomData<fn(&T, &O)>,
}

impl<T, O: IndexOrder<T>> IndexPriorityQueue<T, O> {
    /// Creates a queue of the given maximum size.
    ///
    /// Equivalent to `new PriorityQueue<>(int)`.
    pub fn new(max_size: usize) -> Self {
        // Java allocates one extra slot when maxSize is 0 so that `top()` needs
        // no size check; the same trick keeps the indexing below in range.
        let heap_size = if max_size == 0 { 2 } else { max_size + 1 };
        Self {
            heap: vec![usize::MAX; heap_size],
            size: 0,
            max_size,
            marker: PhantomData,
        }
    }

    /// Returns the number of entries.
    ///
    /// Equivalent to `PriorityQueue.size()`.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Returns the least entry, or `None` when the queue is empty.
    ///
    /// Equivalent to `PriorityQueue.top()`, which returns `null` when empty.
    pub fn top(&self) -> Option<usize> {
        if self.size == 0 {
            None
        } else {
            Some(self.heap[1])
        }
    }

    /// Returns the entry stored at 1-based heap slot `i + 1`.
    ///
    /// Equivalent to reading `getHeapArray()[1 + i]`, which
    /// `BooleanScorer.TailPriorityQueue.get(int)` does.
    ///
    /// # Panics
    ///
    /// Panics when `i` is not less than [`size`](Self::size), matching
    /// `Objects.checkIndex`.
    pub fn get(&self, i: usize) -> usize {
        assert!(
            i < self.size,
            "index {i} out of bounds for size {}",
            self.size
        );
        self.heap[1 + i]
    }

    /// Adds an entry and returns the new least entry.
    ///
    /// Equivalent to `PriorityQueue.add(T)`.
    ///
    /// # Panics
    ///
    /// Panics when the queue is full, which is the
    /// `ArrayIndexOutOfBoundsException` Java raises.
    pub fn add(&mut self, values: &[T], element: usize) -> usize {
        let index = self.size + 1;
        assert!(
            index < self.heap.len(),
            "cannot add to a full PriorityQueue of max size {}",
            self.max_size
        );
        self.heap[index] = element;
        self.size = index;
        self.up_heap(values, index);
        self.heap[1]
    }

    /// Adds every entry and heapifies once.
    ///
    /// Equivalent to `PriorityQueue.addAll(Collection<T>)`.
    ///
    /// # Panics
    ///
    /// Panics when the entries do not fit.
    pub fn add_all(&mut self, values: &[T], elements: impl IntoIterator<Item = usize>) {
        for element in elements {
            self.heap[self.size + 1] = element;
            self.size += 1;
        }
        // The loop goes down to 1 as heap is 1-based not 0-based.
        for i in (1..=(self.size >> 1)).rev() {
            self.down_heap(values, i);
        }
    }

    /// Adds an entry, evicting and returning the least entry when the queue is
    /// already full.
    ///
    /// Equivalent to `PriorityQueue.insertWithOverflow(T)`, which returns
    /// `null` when nothing was evicted and the argument itself when the queue
    /// is full and the argument is not competitive.
    pub fn insert_with_overflow(&mut self, values: &[T], element: usize) -> Option<usize> {
        if self.size < self.max_size {
            self.add(values, element);
            None
        } else if self.size > 0 && O::less_than(&values[self.heap[1]], &values[element]) {
            let ret = self.heap[1];
            self.heap[1] = element;
            self.update_top(values);
            Some(ret)
        } else {
            Some(element)
        }
    }

    /// Removes and returns the least entry.
    ///
    /// Equivalent to `PriorityQueue.pop()`.
    pub fn pop(&mut self, values: &[T]) -> Option<usize> {
        if self.size > 0 {
            let result = self.heap[1];
            self.heap[1] = self.heap[self.size];
            self.size -= 1;
            self.down_heap(values, 1);
            Some(result)
        } else {
            None
        }
    }

    /// Rebalances the heap and returns the least entry.
    ///
    /// Equivalent to `PriorityQueue.updateTop()`.
    pub fn update_top(&mut self, values: &[T]) -> Option<usize> {
        self.down_heap(values, 1);
        self.top()
    }

    /// Replaces the least entry and rebalances the heap.
    ///
    /// Equivalent to `PriorityQueue.updateTop(T)`.
    pub fn update_top_with(&mut self, values: &[T], new_top: usize) -> Option<usize> {
        self.heap[1] = new_top;
        self.update_top(values)
    }

    /// Removes every entry.
    ///
    /// Equivalent to `PriorityQueue.clear()`.
    pub fn clear(&mut self) {
        self.size = 0;
    }

    /// Returns the entries in heap-array order.
    ///
    /// Equivalent to `PriorityQueue.iterator()`, which walks slots `1..=size`.
    pub fn entries(&self) -> &[usize] {
        &self.heap[1..=self.size]
    }

    /// Equivalent to the private `PriorityQueue.upHeap(int)`.
    fn up_heap(&mut self, values: &[T], orig_pos: usize) -> bool {
        let mut i = orig_pos;
        let node = self.heap[i];
        let mut j = i >> 1;
        while j > 0 && O::less_than(&values[node], &values[self.heap[j]]) {
            self.heap[i] = self.heap[j];
            i = j;
            j >>= 1;
        }
        self.heap[i] = node;
        i != orig_pos
    }

    /// Equivalent to the private `PriorityQueue.downHeap(int)`.
    fn down_heap(&mut self, values: &[T], mut i: usize) {
        if self.size == 0 {
            return;
        }
        let node = self.heap[i];
        let mut j = i << 1;
        let mut k = j + 1;
        if k <= self.size && O::less_than(&values[self.heap[k]], &values[self.heap[j]]) {
            j = k;
        }
        while j <= self.size && O::less_than(&values[self.heap[j]], &values[node]) {
            self.heap[i] = self.heap[j];
            i = j;
            j = i << 1;
            k = j + 1;
            if k <= self.size && O::less_than(&values[self.heap[k]], &values[self.heap[j]]) {
                j = k;
            }
        }
        self.heap[i] = node;
    }
}
