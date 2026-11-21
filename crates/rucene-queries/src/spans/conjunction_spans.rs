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

//! Port of `ConjunctionSpans.java`, of the Java package
//! `org.apache.lucene.queries.spans`.

use std::cell::Cell;

use rucene::error::{LuceneError, Result};
use rucene::search::conjunction_disi::ConjunctionMember;
use rucene::search::conjunction_utils::ConjunctionUtils;
use rucene::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};

use crate::spans::spans::{SharedSpans, Spans};

/// Common state of multiple sub spans required in a document.
///
/// Equivalent to the package-private abstract class
/// `org.apache.lucene.queries.spans.ConjunctionSpans`. Its fields and concrete
/// members are here; its subclasses — [`NearSpansOrdered`],
/// [`NearSpansUnordered`] and the two
/// [`ContainSpans`](crate::spans::contain_spans::ContainSpans) — hold one and
/// implement [`ConjunctionSpansImpl`], the abstract
/// `twoPhaseCurrentDocMatches()`. Each of them implements [`Spans`] and
/// [`TwoPhaseIterator`](rucene::search::two_phase_iterator::TwoPhaseIterator)
/// through [`conjunction_next_doc`], [`conjunction_advance`] and the members
/// of this type, as Java's subclasses inherit them.
///
/// The sub-spans are [`SharedSpans`] because Java holds them twice: in
/// `subSpans`, in query order, and inside the `conjunction` that intersects
/// them.
///
/// [`NearSpansOrdered`]: crate::spans::near_spans_ordered::NearSpansOrdered
/// [`NearSpansUnordered`]: crate::spans::near_spans_unordered::NearSpansUnordered
pub(crate) struct ConjunctionSpans {
    /// In query order; the `subSpans` field.
    pub(crate) sub_spans: Vec<SharedSpans>,
    /// Use to move to next doc with all clauses; the `conjunction` field.
    pub(crate) conjunction: Box<dyn DocIdSetIterator>,
    /// A first start position is available in current doc for
    /// `nextStartPosition`; the `atFirstInCurrentDoc` field.
    pub(crate) at_first_in_current_doc: bool,
    /// One sub-spans exhausted in current doc; the
    /// `oneExhaustedInCurrentDoc` field.
    pub(crate) one_exhausted_in_current_doc: bool,
    /// The match cost of the two-phase view, computed on the first
    /// `asTwoPhaseIterator()` call. Java computes it on every call; it only
    /// reads construction-time costs of the sub-spans, so every call answers
    /// the same value.
    match_cost: Cell<Option<f32>>,
}

impl ConjunctionSpans {
    /// Equivalent to the package-private `ConjunctionSpans(List<Spans>)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`](rucene::error::LuceneErrorKind::IllegalArgument)
    /// when fewer than two sub-spans are given.
    pub(crate) fn new(sub_spans: Vec<Box<dyn Spans>>) -> Result<Self> {
        if sub_spans.len() < 2 {
            return Err(LuceneError::illegal_argument(format!(
                "Less than 2 subSpans.size():{}",
                sub_spans.len()
            )));
        }
        let sub_spans: Vec<SharedSpans> = sub_spans.into_iter().map(SharedSpans::new).collect();
        let conjunction = Self::intersect_spans(&sub_spans)?;
        Ok(Self {
            sub_spans,
            conjunction,
            at_first_in_current_doc: true, // ensure for doc -1 that start/end positions are -1
            one_exhausted_in_current_doc: false,
            match_cost: Cell::new(None),
        })
    }

    /// `ConjunctionSpans.docID()`.
    pub(crate) fn doc_id(&self) -> i32 {
        self.conjunction.doc_id()
    }

    /// `ConjunctionSpans.cost()`.
    pub(crate) fn cost(&self) -> i64 {
        self.conjunction.cost()
    }

