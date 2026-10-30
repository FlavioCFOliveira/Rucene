//! Port of `org.apache.lucene.util.hnsw.HnswConcurrentMergeBuilder`.

use std::sync::atomic::{AtomicI32, AtomicI64, Ordering};
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::merge_scheduler::now_ns;
use crate::internal::hppc::IntHashSet;
use crate::search::TaskExecutor;
use crate::util::{FixedBitSet, InfoStream};

use super::builder::{
    check_params, HnswBuilder, HnswGraphBuilder, DEFAULT_RAND_SEED, HNSW_COMPONENT,
};
use super::on_heap::OnHeapHnswGraph;
use super::scorer::RandomVectorScorerSupplier;

/// Number of vectors a worker handles sequentially in one batch.
///
/// Equivalent to `HnswConcurrentMergeBuilder.DEFAULT_BATCH_SIZE`.
const DEFAULT_BATCH_SIZE: i32 = 2048;

/// A graph builder that manages multiple workers; it only supports adding the whole
/// graph at once.
///
/// Equivalent to `org.apache.lucene.util.hnsw.HnswConcurrentMergeBuilder`.
/// [`build`](HnswBuilder::build) hands one task per worker to the
/// [`TaskExecutor`], exactly as Java's `taskExecutor.invokeAll(futures)` does;
/// each worker reserves batches of `batch_size` ordinals by atomically
/// advancing a shared counter and inserts them into the one shared graph, so
/// the workers finish at about the same time. As in Lucene, the graph a
/// concurrent merge produces depends on how the threads interleave.
///
/// # Divergences from Lucene 10.5.0
///
/// * The workers share the graph through the locks of [`OnHeapHnswGraph`] —
///   one per neighbour array, the inserted node's own array included — rather
///   than through the 512 striped [`HnswLock`](super::HnswLock)s Java's
///   workers take only for a neighbour's array; that type's documentation
///   gives the reason (Java's unlocked writes are data races in Rust).
/// * Java builds the `ConcurrentMergeWorker`s in the constructor, each from its
///   own `scorerSupplier.copy()`, and runs them later. A worker's scorer is not
///   required to be [`Send`], so a worker cannot be moved onto a pool thread;
///   the constructor takes the supplier copies, as Java does, and each task
///   builds its worker from its own copy on the thread that runs it. A worker is
///   used by exactly one `build`, so its state — a fresh level generator seeded
///   with `randSeed`, a fresh searcher, the info stream and the batch size this
///   builder holds — is the state Java's worker has when its `run` starts. A
///   scorer the supplier cannot create is reported by `build` rather than by the
///   constructor.
pub struct HnswConcurrentMergeBuilder {
    /// Runs the workers.
    ///
    /// Equivalent to `HnswConcurrentMergeBuilder.taskExecutor`.
    task_executor: Arc<TaskExecutor>,
    /// One scorer supplier per worker, each a `copy()` of the merge's supplier,
    /// consumed by the `build` that runs the workers.
    worker_suppliers: Vec<Box<dyn RandomVectorScorerSupplier>>,
    m: i32,
    beam_width: i32,
    /// The graph every worker inserts into.
    hnsw: Arc<OnHeapHnswGraph>,
    /// A common counter shared among all workers, tracking the next vector to add.
    work_progress: Arc<AtomicI32>,
    initialized_nodes: Option<Arc<FixedBitSet>>,
    batch_size: i32,
    num_worker: i32,
    /// Where this builder reports the merge as a whole.
    ///
    /// Equivalent to `HnswConcurrentMergeBuilder.infoStream`, which defaults
    /// to `InfoStream.getDefault()`; [`set_info_stream`](HnswBuilder::set_info_stream)
    /// also forwards it to the workers, as Java's does.
    info_stream: Arc<dyn InfoStream>,
    frozen: bool,
}

