//! HNSW graph construction.
//!
//! Equivalent to `org.apache.lucene.util.hnsw.HnswGraphBuilder` and
//! `org.apache.lucene.util.hnsw.HnswBuilder`.

#![deny(unsafe_code)]

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use crate::error::{LuceneError, Result};
use crate::index::merge_scheduler::now_ns;
use crate::internal::hppc::IntHashSet;
use crate::search::knn::KnnCollector;
use crate::search::NO_MORE_DOCS;
use crate::util::bit_set::FixedBits;
use crate::util::hnsw::on_heap::{OnHeapGraphReader, OnHeapHnswGraph};
use crate::util::hnsw::scorer::{RandomVectorScorerSupplier, UpdateableRandomVectorScorer};
use crate::util::hnsw::searcher::HnswGraphSearcher;
use crate::util::hnsw::util::HnswUtil;
use crate::util::hnsw::{HnswGraph, NeighborArray, NeighborQueue};
use crate::util::{FixedBitSet, InfoStream};

/// Default number of maximum connections per node on upper levels.
pub const DEFAULT_MAX_CONN: i32 = 16;

/// Default beam width (`efConstruction`).
pub const DEFAULT_BEAM_WIDTH: i32 = 100;

/// Default seed for reproducible level generation.
pub const DEFAULT_RAND_SEED: u64 = 42;

const MAX_BULK_SCORE_NODES: usize = 8;

/// Builder interface for `OnHeapHnswGraph`.
///
/// Equivalent to `org.apache.lucene.util.hnsw.HnswBuilder`.
pub trait HnswBuilder {
    /// Adds all nodes in `[0, max_ord)` and returns the completed graph.
    fn build(&mut self, max_ord: i32) -> Result<&OnHeapHnswGraph>;

    /// Inserts a single node.
    fn add_graph_node(&mut self, node: i32) -> Result<()>;

    /// Inserts a single node, searching level 0 with the provided entry points.
    ///
    /// Equivalent to `HnswBuilder.addGraphNode(int, IntHashSet)`.
    fn add_graph_node_with_eps(&mut self, node: i32, eps: &IntHashSet) -> Result<()>;

    /// Sets where the builder reports its progress.
    ///
    /// Equivalent to `org.apache.lucene.util.hnsw.HnswBuilder#setInfoStream`
    /// (`HnswBuilder.java:47`), which every implementation supplies.
    fn set_info_stream(&mut self, info_stream: Arc<dyn InfoStream>);

    /// Returns the partially built graph.
    fn get_graph(&self) -> &OnHeapHnswGraph;

    /// Freezes the builder and returns the finished graph.
    ///
    /// Once this method is called, no further updates to the graph are accepted.
    ///
    /// # Errors
    ///
    /// Returns an error if the final modifications to the graph fail.
    fn get_completed_graph(&mut self) -> Result<&OnHeapHnswGraph>;

    /// Consumes the builder and hands back the finished graph.
    ///
    /// Java returns the graph by reference from `getCompletedGraph()` and lets the
    /// garbage collector keep the builder alive; Rust needs an explicit way to move
    /// the graph out of a boxed builder.
    ///
    /// # Errors
    ///
    /// Returns an error if the final modifications to the graph fail.
    fn into_completed_graph(self: Box<Self>) -> Result<OnHeapHnswGraph>;
}

/// Builds an in-memory HNSW graph.
///
/// Equivalent to `org.apache.lucene.util.hnsw.HnswGraphBuilder`.
pub struct HnswGraphBuilder {
    pub(crate) m: i32,
    ml: f64,
    bulk_score_nodes: [i32; MAX_BULK_SCORE_NODES],
    bulk_scores: [f32; MAX_BULK_SCORE_NODES],
    random: SplittableRandom,
    pub(crate) scorer: Box<dyn UpdateableRandomVectorScorer>,
    pub(crate) graph_searcher: HnswGraphSearcher,
    entry_candidates: GraphBuilderKnnCollector,
    pub(crate) beam_candidates: GraphBuilderKnnCollector,
    beam_candidates0: GraphBuilderKnnCollector,
    /// The graph being built, shared with the other workers of a concurrent
    /// merge and owned alone otherwise.
    ///
    /// Equivalent to `HnswGraphBuilder.hnsw`. Java's builder also keeps the
    /// `hnswLock` its workers lock the graph with; this port's graph locks
    /// each neighbour array itself (see [`OnHeapHnswGraph`]), so the builder
    /// holds no lock of its own.
    pub(crate) hnsw: Arc<OnHeapHnswGraph>,
    /// The buffer the graph searches copy each neighbour list into.
    ///
    /// Equivalent to `HnswConcurrentMergeBuilder.MergeSearcher.nodeBuffer`;
    /// see [`OnHeapGraphReader`].
    node_buffer: Vec<i32>,
    pub(crate) frozen: bool,
    /// Where the builder reports its progress.
    ///
    /// Equivalent to `HnswGraphBuilder.infoStream`, which defaults to
    /// `InfoStream.getDefault()`.
    info_stream: Arc<dyn InfoStream>,
    /// When the merge this builder works for began, as a [`now_ns`] reading,
    /// or `-1` when it builds on its own; progress reports measure from it
    /// when it is set.
    ///
    /// Equivalent to `HnswGraphBuilder.mergeStartTimeNs`.
    merge_start_time_ns: i64,
    /// The accumulator every concurrent merge worker adds its elapsed time
    /// to, or `None` on the non-concurrent path.
    ///
    /// Equivalent to `HnswGraphBuilder.cumulativeWorkTimeNs`.
    cumulative_work_time_ns: Option<Arc<AtomicI64>>,
}

