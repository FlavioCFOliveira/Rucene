// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html
/*
 *******************************************************************************
 * Copyright (C) 1996-2016, International Business Machines Corporation and
 * others. All Rights Reserved.
 *******************************************************************************
 */

//! `com.ibm.icu.text.DecimalFormatSymbols`.

use std::sync::{Arc, LazyLock};

use rucene::error::{LuceneError, Result};
use rucene_queryparser::jdk::locale::Locale;
use rucene_queryparser::jdk::resource_bundle::MissingResourceException;

use crate::jdk::{null_pointer, units};
use crate::r#impl::cache_base::CacheBase;
use crate::r#impl::currency_data::{self, CurrencyDisplayInfo, CurrencySpacingInfo};
use crate::r#impl::icu_data::ICUData;
use crate::r#impl::icu_resource_bundle::ICUResourceBundle;
use crate::r#impl::soft_cache::SoftCache;
use crate::r#impl::u_resource::{Key, Sink, Value};
use crate::text::numbering_system::NumberingSystem;
use crate::util::currency::Currency;
use crate::util::u_locale::{self, Category, ULocale};
use crate::util::u_resource_bundle::UResourceBundle;

/// `CURRENCY_SPC_CURRENCY_MATCH`: indicates the currency match pattern used
/// in getPatternForCurrencySpacing.
pub const CURRENCY_SPC_CURRENCY_MATCH: i32 = 0;
/// `CURRENCY_SPC_SURROUNDING_MATCH`: indicates the surrounding match pattern
/// used in getPatternForCurrencySpacing.
pub const CURRENCY_SPC_SURROUNDING_MATCH: i32 = 1;
/// `CURRENCY_SPC_INSERT`: indicates the insertion value used in
/// getPatternForCurrencySpacing.
pub const CURRENCY_SPC_INSERT: i32 = 2;

/// `SYMBOL_KEYS`: the keys of the symbols in `NumberElements`.
const SYMBOL_KEYS: [&str; 13] = [
    "decimal",
    "group",
    "percentSign",
    "minusSign",
    "plusSign",
    "exponential",
    "perMille",
    "infinity",
    "nan",
    "currencyDecimal",
    "currencyGroup",
    "superscriptingExponent",
    "approximatelySign",
];

/// `DEF_DIGIT_STRINGS_ARRAY`.
const DEF_DIGIT_STRINGS_ARRAY: [&str; 10] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];

/// `DEF_DIGIT_CHARS_ARRAY`.
const DEF_DIGIT_CHARS_ARRAY: [u16; 10] = [0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39];

/// `DEF_DECIMAL_SEPARATOR`.
const DEF_DECIMAL_SEPARATOR: u16 = b'.' as u16;
/// `DEF_GROUPING_SEPARATOR`.
const DEF_GROUPING_SEPARATOR: u16 = b',' as u16;
/// `DEF_PERCENT`.
const DEF_PERCENT: u16 = b'%' as u16;
/// `DEF_MINUS_SIGN`.
const DEF_MINUS_SIGN: u16 = b'-' as u16;
/// `DEF_PLUS_SIGN`.
const DEF_PLUS_SIGN: u16 = b'+' as u16;
/// `DEF_PERMILL`.
const DEF_PERMILL: u16 = 0x2030;

/// `SYMBOL_DEFAULTS`.
const SYMBOL_DEFAULTS: [Option<&str>; 13] = [
    Some("."),
    Some(","),
    Some("%"),
    Some("-"),
    Some("+"),
    Some("E"),
    Some("\u{2030}"),
    Some("\u{221e}"),
    Some("NaN"),
    None,
    None,
    Some("\u{00D7}"),
    Some("~"),
];

/// `LATIN_NUMBERING_SYSTEM`.
const LATIN_NUMBERING_SYSTEM: &str = "latn";
/// `NUMBER_ELEMENTS`.
const NUMBER_ELEMENTS: &str = "NumberElements";
/// `SYMBOLS`.
const SYMBOLS: &str = "symbols";

/// `serialVersionUID`.
pub const SERIAL_VERSION_UID: i64 = 5772796243397350300;

/// `currentSerialVersion`.
const CURRENT_SERIAL_VERSION: i32 = 10;

/// `cachedLocaleData`.
static CACHED_LOCALE_DATA: LazyLock<SoftCache<ULocale, Arc<CacheData>, ()>> =
    LazyLock::new(|| SoftCache::new(|locale: &ULocale, _unused: ()| DecimalFormatSymbols::load_data(locale).map(Some)));

/// The locale data `loadData` reads. Equivalent to
/// `DecimalFormatSymbols.CacheData`.
#[derive(Debug)]
struct CacheData {
    valid_locale: ULocale,
    digits: Vec<Vec<u16>>,
    number_elements: Vec<Vec<u16>>,
}

/// Sink for enumerating all of the decimal format symbols (more
/// specifically, anything under the "NumberElements.symbols" tree). Does
/// not override values already set. Equivalent to
/// `DecimalFormatSymbols.DecFmtDataSink`.
struct DecFmtDataSink<'a> {
    number_elements: &'a mut [Option<Vec<u16>>],
}

impl Sink for DecFmtDataSink<'_> {
    fn put(&mut self, key: &mut Key, value: &mut dyn Value, _no_fallback: bool) -> Result<()> {
        let symbols_table = value.get_table()?;
        let mut j = 0;
        while symbols_table.get_key_and_value(j, key, value) {
            for (i, symbol_key) in SYMBOL_KEYS.iter().enumerate() {
                if key.content_equals(symbol_key) {
                    if self.number_elements[i].is_none() {
                        self.number_elements[i] = Some(value.get_string()?);
                    }
                    break;
                }
            }
            j += 1;
        }
        Ok(())
    }
}

