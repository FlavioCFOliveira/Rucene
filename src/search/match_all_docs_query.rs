//! Matching every document, ported from
//! `org.apache.lucene.search.MatchAllDocsQuery`.

#![deny(unsafe_code)]

use std::any::Any;
use std::sync::Arc;

use crate::error::Result;
use crate::index::LeafReaderContext;
use crate::search::constant_score_scorer_supplier::ConstantScoreScorerSupplier;
use crate::search::constant_score_weight::{ConstantScoreWeight, ConstantScoreWeightImpl};
use crate::search::index_searcher::IndexSearcher;
use crate::search::query::Query;
use crate::search::query_visitor::QueryVisitor;
use crate::search::score_mode::ScoreMode;
use crate::search::scorer_supplier::ScorerSupplier;
use crate::search::weight::Weight;

/// A query that matches all documents.
///
/// Equivalent to the `final org.apache.lucene.search.MatchAllDocsQuery`.
///
/// **Divergence from Lucene 10.5.0.** Java exposes a singleton `INSTANCE` and a
/// deprecated public constructor. Rust needs no singleton for a unit struct —
/// every value is the same value — so `MatchAllDocsQuery` itself plays that
/// role and [`instance`](Self::instance) is provided for call sites that read
/// like Java's.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MatchAllDocsQuery;

impl MatchAllDocsQuery {
    /// Returns the query.
    ///
    /// Equivalent to the `MatchAllDocsQuery.INSTANCE` constant.
    pub fn instance() -> Self {
        Self
    }
}

/// The leaf-level behaviour of the weight [`MatchAllDocsQuery`] creates.
///
/// Equivalent to the anonymous `ConstantScoreWeight` subclass in
/// `MatchAllDocsQuery.createWeight`.
///
/// **Divergence from Lucene 10.5.0.** Java's anonymous class reads the enclosing
/// weight's `score()` and the captured `scoreMode`. This port's
/// [`ConstantScoreWeightImpl`] is a separate object, so it carries both itself;
/// the score is the same value the surrounding [`ConstantScoreWeight`] holds.
#[derive(Debug)]
struct MatchAllDocsWeight {
    score: f32,
    score_mode: ScoreMode,
}

impl ConstantScoreWeightImpl for MatchAllDocsWeight {
    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let max_doc = context.leaf_reader().max_doc();
        Ok(Some(Box::new(ConstantScoreScorerSupplier::match_all(
            self.score,
            self.score_mode,
            max_doc,
        )?)))
    }

    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        true
    }

    fn count(&self, context: &LeafReaderContext) -> Result<i32> {
        Ok(context.leaf_reader().num_docs())
    }
}

impl Query for MatchAllDocsQuery {
    fn to_query_string(&self, _field: &str) -> String {
        "*:*".to_string()
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        visitor.visit_leaf(self);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn create_weight(
        &self,
        _searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        Ok(Arc::new(ConstantScoreWeight::new(
            Arc::new(*self),
            boost,
            MatchAllDocsWeight {
                score: boost,
                score_mode,
            },
        )))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
    }

    fn query_hash(&self) -> u64 {
        self.class_hash()
    }
}
