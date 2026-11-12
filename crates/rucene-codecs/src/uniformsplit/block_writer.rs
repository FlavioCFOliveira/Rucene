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

//! Port of `org.apache.lucene.codecs.uniformsplit.BlockWriter`.

use rucene::codecs::BlockTermState;
use rucene::error::{LuceneError, Result};
use rucene::store::{ByteBuffersDataOutput, DataOutput, IndexOutput};
use rucene::util::BytesRef;

use super::block_encoder::BlockEncoder;
use super::block_header::{self, BlockHeader};
use super::block_line::{self, BlockLine};
use super::delta_base_term_state_serializer::DeltaBaseTermStateSerializer;
use super::field_metadata::FieldMetadata;
use super::index_dictionary::Builder;
use super::term_bytes::TermBytes;

use crate::blocktreeords::java_assert;

/// A line the [`BlockWriter`] buffers: Java's `BlockLine` or one of its
/// subclasses (`STBlockLine`).
pub trait WriterLine {
    /// The [`BlockLine`] part of the line.
    fn block_line(&self) -> &BlockLine;
}

impl WriterLine for BlockLine {
    fn block_line(&self) -> &BlockLine {
        self
    }
}

/// The field context of a [`BlockWriter`]: the two methods its subclasses
/// override that depend on the fields being written.
///
/// Java's `BlockWriter` keeps a reference to the `FieldMetadata` of the
/// field being written (`setField`), which `writeBlockLine` and
/// `updateFieldMetadata` read and update; `STBlockWriter` overrides both to
/// work on the fields of each line. Rust cannot keep a mutable reference to
/// metadata the terms writer also updates, so the writer's methods receive
/// the context they act on: [`FieldContext`] for `BlockWriter`, the shared
/// terms context for `STBlockWriter`.
pub trait LineContext<L> {
    /// The term-states half of `writeBlockLine(boolean, BlockLine,
    /// BlockLine)`: writes the line's term state(s) into the details region.
    ///
    /// # Errors
    ///
    /// The I/O errors of the output.
    fn write_line_term_states(
        &mut self,
        block_line_writer: &block_line::Serializer,
        term_states_output: &mut dyn DataOutput,
        line: &L,
        term_state_serializer: &mut DeltaBaseTermStateSerializer,
    ) -> Result<()>;

    /// Port of `BlockWriter.updateFieldMetadata(long)`: updates the field
    /// metadata after all lines were written for the block.
    ///
    /// # Errors
    ///
    /// `AssertionError` (debug builds) when no field is set.
    fn update_field_metadata(&mut self, block_start_fp: i64) -> Result<()>;
}

/// The context of `BlockWriter`: the `FieldMetadata` of the field being
/// written (Java's `setField(FieldMetadata)`).
pub struct FieldContext<'m>(pub &'m mut FieldMetadata);

impl LineContext<BlockLine> for FieldContext<'_> {
    /// Port of the term-states half of `BlockWriter.writeBlockLine`.
    fn write_line_term_states(
        &mut self,
        block_line_writer: &block_line::Serializer,
        term_states_output: &mut dyn DataOutput,
        line: &BlockLine,
        term_state_serializer: &mut DeltaBaseTermStateSerializer,
    ) -> Result<()> {
        let field_info = self.0.field_info_or_npe()?;
        block_line_writer.write_line_term_state(
            term_states_output,
            line,
            field_info,
            term_state_serializer,
        )
    }

    /// Port of `BlockWriter.updateFieldMetadata(long)`.
    fn update_field_metadata(&mut self, block_start_fp: i64) -> Result<()> {
        let field_metadata = &mut *self.0;
        if field_metadata.get_first_block_start_fp() == -1 {
            field_metadata.set_first_block_start_fp(block_start_fp);
        }
        field_metadata.set_last_block_start_fp(block_start_fp);
        Ok(())
    }
}

