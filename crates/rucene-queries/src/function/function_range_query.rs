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

//! Port of `FunctionRangeQuery.java`, of the Java package
//! `org.apache.lucene.queries.function`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::doc_id_set_iterator::DocIdSetIterator;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::Query;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::scorable::Scorable;
use rucene::search::score_mode::ScoreMode;
use rucene::search::scorer_supplier::ScorerSupplier;
use rucene::search::segment_cacheable::SegmentCacheable;
use rucene::search::similarities::{Explanation, ExplanationValue};
use rucene::search::weight::{DefaultScorerSupplier, Weight};
use rucene::util::java_collections::{java_boolean_hash_code, java_string_hash_code};

use crate::function::function_values::get_range_scorer;
use crate::function::value_source::{
    new_context, objects_equals, objects_hash, ContextMap, ValueSource, ValueSourceContext,
};
use crate::function::value_source_scorer::ValueSourceScorer;

/// A query wrapping a [`ValueSource`] that matches documents in which the
/// values in the value source match a configured range. The score is the
/// float value.
///
/// Equivalent to the class
/// `org.apache.lucene.queries.function.FunctionRangeQuery`. This can be a slow
/// query if run by itself since it must visit all documents; ideally it is
/// combined with other queries. It is mostly a wrapper around
/// [`get_range_scorer`].
#[derive(Debug, Clone)]
pub struct FunctionRangeQuery {
    value_source: Arc<dyn ValueSource>,
    // These two are declared as strings because getRangeScorer takes String
    // args and parses them.
    lower_val: Option<String>,
    upper_val: Option<String>,
    include_lower: bool,
    include_upper: bool,
}

impl FunctionRangeQuery {
    /// Equivalent to `FunctionRangeQuery(ValueSource, Number, Number,
    /// boolean, boolean)`, which stores `toString()` of each non-`null`
    /// bound; a Java `Number` is an [`ExplanationValue`], whose rendering is
    /// the boxed type's `toString()`.
    pub fn from_numbers(
        value_source: Arc<dyn ValueSource>,
        lower_val: Option<ExplanationValue>,
        upper_val: Option<ExplanationValue>,
        include_lower: bool,
        include_upper: bool,
    ) -> Self {
        Self::new(
            value_source,
            lower_val.map(|v| v.to_string()),
            upper_val.map(|v| v.to_string()),
            include_lower,
            include_upper,
        )
    }

    /// Equivalent to `FunctionRangeQuery(ValueSource, String, String, boolean,
    /// boolean)`; `None` bounds stand for `null`, an open end.
    pub fn new(
        value_source: Arc<dyn ValueSource>,
        lower_val: Option<String>,
        upper_val: Option<String>,
        include_lower: bool,
        include_upper: bool,
    ) -> Self {
        Self {
            value_source,
            lower_val,
            upper_val,
            include_lower,
            include_upper,
        }
    }

    /// Equivalent to `FunctionRangeQuery.getValueSource()`.
    pub fn get_value_source(&self) -> &Arc<dyn ValueSource> {
        &self.value_source
    }

    /// Equivalent to `FunctionRangeQuery.getLowerVal()`.
    pub fn get_lower_val(&self) -> Option<&str> {
        self.lower_val.as_deref()
    }

    /// Equivalent to `FunctionRangeQuery.getUpperVal()`.
    pub fn get_upper_val(&self) -> Option<&str> {
        self.upper_val.as_deref()
    }

    /// Equivalent to `FunctionRangeQuery.isIncludeLower()`.
    pub fn is_include_lower(&self) -> bool {
        self.include_lower
    }

    /// Equivalent to `FunctionRangeQuery.isIncludeUpper()`.
    pub fn is_include_upper(&self) -> bool {
        self.include_upper
    }

    /// Equivalent to the private `FunctionRangeQuery.equalsTo(FunctionRangeQuery)`.
    fn equals_to(&self, other: &FunctionRangeQuery) -> bool {
        self.include_lower == other.include_lower
            && self.include_upper == other.include_upper
            && objects_equals(&self.value_source, &other.value_source)
            && self.lower_val == other.lower_val
            && self.upper_val == other.upper_val
    }
}

