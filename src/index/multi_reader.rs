//! `MultiReader`, `ReaderSlice`, and `ReaderUtil` ported from
//! `org.apache.lucene.index`.
//!
//! This module provides the in-memory composite reader over an arbitrary set
//! of sub-`IndexReader`s, together with the doc-ID mapping helpers shared by
//! all composite readers.
//!
//! # Decision on `BaseCompositeReader`
//!
//! Lucene models the shared behaviour of every array-backed composite reader
//! in an abstract `BaseCompositeReader<R extends IndexReader>` base class: it
//! stores the `subReaders` array, the `starts[]` doc-base table, a precomputed
//! `maxDoc`, a lazily-computed `numDocs`, and final implementations of
//! `termVectors`, `storedFields`, `docFreq`, `totalTermFreq`,
//! `getSumDocFreq`, `getDocCount`, and `getSumTotalTermFreq` that dispatch to
//! (or aggregate across) the sub-readers.
//!
//! This port does **not** introduce a separate `BaseCompositeReader` type.
//! Rust expresses the same sharing through the [`CompositeReader`] trait +
//! the [`build_composite_context`] helper (already used by
//! [`StandardDirectoryReader`](crate::index::StandardDirectoryReader)), plus
//! the [`reader_util`] free functions for doc-ID mapping. Each concrete
//! composite reader in this crate is thin enough that the small amount of
//! duplicated aggregation arithmetic is preferable to an extra abstract layer
//! that would fight the trait-based design (there is no `R extends IndexReader`
//! generic parameter to carry in Rust, and `numDocs`/`maxDoc` are already
//! trait methods). The `starts[]` table and the dispatch/aggregation logic that
//! would live in `BaseCompositeReader` instead live here, in
//! [`MultiReader`] and in [`reader_util`]; a future `ParallelLeafReader`/
//! `ParallelCompositeReader` port will reuse [`reader_util`] directly.

#![deny(unsafe_code)]

use std::{
    collections::HashSet,
    fmt::{Debug, Formatter},
    sync::{
        atomic::{AtomicI32, Ordering},
        Arc, Weak,
    },
};

use crate::document::Document;
use crate::error::{LuceneError, Result};
use crate::index::index_reader::{
    build_composite_context, CacheHelper, CompositeReader, IndexReader, IndexReaderCore,
    StoredFields,
};
use crate::index::leaf_reader::TermVectors;
use crate::index::reader_context::{IndexReaderContext, LeafReaderContext};
use crate::index::{Fields, Term};

// ---------------------------------------------------------------------------
// ReaderSlice
// ---------------------------------------------------------------------------

/// A contiguous slice of global doc IDs belonging to one sub-reader of a
/// composite reader.
///
/// Equivalent to `org.apache.lucene.index.ReaderSlice`. `start` is the first
/// global doc ID of the slice, `length` is the number of docs, and
/// `reader_index` is the ordinal of the owning sub-reader in
/// [`CompositeReader::get_sequential_sub_readers`].
///
/// # The `length` convention, and where Lucene breaks it
///
/// `length` means what the Java record's javadoc says it means: *"Number of
/// documents in this slice."* It is authoritative for exactly one consumer —
/// [`BitsSlice::new`](crate::index::BitsSlice::new), which narrows a global
/// [`Bits`](crate::util::Bits) to `start..start + length`. Anything else in
/// this crate reads only `start` (to offset local doc IDs into the global
/// space) and `reader_index` (to order the subs).
///
/// Lucene's own factories do not all honour the convention, and the ports of
/// those factories reproduce whatever the Java site does, so that a
/// side-by-side comparison stays exact:
///
/// * [`MultiFields::get_fields`](crate::index::MultiFields::get_fields) sets
///   `length` to the **leaf's** `maxDoc`. This is the true slice width and
///   matches `FieldsConsumer.merge`, which builds its slices from
///   `mergeState.maxDocs[readerIndex]`.
/// * [`MultiTerms::get_terms`](crate::index::MultiTerms::get_terms) sets
///   `length` to the **composite** reader's `maxDoc`, transcribing
///   `new ReaderSlice(ctx.docBase, r.maxDoc(), leafIdx)`
///   (`MultiTerms.java:82`). Lucene's `SlowCompositeCodecReaderWrapper` is
///   looser still: it passes `docStarts[i + 1]`, the slice's *end*.
///
/// Neither value is ever read, in Lucene or here, which is why the
/// discrepancy has survived. The rule for this port is therefore: **never feed
/// a `ReaderSlice` from `MultiTerms::get_terms` to `BitsSlice`** — build one
/// with the sub-reader's own `max_doc()` instead. `MultiBits` already does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReaderSlice {
    /// First global document ID of this slice.
    pub start: i32,
    /// Number of documents in this slice.
    pub length: i32,
    /// Ordinal of the owning sub-reader.
    pub reader_index: i32,
}

impl ReaderSlice {
    /// Creates a new `ReaderSlice`.
    ///
    /// Equivalent to the `ReaderSlice(int, int, int)` record constructor.
    pub const fn new(start: i32, length: i32, reader_index: i32) -> Self {
        Self {
            start,
            length,
            reader_index,
        }
    }
}

// ---------------------------------------------------------------------------
// ReaderUtil
// ---------------------------------------------------------------------------

/// Common helpers for mapping global doc IDs to sub-readers and walking the
/// reader context tree.
///
/// Equivalent to `org.apache.lucene.index.ReaderUtil`. Exposed as free
/// functions rather than a class, following Rust idiom.
pub mod reader_util {
    use super::{LeafReaderContext, ReaderSlice};
    use std::sync::Arc;

    /// Returns the index of the sub-reader that owns the global document ID
    /// `doc_id`, given the per-sub-reader doc-start table `doc_starts` (where
    /// `doc_starts[i]` is the first global doc ID of sub-reader `i`).
    ///
    /// Ports `ReaderUtil.subIndex(int n, int[] docStarts)`. The array need not
    /// carry a trailing sentinel: the binary search returns the last index
    /// whose start is `<= doc_id`.
    ///
    /// # Deliberate divergence from Lucene: the "no such sub-reader" result
    ///
    /// Java returns `int` and, when the search falls off the left edge, returns
    /// the raw `hi`, which is **`-1`** — either because `doc_id` is smaller
    /// than `docStarts[0]` or because the table is empty. Callers treat that as
    /// a bug signal rather than an index (`MultiBits.get` asserts
    /// `reader != -1`, `MultiBits.java:92`).
    ///
    /// This port returns `usize`, which cannot represent `-1`, so it clamps to
    /// `0` instead. The two behaviours differ only on inputs that cannot occur
    /// in a well-formed reader: document IDs are non-negative and a composite
    /// reader's `starts[0]` is always `0`, so `doc_id < doc_starts[0]` is
    /// unreachable, and an empty table means there is no sub-reader to return
    /// an index for at all. Rucene's callers (this module's private
    /// `resolve_sub`, and `MultiBits::get`) range-check `doc_id` against
    /// `max_doc` before calling, so the clamped value is never used to index
    /// anything.
    ///
    /// Changing the signature to `Option<usize>` or `isize` would propagate a
    /// case that cannot happen into every call site; the clamp is documented
    /// here instead and pinned by
    /// `sub_index_clamps_to_zero_where_java_returns_minus_one`.
    ///
    /// # Panics
    ///
    /// Does not panic. Out-of-range `doc_id` values resolve to the nearest
    /// boundary index; callers that need strict bounds checking must validate
    /// `doc_id` first.
    pub fn sub_index(doc_id: i32, doc_starts: &[i32]) -> usize {
        // find the sub-reader for doc_id:
        let size = doc_starts.len();
        let mut lo: isize = 0; // search starts array
        let mut hi: isize = size as isize - 1; // for first element less than n, return its index
        while hi >= lo {
            let mid = ((lo + hi) >> 1) as usize;
            let mid_value = doc_starts[mid];
            if doc_id < mid_value {
                hi = mid as isize - 1;
            } else if doc_id > mid_value {
                lo = mid as isize + 1;
            } else {
                // found a match
                let mut mid = mid;
                while mid + 1 < size && doc_starts[mid + 1] == mid_value {
                    mid += 1; // scan to last match
                }
                return mid;
            }
        }
        // hi < lo. Java returns `hi` here, which is -1 when the search fell
        // off the left edge; `usize` cannot express that, so clamp to 0. See
        // the deliberate-divergence note on this function.
        hi.max(0) as usize
    }

