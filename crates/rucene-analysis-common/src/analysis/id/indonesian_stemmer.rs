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

//! Stemmer for Indonesian, ported from
//! `org.apache.lucene.analysis.id.IndonesianStemmer`.

use crate::analysis::util::stemmer_util::StemmerUtil;

const REMOVED_KE: i32 = 1;
const REMOVED_PENG: i32 = 2;
const REMOVED_DI: i32 = 4;
const REMOVED_MENG: i32 = 8;
const REMOVED_TER: i32 = 16;
const REMOVED_BER: i32 = 32;
const REMOVED_PE: i32 = 64;

/// `StemmerUtil.endsWith(char[], int, String)`, statically imported by the
/// Java class.
fn ends_with(s: &[u16], len: usize, suffix: &str) -> bool {
    StemmerUtil::ends_with(s, len, suffix)
}

/// `StemmerUtil.startsWith(char[], int, String)`, statically imported by the
/// Java class.
fn starts_with(s: &[u16], len: usize, prefix: &str) -> bool {
    StemmerUtil::starts_with(s, len, prefix)
}

/// `StemmerUtil.deleteN(char[], int, int, int)`, statically imported by the
/// Java class.
fn delete_n(s: &mut [u16], pos: usize, len: usize, n_chars: usize) -> usize {
    StemmerUtil::delete_n(s, pos, len, n_chars)
}

/// Stemmer for Indonesian.
///
/// Stems Indonesian words with the algorithm presented in: *A Study of
/// Stemming Effects on Information Retrieval in Bahasa Indonesia*, Fadillah
/// Z Tala. http://www.illc.uva.nl/Publications/ResearchReports/MoL-2003-02.text.pdf
///
/// Equivalent to `org.apache.lucene.analysis.id.IndonesianStemmer`. The term
/// it stems is the `char[]` of Java, UTF-16 units.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct IndonesianStemmer {
    num_syllables: i32,
    flags: i32,
}

impl IndonesianStemmer {
    /// Equivalent to the implicit `IndonesianStemmer()`.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Stem a term (returning its new length).
    ///
    /// Use `stem_derivational` to control whether full stemming or only
    /// light inflectional stemming is done.
    ///
    /// Equivalent to `stem(char[], int, boolean)`.
    pub(crate) fn stem(
        &mut self,
        text: &mut [u16],
        mut length: usize,
        stem_derivational: bool,
    ) -> usize {
        self.flags = 0;
        self.num_syllables = 0;
        for &c in text.iter().take(length) {
            if self.is_vowel(c) {
                self.num_syllables += 1;
            }
        }

        if self.num_syllables > 2 {
            length = self.remove_particle(text, length);
        }
        if self.num_syllables > 2 {
            length = self.remove_possessive_pronoun(text, length);
        }

        if stem_derivational {
            length = self.stem_derivational(text, length);
        }
        length
    }

    /// Equivalent to `stemDerivational(char[], int)`.
    fn stem_derivational(&mut self, text: &mut [u16], mut length: usize) -> usize {
        let mut old_length = length;
        if self.num_syllables > 2 {
            length = self.remove_first_order_prefix(text, length);
        }
        if old_length != length {
            // a rule is fired
            old_length = length;
            if self.num_syllables > 2 {
                length = self.remove_suffix(text, length);
            }
            if old_length != length {
                // a rule is fired
                if self.num_syllables > 2 {
                    length = self.remove_second_order_prefix(text, length);
                }
            }
        } else {
            // fail
            if self.num_syllables > 2 {
                length = self.remove_second_order_prefix(text, length);
            }
            if self.num_syllables > 2 {
                length = self.remove_suffix(text, length);
            }
        }
        length
    }

    /// Equivalent to `isVowel(char)`.
    fn is_vowel(&self, ch: u16) -> bool {
        matches!(
            char::from_u32(u32::from(ch)),
            Some('a' | 'e' | 'i' | 'o' | 'u')
        )
    }

    /// Equivalent to `removeParticle(char[], int)`.
    fn remove_particle(&mut self, text: &[u16], length: usize) -> usize {
        if ends_with(text, length, "kah")
            || ends_with(text, length, "lah")
            || ends_with(text, length, "pun")
        {
            self.num_syllables -= 1;
            return length - 3;
        }

        length
    }

    /// Equivalent to `removePossessivePronoun(char[], int)`.
    fn remove_possessive_pronoun(&mut self, text: &[u16], length: usize) -> usize {
        if ends_with(text, length, "ku") || ends_with(text, length, "mu") {
            self.num_syllables -= 1;
            return length - 2;
        }

        if ends_with(text, length, "nya") {
            self.num_syllables -= 1;
            return length - 3;
        }

        length
    }