/// This class represents the set of symbols (such as the decimal separator,
/// the grouping separator, and so on) needed by `DecimalFormat` to format
/// numbers. `DecimalFormat` creates for itself an instance of
/// `DecimalFormatSymbols` from its locale data.
///
/// Equivalent to `com.ibm.icu.text.DecimalFormatSymbols`. `char` values are
/// UTF-16 units and `String` values UTF-16 texts; `clone()` is `Clone`
/// (Java's shallow copy, the setters replacing shared arrays before writing
/// them). Java serialization's `readObject` upgrade of older streams is
/// [`read_object`](Self::read_object).
#[derive(Debug, Clone)]
pub struct DecimalFormatSymbols {
    zero_digit: u16,
    digits: Vec<u16>,
    digit_strings: Vec<Vec<u16>>,
    code_point_zero: i32,
    grouping_separator: u16,
    grouping_separator_string: Vec<u16>,
    decimal_separator: u16,
    decimal_separator_string: Vec<u16>,
    per_mill: u16,
    per_mill_string: Vec<u16>,
    percent: u16,
    percent_string: Vec<u16>,
    digit: u16,
    sig_digit: u16,
    pattern_separator: u16,
    infinity: Vec<u16>,
    nan: Vec<u16>,
    minus_sign: u16,
    minus_string: Vec<u16>,
    plus_sign: u16,
    plus_string: Vec<u16>,
    approximately_string: Vec<u16>,
    currency_symbol: Vec<u16>,
    intl_currency_symbol: Vec<u16>,
    monetary_separator: u16,
    monetary_separator_string: Vec<u16>,
    monetary_grouping_separator: u16,
    monetary_grouping_separator_string: Vec<u16>,
    exponential: u16,
    exponent_separator: Vec<u16>,
    pad_escape: u16,
    requested_locale: Locale,
    ulocale: ULocale,
    exponent_multiplication_sign: Vec<u16>,
    serial_version_on_stream: i32,
    currency_pattern: Option<Vec<u16>>,
    valid_locale: Option<ULocale>,
    actual_locale: Option<ULocale>,
    currency: Option<Currency>,
    currency_spc_before_sym: Arc<Vec<Option<Vec<u16>>>>,
    currency_spc_after_sym: Arc<Vec<Option<Vec<u16>>>>,
}

impl DecimalFormatSymbols {
    /// An object with the initial field values of a Java instance before
    /// `initialize` runs.
    fn blank(locale: &ULocale) -> Self {
        Self {
            zero_digit: 0,
            digits: Vec::new(),
            digit_strings: Vec::new(),
            code_point_zero: 0,
            grouping_separator: 0,
            grouping_separator_string: Vec::new(),
            decimal_separator: 0,
            decimal_separator_string: Vec::new(),
            per_mill: 0,
            per_mill_string: Vec::new(),
            percent: 0,
            percent_string: Vec::new(),
            digit: 0,
            sig_digit: 0,
            pattern_separator: 0,
            infinity: Vec::new(),
            nan: Vec::new(),
            minus_sign: 0,
            minus_string: Vec::new(),
            plus_sign: 0,
            plus_string: Vec::new(),
            approximately_string: Vec::new(),
            currency_symbol: Vec::new(),
            intl_currency_symbol: Vec::new(),
            monetary_separator: 0,
            monetary_separator_string: Vec::new(),
            monetary_grouping_separator: 0,
            monetary_grouping_separator_string: Vec::new(),
            exponential: 0,
            exponent_separator: Vec::new(),
            pad_escape: 0,
            requested_locale: locale.to_locale(),
            ulocale: locale.clone(),
            exponent_multiplication_sign: Vec::new(),
            serial_version_on_stream: CURRENT_SERIAL_VERSION,
            currency_pattern: None,
            valid_locale: None,
            actual_locale: None,
            currency: None,
            currency_spc_before_sym: Arc::new(Vec::new()),
            currency_spc_after_sym: Arc::new(Vec::new()),
        }
    }

    /// Creates a DecimalFormatSymbols object for the default `FORMAT`
    /// locale. Equivalent to `DecimalFormatSymbols()`.
    ///
    /// # Errors
    ///
    /// The errors of the locale data.
    pub fn new() -> Result<Self> {
        Self::for_ulocale(&ULocale::get_default_category(Category::Format))
    }

    /// Creates a DecimalFormatSymbols object for the given locale.
    /// Equivalent to `DecimalFormatSymbols(Locale)`.
    ///
    /// # Errors
    ///
    /// The errors of the locale data.
    pub fn for_locale(locale: &Locale) -> Result<Self> {
        let ulocale = ULocale::for_locale(Some(locale))?.unwrap_or_else(ULocale::root);
        Self::for_ulocale(&ulocale)
    }

    /// Creates a DecimalFormatSymbols object for the given ulocale.
    /// Equivalent to `DecimalFormatSymbols(ULocale)`.
    ///
    /// # Errors
    ///
    /// The errors of the locale data.
    pub fn for_ulocale(locale: &ULocale) -> Result<Self> {
        let mut s = Self::blank(locale);
        s.initialize(locale, None)?;
        Ok(s)
    }

    /// `DecimalFormatSymbols(ULocale, NumberingSystem)`.
    fn for_ulocale_ns(locale: &ULocale, ns: &NumberingSystem) -> Result<Self> {
        let mut s = Self::blank(locale);
        s.initialize(locale, Some(ns))?;
        Ok(s)
    }

    /// Returns a DecimalFormatSymbols instance for the default locale.
    /// Equivalent to `getInstance()`.
    ///
    /// # Errors
    ///
    /// The errors of the locale data.
    pub fn get_instance() -> Result<DecimalFormatSymbols> {
        DecimalFormatSymbols::new()
    }

    /// Equivalent to `getInstance(Locale)`.
    ///
    /// # Errors
    ///
    /// The errors of the locale data.
    pub fn get_instance_locale(locale: &Locale) -> Result<DecimalFormatSymbols> {
        DecimalFormatSymbols::for_locale(locale)
    }

    /// Equivalent to `getInstance(ULocale)`.
    ///
    /// # Errors
    ///
    /// The errors of the locale data.
    pub fn get_instance_ulocale(locale: &ULocale) -> Result<DecimalFormatSymbols> {
        DecimalFormatSymbols::for_ulocale(locale)
    }

    /// Returns a DecimalFormatSymbols instance for the given locale with
    /// digits and symbols corresponding to the given NumberingSystem.
    /// Equivalent to `forNumberingSystem(Locale, NumberingSystem)`.
    ///
    /// # Errors
    ///
    /// The errors of the locale data.
    pub fn for_numbering_system_locale(locale: &Locale, ns: &NumberingSystem) -> Result<DecimalFormatSymbols> {
        let ulocale = ULocale::for_locale(Some(locale))?.unwrap_or_else(ULocale::root);
        DecimalFormatSymbols::for_ulocale_ns(&ulocale, ns)
    }

    /// Equivalent to `forNumberingSystem(ULocale, NumberingSystem)`.
    ///
    /// # Errors
    ///
    /// The errors of the locale data.
    pub fn for_numbering_system(locale: &ULocale, ns: &NumberingSystem) -> Result<DecimalFormatSymbols> {
        DecimalFormatSymbols::for_ulocale_ns(locale, ns)
    }

