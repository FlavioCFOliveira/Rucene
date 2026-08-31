//! Port of `org.apache.lucene.internal.vectorization.VectorUtilSupport`.

#![deny(unsafe_code)]

use std::fmt::Debug;

use crate::error::Result;

/// Backend that [`crate::util::vector_util`] delegates its arithmetic to.
///
/// Equivalent to `org.apache.lucene.internal.vectorization.VectorUtilSupport`.
///
/// Lucene has exactly two implementations: the scalar
/// [`DefaultVectorUtilSupport`](super::DefaultVectorUtilSupport) and the SIMD
/// `PanamaVectorUtilSupport`, chosen at run time by
/// [`VectorizationProvider`](super::VectorizationProvider). Only the scalar one
/// exists in this port; see the [module docs](super) for why.
///
/// # Divergences from Lucene 10.5.0
///
/// * **Overload split.** Java overloads `dotProduct`, `cosine` and
///   `squareDistance` on `float[]` and `byte[]`. Rust has no overloading, so
///   the names carry the `_f32` / `_bytes` suffixes already established by
///   [`crate::util::vector_util`].
/// * **`Result` instead of primitives.** Java returns a bare primitive and lets
///   a length mismatch surface as `ArrayIndexOutOfBoundsException`. The methods
///   whose operands must agree in length return [`Result`] here, matching the
///   signatures [`crate::util::vector_util`] already uses so that the port can
///   delegate to them instead of duplicating the arithmetic. Methods that
///   cannot fail keep a bare return value.
/// * **`Send + Sync + Debug` bounds.** Lucene documents the implementations as
///   stateless singletons; Rust must state that in the type system because the
///   provider holding one lives in a `static`.
pub trait VectorUtilSupport: Send + Sync + Debug {
    /// Calculates the dot product of the given float arrays.
    ///
    /// Equivalent to `VectorUtilSupport.dotProduct(float[], float[])`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneError::IllegalArgument`] when the dimensions
    /// differ.
    fn dot_product_f32(&self, a: &[f32], b: &[f32]) -> Result<f32>;

    /// Returns the cosine similarity between the two vectors.
    ///
    /// Equivalent to `VectorUtilSupport.cosine(float[], float[])`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneError::IllegalArgument`] when the dimensions
    /// differ.
    fn cosine_f32(&self, a: &[f32], b: &[f32]) -> Result<f32>;

    /// Returns the sum of squared differences of the two vectors.
    ///
    /// Equivalent to `VectorUtilSupport.squareDistance(float[], float[])`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneError::IllegalArgument`] when the dimensions
    /// differ.
    fn square_distance_f32(&self, a: &[f32], b: &[f32]) -> Result<f32>;

    /// Returns the dot product computed over signed bytes.
    ///
    /// Equivalent to `VectorUtilSupport.dotProduct(byte[], byte[])`. The bytes
    /// are stored in a `u8` slice and reinterpreted as Java `byte` values.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneError::IllegalArgument`] when the dimensions
    /// differ.
    fn dot_product_bytes(&self, a: &[u8], b: &[u8]) -> Result<i32>;

    /// Returns the dot product computed over unsigned half-bytes, both uncompressed.
    ///
    /// Equivalent to `VectorUtilSupport.int4DotProduct(byte[], byte[])`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneError::IllegalArgument`] when the dimensions
    /// differ.
    fn int4_dot_product(&self, a: &[u8], b: &[u8]) -> Result<i32>;

    /// Returns the dot product computed over unsigned half-bytes, one compressed.
    ///
    /// Equivalent to `VectorUtilSupport.int4DotProductSinglePacked(byte[], byte[])`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneError::IllegalArgument`] when `unpacked` is
    /// shorter than twice the length of `packed`. Lucene has no explicit check
    /// and reads out of bounds instead.
    fn int4_dot_product_single_packed(&self, unpacked: &[u8], packed: &[u8]) -> Result<i32>;

    /// Returns the dot product computed over unsigned half-bytes, both compressed.
    ///
    /// Equivalent to `VectorUtilSupport.int4DotProductBothPacked(byte[], byte[])`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneError::IllegalArgument`] when the dimensions
    /// differ.
    fn int4_dot_product_both_packed(&self, a: &[u8], b: &[u8]) -> Result<i32>;

    /// Returns the dot product computed as though the bytes were unsigned.
    ///
    /// Equivalent to `VectorUtilSupport.uint8DotProduct(byte[], byte[])`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneError::IllegalArgument`] when the dimensions
    /// differ.
    fn uint8_dot_product(&self, a: &[u8], b: &[u8]) -> Result<i32>;

    /// Returns the cosine similarity between the two byte vectors.
    ///
    /// Equivalent to `VectorUtilSupport.cosine(byte[], byte[])`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneError::IllegalArgument`] when the dimensions
    /// differ.
    fn cosine_bytes(&self, a: &[u8], b: &[u8]) -> Result<f32>;

    /// Returns the sum of squared differences of the two byte vectors.
    ///
    /// Equivalent to `VectorUtilSupport.squareDistance(byte[], byte[])`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneError::IllegalArgument`] when the dimensions
    /// differ.
    fn square_distance_bytes(&self, a: &[u8], b: &[u8]) -> Result<i32>;