    /// Equivalent to `removeFirstOrderPrefix(char[], int)`.
    fn remove_first_order_prefix(&mut self, text: &mut [u16], length: usize) -> usize {
        if starts_with(text, length, "meng") {
            self.flags |= REMOVED_MENG;
            self.num_syllables -= 1;
            return delete_n(text, 0, length, 4);
        }

        if starts_with(text, length, "meny") && length > 4 && self.is_vowel(text[4]) {
            self.flags |= REMOVED_MENG;
            text[3] = 's' as u16;
            self.num_syllables -= 1;
            return delete_n(text, 0, length, 3);
        }

        if starts_with(text, length, "men") {
            self.flags |= REMOVED_MENG;
            self.num_syllables -= 1;
            return delete_n(text, 0, length, 3);
        }

        if starts_with(text, length, "mem") {
            self.flags |= REMOVED_MENG;
            self.num_syllables -= 1;
            return delete_n(text, 0, length, 3);
        }

        if starts_with(text, length, "me") {
            self.flags |= REMOVED_MENG;
            self.num_syllables -= 1;
            return delete_n(text, 0, length, 2);
        }

        if starts_with(text, length, "peng") {
            self.flags |= REMOVED_PENG;
            self.num_syllables -= 1;
            return delete_n(text, 0, length, 4);
        }

        if starts_with(text, length, "peny") && length > 4 && self.is_vowel(text[4]) {
            self.flags |= REMOVED_PENG;
            text[3] = 's' as u16;
            self.num_syllables -= 1;
            return delete_n(text, 0, length, 3);
        }

        if starts_with(text, length, "peny") {
            self.flags |= REMOVED_PENG;
            self.num_syllables -= 1;
            return delete_n(text, 0, length, 4);
        }

        if starts_with(text, length, "pen") && length > 3 && self.is_vowel(text[3]) {
            self.flags |= REMOVED_PENG;
            text[2] = 't' as u16;
            self.num_syllables -= 1;
            return delete_n(text, 0, length, 2);
        }

        if starts_with(text, length, "pen") {
            self.flags |= REMOVED_PENG;
            self.num_syllables -= 1;
            return delete_n(text, 0, length, 3);
        }

        if starts_with(text, length, "pem") {
            self.flags |= REMOVED_PENG;
            self.num_syllables -= 1;
            return delete_n(text, 0, length, 3);
        }

        if starts_with(text, length, "di") {
            self.flags |= REMOVED_DI;
            self.num_syllables -= 1;
            return delete_n(text, 0, length, 2);
        }

        if starts_with(text, length, "ter") {
            self.flags |= REMOVED_TER;
            self.num_syllables -= 1;
            return delete_n(text, 0, length, 3);
        }

        if starts_with(text, length, "ke") {
            self.flags |= REMOVED_KE;
            self.num_syllables -= 1;
            return delete_n(text, 0, length, 2);
        }

        length
    }

    /// Equivalent to `removeSecondOrderPrefix(char[], int)`.
    fn remove_second_order_prefix(&mut self, text: &mut [u16], length: usize) -> usize {
        if starts_with(text, length, "ber") {
            self.flags |= REMOVED_BER;
            self.num_syllables -= 1;
            return delete_n(text, 0, length, 3);
        }

        if length == 7 && starts_with(text, length, "belajar") {
            self.flags |= REMOVED_BER;
            self.num_syllables -= 1;
            return delete_n(text, 0, length, 3);
        }

        if starts_with(text, length, "be")
            && length > 4
            && !self.is_vowel(text[2])
            && text[3] == 'e' as u16
            && text[4] == 'r' as u16
        {
            self.flags |= REMOVED_BER;
            self.num_syllables -= 1;
            return delete_n(text, 0, length, 2);
        }

        if starts_with(text, length, "per") {
            self.num_syllables -= 1;
            return delete_n(text, 0, length, 3);
        }

        if length == 7 && starts_with(text, length, "pelajar") {
            self.num_syllables -= 1;
            return delete_n(text, 0, length, 3);
        }

        if starts_with(text, length, "pe") {
            self.flags |= REMOVED_PE;
            self.num_syllables -= 1;
            return delete_n(text, 0, length, 2);
        }

        length
    }

    /// Equivalent to `removeSuffix(char[], int)`.
    fn remove_suffix(&mut self, text: &[u16], length: usize) -> usize {
        if ends_with(text, length, "kan")
            && (self.flags & REMOVED_KE) == 0
            && (self.flags & REMOVED_PENG) == 0
            && (self.flags & REMOVED_PE) == 0
        {
            self.num_syllables -= 1;
            return length - 3;
        }

        if ends_with(text, length, "an")
            && (self.flags & REMOVED_DI) == 0
            && (self.flags & REMOVED_MENG) == 0
            && (self.flags & REMOVED_TER) == 0
        {
            self.num_syllables -= 1;
            return length - 2;
        }

        if ends_with(text, length, "i")
            && !ends_with(text, length, "si")
            && (self.flags & REMOVED_BER) == 0
            && (self.flags & REMOVED_KE) == 0
            && (self.flags & REMOVED_PENG) == 0
        {
            self.num_syllables -= 1;
            return length - 1;
        }
        length
    }
}
