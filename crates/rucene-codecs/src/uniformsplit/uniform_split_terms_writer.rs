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

//! Port of `org.apache.lucene.codecs.uniformsplit.UniformSplitTermsWriter`.

use std::sync::Arc;

use rucene::codecs::codec_util::{write_footer, write_index_header};
use rucene::codecs::{
    BlockTermState, FieldsConsumer, NormsProducer, PostingsWriterBase, SegmentWriteState,
};
use rucene::error::{LuceneError, Result};
use rucene::index::index_file_names::segment_file_name;
use rucene::index::{FieldInfo, FieldInfos, Fields, TermsEnum};
use rucene::store::{ByteBuffersDataOutput, DataOutput, IndexOutput};
use rucene::util::{BytesRef, IOUtils};

use super::block_encoder::BlockEncoder;
use super::block_writer::{BlockWriter, FieldContext};
use super::field_metadata::{self, FieldMetadata};
use super::fst_dictionary;
use super::index_dictionary::Builder;
use super::uniform_split_postings_format::{
    NAME, TERMS_BLOCKS_EXTENSION, TERMS_DICTIONARY_EXTENSION, VERSION_CURRENT,
};

/// Default value for the target block size (number of terms per block).
///
/// Port of `UniformSplitTermsWriter.DEFAULT_TARGET_NUM_BLOCK_LINES`.
pub const DEFAULT_TARGET_NUM_BLOCK_LINES: i32 = 32;

/// Default value for the maximum allowed delta variation of the block size
/// (delta of the number of terms per block). The block size will be [target
/// block size]+-[allowed delta].
///
/// Port of `UniformSplitTermsWriter.DEFAULT_DELTA_NUM_LINES`:
/// `(int) (DEFAULT_TARGET_NUM_BLOCK_LINES * 0.1)`.
pub const DEFAULT_DELTA_NUM_LINES: i32 = (DEFAULT_TARGET_NUM_BLOCK_LINES as f64 * 0.1) as i32;

/// Upper limit of the block size (maximum number of terms per block).
///
/// Port of `UniformSplitTermsWriter.MAX_NUM_BLOCK_LINES`.
pub const MAX_NUM_BLOCK_LINES: i32 = 1_000;

/// The parameters of the protected `UniformSplitTermsWriter` constructor
/// beyond the postings writer, the write state, the block settings and the
/// block encoder, which its subclass passes.
#[derive(Debug, Clone, Copy)]
pub struct WriterParameters {
    /// `codecName`.
    pub codec_name: &'static str,
    /// `versionCurrent`.
    pub version_current: i32,
    /// `termsBlocksExtension`.
    pub terms_blocks_extension: &'static str,
    /// `dictionaryExtension`.
    pub dictionary_extension: &'static str,
}

impl WriterParameters {
    /// The parameters of `UniformSplitPostingsFormat`.
    pub const UNIFORM_SPLIT: WriterParameters = WriterParameters {
        codec_name: NAME,
        version_current: VERSION_CURRENT,
        terms_blocks_extension: TERMS_BLOCKS_EXTENSION,
        dictionary_extension: TERMS_DICTIONARY_EXTENSION,
    };
}

