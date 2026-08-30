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
use std::sync::{Arc, Mutex, OnceLock};

use crate::codecs::codec_util::{
    check_footer, check_index_header, checksum_entire_file, retrieve_checksum, write_footer,
    write_index_header,
};
use crate::codecs::hnsw::flat_vectors::{
    DefaultFlatFieldVectorsWriter, DefaultFlatVectorScorer, DocsWithFieldSet,
    FlatFieldVectorsWriter, FlatVectorsFormat, FlatVectorsReader, FlatVectorsScorer,
    FlatVectorsWriter,
};
use crate::codecs::knn_vectors::{
    BufferingKnnVectorsWriter, FieldVectorWriter, KnnFieldVectorsWriter, KnnVectorsFormat,
    KnnVectorsReader, KnnVectorsWriter, SorterDocMap,
};
use crate::codecs::lucene90::indexed_disi::{write_bit_set, IndexedDISI, DEFAULT_DENSE_RANK_POWER};
use crate::codecs::lucene95::OrdToDocDISIReaderConfiguration;
use crate::codecs::postings::MergeState;
use crate::codecs::state::{OwnedSegmentWriteState, SegmentReadState, SegmentWriteState};
use crate::codecs::stub::FieldInfo;
use crate::error::{LuceneError, Result};
use crate::index::vector_values::{
    ByteVectorValues, DenseDocIndexIterator, DocIndexIterator, EmptyByteVectorValues,
    EmptyFloatVectorValues, FloatVectorValues, KnnVectorValues,
};
use crate::index::{segment_file_name, FieldInfos, VectorEncoding, VectorSimilarityFunction};
use crate::search::{AcceptDocs, DocIdSetIterator, NO_MORE_DOCS};
use crate::store::{DataInput, IndexInput, IndexOutput};
use crate::util::extra::LongValues;
use crate::util::hnsw::RandomVectorScorer;
use crate::util::packed::{DirectMonotonicMeta, DirectMonotonicReader, DirectMonotonicWriter};

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
#[derive(Debug, Default, Clone, Copy)]
pub struct Lucene99FlatVectorsFormat {
    vectors_scorer: DefaultFlatVectorScorer,
}

impl Lucene99FlatVectorsFormat {
    /// Creates a new format instance using the default flat-vector scorer.
    pub const fn new(vectors_scorer: DefaultFlatVectorScorer) -> Self {
        Self { vectors_scorer }
    }

    fn create_writer<'a>(
        &self,
        state: &SegmentWriteState<'a>,
    ) -> Result<Lucene99FlatVectorsWriter> {
        ensure_registered();
        Lucene99FlatVectorsWriter::new(state, self.vectors_scorer)
    }

    fn create_reader<'a>(&self, state: &SegmentReadState<'a>) -> Result<Lucene99FlatVectorsReader> {
        ensure_registered();
        Lucene99FlatVectorsReader::new(state, self.vectors_scorer)
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
        state: &SegmentWriteState<'_>,
    ) -> Result<Box<dyn FlatVectorsWriter>> {
        Ok(Box::new(self.create_writer(state)?))
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
    vector_scorer: DefaultFlatVectorScorer,
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

enum FieldWriterEntry {
    Float(
        Arc<Mutex<DefaultFlatFieldVectorsWriter<Vec<f32>>>>,
        FieldInfo,
    ),
    Byte(
        Arc<Mutex<DefaultFlatFieldVectorsWriter<Vec<u8>>>>,
        FieldInfo,
    ),
}

impl fmt::Debug for FieldWriterEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FieldWriterEntry::Float(_, info) => f.debug_tuple("Float").field(&info.name).finish(),
            FieldWriterEntry::Byte(_, info) => f.debug_tuple("Byte").field(&info.name).finish(),
        }
    }
}

struct SharedFieldWriter<T> {
    inner: Arc<Mutex<DefaultFlatFieldVectorsWriter<T>>>,
}

impl<T: Clone + Send + Sync + 'static> KnnFieldVectorsWriter<T> for SharedFieldWriter<T> {
    fn add_value(&mut self, doc_id: i32, vector_value: T) -> Result<()> {
        let mut guard = self.inner.lock().map_err(|_| {
            LuceneError::IllegalState("FlatFieldVectorsWriter mutex was poisoned".to_string())
        })?;
        guard.add_value(doc_id, vector_value)
    }

    fn copy_value(&self, vector_value: T) -> T {
        vector_value
    }

    fn ram_bytes_used(&self) -> i64 {
        // The shared handle owns nothing: the buffer it locks is accounted by
        // the writer that also holds it, and counting it here would count it
        // twice in `KnnVectorsWriter::ram_bytes_used`.
        0
    }
}

