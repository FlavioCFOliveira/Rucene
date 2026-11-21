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

//! A stemmer for German words, ported from
//! `org.apache.lucene.analysis.de.GermanStemmer`.

// This file is encoded in UTF-8

use std::sync::LazyLock;

use crate::jdk::character::is_letter;
use crate::jdk::locale::Locale;
use crate::jdk::string::{equals_str, to_lower_case};

/// `GermanStemmer.locale`, `new Locale.Builder().setLanguageTag("de-DE").build()`.
static LOCALE: LazyLock<Locale> = LazyLock::new(|| {
    Locale::builder_from_language_tag("de-DE").expect("INVARIANT: de-DE is a well-formed tag")
});

/// A stemmer for German words.
///
/// The algorithm is based on the report "A Fast and Simple Stemming
/// Algorithm for German Words" by Jörg Caumanns (joerg.caumanns at
/// isst.fhg.de).
///
/// Equivalent to `org.apache.lucene.analysis.de.GermanStemmer`. Its
/// `String`s and `StringBuilder` are held as UTF-16 units.
#[derive(Debug, Default, Clone)]
pub(crate) struct GermanStemmer {
    /// Buffer for the terms while stemming them.
    sb: Vec<u16>,
    /// Amount of characters that are removed with `substitute()` while
    /// stemming.
    subst_count: usize,
}

impl GermanStemmer {
    /// Equivalent to the implicit `GermanStemmer()`.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Stemms the given term to an unique `discriminator`.
    ///
    /// Equivalent to `stem(String)`: `term` is the term that should be
    /// stemmed; returns the discriminator for `term`.
    pub(crate) fn stem(&mut self, term: &[u16]) -> Vec<u16> {
        // Use lowercase for medium stemming.
        let term = to_lower_case(term, &LOCALE);
        if !self.is_stemmable(&term) {
            return term;
        }
        // Reset the StringBuilder.
        let mut sb = std::mem::take(&mut self.sb);
        sb.clear();
        sb.extend_from_slice(&term);
        // Stemming starts here...
        self.substitute(&mut sb);
        self.strip(&mut sb);
        self.optimize(&mut sb);
        self.resubstitute(&mut sb);
        self.remove_particle_denotion(&mut sb);
        let result = sb.clone();
        self.sb = sb;
        result
    }

    /// Checks if a term could be stemmed: true if, and only if, the given
    /// term consists in letters.
    ///
    /// Equivalent to `isStemmable(String)`.
    fn is_stemmable(&self, term: &[u16]) -> bool {
        term.iter().all(|&c| is_letter(i32::from(c)))
    }

    /// suffix stripping (stemming) on the current term. The stripping is
    /// reduced to the seven "base" suffixes "e", "s", "n", "t", "em", "er"
    /// and * "nd", from which all regular suffixes are build of. The
    /// simplification causes some overstemming, and way more irregular
    /// stems, but still provides unique. discriminators in the most of those
    /// cases. The algorithm is context free, except of the length
    /// restrictions.
    ///
    /// Equivalent to `strip(StringBuilder)`, whose `if`/`else` chain is kept
    /// branch for branch although several branches do the same.
    #[allow(clippy::if_same_then_else)]
    fn strip(&self, buffer: &mut Vec<u16>) {
        let mut do_more = true;
        while do_more && buffer.len() > 3 {
            let len = buffer.len();
            if (len + self.subst_count > 5) && equals_str(&buffer[len - 2..len], "nd") {
                buffer.truncate(len - 2);
            } else if (len + self.subst_count > 4) && equals_str(&buffer[len - 2..len], "em") {
                buffer.truncate(len - 2);
            } else if (len + self.subst_count > 4) && equals_str(&buffer[len - 2..len], "er") {
                buffer.truncate(len - 2);
            } else if buffer[len - 1] == 'e' as u16 {
                buffer.truncate(len - 1);
            } else if buffer[len - 1] == 's' as u16 {
                buffer.truncate(len - 1);
            } else if buffer[len - 1] == 'n' as u16 {
                buffer.truncate(len - 1);
            }
            // "t" occurs only as suffix of verbs.
            else if buffer[len - 1] == 't' as u16 {
                buffer.truncate(len - 1);
            } else {
                do_more = false;
            }
        }
    }

    /// Does some optimizations on the term. This optimisations are
    /// contextual.
    ///
    /// Equivalent to `optimize(StringBuilder)`.
    fn optimize(&self, buffer: &mut Vec<u16>) {
        // Additional step for female plurals of professions and inhabitants.
        if buffer.len() > 5 && equals_str(&buffer[buffer.len() - 5..], "erin*") {
            buffer.pop();
            self.strip(buffer);
        }
        // Additional step for irregular plural nouns like "Matrizen -> Matrix".
        // NOTE: this length constraint is probably not a great value, it's just to prevent AIOOBE on
        // empty terms
        if let Some(last) = buffer.last_mut() {
            if *last == 'z' as u16 {
                *last = 'x' as u16;
            }
        }
    }

