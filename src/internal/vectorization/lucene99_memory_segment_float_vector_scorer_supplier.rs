//! Port of
//! `org.apache.lucene.internal.vectorization.Lucene99MemorySegmentFloatVectorScorerSupplier`.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::error::Result;
use crate::index::vector_values::FloatVectorValues;
use crate::index::VectorSimilarityFunction;
use crate::internal::vectorization::lucene99_memory_segment_byte_vector_scorer::{
    check_invariants, check_ordinal,
};
use crate::internal::vectorization::lucene99_memory_segment_float_vector_scorer::{
    java_math_max, normalize_raw_score,
};
use crate::internal::vectorization::MemorySegmentBulkVectorOps;
use crate::store::{MemorySegment, MemorySegmentAccessInput};
use crate::util::hnsw::{
    RandomVectorScorer, RandomVectorScorerSupplier, UpdateableRandomVectorScorer,
};

/// A score supplier of vectors whose element size is float.
///
/// Equivalent to
/// `org.apache.lucene.internal.vectorization.Lucene99MemorySegmentFloatVectorScorerSupplier`.
///
/// Both the query vector and the four scored vectors are read in place from the
/// mapped file, so an HNSW neighbour block is scored without copying anything
/// to the heap.
///
/// # Divergences from Lucene 10.5.0
///
/// * **One type instead of a sealed hierarchy**, matching on
///   [`VectorSimilarityFunction`] where Java has four subclasses.
/// * **The caller narrows the input**, because Rust cannot test a
///   `dyn IndexInput` for [`MemorySegmentAccessInput`]. `None` is still
///   returned for Lucene's other reason: the data does not fit one segment.
/// * **The kernels are scalar**, as explained on
///   [`MemorySegmentBulkVectorOps`].
pub struct Lucene99MemorySegmentFloatVectorScorerSupplier {
    similarity: VectorSimilarityFunction,
    vector_byte_size: i64,
    max_ord: i32,
    dims: usize,
    seg: MemorySegment,
    values: Arc<dyn FloatVectorValues>,
}

impl std::fmt::Debug for Lucene99MemorySegmentFloatVectorScorerSupplier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lucene99MemorySegmentFloatVectorScorerSupplier")
            .field("similarity", &self.similarity)
            .field("vectorByteSize", &self.vector_byte_size)
            .field("maxOrd", &self.max_ord)
            .field("dims", &self.dims)
            .finish()
    }
}

impl Lucene99MemorySegmentFloatVectorScorerSupplier {
    /// Creates the supplier, or returns `None` when the vector data does not
    /// fit in a single mapped segment.
    ///
    /// Equivalent to
    /// `Lucene99MemorySegmentFloatVectorScorerSupplier.create(VectorSimilarityFunction, IndexInput, FloatVectorValues)`,
    /// minus the `instanceof` narrowing described on the type.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneError::IllegalArgument`] when the input is
    /// shorter than the vector data it is supposed to hold, and propagates any
    /// error raised while slicing the input.
    pub fn create(
        similarity: VectorSimilarityFunction,
        input: &dyn MemorySegmentAccessInput,
        values: Arc<dyn FloatVectorValues>,
    ) -> Result<Option<Self>> {
        let Some(seg) = input.segment_slice_or_null(0, input.length())? else {
            return Ok(None);
        };
        check_invariants(values.size(), values.vector_byte_length() as usize, input)?;
        Ok(Some(Self {
            similarity,
            vector_byte_size: i64::from(values.vector_byte_length()),
            max_ord: values.size(),
            dims: values.dimension() as usize,
            seg,
            values,
        }))
    }
}

impl RandomVectorScorerSupplier for Lucene99MemorySegmentFloatVectorScorerSupplier {
    fn scorer(&self) -> Result<Box<dyn UpdateableRandomVectorScorer>> {
        Ok(Box::new(AbstractBulkScorer {
            similarity: self.similarity,
            vector_byte_size: self.vector_byte_size,
            max_ord: self.max_ord,
            dims: self.dims,
            seg: self.seg.clone(),
            values: Arc::clone(&self.values),
            query_ord: 0,
            scratch_scores: [0.0; 4],
        }))
    }

    fn copy(&self) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        Ok(Box::new(Self {
            similarity: self.similarity,
            vector_byte_size: self.vector_byte_size,
            max_ord: self.max_ord,
            dims: self.dims,
            seg: self.seg.clone(),
            values: self.values.copy_float()?.into(),
        }))
    }
}

/// The scorer [`Lucene99MemorySegmentFloatVectorScorerSupplier::scorer`] hands
/// out.
///
/// Equivalent to the package-private
/// `Lucene99MemorySegmentFloatVectorScorerSupplier.AbstractBulkScorer`, whose
/// four anonymous subclasses supply the per-similarity operations; here the
/// similarity is a field.
struct AbstractBulkScorer {
    similarity: VectorSimilarityFunction,
    vector_byte_size: i64,
    max_ord: i32,
    dims: usize,
    seg: MemorySegment,
    values: Arc<dyn FloatVectorValues>,
    query_ord: i32,
    scratch_scores: [f32; 4],
}

