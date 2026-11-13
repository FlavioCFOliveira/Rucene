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

//! A `COMPOUNDRULE` pattern, ported from
//! `org.apache.lucene.analysis.hunspell.CompoundRule`.

use std::fmt;

use rucene::error::{LuceneError, Result};
use rucene::util::IntsRef;

use super::dictionary::{Dictionary, FlagParsingStrategy};
use crate::jdk::string::{index_of_unit, to_rust_string};

/// A compound rule: a sequence of flags, each optionally followed by `?` or
/// `*`, that the parts of a compound must carry in order.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.CompoundRule`.
///
/// **Divergence.** Java keeps the `Dictionary` the rule belongs to and asks
/// it for flags; a Rust value cannot point back at the dictionary that owns
/// it, so the matching methods take the dictionary.
#[derive(Debug, Clone)]
pub(crate) struct CompoundRule {
    data: Vec<u16>,
}

impl CompoundRule {
    /// Parses `rule` with `strategy`, the dictionary's flag parsing
    /// strategy.
    ///
    /// Equivalent to `CompoundRule(String, Dictionary)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Unmatched parentheses: " + rule)`, and what
    /// parsing the flags raises.
    pub(crate) fn new(rule: &[u16], strategy: &dyn FlagParsingStrategy) -> Result<Self> {
        let mut parsed_flags: Vec<u16> = Vec::new();
        let mut pos = 0;
        while pos < rule.len() {
            let Some(l_paren) = index_of_unit(rule, u16::from(b'('), pos) else {
                parsed_flags.extend(strategy.parse_flags(&rule[pos..])?);
                break;
            };
            parsed_flags.extend(strategy.parse_flags(&rule[pos..l_paren])?);
            let Some(r_paren) = index_of_unit(rule, u16::from(b')'), l_paren + 1) else {
                return Err(LuceneError::illegal_argument(format!(
                    "Unmatched parentheses: {}",
                    to_rust_string(rule)
                )));
            };
            parsed_flags.extend(strategy.parse_flags(&rule[l_paren + 1..r_paren])?);
            pos = r_paren + 1;
            if pos < rule.len() && (rule[pos] == u16::from(b'?') || rule[pos] == u16::from(b'*')) {
                parsed_flags.push(rule[pos]);
                pos += 1;
            }
        }
        Ok(Self { data: parsed_flags })
    }

    /// Equivalent to `mayMatch(List<IntsRef>)`.
    pub(crate) fn may_match(&self, dictionary: &Dictionary, words: &[IntsRef]) -> bool {
        self.match_(dictionary, words, 0, 0, false)
    }

    /// Equivalent to `fullyMatches(List<IntsRef>)`.
    pub(crate) fn fully_matches(&self, dictionary: &Dictionary, words: &[IntsRef]) -> bool {
        self.match_(dictionary, words, 0, 0, true)
    }

    /// Equivalent to `match(List<IntsRef>, int, int, boolean)`.
    fn match_(
        &self,
        dictionary: &Dictionary,
        words: &[IntsRef],
        pattern_index: usize,
        word_index: usize,
        fully: bool,
    ) -> bool {
        let data = &self.data;
        if pattern_index >= data.len() {
            return word_index >= words.len();
        }
        if word_index >= words.len() && !fully {
            return true;
        }
        let mut word_index = word_index;
        let flag = data[pattern_index];
        if pattern_index < data.len() - 1 && data[pattern_index + 1] == u16::from(b'*') {
            let start_wi = word_index as i64;
            while word_index < words.len() && dictionary.has_flag_forms(&words[word_index], flag) {
                word_index += 1;
            }
            let mut wi = word_index as i64;
            while wi >= start_wi {
                if self.match_(dictionary, words, pattern_index + 2, wi as usize, fully) {
                    return true;
                }
                wi -= 1;
            }
            return false;
        }
        let current_word_matches =
            word_index < words.len() && dictionary.has_flag_forms(&words[word_index], flag);
        if pattern_index < data.len() - 1 && data[pattern_index + 1] == u16::from(b'?') {
            if current_word_matches
                && self.match_(dictionary, words, pattern_index + 2, word_index + 1, fully)
            {
                return true;
            }
            return self.match_(dictionary, words, pattern_index + 2, word_index, fully);
        }
        current_word_matches
            && self.match_(dictionary, words, pattern_index + 1, word_index + 1, fully)
    }
}

impl fmt::Display for CompoundRule {
    /// Equivalent to `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&to_rust_string(&self.data))
    }
}
