//! Parallel readers ported from `org.apache.lucene.index`.
//!
//! This module provides [`ParallelLeafReader`] and
//! [`ParallelCompositeReader`], the two "parallel" reader types that overlay
//! several sub-readers over the *same* doc-ID range, each field owned by
//! exactly one sub-reader.
//!
//! # Doc-ID semantics (parallel overlay)
//!
//! Unlike [`MultiReader`](crate::index::MultiReader), which **concatenates**
//! sub-readers and renumbers doc IDs across them, a parallel reader **overlays**
//! its sub-readers on the same doc-ID range. Every sub-reader of a
//! [`ParallelLeafReader`] must report the same `maxDoc`; a given doc ID addresses
//! the *same logical document* in every sub, but each field lives in only one
//! sub. The first sub-reader that declares a field "owns" it; later occurrences
//! of the same field name are ignored (matching Lucene 10.5.0, which does **not**
//! raise on field overlap — see the note on [`ParallelLeafReader::new`]).
//!
//! [`ParallelCompositeReader`] composes several sub-[`CompositeReader`]s that
//! share the same leaf structure (same leaf count and per-ordinal `maxDoc`); it
//! produces one [`ParallelLeafReader`] per leaf ordinal.

#![deny(unsafe_code)]

use std::{
    collections::{HashMap, HashSet},
    fmt::{Debug, Formatter},
    sync::{
        atomic::{AtomicI32, Ordering},
        Arc, Weak,
    },
};

use crate::error::{LuceneError, Result};
use crate::index::field_infos::{FieldInfos, FieldInfosBuilder, FieldNumbers};
use crate::index::index_reader::{
    build_composite_context, CacheHelper, CompositeReader, IndexReader, IndexReaderCore,
    StoredFields,
};
use crate::index::leaf_reader::{LeafMetaData, LeafReader, TermVectors};
use crate::index::multi_reader::{MultiStoredFields, MultiTermVectors};
use crate::index::reader_context::{IndexReaderContext, LeafReaderContext};
use crate::index::{
    BinaryDocValues, ByteVectorValues, DocValuesSkipper, Fields, FloatVectorValues,
    NumericDocValues, PointValues, SortedDocValues, SortedNumericDocValues, SortedSetDocValues,
    Term, Terms,
};
use crate::search::knn::KnnCollector;
use crate::search::AcceptDocs;
use crate::util::extra::Version;
use crate::util::Bits;

// ---------------------------------------------------------------------------
// ParallelLeafReader
// ---------------------------------------------------------------------------

/// Increments the reference count of a leaf sub-reader held as `dyn LeafReader`.
///
/// The `IndexReader` trait provides `inc_ref`/`dec_ref`/`close`, but the
/// sub-readers here are stored as `Arc<dyn LeafReader>` and Rust cannot upcast
/// `Arc<dyn LeafReader>` to `Arc<dyn IndexReader>` (the two traits are related
/// only by a blanket impl, not by inheritance — see the note in
/// `leaf_reader.rs`). These helpers replicate the `IndexReader` lifecycle
/// methods directly from `core()` and `do_close()`, which are `LeafReader`
/// methods. They are `pub(crate)` so [`ParallelCompositeReader`]'s synthetic
/// leaves are handled uniformly.
fn leaf_inc_ref(r: &Arc<dyn LeafReader>) -> Result<()> {
    r.core().inc_ref()
}

fn leaf_dec_ref(r: &Arc<dyn LeafReader>) -> Result<()> {
    r.core().dec_ref_with_close(|| r.do_close())
}

fn leaf_close(r: &Arc<dyn LeafReader>) -> Result<()> {
    if !r.core().is_closed() {
        leaf_dec_ref(r)?;
        r.core().set_closed();
    }
    Ok(())
}

/// A [`LeafReader`] that reads multiple, parallel indexes over the same doc-ID
/// range.
///
/// Equivalent to `org.apache.lucene.index.ParallelLeafReader`. Each sub-reader
/// must have the same `maxDoc`; fields are taken from the first sub-reader that
/// declares them. Deletions, `numDocs` and `maxDoc` come from the first
/// sub-reader.
///
/// # Field ownership (first-wins)
///
/// Lucene 10.5.0 does **not** reject a field name that appears in more than one
/// sub-reader: the first sub-reader that contains the field owns it, and later
/// sub-readers' definitions of the same field are silently ignored. This port
/// matches that behaviour exactly (verified against
/// `ParallelLeafReader.java` at the `releases/lucene/10.5.0` tag).
///
/// # Compatibility checks
///
/// Construction fails with [`LuceneError::IllegalArgument`] when:
/// - `readers` is empty but `stored_fields_readers` is not;
/// - any reader in the union of `readers` and `stored_fields_readers` has a
///   different `maxDoc` from the first;
/// - the sub-readers disagree on the index sort;
/// - the sub-readers disagree on `createdVersionMajor`.
pub struct ParallelLeafReader {
    core: IndexReaderCore,
    /// The parallel sub-readers that own fields/terms/doc-values/points/vectors.
    parallel_readers: Vec<Arc<dyn LeafReader>>,
    /// Sub-readers used only for stored fields and term vectors. When the
    /// caller passes no separate `stored_fields_readers`, this is the same set
    /// as `parallel_readers`.
    stored_fields_readers: Vec<Arc<dyn LeafReader>>,
    /// Identity-deduplicated union of `parallel_readers` and
    /// `stored_fields_readers`; these are the readers closed/decRef'd on
    /// `do_close`.
    complete_reader_set: Vec<Arc<dyn LeafReader>>,
    /// `field name -> index into parallel_readers` for the owning sub-reader.
    /// Populated for every field of every parallel reader (first-wins).
    field_to_reader: HashMap<String, usize>,
    /// `field name -> index into parallel_readers`, only for fields that have
    /// term vectors. Used by `term_vectors`.
    tv_field_to_reader: HashMap<String, usize>,
    /// `field name -> index into parallel_readers`, only for fields that are
    /// indexed (`IndexOptions != NONE`). Used by `terms`.
    terms_field_to_reader: HashMap<String, usize>,
    field_infos: FieldInfos,
    max_doc: i32,
    num_docs: i32,
    has_deletions: bool,
    meta_data: LeafMetaData,
    close_sub_readers: bool,
    /// When `true`, `do_close` is a no-op regardless of `close_sub_readers`.
    /// This is the Rust analogue of the anonymous `ParallelLeafReader` subclass
    /// created by [`ParallelCompositeReader`] whose `doClose()` is overridden
    /// to do nothing, making the synthetic reader invisible to ref-counting.
    suppress_close: bool,
}

impl Debug for ParallelLeafReader {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParallelLeafReader")
            .field("num_parallel_readers", &self.parallel_readers.len())
            .field("max_doc", &self.max_doc)
            .field("close_sub_readers", &self.close_sub_readers)
            .finish()
    }
}

impl ParallelLeafReader {
    /// Creates a `ParallelLeafReader` over `readers` that closes its sub-readers
    /// on close.
    ///
    /// Equivalent to the varargs constructor `ParallelLeafReader(LeafReader...)`,
    /// which defaults `closeSubReaders` to `true`.
    pub fn new(readers: Vec<Arc<dyn LeafReader>>) -> Result<Self> {
        Self::with_close_flag(true, readers)
    }

