//! HNSW graph utilities for approximate nearest-neighbor search.
//!
//! Equivalent to `org.apache.lucene.util.hnsw`.
//!
//! These types provide the in-memory graph representation and construction
//! primitives used by the KNN-vectors codecs (`Lucene99HnswVectorsFormat`).
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`AbstractHnswGraphSearcher`] | `AbstractHnswGraphSearcher` |
//! | [`BlockingFloatHeap`] | `BlockingFloatHeap` |
//! | [`ConcurrentHnswMerger`] | `ConcurrentHnswMerger` |
//! | [`FilteredHnswGraphSearcher`] | `FilteredHnswGraphSearcher` |
//! | [`FloatHeap`] | `FloatHeap` |
//! | [`HasKnnVectorValues`] | `HasKnnVectorValues` |
//! | [`HnswBuilder`] | `HnswBuilder` |
//! | [`HnswConcurrentMergeBuilder`] | `HnswConcurrentMergeBuilder` |
//! | [`HnswGraph`] | `HnswGraph` |
//! | [`HnswGraphBuilder`] | `HnswGraphBuilder` |
//! | [`HnswGraphMerger`] | `HnswGraphMerger` |
//! | [`HnswGraphSearcher`] | `HnswGraphSearcher` |
//! | [`HnswLock`] | `HnswLock` |
//! | [`HnswUtil`] | `HnswUtil` |
//! | [`IncrementalHnswGraphMerger`] | `IncrementalHnswGraphMerger` |
//! | [`GraphReader`] | `IncrementalHnswGraphMerger.GraphReader` |
//! | [`InitializedHnswGraphBuilder`] | `InitializedHnswGraphBuilder` |
//! | [`IntToIntFunction`] | `IntToIntFunction` |
//! | [`MergingHnswGraphBuilder`] | `MergingHnswGraphBuilder` |
//! | [`NeighborArray`] | `NeighborArray` |
//! | [`NeighborQueue`] | `NeighborQueue` |
//! | [`OnHeapHnswGraph`] | `OnHeapHnswGraph` |
//! | [`OrdinalTranslatedKnnCollector`] | `OrdinalTranslatedKnnCollector` |
//! | [`RandomVectorScorer`] | `RandomVectorScorer` |
//! | [`RandomVectorScorerSupplier`] | `RandomVectorScorerSupplier` |
//! | [`SeededHnswGraphSearcher`] | `SeededHnswGraphSearcher` |
//! | [`UpdateGraphsUtils`] | `UpdateGraphsUtils` |
//! | [`UpdateableRandomVectorScorer`] | `UpdateableRandomVectorScorer` |

#![deny(unsafe_code)]

use crate::error::Result;

pub mod abstract_searcher;
pub mod blocking_float_heap;
pub mod builder;
pub mod concurrent_merge_builder;
pub mod filtered_searcher;
pub mod float_heap;
pub mod has_knn_vector_values;
pub mod hnsw_lock;
pub mod incremental_merger;
pub mod initialized_builder;
pub mod int_to_int_function;
pub mod merger;
pub mod merging_builder;
pub mod neighbor;
pub mod on_heap;
pub mod ordinal_translated_knn_collector;
pub mod scorer;
pub mod searcher;
pub mod seeded_searcher;
pub mod update_graphs_utils;
pub mod util;

pub use abstract_searcher::{AbstractHnswGraphSearcher, UNK_EP};
pub use blocking_float_heap::BlockingFloatHeap;
pub use builder::{HnswBuilder, HnswGraphBuilder};
pub use concurrent_merge_builder::HnswConcurrentMergeBuilder;
pub use filtered_searcher::FilteredHnswGraphSearcher;
pub use float_heap::FloatHeap;
pub use has_knn_vector_values::HasKnnVectorValues;
pub use hnsw_lock::HnswLock;
pub use incremental_merger::{GraphReader, IncrementalHnswGraphMerger};
pub use initialized_builder::InitializedHnswGraphBuilder;
pub use int_to_int_function::IntToIntFunction;
pub use merger::{ConcurrentHnswMerger, HnswGraphMerger};
pub use merging_builder::MergingHnswGraphBuilder;
pub use neighbor::{NeighborArray, NeighborQueue};
pub use on_heap::{EmptyHnswGraph, OnHeapHnswGraph};
pub use ordinal_translated_knn_collector::OrdinalTranslatedKnnCollector;
pub use scorer::{
    CloseableRandomVectorScorerSupplier, RandomVectorScorer, RandomVectorScorerSupplier,
    UpdateableRandomVectorScorer,
};
pub use searcher::HnswGraphSearcher;
pub use seeded_searcher::SeededHnswGraphSearcher;
pub use update_graphs_utils::UpdateGraphsUtils;
pub use util::{Component, HnswUtil};

