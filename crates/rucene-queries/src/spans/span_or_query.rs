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

//! Port of `SpanOrQuery.java`, of the Java package
//! `org.apache.lucene.queries.spans`.

use std::any::Any;
use std::cell::Cell;
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
use rucene::search::two_phase_iterator::TwoPhaseIterator;
use rucene::search::weight::Weight;

use crate::spans::span_collector::SpanCollector;
use crate::spans::span_disi_priority_queue::SpanDisiPriorityQueue;
use crate::spans::span_disi_wrapper::SpanDisiWrapper;
use crate::spans::span_disjunction_disi_approximation::SpanDisjunctionDISIApproximation;
use crate::spans::span_position_queue::SpanPositionQueue;
use crate::spans::span_query::{
    accept_field, get_term_states_of, span_query_list_eq, span_query_list_hash, SpanQuery,
    TermStatesMap,
};
use crate::spans::span_weight::{
    new_span_weight, span_weight_explain, span_weight_matches, span_weight_scorer_supplier,
    Postings, SpanWeight, SpanWeightBase,
};
use crate::spans::spans::{SharedSpans, Spans, NO_MORE_POSITIONS};

/// Message for the `NullPointerException` Java raises on an empty queue.
const QUEUE_TOP: &str = "INVARIANT: a span disjunction holds at least two sub-spans";

/// Matches the union of its clauses.
///
/// Equivalent to the final `org.apache.lucene.queries.spans.SpanOrQuery`.
#[derive(Debug, Clone)]
pub struct SpanOrQuery {
    clauses: Vec<Arc<dyn SpanQuery>>,
    field: Option<String>,
}

impl SpanOrQuery {
    /// Constructs a query merging the provided clauses. All clauses must have
    /// the same field.
    ///
    /// Equivalent to `SpanOrQuery(SpanQuery...)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`](rucene::error::LuceneErrorKind::IllegalArgument)
    /// when the clauses have different fields.
    pub fn new(clauses: Vec<Arc<dyn SpanQuery>>) -> Result<Self> {
        let mut query = Self {
            clauses: Vec::with_capacity(clauses.len()),
            field: None,
        };
        for seq in clauses {
            query.add_clause(seq)?;
        }
        Ok(query)
    }

    /// Adds a clause to this query.
    ///
    /// Equivalent to the private final `SpanOrQuery.addClause(SpanQuery)`.
    fn add_clause(&mut self, clause: Arc<dyn SpanQuery>) -> Result<()> {
        match self.field.as_deref() {
            None => self.field = clause.get_field().map(str::to_string),
            Some(field) => {
                if let Some(clause_field) = clause.get_field() {
                    if clause_field != field {
                        return Err(LuceneError::illegal_argument(
                            "Clauses must have same field.".to_string(),
                        ));
                    }
                }
            }
        }
        self.clauses.push(clause);
        Ok(())
    }

    /// Returns the clauses whose spans are matched.
    ///
    /// Equivalent to `SpanOrQuery.getClauses()`, which returns a copy.
    pub fn get_clauses(&self) -> Vec<Arc<dyn SpanQuery>> {
        self.clauses.clone()
    }
}

impl Query for SpanOrQuery {
    fn to_query_string(&self, field: &str) -> String {
        let mut buffer = String::new();
        buffer.push_str("spanOr([");
        let mut i = self.clauses.iter().peekable();
        while let Some(clause) = i.next() {
            buffer.push_str(&clause.to_query_string(field));
            if i.peek().is_some() {
                buffer.push_str(", ");
            }
        }
        buffer.push_str("])");
        buffer
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        if !accept_field(visitor, self.get_field()) {
            return;
        }
        let mut v = visitor.get_sub_visitor(Occur::SHOULD, self);
        for q in &self.clauses {
            q.visit(&mut *v);
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
                .downcast_ref::<SpanOrQuery>()
                .is_some_and(|other| span_query_list_eq(&self.clauses, &other.clauses))
    }

    /// `classHash() ^ clauses.hashCode()`.
    fn query_hash(&self) -> i32 {
        self.class_hash() ^ span_query_list_hash(&self.clauses)
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.queries.spans.SpanOrQuery"
    }
}

impl SpanQuery for SpanOrQuery {
    fn get_field(&self) -> Option<&str> {
        self.field.as_deref()
    }