    /// Removes a particle denotion ("ge") from a term.
    ///
    /// Equivalent to `removeParticleDenotion(StringBuilder)`.
    fn remove_particle_denotion(&self, buffer: &mut Vec<u16>) {
        if buffer.len() > 4 {
            for c in 0..buffer.len() - 3 {
                if equals_str(&buffer[c..c + 4], "gege") {
                    buffer.drain(c..c + 2);
                    return;
                }
            }
        }
    }

    /// Do some substitutions for the term to reduce overstemming:
    ///
    /// - Substitute Umlauts with their corresponding vowel: `äöü -> aou`,
    ///   "ß" is substituted by "ss" - Substitute a second char of a pair of
    ///   equal characters with an asterisk: `?? -> ?*` - Substitute some
    ///   common character combinations with a token: `sch/ch/ei/ie/ig/st ->
    ///   $/§/%/&/#/!`
    ///
    /// Equivalent to `substitute(StringBuilder)`.
    fn substitute(&mut self, buffer: &mut Vec<u16>) {
        self.subst_count = 0;
        let mut c = 0;
        while c < buffer.len() {
            // Replace the second char of a pair of the equal characters with an asterisk
            if c > 0 && buffer[c] == buffer[c - 1] {
                buffer[c] = '*' as u16;
            }
            // Substitute Umlauts.
            else if buffer[c] == 'ä' as u16 {
                buffer[c] = 'a' as u16;
            } else if buffer[c] == 'ö' as u16 {
                buffer[c] = 'o' as u16;
            } else if buffer[c] == 'ü' as u16 {
                buffer[c] = 'u' as u16;
            }
            // Fix bug so that 'ß' at the end of a word is replaced.
            else if buffer[c] == 'ß' as u16 {
                buffer[c] = 's' as u16;
                buffer.insert(c + 1, 's' as u16);
                self.subst_count += 1;
            }
            // Take care that at least one character is left left side from the current one
            if c + 1 < buffer.len() {
                // Masking several common character combinations with an token
                if (c + 2 < buffer.len())
                    && buffer[c] == 's' as u16
                    && buffer[c + 1] == 'c' as u16
                    && buffer[c + 2] == 'h' as u16
                {
                    buffer[c] = '$' as u16;
                    buffer.drain(c + 1..c + 3);
                    self.subst_count += 2;
                } else if buffer[c] == 'c' as u16 && buffer[c + 1] == 'h' as u16 {
                    buffer[c] = '§' as u16;
                    buffer.remove(c + 1);
                    self.subst_count += 1;
                } else if buffer[c] == 'e' as u16 && buffer[c + 1] == 'i' as u16 {
                    buffer[c] = '%' as u16;
                    buffer.remove(c + 1);
                    self.subst_count += 1;
                } else if buffer[c] == 'i' as u16 && buffer[c + 1] == 'e' as u16 {
                    buffer[c] = '&' as u16;
                    buffer.remove(c + 1);
                    self.subst_count += 1;
                } else if buffer[c] == 'i' as u16 && buffer[c + 1] == 'g' as u16 {
                    buffer[c] = '#' as u16;
                    buffer.remove(c + 1);
                    self.subst_count += 1;
                } else if buffer[c] == 's' as u16 && buffer[c + 1] == 't' as u16 {
                    buffer[c] = '!' as u16;
                    buffer.remove(c + 1);
                    self.subst_count += 1;
                }
            }
            c += 1;
        }
    }

    /// Undoes the changes made by substitute(). That are character pairs
    /// and character combinations. Umlauts will remain as their
    /// corresponding vowel, as "ß" remains as "ss".
    ///
    /// Equivalent to `resubstitute(StringBuilder)`.
    fn resubstitute(&self, buffer: &mut Vec<u16>) {
        let mut c = 0;
        while c < buffer.len() {
            if buffer[c] == '*' as u16 {
                let x = buffer[c - 1];
                buffer[c] = x;
            } else if buffer[c] == '$' as u16 {
                buffer[c] = 's' as u16;
                buffer.splice(c + 1..c + 1, ['c' as u16, 'h' as u16]);
            } else if buffer[c] == '§' as u16 {
                buffer[c] = 'c' as u16;
                buffer.insert(c + 1, 'h' as u16);
            } else if buffer[c] == '%' as u16 {
                buffer[c] = 'e' as u16;
                buffer.insert(c + 1, 'i' as u16);
            } else if buffer[c] == '&' as u16 {
                buffer[c] = 'i' as u16;
                buffer.insert(c + 1, 'e' as u16);
            } else if buffer[c] == '#' as u16 {
                buffer[c] = 'i' as u16;
                buffer.insert(c + 1, 'g' as u16);
            } else if buffer[c] == '!' as u16 {
                buffer[c] = 's' as u16;
                buffer.insert(c + 1, 't' as u16);
            }
            c += 1;
        }
    }
}
