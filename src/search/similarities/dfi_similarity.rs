//! The divergence-from-independence model, ported from
//! `org.apache.lucene.search.similarities.DFISimilarity`.

#![deny(unsafe_code)]

use std::fmt;

use super::similarity_base::{log2, similarity_base_scorer};
use super::{
    BasicStats, CollectionStatistics, Explanation, Independence, SimScorer, Similarity,
    SimilarityBase, TermStatistics,
};

/// Implements the *divergence from independence (DFI)* model, based on
/// chi-square statistics.
///
/// Equivalent to `org.apache.lucene.search.similarities.DFISimilarity`. The
/// model is both parameter-free — it needs no tuning or training — and
/// non-parametric — it assumes nothing about word frequency distributions in
/// the collection.
///
/// It is highly recommended **not** to remove stopwords with this similarity.
///
/// See <http://dx.doi.org/10.1007/s10791-013-9225-4>, *A nonparametric term
/// weighting method for information retrieval based on measuring the divergence
/// from independence*, and the three measures:
/// [`IndependenceStandardized`](super::IndependenceStandardized),
/// [`IndependenceSaturated`](super::IndependenceSaturated) and
/// [`IndependenceChiSquared`](super::IndependenceChiSquared).
#[derive(Debug)]
pub struct DFISimilarity {
    independence: Box<dyn Independence>,
    discount_overlaps: bool,
}

impl DFISimilarity {
    /// Creates a `DFISimilarity` with the given measure, discounting overlaps.
    ///
    /// Equivalent to `new DFISimilarity(Independence)`
    /// (`DFISimilarity.java:52-54`).
    pub fn new(independence_measure: Box<dyn Independence>) -> Self {
        Self::with_discount_overlaps(independence_measure, true)
    }

    /// Creates a `DFISimilarity` with the given measure and an explicit
    /// `discount_overlaps`.
    ///
    /// Equivalent to `new DFISimilarity(Independence, boolean)`
    /// (`DFISimilarity.java:62-65`).
    pub fn with_discount_overlaps(
        independence_measure: Box<dyn Independence>,
        discount_overlaps: bool,
    ) -> Self {
        Self {
            independence: independence_measure,
            discount_overlaps,
        }
    }

    /// Returns the measure of independence.
    ///
    /// Equivalent to `DFISimilarity.getIndependence()`.
    pub fn independence(&self) -> &dyn Independence {
        self.independence.as_ref()
    }

    /// The expected term frequency, `(F + 1) * dl / (T + 1)`, shared by `score`
    /// and `explain`.
    fn expected(stats: &BasicStats, doc_len: f64) -> f64 {
        stats.total_term_freq().wrapping_add(1) as f64 * doc_len
            / stats.number_of_field_tokens().wrapping_add(1) as f64
    }
}

impl Similarity for DFISimilarity {
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

impl SimilarityBase for DFISimilarity {
    type Stats = BasicStats;

    fn simple_name(&self) -> &'static str {
        "DFISimilarity"
    }

    fn score(&self, stats: &BasicStats, freq: f64, doc_len: f64) -> f64 {
        let expected = Self::expected(stats, doc_len);

        // A frequency at or below the expected value carries no evidence.
        if freq <= expected {
            return 0.0;
        }

        let measure = self.independence.score(freq, expected);
        stats.boost() * log2(measure + 1.0)
    }

    /// Overrides the base explanation, as `DFISimilarity.explain(BasicStats,
    /// Explanation, double)` does (`DFISimilarity.java:80-119`), including the
    /// short-circuit for a frequency at or below the expected value.
    fn explain(&self, stats: &BasicStats, freq: &Explanation, doc_len: f64) -> Explanation {
        let expected = Self::expected(stats, doc_len);
        if freq.value().double_value() <= expected {
            return Explanation::matched(
                0.0f32,
                format!(
                    "score({}, freq={}), equals to 0",
                    self.simple_name(),
                    freq.value()
                ),
                vec![],
            );
        }
        let expl_expected = Explanation::matched(
            expected as f32,
            "expected, computed as (F + 1) * dl / (T + 1) from:",
            vec![
                Explanation::matched(
                    stats.total_term_freq(),
                    "F, total number of occurrences of term across all docs",
                    vec![],
                ),
                Explanation::matched(doc_len as f32, "dl, length of field", vec![]),
                Explanation::matched(
                    stats.number_of_field_tokens(),
                    "T, total number of tokens in the field",
                    vec![],
                ),
            ],
        );

        let measure = self
            .independence
            .score(freq.value().double_value(), expected);
        let expl_measure = Explanation::matched(
            measure as f32,
            "measure, computed as independence.score(freq, expected) from:",
            vec![freq.clone(), expl_expected],
        );

        Explanation::matched(
            self.score(stats, freq.value().double_value(), doc_len) as f32,
            format!(
                "score({}, freq={}), computed as boost * log2(measure + 1) from:",
                self.simple_name(),
                freq.value()
            ),
            vec![
                Explanation::matched(stats.boost() as f32, "boost, query boost", vec![]),
                expl_measure,
            ],
        )
    }
}

impl fmt::Display for DFISimilarity {
    /// Renders the similarity as `DFISimilarity.toString()` does:
    /// `DFI(<measure>)`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DFI({})", self.independence)
    }
}
