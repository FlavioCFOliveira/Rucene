//! Port of `org.apache.lucene.internal.vectorization.Lucene99MemorySegmentFloatVectorScorer`.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::error::Result;
use crate::index::vector_values::FloatVectorValues;
use crate::index::VectorSimilarityFunction;
use crate::internal::vectorization::lucene99_memory_segment_byte_vector_scorer::{
    check_invariants, check_ordinal,
};
use crate::internal::vectorization::MemorySegmentBulkVectorOps;
use crate::store::{MemorySegment, MemorySegmentAccessInput};
use crate::util::hnsw::RandomVectorScorer;
use crate::util::vector_util;

/// `Math.max(float, float)` as specified by the JLS.
///
/// `f32::max` returns the non-NaN operand when exactly one operand is NaN,
/// whereas Java propagates NaN. The distinction is observable here: the cosine
/// of a zero vector is NaN, and Lucene's `bulkScore` therefore returns a NaN
/// maximum once any neighbour scores NaN, where `f32::max` would hide it.
#[inline]
pub(crate) fn java_math_max(a: f32, b: f32) -> f32 {
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

/// Maps a raw similarity onto the score Lucene reports.
///
/// Equivalent to the `normalizeRawScore(float)` method each subclass of
/// `Lucene99MemorySegmentFloatVectorScorer` overrides.
pub(crate) fn normalize_raw_score(similarity: VectorSimilarityFunction, raw_score: f32) -> f32 {
    match similarity {
        VectorSimilarityFunction::COSINE | VectorSimilarityFunction::DOT_PRODUCT => {
            vector_util::normalize_to_unit_interval(raw_score)
        }
        VectorSimilarityFunction::EUCLIDEAN => {
            vector_util::normalize_distance_to_unit_interval(raw_score)
        }
        VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT => {
            vector_util::scale_max_inner_product_score(raw_score)
        }
    }
}

/// Fills `scores[0..4]` with the raw similarities of four document vectors
/// against an on-heap query.
///
/// Equivalent to the `vectorOp(MemorySegment, float[], long, long, long, long, int)`
/// method each subclass overrides, which picks the matching
/// [`MemorySegmentBulkVectorOps`] instance.
#[allow(clippy::too_many_arguments)]
pub(crate) fn bulk_vector_op(
    similarity: VectorSimilarityFunction,
    seg: &MemorySegment,
    scores: &mut [f32],
    query: &[f32],
    node1_offset: i64,
    node2_offset: i64,
    node3_offset: i64,
    node4_offset: i64,
    element_count: usize,
) {
    match similarity {
        VectorSimilarityFunction::COSINE => MemorySegmentBulkVectorOps::COS_INSTANCE.cosine_bulk(
            seg,
            scores,
            query,
            node1_offset,
            node2_offset,
            node3_offset,
            node4_offset,
            element_count,
        ),
        VectorSimilarityFunction::DOT_PRODUCT | VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT => {
            MemorySegmentBulkVectorOps::DOT_INSTANCE.dot_product_bulk(
                seg,
                scores,
                query,
                node1_offset,
                node2_offset,
                node3_offset,
                node4_offset,
                element_count,
            )
        }
        VectorSimilarityFunction::EUCLIDEAN => MemorySegmentBulkVectorOps::SQR_INSTANCE
            .sqr_distance_bulk(
                seg,
                scores,
                query,
                node1_offset,
                node2_offset,
                node3_offset,
                node4_offset,
                element_count,
            ),
    }
}

/// Scores a float query vector against vectors read from a mapped segment.
///
/// Equivalent to
/// `org.apache.lucene.internal.vectorization.Lucene99MemorySegmentFloatVectorScorer`.
///
/// The single-node [`score`](RandomVectorScorer::score) delegates to the
/// ordinary on-heap comparison, exactly as Lucene does; it is
/// [`bulk_score`](RandomVectorScorer::bulk_score) that pays off, scoring four
/// neighbours per pass straight out of the mapped file.
///
/// # Divergences from Lucene 10.5.0
///
/// * **One type instead of a sealed hierarchy.** Java's four subclasses differ
///   only in which bulk operation and which score normalisation they use, so
///   the port matches on [`VectorSimilarityFunction`] instead, in the
///   crate-private `bulk_vector_op` and `normalize_raw_score` helpers of this
///   module.
/// * **The caller narrows the input.** Java's `create` takes an `IndexInput`,
///   unwraps test filters and tests `instanceof MemorySegmentAccessInput`. Rust
///   cannot test a trait object for another trait, so [`create`](Self::create)
///   takes the narrowed [`MemorySegmentAccessInput`]. It still returns `None`
///   for the other reason Lucene does: the file does not fit in one segment.
/// * **The kernels are scalar**, as explained on
///   [`MemorySegmentBulkVectorOps`].
pub struct Lucene99MemorySegmentFloatVectorScorer {
    similarity: VectorSimilarityFunction,
    values: Arc<dyn FloatVectorValues>,
    vector_byte_size: i64,
    max_ord: i32,
    seg: MemorySegment,
    query: Vec<f32>,
    scratch_scores: [f32; 4],
}

impl std::fmt::Debug for Lucene99MemorySegmentFloatVectorScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lucene99MemorySegmentFloatVectorScorer")
            .field("similarity", &self.similarity)
            .field("vectorByteSize", &self.vector_byte_size)
            .field("maxOrd", &self.max_ord)
            .finish()
    }
}

