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

//! Port of `org.apache.lucene.codecs.simpletext.SimpleTextUtil`, plus the
//! small JDK and Lucene helpers every `SimpleText` reader shares.

use rucene::error::{LuceneError, Result};
use rucene::store::{ChecksumIndexInput, DataInput, DataOutput, IndexOutput};
use rucene::util::java_fmt::java_utf8_to_string;
use rucene::util::{ArrayUtil, BytesRef};

/// `SimpleTextUtil.NEWLINE`.
pub const NEWLINE: u8 = 10;
/// `SimpleTextUtil.ESCAPE`.
pub const ESCAPE: u8 = 92;
/// `SimpleTextUtil.CHECKSUM`.
pub const CHECKSUM: &[u8] = b"checksum ";

/// Writes `s` as UTF-8, escaping newlines and backslashes.
///
/// Port of `SimpleTextUtil.write(DataOutput, String, BytesRefBuilder)`: the
/// string is copied into the scratch as UTF-8 (`BytesRefBuilder.copyChars`)
/// and written with [`write_bytes`]. A Rust `&str` is valid UTF-8, so the
/// copy is the string's own bytes.
///
/// # Errors
///
/// Propagates the output's error.
pub fn write_str(out: &mut dyn DataOutput, s: &str) -> Result<()> {
    write_bytes(out, s.as_bytes())
}

/// Writes `b`, escaping newlines and backslashes.
///
/// Port of `SimpleTextUtil.write(DataOutput, BytesRef)`.
///
/// # Errors
///
/// Propagates the output's error.
pub fn write_bytes(out: &mut dyn DataOutput, b: &[u8]) -> Result<()> {
    for &bx in b {
        if bx == NEWLINE || bx == ESCAPE {
            out.write_byte(ESCAPE)?;
        }
        out.write_byte(bx)?;
    }
    Ok(())
}

/// Writes a newline.
///
/// Port of `SimpleTextUtil.writeNewline`.
///
/// # Errors
///
/// Propagates the output's error.
pub fn write_newline(out: &mut dyn DataOutput) -> Result<()> {
    out.write_byte(NEWLINE)
}

/// Reads one line into `scratch`, unescaping, without the newline.
///
/// Port of `SimpleTextUtil.readLine`.
///
/// # Errors
///
/// Propagates the input's error (an `EOFException` past the end).
pub fn read_line(input: &mut dyn DataInput, scratch: &mut Vec<u8>) -> Result<()> {
    scratch.clear();
    loop {
        let b = input.read_byte()?;
        if b == ESCAPE {
            scratch.push(input.read_byte()?);
        } else if b == NEWLINE {
            break;
        } else {
            scratch.push(b);
        }
    }
    Ok(())
}

/// Writes the checksum line: `checksum `, the output's checksum zero-padded
/// to 20 digits, a newline.
///
/// Port of `SimpleTextUtil.writeChecksum`, which formats the checksum with
/// `String.format(Locale.ROOT, "%020d", out.getChecksum())`.
///
/// # Errors
///
/// Propagates the output's error.
pub fn write_checksum(out: &mut dyn IndexOutput) -> Result<()> {
    // Pad with zeros so different checksum values use the same number of
    // bytes (BaseIndexFileFormatTestCase.testMergeStability cares):
    let checksum = format!("{:020}", out.checksum()?);
    write_bytes(out, CHECKSUM)?;
    write_str(out, &checksum)?;
    write_newline(out)
}

