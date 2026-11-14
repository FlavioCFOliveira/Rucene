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

//! Port of `QueryValueSource.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`, which also declares the
//! package-private class `QueryDocValues`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::LeafReaderContext;
use rucene::search::doc_id_set_iterator::{self, DocIdSetIterator, EmptyDocIdSetIterator};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::{query_to_string, Query};
use rucene::search::score_mode::ScoreMode;
use rucene::search::scorer::Scorer;
use rucene::search::weight::Weight;
use rucene::util::java_fmt::float_to_string;
use rucene::util::mutable::{MutableValue, MutableValueFloat, MutableValueObject};

use crate::function::docvalues::float_doc_values::{float_doc_values_members, FloatDocValues};
use crate::function::function_values::{runtime_exception, FunctionValues, ValueFiller};
use crate::function::value_source::{top_level_context, ValueSource, ValueSourceContext};

/// The weight a [`QueryValueSource`] stores in the context under its
/// identity, `context.put(this, w)`.
#[derive(Debug)]
pub(crate) struct StoredWeight(pub(crate) Arc<dyn Weight>);

/// `QueryValueSource` returns the relevance score of the query.
///
/// Equivalent to `org.apache.lucene.queries.function.valuesource.QueryValueSource`.
#[derive(Debug, Clone)]
pub struct QueryValueSource {
    /// The package-private `q` field.
    q: Arc<dyn Query>,
    /// The package-private `defVal` field.
    def_val: f32,
}

impl QueryValueSource {
    /// Equivalent to `QueryValueSource(Query, float)`; Java's `null` check is
    /// the type itself.
    pub fn new(q: Arc<dyn Query>, def_val: f32) -> Self {
        Self { q, def_val }
    }

    /// Equivalent to `QueryValueSource.getQuery()`.
    pub fn get_query(&self) -> &Arc<dyn Query> {
        &self.q
    }

    /// Equivalent to `QueryValueSource.getDefaultValue()`.
    pub fn get_default_value(&self) -> f32 {
        self.def_val
    }
}

impl ValueSource for QueryValueSource {
    fn description(&self) -> String {
        format!(
            "query({},def={})",
            query_to_string(&*self.q),
            float_to_string(self.def_val)
        )
    }

    fn get_values<'a>(
        self: Arc<Self>,
        fcontext: &ValueSourceContext<'a>,
        reader_context: &LeafReaderContext,
    ) -> Result<Box<dyn FunctionValues + 'a>> {
        Ok(Box::new(QueryDocValues::new(
            self,
            reader_context,
            fcontext,
        )?))
    }

    /// `q.hashCode() * 29`.
    fn value_source_hash(&self) -> i32 {
        self.q.query_hash().wrapping_mul(29)
    }

    /// `QueryValueSource.class == o.getClass()` with an equal query and
    /// default value.
    fn value_source_eq(&self, other: &dyn ValueSource) -> bool {
        other
            .as_any()
            .downcast_ref::<QueryValueSource>()
            .is_some_and(|other| self.q.query_eq(&*other.q) && self.def_val == other.def_val)
    }

    fn create_weight(
        &self,
        context: &ValueSourceContext<'_>,
        searcher: &IndexSearcher,
    ) -> Result<()> {
        let rewritten = searcher.rewrite(Arc::clone(&self.q))?;
        let w = searcher.create_weight(rewritten, ScoreMode::COMPLETE, 1.0)?;
        context.put(self, Arc::new(StoredWeight(w)));
        Ok(())
    }
}

/// The values of a [`QueryValueSource`]: the score of the query for the
/// documents it matches, the default value for the others.
///
/// Equivalent to the package-private class
/// `org.apache.lucene.queries.function.valuesource.QueryDocValues`, a
/// `FloatDocValues`.
pub(crate) struct QueryDocValues {
    vs: Arc<QueryValueSource>,
    reader_context: LeafReaderContext,
    weight: Arc<dyn Weight>,
    def_val: f32,
    q: Arc<dyn Query>,
    /// The `scorer` field, `None` until the first `exists` and when the
    /// weight has no scorer for the leaf.
    scorer: Option<Box<dyn Scorer>>,
    /// Whether `disi` was assigned: Java's `disi == null` test.
    disi_set: bool,
    /// `DocIdSetIterator.empty()`, the `disi` of a leaf without a scorer.
    empty: EmptyDocIdSetIterator,
    /// Whether `tpi` is non-null.
    has_tpi: bool,
    /// The `thisDocMatches` field, `None` for `null`.
    this_doc_matches: Option<bool>,
    last_doc_requested: i32,
}

