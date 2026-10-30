//! Neighbor storage and priority queues for HNSW graphs.
//!
//! Equivalent to `org.apache.lucene.util.hnsw.NeighborArray` and
//! `org.apache.lucene.util.hnsw.NeighborQueue`.

#![deny(unsafe_code)]

use std::fmt::{self, Debug, Formatter};
use std::sync::Arc;

use crate::error::LuceneError;
use crate::internal::hppc::max_sized_float_array_list::MaxSizedFloatArrayList;
use crate::internal::hppc::max_sized_int_array_list::MaxSizedIntArrayList;
use crate::util::{Accountable, NumericUtils, TernaryLongHeap};

use super::scorer::{RandomVectorScorer, UpdateableRandomVectorScorer};
use super::Result;

/// Shallow size of a `NeighborArray` instance, Java's
/// `NeighborArray.BASE_RAM_BYTES_USED`.
///
/// `RamUsageEstimator.shallowSizeOfInstance(NeighborArray.class)` on a 64-bit
/// JVM with compressed oops (`RamUsageEstimator.java:535-597`): the 12-byte
/// object header, plus the `boolean scoresDescOrder` (1), the `int`s `size`,
/// `maxSize` and `sortedNodeSize` (3 × 4), the `long ramBytesUsed` (8), and the
/// references `scores`, `nodes` and `onHeapMemoryUsageListener` (3 × 4) — 45
/// bytes, aligned to 8. It is a literal, as the hppc lists' are, because the
/// figure is Java's object layout, not this struct's.
const BASE_RAM_BYTES_USED: i64 = 48;

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
    /// Equivalent to `NeighborArray.nodes`, a `MaxSizedIntArrayList` that
    /// starts at `maxSize / 8` slots and grows with `ArrayUtil.growInRange`.
    nodes: MaxSizedIntArrayList,
    /// Equivalent to `NeighborArray.scores`, sized as `nodes`.
    scores: MaxSizedFloatArrayList,
    sorted_node_size: usize,
    on_heap_memory_usage_listener: Option<OnHeapMemoryUsageListener>,
}

/// The callback a [`NeighborArray`] notifies whenever its buffers grow.
///
/// Equivalent to the `java.util.function.LongConsumer` that
/// `org.apache.lucene.util.hnsw.NeighborArray` keeps in
/// `onHeapMemoryUsageListener`. It is wrapped in a named type only so that
/// [`NeighborArray`] can keep deriving [`Debug`], which a bare closure does not
/// implement.
#[derive(Clone)]
pub struct OnHeapMemoryUsageListener(Arc<dyn Fn(i64) + Send + Sync>);

impl OnHeapMemoryUsageListener {
    /// Wraps `listener` so that a [`NeighborArray`] can hold it.
    pub fn new(listener: Arc<dyn Fn(i64) + Send + Sync>) -> Self {
        Self(listener)
    }

    /// Reports `bytes` of newly used on-heap memory.
    ///
    /// Equivalent to `LongConsumer.accept(long)`.
    pub fn accept(&self, bytes: i64) {
        (self.0)(bytes);
    }
}

impl Debug for OnHeapMemoryUsageListener {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str("OnHeapMemoryUsageListener")
    }
}

impl NeighborArray {
    /// Creates an empty array with the given maximum size and ordering.
    ///
    /// Equivalent to `NeighborArray(int, boolean)`, which delegates to the
    /// three-argument constructor with no listener.
    pub fn new(max_size: i32, desc_order: bool) -> Self {
        Self::with_listener(max_size, desc_order, None)
    }

    /// Creates an empty array that reports every on-heap growth to `listener`.
    ///
    /// Equivalent to `NeighborArray(int, boolean, LongConsumer)`.
    pub fn with_listener(
        max_size: i32,
        desc_order: bool,
        on_heap_memory_usage_listener: Option<OnHeapMemoryUsageListener>,
    ) -> Self {
        let nodes = MaxSizedIntArrayList::with_expected_elements(max_size, max_size / 8);
        let scores = MaxSizedFloatArrayList::with_expected_elements(max_size, max_size / 8);
        let max_size = max_size.max(0) as usize;
        // `BASE_RAM_BYTES_USED + nodes.ramBytesUsed() + scores.ramBytesUsed()`
        // (`NeighborArray.java:45,56`).
        let ram_bytes_used = BASE_RAM_BYTES_USED + nodes.ram_bytes_used() + scores.ram_bytes_used();
        if let Some(listener) = &on_heap_memory_usage_listener {
            listener.accept(ram_bytes_used);
        }
        Self {
            scores_desc_order: desc_order,
            size: 0,
            max_size,
            nodes,
            scores,
            sorted_node_size: 0,
            on_heap_memory_usage_listener,
        }
    }

