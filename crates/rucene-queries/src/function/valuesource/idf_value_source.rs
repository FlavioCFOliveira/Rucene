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

//! Port of `IDFValueSource.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::LeafReaderContext;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::similarities::{Similarity, TFIDFSimilarity};
use rucene::util::BytesRef;

use crate::function::function_values::FunctionValues;
use crate::function::value_source::{ValueSource, ValueSourceContext};
use crate::function::valuesource::doc_freq_value_source::{
    ConstDoubleDocValues, DocFreqValueSource,
};

/// The message of the sources that need a `TFIDFSimilarity`.
pub(crate) const REQUIRES_TFIDF: &str = "requires a TFIDFSimilarity (such as ClassicSimilarity)";

/// Function that returns `TFIDFSimilarity.idf(docFreq, maxDoc)` for every
/// document.
///
/// Equivalent to the class
/// `org.apache.lucene.queries.function.valuesource.IDFValueSource`, a
/// subclass of [`DocFreqValueSource`]. Note that the configured similarity for
/// the field must be a `TFIDFSimilarity`.
#[derive(Debug, Clone)]
pub struct IDFValueSource {
    base: DocFreqValueSource,
}

impl IDFValueSource {
    /// Equivalent to `IDFValueSource(String, String, String, BytesRef)`.
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

    /// Equivalent to `IDFValueSource.name()`.
    pub fn name(&self) -> String {
        "idf".to_string()
    }
}

/// Unwraps per-field similarities down to the similarity of `field`, and
/// returns it when it is a `TFIDFSimilarity`.
///
/// Equivalent to the package-private static
/// `IDFValueSource.asTFIDF(Similarity, String)`, whose `instanceof` tests are
/// [`Similarity::as_per_field_similarity_wrapper`] and
/// [`Similarity::as_tfidf_similarity`].
pub(crate) fn as_tfidf<'s>(
    mut sim: &'s dyn Similarity,
    field: &str,
) -> Option<&'s dyn TFIDFSimilarity> {
    while let Some(wrapper) = sim.as_per_field_similarity_wrapper() {
        sim = wrapper.get(field);
    }
    sim.as_tfidf_similarity()
}

impl ValueSource for IDFValueSource {
    fn get_values<'a>(
        self: Arc<Self>,
        context: &ValueSourceContext<'a>,
        _reader_context: &LeafReaderContext,
    ) -> Result<Box<dyn FunctionValues + 'a>> {
        let searcher = DocFreqValueSource::searcher(context)?;
        let Some(sim) = as_tfidf(&**searcher.get_similarity(), &self.base.field) else {
            return Err(LuceneError::unsupported_operation(REQUIRES_TFIDF));
        };
        let reader = searcher.get_index_reader();
        let docfreq = reader.doc_freq(&self.base.term())?;
        let idf = sim.idf(i64::from(docfreq), i64::from(reader.max_doc()));
        Ok(Box::new(ConstDoubleDocValues::new(f64::from(idf), self)))
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
            .hash_with("org.apache.lucene.queries.function.valuesource.IDFValueSource")
    }

    fn value_source_eq(&self, other: &dyn ValueSource) -> bool {
        other
            .as_any()
            .downcast_ref::<IDFValueSource>()
            .is_some_and(|other| self.base.fields_eq(&other.base))
    }
}
