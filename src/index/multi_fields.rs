//! Multi-segment postings aggregation ported from `org.apache.lucene.index`.
//!
//! This module provides [`MultiFields`] and [`MultiTerms`], the utilities that
//! present a composite reader's multiple leaves as a single [`Fields`] /
//! [`Terms`] view. The merge is driven by [`MultiTermsEnum`], which merges the
//! per-leaf [`TermsEnum`]s in sorted term order, dedups identical terms across
//! leaves, and concatenates postings with the correct per-leaf `docBase`
//! offset.
//!
//! # Merge strategy
//!
//! `MultiTermsEnum` is Lucene's `MultiTermsEnum`, structure included:
//!
//! - every sub-enum is a [`TermsEnumWithSlice`] — a
//!   [`TermsEnumIndex`] with its [`ReaderSlice`] — and the subs are merged by a
//!   `TermMergeQueue` built on the crate's port of Lucene's `PriorityQueue`;
//! - `next()` pushes every sub on the current term back into the queue after
//!   advancing it (`pushTop`), then gathers every sub that shares the new
//!   smallest term into `top` (`pullTop`, `TermMergeQueue.fillTop`). Identical
//!   terms across leaves are thus exposed once;
//! - `doc_freq` / `total_term_freq` sum over `top`;
//! - `postings` builds or reuses a [`MultiPostingsEnum`] that concatenates the
//!   per-leaf postings (each leaf's local doc IDs offset by its slice
//!   `start`); because the slices partition the global doc-ID space, the
//!   concatenation is a globally sorted doc-ID stream;
//! - `seek_exact` / `seek_ceil` apply the LUCENE-2130 optimisation: when the
//!   new seek term is not before the last `seek_ceil` term, a sub whose current
//!   term is already past the seek term is not re-seeked.
//!
//! # Reference
//!
//! - `org.apache.lucene.index.MultiFields`
//! - `org.apache.lucene.index.MultiTerms`
//! - `org.apache.lucene.index.MultiTermsEnum`
//! - `org.apache.lucene.index.MultiPostingsEnum`
//! - `org.apache.lucene.util.MergedIterator` (for the field-name merge)

#![deny(unsafe_code)]

use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, RwLock};

