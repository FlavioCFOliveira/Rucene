//! Port of
//! `org.apache.lucene.internal.vectorization.Lucene99MemorySegmentByteVectorScorerSupplier`.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::error::Result;
use crate::index::vector_values::ByteVectorValues;
use crate::index::VectorSimilarityFunction;
use crate::internal::vectorization::lucene99_memory_segment_byte_vector_scorer::{
    check_invariants, check_ordinal, load_vector, VectorBytes,
};
use crate::internal::vectorization::PanamaVectorUtilSupport;
use crate::store::{IndexInput, MemorySegmentAccessInput};
use crate::util::hnsw::{
    RandomVectorScorer, RandomVectorScorerSupplier, UpdateableRandomVectorScorer,
};
use crate::util::Bits;

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
        Self::check_invariants(values.size(), vector_byte_size, input)?;
        Ok(Self {
            similarity,
            vector_byte_size,
            max_ord: values.size(),
            dimension: values.dimension(),
            input: input.clone_access_input()?,
            values,
        })
    }

    /// Fails when `input` is shorter than the vector data it is supposed to
    /// hold.
    ///
    /// Lucene repeats this body verbatim in each of its four memory-segment
    /// scorers and suppliers. This port keeps a single copy of it, the
    /// crate-private `check_invariants` of the byte scorer's module, and each
    /// type's own member calls it, so the four members stay in step by
    /// construction.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneError::IllegalArgument`], with the message
    /// Lucene's `IllegalArgumentException` carries, when the input is too
    /// short.
    ///
    /// Equivalent to
    /// `org.apache.lucene.internal.vectorization.Lucene99MemorySegmentByteVectorScorerSupplier#checkInvariants`.
    pub fn check_invariants(
        max_ord: i32,
        vector_byte_length: usize,
        input: &dyn IndexInput,
    ) -> Result<()> {
        check_invariants(max_ord, vector_byte_length, input)
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

impl MemorySegmentByteScorer {
    /// Fails when `ord` is outside `[0, maxOrd)`.
    ///
    /// Equivalent to
    /// `org.apache.lucene.internal.vectorization.Lucene99MemorySegmentByteVectorScorerSupplier#checkOrdinal`.
    ///
    /// Java declares it `final` on the supplier and calls it from the anonymous
    /// scorer through the enclosing instance. This port's scorer does not hold
    /// its supplier — it carries `maxOrd` itself — so the member lives on the
    /// scorer, which is where Lucene's call sites are.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneError::IllegalArgument`] with Lucene's message.
    fn check_ordinal(&self, ord: i32) -> Result<()> {
        check_ordinal(ord, self.max_ord)
    }

    /// Reads the vector at `ord`, in place when it lies inside one mapped
    /// segment and through the first scratch buffer otherwise.
    ///
    /// Equivalent to
    /// `org.apache.lucene.internal.vectorization.Lucene99MemorySegmentByteVectorScorerSupplier#getFirstSegment`,
    /// which lives on the scorer here for the reason given on
    /// [`check_ordinal`](Self::check_ordinal); `scratch1` moved with it.
    ///
    /// # Errors
    ///
    /// Returns any error raised while reading the vector.
    fn get_first_segment(&mut self, ord: i32) -> Result<VectorBytes> {
        let byte_offset = i64::from(ord) * self.vector_byte_size as i64;
        load_vector(
            self.input.as_mut(),
            &mut self.scratch1,
            byte_offset,
            self.vector_byte_size,
        )
    }

    /// Reads the vector at `ord` through the *second* scratch buffer when it
    /// straddles a segment boundary.
    ///
    /// Equivalent to
    /// `org.apache.lucene.internal.vectorization.Lucene99MemorySegmentByteVectorScorerSupplier#getSecondSegment`.
    /// It differs from [`get_first_segment`](Self::get_first_segment) only in
    /// which buffer it falls back to; both exist because the query vector and
    /// the scored vector can straddle a boundary at the same time.
    ///
    /// # Errors
    ///
    /// Returns any error raised while reading the vector.
    fn get_second_segment(&mut self, ord: i32) -> Result<VectorBytes> {
        let byte_offset = i64::from(ord) * self.vector_byte_size as i64;
        load_vector(
            self.input.as_mut(),
            &mut self.scratch2,
            byte_offset,
            self.vector_byte_size,
        )
    }
}

impl RandomVectorScorer for MemorySegmentByteScorer {
    fn score(&mut self, node: i32) -> Result<f32> {
        self.check_ordinal(node)?;
        let first = self.get_first_segment(self.query_ord)?;
        let second = self.get_second_segment(node)?;
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

    /// Equivalent to the `getAcceptOrds` this scorer inherits from Java's
    /// `AbstractRandomVectorScorer` / `AbstractUpdateableRandomVectorScorer`,
    /// `values.getAcceptOrds(acceptDocs)`.
    fn get_accept_ords<'a>(&self, accept_docs: Option<&'a dyn Bits>) -> Option<Box<dyn Bits + 'a>> {
        self.values.get_accept_ords(accept_docs)
    }
}

impl UpdateableRandomVectorScorer for MemorySegmentByteScorer {
    fn set_scoring_ordinal(&mut self, node: i32) -> Result<()> {
        self.check_ordinal(node)?;
        self.query_ord = node;
        Ok(())
    }
}
