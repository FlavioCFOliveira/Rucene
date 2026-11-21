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

//! Matches single or multi word synonyms in a token stream, ported from
//! `org.apache.lucene.analysis.synonym.SynonymFilter`.

use std::sync::Arc;

use rucene::analysis::character_utils::{char_count, code_point_at};
use rucene::analysis::{
    CharTermAttribute, OffsetAttribute, PositionIncrementAttribute, PositionLengthAttribute,
    TokenFilter, TokenFilterLogic, TokenStream, TypeAttribute,
};
use rucene::error::{LuceneError, Result};
use rucene::store::{ByteArrayDataInput, DataInput};
use rucene::util::attribute::{AttributeSource, CapturedState};
use rucene::util::automaton::case_folding;
use rucene::util::chars_ref::CharsRef;
use rucene::util::fst::{Arc as FSTArc, Outputs};
use rucene::util::refs::CharsRefBuilder;
use rucene::util::BytesRef;

use super::synonym_map::{read_words, SynonymMap};
use crate::jdk::character::{chars_to_utf16, utf16_to_term_chars};

/// Hold all buffered (read ahead) stacked input tokens for a future
/// position. When multiple tokens are at the same position, we only store
/// (and match against) the term for the first token at the position, but
/// capture state for (and enumerate) all other tokens at this position.
///
/// Equivalent to `org.apache.lucene.analysis.synonym.SynonymFilter$PendingInput`.
#[derive(Debug)]
pub(crate) struct PendingInput {
    term: CharsRefBuilder,
    state: Option<CapturedState>,
    keep_orig: bool,
    matched: bool,
    consumed: bool,
    start_offset: i32,
    end_offset: i32,
}

impl PendingInput {
    fn new() -> Self {
        Self {
            term: CharsRefBuilder::new(),
            state: None,
            keep_orig: false,
            matched: false,
            consumed: true,
            start_offset: 0,
            end_offset: 0,
        }
    }

    /// Equivalent to `PendingInput.reset()`.
    fn reset(&mut self) {
        self.state = None;
        self.consumed = true;
        self.keep_orig = false;
        self.matched = false;
    }
}

/// Holds pending output synonyms for one future position.
///
/// Equivalent to `org.apache.lucene.analysis.synonym.SynonymFilter$PendingOutputs`.
#[derive(Debug)]
pub(crate) struct PendingOutputs {
    outputs: Vec<CharsRefBuilder>,
    end_offsets: Vec<i32>,
    pos_lengths: Vec<i32>,
    upto: usize,
    count: usize,
    pos_incr: i32,
    last_end_offset: i32,
    last_pos_length: i32,
}

impl PendingOutputs {
    /// Equivalent to `PendingOutputs()`.
    fn new() -> Self {
        Self {
            outputs: Vec::with_capacity(1),
            end_offsets: Vec::with_capacity(1),
            pos_lengths: Vec::with_capacity(1),
            upto: 0,
            count: 0,
            pos_incr: 1,
            last_end_offset: 0,
            last_pos_length: 0,
        }
    }

    /// Equivalent to `PendingOutputs.reset()`.
    fn reset(&mut self) {
        self.upto = 0;
        self.count = 0;
        self.pos_incr = 1;
    }

    /// Equivalent to `PendingOutputs.pullNext()`.
    fn pull_next(&mut self) -> CharsRef {
        debug_assert!(self.upto < self.count);
        self.last_end_offset = self.end_offsets[self.upto];
        self.last_pos_length = self.pos_lengths[self.upto];
        let result = self.outputs[self.upto].get();
        self.upto += 1;
        self.pos_incr = 0;
        if self.upto == self.count {
            self.reset();
        }
        result
    }

    /// Equivalent to `getLastEndOffset()`.
    fn get_last_end_offset(&self) -> i32 {
        self.last_end_offset
    }

    /// Equivalent to `getLastPosLength()`.
    fn get_last_pos_length(&self) -> i32 {
        self.last_pos_length
    }

