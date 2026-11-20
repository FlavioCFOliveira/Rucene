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

//! Port of `NearSpansUnordered.java`, of the Java package
//! `org.apache.lucene.queries.spans`.

use rucene::error::Result;
use rucene::search::doc_id_set_iterator::DocIdSetIterator;
use rucene::search::two_phase_iterator::TwoPhaseIterator;
use rucene::util::extra::{PriorityQueue, PriorityQueueComparator};

use crate::spans::conjunction_spans::{
    conjunction_advance, conjunction_next_doc, ConjunctionSpans, ConjunctionSpansImpl,
};
use crate::spans::span_collector::SpanCollector;
use crate::spans::spans::{
    positions_cost_unsupported, spans_to_string, SharedSpans, Spans, NO_MORE_POSITIONS,
};

/// Message for the `NullPointerException` Java raises on an empty window.
const WINDOW_TOP: &str = "INVARIANT: the span window holds every sub-spans once a doc is started";

/// Similar to [`NearSpansOrdered`](crate::spans::near_spans_ordered::NearSpansOrdered),
/// but for the unordered case.
///
/// Equivalent to `org.apache.lucene.queries.spans.NearSpansUnordered`, which
/// extends `ConjunctionSpans`.
///
/// Expert: only public for subclassing. Most implementations should not need
/// this class.
pub struct NearSpansUnordered {
    conj: ConjunctionSpans,
    allowed_slop: i32,
    span_window: SpanTotalLengthEndPositionWindow,
}

impl NearSpansUnordered {
    /// Equivalent to `NearSpansUnordered(int, List<Spans>)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`](rucene::error::LuceneErrorKind::IllegalArgument)
    /// when fewer than two sub-spans are given.
    pub fn new(allowed_slop: i32, sub_spans: Vec<Box<dyn Spans>>) -> Result<Self> {
        let conj = ConjunctionSpans::new(sub_spans)?;
        let span_window = SpanTotalLengthEndPositionWindow::new(conj.sub_spans.len())?;
        Ok(Self {
            conj,
            allowed_slop,
            span_window,
        })
    }

    /// Returns the sub-spans, in query order.
    ///
    /// Equivalent to the inherited `ConjunctionSpans.getSubSpans()`.
    pub fn get_sub_spans(&self) -> &[SharedSpans] {
        self.conj.get_sub_spans()
    }

    /// Checks whether two spans in the same document are ordered with possible
    /// overlap: `true` iff `spans1` starts before `spans2`, or the spans start
    /// at the same position and `spans1` ends before `spans2`.
    ///
    /// Equivalent to the package-private static
    /// `NearSpansUnordered.positionsOrdered(Spans, Spans)`.
    pub(crate) fn positions_ordered(spans1: &SharedSpans, spans2: &SharedSpans) -> bool {
        debug_assert!(
            spans1.doc_id() == spans2.doc_id(),
            "doc1 {} != doc2 {}",
            spans1.doc_id(),
            spans2.doc_id()
        );
        let start1 = spans1.start_position();
        let start2 = spans2.start_position();
        if start1 == start2 {
            spans1.end_position() < spans2.end_position()
        } else {
            start1 < start2
        }
    }

    fn window_top_start(&self) -> i32 {
        self.span_window
            .queue
            .top()
            .expect(WINDOW_TOP)
            .start_position()
    }
}

/// The ordering of [`SpanTotalLengthEndPositionWindow`]: its `lessThan`.
struct PositionsOrdered;

impl PriorityQueueComparator<SharedSpans> for PositionsOrdered {
    fn less_than(&self, spans1: &SharedSpans, spans2: &SharedSpans) -> bool {
        NearSpansUnordered::positions_ordered(spans1, spans2)
    }
}

/// Maintains `totalSpanLength` and `maxEndPosition`.
///
/// Equivalent to the private inner class
/// `org.apache.lucene.queries.spans.NearSpansUnordered.SpanTotalLengthEndPositionWindow`,
/// which extends `PriorityQueue<Spans>`. Java's inner class reads the
/// enclosing instance's `subSpans` and `allowedSlop`; they are passed to the
/// methods that need them.
pub(crate) struct SpanTotalLengthEndPositionWindow {
    queue: PriorityQueue<SharedSpans, PositionsOrdered>,
    total_span_length: i32,
    max_end_position: i32,
}

impl SpanTotalLengthEndPositionWindow {
    /// Equivalent to `SpanTotalLengthEndPositionWindow()`, which is
    /// `super(subSpans.length)`.
    fn new(max_size: usize) -> Result<Self> {
        Ok(Self {
            queue: PriorityQueue::new(max_size, PositionsOrdered)?,
            total_span_length: 0,
            max_end_position: 0,
        })
    }

