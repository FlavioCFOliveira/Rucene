//! Ordinal-based term sorting, ported from
//! `org.apache.lucene.search.comparators.TermOrdValComparator` and its
//! competitive-state hierarchy.

#![deny(unsafe_code)]

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::{
    DocValuesSkipIndexType, IndexOptions, LeafReader, LeafReaderContext, PostingsEnum,
    SortedDocValues, POSTINGS_ENUM_NONE,
};
use crate::search::abstract_doc_id_set_iterator::AbstractDocIdSetIterator;
use crate::search::comparators::updateable_doc_id_set_iterator::UpdateableDocIdSetIterator;
use crate::search::doc_id_set_iterator::{all, empty, DocIdSetIterator, NO_MORE_DOCS};
use crate::search::doc_values_access::get_sorted;
use crate::search::doc_values_iteration::sorted_as_iterator;
use crate::search::field_comparator::{FieldComparator, SortValue};
use crate::search::index_searcher::IndexSearcher;
use crate::search::leaf_field_comparator::LeafFieldComparator;
use crate::search::pruning::Pruning;
use crate::search::scorable::Scorable;
use crate::search::skip_block_range_iterator::SkipBlockRangeIterator;
use crate::util::{BytesRef, FixedBitSet, PriorityQueue, PriorityQueueComparator};

/// A postings enum together with the doc-values ordinal of its term.
///
/// Equivalent to the private record
/// `TermOrdValComparator.PostingsEnumAndOrd`.
struct PostingsEnumAndOrd {
    postings: Box<dyn PostingsEnum>,
    ord: i32,
}

/// Orders the disjunction so that the least doc ID is the top of the queue.
///
/// Equivalent to the anonymous `PriorityQueue.lessThan` of
/// `PostingsBasedCompetitiveState.init`.
struct PostingsDocIdComparator;

impl PriorityQueueComparator<Rc<RefCell<PostingsEnumAndOrd>>> for PostingsDocIdComparator {
    fn less_than(
        &self,
        a: &Rc<RefCell<PostingsEnumAndOrd>>,
        b: &Rc<RefCell<PostingsEnumAndOrd>>,
    ) -> bool {
        a.borrow().postings.doc_id() < b.borrow().postings.doc_id()
    }
}

/// The postings of the competitive ordinal range, in ordinal order and as a
/// priority queue ordered by doc ID.
///
/// **Divergence from Lucene 10.5.0.** Java keeps the same
/// `PostingsEnumAndOrd` objects in an `ArrayDeque` (ordered by ordinal, so that
/// ordinals falling out of the competitive range can be trimmed from either
/// end) and in a `PriorityQueue` (ordered by doc ID, which is what iteration
/// needs). Rust cannot put one owned value into two containers, so each entry
/// is an [`Rc`] shared by both — which is exactly the object sharing Java
/// expresses — and both live in this one structure so that the disjunction
/// iterator and the comparator can share it.
struct PostingsDisjunction {
    postings: VecDeque<Rc<RefCell<PostingsEnumAndOrd>>>,
    queue: PriorityQueue<Rc<RefCell<PostingsEnumAndOrd>>, PostingsDocIdComparator>,
}

/// Iterates the union of the postings of every competitive term.
///
/// Equivalent to the anonymous `AbstractDocIdSetIterator` that
/// `PostingsBasedCompetitiveState` builds as its `disjunctionIterator`.
struct DisjunctionIterator {
    base: AbstractDocIdSetIterator,
    disjunction: Rc<RefCell<PostingsDisjunction>>,
}

impl DocIdSetIterator for DisjunctionIterator {
    fn doc_id(&self) -> i32 {
        self.base.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.advance(self.base.doc + 1)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let mut disjunction = self.disjunction.borrow_mut();
        loop {
            let top_doc = match disjunction.queue.top() {
                // The priority queue is empty, so none of the remaining
                // documents are competitive.
                None => return Ok(self.base.set(NO_MORE_DOCS)),
                Some(top) => top.borrow().postings.doc_id(),
            };
            if top_doc >= target {
                return Ok(self.base.set(top_doc));
            }
            // Java mutates the top in place and calls `updateTop()`. This crate's
            // priority queue exposes no mutable view of its top, so the entry is
            // popped, advanced and re-added; the queue holds the same entries
            // afterwards, and a binary heap always reports the same minimum.
            let top = disjunction
                .queue
                .pop()
                .expect("INVARIANT: the top was just observed to be present");
            top.borrow_mut().postings.advance(target)?;
            disjunction.queue.add(top);
        }
    }

