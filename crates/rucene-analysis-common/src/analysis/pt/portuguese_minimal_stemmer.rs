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

//! Minimal stemmer for Portuguese, ported from
//! `org.apache.lucene.analysis.pt.PortugueseMinimalStemmer`.

use std::sync::LazyLock;

use super::rslp_stemmer_base::{RSLPStemmerBase, Step};

/// `PortugueseMinimalStemmer.pluralStep`, `parse(PortugueseMinimalStemmer.class, "portuguese.rslp").get("Plural")`.
///
/// # Panics
///
/// As Java's static initializer throws, when the resource cannot be parsed;
/// it is part of the crate, as it is of Java's JAR, and declares the step.
static PLURAL_STEP: LazyLock<Step> = LazyLock::new(|| {
    RSLPStemmerBase::parse(
        "org.apache.lucene.analysis.pt.PortugueseMinimalStemmer",
        "portuguese.rslp",
    )
    .unwrap_or_else(|e| panic!("{e}"))
    .remove("Plural")
    .expect("INVARIANT: portuguese.rslp declares the step Plural")
});

/// Minimal Stemmer for Portuguese
///
/// This follows the "RSLP-S" algorithm presented in: *A study on the Use of
/// Stemming for Monolingual Ad-Hoc Portuguese Information Retrieval*
/// (Orengo, et al) which is just the plural reduction step of the RSLP
/// algorithm from *A Stemming Algorithm for the Portuguese Language*, Orengo
/// et al.
///
/// * see: `RSLPStemmerBase`
///
/// Equivalent to `org.apache.lucene.analysis.pt.PortugueseMinimalStemmer`,
/// which extends `RSLPStemmerBase`. The term it stems is the `char[]` of
/// Java, UTF-16 units.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct PortugueseMinimalStemmer;

impl PortugueseMinimalStemmer {
    /// Equivalent to the implicit `PortugueseMinimalStemmer()`.
    pub(crate) fn new() -> Self {
        Self
    }

    /// Stems the first `len` units of `s` in place and returns the new
    /// length.
    ///
    /// Equivalent to `stem(char[], int)`.
    pub(crate) fn stem(&self, s: &mut [u16], len: usize) -> usize {
        PLURAL_STEP.apply(s, len)
    }
}
