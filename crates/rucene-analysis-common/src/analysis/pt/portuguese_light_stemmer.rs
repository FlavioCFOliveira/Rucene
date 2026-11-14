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

//! Light stemmer for Portuguese, ported from
//! `org.apache.lucene.analysis.pt.PortugueseLightStemmer`.

use crate::analysis::util::stemmer_util::StemmerUtil;

/// `StemmerUtil.endsWith(char[], int, String)`, statically imported by the
/// Java class.
fn ends_with(s: &[u16], len: usize, suffix: &str) -> bool {
    StemmerUtil::ends_with(s, len, suffix)
}

/// Light Stemmer for Portuguese
///
/// This stemmer implements the "UniNE" algorithm in: *Light Stemming
/// Approaches for the French, Portuguese, German and Hungarian Languages*
/// Jacques Savoy
///
/// Equivalent to `org.apache.lucene.analysis.pt.PortugueseLightStemmer`.
/// The term it stems is the `char[]` of Java, UTF-16 units.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct PortugueseLightStemmer;

impl PortugueseLightStemmer {
    /// Equivalent to the implicit `PortugueseLightStemmer()`.
    pub(crate) fn new() -> Self {
        Self
    }

    /// Stems the first `len` units of `s` in place and returns the new
    /// length.
    ///
    /// Equivalent to `stem(char[], int)`.
    pub(crate) fn stem(&self, s: &mut [u16], mut len: usize) -> usize {
        if len < 4 {
            return len;
        }

        len = self.remove_suffix(s, len);

        if len > 3 && s[len - 1] == 'a' as u16 {
            len = self.norm_feminine(s, len);
        }

        if len > 4 && matches!(char::from_u32(u32::from(s[len - 1])), Some('e' | 'a' | 'o')) {
            len -= 1;
        }

        for c in s.iter_mut().take(len) {
            match char::from_u32(u32::from(*c)) {
                Some('à' | 'á' | 'â' | 'ä' | 'ã') => *c = 'a' as u16,
                Some('ò' | 'ó' | 'ô' | 'ö' | 'õ') => *c = 'o' as u16,
                Some('è' | 'é' | 'ê' | 'ë') => *c = 'e' as u16,
                Some('ù' | 'ú' | 'û' | 'ü') => *c = 'u' as u16,
                Some('ì' | 'í' | 'î' | 'ï') => *c = 'i' as u16,
                Some('ç') => *c = 'c' as u16,
                _ => {}
            }
        }

        len
    }

    /// Equivalent to `removeSuffix(char[], int)`.
    fn remove_suffix(&self, s: &mut [u16], mut len: usize) -> usize {
        if len > 4
            && ends_with(s, len, "es")
            && matches!(
                char::from_u32(u32::from(s[len - 3])),
                Some('r' | 's' | 'l' | 'z')
            )
        {
            return len - 2;
        }

        if len > 3 && ends_with(s, len, "ns") {
            s[len - 2] = 'm' as u16;
            return len - 1;
        }

        if len > 4 && (ends_with(s, len, "eis") || ends_with(s, len, "éis")) {
            s[len - 3] = 'e' as u16;
            s[len - 2] = 'l' as u16;
            return len - 1;
        }

        if len > 4 && ends_with(s, len, "ais") {
            s[len - 2] = 'l' as u16;
            return len - 1;
        }

        if len > 4 && ends_with(s, len, "óis") {
            s[len - 3] = 'o' as u16;
            s[len - 2] = 'l' as u16;
            return len - 1;
        }

        if len > 4 && ends_with(s, len, "is") {
            s[len - 1] = 'l' as u16;
            return len;
        }

        if len > 3 && (ends_with(s, len, "ões") || ends_with(s, len, "ães")) {
            len -= 1;
            s[len - 2] = 'ã' as u16;
            s[len - 1] = 'o' as u16;
            return len;
        }

        if len > 6 && ends_with(s, len, "mente") {
            return len - 5;
        }

        if len > 3 && s[len - 1] == 's' as u16 {
            return len - 1;
        }
        len
    }

    /// Equivalent to `normFeminine(char[], int)`.
    fn norm_feminine(&self, s: &mut [u16], len: usize) -> usize {
        if len > 7
            && (ends_with(s, len, "inha") || ends_with(s, len, "iaca") || ends_with(s, len, "eira"))
        {
            s[len - 1] = 'o' as u16;
            return len;
        }

        if len > 6 {
            if ends_with(s, len, "osa")
                || ends_with(s, len, "ica")
                || ends_with(s, len, "ida")
                || ends_with(s, len, "ada")
                || ends_with(s, len, "iva")
                || ends_with(s, len, "ama")
            {
                s[len - 1] = 'o' as u16;
                return len;
            }

            if ends_with(s, len, "ona") {
                s[len - 3] = 'ã' as u16;
                s[len - 2] = 'o' as u16;
                return len - 1;
            }

            if ends_with(s, len, "ora") {
                return len - 1;
            }

            if ends_with(s, len, "esa") {
                s[len - 3] = 'ê' as u16;
                return len - 1;
            }

            if ends_with(s, len, "na") {
                s[len - 1] = 'o' as u16;
                return len;
            }
        }
        len
    }
}
