// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.number.Precision`.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, LazyLock};

use rucene::error::{LuceneError, Result};

use crate::jdk::math::{BigDecimal, MathContext};
use crate::jdk::units;
use crate::number::currency_precision::CurrencyPrecision;
use crate::number::fraction_precision::FractionPrecision;
use crate::number::number_formatter::{RoundingPriority, TrailingZeroDisplay};
use crate::r#impl::number::decimal_quantity::DecimalQuantity;
use crate::r#impl::number::multiplier_producer::MultiplierProducer;
use crate::r#impl::number::rounding_utils::{RoundingUtils, DEFAULT_MATH_CONTEXT_UNLIMITED};
use crate::text::plural_rules::Operand;
use crate::util::currency::{Currency, CurrencyUsage};

/// The concrete Java class of a `Precision` object, with its fields.
#[derive(Debug, Clone)]
pub(crate) enum PrecisionKind {
    /// `Precision.BogusRounder`.
    Bogus,
    /// `Precision.InfiniteRounderImpl`.
    Infinite,
    /// `Precision.FractionRounderImpl` (a `FractionPrecision`).
    Fraction { min_frac: i32, max_frac: i32 },
    /// `Precision.SignificantRounderImpl`.
    Significant { min_sig: i32, max_sig: i32 },
    /// `Precision.FracSigRounderImpl`.
    FracSig { min_frac: i32, max_frac: i32, min_sig: i32, max_sig: i32, priority: RoundingPriority, retain: bool },
    /// `Precision.IncrementRounderImpl`.
    Increment { increment: BigDecimal },
    /// `Precision.IncrementOneRounderImpl` (extends `IncrementRounderImpl`).
    IncrementOne { increment: BigDecimal, min_frac: i32, max_frac: i32 },
    /// `Precision.IncrementFiveRounderImpl` (extends `IncrementRounderImpl`).
    IncrementFive { increment: BigDecimal, min_frac: i32, max_frac: i32 },
    /// `Precision.CurrencyRounderImpl` (a `CurrencyPrecision`).
    Currency { usage: CurrencyUsage },
}

impl PrecisionKind {
    /// The Java binary class name.
    fn java_class(&self) -> &'static str {
        match self {
            PrecisionKind::Bogus => "com.ibm.icu.number.Precision$BogusRounder",
            PrecisionKind::Infinite => "com.ibm.icu.number.Precision$InfiniteRounderImpl",
            PrecisionKind::Fraction { .. } => "com.ibm.icu.number.Precision$FractionRounderImpl",
            PrecisionKind::Significant { .. } => "com.ibm.icu.number.Precision$SignificantRounderImpl",
            PrecisionKind::FracSig { .. } => "com.ibm.icu.number.Precision$FracSigRounderImpl",
            PrecisionKind::Increment { .. } => "com.ibm.icu.number.Precision$IncrementRounderImpl",
            PrecisionKind::IncrementOne { .. } => "com.ibm.icu.number.Precision$IncrementOneRounderImpl",
            PrecisionKind::IncrementFive { .. } => "com.ibm.icu.number.Precision$IncrementFiveRounderImpl",
            PrecisionKind::Currency { .. } => "com.ibm.icu.number.Precision$CurrencyRounderImpl",
        }
    }
}

/// The mutable field `trailingZeroDisplay` (`None` for Java's `null`).
///
/// Java writes this field after construction in
/// `CurrencyPrecision.withCurrency` (`CurrencyPrecision.java:45`), also on
/// the shared constants `constructFraction` and `constructIncrement` return
/// (`FIXED_FRAC_2`, `NICKEL`, ...); the cell keeps that write visible to
/// every holder of the object, as in Java. Plain (relaxed) atomics: Java's
/// field is neither `volatile` nor synchronized.
#[derive(Debug, Default)]
pub(crate) struct TrailingZeroDisplayCell(AtomicU8);

impl TrailingZeroDisplayCell {
    fn encode(v: Option<TrailingZeroDisplay>) -> u8 {
        match v {
            None => 0,
            Some(TrailingZeroDisplay::Auto) => 1,
            Some(TrailingZeroDisplay::HideIfWhole) => 2,
        }
    }

