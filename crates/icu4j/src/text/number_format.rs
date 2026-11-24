// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.NumberFormat`.

use std::any::Any;
use std::collections::HashSet;
use std::sync::{Arc, Mutex, MutexGuard};

use rucene::error::{LuceneError, Result};
use rucene_queryparser::jdk::locale::Locale;
use crate::util::u_locale::Category;
use rucene_queryparser::jdk::resource_bundle::MissingResourceException;

use crate::jdk::math::{BigDecimal, BigInteger};
use crate::jdk::text::{FieldPosition, Format};
use crate::jdk::{units, ParsePosition};
use crate::math::big_decimal::BigDecimal as IcuBigDecimal;
use crate::r#impl::icu_data::ICUData;
use crate::r#impl::icu_resource_bundle::ICUResourceBundle;
use crate::r#impl::number::decimal_quantity_dual_storage_bcd::JavaNumber;
use crate::text::currency_plural_info::CurrencyPluralInfo;
use crate::text::decimal_format::DecimalFormat;
use crate::text::decimal_format_symbols::DecimalFormatSymbols;
use crate::text::display_context::{self, DisplayContext};
use crate::text::number_format_service_shim::NumberFormatServiceShim;
use crate::text::numbering_system::NumberingSystem;
use crate::text::rule_based_number_format::{self, RuleBasedNumberFormat};
use crate::text::u_format::UFormatBase;
use crate::util::currency::{Currency, CurrencyUsage};
use crate::util::currency_amount::CurrencyAmount;
use crate::util::u_locale::{self, ULocale};
use crate::util::u_resource_bundle::UResourceBundle;

/// Constant to specify normal number style of format.
pub const NUMBERSTYLE: i32 = 0;
/// Constant to specify general currency style of format. Defaults to
/// STANDARDCURRENCYSTYLE, using currency symbol, for example "$3.00", with
/// non-accounting style for negative values (e.g. minus sign).
pub const CURRENCYSTYLE: i32 = 1;
/// Constant to specify a style of format to display percent.
pub const PERCENTSTYLE: i32 = 2;
/// Constant to specify a style of format to display scientific number.
pub const SCIENTIFICSTYLE: i32 = 3;
/// Constant to specify a integer number style format.
pub const INTEGERSTYLE: i32 = 4;
/// Constant to specify currency style of format which uses currency ISO
/// code to represent currency, for example: "USD3.00".
pub const ISOCURRENCYSTYLE: i32 = 5;
/// Constant to specify currency style of format which uses currency long
/// name with plural format to represent currency, for example, "3.00 US
/// Dollars".
pub const PLURALCURRENCYSTYLE: i32 = 6;
/// Constant to specify currency style of format which uses currency symbol
/// to represent currency for accounting, for example: "($3.00), instead of
/// "-$3.00" (CURRENCYSTYLE).
pub const ACCOUNTINGCURRENCYSTYLE: i32 = 7;
/// Constant to specify currency cash style of format which uses currency
/// ISO code to represent currency, for example: "NT$3" instead of
/// "NT$3.23".
pub const CASHCURRENCYSTYLE: i32 = 8;
/// Constant to specify currency style of format which uses currency symbol
/// to represent currency, for example "$3.00", using non-accounting style
/// for negative values (e.g. minus sign).
pub const STANDARDCURRENCYSTYLE: i32 = 9;

/// Field constant used to construct a FieldPosition object. Signifies that
/// the position of the integer part or fraction part of a formatted number
/// should be returned.
pub const INTEGER_FIELD: i32 = 0;
/// Field constant used to construct a FieldPosition object. Signifies that
/// the position of the fraction part of a formatted number should be
/// returned.
pub const FRACTION_FIELD: i32 = 1;

/// `currentSerialVersion`.
pub(crate) const CURRENT_SERIAL_VERSION: i32 = 2;
/// `serialVersionUID`.
pub const SERIAL_VERSION_UID: i64 = -2308460125733713944;

/// `doubleCurrencyStr`.
const DOUBLE_CURRENCY_STR: [u16; 2] = [0xA4, 0xA4];

/// `NumberFormat.Field` and its constants.
pub mod field {
    use rucene::error::{LuceneError, Result};

    use crate::jdk::text::{Attribute, FieldRef};

    crate::java_format_field_class!(
        /// The instances of this inner class are used as attribute keys and
        /// values in AttributedCharacterIterator that
        /// NumberFormat.formatToCharacterIterator() method returns.
        /// Equivalent to `NumberFormat.Field`.
        pub Field, "com.ibm.icu.text.NumberFormat$Field", []
    );

    /// `serialVersionUID`.
    pub const SERIAL_VERSION_UID: i64 = -4516273749929385842;

    /// `SIGN`.
    pub static SIGN: Field = Field::new("sign");
    /// `INTEGER`.
    pub static INTEGER: Field = Field::new("integer");
    /// `FRACTION`.
    pub static FRACTION: Field = Field::new("fraction");
    /// `EXPONENT`.
    pub static EXPONENT: Field = Field::new("exponent");
    /// `EXPONENT_SIGN`.
    pub static EXPONENT_SIGN: Field = Field::new("exponent sign");
    /// `EXPONENT_SYMBOL`.
    pub static EXPONENT_SYMBOL: Field = Field::new("exponent symbol");
    /// `DECIMAL_SEPARATOR`.
    pub static DECIMAL_SEPARATOR: Field = Field::new("decimal separator");
    /// `GROUPING_SEPARATOR`.
    pub static GROUPING_SEPARATOR: Field = Field::new("grouping separator");
    /// `PERCENT`.
    pub static PERCENT: Field = Field::new("percent");
    /// `PERMILLE`.
    pub static PERMILLE: Field = Field::new("per mille");
    /// `CURRENCY`.
    pub static CURRENCY: Field = Field::new("currency");
    /// `MEASURE_UNIT`.
    pub static MEASURE_UNIT: Field = Field::new("measure unit");
    /// `COMPACT`.
    pub static COMPACT: Field = Field::new("compact");
    /// Approximately sign. In ICU 70, this was categorized under the generic
    /// SIGN field. Equivalent to `APPROXIMATELY_SIGN`.
    pub static APPROXIMATELY_SIGN: Field = Field::new("approximately sign");

    impl Field {
        /// Serizalization method resolve instances to the constant
        /// NumberFormat.Field values. Equivalent to `readResolve()` (which
        /// does not resolve `APPROXIMATELY_SIGN`).
        ///
        /// # Errors
        ///
        /// `InvalidObjectException("An invalid object.")`.
        pub fn read_resolve(&self) -> Result<FieldRef> {
            for f in [
                &INTEGER,
                &FRACTION,
                &EXPONENT,
                &EXPONENT_SIGN,
                &EXPONENT_SYMBOL,
                &CURRENCY,
                &DECIMAL_SEPARATOR,
                &GROUPING_SEPARATOR,
                &PERCENT,
                &PERMILLE,
                &SIGN,
                &MEASURE_UNIT,
                &COMPACT,
            ] {
                if self.get_name() == f.get_name() {
                    return Ok(f);
                }
            }
            Err(LuceneError::unchecked("java.io.InvalidObjectException", "An invalid object."))
        }
    }
}

