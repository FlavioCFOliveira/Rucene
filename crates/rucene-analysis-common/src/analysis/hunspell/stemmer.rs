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

//! The affix-stripping stemmer, ported from
//! `org.apache.lucene.analysis.hunspell.Stemmer`.

use std::sync::Arc;

use rucene::analysis::CharArraySet;
use rucene::error::Result;
use rucene::util::fst::{Arc as FSTArc, Outputs};
use rucene::util::{CharsRef, IntsRef};

use super::dictionary::{Dictionary, AFFIX_APPEND, AFFIX_FLAG, AFFIX_STRIP_ORD};
use super::word_case::WordCase;
use super::word_context::WordContext;
use crate::jdk::character::{is_lower_case, to_upper_case_unit};
use crate::jdk::string::{index_of, starts_with_str, units};

/// What [`Stemmer::strip_affix`] returns: no stem, the same `char[]` (the
/// affix has no strip data and can simply be removed), or a new one.
enum Stripped {
    None,
    Pure,
    New(Vec<u16>),
}

/// Stemmer uses the affix rules declared in the Dictionary to generate one or
/// more stems for a word. It conforms to the algorithm in the original
/// hunspell algorithm, including recursive suffix stripping.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.Stemmer`.
#[derive(Debug, Clone)]
pub(crate) struct Stemmer {
    dictionary: Arc<Dictionary>,
    // it's '1' if we have no stem exceptions, otherwise every other form
    // is really an ID pointing to the exception table
    form_step: usize,
}

/// A visitor of the case variants of a word.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.Stemmer$CaseVariationProcessor`.
pub(crate) trait CaseVariationProcessor {
    /// Equivalent to `process(char[], int, WordCase)`: whether the
    /// processing should be continued.
    ///
    /// # Errors
    ///
    /// What the processor raises.
    fn process(
        &mut self,
        word: &[u16],
        length: usize,
        original_case: Option<WordCase>,
    ) -> Result<bool>;
}

/// A visitor of the roots found for a word.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.Stemmer$RootProcessor`.
pub(crate) trait RootProcessor {
    /// `stem` is the text of the found dictionary entry, `form_id` the
    /// internal id of the dictionary entry, `morph_data_id` the id of the
    /// custom morphological data (0 if none), and the affix ids those of the
    /// outer and inner prefix and suffix applied to the stem, or -1 if none.
    /// Returns whether the processing should be continued.
    ///
    /// Equivalent to `processRoot(CharsRef, int, int, int, int, int, int)`.
    ///
    /// # Errors
    ///
    /// What the processor raises.
    #[allow(clippy::too_many_arguments)]
    fn process_root(
        &mut self,
        stem: &[u16],
        form_id: i32,
        morph_data_id: i32,
        outer_prefix: i32,
        inner_prefix: i32,
        outer_suffix: i32,
        inner_suffix: i32,
    ) -> Result<bool>;
}

/// A visitor of the stem candidates affix removal produces.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.Stemmer$StemCandidateProcessor`,
/// an abstract class holding the [`WordContext`] it was created with.
pub(crate) trait StemCandidateProcessor {
    /// The field `context`.
    fn context(&self) -> WordContext;

    /// Equivalent to `processStemCandidate(char[], int, int, int, int, int,
    /// int, int)`: whether the processing should be continued.
    ///
    /// # Errors
    ///
    /// What the processor raises.
    #[allow(clippy::too_many_arguments)]
    fn process_stem_candidate(
        &mut self,
        word: &[u16],
        offset: usize,
        length: usize,
        last_affix: i32,
        outer_prefix: i32,
        inner_prefix: i32,
        outer_suffix: i32,
        inner_suffix: i32,
    ) -> Result<bool>;
}

/// The stem candidate processor of `doStem`: the candidates found in the
/// dictionary with the flag of the last affix become roots.
struct DoStemProcessor<'a> {
    stemmer: &'a Stemmer,
    context: WordContext,
    processor: &'a mut dyn RootProcessor,
}

