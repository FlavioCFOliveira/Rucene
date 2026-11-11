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

//! Port of `org.apache.lucene.codecs.uniformsplit.IntersectBlockReader`.

use std::fmt;
use std::sync::Arc;

use rucene::codecs::PostingsReaderBase;
use rucene::error::{LuceneError, Result};
use rucene::index::index_reader::java_object_to_string;
use rucene::index::{ImpactsEnum, PostingsEnum, SeekStatus, TermState, TermsEnum};
use rucene::util::attribute::AttributeSource;
use rucene::util::automaton::{ByteRunnable, CompiledAutomaton, Transition, TransitionAccessor};
use rucene::util::BytesRef;

use super::block_decoder::BlockDecoder;
use super::block_reader::BlockReader;
use super::field_metadata::FieldMetadata;
use super::index_dictionary::BrowserSupplier;

use crate::blocktreeords::{java_assert, TermBuilder};

/// Block iteration order. Whether to move next block, jump to a block away,
/// or end the iteration.
///
/// Port of the protected enum `IntersectBlockReader.BlockIteration`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockIteration {
    /// `NEXT`.
    Next,
    /// `SEEK`.
    Seek,
    /// `END`.
    End,
}

/// Threshold that controls when to attempt to jump to a block away.
///
/// This counter is 0 when entering a block. It is incremented each time a
/// term is rejected by the automaton. When the counter is greater than or
/// equal to this threshold, then we compute the next term accepted by the
/// automaton, with [`AutomatonNextTermCalculator`], and we jump to a block
/// away if the next term accepted is greater than the immediate next term in
/// the block.
///
/// A low value, for example 1, improves the performance of automatons
/// requiring many jumps, for example `FuzzyQuery` and most `WildcardQuery`.
/// A higher value improves the performance of automatons with less or no
/// jump, for example `PrefixQuery`. A threshold of 4 seems to be a good
/// balance.
///
/// Port of `IntersectBlockReader.NUM_CONSECUTIVELY_REJECTED_TERMS_THRESHOLD`.
pub const NUM_CONSECUTIVELY_REJECTED_TERMS_THRESHOLD: i32 = 4;

/// The "intersect" `TermsEnum` response to `UniformSplitTerms.intersect`,
/// intersecting the terms with an automaton.
///
/// By design of the UniformSplit block keys, it is less efficient than
/// `org.apache.lucene.backward_codecs.lucene40.blocktree.IntersectTermsEnum`
/// for `FuzzyQuery` (-37%). It is slightly slower for `WildcardQuery` (-5%)
/// and slightly faster for `PrefixQuery` (+5%).
///
/// Port of `org.apache.lucene.codecs.uniformsplit.IntersectBlockReader`
/// (`@lucene.experimental`), which extends `BlockReader`: the base reader is
/// `base`. The automaton's `ByteRunnable` and
/// `TransitionAccessor` are read from the compiled automaton the reader
/// owns.
pub struct IntersectBlockReader {
    pub(crate) base: BlockReader,

    pub(crate) compiled: CompiledAutomaton,
    pub(crate) finite: bool,
    /// Maybe `None`.
    pub(crate) common_suffix: Option<Vec<u8>>,
    pub(crate) min_term_length: i32,
    pub(crate) next_string_calculator: AutomatonNextTermCalculator,

    /// Set this when our current mode is seeking to this term. Set to `None`
    /// after.
    pub(crate) seek_term: Option<Vec<u8>>,

    /// Number of bytes accepted by the automaton when validating the current
    /// term.
    pub(crate) num_matched_bytes: i32,

    /// Automaton states reached when validating the current term, from 0 to
    /// `num_matched_bytes - 1`.
    pub(crate) states: Vec<i32>,

    /// Block iteration order determined when scanning the terms in the
    /// current block.
    pub(crate) block_iteration: BlockIteration,

    /// Counter of the number of consecutively rejected terms. Depending on
    /// [`NUM_CONSECUTIVELY_REJECTED_TERMS_THRESHOLD`], this may trigger a jump
    /// to a block away.
    pub(crate) num_consecutively_rejected_terms: i32,
}