use crate::error::{LuceneError, Result};
use crate::index::leaf_reader::LeafReader;
use crate::index::postings_enum::{Impacts, ImpactsEnum, ImpactsSource, PostingsEnum};
use crate::index::reader_context::LeafReaderContext;
use crate::index::terms::{EmptyTermsEnum, Fields, SeekStatus, Terms, TermsEnum};
use crate::index::terms_enum_index::TermsEnumIndex;
use crate::index::{IndexReader, ReaderSlice};
use crate::search::DocIdSetIterator;
use crate::store::java_nio_file::throw_unchecked;
use crate::util::attribute::AttributeSource;
use crate::util::automaton::CompiledAutomaton;
use crate::util::BytesRef;
use crate::util::{MergedIterator, PriorityQueue, PriorityQueueComparator};

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
        // so the returned iterator does not borrow from a local. A
        // `FieldsProducer` lists its fields in sorted order — the block tree's
        // `TreeMap` keys (`Lucene103BlockTreeTermsReader.java:178, 289-291`) —
        // and `MultiFields` merges the sorted lists with a `MergedIterator`,
        // so the names are sorted here too, in the `TreeMap`'s order:
        // `String.compareTo`, UTF-16 code units.
        let mut names: Vec<String> = self
            .reader
            .get_field_infos()
            .iter()
            .filter(|fi| fi.index_options != crate::index::IndexOptions::NONE)
            .map(|fi| fi.get_name().to_string())
            .collect();
        names.sort_by(|left, right| crate::util::compare_utf16(left, right));
        names
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
        let leaves: Vec<Arc<LeafReaderContext>> = Arc::clone(reader).leaves()?;
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

        // `Arc`: the memo is shared across threads, as Java's `ConcurrentHashMap` is.
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
    /// Equivalent to `MultiFields.iterator()` (`MultiFields.java:51-58`): the
    /// sub-iterators merged by a `MergedIterator`, which removes the
    /// duplicates across them and pulls each one only as far as it is read.
    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        Box::new(MergedIterator::new(
            self.subs
                .iter()
                .map(|sub| sub.iterator())
                .collect::<Vec<_>>(),
        ))
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
        let leaves = Arc::clone(reader).leaves()?;
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
                return te.postings(None, flags);
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

    /// Builds a [`MultiTermsEnum`] over `sub_slices` and resets it on the given
    /// per-sub enums, returning `None` when no sub yields a term.
    ///
    /// The shared tail of `MultiTerms.iterator()` and `MultiTerms.intersect`
    /// (`MultiTerms.java:138-150`, `MultiTerms.java:181-195`): each enum is
    /// wrapped in a [`TermsEnumIndex`] tagged with its sub's position, and
    /// `new MultiTermsEnum(subSlices).reset(...)` answers `TermsEnum.EMPTY`
    /// when no sub yields a term — `None` here.
    fn build_enum(
        sub_slices: &[ReaderSlice],
        terms_enums: Vec<TermsEnumIndex>,
    ) -> Result<Option<MultiTermsEnum>> {
        if terms_enums.is_empty() {
            return Ok(None);
        }
        let mut multi_terms_enum = MultiTermsEnum::new(sub_slices)?;
        let has_terms = multi_terms_enum.reset(terms_enums)?.is_some();
        Ok(has_terms.then_some(multi_terms_enum))
    }

    /// Concretely typed counterpart of [`Terms::intersect`], returning `None`
    /// when no sub contributed a term.
    ///
    /// Equivalent to `MultiTerms.intersect(CompiledAutomaton, BytesRef)`
    /// (`MultiTerms.java:138-150`).
    pub fn multi_intersect(
        &self,
        compiled: &CompiledAutomaton,
        start_term: Option<&BytesRef>,
    ) -> Result<Option<MultiTermsEnum>> {
        let mut terms_enums: Vec<TermsEnumIndex> = Vec::with_capacity(self.subs.len());
        for (i, sub) in self.subs.iter().enumerate() {
            // Java skips a `null` enum; `Terms::intersect` always answers one.
            terms_enums.push(TermsEnumIndex::new(sub.intersect(compiled, start_term)?, i));
        }
        Self::build_enum(&self.sub_slices, terms_enums)
    }

    /// Concretely typed counterpart of [`Terms::iterator`].
    ///
    /// Equivalent to `MultiTerms.iterator()` (`MultiTerms.java:181-195`).
    /// Returns `None` exactly when [`Terms::iterator`] would return an
    /// [`EmptyTermsEnum`], i.e. when no sub contributed a term. Java expresses
    /// the same distinction by comparing the returned enum against the
    /// `TermsEnum.EMPTY` singleton (LUCENE-6826) after a cast to
    /// `MultiTermsEnum`; `Option` makes it type-safe here.
    pub fn multi_iterator(&self) -> Result<Option<MultiTermsEnum>> {
        let mut terms_enums: Vec<TermsEnumIndex> = Vec::with_capacity(self.subs.len());
        for (i, sub) in self.subs.iter().enumerate() {
            // Java skips a `null` enum; `Terms::iterator` always answers one.
            terms_enums.push(TermsEnumIndex::new(sub.iterator()?, i));
        }
        Self::build_enum(&self.sub_slices, terms_enums)
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

    fn size(&self) -> Result<i64> {
        Ok({
            // Matches Lucene: the merged term count is not cheap to compute.
            -1
        })
    }

    fn sum_total_term_freq(&self) -> Result<i64> {
        Ok({
            let mut sum: i64 = 0;
            for sub in &self.subs {
                let v = sub.sum_total_term_freq()?;
                debug_assert!(v != -1);
                sum += v;
            }
            sum
        })
    }

    fn sum_doc_freq(&self) -> Result<i64> {
        Ok({
            let mut sum: i64 = 0;
            for sub in &self.subs {
                let v = sub.sum_doc_freq()?;
                debug_assert!(v != -1);
                sum += v;
            }
            sum
        })
    }

    fn doc_count(&self) -> Result<i32> {
        Ok({
            let mut sum: i32 = 0;
            for sub in &self.subs {
                let v = sub.doc_count()?;
                debug_assert!(v != -1);
                sum = sum.wrapping_add(v);
            }
            sum
        })
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

    fn size(&self) -> Result<i64> {
        (**self).size()
    }

    fn sum_total_term_freq(&self) -> Result<i64> {
        (**self).sum_total_term_freq()
    }

    fn sum_doc_freq(&self) -> Result<i64> {
        (**self).sum_doc_freq()
    }

    fn doc_count(&self) -> Result<i32> {
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

    fn stats(&self) -> Result<Box<dyn crate::index::terms::StatsObject>> {
        (**self).stats()
    }
}

// ---------------------------------------------------------------------------
// MultiTermsEnum
// ---------------------------------------------------------------------------

/// Source of the per-instance identity [`MultiTermsEnum`] stamps onto every
/// [`MultiPostingsEnum`] it builds.
///
/// Java compares the two enums by object reference in
/// `MultiPostingsEnum.canReuse`. A Rust value has no address it keeps across
/// moves, so each `MultiTermsEnum` takes a number that no other instance of
/// the process ever takes, and the two are compared on that instead. The
/// comparison answers exactly what the reference comparison answers.
static NEXT_MULTI_TERMS_ENUM_ID: AtomicU64 = AtomicU64::new(0);

/// The `NullPointerException` Java raises when a method is invoked on a
/// sub-enum's `null` `termsEnum` field, reached through `expression`.
///
/// The JDK names the dereferenced expression: `this.top[i].termsEnum` in
/// `docFreq` and `totalTermFreq` (`MultiTermsEnum.java:299`, `:308`), and
/// `entry.termsEnum` in `postings` (`MultiTermsEnum.java:340-341`). Measured
/// against `lucene-core-10.5.0.jar` by setting the field to `null` through
/// reflection.
fn null_sub_terms_enum(method: &str, expression: &str) -> LuceneError {
    LuceneError::null_pointer_with_message(format!(
        "Cannot invoke \"org.apache.lucene.index.TermsEnum.{method}\" because \"{expression}\" is null"
    ))
}

/// One sub-enum of [`MultiTermsEnum`]: a [`TermsEnumIndex`] paired with the
/// [`ReaderSlice`] that says where its sub-reader sits in the global doc-ID
/// space.
///
/// Equivalent to `MultiTermsEnum.TermsEnumWithSlice`
/// (`MultiTermsEnum.java:358-371`), which extends `TermsEnumIndex`; Rust has no
/// implementation inheritance, so the `TermsEnumIndex` is held and reached
/// through [`terms_enum_index`](Self::terms_enum_index). Java declares the
/// class package-private and hands arrays of it out through `getMatchArray()`;
/// [`MultiTermsEnum::get_match_array`] does the same here.
pub struct TermsEnumWithSlice {
    /// The `TermsEnumIndex` this class extends. Its enum is `None` — Java's
    /// `super(null, index)` — until [`MultiTermsEnum::reset`] hands it one.
    base: TermsEnumIndex,
    /// Java's `private final ReaderSlice subSlice`.
    sub_slice: ReaderSlice,
}

impl TermsEnumWithSlice {
    /// Equivalent to `TermsEnumWithSlice(int index, ReaderSlice subSlice)`
    /// (`MultiTermsEnum.java:361-365`), which calls `super(null, index)`.
    fn new(index: usize, sub_slice: ReaderSlice) -> Self {
        debug_assert!(sub_slice.length >= 0, "length={}", sub_slice.length);
        Self {
            base: TermsEnumIndex::with_optional_enum(None, index),
            sub_slice,
        }
    }

    /// Returns the [`TermsEnumIndex`] this sub-enum extends in Java.
    pub fn terms_enum_index(&self) -> &TermsEnumIndex {
        &self.base
    }

    /// Returns the slice of the sub-reader this enum belongs to.
    ///
    /// Equivalent to reading the `TermsEnumWithSlice.subSlice` field.
    pub fn sub_slice(&self) -> ReaderSlice {
        self.sub_slice
    }
}

impl std::fmt::Display for TermsEnumWithSlice {
    /// Port of `MultiTermsEnum.TermsEnumWithSlice.toString()`
    /// (`MultiTermsEnum.java:367-370`): the slice, a colon, then
    /// `super.toString()` — `TermsEnumIndex`'s, i.e. the wrapped enumeration's
    /// own rendering, or `null` for a sub that was never reset.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.sub_slice, self.base)
    }
}

/// `TermMergeQueue.lessThan`: `termsA.compareTermTo(termsB) < 0`
/// (`MultiTermsEnum.java:382-385`), over the subs' live terms.
///
/// Java's queue holds the `TermsEnumWithSlice` objects themselves — the same
/// objects `MultiTermsEnum` keeps in its `subs`, `currentSubs` and `top`
/// arrays — and `lessThan` compares their current terms as they stand at that
/// moment. Rust cannot let the queue and the enum alias the subs, so this
/// comparator owns `subs`, the queue holds each queued sub's position in it
/// (which is its `subIndex`), and every comparison reads the two subs' current
/// terms, exactly as Java's does.
struct TermMergeLessThan {
    /// Java's `MultiTermsEnum.subs`: every sub, one per sub-reader slice, at
    /// its sub index.
    subs: Vec<TermsEnumWithSlice>,
}

impl PriorityQueueComparator<usize> for TermMergeLessThan {
    fn less_than(&self, a: &usize, b: &usize) -> bool {
        self.subs[*a].base.compare_term_to(&self.subs[*b].base) == Ordering::Less
    }
}

/// The priority queue [`MultiTermsEnum`] merges its subs with, ordered by
/// current term.
///
/// Equivalent to `MultiTermsEnum.TermMergeQueue` (`MultiTermsEnum.java:373-418`),
/// a `PriorityQueue<TermsEnumWithSlice>`, built here on the crate's port of
/// that `PriorityQueue`, so that the heap — and therefore the order
/// [`fill_top`](Self::fill_top) gathers equal terms in — is Lucene's. Its
/// comparator owns the subs ([`TermMergeLessThan`]); the queue holds their
/// positions, and [`subs`](Self::subs) / [`subs_mut`](Self::subs_mut) are how
/// [`MultiTermsEnum`] reaches them.
struct TermMergeQueue {
    queue: PriorityQueue<usize, TermMergeLessThan>,
    /// Java's `final int[] stack`, scratch for [`fill_top`](Self::fill_top).
    stack: Vec<usize>,
}

impl TermMergeQueue {
    /// Equivalent to `TermMergeQueue(int size)` with `size = subs.len()`,
    /// taking the subs its comparator reads.
    fn new(subs: Vec<TermsEnumWithSlice>) -> Result<Self> {
        let size = subs.len();
        Ok(Self {
            queue: PriorityQueue::new(size, TermMergeLessThan { subs })?,
            stack: vec![0; size],
        })
    }

    /// Java's `MultiTermsEnum.subs`.
    fn subs(&self) -> &[TermsEnumWithSlice] {
        &self.queue.comparator().subs
    }

    /// Java's `MultiTermsEnum.subs`, for moving a sub. A queued sub moved
    /// here is re-sifted with [`update_top`](Self::update_top), as in Java.
    fn subs_mut(&mut self) -> &mut [TermsEnumWithSlice] {
        &mut self.queue.comparator_mut().subs
    }

    /// Equivalent to `PriorityQueue.add(TermsEnumWithSlice)`.
    fn add(&mut self, sub: usize) {
        self.queue.add(sub);
    }

    /// Equivalent to `PriorityQueue.top()`.
    fn top(&self) -> Option<usize> {
        self.queue.top().copied()
    }

    /// Equivalent to `PriorityQueue.pop()`.
    fn pop(&mut self) -> Option<usize> {
        self.queue.pop()
    }

    /// Equivalent to `PriorityQueue.updateTop()`, after the top sub moved.
    fn update_top(&mut self) {
        self.queue.update_top();
    }

    /// Equivalent to `PriorityQueue.size()`.
    fn size(&self) -> usize {
        self.queue.size()
    }

    /// Equivalent to `PriorityQueue.clear()`.
    fn clear(&mut self) {
        self.queue.clear();
    }

    /// Adds the [`top`](Self::top) slice as well as all slices that are
    /// positioned on the same term to `tops` and returns how many of them there
    /// are.
    ///
    /// Equivalent to `TermMergeQueue.fillTop(TermsEnumWithSlice[])`
    /// (`MultiTermsEnum.java:391-413`): a depth-first walk of the heap from its
    /// root that only descends into children holding the top term. Java writes
    /// the first `numTop` slots of its array; `tops` is cleared and filled with
    /// exactly those.
    fn fill_top(&mut self, tops: &mut Vec<usize>) -> usize {
        tops.clear();
        let size = self.queue.size();
        if size == 0 {
            return 0;
        }
        let subs = &self.queue.comparator().subs;
        let heap = self.queue.heap_array();
        let Some(top0) = heap[1] else {
            return 0;
        };
        tops.push(top0);
        let mut num_top = 1;
        self.stack[0] = 1;
        let mut stack_len = 1;

        while stack_len != 0 {
            stack_len -= 1;
            let index = self.stack[stack_len];
            let left_child = index << 1;
            let end = size.min(left_child + 1);
            let mut child = left_child;
            while child <= end {
                if let Some(te) = heap[child] {
                    if subs[te].base.compare_term_to(&subs[tops[0]].base) == Ordering::Equal {
                        tops.push(te);
                        num_top += 1;
                        self.stack[stack_len] = child;
                        stack_len += 1;
                    }
                }
                child += 1;
            }
        }
        num_top
    }
}

/// A [`TermsEnum`] that merge-sorts sub-`TermsEnum`s by term text, dedups
/// identical terms across subs, and aggregates per-term statistics and
/// postings across the subs positioned at the current term.
///
/// Equivalent to `org.apache.lucene.index.MultiTermsEnum`. Built with
/// [`new`](Self::new) over the sub-readers' slices, then
/// [`reset`](Self::reset) on the sub-enums, exactly as
/// `new MultiTermsEnum(slices).reset(termsEnumsIndex)`.
///
/// Java keeps the subs in four places at once — `subs`, `currentSubs`, `top`
/// and the merge queue — as references to the same objects. Here the merge
/// queue's comparator owns them (`TermMergeLessThan`) and `currentSubs`,
/// `top` and the queue hold their positions in it.
pub struct MultiTermsEnum {
    /// Java's `private final TermMergeQueue queue`, whose comparator also
    /// holds Java's `subs`.
    queue: TermMergeQueue,
    /// Java's `currentSubs[0..numSubs]`: the subs that had at least one term
    /// for this field on [`reset`](Self::reset).
    current_subs: Vec<usize>,
    /// Java's `top[0..numTop]`: the subs positioned on the current term.
    top: Vec<usize>,
    /// Java's `lastSeek`: the last `seekCeil` term, for the LUCENE-2130
    /// seek optimisation, or `None`.
    last_seek: Option<BytesRef>,
    /// Java's `lastSeekExact`.
    last_seek_exact: bool,
    /// Java's `current`, as the sub whose term it is, or `None` for `null`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java's `current` is the `BytesRef`
    /// object the supplying sub returned (`MultiTermsEnum.java:156`, `:208`,
    /// `:249`). A codec's terms enum returns its own term buffer, so Java's
    /// `current` keeps following that enum as it moves: after a missed
    /// `seekExact` it shows the term the miss scanned to, after an `END`
    /// `seekCeil` the emptied buffer (`StaleCurrentOracle` over two Lucene104
    /// segments: `cccc` and `""`). [`TermsEnum::term`] returns an owned copy
    /// here, so the port keeps the supplying sub and reads its enum's term
    /// whenever `current` is read, which is Java's behaviour for every enum
    /// that returns its own buffer. A sub enum that answers a new `BytesRef`
    /// on each call would leave Java's `current` on the old value, where this
    /// port reads the moved enum; and `next()` after a missed `seekExact`
    /// re-seeks a copy of the term, where Java passes the buffer itself.
    current: Option<usize>,
    /// Identity of this enum, so that a [`MultiPostingsEnum`] it built can be
    /// told apart from one another enum built. Stands in for the object
    /// reference Java compares in `MultiPostingsEnum.canReuse`.
    id: u64,
    /// The attributes `BaseTermsEnum.attributes()` creates.
    atts: AttributeSource,
}

impl MultiTermsEnum {
    /// Creates an enum over the sub-readers described by `slices`, holding no
    /// sub-enum yet; [`reset`](Self::reset) supplies them.
    ///
    /// Equivalent to `MultiTermsEnum(ReaderSlice[] slices)`
    /// (`MultiTermsEnum.java:65-76`). Java also allocates a
    /// `MultiPostingsEnum.EnumWithSlice[] subDocs` scratch array here, which
    /// [`postings`](TermsEnum::postings) builds per call instead.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`](crate::error::LuceneErrorKind::IllegalArgument) when `slices` is too large for
    /// a `PriorityQueue`, as Java's constructor throws.
    pub fn new(slices: &[ReaderSlice]) -> Result<Self> {
        let n = slices.len();
        let subs = slices
            .iter()
            .enumerate()
            .map(|(i, &slice)| TermsEnumWithSlice::new(i, slice))
            .collect();
        Ok(Self {
            queue: TermMergeQueue::new(subs)?,
            current_subs: Vec::with_capacity(n),
            top: Vec::with_capacity(n),
            last_seek: None,
            last_seek_exact: false,
            current: None,
            id: NEXT_MULTI_TERMS_ENUM_ID.fetch_add(1, AtomicOrdering::Relaxed),
            atts: AttributeSource::new(),
        })
    }

    /// Returns how many sub-reader slices contain the current term.
    ///
    /// Equivalent to `org.apache.lucene.index.MultiTermsEnum#getMatchCount`,
    /// which returns the `numTop` counter that says how much of the `top`
    /// array is live.
    ///
    /// See also [`get_match_array`](Self::get_match_array).
    pub fn get_match_count(&self) -> usize {
        self.top.len()
    }

    /// Returns the sub-reader slices positioned at the current term.
    ///
    /// Equivalent to `org.apache.lucene.index.MultiTermsEnum#getMatchArray`.
    /// Java hands back the whole `top` array, of which only the first
    /// [`get_match_count`](Self::get_match_count) entries are live; this port
    /// returns exactly the live ones, in the array's order.
    pub fn get_match_array(&self) -> Vec<&TermsEnumWithSlice> {
        let subs = self.queue.subs();
        self.top.iter().map(|&sub| &subs[sub]).collect()
    }

    /// Positions this enum on the given sub-enums, which must be newly
    /// created ([`TermsEnum::next`] not yet called), and returns `Some(self)`,
    /// or `None` when none of them has a term.
    ///
    /// Equivalent to `MultiTermsEnum.reset(TermsEnumIndex[])`
    /// (`MultiTermsEnum.java:87-113`): each sub-enum is advanced once, and the
    /// ones that yield a term are adopted by the sub at their
    /// [`sub_index`](TermsEnumIndex::sub_index) and queued. Java returns
    /// `TermsEnum.EMPTY` when the queue stays empty and `this` otherwise; `None`
    /// stands for `EMPTY`.
    ///
    /// # Errors
    ///
    /// Propagates whatever advancing a sub-enum raises, and returns
    /// [`LuceneErrorKind::IllegalArgument`](crate::error::LuceneErrorKind::IllegalArgument) — Java's
    /// `ArrayIndexOutOfBoundsException` — for a sub index this enum has no
    /// slice for.
    pub fn reset(&mut self, terms_enums_index: Vec<TermsEnumIndex>) -> Result<Option<&mut Self>> {
        debug_assert!(terms_enums_index.len() <= self.queue.subs().len());
        self.current_subs.clear();
        self.top.clear();
        self.queue.clear();
        let num_slices = self.queue.subs().len();
        for mut terms_enum_index in terms_enums_index {
            let has_term = terms_enum_index.next()?.is_some();
            if has_term {
                let sub_index = terms_enum_index.sub_index();
                let entry = self.queue.subs_mut().get_mut(sub_index).ok_or_else(|| {
                    LuceneError::illegal_argument(format!(
                        "Index {sub_index} out of bounds for length {num_slices}"
                    ))
                })?;
                entry.base.reset(terms_enum_index);
                self.queue.add(sub_index);
                self.current_subs.push(sub_index);
            } else {
                // field has no terms
            }
        }

        if self.queue.size() == 0 {
            Ok(None)
        } else {
            Ok(Some(self))
        }
    }

    /// Reads the term `current` refers to: the current term of the enum of
    /// sub `sub`, as it stands now (see [`current`](Self#structfield.current)).
    fn sub_term(&self, sub: usize) -> Result<Option<BytesRef>> {
        // Java's `current` is the `BytesRef` the sub's `term()` returned; it is
        // read as it stands, without calling `term()` again
        // (`MultiTermsEnum.java:208, 249, 279-281`).
        self.queue.subs()[sub]
            .base
            .terms_enum()
            .ok_or_else(|| {
                LuceneError::illegal_state(
                    "INVARIANT: a sub that supplied a term holds an enum".to_string(),
                )
            })?
            .term_buffer()
    }

    /// Extracts every sub that sits on the top term out of the queue into
    /// `top`. Equivalent to `MultiTermsEnum.pullTop()`
    /// (`MultiTermsEnum.java:244-250`).
    fn pull_top(&mut self) {
        // extract all subs from the queue that have the same top term
        debug_assert!(self.top.is_empty());
        self.queue.fill_top(&mut self.top);
        self.current = self.top.first().copied();
    }

    /// Advances every sub on the current term and restores the queue.
    /// Equivalent to `MultiTermsEnum.pushTop()` (`MultiTermsEnum.java:252-263`),
    /// which advances the queue's top `numTop` times.
    fn push_top(&mut self) -> Result<()> {
        // call next() on each top, and reorder queue
        for _ in 0..self.top.len() {
            let top = self.queue.top().ok_or_else(|| {
                LuceneError::null_pointer_with_message(
                    "Cannot invoke \"org.apache.lucene.index.MultiTermsEnum$TermsEnumWithSlice.next()\" because \"top\" is null"
                        .to_string(),
                )
            })?;
            if self.queue.subs_mut()[top].base.next()?.is_none() {
                self.queue.pop();
            } else {
                self.queue.update_top();
            }
        }
        self.top.clear();
        Ok(())
    }
}

impl std::fmt::Display for MultiTermsEnum {
    /// Port of `MultiTermsEnum.toString()` (`MultiTermsEnum.java:420-423`):
    /// the name, then `Arrays.toString(subs)` — Java's array rendering, which
    /// is the elements separated by `", "` inside square brackets.
    ///
    /// Measured against `lucene-core-10.5.0.jar`:
    /// `MultiTermsEnum([ReaderSlice[start=0, length=6, readerIndex=0]:org.apache.lucene.codecs.lucene103.blocktree.SegmentTermsEnum@1292f6e0, …])`.
    /// Only the hexadecimal identity of each sub-enumeration diverges; see
    /// [`java_object_to_string`](crate::index::index_reader::java_object_to_string).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MultiTermsEnum([")?;
        for (i, sub) in self.queue.subs().iter().enumerate() {
            if i != 0 {
                f.write_str(", ")?;
            }
            std::fmt::Display::fmt(sub, f)?;
        }
        f.write_str("])")
    }
}

