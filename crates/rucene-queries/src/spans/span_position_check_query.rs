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

//! Port of `SpanPositionCheckQuery.java`, of the Java package
//! `org.apache.lucene.queries.spans`.

use std::sync::{Arc, Weak};

use rucene::error::Result;
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
use crate::spans::spans::Spans;

/// Base trait for filtering a span query based on the position of a match.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queries.spans.SpanPositionCheckQuery`.
///
/// **Divergence from Lucene 10.5.0.** Java's abstract class holds the
/// protected `match` field (`SpanPositionCheckQuery.java:34`), clones itself
/// in `rewrite` (`SpanPositionCheckQuery.java:115-128`) and makes
/// `createWeight`, `rewrite`, `visit`,
/// `equals` and `hashCode` concrete. A Rust trait cannot hold state, so the
/// field becomes the accessor [`get_match`](Self::get_match), Java's
/// `clone()`-and-assign in `rewrite` becomes [`with_match`](Self::with_match),
/// and the concrete members become the free functions
/// [`span_position_check_create_weight`], [`span_position_check_rewrite`],
/// [`span_position_check_visit`], [`span_position_check_eq`] and
/// [`span_position_check_hash`], which every implementation calls.
pub trait SpanPositionCheckQuery: SpanQuery {
    /// Returns the span query whose matches are filtered.
    ///
    /// Equivalent to `SpanPositionCheckQuery.getMatch()`.
    fn get_match(&self) -> &Arc<dyn SpanQuery>;

    /// Returns whether the current position is a match for the wrapped
    /// span query.
    ///
    /// Equivalent to the protected abstract
    /// `SpanPositionCheckQuery.acceptPosition(Spans)`. This is only called if
    /// the underlying last [`Spans::next_start_position`] for the match
    /// indicated a valid start position.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while inspecting the spans.
    fn accept_position(&self, spans: &mut dyn Spans) -> Result<AcceptStatus>;

    /// Returns a copy of this query whose wrapped query is `match_query`.
    ///
    /// Equivalent to `(SpanPositionCheckQuery) this.clone()` followed by
    /// `clone.match = rewritten` in `SpanPositionCheckQuery.rewrite`.
    fn with_match(&self, match_query: Arc<dyn SpanQuery>) -> Arc<dyn SpanPositionCheckQuery>;
}

/// The body of `SpanPositionCheckQuery.createWeight(IndexSearcher, ScoreMode, float)`.
///
/// # Errors
///
/// Propagates any I/O error raised while building the weights.
pub fn span_position_check_create_weight(
    query: Arc<dyn SpanPositionCheckQuery>,
    searcher: &IndexSearcher,
    score_mode: ScoreMode,
    boost: f32,
) -> Result<Arc<dyn SpanWeight>> {
    let match_weight = query
        .get_match()
        .create_span_weight(searcher, score_mode, boost)?;
    let terms = if score_mode.needs_scores() {
        Some(get_term_states(&[&*match_weight]))
    } else {
        None
    };
    Ok(SpanPositionCheckWeight::new(
        query,
        match_weight,
        searcher,
        terms.as_ref(),
        boost,
    )?)
}

/// The body of `SpanPositionCheckQuery.rewrite(IndexSearcher)`.
///
/// # Errors
///
/// Propagates any I/O error raised while rewriting the wrapped query.
pub fn span_position_check_rewrite(
    query: &dyn SpanPositionCheckQuery,
    searcher: &IndexSearcher,
) -> Result<Option<Arc<dyn SpanQuery>>> {
    if let Some(rewritten) = query.get_match().rewrite_span(searcher)? {
        let clone: Arc<dyn SpanQuery> = query.with_match(rewritten);
        return Ok(Some(clone));
    }

    Ok(None)
}

/// The body of `SpanPositionCheckQuery.visit(QueryVisitor)`.
pub fn span_position_check_visit(
    query: &dyn SpanPositionCheckQuery,
    visitor: &mut dyn QueryVisitor,
) {
    if accept_field(visitor, query.get_field()) {
        query
            .get_match()
            .visit(&mut *visitor.get_sub_visitor(Occur::MUST, query));
    }
}

