//! The `java.lang.Math` primitives whose Rust counterparts differ, reproduced
//! for the similarity formulas that depend on them.
//!
//! Three of them are load-bearing here:
//!
//! * `Math.nextUp` / `Math.nextDown` — [`DistributionSPL`] and the two
//!   [`Lambda`] implementations nudge a value by one ulp to step off a
//!   singularity, and the nudge decides whether the score is finite.
//!   They are the IEEE 754 `nextUp` / `nextDown` that [`f32::next_up`] and
//!   its siblings implement, and delegate to them.
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
    value.next_up()
}

/// Returns the `f32` adjacent to `value` in the direction of negative
/// infinity, as `Math.nextDown(float)` does.
///
/// NaN and negative infinity are returned unchanged; both zeros step to the
/// largest negative subnormal.
pub(crate) fn next_down_f32(value: f32) -> f32 {
    value.next_down()
}

/// Returns the `f64` adjacent to `value` in the direction of positive
/// infinity, as `Math.nextUp(double)` does.
pub(crate) fn next_up_f64(value: f64) -> f64 {
    value.next_up()
}

/// Returns the `f64` adjacent to `value` in the direction of negative
/// infinity, as `Math.nextDown(double)` does.
pub(crate) fn next_down_f64(value: f64) -> f64 {
    value.next_down()
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
