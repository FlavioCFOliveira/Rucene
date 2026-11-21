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

//! Light stemmer for French, ported from
//! `org.apache.lucene.analysis.fr.FrenchLightStemmer`.

use crate::analysis::util::stemmer_util::StemmerUtil;
use crate::jdk::character::is_letter;

/// Light Stemmer for French.
///
/// This stemmer implements the "UniNE" algorithm in: *Light Stemming
/// Approaches for the French, Portuguese, German and Hungarian Languages*
/// Jacques Savoy
///
/// Equivalent to `org.apache.lucene.analysis.fr.FrenchLightStemmer`. The
/// term it stems is the `char[]` of Java, UTF-16 units.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct FrenchLightStemmer;

/// `StemmerUtil.endsWith(char[], int, String)`, statically imported by the
/// Java class.
fn ends_with(s: &[u16], len: usize, suffix: &str) -> bool {
    StemmerUtil::ends_with(s, len, suffix)
}

impl FrenchLightStemmer {
    /// Equivalent to the implicit `FrenchLightStemmer()`.
    pub(crate) fn new() -> Self {
        Self
    }

    /// Stems the first `len` units of `s` in place and returns the new
    /// length.
    ///
    /// Equivalent to `stem(char[], int)`.
    pub(crate) fn stem(&self, s: &mut [u16], mut len: usize) -> usize {
        if len > 5 && s[len - 1] == 'x' as u16 {
            if s[len - 3] == 'a' as u16 && s[len - 2] == 'u' as u16 && s[len - 4] != 'e' as u16 {
                s[len - 2] = 'l' as u16;
            }
            len -= 1;
        }

        if len > 3 && s[len - 1] == 'x' as u16 {
            len -= 1;
        }

        if len > 3 && s[len - 1] == 's' as u16 {
            len -= 1;
        }

        if len > 9 && ends_with(s, len, "issement") {
            len -= 6;
            s[len - 1] = 'r' as u16;
            return self.norm(s, len);
        }

        if len > 8 && ends_with(s, len, "issant") {
            len -= 4;
            s[len - 1] = 'r' as u16;
            return self.norm(s, len);
        }

        if len > 6 && ends_with(s, len, "ement") {
            len -= 4;
            if len > 3 && ends_with(s, len, "ive") {
                len -= 1;
                s[len - 1] = 'f' as u16;
            }
            return self.norm(s, len);
        }

        if len > 11 && ends_with(s, len, "ficatrice") {
            len -= 5;
            s[len - 2] = 'e' as u16;
            s[len - 1] = 'r' as u16;
            return self.norm(s, len);
        }

        if len > 10 && ends_with(s, len, "ficateur") {
            len -= 4;
            s[len - 2] = 'e' as u16;
            s[len - 1] = 'r' as u16;
            return self.norm(s, len);
        }

        if len > 9 && ends_with(s, len, "catrice") {
            len -= 3;
            s[len - 4] = 'q' as u16;
            s[len - 3] = 'u' as u16;
            s[len - 2] = 'e' as u16;
            // s[len-1] = 'r' <-- unnecessary, already 'r'.
            return self.norm(s, len);
        }

        if len > 8 && ends_with(s, len, "cateur") {
            len -= 2;
            s[len - 4] = 'q' as u16;
            s[len - 3] = 'u' as u16;
            s[len - 2] = 'e' as u16;
            s[len - 1] = 'r' as u16;
            return self.norm(s, len);
        }

        if len > 8 && ends_with(s, len, "atrice") {
            len -= 4;
            s[len - 2] = 'e' as u16;
            s[len - 1] = 'r' as u16;
            return self.norm(s, len);
        }

        if len > 7 && ends_with(s, len, "ateur") {
            len -= 3;
            s[len - 2] = 'e' as u16;
            s[len - 1] = 'r' as u16;
            return self.norm(s, len);
        }

        if len > 6 && ends_with(s, len, "trice") {
            len -= 1;
            s[len - 3] = 'e' as u16;
            s[len - 2] = 'u' as u16;
            s[len - 1] = 'r' as u16;
        }

        if len > 5 && ends_with(s, len, "ième") {
            return self.norm(s, len - 4);
        }

        if len > 7 && ends_with(s, len, "teuse") {
            len -= 2;
            s[len - 1] = 'r' as u16;
            return self.norm(s, len);
        }

        if len > 6 && ends_with(s, len, "teur") {
            len -= 1;
            s[len - 1] = 'r' as u16;
            return self.norm(s, len);
        }

        if len > 5 && ends_with(s, len, "euse") {
            return self.norm(s, len - 2);
        }

        if len > 8 && ends_with(s, len, "ère") {
            len -= 1;
            s[len - 2] = 'e' as u16;
            return self.norm(s, len);
        }

        if len > 7 && ends_with(s, len, "ive") {
            len -= 1;
            s[len - 1] = 'f' as u16;
            return self.norm(s, len);
        }

        if len > 4 && (ends_with(s, len, "folle") || ends_with(s, len, "molle")) {
            len -= 2;
            s[len - 1] = 'u' as u16;
            return self.norm(s, len);
        }

        if len > 9 && ends_with(s, len, "nnelle") {
            return self.norm(s, len - 5);
        }

        if len > 9 && ends_with(s, len, "nnel") {
            return self.norm(s, len - 3);
        }

        if len > 4 && ends_with(s, len, "ète") {
            len -= 1;
            s[len - 2] = 'e' as u16;
        }

        if len > 8 && ends_with(s, len, "ique") {
            len -= 4;
        }

        if len > 8 && ends_with(s, len, "esse") {
            return self.norm(s, len - 3);
        }

        if len > 7 && ends_with(s, len, "inage") {
            return self.norm(s, len - 3);
        }

        if len > 9 && ends_with(s, len, "isation") {
            len -= 7;
            if len > 5 && ends_with(s, len, "ual") {
                s[len - 2] = 'e' as u16;
            }
            return self.norm(s, len);
        }

        if len > 9 && ends_with(s, len, "isateur") {
            return self.norm(s, len - 7);
        }

        if len > 8 && ends_with(s, len, "ation") {
            return self.norm(s, len - 5);
        }

        if len > 8 && ends_with(s, len, "ition") {
            return self.norm(s, len - 5);
        }

        self.norm(s, len)
    }

