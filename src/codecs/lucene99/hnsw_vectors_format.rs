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
    check_footer_after, check_index_header, checksum_entire_file, retrieve_checksum, write_footer,
    write_index_header,
};
use crate::codecs::hnsw::flat_vectors::{
    DefaultFlatVectorScorer, DocsWithFieldSet, FlatFieldVectorWriter, FlatFieldVectorsWriter,
    FlatVectorsFormat, FlatVectorsReader, FlatVectorsWriter, SharedFlatFieldVectorsWriter,
};
use crate::codecs::hnsw::graph_provider::HnswGraphProvider;
use crate::codecs::knn_vectors::{
    FieldVectorWriter, KnnFieldVectorsWriter, KnnVectorsFormat, KnnVectorsReader, KnnVectorsWriter,
    MergedVectorValues, SorterDocMap,
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
use crate::internal::hppc::IntObjectHashMap;
use crate::search::knn::KnnCollector;
use crate::search::{AcceptDocs, DocIdSetIterator, TaskExecutor, NO_MORE_DOCS};
use crate::store::SharedIndexInput;
use crate::store::{DataInput, IndexInput, IndexOutput};
use crate::util::hnsw::{
    ArrayNodesIterator, CloseableRandomVectorScorerSupplier, ConcurrentHnswMerger,
    DenseNodesIterator, EmptyHnswGraph, HnswBuilder, HnswGraph, HnswGraphBuilder, HnswGraphMerger,
    HnswGraphSearcher, IncrementalHnswGraphMerger, NeighborArray, NodesIterator, OnHeapHnswGraph,
    OrdinalTranslatedKnnCollector, RandomVectorScorer,
};
use crate::util::packed::{DirectMonotonicMeta, DirectMonotonicReader, DirectMonotonicWriter};
use crate::util::quantization::{QuantizedByteVectorValues, ScalarQuantizer};
use crate::util::IOUtils;

// -----------------------------------------------------------------------------
// Format constants
// -----------------------------------------------------------------------------

const NAME: &str = "Lucene99HnswVectorsFormat";
const META_CODEC_NAME: &str = "Lucene99HnswVectorsFormatMeta";
const VECTOR_INDEX_CODEC_NAME: &str = "Lucene99HnswVectorsFormatIndex";
const META_EXTENSION: &str = "vem";
const VECTOR_INDEX_EXTENSION: &str = "vex";
/// The first version of the format: `Lucene99HnswVectorsFormat.VERSION_START`.
pub const VERSION_START: i32 = 0;
/// The version that writes neighbours with group-varint:
/// `Lucene99HnswVectorsFormat.VERSION_GROUPVARINT`.
pub const VERSION_GROUPVARINT: i32 = 1;
/// The version written: `Lucene99HnswVectorsFormat.VERSION_CURRENT`.
pub const VERSION_CURRENT: i32 = VERSION_GROUPVARINT;

/// A maximum configurable maximum max conn.
///
/// NOTE: We eagerly populate `float[MAX_CONN*2]` and `int[MAX_CONN*2]`, so
/// exceptionally large numbers here will use an inordinate amount of heap.
/// `Lucene99HnswVectorsFormat.MAXIMUM_MAX_CONN`.
pub const MAXIMUM_MAX_CONN: i32 = 512;
/// The maximum size of the queue to maintain while searching during graph
/// construction. This maximum value preserves the ratio of the
/// `DEFAULT_BEAM_WIDTH`/`DEFAULT_MAX_CONN` (i.e. `6.25 * 16 = 3200`).
/// `Lucene99HnswVectorsFormat.MAXIMUM_BEAM_WIDTH`.
pub const MAXIMUM_BEAM_WIDTH: i32 = 3200;
/// Default number of maximum connections per node:
/// `Lucene99HnswVectorsFormat.DEFAULT_MAX_CONN`.
pub const DEFAULT_MAX_CONN: i32 = crate::util::hnsw::DEFAULT_M;
/// Default number of the size of the queue maintained while searching
/// during a graph construction: `Lucene99HnswVectorsFormat.DEFAULT_BEAM_WIDTH`.
pub const DEFAULT_BEAM_WIDTH: i32 = crate::util::hnsw::DEFAULT_BEAM_WIDTH;
/// Default to use single-thread merge:
/// `Lucene99HnswVectorsFormat.DEFAULT_NUM_MERGE_WORKER`.
pub const DEFAULT_NUM_MERGE_WORKER: i32 = 1;
/// Minimum estimated search effort (in terms of expected visited nodes)
/// required before building an HNSW graph for a segment; below it, graph
/// construction is skipped for that segment. Default: `100`.
/// `Lucene99HnswVectorsFormat.HNSW_GRAPH_THRESHOLD`.
pub const HNSW_GRAPH_THRESHOLD: i32 = 100;
const DIRECT_MONOTONIC_BLOCK_SHIFT: i32 = 16;
/// How many ordinals an exhaustive search scores in one bulk call.
///
/// Equivalent to `Lucene99HnswVectorsReader.EXHAUSTIVE_BULK_SCORE_ORDS`,
/// which `BaseKnnVectorsFormatTestCase.testSearchWithVisitedLimit` reads.
pub const EXHAUSTIVE_BULK_SCORE_ORDS: usize = 64;

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
#[derive(Debug, Clone)]
pub struct Lucene99HnswVectorsFormat {
    max_conn: i32,
    beam_width: i32,
    num_merge_workers: i32,
    /// The executor every writer of this format merges on, or `None`.
    ///
    /// Equivalent to `Lucene99HnswVectorsFormat.mergeExec`, the
    /// `TaskExecutor` the constructor wraps around the `ExecutorService` it is
    /// given (`Lucene99HnswVectorsFormat.java:291-295`).
    merge_exec: Option<Arc<TaskExecutor>>,
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
            None,
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
            None,
            HNSW_GRAPH_THRESHOLD,
            VERSION_CURRENT,
        )
    }

    /// Constructs a format using the given graph construction parameters.
    ///
    /// Equivalent to `Lucene99HnswVectorsFormat(int maxConn, int beamWidth, int
    /// tinySegmentsThreshold)` (`Lucene99HnswVectorsFormat.java:194-205`):
    /// `this(maxConn, beamWidth, DEFAULT_NUM_MERGE_WORKER, null,
    /// tinySegmentsThreshold, VERSION_CURRENT)`.
    ///
    /// # Panics
    ///
    /// As [`with_params`](Self::with_params).
    pub fn with_max_conn_beam_width_threshold(
        max_conn: i32,
        beam_width: i32,
        tiny_segments_threshold: i32,
    ) -> Self {
        Self::with_params(
            max_conn,
            beam_width,
            DEFAULT_NUM_MERGE_WORKER,
            None,
            tiny_segments_threshold,
            VERSION_CURRENT,
        )
    }

    /// Creates a format whose writers merge with `num_merge_workers` workers,
    /// on `merge_exec` when one is given and otherwise on the merge
    /// scheduler's intra-merge executor.
    ///
    /// Equivalent to `Lucene99HnswVectorsFormat(int, int, int, ExecutorService)`
    /// (`Lucene99HnswVectorsFormat.java:218-221`); an `ExecutorService` is an
    /// [`Executor`](crate::search::Executor) here.
    ///
    /// # Panics
    ///
    /// As [`with_params`](Self::with_params).
    pub fn with_merge_exec(
        max_conn: i32,
        beam_width: i32,
        num_merge_workers: i32,
        merge_exec: Option<Arc<dyn crate::search::Executor>>,
    ) -> Self {
        Self::with_params(
            max_conn,
            beam_width,
            num_merge_workers,
            merge_exec,
            HNSW_GRAPH_THRESHOLD,
            VERSION_CURRENT,
        )
    }

    /// As [`with_merge_exec`](Self::with_merge_exec), with a tiny-segments
    /// threshold.
    ///
    /// Equivalent to `Lucene99HnswVectorsFormat(int, int, int, ExecutorService, int)`
    /// (`Lucene99HnswVectorsFormat.java:237-244`).
    ///
    /// # Panics
    ///
    /// As [`with_params`](Self::with_params).
    pub fn with_merge_exec_and_threshold(
        max_conn: i32,
        beam_width: i32,
        num_merge_workers: i32,
        merge_exec: Option<Arc<dyn crate::search::Executor>>,
        tiny_segments_threshold: i32,
    ) -> Self {
        Self::with_params(
            max_conn,
            beam_width,
            num_merge_workers,
            merge_exec,
            tiny_segments_threshold,
            VERSION_CURRENT,
        )
    }

    /// Creates a format with the given graph construction parameters.
    ///
    /// Equivalent to the package-private
    /// `Lucene99HnswVectorsFormat(int, int, int, ExecutorService, int, int)`
    /// (`Lucene99HnswVectorsFormat.java:261-296`).
    ///
    /// # Panics
    ///
    /// Panics with Java's `IllegalArgumentException` (its class and message,
    /// as the panic text) when `max_conn` or `beam_width` is out of range, or
    /// when `num_merge_workers` is `1` and a `merge_exec` is given.
    pub fn with_params(
        max_conn: i32,
        beam_width: i32,
        num_merge_workers: i32,
        merge_exec: Option<Arc<dyn crate::search::Executor>>,
        tiny_segments_threshold: i32,
        write_version: i32,
    ) -> Self {
        if max_conn <= 0 || max_conn > MAXIMUM_MAX_CONN {
            panic!(
                "java.lang.IllegalArgumentException: maxConn must be positive and less than or equal to {MAXIMUM_MAX_CONN}; maxConn={max_conn}"
            );
        }
        if beam_width <= 0 || beam_width > MAXIMUM_BEAM_WIDTH {
            panic!(
                "java.lang.IllegalArgumentException: beamWidth must be positive and less than or equal to {MAXIMUM_BEAM_WIDTH}; beamWidth={beam_width}"
            );
        }
        if num_merge_workers == 1 && merge_exec.is_some() {
            panic!(
                "java.lang.IllegalArgumentException: No executor service is needed as we'll use single thread to merge"
            );
        }
        Self {
            max_conn,
            beam_width,
            num_merge_workers,
            merge_exec: merge_exec.map(|executor| Arc::new(TaskExecutor::new(executor))),
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
            state,
            self.max_conn,
            self.beam_width,
            self.num_merge_workers,
            self.merge_exec.clone(),
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

/// Which graph merger `Lucene99HnswVectorsWriter.createGraphMerger` builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GraphMergerChoice {
    /// A `ConcurrentHnswMerger`.
    Concurrent,
    /// An `IncrementalHnswGraphMerger`.
    Incremental,
}

/// The selection of `Lucene99HnswVectorsWriter.createGraphMerger`
/// (`Lucene99HnswVectorsWriter.java:628-647`): a `ConcurrentHnswMerger` when
/// the writer has its own `mergeExec`, or when the merge has an intra-merge
/// executor and more than one worker was asked for; otherwise an
/// `IncrementalHnswGraphMerger`.
fn choose_graph_merger(
    merge_exec: Option<&Arc<TaskExecutor>>,
    parallel_merge_task_executor: Option<&Arc<dyn crate::search::Executor>>,
    num_parallel_merge_workers: i32,
) -> GraphMergerChoice {
    if merge_exec.is_some() {
        return GraphMergerChoice::Concurrent;
    }
    if parallel_merge_task_executor.is_some() && num_parallel_merge_workers > 1 {
        GraphMergerChoice::Concurrent
    } else {
        GraphMergerChoice::Incremental
    }
}

/// Writer for the Lucene 9.9 HNSW vector format.
///
/// Equivalent to `org.apache.lucene.codecs.lucene99.Lucene99HnswVectorsWriter`.
///
/// **Divergence from Lucene 10.5.0.** Java's `mergeOneField` returns an
/// `IORunnable` closure that captures the writer itself, and
/// `KnnVectorsWriter.merge` runs it only after every field has been through
/// its first phase (`Lucene99HnswVectorsWriter.java:423-461`). A Rust closure
/// cannot hold the writer while the merge still calls it, so the writer's state
/// lives behind an `Arc<Mutex<_>>` that the writer and its deferred merge work
/// share; every method locks it for its own duration. The work done, and its
/// order, are Java's.
pub struct Lucene99HnswVectorsWriter {
    state: Arc<Mutex<HnswWriterState>>,
}

/// Everything a [`Lucene99HnswVectorsWriter`] holds: the fields of Java's
/// writer, which its deferred merge work reaches as well.
struct HnswWriterState {
    /// The state the writer was created with.
    ///
    /// Equivalent to Lucene's `final SegmentWriteState segmentWriteState`,
    /// held so that the deferred merge work can reopen the flat vectors it
    /// wrote (`Lucene99HnswVectorsWriter.java:499-511`).
    segment_write_state: OwnedSegmentWriteState,
    meta: Option<Box<dyn IndexOutput>>,
    vector_index: Option<Box<dyn IndexOutput>>,
    flat_vector_writer: Box<dyn FlatVectorsWriter>,
    /// The format the flat vectors were written with, reopened for reading by
    /// [`Lucene99HnswVectorsWriter::ensure_flat_reader_open`] once the merge
    /// needs to score them.
    ///
    /// Equivalent to Lucene's `final FlatVectorsFormat flatVectorsFormat`
    /// constructor parameter: the graph writer does not choose the flat format,
    /// its caller does — which is how
    /// `Lucene104HnswScalarQuantizedVectorsFormat` puts the scalar-quantized
    /// format underneath the same graph writer.
    flat_vectors_format: Box<dyn FlatVectorsFormat>,
    /// The reader over the flat vectors this writer just wrote, opened lazily.
    flat_vectors_reader: Option<Box<dyn FlatVectorsReader>>,
    /// Whether [`Lucene99HnswVectorsWriter::ensure_flat_reader_open`] already
    /// closed [`Self::flat_vector_writer`], so `close` does not close it twice.
    flat_writer_closed: bool,
    m: i32,
    beam_width: i32,
    num_merge_workers: i32,
    /// Equivalent to `Lucene99HnswVectorsWriter.mergeExec`.
    merge_exec: Option<Arc<TaskExecutor>>,
    tiny_segments_threshold: i32,
    version: i32,
    /// Equivalent to `Lucene99HnswVectorsWriter.fields`, the
    /// `List<FieldWriter<?>>` the writer keeps for `flush` and
    /// `ramBytesUsed`.
    fields: Vec<Arc<Mutex<dyn HnswFieldWriterOps>>>,
    finished: bool,
}

impl fmt::Debug for Lucene99HnswVectorsWriter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        f.debug_struct("Lucene99HnswVectorsWriter")
            .field("m", &state.m)
            .field("beam_width", &state.beam_width)
            .field("finished", &state.finished)
            .field("fields", &state.fields.len())
            .finish_non_exhaustive()
    }
}

/// `RamUsageEstimator.shallowSizeOfInstance(Lucene99HnswVectorsWriter.class)`
/// (`Lucene99HnswVectorsWriter.java:77-78`) on a 64-bit JVM with compressed
/// oops: a 12-byte header, eight references (`segmentWriteState`, `meta`,
/// `vectorIndex`, `flatVectorWriter`, `flatVectorsFormat`,
/// `flatVectorsReader`, `mergeExec`, `fields`), five `int`s (`M`,
/// `beamWidth`, `numMergeWorkers`, `tinySegmentsThreshold`, `version`) and two
/// `boolean`s (`flatWriterClosed`, `finished`), aligned to 8: 72 bytes, as
/// measured on the JDK 25 oracle.
const HNSW_VECTORS_WRITER_SHALLOW_SIZE: i64 = crate::util::RamUsageEstimator::align_object_size(
    crate::util::RamUsageEstimator::NUM_BYTES_OBJECT_HEADER
        + 8 * crate::util::RamUsageEstimator::NUM_BYTES_OBJECT_REF
        + 5 * 4
        + 2,
);

