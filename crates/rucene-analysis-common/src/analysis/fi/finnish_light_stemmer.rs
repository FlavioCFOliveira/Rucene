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

//! Light stemmer for Finnish, ported from
//! `org.apache.lucene.analysis.fi.FinnishLightStemmer`.

use crate::analysis::util::stemmer_util::StemmerUtil;

/// Light Stemmer for Finnish.
///
/// This stemmer implements the algorithm described in: *Report on CLEF-2003
/// Monolingual Tracks* Jacques Savoy
///
/// Equivalent to `org.apache.lucene.analysis.fi.FinnishLightStemmer`. The
/// term it stems is the `char[]` of Java, UTF-16 units.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct FinnishLightStemmer;

impl FinnishLightStemmer {
    /// Equivalent to the implicit `FinnishLightStemmer()`.
    pub(crate) fn new() -> Self {
        Self
    }

    /// Stems the first `len` units of `s` in place and returns the new
    /// length.
    ///
    /// Equivalent to `stem(char[], int)`.
    pub(crate) fn stem(&self, s: &mut [u16], len: usize) -> usize {
        if len < 4 {
            return len;
        }

        for c in s.iter_mut().take(len) {
            match char::from_u32(u32::from(*c)) {
                Some('ä' | 'å') => *c = 'a' as u16,
                Some('ö') => *c = 'o' as u16,
                _ => {}
            }
        }

        let mut len = self.step1(s, len);
        len = self.step2(s, len);
        len = self.step3(s, len);
        len = self.norm1(s, len);
        self.norm2(s, len)
    }

    /// Equivalent to `step1(char[], int)`.
    fn step1(&self, s: &[u16], len: usize) -> usize {
        if len > 8 {
            if StemmerUtil::ends_with(s, len, "kin") {
                return self.step1(s, len - 3);
            }
            if StemmerUtil::ends_with(s, len, "ko") {
                return self.step1(s, len - 2);
            }
        }

        if len > 11 {
            if StemmerUtil::ends_with(s, len, "dellinen") {
                return len - 8;
            }
            if StemmerUtil::ends_with(s, len, "dellisuus") {
                return len - 9;
            }
        }
        len
    }

    /// Equivalent to `step2(char[], int)`.
    fn step2(&self, s: &[u16], len: usize) -> usize {
        if len > 5 {
            if StemmerUtil::ends_with(s, len, "lla")
                || StemmerUtil::ends_with(s, len, "tse")
                || StemmerUtil::ends_with(s, len, "sti")
            {
                return len - 3;
            }

            if StemmerUtil::ends_with(s, len, "ni") {
                return len - 2;
            }

            if StemmerUtil::ends_with(s, len, "aa") {
                return len - 1; // aa -> a
            }
        }

        len
    }

    /// Equivalent to `step3(char[], int)`.
    fn step3(&self, s: &mut [u16], len: usize) -> usize {
        if len > 8 {
            if StemmerUtil::ends_with(s, len, "nnen") {
                s[len - 4] = 's' as u16;
                return len - 3;
            }

            if StemmerUtil::ends_with(s, len, "ntena") {
                s[len - 5] = 's' as u16;
                return len - 4;
            }

            if StemmerUtil::ends_with(s, len, "tten") {
                return len - 4;
            }

            if StemmerUtil::ends_with(s, len, "eiden") {
                return len - 5;
            }
        }

        if len > 6 {
            if StemmerUtil::ends_with(s, len, "neen")
                || StemmerUtil::ends_with(s, len, "niin")
                || StemmerUtil::ends_with(s, len, "seen")
                || StemmerUtil::ends_with(s, len, "teen")
                || StemmerUtil::ends_with(s, len, "inen")
            {
                return len - 4;
            }

            if s[len - 3] == 'h' as u16 && self.is_vowel(s[len - 2]) && s[len - 1] == 'n' as u16 {
                return len - 3;
            }

            if StemmerUtil::ends_with(s, len, "den") {
                s[len - 3] = 's' as u16;
                return len - 2;
            }

            if StemmerUtil::ends_with(s, len, "ksen") {
                s[len - 4] = 's' as u16;
                return len - 3;
            }

            if StemmerUtil::ends_with(s, len, "ssa")
                || StemmerUtil::ends_with(s, len, "sta")
                || StemmerUtil::ends_with(s, len, "lla")
                || StemmerUtil::ends_with(s, len, "lta")
                || StemmerUtil::ends_with(s, len, "tta")
                || StemmerUtil::ends_with(s, len, "ksi")
                || StemmerUtil::ends_with(s, len, "lle")
            {
                return len - 3;
            }
        }

        if len > 5 {
            if StemmerUtil::ends_with(s, len, "na") || StemmerUtil::ends_with(s, len, "ne") {
                return len - 2;
            }

            if StemmerUtil::ends_with(s, len, "nei") {
                return len - 3;
            }
        }

        if len > 4 {
            if StemmerUtil::ends_with(s, len, "ja") || StemmerUtil::ends_with(s, len, "ta") {
                return len - 2;
            }

            if s[len - 1] == 'a' as u16 {
                return len - 1;
            }

            if s[len - 1] == 'n' as u16 && self.is_vowel(s[len - 2]) {
                return len - 2;
            }

            if s[len - 1] == 'n' as u16 {
                return len - 1;
            }
        }

        len
    }

    /// Equivalent to `norm1(char[], int)`.
    fn norm1(&self, s: &mut [u16], len: usize) -> usize {
        if len > 5 && StemmerUtil::ends_with(s, len, "hde") {
            s[len - 3] = 'k' as u16;
            s[len - 2] = 's' as u16;
            s[len - 1] = 'i' as u16;
        }

        if len > 4 && (StemmerUtil::ends_with(s, len, "ei") || StemmerUtil::ends_with(s, len, "at"))
        {
            return len - 2;
        }

        if len > 3
            && matches!(
                char::from_u32(u32::from(s[len - 1])),
                Some('t' | 's' | 'j' | 'e' | 'a' | 'i')
            )
        {
            return len - 1;
        }

        len
    }

    /// Equivalent to `norm2(char[], int)`.
    fn norm2(&self, s: &mut [u16], len: usize) -> usize {
        let mut len = len;
        if len > 8
            && (s[len - 1] == 'e' as u16 || s[len - 1] == 'o' as u16 || s[len - 1] == 'u' as u16)
        {
            len -= 1;
        }

        if len > 4 {
            if s[len - 1] == 'i' as u16 {
                len -= 1;
            }

            if len > 4 {
                let mut ch = s[0];
                // Java's `for (int i = 1; i < len; i++)` whose deleting branch
                // runs `delete(s, i--, len)`: after a deletion the same index
                // is read again.
                let mut i = 1;
                while i < len {
                    if s[i] == ch && (ch == 'k' as u16 || ch == 'p' as u16 || ch == 't' as u16) {
                        len = StemmerUtil::delete(s, i, len);
                    } else {
                        ch = s[i];
                        i += 1;
                    }
                }
            }
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
