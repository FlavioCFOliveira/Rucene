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

//! Port of `org.apache.lucene.codecs.blocktreeords.OrdsSegmentTermsEnum`.
//!
//! Java's private `printSeekState(PrintStream)`, a debugging aid no code
//! calls (`@SuppressWarnings("unused")`), is not ported.

use std::cmp::Ordering;
use std::fmt;
use std::sync::Arc;

use rucene::codecs::BlockTermState;
use rucene::error::{LuceneError, Result};
use rucene::index::{ImpactsEnum, PostingsEnum, SeekStatus, TermState, TermsEnum};
use rucene::store::{ByteArrayDataInput, DataInput, IndexInput};
use rucene::util::attribute::AttributeSource;
use rucene::util::fst::{
    target_has_arcs, Arc as FstArc, BytesReader, Outputs, ARCS_FOR_BINARY_SEARCH,
    BIT_ARC_HAS_OUTPUT, FST,
};
use rucene::util::BytesRef;

use super::fst_ords_outputs::{FSTOrdsOutputs, Output};
use super::ords_block_tree_terms_writer::{
    FST_OUTPUTS, OUTPUT_FLAGS_NUM_BITS, OUTPUT_FLAG_HAS_TERMS, OUTPUT_FLAG_IS_FLOOR,
};
use super::ords_field_reader::OrdsFieldReader;
use super::ords_segment_terms_enum_frame::{OrdsSegmentTermsEnumFrame, SteCtx};
use super::{java_assert, TermBuilder};

/// The ord of `staticFrame`.
const STATIC_FRAME: i32 = -1;

/// Iterates through terms in this field.
///
/// Port of `org.apache.lucene.codecs.blocktreeords.OrdsSegmentTermsEnum`.
/// Java's `currentFrame` reference is the frame's `ord` here: `-1` for
/// `staticFrame`, else the index into `stack`.
pub struct OrdsSegmentTermsEnum {
    // Lazy init:
    input: Option<Box<dyn IndexInput>>,

    stack: Vec<OrdsSegmentTermsEnumFrame>,
    static_frame: OrdsSegmentTermsEnumFrame,
    current_frame: i32,
    term_exists: bool,
    fr: OrdsFieldReader,

    target_before_current_length: i32,

    // What prefix of the current term was present in the index:
    valid_index_prefix: usize,

    // assert only:
    eof: bool,

    term: TermBuilder,
    index: Option<Arc<FST<FSTOrdsOutputs>>>,
    fst_reader: Option<Box<dyn BytesReader>>,

    arcs: Vec<FstArc<Output>>,

    positioned: bool,

    /// The private `arc` field `getByOutput` walks.
    arc: FstArc<Output>,

    attributes: AttributeSource,
}

/// Port of the private static class `OrdsSegmentTermsEnum.InputOutput`:
/// holds a single input (`IntsRef`) + output pair.
struct InputOutput {
    input: Vec<i32>,
    output: Option<Output>,
}

impl OrdsSegmentTermsEnum {
    /// Port of `OrdsSegmentTermsEnum(OrdsFieldReader)`.
    pub(crate) fn new(fr: OrdsFieldReader) -> Result<Self> {
        // Used to hold seek by TermState, or cached seek
        let static_frame = OrdsSegmentTermsEnumFrame::new(&fr, STATIC_FRAME)?;

        let index = fr.index().cloned();
        let fst_reader = match &index {
            None => None,
            Some(index) => Some(index.get_owned_bytes_reader()?),
        };

        // Init w/ root block; don't use index since it may
        // not (and need not) have been loaded
        let mut arcs = vec![FstArc::default()];

        if let Some(index) = &index {
            index.get_first_arc(&mut arcs[0]);
            // Empty string prefix must have an output in the index!
            java_assert!(arcs[0].is_final());
        }

        Ok(Self {
            input: None,
            stack: Vec::new(),
            static_frame,
            current_frame: STATIC_FRAME,
            term_exists: false,
            fr,
            target_before_current_length: 0,
            valid_index_prefix: 0,
            eof: false,
            term: TermBuilder::default(),
            index,
            fst_reader,
            arcs,
            positioned: false,
            arc: FstArc::default(),
            attributes: AttributeSource::new(),
        })
    }

    /// The frame whose ord is `ord` (`-1` is `staticFrame`).
    fn frame(&self, ord: i32) -> &OrdsSegmentTermsEnumFrame {
        if ord == STATIC_FRAME {
            &self.static_frame
        } else {
            &self.stack[ord as usize]
        }
    }

