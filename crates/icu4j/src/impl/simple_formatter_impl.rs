// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html
/*
 *******************************************************************************
 * Copyright (C) 2014-2016, International Business Machines Corporation and
 * others. All Rights Reserved.
 *******************************************************************************
 */

//! `com.ibm.icu.impl.SimpleFormatterImpl`.

use rucene::error::{LuceneError, Result};

use crate::jdk::text::FieldRef;
use crate::jdk::{to_rust_string, units};
use crate::r#impl::formatted_string_builder::{FieldObject, FormattedStringBuilder};

/// Argument numbers must be smaller than this limit.
/// Text segment lengths are offset by this much.
/// This is currently the only unused char value in compiled patterns,
/// except it is the maximum value of the first unit (max arg +1).
const ARG_NUM_LIMIT: i32 = 0x100;
/// Initial and maximum char/UChar value set for a text segment.
/// Segment length char values are from ARG_NUM_LIMIT+1 to this value here.
/// Normally 0xffff, but can be as small as ARG_NUM_LIMIT+1 for testing.
const LEN1_CHAR: u16 = (ARG_NUM_LIMIT + 1) as u16;
const LEN2_CHAR: u16 = (ARG_NUM_LIMIT + 2) as u16;
const LEN3_CHAR: u16 = (ARG_NUM_LIMIT + 3) as u16;
const SEGMENT_LENGTH_ARGUMENT_CHAR: u16 = 0xffff;
/// Maximum length of a text segment. Longer segments are split into shorter ones.
const MAX_SEGMENT_LENGTH: i32 = SEGMENT_LENGTH_ARGUMENT_CHAR as i32 - ARG_NUM_LIMIT;

/// `COMMON_PATTERNS`: precompiled common two-argument patterns.
fn common_patterns() -> [(Vec<u16>, Vec<u16>); 4] {
    let compiled = |parts: &[&[u16]]| parts.concat();
    [
        (
            units("{0} {1}"),
            compiled(&[&[2, 0, LEN1_CHAR], &units(" "), &[1]]),
        ),
        (
            units("{0} ({1})"),
            compiled(&[
                &[2, 0, LEN2_CHAR],
                &units(" ("),
                &[1, LEN1_CHAR],
                &units(")"),
            ]),
        ),
        (
            units("{0}, {1}"),
            compiled(&[&[2, 0, LEN2_CHAR], &units(", "), &[1]]),
        ),
        (
            units("{0} – {1}"),
            compiled(&[&[2, 0, LEN3_CHAR], &units(" – "), &[1]]),
        ), // en dash
    ]
}

/// One `CharSequence` argument value of a format call.
///
/// Java compares each value with the result `StringBuilder` by identity;
/// Rust cannot alias the result with a value, so the result object passed
/// as a value is the variant [`Arg::Result`].
#[derive(Debug, Clone, Copy)]
pub enum Arg<'a> {
    /// A text value.
    Text(&'a [u16]),
    /// A `null` value (`StringBuilder.append(null)` appends `"null"`).
    Null,
    /// The result `StringBuilder` itself.
    Result,
}

/// Formats simple patterns like `"{1} was born in {0}"`. Internal version
/// of `SimpleFormatter` with only static methods, to avoid wrapper objects.
///
/// Equivalent to `com.ibm.icu.impl.SimpleFormatterImpl`. Patterns and
/// compiled patterns are UTF-16 (a compiled pattern stores lengths and
/// argument numbers as units).
pub struct SimpleFormatterImpl;

