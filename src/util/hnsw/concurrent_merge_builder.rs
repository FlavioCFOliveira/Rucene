//! Port of `org.apache.lucene.util.hnsw.HnswConcurrentMergeBuilder`.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::internal::hppc::IntHashSet;
use crate::util::FixedBitSet;

use super::builder::{HnswBuilder, HnswGraphBuilder, DEFAULT_RAND_SEED};
use super::hnsw_lock::HnswLock;
use super::on_heap::OnHeapHnswGraph;
use super::scorer::RandomVectorScorerSupplier;

/// Number of vectors a worker handles sequentially in one batch.
///
/// Equivalent to `HnswConcurrentMergeBuilder.DEFAULT_BATCH_SIZE`.
const DEFAULT_BATCH_SIZE: i32 = 2048;

/// A graph builder that manages multiple workers; it only supports adding the whole
/// graph at once.
///
/// Equivalent to `org.apache.lucene.util.hnsw.HnswConcurrentMergeBuilder`. The
/// workers pick up work in batches, reserving each batch by atomically advancing a
/// shared counter, and the graph they share is guarded by the striped
/// [`HnswLock`] at the granularity Lucene uses: one write lock per
/// `(level, neighbour)` while the back-links are updated.
///
/// # Divergences from Lucene 10.5.0
///
/// * Java hands the workers to a `TaskExecutor`, which runs them on separate
///   threads. `OnHeapHnswGraph` is not `Sync` in this port — it is mutated through
///   `&mut self` — and the vector-scorer traits carry no `Send` bound, so the workers
///   cannot be moved onto threads without redesigning components outside this port's
///   scope. The batches are therefore reserved through the same shared
///   [`AtomicI32`] and executed on the calling thread, which produces a correct
///   graph and keeps the work-reservation protocol and the locking granularity;
///   what it does not reproduce is the wall-clock speed-up. Note that Lucene's
///   concurrent merge is itself non-deterministic: the graph it produces depends on
///   how the threads interleave.
/// * Java's `setInfoStream` forwards a debugging stream to each worker; this port's
///   [`HnswGraphBuilder`] has no info stream, so there is nothing to forward.
pub struct HnswConcurrentMergeBuilder {
    builder: HnswGraphBuilder,
    /// Striped locks shared by every worker, exactly as Lucene shares them.
    hnsw_lock: Arc<HnswLock>,
    /// A common counter shared among all workers, tracking the next vector to add.
    work_progress: Arc<AtomicI32>,
    initialized_nodes: Option<FixedBitSet>,
    batch_size: i32,
    num_worker: i32,
    frozen: bool,
}

impl HnswConcurrentMergeBuilder {
    /// Creates a merge builder driving `num_worker` workers over `hnsw`.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying builder cannot be created.
    pub fn new(
        num_worker: i32,
        scorer_supplier: &dyn RandomVectorScorerSupplier,
        m: i32,
        beam_width: i32,
        hnsw: OnHeapHnswGraph,
        initialized_nodes: Option<FixedBitSet>,
    ) -> Result<Self> {
        let hnsw_lock = Arc::new(HnswLock::new());
        let mut builder = HnswGraphBuilder::with_graph(
            scorer_supplier.scorer()?,
            m,
            beam_width,
            DEFAULT_RAND_SEED,
            hnsw,
        )?;
        builder.set_hnsw_lock(Arc::clone(&hnsw_lock));
        Ok(Self {
            builder,
            hnsw_lock,
            work_progress: Arc::new(AtomicI32::new(0)),
            initialized_nodes,
            batch_size: DEFAULT_BATCH_SIZE,
            num_worker: num_worker.max(1),
            frozen: false,
        })
    }

    /// Returns the striped locks the workers share.
    pub fn hnsw_lock(&self) -> &Arc<HnswLock> {
        &self.hnsw_lock
    }

    /// Number of workers this builder drives.
    pub fn num_worker(&self) -> i32 {
        self.num_worker
    }

    /// Sets the batch size; test-only, as in Lucene.
    pub fn set_batch_size(&mut self, new_size: i32) {
        self.batch_size = new_size;
    }

    /// Reserves a batch of work by atomically advancing the shared counter.
    ///
    /// Equivalent to `ConcurrentMergeWorker.getStartPos`.
    fn get_start_pos(&self, max_ord: i32) -> i32 {
        let start = self
            .work_progress
            .fetch_add(self.batch_size, Ordering::AcqRel);
        if start < max_ord {
            start
        } else {
            -1
        }
    }

    /// Adds every node in `[start, end)`, skipping the ones already initialized.
    ///
    /// Equivalent to `ConcurrentMergeWorker.addVectors` together with its
    /// `addGraphNode` override.
    fn add_vectors(&mut self, start: i32, end: i32) -> Result<()> {
        for node in start..end {
            if let Some(initialized) = &self.initialized_nodes {
                if initialized.get(node as usize) {
                    continue;
                }
            }
            self.builder.add_graph_node(node)?;
        }
        Ok(())
    }
}

impl HnswBuilder for HnswConcurrentMergeBuilder {
    fn build(&mut self, max_ord: i32) -> Result<&OnHeapHnswGraph> {
        if self.frozen {
            return Err(LuceneError::IllegalState(
                "graph has already been built".to_string(),
            ));
        }
        let mut start = self.get_start_pos(max_ord);
        while start != -1 {
            let end = max_ord.min(start + self.batch_size);
            self.add_vectors(start, end)?;
            start = self.get_start_pos(max_ord);
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
        self.builder.get_graph()
    }

    fn get_completed_graph(&mut self) -> Result<&OnHeapHnswGraph> {
        if !self.frozen {
            // Should already have been done in build(), but just in case.
            self.builder.finish();
            self.frozen = true;
        }
        Ok(self.builder.get_graph())
    }

    fn into_completed_graph(mut self: Box<Self>) -> Result<OnHeapHnswGraph> {
        self.get_completed_graph()?;
        Ok(self.builder.into_graph())
    }
}
