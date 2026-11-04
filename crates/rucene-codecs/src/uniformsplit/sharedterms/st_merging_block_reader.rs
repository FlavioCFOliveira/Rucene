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

//! Port of `org.apache.lucene.codecs.uniformsplit.sharedterms.STMergingBlockReader`.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use rucene::codecs::{BlockTermState, PostingsReaderBase};
use rucene::error::{LuceneError, Result};
use rucene::index::index_reader::java_object_to_string;
use rucene::index::{FieldInfos, ImpactsEnum, PostingsEnum, SeekStatus, TermState, TermsEnum};
use rucene::store::{ByteArrayDataInput, IndexInput};
use rucene::util::attribute::AttributeSource;
use rucene::util::BytesRef;

use super::st_block_line;
use crate::blocktreeords::array_len;
use crate::uniformsplit::block_decoder::BlockDecoder;
use crate::uniformsplit::block_reader::{BlockReader, SharedTermsHooks};
use crate::uniformsplit::field_metadata::FieldMetadata;
use crate::uniformsplit::index_dictionary::BrowserSupplier;

/// `TermsEnum` used when merging segments, to enumerate the terms of an input
/// segment and get all the fields `TermState`s of each term.
///
/// It only supports calls to `next()` and no seek method.
///
/// Port of `org.apache.lucene.codecs.uniformsplit.sharedterms.STMergingBlockReader`
/// (`@lucene.experimental`), which extends `STBlockReader`: the base reader
/// is `base`, with `STBlockReader`'s `SharedTermsHooks`.
pub struct STMergingBlockReader {
    pub(crate) base: BlockReader,
    pub(crate) postings: MergingPostings,
}

/// What `STMergingBlockReader.postings(String, BlockTermState, PostingsEnum,
/// int)` reads: the postings reader and the field infos of the segment.
///
/// Java's `STUniformSplitTermsWriter.SegmentPostings` keeps the merging
/// block reader to call that method; it keeps this handle, the only state
/// the method reads.
#[derive(Clone)]
pub struct MergingPostings {
    pub(crate) postings_reader: Arc<dyn PostingsReaderBase>,
    pub(crate) field_infos: Arc<FieldInfos>,
}

impl MergingPostings {
    /// Creates a new `PostingsEnum` for the provided field and
    /// [`BlockTermState`].
    ///
    /// `reuse` is a previous `PostingsEnum` to reuse, or `None` to create a
    /// new one; `flags` the postings flags.
    ///
    /// Port of `STMergingBlockReader.postings(String, BlockTermState,
    /// PostingsEnum, int)`.
    ///
    /// # Errors
    ///
    /// The errors of the postings reader; `NullPointerException` for an
    /// unknown field.
    pub fn postings(
        &self,
        field_name: &str,
        term_state: &BlockTermState,
        reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Box<dyn PostingsEnum>> {
        let Some(field_info) = self.field_infos.field_info(field_name) else {
            return Err(crate::uniformsplit::npe(
                "Cannot invoke \"org.apache.lucene.index.FieldInfo.getIndexOptions()\" because \
                 \"fieldInfo\" is null",
            ));
        };
        self.postings_reader
            .postings(field_info, term_state, reuse, flags)
    }
}

impl STMergingBlockReader {
    /// Port of `STMergingBlockReader(IndexDictionary.BrowserSupplier,
    /// IndexInput, PostingsReaderBase, FieldMetadata, BlockDecoder,
    /// FieldInfos)`.
    ///
    /// # Errors
    ///
    /// The errors of the base reader's constructor.
    pub fn new(
        dictionary_browser_supplier: Arc<dyn BrowserSupplier>,
        block_input: Arc<Box<dyn IndexInput>>,
        postings_reader: Arc<dyn PostingsReaderBase>,
        field_metadata: Arc<FieldMetadata>,
        block_decoder: Option<Arc<dyn BlockDecoder>>,
        field_infos: Arc<FieldInfos>,
    ) -> Result<Self> {
        let mut base = BlockReader::new(
            dictionary_browser_supplier,
            block_input,
            Arc::clone(&postings_reader),
            field_metadata,
            block_decoder,
        )?;
        base.shared_terms = Some(SharedTermsHooks {
            field_infos: Arc::clone(&field_infos),
            bounded: true,
        });
        Ok(Self {
            base,
            postings: MergingPostings {
                postings_reader,
                field_infos,
            },
        })
    }

