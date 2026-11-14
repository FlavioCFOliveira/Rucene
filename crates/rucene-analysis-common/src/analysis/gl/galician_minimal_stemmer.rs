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

//! Minimal stemmer for Galician, ported from
//! `org.apache.lucene.analysis.gl.GalicianMinimalStemmer`.

use std::sync::LazyLock;

use crate::analysis::pt::rslp_stemmer_base::{RSLPStemmerBase, Step};

/// `GalicianMinimalStemmer.pluralStep`, `parse(GalicianMinimalStemmer.class, "galician.rslp").get("Plural")`.
///
/// # Panics
///
/// As Java's static initializer throws, when the resource cannot be parsed;
/// it is part of the crate, as it is of Java's JAR, and declares the step.
static PLURAL_STEP: LazyLock<Step> = LazyLock::new(|| {
    RSLPStemmerBase::parse(
        "org.apache.lucene.analysis.gl.GalicianMinimalStemmer",
        "galician.rslp",
    )
    .unwrap_or_else(|e| panic!("{e}"))
    .remove("Plural")
    .expect("INVARIANT: galician.rslp declares the step Plural")
});

/// Minimal Stemmer for Galician
///
/// This follows the "RSLP-S" algorithm, but modified for Galician. Hence
/// this stemmer only applies the plural reduction step of: "Regras do
/// lematizador para o galego"
///
/// * see: `RSLPStemmerBase`
///
/// Equivalent to `org.apache.lucene.analysis.gl.GalicianMinimalStemmer`,
/// which extends `RSLPStemmerBase`. The term it stems is the `char[]` of
/// Java, UTF-16 units.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct GalicianMinimalStemmer;

impl GalicianMinimalStemmer {
    /// Equivalent to the implicit `GalicianMinimalStemmer()`.
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
