// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.RoundingUtils`.

use std::sync::LazyLock;

use rucene::error::Result;

use crate::jdk::arithmetic;
use crate::jdk::math::{MathContext, RoundingMode};
use crate::number::precision::Precision;
use crate::number::scale::Scale;
use crate::r#impl::number::decimal_format_properties::DecimalFormatProperties;
use crate::r#impl::number::decimal_quantity::DecimalQuantity;
use crate::r#impl::standard_plural::StandardPlural;
use crate::text::plural_rules::PluralRules;

/// `MATH_CONTEXT_BY_ROUNDING_MODE_UNLIMITED`: one unlimited-precision
/// context per rounding mode, indexed by ordinal.
static MATH_CONTEXT_BY_ROUNDING_MODE_UNLIMITED: LazyLock<Vec<MathContext>> = LazyLock::new(|| {
    (0..RoundingMode::values().len() as i32)
        .map(|i| {
            MathContext::with_rounding_mode(
                0,
                RoundingMode::value_of(i).expect("every ordinal below values().length is a rounding mode"),
            )
            .expect("precision 0 is valid")
        })
        .collect()
});

/// `MATH_CONTEXT_BY_ROUNDING_MODE_34_DIGITS`: indexed by ordinal; every
/// element is `new MathContext(34)` (rounding mode `HALF_UP`), as the Java
/// static initializer fills it.
static MATH_CONTEXT_BY_ROUNDING_MODE_34_DIGITS: LazyLock<Vec<MathContext>> = LazyLock::new(|| {
    (0..RoundingMode::values().len())
        .map(|_| MathContext::new(34).expect("precision 34 is valid"))
        .collect()
});

/// `DEFAULT_MATH_CONTEXT_UNLIMITED`: the unlimited context of
/// [`RoundingUtils::DEFAULT_ROUNDING_MODE`].
pub static DEFAULT_MATH_CONTEXT_UNLIMITED: LazyLock<MathContext> = LazyLock::new(|| {
    MATH_CONTEXT_BY_ROUNDING_MODE_UNLIMITED[RoundingUtils::DEFAULT_ROUNDING_MODE.ordinal() as usize]
});

/// `DEFAULT_MATH_CONTEXT_34_DIGITS`: the 34-digit context at the ordinal of
/// [`RoundingUtils::DEFAULT_ROUNDING_MODE`].
pub static DEFAULT_MATH_CONTEXT_34_DIGITS: LazyLock<MathContext> = LazyLock::new(|| {
    MATH_CONTEXT_BY_ROUNDING_MODE_34_DIGITS[RoundingUtils::DEFAULT_ROUNDING_MODE.ordinal() as usize]
});

/// Rounding helpers of the number formatting pipeline.
///
/// Equivalent to `com.ibm.icu.impl.number.RoundingUtils`. Its static
/// `MathContext` constants are the statics
/// [`DEFAULT_MATH_CONTEXT_UNLIMITED`] and [`DEFAULT_MATH_CONTEXT_34_DIGITS`]
/// of this module.
pub struct RoundingUtils;

impl RoundingUtils {
    /// `java.math.BigDecimal.ROUND_UP` (the legacy rounding constants, equal to
    /// the `RoundingMode` ordinals).
    const ROUND_UP: i32 = 0;
    /// `BigDecimal.ROUND_DOWN`.
    const ROUND_DOWN: i32 = 1;
    /// `BigDecimal.ROUND_CEILING`.
    const ROUND_CEILING: i32 = 2;
    /// `BigDecimal.ROUND_FLOOR`.
    const ROUND_FLOOR: i32 = 3;
    /// `BigDecimal.ROUND_HALF_UP`.
    const ROUND_HALF_UP: i32 = 4;
    /// `BigDecimal.ROUND_HALF_DOWN`.
    const ROUND_HALF_DOWN: i32 = 5;
    /// `BigDecimal.ROUND_HALF_EVEN`.
    const ROUND_HALF_EVEN: i32 = 6;

    /// `SECTION_LOWER`.
    pub const SECTION_LOWER: i32 = 1;
    /// `SECTION_MIDPOINT`.
    pub const SECTION_MIDPOINT: i32 = 2;
    /// `SECTION_UPPER`.
    pub const SECTION_UPPER: i32 = 3;

    /// The default rounding mode.
    pub const DEFAULT_ROUNDING_MODE: RoundingMode = RoundingMode::HalfEven;

    /// The maximum number of fraction places, integer numerals, or
    /// significant digits.
    pub const MAX_INT_FRAC_SIG: i32 = 999;

