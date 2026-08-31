//! The *λ_w* parameter of the information-based framework, ported from
//! `org.apache.lucene.search.similarities.Lambda` and its two implementations.

#![deny(unsafe_code)]

use std::fmt::{self, Debug};

use super::java_math;
use super::{BasicStats, Explanation};

/// The *lambda (λ_w)* parameter in information-based models.
///
/// Equivalent to `org.apache.lucene.search.similarities.Lambda`.
///
/// See [`IBSimilarity`](super::IBSimilarity).
pub trait Lambda: fmt::Display + Debug + Send + Sync {
    /// Returns the name this parameter puts into its explanation.
    ///
    /// Equivalent to `getClass().getSimpleName()`.
    fn simple_name(&self) -> &'static str;

    /// Computes the lambda parameter.
    ///
    /// Equivalent to `Lambda.lambda(BasicStats)` (`Lambda.java:30`). The
    /// result is a `float`, and [`IBSimilarity`](super::IBSimilarity) widens it
    /// to a `double` at the call site.
    fn lambda(&self, stats: &BasicStats) -> f32;

    /// Explains the lambda parameter.
    ///
    /// Equivalent to `Lambda.explain(BasicStats)` (`Lambda.java:33`).
    fn explain(&self, stats: &BasicStats) -> Explanation;
}

/// Computes lambda as `(docFreq + 1) / (numberOfDocuments + 1)`.
///
/// Equivalent to `org.apache.lucene.search.similarities.LambdaDF`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LambdaDF;

impl LambdaDF {
    /// Creates the parameter-free lambda.
    ///
    /// Equivalent to `new LambdaDF()`.
    pub fn new() -> Self {
        Self
    }
}

impl Lambda for LambdaDF {
    fn simple_name(&self) -> &'static str {
        "LambdaDF"
    }

    fn lambda(&self, stats: &BasicStats) -> f32 {
        let lambda =
            ((stats.doc_freq() as f64 + 1.0) / (stats.number_of_documents() as f64 + 1.0)) as f32;
        if lambda == 1.0 {
            // `DistributionSPL` divides by `1 - lambda`, so a lambda of exactly
            // one is nudged off the singularity.
            java_math::next_down_f32(lambda)
        } else {
            lambda
        }
    }

    fn explain(&self, stats: &BasicStats) -> Explanation {
        Explanation::matched(
            self.lambda(stats),
            format!(
                "{}, computed as (n + 1) / (N + 1) from:",
                self.simple_name()
            ),
            vec![
                Explanation::matched(
                    stats.doc_freq(),
                    "n, number of documents containing term",
                    vec![],
                ),
                Explanation::matched(
                    stats.number_of_documents(),
                    "N, total number of documents with field",
                    vec![],
                ),
            ],
        )
    }
}

impl fmt::Display for LambdaDF {
    /// Renders the lambda code, as `LambdaDF.toString()` does. The codes were
    /// chosen arbitrarily, because the original paper is unclear on the matter
    /// and misuses the DFR naming scheme.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("D")
    }
}

/// Computes lambda as `(totalTermFreq + 1) / (numberOfDocuments + 1)`.
///
/// Equivalent to `org.apache.lucene.search.similarities.LambdaTTF`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LambdaTTF;

impl LambdaTTF {
    /// Creates the parameter-free lambda.
    ///
    /// Equivalent to `new LambdaTTF()`.
    pub fn new() -> Self {
        Self
    }
}

impl Lambda for LambdaTTF {
    fn simple_name(&self) -> &'static str {
        "LambdaTTF"
    }

    fn lambda(&self, stats: &BasicStats) -> f32 {
        let lambda = ((stats.total_term_freq() as f64 + 1.0)
            / (stats.number_of_documents() as f64 + 1.0)) as f32;
        if lambda == 1.0 {
            // Nudged in the opposite direction from `LambdaDF`, as in Lucene.
            java_math::next_up_f32(lambda)
        } else {
            lambda
        }
    }

    fn explain(&self, stats: &BasicStats) -> Explanation {
        Explanation::matched(
            self.lambda(stats),
            format!(
                "{}, computed as (F + 1) / (N + 1) from:",
                self.simple_name()
            ),
            vec![
                Explanation::matched(
                    stats.total_term_freq(),
                    "F, total number of occurrences of term across all documents",
                    vec![],
                ),
                Explanation::matched(
                    stats.number_of_documents(),
                    "N, total number of documents with field",
                    vec![],
                ),
            ],
        )
    }
}

impl fmt::Display for LambdaTTF {
    /// Renders the lambda code, as `LambdaTTF.toString()` does.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("L")
    }
}