    /// Returns an array of all locales for which the getInstance methods of
    /// this class can return localized instances. Equivalent to
    /// `getAvailableLocales()`.
    ///
    /// # Errors
    ///
    /// The errors of the locale index data.
    pub fn get_available_locales() -> Result<Vec<Locale>> {
        Ok(ICUResourceBundle::get_available_locales()?.as_ref().clone())
    }

    /// Equivalent to `getAvailableULocales()`.
    ///
    /// # Errors
    ///
    /// The errors of the locale index data.
    pub fn get_available_u_locales() -> Result<Vec<ULocale>> {
        Ok(ICUResourceBundle::get_available_u_locales()?.unwrap_or_default())
    }

    /// Returns the character used for zero. Equivalent to `getZeroDigit()`.
    pub fn get_zero_digit(&self) -> u16 {
        self.zero_digit
    }

    /// Returns the array of characters used as digits, in order from 0
    /// through 9. Equivalent to `getDigits()`.
    pub fn get_digits(&self) -> Vec<u16> {
        self.digits.clone()
    }

    /// Sets the character used for zero; the next nine code units are the
    /// other digits. Equivalent to `setZeroDigit(char)`.
    pub fn set_zero_digit(&mut self, zero_digit: u16) {
        self.zero_digit = zero_digit;

        // digitStrings or digits might be referencing a cached copy for
        // optimization purposes, so we make a copy before making changes.
        let mut digit_strings = self.digit_strings.clone();
        let mut digits = self.digits.clone();

        // Make sure digitStrings field is updated when setting zero digit.
        digit_strings[0] = vec![zero_digit];
        digits[0] = zero_digit;

        // Always generate digits for setZeroDigit (to support
        // DecimalFormat).
        for i in 1..10u16 {
            let d = zero_digit.wrapping_add(i);
            digit_strings[i as usize] = vec![d];
            digits[i as usize] = d;
        }
        self.digit_strings = digit_strings;
        self.digits = digits;

        // Also generate codePointZero
        self.code_point_zero = zero_digit as i32;
    }

    /// Returns the array of strings used as digits, in order from 0 through
    /// 9. Equivalent to `getDigitStrings()`.
    pub fn get_digit_strings(&self) -> Vec<Vec<u16>> {
        self.digit_strings.clone()
    }

    /// Returns the array of strings used as digits, without copying it.
    /// Equivalent to `getDigitStringsLocal()` (internal).
    pub fn get_digit_strings_local(&self) -> &[Vec<u16>] {
        &self.digit_strings
    }

    /// Returns the code point corresponding to zero, or -1 when the digits
    /// are not ten consecutive code points. Equivalent to
    /// `getCodePointZero()` (internal).
    pub fn get_code_point_zero(&self) -> i32 {
        self.code_point_zero
    }

    /// Sets the array of strings used as digits, in order from 0 through 9.
    /// Equivalent to `setDigitStrings(String[])` (`null` elements are
    /// `None`).
    ///
    /// # Errors
    ///
    /// `NullPointerException("The input digit string array is null")`,
    /// `IllegalArgumentException("Number of digit strings is not 10")`,
    /// `IllegalArgumentException("The input digit string array contains a
    /// null element")`.
    pub fn set_digit_strings(&mut self, digit_strings: Option<&[Option<Vec<u16>>]>) -> Result<()> {
        let digit_strings = digit_strings.ok_or_else(|| null_pointer("The input digit string array is null"))?;
        if digit_strings.len() != 10 {
            return Err(LuceneError::illegal_argument("Number of digit strings is not 10"));
        }

        // Scan input array and create char[] representation if possible
        // Also update codePointZero if possible
        let mut tmp_digit_strings: Vec<Vec<u16>> = vec![Vec::new(); 10];
        let mut tmp_digits: Option<Vec<u16>> = Some(vec![0; 10]);
        let mut tmp_code_point_zero = -1;
        for i in 0..10 {
            let digit_str = match &digit_strings[i] {
                None => {
                    return Err(LuceneError::illegal_argument(
                        "The input digit string array contains a null element",
                    ));
                }
                Some(s) => s,
            };
            tmp_digit_strings[i] = digit_str.clone();
            let (cp, cc) = if digit_str.is_empty() {
                (-1, 0)
            } else {
                let cp = crate::jdk::code_point_at(digit_str, 0);
                (cp, crate::jdk::char_count(cp))
            };
            if cc == digit_str.len() as i32 {
                // One code point in this digit.
                // If it is 1 UTF-16 code unit long, set it in tmpDigits.
                if cc == 1 && tmp_digits.is_some() {
                    tmp_digits.as_mut().expect("checked")[i] = cp as u16;
                } else {
                    tmp_digits = None;
                }

                if i == 0 {
                    tmp_code_point_zero = cp;
                } else if cp != tmp_code_point_zero + i as i32 {
                    tmp_code_point_zero = -1;
                }
            } else {
                // More than one code point in this digit.
                // codePointZero and tmpDigits are going to be invalid.
                tmp_code_point_zero = -1;
                tmp_digits = None;
            }
        }

        self.digit_strings = tmp_digit_strings;
        self.code_point_zero = tmp_code_point_zero;

        match tmp_digits {
            None => {
                // fallback to the default digit chars
                self.zero_digit = DEF_DIGIT_CHARS_ARRAY[0];
                self.digits = DEF_DIGIT_CHARS_ARRAY.to_vec();
            }
            Some(d) => {
                self.zero_digit = d[0];
                self.digits = d;
            }
        }
        Ok(())
    }

    /// Returns the character used to represent a significant digit in a
    /// pattern. Equivalent to `getSignificantDigit()`.
    pub fn get_significant_digit(&self) -> u16 {
        self.sig_digit
    }

    /// Sets the character used to represent a significant digit in a
    /// pattern. Equivalent to `setSignificantDigit(char)`.
    pub fn set_significant_digit(&mut self, sig_digit: u16) {
        self.sig_digit = sig_digit;
    }

    /// Returns the character used for grouping separator. Equivalent to
    /// `getGroupingSeparator()`.
    pub fn get_grouping_separator(&self) -> u16 {
        self.grouping_separator
    }

    /// Sets the character used for grouping separator. Equivalent to
    /// `setGroupingSeparator(char)`.
    pub fn set_grouping_separator(&mut self, grouping_separator: u16) {
        self.grouping_separator = grouping_separator;
        self.grouping_separator_string = vec![grouping_separator];
    }

    /// Returns the string used for grouping separator. Equivalent to
    /// `getGroupingSeparatorString()`.
    pub fn get_grouping_separator_string(&self) -> &[u16] {
        &self.grouping_separator_string
    }

