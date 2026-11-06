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

//! Port of `org.apache.lucene.codecs.uniformsplit.UniformSplitTermsReader`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::codecs::codec_util::{
    check_index_header, checksum_entire_file, footer_length, retrieve_checksum,
};
use rucene::codecs::{FieldsProducer, PostingsReaderBase, SegmentReadState};
use rucene::error::{LuceneError, Result};
use rucene::index::index_file_names::segment_file_name;
use rucene::index::{FieldInfos, Fields, Terms};
use rucene::store::{ByteArrayDataInput, DataInput, IndexInput};
use rucene::util::java_collections::java_compare_to;
use rucene::util::IOUtils;

use super::block_decoder::BlockDecoder;
use super::field_metadata::{self, FieldMetadata};
use super::fst_dictionary;
use super::index_dictionary::BrowserSupplier;
use super::sharedterms::st_uniform_split_terms::STUniformSplitTerms;
use super::sharedterms::st_uniform_split_terms_reader;
use super::uniform_split_postings_format::{
    NAME, TERMS_BLOCKS_EXTENSION, TERMS_DICTIONARY_EXTENSION, VERSION_CURRENT,
    VERSION_ENCODABLE_FIELDS_METADATA, VERSION_START,
};
use super::uniform_split_terms::UniformSplitTerms;

/// The `Terms` of one field of a [`UniformSplitTermsReader`]: a
/// [`UniformSplitTerms`], or the `STUniformSplitTerms` the shared-terms
/// reader's `fillFieldMap` puts in Java's `Map<String, UniformSplitTerms>`.
#[derive(Clone)]
pub enum FieldTerms {
    /// A `UniformSplitTerms`.
    Plain(UniformSplitTerms),
    /// An `STUniformSplitTerms`.
    Shared(STUniformSplitTerms),
}

impl FieldTerms {
    fn to_terms(&self) -> Box<dyn Terms> {
        match self {
            FieldTerms::Plain(t) => Box::new(t.clone()),
            FieldTerms::Shared(t) => Box::new(t.clone()),
        }
    }
}

/// The parameters of the protected `UniformSplitTermsReader` constructor
/// beyond the postings reader, the read state, the block decoder and
/// `dictionaryOnHeap`, which its subclass passes.
///
/// `shared_terms` selects `STUniformSplitTermsReader.fillFieldMap`, the
/// subclass's override.
#[derive(Debug, Clone, Copy)]
pub struct ReaderParameters {
    /// `codecName`.
    pub codec_name: &'static str,
    /// `versionStart`.
    pub version_start: i32,
    /// `versionCurrent`.
    pub version_current: i32,
    /// `termsBlocksExtension`.
    pub terms_blocks_extension: &'static str,
    /// `dictionaryExtension`.
    pub dictionary_extension: &'static str,
    /// Whether the fields share one terms dictionary.
    pub shared_terms: bool,
}

/// A block-based terms index and dictionary based on the Uniform Split
/// technique.
///
/// Port of `org.apache.lucene.codecs.uniformsplit.UniformSplitTermsReader`
/// (`@lucene.experimental`).
///
/// `close()` closes the inputs and the postings reader when this reader is
/// their last holder; one a live `Terms` or enum still shares is released
/// when that holder is dropped (see `FSTTermsReader`).
pub struct UniformSplitTermsReader {
    pub(crate) postings_reader: Option<Arc<dyn PostingsReaderBase>>,
    pub(crate) version: i32,
    pub(crate) block_input: Option<Arc<Box<dyn IndexInput>>>,
    pub(crate) dictionary_input: Option<Arc<Box<dyn IndexInput>>>,

    pub(crate) field_to_terms_map: HashMap<String, FieldTerms>,
    // Keeps the order of the field names; much more efficient than having a TreeMap for the
    // fieldToTermsMap.
    pub(crate) sorted_field_names: Vec<String>,
}

