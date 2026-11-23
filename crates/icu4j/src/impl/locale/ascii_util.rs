// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.locale.AsciiUtil`, with the UTF-16 index helpers the
//! locale classes share.
//!
//! The locale classes take locale identifiers, tags, keys and types as
//! Rust strings (crate convention, rmp #643 DECISION: texts that only name
//! things); where Java exposes or consumes a `String` index or length, the
//! port counts UTF-16 units with the helpers below, so that the indices are
//! Java's.

use std::cmp::Ordering;
use std::hash::{Hash, Hasher};

use rucene::util::java_collections::{java_compare_to, java_string_hash_code};

/// `String.length()`: the number of UTF-16 units.
pub fn len16(s: &str) -> i32 {
    s.encode_utf16().count() as i32
}

/// The byte offset of the UTF-16 index `index` of `s`.
///
/// # Panics
///
/// `StringIndexOutOfBoundsException` when `index` is outside `0..=len16(s)`
/// or splits a surrogate pair.
pub fn byte_offset16(s: &str, index: i32) -> usize {
    if index < 0 {
        crate::jdk::string_index_out_of_bounds(index);
    }
    let mut units = 0i32;
    for (b, c) in s.char_indices() {
        if units == index {
            return b;
        }
        units += c.len_utf16() as i32;
        if units > index {
            crate::jdk::string_index_out_of_bounds(index);
        }
    }
    if units == index {
        return s.len();
    }
    crate::jdk::string_index_out_of_bounds(index)
}

/// `String.substring(begin, end)` over UTF-16 indices.
///
/// # Panics
///
/// `StringIndexOutOfBoundsException` for indices outside the string or
/// `begin > end`.
pub fn substring16(s: &str, begin: i32, end: i32) -> String {
    if begin > end {
        crate::jdk::string_index_out_of_bounds(end - begin);
    }
    s[byte_offset16(s, begin)..byte_offset16(s, end)].to_string()
}

/// `String.substring(begin)` over UTF-16 indices.
///
/// # Panics
///
/// `StringIndexOutOfBoundsException` for `begin` outside the string.
pub fn substring16_from(s: &str, begin: i32) -> String {
    s[byte_offset16(s, begin)..].to_string()
}

/// `String.charAt(index)`: the UTF-16 unit at `index`.
///
/// # Panics
///
/// `StringIndexOutOfBoundsException` for `index` outside the string.
pub fn char_at16(s: &str, index: i32) -> u16 {
    if index < 0 {
        crate::jdk::string_index_out_of_bounds(index);
    }
    match s.encode_utf16().nth(index as usize) {
        Some(c) => c,
        None => crate::jdk::string_index_out_of_bounds(index),
    }
}

/// `String.indexOf(String str, int fromIndex)` over UTF-16 indices.
pub fn index_of16(s: &str, target: &str, from_index: i32) -> i32 {
    let su: Vec<u16> = s.encode_utf16().collect();
    let tu: Vec<u16> = target.encode_utf16().collect();
    crate::jdk::index_of(&su, &tu, from_index)
}

/// `String.lastIndexOf(String str)` over UTF-16 indices.
pub fn last_index_of16(s: &str, target: &str) -> i32 {
    let su: Vec<u16> = s.encode_utf16().collect();
    let tu: Vec<u16> = target.encode_utf16().collect();
    crate::jdk::last_index_of(&su, &tu, su.len() as i32)
}

/// `String.hashCode()`.
pub fn string_hash(s: &str) -> i32 {
    java_string_hash_code(s)
}

/// `String.compareTo(String)`: the UTF-16 order, as an `int` sign.
pub fn string_compare(a: &str, b: &str) -> i32 {
    match java_compare_to(a, b) {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }
}

/// `String.toUpperCase()` (the default locale) of a string whose letters
/// are ASCII: `i` maps to `U+0130` when the default locale's language is
/// Turkish or Azerbaijani, as the JDK's locale-sensitive case mapping does;
/// every other unit maps as `Character.toUpperCase`.
pub fn to_upper_case_default_locale(s: &str) -> String {
    let lang = rucene_queryparser::jdk::locale::Locale::get_default()
        .get_language()
        .to_string();
    let turkic = lang == "tr" || lang == "az";
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if turkic && c == 'i' {
            out.push('\u{130}');
        } else {
            out.extend(c.to_uppercase());
        }
    }
    out
}

/// `String.toLowerCase()` (the default locale) of a string whose letters
/// are ASCII: `I` maps to `U+0131` when the default locale's language is
/// Turkish or Azerbaijani, as the JDK's locale-sensitive case mapping does;
/// every other unit maps as `Character.toLowerCase`.
pub fn to_lower_case_default_locale(s: &str) -> String {
    let lang = rucene_queryparser::jdk::locale::Locale::get_default()
        .get_language()
        .to_string();
    let turkic = lang == "tr" || lang == "az";
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if turkic && c == 'I' {
            out.push('\u{131}');
        } else {
            out.extend(c.to_lowercase());
        }
    }
    out
}

/// ASCII-only case and class utilities for locale identifiers.
///
/// Equivalent to `com.ibm.icu.impl.locale.AsciiUtil`. Java compares
/// `char`s; only ASCII letters change case, so the per-`char` loops of the
/// port give the same answers over the same strings.
pub struct AsciiUtil;

