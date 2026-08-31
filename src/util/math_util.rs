//! Math helpers ported from `org.apache.lucene.util.MathUtil`.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`MathUtil`] | `MathUtil` |

#![deny(unsafe_code)]

use crate::error::{LuceneError, Result};

/// Static math utility methods.
///
/// Port of `org.apache.lucene.util.MathUtil`.
pub struct MathUtil;

impl MathUtil {
    /// Returns `if x <= 0 { 0 } else { floor(log(x) / log(base)) }`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `base <= 1`, which is
    /// Java's `IllegalArgumentException`.
    pub fn log(x: i64, base: i32) -> Result<i32> {
        if base == 2 {
            // This specialised branch is 30x faster in Java, and free here.
            return Ok(if x <= 0 {
                0
            } else {
                63 - x.leading_zeros() as i32
            });
        } else if base <= 1 {
            return Err(LuceneError::IllegalArgument("base must be > 1".to_string()));
        }
        let mut x = x;
        let mut ret = 0i32;
        while x >= base as i64 {
            x /= base as i64;
            ret += 1;
        }
        Ok(ret)
    }

    /// Calculates the logarithm of `x` in the given base with doubles.
    ///
    /// Equivalent to `MathUtil.log(double, double)`.
    pub fn log_base(base: f64, x: f64) -> f64 {
        x.ln() / base.ln()
    }

    /// Returns the greatest common divisor of `a` and `b`, consistently with
    /// `java.math.BigInteger.gcd`.
    ///
    /// A greatest common divisor must be positive, but `2^64` is not
    /// representable as an `i64` even though it is the GCD of [`i64::MIN`] and
    /// `0`, and of [`i64::MIN`] with itself. In those two cases — and only
    /// those — this returns [`i64::MIN`], exactly as Lucene does.
    pub fn gcd(a: i64, b: i64) -> i64 {
        // Java's `Math.abs(Long.MIN_VALUE)` is `Long.MIN_VALUE`.
        let mut a = a.wrapping_abs();
        let mut b = b.wrapping_abs();
        if a == 0 {
            return b;
        } else if b == 0 {
            return a;
        }
        let common_trailing_zeros = (a | b).trailing_zeros();
        a = ((a as u64) >> a.trailing_zeros()) as i64;
        loop {
            b = ((b as u64) >> b.trailing_zeros()) as i64;
            if a == b {
                break;
            } else if a > b || a == i64::MIN {
                // `Long.MIN_VALUE` is treated as 2^64.
                std::mem::swap(&mut a, &mut b);
            }
            if a == 1 {
                break;
            }
            b = b.wrapping_sub(a);
        }
        ((a as u64) << common_trailing_zeros) as i64
    }

    /// Calculates the inverse hyperbolic sine of `a`.
    ///
    /// Special cases: NaN yields NaN; a zero yields a zero with the same sign;
    /// an infinity yields an infinity with the same sign.
    pub fn asinh(a: f64) -> f64 {
        // Check the sign bit of the raw representation to handle `-0`.
        let (a, sign) = if (a.to_bits() as i64) < 0 {
            (a.abs(), -1.0f64)
        } else {
            (a, 1.0f64)
        };
        sign * ((a * a + 1.0).sqrt() + a).ln()
    }

    /// Calculates the inverse hyperbolic cosine of `a`.
    ///
    /// Special cases: NaN yields NaN; `+1` yields a zero; positive infinity
    /// yields positive infinity; anything below 1 yields NaN.
    pub fn acosh(a: f64) -> f64 {
        ((a * a - 1.0).sqrt() + a).ln()
    }

    /// Calculates the inverse hyperbolic tangent of `a`.
    ///
    /// Special cases: NaN yields NaN; a zero yields a zero with the same sign;
    /// `+1` yields positive infinity; `-1` yields negative infinity; an
    /// absolute value above 1 yields NaN.
    pub fn atanh(a: f64) -> f64 {
        // Check the sign bit of the raw representation to handle `-0`.
        let (a, mult) = if (a.to_bits() as i64) < 0 {
            (a.abs(), -0.5f64)
        } else {
            (a, 0.5f64)
        };
        mult * ((1.0 + a) / (1.0 - a)).ln()
    }

    /// Returns a relative error bound for a sum of `num_values` positive
    /// doubles computed by recursive summation.
    ///
    /// This only holds when all values are positive, so that `Σ|xi| == |Σxi|`.
    /// Uses formula 3.5 from Higham, Nicholas J. (1993), "The accuracy of
    /// floating point summation", SIAM Journal on Scientific Computing.
    pub fn sum_relative_error_bound(num_values: i32) -> f64 {
        if num_values <= 1 {
            return 0.0;
        }
        // `u` is the unit roundoff, also called machine precision or machine
        // epsilon: `Math.scalb(1.0, -52)`.
        let u = f64::from_bits(0x3CB0_0000_0000_0000);
        (num_values - 1) as f64 * u
    }

    /// Returns the maximum possible sum across `num_values` non-negative
    /// doubles, assuming one summation order yielded `sum`.
    pub fn sum_upper_bound(sum: f64, num_values: i32) -> f64 {
        if num_values <= 2 {
            // With only two clauses the sum is the same whatever the order.
            return sum;
        }

        // The error of a sum depends on the order in which the values are added
        // up. To avoid that, compute an upper bound of the value the sum may
        // take: if the maximum relative error is `b`, two sums are always
        // within `2 * b` of each other.
        let b = Self::sum_relative_error_bound(num_values);
        (1.0 + 2.0 * b) * sum
    }

    /// Returns the minimum of two integers compared as unsigned.
    pub fn unsigned_min(a: i32, b: i32) -> i32 {
        if (a as u32) < (b as u32) {
            a
        } else {
            b
        }
    }
}
