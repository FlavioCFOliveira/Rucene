//! Lucene 9.9 HNSW vector format.
//!
//! Equivalent to `org.apache.lucene.codecs.lucene99.Lucene99HnswVectorsFormat`,
//! `Lucene99HnswVectorsReader`, and `Lucene99HnswVectorsWriter`.
//!
//! This format stores an approximate nearest-neighbor graph for each vector
//! field in a `.vex` file and per-field metadata in a `.vem` file. The actual
//! vector values are delegated to a [`Lucene99FlatVectorsWriter`] / reader.

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
    FieldVectorWriter, KnnFieldVectorsWriter, KnnVectorsFormat, KnnVectorsReader, KnnVectorsWriter,
    SorterDocMap,
};
use crate::codecs::lucene99::flat_vectors_format::Lucene99FlatVectorsFormat;
use crate::codecs::postings::MergeState;
use crate::codecs::state::{SegmentReadState, SegmentWriteState};
use crate::codecs::stub::FieldInfo;
use crate::error::{LuceneError, Result};
use crate::index::vector_values::{
    ByteVectorValues, DocIndexIterator, FloatVectorValues, KnnVectorValues,
};
use crate::index::{segment_file_name, FieldInfos, VectorEncoding, VectorSimilarityFunction};
use crate::search::knn::{KnnCollector, TopKnnCollector};
use crate::search::{AcceptDocs, DocIdSetIterator, NO_MORE_DOCS};
use crate::store::{DataInput, IndexInput, IndexOutput};
use crate::util::extra::LongValues;
use crate::util::hnsw::{
    ArrayNodesIterator, DenseNodesIterator, HnswBuilder, HnswGraph, HnswGraphBuilder,
    HnswGraphSearcher, NeighborArray, NeighborQueue, NodesIterator, OnHeapHnswGraph,
    RandomVectorScorer,
};
use crate::util::packed::{DirectMonotonicMeta, DirectMonotonicReader, DirectMonotonicWriter};
use crate::util::FixedBitSet;

// -----------------------------------------------------------------------------
// Format constants
// -----------------------------------------------------------------------------

const NAME: &str = "Lucene99HnswVectorsFormat";
const META_CODEC_NAME: &str = "Lucene99HnswVectorsFormatMeta";
const VECTOR_INDEX_CODEC_NAME: &str = "Lucene99HnswVectorsFormatIndex";
const META_EXTENSION: &str = "vem";
const VECTOR_INDEX_EXTENSION: &str = "vex";
const VERSION_START: i32 = 0;
const VERSION_GROUPVARINT: i32 = 1;
const VERSION_CURRENT: i32 = VERSION_GROUPVARINT;

const MAXIMUM_MAX_CONN: i32 = 512;
const MAXIMUM_BEAM_WIDTH: i32 = 3200;
const DEFAULT_MAX_CONN: i32 = crate::util::hnsw::DEFAULT_M;
const DEFAULT_BEAM_WIDTH: i32 = crate::util::hnsw::DEFAULT_BEAM_WIDTH;
const DEFAULT_NUM_MERGE_WORKER: i32 = 1;
const HNSW_GRAPH_THRESHOLD: i32 = 100;
const DIRECT_MONOTONIC_BLOCK_SHIFT: i32 = 16;
const EXHAUSTIVE_BULK_SCORE_ORDS: usize = 64;

// -----------------------------------------------------------------------------
// Global format registration
// -----------------------------------------------------------------------------

static LUCENE99_HNSW_VECTORS_FORMAT_REGISTERED: OnceLock<()> = OnceLock::new();

fn ensure_registered() {
    LUCENE99_HNSW_VECTORS_FORMAT_REGISTERED.get_or_init(|| {
        let _ = crate::codecs::knn_vectors::register_global_knn_vectors_format(
            NAME,
            Lucene99HnswVectorsFormat::new(),
        );
    });
}

// -----------------------------------------------------------------------------
// Lucene99HnswVectorsFormat
// -----------------------------------------------------------------------------

/// Lucene 9.9 HNSW vector format.
///
/// Equivalent to `org.apache.lucene.codecs.lucene99.Lucene99HnswVectorsFormat`.
#[derive(Debug, Clone, Copy)]
pub struct Lucene99HnswVectorsFormat {
    max_conn: i32,
    beam_width: i32,
    num_merge_workers: i32,
    tiny_segments_threshold: i32,
    write_version: i32,
}

impl Lucene99HnswVectorsFormat {
    /// Creates a format using the default graph construction parameters.
    pub fn new() -> Self {
        Self::with_params(
            DEFAULT_MAX_CONN,
            DEFAULT_BEAM_WIDTH,
            DEFAULT_NUM_MERGE_WORKER,
            HNSW_GRAPH_THRESHOLD,
            VERSION_CURRENT,
        )
    }

    /// Creates a format with the given `max_conn` and `beam_width`.
    pub fn with_max_conn_beam_width(max_conn: i32, beam_width: i32) -> Self {
        Self::with_params(
            max_conn,
            beam_width,
            DEFAULT_NUM_MERGE_WORKER,
            HNSW_GRAPH_THRESHOLD,
            VERSION_CURRENT,
        )
    }

    /// Creates a format with the given graph construction parameters.
    pub fn with_params(
        max_conn: i32,
        beam_width: i32,
        num_merge_workers: i32,
        tiny_segments_threshold: i32,
        write_version: i32,
    ) -> Self {
        if max_conn <= 0 || max_conn > MAXIMUM_MAX_CONN {
            panic!("maxConn must be positive and <= {MAXIMUM_MAX_CONN}; maxConn={max_conn}");
        }
        if beam_width <= 0 || beam_width > MAXIMUM_BEAM_WIDTH {
            panic!(
                "beamWidth must be positive and <= {MAXIMUM_BEAM_WIDTH}; beamWidth={beam_width}"
            );
        }
        Self {
            max_conn,
            beam_width,
            num_merge_workers,
            tiny_segments_threshold,
            write_version,
        }
    }

    fn flat_format(&self) -> Lucene99FlatVectorsFormat {
        Lucene99FlatVectorsFormat::new(DefaultFlatVectorScorer::INSTANCE)
    }
}

impl Default for Lucene99HnswVectorsFormat {
    fn default() -> Self {
        Self::new()
    }
}

