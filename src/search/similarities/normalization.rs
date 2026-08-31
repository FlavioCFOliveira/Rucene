//! Term frequency normalization for the DFR and information-based frameworks,
//! ported from `org.apache.lucene.search.similarities.Normalization`, its
//! nested `NoNormalization`, and the four parameterized models.

#![deny(unsafe_code)]

use std::fmt::{self, Debug};

use crate::error::{LuceneError, Result};

use super::java_fmt;
use super::similarity_base::log2;
use super::{BasicStats, Explanation};

/// Base of the term frequency normalization methods in the DFR framework.
///
/// Equivalent to `org.apache.lucene.search.similarities.Normalization`.
///
/// See [`DFRSimilarity`](super::DFRSimilarity).
pub trait Normalization: fmt::Display + Debug + Send + Sync {
    /// Returns the name this normalization puts into its explanations.
    ///
    /// Equivalent to `getClass().getSimpleName()`.
    fn simple_name(&self) -> &'static str;

    /// Returns the normalized term frequency.
    ///
    /// Equivalent to `Normalization.tfn(BasicStats, double, double)`
    /// (`Normalization.java:35`). `len` is the field length.
    fn tfn(&self, stats: &BasicStats, tf: f64, len: f64) -> f64;

    /// Returns an explanation for the normalized term frequency.
    ///
    /// Equivalent to `Normalization.explain(BasicStats, double, double)`
    /// (`Normalization.java:43-53`). The default covers the normalizations that
    /// use only the document's field length and the average field length;
    /// models using other statistics override it, as Lucene's do.
    fn explain(&self, stats: &BasicStats, tf: f64, len: f64) -> Explanation {
        Explanation::matched(
            self.tfn(stats, tf, len) as f32,
            format!("{}, computed from:", self.simple_name()),
            vec![
                Explanation::matched(
                    tf as f32,
                    "tf, number of occurrences of term in the document",
                    vec![],
                ),
                Explanation::matched(
                    stats.avg_field_length() as f32,
                    "avgfl, average length of field across all documents",
                    vec![],
                ),
                Explanation::matched(len as f32, "fl, field length of the document", vec![]),
            ],
        )
    }
}

/// Implementation used when there is no normalization.
///
/// Equivalent to the nested `Normalization.NoNormalization`
/// (`Normalization.java:56-74`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoNormalization;

impl NoNormalization {
    /// Creates the parameter-free normalization.
    ///
    /// Equivalent to `new Normalization.NoNormalization()`.
    pub fn new() -> Self {
        Self
    }
}

impl Normalization for NoNormalization {
    fn simple_name(&self) -> &'static str {
        "NoNormalization"
    }

    fn tfn(&self, stats: &BasicStats, tf: f64, len: f64) -> f64 {
        let _ = (stats, len);
        tf
    }

    fn explain(&self, stats: &BasicStats, tf: f64, len: f64) -> Explanation {
        let _ = (stats, tf, len);
        // Java passes the `int` literal 1, which renders as `1` and not `1.0`.
        Explanation::matched(1, "no normalization", vec![])
    }
}

impl fmt::Display for NoNormalization {
    /// Renders the empty normalization code, as
    /// `Normalization.NoNormalization.toString()` does.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("")
    }
}

/// Normalization model that assumes a uniform distribution of the term
/// frequency.
///
/// Equivalent to `org.apache.lucene.search.similarities.NormalizationH1`. The
/// model is parameterless in the original article; the information-based models
/// (see [`IBSimilarity`](super::IBSimilarity)) introduced the multiplying
/// factor `c`, whose default is `1`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NormalizationH1 {
    c: f32,
}

impl NormalizationH1 {
    /// Creates the normalization with the supplied parameter `c`.
    ///
    /// Equivalent to `new NormalizationH1(float)`
    /// (`NormalizationH1.java:42-49`). `c` is unbounded above, but typically
    /// lies in `0..10`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `c` is not finite or is
    /// negative, with the message Java's `IllegalArgumentException` carries.
    pub fn with_c(c: f32) -> Result<Self> {
        if !c.is_finite() || c < 0.0 {
            return Err(LuceneError::IllegalArgument(format!(
                "illegal c value: {}, must be a non-negative finite value",
                java_fmt::float_to_string(c)
            )));
        }
        Ok(Self { c })
    }

    /// Returns the `c` parameter.
    ///
    /// Equivalent to `NormalizationH1.getC()`.
    pub fn c(&self) -> f32 {
        self.c
    }
}

impl Default for NormalizationH1 {
    /// Equivalent to `new NormalizationH1()`, which calls
    /// `NormalizationH1(1)` (`NormalizationH1.java:52-54`).
    fn default() -> Self {
        Self { c: 1.0 }
    }
}