impl StemCandidateProcessor for DoStemProcessor<'_> {
    fn context(&self) -> WordContext {
        self.context
    }

    fn process_stem_candidate(
        &mut self,
        word: &[u16],
        offset: usize,
        length: usize,
        last_affix: i32,
        outer_prefix: i32,
        inner_prefix: i32,
        outer_suffix: i32,
        inner_suffix: i32,
    ) -> Result<bool> {
        let dictionary = &self.stemmer.dictionary;
        let Some(forms) = dictionary.lookup_word(word, offset, length) else {
            return Ok(true);
        };
        let flag = dictionary.affix_data(last_affix, AFFIX_FLAG);
        let prefix_id = if inner_prefix >= 0 {
            inner_prefix
        } else {
            outer_prefix
        };
        let mut i = 0;
        while i < forms.length {
            let entry_id = forms.ints[forms.offset + i];
            if dictionary.has_flag(entry_id, flag)
                || dictionary.is_flag_appended_by_affix(prefix_id, flag)
            {
                if inner_prefix < 0 && outer_prefix >= 0 {
                    let prefix_flag = dictionary.affix_data(outer_prefix, AFFIX_FLAG);
                    if !dictionary.has_flag(entry_id, prefix_flag)
                        && !dictionary.is_flag_appended_by_affix(last_affix, prefix_flag)
                    {
                        i += self.stemmer.form_step;
                        continue;
                    }
                }
                if !self
                    .stemmer
                    .is_root_compatible_with_context(self.context, last_affix, entry_id)
                {
                    i += self.stemmer.form_step;
                    continue;
                }
                if !self.processor.process_root(
                    &word[offset..offset + length],
                    entry_id,
                    self.stemmer.morph_data_id(&forms, i),
                    outer_prefix,
                    inner_prefix,
                    outer_suffix,
                    inner_suffix,
                )? {
                    return Ok(false);
                }
            }
            i += self.stemmer.form_step;
        }
        Ok(true)
    }
}

/// The root processor of `stem`: each root becomes a stem.
struct StemCollector<'a> {
    stemmer: &'a Stemmer,
    list: Vec<CharsRef>,
}

impl RootProcessor for StemCollector<'_> {
    fn process_root(
        &mut self,
        stem: &[u16],
        _form_id: i32,
        morph_data_id: i32,
        _outer_prefix: i32,
        _inner_prefix: i32,
        _outer_suffix: i32,
        _inner_suffix: i32,
    ) -> Result<bool> {
        self.list.push(self.stemmer.new_stem(stem, morph_data_id)?);
        Ok(true)
    }
}

/// The case variation processor of `analyze`: each variant is stemmed as a
/// simple word.
struct AnalyzeVariations<'a> {
    stemmer: &'a Stemmer,
    processor: &'a mut dyn RootProcessor,
}

impl CaseVariationProcessor for AnalyzeVariations<'_> {
    fn process(
        &mut self,
        variant: &[u16],
        var_length: usize,
        _original_case: Option<WordCase>,
    ) -> Result<bool> {
        self.stemmer.do_stem(
            variant,
            0,
            var_length,
            WordContext::SimpleWord,
            self.processor,
        )
    }
}

impl Stemmer {
    /// Constructs a new Stemmer which will use the provided Dictionary to
    /// create its stems.
    ///
    /// Equivalent to `Stemmer(Dictionary)`.
    pub(crate) fn new(dictionary: Arc<Dictionary>) -> Self {
        let form_step = dictionary.form_step();
        Self {
            dictionary,
            form_step,
        }
    }

    /// The dictionary.
    pub(crate) fn dictionary(&self) -> &Arc<Dictionary> {
        &self.dictionary
    }

    /// Find the stem(s) of the provided word.
    ///
    /// Equivalent to `stem(String)` and `stem(char[], int)`.
    ///
    /// # Errors
    ///
    /// What reading the affix FSTs raises.
    pub(crate) fn stem(&self, word: &[u16], length: usize) -> Result<Vec<CharsRef>> {
        let mut collector = StemCollector {
            stemmer: self,
            list: Vec::new(),
        };
        self.analyze(word, length, &mut collector)?;
        Ok(collector.list)
    }

