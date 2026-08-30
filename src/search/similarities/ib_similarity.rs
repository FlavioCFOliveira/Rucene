//! The information-based framework, ported from
//! `org.apache.lucene.search.similarities.IBSimilarity`.

#![deny(unsafe_code)]

use std::fmt;

use super::similarity_base::similarity_base_scorer;
use super::{
    BasicStats, CollectionStatistics, Distribution, Explanation, Lambda, Normalization, SimScorer,
    Similarity, SimilarityBase, TermStatistics,
};

/// Provides a framework for the family of information-based models.
///
/// Equivalent to `org.apache.lucene.search.similarities.IBSimilarity`.
/// Described in Stéphane Clinchant and Eric Gaussier, *Information-based models
/// for ad hoc IR*, SIGIR '10, pp. 234-241.
///
/// The retrieval function has the form
/// *RSV(q, d) = Σ -x^q_w log Prob(X_w ≥ t^d_w | λ_w)*, where *x^q_w* is the
/// query boost, *X_w* counts the occurrences of word *w*, *t^d_w* is the
/// normalized term frequency and *λ_w* is a parameter.
///
/// The framework has many similarities to the DFR framework (see
/// [`DFRSimilarity`](super::DFRSimilarity)); the two may be merged one day.
/// Constructing one requires all three components:
///
/// 1. [`Distribution`] — [`DistributionLL`](super::DistributionLL)
///    (log-logistic) or [`DistributionSPL`](super::DistributionSPL) (smoothed
///    power-law);
/// 2. [`Lambda`] — [`LambdaDF`](super::LambdaDF) (average number of documents
///    where *w* occurs) or [`LambdaTTF`](super::LambdaTTF) (average number of
///    occurrences of *w* in the collection);
/// 3. [`Normalization`] — any of the DFR normalizations.
#[derive(Debug)]
pub struct IBSimilarity {
    distribution: Box<dyn Distribution>,
    lambda: Box<dyn Lambda>,
    normalization: Box<dyn Normalization>,
    discount_overlaps: bool,
}

impl IBSimilarity {
    /// Creates an `IBSimilarity` from the three components, discounting
    /// overlaps.
    ///
    /// Equivalent to `new IBSimilarity(Distribution, Lambda, Normalization)`
    /// (`IBSimilarity.java:74-76`). Pass
    /// [`NoNormalization`](super::NoNormalization) for no normalization.
    pub fn new(
        distribution: Box<dyn Distribution>,
        lambda: Box<dyn Lambda>,
        normalization: Box<dyn Normalization>,
    ) -> Self {
        Self::with_discount_overlaps(distribution, lambda, normalization, true)
    }

    /// Creates an `IBSimilarity` from the three components with an explicit
    /// `discount_overlaps`.
    ///
    /// Equivalent to
    /// `new IBSimilarity(Distribution, Lambda, Normalization, boolean)`
    /// (`IBSimilarity.java:89-99`).
    pub fn with_discount_overlaps(
        distribution: Box<dyn Distribution>,
        lambda: Box<dyn Lambda>,
        normalization: Box<dyn Normalization>,
        discount_overlaps: bool,
    ) -> Self {
        Self {
            distribution,
            lambda,
            normalization,
            discount_overlaps,
        }
    }

    /// Returns the distribution.
    ///
    /// Equivalent to `IBSimilarity.getDistribution()`.
    pub fn distribution(&self) -> &dyn Distribution {
        self.distribution.as_ref()
    }

    /// Returns the distribution's lambda parameter.
    ///
    /// Equivalent to `IBSimilarity.getLambda()`.
    pub fn lambda(&self) -> &dyn Lambda {
        self.lambda.as_ref()
    }

    /// Returns the term frequency normalization.
    ///
    /// Equivalent to `IBSimilarity.getNormalization()`.
    pub fn normalization(&self) -> &dyn Normalization {
        self.normalization.as_ref()
    }
}

impl Similarity for IBSimilarity {
    fn discount_overlaps(&self) -> bool {
        self.discount_overlaps
    }

    fn scorer<'a>(
        &'a self,
        boost: f32,
        collection_stats: &CollectionStatistics,
        term_stats: &[TermStatistics],
    ) -> Box<dyn SimScorer + 'a> {
        similarity_base_scorer(self, boost, collection_stats, term_stats)
    }
}

impl SimilarityBase for IBSimilarity {
    type Stats = BasicStats;

    fn simple_name(&self) -> &'static str {
        "IBSimilarity"
    }

    fn score(&self, stats: &BasicStats, freq: f64, doc_len: f64) -> f64 {
        stats.boost()
            * self.distribution.score(
                stats,
                self.normalization.tfn(stats, freq, doc_len),
                f64::from(self.lambda.lambda(stats)),
            )
    }

    fn explain_details(
        &self,
        subs: &mut Vec<Explanation>,
        stats: &BasicStats,
        freq: f64,
        doc_len: f64,
    ) {
        if stats.boost() != 1.0 {
            subs.push(Explanation::matched(
                stats.boost() as f32,
                "boost, query boost",
                vec![],
            ));
        }
        let norm_expl = self.normalization.explain(stats, freq, doc_len);
        let lambda_expl = self.lambda.explain(stats);
        // Java feeds the distribution the values taken *from the two
        // explanations*, narrowed to `float`, rather than the raw `double`s the
        // score uses.
        let distribution_expl = self.distribution.explain(
            stats,
            f64::from(norm_expl.value().float_value()),
            f64::from(lambda_expl.value().float_value()),
        );
        subs.push(norm_expl);
        subs.push(lambda_expl);
        subs.push(distribution_expl);
    }

    /// Overrides the base explanation, as `IBSimilarity.explain(BasicStats,
    /// Explanation, double)` does (`IBSimilarity.java:132-147`). Like
    /// [`DFRSimilarity`](super::DFRSimilarity), it takes the frequency as a
    /// `double` rather than narrowing it to a `float` first.
    fn explain(&self, stats: &BasicStats, freq: &Explanation, doc_len: f64) -> Explanation {
        let mut subs = Vec::new();
        let freq_value = freq.value().double_value();
        self.explain_details(&mut subs, stats, freq_value, doc_len);

        Explanation::matched(
            self.score(stats, freq_value, doc_len) as f32,
            format!(
                "score({}, freq={}), computed as boost * distribution.score(stats, normalization.tfn(stats, freq, docLen), lambda.lambda(stats)) from:",
                self.simple_name(),
                freq.value()
            ),
            subs,
        )
    }
}

impl fmt::Display for IBSimilarity {
    /// Renders the similarity as `IBSimilarity.toString()` does, following the
    /// pattern `IB <distribution>-<lambda><normalization>`. The distribution
    /// name is the one from the original paper; the lambda codes are documented
    /// on the [`Lambda`] implementations.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "IB {}-{}{}",
            self.distribution, self.lambda, self.normalization
        )
    }
}