/// The component name the builder logs under.
///
/// Equivalent to `HnswGraphBuilder.HNSW_COMPONENT`.
pub const HNSW_COMPONENT: &str = "HNSW";

/// Rejects the parameters `HnswGraphBuilder`'s constructor rejects.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when `m` or `beam_width` is not
/// positive.
pub(crate) fn check_params(m: i32, beam_width: i32) -> Result<()> {
    if m <= 0 {
        return Err(LuceneError::IllegalArgument(
            "M (max connections) must be positive".to_string(),
        ));
    }
    if beam_width <= 0 {
        return Err(LuceneError::IllegalArgument(
            "beamWidth must be positive".to_string(),
        ));
    }
    Ok(())
}

impl HnswGraphBuilder {
    /// Creates a builder with a known graph size (-1 means unbounded).
    pub fn create(
        scorer_supplier: &dyn RandomVectorScorerSupplier,
        m: i32,
        beam_width: i32,
        seed: u64,
        graph_size: i32,
    ) -> Result<Self> {
        let hnsw = OnHeapHnswGraph::new(m, graph_size);
        let scorer = scorer_supplier.scorer()?;
        Self::with_graph(scorer, m, beam_width, seed, hnsw)
    }

    /// Creates a builder from an existing graph.
    pub fn with_graph(
        scorer: Box<dyn UpdateableRandomVectorScorer>,
        m: i32,
        beam_width: i32,
        seed: u64,
        hnsw: OnHeapHnswGraph,
    ) -> Result<Self> {
        Self::with_shared_graph(scorer, m, beam_width, seed, Arc::new(hnsw))
    }

    /// Creates a builder over a graph other builders may share.
    ///
    /// Equivalent to the `HnswGraphBuilder` constructor that
    /// `HnswConcurrentMergeBuilder.ConcurrentMergeWorker` calls with the
    /// shared `hnsw` (`HnswGraphBuilder.java:176-197`).
    pub(crate) fn with_shared_graph(
        scorer: Box<dyn UpdateableRandomVectorScorer>,
        m: i32,
        beam_width: i32,
        seed: u64,
        hnsw: Arc<OnHeapHnswGraph>,
    ) -> Result<Self> {
        check_params(m, beam_width)?;
        let ml = if m == 1 { 1.0 } else { 1.0 / (m as f64).ln() };
        let graph_searcher = HnswGraphSearcher::new(
            NeighborQueue::new(beam_width, true)?,
            Box::new(crate::util::FixedBitSet::new(
                hnsw.max_node_id().max(0) as usize + 1,
            )),
        );
        Ok(Self {
            m,
            ml,
            bulk_score_nodes: [0; MAX_BULK_SCORE_NODES],
            bulk_scores: [0.0; MAX_BULK_SCORE_NODES],
            random: SplittableRandom::new(seed),
            scorer,
            graph_searcher,
            entry_candidates: GraphBuilderKnnCollector::new(1)?,
            beam_candidates: GraphBuilderKnnCollector::new(beam_width)?,
            // With a beam width of 1 this is a collector of 0, which Java's
            // constructor rejects too (`HnswGraphBuilder.java:211-213`).
            beam_candidates0: GraphBuilderKnnCollector::new((beam_width / 2).min(m * 3))?,
            hnsw,
            node_buffer: Vec::new(),
            frozen: false,
            info_stream: Arc::new(crate::util::NoOutputInfoStream),
            merge_start_time_ns: -1,
            cumulative_work_time_ns: None,
        })
    }

    /// Consumes the builder and returns the constructed graph.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when another builder still shares
    /// the graph, which happens only while a concurrent merge's workers are
    /// alive.
    pub fn into_graph(mut self) -> Result<OnHeapHnswGraph> {
        if !self.frozen {
            self.finish();
        }
        Arc::try_unwrap(self.hnsw).map_err(|_| {
            LuceneError::illegal_state("the graph is still shared with another builder".to_string())
        })
    }

    /// Adds `node` to the graph on `level`.
    ///
    /// Equivalent to the `hnsw.addNode(level, node)` of
    /// `HnswGraphBuilder.addGraphNodeInternal`. A builder that owns its graph
    /// alone calls the full [`OnHeapHnswGraph::add_node`]; a concurrent-merge
    /// worker shares it and inserts through
    /// [`OnHeapHnswGraph::add_node_shared`].
    ///
    /// # Errors
    ///
    /// Propagates the shared insertion's refusal to grow a shared graph.
    pub(crate) fn add_node_to_graph(&mut self, level: i32, node: i32) -> Result<()> {
        match Arc::get_mut(&mut self.hnsw) {
            Some(graph) => {
                graph.add_node(level, node);
                Ok(())
            }
            None => self.hnsw.add_node_shared(level, node),
        }
    }

    pub(crate) fn finish(&mut self) {
        self.frozen = true;
    }

    /// Sets where the builder reports its progress.
    ///
    /// Equivalent to `HnswGraphBuilder.setInfoStream(InfoStream)`.
    pub fn set_info_stream(&mut self, info_stream: Arc<dyn InfoStream>) {
        self.info_stream = info_stream;
    }