    fn cost(&self) -> i64 {
        let disjunction = self.disjunction.borrow();
        disjunction
            .postings
            .iter()
            .map(|entry| entry.borrow().postings.cost())
            .sum()
    }
}

/// Whether the adaptive skip iterator is still measuring, actively skipping, or
/// has given up.
///
/// Equivalent to the private enum
/// `SkipperBasedCompetitiveState.State`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SkipState {
    Warming,
    Active,
    Disabled,
}

/// The number of block-boundary crossings the adaptive skip iterator tolerates
/// without an effective skip before disabling itself.
///
/// Equivalent to `SkipperBasedCompetitiveState.WARMUP_BOUNDARY_CROSSINGS`.
const WARMUP_BOUNDARY_CROSSINGS: i32 = 16;

/// The maximum number of terms a postings-based competitive iterator will pull
/// from the terms dictionary.
///
/// Equivalent to `PostingsBasedCompetitiveState.MAX_TERMS`.
const MAX_TERMS: i32 = 1024;

/// Wraps a [`SkipBlockRangeIterator`] and monitors whether skipping is
/// effective.
///
/// Equivalent to the private inner class
/// `SkipperBasedCompetitiveState.AdaptiveSkipIterator`. Block-boundary
/// crossings are tracked through the skipper's level-0 block end so that
/// within-block advances — which always return `target` — do not dilute the
/// signal. After [`WARMUP_BOUNDARY_CROSSINGS`] crossings with no effective skip
/// observed, the wrapper disables itself and becomes a trivial pass-through
/// iterator. If any boundary crossing produces an effective skip, the wrapper
/// stops monitoring and uses the skip iterator permanently.
///
/// **Divergence from Lucene 10.5.0.** Java reads the current block boundary
/// back from the shared `DocValuesSkipper` with `skipper.maxDocID(0)`. This
/// port reads it from the wrapped iterator's own
/// [`SkipBlockRangeIterator::block_end`], which is `skipper.maxDocID(0) + 1` on
/// the very same skipper, so that the skipper does not have to be aliased.
struct AdaptiveSkipIterator {
    base: AbstractDocIdSetIterator,
    inner: SkipBlockRangeIterator,
    state: Rc<Cell<SkipState>>,
    /// The handle this iterator replaces itself in when it gives up.
    competitive_iterator: UpdateableDocIdSetIterator,
    max_doc: i32,
    block_end_doc: i32,
    boundary_crossings: i32,
}

impl DocIdSetIterator for AdaptiveSkipIterator {
    fn doc_id(&self) -> i32 {
        self.base.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.advance(self.base.doc + 1)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let doc = match self.state.get() {
            SkipState::Disabled => target,
            SkipState::Active => self.inner.advance(target)?,
            SkipState::Warming => {
                let result = self.inner.advance(target)?;
                if target > self.block_end_doc {
                    self.block_end_doc = self.inner.block_end() - 1;
                    self.boundary_crossings += 1;
                    if result > target {
                        // Skipping has happened, so switch to permanently
                        // active skipping.
                        self.state.set(SkipState::Active);
                    } else if self.boundary_crossings >= WARMUP_BOUNDARY_CROSSINGS {
                        // We have crossed a number of block boundaries without
                        // any skipping happening, so it is not helping. Switch
                        // to no skipping to avoid the overhead.
                        self.state.set(SkipState::Disabled);
                        self.competitive_iterator
                            .update(Box::new(all(self.max_doc)?));
                    }
                }
                result
            }
        };
        Ok(self.base.set(doc))
    }

    fn cost(&self) -> i64 {
        self.inner.cost()
    }

    fn doc_id_run_end(&self) -> Result<i32> {
        if self.state.get() == SkipState::Disabled {
            return Ok(NO_MORE_DOCS);
        }
        self.inner.doc_id_run_end()
    }

    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        if self.state.get() == SkipState::Disabled {
            bit_set.set_range((self.base.doc - offset) as usize, (up_to - offset) as usize);
            self.base.doc = up_to;
            return Ok(());
        }
        self.inner.into_bit_set(up_to, bit_set, offset)?;
        self.base.doc = self.inner.doc_id();
        Ok(())
    }
}

