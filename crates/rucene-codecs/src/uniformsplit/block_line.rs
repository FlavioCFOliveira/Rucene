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

//! Port of `org.apache.lucene.codecs.uniformsplit.BlockLine`.

use rucene::codecs::BlockTermState;
use rucene::error::{LuceneError, Result};
use rucene::index::FieldInfo;
use rucene::store::{DataInput, DataOutput};
use rucene::util::{Accountable, BytesRef, RamUsageEstimator};

use super::delta_base_term_state_serializer::DeltaBaseTermStateSerializer;
use super::ram_usage_util;
use super::term_bytes::{suffix_offset, TermBytes};

use crate::blocktreeords::java_assert;

/// One term block line.
///
/// Contains a term and its details as a [`BlockTermState`].
///
/// The line is written to the block file
/// ([`TERMS_BLOCKS_EXTENSION`](super::uniform_split_postings_format::TERMS_BLOCKS_EXTENSION))
/// in two parts. The first part is the term followed by an offset to the
/// details region. The second part is the term [`BlockTermState`], written in
/// the details region, after all the terms of the block.
///
/// The separate details region allows fast scan of the terms without having
/// to decode the details for each term. At read time, [`Serializer::read_line`]
/// only reads the term and its offset to the details. The corresponding
/// [`BlockTermState`] is decoded on demand in the
/// [`BlockReader`](super::BlockReader).
///
/// Port of `org.apache.lucene.codecs.uniformsplit.BlockLine`
/// (`@lucene.experimental`).
#[derive(Debug, Clone)]
pub struct BlockLine {
    pub(crate) term_bytes: TermBytes,
    pub(crate) term_state_relative_offset: i32,
    /// Only used for writing.
    pub(crate) term_state: Option<BlockTermState>,
}

impl BlockLine {
    /// Constructor used for writing a [`BlockLine`].
    ///
    /// Port of `BlockLine(TermBytes, BlockTermState)`.
    pub fn for_writing(term_bytes: TermBytes, term_state: Option<BlockTermState>) -> Self {
        Self {
            term_bytes,
            term_state_relative_offset: -1,
            term_state,
        }
    }

    /// Constructor used for reading a [`BlockLine`].
    ///
    /// Port of `BlockLine(TermBytes, int)`.
    pub fn for_reading(term_bytes: TermBytes, term_state_relative_offset: i32) -> Self {
        Self {
            term_bytes,
            term_state_relative_offset,
            term_state: None,
        }
    }

    /// Port of `BlockLine.getTermBytes()`.
    pub fn get_term_bytes(&self) -> &TermBytes {
        &self.term_bytes
    }

    /// The offset of the `TermState` bytes in the block, relatively to the
    /// term states base offset.
    pub fn get_term_state_relative_offset(&self) -> i32 {
        self.term_state_relative_offset
    }
}

impl Accountable for BlockLine {
    /// Port of `BlockLine.ramBytesUsed()`.
    fn ram_bytes_used(&self) -> i64 {
        RamUsageEstimator::shallow_size_of_instance::<BlockLine>()
            + self.term_bytes.ram_bytes_used()
            + self
                .term_state
                .as_ref()
                .map_or(0, ram_usage_util::ram_bytes_used_term_state)
    }
}

/// The line `Serializer.readLine` read: its term is the serializer's
/// [`current_term`](Serializer::current_term).
///
/// Java's `readLine` returns a `BlockLine` (the reused one) whose
/// `TermBytes` references the serializer's `currentTerm`, the same `BytesRef`
/// for every line; this port keeps that single buffer in the serializer and
/// returns the rest of the line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReadLine {
    /// The line term's MDP length (`getTermBytes().getMdpLength()`).
    pub mdp_length: i32,
    /// `getTermStateRelativeOffset()`.
    pub term_state_relative_offset: i32,
}

impl ReadLine {
    /// `getTermBytes().getSuffixOffset()`.
    pub fn suffix_offset(&self) -> i32 {
        suffix_offset(self.mdp_length)
    }
}

/// Reads/writes block lines with terms encoded incrementally inside a block.
/// This class keeps a state of the previous term read to decode the next
/// term.
///
/// Port of the static nested class `BlockLine.Serializer`.
#[derive(Debug, Clone)]
pub struct Serializer {
    pub(crate) current_term: BytesRef,
}

impl Default for Serializer {
    fn default() -> Self {
        Self::new()
    }
}

impl Serializer {
    /// Port of `Serializer()`: `currentTerm = new BytesRef(64)`.
    pub fn new() -> Self {
        let mut current_term = BytesRef::new(vec![0u8; 64]);
        current_term.length = 0;
        Self { current_term }
    }

