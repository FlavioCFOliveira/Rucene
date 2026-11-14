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

//! The suggester that scans the whole dictionary, ported from
//! `org.apache.lucene.analysis.hunspell.GeneratingSuggester`.

use std::cmp::Ordering;
use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

use rucene::error::Result;
use rucene::util::fst::{Arc as FSTArc, Outputs};
use rucene::util::IntsRef;

use super::dictionary::{Dictionary, AFFIX_APPEND, AFFIX_FLAG, AFFIX_STRIP_ORD};
use super::flyweight_entry::FlyweightEntry;
use super::hunspell::Hunspell;
use super::root::Root;
use super::suggestible_entry_cache::SuggestibleEntryCache;
use super::suggestion::Suggestion;
use super::trigram_automaton::TrigramAutomaton;
use super::word_case::WordCase;
use crate::jdk::string::{contains, region_matches};

const MAX_ROOTS: usize = 100;
const MAX_WORDS: usize = 100;
const MAX_GUESSES: usize = 200;
const MAX_ROOT_LENGTH_DIFF: i32 = 4;

/// `java.util.PriorityQueue` with a comparator: a binary heap whose
/// iteration order is its array order and whose `poll` and `offer` sift as
/// the JDK's do, so that the order of equal elements is the JDK's.
pub(crate) struct JavaPriorityQueue<T, C: Fn(&T, &T) -> Ordering> {
    queue: Vec<T>,
    comparator: C,
}

impl<T, C: Fn(&T, &T) -> Ordering> JavaPriorityQueue<T, C> {
    /// Equivalent to `new PriorityQueue<>(comparator)`.
    pub(crate) fn new(comparator: C) -> Self {
        Self {
            queue: Vec::new(),
            comparator,
        }
    }

    /// Equivalent to `size()`.
    pub(crate) fn len(&self) -> usize {
        self.queue.len()
    }

    /// Equivalent to `isEmpty()`.
    #[allow(dead_code)] // A member of the Java class that Lucene 10.5.0's main code does not use.
    pub(crate) fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Equivalent to `peek()`.
    pub(crate) fn peek(&self) -> Option<&T> {
        self.queue.first()
    }

    /// Equivalent to `offer(E)` and `add(E)`: `siftUpUsingComparator`.
    pub(crate) fn offer(&mut self, x: T) {
        self.queue.push(x);
        let mut k = self.queue.len() - 1;
        while k > 0 {
            let parent = (k - 1) >> 1;
            if (self.comparator)(&self.queue[k], &self.queue[parent]) != Ordering::Less {
                break;
            }
            self.queue.swap(k, parent);
            k = parent;
        }
    }

    /// Equivalent to `poll()`: `siftDownUsingComparator` of the last element.
    pub(crate) fn poll(&mut self) -> Option<T> {
        if self.queue.is_empty() {
            return None;
        }
        let result = self.queue.swap_remove(0);
        let n = self.queue.len();
        if n > 0 {
            let mut k = 0;
            let half = n >> 1;
            while k < half {
                let mut child = (k << 1) + 1;
                let right = child + 1;
                if right < n
                    && (self.comparator)(&self.queue[child], &self.queue[right])
                        == Ordering::Greater
                {
                    child = right;
                }
                if (self.comparator)(&self.queue[k], &self.queue[child]) != Ordering::Greater {
                    break;
                }
                self.queue.swap(k, child);
                k = child;
            }
        }
        Some(result)
    }

    /// The elements in the queue's array order, as its iterator returns
    /// them.
    pub(crate) fn into_vec(self) -> Vec<T> {
        self.queue
    }
}

/// A class that traverses the entire dictionary and applies affix rules to
/// check if those yield correct suggestions similar enough to the given
/// misspelled word.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.GeneratingSuggester`.
pub(crate) struct GeneratingSuggester<'a> {
    dictionary: Arc<Dictionary>,
    speller: &'a Hunspell,
    entry_cache: Option<&'a SuggestibleEntryCache>,
}

/// The ordering of a [`Weighted`] word: `T.compareTo`.
pub(crate) trait WeightedWord {
    /// Equivalent to `compareTo(T)`.
    fn compare_word(&self, other: &Self) -> Ordering;
}

