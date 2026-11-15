/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! `java.text.MessageFormat`: the constructor `MessageFormat(String
//! pattern, Locale locale)` and `format(Object)` of an argument array, as
//! the flexible query parser's `NLS` calls them.
//!
//! **Divergence.** An emulation of JDK 25 (`MessageFormat` read as
//! specification), not a port of a Lucene file. The pattern is parsed as
//! `applyPattern` parses it, with its quoting rules and its
//! `IllegalArgumentException`s, and the arguments are formatted as
//! `subformat` formats them: an argument without a format type as `"null"`,
//! a `Number` through `NumberFormat.getInstance(locale)`, a `String` as is,
//! any other object as its `toString()` (see [`JavaObject`]), and an index
//! beyond the arguments as `{n}`. Of the format types, `number` with the
//! default style, `integer` or a `DecimalFormat` pattern is formatted; the
//! other types and styles (`date`, `time`, `choice`, `list`, the
//! `DateTimeFormatter` types, `currency`, `percent` and the compact styles)
//! are recognized as `applyPattern` recognizes them, but formatting an
//! argument through one of them is an `UnsupportedOperationException`
//! naming it: the JDK formats those classes the port has no emulation of
//! (`ChoiceFormat`, `ListFormat`, `DateTimeFormatter`, the currency, percent
//! and compact number formats), and no Lucene message uses them.

use rucene::error::{LuceneError, Result};

use super::character::{to_lower_case_unit, to_upper_case_root, to_upper_case_unit};
use super::decimal_format::{DecimalFormat, DecimalFormatSymbols};
use super::locale::Locale;
use super::locale_data::data;
use super::number_format::{get_instance, get_integer_instance};
use super::object::JavaObject;
use super::string::trim as java_trim;

/// `MessageFormat.MAX_ARGUMENT_INDEX`.
const MAX_ARGUMENT_INDEX: i64 = 10000;

/// `SEG_RAW`, `SEG_INDEX`, `SEG_TYPE`, `SEG_MODIFIER`.
const SEG_RAW: usize = 0;
const SEG_INDEX: usize = 1;
const SEG_TYPE: usize = 2;
const SEG_MODIFIER: usize = 3;

/// The format types of `MessageFormat.FormatType`.
const FORMAT_TYPES: [&str; 23] = [
    "NUMBER",
    "DATE",
    "TIME",
    "DTF_DATE",
    "DTF_TIME",
    "DTF_DATETIME",
    "CHOICE",
    "LIST",
    "BASIC_ISO_DATE",
    "ISO_LOCAL_DATE",
    "ISO_OFFSET_DATE",
    "ISO_DATE",
    "ISO_LOCAL_TIME",
    "ISO_OFFSET_TIME",
    "ISO_TIME",
    "ISO_LOCAL_DATE_TIME",
    "ISO_OFFSET_DATE_TIME",
    "ISO_ZONED_DATE_TIME",
    "ISO_DATE_TIME",
    "ISO_ORDINAL_DATE",
    "ISO_WEEK_DATE",
    "ISO_INSTANT",
    "RFC_1123_DATE_TIME",
];

/// The named styles of `MessageFormat.FormatStyle`, in declaration order.
const FORMAT_STYLES: [&str; 12] = [
    "",
    "short",
    "medium",
    "long",
    "full",
    "integer",
    "currency",
    "percent",
    "compact_short",
    "compact_long",
    "or",
    "unit",
];

/// The subformat of one argument.
#[derive(Debug, Clone)]
enum SubFormat {
    /// A `NumberFormat` the port emulates.
    Number(Box<DecimalFormat>),
    /// A subformat the port has no emulation of: its type and style.
    Unsupported { format_type: String, style: String },
}

/// A `java.text.MessageFormat`.
#[derive(Debug, Clone)]
pub struct MessageFormat {
    locale: Locale,
    /// The pattern text without the arguments, as UTF-16 units.
    pattern: Vec<u16>,
    /// The offset of each argument in `pattern`.
    offsets: Vec<usize>,
    /// The argument number of each argument.
    argument_numbers: Vec<i32>,
    /// The subformat of each argument; `None` for none.
    formats: Vec<Option<SubFormat>>,
}

impl MessageFormat {
    /// Constructs a MessageFormat for the specified locale and pattern.
    ///
    /// Equivalent to `new MessageFormat(String pattern, Locale locale)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException`s of `applyPattern`: `Unmatched braces
    /// in the pattern.`, `can't parse argument number: <s>`, `negative
    /// argument number: <n>`, `<n> exceeds the ArgumentIndex implementation
    /// limit`, `unknown format type: <t>`, and those of a `DecimalFormat`
    /// subformat pattern.
    pub fn new(pattern: &str, locale: &Locale) -> Result<Self> {
        let mut format = Self {
            locale: locale.clone(),
            pattern: Vec::new(),
            offsets: Vec::new(),
            argument_numbers: Vec::new(),
            formats: Vec::new(),
        };
        format.apply_pattern(pattern)?;
        Ok(format)
    }