    /// Returns the index of the leaf context that owns the global document ID
    /// `doc_id`, using each leaf's `doc_base()` as the start table.
    ///
    /// Ports `ReaderUtil.subIndex(int n, List<LeafReaderContext> leaves)`, and
    /// shares [`sub_index`]'s deliberate divergence on the "no such leaf" case:
    /// Java returns `-1`, this port clamps to `0`.
    pub fn sub_index_from_leaves(doc_id: i32, leaves: &[Arc<LeafReaderContext>]) -> usize {
        let size = leaves.len();
        let mut lo: isize = 0;
        let mut hi: isize = size as isize - 1;
        while hi >= lo {
            let mid = ((lo + hi) >> 1) as usize;
            let mid_value = leaves[mid].doc_base();
            if doc_id < mid_value {
                hi = mid as isize - 1;
            } else if doc_id > mid_value {
                lo = mid as isize + 1;
            } else {
                let mut mid = mid;
                while mid + 1 < size && leaves[mid + 1].doc_base() == mid_value {
                    mid += 1;
                }
                return mid;
            }
        }
        // See sub_index: Java would return -1 here; `usize` clamps to 0.
        hi.max(0) as usize
    }

    /// Returns the index of the slice in `slices` that owns the global document
    /// ID `doc_id`, using each slice's `start` as the start table.
    ///
    /// This is the `ReaderSlice`-driven counterpart of [`sub_index`]; Lucene
    /// does not expose this exact overload but it is the natural mapping when
    /// the caller already holds a `&[ReaderSlice]`. It shares [`sub_index`]'s
    /// deliberate divergence on the "no such slice" case: Java's `subIndex`
    /// would return `-1`, this port clamps to `0`.
    pub fn index_of(doc_id: i32, slices: &[ReaderSlice]) -> usize {
        let size = slices.len();
        let mut lo: isize = 0;
        let mut hi: isize = size as isize - 1;
        while hi >= lo {
            let mid = ((lo + hi) >> 1) as usize;
            let mid_value = slices[mid].start;
            if doc_id < mid_value {
                hi = mid as isize - 1;
            } else if doc_id > mid_value {
                lo = mid as isize + 1;
            } else {
                let mut mid = mid;
                while mid + 1 < size && slices[mid + 1].start == mid_value {
                    mid += 1;
                }
                return mid;
            }
        }
        // See sub_index: Java would return -1 here; `usize` clamps to 0.
        hi.max(0) as usize
    }

    /// Walks up the reader-context tree and returns the top-level (root)
    /// context.
    ///
    /// Equivalent to `ReaderUtil.getTopLevelContext(IndexReaderContext)`.
    pub fn get_top_level_context(
        mut ctx: Arc<dyn super::IndexReaderContext>,
    ) -> Arc<dyn super::IndexReaderContext> {
        while let Some(parent) = ctx.parent() {
            if let Some(parent) = parent.upgrade() {
                ctx = parent;
            } else {
                break;
            }
        }
        ctx
    }
}

// Re-export the helper functions at the module level for convenience, while
// also keeping them grouped under `reader_util` to mirror Lucene's
// `ReaderUtil.*` static-method call sites.
pub use reader_util::{get_top_level_context, index_of, sub_index, sub_index_from_leaves};

// ---------------------------------------------------------------------------
// MultiReader
// ---------------------------------------------------------------------------

/// Comparator used to order a composite reader's sub-readers before its
/// doc-base table is built.
///
/// Equivalent to the `Comparator<IndexReader> subReadersSorter` parameter of
/// `MultiReader`'s three-argument constructor and of
/// `BaseCompositeReader(R[], Comparator<R>)`.
pub type SubReadersSorter<'a> =
    &'a dyn Fn(&Arc<dyn IndexReader>, &Arc<dyn IndexReader>) -> std::cmp::Ordering;

/// A [`CompositeReader`] over an arbitrary in-memory set of sub-`IndexReader`s.
///
/// Equivalent to `org.apache.lucene.index.MultiReader`. Unlike
/// [`StandardDirectoryReader`](crate::index::StandardDirectoryReader), a
/// `MultiReader` is not backed by a `Directory`; it simply stitches together
/// the sub-readers it is given, renumbering doc IDs across the concatenation.
///
/// The `close_sub_readers` flag passed to [`MultiReader::new`] controls
/// whether closing this reader closes its children. When `false`, the
/// children's reference counts are incremented at construction and
/// decremented at close, so they survive the parent — mirroring Java's
/// `closeSubReaders` semantics.
///
/// Field/Terms-level aggregation via `MultiFields` is not provided in this
/// phase; the per-doc (`term_vectors`/`stored_fields`) and numeric
/// (`doc_freq`/`total_term_freq`/`get_sum_doc_freq`/`get_doc_count`/
/// `get_sum_total_term_freq`) accessors are aggregated across sub-readers,
/// matching `BaseCompositeReader` in Lucene 10.5.0.
pub struct MultiReader {
    core: IndexReaderCore,
    sub_readers: Vec<Arc<dyn IndexReader>>,
    /// `starts[i]` is the first global doc ID of sub-reader `i`.
    starts: Vec<i32>,
    max_doc: i32,
    /// Lazily-computed `num_docs`; `-1` means "not computed yet". Mirrors
    /// `BaseCompositeReader.numDocs` (an `AtomicInteger` initialised to `-1`).
    num_docs: AtomicI32,
    close_sub_readers: bool,
}

impl Debug for MultiReader {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MultiReader")
            .field("num_sub_readers", &self.sub_readers.len())
            .field("max_doc", &self.max_doc)
            .field("close_sub_readers", &self.close_sub_readers)
            .finish()
    }
}

