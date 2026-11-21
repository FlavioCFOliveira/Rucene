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

//! Applies single- or multi-token synonyms from a `SynonymMap` to an
//! incoming `TokenStream`, producing a fully correct graph output, ported
//! from `org.apache.lucene.analysis.synonym.SynonymGraphFilter`.

use std::collections::VecDeque;
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
use rucene::util::fst::{Arc as FSTArc, Outputs};
use rucene::util::refs::CharsRefBuilder;
use rucene::util::ring_buffer::{Resettable, RollingBuffer};
use rucene::util::BytesRef;

use super::synonym_map::{read_words, SynonymMap};
use crate::jdk::character::{chars_to_utf16, utf16_to_term_chars};

// TODO: maybe we should resolve token -> wordID then run
// FST on wordIDs, for better perf?

// TODO: a more efficient approach would be Aho/Corasick's
// algorithm
// http://en.wikipedia.org/wiki/Aho%E2%80%93Corasick_string_matching_algorithm
// It improves over the current approach here
// because it does not fully re-start matching at every
// token.  For example if one pattern is "a b c x"
// and another is "b c d" and the input is "a b c d", on
// trying to parse "a b c x" but failing when you got to x,
// rather than starting over again your really should
// immediately recognize that "b c d" matches at the next
// input.  I suspect this won't matter that much in
// practice, but it's possible on some set of synonyms it
// will.  We'd have to modify Aho/Corasick to enforce our
// conflict resolving (eg greedy matching) because that algo
// finds all matches.  This really amounts to adding a .*
// closure to the FST and then determinizing it.
//
// Another possible solution is described at
// http://www.cis.uni-muenchen.de/people/Schulz/Pub/dictle5.ps

/// An input token buffered for lookahead.
///
/// Equivalent to `org.apache.lucene.analysis.synonym.SynonymGraphFilter$BufferedInputToken`,
/// which implements `RollingBuffer.Resettable`.
#[derive(Debug)]
pub(crate) struct BufferedInputToken {
    term: CharsRefBuilder,
    /// Java's `AttributeSource.State`, shared with the output tokens that
    /// replay it, as Java shares the object.
    state: Option<Arc<CapturedState>>,
    start_offset: i32,
    end_offset: i32,
}

impl BufferedInputToken {
    fn new() -> Self {
        Self {
            term: CharsRefBuilder::new(),
            state: None,
            start_offset: -1,
            end_offset: -1,
        }
    }
}

impl Resettable for BufferedInputToken {
    /// Equivalent to `BufferedInputToken.reset()`.
    fn reset(&mut self) {
        self.state = None;
        self.term.clear();

        // Intentionally invalid to ferret out bugs:
        self.start_offset = -1;
        self.end_offset = -1;
    }
}

/// A token waiting to be returned; `state` is non-null if this was an
/// incoming token.
///
/// Equivalent to the record `org.apache.lucene.analysis.synonym.SynonymGraphFilter$BufferedOutputToken`;
/// `term` holds the UTF-16 units of Java's `String`.
#[derive(Debug, Clone)]
pub(crate) struct BufferedOutputToken {
    state: Option<Arc<CapturedState>>,
    term: Vec<u16>,
    start_node: i32,
    end_node: i32,
}

/// The body of [`SynonymGraphFilter`]: what Lucene's `SynonymGraphFilter`
/// adds to `TokenFilter`.
pub struct SynonymGraphFilterLogic {
    synonyms: Arc<SynonymMap>,
    ignore_case: bool,
    scratch_arc: FSTArc<BytesRef>,
    scratch_chars: CharsRefBuilder,
    output_buffer: VecDeque<BufferedOutputToken>,

    next_node_out: i32,
    last_node_out: i32,
    max_lookahead_used: usize,

    // For testing:
    capture_count: i32,

    live_token: bool,

    // Start/end offset of the current match:
    match_start_offset: i32,
    match_end_offset: i32,

    // True once the input TokenStream is exhausted:
    finished: bool,

    lookahead_next_read: i32,
    lookahead_next_write: i32,

    lookahead: RollingBuffer<BufferedInputToken, fn() -> BufferedInputToken>,
}

impl std::fmt::Debug for SynonymGraphFilterLogic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SynonymGraphFilterLogic")
            .field("ignore_case", &self.ignore_case)
            .field("output_buffer", &self.output_buffer.len())
            .field("next_node_out", &self.next_node_out)
            .field("last_node_out", &self.last_node_out)
            .field("live_token", &self.live_token)
            .field("finished", &self.finished)
            .field("lookahead_next_read", &self.lookahead_next_read)
            .field("lookahead_next_write", &self.lookahead_next_write)
            .finish_non_exhaustive()
    }
}

