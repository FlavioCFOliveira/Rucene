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

//! Port of `SpanNotQuery.java`, of the Java package
//! `org.apache.lucene.queries.spans`.

use std::any::Any;
use std::sync::{Arc, Weak};

use rucene::error::{LuceneError, Result};
use rucene::index::LeafReaderContext;
use rucene::search::boolean_clause::Occur;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::matches::Matches;
use rucene::search::query::Query;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::score_mode::ScoreMode;
use rucene::search::scorer_supplier::ScorerSupplier;
use rucene::search::segment_cacheable::SegmentCacheable;
use rucene::search::similarities::Explanation;
use rucene::search::weight::Weight;

use crate::spans::filter_spans::{AcceptStatus, FilterSpans, FilterSpansAccept};
use crate::spans::span_query::{accept_field, get_term_states, SpanQuery, TermStatesMap};
use crate::spans::span_weight::{
    new_span_weight, span_weight_explain, span_weight_matches, span_weight_scorer_supplier,
    Postings, SpanWeight, SpanWeightBase,
};
use crate::spans::spans::{Spans, NO_MORE_POSITIONS};

/// Message used where the exclude spans' two-phase view was observed when the
/// filter was built.
const EXCLUDE_TWO_PHASE: &str =
    "INVARIANT: the exclude spans answered a two-phase view when the filter was built";

/// Removes matches which overlap with another span query or which are within
/// `x` tokens before or `y` tokens after another span query.
///
/// Equivalent to the final `org.apache.lucene.queries.spans.SpanNotQuery`.
#[derive(Debug, Clone)]
pub struct SpanNotQuery {
    include: Arc<dyn SpanQuery>,
    exclude: Arc<dyn SpanQuery>,
    pre: i32,
    post: i32,
}

impl SpanNotQuery {
    /// Constructs a query matching spans from `include` which have no overlap
    /// with spans from `exclude`.
    ///
    /// Equivalent to `SpanNotQuery(SpanQuery, SpanQuery)`.
    ///
    /// # Errors
    ///
    /// As [`with_pre_post`](Self::with_pre_post).
    pub fn new(include: Arc<dyn SpanQuery>, exclude: Arc<dyn SpanQuery>) -> Result<Self> {
        Self::with_pre_post(include, exclude, 0, 0)
    }

    /// Constructs a query matching spans from `include` which have no overlap
    /// with spans from `exclude` within `dist` tokens of `include`. Inversely,
    /// a negative `dist` value may be used to specify a certain amount of
    /// allowable overlap.
    ///
    /// Equivalent to `SpanNotQuery(SpanQuery, SpanQuery, int)`.
    ///
    /// # Errors
    ///
    /// As [`with_pre_post`](Self::with_pre_post).
    pub fn with_dist(
        include: Arc<dyn SpanQuery>,
        exclude: Arc<dyn SpanQuery>,
        dist: i32,
    ) -> Result<Self> {
        Self::with_pre_post(include, exclude, dist, dist)
    }

    /// Constructs a query matching spans from `include` which have no overlap
    /// with spans from `exclude` within `pre` tokens before or `post` tokens
    /// of `include`. Inversely, negative values for `pre` and/or `post` allow a
    /// certain amount of overlap to occur.
    ///
    /// Equivalent to `SpanNotQuery(SpanQuery, SpanQuery, int, int)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`](rucene::error::LuceneErrorKind::IllegalArgument)
    /// when both clauses have a field and the fields differ.
    pub fn with_pre_post(
        include: Arc<dyn SpanQuery>,
        exclude: Arc<dyn SpanQuery>,
        pre: i32,
        post: i32,
    ) -> Result<Self> {
        if let (Some(include_field), Some(exclude_field)) =
            (include.get_field(), exclude.get_field())
        {
            if include_field != exclude_field {
                return Err(LuceneError::illegal_argument(
                    "Clauses must have same field.".to_string(),
                ));
            }
        }
        Ok(Self {
            include,
            exclude,
            pre,
            post,
        })
    }

    /// Returns the span query whose matches are filtered.
    ///
    /// Equivalent to `SpanNotQuery.getInclude()`.
    pub fn get_include(&self) -> &Arc<dyn SpanQuery> {
        &self.include
    }

    /// Returns the span query whose matches must not overlap those returned.
    ///
    /// Equivalent to `SpanNotQuery.getExclude()`.
    pub fn get_exclude(&self) -> &Arc<dyn SpanQuery> {
        &self.exclude
    }

