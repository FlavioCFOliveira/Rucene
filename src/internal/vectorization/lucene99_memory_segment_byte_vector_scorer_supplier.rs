//! Port of
//! `org.apache.lucene.internal.vectorization.Lucene99MemorySegmentByteVectorScorerSupplier`.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::error::Result;
use crate::index::vector_values::ByteVectorValues;
use crate::index::VectorSimilarityFunction;
use crate::internal::vectorization::lucene99_memory_segment_byte_vector_scorer::{
    check_invariants, check_ordinal, load_vector,
};
use crate::internal::vectorization::PanamaVectorUtilSupport;
use crate::store::MemorySegmentAccessInput;
use crate::util::hnsw::{
    RandomVectorScorer, RandomVectorScorerSupplier, UpdateableRandomVectorScorer,
};

/// A score supplier of vectors whose element size is byte.
///
/// Equivalent to
/// `org.apache.lucene.internal.vectorization.Lucene99MemorySegmentByteVectorScorerSupplier`.
///
/// Both the query vector and the scored vector are read in place from the
/// mapped file, so a whole HNSW neighbour block can be scored without copying a
/// single vector to the heap.
///
/// # Divergences from Lucene 10.5.0
///
/// * **One type instead of a sealed hierarchy**, as in
///   [`Lucene99MemorySegmentByteVectorScorer`](super::Lucene99MemorySegmentByteVectorScorer):
///   Java's four suppliers become a match on [`VectorSimilarityFunction`].
/// * **The caller narrows the input**, for the same reason as the scorer:
///   Rust cannot test a `dyn IndexInput` for [`MemorySegmentAccessInput`].
/// * **The scratch buffers belong to the scorer, not the supplier.** Java keeps
///   `scratch1` and `scratch2` on the supplier and lets the anonymous scorer
///   mutate them through the enclosing instance. `scorer()` takes `&self` here,
///   so each scorer owns its two buffers and its own clone of the input, which
///   is also what makes the scorers usable from different threads.
pub struct Lucene99MemorySegmentByteVectorScorerSupplier {
    similarity: VectorSimilarityFunction,
    vector_byte_size: usize,
    max_ord: i32,
    dimension: i32,
    input: Box<dyn MemorySegmentAccessInput>,
    values: Arc<dyn ByteVectorValues>,
}

impl std::fmt::Debug for Lucene99MemorySegmentByteVectorScorerSupplier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lucene99MemorySegmentByteVectorScorerSupplier")
            .field("similarity", &self.similarity)
            .field("vectorByteSize", &self.vector_byte_size)
            .field("maxOrd", &self.max_ord)
            .finish()
    }
}

impl Lucene99MemorySegmentByteVectorScorerSupplier {
    /// Creates the supplier.
    ///
    /// Equivalent to
    /// `Lucene99MemorySegmentByteVectorScorerSupplier.create(VectorSimilarityFunction, IndexInput, KnnVectorValues)`,
    /// minus the `instanceof` narrowing described on the type.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneError::IllegalArgument`] when the input is
    /// shorter than the vector data it is supposed to hold, and propagates any
    /// error raised while cloning the input.
    pub fn create(
        similarity: VectorSimilarityFunction,
        input: &dyn MemorySegmentAccessInput,
        values: Arc<dyn ByteVectorValues>,
    ) -> Result<Self> {
        let vector_byte_size = values.vector_byte_length() as usize;
        check_invariants(values.size(), vector_byte_size, input)?;
        Ok(Self {
            similarity,
            vector_byte_size,
            max_ord: values.size(),
            dimension: values.dimension(),
            input: input.clone_access_input()?,
            values,
        })
    }
}

