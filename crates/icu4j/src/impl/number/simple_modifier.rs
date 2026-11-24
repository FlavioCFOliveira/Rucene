// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.number.SimpleModifier`.

use std::any::Any;

use rucene::error::Result;

use crate::jdk::text::{same_field, FieldRef};
use crate::r#impl::formatted_string_builder::{FieldObject, FormattedStringBuilder};
use crate::r#impl::number::modifier::{Modifier, Parameters};
use crate::r#impl::number::range::prefix_infix_suffix_length_helper::PrefixInfixSuffixLengthHelper;
use crate::r#impl::simple_formatter_impl::SimpleFormatterImpl;
use crate::util::icu_exception::ICUException;

/// `ARG_NUM_LIMIT`: these are the same constants used by
/// `SimpleFormatterImpl`.
const ARG_NUM_LIMIT: i32 = 0x100;

/// The second primary implementation of `Modifier`, this one consuming a
/// `SimpleFormatter` pattern.
///
/// Equivalent to `com.ibm.icu.impl.number.SimpleModifier`. The compiled
/// pattern is UTF-16.
#[derive(Debug, Clone)]
pub struct SimpleModifier {
    compiled_pattern: Vec<u16>,
    field: Option<FieldRef>,
    strong: bool,

    /// Parameters: used for number range formatting
    parameters: Option<Parameters>,
}

impl SimpleModifier {
    /// Equivalent to `SimpleModifier(String, Field, boolean)`.
    pub fn new(compiled_pattern: &[u16], field: Option<FieldRef>, strong: bool) -> Self {
        Self::with_parameters(compiled_pattern, field, strong, None)
    }

    /// Equivalent to `SimpleModifier(String, Field, boolean, Parameters)`.
    pub fn with_parameters(
        compiled_pattern: &[u16],
        field: Option<FieldRef>,
        strong: bool,
        parameters: Option<Parameters>,
    ) -> Self {
        Self {
            compiled_pattern: compiled_pattern.to_vec(),
            field,
            strong,
            parameters,
        }
    }

    /// TODO: This belongs in SimpleFormatterImpl. The only reason I haven't
    /// moved it there yet is because DoubleSidedStringBuilder is an internal
    /// class and SimpleFormatterImpl feels like it should not depend on it.
    ///
    /// Formats a value that is already stored inside the StringBuilder
    /// `result` between the indices `startIndex` and `endIndex` by inserting
    /// characters before the start index and after the end index.
    ///
    /// This is well-defined only for patterns with exactly two arguments.
    /// Equivalent to `formatTwoArgPattern(String, FormattedStringBuilder,
    /// int, PrefixInfixSuffixLengthHelper, Field)`.
    ///
    /// # Errors
    ///
    /// `ICUException()` when the pattern does not have exactly two
    /// arguments.
    pub fn format_two_arg_pattern(
        compiled_pattern: &[u16],
        result: &mut FormattedStringBuilder,
        index: i32,
        h: &mut PrefixInfixSuffixLengthHelper,
        field: Option<FieldRef>,
    ) -> Result<()> {
        let arg_limit = SimpleFormatterImpl::get_argument_limit(compiled_pattern);
        if arg_limit != 2 {
            return Err(ICUException::new().error());
        }
        let field = field.map(FieldObject::Field);
        let mut offset: i32 = 1; // offset into compiledPattern
        let mut length: i32 = 0; // chars added to result

        let mut prefix_length = compiled_pattern[offset as usize] as i32;
        offset += 1;
        if prefix_length < ARG_NUM_LIMIT {
            // No prefix
            prefix_length = 0;
        } else {
            prefix_length -= ARG_NUM_LIMIT;
            result.insert_range(
                index + length,
                compiled_pattern,
                offset,
                offset + prefix_length,
                field.clone(),
            );
            offset += prefix_length;
            length += prefix_length;
            offset += 1;
        }

        let mut infix_length = compiled_pattern[offset as usize] as i32;
        offset += 1;
        if infix_length < ARG_NUM_LIMIT {
            // No infix
            infix_length = 0;
        } else {
            infix_length -= ARG_NUM_LIMIT;
            result.insert_range(
                index + length,
                compiled_pattern,
                offset,
                offset + infix_length,
                field.clone(),
            );
            offset += infix_length;
            length += infix_length;
            offset += 1;
        }

        let suffix_length;
        if offset == compiled_pattern.len() as i32 {
            // No suffix
            suffix_length = 0;
        } else {
            suffix_length = compiled_pattern[offset as usize] as i32 - ARG_NUM_LIMIT;
            offset += 1;
            result.insert_range(
                index + length,
                compiled_pattern,
                offset,
                offset + suffix_length,
                field,
            );
            length += suffix_length;
        }

        h.length_prefix = prefix_length;
        h.length_infix = infix_length;
        h.length_suffix = suffix_length;
        let _ = length;
        Ok(())
    }
}

impl Modifier for SimpleModifier {
    fn apply(
        &self,
        output: &mut FormattedStringBuilder,
        left_index: i32,
        right_index: i32,
    ) -> Result<i32> {
        Ok(SimpleFormatterImpl::format_prefix_suffix(
            &self.compiled_pattern,
            self.field,
            left_index,
            right_index,
            output,
        ))
    }

    fn get_prefix_length(&self) -> Result<i32> {
        Ok(SimpleFormatterImpl::get_prefix_length(
            &self.compiled_pattern,
        ))
    }

    fn get_code_point_count(&self) -> Result<i32> {
        Ok(SimpleFormatterImpl::get_length(
            &self.compiled_pattern,
            true,
        ))
    }

    fn is_strong(&self) -> bool {
        self.strong
    }

    fn contains_field(&self, _field: FieldRef) -> bool {
        // This method is not currently used.
        debug_assert!(false);
        false
    }

    fn get_parameters(&self) -> Option<&Parameters> {
        self.parameters.as_ref()
    }

    fn strict_equals(&self, other: &dyn Modifier) -> bool {
        let other = match other.as_any().downcast_ref::<SimpleModifier>() {
            Some(o) => o,
            None => return false,
        };
        self.compiled_pattern == other.compiled_pattern
            && same_field(self.field, other.field)
            && self.strong == other.strong
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
