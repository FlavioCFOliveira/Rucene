//! The Indri conjunction query, ported from
//! `org.apache.lucene.search.IndriAndQuery`.

#![deny(unsafe_code)]

use std::any::Any;
use std::sync::Arc;

use crate::error::Result;
use crate::search::boolean_clause::BooleanClause;
use crate::search::index_searcher::IndexSearcher;
use crate::search::indri_and_weight::IndriAndWeight;
use crate::search::indri_query::IndriQuery;
use crate::search::query::Query;
use crate::search::query_visitor::QueryVisitor;
use crate::search::score_mode::ScoreMode;
use crate::search::weight::Weight;

/// A query that matches documents matching combinations of sub-queries,
/// combining their scores the Indri way.
///
/// Equivalent to `org.apache.lucene.search.IndriAndQuery`, which extends
/// [`IndriQuery`]; the base class's state and concrete methods live in an
/// [`IndriQuery`] field here.
#[derive(Debug, Clone)]
pub struct IndriAndQuery {
    base: IndriQuery,
}

impl IndriAndQuery {
    /// Creates a query over the given clauses.
    ///
    /// Equivalent to `new IndriAndQuery(List<BooleanClause>)`.
    pub fn new(clauses: Vec<BooleanClause>) -> Self {
        Self {
            base: IndriQuery::new(clauses),
        }
    }

    /// Returns the clauses of this query.
    ///
    /// Equivalent to the inherited `IndriQuery.getClauses()`.
    pub fn get_clauses(&self) -> &[BooleanClause] {
        self.base.get_clauses()
    }

    /// Iterates the clauses of this query.
    ///
    /// Equivalent to the inherited `IndriQuery.iterator()`.
    pub fn iter(&self) -> std::slice::Iter<'_, BooleanClause> {
        self.base.iter()
    }
}

impl Query for IndriAndQuery {
    fn to_query_string(&self, field: &str) -> String {
        self.base.to_query_string(field)
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        self.base.visit(self, visitor);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        _score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        Ok(Arc::new(IndriAndWeight::new(
            Arc::new(self.clone()),
            self.base.clone(),
            searcher,
            ScoreMode::TOP_SCORES,
            boost,
        )?))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        match other.as_any().downcast_ref::<IndriAndQuery>() {
            Some(other) => self.base.base_eq(&other.base),
            None => false,
        }
    }

    fn query_hash(&self) -> u64 {
        self.base.base_hash()
    }
}