impl KnnVectorsFormat for Lucene99HnswVectorsFormat {
    fn name(&self) -> &str {
        NAME
    }

    fn fields_writer<'a>(
        &self,
        state: &SegmentWriteState<'a>,
    ) -> Result<Box<dyn KnnVectorsWriter + 'a>> {
        ensure_registered();
        Ok(Box::new(Lucene99HnswVectorsWriter::new(
            state,
            self.max_conn,
            self.beam_width,
            self.num_merge_workers,
            self.tiny_segments_threshold,
            self.write_version,
        )?))
    }

    fn fields_reader<'a>(
        &self,
        state: &SegmentReadState<'a>,
    ) -> Result<Box<dyn KnnVectorsReader + 'a>> {
        ensure_registered();
        let flat_reader = self.flat_format().fields_reader_flat(state)?;
        Ok(Box::new(Lucene99HnswVectorsReader::new(
            state,
            flat_reader,
        )?))
    }

    fn get_max_dimensions(&self, _field_name: &str) -> i32 {
        1024
    }
}

// -----------------------------------------------------------------------------
// Lucene99HnswVectorsWriter
// -----------------------------------------------------------------------------

/// Writer for the Lucene 9.9 HNSW vector format.
///
/// Equivalent to `org.apache.lucene.codecs.lucene99.Lucene99HnswVectorsWriter`.
pub struct Lucene99HnswVectorsWriter {
    meta: Option<Box<dyn IndexOutput>>,
    vector_index: Option<Box<dyn IndexOutput>>,
    flat_vector_writer: Box<dyn FlatVectorsWriter>,
    m: i32,
    beam_width: i32,
    #[allow(dead_code)]
    num_merge_workers: i32,
    tiny_segments_threshold: i32,
    version: i32,
    fields: Vec<FieldWriterEntry>,
    finished: bool,
}

impl fmt::Debug for Lucene99HnswVectorsWriter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Lucene99HnswVectorsWriter")
            .field("m", &self.m)
            .field("beam_width", &self.beam_width)
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
}

impl Lucene99HnswVectorsWriter {
    /// Creates a new HNSW vector writer for the given segment.
    pub fn new(
        state: &SegmentWriteState<'_>,
        m: i32,
        beam_width: i32,
        num_merge_workers: i32,
        tiny_segments_threshold: i32,
        version: i32,
    ) -> Result<Self> {
        let flat_vector_writer = Lucene99FlatVectorsFormat::new(DefaultFlatVectorScorer::INSTANCE)
            .fields_writer_flat(state)?;

        let meta_file_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            META_EXTENSION,
        );
        let vector_index_file_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            VECTOR_INDEX_EXTENSION,
        );

        let meta = state
            .directory
            .create_output(&meta_file_name, state.context)?;
        let vector_index = state
            .directory
            .create_output(&vector_index_file_name, state.context)?;

        let mut meta_opt = Some(meta);
        let mut vector_index_opt = Some(vector_index);
        let result = (|| -> Result<Self> {
            let meta = meta_opt.as_mut().ok_or_else(|| {
                LuceneError::IllegalState("meta output missing during writer creation".to_string())
            })?;
            let vector_index = vector_index_opt.as_mut().ok_or_else(|| {
                LuceneError::IllegalState(
                    "vector index output missing during writer creation".to_string(),
                )
            })?;
            write_index_header(
                meta.as_mut(),
                META_CODEC_NAME,
                version,
                &state.segment_info.id(),
                &state.segment_suffix,
            )?;
            write_index_header(
                vector_index.as_mut(),
                VECTOR_INDEX_CODEC_NAME,
                version,
                &state.segment_info.id(),
                &state.segment_suffix,
            )?;
            Ok(Self {
                meta: meta_opt.take(),
                vector_index: vector_index_opt.take(),
                flat_vector_writer,
                m,
                beam_width,
                num_merge_workers,
                tiny_segments_threshold,
                version,
                fields: Vec::new(),
                finished: false,
            })
        })();

        if result.is_err() {
            let _ = close_outputs(&mut meta_opt, &mut vector_index_opt);
        }
        result
    }

    fn write_field(
        &mut self,
        field_info: &FieldInfo,
        vector_index_offset: i64,
        vector_index_length: i64,
        count: i32,
        graph: Option<&OnHeapHnswGraph>,
        graph_level_node_offsets: &[Vec<i32>],
    ) -> Result<()> {
        let meta = self.meta.as_mut().ok_or_else(|| {
            LuceneError::IllegalState("Lucene99HnswVectorsWriter is already closed".to_string())
        })?;
        meta.write_int(field_info.number)?;
        meta.write_int(vector_encoding_ordinal(field_info.vector_encoding))?;
        meta.write_int(vector_similarity_ordinal(
            field_info.vector_similarity_function,
        ))?;
        meta.write_v_long(vector_index_offset)?;
        meta.write_v_long(vector_index_length)?;
        meta.write_v_int(field_info.vector_dimension)?;
        meta.write_int(count)?;
        meta.write_v_int(self.m)?;
        if let Some(graph) = graph {
            meta.write_v_int(graph.num_levels()?)?;
            let mut value_count = 0i64;
            for level in 0..graph.num_levels()? {
                let mut nodes_on_level = graph.get_nodes_on_level(level)?;
                value_count += nodes_on_level.size() as i64;
                if level > 0 {
                    let mut nol = vec![0i32; nodes_on_level.size() as usize];
                    let consumed = nodes_on_level.consume(&mut nol);
                    debug_assert_eq!(consumed, nol.len());
                    nol.sort_unstable();
                    meta.write_v_int(nol.len() as i32)?;
                    for i in (1..nol.len()).rev() {
                        nol[i] -= nol[i - 1];
                    }
                    for n in nol {
                        meta.write_v_int(n)?;
                    }
                }
            }

            let vector_index = self.vector_index.as_mut().ok_or_else(|| {
                LuceneError::IllegalState("Lucene99HnswVectorsWriter is already closed".to_string())
            })?;
            let start = vector_index.file_pointer();
            meta.write_long(start)?;
            meta.write_v_int(DIRECT_MONOTONIC_BLOCK_SHIFT)?;
            let mut writer = DirectMonotonicWriter::new(
                meta.as_mut(),
                vector_index.as_mut(),
                value_count,
                DIRECT_MONOTONIC_BLOCK_SHIFT,
            )?;
            let mut cumulative = 0i64;
            for level_offsets in graph_level_node_offsets {
                for &v in level_offsets {
                    writer.add(cumulative)?;
                    cumulative += v as i64;
                }
            }
            writer.finish()?;
            meta.write_long(vector_index.file_pointer() - start)?;
        } else {
            meta.write_v_int(0)?;
        }
        Ok(())
    }

    fn write_graph(&mut self, graph: &OnHeapHnswGraph) -> Result<Vec<Vec<i32>>> {
        let scratch_capacity = (graph.max_conn() * 2).max(4) as usize;
        let mut offsets = Vec::with_capacity(graph.num_levels()? as usize);
        for level in 0..graph.num_levels()? {
            let mut sorted_nodes = graph.get_sorted_nodes_on_level(level)?;
            let mut level_offsets = Vec::with_capacity(sorted_nodes.size() as usize);
            while sorted_nodes.has_next() {
                let node = sorted_nodes.next_int();
                let neighbors = graph.get_neighbors(level, node)?;
                let vector_index = self.vector_index.as_mut().ok_or_else(|| {
                    LuceneError::IllegalState(
                        "Lucene99HnswVectorsWriter is already closed".to_string(),
                    )
                })?;
                let offset_start = vector_index.file_pointer();
                write_neighbors(
                    vector_index.as_mut(),
                    neighbors,
                    scratch_capacity,
                    self.version,
                )?;
                level_offsets.push((vector_index.file_pointer() - offset_start) as i32);
            }
            offsets.push(level_offsets);
        }
        Ok(offsets)
    }
}