impl SynonymGraphFilterLogic {
    /// Equivalent to `SynonymGraphFilter.TYPE_SYNONYM`.
    pub const TYPE_SYNONYM: &'static str = "SYNONYM";

    /// Equivalent to `releaseBufferedToken()`.
    fn release_buffered_token(&mut self, source: &AttributeSource) -> Result<()> {
        let token = self
            .output_buffer
            .pop_front()
            .expect("INVARIANT: called with a buffered token");

        if let Some(state) = &token.state {
            // This is an original input token (keepOrig=true case):
            source.restore_state(state)?;
        } else {
            source.clear_attributes();
            let chars = utf16_to_term_chars(&token.term);
            {
                let mut term_att = source
                    .get_attribute_interface_mut::<dyn CharTermAttribute>()
                    .expect("INVARIANT: the constructor added the CharTermAttribute");
                let length = term_att.length();
                term_att.resize_buffer(length + chars.len());
                term_att.buffer_mut()[length..length + chars.len()].copy_from_slice(&chars);
                term_att.set_length(length + chars.len());
            }

            // We better have a match already:
            debug_assert!(self.match_start_offset != -1);

            source
                .get_attribute_interface_mut::<dyn OffsetAttribute>()
                .expect("INVARIANT: the constructor added the OffsetAttribute")
                .set_offset(self.match_start_offset, self.match_end_offset);
            source
                .get_attribute_interface_mut::<dyn TypeAttribute>()
                .expect("INVARIANT: the constructor added the TypeAttribute")
                .set_type(Self::TYPE_SYNONYM.to_string());
        }

        source
            .get_attribute_interface_mut::<dyn PositionIncrementAttribute>()
            .expect("INVARIANT: the constructor added the PositionIncrementAttribute")
            .set_position_increment(token.start_node - self.last_node_out);
        self.last_node_out = token.start_node;
        source
            .get_attribute_interface_mut::<dyn PositionLengthAttribute>()
            .expect("INVARIANT: the constructor added the PositionLengthAttribute")
            .set_position_length(token.end_node - token.start_node);
        Ok(())
    }

