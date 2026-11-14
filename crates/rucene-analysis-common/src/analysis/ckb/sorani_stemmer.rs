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

//! Light stemmer for Sorani, ported from
//! `org.apache.lucene.analysis.ckb.SoraniStemmer`.

use crate::analysis::util::stemmer_util::StemmerUtil;

/// Light stemmer for Sorani
///
/// Equivalent to `org.apache.lucene.analysis.ckb.SoraniStemmer`. The term it
/// stems is the `char[]` of Java, UTF-16 units.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct SoraniStemmer;

impl SoraniStemmer {
    /// Equivalent to the implicit `SoraniStemmer()`.
    pub(crate) fn new() -> Self {
        Self
    }

    /// Stem an input buffer of Sorani text.
    ///
    /// * `s`: input buffer
    /// * `len`: length of input buffer
    /// * returns: length of input buffer after normalization
    ///
    /// Equivalent to `stem(char[], int)`, whose `if`/`else if` chains repeat
    /// a branch body where Java does.
    #[allow(clippy::if_same_then_else)]
    pub(crate) fn stem(&self, s: &mut [u16], len: usize) -> usize {
        let mut len = len;
        let ends_with = |len: usize, suffix: &str| StemmerUtil::ends_with(s, len, suffix);
        // postposition
        if len > 5 && ends_with(len, "دا") {
            len -= 2;
        } else if len > 4 && ends_with(len, "نا") {
            len -= 1;
        } else if len > 6 && ends_with(len, "ەوە") {
            len -= 3;
        }

        // possessive pronoun
        if len > 6 && (ends_with(len, "مان") || ends_with(len, "یان") || ends_with(len, "تان"))
        {
            len -= 3;
        }

        // indefinite singular ezafe
        if len > 6 && ends_with(len, "ێکی") {
            return len - 3;
        } else if len > 7 && ends_with(len, "یەکی") {
            return len - 4;
        }
        // indefinite singular
        if len > 5 && ends_with(len, "ێک") {
            return len - 2;
        } else if len > 6 && ends_with(len, "یەک") {
            return len - 3;
        }
        // definite singular
        else if len > 6 && ends_with(len, "ەکە") {
            return len - 3;
        } else if len > 5 && ends_with(len, "کە") {
            return len - 2;
        }
        // definite plural
        else if len > 7 && ends_with(len, "ەکان") {
            return len - 4;
        } else if len > 6 && ends_with(len, "کان") {
            return len - 3;
        }
        // indefinite plural ezafe
        else if len > 7 && ends_with(len, "یانی") {
            return len - 4;
        } else if len > 6 && ends_with(len, "انی") {
            return len - 3;
        }
        // indefinite plural
        else if len > 6 && ends_with(len, "یان") {
            return len - 3;
        } else if len > 5 && ends_with(len, "ان") {
            return len - 2;
        }
        // demonstrative plural
        else if len > 7 && ends_with(len, "یانە") {
            return len - 4;
        } else if len > 6 && ends_with(len, "انە") {
            return len - 3;
        }
        // demonstrative singular
        else if len > 5 && (ends_with(len, "ایە") || ends_with(len, "ەیە")) {
            return len - 2;
        } else if len > 4 && ends_with(len, "ە") {
            return len - 1;
        }
        // absolute singular ezafe
        else if len > 4 && ends_with(len, "ی") {
            return len - 1;
        }
        len
    }
}