impl KnnVectorsWriter for Lucene99HnswVectorsWriter {
    fn add_field(&mut self, field_info: &FieldInfo) -> Result<FieldVectorWriter> {
        if self.finished {
            return Err(LuceneError::IllegalState(
                "Lucene99HnswVectorsWriter is already finished".to_string(),
            ));
        }
        match field_info.vector_encoding {
            VectorEncoding::FLOAT32 => {
                let writer = Arc::new(Mutex::new(DefaultFlatFieldVectorsWriter::<Vec<f32>>::new()));
                self.fields.push(FieldWriterEntry::Float(
                    Arc::clone(&writer),
                    field_info.clone(),
                ));
                Ok(FieldVectorWriter::Float(Box::new(SharedFieldWriter {
                    inner: writer,
                })))
            }
            VectorEncoding::BYTE => {
                let writer = Arc::new(Mutex::new(DefaultFlatFieldVectorsWriter::<Vec<u8>>::new()));
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

    fn flush(&mut self, max_doc: i32, sort_map: Option<&SorterDocMap>) -> Result<()> {
        self.flat_vector_writer.flush(max_doc, sort_map)?;
        let entries = std::mem::take(&mut self.fields);
        for entry in entries {
            match entry {
                FieldWriterEntry::Float(writer, info) => {
                    let values =
                        BufferedFloatVectorValues::new(info.vector_dimension, Arc::clone(&writer))?;
                    self.flat_vector_writer
                        .write_field_float(&info, &values, max_doc)?;
                    let count = values.size();
                    let graph = build_hnsw_graph_float(
                        &values,
                        info.vector_similarity_function,
                        self.m,
                        self.beam_width,
                        self.tiny_segments_threshold,
                    )?;
                    let vector_index_offset = self
                        .vector_index
                        .as_ref()
                        .ok_or_else(|| {
                            LuceneError::IllegalState(
                                "Lucene99HnswVectorsWriter is already closed".to_string(),
                            )
                        })?
                        .file_pointer();
                    let graph_level_offsets = if let Some(ref g) = graph {
                        self.write_graph(g)?
                    } else {
                        Vec::new()
                    };
                    let vector_index_length =
                        self.vector_index.as_ref().unwrap().file_pointer() - vector_index_offset;
                    self.write_field(
                        &info,
                        vector_index_offset,
                        vector_index_length,
                        count,
                        graph.as_ref(),
                        &graph_level_offsets,
                    )?;
                }
                FieldWriterEntry::Byte(writer, info) => {
                    let values =
                        BufferedByteVectorValues::new(info.vector_dimension, Arc::clone(&writer))?;
                    self.flat_vector_writer
                        .write_field_byte(&info, &values, max_doc)?;
                    let count = values.size();
                    let graph = build_hnsw_graph_byte(
                        &values,
                        info.vector_similarity_function,
                        self.m,
                        self.beam_width,
                        self.tiny_segments_threshold,
                    )?;
                    let vector_index_offset = self
                        .vector_index
                        .as_ref()
                        .ok_or_else(|| {
                            LuceneError::IllegalState(
                                "Lucene99HnswVectorsWriter is already closed".to_string(),
                            )
                        })?
                        .file_pointer();
                    let graph_level_offsets = if let Some(ref g) = graph {
                        self.write_graph(g)?
                    } else {
                        Vec::new()
                    };
                    let vector_index_length =
                        self.vector_index.as_ref().unwrap().file_pointer() - vector_index_offset;
                    self.write_field(
                        &info,
                        vector_index_offset,
                        vector_index_length,
                        count,
                        graph.as_ref(),
                        &graph_level_offsets,
                    )?;
                }
            }
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        if self.finished {
            return Err(LuceneError::IllegalState(
                "Lucene99HnswVectorsWriter is already finished".to_string(),
            ));
        }
        self.finished = true;
        self.flat_vector_writer.finish()?;
        if let Some(meta) = self.meta.as_mut() {
            meta.write_int(-1)?;
            write_footer(meta.as_mut())?;
        }
        if let Some(vector_index) = self.vector_index.as_mut() {
            write_footer(vector_index.as_mut())?;
        }
        Ok(())
    }

    fn merge_one_field(
        &mut self,
        field_info: &FieldInfo,
        merge_state: &MergeState,
    ) -> Result<Option<Box<dyn crate::codecs::knn_vectors::IORunnable>>> {
        self.flat_vector_writer
            .merge_one_flat_vector_field(field_info, merge_state)?;
        Ok(None)
    }

    fn close(&mut self) -> Result<()> {
        let mut last_err: Option<LuceneError> = None;
        if let Err(e) = self.flat_vector_writer.close() {
            last_err = Some(e);
        }
        if let Some(meta) = self.meta.as_mut() {
            if let Err(e) = meta.close() {
                last_err = Some(e);
            }
        }
        if let Some(vector_index) = self.vector_index.as_mut() {
            if let Err(e) = vector_index.close() {
                last_err = Some(e);
            }
        }
        if let Some(e) = last_err {
            Err(e)
        } else {
            Ok(())
        }
    }
}

fn close_outputs(
    meta: &mut Option<Box<dyn IndexOutput>>,
    vector_index: &mut Option<Box<dyn IndexOutput>>,
) -> Result<()> {
    if let Some(mut m) = meta.take() {
        m.close()?;
    }
    if let Some(mut v) = vector_index.take() {
        v.close()?;
    }
    Ok(())
}

fn write_neighbors(
    out: &mut dyn IndexOutput,
    neighbors: &NeighborArray,
    scratch_capacity: usize,
    version: i32,
) -> Result<()> {
    let size = neighbors.size() as usize;
    let mut scratch = Vec::with_capacity(scratch_capacity.max(size));
    scratch.extend_from_slice(&neighbors.nodes()[..size]);
    scratch[..size].sort_unstable();
    let mut actual = Vec::with_capacity(size);
    for i in 0..size {
        let node = scratch[i];
        if i == 0 {
            actual.push(node);
        } else if scratch[i] != scratch[i - 1] {
            actual.push(node - scratch[i - 1]);
        }
    }
    out.write_v_int(actual.len() as i32)?;
    if version >= VERSION_GROUPVARINT {
        write_group_v_ints(out, &actual)?;
    } else {
        for v in actual {
            out.write_v_int(v)?;
        }
    }
    Ok(())
}

fn write_group_v_ints(out: &mut dyn IndexOutput, values: &[i32]) -> Result<()> {
    let mut i = 0;
    while i + 4 <= values.len() {
        let n1 = num_bytes(values[i]) - 1;
        let n2 = num_bytes(values[i + 1]) - 1;
        let n3 = num_bytes(values[i + 2]) - 1;
        let n4 = num_bytes(values[i + 3]) - 1;
        let flag = ((n1 as u8) << 6) | ((n2 as u8) << 4) | ((n3 as u8) << 2) | (n4 as u8);
        out.write_byte(flag)?;
        write_int_le_bytes(out, values[i], n1 + 1)?;
        write_int_le_bytes(out, values[i + 1], n2 + 1)?;
        write_int_le_bytes(out, values[i + 2], n3 + 1)?;
        write_int_le_bytes(out, values[i + 3], n4 + 1)?;
        i += 4;
    }
    for &v in &values[i..] {
        out.write_v_int(v)?;
    }
    Ok(())
}

fn num_bytes(v: i32) -> usize {
    // `| 1` ensures 0 encodes as a single byte.
    4 - ((v as u32 | 1).leading_zeros() >> 3) as usize
}

fn write_int_le_bytes(out: &mut dyn IndexOutput, v: i32, n: usize) -> Result<()> {
    let bytes = v.to_le_bytes();
    out.write_bytes(&bytes, 0, n)
}

fn build_hnsw_graph_float(
    values: &BufferedFloatVectorValues,
    similarity: VectorSimilarityFunction,
    m: i32,
    beam_width: i32,
    tiny_segments_threshold: i32,
) -> Result<Option<OnHeapHnswGraph>> {
    let count = values.size();
    if count > 0 && should_create_graph(tiny_segments_threshold, count) {
        let supplier = DefaultFlatVectorScorer::INSTANCE
            .get_random_vector_scorer_supplier_float(similarity, Box::new(values.clone()))?;
        let mut builder = HnswGraphBuilder::create(&*supplier, m, beam_width, 42, count)?;
        builder.build(count)?;
        Ok(Some(builder.into_graph()))
    } else {
        Ok(None)
    }
}

fn build_hnsw_graph_byte(
    values: &BufferedByteVectorValues,
    similarity: VectorSimilarityFunction,
    m: i32,
    beam_width: i32,
    tiny_segments_threshold: i32,
) -> Result<Option<OnHeapHnswGraph>> {
    let count = values.size();
    if count > 0 && should_create_graph(tiny_segments_threshold, count) {
        let supplier = DefaultFlatVectorScorer::INSTANCE
            .get_random_vector_scorer_supplier_byte(similarity, Box::new(values.clone()))?;
        let mut builder = HnswGraphBuilder::create(&*supplier, m, beam_width, 42, count)?;
        builder.build(count)?;
        Ok(Some(builder.into_graph()))
    } else {
        Ok(None)
    }
}

fn should_create_graph(k: i32, num_nodes: i32) -> bool {
    if k <= 0 {
        return true;
    }
    let expected = expected_visited_nodes(k, num_nodes);
    num_nodes > expected && expected > 0
}

fn expected_visited_nodes(k: i32, graph_size: i32) -> i32 {
    if graph_size <= 0 {
        0
    } else {
        ((graph_size as f64).ln() * k as f64) as i32
    }
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
// Buffered vector values used while building the HNSW graph
// -----------------------------------------------------------------------------

#[derive(Clone)]
struct BufferedFloatVectorValues {
    dimension: i32,
    vectors: Vec<Vec<f32>>,
    ord_to_doc: Vec<i32>,
}

impl BufferedFloatVectorValues {
    fn new(
        dimension: i32,
        writer: Arc<Mutex<DefaultFlatFieldVectorsWriter<Vec<f32>>>>,
    ) -> Result<Self> {
        let guard = writer.lock().map_err(|_| {
            LuceneError::IllegalState("FlatFieldVectorsWriter mutex was poisoned".to_string())
        })?;
        let vectors = guard.vectors().to_vec();
        let ord_to_doc = build_ord_to_doc(guard.docs_with_field_set())?;
        Ok(Self {
            dimension,
            vectors,
            ord_to_doc,
        })
    }
}

impl KnnVectorValues for BufferedFloatVectorValues {
    fn dimension(&self) -> i32 {
        self.dimension
    }

    fn size(&self) -> i32 {
        self.vectors.len() as i32
    }

    fn ord_to_doc(&self, ord: i32) -> i32 {
        self.ord_to_doc[ord as usize]
    }

    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        Ok(Box::new(self.clone()))
    }

    fn encoding(&self) -> VectorEncoding {
        VectorEncoding::FLOAT32
    }

    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
        Ok(Box::new(VecDocIndexIterator::new(self.ord_to_doc.clone())))
    }
}

impl FloatVectorValues for BufferedFloatVectorValues {
    fn vector_value(&self, ord: i32) -> Result<Vec<f32>> {
        self.vectors
            .get(ord as usize)
            .cloned()
            .ok_or_else(|| LuceneError::IllegalArgument(format!("ord {ord} out of range")))
    }
}

#[derive(Clone)]
struct BufferedByteVectorValues {
    dimension: i32,
    vectors: Vec<Vec<u8>>,
    ord_to_doc: Vec<i32>,
}

impl BufferedByteVectorValues {
    fn new(
        dimension: i32,
        writer: Arc<Mutex<DefaultFlatFieldVectorsWriter<Vec<u8>>>>,
    ) -> Result<Self> {
        let guard = writer.lock().map_err(|_| {
            LuceneError::IllegalState("FlatFieldVectorsWriter mutex was poisoned".to_string())
        })?;
        let vectors = guard.vectors().to_vec();
        let ord_to_doc = build_ord_to_doc(guard.docs_with_field_set())?;
        Ok(Self {
            dimension,
            vectors,
            ord_to_doc,
        })
    }
}

impl KnnVectorValues for BufferedByteVectorValues {
    fn dimension(&self) -> i32 {
        self.dimension
    }

    fn size(&self) -> i32 {
        self.vectors.len() as i32
    }

    fn ord_to_doc(&self, ord: i32) -> i32 {
        self.ord_to_doc[ord as usize]
    }

    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        Ok(Box::new(self.clone()))
    }

    fn encoding(&self) -> VectorEncoding {
        VectorEncoding::BYTE
    }

    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
        Ok(Box::new(VecDocIndexIterator::new(self.ord_to_doc.clone())))
    }
}

impl ByteVectorValues for BufferedByteVectorValues {
    fn vector_value(&self, ord: i32) -> Result<Vec<u8>> {
        self.vectors
            .get(ord as usize)
            .cloned()
            .ok_or_else(|| LuceneError::IllegalArgument(format!("ord {ord} out of range")))
    }
}

fn build_ord_to_doc(docs_with_field_set: &DocsWithFieldSet) -> Result<Vec<i32>> {
    let mut ord_to_doc = Vec::with_capacity(docs_with_field_set.cardinality() as usize);
    let mut iter = docs_with_field_set.iterator()?;
    while iter.next_doc()? != NO_MORE_DOCS {
        ord_to_doc.push(iter.doc_id());
    }
    Ok(ord_to_doc)
}

struct VecDocIndexIterator {
    docs: Vec<i32>,
    index: i32,
}

impl VecDocIndexIterator {
    fn new(docs: Vec<i32>) -> Self {
        Self { docs, index: -1 }
    }
}

impl DocIdSetIterator for VecDocIndexIterator {
    fn doc_id(&self) -> i32 {
        if self.index < 0 {
            -1
        } else if self.index as usize >= self.docs.len() {
            NO_MORE_DOCS
        } else {
            self.docs[self.index as usize]
        }
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.index += 1;
        if self.index as usize >= self.docs.len() {
            Ok(NO_MORE_DOCS)
        } else {
            Ok(self.docs[self.index as usize])
        }
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        while self.index as usize + 1 < self.docs.len()
            && self.docs[(self.index + 1) as usize] < target
        {
            self.index += 1;
        }
        self.next_doc()
    }

