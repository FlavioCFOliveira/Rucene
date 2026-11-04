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

//! Port of `org.apache.lucene.codecs.blocktreeords.OrdsIntersectTermsEnum`.

use std::fmt;
use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::{ImpactsEnum, PostingsEnum, SeekStatus, TermState, TermsEnum};
use rucene::store::IndexInput;
use rucene::util::attribute::AttributeSource;
use rucene::util::automaton::CompiledAutomaton;
use rucene::util::fst::{Arc as FstArc, BytesReader, Outputs, FST};
use rucene::util::BytesRef;

use super::fst_ords_outputs::FSTOrdsOutputs;
use super::fst_ords_outputs::Output;
use super::java_assert;
use super::ords_block_tree_terms_writer::FST_OUTPUTS;
use super::ords_field_reader::OrdsFieldReader;
use super::ords_intersect_terms_enum_frame::{IteCtx, OrdsIntersectTermsEnumFrame};

/// Iterates the terms of a field an automaton accepts.
///
/// Port of the package-private
/// `org.apache.lucene.codecs.blocktreeords.OrdsIntersectTermsEnum`.
/// Java's `currentFrame` reference is the frame's index in `stack`.
// NOTE: cannot seek!
pub struct OrdsIntersectTermsEnum {
    input: Box<dyn IndexInput>,

    stack: Vec<OrdsIntersectTermsEnumFrame>,

    arcs: Vec<FstArc<Output>>,

    /// The automaton Java's `byteRunnable` and `transitionAccessor` come
    /// from.
    compiled: CompiledAutomaton,
    common_suffix_ref: Option<BytesRef>,

    current_frame: usize,

    /// `term`: Java's `BytesRef`, its array and its length.
    term_bytes: Vec<u8>,
    term_length: usize,

    index: Arc<FST<FSTOrdsOutputs>>,
    fst_reader: Box<dyn BytesReader>,

    fr: OrdsFieldReader,

    saved_start_term: Option<Vec<u8>>,

    attributes: AttributeSource,
}

impl OrdsIntersectTermsEnum {
    /// Port of `OrdsIntersectTermsEnum(OrdsFieldReader, CompiledAutomaton,
    /// BytesRef)`.
    // TODO: in some cases we can filter by length?  eg
    // regexp foo*bar must be at least length 6 bytes
    pub(crate) fn new(
        fr: OrdsFieldReader,
        compiled: &CompiledAutomaton,
        start_term: Option<&BytesRef>,
    ) -> Result<Self> {
        let compiled = compiled.clone();
        if compiled.get_byte_runnable().is_none() || compiled.get_transition_accessor().is_none() {
            return Err(LuceneError::unchecked(
                "java.lang.NullPointerException",
                "Cannot invoke \"org.apache.lucene.util.automaton.TransitionAccessor\
                 .getNumTransitions(int)\" because \"this.ite.transitionAccessor\" is null",
            ));
        }
        let common_suffix_ref = compiled.common_suffix_ref.clone();
        let input = fr.parent.input.clone_input()?;
        let mut stack = Vec::with_capacity(5);
        for idx in 0..5 {
            stack.push(OrdsIntersectTermsEnumFrame::new(&fr, idx)?);
        }
        let arcs = vec![FstArc::default(); 5];

        let index = fr.index().cloned().ok_or_else(|| {
            LuceneError::unchecked(
                "java.lang.NullPointerException",
                "Cannot invoke \"org.apache.lucene.util.fst.FST.getFirstArc(\
                 org.apache.lucene.util.fst.FST$Arc)\" because \"fr.index\" is null",
            )
        })?;
        let fst_reader = index.get_owned_bytes_reader()?;

        let mut e = Self {
            input,
            stack,
            arcs,
            compiled,
            common_suffix_ref,
            current_frame: 0,
            term_bytes: Vec::new(),
            term_length: 0,
            index,
            fst_reader,
            fr,
            saved_start_term: None,
            attributes: AttributeSource::new(),
        };

        // TODO: if the automaton is "smallish" we really
        // should use the terms index to seek at least to
        // the initial term and likely to subsequent terms
        // (or, maybe just fallback to ATE for such cases).
        // Else the seek cost of loading the frames will be
        // too costly.

        e.index.get_first_arc(&mut e.arcs[0]);
        // Empty string prefix must have an output in the index!
        java_assert!(e.arcs[0].is_final());

        // Special pushFrame since it's the first one:
        let root_block_fp = e.fr.data.root_block_fp;
        let root_code = e.fr.data.root_code.clone();
        let arc = e.arcs[0].clone();
        {
            let (f, mut ctx) = e.split(0);
            f.fp = root_block_fp;
            f.fp_orig = root_block_fp;
            f.prefix = 0;
            f.set_state(0, ctx.transition_accessor);
            f.output_prefix = arc.output().clone();
            f.arc = Some(arc);
            f.load(&mut ctx, Some(&root_code))?;
        }

        // for assert:
        java_assert!(e.set_saved_start_term(start_term));

        e.current_frame = 0;
        if let Some(start_term) = start_term {
            e.seek_to_start_term(start_term.slice())?;
        }
        Ok(e)
    }

