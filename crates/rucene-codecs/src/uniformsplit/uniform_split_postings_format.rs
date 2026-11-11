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

//! Port of `org.apache.lucene.codecs.uniformsplit.UniformSplitPostingsFormat`.

use std::fmt;
use std::sync::Arc;

use rucene::codecs::{
    FieldsConsumer, FieldsProducer, Lucene104PostingsReader, Lucene104PostingsWriter,
    PostingsFormat, SegmentReadState, SegmentWriteState,
};
use rucene::error::{LuceneError, Result};

use super::block_decoder::BlockDecoder;
use super::block_encoder::BlockEncoder;
use super::uniform_split_terms_reader::UniformSplitTermsReader;
use super::uniform_split_terms_writer::{
    validate_settings, UniformSplitTermsWriter, DEFAULT_DELTA_NUM_LINES,
    DEFAULT_TARGET_NUM_BLOCK_LINES,
};

/// Extension of the file containing the terms dictionary (the FST "trie").
///
/// Port of `UniformSplitPostingsFormat.TERMS_DICTIONARY_EXTENSION`.
pub const TERMS_DICTIONARY_EXTENSION: &str = "ustd";

/// Extension of the file containing the terms blocks for each field and the
/// fields metadata.
///
/// Port of `UniformSplitPostingsFormat.TERMS_BLOCKS_EXTENSION`.
pub const TERMS_BLOCKS_EXTENSION: &str = "ustb";

/// Port of `UniformSplitPostingsFormat.VERSION_START`.
pub const VERSION_START: i32 = 0;
/// Port of `UniformSplitPostingsFormat.VERSION_ENCODABLE_FIELDS_METADATA`.
pub const VERSION_ENCODABLE_FIELDS_METADATA: i32 = 1;
/// Port of `UniformSplitPostingsFormat.VERSION_CURRENT`.
pub const VERSION_CURRENT: i32 = VERSION_ENCODABLE_FIELDS_METADATA;

/// Port of `UniformSplitPostingsFormat.NAME`.
pub const NAME: &str = "UniformSplit";

/// `PostingsFormat` based on the Uniform Split technique.
///
/// See [`UniformSplitTermsWriter`].
///
/// Port of `org.apache.lucene.codecs.uniformsplit.UniformSplitPostingsFormat`
/// (`@lucene.experimental`). Its `toString()` is `PostingsFormat`'s,
/// `PostingsFormat(name=UniformSplit)`, which the trait object's
/// [`fmt::Display`] prints.
#[derive(Clone)]
pub struct UniformSplitPostingsFormat {
    pub(crate) name: &'static str,
    pub(crate) target_num_block_lines: i32,
    pub(crate) delta_num_lines: i32,
    pub(crate) block_encoder: Option<Arc<dyn BlockEncoder>>,
    pub(crate) block_decoder: Option<Arc<dyn BlockDecoder>>,
    pub(crate) dictionary_on_heap: bool,
}

impl fmt::Debug for UniformSplitPostingsFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UniformSplitPostingsFormat")
            .field("name", &self.name)
            .field("target_num_block_lines", &self.target_num_block_lines)
            .field("delta_num_lines", &self.delta_num_lines)
            .field(
                "block_encoder",
                &self.block_encoder.as_ref().map(|e| e.to_string()),
            )
            .field(
                "block_decoder",
                &self.block_decoder.as_ref().map(|d| d.to_string()),
            )
            .field("dictionary_on_heap", &self.dictionary_on_heap)
            .finish()
    }
}

impl Default for UniformSplitPostingsFormat {
    fn default() -> Self {
        Self::new()
    }
}

impl UniformSplitPostingsFormat {
    /// Creates a [`UniformSplitPostingsFormat`] with default settings.
    ///
    /// Port of `UniformSplitPostingsFormat()`.
    pub fn new() -> Self {
        Self {
            name: NAME,
            target_num_block_lines: DEFAULT_TARGET_NUM_BLOCK_LINES,
            delta_num_lines: DEFAULT_DELTA_NUM_LINES,
            block_encoder: None,
            block_decoder: None,
            dictionary_on_heap: false,
        }
    }

