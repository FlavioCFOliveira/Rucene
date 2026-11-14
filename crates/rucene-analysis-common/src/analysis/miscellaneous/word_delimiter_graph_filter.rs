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
//! Splits words into subwords and performs optional transformations on
//! subword groups, producing a correct token graph, ported from
//! `org.apache.lucene.analysis.miscellaneous.WordDelimiterGraphFilter`.

use std::sync::Arc;

use rucene::analysis::{
    CharArraySet, CharTermAttribute, KeywordAttribute, OffsetAttribute, PositionIncrementAttribute,
    PositionLengthAttribute, TokenFilter, TokenFilterLogic, TokenStream,
};
use rucene::error::{LuceneError, Result};
use rucene::util::attribute::{AttributeSource, CapturedState};
use rucene::util::selector::PivotOps;
use rucene::util::sorter::{InPlaceMergeSorter, Sorter};
use rucene::util::ArrayUtil;

use super::word_delimiter_iterator::{
    is_alpha, is_digit, WordDelimiterIterator, DEFAULT_WORD_DELIM_TABLE, DONE,
};
use crate::jdk::character::{chars_to_utf16, utf16_to_term_chars};

/// Causes parts of words to be generated: "PowerShot" => "Power" "Shot"
///
/// Equivalent to `WordDelimiterGraphFilter.GENERATE_WORD_PARTS`.
pub const GENERATE_WORD_PARTS: i32 = 1;

/// Causes number subwords to be generated: "500-42" => "500" "42"
///
/// Equivalent to `WordDelimiterGraphFilter.GENERATE_NUMBER_PARTS`.
pub const GENERATE_NUMBER_PARTS: i32 = 2;

/// Causes maximum runs of word parts to be catenated: "wi-fi" => "wifi"
///
/// Equivalent to `WordDelimiterGraphFilter.CATENATE_WORDS`.
pub const CATENATE_WORDS: i32 = 4;

/// Causes maximum runs of number parts to be catenated: "500-42" => "50042"
///
/// Equivalent to `WordDelimiterGraphFilter.CATENATE_NUMBERS`.
pub const CATENATE_NUMBERS: i32 = 8;

/// Causes all subword parts to be catenated: "wi-fi-4000" => "wifi4000"
///
/// Equivalent to `WordDelimiterGraphFilter.CATENATE_ALL`.
pub const CATENATE_ALL: i32 = 16;

/// Causes original words are preserved and added to the subword list
/// (Defaults to false): "500-42" => "500" "42" "500-42"
///
/// Equivalent to `WordDelimiterGraphFilter.PRESERVE_ORIGINAL`.
pub const PRESERVE_ORIGINAL: i32 = 32;

/// Causes lowercase -> uppercase transition to start a new subword.
///
/// Equivalent to `WordDelimiterGraphFilter.SPLIT_ON_CASE_CHANGE`.
pub const SPLIT_ON_CASE_CHANGE: i32 = 64;

/// If not set, causes numeric changes to be ignored (subwords will only be
/// generated given SUBWORD_DELIM tokens).
///
/// Equivalent to `WordDelimiterGraphFilter.SPLIT_ON_NUMERICS`.
pub const SPLIT_ON_NUMERICS: i32 = 128;

/// Causes trailing "'s" to be removed for each subword: "O'Neil's" => "O",
/// "Neil"
///
/// Equivalent to `WordDelimiterGraphFilter.STEM_ENGLISH_POSSESSIVE`.
pub const STEM_ENGLISH_POSSESSIVE: i32 = 256;

/// Suppresses processing terms with `KeywordAttribute.isKeyword()`=true.
///
/// Equivalent to `WordDelimiterGraphFilter.IGNORE_KEYWORDS`.
pub const IGNORE_KEYWORDS: i32 = 512;

/// `RamUsageEstimator.NUM_BYTES_OBJECT_REF`.
const NUM_BYTES_OBJECT_REF: usize = 4;

/// Which of the filter's two concatenations an operation works on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Which {
    /// `concat`.
    Concat,
    /// `concatAll`.
    ConcatAll,
}

