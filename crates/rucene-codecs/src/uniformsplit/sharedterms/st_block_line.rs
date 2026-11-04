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

//! Port of `org.apache.lucene.codecs.uniformsplit.sharedterms.STBlockLine`.

use std::collections::{HashMap, HashSet};

use rucene::codecs::BlockTermState;
use rucene::error::Result;
use rucene::index::{FieldInfo, FieldInfos};
use rucene::store::{DataInput, DataOutput};

use super::field_metadata_term_state::FieldMetadataTermState;
use crate::blocktreeords::java_assert;
use crate::uniformsplit::block_header::BlockHeader;
use crate::uniformsplit::block_line::BlockLine;
use crate::uniformsplit::block_writer::WriterLine;
use crate::uniformsplit::delta_base_term_state_serializer::DeltaBaseTermStateSerializer;
use crate::uniformsplit::field_metadata::{field_info_by_number, FieldMetadata};
use crate::uniformsplit::term_bytes::TermBytes;

/// Represents a term and its details stored in the [`BlockTermState`]. It is
/// an extension of [`BlockLine`] for the Shared Terms format. This means the
/// line contains a term and all its fields `TermState`s.
///
/// Port of `org.apache.lucene.codecs.uniformsplit.sharedterms.STBlockLine`
/// (`@lucene.experimental`), which extends `BlockLine`: the base line is
/// `line`.
#[derive(Debug, Clone)]
pub struct STBlockLine {
    pub(crate) line: BlockLine,
    /// List of the fields ond their TermStates for this block line. Only
    /// used for writing.
    pub(crate) term_states: Vec<FieldMetadataTermState>,
}

impl STBlockLine {
    /// Port of `STBlockLine(TermBytes, List<FieldMetadataTermState>)`.
    pub fn new(term_bytes: TermBytes, term_states: &[FieldMetadataTermState]) -> Self {
        debug_assert!(!term_states.is_empty());
        Self {
            line: BlockLine::for_writing(term_bytes, None),
            term_states: term_states.to_vec(),
        }
    }

    /// Collects the `FieldMetadata` of all fields listed in this line.
    ///
    /// Port of `STBlockLine.collectFields(Collection<FieldMetadata>)`; the
    /// collector receives the fields' indices (see
    /// [`FieldMetadataTermState`]).
    pub fn collect_fields(&self, collector: &mut HashSet<usize>) {
        for field_term_state in &self.term_states {
            collector.insert(field_term_state.field_metadata());
        }
    }
}

impl WriterLine for STBlockLine {
    fn block_line(&self) -> &BlockLine {
        &self.line
    }
}

/// The `FieldInfo` of a field metadata of the writer's list.
fn field_info_of(fields: &[FieldMetadata], index: usize) -> Result<&FieldInfo> {
    fields
        .get(index)
        .ok_or_else(|| {
            crate::uniformsplit::npe(
                "Cannot invoke \"org.apache.lucene.codecs.uniformsplit.FieldMetadata.\
                 getFieldInfo()\" because the return value of \
                 \"org.apache.lucene.codecs.uniformsplit.sharedterms.FieldMetadataTermState.\
                 fieldMetadata()\" is null",
            )
        })?
        .field_info_or_npe()
}

/// Writes all the [`BlockTermState`] of the provided [`STBlockLine`] to the
/// given output; `fields` is the writer's list the line's states index.
///
/// Port of `STBlockLine.Serializer.writeLineTermStates(DataOutput,
/// STBlockLine, DeltaBaseTermStateSerializer)`. Java's
/// `STBlockLine.Serializer` extends `BlockLine.Serializer` with methods that
/// use none of its state; they are this module's functions.
///
/// # Errors
///
/// `AssertionError` (debug builds) for a line without term state; the I/O
/// errors of the output.
pub fn write_line_term_states(
    term_states_output: &mut dyn DataOutput,
    line: &STBlockLine,
    encoder: &mut DeltaBaseTermStateSerializer,
    fields: &[FieldMetadata],
) -> Result<()> {
    let size = line.term_states.len();
    java_assert!(size > 0, "not valid block line with :{size} lines.");
    if size == 1 {
        // When there is only 1 field, write its id as negative, followed by the field TermState.
        let field_metadata_term_state = &line.term_states[0];
        let field_info = field_info_of(fields, field_metadata_term_state.field_metadata())?;
        term_states_output.write_z_int(field_info.number.wrapping_neg())?;
        return encoder.write_term_state(
            term_states_output,
            field_info,
            field_metadata_term_state.state(),
        );
    }

    term_states_output.write_z_int(size as i32)?;
    // First iteration writes the fields ids.
    for field_metadata_term_state in &line.term_states {
        let field_info = field_info_of(fields, field_metadata_term_state.field_metadata())?;
        term_states_output.write_v_int(field_info.number)?;
    }
    // Second iteration writes the corresponding field TermStates.
    for field_metadata_term_state in &line.term_states {
        let field_info = field_info_of(fields, field_metadata_term_state.field_metadata())?;
        encoder.write_term_state(
            term_states_output,
            field_info,
            field_metadata_term_state.state(),
        )?;
    }
    Ok(())
}

/// `fieldInfos.fieldInfo(id)` handed to `readTermState`, which dereferences
/// it.
fn field_info_for_read(field_infos: &FieldInfos, field_id: i32) -> Result<&FieldInfo> {
    field_info_by_number(field_infos, field_id)?.ok_or_else(|| {
        crate::uniformsplit::npe(
            "Cannot invoke \"org.apache.lucene.index.FieldInfo.getIndexOptions()\" because \
             \"fieldInfo\" is null",
        )
    })
}

