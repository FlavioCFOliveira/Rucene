//! Neighbor storage and priority queues for HNSW graphs.
//!
//! Equivalent to `org.apache.lucene.util.hnsw.NeighborArray` and
//! `org.apache.lucene.util.hnsw.NeighborQueue`.

#![deny(unsafe_code)]

use std::collections::BinaryHeap;

use crate::error::LuceneError;
use crate::util::NumericUtils;

use super::scorer::{RandomVectorScorer, UpdateableRandomVectorScorer};
use super::Result;

/// Growable array of neighbor node ids and their scores.
///
/// Nodes are maintained in score order: descending when `scores_desc_order`
/// is true, ascending otherwise. The array supports both sorted and unsorted
/// insertions; unsorted nodes are sorted lazily on demand.
///
/// Equivalent to `org.apache.lucene.util.hnsw.NeighborArray`.
#[derive(Clone, Debug)]
pub struct NeighborArray {
    scores_desc_order: bool,
    size: usize,
    max_size: usize,
    nodes: Vec<i32>,
    scores: Vec<f32>,
    sorted_node_size: usize,
}

impl NeighborArray {
    /// Creates an empty array with the given maximum size and ordering.
    pub fn new(max_size: i32, desc_order: bool) -> Self {
        let max_size = max_size.max(0) as usize;
        Self {
            scores_desc_order: desc_order,
            size: 0,
            max_size,
            nodes: Vec::with_capacity(max_size),
            scores: Vec::with_capacity(max_size),
            sorted_node_size: 0,
        }
    }

    /// Returns the configured maximum size.
    pub fn max_size(&self) -> i32 {
        self.max_size as i32
    }

    /// Returns the current number of stored neighbors.
    pub fn size(&self) -> i32 {
        self.size as i32
    }

    /// Returns a reference to the internal node buffer (only the first
    /// `size()` entries are valid).
    pub fn nodes(&self) -> &[i32] {
        &self.nodes[..self.size]
    }

    /// Returns the score at index `i`.
    pub fn score(&self, i: i32) -> f32 {
        self.scores[i as usize]
    }

    /// Adds a node that is known to be worse than all previously stored
    /// nodes. Panics (in debug) if ordering is violated; returns an error
    /// if the array is full.
    pub fn add_in_order(&mut self, new_node: i32, new_score: f32) -> Result<()> {
        debug_assert_eq!(
            self.size, self.sorted_node_size,
            "cannot call add_in_order after add_out_of_order"
        );
        if self.size == self.max_size {
            return Err(LuceneError::IllegalState(
                "No growth is allowed".to_string(),
            ));
        }
        if self.size > 0 {
            let previous_score = self.scores[self.size - 1];
            debug_assert!(
                (self.scores_desc_order && previous_score >= new_score)
                    || (!self.scores_desc_order && previous_score <= new_score),
                "nodes are added in incorrect order"
            );
        }
        self.nodes.push(new_node);
        self.scores.push(new_score);
        self.size += 1;
        self.sorted_node_size += 1;
        Ok(())
    }

    /// Adds a node without preserving sorted order.
    pub fn add_out_of_order(&mut self, new_node: i32, new_score: f32) -> Result<()> {
        if self.size == self.max_size {
            return Err(LuceneError::IllegalState(
                "No growth is allowed".to_string(),
            ));
        }
        self.nodes.push(new_node);
        self.scores.push(new_score);
        self.size += 1;
        Ok(())
    }

    /// Adds an out-of-order node and, if the array overflows, removes the
    /// least-diverse neighbor using the provided scorer.
    pub fn add_and_ensure_diversity(
        &mut self,
        new_node: i32,
        new_score: f32,
        node_id: i32,
        scorer: &mut dyn UpdateableRandomVectorScorer,
    ) -> Result<()> {
        self.add_out_of_order(new_node, new_score)?;
        if self.size < self.max_size {
            return Ok(());
        }
        scorer.set_scoring_ordinal(node_id)?;
        let worst = self.find_worst_non_diverse(scorer)?;
        self.remove_index(worst);
        debug_assert_eq!(self.size, self.max_size - 1);
        Ok(())
    }

