//! The `java.lang.Math` primitives whose Rust counterparts differ, reproduced
//! for the similarity formulas that depend on them.
//!
//! Three of them are load-bearing here:
//!
//! * `Math.nextUp` / `Math.nextDown` — [`DistributionSPL`] and the two
//!   [`Lambda`] implementations nudge a value by one ulp to step off a
//!   singularity, and the nudge decides whether the score is finite.
//!   `f32::next_up` / `f64::next_down` were stabilized in Rust 1.86, above this
//!   crate's minimum supported version of 1.80, so they are implemented here.
//! * `Math.max` — [`Axiomatic`] floors its score at zero with it. Java's
//!   `Math.max` propagates NaN; Rust's `f64::max` returns the non-NaN operand
//!   instead, which would silently turn a NaN score into `0.0`.
//!
//! [`DistributionSPL`]: super::DistributionSPL
//! [`Lambda`]: super::Lambda
//! [`Axiomatic`]: super::Axiomatic

#![deny(unsafe_code)]

/// Returns the `f32` adjacent to `value` in the direction of positive
/// infinity, as `Math.nextUp(float)` does.
///
/// NaN and positive infinity are returned unchanged; `-0.0` is treated as
/// `+0.0`, so it steps to the smallest positive subnormal.
pub(crate) fn next_up_f32(value: f32) -> f32 {
    if value.is_nan() || value == f32::INFINITY {
        return value;
    }
    // Adding `0.0` turns `-0.0` into `+0.0`, which is what makes the branch
    // below step away from zero rather than towards it.
    let value = value + 0.0;
    if value >= 0.0 {
        f32::from_bits(value.to_bits().wrapping_add(1))
    } else {
        f32::from_bits(value.to_bits().wrapping_sub(1))
    }
}

/// Returns the `f32` adjacent to `value` in the direction of negative
/// infinity, as `Math.nextDown(float)` does.
///
/// NaN and negative infinity are returned unchanged; both zeros step to the
/// largest negative subnormal.
pub(crate) fn next_down_f32(value: f32) -> f32 {
    if value.is_nan() || value == f32::NEG_INFINITY {
        return value;
    }
    if value == 0.0 {
        return -f32::from_bits(1);
    }
    if value > 0.0 {
        f32::from_bits(value.to_bits().wrapping_sub(1))
    } else {
        f32::from_bits(value.to_bits().wrapping_add(1))
    }
}

/// Returns the `f64` adjacent to `value` in the direction of positive
/// infinity, as `Math.nextUp(double)` does.
pub(crate) fn next_up_f64(value: f64) -> f64 {
    if value.is_nan() || value == f64::INFINITY {
        return value;
    }
    let value = value + 0.0;
    if value >= 0.0 {
        f64::from_bits(value.to_bits().wrapping_add(1))
    } else {
        f64::from_bits(value.to_bits().wrapping_sub(1))
    }
}

/// Returns the `f64` adjacent to `value` in the direction of negative
/// infinity, as `Math.nextDown(double)` does.
pub(crate) fn next_down_f64(value: f64) -> f64 {
    if value.is_nan() || value == f64::NEG_INFINITY {
        return value;
    }
    if value == 0.0 {
        return -f64::from_bits(1);
    }
    if value > 0.0 {
        f64::from_bits(value.to_bits().wrapping_sub(1))
    } else {
        f64::from_bits(value.to_bits().wrapping_add(1))
    }
}

/// Returns the greater of two `f64`s, as `Math.max(double, double)` does.
///
/// Unlike [`f64::max`], NaN is propagated rather than discarded, and `+0.0` is
/// considered greater than `-0.0`.
pub(crate) fn max_f64(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        return f64::NAN;
    }
    if a == 0.0 && b == 0.0 {
        // Distinguishes `+0.0` from `-0.0`, which `==` does not.
        return if a.is_sign_positive() { a } else { b };
    }
    if a > b {
        a
    } else {
        b
    }
}
