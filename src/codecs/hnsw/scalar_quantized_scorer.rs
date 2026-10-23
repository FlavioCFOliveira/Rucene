//! `ScalarQuantizedVectorScorer` ported from `org.apache.lucene.codecs.hnsw`.
//!
//! Scores against quantized vectors, quantizing the query the same way the
//! stored vectors were quantized so the comparison is like for like.

use crate::codecs::hnsw::flat_vectors::FlatVectorsScorer;
use crate::error::Result;
use crate::index::{ByteVectorValues, FloatVectorValues, VectorSimilarityFunction};
use crate::util::hnsw::scorer::{
    RandomVectorScorer, RandomVectorScorerSupplier, UpdateableRandomVectorScorer,
};
use crate::util::quantization::{
    LegacyQuantizedByteVectorValues, ScalarQuantizedVectorSimilarity, ScalarQuantizer,
};
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
/// `LegacyQuantizedByteVectorValues`, and builds a quantized scorer when it is
/// (`ScalarQuantizedVectorScorer.java:66-71,80-99`). Rust has no such
/// downcast from a trait object, and the port's
/// [`LegacyQuantizedByteVectorValues`] is not a
/// [`ByteVectorValues`], so the quantized values cannot even be passed to the
/// four [`FlatVectorsScorer`] methods. Java's *unquantized* branch is therefore
/// what those four methods implement, and Java's *quantized* branch is reached
/// through the separate, typed
/// [`get_random_vector_scorer_supplier_quantized`](Self::get_random_vector_scorer_supplier_quantized),
/// which a reader that knows its vectors are quantized calls instead. The
/// scorers themselves — [`ScalarQuantizedRandomVectorScorer`] above and
/// [`ScalarQuantizedRandomVectorScorerSupplier`] below — are the port of the
/// ones Java's quantized branch returns.
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

    /// Returns a supplier that scores through the quantized representation of
    /// `vector_values`.
    ///
    /// Equivalent to the quantized branch of
    /// `ScalarQuantizedVectorScorer.getRandomVectorScorerSupplier(VectorSimilarityFunction,
    /// KnnVectorValues)`, which Java takes when the values it was handed are a
    /// `LegacyQuantizedByteVectorValues`: it reads the quantizer off the values
    /// and returns a `ScalarQuantizedRandomVectorScorerSupplier` over them. See
    /// the divergence declared on this type for why the branch is a method of
    /// its own here.
    pub fn get_random_vector_scorer_supplier_quantized(
        &self,
        similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn LegacyQuantizedByteVectorValues>,
    ) -> Box<dyn RandomVectorScorerSupplier> {
        let scalar_quantizer = *vector_values.scalar_quantizer();
        Box::new(ScalarQuantizedRandomVectorScorerSupplier::new(
            similarity_function,
            &scalar_quantizer,
            vector_values,
        ))
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

impl std::fmt::Display for ScalarQuantizedVectorScorer {
    /// Port of `ScalarQuantizedVectorScorer.toString()`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ScalarQuantizedVectorScorer(nonQuantizedDelegate={})",
            self.non_quantized_delegate
        )
    }
}

/// Supplies scorers over one field's quantized vectors, each of which scores
/// one stored vector against the rest of the field.
///
/// Equivalent to
/// `org.apache.lucene.codecs.hnsw.ScalarQuantizedVectorScorer.ScalarQuantizedRandomVectorScorerSupplier`,
/// the supplier Java's quantized branch returns.
///
/// The constant multiplier is kept beside the similarity because this port's
/// [`ScalarQuantizedVectorSimilarity`] takes it at scoring time rather than
/// binding it into the object, as its own documentation records.
pub struct ScalarQuantizedRandomVectorScorerSupplier {
    values: Box<dyn LegacyQuantizedByteVectorValues>,
    similarity: ScalarQuantizedVectorSimilarity,
    const_multiplier: f32,
    vector_similarity_function: VectorSimilarityFunction,
}

impl std::fmt::Debug for ScalarQuantizedRandomVectorScorerSupplier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScalarQuantizedRandomVectorScorerSupplier")
            .field(
                "vector_similarity_function",
                &self.vector_similarity_function,
            )
            .field("size", &self.values.size())
            .finish_non_exhaustive()
    }
}

impl ScalarQuantizedRandomVectorScorerSupplier {
    /// Creates a supplier over `values`, which `scalar_quantizer` compressed.
    ///
    /// Equivalent to the public
    /// `ScalarQuantizedRandomVectorScorerSupplier(VectorSimilarityFunction, ScalarQuantizer,
    /// LegacyQuantizedByteVectorValues)` constructor, which derives the
    /// similarity from the quantizer's bit width and constant multiplier.
    pub fn new(
        similarity_function: VectorSimilarityFunction,
        scalar_quantizer: &ScalarQuantizer,
        values: Box<dyn LegacyQuantizedByteVectorValues>,
    ) -> Self {
        Self {
            values,
            similarity: ScalarQuantizedVectorSimilarity::from_vector_similarity(
                similarity_function,
                scalar_quantizer.get_bits(),
            ),
            const_multiplier: scalar_quantizer.get_constant_multiplier(),
            vector_similarity_function: similarity_function,
        }
    }