    fn cost(&self) -> i64 {
        self.docs.len() as i64
    }
}

impl DocIndexIterator for VecDocIndexIterator {
    fn index(&self) -> i32 {
        self.index
    }
}

// -----------------------------------------------------------------------------
// Lucene99HnswVectorsReader
// -----------------------------------------------------------------------------

/// Reader for the Lucene 9.9 HNSW vector format.
///
/// Equivalent to `org.apache.lucene.codecs.lucene99.Lucene99HnswVectorsReader`.
pub struct Lucene99HnswVectorsReader {
    flat_vectors_reader: Box<dyn FlatVectorsReader>,
    field_infos: FieldInfos,
    fields: HashMap<i32, FieldEntry>,
    vector_index: Box<dyn IndexInput>,
    version: i32,
    data_context: Box<dyn crate::store::IOContext>,
}

impl fmt::Debug for Lucene99HnswVectorsReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Lucene99HnswVectorsReader")
            .field("fields", &self.fields.len())
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
#[allow(dead_code)]
struct FieldEntry {
    field_info: FieldInfo,
    similarity_function: VectorSimilarityFunction,
    vector_encoding: VectorEncoding,
    vector_index_offset: i64,
    vector_index_length: i64,
    m: i32,
    num_levels: i32,
    dimension: i32,
    size: i32,
    nodes_by_level: Vec<Option<Vec<i32>>>,
    offsets_meta: DirectMonotonicMeta,
    offsets_offset: i64,
    offsets_block_shift: i32,
    offsets_length: i64,
}

impl Lucene99HnswVectorsReader {
    /// Creates a new reader for the given segment read state.
    pub fn new(
        state: &SegmentReadState<'_>,
        flat_vectors_reader: Box<dyn FlatVectorsReader>,
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

        let vector_index_file_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            VECTOR_INDEX_EXTENSION,
        );
        let mut vector_index = state
            .directory
            .open_input(&vector_index_file_name, state.context)?;
        let version_vector_index = check_index_header(
            vector_index.as_mut(),
            VECTOR_INDEX_CODEC_NAME,
            VERSION_START,
            VERSION_CURRENT,
            &state.segment_info.id(),
            &state.segment_suffix,
        )?;
        if version_meta != version_vector_index {
            return Err(LuceneError::CorruptIndex(format!(
                "format versions mismatch: meta={version_meta}, vectorIndex={version_vector_index}"
            )));
        }
        retrieve_checksum(vector_index.as_mut())?;

