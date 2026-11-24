// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.DecimalQuantity_AbstractBCD`.

use std::any::Any;

use rucene::error::Result;
use rucene::util::java_fmt::double_to_string;

use crate::jdk::arithmetic;
use crate::jdk::math::{BigDecimal, BigInteger, MathContext};
use crate::jdk::text::FieldPosition;
use crate::r#impl::number::decimal_quantity::DecimalQuantity;
use crate::r#impl::number::modifier::Signum;
use crate::r#impl::number::rounding_utils::RoundingUtils;
use crate::r#impl::standard_plural::StandardPlural;
use crate::r#impl::utility::Utility;
use crate::text::plural_rules::{IFixedDecimal, Operand, PluralRules};
use crate::text::u_field_position::UFieldPosition;

/// `NEGATIVE_FLAG`.
pub const NEGATIVE_FLAG: i8 = 1;
/// `INFINITY_FLAG`.
pub const INFINITY_FLAG: i8 = 2;
/// `NAN_FLAG`.
pub const NAN_FLAG: i8 = 4;

/// `DOUBLE_MULTIPLIERS`: the powers of ten `1e0..=1e21`.
const DOUBLE_MULTIPLIERS: [f64; 22] = [
    1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16, 1e17, 1e18,
    1e19, 1e20, 1e21,
];

/// `INT64_BCD`: the digits of `Long.MIN_VALUE`, most significant first.
pub(crate) const INT64_BCD: [i8; 19] = [9, 2, 2, 3, 3, 7, 2, 0, 3, 6, 8, 5, 4, 7, 7, 5, 8, 0, 8];

/// `SECTION_LOWER_EDGE`.
const SECTION_LOWER_EDGE: i32 = -1;
/// `SECTION_UPPER_EDGE`.
const SECTION_UPPER_EDGE: i32 = -2;

/// The protected fields of `DecimalQuantity_AbstractBCD`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AbstractBcdFields {
    /// `scale`: the power of ten corresponding to the least significant digit
    /// in the BCD.
    pub scale: i32,
    /// `precision`: the number of digits in the BCD.
    pub precision: i32,
    /// `flags`: [`NEGATIVE_FLAG`], [`INFINITY_FLAG`], [`NAN_FLAG`].
    pub flags: i8,
    /// `origDouble`: the original number provided by the user, used when the
    /// value is approximate.
    pub orig_double: f64,
    /// `origDelta`: the change in magnitude relative to the original double.
    pub orig_delta: i32,
    /// `isApproximate`: whether the value is an approximation of
    /// `origDouble`.
    pub is_approximate: bool,
    /// `lReqPos`: the minimum number of integer digits to display.
    pub l_req_pos: i32,
    /// `rReqPos`: minus the minimum number of fraction digits to display.
    pub r_req_pos: i32,
    /// `exponent`: the value of the (suppressed) exponent.
    pub exponent: i32,
    /// `explicitExactDouble` (public, internal): whether the value was
    /// converted to its exact form through `Double.toString`.
    pub explicit_exact_double: bool,
}

/// `safeSubtract(int, int)`: the difference, saturated at the `int` range.
fn safe_subtract(a: i32, b: i32) -> i32 {
    let diff = a.wrapping_sub(b);
    if b < 0 && diff < a {
        return i32::MAX;
    }
    if b > 0 && diff > a {
        return i32::MIN;
    }
    diff
}

