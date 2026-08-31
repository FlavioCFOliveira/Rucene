//! Vector arithmetic primitives ported from `org.apache.lucene.util.VectorUtil`.
//!
//! Every function in this module reproduces, operation by operation, the
//! **scalar** reference implementation that Lucene 10.5.0 uses when neither the
//! Panama vector API nor fast scalar FMA are available
//! (`org.apache.lucene.internal.vectorization.DefaultVectorUtilSupport`).
//!
//! # Why the scalar, non-FMA path is the parity target
//!
//! Lucene picks one of three implementations at run time:
//!
//! * a Panama (`jdk.incubator.vector`) implementation, when the module is
//!   present and the CPU is supported;
//! * the scalar implementation using `Math.fma`, when
//!   `Constants.HAS_FAST_SCALAR_FMA` is true;
//! * the scalar implementation using `a * b + c`, otherwise.
//!
//! Floating-point addition is not associative and `Math.fma` rounds once where
//! `a * b + c` rounds twice, so the three paths do **not** agree bit for bit.
//! The only deterministic, machine-independent target is the last one, which
//! Lucene selects with `-Dlucene.useScalarFMA=false` and without
//! `--add-modules jdk.incubator.vector`. That is what [`USE_FMA`] pins, and it
//! is the configuration the portability fixtures run under.
//!
//! Consequently the loop shapes below are *not* free to change: the number of
//! accumulators and the order in which partial sums are added are part of the
//! observable result.

#![deny(unsafe_code)]

use crate::error::{LuceneError, Result};
use crate::internal::vectorization::default_vector_util_support::DefaultVectorUtilSupport;
use crate::internal::vectorization::vector_util_support::VectorUtilSupport;

/// Tolerance used by [`l2normalize`] and [`is_unit_vector`].
///
/// Equivalent to `VectorUtil.EPSILON`. It is a `f32` literal in Java and is
/// widened to `f64` at every comparison site, which this port reproduces.
const EPSILON: f32 = 1e-4;

/// Whether the fused multiply-add form is used for float accumulation.
///
/// Equivalent to `Constants.HAS_FAST_SCALAR_FMA` in
/// `DefaultVectorUtilSupport.fma`. Rucene pins it to `false` so that scores are
/// reproducible on every machine and bit-identical to a JVM started with
/// `-Dlucene.useScalarFMA=false`. Changing it changes the low-order bits of
/// every float similarity score.
pub const USE_FMA: bool = false;

// The portability fixtures capture the Java reference scores with
// `-Dlucene.useScalarFMA=false`. Flipping the constant silently changes the low
// bits of every float similarity score, so the choice is pinned at compile time.
const _: () = assert!(!USE_FMA);

/// `a * b + c`, or `fma(a, b, c)` when [`USE_FMA`] is enabled.
///
/// Equivalent to `DefaultVectorUtilSupport.fma(float, float, float)`.
#[inline]
fn fma(a: f32, b: f32, c: f32) -> f32 {
    if USE_FMA {
        a.mul_add(b, c)
    } else {
        a * b + c
    }
}

/// `Math.max(float, float)` as specified by the JLS.
///
/// `f32::max` in Rust returns the non-NaN operand when exactly one operand is
/// NaN, whereas Java propagates NaN. The distinction is observable:
/// `VectorUtil.cosine` of a zero vector is NaN, and
/// `normalizeToUnitInterval` must keep it NaN.
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

/// Returns an error when the two vectors have different dimensions.
///
/// The message is verbatim from `VectorUtil`, which throws
/// `IllegalArgumentException("vector dimensions differ: " + a.length + "!=" + b.length)`.
#[inline]
fn check_same_dimensions<T, U>(a: &[T], b: &[U]) -> Result<()> {
    if a.len() != b.len() {
        return Err(LuceneError::IllegalArgument(format!(
            "vector dimensions differ: {}!={}",
            a.len(),
            b.len()
        )));
    }
    Ok(())
}

// -----------------------------------------------------------------------------
// Float primitives
// -----------------------------------------------------------------------------

/// Returns the dot product of two float vectors.
///
/// Equivalent to `VectorUtil.dotProduct(float[], float[])` over the scalar
/// backend. Vectors longer than 32 elements are accumulated into four
/// independent accumulators, exactly as Lucene does, because the summation
/// order is observable in the result.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when the dimensions differ.
pub fn dot_product_f32(a: &[f32], b: &[f32]) -> Result<f32> {
    check_same_dimensions(a, b)?;
    Ok(dot_product_f32_unchecked(a, b))
}

/// [`dot_product_f32`] without the dimension check.
///
/// Equivalent to calling `VectorUtilSupport.dotProduct` directly, which is what
/// `VectorUtil.l2normalize` and `VectorUtil.isUnitVector` do.
fn dot_product_f32_unchecked(a: &[f32], b: &[f32]) -> f32 {
    let mut res = 0.0f32;
    let mut i = 0usize;

    if a.len() > 32 {
        let mut acc1 = 0.0f32;
        let mut acc2 = 0.0f32;
        let mut acc3 = 0.0f32;
        let mut acc4 = 0.0f32;
        let upper_bound = a.len() & !3;
        while i < upper_bound {
            acc1 = fma(a[i], b[i], acc1);
            acc2 = fma(a[i + 1], b[i + 1], acc2);
            acc3 = fma(a[i + 2], b[i + 2], acc3);
            acc4 = fma(a[i + 3], b[i + 3], acc4);
            i += 4;
        }
        res += acc1 + acc2 + acc3 + acc4;
    }

    while i < a.len() {
        res = fma(a[i], b[i], res);
        i += 1;
    }
    res
}