impl SimpleFormatterImpl {
    /// `compileToStringMinMaxArguments(CharSequence pattern, StringBuilder
    /// sb, int min, int max)`: creates a compiled form of the pattern
    /// string, for use with appropriate static methods.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for bad argument syntax and too few or too
    /// many arguments.
    pub fn compile_to_string_min_max_arguments(
        pattern: &[u16],
        sb: &mut Vec<u16>,
        min: i32,
        max: i32,
    ) -> Result<Vec<u16>> {
        // Return some precompiled common two-argument patterns.
        if min <= 2 && 2 <= max {
            for (p, c) in common_patterns() {
                if p == pattern {
                    debug_assert!(c[0] == 2);
                    return Ok(c);
                }
            }
        }
        // Parse consistent with MessagePattern, but
        // - support only simple numbered arguments
        // - build a simple binary structure into the result string
        let pattern_length = pattern.len();
        sb.reserve(pattern_length);
        // Reserve the first char for the number of arguments.
        sb.resize(1, 0);
        sb.truncate(1);
        let mut text_length: i32 = 0;
        let mut max_arg: i32 = -1;
        let mut in_quote = false;
        let mut i = 0usize;
        while i < pattern_length {
            let mut c = pattern[i];
            i += 1;
            if c == b'\'' as u16 {
                if i < pattern_length && {
                    c = pattern[i];
                    c == b'\'' as u16
                } {
                    // double apostrophe, skip the second one
                    i += 1;
                } else if in_quote {
                    // skip the quote-ending apostrophe
                    in_quote = false;
                    continue;
                } else if c == b'{' as u16 || c == b'}' as u16 {
                    // Skip the quote-starting apostrophe, find the end of the quoted literal text.
                    i += 1;
                    in_quote = true;
                } else {
                    // The apostrophe is part of literal text.
                    c = b'\'' as u16;
                }
            } else if !in_quote && c == b'{' as u16 {
                if text_length > 0 {
                    let at = sb.len() - text_length as usize - 1;
                    sb[at] = (ARG_NUM_LIMIT + text_length) as u16;
                    text_length = 0;
                }
                let mut arg_number: i32;
                if (i + 1) < pattern_length
                    && {
                        arg_number = pattern[i] as i32 - '0' as i32;
                        (0..=9).contains(&arg_number)
                    }
                    && pattern[i + 1] == b'}' as u16
                {
                    i += 2;
                } else {
                    // Multi-digit argument number (no leading zero) or syntax error.
                    // MessagePattern permits PatternProps.skipWhiteSpace(pattern, index)
                    // around the number, but this class does not.
                    let arg_start = i - 1;
                    arg_number = -1;
                    if i < pattern_length && {
                        c = pattern[i];
                        i += 1;
                        (b'1' as u16..=b'9' as u16).contains(&c)
                    } {
                        arg_number = c as i32 - '0' as i32;
                        while i < pattern_length && {
                            c = pattern[i];
                            i += 1;
                            (b'0' as u16..=b'9' as u16).contains(&c)
                        } {
                            arg_number = arg_number * 10 + (c as i32 - '0' as i32);
                            if arg_number >= ARG_NUM_LIMIT {
                                break;
                            }
                        }
                    }
                    if arg_number < 0 || c != b'}' as u16 {
                        return Err(LuceneError::illegal_argument(format!(
                            "Argument syntax error in pattern \"{}\" at index {}: {}",
                            to_rust_string(pattern),
                            arg_start,
                            to_rust_string(&pattern[arg_start..i])
                        )));
                    }
                }
                if arg_number > max_arg {
                    max_arg = arg_number;
                }
                sb.push(arg_number as u16);
                continue;
            } // else: c is part of literal text
              // Append c and track the literal-text segment length.
            if text_length == 0 {
                // Reserve a char for the length of a new text segment, preset the maximum length.
                sb.push(SEGMENT_LENGTH_ARGUMENT_CHAR);
            }
            sb.push(c);
            text_length += 1;
            if text_length == MAX_SEGMENT_LENGTH {
                text_length = 0;
            }
        }
        if text_length > 0 {
            let at = sb.len() - text_length as usize - 1;
            sb[at] = (ARG_NUM_LIMIT + text_length) as u16;
        }
        let arg_count = max_arg + 1;
        if arg_count < min {
            return Err(LuceneError::illegal_argument(format!(
                "Fewer than minimum {} arguments in pattern \"{}\"",
                min,
                to_rust_string(pattern)
            )));
        }
        if arg_count > max {
            return Err(LuceneError::illegal_argument(format!(
                "More than maximum {} arguments in pattern \"{}\"",
                max,
                to_rust_string(pattern)
            )));
        }
        sb[0] = arg_count as u16;
        Ok(sb.clone())
    }

