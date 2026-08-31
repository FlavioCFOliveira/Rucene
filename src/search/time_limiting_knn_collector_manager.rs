//! Timed kNN collection, ported from
//! `org.apache.lucene.search.TimeLimitingKnnCollectorManager`.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::error::Result;
use crate::index::{LeafReaderContext, QueryTimeout};
use crate::search::knn::knn_collector_manager::KnnCollectorManager;
use crate::search::knn::{KnnCollector, KnnSearchStrategy, TopDocs};
use crate::search::total_hits::{TotalHits, TotalHitsRelation};

/// A [`KnnCollectorManager`] that collects results with a timeout.
///
/// Equivalent to `org.apache.lucene.search.TimeLimitingKnnCollectorManager`.
#[derive(Debug)]
pub struct TimeLimitingKnnCollectorManager {
    delegate: Arc<dyn KnnCollectorManager>,
    query_timeout: Option<Arc<dyn QueryTimeout>>,
}

impl TimeLimitingKnnCollectorManager {
    /// Wraps `delegate`, terminating its collectors once `timeout` expires.
    ///
    /// Equivalent to
    /// `new TimeLimitingKnnCollectorManager(KnnCollectorManager, QueryTimeout)`,
    /// whose timeout may be `null`.
    pub fn new(
        delegate: Arc<dyn KnnCollectorManager>,
        timeout: Option<Arc<dyn QueryTimeout>>,
    ) -> Self {
        Self {
            delegate,
            query_timeout: timeout,
        }
    }

    /// Returns the configured timeout for terminating graph and exact searches.
    ///
    /// Equivalent to `TimeLimitingKnnCollectorManager.getQueryTimeout()`.
    pub fn get_query_timeout(&self) -> Option<&Arc<dyn QueryTimeout>> {
        self.query_timeout.as_ref()
    }
}

impl KnnCollectorManager for TimeLimitingKnnCollectorManager {
    fn new_collector(
        &self,
        visited_limit: i32,
        search_strategy: Option<&KnnSearchStrategy>,
        context: &LeafReaderContext,
    ) -> Result<Box<dyn KnnCollector>> {
        let collector = self
            .delegate
            .new_collector(visited_limit, search_strategy, context)?;
        match &self.query_timeout {
            None => Ok(collector),
            Some(timeout) => Ok(Box::new(TimeLimitingKnnCollector::new(
                collector,
                Arc::clone(timeout),
            ))),
        }
    }
}

/// A [`KnnCollector`] that reports early termination once its timeout expires.
///
/// Equivalent to the inner class
/// `TimeLimitingKnnCollectorManager.TimeLimitingKnnCollector`, which extends
/// `KnnCollector.Decorator`; Rust has no implementation inheritance, so the
/// decorator's delegation is written out, and the enclosing manager's
/// `queryTimeout` becomes a field rather than a captured outer reference.
pub struct TimeLimitingKnnCollector {
    delegate: Box<dyn KnnCollector>,
    query_timeout: Arc<dyn QueryTimeout>,
}

impl std::fmt::Debug for TimeLimitingKnnCollector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TimeLimitingKnnCollector")
            .field("queryTimeout", &self.query_timeout)
            .finish()
    }
}

impl TimeLimitingKnnCollector {
    /// Wraps `collector`, honouring `query_timeout`.
    pub fn new(collector: Box<dyn KnnCollector>, query_timeout: Arc<dyn QueryTimeout>) -> Self {
        Self {
            delegate: collector,
            query_timeout,
        }
    }
}

impl KnnCollector for TimeLimitingKnnCollector {
    fn early_terminated(&self) -> bool {
        self.query_timeout.should_exit() || self.delegate.early_terminated()
    }

    fn inc_visited_count(&mut self, count: i32) {
        self.delegate.inc_visited_count(count);
    }

    fn visited_count(&self) -> i64 {
        self.delegate.visited_count()
    }

    fn visit_limit(&self) -> i64 {
        self.delegate.visit_limit()
    }

    fn k(&self) -> i32 {
        self.delegate.k()
    }

    fn collect(&mut self, doc_id: i32, similarity: f32) -> bool {
        self.delegate.collect(doc_id, similarity)
    }

    fn min_competitive_similarity(&self) -> f32 {
        self.delegate.min_competitive_similarity()
    }

    fn top_docs(&mut self) -> TopDocs {
        let docs = self.delegate.top_docs();

        // Mark the results as partial if the timeout is met.
        let relation = if self.query_timeout.should_exit() {
            TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO
        } else {
            docs.total_hits.relation()
        };

        TopDocs {
            total_hits: TotalHits::new(docs.total_hits.value(), relation)
                .expect("INVARIANT: a hit count read back from a TopDocs is never negative"),
            score_docs: docs.score_docs,
        }
    }

    fn get_search_strategy(&self) -> Option<&KnnSearchStrategy> {
        self.delegate.get_search_strategy()
    }
}