/// A WDF concatenated 'run'.
///
/// Equivalent to the inner class `org.apache.lucene.analysis.miscellaneous.WordDelimiterGraphFilter$WordDelimiterConcatenation`;
/// its `write()`, which buffers into the enclosing filter, is the filter's
/// `write_concatenation`. The buffer holds UTF-16 units, as Java's
/// `StringBuilder` does.
#[derive(Clone, Debug, Default)]
pub struct WordDelimiterConcatenation {
    buffer: Vec<u16>,
    start_part: i32,
    end_part: i32,
    start_pos: i32,
    type_: u8,
    subword_count: i32,
}

impl WordDelimiterConcatenation {
    /// Appends the given text of the given length, to the concetenation at
    /// the given offset.
    ///
    /// Equivalent to `WordDelimiterConcatenation.append(char[], int, int)`.
    fn append(&mut self, text: &[u16], offset: usize, length: usize) {
        self.buffer
            .extend_from_slice(&text[offset..offset + length]);
        self.subword_count += 1;
    }

    /// Determines if the concatenation is empty.
    ///
    /// Equivalent to `WordDelimiterConcatenation.isEmpty()`.
    fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Equivalent to `WordDelimiterConcatenation.isNotEmpty()`.
    fn is_not_empty(&self) -> bool {
        !self.is_empty()
    }

    /// Clears the concatenation and resets its state.
    ///
    /// Equivalent to `WordDelimiterConcatenation.clear()`.
    fn clear(&mut self) {
        self.buffer.clear();
        self.start_part = 0;
        self.end_part = 0;
        self.type_ = 0;
        self.subword_count = 0;
    }
}

/// Sorts the buffered parts by start part, then by descending end part.
///
/// Equivalent to the private inner class `org.apache.lucene.analysis.miscellaneous.WordDelimiterGraphFilter$PositionSorter`,
/// an `InPlaceMergeSorter` over the filter's `bufferedParts` and
/// `bufferedTermParts`.
#[derive(Debug)]
pub struct PositionSorter<'a> {
    buffered_parts: &'a mut [i32],
    buffered_term_parts: &'a mut [Option<Vec<u16>>],
    pivot_index: usize,
}

impl PivotOps for PositionSorter<'_> {
    /// Equivalent to `PositionSorter.swap(int, int)`.
    fn swap(&mut self, i: usize, j: usize) {
        let i_offset = 4 * i;
        let j_offset = 4 * j;
        for x in 0..4 {
            self.buffered_parts.swap(i_offset + x, j_offset + x);
        }
        self.buffered_term_parts.swap(i, j);
    }

    fn set_pivot(&mut self, i: usize) {
        self.pivot_index = i;
    }

    fn compare_pivot(&mut self, j: usize) -> i32 {
        self.compare(self.pivot_index, j)
    }

    /// Equivalent to `PositionSorter.compare(int, int)`.
    fn compare(&mut self, i: usize, j: usize) -> i32 {
        // smaller start offset
        let mut i_off = self.buffered_parts[4 * i + 2];
        let mut j_off = self.buffered_parts[4 * j + 2];
        let cmp = i_off.cmp(&j_off) as i32;
        if cmp != 0 {
            return cmp;
        }

        // longer end offset
        i_off = self.buffered_parts[4 * i + 3];
        j_off = self.buffered_parts[4 * j + 3];
        j_off.cmp(&i_off) as i32
    }
}

impl Sorter for PositionSorter<'_> {
    fn sort(&mut self, from: usize, to: usize) {
        self.in_place_merge_sort(from, to);
    }
}

/// The body of [`WordDelimiterGraphFilter`]: what Lucene's
/// `WordDelimiterGraphFilter` adds to `TokenFilter`.
///
/// The term is read as its UTF-16 units, as Java's `char[]` is: positions,
/// lengths and offsets are computed on them, and every term the filter
/// writes is a slice of them, which never cuts a surrogate pair.
#[derive(Debug)]
pub struct WordDelimiterGraphFilterLogic {
    prot_words: Option<CharArraySet>,
    flags: i32,

