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

//! Stemmer for Telugu, ported from
//! `org.apache.lucene.analysis.te.TeluguStemmer`.

use crate::analysis::util::stemmer_util::StemmerUtil;

/// Stemmer for Telugu.
///
/// Equivalent to `org.apache.lucene.analysis.te.TeluguStemmer` (since
/// 9.0.0). The term it stems is the `char[]` of Java, UTF-16 units.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct TeluguStemmer;

impl TeluguStemmer {
    /// Equivalent to the implicit `TeluguStemmer()`.
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
        // 4
        if len > 5 && (ends_with("ళ్ళు") || ends_with("డ్లు")) {
            return len - 4;
        }
        // 2
        if len > 3
            && (ends_with("డు")
                || ends_with("ము")
                || ends_with("వు")
                || ends_with("లు")
                || ends_with("ని")
                || ends_with("ను")
                || ends_with("చే")
                || ends_with("కై")
                || ends_with("లో")
                || ends_with("డు")
                || ends_with("ది")
                || ends_with("కి")
                || ends_with("సు")
                || ends_with("వై")
                || ends_with("పై"))
        {
            return len - 2;
        }
        // 1
        if len > 2
            && (ends_with("ి")
                || ends_with("ీ")
                || ends_with("ు")
                || ends_with("ూ")
                || ends_with("ె")
                || ends_with("ే")
                || ends_with("ొ")
                || ends_with("ో")
                || ends_with("ా"))
        {
            return len - 1;
        }
        len
    }
}
