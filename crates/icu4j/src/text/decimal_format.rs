// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.DecimalFormat`.

use std::any::Any;
use std::sync::{Arc, Mutex};

use rucene::error::{LuceneError, Result};
use crate::util::u_locale::Category;

use crate::jdk::math::{BigDecimal, BigInteger, MathContext, RoundingMode};
use crate::jdk::text::{AttributedCharacterIterator, FieldPosition, Format, ParsedObject};
use crate::jdk::ParsePosition;
use crate::math::big_decimal::BigDecimal as IcuBigDecimal;
use crate::math::math_context::MathContext as IcuMathContext;
use crate::number::localized_number_formatter::LocalizedNumberFormatter;
use crate::number::number_formatter::NumberFormatter;
use crate::number::number_formatter_settings::NumberFormatterSettings;
use crate::r#impl::formatted_string_builder::FormattedStringBuilder;
use crate::r#impl::formatted_value_string_builder_impl::FormattedValueStringBuilderImpl;
use crate::r#impl::number::affix_utils::AffixUtils;
use crate::r#impl::number::decimal_format_properties::{DecimalFormatProperties, ParseMode};
use crate::r#impl::number::decimal_quantity::DecimalQuantity;
use crate::r#impl::number::decimal_quantity_dual_storage_bcd::{DecimalQuantity_DualStorageBCD, JavaNumber};
use crate::r#impl::number::padder::{PadPosition, FALLBACK_PADDING_STRING};
use crate::r#impl::number::parse::number_parser_impl::NumberParserImpl;
use crate::r#impl::number::parse::parsed_number::ParsedNumber;
use crate::r#impl::number::pattern_string_parser::PatternStringParser;
use crate::r#impl::number::pattern_string_utils::PatternStringUtils;
use crate::r#impl::number::properties::Properties;
use crate::text::compact_decimal_format::CompactStyle;
use crate::text::currency_plural_info::CurrencyPluralInfo;
use crate::text::decimal_format_symbols::DecimalFormatSymbols;
use crate::text::number_format::{
    self, lock, NumberFormat, NumberFormatBase, ACCOUNTINGCURRENCYSTYLE, CASHCURRENCYSTYLE, CURRENCYSTYLE,
    ISOCURRENCYSTYLE, NUMBERSTYLE, PLURALCURRENCYSTYLE, STANDARDCURRENCYSTYLE,
};
use crate::text::plural_rules::IFixedDecimal;
use crate::util::currency::{Currency, CurrencyUsage};
use crate::util::currency_amount::CurrencyAmount;
use crate::util::u_locale::{self, ULocale};

/// `serialVersionUID`.
pub const SERIAL_VERSION_UID: i64 = 864413376551465018;

/// The new DecimalFormat (ICU 59) serializes its state as version 5.
const SERIAL_VERSION_ON_STREAM: i32 = 5;

/// The transient, `synchronized`-guarded state of a `DecimalFormat`.
#[derive(Debug, Clone)]
struct State {
    /// The property bag corresponding to user-specified settings and
    /// settings from the pattern string.
    properties: DecimalFormatProperties,
    /// The symbols for the current locale (`volatile`).
    symbols: Arc<DecimalFormatSymbols>,
    /// The pre-computed formatter object (`volatile`).
    formatter: Option<LocalizedNumberFormatter>,
    /// The effective properties as exported from the formatter object
    /// (`volatile`; `null` until the constructor sets it).
    exported_properties: Option<DecimalFormatProperties>,
    /// `parser` (`volatile`, created on first use).
    parser: Option<Arc<NumberParserImpl>>,
    /// `currencyParser` (`volatile`, created on first use).
    currency_parser: Option<Arc<NumberParserImpl>>,
    /// `icuMathContextForm`.
    icu_math_context_form: i32,
}

/// The runtime class of a `DecimalFormat` object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DecimalFormatClass {
    /// `com.ibm.icu.text.DecimalFormat`.
    DecimalFormat,
    /// `com.ibm.icu.text.CompactDecimalFormat`, whose `parse` and
    /// `parseCurrency` throw `UnsupportedOperationException`.
    CompactDecimalFormat,
}

/// A value of a field of the pre-ICU-59 serialized form (`readObject` of a
/// stream older than version 5).
#[derive(Debug, Clone)]
pub enum LegacyFieldValue {
    /// A `boolean`.
    Boolean(bool),
    /// An `int`.
    Int(i32),
    /// A `byte`.
    Byte(i8),
    /// A `char`.
    Char(u16),
    /// A `String` (`None` for `null`).
    String(Option<Vec<u16>>),
    /// A `java.math.BigDecimal` (`None` for `null`).
    BigDecimal(Option<BigDecimal>),
    /// A `com.ibm.icu.math.MathContext`.
    MathContext(Option<IcuMathContext>),
    /// A `CurrencyPluralInfo`.
    CurrencyPluralInfo(Option<CurrencyPluralInfo>),
    /// A `DecimalFormatSymbols`.
    Symbols(Option<DecimalFormatSymbols>),
}

/// The serialized properties of a version-5 stream: a
/// `DecimalFormatProperties` or, from ICU 59, a
/// `com.ibm.icu.impl.number.Properties`.
#[derive(Debug, Clone)]
pub enum SerializedProperties {
    /// `DecimalFormatProperties`.
    DecimalFormatProperties(DecimalFormatProperties),
    /// `com.ibm.icu.impl.number.Properties`.
    Properties(Properties),
}

/// `DecimalFormat` is the primary concrete subclass of `NumberFormat`. It
/// has a variety of features designed to make it possible to parse and
/// format numbers in any locale, including support for Western, Arabic, or
/// Indic digits. It supports different flavors of numbers, including
/// integers ("123"), fixed-point numbers ("123.4"), scientific notation
/// ("1.23E4"), percentages ("12%"), and currency amounts ("$123.00",
/// "USD123.00", "123.00 US dollars"). All of these flavors can be easily
/// localized.
///
/// Equivalent to `com.ibm.icu.text.DecimalFormat` (a `NumberFormat`). The
/// `synchronized` methods lock the state; the runtime class
/// (`DecimalFormat` or its subclass `CompactDecimalFormat`) is kept in the
/// object, so that the overrides of the subclass hold through the base.
#[derive(Debug)]
pub struct DecimalFormat {
    class: DecimalFormatClass,
    base: Mutex<NumberFormatBase>,
    state: Mutex<State>,
}

impl DecimalFormat {
    /// `PAD_BEFORE_PREFIX`.
    pub const PAD_BEFORE_PREFIX: i32 = 0;
    /// `PAD_AFTER_PREFIX`.
    pub const PAD_AFTER_PREFIX: i32 = 1;
    /// `PAD_BEFORE_SUFFIX`.
    pub const PAD_BEFORE_SUFFIX: i32 = 2;
    /// `PAD_AFTER_SUFFIX`.
    pub const PAD_AFTER_SUFFIX: i32 = 3;
    /// `MINIMUM_GROUPING_DIGITS_AUTO`.
    pub const MINIMUM_GROUPING_DIGITS_AUTO: i32 = -2;
    /// `MINIMUM_GROUPING_DIGITS_MIN2`.
    pub const MINIMUM_GROUPING_DIGITS_MIN2: i32 = -3;

    fn from_state(class: DecimalFormatClass, state: State) -> Self {
        DecimalFormat { class, base: Mutex::new(NumberFormatBase::new()), state: Mutex::new(state) }
    }

    fn initial_state(symbols: DecimalFormatSymbols) -> State {
        State {
            properties: DecimalFormatProperties::new(),
            symbols: Arc::new(symbols),
            formatter: None,
            exported_properties: Some(DecimalFormatProperties::new()),
            parser: None,
            currency_parser: None,
            icu_math_context_form: IcuMathContext::PLAIN,
        }
    }

    /// Creates a DecimalFormat based on the number pattern and symbols for
    /// the default locale. Equivalent to `DecimalFormat()`.
    ///
    /// # Errors
    ///
    /// The errors of the locale data and of the pattern.
    pub fn new() -> Result<Self> {
        // Use the locale's default pattern
        let def = ULocale::get_default_category(Category::Format);
        let pattern = number_format::get_pattern(&def, NUMBERSTYLE)?;
        let df = Self::from_state(DecimalFormatClass::DecimalFormat, Self::initial_state(Self::get_default_symbols()?));
        // Note: ignoreRounding does not affect the default pattern
        df.with_state(|me, s| {
            Self::set_properties_from_pattern(s, Some(&pattern), PatternStringParser::IGNORE_ROUNDING_IF_CURRENCY)?;
            me.refresh_formatter(s)
        })?;
        Ok(df)
    }

    /// Creates a DecimalFormat based on the given pattern, using symbols for
    /// the default locale. Equivalent to `DecimalFormat(String)`.
    ///
    /// # Errors
    ///
    /// `NullPointerException` for a `None` pattern; the
    /// `IllegalArgumentException` of a malformed pattern.
    pub fn from_pattern(pattern: Option<&[u16]>) -> Result<Self> {
        let df = Self::from_state(DecimalFormatClass::DecimalFormat, Self::initial_state(Self::get_default_symbols()?));
        // Must call refreshFormatter, even if pattern is null
        df.with_state(|me, s| {
            Self::set_properties_from_pattern(s, pattern, PatternStringParser::IGNORE_ROUNDING_IF_CURRENCY)?;
            me.refresh_formatter(s)
        })?;
        Ok(df)
    }

