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

//! Port of `SpanContainingQuery.java`, of the Java package
//! `org.apache.lucene.queries.spans`.

use std::any::Any;
use std::sync::{Arc, Weak};

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::doc_id_set_iterator::DocIdSetIterator;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::matches::Matches;
use rucene::search::query::Query;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::score_mode::ScoreMode;
use rucene::search::scorer_supplier::ScorerSupplier;
use rucene::search::segment_cacheable::SegmentCacheable;
use rucene::search::similarities::Explanation;
use rucene::search::two_phase_iterator::TwoPhaseIterator;
use rucene::search::weight::Weight;

use crate::spans::conjunction_spans::{
    conjunction_advance, conjunction_next_doc, ConjunctionSpans, ConjunctionSpansImpl,
};
use crate::spans::contain_spans::ContainSpans;
use crate::spans::span_collector::SpanCollector;
use crate::spans::span_contain_query::{SpanContainQuery, SpanContainWeight};
use crate::spans::span_query::{get_term_states, SpanQuery, TermStatesMap};
use crate::spans::span_weight::{
    new_span_weight, span_weight_explain, span_weight_matches, span_weight_scorer_supplier,
    Postings, SpanWeight, SpanWeightBase,
};
use crate::spans::spans::{positions_cost_unsupported, spans_to_string, Spans, NO_MORE_POSITIONS};

/// Keeps matches that contain another span query.
///
/// Equivalent to `org.apache.lucene.queries.spans.SpanContainingQuery`, which
/// extends `SpanContainQuery`.
#[derive(Debug, Clone)]
pub struct SpanContainingQuery {
    inner: SpanContainQuery,
}

impl SpanContainingQuery {
    /// Constructs a query matching spans from `big` that contain at least one
    /// spans from `little`. This query has the boost of `big`; `big` and
    /// `little` must be in the same field.
    ///
    /// Equivalent to `SpanContainingQuery(SpanQuery, SpanQuery)`.
    ///
    /// # Errors
    ///
    /// As the constructor of `SpanContainQuery`: a `null` field, or fields that
    /// differ.
    pub fn new(big: Arc<dyn SpanQuery>, little: Arc<dyn SpanQuery>) -> Result<Self> {
        Ok(Self {
            inner: SpanContainQuery::new(big, little)?,
        })
    }

    /// Equivalent to the inherited `SpanContainQuery.getBig()`.
    pub fn get_big(&self) -> &Arc<dyn SpanQuery> {
        self.inner.get_big()
    }

    /// Equivalent to the inherited `SpanContainQuery.getLittle()`.
    pub fn get_little(&self) -> &Arc<dyn SpanQuery> {
        self.inner.get_little()
    }
}

impl Query for SpanContainingQuery {
    fn to_query_string(&self, field: &str) -> String {
        self.inner.to_string_with_name(field, "SpanContaining")
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        self.inner.visit(self, visitor);
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
                .downcast_ref::<SpanContainingQuery>()
                .is_some_and(|other| self.inner.equals_to(&other.inner))
    }

    fn query_hash(&self) -> i32 {
        self.inner.hash_code(self.class_hash())
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.queries.spans.SpanContainingQuery"
    }
}

impl SpanQuery for SpanContainingQuery {
    fn get_field(&self) -> Option<&str> {
        self.inner.get_field()
    }

    fn create_span_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn SpanWeight>> {
        let big_weight = self
            .inner
            .big
            .create_span_weight(searcher, score_mode, boost)?;
        let little_weight = self
            .inner
            .little
            .create_span_weight(searcher, score_mode, boost)?;
        let terms = if score_mode.needs_scores() {
            Some(get_term_states(&[&*big_weight, &*little_weight]))
        } else {
            None
        };
        Ok(SpanContainingWeight::new(
            Arc::new(self.clone()),
            searcher,
            terms.as_ref(),
            big_weight,
            little_weight,
            boost,
        )?)
    }

    fn rewrite_span(&self, searcher: &IndexSearcher) -> Result<Option<Arc<dyn SpanQuery>>> {
        Ok(self
            .inner
            .rewrite(searcher)?
            .map(|inner| Arc::new(SpanContainingQuery { inner }) as Arc<dyn SpanQuery>))
    }
}

/// Creates [`SpanContainingQuery`] scorer instances.
///
/// Equivalent to the inner class
/// `org.apache.lucene.queries.spans.SpanContainingQuery.SpanContainingWeight`,
/// which extends `SpanContainWeight`.
#[derive(Debug)]
pub struct SpanContainingWeight {
    inner: SpanContainWeight,
}

impl SpanContainingWeight {
    /// Equivalent to
    /// `SpanContainingWeight(IndexSearcher, Map<Term, TermStates>, SpanWeight, SpanWeight, float)`;
    /// `query` is the enclosing query.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the statistics.
    pub fn new(
        query: Arc<SpanContainingQuery>,
        searcher: &IndexSearcher,
        terms: Option<&TermStatesMap>,
        big_weight: Arc<dyn SpanWeight>,
        little_weight: Arc<dyn SpanWeight>,
        boost: f32,
    ) -> Result<Arc<Self>> {
        let inner =
            SpanContainWeight::new(query, searcher, terms, big_weight, little_weight, boost)?;
        Ok(new_span_weight(move |this: Weak<dyn SpanWeight>| Self {
            inner: inner.bind(this),
        }))
    }
}

impl SegmentCacheable for SpanContainingWeight {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.inner.big_weight.is_cacheable(ctx) && self.inner.little_weight.is_cacheable(ctx)
    }
}

