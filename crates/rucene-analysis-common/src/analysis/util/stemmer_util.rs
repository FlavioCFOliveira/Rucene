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

//! Some commonly-used stemming functions, ported from
//! `org.apache.lucene.analysis.util.StemmerUtil`.
//!
//! Java's methods take a `char[]` term buffer, UTF-16 units; these take the
//! same units as a `[u16]` buffer, and compare against a string's UTF-16
//! units, as `String.charAt` reads them. Inside this crate, the body of
//! [`StemmerUtil::ends_with`] (`ends_with_units`) also takes the `char` term
//! buffer of `rucene`'s `CharTermAttribute` when every character in it is
//! one UTF-16 unit (`TermUnit`), so that a stemmer can edit the term in
//! place as Java's does. [`StemmerUtil::delete`] and
//! [`StemmerUtil::delete_n`] only move elements, so they take a buffer of
//! any element type: the UTF-16 units of a stemmer, or the `char` term
//! buffer of the filters that edit `rucene`'s `CharTermAttribute` in place
//! (`CJKWidthFilter`, `ScandinavianNormalizer`,
//! `ScandinavianFoldingFilter`).

/// An element of a term buffer that a stemmer reads and writes as Java's
/// `char`, a UTF-16 code unit.
///
/// Java has one such type; the port has two, because `rucene`'s
/// `CharTermAttribute` holds `char`s where Java's holds UTF-16 units (the
/// divergence declared on that trait):
///
/// * `u16`, the unit itself;
/// * `char`, the element of the term buffer, which is the same unit for
///   every character of the Basic Multilingual Plane. A caller passes a
///   `char` buffer only when every character in it is in that plane, so
///   that indices and lengths count the same units as Java's; a character
///   beyond it has a [`code`](Self::code) above `0xFFFF`, which equals no
///   unit, so a comparison with a unit is false.
///
/// No Lucene counterpart: it lets one stemmer body serve both buffers, and
/// is private to this crate, so the public `StemmerUtil` keeps Lucene's
/// signatures.
pub(crate) trait TermUnit: Copy {
    /// The element's value: the unit (or the code point of a `char`).
    fn code(self) -> u32;

    /// The element holding the unit of `c`, a character of the Basic
    /// Multilingual Plane (the `char` literals of the Java stemmers).
    fn from_bmp(c: char) -> Self;
}

impl TermUnit for u16 {
    #[inline]
    fn code(self) -> u32 {
        u32::from(self)
    }

    #[inline]
    fn from_bmp(c: char) -> Self {
        debug_assert!((c as u32) < 0x1_0000, "{c:?} is not in the BMP");
        c as u16
    }
}

impl TermUnit for char {
    #[inline]
    fn code(self) -> u32 {
        self as u32
    }

    #[inline]
    fn from_bmp(c: char) -> Self {
        debug_assert!((c as u32) < 0x1_0000, "{c:?} is not in the BMP");
        c
    }
}

/// Some commonly-used stemming functions.
///
/// Equivalent to `org.apache.lucene.analysis.util.StemmerUtil`, a class of
/// static methods with a private constructor ("no instance"): the type has
/// no value.
#[derive(Debug)]
pub enum StemmerUtil {}

impl StemmerUtil {
    /// Returns true if the character array starts with the prefix.
    ///
    /// Equivalent to `StemmerUtil.startsWith(char[], int, String)`: `s` is
    /// the input buffer, `len` its length, `prefix` the prefix to test.
    pub fn starts_with(s: &[u16], len: usize, prefix: &str) -> bool {
        // `prefixLen > len`. A string has at most as many UTF-16 units as
        // UTF-8 bytes, so the units are counted only when the bytes exceed
        // `len`.
        // PERF: in the common case the prefix is encoded once, by the
        // comparison below, where counting its units first cost a second
        // pass on every call (task #546, round 4).
        if prefix.len() > len && prefix.encode_utf16().count() > len {
            return false;
        }
        for (i, c) in prefix.encode_utf16().enumerate() {
            if s[i] != c {
                return false;
            }
        }
        true
    }

