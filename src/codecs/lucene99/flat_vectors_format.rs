//! Lucene 9.9 flat-vector vectors format.
//!
//! Equivalent to `org.apache.lucene.codecs.lucene99.Lucene99FlatVectorsFormat`,
//! `Lucene99FlatVectorsReader`, and `Lucene99FlatVectorsWriter`.
//!
//! This format stores vector values in a `.vec` file and per-field metadata in
//! a `.vemf` file. It supports both dense and sparse document-to-vector
//! mappings, byte-aligned for `BYTE` vectors and 64-byte-aligned for
//! `FLOAT32` vectors, matching the byte layout produced by Apache Lucene Core
//! 10.5.0.

#![deny(unsafe_code)]

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use crate::codecs::codec_util::{
    check_footer_after, check_index_header, checksum_entire_file, retrieve_checksum, write_footer,
    write_index_header,
};
use crate::codecs::hnsw::flat_vectors::{
    DefaultFlatFieldVectorsWriter, DefaultFlatVectorScorer, DocsWithFieldSet,
    FlatFieldVectorWriter, FlatFieldVectorsWriter, FlatVectorsFormat, FlatVectorsReader,
    FlatVectorsScorer, FlatVectorsWriter, SharedFlatFieldVectorsWriter,
};
use crate::codecs::knn_vectors::{
    FieldVectorWriter, KnnVectorsFormat, KnnVectorsReader, KnnVectorsWriter, MergedVectorValues,
    SorterDocMap,
};
use crate::codecs::lucene90::indexed_disi::IndexedDISI;
use crate::codecs::lucene95::OrdToDocDISIReaderConfiguration;
use crate::codecs::postings::MergeState;
use crate::codecs::state::{OwnedSegmentWriteState, SegmentReadState, SegmentWriteState};
use crate::codecs::stub::FieldInfo;
use crate::error::{LuceneError, Result};
use crate::index::accept_ords_with;
use crate::index::vector_values::{
    ByteVectorValues, DenseDocIndexIterator, DocIndexIterator, FloatVectorValues, KnnVectorValues,
};
use crate::index::{segment_file_name, FieldInfos, VectorEncoding, VectorSimilarityFunction};
use crate::search::vector_scorer::OffHeapVectorScorer;
use crate::search::{AcceptDocs, DocIdSetIterator, VectorScorer, NO_MORE_DOCS};
use crate::store::{DataAccessHint, DataInput, FileOpenHint, IndexInput, IndexOutput};
use crate::util::extra::LongValues;
use crate::util::hnsw::scorer::BorrowedBits;
use crate::util::hnsw::RandomVectorScorer;
use crate::util::packed::SharedDirectMonotonicReader;
use crate::util::{Bits, IOUtils};

// -----------------------------------------------------------------------------
// Format constants
// -----------------------------------------------------------------------------

const NAME: &str = "Lucene99FlatVectorsFormat";
const META_CODEC_NAME: &str = "Lucene99FlatVectorsFormatMeta";
const VECTOR_DATA_CODEC_NAME: &str = "Lucene99FlatVectorsFormatData";
const META_EXTENSION: &str = "vemf";
const VECTOR_DATA_EXTENSION: &str = "vec";
const VERSION_START: i32 = 0;
const VERSION_CURRENT: i32 = VERSION_START;
const DIRECT_MONOTONIC_BLOCK_SHIFT: i32 = 16;

// -----------------------------------------------------------------------------
// Global format registration
// -----------------------------------------------------------------------------

static LUCENE99_FLAT_VECTORS_FORMAT_REGISTERED: OnceLock<()> = OnceLock::new();

fn ensure_registered() {
    LUCENE99_FLAT_VECTORS_FORMAT_REGISTERED.get_or_init(|| {
        let _ = crate::codecs::knn_vectors::register_global_knn_vectors_format(
            NAME,
            Lucene99FlatVectorsFormat::new(DefaultFlatVectorScorer::INSTANCE),
        );
    });
}

// -----------------------------------------------------------------------------
// Lucene99FlatVectorsFormat
// -----------------------------------------------------------------------------

/// Lucene 9.9 flat vector format.
///
/// Equivalent to `org.apache.lucene.codecs.lucene99.Lucene99FlatVectorsFormat`.
#[derive(Debug, Clone)]
pub struct Lucene99FlatVectorsFormat {
    vectors_scorer: Arc<dyn FlatVectorsScorer>,
}

impl Default for Lucene99FlatVectorsFormat {
    fn default() -> Self {
        Self::new(DefaultFlatVectorScorer::INSTANCE)
    }
}

impl Lucene99FlatVectorsFormat {
    /// Creates a new format instance whose writers and readers score with
    /// `vectors_scorer`.
    ///
    /// Equivalent to `Lucene99FlatVectorsFormat(FlatVectorsScorer)`
    /// (`Lucene99FlatVectorsFormat.java:89-92`): any scorer, the default one
    /// or another, such as `lucene-codecs`' `FlatBitVectorsScorer`.
    pub fn new(vectors_scorer: impl FlatVectorsScorer + 'static) -> Self {
        Self::with_shared_scorer(Arc::new(vectors_scorer))
    }

    /// [`new`](Self::new) over a scorer the caller already shares.
    pub fn with_shared_scorer(vectors_scorer: Arc<dyn FlatVectorsScorer>) -> Self {
        Self { vectors_scorer }
    }

    fn create_writer<'a>(
        &self,
        state: &SegmentWriteState<'a>,
    ) -> Result<Lucene99FlatVectorsWriter> {
        ensure_registered();
        Lucene99FlatVectorsWriter::new(state, Arc::clone(&self.vectors_scorer))
    }

    fn create_reader<'a>(&self, state: &SegmentReadState<'a>) -> Result<Lucene99FlatVectorsReader> {
        ensure_registered();
        Lucene99FlatVectorsReader::new(state, Arc::clone(&self.vectors_scorer))
    }
}

impl KnnVectorsFormat for Lucene99FlatVectorsFormat {
    fn name(&self) -> &str {
        NAME
    }

    fn fields_writer(&self, state: &OwnedSegmentWriteState) -> Result<Box<dyn KnnVectorsWriter>> {
        Ok(Box::new(self.create_writer(&state.borrow())?))
    }

    fn fields_reader<'a>(&self, state: &SegmentReadState<'a>) -> Result<Box<dyn KnnVectorsReader>> {
        Ok(Box::new(self.create_reader(state)?))
    }

    fn get_max_dimensions(&self, _field_name: &str) -> i32 {
        1024
    }
}

impl FlatVectorsFormat for Lucene99FlatVectorsFormat {
    fn fields_writer_flat(
        &self,
        state: &OwnedSegmentWriteState,
    ) -> Result<Box<dyn FlatVectorsWriter>> {
        Ok(Box::new(self.create_writer(&state.borrow())?))
    }

    fn fields_reader_flat(
        &self,
        state: &SegmentReadState<'_>,
    ) -> Result<Box<dyn FlatVectorsReader>> {
        Ok(Box::new(self.create_reader(state)?))
    }
}

// -----------------------------------------------------------------------------
// Lucene99FlatVectorsWriter
// -----------------------------------------------------------------------------

/// Writer for the Lucene 9.9 flat-vector format.
///
/// Equivalent to `org.apache.lucene.codecs.lucene99.Lucene99FlatVectorsWriter`.
pub struct Lucene99FlatVectorsWriter {
    meta: Option<Box<dyn IndexOutput>>,
    vector_data: Option<Box<dyn IndexOutput>>,
    vector_scorer: Arc<dyn FlatVectorsScorer>,
    /// `fieldWriterFactory`: builds the per-field storage `addField` hands
    /// out and `flush` writes.
    field_writer_factory: FlatFieldWriterFactory,
    fields: Vec<FieldWriterEntry>,
    /// `maxDoc` of the segment being written, or `None` when it was not set
    /// yet at the moment this writer was created.
    ///
    /// Java holds the whole `SegmentWriteState` and reads
    /// `segmentWriteState.segmentInfo.maxDoc()` lazily, at the one place that
    /// needs it — `mergeOneField` (`Lucene99FlatVectorsWriter.java:288`).
    /// Reading it eagerly in the constructor, as this port used to, made the
    /// writer unconstructible on the flush path: `VectorValuesConsumer` builds
    /// it while the first document is being indexed
    /// (`VectorValuesConsumer.java:52-71`) and `DocumentsWriterPerThread` only
    /// calls `segmentInfo.setMaxDoc` at flush
    /// (`DocumentsWriterPerThread.java:446`), so `maxDoc()` raises
    /// `IllegalStateException("maxDoc isn't set yet")` at that point. The
    /// merge path always creates the writer after `maxDoc` is set, so it still
    /// sees the same value Java sees; the flush path never reads this field,
    /// because `flush(maxDoc, ..)` is given `maxDoc` by its caller.
    segment_max_doc: Option<i32>,
    finished: bool,
}

impl fmt::Debug for Lucene99FlatVectorsWriter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Lucene99FlatVectorsWriter")
            .field("segment_max_doc", &self.segment_max_doc)
            .field("finished", &self.finished)
            .field("fields", &self.fields.len())
            .finish_non_exhaustive()
    }
}

/// The per-field storage factory of a [`Lucene99FlatVectorsWriter`]: Java's
/// `IOFunction<FieldInfo, FlatFieldVectorsWriter<?>> strategyFactory`
/// (`Lucene99FlatVectorsWriter.java:62, 96-103`), whose answer's element type
/// follows the field's encoding.
pub type FlatFieldWriterFactory =
    Arc<dyn Fn(&FieldInfo) -> Result<FlatFieldVectorWriter> + Send + Sync>;

/// `DefaultFieldWriter::create` (`Lucene99FlatVectorsWriter.java:374-391`):
/// the factory of the writer built without one, which keeps every vector on
/// the heap.
fn default_field_writer(field_info: &FieldInfo) -> Result<FlatFieldVectorWriter> {
    Ok(match field_info.vector_encoding {
        VectorEncoding::FLOAT32 => {
            FlatFieldVectorWriter::Float(Arc::new(Mutex::new(DefaultFlatFieldVectorsWriter::<
                Vec<f32>,
            >::new(
                field_info.clone()
            ))))
        }
        VectorEncoding::BYTE => {
            FlatFieldVectorWriter::Byte(Arc::new(Mutex::new(DefaultFlatFieldVectorsWriter::<
                Vec<u8>,
            >::new(field_info.clone()))))
        }
    })
}

/// Java's `record FieldData(FlatFieldVectorsWriter<?> fieldWriter, FieldInfo
/// fieldInfo)`, split by element type.
enum FieldWriterEntry {
    Float(SharedFlatFieldVectorsWriter<Vec<f32>>, FieldInfo),
    Byte(SharedFlatFieldVectorsWriter<Vec<u8>>, FieldInfo),
}

impl fmt::Debug for FieldWriterEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FieldWriterEntry::Float(_, info) => f.debug_tuple("Float").field(&info.name).finish(),
            FieldWriterEntry::Byte(_, info) => f.debug_tuple("Byte").field(&info.name).finish(),
        }
    }
}

impl Lucene99FlatVectorsWriter {
    /// Constructs a writer that uses the default factory to build per-field
    /// vector storage, which stores vector data as a list of on-heap arrays,
    /// one per vector.
    ///
    /// Equivalent to `Lucene99FlatVectorsWriter(SegmentWriteState,
    /// FlatVectorsScorer)`.
    ///
    /// # Errors
    ///
    /// The errors of creating the two outputs and writing their headers.
    pub fn new(
        state: &SegmentWriteState<'_>,
        vectors_scorer: Arc<dyn FlatVectorsScorer>,
    ) -> Result<Self> {
        Self::with_field_writer_factory(state, vectors_scorer, Arc::new(default_field_writer))
    }