    /// Splits the enum into the frame at `idx` and what the frame reaches
    /// through Java's `ite`.
    fn split(&mut self, idx: usize) -> (&mut OrdsIntersectTermsEnumFrame, IteCtx<'_>) {
        let Self {
            input,
            stack,
            compiled,
            ..
        } = self;
        (
            &mut stack[idx],
            IteCtx {
                input: input.as_mut(),
                byte_runnable: compiled
                    .get_byte_runnable()
                    .expect("INVARIANT: the constructor checked the byte runnable"),
                transition_accessor: compiled
                    .get_transition_accessor()
                    .expect("INVARIANT: the constructor checked the transition accessor"),
            },
        )
    }

    fn cur(&self) -> &OrdsIntersectTermsEnumFrame {
        &self.stack[self.current_frame]
    }

    fn cur_mut(&mut self) -> &mut OrdsIntersectTermsEnumFrame {
        &mut self.stack[self.current_frame]
    }

    // only for assert:
    fn set_saved_start_term(&mut self, start_term: Option<&BytesRef>) -> bool {
        self.saved_start_term = start_term.map(|t| t.slice().to_vec());
        true
    }

    /// Port of `getFrame(int)`.
    fn get_frame(&mut self, ord: usize) -> Result<()> {
        while self.stack.len() <= ord {
            let next = OrdsIntersectTermsEnumFrame::new(&self.fr, self.stack.len() as i32)?;
            self.stack.push(next);
        }
        java_assert!(self.stack[ord].ord as usize == ord);
        Ok(())
    }

    /// Port of `getArc(int)`.
    fn get_arc(&mut self, ord: usize) {
        if ord >= self.arcs.len() {
            self.arcs.resize_with(ord + 1, FstArc::default);
        }
    }

    /// Port of `pushFrame(int)`; returns the pushed frame's index.
    fn push_frame(&mut self, state: i32) -> Result<usize> {
        let ord = 1 + self.cur().ord as usize;
        self.get_frame(ord)?;

        let (cur_last_sub_fp, cur_prefix, cur_suffix) = {
            let cur = self.cur();
            (cur.last_sub_fp, cur.prefix, cur.suffix)
        };
        {
            let (f, ctx) = self.split(ord);
            f.fp = cur_last_sub_fp;
            f.fp_orig = cur_last_sub_fp;
            f.prefix = cur_prefix + cur_suffix;
            f.set_state(state, ctx.transition_accessor);
        }
        let f_prefix = cur_prefix + cur_suffix;

        // Walk the arc through the index -- we only
        // "bother" with this so we can get the floor data
        // from the index and skip floor blocks when
        // possible:
        let mut arc = self
            .cur()
            .arc
            .clone()
            .expect("INVARIANT: every pushed frame holds its arc");
        let mut idx = cur_prefix;
        java_assert!(cur_suffix > 0);
        let mut output = self.cur().output_prefix.clone();
        while idx < f_prefix {
            let target = i32::from(self.term_bytes[idx]);
            // TODO: we could be more efficient for the next()
            // case by using current arc as starting point,
            // passed to findTargetArc
            self.get_arc(1 + idx);
            let found = self.index.find_target_arc(
                target,
                &arc,
                &mut self.arcs[1 + idx],
                self.fst_reader.as_mut(),
            )?;
            java_assert!(found);
            if !found {
                return Err(LuceneError::unchecked(
                    "java.lang.NullPointerException",
                    "Cannot invoke \"org.apache.lucene.util.fst.FST$Arc.output()\" because \
                     \"arc\" is null",
                ));
            }
            arc = self.arcs[1 + idx].clone();
            output = FST_OUTPUTS.add(&output, arc.output());
            idx += 1;
        }

        java_assert!(arc.is_final());
        let frame_data = FST_OUTPUTS.add(&output, arc.next_final_output());
        let (f, mut ctx) = self.split(ord);
        f.arc = Some(arc);
        f.output_prefix = output;
        f.load(&mut ctx, Some(&frame_data))?;
        Ok(ord)
    }