    /// Creates a DecimalFormat based on the given pattern and symbols. Use
    /// this constructor if you want complete control over the behavior of
    /// the formatter. Equivalent to `DecimalFormat(String,
    /// DecimalFormatSymbols)`.
    ///
    /// # Errors
    ///
    /// As [`from_pattern`](Self::from_pattern).
    pub fn with_symbols(pattern: Option<&[u16]>, symbols: &DecimalFormatSymbols) -> Result<Self> {
        let df = Self::from_state(DecimalFormatClass::DecimalFormat, Self::initial_state(symbols.clone()));
        df.with_state(|me, s| {
            Self::set_properties_from_pattern(s, pattern, PatternStringParser::IGNORE_ROUNDING_IF_CURRENCY)?;
            me.refresh_formatter(s)
        })?;
        Ok(df)
    }

    /// Creates a DecimalFormat based on the given pattern and symbols, with
    /// additional control over the behavior of currency. Equivalent to
    /// `DecimalFormat(String, DecimalFormatSymbols, CurrencyPluralInfo,
    /// int)`.
    ///
    /// # Errors
    ///
    /// As [`from_pattern`](Self::from_pattern).
    pub fn with_currency_plural_info(
        pattern: Option<&[u16]>,
        symbols: &DecimalFormatSymbols,
        info_input: Option<&CurrencyPluralInfo>,
        style: i32,
    ) -> Result<Self> {
        let df = Self::with_style_opt(pattern, symbols, style)?;
        df.with_state(|me, s| {
            s.properties.set_currency_plural_info(info_input);
            me.refresh_formatter(s)
        })?;
        Ok(df)
    }

    /// Equivalent to the package-private `DecimalFormat(String,
    /// DecimalFormatSymbols, int)`.
    ///
    /// # Errors
    ///
    /// As [`from_pattern`](Self::from_pattern).
    pub(crate) fn with_style(pattern: &[u16], symbols: &DecimalFormatSymbols, choice: i32) -> Result<Self> {
        Self::with_style_opt(Some(pattern), symbols, choice)
    }

    fn with_style_opt(pattern: Option<&[u16]>, symbols: &DecimalFormatSymbols, choice: i32) -> Result<Self> {
        let df = Self::from_state(DecimalFormatClass::DecimalFormat, Self::initial_state(symbols.clone()));
        df.with_state(|me, s| {
            // If choice is a currency type, ignore the rounding information.
            if choice == CURRENCYSTYLE
                || choice == ISOCURRENCYSTYLE
                || choice == ACCOUNTINGCURRENCYSTYLE
                || choice == CASHCURRENCYSTYLE
                || choice == STANDARDCURRENCYSTYLE
                || choice == PLURALCURRENCYSTYLE
            {
                Self::set_properties_from_pattern(s, pattern, PatternStringParser::IGNORE_ROUNDING_ALWAYS)?;
            } else {
                Self::set_properties_from_pattern(s, pattern, PatternStringParser::IGNORE_ROUNDING_IF_CURRENCY)?;
            }
            me.refresh_formatter(s)
        })?;
        Ok(df)
    }

    /// The `CompactDecimalFormat(ULocale, CompactStyle)` constructor body.
    pub(crate) fn new_compact(locale: &ULocale, style: CompactStyle) -> Result<Self> {
        // NOTE: This is an incomplete implementation of CompactDecimalFormat.
        // It is not yet fully supported.
        let mut state = Self::initial_state(DecimalFormatSymbols::get_instance_ulocale(locale)?);
        state.properties.set_compact_style(Some(style));
        state.properties.set_grouping_size(-2); // do not forward grouping information
        state.properties.set_minimum_grouping_digits(2);
        let df = Self::from_state(DecimalFormatClass::CompactDecimalFormat, state);
        df.with_state(|me, s| me.refresh_formatter(s))?;
        Ok(df)
    }

    fn get_default_symbols() -> Result<DecimalFormatSymbols> {
        DecimalFormatSymbols::get_instance()
    }

    /// Runs `f` with the state locked (`synchronized`).
    fn with_state<R>(&self, f: impl FnOnce(&Self, &mut State) -> Result<R>) -> Result<R> {
        let mut s = lock(&self.state);
        f(self, &mut s)
    }

    /// Reads the state (`synchronized`).
    fn read<R>(&self, f: impl FnOnce(&State) -> R) -> R {
        f(&lock(&self.state))
    }

    /// Changes the properties and refreshes the formatter (`synchronized`
    /// setter bodies).
    fn update(&self, f: impl FnOnce(&mut DecimalFormatProperties)) -> Result<()> {
        self.with_state(|me, s| {
            f(&mut s.properties);
            me.refresh_formatter(s)
        })
    }

    /// The exported properties (set by every constructor).
    fn exported(s: &State) -> &DecimalFormatProperties {
        s.exported_properties.as_ref().unwrap_or_else(|| panic!("java.lang.NullPointerException: exportedProperties"))
    }

    /// The formatter (set by every constructor).
    fn formatter(&self) -> LocalizedNumberFormatter {
        self.read(|s| s.formatter.clone())
            .unwrap_or_else(|| panic!("java.lang.NullPointerException: formatter"))
    }

    /// Parses the given pattern string and overwrites the settings
    /// specified in the pattern string. The properties corresponding to the
    /// following setters are overwritten, either with their default values
    /// or with the value specified in the pattern string: grouping, digits,
    /// rounding, decimal separator, scientific notation, padding, affixes,
    /// and the magnitude multiplier. All other settings remain untouched.
    /// Equivalent to `applyPattern(String)`.
    ///
    /// # Errors
    ///
    /// `NullPointerException` for `None`; the `IllegalArgumentException` of
    /// a malformed pattern.
    pub fn apply_pattern(&self, pattern: Option<&[u16]>) -> Result<()> {
        self.with_state(|me, s| {
            Self::set_properties_from_pattern(s, pattern, PatternStringParser::IGNORE_ROUNDING_NEVER)?;
            // Backwards compatibility: clear out user-specified prefix and suffix,
            // as well as CurrencyPluralInfo.
            s.properties.set_positive_prefix(None);
            s.properties.set_negative_prefix(None);
            s.properties.set_positive_suffix(None);
            s.properties.set_negative_suffix(None);
            s.properties.set_currency_plural_info(None);
            me.refresh_formatter(s)
        })
    }

    /// Converts the given string from localized notation to standard
    /// notation and then applies it to this formatter. Equivalent to
    /// `applyLocalizedPattern(String)`.
    ///
    /// # Errors
    ///
    /// The errors of the conversion and of [`apply_pattern`](Self::apply_pattern).
    pub fn apply_localized_pattern(&self, localized_pattern: Option<&[u16]>) -> Result<()> {
        let symbols = self.read(|s| s.symbols.clone());
        let pattern = PatternStringUtils::convert_localized(localized_pattern, &symbols, false)?;
        self.apply_pattern(pattern.as_deref())
    }

    /// Equivalent to `clone()`.
    ///
    /// # Errors
    ///
    /// The errors of the formatter refresh.
    pub fn clone_decimal_format(&self) -> Result<DecimalFormat> {
        let base = lock(&self.base).clone();
        let s = lock(&self.state).clone();
        let mut state = s;
        state.symbols = Arc::new((*state.symbols).clone());
        state.exported_properties = Some(DecimalFormatProperties::new());
        state.formatter = None;
        state.parser = None;
        state.currency_parser = None;
        let other = DecimalFormat { class: self.class, base: Mutex::new(base), state: Mutex::new(state) };
        other.with_state(|me, s| me.refresh_formatter(s))?;
        Ok(other)
    }

    /// Equivalent to `writeObject(ObjectOutputStream)`: after
    /// `defaultWriteObject()` (the `serialVersionOnStream` field, 5) and
    /// `writeInt(0)`, the properties and the symbols.
    pub fn write_object(&self) -> (i32, DecimalFormatProperties, DecimalFormatSymbols) {
        self.read(|s| (SERIAL_VERSION_ON_STREAM, s.properties.clone(), (*s.symbols).clone()))
    }

    /// Equivalent to `readObject(ObjectInputStream)` of a version-5
    /// stream: `serialized_field_count` is the number of fields the stream
    /// describes, then the properties and the symbols follow.
    ///
    /// # Errors
    ///
    /// `IOException("Cannot deserialize newer com.ibm.icu.text.DecimalFormat
    /// (v<n>)")` for a version above 5; `IOException("Too many fields when
    /// reading serial version 5")`; the errors of the refresh.
    pub fn read_object_v5(
        &self,
        serial_version: i32,
        serialized_field_count: usize,
        serialized_properties: SerializedProperties,
        symbols: DecimalFormatSymbols,
    ) -> Result<()> {
        if serial_version > 5 {
            return Err(LuceneError::unchecked(
                "java.io.IOException",
                &format!("Cannot deserialize newer com.ibm.icu.text.DecimalFormat (v{serial_version})"),
            ));
        }
        if serialized_field_count > 1 {
            return Err(LuceneError::unchecked("java.io.IOException", "Too many fields when reading serial version 5"));
        }
        self.with_state(|me, s| {
            s.properties = match serialized_properties {
                SerializedProperties::DecimalFormatProperties(p) => p,
                SerializedProperties::Properties(p) => {
                    p.get_instance().cloned().unwrap_or_else(|| panic!("java.lang.NullPointerException"))
                }
            };
            s.symbols = Arc::new(symbols);
            s.exported_properties = Some(DecimalFormatProperties::new());
            me.refresh_formatter(s)
        })
    }

