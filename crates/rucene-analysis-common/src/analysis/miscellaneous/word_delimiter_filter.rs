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
//! subword groups, ported from
//! `org.apache.lucene.analysis.miscellaneous.WordDelimiterFilter`.

use std::sync::Arc;

use rucene::analysis::{
    CharArraySet, CharTermAttribute, KeywordAttribute, OffsetAttribute, PositionIncrementAttribute,
    TokenFilter, TokenFilterLogic, TokenStream, TypeAttribute,
};
use rucene::error::Result;
use rucene::util::attribute::{AttributeSource, CapturedState};
use rucene::util::selector::PivotOps;
use rucene::util::sorter::{InPlaceMergeSorter, Sorter};
use rucene::util::ArrayUtil;

use super::word_delimiter_iterator::{WordDelimiterIterator, DEFAULT_WORD_DELIM_TABLE, DONE};
use crate::jdk::character::{chars_to_utf16, utf16_to_term_chars};

/// Equivalent to `WordDelimiterFilter.LOWER`.
pub const LOWER: i32 = 0x01;
/// Equivalent to `WordDelimiterFilter.UPPER`.
pub const UPPER: i32 = 0x02;
/// Equivalent to `WordDelimiterFilter.DIGIT`.
pub const DIGIT: i32 = 0x04;
/// Equivalent to `WordDelimiterFilter.SUBWORD_DELIM`.
pub const SUBWORD_DELIM: i32 = 0x08;

// combinations: for testing, not for setting bits
/// Equivalent to `WordDelimiterFilter.ALPHA`.
pub const ALPHA: i32 = 0x03;
/// Equivalent to `WordDelimiterFilter.ALPHANUM`.
pub const ALPHANUM: i32 = 0x07;

/// Causes parts of words to be generated: "PowerShot" => "Power" "Shot"
///
/// Equivalent to `WordDelimiterFilter.GENERATE_WORD_PARTS`.
pub const GENERATE_WORD_PARTS: i32 = 1;

/// Causes number subwords to be generated: "500-42" => "500" "42"
///
/// Equivalent to `WordDelimiterFilter.GENERATE_NUMBER_PARTS`.
pub const GENERATE_NUMBER_PARTS: i32 = 2;

/// Causes maximum runs of word parts to be catenated: "wi-fi" => "wifi"
///
/// Equivalent to `WordDelimiterFilter.CATENATE_WORDS`.
pub const CATENATE_WORDS: i32 = 4;

/// Causes maximum runs of word parts to be catenated: "500-42" => "50042"
///
/// Equivalent to `WordDelimiterFilter.CATENATE_NUMBERS`.
pub const CATENATE_NUMBERS: i32 = 8;

/// Causes all subword parts to be catenated: "wi-fi-4000" => "wifi4000"
///
/// Equivalent to `WordDelimiterFilter.CATENATE_ALL`.
pub const CATENATE_ALL: i32 = 16;

/// Causes original words are preserved and added to the subword list
/// (Defaults to false): "500-42" => "500" "42" "500-42"
///
/// Equivalent to `WordDelimiterFilter.PRESERVE_ORIGINAL`.
pub const PRESERVE_ORIGINAL: i32 = 32;

/// If not set, causes case changes to be ignored (subwords will only be
/// generated given SUBWORD_DELIM tokens)
///
/// Equivalent to `WordDelimiterFilter.SPLIT_ON_CASE_CHANGE`.
pub const SPLIT_ON_CASE_CHANGE: i32 = 64;

/// If not set, causes numeric changes to be ignored (subwords will only be
/// generated given SUBWORD_DELIM tokens).
///
/// Equivalent to `WordDelimiterFilter.SPLIT_ON_NUMERICS`.
pub const SPLIT_ON_NUMERICS: i32 = 128;

/// Causes trailing "'s" to be removed for each subword: "O'Neil's" => "O",
/// "Neil"
///
/// Equivalent to `WordDelimiterFilter.STEM_ENGLISH_POSSESSIVE`.
pub const STEM_ENGLISH_POSSESSIVE: i32 = 256;

