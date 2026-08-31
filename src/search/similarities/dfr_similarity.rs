//! The divergence-from-randomness framework, ported from
//! `org.apache.lucene.search.similarities.DFRSimilarity`.

#![deny(unsafe_code)]

use std::fmt;

use super::similarity_base::similarity_base_scorer;
use super::{
    AfterEffect, BasicModel, BasicStats, CollectionStatistics, Explanation, Normalization,
    SimScorer, Similarity, SimilarityBase, TermStatistics,
};

/// Implements the *divergence from randomness (DFR)* framework.
///
/// Equivalent to `org.apache.lucene.search.similarities.DFRSimilarity`.
/// Introduced in Gianni Amati and Cornelis Joost Van Rijsbergen,
/// *Probabilistic models of information retrieval based on measuring the
/// divergence from randomness*, ACM Trans. Inf. Syst. 20, 4 (October 2002),
/// 357-389.
///
/// The scoring formula is composed of three separate components, named after
/// their counterparts in the Terrier IR engine:
///
/// 1. [`BasicModel`] — basic model of information content:
///    [`BasicModelG`](super::BasicModelG) (geometric approximation of
///    Bose-Einstein), [`BasicModelIn`](super::BasicModelIn) (inverse document
///    frequency), [`BasicModelIne`](super::BasicModelIne) (inverse expected
///    document frequency) and [`BasicModelIF`](super::BasicModelIF)
///    (inverse term frequency).
/// 2. [`AfterEffect`] — first normalization of information gain:
///    [`AfterEffectL`](super::AfterEffectL) (Laplace's law of succession) and
///    [`AfterEffectB`](super::AfterEffectB) (ratio of two Bernoulli processes).
/// 3. [`Normalization`] — second (length) normalization:
///    [`NormalizationH1`](super::NormalizationH1),
///    [`NormalizationH2`](super::NormalizationH2),
///    [`NormalizationH3`](super::NormalizationH3),
///    [`NormalizationZ`](super::NormalizationZ) or
///    [`NoNormalization`](super::NoNormalization).
///
/// *qtf*, the multiplicity of a term's occurrence in the query, is not handled
/// by this implementation.
///
/// The basic models BE (limiting form of Bose-Einstein), P (Poisson
/// approximation of the binomial) and D (divergence approximation of the
/// binomial) are not implemented, because their formulas could not be written
/// in a way that makes scores non-decreasing with the normalized term
/// frequency.
#[derive(Debug)]
pub struct DFRSimilarity {
    basic_model: Box<dyn BasicModel>,
    after_effect: Box<dyn AfterEffect>,
    normalization: Box<dyn Normalization>,
    discount_overlaps: bool,
}

impl DFRSimilarity {
    /// Creates a `DFRSimilarity` from the three components, discounting
    /// overlaps.
    ///
    /// Equivalent to
    /// `new DFRSimilarity(BasicModel, AfterEffect, Normalization)`
    /// (`DFRSimilarity.java:83-86`). Java rejects `null` components and asks
    /// for [`NoNormalization`](super::NoNormalization) when no length
    /// normalization is wanted; the Rust types make the `null` case
    /// unrepresentable, so the check has no counterpart here.
    pub fn new(
        basic_model: Box<dyn BasicModel>,
        after_effect: Box<dyn AfterEffect>,
        normalization: Box<dyn Normalization>,
    ) -> Self {
        Self::with_discount_overlaps(basic_model, after_effect, normalization, true)
    }

    /// Creates a `DFRSimilarity` from the three components with an explicit
    /// `discount_overlaps`.
    ///
    /// Equivalent to
    /// `new DFRSimilarity(BasicModel, AfterEffect, Normalization, boolean)`
    /// (`DFRSimilarity.java:100-113`).
    pub fn with_discount_overlaps(
        basic_model: Box<dyn BasicModel>,
        after_effect: Box<dyn AfterEffect>,
        normalization: Box<dyn Normalization>,
        discount_overlaps: bool,
    ) -> Self {
        Self {
            basic_model,
            after_effect,
            normalization,
            discount_overlaps,
        }
    }

    /// Returns the basic model of information content.
    ///
    /// Equivalent to `DFRSimilarity.getBasicModel()`.
    pub fn basic_model(&self) -> &dyn BasicModel {
        self.basic_model.as_ref()
    }

    /// Returns the first normalization.
    ///
    /// Equivalent to `DFRSimilarity.getAfterEffect()`.
    pub fn after_effect(&self) -> &dyn AfterEffect {
        self.after_effect.as_ref()
    }

    /// Returns the second normalization.
    ///
    /// Equivalent to `DFRSimilarity.getNormalization()`.
    pub fn normalization(&self) -> &dyn Normalization {
        self.normalization.as_ref()
    }
}

impl Similarity for DFRSimilarity {
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

impl SimilarityBase for DFRSimilarity {
    type Stats = BasicStats;

    fn simple_name(&self) -> &'static str {
        "DFRSimilarity"
    }

    fn score(&self, stats: &BasicStats, freq: f64, doc_len: f64) -> f64 {
        let tfn = self.normalization.tfn(stats, freq, doc_len);
        let ae_times_1p_tfn = self.after_effect.score_times_1p_tfn(stats);
        stats.boost() * self.basic_model.score(stats, tfn, ae_times_1p_tfn)
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
        let tfn = self.normalization.tfn(stats, freq, doc_len);
        let ae_times_1p_tfn = self.after_effect.score_times_1p_tfn(stats);
        subs.push(norm_expl);
        subs.push(self.basic_model.explain(stats, tfn, ae_times_1p_tfn));
        subs.push(self.after_effect.explain(stats, tfn));
    }

    /// Overrides the base explanation, as `DFRSimilarity.explain(BasicStats,
    /// Explanation, double)` does (`DFRSimilarity.java:135-150`).
    ///
    /// Note that this override takes the frequency as a `double`, where
    /// [`SimilarityBase::explain`] narrows it to a `float` first.
    fn explain(&self, stats: &BasicStats, freq: &Explanation, doc_len: f64) -> Explanation {
        let mut subs = Vec::new();
        let freq_value = freq.value().double_value();
        self.explain_details(&mut subs, stats, freq_value, doc_len);

        Explanation::matched(
            self.score(stats, freq_value, doc_len) as f32,
            format!(
                "score({}, freq={}), computed as boost * basicModel.score(stats, tfn) * afterEffect.score(stats, tfn) from:",
                self.simple_name(),
                freq.value()
            ),
            subs,
        )
    }
}

impl fmt::Display for DFRSimilarity {
    /// Renders the similarity as `DFRSimilarity.toString()` does: `DFR `
    /// followed by the three component codes, concatenated without separators.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "DFR {}{}{}",
            self.basic_model, self.after_effect, self.normalization
        )
    }
}
