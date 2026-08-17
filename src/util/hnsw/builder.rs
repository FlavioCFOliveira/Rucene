//! HNSW graph construction.
//!
//! Equivalent to `org.apache.lucene.util.hnsw.HnswGraphBuilder` and
//! `org.apache.lucene.util.hnsw.HnswBuilder`.

#![deny(unsafe_code)]

use std::f32;

use crate::error::{LuceneError, Result};
use crate::search::knn::KnnCollector;
use crate::util::hnsw::on_heap::OnHeapHnswGraph;
use crate::util::hnsw::scorer::{RandomVectorScorerSupplier, UpdateableRandomVectorScorer};
use crate::util::hnsw::searcher::HnswGraphSearcher;
use crate::util::hnsw::{HnswGraph, NeighborArray, NeighborQueue};

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

    /// Returns the partially built graph.
    fn get_graph(&self) -> &OnHeapHnswGraph;
}

/// Builds an in-memory HNSW graph.
///
/// Equivalent to `org.apache.lucene.util.hnsw.HnswGraphBuilder`.
pub struct HnswGraphBuilder {
    m: i32,
    ml: f64,
    bulk_score_nodes: [i32; MAX_BULK_SCORE_NODES],
    bulk_scores: [f32; MAX_BULK_SCORE_NODES],
    random: SimpleRng,
    scorer: Box<dyn UpdateableRandomVectorScorer>,
    graph_searcher: HnswGraphSearcher,
    entry_candidates: GraphBuilderKnnCollector,
    beam_candidates: GraphBuilderKnnCollector,
    beam_candidates0: GraphBuilderKnnCollector,
    hnsw: OnHeapHnswGraph,
    frozen: bool,
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
        let ml = if m == 1 { 1.0 } else { 1.0 / (m as f64).ln() };
        let graph_searcher = HnswGraphSearcher::new(
            NeighborQueue::new(beam_width, true),
            crate::util::FixedBitSet::new(hnsw.max_node_id().max(0) as usize + 1),
        );
        Ok(Self {
            m,
            ml,
            bulk_score_nodes: [0; MAX_BULK_SCORE_NODES],
            bulk_scores: [0.0; MAX_BULK_SCORE_NODES],
            random: SimpleRng::new(seed),
            scorer,
            graph_searcher,
            entry_candidates: GraphBuilderKnnCollector::new(1),
            beam_candidates: GraphBuilderKnnCollector::new(beam_width),
            beam_candidates0: GraphBuilderKnnCollector::new((beam_width / 2).min(m * 3)),
            hnsw,
            frozen: false,
        })
    }

    /// Consumes the builder and returns the constructed graph.
    pub fn into_graph(mut self) -> OnHeapHnswGraph {
        if !self.frozen {
            self.finish();
        }
        self.hnsw
    }

    fn finish(&mut self) {
        self.frozen = true;
    }
}

impl HnswBuilder for HnswGraphBuilder {
    fn build(&mut self, max_ord: i32) -> Result<&OnHeapHnswGraph> {
        if self.frozen {
            return Err(LuceneError::IllegalState(
                "This HnswGraphBuilder is frozen and cannot be updated".to_string(),
            ));
        }
        self.add_vectors(0, max_ord)?;
        if !self.frozen {
            self.finish();
        }
        Ok(&self.hnsw)
    }

    fn add_graph_node(&mut self, node: i32) -> Result<()> {
        self.scorer.set_scoring_ordinal(node)?;
        self.add_graph_node_internal(node)
    }

    fn get_graph(&self) -> &OnHeapHnswGraph {
        &self.hnsw
    }
}

impl HnswGraphBuilder {
    fn add_vectors(&mut self, min_ord: i32, max_ord: i32) -> Result<()> {
        if self.frozen {
            return Err(LuceneError::IllegalState(
                "This HnswGraphBuilder is frozen and cannot be updated".to_string(),
            ));
        }
        for node in min_ord..max_ord {
            self.add_graph_node(node)?;
        }
        Ok(())
    }

    fn add_graph_node_internal(&mut self, node: i32) -> Result<()> {
        if self.frozen {
            return Err(LuceneError::IllegalState(
                "Graph builder is already frozen".to_string(),
            ));
        }
        let node_level = self.get_random_graph_level();
        // First add the node to all levels from top to bottom.
        for level in (0..=node_level).rev() {
            self.hnsw.add_node(level, node);
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
                    &mut self.hnsw,
                    None,
                )?;
                eps_local = vec![candidates.pop_node()];
            }

