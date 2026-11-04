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

//! Port of `org.apache.lucene.codecs.uniformsplit.sharedterms.STBlockReader`.

use std::fmt;
use std::sync::Arc;

use rucene::codecs::PostingsReaderBase;
use rucene::error::{LuceneError, Result};
use rucene::index::index_reader::java_object_to_string;
use rucene::index::{FieldInfos, ImpactsEnum, PostingsEnum, SeekStatus, TermState, TermsEnum};
use rucene::store::IndexInput;
use rucene::util::attribute::AttributeSource;
use rucene::util::BytesRef;

use crate::uniformsplit::block_decoder::BlockDecoder;
use crate::uniformsplit::block_reader::{BlockReader, SharedTermsHooks};
use crate::uniformsplit::field_metadata::FieldMetadata;
use crate::uniformsplit::index_dictionary::BrowserSupplier;

/// Reads terms blocks with the Shared Terms format.
///
/// See [`STBlockWriter`](super::STBlockWriter).
///
/// Port of `org.apache.lucene.codecs.uniformsplit.sharedterms.STBlockReader`
/// (`@lucene.experimental`), which extends `BlockReader`: the base reader is
/// `base`, whose `SharedTermsHooks` carry this class's
/// overrides of `readTermState`, `isBeyondLastTerm` and `nextTerm`.
pub struct STBlockReader {
    pub(crate) base: BlockReader,
}

impl STBlockReader {
    /// Port of `STBlockReader(IndexDictionary.BrowserSupplier, IndexInput,
    /// PostingsReaderBase, FieldMetadata, BlockDecoder, FieldInfos)`.
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
            postings_reader,
            field_metadata,
            block_decoder,
        )?;
        base.shared_terms = Some(SharedTermsHooks {
            field_infos,
            bounded: true,
        });
        Ok(Self { base })
    }

    /// Port of `STBlockReader.next()`.
    ///
    /// # Errors
    ///
    /// `CorruptIndexException` for a missing block; I/O errors.
    pub fn next_impl(&mut self) -> Result<Option<BytesRef>> {
        loop {
            let Some(next) = self.base.next_impl()? else {
                // No more terms.
                return Ok(None);
            };
            // Check if the term occurs for the searched field.
            if self.term_occurs_in_field()? {
                // The term occurs for the searched field.
                return Ok(Some(next));
            }
        }
    }

    /// Port of `STBlockReader.termOccursInField()`.
    fn term_occurs_in_field(&mut self) -> Result<bool> {
        self.base.read_term_state_if_not_read()
    }

    /// Port of `STBlockReader.seekCeil(BytesRef)`.
    ///
    /// # Errors
    ///
    /// `CorruptIndexException` for a missing block; I/O errors.
    pub fn seek_ceil_impl(&mut self, searched_term: &BytesRef) -> Result<SeekStatus> {
        let mut seek_status = self.seek_ceil_ignore_field(searched_term)?;
        if seek_status != SeekStatus::END && !self.term_occurs_in_field()? {
            // The term does not occur for the field.
            // We have to move the iterator to the next valid term for the field.
            let next_term = self.next_impl()?;
            seek_status = if next_term.is_none() {
                SeekStatus::END
            } else {
                SeekStatus::NOT_FOUND
            };
        }
        Ok(seek_status)
    }

    /// Port of the package-private `STBlockReader.seekCeilIgnoreField(BytesRef)`
    /// (visible for testing).
    ///
    /// # Errors
    ///
    /// See `seek_ceil_impl`.
    pub fn seek_ceil_ignore_field(&mut self, searched_term: &BytesRef) -> Result<SeekStatus> {
        self.base.seek_ceil_impl(searched_term)
    }

    /// Port of `STBlockReader.seekExact(BytesRef)`.
    ///
    /// # Errors
    ///
    /// See `seek_ceil_impl`.
    pub fn seek_exact_impl(&mut self, searched_term: &BytesRef) -> Result<bool> {
        if self.base.seek_exact_impl(searched_term)? {
            return self.term_occurs_in_field();
        }
        Ok(false)
    }
}

impl fmt::Display for STBlockReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&java_object_to_string(
            "org.apache.lucene.codecs.uniformsplit.sharedterms.STBlockReader",
            self,
        ))
    }
}

impl TermsEnum for STBlockReader {
    fn attributes(&mut self) -> &mut AttributeSource {
        &mut self.base.attributes
    }

    fn seek_exact(&mut self, text: &BytesRef) -> Result<bool> {
        self.seek_exact_impl(text)
    }

    fn seek_ceil(&mut self, text: &BytesRef) -> Result<SeekStatus> {
        self.seek_ceil_impl(text)
    }

    fn seek_ord(&mut self, _ord: i64) -> Result<()> {
        Err(LuceneError::UnsupportedOperation(String::new()))
    }

    fn seek_term_state(&mut self, text: &BytesRef, state: &dyn TermState) -> Result<()> {
        self.base.seek_exact_state(text, state);
        Ok(())
    }

    fn term(&self) -> Result<BytesRef> {
        Ok(self.base.term_bytes().unwrap_or_default())
    }

    fn ord(&self) -> Result<i64> {
        Err(LuceneError::UnsupportedOperation(String::new()))
    }

    fn doc_freq(&mut self) -> Result<i32> {
        self.base.doc_freq_impl()
    }

    fn total_term_freq(&mut self) -> Result<i64> {
        self.base.total_term_freq_impl()
    }

    fn postings(
        &mut self,
        reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Box<dyn PostingsEnum>> {
        self.base.postings_impl(reuse, flags)
    }

    fn impacts(&mut self, flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        self.base.impacts_impl(flags)
    }

    fn term_state(&mut self) -> Result<Box<dyn TermState>> {
        self.base.term_state_impl()
    }

    fn next(&mut self) -> Result<Option<BytesRef>> {
        self.next_impl()
    }
}