/// The competitive-iterator strategies a [`TermOrdValComparator`] can use.
///
/// Equivalent to the private abstract class
/// `TermOrdValComparator.CompetitiveState` and its three subclasses. The
/// updateable iterator every subclass shares is held beside the strategy.
enum CompetitiveStateKind {
    /// Equivalent to `EmptyCompetitiveState`, used when the field has no terms
    /// and no skip index: every value is missing, so nothing is competitive.
    Empty,
    /// Equivalent to `PostingsBasedCompetitiveState`.
    Postings {
        reader: Arc<dyn LeafReader>,
        field: String,
        dense: bool,
        disjunction: Option<Rc<RefCell<PostingsDisjunction>>>,
        docs_with_field_installed: bool,
    },
    /// Equivalent to `SkipperBasedCompetitiveState`.
    Skipper {
        reader: Arc<dyn LeafReader>,
        field: String,
        max_doc: i32,
        prev_min_ord: i32,
        prev_max_ord: i32,
        state: Rc<Cell<SkipState>>,
    },
}

/// The competitive state of the segment being collected.
///
/// Equivalent to `TermOrdValComparator.CompetitiveState`, whose constructor
/// creates the updateable iterator and seeds it with every document.
struct CompetitiveState {
    iterator: UpdateableDocIdSetIterator,
    kind: CompetitiveStateKind,
}

impl CompetitiveState {
    /// Equivalent to `CompetitiveState(LeafReaderContext)`.
    fn new(max_doc: i32, kind: CompetitiveStateKind) -> Result<Self> {
        let iterator = UpdateableDocIdSetIterator::new();
        iterator.update(Box::new(all(max_doc)?));
        if let CompetitiveStateKind::Skipper { max_doc, .. } = &kind {
            // `SkipperBasedCompetitiveState` repeats the seeding in its own
            // constructor; the value is the same.
            iterator.update(Box::new(all(*max_doc)?));
        }
        Ok(Self { iterator, kind })
    }
}

/// How a term comparator obtains the sorted doc values of a segment.
///
/// Equivalent to the `protected SortedDocValues
/// TermOrdValComparator.getSortedDocValues(LeafReaderContext, String)` hook,
/// which defaults to `DocValues.getSorted(context.reader(), field)` and which
/// `SortedSetSortField` overrides to install a
/// [`SortedSetSelector`](crate::search::SortedSetSelector) view.
///
/// **Divergence from Lucene 10.5.0.** Java's hook receives the whole
/// [`LeafReaderContext`]; this port passes the leaf reader it would have read
/// from it, because a context cannot be stored beyond the call that produced
/// it, and the postings-based competitive iterator re-derives the doc values
/// after the call has returned.
pub type SortedDocValuesSource =
    Rc<dyn Fn(&dyn LeafReader, &str) -> Result<Box<dyn SortedDocValues>>>;

/// The per-segment state of a [`TermOrdValComparator`].
///
/// Equivalent to the fields of the inner class
/// `TermOrdValComparator.TermOrdValLeafComparator`.
struct TermOrdValLeafState {
    /// The current reader's doc ordinals and values.
    terms_index: Box<dyn SortedDocValues>,
    /// Whether the current bottom slot matches the current reader.
    bottom_same_reader: bool,
    /// The bottom ordinal, cached for faster comparisons.
    bottom_ord: i32,
    top_same_reader: bool,
    top_ord: i32,
    /// The ordinal to use for a missing value.
    missing_ord: i32,
    competitive_state: Option<CompetitiveState>,
    dense: bool,
}

/// Sorts by the field's natural term sort order, using ordinals.
///
/// Equivalent to `org.apache.lucene.search.comparators.TermOrdValComparator`.
/// This is functionally equivalent to
/// [`TermValComparator`](crate::search::TermValComparator), but it first
/// resolves the strings to their relative ordinal positions — using the index
/// returned by
/// [`LeafReader::get_sorted_doc_values`](crate::index::LeafReader::get_sorted_doc_values) —
/// and does most comparisons using the ordinals. For medium to large results
/// this comparator is much faster; for very small result sets it may be slower.
///
/// **Divergence from Lucene 10.5.0.** Java passes `values.termsEnum()` into the
/// postings-based competitive state and uses it only to map the smallest
/// competitive ordinal to its term. `SortedDocValuesTermsEnum.seekExact(long)`
/// followed by `term()` is `lookupOrd(ord)` on the very same doc values, so
/// this port calls that directly rather than aliasing the doc values into a
/// second object.
pub struct TermOrdValComparator {
    /// The ordinal of each slot.
    ords: Vec<i32>,
    /// The value of each slot.
    values: Vec<Option<BytesRef>>,
    /// Which reader last copied a value into each slot. When two slots are
    /// compared, comparing by ordinal is enough if the reader generation is the
    /// same; otherwise the values must be compared, which is slower.
    reader_gen: Vec<i32>,
    /// The generation of the reader currently being collected.
    current_reader_gen: i32,
    field: String,
    reverse: bool,
    sort_missing_last: bool,
    /// The bottom value, the same as `values[bottom_slot]` once `bottom_slot`
    /// is set. Cached for faster comparisons.
    bottom_value: Option<BytesRef>,
    /// The bottom slot, or `-1` while the queue is not full.
    bottom_slot: i32,
    /// The value set by
    /// [`FieldComparator::set_top_value`](crate::search::FieldComparator::set_top_value).
    top_value: Option<BytesRef>,
    /// `-1` if missing values sort first, `1` if they sort last.
    missing_sort_cmp: i32,
    /// Whether this is the only comparator.
    single_sort: bool,
    /// Whether this comparator is allowed to skip documents.
    can_skip_documents: bool,
    /// Whether the collector is done counting hits, so that documents may start
    /// being skipped.
    hits_threshold_reached: bool,
    doc_values_source: Option<SortedDocValuesSource>,
    leaf: Option<TermOrdValLeafState>,
}