    /// Equivalent to `applyPattern(String)`.
    fn apply_pattern(&mut self, pattern: &str) -> Result<()> {
        let pattern: Vec<u16> = pattern.encode_utf16().collect();
        let mut segments: [Option<Vec<u16>>; 4] = [Some(Vec::new()), None, None, None];
        let mut part = SEG_RAW;
        let mut in_quote = false;
        let mut brace_stack = 0i32;
        self.offsets.clear();
        self.argument_numbers.clear();
        self.formats.clear();
        let quote = u16::from(b'\'');
        let mut i = 0usize;
        while i < pattern.len() {
            let ch = pattern[i];
            if part == SEG_RAW {
                if ch == quote {
                    if i + 1 < pattern.len() && pattern[i + 1] == quote {
                        segments[part].get_or_insert_with(Vec::new).push(ch); // handle doubles
                        i += 1;
                    } else {
                        in_quote = !in_quote;
                    }
                } else if ch == u16::from(b'{') && !in_quote {
                    part = SEG_INDEX;
                    segments[SEG_INDEX].get_or_insert_with(Vec::new);
                } else {
                    segments[part].get_or_insert_with(Vec::new).push(ch);
                }
            } else if in_quote {
                // just copy quotes in parts
                segments[part].get_or_insert_with(Vec::new).push(ch);
                if ch == quote {
                    in_quote = false;
                }
            } else {
                match ch {
                    0x2C /* ',' */ => {
                        if part < SEG_MODIFIER {
                            part += 1;
                            segments[part].get_or_insert_with(Vec::new);
                        } else {
                            segments[part].get_or_insert_with(Vec::new).push(ch);
                        }
                    }
                    0x7B /* '{' */ => {
                        brace_stack += 1;
                        segments[part].get_or_insert_with(Vec::new).push(ch);
                    }
                    0x7D /* '}' */ => {
                        if brace_stack == 0 {
                            part = SEG_RAW;
                            // Set the subformat
                            self.set_format_from_pattern(&segments)?;
                            // throw away other segments
                            segments[SEG_INDEX] = None;
                            segments[SEG_TYPE] = None;
                            segments[SEG_MODIFIER] = None;
                        } else {
                            brace_stack -= 1;
                            segments[part].get_or_insert_with(Vec::new).push(ch);
                        }
                    }
                    0x20 /* ' ' */ => {
                        // Skip any leading space chars for SEG_TYPE.
                        let seg = segments[part].get_or_insert_with(Vec::new);
                        if part != SEG_TYPE || !seg.is_empty() {
                            seg.push(ch);
                        }
                    }
                    0x27 /* '\'' */ => {
                        in_quote = true;
                        // fall through, so we keep quotes in other parts
                        segments[part].get_or_insert_with(Vec::new).push(ch);
                    }
                    _ => {
                        segments[part].get_or_insert_with(Vec::new).push(ch);
                    }
                }
            }
            i += 1;
        }
        if brace_stack == 0 && part != SEG_RAW {
            self.offsets.clear();
            self.argument_numbers.clear();
            self.formats.clear();
            return Err(LuceneError::illegal_argument(
                "Unmatched braces in the pattern.",
            ));
        }
        self.pattern = segments[SEG_RAW].take().unwrap_or_default();
        Ok(())
    }

    /// Equivalent to `setFormatFromPattern(int, int, StringBuilder[])`.
    fn set_format_from_pattern(&mut self, text_segments: &[Option<Vec<u16>>; 4]) -> Result<()> {
        let segments: Vec<String> = text_segments
            .iter()
            .map(|s| {
                s.as_deref()
                    .map(String::from_utf16_lossy)
                    .unwrap_or_default()
            })
            .collect();
        // get the argument number
        let argument_number = match rucene::util::extra::java_parse_int(&segments[SEG_INDEX]) {
            Some(n) => n,
            None => {
                return Err(LuceneError::illegal_argument(format!(
                    "can't parse argument number: {}",
                    segments[SEG_INDEX]
                )))
            }
        };
        if argument_number < 0 {
            return Err(LuceneError::illegal_argument(format!(
                "negative argument number: {argument_number}"
            )));
        }
        if i64::from(argument_number) >= MAX_ARGUMENT_INDEX {
            return Err(LuceneError::illegal_argument(format!(
                "{argument_number} exceeds the ArgumentIndex implementation limit"
            )));
        }
        let offset = text_segments[SEG_RAW].as_ref().map_or(0, Vec::len);
        // Only search for corresponding type/style if type is not empty
        let format = if !segments[SEG_TYPE].is_empty() {
            Some(self.format_from_pattern(&segments[SEG_TYPE], &segments[SEG_MODIFIER])?)
        } else {
            // Type "" is allowed. e.g., "{0,}", "{0,,}", and "{0,,#}" are
            // treated as "{0}".
            None
        };
        self.offsets.push(offset);
        self.argument_numbers.push(argument_number);
        self.formats.push(format);
        Ok(())
    }