    /// The term of the last line read (Java's `currentTerm`).
    pub fn current_term(&self) -> &[u8] {
        &self.current_term.bytes[..self.current_term.length]
    }

    /// Reads the current line.
    ///
    /// `is_incremental_encoding_seed` tells whether the term is a seed of the
    /// incremental encoding: `true` for the first and middle term, `false`
    /// for other terms.
    ///
    /// Port of `Serializer.readLine(DataInput, boolean, BlockLine)`; see
    /// [`ReadLine`].
    ///
    /// # Errors
    ///
    /// `CorruptIndexException` for a negative term state offset; the I/O
    /// errors of the input.
    pub fn read_line(
        &mut self,
        block_input: &mut dyn DataInput,
        is_incremental_encoding_seed: bool,
    ) -> Result<ReadLine> {
        let term_state_relative_offset = block_input.read_v_int()?;
        if term_state_relative_offset < 0 {
            return Err(LuceneError::corrupt_index(
                format!("Illegal termStateRelativeOffset= {term_state_relative_offset}"),
                block_input.to_java_string(),
            ));
        }
        let mdp_length =
            self.read_incrementally_encoded_term(block_input, is_incremental_encoding_seed)?;
        Ok(ReadLine {
            mdp_length,
            term_state_relative_offset,
        })
    }

    /// Writes a line and its offset to the corresponding term state details
    /// in the details region.
    ///
    /// `block_output` points to the block terms region;
    /// `term_state_relative_offset` is the offset to the corresponding term
    /// state details in the details region.
    ///
    /// Port of `Serializer.writeLine(DataOutput, BlockLine, BlockLine, int,
    /// boolean)`.
    ///
    /// # Errors
    ///
    /// The I/O errors of the output.
    pub fn write_line(
        &self,
        block_output: &mut dyn DataOutput,
        line: &BlockLine,
        previous_line: Option<&BlockLine>,
        term_state_relative_offset: i32,
        is_incremental_encoding_seed: bool,
    ) -> Result<()> {
        block_output.write_v_int(term_state_relative_offset)?;
        self.write_incrementally_encoded_term(
            line.get_term_bytes(),
            previous_line.map(BlockLine::get_term_bytes),
            is_incremental_encoding_seed,
            block_output,
        )
    }

    /// Writes the term state details of a line in the details region.
    ///
    /// Port of `Serializer.writeLineTermState(DataOutput, BlockLine,
    /// FieldInfo, DeltaBaseTermStateSerializer)`.
    ///
    /// # Errors
    ///
    /// `AssertionError` (debug builds) for a line without term state; the
    /// I/O errors of the output.
    pub fn write_line_term_state(
        &self,
        term_states_output: &mut dyn DataOutput,
        line: &BlockLine,
        field_info: &FieldInfo,
        encoder: &mut DeltaBaseTermStateSerializer,
    ) -> Result<()> {
        java_assert!(line.term_state.is_some());
        let Some(term_state) = line.term_state.as_ref() else {
            return Err(super::npe(
                "Cannot read field \"docFreq\" because \"termState\" is null",
            ));
        };
        encoder.write_term_state(term_states_output, field_info, term_state)
    }

    /// Port of `Serializer.writeIncrementallyEncodedTerm(TermBytes,
    /// TermBytes, boolean, DataOutput)`.
    ///
    /// # Errors
    ///
    /// `AssertionError` (debug builds) for a missing previous term or an
    /// inconsistent MDP length; the I/O errors of the output.
    pub fn write_incrementally_encoded_term(
        &self,
        term_bytes: &TermBytes,
        previous_term_bytes: Option<&TermBytes>,
        is_incremental_encoding_seed: bool,
        block_output: &mut dyn DataOutput,
    ) -> Result<()> {
        let term = term_bytes.get_term();
        java_assert!(term.offset == 0);
        if is_incremental_encoding_seed {
            // Mdp length is always 1 for an incremental encoding seed.
            block_output.write_v_long(term.length as i64)?;
            block_output.write_bytes(&term.bytes, 0, term.length)?;
            return Ok(());
        }
        if term.length == 0 {
            // Empty term.
            block_output.write_v_long(0)?;
            return Ok(());
        }

        // For other lines we store:
        // - Mdp length.
        // - Suffix length.
        // - Suffix bytes.
        // Instead of writing mdp length and suffix length with 2 VInt, we can compress the storage
        // by merging them in a single VLong. The idea is to leverage the information we have about
        // the previous line. We know the previous line term length. And we know that
        // new line mdp length <= (previous line term length + 1)
        // So if numMdpBits = numBitsToEncode(previous line term length),
        // then we know we can encode (new line mdp length - 1) in numMdpBits.
        // Hence we encode (new line mdp length - 1) in the rightmost numMdpBits of the VLong.
        // And we encode new line suffix length in the remaining left bits of the VLong.
        // Most of the time both values will be encoded in a single byte.

        java_assert!(previous_term_bytes.is_some());
        java_assert!(term_bytes.get_mdp_length() >= 1);
        let Some(previous_term_bytes) = previous_term_bytes else {
            return Err(super::npe(
                "Cannot invoke \"org.apache.lucene.codecs.uniformsplit.TermBytes.getTerm()\" \
                 because \"previousTermBytes\" is null",
            ));
        };

        let num_mdp_bits = num_bits_to_encode(previous_term_bytes.get_term().length as i32);
        java_assert!(num_bits_to_encode(term_bytes.get_mdp_length() - 1) <= num_mdp_bits);

        let mdp_and_suffix_lengths = (i64::from(term_bytes.get_suffix_length())
            .wrapping_shl(num_mdp_bits as u32))
            | i64::from(term_bytes.get_mdp_length() - 1);
        java_assert!(mdp_and_suffix_lengths != 0);
        block_output.write_v_long(mdp_and_suffix_lengths)?;
        block_output.write_bytes(
            &term.bytes,
            term_bytes.get_suffix_offset() as usize,
            term_bytes.get_suffix_length() as usize,
        )
    }

