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

//! Port of `org.apache.lucene.codecs.uniformsplit.sharedterms.STUniformSplitTerms`.

use std::any::Any;
use std::sync::Arc;

use rucene::codecs::PostingsReaderBase;
use rucene::error::Result;
use rucene::index::{FieldInfos, Terms, TermsEnum};
use rucene::store::IndexInput;
use rucene::util::automaton::CompiledAutomaton;
use rucene::util::BytesRef;

use super::st_block_reader::STBlockReader;
use super::st_intersect_block_reader::STIntersectBlockReader;
use super::st_merging_block_reader::STMergingBlockReader;
use crate::uniformsplit::block_decoder::BlockDecoder;
use crate::uniformsplit::field_metadata::FieldMetadata;
use crate::uniformsplit::index_dictionary::BrowserSupplier;
use crate::uniformsplit::uniform_split_terms::UniformSplitTerms;

/// Extends [`UniformSplitTerms`] for a shared-terms dictionary, with all the
/// fields of a term in the same block line.
///
/// Port of `org.apache.lucene.codecs.uniformsplit.sharedterms.STUniformSplitTerms`
/// (`@lucene.experimental`), which extends `UniformSplitTerms`: the base
/// terms are `base`.
#[derive(Clone)]
pub struct STUniformSplitTerms {
    pub(crate) base: UniformSplitTerms,
    pub(crate) union_field_metadata: Arc<FieldMetadata>,
    pub(crate) field_infos: Arc<FieldInfos>,
}

impl STUniformSplitTerms {
    /// Port of `STUniformSplitTerms(IndexInput, FieldMetadata,
    /// FieldMetadata, PostingsReaderBase, BlockDecoder, FieldInfos,
    /// IndexDictionary.BrowserSupplier)`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        block_input: Arc<Box<dyn IndexInput>>,
        field_metadata: Arc<FieldMetadata>,
        union_field_metadata: Arc<FieldMetadata>,
        postings_reader: Arc<dyn PostingsReaderBase>,
        block_decoder: Option<Arc<dyn BlockDecoder>>,
        field_infos: Arc<FieldInfos>,
        dictionary_browser_supplier: Arc<dyn BrowserSupplier>,
    ) -> Self {
        Self {
            base: UniformSplitTerms::new(
                block_input,
                field_metadata,
                postings_reader,
                block_decoder,
                dictionary_browser_supplier,
            ),
            union_field_metadata,
            field_infos,
        }
    }

    /// Port of the package-private `STUniformSplitTerms.createMergingBlockReader()`.
    ///
    /// # Errors
    ///
    /// The errors of the reader's constructor.
    pub fn create_merging_block_reader(&self) -> Result<STMergingBlockReader> {
        STMergingBlockReader::new(
            Arc::clone(&self.base.dictionary_browser_supplier),
            Arc::clone(&self.base.block_input),
            Arc::clone(&self.base.postings_reader),
            Arc::clone(&self.union_field_metadata),
            self.base.block_decoder.clone(),
            Arc::clone(&self.field_infos),
        )
    }
}

impl Terms for STUniformSplitTerms {
    /// Port of `STUniformSplitTerms.iterator()`.
    fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
        Ok(Box::new(STBlockReader::new(
            Arc::clone(&self.base.dictionary_browser_supplier),
            Arc::clone(&self.base.block_input),
            Arc::clone(&self.base.postings_reader),
            Arc::clone(&self.base.field_metadata),
            self.base.block_decoder.clone(),
            Arc::clone(&self.field_infos),
        )?))
    }

    /// Port of `STUniformSplitTerms.intersect(CompiledAutomaton, BytesRef)`.
    fn intersect(
        &self,
        compiled: &CompiledAutomaton,
        start_term: Option<&BytesRef>,
    ) -> Result<Box<dyn TermsEnum>> {
        self.base.check_intersect_automaton_type(compiled)?;
        Ok(Box::new(STIntersectBlockReader::new(
            compiled,
            start_term,
            Arc::clone(&self.base.dictionary_browser_supplier),
            Arc::clone(&self.base.block_input),
            Arc::clone(&self.base.postings_reader),
            Arc::clone(&self.base.field_metadata),
            self.base.block_decoder.clone(),
            Arc::clone(&self.field_infos),
        )?))
    }

    fn max(&self) -> Result<Option<BytesRef>> {
        self.base.max()
    }

    fn size(&self) -> Result<i64> {
        self.base.size()
    }

    fn sum_total_term_freq(&self) -> Result<i64> {
        self.base.sum_total_term_freq()
    }

    fn sum_doc_freq(&self) -> Result<i64> {
        self.base.sum_doc_freq()
    }

    fn doc_count(&self) -> Result<i32> {
        self.base.doc_count()
    }

    fn has_freqs(&self) -> bool {
        self.base.has_freqs()
    }

    fn has_offsets(&self) -> bool {
        self.base.has_offsets()
    }

    fn has_positions(&self) -> bool {
        self.base.has_positions()
    }

    fn has_payloads(&self) -> bool {
        self.base.has_payloads()
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
}