/// Suppresses processing terms with `KeywordAttribute.isKeyword()`=true.
///
/// Equivalent to `WordDelimiterFilter.IGNORE_KEYWORDS`.
pub const IGNORE_KEYWORDS: i32 = 512;

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
/// Equivalent to the inner class `org.apache.lucene.analysis.miscellaneous.WordDelimiterFilter$WordDelimiterConcatenation`;
/// its `write()`, which reaches the enclosing filter's attributes and
/// state, is the filter's `write_concatenation`. The buffer holds UTF-16
/// units, as Java's `StringBuilder` does.
#[derive(Clone, Debug, Default)]
pub struct WordDelimiterConcatenation {
    buffer: Vec<u16>,
    start_offset: i32,
    end_offset: i32,
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

    /// Clears the concatenation and resets its state.
    ///
    /// Equivalent to `WordDelimiterConcatenation.clear()`.
    fn clear(&mut self) {
        self.buffer.clear();
        self.start_offset = 0;
        self.end_offset = 0;
        self.type_ = 0;
        self.subword_count = 0;
    }
}

/// Sorts the buffered tokens by start offset, then by descending position
/// increment.
///
/// Equivalent to the private inner class `org.apache.lucene.analysis.miscellaneous.WordDelimiterFilter$OffsetSorter`,
/// an `InPlaceMergeSorter` over the filter's three parallel arrays.
#[derive(Debug)]
pub struct OffsetSorter<'a> {
    buffered: &'a mut [Option<CapturedState>],
    start_off: &'a mut [i32],
    pos_inc: &'a mut [i32],
    pivot_index: usize,
}

impl PivotOps for OffsetSorter<'_> {
    /// Equivalent to `OffsetSorter.swap(int, int)`.
    fn swap(&mut self, i: usize, j: usize) {
        self.buffered.swap(i, j);
        self.start_off.swap(i, j);
        self.pos_inc.swap(i, j);
    }

    fn set_pivot(&mut self, i: usize) {
        self.pivot_index = i;
    }

    fn compare_pivot(&mut self, j: usize) -> i32 {
        self.compare(self.pivot_index, j)
    }

    /// Equivalent to `OffsetSorter.compare(int, int)`.
    fn compare(&mut self, i: usize, j: usize) -> i32 {
        let mut cmp = self.start_off[i].cmp(&self.start_off[j]) as i32;
        if cmp == 0 {
            cmp = self.pos_inc[j].cmp(&self.pos_inc[i]) as i32;
        }
        cmp
    }
}

impl Sorter for OffsetSorter<'_> {
    fn sort(&mut self, from: usize, to: usize) {
        self.in_place_merge_sort(from, to);
    }
}

/// The body of [`WordDelimiterFilter`]: what Lucene's `WordDelimiterFilter`
/// adds to `TokenFilter`.
///
/// The term is read as its UTF-16 units, as Java's `char[]` is: positions,
/// lengths and offsets are computed on them, and every term the filter
/// writes is a slice of them, which never cuts a surrogate pair (both units
/// of a pair are `ALPHA | DIGIT`, so no break falls between them).
#[derive(Debug)]
pub struct WordDelimiterFilterLogic {
    prot_words: Option<CharArraySet>,
    flags: i32,

    /// used for iterating word delimiter breaks; its text is Java's
    /// `savedBuffer` once a word is saved
    iterator: WordDelimiterIterator,

    /// used for concatenating runs of similar typed subwords (word,number)
    concat: WordDelimiterConcatenation,
    /// number of subwords last output by concat.
    last_concat_count: i32,

    /// used for catenate all
    concat_all: WordDelimiterConcatenation,

    /// used for accumulating position increment gaps
    accum_pos_inc: i32,

    saved_start_offset: i32,
    saved_end_offset: i32,
    saved_type: String,
    has_saved_state: bool,
    /// if length by start + end offsets doesn't match the term text then
    /// assume this is a synonym and don't adjust the offsets.
    has_illegal_offsets: bool,

    /// for a run of the same subword type within a word, have we output
    /// anything?
    has_output_token: bool,
    /// when preserve original is on, have we output any token following it?
    /// this token must have posInc=0!
    has_output_following_original: bool,

    buffered: Vec<Option<CapturedState>>,
    start_off: Vec<i32>,
    pos_inc: Vec<i32>,
    buffered_len: usize,
    buffered_pos: usize,
    first: bool,
}