/// The private state of the abstract class `NumberFormat` (and of its
/// superclass `UFormat`).
#[derive(Debug, Clone)]
pub struct NumberFormatBase {
    /// The locales of `UFormat`.
    pub(crate) uformat: UFormatBase,
    grouping_used: bool,
    max_integer_digits: i8,
    min_integer_digits: i8,
    max_fraction_digits: i8,
    min_fraction_digits: i8,
    parse_integer_only: bool,
    maximum_integer_digits: i32,
    minimum_integer_digits: i32,
    maximum_fraction_digits: i32,
    minimum_fraction_digits: i32,
    currency: Option<Currency>,
    serial_version_on_stream: i32,
    parse_strict: bool,
    capitalization_setting: Option<DisplayContext>,
}

impl Default for NumberFormatBase {
    fn default() -> Self {
        Self::new()
    }
}

impl NumberFormatBase {
    /// Empty constructor. Equivalent to `NumberFormat()`.
    pub fn new() -> Self {
        NumberFormatBase {
            uformat: UFormatBase::new(),
            grouping_used: true,
            max_integer_digits: 40,
            min_integer_digits: 1,
            max_fraction_digits: 3,
            min_fraction_digits: 0,
            parse_integer_only: false,
            maximum_integer_digits: 40,
            minimum_integer_digits: 1,
            maximum_fraction_digits: 3,
            minimum_fraction_digits: 0,
            currency: None,
            serial_version_on_stream: CURRENT_SERIAL_VERSION,
            parse_strict: false,
            capitalization_setting: Some(DisplayContext::CapitalizationNone),
        }
    }

    /// `hashCode()`: `maximumIntegerDigits * 37 + maxFractionDigits`.
    pub fn hash_code(&self) -> i32 {
        self.maximum_integer_digits.wrapping_mul(37).wrapping_add(i32::from(self.max_fraction_digits))
    }

    /// The field comparisons of `equals(Object)` (after the class check).
    pub fn fields_equal(&self, other: &NumberFormatBase) -> bool {
        self.maximum_integer_digits == other.maximum_integer_digits
            && self.minimum_integer_digits == other.minimum_integer_digits
            && self.maximum_fraction_digits == other.maximum_fraction_digits
            && self.minimum_fraction_digits == other.minimum_fraction_digits
            && self.grouping_used == other.grouping_used
            && self.parse_integer_only == other.parse_integer_only
            && self.parse_strict == other.parse_strict
            && self.capitalization_setting == other.capitalization_setting
    }

    /// The `groupingUsed` field (read reflectively by `DecimalFormat`).
    pub fn grouping_used_field(&self) -> bool {
        self.grouping_used
    }

    /// The `parseIntegerOnly` field.
    pub fn parse_integer_only_field(&self) -> bool {
        self.parse_integer_only
    }

    /// The `maximumIntegerDigits` field.
    pub fn maximum_integer_digits_field(&self) -> i32 {
        self.maximum_integer_digits
    }

    /// The `minimumIntegerDigits` field.
    pub fn minimum_integer_digits_field(&self) -> i32 {
        self.minimum_integer_digits
    }

    /// The `maximumFractionDigits` field.
    pub fn maximum_fraction_digits_field(&self) -> i32 {
        self.maximum_fraction_digits
    }

    /// The `minimumFractionDigits` field.
    pub fn minimum_fraction_digits_field(&self) -> i32 {
        self.minimum_fraction_digits
    }

    /// The `currency` field.
    pub fn currency_field(&self) -> Option<Currency> {
        self.currency.clone()
    }

    /// The `parseStrict` field.
    pub fn parse_strict_field(&self) -> bool {
        self.parse_strict
    }

    /// `readObject(ObjectInputStream)` after `defaultReadObject()`: the
    /// upgrades of older streams and the digit-count validation.
    ///
    /// # Errors
    ///
    /// `InvalidObjectException("Digit count range invalid")`.
    pub fn read_object(&mut self) -> Result<()> {
        // Bug 4185761 [Richard/GCL]
        // Validate that the min <= max invariant is maintained, and correct
        // any conflicting settings.
        if self.serial_version_on_stream < 1 {
            // Didn't have additional int fields, reassign to use them.
            self.maximum_integer_digits = i32::from(self.max_integer_digits);
            self.minimum_integer_digits = i32::from(self.min_integer_digits);
            self.maximum_fraction_digits = i32::from(self.max_fraction_digits);
            self.minimum_fraction_digits = i32::from(self.min_fraction_digits);
        }
        if self.serial_version_on_stream < 2 {
            // Didn't have capitalizationSetting, set it to default
            self.capitalization_setting = Some(DisplayContext::CapitalizationNone);
        }
        if self.minimum_integer_digits > self.maximum_integer_digits
            || self.minimum_fraction_digits > self.maximum_fraction_digits
            || self.minimum_integer_digits < 0
            || self.minimum_fraction_digits < 0
        {
            return Err(LuceneError::unchecked("java.io.InvalidObjectException", "Digit count range invalid"));
        }
        self.serial_version_on_stream = CURRENT_SERIAL_VERSION;
        Ok(())
    }

    /// `writeObject(ObjectOutputStream)` before `defaultWriteObject()`: the
    /// `byte` fields take the clamped int values.
    pub fn write_object(&mut self) {
        fn clamp(v: i32) -> i8 {
            if v > i32::from(i8::MAX) {
                i8::MAX
            } else {
                v as i8
            }
        }
        self.max_integer_digits = clamp(self.maximum_integer_digits);
        self.min_integer_digits = clamp(self.minimum_integer_digits);
        self.max_fraction_digits = clamp(self.maximum_fraction_digits);
        self.min_fraction_digits = clamp(self.minimum_fraction_digits);
    }
}

/// `NumberFormat` is the abstract base class for all number formats.
///
/// Equivalent to the abstract class `com.ibm.icu.text.NumberFormat` (a
/// `UFormat`). An implementor holds its [`NumberFormatBase`] (the private
/// state of `NumberFormat`) and overrides the virtual members; the bodies of
/// `NumberFormat` are the provided methods. Java's `Number` arguments are
/// [`JavaNumber`]s, `StringBuffer` results are appended to `&mut Vec<u16>`,
/// and the `null` results of parsing are `None`.
///
/// **Divergence.** `NumberFormat` objects are mutable and their methods take
/// `&self` (Java shares them by reference): an implementor keeps its state
/// behind a `Mutex`; `format(CurrencyAmount, …)`'s `synchronized (this)`
/// block (`NumberFormat.java`, `format(CurrencyAmount, StringBuffer,
/// FieldPosition)`) locks only while reading or writing the currency, as
/// Rust's `Mutex` is not re-entrant.
pub trait NumberFormat: Format {
    /// The state of `NumberFormat`.
    fn number_format_base(&self) -> &Mutex<NumberFormatBase>;

    /// Formats a double. Equivalent to the abstract `format(double,
    /// StringBuffer, FieldPosition)`.
    ///
    /// # Errors
    ///
    /// The implementation's.
    fn format_double_to(&self, number: f64, to_append_to: &mut Vec<u16>, pos: &mut FieldPosition) -> Result<()>;

    /// Equivalent to the abstract `format(long, StringBuffer,
    /// FieldPosition)`.
    ///
    /// # Errors
    ///
    /// The implementation's.
    fn format_long_to(&self, number: i64, to_append_to: &mut Vec<u16>, pos: &mut FieldPosition) -> Result<()>;