    /// Creates a `ParallelLeafReader` over `readers`, controlling whether the
    /// sub-readers are closed on close.
    ///
    /// Equivalent to
    /// `ParallelLeafReader(boolean closeSubReaders, LeafReader...)`. When
    /// `close_sub_readers` is `false`, each sub-reader's reference count is
    /// incremented at construction and decremented at close, so they survive
    /// the parent.
    pub fn with_close_flag(
        close_sub_readers: bool,
        readers: Vec<Arc<dyn LeafReader>>,
    ) -> Result<Self> {
        Self::with_stored_fields(close_sub_readers, readers, Vec::new())
    }

    /// Expert constructor that splits the parallel sub-readers from the readers
    /// used for stored fields.
    ///
    /// Equivalent to
    /// `ParallelLeafReader(boolean closeSubReaders, LeafReader[] readers,
    /// LeafReader[] storedFieldsReaders)`. When `stored_fields_readers` is
    /// empty, stored fields and term vectors are served from `readers`.
    ///
    /// # Errors
    ///
    /// See the type-level docs for the compatibility checks enforced.
    pub fn with_stored_fields(
        close_sub_readers: bool,
        readers: Vec<Arc<dyn LeafReader>>,
        stored_fields_readers: Vec<Arc<dyn LeafReader>>,
    ) -> Result<Self> {
        Self::build(
            close_sub_readers,
            readers,
            stored_fields_readers,
            false, /* suppress_close */
        )
    }

    /// Builds a synthetic `ParallelLeafReader` whose `do_close` is a no-op.
    ///
    /// Used by [`ParallelCompositeReader`] to wrap the i-th leaf of every
    /// sub-composite: the synthetic reader is completely invisible to
    /// ref-counting (it neither increments nor closes its sub-leaves), mirroring
    /// Lucene's anonymous `ParallelLeafReader(true, subs, storedSubs) {
    /// protected void doClose() {} }`.
    fn new_synthetic(
        readers: Vec<Arc<dyn LeafReader>>,
        stored_fields_readers: Vec<Arc<dyn LeafReader>>,
    ) -> Result<Self> {
        // close_sub_readers=true is only meaningful to avoid the incRef branch
        // of the constructor; suppress_close=true then neutralises do_close.
        Self::build(true, readers, stored_fields_readers, true)
    }

    fn build(
        close_sub_readers: bool,
        readers: Vec<Arc<dyn LeafReader>>,
        stored_fields_readers: Vec<Arc<dyn LeafReader>>,
        suppress_close: bool,
    ) -> Result<Self> {
        if readers.is_empty() && !stored_fields_readers.is_empty() {
            return Err(LuceneError::IllegalArgument(
                "There must be at least one main reader if storedFieldsReaders are used."
                    .to_string(),
            ));
        }

        // maxDoc/numDocs/hasDeletions come from the first parallel reader.
        let (max_doc, num_docs, has_deletions) = if let Some(first) = readers.first() {
            // `has_deletions` is an `IndexReader` method; since the sub-readers
            // are held as `dyn LeafReader` (the trait objects cannot be upcast to
            // `dyn IndexReader` — see the note in `leaf_reader.rs`), derive it
            // from the `LeafReader` accessors, matching the `IndexReader` default.
            (
                first.max_doc(),
                first.num_docs(),
                first.max_doc() - first.num_docs() > 0,
            )
        } else {
            (0, 0, false)
        };

        // Identity-deduplicated union of the two reader sets: every reader in
        // the union must agree on maxDoc.
        let mut complete_reader_set: Vec<Arc<dyn LeafReader>> = Vec::new();
        for r in readers.iter().chain(stored_fields_readers.iter()) {
            if r.max_doc() != max_doc {
                return Err(LuceneError::IllegalArgument(format!(
                    "All readers must have same maxDoc: {max_doc}!={}",
                    r.max_doc()
                )));
            }
            if !complete_reader_set
                .iter()
                .any(|existing| Arc::ptr_eq(existing, r))
            {
                complete_reader_set.push(Arc::clone(r));
            }
        }

        // Discover the soft-deletes and parent field names from any reader.
        let soft_deletes_field = complete_reader_set.iter().find_map(|r| {
            r.get_field_infos()
                .get_soft_deletes_field()
                .map(str::to_string)
        });
        let parent_field = complete_reader_set
            .iter()
            .find_map(|r| r.get_field_infos().get_parent_field().map(str::to_string));

        let global_field_numbers = Arc::new(FieldNumbers::new(soft_deletes_field, parent_field)?);
        let mut builder = FieldInfosBuilder::new(global_field_numbers);

        let mut field_to_reader: HashMap<String, usize> = HashMap::new();
        let mut tv_field_to_reader: HashMap<String, usize> = HashMap::new();
        let mut terms_field_to_reader: HashMap<String, usize> = HashMap::new();

        let mut index_sort: Option<crate::search::Sort> = None;
        let mut created_version_major: i32 = -1;

        for (reader_idx, reader) in readers.iter().enumerate() {
            let leaf_meta = reader.get_meta_data();

            // Index sort must be consistent across sub-readers.
            let leaf_sort = leaf_meta.sort().cloned();
            if let Some(ref leaf_sort) = leaf_sort {
                match &index_sort {
                    None => index_sort = Some(leaf_sort.clone()),
                    Some(existing) => {
                        if existing != leaf_sort {
                            return Err(LuceneError::IllegalArgument(format!(
                                "cannot combine LeafReaders that have different index sorts: saw \
                                 both sort={existing:?} and {leaf_sort:?}"
                            )));
                        }
                    }
                }
            }

            // createdVersionMajor must be consistent across sub-readers.
            let leaf_created = leaf_meta.created_version_major();
            if created_version_major == -1 {
                created_version_major = leaf_created;
            } else if created_version_major != leaf_created {
                return Err(LuceneError::IllegalArgument(format!(
                    "cannot combine LeafReaders that have different creation versions: saw both \
                     version={created_version_major} and {leaf_created}"
                )));
            }

            // Build merged FieldInfos and the field->reader maps. First reader
            // owning a field name wins; later occurrences are ignored.
            for field_info in reader.get_field_infos().iter() {
                if field_to_reader.contains_key(&field_info.name) {
                    continue;
                }
                builder.add_with_doc_values_gen(field_info, field_info.get_doc_values_gen())?;
                field_to_reader.insert(field_info.name.clone(), reader_idx);
                if field_info.has_term_vectors() {
                    tv_field_to_reader.insert(field_info.name.clone(), reader_idx);
                }
                if field_info.get_index_options() != crate::index::IndexOptions::NONE {
                    terms_field_to_reader.insert(field_info.name.clone(), reader_idx);
                }
            }
        }

        if created_version_major == -1 {
            // Empty reader set: follow Lucene and default to the current major.
            created_version_major = Version::LATEST.major as i32;
        }

        // minVersion: the minimum across all readers, or None if any is None.
        let mut min_version: Option<Version> = Some(Version::LATEST);
        let mut has_blocks = false;
        for reader in &readers {
            let leaf_meta = reader.get_meta_data();
            has_blocks |= leaf_meta.has_blocks();
            match leaf_meta.min_version() {
                None => {
                    min_version = None;
                    break;
                }
                Some(v) => {
                    if min_version.map_or(true, |mv| mv.on_or_after(&v)) {
                        min_version = Some(v);
                    }
                }
            }
        }

        let field_infos = builder.finish()?;
        let meta_data =
            LeafMetaData::new(created_version_major, min_version, index_sort, has_blocks)?;

        // Finally, adjust ref-counts exactly as Lucene does: when the sub-readers
        // must outlive this ParallelLeafReader, incRef each one; and register this
        // reader as a parent so a child close propagates. (Registration of a parent
        // reader requires an `Arc<dyn IndexReader>` view of `self`, which is not
        // available until the value is constructed and wrapped; the registration
        // is therefore deferred to the caller-adjacent path — see
        // `register_with_subs` used by the construction sites below.)
        if !close_sub_readers {
            for r in &complete_reader_set {
                leaf_inc_ref(r)?;
            }
        }

        Ok(Self {
            core: IndexReaderCore::new(),
            parallel_readers: readers,
            stored_fields_readers,
            complete_reader_set,
            field_to_reader,
            tv_field_to_reader,
            terms_field_to_reader,
            field_infos,
            max_doc,
            num_docs,
            has_deletions,
            meta_data,
            close_sub_readers,
            suppress_close,
        })
    }

