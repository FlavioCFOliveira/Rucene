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

//! Port of `AssertingSpanWeight.java` of Lucene 10.5.0
//! `lucene/queries/src/test/org/apache/lucene/queries/spans/`.

use std::sync::{Arc, Weak};

use rucene::error::Result;
use rucene::index::LeafReaderContext;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::matches::Matches;
use rucene::search::query::Query;
use rucene::search::scorer_supplier::ScorerSupplier;
use rucene::search::segment_cacheable::SegmentCacheable;
use rucene::search::sim_scorer_source::SharedSimScorer;
use rucene::search::similarities::Explanation;
use rucene::search::weight::Weight;
use rucene_queries::spans::span_query::{SpanQuery, TermStatesMap};
use rucene_queries::spans::span_scorer::SpanScorer;
use rucene_queries::spans::span_weight::{
    span_weight_matches, Postings, SpanWeight, SpanWeightBase,
};
use rucene_queries::spans::spans::Spans;

use super::asserting_spans::AssertingSpans;

/// Wraps a `SpanWeight` with additional asserts.
///
/// Equivalent to `org.apache.lucene.queries.spans.AssertingSpanWeight`.
pub struct AssertingSpanWeight {
    base: SpanWeightBase,
    /// `in`.
    in_weight: Arc<dyn SpanWeight>,
}

impl std::fmt::Debug for AssertingSpanWeight {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssertingSpanWeight")
            .field("in", &self.in_weight)
            .finish_non_exhaustive()
    }
}

impl AssertingSpanWeight {
    /// `AssertingSpanWeight(IndexSearcher searcher, SpanWeight in)`: "Create an
    /// AssertingSpanWeight", `super((SpanQuery) in.getQuery(), searcher,
    /// null, 1f)`.
    ///
    /// `in_query` is `(SpanQuery) in.getQuery()`: every span weight of Lucene
    /// hands its own query to `SpanWeight`'s constructor, so that query is
    /// the one the wrapping [`AssertingSpanQuery`](super::asserting_span_query::AssertingSpanQuery)
    /// asked for the weight; Rust cannot cast the `dyn Query` that
    /// [`Weight::get_query`] returns back to `dyn SpanQuery`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while building the weight state.
    pub fn new(
        searcher: &IndexSearcher,
        in_weight: Arc<dyn SpanWeight>,
        in_query: Arc<dyn SpanQuery>,
    ) -> Result<Arc<Self>> {
        let base = SpanWeightBase::new(in_query, searcher, None, 1.0)?;
        Ok(Arc::new_cyclic(|weak: &Weak<AssertingSpanWeight>| {
            let this: Weak<dyn SpanWeight> = weak.clone();
            AssertingSpanWeight {
                base: base.bind(this),
                in_weight,
            }
        }))
    }
}

impl SegmentCacheable for AssertingSpanWeight {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.in_weight.is_cacheable(ctx)
    }
}

impl Weight for AssertingSpanWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        self.base.get_query()
    }

    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation> {
        self.in_weight.explain(context, doc)
    }

    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        self.in_weight.scorer_supplier(context)
    }

    /// Java's `AssertingSpanWeight` inherits `SpanWeight.matches`, which
    /// iterates this weight's own (asserting) spans.
    fn matches(&self, context: &LeafReaderContext, doc: i32) -> Result<Option<Arc<dyn Matches>>> {
        span_weight_matches(self, context, doc)
    }
}

impl SpanWeight for AssertingSpanWeight {
    fn span_weight_base(&self) -> &SpanWeightBase {
        &self.base
    }

    fn extract_term_states(&self, contexts: &mut TermStatesMap) {
        self.in_weight.extract_term_states(contexts);
    }

    fn get_spans(
        &self,
        context: &LeafReaderContext,
        required_postings: Postings,
    ) -> Result<Option<Box<dyn Spans>>> {
        let Some(spans) = self.in_weight.get_spans(context, required_postings)? else {
            return Ok(None);
        };
        Ok(Some(Box::new(AssertingSpans::new(spans))))
    }

    fn get_sim_scorer(&self) -> Option<SharedSimScorer> {
        self.in_weight.get_sim_scorer()
    }

    /// `(SpanScorer) scorer(context)`: Java's `scorer` goes through the
    /// overridden `scorerSupplier`, so the wrapped weight's scorer.
    fn span_scorer(&self, context: &LeafReaderContext) -> Result<Option<SpanScorer>> {
        self.in_weight.span_scorer(context)
    }
}