    /// `getArgumentLimit(String compiledPattern)`: the max argument number
    /// + 1.
    pub fn get_argument_limit(compiled_pattern: &[u16]) -> i32 {
        compiled_pattern[0] as i32
    }

    /// `formatCompiledPattern(String compiledPattern, CharSequence...
    /// values)`: formats the given values.
    ///
    /// # Errors
    ///
    /// As [`format_and_append`](Self::format_and_append).
    pub fn format_compiled_pattern(
        compiled_pattern: &[u16],
        values: &[Arg<'_>],
    ) -> Result<Vec<u16>> {
        let mut sb = Vec::new();
        Self::format_and_append(compiled_pattern, &mut sb, None, values)?;
        Ok(sb)
    }

    /// `formatRawPattern(String pattern, int min, int max, CharSequence...
    /// values)`: formats the not-compiled pattern with the given values.
    ///
    /// # Errors
    ///
    /// The errors of compiling and of formatting.
    pub fn format_raw_pattern(
        pattern: &[u16],
        min: i32,
        max: i32,
        values: &[Arg<'_>],
    ) -> Result<Vec<u16>> {
        let mut sb = Vec::new();
        let compiled_pattern =
            Self::compile_to_string_min_max_arguments(pattern, &mut sb, min, max)?;
        sb.clear();
        Self::format_and_append(&compiled_pattern, &mut sb, None, values)?;
        Ok(sb)
    }

    /// `formatAndAppend(String compiledPattern, StringBuilder appendTo,
    /// int[] offsets, CharSequence... values)`: formats the given values,
    /// appending to `append_to`; `offsets` receives the start index of each
    /// value (-1 for an unused one).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Too few values.")`;
    /// `IllegalArgumentException("Value must not be same object as
    /// result")`.
    pub fn format_and_append<'r>(
        compiled_pattern: &[u16],
        append_to: &'r mut Vec<u16>,
        offsets: Option<&mut [i32]>,
        values: &[Arg<'_>],
    ) -> Result<&'r mut Vec<u16>> {
        let values_length = values.len() as i32;
        if values_length < Self::get_argument_limit(compiled_pattern) {
            return Err(LuceneError::illegal_argument("Too few values."));
        }
        Self::format(compiled_pattern, values, append_to, None, true, offsets)
    }

    /// `formatAndReplace(String compiledPattern, StringBuilder result,
    /// int[] offsets, CharSequence... values)`: formats the given values,
    /// replacing the contents of `result`, which may also be a value
    /// ([`Arg::Result`]).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Too few values.")`.
    pub fn format_and_replace<'r>(
        compiled_pattern: &[u16],
        result: &'r mut Vec<u16>,
        offsets: Option<&mut [i32]>,
        values: &[Arg<'_>],
    ) -> Result<&'r mut Vec<u16>> {
        let values_length = values.len() as i32;
        if values_length < Self::get_argument_limit(compiled_pattern) {
            return Err(LuceneError::illegal_argument("Too few values."));
        }

        // If the pattern starts with an argument whose value is the same object
        // as the result, then we keep the result contents and append to it.
        // Otherwise we replace its contents.
        let mut first_arg: i32 = -1;
        // If any non-initial argument value is the same object as the result,
        // then we first copy its contents and use that instead while formatting.
        let mut result_copy: Option<Vec<u16>> = None;
        if Self::get_argument_limit(compiled_pattern) > 0 {
            let mut i = 1;
            while i < compiled_pattern.len() {
                let n = compiled_pattern[i] as i32;
                i += 1;
                if n < ARG_NUM_LIMIT {
                    if matches!(values[n as usize], Arg::Result) {
                        if i == 2 {
                            first_arg = n;
                        } else if result_copy.is_none() {
                            result_copy = Some(result.clone());
                        }
                    }
                } else {
                    i += (n - ARG_NUM_LIMIT) as usize;
                }
            }
        }
        if first_arg < 0 {
            result.clear();
        }
        Self::format(
            compiled_pattern,
            values,
            result,
            result_copy.as_deref(),
            false,
            offsets,
        )
    }