    /// Equivalent to `PendingOutputs.add(char[], int, int, int, int)`; the
    /// slots past `count` are reused, as Java's grown arrays are.
    fn add(&mut self, output: &[u16], offset: usize, len: usize, end_offset: i32, pos_length: i32) {
        if self.count == self.outputs.len() {
            self.outputs.push(CharsRefBuilder::new());
            self.end_offsets.push(0);
            self.pos_lengths.push(0);
        }
        self.outputs[self.count].copy_chars(output, offset, len);
        // endOffset can be -1, in which case we should simply
        // use the endOffset of the input token, or X >= 0, in
        // which case we use X as the endOffset for this output
        self.end_offsets[self.count] = end_offset;
        self.pos_lengths[self.count] = pos_length;
        self.count += 1;
    }
}

/// The body of [`SynonymFilter`]: what Lucene's `SynonymFilter` adds to
/// `TokenFilter`.
pub struct SynonymFilterLogic {
    synonyms: Arc<SynonymMap>,
    ignore_case: bool,
    roll_buffer_size: usize,
    capture_count: i32,
    /// How many future input tokens have already been matched to a synonym;
    /// because the matching is "greedy" we don't try to do any more matching
    /// for such tokens.
    input_skip_count: usize,
    /// Rolling buffer, holding pending input tokens we had to clone because
    /// we needed to look ahead, indexed by position.
    future_inputs: Vec<PendingInput>,
    /// Rolling buffer, holding stack of pending synonym outputs, indexed by
    /// position.
    future_outputs: Vec<PendingOutputs>,
    /// Where (in rolling buffers) to write next input saved state.
    next_write: usize,
    /// Where (in rolling buffers) to read next input saved state.
    next_read: usize,
    /// True once we've read last token.
    finished: bool,
    scratch_arc: FSTArc<BytesRef>,
    scratch_chars: CharsRefBuilder,
    last_start_offset: i32,
    last_end_offset: i32,
}

impl std::fmt::Debug for SynonymFilterLogic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SynonymFilterLogic")
            .field("ignore_case", &self.ignore_case)
            .field("roll_buffer_size", &self.roll_buffer_size)
            .field("input_skip_count", &self.input_skip_count)
            .field("next_write", &self.next_write)
            .field("next_read", &self.next_read)
            .field("finished", &self.finished)
            .finish_non_exhaustive()
    }
}

/// The `CharTermAttribute`'s UTF-16 units, Java's `termAtt.buffer()` up to
/// `termAtt.length()`.
fn term_units(source: &AttributeSource) -> Vec<u16> {
    let term_att = source
        .get_attribute_interface::<dyn CharTermAttribute>()
        .expect("INVARIANT: the constructor added the CharTermAttribute");
    chars_to_utf16(&term_att.buffer()[..term_att.length()])
}

/// `termAtt.copyBuffer(output.chars, output.offset, output.length)`.
fn set_term(source: &AttributeSource, output: &CharsRef) {
    let chars = utf16_to_term_chars(&output.chars[output.offset..output.offset + output.length]);
    source
        .get_attribute_interface_mut::<dyn CharTermAttribute>()
        .expect("INVARIANT: the constructor added the CharTermAttribute")
        .copy_buffer(&chars, 0, chars.len());
}

/// `typeAtt.setType(type)`.
fn set_type(source: &AttributeSource, type_value: &str) {
    source
        .get_attribute_interface_mut::<dyn TypeAttribute>()
        .expect("INVARIANT: the constructor added the TypeAttribute")
        .set_type(type_value.to_string());
}

/// `offsetAtt.setOffset(start, end)`.
fn set_offset(source: &AttributeSource, start_offset: i32, end_offset: i32) {
    source
        .get_attribute_interface_mut::<dyn OffsetAttribute>()
        .expect("INVARIANT: the constructor added the OffsetAttribute")
        .set_offset(start_offset, end_offset);
}

/// `posIncrAtt.setPositionIncrement(pos_incr)`.
fn set_position_increment(source: &AttributeSource, pos_incr: i32) {
    source
        .get_attribute_interface_mut::<dyn PositionIncrementAttribute>()
        .expect("INVARIANT: the constructor added the PositionIncrementAttribute")
        .set_position_increment(pos_incr);
}

impl SynonymFilterLogic {
    /// Equivalent to `SynonymFilter.TYPE_SYNONYM`.
    pub const TYPE_SYNONYM: &'static str = "SYNONYM";

    /// Equivalent to `capture()`.
    fn capture(&mut self, source: &AttributeSource) {
        self.capture_count += 1;
        let state = source.capture_state();
        let term = term_units(source);
        let input = &mut self.future_inputs[self.next_write];

        input.state = state;
        input.consumed = false;
        input.term.copy_chars(&term, 0, term.len());

        self.next_write = self.roll_incr(self.next_write);

        // Buffer head should never catch up to tail:
        debug_assert!(self.next_write != self.next_read);
    }