impl TermsEnum for MultiTermsEnum {
    fn attributes(&mut self) -> &mut AttributeSource {
        &mut self.atts
    }

    /// Equivalent to `MultiTermsEnum.seekExact(BytesRef)`
    /// (`MultiTermsEnum.java:115-164`), LUCENE-2130 optimisation included.
    ///
    /// As in Java, `current` is only assigned for a sub that holds the term,
    /// so after a miss [`term`](TermsEnum::term) reads the sub that supplied
    /// the previous term — wherever the miss moved that sub's enum.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while seeking a sub-enum.
    fn seek_exact(&mut self, term: &BytesRef) -> Result<bool> {
        self.queue.clear();
        self.top.clear();

        let seek_opt = self.last_seek.as_ref().is_some_and(|last| last <= term);

        self.last_seek = None;
        self.last_seek_exact = true;

        for i in 0..self.current_subs.len() {
            let sub = self.current_subs[i];
            let entry = &mut self.queue.subs_mut()[sub].base;
            // LUCENE-2130: if we had just seek'd already, prior to this seek,
            // and the new seek term is after the previous one, don't try to
            // re-seek this sub if its current term is already beyond this new
            // seek term. Doing so is a waste because this sub will simply seek
            // to the same spot.
            let status = if seek_opt {
                match entry.term().map(|cur_term| term.cmp(cur_term)) {
                    Some(Ordering::Equal) => true,
                    Some(Ordering::Less) => false,
                    Some(Ordering::Greater) => entry.seek_exact(term)?,
                    None => false,
                }
            } else {
                entry.seek_exact(term)?
            };

            if status {
                self.top.push(sub);
                self.current = Some(sub);
                debug_assert_eq!(Some(term), entry.term());
            }
        }

        // if at least one sub had exact match to the requested term then we
        // found match
        Ok(!self.top.is_empty())
    }

    /// Equivalent to `MultiTermsEnum.seekCeil(BytesRef)`
    /// (`MultiTermsEnum.java:166-232`), LUCENE-2130 optimisation included.
    ///
    /// As in Java, [`SeekStatus::END`] leaves `current` on the sub that
    /// supplied the previous term, so [`term`](TermsEnum::term) reads that
    /// sub's enum wherever the seek left it.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while seeking a sub-enum.
    fn seek_ceil(&mut self, term: &BytesRef) -> Result<SeekStatus> {
        self.queue.clear();
        self.top.clear();
        self.last_seek_exact = false;

        let seek_opt = self.last_seek.as_ref().is_some_and(|last| last <= term);

        self.last_seek = Some(term.clone());

        for i in 0..self.current_subs.len() {
            let sub = self.current_subs[i];
            let status = {
                let entry = &mut self.queue.subs_mut()[sub].base;
                // LUCENE-2130: see seek_exact.
                if seek_opt {
                    match entry.term().map(|cur_term| term.cmp(cur_term)) {
                        Some(Ordering::Equal) => SeekStatus::FOUND,
                        Some(Ordering::Less) => SeekStatus::NOT_FOUND,
                        Some(Ordering::Greater) => entry.seek_ceil(term)?,
                        None => SeekStatus::END,
                    }
                } else {
                    entry.seek_ceil(term)?
                }
            };

            if status == SeekStatus::FOUND {
                self.top.push(sub);
                self.current = Some(sub);
                self.queue.add(sub);
            } else if status == SeekStatus::NOT_FOUND {
                debug_assert!(self.queue.subs()[sub].base.term().is_some());
                self.queue.add(sub);
            } else {
                debug_assert_eq!(status, SeekStatus::END);
            }
        }

        if !self.top.is_empty() {
            // at least one sub had exact match to the requested term
            Ok(SeekStatus::FOUND)
        } else if self.queue.size() > 0 {
            // no sub had exact match, but at least one sub found a term after
            // the requested term -- advance to that next term:
            self.pull_top();
            Ok(SeekStatus::NOT_FOUND)
        } else {
            Ok(SeekStatus::END)
        }
    }