impl UniformSplitTermsReader {
    /// Port of `UniformSplitTermsReader(PostingsReaderBase, SegmentReadState,
    /// BlockDecoder, boolean)`.
    ///
    /// `block_decoder` is optional; it can be used for decompression or
    /// decryption. `dictionary_on_heap` forces loading the terms dictionary
    /// on-heap. By default it is kept off-heap without impact on
    /// performance. If block encoding/decoding is used, then the dictionary
    /// is always loaded on-heap whatever this parameter value is.
    ///
    /// The postings reader moves into the constructor, so when it fails it
    /// also closes it, which Java's caller (`UniformSplitPostingsFormat`)
    /// does with `IOUtils.closeWhileHandlingException(postingsReader)`.
    ///
    /// # Errors
    ///
    /// `CorruptIndexException` for a bad header, checksum or fields
    /// metadata; I/O errors.
    pub fn new(
        postings_reader: Box<dyn PostingsReaderBase>,
        state: &SegmentReadState<'_>,
        block_decoder: Option<Arc<dyn BlockDecoder>>,
        dictionary_on_heap: bool,
    ) -> Result<Self> {
        Self::with_parameters(
            postings_reader,
            state,
            block_decoder,
            dictionary_on_heap,
            ReaderParameters {
                codec_name: NAME,
                version_start: VERSION_START,
                version_current: VERSION_CURRENT,
                terms_blocks_extension: TERMS_BLOCKS_EXTENSION,
                dictionary_extension: TERMS_DICTIONARY_EXTENSION,
                shared_terms: false,
            },
        )
    }

    /// Port of the protected `UniformSplitTermsReader(PostingsReaderBase,
    /// SegmentReadState, BlockDecoder, boolean, FieldMetadata.Serializer,
    /// String, int, int, String, String)`; the field metadata reader is the
    /// stateless `FieldMetadata.Serializer.INSTANCE`, the only one Lucene
    /// passes.
    ///
    /// # Errors
    ///
    /// See `new`.
    pub fn with_parameters(
        mut postings_reader: Box<dyn PostingsReaderBase>,
        state: &SegmentReadState<'_>,
        block_decoder: Option<Arc<dyn BlockDecoder>>,
        dictionary_on_heap: bool,
        parameters: ReaderParameters,
    ) -> Result<Self> {
        let mut block_input: Option<Box<dyn IndexInput>> = None;
        let mut dictionary_input: Option<Box<dyn IndexInput>> = None;
        let opened = Self::open(
            postings_reader.as_mut(),
            state,
            block_decoder.as_deref(),
            &parameters,
            &mut block_input,
            &mut dictionary_input,
        );
        let (version, field_metadata_collection) = match opened {
            Ok(v) => v,
            Err(e) => {
                IOUtils::close_while_handling_exception([
                    block_input
                        .as_mut()
                        .and_then(|i| IOUtils::closeable(move || i.close())),
                    dictionary_input
                        .as_mut()
                        .and_then(|i| IOUtils::closeable(move || i.close())),
                ])?;
                IOUtils::close_while_handling_exception([IOUtils::closeable(|| {
                    postings_reader.close()
                })])?;
                return Err(e);
            }
        };
        let (Some(block_input), Some(dictionary_input)) = (block_input, dictionary_input) else {
            return Err(LuceneError::illegal_state("the inputs are opened above"));
        };
        let block_input = Arc::new(block_input);
        let dictionary_input = Arc::new(dictionary_input);
        let postings_reader: Arc<dyn PostingsReaderBase> = Arc::from(postings_reader);

        let mut reader = Self {
            postings_reader: Some(Arc::clone(&postings_reader)),
            version,
            block_input: Some(Arc::clone(&block_input)),
            dictionary_input: Some(Arc::clone(&dictionary_input)),
            field_to_terms_map: HashMap::new(),
            sorted_field_names: Vec::new(),
        };
        let filled = if parameters.shared_terms {
            st_uniform_split_terms_reader::fill_field_map(
                &mut reader,
                &postings_reader,
                state,
                block_decoder.as_ref(),
                dictionary_on_heap,
                &dictionary_input,
                &block_input,
                field_metadata_collection,
                state.field_infos,
            )
        } else {
            reader.fill_field_map(
                &postings_reader,
                block_decoder.as_ref(),
                dictionary_on_heap,
                &dictionary_input,
                &block_input,
                field_metadata_collection,
            )
        };
        if let Err(e) = filled {
            drop(postings_reader);
            drop(block_input);
            drop(dictionary_input);
            let _ = reader.close_inputs_while_handling_exception();
            return Err(e);
        }

        let mut field_names: Vec<String> = reader.field_to_terms_map.keys().cloned().collect();
        field_names.sort_by(|a, b| java_compare_to(a, b));
        reader.sorted_field_names = field_names;
        Ok(reader)
    }