    /// This is the core of this TokenFilter: it locates the synonym matches
    /// and buffers up the results into futureInputs/Outputs.
    ///
    /// NOTE: this calls input.incrementToken and does not capture the state
    /// if no further tokens were checked. So caller must then forward state
    /// to our caller, or capture.
    ///
    /// Equivalent to `parse()`.
    fn parse(&mut self, source: &AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        debug_assert!(self.input_skip_count == 0);

        let mut cur_next_read = self.next_read;

        // Holds the longest match we've seen so far:
        let mut match_output: Option<BytesRef> = None;
        let mut match_input_length = 0;
        let mut match_end_offset = -1;

        let synonyms = Arc::clone(&self.synonyms);
        let fst = synonyms
            .fst
            .as_ref()
            .expect("INVARIANT: the constructor checked the fst");
        let outputs = fst.outputs();
        // Java keeps the `fstReader` of its constructor; a reader borrows
        // the FST here and cannot live in the filter next to the map it
        // reads, so a reader is taken for each parse (its position is set by
        // every arc lookup, so a fresh one reads the same bytes).
        let mut fst_reader = fst.get_bytes_reader()?;
        let mut pending_output = outputs.no_output();
        fst.get_first_arc(&mut self.scratch_arc);

        debug_assert!(*self.scratch_arc.output() == outputs.no_output());

        let mut token_count = 0;

        'by_token: loop {
            // Pull next token's chars:
            let buffer: Vec<u16>;
            let input_end_offset;

            if cur_next_read == self.next_write {
                // We used up our lookahead buffer of input tokens
                // -- pull next real input token:

                if self.finished {
                    break;
                } else {
                    debug_assert!(self.future_inputs[self.next_write].consumed);
                    // Not correct: a syn match whose output is longer
                    // than its input can set future inputs keepOrig
                    // to true:
                    // assert !futureInputs[nextWrite].keepOrig;
                    if input.increment_token()? {
                        buffer = term_units(source);
                        let (start_offset, end_offset) = {
                            let offset_att = source
                                .get_attribute_interface::<dyn OffsetAttribute>()
                                .expect("INVARIANT: the constructor added the OffsetAttribute");
                            (offset_att.start_offset(), offset_att.end_offset())
                        };
                        let pending = &mut self.future_inputs[self.next_write];
                        pending.start_offset = start_offset;
                        self.last_start_offset = start_offset;
                        pending.end_offset = end_offset;
                        self.last_end_offset = end_offset;
                        input_end_offset = end_offset;
                        if self.next_read != self.next_write {
                            self.capture(source);
                        } else {
                            self.future_inputs[self.next_write].consumed = false;
                        }
                    } else {
                        // No more input tokens
                        self.finished = true;
                        break;
                    }
                }
            } else {
                // Still in our lookahead
                let pending = &self.future_inputs[cur_next_read];
                buffer = pending.term.chars()[..pending.term.length()].to_vec();
                input_end_offset = pending.end_offset;
            }

            token_count += 1;

            // Run each char in this token through the FST:
            let buffer_len = buffer.len();
            let mut buf_upto = 0;
            while buf_upto < buffer_len {
                let code_point = code_point_at(&buffer, buf_upto, buffer_len);
                let label = if self.ignore_case {
                    case_folding::to_lower_case(code_point)
                } else {
                    code_point
                };
                if !fst.find_target_arc_in_place(
                    label,
                    &mut self.scratch_arc,
                    fst_reader.as_mut(),
                )? {
                    break 'by_token;
                }

                // Accum the output
                pending_output = outputs.add(&pending_output, self.scratch_arc.output());
                buf_upto += char_count(code_point);
            }

            // OK, entire token matched; now see if this is a final
            // state:
            if self.scratch_arc.is_final() {
                match_output =
                    Some(outputs.add(&pending_output, self.scratch_arc.next_final_output()));
                match_input_length = token_count;
                match_end_offset = input_end_offset;
            }

            // See if the FST wants to continue matching (ie, needs to
            // see the next input token):
            if !fst.find_target_arc_in_place(
                i32::from(SynonymMap::WORD_SEPARATOR),
                &mut self.scratch_arc,
                fst_reader.as_mut(),
            )? {
                // No further rules can match here; we're done
                // searching for matching rules starting at the
                // current input position.
                break;
            } else {
                // More matching is possible -- accum the output (if
                // any) of the WORD_SEP arc:
                pending_output = outputs.add(&pending_output, self.scratch_arc.output());
                if self.next_read == self.next_write {
                    self.capture(source);
                }
            }

            cur_next_read = self.roll_incr(cur_next_read);
        }

