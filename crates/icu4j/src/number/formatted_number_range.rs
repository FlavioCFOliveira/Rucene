// © 2018 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.number.FormattedNumberRange`.

use std::fmt;

use rucene::error::Result;
use rucene::util::java_collections::java_identity_hash_code_stand_in;

use crate::jdk::math::BigDecimal;
use crate::jdk::text::AttributedCharacterIterator;
use crate::number::number_range_formatter::RangeIdentityResult;
use crate::r#impl::formatted_string_builder::{FieldObject, FormattedStringBuilder};
use crate::r#impl::formatted_value_string_builder_impl::FormattedValueStringBuilderImpl;
use crate::r#impl::number::decimal_quantity::DecimalQuantity;
use crate::text::constrained_field_position::ConstrainedFieldPosition;
use crate::text::formatted_value::FormattedValue;
use crate::text::plural_rules::IFixedDecimal;

/// The result of a number range formatting operation. This class allows
/// the result to be exported in several data types, including a String, an
/// AttributedCharacterIterator, and a BigDecimal.
///
/// Instances of this class are immutable and thread-safe.
///
/// Equivalent to `com.ibm.icu.number.FormattedNumberRange`; `toString()` is
/// [`FormattedValue::to_java_string`] (and `Display`).
#[derive(Debug)]
pub struct FormattedNumberRange {
    string: FormattedStringBuilder,
    quantity1: Box<dyn DecimalQuantity>,
    quantity2: Box<dyn DecimalQuantity>,
    identity_result: RangeIdentityResult,
}

impl FormattedNumberRange {
    /// Equivalent to the package-private
    /// `FormattedNumberRange(FormattedStringBuilder, DecimalQuantity,
    /// DecimalQuantity, RangeIdentityResult)`.
    pub(crate) fn new(
        string: FormattedStringBuilder,
        quantity1: Box<dyn DecimalQuantity>,
        quantity2: Box<dyn DecimalQuantity>,
        identity_result: RangeIdentityResult,
    ) -> Self {
        FormattedNumberRange {
            string,
            quantity1,
            quantity2,
            identity_result,
        }
    }

    /// Export the first formatted number as a BigDecimal. Equivalent to
    /// `getFirstBigDecimal()`.
    ///
    /// # Errors
    ///
    /// The errors of `DecimalQuantity.toBigDecimal`.
    pub fn get_first_big_decimal(&self) -> Result<BigDecimal> {
        self.quantity1.create_copy().to_big_decimal()
    }

    /// Export the second formatted number as a BigDecimal. Equivalent to
    /// `getSecondBigDecimal()`.
    ///
    /// # Errors
    ///
    /// The errors of `DecimalQuantity.toBigDecimal`.
    pub fn get_second_big_decimal(&self) -> Result<BigDecimal> {
        self.quantity2.create_copy().to_big_decimal()
    }

    /// Returns whether the pair of numbers was successfully formatted as a
    /// range or whether an identity fallback was used. Equivalent to
    /// `getIdentityResult()`.
    pub fn get_identity_result(&self) -> RangeIdentityResult {
        self.identity_result
    }

    /// Equivalent to `hashCode()`: `Arrays.hashCode(chars) ^
    /// Arrays.hashCode(fields)` and the hash codes of both BigDecimals.
    ///
    /// # Errors
    ///
    /// The errors of `DecimalQuantity.toBigDecimal`.
    pub fn hash_code(&self) -> Result<i32> {
        // NumberStringBuilder imports equals and hashCode from CharSequence, which only uses chars.
        let chars = self.string.to_char_array();
        let chars_hash = chars
            .iter()
            .fold(1i32, |h, &c| h.wrapping_mul(31).wrapping_add(i32::from(c)));
        let fields = self.string.to_field_array();
        let fields_hash = fields.iter().fold(1i32, |h, f| {
            h.wrapping_mul(31).wrapping_add(field_object_hash_code(f))
        });
        Ok(chars_hash
            ^ fields_hash
            ^ self.get_first_big_decimal()?.hash_code()
            ^ self.get_second_big_decimal()?.hash_code())
    }

    /// Equivalent to `equals(Object)`.
    ///
    /// # Errors
    ///
    /// The errors of `DecimalQuantity.toBigDecimal`.
    pub fn java_equals(&self, other: &FormattedNumberRange) -> Result<bool> {
        if std::ptr::eq(self, other) {
            return Ok(true);
        }
        // NumberStringBuilder imports equals and hashCode from CharSequence, which only uses chars.
        Ok(self.string.content_equals(&other.string)
            && self
                .get_first_big_decimal()?
                .java_equals(&other.get_first_big_decimal()?)
            && self
                .get_second_big_decimal()?
                .java_equals(&other.get_second_big_decimal()?))
    }

    /// Equivalent to `getFirstFixedDecimal()` (internal).
    pub fn get_first_fixed_decimal(&self) -> &dyn IFixedDecimal {
        self.quantity1.as_ref()
    }

    /// Equivalent to `getSecondFixedDecimal()` (internal).
    pub fn get_second_fixed_decimal(&self) -> &dyn IFixedDecimal {
        self.quantity2.as_ref()
    }
}

/// `Object.hashCode()` of a field of the builder: `Format.Field` keeps the
/// identity hash of `Object` (see `java_identity_hash_code_stand_in`).
fn field_object_hash_code(f: &Option<FieldObject>) -> i32 {
    match f {
        None => 0,
        Some(FieldObject::Field(field)) => java_identity_hash_code_stand_in(&format!(
            "{}${}",
            field.java_class(),
            field.get_name()
        )),
        Some(FieldObject::Wrapper(w)) => java_identity_hash_code_stand_in(&format!("{w:?}")),
    }
}

impl FormattedValue for FormattedNumberRange {
    fn length(&self) -> i32 {
        self.string.length()
    }

    fn char_at(&self, index: i32) -> u16 {
        self.string.char_at(index)
    }

    fn sub_sequence(&self, start: i32, end: i32) -> Vec<u16> {
        self.string.sub_string(start, end)
    }

    fn to_java_string(&self) -> Vec<u16> {
        self.string.to_java_string()
    }

    fn append_to<'a>(&self, appendable: &'a mut Vec<u16>) -> &'a mut Vec<u16> {
        appendable.extend_from_slice(&self.string.to_java_string());
        appendable
    }

    fn next_position(&self, cfpos: &mut ConstrainedFieldPosition) -> bool {
        FormattedValueStringBuilderImpl::next_position(&self.string, cfpos, None)
    }

    fn to_character_iterator(&self) -> Result<Box<dyn AttributedCharacterIterator>> {
        FormattedValueStringBuilderImpl::to_character_iterator(&self.string, None)
    }
}

impl fmt::Display for FormattedNumberRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&crate::jdk::to_rust_string(&self.string.to_java_string()))
    }
}
