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

//! The suggester that modifies the misspelled word, ported from
//! `org.apache.lucene.analysis.hunspell.ModifyingSuggester`.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use rucene::error::Result;

use super::fragment_checker::FragmentChecker;
use super::hunspell::Hunspell;
use super::suggester::SuggestionSet;
use super::suggestion::Suggestion;
use super::word_case::WordCase;
use crate::jdk::character::{is_upper_case, to_upper_case_root, to_upper_case_unit};
use crate::jdk::string::{
    concat, contains_str, index_of_unit, region_matches, split_unit, trim, units,
};

const MAX_CHAR_DISTANCE: usize = 4;

/// A class that modifies the given misspelled word in various ways to get
/// correct suggestions.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.ModifyingSuggester`.
/// `result` is the suggestion set the `Suggester` shares with its timeout
/// callback; it is never locked across a call into the speller.
pub(crate) struct ModifyingSuggester<'a> {
    result: Arc<Mutex<SuggestionSet>>,
    misspelled: Vec<u16>,
    word_case: WordCase,
    fragment_checker: &'a dyn FragmentChecker,
    proceed_past_rep: bool,
    try_chars: Vec<u16>,
    speller: &'a Hunspell,
    tried: HashSet<Vec<u16>>,
}

/// The grade of the suggestions `tryRep` found.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.ModifyingSuggester$GradedSuggestions`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GradedSuggestions {
    /// Java constant `None`.
    None,
    /// Java constant `Normal`.
    Normal,
    /// Java constant `Best`.
    Best,
}

impl<'a> ModifyingSuggester<'a> {
    /// Equivalent to `ModifyingSuggester(Hunspell, LinkedHashSet<Suggestion>,
    /// String, WordCase, FragmentChecker, boolean)`.
    pub(crate) fn new(
        speller: &'a Hunspell,
        result: Arc<Mutex<SuggestionSet>>,
        misspelled: Vec<u16>,
        word_case: WordCase,
        checker: &'a dyn FragmentChecker,
        proceed_past_rep: bool,
    ) -> Self {
        Self {
            try_chars: speller.dictionary.try_chars.clone(),
            speller,
            result,
            misspelled,
            word_case,
            fragment_checker: checker,
            proceed_past_rep,
            tried: HashSet::new(),
        }
    }

