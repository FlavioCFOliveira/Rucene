//! `java.lang.Math` operations whose Java semantics differ from the Rust
//! operations of the same name.

/// `Math.round(double)`: the closest `long`, ties rounding towards positive
/// infinity; `NaN` is `0` and out-of-range values saturate.
pub fn round(a: f64) -> i64 {
    if a.is_nan() {
        return 0;
    }
    let floor = a.floor();
    // `a - floor` is exact: both share `a`'s binade or `a` is integral.
    let r = if a - floor >= 0.5 { floor + 1.0 } else { floor };
    // `as` saturates, as Java's double-to-long conversion does.
    r as i64
}

/// `Math.min(double, double)`: `NaN` if either argument is `NaN`, and
/// `-0.0` is smaller than `0.0` (`f64::min` returns the other argument
/// for `NaN`).
pub fn min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        return f64::NAN;
    }
    if a == 0.0 && b == 0.0 {
        return if a.is_sign_negative() { a } else { b };
    }
    if a <= b {
        a
    } else {
        b
    }
}

/// `Math.nextUp(double)`.
pub fn next_up(d: f64) -> f64 {
    d.next_up()
}

/// `Math.nextUp(float)`.
pub fn next_up_f32(f: f32) -> f32 {
    f.next_up()
}

/// `Math.nextDown(double)`.
pub fn next_down(d: f64) -> f64 {
    d.next_down()
}

/// `Math.nextDown(float)`.
pub fn next_down_f32(f: f32) -> f32 {
    f.next_down()
}

/// `Math.nextAfter(double start, double direction)`: the neighbour of
/// `start` towards `direction`; `direction` itself when they compare equal;
/// `NaN` when either is `NaN`.
pub fn next_after(start: f64, direction: f64) -> f64 {
    if start.is_nan() || direction.is_nan() {
        f64::NAN
    } else if start == direction {
        direction
    } else if direction > start {
        start.next_up()
    } else {
        start.next_down()
    }
}

/// `Math.nextAfter(float start, double direction)`.
pub fn next_after_f32(start: f32, direction: f64) -> f32 {
    let s = f64::from(start);
    if start.is_nan() || direction.is_nan() {
        f32::NAN
    } else if s == direction {
        direction as f32
    } else if direction > s {
        start.next_up()
    } else {
        start.next_down()
    }
}