impl Query for FunctionRangeQuery {
    fn to_query_string(&self, _field: &str) -> String {
        format!(
            "frange({}):{}{} TO {}{}",
            self.value_source,
            if self.include_lower { '[' } else { '{' },
            self.lower_val.as_deref().unwrap_or("*"),
            self.upper_val.as_deref().unwrap_or("*"),
            if self.include_upper { ']' } else { '}' }
        )
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
        _boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        Ok(Arc::new(FunctionRangeWeight::new(
            Arc::new(self.clone()),
            searcher,
        )?))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
            && other
                .as_any()
                .downcast_ref::<FunctionRangeQuery>()
                .is_some_and(|other| self.equals_to(other))
    }

    /// `classHash() ^ Objects.hash(valueSource, lowerVal, upperVal,
    /// includeLower, includeUpper)`.
    fn query_hash(&self) -> i32 {
        let string_hash = |s: &Option<String>| s.as_deref().map_or(0, java_string_hash_code);
        self.class_hash()
            ^ objects_hash(&[
                self.value_source.value_source_hash(),
                string_hash(&self.lower_val),
                string_hash(&self.upper_val),
                java_boolean_hash_code(self.include_lower),
                java_boolean_hash_code(self.include_upper),
            ])
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.queries.function.FunctionRangeQuery"
    }
}

/// The weight of a [`FunctionRangeQuery`].
///
/// Equivalent to the private inner class
/// `org.apache.lucene.queries.function.FunctionRangeQuery.FunctionRangeWeight`.
#[derive(Debug)]
pub(crate) struct FunctionRangeWeight {
    query: Arc<FunctionRangeQuery>,
    vs_context: Arc<ContextMap>,
}

impl FunctionRangeWeight {
    /// Equivalent to `FunctionRangeWeight(IndexSearcher)`: a new context, and
    /// the callback on the value-source tree.
    ///
    /// # Errors
    ///
    /// Propagates the error of the value source's `create_weight`.
    fn new(query: Arc<FunctionRangeQuery>, searcher: &IndexSearcher) -> Result<Self> {
        let vs_context = new_context(Some(searcher));
        query.value_source.create_weight(&vs_context, searcher)?; // callback on valueSource tree
        Ok(Self {
            query,
            vs_context: Arc::clone(vs_context.map()),
        })
    }

    /// The range scorer of a leaf; `scorer(context)` in Java's `explain`.
    fn range_scorer(&self, context: &LeafReaderContext) -> Result<ValueSourceScorer<'static>> {
        let view = ValueSourceContext::new(Arc::clone(&self.vs_context));
        let function_values = Arc::clone(&self.query.value_source).get_values(&view, context)?;
        get_range_scorer(
            function_values,
            context,
            self.query.lower_val.as_deref(),
            self.query.upper_val.as_deref(),
            self.query.include_lower,
            self.query.include_upper,
        )
    }
}

impl SegmentCacheable for FunctionRangeWeight {
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        false
    }
}

impl Weight for FunctionRangeWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        Arc::clone(&self.query) as Arc<dyn Query>
    }

    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation> {
        let view = ValueSourceContext::new(Arc::clone(&self.vs_context));
        let mut function_values =
            Arc::clone(&self.query.value_source).get_values(&view, context)?;
        // note: by using ValueSourceScorer directly, we avoid calling
        // scorer.advance(doc) and checking if true, which can be slow since if
        // that doc doesn't match, it has to linearly find the next matching
        let mut scorer = self.range_scorer(context)?;
        let description = self.query.to_query_string("");
        if scorer.matches(doc)? {
            scorer.advance(doc)?;
            let score = scorer.score()?;
            Ok(Explanation::matched(
                score,
                description,
                vec![function_values.explain(doc)?],
            ))
        } else {
            Ok(Explanation::no_match(
                description,
                vec![function_values.explain(doc)?],
            ))
        }
    }

    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let scorer = self.range_scorer(context)?;
        Ok(Some(Box::new(DefaultScorerSupplier::new(Box::new(scorer)))))
    }
}