impl std::fmt::Debug for TermOrdValComparator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TermOrdValComparator")
            .field("field", &self.field)
            .field("reverse", &self.reverse)
            .field("sort_missing_last", &self.sort_missing_last)
            .finish_non_exhaustive()
    }
}

impl TermOrdValComparator {
    /// Creates a comparator over `num_hits` slots, with control over how
    /// missing values are sorted.
    ///
    /// Equivalent to
    /// `new TermOrdValComparator(int, String, boolean, boolean, Pruning)`. Pass
    /// `sort_missing_last = true` to put missing values at the end.
    pub fn new(
        num_hits: usize,
        field: impl Into<String>,
        sort_missing_last: bool,
        reverse: bool,
        pruning: Pruning,
    ) -> Self {
        Self {
            ords: vec![0; num_hits],
            values: vec![None; num_hits],
            reader_gen: vec![0; num_hits],
            current_reader_gen: -1,
            field: field.into(),
            reverse,
            sort_missing_last,
            bottom_value: None,
            bottom_slot: -1,
            top_value: None,
            missing_sort_cmp: if sort_missing_last { 1 } else { -1 },
            single_sort: false,
            can_skip_documents: pruning != Pruning::NONE,
            hits_threshold_reached: false,
            doc_values_source: None,
            leaf: None,
        }
    }

    /// Installs a replacement for the default sorted-doc-values lookup.
    ///
    /// Equivalent to overriding
    /// `TermOrdValComparator.getSortedDocValues(LeafReaderContext, String)`,
    /// which is what `SortedSetSortField.getComparator` does; see
    /// [`SortedDocValuesSource`].
    pub fn set_sorted_doc_values_source(&mut self, source: SortedDocValuesSource) {
        self.doc_values_source = Some(source);
    }

    /// Opens the sorted doc values of `reader` through the installed source, or
    /// through `DocValues.getSorted` when none is installed.
    fn open_sorted_doc_values(&self, reader: &dyn LeafReader) -> Result<Box<dyn SortedDocValues>> {
        match self.doc_values_source.as_ref() {
            Some(source) => source(reader, &self.field),
            None => get_sorted(reader, &self.field),
        }
    }

    /// The body of the overridden `compareValues(BytesRef, BytesRef)`.
    fn compare_bytes(&self, val1: Option<&BytesRef>, val2: Option<&BytesRef>) -> i32 {
        match (val1, val2) {
            (None, None) => 0,
            (None, Some(_)) => self.missing_sort_cmp,
            (Some(_), None) => -self.missing_sort_cmp,
            (Some(a), Some(b)) => match a.cmp(b) {
                Ordering::Less => -1,
                Ordering::Equal => 0,
                Ordering::Greater => 1,
            },
        }
    }

    /// Equivalent to the private
    /// `TermOrdValLeafComparator.getOrdForDoc(int)`.
    fn get_ord_for_doc(&mut self, doc: i32) -> Result<i32> {
        let Some(leaf) = self.leaf.as_mut() else {
            return Ok(-1);
        };
        if leaf.terms_index.advance_exact(doc)? {
            leaf.terms_index.ord_value()
        } else {
            Ok(-1)
        }
    }

    /// Equivalent to the private
    /// `TermOrdValLeafComparator.shouldEnableSkipping(boolean)`.
    fn should_enable_skipping(&self, dense: bool) -> bool {
        if dense || self.top_value.is_some() {
            true
        } else if self.reverse == self.sort_missing_last {
            // Missing values are always competitive, so we can never skip.
            false
        } else {
            true
        }
    }

