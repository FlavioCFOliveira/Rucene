//! Multi-segment postings aggregation ported from `org.apache.lucene.index`.
//!
//! This module provides [`MultiFields`] and [`MultiTerms`], the utilities that
//! present a composite reader's multiple leaves as a single [`Fields`] /
//! [`Terms`] view. The merge is driven by [`MultiTermsEnum`], a min-heap of
//! per-leaf [`TermsEnum`]s that interleaves terms in sorted order, dedups
//! identical terms across leaves, and OR-merges postings with the correct
//! per-leaf `docBase` offset.
//!
//! # Merge strategy
//!
//! `MultiTermsEnum` mirrors Lucene's `MultiTermsEnum`:
//!
//! - A min-heap (Rust's `std::collections::BinaryHeap`, reversed by term order)
//!   keyed on each sub-enum's current term, plus a `top` set holding every sub
//!   positioned at the current (smallest) term.
//! - `next()` first `push_top`s each sub in `top` (calling `next()` on it and
//!   re-pushing the survivors), then `pull_top`s the new heap minimum and
//!   gathers all subs that share that term into `top`. Identical terms across
//!   leaves are thus deduped and exposed once.
//! - `doc_freq` / `total_term_freq` sum over `top` (the subs holding the
//!   current term).
//! - `postings` builds a [`MultiPostingsEnum`] that concatenates the per-leaf
//!   postings (each leaf's local doc IDs offset by its slice `start`); because
//!   the slices partition the global doc-ID space, the concatenation is a
//!   globally sorted doc-ID stream — exactly Lucene's `MultiPostingsEnum`.
//! - `seek_exact` / `seek_ceil` re-seed the heap from every sub, applying the
//!   LUCENE-2130 optimisation: when the new seek term is not before the last
//!   one, a sub whose current term is already past the seek term is not
//!   re-seeked.
//!
//! # Reference
//!
//! - `org.apache.lucene.index.MultiFields`
//! - `org.apache.lucene.index.MultiTerms`
//! - `org.apache.lucene.index.MultiTermsEnum`
//! - `org.apache.lucene.index.MultiPostingsEnum`
//! - `org.apache.lucene.util.MergedIterator` (for the field-name merge)
//!
//! This port does **not** transcribe Lucene's `PriorityQueue`; it uses an
//! idiomatic Rust binary heap with a reversed `Ord` wrapper to obtain min-heap
//! semantics by term.

#![deny(unsafe_code)]

use std::cmp::Ordering;
use std::collections::{BTreeSet, BinaryHeap};
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::leaf_reader::LeafReader;
use crate::index::postings_enum::{Impacts, ImpactsEnum, ImpactsSource, PostingsEnum};
use crate::index::reader_context::LeafReaderContext;
use crate::index::terms::{EmptyTermsEnum, Fields, SeekStatus, TermState, Terms, TermsEnum};
use crate::index::{IndexReader, ReaderSlice};
use crate::search::DocIdSetIterator;
use crate::util::attribute::AttributeSource;
use crate::util::automaton::CompiledAutomaton;
use crate::util::BytesRef;

// ---------------------------------------------------------------------------
// LeafFields — adapter exposing a LeafReader as a Fields instance.
// ---------------------------------------------------------------------------

/// `Fields` view over a single [`LeafReader`], built from its
/// [`FieldInfos`](crate::index::FieldInfos) and
/// [`LeafReader::terms`](LeafReader::terms).
///
/// This is the Rust analogue of the `Fields` instance returned by
/// `CodecReader.fields()` in Lucene: it iterates the leaf's field names (via
/// `FieldInfos`) and delegates `terms(field)` to `LeafReader::terms`. It is
/// the per-leaf input that [`MultiFields`] aggregates across a composite
/// reader's leaves.
///
/// Unlike Lucene's `LeafReader.fields()`, which is a codec-level method, this
/// adapter is constructed here because Rucene's `LeafReader` trait exposes
/// `terms(field)` and `get_field_infos()` directly.
struct LeafFields {
    reader: Arc<dyn LeafReader>,
}

impl LeafFields {
    fn new(reader: Arc<dyn LeafReader>) -> Self {
        Self { reader }
    }
}

impl Fields for LeafFields {
    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        // FieldInfos is returned by value; collect the names into an owned
        // Vec so the returned iterator does not borrow from a local. The
        // order follows FieldInfos' iteration order; MultiFields re-sorts the
        // merged stream, so order here is not contractual.
        let infos = self.reader.get_field_infos();
        let names: Vec<String> = infos.iter().map(|fi| fi.get_name().to_string()).collect();
        Box::new(names.into_iter())
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        self.reader.terms(field)
    }

    fn size(&self) -> i32 {
        self.reader.get_field_infos().len() as i32
    }
}

// ---------------------------------------------------------------------------
// MultiFields
// ---------------------------------------------------------------------------

/// A [`Fields`] instance presenting the union of one field index per
/// sub-reader of a composite reader.
///
/// Equivalent to `org.apache.lucene.index.MultiFields`. `iterator()` yields
/// the deduplicated, sorted union of field names across all sub-`Fields`;
/// `terms(field)` returns a [`MultiTerms`] aggregating every sub that has the
/// field.
///
/// # Performance note
///
/// Lucene warns — and the same applies here — that for composite readers it is
/// usually better to operate per-`LeafReader` (via
/// [`IndexReader::leaves`](crate::index::IndexReader::leaves)) than through
/// this class, which stitches the leaves back into a single view.
pub struct MultiFields {
    subs: Vec<Box<dyn Fields>>,
    sub_slices: Vec<ReaderSlice>,
}

impl MultiFields {
    /// Creates a `MultiFields` over the given sub-`Fields` and their
    /// corresponding [`ReaderSlice`]s.
    ///
    /// Equivalent to `MultiFields(Fields[] subs, ReaderSlice[] subSlices)`.
    /// The two arrays must be the same length and index-aligned.
    pub fn new(subs: Vec<Box<dyn Fields>>, sub_slices: Vec<ReaderSlice>) -> Self {
        assert_eq!(
            subs.len(),
            sub_slices.len(),
            "MultiFields: subs and subSlices must have equal length"
        );
        Self { subs, sub_slices }
    }

    /// Builds a `MultiFields` over all the leaves of `reader`, mirroring the
    /// role of Lucene's `MultiFields.getFields(IndexReader)`.
    ///
    /// Each leaf is wrapped in a [`LeafFields`] adapter paired with a
    /// [`ReaderSlice`] whose `start` is the leaf's `docBase`, `length` is its
    /// `maxDoc`, and `reader_index` is its leaf ordinal.
    pub fn get_fields(reader: &Arc<dyn IndexReader>) -> Result<Self> {
        let leaves: Vec<Arc<LeafReaderContext>> = Arc::clone(reader).leaves();
        let mut subs: Vec<Box<dyn Fields>> = Vec::with_capacity(leaves.len());
        let mut sub_slices: Vec<ReaderSlice> = Vec::with_capacity(leaves.len());
        for ctx in &leaves {
            let leaf = ctx.leaf_reader();
            let slice = ReaderSlice::new(ctx.doc_base(), leaf.max_doc(), ctx.ord());
            subs.push(Box::new(LeafFields::new(leaf)));
            sub_slices.push(slice);
        }
        Ok(Self { subs, sub_slices })
    }

    /// Returns the sub-`Fields` instances being merged.
    pub fn get_sub_fields(&self) -> &[Box<dyn Fields>] {
        &self.subs
    }

    /// Returns the [`ReaderSlice`]s parallel to the sub-`Fields`.
    pub fn get_sub_slices(&self) -> &[ReaderSlice] {
        &self.sub_slices
    }
}