    /// Returns true if the character array ends with the suffix.
    ///
    /// Equivalent to `StemmerUtil.endsWith(char[], int, String)`: `s` is the
    /// input buffer, `len` its length, `suffix` the suffix to test, compared
    /// from its last character backwards.
    pub fn ends_with(s: &[u16], len: usize, suffix: &str) -> bool {
        Self::ends_with_units(s, len, suffix)
    }

    /// The body of [`ends_with`](Self::ends_with) over a buffer of UTF-16
    /// units or of `char`s that are each one unit (see [`TermUnit`]).
    // PERF: inlined so that a stemmer's constant suffix folds into a few
    // comparisons; called out of line, each of the ~20 calls per token walked
    // the suffix's bytes (profile of task #546, round 4). A plain `#[inline]`
    // left every call out of line.
    #[inline(always)]
    pub(crate) fn ends_with_units<T: TermUnit>(s: &[T], len: usize, suffix: &str) -> bool {
        // `suffixLen > len`. A string has at most as many UTF-16 units as
        // UTF-8 bytes, so the units are counted only when the bytes exceed
        // `len`.
        if suffix.len() > len && suffix.encode_utf16().count() > len {
            return false;
        }
        // PERF: compares from the last unit backwards, as Java does, reading
        // the suffix's units in that order: a mismatch on the last unit, the
        // common case, ends the call after one comparison, where encoding
        // the whole suffix first (to count it, then to compare it forwards)
        // cost a pass over every unit on every call (profile of task #546,
        // round 3).
        // An ASCII byte is its own unit; a non-ASCII character is decoded
        // and encoded.
        let bytes = suffix.as_bytes();
        let mut end = len;
        let mut i = bytes.len();
        while i > 0 {
            let byte = bytes[i - 1];
            if byte.is_ascii() {
                end -= 1;
                if s[end].code() != u32::from(byte) {
                    return false;
                }
                i -= 1;
            } else {
                let c = suffix[..i]
                    .chars()
                    .next_back()
                    .expect("INVARIANT: a character ends at every boundary");
                let mut units = [0u16; 2];
                for &unit in c.encode_utf16(&mut units).iter().rev() {
                    end -= 1;
                    if s[end].code() != u32::from(unit) {
                        return false;
                    }
                }
                i -= c.len_utf8();
            }
        }
        true
    }

    /// Returns true if the character array ends with the suffix.
    ///
    /// Equivalent to `StemmerUtil.endsWith(char[], int, char[])`.
    pub fn ends_with_chars(s: &[u16], len: usize, suffix: &[u16]) -> bool {
        let suffix_len = suffix.len();
        if suffix_len > len {
            return false;
        }
        for i in (0..suffix_len).rev() {
            if s[len - (suffix_len - i)] != suffix[i] {
                return false;
            }
        }

        true
    }

    /// Delete a character in-place.
    ///
    /// Equivalent to `StemmerUtil.delete(char[], int, int)`: removes the
    /// character at `pos` from the first `len` of `s` and returns the new
    /// length.
    pub fn delete<T: Copy>(s: &mut [T], pos: usize, len: usize) -> usize {
        debug_assert!(pos < len);
        if pos < len - 1 {
            // don't arraycopy if asked to delete last character
            s.copy_within(pos + 1..len, pos);
        }
        len - 1
    }

    /// Delete n characters in-place.
    ///
    /// Equivalent to `StemmerUtil.deleteN(char[], int, int, int)`: removes
    /// `n_chars` characters at `pos` from the first `len` of `s` and
    /// returns the new length.
    pub fn delete_n<T: Copy>(s: &mut [T], pos: usize, len: usize, n_chars: usize) -> usize {
        debug_assert!(pos + n_chars <= len);
        if pos + n_chars < len {
            // don't arraycopy if asked to delete the last characters
            s.copy_within(pos + n_chars..len, pos);
        }
        len - n_chars
    }
}

#[cfg(test)]
mod tests {
    use super::StemmerUtil;

    /// `StemmerUtil.endsWith(char[], int, String)` as Java writes it: the
    /// suffix's UTF-16 length first, then its units from the last backwards.
    fn java_ends_with(s: &[u16], len: usize, suffix: &str) -> bool {
        let suffix: Vec<u16> = suffix.encode_utf16().collect();
        if suffix.len() > len {
            return false;
        }
        (0..suffix.len())
            .rev()
            .all(|i| s[len - (suffix.len() - i)] == suffix[i])
    }

