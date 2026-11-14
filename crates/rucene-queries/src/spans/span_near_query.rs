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

//! Port of `SpanNearQuery.java`, of the Java package
//! `org.apache.lucene.queries.spans`.

use std::any::Any;
use std::sync::{Arc, Weak};

use rucene::error::{LuceneError, Result};
use rucene::index::LeafReaderContext;
use rucene::search::boolean_clause::Occur;
use rucene::search::doc_id_set_iterator::DocIdSetIterator;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::matches::Matches;
use rucene::search::query::Query;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::score_mode::ScoreMode;
use rucene::search::scorer_supplier::ScorerSupplier;
use rucene::search::segment_cacheable::SegmentCacheable;
use rucene::search::similarities::Explanation;
use rucene::search::weight::Weight;
use rucene::util::java_collections::java_string_hash_code;

use crate::spans::near_spans_ordered::NearSpansOrdered;
use crate::spans::near_spans_unordered::NearSpansUnordered;
use crate::spans::span_collector::SpanCollector;
use crate::spans::span_query::{
    accept_field, get_term_states_of, span_query_list_eq, span_query_list_hash, SpanQuery,
    TermStatesMap,
};
use crate::spans::span_weight::{
    new_span_weight, span_weight_explain, span_weight_matches, span_weight_scorer_supplier,
    Postings, SpanWeight, SpanWeightBase,
};
use crate::spans::spans::{spans_to_string, Spans};

/// A builder for span near queries.
///
/// Equivalent to `org.apache.lucene.queries.spans.SpanNearQuery.Builder`.
#[derive(Debug, Clone)]
pub struct Builder {
    ordered: bool,
    field: String,
    clauses: Vec<Arc<dyn SpanQuery>>,
    slop: i32,
}

impl Builder {
    /// Constructs a new builder.
    ///
    /// Equivalent to `Builder(String, boolean)`.
    ///
    /// * `field` — the field to search in;
    /// * `ordered` — whether or not clauses must be in-order to match.
    pub fn new(field: impl Into<String>, ordered: bool) -> Self {
        Self {
            ordered,
            field: field.into(),
            clauses: Vec::new(),
            slop: 0,
        }
    }

    /// Adds a new clause.
    ///
    /// Equivalent to `Builder.addClause(SpanQuery)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`](rucene::error::LuceneErrorKind::IllegalArgument)
    /// when the clause is on another field.
    pub fn add_clause(&mut self, clause: Arc<dyn SpanQuery>) -> Result<&mut Self> {
        if clause.get_field() != Some(self.field.as_str()) {
            return Err(LuceneError::illegal_argument(format!(
                "Cannot add clause {} to SpanNearQuery for field {}",
                clause.to_query_string(""),
                self.field
            )));
        }
        self.clauses.push(clause);
        Ok(self)
    }

    /// Adds a gap after the previous clause of a defined width.
    ///
    /// Equivalent to `Builder.addGap(int)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`](rucene::error::LuceneErrorKind::IllegalArgument)
    /// when the builder is unordered.
    pub fn add_gap(&mut self, width: i32) -> Result<&mut Self> {
        if !self.ordered {
            return Err(LuceneError::illegal_argument(
                "Gaps can only be added to ordered near queries".to_string(),
            ));
        }
        self.clauses
            .push(Arc::new(SpanGapQuery::new(self.field.clone(), width)));
        Ok(self)
    }

    /// Sets the slop for this query.
    ///
    /// Equivalent to `Builder.setSlop(int)`.
    pub fn set_slop(&mut self, slop: i32) -> &mut Self {
        self.slop = slop;
        self
    }

    /// Builds the query.
    ///
    /// Equivalent to `Builder.build()`.
    ///
    /// # Errors
    ///
    /// The constructor's field check, which the clauses the builder accepted
    /// always pass.
    pub fn build(&self) -> Result<SpanNearQuery> {
        SpanNearQuery::new(self.clauses.clone(), self.slop, self.ordered)
    }
}