impl Lucene99FlatVectorsWriter {
    /// Creates a new writer for the given segment write state.
    pub fn new(
        state: &SegmentWriteState<'_>,
        vectors_scorer: DefaultFlatVectorScorer,
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

        let meta = state
            .directory
            .create_output(&meta_file_name, state.context)?;
        let vector_data = state
            .directory
            .create_output(&vector_data_file_name, state.context)?;

        let mut meta_opt = Some(meta);
        let mut vector_data_opt = Some(vector_data);
        let result = (|| -> Result<Self> {
            let meta = meta_opt.as_mut().ok_or_else(|| {
                LuceneError::IllegalState("meta output missing during writer creation".to_string())
            })?;
            let vector_data = vector_data_opt.as_mut().ok_or_else(|| {
                LuceneError::IllegalState(
                    "vector data output missing during writer creation".to_string(),
                )
            })?;
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
            Ok(Self {
                meta: meta_opt.take(),
                vector_data: vector_data_opt.take(),
                vector_scorer: vectors_scorer,
                fields: Vec::new(),
                segment_max_doc: state.segment_info.max_doc().ok(),
                finished: false,
            })
        })();

        if result.is_err() {
            let _ = close_outputs(&mut meta_opt, &mut vector_data_opt);
        }
        result
    }

