//! Port of
//! `org.apache.lucene.internal.vectorization.Lucene99MemorySegmentScalarQuantizedVectorScorer`.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::codecs::hnsw::scalar_quantized_scorer::quantize_query;
use crate::codecs::hnsw::{DefaultFlatVectorScorer, FlatVectorsScorer};
use crate::codecs::lucene99::scalar_quantized_scorer::Lucene99ScalarQuantizedVectorScorer;
use crate::error::{LuceneError, Result};
use crate::index::vector_values::{ByteVectorValues, FloatVectorValues};
use crate::index::VectorSimilarityFunction;
use crate::internal::vectorization::lucene99_memory_segment_byte_vector_scorer::{
    check_ordinal, load_vector,
};
use crate::internal::vectorization::PanamaVectorUtilSupport;
use crate::store::MemorySegmentAccessInput;
use crate::util::hnsw::{
    RandomVectorScorer, RandomVectorScorerSupplier, UpdateableRandomVectorScorer,
};
use crate::util::quantization::{LegacyQuantizedByteVectorValues, ScalarQuantizer};
use crate::util::vector_util;

/// Scores Lucene 9.9 scalar-quantized vectors straight out of a mapped segment.
///
/// Equivalent to
/// `org.apache.lucene.internal.vectorization.Lucene99MemorySegmentScalarQuantizedVectorScorer`.
///
/// Each stored node is the quantized vector followed by its four-byte
/// corrective offset. This scorer reads both in place from the memory-mapped
/// file instead of copying the vector to the heap, and only falls back to a
/// scratch buffer for a node that straddles a segment boundary.
///
/// # Divergences from Lucene 10.5.0
///
/// * **The caller narrows the values and the input.** Java tests
///   `vectorValues instanceof LegacyQuantizedByteVectorValues quantized &&
///   quantized.getSlice() instanceof MemorySegmentAccessInput` inside the
///   `FlatVectorsScorer` methods and delegates when either test fails. Rust
///   cannot test one trait object for another trait, and this port's
///   `QuantizedByteVectorValues` does not expose the backing slice, so the
///   [`FlatVectorsScorer`] implementation always delegates — Lucene's own
///   fallback branch — and the memory-segment path is reached through
///   [`random_vector_scorer`](Self::random_vector_scorer) and
///   [`random_vector_scorer_supplier`](Self::random_vector_scorer_supplier),
///   which take the narrowed types.
/// * **Enum dispatch instead of method references.** Java picks a `Scorer`
///   functional-interface implementation and a `FloatToFloatFunction` scaler in
///   the constructor; the port stores the same two choices as
///   [`VectorSimilarityFunction`] plus a private kernel discriminant.
/// * **The vector byte length comes from the encoding.** Java reads
///   `values.getVectorByteLength()` off `KnnVectorValues`; this port's
///   `QuantizedByteVectorValues` is not a `KnnVectorValues`, so the length is
///   `ScalarEncoding::get_doc_length(dimension)`, which is the same quantity.
/// * **`ord_to_doc` is the identity.** Java keeps the values object to answer
///   `ordToDoc` and `getAcceptOrds`; this port's `QuantizedByteVectorValues`
///   exposes neither, so the default identity mapping applies.
/// * **The kernels are scalar**, as explained on [`PanamaVectorUtilSupport`].
#[derive(Debug, Clone, Copy)]
pub struct Lucene99MemorySegmentScalarQuantizedVectorScorer;

impl Lucene99MemorySegmentScalarQuantizedVectorScorer {
    /// The singleton instance.
    ///
    /// Equivalent to `Lucene99MemorySegmentScalarQuantizedVectorScorer.INSTANCE`.
    pub const INSTANCE: Self = Self;

    /// Returns the scorer Lucene delegates to when the memory-segment path does
    /// not apply.
    ///
    /// Equivalent to the private static `DELEGATE` field, which is
    /// `new Lucene99ScalarQuantizedVectorScorer(DefaultFlatVectorScorer.INSTANCE)`.
    fn delegate() -> Lucene99ScalarQuantizedVectorScorer {
        Lucene99ScalarQuantizedVectorScorer::new(Arc::new(DefaultFlatVectorScorer::INSTANCE))
    }