/// Represents numbers and digit display properties using Binary Coded
/// Decimal (BCD).
///
/// Equivalent to `com.ibm.icu.impl.number.DecimalQuantity_AbstractBCD`. The
/// abstract class is this trait: [`fields`](Self::fields) and
/// [`fields_mut`](Self::fields_mut) give its protected fields, the required
/// methods are its abstract BCD operations (and the `createCopy`,
/// `maxRepresentableDigits` and `toString` its one subclass supplies), the
/// provided methods are its own public and private methods, and every type of
/// this trait is a [`DecimalQuantity`] (and a [`IFixedDecimal`]) through the
/// interface methods the abstract class implements.
pub trait DecimalQuantityAbstractBcd: std::fmt::Debug + Send + Sync + Sized + 'static {
    /// The protected fields.
    fn fields(&self) -> &AbstractBcdFields;
    /// The protected fields, mutably.
    fn fields_mut(&mut self) -> &mut AbstractBcdFields;

    /// Returns a single digit from the BCD list; out of range positions
    /// answer 0. Equivalent to `getDigitPos(int)`.
    fn get_digit_pos(&self, position: i32) -> i8;
    /// Sets the digit in the BCD list. Equivalent to `setDigitPos(int,
    /// byte)`.
    fn set_digit_pos(&mut self, position: i32, value: i8);
    /// Adds zeros to the end of the BCD list, adjusting the scale.
    /// Equivalent to `shiftLeft(int)`.
    fn shift_left(&mut self, num_digits: i32);
    /// Removes digits from the end of the BCD list, adjusting the scale.
    /// Equivalent to `shiftRight(int)`.
    fn shift_right(&mut self, num_digits: i32);
    /// Directly removes digits from the front of the BCD list. Equivalent to
    /// `popFromLeft(int)`.
    fn pop_from_left(&mut self, num_digits: i32);
    /// Sets the internal representation to zero, clearing all fields except
    /// the flags and the requested positions. Equivalent to
    /// `setBcdToZero()`.
    fn set_bcd_to_zero(&mut self);
    /// Sets the BCD to a non-zero positive `int`. Equivalent to
    /// `readIntToBcd(int)`.
    fn read_int_to_bcd(&mut self, input: i32);
    /// Sets the BCD to a non-zero positive `long`. Equivalent to
    /// `readLongToBcd(long)`.
    fn read_long_to_bcd(&mut self, input: i64);
    /// Sets the BCD to a non-zero positive `BigInteger`. Equivalent to
    /// `readBigIntegerToBcd(BigInteger)`.
    fn read_big_integer_to_bcd(&mut self, input: &BigInteger);
    /// The BCD as a `BigDecimal`. Equivalent to `bcdToBigDecimal()`.
    ///
    /// # Errors
    ///
    /// The `ArithmeticException` of `BigDecimal.scaleByPowerOfTen`.
    fn bcd_to_big_decimal(&self) -> Result<BigDecimal>;
    /// Copies the BCD of `other`. Equivalent to
    /// `copyBcdFrom(DecimalQuantity)`.
    fn copy_bcd_from(&mut self, other: &dyn DecimalQuantity);
    /// Removes trailing zeros from the BCD and normalizes the scale and
    /// precision. Equivalent to `compact()`.
    fn compact(&mut self);
    /// The subclass's `maxRepresentableDigits()`.
    fn bcd_max_representable_digits(&self) -> i32;
    /// The subclass's `createCopy()`.
    fn bcd_create_copy(&self) -> Box<dyn DecimalQuantity>;
    /// The subclass's `toString()`.
    fn bcd_to_string(&self) -> String;

    /// Equivalent to `clear()`.
    fn clear(&mut self) -> &mut Self {
        let f = self.fields_mut();
        f.l_req_pos = 0;
        f.r_req_pos = 0;
        f.flags = 0;
        self.set_bcd_to_zero();
        self
    }

    /// Equivalent to `setToInt(int)`.
    fn set_to_int(&mut self, mut n: i32) {
        self.set_bcd_to_zero();
        self.fields_mut().flags = 0;
        if n < 0 {
            self.fields_mut().flags |= NEGATIVE_FLAG;
            n = n.wrapping_neg();
        }
        if n != 0 {
            self.set_to_int_internal(n);
            self.compact();
        }
    }

    /// `_setToInt(int)`.
    fn set_to_int_internal(&mut self, n: i32) {
        if n == i32::MIN {
            self.read_long_to_bcd(-(n as i64));
        } else {
            self.read_int_to_bcd(n);
        }
    }

    /// Equivalent to `setToLong(long)`.
    fn set_to_long(&mut self, mut n: i64) {
        self.set_bcd_to_zero();
        self.fields_mut().flags = 0;
        if n < 0 {
            self.fields_mut().flags |= NEGATIVE_FLAG;
            n = n.wrapping_neg();
        }
        if n != 0 {
            self.set_to_long_internal(n);
            self.compact();
        }
    }

    /// `_setToLong(long)`.
    fn set_to_long_internal(&mut self, n: i64) {
        if n == i64::MIN {
            self.read_big_integer_to_bcd(&BigInteger::value_of(n).negate());
        } else if n <= i32::MAX as i64 {
            self.read_int_to_bcd(n as i32);
        } else {
            self.read_long_to_bcd(n);
        }
    }

    /// Equivalent to `setToBigInteger(BigInteger)`.
    fn set_to_big_integer(&mut self, n: &BigInteger) {
        self.set_bcd_to_zero();
        self.fields_mut().flags = 0;
        let mut n = n.clone();
        if n.signum() == -1 {
            self.fields_mut().flags |= NEGATIVE_FLAG;
            n = n.negate();
        }
        if n.signum() != 0 {
            self.set_to_big_integer_internal(&n);
            self.compact();
        }
    }

    /// `_setToBigInteger(BigInteger)`.
    fn set_to_big_integer_internal(&mut self, n: &BigInteger) {
        if n.bit_length() < 32 {
            self.read_int_to_bcd(n.int_value());
        } else if n.bit_length() < 64 {
            self.read_long_to_bcd(n.long_value());
        } else {
            self.read_big_integer_to_bcd(n);
        }
    }

    /// Sets the internal representation to a double: an approximation until
    /// it is converted to its exact form. Equivalent to
    /// `setToDouble(double)`.
    fn set_to_double(&mut self, mut n: f64) {
        self.set_bcd_to_zero();
        self.fields_mut().flags = 0;
        // Double.doubleToRawLongBits(n) < 0: the sign bit, so -0.0 too.
        if (n.to_bits() as i64) < 0 {
            self.fields_mut().flags |= NEGATIVE_FLAG;
            n = -n;
        }
        if n.is_nan() {
            self.fields_mut().flags |= NAN_FLAG;
        } else if n.is_infinite() {
            self.fields_mut().flags |= INFINITY_FLAG;
        } else if n != 0.0 {
            self.set_to_double_fast(n);
            self.compact();
        }
    }

    /// `_setToDoubleFast(double)`: uses double multiplication and division to
    /// get the number into integer space before converting to digits; the
    /// result is approximate.
    fn set_to_double_fast(&mut self, mut n: f64) {
        {
            let f = self.fields_mut();
            f.is_approximate = true;
            f.orig_double = n;
            f.orig_delta = 0;
        }

        // NOTE: Double.doubleToRawLongBits(n) == Double.doubleToLongBits(n) for
        // a non-NaN value.
        let ieee_bits = n.to_bits() as i64;
        let exponent = (((ieee_bits as u64 & 0x7ff0000000000000u64) >> 52) as i32) - 0x3ff;

        // Not all integers can be represented exactly for exponent > 52
        if exponent <= 52 && (n as i64) as f64 == n {
            self.set_to_long_internal(n as i64);
            return;
        }

        if exponent == -1023 || exponent == 1024 {
            // The extreme values of exponent are special; use slow path.
            self.convert_to_accurate_double();
            return;
        }

        // 3.3219... is log2(10)
        // Java's literal, kept as written (it rounds to `LOG2_10`).
        #[allow(clippy::approx_constant)]
        let frac_length = ((52 - exponent) as f64 / 3.321_928_094_887_362) as i32;
        if frac_length >= 0 {
            let mut i = frac_length;
            // 1e22 is the largest exact double.
            while i >= 22 {
                n *= 1e22;
                i -= 22;
            }
            n *= DOUBLE_MULTIPLIERS[i as usize];
        } else {
            let mut i = frac_length;
            // 1e22 is the largest exact double.
            while i <= -22 {
                n /= 1e22;
                i += 22;
            }
            n /= DOUBLE_MULTIPLIERS[(-i) as usize];
        }
        let result = java_math_round(n);
        if result != 0 {
            self.set_to_long_internal(result);
            self.fields_mut().scale -= frac_length;
        }
    }

    /// `convertToAccurateDouble()`: uses `Double.toString()`, which produces
    /// the shortest representation of the double, to obtain the exact value.
    fn convert_to_accurate_double(&mut self) {
        let n = self.fields().orig_double;
        debug_assert!(n != 0.0);
        let delta = self.fields().orig_delta;
        self.set_bcd_to_zero();

        // Call the slow oracle function (Double.toString in Java)
        let dstr = double_to_string(n);
        let b = dstr.as_bytes();

        if let Some(exp_pos) = dstr.find('E') {
            // Case 1: Exponential notation.
            debug_assert!(dstr.find('.') == Some(1));
            let digits = format!("{}{}", &dstr[0..1], &dstr[2..exp_pos]);
            self.set_to_long_internal(parse_long_digits(&digits));
            let exp: i32 = dstr[exp_pos + 1..].parse().expect("Double.toString renders a decimal exponent");
            self.fields_mut().scale += exp - (exp_pos as i32 - 1) + 1;
        } else if b[0] == b'0' {
            // Case 2: Fraction-only number.
            debug_assert!(dstr.find('.') == Some(1));
            self.set_to_long_internal(parse_long_digits(&dstr[2..]));
            self.fields_mut().scale += 2 - dstr.len() as i32;
        } else if b[b.len() - 1] == b'0' {
            // Case 3: Integer-only number.
            // Note: this path should not normally happen, because integer-only
            // numbers are handled in the fast path of setToDouble.
            debug_assert!(dstr.find('.') == Some(dstr.len() - 2));
            debug_assert!(dstr.len() - 2 <= 18);
            self.set_to_long_internal(parse_long_digits(&dstr[0..dstr.len() - 2]));
            // no need to adjust scale
        } else {
            // Case 4: Number with both a fraction and an integer.
            let decimal_pos = dstr.find('.').expect("Double.toString renders a decimal point");
            let digits = format!("{}{}", &dstr[0..decimal_pos], &dstr[decimal_pos + 1..]);
            self.set_to_long_internal(parse_long_digits(&digits));
            self.fields_mut().scale += decimal_pos as i32 - dstr.len() as i32 + 1;
        }

        self.fields_mut().scale += delta;
        self.compact();
        self.fields_mut().explicit_exact_double = true;
    }

    /// `_setToBigDecimal(BigDecimal)`.
    ///
    /// # Errors
    ///
    /// The `ArithmeticException` of `BigDecimal.scaleByPowerOfTen`.
    fn set_to_big_decimal_internal(&mut self, n: &BigDecimal) -> Result<()> {
        let frac_length = n.scale();
        let n = n.scale_by_power_of_ten(frac_length)?;
        let bi = n.to_big_integer();
        self.set_to_big_integer_internal(&bi);
        self.fields_mut().scale -= frac_length;
        Ok(())
    }

    /// The fraction part of the number as a long, at most 18 digits.
    /// Equivalent to `toFractionLong(boolean)`.
    fn to_fraction_long(&self, include_trailing_zeros: bool) -> i64 {
        let f = *self.fields();
        let mut result: i64 = 0;
        let mut magnitude = -1 - f.exponent;
        let mut lower_magnitude = f.scale;
        if include_trailing_zeros {
            lower_magnitude = lower_magnitude.min(f.r_req_pos);
        }
        // NOTE: Java has only signed longs, so we check result <= 1e17
        // instead of 1e18
        while magnitude >= lower_magnitude && (result as f64) <= 1e17 {
            result = result.wrapping_mul(10).wrapping_add(self.get_digit_pos(magnitude - f.scale) as i64);
            magnitude -= 1;
        }
        // Remove trailing zeros; this can happen during integer overflow
        // cases.
        if !include_trailing_zeros {
            while result > 0 && (result % 10) == 0 {
                result /= 10;
            }
        }
        result
    }

    /// Whether the value fits in a `long` (exactly). Equivalent to
    /// `fitsInLong()`.
    fn fits_in_long(&self) -> bool {
        let f = *self.fields();
        if (f.flags & INFINITY_FLAG) != 0 || (f.flags & NAN_FLAG) != 0 {
            return false;
        }
        if f.precision == 0 {
            return true;
        }
        if f.exponent + f.scale < 0 {
            return false;
        }
        // getMagnitude() cannot throw: the precision is not zero.
        let magnitude = f.scale + f.precision - 1;
        if magnitude < 18 {
            return true;
        }
        if magnitude > 18 {
            return false;
        }
        // Hard case: the magnitude is 10^18.
        // The largest int64 is: 9,223,372,036,854,775,807
        for p in 0..f.precision {
            let digit = self.get_digit_pos(18 - p - f.scale);
            if digit < INT64_BCD[p as usize] {
                return true;
            } else if digit > INT64_BCD[p as usize] {
                return false;
            }
        }
        // Exactly equal to max long plus one.
        (f.flags & NEGATIVE_FLAG) != 0
    }

    /// Equivalent to `truncate()`.
    fn truncate(&mut self) {
        let scale = self.fields().scale;
        if scale < 0 {
            self.shift_right(-scale);
            self.fields_mut().scale = 0;
            self.compact();
        }
    }

    /// `roundToMagnitude(int, MathContext, boolean)`.
    ///
    /// # Errors
    ///
    /// `ArithmeticException("Rounding is required on ...")`.
    fn round_to_magnitude_nickel(&mut self, magnitude: i32, math_context: &MathContext, nickel: bool) -> Result<()> {
        // The position in the BCD at which rounding will be performed;
        // digits to the right of position will be rounded away.
        let mut position = safe_subtract(magnitude, self.fields().scale);

        // "trailing" = least significant digit to the left of rounding
        let mc_precision = math_context.get_precision();
        let precision = self.fields().precision;
        if mc_precision > 0 && precision - mc_precision > position {
            position = precision - mc_precision;
        }

        // "trailing" = least significant digit to the left of rounding
        let mut trailing_digit = self.get_digit_pos(position);

        let is_approximate = self.fields().is_approximate;
        if position <= 0 && !is_approximate && (!nickel || trailing_digit == 0 || trailing_digit == 5) {
            // All digits are to the left of the rounding magnitude.
        } else if precision == 0 {
            // No rounding for zero.
        } else {
            // Perform rounding logic.
            // "leading" = most significant digit to the right of rounding
            let leading_digit = self.get_digit_pos(safe_subtract(position, 1));

            // Compute which section of the number we are in.
            // EDGE means we are at the bottom or top edge, like 1.000 or 1.999
            // (used by doubles)
            // LOWER means we are between the bottom edge and the midpoint,
            // like 1.391
            // MIDPOINT means we are exactly in the middle, like 1.500
            // UPPER means we are between the midpoint and the top edge, like
            // 1.916
            let mut section;
            if !self.fields().is_approximate {
                if nickel && trailing_digit != 2 && trailing_digit != 7 {
                    // Nickel rounding, and not at .02x or .07x
                    if trailing_digit < 2 {
                        // .00, .01 => down to .00
                        section = RoundingUtils::SECTION_LOWER;
                    } else if trailing_digit < 5 {
                        // .03, .04 => up to .05
                        section = RoundingUtils::SECTION_UPPER;
                    } else if trailing_digit < 7 {
                        // .05, .06 => down to .05
                        section = RoundingUtils::SECTION_LOWER;
                    } else {
                        // .08, .09 => up to .10
                        section = RoundingUtils::SECTION_UPPER;
                    }
                } else if leading_digit < 5 {
                    // Includes nickel rounding .020-.024 and .070-.074
                    section = RoundingUtils::SECTION_LOWER;
                } else if leading_digit > 5 {
                    // Includes nickel rounding .026-.029 and .076-.079
                    section = RoundingUtils::SECTION_UPPER;
                } else {
                    // Includes nickel rounding .025 and .075
                    section = RoundingUtils::SECTION_MIDPOINT;
                    let mut p = safe_subtract(position, 2);
                    while p >= 0 {
                        if self.get_digit_pos(p) != 0 {
                            section = RoundingUtils::SECTION_UPPER;
                            break;
                        }
                        p -= 1;
                    }
                }
            } else {
                let mut p = safe_subtract(position, 2);
                let min_p = 0.max(precision - 14);
                if leading_digit == 0 && (!nickel || trailing_digit == 0 || trailing_digit == 5) {
                    section = SECTION_LOWER_EDGE;
                    while p >= min_p {
                        if self.get_digit_pos(p) != 0 {
                            section = RoundingUtils::SECTION_LOWER;
                            break;
                        }
                        p -= 1;
                    }
                } else if leading_digit == 4 && (!nickel || trailing_digit == 2 || trailing_digit == 7) {
                    section = RoundingUtils::SECTION_MIDPOINT;
                    while p >= min_p {
                        if self.get_digit_pos(p) != 9 {
                            section = RoundingUtils::SECTION_LOWER;
                            break;
                        }
                        p -= 1;
                    }
                } else if leading_digit == 5 && (!nickel || trailing_digit == 2 || trailing_digit == 7) {
                    section = RoundingUtils::SECTION_MIDPOINT;
                    while p >= min_p {
                        if self.get_digit_pos(p) != 0 {
                            section = RoundingUtils::SECTION_UPPER;
                            break;
                        }
                        p -= 1;
                    }
                } else if leading_digit == 9 && (!nickel || trailing_digit == 4 || trailing_digit == 9) {
                    section = SECTION_UPPER_EDGE;
                    while p >= min_p {
                        if self.get_digit_pos(p) != 9 {
                            section = RoundingUtils::SECTION_UPPER;
                            break;
                        }
                        p -= 1;
                    }
                } else if nickel && trailing_digit != 2 && trailing_digit != 7 {
                    // Nickel rounding, and not at .02x or .07x
                    if trailing_digit < 2 {
                        // .00, .01 => down to .00
                        section = RoundingUtils::SECTION_LOWER;
                    } else if trailing_digit < 5 {
                        // .03, .04 => up to .05
                        section = RoundingUtils::SECTION_UPPER;
                    } else if trailing_digit < 7 {
                        // .05, .06 => down to .05
                        section = RoundingUtils::SECTION_LOWER;
                    } else {
                        // .08, .09 => up to .10
                        section = RoundingUtils::SECTION_UPPER;
                    }
                } else if leading_digit < 5 {
                    // Includes nickel rounding .020-.024 and .070-.074
                    section = RoundingUtils::SECTION_LOWER;
                } else {
                    // Includes nickel rounding .026-.029 and .076-.079
                    section = RoundingUtils::SECTION_UPPER;
                }

                let rounds_at_midpoint = RoundingUtils::rounds_at_midpoint(math_context.get_rounding_mode().ordinal());
                if safe_subtract(position, 1) < precision - 14
                    || (rounds_at_midpoint && section == RoundingUtils::SECTION_MIDPOINT)
                    || (!rounds_at_midpoint && section < 0)
                {
                    // Oops! This means that we have to get the exact
                    // representation of the double, because the zone of
                    // uncertainty is along the rounding boundary.
                    self.convert_to_accurate_double();
                    return self.round_to_magnitude_nickel(magnitude, math_context, nickel); // start over
                }

                // Turn off the approximate double flag, since the value is now
                // confirmed to be exact.
                {
                    let f = self.fields_mut();
                    f.is_approximate = false;
                    f.orig_double = 0.0;
                    f.orig_delta = 0;
                }

                if position <= 0 && (!nickel || trailing_digit == 0 || trailing_digit == 5) {
                    // All digits are to the left of the rounding magnitude.
                    return Ok(());
                }

                // Good to continue rounding.
                if section == SECTION_LOWER_EDGE {
                    section = RoundingUtils::SECTION_LOWER;
                }
                if section == SECTION_UPPER_EDGE {
                    section = RoundingUtils::SECTION_UPPER;
                }
            }

            // Nickel rounding "half even" goes to the nearest whole (away
            // from the 5).
            let is_even = if nickel {
                !(2..=7).contains(&trailing_digit)
                    || (trailing_digit == 2 && section != RoundingUtils::SECTION_UPPER)
                    || (trailing_digit == 7 && section == RoundingUtils::SECTION_UPPER)
            } else {
                (trailing_digit % 2) == 0
            };

            let is_negative = (self.fields().flags & NEGATIVE_FLAG) != 0;
            let round_down = RoundingUtils::get_rounding_direction(
                is_even,
                is_negative,
                section,
                math_context.get_rounding_mode().ordinal(),
                &|| self.bcd_to_string(),
            )?;

            // Perform truncation
            if position >= self.fields().precision {
                debug_assert!(trailing_digit == 0);
                self.set_bcd_to_zero();
                self.fields_mut().scale = magnitude;
            } else {
                self.shift_right(position);
            }

            if nickel {
                if trailing_digit < 5 && round_down {
                    self.set_digit_pos(0, 0);
                    self.compact();
                    return Ok(());
                } else if trailing_digit >= 5 && !round_down {
                    self.set_digit_pos(0, 9);
                    trailing_digit = 9;
                    // do not return: use the bubbling logic below
                } else {
                    self.set_digit_pos(0, 5);
                    // If the quantity was set to 0, we may need to restore a
                    // digit.
                    if self.fields().precision == 0 {
                        self.fields_mut().precision = 1;
                    }
                    // compact not necessary: digit at position 0 is nonzero
                    return Ok(());
                }
            }

            // Bubble the result to the higher digits
            if !round_down {
                if trailing_digit == 9 {
                    let mut bubble_pos = 0;
                    // Note: in the long implementation, the most digits BCD
                    // can have at this point is 15, so bubblePos <= 15 and
                    // getDigitPos(bubblePos) is safe.
                    while self.get_digit_pos(bubble_pos) == 9 {
                        bubble_pos += 1;
                    }
                    self.shift_right(bubble_pos); // shift off the trailing 9s
                }
                let digit0 = self.get_digit_pos(0);
                debug_assert!(digit0 != 9);
                self.set_digit_pos(0, digit0 + 1);
                self.fields_mut().precision += 1; // in case an extra digit got added
            }

            self.compact();
        }
        Ok(())
    }

    /// Appends a digit, optionally with one or more leading zeros, to the
    /// end of the value represented by this `DecimalQuantity`. Equivalent to
    /// `appendDigit(byte, int, boolean)`.
    fn append_digit(&mut self, value: i8, mut leading_zeros: i32, append_as_integer: bool) {
        debug_assert!(leading_zeros >= 0);

        // Zero requires special handling to maintain the invariant that the
        // least-significant digit in the BCD is nonzero.
        if value == 0 {
            if append_as_integer && self.fields().precision != 0 {
                self.fields_mut().scale += leading_zeros + 1;
            }
            return;
        }

        // Deal with trailing zeros
        let scale = self.fields().scale;
        if scale > 0 {
            leading_zeros += scale;
            if append_as_integer {
                self.fields_mut().scale = 0;
            }
        }

        // Append digit
        self.shift_left(leading_zeros + 1);
        self.set_digit_pos(0, value);

        // Fix scale if in integer mode
        if append_as_integer {
            self.fields_mut().scale += leading_zeros + 1;
        }
    }

    /// Equivalent to `toPlainString(StringBuilder)`.
    fn to_plain_string_into(&self, result: &mut String) {
        let f = *self.fields();
        debug_assert!(!f.is_approximate);
        if (f.flags & NEGATIVE_FLAG) != 0 {
            result.push('-');
        }
        if f.precision == 0 {
            result.push('0');
            return;
        }

        let mut upper = f.scale + f.precision + f.exponent - 1;
        let mut lower = f.scale + f.exponent;
        if upper < f.l_req_pos - 1 {
            upper = f.l_req_pos - 1;
        }
        if lower > f.r_req_pos {
            lower = f.r_req_pos;
        }

        let mut p = upper;
        if p < 0 {
            result.push('0');
        }
        while p >= 0 {
            result.push((b'0' as i8 + self.get_digit_pos(p - f.scale - f.exponent)) as u8 as char);
            p -= 1;
        }
        if lower < 0 {
            result.push('.');
        }
        while p >= lower {
            result.push((b'0' as i8 + self.get_digit_pos(p - f.scale - f.exponent)) as u8 as char);
            p -= 1;
        }
    }

    /// Equivalent to `toScientificString()`.
    fn to_scientific_string(&self) -> String {
        let mut sb = String::new();
        self.to_scientific_string_into(&mut sb);
        sb
    }

    /// Equivalent to `toScientificString(StringBuilder)`.
    fn to_scientific_string_into(&self, result: &mut String) {
        let f = *self.fields();
        debug_assert!(!f.is_approximate);
        if (f.flags & NEGATIVE_FLAG) != 0 {
            result.push('-');
        }
        if f.precision == 0 {
            result.push_str("0E+0");
            return;
        }
        // NOTE: It is not safe to add to lOptPos (aka maxInt) or subtract
        // from rOptPos (aka -maxFrac) due to overflow.
        let upper_pos = f.precision - 1;
        let lower_pos = 0;
        let mut p = upper_pos;
        result.push((b'0' as i8 + self.get_digit_pos(p)) as u8 as char);
        p -= 1;
        if p >= lower_pos {
            result.push('.');
            while p >= lower_pos {
                result.push((b'0' as i8 + self.get_digit_pos(p)) as u8 as char);
                p -= 1;
            }
        }
        result.push('E');
        let mut scale = upper_pos.wrapping_add(f.scale).wrapping_add(f.exponent);
        if scale == i32::MIN {
            result.push_str("-2147483648");
            return;
        } else if scale < 0 {
            scale *= -1;
            result.push('-');
        } else {
            result.push('+');
        }
        if scale == 0 {
            result.push('0');
        }
        let insert_index = result.len();
        while scale > 0 {
            let quot = scale / 10;
            let rem = scale % 10;
            result.insert(insert_index, (b'0' + rem as u8) as char);
            scale = quot;
        }
    }

    /// `toExponentString(StringBuilder)`.
    fn to_exponent_string_into(&self, result: &mut String) {
        let f = *self.fields();
        debug_assert!(!f.is_approximate);
        if (f.flags & NEGATIVE_FLAG) != 0 {
            result.push('-');
        }

        let mut upper = f.scale + f.precision - 1;
        let mut lower = f.scale;
        if upper < f.l_req_pos - 1 {
            upper = f.l_req_pos - 1;
        }
        if lower > f.r_req_pos {
            lower = f.r_req_pos;
        }

        let mut p = upper;
        if p < 0 {
            result.push('0');
        }
        while p >= 0 {
            result.push((b'0' as i8 + self.get_digit_pos(p - f.scale)) as u8 as char);
            p -= 1;
        }
        if lower < 0 {
            result.push('.');
        }
        while p >= lower {
            result.push((b'0' as i8 + self.get_digit_pos(p - f.scale)) as u8 as char);
            p -= 1;
        }

        if f.exponent != 0 {
            result.push('c');
            result.push_str(&f.exponent.to_string());
        }
    }

    /// `fractionCount()`.
    fn fraction_count(&self) -> i32 {
        0.max(-DecimalQuantity::get_lower_display_magnitude(self) - self.fields().exponent)
    }

    /// `fractionCountWithoutTrailingZeros()`.
    fn fraction_count_without_trailing_zeros(&self) -> i32 {
        (-self.fields().scale - self.fields().exponent).max(0)
    }
}

