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

//! Port of `org.apache.lucene.codecs.blocktreeords.OrdsIntersectTermsEnumFrame`.
//!
//! Java's frame reaches its enum (`ite`) for the enum's input and
//! automaton, which the port hands to each method as an [`IteCtx`], and
//! for its field reader, which `decode_meta_data` receives.

use rucene::codecs::BlockTermState;
use rucene::error::Result;
use rucene::index::IndexOptions;
use rucene::store::IndexInput;
use rucene::util::automaton::{ByteRunnable, Transition, TransitionAccessor};
use rucene::util::fst::Arc as FstArc;

use super::fst_ords_outputs::Output;
use super::ords_block_tree_terms_writer::OUTPUT_FLAG_IS_FLOOR;
use super::ords_field_reader::OrdsFieldReader;
use super::{java_assert, OwnedArrayInput};

/// What a frame reaches through Java's `ite` field.
pub(crate) struct IteCtx<'a> {
    /// `ite.in`.
    pub(crate) input: &'a mut dyn IndexInput,
    /// `ite.byteRunnable`.
    pub(crate) byte_runnable: &'a dyn ByteRunnable,
    /// `ite.transitionAccessor`.
    pub(crate) transition_accessor: &'a dyn TransitionAccessor,
}

/// Port of the package-private `OrdsIntersectTermsEnumFrame`.
// TODO: can we share this with the frame in STE?
pub(crate) struct OrdsIntersectTermsEnumFrame {
    pub(crate) ord: i32,
    pub(crate) fp: i64,
    pub(crate) fp_orig: i64,
    pub(crate) fp_end: i64,
    pub(crate) last_sub_fp: i64,

    // State in automaton
    pub(crate) state: i32,

    pub(crate) meta_data_upto: i32,

    /// `suffixBytes` + `suffixesReader`.
    pub(crate) suffixes: OwnedArrayInput,

    /// `statBytes` + `statsReader`.
    pub(crate) stats: OwnedArrayInput,

    /// `floorData` + `floorDataReader`.
    pub(crate) floor_data: OwnedArrayInput,

    // Length of prefix shared by all terms in this block
    pub(crate) prefix: usize,

    // Number of entries (term or sub-block) in this block
    pub(crate) ent_count: i32,

    // Which term we will next read
    pub(crate) next_ent: i32,

    // Starting termOrd for this frame, used to reset termOrd in rewind()
    pub(crate) term_ord_orig: i64,

    // 1 + ordinal of the current term
    pub(crate) term_ord: i64,

    // True if this block is either not a floor block,
    // or, it's the last sub-block of a floor block
    pub(crate) is_last_in_floor: bool,

    // True if all entries are terms
    pub(crate) is_leaf_block: bool,

    pub(crate) num_follow_floor_blocks: i32,
    pub(crate) next_floor_label: i32,

    pub(crate) transition: Transition,
    pub(crate) cur_transition_max: i32,
    pub(crate) transition_index: i32,
    pub(crate) transition_count: i32,

    /// `arc`, a copy of the arc of `ite.arcs` Java's frame references (a
    /// later walk only ever overwrites the arcs of frames above this one).
    pub(crate) arc: Option<FstArc<Output>>,

    pub(crate) term_state: BlockTermState,

    /// `bytes` + `bytesReader` (the metadata).
    pub(crate) bytes: OwnedArrayInput,

    // Cumulative output so far
    pub(crate) output_prefix: Output,

    pub(crate) start_byte_pos: usize,
    pub(crate) suffix: usize,
}

