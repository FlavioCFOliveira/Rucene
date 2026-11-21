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

//! Stemmer for Bengali, ported from
//! `org.apache.lucene.analysis.bn.BengaliStemmer`.

use crate::analysis::util::stemmer_util::StemmerUtil;

/// Stemmer for Bengali.
///
/// The algorithm is based on the report in: *Natural Language Processing in
/// an Indian Language (Bengali)-I: Verb Phrase Analysis* P Sengupta and B B
/// Chaudhuri
///
/// Few Stemmer criteria are taken from:
/// *http://members.unine.ch/jacques.savoy/clef/BengaliStemmerLight.java.txt*
///
/// Equivalent to `org.apache.lucene.analysis.bn.BengaliStemmer`. The term it
/// stems is the `char[]` of Java, UTF-16 units.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct BengaliStemmer;

impl BengaliStemmer {
    /// Equivalent to the implicit `BengaliStemmer()`.
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
        // 8
        if len > 9
            && (ends_with("িয়াছিলাম")
                || ends_with("িতেছিলাম")
                || ends_with("িতেছিলেন")
                || ends_with("ইতেছিলেন")
                || ends_with("িয়াছিলেন")
                || ends_with("ইয়াছিলেন"))
        {
            return len - 8;
        }
        // 7
        if len > 8
            && (ends_with("িতেছিলি")
                || ends_with("িতেছিলে")
                || ends_with("িয়াছিলা")
                || ends_with("িয়াছিলে")
                || ends_with("িতেছিলা")
                || ends_with("িয়াছিলি")
                || ends_with("য়েদেরকে"))
        {
            return len - 7;
        }
        // 6
        if len > 7
            && (ends_with("িতেছিস")
                || ends_with("িতেছেন")
                || ends_with("িয়াছিস")
                || ends_with("িয়াছেন")
                || ends_with("েছিলাম")
                || ends_with("েছিলেন")
                || ends_with("েদেরকে"))
        {
            return len - 6;
        }
        // 5
        if len > 6
            && (ends_with("িতেছি")
                || ends_with("িতেছা")
                || ends_with("িতেছে")
                || ends_with("ছিলাম")
                || ends_with("ছিলেন")
                || ends_with("িয়াছি")
                || ends_with("িয়াছা")
                || ends_with("িয়াছে")
                || ends_with("েছিলে")
                || ends_with("েছিলা")
                || ends_with("য়েদের")
                || ends_with("দেরকে"))
        {
            return len - 5;
        }
        // 4
        if len > 5
            && (ends_with("িলাম")
                || ends_with("িলেন")
                || ends_with("িতাম")
                || ends_with("িতেন")
                || ends_with("িবেন")
                || ends_with("ছিলি")
                || ends_with("ছিলে")
                || ends_with("ছিলা")
                || ends_with("তেছে")
                || ends_with("িতেছ")
                || ends_with("খানা")
                || ends_with("খানি")
                || ends_with("গুলো")
                || ends_with("গুলি")
                || ends_with("য়েরা")
                || ends_with("েদের"))
        {
            return len - 4;
        }
        // 3
        if len > 4
            && (ends_with("লাম")
                || ends_with("িলি")
                || ends_with("ইলি")
                || ends_with("িলে")
                || ends_with("ইলে")
                || ends_with("লেন")
                || ends_with("িলা")
                || ends_with("ইলা")
                || ends_with("তাম")
                || ends_with("িতি")
                || ends_with("ইতি")
                || ends_with("িতে")
                || ends_with("ইতে")
                || ends_with("তেন")
                || ends_with("িতা")
                || ends_with("িবা")
                || ends_with("ইবা")
                || ends_with("িবি")
                || ends_with("ইবি")
                || ends_with("বেন")
                || ends_with("িবে")
                || ends_with("ইবে")
                || ends_with("ছেন")
                || ends_with("য়োন")
                || ends_with("য়ের")
                || ends_with("েরা")
                || ends_with("দের"))
        {
            return len - 3;
        }
        // 2
        if len > 3
            && (ends_with("িস")
                || ends_with("েন")
                || ends_with("লি")
                || ends_with("লে")
                || ends_with("লা")
                || ends_with("তি")
                || ends_with("তে")
                || ends_with("তা")
                || ends_with("বি")
                || ends_with("বে")
                || ends_with("বা")
                || ends_with("ছি")
                || ends_with("ছা")
                || ends_with("ছে")
                || ends_with("ুন")
                || ends_with("ুক")
                || ends_with("টা")
                || ends_with("টি")
                || ends_with("নি")
                || ends_with("ের")
                || ends_with("তে")
                || ends_with("রা")
                || ends_with("কে"))
        {
            return len - 2;
        }
        // 1
        if len > 2
            && (ends_with("ি")
                || ends_with("ী")
                || ends_with("া")
                || ends_with("ো")
                || ends_with("ে")
                || ends_with("ব")
                || ends_with("ত"))
        {
            return len - 1;
        }
        len
    }
}