        if self.next_read == self.next_write && !self.finished {
            self.next_write = self.roll_incr(self.next_write);
        }

        if let Some(match_output) = match_output {
            self.input_skip_count = match_input_length;
            self.add_output(&match_output, match_input_length, match_end_offset)?;
        } else if self.next_read != self.next_write {
            // Even though we had no match here, we set to 1
            // because we need to skip current input token before
            // trying to match again:
            self.input_skip_count = 1;
        } else {
            debug_assert!(self.finished);
        }
        Ok(())
    }

    /// Interleaves all output tokens onto the futureOutputs.
    ///
    /// Equivalent to `addOutput(BytesRef, int, int)`.
    fn add_output(
        &mut self,
        bytes: &BytesRef,
        match_input_length: usize,
        match_end_offset: i32,
    ) -> Result<()> {
        let mut bytes_reader =
            ByteArrayDataInput::with_range(&bytes.bytes, bytes.offset, bytes.length);

        let code = bytes_reader.read_v_int()?;
        let keep_orig = (code & 0x1) == 0;
        let count = (code as u32) >> 1;
        for _output_idx in 0..count {
            let ord = bytes_reader.read_v_int()?;
            {
                let words = read_words(&self.synonyms.words);
                let scratch_bytes = words.get(ord);
                self.scratch_chars
                    .copy_utf8_bytes(scratch_bytes, 0, scratch_bytes.len());
            }
            let mut last_start = 0;
            let ch_end = last_start + self.scratch_chars.length();
            let mut output_upto = self.next_read;
            // `for (int chIDX = lastStart; ...)`, from the initial `lastStart`, 0
            for ch_idx in 0..=ch_end {
                if ch_idx == ch_end
                    || self.scratch_chars.char_at(ch_idx) == SynonymMap::WORD_SEPARATOR
                {
                    let output_len = ch_idx - last_start;
                    // Caller is not allowed to have empty string in
                    // the output:
                    debug_assert!(output_len > 0, "output contains empty string");
                    let (end_offset, pos_len) = if ch_idx == ch_end && last_start == 0 {
                        // This rule had a single output token, so, we set
                        // this output's endOffset to the current
                        // endOffset (ie, endOffset of the last input
                        // token it matched):
                        (
                            match_end_offset,
                            if keep_orig {
                                match_input_length as i32
                            } else {
                                1
                            },
                        )
                    } else {
                        // This rule has more than one output token; we
                        // can't pick any particular endOffset for this
                        // case, so, we inherit the endOffset for the
                        // input token which this output overlaps:
                        (-1, 1)
                    };
                    self.future_outputs[output_upto].add(
                        self.scratch_chars.chars(),
                        last_start,
                        output_len,
                        end_offset,
                        pos_len,
                    );
                    last_start = 1 + ch_idx;
                    output_upto = self.roll_incr(output_upto);
                    debug_assert!(
                        self.future_outputs[output_upto].pos_incr == 1,
                        "outputUpto={output_upto} vs nextWrite={}",
                        self.next_write
                    );
                }
            }
        }

        let mut upto = self.next_read;
        for _idx in 0..match_input_length {
            self.future_inputs[upto].keep_orig |= keep_orig;
            self.future_inputs[upto].matched = true;
            upto = self.roll_incr(upto);
        }
        Ok(())
    }

    /// ++ mod rollBufferSize
    ///
    /// Equivalent to `rollIncr(int)`.
    fn roll_incr(&self, mut count: usize) -> usize {
        count += 1;
        if count == self.roll_buffer_size {
            0
        } else {
            count
        }
    }

    /// for testing
    ///
    /// Equivalent to the package-private `getCaptureCount()`.
    pub fn get_capture_count(&self) -> i32 {
        self.capture_count
    }
}