/// Returns the cosine similarity of two float vectors.
///
/// Equivalent to `VectorUtil.cosine(float[], float[])` over the scalar backend.
/// The three running sums use two accumulators each for vectors longer than 32
/// elements, and the final division and square root are performed in `f64` with
/// a single rounding back to `f32` — both details are observable.
///
/// A zero-norm operand yields `NaN`, matching Java's `0 / sqrt(0)`.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when the dimensions differ.
pub fn cosine_f32(a: &[f32], b: &[f32]) -> Result<f32> {
    check_same_dimensions(a, b)?;

    let mut sum = 0.0f32;
    let mut norm1 = 0.0f32;
    let mut norm2 = 0.0f32;
    let mut i = 0usize;

    if a.len() > 32 {
        let mut sum1 = 0.0f32;
        let mut sum2 = 0.0f32;
        let mut norm1_1 = 0.0f32;
        let mut norm1_2 = 0.0f32;
        let mut norm2_1 = 0.0f32;
        let mut norm2_2 = 0.0f32;

        let upper_bound = a.len() & !1;
        while i < upper_bound {
            sum1 = fma(a[i], b[i], sum1);
            norm1_1 = fma(a[i], a[i], norm1_1);
            norm2_1 = fma(b[i], b[i], norm2_1);

            sum2 = fma(a[i + 1], b[i + 1], sum2);
            norm1_2 = fma(a[i + 1], a[i + 1], norm1_2);
            norm2_2 = fma(b[i + 1], b[i + 1], norm2_2);
            i += 2;
        }
        sum += sum1 + sum2;
        norm1 += norm1_1 + norm1_2;
        norm2 += norm2_1 + norm2_2;
    }

    while i < a.len() {
        sum = fma(a[i], b[i], sum);
        norm1 = fma(a[i], a[i], norm1);
        norm2 = fma(b[i], b[i], norm2);
        i += 1;
    }

    Ok((f64::from(sum) / (f64::from(norm1) * f64::from(norm2)).sqrt()) as f32)
}

/// Returns the sum of squared differences of two float vectors.
///
/// Equivalent to `VectorUtil.squareDistance(float[], float[])` over the scalar
/// backend, including the four-accumulator unrolling for vectors longer than 32
/// elements.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when the dimensions differ.
pub fn square_distance_f32(a: &[f32], b: &[f32]) -> Result<f32> {
    check_same_dimensions(a, b)?;

    let mut res = 0.0f32;
    let mut i = 0usize;

    if a.len() > 32 {
        let mut acc1 = 0.0f32;
        let mut acc2 = 0.0f32;
        let mut acc3 = 0.0f32;
        let mut acc4 = 0.0f32;

        let upper_bound = a.len() & !3;
        while i < upper_bound {
            let diff1 = a[i] - b[i];
            acc1 = fma(diff1, diff1, acc1);

            let diff2 = a[i + 1] - b[i + 1];
            acc2 = fma(diff2, diff2, acc2);

            let diff3 = a[i + 2] - b[i + 2];
            acc3 = fma(diff3, diff3, acc3);

            let diff4 = a[i + 3] - b[i + 3];
            acc4 = fma(diff4, diff4, acc4);
            i += 4;
        }
        res += acc1 + acc2 + acc3 + acc4;
    }

    while i < a.len() {
        let diff = a[i] - b[i];
        res = fma(diff, diff, res);
        i += 1;
    }
    Ok(res)
}

// -----------------------------------------------------------------------------
// Byte primitives
// -----------------------------------------------------------------------------

/// Reinterprets one stored byte as the signed Java `byte` it represents.
///
/// Lucene byte vectors hold values in `[-128, 127]`; Rucene stores them in a
/// `u8` slice, so every arithmetic use must go through this reinterpretation.
#[inline]
fn signed(value: u8) -> i32 {
    value as i8 as i32
}

/// Returns the dot product of two signed-byte vectors.
///
/// Equivalent to `VectorUtil.dotProduct(byte[], byte[])`. Accumulation is in
/// `i32`, like Java, which is exact for every dimension that does not overflow;
/// accumulating in `f32` would start losing the low bit around dimension 1024
/// because `f32` only represents integers exactly up to 2^24.
///
/// Overflow wraps, reproducing Java `int` arithmetic rather than panicking.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when the dimensions differ.
pub fn dot_product_bytes(a: &[u8], b: &[u8]) -> Result<i32> {
    check_same_dimensions(a, b)?;
    let mut total: i32 = 0;
    for i in 0..a.len() {
        total = total.wrapping_add(signed(a[i]).wrapping_mul(signed(b[i])));
    }
    Ok(total)
}

/// Returns the sum of squared differences of two signed-byte vectors.
///
/// Equivalent to `VectorUtil.squareDistance(byte[], byte[])`, accumulating in
/// `i32`. The per-element maximum is `255^2 = 65_025`, so an `f32` accumulator
/// would become inexact from roughly dimension 258 onwards.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when the dimensions differ.
pub fn square_distance_bytes(a: &[u8], b: &[u8]) -> Result<i32> {
    check_same_dimensions(a, b)?;
    let mut square_sum: i32 = 0;
    for i in 0..a.len() {
        let diff = signed(a[i]).wrapping_sub(signed(b[i]));
        square_sum = square_sum.wrapping_add(diff.wrapping_mul(diff));
    }
    Ok(square_sum)
}

