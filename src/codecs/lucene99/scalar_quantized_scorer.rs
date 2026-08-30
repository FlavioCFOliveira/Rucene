//! `Lucene99ScalarQuantizedVectorScorer` ported from
//! `org.apache.lucene.codecs.lucene99`.

use std::sync::Arc;

use crate::codecs::hnsw::flat_vectors::FlatVectorsScorer;
use crate::codecs::hnsw::scalar_quantized_scorer::quantize_query;
use crate::error::Result;
use crate::index::{ByteVectorValues, FloatVectorValues, VectorSimilarityFunction};
use crate::util::hnsw::scorer::{RandomVectorScorer, RandomVectorScorerSupplier};
use crate::util::quantization::{ScalarQuantizedVectorSimilarity, ScalarQuantizer};

/// Scores against the vectors the Lucene 9.9 quantized format stores.
///
/// Equivalent to
/// `org.apache.lucene.codecs.lucene99.Lucene99ScalarQuantizedVectorScorer`.
///
/// **Divergence from Lucene 10.5.0.** Java picks a vectorised inner loop
/// through `VectorizationProvider` and decides at run time whether the values it
/// was handed are quantized. This port keeps the scalar loop and is constructed
/// by the reader that knows its vectors are quantized, for the same reason as
/// `codecs::hnsw::ScalarQuantizedVectorScorer`.
pub struct Lucene99ScalarQuantizedVectorScorer {
    non_quantized_delegate: Arc<dyn FlatVectorsScorer>,
}

impl std::fmt::Debug for Lucene99ScalarQuantizedVectorScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lucene99ScalarQuantizedVectorScorer")
            .finish_non_exhaustive()
    }
}

impl Lucene99ScalarQuantizedVectorScorer {
    /// Creates the scorer, delegating unquantized fields to `delegate`.
    pub fn new(delegate: Arc<dyn FlatVectorsScorer>) -> Self {
        Self {
            non_quantized_delegate: delegate,
        }
    }

    /// Quantizes `query` the way the stored vectors were quantized, returning
    /// the quantized form and its corrective offset.
    pub fn quantize_query(
        query: &[f32],
        similarity_function: VectorSimilarityFunction,
        quantizer: &ScalarQuantizer,
    ) -> Result<(Vec<u8>, f32)> {
        let mut quantized = vec![0u8; query.len()];
        let offset = quantize_query(query, &mut quantized, similarity_function, quantizer)?;
        Ok((quantized, offset))
    }

    /// Scores one quantized query against one stored quantized vector.
    pub fn score(
        similarity_function: VectorSimilarityFunction,
        quantizer: &ScalarQuantizer,
        query: &[u8],
        query_offset: f32,
        stored: &[u8],
        stored_offset: f32,
    ) -> Result<f32> {
        ScalarQuantizedVectorSimilarity::from_vector_similarity(
            similarity_function,
            quantizer.get_bits(),
        )
        .score(
            query,
            query_offset,
            stored,
            stored_offset,
            quantizer.get_constant_multiplier(),
        )
    }
}

impl FlatVectorsScorer for Lucene99ScalarQuantizedVectorScorer {
    fn get_random_vector_scorer_supplier_float(
        &self,
        similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn FloatVectorValues>,
    ) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        self.non_quantized_delegate
            .get_random_vector_scorer_supplier_float(similarity_function, vector_values)
    }

    fn get_random_vector_scorer_supplier_byte(
        &self,
        similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn ByteVectorValues>,
    ) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        self.non_quantized_delegate
            .get_random_vector_scorer_supplier_byte(similarity_function, vector_values)
    }

    fn get_random_vector_scorer_float(
        &self,
        similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn FloatVectorValues>,
        target: &[f32],
    ) -> Result<Box<dyn RandomVectorScorer>> {
        self.non_quantized_delegate.get_random_vector_scorer_float(
            similarity_function,
            vector_values,
            target,
        )
    }

    fn get_random_vector_scorer_byte(
        &self,
        similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn ByteVectorValues>,
        target: &[u8],
    ) -> Result<Box<dyn RandomVectorScorer>> {
        self.non_quantized_delegate.get_random_vector_scorer_byte(
            similarity_function,
            vector_values,
            target,
        )
    }
}
