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

//! The types declared in `Viterbi.java`, of the Java package
//! `org.apache.lucene.analysis.morph`.

use std::sync::Arc;

use rucene::analysis::CharReader;
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::util::fst::Arc as FstArc;
use rucene::util::{ArrayUtil, IntsRef, RamUsageEstimator};

use super::binary_dictionary::BinaryDictionarySubclass;
use super::connection_costs::ConnectionCosts;
use super::dictionary::Dictionary;
use super::morph_data::MorphData;
use super::token::TokenSubclass;
use super::token_info_fst::TokenInfoFST;
use super::token_type::TokenType;
use crate::analysis::util::rolling_char_buffer::RollingCharBuffer;
use crate::jdk::character::{get_type, SPACE_SEPARATOR};

/// Equivalent to `Viterbi.VERBOSE`. It is `false`, so the port omits the
/// `if (VERBOSE)` tracing blocks, which never run.
pub const VERBOSE: bool = false;

/// For safety:
///
/// Equivalent to `Viterbi.MAX_UNKNOWN_WORD_LENGTH`.
pub const MAX_UNKNOWN_WORD_LENGTH: i32 = 1024;

/// Equivalent to the private `Viterbi.MAX_BACKTRACE_GAP`.
const MAX_BACKTRACE_GAP: i32 = 1024;

/// `new UnsupportedOperationException()`: no message.
pub(crate) fn unsupported_operation() -> LuceneError {
    LuceneError::from(LuceneErrorKind::Unchecked {
        class: "java.lang.UnsupportedOperationException",
        message: None,
        cause: None,
    })
}

/// Performs [Viterbi algorithm](https://en.wikipedia.org/wiki/Viterbi_algorithm)
/// for morphological Tokenizers, which split texts by Hidden Markov Model or
/// Conditional Random Fields.
///
/// `T` is the output token class, `U` the position class.
///
/// Equivalent to `org.apache.lucene.analysis.morph.Viterbi<T extends Token, U
/// extends Viterbi.Position>`, an abstract class: this struct holds its
/// fields, and a concrete tokenizer holds a `Viterbi` and implements
/// [`ViterbiSubclass`], whose provided [`forward`](ViterbiSubclass::forward)
/// and [`add`](ViterbiSubclass::add) are the final methods.
///
/// **Divergences.**
///
/// * The `RollingCharBuffer` port does not keep its reader (see
///   [`RollingCharBuffer`]): [`reset_buffer`](Self::reset_buffer) takes no
///   reader, and the methods that read the buffer take the tokenizer's
///   reader, the one Java's `resetBuffer(Reader)` would have kept.
/// * An FST `BytesReader` borrows its FST and is neither `Send` nor `Sync`,
///   so it cannot be a field of a tokenizer: the constructor takes no
///   `fstReader`/`userFSTReader`, and `forward` opens one reader per FST
///   for its call (a reader is repositioned by every `findTargetArc`, so
///   the arcs found are the same). The scratch arc is local to `forward`
///   for the same reason (each use starts with `getFirstArc`).
/// * A `Position` is passed to the hooks by its absolute position (a
///   `Position`'s `pos`), not by reference, since it lives in the
///   position array the hooks may grow.
pub struct Viterbi<T: TokenSubclass, U: ViterbiPosition> {
    fst: Arc<TokenInfoFST>,
    dictionary: Arc<dyn BinaryDictionarySubclass<dyn MorphData>>,
    user_dictionary: Option<Arc<dyn Dictionary<dyn MorphData>>>,
    /// Equivalent to the protected `Viterbi.costs`.
    pub costs: Arc<ConnectionCosts>,
    /// Equivalent to the protected `Viterbi.wordIdRef`.
    pub word_id_ref: IntsRef,
    user_fst: Option<Arc<TokenInfoFST>>,
    /// Equivalent to the protected `Viterbi.buffer`.
    pub buffer: RollingCharBuffer,
    /// Equivalent to the protected `Viterbi.positions`.
    pub positions: WrappedPositionArray<U>,
    /// True once we've hit the EOF from the input reader:
    ///
    /// Equivalent to the protected `Viterbi.end`.
    pub end: bool,
    /// Last absolute position we backtraced from:
    ///
    /// Equivalent to the protected `Viterbi.lastBackTracePos`.
    pub last_back_trace_pos: i32,
    /// Next absolute position to process:
    ///
    /// Equivalent to the protected `Viterbi.pos`.
    pub pos: i32,
    /// Already parsed, but not yet passed to caller, tokens:
    ///
    /// Equivalent to the protected `Viterbi.pending`.
    pub pending: Vec<T>,
    /// Equivalent to the protected `Viterbi.outputNBest`.
    pub output_nbest: bool,
    /// Equivalent to the protected `Viterbi.enableSpacePenaltyFactor`.
    pub enable_space_penalty_factor: bool,
    /// Equivalent to the protected `Viterbi.outputLongestUserEntryOnly`.
    pub output_longest_user_entry_only: bool,
}

