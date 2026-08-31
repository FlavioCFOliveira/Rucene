//! Stripping scores from a wrapped query, ported from
//! `org.apache.lucene.search.ConstantScoreQuery`.

#![deny(unsafe_code)]

use std::any::Any;
use std::sync::Arc;

use crate::error::Result;
use crate::index::LeafReaderContext;
use crate::search::boolean_clause::Occur;
use crate::search::boolean_query::BooleanQuery;
use crate::search::boost_query::BoostQuery;
use crate::search::bulk_scorer::{BulkScorer, DefaultBulkScorer};
use crate::search::collection_terminated_exception::CollectionResult;
use crate::search::collector::LeafCollector;
use crate::search::constant_score_scorer::ConstantScoreScorer;
use crate::search::constant_score_weight::{ConstantScoreWeight, ConstantScoreWeightImpl};
use crate::search::index_searcher::IndexSearcher;
use crate::search::match_no_docs_query::MatchNoDocsQuery;
use crate::search::matches::Matches;
use crate::search::query::Query;
use crate::search::query_visitor::QueryVisitor;
use crate::search::scorable::{ChildScorable, Scorable};
use crate::search::score_mode::ScoreMode;
use crate::search::scorer::{into_scorer_iterator, Scorer};
use crate::search::scorer_supplier::ScorerSupplier;
use crate::search::two_phase_iterator::ScorerIterator;
use crate::search::weight::Weight;
use crate::util::Bits;

/// A query that wraps another query and returns a constant score of one for
/// every document that matches it.
///
/// Equivalent to the `final org.apache.lucene.search.ConstantScoreQuery`. It
/// strips off all scores and always returns one.
#[derive(Debug, Clone)]
pub struct ConstantScoreQuery {
    query: Arc<dyn Query>,
}

impl ConstantScoreQuery {
    /// Strips off the scores of the given query.
    ///
    /// Equivalent to `new ConstantScoreQuery(Query)`; Java's
    /// `Objects.requireNonNull` is unnecessary because an `Arc` cannot be null.
    pub fn new(query: Arc<dyn Query>) -> Self {
        Self { query }
    }

    /// Returns the encapsulated query.
    ///
    /// Equivalent to `ConstantScoreQuery.getQuery()`.
    pub fn get_query(&self) -> Arc<dyn Query> {
        Arc::clone(&self.query)
    }
}

/// A [`Scorable`] that replaces the score of the one it wraps with a constant.
///
/// Equivalent to the anonymous `FilterScorable` that
/// `ConstantScoreQuery.ConstantBulkScorer.wrapCollector` installs; as in Java it
/// overrides `score()` only, so `setMinCompetitiveScore` stays the no-op
/// `Scorable` declares.
struct ConstantScorable<'a> {
    inner: &'a mut dyn Scorable,
    score: f32,
}

impl Scorable for ConstantScorable<'_> {
    fn score(&mut self) -> Result<f32> {
        Ok(self.score)
    }

    fn children(&mut self) -> Result<Vec<ChildScorable<'_>>> {
        // Inherited from `FilterScorable`.
        Ok(vec![ChildScorable::new(&mut *self.inner, "FILTER")])
    }
}

/// The leaf collector a [`ConstantBulkScorer`] installs.
///
/// Equivalent to the anonymous `FilterLeafCollector` in
/// `ConstantScoreQuery.ConstantBulkScorer.wrapCollector`.
///
/// **Divergence from Lucene 10.5.0.** Java wraps the scorable once, in
/// `setScorer`, because the wrapped collector stores it. This port passes the
/// scorable to every collection call, so the wrapper is rebuilt around each —
/// see the [collector module documentation](crate::search::collector). As in
/// Java, the bulk collection paths keep [`LeafCollector`]'s defaults, which
/// route through [`collect`](LeafCollector::collect) and therefore also see the
/// constant score.
struct ConstantScoreLeafCollector<'a> {
    inner: &'a mut dyn LeafCollector,
    the_score: f32,
}

