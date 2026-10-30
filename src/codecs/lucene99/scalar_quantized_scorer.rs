//! `Lucene99ScalarQuantizedVectorScorer` ported from
//! `org.apache.lucene.codecs.lucene99`.

use std::sync::Arc;

use crate::codecs::hnsw::flat_vectors::FlatVectorsScorer;
use crate::codecs::hnsw::scalar_quantized_scorer::quantize_query;
use crate::error::LuceneError;
use crate::error::Result;
use crate::index::{ByteVectorValues, FloatVectorValues, VectorSimilarityFunction};
use crate::util::hnsw::scorer::{
    RandomVectorScorer, RandomVectorScorerSupplier, UpdateableRandomVectorScorer,
};
use crate::util::quantization::LegacyQuantizedByteVectorValues;
use crate::util::vector_util;
use crate::util::Bits;

/// Scores against the vectors the Lucene 9.9 quantized format stores.
///
/// Equivalent to
/// `org.apache.lucene.codecs.lucene99.Lucene99ScalarQuantizedVectorScorer`.
///
/// Java's `instanceof LegacyQuantizedByteVectorValues` branch is the pair of
/// [`FlatVectorsScorer`] methods
/// [`get_random_vector_scorer_supplier_legacy_quantized`](FlatVectorsScorer::get_random_vector_scorer_supplier_legacy_quantized)
/// and
/// [`get_random_vector_scorer_legacy_quantized`](FlatVectorsScorer::get_random_vector_scorer_legacy_quantized),
/// which this type overrides; the four typed methods are Java's unquantized
/// branch and forward to the delegate. The trait documents why the branch is a
/// method of its own.
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

    /// Builds the scorer that matches `sim`, over vectors the Lucene 9.9 format
    /// quantized.
    ///
    /// The returned scorer holds `target_bytes` as its query; a caller that
    /// wants to score a *stored* vector against the rest of the field passes an
    /// empty buffer and then names the ordinal with
    /// [`set_scoring_ordinal`](UpdateableRandomVectorScorer::set_scoring_ordinal).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the query and the field
    /// disagree on dimension.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene99.Lucene99ScalarQuantizedVectorScorer#fromVectorSimilarity`.
    pub fn from_vector_similarity(
        target_bytes: Vec<u8>,
        offset_correction: f32,
        sim: VectorSimilarityFunction,
        const_multiplier: f32,
        values: Box<dyn LegacyQuantizedByteVectorValues>,
    ) -> Result<Box<dyn UpdateableRandomVectorScorer>> {
        // `FlatVectorsScorer.checkDimensions(values.dimension(), targetBytes.length)`
        // (`Lucene99ScalarQuantizedVectorScorer.java:99`), arguments in Java's order.
        crate::codecs::hnsw::flat_vectors::check_dimensions(
            values.dimension(),
            target_bytes.len() as i32,
        )?;
        match sim {
            VectorSimilarityFunction::EUCLIDEAN => Ok(Box::new(Euclidean::new(
                values,
                const_multiplier,
                target_bytes,
            ))),
            VectorSimilarityFunction::COSINE | VectorSimilarityFunction::DOT_PRODUCT => {
                Self::dot_product_factory(
                    target_bytes,
                    offset_correction,
                    const_multiplier,
                    values,
                    |f| java_math_max((1.0 + f) / 2.0, 0.0),
                )
            }
            VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT => Self::dot_product_factory(
                target_bytes,
                offset_correction,
                const_multiplier,
                values,
                vector_util::scale_max_inner_product_score,
            ),
        }
    }

    /// Picks the dot-product scorer that matches how the vectors are stored:
    /// four-bit components packed two to a byte are read straight from the
    /// file, four-bit components stored one per byte take the narrow kernel,
    /// and eight-bit components take the wide one.
    ///
    /// # Errors
    ///
    /// Returns whatever building the chosen scorer raises.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene99.Lucene99ScalarQuantizedVectorScorer#dotProductFactory`.
    pub fn dot_product_factory(
        target_bytes: Vec<u8>,
        offset_correction: f32,
        const_multiplier: f32,
        values: Box<dyn LegacyQuantizedByteVectorValues>,
        score_adjustment_function: fn(f32) -> f32,
    ) -> Result<Box<dyn UpdateableRandomVectorScorer>> {
        if values.scalar_quantizer().get_bits() <= 4 {
            if values.vector_byte_length() != values.dimension() && values.get_slice().is_some() {
                return Ok(Box::new(CompressedInt4DotProduct::new(
                    values,
                    const_multiplier,
                    target_bytes,
                    offset_correction,
                    score_adjustment_function,
                )));
            }
            return Ok(Box::new(Int4DotProduct::new(
                values,
                const_multiplier,
                target_bytes,
                offset_correction,
                score_adjustment_function,
            )));
        }
        Ok(Box::new(DotProduct::new(
            values,
            const_multiplier,
            target_bytes,
            offset_correction,
            score_adjustment_function,
        )))
    }
}