    /// Reports the growth of the node and score buffers to the listener.
    ///
    /// `new_length` and `previous_length` are the node buffer's lengths after
    /// and before the insertion; the delta is charged once for the node `i32`
    /// and once for the score `f32`, exactly as Java charges `Integer.BYTES`
    /// and `Float.BYTES`. Like Java, this tells the listener only: Java's
    /// `ramBytesUsed` field is read only by the constructor, so this port keeps
    /// that value as a constructor local.
    ///
    /// Equivalent to the private
    /// `org.apache.lucene.util.hnsw.NeighborArray#alertOnHeapMemoryUsageChange`.
    fn alert_on_heap_memory_usage_change(&mut self, new_length: usize, previous_length: usize) {
        if new_length > previous_length {
            if let Some(listener) = &self.on_heap_memory_usage_listener {
                let length_delta = (new_length - previous_length) as i64;
                let delta = length_delta * i64::from(std::mem::size_of::<i32>() as i32)
                    + length_delta * i64::from(std::mem::size_of::<f32>() as i32);
                listener.accept(delta);
            }
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
        &self.nodes.buffer[..self.size]
    }

    /// Returns the score at index `i`.
    ///
    /// Equivalent to `org.apache.lucene.util.hnsw.NeighborArray#getScores`.
    pub fn score(&self, i: i32) -> f32 {
        self.scores.buffer[i as usize]
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
            return Err(LuceneError::illegal_state(
                "No growth is allowed".to_string(),
            ));
        }
        if self.size > 0 {
            let previous_score = self.scores.buffer[self.size - 1];
            debug_assert!(
                (self.scores_desc_order && previous_score >= new_score)
                    || (!self.scores_desc_order && previous_score <= new_score),
                "nodes are added in incorrect order"
            );
        }
        let previous_length = self.nodes.buffer.len();
        self.nodes.add(new_node);
        self.scores.add(new_score);
        self.alert_on_heap_memory_usage_change(self.nodes.buffer.len(), previous_length);
        self.size += 1;
        self.sorted_node_size += 1;
        Ok(())
    }