    /// Equivalent to the private `SpanNotQuery.equalsTo(SpanNotQuery)`.
    fn equals_to(&self, other: &SpanNotQuery) -> bool {
        self.include.query_eq(&*other.include as &dyn Query)
            && self.exclude.query_eq(&*other.exclude as &dyn Query)
            && self.pre == other.pre
            && self.post == other.post
    }
}

impl Query for SpanNotQuery {
    fn to_query_string(&self, field: &str) -> String {
        let mut buffer = String::new();
        buffer.push_str("spanNot(");
        buffer.push_str(&self.include.to_query_string(field));
        buffer.push_str(", ");
        buffer.push_str(&self.exclude.to_query_string(field));
        buffer.push_str(", ");
        buffer.push_str(&self.pre.to_string());
        buffer.push_str(", ");
        buffer.push_str(&self.post.to_string());
        buffer.push(')');
        buffer
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        if accept_field(visitor, self.get_field()) {
            self.include
                .visit(&mut *visitor.get_sub_visitor(Occur::MUST, self));
            self.exclude
                .visit(&mut *visitor.get_sub_visitor(Occur::MUST_NOT, self));
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

    /// Returns `true` iff `other` is equal to this.
    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
            && other
                .as_any()
                .downcast_ref::<SpanNotQuery>()
                .is_some_and(|other| self.equals_to(other))
    }

    fn query_hash(&self) -> i32 {
        let mut h = self.class_hash();
        h = h.rotate_left(1);
        h ^= self.include.query_hash();
        h = h.rotate_left(1);
        h ^= self.exclude.query_hash();
        h = h.rotate_left(1);
        h ^= self.pre;
        h = h.rotate_left(1);
        h ^= self.post;
        h
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.queries.spans.SpanNotQuery"
    }
}

impl SpanQuery for SpanNotQuery {
    fn get_field(&self) -> Option<&str> {
        self.include.get_field()
    }

    fn create_span_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn SpanWeight>> {
        let include_weight = self
            .include
            .create_span_weight(searcher, score_mode, boost)?;
        let exclude_weight =
            self.exclude
                .create_span_weight(searcher, ScoreMode::COMPLETE_NO_SCORES, boost)?;
        let terms = if score_mode.needs_scores() {
            Some(get_term_states(&[&*include_weight]))
        } else {
            None
        };
        Ok(SpanNotWeight::new(
            Arc::new(self.clone()),
            searcher,
            terms.as_ref(),
            include_weight,
            exclude_weight,
            boost,
        )?)
    }

    fn rewrite_span(&self, searcher: &IndexSearcher) -> Result<Option<Arc<dyn SpanQuery>>> {
        let rewritten_include = self.include.rewrite_span(searcher)?;
        let rewritten_exclude = self.exclude.rewrite_span(searcher)?;
        if rewritten_include.is_some() || rewritten_exclude.is_some() {
            return Ok(Some(Arc::new(SpanNotQuery::with_pre_post(
                rewritten_include.unwrap_or_else(|| Arc::clone(&self.include)),
                rewritten_exclude.unwrap_or_else(|| Arc::clone(&self.exclude)),
                self.pre,
                self.post,
            )?)));
        }
        Ok(None)
    }
}

/// Creates [`SpanNotQuery`] scorer instances.
///
/// Equivalent to the inner class
/// `org.apache.lucene.queries.spans.SpanNotQuery.SpanNotWeight`.
pub struct SpanNotWeight {
    base: SpanWeightBase,
    /// The package-private final `includeWeight` field.
    include_weight: Arc<dyn SpanWeight>,
    /// The package-private final `excludeWeight` field.
    exclude_weight: Arc<dyn SpanWeight>,
    pre: i32,
    post: i32,
}

impl std::fmt::Debug for SpanNotWeight {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpanNotWeight")
            .field("pre", &self.pre)
            .field("post", &self.post)
            .finish_non_exhaustive()
    }
}

impl SpanNotWeight {
    /// Equivalent to
    /// `SpanNotWeight(IndexSearcher, Map<Term, TermStates>, SpanWeight, SpanWeight, float)`;
    /// `query` is the enclosing query.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the statistics.
    pub fn new(
        query: Arc<SpanNotQuery>,
        searcher: &IndexSearcher,
        terms: Option<&TermStatesMap>,
        include_weight: Arc<dyn SpanWeight>,
        exclude_weight: Arc<dyn SpanWeight>,
        boost: f32,
    ) -> Result<Arc<Self>> {
        let pre = query.pre;
        let post = query.post;
        let base = SpanWeightBase::new(query, searcher, terms, boost)?;
        Ok(new_span_weight(move |this: Weak<dyn SpanWeight>| Self {
            base: base.bind(this),
            include_weight,
            exclude_weight,
            pre,
            post,
        }))
    }
}