    /// Constructs a writer that uses `strategy_factory` to build per-field
    /// vector storage. The factory is consulted on every `add_field` call;
    /// merges write directly to the new segment and do not go through it.
    ///
    /// Equivalent to `Lucene99FlatVectorsWriter(SegmentWriteState,
    /// FlatVectorsScorer, IOFunction<FieldInfo, FlatFieldVectorsWriter<?>>)`
    /// (`Lucene99FlatVectorsWriter.java:96-136`).
    ///
    /// # Errors
    ///
    /// The errors of creating the two outputs and writing their headers.
    pub fn with_field_writer_factory(
        state: &SegmentWriteState<'_>,
        vectors_scorer: Arc<dyn FlatVectorsScorer>,
        strategy_factory: FlatFieldWriterFactory,
    ) -> Result<Self> {
        let meta_file_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            META_EXTENSION,
        );
        let vector_data_file_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            VECTOR_DATA_EXTENSION,
        );

        // Both outputs are created inside the `try` block; when it fails,
        // `IOUtils.closeWhileHandlingException(this)` closes what was created
        // through `close()`, `IOUtils.close(meta, vectorData)`
        // (`Lucene99FlatVectorsWriter.java:113-136, 353-356`).
        let mut meta_opt: Option<Box<dyn IndexOutput>> = None;
        let mut vector_data_opt: Option<Box<dyn IndexOutput>> = None;
        let result = (|| -> Result<()> {
            let meta = meta_opt.insert(
                state
                    .directory
                    .create_output(&meta_file_name, state.context)?,
            );
            let vector_data = vector_data_opt.insert(
                state
                    .directory
                    .create_output(&vector_data_file_name, state.context)?,
            );
            write_index_header(
                meta.as_mut(),
                META_CODEC_NAME,
                VERSION_CURRENT,
                &state.segment_info.id(),
                &state.segment_suffix,
            )?;
            write_index_header(
                vector_data.as_mut(),
                VECTOR_DATA_CODEC_NAME,
                VERSION_CURRENT,
                &state.segment_info.id(),
                &state.segment_suffix,
            )?;
            Ok(())
        })();

        if let Err(error) = result {
            IOUtils::close_while_handling_exception([Some(|| {
                close_outputs(&mut meta_opt, &mut vector_data_opt)
            })])?;
            return Err(error);
        }
        Ok(Self {
            meta: meta_opt,
            vector_data: vector_data_opt,
            vector_scorer: vectors_scorer,
            field_writer_factory: strategy_factory,
            fields: Vec::new(),
            segment_max_doc: state.segment_info.max_doc().ok(),
            finished: false,
        })
    }

    fn write_field_float(
        &mut self,
        field_writer: &mut dyn FlatFieldVectorsWriter<Vec<f32>>,
        field_info: &FieldInfo,
        max_doc: i32,
    ) -> Result<()> {
        let vector_data = self.vector_data.as_mut().ok_or_else(|| {
            LuceneError::illegal_state("Lucene99FlatVectorsWriter is already closed".to_string())
        })?;
        let vector_data_offset = align_output(vector_data.as_mut(), VectorEncoding::FLOAT32)?;
        Self::write_float32_vectors(vector_data.as_mut(), field_writer, field_info)?;
        let vector_data_length = vector_data.file_pointer() - vector_data_offset;
        let docs_with_field = field_writer.docs_with_field_set().into_owned();

        let meta = self.meta.as_mut().ok_or_else(|| {
            LuceneError::illegal_state("Lucene99FlatVectorsWriter is already closed".to_string())
        })?;
        write_meta(
            meta.as_mut(),
            vector_data.as_mut(),
            field_info,
            max_doc,
            vector_data_offset,
            vector_data_length,
            &docs_with_field,
        )
    }

    /// Writes a float field of a sorted segment, in the new ordinal order.
    ///
    /// Equivalent to `Lucene99FlatVectorsWriter.writeSortingField(FlatFieldVectorsWriter, FieldInfo, int, Sorter.DocMap)`
    /// for the `FLOAT32` encoding. The vectors are written in the order the
    /// sort puts their documents in, so a reader can address ordinal *n* by
    /// multiplying, exactly as for an unsorted segment.
    fn write_sorting_field_float(
        &mut self,
        field_writer: &mut dyn FlatFieldVectorsWriter<Vec<f32>>,
        field_info: &FieldInfo,
        max_doc: i32,
        sort_map: &SorterDocMap,
    ) -> Result<()> {
        let old_doc_ids = docs_with_field_to_docs(&field_writer.docs_with_field_set())?;
        let (_old_to_new, ord_map) =
            crate::codecs::knn_vectors::map_old_ord_to_new_ord(&old_doc_ids, sort_map)?;
        let new_docs_with_field = new_docs_with_field(&old_doc_ids, sort_map)?;

        let vector_data = self.vector_data.as_mut().ok_or_else(|| {
            LuceneError::illegal_state("Lucene99FlatVectorsWriter is already closed".to_string())
        })?;
        let vector_data_offset = align_output(vector_data.as_mut(), VectorEncoding::FLOAT32)?;
        Self::write_sorted_float32_vectors(vector_data.as_mut(), field_writer, &ord_map)?;
        let vector_data_length = vector_data.file_pointer() - vector_data_offset;

        let meta = self.meta.as_mut().ok_or_else(|| {
            LuceneError::illegal_state("Lucene99FlatVectorsWriter is already closed".to_string())
        })?;
        write_meta(
            meta.as_mut(),
            vector_data.as_mut(),
            field_info,
            max_doc,
            vector_data_offset,
            vector_data_length,
            &new_docs_with_field,
        )
    }

    /// Writes a byte field of a sorted segment, in the new ordinal order.
    ///
    /// Equivalent to `Lucene99FlatVectorsWriter.writeSortingField(...)` for the
    /// `BYTE` encoding.
    fn write_sorting_field_byte(
        &mut self,
        field_writer: &mut dyn FlatFieldVectorsWriter<Vec<u8>>,
        field_info: &FieldInfo,
        max_doc: i32,
        sort_map: &SorterDocMap,
    ) -> Result<()> {
        let old_doc_ids = docs_with_field_to_docs(&field_writer.docs_with_field_set())?;
        let (_old_to_new, ord_map) =
            crate::codecs::knn_vectors::map_old_ord_to_new_ord(&old_doc_ids, sort_map)?;
        let new_docs_with_field = new_docs_with_field(&old_doc_ids, sort_map)?;

        let vector_data = self.vector_data.as_mut().ok_or_else(|| {
            LuceneError::illegal_state("Lucene99FlatVectorsWriter is already closed".to_string())
        })?;
        let vector_data_offset = align_output(vector_data.as_mut(), VectorEncoding::BYTE)?;
        Self::write_sorted_byte_vectors(vector_data.as_mut(), field_writer, &ord_map)?;
        let vector_data_length = vector_data.file_pointer() - vector_data_offset;

        let meta = self.meta.as_mut().ok_or_else(|| {
            LuceneError::illegal_state("Lucene99FlatVectorsWriter is already closed".to_string())
        })?;
        write_meta(
            meta.as_mut(),
            vector_data.as_mut(),
            field_info,
            max_doc,
            vector_data_offset,
            vector_data_length,
            &new_docs_with_field,
        )
    }

    /// Writes every buffered float vector to the vector-data file, in the
    /// order they were added: each vector goes through a little-endian
    /// buffer of `field_info.getVectorDimension() * Float.BYTES` bytes and
    /// one `writeBytes` call (`Lucene99FlatVectorsWriter.java:205-215`).
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene99.Lucene99FlatVectorsWriter#writeFloat32Vectors`.
    fn write_float32_vectors(
        vector_data: &mut dyn IndexOutput,
        field_writer: &dyn FlatFieldVectorsWriter<Vec<f32>>,
        field_info: &FieldInfo,
    ) -> Result<()> {
        let mut buffer = Vec::with_capacity(field_info.vector_dimension as usize * 4);
        for vector in field_writer.vectors().iter() {
            write_float_vector(vector_data, &mut buffer, vector)?;
        }
        Ok(())
    }

    /// Writes every buffered byte vector to the vector-data file, in the order
    /// they were added.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene99.Lucene99FlatVectorsWriter#writeByteVectors`.
    fn write_byte_vectors(
        vector_data: &mut dyn IndexOutput,
        field_writer: &dyn FlatFieldVectorsWriter<Vec<u8>>,
    ) -> Result<()> {
        for vector in field_writer.vectors().iter() {
            vector_data.write_bytes(vector, 0, vector.len())?;
        }
        Ok(())
    }

    /// Writes `byte_vector_values` to `output` and returns the documents that
    /// carried a vector.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene99.Lucene99FlatVectorsWriter#writeByteVectorData`
    /// (`Lucene99FlatVectorsWriter.java:319-331`), which
    /// [`FlatVectorsWriter::merge_one_flat_vector_field`] calls with the merged
    /// view [`MergedVectorValues::merge_byte_vector_values`] builds.
    fn write_byte_vector_data(
        output: &mut dyn IndexOutput,
        byte_vector_values: &dyn ByteVectorValues,
    ) -> Result<DocsWithFieldSet> {
        let mut docs_with_field = DocsWithFieldSet::new();
        let mut iter = byte_vector_values.iterator()?;
        loop {
            let doc_v = iter.next_doc()?;
            if doc_v == NO_MORE_DOCS {
                break;
            }
            let binary_value = byte_vector_values.vector_value(iter.index())?;
            output.write_bytes(&binary_value, 0, binary_value.len())?;
            docs_with_field.add(doc_v)?;
        }
        Ok(docs_with_field)
    }

    /// Writes `float_vector_values` to `output` and returns the documents that
    /// carried a vector.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene99.Lucene99FlatVectorsWriter#writeVectorData`
    /// (`Lucene99FlatVectorsWriter.java:336-351`), which
    /// [`FlatVectorsWriter::merge_one_flat_vector_field`] calls with the merged
    /// view [`MergedVectorValues::merge_float_vector_values`] builds. Each
    /// vector goes through a little-endian buffer sized from `dimension()`
    /// and one `writeBytes` call, as in Java.
    ///
    /// # Errors
    ///
    /// Returns the error of [`KnnVectorValues::try_dimension`] for a merged
    /// view without subs, where Java's `dimension()` call that sizes the buffer
    /// (`Lucene99FlatVectorsWriter.java:339-341`) throws, and whatever reading
    /// the values or writing the output raises.
    fn write_vector_data(
        output: &mut dyn IndexOutput,
        float_vector_values: &dyn FloatVectorValues,
    ) -> Result<DocsWithFieldSet> {
        let dimension = float_vector_values.try_dimension()?;
        let mut buffer = Vec::with_capacity(dimension as usize * 4);
        let mut docs_with_field = DocsWithFieldSet::new();
        let mut iter = float_vector_values.iterator()?;
        loop {
            let doc_v = iter.next_doc()?;
            if doc_v == NO_MORE_DOCS {
                break;
            }
            let value = float_vector_values.vector_value(iter.index())?;
            write_float_vector(output, &mut buffer, &value)?;
            docs_with_field.add(doc_v)?;
        }
        Ok(docs_with_field)
    }

    /// Writes the float vectors in the order `ord_map` gives.
    ///
    /// Equivalent to `Lucene99FlatVectorsWriter.writeSortedFloat32Vectors(FlatFieldVectorsWriter, FieldInfo, int[])`.
    fn write_sorted_float32_vectors(
        vector_data: &mut dyn IndexOutput,
        field_writer: &dyn FlatFieldVectorsWriter<Vec<f32>>,
        ord_map: &[i32],
    ) -> Result<()> {
        // `fieldWriter.getVectors().get(ordinal)`, the list read once.
        let vectors = field_writer.vectors();
        let mut buffer = Vec::new();
        for &ordinal in ord_map {
            let vector = &vectors[ordinal as usize];
            write_float_vector(vector_data, &mut buffer, vector)?;
        }
        Ok(())
    }

    /// Writes the byte vectors in the order `ord_map` gives.
    ///
    /// Equivalent to `Lucene99FlatVectorsWriter.writeSortedByteVectors(FlatFieldVectorsWriter, int[])`.
    fn write_sorted_byte_vectors(
        vector_data: &mut dyn IndexOutput,
        field_writer: &dyn FlatFieldVectorsWriter<Vec<u8>>,
        ord_map: &[i32],
    ) -> Result<()> {
        // `fieldWriter.getVectors().get(ordinal)`, the list read once.
        let vectors = field_writer.vectors();
        for &ordinal in ord_map {
            let vector = &vectors[ordinal as usize];
            vector_data.write_bytes(vector, 0, vector.len())?;
        }
        Ok(())
    }

    fn write_field_byte(
        &mut self,
        field_writer: &mut dyn FlatFieldVectorsWriter<Vec<u8>>,
        field_info: &FieldInfo,
        max_doc: i32,
    ) -> Result<()> {
        let vector_data = self.vector_data.as_mut().ok_or_else(|| {
            LuceneError::illegal_state("Lucene99FlatVectorsWriter is already closed".to_string())
        })?;
        let vector_data_offset = align_output(vector_data.as_mut(), VectorEncoding::BYTE)?;
        Self::write_byte_vectors(vector_data.as_mut(), field_writer)?;
        let vector_data_length = vector_data.file_pointer() - vector_data_offset;
        let docs_with_field = field_writer.docs_with_field_set().into_owned();

        let meta = self.meta.as_mut().ok_or_else(|| {
            LuceneError::illegal_state("Lucene99FlatVectorsWriter is already closed".to_string())
        })?;
        write_meta(
            meta.as_mut(),
            vector_data.as_mut(),
            field_info,
            max_doc,
            vector_data_offset,
            vector_data_length,
            &docs_with_field,
        )
    }
}

impl KnnVectorsWriter for Lucene99FlatVectorsWriter {
    /// The writer [`FlatVectorsWriter::add_field`] returns, as Java's
    /// covariant override hands out the same object.
    fn add_field(&mut self, field_info: &FieldInfo) -> Result<FieldVectorWriter> {
        FlatVectorsWriter::add_field(self, field_info).map(FieldVectorWriter::from)
    }