    fn frame_mut(&mut self, ord: i32) -> &mut OrdsSegmentTermsEnumFrame {
        if ord == STATIC_FRAME {
            &mut self.static_frame
        } else {
            &mut self.stack[ord as usize]
        }
    }

    fn cur(&self) -> &OrdsSegmentTermsEnumFrame {
        self.frame(self.current_frame)
    }

    fn cur_mut(&mut self) -> &mut OrdsSegmentTermsEnumFrame {
        self.frame_mut(self.current_frame)
    }

    /// Splits the enum into the frame `ord` and what the frame reaches
    /// through Java's `ste`.
    fn split(&mut self, ord: i32) -> (&mut OrdsSegmentTermsEnumFrame, SteCtx<'_>) {
        let Self {
            input,
            stack,
            static_frame,
            term_exists,
            fr,
            term,
            ..
        } = self;
        let frame = if ord == STATIC_FRAME {
            static_frame
        } else {
            &mut stack[ord as usize]
        };
        (
            frame,
            SteCtx {
                term,
                term_exists,
                input,
                fr,
            },
        )
    }

    /// `currentFrame.loadBlock()`.
    fn load_block(&mut self) -> Result<()> {
        let (frame, mut ctx) = self.split(self.current_frame);
        frame.load_block(&mut ctx)
    }

    /// `currentFrame.loadNextFloorBlock()`.
    fn load_next_floor_block(&mut self) -> Result<()> {
        let (frame, mut ctx) = self.split(self.current_frame);
        frame.load_next_floor_block(&mut ctx)
    }

    /// `currentFrame.next()`.
    fn frame_next(&mut self) -> Result<bool> {
        let (frame, mut ctx) = self.split(self.current_frame);
        frame.next(&mut ctx)
    }

    /// `currentFrame.decodeMetaData()`.
    fn decode_meta_data(&mut self) -> Result<()> {
        let Self {
            stack,
            static_frame,
            fr,
            current_frame,
            ..
        } = self;
        let frame = if *current_frame == STATIC_FRAME {
            static_frame
        } else {
            &mut stack[*current_frame as usize]
        };
        frame.decode_meta_data(fr)
    }

    /// `currentFrame.scanToTerm(target, exactOnly)`, with the descent of
    /// `scanToTermNonLeaf` into sub-frames, which Java performs through
    /// `ste.pushFrame`.
    fn scan_to_term(&mut self, target: &[u8], exact_only: bool) -> Result<SeekStatus> {
        let (frame, mut ctx) = self.split(self.current_frame);
        if frame.is_leaf_block {
            return frame.scan_to_term_leaf(&mut ctx, target, exact_only);
        }
        let scan = frame.scan_to_term_non_leaf(&mut ctx, target, exact_only)?;
        if let Some((length, prev_term_ord)) = scan.descend {
            let last_sub_fp = self.cur().last_sub_fp;
            self.current_frame = self.push_frame_fp(false, last_sub_fp, length, prev_term_ord)?;
            self.load_block()?;
            while self.frame_next()? {
                let last_sub_fp = self.cur().last_sub_fp;
                let length = self.term.length;
                self.current_frame =
                    self.push_frame_fp(false, last_sub_fp, length, prev_term_ord)?;
                self.load_block()?;
            }
        }
        Ok(scan.status)
    }

    /// Port of `getFrame(int)`.
    fn get_frame(&mut self, ord: i32) -> Result<()> {
        while self.stack.len() <= ord as usize {
            let next = OrdsSegmentTermsEnumFrame::new(&self.fr, self.stack.len() as i32)?;
            self.stack.push(next);
        }
        java_assert!(self.stack[ord as usize].ord == ord);
        Ok(())
    }

    /// Port of `getArc(int)`.
    fn get_arc(&mut self, ord: usize) {
        if ord >= self.arcs.len() {
            self.arcs.resize_with(ord + 1, FstArc::default);
        }
    }

