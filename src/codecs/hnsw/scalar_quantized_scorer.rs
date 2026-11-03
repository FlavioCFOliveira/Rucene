//! `ScalarQuantizedVectorScorer` ported from `org.apache.lucene.codecs.hnsw`.
//!
//! Scores against quantized vectors, quantizing the query the same way the
//! stored vectors were quantized so the comparison is like for like.

use std::sync::Arc;

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
use crate::util::Bits;

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

/// Scores a float query, quantized the way the stored vectors were, against
/// one field's quantized vectors.
///
/// Equivalent to the anonymous `RandomVectorScorer.AbstractRandomVectorScorer`
/// that the quantized branch of
/// `ScalarQuantizedVectorScorer.getRandomVectorScorer(VectorSimilarityFunction,
/// KnnVectorValues, float[])` returns (`ScalarQuantizedVectorScorer.java:80-99`).
/// Like it, the scorer reads each stored vector and its corrective offset
/// through the values when it is asked to score it, and forwards `ordToDoc`,
/// `maxOrd` and `getAcceptOrds` to those values.
pub struct ScalarQuantizedRandomVectorScorer {
    similarity: Arc<dyn ScalarQuantizedVectorSimilarity>,
    target_bytes: Vec<u8>,
    offset_correction: f32,
    values: Box<dyn LegacyQuantizedByteVectorValues>,
}

impl std::fmt::Debug for ScalarQuantizedRandomVectorScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScalarQuantizedRandomVectorScorer")
            .field("size", &self.values.size())
            .finish_non_exhaustive()
    }
}

impl ScalarQuantizedRandomVectorScorer {
    /// Quantizes `target` with the values' own quantizer and binds the result
    /// to `values`.
    ///
    /// Equivalent to the body of the quantized branch of
    /// `ScalarQuantizedVectorScorer.getRandomVectorScorer(…, float[])`: it
    /// quantizes the target with `quantizeQuery`, derives the similarity with
    /// `ScalarQuantizedVectorSimilarity.fromVectorSimilarity`, and builds the
    /// scorer over the values themselves.
    ///
    /// # Errors
    ///
    /// Returns whatever quantizing the target raises.
    pub fn new(
        similarity_function: VectorSimilarityFunction,
        values: Box<dyn LegacyQuantizedByteVectorValues>,
        target: &[f32],
    ) -> Result<Self> {
        let scalar_quantizer = *values.scalar_quantizer();
        let mut target_bytes = vec![0u8; target.len()];
        let offset_correction = quantize_query(
            target,
            &mut target_bytes,
            similarity_function,
            &scalar_quantizer,
        )?;
        Ok(Self {
            similarity: Arc::from(
                <dyn ScalarQuantizedVectorSimilarity>::from_vector_similarity(
                    similarity_function,
                    scalar_quantizer.get_constant_multiplier(),
                    scalar_quantizer.get_bits(),
                ),
            ),
            target_bytes,
            offset_correction,
            values,
        })
    }
}

impl RandomVectorScorer for ScalarQuantizedRandomVectorScorer {
    /// Equivalent to the anonymous scorer's `score(int)`, which reads the
    /// stored vector and its corrective offset and scores them against the
    /// quantized target.
    fn score(&mut self, node: i32) -> Result<f32> {
        // Java reads the vector, then the correction, and holds both through
        // one reference. The slice `vector_value` lends borrows the values, so
        // the correction is read first here; the vector is then scored in
        // place, without a copy.
        let node_offset = self.values.get_score_correction_constant(node)?;
        let node_vector = self.values.vector_value(node)?;
        self.similarity.score(
            &self.target_bytes,
            self.offset_correction,
            node_vector,
            node_offset,
        )
    }

    /// Equivalent to `AbstractRandomVectorScorer.maxOrd()`, the size of the
    /// values.
    fn max_ord(&self) -> i32 {
        self.values.size()
    }

    /// Equivalent to `AbstractRandomVectorScorer.ordToDoc(int)`, which
    /// forwards to the values.
    fn ord_to_doc(&self, ord: i32) -> i32 {
        self.values.ord_to_doc(ord)
    }

    /// Equivalent to `AbstractRandomVectorScorer.getAcceptOrds(Bits)`; see
    /// `values.getAcceptOrds(acceptDocs)`.
    fn get_accept_ords<'a>(&self, accept_docs: Option<&'a dyn Bits>) -> Option<Box<dyn Bits + 'a>> {
        self.values.get_accept_ords(accept_docs)
    }
}

