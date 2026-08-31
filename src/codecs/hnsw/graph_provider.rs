//! `HnswGraphProvider` and the scorer wrappers ported from
//! `org.apache.lucene.codecs.hnsw`.

use crate::codecs::hnsw::flat_vectors::FlatVectorsScorer;
use crate::error::Result;
use crate::index::{ByteVectorValues, FloatVectorValues, VectorSimilarityFunction};
use crate::util::hnsw::scorer::{RandomVectorScorer, RandomVectorScorerSupplier};
use crate::util::hnsw::HnswGraph;

/// A reader that can hand out the HNSW graph it stores for a field.
///
/// Equivalent to `org.apache.lucene.codecs.hnsw.HnswGraphProvider`, which the
/// HNSW vector readers implement so that a merge can reuse an existing graph
/// instead of rebuilding it.
pub trait HnswGraphProvider {
    /// Returns the graph stored for `field`.
    ///
    /// Equivalent to `HnswGraphProvider.getGraph(String)`.
    fn get_graph(&self, field: &str) -> Result<Box<dyn HnswGraph>>;
}

/// A [`FlatVectorsScorer`] that asks the vector values to prefetch the bytes it
/// is about to score.
///
/// Equivalent to `org.apache.lucene.codecs.hnsw.PrefetchableFlatVectorScorer`,
/// which wraps another scorer and issues the prefetch before each score so the
/// data is warm by the time the comparison runs.
///
/// **Divergence from Lucene 10.5.0.** Java's wrapper returns
/// `UpdateableRandomVectorScorer` instances that call `prefetch` on the
/// underlying `KnnVectorValues` before scoring. Neither
/// `UpdateableRandomVectorScorer` nor a prefetch hook on the vector-values
/// traits exists in this port yet, so the wrapper forwards without prefetching.
/// The scores produced are identical; only the memory-access hint is missing.
#[derive(Debug)]
pub struct PrefetchableFlatVectorScorer {
    inner: Box<dyn FlatVectorsScorer>,
}

impl PrefetchableFlatVectorScorer {
    /// Wraps `inner`.
    pub fn new(inner: Box<dyn FlatVectorsScorer>) -> Self {
        Self { inner }
    }

    /// Returns the wrapped scorer.
    pub fn get_delegate(&self) -> &dyn FlatVectorsScorer {
        self.inner.as_ref()
    }
}

impl FlatVectorsScorer for PrefetchableFlatVectorScorer {
    fn get_random_vector_scorer_supplier_float(
        &self,
        similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn FloatVectorValues>,
    ) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        self.inner
            .get_random_vector_scorer_supplier_float(similarity_function, vector_values)
    }

    fn get_random_vector_scorer_supplier_byte(
        &self,
        similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn ByteVectorValues>,
    ) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        self.inner
            .get_random_vector_scorer_supplier_byte(similarity_function, vector_values)
    }

    fn get_random_vector_scorer_float(
        &self,
        similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn FloatVectorValues>,
        target: &[f32],
    ) -> Result<Box<dyn RandomVectorScorer>> {
        self.inner
            .get_random_vector_scorer_float(similarity_function, vector_values, target)
    }

    fn get_random_vector_scorer_byte(
        &self,
        similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn ByteVectorValues>,
        target: &[u8],
    ) -> Result<Box<dyn RandomVectorScorer>> {
        self.inner
            .get_random_vector_scorer_byte(similarity_function, vector_values, target)
    }
}
