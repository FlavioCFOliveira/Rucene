//! Log-odds fusion, ported from
//! `org.apache.lucene.search.LogOddsFusionQuery`.

#![deny(unsafe_code)]

use std::any::Any;
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::LeafReaderContext;
use crate::search::abstract_multi_term_query_constant_score_wrapper::BOOLEAN_REWRITE_TERM_COUNT_THRESHOLD;
use crate::search::boolean_clause::Occur;
use crate::search::boolean_query::BooleanQuery;
use crate::search::index_searcher::IndexSearcher;
use crate::search::log_odds_fusion_scorer::{logit, sigmoid, softplus, LogOddsFusionScorer};
use crate::search::match_no_docs_query::MatchNoDocsQuery;
use crate::search::matches::{Matches, MatchesUtils};
use crate::search::multiset::Multiset;
use crate::search::query::{Query, QueryKey};
use crate::search::query_visitor::QueryVisitor;
use crate::search::score_mode::ScoreMode;
use crate::search::scorer::Scorer;
use crate::search::scorer_supplier::ScorerSupplier;
use crate::search::segment_cacheable::SegmentCacheable;
use crate::search::similarities::Explanation;
use crate::search::weight::Weight;

/// A query that combines sub-query probability scores through log-odds fusion.
///
/// Equivalent to the `final class
/// org.apache.lucene.search.LogOddsFusionQuery`. Sub-queries are expected to
/// produce scores in `(0, 1)` representing probabilities — for instance from a
/// [`BayesianScoreQuery`](crate::search::BayesianScoreQuery) wrapping a BM25
/// query, or from a kNN cosine similarity.
///
/// The combination formula resolves the shrinkage problem of a naive
/// probabilistic AND by:
///
/// 1. converting each sub-score to log-odds: `logit(p) = log(p / (1 - p))`;
/// 2. computing the mean log-odds across all clauses, non-matching clauses
///    contributing `0` — neutral evidence;
/// 3. applying multiplicative confidence scaling: `meanLogit * n^alpha`;
/// 4. converting back to a probability through the sigmoid.
///
/// `alpha` controls the confidence scaling exponent; the default `0.5`
/// implements the `sqrt(n)` scaling law from "From Bayesian Inference to Neural
/// Computation".
///
/// Optional per-signal weights enable weighted logarithmic opinion pooling,
/// where each signal's log-odds contribution is scaled by its reliability
/// weight. The weights must be non-negative and sum to 1, and the formula
/// becomes `sigmoid(n^alpha * sum(w_i * softplus(logit(p_i))))` instead of the
/// uniform mean.
#[derive(Debug, Clone)]
pub struct LogOddsFusionQuery {
    clauses: Multiset<QueryKey>,
    ordered_clauses: Vec<Arc<dyn Query>>,
    alpha: f32,
    signal_weights: Option<Vec<f32>>,
    logit_min: Option<Vec<f32>>,
    logit_max: Option<Vec<f32>>,
}