    fn create_span_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn SpanWeight>> {
        let mut sub_weights: Vec<Arc<dyn SpanWeight>> = Vec::with_capacity(self.clauses.len());
        for q in &self.clauses {
            sub_weights.push(q.create_span_weight(searcher, score_mode, boost)?);
        }
        let terms = if score_mode.needs_scores() {
            Some(get_term_states_of(&sub_weights))
        } else {
            None
        };
        Ok(SpanOrWeight::new(
            Arc::new(self.clone()),
            searcher,
            terms.as_ref(),
            sub_weights,
            boost,
        )?)
    }

    fn rewrite_span(&self, searcher: &IndexSearcher) -> Result<Option<Arc<dyn SpanQuery>>> {
        let mut rewritten = SpanOrQuery {
            clauses: Vec::with_capacity(self.clauses.len()),
            field: None,
        };
        let mut actually_rewritten = false;
        for c in &self.clauses {
            let query = match c.rewrite_span(searcher)? {
                Some(query) => {
                    actually_rewritten = true;
                    query
                }
                None => Arc::clone(c),
            };
            rewritten.add_clause(query)?;
        }
        if actually_rewritten {
            return Ok(Some(Arc::new(rewritten)));
        }
        Ok(None)
    }
}

/// Creates [`SpanOrQuery`] scorer instances.
///
/// Equivalent to the inner class
/// `org.apache.lucene.queries.spans.SpanOrQuery.SpanOrWeight`.
pub struct SpanOrWeight {
    base: SpanWeightBase,
    query: SpanOrQuery,
    /// The package-private final `subWeights` field.
    sub_weights: Vec<Arc<dyn SpanWeight>>,
}

impl std::fmt::Debug for SpanOrWeight {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpanOrWeight")
            .field("query", &self.query.to_query_string(""))
            .finish_non_exhaustive()
    }
}

impl SpanOrWeight {
    /// Equivalent to
    /// `SpanOrWeight(IndexSearcher, Map<Term, TermStates>, List<SpanWeight>, float)`;
    /// `query` is the enclosing query.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the statistics.
    pub fn new(
        query: Arc<SpanOrQuery>,
        searcher: &IndexSearcher,
        terms: Option<&TermStatesMap>,
        sub_weights: Vec<Arc<dyn SpanWeight>>,
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

impl SegmentCacheable for SpanOrWeight {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        for w in &self.sub_weights {
            if !w.is_cacheable(ctx) {
                return false;
            }
        }
        true
    }
}

impl Weight for SpanOrWeight {
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

impl SpanWeight for SpanOrWeight {
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
        let mut sub_spans: Vec<Box<dyn Spans>> = Vec::with_capacity(self.query.clauses.len());

        for w in &self.sub_weights {
            if let Some(spans) = w.get_spans(context, required_postings)? {
                sub_spans.push(spans);
            }
        }

        if sub_spans.is_empty() {
            return Ok(None);
        } else if sub_spans.len() == 1 {
            return Ok(sub_spans.pop());
        }

        let sub_spans: Vec<SharedSpans> = sub_spans.into_iter().map(SharedSpans::new).collect();
        let mut by_doc_queue = SpanDisiPriorityQueue::new(sub_spans.len());
        for spans in &sub_spans {
            by_doc_queue.add(SpanDisiWrapper::new(spans.clone()));
        }

        let by_position_queue = SpanPositionQueue::new(sub_spans.len())?; // when empty use -1

        Ok(Some(Box::new(SpanOrSpans {
            query: self.query.clone(),
            sub_spans,
            approximation: SpanDisjunctionDISIApproximation::new(by_doc_queue),
            by_position_queue,
            top_position_spans: None,
            two_phase_match_cost: Cell::new(None),
            positions_cost: Cell::new(-1.0),
            last_doc_two_phase_matched: -1,
            cost: Cell::new(-1),
        })))
    }
}

/// The spans of [`SpanOrWeight`].
///
/// Equivalent to the anonymous `Spans` that `SpanOrWeight.getSpans` returns.
/// Its `byDocQueue` is the queue of the
/// [`SpanDisjunctionDISIApproximation`] it owns, which Java builds over the
/// same queue in `asTwoPhaseIterator()`; its construction only sums the costs
/// of the wrappers, so building it up front changes nothing.
struct SpanOrSpans {
    query: SpanOrQuery,
    /// The `subSpans` list the anonymous class captures.
    sub_spans: Vec<SharedSpans>,
    approximation: SpanDisjunctionDISIApproximation,
    by_position_queue: SpanPositionQueue,
    top_position_spans: Option<SharedSpans>,
    /// `Some(Some(matchCost))` once `asTwoPhaseIterator()` found sub-spans
    /// with an approximation, `Some(None)` once it found none. Java computes
    /// it on every call; it only reads construction-time costs, so every call
    /// answers the same.
    two_phase_match_cost: Cell<Option<Option<f32>>>,
    positions_cost: Cell<f32>,
    last_doc_two_phase_matched: i32,
    cost: Cell<i64>,
}

impl SpanOrSpans {
    /// The head of the anonymous class's `asTwoPhaseIterator()`: the match
    /// cost of its view, or `None` when no sub-spans supports approximations,
    /// in which case the positions cost is computed.
    fn two_phase_match_cost(&self) -> Option<f32> {
        if let Some(match_cost) = self.two_phase_match_cost.get() {
            return match_cost;
        }
        let mut sum_match_cost = 0f32; // See also DisjunctionScorer.asTwoPhaseIterator()
        let mut sum_approx_cost = 0i64;

        for w in self.approximation.sub_iterators.iter() {
            if let Some(two_phase_view) = w.two_phase_view.as_ref() {
                let cost_weight = if w.cost <= 1 { 1 } else { w.cost };
                sum_match_cost += two_phase_view.match_cost() * cost_weight as f32;
                sum_approx_cost += cost_weight;
            }
        }

        let match_cost = if sum_approx_cost == 0 {
            // no sub spans supports approximations
            self.compute_positions_cost();
            None
        } else {
            Some(sum_match_cost / sum_approx_cost as f32)
        };
        self.two_phase_match_cost.set(Some(match_cost));
        match_cost
    }