impl<T: TokenSubclass, U: ViterbiPosition> std::fmt::Debug for Viterbi<T, U> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Viterbi")
            .field("end", &self.end)
            .field("last_back_trace_pos", &self.last_back_trace_pos)
            .field("pos", &self.pos)
            .field("pending", &self.pending.len())
            .field("output_nbest", &self.output_nbest)
            .finish_non_exhaustive()
    }
}

impl<T: TokenSubclass, U: ViterbiPosition> Viterbi<T, U> {
    /// Equivalent to the protected `Viterbi(TokenInfoFST fst, FST.BytesReader
    /// fstReader, BinaryDictionary<? extends MorphData> dictionary,
    /// TokenInfoFST userFST, FST.BytesReader userFSTReader, Dictionary<?
    /// extends MorphData> userDictionary, ConnectionCosts costs, Class<U>
    /// positionImpl)`, without the readers (see the type's documentation);
    /// `None` stands for a `null` user FST or dictionary, and `U`'s
    /// [`Default`] for its no-arg constructor.
    pub fn new(
        fst: Arc<TokenInfoFST>,
        dictionary: Arc<dyn BinaryDictionarySubclass<dyn MorphData>>,
        user_fst: Option<Arc<TokenInfoFST>>,
        user_dictionary: Option<Arc<dyn Dictionary<dyn MorphData>>>,
        costs: Arc<ConnectionCosts>,
    ) -> Self {
        Self {
            fst,
            dictionary,
            user_fst,
            user_dictionary,
            costs,
            word_id_ref: IntsRef::default(),
            buffer: RollingCharBuffer::new(),
            positions: WrappedPositionArray::new(),
            end: false,
            last_back_trace_pos: 0,
            pos: 0,
            pending: Vec::new(),
            output_nbest: false,
            enable_space_penalty_factor: false,
            output_longest_user_entry_only: false,
        }
    }

    /// Equivalent to `Viterbi.getPos()`.
    pub fn get_pos(&self) -> i32 {
        self.pos
    }

    /// Equivalent to `Viterbi.isEnd()`.
    pub fn is_end(&self) -> bool {
        self.end
    }

    /// Equivalent to `Viterbi.getPending()`.
    pub fn get_pending(&mut self) -> &mut Vec<T> {
        &mut self.pending
    }

    /// Equivalent to `Viterbi.isOutputNBest()`.
    pub fn is_output_nbest(&self) -> bool {
        self.output_nbest
    }

    /// Equivalent to `Viterbi.resetBuffer(Reader)`: the buffer is reset; the
    /// reader is handed to the calls that read (see the type's
    /// documentation).
    pub fn reset_buffer(&mut self) {
        self.buffer.reset();
    }

    /// Equivalent to `Viterbi.resetState()`.
    pub fn reset_state(&mut self) {
        self.positions.reset();
        self.pos = 0;
        self.end = false;
        self.last_back_trace_pos = 0;
        self.pending.clear();

        // Add BOS:
        self.positions
            .get(0)
            .position_mut()
            .add(0, 0, -1, -1, -1, -1, Some(TokenType::Known));
    }
}

/// A concrete Viterbi tokenizer: a subclass of `Viterbi`, with the abstract
/// and overridable methods of `Viterbi` and its final ones.
pub trait ViterbiSubclass<T: TokenSubclass, U: ViterbiPosition> {
    /// The `Viterbi` this tokenizer extends.
    fn viterbi(&self) -> &Viterbi<T, U>;

    /// The `Viterbi` this tokenizer extends, for writing.
    fn viterbi_mut(&mut self) -> &mut Viterbi<T, U>;

    /// Equivalent to the protected `Viterbi.shouldSkipProcessUnknownWord(int,
    /// Position)`; `pos_data` is the position's `pos`.
    fn should_skip_process_unknown_word(
        &mut self,
        unknown_word_end_index: i32,
        pos_data: i32,
    ) -> bool {
        unknown_word_end_index > pos_data
    }