    /// packs start pos, end pos, start part, end part (= slice of the term
    /// text) for each buffered part:
    buffered_parts: Vec<i32>,
    buffered_len: usize,
    buffered_pos: usize,

    /// holds text for each buffered part, or null if it's a simple slice of
    /// the original term
    buffered_term_parts: Vec<Option<Vec<u16>>>,

    /// used for iterating word delimiter breaks
    iterator: WordDelimiterIterator,

    /// used for concatenating runs of similar typed subwords (word,number)
    concat: WordDelimiterConcatenation,

    adjust_internal_offsets: bool,

    /// number of subwords last output by concat.
    last_concat_count: i32,

    /// used for catenate all
    concat_all: WordDelimiterConcatenation,

    /// used for accumulating position increment gaps so that we preserve
    /// incoming holes:
    accum_pos_inc: i32,

    saved_term_buffer: Vec<u16>,
    saved_term_length: usize,
    saved_start_offset: i32,
    saved_end_offset: i32,
    saved_state: Option<CapturedState>,
    last_start_offset: i32,
    adjusting_offsets: bool,

    word_pos: i32,
}

impl WordDelimiterGraphFilterLogic {
    /// Iterates all words parts and concatenations, buffering up the term
    /// parts we should return.
    ///
    /// Equivalent to the private `WordDelimiterGraphFilter.bufferWordParts()`.
    fn buffer_word_parts(&mut self, source: &AttributeSource) {
        self.save_state(source);

        // if length by start + end offsets doesn't match the term's text then set offsets for
        // all our word parts/concats to the incoming offsets.  this can happen if WDGF is
        // applied to an injected synonym, or to a stem'd form, etc:
        self.adjusting_offsets = self.adjust_internal_offsets
            && (self.saved_end_offset - self.saved_start_offset) as i64
                == self.saved_term_length as i64;

        self.buffered_len = 0;
        self.last_concat_count = 0;
        self.word_pos = 0;

        if self.has(PRESERVE_ORIGINAL) {
            // add the original token now so that it is always emitted first
            // we will edit the term length after all other parts have been buffered
            self.buffer(None, 0, 1, 0, self.saved_term_length as i32);
        }

        if self.iterator.is_single_word() {
            let (current, end) = (self.iterator.current, self.iterator.end);
            self.buffer(None, self.word_pos, self.word_pos + 1, current, end);
            self.word_pos += 1;
            self.iterator.next();
        } else {
            // iterate all words parts, possibly buffering them, building up concatenations and
            // possibly buffering them too:
            while self.iterator.end != DONE {
                let word_type = self.iterator.type_();

                // do we already have queued up incompatible concatenations?
                if self.concat.is_not_empty() && (self.concat.type_ & word_type) == 0 {
                    self.flush_concatenation(Which::Concat);
                }

                // add subwords depending upon options
                if self.should_concatenate(word_type) {
                    self.concatenate(Which::Concat);
                }

                // add all subwords (catenateAll)
                if self.has(CATENATE_ALL) {
                    self.concatenate(Which::ConcatAll);
                }

                // if we should output the word or number part
                if self.should_generate_parts(word_type) {
                    let (current, end) = (self.iterator.current, self.iterator.end);
                    self.buffer(None, self.word_pos, self.word_pos + 1, current, end);
                    self.word_pos += 1;
                }
                self.iterator.next();
            }

            if self.concat.is_not_empty() {
                // flush final concatenation
                self.flush_concatenation(Which::Concat);
            }

            if self.concat_all.is_not_empty() {
                // only if we haven't output this same combo above, e.g. PowerShot with
                // CATENATE_WORDS:
                if self.concat_all.subword_count > self.last_concat_count {
                    if self.word_pos == self.concat_all.start_pos {
                        // we are not generating parts, so we must advance wordPos now
                        self.word_pos += 1;
                    }
                    self.write_concatenation(Which::ConcatAll);
                }
                self.concat_all.clear();
            }
        }

        if self.has(PRESERVE_ORIGINAL) {
            // we now know how many tokens need to be injected, so we can set the original
            // token's position length
            if self.word_pos == 0 {
                // can happen w/ strange flag combos and inputs :)
                self.word_pos += 1;
            }
            self.buffered_parts[1] = self.word_pos;
        }

        let from = if self.has(PRESERVE_ORIGINAL) { 1 } else { 0 };
        let len = self.buffered_len;
        PositionSorter {
            buffered_parts: &mut self.buffered_parts,
            buffered_term_parts: &mut self.buffered_term_parts,
            pivot_index: 0,
        }
        .sort(from, len);
        self.word_pos = 0;

        // set back to 0 for iterating from the buffer
        self.buffered_pos = 0;
    }