    /// Equivalent to `MultiTermsEnum.seekExact(long)`, which throws
    /// `UnsupportedOperationException`.
    fn seek_ord(&mut self, _ord: i64) -> Result<()> {
        Err(LuceneError::unsupported_operation(String::new()))
    }

    /// Equivalent to `MultiTermsEnum.term()`, which answers the `current`
    /// field: the current term of the sub that supplied it, read now (see
    /// [`MultiTermsEnum`]'s `current`), or `None` for Java's `null` when the
    /// enum is not positioned.
    fn term(&self) -> Result<Option<BytesRef>> {
        match self.current {
            Some(sub) => self.sub_term(sub),
            None => Ok(None),
        }
    }

    /// Equivalent to `MultiTermsEnum.ord()`, which throws
    /// `UnsupportedOperationException`.
    fn ord(&self) -> Result<i64> {
        Err(LuceneError::unsupported_operation(String::new()))
    }

    /// Equivalent to `MultiTermsEnum.docFreq()` (`MultiTermsEnum.java:295-302`):
    /// the `int` sum over the subs on the current term.
    fn doc_freq(&mut self) -> Result<i32> {
        let mut sum: i32 = 0;
        for i in 0..self.top.len() {
            let sub = self.top[i];
            let terms_enum = self.queue.subs_mut()[sub]
                .base
                .terms_enum_mut()
                .ok_or_else(|| null_sub_terms_enum("docFreq()", "this.top[i].termsEnum"))?;
            sum = sum.wrapping_add(terms_enum.doc_freq()?);
        }
        Ok(sum)
    }

    /// Equivalent to `MultiTermsEnum.totalTermFreq()`
    /// (`MultiTermsEnum.java:304-313`): the `long` sum over the subs on the
    /// current term.
    fn total_term_freq(&mut self) -> Result<i64> {
        let mut sum: i64 = 0;
        for i in 0..self.top.len() {
            let sub = self.top[i];
            let terms_enum = self.queue.subs_mut()[sub]
                .base
                .terms_enum_mut()
                .ok_or_else(|| null_sub_terms_enum("totalTermFreq()", "this.top[i].termsEnum"))?;
            let v = terms_enum.total_term_freq()?;
            debug_assert_ne!(v, -1);
            sum = sum.wrapping_add(v);
        }
        Ok(sum)
    }

    /// Equivalent to `MultiTermsEnum.postings(PostingsEnum, int)`
    /// (`MultiTermsEnum.java:315-350`).
    ///
    /// `reuse` is reused exactly when Java reuses it: when it is a
    /// [`MultiPostingsEnum`] ([`PostingsEnum::as_any_mut`] standing for
    /// `instanceof`) that this very enum created
    /// ([`MultiPostingsEnum::can_reuse`]); the box handed back is then the one
    /// that came in. The subs on the current term are sorted by sub index —
    /// in place, as Java's `ArrayUtil.timSort(top, 0, numTop, …)` sorts its
    /// `top` array — and each sub-enum is asked for its postings with the
    /// enum it returned last time for the same sub index as `reuse`
    /// (`docsEnum.subPostingsEnums[entry.subIndex]`).
    ///
    /// # Errors
    ///
    /// Propagates any error raised by a sub-enum's `postings`.
    fn postings(
        &mut self,
        reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Option<Box<dyn PostingsEnum>>> {
        // Can only reuse if incoming enum is also a MultiDocsEnum ... and was
        // previously created w/ this MultiTermsEnum:
        let mut docs_enum: Box<dyn PostingsEnum> = match reuse {
            // `reuse != null && reuse instanceof MultiPostingsEnum`
            Some(mut reuse) => match reuse
                .as_any_mut()
                .and_then(|any| any.downcast_mut::<MultiPostingsEnum>())
            {
                // ... and was previously created w/ this MultiTermsEnum:
                Some(docs_enum) if docs_enum.can_reuse(self) => reuse,
                _ => Box::new(MultiPostingsEnum::new(self, self.queue.subs().len())),
            },
            None => Box::new(MultiPostingsEnum::new(self, self.queue.subs().len())),
        };
        let multi = docs_enum
            .as_any_mut()
            .and_then(|any| any.downcast_mut::<MultiPostingsEnum>())
            .ok_or_else(|| {
                LuceneError::illegal_state(
                    "INVARIANT: the enum was just checked or built as a MultiPostingsEnum"
                        .to_string(),
                )
            })?;
        // Rust-only: the live sub-enums go back to `subPostingsEnums`, which
        // in Java still holds them alongside the live slots.
        multi.release_live_subs();

        // `ArrayUtil.timSort(top, 0, numTop, (o1, o2) -> o1.subIndex - o2.subIndex)`
        // is stable, as `sort_by_key` is.
        let subs = self.queue.subs();
        self.top.sort_by_key(|&sub| subs[sub].base.sub_index());

        let num_subs = self.queue.subs().len();
        let mut sub_docs: Vec<(EnumWithSlice, usize)> = Vec::with_capacity(self.top.len());
        for i in 0..self.top.len() {
            let sub = self.top[i];
            let entry = &mut self.queue.subs_mut()[sub];
            let sub_index = entry.base.sub_index();
            debug_assert!(
                sub_index < multi.sub_postings_enums.len(),
                "{sub_index} vs {}; {num_subs}",
                multi.sub_postings_enums.len(),
            );
            let reuse_sub = multi
                .sub_postings_enums
                .get_mut(sub_index)
                .and_then(Option::take);
            let sub_postings_enum = entry
                .base
                .terms_enum_mut()
                .ok_or_else(|| {
                    null_sub_terms_enum(
                        "postings(org.apache.lucene.index.PostingsEnum, int)",
                        "entry.termsEnum",
                    )
                })?
                .postings(reuse_sub, flags)?;
            debug_assert!(sub_postings_enum.is_some());
            let sub_postings_enum = sub_postings_enum.ok_or_else(LuceneError::null_pointer)?;
            // Java stores the enum both in `subPostingsEnums[subIndex]` and in
            // `subDocs[upto]`; here it lives in the slot, and goes back to
            // `subPostingsEnums` on the next reuse.
            sub_docs.push((
                EnumWithSlice::new(sub_postings_enum, entry.sub_slice),
                sub_index,
            ));
        }

        // `docsEnum.reset(subDocs, upto)`
        let (sub_docs, sub_indices): (Vec<EnumWithSlice>, Vec<usize>) =
            sub_docs.into_iter().unzip();
        multi.reset(sub_docs);
        // Rust-only: the `subPostingsEnums` slot each live enum returns to.
        multi.live_sub_indices = sub_indices.into_iter().map(Some).collect();
        Ok(Some(docs_enum))
    }

    /// Equivalent to `MultiTermsEnum.impacts(int)`: implemented to not fail
    /// `CheckIndex`, but one shouldn't be using impacts on a slow reader.
    fn impacts(&mut self, flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        Ok(Box::new(SlowImpactsEnum::new(
            self.postings(None, flags)?
                .ok_or_else(LuceneError::null_pointer)?,
        )))
    }

    /// Equivalent to `MultiTermsEnum.next()` (`MultiTermsEnum.java:265-293`).
    ///
    /// # Errors
    ///
    /// Propagates any error raised while moving a sub-enum. After a
    /// [`seek_exact`](TermsEnum::seek_exact) that found nothing on an enum that
    /// never had a term, Java re-seeks to its `null` `current` and throws
    /// `NullPointerException` with the JDK's message.
    fn next(&mut self) -> Result<Option<BytesRef>> {
        if self.last_seek_exact {
            // Must seekCeil at this point, so those subs that didn't have the
            // term can find the following term. NOTE: we could save some CPU
            // by only seekCeil the subs that didn't match the last exact
            // seek... but most impls short-circuit if you seekCeil to term they
            // are already on.
            let Some(current) = self.current else {
                // `seekCeil(null)`: its first statements run, then
                // `lastSeekScratch.copyBytes(term)` dereferences the `null`.
                self.queue.clear();
                self.top.clear();
                self.last_seek_exact = false;
                return Err(LuceneError::null_pointer_with_message(
                    "Cannot read field \"bytes\" because \"ref\" is null".to_string(),
                ));
            };
            // Java passes `current` itself; this is a copy of the same bytes.
            let current = self.sub_term(current)?.ok_or_else(|| {
                LuceneError::null_pointer_with_message(
                    "Cannot read field \"bytes\" because \"ref\" is null".to_string(),
                )
            })?;
            let status = self.seek_ceil(&current)?;
            debug_assert_eq!(status, SeekStatus::FOUND);
            self.last_seek_exact = false;
        }
        self.last_seek = None;

        // restore queue
        self.push_top()?;

        // gather equal top fields
        if self.queue.size() > 0 {
            // TODO: we could maybe defer this somewhat costly operation until
            // one of the APIs that needs to see the top is invoked (docFreq,
            // postings, etc.)
            self.pull_top();
        } else {
            self.current = None;
        }

        match self.current {
            Some(sub) => self.sub_term(sub),
            None => Ok(None),
        }
    }
}

// ---------------------------------------------------------------------------
// MultiPostingsEnum
// ---------------------------------------------------------------------------

/// A [`PostingsEnum`] paired with the [`ReaderSlice`] describing how its
/// sub-reader fits into the composite reader.
///
/// Equivalent to `MultiPostingsEnum.EnumWithSlice`. Java's `postingsEnum`
/// field may be `null`; it is `None` here while a
/// [`MappingMultiPostingsEnum`](crate::index::MappingMultiPostingsEnum) holds
/// the enum.
pub struct EnumWithSlice {
    /// Postings enum for this sub-reader, in the sub-reader's local doc-ID
    /// space.
    pub postings_enum: Option<Box<dyn PostingsEnum>>,
    /// Placement of this sub-reader inside the composite reader.
    pub slice: ReaderSlice,
}

impl std::fmt::Display for EnumWithSlice {
    /// Port of `MultiPostingsEnum.EnumWithSlice.toString()`
    /// (`MultiPostingsEnum.java:176-179`): the slice, a colon, then the
    /// sub-enum rendered by its own `toString()`, or `null`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.postings_enum {
            Some(postings_enum) => write!(f, "{}:{}", self.slice, postings_enum),
            None => write!(f, "{}:null", self.slice),
        }
    }
}