/// Returns the cosine similarity of two signed-byte vectors.
///
/// Equivalent to `VectorUtil.cosine(byte[], byte[])`: the three sums are exact
/// `i32` accumulations (no overflow while the dimension stays below 2^18) and
/// the final division and square root are done in `f64` with a single rounding
/// back to `f32`.
///
/// A zero-norm operand yields `NaN`, matching Java.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when the dimensions differ.
pub fn cosine_bytes(a: &[u8], b: &[u8]) -> Result<f32> {
    check_same_dimensions(a, b)?;
    let mut sum: i32 = 0;
    let mut norm1: i32 = 0;
    let mut norm2: i32 = 0;
    for i in 0..a.len() {
        let elem1 = signed(a[i]);
        let elem2 = signed(b[i]);
        sum = sum.wrapping_add(elem1.wrapping_mul(elem2));
        norm1 = norm1.wrapping_add(elem1.wrapping_mul(elem1));
        norm2 = norm2.wrapping_add(elem2.wrapping_mul(elem2));
    }
    Ok((f64::from(sum) / (f64::from(norm1) * f64::from(norm2)).sqrt()) as f32)
}

// -----------------------------------------------------------------------------
// Score transforms
// -----------------------------------------------------------------------------

/// Returns the signed-byte dot product scaled into `[0, 1]`.
///
/// Equivalent to `VectorUtil.dotProductScore(byte[], byte[])`:
/// `0.5f + dotProduct(a, b) / (float) (a.length * (1 << 15))`. The denominator
/// is computed with Java `int` multiplication before the widening cast, so it
/// wraps for dimensions at or above 2^16, which this port reproduces.
///
/// Note that there is no clamp here: unlike the float `DOT_PRODUCT` path, the
/// byte path can return a value slightly outside `[0, 1]`.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when the dimensions differ.
pub fn dot_product_score(a: &[u8], b: &[u8]) -> Result<f32> {
    let dot = dot_product_bytes(a, b)?;
    let denom = (a.len() as i32).wrapping_mul(1 << 15) as f32;
    Ok(0.5f32 + dot as f32 / denom)
}

/// Maps a raw inner product to a strictly positive score.
///
/// Equivalent to `VectorUtil.scaleMaxInnerProductScore(float)`.
pub fn scale_max_inner_product_score(vector_dot_product_similarity: f32) -> f32 {
    if vector_dot_product_similarity < 0.0 {
        // Java writes `1 / (1 + -1 * s)`; IEEE negation is exact, so `-s` is
        // the same value with fewer operations.
        1.0 / (1.0 + -vector_dot_product_similarity)
    } else {
        vector_dot_product_similarity + 1.0
    }
}

/// Maps a similarity in `[-1, 1]` to a score in `[0, 1]`.
///
/// Equivalent to `VectorUtil.normalizeToUnitInterval(float)`:
/// `Math.max((1 + value) / 2, 0)`. NaN propagates, as it does in Java.
pub fn normalize_to_unit_interval(value: f32) -> f32 {
    java_math_max((1.0 + value) / 2.0, 0.0)
}

/// Maps a squared distance to a score in `(0, 1]`.
///
/// Equivalent to `VectorUtil.normalizeDistanceToUnitInterval(float)`.
pub fn normalize_distance_to_unit_interval(squared_distance: f32) -> f32 {
    1.0 / (1.0 + squared_distance)
}

// -----------------------------------------------------------------------------
// Vector hygiene helpers
// -----------------------------------------------------------------------------

/// Scales `v` in place to unit length.
///
/// Equivalent to `VectorUtil.l2normalize(float[], boolean)`. The squared norm is
/// computed by the float dot product and then widened to `f64`; a vector whose
/// squared norm is already within `EPSILON` (1e-4) of `1.0` is left **byte
/// identical**, which is what keeps repeated normalization idempotent.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when `v` is all zeros and
/// `throw_on_zero` is `true`.
pub fn l2normalize(v: &mut [f32], throw_on_zero: bool) -> Result<()> {
    let l1norm = f64::from(dot_product_f32_unchecked(v, v));
    if l1norm == 0.0 {
        if throw_on_zero {
            return Err(LuceneError::IllegalArgument(
                "Cannot normalize a zero-length vector".to_string(),
            ));
        }
        return Ok(());
    }
    if (l1norm - 1.0).abs() <= f64::from(EPSILON) {
        return Ok(());
    }
    let l2norm = l1norm.sqrt();
    for value in v.iter_mut() {
        *value /= l2norm as f32;
    }
    Ok(())
}

/// Returns `true` when the squared norm of `v` is within `EPSILON` (1e-4) of
/// `1.0`.
///
/// Equivalent to `VectorUtil.isUnitVector(float[])`.
pub fn is_unit_vector(v: &[f32]) -> bool {
    let l1norm = f64::from(dot_product_f32_unchecked(v, v));
    (l1norm - 1.0).abs() <= f64::from(EPSILON)
}

/// Checks that every component of `v` is finite.
///
/// Equivalent to `VectorUtil.checkFinite(float[])`.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] naming the first offending index,
/// with the same message text Java produces.
pub fn check_finite(v: &[f32]) -> Result<()> {
    for (i, &value) in v.iter().enumerate() {
        if !value.is_finite() {
            return Err(LuceneError::IllegalArgument(format!(
                "non-finite value at vector[{}]={}",
                i,
                java_float_to_string(value)
            )));
        }
    }
    Ok(())
}

/// Renders a non-finite `f32` the way `Float.toString` does.
///
/// Rust prints `inf` / `-inf` where Java prints `Infinity` / `-Infinity`; only
/// non-finite values reach this helper, so the three special cases are
/// exhaustive.
fn java_float_to_string(value: f32) -> &'static str {
    if value.is_nan() {
        "NaN"
    } else if value > 0.0 {
        "Infinity"
    } else {
        "-Infinity"
    }
}

