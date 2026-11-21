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

//! Minimal stemmer for Norwegian, ported from
//! `org.apache.lucene.analysis.no.NorwegianMinimalStemmer`.

use rucene::error::{LuceneError, Result};

use super::norwegian_light_stemmer::{BOKMAAL, NYNORSK};
use crate::analysis::util::stemmer_util::StemmerUtil;

/// `StemmerUtil.endsWith(char[], int, String)`, statically imported by the
/// Java class.
fn ends_with(s: &[u16], len: usize, suffix: &str) -> bool {
    StemmerUtil::ends_with(s, len, suffix)
}

/// Minimal Stemmer for Norwegian Bokmål (no-nb) and Nynorsk (no-nn)
///
/// Stems known plural forms for Norwegian nouns only, together with genitiv
/// -s
///
/// Equivalent to `org.apache.lucene.analysis.no.NorwegianMinimalStemmer`.
/// The term it stems is the `char[]` of Java, UTF-16 units.
#[derive(Debug, Clone, Copy)]
pub(crate) struct NorwegianMinimalStemmer {
    /// Java's field `useBokmaal`, which no method of the class reads.
    #[allow(dead_code)]
    use_bokmaal: bool,
    use_nynorsk: bool,
}

impl NorwegianMinimalStemmer {
    /// Creates a new NorwegianMinimalStemmer
    ///
    /// * `flags`: set to `NorwegianLightStemmer.BOKMAAL`,
    ///   `NorwegianLightStemmer.NYNORSK`, or both.
    ///
    /// Equivalent to `NorwegianMinimalStemmer(int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` "invalid flags" when `flags` is not
    /// positive or exceeds `BOKMAAL + NYNORSK`.
    pub(crate) fn new(flags: i32) -> Result<Self> {
        if flags <= 0 || flags > BOKMAAL + NYNORSK {
            return Err(LuceneError::illegal_argument("invalid flags"));
        }
        Ok(Self {
            use_bokmaal: (flags & BOKMAAL) != 0,
            use_nynorsk: (flags & NYNORSK) != 0,
        })
    }

    /// Stems the first `len` units of `s` and returns the new length.
    ///
    /// Equivalent to `stem(char[], int)`.
    pub(crate) fn stem(&self, s: &mut [u16], mut len: usize) -> usize {
        let use_nynorsk = self.use_nynorsk;
        // Remove genitiv s
        if len > 4 && s[len - 1] == 's' as u16 {
            len -= 1;
        }

        if len > 5
            && (ends_with(s, len, "ene")
                || // masc/fem/neutr pl definite (hus-ene)
                (ends_with(s, len, "ane") && use_nynorsk/* masc pl definite (gut-ane) */))
        {
            return len - 3;
        }

        if len > 4
            && (ends_with(s, len, "er")
                || // masc/fem indefinite
                ends_with(s, len, "en")
                || // masc/fem definite
                ends_with(s, len, "et")
                || // neutr definite
                (ends_with(s, len, "ar") && use_nynorsk/* masc pl indefinite */))
        {
            return len - 2;
        }

        // 'a': fem definite
        // 'e': to get correct stem for nouns ending in -e (kake -> kak, kaker -> kak)
        if len > 3 && matches!(char::from_u32(u32::from(s[len - 1])), Some('a' | 'e')) {
            return len - 1;
        }

        len
    }
}
