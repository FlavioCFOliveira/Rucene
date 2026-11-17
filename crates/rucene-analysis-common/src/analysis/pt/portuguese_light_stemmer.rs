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

use crate::analysis::util::stemmer_util::{StemmerUtil, TermUnit};

/// The UTF-16 unit of a `char` literal of the Java source, all of which are
/// in the Basic Multilingual Plane, as [`TermUnit::code`] reads it.
const fn unit(c: char) -> u32 {
    c as u32
}

// PERF: the Java `switch`es compare `char`s directly; these constants let the
// port compare the units the same way, where converting each unit to a Rust
// `char` first (`char::from_u32`, with its surrogate check) was extra work on
// every unit of every token (profile of task #546, round 3).
const LOWER_A: u32 = unit('a');
const LOWER_E: u32 = unit('e');
const LOWER_L: u32 = unit('l');
const LOWER_O: u32 = unit('o');
const LOWER_R: u32 = unit('r');
const LOWER_S: u32 = unit('s');
const LOWER_Z: u32 = unit('z');
const A_GRAVE: u32 = unit('à');
const A_ACUTE: u32 = unit('á');
const A_CIRCUMFLEX: u32 = unit('â');
const A_DIAERESIS: u32 = unit('ä');
const A_TILDE: u32 = unit('ã');
const O_GRAVE: u32 = unit('ò');
const O_ACUTE: u32 = unit('ó');
const O_CIRCUMFLEX: u32 = unit('ô');
const O_DIAERESIS: u32 = unit('ö');
const O_TILDE: u32 = unit('õ');
const E_GRAVE: u32 = unit('è');
const E_ACUTE: u32 = unit('é');
const E_CIRCUMFLEX: u32 = unit('ê');
const E_DIAERESIS: u32 = unit('ë');
const U_GRAVE: u32 = unit('ù');
const U_ACUTE: u32 = unit('ú');
const U_CIRCUMFLEX: u32 = unit('û');
const U_DIAERESIS: u32 = unit('ü');
const I_GRAVE: u32 = unit('ì');
const I_ACUTE: u32 = unit('í');
const I_CIRCUMFLEX: u32 = unit('î');
const I_DIAERESIS: u32 = unit('ï');
const C_CEDILLA: u32 = unit('ç');

/// `StemmerUtil.endsWith(char[], int, String)`, statically imported by the
/// Java class (its crate-private body, which also reads the `char` term
/// buffer).
fn ends_with<T: TermUnit>(s: &[T], len: usize, suffix: &str) -> bool {
    StemmerUtil::ends_with_units(s, len, suffix)
}

/// `s[i]`, the unit Java's `switch (s[i])` and `s[i] == 'c'` read.
fn at<T: TermUnit>(s: &[T], i: usize) -> u32 {
    s[i].code()
}