    /// `getTextWithNoArguments(String compiledPattern)`: the pattern text
    /// with none of the arguments.
    pub fn get_text_with_no_arguments(compiled_pattern: &[u16]) -> Vec<u16> {
        let capacity =
            compiled_pattern.len() as i32 - 1 - Self::get_argument_limit(compiled_pattern);
        let mut sb = Vec::with_capacity(capacity.max(0) as usize);
        let mut i = 1;
        while i < compiled_pattern.len() {
            let segment_length = compiled_pattern[i] as i32 - ARG_NUM_LIMIT;
            i += 1;
            if segment_length > 0 {
                let limit = i + segment_length as usize;
                sb.extend_from_slice(&compiled_pattern[i..limit]);
                i = limit;
            }
        }
        sb
    }

    /// `getLength(String compiledPattern, boolean codePoints)`: the length
    /// of the pattern text with none of the arguments, in code points or
    /// UTF-16 units.
    pub fn get_length(compiled_pattern: &[u16], code_points: bool) -> i32 {
        let mut result = 0;
        let mut i = 1;
        while i < compiled_pattern.len() {
            let segment_length = compiled_pattern[i] as i32 - ARG_NUM_LIMIT;
            i += 1;
            if segment_length > 0 {
                let limit = i + segment_length as usize;
                if code_points {
                    result += code_point_count(&compiled_pattern[i..limit]);
                } else {
                    result += (limit - i) as i32;
                }
                i = limit;
            }
        }
        result
    }

    /// `getPrefixLength(String compiledPattern)`: the length in code units
    /// of the pattern text up until the first argument.
    pub fn get_prefix_length(compiled_pattern: &[u16]) -> i32 {
        if compiled_pattern.len() == 1 {
            0
        } else if compiled_pattern[0] == 0 {
            compiled_pattern.len() as i32 - 2
        } else if compiled_pattern[1] as i32 <= ARG_NUM_LIMIT {
            0
        } else {
            compiled_pattern[1] as i32 - ARG_NUM_LIMIT
        }
    }

    /// Special case for using `FormattedStringBuilder` with patterns with 0
    /// or 1 argument.
    ///
    /// With 1 argument, treat the current contents of the
    /// `FormattedStringBuilder` between `start` and `end` as the argument
    /// {0}. Insert prefix and suffix text around the argument.
    ///
    /// With 0 arguments, overwrite the entire contents of the
    /// `FormattedStringBuilder` between `start` and `end`.
    ///
    /// Answers the number of chars inserted (negative when chars were
    /// removed). Equivalent to `formatPrefixSuffix(String, Format.Field, int,
    /// int, FormattedStringBuilder)`.
    pub fn format_prefix_suffix(
        compiled_pattern: &[u16],
        field: Option<FieldRef>,
        start: i32,
        end: i32,
        output: &mut FormattedStringBuilder,
    ) -> i32 {
        let arg_limit = Self::get_argument_limit(compiled_pattern);
        let field = field.map(FieldObject::Field);
        if arg_limit == 0 {
            // No arguments in compiled pattern; overwrite the entire segment
            // with our string.
            output.splice(
                start,
                end,
                compiled_pattern,
                2,
                compiled_pattern.len() as i32,
                field,
            )
        } else {
            debug_assert!(arg_limit == 1);
            let suffix_offset;
            let mut length = 0;
            if compiled_pattern[1] != 0 {
                let prefix_length = compiled_pattern[1] as i32 - ARG_NUM_LIMIT;
                length = output.insert_range(
                    start,
                    compiled_pattern,
                    2,
                    2 + prefix_length,
                    field.clone(),
                );
                suffix_offset = 3 + prefix_length;
            } else {
                suffix_offset = 2;
            }
            if suffix_offset < compiled_pattern.len() as i32 {
                let suffix_length = compiled_pattern[suffix_offset as usize] as i32 - ARG_NUM_LIMIT;
                length += output.insert_range(
                    end + length,
                    compiled_pattern,
                    1 + suffix_offset,
                    1 + suffix_offset + suffix_length,
                    field,
                );
            }
            length
        }
    }

