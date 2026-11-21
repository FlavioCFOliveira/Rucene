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

//! Stemmer for Persian, ported from
//! `org.apache.lucene.analysis.fa.PersianStemmer`.

use crate::analysis::util::stemmer_util::StemmerUtil;

const ALEF: u16 = 0x0627;
const HEH: u16 = 0x0647;
const TEH: u16 = 0x062A;
const REH: u16 = 0x0631;
const NOON: u16 = 0x0646;
const YEH: u16 = 0x064A;
const ZWNJ: u16 = 0x200c; // ZERO WIDTH NON-JOINER character

/// Equivalent to the private `PersianStemmer.suffixes`.
const SUFFIXES: [&[u16]; 8] = [
    &[ALEF, TEH],
    &[ALEF, NOON],
    &[TEH, REH, YEH, NOON],
    &[TEH, REH],
    &[YEH, YEH],
    &[YEH],
    &[HEH, ALEF],
    &[ZWNJ],
];

/// Stemmer for Persian.
///
/// Stemming is done in-place for efficiency, operating on a termbuffer.
///
/// Stemming is defined as:
///
/// * Removal of attached definite article, conjunction, and prepositions.
/// * Stemming of common suffixes.
///
/// Equivalent to `org.apache.lucene.analysis.fa.PersianStemmer`. The term it
/// stems is the `char[]` of Java, UTF-16 units.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct PersianStemmer;

impl PersianStemmer {
    /// Equivalent to the implicit `PersianStemmer()`.
    pub(crate) fn new() -> Self {
        Self
    }

    /// Stem an input buffer of Persian text.
    ///
    /// * `s`: input buffer
    /// * `len`: length of input buffer
    /// * returns: length of input buffer after normalization
    ///
    /// Equivalent to `stem(char[], int)`.
    pub(crate) fn stem(&self, s: &mut [u16], len: usize) -> usize {
        self.stem_suffix(s, len)
    }

    /// Stem suffix(es) off a Persian word.
    ///
    /// * `s`: input buffer
    /// * `len`: length of input buffer
    /// * returns: new length of input buffer after stemming
    ///
    /// Equivalent to `stemSuffix(char[], int)`.
    fn stem_suffix(&self, s: &mut [u16], len: usize) -> usize {
        let mut len = len;
        for suffix in SUFFIXES {
            if self.ends_with_check_length(s, len, suffix) {
                len = StemmerUtil::delete_n(s, len - suffix.len(), len, suffix.len());
            }
        }

        len
    }

    /// Returns true if the suffix matches and can be stemmed
    ///
    /// * `s`: input buffer
    /// * `len`: length of input buffer
    /// * `suffix`: suffix to check
    /// * returns: true if the suffix matches and can be stemmed
    ///
    /// Equivalent to `endsWithCheckLength(char[], int, char[])`, whose
    /// `Arrays.equals(s, len - suffix.length, len, suffix, 0, suffix.length)`
    /// compares the ranges.
    fn ends_with_check_length(&self, s: &[u16], len: usize, suffix: &[u16]) -> bool {
        if len < suffix.len() + 2 {
            // all suffixes require at least 2 characters after stemming
            return false;
        }

        s[len - suffix.len()..len] == *suffix
    }
}
