//! KNN-vectors format base traits.
//!
//! Equivalent to `org.apache.lucene.codecs.KnnVectorsFormat`,
//! `KnnVectorsReader`, `KnnVectorsWriter` — with the merged view its nested
//! `MergedVectorValues` builds over the segments of a merge —
//! `KnnFieldVectorsWriter` and `BufferingKnnVectorsWriter`.
//!
//! These traits provide the abstract read/write API for numeric vector fields
//! used for nearest-neighbor search. Concrete codecs implement the
//! format-specific encoding (flat vectors, HNSW graphs, etc.) underneath this
//! API.

#![deny(unsafe_code)]

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, LazyLock, RwLock};

use crate::error::{LuceneError, Result};
use crate::index::merge::{DocIDMerger, DocIDMergerSub, DocMap};
pub use crate::index::vector_values::{
    accept_ords, from_bytes, from_floats, ByteVectorValues, DenseDocIndexIterator,
    DocIndexIterator, FloatVectorValues, FromDisiDocIndexIterator, KnnVectorValues,
    SparseDocIndexIterator,
};
use crate::index::VectorEncoding;
pub use crate::search::knn::{KnnCollector, KnnSearchStrategy, TopDocs};
use crate::search::AcceptDocs;
use crate::search::{DocIdSetIterator, NO_MORE_DOCS};
use crate::util::FixedBitSet;

use super::postings::MergeState;
use super::state::{OwnedSegmentWriteState, SegmentReadState};
use super::stub::{FieldInfo, FieldInfos};

// -----------------------------------------------------------------------------

// Field writer
// -----------------------------------------------------------------------------

/// Vectors writer for a single field.
///
/// Equivalent to `org.apache.lucene.codecs.KnnFieldVectorsWriter`.
pub trait KnnFieldVectorsWriter<T>: Send + Sync {
    /// Adds a new doc ID with its vector value to the given field.
    fn add_value(&mut self, doc_id: i32, vector_value: T) -> Result<()>;

    /// Copies a vector value being indexed to internal storage.
    fn copy_value(&self, vector_value: T) -> T;

    /// Returns the RAM this field writer currently holds.
    ///
    /// Java's `KnnFieldVectorsWriter` implements `Accountable`
    /// (`KnnFieldVectorsWriter.java:28`); the indexing chain adds this to the
    /// segment's footprint through `VectorValuesConsumer.getAccountable`, and
    /// that total is what decides when the `DocumentsWriterPerThread` flushes.
    /// There is deliberately no default: a writer that answered zero would make
    /// a segment full of vectors look free.
    fn ram_bytes_used(&self) -> i64;
}

// -----------------------------------------------------------------------------
// Segment writer
// -----------------------------------------------------------------------------

/// Per-field writer returned by [`KnnVectorsWriter::add_field`].
///
/// Concrete writers return the variant matching the field's
/// [`VectorEncoding`](crate::index::VectorEncoding).
pub enum FieldVectorWriter {
    /// Float-vector field writer.
    Float(Box<dyn KnnFieldVectorsWriter<Vec<f32>>>),
    /// Byte-vector field writer.
    Byte(Box<dyn KnnFieldVectorsWriter<Vec<u8>>>),
}

impl fmt::Debug for FieldVectorWriter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Float(_) => f.write_str("FieldVectorWriter::Float"),
            Self::Byte(_) => f.write_str("FieldVectorWriter::Byte"),
        }
    }
}

impl FieldVectorWriter {
    /// Returns the RAM the underlying field writer holds.
    pub fn ram_bytes_used(&self) -> i64 {
        match self {
            Self::Float(writer) => writer.ram_bytes_used(),
            Self::Byte(writer) => writer.ram_bytes_used(),
        }
    }
}

/// Builds the two ordinal maps a sorted segment needs, from the documents a
/// field carries and the sort's document map.
///
/// Equivalent to `KnnVectorsWriter.mapOldOrdToNewOrd(DocsWithFieldSet, Sorter.DocMap, int[], int[], DocsWithFieldSet)`.
/// Vector ordinals follow document order, so re-ordering the documents
/// re-orders the ordinals: `old_to_new[o]` is where the vector at old ordinal
/// `o` ends up, and `new_to_old` is its inverse.
///
/// # Errors
///
/// Propagates whatever iterating the document set raises.
pub fn map_old_ord_to_new_ord(
    old_doc_ids: &[i32],
    sort_map: &crate::index::index_sorter::SorterDocMap,
) -> Result<(Vec<i32>, Vec<i32>)> {
    let cardinality = old_doc_ids.len();
    let mut new_id_to_old_ord: std::collections::HashMap<i32, i32> =
        std::collections::HashMap::with_capacity(cardinality);
    let mut new_doc_ids = Vec::with_capacity(cardinality);

    // **Divergence from Lucene 10.5.0.** Java takes the `DocsWithFieldSet` and
    // iterates it here. Iterating it produces exactly this list — the documents
    // that carry the field, in ordinal order — and every caller in this port
    // already holds that list, so it is taken directly rather than rebuilt.
    for (old_ord, &old_doc_id) in old_doc_ids.iter().enumerate() {
        let new_id = sort_map.old_to_new(old_doc_id);
        new_id_to_old_ord.insert(new_id, old_ord as i32);
        new_doc_ids.push(new_id);
    }

    new_doc_ids.sort_unstable();
    let mut old_to_new = vec![0i32; cardinality];
    let mut new_to_old = vec![0i32; cardinality];
    for (new_ord, new_doc_id) in new_doc_ids.into_iter().enumerate() {
        let curr_old_ord = new_id_to_old_ord[&new_doc_id];
        old_to_new[curr_old_ord as usize] = new_ord as i32;
        new_to_old[new_ord] = curr_old_ord;
    }
    Ok((old_to_new, new_to_old))
}

/// Writes vectors to an index.
///
/// Equivalent to `org.apache.lucene.codecs.KnnVectorsWriter`.
pub trait KnnVectorsWriter: Send + Sync + fmt::Debug {
    /// Adds a new vector field for indexing.
    ///
    /// The returned variant matches the field's `VectorEncoding`.
    fn add_field(&mut self, field_info: &FieldInfo) -> Result<FieldVectorWriter>;

    /// Flushes all buffered data to disk.
    fn flush(&mut self, max_doc: i32, sort_map: Option<&SorterDocMap>) -> Result<()>;

    /// Called once at the end before close.
    fn finish(&mut self) -> Result<()>;

    /// Merges vectors for a single field, returning deferred work if any.
    ///
    /// Equivalent to `KnnVectorsWriter.mergeOneField(FieldInfo, MergeState)`
    /// (`KnnVectorsWriter.java:72-99`). The default merges naively: it adds the
    /// field, walks the merged view
    /// [`MergedVectorValues`] builds over every segment, hands each surviving
    /// document's vector to the field writer under its merged document id, and
    /// leaves no deferred work. A writer with a two-phase merge overrides it,
    /// writing the flat vectors here and returning the rest — for example an
    /// HNSW graph built from them — as an [`IORunnable`].
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when
    /// [`add_field`](Self::add_field) answers a writer of the other encoding,
    /// where Java's unchecked cast of `addField`'s result would fail, and
    /// propagates whatever building the merged view, reading a vector or
    /// adding it raises.
    fn merge_one_field<'m>(
        &mut self,
        field_info: &FieldInfo,
        merge_state: &'m MergeState,
    ) -> Result<Option<Box<dyn IORunnable + 'm>>> {
        match field_info.vector_encoding {
            VectorEncoding::BYTE => {
                let FieldVectorWriter::Byte(mut byte_writer) = self.add_field(field_info)? else {
                    return Err(LuceneError::IllegalState(format!(
                        "the writer for byte vector field \"{}\" does not accept byte vectors",
                        field_info.name
                    )));
                };
                let merged_bytes =
                    MergedVectorValues::merge_byte_vector_values(field_info, merge_state)?;
                let mut iter = merged_bytes.iterator()?;
                let mut doc = iter.next_doc()?;
                while doc != NO_MORE_DOCS {
                    byte_writer.add_value(doc, merged_bytes.vector_value(iter.index())?)?;
                    doc = iter.next_doc()?;
                }
            }
            VectorEncoding::FLOAT32 => {
                let FieldVectorWriter::Float(mut float_writer) = self.add_field(field_info)? else {
                    return Err(LuceneError::IllegalState(format!(
                        "the writer for float vector field \"{}\" does not accept float vectors",
                        field_info.name
                    )));
                };
                let merged_floats =
                    MergedVectorValues::merge_float_vector_values(field_info, merge_state)?;
                let mut iter = merged_floats.iterator()?;
                let mut doc = iter.next_doc()?;
                while doc != NO_MORE_DOCS {
                    float_writer.add_value(doc, merged_floats.vector_value(iter.index())?)?;
                    doc = iter.next_doc()?;
                }
            }
        }
        Ok(None)
    }

    /// Merges the segment vectors for all fields, in two phases.
    ///
    /// Equivalent to `KnnVectorsWriter.merge(MergeState)`
    /// (`KnnVectorsWriter.java:101-153`), which Java declares `final`; no
    /// writer in this crate overrides it. Every source reader is checked for
    /// integrity first. Phase one calls [`merge_one_field`](Self::merge_one_field)
    /// for every field of the merged segment that has vectors, collecting the
    /// deferred work each returns; phase two runs that work, in the same order;
    /// then the writer is [finished](Self::finish).
    ///
    /// # Errors
    ///
    /// Propagates abort, integrity, merge and finish errors.
    fn merge(&mut self, merge_state: &MergeState) -> Result<()> {
        for i in 0..merge_state.field_infos.len() {
            let reader = merge_state.knn_vectors_readers[i].as_ref();
            debug_assert!(reader.is_some() || !merge_state.field_infos[i].has_vector_values());
            if let Some(reader) = reader {
                merge_state.check_aborted()?;
                reader.check_integrity()?;
            }
        }

        // Phase 1: merge flat vectors for all fields, collecting deferred work.
        let info_stream = &merge_state.info_stream;
        let segment = || match merge_state.segment_info.as_ref() {
            Some(segment_info) => segment_info.to_string_with_del_count(0),
            None => "null".to_string(),
        };
        let mut deferred_work: Vec<Box<dyn IORunnable + '_>> = Vec::new();
        for field_info in merge_state.merge_field_infos.iter() {
            if field_info.has_vector_values() {
                if info_stream.is_enabled("VV") {
                    info_stream.message("VV", &format!("merging {}", segment()));
                }

                if let Some(deferred) = self.merge_one_field(field_info, merge_state)? {
                    deferred_work.push(deferred);
                }

                if info_stream.is_enabled("VV") {
                    info_stream.message("VV", &format!("merge done {}", segment()));
                }
            }
        }

        // Phase 2: execute deferred work (e.g. graph construction using the
        // written flat vectors).
        for runnable in deferred_work.iter_mut() {
            if info_stream.is_enabled("VV") {
                // Java concatenates with no space here.
                info_stream.message("VV", &format!("merging deferred work{}", segment()));
            }

            runnable.run()?;

            if info_stream.is_enabled("VV") {
                info_stream.message("VV", &format!("merge deferred work done {}", segment()));
            }
        }

        self.finish()
    }

    /// Closes this writer, releasing all resources.
    fn close(&mut self) -> Result<()>;

    /// Returns the RAM this writer and every field writer it owns hold.
    ///
    /// Java's `KnnVectorsWriter` implements `Accountable`
    /// (`KnnVectorsWriter.java:46`) and this is the value
    /// `VectorValuesConsumer.getAccountable` exposes to the indexing chain.
    fn ram_bytes_used(&self) -> i64;
}