    /// Equivalent to `readObject(ObjectInputStream)` of a stream older than
    /// version 5 (the pre-ICU-59 serialized form): each `(name, value)`
    /// field of the stream, in stream order, is applied through its setter;
    /// the `NumberFormat` fields are those already read into the base.
    ///
    /// # Errors
    ///
    /// The errors of the setters, `IOException` wrapping their
    /// `IllegalArgumentException`s for the `NumberFormat` fields.
    pub fn read_object_legacy(&self, fields: Vec<(String, LegacyFieldValue)>) -> Result<()> {
        use LegacyFieldValue as F;
        lock(&self.state).properties = DecimalFormatProperties::new();
        let (mut pp, mut ppp, mut ps, mut psp) = (None, None, None, None);
        let (mut np, mut npp, mut ns, mut nsp) = (None, None, None, None);
        for (name, value) in fields {
            match (name.as_str(), value) {
                ("decimalSeparatorAlwaysShown", F::Boolean(v)) => self.set_decimal_separator_always_shown(v)?,
                ("exponentSignAlwaysShown", F::Boolean(v)) => self.set_exponent_sign_always_shown(v)?,
                ("formatWidth", F::Int(v)) => self.set_format_width(v)?,
                ("groupingSize", F::Byte(v)) => self.set_grouping_size(i32::from(v))?,
                ("groupingSize2", F::Byte(v)) => self.set_secondary_grouping_size(i32::from(v))?,
                ("maxSignificantDigits", F::Int(v)) => self.set_maximum_significant_digits(v)?,
                ("minExponentDigits", F::Byte(v)) => self.set_minimum_exponent_digits(v)?,
                ("minSignificantDigits", F::Int(v)) => self.set_minimum_significant_digits(v)?,
                ("multiplier", F::Int(v)) => self.set_multiplier(v)?,
                ("pad", F::Char(v)) => self.set_pad_character(v)?,
                ("padPosition", F::Int(v)) => self.set_pad_position(v)?,
                ("parseBigDecimal", F::Boolean(v)) => self.set_parse_big_decimal(v)?,
                ("parseRequireDecimalPoint", F::Boolean(v)) => self.set_decimal_pattern_match_required(v)?,
                ("roundingMode", F::Int(v)) => self.set_rounding_mode(v)?,
                ("useExponentialNotation", F::Boolean(v)) => self.set_scientific_notation(v)?,
                ("useSignificantDigits", F::Boolean(v)) => self.set_significant_digits_used(v)?,
                ("currencyPluralInfo", F::CurrencyPluralInfo(v)) => self.set_currency_plural_info(v.as_ref())?,
                ("mathContext", F::MathContext(v)) => {
                    let v = v.ok_or_else(|| crate::jdk::null_pointer("mathContext"))?;
                    self.set_math_context_icu(&v)?;
                }
                ("negPrefixPattern", F::String(v)) => npp = v,
                ("negSuffixPattern", F::String(v)) => nsp = v,
                ("negativePrefix", F::String(v)) => np = v,
                ("negativeSuffix", F::String(v)) => ns = v,
                ("posPrefixPattern", F::String(v)) => ppp = v,
                ("posSuffixPattern", F::String(v)) => psp = v,
                ("positivePrefix", F::String(v)) => pp = v,
                ("positiveSuffix", F::String(v)) => ps = v,
                ("roundingIncrement", F::BigDecimal(v)) => self.set_rounding_increment(v)?,
                ("symbols", F::Symbols(v)) => {
                    let v = v.ok_or_else(|| crate::jdk::null_pointer("symbols"))?;
                    self.set_decimal_format_symbols(&v)?;
                }
                (
                    "decimalSeparatorAlwaysShown" | "exponentSignAlwaysShown" | "formatWidth" | "groupingSize"
                    | "groupingSize2" | "maxSignificantDigits" | "minExponentDigits" | "minSignificantDigits"
                    | "multiplier" | "pad" | "padPosition" | "parseBigDecimal" | "parseRequireDecimalPoint"
                    | "roundingMode" | "useExponentialNotation" | "useSignificantDigits" | "currencyPluralInfo"
                    | "mathContext" | "negPrefixPattern" | "negSuffixPattern" | "negativePrefix" | "negativeSuffix"
                    | "posPrefixPattern" | "posSuffixPattern" | "positivePrefix" | "positiveSuffix"
                    | "roundingIncrement" | "symbols",
                    _,
                ) => {
                    // GetField.get with a type the stream field does not have.
                    return Err(LuceneError::illegal_argument(format!("invalid type for {name}")));
                }
                _ => {
                    // The following fields are ignored:
                    // "PARSE_MAX_EXPONENT"
                    // "currencySignCount"
                    // "style"
                    // "attributes"
                    // "currencyChoice"
                    // "formatPattern"
                    // "currencyUsage" => ignore this because the old code puts currencyUsage directly into properties
                }
            }
        }
        // Resolve affixes
        self.with_state(|_, s| {
            match npp {
                None => s.properties.set_negative_prefix(np.as_deref()),
                Some(v) => s.properties.set_negative_prefix_pattern(Some(&v)),
            };
            match nsp {
                None => s.properties.set_negative_suffix(ns.as_deref()),
                Some(v) => s.properties.set_negative_suffix_pattern(Some(&v)),
            };
            match ppp {
                None => s.properties.set_positive_prefix(pp.as_deref()),
                Some(v) => s.properties.set_positive_prefix_pattern(Some(&v)),
            };
            match psp {
                None => s.properties.set_positive_suffix(ps.as_deref()),
                Some(v) => s.properties.set_positive_suffix_pattern(Some(&v)),
            };
            Ok(())
        })?;
        // Extract values from parent NumberFormat class.  Have to use reflection here.
        let base = lock(&self.base).clone();
        let io = |e: LuceneError| LuceneError::unchecked("java.io.IOException", &e.to_string());
        self.set_grouping_used(base.grouping_used_field()).map_err(io)?;
        self.set_parse_integer_only(base.parse_integer_only_field()).map_err(io)?;
        self.set_maximum_integer_digits(base.maximum_integer_digits_field()).map_err(io)?;
        self.set_minimum_integer_digits(base.minimum_integer_digits_field()).map_err(io)?;
        self.set_maximum_fraction_digits(base.maximum_fraction_digits_field()).map_err(io)?;
        self.set_minimum_fraction_digits(base.minimum_fraction_digits_field()).map_err(io)?;
        self.set_currency(base.currency_field()).map_err(io)?;
        self.set_parse_strict(base.parse_strict_field()).map_err(io)?;
        // Finish initialization
        self.with_state(|me, s| {
            s.exported_properties = Some(DecimalFormatProperties::new());
            me.refresh_formatter(s)
        })
    }

    /// `format(<number>, StringBuffer, FieldPosition)` of a quantity.
    fn format_quantity(&self, mut dq: DecimalQuantity_DualStorageBCD, result: &mut Vec<u16>, field_position: &mut FieldPosition) -> Result<()> {
        let mut string = FormattedStringBuilder::new();
        self.formatter().format_impl(&mut dq, &mut string)?;
        Self::field_position_helper(&dq, &string, field_position, result.len() as i32)?;
        result.extend_from_slice(&string.to_java_string());
        Ok(())
    }

    /// Returns a copy of the decimal format symbols used by this formatter.
    /// Equivalent to `getDecimalFormatSymbols()`.
    pub fn get_decimal_format_symbols(&self) -> DecimalFormatSymbols {
        self.read(|s| (*s.symbols).clone())
    }

    /// Sets the decimal format symbols used by this formatter. The formatter
    /// uses a copy of the provided symbols. Equivalent to
    /// `setDecimalFormatSymbols(DecimalFormatSymbols)`.
    ///
    /// # Errors
    ///
    /// The errors of the refresh.
    pub fn set_decimal_format_symbols(&self, new_symbols: &DecimalFormatSymbols) -> Result<()> {
        self.with_state(|me, s| {
            s.symbols = Arc::new(new_symbols.clone());
            me.refresh_formatter(s)
        })
    }

    /// Affixes: Gets the positive prefix string currently being used to
    /// format numbers. Equivalent to `getPositivePrefix()`.
    ///
    /// # Errors
    ///
    /// The errors of the formatter.
    pub fn get_positive_prefix(&self) -> Result<Vec<u16>> {
        self.formatter().get_affix_impl(true, false)
    }

    /// Affixes: Sets the string to prepend to positive numbers. Equivalent
    /// to `setPositivePrefix(String)`.
    ///
    /// # Errors
    ///
    /// `NullPointerException` for `None`; the errors of the refresh.
    pub fn set_positive_prefix(&self, prefix: Option<&[u16]>) -> Result<()> {
        let prefix = prefix.ok_or_else(|| LuceneError::null_pointer())?;
        self.update(|p| {
            p.set_positive_prefix(Some(prefix));
        })
    }