    /// Add unknown words to the position graph; returns the word length.
    ///
    /// Equivalent to the protected abstract `Viterbi.processUnknownWord(boolean,
    /// Position)`; `pos_data` is the position's `pos`.
    ///
    /// # Errors
    ///
    /// What reading the input raises.
    fn process_unknown_word(
        &mut self,
        reader: &mut dyn CharReader,
        any_matches: bool,
        pos_data: i32,
    ) -> Result<i32>;

    /// Backtrace from the provided position, back to the last time we
    /// back-traced, accumulating the resulting tokens to the pending list.
    /// The pending list is then in-reverse (last token should be returned
    /// first).
    ///
    /// Equivalent to the protected abstract `Viterbi.backtrace(Position, int)`;
    /// `end_pos_data` is the position's `pos`.
    ///
    /// # Errors
    ///
    /// What reading the input raises.
    fn backtrace(
        &mut self,
        reader: &mut dyn CharReader,
        end_pos_data: i32,
        from_idx: i32,
    ) -> Result<()>;

    /// Backtrace the n-best path. Subclasses that support n-best paths should
    /// implement this method.
    ///
    /// Equivalent to the protected `Viterbi.backtraceNBest(Position, boolean)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` unless overridden.
    fn backtrace_nbest(
        &mut self,
        _reader: &mut dyn CharReader,
        _end_pos_data: i32,
        _use_eos: bool,
    ) -> Result<()> {
        Err(unsupported_operation())
    }

    /// Remove duplicated tokens from the pending list; this is needed because
    /// `backtrace` and `backtraceNBest` can add same tokens to the list.
    /// Subclasses that support n-best paths should implement this method.
    ///
    /// Equivalent to the protected `Viterbi.fixupPendingList()`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` unless overridden.
    fn fixup_pending_list(&mut self) -> Result<()> {
        Err(unsupported_operation())
    }

    /// Returns the space penalty.
    ///
    /// Equivalent to the protected `Viterbi.computeSpacePenalty(MorphData, int, int)`.
    fn compute_space_penalty(
        &self,
        _morph_data: &dyn MorphData,
        _word_id: i32,
        _num_spaces: i32,
    ) -> i32 {
        0
    }

    /// Returns the penalty for a specific input region
    ///
    /// Equivalent to the protected `Viterbi.computePenalty(int, int)`.
    ///
    /// # Errors
    ///
    /// What reading the input raises.
    fn compute_penalty(
        &mut self,
        _reader: &mut dyn CharReader,
        _pos: i32,
        _length: i32,
    ) -> Result<i32> {
        Ok(0)
    }

    /// Incrementally parse some more characters. This runs the viterbi
    /// search forwards "enough" so that we generate some more tokens. How
    /// much forward depends on the chars coming in, since some chars could
    /// cause longer-lasting ambiguity in the parsing. Once the ambiguity is
    /// resolved, then we back trace, produce the pending tokens, and return.
    ///
    /// Equivalent to the final `Viterbi.forward()`, reading from `reader`.
    ///
    /// # Errors
    ///
    /// What reading the input and the FSTs raises, and what the hooks
    /// return.
    fn forward(&mut self, reader: &mut dyn CharReader) -> Result<()> {
        forward(self, reader)
    }

    /// Add a token on the minimum cost path to the pending token list.
    ///
    /// Equivalent to the final `Viterbi.add(MorphData, Position, int, int,
    /// int, TokenType, boolean)`; `from_pos_data` is the position's `pos`.
    ///
    /// # Errors
    ///
    /// What `computePenalty` returns.
    #[allow(clippy::too_many_arguments)]
    fn add(
        &mut self,
        reader: &mut dyn CharReader,
        morph_data: &dyn MorphData,
        from_pos_data: i32,
        word_pos: i32,
        end_pos: i32,
        word_id: i32,
        token_type: TokenType,
        add_penalty: bool,
    ) -> Result<()> {
        add(
            self,
            reader,
            morph_data,
            from_pos_data,
            word_pos,
            end_pos,
            word_id,
            token_type,
            add_penalty,
        )
    }
}