    /// Equivalent to the package-private `WordDelimiterGraphFilter.buffer(char[],
    /// int, int, int, int)` (and `buffer(int, int, int, int)`, with a `null`
    /// term part).
    fn buffer(
        &mut self,
        term_part: Option<Vec<u16>>,
        start_pos: i32,
        end_pos: i32,
        start_part: i32,
        end_part: i32,
    ) {
        debug_assert!(end_pos > start_pos, "startPos={start_pos} endPos={end_pos}");
        debug_assert!(
            end_part > start_part
                || (end_part == 0 && start_part == 0 && self.saved_term_length == 0),
            "startPart={start_part} endPart={end_part}"
        );
        if (self.buffered_len + 1) * 4 > self.buffered_parts.len() {
            let new_len = ArrayUtil::oversize((self.buffered_len + 1) * 4, 4);
            self.buffered_parts.resize(new_len, 0);
        }
        if self.buffered_term_parts.len() == self.buffered_len {
            let new_size = ArrayUtil::oversize(self.buffered_len + 1, NUM_BYTES_OBJECT_REF);
            self.buffered_term_parts.resize(new_size, None);
        }
        self.buffered_term_parts[self.buffered_len] = term_part;
        self.buffered_parts[self.buffered_len * 4] = start_pos;
        self.buffered_parts[self.buffered_len * 4 + 1] = end_pos;
        self.buffered_parts[self.buffered_len * 4 + 2] = start_part;
        self.buffered_parts[self.buffered_len * 4 + 3] = end_part;
        self.buffered_len += 1;
    }

    /// Saves the existing attribute states.
    ///
    /// Equivalent to the private `WordDelimiterGraphFilter.saveState()`.
    fn save_state(&mut self, source: &AttributeSource) {
        let term = {
            let term_att = source
                .get_attribute_interface::<dyn CharTermAttribute>()
                .expect("INVARIANT: the constructor added the CharTermAttribute");
            chars_to_utf16(&term_att.buffer()[..term_att.length()])
        };
        self.saved_term_length = term.len();
        {
            let offset_att = offset_att(source);
            self.saved_start_offset = offset_att.start_offset();
            self.saved_end_offset = offset_att.end_offset();
        }
        self.saved_state = source.capture_state();
        self.saved_term_buffer = term;
    }

    /// Flushes the given WordDelimiterConcatenation by either writing its
    /// concat and then clearing, or just clearing.
    ///
    /// Equivalent to the private `WordDelimiterGraphFilter.flushConcatenation(WordDelimiterConcatenation)`.
    fn flush_concatenation(&mut self, which: Which) {
        let (start_pos, subword_count, type_) = {
            let concat = self.concatenation(which);
            (concat.start_pos, concat.subword_count, concat.type_)
        };
        if self.word_pos == start_pos {
            // we are not generating parts, so we must advance wordPos now
            self.word_pos += 1;
        }
        self.last_concat_count = subword_count;
        if subword_count != 1 || !self.should_generate_parts(type_) {
            self.write_concatenation(which);
        }
        self.concatenation_mut(which).clear();
    }

    /// Determines whether to concatenate a word or number if the current
    /// word is the given type.
    ///
    /// Equivalent to the private `WordDelimiterGraphFilter.shouldConcatenate(int)`.
    fn should_concatenate(&self, word_type: u8) -> bool {
        (self.has(CATENATE_WORDS) && is_alpha(word_type))
            || (self.has(CATENATE_NUMBERS) && is_digit(word_type))
    }