    /// Returns the sum of squared differences between two unsigned half-byte
    /// vectors, both uncompressed.
    ///
    /// Equivalent to `VectorUtilSupport.int4SquareDistance(byte[], byte[])`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneError::IllegalArgument`] when the dimensions
    /// differ.
    fn int4_square_distance(&self, a: &[u8], b: &[u8]) -> Result<i32>;

    /// Returns the sum of squared differences between two unsigned half-byte
    /// vectors, one compressed.
    ///
    /// Equivalent to `VectorUtilSupport.int4SquareDistanceSinglePacked(byte[], byte[])`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneError::IllegalArgument`] when `unpacked` is
    /// shorter than twice the length of `packed`.
    fn int4_square_distance_single_packed(&self, unpacked: &[u8], packed: &[u8]) -> Result<i32>;

    /// Returns the sum of squared differences between two unsigned half-byte
    /// vectors, both compressed.
    ///
    /// Equivalent to `VectorUtilSupport.int4SquareDistanceBothPacked(byte[], byte[])`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneError::IllegalArgument`] when the dimensions
    /// differ.
    fn int4_square_distance_both_packed(&self, a: &[u8], b: &[u8]) -> Result<i32>;

    /// Returns the sum of squared differences of the two unsigned byte vectors.
    ///
    /// Equivalent to `VectorUtilSupport.uint8SquareDistance(byte[], byte[])`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneError::IllegalArgument`] when the dimensions
    /// differ.
    fn uint8_square_distance(&self, a: &[u8], b: &[u8]) -> Result<i32>;

    /// Given a `buffer` that is sorted between indexes `0` inclusive and `to`
    /// exclusive, finds the first array index whose value is greater than or
    /// equal to `target`.
    ///
    /// Equivalent to `VectorUtilSupport.findNextGEQ(int[], int, int, int)`. The
    /// returned index is guaranteed to be at least `from`; if there is no such
    /// index, `to` is returned.
    ///
    /// # Panics
    ///
    /// Panics when `to` exceeds `buffer.len()`, standing in for Java's
    /// `ArrayIndexOutOfBoundsException`.
    fn find_next_geq(&self, buffer: &[i32], target: i32, from: usize, to: usize) -> usize;

    /// Computes the dot product between a quantized int4 vector and a binary
    /// quantized vector.
    ///
    /// Equivalent to `VectorUtilSupport.int4BitDotProduct(byte[], byte[])`. The
    /// int4 bits are expected to be packed as
    /// `OptimizedScalarQuantizer::transpose_half_byte` produces them, and the
    /// binary bits as `OptimizedScalarQuantizer::pack_as_binary` produces them.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneError::IllegalArgument`] when `int4_quantized` is
    /// not exactly four times as long as `binary_quantized`. Lucene states the
    /// same condition as a Java `assert`, which is disabled in production.
    fn int4_bit_dot_product(&self, int4_quantized: &[u8], binary_quantized: &[u8]) -> Result<i64>;

    /// Computes the dot product between a quantized int4 vector and a dibit
    /// (2-bit) quantized vector.
    ///
    /// Equivalent to `VectorUtilSupport.int4DibitDotProduct(byte[], byte[])`.
    /// The int4 bits are expected to be packed as
    /// `OptimizedScalarQuantizer::transpose_half_byte` produces them (four
    /// stripes) and the dibit bits as
    /// `OptimizedScalarQuantizer::transpose_dibit` produces them (two stripes).
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneError::IllegalArgument`] when `int4_quantized` is
    /// not exactly twice as long as `dibit_quantized`. Lucene states the same
    /// condition as a Java `assert`.
    fn int4_dibit_dot_product(&self, int4_quantized: &[u8], dibit_quantized: &[u8]) -> Result<i64>;

    /// Quantizes `vector`, putting the result into `dest`, and returns the
    /// corrective offset that must be applied to the score.
    ///
    /// Equivalent to
    /// `VectorUtilSupport.minMaxScalarQuantize(float[], byte[], float, float, float, float)`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneError::IllegalArgument`] when `vector` and `dest`
    /// have different lengths. Lucene states the same condition as a Java
    /// `assert`.
    fn min_max_scalar_quantize(
        &self,
        vector: &[f32],
        dest: &mut [u8],
        scale: f32,
        alpha: f32,
        min_quantile: f32,
        max_quantile: f32,
    ) -> Result<f32>;

    /// Recalculates the corrective offset for an already quantized `vector`.
    ///
    /// Equivalent to
    /// `VectorUtilSupport.recalculateScalarQuantizationOffset(byte[], float, float, float, float, float, float)`.
    #[allow(clippy::too_many_arguments)]
    fn recalculate_scalar_quantization_offset(
        &self,
        vector: &[u8],
        old_alpha: f32,
        old_min_quantile: f32,
        scale: f32,
        alpha: f32,
        min_quantile: f32,
        max_quantile: f32,
    ) -> f32;

    /// Filters `doc_buffer` and `score_buffer` in place, keeping only the pairs
    /// whose score is greater than or equal to `min_score_inclusive`, and
    /// returns how many pairs are left.
    ///
    /// Equivalent to `VectorUtilSupport.filterByScore(int[], double[], double, int)`.
    ///
    /// # Panics
    ///
    /// Panics when `up_to` exceeds the length of either buffer, standing in for
    /// Java's `ArrayIndexOutOfBoundsException`.
    fn filter_by_score(
        &self,
        doc_buffer: &mut [i32],
        score_buffer: &mut [f64],
        min_score_inclusive: f64,
        up_to: usize,
    ) -> usize;
}