    /// Pushes a frame we seek'd to.
    ///
    /// Port of `pushFrame(FST.Arc, Output, int)`; `has_arc` is
    /// `arc != null`. Returns the pushed frame's ord.
    fn push_frame_output(
        &mut self,
        has_arc: bool,
        frame_data: &Output,
        length: usize,
    ) -> Result<i32> {
        let mut scratch_reader = ByteArrayDataInput::new(frame_data.bytes());
        let code = scratch_reader.read_v_long()?;
        let fp_seek = ((code as u64) >> OUTPUT_FLAGS_NUM_BITS) as i64;
        let ord = 1 + self.cur().ord;
        self.get_frame(ord)?;
        let f = self.frame_mut(ord);
        f.has_terms = (code & OUTPUT_FLAG_HAS_TERMS) != 0;
        f.has_terms_orig = f.has_terms;
        f.is_floor = (code & OUTPUT_FLAG_IS_FLOOR) != 0;

        // Must setFloorData before pushFrame in case pushFrame tries to rewind:
        if f.is_floor {
            f.term_ord_orig = frame_data.start_ord();
            f.set_floor_data(frame_data.bytes(), scratch_reader.position())?;
        }

        self.push_frame_fp(has_arc, fp_seek, length, frame_data.start_ord())?;

        Ok(ord)
    }

    /// Pushes next'd frame or seek'd frame; we later lazy-load the frame
    /// only when needed.
    ///
    /// Port of `pushFrame(FST.Arc, long, int, long)`; `has_arc` is
    /// `arc != null`. Returns the pushed frame's ord.
    fn push_frame_fp(
        &mut self,
        has_arc: bool,
        fp: i64,
        length: usize,
        term_ord: i64,
    ) -> Result<i32> {
        let ord = 1 + self.cur().ord;
        self.get_frame(ord)?;
        let target_before_current_length = self.target_before_current_length;
        let f = self.frame_mut(ord);
        f.has_arc = has_arc;
        if f.fp_orig == fp && f.next_ent != -1 {
            if (f.prefix_length as i64) > i64::from(target_before_current_length) {
                f.rewind()?;
            }
            java_assert!(length == f.prefix_length);
            java_assert!(term_ord == f.term_ord_orig);
        } else {
            f.next_ent = -1;
            f.prefix_length = length;
            f.state.term_block_ord = 0;
            f.term_ord_orig = term_ord;
            f.term_ord = term_ord;
            f.fp_orig = fp;
            f.fp = fp;
            f.last_sub_fp = -1;
        }

        Ok(ord)
    }

    // asserts only
    fn clear_eof(&mut self) -> bool {
        self.eof = false;
        true
    }

    // asserts only
    fn set_eof(&mut self) -> bool {
        self.eof = true;
        true
    }

    fn index_or_not_loaded(&self) -> Result<Arc<FST<FSTOrdsOutputs>>> {
        self.index
            .clone()
            .ok_or_else(|| LuceneError::illegal_state("terms index was not loaded"))
    }

    /// `fr.index.findTargetArc(targetLabel, arcs[targetUpto],
    /// getArc(1 + targetUpto), fstReader)`: the walks below keep their
    /// current arc in `arcs[targetUpto]`.
    fn find_target_arc(
        &mut self,
        index: &FST<FSTOrdsOutputs>,
        target_label: i32,
        target_upto: usize,
    ) -> Result<bool> {
        self.get_arc(1 + target_upto);
        let (follow, arc) = self.arcs.split_at_mut(1 + target_upto);
        let reader = self
            .fst_reader
            .as_deref_mut()
            .expect("INVARIANT: an enum over an index holds its reader");
        index.find_target_arc(target_label, &follow[target_upto], &mut arc[0], reader)
    }

    /// The common-prefix reuse of `seekExact(BytesRef)` and
    /// `seekCeil(BytesRef)`: compares the target with the current term up
    /// to `validIndexPrefix`, following `arcs` and summing their outputs.
    /// Returns `(cmp, targetUpto, output, lastFrame)`.
    fn reuse_seek_state(&self, target: &[u8]) -> Result<(Ordering, usize, Output, i32)> {
        let arc = &self.arcs[0];
        java_assert!(arc.is_final());
        let mut output = arc.output().clone();
        let mut target_upto = 0usize;

        let mut last_frame = self.stack[0].ord;
        java_assert!(self.valid_index_prefix <= self.term.length);

        let target_limit = target.len().min(self.valid_index_prefix);

        let mut cmp = Ordering::Equal;

        // TODO: reverse vLong byte order for better FST
        // prefix output sharing

        // First compare up to valid seek frames:
        while target_upto < target_limit {
            cmp = self.term.byte_at(target_upto)?.cmp(&target[target_upto]);
            if cmp != Ordering::Equal {
                break;
            }
            let arc = &self.arcs[1 + target_upto];
            java_assert!(
                arc.label() == i32::from(target[target_upto]),
                "arc.label={} targetLabel={}",
                arc.label(),
                target[target_upto]
            );
            if !arc.output().is_no_output() {
                output = FST_OUTPUTS.add(&output, arc.output());
            }
            if arc.is_final() {
                last_frame = self.stack[(1 + last_frame) as usize].ord;
            }
            target_upto += 1;
        }

        if cmp == Ordering::Equal {
            // Second compare the rest of the term, but
            // don't save arc/output/frame; we only do this
            // to find out if the target term is before,
            // equal or after the current term
            let term_rest = self
                .term
                .bytes
                .get(target_upto..self.term.length)
                .unwrap_or(&[]);
            cmp = term_rest.cmp(&target[target_upto..]);
        }
        Ok((cmp, target_upto, output, last_frame))
    }