impl MultiReader {
    /// Creates a `MultiReader` aggregating the given sub-readers.
    ///
    /// Equivalent to
    /// `MultiReader(IndexReader[] subReaders, boolean closeSubReaders)`.
    ///
    /// When `close_sub_readers` is `false`, each sub-reader's reference count
    /// is incremented so that the sub-readers outlive this `MultiReader`; on
    /// close, this reader decrements them (instead of closing them outright).
    pub fn new(sub_readers: Vec<Arc<dyn IndexReader>>, close_sub_readers: bool) -> Result<Self> {
        Self::new_sorted(sub_readers, None, close_sub_readers)
    }

    /// Creates a `MultiReader` aggregating the given sub-readers, optionally
    /// ordering them first.
    ///
    /// Equivalent to `MultiReader(IndexReader[] subReaders,
    /// Comparator<IndexReader> subReadersSorter, boolean closeSubReaders)`.
    /// When `sub_readers_sorter` is `Some`, the sub-readers are sorted with it
    /// **before** the `starts` doc-base table is built, so the global doc-ID
    /// space follows the sorted order — exactly as `BaseCompositeReader`'s
    /// constructor does (`Arrays.sort(subReaders, subReadersSorter)`,
    /// `BaseCompositeReader.java:76`). The sort is stable, matching
    /// `Arrays.sort`'s guarantee for object arrays.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the sub-readers' total
    /// `maxDoc` exceeds the index-wide document limit.
    pub fn new_sorted(
        mut sub_readers: Vec<Arc<dyn IndexReader>>,
        sub_readers_sorter: Option<SubReadersSorter<'_>>,
        close_sub_readers: bool,
    ) -> Result<Self> {
        if let Some(sorter) = sub_readers_sorter {
            sub_readers.sort_by(|a, b| sorter(a, b));
        }

        // The core is created up front so that each sub-reader can register it
        // as a parent while the starts table is built, mirroring the
        // `r.registerParentReader(this)` call in BaseCompositeReader's
        // constructor (`BaseCompositeReader.java:86`). Registration is by core
        // identity because `self` does not exist yet; see
        // `IndexReaderCore::register_parent_core`.
        let core = IndexReaderCore::new();

        // Build the starts table and the precomputed maxDoc, mirroring
        // BaseCompositeReader's constructor.
        let mut starts = Vec::with_capacity(sub_readers.len());
        let mut max_doc: i64 = 0;
        for r in &sub_readers {
            starts.push(max_doc as i32);
            max_doc += r.max_doc() as i64;
            r.core().register_parent_core(&core);
        }

        // Guard against overflow / the IndexWriter.MAX_DOCS ceiling, matching
        // BaseCompositeReader, which throws IllegalArgumentException for
        // non-DirectoryReader composites that exceed the limit.
        let actual_max_docs = i64::from(crate::index::documents_writer::MAX_DOCS);
        if max_doc > actual_max_docs {
            return Err(LuceneError::IllegalArgument(format!(
                "Too many documents: composite IndexReaders cannot exceed {actual_max_docs} \
                 but readers have total maxDoc={max_doc}"
            )));
        }
        let max_doc = i32::try_from(max_doc).map_err(|_| {
            LuceneError::IllegalArgument(format!("Composite reader maxDoc {max_doc} overflows i32"))
        })?;

        // When the sub-readers must outlive this MultiReader, take an extra
        // reference on each, matching Java's `subReaders[i].incRef()` in the
        // `!closeSubReaders` branch.
        if !close_sub_readers {
            for r in &sub_readers {
                r.inc_ref()?;
            }
        }

        Ok(Self {
            core,
            sub_readers,
            starts,
            max_doc,
            num_docs: AtomicI32::new(-1),
            close_sub_readers,
        })
    }

    /// Creates a `MultiReader` that closes its sub-readers when closed.
    ///
    /// Equivalent to the varargs constructor `MultiReader(IndexReader...)`,
    /// which defaults `closeSubReaders` to `true`.
    pub fn new_closing(sub_readers: Vec<Arc<dyn IndexReader>>) -> Result<Self> {
        Self::new(sub_readers, true)
    }

    /// Returns the per-sub-reader doc-start table (`starts[i]` is the first
    /// global doc ID of sub-reader `i`).
    ///
    /// Equivalent to the `starts` field exposed to subclasses of
    /// `BaseCompositeReader` via `readerBase(int)`.
    pub fn starts(&self) -> &[i32] {
        &self.starts
    }

    /// Returns the sub-reader index owning the global doc ID `doc_id`.
    ///
    /// Equivalent to `BaseCompositeReader.readerIndex(int docID)` (which is
    /// `protected` in Java; exposed here as a public convenience for callers
    /// that need to map a global doc ID back to its sub-reader). Returns
    /// `IllegalArgument` when `doc_id` is out of range, matching Java.
    pub fn reader_index(&self, doc_id: i32) -> Result<usize> {
        Ok(resolve_sub(doc_id, self.max_doc, &self.starts)?.0)
    }
}

impl IndexReader for MultiReader {
    fn core(&self) -> &IndexReaderCore {
        &self.core
    }

    fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
        self.ensure_open()?;
        Ok(Box::new(MultiTermVectors::new(
            self.sub_readers.clone(),
            self.starts.clone(),
            self.max_doc,
        )))
    }

    fn num_docs(&self) -> i32 {
        // Don't call ensureOpen() here (it could affect performance) — mirror
        // BaseCompositeReader's lazy, opaque read of numDocs.
        let cached = self.num_docs.load(Ordering::Relaxed);
        if cached != -1 {
            return cached;
        }
        let sum: i32 = self.sub_readers.iter().map(|r| r.num_docs()).sum();
        // sum is always >= 0 (each sub num_docs >= 0), so it never collides
        // with the -1 sentinel.
        self.num_docs.store(sum, Ordering::Relaxed);
        sum
    }

    fn max_doc(&self) -> i32 {
        // Don't call ensureOpen() here — mirror BaseCompositeReader.
        self.max_doc
    }

    fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
        self.ensure_open()?;
        Ok(Box::new(MultiStoredFields::new(
            self.sub_readers.clone(),
            self.starts.clone(),
            self.max_doc,
        )))
    }

    fn do_close(&self) -> Result<()> {
        // Mirror MultiReader.doClose(): close each sub if closeSubReaders,
        // otherwise decRef it; surface the first error encountered.
        let mut first_err: Option<LuceneError> = None;
        for r in &self.sub_readers {
            let res = if self.close_sub_readers {
                r.close()
            } else {
                r.dec_ref()
            };
            if let Err(e) = res {
                if first_err.is_none() {
                    first_err = Some(e);
                }
            }
        }
        match first_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        // MultiReader instances can be short-lived, which would make caching
        // trappy, so we do not cache on them — unless they wrap a single
        // reader, in which case we delegate. Matches MultiReader in Lucene.
        if self.sub_readers.len() == 1 {
            self.sub_readers[0].get_reader_cache_helper()
        } else {
            None
        }
    }

    fn doc_freq(&self, term: &Term) -> Result<i32> {
        self.ensure_open()?;
        let mut total: i32 = 0;
        for r in &self.sub_readers {
            let sub = r.doc_freq(term)?;
            total = total.wrapping_add(sub);
        }
        Ok(total)
    }

    fn total_term_freq(&self, term: &Term) -> Result<i64> {
        self.ensure_open()?;
        let mut total: i64 = 0;
        for r in &self.sub_readers {
            let sub = r.total_term_freq(term)?;
            total += sub;
        }
        Ok(total)
    }

    fn get_sum_doc_freq(&self, field: &str) -> Result<i64> {
        self.ensure_open()?;
        let mut total: i64 = 0;
        for r in &self.sub_readers {
            total += r.get_sum_doc_freq(field)?;
        }
        Ok(total)
    }

    fn get_doc_count(&self, field: &str) -> Result<i32> {
        self.ensure_open()?;
        let mut total: i32 = 0;
        for r in &self.sub_readers {
            total = total.wrapping_add(r.get_doc_count(field)?);
        }
        Ok(total)
    }

    fn get_sum_total_term_freq(&self, field: &str) -> Result<i64> {
        self.ensure_open()?;
        let mut total: i64 = 0;
        for r in &self.sub_readers {
            total += r.get_sum_total_term_freq(field)?;
        }
        Ok(total)
    }

    fn build_context(
        self: Arc<Self>,
        parent: Option<Weak<dyn IndexReaderContext>>,
        ord_in_parent: i32,
        doc_base_in_parent: i32,
        leaf_ord: i32,
        leaf_doc_base: i32,
    ) -> Arc<dyn IndexReaderContext> {
        let composite: Arc<dyn CompositeReader> = self;
        build_composite_context(
            composite,
            parent,
            ord_in_parent,
            doc_base_in_parent,
            leaf_ord,
            leaf_doc_base,
        )
    }
}

