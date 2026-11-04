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

//! Port of `org.apache.lucene.tests.codecs.uniformsplit.UniformSplitRot13PostingsFormat`.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use rucene::codecs::{
    FieldsConsumer, FieldsProducer, Lucene104PostingsReader, Lucene104PostingsWriter,
    PostingsFormat, PostingsReaderBase, PostingsWriterBase, SegmentReadState, SegmentWriteState,
};
use rucene::error::Result;
use rucene_codecs::uniformsplit::uniform_split_terms_writer::{
    DEFAULT_DELTA_NUM_LINES, DEFAULT_TARGET_NUM_BLOCK_LINES,
};
use rucene_codecs::uniformsplit::{
    BlockDecoder, BlockEncoder, TermsWriterOverrides, UniformSplitTermsReader,
    UniformSplitTermsWriter,
};

use super::rot13_cypher_test_util::{Rot13Decoder, Rot13Encoder};

/// `public static volatile boolean encoderCalled`.
pub static ENCODER_CALLED: AtomicBool = AtomicBool::new(false);
/// `public static volatile boolean decoderCalled`.
pub static DECODER_CALLED: AtomicBool = AtomicBool::new(false);
/// `public static volatile boolean blocksEncoded`.
pub static BLOCKS_ENCODED: AtomicBool = AtomicBool::new(false);
/// `public static volatile boolean fieldsMetadataEncoded`.
pub static FIELDS_METADATA_ENCODED: AtomicBool = AtomicBool::new(false);
/// `public static volatile boolean dictionaryEncoded`.
pub static DICTIONARY_ENCODED: AtomicBool = AtomicBool::new(false);

// The flags are Java `volatile` fields: sequentially consistent loads and
// stores.
const VOLATILE: Ordering = Ordering::SeqCst;

/// `UniformSplitPostingsFormat` with block encoding using ROT13 cypher.
///
/// Port of `UniformSplitRot13PostingsFormat` (`extends PostingsFormat`,
/// named `UniformSplitRot13`). Its subclass
/// [`STUniformSplitRot13PostingsFormat`](super::sharedterms::STUniformSplitRot13PostingsFormat)
/// reuses this format's `fieldsConsumer` and `fieldsProducer` bodies, its
/// `createFieldsConsumer`/`createFieldsProducer` overrides passed as the
/// closures Java dispatches virtually.
#[derive(Debug, Clone)]
pub struct UniformSplitRot13PostingsFormat {
    name: &'static str,
    pub(crate) dictionary_on_heap: bool,
}

impl UniformSplitRot13PostingsFormat {
    /// `new UniformSplitRot13PostingsFormat()`: `this("UniformSplitRot13", false)`.
    pub fn new() -> Self {
        Self::with_name("UniformSplitRot13", false)
    }

    /// The protected `UniformSplitRot13PostingsFormat(String name, boolean
    /// dictionaryOnHeap)`.
    pub(crate) fn with_name(name: &'static str, dictionary_on_heap: bool) -> Self {
        UniformSplitRot13PostingsFormat {
            name,
            dictionary_on_heap,
        }
    }

    /// `resetEncodingFlags()`: every flag back to `false`.
    pub fn reset_encoding_flags() {
        ENCODER_CALLED.store(false, VOLATILE);
        DECODER_CALLED.store(false, VOLATILE);
        BLOCKS_ENCODED.store(false, VOLATILE);
        FIELDS_METADATA_ENCODED.store(false, VOLATILE);
        DICTIONARY_ENCODED.store(false, VOLATILE);
    }

    /// `recordBlockEncodingCall()`.
    pub(crate) fn record_block_encoding_call() {
        if ENCODER_CALLED.load(VOLATILE) {
            BLOCKS_ENCODED.store(true, VOLATILE);
            ENCODER_CALLED.store(false, VOLATILE);
        }
    }

    /// `recordFieldsMetadataEncodingCall()`.
    pub(crate) fn record_fields_metadata_encoding_call() {
        if ENCODER_CALLED.load(VOLATILE) {
            FIELDS_METADATA_ENCODED.store(true, VOLATILE);
            ENCODER_CALLED.store(false, VOLATILE);
        }
    }

    /// `recordDictionaryEncodingCall()`.
    pub(crate) fn record_dictionary_encoding_call() {
        if ENCODER_CALLED.load(VOLATILE) {
            DICTIONARY_ENCODED.store(true, VOLATILE);
            ENCODER_CALLED.store(false, VOLATILE);
        }
    }