/// Matches spans which are near one another.
///
/// Equivalent to `org.apache.lucene.queries.spans.SpanNearQuery`. One can
/// specify *slop*, the maximum number of intervening unmatched positions, as
/// well as whether matches are required to be in-order.
#[derive(Debug, Clone)]
pub struct SpanNearQuery {
    /// The protected `clauses` field.
    clauses: Vec<Arc<dyn SpanQuery>>,
    /// The protected `slop` field.
    slop: i32,
    /// The protected `inOrder` field.
    in_order: bool,
    /// The protected `field` field.
    field: Option<String>,
}

impl SpanNearQuery {
    /// Returns a [`Builder`] for an ordered query on a particular field.
    ///
    /// Equivalent to `SpanNearQuery.newOrderedNearQuery(String)`.
    pub fn new_ordered_near_query(field: impl Into<String>) -> Builder {
        Builder::new(field, true)
    }

    /// Returns a [`Builder`] for an unordered query on a particular field.
    ///
    /// Equivalent to `SpanNearQuery.newUnorderedNearQuery(String)`.
    pub fn new_unordered_near_query(field: impl Into<String>) -> Builder {
        Builder::new(field, false)
    }

    /// Constructs a span near query. Matches spans matching a span from each
    /// clause, with up to `slop` total unmatched positions between them.
    ///
    /// Equivalent to `SpanNearQuery(SpanQuery[], int, boolean)`. When
    /// `in_order` is `true`, the spans from each clause must be in the same
    /// order as in `clauses_in` and must be non-overlapping; when it is
    /// `false`, the spans from each clause need not be ordered and may
    /// overlap.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`](rucene::error::LuceneErrorKind::IllegalArgument)
    /// when the clauses have different fields.
    pub fn new(clauses_in: Vec<Arc<dyn SpanQuery>>, slop: i32, in_order: bool) -> Result<Self> {
        let mut clauses = Vec::with_capacity(clauses_in.len());
        let mut field: Option<String> = None;
        for clause in clauses_in {
            match field.as_deref() {
                None => field = clause.get_field().map(str::to_string), // check field
                Some(f) => {
                    if let Some(clause_field) = clause.get_field() {
                        if clause_field != f {
                            return Err(LuceneError::illegal_argument(
                                "Clauses must have same field.".to_string(),
                            ));
                        }
                    }
                }
            }
            clauses.push(clause);
        }
        Ok(Self {
            clauses,
            slop,
            in_order,
            field,
        })
    }

    /// Returns the clauses whose spans are matched.
    ///
    /// Equivalent to `SpanNearQuery.getClauses()`, which returns a copy.
    pub fn get_clauses(&self) -> Vec<Arc<dyn SpanQuery>> {
        self.clauses.clone()
    }

    /// Returns the maximum number of intervening unmatched positions
    /// permitted.
    ///
    /// Equivalent to `SpanNearQuery.getSlop()`.
    pub fn get_slop(&self) -> i32 {
        self.slop
    }

    /// Returns `true` if matches are required to be in-order.
    ///
    /// Equivalent to `SpanNearQuery.isInOrder()`.
    pub fn is_in_order(&self) -> bool {
        self.in_order
    }

    /// Equivalent to the private `SpanNearQuery.equalsTo(SpanNearQuery)`.
    fn equals_to(&self, other: &SpanNearQuery) -> bool {
        self.in_order == other.in_order
            && self.slop == other.slop
            && span_query_list_eq(&self.clauses, &other.clauses)
    }
}