    /// Sets the merge-level start time, so that progress reports show the
    /// time since the merge began rather than since this chunk began.
    ///
    /// Equivalent to `HnswGraphBuilder#setMergeStartTimeNs`
    /// (`HnswGraphBuilder.java:102-104`). `merge_start_time_ns` is a
    /// [`now_ns`] reading, the port's `System.nanoTime()`.
    pub fn set_merge_start_time_ns(&mut self, merge_start_time_ns: i64) {
        self.merge_start_time_ns = merge_start_time_ns;
    }

    /// Sets the shared accumulator that tracks the time spent adding vectors
    /// across concurrent chunks.
    ///
    /// Equivalent to `HnswGraphBuilder#setCumulativeWorkTimeNs`
    /// (`HnswGraphBuilder.java:107-109`); `None` is Java's `null`.
    pub fn set_cumulative_work_time_ns(&mut self, cumulative_work_time_ns: Option<Arc<AtomicI64>>) {
        self.cumulative_work_time_ns = cumulative_work_time_ns;
    }

    /// Logs how long the build has taken to reach `node`, measured from the
    /// [`now_ns`] reading `start_ns`.
    ///
    /// Equivalent to the private
    /// `HnswGraphBuilder.printGraphBuildStatus(int, long)`.
    fn print_graph_build_status(&self, node: i32, start_ns: i64) {
        let elapsed_ms = (now_ns() - start_ns) as f64 / 1_000_000.0;
        self.info_stream.message(
            HNSW_COMPONENT,
            &format!("built {node} in {elapsed_ms:.2} ms"),
        );
    }

    /// Repairs the graph's connectivity, level by level.
    ///
    /// Equivalent to the private `HnswGraphBuilder.connectComponents()`.
    ///
    /// Lucene 10.5.0 has the call to it commented out of `finish()`, because
    /// connecting components can be exceptionally expensive
    /// (<https://github.com/apache/lucene/issues/14214>); the method is kept
    /// here for the same reason it is kept there — so that the repair can be
    /// re-enabled without rewriting it.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the graph or scoring raises.
    pub fn connect_components(&mut self) -> Result<()> {
        let start = Instant::now();
        for level in 0..self.hnsw.num_levels()? {
            if !self.connect_components_on_level(level)?
                && self.info_stream.is_enabled(HNSW_COMPONENT)
            {
                self.info_stream.message(
                    HNSW_COMPONENT,
                    &format!("connectComponents failed on level {level}"),
                );
            }
        }
        if self.info_stream.is_enabled(HNSW_COMPONENT) {
            self.info_stream.message(
                HNSW_COMPONENT,
                &format!("connectComponents {} ms", start.elapsed().as_millis()),
            );
        }
        Ok(())
    }

    /// Connects every component of one level to the largest one.
    ///
    /// Equivalent to the private `HnswGraphBuilder.connectComponents(int)`,
    /// which returns whether every component could be attached.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the graph or scoring raises.
    fn connect_components_on_level(&mut self, level: i32) -> Result<bool> {
        let mut not_fully_connected = FixedBitSet::new(self.hnsw.size().max(0) as usize);
        let mut max_conn = self.m;
        if level == 0 {
            max_conn *= 2;
        }
        let components = HnswUtil::components(
            &mut OnHeapGraphReader::new(&self.hnsw, &mut self.node_buffer),
            level,
            Some(&mut not_fully_connected),
            max_conn,
        )?;
        if self.info_stream.is_enabled(HNSW_COMPONENT) {
            self.info_stream.message(
                HNSW_COMPONENT,
                &format!("connect {} components on level={level}", components.len()),
            );
        }
        let mut result = true;
        if components.len() > 1 {
            // Connect the other components to the largest one.
            let c0 = components
                .iter()
                .max_by_key(|c| c.size())
                .copied()
                .expect("INVARIANT: components is not empty");
            if c0.start() == NO_MORE_DOCS {
                // The component is already fully connected: no room for new
                // connections.
                return Ok(false);
            }
            // Only one extra connection is attempted per component; more and
            // they may fill up while linking.
            let mut beam = GraphBuilderKnnCollector::new(2)?;
            for c in &components {
                if c.start() == c0.start() && c.size() == c0.size() {
                    continue;
                }
                if c.start() == NO_MORE_DOCS {
                    continue;
                }
                if self.info_stream.is_enabled(HNSW_COMPONENT) {
                    self.info_stream.message(
                        HNSW_COMPONENT,
                        &format!("connect component {c:?} to {c0:?}"),
                    );
                }

                beam.clear();
                let eps = [c0.start()];
                self.scorer.set_scoring_ordinal(c.start())?;
                // Find the node of the largest component closest to the
                // lowest-numbered node of this one that still has room for a
                // connection.
                self.graph_searcher.search_level(
                    &mut beam,
                    self.scorer.as_mut(),
                    level,
                    &eps,
                    &mut OnHeapGraphReader::new(&self.hnsw, &mut self.node_buffer),
                    Some(&FixedBits::new(
                        not_fully_connected.get_bits().to_vec(),
                        not_fully_connected.length(),
                    )),
                )?;
                let mut linked = false;
                while beam.size() > 0 {
                    let c0node = beam.pop_node();
                    if c0node == c.start() || !not_fully_connected.get(c0node as usize) {
                        continue;
                    }
                    let score = beam.minimum_score();
                    Self::link(
                        &self.hnsw,
                        level,
                        c0node,
                        c.start(),
                        score,
                        &mut not_fully_connected,
                    )?;
                    linked = true;
                    if self.info_stream.is_enabled(HNSW_COMPONENT) {
                        self.info_stream.message(
                            HNSW_COMPONENT,
                            &format!("connected ok {c0node} -> {}", c.start()),
                        );
                    }
                }
                if !linked {
                    if self.info_stream.is_enabled(HNSW_COMPONENT) {
                        self.info_stream
                            .message(HNSW_COMPONENT, "not connected; no free nodes found");
                    }
                    result = false;
                }
            }
        }
        Ok(result)
    }