impl Weight for SpanContainingWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        self.inner.base.get_query()
    }

    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation> {
        span_weight_explain(self, context, doc)
    }

    /// Equivalent to `SpanContainingWeight.scorerSupplier(LeafReaderContext)`,
    /// which builds the same [`SpanScorer`](crate::spans::span_scorer::SpanScorer)
    /// as the inherited one.
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

impl SpanWeight for SpanContainingWeight {
    fn span_weight_base(&self) -> &SpanWeightBase {
        &self.inner.base
    }

    fn extract_term_states(&self, contexts: &mut TermStatesMap) {
        self.inner.extract_term_states(contexts);
    }

    /// Returns spans from `big` that contain at least one spans from `little`.
    /// The payload is from the spans of `big`.
    ///
    /// Equivalent to `SpanContainingWeight.getSpans(LeafReaderContext, Postings)`.
    fn get_spans(
        &self,
        context: &LeafReaderContext,
        required_postings: Postings,
    ) -> Result<Option<Box<dyn Spans>>> {
        let Some((big, little)) = self.inner.prepare_conjunction(context, required_postings)?
        else {
            return Ok(None);
        };
        Ok(Some(Box::new(ContainingSpans {
            inner: ContainSpans::new(big, little, true)?,
        })))
    }
}

/// The spans of [`SpanContainingWeight`].
///
/// Equivalent to the anonymous subclass of `ContainSpans` that
/// `SpanContainingWeight.getSpans` returns, built as
/// `new ContainSpans(big, little, big)`.
struct ContainingSpans {
    inner: ContainSpans,
}

impl ConjunctionSpansImpl for ContainingSpans {
    fn conjunction_spans(&self) -> &ConjunctionSpans {
        &self.inner.conj
    }

    fn conjunction_spans_mut(&mut self) -> &mut ConjunctionSpans {
        &mut self.inner.conj
    }

    fn two_phase_current_doc_matches(&mut self) -> Result<bool> {
        let big_spans = self.inner.big_spans.clone();
        let little_spans = self.inner.little_spans.clone();
        self.inner.conj.one_exhausted_in_current_doc = false;
        debug_assert!(little_spans.start_position() == -1);
        while big_spans.next_start_position()? != NO_MORE_POSITIONS {
            while little_spans.start_position() < big_spans.start_position() {
                if little_spans.next_start_position()? == NO_MORE_POSITIONS {
                    self.inner.conj.one_exhausted_in_current_doc = true;
                    return Ok(false);
                }
            }
            if big_spans.end_position() >= little_spans.end_position() {
                self.inner.conj.at_first_in_current_doc = true;
                return Ok(true);
            }
        }
        self.inner.conj.one_exhausted_in_current_doc = true;
        Ok(false)
    }
}

impl DocIdSetIterator for ContainingSpans {
    /// `Spans.toString()` of the anonymous class, named by `getName()`.
    fn java_to_string(&self) -> String {
        spans_to_string(
            "org.apache.lucene.queries.spans.SpanContainingQuery$SpanContainingWeight$1",
            self,
        )
    }

    fn doc_id(&self) -> i32 {
        self.inner.conj.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        conjunction_next_doc(self)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        conjunction_advance(self, target)
    }

    fn cost(&self) -> i64 {
        self.inner.conj.cost()
    }
}

impl Spans for ContainingSpans {
    fn next_start_position(&mut self) -> Result<i32> {
        let big_spans = self.inner.big_spans.clone();
        let little_spans = self.inner.little_spans.clone();
        if self.inner.conj.at_first_in_current_doc {
            self.inner.conj.at_first_in_current_doc = false;
            return Ok(big_spans.start_position());
        }
        while big_spans.next_start_position()? != NO_MORE_POSITIONS {
            while little_spans.start_position() < big_spans.start_position() {
                if little_spans.next_start_position()? == NO_MORE_POSITIONS {
                    self.inner.conj.one_exhausted_in_current_doc = true;
                    return Ok(NO_MORE_POSITIONS);
                }
            }
            if big_spans.end_position() >= little_spans.end_position() {
                return Ok(big_spans.start_position());
            }
        }
        self.inner.conj.one_exhausted_in_current_doc = true;
        Ok(NO_MORE_POSITIONS)
    }

    fn start_position(&self) -> i32 {
        self.inner.start_position()
    }

    fn end_position(&self) -> i32 {
        self.inner.end_position()
    }

    fn width(&self) -> i32 {
        self.inner.width()
    }

    fn collect(&mut self, collector: &mut dyn SpanCollector) -> Result<()> {
        self.inner.collect(collector)
    }

    fn positions_cost(&self) -> f32 {
        positions_cost_unsupported() // asTwoPhaseIterator never returns null here.
    }

    fn as_two_phase_iterator(&mut self) -> Option<&mut dyn TwoPhaseIterator> {
        Some(self)
    }

    fn as_two_phase_iterator_ref(&self) -> Option<&dyn TwoPhaseIterator> {
        Some(self)
    }
}

/// The anonymous `TwoPhaseIterator` of the inherited
/// `ConjunctionSpans.asTwoPhaseIterator()`.
impl TwoPhaseIterator for ContainingSpans {
    fn approximation(&mut self) -> &mut dyn DocIdSetIterator {
        &mut *self.inner.conj.conjunction
    }

    fn approximation_ref(&self) -> &dyn DocIdSetIterator {
        &*self.inner.conj.conjunction
    }

    fn matches(&mut self) -> Result<bool> {
        self.two_phase_current_doc_matches()
    }

    fn match_cost(&self) -> f32 {
        self.inner.conj.match_cost()
    }
}
