//! Patience-based early-exit nearest-neighbour search, ported from
//! `org.apache.lucene.search.PatienceKnnVectorQuery`.

#![deny(unsafe_code)]

use std::any::Any;
use std::sync::Arc;

use crate::error::Result;
use crate::index::{FieldInfo, LeafReaderContext, QueryTimeout};
use crate::search::abstract_knn_vector_query::{
    knn_vector_query_rewrite, AbstractKnnVectorQuery, AbstractKnnVectorQueryImpl,
};
use crate::search::doc_id_set_iterator::{AcceptDocs, DocIdSetIterator};
use crate::search::hnsw_queue_saturation_collector::HnswQueueSaturationCollector;
use crate::search::index_searcher::IndexSearcher;
use crate::search::knn::knn_collector_manager::KnnCollectorManager;
use crate::search::knn::{KnnCollector, KnnSearchStrategy, TopDocs};
use crate::search::query::Query;
use crate::search::query_visitor::QueryVisitor;
use crate::search::seeded_knn_vector_query::SeededKnnVectorQuery;
use crate::search::vector_scorer::VectorScorer;

/// The default saturation threshold.
///
/// Equivalent to `PatienceKnnVectorQuery.DEFAULT_SATURATION_THRESHOLD`.
const DEFAULT_SATURATION_THRESHOLD: f64 = 0.995;

/// A kNN vector query that exits early when the HNSW queue saturates over a
/// saturation threshold for more than `patience` consecutive candidate visits.
///
/// Equivalent to `org.apache.lucene.search.PatienceKnnVectorQuery`.
///
/// See ["Patience in Proximity: A Simple Early Termination Strategy for HNSW
/// Graph Traversal in Approximate k-Nearest Neighbor
/// Search"](https://cs.uwaterloo.ca/~jimmylin/publications/Teofili_Lin_ECIR2025.pdf)
/// (Teofili and Lin), ECIR '25.
#[derive(Debug, Clone)]
pub struct PatienceKnnVectorQuery {
    base: AbstractKnnVectorQuery,
    patience: i32,
    saturation_threshold: f64,
    delegate: Arc<dyn AbstractKnnVectorQueryImpl>,
}

impl PatienceKnnVectorQuery {
    /// Wraps a kNN query with an explicit saturation threshold and patience.
    ///
    /// Equivalent to the package-private
    /// `PatienceKnnVectorQuery(AbstractKnnVectorQuery, String, int, Query, KnnSearchStrategy, double, int)`
    /// and to the three public constructors, which all read the wrapped query's
    /// own fields.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`](crate::error::LuceneError::IllegalArgument)
    /// when the wrapped query's `k` is less than 1.
    pub fn new(
        knn_query: Arc<dyn AbstractKnnVectorQueryImpl>,
        saturation_threshold: f64,
        patience: i32,
    ) -> Result<Self> {
        let base = knn_query.knn_base().clone();
        Ok(Self {
            base: AbstractKnnVectorQuery::new(
                base.get_field(),
                base.get_k(),
                base.get_filter().cloned(),
                base.get_search_strategy().cloned(),
            )?,
            patience,
            saturation_threshold,
            delegate: knn_query,
        })
    }

    /// Wraps a kNN query with the default saturation threshold and patience.
    ///
    /// Equivalent to the `fromFloatQuery(KnnFloatVectorQuery)`,
    /// `fromByteQuery(KnnByteVectorQuery)` and
    /// `fromSeededQuery(SeededKnnVectorQuery)` factories, which differ only in
    /// the static type they accept.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn from_query(knn_query: Arc<dyn AbstractKnnVectorQueryImpl>) -> Result<Self> {
        let patience = default_patience(knn_query.knn_base());
        Self::new(knn_query, DEFAULT_SATURATION_THRESHOLD, patience)
    }

    /// Returns the wrapped kNN query.
    pub fn delegate(&self) -> &Arc<dyn AbstractKnnVectorQueryImpl> {
        &self.delegate
    }

    /// Returns the patience parameter.
    pub fn patience(&self) -> i32 {
        self.patience
    }

    /// Returns the saturation threshold.
    pub fn saturation_threshold(&self) -> f64 {
        self.saturation_threshold
    }

    /// Returns the kNN vector field the search runs on.
    ///
    /// Equivalent to `PatienceKnnVectorQuery.getField()`, which delegates.
    pub fn get_field(&self) -> &str {
        self.delegate.knn_base().get_field()
    }

    /// Returns the maximum number of results the search returns.
    ///
    /// Equivalent to `PatienceKnnVectorQuery.getK()`, which delegates.
    pub fn get_k(&self) -> i32 {
        self.delegate.knn_base().get_k()
    }

    /// Returns the filter executed before the vector search.
    ///
    /// Equivalent to `PatienceKnnVectorQuery.getFilter()`, which delegates.
    pub fn get_filter(&self) -> Option<&Arc<dyn Query>> {
        self.delegate.knn_base().get_filter()
    }
}

/// Equivalent to the private
/// `PatienceKnnVectorQuery.defaultPatience(AbstractKnnVectorQuery)`.
fn default_patience(delegate: &AbstractKnnVectorQuery) -> i32 {
    7.max((delegate.get_k() as f64 * 0.3) as i32)
}

impl AbstractKnnVectorQueryImpl for PatienceKnnVectorQuery {
    fn knn_base(&self) -> &AbstractKnnVectorQuery {
        &self.base
    }

    fn as_query(&self) -> &dyn Query {
        self
    }

    fn clone_knn_query(&self) -> Arc<dyn AbstractKnnVectorQueryImpl> {
        Arc::new(self.clone())
    }