    /// Links two nodes, always making the forward connection, and updates
    /// `not_fully_connected`.
    ///
    /// Equivalent to the private `HnswGraphBuilder.link(int, int, int, float,
    /// FixedBitSet)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever adding a neighbour raises.
    fn link(
        hnsw: &OnHeapHnswGraph,
        level: i32,
        n0: i32,
        n1: i32,
        score: f32,
        not_fully_connected: &mut FixedBitSet,
    ) -> Result<()> {
        {
            let mut nbr0 = hnsw.write_neighbors(level, n0)?;
            // One must be subtracted here because the nodes array is one larger
            // than the configured maximum number of neighbours (M or 2M). The
            // fullness check itself was done by searching for not-full nodes.
            let max_conn = nbr0.max_size() - 1;
            debug_assert!(not_fully_connected.get(n0 as usize));
            debug_assert!(nbr0.size() < max_conn, "node {n0} is full");
            nbr0.add_out_of_order(n1, score)?;
            if nbr0.size() == max_conn {
                not_fully_connected.clear(n0 as usize);
            }
        }
        // Java reads `maxConn` off `n0`'s array; both arrays live on one
        // level, so their capacities are equal.
        let mut nbr1 = hnsw.write_neighbors(level, n1)?;
        let max_conn = nbr1.max_size() - 1;
        if nbr1.size() < max_conn {
            nbr1.add_out_of_order(n0, score)?;
            if nbr1.size() == max_conn {
                not_fully_connected.clear(n1 as usize);
            }
        }
        Ok(())
    }
}

impl HnswBuilder for HnswGraphBuilder {
    fn build(&mut self, max_ord: i32) -> Result<&OnHeapHnswGraph> {
        self.build_skipping(max_ord, None)
    }

    fn add_graph_node(&mut self, node: i32) -> Result<()> {
        self.scorer.set_scoring_ordinal(node)?;
        self.add_graph_node_internal(node, None)
    }

    fn add_graph_node_with_eps(&mut self, node: i32, eps: &IntHashSet) -> Result<()> {
        self.scorer.set_scoring_ordinal(node)?;
        self.add_graph_node_internal(node, Some(eps))
    }

    fn set_info_stream(&mut self, info_stream: Arc<dyn InfoStream>) {
        HnswGraphBuilder::set_info_stream(self, info_stream);
    }

    fn get_graph(&self) -> &OnHeapHnswGraph {
        &self.hnsw
    }

    fn get_completed_graph(&mut self) -> Result<&OnHeapHnswGraph> {
        if !self.frozen {
            self.finish();
        }
        Ok(&self.hnsw)
    }

    fn into_completed_graph(self: Box<Self>) -> Result<OnHeapHnswGraph> {
        (*self).into_graph()
    }
}

impl HnswGraphBuilder {
    /// Adds every node in `[0, max_ord)` that `skip` does not hold, then
    /// freezes the builder and returns the graph.
    ///
    /// Equivalent to `HnswGraphBuilder.build(int)` (`HnswGraphBuilder.java:217-226`)
    /// as a subclass that overrides `addGraphNode(int)` runs it: `skip` is that
    /// override, as in [`add_vectors_skipping`](Self::add_vectors_skipping),
    /// and `None` is the base class's own `build`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] once the builder is frozen, and
    /// propagates whatever adding a node raises.
    pub(crate) fn build_skipping(
        &mut self,
        max_ord: i32,
        skip: Option<&FixedBitSet>,
    ) -> Result<&OnHeapHnswGraph> {
        if self.frozen {
            return Err(LuceneError::illegal_state(
                "This HnswGraphBuilder is frozen and cannot be updated".to_string(),
            ));
        }
        if self.info_stream.is_enabled(HNSW_COMPONENT) {
            self.info_stream.message(
                HNSW_COMPONENT,
                &format!("build graph from {max_ord} vectors"),
            );
        }
        self.add_vectors_skipping(0, max_ord, skip)?;
        HnswBuilder::get_completed_graph(self)
    }

    /// Adds every node in `[min_ord, max_ord)` to the graph.
    ///
    /// Equivalent to `HnswGraphBuilder.addVectors(int, int)`.
    pub fn add_vectors(&mut self, min_ord: i32, max_ord: i32) -> Result<()> {
        self.add_vectors_skipping(min_ord, max_ord, None)
    }