    /// Port of `seekExact(BytesRef)`.
    fn do_seek_exact(&mut self, target: &[u8]) -> Result<bool> {
        let index = self.index_or_not_loaded()?;

        self.term.grow(1 + target.len());

        java_assert!(self.clear_eof());

        let mut target_upto: usize;
        let mut output: Output;

        self.target_before_current_length = self.cur().ord;

        if self.positioned && self.current_frame != STATIC_FRAME {
            // We are already seek'd; find the common
            // prefix of new seek term vs current term and
            // re-use the corresponding seek state.  For
            // example, if app first seeks to foobar, then
            // seeks to foobaz, we can re-use the seek state
            // for the first 5 bytes.
            let (cmp, upto, out, last_frame) = self.reuse_seek_state(target)?;
            target_upto = upto;
            output = out;

            match cmp {
                Ordering::Less => {
                    // Common case: target term is after current
                    // term, ie, app is seeking multiple terms
                    // in sorted order
                    self.current_frame = last_frame;
                }
                Ordering::Greater => {
                    // Uncommon case: target term
                    // is before current term; this means we can
                    // keep the currentFrame but we must rewind it
                    // (so we scan from the start)
                    self.target_before_current_length = last_frame;
                    self.current_frame = last_frame;
                    self.cur_mut().rewind()?;
                }
                Ordering::Equal => {
                    // Target is exactly the same as current term
                    java_assert!(self.term.length == target.len());
                    if self.term_exists {
                        return Ok(true);
                    }
                }
            }
        } else {
            self.target_before_current_length = -1;
            index.get_first_arc(&mut self.arcs[0]);

            // Empty string prefix must have an output (block) in the index!
            java_assert!(self.arcs[0].is_final());

            output = self.arcs[0].output().clone();

            self.current_frame = STATIC_FRAME;

            target_upto = 0;
            let frame_data = FST_OUTPUTS.add(&output, self.arcs[0].next_final_output());
            self.current_frame = self.push_frame_output(true, &frame_data, 0)?;
        }

        self.positioned = true;

        // We are done sharing the common prefix with the incoming target and where we are
        // currently seek'd; now continue walking the index:
        while target_upto < target.len() {
            let target_label = i32::from(target[target_upto]);

            let found = self.find_target_arc(&index, target_label, target_upto)?;

            if !found {
                // Index is exhausted
                self.valid_index_prefix = self.cur().prefix_length;

                self.cur_mut().scan_to_floor_frame(target)?;

                if !self.cur().has_terms {
                    self.term_exists = false;
                    self.term.set_byte_at(target_upto, target_label as u8);
                    self.term.set_length(1 + target_upto);
                    return Ok(false);
                }

                self.load_block()?;

                let result = self.scan_to_term(target, true)?;
                return Ok(result == SeekStatus::FOUND);
            } else {
                // Follow this arc
                self.term.set_byte_at(target_upto, target_label as u8);
                // Aggregate output as we go:
                let arc = &self.arcs[1 + target_upto];
                if !arc.output().is_no_output() {
                    output = FST_OUTPUTS.add(&output, arc.output());
                }

                target_upto += 1;

                if self.arcs[target_upto].is_final() {
                    let frame_data =
                        FST_OUTPUTS.add(&output, self.arcs[target_upto].next_final_output());
                    self.current_frame = self.push_frame_output(true, &frame_data, target_upto)?;
                }
            }
        }

        self.valid_index_prefix = self.cur().prefix_length;

        self.cur_mut().scan_to_floor_frame(target)?;

        // Target term is entirely contained in the index:
        if !self.cur().has_terms {
            self.term_exists = false;
            self.term.set_length(target_upto);
            return Ok(false);
        }

        self.load_block()?;

        let result = self.scan_to_term(target, true)?;
        Ok(result == SeekStatus::FOUND)
    }