    /// Equivalent to `analyze(char[], int, RootProcessor)`.
    ///
    /// # Errors
    ///
    /// What the processor raises.
    pub(crate) fn analyze(
        &self,
        word: &[u16],
        length: usize,
        processor: &mut dyn RootProcessor,
    ) -> Result<()> {
        let mut word = word;
        let mut length = length;
        let scratch_buffer;
        if self.dictionary.may_need_input_cleaning() {
            let scratch_segment = &word[..length];
            if self.dictionary.needs_input_cleaning(scratch_segment) {
                let mut segment = Vec::new();
                self.dictionary.clean_input(scratch_segment, &mut segment)?;
                length = segment.len();
                scratch_buffer = segment;
                word = &scratch_buffer;
            }
        }

        if length == 0 {
            return Ok(());
        }

        if !self.do_stem(word, 0, length, WordContext::SimpleWord, processor)? {
            return Ok(());
        }

        let word_case = self.case_of(word, length);
        if word_case == WordCase::Upper || word_case == WordCase::Title {
            let mut variation_processor = AnalyzeVariations {
                stemmer: self,
                processor,
            };
            self.vary_case(word, length, word_case, &mut variation_processor)?;
        }
        Ok(())
    }

    /// Equivalent to `varyCase(char[], int, WordCase, CaseVariationProcessor)`.
    ///
    /// # Errors
    ///
    /// What the processor raises.
    pub(crate) fn vary_case(
        &self,
        word: &[u16],
        length: usize,
        word_case: WordCase,
        processor: &mut dyn CaseVariationProcessor,
    ) -> Result<bool> {
        let title_buffer = if word_case == WordCase::Upper {
            Some(self.case_fold_title(word, length))
        } else {
            None
        };
        if let Some(title_buffer) = &title_buffer {
            if let Some(apos_case) = capitalize_after_apostrophe(title_buffer, length) {
                if !processor.process(&apos_case, length, Some(word_case))? {
                    return Ok(false);
                }
            }
            if !processor.process(title_buffer, length, Some(word_case))? {
                return Ok(false);
            }
            if self.dictionary.check_sharp_s
                && !self.vary_sharp_s(title_buffer, length, processor)?
            {
                return Ok(false);
            }
        }

        if self.dictionary.is_dot_i_case_change_disallowed(word) {
            return Ok(true);
        }

        let lower_buffer = self.case_fold_lower(title_buffer.as_deref().unwrap_or(word), length);
        if !processor.process(&lower_buffer, length, Some(word_case))? {
            return Ok(false);
        }
        if word_case == WordCase::Upper
            && self.dictionary.check_sharp_s
            && !self.vary_sharp_s(&lower_buffer, length, processor)?
        {
            return Ok(false);
        }
        Ok(true)
    }

    /// Returns EXACT_CASE,TITLE_CASE, or UPPER_CASE type for the word.
    ///
    /// Equivalent to `caseOf(char[], int)`.
    pub(crate) fn case_of(&self, word: &[u16], length: usize) -> WordCase {
        if self.dictionary.ignore_case || length == 0 || is_lower_case(i32::from(word[0])) {
            return WordCase::Mixed;
        }
        WordCase::case_of_length(word, length)
    }

    /// Folds titlecase variant of word to titleBuffer.
    ///
    /// Equivalent to `caseFoldTitle(char[], int)`.
    fn case_fold_title(&self, word: &[u16], length: usize) -> Vec<u16> {
        let mut title_buffer = word[..length].to_vec();
        for c in title_buffer.iter_mut().skip(1) {
            *c = self.dictionary.case_fold(*c);
        }
        title_buffer
    }

    /// Folds lowercase variant of word (title cased) to lowerBuffer.
    ///
    /// Equivalent to `caseFoldLower(char[], int)`.
    fn case_fold_lower(&self, word: &[u16], length: usize) -> Vec<u16> {
        let mut lower_buffer = word[..length].to_vec();
        lower_buffer[0] = self.dictionary.case_fold(lower_buffer[0]);
        lower_buffer
    }

