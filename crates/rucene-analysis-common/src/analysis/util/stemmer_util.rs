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
//! **Divergence from Lucene 10.5.0.** Java's methods take the `char[]` term
//! buffer of a `CharTermAttribute`, UTF-16 units; `rucene`'s term buffer
//! holds `char`s (Unicode scalar values, see `CharTermAttribute`), so these
//! take a `[char]` buffer, and their positions and lengths count `char`s.
//! The strings they compare against are read as `char`s too. For a buffer
//! and strings of the Basic Multilingual Plane, which every stemmer of the
//! module compares, the two are the same; a supplementary character is one
//! position here where Java counts two.

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
    pub fn starts_with(s: &[char], len: usize, prefix: &str) -> bool {
        let prefix_len = prefix.chars().count();
        if prefix_len > len {
            return false;
        }
        for (i, c) in prefix.chars().enumerate() {
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
    pub fn ends_with(s: &[char], len: usize, suffix: &str) -> bool {
        let suffix: Vec<char> = suffix.chars().collect();
        Self::ends_with_chars(s, len, &suffix)
    }

    /// Returns true if the character array ends with the suffix.
    ///
    /// Equivalent to `StemmerUtil.endsWith(char[], int, char[])`.
    pub fn ends_with_chars(s: &[char], len: usize, suffix: &[char]) -> bool {
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
    pub fn delete(s: &mut [char], pos: usize, len: usize) -> usize {
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
    pub fn delete_n(s: &mut [char], pos: usize, len: usize, n_chars: usize) -> usize {
        debug_assert!(pos + n_chars <= len);
        if pos + n_chars < len {
            // don't arraycopy if asked to delete the last characters
            s.copy_within(pos + n_chars..len, pos);
        }
        len - n_chars
    }
}
