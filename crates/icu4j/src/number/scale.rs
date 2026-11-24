// © 2018 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.number.Scale`.

use std::sync::{Arc, LazyLock};

use rucene::error::Result;
use rucene::util::java_collections::java_identity_hash_code_stand_in;

use crate::jdk::math::{BigDecimal, BigInteger, MathContext};
use crate::r#impl::number::decimal_quantity::DecimalQuantity;
use crate::r#impl::number::rounding_utils::DEFAULT_MATH_CONTEXT_34_DIGITS;

/// The fields of a `Scale` object.
#[derive(Debug)]
pub(crate) struct ScaleData {
    /// `magnitude`.
    pub(crate) magnitude: i32,
    /// `arbitrary` (`None` for Java's `null`).
    pub(crate) arbitrary: Option<BigDecimal>,
    /// `reciprocal` (`None` for Java's `null`).
    pub(crate) reciprocal: Option<BigDecimal>,
    /// `mc`.
    pub(crate) mc: MathContext,
}

/// A class that defines a quantity by which a number should be multiplied
/// when formatting.
///
/// To create a Multiplier, use one of the factory methods.
///
/// Equivalent to `com.ibm.icu.number.Scale`. Java's objects are immutable
/// and compared by identity; this handle shares one object
/// ([`ptr_eq`](Self::ptr_eq) is Java's `==` and `equals`).
#[derive(Debug, Clone)]
pub struct Scale(pub(crate) Arc<ScaleData>);

/// `DEFAULT`.
static DEFAULT: LazyLock<Scale> = LazyLock::new(|| Scale::new_simple(0));
/// `HUNDRED`.
static HUNDRED: LazyLock<Scale> = LazyLock::new(|| Scale::new_simple(2));
/// `THOUSAND`.
static THOUSAND: LazyLock<Scale> = LazyLock::new(|| Scale::new_simple(3));

impl Scale {
    /// `Scale(int magnitude, null)`: no arbitrary factor, so nothing can
    /// fail.
    fn new_simple(magnitude: i32) -> Scale {
        Scale(Arc::new(ScaleData {
            magnitude,
            arbitrary: None,
            reciprocal: None,
            mc: *DEFAULT_MATH_CONTEXT_34_DIGITS,
        }))
    }

    /// `Scale(int, BigDecimal)`.
    fn new(magnitude: i32, arbitrary: Option<BigDecimal>) -> Result<Scale> {
        Self::with_mc(magnitude, arbitrary, *DEFAULT_MATH_CONTEXT_34_DIGITS)
    }

    /// `Scale(int, BigDecimal, MathContext)`.
    fn with_mc(mut magnitude: i32, arbitrary: Option<BigDecimal>, mc: MathContext) -> Result<Scale> {
        let mut arbitrary = arbitrary;
        if let Some(a) = arbitrary.take() {
            // Attempt to convert the BigDecimal to a magnitude multiplier.
            // ICU-20000: JDKs have inconsistent behavior on stripTrailingZeros()
            // for Zero. Fix by checking for zero.
            let a = if a.compare_to(&BigDecimal::zero()) == 0 { BigDecimal::zero() } else { a.strip_trailing_zeros()? };
            if a.precision() == 1 && a.unscaled_value().java_equals(&BigInteger::one()) {
                // Success!
                magnitude -= a.scale();
            } else {
                arbitrary = Some(a);
            }
        }

        // We need the reciprocal for division operations.
        let reciprocal = match &arbitrary {
            Some(a) if BigDecimal::zero().compare_to(a) != 0 => Some(BigDecimal::one().divide_mc(a, &mc)?),
            _ => None,
        };
        Ok(Scale(Arc::new(ScaleData { magnitude, arbitrary, reciprocal, mc })))
    }

    /// Whether two handles are the same Java object.
    pub fn ptr_eq(&self, other: &Scale) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// `Object.hashCode()` (identity): see
    /// `java_identity_hash_code_stand_in`.
    pub fn hash_code(&self) -> i32 {
        java_identity_hash_code_stand_in("com.ibm.icu.number.Scale")
    }

    /// `magnitude`.
    pub(crate) fn magnitude(&self) -> i32 {
        self.0.magnitude
    }

    /// `arbitrary`.
    pub(crate) fn arbitrary(&self) -> Option<&BigDecimal> {
        self.0.arbitrary.as_ref()
    }