fn byte_runnable(compiled: &CompiledAutomaton) -> Result<&dyn ByteRunnable> {
    compiled.get_byte_runnable().ok_or_else(|| {
        super::npe(
            "Cannot invoke \"org.apache.lucene.util.automaton.ByteRunnable.isAccept(int)\" \
             because \"this.byteRunnable\" is null",
        )
    })
}

fn transition_accessor(compiled: &CompiledAutomaton) -> Result<&dyn TransitionAccessor> {
    compiled.get_transition_accessor().ok_or_else(|| {
        super::npe(
            "Cannot invoke \"org.apache.lucene.util.automaton.TransitionAccessor.\
             getNumTransitions(int)\" because \"this.transitionAccessor\" is null",
        )
    })
}

impl IntersectBlockReader {
    /// Port of `IntersectBlockReader(CompiledAutomaton, BytesRef,
    /// IndexDictionary.BrowserSupplier, IndexInput, PostingsReaderBase,
    /// FieldMetadata, BlockDecoder)`.
    ///
    /// # Errors
    ///
    /// The errors of the base reader's constructor.
    pub fn new(
        compiled: &CompiledAutomaton,
        start_term: Option<&BytesRef>,
        dictionary_browser_supplier: Arc<dyn BrowserSupplier>,
        block_input: Arc<rucene::store::SharedIndexInput>,
        postings_reader: Arc<dyn PostingsReaderBase>,
        field_metadata: Arc<FieldMetadata>,
        block_decoder: Option<Arc<dyn BlockDecoder>>,
    ) -> Result<Self> {
        let base = BlockReader::new(
            dictionary_browser_supplier,
            block_input,
            postings_reader,
            field_metadata,
            block_decoder,
        )?;
        let compiled = compiled.clone();
        let finite = compiled.finite;
        let common_suffix = compiled
            .common_suffix_ref
            .as_ref()
            .map(|s| s.slice().to_vec());
        let next_string_calculator = AutomatonNextTermCalculator::new(&compiled)?;
        let mut reader = Self {
            base,
            compiled,
            finite,
            common_suffix,
            min_term_length: 0,
            next_string_calculator,
            seek_term: start_term.map(|t| t.slice().to_vec()),
            num_matched_bytes: 0,
            states: Vec::new(),
            block_iteration: BlockIteration::Next,
            num_consecutively_rejected_terms: 0,
        };
        reader.min_term_length = reader.get_min_term_length()?;
        Ok(reader)
    }

    /// Computes the minimal length of the terms accepted by the automaton.
    /// This speeds up the term scanning for automatons accepting a finite
    /// language.
    ///
    /// Port of `IntersectBlockReader.getMinTermLength()`.
    ///
    /// # Errors
    ///
    /// `NullPointerException` for an automaton without runnable.
    pub fn get_min_term_length(&self) -> Result<i32> {
        // Automatons accepting infinite language (e.g. PrefixQuery and WildcardQuery) do not
        // benefit much from min term length while it takes time to compute it. More precisely, by
        // skipping this computation PrefixQuery is significantly boosted while WildcardQuery might
        // be slightly degraded on average. This min term length mainly boosts FuzzyQuery.
        let common_suffix_length = self.common_suffix.as_ref().map_or(0, |s| s.len() as i32);
        if !self.finite {
            return Ok(common_suffix_length);
        }
        let byte_runnable = byte_runnable(&self.compiled)?;
        let transition_accessor = transition_accessor(&self.compiled)?;
        // Since we are only dealing with finite language, there is no loop to detect.
        let mut common_prefix_length = 0;
        let mut state = 0;
        let mut t: Option<Transition> = None;
        loop {
            if byte_runnable.is_accept(state) {
                // The common prefix reaches a final state. So common prefix and common suffix
                // overlap. Min term length is the max between common prefix and common suffix
                // lengths.
                return Ok(common_prefix_length.max(common_suffix_length));
            }
            if transition_accessor.get_num_transitions(state) == 1 {
                let t = t.get_or_insert_with(Transition::new);
                transition_accessor.get_transition(state, 0, t);
                if t.min == t.max {
                    state = t.dest;
                    common_prefix_length += 1;
                    continue;
                }
            }
            break;
        }
        // Min term length is the sum of common prefix and common suffix lengths.
        Ok(common_prefix_length + common_suffix_length)
    }

