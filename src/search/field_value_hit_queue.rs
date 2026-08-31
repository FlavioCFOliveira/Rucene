//! Sorted hit queue, ported from
//! `org.apache.lucene.search.FieldValueHitQueue`.

#![deny(unsafe_code)]

use std::fmt;

use crate::error::{LuceneError, Result};
use crate::index::LeafReaderContext;
use crate::search::doc_id_set_iterator::DocIdSetIterator;
use crate::search::field_comparator::FieldComparator;
use crate::search::field_doc::FieldDoc;
use crate::search::multi_leaf_field_comparator::{compare_bottom_weighted, compare_top_weighted};
use crate::search::pruning::Pruning;
use crate::search::scorable::Scorable;
use crate::search::score_doc::ScoreDoc;
use crate::search::sort::SortField;

/// Extension of [`ScoreDoc`] that also stores the
/// [`FieldComparator`](crate::search::FieldComparator) slot.
///
/// Equivalent to `org.apache.lucene.search.FieldValueHitQueue.Entry`, which
/// extends `ScoreDoc` with a score of `NaN`. Rust has no implementation
/// inheritance, so the base hit is a field.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Entry {
    /// The doc ID, the score and the shard index of this entry.
    ///
    /// Equivalent to the state `Entry` inherits from `ScoreDoc`; the score is
    /// `NaN` until the collector fills it.
    pub score_doc: ScoreDoc,

    /// The comparator slot this entry occupies.
    ///
    /// Equivalent to the public `int slot`.
    pub slot: i32,
}

impl Entry {
    /// Creates an entry for `slot`, holding the top-level `doc` and a score of
    /// `NaN`.
    ///
    /// Equivalent to `new FieldValueHitQueue.Entry(int, int)`.
    pub fn new(slot: i32, doc: i32) -> Self {
        Self {
            score_doc: ScoreDoc::new(doc, f32::NAN),
            slot,
        }
    }

    /// The doc ID of this entry.
    pub fn doc(&self) -> i32 {
        self.score_doc.doc
    }
}

impl fmt::Display for Entry {
    /// Renders the entry exactly as `FieldValueHitQueue.Entry.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "slot:{} {}", self.slot, self.score_doc)
    }
}

/// Expert: a hit queue for sorting hits by the terms of one or more fields.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.search.FieldValueHitQueue<T extends Entry>`, which
/// extends `PriorityQueue<T>`, together with its two `private static final`
/// specialisations `OneComparatorFieldValueHitQueue` and
/// `MultiComparatorsFieldValueHitQueue`. The two differ only in `lessThan`, so
/// they become the [`one_comparator`](Self::one_comparator) flag here; Java
/// keeps them apart to avoid an array access in the hot path, not to change
/// behaviour.
///
/// # Adaptation: the queue owns the comparators and the leaf operations
///
/// **Divergence from Lucene 10.5.0.** Java's `lessThan` reads the comparators
/// through `FieldComparator.compare(int, int)` while `TopFieldCollector` holds
/// the *leaf* comparators of the very same objects and keeps calling
/// `copy`, `setBottom` and `compareBottom` on them. Rust cannot hold those two
/// aliases at once, so this type owns the comparators and exposes both halves:
/// the heap operations, and the per-leaf operations that Java performs through
/// `MultiLeafFieldComparator`. The algorithms are unchanged — the weighted,
/// short-circuiting comparisons are
/// [`compare_bottom_weighted`](crate::search::compare_bottom_weighted) and
/// [`compare_top_weighted`](crate::search::compare_top_weighted), the very
/// functions [`MultiLeafFieldComparator`](crate::search::MultiLeafFieldComparator)
/// runs.
///
/// For the same reason the heap itself is implemented here rather than by
/// holding a [`PriorityQueue`](crate::util::PriorityQueue): that type takes its
/// ordering from a separate comparator object, which cannot reach the
/// comparators stored here. The sift-up and sift-down are the same as
/// `PriorityQueue`'s, so the heap behaves identically.
pub struct FieldValueHitQueue {
    /// The sort criteria being used.
    ///
    /// Equivalent to the `protected final SortField[] fields`.
    fields: Vec<SortField>,
    /// Equivalent to the `protected final FieldComparator<?>[] comparators`.
    comparators: Vec<Box<dyn FieldComparator>>,
    /// Equivalent to the `protected final int[] reverseMul`.
    reverse_mul: Vec<i32>,
    one_comparator: bool,
    /// The heap, one-based: index `0` is unused, as in
    /// [`PriorityQueue`](crate::util::PriorityQueue).
    heap: Vec<Option<Entry>>,
    size: usize,
    max_size: usize,
}

