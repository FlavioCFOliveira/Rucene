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

//! Port of `org.apache.lucene.codecs.uniformsplit.DeltaBaseTermStateSerializer`.

use rucene::codecs::{BlockTermState, BLOCK_SIZE};
use rucene::error::Result;
use rucene::index::{FieldInfo, IndexOptions};
use rucene::store::{DataInput, DataOutput};
use rucene::util::{Accountable, RamUsageEstimator};

use crate::blocktreeords::java_assert;

/// `TermState` serializer which encodes each file pointer as a delta
/// relative to a base file pointer. It differs from
/// `Lucene104PostingsWriter.encodeTerm` which encodes each file pointer as a
/// delta relative to the previous file pointer.
///
/// It automatically sets the base file pointer to the first valid file
/// pointer for doc start FP, pos start FP, pay start FP. These base file
/// pointers have to be [reset](Self::reset_base_start_fp) by the caller
/// before starting to write a new block.
///
/// Port of `org.apache.lucene.codecs.uniformsplit.DeltaBaseTermStateSerializer`
/// (`@lucene.experimental`). Java's `IntBlockTermState` is the port's
/// [`BlockTermState`], which carries the `Lucene104` fields.
#[derive(Debug, Clone, Default)]
pub struct DeltaBaseTermStateSerializer {
    pub(crate) base_doc_start_fp: i64,
    pub(crate) base_pos_start_fp: i64,
    pub(crate) base_pay_start_fp: i64,
}

impl DeltaBaseTermStateSerializer {
    /// Port of `DeltaBaseTermStateSerializer()`.
    pub fn new() -> Self {
        let mut s = Self::default();
        s.reset_base_start_fp();
        s
    }

    /// Resets the base file pointers to 0. This method has to be called
    /// before starting to write a new block.
    pub fn reset_base_start_fp(&mut self) {
        self.base_doc_start_fp = 0;
        self.base_pos_start_fp = 0;
        self.base_pay_start_fp = 0;
    }

    /// The base doc start file pointer. It is the file pointer of the first
    /// `TermState` written after `reset_base_start_fp`
    /// is called.
    pub fn get_base_doc_start_fp(&self) -> i64 {
        self.base_doc_start_fp
    }

    /// The base position start file pointer. It is the file pointer of the
    /// first `TermState` written after
    /// `reset_base_start_fp` is called.
    pub fn get_base_pos_start_fp(&self) -> i64 {
        self.base_pos_start_fp
    }

    /// The base payload start file pointer. It is the file pointer of the
    /// first `TermState` written after
    /// `reset_base_start_fp` is called.
    pub fn get_base_pay_start_fp(&self) -> i64 {
        self.base_pay_start_fp
    }