/// Light Stemmer for Portuguese
///
/// This stemmer implements the "UniNE" algorithm in: *Light Stemming
/// Approaches for the French, Portuguese, German and Hungarian Languages*
/// Jacques Savoy
///
/// Equivalent to `org.apache.lucene.analysis.pt.PortugueseLightStemmer`.
/// The term it stems is the `char[]` of Java, UTF-16 units: a `[u16]`, or
/// the `char` term buffer itself when every character in it is one unit
/// (see [`TermUnit`]), which the stemmer then edits in place as Java's does.
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
    pub(crate) fn stem<T: TermUnit>(&self, s: &mut [T], mut len: usize) -> usize {
        if len < 4 {
            return len;
        }

        len = self.remove_suffix(s, len);

        if len > 3 && at(s, len - 1) == LOWER_A {
            len = self.norm_feminine(s, len);
        }

        if len > 4 && matches!(at(s, len - 1), LOWER_E | LOWER_A | LOWER_O) {
            len -= 1;
        }

        for c in s.iter_mut().take(len) {
            match c.code() {
                A_GRAVE | A_ACUTE | A_CIRCUMFLEX | A_DIAERESIS | A_TILDE => *c = T::from_bmp('a'),
                O_GRAVE | O_ACUTE | O_CIRCUMFLEX | O_DIAERESIS | O_TILDE => *c = T::from_bmp('o'),
                E_GRAVE | E_ACUTE | E_CIRCUMFLEX | E_DIAERESIS => *c = T::from_bmp('e'),
                U_GRAVE | U_ACUTE | U_CIRCUMFLEX | U_DIAERESIS => *c = T::from_bmp('u'),
                I_GRAVE | I_ACUTE | I_CIRCUMFLEX | I_DIAERESIS => *c = T::from_bmp('i'),
                C_CEDILLA => *c = T::from_bmp('c'),
                _ => {}
            }
        }

        len
    }

    /// Equivalent to `removeSuffix(char[], int)`.
    fn remove_suffix<T: TermUnit>(&self, s: &mut [T], mut len: usize) -> usize {
        if len > 4
            && ends_with(s, len, "es")
            && matches!(at(s, len - 3), LOWER_R | LOWER_S | LOWER_L | LOWER_Z)
        {
            return len - 2;
        }

        if len > 3 && ends_with(s, len, "ns") {
            s[len - 2] = T::from_bmp('m');
            return len - 1;
        }

        if len > 4 && (ends_with(s, len, "eis") || ends_with(s, len, "éis")) {
            s[len - 3] = T::from_bmp('e');
            s[len - 2] = T::from_bmp('l');
            return len - 1;
        }

        if len > 4 && ends_with(s, len, "ais") {
            s[len - 2] = T::from_bmp('l');
            return len - 1;
        }

        if len > 4 && ends_with(s, len, "óis") {
            s[len - 3] = T::from_bmp('o');
            s[len - 2] = T::from_bmp('l');
            return len - 1;
        }

        if len > 4 && ends_with(s, len, "is") {
            s[len - 1] = T::from_bmp('l');
            return len;
        }

        if len > 3 && (ends_with(s, len, "ões") || ends_with(s, len, "ães")) {
            len -= 1;
            s[len - 2] = T::from_bmp('ã');
            s[len - 1] = T::from_bmp('o');
            return len;
        }

        if len > 6 && ends_with(s, len, "mente") {
            return len - 5;
        }

        if len > 3 && at(s, len - 1) == LOWER_S {
            return len - 1;
        }
        len
    }

    /// Equivalent to `normFeminine(char[], int)`.
    fn norm_feminine<T: TermUnit>(&self, s: &mut [T], len: usize) -> usize {
        if len > 7
            && (ends_with(s, len, "inha") || ends_with(s, len, "iaca") || ends_with(s, len, "eira"))
        {
            s[len - 1] = T::from_bmp('o');
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
                s[len - 1] = T::from_bmp('o');
                return len;
            }

            if ends_with(s, len, "ona") {
                s[len - 3] = T::from_bmp('ã');
                s[len - 2] = T::from_bmp('o');
                return len - 1;
            }

            if ends_with(s, len, "ora") {
                return len - 1;
            }

            if ends_with(s, len, "esa") {
                s[len - 3] = T::from_bmp('ê');
                return len - 1;
            }

            if ends_with(s, len, "na") {
                s[len - 1] = T::from_bmp('o');
                return len;
            }
        }
        len
    }
}

#[cfg(test)]
mod tests {
    use super::PortugueseLightStemmer;