/// `RamUsageEstimator.shallowSizeOfInstance(FieldWriter.class)`
/// (`Lucene99HnswVectorsWriter.java:677-678`): a 12-byte header, five
/// references (`fieldInfo`, `hnswGraphBuilder`, `flatFieldVectorsWriter`,
/// `infoStream`, `scorerSupplier`) and five `int`s (`lastDocID`, `node`,
/// `graphThreshold`, `M`, `beamWidth`), aligned to 8: 56 bytes, as measured on
/// the JDK 25 oracle.
const HNSW_FIELD_WRITER_SHALLOW_SIZE: i64 = crate::util::RamUsageEstimator::align_object_size(
    crate::util::RamUsageEstimator::NUM_BYTES_OBJECT_HEADER
        + 5 * crate::util::RamUsageEstimator::NUM_BYTES_OBJECT_REF
        + 5 * 4,
);

/// The per-field writer of an HNSW segment: it forwards each vector to the
/// flat field writer and inserts it into the field's graph as it arrives.
///
/// Equivalent to `Lucene99HnswVectorsWriter.FieldWriter<T>`
/// (`Lucene99HnswVectorsWriter.java:675-814`). The graph builder is created
/// lazily once the field holds enough vectors to be worth a graph
/// (`tinySegmentsThreshold`), replaying the vectors buffered so far, and from
/// then on each `addValue` adds its node; so the graph grows, and is counted
/// in [`ram_bytes_used`](KnnFieldVectorsWriter::ram_bytes_used), while the
/// documents are indexed, which is what sets the RAM-triggered flush points.
struct HnswFieldWriter<T> {
    field_info: FieldInfo,
    /// Only created when needed.
    hnsw_graph_builder: Option<HnswGraphBuilder>,
    last_doc_id: i32,
    node: i32,
    flat_field_vectors_writer: SharedFlatFieldVectorsWriter<T>,
    graph_threshold: i32,
    m: i32,
    beam_width: i32,
    info_stream: Arc<dyn crate::util::InfoStream>,
    /// Scores over a live view of the flat field writer's growing vector list.
    scorer_supplier: Box<dyn crate::util::hnsw::RandomVectorScorerSupplier>,
}

impl<T> HnswFieldWriter<T> {
    /// Equivalent to the `FieldWriter` constructor
    /// (`Lucene99HnswVectorsWriter.java:725-754`), which receives the scorer
    /// supplier built over `flatFieldVectorsWriter.asKnnVectorValues(...)` and
    /// creates the graph builder at once when the tiny-segment optimisation is
    /// disabled.
    #[allow(clippy::too_many_arguments)]
    fn new(
        scorer_supplier: Box<dyn crate::util::hnsw::RandomVectorScorerSupplier>,
        flat_field_vectors_writer: SharedFlatFieldVectorsWriter<T>,
        field_info: &FieldInfo,
        m: i32,
        beam_width: i32,
        info_stream: Arc<dyn crate::util::InfoStream>,
        tiny_segments_threshold: i32,
    ) -> Result<Self> {
        let mut writer = Self {
            field_info: field_info.clone(),
            hnsw_graph_builder: None,
            last_doc_id: -1,
            node: 0,
            flat_field_vectors_writer,
            graph_threshold: tiny_segments_threshold,
            m,
            beam_width,
            info_stream,
            scorer_supplier,
        };
        if writer.graph_threshold <= 0 {
            // Initialize graph builder if optimization is disabled.
            writer.initialize_graph_builder()?;
        }
        Ok(writer)
    }

    /// Equivalent to `FieldWriter.initializeGraphBuilder()`
    /// (`Lucene99HnswVectorsWriter.java:756-763`): a builder over a graph of
    /// unknown size, seeded with `HnswGraphBuilder.randSeed`.
    fn initialize_graph_builder(&mut self) -> Result<()> {
        if self.hnsw_graph_builder.is_some() {
            return Ok(());
        }
        let mut builder = HnswGraphBuilder::create(
            &*self.scorer_supplier,
            self.m,
            self.beam_width,
            HnswGraphBuilder::rand_seed(),
            -1,
        )?;
        builder.set_info_stream(Arc::clone(&self.info_stream));
        self.hnsw_graph_builder = Some(builder);
        Ok(())
    }

    /// Locks the flat field writer this writer forwards to.
    fn flat(&self) -> std::sync::MutexGuard<'_, dyn FlatFieldVectorsWriter<T> + 'static> {
        self.flat_field_vectors_writer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl<T: Clone> HnswFieldWriter<T> {
    /// Equivalent to `FieldWriter.replayBufferedVectors()`
    /// (`Lucene99HnswVectorsWriter.java:765-769`).
    fn replay_buffered_vectors(&mut self) -> Result<()> {
        let count = self.flat().vector_count();
        if let Some(builder) = self.hnsw_graph_builder.as_mut() {
            for i in 0..count {
                builder.add_graph_node(i as i32)?;
            }
        }
        Ok(())
    }
}

impl<T: Clone + Send + 'static> HnswFieldWriter<T> {
    /// Equivalent to `FieldWriter.addValue(int, T)`
    /// (`Lucene99HnswVectorsWriter.java:771-790`).
    fn add_value(&mut self, doc_id: i32, vector_value: T) -> Result<()> {
        if doc_id == self.last_doc_id {
            return Err(LuceneError::illegal_argument(format!(
                "VectorValuesField \"{}\" appears more than once in this document (only one \
                 value is allowed per field)",
                self.field_info.name
            )));
        }
        self.flat().add_value(doc_id, vector_value)?;
        // Check if we need to initialize graph builder for tiny segment
        // optimization.
        if self.hnsw_graph_builder.is_none()
            && should_create_graph(self.graph_threshold, self.node + 1)
        {
            self.initialize_graph_builder()?;
            self.replay_buffered_vectors()?;
        } else if let Some(builder) = self.hnsw_graph_builder.as_mut() {
            // Graph builder is active, add to graph.
            builder.add_graph_node(self.node)?;
        }
        self.node += 1;
        self.last_doc_id = doc_id;
        Ok(())
    }

    /// Equivalent to `FieldWriter.ramBytesUsed()`
    /// (`Lucene99HnswVectorsWriter.java:806-813`): the shallow size, the flat
    /// field writer and, once it exists, the graph being built.
    fn ram_bytes_used(&self) -> i64 {
        let mut total = HNSW_FIELD_WRITER_SHALLOW_SIZE + self.flat().ram_bytes_used();
        if let Some(builder) = self.hnsw_graph_builder.as_ref() {
            total += crate::util::Accountable::ram_bytes_used(builder.get_graph());
        }
        total
    }
}

/// What [`HnswWriterState`] needs from each of its field writers, whatever
/// their element type: Java reaches `FieldWriter<?>` through its wildcard.
trait HnswFieldWriterOps: Send {
    /// `FieldWriter.fieldInfo`.
    fn field_info(&self) -> &FieldInfo;
    /// `FieldWriter.getDocsWithFieldSet()`, copied out of the flat writer's
    /// lock.
    fn docs_with_field_set(&self) -> DocsWithFieldSet;
    /// `FieldWriter.getGraph()` (`Lucene99HnswVectorsWriter.java:796-804`):
    /// the completed graph, or `None` when no graph was built or no vector
    /// was added.
    fn get_graph(&mut self) -> Result<Option<&OnHeapHnswGraph>>;
    /// `FieldWriter.ramBytesUsed()`.
    fn field_ram_bytes_used(&self) -> i64;
}

impl<T: Clone + Send + 'static> HnswFieldWriterOps for HnswFieldWriter<T> {
    fn field_info(&self) -> &FieldInfo {
        &self.field_info
    }

    fn docs_with_field_set(&self) -> DocsWithFieldSet {
        self.flat().docs_with_field_set().into_owned()
    }

    fn get_graph(&mut self) -> Result<Option<&OnHeapHnswGraph>> {
        debug_assert!(self.flat().is_finished());
        let node = self.node;
        match self.hnsw_graph_builder.as_mut() {
            Some(builder) if node > 0 => Ok(Some(builder.get_completed_graph()?)),
            // No graph.
            _ => Ok(None),
        }
    }

    fn field_ram_bytes_used(&self) -> i64 {
        HnswFieldWriter::ram_bytes_used(self)
    }
}

/// The handle [`KnnVectorsWriter::add_field`] returns: the same
/// [`HnswFieldWriter`] the writer keeps in its field list, as Java returns the
/// `FieldWriter` it adds to `fields`.
struct HnswFieldWriterHandle<T>(Arc<Mutex<HnswFieldWriter<T>>>);

impl<T: Clone + Send + 'static> KnnFieldVectorsWriter<T> for HnswFieldWriterHandle<T> {
    fn add_value(&mut self, doc_id: i32, vector_value: T) -> Result<()> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .add_value(doc_id, vector_value)
    }

    /// Java's `FieldWriter.copyValue` throws `UnsupportedOperationException`
    /// and is never called: the flat field writer copies the values it keeps.
    /// The vector is already owned here, so it is returned unchanged.
    fn copy_value(&self, vector_value: T) -> T {
        vector_value
    }

    fn ram_bytes_used(&self) -> i64 {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .ram_bytes_used()
    }
}

/// Java's `FloatVectorValues.fromFloats(flatFieldVectorsWriter.getVectors(),
/// dim)` as `FlatFieldVectorsWriter.asKnnVectorValues` builds it for the HNSW
/// field writer (`FlatFieldVectorsWriter.java:52-56`): a view over the very
/// list the flat field writer appends to, so the graph builder scores vectors
/// added after the view was made.
#[derive(Clone)]
struct LiveFloatVectorValues {
    writer: SharedFlatFieldVectorsWriter<Vec<f32>>,
    dimension: i32,
}

impl KnnVectorValues for LiveFloatVectorValues {
    fn dimension(&self) -> i32 {
        self.dimension
    }

    fn size(&self) -> i32 {
        self.writer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .vector_count() as i32
    }

    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        Ok(Box::new(self.clone()))
    }

    fn encoding(&self) -> VectorEncoding {
        VectorEncoding::FLOAT32
    }

    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
        Ok(Box::new(
            crate::index::vector_values::DenseDocIndexIterator::new(self.size()),
        ))
    }
}

impl FloatVectorValues for LiveFloatVectorValues {
    /// `vectors.get(ord)`, which raises `IndexOutOfBoundsException` outside
    /// the list.
    fn vector_value(&self, ord: i32) -> Result<Vec<f32>> {
        let writer = self
            .writer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        usize::try_from(ord)
            .ok()
            .and_then(|ord| writer.vector_at(ord))
            .ok_or_else(|| list_index_out_of_bounds(ord, writer.vector_count()))
    }

    /// Java's `fromFloats` returns `this` from `copy()`; the clone shares the
    /// same live list.
    fn copy_float(&self) -> Result<Box<dyn FloatVectorValues>> {
        Ok(Box::new(self.clone()))
    }
}

/// [`LiveFloatVectorValues`] for byte vectors: `ByteVectorValues.fromBytes`.
#[derive(Clone)]
struct LiveByteVectorValues {
    writer: SharedFlatFieldVectorsWriter<Vec<u8>>,
    dimension: i32,
}

impl KnnVectorValues for LiveByteVectorValues {
    fn dimension(&self) -> i32 {
        self.dimension
    }

    fn size(&self) -> i32 {
        self.writer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .vector_count() as i32
    }

    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        Ok(Box::new(self.clone()))
    }

    fn encoding(&self) -> VectorEncoding {
        VectorEncoding::BYTE
    }

    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
        Ok(Box::new(
            crate::index::vector_values::DenseDocIndexIterator::new(self.size()),
        ))
    }
}

impl ByteVectorValues for LiveByteVectorValues {
    fn vector_value(&self, ord: i32) -> Result<Vec<u8>> {
        let writer = self
            .writer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        usize::try_from(ord)
            .ok()
            .and_then(|ord| writer.vector_at(ord))
            .ok_or_else(|| list_index_out_of_bounds(ord, writer.vector_count()))
    }

    fn copy_byte(&self) -> Result<Box<dyn ByteVectorValues>> {
        Ok(Box::new(self.clone()))
    }
}

/// The `IndexOutOfBoundsException` `ArrayList.get(index)` raises.
fn list_index_out_of_bounds(index: i32, size: usize) -> LuceneError {
    LuceneError::unchecked(
        "java.lang.IndexOutOfBoundsException",
        format!("Index {index} out of bounds for length {size}"),
    )
}

impl Lucene99HnswVectorsWriter {
    /// Creates a new HNSW vector writer that stores its vectors with the
    /// default Lucene 9.9 flat format.
    pub fn new(
        state: &OwnedSegmentWriteState,
        m: i32,
        beam_width: i32,
        num_merge_workers: i32,
        merge_exec: Option<Arc<TaskExecutor>>,
        tiny_segments_threshold: i32,
        version: i32,
    ) -> Result<Self> {
        let flat_vectors_format = Lucene99FlatVectorsFormat::new(DefaultFlatVectorScorer::INSTANCE);
        let flat_vector_writer = flat_vectors_format.fields_writer_flat(state)?;
        Self::with_flat_format(
            state,
            m,
            beam_width,
            num_merge_workers,
            merge_exec,
            tiny_segments_threshold,
            version,
            Box::new(flat_vectors_format),
            flat_vector_writer,
        )
    }

    /// Creates a new HNSW vector writer over a caller-supplied flat format and
    /// its writer.
    ///
    /// Equivalent to Lucene's sole constructor
    /// `Lucene99HnswVectorsWriter(SegmentWriteState, int, int, FlatVectorsFormat,
    /// FlatVectorsWriter, int, TaskExecutor, int)`, which is what lets a
    /// quantized flat format share this graph writer.
    #[allow(clippy::too_many_arguments)]
    pub fn with_flat_format(
        state: &OwnedSegmentWriteState,
        m: i32,
        beam_width: i32,
        num_merge_workers: i32,
        merge_exec: Option<Arc<TaskExecutor>>,
        tiny_segments_threshold: i32,
        version: i32,
        flat_vectors_format: Box<dyn FlatVectorsFormat>,
        flat_vector_writer: Box<dyn FlatVectorsWriter>,
    ) -> Result<Self> {
        Ok(Self {
            state: Arc::new(Mutex::new(HnswWriterState::new(
                state,
                m,
                beam_width,
                num_merge_workers,
                merge_exec,
                tiny_segments_threshold,
                version,
                flat_vectors_format,
                flat_vector_writer,
            )?)),
        })
    }

    /// Locks the writer's state for one operation.
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, HnswWriterState>> {
        Ok(self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner))
    }
}

