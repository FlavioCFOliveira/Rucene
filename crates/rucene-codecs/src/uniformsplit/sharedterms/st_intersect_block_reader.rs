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

//! Port of `org.apache.lucene.codecs.uniformsplit.sharedterms.STIntersectBlockReader`.

use std::fmt;
use std::sync::Arc;

use rucene::codecs::PostingsReaderBase;
use rucene::error::{LuceneError, Result};
use rucene::index::index_reader::java_object_to_string;
use rucene::index::{FieldInfos, ImpactsEnum, PostingsEnum, SeekStatus, TermState, TermsEnum};
use rucene::util::attribute::AttributeSource;
use rucene::util::automaton::CompiledAutomaton;
use rucene::util::BytesRef;

use crate::uniformsplit::block_decoder::BlockDecoder;
use crate::uniformsplit::block_reader::SharedTermsHooks;
use crate::uniformsplit::field_metadata::FieldMetadata;
use crate::uniformsplit::index_dictionary::BrowserSupplier;
use crate::uniformsplit::intersect_block_reader::IntersectBlockReader;

/// The "intersect" `TermsEnum` response to `STUniformSplitTerms.intersect`,
/// intersecting the terms with an automaton.
///
/// Port of `org.apache.lucene.codecs.uniformsplit.sharedterms.STIntersectBlockReader`
/// (`@lucene.experimental`), which extends `IntersectBlockReader`: the base
/// reader is `base`, whose `SharedTermsHooks` (not bounded:
/// this class does not override `isBeyondLastTerm` nor `nextTerm`) carry
/// the `readTermState` override.
///
/// Java's comment: this class inherits code from both `IntersectBlockReader`
/// and `STBlockReader`. We choose to extend `IntersectBlockReader` because
/// this is the one that runs the `next()`, reads the block lines and keeps
/// the reader state. But we still need the `STBlockReader` logic to skip
/// terms that do not occur in this `TermsEnum` field. So we end up having a
/// couple of methods directly duplicate from `STBlockReader`.
pub struct STIntersectBlockReader {
    pub(crate) base: IntersectBlockReader,
}

impl STIntersectBlockReader {
    /// Port of `STIntersectBlockReader(CompiledAutomaton, BytesRef,
    /// IndexDictionary.BrowserSupplier, IndexInput, PostingsReaderBase,
    /// FieldMetadata, BlockDecoder, FieldInfos)`.
    ///
    /// # Errors
    ///
    /// The errors of the base reader's constructor.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        compiled: &CompiledAutomaton,
        start_term: Option<&BytesRef>,
        dictionary_browser_supplier: Arc<dyn BrowserSupplier>,
        block_input: Arc<rucene::store::SharedIndexInput>,
        postings_reader: Arc<dyn PostingsReaderBase>,
        field_metadata: Arc<FieldMetadata>,
        block_decoder: Option<Arc<dyn BlockDecoder>>,
        field_infos: Arc<FieldInfos>,
    ) -> Result<Self> {
        let mut base = IntersectBlockReader::new(
            compiled,
            start_term,
            dictionary_browser_supplier,
            block_input,
            postings_reader,
            field_metadata,
            block_decoder,
        )?;
        base.base.shared_terms = Some(SharedTermsHooks {
            field_infos,
            bounded: false,
        });
        Ok(Self { base })
    }

    /// Port of `STIntersectBlockReader.next()`.
    ///
    /// # Errors
    ///
    /// I/O errors.
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

    /// Port of `STIntersectBlockReader.termOccursInField()`.
    fn term_occurs_in_field(&mut self) -> Result<bool> {
        self.base.base.read_term_state_if_not_read()
    }
}

impl fmt::Display for STIntersectBlockReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&java_object_to_string(
            "org.apache.lucene.codecs.uniformsplit.sharedterms.STIntersectBlockReader",
            self,
        ))
    }
}

fn unsupported() -> LuceneError {
    LuceneError::unsupported_operation(String::new())
}

impl TermsEnum for STIntersectBlockReader {
    fn attributes(&mut self) -> &mut AttributeSource {
        self.base.attributes()
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

    fn term(&self) -> Result<Option<BytesRef>> {
        self.base.term()
    }

    fn ord(&self) -> Result<i64> {
        Err(unsupported())
    }

    fn doc_freq(&mut self) -> Result<i32> {
        self.base.doc_freq()
    }

    fn total_term_freq(&mut self) -> Result<i64> {
        self.base.total_term_freq()
    }

    fn postings(
        &mut self,
        reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Option<Box<dyn PostingsEnum>>> {
        self.base.postings(reuse, flags)
    }

    fn impacts(&mut self, flags: i32) -> Result<Box<dyn ImpactsEnum>> {
        self.base.impacts(flags)
    }

    fn term_state(&mut self) -> Result<Box<dyn TermState>> {
        self.base.term_state()
    }

    fn next(&mut self) -> Result<Option<BytesRef>> {
        self.next_impl()
    }
}