impl fmt::Debug for FieldValueHitQueue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FieldValueHitQueue")
            .field("fields", &self.fields)
            .field("size", &self.size)
            .field("max_size", &self.max_size)
            .finish_non_exhaustive()
    }
}

impl FieldValueHitQueue {
    /// Creates a hit queue sorted by the given list of fields.
    ///
    /// Equivalent to `FieldValueHitQueue.create(SortField[], int)`, together
    /// with the private constructor it delegates to.
    ///
    /// The returned queue pre-allocates a full array of length `size`.
    ///
    /// * `fields` — the sort fields, in priority order (highest priority
    ///   first); it cannot be empty;
    /// * `size` — the number of hits to retain.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] — with the message text Java
    /// produces — when `fields` is empty, and propagates any error a
    /// [`SortField`] raises while building its comparator.
    pub fn create(fields: Vec<SortField>, size: usize) -> Result<Self> {
        if fields.is_empty() {
            return Err(LuceneError::IllegalArgument(
                "Sort must contain at least one field".to_string(),
            ));
        }

        // All of these are required by this class's API — it needs to return
        // arrays — so even for a single comparator an array is created anyway.
        let num_comparators = fields.len();
        let mut comparators = Vec::with_capacity(num_comparators);
        let mut reverse_mul = Vec::with_capacity(num_comparators);
        for (i, field) in fields.iter().enumerate() {
            reverse_mul.push(if field.reverse() { -1 } else { 1 });
            let pruning = if i == 0 {
                if num_comparators > 1 {
                    Pruning::GREATER_THAN
                } else {
                    Pruning::GREATER_THAN_OR_EQUAL_TO
                }
            } else {
                Pruning::NONE
            };
            comparators.push(field.get_comparator(size, pruning)?);
        }

        let heap_size = if size == 0 { 2 } else { size + 1 };
        Ok(Self {
            one_comparator: num_comparators == 1,
            fields,
            comparators,
            reverse_mul,
            heap: (0..heap_size).map(|_| None).collect(),
            size: 0,
            max_size: size,
        })
    }

    /// Returns the sort fields being used by this hit queue.
    ///
    /// Equivalent to the package-private `FieldValueHitQueue.getFields()`.
    pub fn get_fields(&self) -> &[SortField] {
        &self.fields
    }

    /// Returns the reverse multipliers of the sort fields.
    ///
    /// Equivalent to `FieldValueHitQueue.getReverseMul()`.
    pub fn get_reverse_mul(&self) -> &[i32] {
        &self.reverse_mul
    }

    /// Returns the comparators, in sort priority order.
    ///
    /// Equivalent to `FieldValueHitQueue.getComparators()`.
    pub fn get_comparators(&self) -> &[Box<dyn FieldComparator>] {
        &self.comparators
    }

    /// Returns the comparators for mutation.
    ///
    /// Equivalent to writing through the `protected` `comparators` array, which
    /// `TopFieldCollectorManager` does when it calls `setSingleSort()`.
    pub fn get_comparators_mut(&mut self) -> &mut [Box<dyn FieldComparator>] {
        &mut self.comparators
    }

    /// Prepares every comparator to collect the given leaf.
    ///
    /// Equivalent to
    /// `FieldValueHitQueue.getComparators(LeafReaderContext)`, which builds one
    /// `LeafFieldComparator` per comparator; see the adaptation note on this
    /// type for why nothing is returned.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while opening the segment's values.
    pub fn set_next_leaf(&mut self, context: &LeafReaderContext) -> Result<()> {
        for comparator in self.comparators.iter_mut() {
            comparator.get_leaf_comparator(context)?;
        }
        Ok(())
    }

    /// Compares the bottom of the queue with `doc`, weighted by the reverse
    /// multipliers.
    ///
    /// Equivalent to `reverseMul * comparator.compareBottom(doc)` in
    /// `TopFieldCollector.TopFieldLeafCollector.thresholdCheck`, for both the
    /// single-comparator and the multi-comparator shapes.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error a comparator raises.
    pub fn compare_bottom(&mut self, doc: i32, scorer: &mut dyn Scorable) -> Result<i32> {
        compare_bottom_weighted(
            self.comparators
                .iter_mut()
                .map(|comparator| comparator.as_leaf_comparator()),
            &self.reverse_mul,
            doc,
            scorer,
        )
    }