/// Document-id mapping produced by index sorting.
///
/// Equivalent to `org.apache.lucene.index.Sorter.DocMap`. This is the real
/// mapping the index sorter produces; it used to be a placeholder struct with
/// no fields, which meant no sort map could ever reach a vectors writer and the
/// sorted write path could not be exercised at all.
pub use crate::index::index_sorter::SorterDocMap;

/// Runnable I/O operation returned by deferred merge steps.
///
/// Equivalent to `org.apache.lucene.util.IORunnable`.
pub trait IORunnable: Send + Sync {
    /// Executes the deferred I/O work.
    fn run(&mut self) -> Result<()>;
}

// -----------------------------------------------------------------------------
// Merged vector values
// -----------------------------------------------------------------------------

/// Locks the iteration state a merged view and its iterator share.
///
/// The state is only ever touched by the one thread that merges; a poisoned
/// lock means that thread already panicked inside it, and is reported as an
/// error rather than propagated as a second panic.
fn lock_merged<S: DocIDMergerSub>(
    state: &std::sync::Mutex<MergedIterationState<S>>,
) -> Result<std::sync::MutexGuard<'_, MergedIterationState<S>>> {
    state.lock().map_err(|_| {
        LuceneError::IllegalState("merged vector values state lock poisoned".to_string())
    })
}

/// Tracks the state of one sub-reader of float vectors that is being merged.
///
/// Equivalent to `org.apache.lucene.codecs.KnnVectorsWriter.FloatVectorValuesSub`
/// (`KnnVectorsWriter.java:161-185`), a `DocIDMerger.Sub` over one segment's
/// vectors. Java marks it `@lucene.internal`.
pub struct FloatVectorValuesSub {
    doc_map: DocMap,
    mapped_doc_id: i32,
    /// The segment's vectors.
    values: Box<dyn FloatVectorValues>,
    /// The one iterator over [`Self::values`], taken when the sub is built.
    iterator: Box<dyn DocIndexIterator>,
}

impl FloatVectorValuesSub {
    /// Wraps one segment's vectors and the map from its document ids to the
    /// merged segment's.
    ///
    /// # Errors
    ///
    /// Propagates whatever taking the iterator over `values` raises.
    pub fn new(doc_map: DocMap, values: Box<dyn FloatVectorValues>) -> Result<Self> {
        let iterator = values.iterator()?;
        debug_assert_eq!(iterator.doc_id(), -1);
        Ok(Self {
            doc_map,
            mapped_doc_id: -1,
            values,
            iterator,
        })
    }

    /// Sets the bits of every document up to `up_to` the iterator still has
    /// to visit, shifted down by `offset`.
    ///
    /// Equivalent to `FloatVectorValuesSub.intoBitSet(int, FixedBitSet, int)`,
    /// which forwards to the iterator.
    ///
    /// # Errors
    ///
    /// Propagates the iterator's error.
    pub fn into_bit_set(
        &mut self,
        up_to: i32,
        bit_set: &mut FixedBitSet,
        offset: i32,
    ) -> Result<()> {
        self.iterator.into_bit_set(up_to, bit_set, offset)
    }

    /// Returns the ordinal of the vector the iterator stands on.
    ///
    /// Equivalent to `FloatVectorValuesSub.index()`.
    pub fn index(&self) -> i32 {
        self.iterator.index()
    }
}

impl DocIDMergerSub for FloatVectorValuesSub {
    fn next_doc(&mut self) -> Result<i32> {
        self.iterator.next_doc()
    }

    fn doc_map(&self) -> &DocMap {
        &self.doc_map
    }

    fn mapped_doc_id(&self) -> i32 {
        self.mapped_doc_id
    }

    fn set_mapped_doc_id(&mut self, doc_id: i32) {
        self.mapped_doc_id = doc_id;
    }
}

/// Tracks the state of one sub-reader of byte vectors that is being merged.
///
/// Equivalent to `org.apache.lucene.codecs.KnnVectorsWriter.ByteVectorValuesSub`
/// (`KnnVectorsWriter.java:192-216`), a `DocIDMerger.Sub` over one segment's
/// vectors. Java marks it `@lucene.internal`.
pub struct ByteVectorValuesSub {
    doc_map: DocMap,
    mapped_doc_id: i32,
    /// The segment's vectors.
    values: Box<dyn ByteVectorValues>,
    /// The one iterator over [`Self::values`], taken when the sub is built.
    iterator: Box<dyn DocIndexIterator>,
}

impl ByteVectorValuesSub {
    /// Wraps one segment's vectors and the map from its document ids to the
    /// merged segment's.
    ///
    /// # Errors
    ///
    /// Propagates whatever taking the iterator over `values` raises.
    pub fn new(doc_map: DocMap, values: Box<dyn ByteVectorValues>) -> Result<Self> {
        let iterator = values.iterator()?;
        debug_assert_eq!(iterator.doc_id(), -1);
        Ok(Self {
            doc_map,
            mapped_doc_id: -1,
            values,
            iterator,
        })
    }

    /// Sets the bits of every document up to `up_to` the iterator still has
    /// to visit, shifted down by `offset`.
    ///
    /// Equivalent to `ByteVectorValuesSub.intoBitSet(int, FixedBitSet, int)`,
    /// which forwards to the iterator.
    ///
    /// # Errors
    ///
    /// Propagates the iterator's error.
    pub fn into_bit_set(
        &mut self,
        up_to: i32,
        bit_set: &mut FixedBitSet,
        offset: i32,
    ) -> Result<()> {
        self.iterator.into_bit_set(up_to, bit_set, offset)
    }

    /// Returns the ordinal of the vector the iterator stands on.
    ///
    /// Equivalent to the package-private `ByteVectorValuesSub.index()`.
    pub(crate) fn index(&self) -> i32 {
        self.iterator.index()
    }
}

impl DocIDMergerSub for ByteVectorValuesSub {
    fn next_doc(&mut self) -> Result<i32> {
        self.iterator.next_doc()
    }

    fn doc_map(&self) -> &DocMap {
        &self.doc_map
    }

    fn mapped_doc_id(&self) -> i32 {
        self.mapped_doc_id
    }

    fn set_mapped_doc_id(&mut self, doc_id: i32) {
        self.mapped_doc_id = doc_id;
    }
}

/// View over multiple vector values supporting iterator-style access via
/// [`DocIDMerger`].
///
/// Equivalent to `org.apache.lucene.codecs.KnnVectorsWriter.MergedVectorValues`
/// (`KnnVectorsWriter.java:272-559`). Java declares a `final` class with a
/// private constructor and only static members; an enum with no variants is
/// the Rust type that likewise cannot be instantiated.
pub enum MergedVectorValues {}

impl MergedVectorValues {
    /// Refuses to merge a field whose vectors are not encoded as `expected`.
    ///
    /// Equivalent to the private `MergedVectorValues.validateFieldEncoding`.
    fn validate_field_encoding(field_info: &FieldInfo, expected: VectorEncoding) -> Result<()> {
        debug_assert!(field_info.has_vector_values());
        let field_encoding = field_info.vector_encoding;
        if field_encoding != expected {
            return Err(LuceneError::UnsupportedOperation(format!(
                "Cannot merge vectors encoded as [{field_encoding:?}] as {expected:?}"
            )));
        }
        Ok(())
    }

    /// Returns `true` if `field_infos` has vector values for `field_name`.
    ///
    /// Equivalent to `MergedVectorValues.hasVectorValues(FieldInfos, String)`.
    pub fn has_vector_values(field_infos: &FieldInfos, field_name: &str) -> bool {
        if !field_infos.has_vector_values() {
            return false;
        }
        field_infos
            .field_info(field_name)
            .is_some_and(|info| info.has_vector_values())
    }

    /// Builds one sub per source segment that has vectors for `merging_field`.
    ///
    /// Equivalent to the private generic `MergedVectorValues.mergeVectorValues`:
    /// a segment is skipped when its field infos have no vectors for the field,
    /// when it has no vectors reader, or when the reader has no values for the
    /// field — the `null` Java's `valuesSupplier` may answer, which the
    /// supplier here reports as `None`.
    fn merge_vector_values<V, S>(
        knn_vectors_readers: &[Option<Arc<dyn KnnVectorsReader>>],
        doc_maps: &[DocMap],
        merging_field: &FieldInfo,
        source_field_infos: &[FieldInfos],
        values_supplier: impl Fn(&dyn KnnVectorsReader) -> Result<Option<V>>,
        new_sub: impl Fn(DocMap, V) -> Result<S>,
    ) -> Result<Vec<S>> {
        let mut subs = Vec::new();
        for (i, knn_vectors_reader) in knn_vectors_readers.iter().enumerate() {
            let source_field_info = &source_field_infos[i];
            if !Self::has_vector_values(source_field_info, &merging_field.name) {
                continue;
            }
            if let Some(knn_vectors_reader) = knn_vectors_reader {
                if let Some(values) = values_supplier(knn_vectors_reader.as_ref())? {
                    subs.push(new_sub(Arc::clone(&doc_maps[i]), values)?);
                }
            }
        }
        Ok(subs)
    }

    /// Returns a merged view over all the segments' float vectors.
    ///
    /// Equivalent to `MergedVectorValues.mergeFloatVectorValues(FieldInfo, MergeState)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::UnsupportedOperation`] when the field is not
    /// encoded as `FLOAT32`, and propagates whatever reading a segment's
    /// values raises.
    pub fn merge_float_vector_values(
        field_info: &FieldInfo,
        merge_state: &MergeState,
    ) -> Result<Box<dyn FloatVectorValues>> {
        Self::validate_field_encoding(field_info, VectorEncoding::FLOAT32)?;
        let subs = Self::merge_vector_values(
            &merge_state.knn_vectors_readers,
            &merge_state.doc_maps,
            field_info,
            &merge_state.field_infos,
            |knn_vectors_reader| knn_vectors_reader.get_float_vector_values(&field_info.name),
            FloatVectorValuesSub::new,
        )?;
        Ok(Box::new(MergedFloat32VectorValues::new(subs, merge_state)?))
    }

