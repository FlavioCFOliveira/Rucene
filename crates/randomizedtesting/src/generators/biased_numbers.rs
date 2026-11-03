//! Port of `generators/BiasedNumbers.java` (randomizedtesting 2.8.1).

use super::RandomNumbers;
use crate::jdk::exception::{self, java_assert, java_assert_no_message};
use crate::jdk::lang::{double_to_string, float_to_string};
use crate::jdk::math;
use crate::jdk::random::Random;

const EVIL_RANGE_LEFT: i32 = 1;
const EVIL_RANGE_RIGHT: i32 = 1;
const EVIL_VERY_CLOSE_RANGE_ENDS: i32 = 20;
const EVIL_ZERO_OR_NEAR: i32 = 5;
const EVIL_SIMPLE_PROPORTION: i32 = 10;
const EVIL_RANDOM_REPRESENTATION_BITS: i32 = 10;

/// Numbers drawn at random but not uniformly: "evil" values (exact range
/// ends, values a few ulps from the ends, zero and near-zero values, values
/// uniform over the binary representation) come more often:
/// `BiasedNumbers`.
pub struct BiasedNumbers;

impl BiasedNumbers {
    /// `randomDoubleBetween(r, min, max)`: a biased double in `[min, max]`;
    /// the bounds may be infinite but not `NaN`.
    ///
    /// # Panics
    ///
    /// `AssertionError` when `max < min` or a bound is `NaN` and assertions
    /// are on.
    pub fn random_double_between<R: Random + ?Sized>(r: &R, min: f64, max: f64) -> f64 {
        java_assert(max >= min, || {
            format!(
                "max must be >= min: {}, {}",
                double_to_string(min),
                double_to_string(max)
            )
        });
        java_assert_no_message(!min.is_nan() && !max.is_nan());
        let (mut min, mut max) = (min, max);

        let has_zero = min <= 0.0 && max >= 0.0;
        let mut pick = r.next_int_bound(
            EVIL_RANGE_LEFT
                + EVIL_RANGE_RIGHT
                + EVIL_VERY_CLOSE_RANGE_ENDS
                + if has_zero { EVIL_ZERO_OR_NEAR } else { 0 }
                + EVIL_SIMPLE_PROPORTION
                + EVIL_RANDOM_REPRESENTATION_BITS,
        );

        // Exact range ends.
        pick -= EVIL_RANGE_LEFT;
        if pick < 0 || min == max {
            return min;
        }
        pick -= EVIL_RANGE_RIGHT;
        if pick < 0 {
            return max;
        }

        // Infinities become discrete values.
        java_assert_no_message(min != max);
        if min.is_infinite() {
            min = math::next_up(min);
        }
        if max.is_infinite() {
            max = math::next_after(max, f64::NEG_INFINITY);
        }

        // A few ulps from the range ends.
        pick -= EVIL_VERY_CLOSE_RANGE_ENDS;
        if pick < 0 {
            return if r.next_boolean() {
                BiasedNumbers::fuzz_up(r, min, max)
            } else {
                BiasedNumbers::fuzz_down(r, max, min)
            };
        }

        // Zero or near zero, within the range.
        if has_zero {
            pick -= EVIL_ZERO_OR_NEAR;
            if pick < 0 {
                match r.next_int_bound(4) {
                    0 => return 0.0,
                    1 => return -0.0,
                    2 => return BiasedNumbers::fuzz_down(r, 0.0, min),
                    3 => return BiasedNumbers::fuzz_up(r, 0.0, max),
                    _ => {}
                }
            }
        }

        // Simple proportional selection.
        pick -= EVIL_SIMPLE_PROPORTION;
        if pick < 0 {
            return min + (max - min) * r.next_double();
        }

        // Uniform over the representation, heavily biased to tiny values.
        pick -= EVIL_RANDOM_REPRESENTATION_BITS;
        if pick < 0 {
            let from = to_sortable_f64(min);
            let to = to_sortable_f64(max);
            return from_sortable_f64(RandomNumbers::random_long_between(r, from, to));
        }

        exception::runtime("Unreachable.")
    }

    /// `fuzzDown(r, v, min)`: `v` lowered by 1..=10 ulps, never past `min`.
    ///
    /// # Panics
    ///
    /// `AssertionError` when `v < min` and assertions are on.
    pub fn fuzz_down<R: Random + ?Sized>(r: &R, v: f64, min: f64) -> f64 {
        java_assert_no_message(v >= min);
        let mut v = v;
        let mut steps = RandomNumbers::random_int_between(r, 1, 10);
        while steps > 0 && v > min {
            v = math::next_after(v, f64::NEG_INFINITY);
            steps -= 1;
        }
        v
    }

    /// `fuzzUp(r, v, max)`: `v` raised by 1..=10 ulps, never past `max`.
    ///
    /// # Panics
    ///
    /// `AssertionError` when `v > max` and assertions are on.
    pub fn fuzz_up<R: Random + ?Sized>(r: &R, v: f64, max: f64) -> f64 {
        java_assert_no_message(v <= max);
        let mut v = v;
        let mut steps = RandomNumbers::random_int_between(r, 1, 10);
        while steps > 0 && v < max {
            v = math::next_up(v);
            steps -= 1;
        }
        v
    }