    /// Sorts the array and returns the sorted indexes of the previously
    /// unsorted nodes, or `None` if already fully sorted.
    pub fn sort(&mut self, scorer: &mut dyn RandomVectorScorer) -> Result<Option<Vec<i32>>> {
        if self.size == self.sorted_node_size {
            return Ok(None);
        }
        debug_assert!(self.sorted_node_size < self.size);
        let mut unchecked_indexes = Vec::with_capacity(self.size - self.sorted_node_size);
        let mut count = 0usize;
        while self.sorted_node_size != self.size {
            let idx = self.insert_sorted_internal(scorer)?;
            for prev in &mut unchecked_indexes[..count] {
                if *prev >= idx {
                    *prev += 1;
                }
            }
            unchecked_indexes.push(idx);
            count += 1;
        }
        unchecked_indexes.sort_unstable();
        Ok(Some(unchecked_indexes))
    }

    /// Test helper: insert a node and immediately sort it.
    #[cfg(test)]
    pub fn insert_sorted(&mut self, new_node: i32, new_score: f32) -> Result<()> {
        self.add_out_of_order(new_node, new_score)?;
        let _ = self.insert_sorted_internal(&mut DummyScorer)?;
        Ok(())
    }

    fn insert_sorted_internal(&mut self, scorer: &mut dyn RandomVectorScorer) -> Result<i32> {
        debug_assert!(self.sorted_node_size < self.size);
        let tmp_node = self.nodes[self.sorted_node_size];
        let mut tmp_score = self.scores[self.sorted_node_size];

        if tmp_score.is_nan() {
            tmp_score = scorer.score(tmp_node)?;
        }

        let insertion_point = if self.scores_desc_order {
            self.desc_sort_find_rightmost_insertion_point(tmp_score, self.sorted_node_size)
        } else {
            self.asc_sort_find_rightmost_insertion_point(tmp_score, self.sorted_node_size)
        };

        // Shift the already-sorted elements to the right, mirroring Java's
        // `System.arraycopy`, then write the new element at the insertion point.
        if insertion_point < self.sorted_node_size {
            self.nodes
                .copy_within(insertion_point..self.sorted_node_size, insertion_point + 1);
            self.scores
                .copy_within(insertion_point..self.sorted_node_size, insertion_point + 1);
        }
        self.nodes[insertion_point] = tmp_node;
        self.scores[insertion_point] = tmp_score;
        self.sorted_node_size += 1;
        Ok(insertion_point as i32)
    }

    fn asc_sort_find_rightmost_insertion_point(&self, new_score: f32, sorted_size: usize) -> usize {
        self.scores[..sorted_size].partition_point(|s| *s <= new_score)
    }

    fn desc_sort_find_rightmost_insertion_point(
        &self,
        new_score: f32,
        sorted_size: usize,
    ) -> usize {
        self.scores[..sorted_size].partition_point(|s| *s >= new_score)
    }

    /// Clears all entries.
    pub fn clear(&mut self) {
        self.size = 0;
        self.sorted_node_size = 0;
        self.nodes.clear();
        self.scores.clear();
    }

    /// Removes the last entry.
    pub fn remove_last(&mut self) {
        if self.size > 0 {
            self.size -= 1;
            self.nodes.pop();
            self.scores.pop();
            self.sorted_node_size = self.sorted_node_size.min(self.size);
        }
    }

    /// Removes the entry at `idx`.
    pub fn remove_index(&mut self, idx: usize) {
        if idx == self.size - 1 {
            self.remove_last();
            return;
        }
        self.nodes.remove(idx);
        self.scores.remove(idx);
        if idx < self.sorted_node_size {
            self.sorted_node_size -= 1;
        }
        self.size -= 1;
    }

    fn find_worst_non_diverse(
        &mut self,
        scorer: &mut dyn UpdateableRandomVectorScorer,
    ) -> Result<usize> {
        let unchecked_indexes = self
            .sort(scorer)?
            .expect("we will always have something unchecked");
        let mut unchecked_cursor = unchecked_indexes.len().saturating_sub(1) as i32;
        for i in (1..self.size).rev() {
            if unchecked_cursor < 0 {
                break;
            }
            let unchecked_idx = unchecked_indexes[unchecked_cursor as usize] as usize;
            scorer.set_scoring_ordinal(self.nodes[i])?;
            if self.is_worst_non_diverse(
                i,
                &unchecked_indexes,
                unchecked_cursor as usize,
                scorer,
            )? {
                return Ok(i);
            }
            if i == unchecked_idx {
                unchecked_cursor -= 1;
            }
        }
        Ok(self.size - 1)
    }