    /// `getBlockEncoder()`: ROT13 encoding, whose `writeTo` sets
    /// `encoderCalled`.
    pub fn get_block_encoder(&self) -> Arc<dyn BlockEncoder> {
        Arc::new(Rot13Encoder {
            on_write: Some(|| ENCODER_CALLED.store(true, VOLATILE)),
        })
    }

    /// `getBlockDecoder()`: ROT13 decoding, which sets `decoderCalled`.
    pub fn get_block_decoder(&self) -> Arc<dyn BlockDecoder> {
        Arc::new(Rot13Decoder {
            on_decode: Some(|| DECODER_CALLED.store(true, VOLATILE)),
        })
    }

    /// `fieldsConsumer(SegmentWriteState)` with `createFieldsConsumer` as
    /// `create`: on failure the postings writer is closed while handling the
    /// exception (`IOUtils.closeWhileHandlingException(postingsWriter)`),
    /// which the terms writers' constructors, which take the postings writer,
    /// do themselves.
    pub(crate) fn fields_consumer_with<'a>(
        state: &SegmentWriteState<'a>,
        create: impl FnOnce(
            &SegmentWriteState<'a>,
            Box<dyn PostingsWriterBase>,
        ) -> Result<Box<dyn FieldsConsumer + 'a>>,
    ) -> Result<Box<dyn FieldsConsumer + 'a>> {
        let postings_writer: Box<dyn PostingsWriterBase> =
            Box::new(Lucene104PostingsWriter::new(state)?);
        create(state, postings_writer)
    }

    /// `fieldsProducer(SegmentReadState)` with `createFieldsProducer` as
    /// `create`; the terms readers' constructors close the postings reader
    /// when they fail, as Java's `closeWhileHandlingException` does.
    pub(crate) fn fields_producer_with(
        state: &SegmentReadState<'_>,
        create: impl FnOnce(
            &SegmentReadState<'_>,
            Box<dyn PostingsReaderBase>,
        ) -> Result<Box<dyn FieldsProducer>>,
    ) -> Result<Box<dyn FieldsProducer>> {
        let postings_reader: Box<dyn PostingsReaderBase> =
            Box::new(Lucene104PostingsReader::new(state)?);
        create(state, postings_reader)
    }
}

/// The overrides of the anonymous `UniformSplitTermsWriter` subclass of
/// `createFieldsConsumer`.
struct Rot13WriterOverrides;

impl TermsWriterOverrides for Rot13WriterOverrides {
    fn write_dictionary(&self, super_: &mut dyn FnMut() -> Result<()>) -> Result<()> {
        UniformSplitRot13PostingsFormat::record_block_encoding_call();
        super_()?;
        UniformSplitRot13PostingsFormat::record_dictionary_encoding_call();
        Ok(())
    }

    fn write_encoded_fields_metadata(&self, super_: &mut dyn FnMut() -> Result<()>) -> Result<()> {
        super_()?;
        UniformSplitRot13PostingsFormat::record_fields_metadata_encoding_call();
        Ok(())
    }
}

impl Default for UniformSplitRot13PostingsFormat {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for UniformSplitRot13PostingsFormat {
    /// `PostingsFormat.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PostingsFormat(name={})", self.name)
    }
}

impl PostingsFormat for UniformSplitRot13PostingsFormat {
    fn name(&self) -> &str {
        self.name
    }

    fn fields_consumer<'a>(
        &self,
        state: &SegmentWriteState<'a>,
    ) -> Result<Box<dyn FieldsConsumer + 'a>> {
        Self::fields_consumer_with(state, |state, postings_writer| {
            // createFieldsConsumer
            Ok(Box::new(
                UniformSplitTermsWriter::with_settings(
                    postings_writer,
                    state,
                    DEFAULT_TARGET_NUM_BLOCK_LINES,
                    DEFAULT_DELTA_NUM_LINES,
                    Some(self.get_block_encoder()),
                )?
                .with_overrides(Arc::new(Rot13WriterOverrides)),
            ))
        })
    }

    fn fields_producer<'a>(&self, state: &SegmentReadState<'a>) -> Result<Box<dyn FieldsProducer>> {
        Self::fields_producer_with(state, |state, postings_reader| {
            // createFieldsProducer
            Ok(Box::new(UniformSplitTermsReader::new(
                postings_reader,
                state,
                Some(self.get_block_decoder()),
                self.dictionary_on_heap,
            )?))
        })
    }
}