    /// A cell holding `v`.
    pub(crate) fn new(v: Option<TrailingZeroDisplay>) -> Self {
        Self(AtomicU8::new(Self::encode(v)))
    }

    /// The field value.
    pub(crate) fn get(&self) -> Option<TrailingZeroDisplay> {
        match self.0.load(Ordering::Relaxed) {
            0 => None,
            1 => Some(TrailingZeroDisplay::Auto),
            _ => Some(TrailingZeroDisplay::HideIfWhole),
        }
    }

    /// Writes the field.
    pub(crate) fn set(&self, v: Option<TrailingZeroDisplay>) {
        self.0.store(Self::encode(v), Ordering::Relaxed);
    }
}

impl Clone for TrailingZeroDisplayCell {
    fn clone(&self) -> Self {
        Self::new(self.get())
    }
}

/// The fields of a `Precision` object.
#[derive(Debug, Clone)]
pub(crate) struct PrecisionData {
    /// `mathContext`.
    pub(crate) math_context: MathContext,
    /// `trailingZeroDisplay`.
    pub(crate) trailing_zero_display: TrailingZeroDisplayCell,
    /// The concrete class and its fields.
    pub(crate) kind: PrecisionKind,
}

/// A class that defines the rounding precision to be used when formatting
/// numbers in NumberFormatter.
///
/// To create a Precision, use one of the factory methods.
///
/// Equivalent to `com.ibm.icu.number.Precision`. Java's `Precision` objects
/// are immutable and compared by identity (`Object.equals`); this handle
/// shares one object through an `Arc` ([`ptr_eq`](Self::ptr_eq) is Java's
/// `==`). The abstract subclasses `FractionPrecision` and
/// `CurrencyPrecision` are the views [`FractionPrecision`] and
/// [`CurrencyPrecision`]; the package-private implementation classes are
/// the variants of the kind this handle holds.
#[derive(Debug, Clone)]
pub struct Precision(pub(crate) Arc<PrecisionData>);

/// `BOGUS_PRECISION`.
pub static BOGUS_PRECISION: LazyLock<Precision> = LazyLock::new(|| Precision::construct(PrecisionKind::Bogus));
/// `NONE`.
pub(crate) static NONE: LazyLock<Precision> = LazyLock::new(|| Precision::construct(PrecisionKind::Infinite));
/// `FIXED_FRAC_0`.
pub(crate) static FIXED_FRAC_0: LazyLock<Precision> =
    LazyLock::new(|| Precision::construct(PrecisionKind::Fraction { min_frac: 0, max_frac: 0 }));
/// `FIXED_FRAC_2`.
pub(crate) static FIXED_FRAC_2: LazyLock<Precision> =
    LazyLock::new(|| Precision::construct(PrecisionKind::Fraction { min_frac: 2, max_frac: 2 }));
/// `DEFAULT_MAX_FRAC_6`.
pub(crate) static DEFAULT_MAX_FRAC_6: LazyLock<Precision> =
    LazyLock::new(|| Precision::construct(PrecisionKind::Fraction { min_frac: 0, max_frac: 6 }));
/// `FIXED_SIG_2`.
pub(crate) static FIXED_SIG_2: LazyLock<Precision> =
    LazyLock::new(|| Precision::construct(PrecisionKind::Significant { min_sig: 2, max_sig: 2 }));
/// `FIXED_SIG_3`.
pub(crate) static FIXED_SIG_3: LazyLock<Precision> =
    LazyLock::new(|| Precision::construct(PrecisionKind::Significant { min_sig: 3, max_sig: 3 }));
/// `RANGE_SIG_2_3`.
pub(crate) static RANGE_SIG_2_3: LazyLock<Precision> =
    LazyLock::new(|| Precision::construct(PrecisionKind::Significant { min_sig: 2, max_sig: 3 }));
