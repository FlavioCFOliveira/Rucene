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

//! Port of `PayloadScoreQuery.java`, of the Java package
//! `org.apache.lucene.queries.payloads`.

use std::any::Any;
use std::sync::{Arc, Weak};

use rucene::error::Result;
use rucene::index::{LeafReaderContext, NumericDocValues, PostingsEnum, Term};
use rucene::search::boolean_clause::Occur;
use rucene::search::doc_id_set_iterator::DocIdSetIterator;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::matches::Matches;
use rucene::search::query::Query;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::scorable::Scorable;
use rucene::search::score_mode::ScoreMode;
use rucene::search::scorer::Scorer;
use rucene::search::scorer_supplier::ScorerSupplier;
use rucene::search::segment_cacheable::SegmentCacheable;
use rucene::search::sim_scorer_source::SharedSimScorer;
use rucene::search::similarities::Explanation;
use rucene::search::two_phase_iterator::TwoPhaseIterator;
use rucene::search::weight::{DefaultScorerSupplier, Weight};
use rucene::util::BytesRef;

use crate::payloads::payload_decoder::PayloadDecoder;
use crate::payloads::payload_function::{java_simple_class_name, PayloadFunction};
use crate::spans::filter_spans::{AcceptStatus, FilterSpans, FilterSpansAccept};
use crate::spans::span_collector::SpanCollector;
use crate::spans::span_query::{SpanQuery, TermStatesMap};
use crate::spans::span_scorer::SpanScorer;
use crate::spans::span_weight::{
    get_norm_values, new_span_weight, span_weight_matches, Postings, SpanWeight, SpanWeightBase,
};
use crate::spans::spans::Spans;

/// A query that uses a [`PayloadFunction`] to modify the score of a wrapped
/// [`SpanQuery`].
///
/// Equivalent to `org.apache.lucene.queries.payloads.PayloadScoreQuery`. A
/// wrapped span query is used due to the way that payload values are indexed;
/// see [`POSTINGS_ENUM_PAYLOADS`](rucene::index::POSTINGS_ENUM_PAYLOADS).
#[derive(Debug, Clone)]
pub struct PayloadScoreQuery {
    wrapped_query: Arc<dyn SpanQuery>,
    function: Arc<dyn PayloadFunction>,
    decoder: Arc<dyn PayloadDecoder>,
    include_span_score: bool,
}

impl PayloadScoreQuery {
    /// Creates a new payload score query.
    ///
    /// Equivalent to
    /// `PayloadScoreQuery(SpanQuery, PayloadFunction, PayloadDecoder, boolean)`.
    ///
    /// * `wrapped_query` — the query to wrap;
    /// * `function` — a payload function to use to modify the scores;
    /// * `decoder` — a payload decoder to convert payloads into float values;
    /// * `include_span_score` — include both span score and payload score in
    ///   the scoring algorithm.
    pub fn new(
        wrapped_query: Arc<dyn SpanQuery>,
        function: Arc<dyn PayloadFunction>,
        decoder: Arc<dyn PayloadDecoder>,
        include_span_score: bool,
    ) -> Self {
        Self {
            wrapped_query,
            function,
            decoder,
            include_span_score,
        }
    }

    /// Creates a new payload score query that includes the underlying span
    /// scores.
    ///
    /// Equivalent to `PayloadScoreQuery(SpanQuery, PayloadFunction, PayloadDecoder)`.
    pub fn with_span_score(
        wrapped_query: Arc<dyn SpanQuery>,
        function: Arc<dyn PayloadFunction>,
        decoder: Arc<dyn PayloadDecoder>,
    ) -> Self {
        Self::new(wrapped_query, function, decoder, true)
    }

    /// Equivalent to the private `PayloadScoreQuery.equalsTo(PayloadScoreQuery)`.
    fn equals_to(&self, other: &PayloadScoreQuery) -> bool {
        self.wrapped_query
            .query_eq(&*other.wrapped_query as &dyn Query)
            && self.function.function_eq(&*other.function)
            && (self.include_span_score == other.include_span_score)
            && self.decoder.decoder_eq(&*other.decoder)
    }
}

impl Query for PayloadScoreQuery {
    fn to_query_string(&self, field: &str) -> String {
        let mut buffer = String::new();
        buffer.push_str("PayloadScoreQuery(");
        buffer.push_str(&self.wrapped_query.to_query_string(field));
        buffer.push_str(", function: ");
        buffer.push_str(java_simple_class_name(self.function.java_class_name()));
        buffer.push_str(", includeSpanScore: ");
        buffer.push_str(if self.include_span_score {
            "true"
        } else {
            "false"
        });
        buffer.push(')');
        buffer
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        self.wrapped_query
            .visit(&mut *visitor.get_sub_visitor(Occur::MUST, self));
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
                .downcast_ref::<PayloadScoreQuery>()
                .is_some_and(|other| self.equals_to(other))
    }

