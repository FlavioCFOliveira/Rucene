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
//!   re-seeked. The optimisation is armed only by `seek_ceil`, which leaves
//!   every sub positioned; `seek_exact` and `next()` disarm it, because a sub
//!   whose exact seek missed is left unpositioned and must be re-seeked from
//!   scratch next time. See `MultiTermsEnum::reseed`.
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
use std::collections::{BTreeSet, BinaryHeap, HashMap};
use std::sync::{Arc, RwLock};

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
/// Lucene has no counterpart to this type: there, the `Fields` instances that
/// `MultiFields` aggregates come straight from the codec's `FieldsProducer`
/// (`CodecReader.getPostingsReader()`), and `LeafReader.fields()` was removed
/// long before 10.5.0. Rucene's `LeafReader` trait exposes `terms(field)` and
/// `get_field_infos()` instead of a `Fields`, so this adapter reconstructs the
/// same view: the per-leaf input that [`MultiFields`] merges across a composite
/// reader's leaves.
///
/// # Only fields with a terms dictionary
///
/// A `FieldsProducer` lists exactly the fields that were inverted. `FieldInfos`
/// is wider: it also describes doc-values-only, points-only and vectors-only
/// fields, which have no terms dictionary at all (`SegmentReader::terms`
/// returns `None` for them, matching `IndexOptions.NONE`). Enumerating those
/// here would break more than iteration order: `PerFieldPostingsFormat` builds
/// its format-to-field grouping from the enumerated names and stamps
/// `PerFieldPostingsFormat.format` / `.suffix` attributes onto each
/// `FieldInfo` it sees, so a doc-values-only field would end up declaring a
/// postings format in the `.fnm` file and the segment would no longer be
/// byte-compatible with Lucene 10.5.0. [`Self::iterator`] therefore filters on
/// `index_options != IndexOptions::NONE`, and [`Self::size`] counts the same
/// set.
struct LeafFields {
    reader: Arc<dyn LeafReader>,
}

impl LeafFields {
    fn new(reader: Arc<dyn LeafReader>) -> Self {
        Self { reader }
    }

    /// Names of the leaf's fields that have a terms dictionary.
    fn indexed_field_names(&self) -> Vec<String> {
        // FieldInfos is returned by value; collect the names into an owned Vec
        // so the returned iterator does not borrow from a local. The order
        // follows FieldInfos' iteration order; MultiFields re-sorts the merged
        // stream, so order here is not contractual.
        self.reader
            .get_field_infos()
            .iter()
            .filter(|fi| fi.index_options != crate::index::IndexOptions::NONE)
            .map(|fi| fi.get_name().to_string())
            .collect()
    }
}