/// `Math.round(double)`: the closest `long`, ties toward positive infinity,
/// saturated at the `long` range (`NaN` gives 0).
fn java_math_round(a: f64) -> i64 {
    if a.is_nan() {
        return 0;
    }
    // floor(a + 1/2) computed exactly (`a - floor(a)` is exact), as the JDK
    // does with its bit arithmetic; the cast saturates as Java's does.
    let f = a.floor();
    if a - f >= 0.5 {
        (f as i64).saturating_add(1)
    } else {
        f as i64
    }
}

/// `Long.parseLong(String)` of the digit strings `convertToAccurateDouble`
/// builds from `Double.toString` (at most 17 digits, always valid).
fn parse_long_digits(digits: &str) -> i64 {
    digits.parse().expect("Double.toString renders at most 17 decimal digits")
}

impl<T: DecimalQuantityAbstractBcd> IFixedDecimal for T {
    fn get_plural_operand(&self, operand: Operand) -> f64 {
        // If this assertion fails, you need to call roundToInfinity() or some
        // other rounding method. See the comment at the top of this file
        // explaining the "isApproximate" field.
        debug_assert!(!self.fields().is_approximate);

        match operand {
            // Invert the negative sign if necessary
            Operand::I => {
                if DecimalQuantity::is_negative(self) {
                    -(DecimalQuantity::to_long(self, true) as f64)
                } else {
                    DecimalQuantity::to_long(self, true) as f64
                }
            }
            Operand::F => self.to_fraction_long(true) as f64,
            Operand::T => self.to_fraction_long(false) as f64,
            Operand::V => self.fraction_count() as f64,
            Operand::W => self.fraction_count_without_trailing_zeros() as f64,
            Operand::E => DecimalQuantity::get_exponent(self) as f64,
            // Plural operand `c` is currently deprecated, but still
            // supported for compatibility.
            Operand::C => DecimalQuantity::get_exponent(self) as f64,
            _ => DecimalQuantity::to_double(self).abs(),
        }
    }