impl WeightedWord for Root {
    fn compare_word(&self, other: &Self) -> Ordering {
        self.compare_to(other)
    }
}

impl WeightedWord for Vec<u16> {
    fn compare_word(&self, other: &Self) -> Ordering {
        self.cmp(other)
    }
}

/// A word with a score.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.GeneratingSuggester$Weighted`,
/// a record ordered by descending score, then by word.
#[derive(Debug, Clone)]
pub(crate) struct Weighted<T: WeightedWord> {
    word: T,
    score: i32,
}

impl<T: WeightedWord> Weighted<T> {
    fn new(word: T, score: i32) -> Self {
        Self { word, score }
    }

    /// Equivalent to `compareTo(Weighted)`.
    fn compare_to(&self, o: &Self) -> Ordering {
        o.score
            .cmp(&self.score)
            .then_with(|| self.word.compare_word(&o.word))
    }
}

impl<T: WeightedWord> PartialEq for Weighted<T> {
    fn eq(&self, other: &Self) -> bool {
        self.compare_to(other) == Ordering::Equal
    }
}

impl<T: WeightedWord> Eq for Weighted<T> {}

impl<T: WeightedWord> PartialOrd for Weighted<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T: WeightedWord> Ord for Weighted<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.compare_to(other)
    }
}

/// The processor of `processAffixes`: an affix length and an affix id.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.GeneratingSuggester$AffixProcessor`.
pub(crate) trait AffixProcessor {
    /// Equivalent to `processAffix(int, int)`.
    fn process_affix(&mut self, affix_length: usize, affix_id: i32);
}

impl<F: FnMut(usize, i32)> AffixProcessor for F {
    fn process_affix(&mut self, affix_length: usize, affix_id: i32) {
        self(affix_length, affix_id);
    }
}

impl<'a> GeneratingSuggester<'a> {
    /// Equivalent to `GeneratingSuggester(Hunspell, SuggestibleEntryCache)`.
    pub(crate) fn new(
        speller: &'a Hunspell,
        entry_cache: Option<&'a SuggestibleEntryCache>,
    ) -> Self {
        Self {
            dictionary: Arc::clone(&speller.dictionary),
            speller,
            entry_cache,
        }
    }

    /// Equivalent to `suggest(String, WordCase, Set<Suggestion>)`.
    ///
    /// # Errors
    ///
    /// What `checkCanceled` raises.
    pub(crate) fn suggest(
        &self,
        word: &[u16],
        original_case: WordCase,
        prev_suggestions: &[Suggestion],
    ) -> Result<Vec<Vec<u16>>> {
        let roots = self.find_similar_dictionary_entries(word, original_case)?;
        let expanded = self.expand_roots(word, &roots)?;
        let by_similarity = self.rank_by_similarity(word, &expanded);
        self.get_most_relevant_suggestions(&by_similarity, prev_suggestions)
    }

    /// Equivalent to `findSimilarDictionaryEntries(String, WordCase)`.
    fn find_similar_dictionary_entries(
        &self,
        word: &[u16],
        original_case: WordCase,
    ) -> Result<Vec<Weighted<Root>>> {
        let mut roots =
            JavaPriorityQueue::new(|a: &Weighted<Root>, b: &Weighted<Root>| b.compare_to(a));
        let ignore_title_case_roots =
            original_case == WordCase::Lower && !self.dictionary.has_language(&["de"]);
        let mut automaton = TrigramAutomaton::new(word)?;

        let form_step = self.dictionary.form_step();
        self.process_suggestible_words(
            1.max(word.len() as i32 - MAX_ROOT_LENGTH_DIFF),
            word.len() as i32 + MAX_ROOT_LENGTH_DIFF,
            &mut |entry: &mut dyn FlyweightEntry| {
                if ignore_title_case_roots && entry.has_title_case() {
                    return Ok(());
                }

                let mut sc = automaton.ngram_score(&entry.lower_case_root());
                if sc == 0 {
                    return Ok(()); // no common characters at all, don't suggest this root
                }

                let root_chars = entry.root().to_vec();
                sc += common_prefix(word, &root_chars) as i32
                    - longer_worse_penalty(word.len() as i32, root_chars.len() as i32);

                if roots.len() == MAX_ROOTS
                    && is_worse_than(sc, &root_chars, roots.peek().expect("INVARIANT: full"))
                {
                    return Ok(());
                }

                self.speller.check_canceled()?;

                let forms = entry.forms();
                for i in (0..forms.len()).step_by(form_step) {
                    let form = forms[i];
                    roots.offer(Weighted::new(Root::new(root_chars.clone(), form), sc));
                    if roots.len() > MAX_ROOTS {
                        roots.poll();
                    }
                }
                Ok(())
            },
        )?;

        let mut sorted = roots.into_vec();
        sorted.sort_by(Weighted::compare_to);
        Ok(sorted)
    }

