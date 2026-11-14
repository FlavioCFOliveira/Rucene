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

//! Port of `FunctionQuery.java`, of the Java package
//! `org.apache.lucene.queries.function`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::doc_id_set_iterator::{self, AllDocIdSetIterator, DocIdSetIterator};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::Query;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::scorable::Scorable;
use rucene::search::score_mode::ScoreMode;
use rucene::search::scorer::Scorer;
use rucene::search::scorer_supplier::ScorerSupplier;
use rucene::search::segment_cacheable::SegmentCacheable;
use rucene::search::similarities::Explanation;
use rucene::search::weight::{DefaultScorerSupplier, Weight};

use crate::function::function_values::FunctionValues;
use crate::function::value_source::{new_context, ContextMap, ValueSource, ValueSourceContext};

/// Returns a score for each document based on a [`ValueSource`], often some
/// function of the value of a field.
///
/// Equivalent to the class
/// `org.apache.lucene.queries.function.FunctionQuery`.
#[derive(Debug, Clone)]
pub struct FunctionQuery {
    /// The package-private `func` field.
    func: Arc<dyn ValueSource>,
}

impl FunctionQuery {
    /// Equivalent to `FunctionQuery(ValueSource)`: `func` defines the
    /// function to be used for scoring.
    pub fn new(func: Arc<dyn ValueSource>) -> Self {
        Self { func }
    }

    /// Returns the associated value source.
    ///
    /// Equivalent to `FunctionQuery.getValueSource()`.
    pub fn get_value_source(&self) -> &Arc<dyn ValueSource> {
        &self.func
    }
}

impl Query for FunctionQuery {
    /// Prints a user-readable version of this query: `func.toString()`.
    fn to_query_string(&self, _field: &str) -> String {
        self.func.to_string()
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        visitor.visit_leaf(self);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        _score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        Ok(Arc::new(FunctionWeight::new(
            Arc::new(self.clone()),
            searcher,
            boost,
        )?))
    }

    /// Returns true if `other` is equal to this:
    /// `sameClassAs(other) && func.equals(other.func)`.
    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
            && other
                .as_any()
                .downcast_ref::<FunctionQuery>()
                .is_some_and(|other| self.func.value_source_eq(&*other.func))
    }

    /// `classHash() ^ func.hashCode()`.
    fn query_hash(&self) -> i32 {
        self.class_hash() ^ self.func.value_source_hash()
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.queries.function.FunctionQuery"
    }
}

/// Creates [`FunctionQuery`] scorer instances.
///
/// Equivalent to the protected inner class
/// `org.apache.lucene.queries.function.FunctionQuery.FunctionWeight`.
#[derive(Debug)]
pub struct FunctionWeight {
    query: Arc<FunctionQuery>,
    /// The protected `searcher` field.
    searcher: IndexSearcher,
    /// The protected `boost` field.
    boost: f32,
    /// The protected `context` field.
    context: Arc<ContextMap>,
}

impl FunctionWeight {
    /// Equivalent to `FunctionWeight(IndexSearcher, float)`: a new context,
    /// populated by `func.createWeight(context, searcher)`.
    ///
    /// # Errors
    ///
    /// Propagates the error of the value source's `create_weight`.
    pub fn new(query: Arc<FunctionQuery>, searcher: &IndexSearcher, boost: f32) -> Result<Self> {
        let context = new_context(Some(searcher));
        query.func.create_weight(&context, searcher)?;
        Ok(Self {
            query,
            searcher: searcher.clone(),
            boost,
            context: Arc::clone(context.map()),
        })
    }

    /// The protected `searcher` field.
    pub fn searcher(&self) -> &IndexSearcher {
        &self.searcher
    }

    /// The protected `boost` field.
    pub fn boost(&self) -> f32 {
        self.boost
    }

    /// The protected `context` field.
    pub fn context(&self) -> &Arc<ContextMap> {
        &self.context
    }
}

impl SegmentCacheable for FunctionWeight {
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        false
    }
}

