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

//! Light stemmer for Russian, ported from
//! `org.apache.lucene.analysis.ru.RussianLightStemmer`.

use crate::analysis::util::stemmer_util::StemmerUtil;

/// Light Stemmer for Russian.
///
/// This stemmer implements the following algorithm: *Indexing and Searching
/// Strategies for the Russian Language.* Ljiljana Dolamic and Jacques Savoy.
///
/// Equivalent to `org.apache.lucene.analysis.ru.RussianLightStemmer`. The
/// term it stems is the `char[]` of Java, UTF-16 units.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct RussianLightStemmer;

impl RussianLightStemmer {
    /// Equivalent to the implicit `RussianLightStemmer()`.
    pub(crate) fn new() -> Self {
        Self
    }

    /// Stems the first `len` units of `s` in place and returns the new
    /// length.
    ///
    /// Equivalent to `stem(char[], int)`.
    pub(crate) fn stem(&self, s: &mut [u16], len: usize) -> usize {
        let len = self.remove_case(s, len);
        self.normalize(s, len)
    }

    /// Equivalent to `normalize(char[], int)`.
    fn normalize(&self, s: &[u16], len: usize) -> usize {
        if len > 3 {
            match char::from_u32(u32::from(s[len - 1])) {
                Some('ь' | 'и') => return len - 1,
                Some('н') if s[len - 2] == 'н' as u16 => return len - 1,
                _ => {}
            }
        }
        len
    }

    /// Equivalent to `removeCase(char[], int)`.
    fn remove_case(&self, s: &[u16], len: usize) -> usize {
        let ends_with = |suffix: &str| StemmerUtil::ends_with(s, len, suffix);
        if len > 6 && (ends_with("иями") || ends_with("оями")) {
            return len - 4;
        }

        if len > 5
            && (ends_with("иям")
                || ends_with("иях")
                || ends_with("оях")
                || ends_with("ями")
                || ends_with("оям")
                || ends_with("оьв")
                || ends_with("ами")
                || ends_with("его")
                || ends_with("ему")
                || ends_with("ери")
                || ends_with("ими")
                || ends_with("ого")
                || ends_with("ому")
                || ends_with("ыми")
                || ends_with("оев"))
        {
            return len - 3;
        }

        if len > 4
            && (ends_with("ая")
                || ends_with("яя")
                || ends_with("ях")
                || ends_with("юю")
                || ends_with("ах")
                || ends_with("ею")
                || ends_with("их")
                || ends_with("ия")
                || ends_with("ию")
                || ends_with("ьв")
                || ends_with("ою")
                || ends_with("ую")
                || ends_with("ям")
                || ends_with("ых")
                || ends_with("ея")
                || ends_with("ам")
                || ends_with("ем")
                || ends_with("ей")
                || ends_with("ём")
                || ends_with("ев")
                || ends_with("ий")
                || ends_with("им")
                || ends_with("ое")
                || ends_with("ой")
                || ends_with("ом")
                || ends_with("ов")
                || ends_with("ые")
                || ends_with("ый")
                || ends_with("ым")
                || ends_with("ми"))
        {
            return len - 2;
        }

        if len > 3
            && matches!(
                char::from_u32(u32::from(s[len - 1])),
                Some('а' | 'е' | 'и' | 'о' | 'у' | 'й' | 'ы' | 'я' | 'ь')
            )
        {
            return len - 1;
        }

        len
    }
}
