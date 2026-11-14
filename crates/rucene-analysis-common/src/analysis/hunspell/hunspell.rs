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

//! The Hunspell spell checker, ported from
//! `org.apache.lucene.analysis.hunspell.Hunspell`.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use rucene::error::Result;
use rucene::util::{CharsRef, IntsRef};

use super::affixed_word::{Affix, AffixedWord};
use super::check_compound_pattern::CheckCompoundPattern;
use super::dictionary::{Dictionary, FLAG_UNSET, HIDDEN_FLAG};
use super::entry_suggestion::EntrySuggestion;
use super::root::Root;
use super::stemmer::{CaseVariationProcessor, RootProcessor, Stemmer};
use super::suggester::Suggester;
use super::suggestion_timeout_exception::SuggestionTimeoutException;
use super::timeout_policy::TimeoutPolicy;
use super::word_case::WordCase;
use super::word_context::WordContext;
use super::word_form_generator::WordFormGenerator;
use crate::jdk::character::is_upper_case;
use crate::jdk::string::{index_of, to_rust_string, units};

/// Java's `Runnable checkCanceled`: an object that's periodically called,
/// allowing to interrupt spell-checking or suggestion generation by throwing
/// an exception (here, returning an error).
pub type CheckCanceled = Arc<dyn Fn() -> Result<()> + Send + Sync>;

/// The `checkCanceled` that never cancels, `() -> {}`.
pub fn no_check_canceled() -> CheckCanceled {
    Arc::new(|| Ok(()))
}

/// The overrides of the anonymous `Hunspell` subclass `Suggester.suggest`
/// creates.
#[derive(Debug, Default)]
pub(crate) struct SuggestionSpeller {
    // Cache for expensive "findStem" requests issued when trying to split a compound word.
    // The suggestion algorithm issues many of them, often with the same text.
    // The cache can be large, but will be GC-ed after the "suggest" call.
    compound_cache: Mutex<HashMap<Vec<u16>, Option<Root>>>,
}

/// A spell checker based on Hunspell dictionaries. This class can be used in
/// place of native Hunspell for many languages for spell-checking and
/// suggesting purposes. Note that not all languages are supported yet. For
/// example:
///
/// * Hungarian (as it doesn't only rely on dictionaries, but has some logic
///   directly in the source code
/// * Languages with Unicode characters outside of the Basic Multilingual
///   Plane
/// * PHONE affix file option for suggestions
///
/// The objects of this class are thread-safe.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.Hunspell`. Its one
/// subclass in Lucene, the anonymous one of `Suggester.suggest`, is this type
/// with the suggestion speller overrides.
pub struct Hunspell {
    /// Java's member is package-private, reached by the same package's tests;
    /// Rust's integration tests are another crate, so the port makes it public
    /// (hidden from the documentation).
    #[doc(hidden)]
    pub dictionary: Arc<Dictionary>,
    pub(crate) stemmer: Stemmer,
    policy: TimeoutPolicy,
    pub(crate) check_canceled: CheckCanceled,
    suggestion_speller: Option<SuggestionSpeller>,
}

impl std::fmt::Debug for Hunspell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hunspell")
            .field("policy", &self.policy)
            .field("suggestion_speller", &self.suggestion_speller.is_some())
            .finish_non_exhaustive()
    }
}

/// `Hunspell.SUGGEST_TIME_LIMIT`, in milliseconds.
///
/// Java's member is package-private, reached by the same package's tests;
/// Rust's integration tests are another crate, so the port makes it public
/// (hidden from the documentation).
#[doc(hidden)]
pub const SUGGEST_TIME_LIMIT: i64 = 250;

/// The root processor of `findStem`.
struct FindStemProcessor<'a> {
    hunspell: &'a Hunspell,
    to_check: Option<WordCase>,
    result: Option<Root>,
}

impl RootProcessor for FindStemProcessor<'_> {
    fn process_root(
        &mut self,
        stem: &[u16],
        form_id: i32,
        _morph_data_id: i32,
        _outer_prefix: i32,
        _inner_prefix: i32,
        _outer_suffix: i32,
        _inner_suffix: i32,
    ) -> Result<bool> {
        let dictionary = &self.hunspell.dictionary;
        if !self.hunspell.accept_case(self.to_check, form_id, stem) {
            return Ok(dictionary.has_flag(form_id, HIDDEN_FLAG));
        }
        if self.hunspell.accepts_stem(form_id) {
            self.result = Some(Root::new(stem.to_vec(), form_id));
        }
        Ok(false)
    }
}