impl CompositeReader for MultiReader {
    fn get_sequential_sub_readers(&self) -> Vec<Arc<dyn IndexReader>> {
        self.sub_readers.clone()
    }
}

// ---------------------------------------------------------------------------
// Dispatching wrappers (the Rust analogue of BaseCompositeReader's anonymous
// TermVectors / StoredFields implementations)
// ---------------------------------------------------------------------------

/// Resolves a global `doc_id` to `(sub-reader index, local doc_id)`, validating
/// that `doc_id` is within `0..max_doc`. Mirrors the bounds check in
/// `BaseCompositeReader.readerIndex(int)` and the per-doc dispatch in its
/// anonymous `TermVectors` / `StoredFields`.
fn resolve_sub(doc_id: i32, max_doc: i32, starts: &[i32]) -> Result<(usize, i32)> {
    if doc_id < 0 || doc_id >= max_doc {
        return Err(LuceneError::IllegalArgument(format!(
            "docID must be >= 0 and < maxDoc={max_doc} (got docID={doc_id})"
        )));
    }
    let i = sub_index(doc_id, starts);
    Ok((i, doc_id - starts[i]))
}

/// `TermVectors` view that dispatches per doc ID to the owning sub-reader.
///
/// Equivalent to the anonymous `TermVectors` returned by
/// `BaseCompositeReader.termVectors()`. Unlike the Java version, this port
/// does not cache the per-sub-reader `TermVectors` handles: each call
/// re-resolves them. The trait's `get(&self)` is non-mutating, so caching
/// would require interior mutability that is not worth the cost here; the
/// sub-readers are expected to cache their own `TermVectors` state.
/// `TermVectors` view that dispatches per doc ID to the owning sub-reader.
///
/// Equivalent to the anonymous `TermVectors` returned by
/// `BaseCompositeReader.termVectors()`. Exposed as `pub(crate)` so that
/// [`ParallelCompositeReader`](crate::index::ParallelCompositeReader) can reuse
/// the same dispatch logic over its synthetic `ParallelLeafReader` sub-readers.
pub(crate) struct MultiTermVectors {
    sub_readers: Vec<Arc<dyn IndexReader>>,
    starts: Vec<i32>,
    max_doc: i32,
}

impl MultiTermVectors {
    pub(crate) fn new(
        sub_readers: Vec<Arc<dyn IndexReader>>,
        starts: Vec<i32>,
        max_doc: i32,
    ) -> Self {
        Self {
            sub_readers,
            starts,
            max_doc,
        }
    }

    fn resolve(&self, doc_id: i32) -> Result<(usize, i32)> {
        resolve_sub(doc_id, self.max_doc, &self.starts)
    }
}

impl Debug for MultiTermVectors {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MultiTermVectors")
            .field("num_sub_readers", &self.sub_readers.len())
            .finish()
    }
}

impl TermVectors for MultiTermVectors {
    fn prefetch(&mut self, doc_id: i32) -> Result<()> {
        let (i, local) = self.resolve(doc_id)?;
        self.sub_readers[i].term_vectors()?.prefetch(local)
    }

    fn get(&self, doc_id: i32) -> Result<Option<Box<dyn Fields>>> {
        let (i, local) = self.resolve(doc_id)?;
        self.sub_readers[i].term_vectors()?.get(local)
    }
}

/// `StoredFields` view that dispatches per doc ID to the owning sub-reader.
///
/// Equivalent to the anonymous `StoredFields` returned by
/// `BaseCompositeReader.storedFields()`. As with [`MultiTermVectors`], per-sub
/// handles are not cached.
/// `StoredFields` view that dispatches per doc ID to the owning sub-reader.
///
/// Equivalent to the anonymous `StoredFields` returned by
/// `BaseCompositeReader.storedFields()`. Exposed as `pub(crate)` so that
/// [`ParallelCompositeReader`](crate::index::ParallelCompositeReader) can reuse
/// the same dispatch logic over its synthetic `ParallelLeafReader` sub-readers.
pub(crate) struct MultiStoredFields {
    sub_readers: Vec<Arc<dyn IndexReader>>,
    starts: Vec<i32>,
    max_doc: i32,
}

impl MultiStoredFields {
    pub(crate) fn new(
        sub_readers: Vec<Arc<dyn IndexReader>>,
        starts: Vec<i32>,
        max_doc: i32,
    ) -> Self {
        Self {
            sub_readers,
            starts,
            max_doc,
        }
    }

    fn resolve(&self, doc_id: i32) -> Result<(usize, i32)> {
        resolve_sub(doc_id, self.max_doc, &self.starts)
    }
}

impl Debug for MultiStoredFields {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MultiStoredFields")
            .field("num_sub_readers", &self.sub_readers.len())
            .finish()
    }
}

impl StoredFields for MultiStoredFields {
    fn prefetch(&mut self, doc_id: i32) -> Result<()> {
        let (i, local) = self.resolve(doc_id)?;
        self.sub_readers[i].stored_fields()?.prefetch(local)
    }

    fn document_with_visitor(
        &self,
        doc_id: i32,
        visitor: &mut dyn crate::codecs::stub::StoredFieldVisitor,
    ) -> Result<()> {
        let (i, local) = self.resolve(doc_id)?;
        self.sub_readers[i]
            .stored_fields()?
            .document_with_visitor(local, visitor)
    }