    /// The `try` block of the constructor up to `readFieldsMetadata`: opens
    /// the inputs into the given slots, so that the caller closes what was
    /// opened when it fails.
    fn open(
        postings_reader: &mut dyn PostingsReaderBase,
        state: &SegmentReadState<'_>,
        block_decoder: Option<&dyn BlockDecoder>,
        parameters: &ReaderParameters,
        block_input_slot: &mut Option<Box<dyn IndexInput>>,
        dictionary_input_slot: &mut Option<Box<dyn IndexInput>>,
    ) -> Result<(i32, Vec<FieldMetadata>)> {
        let segment_name = &state.segment_info.name;
        let terms_name = segment_file_name(
            segment_name,
            &state.segment_suffix,
            parameters.terms_blocks_extension,
        );
        let block_input =
            block_input_slot.insert(state.directory.open_input(&terms_name, state.context)?);

        let version = check_index_header(
            block_input.as_mut(),
            parameters.codec_name,
            parameters.version_start,
            parameters.version_current,
            &state.segment_info.id(),
            &state.segment_suffix,
        )?;
        let index_name = segment_file_name(
            segment_name,
            &state.segment_suffix,
            parameters.dictionary_extension,
        );
        let dictionary_input =
            dictionary_input_slot.insert(state.directory.open_input(&index_name, state.context)?);

        check_index_header(
            dictionary_input.as_mut(),
            parameters.codec_name,
            version,
            version,
            &state.segment_info.id(),
            &state.segment_suffix,
        )?;
        checksum_entire_file(dictionary_input.as_mut())?;

        let block_input = block_input_slot
            .as_mut()
            .ok_or_else(|| LuceneError::illegal_state("the block input is opened above"))?;
        postings_reader.init(block_input.as_mut(), state)?;
        retrieve_checksum(block_input.as_mut())?;

        seek_fields_metadata(block_input.as_mut())?;
        let field_metadata_collection = read_fields_metadata(
            version,
            block_input.as_mut(),
            block_decoder,
            state.field_infos,
            &field_metadata::Serializer::INSTANCE,
            state.segment_info.max_doc()?,
        )?;
        Ok((version, field_metadata_collection))
    }

    /// Port of `UniformSplitTermsReader.fillFieldMap(...)`.
    fn fill_field_map(
        &mut self,
        postings_reader: &Arc<dyn PostingsReaderBase>,
        block_decoder: Option<&Arc<dyn BlockDecoder>>,
        dictionary_on_heap: bool,
        dictionary_input: &Arc<Box<dyn IndexInput>>,
        block_input: &Arc<Box<dyn IndexInput>>,
        field_metadata_collection: Vec<FieldMetadata>,
    ) -> Result<()> {
        for field_metadata in field_metadata_collection {
            let dictionary_browser_supplier = create_dictionary_browser_supplier(
                dictionary_input.as_ref().as_ref(),
                &field_metadata,
                block_decoder,
                dictionary_on_heap,
            )?;
            let name = field_metadata.field_name()?.to_string();
            self.field_to_terms_map.insert(
                name,
                FieldTerms::Plain(UniformSplitTerms::new(
                    Arc::clone(block_input),
                    Arc::new(field_metadata),
                    Arc::clone(postings_reader),
                    block_decoder.cloned(),
                    dictionary_browser_supplier,
                )),
            );
        }
        Ok(())
    }

    /// Closes the inputs and the postings reader when this reader holds them
    /// alone, keeping the first error.
    fn close_inputs_while_handling_exception(&mut self) -> Result<()> {
        self.field_to_terms_map.clear();
        let mut block_input = self.block_input.take();
        let mut dictionary_input = self.dictionary_input.take();
        let mut postings_reader = self.postings_reader.take();
        IOUtils::close_while_handling_exception([
            block_input
                .as_mut()
                .and_then(Arc::get_mut)
                .and_then(|i| IOUtils::closeable(move || i.close())),
            dictionary_input
                .as_mut()
                .and_then(Arc::get_mut)
                .and_then(|i| IOUtils::closeable(move || i.close())),
            postings_reader
                .as_mut()
                .and_then(Arc::get_mut)
                .and_then(|r| IOUtils::closeable(move || r.close())),
        ])
    }