/// Iterator over the node ordinals present on a given graph level.
///
/// Equivalent to `org.apache.lucene.util.hnsw.HnswGraph.NodesIterator`.
pub trait NodesIterator: Send + Sync {
    /// Returns the number of nodes the iterator will yield.
    fn size(&self) -> i32;

    /// Returns the next node ordinal, or `NO_MORE_DOCS` when exhausted.
    fn next_int(&mut self) -> i32;

    /// Returns `true` while there are still nodes to consume.
    fn has_next(&self) -> bool;

    /// Copies as many remaining node ordinals as fit into `dest`.
    ///
    /// Returns the number of values written.
    fn consume(&mut self, dest: &mut [i32]) -> usize;
}

/// Node iterator that enumerates the contiguous range `[0, size)`.
///
/// Equivalent to `org.apache.lucene.util.hnsw.HnswGraph.DenseNodesIterator`.
#[derive(Clone, Debug)]
pub struct DenseNodesIterator {
    size: i32,
    cur: i32,
}

impl DenseNodesIterator {
    /// Creates an iterator over `[0, size)`.
    pub fn new(size: i32) -> Self {
        Self { size, cur: 0 }
    }
}

impl NodesIterator for DenseNodesIterator {
    fn size(&self) -> i32 {
        self.size
    }

    fn next_int(&mut self) -> i32 {
        if self.cur < self.size {
            let v = self.cur;
            self.cur += 1;
            v
        } else {
            crate::search::NO_MORE_DOCS
        }
    }

    fn has_next(&self) -> bool {
        self.cur < self.size
    }

    fn consume(&mut self, dest: &mut [i32]) -> usize {
        let remaining = (self.size - self.cur) as usize;
        let n = remaining.min(dest.len());
        for (i, slot) in dest.iter_mut().enumerate().take(n) {
            *slot = self.cur + i as i32;
        }
        self.cur += n as i32;
        n
    }
}

/// Node iterator backed by an arbitrary array of ordinals.
///
/// Equivalent to `org.apache.lucene.util.hnsw.HnswGraph.ArrayNodesIterator`.
#[derive(Clone, Debug)]
pub struct ArrayNodesIterator {
    nodes: Vec<i32>,
    size: i32,
    cur: i32,
}

impl ArrayNodesIterator {
    /// Creates an iterator over `nodes`.
    pub fn new(nodes: Vec<i32>) -> Self {
        let size = nodes.len() as i32;
        Self {
            nodes,
            size,
            cur: 0,
        }
    }
}

impl NodesIterator for ArrayNodesIterator {
    fn size(&self) -> i32 {
        self.size
    }

    fn next_int(&mut self) -> i32 {
        if self.cur < self.size {
            let v = self.nodes[self.cur as usize];
            self.cur += 1;
            v
        } else {
            crate::search::NO_MORE_DOCS
        }
    }

    fn has_next(&self) -> bool {
        self.cur < self.size
    }

    fn consume(&mut self, dest: &mut [i32]) -> usize {
        let remaining = (self.size - self.cur) as usize;
        let n = remaining.min(dest.len());
        let start = self.cur as usize;
        dest[..n].copy_from_slice(&self.nodes[start..start + n]);
        self.cur += n as i32;
        n
    }
}

/// Marker returned by `HnswGraph::next_neighbor` when a neighbor list is
/// exhausted.
pub const NO_MORE_DOCS: i32 = crate::search::NO_MORE_DOCS;

/// Sentinel value used when the maximum number of connections is unknown.
pub const UNKNOWN_MAX_CONN: i32 = -1;

/// Default `M` (maximum connections on upper levels) used by Lucene.
pub const DEFAULT_M: i32 = 16;

/// Default beam width (`efConstruction`) used by Lucene.
pub const DEFAULT_BEAM_WIDTH: i32 = 100;

/// Hierarchical Navigable Small World graph.
///
/// Equivalent to `org.apache.lucene.util.hnsw.HnswGraph`.
///
/// # Not `Send + Sync`, as in Lucene
///
/// Every method that moves the cursor takes `&mut self`, so an instance is
/// inherently single-threaded: Java's `HnswGraph` carries the same cursor state
/// (`arcCount`, `arcUpTo`, `arc`) and the same rule — one instance per search,
/// never shared. This port used to require `Send + Sync` here, which Lucene
/// does not, and that bound had a cost: `Lucene99HnswVectorsReader`'s off-heap
/// graph could not hold the `DirectMonotonicReader` Java holds for its node
/// offsets, and materialised every offset into a `Vec` sized by a count read
/// out of the `.vem` instead. Dropping the bound removes the divergence and the
/// unbounded allocation with it.
pub trait HnswGraph {
    /// Positions the neighbor iterator on `target` at `level`.
    fn seek(&mut self, level: i32, target: i32) -> Result<()>;

