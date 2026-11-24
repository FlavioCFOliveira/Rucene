// © 2017 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html

//! `com.ibm.icu.impl.StringSegment`.

use crate::jdk::{
    char_count, code_point_at, is_high_surrogate, is_low_surrogate, to_code_point, to_rust_string,
};
use crate::lang::u_character::UCharacter;
use crate::r#impl::utility::Utility;
use crate::text::unicode_set::UnicodeSet;

/// A mutable view of a string with an adjustable start offset and length,
/// used by the number parsers.
///
/// Equivalent to `com.ibm.icu.impl.StringSegment`, which implements
/// `CharSequence`: indexes given to [`char_at`](Self::char_at),
/// [`sub_sequence`](Self::sub_sequence) and
/// [`code_point_at`](Self::code_point_at) are relative to the offset. The
/// string (`str`, Java's immutable `String`) is held as its UTF-16 units.
#[derive(Debug, Clone)]
pub struct StringSegment {
    str: Vec<u16>,
    start: i32,
    end: i32,
    fold_case: bool,
}

/// `Character.codePointAt(CharSequence seq, int index)` over the first
/// `len` units of `seq` starting at `base`.
fn seq_code_point_at(seq: &[u16], base: i32, len: i32, index: i32) -> i32 {
    let c1 = seq[(base + index) as usize];
    if is_high_surrogate(c1) && index + 1 < len {
        let c2 = seq[(base + index + 1) as usize];
        if is_low_surrogate(c2) {
            return to_code_point(c1, c2);
        }
    }
    c1 as i32
}

impl StringSegment {
    /// `new StringSegment(String str, boolean foldCase)`.
    pub fn new(str: &[u16], fold_case: bool) -> Self {
        Self {
            str: str.to_vec(),
            start: 0,
            end: str.len() as i32,
            fold_case,
        }
    }

    /// `getOffset()`.
    pub fn get_offset(&self) -> i32 {
        self.start
    }

    /// `setOffset(int start)`.
    pub fn set_offset(&mut self, start: i32) {
        debug_assert!(start <= self.end);
        self.start = start;
    }

    /// `adjustOffset(int delta)`: moves the offset by `delta` units.
    pub fn adjust_offset(&mut self, delta: i32) {
        debug_assert!(self.start + delta >= 0);
        debug_assert!(self.start + delta <= self.end);
        self.start += delta;
    }

    /// `adjustOffsetByCodePoint()`: moves the offset past the code point
    /// at the offset.
    pub fn adjust_offset_by_code_point(&mut self) {
        self.start += char_count(self.get_code_point());
    }

    /// `setLength(int length)`: sets the end to `offset + length`.
    pub fn set_length(&mut self, length: i32) {
        debug_assert!(length >= 0);
        debug_assert!(self.start + length <= self.str.len() as i32);
        self.end = self.start + length;
    }

    /// `resetLength()`: sets the end to the end of the string.
    pub fn reset_length(&mut self) {
        self.end = self.str.len() as i32;
    }

    /// `length()`.
    pub fn length(&self) -> i32 {
        self.end - self.start
    }

    /// `charAt(int index)`.
    ///
    /// # Panics
    ///
    /// `StringIndexOutOfBoundsException` outside the string.
    pub fn char_at(&self, index: i32) -> u16 {
        match self.str.get((index + self.start) as usize) {
            Some(&c) if index + self.start >= 0 => c,
            _ => crate::jdk::string_index_out_of_bounds(index + self.start),
        }
    }

    /// `subSequence(int start, int end)`: the units between the two
    /// relative indexes.
    pub fn sub_sequence(&self, start: i32, end: i32) -> Vec<u16> {
        self.str[(start + self.start) as usize..(end + self.start) as usize].to_vec()
    }

    /// `getCodePoint()`: the code point at the offset, a surrogate pair
    /// counting only when both halves are before the end.
    pub fn get_code_point(&self) -> i32 {
        debug_assert!(self.start < self.end);
        let lead = self.str[self.start as usize];
        if is_high_surrogate(lead) && self.start + 1 < self.end {
            let trail = self.str[(self.start + 1) as usize];
            if is_low_surrogate(trail) {
                return to_code_point(lead, trail);
            }
        }
        lead as i32
    }

