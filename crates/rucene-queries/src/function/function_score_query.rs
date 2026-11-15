/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `FunctionScoreQuery.java`, of the Java package
//! `org.apache.lucene.queries.function`.
//!
//! # Shared scorers and scores
//!
//! Java hands the same `Scorer` to `DoubleValuesSource.fromScorer` and keeps
//! iterating it, and `MultiplicativeBoostValuesSource` hands the same `scores`
//! to its boost source and keeps reading them. Rust cannot alias the two, so
//! the scorer and the scores are shared ([`Rc`] and [`RefCell`]) between the
//! holders, which read the very same object Java's references do.

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::LeafReaderContext;
use rucene::search::boolean_clause::Occur;
use rucene::search::doc_id_set_iterator::DocIdSetIterator;
use rucene::search::double_values::{self, DoubleValues};
use rucene::search::double_values_source::{self, DoubleValuesSource};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::matches::Matches;
use rucene::search::query::{query_to_string, Query};
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::scorable::Scorable;
use rucene::search::score_mode::ScoreMode;
use rucene::search::scorer::Scorer;
use rucene::search::scorer_supplier::ScorerSupplier;
use rucene::search::segment_cacheable::SegmentCacheable;
use rucene::search::similarities::Explanation;
use rucene::search::two_phase_iterator::TwoPhaseIterator;
use rucene::search::weight::{DefaultScorerSupplier, Weight};
use rucene::util::java_collections::java_float_hash_code;
use rucene::util::java_fmt;

use crate::function::value_source::{
    java_float_compare, objects_equals_double, objects_equals_query, objects_hash, same_object,
};

/// A query that wraps another query, and uses a [`DoubleValuesSource`] to
/// replace or modify the wrapped query's score.
///
/// Equivalent to the final class
/// `org.apache.lucene.queries.function.FunctionScoreQuery`. If the
/// value source does not produce a value for a document, that document's
/// score is `0`; negative and `NaN` values are mapped to `0` too.
#[derive(Debug, Clone)]
pub struct FunctionScoreQuery {
    in_: Arc<dyn Query>,
    source: Arc<dyn DoubleValuesSource>,
}

impl FunctionScoreQuery {
    /// Creates a query that wraps `in_`, scored by `source`.
    ///
    /// Equivalent to `FunctionScoreQuery(Query, DoubleValuesSource)`.
    pub fn new(in_: Arc<dyn Query>, source: Arc<dyn DoubleValuesSource>) -> Self {
        Self { in_, source }
    }

    /// Returns the wrapped query.
    ///
    /// Equivalent to `FunctionScoreQuery.getWrappedQuery()`.
    pub fn get_wrapped_query(&self) -> &Arc<dyn Query> {
        &self.in_
    }

    /// Returns the underlying value source.
    ///
    /// Equivalent to `FunctionScoreQuery.getSource()`.
    pub fn get_source(&self) -> &Arc<dyn DoubleValuesSource> {
        &self.source
    }

    /// Returns a query that multiplies the score of `in_` by the value of
    /// `boost`; documents without a value keep their score.
    ///
    /// Equivalent to the static
    /// `FunctionScoreQuery.boostByValue(Query, DoubleValuesSource)`.
    pub fn boost_by_value(in_: Arc<dyn Query>, boost: Arc<dyn DoubleValuesSource>) -> Self {
        Self::new(in_, Arc::new(MultiplicativeBoostValuesSource::new(boost)))
    }

    /// Returns a query that multiplies by `boost_value` the score of the
    /// documents of `in_` that also match `boost_match`.
    ///
    /// Equivalent to the static
    /// `FunctionScoreQuery.boostByQuery(Query, Query, float)`.
    pub fn boost_by_query(
        in_: Arc<dyn Query>,
        boost_match: Arc<dyn Query>,
        boost_value: f32,
    ) -> Self {
        Self::new(
            in_,
            Arc::new(MultiplicativeBoostValuesSource::new(Arc::new(
                QueryBoostValuesSource::new(
                    double_values_source::from_query(boost_match),
                    boost_value,
                ),
            ))),
        )
    }
}