    /// Do not change the value of numbers when formatting or parsing.
    /// Equivalent to `none()`.
    pub fn none() -> Scale {
        DEFAULT.clone()
    }

    /// Multiply numbers by a power of ten before formatting. Equivalent to
    /// `powerOfTen(int)`.
    pub fn power_of_ten(power: i32) -> Scale {
        match power {
            0 => DEFAULT.clone(),
            2 => HUNDRED.clone(),
            3 => THOUSAND.clone(),
            _ => Scale::new_simple(power),
        }
    }

    /// Multiply numbers by an arbitrary value before formatting. Equivalent
    /// to `byBigDecimal(BigDecimal)`.
    ///
    /// # Errors
    ///
    /// The `ArithmeticException` of the reciprocal computation.
    pub fn by_big_decimal(multiplicand: &BigDecimal) -> Result<Scale> {
        if multiplicand.compare_to(&BigDecimal::one()) == 0 {
            Ok(DEFAULT.clone())
        } else if multiplicand.compare_to(&BigDecimal::value_of(100)) == 0 {
            Ok(HUNDRED.clone())
        } else if multiplicand.compare_to(&BigDecimal::value_of(1000)) == 0 {
            Ok(THOUSAND.clone())
        } else {
            Scale::new(0, Some(multiplicand.clone()))
        }
    }

    /// Multiply numbers by an arbitrary value before formatting. Equivalent
    /// to `byDouble(double)`.
    ///
    /// # Errors
    ///
    /// The `NumberFormatException` of `BigDecimal.valueOf` for an infinite
    /// or NaN value.
    pub fn by_double(multiplicand: f64) -> Result<Scale> {
        if multiplicand == 1.0 {
            Ok(DEFAULT.clone())
        } else if multiplicand == 100.0 {
            Ok(HUNDRED.clone())
        } else if multiplicand == 1000.0 {
            Ok(THOUSAND.clone())
        } else {
            Scale::new(0, Some(BigDecimal::value_of_double(multiplicand)?))
        }
    }

    /// Multiply a number by both a power of ten and by an arbitrary double
    /// value. Equivalent to `byDoubleAndPowerOfTen(double, int)`.
    ///
    /// # Errors
    ///
    /// As [`by_double`](Self::by_double).
    pub fn by_double_and_power_of_ten(multiplicand: f64, power: i32) -> Result<Scale> {
        Scale::new(power, Some(BigDecimal::value_of_double(multiplicand)?))
    }

    /// Returns whether the multiplier will change the number. Equivalent to
    /// `isValid()` (package-private).
    pub(crate) fn is_valid(&self) -> bool {
        self.0.magnitude != 0 || self.0.arbitrary.is_some()
    }

    /// Equivalent to `withMathContext(MathContext)` (internal): this object
    /// when the math context is equal.
    ///
    /// # Errors
    ///
    /// The `ArithmeticException` of the reciprocal computation.
    pub fn with_math_context(&self, mc: &MathContext) -> Result<Scale> {
        if self.0.mc.java_equals(mc) {
            return Ok(self.clone());
        }
        Scale::with_mc(self.0.magnitude, self.0.arbitrary.clone(), *mc)
    }

    /// Equivalent to `applyTo(DecimalQuantity)` (internal).
    ///
    /// # Errors
    ///
    /// The arithmetic errors of the quantity.
    pub fn apply_to(&self, quantity: &mut dyn DecimalQuantity) -> Result<()> {
        quantity.adjust_magnitude(self.0.magnitude)?;
        if let Some(a) = &self.0.arbitrary {
            quantity.multiply_by(a)?;
        }
        Ok(())
    }

    /// Equivalent to `applyReciprocalTo(DecimalQuantity)` (internal).
    ///
    /// # Errors
    ///
    /// The arithmetic errors of the quantity.
    pub fn apply_reciprocal_to(&self, quantity: &mut dyn DecimalQuantity) -> Result<()> {
        quantity.adjust_magnitude(-self.0.magnitude)?;
        if let Some(r) = &self.0.reciprocal {
            quantity.multiply_by(r)?;
            let mag = quantity.get_magnitude()? - self.0.mc.get_precision();
            quantity.round_to_magnitude(mag, &self.0.mc)?;
        }
        Ok(())
    }
}
