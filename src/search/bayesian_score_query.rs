//! Sigmoid score calibration, ported from
//! `org.apache.lucene.search.BayesianScoreQuery`.

#![deny(unsafe_code)]

use std::any::Any;
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::LeafReaderContext;
use crate::search::boolean_clause::Occur;
use crate::search::doc_id_set_iterator::DocIdSetIterator;
use crate::search::index_searcher::IndexSearcher;
use crate::search::match_no_docs_query::MatchNoDocsQuery;
use crate::search::matches::Matches;
use crate::search::query::Query;
use crate::search::query_visitor::QueryVisitor;
use crate::search::scorable::Scorable;
use crate::search::score_mode::ScoreMode;
use crate::search::scorer::{FilterScorer, Scorer};
use crate::search::scorer_supplier::ScorerSupplier;
use crate::search::segment_cacheable::SegmentCacheable;
use crate::search::similarities::Explanation;
use crate::search::weight::Weight;

/// The logistic function, computed in the numerically stable direction.
///
/// Equivalent to the package-private
/// `BayesianScoreQuery.sigmoid(float)`, which is the same function
/// [`LogOddsFusionScorer`](crate::search::LogOddsFusionScorer) declares.
pub fn sigmoid(x: f32) -> f32 {
    if x >= 0.0 {
        (1.0 / (1.0 + (-(x as f64)).exp())) as f32
    } else {
        let exp_x = (x as f64).exp();
        (exp_x / (1.0 + exp_x)) as f32
    }
}

/// A query wrapper that turns the inner query's score into a calibrated
/// probability through sigmoid calibration: `P = sigmoid(alpha * (score -
/// beta))`.
///
/// Equivalent to the `final class
/// org.apache.lucene.search.BayesianScoreQuery`. It implements the query-level
/// Bayesian transform from "Bayesian BM25": the inner query — typically a
/// multi-term boolean query with BM25 — produces a raw score, and this wrapper
/// maps it into a probability in `(0, 1)` suitable for combination with other
/// probability signals through
/// [`LogOddsFusionQuery`](crate::search::LogOddsFusionQuery).
///
/// `alpha` controls the sigmoid steepness — the score sensitivity — and `beta`
/// controls the midpoint, the decision boundary. They can be set by hand or
/// estimated from the score distribution with
/// [`BayesianScoreEstimator`](crate::search::BayesianScoreEstimator).
///
/// An optional base rate encodes the corpus-level prior probability that a
/// random document is relevant to a random query. When set, the posterior is
/// computed in log-odds space: `sigmoid(alpha * (score - beta) +
/// logit(baseRate))`, which shifts scores down for corpora where relevance is
/// rare and improves calibration.
#[derive(Debug, Clone)]
pub struct BayesianScoreQuery {
    query: Arc<dyn Query>,
    alpha: f32,
    beta: f32,
    base_rate: f32,
    logit_base_rate: f32,
}

impl BayesianScoreQuery {
    /// Creates a query with a base rate.
    ///
    /// Equivalent to
    /// `new BayesianScoreQuery(Query, float, float, float)`.
    ///
    /// * `alpha` — the sigmoid steepness, which must be positive and finite;
    /// * `beta` — the sigmoid midpoint, which must be finite;
    /// * `base_rate` — the corpus-level relevance prior in `(0, 1)`, or `0` to
    ///   disable it. When positive, `logit(base_rate)` is added to the log-odds
    ///   before the sigmoid, which accounts for the rarity of relevant
    ///   documents.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] — with Java's messages — when
    /// `alpha` is not positive and finite, `beta` is not finite, or `base_rate`
    /// is outside `[0, 1)`.
    pub fn new(query: Arc<dyn Query>, alpha: f32, beta: f32, base_rate: f32) -> Result<Self> {
        if !alpha.is_finite() || alpha <= 0.0 {
            return Err(LuceneError::IllegalArgument(format!(
                "alpha must be a positive finite value, got {alpha}"
            )));
        }
        if !beta.is_finite() {
            return Err(LuceneError::IllegalArgument(format!(
                "beta must be a finite value, got {beta}"
            )));
        }
        if base_rate < 0.0 || base_rate >= 1.0 {
            return Err(LuceneError::IllegalArgument(format!(
                "baseRate must be in [0, 1), got {base_rate}"
            )));
        }
        let logit_base_rate = if base_rate > 0.0 {
            ((base_rate as f64) / (1.0 - base_rate as f64)).ln() as f32
        } else {
            0.0
        };
        Ok(Self {
            query,
            alpha,
            beta,
            base_rate,
            logit_base_rate,
        })
    }