/// A block-based terms index and dictionary that assigns terms to nearly
/// uniform length blocks. This technique is called Uniform Split.
///
/// The block construction is driven by two parameters, `targetNumBlockLines`
/// and `deltaNumLines`. Each block size (number of terms) is
/// `targetNumBlockLines`+-`deltaNumLines`. The algorithm computes the minimal
/// distinguishing prefix (MDP) between each term and its previous term
/// (alphabetically ordered). Then it selects in the neighborhood of the
/// `targetNumBlockLines`, and within the `deltaNumLines`, the term with the
/// minimal MDP. This term becomes the first term of the next block and its
/// MDP is the block key. This block key is added to the terms dictionary
/// trie.
///
/// We call dictionary the trie structure in memory, and block file the disk
/// file containing the block lines, with one term and its corresponding term
/// state details per line.
///
/// When seeking a term, the dictionary seeks the floor leaf of the trie for
/// the searched term and jumps to the corresponding file pointer in the block
/// file. There, the block terms are scanned for the exact searched term.
///
/// The terms inside a block do not need to share a prefix. Only the block key
/// is used to find the block from the dictionary trie. And the block key is
/// selected because it is the locally smallest MDP. This makes the dictionary
/// trie very compact.
///
/// An interesting property of the Uniform Split technique is the very linear
/// balance between memory usage and lookup performance. By decreasing the
/// target block size, the block scan becomes faster, and since there are more
/// blocks, the dictionary trie memory usage increases. Additionally, small
/// blocks are faster to read from disk. A good sweet spot for the target
/// block size is 32 with delta of 3 (10%) (default values). This can be tuned
/// in the constructor.
///
/// There are additional optimizations:
///
/// * Each block has a header that allows the lookup to jump directly to the
///   middle term with a fast comparison. This reduces the linear scan by 2
///   for a small disk size increase.
/// * Each block term is incrementally encoded according to its previous term.
///   This both reduces the disk size and speeds up the block scan.
/// * All term line details (the terms states) are written after all terms.
///   This allows faster term scan without needing to decode the term states.
/// * All file pointers are base-encoded. Their value is relative to the block
///   base file pointer (not to the previous file pointer), this allows to
///   read the term state of any term independently.
///
/// Blocks can be compressed or encrypted with an optional [`BlockEncoder`]
/// provided in the constructor.
///
/// The block file (`.ustb`) contains all the term blocks for each field
/// sequentially. It also contains the fields metadata at the end of the file.
///
/// The dictionary file (`.ustd`) contains the trie (`FST` bytes) for each
/// field sequentially.
///
/// Port of `org.apache.lucene.codecs.uniformsplit.UniformSplitTermsWriter`
/// (`@lucene.experimental`).
pub struct UniformSplitTermsWriter {
    pub(crate) field_infos: FieldInfos,
    pub(crate) postings_writer: Box<dyn PostingsWriterBase>,
    pub(crate) max_doc: i32,

    pub(crate) target_num_block_lines: i32,
    pub(crate) delta_num_lines: i32,

    pub(crate) block_encoder: Option<Arc<dyn BlockEncoder>>,
    pub(crate) field_metadata_writer: field_metadata::Serializer,
    pub(crate) block_output: Option<Box<dyn IndexOutput>>,
    pub(crate) dictionary_output: Option<Box<dyn IndexOutput>>,

    pub(crate) overrides: Option<Arc<dyn TermsWriterOverrides>>,
}

/// The protected methods of [`UniformSplitTermsWriter`] (and of
/// `STUniformSplitTermsWriter`) that a Java subclass overrides around a call
/// to `super`, as `lucene-test-framework`'s `UniformSplitRot13PostingsFormat`
/// and `STUniformSplitRot13PostingsFormat` do (their anonymous subclasses
/// record which parts of the segment went through the block encoder).
///
/// Each method receives `super_`, the base implementation; the default runs
/// it and nothing else, which is the writer without a subclass.
///
/// **Divergence (declared).** Java overrides the methods in an anonymous
/// subclass of the writer; Rust has no subclassing, so the writer holds the
/// overrides as an optional object ([`UniformSplitTermsWriter::with_overrides`])
/// and dispatches the two methods through it. The bytes written and the
/// order of the calls are Java's.
pub trait TermsWriterOverrides: Send + Sync {
    /// `protected void writeDictionary(IndexDictionary.Builder)`
    /// (`UniformSplitTermsWriter.java:369-371`).
    ///
    /// # Errors
    ///
    /// The errors of `super_` and of the override.
    fn write_dictionary(&self, super_: &mut dyn FnMut() -> Result<()>) -> Result<()> {
        super_()
    }