    fn is_nan(&self) -> bool {
        (self.fields().flags & NAN_FLAG) != 0
    }

    fn is_infinite(&self) -> bool {
        (self.fields().flags & INFINITY_FLAG) != 0
    }

    fn is_has_integer_value(&self) -> bool {
        self.fields().scale >= 0
    }
}

impl<T: DecimalQuantityAbstractBcd> DecimalQuantity for T {
    fn set_min_integer(&mut self, mut min_int: i32) {
        // Validation should happen outside of DecimalQuantity, e.g., in the
        // Precision class.
        debug_assert!(min_int >= 0);

        // Special behavior: do not set minInt to be less than what is
        // already set. This is so significant digits rounding can set the
        // integer length.
        if min_int < self.fields().l_req_pos {
            min_int = self.fields().l_req_pos;
        }

        // Save values into internal state
        self.fields_mut().l_req_pos = min_int;
    }

    fn set_min_fraction(&mut self, min_frac: i32) {
        // Validation should happen outside of DecimalQuantity, e.g., in the
        // Precision class.
        debug_assert!(min_frac >= 0);

        // Save values into internal state
        // Negation is safe for minFrac/maxFrac because -Integer.MAX_VALUE >
        // Integer.MIN_VALUE
        self.fields_mut().r_req_pos = -min_frac;
    }

