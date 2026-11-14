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

//! The suggestion generator, ported from
//! `org.apache.lucene.analysis.hunspell.Suggester`.

use std::collections::HashSet;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rucene::error::{LuceneError, LuceneErrorKind, Result};

use super::dictionary::{Dictionary, FLAG_UNSET};
use super::fragment_checker::{everything_possible, FragmentChecker};
use super::generating_suggester::GeneratingSuggester;
use super::hunspell::{CheckCanceled, Hunspell};
use super::modifying_suggester::ModifyingSuggester;
use super::suggestible_entry_cache::SuggestibleEntryCache;
use super::suggestion::Suggestion;
use super::suggestion_timeout_exception::SuggestionTimeoutException;
use super::word_case::WordCase;
use crate::jdk::string::{concat, contains_str, index_of_unit, to_rust_string, units};

/// The suggestions found so far: Java's `LinkedHashSet<Suggestion>`, in
/// insertion order.
#[derive(Debug, Default, Clone)]
pub(crate) struct SuggestionSet {
    items: Vec<Suggestion>,
    seen: HashSet<Suggestion>,
}

impl SuggestionSet {
    /// Equivalent to `add(E)`: whether the suggestion was not there yet.
    pub(crate) fn add(&mut self, suggestion: Suggestion) -> bool {
        if self.seen.contains(&suggestion) {
            return false;
        }
        self.seen.insert(suggestion.clone());
        self.items.push(suggestion);
        true
    }

    /// Equivalent to `addAll(Collection)`.
    pub(crate) fn add_all(&mut self, suggestions: Vec<Suggestion>) {
        for suggestion in suggestions {
            self.add(suggestion);
        }
    }

    /// Equivalent to `clear()`.
    pub(crate) fn clear(&mut self) {
        self.items.clear();
        self.seen.clear();
    }

    /// Equivalent to `size()`.
    pub(crate) fn len(&self) -> usize {
        self.items.len()
    }

    /// The suggestions in order.
    pub(crate) fn to_vec(&self) -> Vec<Suggestion> {
        self.items.clone()
    }
}

/// A generator for misspelled word corrections based on Hunspell flags. The
/// suggestions are searched for in two main ways:
///
/// 1. Modification: trying to insert/remove/delete/swap parts of the word to
///    get something acceptable. The performance of this part depends heavily
///    on the contents of TRY, MAP, REP, KEY directives in the .aff file. To
///    speed up this part, consider using
///    [`with_fragment_checker`](Self::with_fragment_checker).
/// 2. Enumeration: if the modification hasn't produced "good enough"
///    suggestions, the whole dictionary is scanned and simple affixes are
///    added onto the entries to check if that produces anything similar to
///    the given misspelled word. This depends on the dictionary size and the
///    affix count, and it can take noticeable amount of time. To speed this
///    up, [`with_suggestible_entry_cache`](Self::with_suggestible_entry_cache)
///    can be used.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.Suggester`.
#[derive(Clone)]
pub struct Suggester {
    dictionary: Arc<Dictionary>,
    suggestible_cache: Option<Arc<SuggestibleEntryCache>>,
    fragment_checker: Arc<dyn FragmentChecker>,
    proceed_past_rep: bool,
}

impl std::fmt::Debug for Suggester {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Suggester")
            .field("suggestible_cache", &self.suggestible_cache.is_some())
            .field("proceed_past_rep", &self.proceed_past_rep)
            .finish_non_exhaustive()
    }
}

impl Suggester {
    /// Equivalent to `Suggester(Dictionary)`.
    pub fn new(dictionary: Arc<Dictionary>) -> Self {
        Self::with_all(dictionary, None, everything_possible(), false)
    }

    /// Equivalent to the private `Suggester(Dictionary,
    /// SuggestibleEntryCache, FragmentChecker, boolean)`.
    fn with_all(
        dictionary: Arc<Dictionary>,
        suggestible_cache: Option<Arc<SuggestibleEntryCache>>,
        checker: Arc<dyn FragmentChecker>,
        proceed_past_rep: bool,
    ) -> Self {
        Self {
            dictionary,
            suggestible_cache,
            fragment_checker: checker,
            proceed_past_rep,
        }
    }