    /// Determines whether a word/number part should be generated for a word
    /// of the given type.
    ///
    /// Equivalent to the private `WordDelimiterGraphFilter.shouldGenerateParts(int)`.
    fn should_generate_parts(&self, word_type: u8) -> bool {
        (self.has(GENERATE_WORD_PARTS) && is_alpha(word_type))
            || (self.has(GENERATE_NUMBER_PARTS) && is_digit(word_type))
    }

    /// Concatenates the saved buffer to the given WordDelimiterConcatenation.
    ///
    /// Equivalent to the private `WordDelimiterGraphFilter.concatenate(WordDelimiterConcatenation)`.
    fn concatenate(&mut self, which: Which) {
        let type_ = self.iterator.type_();
        let current = self.iterator.current;
        let end = self.iterator.end;
        let word_pos = self.word_pos;
        let concatenation = match which {
            Which::Concat => &mut self.concat,
            Which::ConcatAll => &mut self.concat_all,
        };
        if concatenation.is_empty() {
            concatenation.type_ = type_;
            concatenation.start_part = current;
            concatenation.start_pos = word_pos;
        }
        concatenation.append(
            &self.saved_term_buffer,
            current as usize,
            (end - current) as usize,
        );
        concatenation.end_part = end;
    }

    /// Determines whether the given flag is set.
    ///
    /// Equivalent to the private `WordDelimiterGraphFilter.has(int)`.
    fn has(&self, flag: i32) -> bool {
        (self.flags & flag) != 0
    }

    fn concatenation(&self, which: Which) -> &WordDelimiterConcatenation {
        match which {
            Which::Concat => &self.concat,
            Which::ConcatAll => &self.concat_all,
        }
    }

    fn concatenation_mut(&mut self, which: Which) -> &mut WordDelimiterConcatenation {
        match which {
            Which::Concat => &mut self.concat,
            Which::ConcatAll => &mut self.concat_all,
        }
    }

    /// Writes the concatenation to part buffer.
    ///
    /// Equivalent to `WordDelimiterConcatenation.write()`: `buffer(termPart,
    /// startPos, wordPos, startPart, endPart)`.
    fn write_concatenation(&mut self, which: Which) {
        let (term_part, start_pos, start_part, end_part) = {
            let concat = self.concatenation(which);
            (
                concat.buffer.clone(),
                concat.start_pos,
                concat.start_part,
                concat.end_part,
            )
        };
        self.buffer(
            Some(term_part),
            start_pos,
            self.word_pos,
            start_part,
            end_part,
        );
    }
}

fn offset_att(
    source: &AttributeSource,
) -> rucene::util::atomic_ref_cell::AtomicRefMut<'_, dyn OffsetAttribute> {
    source
        .get_attribute_interface_mut::<dyn OffsetAttribute>()
        .expect("INVARIANT: the constructor added the OffsetAttribute")
}

fn pos_inc_att(
    source: &AttributeSource,
) -> rucene::util::atomic_ref_cell::AtomicRefMut<'_, dyn PositionIncrementAttribute> {
    source
        .get_attribute_interface_mut::<dyn PositionIncrementAttribute>()
        .expect("INVARIANT: the constructor added the PositionIncrementAttribute")
}

