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

//! Port of `org.apache.lucene.codecs.blocktreeords.BlockTreeOrdsPostingsFormat`.

use std::fmt;

use rucene::codecs::{
    FieldsConsumer, FieldsProducer, Lucene103BlockTreeTermsWriter, Lucene104PostingsReader,
    Lucene104PostingsWriter, PostingsFormat, SegmentReadState, SegmentWriteState,
};
use rucene::error::Result;

use super::ords_block_tree_terms_reader::OrdsBlockTreeTermsReader;
use super::ords_block_tree_terms_writer::{
    OrdsBlockTreeTermsWriter, DEFAULT_MAX_BLOCK_SIZE, DEFAULT_MIN_BLOCK_SIZE,
};

/// Uses `OrdsBlockTreeTermsWriter` with `Lucene104PostingsWriter`.
///
/// Port of `org.apache.lucene.codecs.blocktreeords.BlockTreeOrdsPostingsFormat`,
/// named `BlockTreeOrds`.
///
/// **Divergence.** Java overrides `toString()` to return
/// `BlockTreeOrds(blocksize=128)`. This type's [`fmt::Display`] does; but
/// `PostingsFormat`'s `toString()` is the trait object's `Display` (`impl
/// Display for dyn PostingsFormat`, `PostingsFormat(name=...)`), which a
/// format cannot override, so a `dyn PostingsFormat` holding this format
/// prints `PostingsFormat(name=BlockTreeOrds)`.
#[derive(Debug, Clone, Copy)]
pub struct BlockTreeOrdsPostingsFormat {
    min_term_block_size: i32,
    max_term_block_size: i32,
}

/// Port of `BlockTreeOrdsPostingsFormat.BLOCK_SIZE`.
// NOTE: must be multiple of 64 because of PackedInts long-aligned encoding/decoding
pub const BLOCK_SIZE: i32 = 128;

impl Default for BlockTreeOrdsPostingsFormat {
    fn default() -> Self {
        Self::new()
    }
}

impl BlockTreeOrdsPostingsFormat {
    /// Port of `BlockTreeOrdsPostingsFormat()`.
    pub fn new() -> Self {
        Self {
            min_term_block_size: DEFAULT_MIN_BLOCK_SIZE,
            max_term_block_size: DEFAULT_MAX_BLOCK_SIZE,
        }
    }

    /// Port of `BlockTreeOrdsPostingsFormat(int, int)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for the block sizes
    /// `Lucene103BlockTreeTermsWriter.validateSettings` rejects.
    pub fn with_block_sizes(min_term_block_size: i32, max_term_block_size: i32) -> Result<Self> {
        Lucene103BlockTreeTermsWriter::validate_settings(min_term_block_size, max_term_block_size)?;
        Ok(Self {
            min_term_block_size,
            max_term_block_size,
        })
    }
}

impl fmt::Display for BlockTreeOrdsPostingsFormat {
    /// Port of `BlockTreeOrdsPostingsFormat.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}(blocksize={BLOCK_SIZE})", self.name())
    }
}

impl PostingsFormat for BlockTreeOrdsPostingsFormat {
    /// `toString()`: this format's [`fmt::Display`].
    fn to_java_string(&self) -> String {
        self.to_string()
    }

    fn name(&self) -> &str {
        "BlockTreeOrds"
    }

    fn fields_consumer<'a>(
        &self,
        state: &SegmentWriteState<'a>,
    ) -> Result<Box<dyn FieldsConsumer + 'a>> {
        let postings_writer = Box::new(Lucene104PostingsWriter::new(state)?);
        // `OrdsBlockTreeTermsWriter::new` closes the postings writer when it fails.
        Ok(Box::new(OrdsBlockTreeTermsWriter::new(
            state,
            postings_writer,
            self.min_term_block_size,
            self.max_term_block_size,
        )?))
    }

    fn fields_producer<'a>(&self, state: &SegmentReadState<'a>) -> Result<Box<dyn FieldsProducer>> {
        let postings_reader = Box::new(Lucene104PostingsReader::new(state)?);
        // `OrdsBlockTreeTermsReader::new` closes the postings reader when it fails.
        Ok(Box::new(OrdsBlockTreeTermsReader::new(
            postings_reader,
            state,
        )?))
    }
}