/// `COMPACT_STRATEGY`.
pub(crate) static COMPACT_STRATEGY: LazyLock<Precision> = LazyLock::new(|| {
    Precision::construct(PrecisionKind::FracSig {
        min_frac: 0,
        max_frac: 0,
        min_sig: 1,
        max_sig: 2,
        priority: RoundingPriority::Relaxed,
        retain: false,
    })
});
/// `NICKEL`.
pub(crate) static NICKEL: LazyLock<Precision> = LazyLock::new(|| {
    let increment = BigDecimal::from_string(&units("0.05")).expect("valid decimal literal");
    Precision::construct(PrecisionKind::IncrementFive { increment, min_frac: 2, max_frac: 2 })
});
/// `MONETARY_STANDARD`.
pub(crate) static MONETARY_STANDARD: LazyLock<Precision> =
    LazyLock::new(|| Precision::construct(PrecisionKind::Currency { usage: CurrencyUsage::Standard }));
/// `MONETARY_CASH`.
pub(crate) static MONETARY_CASH: LazyLock<Precision> =
    LazyLock::new(|| Precision::construct(PrecisionKind::Currency { usage: CurrencyUsage::Cash }));

/// The message of the fraction-length range checks.
fn fraction_length_message() -> String {
    format!("Fraction length must be between 0 and {} (inclusive)", RoundingUtils::MAX_INT_FRAC_SIG)
}

/// The message of the significant-digits range checks.
pub(crate) fn significant_digits_message() -> String {
    format!("Significant digits must be between 1 and {} (inclusive)", RoundingUtils::MAX_INT_FRAC_SIG)
}

impl Precision {
    /// `Precision()` (the package-private constructor, which sets
    /// `mathContext` to `DEFAULT_MATH_CONTEXT_UNLIMITED`) of the given
    /// concrete class.
    pub(crate) fn construct(kind: PrecisionKind) -> Precision {
        Precision(Arc::new(PrecisionData {
            math_context: *DEFAULT_MATH_CONTEXT_UNLIMITED,
            trailing_zero_display: TrailingZeroDisplayCell::new(None),
            kind,
        }))
    }

    /// The concrete class and its fields.
    pub(crate) fn kind(&self) -> &PrecisionKind {
        &self.0.kind
    }

    /// `mathContext`.
    pub(crate) fn math_context(&self) -> &MathContext {
        &self.0.math_context
    }

    /// `trailingZeroDisplay`.
    pub(crate) fn trailing_zero_display_value(&self) -> Option<TrailingZeroDisplay> {
        self.0.trailing_zero_display.get()
    }

    /// Whether two handles are the same Java object (`==`, and
    /// `Object.equals`).
    pub fn ptr_eq(&self, other: &Precision) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// `Object.hashCode()` (identity): see
    /// `java_identity_hash_code_stand_in`.
    pub fn hash_code(&self) -> i32 {
        rucene::util::java_collections::java_identity_hash_code_stand_in(self.0.kind.java_class())
    }