    /// Creates a query without a base rate.
    ///
    /// Equivalent to `new BayesianScoreQuery(Query, float, float)`.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn with_alpha_beta(query: Arc<dyn Query>, alpha: f32, beta: f32) -> Result<Self> {
        Self::new(query, alpha, beta, 0.0)
    }

    /// Returns the wrapped query.
    ///
    /// Equivalent to `BayesianScoreQuery.getQuery()`.
    pub fn get_query(&self) -> &Arc<dyn Query> {
        &self.query
    }

    /// Returns the sigmoid steepness parameter.
    ///
    /// Equivalent to `BayesianScoreQuery.getAlpha()`.
    pub fn get_alpha(&self) -> f32 {
        self.alpha
    }

    /// Returns the sigmoid midpoint parameter.
    ///
    /// Equivalent to `BayesianScoreQuery.getBeta()`.
    pub fn get_beta(&self) -> f32 {
        self.beta
    }

    /// Returns the base rate, or `0` when it is not set.
    ///
    /// Equivalent to `BayesianScoreQuery.getBaseRate()`.
    pub fn get_base_rate(&self) -> f32 {
        self.base_rate
    }

    /// Applies the calibration to a raw score.
    fn transform(&self, inner_score: f32) -> f32 {
        sigmoid(self.alpha * (inner_score - self.beta) + self.logit_base_rate)
    }
}

impl Query for BayesianScoreQuery {
    fn to_query_string(&self, field: &str) -> String {
        let mut base = format!(
            "BayesianScore({}, alpha={}, beta={}",
            self.query.to_query_string(field),
            self.alpha,
            self.beta
        );
        if self.base_rate > 0.0 {
            base.push_str(&format!(", baseRate={}", self.base_rate));
        }
        base.push(')');
        base
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        let mut sub_visitor = visitor.get_sub_visitor(Occur::MUST, self);
        self.query.visit(sub_visitor.as_mut());
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
        let inner_weight = self.query.create_weight(searcher, score_mode, boost)?;
        if !score_mode.needs_scores() {
            return Ok(inner_weight);
        }
        Ok(Arc::new(BayesianScoreWeight {
            parent_query: Arc::new(self.clone()),
            query: self.clone(),
            inner_weight,
        }))
    }

    fn rewrite(&self, index_searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        let rewritten = self.query.rewrite(index_searcher)?;
        match rewritten {
            None => Ok(None),
            Some(rewritten) => {
                if rewritten.as_any().is::<MatchNoDocsQuery>() {
                    return Ok(Some(rewritten));
                }
                Ok(Some(Arc::new(BayesianScoreQuery::new(
                    rewritten,
                    self.alpha,
                    self.beta,
                    self.base_rate,
                )?)))
            }
        }
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        match other.as_any().downcast_ref::<BayesianScoreQuery>() {
            Some(other) => {
                self.query.query_eq(other.query.as_ref())
                    && self.alpha.to_bits() == other.alpha.to_bits()
                    && self.beta.to_bits() == other.beta.to_bits()
                    && self.base_rate.to_bits() == other.base_rate.to_bits()
            }
            None => false,
        }
    }

    fn query_hash(&self) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        self.class_hash().hash(&mut hasher);
        self.query.query_hash().hash(&mut hasher);
        self.alpha.to_bits().hash(&mut hasher);
        self.beta.to_bits().hash(&mut hasher);
        self.base_rate.to_bits().hash(&mut hasher);
        hasher.finish()
    }
}

/// The weight of a [`BayesianScoreQuery`].
///
/// Equivalent to the inner class
/// `BayesianScoreQuery.BayesianScoreWeight`, whose enclosing query's fields
/// become a field here.
#[derive(Debug)]
struct BayesianScoreWeight {
    parent_query: Arc<dyn Query>,
    query: BayesianScoreQuery,
    inner_weight: Arc<dyn Weight>,
}

impl SegmentCacheable for BayesianScoreWeight {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.inner_weight.is_cacheable(ctx)
    }
}

