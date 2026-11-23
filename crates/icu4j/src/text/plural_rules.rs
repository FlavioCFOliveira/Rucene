// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html
/*
 *******************************************************************************
 * Copyright (C) 2007-2016, International Business Machines Corporation and
 * others. All Rights Reserved.
 *******************************************************************************
 */

//! `com.ibm.icu.text.PluralRules`.

use std::fmt;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

use rucene::analysis::java_regex::JavaPattern;
use rucene::error::{LuceneError, Result};
use rucene::util::extra::ParseException;
use rucene::util::java_collections::java_identity_hash_code_stand_in;
use rucene::util::java_fmt::double_to_string;

use rucene_queryparser::jdk::locale::Locale;

use crate::jdk::math::BigDecimal;
use crate::jdk::units;
use crate::number::formatted_number::FormattedNumber;
use crate::number::formatted_number_range::FormattedNumberRange;
use crate::r#impl::locale_id_parser::java_trim;
use crate::r#impl::number::decimal_quantity::DecimalQuantity;
use crate::r#impl::number::decimal_quantity_dual_storage_bcd::DecimalQuantity_DualStorageBCD;
use crate::r#impl::number::range::standard_plural_ranges::{self, StandardPluralRanges};
use crate::r#impl::plural_rules_loader::PluralRulesLoader;
use crate::r#impl::standard_plural::StandardPlural;
use crate::text::plural_rules_serial_proxy::PluralRulesSerialProxy;
use crate::text::unicode_set::UnicodeSet;
use crate::util::output::Output;
use crate::util::u_locale::ULocale;

/// A `DecimalQuantity` shared between the sample collections of plural
/// rules and their callers: Java hands out the same mutable objects (the
/// static `NO_UNIQUE_VALUE_DECIMAL_QUANTITY`, the start and end of a sample
/// range), compared by identity in its sets; [`Arc::ptr_eq`] is that
/// identity and the lock its mutability.
pub type SharedDecimalQuantity = Arc<Mutex<Box<dyn DecimalQuantity>>>;

/// Wraps a new `DecimalQuantity` as a [`SharedDecimalQuantity`].
pub fn share(dq: Box<dyn DecimalQuantity>) -> SharedDecimalQuantity {
    Arc::new(Mutex::new(dq))
}

/// Locks a shared quantity (a poisoned lock still yields its value).
pub fn lock_dq(dq: &SharedDecimalQuantity) -> MutexGuard<'_, Box<dyn DecimalQuantity>> {
    dq.lock().unwrap_or_else(|e| e.into_inner())
}

/// Adds `dq` to an insertion-ordered set of shared quantities: Java's
/// `LinkedHashSet<DecimalQuantity>` over a class that overrides `equals`
/// but not `hashCode`, so two distinct objects land in different identity
/// buckets and only the same object is a duplicate.
fn set_add(set: &mut Vec<SharedDecimalQuantity>, dq: SharedDecimalQuantity) -> bool {
    if set.iter().any(|e| Arc::ptr_eq(e, &dq)) {
        false
    } else {
        set.push(dq);
        true
    }
}

/// `DecimalQuantity.equals` between two shared quantities (`this == other`
/// first, so one object is never locked twice).
fn dq_equals(a: &SharedDecimalQuantity, b: &SharedDecimalQuantity) -> bool {
    if Arc::ptr_eq(a, b) {
        return true;
    }
    let a = lock_dq(a);
    let b = lock_dq(b);
    a.equals(&**b)
}

/// `ALLOWED_ID`: `[a-z]`.
static ALLOWED_ID: LazyLock<UnicodeSet> = LazyLock::new(|| {
    let mut set = UnicodeSet::from_pattern(&units("[a-z]")).expect("[a-z] is a valid pattern");
    set.freeze();
    set
});

/// `CATEGORY_SEPARATOR`.
const CATEGORY_SEPARATOR: &str = ";  ";

/// `KEYWORD_ZERO`: common name for the 'zero' plural form.
pub const KEYWORD_ZERO: &str = "zero";
/// `KEYWORD_ONE`: common name for the 'singular' plural form.
pub const KEYWORD_ONE: &str = "one";
/// `KEYWORD_TWO`: common name for the 'dual' plural form.
pub const KEYWORD_TWO: &str = "two";
/// `KEYWORD_FEW`: common name for the 'paucal' or other special plural form.
pub const KEYWORD_FEW: &str = "few";
/// `KEYWORD_MANY`: common name for the arabic (11 to 99) plural form.
pub const KEYWORD_MANY: &str = "many";
/// `KEYWORD_OTHER`: common name for the default plural form.
pub const KEYWORD_OTHER: &str = "other";

/// `NO_UNIQUE_VALUE`: value returned by `getUniqueKeywordValue` when there
/// is no unique value to return.
pub const NO_UNIQUE_VALUE: f64 = -0.00123456777;

/// `NO_UNIQUE_VALUE_DECIMAL_QUANTITY`: value returned by
/// `getUniqueKeywordDecimalQuantityValue` when there is no unique value to
/// return.
pub static NO_UNIQUE_VALUE_DECIMAL_QUANTITY: LazyLock<SharedDecimalQuantity> =
    LazyLock::new(|| share(Box::new(DecimalQuantity_DualStorageBCD::from_double(-0.00123456777))));

/// `NO_CONSTRAINT`: the default constraint that is always satisfied.
static NO_CONSTRAINT: LazyLock<Arc<dyn Constraint>> = LazyLock::new(|| Arc::new(NoConstraint));

/// `DEFAULT_RULE`: a rule that always applies.
static DEFAULT_RULE: LazyLock<Rule> =
    LazyLock::new(|| Rule::new("other".to_string(), NO_CONSTRAINT.clone(), None, None));

/// `DEFAULT`: the default rules that accept any number and return
/// [`KEYWORD_OTHER`].
pub static DEFAULT: LazyLock<Arc<PluralRules>> = LazyLock::new(|| {
    let mut list = RuleList::new();
    list.add_rule(DEFAULT_RULE.clone()).expect("a single rule has no duplicate keyword");
    Arc::new(PluralRules::new(list, Some(standard_plural_ranges::DEFAULT.clone())))
});

/// `AT_SEPARATED`.
static AT_SEPARATED: LazyLock<JavaPattern> = LazyLock::new(|| compile("\\s*\\Q\\E@\\s*"));
/// `OR_SEPARATED`.
static OR_SEPARATED: LazyLock<JavaPattern> = LazyLock::new(|| compile("\\s*or\\s*"));
/// `AND_SEPARATED`.
static AND_SEPARATED: LazyLock<JavaPattern> = LazyLock::new(|| compile("\\s*and\\s*"));
/// `COMMA_SEPARATED`.
static COMMA_SEPARATED: LazyLock<JavaPattern> = LazyLock::new(|| compile("\\s*,\\s*"));
/// `DOTDOT_SEPARATED`.
#[allow(dead_code)]
static DOTDOT_SEPARATED: LazyLock<JavaPattern> = LazyLock::new(|| compile("\\s*\\Q..\\E\\s*"));
/// `TILDE_SEPARATED`.
static TILDE_SEPARATED: LazyLock<JavaPattern> = LazyLock::new(|| compile("\\s*~\\s*"));
/// `SEMI_SEPARATED`.
static SEMI_SEPARATED: LazyLock<JavaPattern> = LazyLock::new(|| compile("\\s*;\\s*"));

fn compile(regex: &str) -> JavaPattern {
    JavaPattern::compile(regex, 0).expect("the PluralRules separator patterns are valid")
}

/// `Pattern.split(CharSequence, 0)` over a Rust string (rule texts are
/// well-formed UTF-16).
fn split(pattern: &JavaPattern, input: &str) -> Vec<String> {
    pattern.split(&units(input), 0).into_iter().map(|s| String::from_utf16_lossy(&s)).collect()
}

/// `String.length()`: the UTF-16 length.
fn len16(s: &str) -> usize {
    s.encode_utf16().count()
}

/// Provides a factory for returning plural rules.
///
/// Equivalent to `PluralRules.Factory` (an abstract class; its final
/// `forLocale(ULocale)` is [`for_locale_cardinal`](Self::for_locale_cardinal)
/// and its static `getDefaultFactory()` is [`get_default_factory`]).
pub trait Factory: Send + Sync {
    /// Provides access to the predefined `PluralRules` for a given locale and
    /// the plural type. Equivalent to `forLocale(ULocale, PluralType)`.
    ///
    /// # Errors
    ///
    /// The `MissingResourceException` of the plural data.
    fn for_locale(&self, locale: &ULocale, plural_type: PluralType) -> Result<Arc<PluralRules>>;

    /// Utility for getting CARDINAL rules. Equivalent to the final
    /// `forLocale(ULocale)`.
    ///
    /// # Errors
    ///
    /// The `MissingResourceException` of the plural data.
    fn for_locale_cardinal(&self, locale: &ULocale) -> Result<Arc<PluralRules>> {
        self.for_locale(locale, PluralType::Cardinal)
    }

    /// Returns the locales for which there is plurals data. Equivalent to
    /// `getAvailableULocales()`.
    ///
    /// # Errors
    ///
    /// The errors of `ULocale.createCanonical`.
    fn get_available_u_locales(&self) -> Result<Vec<ULocale>>;

    /// Returns the 'functionally equivalent' locale with respect to plural
    /// rules. Equivalent to `getFunctionalEquivalent(ULocale, boolean[])`.
    ///
    /// # Errors
    ///
    /// The errors of locale canonicalization.
    fn get_functional_equivalent(&self, locale: &ULocale, is_available: Option<&mut [bool]>) -> Result<ULocale>;

    /// Returns whether or not there are overrides. Equivalent to
    /// `hasOverride(ULocale)`.
    fn has_override(&self, locale: &ULocale) -> bool;
}

/// Returns the default factory. Equivalent to `Factory.getDefaultFactory()`.
pub fn get_default_factory() -> &'static PluralRulesLoader {
    &crate::r#impl::plural_rules_loader::LOADER
}

/// Type of plurals and PluralRules. Equivalent to `PluralRules.PluralType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PluralType {
    /// Plural rules for cardinal numbers: 1 file vs. 2 files.
    Cardinal,
    /// Plural rules for ordinal numbers: 1st file vs. 2nd file.
    Ordinal,
}

/// An operand of a plural rule. Equivalent to `PluralRules.Operand` (the
/// Java constant names are the lower-case [`name`](Self::name)s).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Operand {
    /// `n`: the absolute value of N.
    N,
    /// `i`: the integer value of N.
    I,
    /// `f`: all visible fraction digits as an integer, including trailing
    /// zeros.
    F,
    /// `t`: visible fraction digits as an integer, not including trailing
    /// zeros.
    T,
    /// `v`: number of visible fraction digits.
    V,
    /// `w`: number of visible fraction digits, not including trailing zeros.
    W,
    /// `e`: suppressed exponent for scientific notation.
    E,
    /// `c`: suppressed exponent for compact notation (deprecated).
    C,
    /// `j`: the integer value of N, only when there are no visible fraction
    /// digits (the plural rules syntax `j`).
    J,
}

impl Operand {
    /// The constants in declaration order (`values()`).
    pub const VALUES: [Operand; 9] =
        [Operand::N, Operand::I, Operand::F, Operand::T, Operand::V, Operand::W, Operand::E, Operand::C, Operand::J];

