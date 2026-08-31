//! Port of `org.apache.lucene.internal.vectorization.PanamaVectorUtilSupport`.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::internal::vectorization::{DefaultVectorUtilSupport, VectorUtilSupport};

/// `VectorUtil` methods implemented with the Panama incubating vector API.
///
/// Equivalent to `org.apache.lucene.internal.vectorization.PanamaVectorUtilSupport`.
///
/// Besides implementing [`VectorUtilSupport`], this type carries the `public
/// static` kernels that the `Lucene99MemorySegment*` scorers call to score
/// straight out of a memory-mapped segment.
///
/// # Divergence from Lucene 10.5.0: the kernels are scalar
///
/// Every kernel in the Java class is written twice: a `jdk.incubator.vector`
/// path over 128/256/512-bit lanes, and a scalar tail loop that finishes the
/// elements the lanes do not cover. The vector path is guarded — on
/// `PanamaVectorConstants.HAS_FAST_INTEGER_VECTORS`, on a minimum vector width,
/// and on a minimum input length — and when a guard fails the tail computes the
/// whole result on its own.
///
/// Stable Rust has no portable SIMD (`std::simd` is nightly-only; this crate's
/// MSRV is 1.80), so the lane path cannot be reproduced.
/// [`PanamaVectorConstants::PREFERRED_VECTOR_BITSIZE`](super::PanamaVectorConstants::PREFERRED_VECTOR_BITSIZE)
/// is therefore zero and every guard fails, which makes **the whole
/// computation Lucene's own scalar tail** — the same code path a JVM takes
/// without `--add-modules jdk.incubator.vector`. That path is
/// [`DefaultVectorUtilSupport`], so this type delegates to it rather than
/// keeping a second copy of the same arithmetic.
///
/// The consequence to be aware of: **float** results can differ in their low
/// bits from a JVM that does have the Vector API, because SIMD reduction adds
/// the partial sums in a different order and floating-point addition is not
/// associative. Integer results — the byte dot products, square distances,
/// `int4BitDotProduct` and `findNextGEQ` — are exact and therefore identical on
/// every path. Lucene has the same split; it is why `VectorUtil`'s reference
/// results are defined by the scalar path (see [`crate::util::vector_util`]).
///
/// # Divergence from Lucene 10.5.0: the overloads unify
///
/// Java declares each memory-segment kernel twice more, once taking
/// `(byte[], MemorySegment)` and once `(MemorySegment, MemorySegment)`, because
/// a `MemorySegment` is not a `byte[]`. In Rust both a heap vector and a
/// [`MemorySegment`](crate::store::MemorySegment)'s
/// [`bytes()`](crate::store::MemorySegment::bytes) are a `&[u8]`, so the two
/// overloads collapse into the single function named after the Java operation.
#[derive(Debug, Default, Clone, Copy)]
pub struct PanamaVectorUtilSupport;

/// The scalar kernels every method here delegates to.
const SCALAR: DefaultVectorUtilSupport = DefaultVectorUtilSupport::new();

impl PanamaVectorUtilSupport {
    /// Creates the (stateless) support.
    ///
    /// Equivalent to the package-private `PanamaVectorUtilSupport()`
    /// constructor.
    pub const fn new() -> Self {
        Self
    }

    /// Returns the dot product computed over signed bytes.
    ///
    /// Equivalent to `PanamaVectorUtilSupport.dotProduct(byte[], MemorySegment)`
    /// and `dotProduct(MemorySegment, MemorySegment)`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneError::IllegalArgument`] when the lengths differ.
    pub fn dot_product(a: &[u8], b: &[u8]) -> Result<i32> {
        SCALAR.dot_product_bytes(a, b)
    }

    /// Returns the dot product computed as though the bytes were unsigned.
    ///
    /// Equivalent to `PanamaVectorUtilSupport.uint8DotProduct(byte[], MemorySegment)`
    /// and `uint8DotProduct(MemorySegment, MemorySegment)`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneError::IllegalArgument`] when the lengths differ.
    pub fn uint8_dot_product(a: &[u8], b: &[u8]) -> Result<i32> {
        SCALAR.uint8_dot_product(a, b)
    }

    /// Returns the dot product computed over unsigned half-bytes, both
    /// uncompressed.
    ///
    /// Equivalent to `PanamaVectorUtilSupport.int4DotProduct(byte[], MemorySegment)`
    /// and `int4DotProduct(MemorySegment, MemorySegment)`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneError::IllegalArgument`] when the lengths differ.
    pub fn int4_dot_product(a: &[u8], b: &[u8]) -> Result<i32> {
        SCALAR.int4_dot_product(a, b)
    }