    fn document(&self, doc_id: i32) -> Result<Document> {
        let (i, local) = self.resolve(doc_id)?;
        self.sub_readers[i].stored_fields()?.document(local)
    }

    fn document_fields(&self, doc_id: i32, fields_to_load: &HashSet<String>) -> Result<Document> {
        let (i, local) = self.resolve(doc_id)?;
        self.sub_readers[i]
            .stored_fields()?
            .document_fields(local, fields_to_load)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::stub::StoredFieldVisitor;
    use crate::index::index_reader::IndexReader;
    use crate::index::leaf_reader::{LeafMetaData, LeafReader};
    use crate::index::{
        BinaryDocValues, ByteVectorValues, DocValuesSkipper, EmptyFields, FieldInfos,
        FloatVectorValues, NumericDocValues, PointValues, SortedDocValues, SortedNumericDocValues,
        SortedSetDocValues, Term, Terms,
    };
    use crate::search::knn::KnnCollector;
    use crate::search::AcceptDocs;
    use crate::util::Bits;

    // ----- minimal in-memory leaf reader for tests -----

    #[derive(Debug)]
    struct StubTermVectors;
    impl TermVectors for StubTermVectors {
        fn get(&self, _doc: i32) -> Result<Option<Box<dyn Fields>>> {
            Ok(Some(Box::new(EmptyFields)))
        }
    }

    #[derive(Debug)]
    struct StubStoredFields {
        max_doc: i32,
    }
    impl StoredFields for StubStoredFields {
        fn document_with_visitor(
            &self,
            doc_id: i32,
            _visitor: &mut dyn StoredFieldVisitor,
        ) -> Result<()> {
            assert!(
                (0..self.max_doc).contains(&doc_id),
                "local doc_id {doc_id} out of range for stub sub-reader (max_doc {})",
                self.max_doc
            );
            Ok(())
        }

        fn document(&self, doc_id: i32) -> Result<Document> {
            assert!((0..self.max_doc).contains(&doc_id));
            Ok(Document::new())
        }

        fn document_fields(
            &self,
            doc_id: i32,
            _fields_to_load: &HashSet<String>,
        ) -> Result<Document> {
            assert!((0..self.max_doc).contains(&doc_id));
            Ok(Document::new())
        }
    }

    #[derive(Debug)]
    struct StubLeafReader {
        core: IndexReaderCore,
        max_doc: i32,
        num_docs: i32,
    }

    impl StubLeafReader {
        fn new(max_doc: i32, num_docs: i32) -> Self {
            Self {
                core: IndexReaderCore::new(),
                max_doc,
                num_docs,
            }
        }
    }

    impl LeafReader for StubLeafReader {
        fn core(&self) -> &IndexReaderCore {
            &self.core
        }

        fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
            Ok(Box::new(StubTermVectors))
        }

        fn num_docs(&self) -> i32 {
            self.num_docs
        }

        fn max_doc(&self) -> i32 {
            self.max_doc
        }

        fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
            Ok(Box::new(StubStoredFields {
                max_doc: self.max_doc,
            }))
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

        fn get_numeric_doc_values(
            &self,
            _field: &str,
        ) -> Result<Option<Box<dyn NumericDocValues>>> {
            Ok(None)
        }

        fn get_binary_doc_values(&self, _field: &str) -> Result<Option<Box<dyn BinaryDocValues>>> {
            Ok(None)
        }

        fn get_sorted_doc_values(&self, _field: &str) -> Result<Option<Box<dyn SortedDocValues>>> {
            Ok(None)
        }

        fn get_sorted_numeric_doc_values(
            &self,
            _field: &str,
        ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
            Ok(None)
        }

        fn get_sorted_set_doc_values(
            &self,
            _field: &str,
        ) -> Result<Option<Box<dyn SortedSetDocValues>>> {
            Ok(None)
        }

        fn get_norm_values(&self, _field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
            Ok(None)
        }

        fn get_doc_values_skipper(
            &self,
            _field: &str,
        ) -> Result<Option<Box<dyn DocValuesSkipper>>> {
            Ok(None)
        }

        fn get_float_vector_values(
            &self,
            _field: &str,
        ) -> Result<Option<Box<dyn FloatVectorValues>>> {
            Ok(None)
        }

        fn get_byte_vector_values(
            &self,
            _field: &str,
        ) -> Result<Option<Box<dyn ByteVectorValues>>> {
            Ok(None)
        }

        fn search_nearest_vectors(
            &self,
            _field: &str,
            _target: &[f32],
            _collector: &mut dyn KnnCollector,
            _accept_docs: &mut dyn AcceptDocs,
        ) -> Result<()> {
            Ok(())
        }

        fn search_nearest_vectors_byte(
            &self,
            _field: &str,
            _target: &[u8],
            _collector: &mut dyn KnnCollector,
            _accept_docs: &mut dyn AcceptDocs,
        ) -> Result<()> {
            Ok(())
        }

        fn get_field_infos(&self) -> FieldInfos {
            FieldInfos::empty()
        }

        fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
            None
        }

        fn get_point_values(&self, _field: &str) -> Result<Option<Box<dyn PointValues>>> {
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
        Arc::new(StubLeafReader::new(max_doc, num_docs)) as Arc<dyn IndexReader>
    }

    // ----- ReaderSlice / ReaderUtil tests -----

    #[test]
    fn reader_slice_fields_round_trip() {
        let s = ReaderSlice::new(5, 10, 2);
        assert_eq!(s.start, 5);
        assert_eq!(s.length, 10);
        assert_eq!(s.reader_index, 2);
    }

    #[test]
    fn sub_index_finds_owning_reader_at_boundaries() {
        // Three readers with max_docs 5, 5, 4 → starts [0, 5, 10], maxDoc 14.
        let starts = [0i32, 5, 10];
        assert_eq!(sub_index(0, &starts), 0);
        assert_eq!(sub_index(4, &starts), 0);
        assert_eq!(sub_index(5, &starts), 1);
        assert_eq!(sub_index(9, &starts), 1);
        assert_eq!(sub_index(10, &starts), 2);
        assert_eq!(sub_index(13, &starts), 2);
        // doc_id just past the last start still resolves to the last reader.
        assert_eq!(sub_index(14, &starts), 2);
    }

    #[test]
    fn sub_index_off_by_one_does_not_leak_into_previous_reader() {
        let starts = [0i32, 5, 10];
        // doc 5 belongs to reader 1, not reader 0; doc 10 to reader 2.
        assert_eq!(sub_index(5, &starts), 1);
        assert_eq!(sub_index(10, &starts), 2);
        // And the last doc of reader 0 is 4.
        assert_eq!(sub_index(4, &starts), 0);
    }