    /// Converts a rounding mode and metadata about the quantity being
    /// rounded to a boolean determining whether the value should be rounded
    /// toward infinity or toward zero. `reference` renders the object for the
    /// exception message (Java's `reference.toString()`). Equivalent to
    /// `getRoundingDirection(boolean, boolean, int, int, Object)`.
    ///
    /// # Errors
    ///
    /// `ArithmeticException("Rounding is required on <reference>")` for
    /// `UNNECESSARY` and unknown modes.
    pub fn get_rounding_direction(
        is_even: bool,
        is_negative: bool,
        section: i32,
        rounding_mode: i32,
        reference: &dyn Fn() -> String,
    ) -> Result<bool> {
        match rounding_mode {
            Self::ROUND_UP => {
                // round away from zero
                return Ok(false);
            }
            Self::ROUND_DOWN => {
                // round toward zero
                return Ok(true);
            }
            Self::ROUND_CEILING => {
                // round toward positive infinity
                return Ok(is_negative);
            }
            Self::ROUND_FLOOR => {
                // round toward negative infinity
                return Ok(!is_negative);
            }
            Self::ROUND_HALF_UP => match section {
                Self::SECTION_MIDPOINT => return Ok(false),
                Self::SECTION_LOWER => return Ok(true),
                Self::SECTION_UPPER => return Ok(false),
                _ => {}
            },
            Self::ROUND_HALF_DOWN => match section {
                Self::SECTION_MIDPOINT => return Ok(true),
                Self::SECTION_LOWER => return Ok(true),
                Self::SECTION_UPPER => return Ok(false),
                _ => {}
            },
            Self::ROUND_HALF_EVEN => match section {
                Self::SECTION_MIDPOINT => return Ok(is_even),
                Self::SECTION_LOWER => return Ok(true),
                Self::SECTION_UPPER => return Ok(false),
                _ => {}
            },
            _ => {}
        }

        // Rounding mode UNNECESSARY
        Err(arithmetic(&format!("Rounding is required on {}", reference())))
    }

    /// Whether the given rounding mode's rounding boundary is at the midpoint
    /// (as opposed to the edge of the rounding interval). Equivalent to
    /// `roundsAtMidpoint(int)`.
    pub fn rounds_at_midpoint(rounding_mode: i32) -> bool {
        !matches!(
            rounding_mode,
            Self::ROUND_UP | Self::ROUND_DOWN | Self::ROUND_CEILING | Self::ROUND_FLOOR
        )
    }

    /// The math context of `properties`, or an unlimited-precision context
    /// of its rounding mode (`HALF_EVEN` when unset). Equivalent to
    /// `getMathContextOrUnlimited(DecimalFormatProperties)`.
    pub fn get_math_context_or_unlimited(properties: &DecimalFormatProperties) -> MathContext {
        match properties.get_math_context() {
            Some(math_context) => *math_context,
            None => {
                let rounding_mode = properties.get_rounding_mode().unwrap_or(RoundingMode::HalfEven);
                MATH_CONTEXT_BY_ROUNDING_MODE_UNLIMITED[rounding_mode.ordinal() as usize]
            }
        }
    }

    /// The math context of `properties`, or a 34-digit context. Equivalent
    /// to `getMathContextOr34Digits(DecimalFormatProperties)`.
    pub fn get_math_context_or_34_digits(properties: &DecimalFormatProperties) -> MathContext {
        match properties.get_math_context() {
            Some(math_context) => *math_context,
            None => {
                let rounding_mode = properties.get_rounding_mode().unwrap_or(RoundingMode::HalfEven);
                MATH_CONTEXT_BY_ROUNDING_MODE_34_DIGITS[rounding_mode.ordinal() as usize]
            }
        }
    }

    /// A math context with unlimited precision and the given rounding mode.
    /// Equivalent to `mathContextUnlimited(RoundingMode)`.
    pub fn math_context_unlimited(rounding_mode: RoundingMode) -> MathContext {
        MATH_CONTEXT_BY_ROUNDING_MODE_UNLIMITED[rounding_mode.ordinal() as usize]
    }

    /// Equivalent to `scaleFromProperties(DecimalFormatProperties)` (`None`
    /// for Java's `null`).
    ///
    /// # Errors
    ///
    /// The errors of `Scale.byBigDecimal` and `Scale.withMathContext`.
    pub fn scale_from_properties(properties: &DecimalFormatProperties) -> Result<Option<Scale>> {
        let mc = Self::get_math_context_or_34_digits(properties);
        if properties.get_magnitude_multiplier() != 0 {
            Ok(Some(Scale::power_of_ten(properties.get_magnitude_multiplier()).with_math_context(&mc)?))
        } else if let Some(multiplier) = properties.get_multiplier() {
            Ok(Some(Scale::by_big_decimal(multiplier)?.with_math_context(&mc)?))
        } else {
            Ok(None)
        }
    }

    /// Computes the plural form after copying the number and applying
    /// rounding rules. Equivalent to `getPluralSafe(Precision, PluralRules,
    /// DecimalQuantity)`.
    ///
    /// # Errors
    ///
    /// The `ArithmeticException` of the rounder (`RoundingMode.UNNECESSARY`).
    pub fn get_plural_safe(
        rounder: Option<&Precision>,
        rules: Option<&PluralRules>,
        dq: &dyn DecimalQuantity,
    ) -> Result<StandardPlural> {
        let rounder = match rounder {
            None => return Ok(dq.get_standard_plural(rules)),
            Some(r) => r,
        };
        // TODO(ICU-20500): Avoid the copy?
        let mut copy = dq.create_copy();
        rounder.apply(copy.as_mut())?;
        Ok(copy.get_standard_plural(rules))
    }
}
