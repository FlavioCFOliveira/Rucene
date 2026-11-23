// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.DecimalQuantity`.

use std::any::Any;

use rucene::error::Result;

use crate::jdk::math::{BigDecimal, MathContext};
use crate::jdk::text::FieldPosition;
use crate::r#impl::number::modifier::Signum;
use crate::r#impl::standard_plural::StandardPlural;
use crate::text::plural_rules::{IFixedDecimal, PluralRules};

/// A number to be processed by the decimal formatting pipeline: rounding,
/// plural rules, and decimal digit extraction. Not immutable and not thread
/// safe: an intermediate object holding state during one pass through the
/// pipeline.
///
/// Equivalent to `com.ibm.icu.impl.number.DecimalQuantity` (an interface
/// extending `PluralRules.IFixedDecimal`). The `Object` members it inherits
/// and implementations override are [`equals`](DecimalQuantity::equals)
/// (`equals(Object)`) and [`to_java_string`](DecimalQuantity::to_java_string)
/// (`toString()`); [`as_any`](DecimalQuantity::as_any) and
/// [`as_any_mut`](DecimalQuantity::as_any_mut) answer Java's casts to the
/// implementation class.
///
/// Methods documented to throw `ArithmeticException` return `Result`.
pub trait DecimalQuantity: IFixedDecimal + std::fmt::Debug + Send + Sync + 'static {
    /// Sets the minimum integer digits that this `DecimalQuantity` should
    /// generate. Must be non-negative. Equivalent to `setMinInteger(int)`.
    fn set_min_integer(&mut self, min_int: i32);

    /// Sets the minimum fraction digits that this `DecimalQuantity` should
    /// generate. Must be non-negative. Equivalent to `setMinFraction(int)`.
    fn set_min_fraction(&mut self, min_frac: i32);

    /// Truncates digits from the upper magnitude of the number in order to
    /// satisfy the specified maximum number of integer digits. Equivalent to
    /// `applyMaxInteger(int)`.
    fn apply_max_integer(&mut self, max_int: i32);

    /// Rounds the number to a specified interval, such as 0.05. Equivalent to
    /// `roundToIncrement(BigDecimal, MathContext)`.
    ///
    /// # Errors
    ///
    /// The `ArithmeticException` of `RoundingMode.UNNECESSARY` when rounding
    /// is required.
    fn round_to_increment(&mut self, rounding_increment: &BigDecimal, math_context: &MathContext) -> Result<()>;

    /// Rounds the number to the nearest multiple of 5 at the specified
    /// magnitude. Equivalent to `roundToNickel(int, MathContext)`.
    ///
    /// # Errors
    ///
    /// `ArithmeticException("Rounding is required on ...")`.
    fn round_to_nickel(&mut self, rounding_magnitude: i32, math_context: &MathContext) -> Result<()>;

    /// Rounds the number to a specified magnitude (power of ten). Equivalent
    /// to `roundToMagnitude(int, MathContext)`.
    ///
    /// # Errors
    ///
    /// `ArithmeticException("Rounding is required on ...")`.
    fn round_to_magnitude(&mut self, rounding_magnitude: i32, math_context: &MathContext) -> Result<()>;

    /// Rounds the number to an infinite number of decimal points: converts a
    /// double-based value to its exact decimal form. Equivalent to
    /// `roundToInfinity()`.
    fn round_to_infinity(&mut self);

    /// Multiplies the internal value by `multiplicand`. Equivalent to
    /// `multiplyBy(BigDecimal)`.
    ///
    /// # Errors
    ///
    /// The `ArithmeticException` of `BigDecimal.multiply` (scale overflow).
    fn multiply_by(&mut self, multiplicand: &BigDecimal) -> Result<()>;

    /// Flips the sign from positive to negative and back. Equivalent to
    /// `negate()`.
    fn negate(&mut self);

    /// Scales the number by a power of ten. Equivalent to
    /// `adjustMagnitude(int)`.
    ///
    /// # Errors
    ///
    /// `ArithmeticException("integer overflow")` when the scale overflows.
    fn adjust_magnitude(&mut self, delta: i32) -> Result<()>;

    /// The power of ten of the highest digit represented by this
    /// `DecimalQuantity`. Equivalent to `getMagnitude()`.
    ///
    /// # Errors
    ///
    /// `ArithmeticException("Magnitude is not well-defined for zero")`.
    fn get_magnitude(&self) -> Result<i32>;

    /// The value of the (suppressed) exponent after the number has been put
    /// into a notation with exponents (ex: compact, scientific). Equivalent to
    /// `getExponent()`.
    fn get_exponent(&self) -> i32;

    /// Adjusts the value for the (suppressed) exponent stored when using a
    /// notation with exponents (ex: compact, scientific). Equivalent to
    /// `adjustExponent(int)`.
    fn adjust_exponent(&mut self, delta: i32);

    /// Resets the DecimalQuantity to the value before adjustExponent was
    /// called. Equivalent to `resetExponent()`.
    ///
    /// # Errors
    ///
    /// The `ArithmeticException` of [`adjust_magnitude`](Self::adjust_magnitude).
    fn reset_exponent(&mut self) -> Result<()>;

    /// Whether the value represented internally is zero. Equivalent to
    /// `isZeroish()`.
    fn is_zeroish(&self) -> bool;

    /// Whether the number is negative. Equivalent to `isNegative()`.
    fn is_negative(&self) -> bool;

    /// The appropriate signum (`-1`/`-0`/`0`/`1`). Equivalent to `signum()`.
    fn signum(&self) -> Signum;

    /// The value as a double. Equivalent to `toDouble()`.
    fn to_double(&self) -> f64;

    /// The value as a `BigDecimal`. Equivalent to `toBigDecimal()`; it
    /// converts a double-based value to its exact decimal form first, in
    /// place, as Java does.
    ///
    /// # Errors
    ///
    /// The `ArithmeticException` of `BigDecimal.scaleByPowerOfTen` (scale
    /// overflow).
    fn to_big_decimal(&mut self) -> Result<BigDecimal>;

    /// The value as a long, optionally truncating the most significant digits
    /// when the value overflows. Equivalent to `toLong(boolean)`.
    fn to_long(&self, truncate_if_overflow: bool) -> i64;

    /// Sets the internal value from a `BigDecimal`. Equivalent to
    /// `setToBigDecimal(BigDecimal)`.
    ///
    /// # Errors
    ///
    /// The `ArithmeticException` of `BigDecimal.scaleByPowerOfTen` (scale
    /// overflow).
    fn set_to_big_decimal(&mut self, input: &BigDecimal) -> Result<()>;

    /// The maximum number of digits this implementation can store.
    /// Equivalent to `maxRepresentableDigits()`.
    fn max_representable_digits(&self) -> i32;

    /// The plural form for this number: [`StandardPlural::Other`] when
    /// `rules` is `None`. Equivalent to `getStandardPlural(PluralRules)`.
    fn get_standard_plural(&self, rules: Option<&PluralRules>) -> StandardPlural;

    /// The digit at the specified magnitude (0 when out of range).
    /// Equivalent to `getDigit(int)`.
    fn get_digit(&self, magnitude: i32) -> i8;

    /// The power of ten of the highest digit to display. Equivalent to
    /// `getUpperDisplayMagnitude()`.
    fn get_upper_display_magnitude(&self) -> i32;

    /// The power of ten of the lowest digit to display. Equivalent to
    /// `getLowerDisplayMagnitude()`.
    fn get_lower_display_magnitude(&self) -> i32;

    /// The string representation of the number, with no exponent. Equivalent
    /// to `toPlainString()`.
    fn to_plain_string(&self) -> String;

    /// The string representation of the number in the exponent notation of
    /// the plural rules samples (`1.5c3`). Equivalent to `toExponentString()`.
    fn to_exponent_string(&self) -> String;

    /// A copy of this instance which can be mutated without affecting this
    /// one. Equivalent to `createCopy()`.
    fn create_copy(&self) -> Box<dyn DecimalQuantity>;

    /// Sets this instance to be equal to another instance. Equivalent to
    /// `copyFrom(DecimalQuantity)`.
    ///
    /// # Panics
    ///
    /// With Java's `ClassCastException` text when `other` is not of this
    /// implementation's class.
    fn copy_from(&mut self, other: &dyn DecimalQuantity);

    /// A single `long` used to compare the "position" of numbers (their
    /// requested integer and fraction widths). Equivalent to
    /// `getPositionFingerprint()`.
    fn get_position_fingerprint(&self) -> i64;

    /// Sets the fraction digits of `fp` when it is a `UFieldPosition`.
    /// Equivalent to `populateUFieldPosition(FieldPosition)`.
    fn populate_u_field_position(&self, fp: &mut FieldPosition);

    /// Equivalent to `equals(Object)`.
    fn equals(&self, other: &dyn DecimalQuantity) -> bool;

    /// Equivalent to `toString()`.
    fn to_java_string(&self) -> String;

    /// This object as `Any`, for Java's casts to the implementation class.
    fn as_any(&self) -> &dyn Any;

    /// This object as mutable `Any`, for Java's casts to the implementation
    /// class.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}