impl HnswWriterState {
    /// Opens the writer's two files and writes their headers.
    ///
    /// The body of Lucene's constructor.
    #[allow(clippy::too_many_arguments)]
    fn new(
        owned_state: &OwnedSegmentWriteState,
        m: i32,
        beam_width: i32,
        num_merge_workers: i32,
        merge_exec: Option<Arc<TaskExecutor>>,
        tiny_segments_threshold: i32,
        version: i32,
        flat_vectors_format: Box<dyn FlatVectorsFormat>,
        flat_vector_writer: Box<dyn FlatVectorsWriter>,
    ) -> Result<Self> {
        let state = &owned_state.borrow();
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

        // Both outputs are created inside the `try` block; when it fails,
        // `IOUtils.closeWhileHandlingException(this)` runs `close()`, which
        // is `IOUtils.close(meta, vectorIndex, flatVectorWriter,
        // flatVectorsReader)` with no flat reader open yet
        // (`Lucene99HnswVectorsWriter.java:167-190, 650-656`).
        let mut flat_vector_writer = flat_vector_writer;
        let mut meta_opt: Option<Box<dyn IndexOutput>> = None;
        let mut vector_index_opt: Option<Box<dyn IndexOutput>> = None;
        let result = (|| -> Result<()> {
            let meta = meta_opt.insert(
                state
                    .directory
                    .create_output(&meta_file_name, state.context)?,
            );
            let vector_index = vector_index_opt.insert(
                state
                    .directory
                    .create_output(&vector_index_file_name, state.context)?,
            );
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
            Ok(())
        })();

        if let Err(error) = result {
            IOUtils::close_while_handling_exception([Some(|| {
                IOUtils::close([
                    meta_opt
                        .take()
                        .and_then(|mut o| IOUtils::closeable(move || o.close())),
                    vector_index_opt
                        .take()
                        .and_then(|mut o| IOUtils::closeable(move || o.close())),
                    IOUtils::closeable(|| flat_vector_writer.close()),
                ])
            })])?;
            return Err(error);
        }
        Ok(Self {
            segment_write_state: owned_state.with_new_suffix(owned_state.segment_suffix.clone()),
            meta: meta_opt,
            vector_index: vector_index_opt,
            flat_vector_writer,
            flat_vectors_format,
            flat_vectors_reader: None,
            flat_writer_closed: false,
            m,
            beam_width,
            num_merge_workers,
            merge_exec,
            tiny_segments_threshold,
            version,
            fields: Vec::new(),
            finished: false,
        })
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
        let meta = self
            .meta
            .as_mut()
            .expect("INVARIANT: the writer holds its outputs for its whole life");
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

            let vector_index = self
                .vector_index
                .as_mut()
                .expect("INVARIANT: the writer holds its outputs for its whole life");
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
            let vector_index = self
                .vector_index
                .as_ref()
                .expect("INVARIANT: the writer holds its outputs for its whole life");
            vector_index.file_pointer()
        };
        let (levels, graph_level_node_offsets) =
            self.reconstruct_and_write_graph(graph, &new_to_old_map, &old_to_new_map)?;
        let vector_index_length = {
            let vector_index = self
                .vector_index
                .as_ref()
                .expect("INVARIANT: the writer holds its outputs for its whole life");
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
        let mut nnodes: Vec<i32> = (0..size)
            .map(|i| old_to_new_map[neighbors.node(i as i32) as usize])
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
        let vector_index = self
            .vector_index
            .as_mut()
            .expect("INVARIANT: the writer holds its outputs for its whole life");
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
    /// Returns [`LuceneErrorKind::IllegalState`](crate::error::LuceneErrorKind::IllegalState) when this writer is already
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
            .expect("INVARIANT: the writer holds its outputs for its whole life")
            .file_pointer();
        let mut merger =
            // `Lucene99HnswVectorsWriter.java:474-480`.
            self.create_graph_merger(
                field_info,
                scorer_supplier,
                merge_state.intra_merge_task_executor.as_ref(),
                self.num_merge_workers,
            );
        for i in 0..merge_state.live_docs.len() {
            if !MergedVectorValues::has_vector_values(&merge_state.field_infos[i], &field_info.name)
            {
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
        let graph = merger.merge(
            vector_values,
            Arc::clone(&merge_state.info_stream),
            total_vector_count,
        )?;
        let vector_index_node_offsets = self.write_graph(&graph)?;
        let vector_index_length = self
            .vector_index
            .as_ref()
            .expect("INVARIANT: the writer holds its outputs for its whole life")
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
    /// `parallel_merge_task_executor` is the merge's
    /// `MergeState.intraMergeTaskExecutor`, `None` where Java passes `null`
    /// (`Lucene99HnswVectorsWriter.java:477-479`); see
    /// [`choose_graph_merger`] for the selection.
    fn create_graph_merger(
        &self,
        field_info: &FieldInfo,
        scorer_supplier: Arc<dyn crate::util::hnsw::RandomVectorScorerSupplier>,
        parallel_merge_task_executor: Option<&Arc<dyn crate::search::Executor>>,
        num_parallel_merge_workers: i32,
    ) -> Box<dyn HnswGraphMerger> {
        if choose_graph_merger(
            self.merge_exec.as_ref(),
            parallel_merge_task_executor,
            num_parallel_merge_workers,
        ) == GraphMergerChoice::Concurrent
        {
            // `mergeExec` with `numMergeWorkers` when the writer has one, and
            // otherwise a `TaskExecutor` over the merge's intra-merge executor
            // with `numParallelMergeWorkers` (`Lucene99HnswVectorsWriter.java:628-647`).
            let (task_executor, num_worker) = match (&self.merge_exec, parallel_merge_task_executor)
            {
                (Some(merge_exec), _) => (Arc::clone(merge_exec), self.num_merge_workers),
                (None, Some(executor)) => (
                    Arc::new(TaskExecutor::new(Arc::clone(executor))),
                    num_parallel_merge_workers,
                ),
                (None, None) => unreachable!("INVARIANT: choose_graph_merger needs an executor"),
            };
            Box::new(ConcurrentHnswMerger::new(
                field_info.clone(),
                scorer_supplier,
                self.m,
                self.beam_width,
                task_executor,
                num_worker,
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
    /// `org.apache.lucene.codecs.lucene99.Lucene99HnswVectorsWriter#ensureFlatReaderOpen`
    /// (`Lucene99HnswVectorsWriter.java:499-511`).
    fn ensure_flat_reader_open(&mut self) -> Result<()> {
        if self.flat_vectors_reader.is_none() {
            self.flat_vector_writer.finish()?;
            self.flat_vector_writer.close()?;
            self.flat_writer_closed = true;
            let state = &self.segment_write_state;
            let read_state = SegmentReadState::with_suffix(
                &*state.directory,
                &state.segment_info,
                &state.field_infos,
                &*state.context,
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
            let mut sorted_nodes = graph.get_sorted_nodes(level)?;
            let mut level_offsets = Vec::with_capacity(sorted_nodes.size() as usize);
            while sorted_nodes.has_next() {
                let node = sorted_nodes.next_int();
                let neighbors = graph.get_neighbors(level, node)?;
                let vector_index = self
                    .vector_index
                    .as_mut()
                    .expect("INVARIANT: the writer holds its outputs for its whole life");
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

impl HnswWriterState {
    /// Equivalent to `Lucene99HnswVectorsWriter.addField(FieldInfo)`
    /// (`Lucene99HnswVectorsWriter.java:194-207`): the flat writer adds the
    /// field, and a [`HnswFieldWriter`] over its field writer, scoring with
    /// the flat writer's scorer (`FieldWriter.create`,
    /// `Lucene99HnswVectorsWriter.java:690-723`), joins the field list and is
    /// returned.
    fn add_field(&mut self, field_info: &FieldInfo) -> Result<FieldVectorWriter> {
        let flat_field_vectors_writer =
            FlatVectorsWriter::add_field(self.flat_vector_writer.as_mut(), field_info)?;
        let info_stream = Arc::clone(&self.segment_write_state.info_stream);
        let scorer = self.flat_vector_writer.get_flat_vector_scorer();
        Ok(match flat_field_vectors_writer {
            FlatFieldVectorWriter::Float(writer) => {
                let supplier = scorer.get_random_vector_scorer_supplier_float(
                    field_info.vector_similarity_function,
                    Box::new(LiveFloatVectorValues {
                        writer: Arc::clone(&writer),
                        dimension: field_info.vector_dimension,
                    }),
                )?;
                let field = Arc::new(Mutex::new(HnswFieldWriter::new(
                    supplier,
                    writer,
                    field_info,
                    self.m,
                    self.beam_width,
                    info_stream,
                    self.tiny_segments_threshold,
                )?));
                self.fields.push(field.clone());
                FieldVectorWriter::Float(Box::new(HnswFieldWriterHandle(field)))
            }
            FlatFieldVectorWriter::Byte(writer) => {
                let supplier = scorer.get_random_vector_scorer_supplier_byte(
                    field_info.vector_similarity_function,
                    Box::new(LiveByteVectorValues {
                        writer: Arc::clone(&writer),
                        dimension: field_info.vector_dimension,
                    }),
                )?;
                let field = Arc::new(Mutex::new(HnswFieldWriter::new(
                    supplier,
                    writer,
                    field_info,
                    self.m,
                    self.beam_width,
                    info_stream,
                    self.tiny_segments_threshold,
                )?));
                self.fields.push(field.clone());
                FieldVectorWriter::Byte(Box::new(HnswFieldWriterHandle(field)))
            }
        })
    }

    /// Equivalent to `Lucene99HnswVectorsWriter.flush(int, Sorter.DocMap)`
    /// (`Lucene99HnswVectorsWriter.java:209-219`): the flat writer writes every
    /// field's vectors, then each field's graph — built as the values arrived
    /// — and metadata are written.
    fn flush(&mut self, max_doc: i32, sort_map: Option<&SorterDocMap>) -> Result<()> {
        self.flat_vector_writer.flush(max_doc, sort_map)?;
        for i in 0..self.fields.len() {
            let field = Arc::clone(&self.fields[i]);
            let mut field = field
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let info = field.field_info().clone();
            let docs_with_field_set = field.docs_with_field_set();
            let graph = field.get_graph()?;
            match sort_map {
                None => self.write_field(&info, docs_with_field_set.cardinality(), graph)?,
                Some(sort_map) => {
                    let ord_to_doc = build_ord_to_doc(&docs_with_field_set)?;
                    self.write_sorting_field(&info, &ord_to_doc, graph, sort_map)?
                }
            }
        }
        Ok(())
    }

    /// Writes one field's graph and metadata.
    ///
    /// Equivalent to `Lucene99HnswVectorsWriter.writeField(FieldWriter)`
    /// (`Lucene99HnswVectorsWriter.java:251-265`), with the field's vector
    /// count and its graph, `None` when the field has none.
    fn write_field(
        &mut self,
        info: &FieldInfo,
        count: i32,
        graph: Option<&OnHeapHnswGraph>,
    ) -> Result<()> {
        let vector_index_offset = self
            .vector_index
            .as_ref()
            .expect("INVARIANT: the writer holds its outputs for its whole life")
            .file_pointer();
        let graph_level_offsets = match graph {
            Some(graph) => self.write_graph(graph)?,
            None => Vec::new(),
        };
        let vector_index_length = self
            .vector_index
            .as_ref()
            .expect("INVARIANT: the writer holds its outputs for its whole life")
            .file_pointer()
            - vector_index_offset;
        self.write_meta(
            info,
            vector_index_offset,
            vector_index_length,
            count,
            graph,
            &graph_level_offsets,
        )
    }

    /// Writes the end-of-fields marker and both footers.
    ///
    /// Equivalent to `Lucene99HnswVectorsWriter.finish()`
    /// (`Lucene99HnswVectorsWriter.java:221-238`): the flat writer is finished
    /// here only when a merge's deferred work has not already finished and
    /// closed it to reopen its vectors.
    fn finish(&mut self) -> Result<()> {
        if self.finished {
            // Lucene99HnswVectorsWriter.java:223-225.
            return Err(LuceneError::illegal_state("already finished".to_string()));
        }
        self.finished = true;
        if !self.flat_writer_closed {
            self.flat_vector_writer.finish()?;
        }
        if let Some(meta) = self.meta.as_mut() {
            meta.write_int(-1)?;
            write_footer(meta.as_mut())?;
        }
        if let Some(vector_index) = self.vector_index.as_mut() {
            write_footer(vector_index.as_mut())?;
        }
        Ok(())
    }

    /// The second phase of merging one field, which
    /// [`HnswDeferredMerge`] runs once every field has been through the first.
    ///
    /// Equivalent to the body of the `IORunnable` that
    /// `Lucene99HnswVectorsWriter.mergeOneField` returns
    /// (`Lucene99HnswVectorsWriter.java:425-460`): reopen the flat vectors just
    /// written, then either build the field's graph and write it with its
    /// metadata, or — when there are no vectors or too few to be worth a graph
    /// — write a metadata entry with no graph.
    ///
    /// # Errors
    ///
    /// Returns whatever reopening the flat vectors, scoring them, building the
    /// graph or writing it raises.
    fn merge_one_field_deferred(
        &mut self,
        field_info: &FieldInfo,
        merge_state: &MergeState,
    ) -> Result<()> {
        // Lazily finish flat writer and open a reader for the written segment
        self.ensure_flat_reader_open()?;
        let flat_vectors_reader = self
            .flat_vectors_reader
            .as_ref()
            .expect("ensure_flat_reader_open opened the reader");
        // Get the vector values and scorer supplier from the written segment
        let vector_values: Option<Box<dyn crate::index::vector_values::KnnVectorValues>> =
            match field_info.vector_encoding {
                VectorEncoding::BYTE => flat_vectors_reader
                    .get_byte_vector_values(&field_info.name)?
                    .map(|values| values as Box<dyn crate::index::vector_values::KnnVectorValues>),
                VectorEncoding::FLOAT32 => flat_vectors_reader
                    .get_float_vector_values(&field_info.name)?
                    .map(|values| values as Box<dyn crate::index::vector_values::KnnVectorValues>),
            };
        let total_vector_count = vector_values.as_ref().map_or(0, |values| values.size());
        if let Some(vector_values) = vector_values.filter(|_| {
            total_vector_count > 0
                && should_create_graph(self.tiny_segments_threshold, total_vector_count)
        }) {
            let quantized_vectors_reader = match field_info.vector_encoding {
                VectorEncoding::FLOAT32 => flat_vectors_reader.as_quantized_vectors_reader(),
                VectorEncoding::BYTE => None,
            };
            if let Some(quantized_vectors_reader) = quantized_vectors_reader {
                let scorer_supplier: Arc<dyn CloseableRandomVectorScorerSupplier> = Arc::from(
                    quantized_vectors_reader
                        .get_random_vector_scorer_supplier_for_merge(
                            field_info,
                            &self.segment_write_state.borrow(),
                        )?
                        // A `null` supplier dereferenced: the JVM's
                        // `NullPointerException`.
                        .ok_or_else(LuceneError::null_pointer)?,
                );
                let built = self.build_and_write_graph(
                    field_info,
                    merge_state,
                    vector_values.as_ref(),
                    Arc::clone(&scorer_supplier)
                        as Arc<dyn crate::util::hnsw::RandomVectorScorerSupplier>,
                    total_vector_count,
                );
                let close_supplier = || scorer_supplier.close();
                match built {
                    // `IOUtils.closeWhileSuppressingExceptions(t,
                    // scorerSupplier)`, then `throw t`
                    // (`Lucene99HnswVectorsWriter.java:441-447`).
                    Err(t) => {
                        return Err(IOUtils::close_while_suppressing_exceptions(
                            t,
                            [Some(close_supplier)],
                        ))
                    }
                    // `IOUtils.close(scorerSupplier)` (`:448`).
                    Ok(()) => IOUtils::close([Some(close_supplier)])?,
                }
            } else {
                let scorer = flat_vectors_reader.get_flat_vector_scorer(&field_info.name)?;
                let scorer_supplier: Box<dyn crate::util::hnsw::RandomVectorScorerSupplier> =
                    match field_info.vector_encoding {
                        VectorEncoding::FLOAT32 => scorer.get_random_vector_scorer_supplier_float(
                            field_info.vector_similarity_function,
                            flat_vectors_reader
                                .get_float_vector_values(&field_info.name)?
                                .ok_or_else(|| missing_vectors(field_info))?,
                        )?,
                        VectorEncoding::BYTE => scorer.get_random_vector_scorer_supplier_byte(
                            field_info.vector_similarity_function,
                            flat_vectors_reader
                                .get_byte_vector_values(&field_info.name)?
                                .ok_or_else(|| missing_vectors(field_info))?,
                        )?,
                    };
                self.build_and_write_graph(
                    field_info,
                    merge_state,
                    vector_values.as_ref(),
                    Arc::from(scorer_supplier),
                    total_vector_count,
                )?;
            }
        } else {
            let vector_index_offset = self
                .vector_index
                .as_ref()
                .expect("INVARIANT: the writer holds its outputs for its whole life")
                .file_pointer();
            self.write_meta(
                field_info,
                vector_index_offset,
                0,
                total_vector_count,
                None,
                &[],
            )?;
        }
        Ok(())
    }

    /// Equivalent to `Lucene99HnswVectorsWriter.ramBytesUsed()`
    /// (`Lucene99HnswVectorsWriter.java:240-248`): the writer's shallow size
    /// plus every field writer's footprint (shallow size, flat field writer
    /// and the graph built so far). As in Java, the flat writer itself is not
    /// counted, and the fields stay counted after `flush`.
    fn ram_bytes_used(&self) -> i64 {
        let mut total = HNSW_VECTORS_WRITER_SHALLOW_SIZE;
        for field in &self.fields {
            // The field tracks the delegate field usage.
            total += field
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .field_ram_bytes_used();
        }
        total
    }

    /// Equivalent to `Lucene99HnswVectorsWriter.close()`
    /// (`Lucene99HnswVectorsWriter.java:648-656`): `IOUtils.close(meta,
    /// vectorIndex, flatVectorWriter, flatVectorsReader)`, without the flat
    /// writer once `ensureFlatReaderOpen` has closed it.
    fn close(&mut self) -> Result<()> {
        let flat_writer_closed = self.flat_writer_closed;
        IOUtils::close([
            self.meta
                .as_mut()
                .and_then(|o| IOUtils::closeable(move || o.close())),
            self.vector_index
                .as_mut()
                .and_then(|o| IOUtils::closeable(move || o.close())),
            (!flat_writer_closed)
                .then_some(&mut self.flat_vector_writer)
                .and_then(|w| IOUtils::closeable(move || w.close())),
            self.flat_vectors_reader
                .as_mut()
                .and_then(|r| IOUtils::closeable(move || r.close())),
        ])
    }
}

/// The error for flat vectors that vanished between counting them and scoring
/// them, where Java dereferences `null`: the JVM's `NullPointerException`.
fn missing_vectors(_field_info: &FieldInfo) -> LuceneError {
    LuceneError::null_pointer()
}

impl KnnVectorsWriter for Lucene99HnswVectorsWriter {
    fn add_field(&mut self, field_info: &FieldInfo) -> Result<FieldVectorWriter> {
        self.lock()?.add_field(field_info)
    }

    fn flush(&mut self, max_doc: i32, sort_map: Option<&SorterDocMap>) -> Result<()> {
        self.lock()?.flush(max_doc, sort_map)
    }

    fn finish(&mut self) -> Result<()> {
        self.lock()?.finish()
    }

    /// Writes the field's flat vectors now and returns the work that builds
    /// its graph, to be run once every field has been merged.
    ///
    /// Equivalent to `Lucene99HnswVectorsWriter.mergeOneField(FieldInfo, MergeState)`
    /// (`Lucene99HnswVectorsWriter.java:422-461`), which always returns the
    /// deferred `IORunnable`; [`HnswDeferredMerge`] is that runnable.
    fn merge_one_field<'m>(
        &mut self,
        field_info: &FieldInfo,
        merge_state: &'m MergeState,
    ) -> Result<Option<Box<dyn crate::codecs::knn_vectors::IORunnable + 'm>>> {
        self.lock()?
            .flat_vector_writer
            .merge_one_flat_vector_field(field_info, merge_state)?;
        Ok(Some(Box::new(HnswDeferredMerge {
            state: Arc::clone(&self.state),
            field_info: field_info.clone(),
            merge_state,
        })))
    }

    fn ram_bytes_used(&self) -> i64 {
        match self.state.lock() {
            Ok(state) => state.ram_bytes_used(),
            Err(poisoned) => poisoned.into_inner().ram_bytes_used(),
        }
    }

    fn close(&mut self) -> Result<()> {
        self.lock()?.close()
    }
}

/// The deferred second phase of merging one field into an HNSW segment.
///
/// Equivalent to the lambda `Lucene99HnswVectorsWriter.mergeOneField` returns
/// (`Lucene99HnswVectorsWriter.java:425-460`), which captures the writer, the
/// field and the merge state; this holds the writer's shared state instead of
/// the writer.
struct HnswDeferredMerge<'m> {
    state: Arc<Mutex<HnswWriterState>>,
    field_info: FieldInfo,
    merge_state: &'m MergeState,
}

impl crate::codecs::knn_vectors::IORunnable for HnswDeferredMerge<'_> {
    fn run(&mut self) -> Result<()> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .merge_one_field_deferred(&self.field_info, self.merge_state)
    }
}

fn write_neighbors(
    out: &mut dyn IndexOutput,
    neighbors: &NeighborArray,
    scratch_capacity: usize,
    version: i32,
) -> Result<()> {
    let size = neighbors.size() as usize;
    let mut scratch = vec![0; scratch_capacity.max(size)];
    neighbors.copy_nodes(0, &mut scratch[..size]);
    scratch.truncate(size);
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

/// `VectorEncoding.values()[ordinal]`, `None` outside the table.
fn read_vector_encoding(ordinal: i32) -> Option<VectorEncoding> {
    match ordinal {
        0 => Some(VectorEncoding::BYTE),
        1 => Some(VectorEncoding::FLOAT32),
        _ => None,
    }
}

/// Reads a vector encoding from the metadata stream.
///
/// Equivalent to `Lucene99HnswVectorsReader.readVectorEncoding(DataInput)`
/// (`Lucene99HnswVectorsReader.java:240-246`), which the flat and scalar
/// quantized readers import as well.
///
/// # Errors
///
/// Returns `CorruptIndexException("Invalid vector encoding id: " +
/// encodingId, input)` for an id outside `VectorEncoding.values()`.
pub(crate) fn read_vector_encoding_input(input: &mut dyn DataInput) -> Result<VectorEncoding> {
    let encoding_id = input.read_int()?;
    read_vector_encoding(encoding_id).ok_or_else(|| {
        LuceneError::corrupt_index(
            format!("Invalid vector encoding id: {encoding_id}"),
            input.to_java_string(),
        )
    })
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
/// Returns [`LuceneErrorKind::IllegalArgument`](crate::error::LuceneErrorKind::IllegalArgument) when the ordinal names no
/// similarity function, which is the `IllegalArgumentException` Java raises.
pub(crate) fn read_similarity_function(
    input: &mut dyn DataInput,
) -> Result<VectorSimilarityFunction> {
    let i = input.read_int()?;
    read_vector_similarity(i)
        .ok_or_else(|| LuceneError::illegal_argument(format!("invalid distance function: {i}")))
}

/// `SIMILARITY_FUNCTIONS.get(ordinal)`, `None` outside the list.
fn read_vector_similarity(ordinal: i32) -> Option<VectorSimilarityFunction> {
    match ordinal {
        0 => Some(VectorSimilarityFunction::EUCLIDEAN),
        1 => Some(VectorSimilarityFunction::DOT_PRODUCT),
        2 => Some(VectorSimilarityFunction::COSINE),
        3 => Some(VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT),
        _ => None,
    }
}

/// The documents of a field in ordinal order: Java's ordinal-to-document map
/// as `mapOldOrdToNewOrd` walks `DocsWithFieldSet` (`KnnVectorsWriter.java`).
fn build_ord_to_doc(docs_with_field_set: &DocsWithFieldSet) -> Result<Vec<i32>> {
    let mut ord_to_doc = Vec::with_capacity(docs_with_field_set.cardinality() as usize);
    let mut iter = docs_with_field_set.iterator()?;
    while iter.next_doc()? != NO_MORE_DOCS {
        ord_to_doc.push(iter.doc_id());
    }
    Ok(ord_to_doc)
}

// -----------------------------------------------------------------------------
// Lucene99HnswVectorsReader
// -----------------------------------------------------------------------------

impl Lucene99HnswVectorsReader {
    /// The similarity functions in the order of their ordinals in the file
    /// format, so that it does not depend on the declaration order of
    /// `VectorSimilarityFunction`.
    ///
    /// Equivalent to `Lucene99HnswVectorsReader.SIMILARITY_FUNCTIONS`
    /// (`Lucene99HnswVectorsReader.java:222-229`), a `List.of(...)`.
    pub const SIMILARITY_FUNCTIONS: [VectorSimilarityFunction; 4] = [
        VectorSimilarityFunction::EUCLIDEAN,
        VectorSimilarityFunction::DOT_PRODUCT,
        VectorSimilarityFunction::COSINE,
        VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT,
    ];
}

/// Reader for the Lucene 9.9 HNSW vector format.
///
/// Equivalent to `org.apache.lucene.codecs.lucene99.Lucene99HnswVectorsReader`.
pub struct Lucene99HnswVectorsReader {
    flat_vectors_reader: Box<dyn FlatVectorsReader>,
    field_infos: FieldInfos,
    /// Equivalent to `Lucene99HnswVectorsReader.fields`, an
    /// `IntObjectHashMap<FieldEntry>` keyed by field number, whose
    /// `ramBytesUsed()` the reader reports.
    fields: IntObjectHashMap<FieldEntry>,
    vector_index: SharedIndexInput,
    version: i32,
    data_context: Box<dyn crate::store::IOContext>,
}

impl fmt::Debug for Lucene99HnswVectorsReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Lucene99HnswVectorsReader")
            .field("fields", &self.fields.size())
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
    /// `null` in Java when the field has no node offsets at all.
    offsets_meta: Option<DirectMonotonicMeta>,
    offsets_offset: i64,
    offsets_block_shift: i32,
    offsets_length: i64,
}

impl Lucene99HnswVectorsReader {
    /// Creates a new reader for the given segment read state.
    ///
    /// Equivalent to `Lucene99HnswVectorsReader(SegmentReadState,
    /// FlatVectorsReader)` (`Lucene99HnswVectorsReader.java:83-134`): when it
    /// fails, `IOUtils.closeWhileHandlingException(this)` runs `close()`,
    /// `IOUtils.close(flatVectorsReader, vectorIndex)`, whose `vectorIndex`
    /// is assigned last and so is still null.
    pub fn new(
        state: &SegmentReadState<'_>,
        flat_vectors_reader: Box<dyn FlatVectorsReader>,
    ) -> Result<Self> {
        let flat_vectors_reader = flat_vectors_reader;
        match Self::open(state) {
            Ok((fields, vector_index, version_meta, data_context)) => Ok(Self {
                flat_vectors_reader,
                field_infos: state.field_infos.clone(),
                fields,
                vector_index: SharedIndexInput::new(vector_index),
                version: version_meta,
                data_context,
            }),
            Err(error) => {
                IOUtils::close_while_handling_exception([Some(|| {
                    IOUtils::close([Some(|| flat_vectors_reader.close())])
                })])?;
                Err(error)
            }
        }
    }

    /// The `try` block of the constructor: reads the metadata and opens the
    /// `.vex` input.
    #[allow(clippy::type_complexity)]
    fn open(
        state: &SegmentReadState<'_>,
    ) -> Result<(
        IntObjectHashMap<FieldEntry>,
        Box<dyn IndexInput>,
        i32,
        Box<dyn crate::store::IOContext>,
    )> {
        let meta_file_name = segment_file_name(
            &state.segment_info.name,
            &state.segment_suffix,
            META_EXTENSION,
        );
        let mut meta_checksum = state.directory.open_checksum_input(&meta_file_name)?;
        // The `try` block of the try-with-resources over `meta`, whose failure
        // `CodecUtil.checkFooter(meta, priorE)` weighs in the `finally`
        // (`Lucene99HnswVectorsReader.java:97-112`).
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

        // Even though this input is referred to as `indexIn`, it does not
        // qualify as `FileTypeHint.INDEX` since it is a large file
        // (`Lucene99HnswVectorsReader.java:115-127`).
        let hints: [Arc<dyn crate::store::FileOpenHint>; 4] = [
            Arc::new(crate::store::FileTypeHint::Data),
            Arc::new(crate::store::FileDataHint::KnnVectors),
            Arc::new(crate::store::DataAccessHint::Random),
            Arc::new(crate::store::PreloadHint::instance()),
        ];
        let data_context = state.context.with_hints(&hints);
        let vector_index = Self::open_data_input(
            state,
            version_meta,
            VECTOR_INDEX_EXTENSION,
            VECTOR_INDEX_CODEC_NAME,
            data_context.as_ref(),
        )?;
        Ok((fields, vector_index, version_meta, data_context))
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
                // Lucene99HnswVectorsReader.java:176-183.
                return Err(LuceneError::corrupt_index(
                    format!(
                        "Format versions mismatch: meta={version_meta}, \
                         {codec_name}={version_vector_data}"
                    ),
                    input.resource_description(),
                ));
            }
            retrieve_checksum(input.as_mut())?;
            Ok(())
        })();
        match checked {
            Ok(()) => Ok(input),
            Err(error) => {
                // `IOUtils.closeWhileHandlingException(in)`
                // (`Lucene99HnswVectorsReader.java:188-192`).
                IOUtils::close_while_handling_exception([Some(|| input.close())])?;
                Err(error)
            }
        }
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
            vector_index: SharedIndexInput::new(self.vector_index.clone_input()?),
            version: self.version,
            data_context: self.data_context.with_hints(self.data_context.hints()),
        })
    }

    /// Equivalent to `Lucene99HnswVectorsReader.getFieldEntryOrThrow(String)`
    /// (`Lucene99HnswVectorsReader.java:286-293`): the same
    /// `IllegalArgumentException` for an unknown field and for a field with
    /// no entry.
    fn get_field_entry_or_throw(&self, field: &str) -> Result<&FieldEntry> {
        self.field_infos
            .field_info(field)
            .and_then(|info| self.fields.get(info.number))
            .ok_or_else(|| LuceneError::illegal_argument(format!("field=\"{field}\" not found")))
    }

    fn get_field_entry(
        &self,
        field: &str,
        expected_encoding: VectorEncoding,
    ) -> Result<&FieldEntry> {
        let entry = self.get_field_entry_or_throw(field)?;
        if entry.vector_encoding != expected_encoding {
            // Lucene99HnswVectorsReader.java:295-307; `VectorEncoding.toString()`
            // is the constant name, which `Debug` prints.
            return Err(LuceneError::illegal_argument(format!(
                "field=\"{field}\" is encoded as: {entry_vector_encoding:?} expected: \
                 {expected_encoding:?}",
                entry_vector_encoding = entry.vector_encoding,
            )));
        }
        Ok(entry)
    }

    /// Returns the graph stored for `entry`, or the empty graph when the field
    /// stored no graph at all.
    ///
    /// Equivalent to the private
    /// `Lucene99HnswVectorsReader.getGraph(FieldEntry)`.
    fn get_graph_for_entry(&self, entry: &FieldEntry) -> Result<Box<dyn HnswGraph>> {
        if entry.vector_index_length == 0 {
            return Ok(Box::new(EmptyHnswGraph));
        }
        Ok(Box::new(OffHeapHnswGraph::new(
            entry,
            &**self.vector_index.read(),
            self.version,
        )?))
    }

    /// Searches `entry` for the nearest neighbours of the query the scorer
    /// carries, on the graph or by scanning every vector.
    ///
    /// `accept_docs` restricts the search to the documents that are neither
    /// deleted nor excluded by the caller's filter. It is put to Lucene's three
    /// uses: it caps the filtered document count at the graph size, it yields
    /// the ordinals the search may visit, and that filtered count decides
    /// between the graph and the exhaustive scan.
    ///
    /// Equivalent to the private `Lucene99HnswVectorsReader.search(FieldEntry,
    /// KnnCollector, AcceptDocs, IOSupplier<RandomVectorScorer>)`
    /// (`Lucene99HnswVectorsReader.java:331-384`): the graph search collects
    /// through an `OrdinalTranslatedKnnCollector` wrapping the caller's
    /// collector, which translates each ordinal to its document as it is
    /// collected; the exhaustive scan collects into the caller's collector
    /// directly, translating with `scorer.ordToDoc`. As Java's
    /// `scorerSupplier.get()`, `scorer_supplier` builds the scorer only once
    /// an empty field or a `k` of zero has returned
    /// (`Lucene99HnswVectorsReader.java:337-341`).
    ///
    /// # Divergence from Lucene 10.5.0
    ///
    /// Java's translation function is `scorer::ordToDoc`. A [`KnnCollector`]
    /// is `Send`, and a [`RandomVectorScorer`] is neither `Send` nor shareable
    /// while the graph search scores through it, so the collector cannot call
    /// the scorer; it translates through `ordToDoc` of the field's vector
    /// values instead (`field_values`), the mapping every scorer of the flat
    /// formats delegates its own `ordToDoc` to. The supplier builds both,
    /// the scorer first.
    ///
    /// # Errors
    ///
    /// Returns whatever building the scorer, or reading the graph, the vectors
    /// or the accept docs raises.
    fn search_with_scorer(
        &self,
        entry: &FieldEntry,
        scorer_supplier: impl FnOnce() -> Result<(
            Box<dyn RandomVectorScorer>,
            Box<dyn crate::index::vector_values::KnnVectorValues>,
        )>,
        knn_collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        if entry.size == 0 || knn_collector.k() == 0 {
            return Ok(());
        }
        let (mut scorer, field_values) = scorer_supplier()?;
        let k = knn_collector.k();
        // Use the approximate cardinality, as this is good enough, but ensure
        // it does not exceed the graph size, as that is illogical.
        let graph_size = if entry.vector_index_length == 0 {
            0
        } else {
            entry.size
        };
        let filtered_doc_count = accept_docs.cost()?.min(i64::from(graph_size)) as i32;
        let accepted = accept_docs.bits()?;
        let accepted_ords = scorer.get_accept_ords(accepted.as_deref());
        let num_vectors = scorer.max_ord();
        let mut do_hnsw = k < num_vectors;
        // The approximate number of vectors that would be visited if we did not
        // filter.
        let unfiltered_visit = crate::util::hnsw::searcher::expected_visited_nodes(k, graph_size);
        if unfiltered_visit >= filtered_doc_count || graph_size == 0 {
            do_hnsw = false;
        }
        if do_hnsw {
            let mut collector = OrdinalTranslatedKnnCollector::new(
                knn_collector,
                Box::new(move |ord: i32| field_values.ord_to_doc(ord)),
            );
            let mut graph = self.get_graph_for_entry(entry)?;
            HnswGraphSearcher::search(
                scorer.as_mut(),
                &mut collector,
                graph.as_mut(),
                accepted_ords.as_deref(),
                filtered_doc_count,
            )?;
        } else {
            // If k is larger than the number of vectors we expect to visit in
            // an HNSW search, we can just iterate over all vectors and collect
            // them.
            let mut ords = [0i32; EXHAUSTIVE_BULK_SCORE_ORDS];
            let mut scores = [0f32; EXHAUSTIVE_BULK_SCORE_ORDS];
            let mut num_ords = 0usize;
            for i in 0..num_vectors {
                if accepted_ords
                    .as_deref()
                    .map_or(Ok(true), |accepted| accepted.try_get(i as usize))?
                {
                    if knn_collector.early_terminated() {
                        break;
                    }
                    ords[num_ords] = i;
                    num_ords += 1;
                    if num_ords == ords.len() {
                        score_batch_exhaustive(
                            &ords,
                            &mut scores,
                            num_ords,
                            knn_collector,
                            scorer.as_mut(),
                        )?;
                        num_ords = 0;
                    }
                }
            }
            if num_ords > 0 {
                score_batch_exhaustive(
                    &ords,
                    &mut scores,
                    num_ords,
                    knn_collector,
                    scorer.as_mut(),
                )?;
            }
        }
        Ok(())
    }

    /// The vector values of `field`, whose `ordToDoc` the graph search's
    /// `OrdinalTranslatedKnnCollector` translates through; see
    /// [`search_with_scorer`](Self::search_with_scorer).
    fn field_values(
        &self,
        field: &str,
        encoding: VectorEncoding,
    ) -> Result<Box<dyn crate::index::vector_values::KnnVectorValues>> {
        let values: Option<Box<dyn crate::index::vector_values::KnnVectorValues>> = match encoding {
            VectorEncoding::FLOAT32 => self
                .flat_vectors_reader
                .get_float_vector_values(field)?
                .map(|values| values as Box<dyn crate::index::vector_values::KnnVectorValues>),
            _ => self
                .flat_vectors_reader
                .get_byte_vector_values(field)?
                .map(|values| values as Box<dyn crate::index::vector_values::KnnVectorValues>),
        };
        // `vectorValues::ordToDoc` on a `null` receiver: the JVM's
        // `NullPointerException`.
        values.ok_or_else(LuceneError::null_pointer)
    }
}

/// Hands out the quantized form of the vectors when the flat reader holds
/// one.
///
/// Equivalent to `Lucene99HnswVectorsReader implements QuantizedVectorsReader`
/// (`Lucene99HnswVectorsReader.java:74, 428-452`); each method forwards to the
/// inherent one of the same name.
impl crate::util::quantization::QuantizedVectorsReader for Lucene99HnswVectorsReader {
    fn get_quantized_vector_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn crate::util::quantization::QuantizedByteVectorValues>>> {
        Lucene99HnswVectorsReader::get_quantized_vector_values(self, field)
    }

    fn get_quantization_state(
        &self,
        field: &str,
    ) -> Result<Option<crate::util::quantization::ScalarQuantizer>> {
        Lucene99HnswVectorsReader::get_quantization_state(self, field)
    }

    fn get_random_vector_scorer_supplier_for_merge(
        &self,
        field_info: &FieldInfo,
        segment_write_state: &SegmentWriteState<'_>,
    ) -> Result<Option<Box<dyn CloseableRandomVectorScorerSupplier>>> {
        Lucene99HnswVectorsReader::get_random_vector_scorer_supplier_for_merge(
            self,
            field_info,
            segment_write_state,
        )
    }
}

/// Hands out the HNSW graph a field stored, so that a merge can reuse it
/// instead of rebuilding one.
///
/// Equivalent to `Lucene99HnswVectorsReader implements HnswGraphProvider`.
impl HnswGraphProvider for Lucene99HnswVectorsReader {
    /// Returns the graph stored for `field`.
    ///
    /// Equivalent to `Lucene99HnswVectorsReader.getGraph(String)`: an unknown
    /// field, or a known field with no vector values, is an
    /// `IllegalArgumentException`, while a known field that stored no graph
    /// answers `HnswGraph.EMPTY`.
    fn get_graph(&self, field: &str) -> Result<Box<dyn HnswGraph>> {
        let entry = self.get_field_entry_or_throw(field)?;
        self.get_graph_for_entry(entry)
    }
}

/// Equivalent to `Lucene99HnswVectorsReader.ramBytesUsed()`
/// (`Lucene99HnswVectorsReader.java:264-270`): the shallow size, the field
/// entries and the flat vectors reader.
///
/// `SHALLOW_SIZE` is `shallowSizeOfInstance(Lucene99HnswVectorsFormat.class)`
/// — the format's class, not the reader's (`Lucene99HnswVectorsReader.java:76-77`),
/// an oddity of the original kept as the contract; see
/// [`HNSW_VECTORS_READER_SHALLOW_SIZE`]. `fields.ramBytesUsed()` is the
/// `IntObjectHashMap`'s own estimate.
impl crate::util::Accountable for Lucene99HnswVectorsReader {
    fn ram_bytes_used(&self) -> i64 {
        HNSW_VECTORS_READER_SHALLOW_SIZE
            + self.fields.ram_bytes_used()
            + self.flat_vectors_reader.ram_bytes_used()
    }
}

/// `RamUsageEstimator.shallowSizeOfInstance(Lucene99HnswVectorsFormat.class)`
/// on a 64-bit JVM with compressed oops: a 12-byte header, the two references
/// `KnnVectorsFormat.name` and `mergeExec`, and the five `int` fields
/// `maxConn`, `beamWidth`, `numMergeWorkers`, `tinySegmentsThreshold` and
/// `writeVersion` (the flat format is a `static` field), aligned to 8.
const HNSW_VECTORS_READER_SHALLOW_SIZE: i64 = crate::util::RamUsageEstimator::align_object_size(
    crate::util::RamUsageEstimator::NUM_BYTES_OBJECT_HEADER
        + 2 * crate::util::RamUsageEstimator::NUM_BYTES_OBJECT_REF
        + 5 * 4,
);

impl KnnVectorsReader for Lucene99HnswVectorsReader {
    /// This reader answers with itself: Java's `instanceof
    /// Lucene99HnswVectorsReader`, `instanceof HnswGraphProvider` and
    /// `instanceof QuantizedVectorsReader` on it are `true`
    /// (`BaseKnnVectorsFormatTestCase.hasHNSW`, `hasQuantized`).
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn check_integrity(&self) -> Result<()> {
        self.flat_vectors_reader.check_integrity()?;
        let mut data = self.vector_index.clone_input()?;
        checksum_entire_file(data.as_mut())?;
        Ok(())
    }

    fn get_float_vector_values(&self, field: &str) -> Result<Option<Box<dyn FloatVectorValues>>> {
        self.flat_vectors_reader.get_float_vector_values(field)
    }

    fn get_byte_vector_values(&self, field: &str) -> Result<Option<Box<dyn ByteVectorValues>>> {
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
        // `() -> flatVectorsReader.getRandomVectorScorer(field, target)`.
        let scorer_supplier = || {
            let scorer = self
                .flat_vectors_reader
                .get_random_vector_scorer_float(field, target)?
                // A `null` scorer: `scorer::ordToDoc` dereferences it
                // (Lucene99HnswVectorsReader.java:340-342).
                .ok_or_else(LuceneError::null_pointer)?;
            let values = self.field_values(field, VectorEncoding::FLOAT32)?;
            Ok((scorer, values))
        };
        self.search_with_scorer(&entry, scorer_supplier, knn_collector, accept_docs)
    }

    fn search_byte(
        &self,
        field: &str,
        target: &[u8],
        knn_collector: &mut dyn KnnCollector,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        let entry = self.get_field_entry(field, VectorEncoding::BYTE)?.clone();
        // `() -> flatVectorsReader.getRandomVectorScorer(field, target)`.
        let scorer_supplier = || {
            let scorer = self
                .flat_vectors_reader
                .get_random_vector_scorer_byte(field, target)?
                // A `null` scorer: `scorer::ordToDoc` dereferences it
                // (Lucene99HnswVectorsReader.java:340-342).
                .ok_or_else(LuceneError::null_pointer)?;
            let values = self.field_values(field, VectorEncoding::BYTE)?;
            Ok((scorer, values))
        };
        self.search_with_scorer(&entry, scorer_supplier, knn_collector, accept_docs)
    }

    fn get_merge_instance(&self) -> Result<Box<dyn KnnVectorsReader>> {
        Ok(Box::new(self.clone_reader()?))
    }

    /// Equivalent to `Lucene99HnswVectorsReader.finishMerge()`
    /// (`Lucene99HnswVectorsReader.java:150-153`): the flat reader's.
    fn finish_merge(&mut self) -> Result<()> {
        self.flat_vectors_reader.finish_merge()
    }

    /// Equivalent to `Lucene99HnswVectorsReader.close()`
    /// (`Lucene99HnswVectorsReader.java:424-427`): `IOUtils.close(flatVectorsReader,
    /// vectorIndex)`.
    fn close(&self) -> Result<()> {
        IOUtils::close([
            IOUtils::closeable(|| self.flat_vectors_reader.close()),
            IOUtils::closeable(|| self.vector_index.close()),
        ])
    }

    /// Equivalent to `Lucene99HnswVectorsReader.getOffHeapByteSize(FieldInfo)`
    /// (`Lucene99HnswVectorsReader.java:416-422`): the field entry is looked
    /// up first, raising `IllegalArgumentException` for an unknown field, then
    /// the flat reader's map is merged with the graph's.
    fn get_off_heap_byte_size(&self, field_info: &FieldInfo) -> Result<HashMap<String, i64>> {
        let entry = self.get_field_entry_or_throw(&field_info.name)?;
        let flat = self
            .flat_vectors_reader
            .get_off_heap_byte_size(field_info)?;
        let mut graph = HashMap::new();
        graph.insert(
            VECTOR_INDEX_EXTENSION.to_string(),
            entry.vector_index_length,
        );
        Ok(crate::codecs::knn_vectors::merge_off_heap_byte_size_maps(
            flat, graph,
        ))
    }
}

/// Scores one batch of accepted ordinals and collects it when the batch holds
/// anything competitive.
///
/// Equivalent to the body Lucene repeats twice inside the exhaustive branch of
/// `Lucene99HnswVectorsReader.search(FieldEntry, KnnCollector, AcceptDocs,
/// IOSupplier)`: the visited count is raised before the batch is scored, and
/// the batch is only collected when its best score beats the collector's
/// minimum competitive similarity.
///
/// The ordinals are collected untranslated, because the caller translates them
/// once the search is over — see the divergence recorded on
/// `Lucene99HnswVectorsReader::search_with_scorer`.
///
/// # Errors
///
/// Returns whatever scoring the batch raises.
fn score_batch_exhaustive(
    ords: &[i32],
    scores: &mut [f32],
    num_ords: usize,
    knn_collector: &mut dyn KnnCollector,
    scorer: &mut dyn RandomVectorScorer,
) -> Result<()> {
    if num_ords == 0 {
        return Ok(());
    }
    knn_collector.inc_visited_count(num_ords as i32);
    let best = scorer.bulk_score(ords, &mut scores[..num_ords], num_ords as i32)?;
    if best > knn_collector.min_competitive_similarity() {
        for i in 0..num_ords {
            knn_collector.collect(scorer.ord_to_doc(ords[i]), scores[i]);
        }
    }
    Ok(())
}

/// Equivalent to `Lucene99HnswVectorsReader.readFields(ChecksumIndexInput)`
/// (`Lucene99HnswVectorsReader.java:196-206`).
fn read_fields(
    input: &mut dyn DataInput,
    field_infos: &FieldInfos,
) -> Result<IntObjectHashMap<FieldEntry>> {
    let mut fields = IntObjectHashMap::new();
    let mut field_number = input.read_int()?;
    while field_number != -1 {
        let Some(info) = field_infos.field_info_checked(field_number)? else {
            return Err(LuceneError::corrupt_index(
                format!("Invalid field number: {field_number}"),
                input.to_java_string(),
            ));
        };
        let entry = read_field(input, info)?;
        validate_field_entry(info, &entry)?;
        fields.put(info.number, entry);
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
    let vector_encoding = read_vector_encoding_input(input)?;
    let similarity_function = read_similarity_function(input)?;
    if similarity_function != info.vector_similarity_function {
        // Lucene99HnswVectorsReader.java:251-259; the enum's `toString()` is
        // its constant name, which `Debug` prints.
        return Err(LuceneError::illegal_state(format!(
            "Inconsistent vector similarity function for field=\"{}\"; {:?} != {:?}",
            info.name, similarity_function, info.vector_similarity_function
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
        // Lucene99HnswVectorsReader.java:207-218.
        return Err(LuceneError::illegal_state(format!(
            "Inconsistent vector dimension for field=\"{}\"; {} != {}",
            info.name, info.vector_dimension, field_entry.dimension
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
        // `new int[numLevels][]` (Lucene99HnswVectorsReader.java:483) raises
        // `NegativeArraySizeException` for a negative count.
        if num_levels < 0 {
            return Err(negative_array_size(num_levels));
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
                number_of_offsets += i64::from(num_nodes);
                // `new int[numNodesOnLevel]` (Lucene99HnswVectorsReader.java:492).
                if num_nodes < 0 {
                    return Err(negative_array_size(num_nodes));
                }
                // `nodesByLevel[level][0] = input.readVInt()`: the right-hand
                // side is read before the store into an empty array raises
                // `ArrayIndexOutOfBoundsException` (JLS 15.26.1).
                let first = input.read_v_int()?;
                if num_nodes == 0 {
                    return Err(array_index_out_of_bounds(0, 0));
                }

                // Again grown, not pre-sized: each node is at least one byte,
                // so a hostile count runs out of file instead of out of memory.
                let mut nodes: Vec<i32> = Vec::new();
                nodes.push(first);
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
            offsets_meta,
            offsets_offset,
            offsets_block_shift,
            offsets_length,
        })
    }
}

// -----------------------------------------------------------------------------
// Off-heap HNSW graph
// -----------------------------------------------------------------------------

/// The graph of one field, read from the `.vex` file on demand.
///
/// Equivalent to `Lucene99HnswVectorsReader.OffHeapHnswGraph`
/// (`Lucene99HnswVectorsReader.java:529-655`). As in Java, nothing read off
/// disk is validated here beyond what Java's own array and reader accesses
/// check: an out-of-range level or block raises
/// `ArrayIndexOutOfBoundsException`, a node missing from its level is sought
/// at the offset its negative `binarySearch` result addresses, and neighbour
/// ordinals are handed on as decoded.
struct OffHeapHnswGraph {
    data_in: Box<dyn IndexInput>,
    nodes_by_level: Vec<Option<Vec<i32>>>,
    num_levels: i32,
    entry_node: i32,
    size: i32,
    max_conn: i32,
    /// Equivalent to `graphLevelNodeOffsets`, a `DirectMonotonicReader` over
    /// a `RandomAccessInput` slice of the `.vex` file, read one offset per
    /// [`seek`](HnswGraph::seek).
    offsets: DirectMonotonicReader,
    graph_level_node_index_offsets: Vec<i64>,
    /// Exactly `M * 2` entries, as Java allocates it.
    current_neighbors_buffer: Vec<i32>,
    /// Java's `int arcCount`, a vint off disk that may be negative.
    arc_count: i32,
    arc_upto: i32,
    version: i32,
}

/// `new NegativeArraySizeException(String.valueOf(size))`, what `new T[size]`
/// raises for a negative size.
fn negative_array_size(size: i32) -> LuceneError {
    LuceneError::unchecked("java.lang.NegativeArraySizeException", size.to_string())
}

/// The `ArrayIndexOutOfBoundsException` of an array access at `index` into
/// an array of `length` elements.
fn array_index_out_of_bounds(index: i64, length: usize) -> LuceneError {
    LuceneError::unchecked(
        "java.lang.ArrayIndexOutOfBoundsException",
        format!("Index {index} out of bounds for length {length}"),
    )
}

/// `java.util.Arrays.binarySearch(int[], int, int, int)` over the whole
/// array: the index of `key`, or `-(insertionPoint) - 1` when it is absent.
fn java_binary_search(array: &[i32], key: i32) -> i32 {
    let mut low = 0i32;
    let mut high = array.len() as i32 - 1;
    while low <= high {
        let mid = ((low as u32 + high as u32) >> 1) as i32;
        let mid_val = array[mid as usize];
        if mid_val < key {
            low = mid + 1;
        } else if mid_val > key {
            high = mid - 1;
        } else {
            return mid;
        }
    }
    -(low + 1)
}

impl OffHeapHnswGraph {
    /// Equivalent to the `OffHeapHnswGraph(FieldEntry, IndexInput)`
    /// constructor (`Lucene99HnswVectorsReader.java:546-565`), step by step:
    /// the graph slice, the entry node, the offsets reader over a lazy
    /// `randomAccessSlice` (`DirectMonotonicReader.getInstance` dereferences
    /// a `null` meta), the `M * 2` neighbour buffer and the per-level index
    /// offsets, whose `[0] = 0` store fails for a graph of no levels.
    fn new(entry: &FieldEntry, vector_index: &dyn IndexInput, version: i32) -> Result<Self> {
        let data_in = vector_index.slice(
            "graph-data",
            entry.vector_index_offset,
            entry.vector_index_length,
        )?;
        let num_levels = entry.num_levels;
        let entry_node = if num_levels > 1 {
            match &entry.nodes_by_level[num_levels as usize - 1] {
                Some(nodes) => nodes[0],
                None => return Err(LuceneError::null_pointer()),
            }
        } else {
            0
        };
        let addresses_data =
            vector_index.random_access_slice(entry.offsets_offset, entry.offsets_length)?;
        let offsets = match &entry.offsets_meta {
            Some(meta) => DirectMonotonicReader::with_random_access(
                meta.clone(),
                crate::util::packed::shared_random_access_input(addresses_data),
            )?,
            None => return Err(LuceneError::null_pointer()),
        };
        let max_conn = entry.m;
        let buffer_len = max_conn.wrapping_mul(2);
        if buffer_len < 0 {
            return Err(negative_array_size(buffer_len));
        }
        let current_neighbors_buffer = vec![0i32; buffer_len as usize];
        if num_levels == 0 {
            return Err(array_index_out_of_bounds(0, 0));
        }
        let mut graph_level_node_index_offsets = vec![0i64; num_levels as usize];
        for i in 1..num_levels as usize {
            // `nodesByLevel` is `null` for the zeroth level as its size is known.
            let node_count = entry.nodes_by_level[i - 1]
                .as_ref()
                .map_or(entry.size, |n| n.len() as i32);
            graph_level_node_index_offsets[i] =
                graph_level_node_index_offsets[i - 1] + node_count as i64;
        }
        Ok(Self {
            data_in,
            nodes_by_level: entry.nodes_by_level.clone(),
            num_levels,
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

    /// Returns `nodesByLevel[level]`, raising what Java's array access
    /// raises: `ArrayIndexOutOfBoundsException` for a level outside the
    /// graph and `NullPointerException` for the zeroth level, which has no
    /// node array.
    fn level_nodes(&self, level: i32) -> Result<&Vec<i32>> {
        if level < 0 || level as usize >= self.nodes_by_level.len() {
            return Err(array_index_out_of_bounds(
                i64::from(level),
                self.nodes_by_level.len(),
            ));
        }
        self.nodes_by_level[level as usize]
            .as_ref()
            .ok_or_else(LuceneError::null_pointer)
    }

    /// Stores `value` at `index` of the neighbour buffer, raising
    /// `ArrayIndexOutOfBoundsException` past its `M * 2` entries as Java's
    /// `currentNeighborsBuffer[i] = ...` does.
    fn store_neighbor(&mut self, index: usize, value: i32) -> Result<()> {
        let length = self.current_neighbors_buffer.len();
        match self.current_neighbors_buffer.get_mut(index) {
            Some(slot) => {
                *slot = value;
                Ok(())
            }
            None => Err(array_index_out_of_bounds(index as i64, length)),
        }
    }
}

impl HnswGraph for OffHeapHnswGraph {
    /// Equivalent to `OffHeapHnswGraph.seek(int, int)`
    /// (`Lucene99HnswVectorsReader.java:567-597`).
    fn seek(&mut self, level: i32, target: i32) -> Result<()> {
        let target_index = if level == 0 {
            target
        } else {
            java_binary_search(self.level_nodes(level)?, target)
        };
        debug_assert!(
            target_index >= 0,
            "seek level={level} target={target} not found: {target_index}"
        );
        // Unsafe; no bounds checking: a negative `targetIndex` addresses
        // whatever offset it lands on, as in Java with assertions disabled.
        let level_offset = match self.graph_level_node_index_offsets.get(level as usize) {
            Some(offset) if level >= 0 => *offset,
            _ => {
                return Err(array_index_out_of_bounds(
                    i64::from(level),
                    self.graph_level_node_index_offsets.len(),
                ))
            }
        };
        let offset = self
            .offsets
            .get_java(i64::from(target_index) + level_offset)?;
        self.data_in.seek(offset)?;
        self.arc_count = self.data_in.read_v_int()?;
        debug_assert!(
            self.arc_count as usize <= self.current_neighbors_buffer.len() || self.arc_count < 0,
            "too many neighbors: {}",
            self.arc_count
        );
        if self.arc_count > 0 {
            let limit = self.arc_count as usize;
            if self.version >= VERSION_GROUPVARINT {
                // `GroupVIntUtil.readGroupVInts(dataIn, currentNeighborsBuffer,
                // arcCount)`: whole groups of four, then single vints; each
                // value is stored in order, so the store past the buffer is
                // the one that fails.
                let mut i = 0usize;
                while i + 4 <= limit {
                    let mut group = [0i32; 4];
                    crate::util::GroupVIntUtil::read_group_vint_input(
                        self.data_in.as_mut(),
                        &mut group,
                        0,
                    )?;
                    for (k, value) in group.into_iter().enumerate() {
                        self.store_neighbor(i + k, value)?;
                    }
                    i += 4;
                }
                while i < limit {
                    let value = self.data_in.read_v_int()?;
                    self.store_neighbor(i, value)?;
                    i += 1;
                }
                // Java accumulates with plain `int` arithmetic, which wraps.
                let mut sum = 0i32;
                for slot in &mut self.current_neighbors_buffer[..limit] {
                    sum = sum.wrapping_add(*slot);
                    *slot = sum;
                }
            } else {
                let mut sum = 0i32;
                for i in 0..limit {
                    sum = sum.wrapping_add(self.data_in.read_v_int()?);
                    self.store_neighbor(i, sum)?;
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
        let n = self.current_neighbors_buffer[self.arc_upto as usize];
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
            Ok(Box::new(ArrayNodesIterator::new(
                self.level_nodes(level)?.clone(),
            )))
        }
    }

    fn neighbor_count(&self) -> i32 {
        self.arc_count
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use crate::error::LuceneErrorKind;
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
    use crate::search::knn::TopKnnCollector;
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
            None,
        )
    }

    fn make_states<'a>(
        dir: &'a Arc<dyn crate::store::Directory>,
        seg_info: &'a SegmentInfo,
        field_infos: &'a FieldInfos,
        _seg_updates: &'a BufferedUpdates,
        io_ctx: &'a DefaultIOContext,
    ) -> (OwnedSegmentWriteState, SegmentReadState<'a>) {
        let write_state = OwnedSegmentWriteState::new(
            Arc::new(crate::util::NoOutputInfoStream),
            Arc::clone(dir),
            seg_info.clone(),
            field_infos.clone(),
            BufferedUpdates::default(),
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

    /// A beam width of 1 passes the format's own check, but
    /// `HnswGraphBuilder`'s constructor then builds a
    /// `GraphBuilderKnnCollector(min(beamWidth / 2, M * 3))` = `(0)`, whose
    /// `NeighborQueue` throws `IllegalArgumentException` from
    /// `TernaryLongHeap(0)` (`HnswGraphBuilder.java:211-213`,
    /// `TernaryLongHeap.java:55-63`). With no tiny-segments threshold the graph
    /// is built, so writing the segment fails with that error.
    #[test]
    fn a_beam_width_of_one_fails_the_graph_build_as_lucene_does() -> Result<()> {
        let dir = Arc::new(ByteBuffersDirectory::new()) as Arc<dyn crate::store::Directory>;
        let seg_info = make_segment_info(&dir, 3)?;
        let field_info = float_field_info("float_field", 0, 2)?;
        let field_infos = FieldInfos::new(vec![field_info.clone()])?;
        let seg_updates = BufferedUpdates::default();
        let io_ctx = DefaultIOContext::default();
        let (write_state, _read_state) =
            make_states(&dir, &seg_info, &field_infos, &seg_updates, &io_ctx);

        let format = Lucene99HnswVectorsFormat::with_merge_exec_and_threshold(16, 1, 1, None, 0);
        let mut writer = format.fields_writer(&write_state)?;
        let mut float_writer = match writer.add_field(&field_info)? {
            FieldVectorWriter::Float(w) => w,
            _ => panic!("expected float field writer"),
        };
        float_writer.add_value(0, vec![1.0, 0.0])?;
        float_writer.add_value(1, vec![0.0, 1.0])?;
        float_writer.add_value(2, vec![1.0, 1.0])?;
        match writer.flush(3, None) {
            Err(LuceneError {
                kind: LuceneErrorKind::IllegalArgument { message, .. },
                ..
            }) => assert_eq!(
                message,
                format!(
                    "initialCapacity must be > 0 and < {}; got: 0",
                    crate::util::ArrayUtil::MAX_ARRAY_LENGTH - 1
                )
            ),
            other => panic!("expected IllegalArgumentException, got {other:?}"),
        }
        Ok(())
    }

    /// The graph file is opened with `FileTypeHint.DATA`,
    /// `FileDataHint.KNN_VECTORS`, `DataAccessHint.RANDOM` and
    /// `PreloadHint.INSTANCE` (`Lucene99HnswVectorsReader.java:115-127`).
    #[test]
    fn vector_index_is_opened_with_the_hints_lucene_gives_it() -> Result<()> {
        use crate::codecs::lucene99::flat_vectors_format::tests::RecordingDirectory;

        let recording = Arc::new(RecordingDirectory::default());
        let dir = Arc::clone(&recording) as Arc<dyn crate::store::Directory>;
        let seg_info = make_segment_info(&dir, 2)?;
        let field_info = float_field_info("float_field", 0, 2)?;
        let field_infos = FieldInfos::new(vec![field_info.clone()])?;
        let seg_updates = BufferedUpdates::default();
        let io_ctx = DefaultIOContext::default();
        let (write_state, read_state) =
            make_states(&dir, &seg_info, &field_infos, &seg_updates, &io_ctx);

        let format = Lucene99HnswVectorsFormat::new();
        {
            let mut writer = format.fields_writer(&write_state)?;
            let mut float_writer = match writer.add_field(&field_info)? {
                FieldVectorWriter::Float(w) => w,
                _ => panic!("expected float field writer"),
            };
            float_writer.add_value(0, vec![1.0, 0.0])?;
            float_writer.add_value(1, vec![0.0, 1.0])?;
            writer.flush(2, None)?;
            writer.finish()?;
            writer.close()?;
        }

        let _reader = format.fields_reader(&read_state)?;
        assert_eq!(
            recording.events(".vex", "open"),
            vec![vec![
                "Data".to_string(),
                "KnnVectors".to_string(),
                "PreloadHint".to_string(),
                "Random".to_string(),
            ]]
        );
        Ok(())
    }

    /// `Lucene99HnswVectorsReader.getMergeInstance()` asks the flat reader for
    /// its merge instance, which advises the raw `.vec` sequential, and
    /// `finishMerge()` forwards to the flat reader, which reverts it
    /// (`Lucene99HnswVectorsReader.java:145-153`).
    #[test]
    fn finish_merge_reaches_the_flat_reader() -> Result<()> {
        use crate::codecs::lucene99::flat_vectors_format::tests::RecordingDirectory;

        let recording = Arc::new(RecordingDirectory::default());
        let dir = Arc::clone(&recording) as Arc<dyn crate::store::Directory>;
        let seg_info = make_segment_info(&dir, 2)?;
        let field_info = float_field_info("float_field", 0, 2)?;
        let field_infos = FieldInfos::new(vec![field_info.clone()])?;
        let seg_updates = BufferedUpdates::default();
        let io_ctx = DefaultIOContext::default();
        let (write_state, read_state) =
            make_states(&dir, &seg_info, &field_infos, &seg_updates, &io_ctx);
        let format = Lucene99HnswVectorsFormat::new();
        {
            let mut writer = format.fields_writer(&write_state)?;
            let mut float_writer = match writer.add_field(&field_info)? {
                FieldVectorWriter::Float(w) => w,
                _ => panic!("expected float field writer"),
            };
            float_writer.add_value(0, vec![1.0, 0.0])?;
            float_writer.add_value(1, vec![0.0, 1.0])?;
            writer.flush(2, None)?;
            writer.finish()?;
            writer.close()?;
        }

        let reader = format.fields_reader(&read_state)?;
        let mut merge_instance = reader.get_merge_instance()?;
        assert_eq!(
            recording.events(".vec", "update"),
            vec![vec!["Sequential".to_string()]]
        );
        merge_instance.finish_merge()?;
        let updates = recording.events(".vec", "update");
        assert_eq!(updates.len(), 2, "finishMerge reaches the flat reader");
        let mut reverted = updates[1].clone();
        let mut opened = recording.events(".vec", "open")[0].clone();
        reverted.sort();
        opened.sort();
        assert_eq!(reverted, opened);
        Ok(())
    }

    /// The reader is a `QuantizedVectorsReader` whose methods forward to the
    /// flat reader when it is a quantized one, and answer `null` otherwise
    /// (`Lucene99HnswVectorsReader.java:74, 428-452`).
    #[test]
    fn the_reader_is_a_quantized_vectors_reader_over_its_flat_reader() -> Result<()> {
        use crate::codecs::lucene104::scalar_quantized_vectors::{
            Lucene104HnswScalarQuantizedVectorsFormat, Lucene104ScalarQuantizedVectorsFormat,
        };
        use crate::codecs::FlatVectorsFormat;
        use crate::util::quantization::QuantizedVectorsReader;

        let dir = Arc::new(ByteBuffersDirectory::new()) as Arc<dyn crate::store::Directory>;
        let seg_info = make_segment_info(&dir, 2)?;
        let field_info = float_field_info("float_field", 0, 2)?;
        let field_infos = FieldInfos::new(vec![field_info.clone()])?;
        let seg_updates = BufferedUpdates::default();
        let io_ctx = DefaultIOContext::default();
        let (write_state, read_state) =
            make_states(&dir, &seg_info, &field_infos, &seg_updates, &io_ctx);
        {
            let mut writer =
                Lucene104HnswScalarQuantizedVectorsFormat::new().fields_writer(&write_state)?;
            let mut float_writer = match writer.add_field(&field_info)? {
                FieldVectorWriter::Float(w) => w,
                _ => panic!("expected float field writer"),
            };
            float_writer.add_value(0, vec![1.0, 0.0])?;
            float_writer.add_value(1, vec![0.0, 1.0])?;
            writer.flush(2, None)?;
            writer.finish()?;
            writer.close()?;
        }

        let quantized = Lucene99HnswVectorsReader::new(
            &read_state,
            Lucene104ScalarQuantizedVectorsFormat::new().fields_reader_flat(&read_state)?,
        )?;
        let reader: &dyn QuantizedVectorsReader = &quantized;
        let values = reader
            .get_quantized_vector_values("float_field")?
            .expect("the flat reader is quantized");
        assert_eq!(values.size(), 2);
        assert!(reader.get_quantization_state("float_field")?.is_none());
        Ok(())
    }

    #[test]
    fn format_name_and_max_dimensions() {
        let format = Lucene99HnswVectorsFormat::new();
        assert_eq!(format.name(), "Lucene99HnswVectorsFormat");
        assert_eq!(format.get_max_dimensions("any"), 1024);
    }

    #[test]
    fn round_trip_float_vectors_and_search() -> Result<()> {
        let dir = Arc::new(ByteBuffersDirectory::new()) as Arc<dyn crate::store::Directory>;
        let seg_info = make_segment_info(&dir, 3)?;
        let field_info = float_field_info("float_field", 0, 2)?;
        let field_infos = FieldInfos::new(vec![field_info.clone()])?;
        let seg_updates = BufferedUpdates::default();
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

        let reader = format.fields_reader(&read_state)?;
        let values = reader
            .get_float_vector_values("float_field")?
            .expect("the field has vectors");
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
        let dir = Arc::new(ByteBuffersDirectory::new()) as Arc<dyn crate::store::Directory>;
        let seg_info = make_segment_info(&dir, 3)?;
        let field_info = byte_field_info("byte_field", 0, 2)?;
        let field_infos = FieldInfos::new(vec![field_info.clone()])?;
        let seg_updates = BufferedUpdates::default();
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
        let values = reader
            .get_byte_vector_values("byte_field")?
            .expect("the field has vectors");
        assert_eq!(values.size(), 3);
        assert_eq!(values.dimension(), 2);
        assert_eq!(values.vector_value(0)?, vec![10, 20]);

        reader.check_integrity()?;
        Ok(())
    }

    #[test]
    fn written_files_have_codec_headers() -> Result<()> {
        let dir = Arc::new(ByteBuffersDirectory::new()) as Arc<dyn crate::store::Directory>;
        let seg_info = make_segment_info(&dir, 1)?;
        let field_info = float_field_info("f", 0, 1)?;
        let field_infos = FieldInfos::new(vec![field_info.clone()])?;
        let seg_updates = BufferedUpdates::default();
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

    // -------------------------------------------------------------------------
    // accept_docs (rmp task #205)
    //
    // `Lucene99HnswVectorsReader.search` puts `acceptDocs` to three uses
    // (`Lucene99HnswVectorsReader.java:331-361`): it caps the filtered document
    // count at the graph size, it turns the accepted documents into the
    // accepted ordinals the graph search may visit, and it compares that
    // filtered count against `HnswGraphSearcher.expectedVisitedNodes` to decide
    // between the graph and the exhaustive scan. A search that ignores it
    // returns deleted documents and documents the caller's filter excludes.
    // -------------------------------------------------------------------------

    /// A [`Bits`](crate::util::Bits) over an explicit acceptance vector.
    #[derive(Debug)]
    struct VecBits(Vec<bool>);

    impl crate::util::Bits for VecBits {
        fn get(&self, index: usize) -> bool {
            self.0[index]
        }

        fn length(&self) -> usize {
            self.0.len()
        }
    }

    /// The number of documents the accept-docs fixtures index.
    const ACCEPT_DOCS_SIZE: i32 = 64;

    /// Writes `num_docs` float vectors, document `i` holding `[i, 0]`, so that
    /// the neighbours of `[0, 0]` are documents 0, 1, 2, … in that order.
    ///
    /// `tiny_segments_threshold` is forced to 1 so that a graph is written for
    /// a segment this small: `Lucene99HnswVectorsWriter.shouldCreateGraph`
    /// otherwise skips the graph whenever `numNodes <= log(numNodes) * 100`.
    fn write_accept_docs_segment(
        dir: &Arc<dyn crate::store::Directory>,
        seg_info: &SegmentInfo,
        field_info: &FieldInfo,
        field_infos: &FieldInfos,
        num_docs: i32,
    ) -> Result<()> {
        let seg_updates = BufferedUpdates::default();
        let write_state = OwnedSegmentWriteState::new(
            Arc::new(crate::util::NoOutputInfoStream),
            Arc::clone(dir),
            seg_info.clone(),
            field_infos.clone(),
            seg_updates,
            Arc::new(DefaultIOContext::default()),
        );
        let format = Lucene99HnswVectorsFormat::with_params(
            DEFAULT_MAX_CONN,
            DEFAULT_BEAM_WIDTH,
            DEFAULT_NUM_MERGE_WORKER,
            None,
            1,
            VERSION_CURRENT,
        );
        let mut writer = format.fields_writer(&write_state)?;
        let field_writer = writer.add_field(field_info)?;
        let mut float_writer = match field_writer {
            FieldVectorWriter::Float(w) => w,
            _ => panic!("expected float field writer"),
        };
        for doc in 0..num_docs {
            float_writer.add_value(doc, vec![doc as f32, 0.0])?;
        }
        writer.flush(num_docs, None)?;
        writer.finish()?;
        writer.close()?;
        Ok(())
    }

    /// Returns the documents a search for `[0, 0]` collects, best first.
    fn accept_docs_search(
        reader: &dyn KnnVectorsReader,
        k: i32,
        accept_docs: &mut dyn AcceptDocs,
    ) -> Result<(Vec<i32>, i64)> {
        let mut collector = TopKnnCollector::new(k, i64::MAX);
        reader.search("float_field", &[0.0, 0.0], &mut collector, accept_docs)?;
        let visited = collector.visited_count();
        let docs = collector
            .top_docs_with_scores()
            .into_iter()
            .map(|(doc, _)| doc)
            .collect();
        Ok((docs, visited))
    }

    /// Live docs that accept every document except `deleted`.
    fn live_docs_without(deleted: &[i32], max_doc: i32) -> Box<dyn crate::util::Bits> {
        let mut live = vec![true; max_doc as usize];
        for doc in deleted {
            live[*doc as usize] = false;
        }
        Box::new(VecBits(live))
    }

    /// A collector that records what the reader hands it, over a
    /// [`TopKnnCollector`] that keeps the results.
    struct RecordingKnnCollector {
        inner: TopKnnCollector,
        collected: Vec<(i32, f32)>,
        visited_increments: Vec<i32>,
    }

    impl KnnCollector for RecordingKnnCollector {
        fn early_terminated(&self) -> bool {
            self.inner.early_terminated()
        }

        fn inc_visited_count(&mut self, count: i32) {
            self.visited_increments.push(count);
            self.inner.inc_visited_count(count);
        }

        fn visited_count(&self) -> i64 {
            self.inner.visited_count()
        }

        fn visit_limit(&self) -> i64 {
            self.inner.visit_limit()
        }

        fn k(&self) -> i32 {
            self.inner.k()
        }

        fn collect(&mut self, doc_id: i32, similarity: f32) -> bool {
            self.collected.push((doc_id, similarity));
            self.inner.collect(doc_id, similarity)
        }

        fn min_competitive_similarity(&self) -> f32 {
            self.inner.min_competitive_similarity()
        }

        fn top_docs(&mut self) -> crate::error::Result<crate::search::knn::TopDocs> {
            self.inner.top_docs()
        }

        fn get_search_strategy(&self) -> Option<&crate::search::knn::KnnSearchStrategy> {
            self.inner.get_search_strategy()
        }
    }

    /// `Lucene99HnswVectorsReader.search` (`Lucene99HnswVectorsReader.java:340-363`)
    /// hands the graph search `new OrdinalTranslatedKnnCollector(knnCollector,
    /// scorer::ordToDoc)`: the caller's collector sees every visit and every
    /// collected document, translated, as the search makes them, not one
    /// visit increment and its top `k` once it is over.
    #[test]
    fn the_graph_search_collects_through_the_callers_collector() -> Result<()> {
        let dir = Arc::new(ByteBuffersDirectory::new()) as Arc<dyn crate::store::Directory>;
        let seg_info = make_segment_info(&dir, ACCEPT_DOCS_SIZE)?;
        let field_info = float_field_info("float_field", 0, 2)?;
        let field_infos = FieldInfos::new(vec![field_info.clone()])?;
        write_accept_docs_segment(&dir, &seg_info, &field_info, &field_infos, ACCEPT_DOCS_SIZE)?;
        let io_ctx = DefaultIOContext::default();
        let read_state = SegmentReadState::new(dir.as_ref(), &seg_info, &field_infos, &io_ctx);
        let reader = Lucene99HnswVectorsFormat::new().fields_reader(&read_state)?;

        let mut collector = RecordingKnnCollector {
            inner: TopKnnCollector::new(3, i64::MAX),
            collected: Vec::new(),
            visited_increments: Vec::new(),
        };
        let mut all = from_live_docs(None, ACCEPT_DOCS_SIZE)?;
        reader.search("float_field", &[0.0, 0.0], &mut collector, &mut all)?;
        assert!(
            collector.visited_increments.len() > 1,
            "every visit reaches the caller's collector: {:?}",
            collector.visited_increments
        );
        assert!(
            collector.collected.len() > 3,
            "every competitive document reaches the caller's collector: {:?}",
            collector.collected
        );
        // Document `i` holds `[i, 0]` and its ordinal is `i`, so a translated
        // document is a valid document of the segment.
        assert!(collector
            .collected
            .iter()
            .all(|(doc, _)| (0..ACCEPT_DOCS_SIZE).contains(doc)));
        let docs: Vec<i32> = collector
            .inner
            .top_docs_with_scores()
            .into_iter()
            .map(|(doc, _)| doc)
            .collect();
        assert_eq!(docs, vec![0, 1, 2]);
        Ok(())
    }

    /// A collector over a [`TopKnnCollector`] that reports the `k` it is given.
    struct KCollector {
        inner: TopKnnCollector,
        k: i32,
    }

    impl KnnCollector for KCollector {
        fn early_terminated(&self) -> bool {
            self.inner.early_terminated()
        }

        fn inc_visited_count(&mut self, count: i32) {
            self.inner.inc_visited_count(count);
        }

        fn visited_count(&self) -> i64 {
            self.inner.visited_count()
        }

        fn visit_limit(&self) -> i64 {
            self.inner.visit_limit()
        }

        fn k(&self) -> i32 {
            self.k
        }

        fn collect(&mut self, doc_id: i32, similarity: f32) -> bool {
            self.inner.collect(doc_id, similarity)
        }

        fn min_competitive_similarity(&self) -> f32 {
            self.inner.min_competitive_similarity()
        }

        fn top_docs(&mut self) -> crate::error::Result<crate::search::knn::TopDocs> {
            self.inner.top_docs()
        }

        fn get_search_strategy(&self) -> Option<&crate::search::knn::KnnSearchStrategy> {
            self.inner.get_search_strategy()
        }
    }

    /// `Lucene99HnswVectorsReader.search` returns on `knnCollector.k() == 0`
    /// before `scorerSupplier.get()` builds the scorer
    /// (`Lucene99HnswVectorsReader.java:337-341`), so a target of the wrong
    /// dimension is never checked. Java oracle
    /// (`oracle/passG2/LazyScorerOracle.java`): "k=0 returned visited=0",
    /// "k=1 threw java.lang.IllegalArgumentException: vector query dimension:
    /// 3 differs from field dimension: 2".
    #[test]
    fn a_k_of_zero_returns_before_the_scorer_is_built() -> Result<()> {
        let dir = Arc::new(ByteBuffersDirectory::new()) as Arc<dyn crate::store::Directory>;
        let seg_info = make_segment_info(&dir, ACCEPT_DOCS_SIZE)?;
        let field_info = float_field_info("float_field", 0, 2)?;
        let field_infos = FieldInfos::new(vec![field_info.clone()])?;
        write_accept_docs_segment(&dir, &seg_info, &field_info, &field_infos, ACCEPT_DOCS_SIZE)?;
        let io_ctx = DefaultIOContext::default();
        let read_state = SegmentReadState::new(dir.as_ref(), &seg_info, &field_infos, &io_ctx);
        let reader = Lucene99HnswVectorsFormat::new().fields_reader(&read_state)?;

        let mut zero = KCollector {
            inner: TopKnnCollector::new(1, i64::MAX),
            k: 0,
        };
        let mut all = from_live_docs(None, ACCEPT_DOCS_SIZE)?;
        reader.search("float_field", &[0.0, 0.0, 0.0], &mut zero, &mut all)?;
        assert_eq!(zero.visited_count(), 0);

        let mut one = KCollector {
            inner: TopKnnCollector::new(1, i64::MAX),
            k: 1,
        };
        let mut all = from_live_docs(None, ACCEPT_DOCS_SIZE)?;
        let error = reader
            .search("float_field", &[0.0, 0.0, 0.0], &mut one, &mut all)
            .expect_err("the scorer rejects the target");
        assert!(
            matches!(&error, LuceneError { kind: LuceneErrorKind::IllegalArgument { message, .. }, .. }
                if message == "vector query dimension: 3 differs from field dimension: 2"),
            "{error:?}"
        );
        Ok(())
    }

    #[test]
    fn the_graph_path_does_not_return_deleted_documents() -> Result<()> {
        let dir = Arc::new(ByteBuffersDirectory::new()) as Arc<dyn crate::store::Directory>;
        let seg_info = make_segment_info(&dir, ACCEPT_DOCS_SIZE)?;
        let field_info = float_field_info("float_field", 0, 2)?;
        let field_infos = FieldInfos::new(vec![field_info.clone()])?;
        write_accept_docs_segment(&dir, &seg_info, &field_info, &field_infos, ACCEPT_DOCS_SIZE)?;

        let io_ctx = DefaultIOContext::default();
        let read_state = SegmentReadState::new(dir.as_ref(), &seg_info, &field_infos, &io_ctx);
        let reader = Lucene99HnswVectorsFormat::new().fields_reader(&read_state)?;

        // Without deletions the three nearest vectors to [0, 0] are documents
        // 0, 1 and 2, which fixes what the deletions must displace.
        let mut all = from_live_docs(None, ACCEPT_DOCS_SIZE)?;
        let (docs, _) = accept_docs_search(reader.as_ref(), 3, &mut all)?;
        assert_eq!(docs, vec![0, 1, 2]);

        let mut with_deletions = from_live_docs(
            Some(live_docs_without(&[0, 1], ACCEPT_DOCS_SIZE)),
            ACCEPT_DOCS_SIZE,
        )?;
        let (docs, _) = accept_docs_search(reader.as_ref(), 3, &mut with_deletions)?;
        assert_eq!(
            docs,
            vec![2, 3, 4],
            "a deleted document must not come back from a kNN search"
        );

        reader.close()?;
        Ok(())
    }

    #[test]
    fn the_graph_path_honours_a_caller_filter() -> Result<()> {
        let dir = Arc::new(ByteBuffersDirectory::new()) as Arc<dyn crate::store::Directory>;
        let seg_info = make_segment_info(&dir, ACCEPT_DOCS_SIZE)?;
        let field_info = float_field_info("float_field", 0, 2)?;
        let field_infos = FieldInfos::new(vec![field_info.clone()])?;
        write_accept_docs_segment(&dir, &seg_info, &field_info, &field_infos, ACCEPT_DOCS_SIZE)?;

        let io_ctx = DefaultIOContext::default();
        let read_state = SegmentReadState::new(dir.as_ref(), &seg_info, &field_infos, &io_ctx);
        let reader = Lucene99HnswVectorsFormat::new().fields_reader(&read_state)?;

        // 54 of the 64 documents pass, which keeps the filtered count above the
        // `expectedVisitedNodes(3, 64) == 12` the graph search is expected to
        // touch, so the search stays on the graph path.
        let mut filtered = crate::search::from_iterator_supplier(
            Some(|| {
                Ok(Box::new(crate::search::range(10, ACCEPT_DOCS_SIZE)?)
                    as Box<dyn crate::search::DocIdSetIterator>)
            }),
            None,
            ACCEPT_DOCS_SIZE,
        )?;
        let (docs, _) = accept_docs_search(reader.as_ref(), 3, &mut filtered)?;
        assert_eq!(
            docs,
            vec![10, 11, 12],
            "a caller filter must be honoured by the graph search"
        );

        reader.close()?;
        Ok(())
    }

    #[test]
    fn the_exhaustive_path_does_not_return_deleted_documents() -> Result<()> {
        let dir = Arc::new(ByteBuffersDirectory::new()) as Arc<dyn crate::store::Directory>;
        let seg_info = make_segment_info(&dir, ACCEPT_DOCS_SIZE)?;
        let field_info = float_field_info("float_field", 0, 2)?;
        let field_infos = FieldInfos::new(vec![field_info.clone()])?;
        write_accept_docs_segment(&dir, &seg_info, &field_info, &field_infos, ACCEPT_DOCS_SIZE)?;

        let io_ctx = DefaultIOContext::default();
        let read_state = SegmentReadState::new(dir.as_ref(), &seg_info, &field_infos, &io_ctx);
        let reader = Lucene99HnswVectorsFormat::new().fields_reader(&read_state)?;

        // `k == numVectors` makes `doHnsw` false, so this is the exhaustive
        // scan rather than the graph search.
        let mut with_deletions = from_live_docs(
            Some(live_docs_without(&[0, 1], ACCEPT_DOCS_SIZE)),
            ACCEPT_DOCS_SIZE,
        )?;
        let (docs, visited) =
            accept_docs_search(reader.as_ref(), ACCEPT_DOCS_SIZE, &mut with_deletions)?;
        assert_eq!(docs.len(), (ACCEPT_DOCS_SIZE - 2) as usize);
        assert_eq!(
            docs[0], 2,
            "the exhaustive scan must skip deleted documents"
        );
        assert!(
            !docs.contains(&0) && !docs.contains(&1),
            "the exhaustive scan returned a deleted document: {docs:?}"
        );
        assert_eq!(
            visited,
            i64::from(ACCEPT_DOCS_SIZE - 2),
            "the exhaustive scan must only score the accepted ordinals"
        );

        reader.close()?;
        Ok(())
    }

    #[test]
    fn a_restrictive_filter_sends_the_search_down_the_exhaustive_path() -> Result<()> {
        let dir = Arc::new(ByteBuffersDirectory::new()) as Arc<dyn crate::store::Directory>;
        let seg_info = make_segment_info(&dir, ACCEPT_DOCS_SIZE)?;
        let field_info = float_field_info("float_field", 0, 2)?;
        let field_infos = FieldInfos::new(vec![field_info.clone()])?;
        write_accept_docs_segment(&dir, &seg_info, &field_info, &field_infos, ACCEPT_DOCS_SIZE)?;

        let io_ctx = DefaultIOContext::default();
        let read_state = SegmentReadState::new(dir.as_ref(), &seg_info, &field_infos, &io_ctx);
        let reader = Lucene99HnswVectorsFormat::new().fields_reader(&read_state)?;

        // Ten of the 64 documents pass, below the `expectedVisitedNodes(3, 64)
        // == 12` an unfiltered graph search would visit, so Java clears
        // `doHnsw` and scans exhaustively — visiting exactly the ten accepted
        // ordinals. Deciding against the unfiltered vector count instead, as
        // this port used to, keeps the search on the graph and visits a
        // different number of nodes.
        let mut filtered = crate::search::from_iterator_supplier(
            Some(|| {
                Ok(Box::new(crate::search::range(10, 20)?)
                    as Box<dyn crate::search::DocIdSetIterator>)
            }),
            None,
            ACCEPT_DOCS_SIZE,
        )?;
        let (docs, visited) = accept_docs_search(reader.as_ref(), 3, &mut filtered)?;
        assert_eq!(docs, vec![10, 11, 12]);
        assert_eq!(
            visited, 10,
            "the filtered document count must drive the choice of search path"
        );

        reader.close()?;
        Ok(())
    }
}

impl std::fmt::Display for Lucene99HnswVectorsFormat {
    /// Port of `Lucene99HnswVectorsFormat.toString()`.
    ///
    /// The `name=` is a literal in Java, not `getName()`. `flatVectorFormat` is
    /// the `Lucene99FlatVectorsFormat` the writer and reader are built from,
    /// which this port creates on demand in `flat_format()` rather than keeping
    /// in a field.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Lucene99HnswVectorsFormat(name=Lucene99HnswVectorsFormat, maxConn={}, beamWidth={}, tinySegmentsThreshold={}, flatVectorFormat={})",
            self.max_conn,
            self.beam_width,
            self.tiny_segments_threshold,
            self.flat_format()
        )
    }
}

#[cfg(test)]
mod graph_merger_choice_tests {
    use super::*;

    /// `Lucene99HnswVectorsWriter.createGraphMerger`
    /// (`Lucene99HnswVectorsWriter.java:628-647`): concurrent with the
    /// writer's `mergeExec`, or with an intra-merge executor and more than one
    /// worker.
    #[test]
    fn graph_merger_choice_follows_the_merge_executor_as_lucene_does() {
        let executor: Arc<dyn crate::search::Executor> =
            Arc::new(crate::util::SameThreadExecutorService::new());
        assert_eq!(
            choose_graph_merger(None, Some(&executor), 2),
            GraphMergerChoice::Concurrent
        );
        assert_eq!(
            choose_graph_merger(None, Some(&executor), 1),
            GraphMergerChoice::Incremental
        );
        assert_eq!(
            choose_graph_merger(None, None, 4),
            GraphMergerChoice::Incremental
        );
        // The writer's own `mergeExec` wins whatever the merge offers
        // (`Lucene99HnswVectorsWriter.java:633-636`).
        let merge_exec = Arc::new(TaskExecutor::new(Arc::clone(&executor)));
        assert_eq!(
            choose_graph_merger(Some(&merge_exec), None, 4),
            GraphMergerChoice::Concurrent
        );
    }

    /// `Lucene99HnswVectorsFormat(int, int, int, ExecutorService)` rejects an
    /// executor for a single worker (`Lucene99HnswVectorsFormat.java:286-289`)
    /// and keeps one for several (`:291-295`).
    #[test]
    #[should_panic(expected = "No executor service is needed as we'll use single thread to merge")]
    fn a_single_worker_format_rejects_a_merge_executor() {
        let executor: Arc<dyn crate::search::Executor> =
            Arc::new(crate::util::SameThreadExecutorService::new());
        let format =
            Lucene99HnswVectorsFormat::with_merge_exec(16, 100, 2, Some(Arc::clone(&executor)));
        assert!(format.merge_exec.is_some());
        assert!(Lucene99HnswVectorsFormat::with_merge_exec(16, 100, 2, None)
            .merge_exec
            .is_none());
        let _ = Lucene99HnswVectorsFormat::with_merge_exec(16, 100, 1, Some(executor));
    }
}

/// The `IOUtils` close family at the HNSW and flat vectors call sites (#308).
#[cfg(test)]
mod close_family_tests {
    use super::Lucene99HnswVectorsFormat;
    use crate::codecs::knn_vectors::KnnVectorsFormat;
    use crate::codecs::tests::{
        owned_write_state, read_state, rewrite_file, test_segment_info, tracking_directory,
    };
    use crate::index::FieldInfos;

    fn closed_of(log: &crate::codecs::tests::CloseLog, names: &[&str]) -> Vec<String> {
        log.closed()
            .into_iter()
            .filter(|n| names.contains(&n.as_str()))
            .collect()
    }

    fn write_empty(
        dir: &std::sync::Arc<dyn crate::store::Directory>,
        info: &crate::index::SegmentInfo,
    ) {
        let mut writer = Lucene99HnswVectorsFormat::new()
            .fields_writer(&owned_write_state(dir, info))
            .unwrap();
        writer.finish().unwrap();
        writer.close().unwrap();
    }

    const ALL: [&str; 4] = ["_0.vem", "_0.vex", "_0.vemf", "_0.vec"];

    /// The flat writer's constructor closes the `.vemf` it created when the
    /// `.vec` cannot be: `IOUtils.closeWhileHandlingException(this)`
    /// (`Lucene99FlatVectorsWriter.java:132-136`).
    #[test]
    fn a_failed_flat_writer_constructor_closes_its_meta_output() {
        let (dir, _tracking, log) = tracking_directory();
        let info = test_segment_info("_0", 10);
        log.fail_open("_0.vec");
        Lucene99HnswVectorsFormat::new()
            .fields_writer(&owned_write_state(&dir, &info))
            .unwrap_err();
        assert_eq!(closed_of(&log, &ALL), ["_0.vemf"]);
    }

    /// The HNSW writer's constructor runs `IOUtils.closeWhileHandlingException(this)`
    /// (`Lucene99HnswVectorsWriter.java:187-190`): `close()` closes the `.vem`
    /// and the flat writer it was handed, whose `.vemf` and `.vec` close.
    #[test]
    fn a_failed_hnsw_writer_constructor_closes_meta_and_the_flat_writer() {
        let (dir, _tracking, log) = tracking_directory();
        let info = test_segment_info("_0", 10);
        log.fail_open("_0.vex");
        Lucene99HnswVectorsFormat::new()
            .fields_writer(&owned_write_state(&dir, &info))
            .unwrap_err();
        assert_eq!(closed_of(&log, &ALL), ["_0.vem", "_0.vemf", "_0.vec"]);
    }

    /// `close()` is `IOUtils.close(meta, vectorIndex, flatVectorWriter,
    /// flatVectorsReader)` (`Lucene99HnswVectorsWriter.java:648-656`), and the
    /// flat writer's is `IOUtils.close(meta, vectorData)`
    /// (`Lucene99FlatVectorsWriter.java:353-356`): everything is closed in
    /// that order and the first failure is raised.
    #[test]
    fn writer_close_closes_everything_in_order_and_raises_the_first_failure() {
        let (dir, _tracking, log) = tracking_directory();
        let info = test_segment_info("_0", 10);
        let mut writer = Lucene99HnswVectorsFormat::new()
            .fields_writer(&owned_write_state(&dir, &info))
            .unwrap();
        writer.finish().unwrap();
        for name in ["_0.vem", "_0.vex", "_0.vemf"] {
            log.fail_close(name);
        }
        let error = writer.close().unwrap_err();
        assert!(format!("{error}").contains("_0.vem"), "{error}");
        assert!(!format!("{error}").contains("_0.vex"), "{error}");
        assert_eq!(closed_of(&log, &ALL), ALL);
    }

    /// A bad `.vec` header closes the `.vec` input:
    /// `IOUtils.closeWhileHandlingException(in)`
    /// (`Lucene99FlatVectorsReader.java:169-173`).
    #[test]
    fn a_bad_flat_data_header_closes_the_flat_input() {
        let (dir, tracking, log) = tracking_directory();
        let info = test_segment_info("_0", 10);
        let fis = FieldInfos::default();
        write_empty(&dir, &info);
        let before = closed_of(&log, &ALL).len();
        rewrite_file(&tracking, "_0.vec", |bytes| bytes[6] ^= 0xff);
        Lucene99HnswVectorsFormat::new()
            .fields_reader(&read_state(&*dir, &info, &fis))
            .unwrap_err();
        // The metadata input is closed first, by its try-with-resources.
        assert_eq!(closed_of(&log, &ALL)[before..], ["_0.vemf", "_0.vec"]);
    }

    /// A bad `.vex` header closes the `.vex` input
    /// (`Lucene99HnswVectorsReader.java:188-192`), then
    /// `IOUtils.closeWhileHandlingException(this)` closes the flat reader the
    /// constructor was handed (`Lucene99HnswVectorsReader.java:129-133`).
    #[test]
    fn a_failed_hnsw_reader_constructor_closes_the_index_and_the_flat_reader() {
        let (dir, tracking, log) = tracking_directory();
        let info = test_segment_info("_0", 10);
        let fis = FieldInfos::default();
        write_empty(&dir, &info);
        let before = closed_of(&log, &ALL).len();
        rewrite_file(&tracking, "_0.vex", |bytes| bytes[6] ^= 0xff);
        Lucene99HnswVectorsFormat::new()
            .fields_reader(&read_state(&*dir, &info, &fis))
            .unwrap_err();
        // The metadata input is closed first, by its try-with-resources.
        assert_eq!(
            closed_of(&log, &ALL)[before..],
            ["_0.vemf", "_0.vem", "_0.vex", "_0.vec"]
        );
    }

    /// `close()` is `IOUtils.close(flatVectorsReader, vectorIndex)`
    /// (`Lucene99HnswVectorsReader.java:424-427`).
    #[test]
    fn reader_close_closes_the_index_past_a_flat_reader_failure() {
        let (dir, _tracking, log) = tracking_directory();
        let info = test_segment_info("_0", 10);
        let fis = FieldInfos::default();
        write_empty(&dir, &info);
        let reader = Lucene99HnswVectorsFormat::new()
            .fields_reader(&read_state(&*dir, &info, &fis))
            .unwrap();
        let before = closed_of(&log, &ALL).len();
        log.fail_close("_0.vec");
        let error = reader.close().unwrap_err();
        assert!(format!("{error}").contains("_0.vec"), "{error}");
        assert_eq!(closed_of(&log, &ALL)[before..], ["_0.vec", "_0.vex"]);
    }
}