    /// Port of `IntersectBlockReader.next()`.
    ///
    /// # Errors
    ///
    /// `CorruptIndexException` for a missing block; I/O errors.
    pub fn next_impl(&mut self) -> Result<Option<BytesRef>> {
        if self.base.block_header.is_none() {
            if !self.seek_first_block()? {
                return Ok(None);
            }
            self.states = vec![0; 32];
            self.block_iteration = BlockIteration::Next;
        }
        self.base.term_state_set = false;
        loop {
            if let Some(term) = self.next_term_in_block_matching()? {
                return Ok(Some(term));
            }
            if !self.next_block()? {
                return Ok(None);
            }
        }
    }

    /// Port of `IntersectBlockReader.seekFirstBlock()`.
    ///
    /// # Errors
    ///
    /// I/O errors.
    pub fn seek_first_block(&mut self) -> Result<bool> {
        let byte_runnable = byte_runnable(&self.compiled)?;
        let transition_accessor = transition_accessor(&self.compiled)?;
        self.seek_term = self.next_string_calculator.next_seek_term(
            self.seek_term.as_deref(),
            byte_runnable,
            transition_accessor,
        )?;
        let Some(seek_term) = self.seek_term.clone() else {
            return Ok(false);
        };
        let mut block_start_fp = self
            .base
            .get_or_create_dictionary_browser()?
            .seek_block(&BytesRef::new(seek_term.clone()))?;
        if block_start_fp == -1 {
            block_start_fp = self.base.field_metadata.get_first_block_start_fp();
        } else if self.base.is_beyond_last_term(&seek_term, block_start_fp) {
            return Ok(false);
        }
        self.base
            .initialize_header(Some(&seek_term), block_start_fp)?;
        Ok(self.base.block_header.is_some())
    }

