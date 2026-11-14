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

//! Normalizer for Arabic, ported from
//! `org.apache.lucene.analysis.ar.ArabicNormalizer`.

use crate::analysis::util::stemmer_util::StemmerUtil;

/// Normalizer for Arabic.
///
/// Normalization is done in-place for efficiency, operating on a termbuffer.
///
/// Normalization is defined as:
///
/// * Normalization of hamza with alef seat to a bare alef.
/// * Normalization of teh marbuta to heh
/// * Normalization of dotless yeh (alef maksura) to yeh.
/// * Removal of Arabic diacritics (the harakat)
/// * Removal of tatweel (stretching character).
///
/// Equivalent to `org.apache.lucene.analysis.ar.ArabicNormalizer`. The term
/// it normalizes is the `char[]` of Java, UTF-16 units.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct ArabicNormalizer;

impl ArabicNormalizer {
    pub(crate) const ALEF: u16 = 0x0627;
    pub(crate) const ALEF_MADDA: u16 = 0x0622;
    pub(crate) const ALEF_HAMZA_ABOVE: u16 = 0x0623;
    pub(crate) const ALEF_HAMZA_BELOW: u16 = 0x0625;

    pub(crate) const YEH: u16 = 0x064A;
    pub(crate) const DOTLESS_YEH: u16 = 0x0649;

    pub(crate) const TEH_MARBUTA: u16 = 0x0629;
    pub(crate) const HEH: u16 = 0x0647;

    pub(crate) const TATWEEL: u16 = 0x0640;

    pub(crate) const FATHATAN: u16 = 0x064B;
    pub(crate) const DAMMATAN: u16 = 0x064C;
    pub(crate) const KASRATAN: u16 = 0x064D;
    pub(crate) const FATHA: u16 = 0x064E;
    pub(crate) const DAMMA: u16 = 0x064F;
    pub(crate) const KASRA: u16 = 0x0650;
    pub(crate) const SHADDA: u16 = 0x0651;
    pub(crate) const SUKUN: u16 = 0x0652;

    /// Equivalent to the implicit `ArabicNormalizer()`.
    pub(crate) fn new() -> Self {
        Self
    }

    /// Normalize an input buffer of Arabic text
    ///
    /// * `s`: input buffer
    /// * `len`: length of input buffer
    /// * returns: length of input buffer after normalization
    ///
    /// Equivalent to `normalize(char[], int)`.
    pub(crate) fn normalize(&self, s: &mut [u16], len: usize) -> usize {
        let mut len = len;
        let mut i = 0;
        while i < len {
            match s[i] {
                Self::ALEF_MADDA | Self::ALEF_HAMZA_ABOVE | Self::ALEF_HAMZA_BELOW => {
                    s[i] = Self::ALEF;
                }
                Self::DOTLESS_YEH => s[i] = Self::YEH,
                Self::TEH_MARBUTA => s[i] = Self::HEH,
                Self::TATWEEL
                | Self::KASRATAN
                | Self::DAMMATAN
                | Self::FATHATAN
                | Self::FATHA
                | Self::DAMMA
                | Self::KASRA
                | Self::SHADDA
                | Self::SUKUN => {
                    // Java's `len = delete(s, i, len); i--;`: the next unit
                    // moves to `i`, which the loop visits again.
                    len = StemmerUtil::delete(s, i, len);
                    continue;
                }
                _ => {}
            }
            i += 1;
        }

        len
    }
}