    /// Equivalent to the abstract `format(BigInteger, StringBuffer,
    /// FieldPosition)`.
    ///
    /// # Errors
    ///
    /// The implementation's.
    fn format_big_integer_to(&self, number: &BigInteger, to_append_to: &mut Vec<u16>, pos: &mut FieldPosition) -> Result<()>;

    /// Equivalent to the abstract `format(java.math.BigDecimal,
    /// StringBuffer, FieldPosition)`.
    ///
    /// # Errors
    ///
    /// The implementation's.
    fn format_big_decimal_to(&self, number: &BigDecimal, to_append_to: &mut Vec<u16>, pos: &mut FieldPosition) -> Result<()>;

    /// Equivalent to the abstract `format(com.ibm.icu.math.BigDecimal,
    /// StringBuffer, FieldPosition)`.
    ///
    /// # Errors
    ///
    /// The implementation's.
    fn format_icu_big_decimal_to(
        &self,
        number: &IcuBigDecimal,
        to_append_to: &mut Vec<u16>,
        pos: &mut FieldPosition,
    ) -> Result<()>;

    /// Returns a Long if possible (e.g., within the range [Long.MIN_VALUE,
    /// Long.MAX_VALUE] and with no decimals); otherwise, returns another
    /// type, such as a BigDecimal, BigInteger, or Double. Equivalent to the
    /// abstract `parse(String, ParsePosition)`.
    ///
    /// # Errors
    ///
    /// The implementation's.
    fn parse(&self, text: &[u16], parse_position: &mut ParsePosition) -> Result<Option<JavaNumber>>;

    /// `clone()`.
    fn clone_number_format(&self) -> Box<dyn NumberFormat>;

    /// Formats a CurrencyAmount. Equivalent to `format(CurrencyAmount,
    /// StringBuffer, FieldPosition)`.
    ///
    /// # Errors
    ///
    /// The errors of `setCurrency` and of the number formatting.
    fn format_currency_amount_to(
        &self,
        curr_amt: &CurrencyAmount,
        to_append_to: &mut Vec<u16>,
        pos: &mut FieldPosition,
    ) -> Result<()> {
        // Default implementation -- subclasses may override
        let save = self.get_currency();
        let curr = curr_amt.get_currency();
        let same = save.as_ref() == Some(&curr);
        if !same {
            self.set_currency(Some(curr))?;
        }
        let r = self.format_number_to(curr_amt.get_number(), to_append_to, pos);
        if !same {
            self.set_currency(save)?;
        }
        r
    }

    /// Parses text from the given string as a CurrencyAmount. Unlike the
    /// parse() method, this method will attempt to parse a generic currency
    /// name, searching for a match of this object's locale's currency
    /// display names, or for a 3-letter ISO currency code. Equivalent to
    /// `parseCurrency(CharSequence, ParsePosition)`.
    ///
    /// # Errors
    ///
    /// The errors of the parse and of `getEffectiveCurrency`.
    fn parse_currency(&self, text: &[u16], pos: &mut ParsePosition) -> Result<Option<CurrencyAmount>> {
        // Default implementation only -- subclasses should override
        let n = self.parse(text, pos)?;
        match n {
            None => Ok(None),
            Some(n) => Ok(Some(CurrencyAmount::new(Some(n), self.get_effective_currency()?)?)),
        }
    }

    /// Returns true if this format will parse numbers as integers only.
    /// Equivalent to `isParseIntegerOnly()`.
    fn is_parse_integer_only(&self) -> bool {
        lock(self.number_format_base()).parse_integer_only
    }

    /// Sets whether to ignore the fraction part of a number when parsing
    /// (defaults to false). Equivalent to `setParseIntegerOnly(boolean)`.
    ///
    /// # Errors
    ///
    /// The implementation's (a subclass refreshes its formatter).
    fn set_parse_integer_only(&self, value: bool) -> Result<()> {
        lock(self.number_format_base()).parse_integer_only = value;
        Ok(())
    }

    /// Sets whether strict parsing is in effect. Equivalent to
    /// `setParseStrict(boolean)`.
    ///
    /// # Errors
    ///
    /// The implementation's (a subclass refreshes its formatter).
    fn set_parse_strict(&self, value: bool) -> Result<()> {
        lock(self.number_format_base()).parse_strict = value;
        Ok(())
    }

    /// Returns whether strict parsing is in effect. Equivalent to
    /// `isParseStrict()`.
    fn is_parse_strict(&self) -> bool {
        lock(self.number_format_base()).parse_strict
    }

