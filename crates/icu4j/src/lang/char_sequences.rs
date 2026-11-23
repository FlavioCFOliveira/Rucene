// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.lang.CharSequences`.

use rucene::error::Result;

use crate::jdk::{code_point_at, is_high_surrogate, is_low_surrogate, char_count, to_code_point};

/// A set of utilities for handling `CharSequence`s (UTF-16 code units).
///
/// Equivalent to `com.ibm.icu.lang.CharSequences` (deprecated internal).
pub struct CharSequences;

impl CharSequences {
    /// `matchAfter(CharSequence a, CharSequence b, int aIndex, int bIndex)`:
    /// the length of the common prefix after the indexes, not splitting a
    /// surrogate pair.
    pub fn match_after(a: &[u16], b: &[u16], a_index: i32, b_index: i32) -> i32 {
        let mut i = a_index;
        let mut j = b_index;
        let alen = a.len() as i32;
        let blen = b.len() as i32;
        while i < alen && j < blen {
            let ca = a[i as usize];
            let cb = b[j as usize];
            if ca != cb {
                break;
            }
            i += 1;
            j += 1;
        }
        // if we failed a match make sure that we didn't match half a character
        let mut result = i - a_index;
        if result != 0 && !Self::on_character_boundary(a, i) && !Self::on_character_boundary(b, j) {
            result -= 1; // backup
        }
        result
    }

    /// `codePointLength(CharSequence s)` (an instance method in Java).
    pub fn code_point_length(&self, s: &[u16]) -> i32 {
        let mut n = 0;
        let mut i = 0;
        while i < s.len() as i32 {
            i += char_count(code_point_at(s, i));
            n += 1;
        }
        n
    }

    /// `equals(int codepoint, CharSequence other)`.
    pub fn equals_cp(codepoint: i32, other: Option<&[u16]>) -> bool {
        let Some(other) = other else { return false };
        match other.len() {
            1 => codepoint == other[0] as i32,
            2 => codepoint > 0xFFFF && codepoint == code_point_at(other, 0),
            _ => false,
        }
    }

    /// `compare(CharSequence string, int codePoint)`: as if `codePoint`
    /// were a one-code-point string.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an invalid code point.
    pub fn compare_cp(string: &[u16], code_point: i32) -> Result<i32> {
        if !(0..=0x10FFFF).contains(&code_point) {
            return Err(crate::jdk::illegal_argument_no_message());
        }
        let string_length = string.len() as i32;
        if string_length == 0 {
            return Ok(-1);
        }
        let first_char = string[0];
        let offset = code_point - 0x10000;
        if offset < 0 {
            // BMP codePoint
            let result = first_char as i32 - code_point;
            if result != 0 {
                return Ok(result);
            }
            return Ok(string_length - 1);
        }
        // non BMP
        let lead = (((offset as u32) >> 10) + 0xD800) as u16;
        let result = first_char as i32 - lead as i32;
        if result != 0 {
            return Ok(result);
        }
        if string_length > 1 {
            let trail = ((offset & 0x3ff) + 0xDC00) as u16;
            let result = string[1] as i32 - trail as i32;
            if result != 0 {
                return Ok(result);
            }
        }
        Ok(string_length - 2)
    }

    /// `compare(int codepoint, CharSequence a)`.
    ///
    /// # Errors
    ///
    /// As [`compare_cp`](Self::compare_cp).
    pub fn compare_cp_rev(codepoint: i32, a: &[u16]) -> Result<i32> {
        let result = Self::compare_cp(a, codepoint)?;
        Ok(if result > 0 { -1 } else if result < 0 { 1 } else { 0 })
    }

    /// `getSingleCodePoint(CharSequence s)`: the single code point of
    /// `s`, or `Integer.MAX_VALUE`.
    pub fn get_single_code_point(s: &[u16]) -> i32 {
        let length = s.len();
        if !(1..=2).contains(&length) {
            return i32::MAX;
        }
        let result = code_point_at(s, 0);
        if (result < 0x10000) == (length == 1) {
            result
        } else {
            i32::MAX
        }
    }

    /// `equals(T a, T b)`: null-safe equality.
    pub fn equals<T: PartialEq + ?Sized>(a: Option<&T>, b: Option<&T>) -> bool {
        match (a, b) {
            (None, None) => true,
            (Some(a), Some(b)) => a == b,
            _ => false,
        }
    }

    /// `compare(CharSequence a, CharSequence b)`: code-unit order.
    pub fn compare(a: &[u16], b: &[u16]) -> i32 {
        let alength = a.len();
        let blength = b.len();
        let min = alength.min(blength);
        for i in 0..min {
            let diff = a[i] as i32 - b[i] as i32;
            if diff != 0 {
                return diff;
            }
        }
        alength as i32 - blength as i32
    }

    /// `equalsChars(CharSequence a, CharSequence b)`.
    pub fn equals_chars(a: &[u16], b: &[u16]) -> bool {
        // do length test first for fast path
        a.len() == b.len() && Self::compare(a, b) == 0
    }

    /// `onCharacterBoundary(CharSequence s, int i)`.
    pub fn on_character_boundary(s: &[u16], i: i32) -> bool {
        i <= 0
            || i >= s.len() as i32
            || !is_high_surrogate(s[i as usize - 1])
            || !is_low_surrogate(s[i as usize])
    }

    /// `indexOf(CharSequence s, int codePoint)`.
    pub fn index_of(s: &[u16], code_point: i32) -> i32 {
        let mut i = 0;
        while i < s.len() as i32 {
            let cp = code_point_at(s, i);
            if cp == code_point {
                return i;
            }
            i += char_count(cp);
        }
        -1
    }

    /// `codePoints(CharSequence s)`: the code points, unpaired surrogates
    /// kept as they are.
    pub fn code_points(s: &[u16]) -> Vec<i32> {
        let mut result = vec![0i32; s.len()]; // in the vast majority of cases, the length will be the same
        let mut j = 0usize;
        for i in 0..s.len() {
            let cp = s[i];
            if (0xDC00..=0xDFFF).contains(&cp) && i != 0 {
                // hand-code for speed
                let last = result[j - 1] as u16;
                if (0xD800..=0xDBFF).contains(&last) {
                    // Note: j-1 is safe, because j can only be zero if i is zero. But i!=0 in this block.
                    result[j - 1] = to_code_point(last, cp);
                    continue;
                }
            }
            result[j] = cp as i32;
            j += 1;
        }
        result.truncate(j);
        result
    }
}