impl Query for SpanNearQuery {
    fn to_query_string(&self, field: &str) -> String {
        let mut buffer = String::new();
        buffer.push_str("spanNear([");
        let mut i = self.clauses.iter().peekable();
        while let Some(clause) = i.next() {
            buffer.push_str(&clause.to_query_string(field));
            if i.peek().is_some() {
                buffer.push_str(", ");
            }
        }
        buffer.push_str("], ");
        buffer.push_str(&self.slop.to_string());
        buffer.push_str(", ");
        buffer.push_str(if self.in_order { "true" } else { "false" });
        buffer.push(')');
        buffer
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        if !accept_field(visitor, self.get_field()) {
            return;
        }
        let mut v = visitor.get_sub_visitor(Occur::MUST, self);
        for clause in &self.clauses {
            clause.visit(&mut *v);
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        Ok(self.create_span_weight(searcher, score_mode, boost)?)
    }

    fn rewrite(&self, searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        Ok(self.rewrite_span(searcher)?.map(|q| q as Arc<dyn Query>))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
            && other
                .as_any()
                .downcast_ref::<SpanNearQuery>()
                .is_some_and(|other| self.equals_to(other))
    }

    fn query_hash(&self) -> i32 {
        let mut result = self.class_hash();
        result ^= span_query_list_hash(&self.clauses);
        result = result.wrapping_add(self.slop);
        let fac: i32 = 1 + if self.in_order { 8 } else { 4 };
        fac.wrapping_mul(result)
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.queries.spans.SpanNearQuery"
    }
}

impl SpanQuery for SpanNearQuery {
    fn get_field(&self) -> Option<&str> {
        self.field.as_deref()
    }

    fn create_span_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn SpanWeight>> {
        let mut sub_weights: Vec<Arc<dyn SpanWeight>> = Vec::new();
        for q in &self.clauses {
            sub_weights.push(q.create_span_weight(searcher, score_mode, boost)?);
        }
        let terms = if score_mode.needs_scores() {
            Some(get_term_states_of(&sub_weights))
        } else {
            None
        };
        Ok(SpanNearWeight::new(
            Arc::new(self.clone()),
            sub_weights,
            searcher,
            terms.as_ref(),
            boost,
        )?)
    }

    fn rewrite_span(&self, searcher: &IndexSearcher) -> Result<Option<Arc<dyn SpanQuery>>> {
        let mut actually_rewritten = false;
        let mut rewritten_clauses: Vec<Arc<dyn SpanQuery>> = Vec::new();
        for c in &self.clauses {
            let query = match c.rewrite_span(searcher)? {
                Some(query) => {
                    actually_rewritten = true;
                    query
                }
                None => Arc::clone(c),
            };
            rewritten_clauses.push(query);
        }
        if actually_rewritten {
            // `(SpanNearQuery) clone()` with the rewritten clauses.
            let mut rewritten = self.clone();
            rewritten.clauses = rewritten_clauses;
            return Ok(Some(Arc::new(rewritten)));
        }
        Ok(None)
    }
}

/// Creates [`SpanNearQuery`] scorer instances.
///
/// Equivalent to the inner class
/// `org.apache.lucene.queries.spans.SpanNearQuery.SpanNearWeight`.
pub struct SpanNearWeight {
    base: SpanWeightBase,
    query: SpanNearQuery,
    /// The package-private final `subWeights` field.
    sub_weights: Vec<Arc<dyn SpanWeight>>,
}

impl std::fmt::Debug for SpanNearWeight {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpanNearWeight")
            .field("query", &self.query.to_query_string(""))
            .finish_non_exhaustive()
    }
}

impl SpanNearWeight {
    /// Equivalent to
    /// `SpanNearWeight(List<SpanWeight>, IndexSearcher, Map<Term, TermStates>, float)`;
    /// `query` is the enclosing query.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the statistics.
    pub fn new(
        query: Arc<SpanNearQuery>,
        sub_weights: Vec<Arc<dyn SpanWeight>>,
        searcher: &IndexSearcher,
        terms: Option<&TermStatesMap>,
        boost: f32,
    ) -> Result<Arc<Self>> {
        let query_copy = (*query).clone();
        let base = SpanWeightBase::new(query, searcher, terms, boost)?;
        Ok(new_span_weight(move |this: Weak<dyn SpanWeight>| Self {
            base: base.bind(this),
            query: query_copy,
            sub_weights,
        }))
    }
}

impl SegmentCacheable for SpanNearWeight {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        for w in &self.sub_weights {
            if !w.is_cacheable(ctx) {
                return false;
            }
        }
        true
    }
}

