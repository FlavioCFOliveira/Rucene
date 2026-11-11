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

//! Port of `org.apache.lucene.codecs.uniformsplit.sharedterms.STUniformSplitPostingsFormat`.

use std::sync::Arc;

use rucene::codecs::{
    FieldsConsumer, FieldsProducer, PostingsFormat, SegmentReadState, SegmentWriteState,
};
use rucene::error::Result;

use super::st_uniform_split_terms_reader;
use super::st_uniform_split_terms_writer::STUniformSplitTermsWriter;
use crate::uniformsplit::block_decoder::BlockDecoder;
use crate::uniformsplit::block_encoder::BlockEncoder;
use crate::uniformsplit::uniform_split_postings_format::{self, UniformSplitPostingsFormat};
use crate::uniformsplit::uniform_split_terms_writer::{
    DEFAULT_DELTA_NUM_LINES, DEFAULT_TARGET_NUM_BLOCK_LINES,
};

/// Extension of the file containing the terms dictionary (the FST "trie").
///
/// Port of `STUniformSplitPostingsFormat.TERMS_DICTIONARY_EXTENSION`.
pub const TERMS_DICTIONARY_EXTENSION: &str = "stustd";

/// Extension of the file containing the terms blocks for each field and the
/// fields metadata.
///
/// Port of `STUniformSplitPostingsFormat.TERMS_BLOCKS_EXTENSION`.
pub const TERMS_BLOCKS_EXTENSION: &str = "stustb";

/// Port of `STUniformSplitPostingsFormat.VERSION_CURRENT`.
pub const VERSION_CURRENT: i32 = uniform_split_postings_format::VERSION_CURRENT;

/// Port of `STUniformSplitPostingsFormat.NAME`.
pub const NAME: &str = "SharedTermsUniformSplit";

/// `PostingsFormat` based on the Uniform Split technique and supporting
/// Shared Terms.
///
/// Shared Terms means the terms of all fields are stored in the same block
/// file, with multiple fields associated to one term (one block line). In
/// the same way, the dictionary trie is also shared between all fields. This
/// highly reduces the memory required by the field dictionary compared to
/// having one separate dictionary per field.
///
/// Port of `org.apache.lucene.codecs.uniformsplit.sharedterms.STUniformSplitPostingsFormat`
/// (`@lucene.experimental`), which extends `UniformSplitPostingsFormat`: the
/// base format is `base`, named `SharedTermsUniformSplit`.
#[derive(Debug, Clone)]
pub struct STUniformSplitPostingsFormat {
    pub(crate) base: UniformSplitPostingsFormat,
}

impl Default for STUniformSplitPostingsFormat {
    fn default() -> Self {
        Self::new()
    }
}

impl STUniformSplitPostingsFormat {
    /// Creates a [`STUniformSplitPostingsFormat`] with default settings.
    ///
    /// Port of `STUniformSplitPostingsFormat()`.
    pub fn new() -> Self {
        let mut base = UniformSplitPostingsFormat::new();
        base.name = NAME;
        debug_assert_eq!(base.target_num_block_lines, DEFAULT_TARGET_NUM_BLOCK_LINES);
        debug_assert_eq!(base.delta_num_lines, DEFAULT_DELTA_NUM_LINES);
        Self { base }
    }

    /// Port of `STUniformSplitPostingsFormat(int, int, BlockEncoder,
    /// BlockDecoder, boolean)`; see
    /// [`UniformSplitPostingsFormat::with_settings`].
    ///
    /// # Errors
    ///
    /// See [`UniformSplitPostingsFormat::with_settings`].
    pub fn with_settings(
        target_num_block_lines: i32,
        delta_num_lines: i32,
        block_encoder: Option<Arc<dyn BlockEncoder>>,
        block_decoder: Option<Arc<dyn BlockDecoder>>,
        dictionary_on_heap: bool,
    ) -> Result<Self> {
        Self::with_name(
            NAME,
            target_num_block_lines,
            delta_num_lines,
            block_encoder,
            block_decoder,
            dictionary_on_heap,
        )
    }

    /// Port of the protected `STUniformSplitPostingsFormat(String, int, int,
    /// BlockEncoder, BlockDecoder, boolean)`.
    ///
    /// # Errors
    ///
    /// See [`UniformSplitPostingsFormat::with_settings`].
    pub fn with_name(
        name: &'static str,
        target_num_block_lines: i32,
        delta_num_lines: i32,
        block_encoder: Option<Arc<dyn BlockEncoder>>,
        block_decoder: Option<Arc<dyn BlockDecoder>>,
        dictionary_on_heap: bool,
    ) -> Result<Self> {
        Ok(Self {
            base: UniformSplitPostingsFormat::with_name(
                name,
                target_num_block_lines,
                delta_num_lines,
                block_encoder,
                block_decoder,
                dictionary_on_heap,
            )?,
        })
    }
}

impl PostingsFormat for STUniformSplitPostingsFormat {
    fn name(&self) -> &str {
        self.base.name
    }

    /// Port of `UniformSplitPostingsFormat.fieldsConsumer(SegmentWriteState)`
    /// with `STUniformSplitPostingsFormat.createUniformSplitTermsWriter`.
    fn fields_consumer<'a>(
        &self,
        state: &SegmentWriteState<'a>,
    ) -> Result<Box<dyn FieldsConsumer + 'a>> {
        let postings_writer = UniformSplitPostingsFormat::postings_writer(state)?;
        // `STUniformSplitTermsWriter::with_settings` closes the postings writer when it fails.
        Ok(Box::new(STUniformSplitTermsWriter::with_settings(
            postings_writer,
            state,
            self.base.target_num_block_lines,
            self.base.delta_num_lines,
            self.base.block_encoder.clone(),
        )?))
    }

    /// Port of `UniformSplitPostingsFormat.fieldsProducer(SegmentReadState)`
    /// with `STUniformSplitPostingsFormat.createUniformSplitTermsReader`.
    fn fields_producer<'a>(&self, state: &SegmentReadState<'a>) -> Result<Box<dyn FieldsProducer>> {
        let postings_reader = UniformSplitPostingsFormat::postings_reader(state)?;
        // The reader's constructor closes the postings reader when it fails.
        Ok(Box::new(st_uniform_split_terms_reader::new(
            postings_reader,
            state,
            self.base.block_decoder.clone(),
            self.base.dictionary_on_heap,
        )?))
    }
}
