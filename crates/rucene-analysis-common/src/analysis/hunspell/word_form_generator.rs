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

//! The generator of word forms and dictionary entries, ported from
//! `org.apache.lucene.analysis.hunspell.WordFormGenerator`.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::util::fst::{IntSequenceOutputs, IntsRefFSTEnum, FST};
use rucene::util::IntsRef;

use super::affix_condition::{always_true, AffixCondition};
use super::affix_kind::AffixKind;
use super::affixed_word::{Affix, AffixedWord};
use super::dict_entries::DictEntries;
use super::dict_entry::DictEntry;
use super::dictionary::{
    array_index_out_of_bounds, Dictionary, AFFIX_APPEND, AFFIX_FLAG, AFFIX_STRIP_ORD, FLAG_UNSET,
};
use super::entry_suggestion::EntrySuggestion;
use super::flag_enumerator::FlagEnumerator;
use super::flyweight_entry::FlyweightEntry;
use super::generating_suggester::JavaPriorityQueue;
use super::hunspell::CheckCanceled;
use super::stemmer::{StemCandidateProcessor, Stemmer};
use super::word_context::WordContext;
use crate::jdk::string::{to_rust_string, units};

/// The protected method a subclass of `WordFormGenerator` overrides.
///
/// Java's `WordFormGenerator` is subclassed to override
/// `canStemToOriginal(AffixedWord)`; a Rust type has no subclass, so the
/// overriding object is passed to [`WordFormGenerator::with_subclass`]. The
/// generator is passed to it, so that it can call the base implementation
/// ([`WordFormGenerator::default_can_stem_to_original`], Java's `super`).
pub trait WordFormGeneratorSubclass: Send + Sync {
    /// Equivalent to `canStemToOriginal(AffixedWord)`.
    ///
    /// # Errors
    ///
    /// What the check raises.
    fn can_stem_to_original(
        &self,
        generator: &WordFormGenerator,
        derived: &AffixedWord,
    ) -> Result<bool>;
}

/// A utility class used for generating possible word forms by adding affixes
/// to stems ([`get_all_word_forms_with_flags`](Self::get_all_word_forms_with_flags)),
/// and suggesting stems and flags to generate the given set of words
/// ([`compress`](Self::compress)).
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.WordFormGenerator`.
pub struct WordFormGenerator {
    dictionary: Arc<Dictionary>,
    affixes: HashMap<u16, Vec<AffixEntry>>,
    stemmer: Stemmer,
    subclass: Option<Arc<dyn WordFormGeneratorSubclass>>,
}

impl std::fmt::Debug for WordFormGenerator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WordFormGenerator")
            .field("affix_flags", &self.affixes.len())
            .field("subclass", &self.subclass.is_some())
            .finish_non_exhaustive()
    }
}

/// An affix rule with what applying it needs.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.WordFormGenerator$AffixEntry`,
/// a record.
#[derive(Debug, Clone)]
pub(crate) struct AffixEntry {
    id: i32,
    flag: u16,
    kind: AffixKind,
    affix: Vec<u16>,
    strip: Vec<u16>,
    condition: Arc<dyn AffixCondition>,
}

impl AffixEntry {
    /// Equivalent to `apply(AffixedWord, Dictionary)`.
    fn apply(&self, stem: &AffixedWord, dictionary: &Dictionary) -> Option<AffixedWord> {
        let word = stem.word();
        let is_prefix = self.kind == AffixKind::Prefix;
        if !(if is_prefix {
            word.starts_with(&self.strip)
        } else {
            word.ends_with(&self.strip)
        }) {
            return None;
        }

        let stripped = if is_prefix {
            &word[self.strip.len()..]
        } else {
            &word[..word.len() - self.strip.len()]
        };
        if !self.condition.accepts_stem_string(stripped) {
            return None;
        }

        let mut applied = Vec::with_capacity(stripped.len() + self.affix.len());
        if is_prefix {
            applied.extend_from_slice(&self.affix);
            applied.extend_from_slice(stripped);
        } else {
            applied.extend_from_slice(stripped);
            applied.extend_from_slice(&self.affix);
        }
        let mut prefixes = stem.get_prefixes().to_vec();
        let mut suffixes = stem.get_suffixes().to_vec();
        let affix = Affix::new(dictionary, self.id);
        if is_prefix {
            prefixes.insert(0, affix);
        } else {
            suffixes.insert(0, affix);
        }
        Some(AffixedWord::new(
            applied,
            Arc::clone(stem.entry()),
            prefixes,
            suffixes,
        ))
    }
}

/// The `canStemToOriginal` stem candidate processor.
struct OriginalStemProcessor<'a> {
    generator: &'a WordFormGenerator,
    stem: &'a [u16],
    found_stem: bool,
    found_forbidden: bool,
}