/// Verifies the checksum line that ends `input`.
///
/// Port of `SimpleTextUtil.checkFooter`.
///
/// # Errors
///
/// Returns [`LuceneError::CorruptIndex`] with Lucene's message when the line
/// is not a checksum line, when the checksum does not match, or when bytes
/// follow it.
pub fn check_footer(input: &mut dyn ChecksumIndexInput) -> Result<()> {
    let mut scratch = Vec::new();
    let expected_checksum = format!("{:020}", input.get_checksum()?);
    read_line(input, &mut scratch)?;
    if !scratch.starts_with(CHECKSUM) {
        return Err(LuceneError::corrupt_index(
            format!(
                "SimpleText failure: expected checksum line but got {}",
                java_utf8_to_string(&scratch)
            ),
            input.resource_description(),
        ));
    }
    let actual_checksum = java_utf8_to_string(&scratch[CHECKSUM.len()..]);
    if expected_checksum != actual_checksum {
        return Err(LuceneError::corrupt_index(
            format!("SimpleText checksum failure: {actual_checksum} != {expected_checksum}"),
            input.resource_description(),
        ));
    }
    if input.length() != input.file_pointer() {
        return Err(LuceneError::corrupt_index(
            "Unexpected stuff at the end of file, please be careful with your text editor!",
            input.resource_description(),
        ));
    }
    Ok(())
}

/// Inverse of `BytesRef.toString()`: parses `[6c 75]` back into bytes.
///
/// Port of `SimpleTextUtil.fromBytesRefString`.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] with Lucene's message when `s`
/// is not bracketed, and with `Integer.parseInt`'s `NumberFormatException`
/// message when a byte is not hexadecimal.
pub fn from_bytes_ref_string(s: &str) -> Result<BytesRef> {
    let units: Vec<u16> = s.encode_utf16().collect();
    let not_created = || {
        LuceneError::IllegalArgument(format!(
            "string {s} was not created from BytesRef.toString?"
        ))
    };
    if units.len() < 2 {
        return Err(not_created());
    }
    if units[0] != u16::from(b'[') || units[units.len() - 1] != u16::from(b']') {
        return Err(not_created());
    }
    if units.len() == 2 {
        return Ok(BytesRef::new(Vec::new()));
    }
    // `s.substring(1, s.length() - 1).split(" ")`: the brackets are ASCII.
    let inner = &s[1..s.len() - 1];
    let parts = java_split(inner, ' ');
    let mut bytes = Vec::with_capacity(parts.len());
    for part in parts {
        // `(byte) Integer.parseInt(parts[i], 16)`.
        let value = parse_int_radix16(part)?;
        bytes.push(value as u8);
    }
    Ok(BytesRef::new(bytes))
}

/// `Integer.parseInt(s, 16)`.
fn parse_int_radix16(s: &str) -> Result<i32> {
    let chars: Vec<char> = s.chars().collect();
    let failure =
        || LuceneError::IllegalArgument(format!("For input string: \"{s}\" under radix 16"));
    if chars.is_empty() {
        return Err(LuceneError::IllegalArgument(format!(
            "For input string: \"{s}\" under radix 16"
        )));
    }
    let (negative, digits) = match chars[0] {
        '-' => (true, &chars[1..]),
        '+' => (false, &chars[1..]),
        _ => (false, &chars[..]),
    };
    if digits.is_empty() {
        return Err(failure());
    }
    let mut result: i64 = 0;
    for &c in digits {
        let d = rucene::util::java_character_digit(c, 16).ok_or_else(failure)?;
        result = result * 16 + i64::from(d);
        if result > i64::from(i32::MAX) + 1 {
            return Err(failure());
        }
    }
    let value = if negative { -result } else { result };
    i32::try_from(value).map_err(|_| failure())
}

/// `String.split(String)` with a one-character, non-regex-special separator:
/// the pieces between separators, with trailing empty pieces removed.
pub(crate) fn java_split(s: &str, separator: char) -> Vec<&str> {
    let mut parts: Vec<&str> = s.split(separator).collect();
    while parts.len() > 1 && parts.last().is_some_and(|p| p.is_empty()) {
        parts.pop();
    }
    if parts.len() == 1 && parts[0].is_empty() && !s.is_empty() {
        parts.clear();
    }
    parts
}

/// `new String(scratch.bytes(), offset, scratch.length() - offset, UTF_8)`.
pub(crate) fn read_string(scratch: &[u8], offset: usize) -> String {
    java_utf8_to_string(&scratch[offset.min(scratch.len())..])
}