impl OrdsIntersectTermsEnumFrame {
    /// Port of `OrdsIntersectTermsEnumFrame(OrdsIntersectTermsEnum, int)`.
    /// The other fields keep Java's defaults (`0`, `false`, `null`).
    pub(crate) fn new(fr: &OrdsFieldReader, ord: i32) -> Result<Self> {
        let mut term_state = fr.parent.postings_reader.new_term_state()?;
        term_state.total_term_freq = -1;
        Ok(Self {
            ord,
            fp: 0,
            fp_orig: 0,
            fp_end: 0,
            last_sub_fp: 0,
            state: 0,
            meta_data_upto: 0,
            suffixes: OwnedArrayInput::default(),
            stats: OwnedArrayInput::default(),
            floor_data: OwnedArrayInput::default(),
            prefix: 0,
            ent_count: 0,
            next_ent: 0,
            term_ord_orig: 0,
            term_ord: 0,
            is_last_in_floor: false,
            is_leaf_block: false,
            num_follow_floor_blocks: 0,
            next_floor_label: 0,
            transition: Transition::default(),
            cur_transition_max: 0,
            transition_index: 0,
            transition_count: 0,
            arc: None,
            term_state,
            bytes: OwnedArrayInput::default(),
            output_prefix: Output::default(),
            start_byte_pos: 0,
            suffix: 0,
        })
    }

    /// Port of `loadNextFloorBlock()`.
    pub(crate) fn load_next_floor_block(&mut self, ctx: &mut IteCtx<'_>) -> Result<()> {
        java_assert!(self.num_follow_floor_blocks > 0);

        loop {
            self.fp = self.fp_orig + ((self.floor_data.read_v_long()? as u64) >> 1) as i64;
            self.num_follow_floor_blocks -= 1;
            if self.num_follow_floor_blocks != 0 {
                self.next_floor_label = i32::from(self.floor_data.read_byte()?);
                self.term_ord += self.floor_data.read_v_long()?;
            } else {
                self.next_floor_label = 256;
            }
            if !(self.num_follow_floor_blocks != 0 && self.next_floor_label <= self.transition.min)
            {
                break;
            }
        }

        self.load(ctx, None)
    }

    /// Port of `setState(int)`.
    pub(crate) fn set_state(&mut self, state: i32, transition_accessor: &dyn TransitionAccessor) {
        self.state = state;
        self.transition_index = 0;
        self.transition_count = transition_accessor.get_num_transitions(state);
        if self.transition_count != 0 {
            transition_accessor.init_transition(state, &mut self.transition);
            transition_accessor.get_next_transition(&mut self.transition);
            self.cur_transition_max = self.transition.max;
        } else {
            self.cur_transition_max = -1;
        }
    }

    /// Port of `load(Output)`; `None` is Java's `null`.
    pub(crate) fn load(&mut self, ctx: &mut IteCtx<'_>, output: Option<&Output>) -> Result<()> {
        if let Some(output) = output {
            if self.transition_count != 0 {
                let frame_index_data = output.bytes();

                // Floor frame
                self.floor_data.fill_from(frame_index_data);
                let code = self.floor_data.read_v_long()?;
                if (code & OUTPUT_FLAG_IS_FLOOR) != 0 {
                    self.num_follow_floor_blocks = self.floor_data.read_v_int()?;
                    self.next_floor_label = i32::from(self.floor_data.read_byte()?);

                    self.term_ord = self.term_ord_orig + self.floor_data.read_v_long()?;

                    // If current state is accept, we must process
                    // first block in case it has empty suffix:
                    if !ctx.byte_runnable.is_accept(self.state) {
                        // Maybe skip floor blocks:
                        java_assert!(
                            self.transition_index == 0,
                            "transitionIndex={}",
                            self.transition_index
                        );
                        while self.num_follow_floor_blocks != 0
                            && self.next_floor_label <= self.transition.min
                        {
                            self.fp = self.fp_orig
                                + ((self.floor_data.read_v_long()? as u64) >> 1) as i64;
                            self.num_follow_floor_blocks -= 1;
                            if self.num_follow_floor_blocks != 0 {
                                self.next_floor_label = i32::from(self.floor_data.read_byte()?);
                                self.term_ord += self.floor_data.read_v_long()?;
                            } else {
                                self.next_floor_label = 256;
                            }
                        }
                    }
                }
            }
        }

        let input = &mut *ctx.input;
        input.seek(self.fp)?;
        let code = input.read_v_int()?;
        self.ent_count = ((code as u32) >> 1) as i32;
        java_assert!(self.ent_count > 0);
        self.is_last_in_floor = (code & 1) != 0;

        // term suffixes:
        let code = input.read_v_int()?;
        self.is_leaf_block = (code & 1) != 0;
        let num_bytes = ((code as u32) >> 1) as i32;
        self.suffixes.fill(input, num_bytes)?;

        // stats
        let num_bytes = input.read_v_int()?;
        self.stats.fill(input, num_bytes)?;
        self.meta_data_upto = 0;

        self.term_state.term_block_ord = 0;
        self.next_ent = 0;

        // metadata
        let num_bytes = input.read_v_int()?;
        self.bytes.fill(input, num_bytes)?;

        if !self.is_last_in_floor {
            // Sub-blocks of a single floor block are always
            // written one after another -- tail recurse:
            self.fp_end = input.file_pointer()?;
        }
        Ok(())
    }

