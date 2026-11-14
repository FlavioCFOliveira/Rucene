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

//! Normalizer for Persian, ported from
//! `org.apache.lucene.analysis.fa.PersianNormalizer`.

use crate::analysis::util::stemmer_util::StemmerUtil;

/// Normalizer for Persian.
///
/// Normalization is done in-place for efficiency, operating on a termbuffer.
///
/// Normalization is defined as:
///
/// * Normalization of various heh + hamza forms and heh goal to heh.
/// * Normalization of farsi yeh and yeh barree to arabic yeh
/// * Normalization of persian keheh to arabic kaf
///
/// Equivalent to `org.apache.lucene.analysis.fa.PersianNormalizer`. The term
/// it normalizes is the `char[]` of Java, UTF-16 units.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct PersianNormalizer;

impl PersianNormalizer {
    pub(crate) const YEH: u16 = 0x064A;

    pub(crate) const FARSI_YEH: u16 = 0x06CC;

    pub(crate) const YEH_BARREE: u16 = 0x06D2;

    pub(crate) const KEHEH: u16 = 0x06A9;

    pub(crate) const KAF: u16 = 0x0643;

    pub(crate) const HAMZA_ABOVE: u16 = 0x0654;

    pub(crate) const HEH_YEH: u16 = 0x06C0;

    pub(crate) const HEH_GOAL: u16 = 0x06C1;

    pub(crate) const HEH: u16 = 0x0647;

    /// Equivalent to the implicit `PersianNormalizer()`.
    pub(crate) fn new() -> Self {
        Self
    }

    /// Normalize an input buffer of Persian text
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
                Self::FARSI_YEH | Self::YEH_BARREE => s[i] = Self::YEH,
                Self::KEHEH => s[i] = Self::KAF,
                Self::HEH_YEH | Self::HEH_GOAL => s[i] = Self::HEH,
                Self::HAMZA_ABOVE => {
                    // necessary for HEH + HAMZA
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