    /// `protected void writeEncodedFieldsMetadata(ByteBuffersDataOutput)`
    /// (`UniformSplitTermsWriter.java:303-311`).
    ///
    /// # Errors
    ///
    /// The errors of `super_` and of the override.
    fn write_encoded_fields_metadata(&self, super_: &mut dyn FnMut() -> Result<()>) -> Result<()> {
        super_()
    }
}

impl UniformSplitTermsWriter {
    /// The writer with its protected methods overridden by `overrides` (a
    /// Java subclass's overrides, see [`TermsWriterOverrides`]).
    #[must_use]
    pub fn with_overrides(mut self, overrides: Arc<dyn TermsWriterOverrides>) -> Self {
        self.overrides = Some(overrides);
        self
    }

    /// Port of `UniformSplitTermsWriter(PostingsWriterBase,
    /// SegmentWriteState, BlockEncoder)`.
    ///
    /// # Errors
    ///
    /// See `with_settings`.
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

    /// Port of `UniformSplitTermsWriter(PostingsWriterBase,
    /// SegmentWriteState, int, int, BlockEncoder)`.
    ///
    /// `target_num_block_lines` is the target number of lines per block. Must
    /// be strictly greater than 0. The parameters can be pre-validated with
    /// [`validate_settings`]. There is one term per block line, with its
    /// corresponding details (`TermState`). `delta_num_lines` is the maximum
    /// allowed delta variation of the number of lines per block. Must be
    /// greater than or equal to 0 and strictly less than
    /// `target_num_block_lines`. The block size will be
    /// `target_num_block_lines`+-`delta_num_lines`. The block size must
    /// always be less than or equal to [`MAX_NUM_BLOCK_LINES`].
    /// `block_encoder` is optional; it can be used for compression or
    /// encryption.
    ///
    /// The postings writer moves into the constructor, so when it fails it
    /// also closes it, which Java's caller (`UniformSplitPostingsFormat`)
    /// does with `IOUtils.closeWhileHandlingException(postingsWriter)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for invalid settings; the I/O errors of
    /// creating the files, writing their headers and the postings writer's
    /// `init`.
    pub fn with_settings(
        postings_writer: Box<dyn PostingsWriterBase>,
        state: &SegmentWriteState<'_>,
        target_num_block_lines: i32,
        delta_num_lines: i32,
        block_encoder: Option<Arc<dyn BlockEncoder>>,
    ) -> Result<Self> {
        Self::with_parameters(
            postings_writer,
            state,
            target_num_block_lines,
            delta_num_lines,
            block_encoder,
            WriterParameters::UNIFORM_SPLIT,
        )
    }

    /// Port of the protected `UniformSplitTermsWriter(PostingsWriterBase,
    /// SegmentWriteState, int, int, BlockEncoder, FieldMetadata.Serializer,
    /// String, int, String, String)`; the field metadata writer is the
    /// stateless `FieldMetadata.Serializer.INSTANCE`, the only one Lucene
    /// passes.
    ///
    /// # Errors
    ///
    /// See `with_settings`.
    pub fn with_parameters(
        mut postings_writer: Box<dyn PostingsWriterBase>,
        state: &SegmentWriteState<'_>,
        target_num_block_lines: i32,
        delta_num_lines: i32,
        block_encoder: Option<Arc<dyn BlockEncoder>>,
        parameters: WriterParameters,
    ) -> Result<Self> {
        let mut block_output: Option<Box<dyn IndexOutput>> = None;
        let mut dictionary_output: Option<Box<dyn IndexOutput>> = None;
        let opened = (|| -> Result<i32> {
            validate_settings(target_num_block_lines, delta_num_lines)?;
            let max_doc = state.segment_info.max_doc()?;

            let terms_name = segment_file_name(
                &state.segment_info.name,
                &state.segment_suffix,
                parameters.terms_blocks_extension,
            );
            let out =
                block_output.insert(state.directory.create_output(&terms_name, state.context)?);
            write_index_header(
                out.as_mut(),
                parameters.codec_name,
                parameters.version_current,
                &state.segment_info.id(),
                &state.segment_suffix,
            )?;

            let index_name = segment_file_name(
                &state.segment_info.name,
                &state.segment_suffix,
                parameters.dictionary_extension,
            );
            let dict = dictionary_output
                .insert(state.directory.create_output(&index_name, state.context)?);
            write_index_header(
                dict.as_mut(),
                parameters.codec_name,
                parameters.version_current,
                &state.segment_info.id(),
                &state.segment_suffix,
            )?;

            let out = block_output
                .as_mut()
                .ok_or_else(|| LuceneError::illegal_state("the block output is created above"))?;
            postings_writer.init(out.as_mut(), state)?;
            Ok(max_doc)
        })();
        match opened {
            Ok(max_doc) => Ok(Self {
                field_infos: state.field_infos.clone(),
                postings_writer,
                max_doc,
                target_num_block_lines,
                delta_num_lines,
                block_encoder,
                field_metadata_writer: field_metadata::Serializer::INSTANCE,
                block_output,
                dictionary_output,
                overrides: None,
            }),
            Err(e) => {
                IOUtils::close_while_handling_exception([
                    block_output
                        .as_mut()
                        .and_then(|o| IOUtils::closeable(move || o.close())),
                    dictionary_output
                        .as_mut()
                        .and_then(|o| IOUtils::closeable(move || o.close())),
                ])?;
                IOUtils::close_while_handling_exception([IOUtils::closeable(|| {
                    postings_writer.close()
                })])?;
                Err(e)
            }
        }
    }

