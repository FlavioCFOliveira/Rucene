//! Rewrite-time rescoring, ported from
//! `org.apache.lucene.search.RescoreTopNQuery`.

#![deny(unsafe_code)]

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::VectorSimilarityFunction;
use crate::search::doc_and_score_query::DocAndScoreQuery;
use crate::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use crate::search::double_values::DoubleValues;
use crate::search::double_values_source::DoubleValuesSource;
use crate::search::full_precision_float_vector_similarity_values_source::FullPrecisionFloatVectorSimilarityValuesSource;
use crate::search::hit_queue::HitQueue;
use crate::search::index_searcher::IndexSearcher;
use crate::search::late_interaction_float_values_source::LateInteractionFloatValuesSource;
use crate::search::query::Query;
use crate::search::query_visitor::QueryVisitor;
use crate::search::score_doc::ScoreDoc;
use crate::search::score_mode::ScoreMode;
use crate::search::scorer::Scorer;
use crate::search::top_docs::TopDocs;
use crate::search::total_hits::{TotalHits, TotalHitsRelation};

/// A query that re-scores another query with a [`DoubleValuesSource`] and cuts
/// the results off at the top `n`.
///
/// Equivalent to `org.apache.lucene.search.RescoreTopNQuery`. Unlike
/// [`Rescorer`](crate::search::Rescorer), which rescores after collection, this
/// query rescores during `rewrite`, so that it is compatible with the kNN
/// vector queries — whose results are collected up front — while still working
/// with any query. Unlike a function-score query, it works even with the
/// non-scoring [`ScoreMode`]s.
#[derive(Debug, Clone)]
pub struct RescoreTopNQuery {
    n: i32,
    query: Arc<dyn Query>,
    values_source: Arc<dyn DoubleValuesSource>,
}

impl RescoreTopNQuery {
    /// Executes the inner query, re-scores with the given value source and
    /// trims the result down to `n`.
    ///
    /// Equivalent to
    /// `new RescoreTopNQuery(Query, DoubleValuesSource, int)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] — with Java's message — when
    /// `n` is less than 1.
    pub fn new(
        query: Arc<dyn Query>,
        values_source: Arc<dyn DoubleValuesSource>,
        n: i32,
    ) -> Result<Self> {
        if n < 1 {
            return Err(LuceneError::IllegalArgument("n must be >= 1".to_string()));
        }
        Ok(Self {
            n,
            query,
            values_source,
        })
    }

    /// Creates a query that rescores with full-precision vectors.
    ///
    /// Equivalent to
    /// `RescoreTopNQuery.createFullPrecisionRescorerQuery(Query, float[], String, int)`.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn create_full_precision_rescorer_query(
        inner: Arc<dyn Query>,
        target_vector: Vec<f32>,
        field: impl Into<String>,
        n: i32,
    ) -> Result<Arc<dyn Query>> {
        let values_source: Arc<dyn DoubleValuesSource> = Arc::new(
            FullPrecisionFloatVectorSimilarityValuesSource::with_field_similarity(
                target_vector,
                field,
            ),
        );
        Ok(Arc::new(RescoreTopNQuery::new(inner, values_source, n)?))
    }

    /// Creates a query that computes the top `n` results with multi-vector
    /// similarity comparisons against a late-interaction field.
    ///
    /// Equivalent to
    /// `RescoreTopNQuery.createLateInteractionQuery(Query, int, String, float[][], VectorSimilarityFunction)`.
    ///
    /// This computes the late-interaction similarity for the whole match set of
    /// the wrapped query and returns a query whose match set is only the top-N
    /// hits, which is typically useful for combining a query's results with
    /// other queries in hybrid search. To simply rerank the top-N hits without
    /// scoring the whole match set, see
    /// [`LateInteractionRescorer`](crate::search::LateInteractionRescorer).
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new), plus the validation
    /// [`LateInteractionFloatValuesSource::with_similarity`] performs.
    pub fn create_late_interaction_query(
        inner: Arc<dyn Query>,
        n: i32,
        field_name: impl Into<String>,
        query_vector: Vec<Vec<f32>>,
        vector_similarity_function: VectorSimilarityFunction,
    ) -> Result<Arc<dyn Query>> {
        let values_source: Arc<dyn DoubleValuesSource> =
            Arc::new(LateInteractionFloatValuesSource::with_similarity(
                field_name,
                query_vector,
                vector_similarity_function,
            )?);
        Ok(Arc::new(RescoreTopNQuery::new(inner, values_source, n)?))
    }

    /// Returns the number of hits this query keeps.
    pub fn n(&self) -> i32 {
        self.n
    }

    /// Returns the query executed as the initial phase.
    pub fn query(&self) -> &Arc<dyn Query> {
        &self.query
    }

    /// Returns the source the hits are rescored with.
    pub fn values_source(&self) -> &Arc<dyn DoubleValuesSource> {
        &self.values_source
    }
}

