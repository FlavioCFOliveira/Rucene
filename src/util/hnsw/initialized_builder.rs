//! Port of `org.apache.lucene.util.hnsw.InitializedHnswGraphBuilder`.

use std::collections::HashMap;

use crate::error::Result;
use crate::internal::hppc::IntHashSet;
use crate::search::NO_MORE_DOCS;
use crate::util::FixedBitSet;

use super::builder::{
    pop_to_scratch, GraphBuilderKnnCollector, HnswBuilder, HnswGraphBuilder, SplittableRandom,
    DEFAULT_RAND_SEED,
};
use super::on_heap::OnHeapHnswGraph;
use super::scorer::RandomVectorScorerSupplier;
use super::{HnswGraph, NeighborArray};

/// The threshold factor for deciding whether a node is disconnected.
///
/// Equivalent to `InitializedHnswGraphBuilder.DISCONNECTED_NODE_FACTOR`. A node is
/// considered disconnected if its new neighbour count is less than
/// `old neighbour count * DISCONNECTED_NODE_FACTOR`.
const DISCONNECTED_NODE_FACTOR: f64 = 0.85;

/// A graph builder initialized with the structure of an existing [`HnswGraph`].
///
/// Equivalent to `org.apache.lucene.util.hnsw.InitializedHnswGraphBuilder`. This is
/// useful for merging HNSW graphs from multiple segments. The builder copies the
/// graph structure with ordinal remapping, repairs nodes that lost a portion of
/// their neighbours to deletions, rebalances the level hierarchy, and then allows
/// incremental addition of new nodes while preserving the initialized ones.
///
/// # Divergences from Lucene 10.5.0
///
/// * Java derives this class from `HnswGraphBuilder`; this port holds one instead,
///   because Rust has no implementation inheritance.
/// * `rebalanceGraph` seeds a fresh `SplittableRandom()` from the JVM's entropy, so
///   Lucene's rebalancing is not reproducible. This port derives the generator from
///   the builder's own seed, which keeps the merged graph deterministic; the
///   distribution promoted nodes are drawn from is unchanged.
pub struct InitializedHnswGraphBuilder {
    inner: HnswGraphBuilder,
    /// Tracks which nodes have already been initialized from the source graph.
    initialized_nodes: Option<FixedBitSet>,
    /// Maps each level to the node ordinals present at that level.
    level_to_nodes: Vec<Vec<i32>>,
    /// Tracks whether the source graph had deletions.
    has_deletes: bool,
    seed: u64,
}

impl InitializedHnswGraphBuilder {
    /// Creates a builder initialized with the structure of `initializer_graph`.
    ///
    /// `new_ord_map` maps old ordinals in the initializer graph to new ordinals in
    /// the merged graph; `-1` marks a deleted document that should be skipped.
    ///
    /// # Errors
    ///
    /// Returns an error if reading the initializer graph or scoring fails.
    pub fn from_graph(
        scorer_supplier: &dyn RandomVectorScorerSupplier,
        beam_width: i32,
        seed: u64,
        initializer_graph: &mut dyn HnswGraph,
        new_ord_map: &[i32],
        initialized_nodes: Option<FixedBitSet>,
        total_number_of_vectors: i32,
    ) -> Result<Self> {
        let graph = OnHeapHnswGraph::new(initializer_graph.max_conn(), total_number_of_vectors);
        let m = HnswGraph::max_conn(&graph);
        let inner =
            HnswGraphBuilder::with_graph(scorer_supplier.scorer()?, m, beam_width, seed, graph)?;
        let mut builder = Self {
            inner,
            initialized_nodes,
            level_to_nodes: Vec::new(),
            has_deletes: false,
            seed,
        };
        builder.initialize_from_graph(initializer_graph, new_ord_map)?;
        Ok(builder)
    }

    /// Builds a fully initialized on-heap graph without tracking initialized nodes.
    ///
    /// Equivalent to `InitializedHnswGraphBuilder.initGraph`.
    ///
    /// # Errors
    ///
    /// Returns an error if reading the initializer graph or scoring fails.
    pub fn init_graph(
        initializer_graph: &mut dyn HnswGraph,
        new_ord_map: &[i32],
        total_number_of_vectors: i32,
        beam_width: i32,
        scorer_supplier: &dyn RandomVectorScorerSupplier,
    ) -> Result<OnHeapHnswGraph> {
        let builder = Self::from_graph(
            scorer_supplier,
            beam_width,
            DEFAULT_RAND_SEED,
            initializer_graph,
            new_ord_map,
            None,
            total_number_of_vectors,
        )?;
        Ok(builder.inner.into_graph())
    }

    /// Consumes the builder and returns the graph it holds.
    pub fn into_graph(self) -> OnHeapHnswGraph {
        self.inner.into_graph()
    }

