//! Divergence-from-independence measures, ported from
//! `org.apache.lucene.search.similarities.Independence` and its three
//! implementations.

#![deny(unsafe_code)]

use std::fmt::{self, Debug};

/// Computes the measure of divergence from independence for DFI scoring
/// functions.
///
/// Equivalent to `org.apache.lucene.search.similarities.Independence`. The
/// three measures are compared in
/// <http://trec.nist.gov/pubs/trec21/papers/irra.web.nb.pdf>.
pub trait Independence: fmt::Display + Debug + Send + Sync {
    /// Computes the distance from independence.
    ///
    /// Equivalent to `Independence.score(double, double)`
    /// (`Independence.java:33`). `freq` is the actual term frequency and
    /// `expected` the expected one.
    fn score(&self, freq: f64, expected: f64) -> f64;
}

/// Normalized chi-squared measure of distance from independence.
///
/// Equivalent to `org.apache.lucene.search.similarities.IndependenceChiSquared`.
/// Described as usable "for tasks that require high precision, against both
/// short and long queries".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IndependenceChiSquared;

impl IndependenceChiSquared {
    /// Creates the measure.
    ///
    /// Equivalent to `new IndependenceChiSquared()`.
    pub fn new() -> Self {
        Self
    }
}

impl Independence for IndependenceChiSquared {
    fn score(&self, freq: f64, expected: f64) -> f64 {
        (freq - expected) * (freq - expected) / expected
    }
}

impl fmt::Display for IndependenceChiSquared {
    /// Renders the measure name, as `IndependenceChiSquared.toString()` does.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ChiSquared")
    }
}

/// Saturated measure of distance from independence.
///
/// Equivalent to `org.apache.lucene.search.similarities.IndependenceSaturated`.
/// Described as being "for tasks that require high recall against long
/// queries".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IndependenceSaturated;

impl IndependenceSaturated {
    /// Creates the measure.
    ///
    /// Equivalent to `new IndependenceSaturated()`.
    pub fn new() -> Self {
        Self
    }
}

impl Independence for IndependenceSaturated {
    fn score(&self, freq: f64, expected: f64) -> f64 {
        (freq - expected) / expected
    }
}

impl fmt::Display for IndependenceSaturated {
    /// Renders the measure name, as `IndependenceSaturated.toString()` does.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Saturated")
    }
}

/// Standardized measure of distance from independence.
///
/// Equivalent to
/// `org.apache.lucene.search.similarities.IndependenceStandardized`. Described
/// as "good at tasks that require high recall and high precision, especially
/// against short queries composed of a few words as in the case of Internet
/// searches".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IndependenceStandardized;

impl IndependenceStandardized {
    /// Creates the measure.
    ///
    /// Equivalent to `new IndependenceStandardized()`.
    pub fn new() -> Self {
        Self
    }
}

impl Independence for IndependenceStandardized {
    fn score(&self, freq: f64, expected: f64) -> f64 {
        (freq - expected) / expected.sqrt()
    }
}

impl fmt::Display for IndependenceStandardized {
    /// Renders the measure name, as `IndependenceStandardized.toString()` does.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Standardized")
    }
}