impl SegmentCacheable for SpanNotWeight {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.include_weight.is_cacheable(ctx) && self.exclude_weight.is_cacheable(ctx)
    }
}

impl Weight for SpanNotWeight {
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

impl SpanWeight for SpanNotWeight {
    fn span_weight_base(&self) -> &SpanWeightBase {
        &self.base
    }

    fn extract_term_states(&self, contexts: &mut TermStatesMap) {
        self.include_weight.extract_term_states(contexts);
    }

    fn get_spans(
        &self,
        context: &LeafReaderContext,
        required_postings: Postings,
    ) -> Result<Option<Box<dyn Spans>>> {
        let Some(include_spans) = self.include_weight.get_spans(context, required_postings)? else {
            return Ok(None);
        };

        let Some(mut exclude_spans) = self.exclude_weight.get_spans(context, required_postings)?
        else {
            return Ok(Some(include_spans));
        };

        let has_exclude_two_phase = exclude_spans.as_two_phase_iterator().is_some();

        Ok(Some(Box::new(FilterSpans::new(
            include_spans,
            SpanNotFilter {
                exclude_spans,
                has_exclude_two_phase,
                pre: self.pre,
                post: self.post,
                last_approx_doc: -1,
                last_approx_result: false,
            },
        ))))
    }
}

/// The filter of [`SpanNotWeight`]'s spans.
///
/// Equivalent to the anonymous subclass of `FilterSpans` that
/// `SpanNotWeight.getSpans` returns, together with the `excludeSpans`,
/// `excludeTwoPhase` and `excludeApproximation` it captures: the two views are
/// reached through `exclude_spans` whenever Java reads them.
struct SpanNotFilter {
    exclude_spans: Box<dyn Spans>,
    /// `excludeTwoPhase != null`.
    has_exclude_two_phase: bool,
    pre: i32,
    post: i32,
    /// Last document we have checked matches() against for the exclusion, and
    /// failed when using approximations, so we don't call it again, and pass
    /// thru all inclusions.
    last_approx_doc: i32,
    last_approx_result: bool,
}

impl FilterSpansAccept for SpanNotFilter {
    fn accept(&mut self, candidate: &mut dyn Spans) -> Result<AcceptStatus> {
        // TODO(lucene): this logic is ugly and sneaky, can we clean it up?
        let doc = candidate.doc_id();
        if doc > self.exclude_spans.doc_id() {
            // catch up 'exclude' to the current doc
            if self.has_exclude_two_phase {
                let exclude_two_phase = self
                    .exclude_spans
                    .as_two_phase_iterator()
                    .expect(EXCLUDE_TWO_PHASE);
                if exclude_two_phase.approximation().advance(doc)? == doc {
                    self.last_approx_doc = doc;
                    self.last_approx_result = self
                        .exclude_spans
                        .as_two_phase_iterator()
                        .expect(EXCLUDE_TWO_PHASE)
                        .matches()?;
                }
            } else {
                self.exclude_spans.advance(doc)?;
            }
        } else if self.has_exclude_two_phase
            && doc == self.exclude_spans.doc_id()
            && doc != self.last_approx_doc
        {
            // excludeSpans already sitting on our candidate doc, but matches not called yet.
            self.last_approx_doc = doc;
            self.last_approx_result = self
                .exclude_spans
                .as_two_phase_iterator()
                .expect(EXCLUDE_TWO_PHASE)
                .matches()?;
        }

        if doc != self.exclude_spans.doc_id()
            || (doc == self.last_approx_doc && !self.last_approx_result)
        {
            return Ok(AcceptStatus::Yes);
        }

        if self.exclude_spans.start_position() == -1 {
            // init exclude start position if needed
            self.exclude_spans.next_start_position()?;
        }

        while self.exclude_spans.end_position() <= candidate.start_position().wrapping_sub(self.pre)
        {
            // exclude end position is before a possible exclusion
            if self.exclude_spans.next_start_position()? == NO_MORE_POSITIONS {
                return Ok(AcceptStatus::Yes); // no more exclude at current doc.
            }
        }

        // exclude end position far enough in current doc, check start position:
        if self.exclude_spans.start_position().wrapping_sub(self.post) >= candidate.end_position() {
            Ok(AcceptStatus::Yes)
        } else {
            Ok(AcceptStatus::No)
        }
    }
}