    /// Port of `UniformSplitPostingsFormat(int, int, BlockEncoder,
    /// BlockDecoder, boolean)`.
    ///
    /// `target_num_block_lines` is the target number of lines per block.
    /// Must be strictly greater than 0. The parameters can be pre-validated
    /// with [`validate_settings`]. There is one term per block line, with its
    /// corresponding details (`TermState`). `delta_num_lines` is the maximum
    /// allowed delta variation of the number of lines per block. Must be
    /// greater than or equal to 0 and strictly less than
    /// `target_num_block_lines`. The block size will be
    /// `target_num_block_lines`+-`delta_num_lines`. The block size must
    /// always be less than or equal to
    /// [`MAX_NUM_BLOCK_LINES`](super::uniform_split_terms_writer::MAX_NUM_BLOCK_LINES).
    /// `block_encoder` is an optional block encoder. If present, it is used
    /// to encode all terms blocks, as well as the FST dictionary and the
    /// fields metadata. `block_decoder` is an optional block decoder. If
    /// present, it is used to decode all terms blocks, as well as the FST
    /// dictionary and the fields metadata. `dictionary_on_heap` forces
    /// loading the terms dictionary on-heap. By default it is kept off-heap
    /// without impact on performance. If block encoding/decoding is used,
    /// then the dictionary is always loaded on-heap whatever this parameter
    /// value is.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for invalid settings, or for only one of
    /// the block encoder and decoder.
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

    /// Port of the protected `UniformSplitPostingsFormat(String, int, int,
    /// BlockEncoder, BlockDecoder, boolean)`.
    ///
    /// # Errors
    ///
    /// See `with_settings`.
    pub fn with_name(
        name: &'static str,
        target_num_block_lines: i32,
        delta_num_lines: i32,
        block_encoder: Option<Arc<dyn BlockEncoder>>,
        block_decoder: Option<Arc<dyn BlockDecoder>>,
        dictionary_on_heap: bool,
    ) -> Result<Self> {
        validate_settings(target_num_block_lines, delta_num_lines)?;
        validate_block_encoder(block_encoder.as_deref(), block_decoder.as_deref())?;
        Ok(Self {
            name,
            target_num_block_lines,
            delta_num_lines,
            block_encoder,
            block_decoder,
            dictionary_on_heap,
        })
    }

    /// The postings writer of `fieldsConsumer`.
    pub(crate) fn postings_writer(
        state: &SegmentWriteState<'_>,
    ) -> Result<Box<Lucene104PostingsWriter>> {
        Ok(Box::new(Lucene104PostingsWriter::new(state)?))
    }

    /// The postings reader of `fieldsProducer`.
    pub(crate) fn postings_reader(
        state: &SegmentReadState<'_>,
    ) -> Result<Box<Lucene104PostingsReader>> {
        Ok(Box::new(Lucene104PostingsReader::new(state)?))
    }
}

/// Port of `UniformSplitPostingsFormat.validateBlockEncoder(BlockEncoder,
/// BlockDecoder)`.
fn validate_block_encoder(
    block_encoder: Option<&dyn BlockEncoder>,
    block_decoder: Option<&dyn BlockDecoder>,
) -> Result<()> {
    if block_encoder.is_some() != block_decoder.is_some() {
        return Err(LuceneError::illegal_argument(format!(
            "Invalid blockEncoder={} and blockDecoder={}, both must be null or both must be \
             non-null",
            block_encoder.map_or_else(|| "null".to_string(), |e| e.to_string()),
            block_decoder.map_or_else(|| "null".to_string(), |d| d.to_string()),
        )));
    }
    Ok(())
}

impl PostingsFormat for UniformSplitPostingsFormat {
    fn name(&self) -> &str {
        self.name
    }

    /// Port of `UniformSplitPostingsFormat.fieldsConsumer(SegmentWriteState)`
    /// with `createUniformSplitTermsWriter`.
    fn fields_consumer<'a>(
        &self,
        state: &SegmentWriteState<'a>,
    ) -> Result<Box<dyn FieldsConsumer + 'a>> {
        let postings_writer = Self::postings_writer(state)?;
        // `UniformSplitTermsWriter::with_settings` closes the postings writer when it fails.
        Ok(Box::new(UniformSplitTermsWriter::with_settings(
            postings_writer,
            state,
            self.target_num_block_lines,
            self.delta_num_lines,
            self.block_encoder.clone(),
        )?))
    }

    /// Port of `UniformSplitPostingsFormat.fieldsProducer(SegmentReadState)`
    /// with `createUniformSplitTermsReader`.
    fn fields_producer<'a>(&self, state: &SegmentReadState<'a>) -> Result<Box<dyn FieldsProducer>> {
        let postings_reader = Self::postings_reader(state)?;
        // `UniformSplitTermsReader::new` closes the postings reader when it fails.
        Ok(Box::new(UniformSplitTermsReader::new(
            postings_reader,
            state,
            self.block_decoder.clone(),
            self.dictionary_on_heap,
        )?))
    }
}