/// Writes blocks in the block file.
///
/// According the Uniform Split technique, the writing combines three steps
/// per block, and it is repeated for all the field blocks:
///
/// 1. Select the term with the shortest [minimal distinguishing
///    prefix](TermBytes) (MDP) in the neighborhood of the target block size
///    (+- delta size)
/// 2. The selected term becomes the first term of the next block, and its MDP
///    is the next block key.
/// 3. The current block is written to the block file. And its block key is
///    [added](Builder::add) to the index dictionary.
///
/// This stateful [`BlockWriter`] is called repeatedly to
/// [add](Self::add_line) all the [`BlockLine`] terms of a field. Then
/// `finish_last_block` is called. And then this
/// [`BlockWriter`] can be reused to add the terms of another field.
///
/// Port of `org.apache.lucene.codecs.uniformsplit.BlockWriter`
/// (`@lucene.experimental`), generic over the line it buffers so that
/// `STBlockWriter` can reuse it; see [`LineContext`] for the field context
/// its methods receive.
pub struct BlockWriter<'o, L = BlockLine> {
    pub(crate) target_num_block_lines: i32,
    pub(crate) delta_num_lines: i32,
    pub(crate) block_lines: Vec<L>,

    pub(crate) block_output: &'o mut dyn IndexOutput,
    pub(crate) block_lines_write_buffer: ByteBuffersDataOutput,
    pub(crate) term_states_write_buffer: ByteBuffersDataOutput,

    pub(crate) block_header_writer: block_header::Serializer,
    pub(crate) block_line_writer: block_line::Serializer,
    pub(crate) term_state_serializer: DeltaBaseTermStateSerializer,
    pub(crate) block_encoder: Option<&'o dyn BlockEncoder>,
    pub(crate) block_write_buffer: ByteBuffersDataOutput,

    pub(crate) last_term: Option<BytesRef>,

    pub(crate) reusable_block_header: BlockHeader,
}

/// `Math.toIntExact(long)`.
pub(crate) fn to_int_exact(value: usize) -> Result<i32> {
    i32::try_from(value)
        .map_err(|_| LuceneError::unchecked("java.lang.ArithmeticException", "integer overflow"))
}

impl<'o, L: WriterLine> BlockWriter<'o, L> {
    /// Port of `BlockWriter(IndexOutput, int, int, BlockEncoder)`.
    pub fn new(
        block_output: &'o mut dyn IndexOutput,
        target_num_block_lines: i32,
        delta_num_lines: i32,
        block_encoder: Option<&'o dyn BlockEncoder>,
    ) -> Self {
        debug_assert!(target_num_block_lines > 0);
        debug_assert!(delta_num_lines >= 0);
        debug_assert!(delta_num_lines < target_num_block_lines);
        Self {
            target_num_block_lines,
            delta_num_lines,
            block_lines: Vec::with_capacity(target_num_block_lines as usize),
            block_output,
            block_lines_write_buffer: ByteBuffersDataOutput::new_resettable_instance(),
            term_states_write_buffer: ByteBuffersDataOutput::new_resettable_instance(),
            block_header_writer: block_header::Serializer,
            block_line_writer: block_line::Serializer::new(),
            term_state_serializer: DeltaBaseTermStateSerializer::new(),
            block_encoder,
            block_write_buffer: ByteBuffersDataOutput::new_resettable_instance(),
            last_term: None,
            reusable_block_header: BlockHeader::default(),
        }
    }

    /// The protected field `blockLines`: the
    /// lines added and not yet written.
    pub fn block_lines(&self) -> &[L] {
        &self.block_lines
    }

    /// The protected field `lastTerm`, Java's
    /// `null` being `None`.
    pub fn last_term(&self) -> Option<&BytesRef> {
        self.last_term.as_ref()
    }

    /// The protected field `blockLinesWriteBuffer`.
    pub fn block_lines_write_buffer(&self) -> &ByteBuffersDataOutput {
        &self.block_lines_write_buffer
    }

    /// The protected field `termStatesWriteBuffer`.
    pub fn term_states_write_buffer(&self) -> &ByteBuffersDataOutput {
        &self.term_states_write_buffer
    }

    /// The protected field `blockOutput`: the block file output.
    pub fn block_output(&self) -> &dyn IndexOutput {
        &*self.block_output
    }

    /// Adds a line built by `make_line` from the term's [`TermBytes`]: the
    /// shared body of `BlockWriter.addLine` and `STBlockWriter.addLine`.
    pub(crate) fn add_term_line(
        &mut self,
        term: BytesRef,
        make_line: impl FnOnce(TermBytes) -> L,
        dictionary_builder: &mut dyn Builder,
        context: &mut dyn LineContext<L>,
    ) -> Result<()> {
        let mdp_length = TermBytes::compute_mdp_length(self.last_term.as_ref(), &term)?;
        self.block_lines
            .push(make_line(TermBytes::new(mdp_length, term.clone())));
        self.last_term = Some(term);
        if self.block_lines.len() as i64
            >= i64::from(self.target_num_block_lines) + i64::from(self.delta_num_lines)
        {
            self.split_and_write_block(dictionary_builder, context)?;
        }
        Ok(())
    }