    /// The `Terms` of `field`, as the reader keeps them.
    pub fn field_terms(&self, field: &str) -> Option<&FieldTerms> {
        self.field_to_terms_map.get(field)
    }
}

/// Port of `UniformSplitTermsReader.createDictionaryBrowserSupplier(...)`.
///
/// # Errors
///
/// The I/O errors of cloning and seeking the dictionary input.
pub fn create_dictionary_browser_supplier(
    dictionary_input: &dyn IndexInput,
    field_metadata: &FieldMetadata,
    block_decoder: Option<&Arc<dyn BlockDecoder>>,
    dictionary_on_heap: bool,
) -> Result<Arc<dyn BrowserSupplier>> {
    Ok(Arc::new(fst_dictionary::BrowserSupplier::new(
        dictionary_input,
        field_metadata.get_dictionary_start_fp(),
        block_decoder.cloned(),
        dictionary_on_heap,
    )?))
}

/// Port of `UniformSplitTermsReader.readFieldsMetadata(...)`: the input must
/// be positioned to the fields metadata details by calling
/// [`seek_fields_metadata`] before this call; `block_decoder` is optional.
///
/// # Errors
///
/// `CorruptIndexException` for an illegal number of fields or bad metadata;
/// I/O errors.
pub fn read_fields_metadata(
    version: i32,
    index_input: &mut dyn IndexInput,
    block_decoder: Option<&dyn BlockDecoder>,
    field_infos: &FieldInfos,
    field_metadata_reader: &field_metadata::Serializer,
    max_num_docs: i32,
) -> Result<Vec<FieldMetadata>> {
    let num_fields = index_input.read_v_int()?;
    if num_fields < 0 {
        return Err(LuceneError::corrupt_index(
            format!("Illegal number of fields= {num_fields}"),
            index_input.to_java_string(),
        ));
    }
    match block_decoder {
        Some(block_decoder) if version >= VERSION_ENCODABLE_FIELDS_METADATA => {
            read_encoded_fields_metadata(
                num_fields,
                index_input,
                block_decoder,
                field_infos,
                field_metadata_reader,
                max_num_docs,
            )
        }
        _ => read_unencoded_fields_metadata(
            num_fields,
            index_input,
            field_infos,
            field_metadata_reader,
            max_num_docs,
        ),
    }
}

/// Port of `UniformSplitTermsReader.readEncodedFieldsMetadata(...)`.
///
/// # Errors
///
/// `CorruptIndexException` for an illegal encoded length or bad metadata;
/// the errors of the decoder; I/O errors.
pub fn read_encoded_fields_metadata(
    num_fields: i32,
    metadata_input: &mut dyn DataInput,
    block_decoder: &dyn BlockDecoder,
    field_infos: &FieldInfos,
    field_metadata_reader: &field_metadata::Serializer,
    max_num_docs: i32,
) -> Result<Vec<FieldMetadata>> {
    let encoded_length = metadata_input.read_v_long()?;
    if encoded_length < 0 {
        return Err(LuceneError::corrupt_index(
            format!("Illegal encoded length: {encoded_length}"),
            metadata_input.to_java_string(),
        ));
    }
    let decoded_bytes = block_decoder.decode(metadata_input, encoded_length)?;
    let mut decoded_metadata_input =
        ByteArrayDataInput::with_range(&decoded_bytes.bytes, 0, decoded_bytes.length);
    read_unencoded_fields_metadata(
        num_fields,
        &mut decoded_metadata_input,
        field_infos,
        field_metadata_reader,
        max_num_docs,
    )
}

