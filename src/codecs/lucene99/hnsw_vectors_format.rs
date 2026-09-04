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
use crate::codecs::state::{OwnedSegmentWriteState, SegmentReadState, SegmentWriteState};
use crate::codecs::stub::FieldInfo;
use crate::error::{LuceneError, Result};
use crate::index::vector_values::{
    ByteVectorValues, DocIndexIterator, FloatVectorValues, KnnVectorValues,
};
use crate::index::{segment_file_name, FieldInfos, VectorEncoding, VectorSimilarityFunction};
use crate::search::knn::{KnnCollector, TopKnnCollector};
use crate::search::{AcceptDocs, DocIdSetIterator, NO_MORE_DOCS};
use crate::store::{DataInput, IndexInput, IndexOutput};
use crate::util::hnsw::{
    ArrayNodesIterator, CloseableRandomVectorScorerSupplier, ConcurrentHnswMerger,
    DenseNodesIterator, HnswBuilder, HnswGraph, HnswGraphBuilder, HnswGraphMerger,
    HnswGraphSearcher, IncrementalHnswGraphMerger, NeighborArray, NeighborQueue, NodesIterator,
    OnHeapHnswGraph, RandomVectorScorer,
};
use crate::util::packed::{DirectMonotonicMeta, DirectMonotonicReader, DirectMonotonicWriter};
use crate::util::quantization::{QuantizedByteVectorValues, ScalarQuantizer};
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

    fn fields_writer(&self, state: &OwnedSegmentWriteState) -> Result<Box<dyn KnnVectorsWriter>> {
        ensure_registered();
        Ok(Box::new(Lucene99HnswVectorsWriter::new(
            &state.borrow(),
            self.max_conn,
            self.beam_width,
            self.num_merge_workers,
            self.tiny_segments_threshold,
            self.write_version,
        )?))
    }

    fn fields_reader<'a>(&self, state: &SegmentReadState<'a>) -> Result<Box<dyn KnnVectorsReader>> {
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
    /// The format the flat vectors were written with, reopened for reading by
    /// [`Lucene99HnswVectorsWriter::ensure_flat_reader_open`] once the merge
    /// needs to score them.
    flat_vectors_format: Lucene99FlatVectorsFormat,
    /// The reader over the flat vectors this writer just wrote, opened lazily.
    flat_vectors_reader: Option<Box<dyn FlatVectorsReader>>,
    /// Whether [`Lucene99HnswVectorsWriter::ensure_flat_reader_open`] already
    /// closed [`Self::flat_vector_writer`], so `close` does not close it twice.
    flat_writer_closed: bool,
    m: i32,
    beam_width: i32,
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

    fn ram_bytes_used(&self) -> i64 {
        // The shared handle owns nothing: the buffer it locks is accounted by
        // the writer that also holds it, and counting it here would count it
        // twice in `KnnVectorsWriter::ram_bytes_used`.
        0
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
                flat_vectors_format: Lucene99FlatVectorsFormat::new(
                    DefaultFlatVectorScorer::INSTANCE,
                ),
                flat_vectors_reader: None,
                flat_writer_closed: false,
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

    /// Writes one field's metadata entry.
    ///
    /// Equivalent to `Lucene99HnswVectorsWriter.writeMeta(FieldInfo, long, long, int, HnswGraph, int[][])`.
    fn write_meta(
        &mut self,
        field_info: &FieldInfo,
        vector_index_offset: i64,
        vector_index_length: i64,
        count: i32,
        graph: Option<&OnHeapHnswGraph>,
        graph_level_node_offsets: &[Vec<i32>],
    ) -> Result<()> {
        // The metadata only needs the nodes of each level, which is all Java's
        // "mock graph" answers with; collecting them here lets the sorted path
        // reuse the same writer with the levels it rebuilt.
        let levels = match graph {
            None => Vec::new(),
            Some(graph) => {
                let mut levels = Vec::with_capacity(graph.num_levels()? as usize);
                for level in 0..graph.num_levels()? {
                    let mut nodes_on_level = graph.get_nodes_on_level(level)?;
                    let mut nodes = Vec::with_capacity(nodes_on_level.size() as usize);
                    while nodes_on_level.has_next() {
                        nodes.push(nodes_on_level.next_int());
                    }
                    levels.push(nodes);
                }
                levels
            }
        };
        self.write_meta_from_levels(
            field_info,
            vector_index_offset,
            vector_index_length,
            count,
            &levels,
            graph_level_node_offsets,
        )
    }

    /// Writes one field's metadata entry from the nodes of each level.
    ///
    /// This is the body of `Lucene99HnswVectorsWriter.writeMeta`, taking the
    /// level membership directly rather than through the graph, so that the
    /// sorted path — which has rebuilt the levels under the new numbering — can
    /// write the same metadata.
    fn write_meta_from_levels(
        &mut self,
        field_info: &FieldInfo,
        vector_index_offset: i64,
        vector_index_length: i64,
        count: i32,
        levels: &[Vec<i32>],
        graph_level_node_offsets: &[Vec<i32>],
    ) -> Result<()> {
        let meta = self.meta.as_mut().ok_or_else(|| {
            LuceneError::IllegalState("Lucene99HnswVectorsWriter is already closed".to_string())
        })?;
        meta.write_int(field_info.number)?;
        meta.write_int(vector_encoding_ordinal(field_info.vector_encoding))?;
        meta.write_int(dist_func_to_ord(field_info.vector_similarity_function))?;
        meta.write_v_long(vector_index_offset)?;
        meta.write_v_long(vector_index_length)?;
        meta.write_v_int(field_info.vector_dimension)?;
        meta.write_int(count)?;
        meta.write_v_int(self.m)?;
        if !levels.is_empty() {
            meta.write_v_int(levels.len() as i32)?;
            let mut value_count = 0i64;
            for (level, nodes) in levels.iter().enumerate() {
                value_count += nodes.len() as i64;
                if level > 0 {
                    let mut nol = nodes.clone();
                    nol.sort_unstable();
                    meta.write_v_int(nol.len() as i32)?;
                    // Delta-encode, from the top down so each entry still sees
                    // its undeltaed predecessor.
                    for i in (1..nol.len()).rev() {
                        nol[i] -= nol[i - 1];
                    }
                    for n in nol {
                        debug_assert!(n >= 0, "nodes must be sorted before delta encoding");
                        meta.write_v_int(n)?;
                    }
                } else {
                    debug_assert_eq!(
                        nodes.len() as i32,
                        count,
                        "level 0 expects to have all nodes"
                    );
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

    /// Writes a field of a sorted segment: the graph is rebuilt under the new
    /// ordinal numbering before it is written.
    ///
    /// Equivalent to `Lucene99HnswVectorsWriter.writeSortingField(FieldWriter, Sorter.DocMap)`.
    /// Vector ordinals follow document order, so a sort renumbers them; the
    /// graph's edges have to be renumbered with them or every neighbour list
    /// would point at the wrong vector.
    fn write_sorting_field(
        &mut self,
        field_info: &FieldInfo,
        ord_to_doc: &[i32],
        graph: Option<&OnHeapHnswGraph>,
        sort_map: &crate::index::index_sorter::SorterDocMap,
    ) -> Result<()> {
        let (old_to_new_map, new_to_old_map) =
            crate::codecs::knn_vectors::map_old_ord_to_new_ord(ord_to_doc, sort_map)?;

        let vector_index_offset = {
            let vector_index = self.vector_index.as_ref().ok_or_else(|| {
                LuceneError::IllegalState("Lucene99HnswVectorsWriter is already closed".to_string())
            })?;
            vector_index.file_pointer()
        };
        let (levels, graph_level_node_offsets) =
            self.reconstruct_and_write_graph(graph, &new_to_old_map, &old_to_new_map)?;
        let vector_index_length = {
            let vector_index = self.vector_index.as_ref().ok_or_else(|| {
                LuceneError::IllegalState("Lucene99HnswVectorsWriter is already closed".to_string())
            })?;
            vector_index.file_pointer() - vector_index_offset
        };

        self.write_meta_from_levels(
            field_info,
            vector_index_offset,
            vector_index_length,
            ord_to_doc.len() as i32,
            &levels,
            &graph_level_node_offsets,
        )
    }

    /// Writes every node's neighbours under the new ordinal numbering, and
    /// returns the nodes of each level together with the byte length of each
    /// node's entry.
    ///
    /// Equivalent to `Lucene99HnswVectorsWriter.reconstructAndWriteGraph(OnHeapHnswGraph, int[], int[], int[][])`.
    /// Java returns a "mock graph" that answers only what `writeMeta` asks of
    /// it — the size, the level count, the max connections, and the nodes of
    /// each level; this port returns those directly, because a graph that
    /// raises from every other method is a shape Rust need not construct.
    fn reconstruct_and_write_graph(
        &mut self,
        graph: Option<&OnHeapHnswGraph>,
        new_to_old_map: &[i32],
        old_to_new_map: &[i32],
    ) -> Result<(Vec<Vec<i32>>, Vec<Vec<i32>>)> {
        let Some(graph) = graph else {
            return Ok((Vec::new(), Vec::new()));
        };
        let max_ord = graph.size();
        let scratch_capacity = (graph.max_conn() * 2).max(4) as usize;
        let num_levels = graph.num_levels()?;
        let mut levels: Vec<Vec<i32>> = Vec::with_capacity(num_levels as usize);
        let mut level_node_offsets: Vec<Vec<i32>> = Vec::with_capacity(num_levels as usize);

        // Level 0 holds every node, already numbered 0..size in the new space.
        let mut nodes_on_level0 = graph.get_nodes_on_level(0)?;
        let mut level0_nodes = Vec::with_capacity(nodes_on_level0.size() as usize);
        let mut level0_offsets = vec![0i32; nodes_on_level0.size() as usize];
        while nodes_on_level0.has_next() {
            let node = nodes_on_level0.next_int();
            level0_nodes.push(node);
            let neighbors = graph.get_neighbors(0, new_to_old_map[node as usize])?;
            let written = self.reconstruct_and_write_neighbours(
                neighbors,
                old_to_new_map,
                scratch_capacity,
                max_ord,
            )?;
            level0_offsets[node as usize] = written;
        }
        levels.push(level0_nodes);
        level_node_offsets.push(level0_offsets);

        for level in 1..num_levels {
            let mut nodes_on_level = graph.get_nodes_on_level(level)?;
            let mut new_nodes = Vec::with_capacity(nodes_on_level.size() as usize);
            while nodes_on_level.has_next() {
                new_nodes.push(old_to_new_map[nodes_on_level.next_int() as usize]);
            }
            new_nodes.sort_unstable();
            let mut offsets = Vec::with_capacity(new_nodes.len());
            for &node in &new_nodes {
                let neighbors = graph.get_neighbors(level, new_to_old_map[node as usize])?;
                offsets.push(self.reconstruct_and_write_neighbours(
                    neighbors,
                    old_to_new_map,
                    scratch_capacity,
                    max_ord,
                )?);
            }
            levels.push(new_nodes);
            level_node_offsets.push(offsets);
        }
        Ok((levels, level_node_offsets))
    }

    /// Writes one node's neighbours under the new ordinal numbering, and
    /// returns how many bytes that took.
    ///
    /// Equivalent to `Lucene99HnswVectorsWriter.reconstructAndWriteNeighbours(NeighborArray, int[], int[], int)`:
    /// the neighbours are remapped, sorted, de-duplicated and delta-encoded, in
    /// that order, because the delta encoding is what keeps the entry small and
    /// only a sorted list deltas usefully.
    fn reconstruct_and_write_neighbours(
        &mut self,
        neighbors: &NeighborArray,
        old_to_new_map: &[i32],
        scratch_capacity: usize,
        max_ord: i32,
    ) -> Result<i32> {
        let size = neighbors.size() as usize;
        let mut nnodes: Vec<i32> = neighbors.nodes()[..size]
            .iter()
            .map(|&node| old_to_new_map[node as usize])
            .collect();
        nnodes.sort_unstable();

        let mut scratch: Vec<i32> = Vec::with_capacity(scratch_capacity.max(size));
        if size > 0 {
            scratch.push(nnodes[0]);
        }
        for i in 1..size {
            debug_assert!(
                nnodes[i] < max_ord,
                "node too large: {}>={max_ord}",
                nnodes[i]
            );
            if nnodes[i - 1] == nnodes[i] {
                continue;
            }
            scratch.push(nnodes[i] - nnodes[i - 1]);
        }

        let version = self.version;
        let vector_index = self.vector_index.as_mut().ok_or_else(|| {
            LuceneError::IllegalState("Lucene99HnswVectorsWriter is already closed".to_string())
        })?;
        let start = vector_index.file_pointer();
        vector_index.write_v_int(scratch.len() as i32)?;
        if version >= VERSION_GROUPVARINT {
            write_group_v_ints(vector_index.as_mut(), &scratch)?;
        } else {
            for value in &scratch {
                vector_index.write_v_int(*value)?;
            }
        }
        Ok((vector_index.file_pointer() - start) as i32)
    }

    /// Merges the source segments' graphs into one and writes it, together
    /// with the field's metadata entry.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when this writer is already
    /// closed, and whatever merging or writing the graph raises.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene99.Lucene99HnswVectorsWriter#buildAndWriteGraph`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java hands each source reader to the
    /// merger as a `KnnVectorsReader` and the merger narrows it to an
    /// `HnswGraphProvider` with `instanceof`, reusing the source graph when it
    /// is one. Rust cannot narrow a `dyn KnnVectorsReader`, so
    /// [`HnswGraphMerger::add_reader`] takes the provider as a separate
    /// argument and `None` is passed here — the merger then treats the reader
    /// exactly as Java treats one that is not a provider, and rebuilds. The
    /// divergence is the merger's, already recorded on that trait; this method
    /// only inherits it.
    ///
    /// Nothing calls this yet: Java reaches it from the `IORunnable` that
    /// `mergeOneField` returns, and that closure captures the writer itself,
    /// which a `Box<dyn IORunnable>` cannot own while
    /// [`KnnVectorsWriter::finish`] still needs the same outputs.
    #[allow(dead_code)]
    fn build_and_write_graph(
        &mut self,
        field_info: &FieldInfo,
        merge_state: &MergeState,
        vector_values: &dyn KnnVectorValues,
        scorer_supplier: Arc<dyn crate::util::hnsw::RandomVectorScorerSupplier>,
        total_vector_count: i32,
    ) -> Result<()> {
        let vector_index_offset = self
            .vector_index
            .as_ref()
            .ok_or_else(|| {
                LuceneError::IllegalState("Lucene99HnswVectorsWriter is already closed".to_string())
            })?
            .file_pointer();
        let mut merger =
            self.create_graph_merger(field_info, scorer_supplier, self.num_merge_workers);
        for i in 0..merge_state.live_docs.len() {
            // `KnnVectorsWriter.MergedVectorValues.hasVectorValues(FieldInfos,
            // String)`, inlined: that helper belongs to a type this port has
            // not brought over, and its body is these three tests.
            let field_infos = &merge_state.field_infos[i];
            let has_vector_values = field_infos.has_vector_values()
                && field_infos
                    .field_info(&field_info.name)
                    .is_some_and(|info| info.has_vector_values());
            if !has_vector_values {
                continue;
            }
            let Some(reader) = merge_state
                .knn_vectors_readers
                .get(i)
                .and_then(Clone::clone)
            else {
                continue;
            };
            let doc_map = Arc::new(merge_state.doc_maps[i].clone());
            merger.add_reader(reader, None, doc_map, merge_state.live_docs[i].as_deref())?;
        }
        let graph = merger.merge(vector_values, total_vector_count)?;
        let vector_index_node_offsets = self.write_graph(&graph)?;
        let vector_index_length = self
            .vector_index
            .as_ref()
            .ok_or_else(|| {
                LuceneError::IllegalState("Lucene99HnswVectorsWriter is already closed".to_string())
            })?
            .file_pointer()
            - vector_index_offset;
        self.write_meta(
            field_info,
            vector_index_offset,
            vector_index_length,
            total_vector_count,
            Some(&graph),
            &vector_index_node_offsets,
        )
    }

    /// Returns the merger that will build the merged segment's graph.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene99.Lucene99HnswVectorsWriter#createGraphMerger`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java chooses between three cases: a
    /// `ConcurrentHnswMerger` on the writer's own `mergeExec`, a
    /// `ConcurrentHnswMerger` on the merge's `TaskExecutor` when more than one
    /// worker was asked for, and an `IncrementalHnswGraphMerger` otherwise
    /// (`Lucene99HnswVectorsWriter.java:628-646`). This port has no
    /// `TaskExecutor`: [`ConcurrentHnswMerger`] runs its own workers, so the
    /// first two cases are the same one, selected by the worker count exactly
    /// as Java selects the second.
    fn create_graph_merger(
        &self,
        field_info: &FieldInfo,
        scorer_supplier: Arc<dyn crate::util::hnsw::RandomVectorScorerSupplier>,
        num_parallel_merge_workers: i32,
    ) -> Box<dyn HnswGraphMerger> {
        if num_parallel_merge_workers > 1 {
            Box::new(ConcurrentHnswMerger::new(
                field_info.clone(),
                scorer_supplier,
                self.m,
                self.beam_width,
                num_parallel_merge_workers,
            ))
        } else {
            Box::new(IncrementalHnswGraphMerger::new(
                field_info.clone(),
                scorer_supplier,
                self.m,
                self.beam_width,
            ))
        }
    }

    /// Finishes and closes the flat writer, then opens a reader over the flat
    /// vectors it wrote, so the graph can be built by scoring them.
    ///
    /// Does nothing once the reader is open.
    ///
    /// # Errors
    ///
    /// Returns whatever finishing the flat writer or opening the reader
    /// raises.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene99.Lucene99HnswVectorsWriter#ensureFlatReaderOpen`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java builds the `SegmentReadState`
    /// out of the `segmentWriteState` field it kept from its constructor
    /// (`Lucene99HnswVectorsWriter.java:499-511`). This port's writer outlives
    /// the borrowed [`SegmentWriteState`] it was built from — the indexing
    /// chain creates it on the first vector of the first document and keeps it
    /// until the segment flushes — so it cannot hold one, and the state is
    /// passed in instead. It is the same state Java stored.
    ///
    /// Nothing calls this yet, for the reason recorded on
    /// [`build_and_write_graph`](Self::build_and_write_graph).
    #[allow(dead_code)]
    fn ensure_flat_reader_open(&mut self, state: &SegmentWriteState<'_>) -> Result<()> {
        if self.flat_vectors_reader.is_none() {
            self.flat_vector_writer.finish()?;
            self.flat_vector_writer.close()?;
            self.flat_writer_closed = true;
            let read_state = SegmentReadState::with_suffix(
                state.directory,
                state.segment_info,
                state.field_infos,
                state.context,
                state.segment_suffix.clone(),
            );
            self.flat_vectors_reader =
                Some(self.flat_vectors_format.fields_reader_flat(&read_state)?);
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
                    if let Some(sort_map) = sort_map {
                        // A sorted segment renumbers the vector ordinals, so
                        // the graph is rebuilt under the new numbering.
                        let _ = vector_index_offset;
                        self.write_sorting_field(
                            &info,
                            values.ord_to_doc(),
                            graph.as_ref(),
                            sort_map,
                        )?;
                    } else {
                        let graph_level_offsets = if let Some(ref g) = graph {
                            self.write_graph(g)?
                        } else {
                            Vec::new()
                        };
                        let vector_index_length =
                            self.vector_index.as_ref().unwrap().file_pointer()
                                - vector_index_offset;
                        self.write_meta(
                            &info,
                            vector_index_offset,
                            vector_index_length,
                            count,
                            graph.as_ref(),
                            &graph_level_offsets,
                        )?;
                    }
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
                    if let Some(sort_map) = sort_map {
                        // A sorted segment renumbers the vector ordinals, so
                        // the graph is rebuilt under the new numbering.
                        let _ = vector_index_offset;
                        self.write_sorting_field(
                            &info,
                            values.ord_to_doc(),
                            graph.as_ref(),
                            sort_map,
                        )?;
                    } else {
                        let graph_level_offsets = if let Some(ref g) = graph {
                            self.write_graph(g)?
                        } else {
                            Vec::new()
                        };
                        let vector_index_length =
                            self.vector_index.as_ref().unwrap().file_pointer()
                                - vector_index_offset;
                        self.write_meta(
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

    /// Equivalent to `Lucene99HnswVectorsWriter.ramBytesUsed()`
    /// (`Lucene99HnswVectorsWriter.java:245-252`), which sums a shallow size
    /// and every field's footprint.
    ///
    /// **Divergence:** Java's per-field total also carries the partially built
    /// HNSW graph, because `Lucene99HnswVectorsWriter.FieldWriter.addValue`
    /// feeds `hnswGraphBuilder.addGraphNode` as each vector arrives
    /// (`Lucene99HnswVectorsWriter.java:800-813`). This port builds the graph
    /// in `flush`, from the buffered vectors, so no graph exists while
    /// documents are being indexed and there is nothing to add here. The
    /// buffered vectors — the term that grows with the data — are accounted
    /// exactly as Java accounts them.
    fn ram_bytes_used(&self) -> i64 {
        let mut total = crate::util::RamUsageEstimator::align_object_size(
            crate::util::RamUsageEstimator::NUM_BYTES_OBJECT_HEADER
                + 10 * crate::util::RamUsageEstimator::NUM_BYTES_OBJECT_REF,
        ) + self.flat_vector_writer.ram_bytes_used();
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

    fn close(&mut self) -> Result<()> {
        let mut last_err: Option<LuceneError> = None;
        // Java closes the flat writer only when `ensureFlatReaderOpen` has not
        // already closed it (`Lucene99HnswVectorsWriter.java:648-654`).
        if !self.flat_writer_closed {
            if let Err(e) = self.flat_vector_writer.close() {
                last_err = Some(e);
            }
        }
        if let Some(reader) = self.flat_vectors_reader.as_mut() {
            if let Err(e) = reader.close() {
                last_err = Some(e);
            }
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

/// Returns the ordinal a similarity function is written as.
///
/// Equivalent to `Lucene99HnswVectorsWriter.distFuncToOrd(VectorSimilarityFunction)`.
/// The ordinals are part of the file format, so they are written out here
/// rather than derived from the enum's declaration order.
fn dist_func_to_ord(similarity: VectorSimilarityFunction) -> i32 {
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

/// Reads a similarity function from the metadata stream.
///
/// Equivalent to
/// `org.apache.lucene.codecs.lucene99.Lucene99HnswVectorsReader#readSimilarityFunction`,
/// which reads the ordinal and looks it up in `SIMILARITY_FUNCTIONS`. Java
/// keeps that list so the file format does not depend on the declaration order
/// of `VectorSimilarityFunction`; here [`read_vector_similarity`] is that
/// table.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when the ordinal names no
/// similarity function, which is the `IllegalArgumentException` Java raises.
fn read_similarity_function(input: &mut dyn DataInput) -> Result<VectorSimilarityFunction> {
    let i = input.read_int()?;
    read_vector_similarity(i)
        .map_err(|_| LuceneError::IllegalArgument(format!("invalid distance function: {i}")))
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
    /// Returns the document each ordinal belongs to.
    fn ord_to_doc(&self) -> &[i32] {
        &self.ord_to_doc
    }

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
    fn copy_float(&self) -> Result<Box<dyn FloatVectorValues>> {
        Ok(Box::new(self.clone()))
    }

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
    /// Returns the document each ordinal belongs to.
    fn ord_to_doc(&self) -> &[i32] {
        &self.ord_to_doc
    }

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
    fn copy_byte(&self) -> Result<Box<dyn ByteVectorValues>> {
        Ok(Box::new(self.clone()))
    }

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

        let data_context = state.context.with_hints(state.context.hints());
        let vector_index = Self::open_data_input(
            state,
            version_meta,
            VECTOR_INDEX_EXTENSION,
            VECTOR_INDEX_CODEC_NAME,
            data_context.as_ref(),
        )?;

        Ok(Self {
            flat_vectors_reader,
            field_infos: state.field_infos.clone(),
            fields,
            vector_index,
            version: version_meta,
            data_context,
        })
    }

    /// Opens one of the format's data files, checks its header against
    /// `version_meta` and primes its checksum.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene99.Lucene99HnswVectorsReader#openDataInput`.
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
        Ok(input)
    }

    /// Returns the quantizer the vectors of `field` were compressed with, or
    /// `None` when the flat vectors this reader wraps are not quantized.
    ///
    /// # Errors
    ///
    /// Returns whatever the quantized reader raises while reading its state.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene99.Lucene99HnswVectorsReader#getQuantizationState`.
    /// Java tests `flatVectorsReader instanceof QuantizedVectorsReader`; this
    /// port asks the flat reader through
    /// [`FlatVectorsReader::as_quantized_vectors_reader`], which answers the
    /// same question — see the divergence recorded there.
    pub fn get_quantization_state(&self, field: &str) -> Result<Option<ScalarQuantizer>> {
        match self.flat_vectors_reader.as_quantized_vectors_reader() {
            Some(reader) => reader.get_quantization_state(field),
            None => Ok(None),
        }
    }

    /// Returns the quantized vectors stored for `field`, or `None` when the
    /// flat vectors this reader wraps are not quantized.
    ///
    /// # Errors
    ///
    /// Returns whatever the quantized reader raises while opening the values.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene99.Lucene99HnswVectorsReader#getQuantizedVectorValues`.
    /// The `instanceof` test is answered by
    /// [`FlatVectorsReader::as_quantized_vectors_reader`], as for
    /// [`get_quantization_state`](Self::get_quantization_state).
    pub fn get_quantized_vector_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn QuantizedByteVectorValues>>> {
        match self.flat_vectors_reader.as_quantized_vectors_reader() {
            Some(reader) => reader.get_quantized_vector_values(field),
            None => Ok(None),
        }
    }

    /// Returns a scorer supplier over the merged flat vectors of `field`, or
    /// `None` when the flat vectors this reader wraps are not quantized.
    ///
    /// The supplier may hold temporary file handles, so it should be closed
    /// once the merge that asked for it is done.
    ///
    /// # Errors
    ///
    /// Returns whatever the quantized reader raises while building the
    /// supplier.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene99.Lucene99HnswVectorsReader#getRandomVectorScorerSupplierForMerge`.
    /// The `instanceof` test is answered by
    /// [`FlatVectorsReader::as_quantized_vectors_reader`], as for
    /// [`get_quantization_state`](Self::get_quantization_state).
    pub fn get_random_vector_scorer_supplier_for_merge(
        &self,
        field_info: &FieldInfo,
        segment_write_state: &SegmentWriteState<'_>,
    ) -> Result<Option<Box<dyn CloseableRandomVectorScorerSupplier>>> {
        match self.flat_vectors_reader.as_quantized_vectors_reader() {
            Some(reader) => {
                reader.get_random_vector_scorer_supplier_for_merge(field_info, segment_write_state)
            }
            None => Ok(None),
        }
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
        &self,
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
                Box::new(FixedBitSet::new(graph.max_node_id().max(0) as usize + 1)),
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
        &self,
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
        &self,
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
        let entry = read_field(input, info)?;
        validate_field_entry(info, &entry)?;
        fields.insert(info.number, entry);
        field_number = input.read_int()?;
    }
    Ok(fields)
}

/// Reads one field's entry from the `.vem` metadata.
///
/// The encoding and the similarity function come first, and the similarity is
/// checked against the field's own before the rest of the entry is read, so a
/// metadata file written for a different similarity is rejected at the byte it
/// disagrees on rather than at the end of the record.
///
/// Equivalent to
/// `org.apache.lucene.codecs.lucene99.Lucene99HnswVectorsReader#readField`.
fn read_field(input: &mut dyn DataInput, info: &FieldInfo) -> Result<FieldEntry> {
    let vector_encoding = read_vector_encoding(input.read_int()?)?;
    let similarity_function = read_similarity_function(input)?;
    if similarity_function != info.vector_similarity_function {
        return Err(LuceneError::CorruptIndex(format!(
            "inconsistent vector similarity function for field=\"{}\"",
            info.name
        )));
    }
    FieldEntry::create(input, vector_encoding, similarity_function, info)
}

/// Checks that a field entry agrees with the field's own metadata.
///
/// Equivalent to
/// `org.apache.lucene.codecs.lucene99.Lucene99HnswVectorsReader#validateFieldEntry`.
fn validate_field_entry(info: &FieldInfo, field_entry: &FieldEntry) -> Result<()> {
    if info.vector_dimension != field_entry.dimension {
        return Err(LuceneError::CorruptIndex(format!(
            "inconsistent vector dimension for field=\"{}\"",
            info.name
        )));
    }
    Ok(())
}

impl FieldEntry {
    /// Reads the rest of a field's entry, after `read_field` has taken the
    /// encoding and the similarity function off the input.
    ///
    /// Equivalent to `Lucene99HnswVectorsReader.FieldEntry.create(IndexInput,
    /// VectorEncoding, VectorSimilarityFunction)`, which this port also hands
    /// the `FieldInfo`, because the corruption messages below name the field.
    fn create(
        input: &mut dyn DataInput,
        vector_encoding: VectorEncoding,
        similarity_function: VectorSimilarityFunction,
        field_info: &FieldInfo,
    ) -> Result<Self> {
        let vector_index_offset = input.read_v_long()?;
        let vector_index_length = input.read_v_long()?;
        let dimension = input.read_v_int()?;
        let size = input.read_int()?;
        let m = input.read_v_int()?;
        let num_levels = input.read_v_int()?;
        // `numLevels` is a vint off disk. Java writes `new int[numLevels][]`,
        // which raises `NegativeArraySizeException` for a negative count; that
        // is catchable, whereas `vec![None; negative as usize]` asks for an
        // allocation of about 2^64 entries and aborts the process.
        if num_levels < 0 {
            return Err(LuceneError::CorruptIndex(format!(
                "field \"{}\" declares {num_levels} graph levels",
                field_info.name
            )));
        }
        // Grown as the levels are read rather than sized up front: one level
        // costs at least one byte of input, so a hostile count runs out of file
        // instead of out of memory. Java pre-sizes the array and is exposed to
        // an `OutOfMemoryError` here; a Rust allocation failure is an abort, not
        // an error, so the pre-sizing is the one thing that cannot be copied.
        let mut nodes_by_level: Vec<Option<Vec<i32>>> = Vec::new();
        let mut number_of_offsets = 0i64;
        for level in 0..num_levels {
            if level > 0 {
                let num_nodes = input.read_v_int()?;
                if num_nodes < 0 {
                    return Err(LuceneError::CorruptIndex(format!(
                        "level {level} of field \"{}\" declares {num_nodes} nodes",
                        field_info.name
                    )));
                }
                number_of_offsets += i64::from(num_nodes);
                // Java reads `nodesByLevel[level][0]` unconditionally and hits
                // an `ArrayIndexOutOfBoundsException` on an empty level.

                // Again grown, not pre-sized: each node is at least one byte.
                let mut nodes: Vec<i32> = vec![0i32; num_nodes as usize];
                nodes[0] = input.read_v_int()?;
                nodes.truncate(1);
                for i in 1..num_nodes as usize {
                    let delta = input.read_v_int()?;
                    // Java accumulates with `int` arithmetic, which wraps.
                    nodes.push(nodes[i - 1].wrapping_add(delta));
                }
                nodes_by_level.push(Some(nodes));
            } else {
                number_of_offsets += i64::from(size);
                nodes_by_level.push(None);
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
    /// The per-node start offsets inside the graph data, read on demand.
    ///
    /// Java keeps the `DirectMonotonicReader` itself
    /// (`Lucene99HnswVectorsReader.java:459-470`) and reads one offset per
    /// `seek`. This port used to materialise every offset into a `Vec` sized by
    /// `numberOfOffsets` — a count summed out of the `.vem` — which turned a
    /// corrupt level header into an allocation of arbitrary size. Reading them
    /// the way Java does removes the allocation and the divergence together.
    ///
    /// `None` when the field has no offsets at all, which is the empty-graph
    /// case Java represents with a null `offsetsMeta`.
    offsets: Option<DirectMonotonicReader>,
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
        let offsets = if number_of_offsets > 0 {
            let offsets_data = read_offsets_data(vector_index, entry)?;
            Some(DirectMonotonicReader::new(
                entry.offsets_meta.clone(),
                offsets_data,
            )?)
        } else {
            None
        };
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
        // `level` reaches here from the searcher, which took `num_levels` off
        // disk; Java indexes `nodesByLevel[level]` unchecked and would raise an
        // `ArrayIndexOutOfBoundsException`, which is catchable. A Rust panic is
        // not the same thing, so the bound is checked.
        if level < 0 || level as usize >= self.nodes_by_level.len() {
            return Err(LuceneError::CorruptIndex(format!(
                "level {level} is outside the {} levels of the graph",
                self.nodes_by_level.len()
            )));
        }
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
        let offsets = self.offsets.as_ref().ok_or_else(|| {
            LuceneError::CorruptIndex(
                "the graph has no node offsets, so no node can be sought".to_string(),
            )
        })?;
        // `get_checked` rather than `LongValues::get`: every term of
        // `offset_index` came off disk, and the infallible accessor answers an
        // out-of-range index with a panic. Java's `DirectMonotonicReader.get`
        // raises an `ArrayIndexOutOfBoundsException` or an
        // `UncheckedIOException` here — both catchable.
        let offset = offsets.get_checked(offset_index)?;
        self.data_in.seek(offset)?;
        self.arc_count = self.data_in.read_v_int()? as usize;
        // Java sizes `currentNeighborsBuffer` at exactly `M * 2` and asserts
        // `arcCount <= currentNeighborsBuffer.length`; with assertions off it
        // walks off the end of the array into an
        // `ArrayIndexOutOfBoundsException`. The count is a vint off disk, so it
        // is refused here instead. The bound is Java's `M * 2` rather than this
        // buffer's own length, because the searcher's scratch array is the one
        // sized `maxConn * 2` and it is the smaller of the two.
        let max_arcs = self
            .max_conn
            .checked_mul(2)
            .filter(|arcs| *arcs >= 0)
            .map(|arcs| arcs as usize)
            .unwrap_or(0);
        if self.arc_count > max_arcs.min(self.current_neighbors_buffer.len()) {
            return Err(LuceneError::CorruptIndex(format!(
                "node {target} on level {level} claims {} neighbours, more than the {} a graph \
                 with maxConn {} can have",
                self.arc_count, max_arcs, self.max_conn
            )));
        }
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
            // Java accumulates with plain `int` arithmetic, which wraps; the
            // deltas come off disk, so the sum wraps here too rather than
            // aborting a debug build on bytes Lucene decodes without
            // complaint.
            let mut sum = 0i32;
            for i in 0..self.arc_count {
                sum = sum.wrapping_add(self.current_neighbors_buffer[i]);
                self.current_neighbors_buffer[i] = sum;
                // This is where Java's `assert friendOrd < size` sits, one
                // frame up in `HnswGraphSearcher.searchLevel`. With assertions
                // off Java reaches `FixedBitSet.getAndSet(friendOrd)` and
                // raises an `ArrayIndexOutOfBoundsException`; this refuses the
                // same neighbour, at the boundary where it was read rather than
                // where it is used, so no consumer of this graph can be handed
                // an ordinal it would index an array with.
                if sum < 0 || sum >= self.size {
                    return Err(LuceneError::CorruptIndex(format!(
                        "neighbour {sum} of node {target} on level {level} is outside the {} \
                         nodes of the graph",
                        self.size
                    )));
                }
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
            assert_eq!(super::dist_func_to_ord(similarity), ordinal);
            assert_eq!(super::read_vector_similarity(ordinal).unwrap(), similarity);
        }
    }

    use super::*;
    use crate::codecs::stub::{BufferedUpdates, FieldInfos};
    use crate::codecs::KnnVectorsFormat;
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
    ) -> (OwnedSegmentWriteState, SegmentReadState<'a>) {
        let write_state = OwnedSegmentWriteState::new(
            Arc::new(crate::util::NoOutputInfoStream),
            Arc::clone(dir),
            seg_info.clone(),
            field_infos.clone(),
            seg_updates.clone(),
            Arc::new(DefaultIOContext::default()),
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