impl LogOddsFusionQuery {
    /// Creates a query with per-signal weights and optional logit
    /// normalisation.
    ///
    /// Equivalent to
    /// `new LogOddsFusionQuery(Collection<? extends Query>, float, float[], float[], float[])`.
    ///
    /// * `alpha` — the confidence scaling exponent (`0.5` is the `sqrt(n)`
    ///   law);
    /// * `weights` — per-signal weights, which must be non-negative, finite and
    ///   sum to `1.0`, or `None` for uniform weighting;
    /// * `logit_min` / `logit_max` — per-signal logit bounds for normalisation,
    ///   or `None` to use softplus gating.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] — with Java's messages — when
    /// `alpha` is not in `[0, 1]`, or when the weights or bounds are invalid.
    pub fn new(
        clauses: Vec<Arc<dyn Query>>,
        alpha: f32,
        weights: Option<Vec<f32>>,
        logit_min: Option<Vec<f32>>,
        logit_max: Option<Vec<f32>>,
    ) -> Result<Self> {
        if alpha.is_nan() || !(0.0..=1.0).contains(&alpha) {
            return Err(LuceneError::IllegalArgument(format!(
                "alpha must be in [0, 1], got {alpha}"
            )));
        }
        let signal_weights = match weights {
            None => None,
            Some(weights) => {
                if weights.len() != clauses.len() {
                    return Err(LuceneError::IllegalArgument(format!(
                        "weights length {} must equal clauses size {}",
                        weights.len(),
                        clauses.len()
                    )));
                }
                let mut sum = 0f32;
                for w in &weights {
                    if !w.is_finite() || *w < 0.0 {
                        return Err(LuceneError::IllegalArgument(format!(
                            "weights must be non-negative and finite, got {w}"
                        )));
                    }
                    sum += w;
                }
                if (sum - 1.0f32).abs() > 1e-3 {
                    return Err(LuceneError::IllegalArgument(format!(
                        "weights must sum to 1.0, got {sum}"
                    )));
                }
                Some(weights)
            }
        };
        let (logit_min, logit_max) = match (logit_min, logit_max) {
            (Some(min), Some(max)) => {
                if min.len() != clauses.len() {
                    return Err(LuceneError::IllegalArgument(format!(
                        "logitMin length {} must equal clauses size {}",
                        min.len(),
                        clauses.len()
                    )));
                }
                if max.len() != clauses.len() {
                    return Err(LuceneError::IllegalArgument(format!(
                        "logitMax length {} must equal clauses size {}",
                        max.len(),
                        clauses.len()
                    )));
                }
                (Some(min), Some(max))
            }
            _ => (None, None),
        };
        let mut multiset = Multiset::new();
        multiset.add_all(clauses.iter().map(|query| QueryKey::new(Arc::clone(query))));
        Ok(Self {
            clauses: multiset,
            ordered_clauses: clauses,
            alpha,
            signal_weights,
            logit_min,
            logit_max,
        })
    }

    /// Creates a query with per-signal weights, softplus gating and no
    /// normalisation.
    ///
    /// Equivalent to
    /// `new LogOddsFusionQuery(Collection<? extends Query>, float, float[])`.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn with_weights(
        clauses: Vec<Arc<dyn Query>>,
        alpha: f32,
        weights: Option<Vec<f32>>,
    ) -> Result<Self> {
        Self::new(clauses, alpha, weights, None, None)
    }

    /// Creates a query with uniform weighting and softplus gating.
    ///
    /// Equivalent to
    /// `new LogOddsFusionQuery(Collection<? extends Query>, float)`.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn with_alpha(clauses: Vec<Arc<dyn Query>>, alpha: f32) -> Result<Self> {
        Self::new(clauses, alpha, None, None, None)
    }

    /// Creates a query with `alpha = 0.5`, uniform weighting and softplus
    /// gating.
    ///
    /// Equivalent to
    /// `new LogOddsFusionQuery(Collection<? extends Query>)`.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn with_clauses(clauses: Vec<Arc<dyn Query>>) -> Result<Self> {
        Self::new(clauses, 0.5, None, None, None)
    }

    /// Returns the clauses.
    ///
    /// Equivalent to `LogOddsFusionQuery.getClauses()`, and to the
    /// `Iterable<Query>` implementation.
    pub fn get_clauses(&self) -> &[Arc<dyn Query>] {
        &self.ordered_clauses
    }

    /// Returns the confidence scaling exponent.
    ///
    /// Equivalent to `LogOddsFusionQuery.getAlpha()`.
    pub fn get_alpha(&self) -> f32 {
        self.alpha
    }

    /// Returns a copy of the per-signal weights, or `None` when uniform
    /// weighting is used. The `i`-th element is the weight of the `i`-th clause
    /// of [`get_clauses`](Self::get_clauses).
    ///
    /// Equivalent to `LogOddsFusionQuery.getWeights()`.
    pub fn get_weights(&self) -> Option<Vec<f32>> {
        self.signal_weights.clone()
    }
}

