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

//! Port of `org.apache.lucene.codecs.uniformsplit.sharedterms.STUniformSplitTermsWriter`.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::sync::Arc;

use rucene::codecs::codec_util::write_footer;
use rucene::codecs::postings::default_merge;
use rucene::codecs::{
    BlockTermState, FieldsConsumer, MergeState, NormsProducer, PostingsWriterBase,
    SegmentWriteState,
};
use rucene::error::Result;
use rucene::index::merge::DocMap;
use rucene::index::{FieldInfos, Fields, PostingsEnum, TermsEnum};
use rucene::store::ByteBuffersDataOutput;
use rucene::util::BytesRef;

use super::field_metadata_term_state::FieldMetadataTermState;
use super::st_block_writer::STBlockWriter;
use super::st_merging_block_reader::{MergingPostings, STMergingBlockReader};
use super::st_merging_terms_enum::STMergingTermsEnum;
use super::st_uniform_split_postings_format::{
    NAME, TERMS_BLOCKS_EXTENSION, TERMS_DICTIONARY_EXTENSION, VERSION_CURRENT,
};
use super::st_uniform_split_terms::STUniformSplitTerms;
use crate::uniformsplit::block_encoder::BlockEncoder;
use crate::uniformsplit::field_metadata::FieldMetadata;
use crate::uniformsplit::fst_dictionary;
use crate::uniformsplit::index_dictionary::Builder;
use crate::uniformsplit::npe;
use crate::uniformsplit::uniform_split_terms_writer::{
    PostingParts, UniformSplitTermsWriter, WriterParameters, WriterParts, DEFAULT_DELTA_NUM_LINES,
    DEFAULT_TARGET_NUM_BLOCK_LINES,
};

/// The parameters of `STUniformSplitTermsWriter`.
pub const PARAMETERS: WriterParameters = WriterParameters {
    codec_name: NAME,
    version_current: VERSION_CURRENT,
    terms_blocks_extension: TERMS_BLOCKS_EXTENSION,
    dictionary_extension: TERMS_DICTIONARY_EXTENSION,
};

/// Extends [`UniformSplitTermsWriter`] by sharing all the fields terms in the
/// same dictionary and by writing all the fields of a term in the same block
/// line.
///
/// The block file (`.stustb`) contains all the term blocks for all fields.
/// Each block line, for a single term, may have multiple fields `TermState`.
/// The block file also contains the fields metadata at the end of the file.
///
/// The dictionary file (`.stustd`) contains a single trie (`FST` bytes) for
/// all fields.
///
/// This structure is adapted when there are lots of fields. In this case the
/// shared-terms dictionary trie is much smaller.
///
/// This `FieldsConsumer` requires a custom `merge` method for efficiency. The
/// regular merge would scan all the fields sequentially, which internally
/// would scan the whole shared-terms dictionary as many times as there are
/// fields. Whereas the custom merge directly scans the internal shared-terms
/// dictionary of all segments to merge, thus scanning once whatever the
/// number of fields is.
///
/// Port of `org.apache.lucene.codecs.uniformsplit.sharedterms.STUniformSplitTermsWriter`
/// (`@lucene.experimental`), which extends `UniformSplitTermsWriter`: the
/// base writer is `base`. Java's `TermIteratorQueue` (a
/// Lucene `PriorityQueue` ordered by term, then field number or segment
/// index) is a min-heap on the same keys, which are unique: both pop the
/// same order. The field metadata the block lines reference are the indices
/// of the list `writeSegment` collects (see [`FieldMetadataTermState`]).
pub struct STUniformSplitTermsWriter {
    pub(crate) base: UniformSplitTermsWriter,
}

impl STUniformSplitTermsWriter {
    /// Port of `STUniformSplitTermsWriter(PostingsWriterBase,
    /// SegmentWriteState, BlockEncoder)`.
    ///
    /// # Errors
    ///
    /// See [`UniformSplitTermsWriter::with_settings`].
    pub fn new(
        postings_writer: Box<dyn PostingsWriterBase>,
        state: &SegmentWriteState<'_>,
        block_encoder: Option<Arc<dyn BlockEncoder>>,
    ) -> Result<Self> {
        Self::with_settings(
            postings_writer,
            state,
            DEFAULT_TARGET_NUM_BLOCK_LINES,
            DEFAULT_DELTA_NUM_LINES,
            block_encoder,
        )
    }