    /// Returns a merged view over all the segments' byte vectors.
    ///
    /// Equivalent to `MergedVectorValues.mergeByteVectorValues(FieldInfo, MergeState)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::UnsupportedOperation`] when the field is not
    /// encoded as `BYTE`, and propagates whatever reading a segment's values
    /// raises.
    pub fn merge_byte_vector_values(
        field_info: &FieldInfo,
        merge_state: &MergeState,
    ) -> Result<Box<dyn ByteVectorValues>> {
        Self::validate_field_encoding(field_info, VectorEncoding::BYTE)?;
        let subs = Self::merge_vector_values(
            &merge_state.knn_vectors_readers,
            &merge_state.doc_maps,
            field_info,
            &merge_state.field_infos,
            |knn_vectors_reader| knn_vectors_reader.get_byte_vector_values(&field_info.name),
            ByteVectorValuesSub::new,
        )?;
        Ok(Box::new(MergedByteVectorValues::new(subs, merge_state)?))
    }
}

/// The state a merged view shares with the iterators it hands out: the
/// merger over its subs, and where the merger stands.
///
/// Java keeps these as fields of the view object (`docIdMerger`, `docId`,
/// `lastOrd`, `current`) and its anonymous iterator writes them through the
/// enclosing instance. Rust gives the iterator no borrow of the view that
/// outlives the call creating it, so both hold the same state behind an
/// `Arc<Mutex<_>>`; every iterator moves the one merger forward, exactly as
/// every Java iterator of one view does.
struct MergedIterationState<S: DocIDMergerSub> {
    doc_id_merger: DocIDMerger<S>,
    doc_id: i32,
    last_ord: i32,
    /// Index of the sub the merger last returned, or `None` before the first
    /// step and once exhausted — Java's `current`.
    current: Option<usize>,
}

impl<S: DocIDMergerSub> MergedIterationState<S> {
    fn new(doc_id_merger: DocIDMerger<S>) -> Self {
        Self {
            doc_id_merger,
            doc_id: -1,
            last_ord: -1,
            current: None,
        }
    }
}

/// The iterator a merged view hands out.
///
/// Equivalent to the anonymous `DocIndexIterator` that
/// `MergedFloat32VectorValues.iterator()` and `MergedByteVectorValues.iterator()`
/// return: its own `index`, and the view's `docId`, `lastOrd` and `current`,
/// which it moves forward with the view's single merger.
struct MergedDocIndexIterator<S: DocIDMergerSub> {
    state: Arc<std::sync::Mutex<MergedIterationState<S>>>,
    index: i32,
    size: i32,
}

impl<S: DocIDMergerSub> DocIdSetIterator for MergedDocIndexIterator<S> {
    fn doc_id(&self) -> i32 {
        match self.state.lock() {
            Ok(state) => state.doc_id,
            Err(poisoned) => poisoned.into_inner().doc_id,
        }
    }

    fn next_doc(&mut self) -> Result<i32> {
        let mut state = lock_merged(&self.state)?;
        let next = state
            .doc_id_merger
            .next_sub()?
            .map(|sub| sub.mapped_doc_id());
        match next {
            None => {
                state.current = None;
                state.doc_id = NO_MORE_DOCS;
                self.index = NO_MORE_DOCS;
            }
            Some(mapped_doc_id) => {
                state.current = state.doc_id_merger.current_index();
                state.doc_id = mapped_doc_id;
                state.last_ord = state.last_ord.wrapping_add(1);
                self.index = self.index.wrapping_add(1);
            }
        }
        Ok(state.doc_id)
    }

    /// Java throws `UnsupportedOperationException`: a merged view only walks
    /// forward one document at a time.
    fn advance(&mut self, _target: i32) -> Result<i32> {
        Err(LuceneError::UnsupportedOperation(
            "MergedVectorValues iterator does not support advance".to_string(),
        ))
    }

    fn cost(&self) -> i64 {
        i64::from(self.size)
    }
}

impl<S: DocIDMergerSub + Send> DocIndexIterator for MergedDocIndexIterator<S> {
    fn index(&self) -> i32 {
        self.index
    }
}

/// Unified view over several segments containing float vector values.
///
/// Equivalent to
/// `org.apache.lucene.codecs.KnnVectorsWriter.MergedVectorValues.MergedFloat32VectorValues`
/// (`KnnVectorsWriter.java:358-457`), marked `@lucene.internal`. It supports
/// exactly one forward pass: vectors are read in merged document order, one
/// per document, and only the vector of the document the iteration stands on.
pub struct MergedFloat32VectorValues {
    state: Arc<std::sync::Mutex<MergedIterationState<FloatVectorValuesSub>>>,
    size: i32,
}

impl MergedFloat32VectorValues {
    /// Builds the view over `subs`, interleaving them by merged document id
    /// when the merge sorts the index.
    ///
    /// Equivalent to the package-private constructor
    /// `MergedFloat32VectorValues(List<FloatVectorValuesSub>, MergeState)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever positioning the subs of a sorted merge raises.
    pub(crate) fn new(subs: Vec<FloatVectorValuesSub>, merge_state: &MergeState) -> Result<Self> {
        let doc_id_merger = DocIDMerger::new(subs, merge_state.needs_index_sort)?;
        let mut total_size = 0i32;
        for sub in doc_id_merger.subs() {
            total_size = total_size.wrapping_add(sub.values.size());
        }
        Ok(Self {
            state: Arc::new(std::sync::Mutex::new(MergedIterationState::new(
                doc_id_merger,
            ))),
            size: total_size,
        })
    }
}

impl KnnVectorValues for MergedFloat32VectorValues {
    /// Returns the first sub's dimension.
    ///
    /// # Panics
    ///
    /// When there is no sub at all, as Java's `subs.get(0)` throws
    /// `IndexOutOfBoundsException`. The merge paths call
    /// [`try_dimension`](KnnVectorValues::try_dimension) instead.
    fn dimension(&self) -> i32 {
        self.try_dimension()
            .unwrap_or_else(|error| panic!("{error}"))
    }

    /// Returns the first sub's dimension.
    ///
    /// Equivalent to `MergedFloat32VectorValues.dimension()` (`KnnVectorsWriter.java:439-441`).
    ///
    /// # Errors
    ///
    /// **Divergence from Lucene 10.5.0.** When there is no sub — every source
    /// reader answered `null` for the field, which only a corrupt index can
    /// make happen — Java's `subs.get(0)` throws `IndexOutOfBoundsException`,
    /// which `IndexWriter` reports as the merge's failure. Rust has no
    /// exception to throw, so this returns [`LuceneError::IllegalArgument`],
    /// the variant the crate maps `IndexOutOfBoundsException` to, with Java's
    /// message.
    fn try_dimension(&self) -> Result<i32> {
        let state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        match state.doc_id_merger.subs().first() {
            Some(sub) => Ok(sub.values.dimension()),
            None => Err(LuceneError::IllegalArgument(
                "Index 0 out of bounds for length 0".to_string(),
            )),
        }
    }

    fn size(&self) -> i32 {
        self.size
    }

    /// # Panics
    ///
    /// Always: Java's `ordToDoc` throws `UnsupportedOperationException`, and
    /// this method has no error channel to report it through.
    fn ord_to_doc(&self, _ord: i32) -> i32 {
        panic!("UnsupportedOperationException: MergedFloat32VectorValues.ordToDoc")
    }

    /// Equivalent to the inherited `KnnVectorValues.getAcceptOrds(Bits)`,
    /// whose view calls `ordToDoc`, which throws.
    ///
    /// # Panics
    ///
    /// The returned view panics when queried, as `ord_to_doc` does.
    fn get_accept_ords<'a>(
        &self,
        accept_docs: Option<&'a dyn crate::util::Bits>,
    ) -> Option<Box<dyn crate::util::Bits + 'a>> {
        crate::index::accept_ords_with(
            self.size,
            |_| panic!("UnsupportedOperationException: MergedFloat32VectorValues.ordToDoc"),
            accept_docs,
        )
    }

    /// Java's `copy()` throws `UnsupportedOperationException`.
    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        Err(LuceneError::UnsupportedOperation(
            "MergedFloat32VectorValues::copy".to_string(),
        ))
    }

    fn encoding(&self) -> VectorEncoding {
        VectorEncoding::FLOAT32
    }

    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
        Ok(Box::new(MergedDocIndexIterator {
            state: Arc::clone(&self.state),
            index: -1,
            size: self.size,
        }))
    }
}

impl FloatVectorValues for MergedFloat32VectorValues {
    /// Returns the vector of the document the iteration stands on.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] with Java's message when `ord` is
    /// not the ordinal of that document, and when the iteration has not
    /// started or is over — where Java dereferences a `null` `current` and
    /// throws `NullPointerException`.
    fn vector_value(&self, ord: i32) -> Result<Vec<f32>> {
        let state = lock_merged(&self.state)?;
        if ord != state.last_ord {
            return Err(LuceneError::IllegalState(format!(
                "only supports forward iteration with a single iterator: ord={ord}, lastOrd={}",
                state.last_ord
            )));
        }
        let current = state.current.ok_or_else(|| {
            LuceneError::IllegalState(
                "MergedFloat32VectorValues is not positioned on a document".to_string(),
            )
        })?;
        let current = state.doc_id_merger.sub(current);
        current.values.vector_value(current.index())
    }

    /// Java's `copy()` throws `UnsupportedOperationException`.
    fn copy_float(&self) -> Result<Box<dyn FloatVectorValues>> {
        Err(LuceneError::UnsupportedOperation(
            "MergedFloat32VectorValues::copy".to_string(),
        ))
    }
}

/// Unified view over several segments containing byte vector values.
///
/// Equivalent to
/// `org.apache.lucene.codecs.KnnVectorsWriter.MergedVectorValues.MergedByteVectorValues`
/// (`KnnVectorsWriter.java:464-558`), marked `@lucene.internal`. It supports
/// exactly one forward pass: vectors are read in merged document order, one
/// per document, and only the vector of the document the iteration stands on.
pub struct MergedByteVectorValues {
    state: Arc<std::sync::Mutex<MergedIterationState<ByteVectorValuesSub>>>,
    size: i32,
}

