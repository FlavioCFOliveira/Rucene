// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.CompactDecimalFormat`.

use std::any::Any;
use std::ops::Deref;
use std::sync::Mutex;

use rucene::error::Result;
use rucene_queryparser::jdk::locale::Locale;

use crate::jdk::math::{BigDecimal, BigInteger};
use crate::jdk::text::{AttributedCharacterIterator, FieldPosition, Format, ParsedObject};
use crate::jdk::ParsePosition;
use crate::math::big_decimal::BigDecimal as IcuBigDecimal;
use crate::r#impl::number::decimal_quantity_dual_storage_bcd::JavaNumber;
use crate::text::decimal_format::DecimalFormat;
use crate::text::number_format::{NumberFormat, NumberFormatBase};
use crate::util::currency::Currency;
use crate::util::currency_amount::CurrencyAmount;
use crate::util::u_locale::ULocale;

/// `serialVersionUID`.
pub const SERIAL_VERSION_UID: i64 = 4716293295276629682;

/// Style parameter for CompactDecimalFormat.
///
/// Equivalent to `CompactDecimalFormat.CompactStyle`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompactStyle {
    /// Short version, like "1.2T"
    Short,
    /// Longer version, like "1.2 trillion", if available. May return same
    /// result as SHORT if not.
    Long,
}

impl CompactStyle {
    /// `name()`.
    pub fn name(self) -> &'static str {
        match self {
            CompactStyle::Short => "SHORT",
            CompactStyle::Long => "LONG",
        }
    }
}

/// Formats numbers in compact (abbreviated) notation, like "1.2K" instead
/// of "1200".
///
/// Equivalent to `com.ibm.icu.text.CompactDecimalFormat` (a `DecimalFormat`
/// subclass): it dereferences to its [`DecimalFormat`], whose runtime class
/// is `CompactDecimalFormat` (so `parse` and `parseCurrency` throw
/// `UnsupportedOperationException` through the base as well).
#[derive(Debug)]
pub struct CompactDecimalFormat(DecimalFormat);

impl Deref for CompactDecimalFormat {
    type Target = DecimalFormat;

    fn deref(&self) -> &DecimalFormat {
        &self.0
    }
}

impl CompactDecimalFormat {
    /// Creates a CompactDecimalFormat appropriate for a locale. The result
    /// may be affected by the number system in the locale, such as
    /// ar-u-nu-latn. Equivalent to `getInstance(ULocale, CompactStyle)`.
    ///
    /// # Errors
    ///
    /// The errors of the locale data.
    pub fn get_instance(locale: &ULocale, style: CompactStyle) -> Result<CompactDecimalFormat> {
        Self::new(locale, style)
    }

    /// Equivalent to `getInstance(Locale, CompactStyle)`.
    ///
    /// # Errors
    ///
    /// The errors of the locale data.
    pub fn get_instance_locale(locale: &Locale, style: CompactStyle) -> Result<CompactDecimalFormat> {
        let uloc = ULocale::for_locale(Some(locale))?.unwrap_or_else(ULocale::root);
        Self::new(&uloc, style)
    }

    /// The package-private constructor `CompactDecimalFormat(ULocale,
    /// CompactStyle)`.
    pub(crate) fn new(locale: &ULocale, style: CompactStyle) -> Result<Self> {
        Ok(CompactDecimalFormat(DecimalFormat::new_compact(locale, style)?))
    }
}