impl Query for RescoreTopNQuery {
    fn to_query_string(&self, field: &str) -> String {
        format!(
            "RescoreTopNQuery:{}:{}[{}]",
            self.query.to_query_string(field),
            self.values_source.to_source_string(),
            self.n
        )
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        self.query.visit(visitor);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn rewrite(&self, index_searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        let rewritten_value_source = Arc::clone(&self.values_source).rewrite(index_searcher)?;
        let rewritten = index_searcher.rewrite(Arc::clone(&self.query))?;
        let weight = index_searcher.create_weight(rewritten, ScoreMode::COMPLETE_NO_SCORES, 1.0)?;
        let mut queue = HitQueue::new(self.n.max(0) as usize, false)?;
        let mut original_count = 0i64;
        for leaf in index_searcher.get_leaf_contexts() {
            let Some(inner_scorer) = weight.scorer(leaf)? else {
                continue;
            };
            let shared = Rc::new(RefCell::new(inner_scorer));
            // If the value source does not need the document score to compute
            // its value, Java passes `null` here.
            let scores: Option<Box<dyn DoubleValues>> = if self.values_source.needs_scores() {
                Some(Box::new(SharedScorerValues {
                    scorer: Rc::clone(&shared),
                }))
            } else {
                None
            };
            let mut rescores = Arc::clone(&rewritten_value_source).get_values(leaf, scores)?;
            let mut iterator = SharedScorerIterator {
                scorer: Rc::clone(&shared),
            };
            while iterator.next_doc()? != NO_MORE_DOCS {
                let doc_id = iterator.doc_id();
                let score = if rescores.advance_exact(doc_id)? {
                    rescores.double_value()? as f32
                } else {
                    0.0
                };
                queue.insert_with_overflow(ScoreDoc::new(leaf.doc_base() + doc_id, score));
                original_count += 1;
            }
        }

        // Java iterates the priority queue in heap-array order, whose first
        // element is the queue's minimum; popping yields the same first element
        // — which is the only position `createDocAndScoreQuery` reads before it
        // sorts the array by doc ID.
        let mut score_docs = Vec::with_capacity(queue.size());
        while let Some(hit) = queue.pop() {
            score_docs.push(hit);
        }
        let top_docs = TopDocs::new(
            TotalHits::new(original_count, TotalHitsRelation::EQUAL_TO)?,
            score_docs,
        );
        Ok(Some(DocAndScoreQuery::create_doc_and_score_query(
            index_searcher,
            top_docs,
        )?))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        match other.as_any().downcast_ref::<RescoreTopNQuery>() {
            Some(other) => {
                self.query.query_eq(other.query.as_ref())
                    && self.values_source.source_eq(other.values_source.as_ref())
                    && self.n == other.n
            }
            None => false,
        }
    }

    fn query_hash(&self) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        self.values_source.source_hash().hash(&mut hasher);
        self.query.query_hash().hash(&mut hasher);
        self.n.hash(&mut hasher);
        hasher.finish()
    }
}

/// Reads the score of a shared [`Scorer`].
///
/// Equivalent to what `DoubleValuesSource.fromScorer(Scorer)` returns for the
/// inner scorer of `RescoreTopNQuery.rewrite`.
///
/// **Divergence from Lucene 10.5.0.** Java hands the same `Scorer` to the value
/// source and to the iteration loop. Rust forbids that alias, so the scorer
/// lives behind an `Rc<RefCell<_>>` that both halves share — the same shape
/// [`SharedVectorScorer`](crate::search::SharedVectorScorer) uses.
struct SharedScorerValues {
    scorer: Rc<RefCell<Box<dyn Scorer>>>,
}

impl DoubleValues for SharedScorerValues {
    fn double_value(&mut self) -> Result<f64> {
        Ok(f64::from(self.scorer.borrow_mut().score()?))
    }

    fn advance_exact(&mut self, _doc: i32) -> Result<bool> {
        Ok(true)
    }
}

/// Iterates a shared [`Scorer`]; see [`SharedScorerValues`].
struct SharedScorerIterator {
    scorer: Rc<RefCell<Box<dyn Scorer>>>,
}

impl DocIdSetIterator for SharedScorerIterator {
    fn doc_id(&self) -> i32 {
        self.scorer.borrow().doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.scorer.borrow_mut().iterator().next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.scorer.borrow_mut().iterator().advance(target)
    }

    fn cost(&self) -> i64 {
        self.scorer.borrow_mut().iterator().cost()
    }
}
