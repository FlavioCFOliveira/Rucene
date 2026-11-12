//! Sorted hit queue, ported from
//! `org.apache.lucene.search.FieldValueHitQueue`.

#![deny(unsafe_code)]

use std::fmt;

use crate::error::{LuceneError, Result};
use crate::index::LeafReaderContext;
use crate::search::field_comparator::FieldComparator;
use crate::search::field_doc::FieldDoc;
use crate::search::leaf_field_comparator::LeafFieldComparator;
use crate::search::pruning::Pruning;
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
/// `MultiComparatorsFieldValueHitQueue`. The two differ only in `lessThan`,
/// and the one-comparator form is the multi-comparator loop over one
/// comparator, so both are the one `less_than` here; Java keeps them apart to
/// avoid an array access in the hot path, not to change behaviour.
///
/// # Adaptation: the heap is implemented here
///
/// **Divergence from Lucene 10.5.0.** The heap is implemented here rather than
/// by holding a [`PriorityQueue`](crate::util::PriorityQueue): that type takes
/// its ordering from a separate comparator object, which cannot reach the
/// comparators stored here, where Java's `lessThan` is a method of the queue
/// subclass itself. The sift-up and sift-down are the same as
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
    /// Returns [`LuceneErrorKind::IllegalArgument`](crate::error::LuceneErrorKind::IllegalArgument) — with the message text Java
    /// produces — when `fields` is empty, and propagates any error a
    /// [`SortField`] raises while building its comparator.
    pub fn create(fields: Vec<SortField>, size: usize) -> Result<Self> {
        if fields.is_empty() {
            return Err(LuceneError::illegal_argument(
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

    /// Returns the leaf comparator of every comparator for `context`.
    ///
    /// Equivalent to `FieldValueHitQueue.getComparators(LeafReaderContext)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while opening the segment's values.
    pub fn get_leaf_comparators(
        &mut self,
        context: &LeafReaderContext,
    ) -> Result<Vec<Box<dyn LeafFieldComparator>>> {
        self.comparators
            .iter_mut()
            .map(|comparator| comparator.get_leaf_comparator(context))
            .collect()
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
        let heap = &mut self.heap;
        let mut moved = false;
        with_slot_compares(&self.comparators, &self.reverse_mul, None, &mut |order| {
            moved = up_heap(heap, orig_pos, |a, b| less_than(order, a, b));
        });
        moved
    }

    /// Equivalent to the private `PriorityQueue.downHeap(int)`.
    fn down_heap(&mut self, i: usize) {
        let heap = &mut self.heap;
        let size = self.size;
        with_slot_compares(&self.comparators, &self.reverse_mul, None, &mut |order| {
            down_heap(heap, size, i, |a, b| less_than(order, a, b));
        });
    }
}

/// One link of the sort order a heap operation compares with: a comparator's
/// `compare` and its reverse multiplier, then the next sort field's.
///
/// The [`FieldComparator::with_slot_compare`] scopes of every comparator are
/// nested so that each `compare` runs without re-acquiring its comparator's
/// state; the links live on the stack of those nested calls.
struct SortLink<'a> {
    compare: &'a dyn Fn(i32, i32) -> i32,
    reverse_mul: i32,
    next: Option<&'a SortLink<'a>>,
}

/// Runs `body` with the sort order of `comparators`, `comparators[0]` first,
/// prepended to `next`.
///
/// The comparators are entered from the last to the first, so that the link
/// of `comparators[0]` is the head of the chain `body` receives.
fn with_slot_compares(
    comparators: &[Box<dyn FieldComparator>],
    reverse_mul: &[i32],
    next: Option<&SortLink<'_>>,
    body: &mut dyn FnMut(Option<&SortLink<'_>>),
) {
    match comparators.split_last() {
        None => body(next),
        Some((last, rest)) => {
            let reverse_mul_last = reverse_mul[rest.len()];
            last.with_slot_compare(&mut |compare| {
                let link = SortLink {
                    compare,
                    reverse_mul: reverse_mul_last,
                    next,
                };
                with_slot_compares(rest, reverse_mul, Some(&link), body);
            });
        }
    }
}

/// Returns whether `hit_a` is less relevant than `hit_b`.
///
/// Equivalent to the `lessThan(Entry, Entry)` of
/// `OneComparatorFieldValueHitQueue` and `MultiComparatorsFieldValueHitQueue`
/// (`FieldValueHitQueue.java:75-87,102-118`): the comparators in sort order,
/// short-circuiting on the first that tells the hits apart, then the doc ID.
fn less_than(order: Option<&SortLink<'_>>, hit_a: &Entry, hit_b: &Entry) -> bool {
    debug_assert!(!std::ptr::eq(hit_a, hit_b));
    debug_assert!(hit_a.slot != hit_b.slot);
    let mut link = order;
    while let Some(current) = link {
        let c = current.reverse_mul * (current.compare)(hit_a.slot, hit_b.slot);
        if c != 0 {
            // Short circuit.
            return c > 0;
        }
        link = current.next;
    }
    // Avoid a random sort order that could lead to duplicates (bug #31241).
    hit_a.score_doc.doc > hit_b.score_doc.doc
}

/// `PriorityQueue.upHeap(int)` over `heap`, ordered by `less_than`.
fn up_heap(
    heap: &mut [Option<Entry>],
    orig_pos: usize,
    less_than: impl Fn(&Entry, &Entry) -> bool,
) -> bool {
    let mut i = orig_pos;
    let node = heap[i]
        .take()
        .expect("INVARIANT: the caller just wrote an entry at this index");
    let mut j = i >> 1;
    while j > 0
        && less_than(
            &node,
            heap[j]
                .as_ref()
                .expect("INVARIANT: every index below size holds an entry"),
        )
    {
        heap[i] = heap[j].take();
        i = j;
        j >>= 1;
    }
    heap[i] = Some(node);
    i != orig_pos
}

/// `PriorityQueue.downHeap(int)` over the first `size` entries of `heap`,
/// ordered by `less_than`.
fn down_heap(
    heap: &mut [Option<Entry>],
    size: usize,
    mut i: usize,
    less_than: impl Fn(&Entry, &Entry) -> bool,
) {
    let Some(node) = heap[i].take() else {
        return;
    };
    let entry = |heap: &[Option<Entry>], index: usize| -> Entry {
        heap[index].expect("INVARIANT: every index below size holds an entry")
    };
    let mut j = i << 1;
    let mut k = j + 1;
    if k <= size && less_than(&entry(heap, k), &entry(heap, j)) {
        j = k;
    }
    while j <= size && less_than(&entry(heap, j), &node) {
        heap[i] = heap[j].take();
        i = j;
        j = i << 1;
        k = j + 1;
        if k <= size && less_than(&entry(heap, k), &entry(heap, j)) {
            j = k;
        }
    }
    heap[i] = Some(node);
}