    /// Port of `STUniformSplitTermsWriter(PostingsWriterBase,
    /// SegmentWriteState, int, int, BlockEncoder)`.
    ///
    /// # Errors
    ///
    /// See [`UniformSplitTermsWriter::with_settings`].
    pub fn with_settings(
        postings_writer: Box<dyn PostingsWriterBase>,
        state: &SegmentWriteState<'_>,
        target_num_block_lines: i32,
        delta_num_lines: i32,
        block_encoder: Option<Arc<dyn BlockEncoder>>,
    ) -> Result<Self> {
        Ok(Self {
            base: UniformSplitTermsWriter::with_parameters(
                postings_writer,
                state,
                target_num_block_lines,
                delta_num_lines,
                block_encoder,
                PARAMETERS,
            )?,
        })
    }

    /// Writes the new segment with the provided terms writer, which can be
    /// either a single segment writer, or a multiple segment merging writer.
    ///
    /// Port of `STUniformSplitTermsWriter.writeSegment(SharedTermsWriter)`.
    fn write_segment(
        &mut self,
        terms_writer: impl FnOnce(
            &mut STBlockWriter<'_>,
            &mut dyn Builder,
            &mut PostingParts<'_>,
            &FieldInfos,
        ) -> Result<Vec<FieldMetadata>>,
    ) -> Result<()> {
        let target_num_block_lines = self.base.target_num_block_lines;
        let delta_num_lines = self.base.delta_num_lines;
        let parts = self.base.parts()?;
        let block_encoder = parts.block_encoder;
        let (block_output, mut postings, field_infos) = parts.split();
        let mut dictionary_builder = fst_dictionary::Builder::new()?;
        let mut field_metadata_list;
        {
            let mut block_writer = STBlockWriter::new(
                &mut *block_output,
                target_num_block_lines,
                delta_num_lines,
                block_encoder,
            );
            field_metadata_list = terms_writer(
                &mut block_writer,
                &mut dictionary_builder,
                &mut postings,
                field_infos,
            )?;
            block_writer.finish_last_block(&mut dictionary_builder, &mut field_metadata_list)?;
        }
        let mut parts = WriterParts {
            field_infos,
            postings_writer: postings.postings_writer,
            max_doc: postings.max_doc,
            block_encoder,
            field_metadata_writer: postings.field_metadata_writer,
            block_output,
            dictionary_output: postings.dictionary_output,
        };
        let fields_number = write_field_metadata_list(&mut parts, &field_metadata_list)?;
        write_dictionary(&mut parts, fields_number, &mut dictionary_builder)
    }
}

/// Port of `STUniformSplitTermsWriter.createFieldMetadataList(Iterator<FieldInfo>, int)`.
fn create_field_metadata_list<'f>(
    field_infos: impl Iterator<Item = Option<&'f rucene::index::FieldInfo>>,
    max_doc: i32,
    postings: &PostingParts<'_>,
) -> Result<Vec<FieldMetadata>> {
    let mut field_metadata_list = Vec::new();
    for field_info in field_infos {
        let Some(field_info) = field_info else {
            // `new FieldMetadata(null, maxDoc)` fails at `createFieldTermsQueue`'s
            // `fieldMetadata.getFieldInfo().name`, before anything is written.
            return Err(npe(
                "Cannot read field \"name\" because the return value of \
                 \"org.apache.lucene.codecs.uniformsplit.FieldMetadata.getFieldInfo()\" is null",
            ));
        };
        let mut field_metadata = FieldMetadata::new(field_info.clone(), max_doc);
        field_metadata.set_dictionary_start_fp(postings.dictionary_output.file_pointer());
        field_metadata_list.push(field_metadata);
    }
    Ok(field_metadata_list)
}

