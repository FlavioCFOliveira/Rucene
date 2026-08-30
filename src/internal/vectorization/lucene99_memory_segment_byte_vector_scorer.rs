//! Port of `org.apache.lucene.internal.vectorization.Lucene99MemorySegmentByteVectorScorer`.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::vector_values::ByteVectorValues;
use crate::index::VectorSimilarityFunction;
use crate::internal::vectorization::PanamaVectorUtilSupport;
use crate::store::{IndexInput, MemorySegment, MemorySegmentAccessInput};
use crate::util::hnsw::RandomVectorScorer;

/// Where one vector's bytes were found.
///
/// Java writes both cases as a `MemorySegment`: the mapped slice when the
/// vector lies inside a single segment, and `MemorySegment.ofArray(scratch)`
/// when it straddles two and had to be copied. This port's
/// [`MemorySegment`] is always backed by a mapping and cannot wrap a heap
/// buffer, so the two cases are an enum and the caller resolves them to a
/// `&[u8]`.
pub(crate) enum VectorBytes {
    /// The vector lies inside one mapped segment and is read in place.
    Mapped(MemorySegment),
    /// The vector straddled a segment boundary and was copied into a scratch
    /// buffer.
    Scratch,
}

impl VectorBytes {
    /// Resolves this location to the bytes of the vector.
    pub(crate) fn resolve<'a>(&'a self, scratch: &'a [u8]) -> &'a [u8] {
        match self {
            Self::Mapped(segment) => segment.bytes(),
            Self::Scratch => scratch,
        }
    }
}

/// Reads the vector at `byte_offset` either in place or into `scratch`.
///
/// Equivalent to the `getSegment` / `getFirstSegment` / `getSecondSegment`
/// helpers Lucene repeats in the byte scorer and its supplier.
pub(crate) fn load_vector(
    input: &mut dyn MemorySegmentAccessInput,
    scratch: &mut Vec<u8>,
    byte_offset: i64,
    vector_byte_size: usize,
) -> Result<VectorBytes> {
    if let Some(segment) = input.segment_slice_or_null(byte_offset, vector_byte_size as i64)? {
        return Ok(VectorBytes::Mapped(segment));
    }
    if scratch.len() != vector_byte_size {
        scratch.resize(vector_byte_size, 0);
    }
    input.read_bytes_at(byte_offset, scratch, 0, vector_byte_size)?;
    Ok(VectorBytes::Scratch)
}

/// Fails with Lucene's message when `ord` is outside `[0, max_ord)`.
///
/// Equivalent to the `checkOrdinal` helper Lucene repeats in every
/// memory-segment scorer.
pub(crate) fn check_ordinal(ord: i32, max_ord: i32) -> Result<()> {
    if ord < 0 || ord >= max_ord {
        return Err(LuceneError::IllegalArgument(format!(
            "illegal ordinal: {ord}"
        )));
    }
    Ok(())
}

/// Fails with Lucene's message when the input is too short for the vector data.
///
/// Equivalent to the `checkInvariants(int, int, IndexInput)` helper Lucene
/// repeats in every memory-segment scorer.
pub(crate) fn check_invariants(
    max_ord: i32,
    vector_byte_length: usize,
    input: &dyn IndexInput,
) -> Result<()> {
    if input.length() < (vector_byte_length as i64) * i64::from(max_ord) {
        return Err(LuceneError::IllegalArgument(
            "input length is less than expected vector data".to_string(),
        ));
    }
    Ok(())
}