    /// Equivalent to `formatFromPattern(String type, String style)`.
    fn format_from_pattern(&self, format_type: &str, style: &str) -> Result<SubFormat> {
        let upper = String::from_utf16_lossy(&to_upper_case_root(java_trim(
            &format_type.encode_utf16().collect::<Vec<u16>>(),
        )));
        if !FORMAT_TYPES.contains(&upper.as_str()) {
            return Err(LuceneError::illegal_argument(format!(
                "unknown format type: {format_type}"
            )));
        }
        // Get the style if recognized, otherwise treat style as a
        // SubformatPattern
        let style_units: Vec<u16> = style.encode_utf16().collect();
        let trimmed = java_trim(&style_units);
        let named_style = FORMAT_STYLES
            .iter()
            .find(|s| compare_to_ignore_case(trimmed, &s.encode_utf16().collect::<Vec<u16>>()) == 0)
            .copied();
        let unsupported = || SubFormat::Unsupported {
            format_type: format_type.to_string(),
            style: style.to_string(),
        };
        if upper != "NUMBER" {
            return Ok(unsupported());
        }
        match named_style {
            Some("") => Ok(SubFormat::Number(Box::new(get_instance(&self.locale)?))),
            Some("integer") => Ok(SubFormat::Number(Box::new(get_integer_instance(
                &self.locale,
            )?))),
            Some(_) => Ok(unsupported()),
            None => {
                let record = data().number_record(&self.locale)?;
                Ok(SubFormat::Number(Box::new(DecimalFormat::new(
                    style,
                    DecimalFormatSymbols::of(record),
                )?)))
            }
        }
    }

    /// Formats an array of objects.
    ///
    /// Equivalent to `format(Object)` (`Format.format(Object)` over
    /// `format(Object[], StringBuffer, FieldPosition)`): `arguments` is the
    /// array, `None` for `null`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Cannot format given Object as a Number")`
    /// for a non-number argument of a `number` subformat, and the
    /// `UnsupportedOperationException` of a subformat the port does not
    /// emulate (see the module documentation).
    pub fn format(&self, arguments: Option<&[JavaObject]>) -> Result<String> {
        let mut result: Vec<u16> = Vec::new();
        let mut last_offset = 0usize;
        for i in 0..self.offsets.len() {
            result.extend_from_slice(&self.pattern[last_offset..self.offsets[i]]);
            last_offset = self.offsets[i];
            let argument_number = self.argument_numbers[i];
            let arguments = match arguments {
                Some(arguments) if (argument_number as usize) < arguments.len() => arguments,
                _ => {
                    result.extend(format!("{{{argument_number}}}").encode_utf16());
                    continue;
                }
            };
            let obj = &arguments[argument_number as usize];
            let arg: String = match (obj, &self.formats[i]) {
                (JavaObject::Null, _) => "null".to_string(),
                (obj, Some(SubFormat::Number(format))) => match obj {
                    JavaObject::Number(n) => format.format_number(*n)?,
                    _ => {
                        return Err(LuceneError::illegal_argument(
                            "Cannot format given Object as a Number",
                        ))
                    }
                },
                (_, Some(SubFormat::Unsupported { format_type, style })) => {
                    return Err(LuceneError::unsupported_operation(format!(
                        "MessageFormat subformat {{{argument_number},{format_type},{style}}} is not emulated"
                    )))
                }
                (JavaObject::Number(n), None) => get_instance(&self.locale)?.format_number(*n)?,
                (JavaObject::String(s), None) | (JavaObject::Other(s), None) => s.clone(),
            };
            result.extend(arg.encode_utf16());
        }
        result.extend_from_slice(&self.pattern[last_offset..]);
        Ok(String::from_utf16_lossy(&result))
    }
}

/// `String.compareToIgnoreCase(String)` over UTF-16 units: each pair of
/// differing units compared through `Character.toUpperCase` and then
/// `Character.toLowerCase`.
fn compare_to_ignore_case(a: &[u16], b: &[u16]) -> i32 {
    for (&c1, &c2) in a.iter().zip(b) {
        if c1 != c2 {
            let u1 = to_upper_case_unit(c1);
            let u2 = to_upper_case_unit(c2);
            if u1 != u2 {
                let l1 = to_lower_case_unit(u1);
                let l2 = to_lower_case_unit(u2);
                if l1 != l2 {
                    return i32::from(l1) - i32::from(l2);
                }
            }
        }
    }
    a.len() as i32 - b.len() as i32
}
