//! Vector scorer traits used during HNSW graph construction and search.
//!
//! Equivalent to the `RandomVectorScorer*` interfaces in
//! `org.apache.lucene.util.hnsw`.

#![deny(unsafe_code)]

use super::Result;

/// Scores random graph nodes against an abstract query.
///
/// Equivalent to `org.apache.lucene.util.hnsw.RandomVectorScorer`.
///
/// Implementations are not required to be thread-safe; each search/build
/// thread should obtain its own scorer via a supplier.
pub trait RandomVectorScorer {
    /// Returns the score between the query and the given node.
    fn score(&mut self, node: i32) -> Result<f32>;

    /// Scores a batch of nodes, storing the results in `scores`.
    ///
    /// Returns the maximum score, or `f32::NEG_INFINITY` if
    /// `num_nodes == 0`.
    fn bulk_score(&mut self, nodes: &[i32], scores: &mut [f32], num_nodes: i32) -> Result<f32> {
        let n = num_nodes as usize;
        let mut max = f32::NEG_INFINITY;
        for i in 0..n {
            scores[i] = self.score(nodes[i])?;
            max = max.max(scores[i]);
        }
        Ok(max)
    }

    /// Returns the maximum ordinal that this scorer can score.
    fn max_ord(&self) -> i32;

    /// Translates a vector ordinal to a document id.
    fn ord_to_doc(&self, ord: i32) -> i32 {
        ord
    }
}

/// A scorer whose scoring ordinal can be changed in place.
///
/// Equivalent to `org.apache.lucene.util.hnsw.UpdateableRandomVectorScorer`.
pub trait UpdateableRandomVectorScorer: RandomVectorScorer {
    /// Changes the scoring ordinal to the given node.
    fn set_scoring_ordinal(&mut self, node: i32) -> Result<()>;
}

/// Creates `UpdateableRandomVectorScorer` instances.
///
/// Equivalent to `org.apache.lucene.util.hnsw.RandomVectorScorerSupplier`.
pub trait RandomVectorScorerSupplier {
    /// Creates a new scorer.
    fn scorer(&self) -> Result<Box<dyn UpdateableRandomVectorScorer>>;

    /// Makes a copy of the supplier that is safe to use on another thread.
    fn copy(&self) -> Result<Box<dyn RandomVectorScorerSupplier>>;
}

/// A scorer supplier that also carries a closeable resource and knows the total
/// number of vectors.
///
/// Equivalent to `org.apache.lucene.util.hnsw.CloseableRandomVectorScorerSupplier`.
pub trait CloseableRandomVectorScorerSupplier: RandomVectorScorerSupplier {
    /// Returns the total number of vectors managed by this supplier.
    fn total_vector_count(&self) -> i32;
}