impl StemCandidateProcessor for OriginalStemProcessor<'_> {
    fn context(&self) -> WordContext {
        WordContext::SimpleWord
    }

    fn process_stem_candidate(
        &mut self,
        chars: &[u16],
        offset: usize,
        length: usize,
        _last_affix: i32,
        _outer_prefix: i32,
        _inner_prefix: i32,
        _outer_suffix: i32,
        _inner_suffix: i32,
    ) -> Result<bool> {
        if self.generator.is_forbidden_word(chars, offset, length) {
            self.found_forbidden = true;
            return Ok(false);
        }
        self.found_stem |=
            length == self.stem.len() && *self.stem == chars[offset..offset + length];
        Ok(!self.found_stem)
    }
}

impl WordFormGenerator {
    /// Equivalent to `WordFormGenerator(Dictionary)`.
    ///
    /// # Errors
    ///
    /// What enumerating the affix FSTs raises (Java wraps it in an
    /// `UncheckedIOException`).
    pub fn new(dictionary: Arc<Dictionary>) -> Result<Self> {
        Self::create(dictionary, None)
    }

    /// A generator of a subclass of `WordFormGenerator` that overrides the
    /// method of `subclass`.
    ///
    /// Equivalent to `WordFormGenerator(Dictionary)` called by a subclass
    /// constructor.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn with_subclass(
        dictionary: Arc<Dictionary>,
        subclass: Arc<dyn WordFormGeneratorSubclass>,
    ) -> Result<Self> {
        Self::create(dictionary, Some(subclass))
    }

    fn create(
        dictionary: Arc<Dictionary>,
        subclass: Option<Arc<dyn WordFormGeneratorSubclass>>,
    ) -> Result<Self> {
        let mut affixes = HashMap::new();
        fill_affix_map(
            &dictionary,
            &mut affixes,
            dictionary.prefixes.as_ref(),
            AffixKind::Prefix,
        )?;
        fill_affix_map(
            &dictionary,
            &mut affixes,
            dictionary.suffixes.as_ref(),
            AffixKind::Suffix,
        )?;
        let stemmer = Stemmer::new(Arc::clone(&dictionary));
        Ok(Self {
            dictionary,
            affixes,
            stemmer,
            subclass,
        })
    }

    /// Generate all word forms for all dictionary entries with the given
    /// root word. The result order is stable but not specified. This is
    /// equivalent to "unmunch" from the "hunspell-tools" package.
    ///
    /// Equivalent to `getAllWordForms(String, Runnable)`.
    ///
    /// # Errors
    ///
    /// What `check_canceled` raises.
    pub fn get_all_word_forms(
        &self,
        root: &str,
        check_canceled: &CheckCanceled,
    ) -> Result<Vec<AffixedWord>> {
        let root = units(root);
        let mut result = Vec::new();
        if let Some(entries) = self.dictionary.lookup_entries_units(&root) {
            for entry in entries.to_vec() {
                result.extend(self.get_all_word_forms_units(
                    &root,
                    entry.flags(),
                    check_canceled,
                )?);
            }
        }
        Ok(result)
    }

    /// Generate all word forms for the given root pretending it has the given
    /// flags (in the same format as the dictionary uses). The result order is
    /// stable but not specified. This is equivalent to "unmunch" from the
    /// "hunspell-tools" package.
    ///
    /// Equivalent to `getAllWordForms(String, String, Runnable)`.
    ///
    /// # Errors
    ///
    /// What `check_canceled` raises, and what parsing the flags raises.
    pub fn get_all_word_forms_with_flags(
        &self,
        stem: &str,
        flags: &str,
        check_canceled: &CheckCanceled,
    ) -> Result<Vec<AffixedWord>> {
        self.get_all_word_forms_units(&units(stem), &units(flags), check_canceled)
    }

    /// `getAllWordForms(String, String, Runnable)` of UTF-16 units.
    fn get_all_word_forms_units(
        &self,
        stem: &[u16],
        flags: &[u16],
        check_canceled: &CheckCanceled,
    ) -> Result<Vec<AffixedWord>> {
        let encoded_flags = self
            .dictionary
            .flag_parsing_strategy
            .parse_utf_flags(flags)?;
        if !self.should_consider_at_all(&encoded_flags) {
            return Ok(Vec::new());
        }
        self.get_all_word_forms_of_entry(
            Arc::new(DictEntry::create(stem.to_vec(), flags.to_vec())),
            encoded_flags,
            check_canceled,
        )
    }

    /// Equivalent to `getAllWordForms(DictEntry, char[], Runnable)`.
    fn get_all_word_forms_of_entry(
        &self,
        entry: Arc<DictEntry>,
        encoded_flags: Vec<u16>,
        check_canceled: &CheckCanceled,
    ) -> Result<Vec<AffixedWord>> {
        let encoded_flags = sort_and_deduplicate(encoded_flags);
        let mut result = Vec::new();
        let bare = AffixedWord::new(entry.stem().to_vec(), entry, Vec::new(), Vec::new());
        check_canceled()?;
        if !FlagEnumerator::has_flag_in_sorted_array(
            self.dictionary.needaffix,
            &encoded_flags,
            0,
            encoded_flags.len(),
        ) {
            result.push(bare.clone());
        }
        result.extend(self.expand(&bare, &encoded_flags, check_canceled)?);
        Ok(result)
    }

    /// A sanity-check that the word form generated by affixation in
    /// [`get_all_word_forms_with_flags`](Self::get_all_word_forms_with_flags)
    /// is indeed accepted by the spell-checker and analyzed to be the form of
    /// the original dictionary entry. A subclass can override it
    /// ([`WordFormGeneratorSubclass`]) for cases where such check is
    /// unnecessary or can be done more efficiently.
    ///
    /// Equivalent to `canStemToOriginal(AffixedWord)`, as the subclass
    /// overrides it.
    ///
    /// # Errors
    ///
    /// What the check raises.
    pub fn can_stem_to_original(&self, derived: &AffixedWord) -> Result<bool> {
        match &self.subclass {
            Some(subclass) => subclass.can_stem_to_original(self, derived),
            None => self.default_can_stem_to_original(derived),
        }
    }

    /// `WordFormGenerator.canStemToOriginal(AffixedWord)` itself, which a
    /// subclass calls as `super.canStemToOriginal`.
    ///
    /// # Errors
    ///
    /// What reading the affix FSTs raises.
    pub fn default_can_stem_to_original(&self, derived: &AffixedWord) -> Result<bool> {
        let chars = derived.word();
        if self.is_forbidden_word(chars, 0, chars.len()) {
            return Ok(false);
        }

        let stem = derived.get_dict_entry().stem();
        let mut processor = OriginalStemProcessor {
            generator: self,
            stem,
            found_stem: false,
            found_forbidden: false,
        };
        self.stemmer
            .remove_affixes(chars, 0, chars.len(), true, -1, -1, -1, &mut processor)?;
        Ok(processor.found_stem && !processor.found_forbidden)
    }

    /// Equivalent to `isForbiddenWord(char[], int, int)`.
    fn is_forbidden_word(&self, chars: &[u16], offset: usize, length: usize) -> bool {
        let dictionary = &self.dictionary;
        if dictionary.forbiddenword != FLAG_UNSET {
            if let Some(forms) = dictionary.lookup_word(chars, offset, length) {
                let mut i = 0;
                while i < forms.length {
                    if dictionary.has_flag(forms.ints[forms.offset + i], dictionary.forbiddenword) {
                        return true;
                    }
                    i += dictionary.form_step();
                }
            }
        }
        false
    }

    /// Equivalent to `expand(AffixedWord, char[], Runnable)`.
    fn expand(
        &self,
        stem: &AffixedWord,
        flags: &[u16],
        check_canceled: &CheckCanceled,
    ) -> Result<Vec<AffixedWord>> {
        let mut result = Vec::new();
        for &flag in flags {
            let Some(entries) = self.affixes.get(&flag) else {
                continue;
            };
            let kind = entries[0].kind;
            if !self.is_compatible_with_previous_affixes(stem, kind, flag) {
                continue;
            }

            for affix in entries {
                check_canceled()?;
                if let Some(derived) = affix.apply(stem, &self.dictionary) {
                    let append = self.append_flags(affix);
                    if self.should_consider_at_all(&append) {
                        if self.can_stem_to_original(&derived)? {
                            result.push(derived.clone());
                        }
                        if self.dictionary.is_cross_product(affix.id) {
                            let updated = self.update_flags(flags, flag, &append)?;
                            result.extend(self.expand(&derived, &updated, check_canceled)?);
                        }
                    }
                }
            }
        }
        Ok(result)
    }

    /// Equivalent to `shouldConsiderAtAll(char[])`.
    fn should_consider_at_all(&self, flags: &[u16]) -> bool {
        let dictionary = &self.dictionary;
        !flags.iter().any(|&flag| {
            flag == dictionary.compound_begin
                || flag == dictionary.compound_middle
                || flag == dictionary.compound_end
                || flag == dictionary.forbiddenword
                || flag == dictionary.onlyincompound
        })
    }

    /// Equivalent to `updateFlags(char[], char, char[])`: the flags without
    /// `to_remove` and NEEDAFFIX, with `to_append`, in an array of
    /// `flags.length + toAppend.length - 1` units (zeros fill the rest).
    fn update_flags(&self, flags: &[u16], to_remove: u16, to_append: &[u16]) -> Result<Vec<u16>> {
        let size = flags.len() + to_append.len() - 1;
        let mut result = vec![0u16; size];
        let mut index = 0;
        for &flag in flags {
            if flag != to_remove && flag != self.dictionary.needaffix {
                *result
                    .get_mut(index)
                    .ok_or_else(|| array_index_out_of_bounds(index as i64, size))? = flag;
                index += 1;
            }
        }
        for &flag in to_append {
            *result
                .get_mut(index)
                .ok_or_else(|| array_index_out_of_bounds(index as i64, size))? = flag;
            index += 1;
        }
        Ok(sort_and_deduplicate(result))
    }

    /// Equivalent to `appendFlags(AffixEntry)`.
    fn append_flags(&self, affix: &AffixEntry) -> Vec<u16> {
        let append_id = self.dictionary.affix_data(affix.id, AFFIX_APPEND);
        if append_id == 0 {
            Vec::new()
        } else {
            self.dictionary.flag_lookup.get_flags(i32::from(append_id))
        }
    }

    /// Traverse the whole dictionary and derive all word forms via affixation
    /// (as in [`get_all_word_forms_with_flags`](Self::get_all_word_forms_with_flags))
    /// for each of the entries. The iteration order is undefined. Only
    /// "simple" words are returned, no compounding flags are processed.
    /// Upper- and title-case variations are not returned, even if the
    /// spellchecker accepts them.
    ///
    /// Equivalent to `generateAllSimpleWords(Consumer<AffixedWord>,
    /// Runnable)`.
    ///
    /// # Errors
    ///
    /// What `consumer` and `check_canceled` raise.
    pub fn generate_all_simple_words(
        &self,
        consumer: &mut dyn FnMut(AffixedWord) -> Result<()>,
        check_canceled: &CheckCanceled,
    ) -> Result<()> {
        let dictionary = &self.dictionary;
        dictionary
            .words
            .process_all_words(1, i32::MAX, false, &mut |e: &mut dyn FlyweightEntry| {
                let root_str = e.root().to_vec();
                let forms = e.forms().to_vec();
                let mut i = 0;
                while i < forms.len() {
                    let encoded_flags = dictionary.flag_lookup.get_flags(forms[i]);
                    if self.should_consider_at_all(&encoded_flags) {
                        let presentable_flags =
                            dictionary.flag_parsing_strategy.print_flags(&encoded_flags);
                        let entry =
                            Arc::new(DictEntry::create(root_str.clone(), presentable_flags));
                        for aw in
                            self.get_all_word_forms_of_entry(entry, encoded_flags, check_canceled)?
                        {
                            consumer(aw)?;
                        }
                    }
                    i += dictionary.form_step();
                }
                Ok(())
            })
    }

    /// Given a list of words, try to produce a smaller set of dictionary
    /// entries (with some flags) that would generate these words. This is
    /// equivalent to "munch" from the "hunspell-tools" package. The algorithm
    /// tries to minimize the number of the dictionary entries to add or
    /// change, the number of flags involved, and the number of non-requested
    /// additionally generated words. All the mentioned words are in the
    /// dictionary format and case: no ICONV/OCONV/IGNORE conversions are
    /// applied.
    ///
    /// `words` is the list of words to generate, `forbidden` the set of words
    /// to avoid generating. Returns the information about suggested
    /// dictionary entries and overgenerated words, or `None` if the
    /// algorithm couldn't generate anything.
    ///
    /// Equivalent to `compress(List<String>, Set<String>, Runnable)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("'words' and 'forbidden' shouldn't
    /// intersect")`, and what `check_canceled` raises.
    pub fn compress(
        &self,
        words: &[&str],
        forbidden: &HashSet<String>,
        check_canceled: &CheckCanceled,
    ) -> Result<Option<EntrySuggestion>> {
        if words.is_empty() {
            return Ok(None);
        }
        if words.iter().any(|word| forbidden.contains(*word)) {
            return Err(LuceneError::illegal_argument(
                "'words' and 'forbidden' shouldn't intersect",
            ));
        }

        let words: Vec<Vec<u16>> = words.iter().map(|word| units(word)).collect();
        let forbidden: HashSet<Vec<u16>> = forbidden.iter().map(|word| units(word)).collect();
        WordCompressor::new(self, &words, forbidden, check_canceled)?.compress()
    }

    /// Equivalent to `isCompatibleWithPreviousAffixes(AffixedWord, AffixKind,
    /// char)`.
    fn is_compatible_with_previous_affixes(
        &self,
        stem: &AffixedWord,
        kind: AffixKind,
        flag: u16,
    ) -> bool {
        let is_prefix = kind == AffixKind::Prefix;
        let same_affixes = if is_prefix {
            stem.get_prefixes()
        } else {
            stem.get_suffixes()
        };
        let size = same_affixes.len();
        if size == 2 {
            return false;
        }
        if is_prefix && size == 1 && !self.dictionary.complex_prefixes {
            return false;
        }
        if !is_prefix && !stem.get_prefixes().is_empty() {
            return false;
        }
        if size == 1
            && !self
                .dictionary
                .is_flag_appended_by_affix(same_affixes[0].affix_id, flag)
        {
            return false;
        }
        true
    }
}