impl MergedByteVectorValues {
    /// Builds the view over `subs`, interleaving them by merged document id
    /// when the merge sorts the index.
    ///
    /// Equivalent to the package-private constructor
    /// `MergedByteVectorValues(List<ByteVectorValuesSub>, MergeState)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever positioning the subs of a sorted merge raises.
    pub(crate) fn new(subs: Vec<ByteVectorValuesSub>, merge_state: &MergeState) -> Result<Self> {
        let doc_id_merger = DocIDMerger::new(subs, merge_state.needs_index_sort)?;
        let mut total_size = 0i32;
        for sub in doc_id_merger.subs() {
            total_size = total_size.wrapping_add(sub.values.size());
        }
        Ok(Self {
            state: Arc::new(std::sync::Mutex::new(MergedIterationState::new(
                doc_id_merger,
            ))),
            size: total_size,
        })
    }
}

impl KnnVectorValues for MergedByteVectorValues {
    /// Returns the first sub's dimension.
    ///
    /// # Panics
    ///
    /// When there is no sub at all, as Java's `subs.get(0)` throws
    /// `IndexOutOfBoundsException`. The merge paths call
    /// [`try_dimension`](KnnVectorValues::try_dimension) instead.
    fn dimension(&self) -> i32 {
        self.try_dimension()
            .unwrap_or_else(|error| panic!("{error}"))
    }

    /// Returns the first sub's dimension.
    ///
    /// Equivalent to `MergedByteVectorValues.dimension()` (`KnnVectorsWriter.java:541-543`).
    ///
    /// # Errors
    ///
    /// **Divergence from Lucene 10.5.0.** When there is no sub — every source
    /// reader answered `null` for the field, which only a corrupt index can
    /// make happen — Java's `subs.get(0)` throws `IndexOutOfBoundsException`,
    /// which `IndexWriter` reports as the merge's failure. Rust has no
    /// exception to throw, so this returns [`LuceneError::IllegalArgument`],
    /// the variant the crate maps `IndexOutOfBoundsException` to, with Java's
    /// message.
    fn try_dimension(&self) -> Result<i32> {
        let state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        match state.doc_id_merger.subs().first() {
            Some(sub) => Ok(sub.values.dimension()),
            None => Err(LuceneError::IllegalArgument(
                "Index 0 out of bounds for length 0".to_string(),
            )),
        }
    }

    fn size(&self) -> i32 {
        self.size
    }

    /// # Panics
    ///
    /// Always: Java's `ordToDoc` throws `UnsupportedOperationException`, and
    /// this method has no error channel to report it through.
    fn ord_to_doc(&self, _ord: i32) -> i32 {
        panic!("UnsupportedOperationException: MergedByteVectorValues.ordToDoc")
    }

    /// Equivalent to the inherited `KnnVectorValues.getAcceptOrds(Bits)`,
    /// whose view calls `ordToDoc`, which throws.
    ///
    /// # Panics
    ///
    /// The returned view panics when queried, as `ord_to_doc` does.
    fn get_accept_ords<'a>(
        &self,
        accept_docs: Option<&'a dyn crate::util::Bits>,
    ) -> Option<Box<dyn crate::util::Bits + 'a>> {
        crate::index::accept_ords_with(
            self.size,
            |_| panic!("UnsupportedOperationException: MergedByteVectorValues.ordToDoc"),
            accept_docs,
        )
    }

    /// Java's `copy()` throws `UnsupportedOperationException`.
    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        Err(LuceneError::UnsupportedOperation(
            "MergedByteVectorValues::copy".to_string(),
        ))
    }

    fn encoding(&self) -> VectorEncoding {
        VectorEncoding::BYTE
    }

    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
        Ok(Box::new(MergedDocIndexIterator {
            state: Arc::clone(&self.state),
            index: -1,
            size: self.size,
        }))
    }
}

impl ByteVectorValues for MergedByteVectorValues {
    /// Returns the vector of the document the iteration stands on.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] with Java's message when `ord` is
    /// not the ordinal of that document, and when the iteration has not
    /// started or is over — where Java dereferences a `null` `current` and
    /// throws `NullPointerException`.
    fn vector_value(&self, ord: i32) -> Result<Vec<u8>> {
        let state = lock_merged(&self.state)?;
        if ord != state.last_ord {
            return Err(LuceneError::IllegalState(format!(
                "only supports forward iteration: ord={ord}, lastOrd={}",
                state.last_ord
            )));
        }
        let current = state.current.ok_or_else(|| {
            LuceneError::IllegalState(
                "MergedByteVectorValues is not positioned on a document".to_string(),
            )
        })?;
        let current = state.doc_id_merger.sub(current);
        current.values.vector_value(current.index())
    }

    /// Java's `copy()` throws `UnsupportedOperationException`.
    fn copy_byte(&self) -> Result<Box<dyn ByteVectorValues>> {
        Err(LuceneError::UnsupportedOperation(
            "MergedByteVectorValues::copy".to_string(),
        ))
    }
}

// -----------------------------------------------------------------------------
// Buffering writer
// -----------------------------------------------------------------------------

/// Buffers pending vector values per doc and flushes when the segment flushes.
///
/// Equivalent to `org.apache.lucene.codecs.BufferingKnnVectorsWriter`.
///
/// Concrete implementations provide `write_field_float` and `write_field_byte`.
pub trait BufferingKnnVectorsWriter: KnnVectorsWriter {
    /// Writes the provided float vector field.
    fn write_field_float(
        &mut self,
        field_info: &FieldInfo,
        values: &dyn FloatVectorValues,
        max_doc: i32,
    ) -> Result<()>;

    /// Writes the provided byte vector field.
    fn write_field_byte(
        &mut self,
        field_info: &FieldInfo,
        values: &dyn ByteVectorValues,
        max_doc: i32,
    ) -> Result<()>;
}

// -----------------------------------------------------------------------------
// Reader
// -----------------------------------------------------------------------------

/// Reads vectors from an index.
///
/// Equivalent to `org.apache.lucene.codecs.KnnVectorsReader`.
pub trait KnnVectorsReader: Send + Sync + fmt::Debug {
    /// Checks consistency of this reader.
    fn check_integrity(&self) -> Result<()>;

    /// Returns the float vector values of `field`, or `None` when this reader
    /// holds no vectors for it.
    ///
    /// Equivalent to `KnnVectorsReader.getFloatVectorValues(String)`. Its
    /// contract promises a non-`null` answer for a field with vectors enabled
    /// and leaves any other field undefined; Lucene's implementations answer
    /// `null` for a field they do not hold —
    /// `PerFieldKnnVectorsFormat.FieldsReader` (`PerFieldKnnVectorsFormat.java:285-293`)
    /// and `Lucene104ScalarQuantizedVectorsReader`
    /// (`Lucene104ScalarQuantizedVectorsReader.java:230-234`) — and callers such
    /// as `Lucene104ScalarQuantizedVectorsWriter.mergeAndRecalculateCentroids`
    /// skip a reader that does. That `null` is `None` here.
    fn get_float_vector_values(&self, field: &str) -> Result<Option<Box<dyn FloatVectorValues>>>;

    /// Returns the byte vector values of `field`, or `None` when this reader
    /// holds no vectors for it.
    ///
    /// Equivalent to `KnnVectorsReader.getByteVectorValues(String)`; see
    /// [`get_float_vector_values`](Self::get_float_vector_values) for when
    /// Lucene answers `null`.
    fn get_byte_vector_values(&self, field: &str) -> Result<Option<Box<dyn ByteVectorValues>>>;

    /// Searches the float vector field for the k nearest neighbors to `target`.
    fn search(
        &self,
        field: &str,
        target: &[f32],
        knn_collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()>;

    /// Searches the byte vector field for the k nearest neighbors to `target`.
    fn search_byte(
        &self,
        field: &str,
        target: &[u8],
        knn_collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()>;

    /// Returns the reader this one dispatches `field` to, when it dispatches
    /// per field at all.
    ///
    /// **Divergence from Lucene 10.5.0.** Java narrows the reader with
    /// `vectorsReader instanceof PerFieldKnnVectorsFormat.FieldsReader` before
    /// calling `getFieldReader(field)` on it (`CheckIndex.java:2863-2869`).
    /// Rust cannot narrow a `dyn KnnVectorsReader`, so the question is asked of
    /// the reader itself: only the per-field reader answers, and the default
    /// answers `None` — which a caller reads the same way it reads Java's
    /// failed `instanceof`, by keeping the reader it already has.
    ///
    /// Equivalent to `PerFieldKnnVectorsFormat.FieldsReader#getFieldReader(String)`,
    /// which likewise answers `null` for a field it does not know.
    fn get_field_reader(&self, _field: &str) -> Option<&dyn KnnVectorsReader> {
        None
    }

    /// Returns this reader as a [`FlatVectorsReader`] when it is one, and
    /// `None` when it is not.
    ///
    /// A flat reader stores the raw vectors and nothing else — no graph — so it
    /// cannot answer a nearest-neighbour search; this is how a caller finds
    /// that out.
    ///
    /// **Divergence from Lucene 10.5.0.** Java writes
    /// `vectorsReader instanceof FlatVectorsReader` (`CheckIndex.java:2869`).
    /// Rust cannot narrow a `dyn KnnVectorsReader` to a `dyn FlatVectorsReader`,
    /// so the test is a method the flat readers override, in the same shape as
    /// [`FlatVectorsReader::as_quantized_vectors_reader`]. The default answers
    /// `None`, which is what the `instanceof` answers for every reader that is
    /// not flat.
    ///
    /// [`FlatVectorsReader`]: crate::codecs::hnsw::flat_vectors::FlatVectorsReader
    /// [`FlatVectorsReader::as_quantized_vectors_reader`]: crate::codecs::hnsw::flat_vectors::FlatVectorsReader::as_quantized_vectors_reader
    fn as_flat_vectors_reader(
        &self,
    ) -> Option<&dyn crate::codecs::hnsw::flat_vectors::FlatVectorsReader> {
        None
    }

    /// Returns an instance optimized for merging.
    fn get_merge_instance(&self) -> Result<Box<dyn KnnVectorsReader>>;

    /// Optional: resets or closes merge resources used in the reader.
    fn finish_merge(&mut self) -> Result<()> {
        Ok(())
    }

    /// Returns the desired off-heap memory size for the given field.
    fn get_off_heap_byte_size(&self, _field_info: &FieldInfo) -> HashMap<String, i64> {
        HashMap::new()
    }

    /// Closes this reader, releasing all resources.
    fn close(&mut self) -> Result<()>;
}

/// Merges two off-heap byte-size reports, summing the entries they share.
///
/// Equivalent to the static
/// `KnnVectorsReader.mergeOffHeapByteSizeMaps(Map, Map)`, which a reader that
/// wraps two others uses to report their combined footprint.
pub fn merge_off_heap_byte_size_maps(
    map1: HashMap<String, i64>,
    map2: HashMap<String, i64>,
) -> HashMap<String, i64> {
    let mut merged = map1;
    for (key, value) in map2 {
        *merged.entry(key).or_insert(0) += value;
    }
    merged
}

// -----------------------------------------------------------------------------
// Format
// -----------------------------------------------------------------------------

/// Encodes and decodes numeric vector fields used for KNN search.
///
/// Equivalent to `org.apache.lucene.codecs.KnnVectorsFormat`.
pub trait KnnVectorsFormat: Send + Sync + fmt::Debug {
    /// Returns this format's SPI name.
    fn name(&self) -> &str;

    /// Returns a writer to write vectors to the index.
    ///
    /// # The state is owned, not borrowed
    ///
    /// Every other format factory in this crate takes a borrowed
    /// [`SegmentWriteState`] because its writer is created inside the flush
    /// call and dies there. This one is different: `VectorValuesConsumer`
    /// creates its writer on the first vector field of the first document and
    /// keeps it until the segment flushes, and
    /// [`PerFieldKnnVectorsFormat`](crate::codecs::per_field::PerFieldKnnVectorsFormat)
    /// stores the state so it can build a suffixed sub-state for every field
    /// it later meets. Both need the state to outlive the call, which is what
    /// [`OwnedSegmentWriteState`] is for; see its documentation for the
    /// divergence this records.
    fn fields_writer(&self, state: &OwnedSegmentWriteState) -> Result<Box<dyn KnnVectorsWriter>>;

    /// Returns a reader to read vectors from the index.
    fn fields_reader<'a>(&self, state: &SegmentReadState<'a>) -> Result<Box<dyn KnnVectorsReader>>;

    /// Returns the maximum number of vector dimensions supported for the given
    /// field name.
    fn get_max_dimensions(&self, _field_name: &str) -> i32;
}

// -----------------------------------------------------------------------------
// KNN-vectors format registry
// -----------------------------------------------------------------------------

/// A registry mapping KNN-vectors-format short names to [`KnnVectorsFormat`]
/// implementations.
///
/// The registry intentionally does not use reflection or SPI loading. Formats
/// are registered explicitly with [`KnnVectorsFormatRegistry::register`] and
/// looked up by name with [`KnnVectorsFormatRegistry::for_name`].
#[derive(Debug, Default, Clone)]
pub struct KnnVectorsFormatRegistry {
    formats: Arc<RwLock<HashMap<String, Arc<dyn KnnVectorsFormat>>>>,
}

impl KnnVectorsFormatRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a KNN-vectors format under the given short name.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if `name` is empty, contains
    /// characters other than ASCII alphanumerics, or is longer than 127 bytes.
    /// Returns [`LuceneError::IllegalState`] if the name is already registered.
    pub fn register<F>(&self, name: impl Into<String>, format: F) -> Result<()>
    where
        F: KnnVectorsFormat + 'static,
    {
        let name = name.into();
        super::validate_service_name(&name)?;

        let mut formats = self.formats.write().map_err(|_| {
            LuceneError::IllegalState("KNN-vectors format registry lock was poisoned".to_string())
        })?;

        if formats.contains_key(&name) {
            return Err(LuceneError::IllegalState(format!(
                "KNN-vectors format already registered: {name}"
            )));
        }

        formats.insert(name, Arc::new(format));
        Ok(())
    }