impl Fields for MultiFields {
    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        // Merge sorted-dedup view of all sub field-name iterators. Field-name
        // counts are small, so collecting into a BTreeSet gives the exact
        // semantics of Lucene's MergedIterator (sorted union with dedup) without
        // a streaming k-way merge.
        let mut names: BTreeSet<String> = BTreeSet::new();
        for sub in &self.subs {
            for name in sub.iterator() {
                names.insert(name);
            }
        }
        Box::new(names.into_iter())
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        let mut subs2: Vec<Box<dyn Terms>> = Vec::new();
        let mut slices2: Vec<ReaderSlice> = Vec::new();
        for (i, sub) in self.subs.iter().enumerate() {
            if let Some(terms) = sub.terms(field)? {
                subs2.push(terms);
                slices2.push(self.sub_slices[i]);
            }
        }
        if subs2.is_empty() {
            Ok(None)
        } else {
            Ok(Some(Box::new(MultiTerms::new(subs2, slices2)?)))
        }
    }

    fn size(&self) -> i32 {
        // Matches Lucene: the number of distinct fields is not cheap to report
        // without materialising the union, so return -1 ("unknown").
        -1
    }
}

// ---------------------------------------------------------------------------
// MultiTerms
// ---------------------------------------------------------------------------

/// A [`Terms`] instance aggregating one field across several sub-readers'
/// `Terms`.
///
/// Equivalent to `org.apache.lucene.index.MultiTerms`. `iterator()` returns a
/// merged [`MultiTermsEnum`]; `intersect()` delegates to each sub's
/// `intersect` and merges the results. Statistics (`sum_total_term_freq`,
/// `sum_doc_freq`, `doc_count`) are summed across subs. The `has_*` flags are
/// aggregated with Lucene's semantics: `hasFreqs`/`hasOffsets`/`hasPositions`
/// are AND (all subs must have the feature); `hasPayloads` is
/// `hasPositions && any sub has payloads`.
pub struct MultiTerms {
    subs: Vec<Box<dyn Terms>>,
    sub_slices: Vec<ReaderSlice>,
    has_freqs: bool,
    has_offsets: bool,
    has_positions: bool,
    has_payloads: bool,
}

impl MultiTerms {
    /// Creates a `MultiTerms` over the given sub-`Terms` and their
    /// corresponding [`ReaderSlice`]s.
    ///
    /// Equivalent to `MultiTerms(Terms[] subs, ReaderSlice[] subSlices)`.
    pub fn new(subs: Vec<Box<dyn Terms>>, sub_slices: Vec<ReaderSlice>) -> Result<Self> {
        assert_eq!(
            subs.len(),
            sub_slices.len(),
            "MultiTerms: subs and subSlices must have equal length"
        );
        // Lucene asserts subs.length > 0 ("inefficient: don't use MultiTerms
        // over one sub"); we tolerate it because get_terms() may collapse to
        // a single sub before the caller decides what to do.
        let mut has_freqs = true;
        let mut has_offsets = true;
        let mut has_positions = true;
        let mut any_payloads = false;
        for sub in &subs {
            has_freqs &= sub.has_freqs();
            has_offsets &= sub.has_offsets();
            has_positions &= sub.has_positions();
            any_payloads |= sub.has_payloads();
        }
        let has_payloads = has_positions && any_payloads;
        Ok(Self {
            subs,
            sub_slices,
            has_freqs,
            has_offsets,
            has_positions,
            has_payloads,
        })
    }

    /// Returns the `Terms` for `field` across `reader`'s leaves, or `None` if
    /// no leaf has the field.
    ///
    /// Equivalent to `MultiTerms.getTerms(IndexReader r, String field)`.
    /// When the reader has a single leaf, the leaf's own `terms(field)` is
    /// returned directly (no `MultiTerms` wrapper), matching Lucene.
    pub fn get_terms(reader: &Arc<dyn IndexReader>, field: &str) -> Result<Option<Box<dyn Terms>>> {
        let leaves = Arc::clone(reader).leaves();
        if leaves.len() == 1 {
            return leaves[0].leaf_reader().terms(field);
        }
        let mut subs2: Vec<Box<dyn Terms>> = Vec::with_capacity(leaves.len());
        let mut slices2: Vec<ReaderSlice> = Vec::with_capacity(leaves.len());
        let max_doc = reader.max_doc();
        for (idx, ctx) in leaves.iter().enumerate() {
            if let Some(sub_terms) = ctx.leaf_reader().terms(field)? {
                subs2.push(sub_terms);
                slices2.push(ReaderSlice::new(ctx.doc_base(), max_doc, idx as i32));
            }
        }
        if subs2.is_empty() {
            Ok(None)
        } else {
            Ok(Some(Box::new(MultiTerms::new(subs2, slices2)?)))
        }
    }

    /// Returns a [`PostingsEnum`] for `term` in `field` across `reader`, or
    /// `None` if the field or term does not exist.
    ///
    /// Equivalent to
    /// `MultiTerms.getTermPostingsEnum(IndexReader, String, BytesRef, int)`.
    pub fn get_term_postings_enum(
        reader: &Arc<dyn IndexReader>,
        field: &str,
        term: &BytesRef,
        flags: i32,
    ) -> Result<Option<Box<dyn PostingsEnum>>> {
        if let Some(terms) = Self::get_terms(reader, field)? {
            let mut te = terms.iterator()?;
            if te.seek_exact(term)? {
                return Ok(Some(te.postings(None, flags)?));
            }
        }
        Ok(None)
    }

    /// Returns the sub-`Terms` being merged.
    pub fn get_sub_terms(&self) -> &[Box<dyn Terms>] {
        &self.subs
    }

    /// Returns the [`ReaderSlice`]s parallel to the sub-`Terms`.
    pub fn get_sub_slices(&self) -> &[ReaderSlice] {
        &self.sub_slices
    }

    /// Builds a merged [`TermsEnum`] from the given per-sub `TermsEnum`s.
    ///
    /// Mirrors `MultiTermsEnum.reset(TermsEnumIndex[])`: each sub-enum is
    /// advanced once (its first term seeded into the heap), and if no sub
    /// yields any term an [`EmptyTermsEnum`] is returned instead.
    fn build_enum(
        sub_slices: &[ReaderSlice],
        entries: Vec<(Box<dyn TermsEnum>, ReaderSlice)>,
    ) -> Result<Box<dyn TermsEnum>> {
        let mut me = MultiTermsEnum::new(sub_slices);
        for (te, slice) in entries {
            me.add_sub(te, slice)?;
        }
        me.finish()
    }
}

impl Terms for MultiTerms {
    fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
        let mut entries: Vec<(Box<dyn TermsEnum>, ReaderSlice)> =
            Vec::with_capacity(self.subs.len());
        for (i, sub) in self.subs.iter().enumerate() {
            // Lucene guards against null iterators; Rucene's Terms::iterator
            // is infallible per-sub (returns Result, not Option), so we always
            // include the sub-enum. An exhausted sub-enum simply contributes
            // no terms to the heap.
            entries.push((sub.iterator()?, self.sub_slices[i]));
        }
        Self::build_enum(&self.sub_slices, entries)
    }

    fn intersect(
        &self,
        compiled: &CompiledAutomaton,
        start_term: Option<&BytesRef>,
    ) -> Result<Box<dyn TermsEnum>> {
        let mut entries: Vec<(Box<dyn TermsEnum>, ReaderSlice)> =
            Vec::with_capacity(self.subs.len());
        for (i, sub) in self.subs.iter().enumerate() {
            // Terms::intersect returns UnsupportedOperation by default; subs
            // that do not implement it surface as an error here, matching the
            // expectation that a real segment reader provides intersect.
            let te = sub.intersect(compiled, start_term)?;
            entries.push((te, self.sub_slices[i]));
        }
        Self::build_enum(&self.sub_slices, entries)
    }

    fn size(&self) -> i64 {
        // Matches Lucene: the merged term count is not cheap to compute.
        -1
    }

    fn sum_total_term_freq(&self) -> i64 {
        let mut sum: i64 = 0;
        for sub in &self.subs {
            sum += sub.sum_total_term_freq();
        }
        sum
    }

    fn sum_doc_freq(&self) -> i64 {
        let mut sum: i64 = 0;
        for sub in &self.subs {
            sum += sub.sum_doc_freq();
        }
        sum
    }

    fn doc_count(&self) -> i32 {
        let mut sum: i32 = 0;
        for sub in &self.subs {
            sum = sum.wrapping_add(sub.doc_count());
        }
        sum
    }

    fn has_freqs(&self) -> bool {
        self.has_freqs
    }

    fn has_offsets(&self) -> bool {
        self.has_offsets
    }

    fn has_positions(&self) -> bool {
        self.has_positions
    }

    fn has_payloads(&self) -> bool {
        self.has_payloads
    }

    fn min(&self) -> Result<Option<BytesRef>> {
        let mut min_term: Option<BytesRef> = None;
        for sub in &self.subs {
            if let Some(t) = sub.min()? {
                match &min_term {
                    None => min_term = Some(t),
                    Some(cur) => {
                        if t < *cur {
                            min_term = Some(t);
                        }
                    }
                }
            }
        }
        Ok(min_term)
    }

    fn max(&self) -> Result<Option<BytesRef>> {
        let mut max_term: Option<BytesRef> = None;
        for sub in &self.subs {
            if let Some(t) = sub.max()? {
                match &max_term {
                    None => max_term = Some(t),
                    Some(cur) => {
                        if t > *cur {
                            max_term = Some(t);
                        }
                    }
                }
            }
        }
        Ok(max_term)
    }
}