    /// `randomFloatBetween(r, min, max)`: a biased float in `[min, max]`.
    ///
    /// # Panics
    ///
    /// `AssertionError` when `max < min` or a bound is `NaN` and assertions
    /// are on.
    pub fn random_float_between<R: Random + ?Sized>(r: &R, min: f32, max: f32) -> f32 {
        java_assert(max >= min, || {
            format!(
                "max must be >= min: {}, {}",
                float_to_string(min),
                float_to_string(max)
            )
        });
        java_assert_no_message(!min.is_nan() && !max.is_nan());
        let (mut min, mut max) = (min, max);

        let has_zero = min <= 0.0 && max >= 0.0;
        let mut pick = r.next_int_bound(
            EVIL_RANGE_LEFT
                + EVIL_RANGE_RIGHT
                + EVIL_VERY_CLOSE_RANGE_ENDS
                + if has_zero { EVIL_ZERO_OR_NEAR } else { 0 }
                + EVIL_SIMPLE_PROPORTION
                + EVIL_RANDOM_REPRESENTATION_BITS,
        );

        pick -= EVIL_RANGE_LEFT;
        if pick < 0 || min == max {
            return min;
        }
        pick -= EVIL_RANGE_RIGHT;
        if pick < 0 {
            return max;
        }

        java_assert_no_message(min != max);
        if min.is_infinite() {
            min = math::next_up_f32(min);
        }
        if max.is_infinite() {
            max = math::next_after_f32(max, f64::NEG_INFINITY);
        }

        pick -= EVIL_VERY_CLOSE_RANGE_ENDS;
        if pick < 0 {
            return if r.next_boolean() {
                BiasedNumbers::fuzz_up_f32(r, min, max)
            } else {
                BiasedNumbers::fuzz_down_f32(r, max, min)
            };
        }

        if has_zero {
            pick -= EVIL_ZERO_OR_NEAR;
            if pick < 0 {
                match r.next_int_bound(4) {
                    0 => return 0.0,
                    1 => return -0.0,
                    2 => return BiasedNumbers::fuzz_down_f32(r, 0.0, min),
                    3 => return BiasedNumbers::fuzz_up_f32(r, 0.0, max),
                    _ => {}
                }
            }
        }

        pick -= EVIL_SIMPLE_PROPORTION;
        if pick < 0 {
            // `(float) (min + (((double) max - min) * r.nextDouble()))`.
            let (dmin, dmax) = (f64::from(min), f64::from(max));
            return (dmin + ((dmax - dmin) * r.next_double())) as f32;
        }

        pick -= EVIL_RANDOM_REPRESENTATION_BITS;
        if pick < 0 {
            let from = to_sortable_f32(min);
            let to = to_sortable_f32(max);
            return from_sortable_f32(RandomNumbers::random_int_between(r, from, to));
        }

        exception::runtime("Unreachable.")
    }

    /// `fuzzDown(r, float v, float min)`.
    ///
    /// # Panics
    ///
    /// `AssertionError` when `v < min` and assertions are on.
    pub fn fuzz_down_f32<R: Random + ?Sized>(r: &R, v: f32, min: f32) -> f32 {
        java_assert_no_message(v >= min);
        let mut v = v;
        let mut steps = RandomNumbers::random_int_between(r, 1, 10);
        while steps > 0 && v > min {
            v = math::next_after_f32(v, f64::NEG_INFINITY);
            steps -= 1;
        }
        v
    }

    /// `fuzzUp(r, float v, float max)`.
    ///
    /// # Panics
    ///
    /// `AssertionError` when `v > max` and assertions are on.
    pub fn fuzz_up_f32<R: Random + ?Sized>(r: &R, v: f32, max: f32) -> f32 {
        java_assert_no_message(v <= max);
        let mut v = v;
        let mut steps = RandomNumbers::random_int_between(r, 1, 10);
        while steps > 0 && v < max {
            v = math::next_up_f32(v);
            steps -= 1;
        }
        v
    }
}

/// `flip(long)`: `bits ^ (bits >> 63) & 0x7fffffffffffffffL`.
fn flip_i64(bits: i64) -> i64 {
    bits ^ ((bits >> 63) & i64::MAX)
}

/// `flip(int)`.
fn flip_i32(bits: i32) -> i32 {
    bits ^ ((bits >> 31) & i32::MAX)
}

/// `toSortable(double)`: `flip(Double.doubleToLongBits(value))` (`NaN`
/// cannot reach it).
fn to_sortable_f64(value: f64) -> i64 {
    flip_i64(value.to_bits() as i64)
}

/// `fromSortable(long)`: `Double.longBitsToDouble(flip(sortable))`.
fn from_sortable_f64(sortable: i64) -> f64 {
    f64::from_bits(flip_i64(sortable) as u64)
}

/// `toSortable(float)`.
fn to_sortable_f32(value: f32) -> i32 {
    flip_i32(value.to_bits() as i32)
}

/// `fromSortable(int)`.
fn from_sortable_f32(sortable: i32) -> f32 {
    f32::from_bits(flip_i32(sortable) as u32)
}