    fn write_field_float(
        &mut self,
        field_writer: &mut DefaultFlatFieldVectorsWriter<Vec<f32>>,
        field_info: &FieldInfo,
        max_doc: i32,
    ) -> Result<()> {
        let vector_data = self.vector_data.as_mut().ok_or_else(|| {
            LuceneError::IllegalState("Lucene99FlatVectorsWriter is already closed".to_string())
        })?;
        let vector_data_offset = align_output(vector_data.as_mut(), VectorEncoding::FLOAT32)?;
        for vector in field_writer.vectors() {
            vector_data.write_floats(vector, 0, vector.len())?;
        }
        let vector_data_length = vector_data.file_pointer() - vector_data_offset;
        let docs_with_field = field_writer.docs_with_field_set().clone();

        let meta = self.meta.as_mut().ok_or_else(|| {
            LuceneError::IllegalState("Lucene99FlatVectorsWriter is already closed".to_string())
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

    fn write_field_byte(
        &mut self,
        field_writer: &mut DefaultFlatFieldVectorsWriter<Vec<u8>>,
        field_info: &FieldInfo,
        max_doc: i32,
    ) -> Result<()> {
        let vector_data = self.vector_data.as_mut().ok_or_else(|| {
            LuceneError::IllegalState("Lucene99FlatVectorsWriter is already closed".to_string())
        })?;
        let vector_data_offset = align_output(vector_data.as_mut(), VectorEncoding::BYTE)?;
        for vector in field_writer.vectors() {
            vector_data.write_bytes(vector, 0, vector.len())?;
        }
        let vector_data_length = vector_data.file_pointer() - vector_data_offset;
        let docs_with_field = field_writer.docs_with_field_set().clone();

        let meta = self.meta.as_mut().ok_or_else(|| {
            LuceneError::IllegalState("Lucene99FlatVectorsWriter is already closed".to_string())
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
    fn add_field(&mut self, field_info: &FieldInfo) -> Result<FieldVectorWriter> {
        if self.finished {
            return Err(LuceneError::IllegalState(
                "Lucene99FlatVectorsWriter is already finished".to_string(),
            ));
        }
        match field_info.vector_encoding {
            VectorEncoding::FLOAT32 => {
                let writer = Arc::new(Mutex::new(DefaultFlatFieldVectorsWriter::<Vec<f32>>::new(
                    field_info.clone(),
                )));
                self.fields.push(FieldWriterEntry::Float(
                    Arc::clone(&writer),
                    field_info.clone(),
                ));
                Ok(FieldVectorWriter::Float(Box::new(SharedFieldWriter {
                    inner: writer,
                })))
            }
            VectorEncoding::BYTE => {
                let writer = Arc::new(Mutex::new(DefaultFlatFieldVectorsWriter::<Vec<u8>>::new(
                    field_info.clone(),
                )));
                self.fields.push(FieldWriterEntry::Byte(
                    Arc::clone(&writer),
                    field_info.clone(),
                ));
                Ok(FieldVectorWriter::Byte(Box::new(SharedFieldWriter {
                    inner: writer,
                })))
            }
        }
    }

    fn flush(&mut self, max_doc: i32, _sort_map: Option<&SorterDocMap>) -> Result<()> {
        let entries = std::mem::take(&mut self.fields);
        for entry in entries {
            match entry {
                FieldWriterEntry::Float(writer, info) => {
                    let mut guard = writer.lock().map_err(|_| {
                        LuceneError::IllegalState(
                            "FlatFieldVectorsWriter mutex was poisoned".to_string(),
                        )
                    })?;
                    self.write_field_float(&mut guard, &info, max_doc)?;
                    guard.finish()?;
                }
                FieldWriterEntry::Byte(writer, info) => {
                    let mut guard = writer.lock().map_err(|_| {
                        LuceneError::IllegalState(
                            "FlatFieldVectorsWriter mutex was poisoned".to_string(),
                        )
                    })?;
                    self.write_field_byte(&mut guard, &info, max_doc)?;
                    guard.finish()?;
                }
            }
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        if self.finished {
            return Err(LuceneError::IllegalState(
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

    fn merge_one_field(
        &mut self,
        field_info: &FieldInfo,
        merge_state: &MergeState,
    ) -> Result<Option<Box<dyn crate::codecs::knn_vectors::IORunnable>>> {
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

impl BufferingKnnVectorsWriter for Lucene99FlatVectorsWriter {
    fn write_field_float(
        &mut self,
        field_info: &FieldInfo,
        values: &dyn FloatVectorValues,
        max_doc: i32,
    ) -> Result<()> {
        let mut writer = DefaultFlatFieldVectorsWriter::<Vec<f32>>::new(field_info.clone());
        let mut iter = values.iterator()?;
        while iter.next_doc()? != NO_MORE_DOCS {
            let ord = iter.index();
            let vector = values.vector_value(ord)?;
            writer.add_value(iter.doc_id(), vector)?;
        }
        self.write_field_float(&mut writer, field_info, max_doc)?;
        writer.finish()?;
        Ok(())
    }

    fn write_field_byte(
        &mut self,
        field_info: &FieldInfo,
        values: &dyn ByteVectorValues,
        max_doc: i32,
    ) -> Result<()> {
        let mut writer = DefaultFlatFieldVectorsWriter::<Vec<u8>>::new(field_info.clone());
        let mut iter = values.iterator()?;
        while iter.next_doc()? != NO_MORE_DOCS {
            let ord = iter.index();
            let vector = values.vector_value(ord)?;
            writer.add_value(iter.doc_id(), vector)?;
        }
        self.write_field_byte(&mut writer, field_info, max_doc)?;
        writer.finish()?;
        Ok(())
    }
}

impl FlatVectorsWriter for Lucene99FlatVectorsWriter {
    fn vectors_scorer(&self) -> &dyn FlatVectorsScorer {
        &self.vector_scorer
    }

    fn merge_one_flat_vector_field(
        &mut self,
        field_info: &FieldInfo,
        merge_state: &MergeState,
    ) -> Result<()> {
        let vector_data = self.vector_data.as_mut().ok_or_else(|| {
            LuceneError::IllegalState("Lucene99FlatVectorsWriter is already closed".to_string())
        })?;
        let vector_data_offset = align_output(vector_data.as_mut(), field_info.vector_encoding)?;
        let mut docs_with_field = DocsWithFieldSet::new();

        let mut doc_base = 0i32;
        for (reader_idx, reader_opt) in merge_state.knn_vectors_readers.iter().enumerate() {
            let segment_max_doc = merge_state.max_docs.get(reader_idx).copied().unwrap_or(0);
            if let Some(reader) = reader_opt {
                match field_info.vector_encoding {
                    VectorEncoding::FLOAT32 => {
                        let values = reader.get_float_vector_values(&field_info.name)?;
                        let mut iter = values.iterator()?;
                        while iter.next_doc()? != NO_MORE_DOCS {
                            let mapped = iter.doc_id().checked_add(doc_base).ok_or_else(|| {
                                LuceneError::IllegalState("doc id overflow".to_string())
                            })?;
                            let vector = values.vector_value(iter.index())?;
                            vector_data.write_floats(&vector, 0, vector.len())?;
                            docs_with_field.add(mapped)?;
                        }
                    }
                    VectorEncoding::BYTE => {
                        let values = reader.get_byte_vector_values(&field_info.name)?;
                        let mut iter = values.iterator()?;
                        while iter.next_doc()? != NO_MORE_DOCS {
                            let mapped = iter.doc_id().checked_add(doc_base).ok_or_else(|| {
                                LuceneError::IllegalState("doc id overflow".to_string())
                            })?;
                            let vector = values.vector_value(iter.index())?;
                            vector_data.write_bytes(&vector, 0, vector.len())?;
                            docs_with_field.add(mapped)?;
                        }
                    }
                }
            }
            doc_base += segment_max_doc;
        }

        let vector_data_length = vector_data.file_pointer() - vector_data_offset;
        let meta = self.meta.as_mut().ok_or_else(|| {
            LuceneError::IllegalState("Lucene99FlatVectorsWriter is already closed".to_string())
        })?;
        // Java reads `segmentWriteState.segmentInfo.maxDoc()` here, which
        // raises `IllegalStateException("maxDoc isn't set yet")` when it was
        // never set; this reproduces that error at the same place.
        let segment_max_doc = self
            .segment_max_doc
            .ok_or_else(|| LuceneError::IllegalState("maxDoc isn't set yet".to_string()))?;
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

fn close_outputs(
    meta: &mut Option<Box<dyn IndexOutput>>,
    vector_data: &mut Option<Box<dyn IndexOutput>>,
) -> Result<()> {
    if let Some(mut m) = meta.take() {
        m.close()?;
    }
    if let Some(mut v) = vector_data.take() {
        v.close()?;
    }
    Ok(())
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
    vector_scorer: DefaultFlatVectorScorer,
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
    /// Creates a new reader for the given segment read state.
    pub fn new(
        state: &SegmentReadState<'_>,
        vectors_scorer: DefaultFlatVectorScorer,
    ) -> Result<Self> {
        let meta_file_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            META_EXTENSION,
        );
        let mut meta_checksum = state.directory.open_checksum_input(&meta_file_name)?;
        let version_meta = check_index_header(
            meta_checksum.as_mut(),
            META_CODEC_NAME,
            VERSION_START,
            VERSION_CURRENT,
            &state.segment_info.id(),
            &state.segment_suffix,
        )?;
        let fields = read_fields(meta_checksum.as_mut(), state.field_infos)?;
        check_footer(meta_checksum.as_mut())?;

        let vector_data_file_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            VECTOR_DATA_EXTENSION,
        );
        let mut vector_data = state
            .directory
            .open_input(&vector_data_file_name, state.context)?;
        let version_vector_data = check_index_header(
            vector_data.as_mut(),
            VECTOR_DATA_CODEC_NAME,
            VERSION_START,
            VERSION_CURRENT,
            &state.segment_info.id(),
            &state.segment_suffix,
        )?;
        if version_meta != version_vector_data {
            return Err(LuceneError::CorruptIndex(format!(
                "format versions mismatch: meta={version_meta}, vectorData={version_vector_data}"
            )));
        }
        retrieve_checksum(vector_data.as_mut())?;

        Ok(Self {
            fields,
            vector_scorer: vectors_scorer,
            vector_data,
            field_infos: state.field_infos.clone(),
            data_context: state.context.with_hints(state.context.hints()),
        })
    }

    fn clone_reader(&self) -> Result<Self> {
        Ok(Self {
            fields: self.fields.clone(),
            vector_scorer: self.vector_scorer,
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

    fn get_float_vector_values(&self, field: &str) -> Result<Box<dyn FloatVectorValues>> {
        let entry = self.get_field_entry(field, VectorEncoding::FLOAT32)?;
        OffHeapFloatVectorValues::load(
            &entry.ord_to_doc,
            entry.dimension,
            entry.vector_data_offset,
            entry.vector_data_length,
            self.vector_data.as_ref(),
        )
    }

    fn get_byte_vector_values(&self, field: &str) -> Result<Box<dyn ByteVectorValues>> {
        let entry = self.get_field_entry(field, VectorEncoding::BYTE)?;
        OffHeapByteVectorValues::load(
            &entry.ord_to_doc,
            entry.dimension,
            entry.vector_data_offset,
            entry.vector_data_length,
            self.vector_data.as_ref(),
        )
    }

    fn search(
        &self,
        _field: &str,
        _target: &[f32],
        _knn_collector: &mut dyn crate::codecs::knn_vectors::KnnCollector,
        _accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        Ok(())
    }

    fn search_byte(
        &self,
        _field: &str,
        _target: &[u8],
        _knn_collector: &mut dyn crate::codecs::knn_vectors::KnnCollector,
        _accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        Ok(())
    }

    fn get_merge_instance(&self) -> Result<Box<dyn KnnVectorsReader>> {
        Ok(Box::new(self.clone_reader()?))
    }

    fn close(&mut self) -> Result<()> {
        self.vector_data.close()
    }

    fn get_off_heap_byte_size(&self, field_info: &FieldInfo) -> HashMap<String, i64> {
        let mut map = HashMap::new();
        if let Ok(entry) = self.get_field_entry_or_throw(&field_info.name) {
            map.insert(VECTOR_DATA_EXTENSION.to_string(), entry.vector_data_length);
        }
        map
    }
}

impl FlatVectorsReader for Lucene99FlatVectorsReader {
    fn get_flat_vector_scorer(&self, _field: &str) -> Result<Box<dyn FlatVectorsScorer>> {
        Ok(Box::new(self.vector_scorer))
    }

    fn get_random_vector_scorer_float(
        &self,
        field: &str,
        target: &[f32],
    ) -> Result<Box<dyn RandomVectorScorer>> {
        let entry = self.get_field_entry(field, VectorEncoding::FLOAT32)?;
        let values = OffHeapFloatVectorValues::load(
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
            &entry.ord_to_doc,
            entry.dimension,
            entry.vector_data_offset,
            entry.vector_data_length,
            self.vector_data.as_ref(),
        )?;
        self.vector_scorer
            .get_random_vector_scorer_byte(entry.similarity_function, values, target)
    }

    fn get_merge_instance_flat(&self) -> Result<Box<dyn FlatVectorsReader>> {
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

struct OffHeapFloatVectorValues {
    dimension: i32,
    size: i32,
    byte_size: i32,
    slice: Mutex<Box<dyn IndexInput>>,
    config: Option<OrdToDocDISIReaderConfiguration>,
    data_input: Option<Box<dyn IndexInput>>,
    ord_to_doc_data: Vec<u8>,
}

impl OffHeapFloatVectorValues {
    fn load(
        config: &OrdToDocDISIReaderConfiguration,
        dimension: i32,
        vector_data_offset: i64,
        vector_data_length: i64,
        vector_data: &dyn IndexInput,
    ) -> Result<Box<dyn FloatVectorValues>> {
        if config.is_empty() {
            return Ok(Box::new(EmptyFloatVectorValues));
        }
        let byte_size = dimension * VectorEncoding::FLOAT32.byte_size();
        let slice =
            Mutex::new(vector_data.slice("vector-data", vector_data_offset, vector_data_length)?);
        let ord_to_doc_data = read_ord_to_doc_data(vector_data, config)?;
        if config.is_dense() {
            Ok(Box::new(Self {
                dimension,
                size: config.size,
                byte_size,
                slice,
                config: None,
                data_input: None,
                ord_to_doc_data,
            }))
        } else {
            Ok(Box::new(Self {
                dimension,
                size: config.size,
                byte_size,
                slice,
                config: Some(config.clone()),
                data_input: Some(vector_data.clone_input()?),
                ord_to_doc_data,
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
                    LuceneError::IllegalState(
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
                ord_to_doc_data: self.ord_to_doc_data.clone(),
            }),
            _ => Ok(Self {
                dimension: self.dimension,
                size: self.size,
                byte_size: self.byte_size,
                slice,
                config: None,
                data_input: None,
                ord_to_doc_data: Vec::new(),
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
        match &self.config {
            Some(config) => {
                let reader =
                    DirectMonotonicReader::new(config.meta.clone(), self.ord_to_doc_data.clone())
                        .expect("INVARIANT: ord-to-doc data was validated at load time");
                reader.get(ord as i64) as i32
            }
            None => ord,
        }
    }

    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        Ok(Box::new(self.clone_values()?))
    }

    fn encoding(&self) -> VectorEncoding {
        VectorEncoding::FLOAT32
    }

    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
        match &self.config {
            Some(config) => {
                let data_input = self
                    .data_input
                    .as_ref()
                    .expect("INVARIANT: sparse values have a data input");
                let disi = IndexedDISI::new(
                    data_input.as_ref(),
                    config.docs_with_field_offset,
                    config.docs_with_field_length,
                    config.jump_table_entry_count as i32,
                    config.dense_rank_power,
                    config.size as i64,
                )?;
                Ok(Box::new(disi))
            }
            None => Ok(Box::new(DenseDocIndexIterator::new(self.size))),
        }
    }
}

impl FloatVectorValues for OffHeapFloatVectorValues {
    fn copy_float(&self) -> Result<Box<dyn FloatVectorValues>> {
        Ok(Box::new(self.clone_values()?))
    }

    fn vector_value(&self, ord: i32) -> Result<Vec<f32>> {
        let mut value = vec![0.0f32; self.dimension as usize];
        let mut slice = self.slice.lock().map_err(|_| {
            LuceneError::IllegalState("off-heap vector slice mutex was poisoned".to_string())
        })?;
        slice.seek((ord as i64) * self.byte_size as i64)?;
        slice.read_floats(&mut value, 0, self.dimension as usize)?;
        Ok(value)
    }
}

struct OffHeapByteVectorValues {
    dimension: i32,
    size: i32,
    byte_size: i32,
    slice: Mutex<Box<dyn IndexInput>>,
    config: Option<OrdToDocDISIReaderConfiguration>,
    data_input: Option<Box<dyn IndexInput>>,
    ord_to_doc_data: Vec<u8>,
}

impl OffHeapByteVectorValues {
    fn load(
        config: &OrdToDocDISIReaderConfiguration,
        dimension: i32,
        vector_data_offset: i64,
        vector_data_length: i64,
        vector_data: &dyn IndexInput,
    ) -> Result<Box<dyn ByteVectorValues>> {
        if config.is_empty() {
            return Ok(Box::new(EmptyByteVectorValues));
        }
        let byte_size = dimension * VectorEncoding::BYTE.byte_size();
        let slice =
            Mutex::new(vector_data.slice("vector-data", vector_data_offset, vector_data_length)?);
        let ord_to_doc_data = read_ord_to_doc_data(vector_data, config)?;
        if config.is_dense() {
            Ok(Box::new(Self {
                dimension,
                size: config.size,
                byte_size,
                slice,
                config: None,
                data_input: None,
                ord_to_doc_data,
            }))
        } else {
            Ok(Box::new(Self {
                dimension,
                size: config.size,
                byte_size,
                slice,
                config: Some(config.clone()),
                data_input: Some(vector_data.clone_input()?),
                ord_to_doc_data,
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
                    LuceneError::IllegalState(
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
                ord_to_doc_data: self.ord_to_doc_data.clone(),
            }),
            _ => Ok(Self {
                dimension: self.dimension,
                size: self.size,
                byte_size: self.byte_size,
                slice,
                config: None,
                data_input: None,
                ord_to_doc_data: Vec::new(),
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
        match &self.config {
            Some(config) => {
                let reader =
                    DirectMonotonicReader::new(config.meta.clone(), self.ord_to_doc_data.clone())
                        .expect("INVARIANT: ord-to-doc data was validated at load time");
                reader.get(ord as i64) as i32
            }
            None => ord,
        }
    }

    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        Ok(Box::new(self.clone_values()?))
    }

    fn encoding(&self) -> VectorEncoding {
        VectorEncoding::BYTE
    }

    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
        match &self.config {
            Some(config) => {
                let data_input = self
                    .data_input
                    .as_ref()
                    .expect("INVARIANT: sparse values have a data input");
                let disi = IndexedDISI::new(
                    data_input.as_ref(),
                    config.docs_with_field_offset,
                    config.docs_with_field_length,
                    config.jump_table_entry_count as i32,
                    config.dense_rank_power,
                    config.size as i64,
                )?;
                Ok(Box::new(disi))
            }
            None => Ok(Box::new(DenseDocIndexIterator::new(self.size))),
        }
    }
}

impl ByteVectorValues for OffHeapByteVectorValues {
    fn copy_byte(&self) -> Result<Box<dyn ByteVectorValues>> {
        Ok(Box::new(self.clone_values()?))
    }

    fn vector_value(&self, ord: i32) -> Result<Vec<u8>> {
        let mut value = vec![0u8; self.dimension as usize];
        let mut slice = self.slice.lock().map_err(|_| {
            LuceneError::IllegalState("off-heap vector slice mutex was poisoned".to_string())
        })?;
        slice.seek((ord as i64) * self.byte_size as i64)?;
        slice.read_bytes(&mut value, 0, self.dimension as usize)?;
        Ok(value)
    }
}

fn read_ord_to_doc_data(
    vector_data: &dyn IndexInput,
    config: &OrdToDocDISIReaderConfiguration,
) -> Result<Vec<u8>> {
    if config.addresses_length <= 0 {
        return Ok(Vec::new());
    }
    let mut random_access =
        vector_data.random_access_slice(config.addresses_offset, config.addresses_length)?;
    let mut data = vec![0u8; config.addresses_length as usize];
    let len = data.len();
    random_access.read_bytes_at(0, &mut data, 0, len)?;
    Ok(data)
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    /// The four serialization sites for `VectorEncoding` and
    /// `VectorSimilarityFunction` each carry their own ordinal table. Java has
    /// one, `Enum.ordinal()`, so the tables must agree with the declaration
    /// order of the Rust enums and therefore with each other; a divergence here
    /// silently writes an unreadable index.
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
    use crate::store::{DefaultIOContext, RamDirectory};
    use crate::util::default_info_stream;
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

    fn make_states<'a>(
        dir: &'a Arc<dyn crate::store::Directory>,
        seg_info: &'a SegmentInfo,
        field_infos: &'a FieldInfos,
        seg_updates: &'a BufferedUpdates,
        io_ctx: &'a DefaultIOContext,
    ) -> (SegmentWriteState<'a>, SegmentReadState<'a>) {
        let info_stream = default_info_stream();
        let write_state = SegmentWriteState::new(
            info_stream,
            dir.as_ref(),
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
            HashMap::new(),
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
            HashMap::new(),
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
        let dir = Arc::new(RamDirectory::new()) as Arc<dyn crate::store::Directory>;
        let seg_info = make_segment_info(&dir, 3)?;
        let field_info = float_field_info("float_field", 0, 2)?;
        let field_infos = FieldInfos::new(vec![field_info.clone()])?;
        let seg_updates = BufferedUpdates;
        let io_ctx = DefaultIOContext::default();
        let (write_state, read_state) =
            make_states(&dir, &seg_info, &field_infos, &seg_updates, &io_ctx);

        let format = Lucene99FlatVectorsFormat::new(DefaultFlatVectorScorer::INSTANCE);
        {
            let mut writer = format.fields_writer_flat(&write_state)?;
            let field_writer = writer.add_field(&field_info)?;
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
        let values = reader.get_float_vector_values("float_field")?;
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
        let dir = Arc::new(RamDirectory::new()) as Arc<dyn crate::store::Directory>;
        let seg_info = make_segment_info(&dir, 5)?;
        let field_info = byte_field_info("byte_field", 0, 2)?;
        let field_infos = FieldInfos::new(vec![field_info.clone()])?;
        let seg_updates = BufferedUpdates;
        let io_ctx = DefaultIOContext::default();
        let (write_state, read_state) =
            make_states(&dir, &seg_info, &field_infos, &seg_updates, &io_ctx);

        let format = Lucene99FlatVectorsFormat::new(DefaultFlatVectorScorer::INSTANCE);
        {
            let mut writer = format.fields_writer_flat(&write_state)?;
            let field_writer = writer.add_field(&field_info)?;
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
        let values = reader.get_byte_vector_values("byte_field")?;
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
        let dir = Arc::new(RamDirectory::new()) as Arc<dyn crate::store::Directory>;
        let seg_info = make_segment_info(&dir, 1)?;
        let field_info = float_field_info("f", 0, 1)?;
        let field_infos = FieldInfos::new(vec![field_info.clone()])?;
        let seg_updates = BufferedUpdates;
        let io_ctx = DefaultIOContext::default();
        let (write_state, _read_state) =
            make_states(&dir, &seg_info, &field_infos, &seg_updates, &io_ctx);

        let format = Lucene99FlatVectorsFormat::new(DefaultFlatVectorScorer::INSTANCE);
        {
            let mut writer = format.fields_writer_flat(&write_state)?;
            let field_writer = writer.add_field(&field_info)?;
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