    /// The match cost of the two-phase view: the total match cost, or
    /// positions cost, of the sub-spans.
    ///
    /// Equivalent to the computation at the head of
    /// `ConjunctionSpans.asTwoPhaseIterator()`.
    pub(crate) fn match_cost(&self) -> f32 {
        if let Some(match_cost) = self.match_cost.get() {
            return match_cost;
        }
        let mut total_match_cost = 0f32;
        // Compute the matchCost as the total matchCost/positionsCostant of the sub spans.
        for spans in &self.sub_spans {
            let match_cost = spans.with_ref(|spans| match spans.as_two_phase_iterator_ref() {
                Some(tpi) => tpi.match_cost(),
                None => spans.positions_cost(),
            });
            total_match_cost += match_cost;
        }
        self.match_cost.set(Some(total_match_cost));
        total_match_cost
    }

    /// Returns the sub-spans, in query order.
    ///
    /// Equivalent to `ConjunctionSpans.getSubSpans()`.
    pub(crate) fn get_sub_spans(&self) -> &[SharedSpans] {
        &self.sub_spans
    }

    /// Equivalent to the private static
    /// `ConjunctionSpans.intersectSpans(List<Spans>)`.
    fn intersect_spans(span_list: &[SharedSpans]) -> Result<Box<dyn DocIdSetIterator>> {
        if span_list.len() < 2 {
            return Err(LuceneError::illegal_argument(
                "Cannot make a ConjunctionDISI of less than 2 iterators".to_string(),
            ));
        }
        let mut members: Vec<ConjunctionMember> = Vec::with_capacity(span_list.len());
        for spans in span_list {
            Self::add_spans(spans, &mut members);
        }

        Ok(ConjunctionUtils::create_conjunction(members)?.into_doc_id_set_iterator())
    }

    /// Equivalent to the private static
    /// `ConjunctionSpans.addSpans(Spans, List<DocIdSetIterator>, List<TwoPhaseIterator>)`.
    fn add_spans(spans: &SharedSpans, members: &mut Vec<ConjunctionMember>) {
        match spans.two_phase_view() {
            Some(two_phase_iter) => {
                ConjunctionUtils::add_two_phase_iterator(Box::new(two_phase_iter), members);
            }
            None => {
                // no approximation support, use the iterator as-is
                ConjunctionUtils::add_iterator(Box::new(spans.clone()), members);
            }
        }
    }
}

/// The abstract member of [`ConjunctionSpans`] its subclasses implement.
///
/// Equivalent to the package-private abstract
/// `ConjunctionSpans.twoPhaseCurrentDocMatches()`, together with the access to
/// the inherited state.
pub(crate) trait ConjunctionSpansImpl {
    /// The inherited state.
    fn conjunction_spans(&self) -> &ConjunctionSpans;

    /// The inherited state, mutably.
    fn conjunction_spans_mut(&mut self) -> &mut ConjunctionSpans;

    /// Equivalent to `ConjunctionSpans.twoPhaseCurrentDocMatches()`.
    fn two_phase_current_doc_matches(&mut self) -> Result<bool>;
}

/// `ConjunctionSpans.nextDoc()`.
pub(crate) fn conjunction_next_doc<T: ConjunctionSpansImpl + ?Sized>(spans: &mut T) -> Result<i32> {
    if spans.conjunction_spans_mut().conjunction.next_doc()? == NO_MORE_DOCS {
        Ok(NO_MORE_DOCS)
    } else {
        to_match_doc(spans)
    }
}

/// `ConjunctionSpans.advance(int)`.
pub(crate) fn conjunction_advance<T: ConjunctionSpansImpl + ?Sized>(
    spans: &mut T,
    target: i32,
) -> Result<i32> {
    if spans.conjunction_spans_mut().conjunction.advance(target)? == NO_MORE_DOCS {
        Ok(NO_MORE_DOCS)
    } else {
        to_match_doc(spans)
    }
}

/// Equivalent to the package-private `ConjunctionSpans.toMatchDoc()`.
pub(crate) fn to_match_doc<T: ConjunctionSpansImpl + ?Sized>(spans: &mut T) -> Result<i32> {
    spans.conjunction_spans_mut().one_exhausted_in_current_doc = false;
    loop {
        if spans.two_phase_current_doc_matches()? {
            return Ok(spans.conjunction_spans().doc_id());
        }
        if spans.conjunction_spans_mut().conjunction.next_doc()? == NO_MORE_DOCS {
            return Ok(NO_MORE_DOCS);
        }
    }
}