    /// Finds the next block line that matches (accepted by the automaton), or
    /// `None` when at end of block.
    ///
    /// Returns the next term in the current block that is accepted by the
    /// automaton, or `None` if none.
    ///
    /// Port of `IntersectBlockReader.nextTermInBlockMatching()`.
    ///
    /// # Errors
    ///
    /// I/O errors.
    pub fn next_term_in_block_matching(&mut self) -> Result<Option<BytesRef>> {
        match self.seek_term.take() {
            None => {
                if self.base.read_line_in_block()?.is_none() {
                    return Ok(None);
                }
            }
            Some(seek_term) => {
                let seek_status = self.base.seek_in_block(&seek_term)?;
                if seek_status == SeekStatus::END {
                    return Ok(None);
                }
                java_assert!(self.num_matched_bytes == 0);
                java_assert!(self.num_consecutively_rejected_terms == 0);
            }
        }
        let compiled = &self.compiled;
        let byte_runnable = byte_runnable(compiled)?;
        let transition_accessor = transition_accessor(compiled)?;
        loop {
            let Some(line) = self.base.block_line else {
                return Err(super::npe(
                    "Cannot invoke \"org.apache.lucene.codecs.uniformsplit.BlockLine.\
                     getTermBytes()\" because \"this.blockLine\" is null",
                ));
            };
            let line_term = self.base.block_line_reader.current_term();
            let line_term_length = line_term.len();
            if self.states.len() <= line_term_length {
                self.states.resize(oversize(line_term_length + 1), 0);
            }
            // Since terms are delta encoded, we may start the automaton steps from the last state
            // reached by the previous term.
            let mut index = line.suffix_offset().min(self.num_matched_bytes) as usize;
            // Skip this term early if it is shorter than the min term length, or if it does not end
            // with the common suffix accepted by the automaton.
            if line_term_length as i64 >= i64::from(self.min_term_length)
                && self
                    .common_suffix
                    .as_ref()
                    .is_none_or(|suffix| line_term.ends_with(suffix))
            {
                let mut state = self.states[index];
                loop {
                    if index == line_term_length {
                        if byte_runnable.is_accept(state) {
                            // The automaton accepts the current term. Record the number of matched
                            // bytes and return the term.
                            java_assert!(byte_runnable.run(line_term));
                            self.num_matched_bytes = index as i32;
                            if self.num_consecutively_rejected_terms > 0 {
                                self.num_consecutively_rejected_terms = 0;
                            }
                            java_assert!(self.block_iteration == BlockIteration::Next);
                            return Ok(Some(BytesRef::new(line_term.to_vec())));
                        }
                        break;
                    }
                    state = byte_runnable.step(state, i32::from(line_term[index]));
                    if state == -1 {
                        // The automaton rejects the current term.
                        break;
                    }
                    // Record the reached automaton state.
                    index += 1;
                    self.states[index] = state;
                }
            }
            // The current term is not accepted by the automaton.
            // Still record the reached automaton state to start the next term steps from there.
            java_assert!(!byte_runnable.run(line_term));
            self.num_matched_bytes = index as i32;
            // If the number of consecutively rejected terms reaches the threshold,
            // then determine whether it is worthwhile to jump to a block away.
            self.num_consecutively_rejected_terms =
                self.num_consecutively_rejected_terms.wrapping_add(1);
            let lines_count = self.base.header()?.get_lines_count();
            if self.num_consecutively_rejected_terms >= NUM_CONSECUTIVELY_REJECTED_TERMS_THRESHOLD
                && self.base.line_index_in_block < lines_count - 1
                && !self.next_string_calculator.is_linear_state(line_term)
            {
                // Compute the next term accepted by the automaton after the current term.
                let line_term = line_term.to_vec();
                self.seek_term = self.next_string_calculator.next_seek_term(
                    Some(&line_term),
                    byte_runnable,
                    transition_accessor,
                )?;
                let Some(seek_term) = self.seek_term.as_ref() else {
                    self.block_iteration = BlockIteration::End;
                    return Ok(None);
                };
                // It is worthwhile to jump to a block away if the next term accepted is after the
                // next term in the block.
                // Actually the block away may be the current block, but this is a good heuristic.
                let seek_term = seek_term.clone();
                self.base.read_line_in_block()?;
                if self.base.block_line.is_none() {
                    return Err(super::npe(
                        "Cannot invoke \"org.apache.lucene.codecs.uniformsplit.BlockLine.\
                         getTermBytes()\" because \"this.blockLine\" is null",
                    ));
                }
                if seek_term.as_slice() > self.base.block_line_reader.current_term() {
                    // Stop scanning this block terms and set the iteration order to jump to a block
                    // away by seeking seekTerm.
                    self.block_iteration = BlockIteration::Seek;
                    return Ok(None);
                }
                self.seek_term = None;
                // If it is not worthwhile to jump to a block away, do not attempt anymore for the
                // current block.
                self.num_consecutively_rejected_terms = i32::MIN;
            } else if self.base.read_line_in_block()?.is_none() {
                // No more terms in the block. The iteration order is to open the very next block.
                java_assert!(self.block_iteration == BlockIteration::Next);
                return Ok(None);
            }
        }
    }

    /// Indicates whether the given term ends with the automaton common
    /// suffix. This allows to quickly skip terms that the automaton would
    /// reject eventually.
    ///
    /// Port of `IntersectBlockReader.endsWithCommonSuffix(byte[], int)`.
    pub fn ends_with_common_suffix(&self, term_bytes: &[u8]) -> bool {
        self.common_suffix
            .as_ref()
            .is_some_and(|suffix| term_bytes.ends_with(suffix))
    }