impl TokenFilterLogic for WordDelimiterGraphFilterLogic {
    /// Equivalent to `WordDelimiterGraphFilter.incrementToken()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        loop {
            if self.saved_state.is_none() {
                // process a new input token
                if !input.increment_token()? {
                    return Ok(false);
                }
                if self.has(IGNORE_KEYWORDS)
                    && source
                        .get_attribute_interface::<dyn KeywordAttribute>()
                        .expect("INVARIANT: the constructor added the KeywordAttribute")
                        .is_keyword()
                {
                    return Ok(true);
                }
                let term_buffer = {
                    let term_att = source
                        .get_attribute_interface::<dyn CharTermAttribute>()
                        .expect("INVARIANT: the constructor added the CharTermAttribute");
                    chars_to_utf16(&term_att.buffer()[..term_att.length()])
                };
                let term_length = term_buffer.len();

                self.accum_pos_inc += pos_inc_att(source).get_position_increment();

                // iterate & cache all word parts up front:
                self.iterator.set_text(&term_buffer, term_length);
                self.iterator.next();

                // word of no delimiters, or protected word: just return it
                if (self.iterator.current == 0 && self.iterator.end == term_length as i32)
                    || self
                        .prot_words
                        .as_ref()
                        .is_some_and(|p| p.contains_chars(Some(&term_buffer), 0, term_length))
                {
                    pos_inc_att(source).set_position_increment(self.accum_pos_inc);
                    self.accum_pos_inc = 0;
                    return Ok(true);
                }

                // word of simply delimiters: swallow this token, creating a hole, and move on
                // to next token
                if self.iterator.end == DONE {
                    if !self.has(PRESERVE_ORIGINAL) {
                        continue;
                    } else {
                        self.accum_pos_inc = 0;
                        return Ok(true);
                    }
                }

                // otherwise, we have delimiters, process & buffer all parts:
                self.buffer_word_parts(source);
            }

            if self.buffered_pos < self.buffered_len {
                source.clear_attributes();
                source.restore_state(
                    self.saved_state
                        .as_ref()
                        .expect("INVARIANT: the word was saved"),
                )?;

                let term_part = self.buffered_term_parts[self.buffered_pos].clone();
                let start_pos = self.buffered_parts[4 * self.buffered_pos];
                let end_pos = self.buffered_parts[4 * self.buffered_pos + 1];
                let start_part = self.buffered_parts[4 * self.buffered_pos + 2];
                let end_part = self.buffered_parts[4 * self.buffered_pos + 3];
                self.buffered_pos += 1;

                let (mut start_offset, mut end_offset) = if !self.adjusting_offsets {
                    (self.saved_start_offset, self.saved_end_offset)
                } else {
                    (
                        self.saved_start_offset + start_part,
                        self.saved_start_offset + end_part,
                    )
                };

                // never let offsets go backwards:
                start_offset = start_offset.max(self.last_start_offset);
                end_offset = end_offset.max(self.last_start_offset);

                offset_att(source).set_offset(start_offset, end_offset);
                self.last_start_offset = start_offset;

                let chars = match &term_part {
                    None => utf16_to_term_chars(
                        &self.saved_term_buffer[start_part as usize..end_part as usize],
                    ),
                    Some(term_part) => utf16_to_term_chars(term_part),
                };
                source
                    .get_attribute_interface_mut::<dyn CharTermAttribute>()
                    .expect("INVARIANT: the constructor added the CharTermAttribute")
                    .copy_buffer(&chars, 0, chars.len());

                pos_inc_att(source)
                    .set_position_increment(self.accum_pos_inc + start_pos - self.word_pos);
                self.accum_pos_inc = 0;
                source
                    .get_attribute_interface_mut::<dyn PositionLengthAttribute>()
                    .expect("INVARIANT: the constructor added the PositionLengthAttribute")
                    .set_position_length(end_pos - start_pos);
                self.word_pos = start_pos;
                return Ok(true);
            }

            // no saved concatenations, on to the next input word
            self.saved_state = None;
        }
    }

    /// Equivalent to `WordDelimiterGraphFilter.reset()`.
    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.accum_pos_inc = 0;
        self.saved_state = None;
        self.last_start_offset = 0;
        self.concat.clear();
        self.concat_all.clear();
        Ok(())
    }
}

/// Returns string representation of configuration flags.
///
/// Equivalent to the static `WordDelimiterGraphFilter.flagsToString(int)`.
pub fn flags_to_string(flags: i32) -> String {
    let names = [
        (GENERATE_WORD_PARTS, "GENERATE_WORD_PARTS"),
        (GENERATE_NUMBER_PARTS, "GENERATE_NUMBER_PARTS"),
        (CATENATE_WORDS, "CATENATE_WORDS"),
        (CATENATE_NUMBERS, "CATENATE_NUMBERS"),
        (CATENATE_ALL, "CATENATE_ALL"),
        (PRESERVE_ORIGINAL, "PRESERVE_ORIGINAL"),
        (SPLIT_ON_CASE_CHANGE, "SPLIT_ON_CASE_CHANGE"),
        (SPLIT_ON_NUMERICS, "SPLIT_ON_NUMERICS"),
        (STEM_ENGLISH_POSSESSIVE, "STEM_ENGLISH_POSSESSIVE"),
    ];
    let mut b = String::new();
    for (flag, name) in names {
        if (flags & flag) != 0 {
            if !b.is_empty() {
                b.push_str(" | ");
            }
            b.push_str(name);
        }
    }
    b
}