            // For levels <= nodeLevel, search with the beam and collect candidates.
            let scratch_levels = (node_level.min(cur_max_level) - lowest_unset_level + 1) as usize;
            let mut scratch_per_level: Vec<NeighborArray> = Vec::with_capacity(scratch_levels);
            let mut best_eps = eps_local;
            for i in (0..scratch_levels).rev() {
                let level = i as i32 + lowest_unset_level;
                let candidates_ref = if level == 0 {
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
                    &mut self.hnsw,
                    None,
                )?;
                best_eps = candidates_ref.pop_until_nearest_k_nodes();
                let max_conn = if level == 0 { self.m * 2 } else { self.m };
                let mut scratch = NeighborArray::new(candidates_ref.k().max(max_conn + 1), false);
                pop_to_scratch(candidates_ref, &mut scratch);
                scratch_per_level.push(scratch);
            }
            scratch_per_level.reverse();

            // Connect from bottom to top.
            for (i, scratch) in scratch_per_level.iter_mut().enumerate() {
                self.add_diverse_neighbors(i as i32 + lowest_unset_level, node, scratch)?;
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
                return Err(LuceneError::IllegalState(format!(
                    "We're not able to promote node {} at level {} as entry node",
                    node, node_level
                )));
            }
        }
    }

    fn add_diverse_neighbors(
        &mut self,
        level: i32,
        node: i32,
        candidates: &mut NeighborArray,
    ) -> Result<()> {
        let max_conn_on_level = if level == 0 { self.m * 2 } else { self.m };
        let mask = self.select_and_link_diverse(level, node, candidates, max_conn_on_level)?;

        for (i, keep) in mask.iter().enumerate().take(candidates.size() as usize) {
            if !keep {
                continue;
            }
            let nbr = candidates.nodes()[i];
            let score = candidates.score(i as i32);
            self.scorer.set_scoring_ordinal(nbr)?;
            let nbrs_of_nbr = self.hnsw.get_neighbors(level, nbr)?;
            // We cannot mutate through the immutable reference returned by
            // get_neighbors, so clone it, update, and write back.
            let mut nbrs_of_nbr_clone = nbrs_of_nbr.clone();
            nbrs_of_nbr_clone.add_and_ensure_diversity(node, score, nbr, self.scorer.as_mut())?;
            let _ = self.hnsw.set_neighbors(level, nbr, nbrs_of_nbr_clone);
        }
        Ok(())
    }

    fn select_and_link_diverse(
        &mut self,
        level: i32,
        node: i32,
        candidates: &mut NeighborArray,
        max_conn_on_level: i32,
    ) -> Result<Vec<bool>> {
        let mut mask = vec![false; candidates.size() as usize];
        // candidates are sorted ascending (worst to best), so iterate backward.
        for i in (0..candidates.size() as usize).rev() {
            if self.hnsw.get_neighbors(level, node)?.size() >= max_conn_on_level {
                break;
            }
            let c_node = candidates.nodes()[i];
            if node == c_node {
                continue;
            }
            let c_score = candidates.score(i as i32);
            self.scorer.set_scoring_ordinal(c_node)?;
            let neighbors = self.hnsw.get_neighbors(level, node)?.clone();
            if self.diversity_check(c_score, &neighbors)? {
                mask[i] = true;
                let mut node_neighbors = self.hnsw.get_neighbors(level, node)?.clone();
                node_neighbors.add_in_order(c_node, c_score)?;
                let _ = self.hnsw.set_neighbors(level, node, node_neighbors);
            }
        }
        Ok(mask)
    }

    fn diversity_check(&mut self, score: f32, neighbors: &NeighborArray) -> Result<bool> {
        let bulk_chunk = ((neighbors.size() + 1) / 2).min(MAX_BULK_SCORE_NODES as i32) as usize;
        let mut scored = 0usize;
        while scored < neighbors.size() as usize {
            let chunk_size = bulk_chunk.min(neighbors.size() as usize - scored);
            self.bulk_score_nodes[..chunk_size]
                .copy_from_slice(&neighbors.nodes()[scored..scored + chunk_size]);
            let max_score = self.scorer.bulk_score(
                &self.bulk_score_nodes,
                &mut self.bulk_scores,
                chunk_size as i32,
            )?;
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

fn pop_to_scratch(candidates: &mut GraphBuilderKnnCollector, scratch: &mut NeighborArray) {
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
    pub fn new(k: i32) -> Self {
        Self {
            queue: NeighborQueue::new(k, false),
            k,
            visited_count: 0,
        }
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

    fn as_collector(&mut self) -> &mut dyn KnnCollector {
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
}

/// A tiny deterministic RNG used for graph level assignment.
#[derive(Debug, Clone, Copy)]
struct SimpleRng {
    state: u64,
}

impl SimpleRng {
    fn new(seed: u64) -> Self {
        let state = if seed == 0 { 0x9E3779B97F4A7C15 } else { seed };
        Self { state }
    }

    fn next_u64(&mut self) -> u64 {
        // xorshift64*
        self.state ^= self.state >> 12;
        self.state ^= self.state << 25;
        self.state ^= self.state >> 27;
        self.state.wrapping_mul(0x2545F4914F6CDD1D)
    }

    fn next_f64(&mut self) -> f64 {
        // Return a value in (0, 1).
        let u = self.next_u64();
        (u >> 11) as f64 / (1u64 << 53) as f64
    }
}