    fn flush(&mut self, max_doc: i32, sort_map: Option<&SorterDocMap>) -> Result<()> {
        let entries = std::mem::take(&mut self.fields);
        for entry in entries {
            match entry {
                FieldWriterEntry::Float(writer, info) => {
                    let mut guard = writer.lock().map_err(|_| {
                        LuceneError::illegal_state(
                            "FlatFieldVectorsWriter mutex was poisoned".to_string(),
                        )
                    })?;
                    match sort_map {
                        Some(sort_map) => {
                            self.write_sorting_field_float(&mut *guard, &info, max_doc, sort_map)?
                        }
                        None => self.write_field_float(&mut *guard, &info, max_doc)?,
                    }
                    guard.finish()?;
                }
                FieldWriterEntry::Byte(writer, info) => {
                    let mut guard = writer.lock().map_err(|_| {
                        LuceneError::illegal_state(
                            "FlatFieldVectorsWriter mutex was poisoned".to_string(),
                        )
                    })?;
                    match sort_map {
                        Some(sort_map) => {
                            self.write_sorting_field_byte(&mut *guard, &info, max_doc, sort_map)?
                        }
                        None => self.write_field_byte(&mut *guard, &info, max_doc)?,
                    }
                    guard.finish()?;
                }
            }
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        if self.finished {
            return Err(LuceneError::illegal_state(
                "Lucene99FlatVectorsWriter is already finished".to_string(),
            ));
        }
        self.finished = true;
        if let Some(meta) = self.meta.as_mut() {
            meta.write_int(-1)?;
            write_footer(meta.as_mut())?;
        }
        if let Some(vector_data) = self.vector_data.as_mut() {
            write_footer(vector_data.as_mut())?;
        }
        Ok(())
    }

    fn merge_one_field<'m>(
        &mut self,
        field_info: &FieldInfo,
        merge_state: &'m MergeState,
    ) -> Result<Option<Box<dyn crate::codecs::knn_vectors::IORunnable + 'm>>> {
        self.merge_one_flat_vector_field(field_info, merge_state)?;
        Ok(None)
    }

    fn close(&mut self) -> Result<()> {
        close_outputs(&mut self.meta, &mut self.vector_data)
    }

    /// Equivalent to `Lucene99FlatVectorsWriter.ramBytesUsed()`
    /// (`Lucene99FlatVectorsWriter.java:174-181`): a shallow size plus the
    /// footprint of every field writer it holds.
    fn ram_bytes_used(&self) -> i64 {
        let mut total = crate::util::RamUsageEstimator::align_object_size(
            crate::util::RamUsageEstimator::NUM_BYTES_OBJECT_HEADER
                + 6 * crate::util::RamUsageEstimator::NUM_BYTES_OBJECT_REF,
        );
        for entry in &self.fields {
            total += match entry {
                FieldWriterEntry::Float(writer, _) => writer
                    .lock()
                    .map(|guard| guard.ram_bytes_used())
                    .unwrap_or(0),
                FieldWriterEntry::Byte(writer, _) => writer
                    .lock()
                    .map(|guard| guard.ram_bytes_used())
                    .unwrap_or(0),
            };
        }
        total
    }
}

impl FlatVectorsWriter for Lucene99FlatVectorsWriter {
    /// Creates the field's buffer, keeps it to write at
    /// [`flush`](KnnVectorsWriter::flush), and returns it.
    ///
    /// Equivalent to `Lucene99FlatVectorsWriter.addField(FieldInfo)`
    /// (`Lucene99FlatVectorsWriter.java:139-144`), which adds the new
    /// `FieldWriter` to its `fields` and returns it.
    fn add_field(&mut self, field_info: &FieldInfo) -> Result<FlatFieldVectorWriter> {
        if self.finished {
            return Err(LuceneError::illegal_state(
                "Lucene99FlatVectorsWriter is already finished".to_string(),
            ));
        }
        // `var newFieldWriter = fieldWriterFactory.apply(fieldInfo)`
        let new_field_writer = (self.field_writer_factory)(field_info)?;
        match &new_field_writer {
            FlatFieldVectorWriter::Float(writer) => self.fields.push(FieldWriterEntry::Float(
                Arc::clone(writer),
                field_info.clone(),
            )),
            FlatFieldVectorWriter::Byte(writer) => self.fields.push(FieldWriterEntry::Byte(
                Arc::clone(writer),
                field_info.clone(),
            )),
        }
        Ok(new_field_writer)
    }

    fn get_flat_vector_scorer(&self) -> &dyn FlatVectorsScorer {
        &*self.vector_scorer
    }

    /// Writes one field's merged vectors straight into the new segment: the
    /// flat format has no further index to build, so no deferred work is left.
    ///
    /// Equivalent to `Lucene99FlatVectorsWriter.mergeOneFlatVectorField(FieldInfo, MergeState)`
    /// (`Lucene99FlatVectorsWriter.java:265-292`): align the data file, write
    /// the vectors of the merged view
    /// [`MergedVectorValues`] builds — every surviving document, in merged
    /// order, under its merged document id — then the field's metadata.
    fn merge_one_flat_vector_field(
        &mut self,
        field_info: &FieldInfo,
        merge_state: &MergeState,
    ) -> Result<()> {
        let vector_data = self.vector_data.as_mut().ok_or_else(|| {
            LuceneError::illegal_state("Lucene99FlatVectorsWriter is already closed".to_string())
        })?;
        let encoding = field_info.vector_encoding;
        let vector_data_offset = align_output(vector_data.as_mut(), encoding)?;
        // No need to use temporary file as we don't have to re-open for reading
        let docs_with_field = match encoding {
            VectorEncoding::BYTE => Self::write_byte_vector_data(
                vector_data.as_mut(),
                MergedVectorValues::merge_byte_vector_values(field_info, merge_state)?.as_ref(),
            )?,
            VectorEncoding::FLOAT32 => Self::write_vector_data(
                vector_data.as_mut(),
                MergedVectorValues::merge_float_vector_values(field_info, merge_state)?.as_ref(),
            )?,
        };
        let vector_data_length = vector_data.file_pointer() - vector_data_offset;
        let meta = self.meta.as_mut().ok_or_else(|| {
            LuceneError::illegal_state("Lucene99FlatVectorsWriter is already closed".to_string())
        })?;
        // Java reads `segmentWriteState.segmentInfo.maxDoc()` here, which
        // raises `IllegalStateException("maxDoc isn't set yet")` when it was
        // never set; this reproduces that error at the same place.
        let segment_max_doc = self
            .segment_max_doc
            .ok_or_else(|| LuceneError::illegal_state("maxDoc isn't set yet".to_string()))?;
        write_meta(
            meta.as_mut(),
            vector_data.as_mut(),
            field_info,
            segment_max_doc,
            vector_data_offset,
            vector_data_length,
            &docs_with_field,
        )
    }
}

/// `Lucene99FlatVectorsWriter.close()` (`Lucene99FlatVectorsWriter.java:353-356`):
/// `IOUtils.close(meta, vectorData)`.
/// `buffer.asFloatBuffer().put(vector); out.writeBytes(buffer.array(),
/// buffer.array().length)` (`Lucene99FlatVectorsWriter.java:208-213`,
/// `247-253`, `339-347`): the vector's little-endian bytes in one
/// `writeBytes` call. A wrapping output (e.g. `MockIndexOutputWrapper`)
/// sees one write per vector, as in Java, not one per byte.
fn write_float_vector(
    out: &mut dyn IndexOutput,
    buffer: &mut Vec<u8>,
    vector: &[f32],
) -> Result<()> {
    buffer.clear();
    for v in vector {
        buffer.extend_from_slice(&v.to_le_bytes());
    }
    out.write_bytes(buffer, 0, buffer.len())
}

fn close_outputs(
    meta: &mut Option<Box<dyn IndexOutput>>,
    vector_data: &mut Option<Box<dyn IndexOutput>>,
) -> Result<()> {
    IOUtils::close([meta.take(), vector_data.take()].map(|o| o.map(|mut o| move || o.close())))
}

fn align_output(out: &mut dyn IndexOutput, encoding: VectorEncoding) -> Result<i64> {
    let alignment = match encoding {
        VectorEncoding::BYTE => 4,
        VectorEncoding::FLOAT32 => 64,
    };
    let pos = out.file_pointer();
    let rem = pos % alignment;
    if rem != 0 {
        let padding = alignment - rem;
        for _ in 0..padding {
            out.write_byte(0)?;
        }
    }
    Ok(pos + (alignment - rem) % alignment)
}

fn write_meta(
    meta: &mut dyn IndexOutput,
    vector_data: &mut dyn IndexOutput,
    field: &FieldInfo,
    max_doc: i32,
    vector_data_offset: i64,
    vector_data_length: i64,
    docs_with_field: &DocsWithFieldSet,
) -> Result<()> {
    meta.write_int(field.number)?;
    meta.write_int(vector_encoding_ordinal(field.vector_encoding))?;
    meta.write_int(vector_similarity_ordinal(field.vector_similarity_function))?;
    meta.write_v_long(vector_data_offset)?;
    meta.write_v_long(vector_data_length)?;
    meta.write_v_int(field.vector_dimension)?;

    let count = docs_with_field.cardinality();
    meta.write_int(count)?;
    OrdToDocDISIReaderConfiguration::write_stored_meta(
        DIRECT_MONOTONIC_BLOCK_SHIFT,
        meta,
        vector_data,
        count,
        max_doc,
        docs_with_field,
    )
}

fn vector_encoding_ordinal(encoding: VectorEncoding) -> i32 {
    match encoding {
        VectorEncoding::BYTE => 0,
        VectorEncoding::FLOAT32 => 1,
    }
}

fn vector_similarity_ordinal(similarity: VectorSimilarityFunction) -> i32 {
    match similarity {
        VectorSimilarityFunction::EUCLIDEAN => 0,
        VectorSimilarityFunction::DOT_PRODUCT => 1,
        VectorSimilarityFunction::COSINE => 2,
        VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT => 3,
    }
}

fn read_vector_encoding(ordinal: i32) -> Result<VectorEncoding> {
    match ordinal {
        0 => Ok(VectorEncoding::BYTE),
        1 => Ok(VectorEncoding::FLOAT32),
        _ => Err(LuceneError::CorruptIndex(format!(
            "invalid VectorEncoding ordinal: {ordinal}"
        ))),
    }
}

fn read_vector_similarity(ordinal: i32) -> Result<VectorSimilarityFunction> {
    match ordinal {
        0 => Ok(VectorSimilarityFunction::EUCLIDEAN),
        1 => Ok(VectorSimilarityFunction::DOT_PRODUCT),
        2 => Ok(VectorSimilarityFunction::COSINE),
        3 => Ok(VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT),
        _ => Err(LuceneError::CorruptIndex(format!(
            "invalid VectorSimilarityFunction ordinal: {ordinal}"
        ))),
    }
}

// -----------------------------------------------------------------------------
// Lucene99FlatVectorsReader
// -----------------------------------------------------------------------------

/// Reader for the Lucene 9.9 flat-vector format.
///
/// Equivalent to `org.apache.lucene.codecs.lucene99.Lucene99FlatVectorsReader`.
pub struct Lucene99FlatVectorsReader {
    fields: HashMap<i32, FieldEntry>,
    vector_scorer: Arc<dyn FlatVectorsScorer>,
    vector_data: Box<dyn IndexInput>,
    field_infos: FieldInfos,
    data_context: Box<dyn crate::store::IOContext>,
}

impl fmt::Debug for Lucene99FlatVectorsReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Lucene99FlatVectorsReader")
            .field("fields", &self.fields.len())
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
#[allow(dead_code)]
struct FieldEntry {
    field_info: FieldInfo,
    similarity_function: VectorSimilarityFunction,
    vector_encoding: VectorEncoding,
    vector_data_offset: i64,
    vector_data_length: i64,
    dimension: i32,
    size: i32,
    ord_to_doc: OrdToDocDISIReaderConfiguration,
}

impl Lucene99FlatVectorsReader {
    /// Creates a new reader for the given segment read state, reading the
    /// vectors randomly.
    ///
    /// Equivalent to `Lucene99FlatVectorsReader(SegmentReadState,
    /// FlatVectorsScorer)` (`Lucene99FlatVectorsReader.java:70-73`), which
    /// passes `DataAccessHint.RANDOM`.
    pub fn new(
        state: &SegmentReadState<'_>,
        vectors_scorer: Arc<dyn FlatVectorsScorer>,
    ) -> Result<Self> {
        Self::with_access_hint(state, vectors_scorer, Some(DataAccessHint::Random))
    }

    /// Creates a new reader whose data file is opened with
    /// `FileTypeHint.DATA`, `FileDataHint.KNN_VECTORS` and `access_hint`, when
    /// one is given.
    ///
    /// Equivalent to `Lucene99FlatVectorsReader(SegmentReadState,
    /// FlatVectorsScorer, DataAccessHint)` (`Lucene99FlatVectorsReader.java:82-93`).
    ///
    /// # Errors
    ///
    /// Returns whatever reading the metadata or opening the data file raises.
    pub fn with_access_hint(
        state: &SegmentReadState<'_>,
        vectors_scorer: Arc<dyn FlatVectorsScorer>,
        access_hint: Option<DataAccessHint>,
    ) -> Result<Self> {
        let (version_meta, fields) = Self::read_metadata(state)?;
        let mut hints: Vec<Arc<dyn FileOpenHint>> = vec![
            Arc::new(crate::store::FileTypeHint::Data),
            Arc::new(crate::store::FileDataHint::KnnVectors),
        ];
        if let Some(access_hint) = access_hint {
            hints.push(Arc::new(access_hint));
        }
        let data_context = state.context.with_hints(&hints);
        let vector_data = Self::open_data_input(
            state,
            version_meta,
            VECTOR_DATA_EXTENSION,
            VECTOR_DATA_CODEC_NAME,
            data_context.as_ref(),
        )?;

        Ok(Self {
            fields,
            vector_scorer: vectors_scorer,
            vector_data,
            field_infos: state.field_infos.clone(),
            data_context,
        })
    }