// ---------------------------------------------------------------------------
// MultiTermsEnum
// ---------------------------------------------------------------------------

/// One sub-enum entry inside [`MultiTermsEnum`].
struct TermsEnumEntry {
    tenum: Box<dyn TermsEnum>,
    /// Slice identifying this sub-reader's place in the global doc-ID space.
    slice: ReaderSlice,
    /// The sub-enum's current term, or `None` when exhausted. Updated only
    /// while the entry is not in the heap (after popping).
    current: Option<BytesRef>,
}

impl TermsEnumEntry {
    fn new(tenum: Box<dyn TermsEnum>, slice: ReaderSlice) -> Self {
        Self {
            tenum,
            slice,
            current: None,
        }
    }
}

/// Heap element: index into `MultiTermsEnum::subs` plus a snapshot of that
/// sub's current term. The snapshot is taken at push time and stays valid
/// while the element sits in the heap (the entry is only mutated after
/// popping), so the heap invariant is preserved.
///
/// `Ord` is reversed on `term` so that Rust's max-heap `BinaryHeap` behaves as
/// a min-heap by unsigned byte order (the same order `BytesRef::cmp` gives).
struct HeapEntry {
    idx: usize,
    term: BytesRef,
}

impl PartialEq for HeapEntry {
    fn eq(&self, other: &Self) -> bool {
        self.term == other.term && self.idx == other.idx
    }
}

impl Eq for HeapEntry {}

impl PartialOrd for HeapEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for HeapEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse term comparison for min-heap; idx is a stable tiebreaker.
        other.term.cmp(&self.term).then(other.idx.cmp(&self.idx))
    }
}

/// A [`TermsEnum`] that merge-sorts sub-`TermsEnum`s by term text, dedups
/// identical terms across subs, and aggregates per-term statistics and
/// postings across the subs positioned at the current term.
///
/// Equivalent to `org.apache.lucene.index.MultiTermsEnum`.
pub struct MultiTermsEnum {
    /// All sub-entries, persistent for the life of this enum (needed by seek).
    subs: Vec<TermsEnumEntry>,
    /// Min-heap of indices into `subs`, keyed by the sub's current term.
    heap: BinaryHeap<HeapEntry>,
    /// Indices of subs positioned at the current term.
    top: Vec<usize>,
    /// The current term, or `None` once exhausted.
    current: Option<BytesRef>,
    /// Last seek term, for the LUCENE-2130 seek-optimisation.
    last_seek: Option<BytesRef>,
    /// Set by `seek_exact` so the next `next()` re-seeds non-matching subs via
    /// `seek_ceil(current)` before advancing, matching Lucene.
    last_seek_exact: bool,
    atts: AttributeSource,
}

impl MultiTermsEnum {
    /// Creates a fresh `MultiTermsEnum` sized for `sub_slices`.
    ///
    /// Use [`Self::add_sub`] to seed each sub-enum, then [`Self::finish`] to
    /// obtain the boxed enum (or an [`EmptyTermsEnum`] if no sub yields a term).
    fn new(sub_slices: &[ReaderSlice]) -> Self {
        let n = sub_slices.len();
        Self {
            subs: Vec::with_capacity(n),
            heap: BinaryHeap::with_capacity(n),
            top: Vec::with_capacity(n),
            current: None,
            last_seek: None,
            last_seek_exact: false,
            atts: AttributeSource::new(),
        }
    }

    /// Adds a sub-enum with its slice, seeding the heap with the sub's first
    /// term (if any). Mirrors the per-sub loop in `MultiTermsEnum.reset`.
    fn add_sub(&mut self, tenum: Box<dyn TermsEnum>, slice: ReaderSlice) -> Result<()> {
        let idx = self.subs.len();
        let mut entry = TermsEnumEntry::new(tenum, slice);
        let first = entry.tenum.next()?;
        if let Some(t) = first {
            entry.current = Some(t.clone());
            self.heap.push(HeapEntry { idx, term: t });
        } else {
            entry.current = None;
        }
        self.subs.push(entry);
        Ok(())
    }

    /// Finalises construction, returning a boxed enum. If no sub yielded a
    /// term, returns [`EmptyTermsEnum`] — matching Lucene's
    /// `reset()` returning `TermsEnum.EMPTY` when the queue is empty.
    fn finish(self) -> Result<Box<dyn TermsEnum>> {
        if self.heap.is_empty() {
            Ok(Box::new(EmptyTermsEnum::new()))
        } else {
            Ok(Box::new(self))
        }
    }

    /// Advances every sub in `top` and pushes the survivors back into the
    /// heap. Mirrors `MultiTermsEnum.pushTop`.
    fn push_top(&mut self) -> Result<()> {
        for &idx in &self.top {
            let next_term = self.subs[idx].tenum.next()?;
            match next_term {
                Some(t) => {
                    self.subs[idx].current = Some(t.clone());
                    self.heap.push(HeapEntry { idx, term: t });
                }
                None => {
                    self.subs[idx].current = None;
                }
            }
        }
        self.top.clear();
        Ok(())
    }

    /// Pops the heap minimum into `top` and gathers every sub sharing that
    /// term. Sets `current` to the gathered term. Mirrors
    /// `MultiTermsEnum.pullTop` / `TermMergeQueue.fillTop`.
    fn pull_top(&mut self) {
        let Some(min) = self.heap.pop() else {
            self.current = None;
            return;
        };
        let first_term = min.term.clone();
        self.top.push(min.idx);
        while let Some(peek) = self.heap.peek() {
            if peek.term == first_term {
                let e = self.heap.pop().unwrap();
                self.top.push(e.idx);
            } else {
                break;
            }
        }
        self.current = Some(first_term);
    }