    /// Sets the string used for grouping separator. Equivalent to
    /// `setGroupingSeparatorString(String)`.
    ///
    /// # Errors
    ///
    /// `NullPointerException("The input grouping separator is null")`.
    pub fn set_grouping_separator_string(&mut self, grouping_separator_string: Option<&[u16]>) -> Result<()> {
        let s = grouping_separator_string.ok_or_else(|| null_pointer("The input grouping separator is null"))?;
        self.grouping_separator_string = s.to_vec();
        if s.len() == 1 {
            self.grouping_separator = s[0];
        } else {
            // Use the default grouping separator character as fallback
            self.grouping_separator = DEF_GROUPING_SEPARATOR;
        }
        Ok(())
    }

    /// Returns the character used for decimal sign. Equivalent to
    /// `getDecimalSeparator()`.
    pub fn get_decimal_separator(&self) -> u16 {
        self.decimal_separator
    }

    /// Sets the character used for decimal sign. Equivalent to
    /// `setDecimalSeparator(char)`.
    pub fn set_decimal_separator(&mut self, decimal_separator: u16) {
        self.decimal_separator = decimal_separator;
        self.decimal_separator_string = vec![decimal_separator];
    }

    /// Returns the string used for decimal sign. Equivalent to
    /// `getDecimalSeparatorString()`.
    pub fn get_decimal_separator_string(&self) -> &[u16] {
        &self.decimal_separator_string
    }

    /// Sets the string used for decimal sign. Equivalent to
    /// `setDecimalSeparatorString(String)`.
    ///
    /// # Errors
    ///
    /// `NullPointerException("The input decimal separator is null")`.
    pub fn set_decimal_separator_string(&mut self, decimal_separator_string: Option<&[u16]>) -> Result<()> {
        let s = decimal_separator_string.ok_or_else(|| null_pointer("The input decimal separator is null"))?;
        self.decimal_separator_string = s.to_vec();
        if s.len() == 1 {
            self.decimal_separator = s[0];
        } else {
            // Use the default decimal separator character as fallback
            self.decimal_separator = DEF_DECIMAL_SEPARATOR;
        }
        Ok(())
    }

    /// Returns the character used for mille percent sign. Equivalent to
    /// `getPerMill()`.
    pub fn get_per_mill(&self) -> u16 {
        self.per_mill
    }

    /// Sets the character used for mille percent sign. Equivalent to
    /// `setPerMill(char)`.
    pub fn set_per_mill(&mut self, per_mill: u16) {
        self.per_mill = per_mill;
        self.per_mill_string = vec![per_mill];
    }

    /// Returns the string used for permille sign. Equivalent to
    /// `getPerMillString()`.
    pub fn get_per_mill_string(&self) -> &[u16] {
        &self.per_mill_string
    }

    /// Sets the string used for permille sign. Equivalent to
    /// `setPerMillString(String)`.
    ///
    /// # Errors
    ///
    /// `NullPointerException("The input permille string is null")`.
    pub fn set_per_mill_string(&mut self, per_mill_string: Option<&[u16]>) -> Result<()> {
        let s = per_mill_string.ok_or_else(|| null_pointer("The input permille string is null"))?;
        self.per_mill_string = s.to_vec();
        if s.len() == 1 {
            self.per_mill = s[0];
        } else {
            // Use the default permille character as fallback
            self.per_mill = DEF_PERMILL;
        }
        Ok(())
    }

    /// Returns the character used for percent sign. Equivalent to
    /// `getPercent()`.
    pub fn get_percent(&self) -> u16 {
        self.percent
    }

    /// Sets the character used for percent sign. Equivalent to
    /// `setPercent(char)`.
    pub fn set_percent(&mut self, percent: u16) {
        self.percent = percent;
        self.percent_string = vec![percent];
    }

    /// Returns the string used for percent sign. Equivalent to
    /// `getPercentString()`.
    pub fn get_percent_string(&self) -> &[u16] {
        &self.percent_string
    }

    /// Sets the string used for percent sign. Equivalent to
    /// `setPercentString(String)`.
    ///
    /// # Errors
    ///
    /// `NullPointerException("The input percent sign is null")`.
    pub fn set_percent_string(&mut self, percent_string: Option<&[u16]>) -> Result<()> {
        let s = percent_string.ok_or_else(|| null_pointer("The input percent sign is null"))?;
        self.percent_string = s.to_vec();
        if s.len() == 1 {
            self.percent = s[0];
        } else {
            // Use default percent character as fallback
            self.percent = DEF_PERCENT;
        }
        Ok(())
    }

    /// Returns the character used for a digit in a pattern. Equivalent to
    /// `getDigit()`.
    pub fn get_digit(&self) -> u16 {
        self.digit
    }

    /// Sets the character used for a digit in a pattern. Equivalent to
    /// `setDigit(char)`.
    pub fn set_digit(&mut self, digit: u16) {
        self.digit = digit;
    }

    /// Returns the character used to separate positive and negative
    /// subpatterns in a pattern. Equivalent to `getPatternSeparator()`.
    pub fn get_pattern_separator(&self) -> u16 {
        self.pattern_separator
    }

    /// Sets the character used to separate positive and negative
    /// subpatterns in a pattern. Equivalent to `setPatternSeparator(char)`.
    pub fn set_pattern_separator(&mut self, pattern_separator: u16) {
        self.pattern_separator = pattern_separator;
    }

    /// Returns the String used to represent infinity. Equivalent to
    /// `getInfinity()`.
    pub fn get_infinity(&self) -> &[u16] {
        &self.infinity
    }

    /// Sets the String used to represent infinity. Equivalent to
    /// `setInfinity(String)`.
    pub fn set_infinity(&mut self, infinity: &[u16]) {
        self.infinity = infinity.to_vec();
    }

    /// Returns the String used to represent NaN. Equivalent to `getNaN()`.
    pub fn get_nan(&self) -> &[u16] {
        &self.nan
    }

    /// Sets the String used to represent NaN. Equivalent to `setNaN(String)`.
    pub fn set_nan(&mut self, nan: &[u16]) {
        self.nan = nan.to_vec();
    }

    /// Returns the character used to represent minus sign. Equivalent to
    /// `getMinusSign()`.
    pub fn get_minus_sign(&self) -> u16 {
        self.minus_sign
    }

    /// Sets the character used to represent minus sign. Equivalent to
    /// `setMinusSign(char)`.
    pub fn set_minus_sign(&mut self, minus_sign: u16) {
        self.minus_sign = minus_sign;
        self.minus_string = vec![minus_sign];
    }