    /// Port of `seekCeil(BytesRef)`.
    fn do_seek_ceil(&mut self, target: &[u8]) -> Result<SeekStatus> {
        let index = self.index_or_not_loaded()?;

        self.term.grow(1 + target.len());

        java_assert!(self.clear_eof());

        let mut target_upto: usize;
        let mut output: Output;

        self.target_before_current_length = self.cur().ord;

        if self.positioned && self.current_frame != STATIC_FRAME {
            // We are already seek'd; find the common
            // prefix of new seek term vs current term and
            // re-use the corresponding seek state.  For
            // example, if app first seeks to foobar, then
            // seeks to foobaz, we can re-use the seek state
            // for the first 5 bytes.

            // TODO: we should write our vLong backwards (MSB
            // first) to get better sharing from the FST
            let (cmp, upto, out, last_frame) = self.reuse_seek_state(target)?;
            target_upto = upto;
            output = out;

            match cmp {
                Ordering::Less => {
                    // Common case: target term is after current
                    // term, ie, app is seeking multiple terms
                    // in sorted order
                    self.current_frame = last_frame;
                }
                Ordering::Greater => {
                    // Uncommon case: target term
                    // is before current term; this means we can
                    // keep the currentFrame but we must rewind it
                    // (so we scan from the start)
                    self.target_before_current_length = 0;
                    self.current_frame = last_frame;
                    self.cur_mut().rewind()?;
                }
                Ordering::Equal => {
                    // Target is exactly the same as current term
                    java_assert!(self.term.length == target.len());
                    if self.term_exists {
                        return Ok(SeekStatus::FOUND);
                    }
                }
            }
        } else {
            self.target_before_current_length = -1;
            index.get_first_arc(&mut self.arcs[0]);

            // Empty string prefix must have an output (block) in the index!
            java_assert!(self.arcs[0].is_final());

            output = self.arcs[0].output().clone();

            self.current_frame = STATIC_FRAME;

            target_upto = 0;
            let frame_data = FST_OUTPUTS.add(&output, self.arcs[0].next_final_output());
            self.current_frame = self.push_frame_output(true, &frame_data, 0)?;
        }

        self.positioned = true;

        // We are done sharing the common prefix with the incoming target and where we are
        // currently seek'd; now continue walking the index:
        while target_upto < target.len() {
            let target_label = i32::from(target[target_upto]);

            let found = self.find_target_arc(&index, target_label, target_upto)?;

            if !found {
                // Index is exhausted
                self.valid_index_prefix = self.cur().prefix_length;

                self.cur_mut().scan_to_floor_frame(target)?;

                self.load_block()?;

                let result = self.scan_to_term(target, false)?;
                if result == SeekStatus::END {
                    self.term.copy_bytes(target);
                    self.term_exists = false;

                    return if self.next()?.is_some() {
                        Ok(SeekStatus::NOT_FOUND)
                    } else {
                        Ok(SeekStatus::END)
                    };
                } else {
                    return Ok(result);
                }
            } else {
                // Follow this arc
                self.term.set_byte_at(target_upto, target_label as u8);
                // Aggregate output as we go:
                let arc = &self.arcs[1 + target_upto];
                if !arc.output().is_no_output() {
                    output = FST_OUTPUTS.add(&output, arc.output());
                }

                target_upto += 1;

                if self.arcs[target_upto].is_final() {
                    let frame_data =
                        FST_OUTPUTS.add(&output, self.arcs[target_upto].next_final_output());
                    self.current_frame = self.push_frame_output(true, &frame_data, target_upto)?;
                }
            }
        }

        self.valid_index_prefix = self.cur().prefix_length;

        self.cur_mut().scan_to_floor_frame(target)?;

        self.load_block()?;

        let result = self.scan_to_term(target, false)?;

        if result == SeekStatus::END {
            self.term.copy_bytes(target);
            self.term_exists = false;
            if self.next()?.is_some() {
                Ok(SeekStatus::NOT_FOUND)
            } else {
                Ok(SeekStatus::END)
            }
        } else {
            Ok(result)
        }
    }

