// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.ConstantMultiFieldModifier`.

use rucene::error::Result;
use std::any::Any;

use crate::jdk::text::FieldRef;
use crate::jdk::units;
use crate::lang::u_character::UCharacter;
use crate::r#impl::formatted_string_builder::{FieldObject, FormattedStringBuilder};
use crate::r#impl::number::modifier::{Modifier, Parameters};

/// An implementation of `Modifier` that allows for multiple types of fields
/// in the same modifier. Constructed based on the contents of two
/// `FormattedStringBuilder` instances (one for the prefix, one for the
/// suffix).
///
/// Equivalent to `com.ibm.icu.impl.number.ConstantMultiFieldModifier`. Its
/// subclass `CurrencySpacingEnabledModifier` holds one of these and
/// delegates to it (Java inheritance of the protected arrays).
#[derive(Debug, Clone)]
pub struct ConstantMultiFieldModifier {
    // NOTE: In Java, these are stored as array pointers. In C++, the
    // FormattedStringBuilder is stored by value and is not copied.
    /// `prefixChars`.
    pub(crate) prefix_chars: Vec<u16>,
    /// `suffixChars`.
    pub(crate) suffix_chars: Vec<u16>,
    /// `prefixFields`.
    pub(crate) prefix_fields: Vec<Option<FieldObject>>,
    /// `suffixFields`.
    pub(crate) suffix_fields: Vec<Option<FieldObject>>,
    overwrite: bool,
    strong: bool,

    /// Parameters: used for number range formatting
    parameters: Option<Parameters>,
}

impl ConstantMultiFieldModifier {
    /// Equivalent to `ConstantMultiFieldModifier(FormattedStringBuilder,
    /// FormattedStringBuilder, boolean, boolean)`.
    pub fn new(prefix: &FormattedStringBuilder, suffix: &FormattedStringBuilder, overwrite: bool, strong: bool) -> Self {
        Self::with_parameters(prefix, suffix, overwrite, strong, None)
    }

    /// Equivalent to `ConstantMultiFieldModifier(FormattedStringBuilder,
    /// FormattedStringBuilder, boolean, boolean, Parameters)`.
    pub fn with_parameters(
        prefix: &FormattedStringBuilder,
        suffix: &FormattedStringBuilder,
        overwrite: bool,
        strong: bool,
        parameters: Option<Parameters>,
    ) -> Self {
        Self {
            prefix_chars: prefix.to_char_array(),
            suffix_chars: suffix.to_char_array(),
            prefix_fields: prefix.to_field_array(),
            suffix_fields: suffix.to_field_array(),
            overwrite,
            strong,
            parameters,
        }
    }

    /// Equivalent to `toString()`.
    pub fn to_java_string(&self) -> Vec<u16> {
        let mut temp = FormattedStringBuilder::new();
        // This modifier's apply never fails (it only inserts its own chars).
        let _ = self.apply(&mut temp, 0, 0);
        // Infallible for this modifier.
        let prefix_length = self.prefix_chars.len() as i32;
        let mut s = units("<ConstantMultiFieldModifier prefix:'");
        s.extend_from_slice(&temp.sub_sequence(0, prefix_length).to_java_string());
        s.extend_from_slice(&units("' suffix:'"));
        s.extend_from_slice(&temp.sub_sequence(prefix_length, temp.length()).to_java_string());
        s.extend_from_slice(&units("'>"));
        s
    }
}

impl Modifier for ConstantMultiFieldModifier {
    fn apply(&self, output: &mut FormattedStringBuilder, left_index: i32, right_index: i32) -> Result<i32> {
        let mut length = output.insert_chars(left_index, &self.prefix_chars, Some(&self.prefix_fields));
        if self.overwrite {
            length += output.splice(left_index + length, right_index + length, &[], 0, 0, None);
        }
        length += output.insert_chars(right_index + length, &self.suffix_chars, Some(&self.suffix_fields));
        Ok(length)
    }

    fn get_prefix_length(&self) -> Result<i32> {
        Ok(self.prefix_chars.len() as i32)
    }

    fn get_code_point_count(&self) -> Result<i32> {
        Ok(UCharacter::code_point_count(&self.prefix_chars, 0, self.prefix_chars.len() as i32)
            + UCharacter::code_point_count(&self.suffix_chars, 0, self.suffix_chars.len() as i32))
    }

    fn is_strong(&self) -> bool {
        self.strong
    }

    fn contains_field(&self, field: FieldRef) -> bool {
        for f in &self.prefix_fields {
            if crate::r#impl::formatted_string_builder::is_field(f.as_ref(), field) {
                return true;
            }
        }
        for f in &self.suffix_fields {
            if crate::r#impl::formatted_string_builder::is_field(f.as_ref(), field) {
                return true;
            }
        }
        false
    }

    fn get_parameters(&self) -> Option<&Parameters> {
        self.parameters.as_ref()
    }

    fn strict_equals(&self, other: &dyn Modifier) -> bool {
        let other = match other.as_any().downcast_ref::<ConstantMultiFieldModifier>() {
            Some(o) => o,
            None => match other
                .as_any()
                .downcast_ref::<crate::r#impl::number::currency_spacing_enabled_modifier::CurrencySpacingEnabledModifier>()
            {
                // `instanceof ConstantMultiFieldModifier` also holds for the
                // subclass.
                Some(o) => o.base(),
                None => return false,
            },
        };
        // Arrays.equals(Object[], Object[]) compares the fields with
        // equals(Object), which is identity for fields and wrappers.
        self.prefix_chars == other.prefix_chars
            && self.prefix_fields == other.prefix_fields
            && self.suffix_chars == other.suffix_chars
            && self.suffix_fields == other.suffix_fields
            && self.overwrite == other.overwrite
            && self.strong == other.strong
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
