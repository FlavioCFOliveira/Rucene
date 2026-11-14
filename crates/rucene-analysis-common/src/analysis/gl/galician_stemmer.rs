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

//! Galician stemmer implementing "Regras do lematizador para o galego",
//! ported from `org.apache.lucene.analysis.gl.GalicianStemmer`.

use std::sync::LazyLock;

use crate::analysis::pt::rslp_stemmer_base::{RSLPStemmerBase, Step};

/// The steps the static initializer of `GalicianStemmer` reads from
/// `parse(GalicianStemmer.class, "galician.rslp")`: Java's static fields
/// `plural`, `unification`, `adverb`, `augmentative`, `noun`, `verb`,
/// `vowel`.
#[derive(Debug)]
struct Steps {
    plural: Step,
    unification: Step,
    adverb: Step,
    augmentative: Step,
    noun: Step,
    verb: Step,
    vowel: Step,
}

/// The static initializer of `GalicianStemmer`.
///
/// # Panics
///
/// As Java's static initializer throws, when the resource cannot be parsed;
/// it is part of the crate, as it is of Java's JAR, and declares every step.
static STEPS: LazyLock<Steps> = LazyLock::new(|| {
    let mut steps = RSLPStemmerBase::parse(
        "org.apache.lucene.analysis.gl.GalicianStemmer",
        "galician.rslp",
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let mut get = |name: &str| {
        steps
            .remove(name)
            .unwrap_or_else(|| panic!("INVARIANT: galician.rslp declares the step {name}"))
    };
    Steps {
        plural: get("Plural"),
        unification: get("Unification"),
        adverb: get("Adverb"),
        augmentative: get("Augmentative"),
        noun: get("Noun"),
        verb: get("Verb"),
        vowel: get("Vowel"),
    }
});

/// Galician stemmer implementing "Regras do lematizador para o galego".
///
/// * see: `RSLPStemmerBase`
/// * see: [Description of rules](http://bvg.udc.es/recursos_lingua/stemming.jsp)
///
/// Equivalent to `org.apache.lucene.analysis.gl.GalicianStemmer`, which
/// extends `RSLPStemmerBase`. The term it stems is the `char[]` of Java,
/// UTF-16 units.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct GalicianStemmer;

impl GalicianStemmer {
    /// Equivalent to the implicit `GalicianStemmer()`.
    pub(crate) fn new() -> Self {
        Self
    }

    /// * `s`: buffer, oversized to at least `len+1`
    /// * `len`: initial valid length of buffer
    /// * returns: new valid length, stemmed
    ///
    /// Equivalent to `stem(char[], int)`.
    pub(crate) fn stem(&self, s: &mut [u16], mut len: usize) -> usize {
        debug_assert!(
            s.len() > len,
            "this stemmer requires an oversized array of at least 1"
        );
        let steps = &*STEPS;

        len = steps.plural.apply(s, len);
        len = steps.unification.apply(s, len);
        len = steps.adverb.apply(s, len);

        let mut oldlen;
        loop {
            oldlen = len;
            len = steps.augmentative.apply(s, len);
            if len == oldlen {
                break;
            }
        }

        oldlen = len;
        len = steps.noun.apply(s, len);
        if len == oldlen {
            /* suffix not removed */
            len = steps.verb.apply(s, len);
        }

        len = steps.vowel.apply(s, len);

        // RSLG accent removal
        for c in s.iter_mut().take(len) {
            match char::from_u32(u32::from(*c)) {
                Some('á') => *c = 'a' as u16,
                Some('é' | 'ê') => *c = 'e' as u16,
                Some('í') => *c = 'i' as u16,
                Some('ó') => *c = 'o' as u16,
                Some('ú') => *c = 'u' as u16,
                _ => {}
            }
        }

        len
    }
}