    /// Returns a copy of this suggester instance with better "Enumeration"
    /// phase performance (see [`Suggester`] documentation), but using more
    /// memory. With this option, the dictionary entries are stored as
    /// fast-to-iterate plain words instead of highly compressed prefix
    /// trees.
    ///
    /// Equivalent to `withSuggestibleEntryCache()`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` for an entry longer than 32767 units.
    pub fn with_suggestible_entry_cache(&self) -> Result<Suggester> {
        let cache = SuggestibleEntryCache::build_cache(&self.dictionary.words)?;
        Ok(Self::with_all(
            Arc::clone(&self.dictionary),
            Some(Arc::new(cache)),
            Arc::clone(&self.fragment_checker),
            self.proceed_past_rep,
        ))
    }

    /// Returns a copy of this suggester instance with [`FragmentChecker`]
    /// hint that can improve the performance of the "Modification" phase
    /// performance.
    ///
    /// Equivalent to `withFragmentChecker(FragmentChecker)`.
    pub fn with_fragment_checker(&self, checker: Arc<dyn FragmentChecker>) -> Suggester {
        Self::with_all(
            Arc::clone(&self.dictionary),
            self.suggestible_cache.clone(),
            checker,
            self.proceed_past_rep,
        )
    }

    /// Returns a copy of this suggester instance that doesn't stop after
    /// encountering acceptable words after applying REP rules. By default,
    /// Hunspell stops when it finds any, but this behavior may not always be
    /// desirable, e.g., if we have "REP i ea", "tims" be replaced only by
    /// "teams" and not "times", which could also be meant.
    ///
    /// Equivalent to `proceedPastRep()`.
    pub fn proceed_past_rep(&self) -> Suggester {
        Self::with_all(
            Arc::clone(&self.dictionary),
            self.suggestible_cache.clone(),
            Arc::clone(&self.fragment_checker),
            true,
        )
    }

    /// Compute suggestions for the given misspelled word; `check_canceled`
    /// is an object that's periodically called, allowing to interrupt or
    /// suggestion generation by throwing an exception.
    ///
    /// Equivalent to `suggestNoTimeout(String, Runnable)`.
    ///
    /// # Errors
    ///
    /// What `check_canceled` raises; a [`SuggestionTimeoutException`] it
    /// raises without a partial result gets the suggestions found so far.
    pub fn suggest_no_timeout(
        &self,
        word: &str,
        check_canceled: CheckCanceled,
    ) -> Result<Vec<String>> {
        self.suggest_no_timeout_units(&units(word), check_canceled)
            .map(|suggestions| suggestions.iter().map(|s| to_rust_string(s)).collect())
    }

    /// [`suggest_no_timeout`](Self::suggest_no_timeout) of a word given as
    /// UTF-16 units.
    fn suggest_no_timeout_units(
        &self,
        word: &[u16],
        check_canceled: CheckCanceled,
    ) -> Result<Vec<Vec<u16>>> {
        let suggestions = Arc::new(Mutex::new(SuggestionSet::default()));
        let handled = handle_custom_timeout_exception(check_canceled, Arc::clone(&suggestions));
        self.suggest(word, suggestions, handled)
    }

