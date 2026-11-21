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

//! Light stemmer for German, ported from
//! `org.apache.lucene.analysis.de.GermanLightStemmer`.

/// Light Stemmer for German.
///
/// This stemmer implements the "UniNE" algorithm in: *Light Stemming
/// Approaches for the French, Portuguese, German and Hungarian Languages*
/// Jacques Savoy
///
/// Equivalent to `org.apache.lucene.analysis.de.GermanLightStemmer`. The
/// term it stems is the `char[]` of Java, UTF-16 units.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct GermanLightStemmer;

impl GermanLightStemmer {
    /// Equivalent to the implicit `GermanLightStemmer()`.
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
                Some('ä' | 'à' | 'á' | 'â') => *c = 'a' as u16,
                Some('ö' | 'ò' | 'ó' | 'ô') => *c = 'o' as u16,
                Some('ï' | 'ì' | 'í' | 'î') => *c = 'i' as u16,
                Some('ü' | 'ù' | 'ú' | 'û') => *c = 'u' as u16,
                _ => {}
            }
        }

        let len = self.step1(s, len);
        self.step2(s, len)
    }

    /// Equivalent to `stEnding(char)`.
    fn st_ending(&self, ch: u16) -> bool {
        matches!(
            char::from_u32(u32::from(ch)),
            Some('b' | 'd' | 'f' | 'g' | 'h' | 'k' | 'l' | 'm' | 'n' | 't')
        )
    }

    /// Equivalent to `step1(char[], int)`.
    fn step1(&self, s: &[u16], len: usize) -> usize {
        if len > 5
            && s[len - 3] == 'e' as u16
            && s[len - 2] == 'r' as u16
            && s[len - 1] == 'n' as u16
        {
            return len - 3;
        }

        if len > 4
            && s[len - 2] == 'e' as u16
            && matches!(
                char::from_u32(u32::from(s[len - 1])),
                Some('m' | 'n' | 'r' | 's')
            )
        {
            return len - 2;
        }

        if len > 3 && s[len - 1] == 'e' as u16 {
            return len - 1;
        }

        if len > 3 && s[len - 1] == 's' as u16 && self.st_ending(s[len - 2]) {
            return len - 1;
        }

        len
    }

    /// Equivalent to `step2(char[], int)`.
    fn step2(&self, s: &[u16], len: usize) -> usize {
        if len > 5
            && s[len - 3] == 'e' as u16
            && s[len - 2] == 's' as u16
            && s[len - 1] == 't' as u16
        {
            return len - 3;
        }

        if len > 4
            && s[len - 2] == 'e' as u16
            && (s[len - 1] == 'r' as u16 || s[len - 1] == 'n' as u16)
        {
            return len - 2;
        }

        if len > 4
            && s[len - 2] == 's' as u16
            && s[len - 1] == 't' as u16
            && self.st_ending(s[len - 3])
        {
            return len - 2;
        }

        len
    }
}