    /// Equivalent to `name()`.
    pub fn name(self) -> &'static str {
        match self {
            Operand::N => "n",
            Operand::I => "i",
            Operand::F => "f",
            Operand::T => "t",
            Operand::V => "v",
            Operand::W => "w",
            Operand::E => "e",
            Operand::C => "c",
            Operand::J => "j",
        }
    }

    /// Equivalent to `Operand.valueOf(String)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("No enum constant
    /// com.ibm.icu.text.PluralRules.Operand.<name>")`.
    pub fn value_of(name: &str) -> Result<Operand> {
        Self::VALUES.iter().copied().find(|o| o.name() == name).ok_or_else(|| {
            LuceneError::illegal_argument(format!("No enum constant com.ibm.icu.text.PluralRules.Operand.{name}"))
        })
    }
}

impl fmt::Display for Operand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// An interface to FixedDecimal, allowing for other implementations.
/// Equivalent to `PluralRules.IFixedDecimal`.
pub trait IFixedDecimal {
    /// Returns the value corresponding to the specified operand (n, i, f, t,
    /// v, or w). If the operand is 'n', returns a double; otherwise, returns
    /// an integer. Equivalent to `getPluralOperand(Operand)`.
    fn get_plural_operand(&self, operand: Operand) -> f64;

    /// Equivalent to `isNaN()`.
    fn is_nan(&self) -> bool;

    /// Equivalent to `isInfinite()`.
    fn is_infinite(&self) -> bool;

    /// Whether the number has no nonzero fraction digits. Equivalent to
    /// `isHasIntegerValue()`.
    fn is_has_integer_value(&self) -> bool;
}

/// `FixedDecimal.MAX`.
const FIXED_DECIMAL_MAX: i64 = 1_000_000_000_000_000_000;

/// `FixedDecimal.MAX_INTEGER_PART`.
const MAX_INTEGER_PART: i64 = 1000000000;

/// A number with its visible fraction digits, as plural rules see it.
///
/// Equivalent to `PluralRules.FixedDecimal` (a `java.lang.Number`, so it
/// also has the `intValue`/`longValue`/`floatValue`/`doubleValue` methods).
/// Java serialization is refused by `writeObject`/`readObject`
/// ([`write_object`](Self::write_object)).
#[derive(Debug, Clone, Copy)]
pub struct FixedDecimal {
    source: f64,
    visible_decimal_digit_count: i32,
    visible_decimal_digit_count_without_trailing_zeros: i32,
    decimal_digits: i64,
    decimal_digits_without_trailing_zeros: i64,
    integer_value: i64,
    has_integer_value: bool,
    is_negative: bool,
    exponent: i32,
    base_factor: i32,
}

impl FixedDecimal {
    /// Equivalent to `getSource()`.
    pub fn get_source(&self) -> f64 {
        self.source
    }

    /// Equivalent to `getVisibleDecimalDigitCount()`.
    pub fn get_visible_decimal_digit_count(&self) -> i32 {
        self.visible_decimal_digit_count
    }

    /// Equivalent to `getVisibleDecimalDigitCountWithoutTrailingZeros()`.
    pub fn get_visible_decimal_digit_count_without_trailing_zeros(&self) -> i32 {
        self.visible_decimal_digit_count_without_trailing_zeros
    }

    /// Equivalent to `getDecimalDigits()`.
    pub fn get_decimal_digits(&self) -> i64 {
        self.decimal_digits
    }

    /// Equivalent to `getDecimalDigitsWithoutTrailingZeros()`.
    pub fn get_decimal_digits_without_trailing_zeros(&self) -> i64 {
        self.decimal_digits_without_trailing_zeros
    }

    /// Equivalent to `getIntegerValue()`.
    pub fn get_integer_value(&self) -> i64 {
        self.integer_value
    }

    /// Equivalent to `isNegative()`.
    pub fn is_negative(&self) -> bool {
        self.is_negative
    }

    /// Equivalent to `getBaseFactor()`.
    pub fn get_base_factor(&self) -> i32 {
        self.base_factor
    }

    /// Equivalent to `FixedDecimal(double, int, long, int, int)`.
    pub fn new_with_compact(n: f64, v: i32, f: i64, e: i32, c: i32) -> Self {
        let is_negative = n < 0.0;
        let source = if is_negative { -n } else { n };
        let integer_value = if n > FIXED_DECIMAL_MAX as f64 { FIXED_DECIMAL_MAX } else { source as i64 };
        let mut init_exp_val = e;
        if init_exp_val == 0 {
            init_exp_val = c;
        }
        let exponent = init_exp_val;
        let has_integer_value = source == integer_value as f64;
        // check values. TODO make into unit test.
        //
        //            long visiblePower = (int) Math.pow(10.0, v);
        //            if (fractionalDigits > visiblePower) {
        //                throw new IllegalArgumentException();
        //            }
        //            double fraction = intValue + (fractionalDigits / (double) visiblePower);
        //            if (fraction != source) {
        //                double diff = Math.abs(fraction - source)/(Math.abs(fraction) + Math.abs(source));
        //                if (diff > 0.00000001d) {
        //                    throw new IllegalArgumentException();
        //                }
        //            }
        let (dwtz, vwtz) = if f == 0 {
            (0, 0)
        } else {
            let mut fdwtz = f;
            let mut trimmed_count = v;
            while (fdwtz % 10) == 0 {
                fdwtz /= 10;
                trimmed_count -= 1;
            }
            (fdwtz, trimmed_count)
        };
        Self {
            source,
            visible_decimal_digit_count: v,
            visible_decimal_digit_count_without_trailing_zeros: vwtz,
            decimal_digits: f,
            decimal_digits_without_trailing_zeros: dwtz,
            integer_value,
            has_integer_value,
            is_negative,
            exponent,
            base_factor: java_pow10_int(v),
        }
    }

    /// Equivalent to `FixedDecimal(double, int, long, int)`.
    pub fn new_with_exponent(n: f64, v: i32, f: i64, e: i32) -> Self {
        Self::new_with_compact(n, v, f, e, e)
    }

    /// Equivalent to `FixedDecimal(double, int, long)`.
    pub fn new_with_digits(n: f64, v: i32, f: i64) -> Self {
        Self::new_with_exponent(n, v, f, 0)
    }

    /// Equivalent to `createWithExponent(double, int, int)`.
    pub fn create_with_exponent(n: f64, v: i32, e: i32) -> FixedDecimal {
        FixedDecimal::new_with_exponent(n, v, Self::get_fractional_digits(n, v) as i64, e)
    }

    /// Equivalent to `FixedDecimal(double, int)`.
    pub fn new_with_visible(n: f64, v: i32) -> Self {
        Self::new_with_digits(n, v, Self::get_fractional_digits(n, v) as i64)
    }

    /// `getFractionalDigits(double, int)`.
    fn get_fractional_digits(mut n: f64, v: i32) -> i32 {
        if v == 0 {
            0
        } else {
            if n < 0.0 {
                n = -n;
            }
            let base_factor = java_pow10_int(v);
            let scaled = java_math_round(n * base_factor as f64);
            (scaled % base_factor as i64) as i32
        }
    }

    /// Equivalent to `FixedDecimal(double)`.
    pub fn from_double(n: f64) -> Self {
        Self::new_with_visible(n, Self::decimals(n))
    }

    /// Equivalent to `FixedDecimal(long)`.
    pub fn from_long(n: i64) -> Self {
        Self::new_with_visible(n as f64, 0)
    }

    /// Return a guess as to the number of decimals that would be displayed.
    /// This is only a guess; callers should always supply the decimals
    /// explicitly if possible. Currently, it is up to 6 decimals (without
    /// trailing zeros). Returns 0 for infinities and nans. Equivalent to
    /// `decimals(double)`.
    pub fn decimals(mut n: f64) -> i32 {
        // Ugly...
        if n.is_infinite() || n.is_nan() {
            return 0;
        }
        if n < 0.0 {
            n = -n;
        }
        if n == n.floor() {
            return 0;
        }
        if n < MAX_INTEGER_PART as f64 {
            let temp = ((n * 1000000.0) as i64) % 1000000;
            let mut mask = 10;
            let mut digits = 6;
            while digits > 0 {
                if (temp % mask) != 0 {
                    return digits;
                }
                mask *= 10;
                digits -= 1;
            }
            0
        } else {
            let buf: Vec<char> = java_format_e(n, 15).chars().collect();
            let e_pos = buf.iter().rposition(|&c| c == 'e').expect("%e renders an exponent") as i32;
            let mut exp_num_pos = e_pos + 1;
            if buf[exp_num_pos as usize] == '+' {
                exp_num_pos += 1;
            }
            let exponent_str: String = buf[exp_num_pos as usize..].iter().collect();
            let exponent: i32 = exponent_str.parse().expect("%e renders a decimal exponent");
            let mut num_fraction_digits = e_pos - 2 - exponent;
            if num_fraction_digits < 0 {
                return 0;
            }
            let mut i = e_pos - 1;
            while num_fraction_digits > 0 {
                if buf[i as usize] != '0' {
                    break;
                }
                num_fraction_digits -= 1;
                i -= 1;
            }
            num_fraction_digits
        }
    }

    /// Returns the operand for the plural rules syntax name. Equivalent to
    /// `getOperand(String)` (deprecated).
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of `Operand.valueOf`.
    pub fn get_operand(t: &str) -> Result<Operand> {
        Operand::value_of(t)
    }

    /// We're not going to care about NaN. Equivalent to
    /// `compareTo(FixedDecimal)` (deprecated).
    pub fn compare_to(&self, other: &FixedDecimal) -> i32 {
        if self.exponent != other.exponent {
            return if self.double_value() < other.double_value() { -1 } else { 1 };
        }
        if self.integer_value != other.integer_value {
            return if self.integer_value < other.integer_value { -1 } else { 1 };
        }
        if self.source != other.source {
            return if self.source < other.source { -1 } else { 1 };
        }
        if self.visible_decimal_digit_count != other.visible_decimal_digit_count {
            return if self.visible_decimal_digit_count < other.visible_decimal_digit_count { -1 } else { 1 };
        }
        let diff = self.decimal_digits.wrapping_sub(other.decimal_digits);
        if diff != 0 {
            return if diff < 0 { -1 } else { 1 };
        }
        0
    }

    /// Equivalent to `hashCode()` (deprecated).
    pub fn hash_code(&self) -> i32 {
        // TODO Auto-generated method stub
        let inner = self.visible_decimal_digit_count.wrapping_add(java_d2i(37.0 * self.source));
        java_l2i(self.decimal_digits.wrapping_add(37 * inner as i64))
    }

    /// Equivalent to `toString()` (deprecated):
    /// `String.format(Locale.ROOT, "%.<v>f", source)`, then `e<exponent>`
    /// when the exponent is not zero.
    pub fn to_java_string(&self) -> String {
        let base_string = java_format_f(self.source, self.visible_decimal_digit_count.max(0) as usize);
        if self.exponent != 0 {
            format!("{base_string}e{}", self.exponent)
        } else {
            base_string
        }
    }

    /// Equivalent to `hasIntegerValue()` (deprecated).
    pub fn has_integer_value(&self) -> bool {
        self.has_integer_value
    }

    /// Equivalent to `intValue()` (deprecated).
    pub fn int_value(&self) -> i32 {
        // TODO Auto-generated method stub
        self.long_value() as i32
    }

    /// Equivalent to `longValue()` (deprecated).
    pub fn long_value(&self) -> i64 {
        if self.exponent == 0 {
            self.integer_value
        } else {
            (10f64.powi(self.exponent) * self.integer_value as f64) as i64
        }
    }