    /// Compares the top value with `doc`, weighted by the reverse multipliers.
    ///
    /// Equivalent to `reverseMul * comparator.compareTop(doc)` in
    /// `TopFieldCollector.PagingFieldCollector`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error a comparator raises.
    pub fn compare_top(&mut self, doc: i32, scorer: &mut dyn Scorable) -> Result<i32> {
        compare_top_weighted(
            self.comparators
                .iter_mut()
                .map(|comparator| comparator.as_leaf_comparator()),
            &self.reverse_mul,
            doc,
            scorer,
        )
    }

    /// Copies the values of `doc` into `slot` on every comparator.
    ///
    /// Equivalent to `LeafFieldComparator.copy(int, int)` on the leaf
    /// comparator of every sort field.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error a comparator raises.
    pub fn copy(&mut self, slot: i32, doc: i32, scorer: &mut dyn Scorable) -> Result<()> {
        for comparator in self.comparators.iter_mut() {
            comparator.copy(slot, doc, scorer)?;
        }
        Ok(())
    }

    /// Sets the bottom slot on every comparator.
    ///
    /// Equivalent to `LeafFieldComparator.setBottom(int)` on the leaf
    /// comparator of every sort field.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error a comparator raises.
    pub fn set_bottom(&mut self, slot: i32) -> Result<()> {
        for comparator in self.comparators.iter_mut() {
            comparator.set_bottom(slot)?;
        }
        Ok(())
    }

    /// Installs the scorer on every comparator.
    ///
    /// Equivalent to `LeafFieldComparator.setScorer(Scorable)` on the leaf
    /// comparator of every sort field.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error a comparator raises.
    pub fn set_scorer(&mut self, scorer: &mut dyn Scorable) -> Result<()> {
        for comparator in self.comparators.iter_mut() {
            comparator.set_scorer(scorer)?;
        }
        Ok(())
    }

    /// Notifies the primary comparator that the hits threshold was reached.
    ///
    /// Equivalent to `MultiLeafFieldComparator.setHitsThresholdReached()`,
    /// which only notifies the first comparator because skipping is only
    /// relevant for the primary sort field.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error the comparator raises.
    pub fn set_hits_threshold_reached(&mut self) -> Result<()> {
        self.comparators[0].set_hits_threshold_reached()
    }