impl QueryDocValues {
    /// Equivalent to `QueryDocValues(QueryValueSource, LeafReaderContext,
    /// Map)`: the weight stored in the context, created now if the context has
    /// none.
    ///
    /// # Errors
    ///
    /// Propagates the errors of creating the weight.
    pub(crate) fn new(
        vs: Arc<QueryValueSource>,
        reader_context: &LeafReaderContext,
        fcontext: &ValueSourceContext<'_>,
    ) -> Result<Self> {
        let stored = |vs: &QueryValueSource| -> Option<Arc<dyn Weight>> {
            fcontext
                .get(vs)
                .and_then(|w| w.downcast_ref::<StoredWeight>().map(|w| Arc::clone(&w.0)))
        };
        let mut w = stored(&vs);
        if w.is_none() {
            let weight_searcher = match fcontext.searcher() {
                Some(searcher) => searcher,
                None => IndexSearcher::from_context(top_level_context(reader_context), None)?,
            };
            vs.create_weight(fcontext, &weight_searcher)?;
            w = stored(&vs);
        }
        let weight = w.ok_or_else(LuceneError::null_pointer)?;
        Ok(Self {
            def_val: vs.def_val,
            q: Arc::clone(&vs.q),
            vs,
            reader_context: reader_context.clone(),
            weight,
            scorer: None,
            disi_set: false,
            empty: doc_id_set_iterator::empty(),
            has_tpi: false,
            this_doc_matches: None,
            last_doc_requested: -1,
        })
    }

    /// The `disi` field: the scorer's approximation or iterator, or the empty
    /// iterator.
    fn disi(&mut self) -> &mut dyn DocIdSetIterator {
        match self.scorer.as_mut() {
            None => &mut self.empty,
            Some(scorer) => {
                if self.has_tpi {
                    scorer
                        .two_phase_iterator()
                        .expect("INVARIANT: the scorer was observed to have a two-phase view")
                        .approximation()
                } else {
                    scorer.iterator()
                }
            }
        }
    }

    /// `catch (IOException e) { throw new RuntimeException("caught exception
    /// in QueryDocVals(" + q + ") doc=" + doc, e); }`.
    fn wrap(&self, e: LuceneError, doc: i32) -> LuceneError {
        if e.is_io_exception() {
            runtime_exception(
                format!(
                    "caught exception in QueryDocVals({}) doc={doc}",
                    query_to_string(&*self.q)
                ),
                e,
            )
        } else {
            e
        }
    }

    fn exists_inner(&mut self, doc: i32) -> Result<bool> {
        if !self.disi_set {
            self.scorer = self.weight.scorer(&self.reader_context)?;
            if let Some(scorer) = self.scorer.as_mut() {
                self.has_tpi = scorer.two_phase_iterator().is_some();
            }
            self.disi_set = true;
            self.this_doc_matches = None;
        }
        if self.disi().doc_id() < doc {
            self.disi().advance(doc)?;
            self.this_doc_matches = None;
        }
        if self.disi().doc_id() == doc {
            if self.this_doc_matches.is_none() {
                let matches = if self.has_tpi {
                    self.scorer
                        .as_mut()
                        .and_then(|s| s.two_phase_iterator())
                        .expect("INVARIANT: the scorer was observed to have a two-phase view")
                        .matches()?
                } else {
                    true
                };
                self.this_doc_matches = Some(matches);
            }
            Ok(self.this_doc_matches == Some(true))
        } else {
            Ok(false)
        }
    }

    fn float_val_inner(&mut self, doc: i32) -> Result<f32> {
        if self.exists(doc)? {
            self.scorer
                .as_mut()
                .ok_or_else(LuceneError::null_pointer)?
                .score()
        } else {
            Ok(self.def_val)
        }
    }
}

impl FunctionValues for QueryDocValues {
    fn float_val(&mut self, doc: i32) -> Result<f32> {
        self.float_val_inner(doc).map_err(|e| self.wrap(e, doc))
    }

    fn exists(&mut self, doc: i32) -> Result<bool> {
        if doc < self.last_doc_requested {
            return Err(LuceneError::illegal_argument(format!(
                "docs were sent out-of-order: lastDocID={} vs docID={doc}",
                self.last_doc_requested
            )));
        }
        self.last_doc_requested = doc;
        self.exists_inner(doc).map_err(|e| self.wrap(e, doc))
    }

    fn object_val(&mut self, doc: i32) -> Result<Option<MutableValueObject>> {
        Ok(Some(MutableValueObject::Float(self.float_val(doc)?)))
    }

    fn to_doc_string(&mut self, doc: i32) -> Result<String> {
        let value = self.float_val(doc)?;
        Ok(format!(
            "query({},def={})={}",
            query_to_string(&*self.q),
            float_to_string(self.def_val),
            float_to_string(value)
        ))
    }

    fn get_value_filler(&self) -> Box<dyn ValueFiller> {
        Box::new(QueryValuesFiller {
            mval: MutableValueFloat::new(),
        })
    }

    float_doc_values_members!(
        byte_val, short_val, int_val, long_val, bool_val, double_val, str_val
    );
}

/// The anonymous `ValueFiller` of `QueryDocValues.getValueFiller`: the score
/// when the document matches, the default value otherwise. It reads the score
/// through `floatVal`, which is `scorer.score()` for a matching document (see
/// [`ValueFiller`]).
struct QueryValuesFiller {
    mval: MutableValueFloat,
}

impl ValueFiller for QueryValuesFiller {
    fn get_value(&self) -> &dyn MutableValue {
        &self.mval
    }

    fn fill_value(&mut self, values: &mut dyn FunctionValues, doc: i32) -> Result<()> {
        if values.exists(doc)? {
            self.mval.value = values.float_val(doc)?;
            self.mval.exists = true;
        } else {
            self.mval.value = values.float_val(doc)?;
            self.mval.exists = false;
        }
        Ok(())
    }
}

impl FloatDocValues for QueryDocValues {
    fn vs(&self) -> &dyn ValueSource {
        &*self.vs
    }
}