/// Equivalent to `fillAffixMap(FST<IntsRef>, AffixKind)`.
fn fill_affix_map(
    dictionary: &Dictionary,
    affixes: &mut HashMap<u16, Vec<AffixEntry>>,
    fst: Option<&FST<IntSequenceOutputs>>,
    kind: AffixKind,
) -> Result<()> {
    let Some(fst) = fst else {
        return Ok(());
    };
    let mut fst_enum = IntsRefFSTEnum::new(fst)?;
    while let Some(io) = fst_enum.next()? {
        let affix_ids: &IntsRef = io.output;
        let affix = affix_to_string(kind, io.input);
        for j in 0..affix_ids.length {
            let id = affix_ids.ints[affix_ids.offset + j];
            let flag = dictionary.affix_data(id, AFFIX_FLAG);
            let entry = AffixEntry {
                id,
                flag,
                kind,
                affix: affix.clone(),
                strip: strip(dictionary, id),
                condition: condition(dictionary, id),
            };
            affixes.entry(flag).or_default().push(entry);
        }
    }
    Ok(())
}

/// Equivalent to `toString(AffixKind, IntsRef)`: the labels as units,
/// reversed for a suffix.
fn affix_to_string(kind: AffixKind, input: &IntsRef) -> Vec<u16> {
    let mut affix_chars = vec![0u16; input.length];
    let len = affix_chars.len();
    for i in 0..len {
        let index = if kind == AffixKind::Prefix {
            i
        } else {
            len - i - 1
        };
        affix_chars[index] = input.ints[input.offset + i] as u16;
    }
    affix_chars
}