impl EnumWithSlice {
    /// Pairs `postings_enum` with `slice`.
    pub fn new(postings_enum: Box<dyn PostingsEnum>, slice: ReaderSlice) -> Self {
        Self {
            postings_enum: Some(postings_enum),
            slice,
        }
    }
}

impl std::fmt::Debug for EnumWithSlice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The postings enum itself has no Debug bound in Rucene.
        f.debug_struct("EnumWithSlice")
            .field("slice", &self.slice)
            .finish_non_exhaustive()
    }
}

/// The `NullPointerException` Java raises when `MultiPostingsEnum` reaches a
/// sub whose `postingsEnum` is `null`.
fn null_current_postings(method: &str) -> LuceneError {
    LuceneError::null_pointer_with_message(null_current_postings_message(method))
}

/// The JVM's helpful message for the `NullPointerException` of
/// `current.<method>` when `current` is `null`.
fn null_current_postings_message(method: &str) -> String {
    format!(
        "Cannot invoke \"org.apache.lucene.index.PostingsEnum.{method}\" because \"this.current\" is null"
    )
}

/// A [`PostingsEnum`] that concatenates several sub-readers' postings, offset
/// by each sub's slice `start`, producing a globally sorted doc-ID stream.
///
/// Equivalent to `org.apache.lucene.index.MultiPostingsEnum`. The slices
/// partition the global doc-ID space (each sub's local doc IDs are re-based by
/// `slice.start`), so a plain concatenation yields the same globally increasing
/// doc-ID sequence a merge would — without a per-doc comparison.
///
/// Java keeps every sub-enum in two arrays at once: `subPostingsEnums`, by sub
/// index, which `MultiTermsEnum.postings` offers back as `reuse`, and the live
/// `subs[0..numSubs]`. Rust gives each enum one owner: a live enum is owned by
/// its slot in `subs`, and returns to `sub_postings_enums` when this enum is
/// reused or reset; `sub_postings_enums` owns the rest.
///
/// **Divergence from Lucene 10.5.0.** Java's `subs` is pre-allocated with one
/// `EnumWithSlice` per sub-reader, and `getSubs()` and `toString()` expose the
/// whole array: past `numSubs` it holds the entries of earlier terms, whose
/// enums `subPostingsEnums` or a codec may have replaced since, and entries
/// that were never filled, whose `null` slice makes `toString()` throw. The
/// stale entries alias enums owned elsewhere, which Rust ownership cannot
/// express, so [`get_subs`](Self::get_subs) and `Display` cover the live
/// entries only — the part every Lucene caller reads, bounded by
/// [`get_num_subs`](Self::get_num_subs).
pub struct MultiPostingsEnum {
    /// Identity of the [`MultiTermsEnum`] that created this instance.
    /// Equivalent to Java's `parent` field, which holds the creating
    /// `MultiTermsEnum` itself.
    parent: u64,
    /// Java's `final PostingsEnum[] subPostingsEnums`, indexed by sub index:
    /// the enum each sub returned last, minus the live ones.
    sub_postings_enums: Vec<Option<Box<dyn PostingsEnum>>>,
    /// Java's `subs[0..numSubs]`, in ascending slice order.
    subs: Vec<EnumWithSlice>,
    /// The sub index each live slot's enum belongs to in
    /// `sub_postings_enums`, or `None` for an enum [`reset`](Self::reset)
    /// received from outside a [`MultiTermsEnum`]. Rust-only bookkeeping for
    /// the one-owner rule above.
    live_sub_indices: Vec<Option<usize>>,
    /// Java's `upto`: index into `subs` of the current sub, or `-1`.
    upto: i32,
    /// Java's `current`: the slot whose enum is current, or `None` for `null`.
    current: Option<usize>,
    /// Java's `currentBase`.
    current_base: i32,
    /// Java's `doc`: the current global doc ID (`-1` before the first
    /// positioning).
    doc: i32,
}

impl MultiPostingsEnum {
    /// Creates an enum for `parent` over `sub_reader_count` sub-readers.
    ///
    /// Equivalent to `MultiPostingsEnum(MultiTermsEnum parent, int
    /// subReaderCount)` (`MultiPostingsEnum.java:44-51`). The enum has no live
    /// sub until [`reset`](Self::reset).
    pub fn new(parent: &MultiTermsEnum, sub_reader_count: usize) -> Self {
        Self {
            parent: parent.id,
            sub_postings_enums: (0..sub_reader_count).map(|_| None).collect(),
            subs: Vec::with_capacity(sub_reader_count),
            live_sub_indices: Vec::with_capacity(sub_reader_count),
            upto: -1,
            current: None,
            current_base: 0,
            doc: -1,
        }
    }

    /// Returns `true` if this instance can be reused by `parent`.
    ///
    /// Equivalent to `org.apache.lucene.index.MultiPostingsEnum#canReuse`,
    /// which answers `this.parent == parent`: an instance may only be reused by
    /// the very enum that built it, because its sub-enum array is sized and
    /// indexed for that enum's sub-readers. The identity here is the number
    /// [`MultiTermsEnum`] carries in place of the object reference Java
    /// compares.
    pub fn can_reuse(&self, parent: &MultiTermsEnum) -> bool {
        self.parent == parent.id
    }

    /// Re-uses and resets this instance on `subs`, returning it positioned
    /// before the first document.
    ///
    /// Equivalent to `org.apache.lucene.index.MultiPostingsEnum#reset`
    /// (`MultiPostingsEnum.java:59-69`). Java copies the incoming slices into
    /// its pre-allocated array and records how many are live in `numSubs`;
    /// this port's live slots are a `Vec`, so the count is its length. Java
    /// returns `this`, which is `&mut Self` here.
    ///
    /// The enums the live slots held go back to `subPostingsEnums` first, where
    /// Java's array still holds them. The new slots carry no sub index:
    /// [`MultiTermsEnum::postings`] records the one of each enum it obtained
    /// right after calling this.
    pub fn reset(&mut self, subs: Vec<EnumWithSlice>) -> &mut Self {
        self.release_live_subs();
        self.live_sub_indices = vec![None; subs.len()];
        self.subs = subs;
        self.upto = -1;
        self.doc = -1;
        self.current = None;
        self
    }

    /// Moves every live enum back to its slot in `sub_postings_enums`, where
    /// Java's `subPostingsEnums` still holds it. An enum with no known sub
    /// index has no such slot: Java's array never held it.
    fn release_live_subs(&mut self) {
        let subs = std::mem::take(&mut self.subs);
        let sub_indices = std::mem::take(&mut self.live_sub_indices);
        for (sub, sub_index) in subs.into_iter().zip(sub_indices) {
            if let (Some(postings_enum), Some(sub_index)) = (sub.postings_enum, sub_index) {
                if let Some(slot) = self.sub_postings_enums.get_mut(sub_index) {
                    *slot = Some(postings_enum);
                }
            }
        }
        self.current = None;
    }

    /// Returns how many sub-readers are being merged.
    ///
    /// Equivalent to `MultiPostingsEnum.getNumSubs()`.
    pub fn get_num_subs(&self) -> usize {
        self.subs.len()
    }

    /// Returns the sub-readers being merged.
    ///
    /// Equivalent to `MultiPostingsEnum.getSubs()`; see the type's divergence
    /// note for why only the live entries are returned.
    pub fn get_subs(&self) -> &[EnumWithSlice] {
        &self.subs
    }

    /// Mutable counterpart of [`get_subs`](Self::get_subs).
    ///
    /// Java's `getSubs()` hands out the array itself, whose `EnumWithSlice`
    /// fields are public and assignable; `MappingMultiPostingsEnum` reads its
    /// enums out of it.
    pub fn get_subs_mut(&mut self) -> &mut [EnumWithSlice] {
        &mut self.subs
    }

    /// Returns the enum of the current sub, or Java's `NullPointerException`.
    fn current_enum(&self, method: &str) -> Result<&dyn PostingsEnum> {
        self.current
            .and_then(|i| self.subs[i].postings_enum.as_deref())
            .ok_or_else(|| null_current_postings(method))
    }

    /// Mutable counterpart of [`current_enum`](Self::current_enum).
    fn current_enum_mut(&mut self, method: &str) -> Result<&mut dyn PostingsEnum> {
        match self.current {
            Some(i) => match &mut self.subs[i].postings_enum {
                Some(postings_enum) => Ok(postings_enum.as_mut()),
                None => Err(null_current_postings(method)),
            },
            None => Err(null_current_postings(method)),
        }
    }

    /// Moves to the next live slot: Java's `upto++; current =
    /// subs[upto].postingsEnum; currentBase = subs[upto].slice.start();`.
    fn step_to_next_sub(&mut self) {
        self.upto += 1;
        let slot = self.upto as usize;
        self.current = self.subs[slot].postings_enum.as_ref().map(|_| slot);
        self.current_base = self.subs[slot].slice.start;
    }
}

impl DocIdSetIterator for MultiPostingsEnum {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    /// Equivalent to `MultiPostingsEnum.nextDoc()`
    /// (`MultiPostingsEnum.java:123-143`).
    fn next_doc(&mut self) -> Result<i32> {
        loop {
            if self.current.is_none() {
                if self.upto == self.subs.len() as i32 - 1 {
                    self.doc = crate::search::NO_MORE_DOCS;
                    return Ok(self.doc);
                }
                self.step_to_next_sub();
            }

            // Java dereferences `current` here, `null` included.
            let doc = self.current_enum_mut("nextDoc()")?.next_doc()?;
            if doc != crate::search::NO_MORE_DOCS {
                self.doc = self.current_base + doc;
                return Ok(self.doc);
            }
            self.current = None;
        }
    }

