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

//! The trigram scorer of suggestions, ported from
//! `org.apache.lucene.analysis.hunspell.TrigramAutomaton`.

use std::collections::HashMap;

use rucene::error::{LuceneError, Result};
use rucene::util::automaton::character_run_automaton::CharacterRunAutomaton;
use rucene::util::automaton::operations::{Operations, DEFAULT_DETERMINIZE_WORK_LIMIT};
use rucene::util::automaton::AutomatonBuilder;
use rucene::util::FixedBitSet;

use super::flyweight_entry::CharSequence;

const N: usize = 3;

/// An automaton allowing to achieve the same results as non-weighted
/// `GeneratingSuggester.ngramScore`, but faster (in `O(s2.length)` time).
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.TrigramAutomaton`.
#[derive(Debug)]
pub(crate) struct TrigramAutomaton {
    automaton: CharacterRunAutomaton,
    state2_score: Vec<i32>,
    counted_substrings: FixedBitSet,
    min_char: u16,
    max_char: u16,
}

impl TrigramAutomaton {
    /// Equivalent to `TrigramAutomaton(String)`.
    ///
    /// # Errors
    ///
    /// Java's `NoSuchElementException("No value present")` for an empty
    /// `s1`, and what determinizing the automaton raises.
    pub(crate) fn new(s1: &[u16]) -> Result<Self> {
        let mut substring_counts: HashMap<Vec<u16>, i32> = HashMap::new();

        let mut builder = AutomatonBuilder::with_capacity(s1.len() * N, s1.len() * N);
        let initial_state = builder.create_state();

        let no_value =
            || LuceneError::unchecked("java.util.NoSuchElementException", "No value present");
        let min_char = *s1.iter().min().ok_or_else(no_value)?;
        let max_char = *s1.iter().max().ok_or_else(no_value)?;
        for start in 0..s1.len() {
            let limit = s1.len().min(start + N);
            for end in start + 1..=limit {
                *substring_counts.entry(s1[start..end].to_vec()).or_insert(0) += 1;
            }

            let mut state = initial_state;
            for &c in &s1[start..limit] {
                let next = builder.create_state();
                builder.add_transition(state, next, i32::from(c - min_char));
                state = next;
            }
        }
        let automaton = CharacterRunAutomaton::new(Operations::determinize(
            &builder.finish(),
            DEFAULT_DETERMINIZE_WORK_LIMIT,
        )?)?;
        let mut state2_score = vec![0; automaton.size() as usize];
        for (key, count) in substring_counts {
            let state = run_automaton_on_string_chars(&automaton, &key, min_char);
            debug_assert_eq!(state2_score[state as usize], 0);
            state2_score[state as usize] = count;
        }
        let counted_substrings = FixedBitSet::new(state2_score.len());
        Ok(Self {
            automaton,
            state2_score,
            counted_substrings,
            min_char,
            max_char,
        })
    }

    /// Equivalent to `ngramScore(CharSequence)`.
    pub(crate) fn ngram_score<S: CharSequence + ?Sized>(&mut self, s2: &S) -> i32 {
        self.counted_substrings.clear_all();

        let mut score = 0;
        // states of running the automaton on substrings [i-1, i) and [i-2, i)
        let mut state1 = -1;
        let mut state2 = -1;

        let limit = s2.length();
        for i in 0..limit {
            let c = s2.char_at(i);
            if c < self.min_char || c > self.max_char {
                state1 = -1;
                state2 = -1;
                continue;
            }
            let c = i32::from(c - self.min_char);

            let state3 = if state2 <= 0 {
                0
            } else {
                self.automaton.step(state2, c)
            };
            if state3 > 0 {
                score += self.substring_score(state3);
            }

            state2 = if state1 <= 0 {
                0
            } else {
                self.automaton.step(state1, c)
            };
            if state2 > 0 {
                score += self.substring_score(state2);
            }

            state1 = self.automaton.step(0, c);
            if state1 > 0 {
                score += self.substring_score(state1);
            }
        }

        score
    }

    /// Equivalent to `substringScore(int, FixedBitSet)`.
    fn substring_score(&mut self, state: i32) -> i32 {
        if self.counted_substrings.get_and_set(state as usize) {
            return 0;
        }
        let score = self.state2_score[state as usize];
        debug_assert!(score > 0);
        score
    }
}

/// Equivalent to `runAutomatonOnStringChars(String)`.
fn run_automaton_on_string_chars(
    automaton: &CharacterRunAutomaton,
    s: &[u16],
    min_char: u16,
) -> i32 {
    let mut state = 0;
    for &c in s {
        state = automaton.step(state, i32::from(c - min_char));
    }
    state
}
