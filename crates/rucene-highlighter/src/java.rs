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

//! JDK behaviours the highlighter relies on: UTF-16 string indexing, the
//! term text of a `CharTermAttribute`, and `Character.digit`.

use rucene::analysis::tokenattributes::CharTermAttribute;
use rucene::error::{LuceneError, LuceneErrorKind, Result};

/// The UTF-16 code units of `s`: Java's `String` contents.
pub(crate) fn units(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// `s.length()` of a Java string: its count of UTF-16 code units.
pub(crate) fn utf16_len(s: &str) -> i32 {
    s.encode_utf16().count() as i32
}

/// `String.substring(begin, end)` over the UTF-16 units `units`.
///
/// A range that cuts a surrogate pair yields U+FFFD for the cut half, the
/// character Java's lone surrogate becomes when the string is encoded.
///
/// # Errors
///
/// `StringIndexOutOfBoundsException("begin B, end E, length L")` when the
/// range is invalid, as `String.checkBoundsBeginEnd` throws.
pub(crate) fn substring(units: &[u16], begin: i32, end: i32) -> Result<String> {
    let length = units.len() as i32;
    if begin < 0 || begin > end || end > length {
        return Err(LuceneError::from(LuceneErrorKind::StringIndexOutOfBounds(
            format!("begin {begin}, end {end}, length {length}"),
        )));
    }
    Ok(String::from_utf16_lossy(
        &units[begin as usize..end as usize],
    ))
}

/// `termAtt.toString()`: the term text of a `CharTermAttribute`.
pub(crate) fn term_text(att: &dyn CharTermAttribute) -> String {
    att.buffer()[..att.length()].iter().collect()
}

/// `Character.digit(codePoint, radix)` for a radix between 2 and 36: the value
/// of a Unicode decimal digit, or of a Latin (ASCII or fullwidth) letter
/// counted from 10, when it is below `radix`; `-1` otherwise.
pub(crate) fn character_digit(code_point: i32, radix: i32) -> i32 {
    let value = if rucene_analysis_common::jdk::character::is_digit(code_point) {
        rucene_analysis_common::jdk::character::get_numeric_value(code_point)
    } else {
        match code_point {
            0x41..=0x5A => code_point - 0x41 + 10,
            0x61..=0x7A => code_point - 0x61 + 10,
            0xFF21..=0xFF3A => code_point - 0xFF21 + 10,
            0xFF41..=0xFF5A => code_point - 0xFF41 + 10,
            _ => -1,
        }
    };
    if value >= 0 && value < radix {
        value
    } else {
        -1
    }
}