/// The body of the final `Viterbi.forward()`.
fn forward<T, U, H>(host: &mut H, reader: &mut dyn CharReader) -> Result<()>
where
    T: TokenSubclass,
    U: ViterbiPosition,
    H: ViterbiSubclass<T, U> + ?Sized,
{
    // Index of the last character of unknown word:
    let mut unknown_word_end_index: i32 = -1;

    // Maximum posAhead of user word in the entire input
    let mut user_word_max_pos_ahead: i32 = -1;

    let fst = Arc::clone(&host.viterbi().fst);
    let user_fst = host.viterbi().user_fst.clone();
    let dictionary = Arc::clone(&host.viterbi().dictionary);
    let user_dictionary = host.viterbi().user_dictionary.clone();
    let mut fst_reader = fst.get_bytes_reader()?;
    let mut user_fst_reader = match &user_fst {
        Some(user_fst) => Some(user_fst.get_bytes_reader()?),
        None => None,
    };
    let mut arc: FstArc<i64> = FstArc::default();

    // Advances over each position (character):
    loop {
        let pos = host.viterbi().pos;
        if host.viterbi_mut().buffer.get(pos, reader)? == -1 {
            break;
        }
        // `posData`, the position at `pos` when the step starts:
        let pos_data = pos;
        let (pos_data_count, is_frontier) = {
            let v = host.viterbi_mut();
            let count = v.positions.get(pos).position().count;
            (count, v.positions.get_next_pos() == pos + 1)
        };

        if pos_data_count == 0 {
            // No arcs arrive here; move to next position:
            host.viterbi_mut().pos += 1;
            continue;
        }

        if pos > host.viterbi().last_back_trace_pos && pos_data_count == 1 && is_frontier {
            // We are at a "frontier", and only one node is
            // alive, so whatever the eventual best path is must
            // come through this node.  So we can safely commit
            // to the prefix of the best path at this point:
            if host.viterbi().output_nbest {
                host.backtrace_nbest(reader, pos_data, false)?;
            }
            host.backtrace(reader, pos_data, 0)?;
            if host.viterbi().output_nbest {
                host.fixup_pending_list()?;
            }

            // Re-base cost so we don't risk int overflow:
            let v = host.viterbi_mut();
            v.positions.get(pos_data).position_mut().costs[0] = 0;
            if !v.pending.is_empty() {
                return Ok(());
            } else {
                // This means the backtrace only produced
                // punctuation tokens, so we must keep parsing.
            }
        }

        if pos - host.viterbi().last_back_trace_pos >= MAX_BACKTRACE_GAP {
            // Safety: if we've buffered too much, force a
            // backtrace now.  We find the least-cost partial
            // path, across all paths, backtrace from it, and
            // then prune all others.  Note that this, in
            // general, can produce the wrong result, if the
            // total best path did not in fact back trace
            // through this partial best path.  But it's the
            // best we can do... (short of not having a
            // safety!).

            // First pass: find least cost partial path so far,
            // including ending at future positions:
            let mut least_idx: i32 = -1;
            let mut least_cost = i32::MAX;
            let mut least_pos_data: i32 = -1;
            {
                let v = host.viterbi_mut();
                let next_pos = v.positions.get_next_pos();
                for pos2 in pos..next_pos {
                    let pos_data2 = v.positions.get(pos2).position();
                    for idx in 0..pos_data2.count {
                        let cost = pos_data2.costs[idx as usize];
                        if cost < least_cost {
                            least_cost = cost;
                            least_idx = idx;
                            least_pos_data = pos2;
                        }
                    }
                }
            }

            // We will always have at least one live path:
            debug_assert!(least_idx != -1);

            if host.viterbi().output_nbest {
                host.backtrace_nbest(reader, least_pos_data, false)?;
            }

            // Second pass: prune all but the best path:
            {
                let v = host.viterbi_mut();
                let next_pos = v.positions.get_next_pos();
                for pos2 in pos..next_pos {
                    let pos_data2 = v.positions.get(pos2);
                    if pos2 != least_pos_data {
                        pos_data2.reset();
                    } else {
                        let p = pos_data2.position_mut();
                        if least_idx != 0 {
                            let l = least_idx as usize;
                            p.costs[0] = p.costs[l];
                            p.last_right_id[0] = p.last_right_id[l];
                            p.back_pos[0] = p.back_pos[l];
                            p.back_word_pos[0] = p.back_word_pos[l];
                            p.back_index[0] = p.back_index[l];
                            p.back_id[0] = p.back_id[l];
                            p.back_type[0] = p.back_type[l];
                        }
                        p.count = 1;
                    }
                }
            }

            host.backtrace(reader, least_pos_data, 0)?;
            if host.viterbi().output_nbest {
                host.fixup_pending_list()?;
            }

            // Re-base cost so we don't risk int overflow:
            let v = host.viterbi_mut();
            {
                let p = v.positions.get(least_pos_data).position_mut();
                let count = p.count as usize;
                p.costs[..count].fill(0);
            }

            if v.pos != least_pos_data {
                // We jumped into a future position:
                debug_assert!(v.pos < least_pos_data);
                v.pos = least_pos_data;
            }
            if !v.pending.is_empty() {
                return Ok(());
            } else {
                // This means the backtrace only produced
                // punctuation tokens, so we must keep parsing.
                continue;
            }
        }

        if host.viterbi().enable_space_penalty_factor {
            let v = host.viterbi_mut();
            let ch = v.buffer.get(v.pos, reader)?;
            if get_type(ch) == SPACE_SEPARATOR {
                // We add single space separator as prefixes of the terms that we extract.
                // This information is needed to compute the space penalty factor of each term.
                // These whitespace prefixes are removed when the final tokens are generated, or
                // added as separated tokens when discardPunctuation is unset.
                v.pos += 1;
                if v.buffer.get(v.pos, reader)? == -1 {
                    v.pos = pos_data;
                }
            }
        }

        let mut any_matches = false;
        let pos = host.viterbi().pos;

        // First try user dict:
        if let (Some(user_fst), Some(user_fst_reader)) = (&user_fst, user_fst_reader.as_mut()) {
            let user_dictionary = user_dictionary
                .as_ref()
                .expect("INVARIANT: a user FST comes with its user dictionary");
            user_fst.get_first_arc(&mut arc);
            let mut output: i32 = 0;
            let mut max_pos_ahead: i32 = 0;
            let mut output_max_pos_ahead: i32 = 0;
            let mut arc_final_out_max_pos_ahead: i32 = 0;

            let mut pos_ahead = pos;
            loop {
                let ch = host.viterbi_mut().buffer.get(pos_ahead, reader)?;
                if ch == -1 {
                    break;
                }
                if !user_fst.find_target_arc_in_place(
                    ch,
                    &mut arc,
                    pos_ahead == pos,
                    user_fst_reader.as_mut(),
                )? {
                    break;
                }
                output = output.wrapping_add(*arc.output() as i32);
                if arc.is_final() {
                    max_pos_ahead = pos_ahead;
                    output_max_pos_ahead = output;
                    arc_final_out_max_pos_ahead = *arc.next_final_output() as i32;
                    any_matches = true;
                    if !host.viterbi().output_longest_user_entry_only {
                        // add all matched user entries.
                        host.add(
                            reader,
                            user_dictionary.get_morph_attributes(),
                            pos_data,
                            pos,
                            pos_ahead + 1,
                            output.wrapping_add(*arc.next_final_output() as i32),
                            TokenType::User,
                            false,
                        )?;
                    }
                }
                pos_ahead += 1;
            }

            // Longest matching for user word
            if any_matches && max_pos_ahead > user_word_max_pos_ahead {
                if host.viterbi().output_longest_user_entry_only {
                    host.add(
                        reader,
                        user_dictionary.get_morph_attributes(),
                        pos_data,
                        pos,
                        max_pos_ahead + 1,
                        output_max_pos_ahead.wrapping_add(arc_final_out_max_pos_ahead),
                        TokenType::User,
                        false,
                    )?;
                }
                user_word_max_pos_ahead = user_word_max_pos_ahead.max(max_pos_ahead);
            }
        }

        // TODO: we can be more aggressive about user
        // matches?  if we are "under" a user match then don't
        // extend KNOWN/UNKNOWN paths?

        if !any_matches {
            // Next, try known dictionary matches
            fst.get_first_arc(&mut arc);
            let mut output: i32 = 0;

            let mut pos_ahead = pos;
            loop {
                let ch = host.viterbi_mut().buffer.get(pos_ahead, reader)?;
                if ch == -1 {
                    break;
                }

                if !fst.find_target_arc_in_place(
                    ch,
                    &mut arc,
                    pos_ahead == pos,
                    fst_reader.as_mut(),
                )? {
                    break;
                }

                output = output.wrapping_add(*arc.output() as i32);

                // Optimization: for known words that are too-long
                // (compound), we should pre-compute the 2nd
                // best segmentation and store it in the
                // dictionary instead of recomputing it each time a
                // match is found.

                if arc.is_final() {
                    let mut word_id_ref = std::mem::take(&mut host.viterbi_mut().word_id_ref);
                    dictionary.lookup_word_ids(
                        output.wrapping_add(*arc.next_final_output() as i32),
                        &mut word_id_ref,
                    );
                    let mut result = Ok(());
                    for ofs in 0..word_id_ref.length {
                        if let Err(e) = host.add(
                            reader,
                            dictionary.get_morph_attributes(),
                            pos_data,
                            pos,
                            pos_ahead + 1,
                            word_id_ref.ints[word_id_ref.offset + ofs],
                            TokenType::Known,
                            false,
                        ) {
                            result = Err(e);
                            break;
                        }
                        any_matches = true;
                    }
                    host.viterbi_mut().word_id_ref = word_id_ref;
                    result?;
                }
                pos_ahead += 1;
            }
        }

        if !host.should_skip_process_unknown_word(unknown_word_end_index, pos_data) {
            let unknown_word_length = host.process_unknown_word(reader, any_matches, pos_data)?;
            unknown_word_end_index = pos_data + unknown_word_length;
        }
        host.viterbi_mut().pos += 1;
    }

    let v = host.viterbi_mut();
    v.end = true;

    if v.pos > 0 {
        let end_pos_data = v.pos;
        let mut least_cost = i32::MAX;
        let mut least_idx: i32 = -1;
        {
            let costs = Arc::clone(&v.costs);
            let p = v.positions.get(end_pos_data).position();
            for idx in 0..p.count {
                // Add EOS cost:
                let cost =
                    p.costs[idx as usize].wrapping_add(costs.get(p.last_right_id[idx as usize], 0));
                if cost < least_cost {
                    least_cost = cost;
                    least_idx = idx;
                }
            }
        }

        if host.viterbi().output_nbest {
            host.backtrace_nbest(reader, end_pos_data, true)?;
        }
        host.backtrace(reader, end_pos_data, least_idx)?;
        if host.viterbi().output_nbest {
            host.fixup_pending_list()?;
        }
    } else {
        // No characters in the input string; return no tokens!
    }
    Ok(())
}