impl Weight for FunctionWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        Arc::clone(&self.query) as Arc<dyn Query>
    }

    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let scorer = AllScorer::new(context, self, self.boost)?;
        Ok(Some(Box::new(DefaultScorerSupplier::new(Box::new(scorer)))))
    }

    /// `((AllScorer) scorer(context)).explain(doc)`.
    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation> {
        AllScorer::new(context, self, self.boost)?.explain(doc)
    }
}

/// Scores all documents, applying the function to each document.
///
/// Equivalent to the protected inner class
/// `org.apache.lucene.queries.function.FunctionQuery.AllScorer`.
pub struct AllScorer {
    /// `FunctionQuery.this.func`.
    func: Arc<dyn ValueSource>,
    /// `weight.boost`.
    weight_boost: f32,
    /// The `maxDoc` field.
    max_doc: i32,
    /// The `boost` field.
    boost: f32,
    /// The `iterator` field.
    iterator: AllDocIdSetIterator,
    /// The `vals` field.
    vals: Box<dyn FunctionValues>,
}

impl std::fmt::Debug for AllScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AllScorer")
            .field("max_doc", &self.max_doc)
            .field("boost", &self.boost)
            .finish_non_exhaustive()
    }
}

impl AllScorer {
    /// Equivalent to `AllScorer(LeafReaderContext, FunctionWeight, float)`.
    ///
    /// # Errors
    ///
    /// Propagates the errors of building the iterator and reading the values.
    pub fn new(context: &LeafReaderContext, w: &FunctionWeight, boost: f32) -> Result<Self> {
        let max_doc = context.leaf_reader().max_doc();
        let iterator = doc_id_set_iterator::all(context.leaf_reader().max_doc())?;
        let func = Arc::clone(&w.query.func);
        let view = ValueSourceContext::new(Arc::clone(&w.context));
        let vals = Arc::clone(&func).get_values(&view, context)?;
        Ok(Self {
            func,
            weight_boost: w.boost,
            max_doc,
            boost,
            iterator,
            vals,
        })
    }

    /// Equivalent to `AllScorer.explain(int)`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while reading the values.
    pub fn explain(&mut self, doc: i32) -> Result<Explanation> {
        let mut expl = self.vals.explain(doc)?;
        if expl.value().float_value() < 0.0 {
            expl = Explanation::matched(
                0,
                "truncated score, max of:",
                vec![
                    Explanation::matched(0f32, "minimum score", Vec::new()),
                    expl,
                ],
            );
        } else if expl.value().float_value().is_nan() {
            expl = Explanation::matched(
                0,
                "score, computed as (score == NaN ? 0 : score) since NaN is an illegal score from:",
                vec![expl],
            );
        }
        let value = self.boost * expl.value().float_value();
        Ok(Explanation::matched(
            value,
            format!("FunctionQuery({}), product of:", self.func),
            vec![
                self.vals.explain(doc)?,
                Explanation::matched(self.weight_boost, "boost", Vec::new()),
            ],
        ))
    }
}

impl Scorable for AllScorer {
    /// Equivalent to `AllScorer.score()`: `boost * val`, or `0` for a
    /// negative or `NaN` value.
    fn score(&mut self) -> Result<f32> {
        let doc = self.iterator.doc_id();
        let val = self.vals.float_val(doc)?;
        // `val >= 0 == false` covers NaN as well since comparisons with NaN
        // return false.
        #[allow(clippy::neg_cmp_op_on_partial_ord)]
        if !(val >= 0.0) {
            Ok(0.0)
        } else {
            Ok(self.boost * val)
        }
    }
}

impl Scorer for AllScorer {
    fn doc_id(&self) -> i32 {
        self.iterator.doc_id()
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        &mut self.iterator
    }

    fn iterator_ref(&self) -> Option<&dyn DocIdSetIterator> {
        Some(&self.iterator)
    }

    fn get_max_score(&mut self, _up_to: i32) -> Result<f32> {
        Ok(f32::INFINITY)
    }
}
