//! Basic models of information content for the DFR framework, ported from
//! `org.apache.lucene.search.similarities.BasicModel` and its four
//! implementations.

#![deny(unsafe_code)]

use std::fmt::{self, Debug};

use super::similarity_base::log2;
use super::{BasicStats, Explanation};

/// Base of the *basic model* implementations in the DFR framework.
///
/// Equivalent to `org.apache.lucene.search.similarities.BasicModel`. Basic
/// models compute the informative content *Inf₁ = -log₂Prob₁*.
///
/// See [`DFRSimilarity`](super::DFRSimilarity).
pub trait BasicModel: fmt::Display + Debug + Send + Sync {
    /// Returns the name this model puts into its explanations.
    ///
    /// Equivalent to `getClass().getSimpleName()`, which Java resolves
    /// reflectively.
    fn simple_name(&self) -> &'static str;

    /// Returns the informative content score combined with the after effect —
    /// `informationContentScore * ae_times_1p_tfn / (1 + tfn)`.
    ///
    /// Equivalent to `BasicModel.score(BasicStats, double, double)`
    /// (`BasicModel.java:38`). The result must be non-decreasing with `tfn`,
    /// which is why every implementation below is written in the rearranged
    /// form Lucene uses rather than the form in the original paper.
    fn score(&self, stats: &BasicStats, tfn: f64, ae_times_1p_tfn: f64) -> f64;

    /// Returns an explanation for the score.
    ///
    /// Equivalent to `BasicModel.explain(BasicStats, double, double)`
    /// (`BasicModel.java:41`). Note that every implementation reports the
    /// *un-combined* model value, recovered as
    /// `score * (1 + tfn) / ae_times_1p_tfn`.
    fn explain(&self, stats: &BasicStats, tfn: f64, ae_times_1p_tfn: f64) -> Explanation;
}

/// Geometric approximation as the limiting form of the Bose-Einstein model.
///
/// Equivalent to `org.apache.lucene.search.similarities.BasicModelG`. The
/// formula used in Lucene differs slightly from the one in the original paper:
/// `F` is increased by `1` and `N` is increased by `F`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BasicModelG;

impl BasicModelG {
    /// Creates the parameter-free model.
    ///
    /// Equivalent to `new BasicModelG()`.
    pub fn new() -> Self {
        Self
    }

    /// The `lambda = F / (N + F)` both `score` and `explain` start from.
    fn lambda(stats: &BasicStats) -> (f64, f64, f64) {
        // The approximation only holds when F << N, so Lucene uses
        // lambda = F / (N + F) rather than F / N.
        let f = stats.total_term_freq().wrapping_add(1) as f64;
        let n = stats.number_of_documents() as f64;
        (f, n, f / (n + f))
    }
}

impl BasicModel for BasicModelG {
    fn simple_name(&self) -> &'static str {
        "BasicModelG"
    }

    fn score(&self, stats: &BasicStats, tfn: f64, ae_times_1p_tfn: f64) -> f64 {
        let (_, _, lambda) = Self::lambda(stats);
        // -log(1 / (lambda + 1)) -> log(lambda + 1)
        let a = log2(lambda + 1.0);
        let b = log2((1.0 + lambda) / lambda);

        // The model should return (A + B * tfn); Lucene rewrites that to
        // B * (1 + tfn) - (B - A) so that it can be combined with the after
        // effect while staying non-decreasing with tfn, since B >= A.
        (b - (b - a) / (1.0 + tfn)) * ae_times_1p_tfn
    }

    fn explain(&self, stats: &BasicStats, tfn: f64, ae_times_1p_tfn: f64) -> Explanation {
        let (f, n, lambda) = Self::lambda(stats);
        let expl_lambda = Explanation::matched(
            lambda as f32,
            "lambda, computed as F / (N + F) from:",
            vec![
                Explanation::matched(
                    f as f32,
                    "F, total number of occurrences of term across all docs + 1",
                    vec![],
                ),
                Explanation::matched(n as f32, "N, total number of documents with field", vec![]),
            ],
        );

        Explanation::matched(
            (self.score(stats, tfn, ae_times_1p_tfn) * (1.0 + tfn) / ae_times_1p_tfn) as f32,
            format!(
                "{}, computed as log2(lambda + 1) + tfn * log2((1 + lambda) / lambda) from:",
                self.simple_name()
            ),
            vec![
                Explanation::matched(tfn as f32, "tfn, normalized term frequency", vec![]),
                expl_lambda,
            ],
        )
    }
}

impl fmt::Display for BasicModelG {
    /// Renders the model code, as `BasicModelG.toString()` does.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("G")
    }
}

/// An approximation of the *I(nₑ)* model.
///
/// Equivalent to `org.apache.lucene.search.similarities.BasicModelIF`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BasicModelIF;

impl BasicModelIF {
    /// Creates the parameter-free model.
    ///
    /// Equivalent to `new BasicModelIF()`.
    pub fn new() -> Self {
        Self
    }
}

