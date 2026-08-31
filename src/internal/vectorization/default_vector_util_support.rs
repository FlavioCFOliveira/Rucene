//! Port of `org.apache.lucene.internal.vectorization.DefaultVectorUtilSupport`.

#![deny(unsafe_code)]

use crate::error::{LuceneError, Result};
use crate::internal::vectorization::VectorUtilSupport;
use crate::util::vector_util;
use crate::util::BitUtil;

/// Scalar implementation of [`VectorUtilSupport`].
///
/// Equivalent to `org.apache.lucene.internal.vectorization.DefaultVectorUtilSupport`.
///
/// The float and byte similarity kernels are not re-derived here: they already
/// exist in [`crate::util::vector_util`], which reproduces this very class
/// operation by operation (the accumulator count and the summation order are
/// observable in the result), so this type delegates to them. The kernels that
/// `VectorUtil` does not expose — the int4/uint8 variants, the bit dot
/// products, the scalar quantizer and the buffer filters — are implemented
/// below.
///
/// # Divergence from Lucene 10.5.0
///
/// Lucene declares this class package-private. Rust has no package visibility,
/// and the type must be nameable for
/// [`DefaultVectorizationProvider`](super::DefaultVectorizationProvider) to
/// return it, so it is `pub` here. It is not part of Rucene's supported API.
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultVectorUtilSupport;

impl DefaultVectorUtilSupport {
    /// Creates the (stateless) scalar support.
    ///
    /// Equivalent to the package-private `DefaultVectorUtilSupport()` constructor.
    pub const fn new() -> Self {
        Self
    }

    /// Computes the dot product between a transposed 4-bit query vector and a
    /// transposed binary document vector.
    ///
    /// Equivalent to the `public static`
    /// `DefaultVectorUtilSupport.int4BitDotProductImpl(byte[], byte[])`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `q` is not exactly four
    /// times as long as `d`; Lucene states this as a Java `assert`.
    pub fn int4_bit_dot_product_impl(q: &[u8], d: &[u8]) -> Result<i64> {
        if q.len() != d.len() * 4 {
            return Err(LuceneError::IllegalArgument(format!(
                "vector dimensions incompatible: {}!={}",
                q.len(),
                d.len() * 4
            )));
        }
        Ok(Self::int4_bit_dot_product_stripe(q, d, 0, d.len()))
    }

    /// Computes the dot product between a transposed 4-bit query vector and a
    /// transposed 2-bit document vector.
    ///
    /// Equivalent to the `public static`
    /// `DefaultVectorUtilSupport.int4DibitDotProductImpl(byte[], byte[])`. The
    /// dibit vector has two stripes (lower bits first, then upper bits), so the
    /// scoring is two passes of the int4-bit dot product with the results
    /// shifted appropriately.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `q` is not exactly twice
    /// as long as `d`; Lucene states this as a Java `assert`.
    pub fn int4_dibit_dot_product_impl(q: &[u8], d: &[u8]) -> Result<i64> {
        if q.len() != d.len() * 2 {
            return Err(LuceneError::IllegalArgument(format!(
                "vector dimensions incompatible: {}!={}",
                q.len(),
                d.len() * 2
            )));
        }
        let stripe_size = d.len() / 2;
        let ret0 = Self::int4_bit_dot_product_stripe(q, d, 0, stripe_size);
        let ret1 = Self::int4_bit_dot_product_stripe(q, d, stripe_size, stripe_size);
        Ok(ret0 + (ret1 << 1))
    }

    /// Computes the int4-bit dot product against one stripe of the document
    /// vector.
    ///
    /// Equivalent to the private
    /// `DefaultVectorUtilSupport.int4BitDotProductImpl(byte[], byte[], int, int)`.
    ///
    /// Lucene reads four bytes at a time through `BitUtil.VH_NATIVE_INT`. The
    /// byte order is irrelevant to the result because the value is only ever
    /// fed to a population count, which is invariant under byte permutation, so
    /// this port reads little-endian unconditionally.
    fn int4_bit_dot_product_stripe(q: &[u8], d: &[u8], d_offset: usize, stripe_size: usize) -> i64 {
        let mut ret: i64 = 0;
        for i in 0..4usize {
            let mut r = 0usize;
            let mut sub_ret: i64 = 0;
            let upper_bound = stripe_size & !(std::mem::size_of::<i32>() - 1);
            while r < upper_bound {
                let qv = BitUtil::read_le_int(q, i * stripe_size + r) as u32;
                let dv = BitUtil::read_le_int(d, d_offset + r) as u32;
                sub_ret += i64::from((qv & dv).count_ones());
                r += std::mem::size_of::<i32>();
            }
            while r < stripe_size {
                sub_ret += i64::from((q[i * stripe_size + r] & d[d_offset + r]).count_ones());
                r += 1;
            }
            ret += sub_ret << i;
        }
        ret
    }
}

