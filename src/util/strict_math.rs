//! The `java.lang.StrictMath` functions `org.apache.lucene.util.SloppyMath`
//! seeds its lookup tables from.
//!
//! | Rucene | JDK |
//! | --- | --- |
//! | [`StrictMath`] | `java.lang.StrictMath` (fdlibm 5.3) |
//!
//! `StrictMath` pins its results to the fdlibm algorithms, so they are the same
//! on every JVM. The functions here are a Rust translation of the fdlibm C
//! sources (Sun Microsystems permissive licence, whose notice the translated
//! sources keep).

#![deny(unsafe_code)]

/// The fdlibm functions `SloppyMath` uses.
///
/// Equivalent to the matching static methods of `java.lang.StrictMath`.
pub struct StrictMath;

impl StrictMath {
    /// The sine of `x`, from fdlibm `s_sin.c` (with `k_sin.c`, `k_cos.c` and
    /// `e_rem_pio2.c`).
    ///
    /// Equivalent to `java.lang.StrictMath.sin(double)`.
    pub fn sin(x: f64) -> f64 {
        let _ = x;
        todo!("PORT G-032")
    }

    /// The cosine of `x`, from fdlibm `s_cos.c` (with `k_sin.c`, `k_cos.c`
    /// and `e_rem_pio2.c`).
    ///
    /// Equivalent to `java.lang.StrictMath.cos(double)`.
    pub fn cos(x: f64) -> f64 {
        let _ = x;
        todo!("PORT G-032")
    }

    /// The arc sine of `x`, from fdlibm `e_asin.c`.
    ///
    /// Equivalent to `java.lang.StrictMath.asin(double)`.
    pub fn asin(x: f64) -> f64 {
        let _ = x;
        todo!("PORT G-032")
    }

    /// The correctly rounded square root of `x`, from fdlibm `e_sqrt.c`.
    ///
    /// Equivalent to `java.lang.StrictMath.sqrt(double)`.
    pub fn sqrt(x: f64) -> f64 {
        let _ = x;
        todo!("PORT G-032")
    }
}