/// Equivalent to `condition(int)`.
fn condition(dictionary: &Dictionary, affix_id: i32) -> Arc<dyn AffixCondition> {
    let condition = dictionary.get_affix_condition(affix_id);
    if condition == 0 {
        always_true()
    } else {
        Arc::clone(
            dictionary.patterns[condition]
                .as_ref()
                .expect("INVARIANT: only pattern 0 is null"),
        )
    }
}

/// Equivalent to `strip(int)`.
fn strip(dictionary: &Dictionary, affix_id: i32) -> Vec<u16> {
    let strip_ord = usize::from(dictionary.affix_data(affix_id, AFFIX_STRIP_ORD));
    let strip_start = dictionary.strip_offsets[strip_ord] as usize;
    let strip_end = dictionary.strip_offsets[strip_ord + 1] as usize;
    dictionary.strip_data[strip_start..strip_end].to_vec()
}

/// Equivalent to `sortAndDeduplicate(char[])`.
fn sort_and_deduplicate(mut flags: Vec<u16>) -> Vec<u16> {
    flags.sort_unstable();
    flags.dedup();
    flags
}

/// A set of affix flags.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.WordFormGenerator$FlagSet`,
/// a record of a `CharHashSet` (held sorted) and the dictionary (the same for
/// every flag set of a generator).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct FlagSet {
    flags: Vec<u16>,
}