    /// Adds every node in `[min_ord, max_ord)` that `skip` does not already
    /// hold, reporting progress the whole time.
    ///
    /// This is still `HnswGraphBuilder.addVectors(int, int)`: Java's
    /// `HnswConcurrentMergeBuilder.ConcurrentMergeWorker` calls the *inherited*
    /// `addVectors` and overrides `addGraphNode(int)` to return early for a
    /// node the initializer graph already placed
    /// (`HnswConcurrentMergeBuilder.java:216-222`). Rust has no implementation
    /// inheritance, so the override becomes this parameter — which keeps the
    /// progress reporting and the elapsed-time message on the shared path,
    /// where Java's inheritance puts them, instead of duplicating the loop.
    pub fn add_vectors_skipping(
        &mut self,
        min_ord: i32,
        max_ord: i32,
        skip: Option<&FixedBitSet>,
    ) -> Result<()> {
        if self.frozen {
            return Err(LuceneError::illegal_state(
                "This HnswGraphBuilder is frozen and cannot be updated".to_string(),
            ));
        }
        let start_ns = now_ns();
        let progress_start_ns = if self.merge_start_time_ns != -1 {
            self.merge_start_time_ns
        } else {
            start_ns
        };
        for node in min_ord..max_ord {
            if let Some(initialized) = skip {
                if initialized.get(node as usize) {
                    continue;
                }
            }
            self.add_graph_node(node)?;
            if node % 10000 == 0 && self.info_stream.is_enabled(HNSW_COMPONENT) {
                self.print_graph_build_status(node, progress_start_ns);
            }
        }
        let chunked_elapsed_ns = now_ns() - start_ns;
        if let Some(cumulative_work_time_ns) = &self.cumulative_work_time_ns {
            cumulative_work_time_ns.fetch_add(chunked_elapsed_ns, Ordering::Relaxed);
        }
        let elapsed_ms = chunked_elapsed_ns as f64 / 1_000_000.0;
        if self.info_stream.is_enabled(HNSW_COMPONENT) {
            self.info_stream.message(
                HNSW_COMPONENT,
                &format!(
                    "addVectors [{min_ord} {max_ord}): {} vectors in {elapsed_ms:.2} ms",
                    max_ord - min_ord
                ),
            );
        }
        Ok(())
    }

    fn add_graph_node_internal(&mut self, node: i32, eps0: Option<&IntHashSet>) -> Result<()> {
        if self.frozen {
            return Err(LuceneError::illegal_state(
                "Graph builder is already frozen".to_string(),
            ));
        }
        let node_level = self.get_random_graph_level();
        // First add the node to all levels from top to bottom.
        for level in (0..=node_level).rev() {
            self.add_node_to_graph(level, node)?;
        }
        // If this is the first node, set it as the entry node and stop.
        if self.hnsw.try_set_new_entry_node(node, node_level) {
            return Ok(());
        }

        let mut lowest_unset_level = 0i32;
        loop {
            let cur_max_level = self.hnsw.num_levels()? - 1;
            let eps = vec![self.hnsw.entry_node()?];

            // Search upper levels with topk=1 to find the best entry point.
            let candidates = &mut self.entry_candidates;
            let mut eps_local = eps;
            for level in (node_level + 1..=cur_max_level).rev() {
                candidates.clear();
                self.graph_searcher.search_level(
                    candidates.as_collector(),
                    self.scorer.as_mut(),
                    level,
                    &eps_local,
                    &mut OnHeapGraphReader::new(&self.hnsw, &mut self.node_buffer),
                    None,
                )?;
                eps_local = vec![candidates.pop_node()];
            }

            // For levels <= nodeLevel, search with the beam and collect candidates.
            let scratch_levels = (node_level.min(cur_max_level) - lowest_unset_level + 1) as usize;
            let mut scratch_per_level: Vec<NeighborArray> = Vec::with_capacity(scratch_levels);
            let mut best_eps = eps_local;
            // Java keeps `candidates = beamCandidates` and only switches to
            // `beamCandidates0` on level 0 when explicit entry points were supplied.
            let mut use_beam0 = false;
            for i in (0..scratch_levels).rev() {
                let level = i as i32 + lowest_unset_level;
                if level == 0 {
                    if let Some(eps0) = eps0 {
                        if eps0.size() > 0 {
                            best_eps = eps0.to_array();
                            use_beam0 = true;
                        }
                    }
                }
                let candidates_ref = if use_beam0 {
                    &mut self.beam_candidates0
                } else {
                    &mut self.beam_candidates
                };
                candidates_ref.clear();
                self.graph_searcher.search_level(
                    candidates_ref.as_collector(),
                    self.scorer.as_mut(),
                    level,
                    &best_eps,
                    &mut OnHeapGraphReader::new(&self.hnsw, &mut self.node_buffer),
                    None,
                )?;
                best_eps = candidates_ref.pop_until_nearest_k_nodes();
                let mut scratch = NeighborArray::new(candidates_ref.k().max(self.m + 1), false);
                pop_to_scratch(candidates_ref, &mut scratch);
                scratch_per_level.push(scratch);
            }
            scratch_per_level.reverse();

            // Connect from bottom to top.
            for (i, scratch) in scratch_per_level.iter_mut().enumerate() {
                let level = i as i32 + lowest_unset_level;
                Self::add_diverse_neighbors_inner(
                    &self.hnsw,
                    self.scorer.as_mut(),
                    &mut self.bulk_score_nodes,
                    &mut self.bulk_scores,
                    self.m,
                    level,
                    node,
                    scratch,
                    false,
                )?;
            }
            lowest_unset_level += scratch_levels as i32;
            debug_assert_eq!(lowest_unset_level, node_level.min(cur_max_level) + 1);
            if lowest_unset_level == node_level + 1 {
                return Ok(());
            }
            debug_assert!(lowest_unset_level == cur_max_level + 1 && node_level > cur_max_level);
            if self
                .hnsw
                .try_promote_new_entry_node(node, node_level, cur_max_level)
            {
                return Ok(());
            }
            if self.hnsw.num_levels()? == cur_max_level + 1 {
                return Err(LuceneError::illegal_state(format!(
                    "We're not able to promote node {} at level {} as entry node",
                    node, node_level
                )));
            }
        }
    }