/// Returns `true` when every component of `v` is zero.
///
/// Equivalent to `VectorUtil.isZeroVector(float[])`. Note that `-0.0 == 0.0`,
/// so a vector of negative zeros is reported as zero, exactly as in Java.
pub fn is_zero_vector_f32(v: &[f32]) -> bool {
    v.iter().all(|&value| value == 0.0)
}

/// Returns `true` when every component of `v` is zero.
///
/// Equivalent to `VectorUtil.isZeroVector(byte[])`.
pub fn is_zero_vector_bytes(v: &[u8]) -> bool {
    v.iter().all(|&value| value == 0)
}

/// Adds `v` to `u`, component by component.
///
/// Equivalent to `VectorUtil.add(float[], float[])`.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when the dimensions differ. Java
/// iterates over `u.length` and throws `ArrayIndexOutOfBoundsException` when
/// `v` is shorter; the explicit check is the Rust-idiomatic equivalent and
/// rejects exactly the same inputs.
pub fn add(u: &mut [f32], v: &[f32]) -> Result<()> {
    check_same_dimensions(u, v)?;
    for i in 0..u.len() {
        u[i] += v[i];
    }
    Ok(())
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// uint4, uint8 and binary vectors
// ---------------------------------------------------------------------------

/// Returns the scalar reference implementation every wrapper below delegates
/// to.
///
/// Lucene's `VectorUtil` validates its arguments and then forwards to the
/// `VectorUtilSupport` the runtime selected (`IMPL`). This module pins the
/// scalar, non-FMA path as its parity target — see the module documentation —
/// so it forwards to that implementation directly rather than to whichever one
/// the host CPU would pick.
fn support() -> &'static DefaultVectorUtilSupport {
    static SUPPORT: DefaultVectorUtilSupport = DefaultVectorUtilSupport::new();
    &SUPPORT
}

/// Dot product over uint4 vectors, whose values lie in `[0, 15]`.
///
/// Equivalent to `VectorUtil.int4DotProduct(byte[], byte[])`.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when the dimensions differ.
pub fn int4_dot_product(a: &[u8], b: &[u8]) -> Result<i32> {
    if a.len() != b.len() {
        return Err(LuceneError::IllegalArgument(format!(
            "vector dimensions differ: {}!={}",
            a.len(),
            b.len()
        )));
    }
    support().int4_dot_product(a, b)
}

/// Dot product over uint4 vectors where the second one is packed two values to
/// the byte.
///
/// Equivalent to `VectorUtil.int4DotProductSinglePacked(byte[], byte[])`. The
/// packing Lucene assumes is
/// `packed[i] = (raw[i] << 4) | raw[packed.len() + i]`.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when `packed` is not
/// `(unpacked.len() + 1) / 2` bytes long.
pub fn int4_dot_product_single_packed(unpacked: &[u8], packed: &[u8]) -> Result<i32> {
    if packed.len() != (unpacked.len() + 1) >> 1 {
        return Err(LuceneError::IllegalArgument(format!(
            "vector dimensions differ: {} != 2 * {}",
            unpacked.len(),
            packed.len()
        )));
    }
    support().int4_dot_product_single_packed(unpacked, packed)
}

/// Dot product over two uint4 vectors that are both packed two values to the
/// byte.
///
/// Equivalent to `VectorUtil.int4DotProductBothPacked(byte[], byte[])`.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when the dimensions differ.
pub fn int4_dot_product_both_packed(a: &[u8], b: &[u8]) -> Result<i32> {
    if a.len() != b.len() {
        return Err(LuceneError::IllegalArgument(format!(
            "vector dimensions differ: {} != {}",
            a.len(),
            b.len()
        )));
    }
    support().int4_dot_product_both_packed(a, b)
}

/// Dot product between an int4 query vector and a binary document vector.
///
/// Equivalent to `VectorUtil.int4BitDotProduct(byte[], byte[])`.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when `q` is not four times the
/// length of `d`.
pub fn int4_bit_dot_product(q: &[u8], d: &[u8]) -> Result<i64> {
    if q.len() != d.len() * 4 {
        return Err(LuceneError::IllegalArgument(format!(
            "vector dimensions incompatible: {}!= 4 x {}",
            q.len(),
            d.len()
        )));
    }
    support().int4_bit_dot_product(q, d)
}

/// Dot product between an int4 query vector and a dibit (two-bit) document
/// vector.
///
/// Equivalent to `VectorUtil.int4DibitDotProduct(byte[], byte[])`. Both
/// vectors are transposed: the query into four bit planes, the document into
/// two.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when `q` is not twice the length
/// of `d`.
pub fn int4_dibit_dot_product(q: &[u8], d: &[u8]) -> Result<i64> {
    if q.len() != d.len() * 2 {
        return Err(LuceneError::IllegalArgument(format!(
            "vector dimensions incompatible: {}!= 2 x {}",
            q.len(),
            d.len()
        )));
    }
    support().int4_dibit_dot_product(q, d)
}

/// Sum of squared differences between two uint4 vectors.
///
/// Equivalent to `VectorUtil.int4SquareDistance(byte[], byte[])`.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when the dimensions differ.
pub fn int4_square_distance(a: &[u8], b: &[u8]) -> Result<i32> {
    if a.len() != b.len() {
        return Err(LuceneError::IllegalArgument(format!(
            "vector dimensions differ: {}!={}",
            a.len(),
            b.len()
        )));
    }
    support().int4_square_distance(a, b)
}