/// `Math.max(float, float)` as specified by the JLS, which propagates NaN where
/// `f32::max` would drop it.
#[inline]
fn java_math_max(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && a.is_sign_negative() {
        return b;
    }
    if a >= b {
        a
    } else {
        b
    }
}

/// Scores quantized vectors under the Euclidean similarity.
///
/// Equivalent to the private
/// `Lucene99ScalarQuantizedVectorScorer.Euclidean`.
struct Euclidean {
    const_multiplier: f32,
    target_bytes: Vec<u8>,
    values: Box<dyn LegacyQuantizedByteVectorValues>,
}

impl Euclidean {
    fn new(
        values: Box<dyn LegacyQuantizedByteVectorValues>,
        const_multiplier: f32,
        target_bytes: Vec<u8>,
    ) -> Self {
        Self {
            const_multiplier,
            target_bytes,
            values,
        }
    }
}

impl RandomVectorScorer for Euclidean {
    fn score(&mut self, node: i32) -> Result<f32> {
        let node_vector = self.values.vector_value(node)?;
        let square_distance = vector_util::uint8_square_distance(node_vector, &self.target_bytes)?;
        let adjusted_distance = square_distance as f32 * self.const_multiplier;
        Ok(1.0 / (1.0 + adjusted_distance))
    }

    fn max_ord(&self) -> i32 {
        self.values.size()
    }

    fn ord_to_doc(&self, ord: i32) -> i32 {
        self.values.ord_to_doc(ord)
    }

    fn get_accept_ords<'a>(&self, accept_docs: Option<&'a dyn Bits>) -> Option<Box<dyn Bits + 'a>> {
        self.values.get_accept_ords(accept_docs)
    }
}

impl UpdateableRandomVectorScorer for Euclidean {
    fn set_scoring_ordinal(&mut self, node: i32) -> Result<()> {
        let len = self.target_bytes.len();
        let vector = self.values.vector_value(node)?;
        self.target_bytes.copy_from_slice(&vector[..len]);
        Ok(())
    }
}

/// Scores quantized vectors with a dot product, applying the corrections both
/// sides carry.
///
/// Equivalent to the private
/// `Lucene99ScalarQuantizedVectorScorer.DotProduct`.
struct DotProduct {
    const_multiplier: f32,
    values: Box<dyn LegacyQuantizedByteVectorValues>,
    target_bytes: Vec<u8>,
    offset_correction: f32,
    score_adjustment_function: fn(f32) -> f32,
}

impl DotProduct {
    fn new(
        values: Box<dyn LegacyQuantizedByteVectorValues>,
        const_multiplier: f32,
        target_bytes: Vec<u8>,
        offset_correction: f32,
        score_adjustment_function: fn(f32) -> f32,
    ) -> Self {
        Self {
            const_multiplier,
            values,
            target_bytes,
            offset_correction,
            score_adjustment_function,
        }
    }
}

