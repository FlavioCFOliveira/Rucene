//! Cost-based access-path selection, ported from
//! `org.apache.lucene.search.IndexOrDocValuesQuery`.

#![deny(unsafe_code)]

use std::any::Any;
use std::sync::Arc;

use crate::error::Result;
use crate::index::LeafReaderContext;
use crate::search::boolean_clause::Occur;
use crate::search::bulk_scorer::BulkScorer;
use crate::search::index_searcher::IndexSearcher;
use crate::search::match_all_docs_query::MatchAllDocsQuery;
use crate::search::match_no_docs_query::MatchNoDocsQuery;
use crate::search::matches::Matches;
use crate::search::query::Query;
use crate::search::query_visitor::QueryVisitor;
use crate::search::score_mode::ScoreMode;
use crate::search::scorer::Scorer;
use crate::search::scorer_supplier::ScorerSupplier;
use crate::search::segment_cacheable::SegmentCacheable;
use crate::search::similarities::Explanation;
use crate::search::weight::Weight;

/// A [`Query`] that uses either an index structure — points or terms — or
/// doc values in order to run a query, depending on which is expected to be
/// more efficient.
///
/// Equivalent to `org.apache.lucene.search.IndexOrDocValuesQuery`.
///
/// This is typically useful for range queries, where both access paths are
/// available: the index structure is fast at finding the set of matching
/// documents, while doc values are fast at verifying whether one document
/// matches. So a query that is a required clause of a conjunction with a
/// selective lead clause is better served by doc values, and one that leads
/// iteration is better served by the index structure.
///
/// Note that this query is only useful when the two queries agree on the set of
/// matching documents; otherwise the results are undefined.
#[derive(Debug)]
pub struct IndexOrDocValuesQuery {
    index_query: Arc<dyn Query>,
    dv_query: Arc<dyn Query>,
}

impl IndexOrDocValuesQuery {
    /// Creates a query that chooses between `index_query` and `dv_query`.
    ///
    /// Equivalent to `new IndexOrDocValuesQuery(Query, Query)`.
    ///
    /// * `index_query` — a query that has a good iteration cost but a high
    ///   per-document verification cost, typically a points or terms query;
    /// * `dv_query` — a query that has a high iteration cost but a low
    ///   per-document verification cost, typically a doc-values query.
    pub fn new(index_query: Arc<dyn Query>, dv_query: Arc<dyn Query>) -> Self {
        Self {
            index_query,
            dv_query,
        }
    }

    /// Returns the query that uses the index structure.
    ///
    /// Equivalent to `IndexOrDocValuesQuery.getIndexQuery()`.
    pub fn get_index_query(&self) -> &Arc<dyn Query> {
        &self.index_query
    }

    /// Returns the query that uses doc values.
    ///
    /// Equivalent to `IndexOrDocValuesQuery.getRandomAccessQuery()`.
    pub fn get_random_access_query(&self) -> &Arc<dyn Query> {
        &self.dv_query
    }
}

/// The supplier that picks an access path from the lead cost.
///
/// Equivalent to the anonymous `ScorerSupplier` the weight returns.
struct IndexOrDocValuesScorerSupplier {
    index_scorer_supplier: Box<dyn ScorerSupplier>,
    dv_scorer_supplier: Box<dyn ScorerSupplier>,
}

impl ScorerSupplier for IndexOrDocValuesScorerSupplier {
    fn get(&mut self, lead_cost: i64) -> Result<Box<dyn Scorer>> {
        // At equal costs, doc values tend to be worse than points, since they
        // still need to perform one comparison per document while points can do
        // much better than that given how values are organised. So doc values
        // get an arbitrary 8x penalty.
        let threshold = ((self.cost() as u64) >> 3) as i64;
        if threshold <= lead_cost {
            self.index_scorer_supplier.get(lead_cost)
        } else {
            self.dv_scorer_supplier.get(lead_cost)
        }
    }

    fn bulk_scorer(&mut self) -> Result<Box<dyn BulkScorer>> {
        // Bulk scorers need to consume the entire set of docs, so an index
        // structure should perform better.
        self.index_scorer_supplier.bulk_scorer()
    }

    fn cost(&self) -> i64 {
        self.index_scorer_supplier.cost()
    }
}