impl FlagSet {
    /// Equivalent to `FlagSet(CharHashSet, Dictionary)`.
    fn new(flags: Vec<u16>) -> Self {
        Self {
            flags: sort_and_deduplicate(flags),
        }
    }

    /// Equivalent to `flatten(Set<FlagSet>)`: the union of the flags.
    fn flatten(flag_sets: &[FlagSet]) -> Vec<u16> {
        sort_and_deduplicate(
            flag_sets
                .iter()
                .flat_map(|f| f.flags.iter().copied())
                .collect(),
        )
    }

    /// Equivalent to `toString()`.
    pub(crate) fn to_java_string(&self, dictionary: &Dictionary) -> String {
        to_rust_string(&dictionary.flag_parsing_strategy.print_flags(&self.flags))
    }
}

/// A stem and its flag sets: `Map<String, Set<FlagSet>>` entries in
/// insertion order.
type StemToFlags = Vec<(Vec<u16>, Vec<FlagSet>)>;

/// The order-free form of a [`StemToFlags`], for `Map.equals`/`Set.equals`.
fn canonical(stem_to_flags: &StemToFlags) -> Vec<(Vec<u16>, Vec<FlagSet>)> {
    let mut canonical: Vec<(Vec<u16>, Vec<FlagSet>)> = stem_to_flags
        .iter()
        .map(|(stem, flags)| {
            let mut flags = flags.clone();
            flags.sort();
            (stem.clone(), flags)
        })
        .collect();
    canonical.sort();
    canonical
}

