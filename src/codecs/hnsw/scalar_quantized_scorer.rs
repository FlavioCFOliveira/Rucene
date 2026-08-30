//! `ScalarQuantizedVectorScorer` ported from `org.apache.lucene.codecs.hnsw`.
//!
//! Scores against quantized vectors, quantizing the query the same way the
//! stored vectors were quantized so the comparison is like for like.

use crate::codecs::hnsw::flat_vectors::FlatVectorsScorer;
use crate::error::Result;
use crate::index::{ByteVectorValues, FloatVectorValues, VectorSimilarityFunction};
use crate::util::hnsw::scorer::{RandomVectorScorer, RandomVectorScorerSupplier};
use crate::util::quantization::{ScalarQuantizedVectorSimilarity, ScalarQuantizer};
use crate::util::vector_util;

/// Quantizes a query the same way the stored vectors were quantized.
///
/// Equivalent to `ScalarQuantizedVectorScorer.quantizeQuery`. A cosine query is
/// normalised first, because cosine quantization assumes unit vectors.
pub fn quantize_query(
    query: &[f32],
    quantized_query: &mut [u8],
    similarity_function: VectorSimilarityFunction,
    scalar_quantizer: &ScalarQuantizer,
) -> Result<f32> {
    match similarity_function {
        VectorSimilarityFunction::COSINE => {
            let mut normalized = query.to_vec();
            vector_util::l2normalize(&mut normalized, true)?;
            scalar_quantizer.quantize(&normalized, quantized_query, similarity_function)
        }
        _ => scalar_quantizer.quantize(query, quantized_query, similarity_function),
    }
}

/// Scores a query against one field's quantized vectors.
///
/// Equivalent to the scoring half of
/// `ScalarQuantizedVectorScorer.ScalarQuantizedRandomVectorScorer`.
pub struct ScalarQuantizedRandomVectorScorer {
    similarity: ScalarQuantizedVectorSimilarity,
    const_multiplier: f32,
    query: Vec<u8>,
    query_offset: f32,
    /// The stored vectors, each followed by its corrective offset.
    vectors: Vec<(Vec<u8>, f32)>,
}

impl std::fmt::Debug for ScalarQuantizedRandomVectorScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScalarQuantizedRandomVectorScorer")
            .field("vectors", &self.vectors.len())
            .finish_non_exhaustive()
    }
}

impl ScalarQuantizedRandomVectorScorer {
    /// Creates a scorer over `vectors` for the already quantized `query`.
    pub fn new(
        similarity_function: VectorSimilarityFunction,
        quantizer: &ScalarQuantizer,
        query: Vec<u8>,
        query_offset: f32,
        vectors: Vec<(Vec<u8>, f32)>,
    ) -> Self {
        Self {
            similarity: ScalarQuantizedVectorSimilarity::from_vector_similarity(
                similarity_function,
                quantizer.get_bits(),
            ),
            const_multiplier: quantizer.get_constant_multiplier(),
            query,
            query_offset,
            vectors,
        }
    }
}

impl RandomVectorScorer for ScalarQuantizedRandomVectorScorer {
    fn score(&mut self, node: i32) -> Result<f32> {
        let (stored, offset) = &self.vectors[node as usize];
        self.similarity.score(
            &self.query,
            self.query_offset,
            stored,
            *offset,
            self.const_multiplier,
        )
    }

    fn max_ord(&self) -> i32 {
        self.vectors.len() as i32
    }
}

/// A [`FlatVectorsScorer`] that scores against quantized vectors when the field
/// holds them, and forwards to a plain scorer otherwise.
///
/// Equivalent to `org.apache.lucene.codecs.hnsw.ScalarQuantizedVectorScorer`.
///
/// **Divergence from Lucene 10.5.0.** Java decides at run time by testing
/// whether the `KnnVectorValues` it was handed is a
/// `LegacyQuantizedByteVectorValues`, and builds a quantized scorer when it is.
/// Rust has no such downcast from a trait object, so this wrapper forwards
/// every call and the quantized scorers are constructed directly by the readers
/// that know their vectors are quantized —
/// [`ScalarQuantizedRandomVectorScorer`] above.
#[derive(Debug)]
pub struct ScalarQuantizedVectorScorer {
    non_quantized_delegate: Box<dyn FlatVectorsScorer>,
}

impl ScalarQuantizedVectorScorer {
    /// Wraps `delegate`, which handles the unquantized fields.
    pub fn new(delegate: Box<dyn FlatVectorsScorer>) -> Self {
        Self {
            non_quantized_delegate: delegate,
        }
    }

    /// Returns the scorer used for unquantized fields.
    pub fn get_non_quantized_delegate(&self) -> &dyn FlatVectorsScorer {
        self.non_quantized_delegate.as_ref()
    }
}

impl FlatVectorsScorer for ScalarQuantizedVectorScorer {
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