impl HnswConcurrentMergeBuilder {
    /// Creates a merge builder driving `num_worker` workers over `hnsw`, on
    /// `task_executor`.
    ///
    /// Equivalent to `HnswConcurrentMergeBuilder(TaskExecutor, int,
    /// RandomVectorScorerSupplier, int, int, OnHeapHnswGraph, BitSet)`.
    ///
    /// # Errors
    ///
    /// Returns an error if `m` or `beam_width` is not positive, which Java's
    /// worker constructors reject, or if copying the scorer supplier fails.
    pub fn new(
        task_executor: Arc<TaskExecutor>,
        num_worker: i32,
        scorer_supplier: &dyn RandomVectorScorerSupplier,
        m: i32,
        beam_width: i32,
        hnsw: OnHeapHnswGraph,
        initialized_nodes: Option<FixedBitSet>,
    ) -> Result<Self> {
        check_params(m, beam_width)?;
        let num_worker = num_worker.max(1);
        let worker_suppliers = (0..num_worker)
            .map(|_| scorer_supplier.copy())
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            task_executor,
            worker_suppliers,
            m,
            beam_width,
            hnsw: Arc::new(hnsw),
            work_progress: Arc::new(AtomicI32::new(0)),
            initialized_nodes: initialized_nodes.map(Arc::new),
            batch_size: DEFAULT_BATCH_SIZE,
            num_worker,
            info_stream: Arc::new(crate::util::NoOutputInfoStream),
            frozen: false,
        })
    }

    /// Number of workers this builder drives.
    pub fn num_worker(&self) -> i32 {
        self.num_worker
    }

    /// Sets the batch size; test-only, as in Lucene.
    pub fn set_batch_size(&mut self, new_size: i32) {
        self.batch_size = new_size;
    }

    /// Freezes the workers' builders.
    ///
    /// Equivalent to the private `HnswConcurrentMergeBuilder.finish()`
    /// (`HnswConcurrentMergeBuilder.java:149-151`), which calls
    /// `workers[0].finish()`. That only marks worker 0 frozen — Lucene 10.5.0
    /// has the component repair commented out of `HnswGraphBuilder.finish()` —
    /// and the workers here live only for the `build` that runs them, so there
    /// is no worker left to mark; `frozen` on this builder is the state that
    /// refuses further work.
    fn finish(&mut self) {}
}

/// One worker of a concurrent merge.
///
/// Equivalent to the private
/// `HnswConcurrentMergeBuilder.ConcurrentMergeWorker`, which extends
/// `HnswGraphBuilder`; this port holds one instead, because Rust has no
/// implementation inheritance. Its `addGraphNode` override, which skips a node
/// the initializer graph already placed, is the `skip` argument of
/// [`HnswGraphBuilder::add_vectors_skipping`], so progress is still reported on
/// the inherited `addVectors` path.
struct ConcurrentMergeWorker {
    builder: HnswGraphBuilder,
    /// A common counter shared among all workers, tracking the next vector to add.
    work_progress: Arc<AtomicI32>,
    initialized_nodes: Option<Arc<FixedBitSet>>,
    batch_size: i32,
}

impl ConcurrentMergeWorker {
    /// Reserves batches of work and adds their nodes until none is left.
    ///
    /// Equivalent to `ConcurrentMergeWorker.run(int)`.
    fn run(&mut self, max_ord: i32) -> Result<()> {
        let mut start = self.get_start_pos(max_ord);
        while start != -1 {
            let end = max_ord.min(start + self.batch_size);
            self.builder
                .add_vectors_skipping(start, end, self.initialized_nodes.as_deref())?;
            start = self.get_start_pos(max_ord);
        }
        Ok(())
    }

    /// Reserves a batch of work by atomically advancing the shared counter.
    ///
    /// Equivalent to `ConcurrentMergeWorker.getStartPos`.
    fn get_start_pos(&self, max_ord: i32) -> i32 {
        // Only the atomicity of the increment matters: the counter hands out
        // disjoint ranges and orders nothing else.
        let start = self
            .work_progress
            .fetch_add(self.batch_size, Ordering::Relaxed);
        if start < max_ord {
            start
        } else {
            -1
        }
    }
}