    fn apply_max_integer(&mut self, max_int: i32) {
        // Validation should happen outside of DecimalQuantity, e.g., in the
        // Precision class.
        debug_assert!(max_int >= 0);

        let f = *self.fields();
        if f.precision == 0 {
            return;
        }

        if max_int <= f.scale {
            self.set_bcd_to_zero();
            return;
        }

        // getMagnitude() cannot throw: the precision is not zero.
        let magnitude = f.scale + f.precision - 1;
        if max_int <= magnitude {
            self.pop_from_left(magnitude - max_int + 1);
            self.compact();
        }
    }

    fn round_to_increment(&mut self, rounding_increment: &BigDecimal, math_context: &MathContext) -> Result<()> {
        // Do not call this method with an increment having only a 1 or a 5
        // digit! Use a more efficient call to either roundToMagnitude() or
        // roundToNickel(). Note: The check, which is somewhat expensive, is
        // performed in an assertion to disable it in production.
        debug_assert!({
            let stripped = rounding_increment.strip_trailing_zeros()?;
            stripped.precision() != 1
                || stripped.unscaled_value().int_value() != 5
                || stripped.unscaled_value().int_value() != 1
        });
        let mut temp = self.to_big_decimal()?;
        temp = temp
            .divide_scale(rounding_increment, 0, math_context.get_rounding_mode())?
            .multiply(rounding_increment)?
            .round(math_context)?;
        if temp.signum() == 0 {
            self.set_bcd_to_zero(); // keeps negative flag for -0.0
        } else {
            self.set_to_big_decimal(&temp)?;
        }
        Ok(())
    }