    /// Set a particular DisplayContext value in the formatter, such as
    /// CAPITALIZATION_FOR_STANDALONE. Equivalent to
    /// `setContext(DisplayContext)`.
    ///
    /// # Errors
    ///
    /// None here; an override's unchecked exceptions (such as
    /// `RuleBasedNumberFormat`'s locale data and break iterator errors) are
    /// its `Err` (crate convention, rmp #643).
    fn set_context(&self, context: DisplayContext) -> Result<()> {
        if context.r#type() == display_context::Type::Capitalization {
            lock(self.number_format_base()).capitalization_setting = Some(context);
        }
        Ok(())
    }

    /// Get the formatter's DisplayContext value for the specified
    /// DisplayContext.Type, such as CAPITALIZATION. Equivalent to
    /// `getContext(DisplayContext.Type)`.
    fn get_context(&self, type_: display_context::Type) -> DisplayContext {
        let base = lock(self.number_format_base());
        match base.capitalization_setting {
            Some(c) if type_ == display_context::Type::Capitalization => c,
            _ => DisplayContext::CapitalizationNone,
        }
    }

    /// Returns true if grouping is used in this format. Equivalent to
    /// `isGroupingUsed()`.
    fn is_grouping_used(&self) -> bool {
        lock(self.number_format_base()).grouping_used
    }

    /// Sets whether or not grouping will be used in this format. Equivalent
    /// to `setGroupingUsed(boolean)`.
    ///
    /// # Errors
    ///
    /// The implementation's (a subclass refreshes its formatter).
    fn set_grouping_used(&self, new_value: bool) -> Result<()> {
        lock(self.number_format_base()).grouping_used = new_value;
        Ok(())
    }

    /// Returns the maximum number of digits allowed in the integer portion
    /// of a number. Equivalent to `getMaximumIntegerDigits()`.
    fn get_maximum_integer_digits(&self) -> i32 {
        lock(self.number_format_base()).maximum_integer_digits
    }

    /// Sets the maximum number of digits allowed in the integer portion of a
    /// number. Equivalent to `setMaximumIntegerDigits(int)`.
    ///
    /// # Errors
    ///
    /// The implementation's (a subclass refreshes its formatter).
    fn set_maximum_integer_digits(&self, new_value: i32) -> Result<()> {
        let mut b = lock(self.number_format_base());
        b.maximum_integer_digits = 0.max(new_value);
        if b.minimum_integer_digits > b.maximum_integer_digits {
            b.minimum_integer_digits = b.maximum_integer_digits;
        }
        Ok(())
    }

    /// Returns the minimum number of digits allowed in the integer portion
    /// of a number. Equivalent to `getMinimumIntegerDigits()`.
    fn get_minimum_integer_digits(&self) -> i32 {
        lock(self.number_format_base()).minimum_integer_digits
    }

    /// Sets the minimum number of digits allowed in the integer portion of a
    /// number. Equivalent to `setMinimumIntegerDigits(int)`.
    ///
    /// # Errors
    ///
    /// The implementation's (a subclass refreshes its formatter).
    fn set_minimum_integer_digits(&self, new_value: i32) -> Result<()> {
        let mut b = lock(self.number_format_base());
        b.minimum_integer_digits = 0.max(new_value);
        if b.minimum_integer_digits > b.maximum_integer_digits {
            b.maximum_integer_digits = b.minimum_integer_digits;
        }
        Ok(())
    }

    /// Returns the maximum number of digits allowed in the fraction portion
    /// of a number. Equivalent to `getMaximumFractionDigits()`.
    fn get_maximum_fraction_digits(&self) -> i32 {
        lock(self.number_format_base()).maximum_fraction_digits
    }

    /// Sets the maximum number of digits allowed in the fraction portion of
    /// a number. Equivalent to `setMaximumFractionDigits(int)`.
    ///
    /// # Errors
    ///
    /// The implementation's (a subclass refreshes its formatter).
    fn set_maximum_fraction_digits(&self, new_value: i32) -> Result<()> {
        let mut b = lock(self.number_format_base());
        b.maximum_fraction_digits = 0.max(new_value);
        if b.maximum_fraction_digits < b.minimum_fraction_digits {
            b.minimum_fraction_digits = b.maximum_fraction_digits;
        }
        Ok(())
    }

    /// Returns the minimum number of digits allowed in the fraction portion
    /// of a number. Equivalent to `getMinimumFractionDigits()`.
    fn get_minimum_fraction_digits(&self) -> i32 {
        lock(self.number_format_base()).minimum_fraction_digits
    }

    /// Sets the minimum number of digits allowed in the fraction portion of
    /// a number. Equivalent to `setMinimumFractionDigits(int)`.
    ///
    /// # Errors
    ///
    /// The implementation's (a subclass refreshes its formatter).
    fn set_minimum_fraction_digits(&self, new_value: i32) -> Result<()> {
        let mut b = lock(self.number_format_base());
        b.minimum_fraction_digits = 0.max(new_value);
        if b.maximum_fraction_digits < b.minimum_fraction_digits {
            b.maximum_fraction_digits = b.minimum_fraction_digits;
        }
        Ok(())
    }

    /// Sets the Currency object used to display currency amounts. This
    /// takes effect immediately, if this format is a currency format. If
    /// this format is not a currency format, then the currency object is
    /// used if and when this object becomes a currency format. Equivalent to
    /// `setCurrency(Currency)`.
    ///
    /// # Errors
    ///
    /// The implementation's.
    fn set_currency(&self, the_currency: Option<Currency>) -> Result<()> {
        lock(self.number_format_base()).currency = the_currency;
        Ok(())
    }

    /// Returns the Currency object used to display currency amounts. This
    /// may be null. Equivalent to `getCurrency()`.
    fn get_currency(&self) -> Option<Currency> {
        lock(self.number_format_base()).currency.clone()
    }

    /// Returns the currency in effect for this formatter. Subclasses should
    /// override this method as needed. Unlike getCurrency(), this method
    /// should never return null. Equivalent to `getEffectiveCurrency()`.
    ///
    /// # Errors
    ///
    /// The errors of `Currency.getInstance(ULocale)`.
    fn get_effective_currency(&self) -> Result<Option<Currency>> {
        let c = self.get_currency();
        if c.is_some() {
            return Ok(c);
        }
        let uloc = self
            .get_locale(u_locale::VALID_LOCALE)
            .unwrap_or_else(|| ULocale::get_default_category(Category::Format));
        Currency::get_instance_ulocale(&uloc)
    }

    /// Returns the rounding mode used in this NumberFormat. The default
    /// implementation of this method in NumberFormat always throws
    /// UnsupportedOperationException. Equivalent to `getRoundingMode()`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException`.
    fn get_rounding_mode(&self) -> Result<i32> {
        Err(LuceneError::unsupported_operation(
            "getRoundingMode must be implemented by the subclass implementation.",
        ))
    }

    /// Set the rounding mode used in this NumberFormat. The default
    /// implementation of this method in NumberFormat always throws
    /// UnsupportedOperationException. Equivalent to `setRoundingMode(int)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException`.
    fn set_rounding_mode(&self, _rounding_mode: i32) -> Result<()> {
        Err(LuceneError::unsupported_operation(
            "setRoundingMode must be implemented by the subclass implementation.",
        ))
    }

    /// `UFormat.getLocale(ULocale.Type)`.
    fn get_locale(&self, type_: u_locale::Type) -> Option<ULocale> {
        lock(self.number_format_base()).uformat.get_locale(type_)
    }

    /// `UFormat.setLocale(ULocale, ULocale)` (package-private).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when exactly one of the two is `None`.
    fn set_locale(&self, valid: Option<ULocale>, actual: Option<ULocale>) -> Result<()> {
        lock(self.number_format_base()).uformat.set_locale(valid, actual)
    }

    /// Equivalent to `hashCode()`.
    fn hash_code(&self) -> i32 {
        lock(self.number_format_base()).hash_code()
    }

    /// Overrides equals. Two NumberFormats are equal if they are of the same
    /// class and the user-specified values for settings are equal.
    /// Equivalent to `equals(Object)`.
    fn java_equals(&self, obj: &dyn NumberFormat) -> bool {
        if std::ptr::addr_eq(self as *const Self, obj as *const dyn NumberFormat) {
            return true;
        }
        if Format::as_any(self).type_id() != Format::as_any(obj).type_id() {
            return false;
        }
        let me = lock(self.number_format_base()).clone();
        let other = lock(obj.number_format_base()).clone();
        me.fields_equal(&other)
    }

    /// `format(Object, StringBuffer, FieldPosition)` for a `Number`:
    /// `Long`, `BigInteger`, `java.math.BigDecimal` and
    /// `com.ibm.icu.math.BigDecimal` keep their overloads, every other
    /// `Number` is formatted as its `doubleValue()`.
    ///
    /// # Errors
    ///
    /// The errors of the overload.
    fn format_number_to(&self, number: &JavaNumber, to_append_to: &mut Vec<u16>, pos: &mut FieldPosition) -> Result<()> {
        match number {
            JavaNumber::Long(l) => self.format_long_to(*l, to_append_to, pos),
            JavaNumber::BigInteger(b) => self.format_big_integer_to(b, to_append_to, pos),
            JavaNumber::BigDecimal(b) => self.format_big_decimal_to(b, to_append_to, pos),
            JavaNumber::IcuBigDecimal(b) => self.format_icu_big_decimal_to(b, to_append_to, pos),
            other => self.format_double_to(other.double_value(), to_append_to, pos),
        }
    }

    /// Formats a number and appends the resulting text to the given string
    /// buffer. Equivalent to `format(Object, StringBuffer, FieldPosition)`:
    /// `obj` may be a [`JavaNumber`], an `i64` (`Long`), an `f64`
    /// (`Double`), an `i32` (`Integer`), an `f32` (`Float`), a
    /// [`BigInteger`], a [`BigDecimal`], an [`IcuBigDecimal`] or a
    /// [`CurrencyAmount`].
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Cannot format given Object as a Number")`
    /// for any other object; the errors of the overload.
    fn format_any_to(&self, number: &dyn Any, to_append_to: &mut Vec<u16>, pos: &mut FieldPosition) -> Result<()> {
        if let Some(n) = number.downcast_ref::<JavaNumber>() {
            self.format_number_to(n, to_append_to, pos)
        } else if let Some(l) = number.downcast_ref::<i64>() {
            self.format_long_to(*l, to_append_to, pos)
        } else if let Some(b) = number.downcast_ref::<BigInteger>() {
            self.format_big_integer_to(b, to_append_to, pos)
        } else if let Some(b) = number.downcast_ref::<BigDecimal>() {
            self.format_big_decimal_to(b, to_append_to, pos)
        } else if let Some(b) = number.downcast_ref::<IcuBigDecimal>() {
            self.format_icu_big_decimal_to(b, to_append_to, pos)
        } else if let Some(c) = number.downcast_ref::<CurrencyAmount>() {
            self.format_currency_amount_to(c, to_append_to, pos)
        } else if let Some(d) = number.downcast_ref::<f64>() {
            self.format_double_to(*d, to_append_to, pos)
        } else if let Some(i) = number.downcast_ref::<i32>() {
            self.format_double_to(f64::from(*i), to_append_to, pos)
        } else if let Some(f) = number.downcast_ref::<f32>() {
            self.format_double_to(f64::from(*f), to_append_to, pos)
        } else {
            Err(LuceneError::illegal_argument("Cannot format given Object as a Number"))
        }
    }

    /// Specialization of format. Equivalent to `format(double)`.
    ///
    /// # Errors
    ///
    /// The errors of the formatting.
    fn format_f64(&self, number: f64) -> Result<Vec<u16>> {
        let mut buf = Vec::new();
        self.format_double_to(number, &mut buf, &mut FieldPosition::new(0))?;
        Ok(buf)
    }

    /// Specialization of format. Equivalent to `format(long)`.
    ///
    /// # Errors
    ///
    /// The errors of the formatting.
    fn format_i64(&self, number: i64) -> Result<Vec<u16>> {
        let mut buf = Vec::with_capacity(19);
        self.format_long_to(number, &mut buf, &mut FieldPosition::new(0))?;
        Ok(buf)
    }

    /// Convenience method to format a BigInteger. Equivalent to
    /// `format(BigInteger)`.
    ///
    /// # Errors
    ///
    /// The errors of the formatting.
    fn format_big_integer(&self, number: &BigInteger) -> Result<Vec<u16>> {
        let mut buf = Vec::new();
        self.format_big_integer_to(number, &mut buf, &mut FieldPosition::new(0))?;
        Ok(buf)
    }

    /// Convenience method to format a BigDecimal. Equivalent to
    /// `format(java.math.BigDecimal)`.
    ///
    /// # Errors
    ///
    /// The errors of the formatting.
    fn format_big_decimal(&self, number: &BigDecimal) -> Result<Vec<u16>> {
        let mut buf = Vec::new();
        self.format_big_decimal_to(number, &mut buf, &mut FieldPosition::new(0))?;
        Ok(buf)
    }

    /// Convenience method to format an ICU BigDecimal. Equivalent to
    /// `format(com.ibm.icu.math.BigDecimal)`.
    ///
    /// # Errors
    ///
    /// The errors of the formatting.
    fn format_icu_big_decimal(&self, number: &IcuBigDecimal) -> Result<Vec<u16>> {
        let mut buf = Vec::new();
        self.format_icu_big_decimal_to(number, &mut buf, &mut FieldPosition::new(0))?;
        Ok(buf)
    }

    /// Convenience method to format a CurrencyAmount. Equivalent to
    /// `format(CurrencyAmount)`.
    ///
    /// # Errors
    ///
    /// The errors of the formatting.
    fn format_currency_amount(&self, curr_amt: &CurrencyAmount) -> Result<Vec<u16>> {
        let mut buf = Vec::new();
        self.format_currency_amount_to(curr_amt, &mut buf, &mut FieldPosition::new(0))?;
        Ok(buf)
    }

    /// Parses text from the beginning of the given string to produce a
    /// number. Equivalent to `parse(String)`.
    ///
    /// # Errors
    ///
    /// `ParseException("Unparseable number: \"<text>\"", errorIndex)` when
    /// nothing was parsed; the errors of the parse.
    fn parse_text(&self, text: &[u16]) -> Result<Option<JavaNumber>> {
        let mut parse_position = ParsePosition::new(0);
        let result = self.parse(text, &mut parse_position)?;
        if parse_position.get_index() == 0 {
            return Err(rucene::util::extra::ParseException::error(
                format!("Unparseable number: \"{}\"", String::from_utf16_lossy(text)),
                parse_position.get_error_index(),
            ));
        }
        Ok(result)
    }
}