    /// Returns the competitive iterator of the primary comparator.
    ///
    /// Equivalent to `MultiLeafFieldComparator.competitiveIterator()`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error the comparator raises.
    pub fn competitive_iterator(&mut self) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        self.comparators[0].competitive_iterator()
    }

    /// Builds the [`FieldDoc`] that carries the values used to sort `entry`.
    ///
    /// Equivalent to the package-private
    /// `FieldValueHitQueue.fillFields(Entry)`. The values are not the raw ones
    /// out of the index but the internal representation of them, so that the
    /// hit can be collated with hits from other searchers.
    pub fn fill_fields(&self, entry: &Entry) -> FieldDoc {
        let fields = self
            .comparators
            .iter()
            .map(|comparator| comparator.value(entry.slot))
            .collect();
        FieldDoc::with_fields(entry.score_doc.doc, entry.score_doc.score, fields)
    }

    /// Returns whether `hit_a` is less relevant than `hit_b`.
    ///
    /// Equivalent to the `lessThan(Entry, Entry)` of
    /// `OneComparatorFieldValueHitQueue` and
    /// `MultiComparatorsFieldValueHitQueue`.
    fn less_than(&self, hit_a: &Entry, hit_b: &Entry) -> bool {
        debug_assert!(hit_a.slot != hit_b.slot);
        if self.one_comparator {
            let c = self.reverse_mul[0] * self.comparators[0].compare(hit_a.slot, hit_b.slot);
            if c != 0 {
                return c > 0;
            }
        } else {
            for (i, comparator) in self.comparators.iter().enumerate() {
                let c = self.reverse_mul[i] * comparator.compare(hit_a.slot, hit_b.slot);
                if c != 0 {
                    // Short circuit.
                    return c > 0;
                }
            }
        }
        // Avoid a random sort order that could lead to duplicates (bug #31241).
        hit_a.score_doc.doc > hit_b.score_doc.doc
    }

    /// Adds an entry and returns the new top of the queue.
    ///
    /// Equivalent to the inherited `PriorityQueue.add(T)`.
    ///
    /// # Panics
    ///
    /// Panics when the queue is already full, as Java's
    /// `ArrayIndexOutOfBoundsException` does. `TopFieldCollector` only adds
    /// while `queueFull` is false.
    pub fn add(&mut self, element: Entry) -> Option<Entry> {
        let index = self.size + 1;
        self.heap[index] = Some(element);
        self.size = index;
        self.up_heap(index);
        self.heap[1]
    }

    /// Returns the least competitive entry.
    ///
    /// Equivalent to the inherited `PriorityQueue.top()`.
    pub fn top(&self) -> Option<Entry> {
        self.heap[1]
    }

    /// Removes and returns the least competitive entry.
    ///
    /// Equivalent to the inherited `PriorityQueue.pop()`.
    pub fn pop(&mut self) -> Option<Entry> {
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

    /// Replaces the doc ID of the top entry, which the collector mutates in
    /// place before re-establishing the heap.
    ///
    /// Equivalent to `bottom.doc = docBase + doc` in
    /// `TopFieldCollector.updateBottom(int)`, where `bottom` is the queue's
    /// top entry.
    pub fn set_top_doc(&mut self, doc: i32) {
        if let Some(top) = self.heap[1].as_mut() {
            top.score_doc.doc = doc;
        }
    }

    /// Re-establishes the heap invariant after the top entry was mutated in
    /// place, returning the new top.
    ///
    /// Equivalent to the inherited `PriorityQueue.updateTop()`.
    pub fn update_top(&mut self) -> Option<Entry> {
        self.down_heap(1);
        self.heap[1]
    }

    /// Returns the number of entries currently stored.
    ///
    /// Equivalent to the inherited `PriorityQueue.size()`.
    pub fn size(&self) -> usize {
        self.size
    }

    /// The number of entries the queue can hold.
    ///
    /// Equivalent to the inherited `PriorityQueue.maxSize()`.
    pub fn max_size(&self) -> usize {
        self.max_size
    }

    /// Removes every entry.
    ///
    /// Equivalent to the inherited `PriorityQueue.clear()`.
    pub fn clear(&mut self) {
        for i in 0..=self.size {
            self.heap[i] = None;
        }
        self.size = 0;
    }

    /// Equivalent to the private `PriorityQueue.upHeap(int)`.
    fn up_heap(&mut self, orig_pos: usize) -> bool {
        let mut i = orig_pos;
        let node = self.heap[i]
            .take()
            .expect("INVARIANT: the caller just wrote an entry at this index");
        let mut j = i >> 1;
        while j > 0
            && self.less_than(
                &node,
                self.heap[j]
                    .as_ref()
                    .expect("INVARIANT: every index below size holds an entry"),
            )
        {
            self.heap[i] = self.heap[j].take();
            i = j;
            j >>= 1;
        }
        self.heap[i] = Some(node);
        i != orig_pos
    }

    /// Equivalent to the private `PriorityQueue.downHeap(int)`.
    fn down_heap(&mut self, mut i: usize) {
        let Some(node) = self.heap[i].take() else {
            return;
        };
        let mut j = i << 1;
        let mut k = j + 1;
        if k <= self.size
            && self.less_than(
                self.heap[k]
                    .as_ref()
                    .expect("INVARIANT: every index below size holds an entry"),
                self.heap[j]
                    .as_ref()
                    .expect("INVARIANT: every index below size holds an entry"),
            )
        {
            j = k;
        }
        while j <= self.size
            && self.less_than(
                self.heap[j]
                    .as_ref()
                    .expect("INVARIANT: every index below size holds an entry"),
                &node,
            )
        {
            self.heap[i] = self.heap[j].take();
            i = j;
            j = i << 1;
            k = j + 1;
            if k <= self.size
                && self.less_than(
                    self.heap[k]
                        .as_ref()
                        .expect("INVARIANT: every index below size holds an entry"),
                    self.heap[j]
                        .as_ref()
                        .expect("INVARIANT: every index below size holds an entry"),
                )
            {
                j = k;
            }
        }
        self.heap[i] = Some(node);
    }
}