    /// Returns the string used to represent minus sign. Equivalent to
    /// `getMinusSignString()`.
    pub fn get_minus_sign_string(&self) -> &[u16] {
        &self.minus_string
    }

    /// Sets the string used to represent minus sign. Equivalent to
    /// `setMinusSignString(String)`.
    ///
    /// # Errors
    ///
    /// `NullPointerException("The input minus sign is null")`.
    pub fn set_minus_sign_string(&mut self, minus_sign_string: Option<&[u16]>) -> Result<()> {
        let s = minus_sign_string.ok_or_else(|| null_pointer("The input minus sign is null"))?;
        self.minus_string = s.to_vec();
        if s.len() == 1 {
            self.minus_sign = s[0];
        } else {
            // Use the default minus sign as fallback
            self.minus_sign = DEF_MINUS_SIGN;
        }
        Ok(())
    }

    /// Returns the localized plus sign. Equivalent to `getPlusSign()`.
    pub fn get_plus_sign(&self) -> u16 {
        self.plus_sign
    }

    /// Sets the localized plus sign. Equivalent to `setPlusSign(char)`.
    pub fn set_plus_sign(&mut self, plus: u16) {
        self.plus_sign = plus;
        self.plus_string = vec![plus];
    }

    /// Returns the string used to represent plus sign. Equivalent to
    /// `getPlusSignString()`.
    pub fn get_plus_sign_string(&self) -> &[u16] {
        &self.plus_string
    }

    /// Sets the localized plus sign string. Equivalent to
    /// `setPlusSignString(String)`.
    ///
    /// # Errors
    ///
    /// `NullPointerException("The input plus sign is null")`.
    pub fn set_plus_sign_string(&mut self, plus_sign_string: Option<&[u16]>) -> Result<()> {
        let s = plus_sign_string.ok_or_else(|| null_pointer("The input plus sign is null"))?;
        self.plus_string = s.to_vec();
        if s.len() == 1 {
            self.plus_sign = s[0];
        } else {
            // Use the default plus sign as fallback
            self.plus_sign = DEF_PLUS_SIGN;
        }
        Ok(())
    }

    /// Returns the string used to represent approximately sign. Equivalent
    /// to `getApproximatelySignString()` (internal).
    pub fn get_approximately_sign_string(&self) -> &[u16] {
        &self.approximately_string
    }

    /// Sets the approximately sign string. Equivalent to
    /// `setApproximatelySignString(String)` (internal).
    ///
    /// # Errors
    ///
    /// `NullPointerException("The input plus sign is null")` (sic).
    pub fn set_approximately_sign_string(&mut self, approximately_sign_string: Option<&[u16]>) -> Result<()> {
        let s = approximately_sign_string.ok_or_else(|| null_pointer("The input plus sign is null"))?;
        self.approximately_string = s.to_vec();
        Ok(())
    }

    /// Returns the string denoting the local currency. Equivalent to
    /// `getCurrencySymbol()`.
    pub fn get_currency_symbol(&self) -> Vec<u16> {
        self.currency_symbol.clone()
    }

    /// Sets the string denoting the local currency. Equivalent to
    /// `setCurrencySymbol(String)`.
    pub fn set_currency_symbol(&mut self, currency: &[u16]) {
        self.currency_symbol = currency.to_vec();
    }

    /// Returns the international string denoting the local currency.
    /// Equivalent to `getInternationalCurrencySymbol()`.
    pub fn get_international_currency_symbol(&self) -> Vec<u16> {
        self.intl_currency_symbol.clone()
    }

    /// Sets the international string denoting the local currency.
    /// Equivalent to `setInternationalCurrencySymbol(String)`.
    pub fn set_international_currency_symbol(&mut self, currency: &[u16]) {
        self.intl_currency_symbol = currency.to_vec();
    }

    /// Returns the currency symbol, for JDK 1.4 compatibility only.
    /// Equivalent to `getCurrency()`.
    pub fn get_currency(&self) -> Option<Currency> {
        self.currency.clone()
    }

    /// Sets the currency, and the symbols, pattern and separators the
    /// locale's data gives it. Equivalent to `setCurrency(Currency)`.
    ///
    /// # Errors
    ///
    /// `NullPointerException` for a `None` currency, and the errors of the
    /// currency data.
    pub fn set_currency(&mut self, currency: Option<Currency>) -> Result<()> {
        let currency = currency.ok_or_else(LuceneError::null_pointer)?;
        if let Some(own) = &self.currency {
            if currency.java_equals(own) {
                return Ok(());
            }
        }
        let display_info = Self::display_info(&self.ulocale)?;
        self.set_currency_or_null(Some(currency), display_info.as_ref())
    }

    /// `CurrencyData.provider.getInstance(ulocale, true)`, which never
    /// answers `null` with fallback.
    fn display_info(ulocale: &ULocale) -> Result<Arc<dyn CurrencyDisplayInfo>> {
        currency_data::provider()
            .get_instance(Some(ulocale), true)?
            .ok_or_else(|| LuceneError::null_pointer_with_message("CurrencyData.provider.getInstance"))
    }

    /// `setCurrencyOrNull(Currency, CurrencyDisplayInfo)`.
    fn set_currency_or_null(&mut self, currency: Option<Currency>, display_info: &dyn CurrencyDisplayInfo) -> Result<()> {
        self.currency = currency.clone();

        let currency = match currency {
            None => {
                self.intl_currency_symbol = units("XXX");
                self.currency_symbol = units("\u{00A4}");
                self.currency_pattern = None;
                return Ok(());
            }
            Some(c) => c,
        };

        self.intl_currency_symbol = units(&currency.get_currency_code());
        self.currency_symbol = match currency.get_symbol_ulocale(&self.ulocale)? {
            Some(s) => units(&s),
            None => panic!("java.lang.NullPointerException: Currency.getSymbol"),
        };

        if let Some(format_info) = display_info.get_format_info(&currency.get_currency_code())? {
            self.set_monetary_decimal_separator_string(Some(&units(&format_info.monetary_decimal_separator)))?;
            self.set_monetary_grouping_separator_string(Some(&units(&format_info.monetary_grouping_separator)))?;
            self.currency_pattern = Some(units(&format_info.currency_pattern));
        }
        Ok(())
    }

    /// Returns the monetary decimal separator. Equivalent to
    /// `getMonetaryDecimalSeparator()`.
    pub fn get_monetary_decimal_separator(&self) -> u16 {
        self.monetary_separator
    }