    /// Equivalent to `floatValue()` (deprecated).
    pub fn float_value(&self) -> f32 {
        (self.source * 10f64.powi(self.exponent)) as f32
    }

    /// Equivalent to `doubleValue()` (deprecated).
    pub fn double_value(&self) -> f64 {
        (if self.is_negative { -self.source } else { self.source }) * 10f64.powi(self.exponent)
    }

    /// Equivalent to `writeObject(ObjectOutputStream)`.
    ///
    /// # Errors
    ///
    /// Always `NotSerializableException`.
    pub fn write_object(&self) -> Result<()> {
        Err(not_serializable())
    }

    /// Equivalent to `readObject(ObjectInputStream)`.
    ///
    /// # Errors
    ///
    /// Always `NotSerializableException`.
    pub fn read_object(&self) -> Result<()> {
        Err(not_serializable())
    }
}

impl PartialEq for FixedDecimal {
    /// Equivalent to `equals(Object)` (deprecated).
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
            && self.visible_decimal_digit_count == other.visible_decimal_digit_count
            && self.decimal_digits == other.decimal_digits
            && self.exponent == other.exponent
    }
}

impl fmt::Display for FixedDecimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_java_string())
    }
}

impl IFixedDecimal for FixedDecimal {
    fn get_plural_operand(&self, operand: Operand) -> f64 {
        match operand {
            Operand::N => {
                if self.exponent == 0 {
                    self.source
                } else {
                    self.source * 10f64.powi(self.exponent)
                }
            }
            Operand::I => self.int_value() as f64,
            Operand::F => self.decimal_digits as f64,
            Operand::T => self.decimal_digits_without_trailing_zeros as f64,
            Operand::V => self.visible_decimal_digit_count as f64,
            Operand::W => self.visible_decimal_digit_count_without_trailing_zeros as f64,
            Operand::E => self.exponent as f64,
            Operand::C => self.exponent as f64,
            _ => self.double_value(),
        }
    }

    fn is_nan(&self) -> bool {
        self.source.is_nan()
    }

    fn is_infinite(&self) -> bool {
        self.source.is_infinite()
    }

    fn is_has_integer_value(&self) -> bool {
        self.has_integer_value
    }
}

/// `new NotSerializableException()`.
fn not_serializable() -> LuceneError {
    LuceneError::unchecked_with_message("java.io.NotSerializableException", None)
}

/// `(int) Math.pow(10, v)`: Java's saturating double-to-int conversion.
fn java_pow10_int(v: i32) -> i32 {
    java_d2i(10f64.powi(v))
}

/// Java's `(int)` conversion of a double (saturating, `NaN` is 0).
fn java_d2i(d: f64) -> i32 {
    d as i32
}

/// Java's `(int)` conversion of a long (wrapping).
fn java_l2i(l: i64) -> i32 {
    l as i32
}

/// `Math.round(double)`.
fn java_math_round(a: f64) -> i64 {
    if a.is_nan() {
        return 0;
    }
    let f = a.floor();
    if a - f >= 0.5 {
        (f as i64).saturating_add(1)
    } else {
        f as i64
    }
}

/// The digits and decimal exponent of the shortest decimal that rounds to
/// the positive finite `value` (the digits of `Double.toString`): `value =
/// 0.d1d2...dn × 10^point`.
fn shortest_digits(value: f64) -> (Vec<u8>, i32) {
    let s = double_to_string(value);
    let (mantissa, exp) = match s.find('E') {
        Some(e) => (&s[..e], s[e + 1..].parse::<i32>().expect("Double.toString renders a decimal exponent")),
        None => (&s[..], 0),
    };
    let dot = mantissa.find('.').unwrap_or(mantissa.len());
    let mut digits: Vec<u8> = mantissa.bytes().filter(|b| b.is_ascii_digit()).map(|b| b - b'0').collect();
    let mut point = dot as i32 + exp;
    // Strip leading zeros (fraction-only renderings like 0.0012).
    while digits.len() > 1 && digits[0] == 0 {
        digits.remove(0);
        point -= 1;
    }
    (digits, point)
}

/// Rounds `digits` half-up to `keep` digits; answers whether a carry out of
/// the first digit happened (the digits are then `1` followed by zeros).
fn round_half_up(digits: &mut Vec<u8>, keep: usize) -> bool {
    if digits.len() <= keep {
        digits.resize(keep, 0);
        return false;
    }
    let round_up = digits[keep] >= 5;
    digits.truncate(keep);
    if round_up {
        let mut i = keep;
        loop {
            if i == 0 {
                digits.insert(0, 1);
                digits.truncate(keep.max(1));
                return true;
            }
            i -= 1;
            if digits[i] == 9 {
                digits[i] = 0;
            } else {
                digits[i] += 1;
                break;
            }
        }
    }
    false
}

/// `String.format(Locale.ROOT, "%.<prec>f", value)` for the non-negative
/// values `FixedDecimal.toString` renders: JDK 25's `Formatter` rounds the
/// digits of the shortest decimal (those of `Double.toString`) half-up.
fn java_format_f(value: f64, prec: usize) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    let negative = value.is_sign_negative() && value != 0.0;
    let v = value.abs();
    let (mut digits, mut point) = if v == 0.0 { (vec![0u8], 1) } else { shortest_digits(v) };
    // Number of digits to keep: the integer digits plus `prec` fraction
    // digits.
    let keep = point + prec as i32;
    let mut int_part = String::new();
    let mut frac_part = String::new();
    if keep < 0 {
        digits = vec![0; 0];
        point = 1;
    } else {
        let carry = round_half_up(&mut digits, keep as usize);
        if carry {
            point += 1;
            digits.push(0);
        }
    }
    for i in 0..point.max(0) {
        int_part.push((b'0' + digits.get(i as usize).copied().unwrap_or(0)) as char);
    }
    if int_part.is_empty() {
        int_part.push('0');
    }
    for k in 0..prec as i32 {
        let idx = point + k;
        let d = if idx < 0 { 0 } else { digits.get(idx as usize).copied().unwrap_or(0) };
        frac_part.push((b'0' + d) as char);
    }
    let mut out = String::new();
    if negative {
        out.push('-');
    }
    out.push_str(&int_part);
    if prec > 0 {
        out.push('.');
        out.push_str(&frac_part);
    }
    out
}

/// `String.format(Locale.ENGLISH, "%1.<prec>e", value)` for the positive
/// finite values `FixedDecimal.decimals` renders: the shortest digits
/// rounded half-up to `prec + 1` significant digits, a two-digit (at least)
/// signed exponent.
fn java_format_e(value: f64, prec: usize) -> String {
    let (mut digits, mut point) = shortest_digits(value);
    if round_half_up(&mut digits, prec + 1) {
        point += 1;
    }
    let exp = point - 1;
    let mut out = String::new();
    out.push((b'0' + digits[0]) as char);
    if prec > 0 {
        out.push('.');
        for d in &digits[1..] {
            out.push((b'0' + d) as char);
        }
    }
    out.push('e');
    out.push(if exp < 0 { '-' } else { '+' });
    out.push_str(&format!("{:02}", exp.abs()));
    out
}

/// Selection parameter for either integer-only or decimal-only. Equivalent
/// to `PluralRules.SampleType` (internal, deprecated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SampleType {
    /// `INTEGER`.
    Integer,
    /// `DECIMAL`.
    Decimal,
}

impl SampleType {
    /// Equivalent to `toString()` (the constant name).
    pub fn name(self) -> &'static str {
        match self {
            SampleType::Integer => "INTEGER",
            SampleType::Decimal => "DECIMAL",
        }
    }
}

/// A range of DecimalQuantity representing PluralRules samples that
/// includes all values with the same visibleFractionDigitCount. Equivalent
/// to `PluralRules.DecimalQuantitySamplesRange` (internal, deprecated).
#[derive(Debug, Clone)]
pub struct DecimalQuantitySamplesRange {
    /// `start`.
    pub start: SharedDecimalQuantity,
    /// `end`.
    pub end: SharedDecimalQuantity,
}

impl DecimalQuantitySamplesRange {
    /// Equivalent to `DecimalQuantitySamplesRange(DecimalQuantity,
    /// DecimalQuantity)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Ranges must have the same number of
    /// visible decimals: <start>~<end>")`.
    pub fn new(start: SharedDecimalQuantity, end: SharedDecimalQuantity) -> Result<Self> {
        let (sv, ev, ss, es) = if Arc::ptr_eq(&start, &end) {
            let s = lock_dq(&start);
            let v = s.get_plural_operand(Operand::V);
            let t = s.to_java_string();
            (v, v, t.clone(), t)
        } else {
            let s = lock_dq(&start);
            let e = lock_dq(&end);
            (s.get_plural_operand(Operand::V), e.get_plural_operand(Operand::V), s.to_java_string(), e.to_java_string())
        };
        if sv != ev {
            return Err(LuceneError::illegal_argument(format!(
                "Ranges must have the same number of visible decimals: {ss}~{es}"
            )));
        }
        Ok(Self { start, end })
    }

    /// Equivalent to `toString()`.
    pub fn to_java_string(&self) -> String {
        let start = lock_dq(&self.start).to_exponent_string();
        if Arc::ptr_eq(&self.end, &self.start) {
            start
        } else {
            format!("{start}~{}", lock_dq(&self.end).to_exponent_string())
        }
    }
}

impl fmt::Display for DecimalQuantitySamplesRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_java_string())
    }
}

/// A list of DecimalQuantity representing PluralRules samples. Equivalent to
/// `PluralRules.DecimalQuantitySamples` (internal, deprecated). Its
/// `samples` set is insertion-ordered; the ranges do not override
/// `equals`/`hashCode`, so every parsed range is a distinct element.
#[derive(Debug, Clone)]
pub struct DecimalQuantitySamples {
    /// `sampleType`.
    pub sample_type: SampleType,
    /// `samples`.
    pub samples: Vec<DecimalQuantitySamplesRange>,
    /// `bounded`.
    pub bounded: bool,
}

impl DecimalQuantitySamples {
    /// Parse a list of the form described in CLDR. The source must be
    /// trimmed. Equivalent to `parse(String)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for ill-formed samples, and the
    /// `NumberFormatException` of malformed numbers.
    pub(crate) fn parse(source: &str) -> Result<DecimalQuantitySamples> {
        let sample_type2;
        let mut bounded2 = true;
        let mut have_bound = false;
        let mut samples2: Vec<DecimalQuantitySamplesRange> = Vec::new();

        if source.starts_with("integer") {
            sample_type2 = SampleType::Integer;
        } else if source.starts_with("decimal") {
            sample_type2 = SampleType::Decimal;
        } else {
            return Err(LuceneError::illegal_argument("Samples must start with 'integer' or 'decimal'"));
        }
        let source = java_trim(substring16(source, 7));

        for range in split(&COMMA_SEPARATED, source) {
            if range == "\u{2026}" || range == "..." {
                bounded2 = false;
                have_bound = true;
                continue;
            }
            if have_bound {
                return Err(LuceneError::illegal_argument(format!("Can only have \u{2026} at the end of samples: {range}")));
            }
            let range_parts = split(&TILDE_SEPARATED, &range);
            match range_parts.len() {
                1 => {
                    let sample: SharedDecimalQuantity =
                        share(Box::new(DecimalQuantity_DualStorageBCD::from_exponent_string(&range_parts[0])?));
                    Self::check_decimal(sample_type2, &sample)?;
                    samples2.push(DecimalQuantitySamplesRange::new(sample.clone(), sample)?);
                }
                2 => {
                    let start: SharedDecimalQuantity =
                        share(Box::new(DecimalQuantity_DualStorageBCD::from_exponent_string(&range_parts[0])?));
                    let end: SharedDecimalQuantity =
                        share(Box::new(DecimalQuantity_DualStorageBCD::from_exponent_string(&range_parts[1])?));
                    Self::check_decimal(sample_type2, &start)?;
                    Self::check_decimal(sample_type2, &end)?;
                    samples2.push(DecimalQuantitySamplesRange::new(start, end)?);
                }
                _ => return Err(LuceneError::illegal_argument(format!("Ill-formed number range: {range}"))),
            }
        }
        Ok(DecimalQuantitySamples { sample_type: sample_type2, samples: samples2, bounded: bounded2 })
    }