    /// Equivalent to `norm(char[], int)`.
    fn norm(&self, s: &mut [u16], mut len: usize) -> usize {
        if len > 4 {
            for c in s.iter_mut().take(len) {
                match char::from_u32(u32::from(*c)) {
                    Some('à' | 'á' | 'â') => *c = 'a' as u16,
                    Some('ô') => *c = 'o' as u16,
                    Some('è' | 'é' | 'ê') => *c = 'e' as u16,
                    Some('ù' | 'û') => *c = 'u' as u16,
                    Some('î') => *c = 'i' as u16,
                    Some('ç') => *c = 'c' as u16,
                    _ => {}
                }
            }

            let mut ch = s[0];
            let mut i = 1;
            while i < len {
                if s[i] == ch && is_letter(i32::from(ch)) {
                    // Java's `delete(s, i--, len)`: the loop visits `i` again.
                    len = StemmerUtil::delete(s, i, len);
                } else {
                    ch = s[i];
                    i += 1;
                }
            }
        }

        if len > 4 && ends_with(s, len, "ie") {
            len -= 2;
        }

        if len > 4 {
            if s[len - 1] == 'r' as u16 {
                len -= 1;
            }
            if s[len - 1] == 'e' as u16 {
                len -= 1;
            }
            if s[len - 1] == 'e' as u16 {
                len -= 1;
            }
            if s[len - 1] == s[len - 2] && is_letter(i32::from(s[len - 1])) {
                len -= 1;
            }
        }
        len
    }
}