/// Reinterprets one stored byte as the signed Java `byte` it represents.
///
/// Lucene byte vectors hold values in `[-128, 127]`; Rucene stores them in a
/// `u8` slice, so every arithmetic use must go through this reinterpretation.
#[inline]
fn signed(value: u8) -> i32 {
    value as i8 as i32
}

/// `Byte.toUnsignedInt(byte)`.
#[inline]
fn unsigned(value: u8) -> i32 {
    value as i32
}

/// Returns an error when the two vectors have different dimensions.
///
/// The message is verbatim from `VectorUtil`, which throws
/// `IllegalArgumentException("vector dimensions differ: " + a.length + "!=" + b.length)`.
#[inline]
fn check_same_dimensions(a: &[u8], b: &[u8]) -> Result<()> {
    if a.len() != b.len() {
        return Err(LuceneError::IllegalArgument(format!(
            "vector dimensions differ: {}!={}",
            a.len(),
            b.len()
        )));
    }
    Ok(())
}

/// Returns an error when `unpacked` cannot hold the two half-byte planes that
/// `packed` addresses.
#[inline]
fn check_single_packed(unpacked: &[u8], packed: &[u8]) -> Result<()> {
    if unpacked.len() < packed.len() * 2 {
        return Err(LuceneError::IllegalArgument(format!(
            "vector dimensions differ: {}!={}",
            unpacked.len(),
            packed.len() * 2
        )));
    }
    Ok(())
}

/// `Math.min(float, float)` as specified by the JLS.
///
/// `f32::min` in Rust returns the non-NaN operand when exactly one operand is
/// NaN, whereas Java propagates NaN, and Java also prefers `-0.0` over `0.0`.
#[inline]
fn java_math_min(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && b.is_sign_negative() {
        return b;
    }
    if a <= b {
        a
    } else {
        b
    }
}