    /// `Objects.hash(wrappedQuery, function, decoder, includeSpanScore)`.
    fn query_hash(&self) -> i32 {
        let mut h = 1i32;
        h = h
            .wrapping_mul(31)
            .wrapping_add(self.wrapped_query.query_hash());
        h = h
            .wrapping_mul(31)
            .wrapping_add(self.function.function_hash());
        h = h.wrapping_mul(31).wrapping_add(self.decoder.decoder_hash());
        h.wrapping_mul(31)
            .wrapping_add(if self.include_span_score { 1231 } else { 1237 })
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.queries.payloads.PayloadScoreQuery"
    }
}

impl SpanQuery for PayloadScoreQuery {
    fn get_field(&self) -> Option<&str> {
        self.wrapped_query.get_field()
    }

    fn create_span_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn SpanWeight>> {
        let inner_weight = self
            .wrapped_query
            .create_span_weight(searcher, score_mode, boost)?;
        if !score_mode.needs_scores() {
            return Ok(inner_weight);
        }
        Ok(PayloadSpanWeight::new(
            Arc::new(self.clone()),
            searcher,
            inner_weight,
            boost,
        )?)
    }

    /// Equivalent to `PayloadScoreQuery.rewrite(IndexSearcher)`, whose
    /// `matchRewritten instanceof SpanQuery` test always holds for a rewritten
    /// span query.
    fn rewrite_span(&self, searcher: &IndexSearcher) -> Result<Option<Arc<dyn SpanQuery>>> {
        if let Some(match_rewritten) = self.wrapped_query.rewrite_span(searcher)? {
            return Ok(Some(Arc::new(PayloadScoreQuery::new(
                match_rewritten,
                Arc::clone(&self.function),
                Arc::clone(&self.decoder),
                self.include_span_score,
            ))));
        }
        Ok(None)
    }
}

/// The weight of a [`PayloadScoreQuery`] when scores are needed.
///
/// Equivalent to the private inner class
/// `org.apache.lucene.queries.payloads.PayloadScoreQuery.PayloadSpanWeight`.
pub(crate) struct PayloadSpanWeight {
    base: SpanWeightBase,
    inner_weight: Arc<dyn SpanWeight>,
    function: Arc<dyn PayloadFunction>,
    decoder: Arc<dyn PayloadDecoder>,
    include_span_score: bool,
}

impl std::fmt::Debug for PayloadSpanWeight {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PayloadSpanWeight")
            .field("function", &self.function)
            .field("include_span_score", &self.include_span_score)
            .finish_non_exhaustive()
    }
}

impl PayloadSpanWeight {
    /// Equivalent to `PayloadSpanWeight(IndexSearcher, SpanWeight, float)`,
    /// which passes no term states; `query` is the enclosing query.
    fn new(
        query: Arc<PayloadScoreQuery>,
        searcher: &IndexSearcher,
        inner_weight: Arc<dyn SpanWeight>,
        boost: f32,
    ) -> Result<Arc<Self>> {
        let function = Arc::clone(&query.function);
        let decoder = Arc::clone(&query.decoder);
        let include_span_score = query.include_span_score;
        let base = SpanWeightBase::new(query, searcher, None, boost)?;
        Ok(new_span_weight(move |this: Weak<dyn SpanWeight>| Self {
            base: base.bind(this),
            inner_weight,
            function,
            decoder,
            include_span_score,
        }))
    }

    /// Builds the [`PayloadSpanScorer`] of a leaf, or `None` when no document
    /// matches.
    ///
    /// Equivalent to the body of `PayloadSpanWeight.scorerSupplier(LeafReaderContext)`,
    /// keeping the concrete type for `explain`, which Java reaches with a cast.
    fn payload_span_scorer(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<PayloadSpanScorer>> {
        let Some(spans) = self.get_spans(context, Postings::Payloads)? else {
            return Ok(None);
        };
        let norms = get_norm_values(context, self.base.field())?;
        let field = self.base.field().map(str::to_string);
        let payload_spans = FilterSpans::new(
            spans,
            PayloadSpans::new(
                Arc::clone(&self.decoder),
                Arc::clone(&self.function),
                field.clone(),
            ),
        );
        Ok(Some(PayloadSpanScorer::new(
            payload_spans,
            self.inner_weight.get_sim_scorer(),
            norms,
            Arc::clone(&self.function),
            self.include_span_score,
            field,
        )))
    }
}

impl SegmentCacheable for PayloadSpanWeight {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.inner_weight.is_cacheable(ctx)
    }
}