    #[test]
    fn sub_index_clamps_to_zero_where_java_returns_minus_one() {
        // Pins the documented divergence from `ReaderUtil.subIndex`, which
        // returns the raw `hi` (= -1) in both of these cases. `usize` cannot
        // express -1, so this port clamps to 0. Neither input is reachable
        // through a well-formed composite reader: doc IDs are non-negative and
        // `starts[0]` is always 0.
        let empty: [i32; 0] = [];
        assert_eq!(sub_index(0, &empty), 0, "empty table: Java returns -1");
        assert_eq!(index_of(0, &[]), 0, "empty table: Java returns -1");

        let starts = [4i32, 9];
        assert_eq!(
            sub_index(0, &starts),
            0,
            "doc_id below starts[0]: Java returns -1"
        );
        assert_eq!(
            sub_index_from_leaves(0, &[]),
            0,
            "empty leaf list: Java returns -1"
        );
    }

    #[test]
    fn sub_index_scans_to_the_last_sub_reader_sharing_a_doc_base() {
        // Empty sub-readers contribute a zero-width slice, so several entries
        // of `starts` can carry the same value. `ReaderUtil.subIndex` resolves
        // such a doc ID to the *last* matching index — the only sub-reader of
        // the group that can actually contain the document — which is the
        // "scan to last match" branch of the binary search.
        //
        // starts = [0, 1, 1, 2] describes sub-readers with maxDoc 1, 0, 1, ...
        let starts = [0i32, 1, 1, 2];
        assert_eq!(sub_index(0, &starts), 0);
        assert_eq!(sub_index(1, &starts), 2, "doc 1 lives in the non-empty sub");
        assert_eq!(sub_index(2, &starts), 3);

        // A run of empty sub-readers at the very front: every start is 0.
        let all_empty = [0i32, 0, 0];
        assert_eq!(sub_index(0, &all_empty), 2);

        // A run in the middle, longer than one.
        let middle_run = [0i32, 5, 5, 5, 9];
        assert_eq!(sub_index(5, &middle_run), 3);
        assert_eq!(sub_index(4, &middle_run), 0);
        assert_eq!(sub_index(9, &middle_run), 4);

        // A run at the end resolves to the last index.
        let trailing_run = [0i32, 3, 3];
        assert_eq!(sub_index(3, &trailing_run), 2);
    }

    #[test]
    fn index_of_scans_to_the_last_slice_sharing_a_start() {
        let slices = [
            ReaderSlice::new(0, 1, 0),
            ReaderSlice::new(1, 0, 1),
            ReaderSlice::new(1, 1, 2),
            ReaderSlice::new(2, 1, 3),
        ];
        assert_eq!(index_of(0, &slices), 0);
        assert_eq!(index_of(1, &slices), 2);
        assert_eq!(index_of(2, &slices), 3);

        let all_zero = [
            ReaderSlice::new(0, 0, 0),
            ReaderSlice::new(0, 0, 1),
            ReaderSlice::new(0, 3, 2),
        ];
        assert_eq!(index_of(0, &all_zero), 2);
    }

    #[test]
    fn sub_index_from_leaves_scans_to_the_last_leaf_sharing_a_doc_base() {
        // A zero-document leaf between two populated ones makes leaves 1 and 2
        // share docBase 2.
        let a = leaf(2, 2);
        let empty = leaf(0, 0);
        let c = leaf(3, 3);
        let mr: Arc<dyn IndexReader> = Arc::new(MultiReader::new(vec![a, empty, c], true).unwrap());
        let leaves = mr.leaves();
        assert_eq!(leaves.len(), 3);
        assert_eq!(leaves[1].doc_base(), 2);
        assert_eq!(leaves[2].doc_base(), 2);
        assert_eq!(
            sub_index_from_leaves(2, &leaves),
            2,
            "doc 2 belongs to the last leaf sharing docBase 2"
        );
        assert_eq!(sub_index_from_leaves(1, &leaves), 0);
        assert_eq!(sub_index_from_leaves(4, &leaves), 2);
    }

    #[test]
    fn sub_index_single_reader() {
        let starts = [0i32];
        assert_eq!(sub_index(0, &starts), 0);
        assert_eq!(sub_index(99, &starts), 0);
    }

    #[test]
    fn index_of_uses_slice_starts() {
        let slices = [
            ReaderSlice::new(0, 5, 0),
            ReaderSlice::new(5, 5, 1),
            ReaderSlice::new(10, 4, 2),
        ];
        assert_eq!(index_of(0, &slices), 0);
        assert_eq!(index_of(5, &slices), 1);
        assert_eq!(index_of(9, &slices), 1);
        assert_eq!(index_of(10, &slices), 2);
        assert_eq!(index_of(13, &slices), 2);
    }

    #[test]
    fn sub_index_from_leaves_matches_doc_base() {
        // Build a real MultiReader so we get real LeafReaderContexts with
        // cumulative docBase values.
        let a = leaf(2, 2);
        let b = leaf(3, 3);
        let c = leaf(4, 4);
        let mr: Arc<dyn IndexReader> = Arc::new(MultiReader::new(vec![a, b, c], true).unwrap());
        let leaves = mr.leaves();
        assert_eq!(leaves.len(), 3);
        assert_eq!(leaves[0].doc_base(), 0);
        assert_eq!(leaves[1].doc_base(), 2);
        assert_eq!(leaves[2].doc_base(), 5);
        assert_eq!(sub_index_from_leaves(0, &leaves), 0);
        assert_eq!(sub_index_from_leaves(1, &leaves), 0);
        assert_eq!(sub_index_from_leaves(2, &leaves), 1);
        assert_eq!(sub_index_from_leaves(4, &leaves), 1);
        assert_eq!(sub_index_from_leaves(5, &leaves), 2);
        assert_eq!(sub_index_from_leaves(8, &leaves), 2);
    }

    #[test]
    fn get_top_level_context_walks_to_root() {
        let a = leaf(2, 2);
        let b = leaf(3, 3);
        let mr: Arc<dyn IndexReader> = Arc::new(MultiReader::new(vec![a, b], true).unwrap());
        let ctx = mr.get_context();
        // `leaves()` takes the `Arc` by value; clone it so `ctx` (the strong
        // reference keeping the composite alive) is not moved.
        let leaves = ctx.clone().leaves();
        assert_eq!(leaves.len(), 2);
        let leaf_ctx: Arc<dyn IndexReaderContext> =
            Arc::clone(&leaves[0]) as Arc<dyn IndexReaderContext>;
        let top = get_top_level_context(leaf_ctx);
        assert!(top.is_top_level());
        assert!(!top.is_leaf_context());
    }

    // ----- MultiReader aggregation tests -----

    #[test]
    fn multi_reader_aggregates_num_docs_and_max_doc() {
        let a = leaf(5, 5);
        let b = leaf(7, 6);
        let c = leaf(3, 2);
        let mr = MultiReader::new(vec![a, b, c], true).unwrap();
        assert_eq!(mr.max_doc(), 15);
        assert_eq!(mr.num_docs(), 13);
        assert_eq!(mr.num_deleted_docs(), 2);
        assert!(mr.has_deletions());
    }