impl Normalization for NormalizationH1 {
    fn simple_name(&self) -> &'static str {
        "NormalizationH1"
    }

    fn tfn(&self, stats: &BasicStats, tf: f64, len: f64) -> f64 {
        // Java widens `c` to `double` before the multiplication.
        tf * f64::from(self.c) * (stats.avg_field_length() / len)
    }

    fn explain(&self, stats: &BasicStats, tf: f64, len: f64) -> Explanation {
        Explanation::matched(
            self.tfn(stats, tf, len) as f32,
            format!(
                "{}, computed as tf * c * (avgfl / fl) from:",
                self.simple_name()
            ),
            vec![
                Explanation::matched(
                    tf as f32,
                    "tf, number of occurrences of term in the document",
                    vec![],
                ),
                Explanation::matched(self.c, "c, hyper-parameter", vec![]),
                Explanation::matched(
                    stats.avg_field_length() as f32,
                    "avgfl, average length of field across all documents",
                    vec![],
                ),
                Explanation::matched(len as f32, "fl, field length of the document", vec![]),
            ],
        )
    }
}

impl fmt::Display for NormalizationH1 {
    /// Renders the normalization code, as `NormalizationH1.toString()` does.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("1")
    }
}

/// Normalization model in which the term frequency is inversely related to the
/// length.
///
/// Equivalent to `org.apache.lucene.search.similarities.NormalizationH2`. The
/// model is parameterless in the original article; the parameterized variant
/// with `c`, whose default is `1`, comes from the thesis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NormalizationH2 {
    c: f32,
}

impl NormalizationH2 {
    /// Creates the normalization with the supplied parameter `c`.
    ///
    /// Equivalent to `new NormalizationH2(float)`
    /// (`NormalizationH2.java:43-50`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `c` is not finite or is
    /// negative.
    pub fn with_c(c: f32) -> Result<Self> {
        if !c.is_finite() || c < 0.0 {
            return Err(LuceneError::IllegalArgument(format!(
                "illegal c value: {}, must be a non-negative finite value",
                java_fmt::float_to_string(c)
            )));
        }
        Ok(Self { c })
    }

    /// Returns the `c` parameter.
    ///
    /// Equivalent to `NormalizationH2.getC()`.
    pub fn c(&self) -> f32 {
        self.c
    }
}

impl Default for NormalizationH2 {
    /// Equivalent to `new NormalizationH2()`, which calls
    /// `NormalizationH2(1)`.
    fn default() -> Self {
        Self { c: 1.0 }
    }
}

impl Normalization for NormalizationH2 {
    fn simple_name(&self) -> &'static str {
        "NormalizationH2"
    }

    fn tfn(&self, stats: &BasicStats, tf: f64, len: f64) -> f64 {
        tf * log2(1.0 + f64::from(self.c) * stats.avg_field_length() / len)
    }

    fn explain(&self, stats: &BasicStats, tf: f64, len: f64) -> Explanation {
        Explanation::matched(
            self.tfn(stats, tf, len) as f32,
            format!(
                "{}, computed as tf * log2(1 + c * avgfl / fl) from:",
                self.simple_name()
            ),
            vec![
                Explanation::matched(
                    tf as f32,
                    "tf, number of occurrences of term in the document",
                    vec![],
                ),
                Explanation::matched(self.c, "c, hyper-parameter", vec![]),
                Explanation::matched(
                    stats.avg_field_length() as f32,
                    "avgfl, average length of field across all documents",
                    vec![],
                ),
                Explanation::matched(len as f32, "fl, field length of the document", vec![]),
            ],
        )
    }
}

impl fmt::Display for NormalizationH2 {
    /// Renders the normalization code, as `NormalizationH2.toString()` does.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("2")
    }
}

/// Dirichlet priors normalization.
///
/// Equivalent to `org.apache.lucene.search.similarities.NormalizationH3`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NormalizationH3 {
    mu: f32,
}

impl NormalizationH3 {
    /// Creates the normalization with the supplied smoothing parameter `mu`.
    ///
    /// Equivalent to `new NormalizationH3(float)`
    /// (`NormalizationH3.java:37-43`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `mu` is not finite or is
    /// negative.
    pub fn with_mu(mu: f32) -> Result<Self> {
        if !mu.is_finite() || mu < 0.0 {
            return Err(LuceneError::IllegalArgument(format!(
                "illegal mu value: {}, must be a non-negative finite value",
                java_fmt::float_to_string(mu)
            )));
        }
        Ok(Self { mu })
    }

    /// Returns the smoothing parameter `mu`.
    ///
    /// Equivalent to `NormalizationH3.getMu()`.
    pub fn mu(&self) -> f32 {
        self.mu
    }
}