    /// Looks up a KNN-vectors format by name.
    ///
    /// Returns `None` if no format has been registered under the given name.
    /// Registers an already-shared format under its own name.
    ///
    /// The counterpart of [`register`](Self::register) for a format the caller
    /// already holds behind an `Arc`. As in Lucene's `reload`, a name that is
    /// already taken is left alone rather than replaced.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the format's name is not
    /// a valid service name.
    pub fn register_arc(&self, format: Arc<dyn KnnVectorsFormat>) -> Result<()> {
        let name = format.name().to_string();
        crate::codecs::validate_service_name(&name)?;
        let mut formats = self.formats.write().map_err(|_| {
            LuceneError::IllegalState("KNN vectors format registry lock was poisoned".to_string())
        })?;
        // Only new formats are added; an existing one is never replaced.
        formats.entry(name).or_insert(format);
        Ok(())
    }

    pub fn for_name(&self, name: &str) -> Option<Arc<dyn KnnVectorsFormat>> {
        self.formats
            .read()
            .map_err(|_| {
                LuceneError::IllegalState(
                    "KNN-vectors format registry lock was poisoned".to_string(),
                )
            })
            .ok()?
            .get(name)
            .cloned()
    }

    /// Returns the names of all registered KNN-vectors formats, sorted
    /// alphabetically.
    pub fn available_knn_vectors_formats(&self) -> Vec<String> {
        let Ok(formats) = self.formats.read() else {
            return Vec::new();
        };
        let mut names: Vec<String> = formats.keys().cloned().collect();
        names.sort();
        names
    }
}

/// Adds `format` to the global registry under its own name.
///
/// Equivalent to `KnnVectorsFormat.reloadKnnVectorsFormat(ClassLoader)`, which rescans a classloader
/// for formats it has not seen. Rust has no run-time service discovery, so the
/// caller supplies the format.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when the format's name is not a
/// valid service name.
pub fn reload_knn_vectors_format(format: Arc<dyn KnnVectorsFormat>) -> Result<()> {
    GLOBAL_KNN_VECTORS_REGISTRY.register_arc(format)
}

/// The global registry, seeded with the formats this crate ships.
///
/// The `lucene-core` jar lists three KNN-vectors formats in
/// `META-INF/services/org.apache.lucene.codecs.KnnVectorsFormat` —
/// `Lucene99HnswVectorsFormat`, `Lucene104ScalarQuantizedVectorsFormat` and
/// `Lucene104HnswScalarQuantizedVectorsFormat` — which the Java service loader
/// makes resolvable by name without anyone registering them. Rust has no
/// run-time service discovery, so those entries are registered here when the
/// registry is first touched; see the postings registry for the full reasoning.
///
/// All three are seeded. Note that the third registers under
/// `Lucene104HnswBinaryQuantizedVectorsFormat`, which is the string Lucene's
/// `Lucene104HnswScalarQuantizedVectorsFormat.NAME` holds and therefore the one
/// a segment written by Lucene carries — not the class name.
static GLOBAL_KNN_VECTORS_REGISTRY: LazyLock<KnnVectorsFormatRegistry> = LazyLock::new(|| {
    let registry = KnnVectorsFormatRegistry::new();
    for format in [
        Arc::new(crate::codecs::Lucene99HnswVectorsFormat::new()) as Arc<dyn KnnVectorsFormat>,
        Arc::new(
            crate::codecs::lucene104::scalar_quantized_vectors::Lucene104ScalarQuantizedVectorsFormat::new(),
        ),
        Arc::new(
            crate::codecs::lucene104::scalar_quantized_vectors::Lucene104HnswScalarQuantizedVectorsFormat::new(),
        ),
    ] {
        registry
            .register_arc(format)
            .expect("built-in KNN-vectors formats have valid, distinct SPI names");
    }
    registry
});

/// Looks up a KNN-vectors format by name from the global registry.
///
/// Returns `None` if no format has been registered under the given name.
pub fn knn_vectors_for_name(name: &str) -> Option<Arc<dyn KnnVectorsFormat>> {
    GLOBAL_KNN_VECTORS_REGISTRY.for_name(name)
}

/// Returns the names of all KNN-vectors formats registered in the global
/// registry.
pub fn available_knn_vectors_formats() -> Vec<String> {
    GLOBAL_KNN_VECTORS_REGISTRY.available_knn_vectors_formats()
}

/// Registers a KNN-vectors format in the global registry.
///
/// If the format has already been registered, this is a no-op so that formats
/// can safely call it from their constructors.
pub fn register_global_knn_vectors_format<F>(name: &str, format: F) -> Result<()>
where
    F: KnnVectorsFormat + 'static,
{
    if GLOBAL_KNN_VECTORS_REGISTRY.for_name(name).is_some() {
        return Ok(());
    }
    GLOBAL_KNN_VECTORS_REGISTRY.register(name, format)
}

// -----------------------------------------------------------------------------
// No-op implementations
// -----------------------------------------------------------------------------

/// A no-op KNN collector.
#[derive(Debug, Default, Clone)]
pub struct EmptyKnnCollector;

impl KnnCollector for EmptyKnnCollector {
    fn early_terminated(&self) -> bool {
        false
    }

    fn inc_visited_count(&mut self, _count: i32) {}

    fn visited_count(&self) -> i64 {
        0
    }

    fn visit_limit(&self) -> i64 {
        0
    }

    fn k(&self) -> i32 {
        0
    }

    fn collect(&mut self, _doc_id: i32, _similarity: f32) -> bool {
        false
    }

    fn min_competitive_similarity(&self) -> f32 {
        f32::NEG_INFINITY
    }

    fn top_docs(&mut self) -> TopDocs {
        crate::search::empty_top_docs()
    }

    fn get_search_strategy(&self) -> Option<&KnnSearchStrategy> {
        None
    }
}

/// A no-op field vectors writer for float vectors.
#[derive(Debug, Default, Clone)]
pub struct EmptyKnnFieldVectorsWriter;

impl KnnFieldVectorsWriter<Vec<f32>> for EmptyKnnFieldVectorsWriter {
    fn add_value(&mut self, _doc_id: i32, _vector_value: Vec<f32>) -> Result<()> {
        Ok(())
    }

    fn copy_value(&self, vector_value: Vec<f32>) -> Vec<f32> {
        vector_value
    }

    fn ram_bytes_used(&self) -> i64 {
        0
    }
}

/// The writer of a codec that supports no vectors.
///
/// `KnnVectorsFormat.EMPTY` throws `UnsupportedOperationException("Attempt to
/// write EMPTY vector values")` instead of handing out a writer
/// (`codecs/KnnVectorsFormat.java:118-123`); this writer refuses every field
/// with the same error.
#[derive(Debug, Default, Clone)]
pub struct EmptyKnnVectorsWriter;

impl KnnVectorsWriter for EmptyKnnVectorsWriter {
    /// # Errors
    ///
    /// Always returns [`LuceneError::UnsupportedOperation`].
    fn add_field(&mut self, _field_info: &FieldInfo) -> Result<FieldVectorWriter> {
        Err(LuceneError::UnsupportedOperation(
            "Attempt to write EMPTY vector values".to_string(),
        ))
    }

    fn flush(&mut self, _max_doc: i32, _sort_map: Option<&SorterDocMap>) -> Result<()> {
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }

    fn ram_bytes_used(&self) -> i64 {
        0
    }
}