    fn initialize_from_graph(
        &mut self,
        initializer_graph: &mut dyn HnswGraph,
        new_ord_map: &[i32],
    ) -> Result<()> {
        self.has_deletes = false;
        // Phase 1: copy the structure and identify nodes that lost too many
        // neighbours.
        let disconnected_nodes_by_level =
            self.copy_graph_structure(initializer_graph, new_ord_map)?;

        // Repair the graph if it has deletes.
        if self.has_deletes {
            // Phase 2: repair nodes with insufficient connections.
            let num_levels = initializer_graph.num_levels()?;
            self.repair_disconnected_nodes(&disconnected_nodes_by_level, num_levels)?;

            // Phase 3: rebalance the graph to maintain a proper level distribution.
            self.rebalance_graph()?;
        }
        Ok(())
    }

    fn copy_graph_structure(
        &mut self,
        initializer_graph: &mut dyn HnswGraph,
        new_ord_map: &[i32],
    ) -> Result<HashMap<i32, Vec<i32>>> {
        let num_levels = initializer_graph.num_levels()?;
        self.level_to_nodes = vec![Vec::new(); num_levels.max(0) as usize];
        let mut disconnected_nodes_by_level: HashMap<i32, Vec<i32>> = HashMap::new();

        for level in (0..num_levels).rev() {
            let mut disconnected_nodes: Vec<i32> = Vec::new();
            let mut it = initializer_graph.get_nodes_on_level(level)?;

            while it.has_next() {
                let old_ord = it.next_int();
                let new_ord = new_ord_map[old_ord as usize];

                // Skip deleted documents (mapped to -1).
                if new_ord == -1 {
                    self.has_deletes = true;
                    continue;
                }

                self.inner.hnsw.add_node(level, new_ord);
                self.level_to_nodes[level as usize].push(new_ord);
                self.inner.hnsw.try_set_new_entry_node(new_ord, level);
                self.inner.scorer.set_scoring_ordinal(new_ord)?;

                // Copy neighbours.
                let mut new_neighbors = self.inner.hnsw.get_neighbors(level, new_ord)?.clone();
                initializer_graph.seek(level, old_ord)?;
                let mut old_neighbour_count = 0i32;
                loop {
                    let old_neighbor = initializer_graph.next_neighbor()?;
                    if old_neighbor == NO_MORE_DOCS {
                        break;
                    }
                    old_neighbour_count += 1;
                    let new_neighbor = new_ord_map[old_neighbor as usize];

                    // Only add neighbours that weren't deleted.
                    if new_neighbor != -1 {
                        new_neighbors.add_out_of_order(new_neighbor, f32::NAN)?;
                    }
                }

                // Mark as disconnected if the node lost more than the acceptable
                // threshold of neighbours.
                let kept = new_neighbors.size();
                let _ = self.inner.hnsw.set_neighbors(level, new_ord, new_neighbors);
                if f64::from(kept) < f64::from(old_neighbour_count) * DISCONNECTED_NODE_FACTOR {
                    disconnected_nodes.push(new_ord);
                }
            }
            disconnected_nodes_by_level.insert(level, disconnected_nodes);
        }
        Ok(disconnected_nodes_by_level)
    }

    fn repair_disconnected_nodes(
        &mut self,
        disconnected_nodes_by_level: &HashMap<i32, Vec<i32>>,
        num_levels: i32,
    ) -> Result<()> {
        for level in (0..num_levels).rev() {
            if let Some(nodes) = disconnected_nodes_by_level.get(&level) {
                self.fix_disconnected_nodes(nodes, level)?;
            }
        }
        Ok(())
    }

    fn fix_disconnected_nodes(&mut self, disconnected_nodes: &[i32], level: i32) -> Result<()> {
        if disconnected_nodes.is_empty() {
            return Ok(());
        }

        let beam_width = self.inner.beam_candidates.k();
        let mut candidates = GraphBuilderKnnCollector::new(beam_width);
        let mut scratch_array = NeighborArray::new(beam_width, false);

        for &node in disconnected_nodes {
            self.inner.scorer.set_scoring_ordinal(node)?;
            let existing_neighbors = self.inner.hnsw.get_neighbors(level, node)?.clone();

            // Only repair if the node has at least one neighbour to enter from.
            if existing_neighbors.size() > 0 {
                let entry_points: Vec<i32> =
                    existing_neighbors.nodes()[..existing_neighbors.size() as usize].to_vec();

                // Search from the entry points to find candidate neighbours.
                self.inner.graph_searcher.search_level(
                    candidates.as_collector(),
                    self.inner.scorer.as_mut(),
                    level,
                    &entry_points,
                    &mut self.inner.hnsw,
                    None,
                )?;
                pop_to_scratch(&mut candidates, &mut scratch_array);

                // Add diverse neighbours using the HNSW heuristic.
                self.inner
                    .add_diverse_neighbors(level, node, &mut scratch_array, true)?;
            } else {
                // The node has no neighbours; add connections from scratch.
                self.add_connections(node, level)?;
            }

            scratch_array.clear();
            candidates.clear();
        }
        Ok(())
    }