    /// `word` is the misspelled word to calculate suggestions for,
    /// `time_limit_ms` the duration limit in milliseconds after which the
    /// computation is interruped by an exception, and `check_canceled` an
    /// object that's periodically called, allowing to interrupt or
    /// suggestion generation by throwing an exception.
    ///
    /// Equivalent to `suggestWithTimeout(String, long, Runnable)`; Java's
    /// `System.nanoTime()` is [`Instant::now`].
    ///
    /// # Errors
    ///
    /// A [`SuggestionTimeoutException`] if the computation takes too long:
    /// use [`SuggestionTimeoutException::get_partial_result`] to get the
    /// suggestions computed up to that point.
    pub fn suggest_with_timeout(
        &self,
        word: &str,
        time_limit_ms: i64,
        check_canceled: CheckCanceled,
    ) -> Result<Vec<String>> {
        let word = units(word);
        let suggestions = Arc::new(Mutex::new(SuggestionSet::default()));
        let check_time = check_time_limit(
            &word,
            Arc::clone(&suggestions),
            time_limit_ms,
            check_canceled,
        );
        let handled = handle_custom_timeout_exception(check_time, Arc::clone(&suggestions));
        self.suggest(&word, suggestions, handled)
            .map(|suggestions| suggestions.iter().map(|s| to_rust_string(s)).collect())
    }

    /// Equivalent to `suggest(String, LinkedHashSet<Suggestion>, Runnable)`.
    fn suggest(
        &self,
        word: &[u16],
        suggestions: Arc<Mutex<SuggestionSet>>,
        check_canceled: CheckCanceled,
    ) -> Result<Vec<Vec<u16>>> {
        check_canceled()?;
        if word.len() >= 100 {
            return Ok(Vec::new());
        }

        let mut word = word.to_vec();
        if self.dictionary.needs_input_cleaning(&word) {
            let mut cleaned = Vec::new();
            self.dictionary.clean_input(&word, &mut cleaned)?;
            word = cleaned;
        }
        if word.is_empty() {
            // `WordCase.caseOf(word)` reads `word.charAt(0)`
            return Err(LuceneError::from(LuceneErrorKind::StringIndexOutOfBounds(
                "Index 0 out of bounds for length 0".to_string(),
            )));
        }

        let suggestion_speller =
            Hunspell::for_suggestions(Arc::clone(&self.dictionary), Arc::clone(&check_canceled));

        let word_case = WordCase::case_of(&word);
        if self.dictionary.force_u_case != FLAG_UNSET && word_case == WordCase::Lower {
            let title = self.dictionary.to_title_case(&word);
            if suggestion_speller.spell_units(&title)? {
                return Ok(vec![title]);
            }
        }

        let has_good_suggestions = ModifyingSuggester::new(
            &suggestion_speller,
            Arc::clone(&suggestions),
            word.clone(),
            word_case,
            self.fragment_checker.as_ref(),
            self.proceed_past_rep,
        )
        .suggest()?;

        if !has_good_suggestions && self.dictionary.max_n_gram_suggestions > 0 {
            let previous = lock(&suggestions).to_vec();
            let generated =
                GeneratingSuggester::new(&suggestion_speller, self.suggestible_cache.as_deref())
                    .suggest(&self.dictionary.to_lower_case(&word), word_case, &previous)?;
            for raw in generated {
                let suggestion = Suggestion::new(raw, &word, word_case, &suggestion_speller)?;
                lock(&suggestions).add(suggestion);
            }
        }

        let dash = u16::from(b'-');
        if contains_str(&word, "-")
            && !lock(&suggestions)
                .to_vec()
                .iter()
                .any(|s| s.raw.contains(&dash))
        {
            for raw in
                self.modify_chunks_between_dashes(&word, &suggestion_speller, &check_canceled)?
            {
                let suggestion = Suggestion::new(raw, &word, word_case, &suggestion_speller)?;
                lock(&suggestions).add(suggestion);
            }
        }
        let suggestions = lock(&suggestions).to_vec();
        Ok(postprocess(&suggestions))
    }