    fn format<'r>(
        compiled_pattern: &[u16],
        values: &[Arg<'_>],
        result: &'r mut Vec<u16>,
        result_copy: Option<&[u16]>,
        forbid_result_as_value: bool,
        offsets: Option<&mut [i32]>,
    ) -> Result<&'r mut Vec<u16>> {
        let mut offsets = offsets;
        let offsets_length = match offsets.as_deref_mut() {
            None => 0,
            Some(o) => {
                for v in o.iter_mut() {
                    *v = -1;
                }
                o.len() as i32
            }
        };
        let mut i = 1;
        while i < compiled_pattern.len() {
            let n = compiled_pattern[i] as i32;
            i += 1;
            if n < ARG_NUM_LIMIT {
                let value = values[n as usize];
                match value {
                    Arg::Result => {
                        if forbid_result_as_value {
                            return Err(LuceneError::illegal_argument(
                                "Value must not be same object as result",
                            ));
                        }
                        if i == 2 {
                            // We are appending to result which is also the first value object.
                            if n < offsets_length {
                                offsets.as_deref_mut().expect("offsets_length > 0")[n as usize] = 0;
                            }
                        } else {
                            if n < offsets_length {
                                offsets.as_deref_mut().expect("offsets_length > 0")[n as usize] =
                                    result.len() as i32;
                            }
                            match result_copy {
                                Some(c) => result.extend_from_slice(c),
                                // StringBuilder.append((String) null)
                                None => result.extend_from_slice(&units("null")),
                            }
                        }
                    }
                    Arg::Text(t) => {
                        if n < offsets_length {
                            offsets.as_deref_mut().expect("offsets_length > 0")[n as usize] =
                                result.len() as i32;
                        }
                        result.extend_from_slice(t);
                    }
                    Arg::Null => {
                        if n < offsets_length {
                            offsets.as_deref_mut().expect("offsets_length > 0")[n as usize] =
                                result.len() as i32;
                        }
                        result.extend_from_slice(&units("null"));
                    }
                }
            } else {
                let limit = i + (n - ARG_NUM_LIMIT) as usize;
                result.extend_from_slice(&compiled_pattern[i..limit]);
                i = limit;
            }
        }
        Ok(result)
    }
}

/// `Character.codePointCount(CharSequence, int, int)` over a range.
fn code_point_count(s: &[u16]) -> i32 {
    let mut count = 0;
    let mut i = 0;
    while i < s.len() {
        if (0xd800..0xdc00).contains(&s[i])
            && i + 1 < s.len()
            && (0xdc00..0xe000).contains(&s[i + 1])
        {
            i += 2;
        } else {
            i += 1;
        }
        count += 1;
    }
    count
}

/// Internal iterator interface for maximum efficiency.
///
/// Equivalent to `SimpleFormatterImpl.IterInternal` (internal, deprecated).
pub struct IterInternal;

impl IterInternal {
    /// `DONE`.
    pub const DONE: i64 = -1;

    /// `step(long state, CharSequence compiledPattern, Appendable output)`:
    /// appends the next text segments and returns the next state, or
    /// `DONE`.
    pub fn step(state: i64, compiled_pattern: &[u16], output: &mut Vec<u16>) -> i64 {
        let mut i = ((state as u64) >> 32) as i32 as usize;
        debug_assert!(i < compiled_pattern.len());
        i += 1;
        while i < compiled_pattern.len() && compiled_pattern[i] as i32 > ARG_NUM_LIMIT {
            let limit = i + compiled_pattern[i] as usize + 1 - ARG_NUM_LIMIT as usize;
            output.extend_from_slice(&compiled_pattern[i + 1..limit]);
            i = limit;
        }
        if i == compiled_pattern.len() {
            return Self::DONE;
        }
        ((i as i64) << 32) | compiled_pattern[i] as i64
    }

    /// `getArgIndex(long state)`.
    pub fn get_arg_index(state: i64) -> i32 {
        state as i32
    }
}