impl Weight for BayesianScoreWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        Arc::clone(&self.parent_query)
    }

    fn matches(&self, context: &LeafReaderContext, doc: i32) -> Result<Option<Arc<dyn Matches>>> {
        self.inner_weight.matches(context, doc)
    }

    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation> {
        let inner_expl = self.inner_weight.explain(context, doc)?;
        if !inner_expl.is_match() {
            return Ok(inner_expl);
        }
        let inner_score = inner_expl.value().float_value();
        let transformed = self.query.transform(inner_score);
        if self.query.base_rate > 0.0 {
            return Ok(Explanation::matched(
                transformed,
                "sigmoid calibration with base rate, computed as sigmoid(alpha * (score - beta) \
                 + logit(baseRate)) from:",
                vec![
                    inner_expl,
                    Explanation::matched(self.query.alpha, "alpha, sigmoid steepness", Vec::new()),
                    Explanation::matched(self.query.beta, "beta, sigmoid midpoint", Vec::new()),
                    Explanation::matched(
                        self.query.base_rate,
                        "baseRate, corpus-level relevance prior",
                        Vec::new(),
                    ),
                ],
            ));
        }
        Ok(Explanation::matched(
            transformed,
            "sigmoid calibration, computed as sigmoid(alpha * (score - beta)) from:",
            vec![
                inner_expl,
                Explanation::matched(self.query.alpha, "alpha, sigmoid steepness", Vec::new()),
                Explanation::matched(self.query.beta, "beta, sigmoid midpoint", Vec::new()),
            ],
        ))
    }

    fn count(&self, context: &LeafReaderContext) -> Result<i32> {
        self.inner_weight.count(context)
    }

    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        match self.inner_weight.scorer_supplier(context)? {
            None => Ok(None),
            Some(inner_supplier) => Ok(Some(Box::new(BayesianScorerSupplier {
                inner_supplier,
                query: self.query.clone(),
            }))),
        }
    }
}

/// The supplier of a [`BayesianScoreScorer`].
///
/// Equivalent to the anonymous `ScorerSupplier` of
/// `BayesianScoreQuery.BayesianScoreWeight.scorerSupplier`.
struct BayesianScorerSupplier {
    inner_supplier: Box<dyn ScorerSupplier>,
    query: BayesianScoreQuery,
}

impl std::fmt::Debug for BayesianScorerSupplier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BayesianScorerSupplier")
            .field("query", &self.query)
            .finish_non_exhaustive()
    }
}

impl ScorerSupplier for BayesianScorerSupplier {
    fn get(&mut self, lead_cost: i64) -> Result<Box<dyn Scorer>> {
        let inner_scorer = self.inner_supplier.get(lead_cost)?;
        Ok(Box::new(BayesianScoreScorer {
            inner: FilterScorer::new(inner_scorer),
            query: self.query.clone(),
        }))
    }

    fn cost(&self) -> i64 {
        self.inner_supplier.cost()
    }

    fn set_top_level_scoring_clause(&mut self) -> Result<()> {
        self.inner_supplier.set_top_level_scoring_clause()
    }
}

/// The scorer of a [`BayesianScoreQuery`].
///
/// Equivalent to the inner class
/// `BayesianScoreQuery.BayesianScoreScorer`, which extends `FilterScorer`.
struct BayesianScoreScorer {
    inner: FilterScorer,
    query: BayesianScoreQuery,
}

impl std::fmt::Debug for BayesianScoreScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BayesianScoreScorer")
            .field("query", &self.query)
            .finish_non_exhaustive()
    }
}

impl Scorable for BayesianScoreScorer {
    fn score(&mut self) -> Result<f32> {
        let inner_score = self.inner.inner_mut().score()?;
        Ok(self.query.transform(inner_score))
    }

    fn set_min_competitive_score(&mut self, min_score: f32) -> Result<()> {
        // Invert the sigmoid to get the minimum inner score needed:
        //   minScore = sigmoid(alpha * (innerScore - beta) + logitBaseRate)
        //   => alpha * (innerScore - beta) + logitBaseRate = logit(minScore)
        //   => innerScore = (logit(minScore) - logitBaseRate) / alpha + beta
        if min_score > 0.0 && min_score < 1.0 {
            let clamped = 1e-7f32.max((1.0f32 - 1e-7).min(min_score));
            let logit_min = ((clamped as f64) / (1.0 - clamped as f64)).ln() as f32;
            let inner_min =
                (logit_min - self.query.logit_base_rate) / self.query.alpha + self.query.beta;
            self.inner
                .inner_mut()
                .set_min_competitive_score(0.0f32.max(inner_min))?;
        }
        Ok(())
    }

    fn smoothing_score(&mut self, doc_id: i32) -> Result<f32> {
        self.inner.inner_mut().smoothing_score(doc_id)
    }
}

impl Scorer for BayesianScoreScorer {
    fn as_scorable(&mut self) -> &mut dyn Scorable {
        self
    }

    fn doc_id(&self) -> i32 {
        self.inner.inner().doc_id()
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        self.inner.inner_mut().iterator()
    }

    fn advance_shallow(&mut self, target: i32) -> Result<i32> {
        self.inner.inner_mut().advance_shallow(target)
    }

    fn get_max_score(&mut self, up_to: i32) -> Result<f32> {
        let inner_max = self.inner.inner_mut().get_max_score(up_to)?;
        // The sigmoid is monotone, so max(sigmoid(f(x))) = sigmoid(max(f(x))).
        Ok(self.query.transform(inner_max))
    }
}
