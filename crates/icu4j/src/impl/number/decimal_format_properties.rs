// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.DecimalFormatProperties`.

use std::sync::{Arc, LazyLock};

use rucene::error::{LuceneError, Result};
use rucene::util::java_collections::{java_identity_hash_code_stand_in, java_string_hash_code};

use crate::jdk::math::{BigDecimal, MathContext, RoundingMode};
use crate::r#impl::number::macro_props::enum_hash_code;
use crate::r#impl::number::padder::PadPosition;
use crate::number::compact_notation::CompactCustomData;
use crate::text::compact_decimal_format::CompactStyle;
use crate::text::currency_plural_info::CurrencyPluralInfo;
use crate::text::plural_rules::PluralRules;
use crate::util::currency::{Currency, CurrencyUsage};

/// A Java `String` reference held by the properties.
///
/// **Divergence.** ICU compares two of these references with `!=`
/// (`PropertiesAffixPatternProvider.hasNegativeSubpattern`,
/// `PropertiesAffixPatternProvider.java:126`), so the port keeps Java's
/// reference identity: a string is an `Arc<[u16]>`, a copy of the reference
/// is an `Arc` clone, and the interned `""` literal is the one shared
/// [`empty_literal`].
pub type JString = Arc<[u16]>;

static EMPTY_LITERAL: LazyLock<JString> = LazyLock::new(|| Arc::from(Vec::<u16>::new()));

/// The interned Java literal `""`: every `""` of the Java sources is this one
/// reference.
pub fn empty_literal() -> JString {
    EMPTY_LITERAL.clone()
}

/// A new Java `String` object holding `s` (`new String(...)`, `substring`,
/// `StringBuilder.toString()`): never the same reference as another.
pub fn new_jstring(s: &[u16]) -> JString {
    Arc::from(s.to_vec())
}

/// `serialVersionUID`.
pub const SERIAL_VERSION_UID: i64 = 4095518955889349243;

static DEFAULT: LazyLock<DecimalFormatProperties> = LazyLock::new(DecimalFormatProperties::new);

/// Equivalent to `DecimalFormatProperties.ParseMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParseMode {
    /// Lenient mode should be used if you want to accept malformed user input. It will use
    /// heuristics to attempt to parse through typographical errors in the string.
    Lenient,
    /// Strict mode should be used if you want to require that the input is well-formed. More
    /// specifically, it differs from lenient mode in the following ways:
    /// grouping widths must match the grouping settings, a decimal separator is not accepted in
    /// integer-only parsing, affixes must match exactly, and the string must be fully consumed.
    Strict,
    /// Internal parse mode for increased compatibility with java.text.DecimalFormat. Used by
    /// Android libcore to implement java.text.DecimalFormat.
    JavaCompatibility,
}

impl ParseMode {
    /// `name()`.
    pub fn name(self) -> &'static str {
        match self {
            ParseMode::Lenient => "LENIENT",
            ParseMode::Strict => "STRICT",
            ParseMode::JavaCompatibility => "JAVA_COMPATIBILITY",
        }
    }

    /// `hashCode()` (identity): see `java_identity_hash_code_stand_in`.
    pub fn hash_code(self) -> i32 {
        enum_hash_code("com.ibm.icu.impl.number.DecimalFormatProperties$ParseMode", self.name())
    }
}

/// A value of one declared field, as Java reflection (`Field.get`) boxes it.
///
/// It is the unit of `toStringBare` and of the custom serialized form
/// (`writeObjectImpl`/`readObjectImpl`).
#[derive(Debug, Clone)]
pub enum PropertyValue {
    /// `Map<String, Map<String, String>>`.
    CompactCustomData(Arc<CompactCustomData>),
    /// `CompactStyle`.
    CompactStyle(CompactStyle),
    /// `Currency`.
    Currency(Currency),
    /// `CurrencyPluralInfo`.
    CurrencyPluralInfo(Arc<CurrencyPluralInfo>),
    /// `CurrencyUsage`.
    CurrencyUsage(CurrencyUsage),
    /// `Boolean`.
    Boolean(bool),
    /// `Integer`.
    Integer(i32),
    /// `MathContext`.
    MathContext(MathContext),
    /// `BigDecimal`.
    BigDecimal(BigDecimal),
    /// `String`.
    String(JString),
    /// `PadPosition`.
    PadPosition(PadPosition),
    /// `ParseMode`.
    ParseMode(ParseMode),
    /// `PluralRules`.
    PluralRules(Arc<PluralRules>),
    /// `RoundingMode`.
    RoundingMode(RoundingMode),
}

impl PropertyValue {
    /// `equals(Object)` of the boxed value.
    pub fn java_equals(&self, other: &PropertyValue) -> bool {
        use PropertyValue as V;
        match (self, other) {
            (V::CompactCustomData(a), V::CompactCustomData(b)) => Arc::ptr_eq(a, b) || **a == **b,
            (V::CompactStyle(a), V::CompactStyle(b)) => a == b,
            (V::Currency(a), V::Currency(b)) => a == b,
            (V::CurrencyPluralInfo(a), V::CurrencyPluralInfo(b)) => a.java_equals(b),
            (V::CurrencyUsage(a), V::CurrencyUsage(b)) => a == b,
            (V::Boolean(a), V::Boolean(b)) => a == b,
            (V::Integer(a), V::Integer(b)) => a == b,
            (V::MathContext(a), V::MathContext(b)) => a.java_equals(b),
            (V::BigDecimal(a), V::BigDecimal(b)) => a.java_equals(b),
            (V::String(a), V::String(b)) => a[..] == b[..],
            (V::PadPosition(a), V::PadPosition(b)) => a == b,
            (V::ParseMode(a), V::ParseMode(b)) => a == b,
            (V::PluralRules(a), V::PluralRules(b)) => Arc::ptr_eq(a, b) || a.equals(Some(b)),
            (V::RoundingMode(a), V::RoundingMode(b)) => a == b,
            _ => false,
        }
    }