impl WordDelimiterFilterLogic {
    /// Equivalent to the private `WordDelimiterFilter.buffer()`.
    fn buffer(&mut self, source: &AttributeSource) {
        if self.buffered_len == self.buffered.len() {
            let new_size = ArrayUtil::oversize(self.buffered_len + 1, 8);
            self.buffered.resize(new_size, None);
            self.start_off.resize(new_size, 0);
            self.pos_inc.resize(new_size, 0);
        }
        self.start_off[self.buffered_len] = offset_att(source).start_offset();
        self.pos_inc[self.buffered_len] = pos_inc_att(source).get_position_increment();
        self.buffered[self.buffered_len] = source.capture_state();
        self.buffered_len += 1;
    }

    /// Saves the existing attribute states.
    ///
    /// Equivalent to the private `WordDelimiterFilter.saveState()`; the term
    /// was copied into the iterator's text by `setText`, which Java's
    /// `savedBuffer` copy and `iterator.text = savedBuffer` amount to.
    fn save_state(&mut self, source: &AttributeSource, term_length: usize) {
        // otherwise, we have delimiters, save state
        {
            let offset_att = offset_att(source);
            self.saved_start_offset = offset_att.start_offset();
            self.saved_end_offset = offset_att.end_offset();
        }
        // if length by start + end offsets doesn't match the term text then assume this is a
        // synonym and don't adjust the offsets.
        self.has_illegal_offsets =
            (self.saved_end_offset - self.saved_start_offset) as i64 != term_length as i64;
        self.saved_type = source
            .get_attribute_interface::<dyn TypeAttribute>()
            .expect("INVARIANT: the constructor added the TypeAttribute")
            .type_value()
            .to_string();
        self.has_saved_state = true;
    }

    /// Flushes the given WordDelimiterConcatenation by either writing its
    /// concat and then clearing, or just clearing.
    ///
    /// Equivalent to the private `WordDelimiterFilter.flushConcatenation(WordDelimiterConcatenation)`.
    fn flush_concatenation(&mut self, source: &AttributeSource, which: Which) -> Result<bool> {
        let (subword_count, type_) = {
            let concatenation = self.concatenation(which);
            (concatenation.subword_count, concatenation.type_)
        };
        self.last_concat_count = subword_count;
        if subword_count != 1 || !self.should_generate_parts(type_) {
            self.write_and_clear(source, which)?;
            return Ok(true);
        }
        self.concatenation_mut(which).clear();
        Ok(false)
    }

    /// Determines whether to concatenate a word or number if the current
    /// word is the given type.
    ///
    /// Equivalent to the private `WordDelimiterFilter.shouldConcatenate(int)`.
    fn should_concatenate(&self, word_type: u8) -> bool {
        (self.has(CATENATE_WORDS) && is_alpha(word_type))
            || (self.has(CATENATE_NUMBERS) && is_digit(word_type))
    }

    /// Determines whether a word/number part should be generated for a word
    /// of the given type.
    ///
    /// Equivalent to the private `WordDelimiterFilter.shouldGenerateParts(int)`.
    fn should_generate_parts(&self, word_type: u8) -> bool {
        (self.has(GENERATE_WORD_PARTS) && is_alpha(word_type))
            || (self.has(GENERATE_NUMBER_PARTS) && is_digit(word_type))
    }

    /// Concatenates the saved buffer to the given WordDelimiterConcatenation.
    ///
    /// Equivalent to the private `WordDelimiterFilter.concatenate(WordDelimiterConcatenation)`.
    fn concatenate(&mut self, which: Which) {
        let current = self.iterator.current;
        let end = self.iterator.end;
        let saved_start_offset = self.saved_start_offset;
        let concatenation = match which {
            Which::Concat => &mut self.concat,
            Which::ConcatAll => &mut self.concat_all,
        };
        if concatenation.is_empty() {
            concatenation.start_offset = saved_start_offset + current;
        }
        concatenation.append(
            &self.iterator.text,
            current as usize,
            (end - current) as usize,
        );
        concatenation.end_offset = saved_start_offset + end;
    }

    /// Generates a word/number part, updating the appropriate attributes.
    ///
    /// Equivalent to the private `WordDelimiterFilter.generatePart(boolean)`.
    fn generate_part(&mut self, source: &AttributeSource, is_single_word: bool) {
        source.clear_attributes();
        let current = self.iterator.current;
        let end = self.iterator.end;
        let chars = utf16_to_term_chars(&self.iterator.text[current as usize..end as usize]);
        term_att(source).copy_buffer(&chars, 0, chars.len());

        let start_offset = self.saved_start_offset + current;
        let end_offset = self.saved_start_offset + end;

        {
            let mut offset_att = offset_att(source);
            if self.has_illegal_offsets {
                // historically this filter did this regardless for 'isSingleWord',
                // but we must do a sanity check:
                if is_single_word && start_offset <= self.saved_end_offset {
                    offset_att.set_offset(start_offset, self.saved_end_offset);
                } else {
                    offset_att.set_offset(self.saved_start_offset, self.saved_end_offset);
                }
            } else {
                offset_att.set_offset(start_offset, end_offset);
            }
        }
        let position = self.position(false);
        pos_inc_att(source).set_position_increment(position);
        type_att_set(source, &self.saved_type);
    }