    /// Opens the next block. Depending on the block iteration order, it may
    /// be the very next block, or a block away that may contain the seek
    /// term.
    ///
    /// Returns `true` if the next block is opened, `false` if there is no
    /// blocks anymore and the iteration is over.
    ///
    /// Port of `IntersectBlockReader.nextBlock()`.
    ///
    /// # Errors
    ///
    /// I/O errors.
    pub fn next_block(&mut self) -> Result<bool> {
        let block_start_fp = match self.block_iteration {
            BlockIteration::Next => {
                java_assert!(self.seek_term.is_none());
                self.base.input()?.file_pointer()
            }
            BlockIteration::Seek => {
                java_assert!(self.seek_term.is_some());
                let seek_term = self.seek_term.clone().unwrap_or_default();
                let block_start_fp = self
                    .base
                    .get_or_create_dictionary_browser()?
                    .seek_block(&BytesRef::new(seek_term.clone()))?;
                if self.base.is_beyond_last_term(&seek_term, block_start_fp) {
                    return Ok(false);
                }
                self.block_iteration = BlockIteration::Next;
                block_start_fp
            }
            BlockIteration::End => return Ok(false),
        };
        self.num_matched_bytes = 0;
        self.num_consecutively_rejected_terms = 0;
        let seek_term = self.seek_term.clone();
        self.base
            .initialize_header(seek_term.as_deref(), block_start_fp)?;
        Ok(self.base.block_header.is_some())
    }
}

/// `ArrayUtil.oversize(minTargetSize, Byte.BYTES)` on a 64-bit JVM.
fn oversize(min_target_size: usize) -> usize {
    let extra = (min_target_size >> 3).max(3);
    let new_size = min_target_size + extra;
    // Byte.BYTES: round up to a multiple of 8.
    (new_size + 7) & !7
}

impl fmt::Display for IntersectBlockReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&java_object_to_string(
            "org.apache.lucene.codecs.uniformsplit.IntersectBlockReader",
            self,
        ))
    }
}

fn unsupported() -> LuceneError {
    LuceneError::unsupported_operation(String::new())
}

impl TermsEnum for IntersectBlockReader {
    fn attributes(&mut self) -> &mut AttributeSource {
        &mut self.base.attributes
    }

    fn seek_exact(&mut self, _text: &BytesRef) -> Result<bool> {
        Err(unsupported())
    }

    fn seek_ceil(&mut self, _text: &BytesRef) -> Result<SeekStatus> {
        Err(unsupported())
    }

    fn seek_ord(&mut self, _ord: i64) -> Result<()> {
        Err(unsupported())
    }

    fn seek_term_state(&mut self, _text: &BytesRef, _state: &dyn TermState) -> Result<()> {
        Err(unsupported())
    }

    fn term(&self) -> Result<Option<BytesRef>> {
        Ok(self.base.term_bytes())
    }

    fn ord(&self) -> Result<i64> {
        Err(unsupported())
    }

    fn doc_freq(&mut self) -> Result<i32> {
        self.base.doc_freq_impl()
    }

    fn total_term_freq(&mut self) -> Result<i64> {
        self.base.total_term_freq_impl()
    }