impl Query for LogOddsFusionQuery {
    fn to_query_string(&self, field: &str) -> String {
        let joined = self
            .ordered_clauses
            .iter()
            .map(|subquery| {
                if subquery.as_any().is::<BooleanQuery>() {
                    format!("({})", subquery.to_query_string(field))
                } else {
                    subquery.to_query_string(field)
                }
            })
            .collect::<Vec<_>>()
            .join(" & ");
        let base = format!("LogOdds({joined})^{}", self.alpha);
        match &self.signal_weights {
            None => base,
            Some(weights) => format!("{base} w={weights:?}"),
        }
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        let mut sub_visitor = visitor.get_sub_visitor(Occur::SHOULD, self);
        for query in self.clauses.iter() {
            query.query().visit(sub_visitor.as_mut());
        }
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
        let mut weights = Vec::with_capacity(self.ordered_clauses.len());
        for clause_query in &self.ordered_clauses {
            weights.push(searcher.create_weight(Arc::clone(clause_query), score_mode, boost)?);
        }
        Ok(Arc::new(LogOddsFusionWeight {
            parent_query: Arc::new(self.clone()),
            query: self.clone(),
            weights,
            score_mode,
        }))
    }

    fn rewrite(&self, index_searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        if self.clauses.is_empty() {
            return Ok(Some(Arc::new(MatchNoDocsQuery::new(
                "empty LogOddsFusionQuery",
            ))));
        }
        if self.clauses.len() == 1 {
            return Ok(Some(Arc::clone(&self.ordered_clauses[0])));
        }

        let mut actually_rewritten = false;
        let mut rewritten_clauses: Vec<Arc<dyn Query>> = Vec::new();
        let mut new_weights = self.signal_weights.as_ref().map(|_| Vec::new());
        let mut new_logit_min = self.logit_min.as_ref().map(|_| Vec::new());
        let mut new_logit_max = self.logit_max.as_ref().map(|_| Vec::new());

        for (i, sub) in self.ordered_clauses.iter().enumerate() {
            let rewritten_sub = sub.rewrite(index_searcher)?;
            let sub_is_match_no_docs = sub.as_any().is::<MatchNoDocsQuery>();
            if rewritten_sub.is_some() || sub_is_match_no_docs {
                actually_rewritten = true;
            }
            let rewritten_sub = rewritten_sub.unwrap_or_else(|| Arc::clone(sub));
            if !rewritten_sub.as_any().is::<MatchNoDocsQuery>() {
                rewritten_clauses.push(rewritten_sub);
                if let (Some(new_weights), Some(weights)) =
                    (new_weights.as_mut(), self.signal_weights.as_ref())
                {
                    new_weights.push(weights[i]);
                }
                if let (Some(new_min), Some(min), Some(new_max), Some(max)) = (
                    new_logit_min.as_mut(),
                    self.logit_min.as_ref(),
                    new_logit_max.as_mut(),
                    self.logit_max.as_ref(),
                ) {
                    new_min.push(min[i]);
                    new_max.push(max[i]);
                }
            }
        }

        if !actually_rewritten {
            return Ok(None);
        }
        if rewritten_clauses.is_empty() {
            return Ok(Some(Arc::new(MatchNoDocsQuery::new(
                "empty LogOddsFusionQuery",
            ))));
        }
        if rewritten_clauses.len() == 1 {
            return Ok(Some(rewritten_clauses.remove(0)));
        }

        if let Some(filtered) = new_weights.as_mut() {
            let sum: f32 = filtered.iter().sum();
            if sum > 0.0 {
                for weight in filtered.iter_mut() {
                    *weight /= sum;
                }
            }
        }

        Ok(Some(Arc::new(LogOddsFusionQuery::new(
            rewritten_clauses,
            self.alpha,
            new_weights,
            new_logit_min,
            new_logit_max,
        )?)))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        match other.as_any().downcast_ref::<LogOddsFusionQuery>() {
            Some(other) => {
                self.alpha == other.alpha
                    && self.clauses == other.clauses
                    && self.signal_weights == other.signal_weights
                    && self.logit_min == other.logit_min
                    && self.logit_max == other.logit_max
            }
            None => false,
        }
    }