impl Weight for SpanNearWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        self.base.get_query()
    }

    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation> {
        span_weight_explain(self, context, doc)
    }

    /// Equivalent to `SpanNearWeight.scorerSupplier(LeafReaderContext)`, which
    /// wraps the [`SpanScorer`](crate::spans::span_scorer::SpanScorer) in a
    /// `DefaultScorerSupplier`.
    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        span_weight_scorer_supplier(self, context)
    }

    fn matches(&self, context: &LeafReaderContext, doc: i32) -> Result<Option<Arc<dyn Matches>>> {
        span_weight_matches(self, context, doc)
    }
}

impl SpanWeight for SpanNearWeight {
    fn span_weight_base(&self) -> &SpanWeightBase {
        &self.base
    }

    fn extract_term_states(&self, contexts: &mut TermStatesMap) {
        for w in &self.sub_weights {
            w.extract_term_states(contexts);
        }
    }

    fn get_spans(
        &self,
        context: &LeafReaderContext,
        required_postings: Postings,
    ) -> Result<Option<Box<dyn Spans>>> {
        // `context.reader().terms(field)` on a `null` field fails on the
        // codec reader's field map, a `TreeMap` that rejects `null` keys
        // (`PerFieldPostingsFormat.java:272,341`).
        let field = self
            .query
            .field
            .as_deref()
            .ok_or_else(LuceneError::null_pointer)?;
        if context.leaf_reader().terms(field)?.is_none() {
            return Ok(None); // field does not exist
        }

        let mut sub_spans: Vec<Box<dyn Spans>> = Vec::with_capacity(self.query.clauses.len());
        for w in &self.sub_weights {
            match w.get_spans(context, required_postings)? {
                Some(sub_span) => sub_spans.push(sub_span),
                None => return Ok(None), // all required
            }
        }

        // all NearSpans require at least two subSpans
        if !self.query.in_order {
            Ok(Some(Box::new(NearSpansUnordered::new(
                self.query.slop,
                sub_spans,
            )?)))
        } else {
            Ok(Some(Box::new(NearSpansOrdered::new(
                self.query.slop,
                sub_spans,
            )?)))
        }
    }
}

/// A clause of an ordered [`SpanNearQuery`] that matches a gap of a defined
/// width.
///
/// Equivalent to the private static nested class
/// `org.apache.lucene.queries.spans.SpanNearQuery.SpanGapQuery`.
#[derive(Debug, Clone)]
pub(crate) struct SpanGapQuery {
    field: String,
    width: i32,
}

impl SpanGapQuery {
    /// Equivalent to `SpanGapQuery(String, int)`.
    pub(crate) fn new(field: String, width: i32) -> Self {
        Self { field, width }
    }

    /// Equivalent to the private `SpanGapQuery.equalsTo(SpanGapQuery)`.
    fn equals_to(&self, other: &SpanGapQuery) -> bool {
        self.width == other.width && self.field == other.field
    }
}

impl Query for SpanGapQuery {
    /// Equivalent to `SpanGapQuery.toString(String)`, which renders its
    /// argument — not its own field — before the width.
    fn to_query_string(&self, field: &str) -> String {
        format!("SpanGap({field}:{})", self.width)
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
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        Ok(self.create_span_weight(searcher, score_mode, boost)?)
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
            && other
                .as_any()
                .downcast_ref::<SpanGapQuery>()
                .is_some_and(|other| self.equals_to(other))
    }

    fn query_hash(&self) -> i32 {
        let mut result = self.class_hash();
        result = result.wrapping_sub(7i32.wrapping_mul(self.width));
        result
            .wrapping_mul(15)
            .wrapping_sub(java_string_hash_code(&self.field))
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.queries.spans.SpanNearQuery$SpanGapQuery"
    }
}

impl SpanQuery for SpanGapQuery {
    fn get_field(&self) -> Option<&str> {
        Some(&self.field)
    }

