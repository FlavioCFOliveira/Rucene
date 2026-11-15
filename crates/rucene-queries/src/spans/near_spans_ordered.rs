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

//! Port of `NearSpansOrdered.java`, of the Java package
//! `org.apache.lucene.queries.spans`.

use rucene::error::Result;
use rucene::search::doc_id_set_iterator::DocIdSetIterator;
use rucene::search::two_phase_iterator::TwoPhaseIterator;

use crate::spans::conjunction_spans::{
    conjunction_advance, conjunction_next_doc, ConjunctionSpans, ConjunctionSpansImpl,
};
use crate::spans::span_collector::SpanCollector;
use crate::spans::span_near_query::GapSpans;
use crate::spans::spans::{
    positions_cost_unsupported, spans_to_string, SharedSpans, Spans, NO_MORE_POSITIONS,
};

/// A spans that is formed from the ordered sub-spans of a
/// [`SpanNearQuery`](crate::spans::span_near_query::SpanNearQuery) where the
/// sub-spans do not overlap and have a maximum slop between them.
///
/// Equivalent to `org.apache.lucene.queries.spans.NearSpansOrdered`, which
/// extends `ConjunctionSpans`.
///
/// The formed spans only contains minimum slop matches. The matching slop is
/// computed from the distance(s) between the non overlapping matching spans.
/// Successive matches are always formed from the successive spans of the span
/// near query.
///
/// The formed spans may contain overlaps when the slop is at least 1. For
/// example, when querying using `t1 t2 t3` with slop at least 1, the fragment
/// `t1 t2 t1 t3 t2 t3` matches twice: `t1 t2 .. t3` and `t1 .. t2 t3`.
///
/// Expert: only public for subclassing. Most implementations should not need
/// this class.
pub struct NearSpansOrdered {
    conj: ConjunctionSpans,
    /// The protected `matchStart` field.
    match_start: i32,
    /// The protected `matchEnd` field.
    match_end: i32,
    /// The protected `matchWidth` field.
    match_width: i32,
    allowed_slop: i32,
}

impl NearSpansOrdered {
    /// Equivalent to `NearSpansOrdered(int, List<Spans>)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`](rucene::error::LuceneErrorKind::IllegalArgument)
    /// when fewer than two sub-spans are given.
    pub fn new(allowed_slop: i32, sub_spans: Vec<Box<dyn Spans>>) -> Result<Self> {
        let mut conj = ConjunctionSpans::new(sub_spans)?;
        conj.at_first_in_current_doc = true; // -1 startPosition/endPosition also at doc -1
        Ok(Self {
            conj,
            match_start: -1,
            match_end: -1,
            match_width: -1,
            allowed_slop,
        })
    }

    /// Returns the sub-spans, in query order.
    ///
    /// Equivalent to the inherited `ConjunctionSpans.getSubSpans()`.
    pub fn get_sub_spans(&self) -> &[SharedSpans] {
        self.conj.get_sub_spans()
    }

    /// Equivalent to the private `NearSpansOrdered.unpositioned()`.
    fn unpositioned(&self) -> bool {
        for span in &self.conj.sub_spans {
            if span.start_position() != -1 {
                return false;
            }
        }
        true
    }

    /// Orders the sub-spans within the same document by using
    /// `nextStartPosition` on all sub-spans after the first as little as
    /// necessary. Returns `true` when the sub-spans could be ordered in this
    /// way, otherwise at least one is exhausted in the current doc.
    ///
    /// Equivalent to the private `NearSpansOrdered.stretchToOrder()`.
    fn stretch_to_order(&mut self) -> Result<bool> {
        let mut prev_spans = self.conj.sub_spans[0].clone();
        self.match_start = prev_spans.start_position();
        debug_assert!(
            prev_spans.start_position() != NO_MORE_POSITIONS,
            "prevSpans no start position {}",
            prev_spans.spans_to_string()
        );
        debug_assert!(prev_spans.end_position() != NO_MORE_POSITIONS);
        self.match_width = 0;
        for i in 1..self.conj.sub_spans.len() {
            let spans = self.conj.sub_spans[i].clone();
            debug_assert!(spans.start_position() != NO_MORE_POSITIONS);
            debug_assert!(spans.end_position() != NO_MORE_POSITIONS);
            if Self::advance_position(&spans, prev_spans.end_position())? == NO_MORE_POSITIONS {
                self.conj.one_exhausted_in_current_doc = true;
                return Ok(false);
            }
            self.match_width = self.match_width.wrapping_add(
                spans
                    .start_position()
                    .wrapping_sub(prev_spans.end_position()),
            );
            prev_spans = spans;
        }
        self.match_end = self.conj.sub_spans[self.conj.sub_spans.len() - 1].end_position();
        Ok(true) // all subSpans ordered and non overlapping
    }

