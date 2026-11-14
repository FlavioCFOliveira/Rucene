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

/*
 * This algorithm is updated based on code located at:
 * http://members.unine.ch/jacques.savoy/clef/
 *
 * Full copyright for that code follows:
 */

/*
 * Copyright (c) 2005, Jacques Savoy
 * All rights reserved.
 *
 * Redistribution and use in source and binary forms, with or without
 * modification, are permitted provided that the following conditions are met:
 *
 * Redistributions of source code must retain the above copyright notice, this
 * list of conditions and the following disclaimer. Redistributions in binary
 * form must reproduce the above copyright notice, this list of conditions and
 * the following disclaimer in the documentation and/or other materials
 * provided with the distribution. Neither the name of the author nor the names
 * of its contributors may be used to endorse or promote products derived from
 * this software without specific prior written permission.
 *
 * THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
 * AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
 * IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
 * ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE
 * LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR
 * CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF
 * SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS
 * INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN
 * CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE)
 * ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE
 * POSSIBILITY OF SUCH DAMAGE.
 */

//! Light stemmer for Hungarian, ported from
//! `org.apache.lucene.analysis.hu.HungarianLightStemmer`.

use crate::analysis::util::stemmer_util::StemmerUtil;

/// Light Stemmer for Hungarian.
///
/// This stemmer implements the "UniNE" algorithm in: *Light Stemming
/// Approaches for the French, Portuguese, German and Hungarian Languages*
/// Jacques Savoy
///
/// Equivalent to `org.apache.lucene.analysis.hu.HungarianLightStemmer`. The
/// term it stems is the `char[]` of Java, UTF-16 units.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct HungarianLightStemmer;

impl HungarianLightStemmer {
    /// Equivalent to the implicit `HungarianLightStemmer()`.
    pub(crate) fn new() -> Self {
        Self
    }

    /// Stems the first `len` units of `s` in place and returns the new
    /// length.
    ///
    /// Equivalent to `stem(char[], int)`.
    pub(crate) fn stem(&self, s: &mut [u16], len: usize) -> usize {
        for c in s.iter_mut().take(len) {
            match char::from_u32(u32::from(*c)) {
                Some('á') => *c = 'a' as u16,
                Some('ë' | 'é') => *c = 'e' as u16,
                Some('í') => *c = 'i' as u16,
                Some('ó' | 'ő' | 'õ' | 'ö') => *c = 'o' as u16,
                Some('ú' | 'ű' | 'ũ' | 'û' | 'ü') => *c = 'u' as u16,
                _ => {}
            }
        }

        let mut len = self.remove_case(s, len);
        len = self.remove_possessive(s, len);
        len = self.remove_plural(s, len);
        self.normalize(s, len)
    }

    /// Equivalent to `removeCase(char[], int)`.
    fn remove_case(&self, s: &[u16], len: usize) -> usize {
        let ends_with = |suffix: &str| StemmerUtil::ends_with(s, len, suffix);
        if len > 6 && ends_with("kent") {
            return len - 4;
        }

        if len > 5 {
            if ends_with("nak")
                || ends_with("nek")
                || ends_with("val")
                || ends_with("vel")
                || ends_with("ert")
                || ends_with("rol")
                || ends_with("ban")
                || ends_with("ben")
                || ends_with("bol")
                || ends_with("nal")
                || ends_with("nel")
                || ends_with("hoz")
                || ends_with("hez")
                || ends_with("tol")
            {
                return len - 3;
            }

            if (ends_with("al") || ends_with("el"))
                && !self.is_vowel(s[len - 3])
                && s[len - 3] == s[len - 4]
            {
                return len - 3;
            }
        }

        if len > 4 {
            if ends_with("at")
                || ends_with("et")
                || ends_with("ot")
                || ends_with("va")
                || ends_with("ve")
                || ends_with("ra")
                || ends_with("re")
                || ends_with("ba")
                || ends_with("be")
                || ends_with("ul")
                || ends_with("ig")
            {
                return len - 2;
            }

            if (ends_with("on") || ends_with("en")) && !self.is_vowel(s[len - 3]) {
                return len - 2;
            }

            match char::from_u32(u32::from(s[len - 1])) {
                Some('t' | 'n') => return len - 1,
                Some('a' | 'e') if s[len - 2] == s[len - 3] && !self.is_vowel(s[len - 2]) => {
                    return len - 2;
                }
                _ => {}
            }
        }

        len
    }

    /// Equivalent to `removePossessive(char[], int)`.
    fn remove_possessive(&self, s: &[u16], len: usize) -> usize {
        let ends_with = |suffix: &str| StemmerUtil::ends_with(s, len, suffix);
        if len > 6 {
            if !self.is_vowel(s[len - 5])
                && (ends_with("atok") || ends_with("otok") || ends_with("etek"))
            {
                return len - 4;
            }

            if ends_with("itek") || ends_with("itok") {
                return len - 4;
            }
        }

        if len > 5 {
            if !self.is_vowel(s[len - 4])
                && (ends_with("unk") || ends_with("tok") || ends_with("tek"))
            {
                return len - 3;
            }

            if self.is_vowel(s[len - 4]) && ends_with("juk") {
                return len - 3;
            }

            if ends_with("ink") {
                return len - 3;
            }
        }

        if len > 4 {
            if !self.is_vowel(s[len - 3])
                && (ends_with("am")
                    || ends_with("em")
                    || ends_with("om")
                    || ends_with("ad")
                    || ends_with("ed")
                    || ends_with("od")
                    || ends_with("uk"))
            {
                return len - 2;
            }

            if self.is_vowel(s[len - 3]) && (ends_with("nk") || ends_with("ja") || ends_with("je"))
            {
                return len - 2;
            }

            if ends_with("im") || ends_with("id") || ends_with("ik") {
                return len - 2;
            }
        }

        if len > 3 {
            match char::from_u32(u32::from(s[len - 1])) {
                Some('a' | 'e') => {
                    if !self.is_vowel(s[len - 2]) {
                        return len - 1;
                    }
                }
                Some('m' | 'd') => {
                    if self.is_vowel(s[len - 2]) {
                        return len - 1;
                    }
                }
                Some('i') => return len - 1,
                _ => {}
            }
        }

        len
    }

    /// Equivalent to `removePlural(char[], int)`, whose `switch` falls
    /// through from the vowel cases to `default` when `len <= 4`.
    fn remove_plural(&self, s: &[u16], len: usize) -> usize {
        if len > 3 && s[len - 1] == 'k' as u16 {
            if matches!(char::from_u32(u32::from(s[len - 2])), Some('a' | 'o' | 'e')) && len > 4 {
                return len - 2; /* intentional fallthru */
            }
            return len - 1;
        }
        len
    }

    /// Equivalent to `normalize(char[], int)`.
    fn normalize(&self, s: &[u16], len: usize) -> usize {
        if len > 3
            && matches!(
                char::from_u32(u32::from(s[len - 1])),
                Some('a' | 'e' | 'i' | 'o')
            )
        {
            return len - 1;
        }
        len
    }

    /// Equivalent to `isVowel(char)`.
    fn is_vowel(&self, ch: u16) -> bool {
        matches!(
            char::from_u32(u32::from(ch)),
            Some('a' | 'e' | 'i' | 'o' | 'u' | 'y')
        )
    }
}