    /// Re-seeds the heap and top from every sub after a seek, applying the
    /// LUCENE-2130 optimisation when `seek_opt` is set.
    ///
    /// Shared by `seek_exact` (with `seek_opt = false`) and `seek_ceil`.
    fn reseed(&mut self, term: &BytesRef, seek_opt: bool, exact: bool) -> Result<()> {
        self.heap.clear();
        self.top.clear();
        self.last_seek_exact = exact;
        self.last_seek = Some(term.clone());

        for i in 0..self.subs.len() {
            if seek_opt {
                // LUCENE-2130: if this sub's current term is already past
                // `term`, don't re-seek it. Compare against the live current.
                if let Some(cur) = &self.subs[i].current {
                    match term.cmp(cur) {
                        Ordering::Equal => {
                            if exact {
                                // seek_exact: sub is already on the term.
                                self.top.push(i);
                            } else {
                                // seek_ceil: sub is on the term → FOUND.
                                self.top.push(i);
                            }
                            continue;
                        }
                        Ordering::Less => {
                            // Sub's current term is past `term`: for seek_ceil
                            // this is a NOT_FOUND ceiling; for seek_exact the
                            // sub does not hold the term. In both cases leave
                            // the sub where it is and push to the heap (its
                            // current term is the ceiling).
                            self.heap.push(HeapEntry {
                                idx: i,
                                term: cur.clone(),
                            });
                            continue;
                        }
                        Ordering::Greater => {
                            // Sub's current term is before `term`: fall through
                            // to the actual seek.
                        }
                    }
                } else {
                    // Sub exhausted previously; for seek_ceil it's END, for
                    // seek_exact it cannot match. Skip.
                    continue;
                }
            }

            if exact {
                let found = self.subs[i].tenum.seek_exact(term)?;
                if found {
                    self.subs[i].current = Some(term.clone());
                    self.top.push(i);
                } else {
                    self.subs[i].current = None;
                }
            } else {
                let status = self.subs[i].tenum.seek_ceil(term)?;
                match status {
                    SeekStatus::FOUND => {
                        self.subs[i].current = Some(term.clone());
                        self.top.push(i);
                    }
                    SeekStatus::NOT_FOUND => {
                        let cur = self.subs[i].tenum.term()?;
                        self.subs[i].current = Some(cur.clone());
                        self.heap.push(HeapEntry { idx: i, term: cur });
                    }
                    SeekStatus::END => {
                        self.subs[i].current = None;
                    }
                }
            }
        }
        Ok(())
    }
}

impl TermsEnum for MultiTermsEnum {
    fn attributes(&mut self) -> &mut AttributeSource {
        &mut self.atts
    }

    fn seek_exact(&mut self, text: &BytesRef) -> Result<bool> {
        // LUCENE-2130: optimise only if the new term is not before the last
        // seek term.
        let seek_opt = self.last_seek.as_ref().is_some_and(|ls| ls <= text);
        self.reseed(text, seek_opt, true)?;
        if self.top.is_empty() {
            self.current = None;
            Ok(false)
        } else {
            self.current = Some(text.clone());
            Ok(true)
        }
    }

    fn seek_ceil(&mut self, text: &BytesRef) -> Result<SeekStatus> {
        let seek_opt = self.last_seek.as_ref().is_some_and(|ls| ls <= text);
        self.reseed(text, seek_opt, false)?;
        if !self.top.is_empty() {
            // At least one sub had an exact match.
            self.current = Some(text.clone());
            Ok(SeekStatus::FOUND)
        } else if !self.heap.is_empty() {
            // No exact match; advance to the ceiling term.
            self.pull_top();
            Ok(SeekStatus::NOT_FOUND)
        } else {
            self.current = None;
            Ok(SeekStatus::END)
        }
    }

    fn seek_ord(&mut self, _ord: i64) -> Result<()> {
        Err(LuceneError::UnsupportedOperation(
            "MultiTermsEnum does not support seekOrd".to_string(),
        ))
    }

    fn term(&self) -> Result<BytesRef> {
        self.current.clone().ok_or_else(|| {
            LuceneError::IllegalState(
                "MultiTermsEnum.term() called with no current term".to_string(),
            )
        })
    }

    fn ord(&self) -> Result<i64> {
        Err(LuceneError::UnsupportedOperation(
            "MultiTermsEnum does not support ord".to_string(),
        ))
    }

    fn doc_freq(&self) -> Result<i32> {
        let mut sum: i32 = 0;
        for &idx in &self.top {
            sum = sum.wrapping_add(self.subs[idx].tenum.doc_freq()?);
        }
        Ok(sum)
    }

    fn total_term_freq(&self) -> Result<i64> {
        let mut sum: i64 = 0;
        for &idx in &self.top {
            sum += self.subs[idx].tenum.total_term_freq()?;
        }
        Ok(sum)
    }

    fn postings(
        &mut self,
        _reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Box<dyn PostingsEnum>> {
        // Sort the top subs by reader_index so the MultiPostingsEnum receives
        // slices in ascending doc-ID order (Lucene does the same via
        // ArrayUtil.timSort on subIndex).
        let mut top_sorted: Vec<usize> = self.top.clone();
        top_sorted.sort_by_key(|&idx| self.subs[idx].slice.reader_index);

        let mut sub_docs: Vec<(Box<dyn PostingsEnum>, i32)> = Vec::with_capacity(top_sorted.len());
        for &idx in &top_sorted {
            // Sub-enum reuse across calls is not tracked here; each call asks
            // the sub for a fresh postings enum. This matches Lucene's behaviour
            // (which passes the previous sub-enum as reuse) when the sub
            // implementations do not special-case reuse, and is correct.
            let pe = self.subs[idx].tenum.postings(None, flags)?;
            sub_docs.push((pe, self.subs[idx].slice.start));
        }
        Ok(Box::new(MultiPostingsEnum::new(sub_docs)))
    }

    fn impacts(&mut self, flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        // Mirrors Lucene: wrap the merged postings in a SlowImpactsEnum, since
        // impacts are not indexed across the merge.
        Ok(Box::new(SlowImpactsEnum::new(self.postings(None, flags)?)))
    }

    fn term_state(&mut self) -> Result<Box<dyn TermState>> {
        Err(LuceneError::UnsupportedOperation(
            "MultiTermsEnum does not support term_state".to_string(),
        ))
    }

    fn next(&mut self) -> Result<Option<BytesRef>> {
        if self.last_seek_exact {
            // After a seek_exact, subs that did not hold the term are left at
            // arbitrary positions; re-seed via seek_ceil(current) so they line
            // up at/below the current term before we advance, exactly as in
            // Lucene's next().
            self.last_seek_exact = false;
            if let Some(cur) = self.current.clone() {
                let _ = self.seek_ceil(&cur)?;
            }
        }
        self.last_seek = None;

        self.push_top()?;
        if !self.heap.is_empty() {
            self.pull_top();
        } else {
            self.current = None;
        }
        Ok(self.current.clone())
    }
}

// ---------------------------------------------------------------------------
// MultiPostingsEnum
// ---------------------------------------------------------------------------

/// A [`PostingsEnum`] that concatenates several sub-readers' postings, offset
/// by each sub's slice `start`, producing a globally sorted doc-ID stream.
///
/// Equivalent to `org.apache.lucene.index.MultiPostingsEnum`. The slices
/// partition the global doc-ID space (each sub's local doc IDs are re-based by
/// `slice.start`), so a plain concatenation yields the same globally increasing
/// doc-ID sequence a merge would — without a per-doc comparison.
struct MultiPostingsEnum {
    /// `(postings, slice_start)` pairs, in ascending slice order.
    subs: Vec<(Box<dyn PostingsEnum>, i32)>,
    /// Index into `subs` of the current sub, or `-1` before the first
    /// `next_doc`/`advance`.
    upto: i32,
    /// Current global doc ID (`-1` before the first positioning).
    doc: i32,
}

impl MultiPostingsEnum {
    fn new(subs: Vec<(Box<dyn PostingsEnum>, i32)>) -> Self {
        Self {
            subs,
            upto: -1,
            doc: -1,
        }
    }