impl Query for FunctionScoreQuery {
    fn to_query_string(&self, field: &str) -> String {
        format!(
            "FunctionScoreQuery({}, scored by {})",
            self.in_.to_query_string(field),
            self.source.to_source_string()
        )
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        let mut sub = visitor.get_sub_visitor(Occur::MUST, self);
        self.in_.visit(&mut *sub);
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
        let sm = if score_mode.needs_scores() && self.source.needs_scores() {
            ScoreMode::COMPLETE
        } else {
            ScoreMode::COMPLETE_NO_SCORES
        };
        let inner = self.in_.create_weight(searcher, sm, 1.0)?;
        if !score_mode.needs_scores() {
            return Ok(inner);
        }
        Ok(Arc::new(FunctionScoreWeight::new(
            Arc::new(self.clone()),
            inner,
            Arc::clone(&self.source).rewrite(searcher)?,
            boost,
        )))
    }

    fn rewrite(&self, searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        match self.in_.rewrite(searcher)? {
            None => Ok(None),
            Some(rewritten) => Ok(Some(Arc::new(FunctionScoreQuery::new(
                rewritten,
                Arc::clone(&self.source),
            )))),
        }
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        same_object(self, other)
            || other
                .as_any()
                .downcast_ref::<FunctionScoreQuery>()
                .is_some_and(|that| {
                    objects_equals_query(&self.in_, &that.in_)
                        && objects_equals_double(&self.source, &that.source)
                })
    }

    /// `Objects.hash(in, source)`.
    fn query_hash(&self) -> i32 {
        objects_hash(&[self.in_.query_hash(), self.source.source_hash()])
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.queries.function.FunctionScoreQuery"
    }
}

/// The weight of a [`FunctionScoreQuery`].
///
/// Equivalent to the private static class
/// `org.apache.lucene.queries.function.FunctionScoreQuery.FunctionScoreWeight`.
#[derive(Debug)]
pub(crate) struct FunctionScoreWeight {
    query: Arc<FunctionScoreQuery>,
    inner: Arc<dyn Weight>,
    value_source: Arc<dyn DoubleValuesSource>,
    boost: f32,
}

impl FunctionScoreWeight {
    /// Equivalent to `FunctionScoreWeight(Query, Weight, DoubleValuesSource,
    /// float)`.
    fn new(
        query: Arc<FunctionScoreQuery>,
        inner: Arc<dyn Weight>,
        value_source: Arc<dyn DoubleValuesSource>,
        boost: f32,
    ) -> Self {
        Self {
            query,
            inner,
            value_source,
            boost,
        }
    }
}

impl SegmentCacheable for FunctionScoreWeight {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.inner.is_cacheable(ctx) && self.value_source.is_cacheable(ctx)
    }
}