    /// `checkDecimal(SampleType, DecimalQuantity)`.
    fn check_decimal(sample_type2: SampleType, sample: &SharedDecimalQuantity) -> Result<()> {
        let s = lock_dq(sample);
        // TODO(CLDR-15452): Remove the need for the fallback check for
        // exponent notation integers classified as "@decimal" type samples,
        // if/when changes are made to the CLDR plural rules sample format.
        if (sample_type2 == SampleType::Integer && s.get_plural_operand(Operand::V) != 0.0)
            || (sample_type2 == SampleType::Decimal
                && s.get_plural_operand(Operand::V) == 0.0
                && s.get_plural_operand(Operand::E) == 0.0)
        {
            return Err(LuceneError::illegal_argument(format!("Ill-formed number range: {}", s.to_java_string())));
        }
        Ok(())
    }

    /// Equivalent to `addSamples(Collection<Double>)` (deprecated).
    ///
    /// # Errors
    ///
    /// The `ArithmeticException` of the `BigDecimal` increments (scale
    /// overflow).
    pub fn add_samples_doubles(&self, result: &mut Vec<f64>) -> Result<()> {
        self.add_samples(Some(result), None)
    }

    /// Equivalent to `addDecimalQuantitySamples(Collection<DecimalQuantity>)`
    /// (deprecated).
    ///
    /// # Errors
    ///
    /// As [`add_samples`](Self::add_samples).
    pub fn add_decimal_quantity_samples(&self, result: &mut Vec<SharedDecimalQuantity>) -> Result<()> {
        self.add_samples(None, Some(result))
    }

    /// Equivalent to `addSamples(Collection<Double>,
    /// Collection<DecimalQuantity>)` (deprecated): exactly one of the two
    /// collections must be given, else nothing is added. The `Collection`
    /// the callers pass is a `LinkedHashSet` for the quantities (identity
    /// set: every quantity added is new) and a list for the doubles.
    ///
    /// # Errors
    ///
    /// The `ArithmeticException` of the `BigDecimal` increments (scale
    /// overflow).
    pub fn add_samples(
        &self,
        double_result: Option<&mut Vec<f64>>,
        dq_result: Option<&mut Vec<SharedDecimalQuantity>>,
    ) -> Result<()> {
        let (mut double_result, mut dq_result) = match (double_result, dq_result) {
            (None, None) | (Some(_), Some(_)) => return Ok(()),
            (d, q) => (d, q),
        };
        let is_double = double_result.is_some();
        for range in &self.samples {
            let (lower_disp_mag, exponent, mut dq, end_double) = {
                let start = lock_dq(&range.start);
                let end_double = if Arc::ptr_eq(&range.start, &range.end) {
                    start.to_double()
                } else {
                    lock_dq(&range.end).to_double()
                };
                (start.get_lower_display_magnitude(), start.get_exponent(), start.create_copy(), end_double)
            };
            let increment_scale = lower_disp_mag + exponent;
            let increment_bd = BigDecimal::one().move_point_right(increment_scale)?;

            while dq.to_double() <= end_double {
                let next_bd;
                if is_double {
                    let dbl_value = dq.to_double();
                    // Hack Alert: don't return any decimal samples with
                    // integer values that originated from a format with
                    // trailing decimals. This API is expected to be removed
                    // in ICU 70, so this code path should not need to be
                    // optimized.
                    if dbl_value != dbl_value.floor() && dq.get_plural_operand(Operand::V) > 0.0 {
                        double_result.as_mut().expect("is_double").push(dbl_value);
                    }
                    next_bd = dq.to_big_decimal()?;
                } else {
                    let shared = share(dq);
                    next_bd = lock_dq(&shared).to_big_decimal()?;
                    set_add(dq_result.as_mut().expect("!is_double"), shared);
                }

                // Increment dq for next iteration
                let new_dq_bd = next_bd.add(&increment_bd)?;
                let mut next = DecimalQuantity_DualStorageBCD::from_big_decimal(&new_dq_bd)?;
                next.set_min_fraction(-lower_disp_mag);
                next.adjust_magnitude(-exponent)?;
                next.adjust_exponent(exponent);
                dq = Box::new(next);
            }
        }
        Ok(())
    }

    /// Equivalent to `toString()` (deprecated).
    pub fn to_java_string(&self) -> String {
        let mut b = String::from("@");
        b.push_str(&self.sample_type.name().to_lowercase());
        let mut first = true;
        for item in &self.samples {
            if first {
                first = false;
            } else {
                b.push(',');
            }
            b.push(' ');
            b.push_str(&item.to_java_string());
        }
        if !self.bounded {
            b.push_str(", \u{2026}");
        }
        b
    }

    /// Equivalent to `getSamples()` (deprecated).
    pub fn get_samples(&self) -> &[DecimalQuantitySamplesRange] {
        &self.samples
    }

    /// Equivalent to `getStartEndSamples(Set<DecimalQuantity>)` (deprecated):
    /// adds the start and end of every range to the identity set `target`.
    pub fn get_start_end_samples(&self, target: &mut Vec<SharedDecimalQuantity>) {
        for range in &self.samples {
            set_add(target, range.start.clone());
            set_add(target, range.end.clone());
        }
    }
}

impl fmt::Display for DecimalQuantitySamples {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_java_string())
    }
}

/// `String.substring(int)` of the ASCII-led sample source (index in UTF-16
/// units, the source starting with `integer` or `decimal`).
fn substring16(s: &str, begin: usize) -> &str {
    let mut count = 0;
    for (i, c) in s.char_indices() {
        if count >= begin {
            return &s[i..];
        }
        count += c.len_utf16();
    }
    ""
}

/// A constraint on a number. Equivalent to the private interface
/// `PluralRules.Constraint` (`Serializable`); `toString()` is
/// [`Display`](fmt::Display).
trait Constraint: fmt::Debug + fmt::Display + Send + Sync {
    /// Returns true if the number fulfills the constraint. Equivalent to
    /// `isFulfilled(IFixedDecimal)`.
    fn is_fulfilled(&self, n: &dyn IFixedDecimal) -> bool;

    /// Returns false if an unlimited number of values fulfills the
    /// constraint. Equivalent to `isLimited(SampleType)`.
    fn is_limited(&self, sample_type: SampleType) -> bool;

    /// The identity hash code stand-in of the object (`Object.hashCode()`,
    /// which no constraint class overrides).
    fn identity_hash_code(&self) -> i32;
}

/// The anonymous `NO_CONSTRAINT` class.
#[derive(Debug)]
struct NoConstraint;

impl fmt::Display for NoConstraint {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Ok(())
    }
}

impl Constraint for NoConstraint {
    fn is_fulfilled(&self, _n: &dyn IFixedDecimal) -> bool {
        true
    }

    fn is_limited(&self, _sample_type: SampleType) -> bool {
        false
    }

    fn identity_hash_code(&self) -> i32 {
        java_identity_hash_code_stand_in("com.ibm.icu.text.PluralRules$1")
    }
}

/// Splits a constraint into tokens. Equivalent to
/// `PluralRules.SimpleTokenizer`.
struct SimpleTokenizer;

/// `SimpleTokenizer.BREAK_AND_IGNORE`.
static BREAK_AND_IGNORE: LazyLock<UnicodeSet> = LazyLock::new(|| {
    let mut set = UnicodeSet::from_pairs(&[0x09, 0x0a, 0x0c, 0x0d, 0x20, 0x20]).expect("valid ranges");
    set.freeze();
    set
});

/// `SimpleTokenizer.BREAK_AND_KEEP`.
static BREAK_AND_KEEP: LazyLock<UnicodeSet> = LazyLock::new(|| {
    let mut set = UnicodeSet::from_pairs(&[
        '!' as i32, '!' as i32, '%' as i32, '%' as i32, ',' as i32, ',' as i32, '.' as i32, '.' as i32, '=' as i32,
        '=' as i32,
    ])
    .expect("valid ranges");
    set.freeze();
    set
});

impl SimpleTokenizer {
    /// `split(String)`.
    fn split(source: &str) -> Vec<String> {
        let source: Vec<u16> = units(source);
        let mut last: i32 = -1;
        let mut result: Vec<String> = Vec::new();
        for i in 0..source.len() as i32 {
            let ch = source[i as usize];
            if BREAK_AND_IGNORE.contains(ch as i32).unwrap_or(false) {
                if last >= 0 {
                    result.push(String::from_utf16_lossy(&source[last as usize..i as usize]));
                    last = -1;
                }
            } else if BREAK_AND_KEEP.contains(ch as i32).unwrap_or(false) {
                if last >= 0 {
                    result.push(String::from_utf16_lossy(&source[last as usize..i as usize]));
                }
                result.push(String::from_utf16_lossy(&source[i as usize..(i + 1) as usize]));
                last = -1;
            } else if last < 0 {
                last = i;
            }
        }
        if last >= 0 {
            result.push(String::from_utf16_lossy(&source[last as usize..]));
        }
        result
    }
}