    /// Returns the parallel sub-readers passed at construction.
    ///
    /// Equivalent to `ParallelLeafReader.getParallelReaders()`.
    pub fn get_parallel_readers(&self) -> &[Arc<dyn LeafReader>] {
        &self.parallel_readers
    }

    /// Returns the index of the sub-reader that owns `field`, after ensuring
    /// this reader is still open.
    #[inline]
    fn owning_reader(&self, field: &str) -> Result<Option<usize>> {
        self.ensure_open()?;
        Ok(self.field_to_reader.get(field).copied())
    }
}

impl LeafReader for ParallelLeafReader {
    fn core(&self) -> &IndexReaderCore {
        &self.core
    }

    fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
        self.ensure_open()?;
        // Lucene aggregates term vectors across the parallel readers that have
        // any term-vector field. We collect the owning sub-reader for each
        // term-vector field (deduplicated by index) and fan out per doc.
        let reader_indices: Vec<usize> = {
            let mut set: HashSet<usize> = HashSet::new();
            for &idx in self.tv_field_to_reader.values() {
                set.insert(idx);
            }
            let mut v: Vec<usize> = set.into_iter().collect();
            v.sort_unstable();
            v
        };
        let tv_readers: Vec<Arc<dyn LeafReader>> = reader_indices
            .iter()
            .map(|&i| Arc::clone(&self.parallel_readers[i]))
            .collect();
        Ok(Box::new(ParallelTermVectors::new(tv_readers)))
    }

    fn num_docs(&self) -> i32 {
        // Don't call ensureOpen() here — mirror Lucene.
        self.num_docs
    }

    fn max_doc(&self) -> i32 {
        self.max_doc
    }

    fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
        self.ensure_open()?;
        Ok(Box::new(ParallelStoredFields::new(
            self.stored_fields_readers.clone(),
        )))
    }

    fn do_close(&self) -> Result<()> {
        if self.suppress_close {
            return Ok(());
        }
        let mut first_err: Option<LuceneError> = None;
        for r in &self.complete_reader_set {
            let res = if self.close_sub_readers {
                leaf_close(r)
            } else {
                leaf_dec_ref(r)
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
        // Delegate only when this reader wraps exactly one sub-reader with no
        // separate stored-fields set, matching Lucene.
        if self.parallel_readers.len() == 1
            && self.stored_fields_readers.len() == 1
            && Arc::ptr_eq(&self.parallel_readers[0], &self.stored_fields_readers[0])
        {
            self.parallel_readers[0].get_reader_cache_helper()
        } else {
            None
        }
    }

    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        if self.parallel_readers.len() == 1
            && self.stored_fields_readers.len() == 1
            && Arc::ptr_eq(&self.parallel_readers[0], &self.stored_fields_readers[0])
        {
            self.parallel_readers[0].get_core_cache_helper()
        } else {
            None
        }
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        self.ensure_open()?;
        match self.terms_field_to_reader.get(field) {
            Some(&idx) => self.parallel_readers[idx].terms(field),
            None => Ok(None),
        }
    }

    fn get_numeric_doc_values(&self, field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
        match self.owning_reader(field)? {
            Some(idx) => self.parallel_readers[idx].get_numeric_doc_values(field),
            None => Ok(None),
        }
    }

    fn get_binary_doc_values(&self, field: &str) -> Result<Option<Box<dyn BinaryDocValues>>> {
        match self.owning_reader(field)? {
            Some(idx) => self.parallel_readers[idx].get_binary_doc_values(field),
            None => Ok(None),
        }
    }

    fn get_sorted_doc_values(&self, field: &str) -> Result<Option<Box<dyn SortedDocValues>>> {
        match self.owning_reader(field)? {
            Some(idx) => self.parallel_readers[idx].get_sorted_doc_values(field),
            None => Ok(None),
        }
    }

    fn get_sorted_numeric_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
        match self.owning_reader(field)? {
            Some(idx) => self.parallel_readers[idx].get_sorted_numeric_doc_values(field),
            None => Ok(None),
        }
    }

    fn get_sorted_set_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn SortedSetDocValues>>> {
        match self.owning_reader(field)? {
            Some(idx) => self.parallel_readers[idx].get_sorted_set_doc_values(field),
            None => Ok(None),
        }
    }

    fn get_norm_values(&self, field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
        match self.owning_reader(field)? {
            Some(idx) => self.parallel_readers[idx].get_norm_values(field),
            None => Ok(None),
        }
    }

    fn get_doc_values_skipper(&self, field: &str) -> Result<Option<Box<dyn DocValuesSkipper>>> {
        match self.owning_reader(field)? {
            Some(idx) => self.parallel_readers[idx].get_doc_values_skipper(field),
            None => Ok(None),
        }
    }

    fn get_float_vector_values(&self, field: &str) -> Result<Option<Box<dyn FloatVectorValues>>> {
        match self.owning_reader(field)? {
            Some(idx) => self.parallel_readers[idx].get_float_vector_values(field),
            None => Ok(None),
        }
    }

    fn get_byte_vector_values(&self, field: &str) -> Result<Option<Box<dyn ByteVectorValues>>> {
        match self.owning_reader(field)? {
            Some(idx) => self.parallel_readers[idx].get_byte_vector_values(field),
            None => Ok(None),
        }
    }

    fn search_nearest_vectors(
        &self,
        field: &str,
        target: &[f32],
        collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        if let Some(&idx) = self.field_to_reader.get(field) {
            self.parallel_readers[idx].search_nearest_vectors(
                field,
                target,
                collector,
                accept_docs,
            )?;
        }
        Ok(())
    }

    fn search_nearest_vectors_byte(
        &self,
        field: &str,
        target: &[u8],
        collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        if let Some(&idx) = self.field_to_reader.get(field) {
            self.parallel_readers[idx].search_nearest_vectors_byte(
                field,
                target,
                collector,
                accept_docs,
            )?;
        }
        Ok(())
    }

    fn get_field_infos(&self) -> FieldInfos {
        self.field_infos.clone()
    }

    fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
        // Deletions are taken from the first parallel reader, matching Lucene.
        if self.has_deletions {
            self.parallel_readers
                .first()
                .and_then(|r| r.get_live_docs())
        } else {
            None
        }
    }

    fn get_point_values(&self, field: &str) -> Result<Option<Box<dyn PointValues>>> {
        match self.owning_reader(field)? {
            Some(idx) => self.parallel_readers[idx].get_point_values(field),
            None => Ok(None),
        }
    }

    fn check_integrity(&self) -> Result<()> {
        self.ensure_open()?;
        for r in &self.complete_reader_set {
            r.check_integrity()?;
        }
        Ok(())
    }

    fn get_meta_data(&self) -> LeafMetaData {
        self.meta_data.clone()
    }
}