    /// Equivalent to the anonymous class's `computePositionsCost()`.
    fn compute_positions_cost(&self) {
        let mut sum_positions_cost = 0f32;
        let mut sum_cost = 0i64;
        for w in self.approximation.sub_iterators.iter() {
            let cost_weight = if w.cost <= 1 { 1 } else { w.cost };
            sum_positions_cost += w.spans.positions_cost() * cost_weight as f32;
            sum_cost += cost_weight;
        }
        self.positions_cost
            .set(sum_positions_cost / sum_cost as f32);
    }

    /// Equivalent to the anonymous class's `twoPhaseCurrentDocMatches()`.
    fn two_phase_current_doc_matches(&mut self) -> Result<bool> {
        let queue = &mut self.approximation.sub_iterators;
        let list_at_current_doc = queue.top_list();
        // remove the head of the list as long as it does not match
        let current_doc = queue.wrapper(list_at_current_doc[0]).doc;
        let mut list = list_at_current_doc.into_iter();
        let mut head = list.next();
        while let Some(position) = head {
            let w = queue.wrapper_mut(position);
            let Some(two_phase_view) = w.two_phase_view.as_mut() else {
                break;
            };
            if two_phase_view.matches()? {
                // use this spans for positions at current doc:
                w.last_approx_match_doc = current_doc;
                break;
            }
            // do not use this spans for positions at current doc:
            w.last_approx_non_match_doc = current_doc;
            head = list.next();
            if head.is_none() {
                return Ok(false);
            }
        }
        self.last_doc_two_phase_matched = current_doc;
        self.top_position_spans = None;
        Ok(true)
    }