impl LeafCollector for ConstantScoreLeafCollector<'_> {
    fn set_scorer(&mut self, scorer: &mut dyn Scorable) -> Result<()> {
        // we must wrap again here, but using the scorer passed in as parameter
        let mut wrapped = ConstantScorable {
            inner: scorer,
            score: self.the_score,
        };
        self.inner.set_scorer(&mut wrapped)
    }

    fn collect(&mut self, doc: i32, scorer: &mut dyn Scorable) -> CollectionResult<()> {
        let mut wrapped = ConstantScorable {
            inner: scorer,
            score: self.the_score,
        };
        self.inner.collect(doc, &mut wrapped)
    }

    fn finish(&mut self) -> Result<()> {
        self.inner.finish()
    }
}

/// The bulk scorer a [`ConstantScoreQuery`] returns, so that a wrapped query
/// with its own optimised top-level scorer can still be used.
///
/// Equivalent to the `protected static
/// ConstantScoreQuery.ConstantBulkScorer`.
///
/// **Divergence from Lucene 10.5.0.** Java's class also stores the inner
/// `Weight`; the field is never read, so this port does not carry it.
pub struct ConstantBulkScorer {
    bulk_scorer: Box<dyn BulkScorer>,
    the_score: f32,
}

impl ConstantBulkScorer {
    /// Wraps the given bulk scorer.
    ///
    /// Equivalent to `new ConstantBulkScorer(BulkScorer, Weight, float)`.
    pub fn new(bulk_scorer: Box<dyn BulkScorer>, the_score: f32) -> Self {
        Self {
            bulk_scorer,
            the_score,
        }
    }
}

impl BulkScorer for ConstantBulkScorer {
    fn score(
        &mut self,
        collector: &mut dyn LeafCollector,
        accept_docs: Option<&dyn Bits>,
        min: i32,
        max: i32,
    ) -> CollectionResult<i32> {
        let mut wrapped = ConstantScoreLeafCollector {
            inner: collector,
            the_score: self.the_score,
        };
        self.bulk_scorer.score(&mut wrapped, accept_docs, min, max)
    }

    fn cost(&self) -> i64 {
        self.bulk_scorer.cost()
    }
}

/// The scorer supplier of a [`ConstantScoreQuery`].
///
/// Equivalent to the anonymous `ScorerSupplier` in
/// `ConstantScoreQuery.createWeight`.
struct ConstantScoreQuerySupplier {
    inner: Box<dyn ScorerSupplier>,
    score: f32,
    score_mode: ScoreMode,
}

impl ScorerSupplier for ConstantScoreQuerySupplier {
    fn get(&mut self, lead_cost: i64) -> Result<Box<dyn Scorer>> {
        let inner_scorer = self.inner.get(lead_cost)?;
        Ok(match into_scorer_iterator(inner_scorer) {
            ScorerIterator::Simple(iterator) => Box::new(ConstantScoreScorer::from_iterator(
                self.score,
                self.score_mode,
                iterator,
            )),
            ScorerIterator::TwoPhase(two_phase) => Box::new(ConstantScoreScorer::from_two_phase(
                self.score,
                self.score_mode,
                two_phase,
            )),
        })
    }

    fn bulk_scorer(&mut self) -> Result<Box<dyn BulkScorer>> {
        if !self.score_mode.is_exhaustive() {
            // Reproduces `ScorerSupplier`'s default, which Java reaches with
            // `super.bulkScorer()`.
            return Ok(Box::new(DefaultBulkScorer::new(self.get(i64::MAX)?)));
        }
        let inner_scorer = self.inner.bulk_scorer()?;
        Ok(Box::new(ConstantBulkScorer::new(inner_scorer, self.score)))
    }

    fn cost(&self) -> i64 {
        self.inner.cost()
    }
}

/// The leaf-level behaviour of the weight a [`ConstantScoreQuery`] creates when
/// scores are needed.
///
/// Equivalent to the anonymous `ConstantScoreWeight` subclass in
/// `ConstantScoreQuery.createWeight`.
#[derive(Debug)]
struct ConstantScoreQueryWeight {
    inner_weight: Arc<dyn Weight>,
    score: f32,
    score_mode: ScoreMode,
}

