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
//! Normalizes the use of the interchangeable Scandinavian characters, ported
//! from `org.apache.lucene.analysis.miscellaneous.ScandinavianNormalizer`.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use crate::analysis::util::stemmer_util::StemmerUtil;

/// This Normalizer does the heavy lifting for a set of Scandinavian
/// normalization filters, normalizing use of the interchangeable
/// Scandinavian characters æÆäÄöÖøØ and folded variants (aa, ao, ae, oe and
/// oo) by transforming them to åÅæÆøØ.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.ScandinavianNormalizer`
/// (since 9.0). Java's `Set<Foldings>` is a `BTreeSet` here.
#[derive(Clone, Debug)]
pub struct ScandinavianNormalizer {
    foldings: BTreeSet<Foldings>,
}

/// List of possible foldings that can be used when configuring the filter.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.ScandinavianNormalizer$Foldings`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Foldings {
    /// `AA`.
    AA,
    /// `AO`.
    AO,
    /// `AE`.
    AE,
    /// `OE`.
    OE,
    /// `OO`.
    OO,
}

impl Foldings {
    /// `Foldings.values()`, in declaration order.
    pub const fn values() -> [Foldings; 5] {
        [
            Foldings::AA,
            Foldings::AO,
            Foldings::AE,
            Foldings::OE,
            Foldings::OO,
        ]
    }
}

/// Equivalent to `ScandinavianNormalizer.ALL_FOLDINGS`,
/// `Collections.unmodifiableSet(EnumSet.allOf(Foldings.class))`.
pub static ALL_FOLDINGS: LazyLock<BTreeSet<Foldings>> =
    LazyLock::new(|| Foldings::values().into_iter().collect());

const AA: char = '\u{00C5}'; // Å
const AA_LOWER: char = '\u{00E5}'; // å
const AE: char = '\u{00C6}'; // Æ
const AE_LOWER: char = '\u{00E6}'; // æ
const AE_SE: char = '\u{00C4}'; // Ä
const AE_SE_LOWER: char = '\u{00E4}'; // ä
const OE: char = '\u{00D8}'; // Ø
const OE_LOWER: char = '\u{00F8}'; // ø
const OE_SE: char = '\u{00D6}'; // Ö
const OE_SE_LOWER: char = '\u{00F6}'; // ö

impl ScandinavianNormalizer {
    /// Create the instance, while choosing which foldings to apply. This may
    /// differ between Norwegian, Danish and Swedish.
    ///
    /// Equivalent to `ScandinavianNormalizer(Set<Foldings>)`.
    pub fn new(foldings: BTreeSet<Foldings>) -> Self {
        Self { foldings }
    }

    /// Takes the original buffer and length as input. Modifies the buffer in
    /// place and returns new length.
    ///
    /// Equivalent to `ScandinavianNormalizer.processToken(char[], int)`; the
    /// buffer holds `char`s (every character compared is in the BMP, so the
    /// result is Java's).
    pub fn process_token(&self, buffer: &mut [char], length: usize) -> usize {
        let mut length = length;
        let has = |folding: Foldings| self.foldings.contains(&folding);
        let mut i = 0;
        while i < length {
            if buffer[i] == AE_SE_LOWER {
                buffer[i] = AE_LOWER;
            } else if buffer[i] == AE_SE {
                buffer[i] = AE;
            } else if buffer[i] == OE_SE_LOWER {
                buffer[i] = OE_LOWER;
            } else if buffer[i] == OE_SE {
                buffer[i] = OE;
            } else if length - 1 > i {
                let next = buffer[i + 1];
                if buffer[i] == 'a'
                    && (has(Foldings::AA) && (next == 'a' || next == 'A')
                        || has(Foldings::AO) && (next == 'o' || next == 'O'))
                {
                    length = StemmerUtil::delete(buffer, i + 1, length);
                    buffer[i] = AA_LOWER;
                } else if buffer[i] == 'A'
                    && (has(Foldings::AA) && (next == 'a' || next == 'A')
                        || has(Foldings::AO) && (next == 'o' || next == 'O'))
                {
                    length = StemmerUtil::delete(buffer, i + 1, length);
                    buffer[i] = AA;
                } else if buffer[i] == 'a' && has(Foldings::AE) && (next == 'e' || next == 'E') {
                    length = StemmerUtil::delete(buffer, i + 1, length);
                    buffer[i] = AE_LOWER;
                } else if buffer[i] == 'A' && has(Foldings::AE) && (next == 'e' || next == 'E') {
                    length = StemmerUtil::delete(buffer, i + 1, length);
                    buffer[i] = AE;
                } else if buffer[i] == 'o'
                    && (has(Foldings::OE) && (next == 'e' || next == 'E')
                        || has(Foldings::OO) && (next == 'o' || next == 'O'))
                {
                    length = StemmerUtil::delete(buffer, i + 1, length);
                    buffer[i] = OE_LOWER;
                } else if buffer[i] == 'O'
                    && (has(Foldings::OE) && (next == 'e' || next == 'E')
                        || has(Foldings::OO) && (next == 'o' || next == 'O'))
                {
                    length = StemmerUtil::delete(buffer, i + 1, length);
                    buffer[i] = OE;
                }
            }
            i += 1;
        }
        length
    }
}
