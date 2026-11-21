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

//! Port of `org.apache.lucene.tests.codecs.uniformsplit.sharedterms.STUniformSplitRot13PostingsFormat`.

use std::fmt;
use std::sync::Arc;

use rucene::codecs::{
    FieldsConsumer, FieldsProducer, PostingsFormat, SegmentReadState, SegmentWriteState,
};
use rucene::error::Result;
use rucene_codecs::uniformsplit::sharedterms::st_uniform_split_terms_reader;
use rucene_codecs::uniformsplit::sharedterms::STUniformSplitTermsWriter;
use rucene_codecs::uniformsplit::uniform_split_terms_writer::{
    DEFAULT_DELTA_NUM_LINES, DEFAULT_TARGET_NUM_BLOCK_LINES,
};
use rucene_codecs::uniformsplit::TermsWriterOverrides;

use crate::codecs::uniformsplit::UniformSplitRot13PostingsFormat;

/// `STUniformSplitPostingsFormat` with block encoding using ROT13 cypher.
///
/// Port of `STUniformSplitRot13PostingsFormat` (`extends
/// UniformSplitRot13PostingsFormat`, named `STUniformSplitRot13`): the base
/// format's encoder, decoder and flags, with `createFieldsConsumer` and
/// `createFieldsProducer` overridden.
#[derive(Debug, Clone)]
pub struct STUniformSplitRot13PostingsFormat {
    base: UniformSplitRot13PostingsFormat,
}

impl STUniformSplitRot13PostingsFormat {
    /// `new STUniformSplitRot13PostingsFormat()`: `super("STUniformSplitRot13", false)`.
    pub fn new() -> Self {
        STUniformSplitRot13PostingsFormat {
            base: UniformSplitRot13PostingsFormat::with_name("STUniformSplitRot13", false),
        }
    }
}

/// The overrides of the anonymous `STUniformSplitTermsWriter` subclass of
/// `createFieldsConsumer`: unlike the base format's, its
/// `writeEncodedFieldsMetadata` also records a block encoding call first.
struct STRot13WriterOverrides;

impl TermsWriterOverrides for STRot13WriterOverrides {
    fn write_dictionary(&self, super_: &mut dyn FnMut() -> Result<()>) -> Result<()> {
        UniformSplitRot13PostingsFormat::record_block_encoding_call();
        super_()?;
        UniformSplitRot13PostingsFormat::record_dictionary_encoding_call();
        Ok(())
    }

    fn write_encoded_fields_metadata(&self, super_: &mut dyn FnMut() -> Result<()>) -> Result<()> {
        UniformSplitRot13PostingsFormat::record_block_encoding_call();
        super_()?;
        UniformSplitRot13PostingsFormat::record_fields_metadata_encoding_call();
        Ok(())
    }
}

impl Default for STUniformSplitRot13PostingsFormat {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for STUniformSplitRot13PostingsFormat {
    /// `PostingsFormat.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.base, f)
    }
}

impl PostingsFormat for STUniformSplitRot13PostingsFormat {
    fn name(&self) -> &str {
        self.base.name()
    }

    fn fields_consumer<'a>(
        &self,
        state: &SegmentWriteState<'a>,
    ) -> Result<Box<dyn FieldsConsumer + 'a>> {
        UniformSplitRot13PostingsFormat::fields_consumer_with(state, |state, postings_writer| {
            // createFieldsConsumer
            Ok(Box::new(
                STUniformSplitTermsWriter::with_settings(
                    postings_writer,
                    state,
                    DEFAULT_TARGET_NUM_BLOCK_LINES,
                    DEFAULT_DELTA_NUM_LINES,
                    Some(self.base.get_block_encoder()),
                )?
                .with_overrides(Arc::new(STRot13WriterOverrides)),
            ))
        })
    }

    fn fields_producer<'a>(&self, state: &SegmentReadState<'a>) -> Result<Box<dyn FieldsProducer>> {
        UniformSplitRot13PostingsFormat::fields_producer_with(state, |state, postings_reader| {
            // createFieldsProducer
            Ok(Box::new(st_uniform_split_terms_reader::new(
                postings_reader,
                state,
                Some(self.base.get_block_decoder()),
                self.base.dictionary_on_heap,
            )?))
        })
    }
}
