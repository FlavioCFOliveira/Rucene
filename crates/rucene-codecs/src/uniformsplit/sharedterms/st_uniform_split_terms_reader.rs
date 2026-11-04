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

//! Port of `org.apache.lucene.codecs.uniformsplit.sharedterms.STUniformSplitTermsReader`.
//!
//! A block-based terms index and dictionary based on the Uniform Split
//! technique, and sharing all the fields terms in the same dictionary, with
//! all the fields of a term in the same block line (`@lucene.experimental`).
//!
//! Java's class extends `UniformSplitTermsReader` and overrides only
//! `fillFieldMap`; the port's reader is a
//! [`UniformSplitTermsReader`] built by [`new`] with that override
//! (`fill_field_map`).

use std::sync::Arc;

use rucene::codecs::{PostingsReaderBase, SegmentReadState};
use rucene::error::Result;
use rucene::index::FieldInfos;
use rucene::store::IndexInput;

use super::st_uniform_split_postings_format::{
    NAME, TERMS_BLOCKS_EXTENSION, TERMS_DICTIONARY_EXTENSION, VERSION_CURRENT,
};
use super::st_uniform_split_terms::STUniformSplitTerms;
use super::union_field_metadata_builder::UnionFieldMetadataBuilder;
use crate::uniformsplit::block_decoder::BlockDecoder;
use crate::uniformsplit::field_metadata::FieldMetadata;
use crate::uniformsplit::uniform_split_postings_format::VERSION_START;
use crate::uniformsplit::uniform_split_terms_reader::{
    create_dictionary_browser_supplier, FieldTerms, ReaderParameters, UniformSplitTermsReader,
};

/// The parameters of `STUniformSplitTermsReader`.
pub const PARAMETERS: ReaderParameters = ReaderParameters {
    codec_name: NAME,
    version_start: VERSION_START,
    version_current: VERSION_CURRENT,
    terms_blocks_extension: TERMS_BLOCKS_EXTENSION,
    dictionary_extension: TERMS_DICTIONARY_EXTENSION,
    shared_terms: true,
};

/// Port of `STUniformSplitTermsReader(PostingsReaderBase, SegmentReadState,
/// BlockDecoder, boolean)`; see `UniformSplitTermsReader::new`.
///
/// # Errors
///
/// See `UniformSplitTermsReader::new`.
pub fn new(
    postings_reader: Box<dyn PostingsReaderBase>,
    state: &SegmentReadState<'_>,
    block_decoder: Option<Arc<dyn BlockDecoder>>,
    dictionary_on_heap: bool,
) -> Result<UniformSplitTermsReader> {
    UniformSplitTermsReader::with_parameters(
        postings_reader,
        state,
        block_decoder,
        dictionary_on_heap,
        PARAMETERS,
    )
}

/// Port of `STUniformSplitTermsReader.fillFieldMap(...)`.
///
/// # Errors
///
/// The I/O errors of creating the dictionary browser supplier.
#[allow(clippy::too_many_arguments)]
pub(crate) fn fill_field_map(
    reader: &mut UniformSplitTermsReader,
    postings_reader: &Arc<dyn PostingsReaderBase>,
    _state: &SegmentReadState<'_>,
    block_decoder: Option<&Arc<dyn BlockDecoder>>,
    dictionary_on_heap: bool,
    dictionary_input: &Arc<Box<dyn IndexInput>>,
    block_input: &Arc<Box<dyn IndexInput>>,
    field_metadata_collection: Vec<FieldMetadata>,
    field_infos: &FieldInfos,
) -> Result<()> {
    if !field_metadata_collection.is_empty() {
        let union_field_metadata =
            Arc::new(create_union_field_metadata(&field_metadata_collection)?);
        // Share the same immutable dictionary between all fields.
        let dictionary_browser_supplier = create_dictionary_browser_supplier(
            dictionary_input.as_ref().as_ref(),
            &union_field_metadata,
            block_decoder,
            dictionary_on_heap,
        )?;
        let field_infos = Arc::new(field_infos.clone());
        for field_metadata in field_metadata_collection {
            let name = field_metadata.field_name()?.to_string();
            reader.field_to_terms_map.insert(
                name,
                FieldTerms::Shared(STUniformSplitTerms::new(
                    Arc::clone(block_input),
                    Arc::new(field_metadata),
                    Arc::clone(&union_field_metadata),
                    Arc::clone(postings_reader),
                    block_decoder.cloned(),
                    Arc::clone(&field_infos),
                    Arc::clone(&dictionary_browser_supplier),
                )),
            );
        }
    }
    Ok(())
}

/// Creates a virtual [`FieldMetadata`] that is the union of the given
/// [`FieldMetadata`]s. Its first block start, last block start and last term
/// are respectively the min and max among the [`FieldMetadata`]s provided as
/// parameter.
///
/// Port of `STUniformSplitTermsReader.createUnionFieldMetadata(Iterable)`.
///
/// # Errors
///
/// `IllegalStateException` for no field metadata.
pub fn create_union_field_metadata(
    field_metadata_iterable: &[FieldMetadata],
) -> Result<FieldMetadata> {
    let mut builder = UnionFieldMetadataBuilder::new();
    for field_metadata in field_metadata_iterable {
        builder.add_field_metadata(field_metadata);
    }
    builder.build()
}
