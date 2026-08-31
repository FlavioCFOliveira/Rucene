//! Port-support helpers for `org.apache.lucene.internal.hppc`.
//!
//! This module has no counterpart in Lucene. It exists only because the hppc
//! containers use a handful of `org.apache.lucene.util.ArrayUtil`,
//! `org.apache.lucene.util.RamUsageEstimator` and `java.lang.Float`/
//! `java.lang.Double` operations for element types that [`crate::util`] does
//! not yet expose. Every function here reproduces the exact semantics of the
//! Java operation it stands in for; none of them introduces new behaviour.

use crate::util::{ArrayUtil, RamUsageEstimator};

// ---------------------------------------------------------------------------
// java.lang.Float / java.lang.Double bit conversions
// ---------------------------------------------------------------------------

/// Equivalent of `java.lang.Float.floatToIntBits`.
///
/// Unlike [`f32::to_bits`] (which matches `floatToRawIntBits`), Java collapses
/// every NaN payload to the canonical quiet NaN `0x7fc00000`. The hppc float
/// containers depend on that collapsing: it is what makes NaN compare equal to
/// itself in `IntFloatHashMap.equalElements`.
#[inline]
pub(crate) fn float_to_int_bits(value: f32) -> i32 {
    if value.is_nan() {
        0x7fc0_0000_u32 as i32
    } else {
        value.to_bits() as i32
    }
}

/// Equivalent of `java.lang.Double.doubleToLongBits`.
///
/// Collapses every NaN payload to the canonical quiet NaN
/// `0x7ff8000000000000`, for the reason given on [`float_to_int_bits`].
#[inline]
pub(crate) fn double_to_long_bits(value: f64) -> i64 {
    if value.is_nan() {
        0x7ff8_0000_0000_0000_u64 as i64
    } else {
        value.to_bits() as i64
    }
}

/// Equivalent of `java.lang.Float.compare`, i.e. the total order used by
/// `java.util.Arrays.sort(float[])`.
///
/// It differs from [`f32::total_cmp`] on negatively-signed NaNs: Java canonicalises
/// every NaN to `0x7fc00000`, so all NaNs are equal to one another and greater
/// than `+Infinity`, whereas `total_cmp` orders a sign-negative NaN below
/// `-Infinity`.
#[inline]
pub(crate) fn java_float_compare(a: f32, b: f32) -> std::cmp::Ordering {
    if a < b {
        return std::cmp::Ordering::Less;
    }
    if a > b {
        return std::cmp::Ordering::Greater;
    }
    float_to_int_bits(a).cmp(&float_to_int_bits(b))
}

// ---------------------------------------------------------------------------
// org.apache.lucene.util.RamUsageEstimator array sizing
// ---------------------------------------------------------------------------

/// Equivalent of `RamUsageEstimator.sizeOf(char[])`.
#[inline]
pub(crate) fn size_of_char_array(len: usize) -> i64 {
    RamUsageEstimator::align_object_size(RamUsageEstimator::NUM_BYTES_ARRAY_HEADER + 2 * len as i64)
}

/// Equivalent of `RamUsageEstimator.sizeOf(int[])`.
#[inline]
pub(crate) fn size_of_int_array(len: usize) -> i64 {
    RamUsageEstimator::align_object_size(RamUsageEstimator::NUM_BYTES_ARRAY_HEADER + 4 * len as i64)
}

/// Equivalent of `RamUsageEstimator.sizeOf(long[])`.
#[inline]
pub(crate) fn size_of_long_array(len: usize) -> i64 {
    RamUsageEstimator::align_object_size(RamUsageEstimator::NUM_BYTES_ARRAY_HEADER + 8 * len as i64)
}

/// Equivalent of `RamUsageEstimator.sizeOf(float[])`.
#[inline]
pub(crate) fn size_of_float_array(len: usize) -> i64 {
    size_of_int_array(len)
}

/// Equivalent of `RamUsageEstimator.sizeOf(double[])`.
#[inline]
pub(crate) fn size_of_double_array(len: usize) -> i64 {
    size_of_long_array(len)
}

/// Equivalent of `RamUsageEstimator.shallowSizeOf(Object[])`.
#[inline]
pub(crate) fn shallow_size_of_object_array(len: usize) -> i64 {
    RamUsageEstimator::align_object_size(
        RamUsageEstimator::NUM_BYTES_ARRAY_HEADER
            + RamUsageEstimator::NUM_BYTES_OBJECT_REF * len as i64,
    )
}

// ---------------------------------------------------------------------------
// org.apache.lucene.util.ArrayUtil growth
// ---------------------------------------------------------------------------

