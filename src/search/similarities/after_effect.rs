//! First normalization of the informative content for the DFR framework,
//! ported from `org.apache.lucene.search.similarities.AfterEffect` and its two
//! implementations.

#![deny(unsafe_code)]

use std::fmt::{self, Debug};

use super::{BasicStats, Explanation};

/// Base of the implementations of the *first normalization of the informative
/// content* in the DFR framework.
///
/// Equivalent to `org.apache.lucene.search.similarities.AfterEffect`. This
/// component is also called the *after effect* and is defined by
/// *Inf₂ = 1 - Prob₂*, where *Prob₂* measures the information gain.
///
/// See [`DFRSimilarity`](super::DFRSimilarity).
pub trait AfterEffect: fmt::Display + Debug + Send + Sync {
    /// Returns the name this after effect puts into its explanations.
    ///
    /// Equivalent to `getClass().getSimpleName()`.
    fn simple_name(&self) -> &'static str;

    /// Returns the product of the after effect with `1 + tfn`.
    ///
    /// Equivalent to `AfterEffect.scoreTimes1pTfn(BasicStats)`
    /// (`AfterEffect.java:36`). As the Java signature makes explicit, this
    /// product may not depend on `tfn`, which is what lets
    /// [`DFRSimilarity`](super::DFRSimilarity) hoist it out of the basic
    /// model's rearranged formula.
    fn score_times_1p_tfn(&self, stats: &BasicStats) -> f64;

    /// Returns an explanation for the score.
    ///
    /// Equivalent to `AfterEffect.explain(BasicStats, double)`
    /// (`AfterEffect.java:39`), which reports the after effect itself —
    /// `score_times_1p_tfn / (1 + tfn)`.
    fn explain(&self, stats: &BasicStats, tfn: f64) -> Explanation;
}

/// Model of the information gain based on the ratio of two Bernoulli processes.
///
/// Equivalent to `org.apache.lucene.search.similarities.AfterEffectB`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AfterEffectB;

impl AfterEffectB {
    /// Creates the parameter-free after effect.
    ///
    /// Equivalent to `new AfterEffectB()`.
    pub fn new() -> Self {
        Self
    }
}

impl AfterEffect for AfterEffectB {
    fn simple_name(&self) -> &'static str {
        "AfterEffectB"
    }

    fn score_times_1p_tfn(&self, stats: &BasicStats) -> f64 {
        // Both increments are `long` arithmetic in Java; only `F + 1.0` widens.
        let f = stats.total_term_freq().wrapping_add(1);
        let n = stats.doc_freq().wrapping_add(1);
        (f as f64 + 1.0) / n as f64
    }

    fn explain(&self, stats: &BasicStats, tfn: f64) -> Explanation {
        Explanation::matched(
            (self.score_times_1p_tfn(stats) / (1.0 + tfn)) as f32,
            format!(
                "{}, computed as (F + 1) / (n * (tfn + 1)) from:",
                self.simple_name()
            ),
            vec![
                Explanation::matched(tfn as f32, "tfn, normalized term frequency", vec![]),
                Explanation::matched(
                    stats.total_term_freq(),
                    "F, total number of occurrences of term across all documents + 1",
                    vec![],
                ),
                Explanation::matched(
                    stats.doc_freq(),
                    "n, number of documents containing term + 1",
                    vec![],
                ),
                // Lucene lists `tfn` twice (`AfterEffectB.java:44-52`); the
                // duplicate is part of the explanation tree it produces.
                Explanation::matched(tfn as f32, "tfn, normalized term frequency", vec![]),
            ],
        )
    }
}

impl fmt::Display for AfterEffectB {
    /// Renders the after-effect code, as `AfterEffectB.toString()` does.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("B")
    }
}

/// Model of the information gain based on Laplace's law of succession.
///
/// Equivalent to `org.apache.lucene.search.similarities.AfterEffectL`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AfterEffectL;

impl AfterEffectL {
    /// Creates the parameter-free after effect.
    ///
    /// Equivalent to `new AfterEffectL()`.
    pub fn new() -> Self {
        Self
    }
}

impl AfterEffect for AfterEffectL {
    fn simple_name(&self) -> &'static str {
        "AfterEffectL"
    }

    fn score_times_1p_tfn(&self, stats: &BasicStats) -> f64 {
        let _ = stats;
        1.0
    }

    fn explain(&self, stats: &BasicStats, tfn: f64) -> Explanation {
        Explanation::matched(
            (self.score_times_1p_tfn(stats) / (1.0 + tfn)) as f32,
            format!("{}, computed as 1 / (tfn + 1) from:", self.simple_name()),
            vec![Explanation::matched(
                tfn as f32,
                "tfn, normalized term frequency",
                vec![],
            )],
        )
    }
}

impl fmt::Display for AfterEffectL {
    /// Renders the after-effect code, as `AfterEffectL.toString()` does.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("L")
    }
}