    fn round_to_nickel(&mut self, magnitude: i32, math_context: &MathContext) -> Result<()> {
        self.round_to_magnitude_nickel(magnitude, math_context, true)
    }

    fn round_to_magnitude(&mut self, magnitude: i32, math_context: &MathContext) -> Result<()> {
        self.round_to_magnitude_nickel(magnitude, math_context, false)
    }

    fn round_to_infinity(&mut self) {
        if self.fields().is_approximate {
            self.convert_to_accurate_double();
        }
    }

    fn multiply_by(&mut self, multiplicand: &BigDecimal) -> Result<()> {
        if DecimalQuantity::is_zeroish(self) {
            return Ok(());
        }
        let mut temp = self.to_big_decimal()?;
        temp = temp.multiply(multiplicand)?;
        self.set_to_big_decimal(&temp)
    }

    fn negate(&mut self) {
        self.fields_mut().flags ^= NEGATIVE_FLAG;
    }

    fn adjust_magnitude(&mut self, delta: i32) -> Result<()> {
        let f = *self.fields();
        if f.precision != 0 {
            // TODO: Math.addExact() requires API level 24
            let scale = Utility::add_exact(f.scale, delta)?;
            self.fields_mut().scale = scale;
            let orig_delta = Utility::add_exact(f.orig_delta, delta)?;
            self.fields_mut().orig_delta = orig_delta;
            // Make sure that precision + scale won't overflow, either
            Utility::add_exact(scale, f.precision)?;
        }
        Ok(())
    }

