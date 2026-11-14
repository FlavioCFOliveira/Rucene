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

//! Normalizer for Hindi, ported from
//! `org.apache.lucene.analysis.hi.HindiNormalizer`.

use crate::analysis::util::stemmer_util::StemmerUtil;

/// Normalizer for Hindi.
///
/// Normalizes text to remove some differences in spelling variations.
///
/// Implements the Hindi-language specific algorithm specified in: *Word
/// normalization in Indian languages* Prasad Pingali and Vasudeva Varma.
/// http://web2py.iiit.ac.in/publications/default/download/inproceedings.pdf.3fe5b38c-02ee-41ce-9a8f-3e745670be32.pdf
///
/// with the following additions from *Hindi CLIR in Thirty Days* Leah S.
/// Larkey, Margaret E. Connell, and Nasreen AbdulJaleel.
/// http://maroo.cs.umass.edu/pub/web/getpdf.php?id=454:
///
/// * Internal Zero-width joiner and Zero-width non-joiners are removed
/// * In addition to chandrabindu, NA+halant is normalized to anusvara
///
/// Equivalent to `org.apache.lucene.analysis.hi.HindiNormalizer`. The term
/// it normalizes is the `char[]` of Java, UTF-16 units.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct HindiNormalizer;

impl HindiNormalizer {
    /// Equivalent to the implicit `HindiNormalizer()`.
    pub(crate) fn new() -> Self {
        Self
    }

    /// Normalize an input buffer of Hindi text
    ///
    /// * `s`: input buffer
    /// * `len`: length of input buffer
    /// * returns: length of input buffer after normalization
    ///
    /// Equivalent to `normalize(char[], int)`. Where Java runs
    /// `len = delete(s, i, len); i--;`, the next unit moves to `i`, which the
    /// loop visits again.
    pub(crate) fn normalize(&self, s: &mut [u16], len: usize) -> usize {
        let mut len = len;
        let mut i = 0;
        while i < len {
            match s[i] {
                // dead n -> bindu
                0x0928 => {
                    if i + 1 < len && s[i + 1] == 0x094D {
                        s[i] = 0x0902;
                        len = StemmerUtil::delete(s, i + 1, len);
                    }
                }
                // candrabindu -> bindu
                0x0901 => s[i] = 0x0902,
                // nukta deletions
                0x093C => {
                    len = StemmerUtil::delete(s, i, len);
                    continue;
                }
                0x0929 => s[i] = 0x0928,
                0x0931 => s[i] = 0x0930,
                0x0934 => s[i] = 0x0933,
                0x0958 => s[i] = 0x0915,
                0x0959 => s[i] = 0x0916,
                0x095A => s[i] = 0x0917,
                0x095B => s[i] = 0x091C,
                0x095C => s[i] = 0x0921,
                0x095D => s[i] = 0x0922,
                0x095E => s[i] = 0x092B,
                0x095F => s[i] = 0x092F,
                // zwj/zwnj -> delete
                0x200D | 0x200C => {
                    len = StemmerUtil::delete(s, i, len);
                    continue;
                }
                // virama -> delete
                0x094D => {
                    len = StemmerUtil::delete(s, i, len);
                    continue;
                }
                // chandra/short -> replace
                0x0945 | 0x0946 => s[i] = 0x0947,
                0x0949 | 0x094A => s[i] = 0x094B,
                0x090D | 0x090E => s[i] = 0x090F,
                0x0911 | 0x0912 => s[i] = 0x0913,
                0x0972 => s[i] = 0x0905,
                // long -> short ind. vowels
                0x0906 => s[i] = 0x0905,
                0x0908 => s[i] = 0x0907,
                0x090A => s[i] = 0x0909,
                0x0960 => s[i] = 0x090B,
                0x0961 => s[i] = 0x090C,
                0x0910 => s[i] = 0x090F,
                0x0914 => s[i] = 0x0913,
                // long -> short dep. vowels
                0x0940 => s[i] = 0x093F,
                0x0942 => s[i] = 0x0941,
                0x0944 => s[i] = 0x0943,
                0x0963 => s[i] = 0x0962,
                0x0948 => s[i] = 0x0947,
                0x094C => s[i] = 0x094B,
                _ => {}
            }
            i += 1;
        }

        len
    }
}