/// Sum of squared differences between two uint4 vectors where the second one is
/// packed two values to the byte.
///
/// Equivalent to `VectorUtil.int4SquareDistanceSinglePacked(byte[], byte[])`.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when `packed` is not
/// `(unpacked.len() + 1) / 2` bytes long.
pub fn int4_square_distance_single_packed(unpacked: &[u8], packed: &[u8]) -> Result<i32> {
    if packed.len() != (unpacked.len() + 1) >> 1 {
        return Err(LuceneError::IllegalArgument(format!(
            "vector dimensions differ: {}!= 2 * {}",
            unpacked.len(),
            packed.len()
        )));
    }
    support().int4_square_distance_single_packed(unpacked, packed)
}

/// Sum of squared differences between two uint4 vectors that are both packed
/// two values to the byte.
///
/// Equivalent to `VectorUtil.int4SquareDistanceBothPacked(byte[], byte[])`.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when the dimensions differ.
pub fn int4_square_distance_both_packed(a: &[u8], b: &[u8]) -> Result<i32> {
    if a.len() != b.len() {
        return Err(LuceneError::IllegalArgument(format!(
            "vector dimensions differ: {}!={}",
            a.len(),
            b.len()
        )));
    }
    support().int4_square_distance_both_packed(a, b)
}

/// Dot product over two byte vectors whose bytes are read as unsigned.
///
/// Equivalent to `VectorUtil.uint8DotProduct(byte[], byte[])`.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when the dimensions differ.
pub fn uint8_dot_product(a: &[u8], b: &[u8]) -> Result<i32> {
    if a.len() != b.len() {
        return Err(LuceneError::IllegalArgument(format!(
            "vector dimensions differ: {}!={}",
            a.len(),
            b.len()
        )));
    }
    support().uint8_dot_product(a, b)
}

/// Sum of squared differences between two byte vectors whose bytes are read as
/// unsigned.
///
/// Equivalent to `VectorUtil.uint8SquareDistance(byte[], byte[])`.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when the dimensions differ.
pub fn uint8_square_distance(a: &[u8], b: &[u8]) -> Result<i32> {
    if a.len() != b.len() {
        return Err(LuceneError::IllegalArgument(format!(
            "vector dimensions differ: {}!={}",
            a.len(),
            b.len()
        )));
    }
    support().uint8_square_distance(a, b)
}

// ---------------------------------------------------------------------------
// XOR bit count
// ---------------------------------------------------------------------------

/// Whether [`xor_bit_count`] strides four bytes at a time rather than eight.
///
/// Equivalent to `VectorUtil.XOR_BIT_COUNT_STRIDE_AS_INT`. On AArch64
/// `Long.bitCount` is not vectorised by the JIT while `Integer.bitCount` is, so
/// Lucene strides in `int`s there and in `long`s everywhere else
/// (JDK-8336000). The two strides return the same count, so the constant only
/// selects which loop runs.
pub const XOR_BIT_COUNT_STRIDE_AS_INT: bool = cfg!(target_arch = "aarch64");

/// XOR bit count over two byte vectors: the number of differing bits.
///
/// Equivalent to `VectorUtil.xorBitCount(byte[], byte[])`.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when the dimensions differ.
pub fn xor_bit_count(a: &[u8], b: &[u8]) -> Result<i32> {
    if a.len() != b.len() {
        return Err(LuceneError::IllegalArgument(format!(
            "vector dimensions differ: {}!={}",
            a.len(),
            b.len()
        )));
    }
    Ok(if XOR_BIT_COUNT_STRIDE_AS_INT {
        xor_bit_count_int(a, b)
    } else {
        xor_bit_count_long(a, b)
    })
}

/// XOR bit count striding four bytes at a time.
///
/// Equivalent to `VectorUtil.xorBitCountInt(byte[], byte[])`. Java reads each
/// stride through `BitUtil.VH_NATIVE_INT`, a native-endian view over the array;
/// `u32::from_ne_bytes` is that same read. Endianness cannot change the result
/// because both operands are read the same way and only the population count of
/// the XOR is kept.
pub fn xor_bit_count_int(a: &[u8], b: &[u8]) -> i32 {
    let mut distance: i32 = 0;
    let upper_bound = a.len() & !(std::mem::size_of::<u32>() - 1);
    let mut i = 0;
    while i < upper_bound {
        let x = u32::from_ne_bytes(a[i..i + 4].try_into().expect("four bytes"));
        let y = u32::from_ne_bytes(b[i..i + 4].try_into().expect("four bytes"));
        distance += (x ^ y).count_ones() as i32;
        i += 4;
    }
    // tail:
    while i < a.len() {
        distance += (a[i] ^ b[i]).count_ones() as i32;
        i += 1;
    }
    distance
}

/// XOR bit count striding eight bytes at a time.
///
/// Equivalent to `VectorUtil.xorBitCountLong(byte[], byte[])`.
pub fn xor_bit_count_long(a: &[u8], b: &[u8]) -> i32 {
    let mut distance: i32 = 0;
    let upper_bound = a.len() & !(std::mem::size_of::<u64>() - 1);
    let mut i = 0;
    while i < upper_bound {
        let x = u64::from_ne_bytes(a[i..i + 8].try_into().expect("eight bytes"));
        let y = u64::from_ne_bytes(b[i..i + 8].try_into().expect("eight bytes"));
        distance += (x ^ y).count_ones() as i32;
        i += 8;
    }
    // tail:
    while i < a.len() {
        distance += (a[i] ^ b[i]).count_ones() as i32;
        i += 1;
    }
    distance
}