impl Weight for PayloadSpanWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        self.base.get_query()
    }

    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation> {
        let scorer = self.payload_span_scorer(context)?;
        let Some(mut scorer) = scorer else {
            return Ok(Explanation::no_match("No match", Vec::new()));
        };
        if scorer.iterator().advance(doc)? != doc {
            return Ok(Explanation::no_match("No match", Vec::new()));
        }

        scorer.score()?; // force freq calculation
        let payload_expl = scorer.get_payload_explanation();

        if self.include_span_score {
            let inner_expl = self.inner_weight.explain(context, doc)?;
            return Ok(Explanation::matched(
                scorer.score_current_doc()?,
                "PayloadSpanQuery, product of:",
                vec![inner_expl, payload_expl],
            ));
        }

        Ok(scorer.get_payload_explanation())
    }

    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        Ok(self.payload_span_scorer(context)?.map(|scorer| {
            Box::new(DefaultScorerSupplier::new(Box::new(scorer))) as Box<dyn ScorerSupplier>
        }))
    }

    fn matches(&self, context: &LeafReaderContext, doc: i32) -> Result<Option<Arc<dyn Matches>>> {
        span_weight_matches(self, context, doc)
    }
}

impl SpanWeight for PayloadSpanWeight {
    fn span_weight_base(&self) -> &SpanWeightBase {
        &self.base
    }

    fn extract_term_states(&self, contexts: &mut TermStatesMap) {
        self.inner_weight.extract_term_states(contexts);
    }

    fn get_spans(
        &self,
        ctx: &LeafReaderContext,
        required_postings: Postings,
    ) -> Result<Option<Box<dyn Spans>>> {
        self.inner_weight
            .get_spans(ctx, required_postings.at_least(Postings::Payloads))
    }
}

/// The payload-collecting filter of the spans of a [`PayloadSpanWeight`].
///
/// Equivalent to the private inner class
/// `org.apache.lucene.queries.payloads.PayloadScoreQuery.PayloadSpans`, which
/// extends `FilterSpans` and implements `SpanCollector`: the spans are a
/// [`FilterSpans`] holding this filter.
///
/// Java's `collectLeaf` reads `docID()`, `in.startPosition()` and
/// `in.endPosition()` while `in.collect(this)` runs. The wrapped spans are
/// borrowed by that call here, so `doCurrentSpans` reads the three values just
/// before it — collecting does not move the spans, so they are the values Java
/// reads.
#[derive(Debug)]
pub(crate) struct PayloadSpans {
    decoder: Arc<dyn PayloadDecoder>,
    function: Arc<dyn PayloadFunction>,
    /// `PayloadScoreQuery.this.getField()`.
    field: Option<String>,
    /// The public `payloadsSeen` field.
    pub(crate) payloads_seen: i32,
    /// The public `payloadScore` field.
    pub(crate) payload_score: f32,
    in_doc_id: i32,
    in_start_position: i32,
    in_end_position: i32,
}

impl PayloadSpans {
    /// Equivalent to the private `PayloadSpans(Spans, PayloadDecoder)`; the
    /// wrapped spans are held by the [`FilterSpans`].
    fn new(
        decoder: Arc<dyn PayloadDecoder>,
        function: Arc<dyn PayloadFunction>,
        field: Option<String>,
    ) -> Self {
        Self {
            decoder,
            function,
            field,
            payloads_seen: 0,
            payload_score: 0.0,
            in_doc_id: -1,
            in_start_position: -1,
            in_end_position: -1,
        }
    }
}

impl FilterSpansAccept for PayloadSpans {
    fn accept(&mut self, _candidate: &mut dyn Spans) -> Result<AcceptStatus> {
        Ok(AcceptStatus::Yes)
    }

    fn do_start_current_doc(&mut self) -> Result<()> {
        self.payload_score = 0.0;
        self.payloads_seen = 0;
        Ok(())
    }

    fn do_current_spans(&mut self, in_spans: &mut dyn Spans) -> Result<()> {
        self.in_doc_id = in_spans.doc_id();
        self.in_start_position = in_spans.start_position();
        self.in_end_position = in_spans.end_position();
        in_spans.collect(self)
    }
}

impl SpanCollector for PayloadSpans {
    fn collect_leaf(
        &mut self,
        postings: &dyn PostingsEnum,
        _position: i32,
        _term: &Term,
    ) -> Result<()> {
        let payload = postings
            .get_payload()?
            .map(|bytes| BytesRef::new(bytes.to_vec()));
        let payload_factor = self.decoder.compute_payload_factor(payload.as_ref());
        // A span query with a `null` field opens no spans, so the field is
        // present whenever a leaf is collected.
        let field = self.field.as_deref().unwrap_or_default();
        self.payload_score = self.function.current_score(
            self.in_doc_id,
            field,
            self.in_start_position,
            self.in_end_position,
            self.payloads_seen,
            self.payload_score,
            payload_factor,
        );
        self.payloads_seen += 1;
        Ok(())
    }