    /// `codePointAt(int index)`: `String.codePointAt(start + index)` over
    /// the whole string (not bounded by the end).
    pub fn code_point_at(&self, index: i32) -> i32 {
        code_point_at(&self.str, self.start + index)
    }

    /// `startsWith(int otherCp)`: whether the code point at the offset
    /// equals `other_cp` (case-folded when folding).
    pub fn starts_with_code_point(&self, other_cp: i32) -> bool {
        Self::code_points_equal(self.get_code_point(), other_cp, self.fold_case)
    }

    /// `startsWith(UnicodeSet uniset)`: whether `uniset` contains the code
    /// point at the offset (never folded).
    pub fn starts_with_set(&self, uniset: &UnicodeSet) -> bool {
        // TODO: Move UnicodeSet case-folding logic here.
        // TODO: Handle string matches here instead of separately.
        let cp = self.get_code_point();
        if cp == -1 {
            return false;
        }
        uniset.contains_valid(cp)
    }

    /// `startsWith(CharSequence other)`: whether the first code points of
    /// this segment and `other` are equal (case-folded when folding);
    /// `false` when either is empty. Java's `null` `other` is `None`.
    pub fn starts_with(&self, other: Option<&[u16]>) -> bool {
        let other = match other {
            Some(o) if !o.is_empty() && self.length() != 0 => o,
            _ => return false,
        };
        let cp1 = seq_code_point_at(&self.str, self.start, self.length(), 0);
        let cp2 = seq_code_point_at(other, 0, other.len() as i32, 0);
        Self::code_points_equal(cp1, cp2, self.fold_case)
    }

    /// `getCommonPrefixLength(CharSequence other)`: the length in units of
    /// the common prefix (folded when folding).
    pub fn get_common_prefix_length(&self, other: &[u16]) -> i32 {
        self.get_prefix_length_internal(other, self.fold_case)
    }

    /// `getCaseSensitivePrefixLength(CharSequence other)`.
    pub fn get_case_sensitive_prefix_length(&self, other: &[u16]) -> i32 {
        self.get_prefix_length_internal(other, false)
    }

    fn get_prefix_length_internal(&self, other: &[u16], fold_case: bool) -> i32 {
        debug_assert!(!other.is_empty());
        let mut offset = 0;
        let len = self.length();
        let olen = other.len() as i32;
        while offset < len.min(olen) {
            // TODO: case-fold code points, not chars
            let cp1 = seq_code_point_at(&self.str, self.start, len, offset);
            let cp2 = seq_code_point_at(other, 0, olen, offset);
            if !Self::code_points_equal(cp1, cp2, fold_case) {
                break;
            }
            offset += char_count(cp1);
        }
        offset
    }

    fn code_points_equal(cp1: i32, cp2: i32, fold_case: bool) -> bool {
        if cp1 == cp2 {
            return true;
        }
        if !fold_case {
            return false;
        }
        UCharacter::fold_case_cp(cp1, true) == UCharacter::fold_case_cp(cp2, true)
    }

    /// `contentEquals(CharSequence other)`: whether the segment's units
    /// equal `other`'s.
    pub fn content_equals(&self, other: Option<&[u16]>) -> bool {
        Utility::char_sequence_equals(
            Some(&self.str[self.start as usize..self.end as usize]),
            other,
        )
    }

    /// `asString()`: the segment's units.
    pub fn as_string(&self) -> Vec<u16> {
        self.str[self.start as usize..self.end as usize].to_vec()
    }

    /// `toString()`: the string with the segment in brackets.
    pub fn to_java_string(&self) -> Vec<u16> {
        let mut s = self.str[..self.start as usize].to_vec();
        s.push(b'[' as u16);
        s.extend_from_slice(&self.str[self.start as usize..self.end as usize]);
        s.push(b']' as u16);
        s.extend_from_slice(&self.str[self.end as usize..]);
        s
    }
}

impl std::fmt::Display for StringSegment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&to_rust_string(&self.to_java_string()))
    }
}