/// Locks a state mutex, ignoring poisoning (Java has none).
pub(crate) fn super_set_context(base: &Mutex<NumberFormatBase>, context: DisplayContext) {
    if context.r#type() == display_context::Type::Capitalization {
        lock(base).capitalization_setting = Some(context);
    }
}

pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// A NumberFormatFactory is used to register new number formats. The factory
/// should be able to create any of the predefined formats for each locale it
/// supports. When registered, the locales it supports extend or override
/// the locales already supported by ICU.
///
/// Equivalent to the abstract class `NumberFormat.NumberFormatFactory`.
pub trait NumberFormatFactory: Send + Sync {
    /// Returns true if this factory is visible. Default is true. If not
    /// visible, the locales supported by this factory will not be listed by
    /// getAvailableLocales. Equivalent to `visible()`.
    fn visible(&self) -> bool {
        true
    }

    /// Returns an immutable collection of the locale names directly
    /// supported by this factory. Equivalent to `getSupportedLocaleNames()`.
    fn get_supported_locale_names(&self) -> HashSet<String>;

    /// Returns a number format of the appropriate type. If the locale is not
    /// supported, return null. If the locale is supported, but the type is
    /// not provided by this service, return null. Otherwise return an
    /// appropriate instance of NumberFormat. Equivalent to
    /// `createFormat(ULocale, int)` (which defaults to
    /// `createFormat(loc.toLocale(), formatType)`).
    ///
    /// # Errors
    ///
    /// The implementation's.
    fn create_format(&self, loc: &ULocale, format_type: i32) -> Result<Option<Box<dyn NumberFormat>>> {
        self.create_format_locale(&loc.to_locale(), format_type)
    }

    /// Equivalent to `createFormat(Locale, int)` (which defaults to
    /// `createFormat(ULocale.forLocale(loc), formatType)`). An
    /// implementation overrides at least one of the two, as in Java, where
    /// overriding neither recurses without end.
    ///
    /// # Errors
    ///
    /// The implementation's.
    fn create_format_locale(&self, loc: &Locale, format_type: i32) -> Result<Option<Box<dyn NumberFormat>>> {
        let uloc = ULocale::for_locale(Some(loc))?.unwrap_or_else(ULocale::root);
        self.create_format(&uloc, format_type)
    }
}

/// `NumberFormatFactory.FORMAT_NUMBER`.
pub const FORMAT_NUMBER: i32 = NUMBERSTYLE;
/// `NumberFormatFactory.FORMAT_CURRENCY`.
pub const FORMAT_CURRENCY: i32 = CURRENCYSTYLE;
/// `NumberFormatFactory.FORMAT_PERCENT`.
pub const FORMAT_PERCENT: i32 = PERCENTSTYLE;
/// `NumberFormatFactory.FORMAT_SCIENTIFIC`.
pub const FORMAT_SCIENTIFIC: i32 = SCIENTIFICSTYLE;
/// `NumberFormatFactory.FORMAT_INTEGER`.
pub const FORMAT_INTEGER: i32 = INTEGERSTYLE;