/// The case variation processor of `spellClean`: stops at a correct variant.
struct CheckVariation<'a> {
    hunspell: &'a Hunspell,
}

impl CaseVariationProcessor for CheckVariation<'_> {
    fn process(
        &mut self,
        variant: &[u16],
        var_length: usize,
        original_case: Option<WordCase>,
    ) -> Result<bool> {
        Ok(!self
            .hunspell
            .check_word_with_case(variant, var_length, original_case)?)
    }
}

/// The root processor of `checkLastCompoundPart`: stops at a root that
/// completes a compound rule.
struct StopOnMatching<'a> {
    dictionary: &'a Dictionary,
    words: &'a mut Vec<IntsRef>,
}

impl RootProcessor for StopOnMatching<'_> {
    fn process_root(
        &mut self,
        _stem: &[u16],
        form_id: i32,
        _morph_data_id: i32,
        _outer_prefix: i32,
        _inner_prefix: i32,
        _outer_suffix: i32,
        _inner_suffix: i32,
    ) -> Result<bool> {
        let last = self.words.len() - 1;
        self.words[last].ints[0] = form_id;
        let rules = self
            .dictionary
            .compound_rules
            .as_ref()
            .expect("INVARIANT: compound rules are checked only when present");
        for r in rules {
            if r.fully_matches(self.dictionary, self.words) {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

/// The root processor of `analyzeSimpleWord`.
struct AnalyzeProcessor<'a> {
    dictionary: &'a Dictionary,
    word: &'a [u16],
    result: Vec<AffixedWord>,
}

impl RootProcessor for AnalyzeProcessor<'_> {
    fn process_root(
        &mut self,
        stem: &[u16],
        form_id: i32,
        morph_data_id: i32,
        outer_prefix: i32,
        inner_prefix: i32,
        outer_suffix: i32,
        inner_suffix: i32,
    ) -> Result<bool> {
        let mut prefixes = Vec::new();
        let mut suffixes = Vec::new();
        if outer_prefix >= 0 {
            prefixes.push(Affix::new(self.dictionary, outer_prefix));
        }
        if inner_prefix >= 0 {
            prefixes.push(Affix::new(self.dictionary, inner_prefix));
        }
        if outer_suffix >= 0 {
            suffixes.push(Affix::new(self.dictionary, outer_suffix));
        }
        if inner_suffix >= 0 {
            suffixes.push(Affix::new(self.dictionary, inner_suffix));
        }

        let entry = self.dictionary.dict_entry(stem, form_id, morph_data_id);
        self.result.push(AffixedWord::new(
            self.word.to_vec(),
            Arc::new(entry),
            prefixes,
            suffixes,
        ));
        Ok(true)
    }
}

impl Hunspell {
    /// A spell checker with the [`TimeoutPolicy::ReturnPartialResult`]
    /// policy and a `checkCanceled` that never cancels.
    ///
    /// Equivalent to `Hunspell(Dictionary)`.
    pub fn new(dictionary: Arc<Dictionary>) -> Self {
        Self::with_policy(
            dictionary,
            TimeoutPolicy::ReturnPartialResult,
            no_check_canceled(),
        )
    }

    /// `policy` is a strategy determining what to do when API calls take too
    /// much time; `check_canceled` an object that's periodically called,
    /// allowing to interrupt spell-checking or suggestion generation by
    /// throwing an exception.
    ///
    /// Equivalent to `Hunspell(Dictionary, TimeoutPolicy, Runnable)`.
    pub fn with_policy(
        dictionary: Arc<Dictionary>,
        policy: TimeoutPolicy,
        check_canceled: CheckCanceled,
    ) -> Self {
        let stemmer = Stemmer::new(Arc::clone(&dictionary));
        Self {
            dictionary,
            stemmer,
            policy,
            check_canceled,
            suggestion_speller: None,
        }
    }

    /// The anonymous subclass of `Suggester.suggest`: `NO_TIMEOUT`, stems
    /// with NOSUGGEST or SUBSTANDARD not accepted, and the compound-begin
    /// stems of `findStem` cached.
    pub(crate) fn for_suggestions(
        dictionary: Arc<Dictionary>,
        check_canceled: CheckCanceled,
    ) -> Self {
        let mut speller = Self::with_policy(dictionary, TimeoutPolicy::NoTimeout, check_canceled);
        speller.suggestion_speller = Some(SuggestionSpeller::default());
        speller
    }

    /// Runs `checkCanceled`.
    pub(crate) fn check_canceled(&self) -> Result<()> {
        (self.check_canceled)()
    }

    /// Whether the given word's spelling is considered correct according to
    /// Hunspell rules.
    ///
    /// Equivalent to `spell(String)`.
    ///
    /// # Errors
    ///
    /// What `checkCanceled` raises, and what reading the dictionary raises.
    pub fn spell(&self, word: &str) -> Result<bool> {
        self.spell_units(&units(word))
    }

    /// [`spell`](Self::spell) of a word given as UTF-16 units.
    pub(crate) fn spell_units(&self, word: &[u16]) -> Result<bool> {
        self.check_canceled()?;
        if word.is_empty() {
            return Ok(true);
        }

        let mut word = word.to_vec();
        if self.dictionary.needs_input_cleaning(&word) {
            let mut cleaned = Vec::new();
            self.dictionary.clean_input(&word, &mut cleaned)?;
            word = cleaned;
        }

        if word.ends_with(&[u16::from(b'.')]) {
            return self.spell_with_trailing_dots(&word);
        }

        self.spell_clean(&word)
    }

    /// Equivalent to `spellClean(String)`.
    fn spell_clean(&self, word: &[u16]) -> Result<bool> {
        if is_number(word) {
            return Ok(true);
        }

        if let Some(simple_result) = self.check_simple_word(word, word.len(), None)? {
            return Ok(simple_result);
        }

        if self.check_compounds_word(word, word.len(), None)? {
            return Ok(true);
        }

        let wc = self.stemmer.case_of(word, word.len());
        if wc == WordCase::Upper || wc == WordCase::Title {
            let mut variation_processor = CheckVariation { hunspell: self };
            if !self
                .stemmer
                .vary_case(word, word.len(), wc, &mut variation_processor)?
            {
                return Ok(true);
            }
        }

        if self.dictionary.breaks.is_not_empty() && !self.has_too_many_break_occurrences(word) {
            return self.try_breaks(word);
        }

        Ok(false)
    }

    /// Equivalent to `spellWithTrailingDots(String)`.
    fn spell_with_trailing_dots(&self, word: &[u16]) -> Result<bool> {
        let mut length = word.len() - 1;
        while length > 0 && word[length - 1] == u16::from(b'.') {
            length -= 1;
        }
        Ok(self.spell_clean(&word[..length])? || self.spell_clean(&word[..length + 1])?)
    }

    /// Equivalent to `checkWord(String)`.
    pub(crate) fn check_word(&self, word: &[u16]) -> Result<bool> {
        self.check_word_with_case(word, word.len(), None)
    }

    /// Whether the word is a correct simple word (`Some(true)`), a forbidden
    /// one (`Some(false)`), or not a simple word of the dictionary (`None`).
    ///
    /// Equivalent to `checkSimpleWord(char[], int, WordCase)`.
    pub(crate) fn check_simple_word(
        &self,
        word_chars: &[u16],
        length: usize,
        original_case: Option<WordCase>,
    ) -> Result<Option<bool>> {
        let entry = self.find_stem(
            word_chars,
            0,
            length,
            original_case,
            WordContext::SimpleWord,
        )?;
        Ok(entry.map(|entry| {
            !self
                .dictionary
                .has_flag(entry.entry_id, self.dictionary.forbiddenword)
        }))
    }

    /// Equivalent to `checkWord(char[], int, WordCase)`.
    fn check_word_with_case(
        &self,
        word_chars: &[u16],
        length: usize,
        original_case: Option<WordCase>,
    ) -> Result<bool> {
        if let Some(simple_result) = self.check_simple_word(word_chars, length, original_case)? {
            return Ok(simple_result);
        }
        self.check_compounds_word(word_chars, length, original_case)
    }

    /// Equivalent to `checkCompounds(char[], int, WordCase)`.
    fn check_compounds_word(
        &self,
        word_chars: &[u16],
        length: usize,
        original_case: Option<WordCase>,
    ) -> Result<bool> {
        if self.dictionary.compound_rules.is_some()
            && self.check_compound_rules(word_chars, 0, length, &mut Vec::new())?
        {
            return Ok(true);
        }

        if self.dictionary.compound_begin != FLAG_UNSET
            || self.dictionary.compound_flag != FLAG_UNSET
        {
            return self.check_compounds(
                &CharsRef::from_chars_offset(word_chars.to_vec(), 0, length),
                original_case,
                None,
            );
        }

        Ok(false)
    }

    /// Equivalent to `findStem(char[], int, int, WordCase, WordContext)`,
    /// with the override of the suggestion speller.
    pub(crate) fn find_stem(
        &self,
        word_chars: &[u16],
        offset: usize,
        length: usize,
        original_case: Option<WordCase>,
        context: WordContext,
    ) -> Result<Option<Root>> {
        if let Some(suggestion_speller) = &self.suggestion_speller {
            if context == WordContext::CompoundBegin && original_case.is_none() {
                let key = word_chars[offset..offset + length].to_vec();
                let cached = suggestion_speller
                    .compound_cache
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .get(&key)
                    .cloned();
                if let Some(cached) = cached {
                    return Ok(cached);
                }
                let found = self.find_stem_base(word_chars, offset, length, None, context)?;
                suggestion_speller
                    .compound_cache
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .insert(key, found.clone());
                return Ok(found);
            }
        }
        self.find_stem_base(word_chars, offset, length, original_case, context)
    }

    /// `Hunspell.findStem`, the method the suggestion speller overrides.
    fn find_stem_base(
        &self,
        word_chars: &[u16],
        offset: usize,
        length: usize,
        original_case: Option<WordCase>,
        context: WordContext,
    ) -> Result<Option<Root>> {
        self.check_canceled()?;
        let to_check =
            if context != WordContext::CompoundMiddle && context != WordContext::CompoundEnd {
                original_case
            } else {
                None
            };
        let mut processor = FindStemProcessor {
            hunspell: self,
            to_check,
            result: None,
        };
        self.stemmer
            .do_stem(word_chars, offset, length, context, &mut processor)?;
        Ok(processor.result)
    }

    /// Equivalent to `acceptCase(WordCase, int, CharsRef)`.
    fn accept_case(&self, original_case: Option<WordCase>, entry_id: i32, root: &[u16]) -> bool {
        let dictionary = &self.dictionary;
        let keep_case = dictionary.has_flag(entry_id, dictionary.keepcase);
        if original_case.is_some() {
            if keep_case
                && dictionary.check_sharp_s
                && original_case == Some(WordCase::Title)
                && contains_sharp_s(root)
            {
                return true;
            }
            return !keep_case;
        }
        !dictionary.has_flag(entry_id, HIDDEN_FLAG)
    }

    /// Equivalent to `acceptsStem(int)`, with the override of the suggestion
    /// speller (no NOSUGGEST, no SUBSTANDARD).
    pub(crate) fn accepts_stem(&self, form_id: i32) -> bool {
        if self.suggestion_speller.is_some() {
            return !self
                .dictionary
                .has_flag(form_id, self.dictionary.no_suggest)
                && !self
                    .dictionary
                    .has_flag(form_id, self.dictionary.sub_standard);
        }
        true
    }

    /// Equivalent to `checkCompounds(CharsRef, WordCase, CompoundPart)`.
    fn check_compounds(
        &self,
        word: &CharsRef,
        original_case: Option<WordCase>,
        prev: Option<&CompoundPart<'_>>,
    ) -> Result<bool> {
        if let Some(prev) = prev {
            if i64::from(prev.index) > i64::from(self.dictionary.compound_max) - 2 {
                return Ok(false);
            }
        }
        if prev.is_none() && word.offset != 0 {
            // we check the word's beginning for FORCEUCASE and expect to find it at 0
            // `new IllegalArgumentException()`, whose message is null; the
            // kind holds an empty one
            return Err(rucene::error::LuceneError::illegal_argument(String::new()));
        }

        let compound_min = self.dictionary.compound_min as i64;
        let limit = word.length as i64 - compound_min + 1;
        let mut break_pos = compound_min;
        while break_pos < limit {
            let bp = break_pos as usize;
            let context = if prev.is_none() {
                WordContext::CompoundBegin
            } else {
                WordContext::CompoundMiddle
            };
            let break_offset = word.offset + bp;
            if self.may_break_into_compounds(&word.chars, word.offset, word.length, break_offset) {
                let mut stem =
                    self.find_stem(&word.chars, word.offset, bp, original_case, context)?;
                if stem.is_none()
                    && self.dictionary.simplified_triple
                    && word.chars[break_offset - 1] == word.chars[break_offset]
                {
                    stem =
                        self.find_stem(&word.chars, word.offset, bp + 1, original_case, context)?;
                }
                if let Some(stem) = stem {
                    if !self
                        .dictionary
                        .has_flag(stem.entry_id, self.dictionary.forbiddenword)
                        && match prev {
                            None => true,
                            Some(prev) => prev.may_compound(self, &stem, bp, original_case)?,
                        }
                    {
                        let part = CompoundPart::new(prev, word.clone(), bp, stem, None);
                        if self.check_compounds_after(original_case, &part)? {
                            return Ok(true);
                        }
                    }
                }
            }

            if self.check_compound_pattern_replacements(word, bp, original_case, prev)? {
                return Ok(true);
            }
            break_pos += 1;
        }

        Ok(false)
    }

    /// Equivalent to `checkCompoundPatternReplacements(CharsRef, int,
    /// WordCase, CompoundPart)`.
    fn check_compound_pattern_replacements(
        &self,
        word: &CharsRef,
        pos: usize,
        original_case: Option<WordCase>,
        prev: Option<&CompoundPart<'_>>,
    ) -> Result<bool> {
        for pattern in &self.dictionary.check_compound_patterns {
            if let Some(expanded) = pattern.expand_replacement(word, pos) {
                let context = if prev.is_none() {
                    WordContext::CompoundBegin
                } else {
                    WordContext::CompoundMiddle
                };
                let break_pos = pos + pattern.end_length();
                let stem = self.find_stem(
                    &expanded.chars,
                    expanded.offset,
                    break_pos,
                    original_case,
                    context,
                )?;
                if let Some(stem) = stem {
                    let part = CompoundPart::new(prev, expanded, break_pos, stem, Some(pattern));
                    if self.check_compounds_after(original_case, &part)? {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }

    /// Equivalent to `checkCompoundsAfter(WordCase, CompoundPart)`.
    fn check_compounds_after(
        &self,
        original_case: Option<WordCase>,
        prev: &CompoundPart<'_>,
    ) -> Result<bool> {
        let word = &prev.tail;
        let break_pos = prev.length;
        let remaining_length = word.length - break_pos;
        let break_offset = word.offset + break_pos;
        let last_root = self.find_stem(
            &word.chars,
            break_offset,
            remaining_length,
            original_case,
            WordContext::CompoundEnd,
        )?;
        if let Some(last_root) = &last_root {
            if !self
                .dictionary
                .has_flag(last_root.entry_id, self.dictionary.forbiddenword)
                && !(self.dictionary.check_compound_dup && prev.root == *last_root)
                && !self.has_force_u_case_problem(last_root, original_case, &word.chars)
                && prev.may_compound(self, last_root, remaining_length, original_case)?
            {
                return Ok(true);
            }
        }

        let tail = CharsRef::from_chars_offset(word.chars.clone(), break_offset, remaining_length);
        self.check_compounds(&tail, original_case, Some(prev))
    }

    /// Equivalent to `hasForceUCaseProblem(Root, WordCase, char[])`.
    fn has_force_u_case_problem(
        &self,
        root: &Root,
        original_case: Option<WordCase>,
        word_chars: &[u16],
    ) -> bool {
        if original_case == Some(WordCase::Title) || original_case == Some(WordCase::Upper) {
            return false;
        }
        if original_case.is_none() && is_upper_case(i32::from(word_chars[0])) {
            return false;
        }
        self.dictionary
            .has_flag(root.entry_id, self.dictionary.force_u_case)
    }

    /// Find all roots that could result in the given word after case
    /// conversion and adding affixes. This corresponds to the original
    /// `hunspell -s` (stemming) functionality.
    ///
    /// Some affix rules are relaxed in this stemming process: e.g. explicitly
    /// forbidden words are still returned. Some of the returned roots may be
    /// synthetic and not directly occur in the *.dic file (but differ from
    /// some existing entries in case). No roots are returned for compound
    /// words.
    ///
    /// The returned roots may be used to retrieve morphological data via
    /// [`Dictionary::lookup_entries`].
    ///
    /// Equivalent to `getRoots(String)`.
    ///
    /// # Errors
    ///
    /// What reading the dictionary raises.
    pub fn get_roots(&self, word: &str) -> Result<Vec<String>> {
        let word = units(word);
        let mut seen = HashSet::new();
        Ok(self
            .stemmer
            .stem(&word, word.len())?
            .into_iter()
            .map(|stem| stem.to_string())
            .filter(|root| seen.insert(root.clone()))
            .collect())
    }

    /// All possible analyses of the given word with stems, prefixes, suffixed
    /// and morphological data. Note that the order of the returned objects
    /// might not correspond to the *.dic file order!
    ///
    /// Equivalent to `analyzeSimpleWord(String)`.
    ///
    /// # Errors
    ///
    /// What reading the dictionary raises.
    pub fn analyze_simple_word(&self, word: &str) -> Result<Vec<AffixedWord>> {
        let word = units(word);
        let mut processor = AnalyzeProcessor {
            dictionary: &self.dictionary,
            word: &word,
            result: Vec::new(),
        };
        self.stemmer.analyze(&word, word.len(), &mut processor)?;
        Ok(processor.result)
    }

    /// Generate all word forms for all dictionary entries with the given root
    /// word. The result order is stable but not specified. This is
    /// equivalent to "unmunch" from the "hunspell-tools" package.
    ///
    /// Equivalent to `getAllWordForms(String)`; see [`WordFormGenerator`] for
    /// finer-grained APIs.
    ///
    /// # Errors
    ///
    /// What `checkCanceled` raises.
    pub fn get_all_word_forms(&self, root: &str) -> Result<Vec<AffixedWord>> {
        WordFormGenerator::new(Arc::clone(&self.dictionary))?
            .get_all_word_forms(root, &self.check_canceled)
    }

    /// Given a list of words, try to produce a smaller set of dictionary
    /// entries (with some flags) that would generate these words. This is
    /// equivalent to "munch" from the "hunspell-tools" package.
    ///
    /// Equivalent to `compress(List<String>)`; see
    /// [`WordFormGenerator::compress`] for more details and control.
    ///
    /// # Errors
    ///
    /// What `checkCanceled` raises.
    pub fn compress(&self, words: &[&str]) -> Result<Option<EntrySuggestion>> {
        WordFormGenerator::new(Arc::clone(&self.dictionary))?.compress(
            words,
            &HashSet::new(),
            &self.check_canceled,
        )
    }

    /// Equivalent to `mayBreakIntoCompounds(char[], int, int, int)`.
    fn may_break_into_compounds(
        &self,
        chars: &[u16],
        offset: usize,
        length: usize,
        break_pos: usize,
    ) -> bool {
        if self.dictionary.check_compound_case {
            let a = chars[break_pos - 1];
            let b = chars[break_pos];
            if (is_upper_case(i32::from(a)) || is_upper_case(i32::from(b)))
                && a != u16::from(b'-')
                && b != u16::from(b'-')
            {
                return false;
            }
        }
        if self.dictionary.check_compound_triple
            && chars[break_pos - 1] == chars[break_pos]
            && (break_pos > offset + 1 && chars[break_pos - 2] == chars[break_pos - 1]
                || (break_pos as i64) < length as i64 - 1
                    && chars[break_pos] == chars[break_pos + 1])
        {
            return false;
        }
        true
    }

    /// Equivalent to `checkCompoundRules(char[], int, int, List<IntsRef>)`.
    fn check_compound_rules(
        &self,
        word_chars: &[u16],
        offset: usize,
        length: usize,
        words: &mut Vec<IntsRef>,
    ) -> Result<bool> {
        if words.len() >= 100 {
            return Ok(false);
        }
        self.check_canceled()?;

        let compound_min = self.dictionary.compound_min as i64;
        let limit = length as i64 - compound_min + 1;
        let mut break_pos = compound_min;
        while break_pos < limit {
            let bp = break_pos as usize;
            if let Some(forms) = self.dictionary.lookup_word(word_chars, offset, bp) {
                words.push(forms);

                if self.may_have_compound_rule(words) {
                    if self.check_last_compound_part(word_chars, offset + bp, length - bp, words)? {
                        return Ok(true);
                    }

                    if self.check_compound_rules(word_chars, offset + bp, length - bp, words)? {
                        return Ok(true);
                    }
                }

                words.pop();
            }
            break_pos += 1;
        }

        Ok(false)
    }

    /// Equivalent to `mayHaveCompoundRule(List<IntsRef>)`.
    fn may_have_compound_rule(&self, words: &[IntsRef]) -> bool {
        self.dictionary
            .compound_rules
            .as_ref()
            .is_some_and(|rules| {
                rules
                    .iter()
                    .any(|rule| rule.may_match(&self.dictionary, words))
            })
    }

    /// Equivalent to `checkLastCompoundPart(char[], int, int, List<IntsRef>)`.
    fn check_last_compound_part(
        &self,
        word_chars: &[u16],
        start: usize,
        length: usize,
        words: &mut Vec<IntsRef>,
    ) -> Result<bool> {
        words.push(IntsRef::with_offset_length(vec![0], 0, 1));
        let mut stop_on_matching = StopOnMatching {
            dictionary: &self.dictionary,
            words,
        };
        let found = !self.stemmer.do_stem(
            word_chars,
            start,
            length,
            WordContext::CompoundRuleEnd,
            &mut stop_on_matching,
        )?;
        words.pop();
        Ok(found)
    }

    /// Equivalent to `tryBreaks(String)`.
    fn try_breaks(&self, word: &[u16]) -> Result<bool> {
        for br in &self.dictionary.breaks.starting {
            if word.len() > br.len()
                && word.starts_with(br)
                && self.spell_units(&word[br.len()..])?
            {
                return Ok(true);
            }
        }

        for br in &self.dictionary.breaks.ending {
            if word.len() > br.len()
                && word.ends_with(br)
                && self.spell_units(&word[..word.len() - br.len()])?
            {
                return Ok(true);
            }
        }

        for br in &self.dictionary.breaks.middle {
            let pos = index_of(word, br, 0);
            if self.can_be_broken_at(word, br, pos)? {
                return Ok(true);
            }

            // try to break at the second occurrence
            // to recognize dictionary words with a word break
            if let Some(pos) = pos {
                if pos > 0 && self.can_be_broken_at(word, br, index_of(word, br, pos + 1))? {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// Equivalent to `hasTooManyBreakOccurrences(String)`.
    fn has_too_many_break_occurrences(&self, word: &[u16]) -> bool {
        let mut occurrences = 0;
        for br in &self.dictionary.breaks.middle {
            let mut pos = 0;
            while let Some(found) = index_of(word, br, pos) {
                occurrences += 1;
                if occurrences >= 10 {
                    return true;
                }
                pos = found + br.len();
            }
        }
        false
    }

    /// Equivalent to `canBeBrokenAt(String, String, int)`.
    fn can_be_broken_at(
        &self,
        word: &[u16],
        break_str: &[u16],
        break_pos: Option<usize>,
    ) -> Result<bool> {
        let Some(break_pos) = break_pos else {
            return Ok(false);
        };
        Ok(break_pos > 0
            && (break_pos as i64) < word.len() as i64 - break_str.len() as i64
            && self.spell_units(&word[..break_pos])?
            && self.spell_units(&word[break_pos + break_str.len()..])?)
    }

    /// Suggestions for the given misspelled word.
    ///
    /// Equivalent to `suggest(String)`, with `SUGGEST_TIME_LIMIT` (250)
    /// milliseconds; see [`Suggester`] for finer-grained APIs and performance
    /// optimizations.
    ///
    /// # Errors
    ///
    /// A [`SuggestionTimeoutException`] if the computation takes too long and
    /// [`TimeoutPolicy::ThrowException`] was specified in the constructor,
    /// and what `checkCanceled` raises.
    pub fn suggest(&self, word: &str) -> Result<Vec<String>> {
        self.suggest_with_time_limit(word, SUGGEST_TIME_LIMIT)
    }

    /// `word` is the misspelled word to calculate suggestions for,
    /// `time_limit_ms` the duration limit in milliseconds, after which the
    /// associated [`TimeoutPolicy`]'s effects (exception or partial result)
    /// may kick in.
    ///
    /// Equivalent to `suggest(String, long)`.
    ///
    /// # Errors
    ///
    /// As [`suggest`](Self::suggest).
    pub fn suggest_with_time_limit(&self, word: &str, time_limit_ms: i64) -> Result<Vec<String>> {
        let suggester = Suggester::new(Arc::clone(&self.dictionary));
        if self.policy == TimeoutPolicy::NoTimeout {
            return suggester.suggest_no_timeout(word, Arc::clone(&self.check_canceled));
        }

        match suggester.suggest_with_timeout(word, time_limit_ms, Arc::clone(&self.check_canceled))
        {
            Err(e) => match SuggestionTimeoutException::of(&e) {
                Some(timeout) if self.policy == TimeoutPolicy::ReturnPartialResult => Ok(timeout
                    .get_partial_result()
                    .map(<[String]>::to_vec)
                    .unwrap_or_default()),
                _ => Err(e),
            },
            ok => ok,
        }
    }
}

/// Equivalent to `containsSharpS(char[], int, int)`.
fn contains_sharp_s(word: &[u16]) -> bool {
    word.contains(&0x00DF)
}

/// Equivalent to `isNumber(String)`.
fn is_number(s: &[u16]) -> bool {
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        if is_digit(c) {
            i += 1;
        } else if c == u16::from(b'.') || c == u16::from(b',') || c == u16::from(b'-') {
            if i == 0 || i >= s.len() - 1 || !is_digit(s[i + 1]) {
                return false;
            }
            i += 2;
        } else {
            return false;
        }
    }
    true
}

/// Equivalent to `isDigit(char)`.
fn is_digit(c: u16) -> bool {
    (u16::from(b'0')..=u16::from(b'9')).contains(&c)
}

/// One part of a compound word being checked, linked to the parts before it.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.Hunspell$CompoundPart`.
#[derive(Debug)]
pub(crate) struct CompoundPart<'a> {
    prev: Option<&'a CompoundPart<'a>>,
    index: i32,
    length: usize,
    tail: CharsRef,
    root: Root,
    enabling_pattern: Option<&'a CheckCompoundPattern>,
}

impl<'a> CompoundPart<'a> {
    /// Equivalent to `CompoundPart(CompoundPart, CharsRef, int, Root,
    /// CheckCompoundPattern)`.
    fn new(
        prev: Option<&'a CompoundPart<'a>>,
        tail: CharsRef,
        length: usize,
        root: Root,
        enabler: Option<&'a CheckCompoundPattern>,
    ) -> Self {
        let index = match prev {
            None => 1,
            Some(prev) => prev.index + 1,
        };
        Self {
            prev,
            index,
            length,
            tail,
            root,
            enabling_pattern: enabler,
        }
    }

    /// Equivalent to `mayCompound(Root, int, WordCase)`.
    fn may_compound(
        &self,
        hunspell: &Hunspell,
        next_root: &Root,
        next_part_length: usize,
        original_case: Option<WordCase>,
    ) -> Result<bool> {
        let dictionary = &hunspell.dictionary;
        let patterns_ok = match self.enabling_pattern {
            Some(pattern) => pattern.prohibits_compounding(
                dictionary,
                &self.tail,
                self.length,
                &self.root,
                next_root,
            ),
            None => !dictionary.check_compound_patterns.iter().any(|p| {
                p.prohibits_compounding(dictionary, &self.tail, self.length, &self.root, next_root)
            }),
        };
        if !patterns_ok {
            return Ok(false);
        }

        if dictionary.check_compound_rep
            && self.is_misspelled_simple_word(
                hunspell,
                self.length + next_part_length,
                original_case,
            )?
        {
            return Ok(false);
        }

        let mut space_separated = vec![0u16; self.length + next_part_length + 1];
        let start = self.tail.offset;
        space_separated[..self.length]
            .copy_from_slice(&self.tail.chars[start..start + self.length]);
        space_separated[self.length + 1..].copy_from_slice(
            &self.tail.chars[start + self.length..start + self.length + next_part_length],
        );
        space_separated[self.length] = u16::from(b' ');
        Ok(
            hunspell.check_simple_word(&space_separated, space_separated.len(), None)?
                != Some(true),
        )
    }

    /// Equivalent to `isMisspelledSimpleWord(int, WordCase)`.
    fn is_misspelled_simple_word(
        &self,
        hunspell: &Hunspell,
        length: usize,
        original_case: Option<WordCase>,
    ) -> Result<bool> {
        let start = self.tail.offset;
        let word = &self.tail.chars[start..start + length];
        for entry in &hunspell.dictionary.rep_table {
            if entry.is_middle() {
                for sug in entry.substitute(word) {
                    if hunspell
                        .find_stem(&sug, 0, sug.len(), original_case, WordContext::SimpleWord)?
                        .is_some()
                    {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }
}

impl std::fmt::Display for CompoundPart<'_> {
    /// Equivalent to `toString()`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(prev) = self.prev {
            write!(f, "{prev}+")?;
        }
        let start = self.tail.offset;
        f.write_str(&to_rust_string(
            &self.tail.chars[start..start + self.length],
        ))
    }
}
