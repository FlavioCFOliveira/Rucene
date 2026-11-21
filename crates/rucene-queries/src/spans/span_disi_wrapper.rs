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

//! Port of `SpanDisiWrapper.java`, of the Java package
//! `org.apache.lucene.queries.spans`.

use rucene::search::doc_id_set_iterator::DocIdSetIterator;
use rucene::search::two_phase_iterator::TwoPhaseIterator;

use crate::spans::spans::{SharedSpans, SharedSpansTwoPhase};

/// Wrapper used in `SpanDisiPriorityQueue`.
///
/// Equivalent to `org.apache.lucene.queries.spans.SpanDisiWrapper`.
///
/// **Divergence from Lucene 10.5.0.** Java's `next` field
/// (`SpanDisiWrapper.java:32`) threads the list that
/// `SpanDisiPriorityQueue.topList()` builds through the wrappers
/// (`SpanDisiPriorityQueue.java:65,77`). Rust
/// cannot let each wrapper point at another one it does not own, so
/// `topList()` returns the list as a vector of wrapper positions instead (see
/// `SpanDisiPriorityQueue::top_list`),
/// in the same traversal order. The views `iterator`, `approximation`,
/// `twoPhaseView` and `spans` are shared handles on the same spans, as Java's
/// fields are references to it.
pub struct SpanDisiWrapper {
    /// The spans as an iterator; the `iterator` field.
    pub iterator: SharedSpans,
    /// The `cost` field.
    pub cost: i64,
    /// The match cost for two-phase iterators, `0` otherwise; the `matchCost`
    /// field.
    pub match_cost: f32,
    /// The current doc, used for comparison; the `doc` field.
    pub doc: i32,
    /// An approximation of the iterator, or the iterator itself if it does not
    /// support two-phase iteration; the `approximation` field.
    pub approximation: Box<dyn DocIdSetIterator>,
    /// A two-phase view of the iterator, or `None` if the iterator does not
    /// support two-phase iteration; the `twoPhaseView` field.
    pub two_phase_view: Option<SharedSpansTwoPhase>,
    /// The `spans` field.
    pub spans: SharedSpans,
    /// Last doc of approximation that did match; the `lastApproxMatchDoc`
    /// field.
    pub last_approx_match_doc: i32,
    /// Last doc of approximation that did not match; the
    /// `lastApproxNonMatchDoc` field.
    pub last_approx_non_match_doc: i32,
}

impl std::fmt::Debug for SpanDisiWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpanDisiWrapper")
            .field("spans", &self.spans)
            .field("cost", &self.cost)
            .field("match_cost", &self.match_cost)
            .field("doc", &self.doc)
            .field("two_phase", &self.two_phase_view.is_some())
            .finish_non_exhaustive()
    }
}

impl SpanDisiWrapper {
    /// Equivalent to `SpanDisiWrapper(Spans)`.
    pub fn new(spans: SharedSpans) -> Self {
        let iterator = spans.clone();
        let cost = iterator.cost();
        let two_phase_view = spans.two_phase_view();

        let (approximation, match_cost): (Box<dyn DocIdSetIterator>, f32) = match &two_phase_view {
            Some(two_phase_view) => (
                Box::new(two_phase_view.approximation_handle()),
                two_phase_view.match_cost(),
            ),
            None => (Box::new(iterator.clone()), 0f32),
        };
        Self {
            iterator,
            cost,
            match_cost,
            doc: -1,
            approximation,
            two_phase_view,
            spans,
            last_approx_match_doc: -2,
            last_approx_non_match_doc: -2,
        }
    }
}