#[cfg(test)]
mod tests {
    use rucene::error::LuceneErrorKind;
    use rucene::store::{DataInput, DataOutput};
    use rucene::util::BytesRef;

    use super::*;
    use crate::uniformsplit::block_encoder::WritableBytes;

    struct Identity;

    impl fmt::Display for Identity {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("Identity")
        }
    }

    impl BlockDecoder for Identity {
        fn decode(&self, _block_bytes: &mut dyn DataInput, _length: i64) -> Result<BytesRef> {
            Ok(BytesRef::default())
        }
    }

    struct NoBytes;

    impl WritableBytes for NoBytes {
        fn size(&self) -> i64 {
            0
        }

        fn write_to(&self, _data_output: &mut dyn DataOutput) -> Result<()> {
            Ok(())
        }
    }

    impl BlockEncoder for Identity {
        fn encode(
            &self,
            _block_bytes: &mut dyn DataInput,
            _length: i64,
        ) -> Result<Box<dyn WritableBytes>> {
            Ok(Box::new(NoBytes))
        }
    }

    fn message(r: Result<UniformSplitPostingsFormat>) -> String {
        match r {
            Err(LuceneError {
                kind: LuceneErrorKind::IllegalArgument { message: m, .. },
                ..
            }) => m,
            other => panic!("expected IllegalArgumentException, got {other:?}"),
        }
    }

    /// `UniformSplitTermsWriter.validateSettings`, through the format's
    /// constructor, at every bound.
    #[test]
    fn settings_are_validated() {
        assert!(UniformSplitPostingsFormat::with_settings(1, 0, None, None, false).is_ok());
        assert!(UniformSplitPostingsFormat::with_settings(999, 1, None, None, false).is_ok());
        assert_eq!(
            message(UniformSplitPostingsFormat::with_settings(
                0, 0, None, None, false
            )),
            "Invalid negative or nul targetNumBlockLines=0"
        );
        assert_eq!(
            message(UniformSplitPostingsFormat::with_settings(
                5, -1, None, None, false
            )),
            "Invalid negative deltaNumLines=-1"
        );
        assert_eq!(
            message(UniformSplitPostingsFormat::with_settings(
                5, 5, None, None, false
            )),
            "Invalid too large deltaNumLines=5, it must be < targetNumBlockLines=5"
        );
        assert_eq!(
            message(UniformSplitPostingsFormat::with_settings(
                999, 2, None, None, false
            )),
            "Invalid (targetNumBlockLines + deltaNumLines)=1001, it must be <= \
             MAX_NUM_BLOCK_LINES=1000"
        );
    }

    /// `UniformSplitPostingsFormat.validateBlockEncoder`: both or neither.
    #[test]
    fn block_encoder_and_decoder_go_together() {
        let codec: Arc<Identity> = Arc::new(Identity);
        assert!(UniformSplitPostingsFormat::with_settings(
            32,
            3,
            Some(codec.clone()),
            Some(codec.clone()),
            false
        )
        .is_ok());
        assert_eq!(
            message(UniformSplitPostingsFormat::with_settings(
                32,
                3,
                Some(codec.clone()),
                None,
                false
            )),
            "Invalid blockEncoder=Identity and blockDecoder=null, both must be null or both \
             must be non-null"
        );
        assert_eq!(
            message(UniformSplitPostingsFormat::with_settings(
                32,
                3,
                None,
                Some(codec),
                false
            )),
            "Invalid blockEncoder=null and blockDecoder=Identity, both must be null or both \
             must be non-null"
        );
    }

    /// `DEFAULT_DELTA_NUM_LINES` is `(int) (32 * 0.1)`.
    #[test]
    fn default_settings() {
        let f = UniformSplitPostingsFormat::new();
        assert_eq!((f.target_num_block_lines, f.delta_num_lines), (32, 3));
        assert_eq!(f.name(), "UniformSplit");
    }
}
