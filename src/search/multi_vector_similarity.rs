//! Multi-vector similarity, ported from
//! `org.apache.lucene.search.MultiVectorSimilarity`.

#![deny(unsafe_code)]

use std::any::Any;
use std::fmt::Debug;

use crate::error::Result;
use crate::index::VectorSimilarityFunction;

/// Defines the similarity function between two multi-vectors.
///
/// Equivalent to the interface
/// `org.apache.lucene.search.MultiVectorSimilarity`.
///
/// **Divergence from Lucene 10.5.0.** `compare` returns a [`Result`] because
/// the only implementation Lucene ships,
/// [`ScoreFunction::SUM_MAX_SIM`](crate::search::LateInteractionScoreFunction),
/// throws `IllegalArgumentException` on incompatible token dimensions; this
/// crate reports such failures rather than unwinding. The [`Any`]-based
/// identity methods exist for the same reason
/// [`Query::as_any`](crate::search::Query::as_any) does: Java compares
/// implementations with `==` on enum constants, which needs the concrete type
/// back.
pub trait MultiVectorSimilarity: Send + Sync + Debug {
    /// Computes the similarity between two multi-vectors using the provided
    /// [`VectorSimilarityFunction`].
    ///
    /// Equivalent to
    /// `MultiVectorSimilarity.compare(float[][], float[][], VectorSimilarityFunction)`.
    /// The two multi-vectors may hold a different number of token vectors, but
    /// their token vectors must have the same dimension.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`](crate::error::LuceneError::IllegalArgument)
    /// when the token vectors do not have the same dimension.
    fn compare(
        &self,
        outer: &[Vec<f32>],
        inner: &[Vec<f32>],
        vector_similarity_function: VectorSimilarityFunction,
    ) -> Result<f32>;

    /// Returns this similarity as [`Any`], so that
    /// [`similarity_eq`](Self::similarity_eq) can recover the concrete type.
    fn as_any(&self) -> &dyn Any;

    /// Similarity instance equivalence.
    ///
    /// Equivalent to the `==` Java uses on the enum constants that implement
    /// this interface.
    fn similarity_eq(&self, other: &dyn MultiVectorSimilarity) -> bool;

    /// Similarity hash code, consistent with
    /// [`similarity_eq`](Self::similarity_eq).
    fn similarity_hash(&self) -> u64;
}