impl Default for NormalizationH3 {
    /// Equivalent to `new NormalizationH3()`, which calls
    /// `NormalizationH3(800)` (`NormalizationH3.java:30-32`).
    fn default() -> Self {
        Self { mu: 800.0 }
    }
}

impl Normalization for NormalizationH3 {
    fn simple_name(&self) -> &'static str {
        "NormalizationH3"
    }

    fn tfn(&self, stats: &BasicStats, tf: f64, len: f64) -> f64 {
        // `(F + 1F) / (T + 1F)` and its product with `mu` are `float`
        // expressions in Java — the statistics are `long`s added to a `float`
        // literal — and only then does the sum with `tf` widen to `double`.
        let smoothed = self.mu
            * ((stats.total_term_freq() as f32 + 1.0)
                / (stats.number_of_field_tokens() as f32 + 1.0));
        (tf + f64::from(smoothed)) / (len + f64::from(self.mu)) * f64::from(self.mu)
    }

    fn explain(&self, stats: &BasicStats, tf: f64, len: f64) -> Explanation {
        Explanation::matched(
            self.tfn(stats, tf, len) as f32,
            format!(
                "{}, computed as (tf + mu * ((F+1) / (T+1))) / (fl + mu) * mu from:",
                self.simple_name()
            ),
            vec![
                Explanation::matched(
                    tf as f32,
                    "tf, number of occurrences of term in the document",
                    vec![],
                ),
                Explanation::matched(self.mu, "mu, smoothing parameter", vec![]),
                Explanation::matched(
                    stats.total_term_freq() as f32,
                    "F,  total number of occurrences of term across all documents",
                    vec![],
                ),
                Explanation::matched(
                    stats.number_of_field_tokens() as f32,
                    "T, total number of tokens of the field across all documents",
                    vec![],
                ),
                Explanation::matched(len as f32, "fl, field length of the document", vec![]),
            ],
        )
    }
}

impl fmt::Display for NormalizationH3 {
    /// Renders the normalization code, as `NormalizationH3.toString()` does:
    /// `3(` followed by `Float.toString(mu)` and `)`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "3({})", java_fmt::float_to_string(self.mu))
    }
}

/// Pareto-Zipf normalization.
///
/// Equivalent to `org.apache.lucene.search.similarities.NormalizationZ`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NormalizationZ {
    z: f32,
}

impl NormalizationZ {
    /// Creates the normalization with the supplied parameter `z`.
    ///
    /// Equivalent to `new NormalizationZ(float)`
    /// (`NormalizationZ.java:38-44`). `z` represents `A / (A + 1)`, where `A`
    /// measures the specificity of the language, and ranges over `(0 .. 0.5)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `z` is NaN or outside the
    /// open interval `(0, 0.5)`.
    pub fn with_z(z: f32) -> Result<Self> {
        if z.is_nan() || z <= 0.0 || z >= 0.5 {
            return Err(LuceneError::IllegalArgument(format!(
                "illegal z value: {}, must be in the range (0 .. 0.5)",
                java_fmt::float_to_string(z)
            )));
        }
        Ok(Self { z })
    }

    /// Returns the `z` parameter.
    ///
    /// Equivalent to `NormalizationZ.getZ()`.
    pub fn z(&self) -> f32 {
        self.z
    }
}

impl Default for NormalizationZ {
    /// Equivalent to `new NormalizationZ()`, which calls
    /// `NormalizationZ(0.3)` (`NormalizationZ.java:30-32`).
    fn default() -> Self {
        Self { z: 0.30 }
    }
}

impl Normalization for NormalizationZ {
    fn simple_name(&self) -> &'static str {
        "NormalizationZ"
    }

    fn tfn(&self, stats: &BasicStats, tf: f64, len: f64) -> f64 {
        tf * (stats.avg_field_length() / len).powf(f64::from(self.z))
    }

    fn explain(&self, stats: &BasicStats, tf: f64, len: f64) -> Explanation {
        Explanation::matched(
            self.tfn(stats, tf, len) as f32,
            format!(
                "{}, computed as tf * Math.pow(avgfl / fl, z) from:",
                self.simple_name()
            ),
            vec![
                Explanation::matched(
                    tf as f32,
                    "tf, number of occurrences of term in the document",
                    vec![],
                ),
                Explanation::matched(
                    stats.avg_field_length() as f32,
                    "avgfl, average length of field across all documents",
                    vec![],
                ),
                Explanation::matched(len as f32, "fl, field length of the document", vec![]),
                Explanation::matched(self.z, "z, relates to specificity of the language", vec![]),
            ],
        )
    }
}

impl fmt::Display for NormalizationZ {
    /// Renders the normalization code, as `NormalizationZ.toString()` does:
    /// `Z(` followed by `Float.toString(z)` and `)`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Z({})", java_fmt::float_to_string(self.z))
    }
}
