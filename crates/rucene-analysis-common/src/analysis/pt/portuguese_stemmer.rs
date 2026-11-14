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

//! Portuguese stemmer implementing the RSLP algorithm, ported from
//! `org.apache.lucene.analysis.pt.PortugueseStemmer`.

use std::sync::LazyLock;

use super::rslp_stemmer_base::{RSLPStemmerBase, Step};

/// The steps the static initializer of `PortugueseStemmer` reads from
/// `parse(PortugueseStemmer.class, "portuguese.rslp")`: Java's static fields
/// `plural`, `feminine`, `adverb`, `augmentative`, `noun`, `verb`, `vowel`.
#[derive(Debug)]
struct Steps {
    plural: Step,
    feminine: Step,
    adverb: Step,
    augmentative: Step,
    noun: Step,
    verb: Step,
    vowel: Step,
}

/// The static initializer of `PortugueseStemmer`.
///
/// # Panics
///
/// As Java's static initializer throws, when the resource cannot be parsed;
/// it is part of the crate, as it is of Java's JAR, and declares every step.
static STEPS: LazyLock<Steps> = LazyLock::new(|| {
    let mut steps = RSLPStemmerBase::parse(
        "org.apache.lucene.analysis.pt.PortugueseStemmer",
        "portuguese.rslp",
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let mut get = |name: &str| {
        steps
            .remove(name)
            .unwrap_or_else(|| panic!("INVARIANT: portuguese.rslp declares the step {name}"))
    };
    Steps {
        plural: get("Plural"),
        feminine: get("Feminine"),
        adverb: get("Adverb"),
        augmentative: get("Augmentative"),
        noun: get("Noun"),
        verb: get("Verb"),
        vowel: get("Vowel"),
    }
});

/// Portuguese stemmer implementing the RSLP (Removedor de Sufixos da Lingua
/// Portuguesa) algorithm. This is sometimes also referred to as the Orengo
/// stemmer.
///
/// * see: `RSLPStemmerBase`
///
/// Equivalent to `org.apache.lucene.analysis.pt.PortugueseStemmer`, which
/// extends `RSLPStemmerBase`. The term it stems is the `char[]` of Java,
/// UTF-16 units.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct PortugueseStemmer;

impl PortugueseStemmer {
    /// Equivalent to the implicit `PortugueseStemmer()`.
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
        len = steps.adverb.apply(s, len);
        len = steps.feminine.apply(s, len);
        len = steps.augmentative.apply(s, len);

        let mut oldlen = len;
        len = steps.noun.apply(s, len);

        if len == oldlen {
            /* suffix not removed */
            oldlen = len;

            len = steps.verb.apply(s, len);

            if len == oldlen {
                /* suffix not removed */
                len = steps.vowel.apply(s, len);
            }
        }

        // rslp accent removal
        for c in s.iter_mut().take(len) {
            match char::from_u32(u32::from(*c)) {
                Some('à' | 'á' | 'â' | 'ã' | 'ä' | 'å') => *c = 'a' as u16,
                Some('ç') => *c = 'c' as u16,
                Some('è' | 'é' | 'ê' | 'ë') => *c = 'e' as u16,
                Some('ì' | 'í' | 'î' | 'ï') => *c = 'i' as u16,
                Some('ñ') => *c = 'n' as u16,
                Some('ò' | 'ó' | 'ô' | 'õ' | 'ö') => *c = 'o' as u16,
                Some('ù' | 'ú' | 'û' | 'ü') => *c = 'u' as u16,
                Some('ý' | 'ÿ') => *c = 'y' as u16,
                _ => {}
            }
        }
        len
    }
}