    /// Returns the current sub's index, or an error if none is active.
    fn current_idx(&self) -> Result<usize> {
        if self.upto >= 0 && (self.upto as usize) < self.subs.len() {
            Ok(self.upto as usize)
        } else {
            Err(LuceneError::IllegalState(
                "MultiPostingsEnum: no current postings enum".to_string(),
            ))
        }
    }
}

impl DocIdSetIterator for MultiPostingsEnum {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        loop {
            let i = self.upto;
            if i >= 0 && (i as usize) < self.subs.len() {
                let d = self.subs[i as usize].0.next_doc()?;
                if d == crate::search::NO_MORE_DOCS {
                    // Exhausted current sub; advance to the next.
                    self.upto += 1;
                } else {
                    self.doc = d + self.subs[i as usize].1;
                    return Ok(self.doc);
                }
            } else if i < self.subs.len() as i32 - 1 {
                self.upto += 1;
            } else {
                self.doc = crate::search::NO_MORE_DOCS;
                return Ok(self.doc);
            }
        }
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        loop {
            let i = self.upto;
            if i >= 0 && (i as usize) < self.subs.len() {
                let base = self.subs[i as usize].1;
                // target < base: target was in a previous slice that had no
                // matching doc after it — just next_doc the current sub.
                let d = if target < base {
                    self.subs[i as usize].0.next_doc()?
                } else {
                    self.subs[i as usize].0.advance(target - base)?
                };
                if d == crate::search::NO_MORE_DOCS {
                    self.upto += 1;
                } else {
                    self.doc = d + base;
                    return Ok(self.doc);
                }
            } else if i < self.subs.len() as i32 - 1 {
                self.upto += 1;
            } else {
                self.doc = crate::search::NO_MORE_DOCS;
                return Ok(self.doc);
            }
        }
    }

    fn cost(&self) -> i64 {
        let mut cost: i64 = 0;
        for (pe, _) in &self.subs {
            cost += pe.cost();
        }
        cost
    }
}

impl PostingsEnum for MultiPostingsEnum {
    fn freq(&self) -> Result<i32> {
        self.current_idx().and_then(|i| self.subs[i].0.freq())
    }

    fn next_position(&mut self) -> Result<i32> {
        let i = self.current_idx()?;
        self.subs[i].0.next_position()
    }

    fn start_offset(&self) -> i32 {
        match self.current_idx() {
            Ok(i) => self.subs[i].0.start_offset(),
            Err(_) => -1,
        }
    }

    fn end_offset(&self) -> i32 {
        match self.current_idx() {
            Ok(i) => self.subs[i].0.end_offset(),
            Err(_) => -1,
        }
    }

    fn get_payload(&self) -> Result<Option<&[u8]>> {
        let i = self.current_idx()?;
        self.subs[i].0.get_payload()
    }
}

// ---------------------------------------------------------------------------
// SlowImpactsEnum — used by MultiTermsEnum::impacts.
// ---------------------------------------------------------------------------

/// `ImpactsEnum` that wraps a [`PostingsEnum`] and reports a single trivial
/// impact level, matching `org.apache.lucene.index.SlowImpactsEnum`.
struct SlowImpactsEnum {
    delegate: Box<dyn PostingsEnum>,
}

impl SlowImpactsEnum {
    fn new(delegate: Box<dyn PostingsEnum>) -> Self {
        Self { delegate }
    }
}

struct SlowImpacts;

impl SlowImpacts {
    fn new() -> Self {
        Self
    }
}

impl DocIdSetIterator for SlowImpactsEnum {
    fn doc_id(&self) -> i32 {
        self.delegate.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.delegate.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.delegate.advance(target)
    }

    fn cost(&self) -> i64 {
        self.delegate.cost()
    }
}

impl PostingsEnum for SlowImpactsEnum {
    fn freq(&self) -> Result<i32> {
        self.delegate.freq()
    }

    fn next_position(&mut self) -> Result<i32> {
        self.delegate.next_position()
    }

    fn start_offset(&self) -> i32 {
        self.delegate.start_offset()
    }

    fn end_offset(&self) -> i32 {
        self.delegate.end_offset()
    }

    fn get_payload(&self) -> Result<Option<&[u8]>> {
        self.delegate.get_payload()
    }
}

impl ImpactsSource for SlowImpactsEnum {
    fn advance_shallow(&mut self, _target: i32) -> Result<()> {
        Ok(())
    }

    fn get_impacts(&mut self) -> Result<Box<dyn Impacts>> {
        Ok(Box::new(SlowImpacts::new()))
    }
}

impl ImpactsEnum for SlowImpactsEnum {}

impl Impacts for SlowImpacts {
    fn num_levels(&self) -> i32 {
        1
    }

    fn doc_id_up_to(&self, _level: i32) -> i32 {
        crate::search::NO_MORE_DOCS
    }

    fn get_impacts(&self, _level: i32) -> crate::index::postings_enum::FreqAndNormBuffer {
        let mut buf = crate::index::postings_enum::FreqAndNormBuffer::new();
        buf.add(i32::MAX, 1);
        buf
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::postings_enum::{ImpactsEnum, PostingsEnum, POSTINGS_ENUM_FREQS};
    use crate::index::terms::{EmptyTerms, Fields, SeekStatus, Terms, TermsEnum};
    use crate::index::{EmptyFields, MultiReader, ReaderSlice};
    use crate::search::DocIdSetIterator;
    use crate::util::BytesRef;

    // ----- minimal in-memory fakes for Terms / TermsEnum / PostingsEnum -----

    /// Postings enum over a fixed list of local doc IDs, each with freq 1.
    struct VecPostings {
        docs: Vec<i32>,
        pos: i32, // index into docs, -1 before first
    }

    impl VecPostings {
        fn new(docs: Vec<i32>) -> Self {
            Self { docs, pos: -1 }
        }
    }

    impl DocIdSetIterator for VecPostings {
        fn doc_id(&self) -> i32 {
            if self.pos < 0 {
                -1
            } else if (self.pos as usize) >= self.docs.len() {
                crate::search::NO_MORE_DOCS
            } else {
                self.docs[self.pos as usize]
            }
        }

        fn next_doc(&mut self) -> Result<i32> {
            self.pos += 1;
            if (self.pos as usize) >= self.docs.len() {
                Ok(crate::search::NO_MORE_DOCS)
            } else {
                Ok(self.docs[self.pos as usize])
            }
        }

        fn advance(&mut self, target: i32) -> Result<i32> {
            let start = ((self.pos + 1).max(0)) as usize;
            match self.docs[start..].iter().position(|&d| d >= target) {
                Some(p) => {
                    self.pos = (start + p) as i32;
                    Ok(self.docs[start + p])
                }
                None => {
                    self.pos = self.docs.len() as i32;
                    Ok(crate::search::NO_MORE_DOCS)
                }
            }
        }

        fn cost(&self) -> i64 {
            self.docs.len() as i64
        }
    }

    impl PostingsEnum for VecPostings {
        fn freq(&self) -> Result<i32> {
            Ok(1)
        }

        fn next_position(&mut self) -> Result<i32> {
            Ok(-1)
        }

        fn start_offset(&self) -> i32 {
            -1
        }

        fn end_offset(&self) -> i32 {
            -1
        }

        fn get_payload(&self) -> Result<Option<&[u8]>> {
            Ok(None)
        }
    }

    /// TermsEnum over a sorted list of `(term, doc_freq, total_term_freq, docs)`.
    struct VecTermsEnum {
        entries: Vec<(BytesRef, i32, i64, Vec<i32>)>,
        pos: i32, // -1 before first next(); entry index otherwise
        atts: AttributeSource,
    }

    impl VecTermsEnum {
        fn new(entries: Vec<(BytesRef, i32, i64, Vec<i32>)>) -> Self {
            // Caller must supply entries sorted by term.
            Self {
                entries,
                pos: -1,
                atts: AttributeSource::new(),
            }
        }
    }

    impl TermsEnum for VecTermsEnum {
        fn attributes(&mut self) -> &mut AttributeSource {
            &mut self.atts
        }

        fn seek_exact(&mut self, text: &BytesRef) -> Result<bool> {
            match self.entries.iter().position(|(t, _, _, _)| t == text) {
                Some(i) => {
                    self.pos = i as i32;
                    Ok(true)
                }
                None => Ok(false),
            }
        }

        fn seek_ceil(&mut self, text: &BytesRef) -> Result<SeekStatus> {
            match self.entries.iter().position(|(t, _, _, _)| t >= text) {
                Some(i) => {
                    self.pos = i as i32;
                    if self.entries[i].0 == *text {
                        Ok(SeekStatus::FOUND)
                    } else {
                        Ok(SeekStatus::NOT_FOUND)
                    }
                }
                None => {
                    self.pos = self.entries.len() as i32;
                    Ok(SeekStatus::END)
                }
            }
        }

        fn seek_ord(&mut self, ord: i64) -> Result<()> {
            self.pos = ord as i32;
            Ok(())
        }

        fn term(&self) -> Result<BytesRef> {
            Ok(self.entries[self.pos as usize].0.clone())
        }

        fn ord(&self) -> Result<i64> {
            Ok(self.pos as i64)
        }

        fn doc_freq(&self) -> Result<i32> {
            Ok(self.entries[self.pos as usize].1)
        }

        fn total_term_freq(&self) -> Result<i64> {
            Ok(self.entries[self.pos as usize].2)
        }

        fn postings(
            &mut self,
            _reuse: Option<Box<dyn PostingsEnum>>,
            _flags: i32,
        ) -> Result<Box<dyn PostingsEnum>> {
            Ok(Box::new(VecPostings::new(
                self.entries[self.pos as usize].3.clone(),
            )))
        }

        fn impacts(&mut self, _flags: i32) -> Result<Box<dyn ImpactsEnum>> {
            Err(LuceneError::IllegalState("not used".to_string()))
        }

        fn next(&mut self) -> Result<Option<BytesRef>> {
            self.pos += 1;
            if (self.pos as usize) >= self.entries.len() {
                self.pos = self.entries.len() as i32;
                Ok(None)
            } else {
                Ok(Some(self.entries[self.pos as usize].0.clone()))
            }
        }
    }

    /// `Terms` over a sorted list of `(term, doc_freq, ttf, docs)`.
    #[derive(Clone)]
    struct VecTerms {
        entries: Vec<(BytesRef, i32, i64, Vec<i32>)>,
        has_freqs: bool,
        has_offsets: bool,
        has_positions: bool,
        has_payloads: bool,
    }

    impl VecTerms {
        fn new(
            entries: Vec<(BytesRef, i32, i64, Vec<i32>)>,
            has_freqs: bool,
            has_offsets: bool,
            has_positions: bool,
            has_payloads: bool,
        ) -> Self {
            Self {
                entries,
                has_freqs,
                has_offsets,
                has_positions,
                has_payloads,
            }
        }
    }

    impl Terms for VecTerms {
        fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
            Ok(Box::new(VecTermsEnum::new(self.entries.clone())))
        }

        fn size(&self) -> i64 {
            self.entries.len() as i64
        }

        fn sum_total_term_freq(&self) -> i64 {
            self.entries.iter().map(|(_, _, ttf, _)| *ttf).sum()
        }

        fn sum_doc_freq(&self) -> i64 {
            self.entries.iter().map(|(_, df, _, _)| *df as i64).sum()
        }

        fn doc_count(&self) -> i32 {
            // Number of distinct docs across all terms (approximation enough
            // for the tests; not the focus here).
            self.entries
                .iter()
                .flat_map(|(_, _, _, docs)| docs.iter().copied())
                .collect::<std::collections::BTreeSet<_>>()
                .len() as i32
        }

        fn has_freqs(&self) -> bool {
            self.has_freqs
        }

        fn has_offsets(&self) -> bool {
            self.has_offsets
        }

        fn has_positions(&self) -> bool {
            self.has_positions
        }

        fn has_payloads(&self) -> bool {
            self.has_payloads
        }
    }