/// A candidate solution.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.WordFormGenerator$State`,
/// a record; `potential_coverage` is the maximum number of requested forms
/// possibly generated by adding only flags to this state.
#[derive(Debug, Clone)]
pub(crate) struct State {
    stem_to_flags: StemToFlags,
    under_generated: i32,
    over_generated: i32,
    potential_coverage: i32,
}

/// A stem with its flag sets, the key of the expansion cache.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.WordFormGenerator$WordCompressor$StemWithFlags`,
/// a record whose `Set<FlagSet>` is held sorted, as set equality compares
/// it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct StemWithFlags {
    stem: Vec<u16>,
    flags: Vec<FlagSet>,
}

impl StemWithFlags {
    fn new(stem: Vec<u16>, flags: &[FlagSet]) -> Self {
        let mut flags = flags.to_vec();
        flags.sort();
        flags.dedup();
        Self { stem, flags }
    }
}

/// The search for the entries that generate a list of words.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.WordFormGenerator$WordCompressor`.
pub(crate) struct WordCompressor<'a> {
    generator: &'a WordFormGenerator,
    forbidden: HashSet<Vec<u16>>,
    check_canceled: &'a CheckCanceled,
    word_set: HashSet<Vec<u16>>,
    existing_stems: HashSet<Vec<u16>>,
    stem_to_possible_flags: HashMap<Vec<u16>, Vec<FlagSet>>,
    /// `stemsToForms` (a `LinkedHashMap`): the stems in insertion order.
    stems: Vec<Vec<u16>>,
    stems_to_forms: HashMap<Vec<u16>, Vec<Vec<u16>>>,
    expansion_cache: HashMap<StemWithFlags, Vec<Vec<u16>>>,
}

/// The stem candidate processor of the `WordCompressor` constructor.
struct CompressorProcessor<'c, 'a> {
    compressor: &'c mut WordCompressor<'a>,
    word: &'c [u16],
}

impl StemCandidateProcessor for CompressorProcessor<'_, '_> {
    fn context(&self) -> WordContext {
        WordContext::SimpleWord
    }

    fn process_stem_candidate(
        &mut self,
        chars: &[u16],
        offset: usize,
        length: usize,
        _last_affix: i32,
        outer_prefix: i32,
        inner_prefix: i32,
        outer_suffix: i32,
        inner_suffix: i32,
    ) -> Result<bool> {
        let candidate = chars[offset..offset + length].to_vec();
        let dictionary = &self.compressor.generator.dictionary;
        let mut flags = Vec::new();
        for affix in [outer_prefix, inner_prefix, outer_suffix, inner_suffix] {
            if affix >= 0 {
                flags.push(dictionary.affix_data(affix, AFFIX_FLAG));
            }
        }
        let flag_set = FlagSet::new(flags);
        let swf = StemWithFlags::new(candidate.clone(), std::slice::from_ref(&flag_set));
        let allowed = self.compressor.forbidden.is_empty()
            || !self
                .compressor
                .all_generated_of(&swf)?
                .iter()
                .any(|word| self.compressor.forbidden.contains(word));
        if allowed {
            self.compressor.register_stem(&candidate, self.word);
            let possible = self
                .compressor
                .stem_to_possible_flags
                .entry(candidate)
                .or_default();
            if !possible.contains(&flag_set) {
                possible.push(flag_set);
            }
        }
        Ok(true)
    }
}