    /// Port of `Serializer.readIncrementallyEncodedTerm(DataInput, boolean,
    /// TermBytes)`: decodes the term into
    /// `current_term` and returns its MDP length.
    ///
    /// # Errors
    ///
    /// The I/O errors of the input.
    pub fn read_incrementally_encoded_term(
        &mut self,
        block_input: &mut dyn DataInput,
        is_incremental_encoding_seed: bool,
    ) -> Result<i32> {
        java_assert!(self.current_term.offset == 0);
        let mdp_length;
        if is_incremental_encoding_seed {
            let length = block_input.read_v_long()? as i32;
            mdp_length = if length == 0 { 0 } else { 1 };
            self.read_bytes(block_input, 0, length)?;
        } else {
            let mdp_and_suffix_lengths = block_input.read_v_long()?;
            if mdp_and_suffix_lengths == 0 {
                // Empty term.
                mdp_length = 0;
                self.current_term.length = 0;
            } else {
                let num_mdp_bits = num_bits_to_encode(self.current_term.length as i32);
                // Get rightmost numMdpBits.
                let mask = i64::from(1i32.wrapping_shl(num_mdp_bits as u32).wrapping_sub(1));
                mdp_length = (mdp_and_suffix_lengths & mask) as i32 + 1;
                // Get remaining left bits.
                let suffix_length =
                    ((mdp_and_suffix_lengths as u64).wrapping_shr(num_mdp_bits as u32)) as i32;
                java_assert!(mdp_length >= 1);
                java_assert!(suffix_length >= 1);
                self.read_bytes(block_input, mdp_length - 1, suffix_length)?;
            }
        }
        Ok(mdp_length)
    }

    /// Reads `length` bytes from the given [`DataInput`] and stores them at
    /// `offset` in the current term's bytes.
    ///
    /// Port of `Serializer.readBytes(DataInput, BytesRef, int, int)` on
    /// `currentTerm`.
    fn read_bytes(&mut self, input: &mut dyn DataInput, offset: i32, length: i32) -> Result<()> {
        let end = offset.wrapping_add(length);
        let (Ok(offset), Ok(length), Ok(end)) = (
            usize::try_from(offset),
            usize::try_from(length),
            usize::try_from(end),
        ) else {
            return Err(LuceneError::unchecked(
                "java.lang.IndexOutOfBoundsException",
                format!("Range [{offset}, {offset} + {length}) out of bounds"),
            ));
        };
        self.current_term.length = end;
        if self.current_term.bytes.len() < end {
            self.current_term.bytes.resize(end, 0);
        }
        input.read_bytes(Some(&mut self.current_term.bytes), offset, length)
    }
}

impl Accountable for Serializer {
    /// Port of `Serializer.ramBytesUsed()`.
    fn ram_bytes_used(&self) -> i64 {
        RamUsageEstimator::shallow_size_of_instance::<Serializer>()
            + ram_usage_util::ram_bytes_used_bytes_ref(&self.current_term)
    }
}

/// Gets the number of bits required to encode the value of the provided int.
/// Returns 0 for int value 0. Equivalent to (log2(i) + 1).
///
/// Port of `Serializer.numBitsToEncode(int)`.
pub fn num_bits_to_encode(i: i32) -> i32 {
    32 - i.leading_zeros() as i32
}