impl AbstractBulkScorer {
    /// Returns the raw similarity of two vectors that both live in the segment.
    ///
    /// Equivalent to the abstract
    /// `AbstractBulkScorer.vectorOp(MemorySegment, long, long, int)`.
    fn vector_op(&self, q: i64, d: i64) -> f32 {
        match self.similarity {
            VectorSimilarityFunction::COSINE => {
                MemorySegmentBulkVectorOps::COS_INSTANCE.cosine(&self.seg, q, d, self.dims)
            }
            VectorSimilarityFunction::DOT_PRODUCT
            | VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT => {
                MemorySegmentBulkVectorOps::DOT_INSTANCE.dot_product(&self.seg, q, d, self.dims)
            }
            VectorSimilarityFunction::EUCLIDEAN => {
                MemorySegmentBulkVectorOps::SQR_INSTANCE.sqr_distance(&self.seg, q, d, self.dims)
            }
        }
    }

    /// Fills `scores[0..4]` with the raw similarities of four document vectors
    /// against a query that also lives in the segment.
    ///
    /// Equivalent to the abstract
    /// `AbstractBulkScorer.vectorOp(MemorySegment, float[], long, long, long, long, long, int)`.
    #[allow(clippy::too_many_arguments)]
    fn bulk_vector_op(
        similarity: VectorSimilarityFunction,
        seg: &MemorySegment,
        scores: &mut [f32],
        query_offset: i64,
        node1_offset: i64,
        node2_offset: i64,
        node3_offset: i64,
        node4_offset: i64,
        dims: usize,
    ) {
        match similarity {
            VectorSimilarityFunction::COSINE => MemorySegmentBulkVectorOps::COS_INSTANCE
                .cosine_bulk_at(
                    seg,
                    scores,
                    query_offset,
                    node1_offset,
                    node2_offset,
                    node3_offset,
                    node4_offset,
                    dims,
                ),
            VectorSimilarityFunction::DOT_PRODUCT
            | VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT => {
                MemorySegmentBulkVectorOps::DOT_INSTANCE.dot_product_bulk_at(
                    seg,
                    scores,
                    query_offset,
                    node1_offset,
                    node2_offset,
                    node3_offset,
                    node4_offset,
                    dims,
                )
            }
            VectorSimilarityFunction::EUCLIDEAN => MemorySegmentBulkVectorOps::SQR_INSTANCE
                .sqr_distance_bulk_at(
                    seg,
                    scores,
                    query_offset,
                    node1_offset,
                    node2_offset,
                    node3_offset,
                    node4_offset,
                    dims,
                ),
        }
    }
}

impl RandomVectorScorer for AbstractBulkScorer {
    fn score(&mut self, node: i32) -> Result<f32> {
        check_ordinal(node, self.max_ord)?;
        let query_addr = i64::from(self.query_ord) * self.vector_byte_size;
        let addr = i64::from(node) * self.vector_byte_size;
        let raw = self.vector_op(query_addr, addr);
        Ok(normalize_raw_score(self.similarity, raw))
    }

    fn bulk_score(&mut self, nodes: &[i32], scores: &mut [f32], num_nodes: i32) -> Result<f32> {
        let num_nodes = num_nodes as usize;
        let mut i = 0usize;
        let query_addr = i64::from(self.query_ord) * self.vector_byte_size;
        let mut max_score = f32::NEG_INFINITY;
        let limit = num_nodes & !3;
        while i < limit {
            let offset1 = i64::from(nodes[i]) * self.vector_byte_size;
            let offset2 = i64::from(nodes[i + 1]) * self.vector_byte_size;
            let offset3 = i64::from(nodes[i + 2]) * self.vector_byte_size;
            let offset4 = i64::from(nodes[i + 3]) * self.vector_byte_size;
            Self::bulk_vector_op(
                self.similarity,
                &self.seg,
                &mut self.scratch_scores,
                query_addr,
                offset1,
                offset2,
                offset3,
                offset4,
                self.dims,
            );
            for k in 0..4 {
                scores[i + k] = normalize_raw_score(self.similarity, self.scratch_scores[k]);
                max_score = java_math_max(max_score, scores[i + k]);
            }
            i += 4;
        }
        // Handle the remaining one to three nodes in bulk, if any. Lucene
        // repeats `addr1` in the fourth slot here, where the non-supplier
        // scorer repeats `addr3`; the repeated slot is discarded either way.
        let remaining = num_nodes - i;
        if remaining > 0 {
            let addr1 = i64::from(nodes[i]) * self.vector_byte_size;
            let addr2 = if remaining > 1 {
                i64::from(nodes[i + 1]) * self.vector_byte_size
            } else {
                addr1
            };
            let addr3 = if remaining > 2 {
                i64::from(nodes[i + 2]) * self.vector_byte_size
            } else {
                addr1
            };
            Self::bulk_vector_op(
                self.similarity,
                &self.seg,
                &mut self.scratch_scores,
                query_addr,
                addr1,
                addr2,
                addr3,
                addr1,
                self.dims,
            );
            for k in 0..remaining {
                scores[i + k] = normalize_raw_score(self.similarity, self.scratch_scores[k]);
                max_score = java_math_max(max_score, scores[i + k]);
            }
        }
        Ok(max_score)
    }

    fn max_ord(&self) -> i32 {
        self.max_ord
    }

    fn ord_to_doc(&self, ord: i32) -> i32 {
        self.values.ord_to_doc(ord)
    }
}

impl UpdateableRandomVectorScorer for AbstractBulkScorer {
    fn set_scoring_ordinal(&mut self, node: i32) -> Result<()> {
        check_ordinal(node, self.max_ord)?;
        self.query_ord = node;
        Ok(())
    }
}
