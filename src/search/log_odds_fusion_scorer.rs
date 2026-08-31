//! Log-odds fusion scoring, ported from
//! `org.apache.lucene.search.LogOddsFusionScorer`.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::search::disjunction_score_block_boundary_propagator::DisjunctionScoreBlockBoundaryPropagator;
use crate::search::disjunction_scorer::DisjunctionScorer;
use crate::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use crate::search::scorable::{ChildScorable, Scorable};
use crate::search::score_mode::ScoreMode;
use crate::search::scorer::Scorer;
use crate::search::two_phase_iterator::TwoPhaseIterator;

/// The lowest probability a sub-score is clamped to.
///
/// Equivalent to `LogOddsFusionScorer.CLAMP_MIN`.
const CLAMP_MIN: f32 = 1e-7;

/// The highest probability a sub-score is clamped to.
///
/// Equivalent to `LogOddsFusionScorer.CLAMP_MAX`.
const CLAMP_MAX: f32 = 1.0 - 1e-7;

/// Clamps a probability into `(0, 1)`.
///
/// Equivalent to `LogOddsFusionScorer.clampProbability(float)`, which is
/// `Math.clamp(p, CLAMP_MIN, CLAMP_MAX)` — and therefore propagates `NaN`.
pub fn clamp_probability(p: f32) -> f32 {
    if p.is_nan() {
        p
    } else if p < CLAMP_MIN {
        CLAMP_MIN
    } else if p > CLAMP_MAX {
        CLAMP_MAX
    } else {
        p
    }
}

/// The log-odds of a probability: `log(p / (1 - p))`.
///
/// Equivalent to `LogOddsFusionScorer.logit(float)`.
pub fn logit(p: f32) -> f32 {
    let clamped = clamp_probability(p);
    ((clamped as f64) / (1.0 - clamped as f64)).ln() as f32
}

/// The logistic function, computed in the numerically stable direction.
///
/// Equivalent to `LogOddsFusionScorer.sigmoid(float)`.
pub fn sigmoid(x: f32) -> f32 {
    if x >= 0.0 {
        (1.0 / (1.0 + (-(x as f64)).exp())) as f32
    } else {
        let exp_x = (x as f64).exp();
        (exp_x / (1.0 + exp_x)) as f32
    }
}

/// The softplus function, `log(1 + exp(x))`.
///
/// Equivalent to `LogOddsFusionScorer.softplus(float)`. It is always positive
/// and is a smooth approximation of ReLU: for large positive `x` it approaches
/// `x`, for large negative `x` it approaches `0` from above, and at `x = 0` it
/// returns `log(2) ~ 0.693`. The formulation is numerically stable: above 20 it
/// simply returns `x`.
pub fn softplus(x: f32) -> f32 {
    if x > 20.0 {
        return x;
    }
    (x as f64).exp().ln_1p() as f32
}

/// The scorer of a [`LogOddsFusionQuery`](crate::search::LogOddsFusionQuery).
///
/// Equivalent to the package-private `final class
/// org.apache.lucene.search.LogOddsFusionScorer`, which extends
/// `DisjunctionScorer`; it is `pub` here because the port has no package
/// visibility, and it holds a [`DisjunctionScorer`] rather than extending one.
///
/// It combines the sub-scorer outputs — assumed to be probabilities in
/// `(0, 1)` — through log-odds fusion with multiplicative confidence scaling:
///
/// ```text
/// gatedLogit  = softplus(logit(clamp(subScore)))   for each matching sub-scorer
/// logitSum    = sum of gatedLogit values
/// meanLogit   = logitSum / n   (n = total clause count, not just matching)
/// scaledLogit = meanLogit * pow(n, alpha)
/// score       = sigmoid(scaledLogit)
/// ```
///
/// Softplus gating distinguishes "absence of evidence" — a non-matching
/// sub-scorer, which contributes `0` — from "evidence of absence" — a matching
/// sub-scorer with a weak probability, which contributes a small positive
/// value. A matching sub-scorer therefore always contributes more than a
/// non-matching one, which preserves the ordering among weak matches while
/// never penalising a match.
#[derive(Debug)]
pub struct LogOddsFusionScorer {
    base: DisjunctionScorer,
    total_clauses: i32,
    scaling_factor: f32,
    signal_weights: Option<Vec<f32>>,
    logit_min: Option<Vec<f32>>,
    logit_max: Option<Vec<f32>>,
    /// Maps a position handed out by
    /// [`DisjunctionScorer::sub_matches`] — an index into the cost-sorted
    /// clause array — back to the clause's index in the supplied list.
    ///
    /// **Divergence from Lucene 10.5.0.** Java keeps an
    /// `IdentityHashMap<Scorer, Integer>` because it can compare scorer object
    /// identities. Rust has positions instead of identities, so the mapping is
    /// an array; it answers exactly the same index.
    sorted_to_original: Vec<usize>,
    disjunction_block_propagator: Option<DisjunctionScoreBlockBoundaryPropagator>,
}

