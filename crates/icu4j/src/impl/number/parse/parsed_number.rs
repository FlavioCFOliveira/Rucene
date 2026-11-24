// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.parse.ParsedNumber`.

use std::cmp::Ordering;

use rucene::error::Result;

use crate::r#impl::number::decimal_quantity::DecimalQuantity;
use crate::r#impl::number::decimal_quantity_abstract_bcd::DecimalQuantityAbstractBcd;
use crate::r#impl::number::decimal_quantity_dual_storage_bcd::{
    DecimalQuantity_DualStorageBCD, JavaNumber,
};
use crate::r#impl::number::parse::parsing_utils::ParsingUtils;
use crate::r#impl::string_segment::StringSegment;

/// The quantity is negative.
pub const FLAG_NEGATIVE: i32 = 0x0001;
/// The number was parsed as a percentage.
pub const FLAG_PERCENT: i32 = 0x0002;
/// The number was parsed as a permille.
pub const FLAG_PERMILLE: i32 = 0x0004;
/// The number has an exponent.
pub const FLAG_HAS_EXPONENT: i32 = 0x0008;
/// The number had a decimal separator.
pub const FLAG_HAS_DECIMAL_SEPARATOR: i32 = 0x0020;
/// NaN was parsed.
pub const FLAG_NAN: i32 = 0x0040;
/// Infinity was parsed.
pub const FLAG_INFINITY: i32 = 0x0080;
/// The parse failed.
pub const FLAG_FAIL: i32 = 0x0100;

/// The result of a number parse.
///
/// Equivalent to `com.ibm.icu.impl.number.parse.ParsedNumber`; the `null`s
/// of its fields are `None`.
#[derive(Debug, Clone, Default)]
pub struct ParsedNumber {
    /// `quantity`.
    pub quantity: Option<DecimalQuantity_DualStorageBCD>,
    /// The index of the last char consumed during parsing. If parsing
    /// started at index 0, this is equal to the number of chars consumed.
    /// This is NOT necessarily the same as the StringSegment offset; "weak"
    /// chars, like whitespace, change the offset, but the charsConsumed is
    /// not touched until a "strong" char is encountered.
    pub char_end: i32,
    /// Boolean flags (see constants above).
    pub flags: i32,
    /// The pattern string corresponding to the prefix that got consumed.
    pub prefix: Option<Vec<u16>>,
    /// The pattern string corresponding to the suffix that got consumed.
    pub suffix: Option<Vec<u16>>,
    /// The currency that got consumed.
    pub currency_code: Option<String>,
}

impl ParsedNumber {
    /// Equivalent to `ParsedNumber()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Equivalent to `COMPARATOR.compare(o1, o2)`: `o1.charEnd -
    /// o2.charEnd`.
    pub fn compare(o1: &ParsedNumber, o2: &ParsedNumber) -> Ordering {
        (o1.char_end - o2.char_end).cmp(&0)
    }

    /// Clears the data from this ParsedNumber, in effect failing the current
    /// parse. Equivalent to `clear()`.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Equivalent to `copyFrom(ParsedNumber)` (the quantity is copied).
    pub fn copy_from(&mut self, other: &ParsedNumber) {
        self.quantity = other
            .quantity
            .as_ref()
            .map(DecimalQuantity_DualStorageBCD::from_other);
        self.char_end = other.char_end;
        self.flags = other.flags;
        self.prefix = other.prefix.clone();
        self.suffix = other.suffix.clone();
        self.currency_code = other.currency_code.clone();
    }

    /// Call this method to register that a "strong" char was consumed. This
    /// should be done after calling `segment.adjustOffset` or
    /// `segment.adjustOffsetByCodePoint`. Equivalent to
    /// `setCharsConsumed(StringSegment)`.
    pub fn set_chars_consumed(&mut self, segment: &StringSegment) {
        self.char_end = segment.get_offset();
    }

    /// Method called by the NumberParserImpl at the end of a successful
    /// parse. Equivalent to `postProcess()`.
    pub fn post_process(&mut self) {
        if let Some(quantity) = self.quantity.as_mut() {
            if 0 != (self.flags & FLAG_NEGATIVE) {
                quantity.negate();
            }
        }
    }

    /// Returns whether this the parse was successful. To be successful, at
    /// least one char must have been consumed, and the failure flag must not
    /// be set. Equivalent to `success()`.
    pub fn success(&self) -> bool {
        self.char_end > 0 && 0 == (self.flags & FLAG_FAIL)
    }

    /// Equivalent to `seenNumber()`.
    pub fn seen_number(&self) -> bool {
        self.quantity.is_some() || 0 != (self.flags & FLAG_NAN) || 0 != (self.flags & FLAG_INFINITY)
    }

    /// Equivalent to `getNumber()`.
    ///
    /// # Errors
    ///
    /// As [`get_number_flags`](Self::get_number_flags).
    pub fn get_number(&mut self) -> Result<JavaNumber> {
        self.get_number_flags(0)
    }

    /// Equivalent to `getNumber(int)`: `Double` for NaN, infinities and
    /// negative zero, `Long` when it fits, else `java.math.BigDecimal`.
    ///
    /// # Errors
    ///
    /// The errors of `DecimalQuantity.toBigDecimal`.
    pub fn get_number_flags(&mut self, parse_flags: i32) -> Result<JavaNumber> {
        let saw_nan = 0 != (self.flags & FLAG_NAN);
        let saw_infinity = 0 != (self.flags & FLAG_INFINITY);
        let force_big_decimal = 0 != (parse_flags & ParsingUtils::PARSE_FLAG_FORCE_BIG_DECIMAL);
        let integer_only = 0 != (parse_flags & ParsingUtils::PARSE_FLAG_INTEGER_ONLY);

        // Check for NaN, infinity, and -0.0
        if saw_nan {
            return Ok(JavaNumber::Double(f64::NAN));
        }
        if saw_infinity {
            if 0 != (self.flags & FLAG_NEGATIVE) {
                return Ok(JavaNumber::Double(f64::NEG_INFINITY));
            } else {
                return Ok(JavaNumber::Double(f64::INFINITY));
            }
        }
        let quantity = self
            .quantity
            .as_mut()
            .unwrap_or_else(|| panic!("java.lang.NullPointerException: quantity"));
        if quantity.is_zeroish() && quantity.is_negative() && !integer_only {
            return Ok(JavaNumber::Double(-0.0));
        }

        if quantity.fits_in_long() && !force_big_decimal {
            Ok(JavaNumber::Long(quantity.to_long(false)))
        } else {
            Ok(JavaNumber::BigDecimal(quantity.to_big_decimal()?))
        }
    }

    /// Equivalent to the package-private `isBetterThan(ParsedNumber)`.
    pub(crate) fn is_better_than(&self, other: &ParsedNumber) -> bool {
        Self::compare(self, other) == Ordering::Greater
    }
}