    /// `hashCode()` of the boxed value.
    pub fn hash_code(&self) -> i32 {
        use PropertyValue as V;
        match self {
            V::CompactCustomData(m) => compact_custom_data_hash_code(m),
            V::CompactStyle(s) => enum_hash_code("com.ibm.icu.text.CompactDecimalFormat$CompactStyle", s.name()),
            V::Currency(c) => c.hash_code(),
            V::CurrencyPluralInfo(c) => c.hash_code(),
            V::CurrencyUsage(u) => enum_hash_code("com.ibm.icu.util.Currency$CurrencyUsage", currency_usage_name(*u)),
            V::Boolean(b) => {
                if *b {
                    1231
                } else {
                    1237
                }
            }
            V::Integer(i) => *i,
            V::MathContext(mc) => math_context_hash_code(mc),
            V::BigDecimal(b) => b.hash_code(),
            V::String(s) => string_hash_code(s),
            V::PadPosition(p) => enum_hash_code("com.ibm.icu.impl.number.Padder$PadPosition", pad_position_name(*p)),
            V::ParseMode(m) => m.hash_code(),
            V::PluralRules(r) => r.hash_code(),
            V::RoundingMode(m) => enum_hash_code("java.math.RoundingMode", m.name()),
        }
    }

    /// `String.valueOf(Object)` of the boxed value.
    pub fn to_java_string(&self) -> String {
        use PropertyValue as V;
        match self {
            V::CompactCustomData(m) => compact_custom_data_to_string(m),
            V::CompactStyle(s) => s.name().to_string(),
            V::Currency(c) => c.to_java_string(),
            V::CurrencyPluralInfo(c) => c.to_java_string(),
            V::CurrencyUsage(u) => currency_usage_name(*u).to_string(),
            V::Boolean(b) => b.to_string(),
            V::Integer(i) => i.to_string(),
            V::MathContext(mc) => mc.to_string(),
            V::BigDecimal(b) => b.to_string(),
            V::String(s) => String::from_utf16_lossy(s),
            V::PadPosition(p) => pad_position_name(*p).to_string(),
            V::ParseMode(m) => m.name().to_string(),
            V::PluralRules(r) => r.to_java_string(),
            V::RoundingMode(m) => m.name().to_string(),
        }
    }
}

/// `CurrencyUsage.name()`.
pub(crate) fn currency_usage_name(u: CurrencyUsage) -> &'static str {
    match u {
        CurrencyUsage::Standard => "STANDARD",
        CurrencyUsage::Cash => "CASH",
    }
}

/// `PadPosition.name()`.
pub(crate) fn pad_position_name(p: PadPosition) -> &'static str {
    match p {
        PadPosition::BeforePrefix => "BEFORE_PREFIX",
        PadPosition::AfterPrefix => "AFTER_PREFIX",
        PadPosition::BeforeSuffix => "BEFORE_SUFFIX",
        PadPosition::AfterSuffix => "AFTER_SUFFIX",
    }
}

/// `String.hashCode()` over UTF-16 units.
pub(crate) fn string_hash_code(s: &[u16]) -> i32 {
    s.iter().fold(0i32, |h, &c| h.wrapping_mul(31).wrapping_add(i32::from(c)))
}

/// `MathContext.hashCode()`: `roundingMode.hashCode() << 24 ^ precision`.
pub(crate) fn math_context_hash_code(mc: &MathContext) -> i32 {
    (enum_hash_code("java.math.RoundingMode", mc.get_rounding_mode().name()) << 24) ^ mc.get_precision()
}

/// `AbstractMap.hashCode()` of a `Map<String, Map<String, String>>`.
fn compact_custom_data_hash_code(m: &CompactCustomData) -> i32 {
    let mut h = 0i32;
    for (k, inner) in m.iter() {
        let mut ih = 0i32;
        for (ik, iv) in inner.iter() {
            ih = ih.wrapping_add(java_string_hash_code(ik) ^ java_string_hash_code(iv));
        }
        h = h.wrapping_add(java_string_hash_code(k) ^ ih);
    }
    h
}

/// `AbstractMap.toString()` of a `Map<String, Map<String, String>>`.
fn compact_custom_data_to_string(m: &CompactCustomData) -> String {
    let mut sb = String::from("{");
    for (i, (k, inner)) in m.iter().enumerate() {
        if i > 0 {
            sb.push_str(", ");
        }
        sb.push_str(k);
        sb.push_str("={");
        for (j, (ik, iv)) in inner.iter().enumerate() {
            if j > 0 {
                sb.push_str(", ");
            }
            sb.push_str(ik);
            sb.push('=');
            sb.push_str(iv);
        }
        sb.push('}');
    }
    sb.push('}');
    sb
}