    /// Equivalent to `processSuggestibleWords(int, int, Consumer)`.
    fn process_suggestible_words(
        &self,
        min_length: i32,
        max_length: i32,
        processor: &mut dyn FnMut(&mut dyn FlyweightEntry) -> Result<()>,
    ) -> Result<()> {
        match self.entry_cache {
            Some(entry_cache) => {
                entry_cache.process_suggestible_words(min_length, max_length, processor)
            }
            None => self
                .dictionary
                .words
                .process_suggestible_words(min_length, max_length, processor),
        }
    }

    /// Equivalent to `expandRoots(String, List<Weighted<Root<String>>>)`.
    fn expand_roots(
        &self,
        misspelled: &[u16],
        roots: &[Weighted<Root>],
    ) -> Result<Vec<Weighted<Vec<u16>>>> {
        let thresh = calc_threshold(misspelled);

        let mut expanded: BTreeSet<Weighted<Vec<u16>>> = BTreeSet::new();
        for weighted in roots {
            for guess in self.expand_root(&weighted.word, misspelled)? {
                let lower = self.dictionary.to_lower_case(&guess);
                let sc = any_mismatch_ngram(misspelled.len() as i32, misspelled, &lower, false)
                    + common_prefix(misspelled, &guess) as i32;
                if sc > thresh {
                    expanded.insert(Weighted::new(guess, sc));
                }
            }
        }
        Ok(expanded.into_iter().take(MAX_GUESSES).collect())
    }

    /// Equivalent to `expandRoot(Root<String>, String)`.
    fn expand_root(&self, root: &Root, misspelled: &[u16]) -> Result<Vec<Vec<u16>>> {
        let dictionary = &self.dictionary;
        let mut cross_products: Vec<Vec<u16>> = Vec::new();
        let mut result = LinkedStrings::default();

        if !dictionary.has_flag(root.entry_id, dictionary.needaffix) {
            result.add(root.word.clone());
        }

        let word_chars = &root.word;

        // suffixes
        self.process_affixes(
            false,
            misspelled,
            &mut |suffix_length: usize, suffix_id: i32| {
                let strip_length = self.affix_strip_length(suffix_id);
                if !self.has_compatible_flags(root, suffix_id)
                    || !self.check_affix_condition(
                        suffix_id,
                        word_chars,
                        0,
                        word_chars.len() as i64 - strip_length as i64,
                    )
                {
                    return;
                }

                let suffix = &misspelled[misspelled.len() - suffix_length..];
                let mut with_suffix = root.word[..root.word.len() - strip_length].to_vec();
                with_suffix.extend_from_slice(suffix);
                result.add(with_suffix.clone());
                if dictionary.is_cross_product(suffix_id) {
                    cross_products.push(with_suffix);
                }
            },
        )?;

        // cross-product prefixes
        self.process_affixes(
            true,
            misspelled,
            &mut |prefix_length: usize, prefix_id: i32| {
                if !dictionary.has_flag(root.entry_id, dictionary.affix_data(prefix_id, AFFIX_FLAG))
                    || !dictionary.is_cross_product(prefix_id)
                {
                    return;
                }

                let strip_length = self.affix_strip_length(prefix_id);
                let prefix = &misspelled[..prefix_length];
                for suffixed in &cross_products {
                    let stem_length = suffixed.len() as i64 - strip_length as i64;
                    if self.check_affix_condition(prefix_id, suffixed, strip_length, stem_length) {
                        let mut word = prefix.to_vec();
                        word.extend_from_slice(
                            &suffixed[strip_length..strip_length + stem_length as usize],
                        );
                        result.add(word);
                    }
                }
            },
        )?;

        // pure prefixes
        self.process_affixes(
            true,
            misspelled,
            &mut |prefix_length: usize, prefix_id: i32| {
                let strip_length = self.affix_strip_length(prefix_id);
                let stem_length = word_chars.len() as i64 - strip_length as i64;
                if self.has_compatible_flags(root, prefix_id)
                    && self.check_affix_condition(prefix_id, word_chars, strip_length, stem_length)
                {
                    let mut word = misspelled[..prefix_length].to_vec();
                    word.extend_from_slice(&root.word[strip_length..]);
                    result.add(word);
                }
            },
        )?;

        Ok(result.items.into_iter().take(MAX_WORDS).collect())
    }