/// The body of the final `Viterbi.add(MorphData, Position, int, int, int,
/// TokenType, boolean)`.
#[allow(clippy::too_many_arguments)]
fn add<T, U, H>(
    host: &mut H,
    reader: &mut dyn CharReader,
    morph_data: &dyn MorphData,
    from_pos_data: i32,
    word_pos: i32,
    end_pos: i32,
    word_id: i32,
    token_type: TokenType,
    add_penalty: bool,
) -> Result<()>
where
    T: TokenSubclass,
    U: ViterbiPosition,
    H: ViterbiSubclass<T, U> + ?Sized,
{
    let word_cost = morph_data.get_word_cost(word_id);
    let left_id = morph_data.get_left_id(word_id);
    let mut least_cost = i32::MAX;
    let mut least_idx: i32 = -1;
    let (from_pos, from_count, from_costs, from_last_right_id) = {
        let p = host.viterbi_mut().positions.get(from_pos_data).position();
        let count = p.count as usize;
        (
            p.pos,
            p.count,
            p.costs[..count].to_vec(),
            p.last_right_id[..count].to_vec(),
        )
    };
    debug_assert!(from_count > 0);
    let costs = Arc::clone(&host.viterbi().costs);
    for idx in 0..from_count as usize {
        // The number of spaces before the term
        let num_spaces = word_pos - from_pos;

        // Cost is path cost so far, plus word cost (added at
        // end of loop), plus bigram cost and space penalty cost.
        let cost = from_costs[idx]
            .wrapping_add(costs.get(from_last_right_id[idx], left_id))
            .wrapping_add(host.compute_space_penalty(morph_data, word_id, num_spaces));
        if cost < least_cost {
            least_cost = cost;
            least_idx = idx as i32;
        }
    }

    least_cost = least_cost.wrapping_add(word_cost);

    if add_penalty && token_type != TokenType::User {
        let penalty = host.compute_penalty(reader, from_pos, end_pos - from_pos)?;
        least_cost = least_cost.wrapping_add(penalty);
    }

    let right_id = morph_data.get_right_id(word_id);
    host.viterbi_mut()
        .positions
        .get(end_pos)
        .position_mut()
        .add(
            least_cost,
            right_id,
            from_pos,
            word_pos,
            least_idx,
            word_id,
            Some(token_type),
        );
    Ok(())
}