    /// Equivalent to `MultiPostingsEnum.advance(int)`
    /// (`MultiPostingsEnum.java:96-121`). Unlike `nextDoc`, a slot whose enum is
    /// `null` is stepped over, as Java's loop tests `current != null` first.
    fn advance(&mut self, target: i32) -> Result<i32> {
        debug_assert!(target > self.doc);
        loop {
            if let Some(i) = self.current {
                let current_base = self.current_base;
                let current = self.subs[i]
                    .postings_enum
                    .as_mut()
                    .ok_or_else(|| null_current_postings("advance(int)"))?;
                let doc = if target < current_base {
                    // target was in the previous slice but there was no
                    // matching doc after it
                    current.next_doc()?
                } else {
                    current.advance(target - current_base)?
                };
                if doc == crate::search::NO_MORE_DOCS {
                    self.current = None;
                } else {
                    self.doc = doc + current_base;
                    return Ok(self.doc);
                }
            } else if self.upto == self.subs.len() as i32 - 1 {
                self.doc = crate::search::NO_MORE_DOCS;
                return Ok(self.doc);
            } else {
                self.step_to_next_sub();
            }
        }
    }

    /// Equivalent to `MultiPostingsEnum.cost()`: the sum over the live subs.
    /// A slot whose enum another holder has taken contributes nothing.
    fn cost(&self) -> i64 {
        let mut cost: i64 = 0;
        for sub in &self.subs {
            if let Some(postings_enum) = &sub.postings_enum {
                cost += postings_enum.cost();
            }
        }
        cost
    }
}

impl std::fmt::Display for MultiPostingsEnum {
    /// Port of `MultiPostingsEnum.toString()`
    /// (`MultiPostingsEnum.java:191-194`).
    ///
    /// **The class renames itself**: the string opens with
    /// `MultiDocsAndPositionsEnum`, the name this class carried before docs and
    /// positions were merged into one enumeration — not `MultiPostingsEnum`.
    /// Measured against `lucene-core-10.5.0.jar`. Java renders the whole
    /// pre-allocated array; see the type's divergence note.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MultiDocsAndPositionsEnum([")?;
        for (i, sub) in self.subs.iter().enumerate() {
            if i != 0 {
                f.write_str(", ")?;
            }
            std::fmt::Display::fmt(sub, f)?;
        }
        f.write_str("])")
    }
}

impl PostingsEnum for MultiPostingsEnum {
    /// Equivalent to `MultiPostingsEnum.freq()`, which asserts and then
    /// dereferences `current`.
    fn freq(&self) -> Result<i32> {
        debug_assert!(self.current.is_some());
        self.current_enum("freq()")?.freq()
    }

    fn next_position(&mut self) -> Result<i32> {
        self.current_enum_mut("nextPosition()")?.next_position()
    }

    /// Equivalent to `MultiPostingsEnum.startOffset()`
    /// (`MultiPostingsEnum.java:151-153`), which dereferences `current`.
    ///
    /// # Panics
    ///
    /// With Java's `NullPointerException` when `current` is `null`.
    fn start_offset(&self) -> i32 {
        match self.current_enum("startOffset()") {
            Ok(current) => current.start_offset(),
            Err(_) => throw_unchecked(
                "java.lang.NullPointerException",
                Some(&null_current_postings_message("startOffset()")),
            ),
        }
    }

    /// Equivalent to `MultiPostingsEnum.endOffset()`
    /// (`MultiPostingsEnum.java:156-158`), which dereferences `current`.
    ///
    /// # Panics
    ///
    /// With Java's `NullPointerException` when `current` is `null`.
    fn end_offset(&self) -> i32 {
        match self.current_enum("endOffset()") {
            Ok(current) => current.end_offset(),
            Err(_) => throw_unchecked(
                "java.lang.NullPointerException",
                Some(&null_current_postings_message("endOffset()")),
            ),
        }
    }

    fn get_payload(&self) -> Result<Option<&[u8]>> {
        self.current_enum("getPayload()")?.get_payload()
    }