    /// Equivalent to `processAffixes(boolean, String, AffixProcessor)`.
    fn process_affixes(
        &self,
        prefixes: bool,
        word: &[u16],
        processor: &mut dyn AffixProcessor,
    ) -> Result<()> {
        let fst = if prefixes {
            &self.dictionary.prefixes
        } else {
            &self.dictionary.suffixes
        };
        let Some(fst) = fst else {
            return Ok(());
        };

        let mut arc: FSTArc<IntsRef> = FSTArc::default();
        fst.get_first_arc(&mut arc);
        if arc.is_final() {
            process_affix_ids(0, arc.next_final_output(), processor);
        }

        let mut reader = fst.get_bytes_reader()?;

        let mut output = fst.outputs().no_output();
        let length = word.len() as i64;
        let step: i64 = if prefixes { 1 } else { -1 };
        let limit: i64 = if prefixes { length } else { -1 };
        let mut i: i64 = if prefixes { 0 } else { length - 1 };
        while i != limit {
            match Dictionary::next_arc(
                fst,
                &mut arc,
                reader.as_mut(),
                &output,
                i32::from(word[i as usize]),
            )? {
                None => break,
                Some(next) => output = next,
            }
            if arc.is_final() {
                let affix_ids = fst.outputs().add(&output, arc.next_final_output());
                process_affix_ids(
                    if prefixes {
                        (i + 1) as usize
                    } else {
                        (length - i) as usize
                    },
                    &affix_ids,
                    processor,
                );
            }
            i += step;
        }
        Ok(())
    }

    /// Equivalent to `hasCompatibleFlags(Root, int)`.
    fn has_compatible_flags(&self, root: &Root, affix_id: i32) -> bool {
        let dictionary = &self.dictionary;
        if !dictionary.has_flag(root.entry_id, dictionary.affix_data(affix_id, AFFIX_FLAG)) {
            return false;
        }

        let append = i32::from(dictionary.affix_data(affix_id, AFFIX_APPEND));
        !dictionary.has_flag(append, dictionary.needaffix)
            && !dictionary.has_flag(append, dictionary.circumfix)
            && !dictionary.has_flag(append, dictionary.onlyincompound)
    }

    /// Equivalent to `checkAffixCondition(int, char[], int, int)`.
    fn check_affix_condition(
        &self,
        suffix_id: i32,
        word: &[u16],
        offset: usize,
        length: i64,
    ) -> bool {
        if length < 0 {
            return false;
        }
        let condition = self.dictionary.get_affix_condition(suffix_id);
        condition == 0
            || self.dictionary.patterns[condition]
                .as_ref()
                .expect("INVARIANT: only pattern 0 is null")
                .accepts_stem(word, offset, length as usize)
    }

    /// Equivalent to `affixStripLength(int)`.
    fn affix_strip_length(&self, affix_id: i32) -> usize {
        let strip_ord = usize::from(self.dictionary.affix_data(affix_id, AFFIX_STRIP_ORD));
        (self.dictionary.strip_offsets[strip_ord + 1] - self.dictionary.strip_offsets[strip_ord])
            as usize
    }