    /// Reads the `.vemf` metadata file and returns its format version together
    /// with the field entries it declares.
    ///
    /// **Divergence from Lucene 10.5.0.** Java's method returns only the
    /// version and stores the entries straight into the reader's `fields` map
    /// (`Lucene99FlatVectorsReader.java:110-134`), because it runs from a
    /// constructor that already has `this`. A Rust constructor has no `self`
    /// before it returns, so the entries come back with the version.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene99.Lucene99FlatVectorsReader#readMetadata`.
    fn read_metadata(state: &SegmentReadState<'_>) -> Result<(i32, HashMap<i32, FieldEntry>)> {
        let meta_file_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            META_EXTENSION,
        );
        let mut meta_checksum = state.directory.open_checksum_input(&meta_file_name)?;
        // The `try` block of the try-with-resources over `meta`, whose failure
        // `CodecUtil.checkFooter(meta, priorE)` weighs in the `finally`
        // (`Lucene99FlatVectorsReader.java:115-131`).
        let read = (|| -> Result<_> {
            let version_meta = check_index_header(
                meta_checksum.as_mut(),
                META_CODEC_NAME,
                VERSION_START,
                VERSION_CURRENT,
                &state.segment_info.id(),
                &state.segment_suffix,
            )?;
            let fields = read_fields(meta_checksum.as_mut(), state.field_infos)?;
            Ok((version_meta, fields))
        })();
        let read = check_footer_after(meta_checksum.as_mut(), read);
        let (version_meta, fields) = crate::store::close_after(meta_checksum.as_mut(), read)?;
        Ok((version_meta, fields))
    }

    /// Opens one of the format's data files, checks its header against
    /// `version_meta` and primes its checksum.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene99.Lucene99FlatVectorsReader#openDataInput`.
    fn open_data_input(
        state: &SegmentReadState<'_>,
        version_meta: i32,
        file_extension: &str,
        codec_name: &str,
        context: &dyn crate::store::IOContext,
    ) -> Result<Box<dyn IndexInput>> {
        let file_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            file_extension,
        );
        let mut input = state.directory.open_input(&file_name, context)?;
        let checked = (|| -> Result<()> {
            let version_vector_data = check_index_header(
                input.as_mut(),
                codec_name,
                VERSION_START,
                VERSION_CURRENT,
                &state.segment_info.id(),
                &state.segment_suffix,
            )?;
            if version_meta != version_vector_data {
                return Err(LuceneError::CorruptIndex(format!(
                    "format versions mismatch: meta={version_meta}, {codec_name}={version_vector_data}"
                )));
            }
            retrieve_checksum(input.as_mut())?;
            Ok(())
        })();
        match checked {
            Ok(()) => Ok(input),
            Err(error) => {
                // `IOUtils.closeWhileHandlingException(in)`
                // (`Lucene99FlatVectorsReader.java:169-173`).
                IOUtils::close_while_handling_exception([Some(|| input.close())])?;
                Err(error)
            }
        }
    }

    /// `vectorData.updateIOContext(dataContext.withHints(DataAccessHint.SEQUENTIAL))`
    /// (`Lucene99FlatVectorsReader.java:206`).
    fn update_io_context_for_merge(&self) -> Result<()> {
        let sequential: Arc<dyn FileOpenHint> = Arc::new(DataAccessHint::Sequential);
        self.vector_data
            .update_io_context(self.data_context.with_hints(&[sequential]).as_ref())
    }

    fn clone_reader(&self) -> Result<Self> {
        Ok(Self {
            fields: self.fields.clone(),
            vector_scorer: Arc::clone(&self.vector_scorer),
            vector_data: self.vector_data.clone_input()?,
            field_infos: self.field_infos.clone(),
            data_context: self.data_context.with_hints(self.data_context.hints()),
        })
    }

    fn get_field_entry_or_throw(&self, field: &str) -> Result<&FieldEntry> {
        let info = self
            .field_infos
            .field_info(field)
            .ok_or_else(|| LuceneError::IllegalArgument(format!("field=\"{field}\" not found")))?;
        self.fields.get(&info.number).ok_or_else(|| {
            LuceneError::IllegalArgument(format!("field=\"{field}\" has no vector values"))
        })
    }

    fn get_field_entry(
        &self,
        field: &str,
        expected_encoding: VectorEncoding,
    ) -> Result<&FieldEntry> {
        let entry = self.get_field_entry_or_throw(field)?;
        if entry.vector_encoding != expected_encoding {
            return Err(LuceneError::IllegalArgument(format!(
                "field=\"{field}\" is encoded as {entry_vector_encoding:?}, expected {expected_encoding:?}",
                entry_vector_encoding = entry.vector_encoding,
            )));
        }
        Ok(entry)
    }
}

impl KnnVectorsReader for Lucene99FlatVectorsReader {
    fn check_integrity(&self) -> Result<()> {
        let mut data = self.vector_data.clone_input()?;
        checksum_entire_file(data.as_mut())?;
        Ok(())
    }

    fn get_float_vector_values(&self, field: &str) -> Result<Option<Box<dyn FloatVectorValues>>> {
        let entry = self.get_field_entry(field, VectorEncoding::FLOAT32)?;
        OffHeapFloatVectorValues::load(
            entry.similarity_function,
            Arc::clone(&self.vector_scorer),
            &entry.ord_to_doc,
            entry.dimension,
            entry.vector_data_offset,
            entry.vector_data_length,
            self.vector_data.as_ref(),
        )
        .map(Some)
    }

    fn get_byte_vector_values(&self, field: &str) -> Result<Option<Box<dyn ByteVectorValues>>> {
        let entry = self.get_field_entry(field, VectorEncoding::BYTE)?;
        OffHeapByteVectorValues::load(
            entry.similarity_function,
            Arc::clone(&self.vector_scorer),
            &entry.ord_to_doc,
            entry.dimension,
            entry.vector_data_offset,
            entry.vector_data_length,
            self.vector_data.as_ref(),
        )
        .map(Some)
    }

    /// Collects nothing, whatever the query and whatever the accept docs.
    ///
    /// Equivalent to `FlatVectorsReader.search(String, float[], KnnCollector,
    /// AcceptDocs)`, the empty method every flat reader inherits: "don't scan
    /// stored field data. If we didn't index it, produce no search results".
    /// `accept_docs` is unused here for the same reason it is unused in Java —
    /// nothing is collected, so there is nothing to filter — and not because
    /// the parameter is being ignored.
    fn search(
        &self,
        _field: &str,
        _target: &[f32],
        _knn_collector: &mut dyn crate::codecs::knn_vectors::KnnCollector,
        _accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        Ok(())
    }

    /// Collects nothing, as [`search`](Self::search) does.
    ///
    /// Equivalent to `FlatVectorsReader.search(String, byte[], KnnCollector,
    /// AcceptDocs)`.
    fn search_byte(
        &self,
        _field: &str,
        _target: &[u8],
        _knn_collector: &mut dyn crate::codecs::knn_vectors::KnnCollector,
        _accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        Ok(())
    }

    /// Equivalent to `Lucene99FlatVectorsReader.getMergeInstance()`
    /// (`Lucene99FlatVectorsReader.java:203-208`): the vectors are read
    /// sequentially during a merge, so the data input is re-advised with
    /// `DataAccessHint.SEQUENTIAL` first.
    ///
    /// Java returns `this`; this port returns a clone over the same mapped
    /// data, which the advice reaches because it applies to the mapping.
    fn get_merge_instance(&self) -> Result<Box<dyn KnnVectorsReader>> {
        self.update_io_context_for_merge()?;
        Ok(Box::new(self.clone_reader()?))
    }

    /// Equivalent to `Lucene99FlatVectorsReader.finishMerge()`
    /// (`Lucene99FlatVectorsReader.java:300-305`): reverts the data input to
    /// the access pattern of the reader's own context, which the HNSW search
    /// needs.
    fn finish_merge(&mut self) -> Result<()> {
        self.vector_data
            .update_io_context(self.data_context.as_ref())
    }

    /// Equivalent to `Lucene99FlatVectorsReader.close()`
    /// (`Lucene99FlatVectorsReader.java:307-310`): `IOUtils.close(vectorData)`.
    fn close(&mut self) -> Result<()> {
        IOUtils::close([Some(|| self.vector_data.close())])
    }

    fn get_off_heap_byte_size(&self, field_info: &FieldInfo) -> HashMap<String, i64> {
        let mut map = HashMap::new();
        if let Ok(entry) = self.get_field_entry_or_throw(&field_info.name) {
            map.insert(VECTOR_DATA_EXTENSION.to_string(), entry.vector_data_length);
        }
        map
    }

    /// This reader is a [`FlatVectorsReader`], so it answers with itself.
    ///
    /// This is the `true` arm of the `instanceof FlatVectorsReader` test Java
    /// writes; see [`KnnVectorsReader::as_flat_vectors_reader`].
    fn as_flat_vectors_reader(&self) -> Option<&dyn FlatVectorsReader> {
        Some(self)
    }
}

/// The shallow size Lucene charges for this reader.
///
/// Lucene computes it from `Lucene99FlatVectorsFormat.class`, not from the
/// reader's own class (`Lucene99FlatVectorsReader.java:61-62`) — an oddity of
/// the original that is nonetheless the contract. Measured against
/// `lucene-core-10.5.0.jar`: `shallowSizeOfInstance(Lucene99FlatVectorsFormat.class)`
/// is 24.
const FLAT_VECTORS_READER_SHALLOW_SIZE: i64 = 24;

/// The shallow size of the reader's `FieldEntry` on the JVM, measured against
/// `lucene-core-10.5.0.jar`: 56.
const FLAT_VECTORS_FIELD_ENTRY_SHALLOW_SIZE: i64 = 56;

/// Accounts for the field map this reader holds.
///
/// Equivalent to `Lucene99FlatVectorsReader.ramBytesUsed()`, which is
/// `SHALLOW_SIZE + fields.ramBytesUsed()`.
///
/// **Divergence from Lucene 10.5.0.** Java's `fields` is an
/// `IntObjectHashMap`, whose `ramBytesUsed()` reports its *table* capacity and
/// so steps in powers of two; this port charges the crate's
/// [`RamUsageEstimator::size_of_map`] model instead, which is linear in the
/// entry count. Both are estimates of the same object; neither is read back
/// from a file, so no index bytes depend on the difference.
impl crate::util::Accountable for Lucene99FlatVectorsReader {
    fn ram_bytes_used(&self) -> i64 {
        FLAT_VECTORS_READER_SHALLOW_SIZE
            + crate::util::RamUsageEstimator::size_of_map(
                self.fields.len(),
                FLAT_VECTORS_FIELD_ENTRY_SHALLOW_SIZE,
            )
    }
}

impl FlatVectorsReader for Lucene99FlatVectorsReader {
    fn get_flat_vector_scorer(&self, _field: &str) -> Result<Box<dyn FlatVectorsScorer>> {
        Ok(Box::new(Arc::clone(&self.vector_scorer)))
    }

    fn get_random_vector_scorer_float(
        &self,
        field: &str,
        target: &[f32],
    ) -> Result<Box<dyn RandomVectorScorer>> {
        let entry = self.get_field_entry(field, VectorEncoding::FLOAT32)?;
        let values = OffHeapFloatVectorValues::load(
            entry.similarity_function,
            Arc::clone(&self.vector_scorer),
            &entry.ord_to_doc,
            entry.dimension,
            entry.vector_data_offset,
            entry.vector_data_length,
            self.vector_data.as_ref(),
        )?;
        self.vector_scorer
            .get_random_vector_scorer_float(entry.similarity_function, values, target)
    }

    fn get_random_vector_scorer_byte(
        &self,
        field: &str,
        target: &[u8],
    ) -> Result<Box<dyn RandomVectorScorer>> {
        let entry = self.get_field_entry(field, VectorEncoding::BYTE)?;
        let values = OffHeapByteVectorValues::load(
            entry.similarity_function,
            Arc::clone(&self.vector_scorer),
            &entry.ord_to_doc,
            entry.dimension,
            entry.vector_data_offset,
            entry.vector_data_length,
            self.vector_data.as_ref(),
        )?;
        self.vector_scorer
            .get_random_vector_scorer_byte(entry.similarity_function, values, target)
    }

    /// See [`KnnVectorsReader::get_merge_instance`] for this reader.
    fn get_merge_instance_flat(&self) -> Result<Box<dyn FlatVectorsReader>> {
        self.update_io_context_for_merge()?;
        Ok(Box::new(self.clone_reader()?))
    }

    fn clone_flat_reader(&self) -> Result<Box<dyn FlatVectorsReader>> {
        Ok(Box::new(self.clone_reader()?))
    }
}

fn read_fields(
    input: &mut dyn DataInput,
    field_infos: &FieldInfos,
) -> Result<HashMap<i32, FieldEntry>> {
    let mut fields = HashMap::new();
    let mut field_number = input.read_int()?;
    while field_number != -1 {
        let info = field_infos
            .field_info_by_number(field_number)
            .ok_or_else(|| {
                LuceneError::CorruptIndex(format!("invalid field number: {field_number}"))
            })?;
        let entry = FieldEntry::create(input, info)?;
        fields.insert(info.number, entry);
        field_number = input.read_int()?;
    }
    Ok(fields)
}

impl FieldEntry {
    fn create(input: &mut dyn DataInput, field_info: &FieldInfo) -> Result<Self> {
        let vector_encoding = read_vector_encoding(input.read_int()?)?;
        let similarity_function = read_vector_similarity(input.read_int()?)?;
        let vector_data_offset = input.read_v_long()?;
        let vector_data_length = input.read_v_long()?;
        let dimension = input.read_v_int()?;
        let size = input.read_int()?;
        let ord_to_doc = OrdToDocDISIReaderConfiguration::read_stored_meta(input, size)?;

        if similarity_function != field_info.vector_similarity_function {
            return Err(LuceneError::CorruptIndex(format!(
                "inconsistent vector similarity function for field=\"{}\"",
                field_info.name
            )));
        }
        if dimension != field_info.vector_dimension {
            return Err(LuceneError::CorruptIndex(format!(
                "inconsistent vector dimension for field=\"{}\"",
                field_info.name
            )));
        }

        let byte_size = vector_encoding.byte_size();
        let expected_length = (size as i64)
            .checked_mul(dimension as i64)
            .and_then(|v| v.checked_mul(byte_size as i64))
            .ok_or_else(|| {
                LuceneError::CorruptIndex(format!(
                    "vector data length overflow for field=\"{}\"",
                    field_info.name
                ))
            })?;
        if expected_length != vector_data_length {
            return Err(LuceneError::CorruptIndex(format!(
                "vector data length {vector_data_length} does not match size={size} * dim={dimension} * byteSize={byte_size} = {expected_length} for field=\"{}\"",
                field_info.name
            )));
        }

        Ok(Self {
            field_info: field_info.clone(),
            similarity_function,
            vector_encoding,
            vector_data_offset,
            vector_data_length,
            dimension,
            size,
            ord_to_doc,
        })
    }
}

