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

//! Port of `OverlaySingleDocTermsLeafReader.java`, of the Java package
//! `org.apache.lucene.search.uhighlight`.

use std::fmt;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::filter_reader::FilterLeafReader;
use rucene::index::index_reader::CacheHelper;
use rucene::index::{LeafReader, PostingsEnum, Terms, TermsEnum};
use rucene::search::DocIdSetIterator;
use rucene::util::automaton::compiled_automaton::CompiledAutomaton;
use rucene::util::BytesRef;

use crate::wrappers::{forward_terms, forward_terms_enum};

/// Overlays a 2nd LeafReader for the terms of one field, otherwise the
/// primary reader is consulted. The 2nd reader is assumed to have one
/// document of 0 and we remap it to a target doc ID.
///
/// Equivalent to `org.apache.lucene.search.uhighlight.OverlaySingleDocTermsLeafReader`.
pub struct OverlaySingleDocTermsLeafReader {
    base: FilterLeafReader,
    in2: Arc<dyn LeafReader>,
    in2_field: String,
    in2_target_doc_id: i32,
}

impl OverlaySingleDocTermsLeafReader {
    /// Equivalent to `new OverlaySingleDocTermsLeafReader(LeafReader,
    /// LeafReader, String, int)`.
    ///
    /// # Errors
    ///
    /// What registering with the wrapped reader raises (a closed reader).
    pub fn new(
        input: Arc<dyn LeafReader>,
        in2: Arc<dyn LeafReader>,
        in2_field: &str,
        in2_target_doc_id: i32,
    ) -> Result<Self> {
        debug_assert!(in2.max_doc() == 1);
        Ok(Self {
            base: FilterLeafReader::new(input)?,
            in2,
            in2_field: in2_field.to_string(),
            in2_target_doc_id,
        })
    }
}

impl fmt::Debug for OverlaySingleDocTermsLeafReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OverlaySingleDocTermsLeafReader")
            .field("in2_field", &self.in2_field)
            .field("in2_target_doc_id", &self.in2_target_doc_id)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for OverlaySingleDocTermsLeafReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.base, f)
    }
}

impl LeafReader for OverlaySingleDocTermsLeafReader {
    crate::forward_leaf_reader!(filter base;
        core,
        filter_leaf_reader_delegate,
        do_close,
        term_vectors,
        num_docs,
        max_doc,
        stored_fields,
        get_numeric_doc_values,
        get_binary_doc_values,
        get_sorted_doc_values,
        get_sorted_numeric_doc_values,
        get_sorted_set_doc_values,
        get_norm_values,
        get_doc_values_skipper,
        get_float_vector_values,
        get_byte_vector_values,
        search_nearest_vectors,
        search_nearest_vectors_byte,
        get_field_infos,
        get_live_docs,
        get_point_values,
        check_integrity,
        get_meta_data,
    );

    fn terms(&self, field: &str) -> Result<Option<Arc<dyn Terms>>> {
        if self.in2_field != field {
            return self.base.get_delegate().terms(field);
        }

        // Shared-nothing so we need to remap the doc ID.
        let Some(terms) = self.in2.terms(field)? else {
            return Ok(None);
        };
        if self.in2_target_doc_id == 0 {
            // no doc ID remapping to do
            return Ok(Some(terms));
        }
        Ok(Some(Arc::new(RemapTerms {
            inner: terms,
            in2_target_doc_id: self.in2_target_doc_id,
        })))
    }

    // TODO maybe override others, but not necessary at the moment

    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }

    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }
}

/// The anonymous `FilterTerms` remapping document 0 to the target doc.
struct RemapTerms {
    inner: Arc<dyn Terms>,
    in2_target_doc_id: i32,
}

impl RemapTerms {
    fn filter_terms_enum(&self, terms_enum: Box<dyn TermsEnum>) -> Box<dyn TermsEnum> {
        Box::new(RemapTermsEnum {
            inner: terms_enum,
            in2_target_doc_id: self.in2_target_doc_id,
        })
    }
}

impl Terms for RemapTerms {
    fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
        Ok(self.filter_terms_enum(self.inner.iterator()?))
    }

    fn intersect(
        &self,
        compiled: &CompiledAutomaton,
        start_term: Option<&BytesRef>,
    ) -> Result<Box<dyn TermsEnum>> {
        // `super.intersect`: the Terms default over this terms' iterator
        let terms_enum = rucene::index::terms::default_intersect(self, compiled, start_term)?;
        Ok(self.filter_terms_enum(terms_enum))
    }

    forward_terms!(inner);
}

/// The anonymous `FilterTermsEnum` remapping postings.
struct RemapTermsEnum {
    inner: Box<dyn TermsEnum>,
    in2_target_doc_id: i32,
}

impl fmt::Display for RemapTermsEnum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&*self.inner, f)
    }
}

impl TermsEnum for RemapTermsEnum {
    forward_terms_enum!(inner);

    fn postings(
        &mut self,
        reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Option<Box<dyn PostingsEnum>>> {
        Ok(self.inner.postings(reuse, flags)?.map(|inner| {
            Box::new(RemapPostingsEnum {
                inner,
                in2_target_doc_id: self.in2_target_doc_id,
            }) as Box<dyn PostingsEnum>
        }))
    }
}

/// The anonymous `FilterPostingsEnum` reporting document 0 as the target
/// doc.
struct RemapPostingsEnum {
    inner: Box<dyn PostingsEnum>,
    in2_target_doc_id: i32,
}

impl fmt::Display for RemapPostingsEnum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&*self.inner, f)
    }
}

impl fmt::Debug for RemapPostingsEnum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OverlaySingleDocTermsLeafReader$1$1$1").finish_non_exhaustive()
    }
}

impl DocIdSetIterator for RemapPostingsEnum {
    fn doc_id(&self) -> i32 {
        let doc = self.inner.doc_id();
        if doc == 0 {
            self.in2_target_doc_id
        } else {
            doc
        }
    }

    fn next_doc(&mut self) -> Result<i32> {
        let doc = self.inner.next_doc()?;
        Ok(if doc == 0 { self.in2_target_doc_id } else { doc })
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.slow_advance(target)
    }

    fn cost(&self) -> i64 {
        self.inner.cost()
    }
}

impl PostingsEnum for RemapPostingsEnum {
    fn freq(&self) -> Result<i32> {
        self.inner.freq()
    }

    fn next_position(&mut self) -> Result<i32> {
        self.inner.next_position()
    }

    fn start_offset(&self) -> i32 {
        self.inner.start_offset()
    }

    fn end_offset(&self) -> i32 {
        self.inner.end_offset()
    }

    fn get_payload(&self) -> Result<Option<&[u8]>> {
        self.inner.get_payload()
    }
}
