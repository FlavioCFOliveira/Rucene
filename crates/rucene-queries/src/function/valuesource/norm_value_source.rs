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

//! Port of `NormValueSource.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.

use std::sync::Arc;

use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::index::{LeafReaderContext, NumericDocValues};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::similarities::{CollectionStatistics, SimScorer, TermStatistics};
use rucene::util::java_collections::java_string_hash_code;
use rucene::util::BytesRef;

use crate::function::docvalues::float_doc_values::{float_doc_values_members, FloatDocValues};
use crate::function::function_values::FunctionValues;
use crate::function::value_source::{class_hash, ValueSource, ValueSourceContext};
use crate::function::valuesource::doc_freq_value_source::DocFreqValueSource;
use crate::function::valuesource::idf_value_source::{as_tfidf, REQUIRES_TFIDF};

/// Function that returns the decoded norm for every document.
///
/// Equivalent to `org.apache.lucene.queries.function.valuesource.NormValueSource`.
/// This returns the decoded norm, as computed by `TFIDFSimilarity`; note that
/// the configured similarity for the field must be a `TFIDFSimilarity`.
#[derive(Debug, Clone)]
pub struct NormValueSource {
    /// The protected `field` field.
    field: String,
}

impl NormValueSource {
    /// Equivalent to `NormValueSource(String)`.
    pub fn new(field: impl Into<String>) -> Self {
        Self {
            field: field.into(),
        }
    }

    /// Equivalent to `NormValueSource.name()`.
    pub fn name(&self) -> String {
        "norm".to_string()
    }
}

/// The anonymous `FloatDocValues` of `NormValueSource.getValues`.
struct NormValues {
    this: Arc<NormValueSource>,
    sim_scorer: Box<dyn SimScorer + Send + Sync>,
    norms: Option<Box<dyn NumericDocValues>>,
    last_doc_id: i32,
}

impl FunctionValues for NormValues {
    fn float_val(&mut self, doc_id: i32) -> Result<f32> {
        if doc_id < self.last_doc_id {
            return Err(LuceneError::from(LuceneErrorKind::AssertionError(format!(
                "docs out of order: lastDocID={} docID={doc_id}",
                self.last_doc_id
            ))));
        }
        self.last_doc_id = doc_id;
        let mut norm = 1i64;
        if let Some(norms) = self.norms.as_mut() {
            if norms.advance_exact(doc_id)? {
                norm = norms.long_value()?;
            }
        }
        Ok(self.sim_scorer.score(1.0, norm))
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

impl FloatDocValues for NormValues {
    fn vs(&self) -> &dyn ValueSource {
        &*self.this
    }
}

impl ValueSource for NormValueSource {
    fn description(&self) -> String {
        format!("{}({})", self.name(), self.field)
    }

    /// `context.put("searcher", searcher)`.
    fn create_weight(
        &self,
        context: &ValueSourceContext<'_>,
        searcher: &IndexSearcher,
    ) -> Result<()> {
        context.put_searcher(searcher);
        Ok(())
    }

    /// **Divergence.** Java scores with the scorer of the unwrapped
    /// `TFIDFSimilarity` (`NormValueSource.java:66-75`), a borrow of the
    /// searcher's similarity that the returned values could not keep. These
    /// values build the same scorer as an owned one from the searcher's
    /// similarity: a `PerFieldSimilarityWrapper` hands `scorer(...)` to the
    /// similarity of `collectionStats.field()`, which is this field, so the
    /// scorer is the one of the similarity `asTFIDF` found.
    fn get_values<'a>(
        self: Arc<Self>,
        context: &ValueSourceContext<'a>,
        reader_context: &LeafReaderContext,
    ) -> Result<Box<dyn FunctionValues + 'a>> {
        let searcher = DocFreqValueSource::searcher(context)?;
        if as_tfidf(&**searcher.get_similarity(), &self.field).is_none() {
            return Err(LuceneError::unsupported_operation(REQUIRES_TFIDF));
        }
        // Only the norm is used in this function, so the statistics are dummies.
        let sim_scorer = Arc::clone(searcher.get_similarity()).scorer_owned(
            1.0,
            &CollectionStatistics::new(self.field.clone(), 1, 1, 1, 1)?,
            &[TermStatistics::new(
                BytesRef::from_char_sequence("bogus"),
                1,
                1,
            )?],
        );
        let norms = reader_context.leaf_reader().get_norm_values(&self.field)?;
        Ok(Box::new(NormValues {
            this: self,
            sim_scorer,
            norms,
            last_doc_id: -1,
        }))
    }

    /// The same class and an equal field.
    fn value_source_eq(&self, other: &dyn ValueSource) -> bool {
        other
            .as_any()
            .downcast_ref::<NormValueSource>()
            .is_some_and(|other| self.field == other.field)
    }

    /// `this.getClass().hashCode() + field.hashCode()`.
    fn value_source_hash(&self) -> i32 {
        class_hash("org.apache.lucene.queries.function.valuesource.NormValueSource")
            .wrapping_add(java_string_hash_code(&self.field))
    }
}