impl Weight for FunctionScoreWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        Arc::clone(&self.query) as Arc<dyn Query>
    }

    fn matches(&self, context: &LeafReaderContext, doc: i32) -> Result<Option<Arc<dyn Matches>>> {
        self.inner.matches(context, doc)
    }

    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation> {
        let score_explanation = self.inner.explain(context, doc)?;
        if !score_explanation.is_match() {
            return Ok(score_explanation);
        }

        let scorer = self
            .inner
            .scorer(context)?
            .ok_or_else(LuceneError::null_pointer)?;
        let scorer = Rc::new(RefCell::new(scorer));
        let mut values = Arc::clone(&self.value_source).get_values(
            context,
            Some(Box::new(SharedScorerScores {
                scorer: Rc::clone(&scorer),
            })),
        )?;
        let advanced = scorer.borrow_mut().iterator().advance(doc)?;
        debug_assert_eq!(advanced, doc);

        let mut value;
        let mut expl;
        if values.advance_exact(doc)? {
            value = values.double_value()?;
            expl = Arc::clone(&self.value_source).explain(context, doc, &score_explanation)?;
            if value < 0.0 {
                value = 0.0;
                expl = Explanation::matched(
                    0,
                    "truncated score, max of:",
                    vec![
                        Explanation::matched(0f32, "minimum score", Vec::new()),
                        expl,
                    ],
                );
            } else if value.is_nan() {
                value = 0.0;
                expl = Explanation::matched(
                    0,
                    "score, computed as (score == NaN ? 0 : score) since NaN is an illegal score from:",
                    vec![expl],
                );
            }
        } else {
            value = 0.0;
            expl = Arc::clone(&self.value_source).explain(context, doc, &score_explanation)?;
        }

        let query = query_to_string(&*self.get_query());
        if !expl.is_match() {
            expl = Explanation::matched(
                0f32,
                format!("weight({query}) using default score of 0 because the function produced no value:"),
                vec![expl],
            );
        } else if self.boost != 1.0 {
            expl = Explanation::matched(
                (value * f64::from(self.boost)) as f32,
                format!("weight({query}), product of:"),
                vec![Explanation::matched(self.boost, "boost", Vec::new()), expl],
            );
        } else {
            expl = Explanation::matched(
                expl.value(),
                format!("weight({query}), result of:"),
                vec![expl],
            );
        }
        Ok(expl)
    }

    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let Some(in_) = self.inner.scorer(context)? else {
            return Ok(None);
        };
        let in_ = Rc::new(RefCell::new(in_));
        let scores = Arc::clone(&self.value_source).get_values(
            context,
            Some(Box::new(SharedScorerScores {
                scorer: Rc::clone(&in_),
            })),
        )?;
        let scorer = FunctionScoreScorer::new(in_, scores, self.boost);
        Ok(Some(Box::new(DefaultScorerSupplier::new(Box::new(scorer)))))
    }

    fn count(&self, context: &LeafReaderContext) -> Result<i32> {
        self.inner.count(context)
    }
}

/// A scorer shared between the scorer that iterates it and the values that
/// read its score.
type SharedScorer = Rc<RefCell<Box<dyn Scorer>>>;

/// `DoubleValuesSource.fromScorer(in)` over a [`SharedScorer`]: `advanceExact`
/// always accepts and `doubleValue` reads `in.score()`.
struct SharedScorerScores {
    scorer: SharedScorer,
}

impl DoubleValues for SharedScorerScores {
    fn double_value(&mut self) -> Result<f64> {
        Ok(f64::from(self.scorer.borrow_mut().score()?))
    }

    fn advance_exact(&mut self, _doc: i32) -> Result<bool> {
        Ok(true)
    }
}

/// `in.iterator()` of a [`SharedScorer`].
struct SharedScorerIterator {
    scorer: SharedScorer,
}

