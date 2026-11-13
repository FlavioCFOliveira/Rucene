//! Port of `org.apache.lucene.internal.vectorization.MemorySegmentBulkVectorOps`.

#![deny(unsafe_code)]

use crate::internal::vectorization::panama_vector_util_support::{
    fma, fma_vec, loop_bound, with_float_lanes, FloatVector,
};
use crate::internal::vectorization::PanamaVectorConstants;
use crate::store::MemorySegment;

/// Implementations of bulk vector comparison operations. Currently only
/// supports float32.
///
/// Equivalent to `org.apache.lucene.internal.vectorization.MemorySegmentBulkVectorOps`.
///
/// The three operations score one query against **four** document vectors in a
/// single pass over the mapped bytes, which is what lets the HNSW search score
/// a whole neighbour block without copying any vector to the heap.
///
/// Every method is Java's: a `FloatVector` loop over
/// `FLOAT_SPECIES.loopBound(elementCount)` elements — `FLOAT_SPECIES` being
/// the float species of
/// [`PanamaVectorConstants::preferred_vector_bitsize`] — accumulating with the
/// vector `fma` gated on `Constants.HAS_FAST_VECTOR_FMA`, reduced in the order
/// HotSpot's C2 uses, followed by a scalar loop that finishes the remainder
/// with the scalar `fma` gated on `Constants.HAS_FAST_SCALAR_FMA` (and, in
/// `DotProduct.dotProduct`, a plain `+=` of the product, as in Java). The
/// lanes are fixed-width arrays (see
/// [`PanamaVectorUtilSupport`](super::PanamaVectorUtilSupport)), so the
/// results are Java's bit for bit.
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

/// Lane count of `MemorySegmentBulkVectorOps.FLOAT_SPECIES`.
#[inline]
fn float_lanes() -> usize {
    PanamaVectorConstants::int_lanes()
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

/// The query of a bulk operation: Java's `qArray`, or, when Java passes
/// `qOffset != -1`, a vector in the segment.
#[derive(Clone, Copy)]
enum Query<'a> {
    Array(&'a [f32]),
    Segment(i64),
}

impl Query<'_> {
    #[inline]
    fn vector<const N: usize>(self, data: &[u8], i: usize) -> FloatVector<N> {
        match self {
            Query::Array(q) => FloatVector::from_array(q, i),
            Query::Segment(q) => FloatVector::from_le_bytes(data, elem(q, i)),
        }
    }

    #[inline]
    fn value(self, data: &[u8], i: usize) -> f32 {
        match self {
            Query::Array(q) => q[i],
            Query::Segment(q) => read_f32(data, elem(q, i)),
        }
    }

    /// Java fails with an index-out-of-bounds exception when the heap query
    /// is shorter than the vectors it is scored against.
    #[inline]
    fn check(self, element_count: usize) {
        if let Query::Array(q) = self {
            let _ = &q[..element_count];
        }
    }
}

/// `DotProduct.dotProductBulkImpl`.
fn dot_product_bulk_impl<const N: usize>(
    data: &[u8],
    q: Query<'_>,
    d: [i64; 4],
    element_count: usize,
) -> [f32; 4] {
    q.check(element_count);
    let mut sv = [FloatVector::<N>::zero(); 4];
    let limit = loop_bound(element_count, N);
    let mut i = 0;
    while i < limit {
        let qv = q.vector::<N>(data, i);
        for k in 0..4 {
            let dv = FloatVector::<N>::from_le_bytes(data, elem(d[k], i));
            sv[k] = fma_vec(qv, dv, sv[k]);
        }
        i += N;
    }
    let mut sums = sv.map(FloatVector::reduce_add);
    while i < element_count {
        let q_value = q.value(data, i);
        for k in 0..4 {
            sums[k] = fma(q_value, read_f32(data, elem(d[k], i)), sums[k]);
        }
        i += 1;
    }
    sums
}

/// `DotProduct.dotProduct(MemorySegment, long, long, int)`.
fn dot_product_impl<const N: usize>(data: &[u8], q: i64, d: i64, element_count: usize) -> f32 {
    let mut i = 0;
    let mut sv = FloatVector::<N>::zero();
    let limit = loop_bound(element_count, N);
    while i < limit {
        let qv = FloatVector::<N>::from_le_bytes(data, elem(q, i));
        let dv = FloatVector::<N>::from_le_bytes(data, elem(d, i));
        sv = fma_vec(qv, dv, sv);
        i += N;
    }
    let mut score = sv.reduce_add();
    while i < element_count {
        score += read_f32(data, elem(q, i)) * read_f32(data, elem(d, i));
        i += 1;
    }
    score
}