/// A position class of the Viterbi lattice: `Viterbi.Position` or a
/// subclass, reaching the `Position` it extends.
///
/// Java creates positions with the class's no-arg constructor by
/// reflection; here [`Default`] is that constructor.
pub trait ViterbiPosition: Default + Send + Sync {
    /// The `Position` this position extends.
    fn position(&self) -> &Position;

    /// The `Position` this position extends, for writing.
    fn position_mut(&mut self) -> &mut Position;

    /// Equivalent to `Position.reset()`, which a subclass may override.
    fn reset(&mut self) {
        self.position_mut().reset();
    }
}

/// Holds all back pointers arriving to this position.
///
/// NOTE: This and subclasses must have no-arg constructor. See
/// [`WrappedPositionArray`].
///
/// Equivalent to `org.apache.lucene.analysis.morph.Viterbi$Position`; a
/// `null` back type is `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Position {
    pub(crate) pos: i32,
    pub(crate) count: i32,
    // maybe single int array * 5?
    pub(crate) costs: Vec<i32>,
    pub(crate) last_right_id: Vec<i32>,
    pub(crate) back_pos: Vec<i32>,
    pub(crate) back_word_pos: Vec<i32>,
    pub(crate) back_index: Vec<i32>,
    pub(crate) back_id: Vec<i32>,
    pub(crate) back_type: Vec<Option<TokenType>>,
}