    /// Port of `getState()`.
    fn get_state(&self) -> Result<i32> {
        let cur = self.cur();
        let byte_runnable = self
            .compiled
            .get_byte_runnable()
            .expect("INVARIANT: the constructor checked the byte runnable");
        let mut state = cur.state;
        for idx in 0..cur.suffix {
            state = byte_runnable.step(
                state,
                i32::from(cur.suffixes.bytes[cur.start_byte_pos + idx]),
            );
            java_assert!(state != -1);
        }
        Ok(state)
    }

    /// `term.bytes = ArrayUtil.grow(term.bytes, len)`.
    fn grow_term(&mut self, len: usize) {
        if self.term_bytes.len() < len {
            self.term_bytes.resize(len, 0);
        }
    }

    /// The current entry's suffix into `term.bytes[prefix ..]`.
    fn copy_suffix_into_term(&mut self) {
        let Self {
            stack,
            current_frame,
            term_bytes,
            ..
        } = self;
        let cur = &stack[*current_frame];
        term_bytes[cur.prefix..cur.prefix + cur.suffix].copy_from_slice(
            &cur.suffixes.bytes[cur.start_byte_pos..cur.start_byte_pos + cur.suffix],
        );
    }

    /// The current term, `term.bytes[0 .. term.length]`.
    fn term_slice(&self) -> &[u8] {
        &self.term_bytes[..self.term_length]
    }

    /// NOTE: specialized to only doing the first-time seek, but we could
    /// generalize it to allow arbitrary seekExact/Ceil. Note that this is a
    /// seekFloor!
    ///
    /// Port of `seekToStartTerm(BytesRef)`.
    fn seek_to_start_term(&mut self, target: &[u8]) -> Result<()> {
        java_assert!(self.cur().ord == 0);
        if self.term_length < target.len() {
            self.grow_term(target.len());
        }

        for _idx in 0..=target.len() {
            loop {
                let (save_pos, save_start_byte_pos, save_suffix, save_last_sub_fp, save_tbo) = {
                    let cur = self.cur();
                    (
                        cur.suffixes.position(),
                        cur.start_byte_pos,
                        cur.suffix,
                        cur.last_sub_fp,
                        cur.term_state.term_block_ord,
                    )
                };

                let is_sub_block = self.cur_mut().next()?;

                self.term_length = self.cur().prefix + self.cur().suffix;
                self.grow_term(self.term_length);
                self.copy_suffix_into_term();

                if is_sub_block && target.starts_with(self.term_slice()) {
                    // Recurse
                    let state = self.get_state()?;
                    self.current_frame = self.push_frame(state)?;
                    break;
                } else {
                    let cmp = self.term_slice().cmp(target);
                    if cmp == std::cmp::Ordering::Less {
                        if self.cur().next_ent == self.cur().ent_count {
                            if !self.cur().is_last_in_floor {
                                let idx = self.current_frame;
                                let (f, mut ctx) = self.split(idx);
                                f.load_next_floor_block(&mut ctx)?;
                                continue;
                            } else {
                                return Ok(());
                            }
                        }
                        continue;
                    } else if cmp == std::cmp::Ordering::Equal {
                        return Ok(());
                    } else {
                        // Fallback to prior entry: the semantics of
                        // this method is that the first call to
                        // next() will return the term after the
                        // requested term
                        let cur = self.cur_mut();
                        cur.next_ent -= 1;
                        cur.last_sub_fp = save_last_sub_fp;
                        cur.start_byte_pos = save_start_byte_pos;
                        cur.suffix = save_suffix;
                        cur.suffixes.set_position(save_pos);
                        cur.term_state.term_block_ord = save_tbo;
                        self.copy_suffix_into_term();
                        self.term_length = self.cur().prefix + self.cur().suffix;
                        // If the last entry was a block we don't
                        // need to bother recursing and pushing to
                        // the last term under it because the first
                        // next() will simply skip the frame anyway
                        return Ok(());
                    }
                }
            }
        }

        java_assert!(false);
        Ok(())
    }