    /// The writer's parts its writing methods borrow at once.
    pub(crate) fn parts(&mut self) -> Result<WriterParts<'_>> {
        let (Some(block_output), Some(dictionary_output)) =
            (self.block_output.as_mut(), self.dictionary_output.as_mut())
        else {
            return Err(LuceneError::already_closed(
                "this UniformSplitTermsWriter is closed",
            ));
        };
        Ok(WriterParts {
            field_infos: &self.field_infos,
            postings_writer: self.postings_writer.as_mut(),
            max_doc: self.max_doc,
            block_encoder: self.block_encoder.as_deref(),
            field_metadata_writer: self.field_metadata_writer,
            block_output: block_output.as_mut(),
            dictionary_output: dictionary_output.as_mut(),
            overrides: self.overrides.as_deref(),
        })
    }
}

/// The fields of a [`UniformSplitTermsWriter`], borrowed at once by its
/// writing methods (and by `STUniformSplitTermsWriter`'s), which Java calls
/// on `this`.
pub(crate) struct WriterParts<'w> {
    pub(crate) field_infos: &'w FieldInfos,
    pub(crate) postings_writer: &'w mut dyn PostingsWriterBase,
    pub(crate) max_doc: i32,
    pub(crate) block_encoder: Option<&'w dyn BlockEncoder>,
    pub(crate) field_metadata_writer: field_metadata::Serializer,
    pub(crate) block_output: &'w mut dyn IndexOutput,
    pub(crate) dictionary_output: &'w mut dyn IndexOutput,
    pub(crate) overrides: Option<&'w dyn TermsWriterOverrides>,
}

/// Validates the constructor settings.
///
/// `target_num_block_lines` is the target number of lines per block. Must be
/// strictly greater than 0. `delta_num_lines` is the maximum allowed delta
/// variation of the number of lines per block. Must be greater than or equal
/// to 0 and strictly less than `target_num_block_lines`. Additionally,
/// `target_num_block_lines` + `delta_num_lines` must be less than or equal
/// to [`MAX_NUM_BLOCK_LINES`].
///
/// Port of `UniformSplitTermsWriter.validateSettings(int, int)`.
///
/// # Errors
///
/// `IllegalArgumentException` for invalid settings.
pub fn validate_settings(target_num_block_lines: i32, delta_num_lines: i32) -> Result<()> {
    if target_num_block_lines <= 0 {
        return Err(LuceneError::illegal_argument(format!(
            "Invalid negative or nul targetNumBlockLines={target_num_block_lines}"
        )));
    }
    if delta_num_lines < 0 {
        return Err(LuceneError::illegal_argument(format!(
            "Invalid negative deltaNumLines={delta_num_lines}"
        )));
    }
    if delta_num_lines >= target_num_block_lines {
        return Err(LuceneError::illegal_argument(format!(
            "Invalid too large deltaNumLines={delta_num_lines}, it must be < \
             targetNumBlockLines={target_num_block_lines}"
        )));
    }
    if target_num_block_lines.wrapping_add(delta_num_lines) > MAX_NUM_BLOCK_LINES {
        return Err(LuceneError::illegal_argument(format!(
            "Invalid (targetNumBlockLines + deltaNumLines)={}, it must be <= \
             MAX_NUM_BLOCK_LINES={MAX_NUM_BLOCK_LINES}",
            target_num_block_lines.wrapping_add(delta_num_lines)
        )));
    }
    Ok(())
}

