//! Float-vector nearest-neighbour search, ported from
//! `org.apache.lucene.search.KnnFloatVectorQuery`.

#![deny(unsafe_code)]

use std::any::Any;
use std::sync::Arc;

use crate::error::Result;
use crate::index::{check_float_field, FieldInfo, LeafReaderContext};
use crate::search::abstract_knn_vector_query::{
    knn_vector_query_rewrite, knn_vector_query_visit, AbstractKnnVectorQuery,
    AbstractKnnVectorQueryImpl,
};
use crate::search::doc_id_set_iterator::AcceptDocs;
use crate::search::index_searcher::IndexSearcher;
use crate::search::knn::knn_collector_manager::KnnCollectorManager;
use crate::search::knn::{KnnSearchStrategy, TopDocs};
use crate::search::query::Query;
use crate::search::query_visitor::QueryVisitor;
use crate::search::top_docs_collector::empty_top_docs;
use crate::search::vector_scorer::VectorScorer;
use crate::util::vector_util;

/// Finds the `k` nearest documents to a target float vector.
///
/// Equivalent to `org.apache.lucene.search.KnnFloatVectorQuery`.
///
/// The query also allows a kNN search subject to a filter. In that case it
/// first executes the filter for each leaf, then chooses a strategy
/// dynamically:
///
/// * if the filter cost is less than `k`, run an exact search;
/// * otherwise run a kNN search subject to the filter;
/// * if the kNN search visits too many vectors without completing, stop and run
///   an exact search.
#[derive(Debug, Clone)]
pub struct KnnFloatVectorQuery {
    base: AbstractKnnVectorQuery,
    /// The target of the search.
    ///
    /// Equivalent to the `protected final float[] target` field.
    target: Vec<f32>,
}

impl KnnFloatVectorQuery {
    /// Finds the `k` nearest documents to `target`, with no filter and the
    /// default search strategy.
    ///
    /// Equivalent to `new KnnFloatVectorQuery(String, float[], int)`.
    ///
    /// # Errors
    ///
    /// As [`with_strategy`](Self::with_strategy).
    pub fn new(field: impl Into<String>, target: Vec<f32>, k: i32) -> Result<Self> {
        Self::with_filter(field, target, k, None)
    }

    /// Finds the `k` nearest documents to `target` among those a filter
    /// accepts.
    ///
    /// Equivalent to `new KnnFloatVectorQuery(String, float[], int, Query)`,
    /// which passes `KnnSearchStrategy.Hnsw.DEFAULT`.
    ///
    /// # Errors
    ///
    /// As [`with_strategy`](Self::with_strategy).
    pub fn with_filter(
        field: impl Into<String>,
        target: Vec<f32>,
        k: i32,
        filter: Option<Arc<dyn Query>>,
    ) -> Result<Self> {
        Self::with_strategy(
            field,
            target,
            k,
            filter,
            Some(KnnSearchStrategy::hnsw_default()),
        )
    }

    /// Finds the `k` nearest documents to `target` with an explicit search
    /// strategy.
    ///
    /// Equivalent to
    /// `new KnnFloatVectorQuery(String, float[], int, Query, KnnSearchStrategy)`.
    /// The underlying format may not support all strategies and is free to
    /// ignore the requested one.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`](crate::error::LuceneError::IllegalArgument)
    /// when `k` is less than 1, or when `target` holds a non-finite value —
    /// which is `VectorUtil.checkFinite`.
    pub fn with_strategy(
        field: impl Into<String>,
        target: Vec<f32>,
        k: i32,
        filter: Option<Arc<dyn Query>>,
        search_strategy: Option<KnnSearchStrategy>,
    ) -> Result<Self> {
        let base = AbstractKnnVectorQuery::new(field, k, filter, search_strategy)?;
        vector_util::check_finite(&target)?;
        Ok(Self { base, target })
    }

    /// Returns a copy of the target query vector.
    ///
    /// Equivalent to `KnnFloatVectorQuery.getTargetCopy()`.
    pub fn get_target_copy(&self) -> Vec<f32> {
        self.target.clone()
    }

    /// Returns the kNN vector field the search runs on.
    ///
    /// Equivalent to the inherited `AbstractKnnVectorQuery.getField()`.
    pub fn get_field(&self) -> &str {
        self.base.get_field()
    }

    /// Returns the maximum number of results the search returns.
    ///
    /// Equivalent to the inherited `AbstractKnnVectorQuery.getK()`.
    pub fn get_k(&self) -> i32 {
        self.base.get_k()
    }

    /// Returns the filter executed before the vector search.
    ///
    /// Equivalent to the inherited `AbstractKnnVectorQuery.getFilter()`.
    pub fn get_filter(&self) -> Option<&Arc<dyn Query>> {
        self.base.get_filter()
    }

    /// Returns the search strategy.
    ///
    /// Equivalent to the inherited `AbstractKnnVectorQuery.getSearchStrategy()`.
    pub fn get_search_strategy(&self) -> Option<&KnnSearchStrategy> {
        self.base.get_search_strategy()
    }
}

impl AbstractKnnVectorQueryImpl for KnnFloatVectorQuery {
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
        let mut knn_collector = knn_collector_manager.new_collector(
            visited_limit,
            self.base.get_search_strategy(),
            context,
        )?;
        let reader = context.leaf_reader();
        let Some(float_vector_values) = reader.get_float_vector_values(self.base.get_field())?
        else {
            check_float_field(reader.as_ref(), self.base.get_field())?;
            return Ok(empty_top_docs());
        };
        if knn_collector.k().min(float_vector_values.size()) == 0 {
            return Ok(empty_top_docs());
        }
        reader.search_nearest_vectors(
            self.base.get_field(),
            &self.target,
            knn_collector.as_mut(),
            accept_docs,
        )?;
        Ok(knn_collector.top_docs())
    }

    fn create_vector_scorer(
        &self,
        context: &LeafReaderContext,
        _field_info: &FieldInfo,
    ) -> Result<Option<Box<dyn VectorScorer>>> {
        let reader = context.leaf_reader();
        let Some(vector_values) = reader.get_float_vector_values(self.base.get_field())? else {
            check_float_field(reader.as_ref(), self.base.get_field())?;
            return Ok(None);
        };
        vector_values.scorer(&self.target)
    }
}

impl Query for KnnFloatVectorQuery {
    fn to_query_string(&self, _field: &str) -> String {
        let mut buffer = String::from("KnnFloatVectorQuery:");
        buffer.push_str(&format!(
            "{}[{},...]",
            self.base.get_field(),
            self.target.first().copied().unwrap_or(0.0)
        ));
        buffer.push_str(&format!("[{}]", self.base.get_k()));
        if let Some(filter) = self.base.get_filter() {
            buffer.push_str(&format!(
                "[{}]",
                crate::search::query_to_string(filter.as_ref())
            ));
        }
        buffer
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        knn_vector_query_visit(&self.base, self, visitor);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn rewrite(&self, searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        knn_vector_query_rewrite(self.clone_knn_query(), searcher)
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        match other.as_any().downcast_ref::<KnnFloatVectorQuery>() {
            Some(other) => self.base.base_eq(&other.base) && self.target == other.target,
            None => false,
        }
    }

    fn query_hash(&self) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        self.base.base_hash().hash(&mut hasher);
        for value in &self.target {
            value.to_bits().hash(&mut hasher);
        }
        hasher.finish()
    }
}
