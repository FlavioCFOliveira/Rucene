//! Port of `org.apache.lucene.internal.vectorization.MemorySegmentBulkVectorOps`.

#![deny(unsafe_code)]

use crate::store::MemorySegment;
use crate::util::vector_util;

/// Implementations of bulk vector comparison operations. Currently only
/// supports float32.
///
/// Equivalent to `org.apache.lucene.internal.vectorization.MemorySegmentBulkVectorOps`.
///
/// The three operations score one query against **four** document vectors in a
/// single pass over the mapped bytes, which is what lets the HNSW search score
/// a whole neighbour block without copying any vector to the heap.
///
/// # Divergence from Lucene 10.5.0: the lane loop is absent
///
/// Every method in the Java class is a `FloatVector` loop over
/// `FLOAT_SPECIES.loopBound(elementCount)` elements followed by a scalar loop
/// that finishes the remainder. Stable Rust has no portable SIMD (`std::simd`
/// is nightly-only; this crate's MSRV is 1.80), so
/// [`PanamaVectorConstants::PREFERRED_VECTOR_BITSIZE`](super::PanamaVectorConstants::PREFERRED_VECTOR_BITSIZE)
/// is zero, the loop bound is zero, and **Lucene's own scalar remainder
/// computes the whole result**. Each method below is that remainder,
/// transcribed operation for operation — including which accumulator each
/// product is added to and the `f64` square root at the end of the cosine —
/// because floating-point addition is not associative and the accumulation
/// order is observable in the result.
///
/// A JVM that does have the Vector API reduces its lanes in a different order
/// and can therefore return a slightly different low bit; that is the same
/// split Lucene already has between its vectorized and scalar paths, and this
/// port matches the scalar one, as [`crate::util::vector_util`] does.
#[derive(Debug, Clone, Copy)]
pub struct MemorySegmentBulkVectorOps;

impl MemorySegmentBulkVectorOps {
    /// The shared dot-product operations.
    ///
    /// Equivalent to `MemorySegmentBulkVectorOps.DOT_INSTANCE`.
    pub const DOT_INSTANCE: DotProduct = DotProduct;

    /// The shared cosine operations.
    ///
    /// Equivalent to `MemorySegmentBulkVectorOps.COS_INSTANCE`.
    pub const COS_INSTANCE: Cosine = Cosine;

    /// The shared square-distance operations.
    ///
    /// Equivalent to `MemorySegmentBulkVectorOps.SQR_INSTANCE`.
    pub const SQR_INSTANCE: SqrDistance = SqrDistance;
}

/// `a * b + c`, or `fma(a, b, c)` when [`vector_util::USE_FMA`] is enabled.
///
/// Equivalent to `PanamaVectorUtilSupport.fma(float, float, float)`, which
/// Lucene's bulk ops import statically. The crate pins the non-fused form so
/// that scores are reproducible on every machine; see
/// [`crate::util::vector_util`].
#[inline]
fn fma(a: f32, b: f32, c: f32) -> f32 {
    if vector_util::USE_FMA {
        a.mul_add(b, c)
    } else {
        a * b + c
    }
}