    /// Equivalent to the private
    /// `TermOrdValLeafComparator.updateCompetitiveIterator()`.
    fn update_competitive_iterator(&mut self) -> Result<()> {
        if self.leaf.as_ref().map_or(true, |leaf| {
            leaf.competitive_state.is_none() || !self.hits_threshold_reached
        }) || self.bottom_slot == -1
        {
            return Ok(());
        }

        let (min_ord, max_ord) = {
            let leaf = self
                .leaf
                .as_ref()
                .expect("INVARIANT: the leaf was just observed to be present");
            let value_count = leaf.terms_index.get_value_count()?;
            // This logic to figure out the minimum and maximum ordinals is
            // quite complex and verbose; it mirrors Lucene's exactly.
            let (min_ord, max_ord) = if !self.reverse {
                let min_ord = if self.top_value.is_some() {
                    if leaf.top_same_reader {
                        leaf.top_ord
                    } else {
                        // When the top value does not exist in the segment,
                        // topOrd is the previous ordinal, and we are only
                        // interested in values that compare strictly greater.
                        leaf.top_ord + 1
                    }
                } else if self.sort_missing_last || leaf.dense {
                    0
                } else {
                    // Missing values are still competitive.
                    -1
                };

                let max_ord = if leaf.bottom_ord == leaf.missing_ord {
                    // The queue still contains missing values.
                    if self.single_sort {
                        // Without a tie breaker we can start ignoring missing
                        // values from now on.
                        value_count - 1
                    } else {
                        i32::MAX
                    }
                } else if leaf.bottom_same_reader {
                    // Without a tie breaker we can start ignoring values that
                    // compare equal to the current top value too.
                    if self.single_sort {
                        leaf.bottom_ord - 1
                    } else {
                        leaf.bottom_ord
                    }
                } else {
                    leaf.bottom_ord
                };
                (min_ord, max_ord)
            } else {
                let min_ord = if leaf.bottom_ord == leaf.missing_ord {
                    // The queue still contains missing values.
                    if self.single_sort {
                        0
                    } else {
                        -1
                    }
                } else if leaf.bottom_same_reader {
                    if self.single_sort {
                        leaf.bottom_ord + 1
                    } else {
                        leaf.bottom_ord
                    }
                } else {
                    leaf.bottom_ord + 1
                };

                let max_ord = if self.top_value.is_some() {
                    leaf.top_ord
                } else if !self.sort_missing_last || leaf.dense {
                    value_count - 1
                } else {
                    i32::MAX
                };
                (min_ord, max_ord)
            };
            (min_ord, max_ord)
        };

        if min_ord == -1 || max_ord == i32::MAX {
            // Missing values are still competitive, so we cannot skip yet.
            return Ok(());
        }
        debug_assert!(min_ord >= 0);
        self.competitive_state_update(min_ord, max_ord)
    }