    /// Scans the next input token(s) to see if a synonym matches. Returns
    /// true if a match was found.
    ///
    /// Equivalent to `parse()`.
    fn parse(&mut self, source: &AttributeSource, input: &mut dyn TokenStream) -> Result<bool> {
        // Holds the longest match we've seen so far:
        let mut match_output: Option<BytesRef> = None;
        let mut match_input_length = 0;

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

        // How many tokens in the current match
        let mut match_length = 0;
        let mut do_final_capture = false;

        let mut lookahead_upto = self.lookahead_next_read;
        self.match_start_offset = -1;

        'by_token: loop {
            // Pull next token's chars:
            let buffer: Vec<u16>;
            let input_end_offset: i32;

            if lookahead_upto <= self.lookahead.get_max_pos() {
                // Still in our lookahead buffer
                let token = self.lookahead.get(lookahead_upto);
                lookahead_upto += 1;
                buffer = token.term.chars()[..token.term.length()].to_vec();
                input_end_offset = token.end_offset;
                let token_start_offset = token.start_offset;
                if self.match_start_offset == -1 {
                    self.match_start_offset = token_start_offset;
                }
            } else {
                // We used up our lookahead buffer of input tokens
                // -- pull next real input token:

                debug_assert!(self.finished || !self.live_token);

                if self.finished {
                    break;
                } else if input.increment_token()? {
                    self.live_token = true;
                    {
                        let term_att = source
                            .get_attribute_interface::<dyn CharTermAttribute>()
                            .expect("INVARIANT: the constructor added the CharTermAttribute");
                        buffer = chars_to_utf16(&term_att.buffer()[..term_att.length()]);
                    }
                    let (start_offset, end_offset) = {
                        let offset_att = source
                            .get_attribute_interface::<dyn OffsetAttribute>()
                            .expect("INVARIANT: the constructor added the OffsetAttribute");
                        (offset_att.start_offset(), offset_att.end_offset())
                    };
                    if self.match_start_offset == -1 {
                        self.match_start_offset = start_offset;
                    }
                    input_end_offset = end_offset;

                    lookahead_upto += 1;
                } else {
                    // No more input tokens
                    self.finished = true;
                    break;
                }
            }

            match_length += 1;

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

            debug_assert!(buf_upto == buffer_len);

            // OK, entire token matched; now see if this is a final
            // state in the FST (a match):
            if self.scratch_arc.is_final() {
                match_output =
                    Some(outputs.add(&pending_output, self.scratch_arc.next_final_output()));
                match_input_length = match_length;
                self.match_end_offset = input_end_offset;
            }

            // See if the FST can continue matching (ie, needs to
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
                do_final_capture = true;
                if self.live_token {
                    self.capture(source);
                }
            }
        }

        if do_final_capture && self.live_token && !self.finished {
            // Must capture the final token if we captured any prior tokens:
            self.capture(source);
        }

        if let Some(match_output) = match_output {
            if self.live_token {
                // Single input token synonym; we must buffer it now:
                self.capture(source);
            }

            // There is a match!
            self.buffer_output_tokens(&match_output, match_input_length)?;
            self.lookahead_next_read += match_input_length;
            self.lookahead.free_before(self.lookahead_next_read);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Expands the output graph into the necessary tokens, adding synonyms
    /// as side paths parallel to the input tokens, and buffers them in the
    /// output token buffer.
    ///
    /// Equivalent to `bufferOutputTokens(BytesRef, int)`.
    fn buffer_output_tokens(&mut self, bytes: &BytesRef, match_input_length: i32) -> Result<()> {
        let mut bytes_reader =
            ByteArrayDataInput::with_range(&bytes.bytes, bytes.offset, bytes.length);

        let code = bytes_reader.read_v_int()?;
        let keep_orig = (code & 0x1) == 0;

        // How many nodes along all paths; we need this to assign the
        // node ID for the final end node where all paths merge back:
        let mut total_path_nodes = if keep_orig {
            debug_assert!(match_input_length > 0);
            match_input_length - 1
        } else {
            0
        };

        // How many synonyms we will insert over this match:
        let count = (code as u32) >> 1;

        // TODO: we could encode this instead into the FST:

        // 1st pass: count how many new nodes we need
        let mut paths: Vec<Vec<Vec<u16>>> = Vec::new();
        for _output_idx in 0..count {
            let word_id = bytes_reader.read_v_int()?;
            {
                let words = read_words(&self.synonyms.words);
                let scratch_bytes = words.get(word_id);
                self.scratch_chars
                    .copy_utf8_bytes(scratch_bytes, 0, scratch_bytes.len());
            }
            let mut last_start = 0;

            let mut path: Vec<Vec<u16>> = Vec::new();
            let ch_end = self.scratch_chars.length();
            for ch_upto in 0..=ch_end {
                if ch_upto == ch_end
                    || self.scratch_chars.char_at(ch_upto) == SynonymMap::WORD_SEPARATOR
                {
                    path.push(self.scratch_chars.chars()[last_start..ch_upto].to_vec());
                    last_start = 1 + ch_upto;
                }
            }

            debug_assert!(!path.is_empty());
            total_path_nodes += path.len() as i32 - 1;
            paths.push(path);
        }

        // 2nd pass: buffer tokens for the graph fragment

        // NOTE: totalPathNodes will be 0 in the case where the matched
        // input is a single token and all outputs are also a single token

        // We "spawn" a side-path for each of the outputs for this matched
        // synonym, all ending back at this end node:

        let start_node = self.next_node_out;

        let end_node = start_node + total_path_nodes + 1;

        // First, fanout all tokens departing start node for these new side paths:
        let mut new_node_count = 0;
        for path in &paths {
            let path_end_node = if path.len() == 1 {
                // Single token output, so there are no intermediate nodes:
                end_node
            } else {
                let path_end_node = self.next_node_out + new_node_count + 1;
                new_node_count += path.len() as i32 - 1;
                path_end_node
            };
            self.output_buffer.push_back(BufferedOutputToken {
                state: None,
                term: path[0].clone(),
                start_node,
                end_node: path_end_node,
            });
        }

        // We must do the original tokens last, else the offsets "go backwards":
        if keep_orig {
            let token = self.lookahead.get(self.lookahead_next_read);
            let input_end_node = if match_input_length == 1 {
                // Single token matched input, so there are no intermediate nodes:
                end_node
            } else {
                self.next_node_out + new_node_count + 1
            };

            let output = BufferedOutputToken {
                state: token.state.clone(),
                term: token.term.chars()[..token.term.length()].to_vec(),
                start_node,
                end_node: input_end_node,
            };
            self.output_buffer.push_back(output);
        }

        self.next_node_out = end_node;

        // Do full side-path for each syn output:
        for (path_id, path) in paths.iter().enumerate() {
            if path.len() > 1 {
                let mut last_node = self.output_buffer[path_id].end_node;
                for item in &path[1..path.len() - 1] {
                    self.output_buffer.push_back(BufferedOutputToken {
                        state: None,
                        term: item.clone(),
                        start_node: last_node,
                        end_node: last_node + 1,
                    });
                    last_node += 1;
                }
                self.output_buffer.push_back(BufferedOutputToken {
                    state: None,
                    term: path[path.len() - 1].clone(),
                    start_node: last_node,
                    end_node,
                });
            }
        }

        if keep_orig && match_input_length > 1 {
            // Do full "side path" with the original tokens:
            let mut last_node = self.output_buffer[paths.len()].end_node;
            for i in 1..match_input_length - 1 {
                let token = self.lookahead.get(self.lookahead_next_read + i);
                let output = BufferedOutputToken {
                    state: token.state.clone(),
                    term: token.term.chars()[..token.term.length()].to_vec(),
                    start_node: last_node,
                    end_node: last_node + 1,
                };
                self.output_buffer.push_back(output);
                last_node += 1;
            }
            let token = self
                .lookahead
                .get(self.lookahead_next_read + match_input_length - 1);
            let output = BufferedOutputToken {
                state: token.state.clone(),
                term: token.term.chars()[..token.term.length()].to_vec(),
                start_node: last_node,
                end_node,
            };
            self.output_buffer.push_back(output);
        }
        Ok(())
    }

    /// Buffers the current input token into lookahead buffer.
    ///
    /// Equivalent to `capture()`.
    fn capture(&mut self, source: &AttributeSource) {
        debug_assert!(self.live_token);
        self.live_token = false;
        let state = source.capture_state();
        let (start_offset, end_offset) = {
            let offset_att = source
                .get_attribute_interface::<dyn OffsetAttribute>()
                .expect("INVARIANT: the constructor added the OffsetAttribute");
            (offset_att.start_offset(), offset_att.end_offset())
        };
        let term = {
            let term_att = source
                .get_attribute_interface::<dyn CharTermAttribute>()
                .expect("INVARIANT: the constructor added the CharTermAttribute");
            chars_to_utf16(&term_att.buffer()[..term_att.length()])
        };
        let token = self.lookahead.get(self.lookahead_next_write);
        self.lookahead_next_write += 1;

        token.state = state.map(Arc::new);
        token.start_offset = start_offset;
        token.end_offset = end_offset;
        debug_assert!(token.term.length() == 0);
        token.term.append_chars(&term, 0, term.len());

        self.capture_count += 1;
        self.max_lookahead_used = self
            .max_lookahead_used
            .max(self.lookahead.get_buffer_size());
    }

    /// for testing
    ///
    /// Equivalent to the package-private `getCaptureCount()`.
    pub fn get_capture_count(&self) -> i32 {
        self.capture_count
    }

    /// for testing
    ///
    /// Equivalent to the package-private `getMaxLookaheadUsed()`.
    pub fn get_max_lookahead_used(&self) -> usize {
        self.max_lookahead_used
    }
}