// -----------------------------------------------------------------------------
// Off-heap vector values
// -----------------------------------------------------------------------------

/// Port of `org.apache.lucene.codecs.lucene95.OffHeapFloatVectorValues`.
///
/// Java splits the class into `DenseOffHeapVectorValues` and
/// `SparseOffHeapVectorValues`; they differ only in how an ordinal maps back
/// to a document, in the iterator, and in which bulk scorer `scorer` builds,
/// so this port carries the sparse configuration in an `Option`. The third
/// subclass, `EmptyOffHeapVectorValues`, is [`EmptyOffHeapFloatVectorValues`].
struct OffHeapFloatVectorValues {
    dimension: i32,
    size: i32,
    byte_size: i32,
    slice: Mutex<Box<dyn IndexInput>>,
    config: Option<OrdToDocDISIReaderConfiguration>,
    data_input: Option<Box<dyn IndexInput>>,
    /// The ordinal-to-document reader of a sparse field, built once at load
    /// and shared between copies; `None` when the field is dense.
    ///
    /// Equivalent to `SparseOffHeapVectorValues.ordToDoc`, the
    /// `DirectMonotonicReader` the constructor builds
    /// (`OffHeapFloatVectorValues.java:225`, `OffHeapByteVectorValues.java:229`).
    ord_to_doc: Option<Arc<SharedDirectMonotonicReader>>,
    /// The `IndexedDISI disi` of a sparse field, which every `iterator()`
    /// call wraps (`OffHeapFloatVectorValues.java:205, 226-233, 272-274`,
    /// `OffHeapByteVectorValues.java:229-237, 258-260`); `None` when dense.
    disi: Option<Arc<Mutex<IndexedDISI>>>,
    /// Equivalent to the `protected final VectorSimilarityFunction
    /// similarityFunction` field.
    similarity_function: VectorSimilarityFunction,
    /// Equivalent to the `protected final FlatVectorsScorer flatVectorsScorer`
    /// field, which `scorer` scores through.
    flat_vectors_scorer: Arc<dyn FlatVectorsScorer>,
}

impl OffHeapFloatVectorValues {
    /// Opens one field's vectors.
    ///
    /// Equivalent to `OffHeapFloatVectorValues.load`: an empty configuration
    /// answers the empty values, a dense one indexes ordinals directly, and a
    /// sparse one reads the ordinal-to-document mapping.
    #[allow(clippy::too_many_arguments)]
    fn load(
        similarity_function: VectorSimilarityFunction,
        flat_vectors_scorer: Arc<dyn FlatVectorsScorer>,
        config: &OrdToDocDISIReaderConfiguration,
        dimension: i32,
        vector_data_offset: i64,
        vector_data_length: i64,
        vector_data: &dyn IndexInput,
    ) -> Result<Box<dyn FloatVectorValues>> {
        if config.is_empty() {
            return Ok(Box::new(EmptyOffHeapFloatVectorValues { dimension }));
        }
        let byte_size = dimension * VectorEncoding::FLOAT32.byte_size();
        let slice =
            Mutex::new(vector_data.slice("vector-data", vector_data_offset, vector_data_length)?);
        if config.is_dense() {
            Ok(Box::new(Self {
                dimension,
                size: config.size,
                byte_size,
                slice,
                config: None,
                data_input: None,
                ord_to_doc: None,
                disi: None,
                similarity_function,
                flat_vectors_scorer: Arc::clone(&flat_vectors_scorer),
            }))
        } else {
            Ok(Box::new(Self {
                dimension,
                size: config.size,
                byte_size,
                slice,
                config: Some(config.clone()),
                data_input: Some(vector_data.clone_input()?),
                // Built only for a sparse field, as Java builds it only in the
                // sparse constructor (`OffHeapFloatVectorValues.java:225`).
                ord_to_doc: Some(config.get_direct_monotonic_reader(vector_data)?),
                // `this.disi = new IndexedDISI(dataIn, ...)` of the sparse constructor.
                disi: Some(Arc::new(Mutex::new(config.get_indexed_disi(vector_data)?))),
                similarity_function,
                flat_vectors_scorer: Arc::clone(&flat_vectors_scorer),
            }))
        }
    }
}

impl OffHeapFloatVectorValues {
    /// Clones the instance together with a fresh view over the vector data.
    ///
    /// Shared by `copy` and `copy_float`, which differ only in the trait object
    /// they hand back; Rust cannot express Java's covariant return.
    fn clone_values(&self) -> Result<Self> {
        let slice = Mutex::new(
            self.slice
                .lock()
                .map_err(|_| {
                    LuceneError::illegal_state(
                        "off-heap vector slice mutex was poisoned".to_string(),
                    )
                })?
                .clone_input()?,
        );
        match (&self.config, &self.data_input) {
            (Some(config), Some(data_input)) => Ok(Self {
                dimension: self.dimension,
                size: self.size,
                byte_size: self.byte_size,
                slice,
                config: Some(config.clone()),
                data_input: Some(data_input.clone_input()?),
                ord_to_doc: self.ord_to_doc.clone(),
                // `copy()` builds a new sparse instance, with its own disi.
                disi: Some(Arc::new(Mutex::new(
                    config.get_indexed_disi(data_input.as_ref())?,
                ))),
                similarity_function: self.similarity_function,
                flat_vectors_scorer: Arc::clone(&self.flat_vectors_scorer),
            }),
            _ => Ok(Self {
                dimension: self.dimension,
                size: self.size,
                byte_size: self.byte_size,
                slice,
                config: None,
                data_input: None,
                ord_to_doc: None,
                disi: None,
                similarity_function: self.similarity_function,
                flat_vectors_scorer: Arc::clone(&self.flat_vectors_scorer),
            }),
        }
    }
}

impl KnnVectorValues for OffHeapFloatVectorValues {
    fn dimension(&self) -> i32 {
        self.dimension
    }

    fn size(&self) -> i32 {
        self.size
    }

    fn ord_to_doc(&self, ord: i32) -> i32 {
        match &self.ord_to_doc {
            Some(reader) => reader.get(i64::from(ord)) as i32,
            None => ord,
        }
    }

    /// Equivalent to `DenseOffHeapVectorValues.getAcceptOrds` (`acceptDocs`)
    /// and `SparseOffHeapVectorValues.getAcceptOrds`, a lazy view reading
    /// `acceptDocs.get(ordToDoc(ord))`.
    fn get_accept_ords<'a>(&self, accept_docs: Option<&'a dyn Bits>) -> Option<Box<dyn Bits + 'a>> {
        sparse_accept_ords(self.size, self.ord_to_doc.as_ref(), accept_docs)
    }

    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        Ok(Box::new(self.clone_values()?))
    }

    fn encoding(&self) -> VectorEncoding {
        VectorEncoding::FLOAT32
    }

    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
        match &self.disi {
            // `IndexedDISI.asDocIndexIterator(disi)` over the instance's one
            // disi (`OffHeapFloatVectorValues.java:272-274`,
            // `OffHeapByteVectorValues.java:258-260`): successive iterators
            // share its position.
            Some(disi) => Ok(IndexedDISI::as_doc_index_iterator(Arc::clone(disi))),
            None => Ok(Box::new(DenseDocIndexIterator::new(self.size))),
        }
    }
}

impl OffHeapFloatVectorValues {
    /// Returns the input slice the vectors are read from.
    ///
    /// **Divergence from Lucene 10.5.0.** Java returns the `IndexInput` field
    /// itself (`OffHeapFloatVectorValues.java:426-429`), which its callers then seek in.
    /// Reading a vector mutates that input, and
    /// [`FloatVectorValues::vector_value`] takes `&self`, so this port keeps
    /// the slice behind a `Mutex`; the accessor therefore hands back the lock
    /// guard, which derefs to the very same `IndexInput` Java returns.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the mutex was poisoned.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene95.OffHeapFloatVectorValues#getSlice`.
    fn get_slice(&self) -> Result<MutexGuard<'_, Box<dyn IndexInput>>> {
        self.slice.lock().map_err(|_| {
            LuceneError::illegal_state("off-heap vector slice mutex was poisoned".to_string())
        })
    }
}

impl FloatVectorValues for OffHeapFloatVectorValues {
    fn copy_float(&self) -> Result<Box<dyn FloatVectorValues>> {
        Ok(Box::new(self.clone_values()?))
    }

    fn vector_value(&self, ord: i32) -> Result<Vec<f32>> {
        let mut value = vec![0.0f32; self.dimension as usize];
        let mut slice = self.get_slice()?;
        slice.seek((ord as i64) * self.byte_size as i64)?;
        slice.read_floats(&mut value, 0, self.dimension as usize)?;
        Ok(value)
    }

    /// Scores `query` against a copy of these values, driven by the copy's
    /// own iterator.
    ///
    /// Equivalent to `DenseOffHeapVectorValues.scorer(float[])` and
    /// `SparseOffHeapVectorValues.scorer(float[])`
    /// (`OffHeapFloatVectorValues.java:179-199, 277-297`): both take a
    /// `copy()`, its `iterator()`, and
    /// `flatVectorsScorer.getRandomVectorScorer(similarityFunction, copy, query)`.
    fn scorer(&self, query: &[f32]) -> Result<Option<Box<dyn VectorScorer>>> {
        let copy = self.clone_values()?;
        let iterator = copy.iterator()?;
        let dense = copy.config.is_none();
        let random_vector_scorer = self.flat_vectors_scorer.get_random_vector_scorer_float(
            self.similarity_function,
            Box::new(copy),
            query,
        )?;
        Ok(Some(Box::new(OffHeapVectorScorer::new(
            random_vector_scorer,
            iterator,
            dense,
        ))))
    }
}

/// The values of a field that has no vector in this segment.
///
/// Equivalent to the private
/// `org.apache.lucene.codecs.lucene95.OffHeapFloatVectorValues.EmptyOffHeapVectorValues`
/// (`OffHeapFloatVectorValues.java:301-345`): it keeps the field's dimension,
/// has no vector, refuses `copy()` and `vectorValue`, and has no scorer.
pub(crate) struct EmptyOffHeapFloatVectorValues {
    pub(crate) dimension: i32,
}

impl KnnVectorValues for EmptyOffHeapFloatVectorValues {
    fn dimension(&self) -> i32 {
        self.dimension
    }

    fn size(&self) -> i32 {
        0
    }

    /// Java's `copy()` throws `UnsupportedOperationException`.
    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        Err(LuceneError::UnsupportedOperation(
            "EmptyOffHeapVectorValues::copy".to_string(),
        ))
    }

    fn encoding(&self) -> VectorEncoding {
        VectorEncoding::FLOAT32
    }

    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
        Ok(self.create_dense_iterator())
    }

    /// Java's `getAcceptOrds(Bits)` returns `null`.
    fn get_accept_ords<'a>(
        &self,
        _accept_docs: Option<&'a dyn Bits>,
    ) -> Option<Box<dyn Bits + 'a>> {
        None
    }
}

impl FloatVectorValues for EmptyOffHeapFloatVectorValues {
    /// Java's `vectorValue(int)` throws `UnsupportedOperationException`.
    fn vector_value(&self, _ord: i32) -> Result<Vec<f32>> {
        Err(LuceneError::UnsupportedOperation(
            "EmptyOffHeapVectorValues::vector_value".to_string(),
        ))
    }

    /// Java's `copy()` throws `UnsupportedOperationException`.
    fn copy_float(&self) -> Result<Box<dyn FloatVectorValues>> {
        Err(LuceneError::UnsupportedOperation(
            "EmptyOffHeapVectorValues::copy".to_string(),
        ))
    }

    /// Java's `scorer(float[])` returns `null`.
    fn scorer(&self, _query: &[f32]) -> Result<Option<Box<dyn VectorScorer>>> {
        Ok(None)
    }
}

/// Port of `org.apache.lucene.codecs.lucene95.OffHeapByteVectorValues`.
///
/// Java splits the class into `DenseOffHeapVectorValues` and
/// `SparseOffHeapVectorValues`; they differ only in how an ordinal maps back
/// to a document, in the iterator, and in which bulk scorer `scorer` builds,
/// so this port carries the sparse configuration in an `Option`. The third
/// subclass, `EmptyOffHeapVectorValues`, is [`EmptyOffHeapByteVectorValues`].
struct OffHeapByteVectorValues {
    dimension: i32,
    size: i32,
    byte_size: i32,
    slice: Mutex<Box<dyn IndexInput>>,
    config: Option<OrdToDocDISIReaderConfiguration>,
    data_input: Option<Box<dyn IndexInput>>,
    /// The ordinal-to-document reader of a sparse field, built once at load
    /// and shared between copies; `None` when the field is dense.
    ///
    /// Equivalent to `SparseOffHeapVectorValues.ordToDoc`, the
    /// `DirectMonotonicReader` the constructor builds
    /// (`OffHeapFloatVectorValues.java:225`, `OffHeapByteVectorValues.java:229`).
    ord_to_doc: Option<Arc<SharedDirectMonotonicReader>>,
    /// The `IndexedDISI disi` of a sparse field, which every `iterator()`
    /// call wraps (`OffHeapFloatVectorValues.java:205, 226-233, 272-274`,
    /// `OffHeapByteVectorValues.java:229-237, 258-260`); `None` when dense.
    disi: Option<Arc<Mutex<IndexedDISI>>>,
    /// Equivalent to the `protected final VectorSimilarityFunction
    /// similarityFunction` field.
    similarity_function: VectorSimilarityFunction,
    /// Equivalent to the `protected final FlatVectorsScorer flatVectorsScorer`
    /// field, which `scorer` scores through.
    flat_vectors_scorer: Arc<dyn FlatVectorsScorer>,
}