/// `(float) (sum / Math.sqrt((double) qNorm * (double) dNorm))`.
#[inline]
fn normalize(sum: f32, q_norm: f32, d_norm: f32) -> f32 {
    (f64::from(sum) / (f64::from(q_norm) * f64::from(d_norm)).sqrt()) as f32
}

/// `Cosine.cosineBulkImpl`.
fn cosine_bulk_impl<const N: usize>(
    data: &[u8],
    q: Query<'_>,
    d: [i64; 4],
    element_count: usize,
) -> [f32; 4] {
    q.check(element_count);
    let mut sv = [FloatVector::<N>::zero(); 4];
    let mut qv_norm = FloatVector::<N>::zero();
    let mut dv_norm = [FloatVector::<N>::zero(); 4];
    let limit = loop_bound(element_count, N);
    let mut i = 0;
    while i < limit {
        let dv = d.map(|base| FloatVector::<N>::from_le_bytes(data, elem(base, i)));
        let qv = q.vector::<N>(data, i);
        qv_norm = fma_vec(qv, qv, qv_norm);
        for k in 0..4 {
            dv_norm[k] = fma_vec(dv[k], dv[k], dv_norm[k]);
            sv[k] = fma_vec(qv, dv[k], sv[k]);
        }
        i += N;
    }
    let mut sums = sv.map(FloatVector::reduce_add);
    let mut q_norm = qv_norm.reduce_add();
    let mut d_norms = dv_norm.map(FloatVector::reduce_add);
    while i < element_count {
        let q_value = q.value(data, i);
        let d_values = d.map(|base| read_f32(data, elem(base, i)));
        for k in 0..4 {
            sums[k] = fma(q_value, d_values[k], sums[k]);
        }
        q_norm = fma(q_value, q_value, q_norm);
        for k in 0..4 {
            d_norms[k] = fma(d_values[k], d_values[k], d_norms[k]);
        }
        i += 1;
    }
    [0, 1, 2, 3].map(|k| normalize(sums[k], q_norm, d_norms[k]))
}

/// `Cosine.cosine(MemorySegment, long, long, int)`.
fn cosine_impl<const N: usize>(data: &[u8], q: i64, d: i64, element_count: usize) -> f32 {
    let mut i = 0;
    let mut sv = FloatVector::<N>::zero();
    let mut qv_norm = FloatVector::<N>::zero();
    let mut dv_norm = FloatVector::<N>::zero();
    let limit = loop_bound(element_count, N);
    while i < limit {
        let qv = FloatVector::<N>::from_le_bytes(data, elem(q, i));
        let dv = FloatVector::<N>::from_le_bytes(data, elem(d, i));
        sv = fma_vec(qv, dv, sv);
        qv_norm = fma_vec(qv, qv, qv_norm);
        dv_norm = fma_vec(dv, dv, dv_norm);
        i += N;
    }
    let mut sum = sv.reduce_add();
    let mut q_norm = qv_norm.reduce_add();
    let mut d_norm = dv_norm.reduce_add();
    while i < element_count {
        let q_value = read_f32(data, elem(q, i));
        let d_value = read_f32(data, elem(d, i));
        sum = fma(q_value, d_value, sum);
        q_norm = fma(q_value, q_value, q_norm);
        d_norm = fma(d_value, d_value, d_norm);
        i += 1;
    }
    normalize(sum, q_norm, d_norm)
}

/// `SqrDistance.sqrDistanceBulkImpl`.
fn sqr_distance_bulk_impl<const N: usize>(
    data: &[u8],
    q: Query<'_>,
    d: [i64; 4],
    element_count: usize,
) -> [f32; 4] {
    q.check(element_count);
    let mut sv = [FloatVector::<N>::zero(); 4];
    let limit = loop_bound(element_count, N);
    let mut i = 0;
    while i < limit {
        let dv = d.map(|base| FloatVector::<N>::from_le_bytes(data, elem(base, i)));
        let qv = q.vector::<N>(data, i);
        let diff = dv.map(|dk| qv.sub(dk));
        for k in 0..4 {
            sv[k] = fma_vec(diff[k], diff[k], sv[k]);
        }
        i += N;
    }
    let mut sums = sv.map(FloatVector::reduce_add);
    while i < element_count {
        let q_value = q.value(data, i);
        let diff = d.map(|base| q_value - read_f32(data, elem(base, i)));
        for k in 0..4 {
            sums[k] = fma(diff[k], diff[k], sums[k]);
        }
        i += 1;
    }
    sums
}

