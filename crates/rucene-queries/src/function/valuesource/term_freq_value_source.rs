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

//! Port of `TermFreqValueSource.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::terms::{get_terms, Terms};
use rucene::index::{LeafReaderContext, PostingsEnum, POSTINGS_ENUM_FREQS};
use rucene::search::doc_id_set_iterator::NO_MORE_DOCS;
use rucene::search::index_searcher::IndexSearcher;
use rucene::util::BytesRef;

use crate::function::docvalues::int_doc_values::{int_doc_values_members, IntDocValues};
use crate::function::function_values::{runtime_exception, FunctionValues};
use crate::function::value_source::{ValueSource, ValueSourceContext};
use crate::function::valuesource::doc_freq_value_source::DocFreqValueSource;

/// The postings of a term of a frequency function: the term's postings, or
/// `None` for the anonymous empty `PostingsEnum` Java substitutes when the
/// term is absent, whose `advance` answers `NO_MORE_DOCS` and whose `freq`
/// answers `0` — the only two members the functions call.
pub(crate) type TermPostings = Option<Box<dyn PostingsEnum>>;

/// `termsEnum.seekExact(indexedBytes) ? termsEnum.postings(null) : null`,
/// the first half of the `reset()` of the frequency functions' values.
///
/// # Errors
///
/// Propagates any I/O error of the terms.
pub(crate) fn term_postings(terms: &dyn Terms, indexed_bytes: &BytesRef) -> Result<TermPostings> {
    let mut terms_enum = terms.iterator()?;
    if terms_enum.seek_exact(indexed_bytes)? {
        terms_enum.postings(None, POSTINGS_ENUM_FREQS)
    } else {
        Ok(None)
    }
}

/// `docs.advance(target)`, the empty postings answering `NO_MORE_DOCS`.
pub(crate) fn advance(docs: &mut TermPostings, target: i32) -> Result<i32> {
    match docs {
        Some(docs) => docs.advance(target),
        None => Ok(NO_MORE_DOCS),
    }
}

/// `docs.freq()`, the empty postings answering `0`.
pub(crate) fn freq(docs: &TermPostings) -> Result<i32> {
    match docs {
        Some(docs) => docs.freq(),
        None => Ok(0),
    }
}

/// `catch (IOException e) { throw new RuntimeException("caught exception in
/// function " + description() + " : doc=" + doc, e); }`.
pub(crate) fn wrap_io(e: LuceneError, description: &str, doc: i32) -> LuceneError {
    if e.is_io_exception() {
        runtime_exception(
            format!("caught exception in function {description} : doc={doc}"),
            e,
        )
    } else {
        e
    }
}

/// Function that returns `PostingsEnum.freq()` for the supplied term in every
/// document.
///
/// Equivalent to the class
/// `org.apache.lucene.queries.function.valuesource.TermFreqValueSource`, a
/// subclass of [`DocFreqValueSource`]. If the term does not exist in the
/// document, returns `0`. If frequencies are omitted, returns `1`.
#[derive(Debug, Clone)]
pub struct TermFreqValueSource {
    base: DocFreqValueSource,
}

impl TermFreqValueSource {
    /// Equivalent to `TermFreqValueSource(String, String, String, BytesRef)`.
    pub fn new(
        field: impl Into<String>,
        val: impl Into<String>,
        indexed_field: impl Into<String>,
        indexed_bytes: BytesRef,
    ) -> Self {
        Self {
            base: DocFreqValueSource::new(field, val, indexed_field, indexed_bytes),
        }
    }

    /// Equivalent to `TermFreqValueSource.name()`.
    pub fn name(&self) -> String {
        "termfreq".to_string()
    }
}

/// The anonymous `IntDocValues` of `TermFreqValueSource.getValues`.
struct TermFreqValues {
    this: Arc<TermFreqValueSource>,
    terms: Arc<dyn Terms>,
    docs: TermPostings,
    at_doc: i32,
    last_doc_requested: i32,
}

impl TermFreqValues {
    /// Equivalent to the public `reset()` of the anonymous class.
    fn reset(&mut self) -> Result<()> {
        self.docs = term_postings(&*self.terms, &self.this.base.indexed_bytes)?;
        self.at_doc = -1;
        Ok(())
    }

    fn int_val_inner(&mut self, doc: i32) -> Result<i32> {
        if doc < self.last_doc_requested {
            // out-of-order access.... reset
            self.reset()?;
        }
        self.last_doc_requested = doc;

        if self.at_doc < doc {
            self.at_doc = advance(&mut self.docs, doc)?;
        }

        if self.at_doc > doc {
            // term doesn't match this document... either because we hit the
            // end, or because the next doc is after this doc.
            return Ok(0);
        }

        // a match!
        freq(&self.docs)
    }
}

impl FunctionValues for TermFreqValues {
    fn int_val(&mut self, doc: i32) -> Result<i32> {
        self.int_val_inner(doc)
            .map_err(|e| wrap_io(e, &self.this.description(), doc))
    }

    int_doc_values_members!(
        byte_val,
        short_val,
        float_val,
        long_val,
        double_val,
        str_val,
        object_val,
        to_doc_string,
        get_range_matcher,
        get_value_filler
    );
}

impl IntDocValues for TermFreqValues {
    fn vs(&self) -> &dyn ValueSource {
        &*self.this
    }
}

impl ValueSource for TermFreqValueSource {
    fn get_values<'a>(
        self: Arc<Self>,
        _context: &ValueSourceContext<'a>,
        reader_context: &LeafReaderContext,
    ) -> Result<Box<dyn FunctionValues + 'a>> {
        let terms = get_terms(
            reader_context.leaf_reader().as_ref(),
            &self.base.indexed_field,
        )?;
        let mut values = TermFreqValues {
            this: self,
            terms,
            docs: None,
            at_doc: -1,
            last_doc_requested: -1,
        };
        values.reset()?;
        Ok(Box::new(values))
    }

    /// The inherited `DocFreqValueSource.createWeight`.
    fn create_weight(
        &self,
        context: &ValueSourceContext<'_>,
        searcher: &IndexSearcher,
    ) -> Result<()> {
        context.put_searcher(searcher);
        Ok(())
    }

    fn description(&self) -> String {
        self.base.description_with(&self.name())
    }

    fn value_source_hash(&self) -> i32 {
        self.base
            .hash_with("org.apache.lucene.queries.function.valuesource.TermFreqValueSource")
    }

    fn value_source_eq(&self, other: &dyn ValueSource) -> bool {
        other
            .as_any()
            .downcast_ref::<TermFreqValueSource>()
            .is_some_and(|other| self.base.fields_eq(&other.base))
    }
}
