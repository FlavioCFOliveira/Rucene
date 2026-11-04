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

//! Port of `org.apache.lucene.codecs.blocktreeords.OrdsFieldReader`.

use std::fmt;
use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::{FieldInfo, IndexOptions, Terms, TermsEnum};
use rucene::store::{ByteArrayDataInput, DataInput, IndexInput};
use rucene::util::automaton::{AutomatonType, CompiledAutomaton};
use rucene::util::fst::FST;
use rucene::util::BytesRef;

use super::fst_ords_outputs::{FSTOrdsOutputs, Output};
use super::java_assert;
use super::ords_block_tree_terms_reader::Shared;
use super::ords_block_tree_terms_writer::{FST_OUTPUTS, OUTPUT_FLAGS_NUM_BITS};
use super::ords_intersect_terms_enum::OrdsIntersectTermsEnum;
use super::ords_segment_terms_enum::OrdsSegmentTermsEnum;

/// The fields of `OrdsFieldReader`, read once per field by the reader's
/// constructor.
pub(crate) struct FieldData {
    pub(crate) num_terms: i64,
    pub(crate) field_info: FieldInfo,
    pub(crate) sum_total_term_freq: i64,
    pub(crate) sum_doc_freq: i64,
    pub(crate) doc_count: i32,
    #[allow(dead_code)]
    pub(crate) index_start_fp: i64,
    pub(crate) root_block_fp: i64,
    pub(crate) root_code: Output,
    pub(crate) min_term: Vec<u8>,
    pub(crate) max_term: Vec<u8>,
    pub(crate) index: Option<Arc<FST<FSTOrdsOutputs>>>,
}

impl FieldData {
    /// Port of `OrdsFieldReader(OrdsBlockTreeTermsReader, FieldInfo, long,
    /// Output, long, long, int, long, IndexInput, BytesRef, BytesRef)`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        field_info: FieldInfo,
        num_terms: i64,
        root_code: Output,
        sum_total_term_freq: i64,
        sum_doc_freq: i64,
        doc_count: i32,
        index_start_fp: i64,
        index_in: Option<&dyn IndexInput>,
        min_term: Vec<u8>,
        max_term: Vec<u8>,
    ) -> Result<Self> {
        java_assert!(num_terms > 0);

        let root_block_fp = ((ByteArrayDataInput::new(root_code.bytes()).read_v_long()? as u64)
            >> OUTPUT_FLAGS_NUM_BITS) as i64;

        let index = match index_in {
            Some(index_in) => {
                let mut clone = index_in.clone_input()?;
                clone.seek(index_start_fp)?;
                let metadata = FST::read_metadata(clone.as_mut(), FST_OUTPUTS)?;
                Some(Arc::new(FST::new(metadata, clone.as_mut())?))
            }
            None => None,
        };

        Ok(Self {
            num_terms,
            field_info,
            sum_total_term_freq,
            sum_doc_freq,
            doc_count,
            index_start_fp,
            root_block_fp,
            root_code,
            min_term,
            max_term,
            index,
        })
    }
}

/// BlockTree's implementation of [`Terms`].
///
/// Port of the package-private
/// `org.apache.lucene.codecs.blocktreeords.OrdsFieldReader`: the field's
/// data and its reader (Java's `parent`).
#[derive(Clone)]
pub struct OrdsFieldReader {
    pub(crate) parent: Arc<Shared>,
    pub(crate) data: Arc<FieldData>,
}

impl OrdsFieldReader {
    pub(crate) fn new(parent: Arc<Shared>, data: Arc<FieldData>) -> Self {
        Self { parent, data }
    }

    /// The field's terms index; `None` is Java's `index == null`.
    pub(crate) fn index(&self) -> Option<&Arc<FST<FSTOrdsOutputs>>> {
        self.data.index.as_ref()
    }
}

impl fmt::Display for OrdsFieldReader {
    /// Port of `OrdsFieldReader.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "OrdsBlockTreeTerms(terms={},postings={},positions={},docs={})",
            self.data.num_terms,
            self.data.sum_doc_freq,
            self.data.sum_total_term_freq,
            self.data.doc_count
        )
    }
}

impl Terms for OrdsFieldReader {
    fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
        Ok(Box::new(OrdsSegmentTermsEnum::new(self.clone())?))
    }

    fn intersect(
        &self,
        compiled: &CompiledAutomaton,
        start_term: Option<&BytesRef>,
    ) -> Result<Box<dyn TermsEnum>> {
        if compiled.automaton_type != AutomatonType::Normal {
            return Err(LuceneError::IllegalArgument(
                "please use CompiledAutomaton.getTermsEnum instead".to_string(),
            ));
        }
        Ok(Box::new(OrdsIntersectTermsEnum::new(
            self.clone(),
            compiled,
            start_term,
        )?))
    }

    fn min(&self) -> Result<Option<BytesRef>> {
        Ok(Some(BytesRef::new(self.data.min_term.clone())))
    }

    fn max(&self) -> Result<Option<BytesRef>> {
        Ok(Some(BytesRef::new(self.data.max_term.clone())))
    }

    fn has_freqs(&self) -> bool {
        self.data
            .field_info
            .get_index_options()
            .subsumes(IndexOptions::DOCS_AND_FREQS)
    }

    fn has_offsets(&self) -> bool {
        self.data
            .field_info
            .get_index_options()
            .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS)
    }

    fn has_positions(&self) -> bool {
        self.data
            .field_info
            .get_index_options()
            .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)
    }

    fn has_payloads(&self) -> bool {
        self.data.field_info.has_payloads()
    }

    fn size(&self) -> Result<i64> {
        Ok(self.data.num_terms)
    }

    fn sum_total_term_freq(&self) -> Result<i64> {
        Ok(self.data.sum_total_term_freq)
    }

    fn sum_doc_freq(&self) -> Result<i64> {
        Ok(self.data.sum_doc_freq)
    }

    fn doc_count(&self) -> Result<i32> {
        Ok(self.data.doc_count)
    }
}