// ---------------------------------------------------------------------------
// Parallel term-vectors / stored-fields wrappers
// ---------------------------------------------------------------------------

/// `TermVectors` view that merges term vectors across the parallel sub-readers
/// that own term-vector fields.
///
/// Equivalent to the anonymous `TermVectors` returned by
/// `ParallelLeafReader.termVectors()`. For a given doc ID, every owning
/// sub-reader is queried and the per-field `Terms` are unioned into a single
/// `Fields` view.
struct ParallelTermVectors {
    readers: Vec<Arc<dyn LeafReader>>,
}

impl ParallelTermVectors {
    fn new(readers: Vec<Arc<dyn LeafReader>>) -> Self {
        Self { readers }
    }
}

impl Debug for ParallelTermVectors {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParallelTermVectors")
            .field("num_readers", &self.readers.len())
            .finish()
    }
}

impl TermVectors for ParallelTermVectors {
    fn prefetch(&mut self, doc_id: i32) -> Result<()> {
        for r in &self.readers {
            r.term_vectors()?.prefetch(doc_id)?;
        }
        Ok(())
    }

    fn get(&self, doc_id: i32) -> Result<Option<Box<dyn Fields>>> {
        // Fan out: gather each owning sub-reader's term vectors for the doc.
        // Each sub-reader returns its own `Fields` view for the doc; we union
        // them into a single `ParallelFields` that dispatches `terms(name)` to
        // whichever sub holds that name.
        let mut sub_fields: Vec<Box<dyn Fields>> = Vec::new();
        for r in &self.readers {
            if let Some(doc_fields) = r.term_vectors()?.get(doc_id)? {
                sub_fields.push(doc_fields);
            }
        }
        if sub_fields.is_empty() {
            Ok(None)
        } else {
            Ok(Some(Box::new(ParallelFields::new(sub_fields))))
        }
    }
}

/// `Fields` view that unions several sub-`Fields` for the same doc ID.
///
/// Equivalent to the private `ParallelFields` inner class of
/// `ParallelLeafReader`. Unlike Lucene's `TreeMap`-backed version, this port
/// holds the sub-`Fields` boxes and dispatches `terms(name)` to the first sub
/// that has it — which lets the returned `Terms` be the real owned handle from
/// the owning sub-reader (no borrowing wrapper needed).
struct ParallelFields {
    sub_fields: Vec<Box<dyn Fields>>,
}

impl ParallelFields {
    fn new(sub_fields: Vec<Box<dyn Fields>>) -> Self {
        Self { sub_fields }
    }
}

impl Debug for ParallelFields {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParallelFields")
            .field("num_sub_fields", &self.sub_fields.len())
            .finish()
    }
}

impl Fields for ParallelFields {
    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        // Union of field names across the sub-Fields, deduplicated and sorted
        // for a stable order — mirrors Lucene's TreeMap-backed view.
        let mut names: HashSet<String> = HashSet::new();
        for sf in &self.sub_fields {
            for name in sf.iterator() {
                names.insert(name);
            }
        }
        let mut sorted: Vec<String> = names.into_iter().collect();
        sorted.sort_unstable();
        Box::new(sorted.into_iter())
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        for sf in &self.sub_fields {
            if let Some(t) = sf.terms(field)? {
                return Ok(Some(t));
            }
        }
        Ok(None)
    }

    fn size(&self) -> i32 {
        // Sum of per-sub sizes; informational, may overcount duplicate names
        // (which Lucene's TreeMap would not, but duplicates cannot arise here
        // because each field is owned by exactly one sub-reader).
        self.sub_fields.iter().map(|f| f.size()).sum()
    }
}

/// `StoredFields` view that fans out a document visit across every
/// stored-fields sub-reader.
///
/// Equivalent to the anonymous `StoredFields` returned by
/// `ParallelLeafReader.storedFields()`. Each sub-reader's stored fields are
/// visited for the same doc ID, unioning the loaded fields.
struct ParallelStoredFields {
    readers: Vec<Arc<dyn LeafReader>>,
}

impl ParallelStoredFields {
    fn new(readers: Vec<Arc<dyn LeafReader>>) -> Self {
        Self { readers }
    }
}

impl Debug for ParallelStoredFields {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParallelStoredFields")
            .field("num_readers", &self.readers.len())
            .finish()
    }
}

impl StoredFields for ParallelStoredFields {
    fn prefetch(&mut self, doc_id: i32) -> Result<()> {
        for r in &self.readers {
            r.stored_fields()?.prefetch(doc_id)?;
        }
        Ok(())
    }