/// The body of `SpanPositionCheckQuery.equals(Object)`:
/// `sameClassAs(other) && match.equals(((SpanPositionCheckQuery) other).match)`.
pub fn span_position_check_eq<T: SpanPositionCheckQuery>(query: &T, other: &dyn Query) -> bool {
    query.same_class_as(other)
        && other.as_any().downcast_ref::<T>().is_some_and(|other| {
            query
                .get_match()
                .query_eq(&**other.get_match() as &dyn Query)
        })
}

/// The body of `SpanPositionCheckQuery.hashCode()`: `classHash() ^ match.hashCode()`.
pub fn span_position_check_hash(query: &dyn SpanPositionCheckQuery) -> i32 {
    query.class_hash() ^ query.get_match().query_hash()
}

/// Creates span position check query scorer instances.
///
/// Equivalent to the inner class
/// `org.apache.lucene.queries.spans.SpanPositionCheckQuery.SpanPositionCheckWeight`.
pub struct SpanPositionCheckWeight {
    base: SpanWeightBase,
    query: Arc<dyn SpanPositionCheckQuery>,
    /// The package-private final `matchWeight` field.
    match_weight: Arc<dyn SpanWeight>,
}

impl std::fmt::Debug for SpanPositionCheckWeight {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpanPositionCheckWeight")
            .field("query", &self.query.to_query_string(""))
            .finish_non_exhaustive()
    }
}

impl SpanPositionCheckWeight {
    /// Equivalent to
    /// `SpanPositionCheckWeight(SpanWeight, IndexSearcher, Map<Term, TermStates>, float)`;
    /// `query` is the enclosing query.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the statistics.
    pub fn new(
        query: Arc<dyn SpanPositionCheckQuery>,
        match_weight: Arc<dyn SpanWeight>,
        searcher: &IndexSearcher,
        terms: Option<&TermStatesMap>,
        boost: f32,
    ) -> Result<Arc<Self>> {
        let parent: Arc<dyn SpanQuery> = Arc::clone(&query) as Arc<dyn SpanQuery>;
        let base = SpanWeightBase::new(parent, searcher, terms, boost)?;
        Ok(new_span_weight(move |this: Weak<dyn SpanWeight>| Self {
            base: base.bind(this),
            query,
            match_weight,
        }))
    }
}

impl SegmentCacheable for SpanPositionCheckWeight {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.match_weight.is_cacheable(ctx)
    }
}

impl Weight for SpanPositionCheckWeight {
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

impl SpanWeight for SpanPositionCheckWeight {
    fn span_weight_base(&self) -> &SpanWeightBase {
        &self.base
    }

    fn extract_term_states(&self, contexts: &mut TermStatesMap) {
        self.match_weight.extract_term_states(contexts);
    }

    fn get_spans(
        &self,
        context: &LeafReaderContext,
        required_postings: Postings,
    ) -> Result<Option<Box<dyn Spans>>> {
        let match_spans = self.match_weight.get_spans(context, required_postings)?;
        Ok(match_spans.map(|match_spans| {
            Box::new(FilterSpans::new(
                match_spans,
                PositionCheckFilter {
                    query: Arc::clone(&self.query),
                },
            )) as Box<dyn Spans>
        }))
    }
}

/// The filter of [`SpanPositionCheckWeight`]'s spans.
///
/// Equivalent to the anonymous subclass of `FilterSpans` that
/// `SpanPositionCheckWeight.getSpans` returns, whose `accept` is the enclosing
/// query's `acceptPosition`.
struct PositionCheckFilter {
    query: Arc<dyn SpanPositionCheckQuery>,
}

impl FilterSpansAccept for PositionCheckFilter {
    fn accept(&mut self, candidate: &mut dyn Spans) -> Result<AcceptStatus> {
        self.query.accept_position(candidate)
    }
}
