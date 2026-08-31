//! Top-k collector creation, ported from
//! `org.apache.lucene.search.knn.TopKnnCollectorManager`.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::index::LeafReaderContext;
use crate::search::index_searcher::IndexSearcher;
use crate::search::knn::knn_collector_manager::KnnCollectorManager;
use crate::search::knn::{KnnCollector, KnnSearchStrategy, TopKnnCollector};

/// Creates [`TopKnnCollector`] instances.
///
/// Equivalent to `org.apache.lucene.search.knn.TopKnnCollectorManager`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TopKnnCollectorManager {
    /// The number of documents to collect.
    k: i32,
}

impl TopKnnCollectorManager {
    /// Creates a manager collecting `k` documents per leaf.
    ///
    /// Equivalent to `new TopKnnCollectorManager(int, IndexSearcher)`, whose
    /// searcher argument is unused; this port keeps the parameter so that call
    /// sites read the same, and ignores it for the same reason Java does.
    pub fn new(k: i32, _index_searcher: &IndexSearcher) -> Self {
        Self { k }
    }

    /// Creates a manager collecting `k` documents per leaf, without a searcher.
    ///
    /// **Divergence from Lucene 10.5.0.** Java has no such constructor, but its
    /// only one ignores the searcher; this one exists so that callers who have
    /// no searcher at hand do not have to invent one.
    pub fn with_k(k: i32) -> Self {
        Self { k }
    }

    /// Returns the number of documents this manager collects.
    pub fn k(&self) -> i32 {
        self.k
    }
}

impl KnnCollectorManager for TopKnnCollectorManager {
    fn new_collector(
        &self,
        visited_limit: i32,
        search_strategy: Option<&KnnSearchStrategy>,
        _context: &LeafReaderContext,
    ) -> Result<Box<dyn KnnCollector>> {
        Ok(Box::new(TopKnnCollector::with_strategy(
            self.k,
            visited_limit as i64,
            search_strategy.cloned(),
        )))
    }

    fn new_optimistic_collector(
        &self,
        visited_limit: i32,
        search_strategy: Option<&KnnSearchStrategy>,
        _context: &LeafReaderContext,
        k: i32,
    ) -> Result<Option<Box<dyn KnnCollector>>> {
        Ok(Some(Box::new(TopKnnCollector::with_strategy(
            k,
            visited_limit as i64,
            search_strategy.cloned(),
        ))))
    }

    fn is_optimistic(&self) -> bool {
        true
    }
}