impl DocIdSetIterator for SharedScorerIterator {
    fn doc_id(&self) -> i32 {
        self.scorer.borrow_mut().iterator().doc_id()
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

/// The approximation of `in.twoPhaseIterator()` of a [`SharedScorer`].
struct SharedScorerApproximation {
    scorer: SharedScorer,
}

impl SharedScorerApproximation {
    fn with<T>(&self, f: impl FnOnce(&mut dyn DocIdSetIterator) -> T) -> T {
        let mut scorer = self.scorer.borrow_mut();
        let two_phase = scorer
            .two_phase_iterator()
            .expect("INVARIANT: the wrapped scorer was observed to have a two-phase view");
        f(two_phase.approximation())
    }
}

impl DocIdSetIterator for SharedScorerApproximation {
    fn doc_id(&self) -> i32 {
        self.with(|a| a.doc_id())
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.with(|a| a.next_doc())
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.with(|a| a.advance(target))
    }

    fn cost(&self) -> i64 {
        self.with(|a| a.cost())
    }
}

/// `in.twoPhaseIterator()` of a [`SharedScorer`].
struct SharedScorerTwoPhase {
    scorer: SharedScorer,
    approximation: SharedScorerApproximation,
}

impl TwoPhaseIterator for SharedScorerTwoPhase {
    fn approximation(&mut self) -> &mut dyn DocIdSetIterator {
        &mut self.approximation
    }

    fn approximation_ref(&self) -> &dyn DocIdSetIterator {
        &self.approximation
    }

    fn matches(&mut self) -> Result<bool> {
        let mut scorer = self.scorer.borrow_mut();
        scorer
            .two_phase_iterator()
            .expect("INVARIANT: the wrapped scorer was observed to have a two-phase view")
            .matches()
    }

    fn match_cost(&self) -> f32 {
        let mut scorer = self.scorer.borrow_mut();
        scorer
            .two_phase_iterator()
            .expect("INVARIANT: the wrapped scorer was observed to have a two-phase view")
            .match_cost()
    }
}

/// The anonymous `FilterScorer` of `FunctionScoreWeight.scorerSupplier`.
///
/// **Divergence.** Java's scorer is a `FilterScorer` over `in`, which the
/// value source also reads through `DoubleValuesSource.fromScorer(in)`. The
/// core [`FilterScorer`](rucene::search::scorer::FilterScorer) owns its
/// wrapped scorer, so the two cannot share it; this scorer reproduces the
/// subclass — `docID`, `iterator` and `twoPhaseIterator` delegated to `in`,
/// `score` and `getMaxScore` overridden — over the shared scorer, and does not
/// answer as a `FilterScorer` to a caller that tests the class.
struct FunctionScoreScorer {
    in_: SharedScorer,
    iterator: SharedScorerIterator,
    two_phase: Option<SharedScorerTwoPhase>,
    scores: Box<dyn DoubleValues>,
    boost: f32,
}

impl FunctionScoreScorer {
    fn new(in_: SharedScorer, scores: Box<dyn DoubleValues>, boost: f32) -> Self {
        let has_two_phase = in_.borrow_mut().two_phase_iterator().is_some();
        let two_phase = has_two_phase.then(|| SharedScorerTwoPhase {
            scorer: Rc::clone(&in_),
            approximation: SharedScorerApproximation {
                scorer: Rc::clone(&in_),
            },
        });
        Self {
            iterator: SharedScorerIterator {
                scorer: Rc::clone(&in_),
            },
            in_,
            two_phase,
            scores,
            boost,
        }
    }
}

impl Scorable for FunctionScoreScorer {
    fn score(&mut self) -> Result<f32> {
        let doc = self.in_.borrow().doc_id();
        if self.scores.advance_exact(doc)? {
            let factor = self.scores.double_value()?;
            if factor >= 0.0 {
                return Ok((factor * f64::from(self.boost)) as f32);
            }
        }
        Ok(0.0)
    }
}

impl Scorer for FunctionScoreScorer {
    fn doc_id(&self) -> i32 {
        self.in_.borrow().doc_id()
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        &mut self.iterator
    }

    fn two_phase_iterator(&mut self) -> Option<&mut dyn TwoPhaseIterator> {
        self.two_phase
            .as_mut()
            .map(|t| t as &mut dyn TwoPhaseIterator)
    }

    fn get_max_score(&mut self, _up_to: i32) -> Result<f32> {
        Ok(f32::INFINITY)
    }
}

/// The `scores` of a [`MultiplicativeBoostValuesSource`], shared between the
/// boost source it is handed to and the values that multiply by it.
struct SharedScores<'a> {
    scores: Rc<RefCell<Box<dyn DoubleValues + 'a>>>,
}

impl DoubleValues for SharedScores<'_> {
    fn double_value(&mut self) -> Result<f64> {
        self.scores.borrow_mut().double_value()
    }

    fn advance_exact(&mut self, doc: i32) -> Result<bool> {
        self.scores.borrow_mut().advance_exact(doc)
    }
}