    /// `getClass().getName()`.
    pub fn java_class_name(&self) -> &'static str {
        self.0.kind.java_class()
    }

    /// Show all available digits to full precision. Equivalent to
    /// `unlimited()`.
    pub fn unlimited() -> Precision {
        Self::construct_infinite()
    }

    /// Show numbers rounded if necessary to the nearest integer. Equivalent
    /// to `integer()`.
    pub fn integer() -> FractionPrecision {
        Self::construct_fraction(0, 0)
    }

    /// Show numbers rounded if necessary to a certain number of fraction
    /// places, padding with zeros. Equivalent to `fixedFraction(int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the argument is outside `0..=999`.
    pub fn fixed_fraction(min_max_fraction_places: i32) -> Result<FractionPrecision> {
        if (0..=RoundingUtils::MAX_INT_FRAC_SIG).contains(&min_max_fraction_places) {
            Ok(Self::construct_fraction(min_max_fraction_places, min_max_fraction_places))
        } else {
            Err(LuceneError::illegal_argument(fraction_length_message()))
        }
    }

    /// Always show at least a certain number of fraction places. Equivalent
    /// to `minFraction(int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the argument is outside `0..=999`.
    pub fn min_fraction(min_fraction_places: i32) -> Result<FractionPrecision> {
        if (0..=RoundingUtils::MAX_INT_FRAC_SIG).contains(&min_fraction_places) {
            Ok(Self::construct_fraction(min_fraction_places, -1))
        } else {
            Err(LuceneError::illegal_argument(fraction_length_message()))
        }
    }

    /// Show numbers rounded if necessary to a certain number of fraction
    /// places. Equivalent to `maxFraction(int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the argument is outside `0..=999`.
    pub fn max_fraction(max_fraction_places: i32) -> Result<FractionPrecision> {
        if (0..=RoundingUtils::MAX_INT_FRAC_SIG).contains(&max_fraction_places) {
            Ok(Self::construct_fraction(0, max_fraction_places))
        } else {
            Err(LuceneError::illegal_argument(fraction_length_message()))
        }
    }

    /// Show numbers rounded if necessary to a certain number of fraction
    /// places, always showing at least a certain number. Equivalent to
    /// `minMaxFraction(int, int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an invalid range.
    pub fn min_max_fraction(min_fraction_places: i32, max_fraction_places: i32) -> Result<FractionPrecision> {
        if min_fraction_places >= 0
            && max_fraction_places <= RoundingUtils::MAX_INT_FRAC_SIG
            && min_fraction_places <= max_fraction_places
        {
            Ok(Self::construct_fraction(min_fraction_places, max_fraction_places))
        } else {
            Err(LuceneError::illegal_argument(fraction_length_message()))
        }
    }

    /// Show numbers rounded if necessary to a certain number of significant
    /// digits or significant figures. Equivalent to
    /// `fixedSignificantDigits(int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the argument is outside `1..=999`.
    pub fn fixed_significant_digits(min_max_significant_digits: i32) -> Result<Precision> {
        if (1..=RoundingUtils::MAX_INT_FRAC_SIG).contains(&min_max_significant_digits) {
            Ok(Self::construct_significant(min_max_significant_digits, min_max_significant_digits))
        } else {
            Err(LuceneError::illegal_argument(significant_digits_message()))
        }
    }

    /// Always show at least a certain number of significant digits.
    /// Equivalent to `minSignificantDigits(int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the argument is outside `1..=999`.
    pub fn min_significant_digits(min_significant_digits: i32) -> Result<Precision> {
        if (1..=RoundingUtils::MAX_INT_FRAC_SIG).contains(&min_significant_digits) {
            Ok(Self::construct_significant(min_significant_digits, -1))
        } else {
            Err(LuceneError::illegal_argument(significant_digits_message()))
        }
    }

    /// Show numbers rounded if necessary to a certain number of significant
    /// digits. Equivalent to `maxSignificantDigits(int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the argument is outside `1..=999`.
    pub fn max_significant_digits(max_significant_digits: i32) -> Result<Precision> {
        if (1..=RoundingUtils::MAX_INT_FRAC_SIG).contains(&max_significant_digits) {
            Ok(Self::construct_significant(1, max_significant_digits))
        } else {
            Err(LuceneError::illegal_argument(significant_digits_message()))
        }
    }

    /// Show numbers rounded if necessary to a certain number of significant
    /// digits; in addition, always show at least a certain number.
    /// Equivalent to `minMaxSignificantDigits(int, int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an invalid range.
    pub fn min_max_significant_digits(min_significant_digits: i32, max_significant_digits: i32) -> Result<Precision> {
        if min_significant_digits >= 1
            && max_significant_digits <= RoundingUtils::MAX_INT_FRAC_SIG
            && min_significant_digits <= max_significant_digits
        {
            Ok(Self::construct_significant(min_significant_digits, max_significant_digits))
        } else {
            Err(LuceneError::illegal_argument(significant_digits_message()))
        }
    }

    /// Show numbers rounded if necessary to the closest multiple of a
    /// certain rounding increment. Equivalent to `increment(BigDecimal)`
    /// (`None` for Java's `null`).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Rounding increment must be positive and
    /// non-null")`, and the arithmetic errors of the increment analysis.
    pub fn increment(rounding_increment: Option<&BigDecimal>) -> Result<Precision> {
        match rounding_increment {
            Some(inc) if inc.compare_to(&BigDecimal::zero()) > 0 => Self::construct_increment(inc),
            _ => Err(LuceneError::illegal_argument("Rounding increment must be positive and non-null")),
        }
    }

    /// Show numbers rounded and padded according to the rules for the
    /// currency unit. Equivalent to `currency(CurrencyUsage)` (`None` for
    /// Java's `null`).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("CurrencyUsage must be non-null")`.
    pub fn currency(currency_usage: Option<CurrencyUsage>) -> Result<CurrencyPrecision> {
        match currency_usage {
            Some(usage) => Ok(Self::construct_currency(usage)),
            None => Err(LuceneError::illegal_argument("CurrencyUsage must be non-null")),
        }
    }

    /// Configure how trailing zeros are displayed on numbers. Equivalent to
    /// `trailingZeroDisplay(TrailingZeroDisplay)` (`None` for Java's
    /// `null`).
    pub fn trailing_zero_display(&self, trailing_zero_display: Option<TrailingZeroDisplay>) -> Precision {
        let result = self.create_copy_data();
        result.trailing_zero_display.set(trailing_zero_display);
        Precision(Arc::new(result))
    }

    /// Equivalent to `withMode(MathContext)` (internal): this object when
    /// the math context is equal, else a copy with it.
    pub fn with_mode(&self, math_context: &MathContext) -> Precision {
        if self.0.math_context.java_equals(math_context) {
            return self.clone();
        }
        let mut other = self.create_copy_data();
        other.math_context = *math_context;
        Precision(Arc::new(other))
    }

    /// `createCopy()` with `createCopyHelper`: the same class and fields,
    /// with this object's `mathContext` and `trailingZeroDisplay`.
    fn create_copy_data(&self) -> PrecisionData {
        PrecisionData {
            math_context: self.0.math_context,
            trailing_zero_display: self.0.trailing_zero_display.clone(),
            kind: self.0.kind.clone(),
        }
    }

    /// Equivalent to `apply(DecimalQuantity)` (internal).
    ///
    /// # Errors
    ///
    /// The `ArithmeticException` of `RoundingMode.UNNECESSARY` rounding.
    ///
    /// # Panics
    ///
    /// Java's `AssertionError` for a `BogusRounder` ("BogusRounder must not
    /// be applied") and a `CurrencyRounderImpl`, which must be resolved
    /// with a currency first.
    pub fn apply(&self, value: &mut dyn DecimalQuantity) -> Result<()> {
        let mc = &self.0.math_context;
        match &self.0.kind {
            PrecisionKind::Bogus => panic!("java.lang.AssertionError: BogusRounder must not be applied"),
            PrecisionKind::Infinite => {
                value.round_to_infinity();
                self.set_resolved_min_fraction(value, 0);
            }
            PrecisionKind::Fraction { min_frac, max_frac } => {
                value.round_to_magnitude(get_rounding_magnitude_fraction(*max_frac), mc)?;
                self.set_resolved_min_fraction(value, 0.max(-get_display_magnitude_fraction(*min_frac)));
            }
            PrecisionKind::Significant { min_sig, max_sig } => {
                let mag = get_rounding_magnitude_significant(value, *max_sig)?;
                value.round_to_magnitude(mag, mc)?;
                let disp = get_display_magnitude_significant(value, *min_sig)?;
                self.set_resolved_min_fraction(value, 0.max(-disp));
                // Make sure that digits are displayed on zero.
                if value.is_zeroish() && *min_sig > 0 {
                    value.set_min_integer(1);
                }
            }
            PrecisionKind::FracSig { min_frac, max_frac, min_sig, max_sig, priority, retain } => {
                let rounding_mag1 = get_rounding_magnitude_fraction(*max_frac);
                let mut rounding_mag2 = get_rounding_magnitude_significant(value, *max_sig)?;
                let rounding_mag = if *priority == RoundingPriority::Relaxed {
                    rounding_mag1.min(rounding_mag2)
                } else {
                    rounding_mag1.max(rounding_mag2)
                };
                if !value.is_zeroish() {
                    let upper_mag = value.get_magnitude()?;
                    value.round_to_magnitude(rounding_mag, mc)?;
                    if !value.is_zeroish() && value.get_magnitude()? != upper_mag && rounding_mag1 == rounding_mag2 {
                        // roundingMag2 needs to be the magnitude after rounding
                        rounding_mag2 += 1;
                    }
                }

                let display_mag1 = get_display_magnitude_fraction(*min_frac);
                let display_mag2 = get_display_magnitude_significant(value, *min_sig)?;
                let display_mag = if *retain {
                    // withMinDigits + withMaxDigits
                    display_mag1.min(display_mag2)
                } else if *priority == RoundingPriority::Relaxed {
                    if rounding_mag2 <= rounding_mag1 {
                        display_mag2
                    } else {
                        display_mag1
                    }
                } else {
                    debug_assert!(*priority == RoundingPriority::Strict);
                    if rounding_mag2 <= rounding_mag1 {
                        display_mag1
                    } else {
                        display_mag2
                    }
                };
                self.set_resolved_min_fraction(value, 0.max(-display_mag));
            }
            PrecisionKind::Increment { increment } => {
                value.round_to_increment(increment, mc)?;
                self.set_resolved_min_fraction(value, 0.max(increment.scale()));
            }
            PrecisionKind::IncrementOne { min_frac, max_frac, .. } => {
                value.round_to_magnitude(-max_frac, mc)?;
                self.set_resolved_min_fraction(value, *min_frac);
            }
            PrecisionKind::IncrementFive { min_frac, max_frac, .. } => {
                value.round_to_nickel(-max_frac, mc)?;
                self.set_resolved_min_fraction(value, *min_frac);
            }
            PrecisionKind::Currency { .. } => panic!("java.lang.AssertionError"),
        }
        Ok(())
    }

    /// `SignificantRounderImpl.apply(DecimalQuantity quantity, int minInt)`:
    /// version of apply that obeys minInt constraints. Used for scientific
    /// notation compatibility mode.
    pub(crate) fn apply_significant_min_int(&self, quantity: &mut dyn DecimalQuantity, min_int: i32) {
        debug_assert!(quantity.is_zeroish());
        if let PrecisionKind::Significant { min_sig, .. } = &self.0.kind {
            self.set_resolved_min_fraction(quantity, min_sig - min_int);
        }
    }

    /// `constructInfinite()`.
    pub(crate) fn construct_infinite() -> Precision {
        NONE.clone()
    }

    /// `constructFraction(int, int)`.
    pub(crate) fn construct_fraction(min_frac: i32, max_frac: i32) -> FractionPrecision {
        let p = if min_frac == 0 && max_frac == 0 {
            FIXED_FRAC_0.clone()
        } else if min_frac == 2 && max_frac == 2 {
            FIXED_FRAC_2.clone()
        } else if min_frac == 0 && max_frac == 6 {
            DEFAULT_MAX_FRAC_6.clone()
        } else {
            Precision::construct(PrecisionKind::Fraction { min_frac, max_frac })
        };
        FractionPrecision::cast(p)
    }

    /// `constructSignificant(int, int)`.
    pub(crate) fn construct_significant(min_sig: i32, max_sig: i32) -> Precision {
        if min_sig == 2 && max_sig == 2 {
            FIXED_SIG_2.clone()
        } else if min_sig == 3 && max_sig == 3 {
            FIXED_SIG_3.clone()
        } else if min_sig == 2 && max_sig == 3 {
            RANGE_SIG_2_3.clone()
        } else {
            Precision::construct(PrecisionKind::Significant { min_sig, max_sig })
        }
    }

    /// `constructFractionSignificant(FractionPrecision, int, int,
    /// RoundingPriority, boolean)`.
    pub(crate) fn construct_fraction_significant(
        base: &FractionPrecision,
        min_sig: i32,
        max_sig: i32,
        priority: RoundingPriority,
        retain: bool,
    ) -> Precision {
        let (min_frac, max_frac) = match base.kind() {
            PrecisionKind::Fraction { min_frac, max_frac } => (*min_frac, *max_frac),
            _ => panic!("java.lang.AssertionError"),
        };
        let return_value = if min_frac == 0
            && max_frac == 0
            && min_sig == 1
            && max_sig == 2
            && priority == RoundingPriority::Relaxed
            && !retain
        {
            COMPACT_STRATEGY.clone()
        } else {
            Precision::construct(PrecisionKind::FracSig { min_frac, max_frac, min_sig, max_sig, priority, retain })
        };
        return_value.with_mode(base.math_context())
    }

    /// `constructIncrement(BigDecimal)`.
    ///
    /// # Errors
    ///
    /// The arithmetic errors of `stripTrailingZeros`.
    pub(crate) fn construct_increment(increment: &BigDecimal) -> Result<Precision> {
        // NOTE: .equals() is what we want, not .compareTo()
        if let PrecisionKind::IncrementFive { increment: nickel, .. } = NICKEL.kind() {
            if increment.java_equals(nickel) {
                return Ok(NICKEL.clone());
            }
        }

        let reduced = increment.strip_trailing_zeros()?;
        if reduced.precision() == 1 {
            let min_frac = increment.scale();
            let max_frac = reduced.scale();
            let digit = reduced.unscaled_value();
            if digit.int_value() == 1 {
                return Ok(Precision::construct(PrecisionKind::IncrementOne {
                    increment: increment.clone(),
                    min_frac,
                    max_frac,
                }));
            } else if digit.int_value() == 5 {
                return Ok(Precision::construct(PrecisionKind::IncrementFive {
                    increment: increment.clone(),
                    min_frac,
                    max_frac,
                }));
            }
        }
        Ok(Precision::construct(PrecisionKind::Increment { increment: increment.clone() }))
    }

    /// `constructCurrency(CurrencyUsage)`.
    pub(crate) fn construct_currency(usage: CurrencyUsage) -> CurrencyPrecision {
        match usage {
            CurrencyUsage::Standard => CurrencyPrecision::cast(MONETARY_STANDARD.clone()),
            CurrencyUsage::Cash => CurrencyPrecision::cast(MONETARY_CASH.clone()),
        }
    }

    /// `constructFromCurrency(CurrencyPrecision, Currency)`.
    ///
    /// # Errors
    ///
    /// The arithmetic errors of the increment analysis.
    pub(crate) fn construct_from_currency(base: &CurrencyPrecision, currency: &Currency) -> Result<Precision> {
        let usage = match base.kind() {
            PrecisionKind::Currency { usage } => *usage,
            _ => panic!("java.lang.AssertionError"),
        };
        let increment_double = currency.get_rounding_increment_usage(usage)?;
        let return_value = if increment_double != 0.0 {
            let increment = BigDecimal::value_of_double(increment_double)?;
            Self::construct_increment(&increment)?
        } else {
            let min_max_frac = currency.get_default_fraction_digits_usage(usage)?;
            Self::construct_fraction(min_max_frac, min_max_frac).into_precision()
        };
        Ok(return_value.with_mode(base.math_context()))
    }

    /// `withLocaleData(Currency)`: if this is a `CurrencyPrecision`, its
    /// resolution with the currency; else this object.
    ///
    /// # Errors
    ///
    /// Those of [`CurrencyPrecision::with_currency`].
    pub(crate) fn with_locale_data(&self, currency: &Currency) -> Result<Precision> {
        match CurrencyPrecision::try_cast(self) {
            Some(cp) => cp.with_currency(Some(currency)),
            None => Ok(self.clone()),
        }
    }

    /// `chooseMultiplierAndApply(DecimalQuantity, MultiplierProducer)`:
    /// rounding endpoint used by Engineering and Compact notation; answers
    /// the multiplier applied.
    ///
    /// # Errors
    ///
    /// Those of [`apply`](Self::apply).
    pub(crate) fn choose_multiplier_and_apply(
        &self,
        input: &mut dyn DecimalQuantity,
        producer: &dyn MultiplierProducer,
    ) -> Result<i32> {
        // Do not call this method with zero, NaN, or infinity.
        debug_assert!(!input.is_zeroish());

        // Perform the first attempt at rounding.
        let magnitude = input.get_magnitude()?;
        let multiplier = producer.get_multiplier(magnitude);
        input.adjust_magnitude(multiplier)?;
        self.apply(input)?;

        // If the number rounded to zero, exit.
        if input.is_zeroish() {
            return Ok(multiplier);
        }

        // If the new magnitude after rounding is the same as it was before
        // rounding, then we are done. This case applies to most numbers.
        if input.get_magnitude()? == magnitude + multiplier {
            return Ok(multiplier);
        }

        // If the above case DIDN'T apply, then we have a case like 99.9 ->
        // 100 or 999.9 -> 1000: The number rounded up to the next magnitude.
        // Check if the multiplier changes; if it doesn't, we are done.
        let multiplier2 = producer.get_multiplier(magnitude + 1);
        if multiplier == multiplier2 {
            return Ok(multiplier);
        }

        // We have a case like 999.9 -> 1000, where the correct output is
        // "1K", not "1000". Fix the magnitude and re-apply the rounding
        // strategy.
        input.adjust_magnitude(multiplier2 - multiplier)?;
        self.apply(input)?;
        Ok(multiplier2)
    }

    /// `setResolvedMinFraction(DecimalQuantity, int)`.
    fn set_resolved_min_fraction(&self, value: &mut dyn DecimalQuantity, resolved_min_fraction: i32) {
        let tzd = self.0.trailing_zero_display.get();
        if tzd.is_none()
            || tzd == Some(TrailingZeroDisplay::Auto)
            // PLURAL_OPERAND_T returns fraction digits as an integer
            || value.get_plural_operand(Operand::T) != 0.0
        {
            value.set_min_fraction(resolved_min_fraction);
        }
    }

    /// `BogusRounder.into(Precision)`: copies this Rounder and its settings
    /// into a new Rounder of the given class.
    ///
    /// # Panics
    ///
    /// When this object is not a `BogusRounder` (Java's
    /// `ClassCastException` at the cast that precedes the call).
    pub fn bogus_into(&self, precision: &Precision) -> Precision {
        assert!(matches!(self.0.kind, PrecisionKind::Bogus), "java.lang.ClassCastException");
        Precision(Arc::new(PrecisionData {
            math_context: self.0.math_context,
            trailing_zero_display: self.0.trailing_zero_display.clone(),
            kind: precision.0.kind.clone(),
        }))
    }

    /// Whether this is a `BogusRounder` (`instanceof
    /// Precision.BogusRounder`).
    pub fn is_bogus(&self) -> bool {
        matches!(self.0.kind, PrecisionKind::Bogus)
    }
}

/// `getRoundingMagnitudeFraction(int)`.
fn get_rounding_magnitude_fraction(max_frac: i32) -> i32 {
    if max_frac == -1 {
        return i32::MIN;
    }
    -max_frac
}

/// `getRoundingMagnitudeSignificant(DecimalQuantity, int)`.
fn get_rounding_magnitude_significant(value: &dyn DecimalQuantity, max_sig: i32) -> Result<i32> {
    if max_sig == -1 {
        return Ok(i32::MIN);
    }
    let magnitude = if value.is_zeroish() { 0 } else { value.get_magnitude()? };
    Ok(magnitude - max_sig + 1)
}

/// `getDisplayMagnitudeFraction(int)`.
fn get_display_magnitude_fraction(min_frac: i32) -> i32 {
    if min_frac == 0 {
        return i32::MAX;
    }
    -min_frac
}

/// `getDisplayMagnitudeSignificant(DecimalQuantity, int)`.
fn get_display_magnitude_significant(value: &dyn DecimalQuantity, min_sig: i32) -> Result<i32> {
    let magnitude = if value.is_zeroish() { 0 } else { value.get_magnitude()? };
    Ok(magnitude - min_sig + 1)
}