    /// Stemming the `char` term buffer in place gives the stem of its UTF-16
    /// units, for words of the Basic Multilingual Plane that reach every
    /// rule (every suffix of `removeSuffix` and `normFeminine`, every letter
    /// the final loop folds, lengths around each rule's bound).
    #[test]
    fn stem_over_chars_equals_stem_over_units() {
        let stems = [
            "",
            "a",
            "ab",
            "abc",
            "c",
            "can",
            "casa",
            "anel",
            "papel",
            "lanç",
            "pão",
            "ĀĀ",
            "àáâäãòóôöõ",
            "èéêëùúûüìíîïç",
            "rapida",
            "imens",
            "portuguê",
        ];
        let suffixes = [
            "", "a", "e", "o", "s", "x", "es", "res", "ses", "les", "zes", "ns", "eis", "éis",
            "ais", "óis", "is", "ões", "ães", "mente", "inha", "iaca", "eira", "osa", "ica", "ida",
            "ada", "iva", "ama", "ona", "ora", "esa", "na", "ã", "ç",
        ];
        let stemmer = PortugueseLightStemmer::new();
        let mut changed = 0;
        for stem in stems {
            for suffix in suffixes {
                let word = format!("{stem}{suffix}");
                let mut units: Vec<u16> = word.encode_utf16().collect();
                let mut chars: Vec<char> = word.chars().collect();
                assert_eq!(units.len(), chars.len(), "{word:?} is in the BMP");
                let (ulen, clen) = (units.len(), chars.len());
                let unit_len = stemmer.stem(&mut units, ulen);
                let char_len = stemmer.stem(&mut chars, clen);
                assert_eq!(char_len, unit_len, "{word:?}");
                let from_units = String::from_utf16(&units[..unit_len]).expect("BMP units");
                let from_chars: String = chars[..char_len].iter().collect();
                assert_eq!(from_chars, from_units, "{word:?}");
                changed += usize::from(from_units != word);
            }
        }
        assert!(changed > 300, "only {changed} words were stemmed");
    }

    /// `PortugueseLightStemFilter` gives every term the stem of its UTF-16
    /// units, decoded (the conversion it made before stemming in place):
    /// terms of the Basic Multilingual Plane, stemmed in place, and terms
    /// holding supplementary characters, stemmed over their units.
    #[test]
    fn filter_stems_as_the_utf16_round_trip() {
        use rucene::analysis::{CharTermAttribute, StringCharReader, TokenStream};

        use crate::analysis::core::whitespace_tokenizer::new_whitespace_tokenizer;
        use crate::analysis::pt::portuguese_light_stem_filter::new_portuguese_light_stem_filter;

        let words = [
            "ações",
            "papéis",
            "anzóis",
            "rapidamente",
            "casinha",
            "famosa",
            "leões",
            "pães",
            "bons",
            "anéis",
            "cidadã",
            "ab",
            "x😀ões",
            "😀papéis",
            "pães😀",
            "😀😀s",
            "a😀mente",
            "famosa😀",
            "\u{10FFFF}inha",
            "ÿĀÿĀs",
            "comboios",
            "português",
            "portuguesa",
        ];
        let reference = |word: &str| {
            let mut units: Vec<u16> = word.encode_utf16().collect();
            let len = units.len();
            let newlen = PortugueseLightStemmer::new().stem(&mut units, len);
            char::decode_utf16(units[..newlen].iter().copied())
                .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
                .collect::<String>()
        };
        let mut tokenizer = new_whitespace_tokenizer();
        tokenizer
            .set_reader(Box::new(StringCharReader::new(words.join(" "))))
            .expect("reader");
        let mut filter = new_portuguese_light_stem_filter(Box::new(tokenizer)).expect("filter");
        filter.reset().expect("reset");
        let mut terms = Vec::new();
        while filter.increment_token().expect("token") {
            let term = filter
                .attribute_source()
                .get_attribute_interface::<dyn CharTermAttribute>()
                .expect("term attribute");
            terms.push(term.buffer()[..term.length()].iter().collect::<String>());
        }
        filter.end().expect("end");
        filter.close().expect("close");
        let expected: Vec<String> = words.iter().map(|w| reference(w)).collect();
        assert_eq!(terms, expected);
        assert!(terms.iter().zip(words).filter(|(t, w)| t != w).count() > 15);
    }
}