    fn postings(
        &mut self,
        reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Option<Box<dyn PostingsEnum>>> {
        Ok(Some(self.base.postings_impl(reuse, flags)?))
    }

    fn impacts(&mut self, flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        self.base.impacts_impl(flags)
    }

    fn term_state(&mut self) -> Result<Box<dyn TermState>> {
        self.base.term_state_impl()
    }

    fn next(&mut self) -> Result<Option<BytesRef>> {
        self.next_impl()
    }
}

/// This is mostly a copy of `AutomatonTermsEnum`. Since it's an inner class,
/// the outer class can call methods that ATE does not expose. It'd be nice if
/// ATE's logic could be more extensible.
///
/// Port of the protected inner class
/// `IntersectBlockReader.AutomatonNextTermCalculator`; the outer reader's
/// `byteRunnable`, `transitionAccessor` and `finite` are passed to (or kept
/// by) it.
pub struct AutomatonNextTermCalculator {
    /// For path tracking: each short records gen when we last visited the
    /// state; we use gens to avoid having to clear.
    visited: Option<Vec<i16>>,
    cur_gen: i16,
    /// The reference used for seeking forwards through the term dictionary.
    seek_bytes_ref: TermBuilder,
    /// True if we are enumerating an infinite portion of the DFA. In this
    /// case it is faster to drive the query based on the terms dictionary.
    /// When this is true, `linear_upper_bound` indicate the end of range of
    /// terms where we should simply do sequential reads instead.
    linear: bool,
    linear_upper_bound: Vec<u8>,
    transition: Transition,
    saved_states: Vec<i32>,
    finite: bool,
}

impl AutomatonNextTermCalculator {
    /// Port of `AutomatonNextTermCalculator(CompiledAutomaton)`.
    ///
    /// # Errors
    ///
    /// `NullPointerException` for an infinite automaton without runnable.
    pub fn new(compiled: &CompiledAutomaton) -> Result<Self> {
        let visited = if compiled.finite {
            None
        } else {
            Some(vec![0i16; byte_runnable(compiled)?.size().max(0) as usize])
        };
        Ok(Self {
            visited,
            cur_gen: 0,
            seek_bytes_ref: TermBuilder::default(),
            linear: false,
            linear_upper_bound: Vec::new(),
            transition: Transition::new(),
            saved_states: Vec::new(),
            finite: compiled.finite,
        })
    }

    /// Records the given state has been visited.
    fn set_visited(&mut self, state: i32) {
        if !self.finite {
            if let Some(visited) = self.visited.as_mut() {
                let state = state as usize;
                if state >= visited.len() {
                    let new_len = (state + 1)
                        .max(visited.len() + (visited.len() >> 3))
                        .max(state + 1);
                    visited.resize(new_len, 0);
                }
                visited[state] = self.cur_gen;
            }
        }
    }

    /// Indicates whether the given state has been visited.
    fn is_visited(&self, state: i32) -> bool {
        !self.finite
            && self
                .visited
                .as_ref()
                .is_some_and(|v| (state as usize) < v.len() && v[state as usize] == self.cur_gen)
    }

    /// True if the current state of the automata is best iterated linearly
    /// (without seeking).
    ///
    /// Port of `AutomatonNextTermCalculator.isLinearState(BytesRef)`.
    pub fn is_linear_state(&self, term: &[u8]) -> bool {
        self.linear && term < self.linear_upper_bound.as_slice()
    }

    /// Port of `AutomatonNextTermCalculator.nextSeekTerm(BytesRef)` (see
    /// `FilteredTermsEnum.nextSeekTerm`); returns a copy of the seek bytes
    /// Java returns.
    ///
    /// # Errors
    ///
    /// `ArrayIndexOutOfBoundsException` where Java's `byteAt` raises it.
    pub fn next_seek_term(
        &mut self,
        term: Option<&[u8]>,
        byte_runnable: &dyn ByteRunnable,
        transition_accessor: &dyn TransitionAccessor,
    ) -> Result<Option<Vec<u8>>> {
        match term {
            None => {
                java_assert!(self.seek_bytes_ref.length == 0);
                // return the empty term, as it's valid
                if byte_runnable.is_accept(0) {
                    return Ok(Some(self.seek_bytes_ref.get().to_vec()));
                }
            }
            Some(term) => self.seek_bytes_ref.copy_bytes(term),
        }

        // seek to the next possible string;
        if self.next_string(byte_runnable, transition_accessor)? {
            Ok(Some(self.seek_bytes_ref.get().to_vec())) // reposition
        } else {
            Ok(None) // no more possible strings can match
        }
    }