    /// Called at first `nextStartPosition`.
    ///
    /// Equivalent to the anonymous class's `fillPositionQueue()`.
    fn fill_position_queue(&mut self) -> Result<()> {
        debug_assert!(self.by_position_queue.size() == 0);
        // add all matching Spans at current doc to byPositionQueue
        let queue = &mut self.approximation.sub_iterators;
        for position in queue.top_list() {
            let w = queue.wrapper_mut(position);
            let mut spans_at_doc = Some(w.spans.clone());
            if self.last_doc_two_phase_matched == w.doc {
                // matched by DisjunctionDisiApproximation
                if let Some(two_phase_view) = w.two_phase_view.as_mut() {
                    // matched by approximation
                    if w.last_approx_non_match_doc == w.doc {
                        // matches() returned false
                        spans_at_doc = None;
                    } else if w.last_approx_match_doc != w.doc && !two_phase_view.matches()? {
                        spans_at_doc = None;
                    }
                }
            }

            if let Some(spans_at_doc) = spans_at_doc {
                debug_assert!(spans_at_doc.doc_id() == w.doc);
                debug_assert!(spans_at_doc.start_position() == -1);
                spans_at_doc.next_start_position()?;
                debug_assert!(spans_at_doc.start_position() != NO_MORE_POSITIONS);
                self.by_position_queue.add(spans_at_doc);
            }
        }
        debug_assert!(self.by_position_queue.size() > 0);
        Ok(())
    }
}

impl DocIdSetIterator for SpanOrSpans {
    /// Equivalent to the anonymous class's `toString()`.
    fn java_to_string(&self) -> String {
        format!(
            "spanOr({})@{}: {} - {}",
            self.query.to_query_string(""),
            self.doc_id(),
            self.start_position(),
            self.end_position()
        )
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.top_position_spans = None;
        let queue = &mut self.approximation.sub_iterators;
        let mut top_doc_spans = queue.top().expect(QUEUE_TOP);
        let current_doc = queue.wrapper(top_doc_spans).doc;
        loop {
            let w = queue.wrapper_mut(top_doc_spans);
            w.doc = w.iterator.next_doc()?;
            top_doc_spans = queue.update_top().expect(QUEUE_TOP);
            if queue.wrapper(top_doc_spans).doc != current_doc {
                break;
            }
        }
        Ok(queue.wrapper(top_doc_spans).doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.top_position_spans = None;
        let queue = &mut self.approximation.sub_iterators;
        let mut top_doc_spans = queue.top().expect(QUEUE_TOP);
        loop {
            let w = queue.wrapper_mut(top_doc_spans);
            w.doc = w.iterator.advance(target)?;
            top_doc_spans = queue.update_top().expect(QUEUE_TOP);
            if queue.wrapper(top_doc_spans).doc >= target {
                break;
            }
        }
        Ok(queue.wrapper(top_doc_spans).doc)
    }

    fn doc_id(&self) -> i32 {
        let queue = &self.approximation.sub_iterators;
        queue.wrapper(queue.top().expect(QUEUE_TOP)).doc
    }

    fn cost(&self) -> i64 {
        if self.cost.get() == -1 {
            let mut cost = 0i64;
            for spans in &self.sub_spans {
                cost = cost.wrapping_add(spans.cost());
            }
            self.cost.set(cost);
        }
        self.cost.get()
    }
}

impl Spans for SpanOrSpans {
    fn next_start_position(&mut self) -> Result<i32> {
        match self.top_position_spans.clone() {
            None => {
                self.by_position_queue.clear();
                self.fill_position_queue()?; // fills byPositionQueue at first position
                self.top_position_spans = self.by_position_queue.top().cloned();
            }
            Some(top_position_spans) => {
                top_position_spans.next_start_position()?;
                self.top_position_spans = self.by_position_queue.update_top().cloned();
            }
        }
        Ok(self
            .top_position_spans
            .as_ref()
            .ok_or_else(LuceneError::null_pointer)?
            .start_position())
    }

    fn start_position(&self) -> i32 {
        match self.top_position_spans.as_ref() {
            None => -1,
            Some(spans) => spans.start_position(),
        }
    }

    fn end_position(&self) -> i32 {
        match self.top_position_spans.as_ref() {
            None => -1,
            Some(spans) => spans.end_position(),
        }
    }

    fn width(&self) -> i32 {
        self.top_position_spans
            .as_ref()
            .expect("INVARIANT: width is only asked of positioned spans")
            .width()
    }

    fn collect(&mut self, collector: &mut dyn SpanCollector) -> Result<()> {
        if let Some(top_position_spans) = self.top_position_spans.as_ref() {
            top_position_spans.collect(collector)?;
        }
        Ok(())
    }

    fn positions_cost(&self) -> f32 {
        // This may be called when asTwoPhaseIterator returned null,
        // which happens when none of the sub spans supports approximations.
        debug_assert!(self.positions_cost.get() > 0.0);
        self.positions_cost.get()
    }

    fn as_two_phase_iterator(&mut self) -> Option<&mut dyn TwoPhaseIterator> {
        match self.two_phase_match_cost() {
            Some(_) => Some(self),
            None => None,
        }
    }

    fn as_two_phase_iterator_ref(&self) -> Option<&dyn TwoPhaseIterator> {
        match self.two_phase_match_cost() {
            Some(_) => Some(self),
            None => None,
        }
    }
}

/// The anonymous `TwoPhaseIterator` of the anonymous class's
/// `asTwoPhaseIterator()`, whose approximation is a
/// [`SpanDisjunctionDISIApproximation`] over the `byDocQueue`.
impl TwoPhaseIterator for SpanOrSpans {
    fn approximation(&mut self) -> &mut dyn DocIdSetIterator {
        &mut self.approximation
    }

    fn approximation_ref(&self) -> &dyn DocIdSetIterator {
        &self.approximation
    }

    fn matches(&mut self) -> Result<bool> {
        self.two_phase_current_doc_matches()
    }

    fn match_cost(&self) -> f32 {
        self.two_phase_match_cost()
            .expect("INVARIANT: the view exists only when sub-spans have approximations")
    }
}
