//! Combination of several similarities, ported from
//! `org.apache.lucene.search.similarities.MultiSimilarity`.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::indexing_chain::FieldInvertState;

use super::{CollectionStatistics, Explanation, SimScorer, Similarity, TermStatistics};

/// Implements the CombSUM method for combining evidence from several
/// similarity values.
///
/// Equivalent to `org.apache.lucene.search.similarities.MultiSimilarity`.
/// Described in Joseph A. Shaw and Edward A. Fox, *Combination of Multiple
/// Searches*, TREC 1993, pp. 243-252.
///
/// Java holds a `Similarity[]`; this port holds `Arc<dyn Similarity>` because
/// the trait is used behind a pointer everywhere else in the crate — for
/// instance in `IndexWriterConfig` — and because the scorers this similarity
/// returns borrow the sub-similarities for as long as they live.
#[derive(Debug, Clone)]
pub struct MultiSimilarity {
    sims: Vec<Arc<dyn Similarity>>,
}

impl MultiSimilarity {
    /// Creates a `MultiSimilarity` which will sum the scores of the provided
    /// similarities.
    ///
    /// Equivalent to `new MultiSimilarity(Similarity[])`
    /// (`MultiSimilarity.java:36-38`). Java accepts an empty array, and so does
    /// this constructor; see [`Similarity::compute_norm`] for what an empty set
    /// then means.
    pub fn new(sims: Vec<Arc<dyn Similarity>>) -> Self {
        Self { sims }
    }

    /// Returns the sub-similarities used to create the combined score.
    ///
    /// Java declares the `sims` field `protected` with no accessor
    /// (`MultiSimilarity.java:33`); this is the Rust equivalent of reaching it
    /// from a subclass.
    pub fn sims(&self) -> &[Arc<dyn Similarity>] {
        &self.sims
    }
}

impl Similarity for MultiSimilarity {
    /// Delegates to the first sub-similarity, as
    /// `MultiSimilarity.computeNorm` does (`MultiSimilarity.java:41-44`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when there is no
    /// sub-similarity to delegate to. Java indexes `sims[0]` unconditionally
    /// and throws `ArrayIndexOutOfBoundsException` in that case; the failure is
    /// reported rather than hidden, and no bound Lucene lacks is added to the
    /// constructor to prevent it.
    fn compute_norm(&self, state: &FieldInvertState) -> Result<i64> {
        match self.sims.first() {
            Some(first) => first.compute_norm(state),
            None => Err(LuceneError::IllegalArgument(
                "MultiSimilarity has no sub-similarity to compute a norm with".to_string(),
            )),
        }
    }

    fn scorer<'a>(
        &'a self,
        boost: f32,
        collection_stats: &CollectionStatistics,
        term_stats: &[TermStatistics],
    ) -> Box<dyn SimScorer + 'a> {
        let sub_scorers = self
            .sims
            .iter()
            .map(|sim| sim.scorer(boost, collection_stats, term_stats))
            .collect();
        Box::new(MultiSimScorer::new(sub_scorers))
    }
}

/// Sums the scores of several sub-scorers.
///
/// Equivalent to `MultiSimilarity.MultiSimScorer`
/// (`MultiSimilarity.java:56-80`), which Java declares package-private and
/// reuses from `SimilarityBase.scorer` for multi-term queries; the crate
/// visibility here plays the same role.
pub(crate) struct MultiSimScorer<'a> {
    sub_scorers: Vec<Box<dyn SimScorer + 'a>>,
}

impl<'a> MultiSimScorer<'a> {
    /// Creates a scorer summing `sub_scorers`.
    pub(crate) fn new(sub_scorers: Vec<Box<dyn SimScorer + 'a>>) -> Self {
        Self { sub_scorers }
    }
}

impl SimScorer for MultiSimScorer<'_> {
    fn score(&self, freq: f32, norm: i64) -> f32 {
        // Java accumulates into a `double` and narrows once at the end.
        let mut sum = 0.0f64;
        for sub_scorer in &self.sub_scorers {
            sum += f64::from(sub_scorer.score(freq, norm));
        }
        sum as f32
    }

    fn explain(&self, freq: &Explanation, norm: i64) -> Explanation {
        let subs = self
            .sub_scorers
            .iter()
            .map(|sub_scorer| sub_scorer.explain(freq, norm))
            .collect();
        Explanation::matched(
            self.score(freq.value().float_value(), norm),
            "sum of:",
            subs,
        )
    }
}