/// `parseConstraint(String)`.
///
/// Syntax:
/// ```text
/// condition :     or_condition
///                 and_condition
/// or_condition :  and_condition 'or' condition
/// and_condition : relation
///                 relation 'and' relation
/// relation :      in_relation
///                 within_relation
/// in_relation :   not? expr not? in not? range
/// within_relation : not? expr not? 'within' not? range
/// not :           'not'
///                 '!'
/// expr :          'n'
///                 'n' mod value
/// mod :           'mod'
///                 '%'
/// in :            'in'
///                 'is'
///                 '='
///                 '≠'
/// value :         digit+
/// digit :         0|1|2|3|4|5|6|7|8|9
/// range :         value'..'value
/// ```
fn parse_constraint(description: &str) -> Result<Arc<dyn Constraint>> {
    let mut result: Option<Arc<dyn Constraint>> = None;
    let or_together = split(&OR_SEPARATED, description);
    for or_part in &or_together {
        let mut and_constraint: Option<Arc<dyn Constraint>> = None;
        let and_together = split(&AND_SEPARATED, or_part);
        for and_part in &and_together {
            let mut new_constraint: Arc<dyn Constraint> = NO_CONSTRAINT.clone();

            let condition = java_trim(and_part).to_string();
            let tokens = SimpleTokenizer::split(&condition);

            let mut mod_: i32 = 0;
            let mut in_range = true;
            let mut integers_only = true;
            let mut low_bound = i64::MAX as f64;
            let mut high_bound = i64::MIN as f64;
            let vals: Option<Vec<i64>>;

            let mut x = 0usize;
            let mut t = match tokens.get(x) {
                Some(t) => t.clone(),
                None => array_index_out_of_bounds_for(x, tokens.len()),
            };
            x += 1;
            let mut hack_for_compatibility = false;
            let operand = match FixedDecimal::get_operand(&t) {
                Ok(o) => o,
                Err(_) => return Err(unexpected(&t, &condition)),
            };
            if x < tokens.len() {
                t = tokens[x].clone();
                x += 1;
                if "mod" == t || "%" == t {
                    let m = match tokens.get(x) {
                        Some(m) => m.clone(),
                        None => array_index_out_of_bounds_for(x, tokens.len()),
                    };
                    x += 1;
                    mod_ = crate::jdk::parse_int(&units(&m), 10)?;
                    t = next_token(&tokens, x, &condition)?;
                    x += 1;
                }
                if "not" == t {
                    in_range = !in_range;
                    t = next_token(&tokens, x, &condition)?;
                    x += 1;
                    if "=" == t {
                        return Err(unexpected(&t, &condition));
                    }
                } else if "!" == t {
                    in_range = !in_range;
                    t = next_token(&tokens, x, &condition)?;
                    x += 1;
                    if "=" != t {
                        return Err(unexpected(&t, &condition));
                    }
                }
                if "is" == t || "in" == t || "=" == t {
                    hack_for_compatibility = "is" == t;
                    if hack_for_compatibility && !in_range {
                        return Err(unexpected(&t, &condition));
                    }
                    t = next_token(&tokens, x, &condition)?;
                    x += 1;
                } else if "within" == t {
                    integers_only = false;
                    t = next_token(&tokens, x, &condition)?;
                    x += 1;
                } else {
                    return Err(unexpected(&t, &condition));
                }
                if "not" == t {
                    if !hack_for_compatibility && !in_range {
                        return Err(unexpected(&t, &condition));
                    }
                    in_range = !in_range;
                    t = next_token(&tokens, x, &condition)?;
                    x += 1;
                }

                let mut value_list: Vec<i64> = Vec::new();

                // the token t is always one item ahead
                loop {
                    let low = parse_long(&t)?;
                    let mut high = low;
                    if x < tokens.len() {
                        t = next_token(&tokens, x, &condition)?;
                        x += 1;
                        if t == "." {
                            t = next_token(&tokens, x, &condition)?;
                            x += 1;
                            if t != "." {
                                return Err(unexpected(&t, &condition));
                            }
                            t = next_token(&tokens, x, &condition)?;
                            x += 1;
                            high = parse_long(&t)?;
                            if x < tokens.len() {
                                t = next_token(&tokens, x, &condition)?;
                                x += 1;
                                if t != "," {
                                    // adjacent number: if we wanted to
                                    // support implicit '=', we'd use ";"
                                    return Err(unexpected(&t, &condition));
                                }
                            }
                        } else if t != "," {
                            // adjacent number: if we wanted to support
                            // implicit '=', we'd use ";"
                            return Err(unexpected(&t, &condition));
                        }
                    }
                    // at this point, either we are out of tokens, or t is ','
                    if low > high {
                        return Err(unexpected(&format!("{low}~{high}"), &condition));
                    } else if mod_ != 0 && high >= mod_ as i64 {
                        return Err(unexpected(&format!("{high}>mod={mod_}"), &condition));
                    }
                    value_list.push(low);
                    value_list.push(high);
                    low_bound = low_bound.min(low as f64);
                    high_bound = high_bound.max(high as f64);
                    if x >= tokens.len() {
                        break;
                    }
                    t = next_token(&tokens, x, &condition)?;
                    x += 1;
                }

                if t == "," {
                    return Err(unexpected(&t, &condition));
                }

                if value_list.len() == 2 {
                    vals = None;
                } else {
                    vals = Some(value_list);
                }

                // Hack to exclude "is not 1,2"
                if low_bound != high_bound && hack_for_compatibility && !in_range {
                    return Err(unexpected("is not <range>", &condition));
                }

                new_constraint = Arc::new(RangeConstraint::new(
                    mod_,
                    in_range,
                    operand,
                    integers_only,
                    low_bound,
                    high_bound,
                    vals,
                ));
            }

            and_constraint = Some(match and_constraint {
                None => new_constraint,
                Some(a) => Arc::new(AndConstraint { a, b: new_constraint }),
            });
        }

        let and_constraint = and_constraint.expect("Pattern.split yields at least one element");
        result = Some(match result {
            None => and_constraint,
            Some(r) => Arc::new(OrConstraint { a: r, b: and_constraint }),
        });
    }
    Ok(result.expect("Pattern.split yields at least one element"))
}

/// The `ArrayIndexOutOfBoundsException` of `tokens[x++]` past the end.
fn array_index_out_of_bounds_for(index: usize, length: usize) -> ! {
    panic!("java.lang.ArrayIndexOutOfBoundsException: Index {index} out of bounds for length {length}")
}

/// `Long.parseLong(String)`.
fn parse_long(s: &str) -> Result<i64> {
    let valid = {
        let b = s.as_bytes();
        let digits = if !b.is_empty() && (b[0] == b'-' || b[0] == b'+') { &b[1..] } else { b };
        !digits.is_empty() && digits.iter().all(|c| c.is_ascii_digit())
    };
    if !valid {
        return Err(LuceneError::number_format_for_input_string(s, 10));
    }
    s.parse::<i64>().map_err(|_| LuceneError::number_format_for_input_string(s, 10))
}

/// Returns a parse exception wrapping the token and context strings.
/// Equivalent to `unexpected(String, String)`.
fn unexpected(token: &str, context: &str) -> LuceneError {
    ParseException::error(format!("unexpected token '{token}' in '{context}'"), -1)
}

/// Returns the token at x if available, else throws a parse exception.
/// Equivalent to `nextToken(String[], int, String)`.
fn next_token(tokens: &[String], x: usize, context: &str) -> Result<String> {
    if x < tokens.len() {
        return Ok(tokens[x].clone());
    }
    Err(ParseException::error(format!("missing token at end of '{context}'"), -1))
}

/// Syntax: `keyword ':' condition`. Equivalent to `parseRule(String)`.
fn parse_rule(description: &str) -> Result<Rule> {
    if description.is_empty() {
        return Ok(DEFAULT_RULE.clone());
    }

    let description = description.to_lowercase();

    let x = match description.find(':') {
        Some(x) => x,
        None => {
            return Err(ParseException::error(format!("missing ':' in rule description '{description}'"), 0));
        }
    };

    let keyword = java_trim(&description[..x]).to_string();
    if !is_valid_keyword(&keyword) {
        return Err(ParseException::error(format!("keyword '{keyword} is not valid"), 0));
    }

    let description = java_trim(&description[x + 1..]).to_string();
    let constraint_or_samples = split(&AT_SEPARATED, &description);
    let sample_failure = false;
    let mut integer_samples: Option<DecimalQuantitySamples> = None;
    let mut decimal_samples: Option<DecimalQuantitySamples> = None;
    match constraint_or_samples.len() {
        1 => {}
        2 => {
            let samples = DecimalQuantitySamples::parse(&constraint_or_samples[1])?;
            if samples.sample_type == SampleType::Decimal {
                decimal_samples = Some(samples);
            } else {
                integer_samples = Some(samples);
            }
        }
        3 => {
            let i = DecimalQuantitySamples::parse(&constraint_or_samples[1])?;
            let d = DecimalQuantitySamples::parse(&constraint_or_samples[2])?;
            if i.sample_type != SampleType::Integer || d.sample_type != SampleType::Decimal {
                return Err(LuceneError::illegal_argument(format!("Must have @integer then @decimal in {description}")));
            }
            integer_samples = Some(i);
            decimal_samples = Some(d);
        }
        _ => return Err(LuceneError::illegal_argument(format!("Too many samples in {description}"))),
    }
    if sample_failure {
        return Err(LuceneError::illegal_argument("Ill-formed samples\u{2014}'@' characters."));
    }

    // 'other' is special, and must have no rules; all other keywords must
    // have rules.
    let is_other = keyword == "other";
    if is_other != (len16(&constraint_or_samples[0]) == 0) {
        return Err(LuceneError::illegal_argument("The keyword 'other' must have no constraints, just samples."));
    }

    let constraint = if is_other { NO_CONSTRAINT.clone() } else { parse_constraint(&constraint_or_samples[0])? };
    Ok(Rule::new(keyword, constraint, integer_samples.map(Arc::new), decimal_samples.map(Arc::new)))
}

/// Syntax: `rule : keyword ':' condition`, `keyword: <identifier>`.
/// Equivalent to `parseRuleChain(String)`.
fn parse_rule_chain(description: &str) -> Result<RuleList> {
    let mut result = RuleList::new();
    // remove trailing ;
    let description = description.strip_suffix(';').unwrap_or(description);
    let rules = split(&SEMI_SEPARATED, description);
    for r in &rules {
        let rule = parse_rule(java_trim(r))?;
        result.has_explicit_bounding_info |= rule.integer_samples.is_some() || rule.decimal_samples.is_some();
        result.add_rule(rule)?;
    }
    result.finish()
}

/// An implementation of Constraint representing a modulus, a range of
/// values, and include/exclude. Provides lots of flexibility. Equivalent to
/// `PluralRules.RangeConstraint`.
#[derive(Debug)]
struct RangeConstraint {
    mod_: i32,
    in_range: bool,
    integers_only: bool,
    lower_bound: f64,
    upper_bound: f64,
    range_list: Option<Vec<i64>>,
    operand: Operand,
}

impl RangeConstraint {
    fn new(
        mod_: i32,
        in_range: bool,
        operand: Operand,
        integers_only: bool,
        low_bound: f64,
        high_bound: f64,
        vals: Option<Vec<i64>>,
    ) -> Self {
        Self { mod_, in_range, integers_only, lower_bound: low_bound, upper_bound: high_bound, range_list: vals, operand }
    }
}

impl Constraint for RangeConstraint {
    fn is_fulfilled(&self, number: &dyn IFixedDecimal) -> bool {
        let mut n = number.get_plural_operand(self.operand);
        if (self.integers_only && (n - (n as i64) as f64) != 0.0)
            || (self.operand == Operand::J && number.get_plural_operand(Operand::V) != 0.0)
        {
            return !self.in_range;
        }
        if self.mod_ != 0 {
            n %= self.mod_ as f64; // java % handles double numerator the way we want
        }
        let mut test = n >= self.lower_bound && n <= self.upper_bound;
        if test {
            if let Some(range_list) = &self.range_list {
                test = false;
                let mut i = 0;
                while !test && i < range_list.len() {
                    test = n >= range_list[i] as f64 && n <= range_list[i + 1] as f64;
                    i += 2;
                }
            }
        }
        self.in_range == test
    }

    fn is_limited(&self, sample_type: SampleType) -> bool {
        let value_is_zero = self.lower_bound == self.upper_bound && self.lower_bound == 0.0;
        let has_decimals = (self.operand == Operand::V
            || self.operand == Operand::W
            || self.operand == Operand::F
            || self.operand == Operand::T)
            && self.in_range != value_is_zero; // either NOT f = zero or f = non-zero
        match sample_type {
            SampleType::Integer => {
                has_decimals // will be empty
                    || ((self.operand == Operand::N || self.operand == Operand::I || self.operand == Operand::J)
                        && self.mod_ == 0
                        && self.in_range)
            }
            SampleType::Decimal => {
                (!has_decimals || self.operand == Operand::N || self.operand == Operand::J)
                    && (self.integers_only || self.lower_bound == self.upper_bound)
                    && self.mod_ == 0
                    && self.in_range
            }
        }
    }

