//! Byte-vector similarity search, ported from
//! `org.apache.lucene.search.ByteVectorSimilarityQuery`.

#![deny(unsafe_code)]

use std::any::Any;
use std::sync::Arc;

use crate::error::Result;
use crate::index::LeafReaderContext;
use crate::search::abstract_vector_similarity_query::{
    vector_similarity_create_weight, vector_similarity_query_visit, AbstractVectorSimilarityQuery,
    AbstractVectorSimilarityQueryImpl, DEFAULT_DECAY,
};
use crate::search::doc_id_set_iterator::AcceptDocs;
use crate::search::index_searcher::IndexSearcher;
use crate::search::knn::knn_collector_manager::KnnCollectorManager;
use crate::search::knn::{KnnSearchStrategy, TopDocs};
use crate::search::query::Query;
use crate::search::query_visitor::QueryVisitor;
use crate::search::score_mode::ScoreMode;
use crate::search::vector_scorer::VectorScorer;
use crate::search::weight::Weight;

/// Searches for every (approximate) byte vector above a similarity threshold.
///
/// Equivalent to `org.apache.lucene.search.ByteVectorSimilarityQuery`.
#[derive(Debug, Clone)]
pub struct ByteVectorSimilarityQuery {
    base: AbstractVectorSimilarityQuery,
    target: Vec<u8>,
}

impl ByteVectorSimilarityQuery {
    /// Searches with an explicit decay factor, filter and search strategy.
    ///
    /// Equivalent to
    /// `new ByteVectorSimilarityQuery(String, byte[], float, float, Query, KnnSearchStrategy)`.
    /// A `None` strategy selects this query's own default — an `Hnsw` with a
    /// filtered-search threshold of 0, which preserves the query's filter
    /// handling. The underlying format may not support all strategies and is
    /// free to ignore the requested one.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`](crate::error::LuceneError::IllegalArgument)
    /// when `result_similarity` or `decay` is `NaN`, or when `decay` is
    /// outside `[0, 1]`.
    pub fn with_strategy(
        field: impl Into<String>,
        target: Vec<u8>,
        result_similarity: f32,
        decay: f32,
        filter: Option<Arc<dyn Query>>,
        search_strategy: Option<KnnSearchStrategy>,
    ) -> Result<Self> {
        let base = AbstractVectorSimilarityQuery::new(
            field,
            result_similarity,
            decay,
            filter,
            search_strategy,
        )?;
        Ok(Self { base, target })
    }

    /// Searches with an explicit decay factor and filter.
    ///
    /// Equivalent to
    /// `new ByteVectorSimilarityQuery(String, byte[], float, float, Query)`.
    ///
    /// # Errors
    ///
    /// As [`with_strategy`](Self::with_strategy).
    pub fn with_decay(
        field: impl Into<String>,
        target: Vec<u8>,
        result_similarity: f32,
        decay: f32,
        filter: Option<Arc<dyn Query>>,
    ) -> Result<Self> {
        Self::with_strategy(field, target, result_similarity, decay, filter, None)
    }

    /// Searches with the default decay factor and a filter.
    ///
    /// Equivalent to
    /// `new ByteVectorSimilarityQuery(String, byte[], float, Query)`.
    ///
    /// # Errors
    ///
    /// As [`with_strategy`](Self::with_strategy).
    pub fn with_filter(
        field: impl Into<String>,
        target: Vec<u8>,
        result_similarity: f32,
        filter: Option<Arc<dyn Query>>,
    ) -> Result<Self> {
        Self::with_decay(field, target, result_similarity, DEFAULT_DECAY, filter)
    }

    /// Searches with the default decay factor and no filter.
    ///
    /// Equivalent to `new ByteVectorSimilarityQuery(String, byte[], float)`.
    ///
    /// # Errors
    ///
    /// As [`with_strategy`](Self::with_strategy).
    pub fn new(field: impl Into<String>, target: Vec<u8>, result_similarity: f32) -> Result<Self> {
        Self::with_filter(field, target, result_similarity, None)
    }

    /// Returns the search strategy used during graph search.
    ///
    /// Equivalent to the inherited
    /// `AbstractVectorSimilarityQuery.getSearchStrategy()`.
    pub fn get_search_strategy(&self) -> &KnnSearchStrategy {
        self.base.get_search_strategy()
    }
}

impl AbstractVectorSimilarityQueryImpl for ByteVectorSimilarityQuery {
    fn similarity_base(&self) -> &AbstractVectorSimilarityQuery {
        &self.base
    }

    fn as_query(&self) -> &dyn Query {
        self
    }

    fn clone_similarity_query(&self) -> Arc<dyn AbstractVectorSimilarityQueryImpl> {
        Arc::new(self.clone())
    }

    fn clone_as_query(&self) -> Arc<dyn Query> {
        Arc::new(self.clone())
    }

    fn create_vector_scorer(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn VectorScorer>>> {
        let Some(vector_values) = context
            .leaf_reader()
            .get_byte_vector_values(self.base.get_field())?
        else {
            return Ok(None);
        };
        vector_values.scorer(&self.target)
    }

    fn approximate_search(
        &self,
        context: &LeafReaderContext,
        accept_docs: &mut dyn AcceptDocs,
        visit_limit: i32,
        knn_collector_manager: &dyn KnnCollectorManager,
    ) -> Result<TopDocs> {
        let mut collector = knn_collector_manager.new_collector(
            visit_limit,
            Some(self.base.get_search_strategy()),
            context,
        )?;
        context.leaf_reader().search_nearest_vectors_byte(
            self.base.get_field(),
            &self.target,
            collector.as_mut(),
            accept_docs,
        )?;
        Ok(collector.top_docs())
    }
}

impl Query for ByteVectorSimilarityQuery {
    fn to_query_string(&self, field: &str) -> String {
        format!(
            "ByteVectorSimilarityQuery[field={} target=[{:?}...] resultSimilarity={:?} decay={:?} filter={}]",
            field,
            self.target.first().copied().unwrap_or(0),
            self.base.result_similarity(),
            self.base.decay(),
            match self.base.get_filter() {
                None => "null".to_string(),
                Some(filter) => crate::search::query_to_string(filter.as_ref()),
            }
        )
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        vector_similarity_query_visit(&self.base, self, visitor);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        vector_similarity_create_weight(self.clone_similarity_query(), searcher, score_mode, boost)
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        match other.as_any().downcast_ref::<ByteVectorSimilarityQuery>() {
            Some(other) => self.base.base_eq(&other.base) && self.target == other.target,
            None => false,
        }
    }

    fn query_hash(&self) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        self.base.base_hash().hash(&mut hasher);
        self.target.hash(&mut hasher);
        hasher.finish()
    }
}