impl AsciiUtil {
    /// `caseIgnoreMatch(String s1, String s2)`.
    pub fn case_ignore_match(s1: &str, s2: &str) -> bool {
        let mut a = s1.encode_utf16();
        let mut b = s2.encode_utf16();
        loop {
            match (a.next(), b.next()) {
                (None, None) => return true,
                (Some(c1), Some(c2)) => {
                    if c1 != c2 && Self::to_lower16(c1) != Self::to_lower16(c2) {
                        return false;
                    }
                }
                _ => return false,
            }
        }
    }

    /// `caseIgnoreCompare(String s1, String s2)`.
    pub fn case_ignore_compare(s1: &str, s2: &str) -> i32 {
        string_compare(&Self::to_lower_string(s1), &Self::to_lower_string(s2))
    }

    /// `toUpper(char c)`.
    pub fn to_upper(c: char) -> char {
        c.to_ascii_uppercase()
    }

    /// `toLower(char c)`.
    pub fn to_lower(c: char) -> char {
        c.to_ascii_lowercase()
    }

    /// `toLower(char c)` over a UTF-16 unit.
    pub fn to_lower16(c: u16) -> u16 {
        if (b'A' as u16..=b'Z' as u16).contains(&c) {
            c + 0x20
        } else {
            c
        }
    }

    /// `toUpper(char c)` over a UTF-16 unit.
    pub fn to_upper16(c: u16) -> u16 {
        if (b'a' as u16..=b'z' as u16).contains(&c) {
            c - 0x20
        } else {
            c
        }
    }

    /// `toLowerString(String s)`.
    pub fn to_lower_string(s: &str) -> String {
        s.to_ascii_lowercase()
    }

    /// `toUpperString(String s)`.
    pub fn to_upper_string(s: &str) -> String {
        s.to_ascii_uppercase()
    }

    /// `toTitleString(String s)`.
    ///
    /// Java's loop tests the first character for every index (a long-lived
    /// ICU quirk), so a string whose first character is not a lower-case
    /// ASCII letter is returned unchanged when it is not an upper-case one,
    /// and otherwise lower-cased after its first character.
    pub fn to_title_string(s: &str) -> String {
        let n = len16(s);
        if n == 0 {
            return s.to_string();
        }
        let units: Vec<u16> = s.encode_utf16().collect();
        let mut idx = 0usize;
        let c = units[idx];
        if !(b'a' as u16..=b'z' as u16).contains(&c) {
            idx = 1;
            while idx < units.len() {
                if (b'A' as u16..=b'Z' as u16).contains(&c) {
                    break;
                }
                idx += 1;
            }
        }
        if idx == units.len() {
            return s.to_string();
        }
        let mut buf: Vec<u16> = units[..idx].to_vec();
        if idx == 0 {
            buf.push(Self::to_upper16(units[idx]));
            idx += 1;
        }
        while idx < units.len() {
            buf.push(Self::to_lower16(units[idx]));
            idx += 1;
        }
        // Only ASCII letters changed: the units stay a valid encoding.
        String::from_utf16(&buf).expect("ASCII case mapping keeps UTF-16 valid")
    }

    /// `isAlpha(char c)`.
    pub fn is_alpha(c: char) -> bool {
        c.is_ascii_alphabetic()
    }

    /// `isAlphaString(String s)`.
    pub fn is_alpha_string(s: &str) -> bool {
        s.bytes().all(|b| b.is_ascii_alphabetic())
    }

    /// `isNumeric(char c)`.
    pub fn is_numeric(c: char) -> bool {
        c.is_ascii_digit()
    }

    /// `isNumericString(String s)`.
    pub fn is_numeric_string(s: &str) -> bool {
        s.bytes().all(|b| b.is_ascii_digit())
    }

    /// `isAlphaNumeric(char c)`.
    pub fn is_alpha_numeric(c: char) -> bool {
        c.is_ascii_alphanumeric()
    }

    /// `isAlphaNumericString(String s)`.
    pub fn is_alpha_numeric_string(s: &str) -> bool {
        s.bytes().all(|b| b.is_ascii_alphanumeric())
    }
}

/// A string key compared ignoring ASCII case.
///
/// Equivalent to `com.ibm.icu.impl.locale.AsciiUtil.CaseInsensitiveKey`:
/// `equals` is `caseIgnoreMatch`, `hashCode` the hash of the lower-cased
/// key.
#[derive(Debug, Clone)]
pub struct CaseInsensitiveKey {
    key: String,
    hash: i32,
}

impl CaseInsensitiveKey {
    /// `new CaseInsensitiveKey(String key)`.
    pub fn new(key: &str) -> Self {
        Self {
            key: key.to_string(),
            hash: string_hash(&AsciiUtil::to_lower_string(key)),
        }
    }

    /// `hashCode()`.
    pub fn hash_code(&self) -> i32 {
        self.hash
    }
}

impl PartialEq for CaseInsensitiveKey {
    fn eq(&self, other: &Self) -> bool {
        AsciiUtil::case_ignore_match(&self.key, &other.key)
    }
}

impl Eq for CaseInsensitiveKey {}

impl Hash for CaseInsensitiveKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash.hash(state);
    }
}