    // TODO: maybe add scanToLabel; should give perf boost

    /// Port of `next()`.
    pub(crate) fn next(&mut self) -> Result<bool> {
        if self.is_leaf_block {
            self.next_leaf()
        } else {
            self.next_non_leaf()
        }
    }

    /// Decodes next entry; returns true if it's a sub-block.
    ///
    /// Port of `nextLeaf()`.
    pub(crate) fn next_leaf(&mut self) -> Result<bool> {
        java_assert!(
            self.next_ent != -1 && self.next_ent < self.ent_count,
            "nextEnt={} entCount={} fp={}",
            self.next_ent,
            self.ent_count,
            self.fp
        );
        self.next_ent += 1;
        self.suffix = super::array_len(self.suffixes.read_v_int()?)?;
        self.start_byte_pos = self.suffixes.position();
        self.suffixes.skip_bytes(self.suffix as i32)?;
        Ok(false)
    }

    /// Port of `nextNonLeaf()`.
    pub(crate) fn next_non_leaf(&mut self) -> Result<bool> {
        java_assert!(
            self.next_ent != -1 && self.next_ent < self.ent_count,
            "nextEnt={} entCount={} fp={}",
            self.next_ent,
            self.ent_count,
            self.fp
        );
        self.next_ent += 1;
        let code = self.suffixes.read_v_int()?;
        self.suffix = ((code as u32) >> 1) as usize;
        self.start_byte_pos = self.suffixes.position();
        self.suffixes.skip_bytes(self.suffix as i32)?;
        if (code & 1) == 0 {
            // A normal term
            self.term_state.term_block_ord += 1;
            Ok(false)
        } else {
            // A sub-block; make sub-FP absolute:
            self.last_sub_fp = self.fp - self.suffixes.read_v_long()?;
            // Skip term ord
            self.suffixes.read_v_long()?;
            Ok(true)
        }
    }

    /// Port of `getTermBlockOrd()`.
    pub(crate) fn get_term_block_ord(&self) -> i32 {
        if self.is_leaf_block {
            self.next_ent
        } else {
            self.term_state.term_block_ord
        }
    }

    /// Port of `decodeMetaData()`.
    pub(crate) fn decode_meta_data(&mut self, fr: &OrdsFieldReader) -> Result<()> {
        // lazily catch up on metadata decode:
        let limit = self.get_term_block_ord();
        let mut absolute = self.meta_data_upto == 0;
        java_assert!(limit > 0);

        let field_info = &fr.data.field_info;
        // TODO: better API would be "jump straight to term=N"???
        while self.meta_data_upto < limit {
            // TODO: we could make "tiers" of metadata, ie,
            // decode docFreq/totalTF but don't decode postings
            // metadata; this way caller could get
            // docFreq/totalTF w/o paying decode cost for
            // postings

            // TODO: if docFreq were bulk decoded we could
            // just skipN here:

            // stats
            self.term_state.doc_freq = self.stats.read_v_int()?;
            if field_info.get_index_options() == IndexOptions::DOCS {
                self.term_state.total_term_freq = i64::from(self.term_state.doc_freq);
            // all tf values are 1
            } else {
                self.term_state.total_term_freq =
                    i64::from(self.term_state.doc_freq) + self.stats.read_v_long()?;
            }
            // metadata
            let state = &mut self.term_state;
            self.bytes.with(|i| {
                fr.parent
                    .postings_reader
                    .decode_term(i, field_info, state, absolute)
            })?;

            self.meta_data_upto += 1;
            absolute = false;
        }
        self.term_state.term_block_ord = self.meta_data_upto;
        Ok(())
    }
}