impl RandomVectorScorer for DotProduct {
    fn score(&mut self, vector_ordinal: i32) -> Result<f32> {
        let stored_vector = self.values.vector_value(vector_ordinal)?;
        let dot_product = vector_util::uint8_dot_product(stored_vector, &self.target_bytes)?;
        // For the current implementation of scalar quantization, all dot
        // products should be >= 0.
        debug_assert!(dot_product >= 0);
        let vector_offset = self.values.get_score_correction_constant(vector_ordinal)?;
        let adjusted_distance =
            dot_product as f32 * self.const_multiplier + self.offset_correction + vector_offset;
        Ok((self.score_adjustment_function)(adjusted_distance))
    }

    fn max_ord(&self) -> i32 {
        self.values.size()
    }

    fn ord_to_doc(&self, ord: i32) -> i32 {
        self.values.ord_to_doc(ord)
    }

    fn get_accept_ords<'a>(&self, accept_docs: Option<&'a dyn Bits>) -> Option<Box<dyn Bits + 'a>> {
        self.values.get_accept_ords(accept_docs)
    }
}

impl UpdateableRandomVectorScorer for DotProduct {
    fn set_scoring_ordinal(&mut self, node: i32) -> Result<()> {
        let len = self.target_bytes.len();
        let vector = self.values.vector_value(node)?;
        self.target_bytes.copy_from_slice(&vector[..len]);
        self.offset_correction = self.values.get_score_correction_constant(node)?;
        Ok(())
    }
}

/// Scores four-bit vectors stored two components to a byte, reading the packed
/// form straight out of the file rather than the decompressed one
/// `QuantizedByteVectorValues::vector_value` hands back.
///
/// Equivalent to the private
/// `Lucene99ScalarQuantizedVectorScorer.CompressedInt4DotProduct`.
struct CompressedInt4DotProduct {
    const_multiplier: f32,
    values: Box<dyn LegacyQuantizedByteVectorValues>,
    compressed_vector: Vec<u8>,
    target_bytes: Vec<u8>,
    offset_correction: f32,
    score_adjustment_function: fn(f32) -> f32,
}

impl CompressedInt4DotProduct {
    fn new(
        values: Box<dyn LegacyQuantizedByteVectorValues>,
        const_multiplier: f32,
        target_bytes: Vec<u8>,
        offset_correction: f32,
        score_adjustment_function: fn(f32) -> f32,
    ) -> Self {
        let compressed_vector = vec![0u8; values.vector_byte_length().max(0) as usize];
        Self {
            const_multiplier,
            values,
            compressed_vector,
            target_bytes,
            offset_correction,
            score_adjustment_function,
        }
    }
}

impl RandomVectorScorer for CompressedInt4DotProduct {
    fn score(&mut self, vector_ordinal: i32) -> Result<f32> {
        // Get compressed vector: in Lucene99, vector values are stored and have
        // a single value for offset correction.
        let byte_length = i64::from(self.values.vector_byte_length());
        let Self {
            values,
            compressed_vector,
            ..
        } = self;
        let slice = values.get_slice_mut().ok_or_else(|| {
            LuceneError::illegal_state(
                "compressed int4 scoring needs the values' index slice".to_string(),
            )
        })?;
        slice
            .seek(i64::from(vector_ordinal) * (byte_length + std::mem::size_of::<f32>() as i64))?;
        let len = compressed_vector.len();
        slice.read_bytes(compressed_vector, 0, len)?;
        let vector_offset = self.values.get_score_correction_constant(vector_ordinal)?;
        let dot_product = vector_util::int4_dot_product_single_packed(
            &self.target_bytes,
            &self.compressed_vector,
        )?;
        // For the current implementation of scalar quantization, all dot
        // products should be >= 0.
        debug_assert!(dot_product >= 0);
        let adjusted_distance =
            dot_product as f32 * self.const_multiplier + self.offset_correction + vector_offset;
        Ok((self.score_adjustment_function)(adjusted_distance))
    }

    fn max_ord(&self) -> i32 {
        self.values.size()
    }

    fn ord_to_doc(&self, ord: i32) -> i32 {
        self.values.ord_to_doc(ord)
    }

    fn get_accept_ords<'a>(&self, accept_docs: Option<&'a dyn Bits>) -> Option<Box<dyn Bits + 'a>> {
        self.values.get_accept_ords(accept_docs)
    }
}