impl LogOddsFusionScorer {
    /// Creates the scorer.
    ///
    /// Equivalent to
    /// `LogOddsFusionScorer(List<Scorer>, int, float, float[], float[], float[], ScoreMode, long)`.
    ///
    /// * `total_clauses` — the total number of clauses, including the
    ///   non-matching ones;
    /// * `alpha` — the confidence scaling exponent (`0.5` is the `sqrt(n)`
    ///   law);
    /// * `signal_weights` — per-signal weights parallel to `sub_scorers`, or
    ///   `None` for uniform weighting. When provided, the scoring formula uses
    ///   a weighted sum instead of the mean; the weights must be non-negative
    ///   and should sum to 1;
    /// * `logit_min` / `logit_max` — per-signal logit bounds for
    ///   normalisation, or `None` to use softplus gating. When provided, logit
    ///   values are normalised to `[0, 1]` instead, which keeps the
    ///   contributions non-negative while preserving the learned signal scale.
    ///
    /// # Errors
    ///
    /// Propagates the errors of [`DisjunctionScorer::new`] and of the
    /// block-boundary propagator.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        sub_scorers: Vec<Box<dyn Scorer>>,
        total_clauses: i32,
        alpha: f32,
        signal_weights: Option<Vec<f32>>,
        logit_min: Option<Vec<f32>>,
        logit_max: Option<Vec<f32>>,
        score_mode: ScoreMode,
        lead_cost: i64,
    ) -> Result<Self> {
        let num_scorers = sub_scorers.len();
        let mut base = DisjunctionScorer::new(sub_scorers, score_mode, lead_cost)?;
        let mut sorted_to_original = vec![0usize; num_scorers];
        for (original, sorted) in base.approximation().original_order().iter().enumerate() {
            sorted_to_original[*sorted] = original;
        }
        let disjunction_block_propagator = if score_mode == ScoreMode::TOP_SCORES {
            Some(DisjunctionScoreBlockBoundaryPropagator::new(&mut base)?)
        } else {
            None
        };
        Ok(Self {
            base,
            total_clauses,
            scaling_factor: (total_clauses as f64).powf(alpha as f64) as f32,
            signal_weights,
            logit_min,
            logit_max,
            sorted_to_original,
            disjunction_block_propagator,
        })
    }

    /// Applies gating to a logit value: normalisation when bounds are set, and
    /// softplus otherwise.
    ///
    /// Equivalent to the private
    /// `LogOddsFusionScorer.gateLogit(float, int)`.
    fn gate_logit(&self, raw_logit: f32, signal_index: usize) -> f32 {
        if let (Some(min), Some(max)) = (&self.logit_min, &self.logit_max) {
            let range = max[signal_index] - min[signal_index];
            if range > 0.0 {
                let normalised = (raw_logit - min[signal_index]) / range;
                return if normalised.is_nan() {
                    normalised
                } else if normalised < 0.0 {
                    0.0
                } else if normalised > 1.0 {
                    1.0
                } else {
                    normalised
                };
            }
            return 0.5;
        }
        softplus(raw_logit)
    }

    /// Turns a sum of gated logits into a score.
    ///
    /// Equivalent to the tail of `LogOddsFusionScorer.score(DisiWrapper)` and
    /// of `getMaxScore(int)`, which are the same three lines.
    fn fuse(&self, logit_sum: f64) -> f32 {
        let scaled_logit = if self.signal_weights.is_some() {
            logit_sum as f32 * self.scaling_factor
        } else {
            (logit_sum / self.total_clauses as f64) as f32 * self.scaling_factor
        };
        sigmoid(scaled_logit)
    }
}