    /// Returns the dot product computed over unsigned half-bytes, one
    /// compressed.
    ///
    /// Equivalent to
    /// `PanamaVectorUtilSupport.int4DotProductSinglePacked(byte[], MemorySegment)`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneError::IllegalArgument`] when `unpacked` is
    /// shorter than twice the length of `packed`.
    pub fn int4_dot_product_single_packed(unpacked: &[u8], packed: &[u8]) -> Result<i32> {
        SCALAR.int4_dot_product_single_packed(unpacked, packed)
    }

    /// Returns the dot product computed over unsigned half-bytes, both
    /// compressed.
    ///
    /// Equivalent to
    /// `PanamaVectorUtilSupport.int4DotProductBothPacked(MemorySegment, MemorySegment)`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneError::IllegalArgument`] when the lengths differ.
    pub fn int4_dot_product_both_packed(a: &[u8], b: &[u8]) -> Result<i32> {
        SCALAR.int4_dot_product_both_packed(a, b)
    }

    /// Returns the cosine similarity between two byte vectors.
    ///
    /// Equivalent to `PanamaVectorUtilSupport.cosine(byte[], MemorySegment)` and
    /// `cosine(MemorySegment, MemorySegment)`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneError::IllegalArgument`] when the lengths differ.
    pub fn cosine(a: &[u8], b: &[u8]) -> Result<f32> {
        SCALAR.cosine_bytes(a, b)
    }

    /// Returns the sum of squared differences of two signed byte vectors.
    ///
    /// Equivalent to `PanamaVectorUtilSupport.squareDistance(byte[], MemorySegment)`
    /// and `squareDistance(MemorySegment, MemorySegment)`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneError::IllegalArgument`] when the lengths differ.
    pub fn square_distance(a: &[u8], b: &[u8]) -> Result<i32> {
        SCALAR.square_distance_bytes(a, b)
    }

    /// Returns the sum of squared differences of two unsigned byte vectors.
    ///
    /// Equivalent to `PanamaVectorUtilSupport.uint8SquareDistance(byte[], MemorySegment)`
    /// and `uint8SquareDistance(MemorySegment, MemorySegment)`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneError::IllegalArgument`] when the lengths differ.
    pub fn uint8_square_distance(a: &[u8], b: &[u8]) -> Result<i32> {
        SCALAR.uint8_square_distance(a, b)
    }

    /// Returns the sum of squared differences between two unsigned half-byte
    /// vectors, both uncompressed.
    ///
    /// Equivalent to `PanamaVectorUtilSupport.int4SquareDistance(byte[], MemorySegment)`
    /// and `int4SquareDistance(MemorySegment, MemorySegment)`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneError::IllegalArgument`] when the lengths differ.
    pub fn int4_square_distance(a: &[u8], b: &[u8]) -> Result<i32> {
        SCALAR.int4_square_distance(a, b)
    }

    /// Returns the sum of squared differences between two unsigned half-byte
    /// vectors, one compressed.
    ///
    /// Equivalent to
    /// `PanamaVectorUtilSupport.int4SquareDistanceSinglePacked(byte[], MemorySegment)`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneError::IllegalArgument`] when `unpacked` is
    /// shorter than twice the length of `packed`.
    pub fn int4_square_distance_single_packed(unpacked: &[u8], packed: &[u8]) -> Result<i32> {
        SCALAR.int4_square_distance_single_packed(unpacked, packed)
    }

    /// Returns the sum of squared differences between two unsigned half-byte
    /// vectors, both compressed.
    ///
    /// Equivalent to
    /// `PanamaVectorUtilSupport.int4SquareDistanceBothPacked(MemorySegment, MemorySegment)`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneError::IllegalArgument`] when the lengths differ.
    pub fn int4_square_distance_both_packed(a: &[u8], b: &[u8]) -> Result<i32> {
        SCALAR.int4_square_distance_both_packed(a, b)
    }
}

impl VectorUtilSupport for PanamaVectorUtilSupport {
    fn dot_product_f32(&self, a: &[f32], b: &[f32]) -> Result<f32> {
        SCALAR.dot_product_f32(a, b)
    }

    fn cosine_f32(&self, a: &[f32], b: &[f32]) -> Result<f32> {
        SCALAR.cosine_f32(a, b)
    }

    fn square_distance_f32(&self, a: &[f32], b: &[f32]) -> Result<f32> {
        SCALAR.square_distance_f32(a, b)
    }

    fn dot_product_bytes(&self, a: &[u8], b: &[u8]) -> Result<i32> {
        Self::dot_product(a, b)
    }

