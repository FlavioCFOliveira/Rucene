//! Port of `lucene/core/src/test/org/apache/lucene/search/ReadAheadMatchAllDocsQuery.java`.
//!
//! Query that matches all docs by returning a DenseConjunctionBulkScorer over
//! a single clause. This helps make sure that the competitive iterators
//! produced by TopFieldCollector are compatible with the read-ahead of this
//! bulk scorer.

use std::any::Any;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::bulk_scorer::BulkScorer;
use rucene::search::constant_score_scorer::ConstantScoreScorer;
use rucene::search::constant_score_weight::{ConstantScoreWeight, ConstantScoreWeightImpl};
use rucene::search::dense_conjunction_bulk_scorer::DenseConjunctionBulkScorer;
use rucene::search::doc_id_set_iterator as disi;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::scorer::Scorer;
use rucene::search::scorer_supplier::ScorerSupplier;
use rucene::search::weight::Weight;
use rucene::search::{ConjunctionMember, Query, ScoreMode};

/// `public final class ReadAheadMatchAllDocsQuery extends Query`.
#[derive(Debug, Clone, Default)]
pub struct ReadAheadMatchAllDocsQuery;

impl ReadAheadMatchAllDocsQuery {
    /// Sole constructor
    pub fn new() -> Self {
        ReadAheadMatchAllDocsQuery
    }
}

/// The anonymous `ConstantScoreWeight` of `createWeight`, which captures the
/// `scoreMode`.
#[derive(Debug)]
struct ReadAheadWeight {
    score_mode: ScoreMode,
}

impl ConstantScoreWeightImpl for ReadAheadWeight {
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        true
    }

    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
        score: f32,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        // Don't use ConstantScoreScoreSupplier, which only uses
        // DenseConjunctionBulkScorer on larg-ish segments.
        Ok(Some(Box::new(ReadAheadScorerSupplier {
            max_doc: context.leaf_reader().max_doc(),
            score,
            score_mode: self.score_mode,
        })))
    }
}

/// The anonymous `ScorerSupplier` of `scorerSupplier`.
struct ReadAheadScorerSupplier {
    /// `context.reader().maxDoc()`.
    max_doc: i32,
    /// `score()`.
    score: f32,
    score_mode: ScoreMode,
}

impl ScorerSupplier for ReadAheadScorerSupplier {
    fn get(&mut self, _lead_cost: i64) -> Result<Box<dyn Scorer>> {
        Ok(Box::new(ConstantScoreScorer::from_iterator(
            self.score,
            self.score_mode,
            Box::new(disi::all(self.max_doc)?),
        )))
    }

    fn bulk_scorer(&mut self) -> Result<Box<dyn BulkScorer>> {
        let clauses = vec![ConjunctionMember::from_iterator(Box::new(disi::all(
            self.max_doc,
        )?))];
        Ok(Box::new(DenseConjunctionBulkScorer::new(
            clauses,
            self.max_doc,
            self.score,
        )?))
    }

    fn cost(&self) -> i64 {
        i64::from(self.max_doc)
    }
}

impl Query for ReadAheadMatchAllDocsQuery {
    fn to_query_string(&self, _field: &str) -> String {
        "ReadAheadMatchAllDocsQuery".to_string()
    }

    fn visit(&self, _visitor: &mut dyn QueryVisitor) {
        // no-op
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
    }

    fn query_hash(&self) -> i32 {
        self.class_hash()
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
            Arc::new(self.clone()),
            boost,
            ReadAheadWeight { score_mode },
        )))
    }
}
