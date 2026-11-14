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

//! Normalizer for Telugu, ported from
//! `org.apache.lucene.analysis.te.TeluguNormalizer`.

use crate::analysis::util::stemmer_util::StemmerUtil;

/// Normalizer for Telugu.
///
/// Normalizes text to remove some differences in spelling variations.
///
/// Equivalent to `org.apache.lucene.analysis.te.TeluguNormalizer` (since
/// 9.0.0). The term it normalizes is the `char[]` of Java, UTF-16 units.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct TeluguNormalizer;

impl TeluguNormalizer {
    /// Equivalent to the implicit `TeluguNormalizer()`.
    pub(crate) fn new() -> Self {
        Self
    }

    /// Normalize an input buffer of Telugu text
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
                // candrabindu (ఀ and ఁ) -> bindu (ం)
                0x0C00 | 0x0C01 => s[i] = 0x0C02, // ఀ ఁ -> ం
                // delete visarga (ః)
                0x0C03 => {
                    len = StemmerUtil::delete(s, i, len);
                    continue;
                }

                // zwj/zwnj -> delete
                0x200D | 0x200C => {
                    len = StemmerUtil::delete(s, i, len);
                    continue;
                }

                // long -> short vowels
                0x0C14 => s[i] = 0x0C13, // ఔ -> ఓ
                0x0C10 => s[i] = 0x0C0F, // ఐ -> ఏ
                0x0C06 => s[i] = 0x0C05, // ఆ -> అ
                0x0C08 => s[i] = 0x0C07, // ఈ -> ఇ
                0x0C0A => s[i] = 0x0C09, // ఊ -> ఉ

                // long -> short vowels matras
                0x0C40 => s[i] = 0x0C3F, // ీ -> ి
                0x0C42 => s[i] = 0x0C41, // ూ -> ు
                0x0C47 => s[i] = 0x0C46, // ే -> ె
                0x0C4B => s[i] = 0x0C4A, // ో -> ొ
                // decomposed dipthong (ె + ౖ) -> precomposed diphthong vowel sign (ై)
                0x0C46 => {
                    if i + 1 < len && s[i + 1] == 0x0C56 {
                        s[i] = 0x0C48;
                        len = StemmerUtil::delete(s, i + 1, len);
                    }
                }
                // composed oo or au -> oo or au
                0x0C12 => {
                    if i + 1 < len && s[i + 1] == 0x0C55 {
                        // (ఒ + ౕ) -> oo (ఓ)
                        s[i] = 0x0C13;
                        len = StemmerUtil::delete(s, i + 1, len);
                    } else if i + 1 < len && s[i + 1] == 0x0C4C {
                        // (ఒ + ౌ) -> au (ఔ)
                        s[i] = 0x0C14;
                        len = StemmerUtil::delete(s, i + 1, len);
                    }
                }

                _ => {}
            }
            i += 1;
        }

        len
    }
}