    /// Sets the enum to operate in linear fashion, as we have found a looping
    /// transition at position: we set an upper bound and act like a
    /// `TermRangeQuery` for this portion of the term space.
    ///
    /// Port of `AutomatonNextTermCalculator.setLinear(int)`.
    fn set_linear(
        &mut self,
        position: usize,
        byte_runnable: &dyn ByteRunnable,
        transition_accessor: &dyn TransitionAccessor,
    ) -> Result<()> {
        java_assert!(!self.linear);

        let mut state = 0;
        let mut max_interval = 0xff;
        for i in 0..position {
            state = byte_runnable.step(state, i32::from(self.seek_bytes_ref.byte_at(i)?));
            java_assert!(state >= 0, "state={state}");
        }
        let num_transitions = transition_accessor.get_num_transitions(state);
        transition_accessor.init_transition(state, &mut self.transition);
        let c = i32::from(self.seek_bytes_ref.byte_at(position)?);
        for _ in 0..num_transitions {
            transition_accessor.get_next_transition(&mut self.transition);
            if self.transition.min <= c && c <= self.transition.max {
                max_interval = self.transition.max;
                break;
            }
        }
        // 0xff terms don't get the optimization... not worth the trouble.
        if max_interval != 0xff {
            max_interval += 1;
        }
        let length = position + 1; /* position + maxTransition */
        self.linear_upper_bound.clear();
        self.linear_upper_bound
            .extend_from_slice(&self.seek_bytes_ref.bytes[..position]);
        self.linear_upper_bound.push(max_interval as u8);
        debug_assert_eq!(self.linear_upper_bound.len(), length);

        self.linear = true;
        Ok(())
    }

    /// Increments the byte buffer to the next String in binary order after s
    /// that will not put the machine into a reject state. If such a string
    /// does not exist, returns false.
    ///
    /// The correctness of this method depends upon the automaton being
    /// deterministic, and having no transitions to dead states.
    ///
    /// Returns true if more possible solutions exist for the DFA.
    ///
    /// Port of `AutomatonNextTermCalculator.nextString()`.
    fn next_string(
        &mut self,
        byte_runnable: &dyn ByteRunnable,
        transition_accessor: &dyn TransitionAccessor,
    ) -> Result<bool> {
        let mut state;
        let mut pos = 0usize;
        let needed = self.seek_bytes_ref.length + 1;
        if self.saved_states.len() < needed {
            self.saved_states.resize(needed, 0);
        }
        self.saved_states[0] = 0;

        loop {
            if !self.finite {
                self.cur_gen = self.cur_gen.wrapping_add(1);
                if self.cur_gen == 0 {
                    // Clear the visited states every time curGen wraps (so very infrequently to
                    // not impact average perf).
                    if let Some(visited) = self.visited.as_mut() {
                        visited.fill(-1);
                    }
                }
            }
            self.linear = false;
            // walk the automaton until a character is rejected.
            state = self.saved_states[pos];
            while pos < self.seek_bytes_ref.length {
                self.set_visited(state);
                let next_state =
                    byte_runnable.step(state, i32::from(self.seek_bytes_ref.byte_at(pos)?));
                if next_state == -1 {
                    break;
                }
                self.saved_states[pos + 1] = next_state;
                // we found a loop, record it for faster enumeration
                if !self.linear && self.is_visited(next_state) {
                    self.set_linear(pos, byte_runnable, transition_accessor)?;
                }
                state = next_state;
                pos += 1;
            }

            // take the useful portion, and the last non-reject state, and attempt to
            // append characters that will match.
            if self.next_string_from(state, pos, byte_runnable, transition_accessor)? {
                return Ok(true);
            } else {
                /* no more solutions exist from this useful portion, backtrack */
                let Some(back) = self.backtrack(pos)? else {
                    /* no more solutions at all */
                    return Ok(false);
                };
                pos = back;
                let new_state = byte_runnable.step(
                    self.saved_states[pos],
                    i32::from(self.seek_bytes_ref.byte_at(pos)?),
                );
                if new_state >= 0 && byte_runnable.is_accept(new_state) {
                    /* String is good to go as-is */
                    return Ok(true);
                }
                /* else advance further */
                // paranoia? if we backtrack thru an infinite DFA, the loop detection is important!
                // for now, restart from scratch for all infinite DFAs
                if !self.finite {
                    pos = 0;
                }
            }
        }
    }