/// A heap entry of Java's `TermIteratorQueue`: the iterator's term, its
/// secondary key (`compareSecondary`: the field number or the segment index)
/// and the iterator's index.
type QueueKey = Reverse<(Vec<u8>, i64, usize)>;

/// Port of `STUniformSplitTermsWriter.groupByTerm(TermIteratorQueue,
/// TermIterator, List)`: pops the iterators on the top iterator's term.
fn group_by_term(
    term_iterator_queue: &mut BinaryHeap<QueueKey>,
    top: (Vec<u8>, i64, usize),
    grouped_term_iterators: &mut Vec<usize>,
) {
    grouped_term_iterators.clear();
    grouped_term_iterators.push(top.2);
    while let Some(Reverse(term_iterator)) = term_iterator_queue.peek() {
        if term_iterator.0 != top.0 {
            return;
        }
        // Same term for another iterator. Combine the iterators.
        grouped_term_iterators.push(term_iterator.2);
        term_iterator_queue.pop();
    }
}

/// Port of the private class `STUniformSplitTermsWriter.FieldTerms`.
struct FieldTerms {
    field_metadata: usize,
    terms_enum: Box<dyn TermsEnum>,
    term: Option<BytesRef>,
}

impl FieldTerms {
    /// Port of `FieldTerms.nextTerm()`.
    fn next_term(&mut self) -> Result<bool> {
        self.term = self.terms_enum.next()?;
        Ok(self.term.is_some())
    }
}

/// Port of `STUniformSplitTermsWriter.writeSingleSegment(Fields,
/// NormsProducer, STBlockWriter, IndexDictionary.Builder)`.
fn write_single_segment(
    fields: &dyn Fields,
    norms_producer: Option<&dyn NormsProducer>,
    block_writer: &mut STBlockWriter<'_>,
    dictionary_builder: &mut dyn Builder,
    postings: &mut PostingParts<'_>,
    field_infos: &FieldInfos,
) -> Result<Vec<FieldMetadata>> {
    let names: Vec<String> = fields.iterator().collect();
    let mut field_metadata_list = create_field_metadata_list(
        names.iter().map(|name| field_infos.field_info(name)),
        postings.max_doc,
        postings,
    )?;
    // createFieldTermsQueue
    let mut iterators: Vec<FieldTerms> = Vec::new();
    let mut field_terms_queue: BinaryHeap<QueueKey> = BinaryHeap::new();
    for (index, field_metadata) in field_metadata_list.iter().enumerate() {
        if let Some(terms) = fields.terms(field_metadata.field_name()?)? {
            let mut field_terms = FieldTerms {
                field_metadata: index,
                terms_enum: terms.iterator()?,
                term: None,
            };
            if field_terms.next_term()? {
                // There is at least one term for the field.
                field_terms_queue.push(field_key(
                    &field_terms,
                    &field_metadata_list,
                    iterators.len(),
                )?);
            }
            iterators.push(field_terms);
        }
    }
    let mut grouped_field_terms: Vec<usize> = Vec::with_capacity(iterators.len());
    let mut term_states: Vec<FieldMetadataTermState> = Vec::with_capacity(iterators.len());

    while let Some(Reverse(top)) = field_terms_queue.pop() {
        let term = BytesRef::new(top.0.clone());
        group_by_term(&mut field_terms_queue, top, &mut grouped_field_terms);
        // writePostingLines
        term_states.clear();
        for &index in &grouped_field_terms {
            let field_terms = &mut iterators[index];
            let field_metadata = &mut field_metadata_list[field_terms.field_metadata];
            postings
                .postings_writer
                .set_field(field_metadata.field_info_or_npe()?)?;
            let current = field_terms.term.clone().unwrap_or_default();
            let block_term_state = postings.write_posting_line(
                &current,
                field_terms.terms_enum.as_mut(),
                field_metadata,
                norms_producer,
            )?;
            if let Some(block_term_state) = block_term_state {
                field_metadata.set_last_term(term.clone());
                term_states.push(FieldMetadataTermState::new(
                    field_terms.field_metadata,
                    block_term_state,
                ));
            }
        }
        block_writer.add_line(
            term,
            &term_states,
            dictionary_builder,
            &mut field_metadata_list,
        )?;
        // nextTermForIterators
        for &index in &grouped_field_terms {
            if iterators[index].next_term()? {
                // There is a next term for the iterator. Add it to the priority queue.
                field_terms_queue.push(field_key(&iterators[index], &field_metadata_list, index)?);
            }
        }
    }
    Ok(field_metadata_list)
}