impl Lucene99MemorySegmentFloatVectorScorer {
    /// Creates the scorer, or returns `None` when the vector data does not fit
    /// in a single mapped segment.
    ///
    /// Equivalent to
    /// `Lucene99MemorySegmentFloatVectorScorer.create(VectorSimilarityFunction, IndexInput, FloatVectorValues, float[])`,
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
        query: &[f32],
    ) -> Result<Option<Self>> {
        let Some(seg) = input.segment_slice_or_null(0, input.length())? else {
            return Ok(None);
        };
        check_invariants(values.size(), values.vector_byte_length() as usize, input)?;
        Ok(Some(Self {
            similarity,
            vector_byte_size: i64::from(values.vector_byte_length()),
            max_ord: values.size(),
            values,
            seg,
            query: query.to_vec(),
            scratch_scores: [0.0; 4],
        }))
    }
}

impl RandomVectorScorer for Lucene99MemorySegmentFloatVectorScorer {
    fn score(&mut self, node: i32) -> Result<f32> {
        check_ordinal(node, self.max_ord)?;
        // just delegates to the existing scorer that copies on-heap
        let vector = self.values.vector_value(node)?;
        self.similarity.compare_f32(&self.query, &vector)
    }

    fn bulk_score(&mut self, nodes: &[i32], scores: &mut [f32], num_nodes: i32) -> Result<f32> {
        let num_nodes = num_nodes as usize;
        let mut i = 0usize;
        let limit = num_nodes & !3;
        let mut max_score = f32::NEG_INFINITY;
        let element_count = self.query.len();
        while i < limit {
            let offset1 = i64::from(nodes[i]) * self.vector_byte_size;
            let offset2 = i64::from(nodes[i + 1]) * self.vector_byte_size;
            let offset3 = i64::from(nodes[i + 2]) * self.vector_byte_size;
            let offset4 = i64::from(nodes[i + 3]) * self.vector_byte_size;
            bulk_vector_op(
                self.similarity,
                &self.seg,
                &mut self.scratch_scores,
                &self.query,
                offset1,
                offset2,
                offset3,
                offset4,
                element_count,
            );
            for k in 0..4 {
                scores[i + k] = normalize_raw_score(self.similarity, self.scratch_scores[k]);
                max_score = java_math_max(max_score, scores[i + k]);
            }
            i += 4;
        }
        // Handle the remaining one to three nodes in bulk, if any.
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
            bulk_vector_op(
                self.similarity,
                &self.seg,
                &mut self.scratch_scores,
                &self.query,
                addr1,
                addr2,
                addr3,
                addr3,
                element_count,
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
