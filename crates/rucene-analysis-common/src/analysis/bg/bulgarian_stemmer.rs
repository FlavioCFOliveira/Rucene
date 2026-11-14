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

//! Light stemmer for Bulgarian, ported from
//! `org.apache.lucene.analysis.bg.BulgarianStemmer`.

use crate::analysis::util::stemmer_util::StemmerUtil;

/// Light Stemmer for Bulgarian.
///
/// Implements the algorithm described in: * Searching Strategies for the
/// Bulgarian Language * http://members.unine.ch/jacques.savoy/Papers/BUIR.pdf
///
/// Equivalent to `org.apache.lucene.analysis.bg.BulgarianStemmer`. The term
/// it stems is the `char[]` of Java, UTF-16 units.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct BulgarianStemmer;

impl BulgarianStemmer {
    /// Equivalent to the implicit `BulgarianStemmer()`.
    pub(crate) fn new() -> Self {
        Self
    }

    /// Stem an input buffer of Bulgarian text.
    ///
    /// * `s`: input buffer
    /// * `len`: length of input buffer
    /// * returns: length of input buffer after normalization
    ///
    /// Equivalent to `stem(char[], int)`.
    pub(crate) fn stem(&self, s: &mut [u16], len: usize) -> usize {
        if len < 4 {
            // do not stem
            return len;
        }

        if len > 5 && StemmerUtil::ends_with(s, len, "ища") {
            return len - 3;
        }

        let mut len = self.remove_article(s, len);
        len = self.remove_plural(s, len);

        if len > 3 {
            if StemmerUtil::ends_with(s, len, "я") {
                len -= 1;
            }
            if StemmerUtil::ends_with(s, len, "а")
                || StemmerUtil::ends_with(s, len, "о")
                || StemmerUtil::ends_with(s, len, "е")
            {
                len -= 1;
            }
        }

        // the rule to rewrite ен -> н is duplicated in the paper.
        // in the perl implementation referenced by the paper, this is fixed.
        // (it is fixed here as well)
        if len > 4 && StemmerUtil::ends_with(s, len, "ен") {
            s[len - 2] = 'н' as u16; // replace with н
            len -= 1;
        }

        if len > 5 && s[len - 2] == 'ъ' as u16 {
            s[len - 2] = s[len - 1]; // replace ъN with N
            len -= 1;
        }

        len
    }

    /// Mainly remove the definite article
    ///
    /// * `s`: input buffer
    /// * `len`: length of input buffer
    /// * returns: new stemmed length
    ///
    /// Equivalent to `removeArticle(char[], int)`.
    fn remove_article(&self, s: &[u16], len: usize) -> usize {
        if len > 6 && StemmerUtil::ends_with(s, len, "ият") {
            return len - 3;
        }

        if len > 5
            && (StemmerUtil::ends_with(s, len, "ът")
                || StemmerUtil::ends_with(s, len, "то")
                || StemmerUtil::ends_with(s, len, "те")
                || StemmerUtil::ends_with(s, len, "та")
                || StemmerUtil::ends_with(s, len, "ия"))
        {
            return len - 2;
        }

        if len > 4 && StemmerUtil::ends_with(s, len, "ят") {
            return len - 2;
        }

        len
    }

    /// Equivalent to `removePlural(char[], int)`.
    fn remove_plural(&self, s: &mut [u16], len: usize) -> usize {
        if len > 6 {
            if StemmerUtil::ends_with(s, len, "овци") {
                return len - 3; // replace with о
            }
            if StemmerUtil::ends_with(s, len, "ове") {
                return len - 3;
            }
            if StemmerUtil::ends_with(s, len, "еве") {
                s[len - 3] = 'й' as u16; // replace with й
                return len - 2;
            }
        }

        if len > 5 {
            if StemmerUtil::ends_with(s, len, "ища") {
                return len - 3;
            }
            if StemmerUtil::ends_with(s, len, "та") {
                return len - 2;
            }
            if StemmerUtil::ends_with(s, len, "ци") {
                s[len - 2] = 'к' as u16; // replace with к
                return len - 1;
            }
            if StemmerUtil::ends_with(s, len, "зи") {
                s[len - 2] = 'г' as u16; // replace with г
                return len - 1;
            }

            if s[len - 3] == 'е' as u16 && s[len - 1] == 'и' as u16 {
                s[len - 3] = 'я' as u16; // replace е with я, remove и
                return len - 1;
            }
        }

        if len > 4 {
            if StemmerUtil::ends_with(s, len, "си") {
                s[len - 2] = 'х' as u16; // replace with х
                return len - 1;
            }
            if StemmerUtil::ends_with(s, len, "и") {
                return len - 1;
            }
        }

        len
    }
}