    /// Returns the number of nodes in the graph.
    fn size(&self) -> i32;

    /// Returns the maximum node id, inclusive.
    fn max_node_id(&self) -> i32 {
        self.size() - 1
    }

    /// Returns the next neighbor ordinal, or `NO_MORE_DOCS`.
    fn next_neighbor(&mut self) -> Result<i32>;

    /// Returns the current number of levels.
    fn num_levels(&self) -> Result<i32>;

    /// Returns `M`, the maximum number of connections per node on upper levels.
    fn max_conn(&self) -> i32;

    /// Returns the entry node on the top level.
    fn entry_node(&self) -> Result<i32>;

    /// Returns an iterator over the nodes on `level`.
    fn get_nodes_on_level(&self, level: i32) -> Result<Box<dyn NodesIterator>>;

    /// Returns the number of neighbors of the current target node.
    fn neighbor_count(&self) -> i32;

    /// Returns the nodes on `level` in sorted order.
    ///
    /// For level 0 this is a dense iterator; otherwise it materializes and
    /// sorts the node list.
    fn get_sorted_nodes_on_level(&self, level: i32) -> Result<Box<dyn NodesIterator>> {
        if level == 0 {
            return Ok(Box::new(DenseNodesIterator::new(self.size())));
        }
        let mut nodes_on_level = self.get_nodes_on_level(level)?;
        let mut sorted_nodes = vec![0i32; nodes_on_level.size() as usize];
        for n in sorted_nodes.iter_mut() {
            *n = nodes_on_level.next_int();
        }
        sorted_nodes.sort_unstable();
        Ok(Box::new(ArrayNodesIterator::new(sorted_nodes)))
    }
}

#[cfg(test)]
mod tests {
    use super::builder::{HnswBuilder, HnswGraphBuilder};
    use super::merger::{ConcurrentHnswMerger, HnswGraphMerger};
    use super::on_heap::OnHeapHnswGraph;
    use super::scorer::{
        RandomVectorScorer, RandomVectorScorerSupplier, UpdateableRandomVectorScorer,
    };
    use super::searcher::HnswGraphSearcher;
    use super::util::HnswUtil;
    use super::{HnswGraph, Result};
    use crate::util::hnsw::concurrent_merge_builder::HnswConcurrentMergeBuilder;

    fn dot(a: &[Vec<f32>], i: usize, j: usize) -> f32 {
        a[i].iter().zip(&a[j]).map(|(x, y)| x * y).sum()
    }

    fn brute_top_k(vectors: &[Vec<f32>], query: usize, k: usize) -> Vec<i32> {
        let mut scored: Vec<(i32, f32)> = (0..vectors.len())
            .map(|i| (i as i32, dot(vectors, query, i)))
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        scored.into_iter().take(k).map(|(id, _)| id).collect()
    }

    /// Vectors used in the tests.
    fn test_vectors() -> Vec<Vec<f32>> {
        vec![
            vec![1.0, 0.0, 0.0],
            vec![0.9, 0.1, 0.0],
            vec![0.0, 1.0, 0.0],
            vec![0.1, 0.9, 0.0],
            vec![0.0, 0.0, 1.0],
            vec![0.0, 0.1, 0.9],
            vec![1.0, 1.0, 0.0],
            vec![0.5, 0.5, 0.5],
            vec![0.0, 0.0, 0.0],
            vec![0.99, 0.01, 0.0],
        ]
    }

    struct TestVectorScorer {
        vectors: Vec<Vec<f32>>,
        query_ord: i32,
    }

    impl TestVectorScorer {
        fn new(vectors: Vec<Vec<f32>>) -> Self {
            Self {
                vectors,
                query_ord: 0,
            }
        }
    }

    impl RandomVectorScorer for TestVectorScorer {
        fn score(&mut self, node: i32) -> Result<f32> {
            Ok(dot(&self.vectors, self.query_ord as usize, node as usize))
        }

        fn max_ord(&self) -> i32 {
            self.vectors.len() as i32
        }
    }

    impl UpdateableRandomVectorScorer for TestVectorScorer {
        fn set_scoring_ordinal(&mut self, node: i32) -> Result<()> {
            self.query_ord = node;
            Ok(())
        }
    }