impl OffHeapByteVectorValues {
    /// Opens one field's vectors.
    ///
    /// Equivalent to `OffHeapByteVectorValues.load`: an empty configuration
    /// answers the empty values, a dense one indexes ordinals directly, and a
    /// sparse one reads the ordinal-to-document mapping.
    #[allow(clippy::too_many_arguments)]
    fn load(
        similarity_function: VectorSimilarityFunction,
        flat_vectors_scorer: Arc<dyn FlatVectorsScorer>,
        config: &OrdToDocDISIReaderConfiguration,
        dimension: i32,
        vector_data_offset: i64,
        vector_data_length: i64,
        vector_data: &dyn IndexInput,
    ) -> Result<Box<dyn ByteVectorValues>> {
        if config.is_empty() {
            return Ok(Box::new(EmptyOffHeapByteVectorValues { dimension }));
        }
        let byte_size = dimension * VectorEncoding::BYTE.byte_size();
        let slice =
            Mutex::new(vector_data.slice("vector-data", vector_data_offset, vector_data_length)?);
        if config.is_dense() {
            Ok(Box::new(Self {
                dimension,
                size: config.size,
                byte_size,
                slice,
                config: None,
                data_input: None,
                ord_to_doc: None,
                disi: None,
                similarity_function,
                flat_vectors_scorer: Arc::clone(&flat_vectors_scorer),
            }))
        } else {
            Ok(Box::new(Self {
                dimension,
                size: config.size,
                byte_size,
                slice,
                config: Some(config.clone()),
                data_input: Some(vector_data.clone_input()?),
                // Built only for a sparse field, as Java builds it only in the
                // sparse constructor (`OffHeapFloatVectorValues.java:225`).
                ord_to_doc: Some(config.get_direct_monotonic_reader(vector_data)?),
                // `this.disi = new IndexedDISI(dataIn, ...)` of the sparse constructor.
                disi: Some(Arc::new(Mutex::new(config.get_indexed_disi(vector_data)?))),
                similarity_function,
                flat_vectors_scorer: Arc::clone(&flat_vectors_scorer),
            }))
        }
    }
}

impl OffHeapByteVectorValues {
    /// Clones the instance together with a fresh view over the vector data.
    ///
    /// Shared by `copy` and `copy_byte`, which differ only in the trait object
    /// they hand back; Rust cannot express Java's covariant return.
    fn clone_values(&self) -> Result<Self> {
        let slice = Mutex::new(
            self.slice
                .lock()
                .map_err(|_| {
                    LuceneError::illegal_state(
                        "off-heap vector slice mutex was poisoned".to_string(),
                    )
                })?
                .clone_input()?,
        );
        match (&self.config, &self.data_input) {
            (Some(config), Some(data_input)) => Ok(Self {
                dimension: self.dimension,
                size: self.size,
                byte_size: self.byte_size,
                slice,
                config: Some(config.clone()),
                data_input: Some(data_input.clone_input()?),
                ord_to_doc: self.ord_to_doc.clone(),
                // `copy()` builds a new sparse instance, with its own disi.
                disi: Some(Arc::new(Mutex::new(
                    config.get_indexed_disi(data_input.as_ref())?,
                ))),
                similarity_function: self.similarity_function,
                flat_vectors_scorer: Arc::clone(&self.flat_vectors_scorer),
            }),
            _ => Ok(Self {
                dimension: self.dimension,
                size: self.size,
                byte_size: self.byte_size,
                slice,
                config: None,
                data_input: None,
                ord_to_doc: None,
                disi: None,
                similarity_function: self.similarity_function,
                flat_vectors_scorer: Arc::clone(&self.flat_vectors_scorer),
            }),
        }
    }
}

impl KnnVectorValues for OffHeapByteVectorValues {
    fn dimension(&self) -> i32 {
        self.dimension
    }

    fn size(&self) -> i32 {
        self.size
    }

    fn ord_to_doc(&self, ord: i32) -> i32 {
        match &self.ord_to_doc {
            Some(reader) => reader.get(i64::from(ord)) as i32,
            None => ord,
        }
    }

    /// Equivalent to `DenseOffHeapVectorValues.getAcceptOrds` (`acceptDocs`)
    /// and `SparseOffHeapVectorValues.getAcceptOrds`, a lazy view reading
    /// `acceptDocs.get(ordToDoc(ord))`.
    fn get_accept_ords<'a>(&self, accept_docs: Option<&'a dyn Bits>) -> Option<Box<dyn Bits + 'a>> {
        sparse_accept_ords(self.size, self.ord_to_doc.as_ref(), accept_docs)
    }

    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        Ok(Box::new(self.clone_values()?))
    }

    fn encoding(&self) -> VectorEncoding {
        VectorEncoding::BYTE
    }

    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
        match &self.disi {
            // `IndexedDISI.asDocIndexIterator(disi)` over the instance's one
            // disi (`OffHeapFloatVectorValues.java:272-274`,
            // `OffHeapByteVectorValues.java:258-260`): successive iterators
            // share its position.
            Some(disi) => Ok(IndexedDISI::as_doc_index_iterator(Arc::clone(disi))),
            None => Ok(Box::new(DenseDocIndexIterator::new(self.size))),
        }
    }
}

impl OffHeapByteVectorValues {
    /// Returns the input slice the vectors are read from.
    ///
    /// **Divergence from Lucene 10.5.0.** Java returns the `IndexInput` field
    /// itself (`OffHeapByteVectorValues.java:101-104`), which its callers then seek in.
    /// Reading a vector mutates that input, and
    /// [`ByteVectorValues::vector_value`] takes `&self`, so this port keeps
    /// the slice behind a `Mutex`; the accessor therefore hands back the lock
    /// guard, which derefs to the very same `IndexInput` Java returns.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the mutex was poisoned.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene95.OffHeapByteVectorValues#getSlice`.
    fn get_slice(&self) -> Result<MutexGuard<'_, Box<dyn IndexInput>>> {
        self.slice.lock().map_err(|_| {
            LuceneError::illegal_state("off-heap vector slice mutex was poisoned".to_string())
        })
    }

    /// Reads the vector at `target_ord` into `value`.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene95.OffHeapByteVectorValues#readValue`.
    /// Java reads into the instance's own `byteBuffer` and its `vectorValue`
    /// hands that buffer back, re-reading only when the ordinal changed; this
    /// port's `vector_value` returns an owned vector, so the destination is a
    /// parameter and there is no `lastOrd` to compare against.
    ///
    /// # Errors
    ///
    /// Returns whatever seeking or reading the slice raises, and
    /// [`LuceneError::IllegalState`] when the slice mutex was poisoned.
    fn read_value(&self, target_ord: i32, value: &mut [u8]) -> Result<()> {
        let mut slice = self.get_slice()?;
        slice.seek((target_ord as i64) * self.byte_size as i64)?;
        slice.read_bytes(value, 0, self.byte_size as usize)
    }
}

impl ByteVectorValues for OffHeapByteVectorValues {
    fn copy_byte(&self) -> Result<Box<dyn ByteVectorValues>> {
        Ok(Box::new(self.clone_values()?))
    }

    fn vector_value(&self, ord: i32) -> Result<Vec<u8>> {
        let mut value = vec![0u8; self.dimension as usize];
        self.read_value(ord, &mut value)?;
        Ok(value)
    }

    /// Scores `query` against a copy of these values, driven by the copy's
    /// own iterator.
    ///
    /// Equivalent to `DenseOffHeapVectorValues.scorer(byte[])` and
    /// `SparseOffHeapVectorValues.scorer(byte[])`
    /// (`OffHeapByteVectorValues.java:177-197, 281-301`): both take a
    /// `copy()`, its `iterator()`, and
    /// `flatVectorsScorer.getRandomVectorScorer(similarityFunction, copy, query)`.
    fn scorer(&self, query: &[u8]) -> Result<Option<Box<dyn VectorScorer>>> {
        let copy = self.clone_values()?;
        let iterator = copy.iterator()?;
        let dense = copy.config.is_none();
        let random_vector_scorer = self.flat_vectors_scorer.get_random_vector_scorer_byte(
            self.similarity_function,
            Box::new(copy),
            query,
        )?;
        Ok(Some(Box::new(OffHeapVectorScorer::new(
            random_vector_scorer,
            iterator,
            dense,
        ))))
    }
}

/// The values of a field that has no vector in this segment.
///
/// Equivalent to the private
/// `org.apache.lucene.codecs.lucene95.OffHeapByteVectorValues.EmptyOffHeapVectorValues`
/// (`OffHeapByteVectorValues.java:305-354`): it keeps the field's dimension,
/// has no vector, refuses `copy()` and `vectorValue`, and has no scorer.
struct EmptyOffHeapByteVectorValues {
    dimension: i32,
}

impl KnnVectorValues for EmptyOffHeapByteVectorValues {
    fn dimension(&self) -> i32 {
        self.dimension
    }

    fn size(&self) -> i32 {
        0
    }

    /// Java's `copy()` throws `UnsupportedOperationException`.
    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        Err(LuceneError::UnsupportedOperation(
            "EmptyOffHeapVectorValues::copy".to_string(),
        ))
    }

    fn encoding(&self) -> VectorEncoding {
        VectorEncoding::BYTE
    }

    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
        Ok(self.create_dense_iterator())
    }

    /// Java's `getAcceptOrds(Bits)` returns `null`.
    fn get_accept_ords<'a>(
        &self,
        _accept_docs: Option<&'a dyn Bits>,
    ) -> Option<Box<dyn Bits + 'a>> {
        None
    }
}

impl ByteVectorValues for EmptyOffHeapByteVectorValues {
    /// Java's `vectorValue(int)` throws `UnsupportedOperationException`.
    fn vector_value(&self, _ord: i32) -> Result<Vec<u8>> {
        Err(LuceneError::UnsupportedOperation(
            "EmptyOffHeapVectorValues::vector_value".to_string(),
        ))
    }

    /// Java's `copy()` throws `UnsupportedOperationException`.
    fn copy_byte(&self) -> Result<Box<dyn ByteVectorValues>> {
        Err(LuceneError::UnsupportedOperation(
            "EmptyOffHeapVectorValues::copy".to_string(),
        ))
    }

    /// Java's `scorer(byte[])` returns `null`.
    fn scorer(&self, _query: &[u8]) -> Result<Option<Box<dyn VectorScorer>>> {
        Ok(None)
    }
}