impl TokenFilterLogic for SynonymFilterLogic {
    /// Equivalent to `SynonymFilter.incrementToken()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        loop {
            // First play back any buffered future inputs/outputs
            // w/o running parsing again:
            while self.input_skip_count != 0 {
                // At each position, we first output the original
                // token

                // TODO: maybe just a PendingState class, holding
                // both input & outputs?
                let next_read = self.next_read;
                let pending_input = &self.future_inputs[next_read];

                if !pending_input.consumed && (pending_input.keep_orig || !pending_input.matched) {
                    if let Some(state) = &pending_input.state {
                        // Return a previously saved token (because we
                        // had to lookahead):
                        source.restore_state(state)?;
                    } else {
                        // Pass-through case: return token we just pulled
                        // but didn't capture:
                        debug_assert!(
                            self.input_skip_count == 1,
                            "inputSkipCount={} nextRead={}",
                            self.input_skip_count,
                            self.next_read
                        );
                    }
                    self.future_inputs[next_read].reset();
                    if self.future_outputs[next_read].count > 0 {
                        self.future_outputs[next_read].pos_incr = 0;
                    } else {
                        self.next_read = self.roll_incr(self.next_read);
                        self.input_skip_count -= 1;
                    }
                    return Ok(true);
                } else if self.future_outputs[next_read].upto < self.future_outputs[next_read].count
                {
                    // Still have pending outputs to replay at this
                    // position
                    self.future_inputs[next_read].reset();
                    let outputs = &mut self.future_outputs[next_read];
                    let pos_incr = outputs.pos_incr;
                    let output = outputs.pull_next();
                    let last_end_offset = outputs.get_last_end_offset();
                    let last_pos_length = outputs.get_last_pos_length();
                    let outputs_count = outputs.count;
                    source.clear_attributes();
                    set_term(source, &output);
                    set_type(source, Self::TYPE_SYNONYM);
                    let pending_input = &self.future_inputs[next_read];
                    let mut end_offset = last_end_offset;
                    if end_offset == -1 {
                        end_offset = pending_input.end_offset;
                    }
                    set_offset(source, pending_input.start_offset, end_offset);
                    set_position_increment(source, pos_incr);
                    source
                        .get_attribute_interface_mut::<dyn PositionLengthAttribute>()
                        .expect("INVARIANT: the constructor added the PositionLengthAttribute")
                        .set_position_length(last_pos_length);
                    if outputs_count == 0 {
                        // Done with the buffered input and all outputs at
                        // this position
                        self.next_read = self.roll_incr(self.next_read);
                        self.input_skip_count -= 1;
                    }
                    return Ok(true);
                } else {
                    // Done with the buffered input and all outputs at
                    // this position
                    self.future_inputs[next_read].reset();
                    self.next_read = self.roll_incr(self.next_read);
                    self.input_skip_count -= 1;
                }
            }

            if self.finished && self.next_read == self.next_write {
                // End case: if any output syns went beyond end of
                // input stream, enumerate them now:
                let next_read = self.next_read;
                if self.future_outputs[next_read].upto < self.future_outputs[next_read].count {
                    let outputs = &mut self.future_outputs[next_read];
                    let pos_incr = outputs.pos_incr;
                    let output = outputs.pull_next();
                    let outputs_count = outputs.count;
                    self.future_inputs[next_read].reset();
                    if outputs_count == 0 {
                        self.next_read = self.roll_incr(next_read);
                        self.next_write = self.next_read;
                    }
                    source.clear_attributes();
                    // Keep offset from last input token:
                    set_offset(source, self.last_start_offset, self.last_end_offset);
                    set_term(source, &output);
                    set_type(source, Self::TYPE_SYNONYM);
                    set_position_increment(source, pos_incr);
                    return Ok(true);
                } else {
                    return Ok(false);
                }
            }

            // Find new synonym matches:
            self.parse(source, input)?;
        }
    }

    /// Equivalent to `SynonymFilter.reset()`.
    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.capture_count = 0;
        self.finished = false;
        self.input_skip_count = 0;
        self.next_read = 0;
        self.next_write = 0;

        // In normal usage these resets would not be needed,
        // since they reset-as-they-are-consumed, but the app
        // may not consume all input tokens (or we might hit an
        // exception), in which case we have leftover state
        // here:
        for pending_input in &mut self.future_inputs {
            pending_input.reset();
        }
        for output in &mut self.future_outputs {
            output.reset();
        }
        Ok(())
    }
}