    /// Equivalent to `modifyChunksBetweenDashes(String, Hunspell, Runnable)`.
    fn modify_chunks_between_dashes(
        &self,
        word: &[u16],
        speller: &Hunspell,
        check_canceled: &CheckCanceled,
    ) -> Result<Vec<Vec<u16>>> {
        let mut result = Vec::new();
        let mut chunk_start = 0;
        while chunk_start < word.len() {
            let chunk_end = index_of_unit(word, u16::from(b'-'), chunk_start).unwrap_or(word.len());

            if chunk_end > chunk_start {
                let chunk = &word[chunk_start..chunk_end];
                if !speller.spell_units(chunk)? {
                    for chunk_sug in
                        self.suggest_no_timeout_units(chunk, Arc::clone(check_canceled))?
                    {
                        let replaced =
                            concat(&[&word[..chunk_start], &chunk_sug, &word[chunk_end..]]);
                        if speller.spell_units(&replaced)? {
                            result.push(replaced);
                        }
                    }
                }
            }

            chunk_start = chunk_end + 1;
        }
        Ok(result)
    }
}

/// Locks the shared suggestion set.
fn lock(suggestions: &Mutex<SuggestionSet>) -> std::sync::MutexGuard<'_, SuggestionSet> {
    suggestions
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Equivalent to `handleCustomTimeoutException(Runnable,
/// LinkedHashSet<Suggestion>)`: a [`SuggestionTimeoutException`] without a
/// partial result gets the suggestions found so far.
fn handle_custom_timeout_exception(
    check_canceled: CheckCanceled,
    suggestions: Arc<Mutex<SuggestionSet>>,
) -> CheckCanceled {
    Arc::new(move || match check_canceled() {
        Err(e) => match SuggestionTimeoutException::of(&e) {
            Some(timeout) if timeout.get_partial_result().is_none() => {
                let partial = postprocess_strings(&lock(&suggestions).to_vec());
                Err(SuggestionTimeoutException::error(
                    timeout.get_message().map(str::to_string),
                    Some(partial),
                ))
            }
            _ => Err(e),
        },
        ok => ok,
    })
}

/// Equivalent to `checkTimeLimit(String, Set<Suggestion>, long, Runnable)`:
/// every hundredth call checks the deadline.
fn check_time_limit(
    word: &[u16],
    suggestions: Arc<Mutex<SuggestionSet>>,
    time_limit_ms: i64,
    check_canceled: CheckCanceled,
) -> CheckCanceled {
    // `System.nanoTime() + TimeUnit.MILLISECONDS.toNanos(timeLimitMs)`; a
    // deadline beyond what an `Instant` holds is never reached
    let start = Instant::now();
    let deadline = if time_limit_ms >= 0 {
        start.checked_add(Duration::from_millis(time_limit_ms as u64))
    } else {
        start
            .checked_sub(Duration::from_millis(time_limit_ms.unsigned_abs()))
            .or(Some(start))
    };
    let invocation_counter = AtomicI32::new(100);
    let message = format!(
        "Time limit of {time_limit_ms}ms exceeded for {}",
        to_rust_string(word)
    );
    Arc::new(move || {
        check_canceled()?;
        if invocation_counter.fetch_sub(1, Ordering::Relaxed) - 1 <= 0 {
            if deadline.is_some_and(|deadline| Instant::now() > deadline) {
                // `stop()`
                let partial = postprocess_strings(&lock(&suggestions).to_vec());
                return Err(SuggestionTimeoutException::error(
                    Some(message.clone()),
                    Some(partial),
                ));
            }
            invocation_counter.store(100, Ordering::Relaxed);
        }
        Ok(())
    })
}

/// Equivalent to `postprocess(Collection<Suggestion>)`: the results of every
/// suggestion, without duplicates, in order.
fn postprocess(suggestions: &[Suggestion]) -> Vec<Vec<u16>> {
    let mut seen = HashSet::new();
    suggestions
        .iter()
        .flat_map(|s| s.result.iter())
        .filter(|r| seen.insert((*r).clone()))
        .cloned()
        .collect()
}

/// [`postprocess`] as Rust strings, the partial result of a timeout.
fn postprocess_strings(suggestions: &[Suggestion]) -> Vec<String> {
    postprocess(suggestions)
        .iter()
        .map(|s| to_rust_string(s))
        .collect()
}