/// The bag of formatting and parsing properties behind `DecimalFormat`.
///
/// Equivalent to `com.ibm.icu.impl.number.DecimalFormatProperties`
/// (`Cloneable`, `Serializable`): [`Clone`] is `clone()` (a shallow copy:
/// the referenced objects are shared, as `super.clone()` shares them).
///
/// **Divergence.** Java's reflective members (`toStringBare`,
/// `readObjectImpl`, `writeObjectImpl` walk `getDeclaredFields()`, which
/// HotSpot answers in declaration order) walk [`field_values`]
/// (Self::field_values), the instance fields in declaration order; the static
/// fields `DEFAULT` and `serialVersionUID` always equal themselves, so
/// `toStringBare` never prints them and the port leaves them out.
#[derive(Debug, Clone)]
pub struct DecimalFormatProperties {
    compact_custom_data: Option<Arc<CompactCustomData>>, // ICU4J-only
    compact_style: Option<CompactStyle>,
    currency: Option<Currency>,
    currency_plural_info: Option<Arc<CurrencyPluralInfo>>,
    currency_usage: Option<CurrencyUsage>,
    decimal_pattern_match_required: bool,
    decimal_separator_always_shown: bool,
    exponent_sign_always_shown: bool,
    currency_as_decimal: bool,
    format_width: i32,
    grouping_size: i32,
    grouping_used: bool,
    magnitude_multiplier: i32,
    math_context: Option<MathContext>, // ICU4J-only
    maximum_fraction_digits: i32,
    maximum_integer_digits: i32,
    maximum_significant_digits: i32,
    minimum_exponent_digits: i32,
    minimum_fraction_digits: i32,
    minimum_grouping_digits: i32,
    minimum_integer_digits: i32,
    minimum_significant_digits: i32,
    multiplier: Option<BigDecimal>,
    negative_prefix: Option<JString>,
    negative_prefix_pattern: Option<JString>,
    negative_suffix: Option<JString>,
    negative_suffix_pattern: Option<JString>,
    pad_position: Option<PadPosition>,
    pad_string: Option<JString>,
    parse_case_sensitive: bool,
    parse_integer_only: bool,
    parse_mode: Option<ParseMode>,
    parse_no_exponent: bool,
    parse_to_big_decimal: bool,
    plural_rules: Option<Arc<PluralRules>>,
    positive_prefix: Option<JString>,
    positive_prefix_pattern: Option<JString>,
    positive_suffix: Option<JString>,
    positive_suffix_pattern: Option<JString>,
    rounding_increment: Option<BigDecimal>,
    rounding_mode: Option<RoundingMode>,
    secondary_grouping_size: i32,
    sign_always_shown: bool,
}

impl Default for DecimalFormatProperties {
    fn default() -> Self {
        Self::new()
    }
}

impl PartialEq for DecimalFormatProperties {
    /// `equals(Object)`.
    fn eq(&self, other: &Self) -> bool {
        self.equals_impl(other)
    }
}

/// Generates the getter and setter of a `String` field: the getter answers
/// the text, the setter stores a new `String` object (any Java caller hands
/// over a reference no other field holds unless it came from another
/// field, which the `_jstring` pair carries).
macro_rules! jstring_accessors {
    ($field:ident, $get:ident, $get_j:ident, $set:ident, $set_j:ident, $jget:literal, $jset:literal) => {
        #[doc = concat!("Equivalent to `", $jget, "()`.")]
        pub fn $get(&self) -> Option<&[u16]> {
            self.$field.as_deref()
        }

        #[doc = concat!("`", $jget, "()` as the Java reference.")]
        pub fn $get_j(&self) -> Option<&JString> {
            self.$field.as_ref()
        }

        #[doc = concat!("Equivalent to `", $jset, "(String)` with a new `String` object.")]
        pub fn $set(&mut self, value: Option<&[u16]>) -> &mut Self {
            self.$field = value.map(new_jstring);
            self
        }

        #[doc = concat!("Equivalent to `", $jset, "(String)` with a given Java reference.")]
        pub fn $set_j(&mut self, value: Option<JString>) -> &mut Self {
            self.$field = value;
            self
        }
    };
}

/// Generates the setter of a `Copy` field (its getter is written out).
macro_rules! copy_accessors {
    ($field:ident, $ty:ty, $get:ident, $set:ident, $jget:literal, $jset:literal) => {
        #[doc = concat!("Equivalent to `", $jset, "`.")]
        pub fn $set(&mut self, value: $ty) -> &mut Self {
            self.$field = value;
            self
        }
    };
}

impl DecimalFormatProperties {
    /// Equivalent to `DecimalFormatProperties()`.
    pub fn new() -> Self {
        DecimalFormatProperties {
            compact_custom_data: None,
            compact_style: None,
            currency: None,
            currency_plural_info: None,
            currency_usage: None,
            decimal_pattern_match_required: false,
            decimal_separator_always_shown: false,
            exponent_sign_always_shown: false,
            currency_as_decimal: false,
            format_width: -1,
            grouping_size: -1,
            grouping_used: true,
            magnitude_multiplier: 0,
            math_context: None,
            maximum_fraction_digits: -1,
            maximum_integer_digits: -1,
            maximum_significant_digits: -1,
            minimum_exponent_digits: -1,
            minimum_fraction_digits: -1,
            minimum_grouping_digits: -1,
            minimum_integer_digits: -1,
            minimum_significant_digits: -1,
            multiplier: None,
            negative_prefix: None,
            negative_prefix_pattern: None,
            negative_suffix: None,
            negative_suffix_pattern: None,
            pad_position: None,
            pad_string: None,
            parse_case_sensitive: false,
            parse_integer_only: false,
            parse_mode: None,
            parse_no_exponent: false,
            parse_to_big_decimal: false,
            plural_rules: None,
            positive_prefix: None,
            positive_prefix_pattern: None,
            positive_suffix: None,
            positive_suffix_pattern: None,
            rounding_increment: None,
            rounding_mode: None,
            secondary_grouping_size: -1,
            sign_always_shown: false,
        }
    }

    /// Sets all properties to their defaults (unset). Equivalent to
    /// `clear()`.
    pub fn clear(&mut self) -> &mut Self {
        *self = Self::new();
        self
    }

    /// Copies all properties from another instance (a shallow copy: the
    /// referenced objects are shared). Equivalent to
    /// `copyFrom(DecimalFormatProperties)`.
    pub fn copy_from(&mut self, other: &DecimalFormatProperties) -> &mut Self {
        *self = other.clone();
        self
    }

