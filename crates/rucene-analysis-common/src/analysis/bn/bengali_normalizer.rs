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

//! Normalizer for Bengali, ported from
//! `org.apache.lucene.analysis.bn.BengaliNormalizer`.

use crate::analysis::util::stemmer_util::StemmerUtil;

/// Normalizer for Bengali.
///
/// Implements the Bengali-language specific algorithm specified in: *A
/// Double Metaphone encoding for Bangla and its application in spelling
/// checker* Naushad UzZaman and Mumit Khan.
/// http://www.panl10n.net/english/final%20reports/pdf%20files/Bangladesh/BAN16.pdf
///
/// Equivalent to `org.apache.lucene.analysis.bn.BengaliNormalizer`. The term
/// it normalizes is the `char[]` of Java, UTF-16 units.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct BengaliNormalizer;

impl BengaliNormalizer {
    /// Equivalent to the implicit `BengaliNormalizer()`.
    pub(crate) fn new() -> Self {
        Self
    }

    /// Normalize an input buffer of Bengali text
    ///
    /// * `s`: input buffer
    /// * `len`: length of input buffer
    /// * returns: length of input buffer after normalization
    ///
    /// Equivalent to `normalize(char[], int)`. Java's `int i` goes down to
    /// `-1` before the loop's `i++` in the phala cases (`i -= 2` at index 1),
    /// so the index here is signed as Java's; it is non-negative wherever it
    /// indexes `s`.
    pub(crate) fn normalize(&self, s: &mut [u16], len: usize) -> usize {
        let mut len = len;
        let mut i: isize = 0;
        while i < len as isize {
            let u = i as usize;
            match s[u] {
                // delete Chandrabindu
                0x0981 => {
                    len = StemmerUtil::delete(s, u, len);
                    i -= 1;
                }

                // DirghoI kar -> RosshoI kar
                0x09C0 => s[u] = 0x09BF,

                // DirghoU kar -> RosshoU kar
                0x09C2 => s[u] = 0x09C1,

                // Khio (Ka + Hoshonto + Murdorno Sh)
                0x0995 => {
                    if u + 2 < len && s[u + 1] == 0x09CD && s[u + 2] == 0x09BF {
                        if i == 0 {
                            s[u] = 0x0996;
                            len = StemmerUtil::delete(s, u + 2, len);
                            len = StemmerUtil::delete(s, u + 1, len);
                        } else {
                            s[u + 1] = 0x0996;
                            len = StemmerUtil::delete(s, u + 2, len);
                        }
                    }
                }

                // Nga to Anusvara
                0x0999 => s[u] = 0x0982,

                // Ja Phala
                0x09AF => {
                    if i - 2 == 0 && s[u - 1] == 0x09CD {
                        s[u - 1] = 0x09C7;

                        if u + 1 < len && s[u + 1] == 0x09BE {
                            len = StemmerUtil::delete(s, u + 1, len);
                        }
                        len = StemmerUtil::delete(s, u, len);
                        i -= 1;
                    } else if i > 0 && s[u - 1] == 0x09CD {
                        // Java's `i - 1 >= 0`.
                        len = StemmerUtil::delete(s, u, len);
                        len = StemmerUtil::delete(s, u - 1, len);
                        i -= 2;
                    }
                }

                // Ba Phalaa
                0x09AC => {
                    if !((i >= 1 && s[u - 1] != 0x09CD) || i == 0) {
                        // Java's first two branches do the same, kept apart
                        // as Java keeps them.
                        #[allow(clippy::if_same_then_else)]
                        if i - 2 == 0 {
                            len = StemmerUtil::delete(s, u, len);
                            len = StemmerUtil::delete(s, u - 1, len);
                            i -= 2;
                        } else if i - 5 >= 0 && s[u - 3] == 0x09CD {
                            len = StemmerUtil::delete(s, u, len);
                            len = StemmerUtil::delete(s, u - 1, len);
                            i -= 2;
                        } else if i - 2 >= 0 {
                            s[u - 1] = s[u - 2];
                            len = StemmerUtil::delete(s, u, len);
                            i -= 1;
                        }
                    }
                }

                // Visarga
                0x0983 => {
                    if u == len - 1 {
                        if len <= 3 {
                            s[u] = 0x09B9;
                        } else {
                            len = StemmerUtil::delete(s, u, len);
                        }
                    } else {
                        s[u] = s[u + 1];
                    }
                }

                // All sh
                0x09B6 | 0x09B7 => s[u] = 0x09B8,

                // check na
                0x09A3 => s[u] = 0x09A8,

                // check ra
                0x09DC | 0x09DD => s[u] = 0x09B0,

                0x09CE => s[u] = 0x09A4,

                _ => {}
            }
            i += 1;
        }

        len
    }
}