    /// Links `node` to the diverse subset of `candidates` on `level`, and links
    /// those neighbours back.
    ///
    /// Equivalent to `HnswGraphBuilder.addDiverseNeighbors`. `is_link_repair` marks
    /// the call as repairing the links of a node that already has neighbours, in
    /// which case the selected candidates are appended out of order and duplicates
    /// are filtered.
    pub(crate) fn add_diverse_neighbors(
        &mut self,
        level: i32,
        node: i32,
        candidates: &mut NeighborArray,
        is_link_repair: bool,
    ) -> Result<()> {
        Self::add_diverse_neighbors_inner(
            &self.hnsw,
            self.scorer.as_mut(),
            &mut self.bulk_score_nodes,
            &mut self.bulk_scores,
            self.m,
            level,
            node,
            candidates,
            is_link_repair,
        )
    }

    /// The body of `HnswGraphBuilder.addDiverseNeighbors`, over the fields it
    /// needs, so that callers can lend the scorer and the scratch buffers
    /// alongside a shared borrow of the graph.
    ///
    /// Java links each selected neighbour back under
    /// `hnswLock.write(level, nbr)` when the builder belongs to a concurrent
    /// merge, and without a lock otherwise; here the neighbour's own array
    /// lock is taken in both cases (see [`OnHeapHnswGraph`]).
    #[allow(clippy::too_many_arguments)]
    fn add_diverse_neighbors_inner(
        hnsw: &OnHeapHnswGraph,
        scorer: &mut dyn UpdateableRandomVectorScorer,
        bulk_score_nodes: &mut [i32; MAX_BULK_SCORE_NODES],
        bulk_scores: &mut [f32; MAX_BULK_SCORE_NODES],
        m: i32,
        level: i32,
        node: i32,
        candidates: &mut NeighborArray,
        is_link_repair: bool,
    ) -> Result<()> {
        // For each of the beamWidth nearest candidates (going from best to worst),
        // select it only if it is closer to the target than it is to any of the
        // already-selected neighbours.
        let max_conn_on_level = if level == 0 { m * 2 } else { m };
        let mask = Self::select_and_link_diverse(
            hnsw,
            scorer,
            bulk_score_nodes,
            bulk_scores,
            level,
            node,
            candidates,
            max_conn_on_level,
            is_link_repair,
        )?;

        // Link the selected nodes to the new node, and the new node to the selected
        // nodes (again applying the diversity heuristic). The candidates and the
        // mask are used rather than the node's own array, because once an incoming
        // link exists another worker may discover the node and modify that array.
        for (i, keep) in mask.iter().enumerate().take(candidates.size() as usize) {
            if !keep {
                continue;
            }
            let nbr = candidates.nodes()[i];
            let score = candidates.score(i as i32);
            let mut nbrs_of_nbr = hnsw.write_neighbors(level, nbr)?;
            Self::update_neighbor(&mut nbrs_of_nbr, scorer, node, score, nbr, is_link_repair)?;
        }
        Ok(())
    }

    /// Equivalent to `HnswGraphBuilder.updateNeighbor`.
    fn update_neighbor(
        nbrs_of_nbr: &mut NeighborArray,
        scorer: &mut dyn UpdateableRandomVectorScorer,
        node: i32,
        score: f32,
        nbr: i32,
        is_link_repair: bool,
    ) -> Result<()> {
        // Only check for duplicates during link repair, to avoid the performance
        // overhead during normal construction.
        if is_link_repair && nbrs_of_nbr.nodes().contains(&node) {
            return Ok(());
        }
        nbrs_of_nbr.add_and_ensure_diversity(node, score, nbr, scorer)
    }

    /// Selects the neighbours to link and returns a mask telling the caller
    /// which candidates were selected.
    ///
    /// Equivalent to the private `HnswGraphBuilder.selectAndLinkDiverse`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java fills the new node's own array
    /// without a lock, reasoning that no other worker can reach a node that
    /// has no incoming link yet. A concurrent unlocked write is still a data
    /// race in Rust whatever the reachability argument, so the array's write
    /// lock is held for the selection; it is uncontended for exactly the
    /// reason Java gives.
    #[allow(clippy::too_many_arguments)]
    fn select_and_link_diverse(
        hnsw: &OnHeapHnswGraph,
        scorer: &mut dyn UpdateableRandomVectorScorer,
        bulk_score_nodes: &mut [i32; MAX_BULK_SCORE_NODES],
        bulk_scores: &mut [f32; MAX_BULK_SCORE_NODES],
        level: i32,
        node: i32,
        candidates: &mut NeighborArray,
        max_conn_on_level: i32,
        is_link_repair: bool,
    ) -> Result<Vec<bool>> {
        let mut neighbors = hnsw.write_neighbors(level, node)?;
        let mut mask = vec![false; candidates.size() as usize];
        // Candidates are sorted ascending (worst to best), so iterate backward.
        for i in (0..candidates.size() as usize).rev() {
            if neighbors.size() >= max_conn_on_level {
                break;
            }
            let c_node = candidates.nodes()[i];
            if node == c_node {
                continue;
            }
            let c_score = candidates.score(i as i32);
            scorer.set_scoring_ordinal(c_node)?;
            if Self::diversity_check(scorer, bulk_score_nodes, bulk_scores, c_score, &neighbors)? {
                mask[i] = true;
                if is_link_repair {
                    neighbors.add_out_of_order(c_node, c_score)?;
                } else {
                    neighbors.add_in_order(c_node, c_score)?;
                }
            }
        }
        Ok(mask)
    }

