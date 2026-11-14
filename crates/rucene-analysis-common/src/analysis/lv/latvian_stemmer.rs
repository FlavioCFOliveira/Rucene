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

//! Light stemmer for Latvian, ported from
//! `org.apache.lucene.analysis.lv.LatvianStemmer`.

use std::sync::LazyLock;

use crate::analysis::util::stemmer_util::StemmerUtil;

/// Light stemmer for Latvian.
///
/// This is a light version of the algorithm in Karlis Kreslin's PhD thesis
/// *A stemming algorithm for Latvian* with the following modifications:
///
/// * Only explicitly stems noun and adjective morphology
/// * Stricter length/vowel checks for the resulting stems (verb etc suffix
///   stripping is removed)
/// * Removes only the primary inflectional suffixes: case and number for
///   nouns ; case, number, gender, and definitiveness for adjectives.
/// * Palatalization is only handled when a declension II,V,VI noun suffix is
///   removed.
///
/// Equivalent to `org.apache.lucene.analysis.lv.LatvianStemmer`. The term it
/// stems is the `char[]` of Java, UTF-16 units.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct LatvianStemmer;

/// Equivalent to `LatvianStemmer.affixes`.
pub(crate) static AFFIXES: LazyLock<Vec<Affix>> = LazyLock::new(|| {
    vec![
        Affix::new("ajiem", 3, false),
        Affix::new("ajai", 3, false),
        Affix::new("ajam", 2, false),
        Affix::new("ajām", 2, false),
        Affix::new("ajos", 2, false),
        Affix::new("ajās", 2, false),
        Affix::new("iem", 2, true),
        Affix::new("ajā", 2, false),
        Affix::new("ais", 2, false),
        Affix::new("ai", 2, false),
        Affix::new("ei", 2, false),
        Affix::new("ām", 1, false),
        Affix::new("am", 1, false),
        Affix::new("ēm", 1, false),
        Affix::new("īm", 1, false),
        Affix::new("im", 1, false),
        Affix::new("um", 1, false),
        Affix::new("us", 1, true),
        Affix::new("as", 1, false),
        Affix::new("ās", 1, false),
        Affix::new("es", 1, false),
        Affix::new("os", 1, true),
        Affix::new("ij", 1, false),
        Affix::new("īs", 1, false),
        Affix::new("ēs", 1, false),
        Affix::new("is", 1, false),
        Affix::new("ie", 1, false),
        Affix::new("u", 1, true),
        Affix::new("a", 1, true),
        Affix::new("i", 1, true),
        Affix::new("e", 1, false),
        Affix::new("ā", 1, false),
        Affix::new("ē", 1, false),
        Affix::new("ī", 1, false),
        Affix::new("ū", 1, false),
        Affix::new("o", 1, false),
        Affix::new("s", 0, false),
        Affix::new("š", 0, false),
    ]
});

/// Equivalent to `org.apache.lucene.analysis.lv.LatvianStemmer.Affix`.
#[derive(Debug, Clone)]
pub(crate) struct Affix {
    /// suffix
    pub(crate) affix: Vec<u16>,
    /// vowel count of the suffix
    pub(crate) vc: i32,
    /// true if we should fire palatalization rules.
    pub(crate) palatalizes: bool,
}

impl Affix {
    /// Equivalent to `Affix(String, int, boolean)`: `affix.toCharArray()`.
    pub(crate) fn new(affix: &str, vc: i32, palatalizes: bool) -> Self {
        Self {
            affix: affix.encode_utf16().collect(),
            vc,
            palatalizes,
        }
    }
}

impl LatvianStemmer {
    /// Equivalent to the implicit `LatvianStemmer()`.
    pub(crate) fn new() -> Self {
        Self
    }

    /// Stem a latvian word. returns the new adjusted length.
    ///
    /// Equivalent to `stem(char[], int)`.
    pub(crate) fn stem(&self, s: &mut [u16], len: usize) -> usize {
        let num_vowels = self.num_vowels(s, len);

        for affix in AFFIXES.iter() {
            if num_vowels > affix.vc
                && len >= affix.affix.len() + 3
                && StemmerUtil::ends_with_chars(s, len, &affix.affix)
            {
                let len = len - affix.affix.len();
                return if affix.palatalizes {
                    self.unpalatalize(s, len)
                } else {
                    len
                };
            }
        }

        len
    }

    /// Most cases are handled except for the ambiguous ones:
    ///
    /// * s -> š
    /// * t -> š
    /// * d -> ž
    /// * z -> ž
    ///
    /// Equivalent to `unpalatalize(char[], int)`.
    fn unpalatalize(&self, s: &mut [u16], len: usize) -> usize {
        let mut len = len;
        // we check the character removed: if it's -u then
        // it's 2,5, or 6 gen pl., and these two can only apply then.
        if s[len] == 'u' as u16 {
            // kš -> kst
            if StemmerUtil::ends_with(s, len, "kš") {
                len += 1;
                s[len - 2] = 's' as u16;
                s[len - 1] = 't' as u16;
                return len;
            }
            // ņņ -> nn
            if StemmerUtil::ends_with(s, len, "ņņ") {
                s[len - 2] = 'n' as u16;
                s[len - 1] = 'n' as u16;
                return len;
            }
        }

        // otherwise all other rules
        if StemmerUtil::ends_with(s, len, "pj")
            || StemmerUtil::ends_with(s, len, "bj")
            || StemmerUtil::ends_with(s, len, "mj")
            || StemmerUtil::ends_with(s, len, "vj")
        {
            // labial consonant
            return len - 1;
        } else if StemmerUtil::ends_with(s, len, "šņ") {
            s[len - 2] = 's' as u16;
            s[len - 1] = 'n' as u16;
            return len;
        } else if StemmerUtil::ends_with(s, len, "žņ") {
            s[len - 2] = 'z' as u16;
            s[len - 1] = 'n' as u16;
            return len;
        } else if StemmerUtil::ends_with(s, len, "šļ") {
            s[len - 2] = 's' as u16;
            s[len - 1] = 'l' as u16;
            return len;
        } else if StemmerUtil::ends_with(s, len, "žļ") {
            s[len - 2] = 'z' as u16;
            s[len - 1] = 'l' as u16;
            return len;
        } else if StemmerUtil::ends_with(s, len, "ļņ") {
            s[len - 2] = 'l' as u16;
            s[len - 1] = 'n' as u16;
            return len;
        } else if StemmerUtil::ends_with(s, len, "ļļ") {
            s[len - 2] = 'l' as u16;
            s[len - 1] = 'l' as u16;
            return len;
        } else if s[len - 1] == 'č' as u16 {
            s[len - 1] = 'c' as u16;
            return len;
        } else if s[len - 1] == 'ļ' as u16 {
            s[len - 1] = 'l' as u16;
            return len;
        } else if s[len - 1] == 'ņ' as u16 {
            s[len - 1] = 'n' as u16;
            return len;
        }

        len
    }

    /// Count the vowels in the string, we always require at least one in the
    /// remaining stem to accept it.
    ///
    /// Equivalent to `numVowels(char[], int)`.
    fn num_vowels(&self, s: &[u16], len: usize) -> i32 {
        let mut n = 0;
        for &c in &s[..len] {
            if matches!(
                char::from_u32(u32::from(c)),
                Some('a' | 'e' | 'i' | 'o' | 'u' | 'ā' | 'ī' | 'ē' | 'ū')
            ) {
                n += 1;
            }
        }
        n
    }
}
