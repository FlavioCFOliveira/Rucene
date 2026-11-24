// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.number.FormattedNumber`.

use std::fmt;

use rucene::error::Result;

use crate::jdk::math::BigDecimal;
use crate::jdk::text::AttributedCharacterIterator;
use crate::r#impl::formatted_string_builder::FormattedStringBuilder;
use crate::r#impl::formatted_value_string_builder_impl::FormattedValueStringBuilderImpl;
use crate::r#impl::number::decimal_quantity::DecimalQuantity;
use crate::text::constrained_field_position::ConstrainedFieldPosition;
use crate::text::display_options::NounClass;
use crate::text::formatted_value::FormattedValue;
use crate::text::plural_rules::IFixedDecimal;
use crate::util::measure_unit::MeasureUnit;

/// The result of a number formatting operation. This class allows the
/// result to be exported in several data types, including a String, an
/// AttributedCharacterIterator, and a BigDecimal.
///
/// Instances of this class are immutable and thread-safe.
///
/// Equivalent to `com.ibm.icu.number.FormattedNumber`; `toString()` is
/// [`FormattedValue::to_java_string`] (and `Display`).
#[derive(Debug)]
pub struct FormattedNumber {
    /// `string`.
    string: FormattedStringBuilder,
    /// `fq`.
    fq: Box<dyn DecimalQuantity>,
    /// `outputUnit`.
    output_unit: Option<MeasureUnit>,
    /// `gender`: grammatical gender of the formatted result.
    gender: Option<String>,
}

impl FormattedNumber {
    /// `FormattedNumber(FormattedStringBuilder, DecimalQuantity,
    /// MeasureUnit, String)` (package-private).
    pub(crate) fn new(
        nsb: FormattedStringBuilder,
        fq: Box<dyn DecimalQuantity>,
        output_unit: Option<MeasureUnit>,
        gender: Option<String>,
    ) -> Self {
        FormattedNumber {
            string: nsb,
            fq,
            output_unit,
            gender,
        }
    }

    /// Export the formatted number as a BigDecimal. This endpoint is useful
    /// for obtaining the exact number being printed after scaling and
    /// rounding have been applied by the number formatting pipeline.
    /// Equivalent to `toBigDecimal()`.
    ///
    /// # Errors
    ///
    /// The errors of the conversion.
    pub fn to_big_decimal(&self) -> Result<BigDecimal> {
        // Java converts the quantity itself; its representation may change
        // but not its value, so a copy answers the same.
        self.fq.create_copy().to_big_decimal()
    }

    /// Gets the resolved output unit. Equivalent to `getOutputUnit()`.
    pub fn get_output_unit(&self) -> Option<&MeasureUnit> {
        self.output_unit.as_ref()
    }

    /// Gets the noun class of the formatted output. Returns `UNDEFINED`
    /// when the noun class is not supported yet. Equivalent to
    /// `getNounClass()`.
    pub fn get_noun_class(&self) -> NounClass {
        NounClass::from_identifier(self.gender.as_deref())
    }

    /// The gender of the formatted output. Equivalent to `getGender()`
    /// (internal).
    pub fn get_gender(&self) -> &str {
        self.gender.as_deref().unwrap_or("")
    }

    /// Equivalent to `getFixedDecimal()` (internal).
    pub fn get_fixed_decimal(&self) -> &dyn IFixedDecimal {
        self.fq.as_ref()
    }

    /// `getFixedDecimal()` handing over the quantity (the result object is
    /// dropped by the caller, as `DecimalFormat.getFixedDecimal` does).
    pub fn into_fixed_decimal(self) -> Box<dyn IFixedDecimal> {
        self.fq
    }
}

impl FormattedValue for FormattedNumber {
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

impl fmt::Display for FormattedNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&crate::jdk::to_rust_string(&self.string.to_java_string()))
    }
}
