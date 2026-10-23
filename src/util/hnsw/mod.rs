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
    BorrowedBits, CloseableRandomVectorScorerSupplier, DelegatingCloseableSupplier,
    RandomVectorScorer, RandomVectorScorerSupplier, UpdateableRandomVectorScorer,
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
    ///
    /// Equivalent to `HnswGraph#getSortedNodes` (`HnswGraph.java:148-159`).
    ///
    /// # Panics
    ///
    /// Panics when the level's iterator yields more nodes than its `size()`,
    /// where Java's array store throws `ArrayIndexOutOfBoundsException`.
    fn get_sorted_nodes(&self, level: i32) -> Result<Box<dyn NodesIterator>> {
        if level == 0 {
            return Ok(Box::new(DenseNodesIterator::new(self.size())));
        }
        let mut nodes_on_level = self.get_nodes_on_level(level)?;
        let mut sorted_nodes = vec![0i32; nodes_on_level.size() as usize];
        // Java fills the array while `hasNext()` holds, not `size()` times: an
        // iterator that yields fewer nodes than it reports leaves zeros in the
        // tail, and one that yields more overruns the array.
        let mut n = 0;
        while nodes_on_level.has_next() {
            sorted_nodes[n] = nodes_on_level.next_int();
            n += 1;
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

    /// A recording [`InfoStream`], to prove that `setInfoStream` actually
    /// reaches the builder that logs.
    #[derive(Debug, Default)]
    struct RecordingInfoStream {
        messages: std::sync::Mutex<Vec<String>>,
    }

    impl crate::util::InfoStream for RecordingInfoStream {
        fn java_class_name(&self) -> &'static str {
            "org.apache.lucene.util.PrintStreamInfoStream"
        }

        fn message(&self, component: &str, message: &str) {
            self.messages
                .lock()
                .expect("lock")
                .push(format!("{component}: {message}"));
        }

        fn is_enabled(&self, _component: &str) -> bool {
            true
        }

        fn close(&self) {}
    }

    /// `HnswConcurrentMergeBuilder.setInfoStream` forwards the stream to the
    /// workers, so the builder that reports build progress must receive it.
    ///
    /// Java's implementation is
    /// `this.infoStream = infoStream; for (worker : workers) worker.setInfoStream(infoStream);`
    /// (`HnswConcurrentMergeBuilder.java:131-137`); executing on the calling
    /// thread means there is one worker here, and it is the one that logs.
    #[test]
    fn concurrent_merge_builder_forwards_the_info_stream_to_its_worker() {
        let vectors = vec![
            vec![1.0f32, 0.0, 0.0],
            vec![0.0, 1.0, 0.0],
            vec![0.0, 0.0, 1.0],
            vec![0.7, 0.7, 0.0],
            vec![0.5, 0.5, 0.5],
        ];
        let supplier = TestVectorScorerSupplier::new(vectors.clone());
        let graph = OnHeapHnswGraph::new(4, vectors.len() as i32);
        let mut builder =
            HnswConcurrentMergeBuilder::new(1, &supplier, 4, 20, graph, None).expect("builder");

        let stream = std::sync::Arc::new(RecordingInfoStream::default());
        builder.set_info_stream(stream.clone());
        builder.build(vectors.len() as i32).expect("build");

        let messages = stream.messages.lock().expect("lock");
        assert!(
            !messages.is_empty(),
            "the forwarded stream must receive the worker's build messages"
        );
        assert!(
            messages.iter().any(|m| m.starts_with("HNSW")),
            "expected HNSW component messages, got {messages:?}"
        );
    }

    /// `finish()` is what `getCompletedGraph()` calls to freeze the workers'
    /// builder, so the graph must be complete and the builder must refuse
    /// further work afterwards.
    #[test]
    fn concurrent_merge_builder_finish_freezes_the_graph() {
        let vectors = vec![
            vec![1.0f32, 0.0, 0.0],
            vec![0.0, 1.0, 0.0],
            vec![0.0, 0.0, 1.0],
        ];
        let supplier = TestVectorScorerSupplier::new(vectors.clone());
        let graph = OnHeapHnswGraph::new(4, vectors.len() as i32);
        let mut builder =
            HnswConcurrentMergeBuilder::new(1, &supplier, 4, 20, graph, None).expect("builder");

        builder.build(vectors.len() as i32).expect("build");
        // `build` ends in `getCompletedGraph()`, which calls `finish()`.
        assert_eq!(
            builder.get_completed_graph().expect("completed").size(),
            vectors.len() as i32
        );
        // A frozen builder refuses to build again, exactly as Java's does.
        assert!(builder.build(vectors.len() as i32).is_err());
    }

    /// A level iterator that reports `size` nodes but yields only `nodes`.
    struct ShortNodesIterator {
        nodes: Vec<i32>,
        size: i32,
        cur: usize,
    }

    impl super::NodesIterator for ShortNodesIterator {
        fn size(&self) -> i32 {
            self.size
        }

        fn next_int(&mut self) -> i32 {
            match self.nodes.get(self.cur) {
                Some(&node) => {
                    self.cur += 1;
                    node
                }
                None => crate::search::NO_MORE_DOCS,
            }
        }

        fn has_next(&self) -> bool {
            self.cur < self.nodes.len()
        }

        fn consume(&mut self, _dest: &mut [i32]) -> usize {
            unreachable!("getSortedNodes does not consume in bulk")
        }
    }

    /// A graph whose upper level is served by [`ShortNodesIterator`].
    struct ShortLevelGraph;

    impl HnswGraph for ShortLevelGraph {
        fn seek(&mut self, _level: i32, _target: i32) -> Result<()> {
            Ok(())
        }

        fn size(&self) -> i32 {
            8
        }

        fn next_neighbor(&mut self) -> Result<i32> {
            Ok(crate::search::NO_MORE_DOCS)
        }

        fn num_levels(&self) -> Result<i32> {
            Ok(2)
        }

        fn max_conn(&self) -> i32 {
            4
        }

        fn entry_node(&self) -> Result<i32> {
            Ok(7)
        }

        fn get_nodes_on_level(&self, _level: i32) -> Result<Box<dyn super::NodesIterator>> {
            Ok(Box::new(ShortNodesIterator {
                nodes: vec![7, 5],
                size: 3,
                cur: 0,
            }))
        }

        fn neighbor_count(&self) -> i32 {
            0
        }
    }

    /// `getSortedNodes` fills its array while `hasNext()` holds
    /// (`HnswGraph.java:152-155`): an iterator reporting three nodes but
    /// yielding two leaves a zero in the third slot, which sorts first.
    /// Calling `nextInt()` `size()` times instead would read past the end.
    #[test]
    fn get_sorted_nodes_stops_when_the_level_iterator_has_no_next() {
        let mut sorted = ShortLevelGraph.get_sorted_nodes(1).expect("sorted nodes");
        assert_eq!(sorted.size(), 3);
        let nodes: Vec<i32> = (0..3).map(|_| sorted.next_int()).collect();
        assert_eq!(nodes, vec![0, 5, 7]);
    }

    /// The number a message prints right before `unit`, e.g. the `12.34` of
    /// `"... in 12.34 ms"`.
    fn number_before(message: &str, unit: &str) -> f64 {
        let end = message
            .find(unit)
            .unwrap_or_else(|| panic!("no `{unit}` in {message:?}"));
        let start = message[..end].trim_end().rfind(' ').map_or(0, |i| i + 1);
        message[start..end]
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("no number before `{unit}` in {message:?}"))
    }

    /// `setMergeStartTimeNs` moves the origin of the progress reports to the
    /// start of the merge, and `setCumulativeWorkTimeNs` makes each
    /// `addVectors` chunk add its elapsed time to the shared accumulator —
    /// exactly the time its own `addVectors` message reports
    /// (`HnswGraphBuilder.java:102-109,246-275`). Without either, the
    /// progress origin is the chunk's own start and nothing is accumulated.
    #[test]
    fn merge_start_time_and_cumulative_work_time_reach_add_vectors() {
        use std::sync::atomic::{AtomicI64, Ordering};
        use std::sync::Arc;

        let vectors = test_vectors();
        let supplier = TestVectorScorerSupplier::new(vectors.clone());
        let n = vectors.len() as i32;

        let mut builder = HnswGraphBuilder::create(&supplier, 4, 20, 42, n).expect("builder");
        let stream = Arc::new(RecordingInfoStream::default());
        builder.set_info_stream(stream.clone());
        builder.set_merge_start_time_ns(crate::index::merge_scheduler::now_ns() - 5_000_000_000);
        let cumulative = Arc::new(AtomicI64::new(0));
        builder.set_cumulative_work_time_ns(Some(Arc::clone(&cumulative)));
        builder.add_vectors(0, n).expect("add vectors");

        let messages = stream.messages.lock().expect("lock").clone();
        let built = messages
            .iter()
            .find(|m| m.starts_with("HNSW: built 0 in "))
            .unwrap_or_else(|| panic!("no progress report in {messages:?}"));
        assert!(
            number_before(built, " ms") >= 5000.0,
            "progress must be measured from the merge start: {built}"
        );
        let added = messages
            .iter()
            .find(|m| m.starts_with(&format!("HNSW: addVectors [0 {n}): {n} vectors in ")))
            .unwrap_or_else(|| panic!("no addVectors report in {messages:?}"));
        let accumulated = cumulative.load(Ordering::Relaxed);
        assert!(accumulated > 0, "the chunk's time must be accumulated");
        assert_eq!(
            format!("{:.2} ms", accumulated as f64 / 1_000_000.0),
            added.rsplit(" in ").next().unwrap(),
            "the accumulator holds exactly the time the chunk reports"
        );

        // The defaults: progress from the chunk's own start, no accumulator.
        let mut plain = HnswGraphBuilder::create(&supplier, 4, 20, 42, n).expect("builder");
        let plain_stream = Arc::new(RecordingInfoStream::default());
        plain.set_info_stream(plain_stream.clone());
        plain.add_vectors(0, n).expect("add vectors");
        let plain_messages = plain_stream.messages.lock().expect("lock").clone();
        let plain_built = plain_messages
            .iter()
            .find(|m| m.starts_with("HNSW: built 0 in "))
            .expect("progress report");
        assert!(number_before(plain_built, " ms") < 5000.0, "{plain_built}");
    }

    /// The concurrent merge sets both on its workers and reports the merge
    /// as a whole, with the two messages of
    /// `HnswConcurrentMergeBuilder.java:85-121`. All work runs on the calling
    /// thread here, so the cumulative worker time cannot exceed the wall
    /// clock and the effective concurrency is at most `1x`.
    #[test]
    fn concurrent_merge_reports_the_merge_with_its_cumulative_worker_time() {
        let vectors = test_vectors();
        let n = vectors.len() as i32;
        let supplier = TestVectorScorerSupplier::new(vectors.clone());
        let graph = OnHeapHnswGraph::new(4, n);
        let mut builder =
            HnswConcurrentMergeBuilder::new(2, &supplier, 4, 20, graph, None).expect("builder");
        builder.set_batch_size(3);
        let stream = std::sync::Arc::new(RecordingInfoStream::default());
        builder.set_info_stream(stream.clone());
        builder.build(n).expect("build");

        let messages = stream.messages.lock().expect("lock").clone();
        assert_eq!(
            messages.first().map(String::as_str),
            Some(format!("HNSW: build graph from {n} vectors, with 2 workers").as_str()),
            "{messages:?}"
        );
        let completed = messages.last().expect("messages");
        assert!(
            completed.starts_with(&format!("HNSW: merge completed: {n} vectors, ")),
            "{messages:?}"
        );
        let wall = number_before(completed, " ms wall clock");
        let worker = number_before(completed, " ms cumulative worker time");
        let concurrency = number_before(completed, "x effective concurrency");
        assert!(worker > 0.0, "{completed}");
        assert!(worker <= wall + 0.01, "{completed}");
        assert!(concurrency <= 1.0, "{completed}");
        // Four chunks of at most three vectors each, every one reported.
        assert_eq!(
            messages
                .iter()
                .filter(|m| m.starts_with("HNSW: addVectors ["))
                .count(),
            4,
            "{messages:?}"
        );
    }
}