impl BasicModel for BasicModelIF {
    fn simple_name(&self) -> &'static str {
        "BasicModelIF"
    }

    fn score(&self, stats: &BasicStats, tfn: f64, ae_times_1p_tfn: f64) -> f64 {
        let n = stats.number_of_documents();
        let f = stats.total_term_freq();
        // Java adds 1 to N in `long` arithmetic, then divides by a `double`.
        let a = log2(1.0 + n.wrapping_add(1) as f64 / (f as f64 + 0.5));

        // The model should return A * tfn; Lucene rewrites that to
        // A * (1 + tfn) - A so that it can be combined with the after effect
        // while staying non-decreasing with tfn.
        a * ae_times_1p_tfn * (1.0 - 1.0 / (1.0 + tfn))
    }

    fn explain(&self, stats: &BasicStats, tfn: f64, ae_times_1p_tfn: f64) -> Explanation {
        Explanation::matched(
            (self.score(stats, tfn, ae_times_1p_tfn) * (1.0 + tfn) / ae_times_1p_tfn) as f32,
            format!(
                "{}, computed as tfn * log2(1 + (N + 1) / (F + 0.5)) from:",
                self.simple_name()
            ),
            vec![
                Explanation::matched(tfn as f32, "tfn, normalized term frequency", vec![]),
                Explanation::matched(
                    stats.number_of_documents(),
                    "N, total number of documents with field",
                    vec![],
                ),
                Explanation::matched(
                    stats.total_term_freq(),
                    "F, total number of occurrences of term across all documents",
                    vec![],
                ),
            ],
        )
    }
}

impl fmt::Display for BasicModelIF {
    /// Renders the model code, as `BasicModelIF.toString()` does.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("I(F)")
    }
}

/// The basic tf-idf model of randomness.
///
/// Equivalent to `org.apache.lucene.search.similarities.BasicModelIn`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BasicModelIn;

impl BasicModelIn {
    /// Creates the parameter-free model.
    ///
    /// Equivalent to `new BasicModelIn()`.
    pub fn new() -> Self {
        Self
    }
}

impl BasicModel for BasicModelIn {
    fn simple_name(&self) -> &'static str {
        "BasicModelIn"
    }

    fn score(&self, stats: &BasicStats, tfn: f64, ae_times_1p_tfn: f64) -> f64 {
        let big_n = stats.number_of_documents();
        let n = stats.doc_freq();
        let a = log2(big_n.wrapping_add(1) as f64 / (n as f64 + 0.5));

        // Rewritten from A * tfn to A * (1 + tfn) - A, as in `BasicModelIF`.
        a * ae_times_1p_tfn * (1.0 - 1.0 / (1.0 + tfn))
    }

    fn explain(&self, stats: &BasicStats, tfn: f64, ae_times_1p_tfn: f64) -> Explanation {
        Explanation::matched(
            (self.score(stats, tfn, ae_times_1p_tfn) * (1.0 + tfn) / ae_times_1p_tfn) as f32,
            format!(
                "{}, computed as tfn * log2((N + 1) / (n + 0.5)) from:",
                self.simple_name()
            ),
            vec![
                Explanation::matched(tfn as f32, "tfn, normalized term frequency", vec![]),
                Explanation::matched(
                    stats.number_of_documents(),
                    "N, total number of documents with field",
                    vec![],
                ),
                Explanation::matched(
                    stats.doc_freq(),
                    "n, number of documents containing term",
                    vec![],
                ),
            ],
        )
    }
}

impl fmt::Display for BasicModelIn {
    /// Renders the model code, as `BasicModelIn.toString()` does.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("I(n)")
    }
}

/// Tf-idf model of randomness, based on a mixture of Poisson and inverse
/// document frequency.
///
/// Equivalent to `org.apache.lucene.search.similarities.BasicModelIne`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BasicModelIne;

impl BasicModelIne {
    /// Creates the parameter-free model.
    ///
    /// Equivalent to `new BasicModelIne()`.
    pub fn new() -> Self {
        Self
    }
}

impl BasicModel for BasicModelIne {
    fn simple_name(&self) -> &'static str {
        "BasicModelIne"
    }

    fn score(&self, stats: &BasicStats, tfn: f64, ae_times_1p_tfn: f64) -> f64 {
        let n = stats.number_of_documents();
        let f = stats.total_term_freq();
        // `N - 1` is a `long` subtraction here, unlike in `explain` where Java
        // has already widened `N` to a `double`.
        let ne = n as f64 * (1.0 - (n.wrapping_sub(1) as f64 / n as f64).powf(f as f64));
        let a = log2(n.wrapping_add(1) as f64 / (ne + 0.5));

        // Rewritten from A * tfn to A * (1 + tfn) - A, as in `BasicModelIF`.
        a * ae_times_1p_tfn * (1.0 - 1.0 / (1.0 + tfn))
    }

    fn explain(&self, stats: &BasicStats, tfn: f64, ae_times_1p_tfn: f64) -> Explanation {
        let f = stats.total_term_freq() as f64;
        let n = stats.number_of_documents() as f64;
        // Java computes `ne` in `double` here, so `N - 1` is a floating-point
        // subtraction; `score` above does it in `long`. The transcription keeps
        // both as written.
        let ne = n * (1.0 - ((n - 1.0) / n).powf(f));
        let expl_ne = Explanation::matched(
            ne as f32,
            "ne, computed as N * (1 - Math.pow((N - 1) / N, F)) from:",
            vec![
                Explanation::matched(
                    f as f32,
                    "F, total number of occurrences of term across all docs",
                    vec![],
                ),
                Explanation::matched(n as f32, "N, total number of documents with field", vec![]),
            ],
        );

        Explanation::matched(
            (self.score(stats, tfn, ae_times_1p_tfn) * (1.0 + tfn) / ae_times_1p_tfn) as f32,
            format!(
                "{}, computed as tfn * log2((N + 1) / (ne + 0.5)) from:",
                self.simple_name()
            ),
            vec![
                Explanation::matched(tfn as f32, "tfn, normalized term frequency", vec![]),
                expl_ne,
            ],
        )
    }
}

impl fmt::Display for BasicModelIne {
    /// Renders the model code, as `BasicModelIne.toString()` does.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("I(ne)")
    }
}