/// Matches single or multi word synonyms in a token stream. This token
/// stream cannot properly handle position increments != 1, ie, you should
/// place this filter before filtering out stop words.
///
/// Note that with the current implementation, parsing is greedy, so
/// whenever multiple parses would apply, the rule starting the earliest and
/// parsing the most tokens wins. For example if you have these rules:
///
/// ```text
///   a -> x
///   a b -> y
///   b c d -> z
/// ```
///
/// Then input `a b c d e` parses to `y b c d`, ie the 2nd rule "wins"
/// because it started earliest and matched the most input tokens of other
/// rules starting at that point.
///
/// A future improvement to this filter could allow non-greedy parsing, such
/// that the 3rd rule would win, and also separately allow multiple parses,
/// such that all 3 rules would match, perhaps even on a rule by rule basis.
///
/// **NOTE**: when a match occurs, the output tokens associated with the
/// matching rule are "stacked" on top of the input stream (if the rule had
/// `keepOrig=true`) and also on top of another matched rule's output tokens.
/// This is not a correct solution, as really the output should be an
/// arbitrary graph/lattice. For example, with the above match, you would
/// expect an exact `PhraseQuery` `"y b c"` to match the parsed tokens, but
/// it will fail to do so. This limitation is necessary because Lucene's
/// TokenStream (and index) cannot yet represent an arbitrary graph.
///
/// **NOTE**: If multiple incoming tokens arrive on the same position, only
/// the first token at that position is used for parsing. Subsequent tokens
/// simply pass through and are not parsed. A future improvement would be to
/// allow these tokens to also be matched.
///
/// Equivalent to `org.apache.lucene.analysis.synonym.SynonymFilter`, which
/// Lucene deprecates: use `SynonymGraphFilter` instead, but be sure to also
/// use `FlattenGraphFilter` at index time (not at search time) as well.
pub type SynonymFilter = TokenFilter<SynonymFilterLogic>;

/// Creates the filter: `ignore_case` case-folds input for matching with
/// `Character.toLowerCase(int)`. Note, if you set this to true, it's your
/// responsibility to lowercase the input entries when you create the
/// [`SynonymMap`].
///
/// Equivalent to `SynonymFilter(TokenStream, SynonymMap, boolean)`, whose
/// fields add the `CharTermAttribute`, the `PositionIncrementAttribute`,
/// the `PositionLengthAttribute`, the `TypeAttribute` and the
/// `OffsetAttribute`, in that order.
///
/// # Errors
///
/// `IllegalArgumentException("fst must be non-null")` for a map without an
/// FST, what reading the FST raises, and what the stream's attribute
/// factory raises for an interface it cannot implement.
pub fn new_synonym_filter(
    input: Box<dyn TokenStream>,
    synonyms: Arc<SynonymMap>,
    ignore_case: bool,
) -> Result<SynonymFilter> {
    if synonyms.fst.is_none() {
        return Err(LuceneError::illegal_argument("fst must be non-null"));
    }

    // Must be 1+ so that when roll buffer is at full
    // lookahead we can distinguish this full buffer from
    // the empty buffer:
    let roll_buffer_size = 1 + synonyms.max_horizontal_context as usize;

    let mut future_inputs = Vec::with_capacity(roll_buffer_size);
    let mut future_outputs = Vec::with_capacity(roll_buffer_size);
    for _pos in 0..roll_buffer_size {
        future_inputs.push(PendingInput::new());
        future_outputs.push(PendingOutputs::new());
    }

    let mut filter = TokenFilter::new(
        input,
        SynonymFilterLogic {
            synonyms,
            ignore_case,
            roll_buffer_size,
            capture_count: 0,
            input_skip_count: 0,
            future_inputs,
            future_outputs,
            next_write: 0,
            next_read: 0,
            finished: false,
            scratch_arc: FSTArc::default(),
            scratch_chars: CharsRefBuilder::new(),
            last_start_offset: 0,
            last_end_offset: 0,
        },
    );
    let source = filter.attribute_source_mut();
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
    source.add_attribute_interface::<dyn PositionLengthAttribute>()?;
    source.add_attribute_interface::<dyn TypeAttribute>()?;
    source.add_attribute_interface::<dyn OffsetAttribute>()?;
    Ok(filter)
}
