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

//! Light stemmer for Czech, ported from
//! `org.apache.lucene.analysis.cz.CzechStemmer`.

use crate::analysis::util::stemmer_util::StemmerUtil;

/// Light Stemmer for Czech.
///
/// Implements the algorithm described in: * Indexing and stemming approaches
/// for the Czech language * http://portal.acm.org/citation.cfm?id=1598600
///
/// Equivalent to `org.apache.lucene.analysis.cz.CzechStemmer`. The term it
/// stems is the `char[]` of Java, UTF-16 units.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct CzechStemmer;

impl CzechStemmer {
    /// Equivalent to the implicit `CzechStemmer()`.
    pub(crate) fn new() -> Self {
        Self
    }

    /// Stem an input buffer of Czech text.
    ///
    /// * `s`: input buffer
    /// * `len`: length of input buffer
    /// * returns: length of input buffer after normalization
    ///
    /// **NOTE**: Input is expected to be in lowercase, but with diacritical
    /// marks
    ///
    /// Equivalent to `stem(char[], int)`.
    pub(crate) fn stem(&self, s: &mut [u16], len: usize) -> usize {
        let mut len = self.remove_case(s, len);
        len = self.remove_possessives(s, len);
        if len > 0 {
            len = self.normalize(s, len);
        }
        len
    }

    /// Equivalent to `removeCase(char[], int)`.
    fn remove_case(&self, s: &[u16], len: usize) -> usize {
        let ends_with = |suffix: &str| StemmerUtil::ends_with(s, len, suffix);
        if len > 7 && ends_with("atech") {
            return len - 5;
        }

        if len > 6 && (ends_with("ětem") || ends_with("etem") || ends_with("atům")) {
            return len - 4;
        }

        if len > 5
            && (ends_with("ech")
                || ends_with("ich")
                || ends_with("ích")
                || ends_with("ého")
                || ends_with("ěmi")
                || ends_with("emi")
                || ends_with("ému")
                || ends_with("ěte")
                || ends_with("ete")
                || ends_with("ěti")
                || ends_with("eti")
                || ends_with("ího")
                || ends_with("iho")
                || ends_with("ími")
                || ends_with("ímu")
                || ends_with("imu")
                || ends_with("ách")
                || ends_with("ata")
                || ends_with("aty")
                || ends_with("ých")
                || ends_with("ama")
                || ends_with("ami")
                || ends_with("ové")
                || ends_with("ovi")
                || ends_with("ými"))
        {
            return len - 3;
        }

        if len > 4
            && (ends_with("em")
                || ends_with("es")
                || ends_with("ém")
                || ends_with("ím")
                || ends_with("ům")
                || ends_with("at")
                || ends_with("ám")
                || ends_with("os")
                || ends_with("us")
                || ends_with("ým")
                || ends_with("mi")
                || ends_with("ou"))
        {
            return len - 2;
        }

        if len > 3
            && matches!(
                char::from_u32(u32::from(s[len - 1])),
                Some('a' | 'e' | 'i' | 'o' | 'u' | 'ů' | 'y' | 'á' | 'é' | 'í' | 'ý' | 'ě')
            )
        {
            return len - 1;
        }

        len
    }

    /// Equivalent to `removePossessives(char[], int)`.
    fn remove_possessives(&self, s: &[u16], len: usize) -> usize {
        if len > 5
            && (StemmerUtil::ends_with(s, len, "ov")
                || StemmerUtil::ends_with(s, len, "in")
                || StemmerUtil::ends_with(s, len, "ův"))
        {
            return len - 2;
        }

        len
    }

    /// Equivalent to `normalize(char[], int)`.
    fn normalize(&self, s: &mut [u16], len: usize) -> usize {
        if StemmerUtil::ends_with(s, len, "čt") {
            // čt -> ck
            s[len - 2] = 'c' as u16;
            s[len - 1] = 'k' as u16;
            return len;
        }

        if StemmerUtil::ends_with(s, len, "št") {
            // št -> sk
            s[len - 2] = 's' as u16;
            s[len - 1] = 'k' as u16;
            return len;
        }

        match char::from_u32(u32::from(s[len - 1])) {
            // [cč] -> k
            Some('c' | 'č') => {
                s[len - 1] = 'k' as u16;
                return len;
            }
            // [zž] -> h
            Some('z' | 'ž') => {
                s[len - 1] = 'h' as u16;
                return len;
            }
            _ => {}
        }

        if len > 1 && s[len - 2] == 'e' as u16 {
            s[len - 2] = s[len - 1]; // e* > *
            return len - 1;
        }

        if len > 2 && s[len - 2] == 'ů' as u16 {
            s[len - 2] = 'o' as u16; // *ů* -> *o*
            return len;
        }

        len
    }
}