    /// Returns a scorer for a float `target`, reading the stored vectors in
    /// place from `input`.
    ///
    /// Equivalent to the `RandomVectorScorerImpl` branch of
    /// `getRandomVectorScorer(VectorSimilarityFunction, KnnVectorValues, float[])`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `input` is shorter than
    /// the node data it is supposed to hold, and propagates any error raised
    /// while quantizing the query or cloning the input.
    pub fn random_vector_scorer(
        similarity_function: VectorSimilarityFunction,
        values: &dyn LegacyQuantizedByteVectorValues,
        input: &dyn MemorySegmentAccessInput,
        target: &[f32],
    ) -> Result<Box<dyn RandomVectorScorer>> {
        let base = QuantizedScorerBase::new(similarity_function, values, input)?;
        let mut target_bytes = vec![0u8; target.len()];
        let query_offset = quantize_query(
            target,
            &mut target_bytes,
            similarity_function,
            &base.quantizer,
        )?;
        Ok(Box::new(HeapQueryScorer {
            base,
            target_bytes,
            query_offset,
        }))
    }

    /// Returns a scorer supplier that scores stored vectors against each other,
    /// both read in place from `input`.
    ///
    /// Equivalent to the `RandomVectorScorerSupplierImpl` branch of
    /// `getRandomVectorScorerSupplier(VectorSimilarityFunction, KnnVectorValues)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `input` is shorter than
    /// the node data it is supposed to hold, and propagates any error raised
    /// while cloning the input.
    pub fn random_vector_scorer_supplier(
        similarity_function: VectorSimilarityFunction,
        values: &dyn LegacyQuantizedByteVectorValues,
        input: &dyn MemorySegmentAccessInput,
    ) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        Ok(Box::new(SegmentQuerySupplier {
            base: QuantizedScorerBase::new(similarity_function, values, input)?,
        }))
    }
}

impl FlatVectorsScorer for Lucene99MemorySegmentScalarQuantizedVectorScorer {
    fn get_random_vector_scorer_supplier_float(
        &self,
        similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn FloatVectorValues>,
    ) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        Self::delegate().get_random_vector_scorer_supplier_float(similarity_function, vector_values)
    }

    fn get_random_vector_scorer_supplier_byte(
        &self,
        similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn ByteVectorValues>,
    ) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        Self::delegate().get_random_vector_scorer_supplier_byte(similarity_function, vector_values)
    }

    fn get_random_vector_scorer_float(
        &self,
        similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn FloatVectorValues>,
        target: &[f32],
    ) -> Result<Box<dyn RandomVectorScorer>> {
        Self::delegate().get_random_vector_scorer_float(similarity_function, vector_values, target)
    }

    fn get_random_vector_scorer_byte(
        &self,
        similarity_function: VectorSimilarityFunction,
        vector_values: Box<dyn ByteVectorValues>,
        target: &[u8],
    ) -> Result<Box<dyn RandomVectorScorer>> {
        // Java routes the byte query straight to the delegate too.
        Self::delegate().get_random_vector_scorer_byte(similarity_function, vector_values, target)
    }
}

/// Which integer kernel scores one quantized vector against another.
///
/// Equivalent to the `Scorer` functional interface Lucene binds in
/// `RandomVectorScorerBase`'s constructor with a method reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuantizedKernel {
    /// `euclidean`: unsigned-byte square distance.
    Euclidean,
    /// `int4Euclidean`: one component per byte.
    Int4Euclidean,
    /// `compressedInt4Euclidean`: two components packed per byte.
    CompressedInt4Euclidean,
    /// `dotProduct`: unsigned-byte dot product.
    DotProduct,
    /// `int4DotProduct`: one component per byte.
    Int4DotProduct,
    /// `compressedInt4DotProduct`: two components packed per byte.
    CompressedInt4DotProduct,
}

/// The state every memory-segment quantized scorer carries.
///
/// Equivalent to the private abstract
/// `Lucene99MemorySegmentScalarQuantizedVectorScorer.RandomVectorScorerBase`.
struct QuantizedScorerBase {
    quantizer: ScalarQuantizer,
    const_multiplier: f32,
    similarity_function: VectorSimilarityFunction,
    kernel: QuantizedKernel,
    input: Box<dyn MemorySegmentAccessInput>,
    vector_byte_size: usize,
    node_size: usize,
    max_ord: i32,
    scratch: Vec<u8>,
}