/// A no-op byte-vector field writer.
#[derive(Debug, Default, Clone)]
pub struct EmptyByteKnnFieldVectorsWriter;

impl KnnFieldVectorsWriter<Vec<u8>> for EmptyByteKnnFieldVectorsWriter {
    fn add_value(&mut self, _doc_id: i32, _vector_value: Vec<u8>) -> Result<()> {
        Ok(())
    }

    fn copy_value(&self, vector_value: Vec<u8>) -> Vec<u8> {
        vector_value
    }

    fn ram_bytes_used(&self) -> i64 {
        0
    }
}

/// A no-op buffering KNN vectors writer.
///
/// Test-only: Lucene has no such class.
#[cfg(test)]
#[derive(Debug, Default, Clone)]
pub(crate) struct EmptyBufferingKnnVectorsWriter;

#[cfg(test)]
impl KnnVectorsWriter for EmptyBufferingKnnVectorsWriter {
    fn add_field(&mut self, field_info: &FieldInfo) -> Result<FieldVectorWriter> {
        Ok(match field_info.vector_encoding {
            crate::index::VectorEncoding::FLOAT32 => {
                FieldVectorWriter::Float(Box::new(EmptyKnnFieldVectorsWriter))
            }
            crate::index::VectorEncoding::BYTE => {
                FieldVectorWriter::Byte(Box::new(EmptyByteKnnFieldVectorsWriter))
            }
        })
    }

    fn flush(&mut self, _max_doc: i32, _sort_map: Option<&SorterDocMap>) -> Result<()> {
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }

    fn ram_bytes_used(&self) -> i64 {
        0
    }
}

#[cfg(test)]
impl BufferingKnnVectorsWriter for EmptyBufferingKnnVectorsWriter {
    fn write_field_float(
        &mut self,
        _field_info: &FieldInfo,
        _values: &dyn FloatVectorValues,
        _max_doc: i32,
    ) -> Result<()> {
        Ok(())
    }

    fn write_field_byte(
        &mut self,
        _field_info: &FieldInfo,
        _values: &dyn ByteVectorValues,
        _max_doc: i32,
    ) -> Result<()> {
        Ok(())
    }
}

/// A KNN vectors reader over a segment that supports no vectors.
///
/// The value accessors are equivalent to those of the anonymous
/// `KnnVectorsReader` that `KnnVectorsFormat.EMPTY`'s
/// `fieldsReader(SegmentReadState)` returns
/// (`codecs/KnnVectorsFormat.java:125-140`), which throw
/// `UnsupportedOperationException`.
#[derive(Debug, Default, Clone)]
pub struct EmptyKnnVectorsReader;

