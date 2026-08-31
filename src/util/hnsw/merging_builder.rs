//! Port of `org.apache.lucene.util.hnsw.MergingHnswGraphBuilder`.

use crate::error::Result;
use crate::internal::hppc::IntHashSet;
use crate::search::NO_MORE_DOCS;
use crate::util::FixedBitSet;

use super::builder::{HnswBuilder, HnswGraphBuilder};
use super::initialized_builder::InitializedHnswGraphBuilder;
use super::on_heap::OnHeapHnswGraph;
use super::scorer::RandomVectorScorerSupplier;
use super::update_graphs_utils::UpdateGraphsUtils;
use super::HnswGraph;

/// A graph builder used during segment merging.
///
/// Equivalent to `org.apache.lucene.util.hnsw.MergingHnswGraphBuilder`.
///
/// This builder uses a smart algorithm to merge multiple graphs into a single graph,
/// based on the idea that if we know where we want to insert a node, we have a good
/// idea of where we want to insert its neighbours:
///
/// * take all graphs that have no deletions and sort them by size, descending;
/// * copy the largest graph to the new graph (`gL`);
/// * for each remaining small graph `gS`:
///   * find the nodes that best cover `gS` — the join set `j`. These nodes are
///     inserted into `gL` as usual, by searching `gL` for the best candidates to
///     connect them to;
///   * for each remaining node of `gS`, provide entry points formed by the union of
///     the node's neighbours in `gS` and those neighbours' neighbours in `gL`.
///
/// # Divergence from Lucene 10.5.0
///
/// Java derives this class from `HnswGraphBuilder`; this port holds one instead,
/// because Rust has no implementation inheritance.
pub struct MergingHnswGraphBuilder {
    inner: HnswGraphBuilder,
    graphs: Vec<Box<dyn HnswGraph>>,
    ord_maps: Vec<Vec<i32>>,
    initialized_nodes: Option<FixedBitSet>,
}

impl MergingHnswGraphBuilder {
    /// Creates a builder initialized with the first of `graphs` and set up to merge
    /// the rest into it.
    ///
    /// `ord_maps` holds one old-to-new ordinal map per graph, and
    /// `total_number_of_vectors` is the number of vectors the merged graph will hold.
    /// When `initialized_nodes` is `None`, every node is expected to be initialized
    /// by the merge itself.
    ///
    /// # Errors
    ///
    /// Returns an error if reading a graph or scoring fails.
    #[allow(clippy::too_many_arguments)]
    pub fn from_graphs(
        scorer_supplier: &dyn RandomVectorScorerSupplier,
        m: i32,
        beam_width: i32,
        seed: u64,
        mut graphs: Vec<Box<dyn HnswGraph>>,
        ord_maps: Vec<Vec<i32>>,
        total_number_of_vectors: i32,
        initialized_nodes: Option<FixedBitSet>,
    ) -> Result<Self> {
        let graph = InitializedHnswGraphBuilder::init_graph(
            graphs[0].as_mut(),
            &ord_maps[0],
            total_number_of_vectors,
            beam_width,
            scorer_supplier,
        )?;
        let inner =
            HnswGraphBuilder::with_graph(scorer_supplier.scorer()?, m, beam_width, seed, graph)?;
        Ok(Self {
            inner,
            graphs,
            ord_maps,
            initialized_nodes,
        })
    }

    /// Merges the smaller graph `index` into the current larger graph.
    fn update_graph(&mut self, index: usize) -> Result<()> {
        let size = self.graphs[index].size();
        let j = UpdateGraphsUtils::compute_join_set(self.graphs[index].as_mut())?;

        // For nodes in the join set, add them directly to the graph.
        let mut nodes = j.to_array();
        nodes.sort_unstable();
        for node in nodes {
            let mapped = self.ord_maps[index][node as usize];
            self.add_graph_node(mapped)?;
        }

        // For each node outside the join set, form the entry point set for the node
        // by joining the node's neighbours in gS with the node's neighbours'
        // neighbours in gL.
        for u in 0..size {
            if j.contains(u) {
                continue;
            }
            let mut eps = IntHashSet::new();
            self.graphs[index].seek(0, u)?;
            let mut neighbors: Vec<i32> = Vec::new();
            loop {
                let v = self.graphs[index].next_neighbor()?;
                if v == NO_MORE_DOCS {
                    break;
                }
                neighbors.push(v);
            }
            for v in neighbors {
                // If u's neighbour v is in the join set, or was already added to gL
                // (v < u), then add v's neighbours from gL to the candidate list.
                if v < u || j.contains(v) {
                    let newv = self.ord_maps[index][v as usize];
                    eps.add(newv);

                    self.inner.hnsw.seek(0, newv)?;
                    loop {
                        let friend_ord = self.inner.hnsw.next_neighbor()?;
                        if friend_ord == NO_MORE_DOCS {
                            break;
                        }
                        eps.add(friend_ord);
                    }
                }
            }
            let mapped = self.ord_maps[index][u as usize];
            self.add_graph_node_with_eps(mapped, &eps)?;
        }
        Ok(())
    }
}

impl HnswBuilder for MergingHnswGraphBuilder {
    fn build(&mut self, max_ord: i32) -> Result<&OnHeapHnswGraph> {
        if self.inner.frozen {
            return Err(crate::error::LuceneError::IllegalState(
                "This HnswGraphBuilder is frozen and cannot be updated".to_string(),
            ));
        }
        for i in 1..self.graphs.len() {
            self.update_graph(i)?;
        }

        if self.initialized_nodes.is_some() && max_ord > 0 {
            // Java walks the clear bits with `nextClearBit(from, maxOrd)`; iterating
            // the range and testing each bit visits exactly the same nodes.
            for node in 0..max_ord {
                let initialized = self
                    .initialized_nodes
                    .as_ref()
                    .expect("INVARIANT: checked with is_some above")
                    .get(node as usize);
                if !initialized {
                    self.add_graph_node(node)?;
                }
            }
        }

        self.get_completed_graph()
    }

    fn add_graph_node(&mut self, node: i32) -> Result<()> {
        self.inner.add_graph_node(node)
    }

    fn add_graph_node_with_eps(&mut self, node: i32, eps: &IntHashSet) -> Result<()> {
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
