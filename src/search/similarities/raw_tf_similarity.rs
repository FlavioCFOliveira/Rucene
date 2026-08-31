//! Raw term-frequency scoring, ported from
//! `org.apache.lucene.search.similarities.RawTFSimilarity`.

#![deny(unsafe_code)]

use super::{CollectionStatistics, SimScorer, Similarity, TermStatistics};

/// Similarity that returns the raw term frequency as the score.
///
/// Equivalent to `org.apache.lucene.search.similarities.RawTFSimilarity`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawTFSimilarity {
    discount_overlaps: bool,
}

impl RawTFSimilarity {
    /// Creates the similarity, discounting overlaps.
    ///
    /// Equivalent to `new RawTFSimilarity()`
    /// (`RawTFSimilarity.java:26-28`).
    pub fn new() -> Self {
        Self {
            discount_overlaps: true,
        }
    }

    /// Creates the similarity with an explicit `discount_overlaps`.
    ///
    /// Equivalent to `new RawTFSimilarity(boolean)`
    /// (`RawTFSimilarity.java:30-32`).
    pub fn with_discount_overlaps(discount_overlaps: bool) -> Self {
        Self { discount_overlaps }
    }
}

impl Default for RawTFSimilarity {
    fn default() -> Self {
        Self::new()
    }
}

impl Similarity for RawTFSimilarity {
    fn discount_overlaps(&self) -> bool {
        self.discount_overlaps
    }

    fn scorer<'a>(
        &'a self,
        boost: f32,
        collection_stats: &CollectionStatistics,
        term_stats: &[TermStatistics],
    ) -> Box<dyn SimScorer + 'a> {
        let _ = (collection_stats, term_stats);
        Box::new(RawTFScorer { boost })
    }
}

/// Equivalent to the anonymous `SimScorer` subclass
/// `RawTFSimilarity.scorer` returns (`RawTFSimilarity.java:37-43`). It
/// overrides only `score`, so explanations come from
/// [`SimScorer::explain`]'s default.
struct RawTFScorer {
    boost: f32,
}

impl SimScorer for RawTFScorer {
    fn score(&self, freq: f32, norm: i64) -> f32 {
        let _ = norm;
        self.boost * freq
    }
}