    fn identity_hash_code(&self) -> i32 {
        java_identity_hash_code_stand_in("com.ibm.icu.text.PluralRules$RangeConstraint")
    }
}

impl fmt::Display for RangeConstraint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut result = String::new();
        result.push_str(self.operand.name());
        if self.mod_ != 0 {
            result.push_str(" % ");
            result.push_str(&self.mod_.to_string());
        }
        let is_list = self.lower_bound != self.upper_bound;
        result.push_str(if !is_list {
            if self.in_range {
                " = "
            } else {
                " != "
            }
        } else if self.integers_only {
            if self.in_range {
                " = "
            } else {
                " != "
            }
        } else if self.in_range {
            " within "
        } else {
            " not within "
        });
        if let Some(range_list) = &self.range_list {
            let mut i = 0;
            while i < range_list.len() {
                add_range(&mut result, range_list[i] as f64, range_list[i + 1] as f64, i != 0);
                i += 2;
            }
        } else {
            add_range(&mut result, self.lower_bound, self.upper_bound, false);
        }
        f.write_str(&result)
    }
}

/// `addRange(StringBuilder, double, double, boolean)`.
fn add_range(result: &mut String, lb: f64, ub: f64, add_separator: bool) {
    if add_separator {
        result.push(',');
    }
    if lb == ub {
        result.push_str(&format_bound(lb));
    } else {
        result.push_str(&format!("{}..{}", format_bound(lb), format_bound(ub)));
    }
}

/// `format(double)`.
fn format_bound(lb: f64) -> String {
    let lbi = lb as i64;
    if lb == lbi as f64 {
        lbi.to_string()
    } else {
        double_to_string(lb)
    }
}

/// A constraint representing the logical and of two constraints.
/// Equivalent to `PluralRules.AndConstraint` (a `BinaryConstraint`).
#[derive(Debug)]
struct AndConstraint {
    a: Arc<dyn Constraint>,
    b: Arc<dyn Constraint>,
}

impl Constraint for AndConstraint {
    fn is_fulfilled(&self, n: &dyn IFixedDecimal) -> bool {
        self.a.is_fulfilled(n) && self.b.is_fulfilled(n)
    }

    fn is_limited(&self, sample_type: SampleType) -> bool {
        // we ignore the case where both a and b are unlimited but no values
        // satisfy both-- we still consider this 'unlimited'
        self.a.is_limited(sample_type) || self.b.is_limited(sample_type)
    }

    fn identity_hash_code(&self) -> i32 {
        java_identity_hash_code_stand_in("com.ibm.icu.text.PluralRules$AndConstraint")
    }
}

impl fmt::Display for AndConstraint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} and {}", self.a, self.b)
    }
}

/// A constraint representing the logical or of two constraints.
/// Equivalent to `PluralRules.OrConstraint` (a `BinaryConstraint`).
#[derive(Debug)]
struct OrConstraint {
    a: Arc<dyn Constraint>,
    b: Arc<dyn Constraint>,
}

impl Constraint for OrConstraint {
    fn is_fulfilled(&self, n: &dyn IFixedDecimal) -> bool {
        self.a.is_fulfilled(n) || self.b.is_fulfilled(n)
    }

    fn is_limited(&self, sample_type: SampleType) -> bool {
        self.a.is_limited(sample_type) && self.b.is_limited(sample_type)
    }

    fn identity_hash_code(&self) -> i32 {
        java_identity_hash_code_stand_in("com.ibm.icu.text.PluralRules$OrConstraint")
    }
}

impl fmt::Display for OrConstraint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} or {}", self.a, self.b)
    }
}

/// Implementation of Rule that uses a constraint. Provides 'and' and 'or'
/// to combine constraints. Immutable. Equivalent to `PluralRules.Rule`.
#[derive(Debug, Clone)]
struct Rule {
    keyword: String,
    constraint: Arc<dyn Constraint>,
    integer_samples: Option<Arc<DecimalQuantitySamples>>,
    decimal_samples: Option<Arc<DecimalQuantitySamples>>,
}

impl Rule {
    fn new(
        keyword: String,
        constraint: Arc<dyn Constraint>,
        integer_samples: Option<Arc<DecimalQuantitySamples>>,
        decimal_samples: Option<Arc<DecimalQuantitySamples>>,
    ) -> Self {
        Self { keyword, constraint, integer_samples, decimal_samples }
    }

    /// Equivalent to `and(Constraint)` (unused).
    #[allow(dead_code)]
    fn and(&self, c: Arc<dyn Constraint>) -> Rule {
        Rule::new(
            self.keyword.clone(),
            Arc::new(AndConstraint { a: self.constraint.clone(), b: c }),
            self.integer_samples.clone(),
            self.decimal_samples.clone(),
        )
    }

    /// Equivalent to `or(Constraint)` (unused).
    #[allow(dead_code)]
    fn or(&self, c: Arc<dyn Constraint>) -> Rule {
        Rule::new(
            self.keyword.clone(),
            Arc::new(OrConstraint { a: self.constraint.clone(), b: c }),
            self.integer_samples.clone(),
            self.decimal_samples.clone(),
        )
    }

    fn get_keyword(&self) -> &str {
        &self.keyword
    }

    fn applies_to(&self, n: &dyn IFixedDecimal) -> bool {
        self.constraint.is_fulfilled(n)
    }

    fn is_limited(&self, sample_type: SampleType) -> bool {
        self.constraint.is_limited(sample_type)
    }

    /// Equivalent to `hashCode()`: the keyword's `String.hashCode()` xor the
    /// constraint's identity hash code.
    #[allow(dead_code)]
    fn hash_code(&self) -> i32 {
        crate::r#impl::utility::Utility::char_sequence_hash_code(&units(&self.keyword))
            ^ self.constraint.identity_hash_code()
    }

    fn get_constraint(&self) -> String {
        self.constraint.to_string()
    }
}

impl fmt::Display for Rule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.keyword, self.constraint)?;
        if let Some(s) = &self.integer_samples {
            write!(f, " {}", s.to_java_string())?;
        }
        if let Some(s) = &self.decimal_samples {
            write!(f, " {}", s.to_java_string())?;
        }
        Ok(())
    }
}

/// The ordered list of rules. Equivalent to `PluralRules.RuleList`.
#[derive(Debug, Clone)]
struct RuleList {
    has_explicit_bounding_info: bool,
    rules: Vec<Rule>,
}

impl RuleList {
    fn new() -> Self {
        Self { has_explicit_bounding_info: false, rules: Vec::new() }
    }

    /// `addRule(Rule)`.
    fn add_rule(&mut self, next_rule: Rule) -> Result<&mut Self> {
        let keyword = next_rule.get_keyword();
        for rule in &self.rules {
            if keyword == rule.get_keyword() {
                return Err(LuceneError::illegal_argument(format!("Duplicate keyword: {keyword}")));
            }
        }
        self.rules.push(next_rule);
        Ok(self)
    }

    /// `finish()`.
    fn finish(mut self) -> Result<Self> {
        // make sure that 'other' is present, and at the end.
        let mut other_rule: Option<Rule> = None;
        let mut i = 0;
        while i < self.rules.len() {
            if "other" == self.rules[i].get_keyword() {
                other_rule = Some(self.rules.remove(i));
            } else {
                i += 1;
            }
        }
        let other_rule = match other_rule {
            Some(r) => r,
            None => parse_rule("other:")?, // make sure we have always have an 'other' a rule
        };
        self.rules.push(other_rule);
        Ok(self)
    }

    fn select_rule(&self, n: &dyn IFixedDecimal) -> Option<&Rule> {
        self.rules.iter().find(|rule| rule.applies_to(n))
    }

    fn select(&self, n: &dyn IFixedDecimal) -> String {
        if n.is_infinite() || n.is_nan() {
            return KEYWORD_OTHER.to_string();
        }
        let r = self.select_rule(n).expect("java.lang.NullPointerException: no rule applies");
        r.get_keyword().to_string()
    }

    fn get_keywords(&self) -> Vec<String> {
        let mut result: Vec<String> = Vec::new();
        for rule in &self.rules {
            let k = rule.get_keyword().to_string();
            if !result.contains(&k) {
                result.push(k);
            }
        }
        // since we have explict 'other', we don't need this.
        //result.add(KEYWORD_OTHER);
        result
    }

    fn is_limited(&self, keyword: &str, sample_type: SampleType) -> bool {
        if self.has_explicit_bounding_info {
            let my_samples = self.get_decimal_samples(keyword, sample_type);
            return match my_samples {
                None => true,
                Some(s) => s.bounded,
            };
        }

        self.compute_limited(keyword, sample_type)
    }

    fn compute_limited(&self, keyword: &str, sample_type: SampleType) -> bool {
        // if all rules with this keyword are limited, it's limited,
        // and if there's no rule with this keyword, it's unlimited
        let mut result = false;
        for rule in &self.rules {
            if keyword == rule.get_keyword() {
                if !rule.is_limited(sample_type) {
                    return false;
                }
                result = true;
            }
        }
        result
    }

    fn get_rules(&self, keyword: &str) -> Option<String> {
        for rule in &self.rules {
            if rule.get_keyword() == keyword {
                return Some(rule.get_constraint());
            }
        }
        None
    }

    fn select_keyword(&self, sample: &dyn IFixedDecimal, keyword: &str) -> bool {
        for rule in &self.rules {
            if rule.get_keyword() == keyword && rule.applies_to(sample) {
                return true;
            }
        }
        false
    }

    fn get_decimal_samples(&self, keyword: &str, sample_type: SampleType) -> Option<&Arc<DecimalQuantitySamples>> {
        for rule in &self.rules {
            if rule.get_keyword() == keyword {
                return if sample_type == SampleType::Integer {
                    rule.integer_samples.as_ref()
                } else {
                    rule.decimal_samples.as_ref()
                };
            }
        }
        None
    }
}

impl fmt::Display for RuleList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut builder = String::new();
        for rule in &self.rules {
            if !builder.is_empty() {
                builder.push_str(CATEGORY_SEPARATOR);
            }
            builder.push_str(&rule.to_string());
        }
        f.write_str(&builder)
    }
}

/// Status of the keyword for the rules, given a set of explicit values.
/// Equivalent to `PluralRules.KeywordStatus` (internal, deprecated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeywordStatus {
    /// The keyword is not valid for the rules.
    Invalid,
    /// The keyword is valid, but unused (it is covered by the explicit
    /// values, OR has no values for the given type).
    Suppressed,
    /// The keyword is valid, used, and has a single possible value (before
    /// considering explicit values).
    Unique,
    /// The keyword is valid, used, not unique, and has a finite set of
    /// values.
    Bounded,
    /// The keyword is valid but not bounded; there indefinitely many
    /// matching values.
    Unbounded,
}

/// Defines rules for mapping non-negative numeric values onto a small set of
/// keywords.
///
/// Equivalent to `com.ibm.icu.text.PluralRules`. Rule descriptions,
/// keywords and renderings are Rust strings (CLDR rule texts are
/// well-formed UTF-16); instances are shared as `Arc<PluralRules>` (the
/// static [`DEFAULT`], the loader's cache). Java's `Locale` overloads of
/// `forLocale` are the `ULocale` ones after `ULocale.forLocale`. Java
/// serialization replaces the rules by a [`PluralRulesSerialProxy`]
/// ([`write_replace`](Self::write_replace)).
#[derive(Debug)]
pub struct PluralRules {
    rules: RuleList,
    keywords: Vec<String>,
    standard_plural_ranges: Option<Arc<StandardPluralRanges>>,
}

