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

//! The case of a word, ported from
//! `org.apache.lucene.analysis.hunspell.WordCase`.

use crate::jdk::character::{is_lower_case, is_upper_case, to_upper_case_unit};

/// The case of a word.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.WordCase`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum WordCase {
    /// Java constant `UPPER`: e.g. WORD.
    Upper,
    /// Java constant `TITLE`: e.g. Word.
    Title,
    /// Java constant `LOWER`: e.g. word.
    Lower,
    /// Java constant `MIXED`: e.g. WoRd or wOrd.
    Mixed,
    /// Java constant `NEUTRAL`: e.g "-" or "/" or "42".
    Neutral,
}

impl WordCase {
    /// Equivalent to `ordinal()`.
    pub(crate) const fn ordinal(self) -> i16 {
        match self {
            Self::Upper => 0,
            Self::Title => 1,
            Self::Lower => 2,
            Self::Mixed => 3,
            Self::Neutral => 4,
        }
    }

    /// The case of the first `length` units of `word`.
    ///
    /// Equivalent to `caseOf(char[], int)` and `caseOf(CharSequence, int)`.
    pub(crate) fn case_of_length(word: &[u16], length: usize) -> WordCase {
        let start_case = char_case(word[0]);
        let mut seen_upper = false;
        let mut seen_lower = false;
        for &c in word.iter().take(length).skip(1) {
            let cc = char_case(c);
            seen_upper = seen_upper || cc == CharCase::Upper;
            seen_lower = seen_lower || cc == CharCase::Lower;
            if seen_upper && seen_lower {
                break;
            }
        }
        get(start_case, seen_upper, seen_lower)
    }

    /// Equivalent to `caseOf(CharSequence)`.
    pub(crate) fn case_of(word: &[u16]) -> WordCase {
        Self::case_of_length(word, word.len())
    }
}

/// Equivalent to `WordCase.get(CharCase, boolean, boolean)`.
fn get(start_case: CharCase, seen_upper: bool, seen_lower: bool) -> WordCase {
    if seen_lower && seen_upper {
        return WordCase::Mixed;
    }
    match start_case {
        CharCase::Lower => {
            if seen_upper {
                WordCase::Mixed
            } else {
                WordCase::Lower
            }
        }
        CharCase::Upper => {
            if !seen_lower {
                WordCase::Upper
            } else {
                WordCase::Title
            }
        }
        CharCase::Neutral => {
            if seen_lower {
                WordCase::Lower
            } else if seen_upper {
                WordCase::Upper
            } else {
                WordCase::Neutral
            }
        }
    }
}

/// Equivalent to `WordCase.charCase(char)`.
fn char_case(c: u16) -> CharCase {
    if is_upper_case(i32::from(c)) {
        return CharCase::Upper;
    }
    if is_lower_case(i32::from(c)) && to_upper_case_unit(c) != c {
        return CharCase::Lower;
    }
    CharCase::Neutral
}

/// The case of one character.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.WordCase$CharCase`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CharCase {
    /// Java constant `UPPER`.
    Upper,
    /// Java constant `LOWER`.
    Lower,
    /// Java constant `NEUTRAL`.
    Neutral,
}
