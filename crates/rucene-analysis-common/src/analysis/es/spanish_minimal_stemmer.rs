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

//! Minimal plural stemmer for Spanish, ported from
//! `org.apache.lucene.analysis.es.SpanishMinimalStemmer`.

/// Minimal plural stemmer for Spanish.
///
/// This stemmer implements the "plurals" stemmer for spanish lanugage.
///
/// Equivalent to `org.apache.lucene.analysis.es.SpanishMinimalStemmer`. The
/// term it stems is the `char[]` of Java, UTF-16 units.
#[deprecated(note = "Use `SpanishPluralStemmer` instead.")]
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct SpanishMinimalStemmer;

#[allow(deprecated)]
impl SpanishMinimalStemmer {
    /// Equivalent to the implicit `SpanishMinimalStemmer()`.
    pub(crate) fn new() -> Self {
        Self
    }

    /// Stems the first `len` units of `s` in place and returns the new
    /// length.
    ///
    /// Equivalent to `stem(char[], int)`.
    pub(crate) fn stem(&self, s: &mut [u16], len: usize) -> usize {
        if len < 4 || s[len - 1] != 's' as u16 {
            return len;
        }

        for c in s.iter_mut().take(len) {
            match char::from_u32(u32::from(*c)) {
                Some('à' | 'á' | 'â' | 'ä') => *c = 'a' as u16,
                Some('ò' | 'ó' | 'ô' | 'ö') => *c = 'o' as u16,
                Some('è' | 'é' | 'ê' | 'ë') => *c = 'e' as u16,
                Some('ù' | 'ú' | 'û' | 'ü') => *c = 'u' as u16,
                Some('ì' | 'í' | 'î' | 'ï') => *c = 'i' as u16,
                Some('ñ') => *c = 'n' as u16,
                _ => {}
            }
        }

        if s[len - 1] == 's' as u16 {
            if s[len - 2] == 'a' as u16 || s[len - 2] == 'o' as u16 {
                return len - 1;
            }
            if s[len - 2] == 'e' as u16 {
                if s[len - 3] == 's' as u16 && s[len - 4] == 'e' as u16 {
                    return len - 2;
                }
                if s[len - 3] == 'c' as u16 {
                    s[len - 3] = 'z' as u16;
                    return len - 2;
                } else {
                    return len - 2;
                }
            } else {
                return len - 1;
            }
        }
        len
    }
}