    /// Runs `body` over the result set.
    fn with_result<T>(&self, body: impl FnOnce(&mut SuggestionSet) -> T) -> T {
        body(
            &mut self
                .result
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    /// Whether any of the added suggestions are considered "good".
    ///
    /// Equivalent to `suggest()`.
    ///
    /// # Errors
    ///
    /// What `checkCanceled` raises.
    pub(crate) fn suggest(&mut self) -> Result<bool> {
        let misspelled = self.misspelled.clone();
        let low = if self.word_case != WordCase::Lower {
            self.speller.dictionary.to_lower_case(&misspelled)
        } else {
            misspelled.clone()
        };
        if self.word_case == WordCase::Upper || self.word_case == WordCase::Mixed {
            self.try_suggestion(&low)?;
        }

        let mut has_good_suggestions = self.try_variations_of(&misspelled)?;

        if self.word_case == WordCase::Title {
            has_good_suggestions |= self.try_variations_of(&low)?;
        } else if self.word_case == WordCase::Upper {
            has_good_suggestions |= self.try_variations_of(&low)?;
            let title = self.speller.dictionary.to_title_case(&misspelled);
            has_good_suggestions |= self.try_variations_of(&title)?;
        } else if self.word_case == WordCase::Mixed {
            if let Some(dot) = index_of_unit(&misspelled, u16::from(b'.'), 0) {
                if dot > 0 && dot < misspelled.len() - 1 {
                    let after_dot = &misspelled[dot + 1..];
                    if WordCase::case_of(after_dot) == WordCase::Title {
                        let candidate = concat(&[&misspelled[..dot + 1], &units(" "), after_dot]);
                        let suggestion = self.create_suggestion(candidate)?;
                        self.with_result(|result| result.add(suggestion));
                    }
                }
            }

            let first = misspelled[0];
            let capitalized = is_upper_case(i32::from(first));
            if capitalized {
                let mut variant = vec![self.speller.dictionary.case_fold(first)];
                variant.extend_from_slice(&misspelled[1..]);
                has_good_suggestions |= self.try_variations_of(&variant)?;
            }

            has_good_suggestions |= self.try_variations_of(&low)?;

            if capitalized {
                let title = self.speller.dictionary.to_title_case(&low);
                has_good_suggestions |= self.try_variations_of(&title)?;
            }

            let candidates = self.with_result(|result| result.to_vec());
            let mut reordered: Vec<Suggestion> = Vec::new();
            for candidate in candidates {
                match self.capitalize_after_space(&candidate.raw)? {
                    None => reordered.push(candidate),
                    Some(changed) => reordered.insert(0, changed),
                }
            }

            self.with_result(|result| {
                result.clear();
                result.add_all(reordered);
            });
        }
        Ok(has_good_suggestions)
    }

    /// Equivalent to `createSuggestion(String)`.
    fn create_suggestion(&self, candidate: Vec<u16>) -> Result<Suggestion> {
        Suggestion::new(candidate, &self.misspelled, self.word_case, self.speller)
    }

    /// aNew -> "a New" (instead of "a new").
    ///
    /// Equivalent to `capitalizeAfterSpace(String)`.
    fn capitalize_after_space(&self, candidate: &[u16]) -> Result<Option<Suggestion>> {
        let space = index_of_unit(candidate, u16::from(b' '), 0);
        let Some(space) = space.filter(|&space| space > 0) else {
            return Ok(None);
        };
        let tail = candidate.len() as i64 - space as i64 - 1;
        let misspelled = &self.misspelled;
        if !region_matches(
            misspelled,
            misspelled.len() as i64 - tail,
            candidate,
            space as i64 + 1,
            tail,
        ) {
            let mut changed = candidate[..space + 1].to_vec();
            changed.push(to_upper_case_unit(candidate[space + 1]));
            changed.extend_from_slice(&candidate[space + 2..]);
            return Ok(Some(self.create_suggestion(changed)?));
        }
        Ok(None)
    }

    /// Equivalent to `tryVariationsOf(String)`.
    fn try_variations_of(&mut self, word: &[u16]) -> Result<bool> {
        let mut has_good_suggestions = self.try_suggestion(&to_upper_case_root(word))?;
        let rep_result = self.try_rep(word)?;
        if rep_result == GradedSuggestions::Best && !self.proceed_past_rep {
            return Ok(true);
        }

        has_good_suggestions |= rep_result != GradedSuggestions::None;

        if !self.speller.dictionary.map_table.is_empty() {
            self.enumerate_map_replacements(word, &[], 0)?;
        }

        self.try_swapping_chars(word)?;
        self.try_long_swap(word)?;
        self.try_neighbor_keys(word)?;
        self.try_removing_char(word)?;
        self.try_adding_char(word)?;
        self.try_moving_char(word)?;
        self.try_replacing_char(word)?;
        self.try_two_duplicate_chars(word)?;

        let good_split = self.check_dictionary_for_split_suggestions(word)?;
        if !good_split.is_empty() {
            self.with_result(|result| {
                let copy = result.to_vec();
                result.clear();
                result.add_all(good_split);
                if has_good_suggestions {
                    result.add_all(copy);
                }
            });
            has_good_suggestions = true;
        }

        if !has_good_suggestions && self.speller.dictionary.enable_split_suggestions {
            self.try_splitting(word)?;
        }
        Ok(has_good_suggestions)
    }

    /// Equivalent to `tryRep(String)`.
    fn try_rep(&mut self, word: &[u16]) -> Result<GradedSuggestions> {
        let mut has_best = false;
        let before = self.with_result(|result| result.len());
        let dictionary = Arc::clone(&self.speller.dictionary);
        for entry in &dictionary.rep_table {
            for candidate in entry.substitute(word) {
                let candidate = trim(&candidate).to_vec();
                if self.try_suggestion(&candidate)? {
                    has_best = true;
                    continue;
                }
                if contains_str(&candidate, " ") {
                    let mut all_simple = true;
                    for part in split_unit(&candidate, u16::from(b' ')) {
                        if !self.check_simple_word(&part)? {
                            all_simple = false;
                            break;
                        }
                    }
                    if all_simple {
                        let suggestion = self.create_suggestion(candidate)?;
                        self.with_result(|result| result.add(suggestion));
                    }
                }
            }
        }
        if has_best {
            return Ok(GradedSuggestions::Best);
        }
        let after = self.with_result(|result| result.len());
        Ok(if after > before {
            GradedSuggestions::Normal
        } else {
            GradedSuggestions::None
        })
    }

    /// Equivalent to `enumerateMapReplacements(String, String, int)`.
    fn enumerate_map_replacements(
        &mut self,
        word: &[u16],
        accumulated: &[u16],
        offset: usize,
    ) -> Result<()> {
        if offset == word.len() {
            self.try_suggestion(accumulated)?;
            return Ok(());
        }

        let length = accumulated.len();
        let dictionary = Arc::clone(&self.speller.dictionary);
        for entries in &dictionary.map_table {
            for entry in entries {
                if region_matches(word, offset as i64, entry, 0, entry.len() as i64) {
                    for replacement in entries {
                        if entry != replacement {
                            let next = concat(&[accumulated, replacement]);
                            let end = length + replacement.len();
                            if !self
                                .fragment_checker
                                .has_impossible_fragment_around(&next, length, end)
                            {
                                self.enumerate_map_replacements(word, &next, offset + entry.len())?;
                            }
                        }
                    }
                }
            }
        }

        let next = concat(&[accumulated, &[word[offset]]]);
        if !self
            .fragment_checker
            .has_impossible_fragment_around(&next, length, length + 1)
        {
            self.enumerate_map_replacements(word, &next, offset + 1)?;
        }
        Ok(())
    }

    /// Equivalent to `checkSimpleWord(String)`.
    fn check_simple_word(&self, part: &[u16]) -> Result<bool> {
        Ok(self.speller.check_simple_word(part, part.len(), None)? == Some(true))
    }

    /// Equivalent to `trySwappingChars(String)`.
    fn try_swapping_chars(&mut self, word: &[u16]) -> Result<()> {
        let length = word.len();
        for i in 0..length.saturating_sub(1) {
            let c1 = word[i];
            let c2 = word[i + 1];
            self.try_suggestion(&concat(&[&word[..i], &[c2, c1], &word[i + 2..]]))?;
        }
        if length == 4 || length == 5 {
            self.try_double_swap_for_short_words(word, length)?;
        }
        Ok(())
    }

    /// ahev -> have, owudl -> would.
    ///
    /// Equivalent to `tryDoubleSwapForShortWords(String, int)`.
    fn try_double_swap_for_short_words(&mut self, word: &[u16], length: usize) -> Result<()> {
        let mut candidate = word.to_vec();
        candidate[0] = word[1];
        candidate[1] = word[0];
        candidate[length - 1] = word[length - 2];
        candidate[length - 2] = word[length - 1];
        self.try_suggestion(&candidate)?;

        if candidate.len() == 5 {
            candidate[0] = word[0];
            candidate[1] = word[2];
            candidate[2] = word[1];
            self.try_suggestion(&candidate)?;
        }
        Ok(())
    }

    /// Equivalent to `tryNeighborKeys(String)`.
    fn try_neighbor_keys(&mut self, word: &[u16]) -> Result<()> {
        let dictionary = Arc::clone(&self.speller.dictionary);
        for i in 0..word.len() {
            let c = word[i];
            let up = to_upper_case_unit(c);
            if up != c {
                self.try_suggestion(&concat(&[&word[..i], &[up], &word[i + 1..]]))?;
            }

            // check neighbor characters in keyboard string
            for group in &dictionary.neighbor_key_groups {
                if group.contains(&c) {
                    for &g in group {
                        if g != c {
                            self.try_modified_suggestions(
                                i,
                                concat(&[&word[..i], &[g], &word[i + 1..]]),
                            )?;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Equivalent to `tryModifiedSuggestions(int, String)`.
    fn try_modified_suggestions(&mut self, mod_offset: usize, candidate: Vec<u16>) -> Result<()> {
        if !self.fragment_checker.has_impossible_fragment_around(
            &candidate,
            mod_offset,
            mod_offset + 1,
        ) {
            self.try_suggestion(&candidate)?;
        }
        Ok(())
    }

    /// Equivalent to `tryLongSwap(String)`.
    fn try_long_swap(&mut self, word: &[u16]) -> Result<()> {
        for i in 0..word.len() {
            let mut j = i + 2;
            while j < word.len() && j <= i + MAX_CHAR_DISTANCE {
                let c1 = word[i];
                let c2 = word[j];
                let prefix = &word[..i];
                let suffix = &word[j + 1..];
                self.try_suggestion(&concat(&[prefix, &[c2], &word[i + 1..j], &[c1], suffix]))?;
                j += 1;
            }
        }
        Ok(())
    }

    /// Equivalent to `tryRemovingChar(String)`.
    fn try_removing_char(&mut self, word: &[u16]) -> Result<()> {
        if word.len() == 1 {
            return Ok(());
        }
        for i in 0..word.len() {
            self.try_suggestion(&concat(&[&word[..i], &word[i + 1..]]))?;
        }
        Ok(())
    }

    /// Equivalent to `tryAddingChar(String)`.
    fn try_adding_char(&mut self, word: &[u16]) -> Result<()> {
        let try_chars = self.try_chars.clone();
        for i in 0..=word.len() {
            let prefix = &word[..i];
            let suffix = &word[i..];
            for &to_insert in &try_chars {
                self.try_modified_suggestions(
                    prefix.len(),
                    concat(&[prefix, &[to_insert], suffix]),
                )?;
            }
        }
        Ok(())
    }

    /// Equivalent to `tryMovingChar(String)`.
    fn try_moving_char(&mut self, word: &[u16]) -> Result<()> {
        for i in 0..word.len() {
            let prefix = &word[..i];
            let mut j = i + 2;
            while j < word.len() && j <= i + MAX_CHAR_DISTANCE {
                self.try_suggestion(&concat(&[prefix, &word[i + 1..j], &[word[i]], &word[j..]]))?;
                self.try_suggestion(&concat(&[prefix, &[word[j]], &word[i..j], &word[j + 1..]]))?;
                j += 1;
            }
            if i + 1 < word.len() {
                self.try_suggestion(&concat(&[prefix, &word[i + 1..], &[word[i]]]))?;
            }
        }
        Ok(())
    }

    /// Equivalent to `tryReplacingChar(String)`.
    fn try_replacing_char(&mut self, word: &[u16]) -> Result<()> {
        let try_chars = self.try_chars.clone();
        for i in 0..word.len() {
            let prefix = &word[..i];
            let suffix = &word[i + 1..];
            for &to_insert in &try_chars {
                if to_insert != word[i] {
                    self.try_modified_suggestions(
                        prefix.len(),
                        concat(&[prefix, &[to_insert], suffix]),
                    )?;
                }
            }
        }
        Ok(())
    }

    /// Perhaps we doubled two characters (for example vacation ->
    /// vacacation).
    ///
    /// Equivalent to `tryTwoDuplicateChars(String)`.
    fn try_two_duplicate_chars(&mut self, word: &[u16]) -> Result<()> {
        let mut dup_len = 0;
        for i in 2..word.len() {
            if word[i] == word[i - 2] {
                dup_len += 1;
                if dup_len == 3 || dup_len == 2 && i >= 4 {
                    self.try_suggestion(&concat(&[&word[..i - 1], &word[i + 1..]]))?;
                    dup_len = 0;
                }
            } else {
                dup_len = 0;
            }
        }
        Ok(())
    }

    /// Equivalent to `checkDictionaryForSplitSuggestions(String)`.
    fn check_dictionary_for_split_suggestions(&self, word: &[u16]) -> Result<Vec<Suggestion>> {
        let mut result = Vec::new();
        for i in 1..word.len().saturating_sub(1) {
            let w1 = &word[..i];
            let w2 = &word[i..];
            let spaced = concat(&[w1, &units(" "), w2]);
            if self.speller.check_word(&spaced)? {
                result.push(self.create_suggestion(spaced)?);
            }
            if self.should_split_by_dash() {
                let dashed = concat(&[w1, &units("-"), w2]);
                if self.speller.check_word(&dashed)? {
                    result.push(self.create_suggestion(dashed)?);
                }
            }
        }
        Ok(result)
    }

    /// Equivalent to `trySplitting(String)`.
    fn try_splitting(&mut self, word: &[u16]) -> Result<()> {
        for i in 1..word.len() {
            let w1 = &word[..i];
            let w2 = &word[i..];
            if self.check_simple_word(w1)? && self.check_simple_word(w2)? {
                let suggestion = self.create_suggestion(concat(&[w1, &units(" "), w2]))?;
                self.with_result(|result| result.add(suggestion));
                if w1.len() > 1 && w2.len() > 1 && self.should_split_by_dash() {
                    let suggestion = self.create_suggestion(concat(&[w1, &units("-"), w2]))?;
                    self.with_result(|result| result.add(suggestion));
                }
            }
        }
        Ok(())
    }

    /// Equivalent to `shouldSplitByDash()`.
    fn should_split_by_dash(&self) -> bool {
        let try_chars = &self.speller.dictionary.try_chars;
        contains_str(try_chars, "-") || contains_str(try_chars, "a")
    }

    /// Equivalent to `trySuggestion(String)`.
    fn try_suggestion(&mut self, candidate: &[u16]) -> Result<bool> {
        if !self.tried.insert(candidate.to_vec()) {
            return Ok(false);
        }
        if !self.speller.check_word(candidate)? {
            return Ok(false);
        }
        let suggestion = self.create_suggestion(candidate.to_vec())?;
        Ok(self.with_result(|result| result.add(suggestion)))
    }
}