    /// Affixes: Gets the negative prefix string currently being used to
    /// format numbers. Equivalent to `getNegativePrefix()`.
    ///
    /// # Errors
    ///
    /// The errors of the formatter.
    pub fn get_negative_prefix(&self) -> Result<Vec<u16>> {
        self.formatter().get_affix_impl(true, true)
    }

    /// Affixes: Sets the string to prepend to negative numbers. Equivalent
    /// to `setNegativePrefix(String)`.
    ///
    /// # Errors
    ///
    /// `NullPointerException` for `None`; the errors of the refresh.
    pub fn set_negative_prefix(&self, prefix: Option<&[u16]>) -> Result<()> {
        let prefix = prefix.ok_or_else(|| LuceneError::null_pointer())?;
        self.update(|p| {
            p.set_negative_prefix(Some(prefix));
        })
    }

    /// Affixes: Gets the positive suffix string currently being used to
    /// format numbers. Equivalent to `getPositiveSuffix()`.
    ///
    /// # Errors
    ///
    /// The errors of the formatter.
    pub fn get_positive_suffix(&self) -> Result<Vec<u16>> {
        self.formatter().get_affix_impl(false, false)
    }

    /// Affixes: Sets the string to append to positive numbers. Equivalent to
    /// `setPositiveSuffix(String)`.
    ///
    /// # Errors
    ///
    /// `NullPointerException` for `None`; the errors of the refresh.
    pub fn set_positive_suffix(&self, suffix: Option<&[u16]>) -> Result<()> {
        let suffix = suffix.ok_or_else(|| LuceneError::null_pointer())?;
        self.update(|p| {
            p.set_positive_suffix(Some(suffix));
        })
    }

    /// Affixes: Gets the negative suffix string currently being used to
    /// format numbers. Equivalent to `getNegativeSuffix()`.
    ///
    /// # Errors
    ///
    /// The errors of the formatter.
    pub fn get_negative_suffix(&self) -> Result<Vec<u16>> {
        self.formatter().get_affix_impl(false, true)
    }

    /// Affixes: Sets the string to append to negative numbers. Equivalent to
    /// `setNegativeSuffix(String)`.
    ///
    /// # Errors
    ///
    /// `NullPointerException` for `None`; the errors of the refresh.
    pub fn set_negative_suffix(&self, suffix: Option<&[u16]>) -> Result<()> {
        let suffix = suffix.ok_or_else(|| LuceneError::null_pointer())?;
        self.update(|p| {
            p.set_negative_suffix(Some(suffix));
        })
    }

    /// Returns whether the sign is being shown on positive numbers.
    /// Equivalent to `isSignAlwaysShown()`.
    pub fn is_sign_always_shown(&self) -> bool {
        self.read(|s| s.properties.get_sign_always_shown())
    }

    /// Sets whether to always shown the plus sign ('+' in *en*) on positive
    /// numbers. Equivalent to `setSignAlwaysShown(boolean)`.
    ///
    /// # Errors
    ///
    /// The errors of the refresh.
    pub fn set_sign_always_shown(&self, value: bool) -> Result<()> {
        self.update(|p| {
            p.set_sign_always_shown(value);
        })
    }

    /// Returns the multiplier being applied to numbers before they are
    /// formatted. Equivalent to `getMultiplier()`.
    pub fn get_multiplier(&self) -> i32 {
        self.read(|s| match s.properties.get_multiplier() {
            Some(m) => m.int_value(),
            None => 10f64.powi(s.properties.get_magnitude_multiplier()) as i32,
        })
    }

    /// Sets a number that will be used to multiply all numbers prior to
    /// formatting. Equivalent to `setMultiplier(int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Multiplier must be nonzero.")`; the errors
    /// of the refresh.
    pub fn set_multiplier(&self, multiplier: i32) -> Result<()> {
        if multiplier == 0 {
            return Err(LuceneError::illegal_argument("Multiplier must be nonzero."));
        }

        // Try to convert to a magnitude multiplier first
        let mut delta = 0;
        let mut value = multiplier;
        while value != 1 {
            delta += 1;
            let temp = value / 10;
            if temp * 10 != value {
                delta = -1;
                break;
            }
            value = temp;
        }
        self.update(|p| {
            if delta != -1 {
                p.set_magnitude_multiplier(delta);
                p.set_multiplier(None);
            } else {
                p.set_magnitude_multiplier(0);
                p.set_multiplier(Some(BigDecimal::value_of(i64::from(multiplier))));
            }
        })
    }

    /// Returns the increment to which numbers are being rounded. Equivalent
    /// to `getRoundingIncrement()`.
    pub fn get_rounding_increment(&self) -> Option<BigDecimal> {
        self.read(|s| Self::exported(s).get_rounding_increment())
    }

    /// Rounding and Digit Limits: Sets an increment, or interval, to which
    /// numbers are rounded. Equivalent to
    /// `setRoundingIncrement(java.math.BigDecimal)`.
    ///
    /// # Errors
    ///
    /// The errors of the refresh.
    pub fn set_rounding_increment(&self, increment: Option<BigDecimal>) -> Result<()> {
        // Backwards compatibility: ignore rounding increment if zero,
        // and instead set maximum fraction digits.
        if let Some(inc) = increment.as_ref() {
            if inc.compare_to(&BigDecimal::zero()) == 0 {
                lock(&self.state).properties.set_maximum_fraction_digits(i32::MAX);
                return Ok(());
            }
        }

        self.update(|p| {
            p.set_rounding_increment(increment);
        })
    }

    /// Equivalent to `setRoundingIncrement(com.ibm.icu.math.BigDecimal)`.
    ///
    /// # Errors
    ///
    /// The errors of the refresh.
    pub fn set_rounding_increment_icu(&self, increment: Option<&IcuBigDecimal>) -> Result<()> {
        let java_big_decimal = increment.map(IcuBigDecimal::to_big_decimal);
        self.set_rounding_increment(java_big_decimal)
    }

    /// Equivalent to `setRoundingIncrement(double)`.
    ///
    /// # Errors
    ///
    /// `NumberFormatException` for an infinite or NaN increment; the errors
    /// of the refresh.
    pub fn set_rounding_increment_double(&self, increment: f64) -> Result<()> {
        if increment == 0.0 {
            self.set_rounding_increment(None)
        } else {
            // ICU-20425: Since doubles have no concept of trailing zeros, we should strip
            // trailing zeros from the BigDecimal.
            let java_big_decimal = BigDecimal::value_of_double(increment)?.strip_trailing_zeros()?;
            self.set_rounding_increment(Some(java_big_decimal))
        }
    }

    /// Returns the java.math.MathContext being used to round numbers.
    /// Equivalent to `getMathContext()`.
    pub fn get_math_context(&self) -> MathContext {
        self.read(|s| {
            let math_context = Self::exported(s).get_math_context().copied();
            debug_assert!(math_context.is_some());
            math_context.unwrap_or_else(|| panic!("java.lang.NullPointerException: mathContext"))
        })
    }

    /// Rounding and Digit Limits: Overrides the rounding mode and precision.
    /// Equivalent to `setMathContext(java.math.MathContext)`.
    ///
    /// # Errors
    ///
    /// The errors of the refresh.
    pub fn set_math_context(&self, math_context: Option<MathContext>) -> Result<()> {
        self.update(|p| {
            p.set_math_context(math_context);
        })
    }

    /// Returns the com.ibm.icu.math.MathContext being used to round
    /// numbers. Equivalent to `getMathContextICU()`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of the ICU MathContext constructor.
    pub fn get_math_context_icu(&self) -> Result<IcuMathContext> {
        let math_context = self.get_math_context();
        let form = self.read(|s| s.icu_math_context_form);
        IcuMathContext::with_all(math_context.get_precision(), form, false, math_context.get_rounding_mode().ordinal())
    }

    /// Overrides the rounding mode and precision. Equivalent to
    /// `setMathContextICU(com.ibm.icu.math.MathContext)`.
    ///
    /// # Errors
    ///
    /// The errors of the conversion and of the refresh.
    pub fn set_math_context_icu(&self, math_context_icu: &IcuMathContext) -> Result<()> {
        lock(&self.state).icu_math_context_form = math_context_icu.get_form();
        let math_context = if math_context_icu.get_lost_digits() {
            // The getLostDigits() feature in ICU MathContext means "throw an ArithmeticException if
            // rounding causes digits to be lost". That feature is called RoundingMode.UNNECESSARY in
            // Java MathContext.
            MathContext::with_rounding_mode(math_context_icu.get_digits(), RoundingMode::Unnecessary)?
        } else {
            MathContext::with_rounding_mode(
                math_context_icu.get_digits(),
                RoundingMode::value_of(math_context_icu.get_rounding_mode())?,
            )?
        };
        self.set_math_context(Some(math_context))
    }

    /// Returns whether significant digits are being used in rounding.
    /// Equivalent to `areSignificantDigitsUsed()`.
    pub fn are_significant_digits_used(&self) -> bool {
        self.read(|s| {
            s.properties.get_minimum_significant_digits() != -1 || s.properties.get_maximum_significant_digits() != -1
        })
    }