/// A NumberFormatFactory that supports a single locale. It can be visible
/// or invisible.
///
/// Equivalent to the state of the abstract class
/// `NumberFormat.SimpleNumberFormatFactory`: an implementor of
/// [`NumberFormatFactory`] holds one and answers `visible()` and
/// `getSupportedLocaleNames()` (final in Java) from it.
#[derive(Debug, Clone)]
pub struct SimpleNumberFormatFactory {
    locale_names: HashSet<String>,
    visible: bool,
}

impl SimpleNumberFormatFactory {
    /// Constructs a SimpleNumberFormatFactory with the given locale.
    /// Equivalent to `SimpleNumberFormatFactory(Locale)`.
    ///
    /// # Errors
    ///
    /// The errors of `ULocale.forLocale`.
    pub fn from_locale(locale: &Locale) -> Result<Self> {
        Self::from_locale_visible(locale, true)
    }

    /// Equivalent to `SimpleNumberFormatFactory(Locale, boolean)`.
    ///
    /// # Errors
    ///
    /// The errors of `ULocale.forLocale`.
    pub fn from_locale_visible(locale: &Locale, visible: bool) -> Result<Self> {
        let uloc = ULocale::for_locale(Some(locale))?.unwrap_or_else(ULocale::root);
        Ok(Self::new_visible(&uloc, visible))
    }

    /// Equivalent to `SimpleNumberFormatFactory(ULocale)`.
    pub fn new(locale: &ULocale) -> Self {
        Self::new_visible(locale, true)
    }

    /// Equivalent to `SimpleNumberFormatFactory(ULocale, boolean)`.
    pub fn new_visible(locale: &ULocale, visible: bool) -> Self {
        let mut locale_names = HashSet::new();
        locale_names.insert(locale.get_base_name());
        SimpleNumberFormatFactory { locale_names, visible }
    }

    /// Returns true if this factory is visible. Equivalent to `visible()`.
    pub fn visible(&self) -> bool {
        self.visible
    }

    /// Returns a collection of the locale names directly supported by this
    /// factory. Equivalent to `getSupportedLocaleNames()`.
    pub fn get_supported_locale_names(&self) -> HashSet<String> {
        self.locale_names.clone()
    }
}

/// A registry key returned by `registerFactory` (Java `Object`).
pub type RegistryKey = Arc<dyn crate::r#impl::icu_service::Factory>;

/// Equivalent to the package-private abstract class
/// `NumberFormat.NumberFormatShim`.
pub(crate) trait NumberFormatShim: Send + Sync {
    /// Equivalent to `getAvailableLocales()`.
    fn get_available_locales(&self) -> Result<Vec<Locale>>;
    /// Equivalent to `getAvailableULocales()`.
    fn get_available_u_locales(&self) -> Result<Vec<ULocale>>;
    /// Equivalent to `registerFactory(NumberFormatFactory)`.
    fn register_factory(&self, f: Arc<dyn NumberFormatFactory>) -> Result<RegistryKey>;
    /// Equivalent to `unregister(Object)`.
    fn unregister(&self, k: &RegistryKey) -> bool;
    /// Equivalent to `createInstance(ULocale, int)`.
    fn create_instance(&self, l: &ULocale, k: i32) -> Result<Box<dyn NumberFormat>>;
}

/// `shim`: set by the first `getShim()`.
static SHIM: Mutex<Option<Arc<dyn NumberFormatShim>>> = Mutex::new(None);

/// `getShim()`: the `NumberFormatServiceShim` Java loads by
/// `Class.forName("com.ibm.icu.text.NumberFormatServiceShim")`.
fn get_shim() -> Arc<dyn NumberFormatShim> {
    // Note: this instantiation is safe on loose-memory-model configurations
    // despite lack of synchronization, since the shim instance has no state--
    // it's all in the class init.  The worst problem is we might instantiate
    // two shim instances, but they'll share the same state so that's ok.
    let mut shim = lock(&SHIM);
    shim.get_or_insert_with(|| Arc::new(NumberFormatServiceShim)).clone()
}

/// `shim == null`.
fn shim_is_null() -> bool {
    lock(&SHIM).is_none()
}

/// Returns the list of Locales for which NumberFormats are available.
/// Equivalent to `getAvailableLocales()`.
///
/// # Errors
///
/// The errors of the locale enumeration.
pub fn get_available_locales() -> Result<Vec<Locale>> {
    if shim_is_null() {
        return Ok((*ICUResourceBundle::get_available_locales()?).clone());
    }
    get_shim().get_available_locales()
}

/// Returns the list of Locales for which NumberFormats are available.
/// Equivalent to `getAvailableULocales()`.
///
/// # Errors
///
/// The errors of the locale enumeration.
pub fn get_available_u_locales() -> Result<Vec<ULocale>> {
    if shim_is_null() {
        return Ok(ICUResourceBundle::get_available_u_locales()?
            .unwrap_or_else(|| panic!("java.lang.NullPointerException")));
    }
    get_shim().get_available_u_locales()
}

/// Registers a new NumberFormatFactory. The factory is adopted by the
/// service and must not be modified. The returned object is a key that can
/// be used to unregister this factory. Equivalent to
/// `registerFactory(NumberFormatFactory)`.
///
/// # Errors
///
/// `IllegalArgumentException("factory must not be null")`; the errors of the
/// service.
pub fn register_factory(factory: Option<Arc<dyn NumberFormatFactory>>) -> Result<RegistryKey> {
    let Some(factory) = factory else {
        return Err(LuceneError::illegal_argument("factory must not be null"));
    };
    get_shim().register_factory(factory)
}

/// Unregisters the factory or instance associated with this key (obtained
/// from registerInstance or registerFactory). Equivalent to
/// `unregister(Object)`.
///
/// # Errors
///
/// `IllegalArgumentException("registryKey must not be null")`.
pub fn unregister(registry_key: Option<&RegistryKey>) -> Result<bool> {
    let Some(registry_key) = registry_key else {
        return Err(LuceneError::illegal_argument("registryKey must not be null"));
    };
    if shim_is_null() {
        return Ok(false);
    }
    Ok(get_shim().unregister(registry_key))
}

/// Returns the default number format for the current default `FORMAT`
/// locale. Equivalent to `getInstance()`.
///
/// # Errors
///
/// As [`get_instance_style`].
pub fn get_instance() -> Result<Box<dyn NumberFormat>> {
    get_instance_style(&ULocale::get_default_category(Category::Format), NUMBERSTYLE)
}

/// Returns the default number format for the specified locale. Equivalent
/// to `getInstance(Locale)`.
///
/// # Errors
///
/// As [`get_instance_style`].
pub fn get_instance_locale(in_locale: &Locale) -> Result<Box<dyn NumberFormat>> {
    get_instance_style(&for_locale(in_locale)?, NUMBERSTYLE)
}

/// Returns the default number format for the specified locale. Equivalent
/// to `getInstance(ULocale)`.
///
/// # Errors
///
/// As [`get_instance_style`].
pub fn get_instance_ulocale(in_locale: &ULocale) -> Result<Box<dyn NumberFormat>> {
    get_instance_style(in_locale, NUMBERSTYLE)
}