/// Port of `UniformSplitTermsReader.readUnencodedFieldsMetadata(...)`.
///
/// # Errors
///
/// `CorruptIndexException` for bad metadata; I/O errors.
pub fn read_unencoded_fields_metadata(
    num_fields: i32,
    metadata_input: &mut dyn DataInput,
    field_infos: &FieldInfos,
    field_metadata_reader: &field_metadata::Serializer,
    max_num_docs: i32,
) -> Result<Vec<FieldMetadata>> {
    let mut field_metadata_collection = Vec::with_capacity(num_fields.max(0) as usize);
    for _ in 0..num_fields {
        field_metadata_collection.push(field_metadata_reader.read(
            metadata_input,
            field_infos,
            max_num_docs,
        )?);
    }
    Ok(field_metadata_collection)
}

/// Positions the given [`IndexInput`] at the beginning of the fields
/// metadata.
///
/// Port of `UniformSplitTermsReader.seekFieldsMetadata(IndexInput)`.
///
/// # Errors
///
/// I/O errors.
pub fn seek_fields_metadata(index_input: &mut dyn IndexInput) -> Result<()> {
    index_input.seek(index_input.length() - i64::from(footer_length()) - 8)?;
    let fields_start = index_input.read_long()?;
    index_input.seek(fields_start)
}

impl Fields for UniformSplitTermsReader {
    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        Box::new(self.sorted_field_names.iter().cloned())
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        Ok(self.field_to_terms_map.get(field).map(FieldTerms::to_terms))
    }

    fn size(&self) -> i32 {
        self.field_to_terms_map.len() as i32
    }
}

impl Drop for UniformSplitTermsReader {
    /// Runs [`close`](FieldsProducer::close) (`UniformSplitTermsReader.java:266-273`) when this reader, or
    /// one of its merge instances, goes away (rmp #351). Java's
    /// `getMergeInstance()` is `this`, so closing the reader closes the files
    /// a merge instance reads; here a merge instance shares them, the
    /// reader's `close()` only closes what it holds alone, and the last
    /// holder to go away closes the rest. `close()` takes what it closes, so
    /// a reader closed already closes nothing again. A failure is dropped: no
    /// caller is left to receive it.
    fn drop(&mut self) {
        let _ = FieldsProducer::close(self);
    }
}

impl FieldsProducer for UniformSplitTermsReader {
    fn check_integrity(&self) -> Result<()> {
        // term dictionary
        let Some(block_input) = &self.block_input else {
            return Err(super::npe(
                "Cannot invoke \"org.apache.lucene.store.IndexInput.clone()\" because \"in\" is \
                 null",
            ));
        };
        let mut input = block_input.clone_input()?;
        checksum_entire_file(input.as_mut())?;

        // postings
        match &self.postings_reader {
            Some(postings_reader) => postings_reader.check_integrity(),
            None => Err(super::npe(
                "Cannot invoke \"org.apache.lucene.codecs.PostingsReaderBase.checkIntegrity()\"",
            )),
        }
    }

    fn get_merge_instance(&self) -> Result<Box<dyn FieldsProducer>> {
        Ok(Box::new(Self {
            postings_reader: self.postings_reader.clone(),
            version: self.version,
            block_input: self.block_input.clone(),
            dictionary_input: self.dictionary_input.clone(),
            field_to_terms_map: self.field_to_terms_map.clone(),
            sorted_field_names: self.sorted_field_names.clone(),
        }))
    }

    fn close(&mut self) -> Result<()> {
        // try { IOUtils.close(blockInput, dictionaryInput, postingsReader) } finally {
        // Clear so refs to terms index is GCable even if app hangs onto us.
        // fieldToTermsMap.clear() }
        self.field_to_terms_map.clear();
        let mut block_input = self.block_input.take();
        let mut dictionary_input = self.dictionary_input.take();
        let mut postings_reader = self.postings_reader.take();
        IOUtils::close([
            block_input
                .as_mut()
                .and_then(Arc::get_mut)
                .and_then(|i| IOUtils::closeable(move || i.close())),
            dictionary_input
                .as_mut()
                .and_then(Arc::get_mut)
                .and_then(|i| IOUtils::closeable(move || i.close())),
            postings_reader
                .as_mut()
                .and_then(Arc::get_mut)
                .and_then(|r| IOUtils::closeable(move || r.close())),
        ])
    }
}