    /// This method is called when there is no more term for the field. It
    /// writes the remaining lines added with `addLine` as the last block of
    /// the field and resets this [`BlockWriter`] state. Then this
    /// [`BlockWriter`] can be used for another field.
    ///
    /// Port of `BlockWriter.finishLastBlock(IndexDictionary.Builder)`.
    ///
    /// # Errors
    ///
    /// The I/O errors of writing the blocks.
    pub fn finish_last_block(
        &mut self,
        dictionary_builder: &mut dyn Builder,
        context: &mut dyn LineContext<L>,
    ) -> Result<()> {
        while !self.block_lines.is_empty() {
            self.split_and_write_block(dictionary_builder, context)?;
        }
        self.last_term = None;
        Ok(())
    }

    /// Defines the new block start according to the target number of block
    /// lines and the delta. The new block is started (including one or more
    /// of the lastly added lines), the current block is written to the block
    /// file, and the current block key is added to the
    /// [`Builder`].
    ///
    /// Port of `BlockWriter.splitAndWriteBlock(IndexDictionary.Builder)`.
    ///
    /// # Errors
    ///
    /// The I/O errors of writing the block.
    pub fn split_and_write_block(
        &mut self,
        dictionary_builder: &mut dyn Builder,
        context: &mut dyn LineContext<L>,
    ) -> Result<()> {
        java_assert!(!self.block_lines.is_empty());
        let num_lines = self.block_lines.len() as i32;

        if num_lines <= self.target_num_block_lines - self.delta_num_lines {
            let lines = std::mem::take(&mut self.block_lines);
            self.write_block(&lines, dictionary_builder, context)?;
            self.block_lines = lines;
            self.block_lines.clear();
            return Ok(());
        }
        let delta_start = num_lines - self.delta_num_lines * 2;
        java_assert!(delta_start >= 1, "blockLines size: {num_lines}");
        let mut min_mdp_length = i32::MAX;
        let mut min_mdp_end_index = 0usize;

        for i in delta_start.max(0)..num_lines {
            let term = self.block_lines[i as usize].block_line().get_term_bytes();
            let mdp_length = term.get_mdp_length();
            if mdp_length <= min_mdp_length {
                min_mdp_length = mdp_length;
                min_mdp_end_index = i as usize;
            }
        }

        let lines = std::mem::take(&mut self.block_lines);
        let written = self.write_block(&lines[..min_mdp_end_index], dictionary_builder, context);
        self.block_lines = lines;
        written?;
        // Clear the written block lines to keep only the lines composing the next block.
        self.block_lines.drain(..min_mdp_end_index);
        Ok(())
    }

    /// Writes a block and adds its block key to the dictionary builder.
    ///
    /// Port of `BlockWriter.writeBlock(List, IndexDictionary.Builder)`.
    ///
    /// # Errors
    ///
    /// The I/O errors of writing the block.
    pub fn write_block(
        &mut self,
        block_lines: &[L],
        dictionary_builder: &mut dyn Builder,
        context: &mut dyn LineContext<L>,
    ) -> Result<()> {
        let block_start_fp = self.block_output.file_pointer();

        self.add_block_key(block_lines, dictionary_builder)?;

        let middle = block_lines.len() >> 1;
        let mut middle_offset = -1;
        let mut previous_line: Option<&L> = None;
        for (i, line) in block_lines.iter().enumerate() {
            let mut is_incremental_encoding_seed = i == 0;
            if i == middle {
                middle_offset = to_int_exact(self.block_lines_write_buffer.size())?;
                is_incremental_encoding_seed = true;
            }
            self.write_block_line(is_incremental_encoding_seed, line, previous_line, context)?;
            previous_line = Some(line);
        }

        self.reusable_block_header.reset(
            block_lines.len() as i32,
            self.term_state_serializer.get_base_doc_start_fp(),
            self.term_state_serializer.get_base_pos_start_fp(),
            self.term_state_serializer.get_base_pay_start_fp(),
            to_int_exact(self.block_lines_write_buffer.size())?,
            middle_offset,
        );
        self.block_header_writer
            .write(&mut self.block_write_buffer, &self.reusable_block_header)?;

        self.block_lines_write_buffer
            .copy_to(&mut self.block_write_buffer)?;
        self.term_states_write_buffer
            .copy_to(&mut self.block_write_buffer)?;

        match self.block_encoder {
            None => {
                self.block_output
                    .write_v_int(to_int_exact(self.block_write_buffer.size())?)?;
                self.block_write_buffer.copy_to(&mut *self.block_output)?;
            }
            Some(block_encoder) => {
                let encoded_bytes = block_encoder.encode(
                    &mut self.block_write_buffer.to_data_input()?,
                    self.block_write_buffer.size() as i64,
                )?;
                let size = usize::try_from(encoded_bytes.size()).unwrap_or(usize::MAX);
                self.block_output.write_v_int(to_int_exact(size)?)?;
                encoded_bytes.write_to(&mut *self.block_output)?;
            }
        }

        self.block_lines_write_buffer.reset()?;
        self.term_states_write_buffer.reset()?;
        self.block_write_buffer.reset()?;

        self.term_state_serializer.reset_base_start_fp();

        context.update_field_metadata(block_start_fp)
    }