impl ConstantScoreWeightImpl for ConstantScoreQueryWeight {
    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let Some(inner) = self.inner_weight.scorer_supplier(context)? else {
            return Ok(None);
        };
        Ok(Some(Box::new(ConstantScoreQuerySupplier {
            inner,
            score: self.score,
            score_mode: self.score_mode,
        })))
    }

    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.inner_weight.is_cacheable(ctx)
    }

    fn count(&self, context: &LeafReaderContext) -> Result<i32> {
        self.inner_weight.count(context)
    }

    fn matches(&self, context: &LeafReaderContext, doc: i32) -> Result<Option<Arc<dyn Matches>>> {
        self.inner_weight.matches(context, doc)
    }
}

impl Query for ConstantScoreQuery {
    fn to_query_string(&self, field: &str) -> String {
        format!("ConstantScore({})", self.query.to_query_string(field))
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        let mut sub = visitor.get_sub_visitor(Occur::FILTER, self);
        self.query.visit(&mut *sub);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn rewrite(&self, searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        let rewritten_inner = self.query.rewrite(searcher)?;
        // Java compares the rewritten query with `query` by reference identity;
        // an `Arc` clone preserves that identity, so `Arc::ptr_eq` models it.
        let mut same_as_query = rewritten_inner.is_none();
        let mut rewritten = rewritten_inner.unwrap_or_else(|| Arc::clone(&self.query));

        // Do some extra simplifications that are legal since scores are not
        // needed on the wrapped query.
        if let Some(boost) = rewritten.as_any().downcast_ref::<BoostQuery>() {
            rewritten = boost.get_query();
            same_as_query = false;
        } else if let Some(constant) = rewritten.as_any().downcast_ref::<ConstantScoreQuery>() {
            rewritten = constant.get_query();
            same_as_query = false;
        } else if let Some(boolean) = rewritten.as_any().downcast_ref::<BooleanQuery>() {
            if let Some(no_scoring) = boolean.rewrite_no_scoring() {
                rewritten = Arc::new(no_scoring);
                same_as_query = false;
            }
        }

        if rewritten.as_any().is::<MatchNoDocsQuery>() {
            // bubble up MatchNoDocsQuery
            return Ok(Some(rewritten));
        }

        if !same_as_query {
            return Ok(Some(Arc::new(ConstantScoreQuery::new(rewritten))));
        }

        if rewritten.as_any().is::<ConstantScoreQuery>() {
            return Ok(Some(rewritten));
        }

        if let Some(boost) = rewritten.as_any().downcast_ref::<BoostQuery>() {
            return Ok(Some(Arc::new(ConstantScoreQuery::new(boost.get_query()))));
        }

        Ok(None)
    }

    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        // If the score mode is exhaustive then pass COMPLETE_NO_SCORES,
        // otherwise pass TOP_DOCS to make sure to not disable any of the dynamic
        // pruning optimizations for queries sorted by field or top scores.
        let inner_score_mode = if score_mode.is_exhaustive() {
            ScoreMode::COMPLETE_NO_SCORES
        } else {
            ScoreMode::TOP_DOCS
        };
        let inner_weight =
            searcher.create_weight(Arc::clone(&self.query), inner_score_mode, 1.0)?;
        if score_mode.needs_scores() {
            Ok(Arc::new(ConstantScoreWeight::new(
                Arc::new(self.clone()),
                boost,
                ConstantScoreQueryWeight {
                    inner_weight,
                    score: boost,
                    score_mode,
                },
            )))
        } else {
            Ok(inner_weight)
        }
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        if !self.same_class_as(other) {
            return false;
        }
        let Some(other) = other.as_any().downcast_ref::<ConstantScoreQuery>() else {
            return false;
        };
        self.query.query_eq(&*other.query)
    }

    fn query_hash(&self) -> u64 {
        self.class_hash()
            .wrapping_mul(31)
            .wrapping_add(self.query.query_hash())
    }
}