impl<'a> WordCompressor<'a> {
    /// Equivalent to `WordCompressor(List<String>, Set<String>, Runnable)`.
    fn new(
        generator: &'a WordFormGenerator,
        words: &[Vec<u16>],
        forbidden: HashSet<Vec<u16>>,
        check_canceled: &'a CheckCanceled,
    ) -> Result<Self> {
        let mut compressor = Self {
            generator,
            forbidden,
            check_canceled,
            word_set: words.iter().cloned().collect(),
            existing_stems: HashSet::new(),
            stem_to_possible_flags: HashMap::new(),
            stems: Vec::new(),
            stems_to_forms: HashMap::new(),
            expansion_cache: HashMap::new(),
        };
        for word in words {
            check_canceled()?;
            compressor
                .stem_to_possible_flags
                .entry(word.clone())
                .or_default();
            compressor.register_stem(word, word);
            let mut processor = CompressorProcessor {
                compressor: &mut compressor,
                word,
            };
            generator.stemmer.remove_affixes(
                word,
                0,
                word.len(),
                true,
                -1,
                -1,
                -1,
                &mut processor,
            )?;
        }

        compressor.existing_stems = compressor
            .stems
            .iter()
            .filter(|stem| generator.dictionary.lookup_entries_units(stem).is_some())
            .cloned()
            .collect();
        Ok(compressor)
    }

    /// `registerStem(String)` of the processor: `word` is a form of `stem`.
    fn register_stem(&mut self, stem: &[u16], word: &[u16]) {
        let forms = match self.stems_to_forms.get_mut(stem) {
            Some(forms) => forms,
            None => {
                self.stems.push(stem.to_vec());
                self.stems_to_forms.entry(stem.to_vec()).or_default()
            }
        };
        if !forms.iter().any(|form| form == word) {
            forms.push(word.to_vec());
        }
    }

    /// Equivalent to `compress()`.
    fn compress(&mut self) -> Result<Option<EntrySuggestion>> {
        let existing_stems = &self.existing_stems;
        let stems_to_forms = &self.stems_to_forms;
        let mut sorted_stems = self.stems.clone();
        // `Comparator.comparing(existingStems::contains).thenComparing(size).reversed()`
        sorted_stems.sort_by(|a, b| {
            let key = |s: &Vec<u16>| (existing_stems.contains(s), stems_to_forms[s].len());
            key(b).cmp(&key(a))
        });

        let mut queue = JavaPriorityQueue::new(solution_fitness);
        let mut visited: HashSet<Vec<(Vec<u16>, Vec<FlagSet>)>> = HashSet::new();
        queue.offer(State {
            stem_to_flags: Vec::new(),
            under_generated: self.word_set.len() as i32,
            over_generated: 0,
            potential_coverage: 0,
        });
        let mut result = None;
        while let Some(state) = queue.poll() {
            if state.under_generated == 0 {
                result = Some(state);
                break;
            }

            for stem in &sorted_stems {
                if !state.stem_to_flags.iter().any(|(s, _)| s == stem) {
                    let with_stem = add_stem(&state, stem);
                    if visited.insert(canonical(&with_stem)) {
                        if let Some(next) = self.new_state(with_stem)? {
                            if state.under_generated > next.under_generated
                                || next.potential_coverage > state.potential_coverage
                            {
                                queue.offer(next);
                            }
                        }
                    }
                }
            }

            if (state.potential_coverage as usize) < self.word_set.len() {
                // don't add flags until the suggested entries can potentially cover all requested forms
                continue;
            }

            for (stem, flag_sets) in &state.stem_to_flags {
                let possible = self.stem_to_possible_flags[stem].clone();
                for flags in possible {
                    if !flag_sets.contains(&flags) {
                        let with_flags = add_flags(&state, stem, flags);
                        if visited.insert(canonical(&with_flags)) {
                            if let Some(next) = self.new_state(with_flags)? {
                                if state.under_generated > next.under_generated {
                                    queue.offer(next);
                                }
                            }
                        }
                    }
                }
            }
        }
        match result {
            None => Ok(None),
            Some(state) => Ok(Some(self.to_suggestion(&state)?)),
        }
    }

    /// Equivalent to `toSuggestion(State)`; it fills the expansion cache,
    /// hence `&mut self`.
    #[allow(clippy::wrong_self_convention)]
    fn to_suggestion(&mut self, state: &State) -> Result<EntrySuggestion> {
        let mut to_edit = Vec::new();
        let mut to_add = Vec::new();
        for (stem, flag_sets) in &state.stem_to_flags {
            self.add_entry(
                &mut to_edit,
                &mut to_add,
                stem,
                &FlagSet::flatten(flag_sets),
            );
        }

        let mut extra_generated = Vec::new();
        let mut all: Vec<Vec<u16>> = self.all_generated(&state.stem_to_flags)?;
        all.sort();
        all.dedup();
        let forbiddenword = self.generator.dictionary.forbiddenword;
        for extra in all {
            if self.word_set.contains(&extra) || self.existing_stems.contains(&extra) {
                continue;
            }

            if self.forbidden.contains(&extra) && forbiddenword != FLAG_UNSET {
                self.add_entry(&mut to_edit, &mut to_add, &extra, &[forbiddenword]);
            } else {
                extra_generated.push(to_rust_string(&extra));
            }
        }

        Ok(EntrySuggestion::new(to_edit, to_add, extra_generated))
    }

