//! Hit priority queue, ported from `org.apache.lucene.search.HitQueue`.

#![deny(unsafe_code)]

use std::cmp::Ordering;

use crate::error::Result;
use crate::search::score_doc::ScoreDoc;
use crate::util::{PriorityQueue, PriorityQueueComparator};

/// Reproduces `java.lang.Float.compare(float, float)`.
///
/// It is a total order that differs from Rust's `PartialOrd`: every `NaN`
/// compares equal to every other `NaN` and greater than positive infinity, and
/// `-0.0` compares less than `0.0`.
fn java_float_compare(a: f32, b: f32) -> Ordering {
    if a < b {
        return Ordering::Less;
    }
    if a > b {
        return Ordering::Greater;
    }
    // Java compares the canonical bit patterns as signed 32-bit integers.
    let a_bits = if a.is_nan() {
        0x7fc0_0000u32 as i32
    } else {
        a.to_bits() as i32
    };
    let b_bits = if b.is_nan() {
        0x7fc0_0000u32 as i32
    } else {
        b.to_bits() as i32
    };
    a_bits.cmp(&b_bits)
}

/// Orders hits so that the least competitive one is the top of the queue.
///
/// Equivalent to `HitQueue.lessThan(ScoreDoc, ScoreDoc)`.
#[derive(Debug, Default, Clone, Copy)]
pub struct HitQueueComparator;

impl PriorityQueueComparator<ScoreDoc> for HitQueueComparator {
    fn less_than(&self, hit_a: &ScoreDoc, hit_b: &ScoreDoc) -> bool {
        match java_float_compare(hit_a.score, hit_b.score) {
            Ordering::Equal => hit_a.doc > hit_b.doc,
            cmp => cmp == Ordering::Less,
        }
    }
}

/// Priority queue containing hit documents.
///
/// Equivalent to `org.apache.lucene.search.HitQueue`, a `final` subclass of
/// `PriorityQueue<ScoreDoc>`. Rust has no implementation inheritance, so the
/// queue is held rather than extended and the delegating methods below are the
/// inherited ones.
///
/// When the queue is pre-populated with sentinels, [`size`](Self::size) is the
/// requested size rather than the number of hits actually added, so a caller
/// must keep track of that itself: first pop `size() - total_hits` sentinels,
/// then pop the truly added elements.
pub struct HitQueue {
    pq: PriorityQueue<ScoreDoc, HitQueueComparator>,
}

impl std::fmt::Debug for HitQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HitQueue")
            .field("size", &self.pq.size())
            .finish()
    }
}

impl HitQueue {
    /// Creates a queue holding up to `size` elements.
    ///
    /// Equivalent to `new HitQueue(int, boolean)`. When `pre_populate` is set,
    /// the queue is filled with sentinel hits whose doc ID is [`i32::MAX`] — so
    /// that they are never favoured by the ordering — and whose score is
    /// negative infinity, and its size is `size`.
    ///
    /// As in Java, a full array of length `size` is pre-allocated either way.
    ///
    /// # Errors
    ///
    /// Propagates the [`PriorityQueue`] construction error for a size the queue
    /// cannot hold.
    pub fn new(size: usize, pre_populate: bool) -> Result<Self> {
        let mut pq = PriorityQueue::new(size, HitQueueComparator)?;
        if pre_populate {
            // Always set the doc ID to i32::MAX so that it is not favoured by
            // the ordering. This generally should not happen, since if the
            // score is not negative infinity TopScoreDocCollector will always
            // add the object to the queue.
            pq.add_all(
                std::iter::repeat_with(|| ScoreDoc::new(i32::MAX, f32::NEG_INFINITY)).take(size),
            );
        }
        Ok(Self { pq })
    }

    /// Adds an element, returning the new top of the queue.
    ///
    /// Equivalent to the inherited `PriorityQueue.add(ScoreDoc)`.
    pub fn add(&mut self, element: ScoreDoc) -> Option<&ScoreDoc> {
        self.pq.add(element)
    }

    /// Adds an element, returning the element that was dropped when the queue
    /// is already full.
    ///
    /// Equivalent to the inherited `PriorityQueue.insertWithOverflow(ScoreDoc)`.
    pub fn insert_with_overflow(&mut self, element: ScoreDoc) -> Option<ScoreDoc> {
        self.pq.insert_with_overflow(element)
    }

    /// Returns the least competitive element.
    ///
    /// Equivalent to the inherited `PriorityQueue.top()`.
    pub fn top(&self) -> Option<&ScoreDoc> {
        self.pq.top()
    }

    /// Removes and returns the least competitive element.
    ///
    /// Equivalent to the inherited `PriorityQueue.pop()`.
    pub fn pop(&mut self) -> Option<ScoreDoc> {
        self.pq.pop()
    }

    /// Re-establishes the heap invariant after the top element was mutated in
    /// place, returning the new top.
    ///
    /// Equivalent to the inherited `PriorityQueue.updateTop()`.
    pub fn update_top(&mut self) -> Option<&ScoreDoc> {
        self.pq.update_top()
    }

    /// Replaces the top element and re-establishes the heap invariant,
    /// returning the new top.
    ///
    /// Equivalent to the inherited `PriorityQueue.updateTop(ScoreDoc)`.
    pub fn update_top_with(&mut self, new_top: ScoreDoc) -> Option<&ScoreDoc> {
        self.pq.update_top_with(new_top)
    }

    /// Returns the number of elements currently stored.
    ///
    /// Equivalent to the inherited `PriorityQueue.size()`.
    pub fn size(&self) -> usize {
        self.pq.size()
    }

    /// Removes every element.
    ///
    /// Equivalent to the inherited `PriorityQueue.clear()`.
    pub fn clear(&mut self) {
        self.pq.clear();
    }
}
