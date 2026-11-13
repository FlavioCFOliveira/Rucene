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

//! The `java.lang.String` methods the module calls, over a string held as
//! its UTF-16 code units (`&[u16]`), as JDK 25 specifies them: positions and
//! lengths count units, and a comparison compares units.

use rucene::error::{LuceneError, Result};

/// The UTF-16 units of `s`: Java's `String` literal `s`.
pub fn units(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// `new String(units)` as a Rust `String`.
///
/// **Divergence.** A Java `String` keeps an unpaired surrogate; a Rust
/// `String` cannot hold one, so each becomes U+FFFD.
pub fn to_rust_string(units: &[u16]) -> String {
    String::from_utf16_lossy(units)
}

/// `indexOf(int ch, int fromIndex)`: the first position of the unit `ch` at
/// or after `from`.
pub fn index_of_unit(s: &[u16], ch: u16, from: usize) -> Option<usize> {
    if from >= s.len() {
        return None;
    }
    s[from..].iter().position(|&c| c == ch).map(|i| i + from)
}

/// `lastIndexOf(int ch)`.
pub fn last_index_of_unit(s: &[u16], ch: u16) -> Option<usize> {
    s.iter().rposition(|&c| c == ch)
}

/// `indexOf(String str, int fromIndex)`: an empty `needle` is found at
/// `min(from, length)`.
pub fn index_of(s: &[u16], needle: &[u16], from: usize) -> Option<usize> {
    let from = from.min(s.len());
    if needle.is_empty() {
        return Some(from);
    }
    if needle.len() > s.len() {
        return None;
    }
    (from..=s.len() - needle.len()).find(|&i| s[i..i + needle.len()] == *needle)
}

/// `contains(CharSequence)`.
pub fn contains(s: &[u16], needle: &[u16]) -> bool {
    index_of(s, needle, 0).is_some()
}

/// `contains(String)` of an ASCII literal.
pub fn contains_str(s: &[u16], needle: &str) -> bool {
    contains(s, &units(needle))
}

/// `startsWith(String)` of an ASCII literal.
pub fn starts_with_str(s: &[u16], prefix: &str) -> bool {
    s.starts_with(&units(prefix))
}

/// `endsWith(String)` of an ASCII literal.
pub fn ends_with_str(s: &[u16], suffix: &str) -> bool {
    s.ends_with(&units(suffix))
}

/// Whether `s` equals the literal `literal`.
pub fn equals_str(s: &[u16], literal: &str) -> bool {
    s.iter().copied().eq(literal.encode_utf16())
}

/// `regionMatches(int toffset, String other, int ooffset, int len)`: false
/// when a region does not lie within its string.
pub fn region_matches(s: &[u16], toffset: i64, other: &[u16], ooffset: i64, len: i64) -> bool {
    if ooffset < 0
        || toffset < 0
        || toffset > s.len() as i64 - len
        || ooffset > other.len() as i64 - len
    {
        return false;
    }
    if len <= 0 {
        return true;
    }
    let (t, o, l) = (toffset as usize, ooffset as usize, len as usize);
    s[t..t + l] == other[o..o + l]
}

/// `trim()`: without the leading and trailing units up to U+0020.
pub fn trim(s: &[u16]) -> &[u16] {
    let mut start = 0;
    let mut end = s.len();
    while start < end && s[start] <= 0x20 {
        start += 1;
    }
    while start < end && s[end - 1] <= 0x20 {
        end -= 1;
    }
    &s[start..end]
}

/// `strip()`: without the leading and trailing code points
/// `Character.isWhitespace` accepts.
pub fn strip(s: &[u16]) -> &[u16] {
    let mut start = 0;
    while start < s.len() {
        let cp = super::character::code_point_at(s, start, s.len());
        if !super::character::is_whitespace(cp) {
            break;
        }
        start += super::character::char_count(cp);
    }
    let mut end = s.len();
    while end > start {
        let mut i = end - 1;
        if i > start
            && super::character::is_low_surrogate(s[i])
            && super::character::is_high_surrogate(s[i - 1])
        {
            i -= 1;
        }
        let cp = super::character::code_point_at(s, i, end);
        if !super::character::is_whitespace(cp) {
            break;
        }
        end = i;
    }
    &s[start..end]
}

/// `isBlank()`: whether every code point is whitespace
/// (`Character.isWhitespace`).
pub fn is_blank(s: &[u16]) -> bool {
    strip(s).is_empty()
}

/// Whether `c` is a `\s` of `java.util.regex` without
/// `UNICODE_CHARACTER_CLASS`: `[ \t\n\x0B\f\r]`.
fn is_regex_space(c: u16) -> bool {
    matches!(c, 0x20 | 0x09 | 0x0A | 0x0B | 0x0C | 0x0D)
}

/// `split(regex)` with limit 0 for a regex whose matches are never empty,
/// given as the positions `[start, end)` of its successive matches: a match
/// at position 0 yields a leading empty string, the trailing empty strings
/// are dropped, and a string without a match is returned whole.
fn split_at_matches(
    s: &[u16],
    mut next_match: impl FnMut(usize) -> Option<(usize, usize)>,
) -> Vec<Vec<u16>> {
    let mut list: Vec<Vec<u16>> = Vec::new();
    let mut index = 0;
    let mut from = 0;
    while let Some((start, end)) = next_match(from) {
        list.push(s[index..start].to_vec());
        index = end;
        from = end;
    }
    if index == 0 && list.is_empty() {
        return vec![s.to_vec()];
    }
    list.push(s[index..].to_vec());
    while list.last().is_some_and(Vec::is_empty) {
        list.pop();
    }
    list
}

/// `split("\\s")` (`plus` false) or `split("\\s+")` (`plus` true).
pub fn split_whitespace(s: &[u16], plus: bool) -> Vec<Vec<u16>> {
    split_at_matches(s, |from| {
        let start = (from..s.len()).find(|&i| is_regex_space(s[i]))?;
        let mut end = start + 1;
        if plus {
            while end < s.len() && is_regex_space(s[end]) {
                end += 1;
            }
        }
        Some((start, end))
    })
}

/// `split(regex)` for a regex matching the single unit `ch` (such as `" "`
/// or `"\\|"`).
pub fn split_unit(s: &[u16], ch: u16) -> Vec<Vec<u16>> {
    split_at_matches(s, |from| index_of_unit(s, ch, from).map(|i| (i, i + 1)))
}

/// `replace(char oldChar, char newChar)`.
pub fn replace_unit(s: &[u16], old: u16, new: u16) -> Vec<u16> {
    s.iter().map(|&c| if c == old { new } else { c }).collect()
}

/// `hashCode()`: `s[0]*31^(n-1) + ... + s[n-1]` in `int` arithmetic.
pub fn hash_code(s: &[u16]) -> i32 {
    s.iter()
        .fold(0i32, |h, &c| h.wrapping_mul(31).wrapping_add(i32::from(c)))
}

/// The concatenation of `parts`: Java's `a + b + ...` of strings.
pub fn concat(parts: &[&[u16]]) -> Vec<u16> {
    let mut out = Vec::with_capacity(parts.iter().map(|p| p.len()).sum());
    for part in parts {
        out.extend_from_slice(part);
    }
    out
}

/// `Integer.parseInt(s)`.
///
/// # Errors
///
/// `NumberFormatException("For input string: \"<s>\"")` where Java throws it.
pub fn parse_int(s: &[u16]) -> Result<i32> {
    let text = to_rust_string(s);
    rucene::util::extra::java_parse_int(&text)
        .ok_or_else(|| LuceneError::number_format_for_input_string(&text, 10))
}
