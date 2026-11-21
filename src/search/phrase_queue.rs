//! The priority queue of a sloppy phrase match, ported from
//! `org.apache.lucene.search.PhraseQueue`.

#![deny(unsafe_code)]

use crate::search::phrase_positions::PhrasePositions;

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
    heap: Vec<Option<usize>>,
    size: usize,
}

impl PhraseQueue {
    /// Creates an empty queue that can hold `size` phrase positions.
    ///
    /// Equivalent to `PhraseQueue(int)`.
    pub fn new(size: usize) -> Self {
        let heap_size = if size == 0 { 2 } else { size + 1 };
        Self {
            heap: (0..heap_size).map(|_| None).collect(),
            size: 0,
        }
    }

    /// Orders two phrase positions.
    ///
    /// Equivalent to the `final PhraseQueue.lessThan(PhrasePositions,
    /// PhrasePositions)`.
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
    pub fn top(&self) -> Option<usize> {
        self.heap[1]
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
        for slot in self.heap.iter_mut() {
            *slot = None;
        }
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
    pub fn add(&mut self, pps: &[PhrasePositions], element: usize) {
        let index = self.size + 1;
        self.heap[index] = Some(element);
        self.size = index;
        self.up_heap(pps, index);
    }

    /// Removes and returns the least element in `O(log size)` time.
    ///
    /// Equivalent to `PriorityQueue.pop()`.
    pub fn pop(&mut self, pps: &[PhrasePositions]) -> Option<usize> {
        if self.size == 0 {
            return None;
        }
        let result = self.heap[1].take();
        if self.size > 1 {
            self.heap[1] = self.heap[self.size].take();
            self.size -= 1;
            self.down_heap(pps, 1);
        } else {
            self.size = 0;
        }
        result
    }

    /// Re-establishes the heap after the top element has been mutated in place.
    ///
    /// Equivalent to `PriorityQueue.updateTop()`.
    pub fn update_top(&mut self, pps: &[PhrasePositions]) -> Option<usize> {
        self.down_heap(pps, 1);
        self.heap[1]
    }

    fn up_heap(&mut self, pps: &[PhrasePositions], orig_pos: usize) {
        let mut i = orig_pos;
        let node = self.heap[i]
            .take()
            .expect("INVARIANT: up_heap starts from an occupied slot");
        let mut j = i >> 1;
        while j > 0
            && Self::less_than(
                pps,
                node,
                self.heap[j].expect("INVARIANT: slots 1..=size are occupied"),
            )
        {
            self.heap[i] = self.heap[j].take();
            i = j;
            j >>= 1;
        }
        self.heap[i] = Some(node);
    }

    fn down_heap(&mut self, pps: &[PhrasePositions], mut i: usize) {
        let node = self.heap[i]
            .take()
            .expect("INVARIANT: down_heap starts from an occupied slot");
        let occupied = "INVARIANT: slots 1..=size are occupied";
        let mut j = i << 1;
        let mut k = j + 1;
        if k <= self.size
            && Self::less_than(
                pps,
                self.heap[k].expect(occupied),
                self.heap[j].expect(occupied),
            )
        {
            j = k;
        }
        while j <= self.size && Self::less_than(pps, self.heap[j].expect(occupied), node) {
            self.heap[i] = self.heap[j].take();
            i = j;
            j = i << 1;
            k = j + 1;
            if k <= self.size
                && Self::less_than(
                    pps,
                    self.heap[k].expect(occupied),
                    self.heap[j].expect(occupied),
                )
            {
                j = k;
            }
        }
        self.heap[i] = Some(node);
    }
}