impl PluralRules {
    /// Parses a plural rules description and returns a PluralRules.
    /// Equivalent to `parseDescription(String)`.
    ///
    /// # Errors
    ///
    /// `ParseException` if the description cannot be parsed (and the
    /// `IllegalArgumentException`/`NumberFormatException` of ill-formed
    /// samples, as Java).
    pub fn parse_description(description: &str) -> Result<Arc<PluralRules>> {
        Self::new_internal(description, None)
    }

    /// Creates a PluralRules from a description if it is parsable,
    /// otherwise returns `None`. Equivalent to `createRules(String)`.
    pub fn create_rules(description: &str) -> Option<Arc<PluralRules>> {
        Self::parse_description(description).ok()
    }

    /// Equivalent to `newInternal(String, StandardPluralRanges)` (internal).
    ///
    /// # Errors
    ///
    /// As [`parse_description`](Self::parse_description).
    pub fn new_internal(description: &str, ranges: Option<Arc<StandardPluralRanges>>) -> Result<Arc<PluralRules>> {
        let description = java_trim(description);
        if description.is_empty() {
            Ok(DEFAULT.clone())
        } else {
            Ok(Arc::new(PluralRules::new(parse_rule_chain(description)?, ranges)))
        }
    }

    /// `PluralRules(RuleList, StandardPluralRanges)`.
    fn new(rules: RuleList, standard_plural_ranges: Option<Arc<StandardPluralRanges>>) -> Self {
        let keywords = rules.get_keywords();
        Self { rules, keywords, standard_plural_ranges }
    }

    /// Provides access to the predefined cardinal-number `PluralRules` for a
    /// given locale. Equivalent to `forLocale(ULocale)` (and, after
    /// `ULocale.forLocale`, `forLocale(Locale)`).
    ///
    /// # Errors
    ///
    /// The `MissingResourceException` of the plural data.
    pub fn for_locale(locale: &ULocale) -> Result<Arc<PluralRules>> {
        get_default_factory().for_locale(locale, PluralType::Cardinal)
    }

    /// Equivalent to `forLocale(Locale)`.
    ///
    /// # Errors
    ///
    /// The errors of `ULocale.forLocale` and of the plural data.
    pub fn for_java_locale(locale: &Locale) -> Result<Arc<PluralRules>> {
        let ulocale = ULocale::for_locale(Some(locale))?.unwrap_or_else(ULocale::root);
        Self::for_locale(&ulocale)
    }

    /// Provides access to the predefined `PluralRules` for a given locale and
    /// the plural type. Equivalent to `forLocale(ULocale, PluralType)`.
    ///
    /// # Errors
    ///
    /// The `MissingResourceException` of the plural data.
    pub fn for_locale_type(locale: &ULocale, plural_type: PluralType) -> Result<Arc<PluralRules>> {
        get_default_factory().for_locale(locale, plural_type)
    }

    /// Equivalent to `forLocale(Locale, PluralType)`.
    ///
    /// # Errors
    ///
    /// The errors of `ULocale.forLocale` and of the plural data.
    pub fn for_java_locale_type(locale: &Locale, plural_type: PluralType) -> Result<Arc<PluralRules>> {
        let ulocale = ULocale::for_locale(Some(locale))?.unwrap_or_else(ULocale::root);
        Self::for_locale_type(&ulocale, plural_type)
    }

    /// Checks whether a token is a valid keyword. Equivalent to
    /// `isValidKeyword(String)`.
    fn is_valid_keyword(token: &str) -> bool {
        is_valid_keyword(token)
    }

    /// Equivalent to `hashCode()`: the identity hash code of the rule list
    /// (`RuleList` does not override `hashCode`).
    pub fn hash_code(&self) -> i32 {
        java_identity_hash_code_stand_in("com.ibm.icu.text.PluralRules$RuleList")
    }

    /// Given a floating-point number, returns the keyword of the first rule
    /// that applies to the number. Equivalent to `select(double)`.
    pub fn select_double(&self, number: f64) -> String {
        self.rules.select(&FixedDecimal::from_double(number))
    }

    /// Given a formatted number, returns the keyword of the first rule that
    /// applies to the number. Equivalent to `select(FormattedNumber)`.
    pub fn select_formatted(&self, number: &FormattedNumber) -> String {
        self.rules.select(number.get_fixed_decimal())
    }

    /// Given a formatted number range, returns the overall plural form of
    /// the range. Equivalent to `select(FormattedNumberRange)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException("Plural ranges are unavailable on this
    /// instance")` for rules not obtained from a locale, and the
    /// `IllegalArgumentException` of `StandardPlural.fromString`.
    pub fn select_range(&self, range: &FormattedNumberRange) -> Result<String> {
        let ranges = match &self.standard_plural_ranges {
            None => {
                return Err(LuceneError::unsupported_operation("Plural ranges are unavailable on this instance"));
            }
            Some(r) => r,
        };
        let form1 = StandardPlural::from_string(&self.select(range.get_first_fixed_decimal()))?;
        let form2 = StandardPlural::from_string(&self.select(range.get_second_fixed_decimal()))?;
        let result = ranges.resolve(form1, form2);
        Ok(result.get_keyword().to_string())
    }

    /// Given a number, returns the keyword of the first rule that applies to
    /// the number. Equivalent to `select(double, int, long)` (internal).
    pub fn select_with_digits(&self, number: f64, count_visible_fraction_digits: i32, fractionaldigits: i64) -> String {
        self.rules.select(&FixedDecimal::new_with_digits(number, count_visible_fraction_digits, fractionaldigits))
    }

    /// Given a number information, returns the keyword of the first rule
    /// that applies to the number. Equivalent to `select(IFixedDecimal)`
    /// (internal).
    pub fn select(&self, number: &dyn IFixedDecimal) -> String {
        self.rules.select(number)
    }

    /// Given a number information, and keyword, return whether the keyword
    /// would match the number. Equivalent to `matches(FixedDecimal, String)`
    /// (internal).
    pub fn matches(&self, sample: &FixedDecimal, keyword: &str) -> bool {
        self.rules.select_keyword(sample, keyword)
    }

    /// Returns a set of all rule keywords used in this `PluralRules` object.
    /// The rule "other" is always present by default. Equivalent to
    /// `getKeywords()`.
    pub fn get_keywords(&self) -> &[String] {
        &self.keywords
    }

    /// Returns the unique value that this keyword matches, or
    /// [`NO_UNIQUE_VALUE`] if the keyword matches multiple values or is not
    /// defined for this PluralRules. Equivalent to
    /// `getUniqueKeywordValue(String)`.
    ///
    /// # Errors
    ///
    /// The `ArithmeticException` of the sample arithmetic (scale overflow).
    pub fn get_unique_keyword_value(&self, keyword: &str) -> Result<f64> {
        let uniq_val_dq = self.get_unique_keyword_decimal_quantity_value(keyword)?;
        if dq_equals(&uniq_val_dq, &NO_UNIQUE_VALUE_DECIMAL_QUANTITY) {
            Ok(NO_UNIQUE_VALUE)
        } else {
            Ok(lock_dq(&uniq_val_dq).to_double())
        }
    }

    /// Returns the unique value that this keyword matches, or
    /// [`NO_UNIQUE_VALUE_DECIMAL_QUANTITY`] if the keyword matches multiple
    /// values or is not defined for this PluralRules. Equivalent to
    /// `getUniqueKeywordDecimalQuantityValue(String)` (internal).
    ///
    /// # Errors
    ///
    /// The `ArithmeticException` of the sample arithmetic (scale overflow).
    pub fn get_unique_keyword_decimal_quantity_value(&self, keyword: &str) -> Result<SharedDecimalQuantity> {
        if let Some(values) = self.get_all_keyword_decimal_quantity_values(keyword)? {
            if values.len() == 1 {
                return Ok(values[0].clone());
            }
        }
        Ok(NO_UNIQUE_VALUE_DECIMAL_QUANTITY.clone())
    }

    /// Returns all the values that trigger this keyword, or `None` if the
    /// number of such values is unlimited. Equivalent to
    /// `getAllKeywordValues(String)`.
    ///
    /// # Errors
    ///
    /// The `ArithmeticException` of the sample arithmetic (scale overflow).
    pub fn get_all_keyword_values(&self, keyword: &str) -> Result<Option<Vec<f64>>> {
        let samples = match self.get_all_keyword_decimal_quantity_values(keyword)? {
            None => return Ok(None),
            Some(s) => s,
        };
        let mut result: Vec<f64> = Vec::new();
        for dq in &samples {
            set_add_double(&mut result, lock_dq(dq).to_double());
        }
        Ok(Some(result))
    }

    /// Returns all the values that trigger this keyword, or `None` if the
    /// number of such values is unlimited. Equivalent to
    /// `getAllKeywordDecimalQuantityValues(String)` (internal).
    ///
    /// # Errors
    ///
    /// The `ArithmeticException` of the sample arithmetic (scale overflow).
    pub fn get_all_keyword_decimal_quantity_values(&self, keyword: &str) -> Result<Option<Vec<SharedDecimalQuantity>>> {
        self.get_all_keyword_values_typed(keyword, SampleType::Integer)
    }

    /// Returns all the values that trigger this keyword, or `None` if the
    /// number of such values is unlimited. Equivalent to
    /// `getAllKeywordValues(String, SampleType)` (internal).
    ///
    /// # Errors
    ///
    /// The `ArithmeticException` of the sample arithmetic (scale overflow).
    pub fn get_all_keyword_values_typed(
        &self,
        keyword: &str,
        sample_type: SampleType,
    ) -> Result<Option<Vec<SharedDecimalQuantity>>> {
        if !self.is_limited(keyword, sample_type) {
            return Ok(None);
        }
        self.get_decimal_quantity_samples_typed(keyword, sample_type)
    }

    /// Returns a list of integer values for which select() would return
    /// that keyword, or `None` if the keyword is not defined. Equivalent to
    /// `getSamples(String)`.
    ///
    /// # Errors
    ///
    /// The `ArithmeticException` of the sample arithmetic (scale overflow).
    pub fn get_samples(&self, keyword: &str) -> Result<Option<Vec<f64>>> {
        self.get_samples_typed(keyword, SampleType::Integer)
    }

    /// Returns a list of integer values for which select() would return
    /// that keyword, or `None` if the keyword is not defined. Equivalent to
    /// `getDecimalQuantitySamples(String)` (internal).
    ///
    /// # Errors
    ///
    /// The `ArithmeticException` of the sample arithmetic (scale overflow).
    pub fn get_decimal_quantity_samples(&self, keyword: &str) -> Result<Option<Vec<SharedDecimalQuantity>>> {
        self.get_decimal_quantity_samples_typed(keyword, SampleType::Integer)
    }

    /// Returns a list of values for which select() would return that keyword,
    /// or `None` if the keyword is not defined. Equivalent to
    /// `getSamples(String, SampleType)` (internal).
    ///
    /// # Errors
    ///
    /// The `ArithmeticException` of the sample arithmetic (scale overflow).
    pub fn get_samples_typed(&self, keyword: &str, sample_type: SampleType) -> Result<Option<Vec<f64>>> {
        let samples = match self.get_decimal_quantity_samples_typed(keyword, sample_type)? {
            None => return Ok(None),
            Some(s) => s,
        };
        let mut result: Vec<f64> = Vec::new();
        for dq in &samples {
            set_add_double(&mut result, lock_dq(dq).to_double());
        }
        Ok(Some(result))
    }