impl Default for Position {
    fn default() -> Self {
        Self {
            pos: 0,
            count: 0,
            costs: vec![0; 8],
            last_right_id: vec![0; 8],
            back_pos: vec![0; 8],
            back_word_pos: vec![0; 8],
            back_index: vec![0; 8],
            back_id: vec![0; 8],
            back_type: vec![None; 8],
        }
    }
}

impl ViterbiPosition for Position {
    fn position(&self) -> &Position {
        self
    }

    fn position_mut(&mut self) -> &mut Position {
        self
    }
}

impl Position {
    /// Equivalent to the private `Position.grow()`.
    fn grow(&mut self) {
        let min = (1 + self.count) as usize;
        self.costs = ArrayUtil::grow_int(&self.costs, min);
        self.last_right_id = ArrayUtil::grow_int(&self.last_right_id, min);
        self.back_pos = ArrayUtil::grow_int(&self.back_pos, min);
        self.back_word_pos = ArrayUtil::grow_int(&self.back_word_pos, min);
        self.back_index = ArrayUtil::grow_int(&self.back_index, min);
        self.back_id = ArrayUtil::grow_int(&self.back_id, min);

        // NOTE: sneaky: grow separately because
        // ArrayUtil.grow will otherwise pick a different
        // length than the int[]s we just grew:
        self.back_type.resize(self.back_id.len(), None);
    }

    /// Equivalent to `Position.add(int cost, int lastRightID, int backPos,
    /// int backRPos, int backIndex, int backID, TokenType backType)`.
    #[allow(clippy::too_many_arguments)]
    pub fn add(
        &mut self,
        cost: i32,
        last_right_id: i32,
        back_pos: i32,
        back_r_pos: i32,
        back_index: i32,
        back_id: i32,
        back_type: Option<TokenType>,
    ) {
        // NOTE: this isn't quite a true Viterbi search,
        // because we should check if lastRightID is
        // already present here, and only update if the new
        // cost is less than the current cost, instead of
        // simply appending.  However, that will likely hurt
        // performance (usually we add a lastRightID only once),
        // and it means we actually create the full graph
        // intersection instead of a "normal" Viterbi lattice:
        if self.count as usize == self.costs.len() {
            self.grow();
        }
        let c = self.count as usize;
        self.costs[c] = cost;
        self.last_right_id[c] = last_right_id;
        self.back_pos[c] = back_pos;
        self.back_word_pos[c] = back_r_pos;
        self.back_index[c] = back_index;
        self.back_id[c] = back_id;
        self.back_type[c] = back_type;
        self.count += 1;
    }

    /// Equivalent to `Position.reset()`.
    pub fn reset(&mut self) {
        self.count = 0;
    }

    /// Equivalent to `Position.getPos()`.
    pub fn get_pos(&self) -> i32 {
        self.pos
    }

    /// Equivalent to `Position.getCount()`.
    pub fn get_count(&self) -> i32 {
        self.count
    }

    /// Equivalent to `Position.setCount(int)`.
    pub fn set_count(&mut self, count: i32) {
        self.count = count;
    }

    /// Equivalent to `Position.getCost(int)`.
    pub fn get_cost(&self, index: i32) -> i32 {
        self.costs[index as usize]
    }

    /// Equivalent to `Position.getBackPos(int)`.
    pub fn get_back_pos(&self, index: i32) -> i32 {
        self.back_pos[index as usize]
    }

    /// Equivalent to `Position.getBackWordPos(int)`.
    pub fn get_back_word_pos(&self, index: i32) -> i32 {
        self.back_word_pos[index as usize]
    }

    /// Equivalent to `Position.getBackID(int)`.
    pub fn get_back_id(&self, index: i32) -> i32 {
        self.back_id[index as usize]
    }

    /// Equivalent to `Position.getBackIndex(int)`.
    pub fn get_back_index(&self, index: i32) -> i32 {
        self.back_index[index as usize]
    }

    /// Equivalent to `Position.getBackType(int)`.
    pub fn get_back_type(&self, index: i32) -> Option<TokenType> {
        self.back_type[index as usize]
    }

