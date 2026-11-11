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

//! Port of `org.apache.lucene.codecs.uniformsplit.UniformSplitTerms`.

use std::any::Any;
use std::sync::Arc;

use rucene::codecs::PostingsReaderBase;
use rucene::error::{LuceneError, Result};
use rucene::index::{IndexOptions, Terms, TermsEnum};
use rucene::util::automaton::{AutomatonType, CompiledAutomaton};
use rucene::util::BytesRef;

use super::block_decoder::BlockDecoder;
use super::block_reader::BlockReader;
use super::field_metadata::FieldMetadata;
use super::index_dictionary::BrowserSupplier;
use super::intersect_block_reader::IntersectBlockReader;

/// `Terms` based on the Uniform Split technique.
///
/// The index dictionary is lazy loaded only when `TermsEnum.seekCeil` or
/// `TermsEnum.seekExact` are called (it is not loaded for a direct terms
/// enumeration).
///
/// Port of `org.apache.lucene.codecs.uniformsplit.UniformSplitTerms`
/// (`@lucene.experimental`). Its fields are shared handles, so the reader
/// hands out clones of the instance it keeps.
#[derive(Clone)]
pub struct UniformSplitTerms {
    pub(crate) block_input: Arc<rucene::store::SharedIndexInput>,
    pub(crate) field_metadata: Arc<FieldMetadata>,
    pub(crate) postings_reader: Arc<dyn PostingsReaderBase>,
    pub(crate) block_decoder: Option<Arc<dyn BlockDecoder>>,
    pub(crate) dictionary_browser_supplier: Arc<dyn BrowserSupplier>,
}

impl UniformSplitTerms {
    /// Port of `UniformSplitTerms(IndexInput, FieldMetadata,
    /// PostingsReaderBase, BlockDecoder, IndexDictionary.BrowserSupplier)`.
    ///
    /// `block_decoder` is optional; it can be used for decompression or
    /// decryption.
    pub fn new(
        block_input: Arc<rucene::store::SharedIndexInput>,
        field_metadata: Arc<FieldMetadata>,
        postings_reader: Arc<dyn PostingsReaderBase>,
        block_decoder: Option<Arc<dyn BlockDecoder>>,
        dictionary_browser_supplier: Arc<dyn BrowserSupplier>,
    ) -> Self {
        debug_assert!(field_metadata.get_field_info().is_some());
        debug_assert!(field_metadata.get_last_term().is_some());
        Self {
            block_input,
            field_metadata,
            postings_reader,
            block_decoder,
            dictionary_browser_supplier,
        }
    }

    /// Port of `UniformSplitTerms.checkIntersectAutomatonType(CompiledAutomaton)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an automaton that is not `NORMAL`.
    pub fn check_intersect_automaton_type(&self, automaton: &CompiledAutomaton) -> Result<()> {
        // This check is consistent with other impls and precondition stated in javadoc.
        if automaton.automaton_type != AutomatonType::Normal {
            return Err(LuceneError::illegal_argument(
                "please use CompiledAutomaton.getTermsEnum instead".to_string(),
            ));
        }
        Ok(())
    }

    /// The `subsumes` of the field's index options.
    fn index_options_subsume(&self, options: IndexOptions) -> bool {
        self.field_metadata
            .get_field_info()
            .is_some_and(|fi| fi.get_index_options().subsumes(options))
    }

    /// Port of `UniformSplitTerms.iterator()`.
    ///
    /// # Errors
    ///
    /// The errors of the reader's constructor.
    pub fn new_block_reader(&self) -> Result<BlockReader> {
        BlockReader::new(
            Arc::clone(&self.dictionary_browser_supplier),
            Arc::clone(&self.block_input),
            Arc::clone(&self.postings_reader),
            Arc::clone(&self.field_metadata),
            self.block_decoder.clone(),
        )
    }

    /// Port of `UniformSplitTerms.intersect(CompiledAutomaton, BytesRef)`
    /// past the automaton check.
    ///
    /// # Errors
    ///
    /// The errors of the reader's constructor.
    pub fn new_intersect_block_reader(
        &self,
        compiled: &CompiledAutomaton,
        start_term: Option<&BytesRef>,
    ) -> Result<IntersectBlockReader> {
        IntersectBlockReader::new(
            compiled,
            start_term,
            Arc::clone(&self.dictionary_browser_supplier),
            Arc::clone(&self.block_input),
            Arc::clone(&self.postings_reader),
            Arc::clone(&self.field_metadata),
            self.block_decoder.clone(),
        )
    }
}

impl Terms for UniformSplitTerms {
    fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
        Ok(Box::new(self.new_block_reader()?))
    }

    fn intersect(
        &self,
        compiled: &CompiledAutomaton,
        start_term: Option<&BytesRef>,
    ) -> Result<Box<dyn TermsEnum>> {
        self.check_intersect_automaton_type(compiled)?;
        Ok(Box::new(
            self.new_intersect_block_reader(compiled, start_term)?,
        ))
    }

    fn max(&self) -> Result<Option<BytesRef>> {
        Ok(self.field_metadata.get_last_term().cloned())
    }

    fn size(&self) -> Result<i64> {
        Ok(self.field_metadata.get_num_terms())
    }

    fn sum_total_term_freq(&self) -> Result<i64> {
        Ok(self.field_metadata.get_sum_total_term_freq())
    }

    fn sum_doc_freq(&self) -> Result<i64> {
        Ok(self.field_metadata.get_sum_doc_freq())
    }

    fn doc_count(&self) -> Result<i32> {
        Ok(self.field_metadata.get_doc_count())
    }

    fn has_freqs(&self) -> bool {
        self.index_options_subsume(IndexOptions::DOCS_AND_FREQS)
    }

    fn has_offsets(&self) -> bool {
        self.index_options_subsume(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS)
    }

    fn has_positions(&self) -> bool {
        self.index_options_subsume(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)
    }

    fn has_payloads(&self) -> bool {
        self.field_metadata
            .get_field_info()
            .is_some_and(|fi| fi.has_payloads())
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
}
