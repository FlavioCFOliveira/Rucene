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

//! Normalizer for Romanian, ported from
//! `org.apache.lucene.analysis.ro.RomanianNormalizer`.

pub(crate) const CAPITAL_S_WITH_COMMA_BELOW: u16 = 0x0218;
pub(crate) const SMALL_S_WITH_COMMA_BELOW: u16 = 0x0219;
pub(crate) const CAPITAL_T_WITH_COMMA_BELOW: u16 = 0x021A;
pub(crate) const SMALL_T_WITH_COMMA_BELOW: u16 = 0x021B;

pub(crate) const CAPITAL_S_WITH_CEDILLA: u16 = 0x015E;
pub(crate) const SMALL_S_WITH_CEDILLA: u16 = 0x015F;
pub(crate) const CAPITAL_T_WITH_CEDILLA: u16 = 0x0162;
pub(crate) const SMALL_T_WITH_CEDILLA: u16 = 0x0163;

/// Normalizer for Romanian.
///
/// Cedilla forms are normalized to forms with comma.
///
/// Equivalent to `org.apache.lucene.analysis.ro.RomanianNormalizer`; its
/// `static final char` constants are this module's constants. The buffer it
/// normalizes is the `char[]` of Java, UTF-16 units.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct RomanianNormalizer;

impl RomanianNormalizer {
    /// Equivalent to the implicit `RomanianNormalizer()`.
    pub(crate) fn new() -> Self {
        Self
    }

    /// Normalize an input buffer of Romanian text
    ///
    /// * `s`: input buffer
    /// * `len`: length of input buffer
    /// * returns: length of input buffer after normalization
    ///
    /// Equivalent to `normalize(char[], int)`.
    pub(crate) fn normalize(&self, s: &mut [u16], len: usize) -> usize {
        for c in s.iter_mut().take(len) {
            match *c {
                CAPITAL_S_WITH_CEDILLA => *c = CAPITAL_S_WITH_COMMA_BELOW,
                SMALL_S_WITH_CEDILLA => *c = SMALL_S_WITH_COMMA_BELOW,
                CAPITAL_T_WITH_CEDILLA => *c = CAPITAL_T_WITH_COMMA_BELOW,
                SMALL_T_WITH_CEDILLA => *c = SMALL_T_WITH_COMMA_BELOW,
                _ => {}
            }
        }

        len
    }
}
