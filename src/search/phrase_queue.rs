//! The priority queue of a sloppy phrase match, ported from
//! `org.apache.lucene.search.PhraseQueue`.

#![deny(unsafe_code)]

use crate::search::phrase_positions::PhrasePositions;

/// The value of an unoccupied heap slot: Java's `null`.
const EMPTY: usize = usize::MAX;

/// A priority queue over the [`PhrasePositions`] of a sloppy phrase match.
///
/// Equivalent to the `final org.apache.lucene.search.PhraseQueue`, which
/// extends `org.apache.lucene.util.PriorityQueue<PhrasePositions>`.
///
/// **Divergence from Lucene 10.5.0.** Java's queue holds the
/// `PhrasePositions` objects themselves, and
/// [`SloppyPhraseMatcher`](crate::search::SloppyPhraseMatcher) mutates the very
/// objects it has already handed to the queue. Rust forbids that aliasing, so
/// the queue holds the *ordinals* of the phrase positions and the matcher keeps
/// the objects in one slice; every method therefore takes that slice, which is
/// what `lessThan` reads. `up_heap` and `down_heap` reproduce
/// `org.apache.lucene.util.PriorityQueue` exactly, including the unused slot at
/// index 0.
#[derive(Debug)]
pub struct PhraseQueue {
    /// Slots `1..=size` hold the ordinals of the queued phrase positions;
    /// the other slots hold [`EMPTY`], Java's `null`.
    ///
    /// PERF: plain ordinals rather than `Option<usize>`, so moving an
    /// element is one word store and no `take()`; the slot values and the
    /// heap order are the same as with `Option`.
    heap: Vec<usize>,
    size: usize,
}

impl PhraseQueue {
    /// Creates an empty queue that can hold `size` phrase positions.
    ///
    /// Equivalent to `PhraseQueue(int)`.
    pub fn new(size: usize) -> Self {
        let heap_size = if size == 0 { 2 } else { size + 1 };
        Self {
            heap: vec![EMPTY; heap_size],
            size: 0,
        }
    }

    /// Orders two phrase positions.
    ///
    /// Equivalent to the `final PhraseQueue.lessThan(PhrasePositions,
    /// PhrasePositions)`.
    #[inline]
    pub fn less_than(pps: &[PhrasePositions], a: usize, b: usize) -> bool {
        let (pp1, pp2) = (&pps[a], &pps[b]);
        if pp1.position == pp2.position {
            // Same doc and position, so decide by the actual term positions;
            // this relies on `pp.position == tp.position - offset`.
            if pp1.offset == pp2.offset {
                pp1.ord < pp2.ord
            } else {
                pp1.offset < pp2.offset
            }
        } else {
            pp1.position < pp2.position
        }
    }

    /// Returns the least element without removing it.
    ///
    /// Equivalent to `PriorityQueue.top()`.
    #[inline]
    pub fn top(&self) -> Option<usize> {
        Some(self.heap[1]).filter(|&top| top != EMPTY)
    }

    /// Returns the number of elements currently stored.
    ///
    /// Equivalent to `PriorityQueue.size()`.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Removes all the elements from the queue.
    ///
    /// Equivalent to `PriorityQueue.clear()`.
    pub fn clear(&mut self) {
        self.heap.fill(EMPTY);
        self.size = 0;
    }

    /// Adds an element in `O(log size)` time.
    ///
    /// Equivalent to `PriorityQueue.add(T)`.
    ///
    /// # Panics
    ///
    /// Panics when the queue is already full, which is the
    /// `IndexOutOfBoundsException` Java raises.
    #[inline]
    pub fn add(&mut self, pps: &[PhrasePositions], element: usize) {
        let index = self.size + 1;
        self.heap[index] = element;
        self.size = index;
        self.up_heap(pps, index);
    }

    /// Removes and returns the least element in `O(log size)` time.
    ///
    /// Equivalent to `PriorityQueue.pop()`.
    #[inline]
    pub fn pop(&mut self, pps: &[PhrasePositions]) -> Option<usize> {
        if self.size == 0 {
            return None;
        }
        let result = self.heap[1];
        if self.size > 1 {
            self.heap[1] = self.heap[self.size];
            self.heap[self.size] = EMPTY;
            self.size -= 1;
            self.down_heap(pps, 1);
        } else {
            self.heap[1] = EMPTY;
            self.size = 0;
        }
        Some(result)
    }

    /// Re-establishes the heap after the top element has been mutated in place.
    ///
    /// Equivalent to `PriorityQueue.updateTop()`.
    pub fn update_top(&mut self, pps: &[PhrasePositions]) -> Option<usize> {
        self.down_heap(pps, 1);
        self.top()
    }

    /// Equivalent to `PriorityQueue.upHeap(int)`.
    fn up_heap(&mut self, pps: &[PhrasePositions], orig_pos: usize) {
        let mut i = orig_pos;
        let node = self.heap[i];
        debug_assert!(node != EMPTY, "up_heap starts from an occupied slot");
        let mut j = i >> 1;
        while j > 0 && Self::less_than(pps, node, self.heap[j]) {
            self.heap[i] = self.heap[j];
            i = j;
            j >>= 1;
        }
        self.heap[i] = node;
    }

    /// Equivalent to `PriorityQueue.downHeap(int)`.
    fn down_heap(&mut self, pps: &[PhrasePositions], mut i: usize) {
        let node = self.heap[i];
        debug_assert!(node != EMPTY, "down_heap starts from an occupied slot");
        let size = self.size;
        let mut j = i << 1;
        let mut k = j + 1;
        if k <= size && Self::less_than(pps, self.heap[k], self.heap[j]) {
            j = k;
        }
        while j <= size && Self::less_than(pps, self.heap[j], node) {
            self.heap[i] = self.heap[j];
            i = j;
            j = i << 1;
            k = j + 1;
            if k <= size && Self::less_than(pps, self.heap[k], self.heap[j]) {
                j = k;
            }
        }
        self.heap[i] = node;
    }
}