    /// Port of `copyTerm()`.
    fn copy_term(&mut self) {
        let len = self.cur().prefix + self.cur().suffix;
        self.grow_term(len);
        self.copy_suffix_into_term();
        self.term_length = len;
    }

    /// The common-suffix test of `next()`: whether the current entry ends
    /// with `commonSuffixRef`.
    fn matches_common_suffix(&self, common_suffix: &BytesRef) -> Result<bool> {
        let cur = self.cur();
        let term_len = cur.prefix + cur.suffix;
        let common_suffix_bytes = common_suffix.slice();
        if term_len < common_suffix_bytes.len() {
            // No match
            return Ok(false);
        }

        let suffix_bytes = &cur.suffixes.bytes;

        let len_in_prefix = common_suffix_bytes.len() as i64 - cur.suffix as i64;
        java_assert!(common_suffix.offset == 0);
        let mut suffix_bytes_pos;
        let mut common_suffix_bytes_pos = 0usize;

        if len_in_prefix > 0 {
            // A prefix of the common suffix overlaps with
            // the suffix of the block prefix so we first
            // test whether the prefix part matches:
            let mut term_bytes_pos = cur.prefix as i64 - len_in_prefix;
            java_assert!(term_bytes_pos >= 0);
            let term_bytes_pos_end = cur.prefix as i64;
            while term_bytes_pos < term_bytes_pos_end {
                if self.term_bytes[term_bytes_pos as usize]
                    != common_suffix_bytes[common_suffix_bytes_pos]
                {
                    return Ok(false);
                }
                term_bytes_pos += 1;
                common_suffix_bytes_pos += 1;
            }
            suffix_bytes_pos = cur.start_byte_pos;
        } else {
            suffix_bytes_pos = cur.start_byte_pos + cur.suffix - common_suffix_bytes.len();
        }

        // Test overlapping suffix part:
        let common_suffix_bytes_pos_end = common_suffix_bytes.len();
        while common_suffix_bytes_pos < common_suffix_bytes_pos_end {
            if suffix_bytes[suffix_bytes_pos] != common_suffix_bytes[common_suffix_bytes_pos] {
                return Ok(false);
            }
            suffix_bytes_pos += 1;
            common_suffix_bytes_pos += 1;
        }
        Ok(true)
    }
}

impl fmt::Display for OrdsIntersectTermsEnum {
    /// `Object.toString()`, which neither `OrdsIntersectTermsEnum` nor
    /// `BaseTermsEnum` overrides.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&rucene::index::index_reader::java_object_to_string(
            "org.apache.lucene.codecs.blocktreeords.OrdsIntersectTermsEnum",
            self,
        ))
    }
}

fn unsupported() -> LuceneError {
    LuceneError::UnsupportedOperation(String::new())
}

impl TermsEnum for OrdsIntersectTermsEnum {
    fn attributes(&mut self) -> &mut AttributeSource {
        &mut self.attributes
    }

    fn term_state(&mut self) -> Result<Box<dyn TermState>> {
        let fr = self.fr.clone();
        self.cur_mut().decode_meta_data(&fr)?;
        Ok(Box::new(self.cur().term_state))
    }

    fn term(&self) -> Result<BytesRef> {
        Ok(BytesRef::new(self.term_slice().to_vec()))
    }

    // TODO: do we need ord() here?  OrdsIntersectTermsEnumFrame tracks termOrd but it may be buggy!

    fn doc_freq(&mut self) -> Result<i32> {
        let fr = self.fr.clone();
        self.cur_mut().decode_meta_data(&fr)?;
        Ok(self.cur().term_state.doc_freq)
    }

    fn total_term_freq(&mut self) -> Result<i64> {
        let fr = self.fr.clone();
        self.cur_mut().decode_meta_data(&fr)?;
        Ok(self.cur().term_state.total_term_freq)
    }