    /// Sets the monetary decimal separator. Equivalent to
    /// `setMonetaryDecimalSeparator(char)`.
    pub fn set_monetary_decimal_separator(&mut self, sep: u16) {
        self.monetary_separator = sep;
        self.monetary_separator_string = vec![sep];
    }

    /// Returns the monetary decimal separator string. Equivalent to
    /// `getMonetaryDecimalSeparatorString()`.
    pub fn get_monetary_decimal_separator_string(&self) -> &[u16] {
        &self.monetary_separator_string
    }

    /// Sets the monetary decimal separator string. Equivalent to
    /// `setMonetaryDecimalSeparatorString(String)`.
    ///
    /// # Errors
    ///
    /// `NullPointerException("The input monetary decimal separator is
    /// null")`.
    pub fn set_monetary_decimal_separator_string(&mut self, sep: Option<&[u16]>) -> Result<()> {
        let sep = sep.ok_or_else(|| null_pointer("The input monetary decimal separator is null"))?;
        self.monetary_separator_string = sep.to_vec();
        if sep.len() == 1 {
            self.monetary_separator = sep[0];
        } else {
            // Use default decimap separator character as fallbacl
            self.monetary_separator = DEF_DECIMAL_SEPARATOR;
        }
        Ok(())
    }

    /// Returns the monetary grouping separator. Equivalent to
    /// `getMonetaryGroupingSeparator()`.
    pub fn get_monetary_grouping_separator(&self) -> u16 {
        self.monetary_grouping_separator
    }

    /// Sets the monetary grouping separator. Equivalent to
    /// `setMonetaryGroupingSeparator(char)`.
    pub fn set_monetary_grouping_separator(&mut self, sep: u16) {
        self.monetary_grouping_separator = sep;
        self.monetary_grouping_separator_string = vec![sep];
    }

    /// Returns the monetary grouping separator string. Equivalent to
    /// `getMonetaryGroupingSeparatorString()`.
    pub fn get_monetary_grouping_separator_string(&self) -> &[u16] {
        &self.monetary_grouping_separator_string
    }

    /// Sets the monetary grouping separator string. Equivalent to
    /// `setMonetaryGroupingSeparatorString(String)`.
    ///
    /// # Errors
    ///
    /// `NullPointerException("The input monetary grouping separator is
    /// null")`.
    pub fn set_monetary_grouping_separator_string(&mut self, sep: Option<&[u16]>) -> Result<()> {
        let sep = sep.ok_or_else(|| null_pointer("The input monetary grouping separator is null"))?;
        self.monetary_grouping_separator_string = sep.to_vec();
        if sep.len() == 1 {
            self.monetary_grouping_separator = sep[0];
        } else {
            // Use default grouping separator character as fallback
            self.monetary_grouping_separator = DEF_GROUPING_SEPARATOR;
        }
        Ok(())
    }

    /// Returns the currency pattern of the currency set (`None` when there
    /// is none). Equivalent to `getCurrencyPattern()` (internal).
    pub fn get_currency_pattern(&self) -> Option<&[u16]> {
        self.currency_pattern.as_deref()
    }

    /// Returns the multiplication sign. Equivalent to
    /// `getExponentMultiplicationSign()`.
    pub fn get_exponent_multiplication_sign(&self) -> &[u16] {
        &self.exponent_multiplication_sign
    }

    /// Sets the multiplication sign. Equivalent to
    /// `setExponentMultiplicationSign(String)`.
    pub fn set_exponent_multiplication_sign(&mut self, exponent_multiplication_sign: &[u16]) {
        self.exponent_multiplication_sign = exponent_multiplication_sign.to_vec();
    }

    /// Returns the string used to separate the mantissa from the exponent.
    /// Equivalent to `getExponentSeparator()`.
    pub fn get_exponent_separator(&self) -> &[u16] {
        &self.exponent_separator
    }

    /// Sets the string used to separate the mantissa from the exponent.
    /// Equivalent to `setExponentSeparator(String)`.
    pub fn set_exponent_separator(&mut self, exp: &[u16]) {
        self.exponent_separator = exp.to_vec();
    }

    /// Returns the character used to pad numbers out to a specified width.
    /// Equivalent to `getPadEscape()`.
    pub fn get_pad_escape(&self) -> u16 {
        self.pad_escape
    }

    /// Sets the character used to pad numbers out to a specified width.
    /// Equivalent to `setPadEscape(char)`.
    pub fn set_pad_escape(&mut self, c: u16) {
        self.pad_escape = c;
    }

    /// Returns the desired currency spacing value (`None` for Java's
    /// `null`). Equivalent to `getPatternForCurrencySpacing(int, boolean)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("unknown currency spacing: <itemType>")`.
    pub fn get_pattern_for_currency_spacing(&self, item_type: i32, before_currency: bool) -> Result<Option<&[u16]>> {
        if !(CURRENCY_SPC_CURRENCY_MATCH..=CURRENCY_SPC_INSERT).contains(&item_type) {
            return Err(LuceneError::illegal_argument(format!("unknown currency spacing: {item_type}")));
        }
        let symbols = if before_currency { &self.currency_spc_before_sym } else { &self.currency_spc_after_sym };
        Ok(symbols[item_type as usize].as_deref())
    }

    /// Sets the indicated currency spacing pattern or value. Equivalent to
    /// `setPatternForCurrencySpacing(int, boolean, String)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("unknown currency spacing: <itemType>")`.
    pub fn set_pattern_for_currency_spacing(&mut self, item_type: i32, before_currency: bool, pattern: Option<&[u16]>) -> Result<()> {
        if !(CURRENCY_SPC_CURRENCY_MATCH..=CURRENCY_SPC_INSERT).contains(&item_type) {
            return Err(LuceneError::illegal_argument(format!("unknown currency spacing: {item_type}")));
        }
        let target = if before_currency { &mut self.currency_spc_before_sym } else { &mut self.currency_spc_after_sym };
        let mut copy = target.as_ref().clone();
        copy[item_type as usize] = pattern.map(<[u16]>::to_vec);
        *target = Arc::new(copy);
        Ok(())
    }

    /// Returns the locale for which this object was constructed. Equivalent
    /// to `getLocale()`.
    pub fn get_locale(&self) -> &Locale {
        &self.requested_locale
    }

    /// Returns the locale for which this object was constructed. Equivalent
    /// to `getULocale()`.
    pub fn get_u_locale(&self) -> ULocale {
        self.ulocale.clone()
    }