/// The `getAcceptOrds(Bits)` of an off-heap values class: `acceptDocs` itself
/// for a dense field, and for a sparse one a lazy view that asks
/// `acceptDocs.get(ordToDoc(ord))` through a shared handle on the field's
/// ordinal-to-document reader.
///
/// Equivalent to `DenseOffHeapVectorValues.getAcceptOrds` and
/// `SparseOffHeapVectorValues.getAcceptOrds` (`OffHeapFloatVectorValues.java:169,
/// 254-270`).
pub(crate) fn sparse_accept_ords<'a>(
    size: i32,
    ord_to_doc: Option<&Arc<SharedDirectMonotonicReader>>,
    accept_docs: Option<&'a dyn Bits>,
) -> Option<Box<dyn Bits + 'a>> {
    match ord_to_doc {
        None => accept_docs.map(|bits| Box::new(BorrowedBits::new(bits)) as Box<dyn Bits + 'a>),
        Some(reader) => {
            let reader = Arc::clone(reader);
            accept_ords_with(
                size,
                move |ord| reader.get(i64::from(ord)) as i32,
                accept_docs,
            )
        }
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
pub(crate) mod tests {
    /// The events a [`RecordingDirectory`] records: `(file name, event,
    /// hints)`, where the event is `"open"` or `"update"`.
    pub(crate) type EventLog = Arc<std::sync::Mutex<Vec<(String, &'static str, Vec<String>)>>>;

    /// A directory over a [`ByteBuffersDirectory`] that records how the files it
    /// opens are advised: the hints of the context each input is opened with,
    /// and the hints of every context an input is later updated to through
    /// [`IndexInput::update_io_context`].
    #[derive(Debug, Default)]
    pub(crate) struct RecordingDirectory {
        inner: ByteBuffersDirectory,
        /// Every recorded event.
        pub(crate) log: EventLog,
    }

    fn hint_names(context: &dyn crate::store::IOContext) -> Vec<String> {
        context
            .hints()
            .iter()
            .map(|hint| format!("{hint:?}"))
            .collect()
    }

    impl RecordingDirectory {
        /// Returns the hints of every recorded event of `event` on files whose
        /// name ends with `suffix`, each sorted: an `IOContext`'s hints are a
        /// set, so only membership is compared.
        pub(crate) fn events(&self, suffix: &str, event: &str) -> Vec<Vec<String>> {
            self.log
                .lock()
                .unwrap()
                .iter()
                .filter(|(name, kind, _)| name.ends_with(suffix) && *kind == event)
                .map(|(_, _, hints)| {
                    let mut hints = hints.clone();
                    hints.sort();
                    hints
                })
                .collect()
        }
    }

    impl crate::store::Directory for RecordingDirectory {
        fn list_all(&self) -> Result<Vec<String>> {
            self.inner.list_all()
        }

        fn delete_file(&self, name: &str) -> Result<()> {
            self.inner.delete_file(name)
        }

        fn file_length(&self, name: &str) -> Result<i64> {
            self.inner.file_length(name)
        }

        fn create_output(
            &self,
            name: &str,
            context: &dyn crate::store::IOContext,
        ) -> Result<Box<dyn IndexOutput>> {
            self.inner.create_output(name, context)
        }

        fn create_temp_output(
            &self,
            prefix: &str,
            suffix: &str,
            context: &dyn crate::store::IOContext,
        ) -> Result<Box<dyn IndexOutput>> {
            self.inner.create_temp_output(prefix, suffix, context)
        }

        fn sync(&self, names: &[String]) -> Result<()> {
            self.inner.sync(names)
        }

        fn sync_metadata(&self) -> Result<()> {
            self.inner.sync_metadata()
        }

        fn rename(&self, source: &str, dest: &str) -> Result<()> {
            self.inner.rename(source, dest)
        }

        fn open_input(
            &self,
            name: &str,
            context: &dyn crate::store::IOContext,
        ) -> Result<Box<dyn IndexInput>> {
            self.log
                .lock()
                .unwrap()
                .push((name.to_string(), "open", hint_names(context)));
            Ok(Box::new(RecordingInput {
                name: name.to_string(),
                inner: self.inner.open_input(name, context)?,
                log: Arc::clone(&self.log),
            }))
        }

        fn obtain_lock(&self, name: &str) -> Result<Box<dyn crate::store::Lock>> {
            self.inner.obtain_lock(name)
        }

        fn close(&mut self) -> Result<()> {
            self.inner.close()
        }

        fn get_pending_deletions(&self) -> Result<std::collections::HashSet<String>> {
            self.inner.get_pending_deletions()
        }
    }

    /// The input [`RecordingDirectory`] hands out.
    struct RecordingInput {
        name: String,
        inner: Box<dyn IndexInput>,
        log: EventLog,
    }

    impl DataInput for RecordingInput {
        /// `IndexInput.toString()` returns the resource description.
        fn to_java_string(&self) -> String {
            crate::store::IndexInput::resource_description(self).to_string()
        }

        fn read_byte(&mut self) -> Result<u8> {
            self.inner.read_byte()
        }

        fn read_bytes(&mut self, b: &mut [u8], offset: usize, len: usize) -> Result<()> {
            self.inner.read_bytes(b, offset, len)
        }

        fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
            self.inner.skip_bytes(num_bytes)
        }
    }

    impl IndexInput for RecordingInput {
        fn close(&mut self) -> Result<()> {
            self.inner.close()
        }

        fn file_pointer(&self) -> i64 {
            self.inner.file_pointer()
        }

        fn length(&self) -> i64 {
            self.inner.length()
        }

        fn seek(&mut self, pos: i64) -> Result<()> {
            self.inner.seek(pos)
        }

        fn slice(
            &self,
            slice_description: &str,
            offset: i64,
            length: i64,
        ) -> Result<Box<dyn IndexInput>> {
            self.inner.slice(slice_description, offset, length)
        }

        fn clone_input(&self) -> Result<Box<dyn IndexInput>> {
            Ok(Box::new(RecordingInput {
                name: self.name.clone(),
                inner: self.inner.clone_input()?,
                log: Arc::clone(&self.log),
            }))
        }

        fn resource_description(&self) -> &str {
            self.inner.resource_description()
        }

        fn update_io_context(&self, context: &dyn crate::store::IOContext) -> Result<()> {
            self.log
                .lock()
                .unwrap()
                .push((self.name.clone(), "update", hint_names(context)));
            self.inner.update_io_context(context)
        }

        fn random_access_slice(
            &self,
            offset: i64,
            length: i64,
        ) -> Result<Box<dyn crate::store::RandomAccessInput>> {
            self.inner.random_access_slice(offset, length)
        }
    }

    /// The data file is opened with `FileTypeHint.DATA`,
    /// `FileDataHint.KNN_VECTORS` and `DataAccessHint.RANDOM`
    /// (`Lucene99FlatVectorsReader.java:70-93`).
    #[test]
    fn vector_data_is_opened_with_the_hints_lucene_gives_it() -> Result<()> {
        let recording = Arc::new(RecordingDirectory::default());
        let dir = Arc::clone(&recording) as Arc<dyn crate::store::Directory>;
        let seg_info = make_segment_info(&dir, 1)?;
        let field_info = float_field_info("float_field", 0, 2)?;
        let field_infos = FieldInfos::new(vec![field_info.clone()])?;
        let seg_updates = BufferedUpdates::default();
        let io_ctx = DefaultIOContext::default();
        let (write_state, read_state) =
            make_states(&dir, &seg_info, &field_infos, &seg_updates, &io_ctx);
        let format = Lucene99FlatVectorsFormat::new(DefaultFlatVectorScorer::INSTANCE);
        {
            let mut writer =
                format.fields_writer_flat(&crate::codecs::state::owned_state(&write_state))?;
            let mut float_writer = match KnnVectorsWriter::add_field(writer.as_mut(), &field_info)?
            {
                FieldVectorWriter::Float(w) => w,
                _ => panic!("expected float field writer"),
            };
            float_writer.add_value(0, vec![1.0, 2.0])?;
            writer.flush(1, None)?;
            writer.finish()?;
            writer.close()?;
        }
        let _reader = format.fields_reader_flat(&read_state)?;
        assert_eq!(
            recording.events(".vec", "open"),
            vec![vec![
                "Data".to_string(),
                "KnnVectors".to_string(),
                "Random".to_string()
            ]]
        );
        Ok(())
    }

    /// A merge reads the vectors sequentially, so
    /// `Lucene99FlatVectorsReader.getMergeInstance()` re-advises the data
    /// input with `DataAccessHint.SEQUENTIAL`, and `finishMerge()` reverts it
    /// to the reader's own context (`Lucene99FlatVectorsReader.java:203-208,
    /// 300-305`).
    #[test]
    fn merge_instance_advises_the_vector_data_sequential_and_finish_merge_reverts_it() -> Result<()>
    {
        let recording = Arc::new(RecordingDirectory::default());
        let dir = Arc::clone(&recording) as Arc<dyn crate::store::Directory>;
        let seg_info = make_segment_info(&dir, 2)?;
        let field_info = float_field_info("float_field", 0, 2)?;
        let field_infos = FieldInfos::new(vec![field_info.clone()])?;
        let seg_updates = BufferedUpdates::default();
        let io_ctx = DefaultIOContext::default();
        let (write_state, read_state) =
            make_states(&dir, &seg_info, &field_infos, &seg_updates, &io_ctx);

        let format = Lucene99FlatVectorsFormat::new(DefaultFlatVectorScorer::INSTANCE);
        {
            let mut writer =
                format.fields_writer_flat(&crate::codecs::state::owned_state(&write_state))?;
            let mut float_writer = match KnnVectorsWriter::add_field(writer.as_mut(), &field_info)?
            {
                FieldVectorWriter::Float(w) => w,
                _ => panic!("expected float field writer"),
            };
            float_writer.add_value(0, vec![1.0, 2.0])?;
            float_writer.add_value(1, vec![3.0, 4.0])?;
            writer.flush(2, None)?;
            writer.finish()?;
            writer.close()?;
        }

        let reader = format.fields_reader_flat(&read_state)?;
        assert!(recording.events(".vec", "update").is_empty());
        let mut merge_instance = reader.get_merge_instance()?;
        assert_eq!(
            recording.events(".vec", "update"),
            vec![vec!["Sequential".to_string()]],
            "getMergeInstance advises the data input sequential"
        );
        merge_instance.finish_merge()?;
        assert_eq!(
            recording.events(".vec", "update")[1],
            recording.events(".vec", "open")[0],
            "finishMerge reverts to the context the data input was opened with"
        );
        Ok(())
    }

    /// The four serialization sites for `VectorEncoding` and
    /// `VectorSimilarityFunction` each carry their own ordinal table. Java has
    /// one, `Enum.ordinal()`, so the tables must agree with the declaration
    /// order of the Rust enums and therefore with each other; a divergence here
    /// silently writes an unreadable index.
    /// The flat writer's merge data writer fails, before writing a byte, on a
    /// merged view without subs: Java's `writeVectorData` sizes its buffer with
    /// `dimension()`, which throws there (`Lucene99FlatVectorsWriter.java:339-341`).
    /// Lucene 10.5.0 fails a merge of such an index at exactly this call
    /// (`SqMergeProbe`).
    #[test]
    fn write_vector_data_refuses_an_empty_merged_view() {
        use crate::codecs::knn_vectors::{FloatVectorValuesSub, MergedFloat32VectorValues};
        use crate::index::merge::MergeState;
        use crate::index::vector_values::from_floats;
        use crate::store::Directory;

        let dir = ByteBuffersDirectory::new();
        let context = DefaultIOContext::new(Vec::new()).unwrap();
        let mut output = dir.create_output("data", &context).unwrap();
        let state = MergeState::from_fields_producers(Vec::new(), Vec::new());
        let empty = MergedFloat32VectorValues::new(Vec::new(), &state).unwrap();
        assert!(matches!(
            Lucene99FlatVectorsWriter::write_vector_data(output.as_mut(), &empty),
            Err(LuceneError::IllegalArgument(_))
        ));
        assert_eq!(output.file_pointer(), 0);

        // Positive control: a view with one two-dimension vector is written.
        let sub = FloatVectorValuesSub::new(
            Arc::new(|doc: i32| doc),
            Box::new(from_floats(vec![vec![1.0, 2.0]], 2)),
        )
        .unwrap();
        let one = MergedFloat32VectorValues::new(vec![sub], &state).unwrap();
        let docs = Lucene99FlatVectorsWriter::write_vector_data(output.as_mut(), &one).unwrap();
        assert_eq!(docs.cardinality(), 1);
        assert_eq!(output.file_pointer(), 8);
    }

    #[test]
    fn vector_ordinals_match_the_enum_declaration_order() {
        use crate::index::{VectorEncoding, VectorSimilarityFunction};

        for encoding in [VectorEncoding::BYTE, VectorEncoding::FLOAT32] {
            let ordinal = encoding as i32;
            assert_eq!(super::vector_encoding_ordinal(encoding), ordinal);
            assert_eq!(super::read_vector_encoding(ordinal).unwrap(), encoding);
        }

        for similarity in [
            VectorSimilarityFunction::EUCLIDEAN,
            VectorSimilarityFunction::DOT_PRODUCT,
            VectorSimilarityFunction::COSINE,
            VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT,
        ] {
            let ordinal = similarity as i32;
            assert_eq!(super::vector_similarity_ordinal(similarity), ordinal);
            assert_eq!(super::read_vector_similarity(ordinal).unwrap(), similarity);
        }
    }

    use super::*;
    use crate::codecs::stub::{BufferedUpdates, FieldInfos};
    use crate::codecs::{FlatVectorsFormat, KnnVectorsFormat};
    use crate::index::field_infos::FieldInfo;
    use crate::index::{
        segment_file_name, DocValuesSkipIndexType, DocValuesType, IndexOptions, SegmentInfo,
        VectorEncoding, VectorSimilarityFunction,
    };
    use crate::search::{DocIdSetIterator, NO_MORE_DOCS};
    use crate::store::{ByteBuffersDirectory, DefaultIOContext};
    use crate::util::string_helper::ID_LENGTH;
    use crate::util::Version;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn make_segment_info(
        dir: &Arc<dyn crate::store::Directory>,
        max_doc: i32,
    ) -> Result<SegmentInfo> {
        SegmentInfo::new_without_codec(
            Arc::clone(dir),
            Version::LUCENE_10_5_0,
            None,
            "test".to_string(),
            max_doc,
            false,
            false,
            HashMap::new(),
            [0u8; ID_LENGTH],
            HashMap::new(),
            crate::search::Sort::new(),
        )
    }

    /// A sparse field's values keep one `IndexedDISI`, which every
    /// `iterator()` call wraps (`OffHeapFloatVectorValues.java:272-274`,
    /// `OffHeapByteVectorValues.java:258-260`): a second iterator continues
    /// where the first one stopped, as `BaseKnnVectorsFormatTestCase
    /// .testSortedIndexBytes` relies on. Documents 0, 2 and 3 of 4 hold a
    /// vector.
    #[test]
    fn sparse_iterators_share_one_disi() {
        use crate::codecs::{register_codec, Lucene104Codec};
        use crate::document::{Document, KnnByteVectorField, KnnFloatVectorField};
        use crate::index::index_writer::IndexWriter;
        use crate::index::index_writer_config::IndexWriterConfig;
        let _ = register_codec("Lucene104", Lucene104Codec::new());
        let dir: Arc<dyn crate::store::Directory> = Arc::new(ByteBuffersDirectory::new());
        let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
        for i in 0..4u8 {
            let mut doc = Document::new();
            if i != 1 {
                doc.add(Box::new(
                    KnnFloatVectorField::with_euclidean("f", &[f32::from(i), 1.0]).unwrap(),
                ));
                doc.add(Box::new(
                    KnnByteVectorField::with_euclidean("b", &[i, 1]).unwrap(),
                ));
            }
            writer.add_document(&doc).unwrap();
        }
        writer.force_merge(1).unwrap();
        let reader = crate::index::directory_reader::open_with_writer(&*writer).unwrap();
        let leaves = Arc::clone(&reader).leaves().unwrap();
        let leaf = leaves[0].leaf_reader();
        let floats = leaf.get_float_vector_values("f").unwrap().unwrap();
        let bytes = leaf.get_byte_vector_values("b").unwrap().unwrap();
        let mut float_docs = Vec::new();
        let mut byte_docs = Vec::new();
        for _ in 0..4 {
            float_docs.push(floats.iterator().unwrap().next_doc().unwrap());
            byte_docs.push(bytes.iterator().unwrap().next_doc().unwrap());
        }
        assert_eq!(float_docs, [0, 2, 3, NO_MORE_DOCS]);
        assert_eq!(byte_docs, [0, 2, 3, NO_MORE_DOCS]);
        drop((floats, bytes, leaves));
        reader.close().unwrap();
        writer.close().unwrap();
    }

    /// An output that counts the `writeByte`/`writeBytes` calls it receives:
    /// the calls a `MockIndexOutputWrapper` draws from its random state for.
    struct CountingOutput {
        data: Vec<u8>,
        calls: usize,
    }

    impl crate::store::DataOutput for CountingOutput {
        fn write_byte(&mut self, b: u8) -> Result<()> {
            self.calls += 1;
            self.data.push(b);
            Ok(())
        }

        fn write_bytes(&mut self, b: &[u8], offset: usize, len: usize) -> Result<()> {
            self.calls += 1;
            self.data.extend_from_slice(&b[offset..offset + len]);
            Ok(())
        }
    }

    impl IndexOutput for CountingOutput {
        fn close(&mut self) -> Result<()> {
            Ok(())
        }

        fn file_pointer(&self) -> i64 {
            self.data.len() as i64
        }

        fn checksum(&self) -> Result<i64> {
            Ok(0)
        }

        fn resource_description(&self) -> &str {
            "counting"
        }

        fn name(&self) -> &str {
            "counting"
        }
    }

    /// Every float vector goes to the vector-data file through a
    /// little-endian buffer and one `writeBytes` call
    /// (`Lucene99FlatVectorsWriter.java:205-215`, `244-255`, `336-351`): the
    /// flush, sorted-flush and merge paths each make one write per vector,
    /// with the vector's little-endian bytes. One write per byte instead makes
    /// `MockIndexOutputWrapper` draw once per byte, which shifted the random
    /// I/O exceptions of `BaseIndexFileFormatTestCase.testRandomExceptions`
    /// away from Java's.
    #[test]
    fn float_vectors_are_written_with_one_write_per_vector() {
        let vectors = vec![vec![1.0f32, -2.5, 3.25], vec![0.5, 0.0, -1.0]];
        let expected: Vec<u8> = vectors
            .iter()
            .flatten()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let info = float_field_info("f", 0, 3).unwrap();
        let mut field_writer = DefaultFlatFieldVectorsWriter::<Vec<f32>>::new(info.clone());
        for (doc, vector) in vectors.iter().enumerate() {
            field_writer.add_value(doc as i32, vector.clone()).unwrap();
        }

        let mut out = CountingOutput {
            data: Vec::new(),
            calls: 0,
        };
        Lucene99FlatVectorsWriter::write_float32_vectors(&mut out, &field_writer, &info).unwrap();
        assert_eq!((out.calls, &out.data), (2, &expected));

        let mut out = CountingOutput {
            data: Vec::new(),
            calls: 0,
        };
        Lucene99FlatVectorsWriter::write_sorted_float32_vectors(&mut out, &field_writer, &[0, 1])
            .unwrap();
        assert_eq!((out.calls, &out.data), (2, &expected));

        let values = crate::index::vector_values::from_floats(vectors.clone(), 3);
        let mut out = CountingOutput {
            data: Vec::new(),
            calls: 0,
        };
        Lucene99FlatVectorsWriter::write_vector_data(&mut out, &values).unwrap();
        assert_eq!((out.calls, &out.data), (2, &expected));
    }

    fn make_states<'a>(
        dir: &'a Arc<dyn crate::store::Directory>,
        seg_info: &'a SegmentInfo,
        field_infos: &'a FieldInfos,
        seg_updates: &'a BufferedUpdates,
        io_ctx: &'a DefaultIOContext,
    ) -> (SegmentWriteState<'a>, SegmentReadState<'a>) {
        let info_stream = &crate::util::NoOutputInfoStream;
        let write_state = SegmentWriteState::new(
            info_stream,
            Arc::clone(dir),
            seg_info,
            field_infos,
            seg_updates,
            io_ctx,
        );
        let read_state = SegmentReadState::new(dir.as_ref(), seg_info, field_infos, io_ctx);
        (write_state, read_state)
    }

    fn float_field_info(name: &str, number: i32, dim: i32) -> Result<FieldInfo> {
        FieldInfo::new_full(
            name,
            number,
            false,
            false,
            false,
            IndexOptions::NONE,
            DocValuesType::NONE,
            DocValuesSkipIndexType::NONE,
            -1,
            crate::util::java_collections::JavaStringMap::default(),
            0,
            0,
            0,
            dim,
            VectorEncoding::FLOAT32,
            VectorSimilarityFunction::EUCLIDEAN,
            false,
            false,
        )
    }

    fn byte_field_info(name: &str, number: i32, dim: i32) -> Result<FieldInfo> {
        FieldInfo::new_full(
            name,
            number,
            false,
            false,
            false,
            IndexOptions::NONE,
            DocValuesType::NONE,
            DocValuesSkipIndexType::NONE,
            -1,
            crate::util::java_collections::JavaStringMap::default(),
            0,
            0,
            0,
            dim,
            VectorEncoding::BYTE,
            VectorSimilarityFunction::DOT_PRODUCT,
            false,
            false,
        )
    }

    #[test]
    fn format_name_and_max_dimensions() {
        let format = Lucene99FlatVectorsFormat::new(DefaultFlatVectorScorer::INSTANCE);
        assert_eq!(format.name(), "Lucene99FlatVectorsFormat");
        assert_eq!(format.get_max_dimensions("any"), 1024);
    }

    #[test]
    fn round_trip_dense_float_vectors() -> Result<()> {
        let dir = Arc::new(ByteBuffersDirectory::new()) as Arc<dyn crate::store::Directory>;
        let seg_info = make_segment_info(&dir, 3)?;
        let field_info = float_field_info("float_field", 0, 2)?;
        let field_infos = FieldInfos::new(vec![field_info.clone()])?;
        let seg_updates = BufferedUpdates::default();
        let io_ctx = DefaultIOContext::default();
        let (write_state, read_state) =
            make_states(&dir, &seg_info, &field_infos, &seg_updates, &io_ctx);

        let format = Lucene99FlatVectorsFormat::new(DefaultFlatVectorScorer::INSTANCE);
        {
            let mut writer =
                format.fields_writer_flat(&crate::codecs::state::owned_state(&write_state))?;
            let field_writer = KnnVectorsWriter::add_field(writer.as_mut(), &field_info)?;
            let mut float_writer = match field_writer {
                FieldVectorWriter::Float(w) => w,
                _ => panic!("expected float field writer"),
            };
            float_writer.add_value(0, vec![1.0, 2.0])?;
            float_writer.add_value(1, vec![3.0, 4.0])?;
            float_writer.add_value(2, vec![5.0, 6.0])?;
            writer.flush(3, None)?;
            writer.finish()?;
            writer.close()?;
        }

        let reader = format.fields_reader_flat(&read_state)?;
        let values = reader
            .get_float_vector_values("float_field")?
            .expect("the field has vectors");
        assert_eq!(values.size(), 3);
        assert_eq!(values.dimension(), 2);
        assert_eq!(values.vector_value(0)?, vec![1.0, 2.0]);
        assert_eq!(values.vector_value(1)?, vec![3.0, 4.0]);
        assert_eq!(values.vector_value(2)?, vec![5.0, 6.0]);

        let mut iter = values.iterator()?;
        assert_eq!(iter.next_doc()?, 0);
        assert_eq!(iter.next_doc()?, 1);
        assert_eq!(iter.next_doc()?, 2);
        assert_eq!(iter.next_doc()?, NO_MORE_DOCS);

        let mut scorer = reader.get_random_vector_scorer_float("float_field", &[1.0, 2.0])?;
        let score_same = scorer.score(0)?;
        let score_other = scorer.score(1)?;
        assert!(score_same > score_other);

        reader.check_integrity()?;
        Ok(())
    }

    #[test]
    fn round_trip_sparse_byte_vectors() -> Result<()> {
        let dir = Arc::new(ByteBuffersDirectory::new()) as Arc<dyn crate::store::Directory>;
        let seg_info = make_segment_info(&dir, 5)?;
        let field_info = byte_field_info("byte_field", 0, 2)?;
        let field_infos = FieldInfos::new(vec![field_info.clone()])?;
        let seg_updates = BufferedUpdates::default();
        let io_ctx = DefaultIOContext::default();
        let (write_state, read_state) =
            make_states(&dir, &seg_info, &field_infos, &seg_updates, &io_ctx);

        let format = Lucene99FlatVectorsFormat::new(DefaultFlatVectorScorer::INSTANCE);
        {
            let mut writer =
                format.fields_writer_flat(&crate::codecs::state::owned_state(&write_state))?;
            let field_writer = KnnVectorsWriter::add_field(writer.as_mut(), &field_info)?;
            let mut byte_writer = match field_writer {
                FieldVectorWriter::Byte(w) => w,
                _ => panic!("expected byte field writer"),
            };
            byte_writer.add_value(0, vec![10, 20])?;
            byte_writer.add_value(2, vec![30, 40])?;
            writer.flush(5, None)?;
            writer.finish()?;
            writer.close()?;
        }

        let reader = format.fields_reader_flat(&read_state)?;
        let values = reader
            .get_byte_vector_values("byte_field")?
            .expect("the field has vectors");
        assert_eq!(values.size(), 2);
        assert_eq!(values.dimension(), 2);
        assert_eq!(values.vector_value(0)?, vec![10, 20]);
        assert_eq!(values.vector_value(1)?, vec![30, 40]);
        assert_eq!(values.ord_to_doc(0), 0);
        assert_eq!(values.ord_to_doc(1), 2);

        let mut iter = values.iterator()?;
        assert_eq!(iter.next_doc()?, 0);
        assert_eq!(iter.next_doc()?, 2);
        assert_eq!(iter.next_doc()?, NO_MORE_DOCS);
        Ok(())
    }

    #[test]
    fn written_files_match_segment_name() -> Result<()> {
        let dir = Arc::new(ByteBuffersDirectory::new()) as Arc<dyn crate::store::Directory>;
        let seg_info = make_segment_info(&dir, 1)?;
        let field_info = float_field_info("f", 0, 1)?;
        let field_infos = FieldInfos::new(vec![field_info.clone()])?;
        let seg_updates = BufferedUpdates::default();
        let io_ctx = DefaultIOContext::default();
        let (write_state, _read_state) =
            make_states(&dir, &seg_info, &field_infos, &seg_updates, &io_ctx);

        let format = Lucene99FlatVectorsFormat::new(DefaultFlatVectorScorer::INSTANCE);
        {
            let mut writer =
                format.fields_writer_flat(&crate::codecs::state::owned_state(&write_state))?;
            let field_writer = KnnVectorsWriter::add_field(writer.as_mut(), &field_info)?;
            let mut float_writer = match field_writer {
                FieldVectorWriter::Float(w) => w,
                _ => panic!("expected float field writer"),
            };
            float_writer.add_value(0, vec![1.0])?;
            writer.flush(1, None)?;
            writer.finish()?;
            writer.close()?;
        }

        let files = dir.list_all()?;
        assert!(files.contains(&segment_file_name("test", "", "vemf")));
        assert!(files.contains(&segment_file_name("test", "", "vec")));
        Ok(())
    }
}

/// Returns the documents a field carries, in ordinal order.
///
/// Java iterates the `DocsWithFieldSet` where it needs this; the list is what
/// the iteration produces, and both callers below need it twice.
fn docs_with_field_to_docs(docs_with_field: &DocsWithFieldSet) -> Result<Vec<i32>> {
    let mut docs = Vec::with_capacity(docs_with_field.cardinality() as usize);
    let mut iterator = docs_with_field.iterator()?;
    loop {
        let doc = iterator.next_doc()?;
        if doc == crate::search::NO_MORE_DOCS {
            break;
        }
        docs.push(doc);
    }
    Ok(docs)
}

/// Returns the document set of a field under the new numbering.
///
/// Equivalent to the `newDocsWithField` that
/// `KnnVectorsWriter.mapOldOrdToNewOrd` fills as its fifth argument: the same
/// documents, at the positions the sort moved them to.
fn new_docs_with_field(old_doc_ids: &[i32], sort_map: &SorterDocMap) -> Result<DocsWithFieldSet> {
    let mut new_ids: Vec<i32> = old_doc_ids
        .iter()
        .map(|&doc| sort_map.old_to_new(doc))
        .collect();
    new_ids.sort_unstable();
    let mut set = DocsWithFieldSet::new();
    for doc in new_ids {
        set.add(doc)?;
    }
    Ok(set)
}

impl std::fmt::Display for Lucene99FlatVectorsFormat {
    /// Port of `Lucene99FlatVectorsFormat.toString()`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Lucene99FlatVectorsFormat(vectorsScorer={})",
            self.vectors_scorer
        )
    }
}