/// Returns a specific style number format for default locale. Equivalent to
/// `getInstance(int)`.
///
/// # Errors
///
/// As [`get_instance_style`].
pub fn get_instance_default_style(style: i32) -> Result<Box<dyn NumberFormat>> {
    get_instance_style(&ULocale::get_default_category(Category::Format), style)
}

/// Returns a specific style number format for a specific locale. Equivalent
/// to `getInstance(Locale, int)`.
///
/// # Errors
///
/// As [`get_instance_style`].
pub fn get_instance_locale_style(in_locale: &Locale, style: i32) -> Result<Box<dyn NumberFormat>> {
    get_instance_style(&for_locale(in_locale)?, style)
}

/// `ULocale.forLocale(Locale)`.
fn for_locale(locale: &Locale) -> Result<ULocale> {
    Ok(ULocale::for_locale(Some(locale))?.unwrap_or_else(ULocale::root))
}

/// Returns a general-purpose number format for the current default
/// `FORMAT` locale. Equivalent to `getNumberInstance()`.
///
/// # Errors
///
/// As [`get_instance_style`].
pub fn get_number_instance() -> Result<Box<dyn NumberFormat>> {
    get_instance_style(&ULocale::get_default_category(Category::Format), NUMBERSTYLE)
}

/// Equivalent to `getNumberInstance(Locale)`.
///
/// # Errors
///
/// As [`get_instance_style`].
pub fn get_number_instance_locale(in_locale: &Locale) -> Result<Box<dyn NumberFormat>> {
    get_instance_style(&for_locale(in_locale)?, NUMBERSTYLE)
}

/// Equivalent to `getNumberInstance(ULocale)`.
///
/// # Errors
///
/// As [`get_instance_style`].
pub fn get_number_instance_ulocale(in_locale: &ULocale) -> Result<Box<dyn NumberFormat>> {
    get_instance_style(in_locale, NUMBERSTYLE)
}

/// Returns an integer number format for the current default `FORMAT`
/// locale. Equivalent to `getIntegerInstance()`.
///
/// # Errors
///
/// As [`get_instance_style`].
pub fn get_integer_instance() -> Result<Box<dyn NumberFormat>> {
    get_instance_style(&ULocale::get_default_category(Category::Format), INTEGERSTYLE)
}

/// Equivalent to `getIntegerInstance(Locale)`.
///
/// # Errors
///
/// As [`get_instance_style`].
pub fn get_integer_instance_locale(in_locale: &Locale) -> Result<Box<dyn NumberFormat>> {
    get_instance_style(&for_locale(in_locale)?, INTEGERSTYLE)
}

/// Equivalent to `getIntegerInstance(ULocale)`.
///
/// # Errors
///
/// As [`get_instance_style`].
pub fn get_integer_instance_ulocale(in_locale: &ULocale) -> Result<Box<dyn NumberFormat>> {
    get_instance_style(in_locale, INTEGERSTYLE)
}

/// Returns a currency format for the current default `FORMAT` locale.
/// Equivalent to `getCurrencyInstance()`.
///
/// # Errors
///
/// As [`get_instance_style`].
pub fn get_currency_instance() -> Result<Box<dyn NumberFormat>> {
    get_instance_style(&ULocale::get_default_category(Category::Format), CURRENCYSTYLE)
}

/// Equivalent to `getCurrencyInstance(Locale)`.
///
/// # Errors
///
/// As [`get_instance_style`].
pub fn get_currency_instance_locale(in_locale: &Locale) -> Result<Box<dyn NumberFormat>> {
    get_instance_style(&for_locale(in_locale)?, CURRENCYSTYLE)
}

/// Equivalent to `getCurrencyInstance(ULocale)`.
///
/// # Errors
///
/// As [`get_instance_style`].
pub fn get_currency_instance_ulocale(in_locale: &ULocale) -> Result<Box<dyn NumberFormat>> {
    get_instance_style(in_locale, CURRENCYSTYLE)
}

/// Returns a percentage format for the current default `FORMAT` locale.
/// Equivalent to `getPercentInstance()`.
///
/// # Errors
///
/// As [`get_instance_style`].
pub fn get_percent_instance() -> Result<Box<dyn NumberFormat>> {
    get_instance_style(&ULocale::get_default_category(Category::Format), PERCENTSTYLE)
}

/// Equivalent to `getPercentInstance(Locale)`.
///
/// # Errors
///
/// As [`get_instance_style`].
pub fn get_percent_instance_locale(in_locale: &Locale) -> Result<Box<dyn NumberFormat>> {
    get_instance_style(&for_locale(in_locale)?, PERCENTSTYLE)
}

/// Equivalent to `getPercentInstance(ULocale)`.
///
/// # Errors
///
/// As [`get_instance_style`].
pub fn get_percent_instance_ulocale(in_locale: &ULocale) -> Result<Box<dyn NumberFormat>> {
    get_instance_style(in_locale, PERCENTSTYLE)
}

/// Returns a scientific format for the current default `FORMAT` locale.
/// Equivalent to `getScientificInstance()`.
///
/// # Errors
///
/// As [`get_instance_style`].
pub fn get_scientific_instance() -> Result<Box<dyn NumberFormat>> {
    get_instance_style(&ULocale::get_default_category(Category::Format), SCIENTIFICSTYLE)
}

/// Equivalent to `getScientificInstance(Locale)`.
///
/// # Errors
///
/// As [`get_instance_style`].
pub fn get_scientific_instance_locale(in_locale: &Locale) -> Result<Box<dyn NumberFormat>> {
    get_instance_style(&for_locale(in_locale)?, SCIENTIFICSTYLE)
}

/// Equivalent to `getScientificInstance(ULocale)`.
///
/// # Errors
///
/// As [`get_instance_style`].
pub fn get_scientific_instance_ulocale(in_locale: &ULocale) -> Result<Box<dyn NumberFormat>> {
    get_instance_style(in_locale, SCIENTIFICSTYLE)
}

/// Returns a specific style number format for a specific locale. Equivalent
/// to `getInstance(ULocale, int)`.
///
/// # Errors
///
/// `IllegalArgumentException("choice should be from NUMBERSTYLE to
/// STANDARDCURRENCYSTYLE")`; the errors of the service.
pub fn get_instance_style(desired_locale: &ULocale, choice: i32) -> Result<Box<dyn NumberFormat>> {
    if !(NUMBERSTYLE..=STANDARDCURRENCYSTYLE).contains(&choice) {
        return Err(LuceneError::illegal_argument("choice should be from NUMBERSTYLE to STANDARDCURRENCYSTYLE"));
    }
    get_shim().create_instance(desired_locale, choice)
}

