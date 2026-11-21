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
use rucene::util::automaton::case_folding;

use super::break_iterator;
use super::character::{
    char_count, code_point_at, get_type, is_high_surrogate, is_low_surrogate, LOWERCASE_LETTER,
    TITLECASE_LETTER, UPPERCASE_LETTER,
};
use super::locale::Locale;

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

/// `String.toLowerCase(Locale)` on JDK 25: `s` lowercased with the rules of
/// `locale`, for a locale whose language is none of `tr`, `az` and `lt`.
///
/// Written from `java.lang.StringUTF16.toLowerCase(String, byte[], Locale)`
/// and `toLowerCaseEx`, and from the language-independent entries of
/// `java.lang.ConditionalSpecialCasing`, read as specification: every code
/// point maps through `Character.toLowerCase(int)`, except
///
/// * U+03A3 GREEK CAPITAL LETTER SIGMA, which becomes U+03C2 (final sigma)
///   when it ends a word that a cased letter precedes, the word boundaries
///   being those of `BreakIterator.getWordInstance(locale)` over `s`, and
///   U+03C3 otherwise;
/// * U+0130 LATIN CAPITAL LETTER I WITH DOT ABOVE, which becomes the two
///   units U+0069 U+0307.
///
/// A string of Latin-1 characters only takes `StringLatin1.toLowerCase`,
/// which maps each through `CharacterDataLatin1.toLowerCase`: the same
/// result, so one path serves both.
///
/// # Panics
///
/// When the language of `locale` is `tr`, `az` or `lt`, whose
/// locale-dependent mappings (`ConditionalSpecialCasing`'s `AFTER_I`,
/// `NOT_BEFORE_DOT`, `MORE_ABOVE`, `AFTER_SOFT_DOTTED`) are not emulated;
/// no caller passes one.
pub fn to_lower_case(s: &[u16], locale: &Locale) -> Vec<u16> {
    let lang = locale.get_language();
    assert!(
        lang != "tr" && lang != "az" && lang != "lt",
        "String.toLowerCase is not emulated for the language {lang}"
    );
    let len = s.len();
    let mut first = 0;
    let mut has_surr = false;
    // Now check if there are any characters that need to be changed, or are surrogate
    while first < len {
        let cp = i32::from(s[first]);
        if is_surrogate(s[first]) {
            has_surr = true;
            break;
        }
        if cp != case_folding::to_lower_case(cp) {
            break;
        }
        first += 1;
    }
    if first == len {
        return s.to_vec();
    }
    // Just copy the first few lowerCase characters.
    let mut result = s[..first].to_vec();
    if has_surr {
        return to_lower_case_ex(s, result, first, locale, false);
    }
    for i in first..len {
        let cp = i32::from(s[i]);
        if cp == 0x03A3 || is_surrogate(s[i]) {
            // GREEK CAPITAL LETTER SIGMA
            return to_lower_case_ex(s, result, i, locale, false);
        }
        if cp == 0x0130 {
            // LATIN CAPITAL LETTER I WITH DOT ABOVE
            return to_lower_case_ex(s, result, i, locale, true);
        }
        let cp = case_folding::to_lower_case(cp);
        if !(0..=0xFFFF).contains(&cp) {
            return to_lower_case_ex(s, result, i, locale, false);
        }
        result.push(cp as u16);
    }
    result
}

/// `Character.ERROR`, `0xFFFFFFFF`.
const CHARACTER_ERROR: i32 = -1;

/// `StringUTF16.toLowerCaseEx(String, byte[], byte[], int, Locale,
/// boolean)`: `result` holds the lowercased units before `first`.
fn to_lower_case_ex(
    s: &[u16],
    mut result: Vec<u16>,
    first: usize,
    locale: &Locale,
    locale_dependent: bool,
) -> Vec<u16> {
    let length = s.len();
    let mut i = first;
    while i < length {
        let mut src_char = i32::from(s[i]);
        let mut src_count = 1;
        if is_surrogate(s[i]) {
            src_char = code_point_at(s, i, length);
            src_count = char_count(src_char);
        }
        let lower_char = if locale_dependent
            || src_char == 0x03A3 // GREEK CAPITAL LETTER SIGMA
            || src_char == 0x0130
        {
            // LATIN CAPITAL LETTER I WITH DOT ABOVE
            conditional_to_lower_case_ex(s, i, locale)
        } else {
            case_folding::to_lower_case(src_char)
        };
        if (0..=0xFFFF).contains(&lower_char) {
            // Character.ERROR is not a bmp
            result.push(lower_char as u16);
        } else {
            let lower_char_array = if lower_char == CHARACTER_ERROR {
                conditional_look_up_lower_case(s, i, locale)
                    .expect("INVARIANT: Character.ERROR comes from a table entry")
            } else {
                to_chars(lower_char)
            };
            result.extend_from_slice(&lower_char_array);
        }
        i += src_count;
    }
    result
}