    /// Equivalent to `CompetitiveState.update(int, int)`, dispatched on the
    /// concrete strategy.
    fn competitive_state_update(&mut self, min_ord: i32, max_ord: i32) -> Result<()> {
        let doc_values_source = self.doc_values_source.clone();
        let Some(leaf) = self.leaf.as_mut() else {
            return Ok(());
        };
        let Some(competitive) = leaf.competitive_state.as_mut() else {
            return Ok(());
        };
        match &mut competitive.kind {
            CompetitiveStateKind::Empty => {
                competitive.iterator.update(Box::new(empty()));
                Ok(())
            }
            CompetitiveStateKind::Skipper {
                reader,
                field,
                max_doc,
                prev_min_ord,
                prev_max_ord,
                state,
            } => {
                if state.get() == SkipState::Disabled {
                    return Ok(());
                }
                if min_ord == *prev_min_ord && max_ord == *prev_max_ord {
                    return Ok(());
                }
                *prev_min_ord = min_ord;
                *prev_max_ord = max_ord;
                let Some(skipper) = reader.get_doc_values_skipper(field)? else {
                    return Ok(());
                };
                let iterator = AdaptiveSkipIterator {
                    base: AbstractDocIdSetIterator::new(),
                    inner: SkipBlockRangeIterator::new(
                        skipper,
                        i64::from(min_ord),
                        i64::from(max_ord),
                    ),
                    state: Rc::clone(state),
                    competitive_iterator: competitive.iterator.clone(),
                    max_doc: *max_doc,
                    block_end_doc: -1,
                    boundary_crossings: 0,
                };
                competitive.iterator.update(Box::new(iterator));
                Ok(())
            }
            CompetitiveStateKind::Postings {
                reader,
                field,
                dense,
                disjunction,
                docs_with_field_installed,
            } => {
                let max_terms = MAX_TERMS.min(IndexSearcher::get_max_clause_count());
                let size = (max_ord - min_ord + 1).max(0);
                if size > max_terms {
                    if !*dense && !*docs_with_field_installed {
                        let docs_with_field = match doc_values_source.as_ref() {
                            Some(source) => source(reader.as_ref(), field)?,
                            None => get_sorted(reader.as_ref(), field)?,
                        };
                        competitive
                            .iterator
                            .update(sorted_as_iterator(docs_with_field));
                        *docs_with_field_installed = true;
                    }
                } else if disjunction.is_none() {
                    let built = init_postings(
                        reader.as_ref(),
                        field,
                        leaf.terms_index.as_ref(),
                        min_ord,
                        max_ord,
                    )?;
                    let shared = Rc::new(RefCell::new(built));
                    *disjunction = Some(Rc::clone(&shared));
                    competitive.iterator.update(Box::new(DisjunctionIterator {
                        base: AbstractDocIdSetIterator::new(),
                        disjunction: shared,
                    }));
                } else {
                    let shared = disjunction
                        .as_ref()
                        .expect("INVARIANT: the disjunction was just observed to be present");
                    let mut shared = shared.borrow_mut();
                    if (size as usize) < shared.postings.len() {
                        // One or more ordinals were removed.
                        while shared
                            .postings
                            .front()
                            .is_some_and(|entry| entry.borrow().ord < min_ord)
                        {
                            shared.postings.pop_front();
                        }
                        while shared
                            .postings
                            .back()
                            .is_some_and(|entry| entry.borrow().ord > max_ord)
                        {
                            shared.postings.pop_back();
                        }
                        shared.queue.clear();
                        let entries: Vec<_> = shared.postings.iter().map(Rc::clone).collect();
                        shared.queue.add_all(entries);
                    }
                }
                Ok(())
            }
        }
    }
}

/// Pulls the postings of every term whose ordinal lies in
/// `[min_ord, max_ord]` and builds the priority queue over them.
///
/// Equivalent to the private `PostingsBasedCompetitiveState.init(int, int)`.
fn init_postings(
    reader: &dyn LeafReader,
    field: &str,
    terms_index: &dyn SortedDocValues,
    min_ord: i32,
    max_ord: i32,
) -> Result<PostingsDisjunction> {
    let size = (max_ord - min_ord + 1).max(0) as usize;
    let mut postings: VecDeque<Rc<RefCell<PostingsEnumAndOrd>>> = VecDeque::with_capacity(size);
    if size > 0 {
        let min_term = terms_index.lookup_ord(min_ord)?;
        let Some(terms) = reader.terms(field)? else {
            return Err(LuceneError::IllegalState(format!(
                "Term {min_term:?} exists in doc values but not in the terms index"
            )));
        };
        let mut terms_enum = terms.iterator()?;
        if !terms_enum.seek_exact(&min_term)? {
            return Err(LuceneError::IllegalState(format!(
                "Term {min_term:?} exists in doc values but not in the terms index"
            )));
        }
        postings.push_back(Rc::new(RefCell::new(PostingsEnumAndOrd {
            postings: terms_enum.postings(None, POSTINGS_ENUM_NONE)?,
            ord: min_ord,
        })));
        for ord in min_ord + 1..=max_ord {
            let next = terms_enum.next()?;
            if next.is_none() {
                return Err(LuceneError::IllegalState(format!(
                    "Terms have more than {ord} unique terms while doc values have exactly {ord} terms"
                )));
            }
            postings.push_back(Rc::new(RefCell::new(PostingsEnumAndOrd {
                postings: terms_enum.postings(None, POSTINGS_ENUM_NONE)?,
                ord,
            })));
        }
    }
    let mut queue = PriorityQueue::new(size, PostingsDocIdComparator)?;
    let entries: Vec<_> = postings.iter().map(Rc::clone).collect();
    queue.add_all(entries);
    Ok(PostingsDisjunction { postings, queue })
}