impl DocIdSetIterator for LogOddsFusionScorer {
    fn doc_id(&self) -> i32 {
        self.base.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.base.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.base.advance(target)
    }

    fn cost(&self) -> i64 {
        self.base.cost()
    }
}

impl Scorable for LogOddsFusionScorer {
    fn score(&mut self) -> Result<f32> {
        let mut logit_sum = 0f64;
        for position in self.base.sub_matches()? {
            let sub_score = self.base.wrapper(position).scorable().score()?;
            let idx = self.sorted_to_original[position];
            let gated = match &self.signal_weights {
                // Java reads the index out of an identity map and falls back to
                // `0` when the map is absent, so an unweighted fusion always
                // gates against signal `0`.
                Some(_) => self.gate_logit(logit(sub_score), idx),
                None => self.gate_logit(logit(sub_score), 0),
            };
            match &self.signal_weights {
                Some(weights) => logit_sum += f64::from(weights[idx] * gated),
                None => logit_sum += f64::from(gated),
            }
        }
        // Non-matching sub-scorers contribute 0. With weights,
        // `sum(w_i * gated_i)` already accounts for the `1/n` factor; without
        // them, dividing by total_clauses computes the mean.
        Ok(self.fuse(logit_sum))
    }

    fn set_min_competitive_score(&mut self, min_score: f32) -> Result<()> {
        if let Some(propagator) = self.disjunction_block_propagator.as_mut() {
            propagator.set_min_competitive_score(min_score);
        }
        Ok(())
    }

    fn children(&mut self) -> Result<Vec<ChildScorable<'_>>> {
        self.base.children()
    }
}

impl Scorer for LogOddsFusionScorer {
    fn as_scorable(&mut self) -> &mut dyn Scorable {
        self
    }

    fn doc_id(&self) -> i32 {
        self.base.doc_id()
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        self
    }

    fn two_phase_iterator(&mut self) -> Option<&mut dyn TwoPhaseIterator> {
        if self.base.has_two_phase() {
            Some(&mut self.base)
        } else {
            None
        }
    }

    fn advance_shallow(&mut self, target: i32) -> Result<i32> {
        match self.disjunction_block_propagator.as_mut() {
            Some(propagator) => propagator.advance_shallow(&mut self.base, target),
            // `super.advanceShallow(int)` is `Scorer`'s default.
            None => Ok(NO_MORE_DOCS),
        }
    }

    fn get_max_score(&mut self, up_to: i32) -> Result<f32> {
        // A safe upper bound: gate_logit is monotone in p — softplus and the
        // normalisation both are — the weights are non-negative, a sum of upper
        // bounds is at least the sum of the actual values, and sigmoid is
        // monotone.
        let mut max_logit_sum = 0f64;
        let num_clauses = self.base.num_clauses();
        for i in 0..num_clauses {
            let scorer = self.base.approximation().sub_scorer(i).scorer();
            if Scorer::doc_id(scorer) <= up_to {
                let max_sub_score = scorer.get_max_score(up_to)?;
                let gated = self.gate_logit(logit(max_sub_score), i);
                match &self.signal_weights {
                    Some(weights) => max_logit_sum += f64::from(weights[i] * gated),
                    None => max_logit_sum += f64::from(gated),
                }
            }
        }
        Ok(self.fuse(max_logit_sum))
    }
}