    /// Get the position increment gap for a subword or concatenation.
    ///
    /// Equivalent to the private `WordDelimiterFilter.position(boolean)`:
    /// `inject` is true if this token wants to be injected.
    fn position(&mut self, inject: bool) -> i32 {
        let pos_inc = self.accum_pos_inc;

        if self.has_output_token {
            self.accum_pos_inc = 0;
            return if inject { 0 } else { pos_inc.max(1) };
        }

        self.has_output_token = true;

        if !self.has_output_following_original {
            // the first token following the original is 0 regardless
            self.has_output_following_original = true;
            return 0;
        }
        // clear the accumulated position increment
        self.accum_pos_inc = 0;
        pos_inc.max(1)
    }

    /// Determines whether the given flag is set.
    ///
    /// Equivalent to the private `WordDelimiterFilter.has(int)`.
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

    /// Writes the concatenation to the attributes.
    ///
    /// Equivalent to `WordDelimiterConcatenation.write()`.
    fn write_concatenation(&mut self, source: &AttributeSource, which: Which) {
        source.clear_attributes();
        let (chars, start_offset, end_offset) = {
            let concatenation = self.concatenation(which);
            (
                utf16_to_term_chars(&concatenation.buffer),
                concatenation.start_offset,
                concatenation.end_offset,
            )
        };
        term_att(source).copy_buffer(&chars, 0, chars.len());

        {
            let mut offset_att = offset_att(source);
            if self.has_illegal_offsets {
                offset_att.set_offset(self.saved_start_offset, self.saved_end_offset);
            } else {
                offset_att.set_offset(start_offset, end_offset);
            }
        }
        let position = self.position(true);
        pos_inc_att(source).set_position_increment(position);
        type_att_set(source, &self.saved_type);
        self.accum_pos_inc = 0;
    }

    /// Convenience method for the common scenario of having to write the
    /// concetenation and then clearing its state.
    ///
    /// Equivalent to `WordDelimiterConcatenation.writeAndClear()`.
    fn write_and_clear(&mut self, source: &AttributeSource, which: Which) -> Result<()> {
        self.write_concatenation(source, which);
        self.concatenation_mut(which).clear();
        Ok(())
    }
}

fn term_att(
    source: &AttributeSource,
) -> rucene::util::atomic_ref_cell::AtomicRefMut<'_, dyn CharTermAttribute> {
    source
        .get_attribute_interface_mut::<dyn CharTermAttribute>()
        .expect("INVARIANT: the constructor added the CharTermAttribute")
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

fn type_att_set(source: &AttributeSource, type_value: &str) {
    source
        .get_attribute_interface_mut::<dyn TypeAttribute>()
        .expect("INVARIANT: the constructor added the TypeAttribute")
        .set_type(type_value.to_string());
}