    struct TestVectorScorerSupplier {
        vectors: Vec<Vec<f32>>,
    }

    impl TestVectorScorerSupplier {
        fn new(vectors: Vec<Vec<f32>>) -> Self {
            Self { vectors }
        }
    }

    impl RandomVectorScorerSupplier for TestVectorScorerSupplier {
        fn scorer(&self) -> Result<Box<dyn UpdateableRandomVectorScorer>> {
            Ok(Box::new(TestVectorScorer::new(self.vectors.clone())))
        }

        fn copy(&self) -> Result<Box<dyn RandomVectorScorerSupplier>> {
            Ok(Box::new(Self::new(self.vectors.clone())))
        }
    }

    fn build_test_graph(vectors: &[Vec<f32>], m: i32, beam_width: i32) -> Result<OnHeapHnswGraph> {
        let supplier = TestVectorScorerSupplier::new(vectors.to_vec());
        let mut builder =
            HnswGraphBuilder::create(&supplier, m, beam_width, 42, vectors.len() as i32)?;
        builder.build(vectors.len() as i32)?;
        Ok(builder.into_graph())
    }

    #[test]
    fn build_small_graph() {
        let vectors = test_vectors();
        let mut graph = build_test_graph(&vectors, 4, 20).expect("graph build failed");
        assert_eq!(graph.size(), vectors.len() as i32);
        assert!(graph.num_levels().unwrap() >= 1);
        assert!(graph.entry_node().unwrap() >= 0);
        // The graph should be connected (single component on level 0).
        assert!(HnswUtil::is_rooted(&mut graph as &mut dyn HnswGraph).expect("rooted check failed"));
    }

    #[test]
    fn search_returns_near_neighbors() {
        let vectors = test_vectors();
        let graph = build_test_graph(&vectors, 4, 20).expect("graph build failed");
        let query = 0usize;
        let k = 3i32;

        let supplier = TestVectorScorerSupplier::new(vectors.clone());
        let mut scorer = supplier.scorer().expect("scorer creation failed");
        scorer.set_scoring_ordinal(query as i32).unwrap();

        let collector =
            HnswGraphSearcher::search_on_heap(scorer.as_mut(), k, &graph, None, i64::MAX)
                .expect("search failed");

        let results: Vec<i32> = collector.nodes();
        let expected = brute_top_k(&vectors, query, k as usize);
        // HNSW is approximate; assert that at least 2 of the top-3 are found.
        let hits = results.iter().filter(|r| expected.contains(r)).count();
        assert!(
            hits >= 2,
            "expected at least 2 of top-3 in {:?}, got {:?}",
            expected,
            results
        );
    }

    #[test]
    fn merge_two_graphs() {
        let vectors_a = vec![
            vec![1.0, 0.0, 0.0],
            vec![0.9, 0.1, 0.0],
            vec![0.0, 1.0, 0.0],
        ];
        let vectors_b = vec![
            vec![0.0, 0.0, 1.0],
            vec![0.0, 0.1, 0.9],
            vec![0.5, 0.5, 0.5],
        ];

        let mut merged_vectors = vectors_a.clone();
        merged_vectors.extend(vectors_b.iter().cloned());

        let supplier = TestVectorScorerSupplier::new(merged_vectors.clone());
        // With no source graph readers registered, `ConcurrentHnswMerger` builds a
        // fresh graph through `HnswConcurrentMergeBuilder`; drive that builder
        // directly, so the test does not need a `KnnVectorsReader` to merge from.
        let graph = OnHeapHnswGraph::new(4, merged_vectors.len() as i32);
        let mut builder =
            HnswConcurrentMergeBuilder::new(2, &supplier, 4, 20, graph, None).expect("builder");
        builder
            .build(merged_vectors.len() as i32)
            .expect("merge failed");
        let mut merged = Box::new(builder)
            .into_completed_graph()
            .expect("merge failed");

        assert_eq!(merged.size(), merged_vectors.len() as i32);
        assert!(
            HnswUtil::is_rooted(&mut merged as &mut dyn HnswGraph).expect("rooted check failed")
        );

        // Verify that the merged graph still returns the closest vector for a
        // query from the first segment.
        let mut scorer = supplier.scorer().unwrap();
        scorer.set_scoring_ordinal(0).unwrap();
        let collector =
            HnswGraphSearcher::search_on_heap(scorer.as_mut(), 1, &merged, None, i64::MAX)
                .expect("search on merged graph failed");
        let results = collector.nodes();
        assert!(
            results.contains(&0),
            "merged graph should still find node 0 for query 0, got {:?}",
            results
        );
    }
}