    /// Equivalent to `rankBySimilarity(String, List<Weighted<String>>)`.
    fn rank_by_similarity(
        &self,
        word: &[u16],
        expanded: &[Weighted<Vec<u16>>],
    ) -> BTreeSet<Weighted<Vec<u16>>> {
        let fact = (10.0 - f64::from(self.dictionary.max_diff)) / 5.0;
        let mut by_similarity = BTreeSet::new();
        for weighted in expanded {
            let guess = &weighted.word;
            let lower = self.dictionary.to_lower_case(guess);
            if lower == word {
                by_similarity.insert(Weighted::new(guess.clone(), weighted.score + 2000));
                break;
            }

            let re = any_mismatch_ngram(2, word, &lower, true)
                + any_mismatch_ngram(2, &lower, word, true);

            let score = 2 * lcs(word, &lower) - (word.len() as i32 - lower.len() as i32).abs()
                + common_character_position_score(word, &lower)
                + common_prefix(word, &lower) as i32
                + any_mismatch_ngram(4, word, &lower, false)
                + re
                + if f64::from(re) < (word.len() + lower.len()) as f64 * fact {
                    -1000
                } else {
                    0
                };
            by_similarity.insert(Weighted::new(guess.clone(), score));
        }
        by_similarity
    }

    /// Equivalent to `getMostRelevantSuggestions(TreeSet<Weighted<String>>,
    /// Set<Suggestion>)`.
    fn get_most_relevant_suggestions(
        &self,
        by_similarity: &BTreeSet<Weighted<Vec<u16>>>,
        prev_suggestions: &[Suggestion],
    ) -> Result<Vec<Vec<u16>>> {
        let mut result: Vec<Vec<u16>> = Vec::new();
        let mut has_excellent = false;
        for weighted in by_similarity {
            if weighted.score > 1000 {
                has_excellent = true;
            } else if has_excellent {
                break; // leave only excellent suggestions, if any
            }

            let bad = weighted.score < -100;
            // keep the best ngram suggestions, unless in ONLYMAXDIFF mode
            if bad && (!result.is_empty() || self.dictionary.only_max_diff) {
                break;
            }

            if !prev_suggestions
                .iter()
                .any(|s| contains(&weighted.word, &s.raw))
                && !result.iter().any(|r| contains(&weighted.word, r))
                && self.speller.check_word(&weighted.word)?
            {
                result.push(weighted.word.clone());
                if result.len() as i64 >= i64::from(self.dictionary.max_n_gram_suggestions) {
                    break;
                }
            }

            if bad {
                break;
            }
        }
        Ok(result)
    }

    /// The length of the common prefix of `s1` and `s2`.
    ///
    /// Equivalent to `commonPrefix(CharSequence, CharSequence)`.
    pub(crate) fn common_prefix(s1: &[u16], s2: &[u16]) -> usize {
        common_prefix(s1, s2)
    }
}

/// Equivalent to `processAffixIds(int, IntsRef, AffixProcessor)`.
fn process_affix_ids(affix_length: usize, affix_ids: &IntsRef, processor: &mut dyn AffixProcessor) {
    for j in 0..affix_ids.length {
        processor.process_affix(affix_length, affix_ids.ints[affix_ids.offset + j]);
    }
}

/// Equivalent to `isWorseThan(int, CharsRef, Weighted<Root<String>>)`.
fn is_worse_than(score: i32, candidate: &[u16], root: &Weighted<Root>) -> bool {
    score < root.score
        || score == root.score && candidate.cmp(root.word.word.as_slice()) == Ordering::Greater
}

/// Find minimum threshold for a passable suggestion: mangle original word
/// three different ways and score them to generate a minimum acceptable
/// score.
///
/// Equivalent to `calcThreshold(String)`.
fn calc_threshold(word: &[u16]) -> i32 {
    let mut thresh = 0;
    for sp in 1..4 {
        let mut mw = word.to_vec();
        let mut k = sp;
        while k < word.len() {
            mw[k] = u16::from(b'*');
            k += 4;
        }
        thresh += any_mismatch_ngram(word.len() as i32, word, &mw, false);
    }
    thresh / 3 - 1
}

/// Equivalent to `commonPrefix(CharSequence, CharSequence)`.
pub(crate) fn common_prefix(s1: &[u16], s2: &[u16]) -> usize {
    s1.iter().zip(s2).take_while(|(a, b)| a == b).count()
}

