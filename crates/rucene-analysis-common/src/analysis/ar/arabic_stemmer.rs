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

//! Stemmer for Arabic, ported from
//! `org.apache.lucene.analysis.ar.ArabicStemmer`.

use crate::analysis::util::stemmer_util::StemmerUtil;

/// Stemmer for Arabic.
///
/// Stemming is done in-place for efficiency, operating on a termbuffer.
///
/// Stemming is defined as:
///
/// * Removal of attached definite article, conjunction, and prepositions.
/// * Stemming of common suffixes.
///
/// Equivalent to `org.apache.lucene.analysis.ar.ArabicStemmer`. The term it
/// stems is the `char[]` of Java, UTF-16 units.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct ArabicStemmer;

impl ArabicStemmer {
    pub(crate) const ALEF: u16 = 0x0627;
    pub(crate) const BEH: u16 = 0x0628;
    pub(crate) const TEH_MARBUTA: u16 = 0x0629;
    pub(crate) const TEH: u16 = 0x062A;
    pub(crate) const FEH: u16 = 0x0641;
    pub(crate) const KAF: u16 = 0x0643;
    pub(crate) const LAM: u16 = 0x0644;
    pub(crate) const NOON: u16 = 0x0646;
    pub(crate) const HEH: u16 = 0x0647;
    pub(crate) const WAW: u16 = 0x0648;
    pub(crate) const YEH: u16 = 0x064A;

    /// Equivalent to `ArabicStemmer.prefixes`.
    pub(crate) const PREFIXES: [&'static [u16]; 7] = [
        &[Self::ALEF, Self::LAM],
        &[Self::WAW, Self::ALEF, Self::LAM],
        &[Self::BEH, Self::ALEF, Self::LAM],
        &[Self::KAF, Self::ALEF, Self::LAM],
        &[Self::FEH, Self::ALEF, Self::LAM],
        &[Self::LAM, Self::LAM],
        &[Self::WAW],
    ];

    /// Equivalent to `ArabicStemmer.suffixes`.
    pub(crate) const SUFFIXES: [&'static [u16]; 10] = [
        &[Self::HEH, Self::ALEF],
        &[Self::ALEF, Self::NOON],
        &[Self::ALEF, Self::TEH],
        &[Self::WAW, Self::NOON],
        &[Self::YEH, Self::NOON],
        &[Self::YEH, Self::HEH],
        &[Self::YEH, Self::TEH_MARBUTA],
        &[Self::HEH],
        &[Self::TEH_MARBUTA],
        &[Self::YEH],
    ];

    /// Equivalent to the implicit `ArabicStemmer()`.
    pub(crate) fn new() -> Self {
        Self
    }

    /// Stem an input buffer of Arabic text.
    ///
    /// * `s`: input buffer
    /// * `len`: length of input buffer
    /// * returns: length of input buffer after normalization
    ///
    /// Equivalent to `stem(char[], int)`.
    pub(crate) fn stem(&self, s: &mut [u16], len: usize) -> usize {
        let len = self.stem_prefix(s, len);
        self.stem_suffix(s, len)
    }

    /// Stem a prefix off an Arabic word.
    ///
    /// * `s`: input buffer
    /// * `len`: length of input buffer
    /// * returns: new length of input buffer after stemming.
    ///
    /// Equivalent to `stemPrefix(char[], int)`.
    pub(crate) fn stem_prefix(&self, s: &mut [u16], len: usize) -> usize {
        for prefix in Self::PREFIXES {
            if self.starts_with_check_length(s, len, prefix) {
                return StemmerUtil::delete_n(s, 0, len, prefix.len());
            }
        }
        len
    }

    /// Stem suffix(es) off an Arabic word.
    ///
    /// * `s`: input buffer
    /// * `len`: length of input buffer
    /// * returns: new length of input buffer after stemming
    ///
    /// Equivalent to `stemSuffix(char[], int)`.
    pub(crate) fn stem_suffix(&self, s: &mut [u16], len: usize) -> usize {
        let mut len = len;
        for suffix in Self::SUFFIXES {
            if self.ends_with_check_length(s, len, suffix) {
                len = StemmerUtil::delete_n(s, len - suffix.len(), len, suffix.len());
            }
        }
        len
    }

    /// Returns true if the prefix matches and can be stemmed
    ///
    /// * `s`: input buffer
    /// * `len`: length of input buffer
    /// * `prefix`: prefix to check
    /// * returns: true if the prefix matches and can be stemmed
    ///
    /// Equivalent to `startsWithCheckLength(char[], int, char[])`.
    pub(crate) fn starts_with_check_length(&self, s: &[u16], len: usize, prefix: &[u16]) -> bool {
        if prefix.len() == 1 && len < 4 {
            // wa- prefix requires at least 3 characters
            false
        } else if len < prefix.len() + 2 {
            // other prefixes require only 2.
            false
        } else {
            for (i, &p) in prefix.iter().enumerate() {
                if s[i] != p {
                    return false;
                }
            }

            true
        }
    }

    /// Returns true if the suffix matches and can be stemmed
    ///
    /// * `s`: input buffer
    /// * `len`: length of input buffer
    /// * `suffix`: suffix to check
    /// * returns: true if the suffix matches and can be stemmed
    ///
    /// Equivalent to `endsWithCheckLength(char[], int, char[])`.
    pub(crate) fn ends_with_check_length(&self, s: &[u16], len: usize, suffix: &[u16]) -> bool {
        if len < suffix.len() + 2 {
            // all suffixes require at least 2 characters after stemming
            false
        } else {
            for (i, &c) in suffix.iter().enumerate() {
                if s[len - suffix.len() + i] != c {
                    return false;
                }
            }

            true
        }
    }
}