impl TokenFilterLogic for SynonymGraphFilterLogic {
    /// Equivalent to `SynonymGraphFilter.incrementToken()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        debug_assert!(self.last_node_out <= self.next_node_out);

        if !self.output_buffer.is_empty() {
            // We still have pending outputs from a prior synonym match:
            self.release_buffered_token(source)?;
            debug_assert!(!self.live_token);
            return Ok(true);
        }

        // Try to parse a new synonym match at the current token:

        if self.parse(source, input)? {
            // A new match was found:
            self.release_buffered_token(source)?;
            debug_assert!(!self.live_token);
            return Ok(true);
        }

        if self.lookahead_next_read == self.lookahead_next_write {
            // Fast path: parse pulled one token, but it didn't match
            // the start for any synonym, so we now return it "live" w/o having
            // cloned all of its atts:
            if self.finished {
                return Ok(false);
            }

            debug_assert!(self.live_token);
            self.live_token = false;

            // NOTE: no need to change posInc since it's relative, i.e. whatever
            // node our output is upto will just increase by the incoming posInc.
            // We also don't need to change posLen, but only because we cannot
            // consume a graph, so the incoming token can never span a future
            // synonym match.
        } else {
            // We still have buffered lookahead tokens from a previous
            // parse attempt that required lookahead; just replay them now:
            debug_assert!(
                self.lookahead_next_read < self.lookahead_next_write,
                "read={} write={}",
                self.lookahead_next_read,
                self.lookahead_next_write
            );
            let state = self.lookahead.get(self.lookahead_next_read).state.clone();
            self.lookahead_next_read += 1;

            if let Some(state) = &state {
                source.restore_state(state)?;
            }

            self.lookahead.free_before(self.lookahead_next_read);

            debug_assert!(!self.live_token);
        }