impl TokenFilterLogic for WordDelimiterFilterLogic {
    /// Equivalent to `WordDelimiterFilter.incrementToken()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        loop {
            if !self.has_saved_state {
                // process a new input word
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
                    let term_att = term_att(source);
                    chars_to_utf16(&term_att.buffer()[..term_att.length()])
                };
                let term_length = term_buffer.len();
                self.accum_pos_inc += pos_inc_att(source).get_position_increment();

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
                    self.first = false;
                    return Ok(true);
                }

                // word of simply delimiters
                if self.iterator.end == DONE && !self.has(PRESERVE_ORIGINAL) {
                    // if the posInc is 1, simply ignore it in the accumulation
                    // TODO: proper hole adjustment (FilteringTokenFilter-like) instead of this
                    // previous logic!
                    if pos_inc_att(source).get_position_increment() == 1 && !self.first {
                        self.accum_pos_inc -= 1;
                    }
                    continue;
                }

                self.save_state(source, term_length);

                self.has_output_token = false;
                self.has_output_following_original = !self.has(PRESERVE_ORIGINAL);
                self.last_concat_count = 0;

                if self.has(PRESERVE_ORIGINAL) {
                    pos_inc_att(source).set_position_increment(self.accum_pos_inc);
                    self.accum_pos_inc = 0;
                    self.first = false;
                    return Ok(true);
                }
            }

            // at the end of the string, output any concatenations
            if self.iterator.end == DONE {
                if !self.concat.is_empty() && self.flush_concatenation(source, Which::Concat)? {
                    self.buffer(source);
                    continue;
                }

                if !self.concat_all.is_empty() {
                    // only if we haven't output this same combo above!
                    if self.concat_all.subword_count > self.last_concat_count {
                        self.write_and_clear(source, Which::ConcatAll)?;
                        self.buffer(source);
                        continue;
                    }
                    self.concat_all.clear();
                }

                if self.buffered_pos < self.buffered_len {
                    if self.buffered_pos == 0 {
                        let len = self.buffered_len;
                        OffsetSorter {
                            buffered: &mut self.buffered,
                            start_off: &mut self.start_off,
                            pos_inc: &mut self.pos_inc,
                            pivot_index: 0,
                        }
                        .sort(0, len);
                    }
                    source.clear_attributes();
                    let state = self.buffered[self.buffered_pos]
                        .as_ref()
                        .expect("INVARIANT: a state was buffered");
                    source.restore_state(state)?;
                    self.buffered_pos += 1;
                    if self.first && pos_inc_att(source).get_position_increment() == 0 {
                        // can easily happen with strange combinations (e.g. not outputting
                        // numbers, but concat-all)
                        pos_inc_att(source).set_position_increment(1);
                    }
                    self.first = false;
                    return Ok(true);
                }

                // no saved concatenations, on to the next input word
                self.buffered_pos = 0;
                self.buffered_len = 0;
                self.has_saved_state = false;
                continue;
            }

            // word surrounded by delimiters: always output
            if self.iterator.is_single_word() {
                self.generate_part(source, true);
                self.iterator.next();
                self.first = false;
                return Ok(true);
            }

            let word_type = self.iterator.type_();

            // do we already have queued up incompatible concatenations?
            if !self.concat.is_empty() && (self.concat.type_ & word_type) == 0 {
                if self.flush_concatenation(source, Which::Concat)? {
                    self.has_output_token = false;
                    self.buffer(source);
                    continue;
                }
                self.has_output_token = false;
            }

            // add subwords depending upon options
            if self.should_concatenate(word_type) {
                if self.concat.is_empty() {
                    self.concat.type_ = word_type;
                }
                self.concatenate(Which::Concat);
            }

            // add all subwords (catenateAll)
            if self.has(CATENATE_ALL) {
                self.concatenate(Which::ConcatAll);
            }

            // if we should output the word or number part
            if self.should_generate_parts(word_type) {
                self.generate_part(source, false);
                self.buffer(source);
            }

            self.iterator.next();
        }
    }

    /// Equivalent to `WordDelimiterFilter.reset()`.
    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.has_saved_state = false;
        self.concat.clear();
        self.concat_all.clear();
        self.accum_pos_inc = 0;
        self.buffered_pos = 0;
        self.buffered_len = 0;
        self.first = true;
        Ok(())
    }
}

/// Checks if the given word type includes `ALPHA`.
///
/// Equivalent to `WordDelimiterFilter.isAlpha(int)`.
fn is_alpha(type_: u8) -> bool {
    (i32::from(type_) & ALPHA) != 0
}

/// Checks if the given word type includes `DIGIT`.
///
/// Equivalent to `WordDelimiterFilter.isDigit(int)`.
fn is_digit(type_: u8) -> bool {
    (i32::from(type_) & DIGIT) != 0
}