    /// Equivalent to `SpanTotalLengthEndPositionWindow.startDocument()`.
    fn start_document(&mut self, sub_spans: &[SharedSpans]) -> Result<()> {
        self.queue.clear();
        self.total_span_length = 0;
        self.max_end_position = -1;
        for spans in sub_spans {
            debug_assert!(spans.start_position() == -1);
            spans.next_start_position()?;
            debug_assert!(spans.start_position() != NO_MORE_POSITIONS);
            self.queue.add(spans.clone());
            if spans.end_position() > self.max_end_position {
                self.max_end_position = spans.end_position();
            }
            let span_length = spans.end_position().wrapping_sub(spans.start_position());
            debug_assert!(span_length >= 0);
            self.total_span_length = self.total_span_length.wrapping_add(span_length);
        }
        Ok(())
    }

    /// Equivalent to `SpanTotalLengthEndPositionWindow.nextPosition()`.
    fn next_position(&mut self) -> Result<bool> {
        let top_spans = self.queue.top().expect(WINDOW_TOP).clone();
        debug_assert!(top_spans.start_position() != NO_MORE_POSITIONS);
        let mut span_length = top_spans
            .end_position()
            .wrapping_sub(top_spans.start_position());
        let next_start_pos = top_spans.next_start_position()?;
        if next_start_pos == NO_MORE_POSITIONS {
            return Ok(false);
        }
        self.total_span_length = self.total_span_length.wrapping_sub(span_length);
        span_length = top_spans
            .end_position()
            .wrapping_sub(top_spans.start_position());
        self.total_span_length = self.total_span_length.wrapping_add(span_length);
        if top_spans.end_position() > self.max_end_position {
            self.max_end_position = top_spans.end_position();
        }
        self.queue.update_top();
        Ok(true)
    }

    /// Equivalent to `SpanTotalLengthEndPositionWindow.atMatch()`.
    fn at_match(&self, allowed_slop: i32) -> bool {
        let top_start = self.queue.top().expect(WINDOW_TOP).start_position();
        self.max_end_position
            .wrapping_sub(top_start)
            .wrapping_sub(self.total_span_length)
            <= allowed_slop
    }
}

impl ConjunctionSpansImpl for NearSpansUnordered {
    fn conjunction_spans(&self) -> &ConjunctionSpans {
        &self.conj
    }

    fn conjunction_spans_mut(&mut self) -> &mut ConjunctionSpans {
        &mut self.conj
    }

    fn two_phase_current_doc_matches(&mut self) -> Result<bool> {
        // at doc with all subSpans
        self.span_window.start_document(&self.conj.sub_spans)?;
        loop {
            if self.span_window.at_match(self.allowed_slop) {
                self.conj.at_first_in_current_doc = true;
                self.conj.one_exhausted_in_current_doc = false;
                return Ok(true);
            }
            if !self.span_window.next_position()? {
                return Ok(false);
            }
        }
    }
}

impl DocIdSetIterator for NearSpansUnordered {
    /// `Spans.toString()`, which `NearSpansUnordered` does not override.
    fn java_to_string(&self) -> String {
        spans_to_string("NearSpansUnordered", self)
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

impl Spans for NearSpansUnordered {
    fn next_start_position(&mut self) -> Result<i32> {
        if self.conj.at_first_in_current_doc {
            self.conj.at_first_in_current_doc = false;
            return Ok(self.window_top_start());
        }
        debug_assert!(self.window_top_start() != -1);
        debug_assert!(self.window_top_start() != NO_MORE_POSITIONS);
        loop {
            if !self.span_window.next_position()? {
                self.conj.one_exhausted_in_current_doc = true;
                return Ok(NO_MORE_POSITIONS);
            }
            if self.span_window.at_match(self.allowed_slop) {
                return Ok(self.window_top_start());
            }
        }
    }

    fn start_position(&self) -> i32 {
        debug_assert!(self.span_window.queue.top().is_some());
        if self.conj.at_first_in_current_doc {
            -1
        } else if self.conj.one_exhausted_in_current_doc {
            NO_MORE_POSITIONS
        } else {
            self.window_top_start()
        }
    }

    fn end_position(&self) -> i32 {
        if self.conj.at_first_in_current_doc {
            -1
        } else if self.conj.one_exhausted_in_current_doc {
            NO_MORE_POSITIONS
        } else {
            self.span_window.max_end_position
        }
    }

    fn width(&self) -> i32 {
        self.span_window
            .max_end_position
            .wrapping_sub(self.window_top_start())
    }

    fn collect(&mut self, collector: &mut dyn SpanCollector) -> Result<()> {
        for spans in &self.conj.sub_spans {
            spans.collect(collector)?;
        }
        Ok(())
    }

    /// Equivalent to the inherited `ConjunctionSpans.positionsCost()`: an
    /// `UnsupportedOperationException`.
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
impl TwoPhaseIterator for NearSpansUnordered {
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