    /// Equivalent to the private static
    /// `NearSpansOrdered.advancePosition(Spans, int)`, whose
    /// `spans instanceof SpanNearQuery.GapSpans` test is a downcast through
    /// [`DocIdSetIterator::disi_as_any_mut`].
    fn advance_position(spans: &SharedSpans, position: i32) -> Result<i32> {
        let skipped = spans.with_mut(|spans| {
            spans
                .disi_as_any_mut()
                .and_then(|any| any.downcast_mut::<GapSpans>())
                .map(|gap_spans| gap_spans.skip_to_position(position))
        });
        if let Some(skipped) = skipped {
            return skipped;
        }
        while spans.start_position() < position {
            spans.next_start_position()?;
        }
        Ok(spans.start_position())
    }
}

impl ConjunctionSpansImpl for NearSpansOrdered {
    fn conjunction_spans(&self) -> &ConjunctionSpans {
        &self.conj
    }

    fn conjunction_spans_mut(&mut self) -> &mut ConjunctionSpans {
        &mut self.conj
    }

    fn two_phase_current_doc_matches(&mut self) -> Result<bool> {
        debug_assert!(self.unpositioned());
        self.conj.one_exhausted_in_current_doc = false;
        while self.conj.sub_spans[0].next_start_position()? != NO_MORE_POSITIONS
            && !self.conj.one_exhausted_in_current_doc
        {
            if self.stretch_to_order()? && self.match_width <= self.allowed_slop {
                self.conj.at_first_in_current_doc = true;
                return Ok(true);
            }
        }
        Ok(false)
    }
}

impl DocIdSetIterator for NearSpansOrdered {
    /// `Spans.toString()`, which `NearSpansOrdered` does not override.
    fn java_to_string(&self) -> String {
        spans_to_string("NearSpansOrdered", self)
    }

    fn doc_id(&self) -> i32 {
        self.conj.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        conjunction_next_doc(self)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        conjunction_advance(self, target)
    }

    fn cost(&self) -> i64 {
        self.conj.cost()
    }
}

impl Spans for NearSpansOrdered {
    fn next_start_position(&mut self) -> Result<i32> {
        if self.conj.at_first_in_current_doc {
            self.conj.at_first_in_current_doc = false;
            return Ok(self.match_start);
        }
        self.conj.one_exhausted_in_current_doc = false;
        while self.conj.sub_spans[0].next_start_position()? != NO_MORE_POSITIONS
            && !self.conj.one_exhausted_in_current_doc
        {
            if self.stretch_to_order()? && self.match_width <= self.allowed_slop {
                return Ok(self.match_start);
            }
        }
        self.match_end = NO_MORE_POSITIONS;
        self.match_start = NO_MORE_POSITIONS;
        Ok(self.match_start)
    }

    fn start_position(&self) -> i32 {
        if self.conj.at_first_in_current_doc {
            -1
        } else {
            self.match_start
        }
    }

    fn end_position(&self) -> i32 {
        if self.conj.at_first_in_current_doc {
            -1
        } else {
            self.match_end
        }
    }

    fn width(&self) -> i32 {
        self.match_width
    }

    fn collect(&mut self, collector: &mut dyn SpanCollector) -> Result<()> {
        for span in &self.conj.sub_spans {
            span.collect(collector)?;
        }
        Ok(())
    }

    fn positions_cost(&self) -> f32 {
        positions_cost_unsupported() // asTwoPhaseIterator never returns null here.
    }

    /// Equivalent to the inherited `ConjunctionSpans.asTwoPhaseIterator()`.
    fn as_two_phase_iterator(&mut self) -> Option<&mut dyn TwoPhaseIterator> {
        Some(self)
    }

    fn as_two_phase_iterator_ref(&self) -> Option<&dyn TwoPhaseIterator> {
        Some(self)
    }
}

/// The anonymous `TwoPhaseIterator` of the inherited
/// `ConjunctionSpans.asTwoPhaseIterator()`.
impl TwoPhaseIterator for NearSpansOrdered {
    fn approximation(&mut self) -> &mut dyn DocIdSetIterator {
        &mut *self.conj.conjunction
    }

    fn approximation_ref(&self) -> &dyn DocIdSetIterator {
        &*self.conj.conjunction
    }

    fn matches(&mut self) -> Result<bool> {
        self.two_phase_current_doc_matches()
    }

    fn match_cost(&self) -> f32 {
        self.conj.match_cost()
    }
}