/// Splits words into subwords and performs optional transformations on
/// subword groups. Words are split into subwords with the following rules:
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
/// The **combinations** parameter affects how subwords are combined:
///
/// * combinations="0" causes no subword combinations: "PowerShot" → 0:"Power",
///   1:"Shot" (0 and 1 are the token positions)
/// * combinations="1" means that in addition to the subwords, maximum runs
///   of non-numeric subwords are catenated and produced at the same
///   position of the last subword in the run:
///   * "PowerShot" → 0:"Power", 1:"Shot" 1:"PowerShot"
///   * "A's+B's&C's" → 0:"A", 1:"B", 2:"C", 2:"ABC"
///   * "Super-Duper-XL500-42-AutoCoder!" → 0:"Super", 1:"Duper", 2:"XL",
///     2:"SuperDuperXL", 3:"500" 4:"42", 5:"Auto", 6:"Coder",
///     6:"AutoCoder"
///
/// One use for WordDelimiterFilter is to help match words with different
/// subword delimiters. For example, if the source text contained "wi-fi"
/// one may want "wifi" "WiFi" "wi-fi" "wi+fi" queries to all match. One way
/// of doing so is to specify combinations="1" in the analyzer used for
/// indexing, and combinations="0" (the default) in the analyzer used for
/// querying. Given that the current `StandardTokenizer` immediately removes
/// many intra-word delimiters, it is recommended that this filter be used
/// after a tokenizer that does not do this (such as `WhitespaceTokenizer`).
///
/// **Deprecated** in Lucene: use `WordDelimiterGraphFilter` instead: it
/// produces a correct token graph so that e.g. `PhraseQuery` works correctly
/// when it's used in the search time analyzer.
///
/// Equivalent to `org.apache.lucene.analysis.miscellaneous.WordDelimiterFilter`.
pub type WordDelimiterFilter = TokenFilter<WordDelimiterFilterLogic>;

/// Creates a new WordDelimiterFilter: `input` the TokenStream to be
/// filtered, `char_type_table` the table containing character types,
/// `configuration_flags` the flags controlling behaviour (see the constants),
/// and `prot_words` the words to protect from being delimited (`None` for
/// Java's `null`).
///
/// Equivalent to `WordDelimiterFilter(TokenStream, byte[], int,
/// CharArraySet)`, which adds the `CharTermAttribute`, `KeywordAttribute`,
/// `OffsetAttribute`, `PositionIncrementAttribute` and `TypeAttribute`.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws.
#[deprecated(note = "Use WordDelimiterGraphFilter instead")]
pub fn new_word_delimiter_filter_with_table(
    input: Box<dyn TokenStream>,
    char_type_table: Arc<[u8]>,
    configuration_flags: i32,
    prot_words: Option<CharArraySet>,
) -> Result<WordDelimiterFilter> {
    let has = |flag: i32| (configuration_flags & flag) != 0;
    let iterator = WordDelimiterIterator::new(
        char_type_table,
        has(SPLIT_ON_CASE_CHANGE),
        has(SPLIT_ON_NUMERICS),
        has(STEM_ENGLISH_POSSESSIVE),
    );
    let mut filter = TokenFilter::new(
        input,
        WordDelimiterFilterLogic {
            prot_words,
            flags: configuration_flags,
            iterator,
            concat: WordDelimiterConcatenation::default(),
            last_concat_count: 0,
            concat_all: WordDelimiterConcatenation::default(),
            accum_pos_inc: 0,
            saved_start_offset: 0,
            saved_end_offset: 0,
            saved_type: String::new(),
            has_saved_state: false,
            has_illegal_offsets: false,
            has_output_token: false,
            has_output_following_original: false,
            buffered: vec![None; 8],
            start_off: vec![0; 8],
            pos_inc: vec![0; 8],
            buffered_len: 0,
            buffered_pos: 0,
            first: false,
        },
    );
    let source = filter.attribute_source_mut();
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_interface::<dyn KeywordAttribute>()?;
    source.add_attribute_interface::<dyn OffsetAttribute>()?;
    source.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
    source.add_attribute_interface::<dyn TypeAttribute>()?;
    Ok(filter)
}

/// Creates a new WordDelimiterFilter using `DEFAULT_WORD_DELIM_TABLE` as its
/// charTypeTable.
///
/// Equivalent to `WordDelimiterFilter(TokenStream, int, CharArraySet)`.
///
/// # Errors
///
/// As [`new_word_delimiter_filter_with_table`].
#[deprecated(note = "Use WordDelimiterGraphFilter instead")]
#[allow(deprecated)]
pub fn new_word_delimiter_filter(
    input: Box<dyn TokenStream>,
    configuration_flags: i32,
    prot_words: Option<CharArraySet>,
) -> Result<WordDelimiterFilter> {
    new_word_delimiter_filter_with_table(
        input,
        Arc::clone(&DEFAULT_WORD_DELIM_TABLE),
        configuration_flags,
        prot_words,
    )
}