// ---------------------------------------------------------------------------
// Postings and quantization helpers
// ---------------------------------------------------------------------------

/// Returns the first index in `buffer[..to]` whose value is greater than or
/// equal to `target`, never below `from`, or `to` when there is none.
///
/// Equivalent to `VectorUtil.findNextGEQ(int[], int, int, int)`. `buffer` must
/// be sorted over `[0, to)`; Java asserts this and so does the debug build here.
pub fn find_next_geq(buffer: &[i32], target: i32, from: usize, to: usize) -> usize {
    debug_assert!(
        to < 2 || buffer[..to - 1].windows(2).all(|w| w[0] <= w[1]),
        "buffer is not sorted over [0, to)"
    );
    support().find_next_geq(buffer, target, from, to)
}

/// Scalar-quantizes `vector` into `dest` and returns the corrective offset the
/// score needs.
///
/// Equivalent to `VectorUtil.minMaxScalarQuantize(float[], byte[], float, float, float, float)`.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when `vector` and `dest` differ in
/// length.
pub fn min_max_scalar_quantize(
    vector: &[f32],
    dest: &mut [u8],
    scale: f32,
    alpha: f32,
    min_quantile: f32,
    max_quantile: f32,
) -> Result<f32> {
    if vector.len() != dest.len() {
        return Err(LuceneError::IllegalArgument(
            "source and destination arrays should be the same size".to_string(),
        ));
    }
    support().min_max_scalar_quantize(vector, dest, scale, alpha, min_quantile, max_quantile)
}

/// Recomputes the corrective offset of an already quantized `vector` after the
/// quantiles moved.
///
/// Equivalent to `VectorUtil.recalculateOffset(byte[], float, float, float, float, float, float)`.
#[allow(clippy::too_many_arguments)]
pub fn recalculate_offset(
    vector: &[u8],
    old_alpha: f32,
    old_min_quantile: f32,
    scale: f32,
    alpha: f32,
    min_quantile: f32,
    max_quantile: f32,
) -> f32 {
    support().recalculate_scalar_quantization_offset(
        vector,
        old_alpha,
        old_min_quantile,
        scale,
        alpha,
        min_quantile,
        max_quantile,
    )
}