/// A source that multiplies the scores by the value of another source;
/// documents without a value are multiplied by `1`.
///
/// Equivalent to the package-private static class
/// `org.apache.lucene.queries.function.FunctionScoreQuery.MultiplicativeBoostValuesSource`.
///
/// Java's member is package-private, reached by the same package's tests;
/// Rust's integration tests are another crate, so the port makes it public
/// (hidden from the documentation).
#[doc(hidden)]
#[derive(Debug)]
pub struct MultiplicativeBoostValuesSource {
    /// The package-private `boost` field, read by the same package's tests.
    #[doc(hidden)]
    pub boost: Arc<dyn DoubleValuesSource>,
}

impl MultiplicativeBoostValuesSource {
    /// Equivalent to `MultiplicativeBoostValuesSource(DoubleValuesSource)`.
    fn new(boost: Arc<dyn DoubleValuesSource>) -> Self {
        Self { boost }
    }
}

/// The anonymous `DoubleValues` of
/// `MultiplicativeBoostValuesSource.getValues`.
struct MultiplicativeBoostValues<'a> {
    /// `scores`, `None` for Java's `null`.
    scores: Option<Rc<RefCell<Box<dyn DoubleValues + 'a>>>>,
    /// `DoubleValues.withDefault(boost.getValues(ctx, scores), 1)`.
    in_: Box<dyn DoubleValues + 'a>,
}

impl DoubleValues for MultiplicativeBoostValues<'_> {
    fn double_value(&mut self) -> Result<f64> {
        let score = match &self.scores {
            Some(scores) => scores.borrow_mut().double_value()?,
            // `scores.doubleValue()` on a `null` scores.
            None => return Err(LuceneError::null_pointer()),
        };
        Ok(score * self.in_.double_value()?)
    }

    fn advance_exact(&mut self, doc: i32) -> Result<bool> {
        self.in_.advance_exact(doc)
    }
}

impl SegmentCacheable for MultiplicativeBoostValuesSource {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.boost.is_cacheable(ctx)
    }
}

impl DoubleValuesSource for MultiplicativeBoostValuesSource {
    fn get_values<'a>(
        self: Arc<Self>,
        ctx: &LeafReaderContext,
        scores: Option<Box<dyn DoubleValues + 'a>>,
    ) -> Result<Box<dyn DoubleValues + 'a>> {
        let scores = scores.map(|s| Rc::new(RefCell::new(s)));
        let boost_scores: Option<Box<dyn DoubleValues + 'a>> = scores.as_ref().map(|s| {
            Box::new(SharedScores {
                scores: Rc::clone(s),
            }) as Box<dyn DoubleValues + 'a>
        });
        let in_ = double_values::with_default(
            Arc::clone(&self.boost).get_values(ctx, boost_scores)?,
            1.0,
        );
        Ok(Box::new(MultiplicativeBoostValues {
            scores,
            in_: Box::new(in_),
        }))
    }

    fn needs_scores(&self) -> bool {
        true
    }

    fn rewrite(self: Arc<Self>, searcher: &IndexSearcher) -> Result<Arc<dyn DoubleValuesSource>> {
        Ok(Arc::new(MultiplicativeBoostValuesSource::new(
            Arc::clone(&self.boost).rewrite(searcher)?,
        )))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn source_eq(&self, other: &dyn DoubleValuesSource) -> bool {
        same_object(self, other)
            || other
                .as_any()
                .downcast_ref::<MultiplicativeBoostValuesSource>()
                .is_some_and(|that| objects_equals_double(&self.boost, &that.boost))
    }

    /// `Objects.hash(boost)`.
    fn source_hash(&self) -> i32 {
        objects_hash(&[self.boost.source_hash()])
    }

    fn to_source_string(&self) -> String {
        format!("boost({})", self.boost.to_source_string())
    }

    fn explain(
        self: Arc<Self>,
        ctx: &LeafReaderContext,
        doc_id: i32,
        score_explanation: &Explanation,
    ) -> Result<Explanation> {
        if !score_explanation.is_match() {
            return Ok(score_explanation.clone());
        }
        let boost_expl = Arc::clone(&self.boost).explain(ctx, doc_id, score_explanation)?;
        if !boost_expl.is_match() {
            return Ok(score_explanation.clone());
        }
        Ok(Explanation::matched(
            score_explanation.value().double_value() * boost_expl.value().double_value(),
            "product of:",
            vec![score_explanation.clone(), boost_expl],
        ))
    }
}