    fn query_hash(&self) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        self.class_hash().hash(&mut hasher);
        self.alpha.to_bits().hash(&mut hasher);
        self.clauses.hash(&mut hasher);
        hash_float_slice(&self.signal_weights, &mut hasher);
        hash_float_slice(&self.logit_min, &mut hasher);
        hash_float_slice(&self.logit_max, &mut hasher);
        hasher.finish()
    }
}

fn hash_float_slice<H: std::hash::Hasher>(values: &Option<Vec<f32>>, hasher: &mut H) {
    use std::hash::Hash;
    match values {
        None => 0u8.hash(hasher),
        Some(values) => {
            1u8.hash(hasher);
            for value in values {
                value.to_bits().hash(hasher);
            }
        }
    }
}

/// The weight of a [`LogOddsFusionQuery`].
///
/// Equivalent to the inner class
/// `LogOddsFusionQuery.LogOddsFusionWeight`, whose enclosing query's fields
/// become a field here.
#[derive(Debug)]
struct LogOddsFusionWeight {
    parent_query: Arc<dyn Query>,
    query: LogOddsFusionQuery,
    weights: Vec<Arc<dyn Weight>>,
    score_mode: ScoreMode,
}

impl SegmentCacheable for LogOddsFusionWeight {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        if self.weights.len() > BOOLEAN_REWRITE_TERM_COUNT_THRESHOLD as usize {
            return false;
        }
        self.weights.iter().all(|weight| weight.is_cacheable(ctx))
    }
}

impl Weight for LogOddsFusionWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        Arc::clone(&self.parent_query)
    }

    fn matches(&self, context: &LeafReaderContext, doc: i32) -> Result<Option<Arc<dyn Matches>>> {
        let mut mis = Vec::new();
        for weight in &self.weights {
            if let Some(mi) = weight.matches(context, doc)? {
                mis.push(mi);
            }
        }
        Ok(MatchesUtils::from_sub_matches(mis))
    }

    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation> {
        let mut is_match = false;
        let mut subs_on_match = Vec::new();
        let mut subs_on_no_match = Vec::new();
        let mut logit_sum = 0f64;
        let total_clauses = self.weights.len();

        for (i, weight) in self.weights.iter().enumerate() {
            let explanation = weight.explain(context, doc)?;
            if explanation.is_match() {
                is_match = true;
                let sub_score = explanation.value().float_value();
                subs_on_match.push(explanation);
                let raw_logit = logit(sub_score);
                let gated = match (&self.query.logit_min, &self.query.logit_max) {
                    (Some(min), Some(max)) => {
                        let range = max[i] - min[i];
                        if range > 0.0 {
                            let normalised = (raw_logit - min[i]) / range;
                            if normalised.is_nan() {
                                normalised
                            } else if normalised < 0.0 {
                                0.0
                            } else if normalised > 1.0 {
                                1.0
                            } else {
                                normalised
                            }
                        } else {
                            0.5
                        }
                    }
                    _ => softplus(raw_logit),
                };
                match &self.query.signal_weights {
                    Some(weights) => logit_sum += f64::from(weights[i] * gated),
                    None => logit_sum += f64::from(gated),
                }
            } else if !is_match {
                subs_on_no_match.push(explanation);
            }
        }

        if is_match {
            let scaling_factor = (total_clauses as f64).powf(self.query.alpha as f64) as f32;
            let (scaled_logit, description) = match &self.query.signal_weights {
                Some(_) => (
                    logit_sum as f32 * scaling_factor,
                    "weighted log-odds fusion, computed as sigmoid(weightedLogit * n^alpha) from:",
                ),
                None => (
                    (logit_sum / total_clauses as f64) as f32 * scaling_factor,
                    "log-odds fusion, computed as sigmoid(meanLogit * n^alpha) from:",
                ),
            };
            Ok(Explanation::matched(
                sigmoid(scaled_logit),
                description,
                subs_on_match,
            ))
        } else {
            Ok(Explanation::no_match(
                "No matching clause",
                subs_on_no_match,
            ))
        }
    }

    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let mut scorer_suppliers: Vec<Box<dyn ScorerSupplier>> = Vec::new();
        let mut active_weights = self.query.signal_weights.as_ref().map(|_| Vec::new());
        let mut active_logit_min = self.query.logit_min.as_ref().map(|_| Vec::new());
        let mut active_logit_max = self.query.logit_max.as_ref().map(|_| Vec::new());

        for (i, weight) in self.weights.iter().enumerate() {
            if let Some(ss) = weight.scorer_supplier(context)? {
                scorer_suppliers.push(ss);
                if let (Some(active), Some(weights)) =
                    (active_weights.as_mut(), self.query.signal_weights.as_ref())
                {
                    active.push(weights[i]);
                }
                if let (Some(active_min), Some(min), Some(active_max), Some(max)) = (
                    active_logit_min.as_mut(),
                    self.query.logit_min.as_ref(),
                    active_logit_max.as_mut(),
                    self.query.logit_max.as_ref(),
                ) {
                    active_min.push(min[i]);
                    active_max.push(max[i]);
                }
            }
        }

        if scorer_suppliers.is_empty() {
            Ok(None)
        } else if scorer_suppliers.len() == 1 {
            Ok(Some(scorer_suppliers.remove(0)))
        } else {
            Ok(Some(Box::new(LogOddsFusionScorerSupplier {
                scorer_suppliers,
                total_clauses: self.query.clauses.len() as i32,
                alpha: self.query.alpha,
                active_weights,
                active_logit_min,
                active_logit_max,
                score_mode: self.score_mode,
                cost: None,
            })))
        }
    }
}