/// Scores a byte query vector against vectors read from a mapped segment.
///
/// Equivalent to `org.apache.lucene.internal.vectorization.Lucene99MemorySegmentByteVectorScorer`.
///
/// The point of this scorer is that the document vector is never copied to the
/// heap: it is read in place from the memory-mapped file, and only the rare
/// vector that straddles a segment boundary goes through a scratch buffer.
///
/// # Divergences from Lucene 10.5.0
///
/// * **One type instead of a sealed hierarchy.** Java declares an
///   `abstract sealed class` with a `CosineScorer`, `DotProductScorer`,
///   `EuclideanScorer` and `MaxInnerProductScorer` subclass, and `create`
///   selects one with a `switch` on the similarity. Rust has no subclassing;
///   the natural counterpart of that dispatch is the
///   [`VectorSimilarityFunction`] discriminant itself, matched inside
///   [`score`](RandomVectorScorer::score). The score formulas are transcribed
///   unchanged.
/// * **The caller narrows the input.** Java's `create` takes an `IndexInput`,
///   calls `FilterIndexInput.unwrapOnlyTest`, tests
///   `instanceof MemorySegmentAccessInput` and returns `Optional.empty()` when
///   the test fails. Rust cannot test one trait object for another trait, and
///   `FilterIndexInput` is not ported yet, so
///   [`create`](Self::create) takes the narrowed
///   [`MemorySegmentAccessInput`] and returns [`Result`] rather than an
///   optional.
/// * **The kernels are scalar.** They go through
///   [`PanamaVectorUtilSupport`], whose divergence note explains why.
pub struct Lucene99MemorySegmentByteVectorScorer {
    similarity: VectorSimilarityFunction,
    vector_byte_size: usize,
    input: Box<dyn MemorySegmentAccessInput>,
    values: Arc<dyn ByteVectorValues>,
    max_ord: i32,
    query: Vec<u8>,
    scratch: Vec<u8>,
}

impl std::fmt::Debug for Lucene99MemorySegmentByteVectorScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lucene99MemorySegmentByteVectorScorer")
            .field("similarity", &self.similarity)
            .field("vectorByteSize", &self.vector_byte_size)
            .field("maxOrd", &self.max_ord)
            .finish()
    }
}

impl Lucene99MemorySegmentByteVectorScorer {
    /// Creates the scorer for `query`.
    ///
    /// Equivalent to
    /// `Lucene99MemorySegmentByteVectorScorer.create(VectorSimilarityFunction, IndexInput, KnnVectorValues, byte[])`,
    /// minus the `instanceof` narrowing described on the type.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the input is shorter than
    /// the vector data it is supposed to hold, matching Lucene's
    /// `checkInvariants`, and propagates any error raised while cloning the
    /// input.
    pub fn create(
        similarity: VectorSimilarityFunction,
        input: &dyn MemorySegmentAccessInput,
        values: Arc<dyn ByteVectorValues>,
        query_vector: &[u8],
    ) -> Result<Self> {
        let vector_byte_size = values.vector_byte_length() as usize;
        check_invariants(values.size(), vector_byte_size, input)?;
        Ok(Self {
            similarity,
            vector_byte_size,
            input: input.clone_access_input()?,
            max_ord: values.size(),
            values,
            query: query_vector.to_vec(),
            scratch: Vec::new(),
        })
    }

    /// Reads the vector at `ord`, in place when possible.
    ///
    /// Equivalent to `Lucene99MemorySegmentByteVectorScorer.getSegment(int)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] for an ordinal outside
    /// `[0, maxOrd)`, or an I/O error raised while reading the vector.
    fn get_segment(&mut self, ord: i32) -> Result<VectorBytes> {
        check_ordinal(ord, self.max_ord)?;
        let byte_offset = i64::from(ord) * self.vector_byte_size as i64;
        load_vector(
            self.input.as_mut(),
            &mut self.scratch,
            byte_offset,
            self.vector_byte_size,
        )
    }
}

impl RandomVectorScorer for Lucene99MemorySegmentByteVectorScorer {
    fn score(&mut self, node: i32) -> Result<f32> {
        check_ordinal(node, self.max_ord)?;
        let located = self.get_segment(node)?;
        let doc = located.resolve(&self.scratch);
        match self.similarity {
            VectorSimilarityFunction::COSINE => {
                let raw = PanamaVectorUtilSupport::cosine(&self.query, doc)?;
                Ok((1.0 + raw) / 2.0)
            }
            VectorSimilarityFunction::DOT_PRODUCT => {
                // divide by 2 * 2^14 (maximum absolute value of the product of
                // two signed bytes) * len
                let raw = PanamaVectorUtilSupport::dot_product(&self.query, doc)? as f32;
                let denominator = (self.query.len() as i32).wrapping_mul(1 << 15) as f32;
                Ok(0.5 + raw / denominator)
            }
            VectorSimilarityFunction::EUCLIDEAN => {
                let raw = PanamaVectorUtilSupport::square_distance(&self.query, doc)? as f32;
                Ok(1.0 / (1.0 + raw))
            }
            VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT => {
                let raw = PanamaVectorUtilSupport::dot_product(&self.query, doc)? as f32;
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