    fn is_worst_non_diverse(
        &self,
        candidate_index: usize,
        unchecked_indexes: &[i32],
        unchecked_cursor: usize,
        scorer: &mut dyn RandomVectorScorer,
    ) -> Result<bool> {
        let min_accepted_similarity = self.scores[candidate_index];
        let candidate_is_unchecked =
            candidate_index == unchecked_indexes[unchecked_cursor] as usize;
        if candidate_is_unchecked {
            for i in (0..candidate_index).rev() {
                let neighbor_similarity = scorer.score(self.nodes[i])?;
                if neighbor_similarity >= min_accepted_similarity {
                    return Ok(true);
                }
            }
        } else {
            debug_assert!(candidate_index > unchecked_indexes[unchecked_cursor] as usize);
            for i in (0..=unchecked_cursor).rev() {
                let unchecked_idx = unchecked_indexes[i] as usize;
                let neighbor_similarity = scorer.score(self.nodes[unchecked_idx])?;
                if neighbor_similarity >= min_accepted_similarity {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
}

/// A simple `RandomVectorScorer` used only by the test-only `insert_sorted`.
#[cfg(test)]
struct DummyScorer;

#[cfg(test)]
impl RandomVectorScorer for DummyScorer {
    fn score(&mut self, _node: i32) -> Result<f32> {
        Ok(0.0)
    }

    fn bulk_score(&mut self, _nodes: &[i32], _scores: &mut [f32], _num_nodes: i32) -> Result<f32> {
        Ok(f32::NEG_INFINITY)
    }

    fn max_ord(&self) -> i32 {
        0
    }
}

/// Priority queue of graph arcs encoded as a single packed long.
///
/// The queue supports both unbounded growth and bounded insertion with
/// overflow. A min-heap keeps the worst score on top; a max-heap keeps the
/// best score on top.
///
/// Equivalent to `org.apache.lucene.util.hnsw.NeighborQueue`.
#[derive(Clone, Debug)]
pub struct NeighborQueue {
    heap: BinaryHeap<u64>,
    max_heap: bool,
    visited_count: i32,
    incomplete: bool,
    capacity: usize,
}

impl NeighborQueue {
    /// Creates a new queue. `max_heap` selects whether the top element is the
    /// best score (true) or the worst score (false).
    pub fn new(initial_size: i32, max_heap: bool) -> Self {
        let capacity = initial_size.max(1) as usize;
        Self {
            heap: BinaryHeap::with_capacity(capacity),
            max_heap,
            visited_count: 0,
            incomplete: false,
            capacity,
        }
    }

    /// Returns the number of elements in the queue.
    pub fn size(&self) -> i32 {
        self.heap.len() as i32
    }

    /// Adds a new graph arc, extending storage as needed.
    pub fn add(&mut self, new_node: i32, new_score: f32) {
        self.heap.push(self.encode(new_node, new_score));
    }

    /// Bounded insertion: if the queue is full, the new arc is kept only if it
    /// is better than the current top. Returns whether it was kept.
    pub fn insert_with_overflow(&mut self, new_node: i32, new_score: f32) -> bool {
        let encoded = self.encode(new_node, new_score);
        if self.heap.len() >= self.capacity {
            if self.is_better(encoded, self.top_raw()) {
                // Replace top and sift down is handled by popping then pushing.
                self.heap.pop();
                self.heap.push(encoded);
                return true;
            }
            return false;
        }
        self.heap.push(encoded);
        true
    }

    fn encode(&self, node: i32, score: f32) -> u64 {
        let sortable = (NumericUtils::float_to_sortable_int(score) as u64) << 32;
        let node_bits = (0xFFFF_FFFFu64) & (!(node as u64));
        let packed = sortable | node_bits;
        self.apply_order(packed)
    }

    fn decode_score(&self, heap_value: u64) -> f32 {
        let raw = self.apply_order(heap_value);
        let sortable = (raw >> 32) as i32;
        NumericUtils::sortable_int_to_float(sortable)
    }

    fn decode_node(&self, heap_value: u64) -> i32 {
        let raw = self.apply_order(heap_value);
        !(raw as i32)
    }

    /// Maps a raw encoded value to the value stored in the heap. Rust's
    /// `BinaryHeap` is a max-heap by default, so min-heap semantics require
    /// the raw order to be inverted. The transformation is its own inverse,
    /// matching Lucene's `Order.MIN_HEAP`/`MAX_HEAP`.
    fn apply_order(&self, v: u64) -> u64 {
        if self.max_heap {
            v
        } else {
            u64::MAX - v
        }
    }

    fn is_better(&self, candidate: u64, top: u64) -> bool {
        if self.max_heap {
            candidate > top
        } else {
            candidate < top
        }
    }

    /// Removes and returns the top node id.
    pub fn pop(&mut self) -> i32 {
        let top = self.heap.pop().expect("queue is empty");
        self.decode_node(top)
    }

    /// Returns all node ids currently stored (order is unspecified).
    pub fn nodes(&self) -> Vec<i32> {
        self.heap.iter().map(|&v| self.decode_node(v)).collect()
    }

    /// Returns the top node id.
    pub fn top_node(&self) -> i32 {
        self.decode_node(*self.heap.peek().expect("queue is empty"))
    }

    fn top_raw(&self) -> u64 {
        *self.heap.peek().expect("queue is empty")
    }

    /// Returns the top score.
    pub fn top_score(&self) -> f32 {
        self.decode_score(*self.heap.peek().expect("queue is empty"))
    }

    /// Clears the queue.
    pub fn clear(&mut self) {
        self.heap.clear();
        self.visited_count = 0;
        self.incomplete = false;
    }

    /// Returns the number of visited nodes tracked by the queue.
    pub fn visited_count(&self) -> i32 {
        self.visited_count
    }

    /// Sets the visited-node counter.
    pub fn set_visited_count(&mut self, count: i32) {
        self.visited_count = count;
    }

    /// Returns true if the search stopped early because it reached the visited
    /// nodes limit.
    pub fn incomplete(&self) -> bool {
        self.incomplete
    }

    /// Marks the result set as incomplete.
    pub fn mark_incomplete(&mut self) {
        self.incomplete = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neighbor_queue_min_heap_ordering() {
        let mut q = NeighborQueue::new(3, false);
        q.add(1, 0.5);
        q.add(2, 0.9);
        q.add(3, 0.1);
        assert_eq!(q.top_node(), 3); // worst score on top
        assert_eq!(q.pop(), 3);
        assert_eq!(q.top_node(), 1);
    }

    #[test]
    fn neighbor_queue_max_heap_ordering() {
        let mut q = NeighborQueue::new(3, true);
        q.add(1, 0.5);
        q.add(2, 0.9);
        q.add(3, 0.1);
        assert_eq!(q.top_node(), 2); // best score on top
        assert_eq!(q.pop(), 2);
        assert_eq!(q.top_node(), 1);
    }

    #[test]
    fn neighbor_queue_insert_with_overflow() {
        let mut q = NeighborQueue::new(2, false);
        assert!(q.insert_with_overflow(1, 0.5));
        assert!(q.insert_with_overflow(2, 0.9));
        // 0.1 is worse than the current worst (0.5), so it is rejected.
        assert!(!q.insert_with_overflow(3, 0.1));
        // 0.95 is better than the current worst (0.5), so it replaces it.
        assert!(q.insert_with_overflow(4, 0.95));
        assert_eq!(q.nodes().len(), 2);
        assert!(q.nodes().contains(&2));
        assert!(q.nodes().contains(&4));
    }

    #[test]
    fn neighbor_array_in_order_insertion() {
        let mut arr = NeighborArray::new(5, true);
        arr.add_in_order(1, 0.9).unwrap();
        arr.add_in_order(2, 0.7).unwrap();
        arr.add_in_order(3, 0.5).unwrap();
        assert_eq!(arr.nodes(), &[1, 2, 3]);
    }

    #[test]
    fn neighbor_array_out_of_order_then_sort() {
        let mut arr = NeighborArray::new(5, true);
        arr.add_out_of_order(3, 0.5).unwrap();
        arr.add_out_of_order(1, 0.9).unwrap();
        arr.add_out_of_order(2, 0.7).unwrap();
        assert_ne!(arr.nodes(), &[1, 2, 3]);
        // sort uses scores so 1(0.9), 2(0.7), 3(0.5)
        let _ = arr.sort(&mut DummyScorer).unwrap();
        assert_eq!(arr.nodes(), &[1, 2, 3]);
    }
}