    /// Specialized getByOutput that can understand the ranges (startOrd to
    /// endOrd) we use here, not just startOrd.
    ///
    /// Port of `getByOutput(long)`.
    // TODO: this is similar to Util.getByOutput ... can we refactor/share?
    fn get_by_output(&mut self, target_ord: i64) -> Result<InputOutput> {
        let index = self.index_or_not_loaded()?;
        let mut result: Vec<i32> = Vec::new();

        index.get_first_arc(&mut self.arc);
        let mut output = self.arc.output().clone();
        let mut upto = 0usize;

        let mut best_upto = 0usize;
        let mut best_output: Option<Output> = None;

        let fst_reader = self
            .fst_reader
            .as_deref_mut()
            .expect("INVARIANT: an enum over an index holds its reader");
        let arc = &mut self.arc;

        let finish = |mut result: Vec<i32>, best_upto: usize, best_output: Option<Output>| {
            result.truncate(best_upto);
            InputOutput {
                input: result,
                output: best_output,
            }
        };

        loop {
            if arc.is_final() {
                let final_output = FST_OUTPUTS.add(&output, arc.next_final_output());
                if target_ord >= final_output.start_ord()
                    && target_ord <= i64::MAX - final_output.end_ord()
                {
                    // Only one range should match across all arc leaving this node
                    // assert bestOutput == null;
                    best_output = Some(final_output);
                    best_upto = upto;
                }
            }

            if target_has_arcs(arc) {
                if result.len() < 1 + upto {
                    result.resize(1 + upto, 0);
                }
                index.read_first_real_target_arc(arc.target(), arc, fst_reader)?;

                if arc.bytes_per_arc() != 0 && arc.node_flags() == ARCS_FOR_BINARY_SEARCH {
                    let mut low = 0i32;
                    let mut high = arc.num_arcs() - 1;
                    let mut mid = 0i32;
                    let mut found = false;
                    while low <= high {
                        mid = ((low + high) as u32 >> 1) as i32;
                        fst_reader.set_position(arc.pos_arcs_start());
                        fst_reader.skip_bytes(i64::from(arc.bytes_per_arc()) * i64::from(mid))?;
                        let flags = fst_reader.read_byte()?;
                        index.read_label(fst_reader.as_data_input())?;
                        let min_arc_output = if (i32::from(flags) & BIT_ARC_HAS_OUTPUT) != 0 {
                            FST_OUTPUTS.add(&output, &FST_OUTPUTS.read(fst_reader.as_data_input())?)
                        } else {
                            output.clone()
                        };
                        if target_ord > i64::MAX - min_arc_output.end_ord() {
                            low = mid + 1;
                        } else if target_ord < min_arc_output.start_ord() {
                            high = mid - 1;
                        } else {
                            found = true;
                            break;
                        }
                    }

                    if !found {
                        return Ok(finish(result, best_upto, best_output));
                    }

                    // Recurse on this arc:
                    index.read_arc_by_index(arc, fst_reader, mid)?;
                    result[upto] = arc.label();
                    upto += 1;
                    output = FST_OUTPUTS.add(&output, arc.output());
                } else {
                    loop {
                        // This is the min output we'd hit if we follow
                        // this arc:
                        let min_arc_output = FST_OUTPUTS.add(&output, arc.output());
                        let end_ord = i64::MAX - min_arc_output.end_ord();

                        if target_ord >= min_arc_output.start_ord() && target_ord <= end_ord {
                            // Recurse on this arc:
                            output = min_arc_output;
                            result[upto] = arc.label();
                            upto += 1;
                            break;
                        } else if target_ord < end_ord || arc.is_last() {
                            java_assert!(best_output.is_some());
                            return Ok(finish(result, best_upto, best_output));
                        } else {
                            // Read next arc in this node:
                            index.read_next_real_arc(arc, fst_reader)?;
                        }
                    }
                }
            } else {
                return Ok(finish(result, best_upto, best_output));
            }
        }
    }
}

impl fmt::Display for OrdsSegmentTermsEnum {
    /// Port of `OrdsSegmentTermsEnum.toString()`: `"OrdsSegmentTermsEnum(seg="
    /// + fr.parent + ")"`.
    ///
    /// **Divergence.** Java prints the reader's live field count, which its
    /// `close()` clears to `0`; the enum shares only what the reader read,
    /// so it prints the count the reader opened with.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "OrdsSegmentTermsEnum(seg=OrdsBlockTreeTermsReader({},delegate={}))",
            self.fr.parent.description, self.fr.parent.postings_reader
        )
    }
}