    /// This enum answers with itself: it is the `true` arm of Java's
    /// `reuse instanceof MultiPostingsEnum` (`MultiTermsEnum.java:320`) and of
    /// the cast `(MultiPostingsEnum) in.postings(...)`
    /// (`MappedMultiFields.java:129-131`).
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
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

impl std::fmt::Display for SlowImpactsEnum {
    /// `org.apache.lucene.index.SlowImpactsEnum` declares no `toString()` in Lucene 10.5.0, so
    /// `java.lang.Object`'s applies: the binary class name, `@`, and the
    /// identity hash. See
    /// [`java_object_to_string`](crate::index::index_reader::java_object_to_string)
    /// for why only the hexadecimal suffix diverges.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&crate::index::index_reader::java_object_to_string(
            "org.apache.lucene.index.SlowImpactsEnum",
            self,
        ))
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
    use crate::index::terms::EmptyFields;
    use crate::index::terms::{EmptyTerms, Fields, SeekStatus, Terms, TermsEnum};
    use crate::index::{MultiReader, ReaderSlice};
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

    impl std::fmt::Display for VecPostings {
        /// Test double; it renders under its own name. `Display` is required
        /// because `TermsEnum`/`PostingsEnum` carry it as a supertrait, standing in
        /// for the `toString()` every Java enumeration inherits from `Object`.
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("VecPostings")
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

        /// The fake reuses itself as a codec's enum does, so that the tests
        /// can see which enum `MultiTermsEnum` offers each sub as `reuse`.
        fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
            Some(self)
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

    impl std::fmt::Display for VecTermsEnum {
        /// Test double; it renders under its own name. `Display` is required
        /// because `TermsEnum`/`PostingsEnum` carry it as a supertrait, standing in
        /// for the `toString()` every Java enumeration inherits from `Object`.
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("VecTermsEnum")
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

        fn term(&self) -> Result<Option<BytesRef>> {
            Ok(Some(self.entries[self.pos as usize].0.clone()))
        }

        fn ord(&self) -> Result<i64> {
            Ok(self.pos as i64)
        }

        fn doc_freq(&mut self) -> Result<i32> {
            Ok(self.entries[self.pos as usize].1)
        }

        fn total_term_freq(&mut self) -> Result<i64> {
            Ok(self.entries[self.pos as usize].2)
        }

        /// Resets `reuse` in place when it is a [`VecPostings`], as a codec
        /// reuses its own enum, and builds a new one otherwise.
        fn postings(
            &mut self,
            reuse: Option<Box<dyn PostingsEnum>>,
            _flags: i32,
        ) -> Result<Option<Box<dyn PostingsEnum>>> {
            let docs = self.entries[self.pos as usize].3.clone();
            if let Some(mut reuse) = reuse {
                if let Some(postings) = reuse
                    .as_any_mut()
                    .and_then(|any| any.downcast_mut::<VecPostings>())
                {
                    *postings = VecPostings::new(docs);
                    return Ok(Some(reuse));
                }
            }
            Ok(Some(Box::new(VecPostings::new(docs))))
        }

        fn impacts(&mut self, _flags: i32) -> Result<Box<dyn ImpactsEnum>> {
            Err(LuceneError::illegal_state("not used".to_string()))
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

        fn size(&self) -> Result<i64> {
            Ok(self.entries.len() as i64)
        }

        fn sum_total_term_freq(&self) -> Result<i64> {
            Ok(self.entries.iter().map(|(_, _, ttf, _)| *ttf).sum())
        }

        fn sum_doc_freq(&self) -> Result<i64> {
            Ok(self.entries.iter().map(|(_, df, _, _)| *df as i64).sum())
        }

        fn doc_count(&self) -> Result<i32> {
            Ok({
                // Number of distinct docs across all terms (approximation enough
                // for the tests; not the focus here).
                self.entries
                    .iter()
                    .flat_map(|(_, _, _, docs)| docs.iter().copied())
                    .collect::<std::collections::BTreeSet<_>>()
                    .len() as i32
            })
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
        let mut pe = te.postings(None, POSTINGS_ENUM_FREQS).unwrap().unwrap();
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
        let mut pe = te.postings(None, POSTINGS_ENUM_FREQS).unwrap().unwrap();
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
        assert_eq!(te.term().unwrap().unwrap().slice(), b"mango");

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
        assert_eq!(te.term().unwrap().unwrap().slice(), b"cherry");
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
        assert_eq!(mt.sum_total_term_freq().unwrap(), 11);
        // sumDocFreq = (2+1) + (1+3) = 7
        assert_eq!(mt.sum_doc_freq().unwrap(), 7);
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

    impl std::fmt::Display for StubLeaf {
        /// Test double; it has no Lucene counterpart whose `toString()` could be
        /// reproduced, so it renders under its own name. `Display` is required
        /// because `LeafReader`/`IndexReader` carry it as a supertrait, standing in
        /// for the `toString()` every Java reader inherits from `Object`.
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("StubLeaf")
        }
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
            Err(LuceneError::unsupported_operation("stub".to_string()))
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
        let mut boxed = enumerator
            .postings(None, POSTINGS_ENUM_FREQS)
            .unwrap()
            .unwrap();
        let postings = as_multi(&mut boxed);
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
        assert_eq!(postings.get_subs().len(), 2);

        assert_eq!(enumerator.next().unwrap(), Some(term(b"b")));
        let mut boxed = enumerator
            .postings(None, POSTINGS_ENUM_FREQS)
            .unwrap()
            .unwrap();
        let postings = as_multi(&mut boxed);
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
        let mut postings = enumerator
            .postings(None, POSTINGS_ENUM_FREQS)
            .unwrap()
            .unwrap();
        assert_eq!(postings.next_doc().unwrap(), 0);
        assert_eq!(postings.next_doc().unwrap(), 7);
        assert_eq!(postings.next_doc().unwrap(), crate::search::NO_MORE_DOCS);
    }

    // ----- LUCENE-2130 seek-optimisation regression tests -----
    //
    // `MultiTermsEnum.seekExact` deliberately clears `lastSeek`
    // (`MultiTermsEnum.java:125`), which disables the LUCENE-2130 skip on the
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

    /// Narrows the postings `MultiTermsEnum::postings` hands out to the
    /// [`MultiPostingsEnum`] they are, as Java casts them.
    fn as_multi(postings: &mut Box<dyn PostingsEnum>) -> &mut MultiPostingsEnum {
        postings
            .as_any_mut()
            .and_then(|any| any.downcast_mut::<MultiPostingsEnum>())
            .expect("MultiTermsEnum hands out a MultiPostingsEnum")
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
        let mut postings = te.postings(None, POSTINGS_ENUM_FREQS).unwrap().unwrap();
        assert_eq!(drain_docs(postings.as_mut()), vec![1, 11]);
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
        let mut postings = te.postings(None, POSTINGS_ENUM_FREQS).unwrap().unwrap();
        assert_eq!(
            drain_docs(postings.as_mut()),
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
        assert_eq!(te.term().unwrap(), Some(term(b"a")));
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
        let mut postings = te.postings(None, POSTINGS_ENUM_FREQS).unwrap().unwrap();
        assert_eq!(drain_docs(postings.as_mut()), vec![1, 11]);
        // Exhausted: every further call keeps reporting NO_MORE_DOCS and never
        // rewinds into an earlier sub.
        // (`advance` below the current document breaks Java's `assert target
        // > doc`, so only `nextDoc` is called again.)
        assert_eq!(postings.doc_id(), crate::search::NO_MORE_DOCS);
        assert_eq!(postings.next_doc().unwrap(), crate::search::NO_MORE_DOCS);
        assert_eq!(postings.doc_id(), crate::search::NO_MORE_DOCS);
        assert_eq!(postings.next_doc().unwrap(), crate::search::NO_MORE_DOCS);
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

    /// rmp #306: a leaf lists its indexed fields in the block tree's `TreeMap`
    /// order, and `MultiFields` merges the leaves' lists with a
    /// `MergedIterator<String>`: both order by `String.compareTo`, UTF-16 code
    /// units. Oracle `scratchpad/fixF/oracle/Utf16Oracle.java`: `TreeMap keys
    /// [a, b😀, b\u{E001}, \u{10000}, \u{E000}, \u{FF21}, \u{FFFD}]`, and
    /// "across the boundary" `[a, \u{10000}, \u{E000}, \u{FFFD}]` for the merge.
    #[test]
    fn field_names_are_ordered_as_java_string_compare_to() {
        use crate::index::field_infos::FieldInfo;
        use crate::index::IndexOptions;
        let names = [
            "a",
            "\u{E000}",
            "\u{FFFD}",
            "\u{10000}",
            "\u{FF21}",
            "b\u{1F600}",
            "b\u{E001}",
        ];
        let infos = |names: &[&str]| {
            FieldInfos::new(
                names
                    .iter()
                    .enumerate()
                    .map(|(number, name)| {
                        let mut info = FieldInfo::default();
                        info.name = name.to_string();
                        info.number = number as i32;
                        info.index_options = IndexOptions::DOCS;
                        info
                    })
                    .collect(),
            )
            .unwrap()
        };
        let java_order = vec![
            "a",
            "b\u{1F600}",
            "b\u{E001}",
            "\u{10000}",
            "\u{E000}",
            "\u{FF21}",
            "\u{FFFD}",
        ];

        let leaf: Arc<dyn LeafReader> =
            Arc::new(StubLeaf::with_fields(1, 1, infos(&names), Vec::new()));
        let listed: Vec<String> = LeafFields::new(leaf).iterator().collect();
        assert_eq!(listed, java_order);

        // Each leaf lists its names in UTF-16 order, and the merge compares the
        // leaves' heads across the boundary: Utf16Oracle's "across the
        // boundary" `[a, \u{10000}, \u{E000}, \u{FFFD}]`.
        let first: Arc<dyn IndexReader> = Arc::new(StubLeaf::with_fields(
            1,
            1,
            infos(&["\u{10000}", "a"]),
            Vec::new(),
        ));
        let second: Arc<dyn IndexReader> = Arc::new(StubLeaf::with_fields(
            1,
            1,
            infos(&["\u{FFFD}", "\u{E000}"]),
            Vec::new(),
        ));
        let multi: Arc<dyn IndexReader> =
            Arc::new(MultiReader::new(vec![first, second], true).unwrap());
        let merged: Vec<String> = MultiFields::get_fields(&multi)
            .unwrap()
            .iterator()
            .collect();
        assert_eq!(merged, vec!["a", "\u{10000}", "\u{E000}", "\u{FFFD}"]);
    }

    // ----- MultiFields.iterator() is a MergedIterator (MultiFields.java:51-58) -----

    /// `Fields` whose iterator counts the names it hands out.
    struct PullCountingFields {
        names: Vec<String>,
        pulled: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl Fields for PullCountingFields {
        fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
            let pulled = Arc::clone(&self.pulled);
            Box::new(self.names.iter().cloned().inspect(move |_| {
                pulled.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }))
        }

        fn terms(&self, _field: &str) -> Result<Option<Box<dyn Terms>>> {
            Ok(None)
        }

        fn size(&self) -> i32 {
            self.names.len() as i32
        }
    }

    /// rmp #306: `MultiFields.iterator()` merges the sorted sub-iterators with
    /// a `MergedIterator`, which takes the first name of each sub up front
    /// and advances a sub only once its current name has been handed out
    /// (`MergedIterator.java:66-82, 110-138`); equal names across subs come out
    /// once.
    #[test]
    fn iterator_merges_the_subs_lazily_with_a_merged_iterator() {
        let pulled = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let sub = |names: &[&str]| -> Box<dyn Fields> {
            Box::new(PullCountingFields {
                names: names.iter().map(|name| name.to_string()).collect(),
                pulled: Arc::clone(&pulled),
            })
        };
        let mf = MultiFields::new(
            vec![sub(&["a", "c", "e"]), sub(&["b", "c", "f"])],
            vec![ReaderSlice::new(0, 5, 0), ReaderSlice::new(5, 5, 1)],
        );
        let mut it = mf.iterator();
        assert_eq!(it.next().as_deref(), Some("a"));
        assert_eq!(
            pulled.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "one name from each sub to prime the merge, none more"
        );
        let rest: Vec<String> = it.collect();
        assert_eq!(rest, vec!["b", "c", "e", "f"]);
        assert_eq!(pulled.load(std::sync::atomic::Ordering::SeqCst), 6);
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
        assert_eq!(Terms::doc_count(&shared).unwrap(), 2);
        assert_eq!(Terms::sum_doc_freq(&shared).unwrap(), 2);
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
    fn a_failed_seek_exact_reads_the_sub_that_supplied_the_previous_term() {
        // `MultiTermsEnum.seekExact` only assigns `current` for a sub that
        // holds the term (`MultiTermsEnum.java:154-158`), so after a miss
        // `term()` reads the sub that supplied the previous term — here leaf
        // 0, which this in-memory enum does not move on a miss — and no sub is
        // on it. `MultiTermsOracle` over the same subs: `term=a docFreq=0
        // next=b`.
        let mt = two_leaves_with_a_shared_trailing_term();
        let mut te = mt.multi_iterator().unwrap().unwrap();
        assert!(te.seek_exact(&term(b"a")).unwrap());
        assert_eq!(te.term().unwrap(), Some(term(b"a")));

        assert!(!te.seek_exact(&term(b"bb")).unwrap());
        assert_eq!(te.term().unwrap(), Some(term(b"a")), "leaf 0's term");
        assert_eq!(te.doc_freq().unwrap(), 0, "no sub holds `bb`");
        // `next()` re-seeks to that term first (`MultiTermsEnum.java:267-276`),
        // so iteration resumes after `a`.
        assert_eq!(te.next().unwrap(), Some(term(b"b")));
    }

    /// Two committed Lucene104 segments holding `{aaaa, cccc}` and
    /// `{bbbb, cccc}` in the `StringField` `f` (`StaleCurrentOracle`).
    fn stale_current_reader() -> Arc<dyn IndexReader> {
        use crate::document::{Document, Store, StringField};
        use crate::index::index_writer::IndexWriter;
        use crate::index::index_writer_config::IndexWriterConfig;
        let _ = crate::codecs::register_codec("Lucene104", crate::codecs::Lucene104Codec::new());
        let dir: Arc<dyn crate::store::Directory> =
            Arc::new(crate::store::ByteBuffersDirectory::new());
        let config = IndexWriterConfig::new();
        config
            .set_merge_policy(Some(crate::index::merge_policy::NoMergePolicy::instance()))
            .unwrap();
        let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
        for segment in [["aaaa", "cccc"], ["bbbb", "cccc"]] {
            for text in segment {
                let mut doc = Document::new();
                doc.add(Box::new(
                    StringField::new("f", text.to_string(), Store::NO).unwrap(),
                ));
                writer.add_document(&doc).unwrap();
            }
            writer.commit().unwrap();
        }
        writer.close().unwrap();
        crate::index::directory_reader::open(dir).unwrap() as Arc<dyn IndexReader>
    }

    /// Java's `current` is the term buffer of the sub that supplied it, so
    /// after a missed `seekExact` it shows where the miss moved that sub's
    /// `SegmentTermsEnum`, and `next()` re-seeks to that term and steps past
    /// it. Measured against `lucene-core-10.5.0.jar` (`StaleCurrentOracle`):
    /// `seekExact aaaa=true term=aaaa`, `seekExact aazz=false term=cccc`,
    /// `next=null`.
    #[test]
    fn a_missed_seek_exact_reads_the_term_the_miss_moved_the_sub_to() {
        let reader = stale_current_reader();
        let terms = MultiTerms::get_terms(&reader, "f").unwrap().unwrap();
        let mut te = terms.iterator().unwrap();
        assert!(te.seek_exact(&term(b"aaaa")).unwrap());
        assert_eq!(te.term().unwrap(), Some(term(b"aaaa")));
        assert!(!te.seek_exact(&term(b"aazz")).unwrap());
        assert_eq!(
            te.term().unwrap(),
            Some(term(b"cccc")),
            "leaf 0 scanned to `cccc`"
        );
        assert_eq!(te.next().unwrap(), None);
    }

    /// After a `seekCeil` that ends past the last term, Java's `current` is
    /// the supplying sub's term buffer, which that `SegmentTermsEnum` emptied.
    /// Measured against `lucene-core-10.5.0.jar` (`StaleCurrentOracle`):
    /// `seekCeil aaaa=FOUND`, `seekCeil zzzz=END term=` (the empty term).
    #[test]
    fn a_seek_ceil_past_the_end_reads_the_term_the_sub_was_left_on() {
        let reader = stale_current_reader();
        let terms = MultiTerms::get_terms(&reader, "f").unwrap().unwrap();
        let mut te = terms.iterator().unwrap();
        assert_eq!(te.seek_ceil(&term(b"aaaa")).unwrap(), SeekStatus::FOUND);
        assert_eq!(te.seek_ceil(&term(b"zzzz")).unwrap(), SeekStatus::END);
        assert_eq!(te.term().unwrap(), Some(term(b"")));
    }

    /// The JDK's messages for the `NullPointerException`s of a sub with a
    /// `null` `termsEnum` on the current term, and of `pushTop` over an empty
    /// queue. Measured against `lucene-core-10.5.0.jar` (`NpeTextOracle`,
    /// which nulls `top[0].termsEnum` and clears the queue by reflection).
    #[test]
    fn null_sub_enums_and_an_empty_queue_report_javas_npe_texts() {
        let npe = |message: &str| LuceneError::null_pointer_with_message(message).to_string();
        let mt = two_leaves_with_a_shared_trailing_term();

        let mut te = mt.multi_iterator().unwrap().unwrap();
        te.next().unwrap();
        let top = te.top[0];
        te.queue.subs_mut()[top]
            .base
            .reset(TermsEnumIndex::with_optional_enum(None, top));
        assert_eq!(
            te.doc_freq().unwrap_err().to_string(),
            npe("Cannot invoke \"org.apache.lucene.index.TermsEnum.docFreq()\" because \"this.top[i].termsEnum\" is null")
        );
        assert_eq!(
            te.total_term_freq().unwrap_err().to_string(),
            npe("Cannot invoke \"org.apache.lucene.index.TermsEnum.totalTermFreq()\" because \"this.top[i].termsEnum\" is null")
        );
        assert_eq!(
            te.postings(None, POSTINGS_ENUM_FREQS).err().unwrap().to_string(),
            npe("Cannot invoke \"org.apache.lucene.index.TermsEnum.postings(org.apache.lucene.index.PostingsEnum, int)\" because \"entry.termsEnum\" is null")
        );

        let mut te = mt.multi_iterator().unwrap().unwrap();
        te.next().unwrap();
        te.queue.clear();
        assert_eq!(
            te.next().unwrap_err().to_string(),
            npe("Cannot invoke \"org.apache.lucene.index.MultiTermsEnum$TermsEnumWithSlice.next()\" because \"top\" is null")
        );
    }

    #[test]
    fn next_after_a_failed_seek_exact_on_a_fresh_enum_reports_javas_npe() {
        // A fresh enum has no `current`; `next()` after a missed seekExact
        // calls `seekCeil(null)`, whose `lastSeekScratch.copyBytes(term)`
        // dereferences it.
        let mt = two_leaves_with_a_shared_trailing_term();
        let mut te = mt.multi_iterator().unwrap().unwrap();
        assert!(!te.seek_exact(&term(b"bb")).unwrap());
        let err = te.next().unwrap_err();
        assert!(
            err.to_string()
                .contains("Cannot read field \"bytes\" because \"ref\" is null"),
            "unexpected error: {err:?}"
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
        let mut postings = te.postings(None, POSTINGS_ENUM_FREQS).unwrap().unwrap();
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
        assert_eq!(terms.size().unwrap(), 1);
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

    // ----- #276 / #277: Lucene's structure, reuse and heap order -----

    /// The address of the object behind a reference, for identity checks:
    /// Java's `==` on references.
    fn identity<T: ?Sized>(object: &T) -> *const () {
        object as *const T as *const ()
    }

    /// The identity of the enum in live slot `slot` of a merged postings.
    fn sub_identity(postings: &mut Box<dyn PostingsEnum>, slot: usize) -> *const () {
        identity(
            as_multi(postings).get_subs()[slot]
                .postings_enum
                .as_deref()
                .expect("a live slot holds its enum"),
        )
    }

    #[test]
    fn postings_reuses_the_multi_postings_enum_it_created_and_offers_each_sub_its_own_enum() {
        // leaf 0 = {a, c}, leaf 1 = {b, c}
        let mt = two_leaves_with_a_shared_trailing_term();
        let mut te = mt.multi_iterator().unwrap().unwrap();

        assert_eq!(te.next().unwrap(), Some(term(b"a")));
        let mut postings = te.postings(None, POSTINGS_ENUM_FREQS).unwrap().unwrap();
        let multi = identity(postings.as_ref());
        let leaf0 = sub_identity(&mut postings, 0);

        assert_eq!(te.next().unwrap(), Some(term(b"b")));
        let mut postings = te
            .postings(Some(postings), POSTINGS_ENUM_FREQS)
            .unwrap()
            .unwrap();
        assert_eq!(
            identity(postings.as_ref()),
            multi,
            "`reuse instanceof MultiPostingsEnum && canReuse(this)`: the same object comes back"
        );
        let leaf1 = sub_identity(&mut postings, 0);
        assert_ne!(leaf1, leaf0);

        assert_eq!(te.next().unwrap(), Some(term(b"c")));
        let mut postings = te
            .postings(Some(postings), POSTINGS_ENUM_FREQS)
            .unwrap()
            .unwrap();
        assert_eq!(identity(postings.as_ref()), multi);
        assert_eq!(
            (
                sub_identity(&mut postings, 0),
                sub_identity(&mut postings, 1)
            ),
            (leaf0, leaf1),
            "each sub is offered `subPostingsEnums[subIndex]`, leaf 0's from two terms back"
        );
        assert_eq!(drain_docs(postings.as_mut()), vec![1, 11]);
    }

    #[test]
    fn postings_does_not_reuse_a_multi_postings_enum_another_enum_created() {
        let mt = two_leaves_with_a_shared_trailing_term();
        let mut first = mt.multi_iterator().unwrap().unwrap();
        let mut second = mt.multi_iterator().unwrap().unwrap();
        assert_eq!(first.next().unwrap(), Some(term(b"a")));
        assert_eq!(second.next().unwrap(), Some(term(b"a")));

        let foreign = first.postings(None, POSTINGS_ENUM_FREQS).unwrap().unwrap();
        let foreign_identity = identity(foreign.as_ref());
        let mut postings = second
            .postings(Some(foreign), POSTINGS_ENUM_FREQS)
            .unwrap()
            .unwrap();
        assert_ne!(
            identity(postings.as_ref()),
            foreign_identity,
            "`canReuse(this)` is false for another MultiTermsEnum's enum"
        );
        assert!(as_multi(&mut postings).can_reuse(&second));
        assert!(!as_multi(&mut postings).can_reuse(&first));
        assert_eq!(drain_docs(postings.as_mut()), vec![0]);
    }

    #[test]
    fn postings_does_not_reuse_an_enum_of_another_type() {
        let mt = two_leaves_with_a_shared_trailing_term();
        let mut te = mt.multi_iterator().unwrap().unwrap();
        assert_eq!(te.next().unwrap(), Some(term(b"a")));
        let other: Box<dyn PostingsEnum> = Box::new(VecPostings::new(vec![3]));
        let other_identity = identity(other.as_ref());
        let mut postings = te
            .postings(Some(other), POSTINGS_ENUM_FREQS)
            .unwrap()
            .unwrap();
        assert_ne!(identity(postings.as_ref()), other_identity);
        assert_eq!(drain_docs(postings.as_mut()), vec![0]);
    }

    /// leaf 0 = {a, c}, leaf 1 = {b, c}, leaf 2 = {a, c}.
    fn three_leaves() -> MultiTerms {
        let leaf = |terms: [&[u8]; 2]| {
            Box::new(VecTerms::new(
                vec![
                    (term(terms[0]), 1, 1, vec![0]),
                    (term(terms[1]), 1, 1, vec![1]),
                ],
                true,
                false,
                false,
                false,
            )) as Box<dyn Terms>
        };
        MultiTerms::new(
            vec![leaf([b"a", b"c"]), leaf([b"b", b"c"]), leaf([b"a", b"c"])],
            vec![
                ReaderSlice::new(0, 10, 0),
                ReaderSlice::new(10, 10, 1),
                ReaderSlice::new(20, 10, 2),
            ],
        )
        .unwrap()
    }

    fn match_order(te: &MultiTermsEnum) -> Vec<usize> {
        te.get_match_array()
            .iter()
            .map(|sub| sub.terms_enum_index().sub_index())
            .collect()
    }

    #[test]
    fn the_match_array_follows_lucenes_term_merge_queue_and_postings_sorts_it() {
        // The order `TermMergeQueue.fillTop` gathers the subs on a term in is
        // the heap's, which `pushTop`'s `updateTop`s rearrange; the expected
        // orders are those `lucene-core-10.5.0.jar` prints for the same three
        // leaves.
        let mt = three_leaves();
        let mut te = mt.multi_iterator().unwrap().unwrap();
        assert_eq!(te.next().unwrap(), Some(term(b"a")));
        assert_eq!(match_order(&te), vec![0, 2]);
        assert_eq!(te.next().unwrap(), Some(term(b"b")));
        assert_eq!(match_order(&te), vec![1]);
        assert_eq!(te.next().unwrap(), Some(term(b"c")));
        assert_eq!(match_order(&te), vec![1, 2, 0]);
        // `postings` sorts `top` in place by sub index
        // (`ArrayUtil.timSort(top, 0, numTop, ...)`).
        let mut postings = te.postings(None, POSTINGS_ENUM_FREQS).unwrap().unwrap();
        assert_eq!(match_order(&te), vec![0, 1, 2]);
        assert_eq!(drain_docs(postings.as_mut()), vec![1, 11, 21]);
        assert_eq!(te.next().unwrap(), None);
    }

    #[test]
    fn to_string_renders_null_for_a_sub_that_never_had_a_term() {
        // Leaf 1 has the field but no term, so `reset` never gives its
        // `TermsEnumWithSlice` an enum: `super(null, index)` stays, and
        // `Objects.toString(null)` is `null`.
        let mt = MultiTerms::new(
            vec![
                Box::new(VecTerms::new(
                    vec![(term(b"a"), 1, 1, vec![0])],
                    true,
                    false,
                    false,
                    false,
                )),
                Box::new(VecTerms::new(vec![], true, false, false, false)),
            ],
            vec![ReaderSlice::new(0, 5, 0), ReaderSlice::new(5, 5, 1)],
        )
        .unwrap();
        let te = mt.multi_iterator().unwrap().unwrap();
        assert_eq!(
            te.to_string(),
            "MultiTermsEnum([ReaderSlice[start=0, length=5, readerIndex=0]:VecTermsEnum, \
             ReaderSlice[start=5, length=5, readerIndex=1]:null])"
        );
    }
}