    /// Adds a node without preserving sorted order.
    pub fn add_out_of_order(&mut self, new_node: i32, new_score: f32) -> Result<()> {
        if self.size == self.max_size {
            return Err(LuceneError::illegal_state(
                "No growth is allowed".to_string(),
            ));
        }
        let previous_length = self.nodes.buffer.len();
        self.nodes.add(new_node);
        self.scores.add(new_score);
        self.alert_on_heap_memory_usage_change(self.nodes.buffer.len(), previous_length);
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
        let tmp_node = self.nodes.buffer[self.sorted_node_size];
        let mut tmp_score = self.scores.buffer[self.sorted_node_size];

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
                .buffer_mut()
                .copy_within(insertion_point..self.sorted_node_size, insertion_point + 1);
            self.scores
                .buffer_mut()
                .copy_within(insertion_point..self.sorted_node_size, insertion_point + 1);
        }
        self.nodes.buffer_mut()[insertion_point] = tmp_node;
        self.scores.buffer_mut()[insertion_point] = tmp_score;
        self.sorted_node_size += 1;
        Ok(insertion_point as i32)
    }

    fn asc_sort_find_rightmost_insertion_point(&self, new_score: f32, sorted_size: usize) -> usize {
        self.scores.buffer[..sorted_size].partition_point(|s| *s <= new_score)
    }

    fn desc_sort_find_rightmost_insertion_point(
        &self,
        new_score: f32,
        sorted_size: usize,
    ) -> usize {
        self.scores.buffer[..sorted_size].partition_point(|s| *s >= new_score)
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
            self.nodes.remove_last();
            self.scores.remove_last();
            self.sorted_node_size = self.sorted_node_size.min(self.size);
        }
    }

    /// Removes the entry at `idx`.
    pub fn remove_index(&mut self, idx: usize) {
        if idx == self.size - 1 {
            self.remove_last();
            return;
        }
        self.nodes.remove_at(idx as i32);
        self.scores.remove_at(idx as i32);
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
            scorer.set_scoring_ordinal(self.nodes.buffer[i])?;
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
        let min_accepted_similarity = self.scores.buffer[candidate_index];
        let candidate_is_unchecked =
            candidate_index == unchecked_indexes[unchecked_cursor] as usize;
        if candidate_is_unchecked {
            for i in (0..candidate_index).rev() {
                let neighbor_similarity = scorer.score(self.nodes.buffer[i])?;
                if neighbor_similarity >= min_accepted_similarity {
                    return Ok(true);
                }
            }
        } else {
            debug_assert!(candidate_index > unchecked_indexes[unchecked_cursor] as usize);
            for i in (0..=unchecked_cursor).rev() {
                let unchecked_idx = unchecked_indexes[i] as usize;
                let neighbor_similarity = scorer.score(self.nodes.buffer[unchecked_idx])?;
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
/// Equivalent to `org.apache.lucene.util.hnsw.NeighborQueue`: each arc is the
/// `long` `floatToSortableInt(score) << 32 | (0xFFFFFFFF & ~node)`, held in a
/// [`TernaryLongHeap`] — a min-heap of signed `i64`s — after the heap order's
/// transform (`v` for a min-heap, `-1 - v` for a max-heap). A min-heap keeps
/// the worst score on top; a max-heap keeps the best score on top; among equal
/// scores the lower node id ranks better.
#[derive(Clone, Debug)]
pub struct NeighborQueue {
    heap: TernaryLongHeap,
    /// `Order.MAX_HEAP` when set, `Order.MIN_HEAP` otherwise.
    max_heap: bool,
    visited_count: i32,
    incomplete: bool,
}

impl NeighborQueue {
    /// Creates a new queue. `max_heap` selects whether the top element is the
    /// best score (true) or the worst score (false).
    ///
    /// Equivalent to `NeighborQueue(int, boolean)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] with Java's message when
    /// `initial_size` is not in `1..ArrayUtil.MAX_ARRAY_LENGTH`, where the
    /// `TernaryLongHeap` constructor throws `IllegalArgumentException`
    /// (`TernaryLongHeap.java:55-63`).
    pub fn new(initial_size: i32, max_heap: bool) -> Result<Self> {
        // `TernaryLongHeap::new` takes a `usize`; a negative size is rejected
        // here with the message Java's heap gives for it.
        if initial_size < 1 {
            return Err(LuceneError::IllegalArgument(format!(
                "initialCapacity must be > 0 and < {}; got: {initial_size}",
                crate::util::ArrayUtil::MAX_ARRAY_LENGTH - 1
            )));
        }
        let heap = TernaryLongHeap::new(initial_size as usize)?;
        Ok(Self {
            heap,
            max_heap,
            visited_count: 0,
            incomplete: false,
        })
    }

    /// Returns the number of elements in the queue.
    pub fn size(&self) -> i32 {
        self.heap.size() as i32
    }

    /// Adds a new graph arc, extending storage as needed.
    pub fn add(&mut self, new_node: i32, new_score: f32) {
        let encoded = self.encode(new_node, new_score);
        self.heap.push(encoded);
    }

    /// Bounded insertion: once the queue holds its initial size, the new arc
    /// replaces the top only if it does not rank below it. Returns whether it
    /// was kept.
    ///
    /// Equivalent to `NeighborQueue.insertWithOverflow`, which is
    /// `TernaryLongHeap.insertWithOverflow` on the encoded arc.
    pub fn insert_with_overflow(&mut self, new_node: i32, new_score: f32) -> bool {
        let encoded = self.encode(new_node, new_score);
        self.heap.insert_with_overflow(encoded)
    }

    /// `Order.apply`: the identity for a min-heap, `-1 - v` for a max-heap,
    /// which maps `Long.MIN_VALUE` and `Long.MAX_VALUE` onto each other.
    fn apply_order(&self, v: i64) -> i64 {
        if self.max_heap {
            -1 - v
        } else {
            v
        }
    }

    /// Equivalent to the private `NeighborQueue.encode`.
    fn encode(&self, node: i32, score: f32) -> i64 {
        let sortable = i64::from(NumericUtils::float_to_sortable_int(score)) << 32;
        // `0xFFFFFFFFL & ~node`: the complemented id in the low 32 bits.
        let node_bits = i64::from((!node) as u32);
        self.apply_order(sortable | node_bits)
    }

    /// Equivalent to the private `NeighborQueue.decodeScore`.
    fn decode_score(&self, heap_value: i64) -> f32 {
        NumericUtils::sortable_int_to_float((self.apply_order(heap_value) >> 32) as i32)
    }

    /// Equivalent to the private `NeighborQueue.decodeNodeId`.
    fn decode_node(&self, heap_value: i64) -> i32 {
        (!self.apply_order(heap_value)) as i32
    }

    /// Removes and returns the top node id.
    ///
    /// Equivalent to `NeighborQueue.pop()`.
    ///
    /// # Divergence from Lucene 10.5.0
    ///
    /// On an empty queue Java's `TernaryLongHeap.pop()` throws the unchecked
    /// `IllegalStateException("The heap is empty")`. Rust has no unchecked
    /// exceptions and the signature returns a bare `i32`, as in Java, so this
    /// panics with the same message instead of returning a `Result`.
    ///
    /// # Panics
    ///
    /// Panics when the queue is empty.
    pub fn pop(&mut self) -> i32 {
        let top = self.heap.pop().unwrap_or_else(|e| panic!("{e}"));
        self.decode_node(top)
    }

    /// Returns all node ids currently stored, in heap-array order.
    ///
    /// Equivalent to `NeighborQueue.nodes()`, which reads `heap.get(i + 1)`.
    pub fn nodes(&self) -> Vec<i32> {
        (1..=self.heap.size())
            .map(|i| self.decode_node(self.heap.get(i)))
            .collect()
    }

    /// Returns the top node id.
    ///
    /// Equivalent to `NeighborQueue.topNode()`; like Java, an empty queue
    /// decodes whatever the heap's first slot holds.
    pub fn top_node(&self) -> i32 {
        self.decode_node(self.heap.top())
    }

    /// Returns the top score.
    ///
    /// Equivalent to `NeighborQueue.topScore()`; like Java, an empty queue
    /// decodes whatever the heap's first slot holds.
    pub fn top_score(&self) -> f32 {
        self.decode_score(self.heap.top())
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
        let mut q = NeighborQueue::new(3, false).unwrap();
        q.add(1, 0.5);
        q.add(2, 0.9);
        q.add(3, 0.1);
        assert_eq!(q.top_node(), 3); // worst score on top
        assert_eq!(q.pop(), 3);
        assert_eq!(q.top_node(), 1);
    }

    #[test]
    fn neighbor_queue_max_heap_ordering() {
        let mut q = NeighborQueue::new(3, true).unwrap();
        q.add(1, 0.5);
        q.add(2, 0.9);
        q.add(3, 0.1);
        assert_eq!(q.top_node(), 2); // best score on top
        assert_eq!(q.pop(), 2);
        assert_eq!(q.top_node(), 1);
    }

    #[test]
    fn neighbor_queue_insert_with_overflow() {
        let mut q = NeighborQueue::new(2, false).unwrap();
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

    /// Java orders the encoded arcs as signed `long`s, so a negative score
    /// ranks below every non-negative one (`NeighborQueue.java` `encode`).
    #[test]
    fn neighbor_queue_orders_negative_scores_as_java() {
        let scores = [(1, -0.5f32), (2, -0.1), (3, 0.3), (4, -2.0), (5, 0.0)];
        let mut min = NeighborQueue::new(2, false).unwrap();
        let mut max = NeighborQueue::new(2, true).unwrap();
        for (node, score) in scores {
            min.add(node, score);
            max.add(node, score);
        }
        assert_eq!(min.top_score(), -2.0);
        assert_eq!(max.top_score(), 0.3);
        let min_order: Vec<i32> = (0..5).map(|_| min.pop()).collect();
        let max_order: Vec<i32> = (0..5).map(|_| max.pop()).collect();
        assert_eq!(min_order, vec![4, 1, 2, 5, 3]);
        assert_eq!(max_order, vec![3, 5, 2, 1, 4]);
    }

    /// Among equal scores the lower node id ranks better, because the low
    /// word holds `~node`.
    #[test]
    fn neighbor_queue_breaks_score_ties_by_node_id() {
        let mut min = NeighborQueue::new(3, false).unwrap();
        let mut max = NeighborQueue::new(3, true).unwrap();
        for node in [7, 3, 5] {
            min.add(node, -1.0);
            max.add(node, -1.0);
        }
        assert_eq!((0..3).map(|_| min.pop()).collect::<Vec<_>>(), vec![7, 5, 3]);
        assert_eq!((0..3).map(|_| max.pop()).collect::<Vec<_>>(), vec![3, 5, 7]);
    }

    /// A bounded min-heap keeps the best arcs when the scores are negative.
    #[test]
    fn neighbor_queue_insert_with_overflow_keeps_the_best_negative_scores() {
        let mut q = NeighborQueue::new(2, false).unwrap();
        assert!(q.insert_with_overflow(1, -0.9));
        assert!(q.insert_with_overflow(2, -0.2));
        assert!(!q.insert_with_overflow(3, -1.5));
        assert!(q.insert_with_overflow(4, -0.1));
        let mut kept = q.nodes();
        kept.sort_unstable();
        assert_eq!(kept, vec![2, 4]);
        assert_eq!(q.top_score(), -0.2);
    }

    /// Java's queue throws `IllegalArgumentException` for a non-positive
    /// initial size (`TernaryLongHeap(int)`).
    #[test]
    fn neighbor_queue_rejects_a_zero_initial_size() {
        match NeighborQueue::new(0, false) {
            Err(LuceneError::IllegalArgument(message)) => assert_eq!(
                message,
                format!(
                    "initialCapacity must be > 0 and < {}; got: 0",
                    crate::util::ArrayUtil::MAX_ARRAY_LENGTH - 1
                )
            ),
            other => panic!("expected IllegalArgumentException, got {other:?}"),
        }
    }

    /// The buffers start at `maxSize / 8` slots and grow through
    /// `ArrayUtil.growInRange` (`NeighborArray.java:52-61`,
    /// `MaxSizedIntArrayList.ensureBufferSpace`); the listener is told the
    /// construction size, then `8` bytes per added slot. For `maxSize = 33`
    /// the node buffer goes 4, 8, 12, 16, 20, 24, 28, 32, 33 slots: Java's
    /// `oversize(n, 4)` is `(n + max(n >> 3, 3) + 1) & ~1`, capped at 33.
    #[test]
    fn neighbor_array_grows_and_reports_memory_as_java() {
        let reported = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = Arc::clone(&reported);
        let mut arr = NeighborArray::with_listener(
            33,
            true,
            Some(OnHeapMemoryUsageListener::new(Arc::new(move |bytes| {
                sink.lock().expect("log").push(bytes);
            }))),
        );
        // `NeighborArray` is 48 bytes, `MaxSizedIntArrayList` and
        // `MaxSizedFloatArrayList` 24 each, and a 4-slot int or float array
        // `align(16 + 16)` = 32.
        let initial = 48 + 2 * (24 + 32);
        assert_eq!(*reported.lock().expect("log"), vec![initial]);
        let mut growth_at = Vec::new();
        for node in 0..33 {
            let before = reported.lock().expect("log").len();
            arr.add_out_of_order(node, 1.0).expect("add");
            if reported.lock().expect("log").len() > before {
                growth_at.push(node + 1);
            }
        }
        assert_eq!(growth_at, vec![5, 9, 13, 17, 21, 25, 29, 33]);
        let log = reported.lock().expect("log").clone();
        assert_eq!(&log[1..], &[32, 32, 32, 32, 32, 32, 32, 8]);
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

// -----------------------------------------------------------------------------
// Object contracts: the `toString()` bodies of the two neighbour containers
// -----------------------------------------------------------------------------

impl std::fmt::Display for NeighborArray {
    /// Port of `NeighborArray.toString()`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NeighborArray[{}]", self.size)
    }
}

impl std::fmt::Display for NeighborQueue {
    /// Port of `NeighborQueue.toString()`, which names itself `Neighbors`
    /// rather than after its own class and reports the size of the backing
    /// heap.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Neighbors[{}]", self.heap.size())
    }
}
