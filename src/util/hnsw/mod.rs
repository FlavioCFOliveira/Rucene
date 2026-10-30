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
    use super::on_heap::OnHeapHnswGraph;
    use super::scorer::{
        RandomVectorScorer, RandomVectorScorerSupplier, UpdateableRandomVectorScorer,
    };
    use super::searcher::HnswGraphSearcher;
    use super::util::HnswUtil;
    use super::{HnswGraph, Result};
    use crate::util::hnsw::concurrent_merge_builder::HnswConcurrentMergeBuilder;

    /// A task executor that runs every task on the calling thread, Java's
    /// `new TaskExecutor(Runnable::run)`.
    fn same_thread() -> std::sync::Arc<crate::search::TaskExecutor> {
        std::sync::Arc::new(crate::search::TaskExecutor::same_thread())
    }

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
        builder.into_graph()
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
            HnswConcurrentMergeBuilder::new(same_thread(), 2, &supplier, 4, 20, graph, None)
                .expect("builder");
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
    /// (`HnswConcurrentMergeBuilder.java:131-137`); here each worker is handed
    /// the stream when its task builds it, and the single worker logs.
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
            HnswConcurrentMergeBuilder::new(same_thread(), 1, &supplier, 4, 20, graph, None)
                .expect("builder");

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
            HnswConcurrentMergeBuilder::new(same_thread(), 1, &supplier, 4, 20, graph, None)
                .expect("builder");

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
    /// `HnswConcurrentMergeBuilder.java:85-121`. The task executor here runs
    /// every worker on the calling thread, one after the other, so the
    /// cumulative worker time cannot exceed the wall clock and the effective
    /// concurrency is at most `1x`.
    #[test]
    fn concurrent_merge_reports_the_merge_with_its_cumulative_worker_time() {
        let vectors = test_vectors();
        let n = vectors.len() as i32;
        let supplier = TestVectorScorerSupplier::new(vectors.clone());
        let graph = OnHeapHnswGraph::new(4, n);
        let mut builder =
            HnswConcurrentMergeBuilder::new(same_thread(), 2, &supplier, 4, 20, graph, None)
                .expect("builder");
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

    // -------------------------------------------------------------------------
    // HnswConcurrentMergeBuilder on a real thread pool
    // -------------------------------------------------------------------------

    use std::collections::HashSet;
    use std::sync::{Arc, Mutex};
    use std::thread::ThreadId;

    /// An executor that runs every command on a thread of its own, so that a
    /// forked worker really runs off the calling thread.
    struct ThreadPerTask;

    impl crate::search::Executor for ThreadPerTask {
        fn execute(
            &self,
            command: crate::search::Command,
        ) -> std::result::Result<(), crate::search::RejectedExecutionException> {
            std::thread::spawn(command);
            Ok(())
        }
    }

    fn thread_pool() -> Arc<crate::search::TaskExecutor> {
        Arc::new(crate::search::TaskExecutor::new(Arc::new(ThreadPerTask)))
    }

    /// `n` unit vectors of dimension `dim`, from a fixed SplitMix64 stream.
    fn random_unit_vectors(n: usize, dim: usize, seed: u64) -> Vec<Vec<f32>> {
        let mut state = seed;
        let mut next = move || {
            state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^= z >> 31;
            (z >> 40) as f32 / (1u64 << 24) as f32 - 0.5
        };
        (0..n)
            .map(|_| {
                let v: Vec<f32> = (0..dim).map(|_| next()).collect();
                let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
                v.into_iter().map(|x| x / norm).collect()
            })
            .collect()
    }

    /// Lucene's `DOT_PRODUCT` similarity of two unit vectors, `(1 + dot) / 2`,
    /// which keeps every score non-negative as Lucene's similarities do.
    fn dot_product_score(a: &[f32], b: &[f32]) -> f32 {
        (1.0 + a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>()) / 2.0
    }

    /// A dot-product scorer that records, on its first score, the thread
    /// that uses it.
    struct RecordingScorer {
        vectors: Arc<Vec<Vec<f32>>>,
        query: Vec<f32>,
        threads: Arc<Mutex<HashSet<ThreadId>>>,
        recorded: bool,
        start_latch: Option<Arc<StartLatch>>,
    }

    /// A latch every worker's first score waits on until `parties` workers
    /// have arrived, so that no worker can drain the batches before the
    /// others have started. It gives up after a minute, so that a merge whose
    /// workers cannot all run at once fails its assertions instead of hanging.
    struct StartLatch {
        parties: usize,
        arrived: Mutex<usize>,
        all_arrived: std::sync::Condvar,
    }

    impl StartLatch {
        fn new(parties: usize) -> Self {
            Self {
                parties,
                arrived: Mutex::new(0),
                all_arrived: std::sync::Condvar::new(),
            }
        }

        fn arrive_and_wait(&self) {
            let mut arrived = self.arrived.lock().expect("latch");
            *arrived += 1;
            self.all_arrived.notify_all();
            let _ = self
                .all_arrived
                .wait_timeout_while(arrived, std::time::Duration::from_secs(60), |n| {
                    *n < self.parties
                })
                .expect("latch");
        }
    }

    impl RandomVectorScorer for RecordingScorer {
        fn score(&mut self, node: i32) -> Result<f32> {
            if !self.recorded {
                self.recorded = true;
                self.threads
                    .lock()
                    .expect("the thread log is not poisoned")
                    .insert(std::thread::current().id());
                if let Some(latch) = &self.start_latch {
                    latch.arrive_and_wait();
                }
            }
            Ok(dot_product_score(&self.vectors[node as usize], &self.query))
        }

        fn max_ord(&self) -> i32 {
            self.vectors.len() as i32
        }
    }

    impl UpdateableRandomVectorScorer for RecordingScorer {
        fn set_scoring_ordinal(&mut self, node: i32) -> Result<()> {
            self.query.clone_from(&self.vectors[node as usize]);
            Ok(())
        }
    }

    /// A supplier whose scorers record the threads that score with them.
    struct RecordingSupplier {
        vectors: Arc<Vec<Vec<f32>>>,
        threads: Arc<Mutex<HashSet<ThreadId>>>,
        start_latch: Option<Arc<StartLatch>>,
    }

    impl RandomVectorScorerSupplier for RecordingSupplier {
        fn scorer(&self) -> Result<Box<dyn UpdateableRandomVectorScorer>> {
            Ok(Box::new(RecordingScorer {
                vectors: Arc::clone(&self.vectors),
                query: vec![0.0; self.vectors[0].len()],
                threads: Arc::clone(&self.threads),
                recorded: false,
                start_latch: self.start_latch.clone(),
            }))
        }

        fn copy(&self) -> Result<Box<dyn RandomVectorScorerSupplier>> {
            Ok(Box::new(Self {
                vectors: Arc::clone(&self.vectors),
                threads: Arc::clone(&self.threads),
                start_latch: self.start_latch.clone(),
            }))
        }
    }

    /// Merges `vectors` with `num_worker` workers on `executor` and returns
    /// the graph together with the threads that scored.
    fn concurrent_merge(
        executor: Arc<crate::search::TaskExecutor>,
        vectors: &Arc<Vec<Vec<f32>>>,
        num_worker: i32,
        batch_size: i32,
    ) -> (OnHeapHnswGraph, HashSet<ThreadId>) {
        concurrent_merge_with_latch(executor, vectors, num_worker, batch_size, None)
    }

    /// As [`concurrent_merge`], each worker's first score waiting on
    /// `start_latch` when one is given.
    fn concurrent_merge_with_latch(
        executor: Arc<crate::search::TaskExecutor>,
        vectors: &Arc<Vec<Vec<f32>>>,
        num_worker: i32,
        batch_size: i32,
        start_latch: Option<Arc<StartLatch>>,
    ) -> (OnHeapHnswGraph, HashSet<ThreadId>) {
        let threads = Arc::new(Mutex::new(HashSet::new()));
        let supplier = RecordingSupplier {
            vectors: Arc::clone(vectors),
            threads: Arc::clone(&threads),
            start_latch,
        };
        let n = vectors.len() as i32;
        let mut builder = HnswConcurrentMergeBuilder::new(
            executor,
            num_worker,
            &supplier,
            16,
            100,
            OnHeapHnswGraph::new(16, n),
            None,
        )
        .expect("builder");
        builder.set_batch_size(batch_size);
        builder.build(n).expect("build");
        let graph = Box::new(builder)
            .into_completed_graph()
            .expect("completed graph");
        let threads = threads.lock().expect("thread log").clone();
        (graph, threads)
    }

    /// Checks what every HNSW graph must satisfy: `n` nodes, each present on
    /// level 0; every neighbour a valid, distinct node other than its owner
    /// that exists on the same level; at most `2M` neighbours on level 0 and
    /// `M` above; every node of an upper level present on all levels below;
    /// and an entry node on the top level.
    fn assert_graph_is_well_formed(graph: &OnHeapHnswGraph, n: i32, m: i32) {
        assert_eq!(HnswGraph::size(graph), n);
        let num_levels = graph.num_levels().expect("levels");
        let entry = graph.entry_node().expect("entry node");
        assert!(graph.node_exists_at_level(num_levels - 1, entry));
        for level in 0..num_levels {
            let max_conn = if level == 0 { 2 * m } else { m };
            let mut nodes = graph.get_nodes_on_level(level).expect("nodes on level");
            let mut count = 0;
            while nodes.has_next() {
                let node = nodes.next_int();
                count += 1;
                for lower in 0..level {
                    assert!(
                        graph.node_exists_at_level(lower, node),
                        "node {node} is on level {level} but not on level {lower}"
                    );
                }
                let neighbors = graph.get_neighbors(level, node).expect("neighbours");
                assert!(neighbors.size() <= max_conn, "node {node} level {level}");
                let mut seen = HashSet::new();
                for &nbr in neighbors.nodes() {
                    assert!((0..n).contains(&nbr), "node {node} links to {nbr}");
                    assert_ne!(nbr, node, "self-link on level {level}");
                    assert!(seen.insert(nbr), "node {node} links twice to {nbr}");
                    assert!(
                        graph.node_exists_at_level(level, nbr),
                        "node {node} links on level {level} to {nbr}, which is not there"
                    );
                }
            }
            if level == 0 {
                assert_eq!(count, n);
            }
        }
    }

    /// Mean recall@10 of `graph` over `queries`, against exact search: each
    /// query searches the graph with a beam of `fanout` and keeps its ten
    /// best results.
    fn recall_at_10(
        graph: &OnHeapHnswGraph,
        vectors: &Arc<Vec<Vec<f32>>>,
        queries: &[Vec<f32>],
        fanout: i32,
    ) -> f64 {
        let k = 10usize;
        let mut hits = 0usize;
        for query in queries {
            let score = |v: &Vec<f32>| dot_product_score(v, query);
            let mut exact: Vec<(usize, f32)> = vectors
                .iter()
                .enumerate()
                .map(|(i, v)| (i, score(v)))
                .collect();
            exact.sort_by(|a, b| b.1.total_cmp(&a.1));
            let truth: HashSet<i32> = exact[..k].iter().map(|(i, _)| *i as i32).collect();
            let mut scorer = RecordingScorer {
                vectors: Arc::clone(vectors),
                query: query.clone(),
                threads: Arc::new(Mutex::new(HashSet::new())),
                recorded: false,
                start_latch: None,
            };
            let mut found: Vec<(i32, f32)> =
                HnswGraphSearcher::search_on_heap(&mut scorer, fanout, graph, None, i64::MAX)
                    .expect("search")
                    .nodes()
                    .into_iter()
                    .map(|node| (node, score(&vectors[node as usize])))
                    .collect();
            found.sort_by(|a, b| b.1.total_cmp(&a.1));
            hits += found
                .iter()
                .take(k)
                .filter(|(node, _)| truth.contains(node))
                .count();
        }
        hits as f64 / (queries.len() * k) as f64
    }

    /// With a pooled executor and several workers, Java's
    /// `taskExecutor.invokeAll(futures)` runs the workers on the pool
    /// (`HnswConcurrentMergeBuilder.java:94-103`): every worker must score on
    /// a thread of its own.
    ///
    /// Each worker's first score waits until all four have reached theirs.
    /// There are far more batches than workers, so every worker holds a batch
    /// by then, and none can drain the work before the others start. Run
    /// sequentially on the calling thread, the first worker alone reaches the
    /// latch, which gives up after a minute, and the test fails.
    #[test]
    fn concurrent_merge_runs_its_workers_on_the_executor_threads() {
        let vectors = Arc::new(random_unit_vectors(3000, 16, 7));
        let latch = Arc::new(StartLatch::new(4));
        let (graph, threads) =
            concurrent_merge_with_latch(thread_pool(), &vectors, 4, 32, Some(latch));
        let caller = std::thread::current().id();
        assert_eq!(
            threads.len(),
            4,
            "each worker scores on its own thread: {threads:?}"
        );
        assert!(
            threads.iter().any(|id| *id != caller),
            "every worker scored on the calling thread: {threads:?}"
        );
        assert_graph_is_well_formed(&graph, 3000, 16);
    }

    /// On a same-thread executor with one worker, the concurrent merge
    /// inserts every node in order with a level generator seeded with
    /// `randSeed`, exactly as `HnswGraphBuilder.create(.., 42, n).build(n)`
    /// does, so the two graphs are identical — whatever the batch size, and
    /// even with four workers, since the first one then drains every batch.
    #[test]
    fn a_same_thread_merge_matches_the_single_threaded_build() {
        let vectors = Arc::new(random_unit_vectors(2000, 16, 53));
        let n = vectors.len() as i32;
        let supplier = RecordingSupplier {
            vectors: Arc::clone(&vectors),
            threads: Arc::new(Mutex::new(HashSet::new())),
            start_latch: None,
        };
        let mut builder = HnswGraphBuilder::create(&supplier, 16, 100, 42, n).expect("builder");
        builder.build(n).expect("build");
        let expected = neighbour_lists(&builder.into_graph().expect("graph"));
        for (workers, batch) in [(1, 2048), (1, 64), (4, 64)] {
            let (merged, _) = concurrent_merge(same_thread(), &vectors, workers, batch);
            assert_eq!(
                neighbour_lists(&merged),
                expected,
                "{workers} worker(s), batch {batch}"
            );
        }
    }

    /// A merge built by four workers on a pool is a well-formed graph whose
    /// recall@10 is within 0.01 of the single-worker merge's, on a fixed
    /// corpus and fixed queries.
    ///
    /// The queries search with a beam of 50. With a beam of 10 the measure is
    /// dominated by which graph instance was drawn, not by how it was built:
    /// on this corpus single-threaded builds seeded 42 and 1..=7 alone span
    /// recall 0.749..=0.773, wider than the bound, while with a beam of 50
    /// the single-worker and four-worker merges measured 0.9917 and
    /// 0.9907..=0.9927 over six runs.
    #[test]
    fn concurrent_merge_on_a_pool_matches_the_single_worker_recall() {
        let vectors = Arc::new(random_unit_vectors(4000, 32, 11));
        let queries = random_unit_vectors(300, 32, 12);
        let (single, _) = concurrent_merge(same_thread(), &vectors, 1, 2048);
        let (pooled, threads) = concurrent_merge(thread_pool(), &vectors, 4, 64);
        assert!(threads.len() >= 2, "{threads:?}");
        assert_graph_is_well_formed(&single, 4000, 16);
        assert_graph_is_well_formed(&pooled, 4000, 16);
        let single_recall = recall_at_10(&single, &vectors, &queries, 50);
        let pooled_recall = recall_at_10(&pooled, &vectors, &queries, 50);
        assert!(
            (single_recall - pooled_recall).abs() <= 0.01,
            "single-worker recall {single_recall}, four-worker recall {pooled_recall}"
        );
    }

    /// Many workers fighting over small batches, many times over, must
    /// neither deadlock nor panic, and must always produce a well-formed
    /// graph. Each merge runs under a watchdog, so a deadlock fails the test
    /// instead of hanging it.
    #[test]
    fn concurrent_merge_with_many_workers_neither_deadlocks_nor_panics() {
        let vectors = Arc::new(random_unit_vectors(1500, 8, 23));
        for round in 0..20 {
            let vectors = Arc::clone(&vectors);
            let (sender, receiver) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let (graph, threads) = concurrent_merge(thread_pool(), &vectors, 16, 8);
                let _ = sender.send((graph, threads));
            });
            let (graph, threads) = receiver
                .recv_timeout(std::time::Duration::from_secs(120))
                .unwrap_or_else(|e| panic!("round {round}: the merge did not finish: {e}"));
            assert!(threads.len() >= 2, "round {round}: {threads:?}");
            assert_graph_is_well_formed(&graph, 1500, 16);
        }
    }

    /// Every neighbour list of `graph`, level by level, node by node.
    fn neighbour_lists(graph: &OnHeapHnswGraph) -> Vec<Vec<Vec<i32>>> {
        (0..graph.num_levels().expect("levels"))
            .map(|level| {
                let mut nodes = graph.get_nodes_on_level(level).expect("nodes");
                let mut lists = Vec::new();
                while nodes.has_next() {
                    let node = nodes.next_int();
                    lists.push(
                        graph
                            .get_neighbors(level, node)
                            .expect("nbrs")
                            .nodes()
                            .to_vec(),
                    );
                }
                lists
            })
            .collect()
    }

    /// `InitializedHnswGraphBuilder` overrides `addGraphNode(int)` to return
    /// early for a node the initializer graph already placed, and inherits
    /// `build`, so building over a fully initialized graph adds nothing
    /// (`InitializedHnswGraphBuilder.java:446-452`, `HnswGraphBuilder.java:217-226`).
    #[test]
    fn initialized_builder_build_skips_the_initialized_nodes() {
        let vectors = Arc::new(random_unit_vectors(200, 8, 31));
        let n = vectors.len() as i32;
        let supplier = RecordingSupplier {
            vectors: Arc::clone(&vectors),
            threads: Arc::new(Mutex::new(HashSet::new())),
            start_latch: None,
        };
        let mut source = HnswGraphBuilder::create(&supplier, 16, 100, 42, n).expect("builder");
        source.build(n).expect("build");
        let mut source = source.into_graph().expect("graph");
        let identity: Vec<i32> = (0..n).collect();
        let mut initialized = crate::util::FixedBitSet::new(n as usize);
        for node in 0..n as usize {
            initialized.set(node);
        }
        let mut builder = super::InitializedHnswGraphBuilder::from_graph(
            &supplier,
            100,
            42,
            &mut source,
            &identity,
            Some(initialized),
            n,
        )
        .expect("initialized builder");
        let before = neighbour_lists(builder.get_graph());
        builder.build(n).expect("build");
        assert_eq!(
            neighbour_lists(builder.get_graph()),
            before,
            "build must not re-insert a node the initializer graph placed"
        );
    }

    /// A scorer returning the raw dot product, negative for vectors more than
    /// a right angle apart.
    struct RawDotScorer {
        vectors: Arc<Vec<Vec<f32>>>,
        query: Vec<f32>,
    }

    impl RandomVectorScorer for RawDotScorer {
        fn score(&mut self, node: i32) -> Result<f32> {
            Ok(self.vectors[node as usize]
                .iter()
                .zip(&self.query)
                .map(|(a, b)| a * b)
                .sum())
        }

        fn max_ord(&self) -> i32 {
            self.vectors.len() as i32
        }
    }

    impl UpdateableRandomVectorScorer for RawDotScorer {
        fn set_scoring_ordinal(&mut self, node: i32) -> Result<()> {
            self.query.clone_from(&self.vectors[node as usize]);
            Ok(())
        }
    }

    struct RawDotSupplier(Arc<Vec<Vec<f32>>>);

    impl RandomVectorScorerSupplier for RawDotSupplier {
        fn scorer(&self) -> Result<Box<dyn UpdateableRandomVectorScorer>> {
            Ok(Box::new(RawDotScorer {
                vectors: Arc::clone(&self.0),
                query: vec![0.0; self.0[0].len()],
            }))
        }

        fn copy(&self) -> Result<Box<dyn RandomVectorScorerSupplier>> {
            Ok(Box::new(Self(Arc::clone(&self.0))))
        }
    }

    /// Negative scores order as Java's signed `long` encoding orders them, so
    /// building over a scorer that yields them keeps every candidate list in
    /// score order: with debug assertions on, `NeighborArray::add_in_order`
    /// panicked here while the queue compared the encoded arcs unsigned.
    #[test]
    fn graph_builds_over_negative_scores() {
        let vectors = Arc::new(random_unit_vectors(500, 8, 41));
        let n = vectors.len() as i32;
        let supplier = RawDotSupplier(Arc::clone(&vectors));
        let mut builder = HnswGraphBuilder::create(&supplier, 16, 100, 42, n).expect("builder");
        builder.build(n).expect("build");
        let graph = builder.into_graph().expect("graph");
        assert_graph_is_well_formed(&graph, n, 16);
    }

    /// Java's `InitializedHnswGraphBuilder` overrides only `addGraphNode(int)`;
    /// the inherited `addGraphNode(int, IntHashSet)` inserts the node even
    /// when the initializer graph already placed it.
    #[test]
    fn initialized_builder_adds_an_initialized_node_given_entry_points() {
        let vectors = Arc::new(random_unit_vectors(200, 8, 31));
        let n = vectors.len() as i32;
        let supplier = RecordingSupplier {
            vectors: Arc::clone(&vectors),
            threads: Arc::new(Mutex::new(HashSet::new())),
            start_latch: None,
        };
        let mut source = HnswGraphBuilder::create(&supplier, 16, 100, 42, n).expect("builder");
        source.build(n).expect("build");
        let mut source = source.into_graph().expect("graph");
        let identity: Vec<i32> = (0..n).collect();
        let mut initialized = crate::util::FixedBitSet::new(n as usize);
        for node in 0..n as usize {
            initialized.set(node);
        }
        let mut builder = super::InitializedHnswGraphBuilder::from_graph(
            &supplier,
            100,
            42,
            &mut source,
            &identity,
            Some(initialized),
            n,
        )
        .expect("initialized builder");
        let before = neighbour_lists(builder.get_graph());
        builder.add_graph_node(0).expect("add");
        assert_eq!(
            neighbour_lists(builder.get_graph()),
            before,
            "addGraphNode(int) skips"
        );
        let mut eps = crate::internal::hppc::IntHashSet::new();
        eps.add(1);
        builder
            .add_graph_node_with_eps(0, &eps)
            .expect("add with eps");
        assert_ne!(
            neighbour_lists(builder.get_graph()),
            before,
            "addGraphNode(int, IntHashSet) inserts the node again"
        );
    }

    /// A worker that adds a level twice is refused, and the refused call
    /// creates no neighbour array, so the graph's memory is not charged.
    #[test]
    fn add_node_shared_refuses_a_set_slot_without_charging_memory() {
        use crate::util::Accountable;
        let graph = OnHeapHnswGraph::new(4, 2);
        graph.add_node_shared(0, 0).expect("first add");
        let charged = graph.ram_bytes_used();
        assert!(matches!(
            graph.add_node_shared(0, 0),
            Err(crate::error::LuceneError::IllegalState { .. })
        ));
        assert_eq!(graph.ram_bytes_used(), charged);
        assert_eq!(HnswGraph::size(&graph), 1);
    }
}