/// A [`FlatVectorsScorer`] that scores against quantized vectors when the field
/// holds them, and forwards to a plain scorer otherwise.
///
/// Equivalent to `org.apache.lucene.codecs.hnsw.ScalarQuantizedVectorScorer`.
///
/// Java's `instanceof LegacyQuantizedByteVectorValues` branch
/// (`ScalarQuantizedVectorScorer.java:66-71, 80-99`) is the pair of
/// [`FlatVectorsScorer`] methods
/// [`get_random_vector_scorer_supplier_legacy_quantized`](FlatVectorsScorer::get_random_vector_scorer_supplier_legacy_quantized)
/// and
/// [`get_random_vector_scorer_legacy_quantized`](FlatVectorsScorer::get_random_vector_scorer_legacy_quantized),
/// which this type overrides with [`ScalarQuantizedRandomVectorScorerSupplier`]
/// and [`ScalarQuantizedRandomVectorScorer`]; the four typed methods are Java's
/// unquantized branch and forward to the delegate. The trait documents why
/// the branch is a method of its own.
///
/// Lucene 10.5.0 itself never constructs this class outside its tests: the
/// 9.9 format's reader, in `backward-codecs`, uses
/// `Lucene99ScalarQuantizedVectorScorer` instead. It is public API, so the
/// port keeps it.
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

    /// Equivalent to the quantized branch of `getRandomVectorScorerSupplier`
    /// (`ScalarQuantizedVectorScorer.java:66-71`): it reads the quantizer off
    /// the values and returns a `ScalarQuantizedRandomVectorScorerSupplier`
    /// over them.
    fn get_random_vector_scorer_supplier_legacy_quantized(
        &self,
        similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn LegacyQuantizedByteVectorValues>,
    ) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        let scalar_quantizer = *vector_values.scalar_quantizer();
        Ok(Box::new(ScalarQuantizedRandomVectorScorerSupplier::new(
            similarity_function,
            &scalar_quantizer,
            vector_values,
        )))
    }

    /// Equivalent to the quantized branch of
    /// `getRandomVectorScorer(VectorSimilarityFunction, KnnVectorValues,
    /// float[])` (`ScalarQuantizedVectorScorer.java:80-99`).
    fn get_random_vector_scorer_legacy_quantized(
        &self,
        similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn LegacyQuantizedByteVectorValues>,
        target: &[f32],
    ) -> Result<Box<dyn RandomVectorScorer>> {
        Ok(Box::new(ScalarQuantizedRandomVectorScorer::new(
            similarity_function,
            vector_values,
            target,
        )?))
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
pub struct ScalarQuantizedRandomVectorScorerSupplier {
    values: Box<dyn LegacyQuantizedByteVectorValues>,
    similarity: Arc<dyn ScalarQuantizedVectorSimilarity>,
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
            similarity: Arc::from(
                <dyn ScalarQuantizedVectorSimilarity>::from_vector_similarity(
                    similarity_function,
                    scalar_quantizer.get_constant_multiplier(),
                    scalar_quantizer.get_bits(),
                ),
            ),
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
        similarity: Arc<dyn ScalarQuantizedVectorSimilarity>,
        vector_similarity_function: VectorSimilarityFunction,
        values: Box<dyn LegacyQuantizedByteVectorValues>,
    ) -> Self {
        Self {
            values,
            similarity,
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
            similarity: Arc::clone(&self.similarity),
            query_vector,
            query_offset: 0.0,
        }))
    }

    /// Equivalent to `ScalarQuantizedRandomVectorScorerSupplier.copy()`, which
    /// keeps the derived similarity and copies only the values.
    fn copy(&self) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        Ok(Box::new(Self::with_similarity(
            Arc::clone(&self.similarity),
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
    similarity: Arc<dyn ScalarQuantizedVectorSimilarity>,
    query_vector: Vec<u8>,
    query_offset: f32,
}

impl RandomVectorScorer for ScalarQuantizedUpdateableRandomVectorScorer {
    /// Equivalent to the anonymous scorer's `score(int)`, which reads the
    /// stored vector and its corrective offset and scores them against the
    /// query through `ScalarQuantizedVectorSimilarity`.
    fn score(&mut self, node: i32) -> Result<f32> {
        // Java reads the vector, then the correction, and holds both through
        // one reference. The slice `vector_value` lends borrows the values, so
        // the correction is read first here; the vector is then scored in
        // place, without a copy.
        let node_offset = self.vectors_copy.get_score_correction_constant(node)?;
        let node_vector = self.vectors_copy.vector_value(node)?;
        self.similarity.score(
            &self.query_vector,
            self.query_offset,
            node_vector,
            node_offset,
        )
    }

    /// Equivalent to `AbstractUpdateableRandomVectorScorer.maxOrd()`, which is
    /// the size of the values the scorer was built over.
    fn max_ord(&self) -> i32 {
        self.vectors_copy.size()
    }

    /// Equivalent to `AbstractUpdateableRandomVectorScorer.ordToDoc(int)`,
    /// which forwards to the values the scorer was built over.
    fn ord_to_doc(&self, ord: i32) -> i32 {
        self.vectors_copy.ord_to_doc(ord)
    }

    /// Equivalent to `AbstractUpdateableRandomVectorScorer.getAcceptOrds(Bits)`;
    /// see `values.getAcceptOrds(acceptDocs)`.
    fn get_accept_ords<'a>(&self, accept_docs: Option<&'a dyn Bits>) -> Option<Box<dyn Bits + 'a>> {
        self.vectors_copy.get_accept_ords(accept_docs)
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