impl LeafFieldComparator for TermOrdValComparator {
    fn set_bottom(&mut self, bottom: i32) -> Result<()> {
        self.bottom_slot = bottom;
        self.bottom_value = self.values[bottom as usize].clone();

        let current_reader_gen = self.current_reader_gen;
        let bottom_value = self.bottom_value.clone();
        let ord_at_slot = self.ords[bottom as usize];
        let reader_gen_at_slot = self.reader_gen[bottom as usize];

        let mut ords_write: Option<i32> = None;
        let mut reader_gen_write: Option<i32> = None;
        if let Some(leaf) = self.leaf.as_mut() {
            if current_reader_gen == reader_gen_at_slot {
                leaf.bottom_ord = ord_at_slot;
                leaf.bottom_same_reader = true;
            } else if bottom_value.is_none() {
                // The missing ordinal is null for all segments.
                debug_assert_eq!(ord_at_slot, leaf.missing_ord);
                leaf.bottom_ord = leaf.missing_ord;
                leaf.bottom_same_reader = true;
                reader_gen_write = Some(current_reader_gen);
            } else {
                let value = bottom_value
                    .as_ref()
                    .expect("INVARIANT: the bottom value was just observed to be present");
                let ord = leaf.terms_index.lookup_term(value)?;
                if ord < 0 {
                    leaf.bottom_ord = -ord - 2;
                    leaf.bottom_same_reader = false;
                } else {
                    leaf.bottom_ord = ord;
                    // Exact value match.
                    leaf.bottom_same_reader = true;
                    reader_gen_write = Some(current_reader_gen);
                    ords_write = Some(ord);
                }
            }
        }
        if let Some(gen) = reader_gen_write {
            self.reader_gen[bottom as usize] = gen;
        }
        if let Some(ord) = ords_write {
            self.ords[bottom as usize] = ord;
        }

        self.update_competitive_iterator()
    }

    fn compare_bottom(&mut self, doc: i32, _scorer: &mut dyn Scorable) -> Result<i32> {
        debug_assert!(self.bottom_slot != -1);
        let mut doc_ord = self.get_ord_for_doc(doc)?;
        let Some(leaf) = self.leaf.as_ref() else {
            return Ok(0);
        };
        if doc_ord == -1 {
            doc_ord = leaf.missing_ord;
        }
        if leaf.bottom_same_reader {
            // The ordinal is precisely comparable, even in the equal case.
            Ok(leaf.bottom_ord - doc_ord)
        } else if leaf.bottom_ord >= doc_ord {
            // The equal case always means bottom is greater than doc, because
            // bottom_ord was set to the lower bound in set_bottom.
            Ok(1)
        } else {
            Ok(-1)
        }
    }

    fn compare_top(&mut self, doc: i32, _scorer: &mut dyn Scorable) -> Result<i32> {
        let mut ord = self.get_ord_for_doc(doc)?;
        let Some(leaf) = self.leaf.as_ref() else {
            return Ok(0);
        };
        if ord == -1 {
            ord = leaf.missing_ord;
        }
        if leaf.top_same_reader {
            // The ordinal is precisely comparable, even in the equal case.
            Ok(leaf.top_ord - ord)
        } else if ord <= leaf.top_ord {
            // The equal case always means doc is less than the value, because
            // top_ord was set to the lower bound.
            Ok(1)
        } else {
            Ok(-1)
        }
    }

    fn copy(&mut self, slot: i32, doc: i32, _scorer: &mut dyn Scorable) -> Result<()> {
        let mut ord = self.get_ord_for_doc(doc)?;
        let current_reader_gen = self.current_reader_gen;
        let value = if ord == -1 {
            let missing_ord = self.leaf.as_ref().map_or(-1, |leaf| leaf.missing_ord);
            ord = missing_ord;
            None
        } else {
            debug_assert!(ord >= 0);
            let leaf = self
                .leaf
                .as_ref()
                .expect("INVARIANT: a non-negative ordinal implies a positioned leaf");
            Some(BytesRef::deep_copy_of(&leaf.terms_index.lookup_ord(ord)?))
        };
        self.values[slot as usize] = value;
        self.ords[slot as usize] = ord;
        self.reader_gen[slot as usize] = current_reader_gen;
        Ok(())
    }

    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn competitive_iterator(&mut self) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        Ok(self.leaf.as_ref().and_then(|leaf| {
            leaf.competitive_state
                .as_ref()
                .map(|state| Box::new(state.iterator.clone()) as Box<dyn DocIdSetIterator>)
        }))
    }

    fn set_hits_threshold_reached(&mut self) -> Result<()> {
        self.hits_threshold_reached = true;
        self.update_competitive_iterator()
    }
}

impl FieldComparator for TermOrdValComparator {
    fn compare(&self, slot1: i32, slot2: i32) -> i32 {
        if self.reader_gen[slot1 as usize] == self.reader_gen[slot2 as usize] {
            return self.ords[slot1 as usize] - self.ords[slot2 as usize];
        }
        self.compare_bytes(
            self.values[slot1 as usize].as_ref(),
            self.values[slot2 as usize].as_ref(),
        )
    }