    /// Equivalent to `Position.getLastRightID(int)`.
    pub fn get_last_right_id(&self, index: i32) -> i32 {
        self.last_right_id[index as usize]
    }
}

/// Holds partial graph (array of positions) for calculating the minimum
/// cost path
///
/// Equivalent to `org.apache.lucene.analysis.morph.Viterbi$WrappedPositionArray<U
/// extends Position>`; new positions come from `U`'s [`Default`] (Java's
/// reflective no-arg constructor).
#[derive(Debug, Clone)]
pub struct WrappedPositionArray<U: ViterbiPosition> {
    positions: Vec<U>,
    /// Next array index to write to in positions:
    next_write: i32,
    /// Next position to write:
    next_pos: i32,
    /// How many valid Position instances are held in the positions array:
    count: i32,
}

impl<U: ViterbiPosition> Default for WrappedPositionArray<U> {
    fn default() -> Self {
        Self::new()
    }
}

impl<U: ViterbiPosition> WrappedPositionArray<U> {
    /// Equivalent to the package-private `WrappedPositionArray(Class<U>)`.
    pub(crate) fn new() -> Self {
        Self {
            positions: (0..8).map(|_| U::default()).collect(),
            next_write: 0,
            next_pos: 0,
            count: 0,
        }
    }

    /// Equivalent to the package-private `WrappedPositionArray.reset()`.
    pub(crate) fn reset(&mut self) {
        self.next_write -= 1;
        while self.count > 0 {
            if self.next_write == -1 {
                self.next_write = self.positions.len() as i32 - 1;
            }
            self.positions[self.next_write as usize].reset();
            self.next_write -= 1;
            self.count -= 1;
        }
        self.next_write = 0;
        self.next_pos = 0;
        self.count = 0;
    }

    /// Get Position instance for this absolute position; this is allowed to
    /// be arbitrarily far "in the future" but cannot be before the last
    /// freeBefore.
    ///
    /// Equivalent to `WrappedPositionArray.get(int)`.
    pub fn get(&mut self, pos: i32) -> &mut U {
        while pos >= self.next_pos {
            if self.count as usize == self.positions.len() {
                let new_len = ArrayUtil::oversize(
                    1 + self.count as usize,
                    RamUsageEstimator::NUM_BYTES_OBJECT_REF as usize,
                );
                let old_len = self.positions.len();
                let next_write = self.next_write as usize;
                let mut new_positions: Vec<U> = Vec::with_capacity(new_len);
                let mut old = std::mem::take(&mut self.positions);
                let head: Vec<U> = old.drain(..next_write).collect();
                new_positions.extend(old);
                new_positions.extend(head);
                new_positions.extend((old_len..new_len).map(|_| U::default()));
                self.next_write = old_len as i32;
                self.positions = new_positions;
            }
            if self.next_write as usize == self.positions.len() {
                self.next_write = 0;
            }
            // Should have already been reset:
            debug_assert!(self.positions[self.next_write as usize].position().count == 0);
            let next_pos = self.next_pos;
            self.positions[self.next_write as usize].position_mut().pos = next_pos;
            self.next_write += 1;
            self.next_pos += 1;
            self.count += 1;
        }
        debug_assert!(self.in_bounds(pos));
        let index = self.get_index(pos);
        debug_assert!(self.positions[index].position().pos == pos);
        &mut self.positions[index]
    }

    /// Equivalent to the package-private `WrappedPositionArray.getNextPos()`.
    pub(crate) fn get_next_pos(&self) -> i32 {
        self.next_pos
    }

    /// For assert:
    ///
    /// Equivalent to the private `WrappedPositionArray.inBounds(int)`.
    fn in_bounds(&self, pos: i32) -> bool {
        pos < self.next_pos && pos >= self.next_pos - self.count
    }

    /// Equivalent to the private `WrappedPositionArray.getIndex(int)`.
    fn get_index(&self, pos: i32) -> usize {
        let mut index = self.next_write - (self.next_pos - pos);
        if index < 0 {
            index += self.positions.len() as i32;
        }
        index as usize
    }

    /// Equivalent to `WrappedPositionArray.freeBefore(int)`.
    pub fn free_before(&mut self, pos: i32) {
        let to_free = self.count - (self.next_pos - pos);
        debug_assert!(to_free >= 0);
        debug_assert!(to_free <= self.count);
        let mut index = self.next_write - self.count;
        if index < 0 {
            index += self.positions.len() as i32;
        }
        for _ in 0..to_free {
            if index as usize == self.positions.len() {
                index = 0;
            }
            self.positions[index as usize].reset();
            index += 1;
        }
        self.count -= to_free;
    }
}