    fn approximate_search(
        &self,
        context: &LeafReaderContext,
        accept_docs: &mut dyn AcceptDocs,
        visited_limit: i32,
        knn_collector_manager: &dyn KnnCollectorManager,
    ) -> Result<TopDocs> {
        self.delegate
            .approximate_search(context, accept_docs, visited_limit, knn_collector_manager)
    }

    fn create_vector_scorer(
        &self,
        context: &LeafReaderContext,
        field_info: &FieldInfo,
    ) -> Result<Option<Box<dyn VectorScorer>>> {
        self.delegate.create_vector_scorer(context, field_info)
    }

    fn get_knn_collector_manager(
        &self,
        k: i32,
        searcher: &IndexSearcher,
    ) -> Arc<dyn KnnCollectorManager> {
        Arc::new(PatienceCollectorManager {
            knn_collector_manager: self.delegate.get_knn_collector_manager(k, searcher),
            saturation_threshold: self.saturation_threshold,
            patience: self.patience,
        })
    }

    fn exact_search(
        &self,
        context: &LeafReaderContext,
        accept_iterator: Box<dyn DocIdSetIterator>,
        query_timeout: Option<&Arc<dyn QueryTimeout>>,
    ) -> Result<TopDocs> {
        self.delegate
            .exact_search(context, accept_iterator, query_timeout)
    }

    fn merge_leaf_results(&self, per_leaf_results: &[TopDocs]) -> Result<TopDocs> {
        self.delegate.merge_leaf_results(per_leaf_results)
    }
}

impl Query for PatienceKnnVectorQuery {
    fn to_query_string(&self, _field: &str) -> String {
        format!(
            "PatienceKnnVectorQuery{{saturationThreshold={}, patience={}, delegate={}}}",
            self.saturation_threshold,
            self.patience,
            crate::search::query_to_string(self.delegate.as_query())
        )
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        self.delegate.visit(visitor);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn rewrite(&self, index_searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        // **Divergence from Lucene 10.5.0.** Java assigns the re-seeded query
        // back into its own `delegate` field before calling `super.rewrite`,
        // mutating the query in place. `Query::rewrite` only has `&self` in
        // this port — and a query that mutates itself during a search is a
        // hazard rather than a contract — so the rewrite runs against a copy
        // that carries the new delegate. The query the rewrite produces is
        // identical; only the mutation of the receiver, which Java's public API
        // exposes solely through `toString`/`equals`, does not happen.
        let query: Arc<dyn AbstractKnnVectorQueryImpl> = match self
            .delegate
            .as_any()
            .downcast_ref::<SeededKnnVectorQuery>(
        ) {
            None => self.clone_knn_query(),
            Some(seeded) => {
                // This is required because SeededKnnVectorQuery now has its own
                // rewriting logic, to create the seed weight.
                let base = self.delegate.knn_base().clone();
                let reseeded = SeededKnnVectorQuery::new(
                    Arc::clone(seeded.delegate()),
                    Arc::clone(seeded.seed()),
                    Some(seeded.create_seed_weight(index_searcher)?),
                    base.get_field(),
                    base.get_k(),
                    base.get_filter().cloned(),
                    base.get_search_strategy().cloned(),
                )?;
                let mut rewritten = self.clone();
                rewritten.delegate = Arc::new(reseeded);
                Arc::new(rewritten)
            }
        };
        knn_vector_query_rewrite(query, index_searcher)
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        match other.as_any().downcast_ref::<PatienceKnnVectorQuery>() {
            Some(other) => {
                self.base.base_eq(&other.base)
                    && self.saturation_threshold == other.saturation_threshold
                    && self.patience == other.patience
                    && self.delegate.query_eq(other.delegate.as_query())
            }
            None => false,
        }
    }

    fn query_hash(&self) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        self.base.base_hash().hash(&mut hasher);
        self.saturation_threshold.to_bits().hash(&mut hasher);
        self.patience.hash(&mut hasher);
        self.delegate.query_hash().hash(&mut hasher);
        hasher.finish()
    }
}

/// A collector manager that wraps every collector with a patience-based early
/// exit.
///
/// Equivalent to the inner class
/// `PatienceKnnVectorQuery.PatienceCollectorManager`.
#[derive(Debug)]
struct PatienceCollectorManager {
    knn_collector_manager: Arc<dyn KnnCollectorManager>,
    saturation_threshold: f64,
    patience: i32,
}

impl KnnCollectorManager for PatienceCollectorManager {
    fn new_collector(
        &self,
        visit_limit: i32,
        search_strategy: Option<&KnnSearchStrategy>,
        ctx: &LeafReaderContext,
    ) -> Result<Box<dyn KnnCollector>> {
        Ok(Box::new(HnswQueueSaturationCollector::new(
            self.knn_collector_manager
                .new_collector(visit_limit, search_strategy, ctx)?,
            self.saturation_threshold,
            self.patience,
        )))
    }

    fn new_optimistic_collector(
        &self,
        visit_limit: i32,
        search_strategy: Option<&KnnSearchStrategy>,
        ctx: &LeafReaderContext,
        k: i32,
    ) -> Result<Option<Box<dyn KnnCollector>>> {
        if !self.knn_collector_manager.is_optimistic() {
            return Ok(None);
        }
        match self.knn_collector_manager.new_optimistic_collector(
            visit_limit,
            search_strategy,
            ctx,
            k,
        )? {
            None => Ok(None),
            Some(collector) => Ok(Some(Box::new(HnswQueueSaturationCollector::new(
                collector,
                self.saturation_threshold,
                self.patience,
            )))),
        }
    }

    fn is_optimistic(&self) -> bool {
        self.knn_collector_manager.is_optimistic()
    }
}