    fn int4_dot_product(&self, a: &[u8], b: &[u8]) -> Result<i32> {
        Self::int4_dot_product(a, b)
    }

    fn int4_dot_product_single_packed(&self, unpacked: &[u8], packed: &[u8]) -> Result<i32> {
        Self::int4_dot_product_single_packed(unpacked, packed)
    }

    fn int4_dot_product_both_packed(&self, a: &[u8], b: &[u8]) -> Result<i32> {
        Self::int4_dot_product_both_packed(a, b)
    }

    fn uint8_dot_product(&self, a: &[u8], b: &[u8]) -> Result<i32> {
        Self::uint8_dot_product(a, b)
    }

    fn cosine_bytes(&self, a: &[u8], b: &[u8]) -> Result<f32> {
        Self::cosine(a, b)
    }

    fn square_distance_bytes(&self, a: &[u8], b: &[u8]) -> Result<i32> {
        Self::square_distance(a, b)
    }

    fn int4_square_distance(&self, a: &[u8], b: &[u8]) -> Result<i32> {
        Self::int4_square_distance(a, b)
    }

    fn int4_square_distance_single_packed(&self, unpacked: &[u8], packed: &[u8]) -> Result<i32> {
        Self::int4_square_distance_single_packed(unpacked, packed)
    }

    fn int4_square_distance_both_packed(&self, a: &[u8], b: &[u8]) -> Result<i32> {
        Self::int4_square_distance_both_packed(a, b)
    }

    fn uint8_square_distance(&self, a: &[u8], b: &[u8]) -> Result<i32> {
        Self::uint8_square_distance(a, b)
    }

    fn find_next_geq(&self, buffer: &[i32], target: i32, from: usize, to: usize) -> usize {
        // Lucene guards the SIMD "V1 intersection" scan (Lemire, Boytsov, Kurz,
        // <https://arxiv.org/pdf/1401.6399>) on having at least eight int lanes,
        // and falls through to this linear scan otherwise. Both branches return
        // the same index; only the number of comparisons differs.
        SCALAR.find_next_geq(buffer, target, from, to)
    }

    fn int4_bit_dot_product(&self, int4_quantized: &[u8], binary_quantized: &[u8]) -> Result<i64> {
        // Lucene calls DefaultVectorUtilSupport.int4BitDotProductImpl here too,
        // whenever the document vector is shorter than 16 bytes or integer
        // vectors are not fast.
        DefaultVectorUtilSupport::int4_bit_dot_product_impl(int4_quantized, binary_quantized)
    }

    fn int4_dibit_dot_product(&self, int4_quantized: &[u8], dibit_quantized: &[u8]) -> Result<i64> {
        // Same fallback as `int4_bit_dot_product`, on a stripe shorter than 16.
        DefaultVectorUtilSupport::int4_dibit_dot_product_impl(int4_quantized, dibit_quantized)
    }

    fn min_max_scalar_quantize(
        &self,
        vector: &[f32],
        dest: &mut [u8],
        scale: f32,
        alpha: f32,
        min_quantile: f32,
        max_quantile: f32,
    ) -> Result<f32> {
        // Lucene vectorizes only when the preferred width is at least 256 bits
        // and then finishes with `ScalarQuantizer.quantize(vector, dest, i)`;
        // with no lanes, that tail quantizes the whole vector.
        SCALAR.min_max_scalar_quantize(vector, dest, scale, alpha, min_quantile, max_quantile)
    }

    fn recalculate_scalar_quantization_offset(
        &self,
        vector: &[u8],
        old_alpha: f32,
        old_min_quantile: f32,
        scale: f32,
        alpha: f32,
        min_quantile: f32,
        max_quantile: f32,
    ) -> f32 {
        // As above, this is Lucene's `ScalarQuantizer.recalculateOffset(vector,
        // i, ...)` tail with `i == 0`.
        SCALAR.recalculate_scalar_quantization_offset(
            vector,
            old_alpha,
            old_min_quantile,
            scale,
            alpha,
            min_quantile,
            max_quantile,
        )
    }

    fn filter_by_score(
        &self,
        doc_buffer: &mut [i32],
        score_buffer: &mut [f64],
        min_score_inclusive: f64,
        up_to: usize,
    ) -> usize {
        // Lucene's vector path is additionally gated on
        // `Constants.HAS_FAST_COMPRESS_MASK_CAST`; the scalar loop that follows
        // it produces the same surviving pairs in the same order.
        SCALAR.filter_by_score(doc_buffer, score_buffer, min_score_inclusive, up_to)
    }
}