    /// Equivalent to `equals(Object)`.
    pub fn java_equals(&self, other: &DecimalFormatSymbols) -> bool {
        if std::ptr::eq(self, other) {
            return true;
        }
        for i in 0..=CURRENCY_SPC_INSERT as usize {
            if !java_string_equals(&self.currency_spc_before_sym[i], &other.currency_spc_before_sym[i]) {
                return false;
            }
            if !java_string_equals(&self.currency_spc_after_sym[i], &other.currency_spc_after_sym[i]) {
                return false;
            }
        }

        // `other.digits == null` cannot hold: the digits are always set.
        if self.digits != other.digits {
            return false;
        }

        self.grouping_separator == other.grouping_separator
            && self.decimal_separator == other.decimal_separator
            && self.percent == other.percent
            && self.per_mill == other.per_mill
            && self.digit == other.digit
            && self.minus_sign == other.minus_sign
            && self.minus_string == other.minus_string
            && self.pattern_separator == other.pattern_separator
            && self.infinity == other.infinity
            && self.nan == other.nan
            && self.currency_symbol == other.currency_symbol
            && self.intl_currency_symbol == other.intl_currency_symbol
            && self.pad_escape == other.pad_escape
            && self.plus_sign == other.plus_sign
            && self.plus_string == other.plus_string
            && self.approximately_string == other.approximately_string
            && self.exponent_separator == other.exponent_separator
            && self.monetary_separator == other.monetary_separator
            && self.monetary_grouping_separator == other.monetary_grouping_separator
            && self.exponent_multiplication_sign == other.exponent_multiplication_sign
    }

    /// Equivalent to `hashCode()`.
    pub fn hash_code(&self) -> i32 {
        let mut result = self.digits[0] as i32;
        result = result.wrapping_mul(37).wrapping_add(self.grouping_separator as i32);
        result = result.wrapping_mul(37).wrapping_add(self.decimal_separator as i32);
        result
    }

    /// Initializes the symbols from the locale data. Equivalent to
    /// `initialize(ULocale, NumberingSystem)`.
    fn initialize(&mut self, locale: &ULocale, ns: Option<&NumberingSystem>) -> Result<()> {
        self.requested_locale = locale.to_locale();
        self.ulocale = locale.clone();

        // TODO: The cache requires a single key, so we just save the
        // NumberingSystem into the locale string. NumberingSystem is then
        // decoded again in the loadData() method. It would be more
        // efficient if we didn't have to serialize and deserialize the
        // NumberingSystem.
        let key_locale = match ns {
            None => locale.clone(),
            Some(ns) => locale.set_keyword_value(Some("numbers"), ns.get_name())?,
        };
        let data = CACHED_LOCALE_DATA
            .get_instance(key_locale, ())?
            .expect("loadData always answers data");

        self.set_locale(Some(data.valid_locale.clone()), Some(data.valid_locale.clone()))?;
        let digits: Vec<Option<Vec<u16>>> = data.digits.iter().cloned().map(Some).collect();
        self.set_digit_strings(Some(&digits))?;
        let number_elements = &data.number_elements;

        // Copy data from the numberElements map into instance fields
        self.set_decimal_separator_string(Some(&number_elements[0]))?;
        self.set_grouping_separator_string(Some(&number_elements[1]))?;

        // See CLDR #9781
        // assert numberElements[2].length() == 1;
        self.pattern_separator = b';' as u16;
        self.set_percent_string(Some(&number_elements[2]))?;
        self.set_minus_sign_string(Some(&number_elements[3]))?;
        self.set_plus_sign_string(Some(&number_elements[4]))?;
        self.set_exponent_separator(&number_elements[5]);
        self.set_per_mill_string(Some(&number_elements[6]))?;
        self.set_infinity(&number_elements[7]);
        self.set_nan(&number_elements[8]);
        self.set_monetary_decimal_separator_string(Some(&number_elements[9]))?;
        self.set_monetary_grouping_separator_string(Some(&number_elements[10]))?;
        self.set_exponent_multiplication_sign(&number_elements[11]);
        self.set_approximately_sign_string(Some(&number_elements[12]))?;

        self.digit = b'#' as u16; // Localized pattern character no longer in CLDR
        self.pad_escape = b'*' as u16;
        self.sig_digit = b'@' as u16;

        let display_info = Self::display_info(&self.ulocale)?;
        let spacing = display_info
            .get_spacing_info()?
            .unwrap_or_else(|| panic!("java.lang.NullPointerException: getSpacingInfo"));
        self.init_spacing_info(&spacing);

        let currency = Currency::get_instance_ulocale(&self.ulocale)?;
        self.set_currency_or_null(currency, display_info.as_ref())
    }

    /// `loadData(ULocale)`.
    fn load_data(locale: &ULocale) -> Result<Arc<CacheData>> {
        let ns_name: String;
        // Attempt to set the zero digit based on the numbering system for
        // the locale requested
        let ns = NumberingSystem::get_instance_for(locale)?;
        let digits: Vec<Vec<u16>>;
        if ns.get_radix() == 10 && !ns.is_algorithmic() && NumberingSystem::is_valid_digit_string(ns.get_description()) {
            let digit_string = ns.get_description();
            let mut d = Vec::with_capacity(10);
            let mut offset = 0;
            for _ in 0..10 {
                let cp = crate::jdk::code_point_at(digit_string, offset);
                let next_offset = offset + crate::jdk::char_count(cp);
                d.push(digit_string[offset as usize..next_offset as usize].to_vec());
                offset = next_offset;
            }
            digits = d;
            ns_name = ns.get_name().unwrap_or("null").to_string();
        } else {
            // Default numbering system
            digits = DEF_DIGIT_STRINGS_ARRAY.iter().map(|s| units(s)).collect();
            ns_name = "latn".to_string();
        }

        // Open the resource bundle and get the locale IDs.
        // TODO: Is there a better way to get the locale than making an
        // ICUResourceBundle?
        let rb = UResourceBundle::get_bundle_instance_for(Some(ICUData::ICU_BASE_NAME), Some(locale))?;
        let rb = rb.as_icu()?.clone();

        // TODO: Determine actual and valid locale correctly.
        let valid_locale = rb.get_u_locale();

        let mut number_elements: Vec<Option<Vec<u16>>> = vec![None; SYMBOL_KEYS.len()];

        // Load using a data sink
        {
            let mut sink = DecFmtDataSink { number_elements: &mut number_elements };
            match rb.get_all_items_with_fallback(&format!("{NUMBER_ELEMENTS}/{ns_name}/{SYMBOLS}"), &mut sink) {
                Ok(()) => {}
                Err(e) if MissingResourceException::of(&e).is_some() => {
                    // The symbols don't exist for the given nsName and
                    // resource bundle. Silently ignore and fall back to
                    // Latin.
                }
                Err(e) => return Err(e),
            }
        }

        // Load the Latin fallback if necessary
        let has_null = number_elements.iter().any(Option::is_none);
        if has_null && ns_name != LATIN_NUMBERING_SYSTEM {
            let mut sink = DecFmtDataSink { number_elements: &mut number_elements };
            rb.get_all_items_with_fallback(&format!("{NUMBER_ELEMENTS}/{LATIN_NUMBERING_SYSTEM}/{SYMBOLS}"), &mut sink)?;
        }

        // Fill in any remaining missing values
        for i in 0..SYMBOL_KEYS.len() {
            if number_elements[i].is_none() {
                number_elements[i] = SYMBOL_DEFAULTS[i].map(units);
            }
        }

        // If monetary decimal or grouping were not explicitly set, then set
        // them to be the same as their non-monetary counterparts.
        if number_elements[9].is_none() {
            number_elements[9] = number_elements[0].clone();
        }
        if number_elements[10].is_none() {
            number_elements[10] = number_elements[1].clone();
        }

        let number_elements: Vec<Vec<u16>> =
            number_elements.into_iter().map(|e| e.expect("every element is set above")).collect();
        Ok(Arc::new(CacheData { valid_locale, digits, number_elements }))
    }