/// Equivalent to the package-private static `createInstance(ULocale, int)`;
/// `None` for Java's `null` (no numbering system).
///
/// # Errors
///
/// The errors of the locale data and of the format construction.
pub(crate) fn create_instance(desired_locale: &ULocale, choice: i32) -> Result<Option<Box<dyn NumberFormat>>> {
    // If the choice is PLURALCURRENCYSTYLE, the pattern is not a single
    // pattern, it is a pattern set, so we do not need to get them here.
    // If the choice is ISOCURRENCYSTYLE, the pattern is the currrency
    // pattern in the locale but by replacing the single currency sign
    // with double currency sign.
    let mut pattern = get_pattern(desired_locale, choice)?;
    let symbols = DecimalFormatSymbols::for_ulocale(desired_locale)?;

    // Here we assume that the locale passed in is in the canonical
    // form, e.g: pt_PT_@currency=PTE not pt_PT_PREEURO
    // This style wont work for currency plural format.
    // For currency plural format, the pattern is get from
    // the locale (from CurrencyUnitPatterns) without override.
    if choice == CURRENCYSTYLE
        || choice == ISOCURRENCYSTYLE
        || choice == ACCOUNTINGCURRENCYSTYLE
        || choice == CASHCURRENCYSTYLE
        || choice == STANDARDCURRENCYSTYLE
    {
        if let Some(temp) = symbols.get_currency_pattern() {
            pattern = temp.to_vec();
        }
    }

    // replace single currency sign in the pattern with double currency sign
    // if the choice is ISOCURRENCYSTYLE.
    if choice == ISOCURRENCYSTYLE {
        let mut replaced = Vec::with_capacity(pattern.len() + 1);
        for &u in &pattern {
            if u == 0xA4 {
                replaced.extend_from_slice(&DOUBLE_CURRENCY_STR);
            } else {
                replaced.push(u);
            }
        }
        pattern = replaced;
    }

    // Get the numbering system
    let ns = NumberingSystem::get_instance_for(desired_locale)?;

    let format: Box<dyn NumberFormat> = if ns.is_algorithmic() {
        let mut desired_rules_type = rule_based_number_format::NUMBERING_SYSTEM;

        let ns_desc = ns.get_description();
        let first_slash = crate::jdk::index_of_unit(ns_desc, u16::from(b'/'), 0);
        let last_slash = crate::jdk::last_index_of_unit(ns_desc, u16::from(b'/'), ns_desc.len() as i32 - 1);

        let (ns_loc, ns_rule_set_name) = if last_slash > first_slash {
            let ns_loc_id = String::from_utf16_lossy(&ns_desc[..first_slash as usize]);
            let ns_rule_set_group = &ns_desc[first_slash as usize + 1..last_slash as usize];
            let ns_rule_set_name = ns_desc[last_slash as usize + 1..].to_vec();

            let ns_loc = ULocale::new(&ns_loc_id)?;
            if ns_rule_set_group == units("SpelloutRules").as_slice() {
                desired_rules_type = rule_based_number_format::SPELLOUT;
            }
            (ns_loc, ns_rule_set_name)
        } else {
            (desired_locale.clone(), ns_desc.to_vec())
        };

        let r = RuleBasedNumberFormat::new(&ns_loc, desired_rules_type)?;
        r.set_default_rule_set(Some(&ns_rule_set_name))?;
        Box::new(r)
    } else {
        let f = DecimalFormat::with_style(&pattern, &symbols, choice)?;
        // System.out.println("loc: " + desiredLocale + " choice: " + choice + " pat: " + pattern + " sym: " + symbols + " result: " + format);

        /*Bug 4408066
         Add codes for the new method getIntegerInstance() [Richard/GCL]
        */
        // TODO: revisit this -- this is almost certainly not the way we want
        // to do this.  aliu 1/6/2004
        if choice == INTEGERSTYLE {
            f.set_maximum_fraction_digits(0)?;
            f.set_decimal_separator_always_shown(false)?;
            f.set_parse_integer_only(true)?;
        }
        if choice == CASHCURRENCYSTYLE {
            f.set_currency_usage(Some(CurrencyUsage::Cash))?;
        }
        if choice == PLURALCURRENCYSTYLE {
            f.set_currency_plural_info(Some(&CurrencyPluralInfo::get_instance_ulocale(desired_locale)?))?;
        }
        Box::new(f)
    };

    // TODO: the actual locale of the *pattern* may differ from that
    // for the *symbols*.  For now, we use the data for the symbols.
    // Get the actual locale, using the symbols as a proxy.
    let valid = symbols.get_locale_type(u_locale::VALID_LOCALE).cloned();
    let actual = symbols.get_locale_type(u_locale::ACTUAL_LOCALE).cloned();
    format.set_locale(valid, actual)?;

    Ok(Some(format))
}

/// Returns the pattern for the provided locale and choice. Equivalent to
/// the deprecated protected static `getPattern(Locale, int)`.
///
/// # Errors
///
/// As [`get_pattern`].
pub fn get_pattern_locale(for_locale_: &Locale, choice: i32) -> Result<Vec<u16>> {
    get_pattern(&for_locale(for_locale_)?, choice)
}

/// Returns the pattern for the provided locale and choice. Equivalent to the
/// protected static `getPattern(ULocale, int)`.
///
/// # Errors
///
/// As [`get_pattern_for_style`].
pub fn get_pattern(for_locale: &ULocale, choice: i32) -> Result<Vec<u16>> {
    get_pattern_for_style(for_locale, choice)
}

/// Returns the pattern for the provided locale and choice. Equivalent to the
/// internal `getPatternForStyle(ULocale, int)`.
///
/// # Errors
///
/// The errors of the numbering system and of the resource lookup.
pub fn get_pattern_for_style(for_locale: &ULocale, choice: i32) -> Result<Vec<u16>> {
    let ns = NumberingSystem::get_instance_for(for_locale)?;
    let ns_name = ns.get_name().unwrap_or_else(|| panic!("java.lang.NullPointerException: nsName"));
    get_pattern_for_style_and_numbering_system(for_locale, ns_name, choice)
}

/// Returns the pattern for the provided locale, numbering system, and
/// choice. Equivalent to the internal
/// `getPatternForStyleAndNumberingSystem(ULocale, String, int)`.
///
/// # Errors
///
/// The errors of the resource lookup (`MissingResourceException` when even
/// the `latn` pattern is absent).
pub fn get_pattern_for_style_and_numbering_system(for_locale: &ULocale, ns_name: &str, choice: i32) -> Result<Vec<u16>> {
    /* for ISOCURRENCYSTYLE and PLURALCURRENCYSTYLE,
     * the pattern is the same as the pattern of CURRENCYSTYLE
     * but by replacing the single currency sign with
     * double currency sign or triple currency sign.
     */
    let pattern_key = match choice {
        NUMBERSTYLE | INTEGERSTYLE | PLURALCURRENCYSTYLE => "decimalFormat",
        CURRENCYSTYLE => {
            let cf_key_value = for_locale.get_keyword_value("cf");
            if cf_key_value.as_deref() == Some("account") {
                "accountingFormat"
            } else {
                "currencyFormat"
            }
        }
        CASHCURRENCYSTYLE | ISOCURRENCYSTYLE | STANDARDCURRENCYSTYLE => "currencyFormat",
        PERCENTSTYLE => "percentFormat",
        SCIENTIFICSTYLE => "scientificFormat",
        ACCOUNTINGCURRENCYSTYLE => "accountingFormat",
        _ => {
            debug_assert!(false);
            "decimalFormat"
        }
    };

    let rb = UResourceBundle::get_bundle_instance_for(Some(ICUData::ICU_BASE_NAME), Some(for_locale))?;
    let rb = rb.as_icu()?.clone();

    let result = rb.find_string_with_fallback(&format!("NumberElements/{ns_name}/patterns/{pattern_key}"))?;
    match result {
        Some(r) => Ok(r),
        None => rb.get_string_with_fallback(&format!("NumberElements/latn/patterns/{pattern_key}")),
    }
}

/// `MissingResourceException("Unable to construct NumberFormat", "", "")`.
pub(crate) fn unable_to_construct() -> LuceneError {
    MissingResourceException::new("Unable to construct NumberFormat", "", "").error()
}