    /// The declared instance fields, in declaration order, with their
    /// values as `Field.get(this)` boxes them (`None` for `null`).
    pub fn field_values(&self) -> [(&'static str, Option<PropertyValue>); 43] {
        use PropertyValue as V;
        [
            ("compactCustomData", self.compact_custom_data.clone().map(V::CompactCustomData)),
            ("compactStyle", self.compact_style.map(V::CompactStyle)),
            ("currency", self.currency.clone().map(V::Currency)),
            ("currencyPluralInfo", self.currency_plural_info.clone().map(V::CurrencyPluralInfo)),
            ("currencyUsage", self.currency_usage.map(V::CurrencyUsage)),
            ("decimalPatternMatchRequired", Some(V::Boolean(self.decimal_pattern_match_required))),
            ("decimalSeparatorAlwaysShown", Some(V::Boolean(self.decimal_separator_always_shown))),
            ("exponentSignAlwaysShown", Some(V::Boolean(self.exponent_sign_always_shown))),
            ("currencyAsDecimal", Some(V::Boolean(self.currency_as_decimal))),
            ("formatWidth", Some(V::Integer(self.format_width))),
            ("groupingSize", Some(V::Integer(self.grouping_size))),
            ("groupingUsed", Some(V::Boolean(self.grouping_used))),
            ("magnitudeMultiplier", Some(V::Integer(self.magnitude_multiplier))),
            ("mathContext", self.math_context.map(V::MathContext)),
            ("maximumFractionDigits", Some(V::Integer(self.maximum_fraction_digits))),
            ("maximumIntegerDigits", Some(V::Integer(self.maximum_integer_digits))),
            ("maximumSignificantDigits", Some(V::Integer(self.maximum_significant_digits))),
            ("minimumExponentDigits", Some(V::Integer(self.minimum_exponent_digits))),
            ("minimumFractionDigits", Some(V::Integer(self.minimum_fraction_digits))),
            ("minimumGroupingDigits", Some(V::Integer(self.minimum_grouping_digits))),
            ("minimumIntegerDigits", Some(V::Integer(self.minimum_integer_digits))),
            ("minimumSignificantDigits", Some(V::Integer(self.minimum_significant_digits))),
            ("multiplier", self.multiplier.clone().map(V::BigDecimal)),
            ("negativePrefix", self.negative_prefix.clone().map(V::String)),
            ("negativePrefixPattern", self.negative_prefix_pattern.clone().map(V::String)),
            ("negativeSuffix", self.negative_suffix.clone().map(V::String)),
            ("negativeSuffixPattern", self.negative_suffix_pattern.clone().map(V::String)),
            ("padPosition", self.pad_position.map(V::PadPosition)),
            ("padString", self.pad_string.clone().map(V::String)),
            ("parseCaseSensitive", Some(V::Boolean(self.parse_case_sensitive))),
            ("parseIntegerOnly", Some(V::Boolean(self.parse_integer_only))),
            ("parseMode", self.parse_mode.map(V::ParseMode)),
            ("parseNoExponent", Some(V::Boolean(self.parse_no_exponent))),
            ("parseToBigDecimal", Some(V::Boolean(self.parse_to_big_decimal))),
            ("pluralRules", self.plural_rules.clone().map(V::PluralRules)),
            ("positivePrefix", self.positive_prefix.clone().map(V::String)),
            ("positivePrefixPattern", self.positive_prefix_pattern.clone().map(V::String)),
            ("positiveSuffix", self.positive_suffix.clone().map(V::String)),
            ("positiveSuffixPattern", self.positive_suffix_pattern.clone().map(V::String)),
            ("roundingIncrement", self.rounding_increment.clone().map(V::BigDecimal)),
            ("roundingMode", self.rounding_mode.map(V::RoundingMode)),
            ("secondaryGroupingSize", Some(V::Integer(self.secondary_grouping_size))),
            ("signAlwaysShown", Some(V::Boolean(self.sign_always_shown))),
        ]
    }

    /// `Field.set(this, value)` of the declared field `name`: `Ok(false)`
    /// for an unknown name (`NoSuchFieldException`).
    fn set_field(&mut self, name: &str, value: Option<PropertyValue>) -> Result<bool> {
        use PropertyValue as V;
        fn mismatch(name: &str) -> LuceneError {
            LuceneError::illegal_state(format!(
                "java.lang.AssertionError: java.lang.IllegalArgumentException: Can not set field {name}"
            ))
        }
        macro_rules! obj {
            ($field:ident, $variant:ident) => {
                match value {
                    None => self.$field = None,
                    Some(V::$variant(v)) => self.$field = Some(v),
                    Some(_) => return Err(mismatch(name)),
                }
            };
        }
        macro_rules! prim {
            ($field:ident, $variant:ident) => {
                match value {
                    Some(V::$variant(v)) => self.$field = v,
                    _ => return Err(mismatch(name)),
                }
            };
        }
        match name {
            "compactCustomData" => obj!(compact_custom_data, CompactCustomData),
            "compactStyle" => obj!(compact_style, CompactStyle),
            "currency" => obj!(currency, Currency),
            "currencyPluralInfo" => obj!(currency_plural_info, CurrencyPluralInfo),
            "currencyUsage" => obj!(currency_usage, CurrencyUsage),
            "decimalPatternMatchRequired" => prim!(decimal_pattern_match_required, Boolean),
            "decimalSeparatorAlwaysShown" => prim!(decimal_separator_always_shown, Boolean),
            "exponentSignAlwaysShown" => prim!(exponent_sign_always_shown, Boolean),
            "currencyAsDecimal" => prim!(currency_as_decimal, Boolean),
            "formatWidth" => prim!(format_width, Integer),
            "groupingSize" => prim!(grouping_size, Integer),
            "groupingUsed" => prim!(grouping_used, Boolean),
            "magnitudeMultiplier" => prim!(magnitude_multiplier, Integer),
            "mathContext" => obj!(math_context, MathContext),
            "maximumFractionDigits" => prim!(maximum_fraction_digits, Integer),
            "maximumIntegerDigits" => prim!(maximum_integer_digits, Integer),
            "maximumSignificantDigits" => prim!(maximum_significant_digits, Integer),
            "minimumExponentDigits" => prim!(minimum_exponent_digits, Integer),
            "minimumFractionDigits" => prim!(minimum_fraction_digits, Integer),
            "minimumGroupingDigits" => prim!(minimum_grouping_digits, Integer),
            "minimumIntegerDigits" => prim!(minimum_integer_digits, Integer),
            "minimumSignificantDigits" => prim!(minimum_significant_digits, Integer),
            "multiplier" => obj!(multiplier, BigDecimal),
            "negativePrefix" => obj!(negative_prefix, String),
            "negativePrefixPattern" => obj!(negative_prefix_pattern, String),
            "negativeSuffix" => obj!(negative_suffix, String),
            "negativeSuffixPattern" => obj!(negative_suffix_pattern, String),
            "padPosition" => obj!(pad_position, PadPosition),
            "padString" => obj!(pad_string, String),
            "parseCaseSensitive" => prim!(parse_case_sensitive, Boolean),
            "parseIntegerOnly" => prim!(parse_integer_only, Boolean),
            "parseMode" => obj!(parse_mode, ParseMode),
            "parseNoExponent" => prim!(parse_no_exponent, Boolean),
            "parseToBigDecimal" => prim!(parse_to_big_decimal, Boolean),
            "pluralRules" => obj!(plural_rules, PluralRules),
            "positivePrefix" => obj!(positive_prefix, String),
            "positivePrefixPattern" => obj!(positive_prefix_pattern, String),
            "positiveSuffix" => obj!(positive_suffix, String),
            "positiveSuffixPattern" => obj!(positive_suffix_pattern, String),
            "roundingIncrement" => obj!(rounding_increment, BigDecimal),
            "roundingMode" => obj!(rounding_mode, RoundingMode),
            "secondaryGroupingSize" => prim!(secondary_grouping_size, Integer),
            "signAlwaysShown" => prim!(sign_always_shown, Boolean),
            // The static final fields: `Field.set` throws
            // IllegalAccessException, rethrown as AssertionError.
            "DEFAULT" | "serialVersionUID" => {
                return Err(LuceneError::illegal_state(format!(
                    "java.lang.AssertionError: java.lang.IllegalAccessException: Can not set static final field {name}"
                )))
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// `_equals(DecimalFormatProperties)`: `_equalsHelper` per field.
    fn equals_impl(&self, other: &DecimalFormatProperties) -> bool {
        let mine = self.field_values();
        let theirs = other.field_values();
        mine.iter().zip(theirs.iter()).all(|((_, a), (_, b))| match (a, b) {
            (None, None) => true,
            (Some(a), Some(b)) => a.java_equals(b),
            _ => false,
        })
    }

    /// Equivalent to `hashCode()` (`_hashCode`): `_hashCodeHelper` per field
    /// (`value ? 1 : 0` for booleans, `value * 13` for ints, `hashCode()`
    /// or 0 for objects), combined with `^`.
    pub fn hash_code(&self) -> i32 {
        let mut hash_code = 0i32;
        for (_, value) in self.field_values() {
            hash_code ^= match value {
                None => 0,
                Some(PropertyValue::Boolean(b)) => i32::from(b),
                Some(PropertyValue::Integer(i)) => i.wrapping_mul(13),
                Some(v) => v.hash_code(),
            };
        }
        hash_code
    }

    /// Equivalent to `getCompactCustomData()`.
    pub fn get_compact_custom_data(&self) -> Option<Arc<CompactCustomData>> {
        self.compact_custom_data.clone()
    }

    /// Equivalent to `getCompactStyle()`.
    pub fn get_compact_style(&self) -> Option<CompactStyle> {
        self.compact_style
    }

    /// Equivalent to `getCurrency()`.
    pub fn get_currency(&self) -> Option<Currency> {
        self.currency.clone()
    }

    /// Equivalent to `getCurrencyPluralInfo()`.
    pub fn get_currency_plural_info(&self) -> Option<Arc<CurrencyPluralInfo>> {
        self.currency_plural_info.clone()
    }

    /// Equivalent to `getCurrencyUsage()`.
    pub fn get_currency_usage(&self) -> Option<CurrencyUsage> {
        self.currency_usage
    }

    /// Equivalent to `getDecimalPatternMatchRequired()`.
    pub fn get_decimal_pattern_match_required(&self) -> bool {
        self.decimal_pattern_match_required
    }

    /// Equivalent to `getDecimalSeparatorAlwaysShown()`.
    pub fn get_decimal_separator_always_shown(&self) -> bool {
        self.decimal_separator_always_shown
    }

    /// Equivalent to `getExponentSignAlwaysShown()`.
    pub fn get_exponent_sign_always_shown(&self) -> bool {
        self.exponent_sign_always_shown
    }

    /// Equivalent to `getCurrencyAsDecimal()`.
    pub fn get_currency_as_decimal(&self) -> bool {
        self.currency_as_decimal
    }

    /// Equivalent to `getFormatWidth()`.
    pub fn get_format_width(&self) -> i32 {
        self.format_width
    }

    /// Equivalent to `getGroupingSize()`.
    pub fn get_grouping_size(&self) -> i32 {
        self.grouping_size
    }

    /// Equivalent to `getGroupingUsed()`.
    pub fn get_grouping_used(&self) -> bool {
        self.grouping_used
    }

    /// Equivalent to `getMagnitudeMultiplier()`.
    pub fn get_magnitude_multiplier(&self) -> i32 {
        self.magnitude_multiplier
    }

    /// Equivalent to `getMathContext()`.
    pub fn get_math_context(&self) -> Option<&MathContext> {
        self.math_context.as_ref()
    }

    /// Equivalent to `getMaximumFractionDigits()`.
    pub fn get_maximum_fraction_digits(&self) -> i32 {
        self.maximum_fraction_digits
    }

    /// Equivalent to `getMaximumIntegerDigits()`.
    pub fn get_maximum_integer_digits(&self) -> i32 {
        self.maximum_integer_digits
    }

    /// Equivalent to `getMaximumSignificantDigits()`.
    pub fn get_maximum_significant_digits(&self) -> i32 {
        self.maximum_significant_digits
    }

    /// Equivalent to `getMinimumExponentDigits()`.
    pub fn get_minimum_exponent_digits(&self) -> i32 {
        self.minimum_exponent_digits
    }

    /// Equivalent to `getMinimumFractionDigits()`.
    pub fn get_minimum_fraction_digits(&self) -> i32 {
        self.minimum_fraction_digits
    }

    /// Equivalent to `getMinimumGroupingDigits()`.
    pub fn get_minimum_grouping_digits(&self) -> i32 {
        self.minimum_grouping_digits
    }

    /// Equivalent to `getMinimumIntegerDigits()`.
    pub fn get_minimum_integer_digits(&self) -> i32 {
        self.minimum_integer_digits
    }

    /// Equivalent to `getMinimumSignificantDigits()`.
    pub fn get_minimum_significant_digits(&self) -> i32 {
        self.minimum_significant_digits
    }

    /// Equivalent to `getMultiplier()`.
    pub fn get_multiplier(&self) -> Option<&BigDecimal> {
        self.multiplier.as_ref()
    }

    jstring_accessors!(
        negative_prefix,
        get_negative_prefix,
        get_negative_prefix_jstring,
        set_negative_prefix,
        set_negative_prefix_jstring,
        "getNegativePrefix",
        "setNegativePrefix"
    );
    jstring_accessors!(
        negative_prefix_pattern,
        get_negative_prefix_pattern,
        get_negative_prefix_pattern_jstring,
        set_negative_prefix_pattern,
        set_negative_prefix_pattern_jstring,
        "getNegativePrefixPattern",
        "setNegativePrefixPattern"
    );
    jstring_accessors!(
        negative_suffix,
        get_negative_suffix,
        get_negative_suffix_jstring,
        set_negative_suffix,
        set_negative_suffix_jstring,
        "getNegativeSuffix",
        "setNegativeSuffix"
    );
    jstring_accessors!(
        negative_suffix_pattern,
        get_negative_suffix_pattern,
        get_negative_suffix_pattern_jstring,
        set_negative_suffix_pattern,
        set_negative_suffix_pattern_jstring,
        "getNegativeSuffixPattern",
        "setNegativeSuffixPattern"
    );

    /// Equivalent to `getPadPosition()`.
    pub fn get_pad_position(&self) -> Option<PadPosition> {
        self.pad_position
    }

    jstring_accessors!(
        pad_string,
        get_pad_string,
        get_pad_string_jstring,
        set_pad_string,
        set_pad_string_jstring,
        "getPadString",
        "setPadString"
    );

    /// Equivalent to `getParseCaseSensitive()`.
    pub fn get_parse_case_sensitive(&self) -> bool {
        self.parse_case_sensitive
    }

    /// Equivalent to `getParseIntegerOnly()`.
    pub fn get_parse_integer_only(&self) -> bool {
        self.parse_integer_only
    }

    /// Equivalent to `getParseMode()`.
    pub fn get_parse_mode(&self) -> Option<ParseMode> {
        self.parse_mode
    }

    /// Equivalent to `getParseNoExponent()`.
    pub fn get_parse_no_exponent(&self) -> bool {
        self.parse_no_exponent
    }

    /// Equivalent to `getParseToBigDecimal()`.
    pub fn get_parse_to_big_decimal(&self) -> bool {
        self.parse_to_big_decimal
    }

    /// Equivalent to `getPluralRules()`.
    pub fn get_plural_rules(&self) -> Option<Arc<PluralRules>> {
        self.plural_rules.clone()
    }

    jstring_accessors!(
        positive_prefix,
        get_positive_prefix,
        get_positive_prefix_jstring,
        set_positive_prefix,
        set_positive_prefix_jstring,
        "getPositivePrefix",
        "setPositivePrefix"
    );
    jstring_accessors!(
        positive_prefix_pattern,
        get_positive_prefix_pattern,
        get_positive_prefix_pattern_jstring,
        set_positive_prefix_pattern,
        set_positive_prefix_pattern_jstring,
        "getPositivePrefixPattern",
        "setPositivePrefixPattern"
    );
    jstring_accessors!(
        positive_suffix,
        get_positive_suffix,
        get_positive_suffix_jstring,
        set_positive_suffix,
        set_positive_suffix_jstring,
        "getPositiveSuffix",
        "setPositiveSuffix"
    );
    jstring_accessors!(
        positive_suffix_pattern,
        get_positive_suffix_pattern,
        get_positive_suffix_pattern_jstring,
        set_positive_suffix_pattern,
        set_positive_suffix_pattern_jstring,
        "getPositiveSuffixPattern",
        "setPositiveSuffixPattern"
    );

    /// Equivalent to `getRoundingIncrement()`.
    pub fn get_rounding_increment(&self) -> Option<BigDecimal> {
        self.rounding_increment.clone()
    }

    /// Equivalent to `getRoundingMode()`.
    pub fn get_rounding_mode(&self) -> Option<RoundingMode> {
        self.rounding_mode
    }

    /// Equivalent to `getSecondaryGroupingSize()`.
    pub fn get_secondary_grouping_size(&self) -> i32 {
        self.secondary_grouping_size
    }

    /// Equivalent to `getSignAlwaysShown()`.
    pub fn get_sign_always_shown(&self) -> bool {
        self.sign_always_shown
    }

    /// Equivalent to `readObjectImpl(ObjectInputStream)` after
    /// `defaultReadObject()` and the unused leading `int`: `clear()`, then
    /// each `(name, value)` pair the stream holds is set reflectively, an
    /// unknown name being skipped.
    ///
    /// # Errors
    ///
    /// The `AssertionError` Java throws when a value does not fit its field
    /// (`IllegalArgumentException`) or names a static field
    /// (`IllegalAccessException`).
    pub fn read_object_impl(&mut self, entries: Vec<(String, Option<PropertyValue>)>) -> Result<()> {
        self.clear();
        for (name, value) in entries {
            self.set_field(&name, value)?;
        }
        Ok(())
    }

    /// Equivalent to `setCompactCustomData(Map)`.
    pub fn set_compact_custom_data(&mut self, compact_custom_data: Option<Arc<CompactCustomData>>) -> &mut Self {
        self.compact_custom_data = compact_custom_data;
        self
    }

    /// Equivalent to `setCompactStyle(CompactStyle)`.
    pub fn set_compact_style(&mut self, compact_style: Option<CompactStyle>) -> &mut Self {
        self.compact_style = compact_style;
        self
    }

    /// Equivalent to `setCurrency(Currency)`.
    pub fn set_currency(&mut self, currency: Option<Currency>) -> &mut Self {
        self.currency = currency;
        self
    }

    /// Equivalent to `setCurrencyPluralInfo(CurrencyPluralInfo)`: stores a
    /// clone.
    pub fn set_currency_plural_info(&mut self, currency_plural_info: Option<&CurrencyPluralInfo>) -> &mut Self {
        self.currency_plural_info = currency_plural_info.map(|c| Arc::new(c.clone()));
        self
    }

    /// Equivalent to `setCurrencyUsage(CurrencyUsage)`.
    pub fn set_currency_usage(&mut self, currency_usage: Option<CurrencyUsage>) -> &mut Self {
        self.currency_usage = currency_usage;
        self
    }

    copy_accessors!(
        decimal_pattern_match_required,
        bool,
        get_decimal_pattern_match_required_copy,
        set_decimal_pattern_match_required,
        "getDecimalPatternMatchRequired",
        "setDecimalPatternMatchRequired(boolean)"
    );
    copy_accessors!(
        decimal_separator_always_shown,
        bool,
        get_decimal_separator_always_shown_copy,
        set_decimal_separator_always_shown,
        "getDecimalSeparatorAlwaysShown",
        "setDecimalSeparatorAlwaysShown(boolean)"
    );
    copy_accessors!(
        exponent_sign_always_shown,
        bool,
        get_exponent_sign_always_shown_copy,
        set_exponent_sign_always_shown,
        "getExponentSignAlwaysShown",
        "setExponentSignAlwaysShown(boolean)"
    );
    copy_accessors!(
        currency_as_decimal,
        bool,
        get_currency_as_decimal_copy,
        set_currency_as_decimal,
        "getCurrencyAsDecimal",
        "setCurrencyAsDecimal(boolean)"
    );
    copy_accessors!(format_width, i32, get_format_width_copy, set_format_width, "getFormatWidth", "setFormatWidth(int)");
    copy_accessors!(grouping_size, i32, get_grouping_size_copy, set_grouping_size, "getGroupingSize", "setGroupingSize(int)");
    copy_accessors!(grouping_used, bool, get_grouping_used_copy, set_grouping_used, "getGroupingUsed", "setGroupingUsed(boolean)");
    copy_accessors!(
        magnitude_multiplier,
        i32,
        get_magnitude_multiplier_copy,
        set_magnitude_multiplier,
        "getMagnitudeMultiplier",
        "setMagnitudeMultiplier(int)"
    );

    /// Equivalent to `setMathContext(MathContext)`.
    pub fn set_math_context(&mut self, math_context: Option<MathContext>) -> &mut Self {
        self.math_context = math_context;
        self
    }

    copy_accessors!(
        maximum_fraction_digits,
        i32,
        get_maximum_fraction_digits_copy,
        set_maximum_fraction_digits,
        "getMaximumFractionDigits",
        "setMaximumFractionDigits(int)"
    );
    copy_accessors!(
        maximum_integer_digits,
        i32,
        get_maximum_integer_digits_copy,
        set_maximum_integer_digits,
        "getMaximumIntegerDigits",
        "setMaximumIntegerDigits(int)"
    );
    copy_accessors!(
        maximum_significant_digits,
        i32,
        get_maximum_significant_digits_copy,
        set_maximum_significant_digits,
        "getMaximumSignificantDigits",
        "setMaximumSignificantDigits(int)"
    );
    copy_accessors!(
        minimum_exponent_digits,
        i32,
        get_minimum_exponent_digits_copy,
        set_minimum_exponent_digits,
        "getMinimumExponentDigits",
        "setMinimumExponentDigits(int)"
    );
    copy_accessors!(
        minimum_fraction_digits,
        i32,
        get_minimum_fraction_digits_copy,
        set_minimum_fraction_digits,
        "getMinimumFractionDigits",
        "setMinimumFractionDigits(int)"
    );
    copy_accessors!(
        minimum_grouping_digits,
        i32,
        get_minimum_grouping_digits_copy,
        set_minimum_grouping_digits,
        "getMinimumGroupingDigits",
        "setMinimumGroupingDigits(int)"
    );
    copy_accessors!(
        minimum_integer_digits,
        i32,
        get_minimum_integer_digits_copy,
        set_minimum_integer_digits,
        "getMinimumIntegerDigits",
        "setMinimumIntegerDigits(int)"
    );
    copy_accessors!(
        minimum_significant_digits,
        i32,
        get_minimum_significant_digits_copy,
        set_minimum_significant_digits,
        "getMinimumSignificantDigits",
        "setMinimumSignificantDigits(int)"
    );

    /// Equivalent to `setMultiplier(BigDecimal)`.
    pub fn set_multiplier(&mut self, multiplier: Option<BigDecimal>) -> &mut Self {
        self.multiplier = multiplier;
        self
    }

    /// Equivalent to `setPadPosition(PadPosition)`.
    pub fn set_pad_position(&mut self, pad_position: Option<PadPosition>) -> &mut Self {
        self.pad_position = pad_position;
        self
    }

    copy_accessors!(
        parse_case_sensitive,
        bool,
        get_parse_case_sensitive_copy,
        set_parse_case_sensitive,
        "getParseCaseSensitive",
        "setParseCaseSensitive(boolean)"
    );
    copy_accessors!(
        parse_integer_only,
        bool,
        get_parse_integer_only_copy,
        set_parse_integer_only,
        "getParseIntegerOnly",
        "setParseIntegerOnly(boolean)"
    );

    /// Equivalent to `setParseMode(ParseMode)`.
    pub fn set_parse_mode(&mut self, parse_mode: Option<ParseMode>) -> &mut Self {
        self.parse_mode = parse_mode;
        self
    }

    copy_accessors!(
        parse_no_exponent,
        bool,
        get_parse_no_exponent_copy,
        set_parse_no_exponent,
        "getParseNoExponent",
        "setParseNoExponent(boolean)"
    );
    copy_accessors!(
        parse_to_big_decimal,
        bool,
        get_parse_to_big_decimal_copy,
        set_parse_to_big_decimal,
        "getParseToBigDecimal",
        "setParseToBigDecimal(boolean)"
    );

    /// Equivalent to `setPluralRules(PluralRules)`.
    pub fn set_plural_rules(&mut self, plural_rules: Option<Arc<PluralRules>>) -> &mut Self {
        self.plural_rules = plural_rules;
        self
    }

    /// Equivalent to `setRoundingIncrement(BigDecimal)`.
    pub fn set_rounding_increment(&mut self, rounding_increment: Option<BigDecimal>) -> &mut Self {
        self.rounding_increment = rounding_increment;
        self
    }

    /// Equivalent to `setRoundingMode(RoundingMode)`.
    pub fn set_rounding_mode(&mut self, rounding_mode: Option<RoundingMode>) -> &mut Self {
        self.rounding_mode = rounding_mode;
        self
    }

    copy_accessors!(
        secondary_grouping_size,
        i32,
        get_secondary_grouping_size_copy,
        set_secondary_grouping_size,
        "getSecondaryGroupingSize",
        "setSecondaryGroupingSize(int)"
    );
    copy_accessors!(
        sign_always_shown,
        bool,
        get_sign_always_shown_copy,
        set_sign_always_shown,
        "getSignAlwaysShown",
        "setSignAlwaysShown(boolean)"
    );

    /// Equivalent to `toString()`: `<Properties` + `toStringBare` + `>`.
    pub fn to_java_string(&self) -> String {
        let mut result = String::from("<Properties");
        self.to_string_bare(&mut result);
        result.push('>');
        result
    }

    /// Appends a string containing properties that differ from the default,
    /// but without being surrounded by `<Properties>`. Equivalent to
    /// `toStringBare(StringBuilder)`.
    pub fn to_string_bare(&self, result: &mut String) {
        let mine = self.field_values();
        let defaults = DEFAULT.field_values();
        for ((name, my_value), (_, default_value)) in mine.iter().zip(defaults.iter()) {
            match (my_value, default_value) {
                (None, None) => continue,
                (Some(m), Some(d)) if m.java_equals(d) => continue,
                _ => {
                    result.push(' ');
                    result.push_str(name);
                    result.push(':');
                    match my_value {
                        Some(v) => result.push_str(&v.to_java_string()),
                        None => result.push_str("null"),
                    }
                }
            }
        }
    }

    /// Equivalent to `writeObjectImpl(ObjectOutputStream)` after
    /// `defaultWriteObject()` and the leading `writeInt(0)`: the fields whose
    /// value is non-null and differs from the default, as the `(name, value)`
    /// pairs written after their count.
    pub fn write_object_impl(&self) -> Vec<(String, Option<PropertyValue>)> {
        let mine = self.field_values();
        let defaults = DEFAULT.field_values();
        let mut out = Vec::new();
        for ((name, my_value), (_, default_value)) in mine.into_iter().zip(defaults.iter()) {
            let Some(my_value) = my_value else {
                continue;
            };
            let equal = match default_value {
                Some(d) => my_value.java_equals(d),
                None => false,
            };
            if !equal {
                out.push((name.to_string(), Some(my_value)));
            }
        }
        out
    }
}

/// `Object.hashCode()` of an object without its own `hashCode`: see
/// `java_identity_hash_code_stand_in`.
pub(crate) fn identity_hash_code(class: &str) -> i32 {
    java_identity_hash_code_stand_in(class)
}
