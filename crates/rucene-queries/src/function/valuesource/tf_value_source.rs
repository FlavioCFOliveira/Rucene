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

//! Port of `TFValueSource.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::terms::{get_terms, Terms};
use rucene::index::LeafReaderContext;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::similarities::Similarity;
use rucene::util::BytesRef;

use crate::function::docvalues::float_doc_values::{float_doc_values_members, FloatDocValues};
use crate::function::function_values::FunctionValues;
use crate::function::value_source::{ValueSource, ValueSourceContext};
use crate::function::valuesource::doc_freq_value_source::DocFreqValueSource;
use crate::function::valuesource::idf_value_source::{as_tfidf, REQUIRES_TFIDF};
use crate::function::valuesource::term_freq_value_source::{
    advance, freq, term_postings, wrap_io, TermPostings,
};

/// Function that returns `TFIDFSimilarity.tf(float)` for every document.
///
/// Equivalent to the class
/// `org.apache.lucene.queries.function.valuesource.TFValueSource`, a subclass
/// of `TermFreqValueSource`. Note that the configured similarity for the
/// field must be a `TFIDFSimilarity`.
#[derive(Debug, Clone)]
pub struct TFValueSource {
    base: DocFreqValueSource,
}

impl TFValueSource {
    /// Equivalent to `TFValueSource(String, String, String, BytesRef)`.
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

    /// Equivalent to `TFValueSource.name()`.
    pub fn name(&self) -> String {
        "tf".to_string()
    }
}

/// The anonymous `FloatDocValues` of `TFValueSource.getValues`.
///
/// **Divergence.** Java captures the unwrapped `TFIDFSimilarity`; a borrow of
/// it cannot outlive the searcher it was read from, so these values keep the
/// searcher's similarity and unwrap it with `asTFIDF` on each call, which
/// reaches the same similarity.
struct TFValues {
    this: Arc<TFValueSource>,
    terms: Arc<dyn Terms>,
    similarity: Arc<dyn Similarity>,
    docs: TermPostings,
    at_doc: i32,
    last_doc_requested: i32,
}

impl TFValues {
    /// Equivalent to the public `reset()` of the anonymous class.
    fn reset(&mut self) -> Result<()> {
        self.docs = term_postings(&*self.terms, &self.this.base.indexed_bytes)?;
        self.at_doc = -1;
        Ok(())
    }

    /// `similarity.tf(freq)`.
    fn tf(&self, freq: f32) -> Result<f32> {
        as_tfidf(&*self.similarity, &self.this.base.indexed_field)
            .map(|similarity| similarity.tf(freq))
            .ok_or_else(|| LuceneError::unsupported_operation(REQUIRES_TFIDF))
    }

    fn float_val_inner(&mut self, doc: i32) -> Result<f32> {
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
            return self.tf(0.0);
        }

        // a match!
        let freq = freq(&self.docs)?;
        self.tf(freq as f32)
    }
}

impl FunctionValues for TFValues {
    fn float_val(&mut self, doc: i32) -> Result<f32> {
        self.float_val_inner(doc)
            .map_err(|e| wrap_io(e, &self.this.description(), doc))
    }

    float_doc_values_members!(
        byte_val,
        short_val,
        int_val,
        long_val,
        bool_val,
        double_val,
        str_val,
        object_val,
        to_doc_string,
        get_value_filler
    );
}

impl FloatDocValues for TFValues {
    fn vs(&self) -> &dyn ValueSource {
        &*self.this
    }
}

impl ValueSource for TFValueSource {
    fn get_values<'a>(
        self: Arc<Self>,
        context: &ValueSourceContext<'a>,
        reader_context: &LeafReaderContext,
    ) -> Result<Box<dyn FunctionValues + 'a>> {
        let terms = get_terms(
            reader_context.leaf_reader().as_ref(),
            &self.base.indexed_field,
        )?;
        let searcher = DocFreqValueSource::searcher(context)?;
        let similarity = Arc::clone(searcher.get_similarity());
        if as_tfidf(&*similarity, &self.base.indexed_field).is_none() {
            return Err(LuceneError::unsupported_operation(REQUIRES_TFIDF));
        }
        let mut values = TFValues {
            this: self,
            terms,
            similarity,
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
            .hash_with("org.apache.lucene.queries.function.valuesource.TFValueSource")
    }

    fn value_source_eq(&self, other: &dyn ValueSource) -> bool {
        other
            .as_any()
            .downcast_ref::<TFValueSource>()
            .is_some_and(|other| self.base.fields_eq(&other.base))
    }
}
