//! Boosting a wrapped query, ported from
//! `org.apache.lucene.search.BoostQuery`.

#![deny(unsafe_code)]

use std::any::Any;
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::search::boolean_clause::Occur;
use crate::search::constant_score_query::ConstantScoreQuery;
use crate::search::constant_score_weight::java_float_to_string;
use crate::search::index_searcher::IndexSearcher;
use crate::search::match_no_docs_query::MatchNoDocsQuery;
use crate::search::query::Query;
use crate::search::query_visitor::QueryVisitor;
use crate::search::score_mode::ScoreMode;
use crate::search::weight::Weight;

/// A [`Query`] wrapper that boosts the wrapped query.
///
/// Equivalent to the `final org.apache.lucene.search.BoostQuery`. Boost values
/// below one give this query less importance than the others, values above one
/// give the scores it returns more importance.
///
/// More complex boosts can be applied with a function-score query, which lives
/// in Lucene's `queries` module.
#[derive(Debug, Clone)]
pub struct BoostQuery {
    query: Arc<dyn Query>,
    boost: f32,
}

impl BoostQuery {
    /// Wraps `query` so that the scores it produces are boosted by `boost`.
    ///
    /// Equivalent to the sole `BoostQuery(Query, float)` constructor.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `boost` is not a finite,
    /// non-negative float, which is the `IllegalArgumentException` Java throws.
    pub fn new(query: Arc<dyn Query>, boost: f32) -> Result<Self> {
        if !boost.is_finite() || boost < 0.0 {
            return Err(LuceneError::IllegalArgument(format!(
                "boost must be a positive float, got {}",
                java_float_to_string(boost)
            )));
        }
        Ok(Self { query, boost })
    }

    /// Returns the wrapped query.
    ///
    /// Equivalent to `BoostQuery.getQuery()`.
    pub fn get_query(&self) -> Arc<dyn Query> {
        Arc::clone(&self.query)
    }

    /// Returns the applied boost.
    ///
    /// Equivalent to `BoostQuery.getBoost()`.
    pub fn get_boost(&self) -> f32 {
        self.boost
    }
}

impl Query for BoostQuery {
    fn to_query_string(&self, field: &str) -> String {
        format!(
            "({}){}{}",
            self.query.to_query_string(field),
            "^",
            java_float_to_string(self.boost)
        )
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        let mut sub = visitor.get_sub_visitor(Occur::MUST, self);
        self.query.visit(&mut *sub);
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
        self.query
            .create_weight(searcher, score_mode, self.boost * boost)
    }

    fn rewrite(&self, searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        let rewritten_inner = self.query.rewrite(searcher)?;
        let changed = rewritten_inner.is_some();
        let rewritten = rewritten_inner.unwrap_or_else(|| Arc::clone(&self.query));

        if self.boost == 1.0 {
            return Ok(Some(rewritten));
        }

        if let Some(inner) = rewritten.as_any().downcast_ref::<BoostQuery>() {
            return Ok(Some(Arc::new(BoostQuery::new(
                inner.get_query(),
                self.boost * inner.boost,
            )?)));
        }

        if rewritten.as_any().is::<MatchNoDocsQuery>() {
            // bubble up MatchNoDocsQuery
            return Ok(Some(rewritten));
        }

        if self.boost == 0.0 && !rewritten.as_any().is::<ConstantScoreQuery>() {
            // so that we pass needScores=false
            return Ok(Some(Arc::new(BoostQuery::new(
                Arc::new(ConstantScoreQuery::new(rewritten)),
                0.0,
            )?)));
        }

        if changed {
            return Ok(Some(Arc::new(BoostQuery::new(rewritten, self.boost)?)));
        }

        Ok(None)
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        if !self.same_class_as(other) {
            return false;
        }
        let Some(other) = other.as_any().downcast_ref::<BoostQuery>() else {
            return false;
        };
        self.query.query_eq(&*other.query) && self.boost.to_bits() == other.boost.to_bits()
    }

    fn query_hash(&self) -> u64 {
        let mut h = self.class_hash();
        h = h.wrapping_mul(31).wrapping_add(self.query.query_hash());
        h = h
            .wrapping_mul(31)
            .wrapping_add(u64::from(self.boost.to_bits()));
        h
    }
}