    /// Port of `STMergingBlockReader.next()`: `nextTerm()`.
    ///
    /// # Errors
    ///
    /// `CorruptIndexException` for a missing block; I/O errors.
    pub fn next_impl(&mut self) -> Result<Option<BytesRef>> {
        self.base.next_term()
    }

    /// Port of `STMergingBlockReader.postings(String, BlockTermState,
    /// PostingsEnum, int)`; see [`MergingPostings::postings`].
    ///
    /// # Errors
    ///
    /// See [`MergingPostings::postings`].
    pub fn postings_for_field(
        &self,
        field_name: &str,
        term_state: &BlockTermState,
        reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Box<dyn PostingsEnum>> {
        self.postings.postings(field_name, term_state, reuse, flags)
    }

    /// Reads all the fields `TermState`s of the current term and put them in
    /// the provided map. Clears the map first, before putting `TermState`s.
    ///
    /// Port of `STMergingBlockReader.readFieldTermStatesMap(Map)`; Java
    /// leaves the map as it is when the reader is not positioned.
    ///
    /// # Errors
    ///
    /// The I/O errors of decoding the term states.
    pub fn read_field_term_states_map(
        &mut self,
        field_term_states_map: &mut HashMap<String, BlockTermState>,
    ) -> Result<()> {
        if self.base.current_term().is_none() {
            return Ok(());
        }
        let header = self.base.header()?;
        let Some(line) = self.base.block_line else {
            return Ok(());
        };
        let pos = self.base.block_first_line_start
            + array_len(header.get_term_states_base_offset())?
            + array_len(line.term_state_relative_offset)?;
        let (offset, limit) = self.base.block_window;
        let bytes: &[u8] = match &self.base.decoded_block {
            Some(decoded) => &decoded.bytes,
            None => &self.base.scratch_block_bytes,
        };
        let mut input = ByteArrayDataInput::with_range(bytes, offset, limit - offset);
        input.set_position(pos);
        let read = st_block_line::read_field_term_states_map(
            &mut input,
            &self.base.term_state_serializer,
            &header,
            &self.postings.field_infos,
            field_term_states_map,
        );
        self.base.term_states_pos = input.position();
        read
    }
}

impl fmt::Display for STMergingBlockReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&java_object_to_string(
            "org.apache.lucene.codecs.uniformsplit.sharedterms.STMergingBlockReader",
            self,
        ))
    }
}

fn unsupported() -> LuceneError {
    LuceneError::UnsupportedOperation(String::new())
}

/// Java's `readTermStateIfNotRead()` override throws
/// `UnsupportedOperationException`, which every method that reads the term
/// state raises.
impl TermsEnum for STMergingBlockReader {
    fn attributes(&mut self) -> &mut AttributeSource {
        &mut self.base.attributes
    }

    fn seek_exact(&mut self, _text: &BytesRef) -> Result<bool> {
        Err(unsupported())
    }

    fn seek_ceil(&mut self, _text: &BytesRef) -> Result<SeekStatus> {
        Err(unsupported())
    }

    fn seek_ord(&mut self, _ord: i64) -> Result<()> {
        Err(unsupported())
    }

    fn seek_term_state(&mut self, _text: &BytesRef, _state: &dyn TermState) -> Result<()> {
        Err(unsupported())
    }

    fn term(&self) -> Result<BytesRef> {
        Ok(self.base.term_bytes().unwrap_or_default())
    }

    fn ord(&self) -> Result<i64> {
        Err(unsupported())
    }

    fn doc_freq(&mut self) -> Result<i32> {
        Err(unsupported())
    }

    fn total_term_freq(&mut self) -> Result<i64> {
        Err(unsupported())
    }

    fn postings(
        &mut self,
        _reuse: Option<Box<dyn PostingsEnum>>,
        _flags: i32,
    ) -> Result<Box<dyn PostingsEnum>> {
        Err(unsupported())
    }

    fn impacts(&mut self, _flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        Err(unsupported())
    }

    fn term_state(&mut self) -> Result<Box<dyn TermState>> {
        Err(unsupported())
    }

    fn next(&mut self) -> Result<Option<BytesRef>> {
        self.next_impl()
    }
}