    /// Returns a list of values for which select() would return that keyword,
    /// or `None` if the keyword is not defined. Equivalent to
    /// `getDecimalQuantitySamples(String, SampleType)` (internal): the
    /// insertion-ordered identity set Java returns.
    pub fn get_decimal_quantity_samples_typed(
        &self,
        keyword: &str,
        sample_type: SampleType,
    ) -> Result<Option<Vec<SharedDecimalQuantity>>> {
        if !self.keywords.iter().any(|k| k == keyword) {
            return Ok(None);
        }
        let mut result: Vec<SharedDecimalQuantity> = Vec::new();

        if self.rules.has_explicit_bounding_info {
            let samples = self.rules.get_decimal_samples(keyword, sample_type);
            return Ok(Some(match samples {
                None => result,
                Some(s) => {
                    s.add_decimal_quantity_samples(&mut result)?;
                    result
                }
            }));
        }

        // hack in case the rule is created without explicit samples
        let max_count = if self.is_limited(keyword, sample_type) { i32::MAX } else { 20 };

        match sample_type {
            SampleType::Integer => {
                for i in 0..200 {
                    if !self.add_sample(keyword, share(Box::new(DecimalQuantity_DualStorageBCD::from_int(i))), max_count, &mut result) {
                        break;
                    }
                }
                self.add_sample(keyword, share(Box::new(DecimalQuantity_DualStorageBCD::from_int(1000000))), max_count, &mut result);
            }
            SampleType::Decimal => {
                for i in 0..2000 {
                    let mut next_sample = DecimalQuantity_DualStorageBCD::from_int(i);
                    next_sample.adjust_magnitude(-1)?;
                    if !self.add_sample(keyword, share(Box::new(next_sample)), max_count, &mut result) {
                        break;
                    }
                }
                let sample = DecimalQuantity_DualStorageBCD::from_exponent_string("1000000.0")?;
                self.add_sample(keyword, share(Box::new(sample)), max_count, &mut result);
            }
        }

        if result.is_empty() {
            Ok(None)
        } else {
            Ok(Some(result))
        }
    }

    /// `addSample(String, DecimalQuantity, int, Set<DecimalQuantity>)`: the
    /// `maxCount` decrement is local to the call, as in Java.
    fn add_sample(&self, keyword: &str, sample: SharedDecimalQuantity, mut max_count: i32, result: &mut Vec<SharedDecimalQuantity>) -> bool {
        let selected_keyword = {
            let s = lock_dq(&sample);
            self.select(&**s)
        };
        if selected_keyword == keyword {
            set_add(result, sample);
            max_count -= 1;
            if max_count < 0 {
                return false;
            }
        }
        true
    }

    /// Returns a list of values for which select() would return that
    /// keyword, or `None` if the keyword is not defined or no samples are
    /// available. Equivalent to `getDecimalSamples(String, SampleType)`
    /// (internal).
    pub fn get_decimal_samples(&self, keyword: &str, sample_type: SampleType) -> Option<Arc<DecimalQuantitySamples>> {
        self.rules.get_decimal_samples(keyword, sample_type).cloned()
    }

    /// Returns the set of locales for which PluralRules are known.
    /// Equivalent to `getAvailableULocales()`.
    ///
    /// # Errors
    ///
    /// The errors of `ULocale.createCanonical`.
    pub fn get_available_u_locales() -> Result<Vec<ULocale>> {
        get_default_factory().get_available_u_locales()
    }

    /// Returns the 'functionally equivalent' locale with respect to plural
    /// rules. Equivalent to `getFunctionalEquivalent(ULocale, boolean[])`.
    ///
    /// # Errors
    ///
    /// The errors of locale canonicalization.
    pub fn get_functional_equivalent(locale: &ULocale, is_available: Option<&mut [bool]>) -> Result<ULocale> {
        get_default_factory().get_functional_equivalent(locale, is_available)
    }

    /// Equivalent to `toString()`.
    pub fn to_java_string(&self) -> String {
        self.rules.to_string()
    }

    /// Returns true if rhs is equal to this: their `toString()` renderings
    /// are equal. Equivalent to `equals(PluralRules)` (and to
    /// `equals(Object)` on a `PluralRules`).
    pub fn equals(&self, rhs: Option<&PluralRules>) -> bool {
        match rhs {
            Some(rhs) => self.to_java_string() == rhs.to_java_string(),
            None => false,
        }
    }

    /// Find the status for the keyword, given a certain set of explicit
    /// values. Equivalent to `getKeywordStatus(String, int,
    /// Set<DecimalQuantity>, Output<DecimalQuantity>)` (internal).
    ///
    /// # Errors
    ///
    /// The `ArithmeticException` of the sample arithmetic.
    pub fn get_keyword_status(
        &self,
        keyword: &str,
        offset: i32,
        explicits: Option<&[SharedDecimalQuantity]>,
        unique_value: Option<&mut Output<SharedDecimalQuantity>>,
    ) -> Result<KeywordStatus> {
        self.get_keyword_status_typed(keyword, offset, explicits, unique_value, SampleType::Integer)
    }

    /// Find the status for the keyword, given a certain set of explicit
    /// values. Equivalent to `getKeywordStatus(String, int,
    /// Set<DecimalQuantity>, Output<DecimalQuantity>, SampleType)`
    /// (internal). `explicits` is Java's (identity) set; `uniqueValue` the
    /// nullable output.
    ///
    /// # Errors
    ///
    /// The `ArithmeticException` of the sample arithmetic.
    pub fn get_keyword_status_typed(
        &self,
        keyword: &str,
        offset: i32,
        explicits: Option<&[SharedDecimalQuantity]>,
        mut unique_value: Option<&mut Output<SharedDecimalQuantity>>,
        sample_type: SampleType,
    ) -> Result<KeywordStatus> {
        if let Some(u) = unique_value.as_mut() {
            u.value = None;
        }

        if !self.keywords.iter().any(|k| k == keyword) {
            return Ok(KeywordStatus::Invalid);
        }

        if !self.is_limited(keyword, sample_type) {
            return Ok(KeywordStatus::Unbounded);
        }

        let values = self
            .get_decimal_quantity_samples_typed(keyword, sample_type)?
            .expect("java.lang.NullPointerException: values");

        let original_size = values.len();

        let explicits: &[SharedDecimalQuantity] = explicits.unwrap_or(&[]);

        // Quick check on whether there are multiple elements

        if original_size > explicits.len() {
            if original_size == 1 {
                if let Some(u) = unique_value.as_mut() {
                    u.value = Some(values[0].clone());
                }
                return Ok(KeywordStatus::Unique);
            }
            return Ok(KeywordStatus::Bounded);
        }

        // Compute if the quick test is insufficient.

        let mut subtracted_set: Vec<SharedDecimalQuantity> = values.clone();
        for explicit in explicits {
            let explicit_bd = lock_dq(explicit).to_big_decimal()?;
            let val_to_remove_bd = explicit_bd.subtract(&BigDecimal::from_int(offset))?;
            let val_to_remove: SharedDecimalQuantity =
                share(Box::new(DecimalQuantity_DualStorageBCD::from_big_decimal(&val_to_remove_bd)?));
            // ArrayList.remove(Object): the first element equal to it.
            if let Some(pos) = subtracted_set.iter().position(|e| dq_equals(&val_to_remove, e)) {
                subtracted_set.remove(pos);
            }
        }
        if subtracted_set.is_empty() {
            return Ok(KeywordStatus::Suppressed);
        }

        if let Some(u) = unique_value.as_mut() {
            if subtracted_set.len() == 1 {
                u.value = Some(subtracted_set[0].clone());
            }
        }

        if original_size == 1 {
            Ok(KeywordStatus::Unique)
        } else {
            Ok(KeywordStatus::Bounded)
        }
    }

    /// Equivalent to `getRules(String)` (internal).
    pub fn get_rules(&self, keyword: &str) -> Option<String> {
        self.rules.get_rules(keyword)
    }

    /// Equivalent to `writeObject(ObjectOutputStream)`.
    ///
    /// # Errors
    ///
    /// Always `NotSerializableException`.
    pub fn write_object(&self) -> Result<()> {
        Err(not_serializable())
    }

    /// Equivalent to `readObject(ObjectInputStream)`.
    ///
    /// # Errors
    ///
    /// Always `NotSerializableException`.
    pub fn read_object(&self) -> Result<()> {
        Err(not_serializable())
    }

    /// Equivalent to `writeReplace()`: the serial proxy of the rules text.
    pub fn write_replace(&self) -> PluralRulesSerialProxy {
        PluralRulesSerialProxy::new(self.to_java_string())
    }

    /// Equivalent to `compareTo(PluralRules)` (internal): the comparison of
    /// the `toString()` renderings (`String.compareTo`, UTF-16 order).
    pub fn compare_to(&self, other: &PluralRules) -> i32 {
        let a = units(&self.to_java_string());
        let b = units(&other.to_java_string());
        let n = a.len().min(b.len());
        for k in 0..n {
            if a[k] != b[k] {
                return a[k] as i32 - b[k] as i32;
            }
        }
        a.len() as i32 - b.len() as i32
    }

    /// Equivalent to the package-private `isLimited(String)`.
    pub(crate) fn is_limited_integer(&self, keyword: &str) -> bool {
        self.rules.is_limited(keyword, SampleType::Integer)
    }

    /// Equivalent to `isLimited(String, SampleType)` (internal).
    pub fn is_limited(&self, keyword: &str, sample_type: SampleType) -> bool {
        self.rules.is_limited(keyword, sample_type)
    }

    /// Equivalent to `computeLimited(String, SampleType)` (internal).
    pub fn compute_limited(&self, keyword: &str, sample_type: SampleType) -> bool {
        self.rules.compute_limited(keyword, sample_type)
    }
}

impl fmt::Display for PluralRules {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_java_string())
    }
}

/// Adds `value` to the `LinkedHashSet<Double>` `set` (`Double.equals`: bit
/// equality of the canonical NaN).
fn set_add_double(set: &mut Vec<f64>, value: f64) {
    let key = |d: f64| if d.is_nan() { f64::NAN.to_bits() } else { d.to_bits() };
    if !set.iter().any(|v| key(*v) == key(value)) {
        set.push(value);
    }
}

/// `isValidKeyword(String)`.
fn is_valid_keyword(token: &str) -> bool {
    ALLOWED_ID.contains_all_str(&units(token))
}

impl PluralRules {
    /// Equivalent to the unused private `addConditional(Set<IFixedDecimal>,
    /// Set<IFixedDecimal>, double)`: adds `new FixedDecimal(trial)` to
    /// `others` when neither set contains it (`FixedDecimal.equals`).
    #[allow(dead_code)]
    fn add_conditional(&self, to_add_to: &[FixedDecimal], others: &mut Vec<FixedDecimal>, trial: f64) -> bool {
        let to_add = FixedDecimal::from_double(trial);
        if !to_add_to.contains(&to_add) && !others.contains(&to_add) {
            others.push(to_add);
            true
        } else {
            false
        }
    }

    #[allow(dead_code)]
    fn valid_keyword(token: &str) -> bool {
        Self::is_valid_keyword(token)
    }
}