/// `Math.max(float, float)` as specified by the JLS.
///
/// See [`java_math_min`] for why `f32::max` cannot be used.
#[inline]
fn java_math_max(a: f32, b: f32) -> f32 {
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

/// `Math.round(float)` as specified by the JLS.
///
/// Reproduces `java.lang.Math.round(float)` bit for bit. `f32::round` in Rust
/// rounds halves away from zero, whereas Java rounds them towards positive
/// infinity (`Math.round(-0.5f)` is `0`, `f32::round(-0.5)` is `-1.0`), and the
/// naive `floor(a + 0.5f)` form double-rounds for arguments just below a half.
/// The scalar quantizer stores the result in a byte, so the difference is
/// directly observable in the quantized vector.
#[inline]
fn java_math_round(a: f32) -> i32 {
    /// `FloatConsts.SIGNIFICAND_WIDTH`.
    const SIGNIFICAND_WIDTH: i32 = 24;
    /// `FloatConsts.EXP_BIAS`.
    const EXP_BIAS: i32 = 127;
    /// `FloatConsts.EXP_BIT_MASK`.
    const EXP_BIT_MASK: i32 = 0x7F80_0000;
    /// `FloatConsts.SIGNIF_BIT_MASK`.
    const SIGNIF_BIT_MASK: i32 = 0x007F_FFFF;

    let int_bits = a.to_bits() as i32;
    let biased_exp = (int_bits & EXP_BIT_MASK) >> (SIGNIFICAND_WIDTH - 1);
    let shift = (SIGNIFICAND_WIDTH - 2 + EXP_BIAS) - biased_exp;
    if (shift & -32) == 0 {
        // shift is in [0, 32): the exponent is small enough that the rounded
        // value fits, so round the significand directly.
        let mut r = (int_bits & SIGNIF_BIT_MASK) | (SIGNIF_BIT_MASK + 1);
        if int_bits < 0 {
            r = -r;
        }
        ((r >> shift) + 1) >> 1
    } else {
        // The argument is NaN, zero, or too large to need rounding; Java's
        // narrowing float-to-int conversion is saturating and maps NaN to zero,
        // which is exactly what `as i32` does in Rust.
        a as i32
    }
}

/// The scalar quantizer used by `minMaxScalarQuantize` and
/// `recalculateScalarQuantizationOffset`.
///
/// Equivalent to the package-private nested class
/// `DefaultVectorUtilSupport.ScalarQuantizer`.
#[derive(Debug, Clone, Copy)]
struct ScalarQuantizer {
    alpha: f32,
    scale: f32,
    min_quantile: f32,
    max_quantile: f32,
}

impl ScalarQuantizer {
    fn new(alpha: f32, scale: f32, min_quantile: f32, max_quantile: f32) -> Self {
        Self {
            alpha,
            scale,
            min_quantile,
            max_quantile,
        }
    }

    /// Equivalent to `ScalarQuantizer.quantize(float[], byte[], int)`.
    fn quantize(&self, vector: &[f32], dest: &mut [u8], start: usize) -> f32 {
        debug_assert_eq!(vector.len(), dest.len());
        let mut correction = 0.0f32;
        for (v, d) in vector[start..].iter().zip(dest[start..].iter_mut()) {
            correction += self.quantize_float(*v, Some(d));
        }
        correction
    }

    /// Equivalent to `ScalarQuantizer.recalculateOffset(byte[], int, float, float)`.
    fn recalculate_offset(
        &self,
        vector: &[u8],
        start: usize,
        old_alpha: f32,
        old_min_quantile: f32,
    ) -> f32 {
        let mut correction = 0.0f32;
        for &b in &vector[start..] {
            // undo the old quantization
            let v = (old_alpha * unsigned(b) as f32) + old_min_quantile;
            correction += self.quantize_float(v, None);
        }
        correction
    }

    /// Equivalent to `ScalarQuantizer.quantizeFloat(float, byte[], int)`.
    ///
    /// Java passes a `byte[]` plus an index, and `null` for the array when only
    /// the corrective offset is wanted; the Rust port passes the destination
    /// slot itself, or `None`.
    fn quantize_float(&self, v: f32, dest: Option<&mut u8>) -> f32 {
        // Make sure the value is within the quantile range, cutting off the
        // tails. See the first parenthesis in the equation:
        // byte = (float - minQuantile) * 127/(maxQuantile - minQuantile)
        let dx = v - self.min_quantile;
        let dxc = java_math_max(self.min_quantile, java_math_min(self.max_quantile, v))
            - self.min_quantile;
        // Scale the value to the range [0, 127]; this is our quantized value.
        // scale = 127/(maxQuantile - minQuantile)
        let rounded_dxs = java_math_round(self.scale * dxc);
        // We multiply by `alpha` here to get the quantized value back into the
        // original range, to aid in calculating the corrective offset.
        let dxq = rounded_dxs as f32 * self.alpha;
        if let Some(dest) = dest {
            *dest = rounded_dxs as u8;
        }
        // Calculate the corrective offset that needs to be applied to the score
        // in addition to the `byte * minQuantile * alpha` term in the equation:
        // the `(dx - dxq) * dxq` term accounts for the rounding of the
        // quantized value, and `minQuantile^2` is the global correction.
        self.min_quantile * (v - self.min_quantile / 2.0f32) + (dx - dxq) * dxq
    }
}

impl VectorUtilSupport for DefaultVectorUtilSupport {
    fn dot_product_f32(&self, a: &[f32], b: &[f32]) -> Result<f32> {
        vector_util::dot_product_f32(a, b)
    }

    fn cosine_f32(&self, a: &[f32], b: &[f32]) -> Result<f32> {
        vector_util::cosine_f32(a, b)
    }

    fn square_distance_f32(&self, a: &[f32], b: &[f32]) -> Result<f32> {
        vector_util::square_distance_f32(a, b)
    }

    fn dot_product_bytes(&self, a: &[u8], b: &[u8]) -> Result<i32> {
        vector_util::dot_product_bytes(a, b)
    }

    fn uint8_dot_product(&self, a: &[u8], b: &[u8]) -> Result<i32> {
        check_same_dimensions(a, b)?;
        let mut total: i32 = 0;
        for i in 0..a.len() {
            total = total.wrapping_add(unsigned(a[i]).wrapping_mul(unsigned(b[i])));
        }
        Ok(total)
    }

    fn int4_dot_product(&self, a: &[u8], b: &[u8]) -> Result<i32> {
        self.dot_product_bytes(a, b)
    }

    fn int4_dot_product_single_packed(&self, unpacked: &[u8], packed: &[u8]) -> Result<i32> {
        check_single_packed(unpacked, packed)?;
        let mut total: i32 = 0;
        for i in 0..packed.len() {
            let packed_byte = packed[i];
            let unpacked1 = signed(unpacked[i]);
            let unpacked2 = signed(unpacked[i + packed.len()]);
            total = total.wrapping_add(i32::from(packed_byte & 0x0F).wrapping_mul(unpacked2));
            total = total.wrapping_add(i32::from(packed_byte >> 4).wrapping_mul(unpacked1));
        }
        Ok(total)
    }

    fn int4_dot_product_both_packed(&self, a: &[u8], b: &[u8]) -> Result<i32> {
        check_same_dimensions(a, b)?;
        let mut total: i32 = 0;
        for i in 0..a.len() {
            let a_byte = a[i];
            let b_byte = b[i];
            total =
                total.wrapping_add(i32::from(a_byte & 0x0F).wrapping_mul(i32::from(b_byte & 0x0F)));
            total = total.wrapping_add(i32::from(a_byte >> 4).wrapping_mul(i32::from(b_byte >> 4)));
        }
        Ok(total)
    }

    fn cosine_bytes(&self, a: &[u8], b: &[u8]) -> Result<f32> {
        vector_util::cosine_bytes(a, b)
    }

    fn square_distance_bytes(&self, a: &[u8], b: &[u8]) -> Result<i32> {
        vector_util::square_distance_bytes(a, b)
    }

    fn int4_square_distance(&self, a: &[u8], b: &[u8]) -> Result<i32> {
        self.square_distance_bytes(a, b)
    }

    fn int4_square_distance_single_packed(&self, unpacked: &[u8], packed: &[u8]) -> Result<i32> {
        check_single_packed(unpacked, packed)?;
        let mut total: i32 = 0;
        for i in 0..packed.len() {
            let packed_byte = packed[i];
            let unpacked1 = signed(unpacked[i]);
            let unpacked2 = signed(unpacked[i + packed.len()]);

            let diff1 = i32::from(packed_byte & 0x0F).wrapping_sub(unpacked2);
            let diff2 = i32::from(packed_byte >> 4).wrapping_sub(unpacked1);

            total = total.wrapping_add(
                diff1
                    .wrapping_mul(diff1)
                    .wrapping_add(diff2.wrapping_mul(diff2)),
            );
        }
        Ok(total)
    }

    fn int4_square_distance_both_packed(&self, a: &[u8], b: &[u8]) -> Result<i32> {
        check_same_dimensions(a, b)?;
        let mut total: i32 = 0;
        for i in 0..a.len() {
            let a_byte = a[i];
            let b_byte = b[i];

            let diff1 = i32::from(a_byte & 0x0F).wrapping_sub(i32::from(b_byte & 0x0F));
            let diff2 = i32::from(a_byte >> 4).wrapping_sub(i32::from(b_byte >> 4));

            total = total.wrapping_add(
                diff1
                    .wrapping_mul(diff1)
                    .wrapping_add(diff2.wrapping_mul(diff2)),
            );
        }
        Ok(total)
    }

    fn uint8_square_distance(&self, a: &[u8], b: &[u8]) -> Result<i32> {
        check_same_dimensions(a, b)?;
        // Note: this will not overflow if dim < 2^16, since max(ubyte * ubyte) = 2^16.
        let mut square_sum: i32 = 0;
        for i in 0..a.len() {
            let diff = unsigned(a[i]).wrapping_sub(unsigned(b[i]));
            square_sum = square_sum.wrapping_add(diff.wrapping_mul(diff));
        }
        Ok(square_sum)
    }

    fn find_next_geq(&self, buffer: &[i32], target: i32, from: usize, to: usize) -> usize {
        for (i, &value) in buffer.iter().enumerate().take(to).skip(from) {
            if value >= target {
                return i;
            }
        }
        to
    }

    fn int4_bit_dot_product(&self, int4_quantized: &[u8], binary_quantized: &[u8]) -> Result<i64> {
        Self::int4_bit_dot_product_impl(int4_quantized, binary_quantized)
    }

    fn int4_dibit_dot_product(&self, int4_quantized: &[u8], dibit_quantized: &[u8]) -> Result<i64> {
        Self::int4_dibit_dot_product_impl(int4_quantized, dibit_quantized)
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
        if vector.len() != dest.len() {
            return Err(LuceneError::IllegalArgument(format!(
                "vector dimensions differ: {}!={}",
                vector.len(),
                dest.len()
            )));
        }
        Ok(
            ScalarQuantizer::new(alpha, scale, min_quantile, max_quantile)
                .quantize(vector, dest, 0),
        )
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
        ScalarQuantizer::new(alpha, scale, min_quantile, max_quantile).recalculate_offset(
            vector,
            0,
            old_alpha,
            old_min_quantile,
        )
    }

    fn filter_by_score(
        &self,
        doc_buffer: &mut [i32],
        score_buffer: &mut [f64],
        min_score_inclusive: f64,
        up_to: usize,
    ) -> usize {
        let mut new_size = 0usize;
        for i in 0..up_to {
            let doc = doc_buffer[i];
            let score = score_buffer[i];
            doc_buffer[new_size] = doc;
            score_buffer[new_size] = score;
            if score >= min_score_inclusive {
                new_size += 1;
            }
        }
        new_size
    }
}
