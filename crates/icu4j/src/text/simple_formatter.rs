// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.text.SimpleFormatter`.

use std::fmt;

use rucene::error::Result;

use crate::jdk::{to_rust_string, units};
use crate::r#impl::simple_formatter_impl::{Arg, SimpleFormatterImpl};

/// Formats simple patterns like `"{1} was born in {0}"`, compiled once
/// and immutable.
///
/// Equivalent to `com.ibm.icu.text.SimpleFormatter`; `CharSequence...`
/// values are [`Arg`]s.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimpleFormatter {
    compiled_pattern: Vec<u16>,
}

impl SimpleFormatter {
    /// `compile(CharSequence pattern)`: any number of arguments.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a bad argument syntax.
    pub fn compile(pattern: &[u16]) -> Result<Self> {
        Self::compile_min_max_arguments(pattern, 0, i32::MAX)
    }

    /// `compileMinMaxArguments(CharSequence pattern, int min, int max)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a bad argument syntax or too few or
    /// too many arguments.
    pub fn compile_min_max_arguments(pattern: &[u16], min: i32, max: i32) -> Result<Self> {
        let mut sb = Vec::new();
        let compiled_pattern =
            SimpleFormatterImpl::compile_to_string_min_max_arguments(pattern, &mut sb, min, max)?;
        Ok(Self { compiled_pattern })
    }

    /// `getArgumentLimit()`: 1 + the highest argument number, 0 when there
    /// are none.
    pub fn get_argument_limit(&self) -> i32 {
        SimpleFormatterImpl::get_argument_limit(&self.compiled_pattern)
    }

    /// `format(CharSequence... values)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Too few values.")`.
    pub fn format(&self, values: &[Arg<'_>]) -> Result<Vec<u16>> {
        SimpleFormatterImpl::format_compiled_pattern(&self.compiled_pattern, values)
    }

    /// `formatAndAppend(StringBuilder appendTo, int[] offsets,
    /// CharSequence... values)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Too few values.")`, and
    /// `IllegalArgumentException` when a value is `appendTo` itself.
    pub fn format_and_append<'r>(
        &self,
        append_to: &'r mut Vec<u16>,
        offsets: Option<&mut [i32]>,
        values: &[Arg<'_>],
    ) -> Result<&'r mut Vec<u16>> {
        SimpleFormatterImpl::format_and_append(&self.compiled_pattern, append_to, offsets, values)
    }

    /// `formatAndReplace(StringBuilder result, int[] offsets,
    /// CharSequence... values)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Too few values.")`.
    pub fn format_and_replace<'r>(
        &self,
        result: &'r mut Vec<u16>,
        offsets: Option<&mut [i32]>,
        values: &[Arg<'_>],
    ) -> Result<&'r mut Vec<u16>> {
        SimpleFormatterImpl::format_and_replace(&self.compiled_pattern, result, offsets, values)
    }

    /// `toString()`: the pattern with `{i}` for every argument.
    pub fn to_java_string(&self) -> Vec<u16> {
        let values: Vec<Vec<u16>> = (0..self.get_argument_limit())
            .map(|i| units(&format!("{{{i}}}")))
            .collect();
        let args: Vec<Arg<'_>> = values.iter().map(|v| Arg::Text(v)).collect();
        let mut sb = Vec::new();
        // The values are exactly getArgumentLimit() texts: cannot fail.
        let _ = self.format_and_append(&mut sb, None, &args);
        sb
    }

    /// `getTextWithNoArguments()`: the pattern text with the arguments
    /// removed.
    pub fn get_text_with_no_arguments(&self) -> Vec<u16> {
        SimpleFormatterImpl::get_text_with_no_arguments(&self.compiled_pattern)
    }
}

impl fmt::Display for SimpleFormatter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&to_rust_string(&self.to_java_string()))
    }
}