/// `SqrDistance.sqrDistance(MemorySegment, long, long, int)`.
fn sqr_distance_impl<const N: usize>(data: &[u8], q: i64, d: i64, element_count: usize) -> f32 {
    let mut i = 0;
    let mut sv = FloatVector::<N>::zero();
    let limit = loop_bound(element_count, N);
    while i < limit {
        let qv = FloatVector::<N>::from_le_bytes(data, elem(q, i));
        let dv = FloatVector::<N>::from_le_bytes(data, elem(d, i));
        let diff = qv.sub(dv);
        sv = fma_vec(diff, diff, sv);
        i += N;
    }
    let mut score = sv.reduce_add();
    while i < element_count {
        let diff = read_f32(data, elem(q, i)) - read_f32(data, elem(d, i));
        score = fma(diff, diff, score);
        i += 1;
    }
    score
}

/// Writes the four bulk results to `scores[0..4]`, as Java does.
#[inline]
fn store(scores: &mut [f32], values: [f32; 4]) {
    scores[..4].copy_from_slice(&values);
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
        let values = with_float_lanes!(
            float_lanes(),
            dot_product_bulk_impl(
                data_seg.bytes(),
                Query::Array(q),
                [d1, d2, d3, d4],
                element_count
            )
        );
        store(scores, values);
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
        let values = with_float_lanes!(
            float_lanes(),
            dot_product_bulk_impl(
                seg.bytes(),
                Query::Segment(q),
                [d1, d2, d3, d4],
                element_count
            )
        );
        store(scores, values);
    }

    /// Returns the dot product of two vectors that both live in the segment.
    ///
    /// Equivalent to `DotProduct.dotProduct(MemorySegment, long, long, int)`.
    ///
    /// # Panics
    ///
    /// Panics when either vector runs past the end of the segment.
    pub fn dot_product(&self, seg: &MemorySegment, q: i64, d: i64, element_count: usize) -> f32 {
        with_float_lanes!(
            float_lanes(),
            dot_product_impl(seg.bytes(), q, d, element_count)
        )
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
    /// The four cosine similarities are written to `scores[0..4]`.
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
        let values = with_float_lanes!(
            float_lanes(),
            cosine_bulk_impl(
                data_seg.bytes(),
                Query::Array(q),
                [d1, d2, d3, d4],
                element_count
            )
        );
        store(scores, values);
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
        let values = with_float_lanes!(
            float_lanes(),
            cosine_bulk_impl(
                seg.bytes(),
                Query::Segment(q),
                [d1, d2, d3, d4],
                element_count
            )
        );
        store(scores, values);
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
        with_float_lanes!(float_lanes(), cosine_impl(seg.bytes(), q, d, element_count))
    }
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
    /// The four squared distances are written to `scores[0..4]`.
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
        let values = with_float_lanes!(
            float_lanes(),
            sqr_distance_bulk_impl(
                data_seg.bytes(),
                Query::Array(q),
                [d1, d2, d3, d4],
                element_count
            )
        );
        store(scores, values);
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
        let values = with_float_lanes!(
            float_lanes(),
            sqr_distance_bulk_impl(
                seg.bytes(),
                Query::Segment(q),
                [d1, d2, d3, d4],
                element_count
            )
        );
        store(scores, values);
    }

    /// Returns the squared Euclidean distance of two vectors that both live in
    /// the segment.
    ///
    /// Equivalent to `SqrDistance.sqrDistance(MemorySegment, long, long, int)`.
    ///
    /// # Panics
    ///
    /// Panics when either vector runs past the end of the segment.
    pub fn sqr_distance(&self, seg: &MemorySegment, q: i64, d: i64, element_count: usize) -> f32 {
        with_float_lanes!(
            float_lanes(),
            sqr_distance_impl(seg.bytes(), q, d, element_count)
        )
    }
}