/// Generate an n-gram score comparing `s1` and `s2`.
///
/// Equivalent to `ngramScore(int, String, String, boolean)`.
///
/// Java's member is package-private, reached by the same package's tests;
/// Rust's integration tests are another crate, so the port makes it public
/// (hidden from the documentation).
#[doc(hidden)]
pub fn ngram_score(n: i32, s1: &[u16], s2: &[u16], weighted: bool) -> i32 {
    let l1 = s1.len() as i32;
    let mut score = 0;
    let mut last_starts = vec![0i32; s1.len()];
    for j in 1..=n {
        let mut ns = 0;
        let mut i = 0;
        while i <= l1 - j {
            let iu = i as usize;
            if last_starts[iu] >= 0 {
                let pos = index_of_substring(s2, last_starts[iu], s1, iu, j);
                last_starts[iu] = pos;
                if pos >= 0 {
                    ns += 1;
                    i += 1;
                    continue;
                }
            }
            if weighted {
                ns -= 1;
                if i == 0 || i == l1 - j {
                    ns -= 1; // side weight
                }
            }
            i += 1;
        }
        score += ns;
        if ns < 2 && !weighted {
            break;
        }
    }
    score
}

/// NGRAM_LONGER_WORSE flag in Hunspell.
///
/// Equivalent to `longerWorsePenalty(int, int)`.
fn longer_worse_penalty(length1: i32, length2: i32) -> i32 {
    ((length2 - length1) - 2).max(0)
}

/// NGRAM_ANY_MISMATCH flag in Hunspell.
///
/// Equivalent to `anyMismatchNgram(int, String, String, boolean)`.
fn any_mismatch_ngram(n: i32, s1: &[u16], s2: &[u16], weighted: bool) -> i32 {
    ngram_score(n, s1, s2, weighted) - ((s2.len() as i32 - s1.len() as i32).abs() - 2).max(0)
}

/// Equivalent to `indexOfSubstring(String, int, String, int, int)`.
fn index_of_substring(
    haystack: &[u16],
    haystack_pos: i32,
    needle: &[u16],
    needle_pos: usize,
    len: i32,
) -> i32 {
    let c = needle[needle_pos];
    let limit = haystack.len() as i32 - len;
    let mut i = haystack_pos;
    while i <= limit {
        if haystack[i as usize] == c
            && region_matches(
                haystack,
                i64::from(i) + 1,
                needle,
                needle_pos as i64 + 1,
                i64::from(len) - 1,
            )
        {
            return i;
        }
        i += 1;
    }
    -1
}

/// Equivalent to `lcs(String, String)`.
fn lcs(s1: &[u16], s2: &[u16]) -> i32 {
    let mut lengths = vec![0i32; s2.len() + 1];
    for i in 1..=s1.len() {
        let mut prev = 0;
        for j in 1..=s2.len() {
            let cur = lengths[j];
            lengths[j] = if s1[i - 1] == s2[j - 1] {
                prev + 1
            } else {
                cur.max(lengths[j - 1])
            };
            prev = cur;
        }
    }
    lengths[s2.len()]
}

/// Equivalent to `commonCharacterPositionScore(String, String)`.
fn common_character_position_score(s1: &[u16], s2: &[u16]) -> i32 {
    let mut num = 0;
    let mut diff_pos1: Option<usize> = None;
    let mut diff_pos2: Option<usize> = None;
    let mut diff = 0;
    let mut i = 0;
    while i < s1.len() && i < s2.len() {
        if s1[i] == s2[i] {
            num += 1;
        } else {
            if diff == 0 {
                diff_pos1 = Some(i);
            } else if diff == 1 {
                diff_pos2 = Some(i);
            }
            diff += 1;
        }
        i += 1;
    }
    let common_score = i32::from(num > 0);
    if diff == 2 && i == s1.len() && i == s2.len() {
        let (p1, p2) = (
            diff_pos1.expect("INVARIANT: two differences"),
            diff_pos2.expect("INVARIANT: two differences"),
        );
        if s1[p1] == s2[p2] && s1[p2] == s2[p1] {
            return common_score + 10;
        }
    }
    common_score
}

/// An insertion-ordered set of strings (Java's `LinkedHashSet<String>`).
#[derive(Default)]
struct LinkedStrings {
    items: Vec<Vec<u16>>,
    seen: HashSet<Vec<u16>>,
}

impl LinkedStrings {
    fn add(&mut self, item: Vec<u16>) {
        if self.seen.insert(item.clone()) {
            self.items.push(item);
        }
    }
}