    /// Writes a [`BlockTermState`] to the provided [`DataOutput`].
    ///
    /// Simpler variant of `Lucene104PostingsWriter.encodeTerm(DataOutput,
    /// FieldInfo, BlockTermState, boolean)`.
    ///
    /// Port of `writeTermState(DataOutput, FieldInfo, BlockTermState)`.
    ///
    /// # Errors
    ///
    /// `AssertionError` (debug builds) for a total term frequency below the
    /// document frequency; the I/O errors of the output.
    pub fn write_term_state(
        &mut self,
        term_states_output: &mut dyn DataOutput,
        field_info: &FieldInfo,
        term_state: &BlockTermState,
    ) -> Result<()> {
        let index_options = field_info.get_index_options();
        let has_freqs = index_options != IndexOptions::DOCS;
        let has_positions = index_options.subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS);
        let has_offsets =
            index_options.subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS);
        let has_payloads = field_info.has_payloads();

        let int_term_state = term_state;

        term_states_output.write_v_int(int_term_state.doc_freq)?;
        if has_freqs {
            java_assert!(int_term_state.total_term_freq >= i64::from(int_term_state.doc_freq));
            term_states_output.write_v_long(
                int_term_state.total_term_freq - i64::from(int_term_state.doc_freq),
            )?;
        }

        if int_term_state.singleton_doc_id != -1 {
            term_states_output.write_v_int(int_term_state.singleton_doc_id)?;
        } else {
            if self.base_doc_start_fp == 0 {
                self.base_doc_start_fp = int_term_state.doc_start_fp;
            }
            term_states_output
                .write_v_long(int_term_state.doc_start_fp - self.base_doc_start_fp)?;
        }

        if has_positions {
            if self.base_pos_start_fp == 0 {
                self.base_pos_start_fp = int_term_state.pos_start_fp;
            }
            term_states_output
                .write_v_long(int_term_state.pos_start_fp - self.base_pos_start_fp)?;
            if has_payloads || has_offsets {
                if self.base_pay_start_fp == 0 {
                    self.base_pay_start_fp = int_term_state.pay_start_fp;
                }
                term_states_output
                    .write_v_long(int_term_state.pay_start_fp - self.base_pay_start_fp)?;
            }
            if int_term_state.last_pos_block_offset != -1 {
                term_states_output.write_v_long(int_term_state.last_pos_block_offset)?;
            }
        }
        Ok(())
    }

    /// Reads a [`BlockTermState`] from the provided [`DataInput`].
    ///
    /// Simpler variant of `Lucene104PostingsReader.decodeTerm(DataInput,
    /// FieldInfo, BlockTermState, boolean)`.
    ///
    /// Port of `readTermState(long, long, long, DataInput, FieldInfo,
    /// BlockTermState)`: `reuse`, when given, is [reset](Self::reset) and
    /// filled (Java returns the reused instance); otherwise a new state is
    /// filled and returned.
    ///
    /// # Errors
    ///
    /// `AssertionError` (debug builds) for a total term frequency below the
    /// document frequency; the I/O errors of the input.
    #[allow(clippy::too_many_arguments)]
    pub fn read_term_state(
        &self,
        base_doc_start_fp: i64,
        base_pos_start_fp: i64,
        base_pay_start_fp: i64,
        term_states_input: &mut dyn DataInput,
        field_info: &FieldInfo,
        reuse: Option<&mut BlockTermState>,
    ) -> Result<BlockTermState> {
        let index_options = field_info.get_index_options();
        let has_freqs = index_options != IndexOptions::DOCS;
        let has_positions = index_options.subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS);

        let mut fresh = BlockTermState::default();
        let int_term_state: &mut BlockTermState = match reuse {
            Some(reuse) => self.reset(reuse),
            None => &mut fresh,
        };

        int_term_state.doc_freq = term_states_input.read_v_int()?;
        int_term_state.total_term_freq = if has_freqs {
            i64::from(int_term_state.doc_freq).wrapping_add(term_states_input.read_v_long()?)
        } else {
            i64::from(int_term_state.doc_freq)
        };
        java_assert!(int_term_state.total_term_freq >= i64::from(int_term_state.doc_freq));

        if int_term_state.doc_freq == 1 {
            int_term_state.singleton_doc_id = term_states_input.read_v_int()?;
        } else {
            int_term_state.doc_start_fp =
                base_doc_start_fp.wrapping_add(term_states_input.read_v_long()?);
        }

        if has_positions {
            int_term_state.pos_start_fp =
                base_pos_start_fp.wrapping_add(term_states_input.read_v_long()?);
            let has_offsets =
                index_options.subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS);
            if has_offsets || field_info.has_payloads() {
                int_term_state.pay_start_fp =
                    base_pay_start_fp.wrapping_add(term_states_input.read_v_long()?);
            }
            if int_term_state.total_term_freq > i64::from(BLOCK_SIZE) {
                int_term_state.last_pos_block_offset = term_states_input.read_v_long()?;
            }
        }
        Ok(*int_term_state)
    }

    /// Port of `DeltaBaseTermStateSerializer.reset(IntBlockTermState)`.
    pub fn reset<'s>(&self, term_state: &'s mut BlockTermState) -> &'s mut BlockTermState {
        // OrdTermState.
        term_state.ord = 0;

        // BlockTermState.
        term_state.doc_freq = 0;
        term_state.total_term_freq = 0;
        term_state.term_block_ord = 0;
        term_state.block_file_pointer = 0;

        // IntBlockTermState.
        term_state.doc_start_fp = 0;
        term_state.pos_start_fp = 0;
        term_state.pay_start_fp = 0;
        term_state.last_pos_block_offset = -1;
        term_state.singleton_doc_id = -1;

        term_state
    }

    /// The estimated RAM usage of the given `TermState`.
    ///
    /// Port of the static `DeltaBaseTermStateSerializer.ramBytesUsed(TermState)`;
    /// every term state of the port is an `IntBlockTermState`.
    pub fn ram_bytes_used_of(_term_state: &BlockTermState) -> i64 {
        RamUsageEstimator::shallow_size_of_instance::<BlockTermState>()
    }
}

impl Accountable for DeltaBaseTermStateSerializer {
    /// Port of `DeltaBaseTermStateSerializer.ramBytesUsed()`.
    fn ram_bytes_used(&self) -> i64 {
        RamUsageEstimator::shallow_size_of_instance::<DeltaBaseTermStateSerializer>()
    }
}