/// The queue key of a `FieldTerms`: its term and its field number.
fn field_key(
    field_terms: &FieldTerms,
    field_metadata_list: &[FieldMetadata],
    index: usize,
) -> Result<QueueKey> {
    let number = field_metadata_list[field_terms.field_metadata]
        .field_info_or_npe()?
        .number;
    Ok(Reverse((
        field_terms
            .term
            .as_ref()
            .map(|t| t.slice().to_vec())
            .unwrap_or_default(),
        i64::from(number),
        index,
    )))
}

/// Port of `STUniformSplitTermsWriter.writeFieldMetadataList(Collection)`.
fn write_field_metadata_list(
    parts: &mut WriterParts<'_>,
    field_metadata_list: &[FieldMetadata],
) -> Result<i32> {
    let mut fields_output = ByteBuffersDataOutput::new();
    let mut fields_number = 0;
    for field_metadata in field_metadata_list {
        if field_metadata.get_num_terms() > 0 {
            parts
                .field_metadata_writer
                .write(&mut fields_output, field_metadata)?;
            fields_number += 1;
        }
    }
    parts.write_fields_metadata(fields_number, &fields_output)?;
    Ok(fields_number)
}

/// Port of `STUniformSplitTermsWriter.writeDictionary(int, IndexDictionary.Builder)`.
fn write_dictionary(
    parts: &mut WriterParts<'_>,
    fields_number: i32,
    dictionary_builder: &mut dyn Builder,
) -> Result<()> {
    if fields_number > 0 {
        parts.write_dictionary(dictionary_builder)?;
    }
    write_footer(parts.dictionary_output)
}

/// Port of the package-private class `STUniformSplitTermsWriter.SegmentPostings`.
///
/// Java keeps the segment's merging block reader to call its
/// `postings(String, BlockTermState, PostingsEnum, int)`; this keeps the
/// [`MergingPostings`] that method reads.
#[derive(Clone)]
pub struct SegmentPostings {
    pub(crate) segment_index: usize,
    pub(crate) term_state: BlockTermState,
    pub(crate) merging_postings: MergingPostings,
    pub(crate) doc_map: DocMap,
}

impl SegmentPostings {
    /// Port of `SegmentPostings.getPostings(String, PostingsEnum, int)`.
    ///
    /// # Errors
    ///
    /// The errors of the postings reader.
    pub fn get_postings(
        &self,
        field_name: &str,
        reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Box<dyn PostingsEnum>> {
        self.merging_postings
            .postings(field_name, &self.term_state, reuse, flags)
    }
}

/// Port of the private class `STUniformSplitTermsWriter.SegmentTerms`.
struct SegmentTerms {
    segment_index: usize,
    merging_block_reader: STMergingBlockReader,
    field_term_states_map: HashMap<String, BlockTermState>,
    doc_map: DocMap,
    term: Option<BytesRef>,
}

impl SegmentTerms {
    /// Port of `SegmentTerms.nextTerm()`.
    fn next_term(&mut self) -> Result<bool> {
        self.term = self.merging_block_reader.next_impl()?;
        if self.term.is_none() {
            return Ok(false);
        }
        self.merging_block_reader
            .read_field_term_states_map(&mut self.field_term_states_map)?;
        Ok(true)
    }