/// Reads a single [`BlockTermState`] for the provided field in the current
/// block line of the provided input.
///
/// `term_states_input` is the data input to read the [`BlockTermState`]
/// from; `block_header` the current block header; `reuse` a previous
/// [`BlockTermState`] to reuse, or `None` to create a new one. Returns the
/// [`BlockTermState`] corresponding to the provided field id, or `None` if
/// the field does not occur in the line.
///
/// Port of `STBlockLine.Serializer.readTermStateForField(int, DataInput,
/// DeltaBaseTermStateSerializer, BlockHeader, FieldInfos, BlockTermState)`.
///
/// # Errors
///
/// The I/O errors of the input.
pub fn read_term_state_for_field(
    field_id: i32,
    term_states_input: &mut dyn DataInput,
    term_state_serializer: &DeltaBaseTermStateSerializer,
    block_header: &BlockHeader,
    field_infos: &FieldInfos,
    mut reuse: Option<&mut BlockTermState>,
) -> Result<Option<BlockTermState>> {
    java_assert!(field_id >= 0);
    let num_fields = term_states_input.read_z_int()?;
    if num_fields <= 0 {
        let read_field_id = num_fields.wrapping_neg();
        if field_id == read_field_id {
            return term_state_serializer
                .read_term_state(
                    block_header.get_base_docs_fp(),
                    block_header.get_base_positions_fp(),
                    block_header.get_base_payloads_fp(),
                    term_states_input,
                    field_info_for_read(field_infos, read_field_id)?,
                    reuse,
                )
                .map(Some);
        }
        return Ok(None);
    }

    // There are multiple fields for the term.
    // We have to read all the field ids (aka field numbers) sequentially.
    // Then if the required field is in the list, we have to read all the TermState
    // sequentially. This could be optimized with a jump-to-middle offset
    // for example, but we don't need that currently.

    let mut is_field_in_list = false;
    let mut read_field_ids = vec![0i32; num_fields as usize];
    for slot in read_field_ids.iter_mut() {
        let read_field_id = term_states_input.read_v_int()?;
        if !is_field_in_list && read_field_id > field_id {
            // As the list of fieldIds is sorted we can return early if we find fieldId greater
            // than the seeked one.
            // But if we found the seeked one, we have to read all the list to get to the term
            // state part afterward (there is no jump offset).
            return Ok(None);
        }
        is_field_in_list |= read_field_id == field_id;
        *slot = read_field_id;
    }
    if is_field_in_list {
        for read_field_id in read_field_ids {
            let term_state = term_state_serializer.read_term_state(
                block_header.get_base_docs_fp(),
                block_header.get_base_positions_fp(),
                block_header.get_base_payloads_fp(),
                term_states_input,
                field_info_for_read(field_infos, read_field_id)?,
                reuse.as_deref_mut(),
            )?;
            if field_id == read_field_id {
                return Ok(Some(term_state));
            }
        }
    }
    Ok(None)
}

/// Reads all the [`BlockTermState`] of all the field in the current block
/// line of the provided input.
///
/// `field_term_states_map` is filled with the term states for each field.
/// It is cleared first.
///
/// Port of `STBlockLine.Serializer.readFieldTermStatesMap(DataInput,
/// DeltaBaseTermStateSerializer, BlockHeader, FieldInfos, Map)`.
///
/// # Errors
///
/// The I/O errors of the input.
pub fn read_field_term_states_map(
    term_states_input: &mut dyn DataInput,
    term_state_serializer: &DeltaBaseTermStateSerializer,
    block_header: &BlockHeader,
    field_infos: &FieldInfos,
    field_term_states_map: &mut HashMap<String, BlockTermState>,
) -> Result<()> {
    field_term_states_map.clear();
    let num_fields = term_states_input.read_z_int()?;
    if num_fields <= 0 {
        let field_id = num_fields.wrapping_neg();
        let field_info = field_info_name_of(field_infos, field_id)?;
        let state = term_state_serializer.read_term_state(
            block_header.get_base_docs_fp(),
            block_header.get_base_positions_fp(),
            block_header.get_base_payloads_fp(),
            term_states_input,
            field_info,
            None,
        )?;
        field_term_states_map.insert(field_info.name.clone(), state);
        return Ok(());
    }
    for field_id in read_field_ids(term_states_input, num_fields)? {
        let field_info = field_info_name_of(field_infos, field_id)?;
        let state = term_state_serializer.read_term_state(
            block_header.get_base_docs_fp(),
            block_header.get_base_positions_fp(),
            block_header.get_base_payloads_fp(),
            term_states_input,
            field_info,
            None,
        )?;
        field_term_states_map.insert(field_info.name.clone(), state);
    }
    Ok(())
}

/// `fieldInfos.fieldInfo(id).name`.
fn field_info_name_of(field_infos: &FieldInfos, field_id: i32) -> Result<&FieldInfo> {
    field_info_by_number(field_infos, field_id)?.ok_or_else(|| {
        crate::uniformsplit::npe(
            "Cannot read field \"name\" because the return value of \
             \"org.apache.lucene.index.FieldInfos.fieldInfo(int)\" is null",
        )
    })
}

/// Reads all the field ids in the current block line of the provided input.
///
/// Port of `STBlockLine.Serializer.readFieldIds(DataInput, int)`.
///
/// # Errors
///
/// The I/O errors of the input; `NegativeArraySizeException` for a
/// negative count.
pub fn read_field_ids(term_states_input: &mut dyn DataInput, num_fields: i32) -> Result<Vec<i32>> {
    let mut field_ids = vec![0i32; crate::blocktreeords::array_len(num_fields)?];
    for slot in field_ids.iter_mut() {
        *slot = term_states_input.read_v_int()?;
    }
    Ok(field_ids)
}