    /// Port of `BlockWriter.writeBlockLine(boolean, BlockLine, BlockLine)`:
    /// the line, then its term state(s) through the context.
    ///
    /// # Errors
    ///
    /// The I/O errors of the buffers.
    pub fn write_block_line(
        &mut self,
        is_incremental_encoding_seed: bool,
        line: &L,
        previous_line: Option<&L>,
        context: &mut dyn LineContext<L>,
    ) -> Result<()> {
        self.block_line_writer.write_line(
            &mut self.block_lines_write_buffer,
            line.block_line(),
            previous_line.map(WriterLine::block_line),
            to_int_exact(self.term_states_write_buffer.size())?,
            is_incremental_encoding_seed,
        )?;
        context.write_line_term_states(
            &self.block_line_writer,
            &mut self.term_states_write_buffer,
            line,
            &mut self.term_state_serializer,
        )
    }

    /// Adds a new block key with its corresponding block file pointer to the
    /// [`Builder`]. The block key is the MDP (see [`TermBytes`]) of the block
    /// first term.
    ///
    /// Port of `BlockWriter.addBlockKey(List, IndexDictionary.Builder)`.
    ///
    /// # Errors
    ///
    /// The errors of the dictionary builder.
    pub fn add_block_key(
        &mut self,
        block_lines: &[L],
        dictionary_builder: &mut dyn Builder,
    ) -> Result<()> {
        java_assert!(!block_lines.is_empty());
        let first_term = block_lines[0].block_line().get_term_bytes();
        java_assert!(first_term.get_term().offset == 0);
        let mdp_length = first_term.get_mdp_length().max(0) as usize;
        let scratch_bytes_ref = BytesRef::new(first_term.get_term().bytes[..mdp_length].to_vec());
        dictionary_builder.add(&scratch_bytes_ref, self.block_output.file_pointer())
    }
}

impl<'o> BlockWriter<'o, BlockLine> {
    /// Adds a new [`BlockLine`] term for the current field.
    ///
    /// This method determines whether the new term is part of the current
    /// block, or if it is part of the next block. In the latter case, a new
    /// block is started (including one or more of the lastly added lines),
    /// the current block is written to the block file, and the current block
    /// key is added to the [`Builder`].
    ///
    /// `term` is the block line term (Java uses the instance directly, the
    /// caller is responsible to make a deep copy if needed);
    /// `block_term_state` the block line details; `dictionary_builder` the
    /// builder to which the block keys are added; `context` the field
    /// being written (Java's `setField`).
    ///
    /// Port of `BlockWriter.addLine(BytesRef, BlockTermState,
    /// IndexDictionary.Builder)`.
    ///
    /// # Errors
    ///
    /// The I/O errors of writing a block; `IllegalArgumentException` for
    /// terms out of order.
    pub fn add_line(
        &mut self,
        term: BytesRef,
        block_term_state: BlockTermState,
        dictionary_builder: &mut dyn Builder,
        context: &mut FieldContext<'_>,
    ) -> Result<()> {
        self.add_term_line(
            term,
            |term_bytes| BlockLine::for_writing(term_bytes, Some(block_term_state)),
            dictionary_builder,
            context,
        )
    }
}