impl TermsEnum for OrdsSegmentTermsEnum {
    fn attributes(&mut self) -> &mut AttributeSource {
        &mut self.attributes
    }

    fn seek_exact(&mut self, target: &BytesRef) -> Result<bool> {
        self.do_seek_exact(target.slice())
    }

    fn seek_ceil(&mut self, target: &BytesRef) -> Result<SeekStatus> {
        self.do_seek_ceil(target.slice())
    }

    /// Decodes only the term bytes of the next term. If caller then asks
    /// for metadata, ie docFreq, totalTermFreq or pulls a D/&PEnum, we then
    /// (lazily) decode all metadata up to the current term.
    fn next(&mut self) -> Result<Option<BytesRef>> {
        if self.input.is_none() {
            // Fresh TermsEnum; seek to first term:
            let has_arc = match &self.index {
                Some(index) => {
                    index.get_first_arc(&mut self.arcs[0]);
                    // Empty string prefix must have an output in the index!
                    java_assert!(self.arcs[0].is_final());
                    true
                }
                None => false,
            };
            let root_code = self.fr.data.root_code.clone();
            self.current_frame = self.push_frame_output(has_arc, &root_code, 0)?;
            self.load_block()?;
            self.positioned = true;
        }

        self.target_before_current_length = self.cur().ord;

        java_assert!(!self.eof);

        if self.current_frame == STATIC_FRAME || !self.positioned {
            // If seek was previously called and the term was
            // cached, or seek(TermState) was called, usually
            // caller is just going to pull a D/&PEnum or get
            // docFreq, etc.  But, if they then call next(),
            // this method catches up all internal state so next()
            // works properly:
            // `seekExact(term.get())`: the target aliases the term, which
            // the seek only rewrites with the target's own bytes.
            let target = self.term.get().to_vec();
            let result = self.do_seek_exact(&target)?;
            java_assert!(result);
        }

        // Pop finished blocks
        while self.cur().next_ent == self.cur().ent_count {
            if !self.cur().is_last_in_floor {
                self.load_next_floor_block()?;
            } else {
                if self.cur().ord == 0 {
                    java_assert!(self.set_eof());
                    self.term.set_length(0);
                    self.valid_index_prefix = 0;
                    self.cur_mut().rewind()?;
                    self.term_exists = false;
                    self.positioned = false;
                    return Ok(None);
                }
                let last_fp = self.cur().fp_orig;
                self.current_frame = self.stack[(self.cur().ord - 1) as usize].ord;

                if self.cur().next_ent == -1 || self.cur().last_sub_fp != last_fp {
                    // We popped into a frame that's not loaded
                    // yet or not scan'd to the right entry
                    let term = self.term.get().to_vec();
                    self.cur_mut().scan_to_floor_frame(&term)?;
                    self.load_block()?;
                    self.cur_mut().scan_to_sub_block(last_fp)?;
                }

                // Note that the seek state (last seek) has been
                // invalidated beyond this depth
                self.valid_index_prefix = self.valid_index_prefix.min(self.cur().prefix_length);
            }
        }

        loop {
            let prev_term_ord = self.cur().term_ord;
            if self.frame_next()? {
                // Push to new block:
                let last_sub_fp = self.cur().last_sub_fp;
                let length = self.term.length;
                self.current_frame =
                    self.push_frame_fp(false, last_sub_fp, length, prev_term_ord)?;
                // This is a "next" frame -- even if it's
                // floor'd we must pretend it isn't so we don't
                // try to scan to the right floor frame:
                self.cur_mut().is_floor = false;
                // currentFrame.hasTerms = true;
                self.load_block()?;
            } else {
                self.positioned = true;
                return Ok(Some(BytesRef::new(self.term.get().to_vec())));
            }
        }
    }

    fn term(&self) -> Result<Option<BytesRef>> {
        java_assert!(!self.eof);
        Ok(Some(BytesRef::new(self.term.get().to_vec())))
    }

    fn ord(&self) -> Result<i64> {
        java_assert!(!self.eof);
        java_assert!(self.cur().term_ord > 0);
        Ok(self.cur().term_ord - 1)
    }

    fn doc_freq(&mut self) -> Result<i32> {
        java_assert!(!self.eof);
        self.decode_meta_data()?;
        Ok(self.cur().state.doc_freq)
    }