    /// Creates a supplier that reuses an already derived similarity.
    ///
    /// Equivalent to the private
    /// `ScalarQuantizedRandomVectorScorerSupplier(ScalarQuantizedVectorSimilarity,
    /// VectorSimilarityFunction, LegacyQuantizedByteVectorValues)` constructor,
    /// which only `copy()` calls.
    fn with_similarity(
        similarity: ScalarQuantizedVectorSimilarity,
        const_multiplier: f32,
        vector_similarity_function: VectorSimilarityFunction,
        values: Box<dyn LegacyQuantizedByteVectorValues>,
    ) -> Self {
        Self {
            values,
            similarity,
            const_multiplier,
            vector_similarity_function,
        }
    }
}

impl RandomVectorScorerSupplier for ScalarQuantizedRandomVectorScorerSupplier {
    /// Equivalent to `ScalarQuantizedRandomVectorScorerSupplier.scorer()`,
    /// which gives the scorer its own copy of the values and an all-zero query
    /// buffer of the field's dimension, to be filled by `setScoringOrdinal`.
    fn scorer(&self) -> Result<Box<dyn UpdateableRandomVectorScorer>> {
        let vectors_copy = self.values.copy_legacy()?;
        let query_vector = vec![0u8; self.values.dimension() as usize];
        Ok(Box::new(ScalarQuantizedUpdateableRandomVectorScorer {
            vectors_copy,
            similarity: self.similarity,
            const_multiplier: self.const_multiplier,
            query_vector,
            query_offset: 0.0,
            node_vector: Vec::new(),
        }))
    }

    /// Equivalent to `ScalarQuantizedRandomVectorScorerSupplier.copy()`, which
    /// keeps the derived similarity and copies only the values.
    fn copy(&self) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        Ok(Box::new(Self::with_similarity(
            self.similarity,
            self.const_multiplier,
            self.vector_similarity_function,
            self.values.copy_legacy()?,
        )))
    }
}

impl std::fmt::Display for ScalarQuantizedRandomVectorScorerSupplier {
    /// Port of `ScalarQuantizedRandomVectorScorerSupplier.toString()`, which
    /// names only the similarity function.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ScalarQuantizedRandomVectorScorerSupplier(vectorSimilarityFunction={:?})",
            self.vector_similarity_function
        )
    }
}

/// Scores the quantized vector last named by
/// [`set_scoring_ordinal`](UpdateableRandomVectorScorer::set_scoring_ordinal)
/// against the rest of the field.
///
/// Equivalent to the anonymous
/// `UpdateableRandomVectorScorer.AbstractUpdateableRandomVectorScorer` that
/// `ScalarQuantizedRandomVectorScorerSupplier.scorer()` returns.
struct ScalarQuantizedUpdateableRandomVectorScorer {
    vectors_copy: Box<dyn LegacyQuantizedByteVectorValues>,
    similarity: ScalarQuantizedVectorSimilarity,
    const_multiplier: f32,
    query_vector: Vec<u8>,
    query_offset: f32,
    /// Holds the vector being scored against, because Rust cannot keep the
    /// slice `vector_value` lends while `get_score_correction_constant` reads
    /// from the same values. Java holds both at once through one reference.
    /// The buffer is reused, so steady-state scoring allocates nothing.
    node_vector: Vec<u8>,
}

impl RandomVectorScorer for ScalarQuantizedUpdateableRandomVectorScorer {
    /// Equivalent to the anonymous scorer's `score(int)`, which reads the
    /// stored vector and its corrective offset and scores them against the
    /// query through `ScalarQuantizedVectorSimilarity`.
    fn score(&mut self, node: i32) -> Result<f32> {
        let node_vector = self.vectors_copy.vector_value(node)?;
        self.node_vector.clear();
        self.node_vector.extend_from_slice(node_vector);
        let node_offset = self.vectors_copy.get_score_correction_constant(node)?;
        self.similarity.score(
            &self.query_vector,
            self.query_offset,
            &self.node_vector,
            node_offset,
            self.const_multiplier,
        )
    }

    /// Equivalent to `AbstractUpdateableRandomVectorScorer.maxOrd()`, which is
    /// the size of the values the scorer was built over.
    fn max_ord(&self) -> i32 {
        self.vectors_copy.size()
    }
}

impl UpdateableRandomVectorScorer for ScalarQuantizedUpdateableRandomVectorScorer {
    /// Equivalent to the anonymous scorer's `setScoringOrdinal(int)`, which
    /// copies `queryVector.length` bytes of the stored vector into the query
    /// buffer and takes that vector's corrective offset as the query's.
    fn set_scoring_ordinal(&mut self, node: i32) -> Result<()> {
        let length = self.query_vector.len();
        let vector = self.vectors_copy.vector_value(node)?;
        self.query_vector.copy_from_slice(&vector[..length]);
        self.query_offset = self.vectors_copy.get_score_correction_constant(node)?;
        Ok(())
    }
}
