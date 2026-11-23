//! `java.lang.Math` functions with the results of the JDK 25 HotSpot JVM.
//!
//! | Rucene | JDK |
//! | --- | --- |
//! | [`sin`], [`cos`], [`tan`], [`asin`], [`acos`], [`atan`], [`atan2`], [`pow`], [`sqrt`], [`ieee_remainder`], [`exp`], [`tanh`] | `java.lang.Math` |
//!
//! `java.lang.Math` may use any algorithm within 1 ulp; each function here is
//! the matching [`StrictMath`] (fdlibm) function. Measured on JDK 25.0.5
//! (x86_64), over the edge cases and 1,000,000 seeded inputs each
//! (`tests/java_math_oracle.rs`, rmp #628):
//!
//! * `Math.asin`, `acos`, `atan`, `atan2`, `sqrt` and `IEEEremainder` equal
//!   `StrictMath` bit for bit on HotSpot's default settings, in the
//!   interpreter, C1 and C2.
//! * **Divergence.** `Math.sin`, `cos`, `tan` and `pow` on default x86_64
//!   HotSpot use its LIBM intrinsics (`UseLibmIntrinsic`), whose results
//!   differ from `StrictMath` by 1 ulp on about 3 % of inputs (2.6 % to
//!   3.9 %). The port uses fdlibm for them (user decision, rmp #628): it
//!   equals `java.lang.Math` under `-XX:-UseLibmIntrinsic` and on JVMs
//!   without those intrinsics, and is the same on every platform. The Java
//!   oracles of the tests run with `-XX:-UseLibmIntrinsic`.
//! * `Math.exp` and `Math.tanh` (icu4j's LSTM break engine, rmp #647) follow
//!   the same decision: fdlibm `StrictMath`, equal to `java.lang.Math` under
//!   `-XX:-UseLibmIntrinsic`.

use super::strict_math::StrictMath;

/// `Math.sin(double)`: [`StrictMath::sin`] (see the module's divergence).
#[inline]
pub fn sin(x: f64) -> f64 {
    StrictMath::sin(x)
}

/// `Math.cos(double)`: [`StrictMath::cos`] (see the module's divergence).
#[inline]
pub fn cos(x: f64) -> f64 {
    StrictMath::cos(x)
}

/// `Math.tan(double)`: [`StrictMath::tan`] (see the module's divergence).
#[inline]
pub fn tan(x: f64) -> f64 {
    StrictMath::tan(x)
}

/// `Math.pow(double, double)`: [`StrictMath::pow`] (see the module's
/// divergence).
#[inline]
pub fn pow(x: f64, y: f64) -> f64 {
    StrictMath::pow(x, y)
}

/// `Math.asin(double)`: [`StrictMath::asin`].
#[inline]
pub fn asin(x: f64) -> f64 {
    StrictMath::asin(x)
}

/// `Math.acos(double)`: [`StrictMath::acos`].
#[inline]
pub fn acos(x: f64) -> f64 {
    StrictMath::acos(x)
}

/// `Math.atan(double)`: [`StrictMath::atan`].
#[inline]
pub fn atan(x: f64) -> f64 {
    StrictMath::atan(x)
}

/// `Math.atan2(double y, double x)`: [`StrictMath::atan2`].
#[inline]
pub fn atan2(y: f64, x: f64) -> f64 {
    StrictMath::atan2(y, x)
}

/// `Math.sqrt(double)`: the correctly rounded square root, which
/// [`f64::sqrt`] and [`StrictMath::sqrt`] both compute.
#[inline]
pub fn sqrt(x: f64) -> f64 {
    x.sqrt()
}

/// `Math.IEEEremainder(double, double)`: [`StrictMath::ieee_remainder`].
#[inline]
pub fn ieee_remainder(x: f64, p: f64) -> f64 {
    StrictMath::ieee_remainder(x, p)
}

/// `Math.exp(double)`: [`StrictMath::exp`] (see the module's divergence).
#[inline]
pub fn exp(x: f64) -> f64 {
    StrictMath::exp(x)
}

/// `Math.tanh(double)`: [`StrictMath::tanh`] (see the module's divergence).
#[inline]
pub fn tanh(x: f64) -> f64 {
    StrictMath::tanh(x)
}