impl KnnVectorsReader for EmptyKnnVectorsReader {
    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }

    /// Equivalent to `KnnVectorsFormat.EMPTY`'s reader's
    /// `getFloatVectorValues(String)` (`codecs/KnnVectorsFormat.java:130-133`).
    ///
    /// # Errors
    ///
    /// Always returns [`LuceneError::UnsupportedOperation`].
    fn get_float_vector_values(&self, _field: &str) -> Result<Option<Box<dyn FloatVectorValues>>> {
        Err(LuceneError::UnsupportedOperation(
            "EmptyKnnVectorsReader has no float vector values".to_string(),
        ))
    }

    /// Equivalent to `KnnVectorsFormat.EMPTY`'s reader's
    /// `getByteVectorValues(String)` (`codecs/KnnVectorsFormat.java:135-138`).
    ///
    /// # Errors
    ///
    /// Always returns [`LuceneError::UnsupportedOperation`].
    fn get_byte_vector_values(&self, _field: &str) -> Result<Option<Box<dyn ByteVectorValues>>> {
        Err(LuceneError::UnsupportedOperation(
            "EmptyKnnVectorsReader has no byte vector values".to_string(),
        ))
    }

    fn search(
        &self,
        _field: &str,
        _target: &[f32],
        _knn_collector: &mut dyn KnnCollector,
        _accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        // `KnnVectorsFormat.EMPTY`'s reader throws from both searches
        // (`codecs/KnnVectorsFormat.java:142-153`).
        Err(LuceneError::UnsupportedOperation(
            "EmptyKnnVectorsReader does not search".to_string(),
        ))
    }

    fn search_byte(
        &self,
        _field: &str,
        _target: &[u8],
        _knn_collector: &mut dyn KnnCollector,
        _accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        // `KnnVectorsFormat.EMPTY`'s reader throws from both searches
        // (`codecs/KnnVectorsFormat.java:142-153`).
        Err(LuceneError::UnsupportedOperation(
            "EmptyKnnVectorsReader does not search".to_string(),
        ))
    }

    fn get_merge_instance(&self) -> Result<Box<dyn KnnVectorsReader>> {
        Ok(Box::new(self.clone()))
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// A KNN vectors format that supports no vectors.
///
/// Equivalent to `KnnVectorsFormat.EMPTY` (`codecs/KnnVectorsFormat.java:114-162`):
/// writing throws, the reader holds no values.
#[derive(Debug, Default, Clone)]
pub struct EmptyKnnVectorsFormat {
    name: String,
}

impl EmptyKnnVectorsFormat {
    /// Creates a new no-op KNN vectors format with the given SPI name.
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

impl KnnVectorsFormat for EmptyKnnVectorsFormat {
    fn name(&self) -> &str {
        &self.name
    }

    /// Equivalent to `KnnVectorsFormat.EMPTY.fieldsWriter(SegmentWriteState)`
    /// (`codecs/KnnVectorsFormat.java:120-123`).
    ///
    /// # Errors
    ///
    /// Always returns [`LuceneError::UnsupportedOperation`].
    fn fields_writer(&self, _state: &OwnedSegmentWriteState) -> Result<Box<dyn KnnVectorsWriter>> {
        Err(LuceneError::UnsupportedOperation(
            "Attempt to write EMPTY vector values".to_string(),
        ))
    }

    fn fields_reader<'a>(
        &self,
        _state: &SegmentReadState<'a>,
    ) -> Result<Box<dyn KnnVectorsReader>> {
        Ok(Box::new(EmptyKnnVectorsReader))
    }

    /// Equivalent to `KnnVectorsFormat.EMPTY.getMaxDimensions(String)`
    /// (`codecs/KnnVectorsFormat.java:164-166`).
    fn get_max_dimensions(&self, _field_name: &str) -> i32 {
        0
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::stub::{BufferedUpdates, FieldInfos};
    use crate::search::from_live_docs;

    #[test]
    fn empty_knn_collector_is_empty() {
        let mut collector = EmptyKnnCollector;
        assert!(!collector.early_terminated());
        assert_eq!(collector.visited_count(), 0);
        assert_eq!(collector.visit_limit(), 0);
        assert_eq!(collector.k(), 0);
        assert!(!collector.collect(0, 1.0));
        assert_eq!(collector.min_competitive_similarity(), f32::NEG_INFINITY);
        assert!(collector.get_search_strategy().is_none());
    }

    #[test]
    fn empty_knn_field_writer_accepts_vectors() {
        let mut writer = EmptyKnnFieldVectorsWriter;
        writer.add_value(0, vec![1.0, 2.0, 3.0]).unwrap();
        assert_eq!(writer.copy_value(vec![4.0]), vec![4.0]);
    }

    #[test]
    fn empty_knn_vectors_writer_lifecycle() {
        let mut writer = EmptyKnnVectorsWriter;
        let field = FieldInfo::default();
        assert!(matches!(
            writer.add_field(&field),
            Err(LuceneError::UnsupportedOperation(_))
        ));
        writer.flush(10, None).unwrap();
        writer.finish().unwrap();
        writer.close().unwrap();
    }

    #[test]
    fn empty_buffering_knn_vectors_writer_lifecycle() {
        let mut writer = EmptyBufferingKnnVectorsWriter;
        let field = FieldInfo::default();
        writer
            .write_field_float(&field, &from_floats(Vec::new(), 0), 10)
            .unwrap();
        writer
            .write_field_byte(&field, &from_bytes(Vec::new(), 0), 10)
            .unwrap();
        writer.flush(10, None).unwrap();
        writer.finish().unwrap();
        writer.close().unwrap();
    }

    #[test]
    fn empty_knn_vectors_reader_lifecycle() {
        let mut reader = EmptyKnnVectorsReader;
        // `KnnVectorsFormat.EMPTY`'s reader throws from both value accessors
        // (`codecs/KnnVectorsFormat.java:130-138`).
        assert!(matches!(
            reader.get_float_vector_values("f"),
            Err(LuceneError::UnsupportedOperation(_))
        ));
        assert!(matches!(
            reader.get_byte_vector_values("f"),
            Err(LuceneError::UnsupportedOperation(_))
        ));
        let mut collector = EmptyKnnCollector;
        let mut accept_docs = from_live_docs(None, 10).unwrap();
        // Both searches throw (`codecs/KnnVectorsFormat.java:142-153`).
        assert!(matches!(
            reader.search("f", &[1.0, 2.0], &mut collector, &mut accept_docs),
            Err(LuceneError::UnsupportedOperation(_))
        ));
        assert!(matches!(
            reader.search_byte("f", &[1, 2], &mut collector, &mut accept_docs),
            Err(LuceneError::UnsupportedOperation(_))
        ));
        reader.check_integrity().unwrap();
        let _merge = reader.get_merge_instance().unwrap();
        reader.finish_merge().unwrap();
        reader.close().unwrap();
    }

    #[test]
    fn empty_knn_vectors_format_name_and_factories() {
        let format = EmptyKnnVectorsFormat::new("EmptyKnn");
        assert_eq!(format.name(), "EmptyKnn");
        assert_eq!(format.get_max_dimensions("field"), 0);

        let dir: Arc<dyn crate::store::Directory> = Arc::new(crate::store::RamDirectory::default());
        let context = &*crate::store::DEFAULT_IO_CONTEXT;
        let field_infos = FieldInfos::default();
        let segment_info = crate::codecs::tests::test_segment_info("test", 10);
        let write_state = OwnedSegmentWriteState::new(
            Arc::new(crate::util::NoOutputInfoStream),
            Arc::clone(&dir),
            segment_info.clone(),
            field_infos.clone(),
            BufferedUpdates,
            Arc::new(crate::store::DefaultIOContext::default()),
        );
        // `KnnVectorsFormat.EMPTY` throws when written
        // (`codecs/KnnVectorsFormat.java:120-123`).
        assert!(matches!(
            format.fields_writer(&write_state),
            Err(LuceneError::UnsupportedOperation(_))
        ));

        let dir_ref: &dyn crate::store::Directory = &*dir;
        let read_state = SegmentReadState::new(dir_ref, &segment_info, &field_infos, context);
        let _reader = format.fields_reader(&read_state).unwrap();
    }

    // -------------------------------------------------------------------------
    // MergedVectorValues
    // -------------------------------------------------------------------------

    /// Port of `TestMergedVectorValues.testSkipsInMergedByteVectorValues`:
    /// two iterators of one view move the same merger, so after a document is
    /// skipped through one of them the next document's vector is the one the
    /// view answers.
    #[test]
    fn skips_in_merged_byte_vector_values() {
        let vectors = vec![vec![0u8], vec![1u8]];
        let sub = ByteVectorValuesSub::new(
            Arc::new(|doc: i32| doc),
            Box::new(from_bytes(vectors.clone(), 1)),
        )
        .unwrap();
        let state = MergeState::from_fields_producers(Vec::new(), Vec::new());
        let values = MergedByteVectorValues::new(vec![sub], &state).unwrap();

        values.iterator().unwrap().next_doc().unwrap(); // doc 0
        values.iterator().unwrap().next_doc().unwrap(); // doc 1

        assert_eq!(values.vector_value(1).unwrap(), vectors[1]);
    }

    /// A merged view with no sub — no source answered vectors for the field —
    /// has no first sub to read its dimension from: `try_dimension` reports
    /// Java's `IndexOutOfBoundsException` (`KnnVectorsWriter.java:439-441,
    /// 541-543`) as an error with Java's message, for both encodings, while a
    /// view with a sub answers that sub's dimension.
    #[test]
    fn an_empty_merged_view_has_no_dimension() {
        let state = MergeState::from_fields_producers(Vec::new(), Vec::new());
        let floats = MergedFloat32VectorValues::new(Vec::new(), &state).unwrap();
        let bytes = MergedByteVectorValues::new(Vec::new(), &state).unwrap();
        for result in [floats.try_dimension(), bytes.try_dimension()] {
            match result {
                Err(LuceneError::IllegalArgument(message)) => {
                    assert_eq!(message, "Index 0 out of bounds for length 0")
                }
                other => panic!("expected an IllegalArgument, got {other:?}"),
            }
        }

        // Positive control: one sub of two dimensions.
        let float_sub = FloatVectorValuesSub::new(
            Arc::new(|doc: i32| doc),
            Box::new(from_floats(vec![vec![1.0, 2.0]], 2)),
        )
        .unwrap();
        let floats = MergedFloat32VectorValues::new(vec![float_sub], &state).unwrap();
        assert_eq!(floats.try_dimension().unwrap(), 2);
        let byte_sub = ByteVectorValuesSub::new(
            Arc::new(|doc: i32| doc),
            Box::new(from_bytes(vec![vec![1u8, 2u8]], 2)),
        )
        .unwrap();
        let bytes = MergedByteVectorValues::new(vec![byte_sub], &state).unwrap();
        assert_eq!(bytes.try_dimension().unwrap(), 2);
    }

    /// Port of `TestMergedVectorValues.testSkipsInMergedFloat32VectorValues`.
    #[test]
    fn skips_in_merged_float32_vector_values() {
        let vectors = vec![vec![0.0f32], vec![1.0f32]];
        let sub = FloatVectorValuesSub::new(
            Arc::new(|doc: i32| doc),
            Box::new(from_floats(vectors.clone(), 1)),
        )
        .unwrap();
        let state = MergeState::from_fields_producers(Vec::new(), Vec::new());
        let values = MergedFloat32VectorValues::new(vec![sub], &state).unwrap();

        values.iterator().unwrap().next_doc().unwrap(); // doc 0
        values.iterator().unwrap().next_doc().unwrap(); // doc 1

        assert_eq!(values.vector_value(1).unwrap(), vectors[1]);
    }

    /// A merged view only answers the vector of the document it stands on,
    /// with Java's message, and refuses `advance`, `copy` and a read past the
    /// end; `cost` is the total number of vectors.
    #[test]
    fn a_merged_view_supports_one_forward_pass_only() {
        let float_sub = FloatVectorValuesSub::new(
            Arc::new(|doc: i32| doc),
            Box::new(from_floats(vec![vec![1.0], vec![2.0]], 1)),
        )
        .unwrap();
        let state = MergeState::from_fields_producers(Vec::new(), Vec::new());
        let floats = MergedFloat32VectorValues::new(vec![float_sub], &state).unwrap();
        let mut iterator = floats.iterator().unwrap();
        assert_eq!(iterator.cost(), 2);
        assert_eq!(iterator.doc_id(), -1);
        assert!(matches!(
            iterator.advance(1),
            Err(LuceneError::UnsupportedOperation(_))
        ));
        assert_eq!(iterator.next_doc().unwrap(), 0);
        match floats.vector_value(1) {
            Err(LuceneError::IllegalState(message)) => assert_eq!(
                message,
                "only supports forward iteration with a single iterator: ord=1, lastOrd=0"
            ),
            other => panic!("expected the forward-only refusal, got {other:?}"),
        }
        assert!(matches!(
            floats.copy(),
            Err(LuceneError::UnsupportedOperation(_))
        ));
        assert!(matches!(
            floats.copy_float(),
            Err(LuceneError::UnsupportedOperation(_))
        ));
        assert_eq!(iterator.next_doc().unwrap(), 1);
        assert_eq!(iterator.next_doc().unwrap(), NO_MORE_DOCS);
        assert_eq!(iterator.index(), NO_MORE_DOCS);
        assert_eq!(iterator.doc_id(), NO_MORE_DOCS);
        assert!(
            floats.vector_value(1).is_err(),
            "nothing is current once the pass is over"
        );

        let byte_sub = ByteVectorValuesSub::new(
            Arc::new(|doc: i32| doc),
            Box::new(from_bytes(vec![vec![1], vec![2]], 1)),
        )
        .unwrap();
        let bytes = MergedByteVectorValues::new(vec![byte_sub], &state).unwrap();
        bytes.iterator().unwrap().next_doc().unwrap();
        match bytes.vector_value(1) {
            Err(LuceneError::IllegalState(message)) => {
                assert_eq!(message, "only supports forward iteration: ord=1, lastOrd=0")
            }
            other => panic!("expected the forward-only refusal, got {other:?}"),
        }
    }

    /// A vectors reader serving fixed float and byte vectors, every document
    /// of the segment having one, and counting integrity checks.
    #[derive(Debug, Default)]
    struct ListVectorsReader {
        floats: HashMap<String, Vec<Vec<f32>>>,
        bytes: HashMap<String, Vec<Vec<u8>>>,
        integrity_checks: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl KnnVectorsReader for ListVectorsReader {
        fn check_integrity(&self) -> Result<()> {
            self.integrity_checks
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }

        fn get_float_vector_values(
            &self,
            field: &str,
        ) -> Result<Option<Box<dyn FloatVectorValues>>> {
            let Some(vectors) = self.floats.get(field).cloned() else {
                return Ok(None);
            };
            let dim = vectors[0].len() as i32;
            Ok(Some(Box::new(from_floats(vectors, dim))))
        }

        fn get_byte_vector_values(&self, field: &str) -> Result<Option<Box<dyn ByteVectorValues>>> {
            let Some(vectors) = self.bytes.get(field).cloned() else {
                return Ok(None);
            };
            let dim = vectors[0].len() as i32;
            Ok(Some(Box::new(from_bytes(vectors, dim))))
        }

        fn search(
            &self,
            _field: &str,
            _target: &[f32],
            _knn_collector: &mut dyn KnnCollector,
            _accept_docs: &mut dyn AcceptDocs,
        ) -> Result<()> {
            Ok(())
        }

        fn search_byte(
            &self,
            _field: &str,
            _target: &[u8],
            _knn_collector: &mut dyn KnnCollector,
            _accept_docs: &mut dyn AcceptDocs,
        ) -> Result<()> {
            Ok(())
        }

        fn get_merge_instance(&self) -> Result<Box<dyn KnnVectorsReader>> {
            Err(LuceneError::UnsupportedOperation(
                "not needed by these tests".to_string(),
            ))
        }

        fn close(&mut self) -> Result<()> {
            Ok(())
        }
    }

    fn vector_field(name: &str, number: i32, encoding: VectorEncoding) -> FieldInfo {
        let mut field = FieldInfo::new(name, number);
        field.vector_dimension = 1;
        field.vector_encoding = encoding;
        field
    }

    /// A merge state over segments holding `readers`, with the given doc maps
    /// and per-segment field infos.
    fn vectors_merge_state(
        readers: Vec<Option<Arc<dyn KnnVectorsReader>>>,
        doc_maps: Vec<DocMap>,
        field_infos: Vec<FieldInfos>,
        merge_field_infos: FieldInfos,
        needs_index_sort: bool,
    ) -> MergeState {
        let n = readers.len();
        let mut state =
            MergeState::from_fields_producers((0..n).map(|_| None).collect(), vec![0; n])
                .with_field_infos(field_infos, merge_field_infos);
        state.doc_maps = doc_maps;
        state.knn_vectors_readers = readers;
        state.needs_index_sort = needs_index_sort;
        state
    }

    fn walk_floats(values: &dyn FloatVectorValues) -> Vec<(i32, Vec<f32>)> {
        let mut iterator = values.iterator().unwrap();
        let mut out = Vec::new();
        loop {
            let doc = iterator.next_doc().unwrap();
            if doc == NO_MORE_DOCS {
                break;
            }
            out.push((doc, values.vector_value(iterator.index()).unwrap()));
        }
        out
    }

    /// The merged view walks every segment that has vectors for the field, in
    /// merged order, under the merged document ids, and skips a segment whose
    /// field infos have no vectors for it even when its reader would answer.
    #[test]
    fn merge_float_vector_values_maps_every_surviving_document() {
        let field = vector_field("v", 0, VectorEncoding::FLOAT32);
        let with_field = FieldInfos::new(vec![field.clone()]).unwrap();
        let without_field = FieldInfos::new(vec![FieldInfo::new("v", 0)]).unwrap();
        let reader = |values: Vec<f32>| {
            let mut reader = ListVectorsReader::default();
            reader.floats.insert(
                "v".to_string(),
                values.into_iter().map(|v| vec![v]).collect(),
            );
            Some(Arc::new(reader) as Arc<dyn KnnVectorsReader>)
        };
        // Segment 0 keeps docs 0 and 2 of three; segment 1 has no vectors for
        // the field; segment 2 holds two documents.
        let doc_maps: Vec<DocMap> = vec![
            Arc::new(|doc: i32| match doc {
                0 => 0,
                2 => 1,
                _ => -1,
            }),
            Arc::new(|doc: i32| 2 + doc),
            Arc::new(|doc: i32| 4 + doc),
        ];
        let state = vectors_merge_state(
            vec![
                reader(vec![10.0, 11.0, 12.0]),
                reader(vec![99.0]),
                reader(vec![20.0, 21.0]),
            ],
            doc_maps,
            vec![with_field.clone(), without_field, with_field.clone()],
            with_field,
            false,
        );
        let merged = MergedVectorValues::merge_float_vector_values(&field, &state).unwrap();
        assert_eq!(merged.size(), 5, "size counts every vector of every sub");
        assert_eq!(merged.dimension(), 1);
        assert_eq!(
            walk_floats(merged.as_ref()),
            vec![
                (0, vec![10.0]),
                (1, vec![12.0]),
                (4, vec![20.0]),
                (5, vec![21.0])
            ]
        );
    }

    /// When the merge sorts the index, the subs are interleaved by merged
    /// document id.
    #[test]
    fn merge_byte_vector_values_interleaves_a_sorted_merge() {
        let field = vector_field("b", 0, VectorEncoding::BYTE);
        let infos = FieldInfos::new(vec![field.clone()]).unwrap();
        let reader = |values: Vec<u8>| {
            let mut reader = ListVectorsReader::default();
            reader.bytes.insert(
                "b".to_string(),
                values.into_iter().map(|v| vec![v]).collect(),
            );
            Some(Arc::new(reader) as Arc<dyn KnnVectorsReader>)
        };
        let doc_maps: Vec<DocMap> = vec![
            Arc::new(|doc: i32| doc * 2),
            Arc::new(|doc: i32| doc * 2 + 1),
        ];
        let state = vectors_merge_state(
            vec![reader(vec![0, 2, 4]), reader(vec![1, 3])],
            doc_maps,
            vec![infos.clone(), infos.clone()],
            infos,
            true,
        );
        let merged = MergedVectorValues::merge_byte_vector_values(&field, &state).unwrap();
        let mut iterator = merged.iterator().unwrap();
        let mut out = Vec::new();
        loop {
            let doc = iterator.next_doc().unwrap();
            if doc == NO_MORE_DOCS {
                break;
            }
            out.push((doc, merged.vector_value(iterator.index()).unwrap()[0]));
        }
        assert_eq!(out, vec![(0, 0), (1, 1), (2, 2), (3, 3), (4, 4)]);
    }

    /// A field of one encoding cannot be merged as the other, with Java's
    /// message.
    #[test]
    fn merging_a_field_as_the_wrong_encoding_is_refused() {
        let field = vector_field("b", 0, VectorEncoding::BYTE);
        let infos = FieldInfos::new(vec![field.clone()]).unwrap();
        let state = vectors_merge_state(
            vec![None],
            vec![Arc::new(|doc: i32| doc)],
            vec![infos.clone()],
            infos,
            false,
        );
        match MergedVectorValues::merge_float_vector_values(&field, &state) {
            Err(LuceneError::UnsupportedOperation(message)) => {
                assert_eq!(message, "Cannot merge vectors encoded as [BYTE] as FLOAT32")
            }
            other => panic!(
                "expected the encoding refusal, got {:?}",
                other.map(|v| v.size())
            ),
        }
    }

    #[test]
    fn has_vector_values_needs_the_segment_and_the_field_to_have_vectors() {
        let with = FieldInfos::new(vec![vector_field("v", 0, VectorEncoding::FLOAT32)]).unwrap();
        assert!(MergedVectorValues::has_vector_values(&with, "v"));
        assert!(!MergedVectorValues::has_vector_values(&with, "w"));
        let without = FieldInfos::new(vec![FieldInfo::new("v", 0)]).unwrap();
        assert!(!MergedVectorValues::has_vector_values(&without, "v"));
    }

    /// Everything a writer under test was asked to do, in order.
    type Events = Arc<std::sync::Mutex<Vec<String>>>;

    /// A field writer that records each vector it is handed.
    struct RecordingFieldWriter {
        field: String,
        events: Events,
    }

    impl KnnFieldVectorsWriter<Vec<f32>> for RecordingFieldWriter {
        fn add_value(&mut self, doc_id: i32, vector_value: Vec<f32>) -> Result<()> {
            self.events
                .lock()
                .unwrap()
                .push(format!("add {} doc={doc_id} {vector_value:?}", self.field));
            Ok(())
        }

        fn copy_value(&self, vector_value: Vec<f32>) -> Vec<f32> {
            vector_value
        }

        fn ram_bytes_used(&self) -> i64 {
            0
        }
    }

    /// Deferred work that records when it runs.
    struct RecordingRunnable {
        field: String,
        events: Events,
    }

    impl IORunnable for RecordingRunnable {
        fn run(&mut self) -> Result<()> {
            self.events
                .lock()
                .unwrap()
                .push(format!("deferred {}", self.field));
            Ok(())
        }
    }

    /// A writer that keeps the default `merge` and `merge_one_field`, except
    /// that `defer` makes its `merge_one_field` merge naively and then hand
    /// back deferred work, as a two-phase writer does.
    #[derive(Debug)]
    struct RecordingVectorsWriter {
        events: Events,
        defer: bool,
    }

    impl KnnVectorsWriter for RecordingVectorsWriter {
        fn add_field(&mut self, field_info: &FieldInfo) -> Result<FieldVectorWriter> {
            self.events
                .lock()
                .unwrap()
                .push(format!("add_field {}", field_info.name));
            Ok(FieldVectorWriter::Float(Box::new(RecordingFieldWriter {
                field: field_info.name.clone(),
                events: Arc::clone(&self.events),
            })))
        }

        fn flush(&mut self, _max_doc: i32, _sort_map: Option<&SorterDocMap>) -> Result<()> {
            Ok(())
        }

        fn finish(&mut self) -> Result<()> {
            self.events.lock().unwrap().push("finish".to_string());
            Ok(())
        }

        fn merge_one_field<'m>(
            &mut self,
            field_info: &FieldInfo,
            merge_state: &'m MergeState,
        ) -> Result<Option<Box<dyn IORunnable + 'm>>> {
            // The default body, reached through a helper writer that does not
            // override it.
            let mut naive = NaiveWriter {
                inner: RecordingVectorsWriter {
                    events: Arc::clone(&self.events),
                    defer: false,
                },
            };
            let deferred = naive.merge_one_field(field_info, merge_state)?;
            assert!(deferred.is_none(), "the default leaves no deferred work");
            Ok(self.defer.then(|| {
                Box::new(RecordingRunnable {
                    field: field_info.name.clone(),
                    events: Arc::clone(&self.events),
                }) as Box<dyn IORunnable>
            }))
        }

        fn close(&mut self) -> Result<()> {
            Ok(())
        }

        fn ram_bytes_used(&self) -> i64 {
            0
        }
    }

    /// Delegates everything but `merge_one_field`, which it leaves to the
    /// trait's default.
    #[derive(Debug)]
    struct NaiveWriter {
        inner: RecordingVectorsWriter,
    }

    impl KnnVectorsWriter for NaiveWriter {
        fn add_field(&mut self, field_info: &FieldInfo) -> Result<FieldVectorWriter> {
            self.inner.add_field(field_info)
        }

        fn flush(&mut self, max_doc: i32, sort_map: Option<&SorterDocMap>) -> Result<()> {
            self.inner.flush(max_doc, sort_map)
        }

        fn finish(&mut self) -> Result<()> {
            self.inner.finish()
        }

        fn close(&mut self) -> Result<()> {
            self.inner.close()
        }

        fn ram_bytes_used(&self) -> i64 {
            0
        }
    }

    /// An info stream that keeps every `VV` message.
    struct VvInfoStream(std::sync::Mutex<Vec<String>>);

    impl crate::util::InfoStream for VvInfoStream {
        fn java_class_name(&self) -> &'static str {
            "org.apache.lucene.tests.util.RecordingInfoStream"
        }

        fn message(&self, component: &str, message: &str) {
            assert_eq!(component, "VV");
            self.0.lock().unwrap().push(message.to_string());
        }

        fn is_enabled(&self, component: &str) -> bool {
            component == "VV"
        }

        fn close(&self) {}
    }

    /// `merge` checks every source reader, merges each vector field through
    /// `merge_one_field` — the default adding every surviving vector under its
    /// merged document id — then runs the deferred work of all fields, and
    /// finishes last (`KnnVectorsWriter.java:72-153`). Fields without vectors
    /// are left alone, and each step is reported on the `VV` component.
    #[test]
    fn merge_runs_both_phases_then_finishes() {
        let a = vector_field("a", 0, VectorEncoding::FLOAT32);
        let plain = FieldInfo::new("plain", 1);
        let c = vector_field("c", 2, VectorEncoding::FLOAT32);
        let infos = FieldInfos::new(vec![a, plain, c]).unwrap();
        let integrity_checks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let reader = |a: f32, c: f32| {
            let mut reader = ListVectorsReader {
                integrity_checks: Arc::clone(&integrity_checks),
                ..ListVectorsReader::default()
            };
            reader.floats.insert("a".to_string(), vec![vec![a]]);
            reader.floats.insert("c".to_string(), vec![vec![c]]);
            Some(Arc::new(reader) as Arc<dyn KnnVectorsReader>)
        };
        let doc_maps: Vec<DocMap> = vec![Arc::new(|doc: i32| doc), Arc::new(|doc: i32| 1 + doc)];
        let mut state = vectors_merge_state(
            vec![reader(1.0, 3.0), reader(2.0, 4.0)],
            doc_maps,
            vec![infos.clone(), infos.clone()],
            infos,
            false,
        );
        state.segment_info = Some(Arc::new(crate::codecs::tests::test_segment_info("_m", 2)));
        let stream = Arc::new(VvInfoStream(std::sync::Mutex::new(Vec::new())));
        state.info_stream = stream.clone();

        let events: Events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut writer = RecordingVectorsWriter {
            events: Arc::clone(&events),
            defer: true,
        };
        writer.merge(&state).unwrap();

        assert_eq!(
            integrity_checks.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "every source reader is checked first"
        );
        assert_eq!(
            *events.lock().unwrap(),
            vec![
                "add_field a",
                "add a doc=0 [1.0]",
                "add a doc=1 [2.0]",
                "add_field c",
                "add c doc=0 [3.0]",
                "add c doc=1 [4.0]",
                "deferred a",
                "deferred c",
                "finish",
            ]
        );
        let segment = "_m(10.5.0):C2";
        assert_eq!(
            *stream.0.lock().unwrap(),
            vec![
                format!("merging {segment}"),
                format!("merge done {segment}"),
                format!("merging {segment}"),
                format!("merge done {segment}"),
                format!("merging deferred work{segment}"),
                format!("merge deferred work done {segment}"),
                format!("merging deferred work{segment}"),
                format!("merge deferred work done {segment}"),
            ]
        );
    }
}