    /// Obtains currency spacing patterns from the spacing info. Equivalent
    /// to `initSpacingInfo(CurrencySpacingInfo)`.
    fn init_spacing_info(&mut self, spc_info: &CurrencySpacingInfo) {
        self.currency_spc_before_sym =
            Arc::new(spc_info.get_before_symbols().iter().map(|s| s.as_deref().map(units)).collect());
        self.currency_spc_after_sym =
            Arc::new(spc_info.get_after_symbols().iter().map(|s| s.as_deref().map(units)).collect());
    }

    /// Reads the default serializable fields, then if
    /// `serialVersionOnStream` is less than the current version, initializes
    /// the fields the older version did not have. Equivalent to
    /// `readObject(ObjectInputStream)` after `defaultReadObject()`, whose
    /// fields this object already holds: the upgrades of the stream versions
    /// below `serial_version_on_stream` are applied in Java's order (the
    /// `null` tests of fields an older stream lacks find none here, every
    /// field of this object being set).
    ///
    /// # Errors
    ///
    /// The errors of `Currency.getInstance` and of the currency data.
    pub fn read_object(&mut self, serial_version_on_stream: i32) -> Result<()> {
        self.serial_version_on_stream = serial_version_on_stream;
        // the following arguments are added in version 1
        if self.serial_version_on_stream < 1 {
            // Didn't have monetarySeparator or exponential field;
            // use defaults.
            self.monetary_separator = self.decimal_separator;
            self.exponential = b'E' as u16;
        }
        if self.serial_version_on_stream < 2 {
            // Didn't have padEscape, plusSign, or exponentSeparator
            self.pad_escape = b'*' as u16;
            self.plus_sign = b'+' as u16;
            self.exponent_separator = vec![self.exponential];
            // Although we read the exponential field on stream to create the
            // exponentSeparator, we don't do the reverse, since scripts
            // like Arabic use a two-character exponent separator.
        }
        if self.serial_version_on_stream < 3 {
            // Resurrected objects from old streams will have no locale.
            // There is no 100% fix for this.  A great majority of objects
            // will be resurrected with the default locale.
            self.requested_locale = Locale::get_default();
        }
        if self.serial_version_on_stream < 4 {
            // use same default behavior as for versions with no Locale
            self.ulocale = ULocale::for_locale(Some(&self.requested_locale))?.unwrap_or_else(ULocale::root);
        }
        if self.serial_version_on_stream < 5 {
            // use the same one for groupingSeparator
            self.monetary_grouping_separator = self.grouping_separator;
        }
        if self.serial_version_on_stream < 6 {
            // Set null to CurrencySpacing related fields.
            let empty: Vec<Option<Vec<u16>>> = vec![None; (CURRENCY_SPC_INSERT + 1) as usize];
            if self.currency_spc_before_sym.is_empty() {
                self.currency_spc_before_sym = Arc::new(empty.clone());
            }
            if self.currency_spc_after_sym.is_empty() {
                self.currency_spc_after_sym = Arc::new(empty);
            }
            self.init_spacing_info(&CurrencySpacingInfo::default_info());
        }
        if self.serial_version_on_stream < 9 {
            // String version of digits and separators are added in
            // version 9. The digit strings, separator strings and the
            // multiplication sign are set on this object (the `null` tests
            // of version 7, 8 and 9 do not apply).
        }
        if self.serial_version_on_stream < 10 {
            // approximately sign added in version 10
            self.approximately_string = units("~");
        }

        self.serial_version_on_stream = CURRENT_SERIAL_VERSION;

        // recreate
        self.currency = Some(Currency::get_instance(Some(&crate::jdk::to_rust_string(&self.intl_currency_symbol)))?);

        // Refresh digitStrings in order to populate codePointZero.
        let digits: Vec<Option<Vec<u16>>> = self.digit_strings.iter().cloned().map(Some).collect();
        self.set_digit_strings(Some(&digits))
    }

    /// Returns the locale that was used to create this object, or `None`.
    /// Equivalent to `getLocale(ULocale.Type)`.
    pub fn get_locale_type(&self, type_: u_locale::Type) -> Option<&ULocale> {
        if type_ == u_locale::ACTUAL_LOCALE {
            self.actual_locale.as_ref()
        } else {
            self.valid_locale.as_ref()
        }
    }

    /// Sets information about the locales that were used to create this
    /// object. Equivalent to the package-private `setLocale(ULocale,
    /// ULocale)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when exactly one of them is `None`.
    pub(crate) fn set_locale(&mut self, valid: Option<ULocale>, actual: Option<ULocale>) -> Result<()> {
        // Change the following to an assertion later
        if valid.is_none() != actual.is_none() {
            return Err(crate::jdk::illegal_argument_no_message());
        }
        // Another check we could do is that the actual locale is at
        // the same level or less specific than the valid locale.
        self.valid_locale = valid;
        self.actual_locale = actual;
        Ok(())
    }
}

/// `String.equals` between two (nullable) strings, `a` dereferenced as Java
/// does.
fn java_string_equals(a: &Option<Vec<u16>>, b: &Option<Vec<u16>>) -> bool {
    match a {
        None => panic!("java.lang.NullPointerException: currency spacing pattern"),
        Some(a) => b.as_ref() == Some(a),
    }
}