/// The supplier of a [`LogOddsFusionScorer`].
///
/// Equivalent to the anonymous `ScorerSupplier` of
/// `LogOddsFusionQuery.LogOddsFusionWeight.scorerSupplier`.
struct LogOddsFusionScorerSupplier {
    scorer_suppliers: Vec<Box<dyn ScorerSupplier>>,
    total_clauses: i32,
    alpha: f32,
    active_weights: Option<Vec<f32>>,
    active_logit_min: Option<Vec<f32>>,
    active_logit_max: Option<Vec<f32>>,
    score_mode: ScoreMode,
    cost: Option<i64>,
}

impl ScorerSupplier for LogOddsFusionScorerSupplier {
    fn get(&mut self, lead_cost: i64) -> Result<Box<dyn Scorer>> {
        let mut scorers = Vec::with_capacity(self.scorer_suppliers.len());
        for ss in self.scorer_suppliers.iter_mut() {
            scorers.push(ss.get(lead_cost)?);
        }
        Ok(Box::new(LogOddsFusionScorer::new(
            scorers,
            self.total_clauses,
            self.alpha,
            self.active_weights.clone(),
            self.active_logit_min.clone(),
            self.active_logit_max.clone(),
            self.score_mode,
            lead_cost,
        )?))
    }

    fn cost(&self) -> i64 {
        // **Divergence from Lucene 10.5.0.** Java memoises the sum in a mutable
        // field read from `cost()`. `ScorerSupplier::cost` takes `&self` in this
        // port, so the sum is recomputed; `ScorerSupplier::cost` is documented
        // as costly and is called sparingly, and the value is the same.
        match self.cost {
            Some(cost) => cost,
            None => self.scorer_suppliers.iter().map(|ss| ss.cost()).sum(),
        }
    }

    fn set_top_level_scoring_clause(&mut self) -> Result<()> {
        for ss in self.scorer_suppliers.iter_mut() {
            ss.set_top_level_scoring_clause()?;
        }
        Ok(())
    }
}

impl std::fmt::Debug for LogOddsFusionScorerSupplier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogOddsFusionScorerSupplier")
            .field("clauses", &self.scorer_suppliers.len())
            .field("alpha", &self.alpha)
            .finish()
    }
}