impl HnswBuilder for HnswConcurrentMergeBuilder {
    fn build(&mut self, max_ord: i32) -> Result<&OnHeapHnswGraph> {
        if self.frozen {
            return Err(LuceneError::illegal_state(
                "graph has already been built".to_string(),
            ));
        }
        let merge_start_time_ns = now_ns();
        if self.info_stream.is_enabled(HNSW_COMPONENT) {
            self.info_stream.message(
                HNSW_COMPONENT,
                &format!(
                    "build graph from {max_ord} vectors, with {} workers",
                    self.num_worker
                ),
            );
        }
        let cumulative_work_time_ns = Arc::new(AtomicI64::new(0));
        type WorkerTask = Box<dyn FnOnce() -> Result<()> + Send + 'static>;
        let tasks: Vec<WorkerTask> = std::mem::take(&mut self.worker_suppliers)
            .into_iter()
            .map(|scorer_supplier| {
                let hnsw = Arc::clone(&self.hnsw);
                let work_progress = Arc::clone(&self.work_progress);
                let initialized_nodes = self.initialized_nodes.clone();
                let info_stream = Arc::clone(&self.info_stream);
                let cumulative_work_time_ns = Arc::clone(&cumulative_work_time_ns);
                let (m, beam_width, batch_size) = (self.m, self.beam_width, self.batch_size);
                Box::new(move || {
                    let mut builder = HnswGraphBuilder::with_shared_graph(
                        scorer_supplier.scorer()?,
                        m,
                        beam_width,
                        DEFAULT_RAND_SEED,
                        hnsw,
                    )?;
                    builder.set_info_stream(info_stream);
                    builder.set_merge_start_time_ns(merge_start_time_ns);
                    builder.set_cumulative_work_time_ns(Some(cumulative_work_time_ns));
                    ConcurrentMergeWorker {
                        builder,
                        work_progress,
                        initialized_nodes,
                        batch_size,
                    }
                    .run(max_ord)
                }) as WorkerTask
            })
            .collect();
        self.task_executor.invoke_all(tasks)?;
        if self.info_stream.is_enabled(HNSW_COMPONENT) {
            let wall_clock_ms = (now_ns() - merge_start_time_ns) as f64 / 1_000_000.0;
            let total_worker_ms =
                cumulative_work_time_ns.load(Ordering::Relaxed) as f64 / 1_000_000.0;
            let effective_concurrency = if wall_clock_ms > 0.0 {
                total_worker_ms / wall_clock_ms
            } else {
                0.0
            };
            self.info_stream.message(
                HNSW_COMPONENT,
                &format!(
                    "merge completed: {max_ord} vectors, {wall_clock_ms:.2} ms wall clock, \
                     {total_worker_ms:.2} ms cumulative worker time, \
                     {effective_concurrency:.2}x effective concurrency"
                ),
            );
        }
        self.get_completed_graph()
    }

    fn add_graph_node(&mut self, _node: i32) -> Result<()> {
        Err(LuceneError::UnsupportedOperation(
            "This builder is for merge only".to_string(),
        ))
    }

    fn add_graph_node_with_eps(&mut self, _node: i32, _eps: &IntHashSet) -> Result<()> {
        Err(LuceneError::UnsupportedOperation(
            "This builder is for merge only".to_string(),
        ))
    }

    fn get_graph(&self) -> &OnHeapHnswGraph {
        &self.hnsw
    }

    /// Equivalent to `HnswConcurrentMergeBuilder.setInfoStream`, which also
    /// forwards the stream to every worker: each worker is handed this
    /// builder's stream when its task builds it.
    fn set_info_stream(&mut self, info_stream: Arc<dyn InfoStream>) {
        self.info_stream = info_stream;
    }

    fn get_completed_graph(&mut self) -> Result<&OnHeapHnswGraph> {
        if !self.frozen {
            // Should already have been done in build(), but just in case.
            self.finish();
            self.frozen = true;
        }
        Ok(&self.hnsw)
    }

    fn into_completed_graph(mut self: Box<Self>) -> Result<OnHeapHnswGraph> {
        self.get_completed_graph()?;
        Arc::try_unwrap(self.hnsw).map_err(|_| {
            LuceneError::illegal_state("the graph is still shared with a worker".to_string())
        })
    }
}
