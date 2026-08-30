//! Boost-only scoring, ported from
//! `org.apache.lucene.search.similarities.BooleanSimilarity`.

#![deny(unsafe_code)]

use super::{CollectionStatistics, Explanation, SimScorer, Similarity, TermStatistics};

/// Simple similarity that gives terms a score equal to their query boost.
///
/// Equivalent to `org.apache.lucene.search.similarities.BooleanSimilarity`.
/// It is typically used with norms disabled, since neither document nor index
/// statistics take part in scoring. When norms are enabled they are computed
/// exactly as [`SimilarityBase`](super::SimilarityBase) and
/// [`BM25Similarity`](super::BM25Similarity) compute them, with overlaps
/// discounted, so that the similarity can still be changed after the index has
/// been created.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BooleanSimilarity;

impl BooleanSimilarity {
    /// Creates the similarity.
    ///
    /// Equivalent to `new BooleanSimilarity()`.
    pub fn new() -> Self {
        Self
    }
}

impl Similarity for BooleanSimilarity {
    fn scorer<'a>(
        &'a self,
        boost: f32,
        collection_stats: &CollectionStatistics,
        term_stats: &[TermStatistics],
    ) -> Box<dyn SimScorer + 'a> {
        let _ = (collection_stats, term_stats);
        Box::new(BooleanWeight { boost })
    }
}

/// Equivalent to the private nested `BooleanSimilarity.BooleanWeight`
/// (`BooleanSimilarity.java:36-62`).
struct BooleanWeight {
    boost: f32,
}

impl SimScorer for BooleanWeight {
    fn score(&self, freq: f32, norm: i64) -> f32 {
        let _ = (freq, norm);
        self.boost
    }

    fn explain(&self, freq: &Explanation, norm: i64) -> Explanation {
        let _ = (freq, norm);
        let query_boost_expl = Explanation::matched(self.boost, "boost, query boost", vec![]);
        // Java's `getClass().getSimpleName()` here resolves to the *scorer*
        // class, so the description names `BooleanWeight`, not
        // `BooleanSimilarity`.
        Explanation::matched(
            query_boost_expl.value(),
            "score(BooleanWeight), computed from:",
            vec![query_boost_expl],
        )
    }
}