    #[test]
    fn multi_reader_aggregates_num_docs_lazily_and_caches() {
        let a = leaf(5, 5);
        let b = leaf(5, 5);
        let mr = MultiReader::new(vec![a, b], true).unwrap();
        // First call computes; second returns the cached value.
        assert_eq!(mr.num_docs(), 10);
        assert_eq!(mr.num_docs(), 10);
        // The sentinel (-1) must have been overwritten.
        assert!(mr.num_docs.load(Ordering::Relaxed) >= 0);
    }

    #[test]
    fn multi_reader_get_sequential_sub_readers_preserves_order() {
        let a = leaf(2, 2);
        let b = leaf(3, 3);
        let mr = MultiReader::new(vec![Arc::clone(&a), Arc::clone(&b)], true).unwrap();
        let subs = mr.get_sequential_sub_readers();
        assert_eq!(subs.len(), 2);
        assert!(Arc::ptr_eq(&subs[0], &a));
        assert!(Arc::ptr_eq(&subs[1], &b));
    }

    #[test]
    fn multi_reader_leaves_have_cumulative_doc_base() {
        let a = leaf(5, 5);
        let b = leaf(5, 5);
        let c = leaf(5, 5);
        let mr: Arc<dyn IndexReader> = Arc::new(MultiReader::new(vec![a, b, c], true).unwrap());
        let leaves = mr.leaves();
        assert_eq!(leaves.len(), 3);
        assert_eq!(leaves[0].doc_base(), 0);
        assert_eq!(leaves[1].doc_base(), 5);
        assert_eq!(leaves[2].doc_base(), 10);
        assert_eq!(leaves[0].ord(), 0);
        assert_eq!(leaves[1].ord(), 1);
        assert_eq!(leaves[2].ord(), 2);
    }

    #[test]
    fn multi_reader_starts_table_matches_doc_bases() {
        let a = leaf(5, 5);
        let b = leaf(5, 5);
        let c = leaf(5, 5);
        let mr = MultiReader::new(vec![a, b, c], true).unwrap();
        assert_eq!(mr.starts(), &[0, 5, 10]);
    }

    #[test]
    fn multi_reader_reader_index_validates_bounds() {
        let a = leaf(5, 5);
        let b = leaf(5, 5);
        let mr = MultiReader::new(vec![a, b], true).unwrap();
        assert_eq!(mr.reader_index(0).unwrap(), 0);
        assert_eq!(mr.reader_index(4).unwrap(), 0);
        assert_eq!(mr.reader_index(5).unwrap(), 1);
        assert_eq!(mr.reader_index(9).unwrap(), 1);
        // Out of range → IllegalArgument.
        assert!(mr.reader_index(-1).is_err());
        assert!(mr.reader_index(10).is_err());
    }

    #[test]
    fn multi_reader_doc_freq_and_term_freq_aggregate() {
        // StubLeafReader.terms() returns None, so LeafReader's blanket
        // IndexReader impl yields doc_freq=0 / total_term_freq=0 per sub.
        let a = leaf(5, 5);
        let b = leaf(5, 5);
        let mr = MultiReader::new(vec![a, b], true).unwrap();
        let term = Term::from_text("f", "v");
        assert_eq!(mr.doc_freq(&term).unwrap(), 0);
        assert_eq!(mr.total_term_freq(&term).unwrap(), 0);
        assert_eq!(mr.get_sum_doc_freq("f").unwrap(), 0);
        assert_eq!(mr.get_doc_count("f").unwrap(), 0);
        assert_eq!(mr.get_sum_total_term_freq("f").unwrap(), 0);
    }

    #[test]
    fn multi_reader_term_vectors_dispatch_to_sub() {
        let a = leaf(5, 5);
        let b = leaf(5, 5);
        let mr = MultiReader::new(vec![a, b], true).unwrap();
        let tv = mr.term_vectors().unwrap();
        // Both halves of the doc-ID space return a (non-empty) EmptyFields.
        assert!(tv.get(0).unwrap().is_some());
        assert!(tv.get(5).unwrap().is_some());
        assert!(tv.get(9).unwrap().is_some());
    }

    #[test]
    fn multi_reader_stored_fields_dispatch_with_local_doc_id() {
        // Each StubStoredFields asserts the local doc_id is in range, so a
        // wrong base subtraction would panic here.
        let a = leaf(5, 5);
        let b = leaf(5, 5);
        let mr = MultiReader::new(vec![a, b], true).unwrap();
        let sf = mr.stored_fields().unwrap();
        sf.document(0).unwrap();
        sf.document(4).unwrap();
        sf.document(5).unwrap();
        sf.document(9).unwrap();
        sf.document_with_visitor(5, &mut NoopVisitor).unwrap();
        sf.document_fields(9, &HashSet::new()).unwrap();
    }

    struct NoopVisitor;
    impl StoredFieldVisitor for NoopVisitor {
        fn needs_field(
            &mut self,
            _info: &crate::index::FieldInfo,
        ) -> crate::codecs::stub::StoredFieldVisitorStatus {
            crate::codecs::stub::StoredFieldVisitorStatus::No
        }
    }

    #[test]
    fn multi_reader_cache_helper_delegates_for_single_sub() {
        // StubLeafReader returns None for get_reader_cache_helper, so a
        // single-sub MultiReader also yields None — but via the delegate path.
        let a = leaf(5, 5);
        let mr = MultiReader::new(vec![a], true).unwrap();
        assert!(mr.get_reader_cache_helper().is_none());
    }

    #[test]
    fn multi_reader_cache_helper_none_for_multi_sub() {
        let a = leaf(5, 5);
        let b = leaf(5, 5);
        let mr = MultiReader::new(vec![a, b], true).unwrap();
        assert!(mr.get_reader_cache_helper().is_none());
    }

    // ----- close / ref-count semantics -----

    #[test]
    fn close_with_close_sub_readers_closes_children() {
        let a = leaf(5, 5);
        let b = leaf(5, 5);
        let mr = MultiReader::new(vec![Arc::clone(&a), Arc::clone(&b)], true).unwrap();
        // Children start at refCount 1.
        assert_eq!(a.get_ref_count(), 1);
        assert_eq!(b.get_ref_count(), 1);
        mr.close().unwrap();
        // close() decrements to 0 and closes — further use is an error.
        assert!(a.ensure_open().is_err());
        assert!(b.ensure_open().is_err());
    }

    #[test]
    fn close_without_close_sub_readers_keeps_children_alive() {
        let a = leaf(5, 5);
        let b = leaf(5, 5);
        // close_sub_readers=false → constructor incRefs each sub (refCount 2).
        let mr = MultiReader::new(vec![Arc::clone(&a), Arc::clone(&b)], false).unwrap();
        assert_eq!(a.get_ref_count(), 2);
        assert_eq!(b.get_ref_count(), 2);
        mr.close().unwrap();
        // MultiReader closed (decRef→0 on itself), children decremented to 1.
        assert_eq!(a.get_ref_count(), 1);
        assert_eq!(b.get_ref_count(), 1);
        assert!(a.ensure_open().is_ok());
        assert!(b.ensure_open().is_ok());
        // Children are still usable.
        assert_eq!(a.num_docs(), 5);
    }