        let pos_inc = source
            .get_attribute_interface::<dyn PositionIncrementAttribute>()
            .expect("INVARIANT: the constructor added the PositionIncrementAttribute")
            .get_position_increment();
        self.last_node_out += pos_inc;
        let pos_len = source
            .get_attribute_interface::<dyn PositionLengthAttribute>()
            .expect("INVARIANT: the constructor added the PositionLengthAttribute")
            .get_position_length();
        self.next_node_out = self.last_node_out + pos_len;

        Ok(true)
    }

    /// Equivalent to `SynonymGraphFilter.reset()`.
    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.lookahead.reset();
        self.lookahead_next_write = 0;
        self.lookahead_next_read = 0;
        self.capture_count = 0;
        self.last_node_out = -1;
        self.next_node_out = 0;
        self.match_start_offset = -1;
        self.match_end_offset = -1;
        self.finished = false;
        self.live_token = false;
        self.output_buffer.clear();
        self.max_lookahead_used = 0;
        Ok(())
    }
}

/// Applies single- or multi-token synonyms from a [`SynonymMap`] to an
/// incoming `TokenStream`, producing a fully correct graph output. This is a
/// replacement for `SynonymFilter`, which produces incorrect graphs for
/// multi-token synonyms.
///
/// However, if you use this during indexing, you must follow it with
/// `FlattenGraphFilter` to squash tokens on top of one another like
/// `SynonymFilter`, because the indexer can't directly consume a graph. To
/// get fully correct positional queries when your synonym replacements are
/// multiple tokens, you should instead apply synonyms using this
/// `TokenFilter` at query time and translate the resulting graph to a
/// `TermAutomatonQuery` e.g. using `TokenStreamToTermAutomatonQuery`.
///
/// **NOTE**: this cannot consume an incoming graph; results will be
/// undefined.
///
/// Equivalent to `org.apache.lucene.analysis.synonym.SynonymGraphFilter`
/// (`@lucene.experimental`).
pub type SynonymGraphFilter = TokenFilter<SynonymGraphFilterLogic>;

/// Apply previously built synonyms to incoming tokens: `ignore_case`
/// case-folds input for matching with `Character.toLowerCase(int)`. Note,
/// if you set this to true, it's your responsibility to lowercase the input
/// entries when you create the [`SynonymMap`].
///
/// Equivalent to `SynonymGraphFilter(TokenStream, SynonymMap, boolean)`,
/// whose fields add the `CharTermAttribute`, the
/// `PositionIncrementAttribute`, the `PositionLengthAttribute`, the
/// `TypeAttribute` and the `OffsetAttribute`, in that order.
///
/// # Errors
///
/// `IllegalArgumentException("fst must be non-null")` for a map without an
/// FST, what reading the FST raises, and what the stream's attribute
/// factory raises for an interface it cannot implement.
pub fn new_synonym_graph_filter(
    input: Box<dyn TokenStream>,
    synonyms: Arc<SynonymMap>,
    ignore_case: bool,
) -> Result<SynonymGraphFilter> {
    if synonyms.fst.is_none() {
        return Err(LuceneError::illegal_argument("fst must be non-null"));
    }
    let mut filter = TokenFilter::new(
        input,
        SynonymGraphFilterLogic {
            synonyms,
            ignore_case,
            scratch_arc: FSTArc::default(),
            scratch_chars: CharsRefBuilder::new(),
            output_buffer: VecDeque::new(),
            next_node_out: 0,
            last_node_out: 0,
            max_lookahead_used: 0,
            capture_count: 0,
            live_token: false,
            match_start_offset: 0,
            match_end_offset: 0,
            finished: false,
            lookahead_next_read: 0,
            lookahead_next_write: 0,
            lookahead: RollingBuffer::new(BufferedInputToken::new as fn() -> BufferedInputToken),
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