impl QuantizedScorerBase {
    /// Equivalent to
    /// `RandomVectorScorerBase(VectorSimilarityFunction, LegacyQuantizedByteVectorValues, MemorySegmentAccessInput)`.
    fn new(
        similarity_function: VectorSimilarityFunction,
        values: &dyn LegacyQuantizedByteVectorValues,
        input: &dyn MemorySegmentAccessInput,
    ) -> Result<Self> {
        let quantizer = *values.scalar_quantizer();
        let dimension = values.dimension();
        let vector_byte_size = values
            .get_scalar_encoding()
            .get_doc_length(dimension as usize);
        let node_size = vector_byte_size + std::mem::size_of::<f32>();
        let max_ord = values.size();

        let compressed = vector_byte_size != dimension as usize;
        let kernel = match similarity_function {
            VectorSimilarityFunction::EUCLIDEAN => {
                if quantizer.get_bits() <= 4 {
                    if compressed {
                        QuantizedKernel::CompressedInt4Euclidean
                    } else {
                        QuantizedKernel::Int4Euclidean
                    }
                } else {
                    QuantizedKernel::Euclidean
                }
            }
            _ => {
                if quantizer.get_bits() <= 4 {
                    if compressed {
                        QuantizedKernel::CompressedInt4DotProduct
                    } else {
                        QuantizedKernel::Int4DotProduct
                    }
                } else {
                    QuantizedKernel::DotProduct
                }
            }
        };

        // Equivalent to `checkInvariants()`.
        if input.length() < (node_size as i64) * i64::from(max_ord) {
            return Err(LuceneError::IllegalArgument(
                "input length is less than expected vector data".to_string(),
            ));
        }

        Ok(Self {
            const_multiplier: quantizer.get_constant_multiplier(),
            quantizer,
            similarity_function,
            kernel,
            input: input.clone_access_input()?,
            vector_byte_size,
            node_size,
            max_ord,
            scratch: Vec::new(),
        })
    }

    /// Returns the quantized vector and corrective offset stored at `ord`.
    ///
    /// Equivalent to `RandomVectorScorerBase.getNode(int)`. The vector is
    /// returned as an owned buffer rather than as a view; see the divergence
    /// note on [`SegmentQueryScorer`].
    fn get_node(&mut self, ord: i32) -> Result<(Vec<u8>, f32)> {
        check_ordinal(ord, self.max_ord)?;
        let byte_offset = i64::from(ord) * self.node_size as i64;
        let located = load_vector(
            self.input.as_mut(),
            &mut self.scratch,
            byte_offset,
            self.node_size,
        )?;
        let node = located.resolve(&self.scratch);
        let vector = node[..self.vector_byte_size].to_vec();
        let offset = f32::from_le_bytes([
            node[self.vector_byte_size],
            node[self.vector_byte_size + 1],
            node[self.vector_byte_size + 2],
            node[self.vector_byte_size + 3],
        ]);
        Ok((vector, offset))
    }

    /// Scores `doc` against `query` with the kernel chosen for this similarity.
    ///
    /// Equivalent to calling the bound `Scorer.score(MemorySegment)`. `packed`
    /// says whether the query is itself a packed stored vector, which selects
    /// between Lucene's `*SinglePacked` and `*BothPacked` kernels.
    fn kernel_score(&self, query: &[u8], doc: &[u8], query_is_packed: bool) -> Result<i32> {
        match self.kernel {
            QuantizedKernel::Euclidean => {
                PanamaVectorUtilSupport::uint8_square_distance(query, doc)
            }
            QuantizedKernel::Int4Euclidean => {
                PanamaVectorUtilSupport::int4_square_distance(query, doc)
            }
            QuantizedKernel::CompressedInt4Euclidean => {
                if query_is_packed {
                    PanamaVectorUtilSupport::int4_square_distance_both_packed(query, doc)
                } else {
                    PanamaVectorUtilSupport::int4_square_distance_single_packed(query, doc)
                }
            }
            QuantizedKernel::DotProduct => PanamaVectorUtilSupport::uint8_dot_product(query, doc),
            QuantizedKernel::Int4DotProduct => {
                PanamaVectorUtilSupport::int4_dot_product(query, doc)
            }
            QuantizedKernel::CompressedInt4DotProduct => {
                if query_is_packed {
                    PanamaVectorUtilSupport::int4_dot_product_both_packed(query, doc)
                } else {
                    PanamaVectorUtilSupport::int4_dot_product_single_packed(query, doc)
                }
            }
        }
    }