/// `ArrayUtil.parseInt` over `scratchUTF16.copyUTF8Bytes(scratch, offset, ...)`.
///
/// # Errors
///
/// Returns `ArrayUtil.parseInt`'s `NumberFormatException` as
/// [`LuceneError::IllegalArgument`].
pub(crate) fn parse_int_at(scratch: &[u8], offset: usize) -> Result<i32> {
    let chars: Vec<char> = read_string(scratch, offset).chars().collect();
    ArrayUtil::parse_int(&chars, 0, chars.len())
}

/// `Integer.parseInt(String)`.
///
/// # Errors
///
/// Returns the `NumberFormatException` message as
/// [`LuceneError::IllegalArgument`].
pub(crate) fn parse_int(s: &str) -> Result<i32> {
    rucene::util::extra::java_parse_int(s)
        .ok_or_else(|| LuceneError::IllegalArgument(number_format_message(s)))
}

/// `Long.parseLong(String)`.
///
/// # Errors
///
/// Returns the `NumberFormatException` message as
/// [`LuceneError::IllegalArgument`].
pub(crate) fn parse_long(s: &str) -> Result<i64> {
    rucene::util::extra::java_parse_long(s)
        .ok_or_else(|| LuceneError::IllegalArgument(number_format_message(s)))
}

/// `Float.parseFloat(String)`.
///
/// # Errors
///
/// Returns the `NumberFormatException` as [`LuceneError::IllegalArgument`].
pub(crate) fn parse_float(s: &str) -> Result<f32> {
    rucene::util::java_fmt::parse_float(s).ok_or_else(|| float_format_error(s))
}

/// `Double.parseDouble(String)`.
///
/// # Errors
///
/// Returns the `NumberFormatException` as [`LuceneError::IllegalArgument`].
pub(crate) fn parse_double(s: &str) -> Result<f64> {
    rucene::util::java_fmt::parse_double(s).ok_or_else(|| float_format_error(s))
}

/// `Boolean.parseBoolean(String)`: `"true".equalsIgnoreCase(s)`.
pub(crate) fn parse_boolean(s: &str) -> bool {
    s.eq_ignore_ascii_case("true")
}

/// The message of the `NumberFormatException` `Integer.parseInt` and
/// `Long.parseLong` throw: `For input string: "<s>"`, or the JDK's special
/// message for an empty string.
fn number_format_message(s: &str) -> String {
    if s.is_empty() {
        "For input string: \"\"".to_string()
    } else {
        format!("For input string: \"{s}\"")
    }
}

/// The `NumberFormatException` `FloatingDecimal.readJavaFormatString`
/// throws: `empty String` for a blank string, `For input string: "<s>"`
/// otherwise.
fn float_format_error(s: &str) -> LuceneError {
    if s.trim_matches(|c: char| c <= '\u{20}').is_empty() {
        LuceneError::IllegalArgument("empty String".to_string())
    } else {
        LuceneError::IllegalArgument(format!("For input string: \"{s}\""))
    }
}

/// `Boolean.toString(boolean)`.
pub(crate) fn bool_str(b: bool) -> &'static str {
    if b {
        "true"
    } else {
        "false"
    }
}

/// The message of a Java exception the port reports as `e`: the text a
/// variant carries, without the port's `Display` prefix.
pub(crate) fn java_message(e: &LuceneError) -> String {
    match e {
        LuceneError::IllegalArgument(m) | LuceneError::CorruptIndex(m) => m.clone(),
        LuceneError::IllegalArgumentWithCause { message, .. } => message.clone(),
        LuceneError::IllegalState { message, .. } => message.clone(),
        LuceneError::UnsupportedOperation(m) => m.clone(),
        other => other.to_string(),
    }
}

/// `HashMap.calculateHashMapCapacity(numMappings)`, the initial capacity
/// `HashMap.newHashMap` and `HashSet.newHashSet` pass on:
/// `(int) Math.ceil(numMappings / 0.75)`.
pub(crate) fn hash_map_capacity(num_mappings: i32) -> i32 {
    (f64::from(num_mappings) / 0.75).ceil() as i32
}