    fn diversity_check(
        scorer: &mut dyn UpdateableRandomVectorScorer,
        bulk_score_nodes: &mut [i32; MAX_BULK_SCORE_NODES],
        bulk_scores: &mut [f32; MAX_BULK_SCORE_NODES],
        score: f32,
        neighbors: &NeighborArray,
    ) -> Result<bool> {
        let bulk_chunk = ((neighbors.size() + 1) / 2).min(MAX_BULK_SCORE_NODES as i32) as usize;
        let mut scored = 0usize;
        while scored < neighbors.size() as usize {
            let chunk_size = bulk_chunk.min(neighbors.size() as usize - scored);
            bulk_score_nodes[..chunk_size]
                .copy_from_slice(&neighbors.nodes()[scored..scored + chunk_size]);
            let max_score = scorer.bulk_score(bulk_score_nodes, bulk_scores, chunk_size as i32)?;
            if max_score >= score {
                return Ok(false);
            }
            scored += chunk_size;
        }
        Ok(true)
    }

    fn get_random_graph_level(&mut self) -> i32 {
        let mut rand_double;
        loop {
            rand_double = self.random.next_f64();
            if rand_double != 0.0 {
                break;
            }
        }
        ((-rand_double.ln()) * self.ml) as i32
    }
}

/// Drains `candidates` into `scratch`, worst score first.
///
/// Equivalent to `HnswGraphBuilder.popToScratch`.
pub(crate) fn pop_to_scratch(
    candidates: &mut GraphBuilderKnnCollector,
    scratch: &mut NeighborArray,
) {
    scratch.clear();
    let candidate_count = candidates.size();
    for _ in 0..candidate_count {
        let max_similarity = candidates.minimum_score();
        scratch
            .add_in_order(candidates.pop_node(), max_similarity)
            .unwrap();
    }
}

/// A restricted `KnnCollector` used during graph construction.
///
/// Equivalent to `HnswGraphBuilder.GraphBuilderKnnCollector`.
#[derive(Debug)]
pub struct GraphBuilderKnnCollector {
    queue: NeighborQueue,
    k: i32,
    visited_count: i64,
}

impl GraphBuilderKnnCollector {
    /// Creates a collector for `k` results.
    ///
    /// Equivalent to `GraphBuilderKnnCollector(int)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `k < 1`, as Java's
    /// `NeighborQueue` constructor throws `IllegalArgumentException`; see
    /// [`NeighborQueue::new`].
    pub fn new(k: i32) -> Result<Self> {
        Ok(Self {
            queue: NeighborQueue::new(k, false)?,
            k,
            visited_count: 0,
        })
    }

    /// Returns the configured `k`.
    pub fn k(&self) -> i32 {
        self.k
    }

    /// Returns the current number of collected nodes.
    pub fn size(&self) -> i32 {
        self.queue.size()
    }

    /// Removes and returns the best node.
    pub fn pop_node(&mut self) -> i32 {
        self.queue.pop()
    }

    /// Pops until only `k` nodes remain and returns them.
    pub fn pop_until_nearest_k_nodes(&mut self) -> Vec<i32> {
        while self.size() > self.k {
            self.queue.pop();
        }
        self.queue.nodes()
    }

    /// Returns the worst score currently in the queue.
    pub fn minimum_score(&self) -> f32 {
        self.queue.top_score()
    }

    /// Clears the collector.
    pub fn clear(&mut self) {
        self.queue.clear();
        self.visited_count = 0;
    }

    pub(crate) fn as_collector(&mut self) -> &mut dyn KnnCollector {
        self
    }
}

impl KnnCollector for GraphBuilderKnnCollector {
    fn early_terminated(&self) -> bool {
        false
    }

    fn inc_visited_count(&mut self, count: i32) {
        self.visited_count += count as i64;
    }

    fn visited_count(&self) -> i64 {
        self.visited_count
    }

    fn visit_limit(&self) -> i64 {
        i64::MAX
    }

    fn k(&self) -> i32 {
        self.k
    }

    fn collect(&mut self, doc_id: i32, similarity: f32) -> bool {
        self.queue.insert_with_overflow(doc_id, similarity)
    }

    fn min_competitive_similarity(&self) -> f32 {
        if self.queue.size() >= self.k {
            self.queue.top_score()
        } else {
            f32::NEG_INFINITY
        }
    }

    /// # Panics
    ///
    /// Always. Equivalent to `HnswGraphBuilder.GraphBuilderKnnCollector.topDocs()`,
    /// which throws `IllegalArgumentException`: this collector feeds graph
    /// construction and never produces search results.
    fn top_docs(&mut self) -> crate::search::knn::TopDocs {
        panic!("GraphBuilderKnnCollector does not produce TopDocs")
    }