    fn get_magnitude(&self) -> Result<i32> {
        let f = self.fields();
        if f.precision == 0 {
            Err(arithmetic("Magnitude is not well-defined for zero"))
        } else {
            Ok(f.scale + f.precision - 1)
        }
    }

    fn get_exponent(&self) -> i32 {
        self.fields().exponent
    }

    fn adjust_exponent(&mut self, delta: i32) {
        let f = self.fields_mut();
        f.exponent = f.exponent.wrapping_add(delta);
    }

    fn reset_exponent(&mut self) -> Result<()> {
        let exponent = self.fields().exponent;
        self.adjust_magnitude(exponent)?;
        self.fields_mut().exponent = 0;
        Ok(())
    }

    fn is_zeroish(&self) -> bool {
        self.fields().precision == 0
    }

    fn is_negative(&self) -> bool {
        (self.fields().flags & NEGATIVE_FLAG) != 0
    }

    fn signum(&self) -> Signum {
        let is_zero = DecimalQuantity::is_zeroish(self) && !IFixedDecimal::is_infinite(self);
        let is_neg = DecimalQuantity::is_negative(self);
        if is_zero && is_neg {
            Signum::NegZero
        } else if is_zero {
            Signum::PosZero
        } else if is_neg {
            Signum::Neg
        } else {
            Signum::Pos
        }
    }

    fn to_double(&self) -> f64 {
        // If this assertion fails, you need to call roundToInfinity() or some
        // other rounding method. See the comment at the top of this file
        // explaining the "isApproximate" field.
        debug_assert!(!self.fields().is_approximate);

        if IFixedDecimal::is_nan(self) {
            return f64::NAN;
        } else if IFixedDecimal::is_infinite(self) {
            return if DecimalQuantity::is_negative(self) { f64::NEG_INFINITY } else { f64::INFINITY };
        }

        // Double.parseDouble is correctly rounded, as Rust's f64 parsing is.
        let sb = self.to_scientific_string();
        sb.parse::<f64>().expect("toScientificString renders a valid double literal")
    }

    fn to_big_decimal(&mut self) -> Result<BigDecimal> {
        if self.fields().is_approximate {
            // Converts the number to exact BCD.
            self.convert_to_accurate_double();
        }
        self.bcd_to_big_decimal()
    }

