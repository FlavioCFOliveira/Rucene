//! kNN collector creation, ported from
//! `org.apache.lucene.search.knn.KnnCollectorManager`.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::index::LeafReaderContext;
use crate::search::knn::{KnnCollector, KnnSearchStrategy};

/// Creates the [`KnnCollector`] instances of one kNN search.
///
/// Equivalent to the interface
/// `org.apache.lucene.search.knn.KnnCollectorManager`. It is useful to create
/// collectors that share global state across leaves, such as a global queue of
/// the results collected so far.
///
/// **Divergence from Lucene 10.5.0.** Java's `newOptimisticCollector` returns
/// `null` when optimistic collection is not supported; this port returns
/// `Option::None`, which is the same signal with a type behind it. A manager is
/// `Send + Sync` because the leaves are searched concurrently and every task
/// shares the one manager, exactly as in Java, and `Debug` because the queries
/// that hold one are.
pub trait KnnCollectorManager: Send + Sync + std::fmt::Debug {
    /// Returns a new collector for a leaf.
    ///
    /// Equivalent to
    /// `KnnCollectorManager.newCollector(int, KnnSearchStrategy, LeafReaderContext)`.
    ///
    /// * `visited_limit` — the maximum number of nodes the search may visit;
    /// * `search_strategy` — the optional search strategy configuration;
    /// * `context` — the leaf reader context.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while creating the collector.
    fn new_collector(
        &self,
        visited_limit: i32,
        search_strategy: Option<&KnnSearchStrategy>,
        context: &LeafReaderContext,
    ) -> Result<Box<dyn KnnCollector>>;

    /// Returns a new collector with a specific `k`, scaled by per-leaf
    /// statistics, or `None` when this manager does not collect optimistically.
    ///
    /// Equivalent to
    /// `KnnCollectorManager.newOptimisticCollector(int, KnnSearchStrategy, LeafReaderContext, int)`,
    /// which returns `null` by default.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while creating the collector.
    fn new_optimistic_collector(
        &self,
        _visited_limit: i32,
        _search_strategy: Option<&KnnSearchStrategy>,
        _context: &LeafReaderContext,
        _k: i32,
    ) -> Result<Option<Box<dyn KnnCollector>>> {
        Ok(None)
    }

    /// Returns whether this manager collects optimistically.
    ///
    /// Equivalent to `KnnCollectorManager.isOptimistic()`, `false` by default.
    fn is_optimistic(&self) -> bool {
        false
    }
}