/// `Character.toChars(int)` of a supplementary code point.
fn to_chars(code_point: i32) -> Vec<u16> {
    let c = u32::try_from(code_point)
        .ok()
        .and_then(char::from_u32)
        .expect("INVARIANT: Character.toLowerCase answers a valid code point");
    let mut buf = [0u16; 2];
    c.encode_utf16(&mut buf).to_vec()
}

/// `ConditionalSpecialCasing.toLowerCaseEx(String, int, Locale)`.
fn conditional_to_lower_case_ex(s: &[u16], index: usize, locale: &Locale) -> i32 {
    match conditional_look_up_lower_case(s, index, locale) {
        Some(result) => {
            if result.len() == 1 {
                i32::from(result[0])
            } else {
                CHARACTER_ERROR
            }
        }
        // default to Character class' one
        None => case_folding::to_lower_case(code_point_at(s, index, s.len())),
    }
}

/// `ConditionalSpecialCasing.lookUpTable(src, index, locale, true)` for a
/// language other than `tr`, `az` and `lt`: only the two entries without a
/// language apply, U+03A3 under the `FINAL_CASED` condition and U+0130
/// unconditionally.
fn conditional_look_up_lower_case(s: &[u16], index: usize, locale: &Locale) -> Option<Vec<u16>> {
    match code_point_at(s, index, s.len()) {
        // # GREEK CAPITAL LETTER SIGMA
        0x03A3 if is_final_cased(s, index, locale) => Some(vec![0x03C2]),
        // # LATIN CAPITAL LETTER I WITH DOT ABOVE
        0x0130 => Some(vec![0x0069, 0x0307]),
        _ => None,
    }
}

/// `ConditionalSpecialCasing.isFinalCased(String, int, Locale)`: a cased
/// letter precedes `index` in its word, and none follows it.
fn is_final_cased(src: &[u16], index: usize, locale: &Locale) -> bool {
    let mut word_boundary = break_iterator::get_word_instance(locale);
    word_boundary.set_text_units(src);
    let mut is_boundary = |offset: usize| {
        word_boundary
            .is_boundary(offset as i32)
            .expect("INVARIANT: the offset lies within the text")
    };

    // Look for a preceding 'cased' letter
    let mut i = index;
    while !is_boundary(i) {
        let ch = code_point_before(src, i);
        if is_cased(ch) {
            let len = src.len();
            // Check that there is no 'cased' letter after the index
            let mut i = index + char_count(code_point_at(src, index, len));
            while i < len && !is_boundary(i) {
                let ch = code_point_at(src, i, len);
                if is_cased(ch) {
                    return false;
                }
                i += char_count(ch);
            }

            return true;
        }
        i -= char_count(ch);
    }

    false
}

/// `String.codePointBefore(int)`.
fn code_point_before(s: &[u16], index: usize) -> i32 {
    let c2 = s[index - 1];
    if is_low_surrogate(c2) && index >= 2 {
        let c1 = s[index - 2];
        if is_high_surrogate(c1) {
            return ((i32::from(c1) - 0xD800) << 10) + (i32::from(c2) - 0xDC00) + 0x10000;
        }
    }
    i32::from(c2)
}

/// `ConditionalSpecialCasing.isCased(int)`.
fn is_cased(ch: i32) -> bool {
    let r#type = get_type(ch);
    if r#type == LOWERCASE_LETTER || r#type == UPPERCASE_LETTER || r#type == TITLECASE_LETTER {
        true
    } else {
        // Check for Other_Lowercase and Other_Uppercase
        //
        // MODIFIER LETTER SMALL H..MODIFIER LETTER SMALL Y
        (0x02B0..=0x02B8).contains(&ch)
            // MODIFIER LETTER GLOTTAL STOP..MODIFIER LETTER REVERSED GLOTTAL STOP
            || (0x02C0..=0x02C1).contains(&ch)
            // MODIFIER LETTER SMALL GAMMA..MODIFIER LETTER SMALL REVERSED GLOTTAL STOP
            || (0x02E0..=0x02E4).contains(&ch)
            // COMBINING GREEK YPOGEGRAMMENI
            || ch == 0x0345
            // GREEK YPOGEGRAMMENI
            || ch == 0x037A
            // MODIFIER LETTER CAPITAL A..MODIFIER LETTER SMALL CHI
            || (0x1D2C..=0x1D61).contains(&ch)
            // ROMAN NUMERAL ONE..ROMAN NUMERAL ONE THOUSAND
            // SMALL ROMAN NUMERAL ONE..SMALL ROMAN NUMERAL ONE THOUSAND
            || (0x2160..=0x217F).contains(&ch)
            // CIRCLED LATIN CAPITAL LETTER A..CIRCLED LATIN CAPITAL LETTER Z
            // CIRCLED LATIN SMALL LETTER A..CIRCLED LATIN SMALL LETTER Z
            || (0x24B6..=0x24E9).contains(&ch)
    }
}

/// `Character.isSurrogate(char)`.
fn is_surrogate(unit: u16) -> bool {
    (0xD800..=0xDFFF).contains(&unit)
}