    fn rebalance_graph(&mut self) -> Result<()> {
        let mut random = SplittableRandom::new(self.seed);
        let size = self.inner.hnsw.size();
        let inv_max_conn = 1.0 / f64::from(self.inner.m);

        // Process each level starting from level 1 (level 0 always holds all nodes).
        let mut level = 1i32;
        loop {
            // Expected number of nodes at this level.
            let max_nodes_at_level = (f64::from(size) * inv_max_conn.powi(level)) as i32;
            if max_nodes_at_level <= 0 {
                break;
            }

            let mut current_nodes_at_level = 0i32;

            if level as usize >= self.level_to_nodes.len() {
                self.level_to_nodes.resize(level as usize + 1, Vec::new());
            } else {
                current_nodes_at_level = self.level_to_nodes[level as usize].len() as i32;
            }

            if current_nodes_at_level >= max_nodes_at_level {
                level += 1;
                continue;
            }

            // Randomly promote nodes from the level below.
            let below: Vec<i32> = self.level_to_nodes[level as usize - 1].clone();
            for node in below {
                if current_nodes_at_level >= max_nodes_at_level {
                    break;
                }
                // Promote with probability 1/M, matching HNSW's level distribution.
                if random.next_f64() < inv_max_conn
                    && !self.inner.hnsw.node_exists_at_level(level, node)
                {
                    self.inner.scorer.set_scoring_ordinal(node)?;
                    self.inner.hnsw.add_node(level, node);

                    if current_nodes_at_level == 0 {
                        let num_levels = self.inner.hnsw.num_levels()?;
                        self.inner
                            .hnsw
                            .try_promote_new_entry_node(node, level, num_levels - 1);
                    } else {
                        self.add_connections(node, level)?;
                    }

                    self.level_to_nodes[level as usize].push(node);
                    current_nodes_at_level += 1;
                }
            }
            level += 1;
        }
        Ok(())
    }

    fn add_connections(&mut self, node: i32, target_level: i32) -> Result<()> {
        let beam_width = self.inner.beam_candidates.k();
        let mut candidates = GraphBuilderKnnCollector::new(beam_width);
        let mut eps = vec![self.inner.hnsw.entry_node()?];

        // Navigate down from the top to the target level, greedily moving toward the
        // new node.
        let mut level = self.inner.hnsw.num_levels()? - 1;
        while level > target_level {
            self.inner.graph_searcher.search_level(
                candidates.as_collector(),
                self.inner.scorer.as_mut(),
                level,
                &eps,
                &mut self.inner.hnsw,
                None,
            )?;
            eps[0] = candidates.pop_node();
            candidates.clear();
            level -= 1;
        }

        // Perform a full search at the target level to find neighbours.
        self.inner.graph_searcher.search_level(
            candidates.as_collector(),
            self.inner.scorer.as_mut(),
            target_level,
            &eps,
            &mut self.inner.hnsw,
            None,
        )?;

        let mut scratch_array = NeighborArray::new(beam_width, false);
        pop_to_scratch(&mut candidates, &mut scratch_array);

        // Add diverse neighbours and establish bidirectional connections.
        self.inner
            .add_diverse_neighbors(target_level, node, &mut scratch_array, true)
    }
}

impl HnswBuilder for InitializedHnswGraphBuilder {
    fn build(&mut self, max_ord: i32) -> Result<&OnHeapHnswGraph> {
        self.inner.build(max_ord)
    }

    fn add_graph_node(&mut self, node: i32) -> Result<()> {
        if let Some(initialized) = &self.initialized_nodes {
            if initialized.get(node as usize) {
                return Ok(());
            }
        }
        self.inner.add_graph_node(node)
    }

    fn add_graph_node_with_eps(&mut self, node: i32, eps: &IntHashSet) -> Result<()> {
        if let Some(initialized) = &self.initialized_nodes {
            if initialized.get(node as usize) {
                return Ok(());
            }
        }
        self.inner.add_graph_node_with_eps(node, eps)
    }

    fn get_graph(&self) -> &OnHeapHnswGraph {
        self.inner.get_graph()
    }

    fn get_completed_graph(&mut self) -> Result<&OnHeapHnswGraph> {
        self.inner.get_completed_graph()
    }

    fn into_completed_graph(mut self: Box<Self>) -> Result<OnHeapHnswGraph> {
        self.inner.get_completed_graph()?;
        Ok(self.inner.into_graph())
    }
}
