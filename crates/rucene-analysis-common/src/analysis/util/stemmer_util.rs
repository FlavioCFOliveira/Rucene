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
        let suffix_len = suffix.encode_utf16().count();
        if suffix_len > len {
            return false;
        }
        // Java compares from the last unit backwards; the comparisons have no
        // side effect, so comparing forwards answers the same.
        let start = len - suffix_len;
        suffix
            .encode_utf16()
            .enumerate()
            .all(|(i, c)| s[start + i] == c)
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