impl WriterParts<'_> {
    /// Port of `UniformSplitTermsWriter.writeFieldsMetadata(int,
    /// ByteBuffersDataOutput)`.
    pub(crate) fn write_fields_metadata(
        &mut self,
        fields_number: i32,
        fields_output: &ByteBuffersDataOutput,
    ) -> Result<()> {
        let fields_start_position = self.block_output.file_pointer();
        self.block_output.write_v_int(fields_number)?;
        match self.block_encoder {
            None => self.write_unencoded_fields_metadata(fields_output)?,
            Some(_) => self.write_encoded_fields_metadata(fields_output)?,
        }
        // Must be a fixed length. Read by UniformSplitTermsReader when seeking fields metadata.
        self.block_output.write_long(fields_start_position)?;
        write_footer(self.block_output)
    }

    /// Port of `UniformSplitTermsWriter.writeUnencodedFieldsMetadata(ByteBuffersDataOutput)`.
    pub(crate) fn write_unencoded_fields_metadata(
        &mut self,
        fields_output: &ByteBuffersDataOutput,
    ) -> Result<()> {
        fields_output.copy_to(self.block_output)
    }

    /// Port of `UniformSplitTermsWriter.writeEncodedFieldsMetadata(ByteBuffersDataOutput)`,
    /// dispatched through the writer's [`TermsWriterOverrides`].
    pub(crate) fn write_encoded_fields_metadata(
        &mut self,
        fields_output: &ByteBuffersDataOutput,
    ) -> Result<()> {
        match self.overrides {
            Some(overrides) => overrides.write_encoded_fields_metadata(&mut || {
                self.base_write_encoded_fields_metadata(fields_output)
            }),
            None => self.base_write_encoded_fields_metadata(fields_output),
        }
    }

    /// The base `writeEncodedFieldsMetadata(ByteBuffersDataOutput)`, an
    /// override's `super`.
    fn base_write_encoded_fields_metadata(
        &mut self,
        fields_output: &ByteBuffersDataOutput,
    ) -> Result<()> {
        let Some(block_encoder) = self.block_encoder else {
            return Err(super::npe(
                "Cannot invoke \"org.apache.lucene.codecs.uniformsplit.BlockEncoder.encode(\
                 org.apache.lucene.store.DataInput, long)\" because \"this.blockEncoder\" is null",
            ));
        };
        let encoded_bytes = block_encoder.encode(
            &mut fields_output.to_data_input()?,
            fields_output.size() as i64,
        )?;
        self.block_output.write_v_long(encoded_bytes.size())?;
        encoded_bytes.write_to(self.block_output)
    }

    /// Writes the dictionary index (FST) to disk.
    ///
    /// Port of `UniformSplitTermsWriter.writeDictionary(IndexDictionary.Builder)`,
    /// dispatched through the writer's [`TermsWriterOverrides`].
    pub(crate) fn write_dictionary(&mut self, dictionary_builder: &mut dyn Builder) -> Result<()> {
        write_dictionary(
            self.overrides,
            &mut *self.dictionary_output,
            self.block_encoder,
            dictionary_builder,
        )
    }

    /// Port of `UniformSplitTermsWriter.writeFieldTerms(BlockWriter,
    /// DataOutput, TermsEnum, FieldInfo, NormsProducer)`: returns 1 if the
    /// field was written, 0 otherwise. The block writer is built on this
    /// writer's block output by the caller, so it is passed with the parts it
    /// does not borrow.
    fn write_field_terms(
        block_writer: &mut BlockWriter<'_>,
        postings: &mut PostingParts<'_>,
        fields_output: &mut dyn DataOutput,
        terms_enum: &mut dyn TermsEnum,
        field_info: &FieldInfo,
        norms_producer: Option<&dyn NormsProducer>,
    ) -> Result<i32> {
        let mut field_metadata = FieldMetadata::new(field_info.clone(), postings.max_doc);
        field_metadata.set_dictionary_start_fp(postings.dictionary_output.file_pointer());

        postings.postings_writer.set_field(field_info)?;
        let mut dictionary_builder = fst_dictionary::Builder::new()?;
        let mut last_term: Option<BytesRef> = None;
        while let Some(term) = terms_enum.next()? {
            let block_term_state = postings.write_posting_line(
                &term,
                terms_enum,
                &mut field_metadata,
                norms_producer,
            )?;
            if let Some(block_term_state) = block_term_state {
                let copy = BytesRef::deep_copy_of(&term);
                last_term = Some(copy.clone());
                block_writer.add_line(
                    copy,
                    block_term_state,
                    &mut dictionary_builder,
                    &mut FieldContext(&mut field_metadata),
                )?;
            }
        }

        // Flush remaining terms.
        block_writer.finish_last_block(
            &mut dictionary_builder,
            &mut FieldContext(&mut field_metadata),
        )?;

        if field_metadata.get_num_terms() > 0 {
            if let Some(last_term) = last_term {
                field_metadata.set_last_term(last_term);
            }
            postings
                .field_metadata_writer
                .write(fields_output, &field_metadata)?;
            write_dictionary(
                postings.overrides,
                &mut *postings.dictionary_output,
                postings.block_encoder,
                &mut dictionary_builder,
            )?;
            return Ok(1);
        }
        Ok(0)
    }
}