/// Reads the little-endian `f32` stored at byte offset `offset`.
///
/// Equivalent to `MemorySegment.get(ValueLayout.JAVA_FLOAT_UNALIGNED.withOrder(LITTLE_ENDIAN), offset)`.
///
/// # Panics
///
/// Panics when the four bytes are not contained in `data`, standing in for
/// Java's `IndexOutOfBoundsException`.
#[inline]
fn read_f32(data: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

/// Byte offset of element `i` of a float vector starting at byte offset `base`.
#[inline]
fn elem(base: i64, i: usize) -> usize {
    (base as usize) + i * std::mem::size_of::<f32>()
}

/// Dot-product bulk operations.
///
/// Equivalent to the nested `MemorySegmentBulkVectorOps.DotProduct`.
#[derive(Debug, Clone, Copy)]
pub struct DotProduct;

impl DotProduct {
    /// Scores an on-heap query against four document vectors in the segment.
    ///
    /// Equivalent to
    /// `DotProduct.dotProductBulk(MemorySegment, float[], float[], long, long, long, long, int)`.
    /// The four raw dot products are written to `scores[0..4]`.
    ///
    /// # Panics
    ///
    /// Panics when `q` is shorter than `element_count` or a document vector
    /// runs past the end of the segment, standing in for Java's
    /// index-out-of-bounds exceptions.
    #[allow(clippy::too_many_arguments)]
    pub fn dot_product_bulk(
        &self,
        data_seg: &MemorySegment,
        scores: &mut [f32],
        q: &[f32],
        d1: i64,
        d2: i64,
        d3: i64,
        d4: i64,
        element_count: usize,
    ) {
        let data = data_seg.bytes();
        let (mut sum1, mut sum2, mut sum3, mut sum4) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        // Slicing to `element_count` first keeps Lucene's behaviour of failing
        // when the query is shorter than the vectors it is scored against.
        for (i, &q_value) in q[..element_count].iter().enumerate() {
            sum1 = fma(q_value, read_f32(data, elem(d1, i)), sum1);
            sum2 = fma(q_value, read_f32(data, elem(d2, i)), sum2);
            sum3 = fma(q_value, read_f32(data, elem(d3, i)), sum3);
            sum4 = fma(q_value, read_f32(data, elem(d4, i)), sum4);
        }
        scores[0] = sum1;
        scores[1] = sum2;
        scores[2] = sum3;
        scores[3] = sum4;
    }

    /// Scores a query that also lives in the segment against four document
    /// vectors.
    ///
    /// Equivalent to
    /// `DotProduct.dotProductBulk(MemorySegment, float[], long, long, long, long, long, int)`,
    /// the overload whose query is a byte offset rather than a heap array.
    ///
    /// # Panics
    ///
    /// Panics when any vector runs past the end of the segment.
    #[allow(clippy::too_many_arguments)]
    pub fn dot_product_bulk_at(
        &self,
        seg: &MemorySegment,
        scores: &mut [f32],
        q: i64,
        d1: i64,
        d2: i64,
        d3: i64,
        d4: i64,
        element_count: usize,
    ) {
        let data = seg.bytes();
        let (mut sum1, mut sum2, mut sum3, mut sum4) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        for i in 0..element_count {
            let q_value = read_f32(data, elem(q, i));
            sum1 = fma(q_value, read_f32(data, elem(d1, i)), sum1);
            sum2 = fma(q_value, read_f32(data, elem(d2, i)), sum2);
            sum3 = fma(q_value, read_f32(data, elem(d3, i)), sum3);
            sum4 = fma(q_value, read_f32(data, elem(d4, i)), sum4);
        }
        scores[0] = sum1;
        scores[1] = sum2;
        scores[2] = sum3;
        scores[3] = sum4;
    }

    /// Returns the dot product of two vectors that both live in the segment.
    ///
    /// Equivalent to `DotProduct.dotProduct(MemorySegment, long, long, int)`.
    ///
    /// # Panics
    ///
    /// Panics when either vector runs past the end of the segment.
    pub fn dot_product(&self, seg: &MemorySegment, q: i64, d: i64, element_count: usize) -> f32 {
        let data = seg.bytes();
        let mut score = 0.0f32;
        for i in 0..element_count {
            score += read_f32(data, elem(q, i)) * read_f32(data, elem(d, i));
        }
        score
    }
}

/// Cosine bulk operations.
///
/// Equivalent to the nested `MemorySegmentBulkVectorOps.Cosine`.
#[derive(Debug, Clone, Copy)]
pub struct Cosine;

impl Cosine {
    /// Scores an on-heap query against four document vectors in the segment.
    ///
    /// Equivalent to
    /// `Cosine.cosineBulk(MemorySegment, float[], float[], long, long, long, long, int)`.
    ///
    /// # Panics
    ///
    /// Panics when `q` is shorter than `element_count` or a document vector
    /// runs past the end of the segment.
    #[allow(clippy::too_many_arguments)]
    pub fn cosine_bulk(
        &self,
        data_seg: &MemorySegment,
        scores: &mut [f32],
        q: &[f32],
        d1: i64,
        d2: i64,
        d3: i64,
        d4: i64,
        element_count: usize,
    ) {
        let data = data_seg.bytes();
        let (mut sum1, mut sum2, mut sum3, mut sum4) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        let mut q_norm = 0.0f32;
        let (mut d1_norm, mut d2_norm, mut d3_norm, mut d4_norm) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        // Slicing to `element_count` first keeps Lucene's behaviour of failing
        // when the query is shorter than the vectors it is scored against.
        for (i, &q_value) in q[..element_count].iter().enumerate() {
            let d1_value = read_f32(data, elem(d1, i));
            let d2_value = read_f32(data, elem(d2, i));
            let d3_value = read_f32(data, elem(d3, i));
            let d4_value = read_f32(data, elem(d4, i));
            sum1 = fma(q_value, d1_value, sum1);
            sum2 = fma(q_value, d2_value, sum2);
            sum3 = fma(q_value, d3_value, sum3);
            sum4 = fma(q_value, d4_value, sum4);
            q_norm = fma(q_value, q_value, q_norm);
            d1_norm = fma(d1_value, d1_value, d1_norm);
            d2_norm = fma(d2_value, d2_value, d2_norm);
            d3_norm = fma(d3_value, d3_value, d3_norm);
            d4_norm = fma(d4_value, d4_value, d4_norm);
        }
        scores[0] = normalize(sum1, q_norm, d1_norm);
        scores[1] = normalize(sum2, q_norm, d2_norm);
        scores[2] = normalize(sum3, q_norm, d3_norm);
        scores[3] = normalize(sum4, q_norm, d4_norm);
    }

    /// Scores a query that also lives in the segment against four document
    /// vectors.
    ///
    /// Equivalent to
    /// `Cosine.cosineBulk(MemorySegment, float[], long, long, long, long, long, int)`.
    ///
    /// # Panics
    ///
    /// Panics when any vector runs past the end of the segment.
    #[allow(clippy::too_many_arguments)]
    pub fn cosine_bulk_at(
        &self,
        seg: &MemorySegment,
        scores: &mut [f32],
        q: i64,
        d1: i64,
        d2: i64,
        d3: i64,
        d4: i64,
        element_count: usize,
    ) {
        let data = seg.bytes();
        let (mut sum1, mut sum2, mut sum3, mut sum4) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        let mut q_norm = 0.0f32;
        let (mut d1_norm, mut d2_norm, mut d3_norm, mut d4_norm) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        for i in 0..element_count {
            let q_value = read_f32(data, elem(q, i));
            let d1_value = read_f32(data, elem(d1, i));
            let d2_value = read_f32(data, elem(d2, i));
            let d3_value = read_f32(data, elem(d3, i));
            let d4_value = read_f32(data, elem(d4, i));
            sum1 = fma(q_value, d1_value, sum1);
            sum2 = fma(q_value, d2_value, sum2);
            sum3 = fma(q_value, d3_value, sum3);
            sum4 = fma(q_value, d4_value, sum4);
            q_norm = fma(q_value, q_value, q_norm);
            d1_norm = fma(d1_value, d1_value, d1_norm);
            d2_norm = fma(d2_value, d2_value, d2_norm);
            d3_norm = fma(d3_value, d3_value, d3_norm);
            d4_norm = fma(d4_value, d4_value, d4_norm);
        }
        scores[0] = normalize(sum1, q_norm, d1_norm);
        scores[1] = normalize(sum2, q_norm, d2_norm);
        scores[2] = normalize(sum3, q_norm, d3_norm);
        scores[3] = normalize(sum4, q_norm, d4_norm);
    }

    /// Returns the cosine similarity of two vectors that both live in the
    /// segment.
    ///
    /// Equivalent to `Cosine.cosine(MemorySegment, long, long, int)`.
    ///
    /// # Panics
    ///
    /// Panics when either vector runs past the end of the segment.
    pub fn cosine(&self, seg: &MemorySegment, q: i64, d: i64, element_count: usize) -> f32 {
        let data = seg.bytes();
        let mut sum = 0.0f32;
        let mut q_norm = 0.0f32;
        let mut d_norm = 0.0f32;
        for i in 0..element_count {
            let q_value = read_f32(data, elem(q, i));
            let d_value = read_f32(data, elem(d, i));
            sum = fma(q_value, d_value, sum);
            q_norm = fma(q_value, q_value, q_norm);
            d_norm = fma(d_value, d_value, d_norm);
        }
        normalize(sum, q_norm, d_norm)
    }
}

/// `(float) (sum / Math.sqrt((double) qNorm * (double) dNorm))`.
///
/// The division and square root are performed in `f64` with a single rounding
/// back to `f32`, exactly as Lucene does; a zero norm therefore yields `NaN`.
#[inline]
fn normalize(sum: f32, q_norm: f32, d_norm: f32) -> f32 {
    (f64::from(sum) / (f64::from(q_norm) * f64::from(d_norm)).sqrt()) as f32
}

/// Square-distance bulk operations.
///
/// Equivalent to the nested `MemorySegmentBulkVectorOps.SqrDistance`.
#[derive(Debug, Clone, Copy)]
pub struct SqrDistance;

impl SqrDistance {
    /// Scores an on-heap query against four document vectors in the segment.
    ///
    /// Equivalent to
    /// `SqrDistance.sqrDistanceBulk(MemorySegment, float[], float[], long, long, long, long, int)`.
    ///
    /// # Panics
    ///
    /// Panics when `q` is shorter than `element_count` or a document vector
    /// runs past the end of the segment.
    #[allow(clippy::too_many_arguments)]
    pub fn sqr_distance_bulk(
        &self,
        data_seg: &MemorySegment,
        scores: &mut [f32],
        q: &[f32],
        d1: i64,
        d2: i64,
        d3: i64,
        d4: i64,
        element_count: usize,
    ) {
        let data = data_seg.bytes();
        let (mut sum1, mut sum2, mut sum3, mut sum4) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        // Slicing to `element_count` first keeps Lucene's behaviour of failing
        // when the query is shorter than the vectors it is scored against.
        for (i, &q_value) in q[..element_count].iter().enumerate() {
            let diff1 = q_value - read_f32(data, elem(d1, i));
            let diff2 = q_value - read_f32(data, elem(d2, i));
            let diff3 = q_value - read_f32(data, elem(d3, i));
            let diff4 = q_value - read_f32(data, elem(d4, i));
            sum1 = fma(diff1, diff1, sum1);
            sum2 = fma(diff2, diff2, sum2);
            sum3 = fma(diff3, diff3, sum3);
            sum4 = fma(diff4, diff4, sum4);
        }
        scores[0] = sum1;
        scores[1] = sum2;
        scores[2] = sum3;
        scores[3] = sum4;
    }

    /// Scores a query that also lives in the segment against four document
    /// vectors.
    ///
    /// Equivalent to
    /// `SqrDistance.sqrDistanceBulk(MemorySegment, float[], long, long, long, long, long, int)`.
    ///
    /// # Panics
    ///
    /// Panics when any vector runs past the end of the segment.
    #[allow(clippy::too_many_arguments)]
    pub fn sqr_distance_bulk_at(
        &self,
        seg: &MemorySegment,
        scores: &mut [f32],
        q: i64,
        d1: i64,
        d2: i64,
        d3: i64,
        d4: i64,
        element_count: usize,
    ) {
        let data = seg.bytes();
        let (mut sum1, mut sum2, mut sum3, mut sum4) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        for i in 0..element_count {
            let q_value = read_f32(data, elem(q, i));
            let diff1 = q_value - read_f32(data, elem(d1, i));
            let diff2 = q_value - read_f32(data, elem(d2, i));
            let diff3 = q_value - read_f32(data, elem(d3, i));
            let diff4 = q_value - read_f32(data, elem(d4, i));
            sum1 = fma(diff1, diff1, sum1);
            sum2 = fma(diff2, diff2, sum2);
            sum3 = fma(diff3, diff3, sum3);
            sum4 = fma(diff4, diff4, sum4);
        }
        scores[0] = sum1;
        scores[1] = sum2;
        scores[2] = sum3;
        scores[3] = sum4;
    }

    /// Returns the sum of squared differences of two vectors that both live in
    /// the segment.
    ///
    /// Equivalent to `SqrDistance.sqrDistance(MemorySegment, long, long, int)`.
    ///
    /// # Panics
    ///
    /// Panics when either vector runs past the end of the segment.
    pub fn sqr_distance(&self, seg: &MemorySegment, q: i64, d: i64, element_count: usize) -> f32 {
        let data = seg.bytes();
        let mut score = 0.0f32;
        for i in 0..element_count {
            let diff = read_f32(data, elem(q, i)) - read_f32(data, elem(d, i));
            score = fma(diff, diff, score);
        }
        score
    }
}