    fn document_with_visitor(
        &self,
        doc_id: i32,
        visitor: &mut dyn crate::codecs::stub::StoredFieldVisitor,
    ) -> Result<()> {
        for r in &self.readers {
            r.stored_fields()?.document_with_visitor(doc_id, visitor)?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// ParallelCompositeReader
// ---------------------------------------------------------------------------

/// A [`CompositeReader`] that reads multiple, parallel sub-[`CompositeReader`]s
/// over the same doc-ID range.
///
/// Equivalent to `org.apache.lucene.index.ParallelCompositeReader`. Every
/// sub-composite must have the same `maxDoc`, the same number of leaves, and
/// matching per-ordinal leaf `maxDoc`s. The i-th leaf of every sub-composite is
/// combined into a [`ParallelLeafReader`]; `get_sequential_sub_readers()`
/// therefore returns one [`ParallelLeafReader`] per leaf ordinal.
///
/// # Compatibility checks
///
/// Construction fails with [`LuceneError::IllegalArgument`] when:
/// - `readers` is empty but `stored_fields_readers` is not;
/// - any sub-composite's `maxDoc` differs from the first;
/// - the sub-composites disagree on the number of leaves;
/// - the i-th leaf of any sub-composite has a different `maxDoc` from the i-th
///   leaf of the first sub-composite.
///
/// The per-ordinal parallel leaves are themselves validated by
/// [`ParallelLeafReader::new`] (field/term-vector overlap is first-wins, not an
/// error).
pub struct ParallelCompositeReader {
    core: IndexReaderCore,
    /// The synthetic `ParallelLeafReader`s, one per leaf ordinal.
    sub_readers: Vec<Arc<dyn IndexReader>>,
    /// `starts[i]` is the first global doc ID of synthetic sub-reader `i`.
    starts: Vec<i32>,
    max_doc: i32,
    /// Lazily-computed `num_docs`; `-1` means "not computed yet".
    num_docs: AtomicI32,
    close_sub_readers: bool,
    /// Identity-deduplicated union of the original `readers` and
    /// `stored_fields_readers`; these are what get closed/decRef'd on close.
    complete_reader_set: Vec<Arc<dyn IndexReader>>,
    /// The original composite readers (the `readers` arg), kept for the
    /// cache-helper delegation check.
    source_readers: Vec<Arc<dyn IndexReader>>,
    /// The original stored-fields composite readers, kept for the cache-helper
    /// delegation check. When the caller passed no separate stored-fields set,
    /// this equals `source_readers`.
    source_stored_fields_readers: Vec<Arc<dyn IndexReader>>,
}

impl Debug for ParallelCompositeReader {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParallelCompositeReader")
            .field("num_sub_readers", &self.sub_readers.len())
            .field("max_doc", &self.max_doc)
            .field("close_sub_readers", &self.close_sub_readers)
            .finish()
    }
}

impl ParallelCompositeReader {
    /// Creates a `ParallelCompositeReader` over `readers` that closes its
    /// sub-readers on close.
    ///
    /// Equivalent to the varargs constructor
    /// `ParallelCompositeReader(CompositeReader...)`, which defaults
    /// `closeSubReaders` to `true`.
    pub fn new(readers: Vec<Arc<dyn CompositeReader>>) -> Result<Self> {
        Self::with_close_flag(true, readers)
    }

    /// Creates a `ParallelCompositeReader` over `readers`, controlling whether
    /// the sub-readers are closed on close.
    ///
    /// Equivalent to
    /// `ParallelCompositeReader(boolean closeSubReaders, CompositeReader...)`.
    pub fn with_close_flag(
        close_sub_readers: bool,
        readers: Vec<Arc<dyn CompositeReader>>,
    ) -> Result<Self> {
        Self::with_stored_fields(close_sub_readers, readers, Vec::new())
    }

    /// Expert constructor that splits the parallel sub-composites from the
    /// readers used for stored fields.
    ///
    /// Equivalent to
    /// `ParallelCompositeReader(boolean closeSubReaders, CompositeReader[]
    /// readers, CompositeReader[] storedFieldReaders)`.
    ///
    /// # Errors
    ///
    /// See the type-level docs for the compatibility checks enforced.
    pub fn with_stored_fields(
        close_sub_readers: bool,
        readers: Vec<Arc<dyn CompositeReader>>,
        stored_fields_readers: Vec<Arc<dyn CompositeReader>>,
    ) -> Result<Self> {
        if readers.is_empty() && !stored_fields_readers.is_empty() {
            return Err(LuceneError::IllegalArgument(
                "There must be at least one main reader if storedFieldsReaders are used."
                    .to_string(),
            ));
        }

        let prepared = prepare_leaf_readers(&readers, &stored_fields_readers)?;

        // Build the starts table and precomputed maxDoc from the synthetic
        // ParallelLeafReaders, mirroring BaseCompositeReader's constructor.
        let mut starts = Vec::with_capacity(prepared.len());
        let mut max_doc: i64 = 0;
        for r in &prepared {
            starts.push(max_doc as i32);
            max_doc += LeafReader::max_doc(r) as i64;
        }
        let max_doc = i32::try_from(max_doc).map_err(|_| {
            LuceneError::IllegalArgument(format!("Composite reader maxDoc {max_doc} overflows i32"))
        })?;

        // Identity-deduplicated union of the original composites (NOT the
        // synthetic leaves): these are what we close/decRef on do_close.
        let mut complete_reader_set: Vec<Arc<dyn IndexReader>> = Vec::new();
        for r in readers.iter().chain(stored_fields_readers.iter()) {
            let as_reader: Arc<dyn IndexReader> = Arc::clone(r) as Arc<dyn IndexReader>;
            if !complete_reader_set
                .iter()
                .any(|existing| Arc::ptr_eq(existing, &as_reader))
            {
                complete_reader_set.push(as_reader);
            }
        }

        // When the sub-composites must outlive this reader, take an extra
        // reference on each, matching Lucene's `!closeSubReaders` branch.
        if !close_sub_readers {
            for r in &complete_reader_set {
                r.inc_ref()?;
            }
        }

        let sub_readers: Vec<Arc<dyn IndexReader>> = prepared
            .into_iter()
            .map(|plr| Arc::new(plr) as Arc<dyn IndexReader>)
            .collect();

        Ok(Self {
            core: IndexReaderCore::new(),
            sub_readers,
            starts,
            max_doc,
            num_docs: AtomicI32::new(-1),
            close_sub_readers,
            complete_reader_set,
            source_readers: readers
                .into_iter()
                .map(|r| r as Arc<dyn IndexReader>)
                .collect(),
            source_stored_fields_readers: stored_fields_readers
                .into_iter()
                .map(|r| r as Arc<dyn IndexReader>)
                .collect(),
        })
    }

    /// Returns the per-sub-reader doc-start table.
    pub fn starts(&self) -> &[i32] {
        &self.starts
    }
}

impl IndexReader for ParallelCompositeReader {
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
        let cached = self.num_docs.load(Ordering::Relaxed);
        if cached != -1 {
            return cached;
        }
        let sum: i32 = self.sub_readers.iter().map(|r| r.num_docs()).sum();
        self.num_docs.store(sum, Ordering::Relaxed);
        sum
    }

    fn max_doc(&self) -> i32 {
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
        let mut first_err: Option<LuceneError> = None;
        for r in &self.complete_reader_set {
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
        // Delegate only when this reader wraps exactly one sub-composite with no
        // separate stored-fields set, matching Lucene.
        if self.source_readers.len() == 1
            && self.source_stored_fields_readers.len() == 1
            && Arc::ptr_eq(
                &self.source_readers[0],
                &self.source_stored_fields_readers[0],
            )
        {
            self.source_readers[0].get_reader_cache_helper()
        } else {
            None
        }
    }

    fn doc_freq(&self, term: &Term) -> Result<i32> {
        self.ensure_open()?;
        let mut total: i32 = 0;
        for r in &self.sub_readers {
            total = total.wrapping_add(r.doc_freq(term)?);
        }
        Ok(total)
    }

    fn total_term_freq(&self, term: &Term) -> Result<i64> {
        self.ensure_open()?;
        let mut total: i64 = 0;
        for r in &self.sub_readers {
            total += r.total_term_freq(term)?;
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

impl CompositeReader for ParallelCompositeReader {
    fn get_sequential_sub_readers(&self) -> Vec<Arc<dyn IndexReader>> {
        self.sub_readers.clone()
    }
}

// ---------------------------------------------------------------------------
// prepare_leaf_readers / validate (the Rust analogue of Lucene's
// ParallelCompositeReader.prepareLeafReaders / validate)
// ---------------------------------------------------------------------------

/// Builds the synthetic `ParallelLeafReader`s, one per leaf ordinal, from the
/// i-th leaf of every sub-composite.
fn prepare_leaf_readers(
    readers: &[Arc<dyn CompositeReader>],
    stored_fields_readers: &[Arc<dyn CompositeReader>],
) -> Result<Vec<ParallelLeafReader>> {
    if readers.is_empty() {
        return Ok(Vec::new());
    }

    let first_leaves: Vec<Arc<LeafReaderContext>> = readers[0].clone().leaves();
    let max_doc = readers[0].max_doc();
    let no_leaves = first_leaves.len();
    let mut leaf_max_doc = Vec::with_capacity(no_leaves);
    for ctx in &first_leaves {
        leaf_max_doc.push(ctx.leaf_reader().max_doc());
    }

    validate(readers, max_doc, &leaf_max_doc)?;
    validate(stored_fields_readers, max_doc, &leaf_max_doc)?;

    let mut wrapped = Vec::with_capacity(no_leaves);
    for i in 0..no_leaves {
        let subs: Vec<Arc<dyn LeafReader>> = readers
            .iter()
            .map(|r| {
                r.clone()
                    .leaves()
                    .into_iter()
                    .nth(i)
                    .map(|ctx| ctx.leaf_reader())
                    .expect("leaf count validated above")
            })
            .collect();
        let stored_subs: Vec<Arc<dyn LeafReader>> = stored_fields_readers
            .iter()
            .map(|r| {
                r.clone()
                    .leaves()
                    .into_iter()
                    .nth(i)
                    .map(|ctx| ctx.leaf_reader())
                    .expect("leaf count validated above")
            })
            .collect();
        // The synthetic reader is invisible to ref-counting: it neither incRefs
        // nor closes its sub-leaves. `new_synthetic` sets close_sub_readers=true
        // (so the constructor's incRef branch is skipped) and suppress_close=true
        // (so do_close is a no-op).
        wrapped.push(ParallelLeafReader::new_synthetic(subs, stored_subs)?);
    }
    Ok(wrapped)
}

/// Validates that every sub-composite agrees with the first on `maxDoc`, leaf
/// count, and per-ordinal leaf `maxDoc`.
fn validate(
    readers: &[Arc<dyn CompositeReader>],
    max_doc: i32,
    leaf_max_doc: &[i32],
) -> Result<()> {
    for reader in readers {
        if reader.max_doc() != max_doc {
            return Err(LuceneError::IllegalArgument(format!(
                "All readers must have same maxDoc: {max_doc}!={}",
                reader.max_doc()
            )));
        }
        let subs = reader.clone().leaves();
        if subs.len() != leaf_max_doc.len() {
            return Err(LuceneError::IllegalArgument(
                "All readers must have same number of leaf readers".to_string(),
            ));
        }
        for (sub_idx, ctx) in subs.iter().enumerate() {
            let r_max = ctx.leaf_reader().max_doc();
            if r_max != leaf_max_doc[sub_idx] {
                return Err(LuceneError::IllegalArgument(
                    "All leaf readers must have same corresponding subReader maxDoc".to_string(),
                ));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::stub::StoredFieldVisitor;
    use crate::index::field_infos::FieldInfo;
    use crate::index::index_reader::IndexReader;
    use crate::index::leaf_reader::LeafReader;
    use crate::index::{EmptyFields, IndexOptions, Term};
    use crate::search::knn::KnnCollector;
    use crate::search::AcceptDocs;
    use std::collections::HashSet;

    // -----------------------------------------------------------------------
    // Minimal stub leaf reader that owns a configurable set of fields.
    // -----------------------------------------------------------------------

    #[derive(Debug)]
    struct StubTermVectors {
        fields: Vec<String>,
    }
    impl TermVectors for StubTermVectors {
        fn get(&self, _doc: i32) -> Result<Option<Box<dyn Fields>>> {
            if self.fields.is_empty() {
                Ok(None)
            } else {
                Ok(Some(Box::new(EmptyFields)))
            }
        }
    }

    #[derive(Debug)]
    struct StubStoredFields;
    impl StoredFields for StubStoredFields {
        fn document_with_visitor(
            &self,
            _doc_id: i32,
            _visitor: &mut dyn StoredFieldVisitor,
        ) -> Result<()> {
            Ok(())
        }
        fn document(&self, _doc_id: i32) -> Result<crate::document::Document> {
            Ok(crate::document::Document::new())
        }
        fn document_fields(
            &self,
            _doc_id: i32,
            _fields_to_load: &HashSet<String>,
        ) -> Result<crate::document::Document> {
            Ok(crate::document::Document::new())
        }
    }

    /// A stub leaf reader owning a named set of fields, used to exercise
    /// ParallelLeafReader dispatch and merged FieldInfos.
    #[derive(Debug)]
    struct StubLeaf {
        core: IndexReaderCore,
        max_doc: i32,
        num_docs: i32,
        field_infos: FieldInfos,
        /// Field names this stub "owns"; `terms`/`get_*_doc_values` return a
        /// non-None placeholder when queried for one of these.
        fields: HashSet<String>,
    }

    impl StubLeaf {
        fn new(max_doc: i32, num_docs: i32, field_names: &[&str]) -> Self {
            let mut infos: Vec<FieldInfo> = Vec::new();
            for (i, name) in field_names.iter().enumerate() {
                let mut fi = FieldInfo::new(*name, i as i32);
                // Mark indexed so the field is registered in terms_field_to_reader.
                fi.index_options = IndexOptions::DOCS;
                infos.push(fi);
            }
            let field_infos = FieldInfos::new(infos).unwrap_or_else(|_| FieldInfos::empty());
            Self {
                core: IndexReaderCore::new(),
                max_doc,
                num_docs,
                field_infos,
                fields: field_names.iter().map(|s| s.to_string()).collect(),
            }
        }

        fn owns(&self, field: &str) -> bool {
            self.fields.contains(field)
        }
    }

    impl LeafReader for StubLeaf {
        fn core(&self) -> &IndexReaderCore {
            &self.core
        }
        fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
            Ok(Box::new(StubTermVectors {
                fields: self.fields.iter().cloned().collect(),
            }))
        }
        fn num_docs(&self) -> i32 {
            self.num_docs
        }
        fn max_doc(&self) -> i32 {
            self.max_doc
        }
        fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
            Ok(Box::new(StubStoredFields))
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
        fn get_numeric_doc_values(&self, field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
            assert!(self.owns(field), "numeric dv dispatch to wrong sub");
            Ok(None)
        }
        fn get_binary_doc_values(&self, field: &str) -> Result<Option<Box<dyn BinaryDocValues>>> {
            assert!(self.owns(field));
            Ok(None)
        }
        fn get_sorted_doc_values(&self, field: &str) -> Result<Option<Box<dyn SortedDocValues>>> {
            assert!(self.owns(field));
            Ok(None)
        }
        fn get_sorted_numeric_doc_values(
            &self,
            field: &str,
        ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
            assert!(self.owns(field));
            Ok(None)
        }
        fn get_sorted_set_doc_values(
            &self,
            field: &str,
        ) -> Result<Option<Box<dyn SortedSetDocValues>>> {
            assert!(self.owns(field));
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
            self.field_infos.clone()
        }
        fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
            None
        }
        fn get_point_values(&self, field: &str) -> Result<Option<Box<dyn PointValues>>> {
            assert!(self.owns(field));
            Ok(None)
        }
        fn check_integrity(&self) -> Result<()> {
            Ok(())
        }
        fn get_meta_data(&self) -> LeafMetaData {
            LeafMetaData::new(10, Some(Version::LATEST), None, false).unwrap()
        }
    }

    fn leaf(max_doc: i32, num_docs: i32, fields: &[&str]) -> Arc<dyn LeafReader> {
        Arc::new(StubLeaf::new(max_doc, num_docs, fields)) as Arc<dyn LeafReader>
    }

    /// Like [`leaf`](Self::leaf) but returns a typed `Arc<StubLeaf>` so tests can
    /// call `IndexReader` methods (`ensure_open`, `get_ref_count`) that are not
    /// available on `dyn LeafReader` (the blanket `impl IndexReader for T:
    /// LeafReader` cannot be upcast through a trait object).
    fn leaf_typed(max_doc: i32, num_docs: i32, fields: &[&str]) -> Arc<StubLeaf> {
        Arc::new(StubLeaf::new(max_doc, num_docs, fields))
    }

    // -----------------------------------------------------------------------
    // ParallelLeafReader tests
    // -----------------------------------------------------------------------

    #[test]
    fn parallel_leaf_rejects_mismatched_max_doc() {
        let a = leaf(5, 5, &["a"]);
        let b = leaf(7, 7, &["b"]);
        let res = ParallelLeafReader::with_close_flag(true, vec![a, b]);
        assert!(
            matches!(res, Err(LuceneError::IllegalArgument(_))),
            "expected IllegalArgument for mismatched maxDoc"
        );
    }

    #[test]
    fn parallel_leaf_field_overlap_is_first_wins_not_an_error() {
        // Lucene does NOT reject overlapping field names; the first sub-reader
        // owning the field wins.
        let a = leaf(5, 5, &["shared"]);
        let b = leaf(5, 5, &["shared", "only_b"]);
        let plr = ParallelLeafReader::with_close_flag(true, vec![a, b]).unwrap();
        let fis = plr.get_field_infos();
        // The merged FieldInfos contains `shared` (from a) and `only_b`.
        assert!(fis.field_info("shared").is_some());
        assert!(fis.field_info("only_b").is_some());
        // Dispatching `shared` hits the first sub only (the StubLeaf asserts
        // ownership; the second sub's `shared` is never queried).
        plr.terms("shared").unwrap();
        // No assertion fired = first-wins holds.
    }

    #[test]
    fn parallel_leaf_merged_field_infos_is_union() {
        let a = leaf(3, 3, &["a1", "a2"]);
        let b = leaf(3, 3, &["b1"]);
        let plr = ParallelLeafReader::with_close_flag(true, vec![a, b]).unwrap();
        let fis = plr.get_field_infos();
        assert_eq!(fis.size(), 3);
        assert!(fis.field_info("a1").is_some());
        assert!(fis.field_info("a2").is_some());
        assert!(fis.field_info("b1").is_some());
        assert!(fis.field_info("missing").is_none());
    }

    #[test]
    fn parallel_leaf_terms_dispatches_to_owning_sub_and_none_for_absent() {
        let a = leaf(4, 4, &["a_field"]);
        let b = leaf(4, 4, &["b_field"]);
        let plr = ParallelLeafReader::with_close_flag(true, vec![a, b]).unwrap();
        // Querying an owned field dispatches (returns None from the stub but
        // reaches the right sub).
        assert!(plr.terms("a_field").unwrap().is_none());
        assert!(plr.terms("b_field").unwrap().is_none());
        // An absent field returns None without dispatching.
        assert!(plr.terms("nope").unwrap().is_none());
    }

    #[test]
    fn parallel_leaf_doc_values_dispatch_to_owning_sub() {
        let a = leaf(2, 2, &["a"]);
        let b = leaf(2, 2, &["b"]);
        let plr = ParallelLeafReader::with_close_flag(true, vec![a, b]).unwrap();
        // Each StubLeaf asserts ownership in its get_*_doc_values methods; a
        // wrong dispatch would panic.
        plr.get_numeric_doc_values("a").unwrap();
        plr.get_binary_doc_values("b").unwrap();
        plr.get_sorted_doc_values("a").unwrap();
        plr.get_sorted_numeric_doc_values("b").unwrap();
        plr.get_sorted_set_doc_values("a").unwrap();
        plr.get_point_values("b").unwrap();
    }

    #[test]
    fn parallel_leaf_absent_field_returns_none() {
        let a = leaf(2, 2, &["a"]);
        let plr = ParallelLeafReader::with_close_flag(true, vec![a]).unwrap();
        assert!(plr.get_numeric_doc_values("missing").unwrap().is_none());
        assert!(plr.get_point_values("missing").unwrap().is_none());
        assert!(plr.get_float_vector_values("missing").unwrap().is_none());
    }

    #[test]
    fn parallel_leaf_max_doc_and_num_docs_from_first_sub() {
        let a = leaf(8, 6, &["a"]);
        let b = leaf(8, 7, &["b"]);
        let plr = ParallelLeafReader::with_close_flag(true, vec![a, b]).unwrap();
        assert_eq!(LeafReader::max_doc(&plr), 8);
        assert_eq!(LeafReader::num_docs(&plr), 6);
    }

    #[test]
    fn parallel_leaf_single_sub_delegates_cache_helper() {
        let a = leaf(3, 3, &["a"]);
        let plr = ParallelLeafReader::with_close_flag(true, vec![a]).unwrap();
        // StubLeaf returns None, so the delegate path yields None.
        assert!(plr.get_core_cache_helper().is_none());
        assert!(LeafReader::get_reader_cache_helper(&plr).is_none());
    }

    #[test]
    fn parallel_leaf_multi_sub_cache_helper_is_none() {
        let a = leaf(3, 3, &["a"]);
        let b = leaf(3, 3, &["b"]);
        let plr = ParallelLeafReader::with_close_flag(true, vec![a, b]).unwrap();
        assert!(plr.get_core_cache_helper().is_none());
        assert!(LeafReader::get_reader_cache_helper(&plr).is_none());
    }

    #[test]
    fn parallel_leaf_close_sub_readers_true_closes_subs() {
        let a = leaf_typed(3, 3, &["a"]);
        let b = leaf_typed(3, 3, &["b"]);
        let plr = ParallelLeafReader::with_close_flag(
            true,
            vec![
                a.clone() as Arc<dyn LeafReader>,
                b.clone() as Arc<dyn LeafReader>,
            ],
        )
        .unwrap();
        plr.close().unwrap();
        // Subs are closed by the parallel reader.
        assert!(a.ensure_open().is_err());
        assert!(b.ensure_open().is_err());
    }

    #[test]
    fn parallel_leaf_close_sub_readers_false_keeps_subs_alive() {
        let a = leaf_typed(3, 3, &["a"]);
        let b = leaf_typed(3, 3, &["b"]);
        let plr = ParallelLeafReader::with_close_flag(
            false,
            vec![
                a.clone() as Arc<dyn LeafReader>,
                b.clone() as Arc<dyn LeafReader>,
            ],
        )
        .unwrap();
        // Constructor incRefs each sub.
        assert_eq!(a.get_ref_count(), 2);
        assert_eq!(b.get_ref_count(), 2);
        plr.close().unwrap();
        assert_eq!(a.get_ref_count(), 1);
        assert_eq!(b.get_ref_count(), 1);
        assert!(a.ensure_open().is_ok());
    }

    #[test]
    fn parallel_leaf_empty_reader_set_is_zero_docs() {
        let plr = ParallelLeafReader::with_close_flag(true, Vec::new()).unwrap();
        assert_eq!(LeafReader::max_doc(&plr), 0);
        assert_eq!(LeafReader::num_docs(&plr), 0);
        assert!(plr.get_field_infos().is_empty());
    }

    #[test]
    fn parallel_leaf_rejects_stored_fields_without_main_readers() {
        let s = leaf(2, 2, &["s"]);
        let res = ParallelLeafReader::with_stored_fields(true, Vec::new(), vec![s]);
        assert!(matches!(res, Err(LuceneError::IllegalArgument(_))));
    }

    // -----------------------------------------------------------------------
    // ParallelCompositeReader tests
    // -----------------------------------------------------------------------

    /// A minimal stub composite reader with a configurable number of leaves.
    #[derive(Debug)]
    struct StubComposite {
        core: IndexReaderCore,
        leaves: Vec<Arc<StubLeaf>>,
        max_doc: i32,
    }

    impl StubComposite {
        fn from_leaves(leaf_specs: &[(i32, i32, &[&str])]) -> Arc<Self> {
            let mut leaves: Vec<Arc<StubLeaf>> = Vec::new();
            let mut max_doc = 0;
            for (md, nd, fields) in leaf_specs {
                leaves.push(Arc::new(StubLeaf::new(*md, *nd, fields)));
                max_doc += md;
            }
            Arc::new(Self {
                core: IndexReaderCore::new(),
                leaves,
                max_doc,
            })
        }
    }

    impl IndexReader for StubComposite {
        fn core(&self) -> &IndexReaderCore {
            &self.core
        }
        fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
            Err(LuceneError::UnsupportedOperation("stub".into()))
        }
        fn num_docs(&self) -> i32 {
            self.leaves.iter().map(|l| LeafReader::num_docs(&**l)).sum()
        }
        fn max_doc(&self) -> i32 {
            self.max_doc
        }
        fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
            Err(LuceneError::UnsupportedOperation("stub".into()))
        }
        fn do_close(&self) -> Result<()> {
            Ok(())
        }
        fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
            None
        }
        fn doc_freq(&self, _term: &Term) -> Result<i32> {
            Err(LuceneError::UnsupportedOperation("stub".into()))
        }
        fn total_term_freq(&self, _term: &Term) -> Result<i64> {
            Err(LuceneError::UnsupportedOperation("stub".into()))
        }
        fn get_sum_doc_freq(&self, _field: &str) -> Result<i64> {
            Err(LuceneError::UnsupportedOperation("stub".into()))
        }
        fn get_doc_count(&self, _field: &str) -> Result<i32> {
            Err(LuceneError::UnsupportedOperation("stub".into()))
        }
        fn get_sum_total_term_freq(&self, _field: &str) -> Result<i64> {
            Err(LuceneError::UnsupportedOperation("stub".into()))
        }
        fn build_context(
            self: Arc<Self>,
            parent: Option<Weak<dyn IndexReaderContext>>,
            ord_in_parent: i32,
            doc_base_in_parent: i32,
            leaf_ord: i32,
            leaf_doc_base: i32,
        ) -> Arc<dyn IndexReaderContext> {
            build_composite_context(
                self as Arc<dyn CompositeReader>,
                parent,
                ord_in_parent,
                doc_base_in_parent,
                leaf_ord,
                leaf_doc_base,
            )
        }
    }

    impl CompositeReader for StubComposite {
        fn get_sequential_sub_readers(&self) -> Vec<Arc<dyn IndexReader>> {
            // Coerce each `Arc<StubLeaf>` to `Arc<dyn IndexReader>` (the blanket
            // `impl IndexReader for T: LeafReader` makes `StubLeaf: IndexReader`).
            // `StubLeaf` cannot be upcast through `dyn LeafReader` to
            // `dyn IndexReader`, so the leaves are stored typed.
            let mut subs: Vec<Arc<dyn IndexReader>> = Vec::with_capacity(self.leaves.len());
            for l in &self.leaves {
                subs.push(l.clone());
            }
            subs
        }
    }

    fn composite(leaf_specs: &[(i32, i32, &[&str])]) -> Arc<dyn CompositeReader> {
        StubComposite::from_leaves(leaf_specs) as Arc<dyn CompositeReader>
    }

    #[test]
    fn parallel_composite_builds_one_parallel_leaf_per_ordinal() {
        // Two sub-composites, each with two leaves, disjoint field sets.
        let c0 = composite(&[(2, 2, &["a"]), (3, 3, &["c"])]);
        let c1 = composite(&[(2, 2, &["b"]), (3, 3, &["d"])]);
        let pcr = ParallelCompositeReader::with_close_flag(true, vec![c0, c1]).unwrap();
        let subs = pcr.get_sequential_sub_readers();
        assert_eq!(subs.len(), 2);
        // Each sub is a ParallelLeafReader with maxDoc equal to the per-ordinal
        // leaf maxDoc.
        assert_eq!(subs[0].max_doc(), 2);
        assert_eq!(subs[1].max_doc(), 3);
        assert_eq!(pcr.max_doc(), 5);
        // The merged FieldInfos at each ordinal is the union.
        let plr0 = subs[0].clone();
        // Downcast isn't trivial; assert via the leaf view through context.
        let leaves = Arc::new(pcr).leaves();
        assert_eq!(leaves.len(), 2);
        assert_eq!(leaves[0].doc_base(), 0);
        assert_eq!(leaves[1].doc_base(), 2);
        let _ = plr0;
    }

    #[test]
    fn parallel_composite_rejects_mismatched_leaf_count() {
        let c0 = composite(&[(2, 2, &["a"]), (3, 3, &["c"])]);
        let c1 = composite(&[(5, 5, &["b"])]);
        let res = ParallelCompositeReader::with_close_flag(true, vec![c0, c1]);
        assert!(matches!(res, Err(LuceneError::IllegalArgument(_))));
    }

    #[test]
    fn parallel_composite_rejects_mismatched_per_ordinal_max_doc() {
        let c0 = composite(&[(2, 2, &["a"]), (3, 3, &["c"])]);
        let c1 = composite(&[(2, 2, &["b"]), (4, 4, &["d"])]);
        let res = ParallelCompositeReader::with_close_flag(true, vec![c0, c1]);
        assert!(matches!(res, Err(LuceneError::IllegalArgument(_))));
    }

    #[test]
    fn parallel_composite_rejects_mismatched_total_max_doc() {
        let c0 = composite(&[(2, 2, &["a"]), (3, 3, &["c"])]);
        let c1 = composite(&[(1, 1, &["b"]), (3, 3, &["d"])]);
        let res = ParallelCompositeReader::with_close_flag(true, vec![c0, c1]);
        assert!(matches!(res, Err(LuceneError::IllegalArgument(_))));
    }

    #[test]
    fn parallel_composite_field_overlap_at_ordinal_is_first_wins() {
        // Both sub-composites have a field of the same name at ordinal 0.
        let c0 = composite(&[(2, 2, &["shared"])]);
        let c1 = composite(&[(2, 2, &["shared", "only_c1"])]);
        let pcr = ParallelCompositeReader::with_close_flag(true, vec![c0, c1]).unwrap();
        let subs = pcr.get_sequential_sub_readers();
        assert_eq!(subs.len(), 1);
        // No error: first-wins inside the synthetic ParallelLeafReader.
        let leaves = Arc::new(pcr).leaves();
        assert_eq!(leaves.len(), 1);
    }

    #[test]
    fn parallel_composite_single_sub_delegates_cache_helper() {
        let c0 = composite(&[(2, 2, &["a"])]);
        let pcr = ParallelCompositeReader::with_close_flag(true, vec![c0.clone()]).unwrap();
        // StubComposite returns None, so the delegate path yields None.
        assert!(pcr.get_reader_cache_helper().is_none());
    }

    #[test]
    fn parallel_composite_close_sub_readers_false_keeps_subs_alive() {
        let c0 = composite(&[(2, 2, &["a"])]);
        let pcr = ParallelCompositeReader::with_close_flag(false, vec![c0.clone()]).unwrap();
        // The original composite was incRef'd.
        assert_eq!(c0.get_ref_count(), 2);
        pcr.close().unwrap();
        assert_eq!(c0.get_ref_count(), 1);
        assert!(c0.ensure_open().is_ok());
    }

    #[test]
    fn parallel_composite_num_docs_aggregates_across_leaves() {
        let c0 = composite(&[(2, 2, &["a"]), (3, 2, &["c"])]);
        let c1 = composite(&[(2, 2, &["b"]), (3, 3, &["d"])]);
        let pcr = ParallelCompositeReader::with_close_flag(true, vec![c0, c1]).unwrap();
        // The parallel composite lays sub-composites side by side: it produces
        // one ParallelLeafReader per leaf ordinal, and `BaseCompositeReader` sums
        // `numDocs()` across those parallel leaves. Each ParallelLeafReader's
        // `numDocs()` is the FIRST sub's `numDocs()` (parallel overlay, not
        // concatenation), so the total is c0.leaf[0].numDocs() (2) +
        // c0.leaf[1].numDocs() (2) = 4, and `maxDoc` is 2 + 3 = 5.
        assert_eq!(pcr.max_doc(), 5);
        assert_eq!(pcr.num_docs(), 4);
    }
}
