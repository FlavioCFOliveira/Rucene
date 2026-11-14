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

//! Light stemmer for Hindi, ported from
//! `org.apache.lucene.analysis.hi.HindiStemmer`.

use crate::analysis::util::stemmer_util::StemmerUtil;

/// Light Stemmer for Hindi.
///
/// Implements the algorithm specified in: *A Lightweight Stemmer for Hindi*
/// Ananthakrishnan Ramanathan and Durgesh D Rao.
/// http://computing.open.ac.uk/Sites/EACLSouthAsia/Papers/p6-Ramanathan.pdf
///
/// Equivalent to `org.apache.lucene.analysis.hi.HindiStemmer`. The term it
/// stems is the `char[]` of Java, UTF-16 units.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct HindiStemmer;

impl HindiStemmer {
    /// Equivalent to the implicit `HindiStemmer()`.
    pub(crate) fn new() -> Self {
        Self
    }

    /// Stems the first `len` units of `buffer` and returns the new length;
    /// the stemmer only shortens the term (Java's `char[]` parameter, which
    /// its callers hand over for writing as to every stemmer).
    ///
    /// Equivalent to `stem(char[], int)`.
    pub(crate) fn stem(&self, buffer: &mut [u16], len: usize) -> usize {
        let buffer: &[u16] = buffer;
        let ends_with = |suffix: &str| StemmerUtil::ends_with(buffer, len, suffix);
        // 5
        if len > 6
            && (ends_with("ाएंगी")
                || ends_with("ाएंगे")
                || ends_with("ाऊंगी")
                || ends_with("ाऊंगा")
                || ends_with("ाइयाँ")
                || ends_with("ाइयों")
                || ends_with("ाइयां"))
        {
            return len - 5;
        }
        // 4
        if len > 5
            && (ends_with("ाएगी")
                || ends_with("ाएगा")
                || ends_with("ाओगी")
                || ends_with("ाओगे")
                || ends_with("एंगी")
                || ends_with("ेंगी")
                || ends_with("एंगे")
                || ends_with("ेंगे")
                || ends_with("ूंगी")
                || ends_with("ूंगा")
                || ends_with("ातीं")
                || ends_with("नाओं")
                || ends_with("नाएं")
                || ends_with("ताओं")
                || ends_with("ताएं")
                || ends_with("ियाँ")
                || ends_with("ियों")
                || ends_with("ियां"))
        {
            return len - 4;
        }
        // 3
        if len > 4
            && (ends_with("ाकर")
                || ends_with("ाइए")
                || ends_with("ाईं")
                || ends_with("ाया")
                || ends_with("ेगी")
                || ends_with("ेगा")
                || ends_with("ोगी")
                || ends_with("ोगे")
                || ends_with("ाने")
                || ends_with("ाना")
                || ends_with("ाते")
                || ends_with("ाती")
                || ends_with("ाता")
                || ends_with("तीं")
                || ends_with("ाओं")
                || ends_with("ाएं")
                || ends_with("ुओं")
                || ends_with("ुएं")
                || ends_with("ुआं"))
        {
            return len - 3;
        }
        // 2
        if len > 3
            && (ends_with("कर")
                || ends_with("ाओ")
                || ends_with("िए")
                || ends_with("ाई")
                || ends_with("ाए")
                || ends_with("ने")
                || ends_with("नी")
                || ends_with("ना")
                || ends_with("ते")
                || ends_with("ीं")
                || ends_with("ती")
                || ends_with("ता")
                || ends_with("ाँ")
                || ends_with("ां")
                || ends_with("ों")
                || ends_with("ें"))
        {
            return len - 2;
        }
        // 1
        if len > 2
            && (ends_with("ो")
                || ends_with("े")
                || ends_with("ू")
                || ends_with("ु")
                || ends_with("ी")
                || ends_with("ि")
                || ends_with("ा"))
        {
            return len - 1;
        }
        len
    }
}