    /// Rounding and Digit Limits: Sets whether significant digits are to be
    /// used in rounding. Equivalent to `setSignificantDigitsUsed(boolean)`.
    ///
    /// # Errors
    ///
    /// The errors of the refresh.
    pub fn set_significant_digits_used(&self, use_significant_digits: bool) -> Result<()> {
        self.with_state(|me, s| {
            let old_min_sig = s.properties.get_minimum_significant_digits();
            let old_max_sig = s.properties.get_maximum_significant_digits();
            // These are the default values from the old implementation.
            if use_significant_digits {
                if old_min_sig != -1 || old_max_sig != -1 {
                    return Ok(());
                }
            } else if old_min_sig == -1 && old_max_sig == -1 {
                return Ok(());
            }
            let min_sig = if use_significant_digits { 1 } else { -1 };
            let max_sig = if use_significant_digits { 6 } else { -1 };
            s.properties.set_minimum_significant_digits(min_sig);
            s.properties.set_maximum_significant_digits(max_sig);
            me.refresh_formatter(s)
        })
    }

    /// Returns the effective minimum number of significant digits displayed.
    /// Equivalent to `getMinimumSignificantDigits()`.
    pub fn get_minimum_significant_digits(&self) -> i32 {
        self.read(|s| Self::exported(s).get_minimum_significant_digits())
    }

    /// Rounding and Digit Limits: Sets the minimum number of significant
    /// digits to be displayed. Equivalent to
    /// `setMinimumSignificantDigits(int)`.
    ///
    /// # Errors
    ///
    /// The errors of the refresh.
    pub fn set_minimum_significant_digits(&self, value: i32) -> Result<()> {
        self.update(|p| {
            let max = p.get_maximum_significant_digits();
            if max >= 0 && max < value {
                p.set_maximum_significant_digits(value);
            }
            p.set_minimum_significant_digits(value);
        })
    }

    /// Returns the effective maximum number of significant digits displayed.
    /// Equivalent to `getMaximumSignificantDigits()`.
    pub fn get_maximum_significant_digits(&self) -> i32 {
        self.read(|s| Self::exported(s).get_maximum_significant_digits())
    }

    /// Rounding and Digit Limits: Sets the maximum number of significant
    /// digits to be displayed. Equivalent to
    /// `setMaximumSignificantDigits(int)`.
    ///
    /// # Errors
    ///
    /// The errors of the refresh.
    pub fn set_maximum_significant_digits(&self, value: i32) -> Result<()> {
        self.update(|p| {
            let min = p.get_minimum_significant_digits();
            if min >= 0 && min > value {
                p.set_minimum_significant_digits(value);
            }
            p.set_maximum_significant_digits(value);
        })
    }

    /// Returns the minimum number of characters in formatted output.
    /// Equivalent to `getFormatWidth()`.
    pub fn get_format_width(&self) -> i32 {
        self.read(|s| s.properties.get_format_width())
    }

    /// Padding: Sets the minimum width of the string output by the
    /// formatting pipeline. Equivalent to `setFormatWidth(int)`.
    ///
    /// # Errors
    ///
    /// The errors of the refresh.
    pub fn set_format_width(&self, width: i32) -> Result<()> {
        self.update(|p| {
            p.set_format_width(width);
        })
    }

    /// Returns the character used for padding. Equivalent to
    /// `getPadCharacter()`.
    pub fn get_pad_character(&self) -> u16 {
        self.read(|s| match s.properties.get_pad_string() {
            None => crate::jdk::units(FALLBACK_PADDING_STRING)[0],
            Some(p) => p[0],
        })
    }

    /// Padding: Sets the character used to pad numbers that are narrower
    /// than the width specified in setFormatWidth. Equivalent to
    /// `setPadCharacter(char)`.
    ///
    /// # Errors
    ///
    /// The errors of the refresh.
    pub fn set_pad_character(&self, pad_char: u16) -> Result<()> {
        self.update(|p| {
            p.set_pad_string(Some(&[pad_char]));
        })
    }

    /// Returns the position used for padding. Equivalent to
    /// `getPadPosition()`.
    pub fn get_pad_position(&self) -> i32 {
        self.read(|s| match s.properties.get_pad_position() {
            None => Self::PAD_BEFORE_PREFIX,
            Some(loc) => loc.to_old(),
        })
    }

    /// Padding: Sets the position where to insert the pad character when
    /// narrower than the width specified in setFormatWidth. Equivalent to
    /// `setPadPosition(int)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of `PadPosition.fromOld`; the errors of
    /// the refresh.
    pub fn set_pad_position(&self, pad_pos: i32) -> Result<()> {
        let pos = PadPosition::from_old(pad_pos)?;
        self.update(|p| {
            p.set_pad_position(Some(pos));
        })
    }

    /// Returns whether scientific (exponential) notation is enabled on this
    /// formatter. Equivalent to `isScientificNotation()`.
    pub fn is_scientific_notation(&self) -> bool {
        self.read(|s| s.properties.get_minimum_exponent_digits() != -1)
    }

    /// Scientific Notation: Sets whether this formatter should print in
    /// scientific (exponential) notation. Equivalent to
    /// `setScientificNotation(boolean)`.
    ///
    /// # Errors
    ///
    /// The errors of the refresh.
    pub fn set_scientific_notation(&self, use_scientific: bool) -> Result<()> {
        self.update(|p| {
            if use_scientific {
                p.set_minimum_exponent_digits(1);
            } else {
                p.set_minimum_exponent_digits(-1);
            }
        })
    }

    /// Returns the minimum number of digits printed in the exponent in
    /// scientific notation. Equivalent to `getMinimumExponentDigits()`.
    pub fn get_minimum_exponent_digits(&self) -> i8 {
        self.read(|s| s.properties.get_minimum_exponent_digits() as i8)
    }

    /// Scientific Notation: Sets the minimum number of digits to be printed
    /// in the exponent. Equivalent to `setMinimumExponentDigits(byte)`.
    ///
    /// # Errors
    ///
    /// The errors of the refresh.
    pub fn set_minimum_exponent_digits(&self, min_exp_dig: i8) -> Result<()> {
        self.update(|p| {
            p.set_minimum_exponent_digits(i32::from(min_exp_dig));
        })
    }

    /// Returns whether the sign (plus or minus) is always printed in
    /// scientific notation. Equivalent to `isExponentSignAlwaysShown()`.
    pub fn is_exponent_sign_always_shown(&self) -> bool {
        self.read(|s| s.properties.get_exponent_sign_always_shown())
    }

    /// Scientific Notation: Sets whether the sign (plus or minus) is always
    /// to be shown in the exponent in scientific notation. Equivalent to
    /// `setExponentSignAlwaysShown(boolean)`.
    ///
    /// # Errors
    ///
    /// The errors of the refresh.
    pub fn set_exponent_sign_always_shown(&self, exp_sign_always: bool) -> Result<()> {
        self.update(|p| {
            p.set_exponent_sign_always_shown(exp_sign_always);
        })
    }

    /// Returns the primary grouping size in use. Equivalent to
    /// `getGroupingSize()`.
    pub fn get_grouping_size(&self) -> i32 {
        self.read(|s| {
            if s.properties.get_grouping_size() < 0 {
                return 0;
            }
            s.properties.get_grouping_size()
        })
    }

    /// Grouping: Sets the primary grouping size (distance between grouping
    /// separators) used when formatting large numbers. Equivalent to
    /// `setGroupingSize(int)`.
    ///
    /// # Errors
    ///
    /// The errors of the refresh.
    pub fn set_grouping_size(&self, width: i32) -> Result<()> {
        self.update(|p| {
            p.set_grouping_size(width);
        })
    }

    /// Returns the secondary grouping size in use. Equivalent to
    /// `getSecondaryGroupingSize()`.
    pub fn get_secondary_grouping_size(&self) -> i32 {
        self.read(|s| {
            let grouping2 = s.properties.get_secondary_grouping_size();
            if grouping2 < 0 {
                return 0;
            }
            grouping2
        })
    }

    /// Grouping: Sets the secondary grouping size (distance between grouping
    /// separators after the first separator) used when formatting large
    /// numbers. Equivalent to `setSecondaryGroupingSize(int)`.
    ///
    /// # Errors
    ///
    /// The errors of the refresh.
    pub fn set_secondary_grouping_size(&self, width: i32) -> Result<()> {
        self.update(|p| {
            p.set_secondary_grouping_size(width);
        })
    }

    /// Returns the minimum number of digits before grouping is triggered.
    /// Equivalent to `getMinimumGroupingDigits()`.
    pub fn get_minimum_grouping_digits(&self) -> i32 {
        self.read(|s| {
            if s.properties.get_minimum_grouping_digits() > 0 {
                return s.properties.get_minimum_grouping_digits();
            }
            1
        })
    }

    /// Sets the minimum number of digits that must be before the first
    /// grouping separator in order for the grouping separator to be
    /// printed. Equivalent to `setMinimumGroupingDigits(int)`.
    ///
    /// # Errors
    ///
    /// The errors of the refresh.
    pub fn set_minimum_grouping_digits(&self, number: i32) -> Result<()> {
        self.update(|p| {
            p.set_minimum_grouping_digits(number);
        })
    }