    /// `Fields` over a map of field name -> VecTerms.
    struct VecFields {
        fields: std::collections::BTreeMap<String, VecTerms>,
    }

    impl VecFields {
        fn new<I>(iter: I) -> Self
        where
            I: IntoIterator<Item = (String, VecTerms)>,
        {
            Self {
                fields: iter.into_iter().collect(),
            }
        }
    }

    impl Fields for VecFields {
        fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
            Box::new(self.fields.keys().cloned().collect::<Vec<_>>().into_iter())
        }

        fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
            Ok(self
                .fields
                .get(field)
                .map(|t| Box::new(t.clone()) as Box<dyn Terms>))
        }

        fn size(&self) -> i32 {
            self.fields.len() as i32
        }
    }

    fn term(bytes: &[u8]) -> BytesRef {
        BytesRef::new(bytes.to_vec())
    }

    // ----- MultiFields tests -----

    #[test]
    fn multi_fields_iterator_returns_union_of_field_names() {
        let f1 = VecFields::new([(
            "body".to_string(),
            VecTerms::new(vec![], true, false, false, false),
        )]);
        let f2 = VecFields::new([
            (
                "body".to_string(),
                VecTerms::new(vec![], true, false, false, false),
            ),
            (
                "title".to_string(),
                VecTerms::new(vec![], true, false, false, false),
            ),
        ]);
        let mf = MultiFields::new(
            vec![Box::new(f1), Box::new(f2)],
            vec![ReaderSlice::new(0, 5, 0), ReaderSlice::new(5, 5, 1)],
        );
        let names: Vec<String> = mf.iterator().collect();
        assert_eq!(names, vec!["body".to_string(), "title".to_string()]);
        assert_eq!(mf.size(), -1); // Lucene reports -1
    }

    #[test]
    fn multi_fields_terms_absent_field_returns_none() {
        let f1 = VecFields::new([(
            "body".to_string(),
            VecTerms::new(vec![], true, false, false, false),
        )]);
        let mf = MultiFields::new(vec![Box::new(f1)], vec![ReaderSlice::new(0, 5, 0)]);
        assert!(mf.terms("missing").unwrap().is_none());
        assert!(mf.terms("body").unwrap().is_some());
    }

    #[test]
    fn multi_fields_terms_aggregates_only_leaves_that_have_the_field() {
        let body_terms = VecTerms::new(
            vec![(term(b"apple"), 1, 1, vec![0])],
            true,
            false,
            false,
            false,
        );
        let f1 = VecFields::new(vec![("body".to_string(), body_terms)]);
        let f2 = VecFields::new(vec![]); // no fields at all
        let mf = MultiFields::new(
            vec![Box::new(f1), Box::new(f2)],
            vec![ReaderSlice::new(0, 3, 0), ReaderSlice::new(3, 4, 1)],
        );
        let terms = mf.terms("body").unwrap().expect("body present in leaf 0");
        // The MultiTerms should iterate "apple" once.
        let mut te = terms.iterator().unwrap();
        assert_eq!(te.next().unwrap().unwrap().slice(), b"apple");
        assert!(te.next().unwrap().is_none());
    }

    // ----- MultiTermsEnum merge tests -----

    #[test]
    fn multi_terms_enum_interleaves_sorted_terms_with_dedup() {
        // Leaf 0 terms: apple, cherry, mango
        let t0 = VecTerms::new(
            vec![
                (term(b"apple"), 1, 1, vec![0]),
                (term(b"cherry"), 1, 1, vec![1]),
                (term(b"mango"), 1, 1, vec![2]),
            ],
            true,
            false,
            false,
            false,
        );
        // Leaf 1 terms: banana, cherry, mango (cherry and mango overlap)
        let t1 = VecTerms::new(
            vec![
                (term(b"banana"), 1, 1, vec![0]),
                (term(b"cherry"), 2, 3, vec![0, 1]),
                (term(b"mango"), 1, 2, vec![2]),
            ],
            true,
            false,
            false,
            false,
        );
        let mt = MultiTerms::new(
            vec![Box::new(t0), Box::new(t1)],
            vec![ReaderSlice::new(0, 3, 0), ReaderSlice::new(3, 3, 1)],
        )
        .unwrap();

        let mut te = mt.iterator().unwrap();
        let collected: Vec<BytesRef> = std::iter::from_fn(|| te.next().unwrap()).collect();
        let texts: Vec<&[u8]> = collected.iter().map(|b| b.slice()).collect();
        assert_eq!(
            texts,
            vec![&b"apple"[..], &b"banana"[..], &b"cherry"[..], &b"mango"[..]],
            "terms should be sorted and deduped"
        );

        // Seek to cherry and check aggregated stats.
        assert!(te.seek_exact(&term(b"cherry")).unwrap());
        assert_eq!(te.doc_freq().unwrap(), 3); // 1 + 2
        assert_eq!(te.total_term_freq().unwrap(), 4); // 1 + 3

        // Seek to mango.
        assert!(te.seek_exact(&term(b"mango")).unwrap());
        assert_eq!(te.doc_freq().unwrap(), 2); // 1 + 1
        assert_eq!(te.total_term_freq().unwrap(), 3); // 1 + 2

        // A term present in only one leaf.
        assert!(te.seek_exact(&term(b"apple")).unwrap());
        assert_eq!(te.doc_freq().unwrap(), 1);
        assert_eq!(te.total_term_freq().unwrap(), 1);

        // A term present in none.
        assert!(!te.seek_exact(&term(b"zzzz")).unwrap());
    }

    #[test]
    fn multi_terms_enum_postings_offset_by_doc_base() {
        // Leaf 0 (docBase 0): cherry -> docs [0, 2]
        let t0 = VecTerms::new(
            vec![(term(b"cherry"), 2, 2, vec![0, 2])],
            true,
            false,
            false,
            false,
        );
        // Leaf 1 (docBase 3): cherry -> docs [1, 2]  (global 4, 5)
        let t1 = VecTerms::new(
            vec![(term(b"cherry"), 2, 2, vec![1, 2])],
            true,
            false,
            false,
            false,
        );
        let mt = MultiTerms::new(
            vec![Box::new(t0), Box::new(t1)],
            vec![ReaderSlice::new(0, 3, 0), ReaderSlice::new(3, 3, 1)],
        )
        .unwrap();
        let mut te = mt.iterator().unwrap();
        assert!(te.seek_exact(&term(b"cherry")).unwrap());
        assert_eq!(te.doc_freq().unwrap(), 4);
        let mut pe = te.postings(None, POSTINGS_ENUM_FREQS).unwrap();
        let docs: Vec<i32> = std::iter::from_fn(|| match pe.next_doc().unwrap() {
            crate::search::NO_MORE_DOCS => None,
            d => Some(d),
        })
        .collect();
        // Locals [0,2] from leaf 0 and [1,2] from leaf 1 -> globals [0,2,4,5].
        assert_eq!(docs, vec![0, 2, 4, 5]);
    }

    #[test]
    fn multi_terms_enum_advance_across_slice_boundary() {
        let t0 = VecTerms::new(
            vec![(term(b"x"), 1, 1, vec![1])], // global 1 (slice start 0)
            true,
            false,
            false,
            false,
        );
        let t1 = VecTerms::new(
            vec![(term(b"x"), 1, 1, vec![1])], // global 3 (slice start 2)
            true,
            false,
            false,
            false,
        );
        let mt = MultiTerms::new(
            vec![Box::new(t0), Box::new(t1)],
            vec![ReaderSlice::new(0, 2, 0), ReaderSlice::new(2, 2, 1)],
        )
        .unwrap();
        let mut te = mt.iterator().unwrap();
        assert!(te.seek_exact(&term(b"x")).unwrap());
        let mut pe = te.postings(None, POSTINGS_ENUM_FREQS).unwrap();
        // advance(2) skips leaf 0's doc 1 and lands on leaf 1's doc 1 (global 3).
        assert_eq!(pe.advance(2).unwrap(), 3);
        assert_eq!(pe.next_doc().unwrap(), crate::search::NO_MORE_DOCS);
    }

    #[test]
    fn multi_terms_enum_disjoint_term_sets() {
        let t0 = VecTerms::new(
            vec![(term(b"aaa"), 1, 1, vec![0])],
            true,
            false,
            false,
            false,
        );
        let t1 = VecTerms::new(
            vec![(term(b"zzz"), 1, 1, vec![0])],
            true,
            false,
            false,
            false,
        );
        let mt = MultiTerms::new(
            vec![Box::new(t0), Box::new(t1)],
            vec![ReaderSlice::new(0, 1, 0), ReaderSlice::new(1, 1, 1)],
        )
        .unwrap();
        let mut te = mt.iterator().unwrap();
        let collected: Vec<BytesRef> = std::iter::from_fn(|| te.next().unwrap()).collect();
        let texts: Vec<&[u8]> = collected.iter().map(|b| b.slice()).collect();
        assert_eq!(texts, vec![&b"aaa"[..], &b"zzz"[..]]);
    }

    #[test]
    fn multi_terms_enum_identical_term_in_all_leaves() {
        let t0 = VecTerms::new(
            vec![(term(b"sameterm"), 3, 5, vec![0, 1, 2])],
            true,
            false,
            false,
            false,
        );
        let t1 = VecTerms::new(
            vec![(term(b"sameterm"), 2, 4, vec![0, 1])],
            true,
            false,
            false,
            false,
        );
        let mt = MultiTerms::new(
            vec![Box::new(t0), Box::new(t1)],
            vec![ReaderSlice::new(0, 3, 0), ReaderSlice::new(3, 2, 1)],
        )
        .unwrap();
        let mut te = mt.iterator().unwrap();
        assert_eq!(te.next().unwrap().unwrap().slice(), b"sameterm");
        assert_eq!(te.doc_freq().unwrap(), 5);
        assert_eq!(te.total_term_freq().unwrap(), 9);
        assert!(te.next().unwrap().is_none());
    }

    #[test]
    fn multi_terms_enum_seek_ceil_returns_not_found_for_gap_term() {
        let t0 = VecTerms::new(
            vec![(term(b"apple"), 1, 1, vec![0])],
            true,
            false,
            false,
            false,
        );
        let t1 = VecTerms::new(
            vec![(term(b"mango"), 1, 1, vec![0])],
            true,
            false,
            false,
            false,
        );
        let mt = MultiTerms::new(
            vec![Box::new(t0), Box::new(t1)],
            vec![ReaderSlice::new(0, 1, 0), ReaderSlice::new(1, 1, 1)],
        )
        .unwrap();
        let mut te = mt.iterator().unwrap();
        // "banana" is between apple and mango -> NOT_FOUND, positioned at mango.
        assert_eq!(
            te.seek_ceil(&term(b"banana")).unwrap(),
            SeekStatus::NOT_FOUND
        );
        assert_eq!(te.term().unwrap().slice(), b"mango");

        // Seek past everything -> END.
        assert_eq!(te.seek_ceil(&term(b"zzz")).unwrap(), SeekStatus::END);
    }

    #[test]
    fn multi_terms_enum_seek_exact_then_next_continues_at_following_term() {
        let t0 = VecTerms::new(
            vec![
                (term(b"apple"), 1, 1, vec![0]),
                (term(b"cherry"), 1, 1, vec![1]),
            ],
            true,
            false,
            false,
            false,
        );
        let t1 = VecTerms::new(
            vec![
                (term(b"banana"), 1, 1, vec![0]),
                (term(b"cherry"), 1, 1, vec![1]),
            ],
            true,
            false,
            false,
            false,
        );
        let mt = MultiTerms::new(
            vec![Box::new(t0), Box::new(t1)],
            vec![ReaderSlice::new(0, 2, 0), ReaderSlice::new(2, 2, 1)],
        )
        .unwrap();
        let mut te = mt.iterator().unwrap();
        // Advance to the first term first, so seekExact's optimisation paths
        // have a valid "current" per sub.
        let _ = te.next().unwrap(); // apple
        assert!(te.seek_exact(&term(b"cherry")).unwrap());
        assert_eq!(te.term().unwrap().slice(), b"cherry");
        assert_eq!(te.doc_freq().unwrap(), 2);
        // After seek_exact, next() should re-seek and advance past cherry.
        assert!(te.next().unwrap().is_none(), "cherry was the last term");
    }

    #[test]
    fn multi_terms_enum_empty_sub_terms_returns_empty_enum() {
        let mt = MultiTerms::new(
            vec![Box::new(EmptyTerms::new()), Box::new(EmptyTerms::new())],
            vec![ReaderSlice::new(0, 1, 0), ReaderSlice::new(1, 1, 1)],
        )
        .unwrap();
        let mut te = mt.iterator().unwrap();
        assert!(te.next().unwrap().is_none());
    }

    #[test]
    fn multi_terms_has_flags_aggregated_with_lucene_semantics() {
        // hasFreqs: both true -> true; hasOffsets: one false -> false;
        // hasPositions: both true -> true; hasPayloads: one true, and positions
        // present -> true.
        let t0 = VecTerms::new(vec![], true, false, true, false);
        let t1 = VecTerms::new(vec![], true, true, true, true);
        let mt = MultiTerms::new(
            vec![Box::new(t0), Box::new(t1)],
            vec![ReaderSlice::new(0, 1, 0), ReaderSlice::new(1, 1, 1)],
        )
        .unwrap();
        assert!(mt.has_freqs());
        assert!(!mt.has_offsets());
        assert!(mt.has_positions());
        assert!(mt.has_payloads()); // positions present and one sub has payloads

        // If positions are absent, payloads are forced false even if a sub
        // claims to have them.
        let t0 = VecTerms::new(vec![], true, false, false, false);
        let t1 = VecTerms::new(vec![], true, false, false, true);
        let mt = MultiTerms::new(
            vec![Box::new(t0), Box::new(t1)],
            vec![ReaderSlice::new(0, 1, 0), ReaderSlice::new(1, 1, 1)],
        )
        .unwrap();
        assert!(!mt.has_positions());
        assert!(!mt.has_payloads());
    }

    #[test]
    fn multi_terms_statistics_sum_across_subs() {
        let t0 = VecTerms::new(
            vec![(term(b"a"), 2, 3, vec![0, 1]), (term(b"b"), 1, 1, vec![2])],
            true,
            false,
            false,
            false,
        );
        let t1 = VecTerms::new(
            vec![
                (term(b"a"), 1, 2, vec![0]),
                (term(b"c"), 3, 5, vec![1, 2, 3]),
            ],
            true,
            false,
            false,
            false,
        );
        let mt = MultiTerms::new(
            vec![Box::new(t0), Box::new(t1)],
            vec![ReaderSlice::new(0, 3, 0), ReaderSlice::new(3, 4, 1)],
        )
        .unwrap();
        // sumTotalTermFreq = (3+1) + (2+5) = 11
        assert_eq!(mt.sum_total_term_freq(), 11);
        // sumDocFreq = (2+1) + (1+3) = 7
        assert_eq!(mt.sum_doc_freq(), 7);
    }

    #[test]
    fn multi_fields_get_fields_over_a_real_multi_reader() {
        // Build a MultiReader with two stub leaves. The stubs have no terms,
        // so MultiFields over them exposes no fields and terms() returns None.
        let a = leaf(2, 2);
        let b = leaf(3, 3);
        let mr: Arc<dyn IndexReader> = Arc::new(MultiReader::new(vec![a, b], true).unwrap());
        let mf = MultiFields::get_fields(&mr).unwrap();
        assert_eq!(mf.get_sub_fields().len(), 2);
        assert_eq!(mf.get_sub_slices()[0].start, 0);
        assert_eq!(mf.get_sub_slices()[1].start, 2);
        // No fields in the stubs:
        assert_eq!(mf.iterator().count(), 0);
        assert!(mf.terms("whatever").unwrap().is_none());
    }

    // ----- stub leaf reader reused for get_fields integration -----

    use crate::index::index_reader::{CacheHelper, IndexReaderCore, StoredFields};
    use crate::index::leaf_reader::{LeafMetaData, LeafReader, TermVectors};
    use crate::index::{
        BinaryDocValues, ByteVectorValues, DocValuesSkipper, FieldInfos, FloatVectorValues,
        NumericDocValues, PointValues, SortedDocValues, SortedNumericDocValues, SortedSetDocValues,
    };
    use crate::search::knn::KnnCollector;
    use crate::search::AcceptDocs;
    use crate::util::Bits;

    #[derive(Debug)]
    struct StubLeaf {
        core: IndexReaderCore,
        max_doc: i32,
        num_docs: i32,
    }

    impl StubLeaf {
        fn new(max_doc: i32, num_docs: i32) -> Self {
            Self {
                core: IndexReaderCore::new(),
                max_doc,
                num_docs,
            }
        }
    }

    impl LeafReader for StubLeaf {
        fn core(&self) -> &IndexReaderCore {
            &self.core
        }
        fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
            Ok(Box::new(crate::index::EmptyTermVectors))
        }
        fn num_docs(&self) -> i32 {
            self.num_docs
        }
        fn max_doc(&self) -> i32 {
            self.max_doc
        }
        fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
            Err(LuceneError::UnsupportedOperation("stub".to_string()))
        }
        fn do_close(&self) -> Result<()> {
            Ok(())
        }
        fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
            None
        }
        fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
            None
        }
        fn terms(&self, _field: &str) -> Result<Option<Box<dyn Terms>>> {
            Ok(None)
        }
        fn get_numeric_doc_values(&self, _: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
            Ok(None)
        }
        fn get_binary_doc_values(&self, _: &str) -> Result<Option<Box<dyn BinaryDocValues>>> {
            Ok(None)
        }
        fn get_sorted_doc_values(&self, _: &str) -> Result<Option<Box<dyn SortedDocValues>>> {
            Ok(None)
        }
        fn get_sorted_numeric_doc_values(
            &self,
            _: &str,
        ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
            Ok(None)
        }
        fn get_sorted_set_doc_values(
            &self,
            _: &str,
        ) -> Result<Option<Box<dyn SortedSetDocValues>>> {
            Ok(None)
        }
        fn get_norm_values(&self, _: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
            Ok(None)
        }
        fn get_doc_values_skipper(&self, _: &str) -> Result<Option<Box<dyn DocValuesSkipper>>> {
            Ok(None)
        }
        fn get_float_vector_values(&self, _: &str) -> Result<Option<Box<dyn FloatVectorValues>>> {
            Ok(None)
        }
        fn get_byte_vector_values(&self, _: &str) -> Result<Option<Box<dyn ByteVectorValues>>> {
            Ok(None)
        }
        fn search_nearest_vectors(
            &self,
            _: &str,
            _: &[f32],
            _: &mut dyn KnnCollector,
            _: &mut dyn AcceptDocs,
        ) -> Result<()> {
            Ok(())
        }
        fn search_nearest_vectors_byte(
            &self,
            _: &str,
            _: &[u8],
            _: &mut dyn KnnCollector,
            _: &mut dyn AcceptDocs,
        ) -> Result<()> {
            Ok(())
        }
        fn get_field_infos(&self) -> FieldInfos {
            FieldInfos::empty()
        }
        fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
            None
        }
        fn get_point_values(&self, _: &str) -> Result<Option<Box<dyn PointValues>>> {
            Ok(None)
        }
        fn check_integrity(&self) -> Result<()> {
            Ok(())
        }
        fn get_meta_data(&self) -> LeafMetaData {
            LeafMetaData::new(10, None, None, false).unwrap()
        }
    }

    fn leaf(max_doc: i32, num_docs: i32) -> Arc<dyn IndexReader> {
        Arc::new(StubLeaf::new(max_doc, num_docs)) as Arc<dyn IndexReader>
    }

    #[test]
    fn empty_fields_round_trip() {
        let mf = MultiFields::new(vec![Box::new(EmptyFields)], vec![ReaderSlice::new(0, 0, 0)]);
        assert_eq!(mf.iterator().count(), 0);
        assert!(mf.terms("f").unwrap().is_none());
    }
}