/// The parts of the writer `writeFieldTerms` uses next to the block writer,
/// which holds the block output.
pub(crate) struct PostingParts<'w> {
    pub(crate) postings_writer: &'w mut dyn PostingsWriterBase,
    pub(crate) max_doc: i32,
    pub(crate) block_encoder: Option<&'w dyn BlockEncoder>,
    pub(crate) field_metadata_writer: field_metadata::Serializer,
    pub(crate) dictionary_output: &'w mut dyn IndexOutput,
    pub(crate) overrides: Option<&'w dyn TermsWriterOverrides>,
}

/// `writeDictionary(IndexDictionary.Builder)` (`UniformSplitTermsWriter.java:369-371`):
/// `dictionaryBuilder.build().write(dictionaryOutput, blockEncoder)`, through
/// `overrides` when the writer has them.
fn write_dictionary(
    overrides: Option<&dyn TermsWriterOverrides>,
    dictionary_output: &mut dyn IndexOutput,
    block_encoder: Option<&dyn BlockEncoder>,
    dictionary_builder: &mut dyn Builder,
) -> Result<()> {
    let mut base = || {
        dictionary_builder
            .build()?
            .write(&mut *dictionary_output, block_encoder)
    };
    match overrides {
        Some(overrides) => overrides.write_dictionary(&mut base),
        None => base(),
    }
}