    fn reset(&mut self) {}
}

/// The scorer of a [`PayloadSpanWeight`].
///
/// Equivalent to the private inner class
/// `org.apache.lucene.queries.payloads.PayloadScoreQuery.PayloadSpanScorer`,
/// which extends `SpanScorer`: the inherited scorer is held, typed over the
/// payload spans, which Java keeps in a second field.
pub(crate) struct PayloadSpanScorer {
    inner: SpanScorer<FilterSpans<PayloadSpans>>,
    function: Arc<dyn PayloadFunction>,
    include_span_score: bool,
    field: Option<String>,
}

impl std::fmt::Debug for PayloadSpanScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PayloadSpanScorer")
            .field("inner", &self.inner)
            .field("include_span_score", &self.include_span_score)
            .finish_non_exhaustive()
    }
}

impl PayloadSpanScorer {
    /// Equivalent to the private
    /// `PayloadSpanScorer(PayloadSpans, SimScorer, NumericDocValues)`.
    fn new(
        spans: FilterSpans<PayloadSpans>,
        scorer: Option<SharedSimScorer>,
        norms: Option<Box<dyn NumericDocValues>>,
        function: Arc<dyn PayloadFunction>,
        include_span_score: bool,
        field: Option<String>,
    ) -> Self {
        Self {
            inner: SpanScorer::new(spans, scorer, norms),
            function,
            include_span_score,
            field,
        }
    }

    fn field(&self) -> &str {
        self.field.as_deref().unwrap_or_default()
    }

    /// Equivalent to the protected `PayloadSpanScorer.getPayloadScore()`.
    fn get_payload_score(&self) -> f32 {
        let spans = self.inner.get_spans().filter();
        let score = self.function.doc_score(
            self.inner.doc_id(),
            self.field(),
            spans.payloads_seen,
            spans.payload_score,
        );
        #[allow(clippy::neg_cmp_op_on_partial_ord)]
        if !(score >= 0.0) {
            0.0
        } else {
            score
        }
    }

    /// Equivalent to the protected `PayloadSpanScorer.getPayloadExplanation()`.
    fn get_payload_explanation(&self) -> Explanation {
        let spans = self.inner.get_spans().filter();
        let mut expl = self.function.explain(
            self.inner.doc_id(),
            self.field(),
            spans.payloads_seen,
            spans.payload_score,
        );
        if expl.value().float_value() < 0.0 {
            expl = Explanation::matched(
                0i32,
                "truncated score, max of:",
                vec![
                    Explanation::matched(0f32, "minimum score", Vec::new()),
                    expl,
                ],
            );
        } else if expl.value().float_value().is_nan() {
            expl = Explanation::matched(
                0i32,
                "payload score, computed as (score == NaN ? 0 : score) since NaN is an illegal \
                 score from:",
                vec![expl],
            );
        }
        expl
    }

    /// Equivalent to the protected `PayloadSpanScorer.getSpanScore()`, which
    /// is `super.scoreCurrentDoc()`.
    fn get_span_score(&mut self) -> Result<f32> {
        self.inner.score_current_doc()
    }

    /// Equivalent to the overriding `PayloadSpanScorer.scoreCurrentDoc()`.
    fn score_current_doc(&mut self) -> Result<f32> {
        if self.include_span_score {
            return Ok(self.get_span_score()? * self.get_payload_score());
        }
        Ok(self.get_payload_score())
    }
}

impl Scorable for PayloadSpanScorer {
    /// The inherited final `SpanScorer.score()`, which calls the overriding
    /// `scoreCurrentDoc()`.
    fn score(&mut self) -> Result<f32> {
        self.inner.ensure_freq()?;
        self.score_current_doc()
    }
}

impl Scorer for PayloadSpanScorer {
    fn doc_id(&self) -> i32 {
        self.inner.doc_id()
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        self.inner.iterator()
    }

    fn iterator_ref(&self) -> Option<&dyn DocIdSetIterator> {
        self.inner.iterator_ref()
    }

    fn two_phase_iterator(&mut self) -> Option<&mut dyn TwoPhaseIterator> {
        self.inner.two_phase_iterator()
    }

    fn get_max_score(&mut self, up_to: i32) -> Result<f32> {
        self.inner.get_max_score(up_to)
    }
}