    /// Returns the next String in lexicographic order that will not put the
    /// machine into a reject state.
    ///
    /// This method traverses the DFA from the given position in the String,
    /// starting at the given state.
    ///
    /// If this cannot satisfy the machine, returns false. This method will
    /// walk the minimal path, in lexicographic order, as long as possible.
    ///
    /// If this method returns false, then there might still be more
    /// solutions, it is necessary to backtrack to find out.
    ///
    /// `state` is the current non-reject state; `position` the useful
    /// portion of the string. Returns true if more possible solutions exist
    /// for the DFA from this position.
    ///
    /// Port of `AutomatonNextTermCalculator.nextString(int, int)`.
    fn next_string_from(
        &mut self,
        mut state: i32,
        position: usize,
        byte_runnable: &dyn ByteRunnable,
        transition_accessor: &dyn TransitionAccessor,
    ) -> Result<bool> {
        /*
         * the next lexicographic character must be greater than the existing
         * character, if it exists.
         */
        let mut c = 0;
        if position < self.seek_bytes_ref.length {
            c = i32::from(self.seek_bytes_ref.byte_at(position)?);
            // if the next byte is 0xff and is not part of the useful portion,
            // then by definition it puts us in a reject state, and therefore this
            // path is dead. there cannot be any higher transitions. backtrack.
            let was = c;
            c += 1;
            if was == 0xff {
                return Ok(false);
            }
        }

        self.seek_bytes_ref.length = position;
        self.set_visited(state);

        let num_transitions = transition_accessor.get_num_transitions(state);
        transition_accessor.init_transition(state, &mut self.transition);
        // find the minimal path (lexicographic order) that is >= c

        for _ in 0..num_transitions {
            transition_accessor.get_next_transition(&mut self.transition);
            if self.transition.max >= c {
                let next_char = c.max(self.transition.min);
                // append either the next sequential char, or the minimum transition
                self.append(next_char as u8);
                state = self.transition.dest;
                /*
                 * as long as is possible, continue down the minimal path in
                 * lexicographic order. if a loop or accept state is encountered, stop.
                 */
                while !self.is_visited(state) && !byte_runnable.is_accept(state) {
                    self.set_visited(state);
                    /*
                     * Note: we work with a DFA with no transitions to dead states.
                     * so the below is ok, if it is not an accept state,
                     * then there MUST be at least one transition.
                     */
                    transition_accessor.init_transition(state, &mut self.transition);
                    transition_accessor.get_next_transition(&mut self.transition);
                    state = self.transition.dest;

                    // append the minimum transition
                    self.append(self.transition.min as u8);

                    // we found a loop, record it for faster enumeration
                    if !self.linear && self.is_visited(state) {
                        let at = self.seek_bytes_ref.length - 1;
                        self.set_linear(at, byte_runnable, transition_accessor)?;
                    }
                }
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// `seekBytesRef.append(byte)`.
    fn append(&mut self, b: u8) {
        let length = self.seek_bytes_ref.length;
        self.seek_bytes_ref.grow(length + 1);
        self.seek_bytes_ref.bytes[length] = b;
        self.seek_bytes_ref.length = length + 1;
    }

    /// Attempts to backtrack thru the string after encountering a dead end at
    /// some given position. Returns `None` if no more possible strings can
    /// match.
    ///
    /// `position` is the current position in the input String. Returns
    /// `Some(position)` (Java's `position >= 0`) if more possible solutions
    /// exist for the DFA.
    ///
    /// Port of `AutomatonNextTermCalculator.backtrack(int)`.
    fn backtrack(&mut self, mut position: usize) -> Result<Option<usize>> {
        while position > 0 {
            position -= 1;
            let next_char = i32::from(self.seek_bytes_ref.byte_at(position)?);
            // if a character is 0xff it's a dead-end too,
            // because there is no higher character in binary sort order.
            if next_char != 0xff {
                self.seek_bytes_ref
                    .set_byte_at(position, (next_char + 1) as u8);
                self.seek_bytes_ref.length = position + 1;
                return Ok(Some(position));
            }
        }
        Ok(None) /* all solutions exhausted */
    }
}
