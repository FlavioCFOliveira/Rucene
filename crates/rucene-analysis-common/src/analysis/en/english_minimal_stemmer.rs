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

//! Minimal plural stemmer for English, ported from
//! `org.apache.lucene.analysis.en.EnglishMinimalStemmer`.

/// Minimal plural stemmer for English.
///
/// This stemmer implements the "S-Stemmer" from *How Effective Is
/// Suffixing?* Donna Harman.
///
/// Equivalent to `org.apache.lucene.analysis.en.EnglishMinimalStemmer`. The
/// term it stems is the `char[]` of Java, UTF-16 units.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct EnglishMinimalStemmer;

impl EnglishMinimalStemmer {
    /// Equivalent to the implicit `EnglishMinimalStemmer()`.
    pub(crate) fn new() -> Self {
        Self
    }

    /// Stems the first `len` units of `s` in place and returns the new
    /// length.
    ///
    /// Equivalent to `stem(char[], int)`; Java's `case 'e'` falls through to
    /// `default` when neither of its tests returns.
    pub(crate) fn stem(&self, s: &mut [u16], len: usize) -> usize {
        if len < 3 || s[len - 1] != 's' as u16 {
            return len;
        }

        match char::from_u32(u32::from(s[len - 2])) {
            Some('u' | 's') => len,
            Some('e') => {
                if len > 3
                    && s[len - 3] == 'i' as u16
                    && s[len - 4] != 'a' as u16
                    && s[len - 4] != 'e' as u16
                {
                    s[len - 3] = 'y' as u16;
                    return len - 2;
                }
                if s[len - 3] == 'i' as u16
                    || s[len - 3] == 'a' as u16
                    || s[len - 3] == 'o' as u16
                    || s[len - 3] == 'e' as u16
                {
                    return len;
                }
                // intentional fallthrough
                len - 1
            }
            _ => len - 1,
        }
    }
}