    #[test]
    fn close_without_close_sub_readers_then_child_close_releases_them() {
        let a = leaf(5, 5);
        let b = leaf(5, 5);
        let mr = MultiReader::new(vec![Arc::clone(&a), Arc::clone(&b)], false).unwrap();
        mr.close().unwrap();
        // Now drop the surviving child refs.
        a.close().unwrap();
        b.close().unwrap();
        assert!(a.ensure_open().is_err());
        assert!(b.ensure_open().is_err());
    }

    #[test]
    fn debug_repr_mentions_multi_reader() {
        let a = leaf(2, 2);
        let mr = MultiReader::new(vec![a], true).unwrap();
        let s = format!("{mr:?}");
        assert!(s.contains("MultiReader"));
    }

    #[test]
    fn rejects_too_many_documents() {
        // MAX_DOCS = i32::MAX - 128 in this port. Two readers each with
        // max_doc = i32::MAX / 2 sum to i32::MAX - 1, which exceeds MAX_DOCS,
        // so the constructor must reject the composite.
        let big = leaf(i32::MAX / 2, i32::MAX / 2);
        let other = leaf(i32::MAX / 2, i32::MAX / 2);
        let res = MultiReader::new(vec![big, other], true);
        assert!(
            matches!(res, Err(LuceneError::IllegalArgument(_))),
            "expected IllegalArgument for exceeding MAX_DOCS"
        );
    }

    #[test]
    fn empty_multi_reader_is_zero_docs() {
        let mr = MultiReader::new(vec![], true).unwrap();
        assert_eq!(mr.max_doc(), 0);
        assert_eq!(mr.num_docs(), 0);
        assert_eq!(mr.get_sequential_sub_readers().len(), 0);
        let leaves = Arc::new(mr).leaves();
        assert_eq!(leaves.len(), 0);
    }

    // ----- parent-reader registration (BaseCompositeReader.java:86) -----

    #[test]
    fn closing_a_sub_reader_directly_invalidates_the_parent() {
        let a = leaf(5, 5);
        let b = leaf(5, 5);
        let mr = MultiReader::new(vec![Arc::clone(&a), Arc::clone(&b)], true).unwrap();
        assert!(mr.ensure_open().is_ok());
        assert!(!mr.core().is_closed_by_child());

        // Java: every sub-reader registers the composite as a parent, so
        // closing a child behind the composite's back poisons the composite.
        a.close().unwrap();

        assert!(mr.core().is_closed_by_child());
        let err = mr.ensure_open().unwrap_err();
        assert!(
            matches!(err, LuceneError::AlreadyClosed(_)),
            "expected AlreadyClosed, got {err:?}"
        );
        // The poisoning reaches the public API, not just ensure_open().
        assert!(mr.stored_fields().is_err());
        assert!(mr.term_vectors().is_err());
    }

    #[test]
    fn closing_a_sub_reader_propagates_through_nested_composites() {
        let a = leaf(2, 2);
        let b = leaf(3, 3);
        let inner: Arc<dyn IndexReader> =
            Arc::new(MultiReader::new(vec![Arc::clone(&a), b], true).unwrap());
        let c = leaf(4, 4);
        let outer = MultiReader::new(vec![Arc::clone(&inner), c], false).unwrap();
        assert!(outer.ensure_open().is_ok());

        a.close().unwrap();

        assert!(inner.core().is_closed_by_child());
        assert!(
            outer.core().is_closed_by_child(),
            "the flag must propagate transitively up the reader tree"
        );
    }

    #[test]
    fn registration_does_not_keep_the_parent_alive() {
        let a = leaf(5, 5);
        {
            let mr = MultiReader::new(vec![Arc::clone(&a)], false).unwrap();
            assert!(mr.ensure_open().is_ok());
        }
        // The parent is gone; closing the child must not panic and the weak
        // link simply fails to upgrade.
        a.dec_ref().unwrap(); // undo the incRef taken by close_sub_readers=false
        a.close().unwrap();
        assert!(a.ensure_open().is_err());
    }

    #[test]
    fn a_parent_registered_before_being_moved_is_still_reachable() {
        // The registration happens inside `new`, while the reader is still a
        // stack value; moving it into an Arc afterwards must not break the
        // link, which is exactly why the flag lives behind an Arc.
        let a = leaf(5, 5);
        let mr = MultiReader::new(vec![Arc::clone(&a)], true).unwrap();
        let moved: Arc<dyn IndexReader> = Arc::new(mr);
        a.close().unwrap();
        assert!(moved.core().is_closed_by_child());
    }

    // ----- sub-reader sorting (BaseCompositeReader.java:76) -----

    #[test]
    fn new_sorted_orders_sub_readers_before_building_the_starts_table() {
        // Sort by descending maxDoc: the doc-ID space must follow the sorted
        // order, not the argument order.
        let a = leaf(2, 2);
        let b = leaf(7, 7);
        let c = leaf(4, 4);
        let sorter =
            |x: &Arc<dyn IndexReader>, y: &Arc<dyn IndexReader>| y.max_doc().cmp(&x.max_doc());
        let mr = MultiReader::new_sorted(vec![a, b, c], Some(&sorter), true).unwrap();

        assert_eq!(mr.starts(), &[0, 7, 11]);
        assert_eq!(mr.max_doc(), 13);
        let subs = mr.get_sequential_sub_readers();
        assert_eq!(
            subs.iter().map(|r| r.max_doc()).collect::<Vec<_>>(),
            vec![7, 4, 2]
        );
        // ... and doc-ID resolution agrees with the sorted table.
        assert_eq!(mr.reader_index(0).unwrap(), 0);
        assert_eq!(mr.reader_index(6).unwrap(), 0);
        assert_eq!(mr.reader_index(7).unwrap(), 1);
        assert_eq!(mr.reader_index(11).unwrap(), 2);
    }

    #[test]
    fn new_sorted_without_a_comparator_preserves_the_argument_order() {
        let a = leaf(2, 2);
        let b = leaf(7, 7);
        let mr = MultiReader::new_sorted(vec![a, b], None, true).unwrap();
        assert_eq!(mr.starts(), &[0, 2]);
        assert_eq!(
            mr.get_sequential_sub_readers()
                .iter()
                .map(|r| r.max_doc())
                .collect::<Vec<_>>(),
            vec![2, 7]
        );
    }

    #[test]
    fn new_sorted_is_a_stable_sort() {
        // Three readers that compare equal keep their relative order, matching
        // Arrays.sort's stability guarantee for object arrays.
        let a = leaf(5, 1);
        let b = leaf(5, 2);
        let c = leaf(5, 3);
        let sorter =
            |x: &Arc<dyn IndexReader>, y: &Arc<dyn IndexReader>| x.max_doc().cmp(&y.max_doc());
        let mr = MultiReader::new_sorted(vec![a, b, c], Some(&sorter), true).unwrap();
        assert_eq!(
            mr.get_sequential_sub_readers()
                .iter()
                .map(|r| r.num_docs())
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
    }
}
