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

//! Port of `org.apache.lucene.codecs.uniformsplit.sharedterms.STBlockWriter`.

use std::collections::HashSet;

use rucene::error::Result;
use rucene::store::{DataOutput, IndexOutput};
use rucene::util::BytesRef;

use super::field_metadata_term_state::FieldMetadataTermState;
use super::st_block_line::{self, STBlockLine};
use crate::blocktreeords::java_assert;
use crate::uniformsplit::block_encoder::BlockEncoder;
use crate::uniformsplit::block_line;
use crate::uniformsplit::block_writer::{BlockWriter, LineContext};
use crate::uniformsplit::delta_base_term_state_serializer::DeltaBaseTermStateSerializer;
use crate::uniformsplit::field_metadata::FieldMetadata;
use crate::uniformsplit::index_dictionary::Builder;

/// Writes terms blocks with the Shared Terms format.
///
/// As defined in [`STUniformSplitTermsWriter`](super::STUniformSplitTermsWriter),
/// all the fields terms are shared in the same dictionary. Each block line
/// contains a term and all the fields `TermState`s for this term.
///
/// Port of `org.apache.lucene.codecs.uniformsplit.sharedterms.STBlockWriter`
/// (`@lucene.experimental`), which extends `BlockWriter`: the base writer is
/// `base`. Java's `Set<FieldMetadata> fieldsInBlock` holds the
/// indices of the fields in the writer's list (see
/// [`FieldMetadataTermState`]), which every method receives.
pub struct STBlockWriter<'o> {
    pub(crate) base: BlockWriter<'o, STBlockLine>,
    pub(crate) fields_in_block: HashSet<usize>,
}

/// The context of `STBlockWriter`: the writer's field metadata list and the
/// fields of the current block.
struct SharedContext<'c> {
    fields: &'c mut [FieldMetadata],
    fields_in_block: &'c mut HashSet<usize>,
}

impl LineContext<STBlockLine> for SharedContext<'_> {
    /// Port of the term-states half of `STBlockWriter.writeBlockLine`.
    fn write_line_term_states(
        &mut self,
        _block_line_writer: &block_line::Serializer,
        term_states_output: &mut dyn DataOutput,
        line: &STBlockLine,
        term_state_serializer: &mut DeltaBaseTermStateSerializer,
    ) -> Result<()> {
        st_block_line::write_line_term_states(
            term_states_output,
            line,
            term_state_serializer,
            self.fields,
        )?;
        line.collect_fields(self.fields_in_block);
        Ok(())
    }

    /// Port of `STBlockWriter.updateFieldMetadata(long)`.
    fn update_field_metadata(&mut self, block_start_fp: i64) -> Result<()> {
        java_assert!(!self.fields_in_block.is_empty());
        for &index in self.fields_in_block.iter() {
            if let Some(field_metadata) = self.fields.get_mut(index) {
                if field_metadata.get_first_block_start_fp() == -1 {
                    field_metadata.set_first_block_start_fp(block_start_fp);
                }
                field_metadata.set_last_block_start_fp(block_start_fp);
            }
        }
        self.fields_in_block.clear();
        Ok(())
    }
}

impl<'o> STBlockWriter<'o> {
    /// Port of `STBlockWriter(IndexOutput, int, int, BlockEncoder)`.
    pub fn new(
        block_output: &'o mut dyn IndexOutput,
        target_num_block_lines: i32,
        delta_num_lines: i32,
        block_encoder: Option<&'o dyn BlockEncoder>,
    ) -> Self {
        Self {
            base: BlockWriter::new(
                block_output,
                target_num_block_lines,
                delta_num_lines,
                block_encoder,
            ),
            fields_in_block: HashSet::new(),
        }
    }

    /// Adds a new `BlockLine` term for the current field.
    ///
    /// This method determines whether the new term is part of the current
    /// block, or if it is part of the next block. In the latter case, a new
    /// block is started (including one or more of the lastly added lines),
    /// the current block is written to the block file, and the current block
    /// key is added to the [`Builder`].
    ///
    /// `term` is the block line term; `term_states` the block line details
    /// for all fields in the line; `dictionary_builder` the builder to which
    /// the block keys are added; `fields` the writer's field metadata list.
    ///
    /// Port of `STBlockWriter.addLine(BytesRef, List<FieldMetadataTermState>,
    /// IndexDictionary.Builder)`.
    ///
    /// # Errors
    ///
    /// The I/O errors of writing a block; `IllegalArgumentException` for
    /// terms out of order.
    pub fn add_line(
        &mut self,
        term: BytesRef,
        term_states: &[FieldMetadataTermState],
        dictionary_builder: &mut dyn Builder,
        fields: &mut [FieldMetadata],
    ) -> Result<()> {
        if term_states.is_empty() {
            return Ok(());
        }
        let mut context = SharedContext {
            fields,
            fields_in_block: &mut self.fields_in_block,
        };
        self.base.add_term_line(
            term,
            |term_bytes| STBlockLine::new(term_bytes, term_states),
            dictionary_builder,
            &mut context,
        )
    }

    /// Port of `STBlockWriter.finishLastBlock(IndexDictionary.Builder)`.
    ///
    /// # Errors
    ///
    /// The I/O errors of writing the blocks.
    pub fn finish_last_block(
        &mut self,
        dictionary_builder: &mut dyn Builder,
        fields: &mut [FieldMetadata],
    ) -> Result<()> {
        let mut context = SharedContext {
            fields,
            fields_in_block: &mut self.fields_in_block,
        };
        self.base
            .finish_last_block(dictionary_builder, &mut context)
    }
}
