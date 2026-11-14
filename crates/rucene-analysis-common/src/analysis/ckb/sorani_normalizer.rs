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

//! Normalizes the Unicode representation of Sorani text, ported from
//! `org.apache.lucene.analysis.ckb.SoraniNormalizer`.

use crate::analysis::util::stemmer_util::StemmerUtil;
use crate::jdk::character::{get_type, FORMAT};

/// Normalizes the Unicode representation of Sorani text.
///
/// Normalization consists of:
///
/// * Alternate forms of 'y' (0064, 0649) are converted to 06CC (FARSI YEH)
/// * Alternate form of 'k' (0643) is converted to 06A9 (KEHEH)
/// * Alternate forms of vowel 'e' (0647+200C, word-final 0647, 0629) are
///   converted to 06D5 (AE)
/// * Alternate (joining) form of 'h' (06BE) is converted to 0647
/// * Alternate forms of 'rr' (0692, word-initial 0631) are converted to 0695
///   (REH WITH SMALL V BELOW)
/// * Harakat, tatweel, and formatting characters such as directional controls
///   are removed.
///
/// Equivalent to `org.apache.lucene.analysis.ckb.SoraniNormalizer`. The term
/// it normalizes is the `char[]` of Java, UTF-16 units.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct SoraniNormalizer;

impl SoraniNormalizer {
    pub(crate) const YEH: u16 = 0x064A;
    pub(crate) const DOTLESS_YEH: u16 = 0x0649;
    pub(crate) const FARSI_YEH: u16 = 0x06CC;

    pub(crate) const KAF: u16 = 0x0643;
    pub(crate) const KEHEH: u16 = 0x06A9;

    pub(crate) const HEH: u16 = 0x0647;
    pub(crate) const AE: u16 = 0x06D5;
    pub(crate) const ZWNJ: u16 = 0x200C;
    pub(crate) const HEH_DOACHASHMEE: u16 = 0x06BE;
    pub(crate) const TEH_MARBUTA: u16 = 0x0629;

    pub(crate) const REH: u16 = 0x0631;
    pub(crate) const RREH: u16 = 0x0695;
    pub(crate) const RREH_ABOVE: u16 = 0x0692;

    pub(crate) const TATWEEL: u16 = 0x0640;
    pub(crate) const FATHATAN: u16 = 0x064B;
    pub(crate) const DAMMATAN: u16 = 0x064C;
    pub(crate) const KASRATAN: u16 = 0x064D;
    pub(crate) const FATHA: u16 = 0x064E;
    pub(crate) const DAMMA: u16 = 0x064F;
    pub(crate) const KASRA: u16 = 0x0650;
    pub(crate) const SHADDA: u16 = 0x0651;
    pub(crate) const SUKUN: u16 = 0x0652;

    /// Equivalent to the implicit `SoraniNormalizer()`.
    pub(crate) fn new() -> Self {
        Self
    }

    /// Normalize an input buffer of Sorani text
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
                Self::YEH | Self::DOTLESS_YEH => s[i] = Self::FARSI_YEH,
                Self::KAF => s[i] = Self::KEHEH,
                Self::ZWNJ => {
                    if i > 0 && s[i - 1] == Self::HEH {
                        s[i - 1] = Self::AE;
                    }
                    len = StemmerUtil::delete(s, i, len);
                    continue;
                }
                Self::HEH => {
                    if i == len - 1 {
                        s[i] = Self::AE;
                    }
                }
                Self::TEH_MARBUTA => s[i] = Self::AE,
                Self::HEH_DOACHASHMEE => s[i] = Self::HEH,
                Self::REH => {
                    if i == 0 {
                        s[i] = Self::RREH;
                    }
                }
                Self::RREH_ABOVE => s[i] = Self::RREH,
                Self::TATWEEL
                | Self::KASRATAN
                | Self::DAMMATAN
                | Self::FATHATAN
                | Self::FATHA
                | Self::DAMMA
                | Self::KASRA
                | Self::SHADDA
                | Self::SUKUN => {
                    len = StemmerUtil::delete(s, i, len);
                    continue;
                }
                _ => {
                    // `Character.getType(char)`: a surrogate unit is
                    // `SURROGATE`, never `FORMAT`.
                    if get_type(i32::from(s[i])) == FORMAT {
                        len = StemmerUtil::delete(s, i, len);
                        continue;
                    }
                }
            }
            i += 1;
        }
        len
    }
}