    fn postings(
        &mut self,
        reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Box<dyn PostingsEnum>> {
        let fr = self.fr.clone();
        self.cur_mut().decode_meta_data(&fr)?;
        let state = self.cur().term_state;
        fr.parent
            .postings_reader
            .postings(&fr.data.field_info, &state, reuse, flags)
    }

    fn impacts(&mut self, flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        let fr = self.fr.clone();
        self.cur_mut().decode_meta_data(&fr)?;
        let state = self.cur().term_state;
        fr.parent
            .postings_reader
            .impacts(&fr.data.field_info, &state, flags)
    }

    fn next(&mut self) -> Result<Option<BytesRef>> {
        'next_term: loop {
            // Pop finished frames
            while self.cur().next_ent == self.cur().ent_count {
                if !self.cur().is_last_in_floor {
                    let idx = self.current_frame;
                    let (f, mut ctx) = self.split(idx);
                    f.load_next_floor_block(&mut ctx)?;
                } else {
                    if self.cur().ord == 0 {
                        return Ok(None);
                    }
                    let last_fp = self.cur().fp_orig;
                    self.current_frame = (self.cur().ord - 1) as usize;
                    java_assert!(self.cur().last_sub_fp == last_fp);
                }
            }

            let is_sub_block = self.cur_mut().next()?;

            if self.cur().suffix != 0 {
                let label = {
                    let cur = self.cur();
                    i32::from(cur.suffixes.bytes[cur.start_byte_pos])
                };
                while label > self.cur().cur_transition_max {
                    if self.cur().transition_index >= self.cur().transition_count - 1 {
                        // Stop processing this frame -- no further
                        // matches are possible because we've moved
                        // beyond what the max transition will allow

                        // sneaky!  forces a pop above
                        let cur = self.cur_mut();
                        cur.is_last_in_floor = true;
                        cur.next_ent = cur.ent_count;
                        continue 'next_term;
                    }
                    let idx = self.current_frame;
                    let (f, ctx) = self.split(idx);
                    f.transition_index += 1;
                    ctx.transition_accessor
                        .get_next_transition(&mut f.transition);
                    f.cur_transition_max = f.transition.max;
                }
            }

            // First test the common suffix, if set:
            if let Some(common_suffix) = self.common_suffix_ref.clone() {
                if !is_sub_block && !self.matches_common_suffix(&common_suffix)? {
                    continue 'next_term;
                }
            }

            // TODO: maybe we should do the same linear test
            // that AutomatonTermsEnum does, so that if we
            // reach a part of the automaton where .* is
            // "temporarily" accepted, we just blindly .next()
            // until the limit

            // See if the term prefix matches the automaton:
            let state = {
                let cur = self.cur();
                let byte_runnable = self
                    .compiled
                    .get_byte_runnable()
                    .expect("INVARIANT: the constructor checked the byte runnable");
                let mut state = cur.state;
                let mut matched = true;
                for idx in 0..cur.suffix {
                    state = byte_runnable.step(
                        state,
                        i32::from(cur.suffixes.bytes[cur.start_byte_pos + idx]),
                    );
                    if state == -1 {
                        // No match
                        matched = false;
                        break;
                    }
                }
                if !matched {
                    continue 'next_term;
                }
                state
            };

            if is_sub_block {
                // Match!  Recurse:
                self.copy_term();
                self.current_frame = self.push_frame(state)?;
            } else if self
                .compiled
                .get_byte_runnable()
                .expect("INVARIANT: the constructor checked the byte runnable")
                .is_accept(state)
            {
                self.copy_term();
                java_assert!(
                    self.saved_start_term
                        .as_deref()
                        .is_none_or(|saved| self.term_slice() > saved),
                    "saveStartTerm={:?} term={:?}",
                    self.saved_start_term,
                    self.term_slice()
                );
                return Ok(Some(BytesRef::new(self.term_slice().to_vec())));
            }
        }
    }

    fn seek_exact(&mut self, _text: &BytesRef) -> Result<bool> {
        Err(unsupported())
    }

    fn seek_ord(&mut self, _ord: i64) -> Result<()> {
        Err(unsupported())
    }

    fn ord(&self) -> Result<i64> {
        Err(unsupported())
    }

    fn seek_ceil(&mut self, _text: &BytesRef) -> Result<SeekStatus> {
        Err(unsupported())
    }
}
