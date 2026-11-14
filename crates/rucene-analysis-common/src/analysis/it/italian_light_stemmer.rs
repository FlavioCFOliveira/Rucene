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

//! Light stemmer for Italian, ported from
//! `org.apache.lucene.analysis.it.ItalianLightStemmer`.

/// Light Stemmer for Italian.
///
/// This stemmer implements the algorithm described in: *Report on
/// CLEF-2001 Experiments* Jacques Savoy
///
/// Equivalent to `org.apache.lucene.analysis.it.ItalianLightStemmer`. The
/// term it stems is the `char[]` of Java, UTF-16 units.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct ItalianLightStemmer;

impl ItalianLightStemmer {
    /// Equivalent to the implicit `ItalianLightStemmer()`.
    pub(crate) fn new() -> Self {
        Self
    }

    /// Stems the first `len` units of `s` in place and returns the new
    /// length.
    ///
    /// Equivalent to `stem(char[], int)`.
    pub(crate) fn stem(&self, s: &mut [u16], len: usize) -> usize {
        if len < 6 {
            return len;
        }

        for c in s.iter_mut().take(len) {
            match char::from_u32(u32::from(*c)) {
                Some('à' | 'á' | 'â' | 'ä') => *c = 'a' as u16,
                Some('ò' | 'ó' | 'ô' | 'ö') => *c = 'o' as u16,
                Some('è' | 'é' | 'ê' | 'ë') => *c = 'e' as u16,
                Some('ù' | 'ú' | 'û' | 'ü') => *c = 'u' as u16,
                Some('ì' | 'í' | 'î' | 'ï') => *c = 'i' as u16,
                _ => {}
            }
        }

        match char::from_u32(u32::from(s[len - 1])) {
            Some('e') => {
                if s[len - 2] == 'i' as u16 || s[len - 2] == 'h' as u16 {
                    len - 2
                } else {
                    len - 1
                }
            }
            Some('i') => {
                if s[len - 2] == 'h' as u16 || s[len - 2] == 'i' as u16 {
                    len - 2
                } else {
                    len - 1
                }
            }
            Some('a') => {
                if s[len - 2] == 'i' as u16 {
                    len - 2
                } else {
                    len - 1
                }
            }
            Some('o') => {
                if s[len - 2] == 'i' as u16 {
                    len - 2
                } else {
                    len - 1
                }
            }
            _ => len,
        }
    }
}