    /// The queue key: the term and the segment index.
    fn key(&self, index: usize) -> QueueKey {
        Reverse((
            self.term
                .as_ref()
                .map(|t| t.slice().to_vec())
                .unwrap_or_default(),
            self.segment_index as i64,
            index,
        ))
    }
}

/// Port of the private class `STUniformSplitTermsWriter.MergingFieldTerms`.
struct MergingFieldTerms {
    field_metadata: usize,
    terms_enum: STMergingTermsEnum,
}

/// Port of `STUniformSplitTermsWriter.mergeSegments(MergeState,
/// NormsProducer, List, STBlockWriter, IndexDictionary.Builder)`.
fn merge_segments(
    merge_state: &MergeState,
    norms_producer: Option<&dyn NormsProducer>,
    mut segment_terms_list: Vec<SegmentTerms>,
    block_writer: &mut STBlockWriter<'_>,
    dictionary_builder: &mut dyn Builder,
    postings: &mut PostingParts<'_>,
) -> Result<Vec<FieldMetadata>> {
    let max_doc = match &merge_state.segment_info {
        Some(segment_info) => segment_info.max_doc()?,
        None => {
            return Err(npe(
                "Cannot invoke \"org.apache.lucene.index.SegmentInfo.maxDoc()\" because \
                 \"mergeState.segmentInfo\" is null",
            ))
        }
    };
    let mut field_metadata_list = create_field_metadata_list(
        merge_state.merge_field_infos.iter().map(Some),
        max_doc,
        postings,
    )?;
    // createMergingFieldTermsMap
    let num_segments = merge_state.fields_producers.len();
    let mut field_terms_map: HashMap<String, MergingFieldTerms> =
        HashMap::with_capacity(field_metadata_list.len());
    for (index, field_metadata) in field_metadata_list.iter().enumerate() {
        let name = field_metadata.field_name()?;
        field_terms_map.insert(
            name.to_string(),
            MergingFieldTerms {
                field_metadata: index,
                terms_enum: STMergingTermsEnum::new(name, num_segments),
            },
        );
    }
    // createSegmentTermsQueue
    let mut segment_terms_queue: BinaryHeap<QueueKey> = BinaryHeap::new();
    for (index, segment_terms) in segment_terms_list.iter_mut().enumerate() {
        if segment_terms.next_term()? {
            // There is at least one term in the segment
            segment_terms_queue.push(segment_terms.key(index));
        }
    }
    let mut grouped_segment_terms: Vec<usize> = Vec::with_capacity(segment_terms_list.len());
    let mut field_postings_map: HashMap<String, Vec<SegmentPostings>> = HashMap::new();
    let mut grouped_field_terms: Vec<String> = Vec::new();
    let mut term_states: Vec<FieldMetadataTermState> = Vec::new();

    while let Some(Reverse(top)) = segment_terms_queue.pop() {
        let term = BytesRef::new(top.0.clone());
        group_by_term(&mut segment_terms_queue, top, &mut grouped_segment_terms);
        // combineSegmentsFields
        field_postings_map.clear();
        for &index in &grouped_segment_terms {
            let segment_terms = &segment_terms_list[index];
            for (field, term_state) in &segment_terms.field_term_states_map {
                field_postings_map
                    .entry(field.clone())
                    .or_insert_with(|| Vec::with_capacity(grouped_segment_terms.len()))
                    .push(SegmentPostings {
                        segment_index: segment_terms.segment_index,
                        term_state: *term_state,
                        merging_postings: segment_terms.merging_block_reader.postings.clone(),
                        doc_map: segment_terms.doc_map.clone(),
                    });
            }
        }
        // combinePostingsPerField
        grouped_field_terms.clear();
        for (field, segment_postings_list) in field_postings_map.drain() {
            // The field defined in fieldPostingsMap comes from the FieldInfos of the
            // SegmentReadState. The fieldTermsMap contains entries for fields coming from the
            // SegmentMergeSate. So it is possible that the field is not present in fieldTermsMap
            // because it is removed.
            if let Some(field_terms) = field_terms_map.get_mut(&field) {
                field_terms
                    .terms_enum
                    .reset(term.clone(), segment_postings_list);
                grouped_field_terms.push(field);
            }
        }
        // Keep the fields ordered by their number in the target merge segment.
        let number_of = |name: &String| -> i32 {
            field_terms_map
                .get(name)
                .and_then(|ft| field_metadata_list[ft.field_metadata].get_field_info())
                .map_or(0, |fi| fi.number)
        };
        grouped_field_terms.sort_by_key(number_of);
        // writePostingLines
        term_states.clear();
        for field in &grouped_field_terms {
            let Some(field_terms) = field_terms_map.get_mut(field) else {
                continue;
            };
            let field_metadata = &mut field_metadata_list[field_terms.field_metadata];
            postings
                .postings_writer
                .set_field(field_metadata.field_info_or_npe()?)?;
            let block_term_state = postings.write_posting_line(
                &term,
                &mut field_terms.terms_enum,
                field_metadata,
                norms_producer,
            )?;
            if let Some(block_term_state) = block_term_state {
                field_metadata.set_last_term(term.clone());
                term_states.push(FieldMetadataTermState::new(
                    field_terms.field_metadata,
                    block_term_state,
                ));
            }
        }
        block_writer.add_line(
            term,
            &term_states,
            dictionary_builder,
            &mut field_metadata_list,
        )?;
        // nextTermForIterators
        for &index in &grouped_segment_terms {
            if segment_terms_list[index].next_term()? {
                // There is a next term for the iterator. Add it to the priority queue.
                segment_terms_queue.push(segment_terms_list[index].key(index));
            }
        }
    }
    Ok(field_metadata_list)
}

impl FieldsConsumer for STUniformSplitTermsWriter {
    /// Port of `STUniformSplitTermsWriter.write(Fields, NormsProducer)`.
    fn write(
        &mut self,
        fields: &dyn Fields,
        norms_producer: Option<&dyn NormsProducer>,
    ) -> Result<()> {
        self.write_segment(|block_writer, dictionary_builder, postings, field_infos| {
            write_single_segment(
                fields,
                norms_producer,
                block_writer,
                dictionary_builder,
                postings,
                field_infos,
            )
        })
    }

