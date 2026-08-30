//! Probabilistic distributions for the information-based framework, ported from
//! `org.apache.lucene.search.similarities.Distribution` and its two
//! implementations.

#![deny(unsafe_code)]

use std::fmt::{self, Debug};

use super::java_math;
use super::{BasicStats, Explanation};

/// The probabilistic distribution used to model term occurrence in
/// information-based models.
///
/// Equivalent to `org.apache.lucene.search.similarities.Distribution`.
///
/// See [`IBSimilarity`](super::IBSimilarity).
pub trait Distribution: fmt::Display + Debug + Send + Sync {
    /// Returns the name this distribution puts into its explanation.
    ///
    /// Equivalent to `getClass().getSimpleName()`.
    fn simple_name(&self) -> &'static str;

    /// Computes the score.
    ///
    /// Equivalent to `Distribution.score(BasicStats, double, double)`
    /// (`Distribution.java:29`).
    fn score(&self, stats: &BasicStats, tfn: f64, lambda: f64) -> f64;

    /// Explains the score.
    ///
    /// Equivalent to `Distribution.explain(BasicStats, double, double)`
    /// (`Distribution.java:35-37`), which reports the model's name only: both
    /// `tfn` and `lambda` are explained elsewhere.
    fn explain(&self, stats: &BasicStats, tfn: f64, lambda: f64) -> Explanation {
        Explanation::matched(
            self.score(stats, tfn, lambda) as f32,
            self.simple_name(),
            vec![],
        )
    }
}

/// Log-logistic distribution.
///
/// Equivalent to `org.apache.lucene.search.similarities.DistributionLL`.
/// Unlike the DFR family, the natural logarithm is used: it is faster to
/// compute, and the original paper expresses no preference for a base.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DistributionLL;

impl DistributionLL {
    /// Creates the parameter-free distribution.
    ///
    /// Equivalent to `new DistributionLL()`.
    pub fn new() -> Self {
        Self
    }
}

impl Distribution for DistributionLL {
    fn simple_name(&self) -> &'static str {
        "DistributionLL"
    }

    fn score(&self, stats: &BasicStats, tfn: f64, lambda: f64) -> f64 {
        let _ = stats;
        -(lambda / (tfn + lambda)).ln()
    }
}

impl fmt::Display for DistributionLL {
    /// Renders the distribution name, as `DistributionLL.toString()` does.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LL")
    }
}

/// The smoothed power-law (SPL) distribution described in the original paper on
/// the information-based framework.
///
/// Equivalent to `org.apache.lucene.search.similarities.DistributionSPL`.
/// Unlike the DFR family, the natural logarithm is used.
///
/// As Lucene warns, this model returns infinite scores for very small term
/// frequencies and negative scores for very large ones.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DistributionSPL;

impl DistributionSPL {
    /// Creates the parameter-free distribution.
    ///
    /// Equivalent to `new DistributionSPL()`.
    pub fn new() -> Self {
        Self
    }
}

impl Distribution for DistributionSPL {
    fn simple_name(&self) -> &'static str {
        "DistributionSPL"
    }

    /// Scores a document.
    ///
    /// Equivalent to `DistributionSPL.score` (`DistributionSPL.java:33-58`).
    /// Java asserts `lambda != 1`, which the [`Lambda`](super::Lambda)
    /// implementations guarantee by nudging a lambda of exactly `1` off the
    /// singularity; the assertion is disabled in production and is not
    /// reproduced, because a NaN score is a better outcome than a panic if a
    /// custom lambda ever breaks the invariant.
    fn score(&self, stats: &BasicStats, tfn: f64, lambda: f64) -> f64 {
        let _ = stats;
        // tfn / (tfn + 1) rewritten as 1 - 1 / (tfn + 1), which is guaranteed
        // to be non-decreasing as tfn increases.
        let mut q = 1.0 - 1.0 / (tfn + 1.0);
        if q == 1.0 {
            q = java_math::next_down_f64(1.0);
        }

        let mut pow = lambda.powf(q);
        if pow == lambda {
            // Floating-point rounding can make the power equal its base, and
            // the logarithm below would then be infinite, so the two are forced
            // apart in the direction the exact result lies in.
            pow = if lambda < 1.0 {
                // x^y > x when x < 1 and y < 1.
                java_math::next_up_f64(lambda)
            } else {
                // x^y < x when x > 1 and y < 1.
                java_math::next_down_f64(lambda)
            };
        }

        -((pow - lambda) / (1.0 - lambda)).ln()
    }
}

impl fmt::Display for DistributionSPL {
    /// Renders the distribution name, as `DistributionSPL.toString()` does.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SPL")
    }
}