impl PostingParts<'_> {
    /// Writes the posting values for the current term in the given
    /// `TermsEnum` and updates the [`FieldMetadata`] stats.
    ///
    /// Returns the written [`BlockTermState`], or `None` if none.
    ///
    /// Port of `UniformSplitTermsWriter.writePostingLine(TermsEnum,
    /// FieldMetadata, NormsProducer)`; `term` is `termsEnum.term()`.
    pub(crate) fn write_posting_line(
        &mut self,
        term: &BytesRef,
        terms_enum: &mut dyn TermsEnum,
        field_metadata: &mut FieldMetadata,
        norms_producer: Option<&dyn NormsProducer>,
    ) -> Result<Option<BlockTermState>> {
        let Some(docs_seen) = field_metadata.get_docs_seen() else {
            return Err(super::npe(
                "Cannot invoke \"org.apache.lucene.util.FixedBitSet.set(int)\" because \
                 \"docsSeen\" is null",
            ));
        };
        let state = self
            .postings_writer
            .write_term(term, terms_enum, docs_seen, norms_producer)?;
        let Some(state) = state else {
            // No doc for this term.
            return Ok(None);
        };
        field_metadata.update_stats(&state);
        Ok(Some(state))
    }
}

impl<'w> WriterParts<'w> {
    /// Splits the parts into the block writer's output and the rest.
    pub(crate) fn split(self) -> (&'w mut dyn IndexOutput, PostingParts<'w>, &'w FieldInfos) {
        (
            self.block_output,
            PostingParts {
                postings_writer: self.postings_writer,
                max_doc: self.max_doc,
                block_encoder: self.block_encoder,
                field_metadata_writer: self.field_metadata_writer,
                dictionary_output: self.dictionary_output,
                overrides: self.overrides,
            },
            self.field_infos,
        )
    }
}

impl FieldsConsumer for UniformSplitTermsWriter {
    /// Port of `UniformSplitTermsWriter.write(Fields, NormsProducer)`.
    fn write(
        &mut self,
        fields: &dyn Fields,
        norms_producer: Option<&dyn NormsProducer>,
    ) -> Result<()> {
        let target_num_block_lines = self.target_num_block_lines;
        let delta_num_lines = self.delta_num_lines;
        let parts = self.parts()?;
        let block_encoder = parts.block_encoder;
        let (block_output, mut postings, field_infos) = parts.split();
        let mut fields_output = ByteBuffersDataOutput::new();
        let mut fields_number = 0;
        {
            let mut block_writer = BlockWriter::new(
                &mut *block_output,
                target_num_block_lines,
                delta_num_lines,
                block_encoder,
            );
            for field in fields.iterator() {
                let Some(terms) = fields.terms(&field)? else {
                    continue;
                };
                let mut terms_enum = terms.iterator()?;
                let Some(field_info) = field_infos.field_info(&field) else {
                    return Err(super::npe(
                        "Cannot invoke \"org.apache.lucene.index.FieldInfo.getIndexOptions()\" \
                         because \"fieldInfo\" is null",
                    ));
                };
                fields_number += WriterParts::write_field_terms(
                    &mut block_writer,
                    &mut postings,
                    &mut fields_output,
                    terms_enum.as_mut(),
                    field_info,
                    norms_producer,
                )?;
            }
        }
        let mut parts = WriterParts {
            field_infos,
            postings_writer: postings.postings_writer,
            max_doc: postings.max_doc,
            block_encoder,
            field_metadata_writer: postings.field_metadata_writer,
            block_output,
            dictionary_output: postings.dictionary_output,
            overrides: postings.overrides,
        };
        parts.write_fields_metadata(fields_number, &fields_output)?;
        write_footer(parts.dictionary_output)
    }

    /// Port of `UniformSplitTermsWriter.close()`.
    fn close(&mut self) -> Result<()> {
        if self.block_output.is_none() && self.dictionary_output.is_none() {
            // Already closed: the postings writer was closed with the outputs.
            return Ok(());
        }
        let mut block_output = self.block_output.take();
        let mut dictionary_output = self.dictionary_output.take();
        let postings_writer = &mut self.postings_writer;
        IOUtils::close([
            block_output
                .as_mut()
                .and_then(|o| IOUtils::closeable(move || o.close())),
            dictionary_output
                .as_mut()
                .and_then(|o| IOUtils::closeable(move || o.close())),
            IOUtils::closeable(move || postings_writer.close()),
        ])
    }
}