    /// Equivalent to `varySharpS(char[], int, CaseVariationProcessor)`.
    fn vary_sharp_s(
        &self,
        word: &[u16],
        length: usize,
        processor: &mut dyn CaseVariationProcessor,
    ) -> Result<bool> {
        let Some(result) = replace_ss(word, length, 0, 0) else {
            return Ok(true);
        };
        let src = &word[..length];
        for s in result {
            if s != src && !processor.process(&s, s.len(), None)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Equivalent to `doStem(char[], int, int, WordContext, RootProcessor)`.
    ///
    /// # Errors
    ///
    /// What the processor raises.
    pub(crate) fn do_stem(
        &self,
        word: &[u16],
        offset: usize,
        length: usize,
        context: WordContext,
        processor: &mut dyn RootProcessor,
    ) -> Result<bool> {
        let dictionary = &self.dictionary;
        if let Some(forms) = dictionary.lookup_word(word, offset, length) {
            let mut i = 0;
            while i < forms.length {
                let entry_id = forms.ints[forms.offset + i];
                // we can't add this form, it's a pseudostem requiring an affix
                if dictionary.has_flag(entry_id, dictionary.needaffix) {
                    i += self.form_step;
                    continue;
                }
                if (context == WordContext::CompoundBegin || context == WordContext::CompoundMiddle)
                    && dictionary.has_flag(entry_id, dictionary.compound_forbid)
                {
                    return Ok(false);
                }
                if !self.is_root_compatible_with_context(context, -1, entry_id) {
                    i += self.form_step;
                    continue;
                }
                if !processor.process_root(
                    &word[offset..offset + length],
                    entry_id,
                    self.morph_data_id(&forms, i),
                    -1,
                    -1,
                    -1,
                    -1,
                )? {
                    return Ok(false);
                }
                i += self.form_step;
            }
        }
        let mut stem_processor = DoStemProcessor {
            stemmer: self,
            context,
            processor,
        };
        self.remove_affixes(word, offset, length, true, -1, -1, -1, &mut stem_processor)
    }

    /// Find the unique stem(s) of the provided word.
    ///
    /// Equivalent to `uniqueStems(char[], int)`.
    ///
    /// # Errors
    ///
    /// What reading the affix FSTs raises.
    pub(crate) fn unique_stems(&self, word: &[u16], length: usize) -> Result<Vec<CharsRef>> {
        let stems = self.stem(word, length)?;
        if stems.len() < 2 {
            return Ok(stems);
        }
        let mut terms = CharArraySet::new(8, self.dictionary.ignore_case);
        let mut deduped = Vec::new();
        for s in stems {
            let chars = s.chars();
            if !terms.contains_chars(Some(chars), 0, chars.len()) {
                let mut copy = chars.to_vec();
                let len = copy.len();
                terms.add_chars(&mut copy, 0, len);
                deduped.push(s);
            }
        }
        Ok(deduped)
    }

    /// Equivalent to `stemException(int)`.
    fn stem_exception(&self, morph_data_id: i32) -> Option<Vec<u16>> {
        if morph_data_id > 0 {
            let data = &self.dictionary.morph_data[morph_data_id as usize];
            let start = if starts_with_str(data, "st:") {
                Some(0)
            } else {
                index_of(data, &units(" st:"), 0)
            };
            if let Some(start) = start {
                let next_space =
                    crate::jdk::string::index_of_unit(data, u16::from(b' '), start + 3);
                return Some(data[start + 3..next_space.unwrap_or(data.len())].to_vec());
            }
        }
        None
    }

    /// Equivalent to `newStem(CharsRef, int)`.
    fn new_stem(&self, stem: &[u16], morph_data_id: i32) -> Result<CharsRef> {
        let exception = self.stem_exception(morph_data_id);
        if let Some(oconv) = &self.dictionary.oconv {
            let mut scratch_segment = match exception {
                Some(exception) => exception,
                None => stem.to_vec(),
            };
            oconv.apply_mappings(&mut scratch_segment)?;
            Ok(CharsRef::from_chars(scratch_segment))
        } else {
            Ok(CharsRef::from_chars(
                exception.unwrap_or_else(|| stem.to_vec()),
            ))
        }
    }

    /// Generates a list of stems for the provided word. It's called
    /// recursively when applying affixes one by one, setting
    /// `(inner/outer)(Suffix/Prefix)` parameters to non-negative values as
    /// that happens. Returns whether the processing should be continued.
    ///
    /// Equivalent to `removeAffixes(char[], int, int, boolean, int, int, int,
    /// StemCandidateProcessor)`.
    ///
    /// # Errors
    ///
    /// What the processor raises, and what reading the affix FSTs raises.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn remove_affixes(
        &self,
        word: &[u16],
        offset: usize,
        length: usize,
        do_prefix: bool,
        outer_prefix: i32,
        inner_prefix: i32,
        outer_suffix: i32,
        processor: &mut dyn StemCandidateProcessor,
    ) -> Result<bool> {
        let dictionary = &self.dictionary;
        let mut arc: FSTArc<IntsRef> = FSTArc::default();
        if do_prefix {
            if let Some(fst) = &dictionary.prefixes {
                let mut reader = fst.get_bytes_reader()?;
                fst.get_first_arc(&mut arc);
                let mut output = fst.outputs().no_output();
                let limit = if dictionary.full_strip {
                    length + 1
                } else {
                    length
                };
                for i in 0..limit {
                    if i > 0 {
                        match Dictionary::next_arc(
                            fst,
                            &mut arc,
                            reader.as_mut(),
                            &output,
                            i32::from(word[offset + i - 1]),
                        )? {
                            None => break,
                            Some(next) => output = next,
                        }
                    }
                    if !arc.is_final() {
                        continue;
                    }
                    let prefixes = fst.outputs().add(&output, arc.next_final_output());
                    for j in 0..prefixes.length {
                        let prefix = prefixes.ints[prefixes.offset + j];
                        if prefix == outer_prefix {
                            continue;
                        }
                        if self.is_affix_compatible(
                            prefix,
                            true,
                            outer_prefix,
                            outer_suffix,
                            processor.context(),
                        ) {
                            let continued =
                                match self.strip_affix(word, offset, length, i, prefix, true) {
                                    Stripped::None => continue,
                                    Stripped::Pure => self.apply_affix(
                                        word,
                                        offset + i,
                                        length - i,
                                        prefix,
                                        true,
                                        outer_prefix,
                                        inner_prefix,
                                        outer_suffix,
                                        processor,
                                    )?,
                                    Stripped::New(stripped_word) => self.apply_affix(
                                        &stripped_word,
                                        0,
                                        stripped_word.len(),
                                        prefix,
                                        true,
                                        outer_prefix,
                                        inner_prefix,
                                        outer_suffix,
                                        processor,
                                    )?,
                                };
                            if !continued {
                                return Ok(false);
                            }
                        }
                    }
                }
            }
        }

        if let Some(fst) = &dictionary.suffixes {
            let mut reader = fst.get_bytes_reader()?;
            fst.get_first_arc(&mut arc);
            let mut output = fst.outputs().no_output();
            let limit = if dictionary.full_strip { 0 } else { 1 };
            let mut i = length as i64;
            while i >= limit {
                let iu = i as usize;
                if iu < length {
                    match Dictionary::next_arc(
                        fst,
                        &mut arc,
                        reader.as_mut(),
                        &output,
                        i32::from(word[offset + iu]),
                    )? {
                        None => break,
                        Some(next) => output = next,
                    }
                }
                if !arc.is_final() {
                    i -= 1;
                    continue;
                }
                let suffixes = fst.outputs().add(&output, arc.next_final_output());
                for j in 0..suffixes.length {
                    let suffix = suffixes.ints[suffixes.offset + j];
                    if suffix == outer_suffix {
                        continue;
                    }
                    if self.is_affix_compatible(
                        suffix,
                        false,
                        outer_prefix,
                        outer_suffix,
                        processor.context(),
                    ) {
                        let continued = match self.strip_affix(
                            word,
                            offset,
                            length,
                            length - iu,
                            suffix,
                            false,
                        ) {
                            Stripped::None => continue,
                            Stripped::Pure => self.apply_affix(
                                word,
                                offset,
                                iu,
                                suffix,
                                false,
                                outer_prefix,
                                inner_prefix,
                                outer_suffix,
                                processor,
                            )?,
                            Stripped::New(stripped_word) => self.apply_affix(
                                &stripped_word,
                                0,
                                stripped_word.len(),
                                suffix,
                                false,
                                outer_prefix,
                                inner_prefix,
                                outer_suffix,
                                processor,
                            )?,
                        };
                        if !continued {
                            return Ok(false);
                        }
                    }
                }
                i -= 1;
            }
        }

        Ok(true)
    }

    /// `None` if affix conditions isn't met; the same word if the affix has
    /// no strip data and can thus be simply removed, or a new word containing
    /// the word affix removal.
    ///
    /// Equivalent to `stripAffix(char[], int, int, int, int, boolean)`.
    fn strip_affix(
        &self,
        word: &[u16],
        offset: usize,
        length: usize,
        affix_len: usize,
        affix: i32,
        is_prefix: bool,
    ) -> Stripped {
        let dictionary = &self.dictionary;
        let de_affixed_len = length - affix_len;

        let strip_ord = usize::from(dictionary.affix_data(affix, AFFIX_STRIP_ORD));
        let strip_start = dictionary.strip_offsets[strip_ord] as usize;
        let strip_end = dictionary.strip_offsets[strip_ord + 1] as usize;
        let strip_len = strip_end - strip_start;

        if strip_len + de_affixed_len == 0 {
            return Stripped::None;
        }

        let strip_data = &dictionary.strip_data;
        let condition = dictionary.get_affix_condition(affix);
        if condition != 0 {
            let de_affixed_offset = if is_prefix {
                offset + affix_len
            } else {
                offset
            };
            let accepts = dictionary.patterns[condition]
                .as_ref()
                .expect("INVARIANT: only pattern 0 is null")
                .accepts_stem(word, de_affixed_offset, de_affixed_len);
            if !accepts {
                return Stripped::None;
            }
        }

        if strip_len == 0 {
            return Stripped::Pure;
        }

        let mut stripped_word = vec![0u16; strip_len + de_affixed_len];
        let src_start = offset + if is_prefix { affix_len } else { 0 };
        let dst_start = if is_prefix { strip_len } else { 0 };
        stripped_word[dst_start..dst_start + de_affixed_len]
            .copy_from_slice(&word[src_start..src_start + de_affixed_len]);
        let strip_dst = if is_prefix { 0 } else { de_affixed_len };
        stripped_word[strip_dst..strip_dst + strip_len]
            .copy_from_slice(&strip_data[strip_start..strip_end]);
        Stripped::New(stripped_word)
    }

    /// Equivalent to `isAffixCompatible(int, boolean, int, int, WordContext)`.
    fn is_affix_compatible(
        &self,
        affix: i32,
        is_prefix: bool,
        outer_prefix: i32,
        outer_suffix: i32,
        context: WordContext,
    ) -> bool {
        let dictionary = &self.dictionary;
        let append = i32::from(dictionary.affix_data(affix, AFFIX_APPEND));

        let previous_was_prefix = outer_suffix < 0 && outer_prefix >= 0;
        if context.is_compound() {
            if !is_prefix && dictionary.has_flag(append, dictionary.compound_forbid) {
                return false;
            }
            if !context.is_affix_allowed_without_special_permit(is_prefix)
                && !dictionary.has_flag(append, dictionary.compound_permit)
            {
                return false;
            }
            if context == WordContext::CompoundEnd
                && !is_prefix
                && !previous_was_prefix
                && dictionary.has_flag(append, dictionary.onlyincompound)
            {
                return false;
            }
        } else if dictionary.has_flag(append, dictionary.onlyincompound) {
            return false;
        }

        if outer_prefix == -1 && outer_suffix == -1 {
            return true;
        }

        if dictionary.is_cross_product(affix) {
            // cross-check incoming continuation class (flag of previous affix) against this affix's flags
            if previous_was_prefix {
                return true;
            }
            if outer_suffix >= 0 {
                let prev_flag = dictionary.affix_data(outer_suffix, AFFIX_FLAG);
                return dictionary.has_flag(append, prev_flag);
            }
        }

        false
    }

    /// Applies the affix rule to the given word, producing a list of stems if
    /// any are found. Non-negative `(inner/outer)(Suffix/Prefix)` parameters
    /// indicate the already applied affixes. `word` holds the word with the
    /// affix removed and the strip added, from `offset` for `length` units;
    /// `affix` is the id of the affix and `prefix` true if we are removing a
    /// prefix. Returns whether the processing should be continued.
    ///
    /// Equivalent to `applyAffix(char[], int, int, int, boolean, int, int,
    /// int, StemCandidateProcessor)`.
    #[allow(clippy::too_many_arguments)]
    fn apply_affix(
        &self,
        word: &[u16],
        offset: usize,
        length: usize,
        affix: i32,
        prefix: bool,
        outer_prefix: i32,
        inner_prefix: i32,
        outer_suffix: i32,
        processor: &mut dyn StemCandidateProcessor,
    ) -> Result<bool> {
        let dictionary = &self.dictionary;
        let (mut outer_prefix, mut inner_prefix, mut outer_suffix) =
            (outer_prefix, inner_prefix, outer_suffix);
        let prefix_id = if inner_prefix >= 0 {
            inner_prefix
        } else {
            outer_prefix
        };
        let previous_affix = if outer_suffix >= 0 {
            outer_suffix
        } else {
            prefix_id
        };

        let mut inner_suffix = -1;
        if prefix {
            if outer_prefix < 0 {
                outer_prefix = affix;
            } else {
                inner_prefix = affix;
            }
        } else if outer_suffix < 0 {
            outer_suffix = affix;
        } else {
            inner_suffix = affix;
        }

        let skip_lookup = self.needs_another_affix(affix, previous_affix, !prefix, prefix_id);
        if !skip_lookup
            && !processor.process_stem_candidate(
                word,
                offset,
                length,
                affix,
                outer_prefix,
                inner_prefix,
                outer_suffix,
                inner_suffix,
            )?
        {
            return Ok(false);
        }

        if inner_suffix >= 0 {
            return Ok(true);
        }

        let recursion_depth = i32::from(outer_suffix >= 0)
            + if inner_prefix >= 0 {
                2
            } else {
                i32::from(outer_prefix >= 0)
            }
            - 1;
        if dictionary.is_cross_product(affix) && recursion_depth <= 1 {
            let flag = dictionary.affix_data(affix, AFFIX_FLAG);
            let do_prefix;
            if recursion_depth == 0 {
                if prefix {
                    do_prefix =
                        dictionary.complex_prefixes && dictionary.is_second_stage_prefix(flag);
                    // we took away the first prefix.
                    // COMPLEXPREFIXES = true:  combine with a second prefix and another suffix
                    // COMPLEXPREFIXES = false: combine with a suffix
                } else if !dictionary.complex_prefixes && dictionary.is_second_stage_suffix(flag) {
                    do_prefix = false;
                    // we took away a suffix.
                    // COMPLEXPREFIXES = true: we don't recurse! only one suffix allowed
                    // COMPLEXPREFIXES = false: combine with another suffix
                } else {
                    return Ok(true);
                }
            } else if prefix && dictionary.complex_prefixes {
                do_prefix = true;
                // we took away the second prefix: go look for another suffix
            } else if prefix
                || dictionary.complex_prefixes
                || !dictionary.is_second_stage_suffix(flag)
            {
                return Ok(true);
            } else {
                // we took away a prefix, then a suffix: go look for another suffix
                do_prefix = false;
            }

            return self.remove_affixes(
                word,
                offset,
                length,
                do_prefix,
                outer_prefix,
                inner_prefix,
                outer_suffix,
                processor,
            );
        }

        Ok(true)
    }

    /// Equivalent to `isRootCompatibleWithContext(WordContext, int, int)`.
    fn is_root_compatible_with_context(
        &self,
        context: WordContext,
        last_affix: i32,
        entry_id: i32,
    ) -> bool {
        let dictionary = &self.dictionary;
        if !context.is_compound() && dictionary.has_flag(entry_id, dictionary.onlyincompound) {
            return false;
        }
        if context.is_compound() && context != WordContext::CompoundRuleEnd {
            let c_flag = context.required_flag(dictionary);
            return dictionary.has_flag(entry_id, c_flag)
                || dictionary.is_flag_appended_by_affix(last_affix, c_flag)
                || dictionary.has_flag(entry_id, dictionary.compound_flag)
                || dictionary.is_flag_appended_by_affix(last_affix, dictionary.compound_flag);
        }
        true
    }

    /// Equivalent to `morphDataId(IntsRef, int)`.
    fn morph_data_id(&self, forms: &IntsRef, i: usize) -> i32 {
        if self.dictionary.has_custom_morph_data {
            forms.ints[forms.offset + i + 1]
        } else {
            0
        }
    }

    /// Equivalent to `needsAnotherAffix(int, int, boolean, int)`.
    fn needs_another_affix(
        &self,
        affix: i32,
        previous_affix: i32,
        is_suffix: bool,
        prefix_id: i32,
    ) -> bool {
        let dictionary = &self.dictionary;
        let circumfix = dictionary.circumfix;
        // if circumfix was previously set by a prefix, we must check this suffix,
        // to ensure it has it, and vice versa
        if is_suffix
            && dictionary.is_flag_appended_by_affix(prefix_id, circumfix)
                != dictionary.is_flag_appended_by_affix(affix, circumfix)
        {
            return true;
        }
        if dictionary.is_flag_appended_by_affix(affix, dictionary.needaffix) {
            return !is_suffix
                || previous_affix < 0
                || dictionary.is_flag_appended_by_affix(previous_affix, dictionary.needaffix);
        }
        false
    }
}

/// Special prefix handling for Catalan, French, Italian: prefixes separated
/// by apostrophe (SANT'ELIA -> Sant'+Elia).
///
/// Equivalent to `capitalizeAfterApostrophe(char[], int)`.
fn capitalize_after_apostrophe(word: &[u16], length: usize) -> Option<Vec<u16>> {
    for i in 1..length.saturating_sub(1) {
        if word[i] == u16::from(b'\'') {
            let next = word[i + 1];
            let upper = to_upper_case_unit(next);
            if upper != next {
                let mut copy = word[..length].to_vec();
                copy[i + 1] = to_upper_case_unit(upper);
                return Some(copy);
            }
        }
    }
    None
}

/// `findSS(int)` of the anonymous object of `varySharpS`.
fn find_ss(word: &[u16], length: usize, start: usize) -> Option<usize> {
    (start..length.saturating_sub(1))
        .find(|&i| word[i] == u16::from(b's') && word[i + 1] == u16::from(b's'))
}

/// `replaceSS(int, int)` of the anonymous object of `varySharpS`: every
/// spelling of `word[start..length]` with each `ss` kept or made `ß`, or
/// `None` when it has no `ss`.
fn replace_ss(word: &[u16], length: usize, start: usize, depth: usize) -> Option<Vec<Vec<u16>>> {
    if depth > 5 {
        // cut off too large enumeration
        return Some(vec![word[start..length].to_vec()]);
    }
    let ss = find_ss(word, length, start)?;
    let prefix = &word[start..ss];
    let tails = replace_ss(word, length, ss + 2, depth + 1)
        .unwrap_or_else(|| vec![word[ss + 2..length].to_vec()]);
    let mut result = Vec::with_capacity(tails.len() * 2);
    for s in tails {
        let mut with_ss = prefix.to_vec();
        with_ss.extend_from_slice(&units("ss"));
        with_ss.extend_from_slice(&s);
        result.push(with_ss);
        let mut with_sharp_s = prefix.to_vec();
        with_sharp_s.push(0x00DF);
        with_sharp_s.extend_from_slice(&s);
        result.push(with_sharp_s);
    }
    Some(result)
}