impl RandomVectorScorerSupplier for Lucene99MemorySegmentByteVectorScorerSupplier {
    fn scorer(&self) -> Result<Box<dyn UpdateableRandomVectorScorer>> {
        Ok(Box::new(MemorySegmentByteScorer {
            similarity: self.similarity,
            vector_byte_size: self.vector_byte_size,
            max_ord: self.max_ord,
            dimension: self.dimension,
            input: self.input.clone_access_input()?,
            values: Arc::clone(&self.values),
            query_ord: 0,
            scratch1: Vec::new(),
            scratch2: Vec::new(),
        }))
    }

    fn copy(&self) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        Ok(Box::new(Self {
            similarity: self.similarity,
            vector_byte_size: self.vector_byte_size,
            max_ord: self.max_ord,
            dimension: self.dimension,
            input: self.input.clone_access_input()?,
            values: Arc::clone(&self.values),
        }))
    }
}

/// The scorer [`Lucene99MemorySegmentByteVectorScorerSupplier::scorer`] hands
/// out.
///
/// Equivalent to the anonymous
/// `UpdateableRandomVectorScorer.AbstractUpdateableRandomVectorScorer`
/// subclasses Lucene creates inside each supplier's `scorer()`.
struct MemorySegmentByteScorer {
    similarity: VectorSimilarityFunction,
    vector_byte_size: usize,
    max_ord: i32,
    dimension: i32,
    input: Box<dyn MemorySegmentAccessInput>,
    values: Arc<dyn ByteVectorValues>,
    query_ord: i32,
    scratch1: Vec<u8>,
    scratch2: Vec<u8>,
}

impl RandomVectorScorer for MemorySegmentByteScorer {
    fn score(&mut self, node: i32) -> Result<f32> {
        check_ordinal(node, self.max_ord)?;
        let query_offset = i64::from(self.query_ord) * self.vector_byte_size as i64;
        let node_offset = i64::from(node) * self.vector_byte_size as i64;
        // Java reads the two vectors through `getFirstSegment` and
        // `getSecondSegment`, which differ only in which scratch buffer they
        // fall back to; both are needed because the two vectors can straddle a
        // boundary at the same time.
        let first = load_vector(
            self.input.as_mut(),
            &mut self.scratch1,
            query_offset,
            self.vector_byte_size,
        )?;
        let second = load_vector(
            self.input.as_mut(),
            &mut self.scratch2,
            node_offset,
            self.vector_byte_size,
        )?;
        let query = first.resolve(&self.scratch1);
        let doc = second.resolve(&self.scratch2);
        match self.similarity {
            VectorSimilarityFunction::COSINE => {
                let raw = PanamaVectorUtilSupport::cosine(query, doc)?;
                Ok((1.0 + raw) / 2.0)
            }
            VectorSimilarityFunction::DOT_PRODUCT => {
                // divide by 2 * 2^14 (maximum absolute value of the product of
                // two signed bytes) * len
                let raw = PanamaVectorUtilSupport::dot_product(query, doc)? as f32;
                let denominator = self.dimension.wrapping_mul(1 << 15) as f32;
                Ok(0.5 + raw / denominator)
            }
            VectorSimilarityFunction::EUCLIDEAN => {
                let raw = PanamaVectorUtilSupport::square_distance(query, doc)? as f32;
                Ok(1.0 / (1.0 + raw))
            }
            VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT => {
                let raw = PanamaVectorUtilSupport::dot_product(query, doc)? as f32;
                if raw < 0.0 {
                    Ok(1.0 / (1.0 + -raw))
                } else {
                    Ok(raw + 1.0)
                }
            }
        }
    }

    fn max_ord(&self) -> i32 {
        self.max_ord
    }

    fn ord_to_doc(&self, ord: i32) -> i32 {
        self.values.ord_to_doc(ord)
    }
}

impl UpdateableRandomVectorScorer for MemorySegmentByteScorer {
    fn set_scoring_ordinal(&mut self, node: i32) -> Result<()> {
        check_ordinal(node, self.max_ord)?;
        self.query_ord = node;
        Ok(())
    }
}