impl Fields for LeafFields {
    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        Box::new(self.indexed_field_names().into_iter())
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        self.reader.terms(field)
    }

    fn size(&self) -> i32 {
        self.indexed_field_names().len() as i32
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
/// [`IndexReader::leaves`]) than through
/// this class, which stitches the leaves back into a single view.
pub struct MultiFields {
    subs: Vec<Box<dyn Fields>>,
    sub_slices: Vec<ReaderSlice>,
    /// Per-field [`MultiTerms`] memo, mirroring Lucene's
    /// `Map<String, Terms> terms = new ConcurrentHashMap<>()`
    /// (`MultiFields.java:28`). Building a `MultiTerms` asks *every* sub for
    /// `terms(field)` and re-derives the aggregated feature flags, so a caller
    /// that iterates fields and then re-reads one of them would pay for the
    /// whole fan-out twice.
    ///
    /// Like Java, only hits are memoised: `terms.put` runs solely on the
    /// `subs2.size() != 0` path (`MultiFields.java:60`), which also bounds the
    /// map by the number of fields that actually exist.
    terms_cache: RwLock<HashMap<String, Arc<MultiTerms>>>,
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
        Self {
            subs,
            sub_slices,
            terms_cache: RwLock::new(HashMap::new()),
        }
    }

    /// Builds a `MultiFields` over all the leaves of `reader`.
    ///
    /// This helper has **no counterpart in Lucene 10.5.0**: neither
    /// `MultiFields.getFields(IndexReader)` nor `LeafReader.fields()` exists
    /// there any more. Java code that needs a merged view of one field calls
    /// `MultiTerms.getTerms(IndexReader, String)`, and the only remaining
    /// caller of the `MultiFields` constructor is the segment merger, which
    /// already holds the per-segment `Fields` produced by the codec.
    ///
    /// Rucene still needs the bridge, because its `LeafReader` trait exposes
    /// `terms(field)` rather than a codec-level `Fields`: something has to
    /// adapt a leaf into a [`Fields`] before [`MultiFields`] can merge it.
    /// Each leaf is therefore wrapped in this module's private `LeafFields`
    /// adapter — which enumerates only the fields that have a terms dictionary,
    /// exactly like a codec `FieldsProducer` — and paired with a
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
        Ok(Self {
            subs,
            sub_slices,
            terms_cache: RwLock::new(HashMap::new()),
        })
    }

    /// Returns the sub-`Fields` instances being merged.
    pub fn get_sub_fields(&self) -> &[Box<dyn Fields>] {
        &self.subs
    }

    /// Returns the [`ReaderSlice`]s parallel to the sub-`Fields`.
    pub fn get_sub_slices(&self) -> &[ReaderSlice] {
        &self.sub_slices
    }

    /// Concretely typed counterpart of [`Fields::terms`].
    ///
    /// Returns the [`MultiTerms`] aggregating `field` across every sub that has
    /// it, or `None` when no sub does. [`Fields::terms`] is implemented on top
    /// of this method.
    ///
    /// Java reaches the concrete type with a cast — `(MultiTerms) in.terms(f)`
    /// in `MappedMultiFields.terms` — which Rust trait objects cannot express.
    /// Exposing the concrete return type here is the equivalent, checked at
    /// compile time instead of at runtime.
    ///
    /// Repeated calls for the same field return the *same* instance, as in
    /// Lucene, which memoises in a `ConcurrentHashMap` (`MultiFields.java:44`).
    /// Java can hand back a bare reference to the cached object; Rust needs an
    /// owned handle, so the shared instance is returned as an
    /// [`Arc`] — `Arc<MultiTerms>` itself implements [`Terms`], so callers can
    /// use it wherever a `Terms` is expected.
    pub fn multi_terms(&self, field: &str) -> Result<Option<Arc<MultiTerms>>> {
        // Fast path: an existing memo. A poisoned lock is treated as "no memo"
        // rather than an error — the cache is an optimisation, and rebuilding
        // is always correct.
        if let Ok(cache) = self.terms_cache.read() {
            if let Some(terms) = cache.get(field) {
                return Ok(Some(Arc::clone(terms)));
            }
        }

        let mut subs2: Vec<Box<dyn Terms>> = Vec::new();
        let mut slices2: Vec<ReaderSlice> = Vec::new();
        for (i, sub) in self.subs.iter().enumerate() {
            if let Some(terms) = sub.terms(field)? {
                subs2.push(terms);
                slices2.push(self.sub_slices[i]);
            }
        }
        if subs2.is_empty() {
            // Matches Lucene: misses are not memoised.
            return Ok(None);
        }

        // `Arc` rather than `Rc`, although `Terms` is not currently bound by
        // `Send + Sync`: Lucene's readers are fully thread-safe, and the
        // reference count is the one part of this memo that must not have to
        // change when Rucene's `Terms` catches up. The atomics are the cost of
        // not baking single-threadedness into the public signature.
        #[allow(clippy::arc_with_non_send_sync)]
        let built = Arc::new(MultiTerms::new(subs2, slices2)?);
        match self.terms_cache.write() {
            // Another caller may have raced us here; keep whichever instance
            // landed first so every caller observes one `Terms` per field.
            Ok(mut cache) => Ok(Some(Arc::clone(
                cache.entry(field.to_string()).or_insert(built),
            ))),
            Err(_) => Ok(Some(built)),
        }
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
        Ok(self
            .multi_terms(field)?
            .map(|terms| Box::new(terms) as Box<dyn Terms>))
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
    ///
    /// # `ReaderSlice::length` here is the composite `maxDoc`
    ///
    /// The slices built below use the **composite** reader's `max_doc()` as
    /// `length`, not the leaf's — a faithful transcription of
    /// `new ReaderSlice(ctx.docBase, r.maxDoc(), leafIdx)`
    /// (`MultiTerms.java:82`). That is not the "slice width" reading of the
    /// field, and it differs from [`MultiFields::get_fields`], which sets
    /// `length` to the leaf's own `max_doc()`. See the note on
    /// [`ReaderSlice`] for why both are correct.
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

    /// Returns a [`PostingsEnum`] with **all** optional features requested
    /// (freqs, positions, offsets and payloads) for `term` in `field` across
    /// `reader`, or `None` if the field or term does not exist.
    ///
    /// Equivalent to
    /// `MultiTerms.getTermPostingsEnum(IndexReader, String, BytesRef)`
    /// (`MultiTerms.java:99-102`), the convenience overload that defaults
    /// `flags` to `PostingsEnum.ALL`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while resolving the field's terms or
    /// seeking to `term`.
    pub fn get_term_postings_enum_all(
        reader: &Arc<dyn IndexReader>,
        field: &str,
        term: &BytesRef,
    ) -> Result<Option<Box<dyn PostingsEnum>>> {
        Self::get_term_postings_enum(
            reader,
            field,
            term,
            crate::index::postings_enum::POSTINGS_ENUM_ALL,
        )
    }

    /// Returns a [`PostingsEnum`] for `term` in `field` across `reader`, or
    /// `None` if the field or term does not exist.
    ///
    /// Equivalent to
    /// `MultiTerms.getTermPostingsEnum(IndexReader, String, BytesRef, int)`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while resolving the field's terms or
    /// seeking to `term`.
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

    /// Builds a merged [`MultiTermsEnum`] from the given per-sub `TermsEnum`s.
    ///
    /// Mirrors `MultiTermsEnum.reset(TermsEnumIndex[])`: each sub-enum is
    /// advanced once (its first term seeded into the heap). `None` is returned
    /// when no sub yields any term — the case where Lucene's `reset()` returns
    /// `TermsEnum.EMPTY`.
    fn build_enum(
        sub_slices: &[ReaderSlice],
        entries: Vec<(Box<dyn TermsEnum>, ReaderSlice)>,
    ) -> Result<Option<MultiTermsEnum>> {
        let mut me = MultiTermsEnum::new(sub_slices);
        for (te, slice) in entries {
            me.add_sub(te, slice)?;
        }
        Ok(me.finish())
    }

    /// Concretely typed counterpart of [`Terms::intersect`], returning `None`
    /// when no sub contributed a term.
    ///
    /// Shares its heap-seeding step with [`Self::multi_iterator`]; the only
    /// difference is that each sub is asked for an automaton-filtered enum.
    pub fn multi_intersect(
        &self,
        compiled: &CompiledAutomaton,
        start_term: Option<&BytesRef>,
    ) -> Result<Option<MultiTermsEnum>> {
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

    /// Concretely typed counterpart of [`Terms::iterator`].
    ///
    /// Returns `None` exactly when [`Terms::iterator`] would return an
    /// [`EmptyTermsEnum`], i.e. when no sub contributed a term. Java expresses
    /// the same distinction by comparing the returned enum against the
    /// `TermsEnum.EMPTY` singleton (LUCENE-6826) after a cast to
    /// `MultiTermsEnum`; `Option` makes it type-safe here.
    pub fn multi_iterator(&self) -> Result<Option<MultiTermsEnum>> {
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
}

impl Terms for MultiTerms {
    fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
        Ok(match self.multi_iterator()? {
            Some(me) => Box::new(me) as Box<dyn TermsEnum>,
            None => Box::new(EmptyTermsEnum::new()) as Box<dyn TermsEnum>,
        })
    }

    fn intersect(
        &self,
        compiled: &CompiledAutomaton,
        start_term: Option<&BytesRef>,
    ) -> Result<Box<dyn TermsEnum>> {
        Ok(match self.multi_intersect(compiled, start_term)? {
            Some(me) => Box::new(me) as Box<dyn TermsEnum>,
            None => Box::new(EmptyTermsEnum::new()) as Box<dyn TermsEnum>,
        })
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

    /// Returns the smallest term across every sub.
    ///
    /// **Deliberate divergence from Lucene.** `MultiTerms.getMin`
    /// (`MultiTerms.java:155-169`) dereferences each sub's `getMin()` result
    /// unconditionally (`term.compareTo(minTerm)`), so a sub whose field is
    /// present but empty — `getMin()` returning `null` — makes it throw a
    /// `NullPointerException`. This port skips such subs and folds only the
    /// terms that exist, which is what the surrounding code already assumes.
    ///
    /// The difference is unobservable through the public API: `MultiTerms` is
    /// only ever built from subs that answered `terms(field)` with a non-`None`
    /// value, and a field with a terms dictionary has at least one term. The
    /// divergence exists so that a future caller constructing `MultiTerms`
    /// directly gets a sensible answer instead of a panic.
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

    /// Returns the largest term across every sub.
    ///
    /// Shares [`Self::min`]'s deliberate divergence: Lucene's
    /// `MultiTerms.getMax` throws `NullPointerException` for a sub with no
    /// terms, this port skips it.
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

/// Delegating [`Terms`] implementation for the shared handle returned by
/// [`MultiFields::multi_terms`].
///
/// Lucene's `MultiFields` hands the *same* `MultiTerms` object back on every
/// call for a field. Rust callers need an owned value, so the shared instance
/// travels as an `Arc`; this impl lets that `Arc` be used directly as a
/// `Terms`, keeping `Fields::terms` a one-liner and avoiding a rebuild per
/// call.
impl Terms for Arc<MultiTerms> {
    fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
        (**self).iterator()
    }

    fn intersect(
        &self,
        compiled: &CompiledAutomaton,
        start_term: Option<&BytesRef>,
    ) -> Result<Box<dyn TermsEnum>> {
        (**self).intersect(compiled, start_term)
    }

    fn size(&self) -> i64 {
        (**self).size()
    }

    fn sum_total_term_freq(&self) -> i64 {
        (**self).sum_total_term_freq()
    }

    fn sum_doc_freq(&self) -> i64 {
        (**self).sum_doc_freq()
    }

    fn doc_count(&self) -> i32 {
        (**self).doc_count()
    }

    fn has_freqs(&self) -> bool {
        (**self).has_freqs()
    }

    fn has_offsets(&self) -> bool {
        (**self).has_offsets()
    }

    fn has_positions(&self) -> bool {
        (**self).has_positions()
    }

    fn has_payloads(&self) -> bool {
        (**self).has_payloads()
    }

    fn min(&self) -> Result<Option<BytesRef>> {
        (**self).min()
    }

    fn max(&self) -> Result<Option<BytesRef>> {
        (**self).max()
    }

    fn stats(&self) -> String {
        (**self).stats()
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

    /// Finalises construction. Returns `None` if no sub yielded a term —
    /// matching Lucene's `reset()` returning `TermsEnum.EMPTY` when the queue
    /// is empty.
    fn finish(self) -> Option<Self> {
        if self.heap.is_empty() {
            None
        } else {
            Some(self)
        }
    }

    /// Concretely typed counterpart of [`TermsEnum::postings`].
    ///
    /// Builds the [`MultiPostingsEnum`] over every sub currently positioned at
    /// the enum's term. Java reaches the same object through a cast
    /// (`(MultiPostingsEnum) in.postings(...)` in `MappedMultiFields`); callers
    /// that need the per-sub [`ReaderSlice`]s — segment merging above all —
    /// use this method instead.
    pub fn multi_postings(&mut self, flags: i32) -> Result<MultiPostingsEnum> {
        // Sort the top subs by reader_index so the MultiPostingsEnum receives
        // slices in ascending doc-ID order (Lucene does the same via
        // ArrayUtil.timSort on subIndex).
        let mut top_sorted: Vec<usize> = self.top.clone();
        top_sorted.sort_by_key(|&idx| self.subs[idx].slice.reader_index);

        let mut sub_docs: Vec<EnumWithSlice> = Vec::with_capacity(top_sorted.len());
        for &idx in &top_sorted {
            // Sub-enum reuse across calls is not tracked here; each call asks
            // the sub for a fresh postings enum. This matches Lucene's behaviour
            // (which passes the previous sub-enum as reuse) when the sub
            // implementations do not special-case reuse, and is correct.
            let pe = self.subs[idx].tenum.postings(None, flags)?;
            sub_docs.push(EnumWithSlice::new(pe, self.subs[idx].slice));
        }
        Ok(MultiPostingsEnum::new(sub_docs))
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
    /// Shared by `seek_exact` (`exact = true`) and `seek_ceil`
    /// (`exact = false`). Both compute `seek_opt` from the *previous*
    /// `last_seek` before calling; this method then records the `last_seek`
    /// that the *next* seek will consult.
    ///
    /// # Why an exact seek forgets `last_seek`
    ///
    /// A `seek_ceil` leaves every sub positioned: on `NOT_FOUND` the sub sits
    /// on its ceiling term, and on `END` it is exhausted for every term from
    /// here on. Recording `last_seek` is therefore safe, and lets the next
    /// non-decreasing seek skip subs that are already past the new term.
    ///
    /// A `seek_exact` does not: a sub whose `seek_exact` returned `false` is
    /// left *unpositioned* (`TermsEnum::seek_exact` promises nothing about
    /// where the underlying enum stopped, so `current` is cleared), and the
    /// sub may well hold terms after the one that was sought. Keeping
    /// `last_seek` would let the next non-decreasing seek take the
    /// "no recorded term ⇒ nothing to find here" branch and drop that sub
    /// permanently. Clearing it forces the next seek to re-seek every sub from
    /// scratch. This mirrors `MultiTermsEnum.seekExact`, which assigns
    /// `lastSeek = null` (`MultiTermsEnum.java:124`) exactly where
    /// `seekCeil` assigns `lastSeek = term` (`MultiTermsEnum.java:177`).
    ///
    /// With `last_seek` cleared, the `seek_opt` branch for a sub with no
    /// recorded term is reachable only after a `seek_ceil` that returned
    /// `SeekStatus::END` for it — for which skipping is correct, because no
    /// term at or after the previous seek term exists in that sub.
    fn reseed(&mut self, term: &BytesRef, seek_opt: bool, exact: bool) -> Result<()> {
        self.heap.clear();
        self.top.clear();
        self.last_seek_exact = exact;
        self.last_seek = if exact { None } else { Some(term.clone()) };

        for i in 0..self.subs.len() {
            if seek_opt {
                // LUCENE-2130: if this sub's current term is already past
                // `term`, don't re-seek it. Compare against the live current.
                if let Some(cur) = &self.subs[i].current {
                    match term.cmp(cur) {
                        Ordering::Equal => {
                            // The sub already sits on the term: a hit for
                            // seek_exact and a FOUND for seek_ceil alike.
                            self.top.push(i);
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
                    // No recorded term. Because `last_seek` is cleared by
                    // `seek_exact` and by `next()`, `seek_opt` can only be set
                    // by a preceding `seek_ceil` — so the sub reached
                    // `SeekStatus::END` there, and `term` is not before that
                    // seek term. Nothing at or after `term` exists in this sub;
                    // skipping it is correct. (Reaching here after a *failed*
                    // seek_exact would instead drop a sub that may still hold
                    // later terms: see this function's doc comment.)
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

    /// Positions the enum on `text` if any sub holds it.
    ///
    /// **Deliberate divergence from Lucene: the state after a miss.** Java
    /// leaves `current` pointing at whatever term the enum was on before
    /// (`MultiTermsEnum.java:154-158` only assigns `current` inside the
    /// `status == true` branch), so a `false` return leaves a stale term
    /// readable through `term()`. This port clears `current`, which is what
    /// [`TermsEnum::seek_exact`]'s own contract requires: *"if this returns
    /// false, the enum is unpositioned"*.
    ///
    /// One consequence is visible in [`Self::next`]: Java's `next()` after a
    /// failed `seek_exact` re-seeks to the stale term and asserts the result is
    /// `FOUND` — an assertion that only holds because the stale term happens to
    /// exist. Here there is no term to re-seek to, so `next()` resumes from the
    /// merge heap instead. Both behaviours are outside the `TermsEnum`
    /// contract, which does not define `next()` on an unpositioned enum; this
    /// one at least cannot read a term the caller never asked for.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while seeking a sub-enum.
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

    /// Positions the enum on `text`, or on the smallest term after it.
    ///
    /// **Deliberate divergence from Lucene: the state after `END`.** Java
    /// leaves `current` untouched when every sub reports `END`
    /// (`MultiTermsEnum.java:229-231` returns `SeekStatus.END` without
    /// clearing it), so `term()` keeps reporting the previous term. This port
    /// clears `current`, matching [`TermsEnum::seek_ceil`]'s contract that
    /// `END` means the enum is exhausted.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while seeking a sub-enum.
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
        Ok(Box::new(self.multi_postings(flags)?))
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

/// A [`PostingsEnum`] paired with the [`ReaderSlice`] describing how its
/// sub-reader fits into the composite reader.
///
/// Equivalent to `MultiPostingsEnum.EnumWithSlice`.
pub struct EnumWithSlice {
    /// Postings enum for this sub-reader, in the sub-reader's local doc-ID
    /// space.
    pub postings_enum: Box<dyn PostingsEnum>,
    /// Placement of this sub-reader inside the composite reader.
    pub slice: ReaderSlice,
}

impl EnumWithSlice {
    /// Pairs `postings_enum` with `slice`.
    pub fn new(postings_enum: Box<dyn PostingsEnum>, slice: ReaderSlice) -> Self {
        Self {
            postings_enum,
            slice,
        }
    }
}

impl std::fmt::Debug for EnumWithSlice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Matches `EnumWithSlice.toString()`, which prints only the slice; the
        // postings enum itself has no Debug bound in Rucene.
        f.debug_struct("EnumWithSlice")
            .field("slice", &self.slice)
            .finish_non_exhaustive()
    }
}

/// A [`PostingsEnum`] that concatenates several sub-readers' postings, offset
/// by each sub's slice `start`, producing a globally sorted doc-ID stream.
///
/// Equivalent to `org.apache.lucene.index.MultiPostingsEnum`. The slices
/// partition the global doc-ID space (each sub's local doc IDs are re-based by
/// `slice.start`), so a plain concatenation yields the same globally increasing
/// doc-ID sequence a merge would — without a per-doc comparison.
///
/// **Deliberate divergence**: Java pre-allocates one `EnumWithSlice` per
/// sub-reader and tracks how many are live in `numSubs`, so the instance can be
/// reset onto a new term without reallocating. Rucene builds a right-sized
/// `Vec` per term instead: [`Self::get_num_subs`] is simply its length. Reuse
/// buys nothing here because [`MultiTermsEnum`] does not thread `reuse` through
/// to its sub-enums either.
pub struct MultiPostingsEnum {
    /// Live sub-enums with their slices, in ascending slice order.
    subs: Vec<EnumWithSlice>,
    /// Index into `subs` of the current sub, or `-1` before the first
    /// `next_doc`/`advance`.
    upto: i32,
    /// Current global doc ID (`-1` before the first positioning).
    doc: i32,
}

impl MultiPostingsEnum {
    /// Builds a merged postings enum over `subs`, which must be ordered by
    /// ascending [`ReaderSlice::start`].
    pub fn new(subs: Vec<EnumWithSlice>) -> Self {
        Self {
            subs,
            upto: -1,
            doc: -1,
        }
    }

    /// Returns how many sub-readers are being merged.
    ///
    /// Equivalent to `MultiPostingsEnum.getNumSubs()`.
    pub fn get_num_subs(&self) -> usize {
        self.subs.len()
    }

    /// Returns the sub-readers being merged.
    ///
    /// Equivalent to `MultiPostingsEnum.getSubs()`.
    pub fn get_subs(&self) -> &[EnumWithSlice] {
        &self.subs
    }

    /// Consumes this enum and returns its sub-readers, so a caller can take
    /// ownership of the per-sub postings.
    ///
    /// Java hands out the live `EnumWithSlice[]` from `getSubs()` and lets the
    /// caller keep the references; Rust ownership makes that transfer explicit.
    pub fn into_subs(self) -> Vec<EnumWithSlice> {
        self.subs
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
                let d = self.subs[i as usize].postings_enum.next_doc()?;
                if d == crate::search::NO_MORE_DOCS {
                    // Exhausted current sub; advance to the next.
                    self.upto += 1;
                } else {
                    self.doc = d + self.subs[i as usize].slice.start;
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
                let base = self.subs[i as usize].slice.start;
                // target < base: target was in a previous slice that had no
                // matching doc after it — just next_doc the current sub.
                let d = if target < base {
                    self.subs[i as usize].postings_enum.next_doc()?
                } else {
                    self.subs[i as usize].postings_enum.advance(target - base)?
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
        for sub in &self.subs {
            cost += sub.postings_enum.cost();
        }
        cost
    }
}

impl PostingsEnum for MultiPostingsEnum {
    fn freq(&self) -> Result<i32> {
        self.current_idx()
            .and_then(|i| self.subs[i].postings_enum.freq())
    }

    fn next_position(&mut self) -> Result<i32> {
        let i = self.current_idx()?;
        self.subs[i].postings_enum.next_position()
    }

    fn start_offset(&self) -> i32 {
        match self.current_idx() {
            Ok(i) => self.subs[i].postings_enum.start_offset(),
            Err(_) => -1,
        }
    }

    fn end_offset(&self) -> i32 {
        match self.current_idx() {
            Ok(i) => self.subs[i].postings_enum.end_offset(),
            Err(_) => -1,
        }
    }

    fn get_payload(&self) -> Result<Option<&[u8]>> {
        let i = self.current_idx()?;
        self.subs[i].postings_enum.get_payload()
    }
}

// ---------------------------------------------------------------------------
// SlowImpactsEnum — used by MultiTermsEnum::impacts.
// ---------------------------------------------------------------------------

/// `ImpactsEnum` that wraps a [`PostingsEnum`] and reports a single trivial
/// impact level, matching `org.apache.lucene.index.SlowImpactsEnum`.
///
/// Use it whenever an [`ImpactsEnum`] is required but no impacts are indexed:
/// it reports one level covering the whole postings list, with the maximum
/// possible frequency and the minimum possible norm, which is always a valid
/// (if useless) upper bound.
pub struct SlowImpactsEnum {
    delegate: Box<dyn PostingsEnum>,
}

impl SlowImpactsEnum {
    /// Wraps `delegate`.
    ///
    /// Equivalent to `SlowImpactsEnum(PostingsEnum)`.
    pub fn new(delegate: Box<dyn PostingsEnum>) -> Self {
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
    #[derive(Clone, Debug)]
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
        field_infos: FieldInfos,
        /// Fields that actually have a terms dictionary, keyed by name.
        terms: std::collections::BTreeMap<String, VecTerms>,
    }

    impl StubLeaf {
        fn new(max_doc: i32, num_docs: i32) -> Self {
            Self {
                core: IndexReaderCore::new(),
                max_doc,
                num_docs,
                field_infos: FieldInfos::empty(),
                terms: std::collections::BTreeMap::new(),
            }
        }

        /// Builds a leaf whose `FieldInfos` may describe more fields than have
        /// a terms dictionary — the doc-values-only / points-only / vectors-only
        /// case that `LeafFields` must not enumerate.
        fn with_fields(
            max_doc: i32,
            num_docs: i32,
            field_infos: FieldInfos,
            terms: Vec<(&str, VecTerms)>,
        ) -> Self {
            Self {
                core: IndexReaderCore::new(),
                max_doc,
                num_docs,
                field_infos,
                terms: terms
                    .into_iter()
                    .map(|(name, t)| (name.to_string(), t))
                    .collect(),
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
        fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
            // Mirrors SegmentReader::terms: a field with IndexOptions::NONE has
            // no terms dictionary at all.
            Ok(self
                .terms
                .get(field)
                .map(|t| Box::new(t.clone()) as Box<dyn Terms>))
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
            self.field_infos.clone()
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

    // ----- concretely typed accessors used by segment merging -----

    #[test]
    fn multi_terms_returns_the_concrete_type_for_a_field_some_sub_has() {
        let f1 = VecFields::new([(
            "body".to_string(),
            VecTerms::new(vec![(term(b"a"), 1, 1, vec![0])], true, false, false, false),
        )]);
        let mf = MultiFields::new(vec![Box::new(f1)], vec![ReaderSlice::new(0, 5, 0)]);
        let terms = mf.multi_terms("body").unwrap().expect("field exists");
        assert_eq!(terms.get_sub_terms().len(), 1);
        assert!(mf.multi_terms("missing").unwrap().is_none());
    }

    #[test]
    fn multi_iterator_is_none_when_no_sub_contributes_a_term() {
        // LUCENE-6826: the field exists in both subs but neither holds a term.
        let f1 = VecFields::new([(
            "body".to_string(),
            VecTerms::new(vec![], true, false, false, false),
        )]);
        let f2 = VecFields::new([(
            "body".to_string(),
            VecTerms::new(vec![], true, false, false, false),
        )]);
        let mf = MultiFields::new(
            vec![Box::new(f1), Box::new(f2)],
            vec![ReaderSlice::new(0, 5, 0), ReaderSlice::new(5, 5, 1)],
        );
        let terms = mf.multi_terms("body").unwrap().unwrap();
        assert!(terms.multi_iterator().unwrap().is_none());
        // The trait method turns that into the empty enum.
        let mut boxed = terms.iterator().unwrap();
        assert!(boxed.next().unwrap().is_none());
    }

    #[test]
    fn multi_postings_expose_one_sub_per_reader_holding_the_term() {
        let f1 = VecFields::new([(
            "body".to_string(),
            VecTerms::new(
                vec![(term(b"a"), 1, 1, vec![0]), (term(b"b"), 1, 1, vec![1])],
                true,
                false,
                false,
                false,
            ),
        )]);
        let f2 = VecFields::new([(
            "body".to_string(),
            VecTerms::new(vec![(term(b"a"), 1, 1, vec![2])], true, false, false, false),
        )]);
        let mf = MultiFields::new(
            vec![Box::new(f1), Box::new(f2)],
            vec![ReaderSlice::new(0, 5, 0), ReaderSlice::new(5, 5, 1)],
        );
        let terms = mf.multi_terms("body").unwrap().unwrap();
        let mut enumerator = terms.multi_iterator().unwrap().unwrap();

        assert_eq!(enumerator.next().unwrap(), Some(term(b"a")));
        let postings = enumerator.multi_postings(POSTINGS_ENUM_FREQS).unwrap();
        assert_eq!(postings.get_num_subs(), 2, "both subs hold 'a'");
        let reader_indexes: Vec<i32> = postings
            .get_subs()
            .iter()
            .map(|sub| sub.slice.reader_index)
            .collect();
        assert_eq!(
            reader_indexes,
            vec![0, 1],
            "subs are ordered by reader index"
        );
        assert_eq!(postings.into_subs().len(), 2);

        assert_eq!(enumerator.next().unwrap(), Some(term(b"b")));
        let postings = enumerator.multi_postings(POSTINGS_ENUM_FREQS).unwrap();
        assert_eq!(postings.get_num_subs(), 1, "only the first sub holds 'b'");
        assert_eq!(postings.get_subs()[0].slice.reader_index, 0);
    }

    #[test]
    fn multi_postings_still_offset_local_doc_ids_by_the_slice_start() {
        let f1 = VecFields::new([(
            "body".to_string(),
            VecTerms::new(vec![(term(b"a"), 1, 1, vec![0])], true, false, false, false),
        )]);
        let f2 = VecFields::new([(
            "body".to_string(),
            VecTerms::new(vec![(term(b"a"), 1, 1, vec![2])], true, false, false, false),
        )]);
        let mf = MultiFields::new(
            vec![Box::new(f1), Box::new(f2)],
            vec![ReaderSlice::new(0, 5, 0), ReaderSlice::new(5, 5, 1)],
        );
        let terms = mf.multi_terms("body").unwrap().unwrap();
        let mut enumerator = terms.multi_iterator().unwrap().unwrap();
        assert_eq!(enumerator.next().unwrap(), Some(term(b"a")));
        let mut postings = enumerator.multi_postings(POSTINGS_ENUM_FREQS).unwrap();
        assert_eq!(postings.next_doc().unwrap(), 0);
        assert_eq!(postings.next_doc().unwrap(), 7);
        assert_eq!(postings.next_doc().unwrap(), crate::search::NO_MORE_DOCS);
    }

    // ----- LUCENE-2130 seek-optimisation regression tests -----
    //
    // `MultiTermsEnum.seekExact` deliberately clears `lastSeek`
    // (`MultiTermsEnum.java:124`), which disables the LUCENE-2130 skip on the
    // *next* seek. It must, because a sub whose `seekExact` failed is left
    // unpositioned: its recorded current term is dropped, so the skip logic
    // would have no term to compare against and would silently discard the
    // sub for every following non-decreasing seek. These tests pin that
    // behaviour: each one loses a whole leaf if `last_seek` survives a
    // `seek_exact`.

    /// leaf 0 = {a, c}, leaf 1 = {b, c}, with leaf 1 based at global doc 10.
    fn two_leaves_with_a_shared_trailing_term() -> MultiTerms {
        let t0 = VecTerms::new(
            vec![(term(b"a"), 1, 1, vec![0]), (term(b"c"), 1, 1, vec![1])],
            true,
            false,
            false,
            false,
        );
        let t1 = VecTerms::new(
            vec![(term(b"b"), 1, 1, vec![0]), (term(b"c"), 1, 1, vec![1])],
            true,
            false,
            false,
            false,
        );
        MultiTerms::new(
            vec![Box::new(t0), Box::new(t1)],
            vec![ReaderSlice::new(0, 10, 0), ReaderSlice::new(10, 10, 1)],
        )
        .unwrap()
    }

    fn drain_docs(postings: &mut dyn PostingsEnum) -> Vec<i32> {
        let mut docs = Vec::new();
        loop {
            let doc = postings.next_doc().unwrap();
            if doc == crate::search::NO_MORE_DOCS {
                return docs;
            }
            docs.push(doc);
        }
    }

    #[test]
    fn seek_exact_after_a_failed_seek_exact_still_sees_every_leaf() {
        let mt = two_leaves_with_a_shared_trailing_term();
        let mut te = mt.multi_iterator().unwrap().unwrap();
        // "b" exists only in leaf 1, so leaf 0's seek_exact fails and leaves it
        // unpositioned.
        assert!(te.seek_exact(&term(b"b")).unwrap());
        assert_eq!(te.doc_freq().unwrap(), 1);
        // "c" exists in both leaves: leaf 0 must be re-seeked from scratch.
        assert!(te.seek_exact(&term(b"c")).unwrap());
        assert_eq!(
            te.doc_freq().unwrap(),
            2,
            "leaf 0 was dropped by the seek optimisation"
        );
        let mut postings = te.multi_postings(POSTINGS_ENUM_FREQS).unwrap();
        assert_eq!(drain_docs(&mut postings), vec![1, 11]);
    }

    #[test]
    fn seek_ceil_after_a_failed_seek_exact_still_sees_every_leaf() {
        let mt = two_leaves_with_a_shared_trailing_term();
        let mut te = mt.multi_iterator().unwrap().unwrap();
        assert!(te.seek_exact(&term(b"b")).unwrap());
        assert_eq!(te.seek_ceil(&term(b"c")).unwrap(), SeekStatus::FOUND);
        assert_eq!(
            te.doc_freq().unwrap(),
            2,
            "leaf 0 was dropped by the seek optimisation"
        );
    }

    #[test]
    fn next_after_a_failed_seek_exact_still_sees_every_leaf() {
        // leaf 0 = {a, z}, leaf 1 = {b}: "b" is missing from leaf 0, so a naive
        // seek optimisation would drop leaf 0 and hide "z" for ever.
        let t0 = VecTerms::new(
            vec![(term(b"a"), 1, 1, vec![0]), (term(b"z"), 1, 1, vec![1])],
            true,
            false,
            false,
            false,
        );
        let t1 = VecTerms::new(vec![(term(b"b"), 1, 1, vec![0])], true, false, false, false);
        let mt = MultiTerms::new(
            vec![Box::new(t0), Box::new(t1)],
            vec![ReaderSlice::new(0, 10, 0), ReaderSlice::new(10, 10, 1)],
        )
        .unwrap();
        let mut te = mt.multi_iterator().unwrap().unwrap();
        assert!(te.seek_exact(&term(b"b")).unwrap());
        assert_eq!(te.next().unwrap(), Some(term(b"z")));
    }

    #[test]
    fn seek_exact_after_a_successful_seek_exact_still_sees_every_leaf() {
        let mt = two_leaves_with_a_shared_trailing_term();
        let mut te = mt.multi_iterator().unwrap().unwrap();
        // "a" exists only in leaf 0, so leaf 1's seek_exact fails.
        assert!(te.seek_exact(&term(b"a")).unwrap());
        assert!(te.seek_exact(&term(b"c")).unwrap());
        let mut postings = te.multi_postings(POSTINGS_ENUM_FREQS).unwrap();
        assert_eq!(
            drain_docs(&mut postings),
            vec![1, 11],
            "leaf 1 was dropped by the seek optimisation"
        );
    }

    #[test]
    fn seek_exact_backwards_re_seeks_every_leaf() {
        // A decreasing seek term switches the optimisation off explicitly
        // (`last_seek > text`), which is the other way a sub can be recovered.
        let mt = two_leaves_with_a_shared_trailing_term();
        let mut te = mt.multi_iterator().unwrap().unwrap();
        assert!(te.seek_exact(&term(b"c")).unwrap());
        assert_eq!(te.doc_freq().unwrap(), 2);
        assert!(te.seek_exact(&term(b"a")).unwrap());
        assert_eq!(te.doc_freq().unwrap(), 1);
        assert_eq!(te.term().unwrap(), term(b"a"));
        // ... and forwards again, which is the optimised direction.
        assert!(te.seek_exact(&term(b"b")).unwrap());
        assert_eq!(te.doc_freq().unwrap(), 1);
        assert!(te.seek_exact(&term(b"c")).unwrap());
        assert_eq!(te.doc_freq().unwrap(), 2);
    }

    #[test]
    fn seek_ceil_end_then_next_reports_exhaustion() {
        let mt = two_leaves_with_a_shared_trailing_term();
        let mut te = mt.multi_iterator().unwrap().unwrap();
        assert_eq!(te.seek_ceil(&term(b"zzz")).unwrap(), SeekStatus::END);
        assert!(te.next().unwrap().is_none());
        // Still exhausted on a second call.
        assert!(te.next().unwrap().is_none());
    }

    #[test]
    fn multi_postings_terminal_state_is_idempotent() {
        let mt = two_leaves_with_a_shared_trailing_term();
        let mut te = mt.multi_iterator().unwrap().unwrap();
        assert!(te.seek_exact(&term(b"c")).unwrap());
        let mut postings = te.multi_postings(POSTINGS_ENUM_FREQS).unwrap();
        assert_eq!(drain_docs(&mut postings), vec![1, 11]);
        // Exhausted: every further call keeps reporting NO_MORE_DOCS and never
        // rewinds into an earlier sub.
        assert_eq!(postings.doc_id(), crate::search::NO_MORE_DOCS);
        assert_eq!(postings.next_doc().unwrap(), crate::search::NO_MORE_DOCS);
        assert_eq!(postings.doc_id(), crate::search::NO_MORE_DOCS);
        assert_eq!(postings.advance(0).unwrap(), crate::search::NO_MORE_DOCS);
        assert_eq!(postings.advance(11).unwrap(), crate::search::NO_MORE_DOCS);
        assert_eq!(
            postings.advance(crate::search::NO_MORE_DOCS).unwrap(),
            crate::search::NO_MORE_DOCS
        );
        assert_eq!(postings.doc_id(), crate::search::NO_MORE_DOCS);
    }

    // ----- LeafFields must expose only fields with a terms dictionary -----

    /// Builds `FieldInfos` describing one postings field and one
    /// doc-values-only field (`IndexOptions::NONE`).
    fn postings_and_doc_values_only_field_infos() -> FieldInfos {
        use crate::index::field_infos::FieldInfo;
        use crate::index::{DocValuesType, IndexOptions};

        let mut body = FieldInfo::default();
        body.name = "body".to_string();
        body.number = 0;
        body.index_options = IndexOptions::DOCS_AND_FREQS;

        let mut price = FieldInfo::default();
        price.name = "price".to_string();
        price.number = 1;
        // Doc-values only: never inverted, so no terms dictionary exists.
        price.index_options = IndexOptions::NONE;
        price.doc_values_type = DocValuesType::NUMERIC;

        FieldInfos::new(vec![body, price]).unwrap()
    }

    #[test]
    fn leaf_fields_skips_fields_that_have_no_terms_dictionary() {
        let body = VecTerms::new(
            vec![(term(b"apple"), 1, 1, vec![0])],
            true,
            false,
            false,
            false,
        );
        let l: Arc<dyn IndexReader> = Arc::new(StubLeaf::with_fields(
            2,
            2,
            postings_and_doc_values_only_field_infos(),
            vec![("body", body)],
        ));
        let other: Arc<dyn IndexReader> = Arc::new(StubLeaf::new(1, 1));
        let mr: Arc<dyn IndexReader> = Arc::new(MultiReader::new(vec![l, other], true).unwrap());

        let mf = MultiFields::get_fields(&mr).unwrap();
        let names: Vec<String> = mf.iterator().collect();
        assert_eq!(
            names,
            vec!["body".to_string()],
            "a doc-values-only field has no postings and must not be enumerated: \
             PerFieldPostingsFormat would otherwise stamp a postings format onto \
             its FieldInfo in the .fnm file"
        );
        // And the field is still not reachable through terms().
        assert!(mf.terms("price").unwrap().is_none());
        assert!(mf.terms("body").unwrap().is_some());
    }

    #[test]
    fn leaf_fields_size_counts_only_fields_with_a_terms_dictionary() {
        let body = VecTerms::new(
            vec![(term(b"apple"), 1, 1, vec![0])],
            true,
            false,
            false,
            false,
        );
        let leaf_reader: Arc<dyn LeafReader> = Arc::new(StubLeaf::with_fields(
            2,
            2,
            postings_and_doc_values_only_field_infos(),
            vec![("body", body)],
        ));
        let fields = LeafFields::new(leaf_reader);
        assert_eq!(fields.size(), 1);
        assert_eq!(fields.iterator().count(), 1);
    }

    // ----- per-field MultiTerms memo (MultiFields.java:28, :44-46) -----

    /// `Fields` wrapper that counts how often `terms(field)` is delegated.
    struct CountingFields {
        inner: VecFields,
        calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl Fields for CountingFields {
        fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
            self.inner.iterator()
        }

        fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.inner.terms(field)
        }

        fn size(&self) -> i32 {
            self.inner.size()
        }
    }

    fn counting_multi_fields() -> (MultiFields, Arc<std::sync::atomic::AtomicUsize>) {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let make = |doc: i32| CountingFields {
            inner: VecFields::new([(
                "body".to_string(),
                VecTerms::new(
                    vec![(term(b"apple"), 1, 1, vec![doc])],
                    true,
                    false,
                    false,
                    false,
                ),
            )]),
            calls: Arc::clone(&calls),
        };
        let mf = MultiFields::new(
            vec![Box::new(make(0)), Box::new(make(1))],
            vec![ReaderSlice::new(0, 5, 0), ReaderSlice::new(5, 5, 1)],
        );
        (mf, calls)
    }

    #[test]
    fn multi_terms_are_memoised_per_field() {
        let (mf, calls) = counting_multi_fields();

        let first = mf.multi_terms("body").unwrap().unwrap();
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "the first call fans out to every sub"
        );

        let second = mf.multi_terms("body").unwrap().unwrap();
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "the second call must be served from the memo"
        );
        assert!(
            Arc::ptr_eq(&first, &second),
            "Lucene hands back the same Terms instance for a field"
        );

        // The trait-object path shares the same memo.
        assert!(mf.terms("body").unwrap().is_some());
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[test]
    fn a_memoised_multi_terms_is_still_independently_iterable() {
        let (mf, _calls) = counting_multi_fields();
        let shared = mf.multi_terms("body").unwrap().unwrap();

        // Two enums over the same shared instance do not interfere.
        let mut a = shared.iterator().unwrap();
        let mut b = shared.iterator().unwrap();
        assert_eq!(a.next().unwrap(), Some(term(b"apple")));
        assert_eq!(b.next().unwrap(), Some(term(b"apple")));
        assert!(a.next().unwrap().is_none());
        assert!(b.next().unwrap().is_none());

        // ... and the Arc handle itself satisfies `Terms`.
        assert_eq!(Terms::doc_count(&shared), 2);
        assert_eq!(Terms::sum_doc_freq(&shared), 2);
        assert!(Terms::has_freqs(&shared));
        assert_eq!(Terms::min(&shared).unwrap(), Some(term(b"apple")));
        assert_eq!(Terms::max(&shared).unwrap(), Some(term(b"apple")));
    }

    #[test]
    fn a_missing_field_is_not_memoised() {
        // Mirrors Lucene, whose `terms.put` runs only on the hit path.
        let (mf, calls) = counting_multi_fields();
        assert!(mf.multi_terms("nope").unwrap().is_none());
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert!(mf.multi_terms("nope").unwrap().is_none());
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            4,
            "a miss is recomputed, exactly as in Lucene"
        );
    }

    // ----- deliberate divergences pinned by tests -----

    #[test]
    fn min_and_max_skip_subs_with_no_terms_where_java_throws() {
        // Lucene's MultiTerms.getMin/getMax dereference each sub's result
        // unconditionally and would NPE on the empty sub; this port folds only
        // the terms that exist.
        let populated = VecTerms::new(
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
            vec![Box::new(EmptyTerms::new()), Box::new(populated)],
            vec![ReaderSlice::new(0, 2, 0), ReaderSlice::new(2, 2, 1)],
        )
        .unwrap();
        assert_eq!(mt.min().unwrap(), Some(term(b"banana")));
        assert_eq!(mt.max().unwrap(), Some(term(b"cherry")));

        // Every sub empty: None rather than a panic.
        let all_empty = MultiTerms::new(
            vec![Box::new(EmptyTerms::new()), Box::new(EmptyTerms::new())],
            vec![ReaderSlice::new(0, 1, 0), ReaderSlice::new(1, 1, 1)],
        )
        .unwrap();
        assert_eq!(all_empty.min().unwrap(), None);
        assert_eq!(all_empty.max().unwrap(), None);
    }

    #[test]
    fn a_failed_seek_exact_leaves_the_enum_unpositioned() {
        // Java leaves `current` stale here; this port honours the TermsEnum
        // contract and reports the enum as unpositioned.
        let mt = two_leaves_with_a_shared_trailing_term();
        let mut te = mt.multi_iterator().unwrap().unwrap();
        assert!(te.seek_exact(&term(b"a")).unwrap());
        assert_eq!(te.term().unwrap(), term(b"a"));

        assert!(!te.seek_exact(&term(b"bb")).unwrap());
        assert!(
            te.term().is_err(),
            "a missed seek_exact must not leave the previous term readable"
        );
        assert_eq!(te.doc_freq().unwrap(), 0);
    }

    #[test]
    fn a_seek_ceil_past_the_end_leaves_the_enum_unpositioned() {
        // Java leaves `current` stale on SeekStatus::END; this port clears it.
        let mt = two_leaves_with_a_shared_trailing_term();
        let mut te = mt.multi_iterator().unwrap().unwrap();
        assert_eq!(te.seek_ceil(&term(b"a")).unwrap(), SeekStatus::FOUND);
        assert_eq!(te.term().unwrap(), term(b"a"));

        assert_eq!(te.seek_ceil(&term(b"zzz")).unwrap(), SeekStatus::END);
        assert!(
            te.term().is_err(),
            "SeekStatus::END must not leave the previous term readable"
        );
    }

    // ----- ReaderSlice length convention across the two factories -----

    /// Composite reader over two leaves that both carry a `body` postings
    /// field, sized 3 and 4 documents.
    fn two_leaf_reader_with_a_body_field() -> Arc<dyn IndexReader> {
        let make = |docs: Vec<i32>| {
            VecTerms::new(
                vec![(term(b"apple"), docs.len() as i32, docs.len() as i64, docs)],
                true,
                false,
                false,
                false,
            )
        };
        let a: Arc<dyn IndexReader> = Arc::new(StubLeaf::with_fields(
            3,
            3,
            postings_and_doc_values_only_field_infos(),
            vec![("body", make(vec![0, 2]))],
        ));
        let b: Arc<dyn IndexReader> = Arc::new(StubLeaf::with_fields(
            4,
            4,
            postings_and_doc_values_only_field_infos(),
            vec![("body", make(vec![1]))],
        ));
        Arc::new(MultiReader::new(vec![a, b], true).unwrap())
    }

    #[test]
    fn get_fields_slices_carry_the_leafs_own_max_doc_as_length() {
        // The convention documented on ReaderSlice: length is the number of
        // documents in the slice. This factory honours it.
        let mr = two_leaf_reader_with_a_body_field();
        let mf = MultiFields::get_fields(&mr).unwrap();
        assert_eq!(mf.get_sub_slices()[0], ReaderSlice::new(0, 3, 0));
        assert_eq!(mf.get_sub_slices()[1], ReaderSlice::new(3, 4, 1));
        // The MultiTerms built from them inherits the same slices.
        let terms = mf.multi_terms("body").unwrap().unwrap();
        assert_eq!(terms.get_sub_slices()[0], ReaderSlice::new(0, 3, 0));
        assert_eq!(terms.get_sub_slices()[1], ReaderSlice::new(3, 4, 1));
    }

    #[test]
    fn get_terms_maps_leaf_doc_ids_into_the_composite_space() {
        // `MultiTerms::get_terms` builds its slices with the *composite*
        // maxDoc as `length` (transcribing MultiTerms.java:82). Only `start`
        // is ever read, and this is what reading it produces.
        let mr = two_leaf_reader_with_a_body_field();
        let terms = MultiTerms::get_terms(&mr, "body").unwrap().unwrap();
        let mut te = terms.iterator().unwrap();
        assert_eq!(te.next().unwrap(), Some(term(b"apple")));
        assert_eq!(te.doc_freq().unwrap(), 3);
        let mut postings = te.postings(None, POSTINGS_ENUM_FREQS).unwrap();
        assert_eq!(
            drain_docs(postings.as_mut()),
            vec![0, 2, 4],
            "leaf 1's local doc 1 is global doc 4"
        );
    }

    #[test]
    fn get_terms_over_a_single_leaf_returns_the_leafs_own_terms() {
        let a: Arc<dyn IndexReader> = Arc::new(StubLeaf::with_fields(
            3,
            3,
            postings_and_doc_values_only_field_infos(),
            vec![(
                "body",
                VecTerms::new(
                    vec![(term(b"apple"), 1, 1, vec![0])],
                    true,
                    false,
                    false,
                    false,
                ),
            )],
        ));
        let mr: Arc<dyn IndexReader> = Arc::new(MultiReader::new(vec![a], true).unwrap());
        let terms = MultiTerms::get_terms(&mr, "body").unwrap().unwrap();
        // Not wrapped: MultiTerms reports size() == -1, the leaf's VecTerms
        // reports the real count.
        assert_eq!(terms.size(), 1);
        assert!(MultiTerms::get_terms(&mr, "missing").unwrap().is_none());
    }

    // ----- MultiTerms::getTermPostingsEnum overloads -----

    #[test]
    fn get_term_postings_enum_all_defaults_to_every_feature() {
        let mr = two_leaf_reader_with_a_body_field();
        let mut all = MultiTerms::get_term_postings_enum_all(&mr, "body", &term(b"apple"))
            .unwrap()
            .expect("apple exists");
        assert_eq!(drain_docs(all.as_mut()), vec![0, 2, 4]);

        // Same result as the explicit-flags overload with PostingsEnum.ALL.
        let mut explicit = MultiTerms::get_term_postings_enum(
            &mr,
            "body",
            &term(b"apple"),
            crate::index::postings_enum::POSTINGS_ENUM_ALL,
        )
        .unwrap()
        .expect("apple exists");
        assert_eq!(drain_docs(explicit.as_mut()), vec![0, 2, 4]);
    }

    #[test]
    fn get_term_postings_enum_all_is_none_for_a_missing_field_or_term() {
        let mr = two_leaf_reader_with_a_body_field();
        assert!(
            MultiTerms::get_term_postings_enum_all(&mr, "missing", &term(b"apple"))
                .unwrap()
                .is_none()
        );
        assert!(
            MultiTerms::get_term_postings_enum_all(&mr, "body", &term(b"durian"))
                .unwrap()
                .is_none()
        );
    }
}