impl NumberFormat for CompactDecimalFormat {
    fn number_format_base(&self) -> &Mutex<NumberFormatBase> {
        self.0.number_format_base()
    }
    fn format_double_to(&self, n: f64, r: &mut Vec<u16>, p: &mut FieldPosition) -> Result<()> {
        self.0.format_double_to(n, r, p)
    }
    fn format_long_to(&self, n: i64, r: &mut Vec<u16>, p: &mut FieldPosition) -> Result<()> {
        self.0.format_long_to(n, r, p)
    }
    fn format_big_integer_to(&self, n: &BigInteger, r: &mut Vec<u16>, p: &mut FieldPosition) -> Result<()> {
        self.0.format_big_integer_to(n, r, p)
    }
    fn format_big_decimal_to(&self, n: &BigDecimal, r: &mut Vec<u16>, p: &mut FieldPosition) -> Result<()> {
        self.0.format_big_decimal_to(n, r, p)
    }
    fn format_icu_big_decimal_to(&self, n: &IcuBigDecimal, r: &mut Vec<u16>, p: &mut FieldPosition) -> Result<()> {
        self.0.format_icu_big_decimal_to(n, r, p)
    }
    fn format_currency_amount_to(&self, c: &CurrencyAmount, r: &mut Vec<u16>, p: &mut FieldPosition) -> Result<()> {
        self.0.format_currency_amount_to(c, r, p)
    }
    /// Parsing is not supported. Equivalent to `parse(String,
    /// ParsePosition)`.
    fn parse(&self, text: &[u16], pos: &mut ParsePosition) -> Result<Option<JavaNumber>> {
        NumberFormat::parse(&self.0, text, pos)
    }
    /// Parsing is not supported. Equivalent to `parseCurrency(CharSequence,
    /// ParsePosition)`.
    fn parse_currency(&self, text: &[u16], pos: &mut ParsePosition) -> Result<Option<CurrencyAmount>> {
        self.0.parse_currency(text, pos)
    }
    fn clone_number_format(&self) -> Box<dyn NumberFormat> {
        Box::new(CompactDecimalFormat(self.0.clone_decimal_format().unwrap_or_else(|e| panic!("{e}"))))
    }
    fn get_rounding_mode(&self) -> Result<i32> {
        self.0.get_rounding_mode()
    }
    fn set_rounding_mode(&self, m: i32) -> Result<()> {
        self.0.set_rounding_mode(m)
    }
    fn get_minimum_integer_digits(&self) -> i32 {
        self.0.get_minimum_integer_digits()
    }
    fn set_minimum_integer_digits(&self, v: i32) -> Result<()> {
        self.0.set_minimum_integer_digits(v)
    }
    fn get_maximum_integer_digits(&self) -> i32 {
        self.0.get_maximum_integer_digits()
    }
    fn set_maximum_integer_digits(&self, v: i32) -> Result<()> {
        self.0.set_maximum_integer_digits(v)
    }
    fn get_minimum_fraction_digits(&self) -> i32 {
        self.0.get_minimum_fraction_digits()
    }
    fn set_minimum_fraction_digits(&self, v: i32) -> Result<()> {
        self.0.set_minimum_fraction_digits(v)
    }
    fn get_maximum_fraction_digits(&self) -> i32 {
        self.0.get_maximum_fraction_digits()
    }
    fn set_maximum_fraction_digits(&self, v: i32) -> Result<()> {
        self.0.set_maximum_fraction_digits(v)
    }
    fn is_grouping_used(&self) -> bool {
        self.0.is_grouping_used()
    }
    fn set_grouping_used(&self, v: bool) -> Result<()> {
        self.0.set_grouping_used(v)
    }
    fn get_currency(&self) -> Option<Currency> {
        self.0.get_currency()
    }
    fn set_currency(&self, c: Option<Currency>) -> Result<()> {
        self.0.set_currency(c)
    }
    fn is_parse_strict(&self) -> bool {
        self.0.is_parse_strict()
    }
    fn set_parse_strict(&self, v: bool) -> Result<()> {
        self.0.set_parse_strict(v)
    }
    fn is_parse_integer_only(&self) -> bool {
        self.0.is_parse_integer_only()
    }
    fn set_parse_integer_only(&self, v: bool) -> Result<()> {
        self.0.set_parse_integer_only(v)
    }
    fn hash_code(&self) -> i32 {
        NumberFormat::hash_code(&self.0)
    }
    fn java_equals(&self, obj: &dyn NumberFormat) -> bool {
        self.0.java_equals(obj)
    }
}

impl Format for CompactDecimalFormat {
    fn format_object_to(&self, obj: &dyn Any, to_append_to: &mut Vec<u16>, pos: &mut FieldPosition) -> Result<()> {
        self.format_any_to(obj, to_append_to, pos)
    }

    fn parse_object_at(&self, source: &[u16], pos: &mut ParsePosition) -> Result<Option<ParsedObject>> {
        Ok(NumberFormat::parse(self, source, pos)?.map(|n| Box::new(n) as ParsedObject))
    }

    fn clone_format(&self) -> Box<dyn Format> {
        Box::new(CompactDecimalFormat(self.0.clone_decimal_format().unwrap_or_else(|e| panic!("{e}"))))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn format_to_character_iterator(&self, obj: &dyn Any) -> Result<Box<dyn AttributedCharacterIterator>> {
        self.0.format_to_character_iterator(obj)
    }
}