        Ok(Self {
            flat_vectors_reader,
            field_infos: state.field_infos.clone(),
            fields,
            vector_index,
            version: version_meta,
            data_context: state.context.with_hints(state.context.hints()),
        })
    }

    fn clone_reader(&self) -> Result<Self> {
        Ok(Self {
            flat_vectors_reader: self.flat_vectors_reader.get_merge_instance_flat()?,
            field_infos: self.field_infos.clone(),
            fields: self.fields.clone(),
            vector_index: self.vector_index.clone_input()?,
            version: self.version,
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

    fn search_with_scorer(
        &mut self,
        entry: &FieldEntry,
        mut scorer: Box<dyn RandomVectorScorer>,
        knn_collector: &mut dyn KnnCollector,
        _accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        if entry.size == 0 || knn_collector.k() == 0 {
            return Ok(());
        }
        let num_vectors = scorer.max_ord();
        let graph_size = if entry.vector_index_length > 0 {
            entry.size
        } else {
            0
        };
        let k = knn_collector.k();
        let visit_limit = knn_collector.visit_limit();
        let use_graph = graph_size > 0
            && k < num_vectors
            && expected_visited_nodes(k, graph_size) < num_vectors;
        let mut internal = TopKnnCollector::new(k, visit_limit);
        if use_graph {
            let mut graph = OffHeapHnswGraph::new(entry, self.vector_index.as_ref(), self.version)?;
            let mut searcher = HnswGraphSearcher::new(
                NeighborQueue::new(k, true),
                FixedBitSet::new(graph.max_node_id().max(0) as usize + 1),
            );
            searcher.search_graph(&mut internal, scorer.as_mut(), &mut graph, None)?;
        } else {
            let mut ords = Vec::with_capacity(EXHAUSTIVE_BULK_SCORE_ORDS);
            let mut scores = vec![0.0f32; EXHAUSTIVE_BULK_SCORE_ORDS];
            for i in 0..num_vectors {
                ords.push(i);
                if ords.len() == EXHAUSTIVE_BULK_SCORE_ORDS {
                    score_batch_exhaustive(&mut ords, &mut scores, &mut internal, scorer.as_mut())?;
                }
            }
            if !ords.is_empty() {
                score_batch_exhaustive(&mut ords, &mut scores, &mut internal, scorer.as_mut())?;
            }
        }
        knn_collector.inc_visited_count(internal.visited_count() as i32);
        for (ord, score) in internal.top_docs_with_scores() {
            knn_collector.collect(scorer.ord_to_doc(ord), score);
        }
        Ok(())
    }
}

impl KnnVectorsReader for Lucene99HnswVectorsReader {
    fn check_integrity(&self) -> Result<()> {
        self.flat_vectors_reader.check_integrity()?;
        let mut data = self.vector_index.clone_input()?;
        checksum_entire_file(data.as_mut())?;
        Ok(())
    }

    fn get_float_vector_values(&self, field: &str) -> Result<Box<dyn FloatVectorValues>> {
        self.flat_vectors_reader.get_float_vector_values(field)
    }

    fn get_byte_vector_values(&self, field: &str) -> Result<Box<dyn ByteVectorValues>> {
        self.flat_vectors_reader.get_byte_vector_values(field)
    }

    fn search(
        &mut self,
        field: &str,
        target: &[f32],
        knn_collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        let entry = self
            .get_field_entry(field, VectorEncoding::FLOAT32)?
            .clone();
        let scorer = self
            .flat_vectors_reader
            .get_random_vector_scorer_float(field, target)?;
        self.search_with_scorer(&entry, scorer, knn_collector, accept_docs)
    }

    fn search_byte(
        &mut self,
        field: &str,
        target: &[u8],
        knn_collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        let entry = self.get_field_entry(field, VectorEncoding::BYTE)?.clone();
        let scorer = self
            .flat_vectors_reader
            .get_random_vector_scorer_byte(field, target)?;
        self.search_with_scorer(&entry, scorer, knn_collector, accept_docs)
    }

    fn get_merge_instance(&self) -> Result<Box<dyn KnnVectorsReader>> {
        Ok(Box::new(self.clone_reader()?))
    }

    fn close(&mut self) -> Result<()> {
        self.flat_vectors_reader.close()?;
        self.vector_index.close()
    }

    fn get_off_heap_byte_size(&self, field_info: &FieldInfo) -> HashMap<String, i64> {
        let mut map = self.flat_vectors_reader.get_off_heap_byte_size(field_info);
        if let Ok(entry) = self.get_field_entry_or_throw(&field_info.name) {
            map.insert(
                VECTOR_INDEX_EXTENSION.to_string(),
                entry.vector_index_length,
            );
        }
        map
    }
}

fn score_batch_exhaustive(
    ords: &mut Vec<i32>,
    scores: &mut [f32],
    collector: &mut TopKnnCollector,
    scorer: &mut dyn RandomVectorScorer,
) -> Result<()> {
    let n = ords.len();
    if n == 0 {
        return Ok(());
    }
    scorer.bulk_score(ords, &mut scores[..n], n as i32)?;
    for i in 0..n {
        collector.collect(ords[i], scores[i]);
    }
    collector.inc_visited_count(n as i32);
    ords.clear();
    Ok(())
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
        let vector_index_offset = input.read_v_long()?;
        let vector_index_length = input.read_v_long()?;
        let dimension = input.read_v_int()?;
        let size = input.read_int()?;
        let m = input.read_v_int()?;
        let num_levels = input.read_v_int()?;
        let mut nodes_by_level = vec![None; num_levels as usize];
        let mut number_of_offsets = 0i64;
        for level in 0..num_levels {
            if level > 0 {
                let num_nodes = input.read_v_int()?;
                number_of_offsets += num_nodes as i64;
                let mut nodes = vec![0i32; num_nodes as usize];
                nodes[0] = input.read_v_int()?;
                for i in 1..num_nodes {
                    let delta = input.read_v_int()?;
                    nodes[i as usize] = nodes[i as usize - 1] + delta;
                }
                nodes_by_level[level as usize] = Some(nodes);
            } else {
                number_of_offsets += size as i64;
            }
        }
        let (offsets_offset, offsets_block_shift, offsets_meta, offsets_length) =
            if number_of_offsets > 0 {
                let offsets_offset = input.read_long()?;
                let offsets_block_shift = input.read_v_int()?;
                let offsets_meta =
                    DirectMonotonicMeta::load(input, number_of_offsets, offsets_block_shift)?;
                let offsets_length = input.read_long()?;
                (
                    offsets_offset,
                    offsets_block_shift,
                    Some(offsets_meta),
                    offsets_length,
                )
            } else {
                (0, 0, None, 0)
            };

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

        Ok(Self {
            field_info: field_info.clone(),
            similarity_function,
            vector_encoding,
            vector_index_offset,
            vector_index_length,
            m,
            num_levels,
            dimension,
            size,
            nodes_by_level,
            offsets_meta: offsets_meta.unwrap_or(DirectMonotonicMeta {
                block_shift: 0,
                num_blocks: 0,
                mins: Vec::new(),
                avgs: Vec::new(),
                offsets: Vec::new(),
                bpvs: Vec::new(),
            }),
            offsets_offset,
            offsets_block_shift,
            offsets_length,
        })
    }
}

// -----------------------------------------------------------------------------
// Off-heap HNSW graph
// -----------------------------------------------------------------------------

struct OffHeapHnswGraph {
    data_in: Box<dyn IndexInput>,
    nodes_by_level: Vec<Option<Vec<i32>>>,
    num_levels: i32,
    entry_node: i32,
    size: i32,
    max_conn: i32,
    offsets: Vec<i64>,
    graph_level_node_index_offsets: Vec<i64>,
    current_neighbors_buffer: Vec<i32>,
    arc_count: usize,
    arc_upto: usize,
    version: i32,
}

impl OffHeapHnswGraph {
    fn new(entry: &FieldEntry, vector_index: &dyn IndexInput, version: i32) -> Result<Self> {
        let data_in = vector_index.slice(
            "graph-data",
            entry.vector_index_offset,
            entry.vector_index_length,
        )?;
        let mut graph_level_node_index_offsets = vec![0i64; entry.num_levels as usize];
        let mut number_of_offsets = entry.size as i64;
        for i in 1..entry.num_levels as usize {
            let node_count = entry.nodes_by_level[i - 1]
                .as_ref()
                .map_or(entry.size, |n| n.len() as i32);
            graph_level_node_index_offsets[i] =
                graph_level_node_index_offsets[i - 1] + node_count as i64;
            number_of_offsets += node_count as i64;
        }
        let offsets_data = read_offsets_data(vector_index, entry)?;
        let offsets_reader = DirectMonotonicReader::new(entry.offsets_meta.clone(), offsets_data)?;
        let mut offsets = Vec::with_capacity(number_of_offsets as usize);
        for i in 0..number_of_offsets {
            offsets.push(offsets_reader.get(i));
        }
        let max_conn = entry.m;
        let current_neighbors_buffer = vec![0i32; (max_conn * 2).max(4) as usize];
        let entry_node = if entry.num_levels > 1 {
            entry.nodes_by_level[entry.num_levels as usize - 1]
                .as_ref()
                .map_or(0, |n| n[0])
        } else {
            0
        };
        Ok(Self {
            data_in,
            nodes_by_level: entry.nodes_by_level.clone(),
            num_levels: entry.num_levels,
            entry_node,
            size: entry.size,
            max_conn,
            offsets,
            graph_level_node_index_offsets,
            current_neighbors_buffer,
            arc_count: 0,
            arc_upto: 0,
            version,
        })
    }
}

fn read_offsets_data(vector_index: &dyn IndexInput, entry: &FieldEntry) -> Result<Vec<u8>> {
    if entry.offsets_length <= 0 {
        return Ok(Vec::new());
    }
    let mut random_access =
        vector_index.random_access_slice(entry.offsets_offset, entry.offsets_length)?;
    let mut data = vec![0u8; entry.offsets_length as usize];
    let len = data.len();
    random_access.read_bytes_at(0, &mut data, 0, len)?;
    Ok(data)
}

impl HnswGraph for OffHeapHnswGraph {
    fn seek(&mut self, level: i32, target: i32) -> Result<()> {
        let target_index = if level == 0 {
            target
        } else {
            let nodes = self.nodes_by_level[level as usize]
                .as_ref()
                .ok_or_else(|| {
                    LuceneError::CorruptIndex(format!("missing nodes for level {level}"))
                })?;
            match nodes.binary_search(&target) {
                Ok(idx) => idx as i32,
                Err(_) => {
                    return Err(LuceneError::CorruptIndex(format!(
                        "node {target} not found on level {level}"
                    )));
                }
            }
        };
        let offset_index =
            target_index as i64 + self.graph_level_node_index_offsets[level as usize];
        let offset = self.offsets[offset_index as usize];
        self.data_in.seek(offset)?;
        self.arc_count = self.data_in.read_v_int()? as usize;
        if self.arc_count > 0 {
            if self.version >= VERSION_GROUPVARINT {
                read_group_v_ints(
                    self.data_in.as_mut(),
                    &mut self.current_neighbors_buffer,
                    self.arc_count,
                )?;
            } else {
                for i in 0..self.arc_count {
                    self.current_neighbors_buffer[i] = self.data_in.read_v_int()?;
                }
            }
            let mut sum = 0i32;
            for i in 0..self.arc_count {
                sum += self.current_neighbors_buffer[i];
                self.current_neighbors_buffer[i] = sum;
            }
        }
        self.arc_upto = 0;
        Ok(())
    }

    fn size(&self) -> i32 {
        self.size
    }

    fn max_node_id(&self) -> i32 {
        self.size - 1
    }

    fn next_neighbor(&mut self) -> Result<i32> {
        if self.arc_upto >= self.arc_count {
            return Ok(crate::util::hnsw::NO_MORE_DOCS);
        }
        let n = self.current_neighbors_buffer[self.arc_upto];
        self.arc_upto += 1;
        Ok(n)
    }

    fn num_levels(&self) -> Result<i32> {
        Ok(self.num_levels)
    }

    fn max_conn(&self) -> i32 {
        self.max_conn
    }

    fn entry_node(&self) -> Result<i32> {
        Ok(self.entry_node)
    }

    fn get_nodes_on_level(&self, level: i32) -> Result<Box<dyn NodesIterator>> {
        if level == 0 {
            Ok(Box::new(DenseNodesIterator::new(self.size)))
        } else {
            let nodes = self.nodes_by_level[level as usize].clone().ok_or_else(|| {
                LuceneError::CorruptIndex(format!("missing nodes for level {level}"))
            })?;
            Ok(Box::new(ArrayNodesIterator::new(nodes)))
        }
    }

    fn neighbor_count(&self) -> i32 {
        self.arc_count as i32
    }
}

fn read_group_v_ints(input: &mut dyn DataInput, dst: &mut [i32], limit: usize) -> Result<()> {
    let mut i = 0;
    while i + 4 <= limit {
        let flag = input.read_byte()? as i32;
        let n1 = (flag >> 6) as usize;
        let n2 = ((flag >> 4) & 0x03) as usize;
        let n3 = ((flag >> 2) & 0x03) as usize;
        let n4 = (flag & 0x03) as usize;
        dst[i] = read_int_in_group(input, n1)?;
        dst[i + 1] = read_int_in_group(input, n2)?;
        dst[i + 2] = read_int_in_group(input, n3)?;
        dst[i + 3] = read_int_in_group(input, n4)?;
        i += 4;
    }
    while i < limit {
        dst[i] = input.read_v_int()?;
        i += 1;
    }
    Ok(())
}

fn read_int_in_group(input: &mut dyn DataInput, num_bytes_minus_1: usize) -> Result<i32> {
    match num_bytes_minus_1 {
        0 => Ok(input.read_byte()? as i32 & 0xFF),
        1 => Ok(input.read_short()? as i32 & 0xFFFF),
        2 => {
            let short = input.read_short()? as i32 & 0xFFFF;
            let byte = input.read_byte()? as i32 & 0xFF;
            Ok(short | (byte << 16))
        }
        _ => Ok(input.read_int()?),
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::stub::{BufferedUpdates, FieldInfos};
    use crate::codecs::{KnnVectorsFormat, KnnVectorsReader, KnnVectorsWriter};
    use crate::index::field_infos::FieldInfo;
    use crate::index::{
        segment_file_name, DocValuesSkipIndexType, DocValuesType, IndexOptions, SegmentInfo,
        VectorEncoding, VectorSimilarityFunction,
    };
    use crate::search::from_live_docs;
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
        let format = Lucene99HnswVectorsFormat::new();
        assert_eq!(format.name(), "Lucene99HnswVectorsFormat");
        assert_eq!(format.get_max_dimensions("any"), 1024);
    }

    #[test]
    fn round_trip_float_vectors_and_search() -> Result<()> {
        let dir = Arc::new(RamDirectory::new()) as Arc<dyn crate::store::Directory>;
        let seg_info = make_segment_info(&dir, 3)?;
        let field_info = float_field_info("float_field", 0, 2)?;
        let field_infos = FieldInfos::new(vec![field_info.clone()])?;
        let seg_updates = BufferedUpdates;
        let io_ctx = DefaultIOContext::default();
        let (write_state, read_state) =
            make_states(&dir, &seg_info, &field_infos, &seg_updates, &io_ctx);

        let format = Lucene99HnswVectorsFormat::new();
        {
            let mut writer = format.fields_writer(&write_state)?;
            let field_writer = writer.add_field(&field_info)?;
            let mut float_writer = match field_writer {
                FieldVectorWriter::Float(w) => w,
                _ => panic!("expected float field writer"),
            };
            float_writer.add_value(0, vec![1.0, 0.0])?;
            float_writer.add_value(1, vec![0.0, 1.0])?;
            float_writer.add_value(2, vec![1.0, 1.0])?;
            writer.flush(3, None)?;
            writer.finish()?;
            writer.close()?;
        }

        let mut reader = format.fields_reader(&read_state)?;
        let values = reader.get_float_vector_values("float_field")?;
        assert_eq!(values.size(), 3);
        assert_eq!(values.dimension(), 2);

        let mut collector = TopKnnCollector::new(1, i64::MAX);
        let mut accept_docs = from_live_docs(None, 3)?;
        reader.search("float_field", &[1.0, 0.0], &mut collector, &mut accept_docs)?;
        let top = collector.top_docs_with_scores();
        assert_eq!(top.len(), 1);
        assert_eq!(top[0].0, 0);

        reader.check_integrity()?;
        reader.close()?;
        Ok(())
    }

    #[test]
    fn round_trip_byte_vectors() -> Result<()> {
        let dir = Arc::new(RamDirectory::new()) as Arc<dyn crate::store::Directory>;
        let seg_info = make_segment_info(&dir, 3)?;
        let field_info = byte_field_info("byte_field", 0, 2)?;
        let field_infos = FieldInfos::new(vec![field_info.clone()])?;
        let seg_updates = BufferedUpdates;
        let io_ctx = DefaultIOContext::default();
        let (write_state, read_state) =
            make_states(&dir, &seg_info, &field_infos, &seg_updates, &io_ctx);

        let format = Lucene99HnswVectorsFormat::new();
        {
            let mut writer = format.fields_writer(&write_state)?;
            let field_writer = writer.add_field(&field_info)?;
            let mut byte_writer = match field_writer {
                FieldVectorWriter::Byte(w) => w,
                _ => panic!("expected byte field writer"),
            };
            byte_writer.add_value(0, vec![10, 20])?;
            byte_writer.add_value(1, vec![30, 40])?;
            byte_writer.add_value(2, vec![50, 60])?;
            writer.flush(3, None)?;
            writer.finish()?;
            writer.close()?;
        }

        let reader = format.fields_reader(&read_state)?;
        let values = reader.get_byte_vector_values("byte_field")?;
        assert_eq!(values.size(), 3);
        assert_eq!(values.dimension(), 2);
        assert_eq!(values.vector_value(0)?, vec![10, 20]);

        reader.check_integrity()?;
        Ok(())
    }

    #[test]
    fn written_files_have_codec_headers() -> Result<()> {
        let dir = Arc::new(RamDirectory::new()) as Arc<dyn crate::store::Directory>;
        let seg_info = make_segment_info(&dir, 1)?;
        let field_info = float_field_info("f", 0, 1)?;
        let field_infos = FieldInfos::new(vec![field_info.clone()])?;
        let seg_updates = BufferedUpdates;
        let io_ctx = DefaultIOContext::default();
        let (write_state, read_state) =
            make_states(&dir, &seg_info, &field_infos, &seg_updates, &io_ctx);

        let format = Lucene99HnswVectorsFormat::new();
        {
            let mut writer = format.fields_writer(&write_state)?;
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
        let vem = segment_file_name("test", "", "vem");
        let vex = segment_file_name("test", "", "vex");
        let vemf = segment_file_name("test", "", "vemf");
        let vec = segment_file_name("test", "", "vec");
        assert!(files.contains(&vem));
        assert!(files.contains(&vex));
        assert!(files.contains(&vemf));
        assert!(files.contains(&vec));

        // Verify the .vem and .vex files have valid headers/footers by reading
        // them through the format reader (which validates the full envelopes).
        let reader = format.fields_reader(&read_state)?;
        reader.check_integrity()?;
        Ok(())
    }
}