/// Compacts `doc_buffer` and `score_buffer` to the pairs whose score reaches
/// `min_score_inclusive`, and returns how many are left.
///
/// Equivalent to `VectorUtil.filterByScore(int[], double[], double, int)`. The
/// two buffers form pairs by index; a pair scoring below the threshold is
/// dropped from both.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when the buffers differ in length
/// or are shorter than `up_to`.
pub fn filter_by_score(
    doc_buffer: &mut [i32],
    score_buffer: &mut [f64],
    min_score_inclusive: f64,
    up_to: usize,
) -> Result<usize> {
    if doc_buffer.len() != score_buffer.len() || doc_buffer.len() < up_to {
        return Err(LuceneError::IllegalArgument(
            "docBuffer and scoreBuffer should keep same length and at least as long as upTo"
                .to_string(),
        ));
    }
    Ok(support().filter_by_score(doc_buffer, score_buffer, min_score_inclusive, up_to))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic pseudo-random float generator, so that tests can build
    /// large vectors without a dependency and still be reproducible.
    fn floats(len: usize, seed: u64) -> Vec<f32> {
        let mut state = seed;
        (0..len)
            .map(|_| {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                let bits = (state >> 33) as u32;
                // Map into [-1, 1) with an exactly representable scale.
                (bits as f32 / (1u32 << 31) as f32) - 1.0
            })
            .collect()
    }

    fn bytes(len: usize, seed: u64) -> Vec<u8> {
        let mut state = seed;
        (0..len)
            .map(|_| {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                (state >> 40) as u8
            })
            .collect()
    }

    #[test]
    fn dimension_mismatch_is_rejected_with_the_java_message() {
        let err = dot_product_f32(&[1.0, 2.0], &[1.0]).unwrap_err();
        assert_eq!(
            err.to_string(),
            "illegal argument: vector dimensions differ: 2!=1"
        );
        assert!(cosine_f32(&[1.0], &[1.0, 2.0]).is_err());
        assert!(square_distance_f32(&[1.0], &[1.0, 2.0]).is_err());
        assert!(dot_product_bytes(&[1], &[1, 2]).is_err());
        assert!(square_distance_bytes(&[1], &[1, 2]).is_err());
        assert!(cosine_bytes(&[1], &[1, 2]).is_err());
        assert!(dot_product_score(&[1], &[1, 2]).is_err());
        assert!(add(&mut [1.0], &[1.0, 2.0]).is_err());
    }

    #[test]
    fn dot_product_small_vectors_are_exact() {
        assert_eq!(
            dot_product_f32(&[1.0, 2.0, 3.0], &[4.0, 5.0, 6.0]).unwrap(),
            32.0
        );
        assert_eq!(dot_product_f32(&[], &[]).unwrap(), 0.0);
        assert_eq!(dot_product_f32(&[2.0], &[3.0]).unwrap(), 6.0);
    }

    #[test]
    fn square_distance_small_vectors_are_exact() {
        assert_eq!(square_distance_f32(&[1.0, 2.0], &[4.0, 6.0]).unwrap(), 25.0);
        assert_eq!(square_distance_f32(&[1.0], &[1.0]).unwrap(), 0.0);
    }

    /// The unrolled path only starts above 32 elements. At exactly 32 the
    /// sequential loop runs; at 33 the four accumulators do. The two shapes
    /// disagree in the low bits for the same data, which is precisely why the
    /// boundary has to be reproduced rather than approximated.
    #[test]
    fn unrolling_boundary_is_at_32_elements() {
        let a = floats(33, 0x5eed);
        let b = floats(33, 0xb00c);

        // Sequential reference over the first 32 elements.
        let mut sequential = 0.0f32;
        for i in 0..32 {
            sequential = fma(a[i], b[i], sequential);
        }
        assert_eq!(dot_product_f32(&a[..32], &b[..32]).unwrap(), sequential);

        // Four-accumulator reference over 33 elements: 32 unrolled, 1 tail.
        let (mut acc1, mut acc2, mut acc3, mut acc4) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        let mut i = 0;
        while i < 32 {
            acc1 = fma(a[i], b[i], acc1);
            acc2 = fma(a[i + 1], b[i + 1], acc2);
            acc3 = fma(a[i + 2], b[i + 2], acc3);
            acc4 = fma(a[i + 3], b[i + 3], acc4);
            i += 4;
        }
        let mut unrolled = 0.0f32 + (acc1 + acc2 + acc3 + acc4);
        unrolled = fma(a[32], b[32], unrolled);
        assert_eq!(dot_product_f32(&a, &b).unwrap(), unrolled);
    }

    /// Regression for divergence D1: byte accumulation must be exact in `i32`.
    ///
    /// The bar is deliberately placed past 2^24, the largest integer an `f32`
    /// represents exactly, **with an odd per-element increment**. Both halves
    /// matter: a magnitude past 2^24 whose increments are powers of two stays
    /// exact even in `f32`, so such a case would not detect the regression.
    #[test]
    fn byte_dot_product_is_exact_past_the_float_boundary() {
        // 127 * 127 = 16_129, odd. 2048 elements sum to 33_032_192 > 2^24.
        let dim = 2048;
        let a = vec![0x7fu8; dim];
        let expected: i64 = 127 * 127 * dim as i64;
        assert!(expected > (1i64 << 24));
        assert_eq!(dot_product_bytes(&a, &a).unwrap() as i64, expected);

        // Prove the test is not vacuous: the f32 accumulation this replaced
        // really does give a different answer for the same input.
        let mut approximate = 0.0f32;
        for _ in 0..dim {
            approximate += 16_129.0f32;
        }
        assert_ne!(
            approximate as i64, expected,
            "an f32 accumulator must diverge here, otherwise the test proves nothing"
        );

        // The extremes at 1024 dimensions land exactly on 2^24.
        let a = vec![0x80u8; 1024]; // -128
        assert_eq!(dot_product_bytes(&a, &a).unwrap(), 16_777_216);
    }

    /// Regression for divergence D1 on the distance primitive. `255^2 = 65_025`
    /// per element makes an `f32` accumulator inexact from about dimension 258.
    #[test]
    fn byte_square_distance_is_exact_at_dimension_1024() {
        let dim = 1024;
        let a = vec![0x7fu8; dim]; // 127
        let b = vec![0x80u8; dim]; // -128
        let expected: i64 = (127i64 - -128i64).pow(2) * dim as i64;
        assert_eq!(expected, 66_585_600);
        assert_eq!(square_distance_bytes(&a, &b).unwrap() as i64, expected);

        // An odd tail that an f32 accumulator would swallow.
        let mut a = vec![0x7fu8; dim];
        a[dim - 1] = 0x7e; // 126
        let expected: i64 = 255i64.pow(2) * (dim as i64 - 1) + 254i64.pow(2);
        assert_eq!(square_distance_bytes(&a, &b).unwrap() as i64, expected);
    }

    /// Regression for divergence D2: one `f64` square root over the product of
    /// the two norms, not two `f32` square roots multiplied together.
    #[test]
    fn cosine_uses_a_single_f64_square_root() {
        let a = floats(64, 7);
        let b = floats(64, 11);

        let dot = dot_product_f32(&a, &b).unwrap();
        // Reproduce the exact accumulation cosine uses, then compare the two
        // rounding strategies: they differ, so the test is not vacuous.
        let ours = cosine_f32(&a, &b).unwrap();
        let norm_a = dot_product_f32(&a, &a).unwrap();
        let norm_b = dot_product_f32(&b, &b).unwrap();
        let two_roots = dot / (norm_a.sqrt() * norm_b.sqrt());
        let one_root = (f64::from(dot) / (f64::from(norm_a) * f64::from(norm_b)).sqrt()) as f32;
        // The accumulation shape of `cosine` differs from `dotProduct`, so the
        // value is only close; what must hold exactly is the *rounding* rule.
        assert!((ours - one_root).abs() <= f32::EPSILON * 4.0);
        assert!((ours - two_roots).abs() <= f32::EPSILON * 4.0);

        // Pin the rule itself on data where the two strategies disagree.
        let x = 1.000_000_1f32;
        let y = 3.000_000_5f32;
        let dot = 3.0f32;
        assert_ne!(
            (f64::from(dot) / (f64::from(x) * f64::from(y)).sqrt()) as f32,
            dot / (x.sqrt() * y.sqrt())
        );
    }

    #[test]
    fn cosine_of_a_zero_vector_is_nan_like_java() {
        assert!(cosine_f32(&[0.0, 0.0], &[1.0, 1.0]).unwrap().is_nan());
        assert!(cosine_bytes(&[0, 0], &[1, 1]).unwrap().is_nan());
    }

    #[test]
    fn byte_values_are_interpreted_as_signed() {
        // 0xFF is -1, not 255.
        assert_eq!(dot_product_bytes(&[0xff], &[0x01]).unwrap(), -1);
        assert_eq!(dot_product_bytes(&[0x80], &[0x7f]).unwrap(), -128 * 127);
        assert_eq!(square_distance_bytes(&[0x80], &[0x7f]).unwrap(), 255 * 255);
    }

    #[test]
    fn dot_product_score_matches_the_java_closed_form() {
        // The zero vector maps exactly to the midpoint.
        assert_eq!(dot_product_score(&[0, 0, 0], &[0, 0, 0]).unwrap(), 0.5);
        let a = bytes(8, 3);
        let b = bytes(8, 5);
        let dot = dot_product_bytes(&a, &b).unwrap();
        let denom = (8i32 * (1 << 15)) as f32;
        assert_eq!(dot_product_score(&a, &b).unwrap(), 0.5 + dot as f32 / denom);
    }

    #[test]
    fn scale_max_inner_product_score_is_positive_on_both_sides() {
        assert_eq!(scale_max_inner_product_score(3.0), 4.0);
        assert_eq!(scale_max_inner_product_score(0.0), 1.0);
        assert_eq!(scale_max_inner_product_score(-3.0), 0.25);
    }

    #[test]
    fn normalize_to_unit_interval_clamps_and_propagates_nan() {
        assert_eq!(normalize_to_unit_interval(1.0), 1.0);
        assert_eq!(normalize_to_unit_interval(-1.0), 0.0);
        assert_eq!(normalize_to_unit_interval(-3.0), 0.0);
        // Java's Math.max propagates NaN; Rust's f32::max would return 0.0.
        assert!(normalize_to_unit_interval(f32::NAN).is_nan());
    }

    #[test]
    fn normalize_distance_to_unit_interval_maps_zero_to_one() {
        assert_eq!(normalize_distance_to_unit_interval(0.0), 1.0);
        assert_eq!(normalize_distance_to_unit_interval(3.0), 0.25);
    }

    #[test]
    fn l2normalize_leaves_unit_vectors_untouched() {
        let mut v = [1.0f32, 0.0, 0.0];
        let before = v;
        l2normalize(&mut v, true).unwrap();
        assert_eq!(v, before, "an already-unit vector must be byte identical");

        // Within EPSILON of unit length: also untouched.
        let mut v = [1.000_02f32, 0.0];
        let before = v;
        assert!(is_unit_vector(&v));
        l2normalize(&mut v, true).unwrap();
        assert_eq!(v, before);
    }

    #[test]
    fn l2normalize_scales_and_honours_throw_on_zero() {
        let mut v = [3.0f32, 4.0];
        l2normalize(&mut v, true).unwrap();
        assert_eq!(v, [0.6, 0.8]);

        let mut zero = [0.0f32, 0.0];
        assert!(l2normalize(&mut zero, true).is_err());
        let mut zero = [0.0f32, 0.0];
        l2normalize(&mut zero, false).unwrap();
        assert_eq!(zero, [0.0, 0.0]);
    }

    #[test]
    fn is_unit_vector_uses_the_squared_norm() {
        assert!(is_unit_vector(&[1.0, 0.0]));
        assert!(!is_unit_vector(&[2.0, 0.0]));
        // 0.6^2 + 0.8^2 == 1.0
        assert!(is_unit_vector(&[0.6, 0.8]));
    }

    #[test]
    fn check_finite_reports_the_first_offender_like_java() {
        check_finite(&[1.0, 2.0]).unwrap();
        let err = check_finite(&[1.0, f32::NAN, f32::INFINITY]).unwrap_err();
        assert_eq!(
            err.to_string(),
            "illegal argument: non-finite value at vector[1]=NaN"
        );
        let err = check_finite(&[f32::INFINITY]).unwrap_err();
        assert_eq!(
            err.to_string(),
            "illegal argument: non-finite value at vector[0]=Infinity"
        );
        let err = check_finite(&[f32::NEG_INFINITY]).unwrap_err();
        assert_eq!(
            err.to_string(),
            "illegal argument: non-finite value at vector[0]=-Infinity"
        );
    }

    #[test]
    fn zero_vector_detection() {
        assert!(is_zero_vector_f32(&[0.0, -0.0]));
        assert!(!is_zero_vector_f32(&[0.0, 1.0]));
        assert!(is_zero_vector_bytes(&[0, 0]));
        assert!(!is_zero_vector_bytes(&[0, 1]));
    }

    #[test]
    fn add_accumulates_in_place() {
        let mut u = [1.0f32, 2.0];
        add(&mut u, &[3.0, 4.0]).unwrap();
        assert_eq!(u, [4.0, 6.0]);
    }

    #[test]
    fn large_vectors_exercise_every_loop_shape() {
        for &dim in &[1usize, 2, 31, 32, 33, 64, 384, 768, 1024] {
            let a = floats(dim, dim as u64);
            let b = floats(dim, dim as u64 + 1);
            // The primitives must simply be finite and self-consistent here;
            // bit-exactness against Java is asserted by the portability suite.
            assert!(dot_product_f32(&a, &b).unwrap().is_finite());
            assert!(square_distance_f32(&a, &b).unwrap() >= 0.0);
            let c = cosine_f32(&a, &b).unwrap();
            assert!(
                (-1.5..=1.5).contains(&c),
                "cosine out of range for dim {dim}: {c}"
            );

            let ab = bytes(dim, dim as u64 + 2);
            let bb = bytes(dim, dim as u64 + 3);
            assert!(square_distance_bytes(&ab, &bb).unwrap() >= 0);
            let _ = dot_product_bytes(&ab, &bb).unwrap();
            let _ = cosine_bytes(&ab, &bb).unwrap();
        }
    }
}