impl UpdateableRandomVectorScorer for CompressedInt4DotProduct {
    fn set_scoring_ordinal(&mut self, node: i32) -> Result<()> {
        let len = self.target_bytes.len();
        let vector = self.values.vector_value(node)?;
        self.target_bytes.copy_from_slice(&vector[..len]);
        self.offset_correction = self.values.get_score_correction_constant(node)?;
        Ok(())
    }
}

/// Scores four-bit vectors stored one component to a byte.
///
/// Equivalent to the private
/// `Lucene99ScalarQuantizedVectorScorer.Int4DotProduct`.
struct Int4DotProduct {
    const_multiplier: f32,
    values: Box<dyn LegacyQuantizedByteVectorValues>,
    target_bytes: Vec<u8>,
    offset_correction: f32,
    score_adjustment_function: fn(f32) -> f32,
}

impl Int4DotProduct {
    fn new(
        values: Box<dyn LegacyQuantizedByteVectorValues>,
        const_multiplier: f32,
        target_bytes: Vec<u8>,
        offset_correction: f32,
        score_adjustment_function: fn(f32) -> f32,
    ) -> Self {
        Self {
            const_multiplier,
            values,
            target_bytes,
            offset_correction,
            score_adjustment_function,
        }
    }
}

impl RandomVectorScorer for Int4DotProduct {
    fn score(&mut self, vector_ordinal: i32) -> Result<f32> {
        let stored_vector = self.values.vector_value(vector_ordinal)?;
        let dot_product = vector_util::int4_dot_product(stored_vector, &self.target_bytes)?;
        // For the current implementation of scalar quantization, all dot
        // products should be >= 0.
        debug_assert!(dot_product >= 0);
        let vector_offset = self.values.get_score_correction_constant(vector_ordinal)?;
        let adjusted_distance =
            dot_product as f32 * self.const_multiplier + self.offset_correction + vector_offset;
        Ok((self.score_adjustment_function)(adjusted_distance))
    }

    fn max_ord(&self) -> i32 {
        self.values.size()
    }

    fn ord_to_doc(&self, ord: i32) -> i32 {
        self.values.ord_to_doc(ord)
    }

    fn get_accept_ords<'a>(&self, accept_docs: Option<&'a dyn Bits>) -> Option<Box<dyn Bits + 'a>> {
        self.values.get_accept_ords(accept_docs)
    }
}

impl UpdateableRandomVectorScorer for Int4DotProduct {
    fn set_scoring_ordinal(&mut self, node: i32) -> Result<()> {
        let len = self.target_bytes.len();
        let vector = self.values.vector_value(node)?;
        self.target_bytes.copy_from_slice(&vector[..len]);
        self.offset_correction = self.values.get_score_correction_constant(node)?;
        Ok(())
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

    /// Equivalent to the `instanceof LegacyQuantizedByteVectorValues` branch of
    /// `getRandomVectorScorerSupplier`
    /// (`Lucene99ScalarQuantizedVectorScorer.java:53-56`).
    fn get_random_vector_scorer_supplier_legacy_quantized(
        &self,
        similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn LegacyQuantizedByteVectorValues>,
    ) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        Self::legacy_supplier(similarity_function, vector_values)
    }

    /// Equivalent to the `instanceof LegacyQuantizedByteVectorValues` branch of
    /// `getRandomVectorScorer(VectorSimilarityFunction, KnnVectorValues,
    /// float[])` (`Lucene99ScalarQuantizedVectorScorer.java:65-76`).
    fn get_random_vector_scorer_legacy_quantized(
        &self,
        similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn LegacyQuantizedByteVectorValues>,
        target: &[f32],
    ) -> Result<Box<dyn RandomVectorScorer>> {
        Self::legacy_scorer(similarity_function, vector_values, target)
    }
}