/// The weight of an [`IndexOrDocValuesQuery`].
///
/// Equivalent to the anonymous `Weight` that
/// `IndexOrDocValuesQuery.createWeight` returns.
#[derive(Debug)]
struct IndexOrDocValuesWeight {
    query: Arc<dyn Query>,
    index_weight: Arc<dyn Weight>,
    dv_weight: Arc<dyn Weight>,
}

impl SegmentCacheable for IndexOrDocValuesWeight {
    /// Both the index and the doc-values query return the same documents, so
    /// the index query's cache helper can be used.
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.index_weight.is_cacheable(ctx)
    }
}

impl Weight for IndexOrDocValuesWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        Arc::clone(&self.query)
    }

    /// A single document has to be checked, so the doc-values query should
    /// perform better.
    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation> {
        self.dv_weight.explain(context, doc)
    }

    /// A single document has to be checked, so the doc-values query should
    /// perform better.
    fn matches(&self, context: &LeafReaderContext, doc: i32) -> Result<Option<Arc<dyn Matches>>> {
        self.dv_weight.matches(context, doc)
    }

    fn count(&self, context: &LeafReaderContext) -> Result<i32> {
        let count = self.index_weight.count(context)?;
        if count != -1 {
            return Ok(count);
        }
        self.dv_weight.count(context)
    }

    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let index_scorer_supplier = self.index_weight.scorer_supplier(context)?;
        let dv_scorer_supplier = self.dv_weight.scorer_supplier(context)?;
        let (Some(index_scorer_supplier), Some(dv_scorer_supplier)) =
            (index_scorer_supplier, dv_scorer_supplier)
        else {
            return Ok(None);
        };
        Ok(Some(Box::new(IndexOrDocValuesScorerSupplier {
            index_scorer_supplier,
            dv_scorer_supplier,
        })))
    }
}

impl Query for IndexOrDocValuesQuery {
    fn to_query_string(&self, field: &str) -> String {
        format!(
            "IndexOrDocValuesQuery(indexQuery={}, dvQuery={})",
            self.index_query.to_query_string(field),
            self.dv_query.to_query_string(field)
        )
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        let mut sub_visitor = visitor.get_sub_visitor(Occur::MUST, self);
        self.index_query.visit(sub_visitor.as_mut());
        self.dv_query.visit(sub_visitor.as_mut());
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn rewrite(&self, searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        let index_rewrite = self.index_query.rewrite(searcher)?;
        let dv_rewrite = self.dv_query.rewrite(searcher)?;
        let index_effective = index_rewrite
            .clone()
            .unwrap_or_else(|| Arc::clone(&self.index_query));
        let dv_effective = dv_rewrite
            .clone()
            .unwrap_or_else(|| Arc::clone(&self.dv_query));

        if index_effective.as_any().is::<MatchAllDocsQuery>()
            || dv_effective.as_any().is::<MatchAllDocsQuery>()
        {
            return Ok(Some(Arc::new(MatchAllDocsQuery::instance())));
        }
        if index_effective.as_any().is::<MatchNoDocsQuery>()
            || dv_effective.as_any().is::<MatchNoDocsQuery>()
        {
            return Ok(Some(Arc::new(MatchNoDocsQuery::instance())));
        }
        if index_rewrite.is_some() || dv_rewrite.is_some() {
            return Ok(Some(Arc::new(IndexOrDocValuesQuery::new(
                index_effective,
                dv_effective,
            ))));
        }
        Ok(None)
    }

    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        let index_weight = self
            .index_query
            .create_weight(searcher, score_mode, boost)?;
        let dv_weight = self.dv_query.create_weight(searcher, score_mode, boost)?;
        Ok(Arc::new(IndexOrDocValuesWeight {
            query: Arc::new(IndexOrDocValuesQuery::new(
                Arc::clone(&self.index_query),
                Arc::clone(&self.dv_query),
            )),
            index_weight,
            dv_weight,
        }))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        match other.as_any().downcast_ref::<IndexOrDocValuesQuery>() {
            Some(other) => {
                self.index_query.query_eq(&*other.index_query)
                    && self.dv_query.query_eq(&*other.dv_query)
            }
            None => false,
        }
    }

    fn query_hash(&self) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        self.class_hash().hash(&mut hasher);
        self.index_query.query_hash().hash(&mut hasher);
        self.dv_query.query_hash().hash(&mut hasher);
        hasher.finish()
    }
}