    fn set_top_value(&mut self, value: SortValue) {
        // Null is fine: it means the last doc of the prior search was missing
        // this value.
        self.top_value = value.as_bytes().cloned();
    }

    fn value(&self, slot: i32) -> SortValue {
        match &self.values[slot as usize] {
            None => SortValue::Null,
            Some(value) => SortValue::Bytes(value.clone()),
        }
    }

    fn get_leaf_comparator(&mut self, context: &LeafReaderContext) -> Result<()> {
        self.current_reader_gen += 1;
        let reader = context.leaf_reader();
        let max_doc = reader.max_doc();
        let terms_index = self.open_sorted_doc_values(reader.as_ref())?;

        let missing_ord = if self.sort_missing_last { i32::MAX } else { -1 };

        let (top_ord, top_same_reader) = match self.top_value.as_ref() {
            Some(top_value) => {
                // Recompute topOrd/topSameReader.
                let ord = terms_index.lookup_term(top_value)?;
                if ord >= 0 {
                    (ord, true)
                } else {
                    (-ord - 2, false)
                }
            }
            None => (missing_ord, true),
        };

        self.leaf = Some(TermOrdValLeafState {
            terms_index,
            bottom_same_reader: false,
            bottom_ord: 0,
            top_same_reader,
            top_ord,
            missing_ord,
            competitive_state: None,
            dense: false,
        });

        if self.bottom_slot != -1 {
            // Recompute bottomOrd/bottomSameReader.
            self.set_bottom(self.bottom_slot)?;
        }

        let mut enable_skipping = false;
        let mut has_terms = false;
        let mut has_skipper = false;
        let mut dense = false;
        if self.can_skip_documents {
            let field_infos = reader.get_field_infos();
            match field_infos.field_info(&self.field) {
                None => {
                    let value_count = self
                        .leaf
                        .as_ref()
                        .expect("INVARIANT: the leaf was just installed")
                        .terms_index
                        .get_value_count()?;
                    if value_count != 0 {
                        return Err(LuceneError::IllegalState(format!(
                            "Field [{}] cannot be found in field infos",
                            self.field
                        )));
                    }
                    enable_skipping = true;
                }
                Some(field_info) if field_info.get_index_options() != IndexOptions::NONE => {
                    let terms = reader.terms(&self.field)?;
                    has_terms = terms.is_some();
                    dense = terms.is_some_and(|terms| terms.doc_count() == max_doc);
                    enable_skipping = self.should_enable_skipping(dense);
                }
                Some(field_info)
                    if field_info.doc_values_skip_index_type() != DocValuesSkipIndexType::NONE =>
                {
                    let skipper = reader.get_doc_values_skipper(&self.field)?;
                    has_skipper = skipper.is_some();
                    dense = skipper.is_some_and(|skipper| skipper.global_doc_count() == max_doc);
                    enable_skipping = self.should_enable_skipping(dense);
                }
                Some(_) => {}
            }
        }

        if enable_skipping {
            let kind = if has_terms {
                CompetitiveStateKind::Postings {
                    reader: Arc::clone(&reader),
                    field: self.field.clone(),
                    dense,
                    disjunction: None,
                    docs_with_field_installed: false,
                }
            } else if has_skipper {
                CompetitiveStateKind::Skipper {
                    reader: Arc::clone(&reader),
                    field: self.field.clone(),
                    max_doc,
                    prev_min_ord: i32::MIN,
                    prev_max_ord: i32::MIN,
                    state: Rc::new(Cell::new(SkipState::Warming)),
                }
            } else {
                CompetitiveStateKind::Empty
            };
            let state = CompetitiveState::new(max_doc, kind)?;
            if let Some(leaf) = self.leaf.as_mut() {
                leaf.dense = dense;
                leaf.competitive_state = Some(state);
            }
        } else if let Some(leaf) = self.leaf.as_mut() {
            leaf.dense = dense;
        }

        self.update_competitive_iterator()
    }

    fn as_leaf_comparator(&mut self) -> &mut dyn LeafFieldComparator {
        self
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn compare_values(&self, first: &SortValue, second: &SortValue) -> i32 {
        self.compare_bytes(first.as_bytes(), second.as_bytes())
    }

    fn set_single_sort(&mut self) {
        self.single_sort = true;
    }

    fn disable_skipping(&mut self) {
        self.can_skip_documents = false;
    }
}
