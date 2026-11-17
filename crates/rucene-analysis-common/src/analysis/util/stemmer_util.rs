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
//! units, as `String.charAt` reads them. [`StemmerUtil::delete`] and
//! [`StemmerUtil::delete_n`] only move elements, so they take a buffer of
//! any element type: the UTF-16 units of a stemmer, or the `char` term
//! buffer of the filters that edit `rucene`'s `CharTermAttribute` in place
//! (`CJKWidthFilter`, `ScandinavianNormalizer`,
//! `ScandinavianFoldingFilter`).

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
        let prefix_len = prefix.encode_utf16().count();
        if prefix_len > len {
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
                if s[end] != u16::from(byte) {
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
                    if s[end] != unit {
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
}