    /// Returns whether the decimal separator is shown on integers.
    /// Equivalent to `isDecimalSeparatorAlwaysShown()`.
    pub fn is_decimal_separator_always_shown(&self) -> bool {
        self.read(|s| s.properties.get_decimal_separator_always_shown())
    }

    /// Separators: Sets whether the decimal separator is always printed.
    /// Equivalent to `setDecimalSeparatorAlwaysShown(boolean)`.
    ///
    /// # Errors
    ///
    /// The errors of the refresh.
    pub fn set_decimal_separator_always_shown(&self, value: bool) -> Result<()> {
        self.update(|p| {
            p.set_decimal_separator_always_shown(value);
        })
    }

    /// Returns the strategy for rounding currency amounts. Equivalent to
    /// `getCurrencyUsage()`.
    pub fn get_currency_usage(&self) -> CurrencyUsage {
        // CurrencyUsage is not exported, so we have to get it from the input property bag.
        // TODO: Should we export CurrencyUsage instead?
        self.read(|s| s.properties.get_currency_usage().unwrap_or(CurrencyUsage::Standard))
    }

    /// Currency Mode: Sets the strategy for rounding currency amounts.
    /// Equivalent to `setCurrencyUsage(CurrencyUsage)`.
    ///
    /// # Errors
    ///
    /// The errors of the refresh.
    pub fn set_currency_usage(&self, usage: Option<CurrencyUsage>) -> Result<()> {
        self.update(|p| {
            p.set_currency_usage(usage);
        })
    }

    /// Returns the current instance of CurrencyPluralInfo. Equivalent to
    /// `getCurrencyPluralInfo()`.
    pub fn get_currency_plural_info(&self) -> Option<Arc<CurrencyPluralInfo>> {
        // CurrencyPluralInfo also is not exported.
        self.read(|s| s.properties.get_currency_plural_info())
    }

    /// Currency Mode: Sets a custom instance of CurrencyPluralInfo.
    /// Equivalent to `setCurrencyPluralInfo(CurrencyPluralInfo)`.
    ///
    /// # Errors
    ///
    /// The errors of the refresh.
    pub fn set_currency_plural_info(&self, new_info: Option<&CurrencyPluralInfo>) -> Result<()> {
        self.update(|p| {
            p.set_currency_plural_info(new_info);
        })
    }

    /// Returns whether {@link #parse} will always return a BigDecimal.
    /// Equivalent to `isParseBigDecimal()`.
    pub fn is_parse_big_decimal(&self) -> bool {
        self.read(|s| s.properties.get_parse_to_big_decimal())
    }

    /// Whether to make {@link #parse} prefer returning a BigDecimal when
    /// possible. Equivalent to `setParseBigDecimal(boolean)`.
    ///
    /// # Errors
    ///
    /// The errors of the refresh.
    pub fn set_parse_big_decimal(&self, value: bool) -> Result<()> {
        self.update(|p| {
            p.set_parse_to_big_decimal(value);
        })
    }

    /// Always returns 1000, the old default value. Equivalent to the
    /// deprecated `getParseMaxDigits()`.
    pub fn get_parse_max_digits(&self) -> i32 {
        1000
    }

    /// Does nothing. Equivalent to the deprecated `setParseMaxDigits(int)`.
    pub fn set_parse_max_digits(&self, _max_digits: i32) {}

    /// Android libcore uses this internal method to set
    /// ParseMode.JAVA_COMPATIBILITY. Equivalent to the internal
    /// `setParseStrictMode(ParseMode)`.
    ///
    /// # Errors
    ///
    /// The errors of the refresh.
    pub fn set_parse_strict_mode(&self, parse_mode: Option<ParseMode>) -> Result<()> {
        self.update(|p| {
            p.set_parse_mode(parse_mode);
        })
    }

    /// Returns whether the presence of a decimal point must match the
    /// pattern. Equivalent to `isDecimalPatternMatchRequired()`.
    pub fn is_decimal_pattern_match_required(&self) -> bool {
        self.read(|s| s.properties.get_decimal_pattern_match_required())
    }

    /// Parsing: This method is used to either require or forbid the
    /// presence of a decimal point in the string being parsed. Equivalent to
    /// `setDecimalPatternMatchRequired(boolean)`.
    ///
    /// # Errors
    ///
    /// The errors of the refresh.
    pub fn set_decimal_pattern_match_required(&self, value: bool) -> Result<()> {
        self.update(|p| {
            p.set_decimal_pattern_match_required(value);
        })
    }

    /// Returns whether to ignore exponents when parsing. Equivalent to
    /// `isParseNoExponent()`.
    pub fn is_parse_no_exponent(&self) -> bool {
        self.read(|s| s.properties.get_parse_no_exponent())
    }

    /// Specifies whether to stop parsing when an exponent separator is
    /// encountered. Equivalent to `setParseNoExponent(boolean)`.
    ///
    /// # Errors
    ///
    /// The errors of the refresh.
    pub fn set_parse_no_exponent(&self, value: bool) -> Result<()> {
        self.update(|p| {
            p.set_parse_no_exponent(value);
        })
    }

    /// Returns whether to force case (uppercase/lowercase) to match when
    /// parsing. Equivalent to `isParseCaseSensitive()`.
    pub fn is_parse_case_sensitive(&self) -> bool {
        self.read(|s| s.properties.get_parse_case_sensitive())
    }

    /// Specifies whether parsing should require cases to match in affixes,
    /// exponent separators, and currency codes. Equivalent to
    /// `setParseCaseSensitive(boolean)`.
    ///
    /// # Errors
    ///
    /// The errors of the refresh.
    pub fn set_parse_case_sensitive(&self, value: bool) -> Result<()> {
        self.update(|p| {
            p.set_parse_case_sensitive(value);
        })
    }