    fn total_term_freq(&mut self) -> Result<i64> {
        java_assert!(!self.eof);
        self.decode_meta_data()?;
        Ok(self.cur().state.total_term_freq)
    }

    fn postings(
        &mut self,
        reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Option<Box<dyn PostingsEnum>>> {
        java_assert!(!self.eof);
        self.decode_meta_data()?;
        let state = self.cur().state;
        Ok(Some(self.fr.parent.postings_reader.postings(
            &self.fr.data.field_info,
            &state,
            reuse,
            flags,
        )?))
    }

    fn impacts(&mut self, flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        java_assert!(!self.eof);
        self.decode_meta_data()?;
        let state = self.cur().state;
        self.fr
            .parent
            .postings_reader
            .impacts(&self.fr.data.field_info, &state, flags)
    }

    fn seek_term_state(&mut self, target: &BytesRef, other_state: &dyn TermState) -> Result<()> {
        java_assert!(self.clear_eof());
        if target.slice() != self.term.get() || !self.term_exists {
            let block_state = other_state.as_any().downcast_ref::<BlockTermState>();
            java_assert!(block_state.is_some());
            let block_state = block_state.ok_or_else(|| {
                LuceneError::unchecked(
                    "java.lang.ClassCastException",
                    "class cannot be cast to class org.apache.lucene.codecs.BlockTermState",
                )
            })?;
            self.current_frame = STATIC_FRAME;
            self.static_frame.state.copy_from(block_state);
            self.term.copy_bytes(target.slice());
            self.static_frame.meta_data_upto = self.static_frame.get_term_block_ord();
            self.static_frame.term_ord = block_state.ord + 1;
            java_assert!(self.static_frame.meta_data_upto > 0);
            self.valid_index_prefix = 0;
        }
        self.positioned = true;
        Ok(())
    }

    fn term_state(&mut self) -> Result<Box<dyn TermState>> {
        java_assert!(!self.eof);
        self.decode_meta_data()?;
        let mut ts = self.cur().state;
        java_assert!(self.cur().term_ord > 0);
        ts.ord = self.cur().term_ord - 1;
        Ok(Box::new(ts))
    }

    /// Port of `seekExact(long)`.
    fn seek_ord(&mut self, target_ord: i64) -> Result<()> {
        if target_ord < 0 || target_ord >= self.fr.data.num_terms {
            return Err(LuceneError::illegal_argument(format!(
                "targetOrd out of bounds (got: {target_ord}, numTerms={})",
                self.fr.data.num_terms
            )));
        }

        java_assert!(self.clear_eof());

        // First do reverse lookup in the index to find the block that holds this term:
        let io = self.get_by_output(target_ord)?;
        self.term.grow(io.input.len());

        // Util.toBytesRef(io.input, term)
        self.term.set_length(io.input.len());
        for (i, &label) in io.input.iter().enumerate() {
            self.term.bytes[i] = label as u8;
        }
        if io.input.is_empty() {
            self.current_frame = STATIC_FRAME;
        } else {
            let ord = (io.input.len() - 1) as i32;
            self.get_frame(ord)?;
            self.current_frame = ord;
        }
        self.get_arc(io.input.len());

        // Don't force rewind based on term length; we rewind below based on ord:
        self.target_before_current_length = i32::MAX;
        let output = io.output.ok_or_else(|| {
            LuceneError::unchecked(
                "java.lang.NullPointerException",
                "Cannot invoke \"org.apache.lucene.codecs.blocktreeords.FSTOrdsOutputs$Output.bytes()\" \
                 because \"frameData\" is null",
            )
        })?;
        self.current_frame = self.push_frame_output(true, &output, io.input.len())?;
        if self.cur().term_ord > target_ord {
            self.cur_mut().rewind()?;
        }

        self.cur_mut().scan_to_floor_frame_ord(target_ord)?;
        self.load_block()?;

        while self.cur().term_ord <= target_ord {
            self.frame_next()?;
        }

        java_assert!(
            self.cur().term_ord == target_ord + 1,
            "currentFrame.termOrd={} vs ord={target_ord}",
            self.cur().term_ord
        );
        java_assert!(self.term_exists);

        // Leave enum fully unpositioned, because we didn't set frames for each byte leading up to
        // current term:
        self.valid_index_prefix = 0;
        self.positioned = false;
        Ok(())
    }
}