    /// Applies the score scaler bound for this similarity.
    ///
    /// Equivalent to the `FloatToFloatFunction scaler` field.
    fn scale(&self, value: f32) -> f32 {
        match self.similarity_function {
            VectorSimilarityFunction::EUCLIDEAN => {
                vector_util::normalize_distance_to_unit_interval(value)
            }
            VectorSimilarityFunction::DOT_PRODUCT | VectorSimilarityFunction::COSINE => {
                vector_util::normalize_to_unit_interval(value)
            }
            VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT => {
                vector_util::scale_max_inner_product_score(value)
            }
        }
    }

    /// Equivalent to `RandomVectorScorerBase.scoreBody(int, float)`.
    fn score_body(
        &mut self,
        ord: i32,
        query: &[u8],
        query_is_packed: bool,
        query_offset: f32,
    ) -> Result<f32> {
        check_ordinal(ord, self.max_ord)?;
        let (vector, node_offset) = self.get_node(ord)?;
        let raw = self.kernel_score(query, &vector, query_is_packed)? as f32;
        Ok(self.scale(raw * self.const_multiplier + node_offset + query_offset))
    }
}

/// Scores stored vectors against a quantized float query held on the heap.
///
/// Equivalent to the private
/// `Lucene99MemorySegmentScalarQuantizedVectorScorer.RandomVectorScorerImpl`.
struct HeapQueryScorer {
    base: QuantizedScorerBase,
    target_bytes: Vec<u8>,
    query_offset: f32,
}

impl RandomVectorScorer for HeapQueryScorer {
    fn score(&mut self, node: i32) -> Result<f32> {
        self.base
            .score_body(node, &self.target_bytes, false, self.query_offset)
    }

    fn max_ord(&self) -> i32 {
        self.base.max_ord
    }
}

/// Supplies scorers that compare two stored vectors.
///
/// Equivalent to the private record
/// `Lucene99MemorySegmentScalarQuantizedVectorScorer.RandomVectorScorerSupplierImpl`.
struct SegmentQuerySupplier {
    base: QuantizedScorerBase,
}

impl RandomVectorScorerSupplier for SegmentQuerySupplier {
    fn scorer(&self) -> Result<Box<dyn UpdateableRandomVectorScorer>> {
        Ok(Box::new(SegmentQueryScorer {
            base: self.base.try_clone()?,
            query: Vec::new(),
            query_offset: 0.0,
        }))
    }

    fn copy(&self) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        Ok(Box::new(Self {
            base: self.base.try_clone()?,
        }))
    }
}

impl QuantizedScorerBase {
    /// Clones the state, giving the copy its own handle on the input.
    ///
    /// Java shares the `MemorySegmentAccessInput` between a supplier and every
    /// scorer it creates; Rust needs one owner per scorer because reading
    /// through the input takes `&mut self`.
    fn try_clone(&self) -> Result<Self> {
        Ok(Self {
            quantizer: self.quantizer,
            const_multiplier: self.const_multiplier,
            similarity_function: self.similarity_function,
            kernel: self.kernel,
            input: self.input.clone_access_input()?,
            vector_byte_size: self.vector_byte_size,
            node_size: self.node_size,
            max_ord: self.max_ord,
            scratch: Vec::new(),
        })
    }
}

/// Scores one stored vector against another, both read from the segment.
///
/// Equivalent to the private
/// `Lucene99MemorySegmentScalarQuantizedVectorScorer.UpdateableRandomVectorScorerImpl`.
///
/// # Divergence from Lucene 10.5.0
///
/// Java stores the query as the `MemorySegment` that `getNode` returned. When
/// the query node straddles a segment boundary that segment *is* the scratch
/// buffer, which the next `getNode` call overwrites, so the query silently
/// changes underneath the scorer. Rust's ownership rules forbid that alias, and
/// reproducing it would mean reproducing a latent correctness hazard, so this
/// port copies the query vector when the scoring ordinal is set.
struct SegmentQueryScorer {
    base: QuantizedScorerBase,
    query: Vec<u8>,
    query_offset: f32,
}

impl RandomVectorScorer for SegmentQueryScorer {
    fn score(&mut self, node: i32) -> Result<f32> {
        self.base
            .score_body(node, &self.query, true, self.query_offset)
    }

    fn max_ord(&self) -> i32 {
        self.base.max_ord
    }
}

impl UpdateableRandomVectorScorer for SegmentQueryScorer {
    fn set_scoring_ordinal(&mut self, ord: i32) -> Result<()> {
        check_ordinal(ord, self.base.max_ord)?;
        let (vector, offset) = self.base.get_node(ord)?;
        self.query = vector;
        self.query_offset = offset;
        Ok(())
    }
}