    /// Port of `STUniformSplitTermsWriter.merge(MergeState, NormsProducer)`.
    fn merge(
        &mut self,
        merge_state: &MergeState,
        norms_producer: Option<&dyn NormsProducer>,
    ) -> Result<()> {
        if merge_state.needs_index_sort {
            // This custom merging does not support sorted index.
            // Fall back to the default merge, which is inefficient for this postings format.
            return default_merge(self, merge_state, norms_producer);
        }
        let mut segment_terms_list: Vec<SegmentTerms> =
            Vec::with_capacity(merge_state.fields_producers.len());
        for (segment_index, fields_producer) in merge_state.fields_producers.iter().enumerate() {
            // Iterate the FieldInfo provided by mergeState.fieldInfos because they may be
            // filtered by PerFieldMergeState.
            for field_info in merge_state.field_infos[segment_index].iter() {
                // Iterate all fields only the get the *first* Terms instanceof STUniformSplitTerms.
                // See the break below.
                let Some(fields_producer) = fields_producer else {
                    return Err(npe(
                        "Cannot invoke \"org.apache.lucene.codecs.FieldsProducer.terms(String)\" \
                         because \"fieldsProducer\" is null",
                    ));
                };
                let Some(terms) = fields_producer.terms(&field_info.name)? else {
                    continue;
                };
                let Some(shared_terms) = terms
                    .as_any()
                    .and_then(|any| any.downcast_ref::<STUniformSplitTerms>())
                else {
                    // Terms is not directly an instance of STUniformSplitTerms, it is
                    // wrapped/filtered. Fall back to the default merge, which is inefficient for
                    // this postings format.
                    return default_merge(self, merge_state, norms_producer);
                };
                segment_terms_list.push(SegmentTerms {
                    segment_index,
                    merging_block_reader: shared_terms.create_merging_block_reader()?,
                    field_term_states_map: HashMap::new(),
                    doc_map: merge_state.doc_maps[segment_index].clone(),
                    term: None,
                });
                // We have the STUniformSplitTerms for the segment. Break the field
                // loop to iterate the next segment.
                break;
            }
        }
        self.write_segment(|block_writer, dictionary_builder, postings, _field_infos| {
            merge_segments(
                merge_state,
                norms_producer,
                segment_terms_list,
                block_writer,
                dictionary_builder,
                postings,
            )
        })
    }

    fn close(&mut self) -> Result<()> {
        self.base.close()
    }
}