/// Equivalent of `ArrayUtil.growInRange(T[], int, int)`.
///
/// Java returns a new array; this port grows `buffer` in place, which has the
/// same observable effect because every hppc caller immediately assigns the
/// result back over its own buffer field. Elements beyond the old length are
/// initialised to `T::default()`, matching the JVM's zero/`null` defaults.
///
/// # Panics
///
/// Panics if `min_length` is greater than `max_length`, as Java's
/// `IllegalArgumentException` does.
pub(crate) fn grow_in_range<T: Copy + Default>(
    buffer: &mut Vec<T>,
    min_length: i32,
    max_length: i32,
    bytes_per_element: usize,
) {
    debug_assert!(
        min_length >= 0,
        "length must be positive (got {min_length}): likely integer overflow?"
    );

    if min_length > max_length {
        panic!(
            "requested minimum array length {min_length} is larger than requested maximum array length {max_length}"
        );
    }

    if buffer.len() as i64 >= min_length as i64 {
        return;
    }

    let potential_length = ArrayUtil::oversize(min_length as usize, bytes_per_element);
    let new_length = std::cmp::min(max_length as usize, potential_length);
    buffer.resize(new_length, T::default());
}

/// Equivalent of `ArrayUtil.grow(T[], int)`, which is
/// `growInRange(array, minSize, Integer.MAX_VALUE)`.
pub(crate) fn grow<T: Copy + Default>(
    buffer: &mut Vec<T>,
    min_length: i32,
    bytes_per_element: usize,
) {
    grow_in_range(buffer, min_length, i32::MAX, bytes_per_element);
}

// ---------------------------------------------------------------------------
// Element arithmetic and equality used by the container templates
// ---------------------------------------------------------------------------

/// Equivalent of Java's `int` addition, which wraps silently on overflow.
#[inline]
pub(crate) fn add_i32(a: i32, b: i32) -> i32 {
    a.wrapping_add(b)
}

/// Equivalent of Java's `long` addition, which wraps silently on overflow.
#[inline]
pub(crate) fn add_i64(a: i64, b: i64) -> i64 {
    a.wrapping_add(b)
}

/// Equivalent of Java's `float` addition.
#[inline]
pub(crate) fn add_f32(a: f32, b: f32) -> f32 {
    a + b
}

/// Equivalent of Java's `double` addition.
#[inline]
pub(crate) fn add_f64(a: f64, b: f64) -> f64 {
    a + b
}

/// Equivalent of Java's `==` on `int`.
#[inline]
pub(crate) fn eq_i32(a: i32, b: i32) -> bool {
    a == b
}

/// Equivalent of Java's `==` on `long`.
#[inline]
pub(crate) fn eq_i64(a: i64, b: i64) -> bool {
    a == b
}

/// Equivalent of Java's `Float.floatToIntBits(a) == Float.floatToIntBits(b)`.
#[inline]
pub(crate) fn eq_f32_bits(a: f32, b: f32) -> bool {
    float_to_int_bits(a) == float_to_int_bits(b)
}

/// Equivalent of Java's `Double.doubleToLongBits(a) == Double.doubleToLongBits(b)`.
#[inline]
pub(crate) fn eq_f64_bits(a: f64, b: f64) -> bool {
    double_to_long_bits(a) == double_to_long_bits(b)
}

/// Equivalent of Java's `==` on `float`, which is *not* a total equality:
/// `NaN != NaN` and `-0.0 == 0.0`.
#[inline]
pub(crate) fn eq_f32(a: f32, b: f32) -> bool {
    a == b
}

// ---------------------------------------------------------------------------
// java.util.Formatter
// ---------------------------------------------------------------------------

/// Equivalent of `String.format(Locale.ROOT, "%,d", value)`.
///
/// Groups the digits of `value` in threes, separated by a comma, which is what
/// the root locale uses. Only needed so that the allocation-failure messages of
/// the hash containers read exactly as Lucene's do.
pub(crate) fn group_digits(value: i64) -> String {
    let negative = value < 0;
    let digits = value.unsigned_abs().to_string();
    let bytes = digits.as_bytes();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 + 1);
    if negative {
        out.push('-');
    }
    for (i, byte) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(*byte as char);
    }
    out
}

// ---------------------------------------------------------------------------
// Object hashing
// ---------------------------------------------------------------------------

/// Derives a 32-bit hash from a value's [`Hash`](std::hash::Hash) implementation.
///
/// Stands in for `Object.hashCode()`, which the object-valued hash maps feed to
/// `BitMixer.mix(Object)` when computing their own hash code. Rust has no
/// universal `hashCode`, so the value's own `Hash` implementation supplies the
/// bits instead. Like Java's `hashCode`, the result only has to be stable for a
/// given program run and consistent with equality.
pub(crate) fn value_hash<V: std::hash::Hash>(value: &V) -> i32 {
    use std::hash::Hasher;

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish() as i32
}

// ---------------------------------------------------------------------------
// java.util.Arrays sorting
// ---------------------------------------------------------------------------

/// Equivalent of `java.util.Arrays.sort(int[], int, int)`.
#[inline]
pub(crate) fn sort_i32(slice: &mut [i32]) {
    slice.sort_unstable();
}

/// Equivalent of `java.util.Arrays.sort(long[], int, int)`.
#[inline]
pub(crate) fn sort_i64(slice: &mut [i64]) {
    slice.sort_unstable();
}

/// Equivalent of `java.util.Arrays.sort(float[], int, int)`, which orders by
/// `Float.compareTo` rather than by `<`.
#[inline]
pub(crate) fn sort_f32(slice: &mut [f32]) {
    slice.sort_unstable_by(|a, b| java_float_compare(*a, *b));
}