    /// Returns the Java class name of this object.
    fn class_name(&self) -> &'static str {
        match self.class {
            DecimalFormatClass::DecimalFormat => "com.ibm.icu.text.DecimalFormat",
            DecimalFormatClass::CompactDecimalFormat => "com.ibm.icu.text.CompactDecimalFormat",
        }
    }

    /// Equivalent to `equals(Object)`: another DecimalFormat (or subclass)
    /// with equal properties and symbols.
    pub fn equals_decimal_format(&self, other: &DecimalFormat) -> bool {
        if std::ptr::eq(self, other) {
            return true;
        }
        let me = lock(&self.state).clone();
        let them = lock(&other.state).clone();
        me.properties == them.properties && me.symbols.java_equals(&them.symbols)
    }

    /// Returns the default value of toString() with extra DecimalFormat-
    /// specific information appended to the end of the string. Equivalent
    /// to `toString()`.
    pub fn to_java_string(&self) -> String {
        let mut result = String::new();
        result.push_str(self.class_name());
        result.push('@');
        result.push_str(&format!("{:x}", NumberFormat::hash_code(self) as u32));
        result.push_str(" { symbols@");
        result.push_str(&format!("{:x}", self.read(|s| s.symbols.hash_code()) as u32));
        self.read(|s| s.properties.to_string_bare(&mut result));
        result.push_str(" }");
        result
    }

    /// Serializes this formatter object to a decimal format pattern string.
    /// Equivalent to `toPattern()`.
    ///
    /// # Errors
    ///
    /// The errors of the pattern serialization.
    pub fn to_pattern(&self) -> Result<Vec<u16>> {
        self.with_state(|_, s| {
            // Pull some properties from exportedProperties and others from properties
            // to keep affix patterns intact.  In particular, pull rounding properties
            // so that CurrencyUsage is reflected properly.
            // TODO: Consider putting this logic in PatternString.java instead.
            let mut tprops = DecimalFormatProperties::new();
            tprops.copy_from(&s.properties);
            let use_currency = tprops.get_currency().is_some()
                || tprops.get_currency_plural_info().is_some()
                || tprops.get_currency_usage().is_some()
                || tprops.get_currency_as_decimal()
                || AffixUtils::has_currency_symbols(tprops.get_positive_prefix_pattern())?
                || AffixUtils::has_currency_symbols(tprops.get_positive_suffix_pattern())?
                || AffixUtils::has_currency_symbols(tprops.get_negative_prefix_pattern())?
                || AffixUtils::has_currency_symbols(tprops.get_negative_suffix_pattern())?;
            if use_currency {
                let exported = Self::exported(s);
                tprops.set_minimum_fraction_digits(exported.get_minimum_fraction_digits());
                tprops.set_maximum_fraction_digits(exported.get_maximum_fraction_digits());
                tprops.set_rounding_increment(exported.get_rounding_increment());
            }
            PatternStringUtils::properties_to_pattern_string(&tprops)
        })
    }

    /// Calls {@link #toPattern} and converts the string to localized
    /// notation. Equivalent to `toLocalizedPattern()`.
    ///
    /// # Errors
    ///
    /// The errors of the pattern serialization and conversion.
    pub fn to_localized_pattern(&self) -> Result<Vec<u16>> {
        let pattern = self.to_pattern()?;
        let symbols = self.read(|s| s.symbols.clone());
        Ok(PatternStringUtils::convert_localized(Some(&pattern), &symbols, true)?.unwrap_or_default())
    }

    /// Converts this DecimalFormat to a NumberFormatter. Starting in ICU 60,
    /// NumberFormatter is the recommended way to format numbers. Equivalent
    /// to `toNumberFormatter()`.
    pub fn to_number_formatter(&self) -> LocalizedNumberFormatter {
        self.formatter()
    }

    /// Equivalent to the deprecated `getFixedDecimal(double)`.
    ///
    /// # Errors
    ///
    /// The errors of the formatting.
    pub fn get_fixed_decimal(&self, number: f64) -> Result<Box<dyn IFixedDecimal>> {
        let formatted = self.formatter().format_double(number)?;
        Ok(formatted.into_fixed_decimal())
    }

    /// Rebuilds the formatter object from the property bag. Equivalent to
    /// the package-private `refreshFormatter()`.
    fn refresh_formatter(&self, s: &mut State) -> Result<()> {
        let Some(exported) = s.exported_properties.as_mut() else {
            // exportedProperties is null only when the formatter is not ready yet.
            // The only time when this happens is during legacy deserialization.
            return Ok(());
        };
        let mut locale = lock(&self.base).uformat.get_locale(u_locale::ACTUAL_LOCALE);
        if locale.is_none() {
            // Constructor
            locale = s.symbols.get_locale_type(u_locale::ACTUAL_LOCALE).cloned();
        }
        let locale = match locale {
            Some(l) => l,
            // Deserialization
            None => s.symbols.get_u_locale(),
        };
        s.formatter = Some(
            NumberFormatter::from_decimal_format(&s.properties, s.symbols.clone(), Some(exported))?.locale(&locale),
        );

        // Lazy-initialize the parsers only when we need them.
        s.parser = None;
        s.currency_parser = None;
        Ok(())
    }

    /// Equivalent to the package-private `getParser()`.
    fn get_parser(&self) -> Result<Arc<NumberParserImpl>> {
        self.with_state(|_, s| {
            if let Some(p) = s.parser.as_ref() {
                return Ok(p.clone());
            }
            let p = Arc::new(NumberParserImpl::create_parser_from_properties(&s.properties, &s.symbols, false)?);
            s.parser = Some(p.clone());
            Ok(p)
        })
    }

    /// Equivalent to the package-private `getCurrencyParser()`.
    fn get_currency_parser(&self) -> Result<Arc<NumberParserImpl>> {
        self.with_state(|_, s| {
            if let Some(p) = s.currency_parser.as_ref() {
                return Ok(p.clone());
            }
            let p = Arc::new(NumberParserImpl::create_parser_from_properties(&s.properties, &s.symbols, true)?);
            s.currency_parser = Some(p.clone());
            Ok(p)
        })
    }

    /// When parsing, convert java.math.BigDecimal to
    /// com.ibm.icu.math.BigDecimal and handle exponents that are out of
    /// range. Equivalent to `safeConvertBigDecimal(java.math.BigDecimal)`.
    fn safe_convert_big_decimal(number: &BigDecimal) -> JavaNumber {
        match IcuBigDecimal::from_java_big_decimal(number) {
            Ok(n) => JavaNumber::IcuBigDecimal(n),
            Err(_) => {
                // The NumberFormatException of an exponent out of range.
                if number.signum() > 0 && number.scale() < 0 {
                    JavaNumber::Double(f64::INFINITY)
                } else if number.scale() < 0 {
                    JavaNumber::Double(f64::NEG_INFINITY)
                } else if number.signum() < 0 {
                    JavaNumber::Double(-0.0)
                } else {
                    JavaNumber::Double(0.0)
                }
            }
        }
    }

    /// Updates the property bag with settings from the given pattern.
    /// Equivalent to the package-private `setPropertiesFromPattern(String,
    /// int)`.
    fn set_properties_from_pattern(s: &mut State, pattern: Option<&[u16]>, ignore_rounding: i32) -> Result<()> {
        let pattern = pattern.ok_or_else(|| LuceneError::null_pointer())?;
        PatternStringParser::parse_to_existing_properties_ignore(Some(pattern), &mut s.properties, ignore_rounding)
    }

    /// Equivalent to the package-private static
    /// `fieldPositionHelper(DecimalQuantity, FormattedStringBuilder,
    /// FieldPosition, int)`.
    pub(crate) fn field_position_helper(
        dq: &dyn DecimalQuantity,
        string: &FormattedStringBuilder,
        field_position: &mut FieldPosition,
        offset: i32,
    ) -> Result<()> {
        // always return first occurrence:
        field_position.set_begin_index(0);
        field_position.set_end_index(0);
        dq.populate_u_field_position(field_position);
        let found = FormattedValueStringBuilderImpl::next_field_position(string, field_position)?;
        if found && offset != 0 {
            field_position.set_begin_index(field_position.get_begin_index() + offset);
            field_position.set_end_index(field_position.get_end_index() + offset);
        }
        Ok(())
    }

    /// Calls the given function on the property bag and refreshes the
    /// formatter. Equivalent to the internal `setProperties(PropertySetter)`.
    ///
    /// # Errors
    ///
    /// The errors of the refresh.
    pub fn set_properties(&self, func: &dyn PropertySetter) -> Result<()> {
        self.update(|p| func.set(p))
    }

    /// `parse(String, ParsePosition)` body; `None` position is Java's
    /// `null` (a fresh `ParsePosition(0)`).
    fn parse_impl(&self, text: &[u16], parse_position: &mut ParsePosition) -> Result<Option<JavaNumber>> {
        if parse_position.get_index() < 0 {
            return Err(LuceneError::illegal_argument("Cannot start parsing at a negative offset"));
        }
        if parse_position.get_index() >= text.len() as i32 {
            // For backwards compatibility, this is not an exception, just an empty result.
            return Ok(None);
        }

        let mut result = ParsedNumber::new();
        // Note: if this is a currency instance, currencies will be matched despite the fact that we are not in the
        // parseCurrency method (backwards compatibility)
        let start_index = parse_position.get_index();
        let parser = self.get_parser()?;
        parser.parse_from(text, start_index, true, &mut result)?;
        if result.success() {
            parse_position.set_index(result.char_end);
            // TODO: Accessing properties here is technically not thread-safe
            let number = result.get_number_flags(parser.get_parse_flags())?;
            // Backwards compatibility: return com.ibm.icu.math.BigDecimal
            Ok(Some(match number {
                JavaNumber::BigDecimal(bd) => Self::safe_convert_big_decimal(&bd),
                other => other,
            }))
        } else {
            parse_position.set_error_index(start_index + result.char_end);
            Ok(None)
        }
    }

    fn parse_currency_impl(&self, text: &[u16], parse_position: &mut ParsePosition) -> Result<Option<CurrencyAmount>> {
        if parse_position.get_index() < 0 {
            return Err(LuceneError::illegal_argument("Cannot start parsing at a negative offset"));
        }
        if parse_position.get_index() >= text.len() as i32 {
            // For backwards compatibility, this is not an exception, just an empty result.
            return Ok(None);
        }

        let mut result = ParsedNumber::new();
        let start_index = parse_position.get_index();
        let parser = self.get_currency_parser()?;
        parser.parse_from(text, start_index, true, &mut result)?;
        if result.success() {
            parse_position.set_index(result.char_end);
            // TODO: Accessing properties here is technically not thread-safe
            let number = result.get_number_flags(parser.get_parse_flags())?;
            // Backwards compatibility: return com.ibm.icu.math.BigDecimal
            let number = match number {
                JavaNumber::BigDecimal(bd) => Self::safe_convert_big_decimal(&bd),
                other => other,
            };
            let currency = Currency::get_instance(result.currency_code.as_deref())?;
            Ok(Some(CurrencyAmount::new(Some(number), Some(currency))?))
        } else {
            parse_position.set_error_index(start_index + result.char_end);
            Ok(None)
        }
    }
}

/// An interface to be used with {@link #setProperties}.
///
/// Equivalent to the internal interface `DecimalFormat.PropertySetter`.
pub trait PropertySetter {
    /// Equivalent to `set(DecimalFormatProperties)`.
    fn set(&self, props: &mut DecimalFormatProperties);
}

impl NumberFormat for DecimalFormat {
    fn number_format_base(&self) -> &Mutex<NumberFormatBase> {
        &self.base
    }

    fn format_double_to(&self, number: f64, result: &mut Vec<u16>, field_position: &mut FieldPosition) -> Result<()> {
        self.format_quantity(DecimalQuantity_DualStorageBCD::from_double(number), result, field_position)
    }

    fn format_long_to(&self, number: i64, result: &mut Vec<u16>, field_position: &mut FieldPosition) -> Result<()> {
        self.format_quantity(DecimalQuantity_DualStorageBCD::from_long(number), result, field_position)
    }

    fn format_big_integer_to(&self, number: &BigInteger, result: &mut Vec<u16>, field_position: &mut FieldPosition) -> Result<()> {
        self.format_quantity(DecimalQuantity_DualStorageBCD::from_big_integer(number), result, field_position)
    }

    fn format_big_decimal_to(&self, number: &BigDecimal, result: &mut Vec<u16>, field_position: &mut FieldPosition) -> Result<()> {
        self.format_quantity(DecimalQuantity_DualStorageBCD::from_big_decimal(number)?, result, field_position)
    }