    fn to_long(&self, truncate_if_overflow: bool) -> i64 {
        // NOTE: Call sites should be guarded by fitsInLong(), like this:
        // if (dq.fitsInLong()) { /* use dq.toLong() */ } else { /* use some
        // fallback */ }
        // Fallback behavior upon truncateIfOverflow is to truncate at 17
        // digits.
        debug_assert!(truncate_if_overflow || self.fits_in_long());
        let f = *self.fields();
        let mut result: i64 = 0;
        let mut upper_magnitude = f.exponent.wrapping_add(f.scale).wrapping_add(f.precision).wrapping_sub(1);
        if truncate_if_overflow {
            upper_magnitude = upper_magnitude.min(17);
        }
        let mut magnitude = upper_magnitude;
        while magnitude >= 0 {
            result = result
                .wrapping_mul(10)
                .wrapping_add(self.get_digit_pos(magnitude - f.scale - f.exponent) as i64);
            magnitude -= 1;
        }
        if (f.flags & NEGATIVE_FLAG) != 0 {
            result = result.wrapping_neg();
        }
        result
    }

    fn set_to_big_decimal(&mut self, n: &BigDecimal) -> Result<()> {
        self.set_bcd_to_zero();
        self.fields_mut().flags = 0;
        let mut n = n.clone();
        if n.signum() == -1 {
            self.fields_mut().flags |= NEGATIVE_FLAG;
            n = n.negate();
        }
        if n.signum() != 0 {
            self.set_to_big_decimal_internal(&n)?;
            self.compact();
        }
        Ok(())
    }

    fn max_representable_digits(&self) -> i32 {
        self.bcd_max_representable_digits()
    }

    fn get_standard_plural(&self, rules: Option<&PluralRules>) -> StandardPlural {
        match rules {
            // Fail gracefully if the user didn't provide a PluralRules
            None => StandardPlural::Other,
            Some(rules) => {
                let rule_string = rules.select(self);
                StandardPlural::or_other_from_string(&rule_string)
            }
        }
    }

    fn get_digit(&self, magnitude: i32) -> i8 {
        // If this assertion fails, you need to call roundToInfinity() or some
        // other rounding method. See the comment at the top of this file
        // explaining the "isApproximate" field.
        debug_assert!(!self.fields().is_approximate);

        self.get_digit_pos(magnitude.wrapping_sub(self.fields().scale))
    }

    fn get_upper_display_magnitude(&self) -> i32 {
        // If this assertion fails, you need to call roundToInfinity() or some
        // other rounding method. See the comment at the top of this file
        // explaining the "isApproximate" field.
        debug_assert!(!self.fields().is_approximate);

        let f = self.fields();
        let magnitude = f.scale + f.precision;
        let result = if f.l_req_pos > magnitude { f.l_req_pos } else { magnitude };
        result - 1
    }

    fn get_lower_display_magnitude(&self) -> i32 {
        // If this assertion fails, you need to call roundToInfinity() or some
        // other rounding method. See the comment at the top of this file
        // explaining the "isApproximate" field.
        debug_assert!(!self.fields().is_approximate);

        let f = self.fields();
        let magnitude = f.scale;
        if f.r_req_pos < magnitude {
            f.r_req_pos
        } else {
            magnitude
        }
    }

    fn to_plain_string(&self) -> String {
        let mut sb = String::new();
        self.to_plain_string_into(&mut sb);
        sb
    }

    fn to_exponent_string(&self) -> String {
        let mut sb = String::new();
        self.to_exponent_string_into(&mut sb);
        sb
    }

    fn create_copy(&self) -> Box<dyn DecimalQuantity> {
        self.bcd_create_copy()
    }

    fn copy_from(&mut self, other: &dyn DecimalQuantity) {
        self.copy_bcd_from(other);
        let other = other.as_any().downcast_ref::<T>().unwrap_or_else(|| {
            panic!(
                "java.lang.ClassCastException: class cannot be cast to class {}",
                std::any::type_name::<T>()
            )
        });
        let o = *other.fields();
        let f = self.fields_mut();
        f.l_req_pos = o.l_req_pos;
        f.r_req_pos = o.r_req_pos;
        f.scale = o.scale;
        f.precision = o.precision;
        f.flags = o.flags;
        f.orig_double = o.orig_double;
        f.orig_delta = o.orig_delta;
        f.is_approximate = o.is_approximate;
        f.exponent = o.exponent;
    }

    fn get_position_fingerprint(&self) -> i64 {
        let f = self.fields();
        let mut fingerprint: i64 = 0;
        // `lReqPos << 16` is an int shift, widened afterwards.
        fingerprint ^= f.l_req_pos.wrapping_shl(16) as i64;
        fingerprint ^= (f.r_req_pos as i64) << 32;
        fingerprint
    }

    fn populate_u_field_position(&self, fp: &mut FieldPosition) {
        // `fp instanceof UFieldPosition`
        if let Some(ufp) = UFieldPosition::of_mut(fp) {
            ufp.set_fraction_digits(
                self.get_plural_operand(Operand::V) as i32,
                self.get_plural_operand(Operand::F) as i64,
            );
        }
    }

    fn equals(&self, other: &dyn DecimalQuantity) -> bool {
        if std::ptr::eq(self as *const T as *const (), other.as_any() as *const dyn Any as *const ()) {
            return true;
        }
        let other = match other.as_any().downcast_ref::<T>() {
            Some(o) => o,
            None => return false,
        };

        let f = self.fields();
        let o = other.fields();
        let basic_equals = f.scale == o.scale
            && f.precision == o.precision
            && f.flags == o.flags
            && f.l_req_pos == o.l_req_pos
            && f.r_req_pos == o.r_req_pos
            && f.is_approximate == o.is_approximate;
        if !basic_equals {
            return false;
        }

        if f.precision == 0 {
            true
        } else if f.is_approximate {
            f.orig_double == o.orig_double && f.orig_delta == o.orig_delta
        } else {
            let mut m = DecimalQuantity::get_upper_display_magnitude(self);
            while m >= DecimalQuantity::get_lower_display_magnitude(self) {
                if DecimalQuantity::get_digit(self, m) != DecimalQuantity::get_digit(other, m) {
                    return false;
                }
                m -= 1;
            }
            true
        }
    }

    fn to_java_string(&self) -> String {
        self.bcd_to_string()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