/// Splits words into subwords and performs optional transformations on
/// subword groups, producing a correct token graph so that e.g.
/// `PhraseQuery` can work correctly when this filter is used in the search
/// time analyzer. Unlike the deprecated `WordDelimiterFilter`, this token
/// filter produces a correct token graph as output. However, it cannot
/// consume an input token graph correctly. Processing is suppressed by
/// `KeywordAttribute.isKeyword()`=true.
///
/// Words are split into subwords with the following rules:
///
/// * split on intra-word delimiters (by default, all non alpha-numeric
///   characters): "Wi-Fi" → "Wi", "Fi"
/// * split on case transitions: "PowerShot" → "Power", "Shot"
/// * split on letter-number transitions: "SD500" → "SD", "500"
/// * leading and trailing intra-word delimiters on each subword are
///   ignored: "//hello---there, 'dude'" → "hello", "there", "dude"
/// * trailing "'s" are removed for each subword: "O'Neil's" → "O", "Neil"
///   * Note: this step isn't performed in a separate filter because of
///     possible subword combinations.
///
/// The **GENERATE...** options affect how incoming tokens are broken into
/// parts, and the various **CATENATE_...** parameters affect how those parts
/// are combined.
///
/// * If no CATENATE option is set, then no subword combinations are
///   generated: "PowerShot" → 0:"Power", 1:"Shot" (0 and 1 are the token
///   positions)
/// * CATENATE_WORDS means that in addition to the subwords, maximum runs of
///   non-numeric subwords are catenated and produced at the same position of
///   the last subword in the run:
///   * "PowerShot" → 0:"Power", 1:"Shot" 1:"PowerShot"
///   * "A's+B's&C's" > 0:"A", 1:"B", 2:"C", 2:"ABC"
///   * "Super-Duper-XL500-42-AutoCoder!" → 0:"Super", 1:"Duper", 2:"XL",
///     2:"SuperDuperXL", 3:"500" 4:"42", 5:"Auto", 6:"Coder",
///     6:"AutoCoder"
/// * CATENATE_NUMBERS works like CATENATE_WORDS, but for adjacent digit
///   sequences.
/// * CATENATE_ALL smushes together all the token parts without distinguishing
///   numbers and words.
///
/// One use for `WordDelimiterGraphFilter` is to help match words with
/// different subword delimiters. For example, if the source text contained
/// "wi-fi" one may want "wifi" "WiFi" "wi-fi" "wi+fi" queries to all match.
/// One way of doing so is to specify CATENATE options in the analyzer used
/// for indexing, and not in the analyzer used for querying. Given that the
/// current `StandardTokenizer` immediately removes many intra-word
/// delimiters, it is recommended that this filter be used after a tokenizer
/// that does not do this (such as `WhitespaceTokenizer`).
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.WordDelimiterGraphFilter`,
/// whose `toString()` is [`word_delimiter_graph_filter_to_string`].
pub type WordDelimiterGraphFilter = TokenFilter<WordDelimiterGraphFilterLogic>;

/// Equivalent to `WordDelimiterGraphFilter.toString()`,
/// `WordDelimiterGraphFilter(flags=...)`.
pub fn word_delimiter_graph_filter_to_string(filter: &WordDelimiterGraphFilter) -> String {
    format!(
        "WordDelimiterGraphFilter(flags={})",
        flags_to_string(filter.logic().flags)
    )
}

/// Creates a new WordDelimiterGraphFilter: `input` the TokenStream to be
/// filtered, `adjust_internal_offsets` whether the internal offsets of the
/// parts are adjusted, `char_type_table` the table containing character
/// types, `configuration_flags` the flags controlling behaviour (see the
/// constants), and `prot_words` the words to protect from being delimited
/// (`None` for Java's `null`).
///
/// Equivalent to `WordDelimiterGraphFilter(TokenStream, boolean, byte[],
/// int, CharArraySet)`, which adds the `CharTermAttribute`,
/// `KeywordAttribute`, `OffsetAttribute`, `PositionIncrementAttribute` and
/// `PositionLengthAttribute`.
///
/// # Errors
///
/// `IllegalArgumentException` "flags contains unrecognized flag: n", and
/// what the attribute factory raises.
pub fn new_word_delimiter_graph_filter_with(
    input: Box<dyn TokenStream>,
    adjust_internal_offsets: bool,
    char_type_table: Arc<[u8]>,
    configuration_flags: i32,
    prot_words: Option<CharArraySet>,
) -> Result<WordDelimiterGraphFilter> {
    let mut filter = TokenFilter::new(
        input,
        WordDelimiterGraphFilterLogic {
            prot_words: None,
            flags: 0,
            buffered_parts: vec![0; 16],
            buffered_len: 0,
            buffered_pos: 0,
            buffered_term_parts: vec![None; 4],
            iterator: WordDelimiterIterator::new(Arc::clone(&char_type_table), false, false, false),
            concat: WordDelimiterConcatenation::default(),
            adjust_internal_offsets: false,
            last_concat_count: 0,
            concat_all: WordDelimiterConcatenation::default(),
            accum_pos_inc: 0,
            saved_term_buffer: Vec::new(),
            saved_term_length: 0,
            saved_start_offset: 0,
            saved_end_offset: 0,
            saved_state: None,
            last_start_offset: 0,
            adjusting_offsets: false,
            word_pos: 0,
        },
    );
    // Java's field initializers `addAttribute` run before the constructor body.
    let source = filter.attribute_source_mut();
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_interface::<dyn KeywordAttribute>()?;
    source.add_attribute_interface::<dyn OffsetAttribute>()?;
    source.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
    source.add_attribute_interface::<dyn PositionLengthAttribute>()?;
    if (configuration_flags
        & !(GENERATE_WORD_PARTS
            | GENERATE_NUMBER_PARTS
            | CATENATE_WORDS
            | CATENATE_NUMBERS
            | CATENATE_ALL
            | PRESERVE_ORIGINAL
            | SPLIT_ON_CASE_CHANGE
            | SPLIT_ON_NUMERICS
            | STEM_ENGLISH_POSSESSIVE
            | IGNORE_KEYWORDS))
        != 0
    {
        return Err(LuceneError::illegal_argument(format!(
            "flags contains unrecognized flag: {configuration_flags}"
        )));
    }
    let logic = filter.logic_mut();
    logic.flags = configuration_flags;
    logic.prot_words = prot_words;
    let has = |flag: i32| (configuration_flags & flag) != 0;
    logic.iterator = WordDelimiterIterator::new(
        char_type_table,
        has(SPLIT_ON_CASE_CHANGE),
        has(SPLIT_ON_NUMERICS),
        has(STEM_ENGLISH_POSSESSIVE),
    );
    logic.adjust_internal_offsets = adjust_internal_offsets;
    Ok(filter)
}

/// Creates a new WordDelimiterGraphFilter using `DEFAULT_WORD_DELIM_TABLE`
/// as its charTypeTable, without adjusting the internal offsets.
///
/// Equivalent to `WordDelimiterGraphFilter(TokenStream, int, CharArraySet)`.
///
/// # Errors
///
/// As [`new_word_delimiter_graph_filter_with`].
pub fn new_word_delimiter_graph_filter(
    input: Box<dyn TokenStream>,
    configuration_flags: i32,
    prot_words: Option<CharArraySet>,
) -> Result<WordDelimiterGraphFilter> {
    new_word_delimiter_graph_filter_with(
        input,
        false,
        Arc::clone(&DEFAULT_WORD_DELIM_TABLE),
        configuration_flags,
        prot_words,
    )
}