    /// Equivalent to `addEntry(List<DictEntry>, List<DictEntry>, String,
    /// CharHashSet)`.
    fn add_entry(
        &self,
        to_edit: &mut Vec<DictEntry>,
        to_add: &mut Vec<DictEntry>,
        stem: &[u16],
        flags: &[u16],
    ) {
        let flag_string = self.to_flag_string(flags);
        let entry = DictEntry::create(stem.to_vec(), flag_string);
        if self.existing_stems.contains(stem) {
            to_edit.push(entry);
        } else {
            to_add.push(entry);
        }
    }

    /// Equivalent to `newState(Map<String, Set<FlagSet>>)`.
    fn new_state(&mut self, stem_to_flags: StemToFlags) -> Result<Option<State>> {
        let all_generated: HashSet<Vec<u16>> =
            self.all_generated(&stem_to_flags)?.into_iter().collect();
        let mut over_generated = 0;
        for s in &all_generated {
            if self.forbidden.contains(s) {
                return Ok(None);
            }
            if !self.word_set.contains(s) {
                over_generated += 1;
            }
        }
        let mut covered: HashSet<&Vec<u16>> = HashSet::new();
        for (stem, _) in &stem_to_flags {
            covered.extend(self.stems_to_forms[stem].iter());
        }
        let potential_coverage = covered.len() as i32;
        let under_generated = self
            .word_set
            .iter()
            .filter(|s| !all_generated.contains(*s))
            .count() as i32;
        Ok(Some(State {
            stem_to_flags,
            under_generated,
            over_generated,
            potential_coverage,
        }))
    }

    /// Equivalent to `allGenerated(StemWithFlags)`, cached.
    fn all_generated_of(&mut self, swc: &StemWithFlags) -> Result<Vec<Vec<u16>>> {
        if let Some(cached) = self.expansion_cache.get(swc) {
            return Ok(cached.clone());
        }
        let words: Vec<Vec<u16>> = self
            .expand(&swc.stem, &FlagSet::flatten(&swc.flags))?
            .iter()
            .map(|w| w.word().to_vec())
            .collect();
        self.expansion_cache.insert(swc.clone(), words.clone());
        Ok(words)
    }

    /// Equivalent to `allGenerated(Map<String, Set<FlagSet>>)`.
    fn all_generated(&mut self, stem_to_flags: &StemToFlags) -> Result<Vec<Vec<u16>>> {
        let mut all = Vec::new();
        for (stem, flags) in stem_to_flags {
            all.extend(self.all_generated_of(&StemWithFlags::new(stem.clone(), flags))?);
        }
        Ok(all)
    }

    /// Equivalent to `expand(String, CharHashSet)`.
    fn expand(&self, stem: &[u16], flag_set: &[u16]) -> Result<Vec<AffixedWord>> {
        self.generator.get_all_word_forms_units(
            stem,
            &self.to_flag_string(flag_set),
            self.check_canceled,
        )
    }

    /// Equivalent to `toFlagString(CharHashSet)`.
    fn to_flag_string(&self, flag_set: &[u16]) -> Vec<u16> {
        let mut sorted = flag_set.to_vec();
        sorted.sort_unstable();
        self.generator
            .dictionary
            .flag_parsing_strategy
            .print_flags(&sorted)
    }
}

/// Equivalent to `WordCompressor.solutionFitness`.
fn solution_fitness(a: &State, b: &State) -> Ordering {
    (-a.potential_coverage)
        .cmp(&-b.potential_coverage)
        .then_with(|| a.stem_to_flags.len().cmp(&b.stem_to_flags.len()))
        .then_with(|| a.under_generated.cmp(&b.under_generated))
        .then_with(|| a.over_generated.cmp(&b.over_generated))
}

/// Equivalent to `addStem(State, String)`.
fn add_stem(state: &State, stem: &[u16]) -> StemToFlags {
    let mut stem_to_flags = state.stem_to_flags.clone();
    stem_to_flags.push((stem.to_vec(), Vec::new()));
    stem_to_flags
}

/// Equivalent to `addFlags(State, String, FlagSet)`: the stem keeps its
/// position, as `LinkedHashMap.put` keeps an existing key's.
fn add_flags(state: &State, stem: &[u16], flags: FlagSet) -> StemToFlags {
    let mut stem_to_flags = state.stem_to_flags.clone();
    if let Some((_, flag_sets)) = stem_to_flags.iter_mut().find(|(s, _)| s == stem) {
        if !flag_sets.contains(&flags) {
            flag_sets.push(flags);
        }
    }
    stem_to_flags
}