impl Lucene99ScalarQuantizedVectorScorer {
    /// The quantized branch of `getRandomVectorScorerSupplier`.
    fn legacy_supplier(
        similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn LegacyQuantizedByteVectorValues>,
    ) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        Ok(Box::new(ScalarQuantizedRandomVectorScorerSupplier::new(
            vector_values,
            similarity_function,
        )?))
    }

    /// The quantized branch of `getRandomVectorScorer(…, float[])`.
    fn legacy_scorer(
        similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn LegacyQuantizedByteVectorValues>,
        target: &[f32],
    ) -> Result<Box<dyn RandomVectorScorer>> {
        let scalar_quantizer = *vector_values.scalar_quantizer();
        let mut target_bytes = vec![0u8; target.len()];
        let offset_correction = quantize_query(
            target,
            &mut target_bytes,
            similarity_function,
            &scalar_quantizer,
        )?;
        let scorer: Box<dyn RandomVectorScorer> = Self::from_vector_similarity(
            target_bytes,
            offset_correction,
            similarity_function,
            scalar_quantizer.get_constant_multiplier(),
            vector_values,
        )?;
        Ok(scorer)
    }
}

/// Supplies scorers that score one stored quantized vector against the rest
/// of the field.
///
/// Equivalent to the private
/// `Lucene99ScalarQuantizedVectorScorer.ScalarQuantizedRandomVectorScorerSupplier`
/// (`Lucene99ScalarQuantizedVectorScorer.java:300-337`).
///
/// **Divergence from Lucene 10.5.0.** Java hands every scorer the one
/// `targetVectors` instance the supplier copied at construction. A scorer here
/// owns the values it reads through `&mut self`, so each scorer gets its own
/// copy of `targetVectors`; the vectors it reads are the same.
struct ScalarQuantizedRandomVectorScorerSupplier {
    vector_similarity_function: VectorSimilarityFunction,
    values: Box<dyn LegacyQuantizedByteVectorValues>,
    target_vectors: Box<dyn LegacyQuantizedByteVectorValues>,
}

impl ScalarQuantizedRandomVectorScorerSupplier {
    /// Equivalent to the constructor, which keeps `values` and a copy of them
    /// as `targetVectors`.
    fn new(
        values: Box<dyn LegacyQuantizedByteVectorValues>,
        vector_similarity_function: VectorSimilarityFunction,
    ) -> Result<Self> {
        let target_vectors = values.copy_legacy()?;
        Ok(Self {
            vector_similarity_function,
            values,
            target_vectors,
        })
    }
}

impl RandomVectorScorerSupplier for ScalarQuantizedRandomVectorScorerSupplier {
    /// Equivalent to `scorer()`: a scorer over `targetVectors` with an
    /// all-zero query of the field's dimension and no offset, to be filled by
    /// `setScoringOrdinal`.
    fn scorer(&self) -> Result<Box<dyn UpdateableRandomVectorScorer>> {
        let vector_value = vec![0u8; self.values.dimension().max(0) as usize];
        let offset_correction = 0.0;
        Lucene99ScalarQuantizedVectorScorer::from_vector_similarity(
            vector_value,
            offset_correction,
            self.vector_similarity_function,
            self.values.scalar_quantizer().get_constant_multiplier(),
            self.target_vectors.copy_legacy()?,
        )
    }

    /// Equivalent to `copy()`, which builds a supplier over `values.copy()`.
    fn copy(&self) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        Ok(Box::new(Self::new(
            self.values.copy_legacy()?,
            self.vector_similarity_function,
        )?))
    }
}

impl std::fmt::Display for ScalarQuantizedRandomVectorScorerSupplier {
    /// Port of `ScalarQuantizedRandomVectorScorerSupplier.toString()`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ScalarQuantizedRandomVectorScorerSupplier(vectorSimilarityFunction={:?})",
            self.vector_similarity_function
        )
    }
}

impl std::fmt::Display for Lucene99ScalarQuantizedVectorScorer {
    /// Port of `Lucene99ScalarQuantizedVectorScorer.toString()`, which names
    /// itself `ScalarQuantizedVectorScorer` — the same text the
    /// `org.apache.lucene.codecs.hnsw` class of that name produces.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ScalarQuantizedVectorScorer(nonQuantizedDelegate={})",
            self.non_quantized_delegate
        )
    }
}