    fn format_icu_big_decimal_to(
        &self,
        number: &IcuBigDecimal,
        result: &mut Vec<u16>,
        field_position: &mut FieldPosition,
    ) -> Result<()> {
        let dq = DecimalQuantity_DualStorageBCD::from_number(&JavaNumber::IcuBigDecimal(number.clone()))?;
        self.format_quantity(dq, result, field_position)
    }

    fn format_currency_amount_to(
        &self,
        curr_amt: &CurrencyAmount,
        result: &mut Vec<u16>,
        field_position: &mut FieldPosition,
    ) -> Result<()> {
        // We need to make localSymbols in order for monetary symbols to be initialized.
        // Also, bypass the CurrencyAmount override of LocalizedNumberFormatter#format,
        // because its caching mechanism will not provide any benefit here.
        let mut local_symbols = self.read(|s| (*s.symbols).clone());
        local_symbols.set_currency(Some(curr_amt.get_currency()))?;

        let mut dq = DecimalQuantity_DualStorageBCD::from_number(curr_amt.get_number())?;
        let mut string = FormattedStringBuilder::new();
        self.formatter()
            .symbols(&local_symbols)
            .unit(Some(curr_amt.get_currency().into_measure_unit()))
            .format_impl(&mut dq, &mut string)?;
        Self::field_position_helper(&dq, &string, field_position, result.len() as i32)?;
        result.extend_from_slice(&string.to_java_string());
        Ok(())
    }

    fn parse(&self, text: &[u16], parse_position: &mut ParsePosition) -> Result<Option<JavaNumber>> {
        if self.class == DecimalFormatClass::CompactDecimalFormat {
            // Parsing is not supported in CompactDecimalFormat.
            return Err(LuceneError::unsupported_operation(""));
        }
        self.parse_impl(text, parse_position)
    }

    fn parse_currency(&self, text: &[u16], parse_position: &mut ParsePosition) -> Result<Option<CurrencyAmount>> {
        if self.class == DecimalFormatClass::CompactDecimalFormat {
            // Parsing is not supported in CompactDecimalFormat.
            return Err(LuceneError::unsupported_operation(""));
        }
        self.parse_currency_impl(text, parse_position)
    }

    fn clone_number_format(&self) -> Box<dyn NumberFormat> {
        // Java's clone() cannot fail; the refresh repeats one that succeeded.
        Box::new(self.clone_decimal_format().unwrap_or_else(|e| panic!("{e}")))
    }

    fn get_rounding_mode(&self) -> Result<i32> {
        Ok(self.read(|s| Self::exported(s).get_rounding_mode().map_or(0, |m| m.ordinal())))
    }

    fn set_rounding_mode(&self, rounding_mode: i32) -> Result<()> {
        let mode = RoundingMode::value_of(rounding_mode)?;
        self.update(|p| {
            p.set_rounding_mode(Some(mode));
        })
    }

    fn get_minimum_integer_digits(&self) -> i32 {
        self.read(|s| Self::exported(s).get_minimum_integer_digits())
    }

    fn set_minimum_integer_digits(&self, value: i32) -> Result<()> {
        // For backwards compatibility, conflicting min/max need to keep the most recent setting.
        self.update(|p| {
            let max = p.get_maximum_integer_digits();
            if max >= 0 && max < value {
                p.set_maximum_integer_digits(value);
            }
            p.set_minimum_integer_digits(value);
        })
    }

    fn get_maximum_integer_digits(&self) -> i32 {
        self.read(|s| Self::exported(s).get_maximum_integer_digits())
    }

    fn set_maximum_integer_digits(&self, value: i32) -> Result<()> {
        // For backwards compatibility, conflicting min/max need to keep the most recent setting.
        self.update(|p| {
            let min = p.get_minimum_integer_digits();
            if min >= 0 && min > value {
                p.set_minimum_integer_digits(value);
            }
            p.set_maximum_integer_digits(value);
        })
    }

    fn get_minimum_fraction_digits(&self) -> i32 {
        self.read(|s| Self::exported(s).get_minimum_fraction_digits())
    }

    fn set_minimum_fraction_digits(&self, value: i32) -> Result<()> {
        // For backwards compatibility, conflicting min/max need to keep the most recent setting.
        self.update(|p| {
            let max = p.get_maximum_fraction_digits();
            if max >= 0 && max < value {
                p.set_maximum_fraction_digits(value);
            }
            p.set_minimum_fraction_digits(value);
        })
    }

    fn get_maximum_fraction_digits(&self) -> i32 {
        self.read(|s| Self::exported(s).get_maximum_fraction_digits())
    }

    fn set_maximum_fraction_digits(&self, value: i32) -> Result<()> {
        // For backwards compatibility, conflicting min/max need to keep the most recent setting.
        self.update(|p| {
            let min = p.get_minimum_fraction_digits();
            if min >= 0 && min > value {
                p.set_minimum_fraction_digits(value);
            }
            p.set_maximum_fraction_digits(value);
        })
    }

    fn is_grouping_used(&self) -> bool {
        self.read(|s| s.properties.get_grouping_used())
    }

    fn set_grouping_used(&self, enabled: bool) -> Result<()> {
        self.update(|p| {
            p.set_grouping_used(enabled);
        })
    }

    fn get_currency(&self) -> Option<Currency> {
        self.read(|s| Self::exported(s).get_currency())
    }

    fn set_currency(&self, currency: Option<Currency>) -> Result<()> {
        self.with_state(|me, s| {
            s.properties.set_currency(currency.clone());
            if let Some(c) = currency {
                Arc::make_mut(&mut s.symbols).set_currency(Some(c))?;
            }
            me.refresh_formatter(s)
        })
    }

    fn is_parse_strict(&self) -> bool {
        self.read(|s| s.properties.get_parse_mode() == Some(ParseMode::Strict))
    }

    fn set_parse_strict(&self, parse_strict: bool) -> Result<()> {
        let mode = if parse_strict { ParseMode::Strict } else { ParseMode::Lenient };
        self.update(|p| {
            p.set_parse_mode(Some(mode));
        })
    }

    fn is_parse_integer_only(&self) -> bool {
        self.read(|s| s.properties.get_parse_integer_only())
    }

    fn set_parse_integer_only(&self, parse_integer_only: bool) -> Result<()> {
        self.update(|p| {
            p.set_parse_integer_only(parse_integer_only);
        })
    }

    fn hash_code(&self) -> i32 {
        self.read(|s| s.properties.hash_code() ^ s.symbols.hash_code())
    }

    fn java_equals(&self, obj: &dyn NumberFormat) -> bool {
        match as_decimal_format(obj) {
            Some(other) => self.equals_decimal_format(other),
            None => false,
        }
    }
}

/// `instanceof DecimalFormat`: the `DecimalFormat` behind a
/// `NumberFormat`, a `CompactDecimalFormat` included.
pub fn as_decimal_format(obj: &dyn NumberFormat) -> Option<&DecimalFormat> {
    let any = Format::as_any(obj);
    any.downcast_ref::<DecimalFormat>()
        .or_else(|| any.downcast_ref::<crate::text::compact_decimal_format::CompactDecimalFormat>().map(|c| &**c))
}

impl Format for DecimalFormat {
    fn format_object_to(&self, obj: &dyn Any, to_append_to: &mut Vec<u16>, pos: &mut FieldPosition) -> Result<()> {
        self.format_any_to(obj, to_append_to, pos)
    }

    fn parse_object_at(&self, source: &[u16], pos: &mut ParsePosition) -> Result<Option<ParsedObject>> {
        Ok(NumberFormat::parse(self, source, pos)?.map(|n| Box::new(n) as ParsedObject))
    }

    fn clone_format(&self) -> Box<dyn Format> {
        Box::new(self.clone_decimal_format().unwrap_or_else(|e| panic!("{e}")))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    /// Equivalent to `formatToCharacterIterator(Object)`.
    fn format_to_character_iterator(&self, obj: &dyn Any) -> Result<Box<dyn AttributedCharacterIterator>> {
        let number = if let Some(n) = obj.downcast_ref::<JavaNumber>() {
            n.clone()
        } else if let Some(l) = obj.downcast_ref::<i64>() {
            JavaNumber::Long(*l)
        } else if let Some(d) = obj.downcast_ref::<f64>() {
            JavaNumber::Double(*d)
        } else if let Some(i) = obj.downcast_ref::<i32>() {
            JavaNumber::Integer(*i)
        } else if let Some(f) = obj.downcast_ref::<f32>() {
            JavaNumber::Float(*f)
        } else if let Some(b) = obj.downcast_ref::<BigInteger>() {
            JavaNumber::BigInteger(b.clone())
        } else if let Some(b) = obj.downcast_ref::<BigDecimal>() {
            JavaNumber::BigDecimal(b.clone())
        } else if let Some(b) = obj.downcast_ref::<IcuBigDecimal>() {
            JavaNumber::IcuBigDecimal(b.clone())
        } else {
            return Err(crate::jdk::illegal_argument_no_message());
        };
        let output = self.formatter().format_number(&number)?;
        crate::text::formatted_value::FormattedValue::to_character_iterator(&output)
    }
}