    fn create_span_weight(
        &self,
        searcher: &IndexSearcher,
        _score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn SpanWeight>> {
        Ok(SpanGapWeight::new(Arc::new(self.clone()), searcher, boost)?)
    }
}

/// The weight of a [`SpanGapQuery`].
///
/// Equivalent to the private inner class
/// `org.apache.lucene.queries.spans.SpanNearQuery.SpanGapQuery.SpanGapWeight`.
pub(crate) struct SpanGapWeight {
    base: SpanWeightBase,
    width: i32,
}

impl std::fmt::Debug for SpanGapWeight {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpanGapWeight")
            .field("width", &self.width)
            .finish_non_exhaustive()
    }
}

impl SpanGapWeight {
    /// Equivalent to `SpanGapWeight(IndexSearcher, float)`, which passes no
    /// term states.
    fn new(query: Arc<SpanGapQuery>, searcher: &IndexSearcher, boost: f32) -> Result<Arc<Self>> {
        let width = query.width;
        let base = SpanWeightBase::new(query, searcher, None, boost)?;
        Ok(new_span_weight(move |this: Weak<dyn SpanWeight>| Self {
            base: base.bind(this),
            width,
        }))
    }
}

impl SegmentCacheable for SpanGapWeight {
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        true
    }
}

impl Weight for SpanGapWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        self.base.get_query()
    }

    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation> {
        span_weight_explain(self, context, doc)
    }

    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        span_weight_scorer_supplier(self, context)
    }

    fn matches(&self, context: &LeafReaderContext, doc: i32) -> Result<Option<Arc<dyn Matches>>> {
        span_weight_matches(self, context, doc)
    }
}

impl SpanWeight for SpanGapWeight {
    fn span_weight_base(&self) -> &SpanWeightBase {
        &self.base
    }

    fn extract_term_states(&self, _contexts: &mut TermStatesMap) {}

    fn get_spans(
        &self,
        _ctx: &LeafReaderContext,
        _required_postings: Postings,
    ) -> Result<Option<Box<dyn Spans>>> {
        Ok(Some(Box::new(GapSpans::new(self.width))))
    }
}

/// The spans of a [`SpanGapQuery`]: every document, every position, `width`
/// positions wide.
///
/// Equivalent to the package-private static nested class
/// `org.apache.lucene.queries.spans.SpanNearQuery.GapSpans`.
#[derive(Debug)]
pub(crate) struct GapSpans {
    doc: i32,
    pos: i32,
    width: i32,
}

impl GapSpans {
    /// Equivalent to `GapSpans(int)`.
    pub(crate) fn new(width: i32) -> Self {
        Self {
            doc: -1,
            pos: -1,
            width,
        }
    }

    /// Positions these spans at `position`.
    ///
    /// Equivalent to `GapSpans.skipToPosition(int)`.
    ///
    /// # Errors
    ///
    /// Never fails; Java declares `IOException`.
    pub(crate) fn skip_to_position(&mut self, position: i32) -> Result<i32> {
        self.pos = position;
        Ok(self.pos)
    }
}

impl DocIdSetIterator for GapSpans {
    /// `Spans.toString()`, which `GapSpans` does not override.
    fn java_to_string(&self) -> String {
        spans_to_string("GapSpans", self)
    }

    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.pos = -1;
        self.doc = self.doc.wrapping_add(1);
        Ok(self.doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.pos = -1;
        self.doc = target;
        Ok(self.doc)
    }

    fn cost(&self) -> i64 {
        0
    }

    /// The `spans instanceof SpanNearQuery.GapSpans` test of
    /// `NearSpansOrdered.advancePosition` reaches this type here.
    fn disi_as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }

    fn disi_as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
}

impl Spans for GapSpans {
    fn next_start_position(&mut self) -> Result<i32> {
        self.pos = self.pos.wrapping_add(1);
        Ok(self.pos)
    }

    fn start_position(&self) -> i32 {
        self.pos
    }

    fn end_position(&self) -> i32 {
        self.pos.wrapping_add(self.width)
    }

    fn width(&self) -> i32 {
        self.width
    }

    fn collect(&mut self, _collector: &mut dyn SpanCollector) -> Result<()> {
        Ok(())
    }

    fn positions_cost(&self) -> f32 {
        0.0
    }
}
