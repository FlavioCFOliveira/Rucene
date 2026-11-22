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

//! Port of `TermVectorFilteredLeafReader.java`, of the Java package
//! `org.apache.lucene.search.uhighlight`.

use std::fmt;
use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::filter_reader::FilterLeafReader;
use rucene::index::index_reader::CacheHelper;
use rucene::index::{LeafReader, PostingsEnum, Terms, TermsEnum};
use rucene::util::automaton::compiled_automaton::CompiledAutomaton;
use rucene::util::BytesRef;

use crate::wrappers::{forward_terms, forward_terms_enum};

/// A filtered LeafReader that only includes the terms that are also in a
/// provided set of terms. Certain methods may be unimplemented or cause
/// large operations on the underlying reader and be slow.
///
/// Equivalent to the package-private final class
/// `org.apache.lucene.search.uhighlight.TermVectorFilteredLeafReader`.
pub(crate) struct TermVectorFilteredLeafReader {
    base: FilterLeafReader,
    // NOTE: super ("in") is baseLeafReader
    filter_terms: Arc<dyn Terms>,
    field_filter: String,
}

impl TermVectorFilteredLeafReader {
    /// Equivalent to `new TermVectorFilteredLeafReader(LeafReader, Terms,
    /// String)`: `base_leaf_reader` is the reader to filter, `filter_terms`
    /// the terms to filter by.
    pub(crate) fn new(
        base_leaf_reader: Arc<dyn LeafReader>,
        filter_terms: Arc<dyn Terms>,
        field_filter: &str,
    ) -> Result<Self> {
        Ok(Self {
            base: FilterLeafReader::new(base_leaf_reader)?,
            filter_terms,
            field_filter: field_filter.to_string(),
        })
    }
}

impl fmt::Debug for TermVectorFilteredLeafReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TermVectorFilteredLeafReader")
            .field("field_filter", &self.field_filter)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for TermVectorFilteredLeafReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.base, f)
    }
}

impl LeafReader for TermVectorFilteredLeafReader {
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
        if field != self.field_filter {
            return self.base.terms(field); // proceed like normal for fields we're not interested in
        }
        let terms = self.base.get_delegate().terms(field)?;
        Ok(terms.map(|terms| {
            Arc::new(TermsFilteredTerms {
                inner: terms,
                filter_terms: Arc::clone(&self.filter_terms),
            }) as Arc<dyn Terms>
        }))
    }

    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }

    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }
}

/// Equivalent to the private nested class
/// `TermVectorFilteredLeafReader.TermsFilteredTerms`.
pub(crate) struct TermsFilteredTerms {
    inner: Arc<dyn Terms>,
    // NOTE: super ("in") is the baseTerms
    filter_terms: Arc<dyn Terms>,
}

impl Terms for TermsFilteredTerms {
    // TODO delegate size() ?

    // TODO delegate getMin, getMax to filterTerms

    fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
        Ok(Box::new(TermVectorFilteredTermsEnum::new(
            self.inner.iterator()?,
            self.filter_terms.iterator()?,
        )))
    }

    fn intersect(
        &self,
        compiled: &CompiledAutomaton,
        start_term: Option<&BytesRef>,
    ) -> Result<Box<dyn TermsEnum>> {
        Ok(Box::new(TermVectorFilteredTermsEnum::new(
            self.inner.iterator()?,
            self.filter_terms.intersect(compiled, start_term)?,
        )))
    }

    forward_terms!(inner);
}

/// Equivalent to the private nested class
/// `TermVectorFilteredLeafReader.TermVectorFilteredTermsEnum`.
pub(crate) struct TermVectorFilteredTermsEnum {
    // NOTE: super ("in") is the filteredTermsEnum. This is different than
    // wrappers above because we navigate the terms using the filter.
    inner: Box<dyn TermsEnum>,
    base_terms_enum: Box<dyn TermsEnum>,
}

impl TermVectorFilteredTermsEnum {
    fn new(base_terms_enum: Box<dyn TermsEnum>, filtered_terms_enum: Box<dyn TermsEnum>) -> Self {
        Self {
            inner: filtered_terms_enum, // note this is reversed from constructors above
            base_terms_enum,
        }
    }

    fn move_to_current_term(&mut self) -> Result<()> {
        let current_term = self
            .inner
            .term()?
            .cloned()
            .ok_or_else(LuceneError::null_pointer)?; // from filteredTermsEnum
        let term_in_both_terms_enum = self.base_terms_enum.seek_exact(&current_term)?;

        if !term_in_both_terms_enum {
            return Err(LuceneError::illegal_state(format!(
                "Term vector term '{}' does not appear in full index.",
                current_term.utf8_to_string()?
            )));
        }
        Ok(())
    }
}

impl fmt::Display for TermVectorFilteredTermsEnum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&*self.inner, f)
    }
}

impl TermsEnum for TermVectorFilteredTermsEnum {
    forward_terms_enum!(inner);

    // TODO delegate docFreq & ttf (moveToCurrentTerm() then call on full?

    fn postings(
        &mut self,
        reuse: Option<Box<dyn PostingsEnum>>,
        flags: i32,
    ) -> Result<Option<Box<dyn PostingsEnum>>> {
        self.move_to_current_term()?;
        self.base_terms_enum.postings(reuse, flags)
    }
}