/// A source answering `boost` for the documents that match a query, and
/// nothing for the others.
///
/// Equivalent to the private static class
/// `org.apache.lucene.queries.function.FunctionScoreQuery.QueryBoostValuesSource`.
#[derive(Debug)]
pub(crate) struct QueryBoostValuesSource {
    query: Arc<dyn DoubleValuesSource>,
    boost: f32,
}

impl QueryBoostValuesSource {
    /// Equivalent to `QueryBoostValuesSource(DoubleValuesSource, float)`.
    fn new(query: Arc<dyn DoubleValuesSource>, boost: f32) -> Self {
        Self { query, boost }
    }
}

/// The anonymous `DoubleValues` of `QueryBoostValuesSource.getValues`.
struct QueryBoostValues<'a> {
    in_: Box<dyn DoubleValues + 'a>,
    boost: f32,
}

impl DoubleValues for QueryBoostValues<'_> {
    fn double_value(&mut self) -> Result<f64> {
        Ok(f64::from(self.boost))
    }

    fn advance_exact(&mut self, doc: i32) -> Result<bool> {
        self.in_.advance_exact(doc)
    }
}

impl SegmentCacheable for QueryBoostValuesSource {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.query.is_cacheable(ctx)
    }
}

impl DoubleValuesSource for QueryBoostValuesSource {
    fn get_values<'a>(
        self: Arc<Self>,
        ctx: &LeafReaderContext,
        _scores: Option<Box<dyn DoubleValues + 'a>>,
    ) -> Result<Box<dyn DoubleValues + 'a>> {
        let in_ = Arc::clone(&self.query).get_values(ctx, None)?;
        Ok(Box::new(double_values::with_default(
            QueryBoostValues {
                in_,
                boost: self.boost,
            },
            1.0,
        )))
    }

    fn needs_scores(&self) -> bool {
        false
    }

    fn rewrite(self: Arc<Self>, searcher: &IndexSearcher) -> Result<Arc<dyn DoubleValuesSource>> {
        Ok(Arc::new(QueryBoostValuesSource::new(
            Arc::clone(&self.query).rewrite(searcher)?,
            self.boost,
        )))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn source_eq(&self, other: &dyn DoubleValuesSource) -> bool {
        same_object(self, other)
            || other
                .as_any()
                .downcast_ref::<QueryBoostValuesSource>()
                .is_some_and(|that| {
                    java_float_compare(that.boost, self.boost) == 0
                        && objects_equals_double(&self.query, &that.query)
                })
    }

    /// `Objects.hash(query, boost)`.
    fn source_hash(&self) -> i32 {
        objects_hash(&[self.query.source_hash(), java_float_hash_code(self.boost)])
    }

    fn to_source_string(&self) -> String {
        format!(
            "queryboost({})^{}",
            self.query.to_source_string(),
            java_fmt::float_to_string(self.boost)
        )
    }

    fn explain(
        self: Arc<Self>,
        ctx: &LeafReaderContext,
        doc_id: i32,
        score_explanation: &Explanation,
    ) -> Result<Explanation> {
        let inner = Arc::clone(&self.query).explain(ctx, doc_id, score_explanation)?;
        if !inner.is_match() {
            return Ok(inner);
        }
        Ok(Explanation::matched(
            self.boost,
            format!("Matched boosting query {}", self.query.to_source_string()),
            Vec::new(),
        ))
    }
}