    /// `ends_with` answers as Java's `endsWith` for ASCII, Latin-1 and
    /// supplementary suffixes, at every length of the buffer, including
    /// suffixes longer than the length and buffers longer than the length.
    #[test]
    fn ends_with_answers_as_java() {
        let words = [
            "",
            "s",
            "es",
            "papéis",
            "ações",
            "pães",
            "anzóis",
            "rapidamente",
            "x😀",
            "😀",
            "a😀a",
            "éis",
            "mente",
        ];
        let suffixes = [
            "", "s", "es", "is", "éis", "óis", "ões", "ães", "mente", "😀", "x😀", "😀a",
        ];
        let mut matched = 0;
        for word in words {
            let s: Vec<u16> = word.encode_utf16().collect();
            for len in 0..=s.len() {
                for suffix in suffixes {
                    let expected = java_ends_with(&s, len, suffix);
                    assert_eq!(
                        StemmerUtil::ends_with(&s, len, suffix),
                        expected,
                        "{word:?}[..{len}] ends with {suffix:?}"
                    );
                    matched += usize::from(expected && !suffix.is_empty());
                }
            }
        }
        assert!(matched > 20, "only {matched} non-empty suffixes matched");
    }

    /// `StemmerUtil.startsWith(char[], int, String)` as Java writes it: the
    /// prefix's UTF-16 length first, then its units from the first forwards.
    fn java_starts_with(s: &[u16], len: usize, prefix: &str) -> bool {
        let prefix: Vec<u16> = prefix.encode_utf16().collect();
        if prefix.len() > len {
            return false;
        }
        (0..prefix.len()).all(|i| s[i] == prefix[i])
    }

    /// `starts_with` answers as Java's `startsWith` for ASCII, Latin-1 and
    /// supplementary prefixes, at every length of the buffer, including
    /// prefixes longer than the length (in units, or only in UTF-8 bytes)
    /// and buffers longer than the length.
    #[test]
    fn starts_with_answers_as_java() {
        let words = [
            "", "m", "me", "mente", "ações", "éis", "😀", "😀a", "a😀", "ber", "peng", "menye",
        ];
        let prefixes = [
            "", "m", "me", "men", "mente", "a", "aç", "ações", "é", "éi", "😀", "😀a", "be", "peng",
        ];
        let mut matched = 0;
        for word in words {
            let s: Vec<u16> = word.encode_utf16().collect();
            for len in 0..=s.len() {
                for prefix in prefixes {
                    let expected = java_starts_with(&s, len, prefix);
                    assert_eq!(
                        StemmerUtil::starts_with(&s, len, prefix),
                        expected,
                        "{word:?}[..{len}] starts with {prefix:?}"
                    );
                    matched += usize::from(expected && !prefix.is_empty());
                }
            }
        }
        assert!(matched > 20, "only {matched} non-empty prefixes matched");
    }

    /// Over a `char` buffer whose characters are each one UTF-16 unit,
    /// `ends_with` answers as over the units of the same term, for every
    /// length and every suffix (supplementary suffixes included, which
    /// match neither).
    #[test]
    fn ends_with_over_chars_answers_as_over_units() {
        let words = [
            "",
            "s",
            "papéis",
            "ações",
            "pães",
            "anzóis",
            "rapidamente",
            "éis",
            "mente",
            "ÿĀ",
        ];
        let suffixes = [
            "", "s", "es", "is", "éis", "óis", "ões", "ães", "mente", "😀", "x😀", "ĀÿĀ", "Ā",
        ];
        for word in words {
            let units: Vec<u16> = word.encode_utf16().collect();
            let chars: Vec<char> = word.chars().collect();
            assert_eq!(units.len(), chars.len(), "{word:?} is in the BMP");
            for len in 0..=units.len() {
                for suffix in suffixes {
                    assert_eq!(
                        StemmerUtil::ends_with_units(&chars, len, suffix),
                        StemmerUtil::ends_with(&units, len, suffix),
                        "{word:?}[..{len}] ends with {suffix:?}"
                    );
                }
            }
        }
    }
}