    fn get_search_strategy(&self) -> Option<&crate::search::knn::KnnSearchStrategy> {
        None
    }
}

/// The `java.util.SplittableRandom` stream, which fixes every graph level.
///
/// Equivalent to `java.util.SplittableRandom`, seeded exactly as
/// `HnswGraphBuilder` seeds it (`HnswGraphBuilder.java:202-203`). This is not a
/// stylistic choice of RNG: `HnswGraphBuilder.getRandomGraphLevel` turns each
/// `nextDouble()` into the level a node is inserted at, so the draw sequence
/// decides how many levels the graph has and which nodes live on each. Any
/// other generator produces a different graph and therefore different `.vex`
/// and `.vem` bytes, and an index that is no longer byte-compatible with
/// Apache Lucene Core 10.5.0.
///
/// The algorithm was **measured** against a JDK 21 `SplittableRandom` rather
/// than transcribed: `HnswRandomFixture` in the Java harness prints the raw
/// bits of the first draws from seed 42, and
/// [`splittable_random_matches_the_jdk`] pins them here. The stream is
/// SplitMix64 — a Weyl sequence stepped by the golden-ratio gamma, passed
/// through the Murmur3 64-bit finalizer — and `nextDouble()` takes the top 53
/// bits of it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SplittableRandom {
    seed: u64,
}

impl SplittableRandom {
    /// `SplittableRandom.GOLDEN_GAMMA`, the increment of the Weyl sequence.
    const GOLDEN_GAMMA: u64 = 0x9e37_79b9_7f4a_7c15;

    pub(crate) fn new(seed: u64) -> Self {
        Self { seed }
    }

    /// `SplittableRandom.nextSeed()`: the seed advances *before* it is mixed,
    /// so the first value drawn from seed `s` mixes `s + GOLDEN_GAMMA`.
    fn next_seed(&mut self) -> u64 {
        self.seed = self.seed.wrapping_add(Self::GOLDEN_GAMMA);
        self.seed
    }

    /// The SplitMix64 mixing function, with the Murmur3 finalizer constants.
    fn mix64(mut z: u64) -> u64 {
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    pub(crate) fn next_f64(&mut self) -> f64 {
        let mixed = Self::mix64(self.next_seed());
        // `DOUBLE_UNIT` is `0x1.0p-53`; the multiplication is exact.
        (mixed >> 11) as f64 * f64::from_bits(0x3ca0_0000_0000_0000)
    }
}

#[cfg(test)]
mod tests {
    use super::SplittableRandom;

    /// The first draws of `java.util.SplittableRandom(42)`, as a JDK 21
    /// `SplittableRandom` produced them.
    ///
    /// Captured with the Java harness:
    /// `mvn exec:java -Dexec.mainClass=org.apache.lucene.rucene.codec.HnswRandomFixture
    /// -Dexec.args="42 12 16"`. These are the raw bits of each `nextDouble()`,
    /// not a decimal rendering, so the comparison cannot be blurred by
    /// formatting.
    ///
    /// This is a load-bearing constant of the HNSW format: each draw becomes
    /// one node's graph level, so a wrong stream writes a different `.vex` and
    /// a different `.vem` while every other byte of the segment stays right.
    #[test]
    fn splittable_random_matches_the_jdk() {
        const JDK_DRAWS: [u64; 12] = [
            0x3fe7_bae6_44c5_fd6d,
            0x3fc4_77f1_99d9_3378,
            0x3fd1_d499_d5c4_c3e6,
            0x3fd6_0738_7fc3_92b8,
            0x3fa3_78b0_b448_9040,
            0x3feb_c886_3f47_901b,
            0x3fcb_f4b3_8e22_9bb4,
            0x3fe9_9ec6_bdd3_d3c5,
            0x3fd5_c16e_1dc2_cf5e,
            0x3fe3_ca9a_e705_2fee,
            0x3fca_3a39_253b_ad8c,
            0x3fdf_8d22_8391_4594,
        ];
        let mut random = SplittableRandom::new(42);
        for (index, expected) in JDK_DRAWS.iter().enumerate() {
            let drawn = random.next_f64().to_bits();
            assert_eq!(
                drawn, *expected,
                "draw {index}: got {drawn:#x}, the JDK produced {expected:#x}"
            );
        }
    }

    /// The level each of those draws maps to, for the default `M` of 16.
    ///
    /// `HnswGraphBuilder.getRandomGraphLevel` computes
    /// `(int)(-log(randDouble) * ml)` with `ml = 1 / log(M)`. Pinning the levels
    /// as well as the draws catches a correct generator paired with a wrong
    /// `ml` or a wrong truncation, which the draws alone would not.
    #[test]
    fn graph_levels_match_the_jdk_for_the_default_m() {
        const JDK_LEVELS: [i32; 12] = [0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0];
        let ml = 1.0 / (16.0f64).ln();
        assert_eq!(
            ml.to_bits(),
            0x3fd7_1547_652b_82fe,
            "ml must be the double the JDK computed"
        );
        let mut random = SplittableRandom::new(42);
        for (index, expected) in JDK_LEVELS.iter().enumerate() {
            let mut rand_double;
            loop {
                rand_double = random.next_f64();
                if rand_double != 0.0 {
                    break;
                }
            }
            let level = ((-rand_double.ln()) * ml) as i32;
            assert_eq!(level, *expected, "level {index}");
        }
    }
}
