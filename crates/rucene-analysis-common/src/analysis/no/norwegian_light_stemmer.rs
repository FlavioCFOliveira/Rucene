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

//! Light stemmer for Norwegian, ported from
//! `org.apache.lucene.analysis.no.NorwegianLightStemmer`.

use rucene::error::{LuceneError, Result};

use crate::analysis::util::stemmer_util::StemmerUtil;

/// `StemmerUtil.endsWith(char[], int, String)`, statically imported by the
/// Java class.
fn ends_with(s: &[u16], len: usize, suffix: &str) -> bool {
    StemmerUtil::ends_with(s, len, suffix)
}

/// Constant to remove Bokmål-specific endings
///
/// Equivalent to `NorwegianLightStemmer.BOKMAAL`.
///
/// Java's member is package-private, reached by the same package's tests;
/// Rust's integration tests are another crate, so the port makes it public
/// (hidden from the documentation).
#[doc(hidden)]
pub const BOKMAAL: i32 = 1;

/// Constant to remove Nynorsk-specific endings
///
/// Equivalent to `NorwegianLightStemmer.NYNORSK`.
///
/// Java's member is package-private, reached by the same package's tests;
/// Rust's integration tests are another crate, so the port makes it public
/// (hidden from the documentation).
#[doc(hidden)]
pub const NYNORSK: i32 = 2;

/// Light Stemmer for Norwegian.
///
/// Parts of this stemmer is adapted from SwedishLightStemFilter, except
/// that while the Swedish one has a pre-defined rule set and a
/// corresponding corpus to validate against whereas the Norwegian one is
/// hand crafted.
///
/// Equivalent to `org.apache.lucene.analysis.no.NorwegianLightStemmer`. The
/// term it stems is the `char[]` of Java, UTF-16 units.
#[derive(Debug, Clone, Copy)]
pub(crate) struct NorwegianLightStemmer {
    use_bokmaal: bool,
    use_nynorsk: bool,
}

impl NorwegianLightStemmer {
    /// Creates a new NorwegianLightStemmer
    ///
    /// * `flags`: set to [`BOKMAAL`], [`NYNORSK`], or both.
    ///
    /// Equivalent to `NorwegianLightStemmer(int)`.
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
        let (use_bokmaal, use_nynorsk) = (self.use_bokmaal, self.use_nynorsk);
        // Remove posessive -s (bilens -> bilen) and continue checking
        if len > 4 && s[len - 1] == 's' as u16 {
            len -= 1;
        }

        // Remove common endings, single-pass
        if len > 7
            && ((ends_with(s, len, "heter") && use_bokmaal)
                || // general ending (hemmelig-heter -> hemmelig)
                (ends_with(s, len, "heten") && use_bokmaal)
                || // general ending (hemmelig-heten -> hemmelig)
                (ends_with(s, len, "heita") && use_nynorsk))
        // general ending (hemmeleg-heita -> hemmeleg)
        {
            return len - 5;
        }

        // Remove Nynorsk common endings, single-pass
        if len > 8
            && use_nynorsk
            && (ends_with(s, len, "heiter")
                || // general ending (hemmeleg-heiter -> hemmeleg)
                ends_with(s, len, "leiken")
                || // general ending (trygg-leiken -> trygg)
                ends_with(s, len, "leikar"))
        // general ending (trygg-leikar -> trygg)
        {
            return len - 6;
        }

        if len > 5
            && (ends_with(s, len, "dom")
                || // general ending (kristen-dom -> kristen)
                (ends_with(s, len, "het") && use_bokmaal))
        // general ending (hemmelig-het -> hemmelig)
        {
            return len - 3;
        }

        if len > 6
            && use_nynorsk
            && (ends_with(s, len, "heit")
                || // general ending (hemmeleg-heit -> hemmeleg)
                ends_with(s, len, "semd")
                || // general ending (verk-semd -> verk)
                ends_with(s, len, "leik"))
        // general ending (trygg-leik -> trygg)
        {
            return len - 4;
        }

        if len > 7
            && (ends_with(s, len, "elser")
                || // general ending (føl-elser -> føl)
                ends_with(s, len, "elsen"))
        // general ending (føl-elsen -> føl)
        {
            return len - 5;
        }

        if len > 6
            && ((ends_with(s, len, "ende") && use_bokmaal)
                || // (sov-ende -> sov)
                (ends_with(s, len, "ande") && use_nynorsk)
                || // (sov-ande -> sov)
                ends_with(s, len, "else")
                || // general ending (føl-else -> føl)
                (ends_with(s, len, "este") && use_bokmaal)
                || // adj (fin-este -> fin)
                (ends_with(s, len, "aste") && use_nynorsk)
                || // adj (fin-aste -> fin)
                (ends_with(s, len, "eren") && use_bokmaal)
                || // masc
                (ends_with(s, len, "aren") && use_nynorsk))
        // masc
        {
            return len - 4;
        }

        if len > 5
            && ((ends_with(s, len, "ere") && use_bokmaal)
                || // adj (fin-ere -> fin)
                (ends_with(s, len, "are") && use_nynorsk)
                || // adj (fin-are -> fin)
                (ends_with(s, len, "est") && use_bokmaal)
                || // adj (fin-est -> fin)
                (ends_with(s, len, "ast") && use_nynorsk)
                || // adj (fin-ast -> fin)
                ends_with(s, len, "ene")
                || // masc/fem/neutr pl definite (hus-ene)
                (ends_with(s, len, "ane") && use_nynorsk))
        // masc pl definite (gut-ane)
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
                (ends_with(s, len, "ar") && use_nynorsk)
                || // masc pl indefinite
                (ends_with(s, len, "st") && use_bokmaal)
                || // adj (billig-st -> billig)
                ends_with(s, len, "te"))
        {
            return len - 2;
        }

        // 'a': fem definite
        // 'e': to get correct stem for nouns ending in -e (kake -> kak, kaker -> kak)
        if len > 3 && matches!(char::from_u32(u32::from(s[len - 1])), Some('a' | 'e' | 'n')) {
            return len - 1;
        }

        len
    }
}
