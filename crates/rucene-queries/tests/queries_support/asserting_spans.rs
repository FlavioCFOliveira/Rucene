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

//! Port of `AssertingSpans.java` of Lucene 10.5.0
//! `lucene/queries/src/test/org/apache/lucene/queries/spans/`: wraps a
//! `Spans` with additional asserts.
//!
//! # Layout
//!
//! Java's `AssertingSpans` hands out a new `AssertingTwoPhaseView` from each
//! `asTwoPhaseIterator()` call, whose approximation (`AssertingDISI`) shares
//! the outer spans' `doc` and `state` through `AssertingSpans.this`. Rust's
//! [`Spans::as_two_phase_iterator`] returns a borrow, so the view is the
//! [`AssertingSpans`] itself (the pattern of the port's `FilterSpans`) and
//! the approximation view is the field holding the shared state
//! ([`Core`]), which the outer spans reach through that field.
//!
//! The view's `lastDoc` is a field of the spans rather than of a view object
//! created per call: each Rust consumer re-borrows the view instead of
//! keeping the object Java's consumer keeps, so one `lastDoc` per spans is
//! the one `lastDoc` per kept view of the Java consumers.

use std::fmt;

use rucene::error::Result;
use rucene::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use rucene::search::two_phase_iterator::TwoPhaseIterator;
use rucene::util::java_fmt::float_to_string;
use rucene_queries::spans::span_collector::SpanCollector;
use rucene_queries::spans::spans::{Spans, NO_MORE_POSITIONS};

use super::{assertion_error, throw_assertion};

const TWO_PHASE: &str = "INVARIANT: the wrapped spans answered a two-phase view";

/// Tracks current state of this spans.
///
/// Equivalent to the enum `AssertingSpans.State`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Document iteration has not yet begun (`docID()` = -1).
    DocStart,
    /// Two-phase iterator has moved to a new docid, but
    /// `TwoPhaseIterator.matches()` has not been called or it returned false
    /// (so you should not do things with the enum).
    DocUnverified,
    /// Iterator set to a valid docID, but position iteration has not yet
    /// begun (`startPosition() == -1`).
    PosStart,
    /// Iterator set to a valid docID, and positioned
    /// (-1 < `startPosition()` < `NO_MORE_POSITIONS`).
    Iterating,
    /// Positions exhausted (`startPosition()` = `NO_MORE_POSITIONS`).
    PosFinished,
    /// Documents exhausted (`docID()` = `NO_MORE_DOCS`).
    DocFinished,
}

impl fmt::Display for State {
    /// The Java constant name, as the assertion messages print it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            State::DocStart => "DOC_START",
            State::DocUnverified => "DOC_UNVERIFIED",
            State::PosStart => "POS_START",
            State::Iterating => "ITERATING",
            State::PosFinished => "POS_FINISHED",
            State::DocFinished => "DOC_FINISHED",
        })
    }
}

/// The fields `in`, `doc` and `state` of Java's `AssertingSpans`, shared by
/// the spans and their approximation view; as a [`DocIdSetIterator`] it is
/// the approximation view, Java's inner class `AssertingDISI`.
struct Core {
    /// `in`.
    in_spans: Box<dyn Spans>,
    /// `doc`.
    doc: i32,
    /// `state`.
    state: State,
}

impl Core {
    /// `"" + in`.
    fn in_str(&self) -> String {
        self.in_spans.java_to_string()
    }

    /// `AssertingSpans.this.docID()`.
    fn spans_doc_id(&self) -> i32 {
        let doc = self.in_spans.doc_id();
        java_assert_unchecked!(
            doc == self.doc,
            "broken docID() impl: docID() = {doc}, but next/advance last returned: {}: {}",
            self.doc,
            self.in_str()
        );
        doc
    }

    /// The approximation `AssertingDISI` filters: the wrapped spans'
    /// `asTwoPhaseIterator().approximation()`.
    fn approximation(&mut self) -> &mut dyn DocIdSetIterator {
        self.in_spans
            .as_two_phase_iterator()
            .expect(TWO_PHASE)
            .approximation()
    }

    fn approximation_ref(&self) -> &dyn DocIdSetIterator {
        self.in_spans
            .as_two_phase_iterator_ref()
            .expect(TWO_PHASE)
            .approximation_ref()
    }

    /// `"" + in` of `AssertingDISI` (its filtered approximation).
    fn approximation_str(&self) -> String {
        self.approximation_ref().java_to_string()
    }
}

/// `AssertingSpans.AssertingDISI`.
impl DocIdSetIterator for Core {
    fn doc_id(&self) -> i32 {
        let doc = self.approximation_ref().doc_id();
        java_assert_unchecked!(doc == self.spans_doc_id());
        doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        java_assert!(
            self.state != State::DocFinished,
            "nextDoc() called after NO_MORE_DOCS: {}",
            self.approximation_str()
        );
        let next_doc = self.approximation().next_doc()?;
        java_assert!(
            next_doc > self.doc,
            "backwards nextDoc from {} to {next_doc}: {}",
            self.doc,
            self.approximation_str()
        );
        if next_doc == NO_MORE_DOCS {
            self.state = State::DocFinished;
        } else {
            self.state = State::DocUnverified;
        }
        self.doc = next_doc;
        Ok(self.doc_id())
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        java_assert!(
            self.state != State::DocFinished,
            "advance() called after NO_MORE_DOCS: {}",
            self.approximation_str()
        );
        java_assert!(
            target > self.doc,
            "target must be > docID(), got {target} <= {}: {}",
            self.doc,
            self.approximation_str()
        );
        let advanced = self.approximation().advance(target)?;
        java_assert!(
            advanced >= target,
            "backwards advance from: {target} to: {advanced}: {}",
            self.approximation_str()
        );
        if advanced == NO_MORE_DOCS {
            self.state = State::DocFinished;
        } else {
            self.state = State::DocUnverified;
        }
        self.doc = advanced;
        Ok(self.doc_id())
    }

    fn cost(&self) -> i64 {
        self.approximation_ref().cost()
    }
}

/// Wraps a `Spans` with additional asserts.
///
/// Equivalent to the package-private class
/// `org.apache.lucene.queries.spans.AssertingSpans`.
pub struct AssertingSpans {
    core: Core,
    /// `AssertingTwoPhaseView.lastDoc` (see the module documentation).
    last_doc: i32,
}

impl AssertingSpans {
    /// `AssertingSpans(Spans in)`.
    pub fn new(in_spans: Box<dyn Spans>) -> Self {
        Self {
            core: Core {
                in_spans,
                doc: -1,
                state: State::DocStart,
            },
            last_doc: -1,
        }
    }

    /// The package-private `state` field.
    pub fn state(&self) -> State {
        self.core.state
    }

    /// The message of the first failing assertion of
    /// `checkCurrentPositions()`, if any.
    fn current_positions_failure(&self) -> Option<String> {
        if !cfg!(debug_assertions) {
            return None;
        }
        let start = self.core.in_spans.start_position();
        let end = self.core.in_spans.end_position();
        let state = self.core.state;
        let in_str = || self.core.in_str();
        if state == State::DocStart || state == State::DocUnverified || state == State::PosStart {
            if start != -1 {
                return Some(format!(
                    "invalid startPosition (should be -1): {}",
                    in_str()
                ));
            }
            if end != -1 {
                return Some(format!("invalid endPosition (should be -1): {}", in_str()));
            }
        } else if state == State::PosFinished {
            if start != NO_MORE_POSITIONS {
                return Some(format!(
                    "invalid startPosition (should be NO_MORE_POSITIONS): {}",
                    in_str()
                ));
            }
            if end != NO_MORE_POSITIONS {
                return Some(format!(
                    "invalid endPosition (should be NO_MORE_POSITIONS): {}",
                    in_str()
                ));
            }
        } else {
            if start < 0 {
                return Some(format!("invalid startPosition (negative): {}", in_str()));
            }
            if start > end {
                return Some(format!(
                    "invalid startPosition (> endPosition): {}",
                    in_str()
                ));
            }
        }
        None
    }

    /// `checkCurrentPositions()` in a method that returns a `Result`.
    fn check_current_positions(&self) -> Result<()> {
        match self.current_positions_failure() {
            Some(message) => Err(assertion_error(message)),
            None => Ok(()),
        }
    }

    /// `checkCurrentPositions()` in a method that cannot fail.
    fn check_current_positions_unchecked(&self) {
        if let Some(message) = self.current_positions_failure() {
            throw_assertion(message)
        }
    }
}

impl DocIdSetIterator for AssertingSpans {
    fn java_to_string(&self) -> String {
        format!("Asserting({})", self.core.in_str())
    }

    fn doc_id(&self) -> i32 {
        self.core.spans_doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        java_assert!(
            self.core.state != State::DocFinished,
            "nextDoc() called after NO_MORE_DOCS: {}",
            self.core.in_str()
        );
        let next_doc = self.core.in_spans.next_doc()?;
        java_assert!(
            next_doc > self.core.doc,
            "backwards nextDoc from {} to {next_doc}: {}",
            self.core.doc,
            self.core.in_str()
        );
        if next_doc == NO_MORE_DOCS {
            self.core.state = State::DocFinished;
        } else {
            java_assert!(
                self.core.in_spans.start_position() == -1,
                "invalid initial startPosition() [should be -1]: {}",
                self.core.in_str()
            );
            java_assert!(
                self.core.in_spans.end_position() == -1,
                "invalid initial endPosition() [should be -1]: {}",
                self.core.in_str()
            );
            self.core.state = State::PosStart;
        }
        self.core.doc = next_doc;
        Ok(self.doc_id())
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        java_assert!(
            self.core.state != State::DocFinished,
            "advance() called after NO_MORE_DOCS: {}",
            self.core.in_str()
        );
        java_assert!(
            target > self.core.doc,
            "target must be > docID(), got {target} <= {}: {}",
            self.core.doc,
            self.core.in_str()
        );
        let advanced = self.core.in_spans.advance(target)?;
        java_assert!(
            advanced >= target,
            "backwards advance from: {target} to: {advanced}: {}",
            self.core.in_str()
        );
        if advanced == NO_MORE_DOCS {
            self.core.state = State::DocFinished;
        } else {
            java_assert!(
                self.core.in_spans.start_position() == -1,
                "invalid initial startPosition() [should be -1]: {}",
                self.core.in_str()
            );
            java_assert!(
                self.core.in_spans.end_position() == -1,
                "invalid initial endPosition() [should be -1]: {}",
                self.core.in_str()
            );
            self.core.state = State::PosStart;
        }
        self.core.doc = advanced;
        Ok(self.doc_id())
    }

    fn cost(&self) -> i64 {
        self.core.in_spans.cost()
    }
}

impl Spans for AssertingSpans {
    fn next_start_position(&mut self) -> Result<i32> {
        let state = self.core.state;
        java_assert!(
            state != State::DocStart,
            "invalid position access, state={state}: {}",
            self.core.in_str()
        );
        java_assert!(
            state != State::DocFinished,
            "invalid position access, state={state}: {}",
            self.core.in_str()
        );
        java_assert!(
            state != State::DocUnverified,
            "invalid position access, state={state}: {}",
            self.core.in_str()
        );

        self.check_current_positions()?;

        // move to next position
        let prev = self.core.in_spans.start_position();
        let start = self.core.in_spans.next_start_position()?;
        java_assert!(
            start >= prev,
            "invalid startPosition (positions went backwards, previous={prev}): {}",
            self.core.in_str()
        );

        // transition state if necessary
        if start == NO_MORE_POSITIONS {
            self.core.state = State::PosFinished;
        } else {
            self.core.state = State::Iterating;
        }

        // check new positions
        self.check_current_positions()?;
        Ok(start)
    }

    fn start_position(&self) -> i32 {
        self.check_current_positions_unchecked();
        self.core.in_spans.start_position()
    }

    fn end_position(&self) -> i32 {
        self.check_current_positions_unchecked();
        self.core.in_spans.end_position()
    }

    fn width(&self) -> i32 {
        java_assert_unchecked!(self.core.state == State::Iterating);
        let distance = self.core.in_spans.width();
        java_assert_unchecked!(distance >= 0);
        distance
    }

    fn collect(&mut self, collector: &mut dyn SpanCollector) -> Result<()> {
        java_assert!(
            self.core.state == State::Iterating,
            "collect() called in illegal state: {}: {}",
            self.core.state,
            self.core.in_str()
        );
        self.core.in_spans.collect(collector)
    }

    fn positions_cost(&self) -> f32 {
        let cost = self.core.in_spans.positions_cost();
        java_assert_unchecked!(!cost.is_nan(), "positionsCost() should not be NaN");
        java_assert_unchecked!(cost > 0.0, "positionsCost() must be positive");
        cost
    }

    fn as_two_phase_iterator(&mut self) -> Option<&mut dyn TwoPhaseIterator> {
        self.core.in_spans.as_two_phase_iterator()?;
        Some(self)
    }

    fn as_two_phase_iterator_ref(&self) -> Option<&dyn TwoPhaseIterator> {
        self.core.in_spans.as_two_phase_iterator_ref()?;
        Some(self)
    }
}

/// `AssertingSpans.AssertingTwoPhaseView`.
impl TwoPhaseIterator for AssertingSpans {
    fn approximation(&mut self) -> &mut dyn DocIdSetIterator {
        &mut self.core
    }

    fn approximation_ref(&self) -> &dyn DocIdSetIterator {
        &self.core
    }

    fn matches(&mut self) -> Result<bool> {
        let doc = self.core.doc_id();
        if doc == -1 || doc == NO_MORE_DOCS {
            return Err(assertion_error(format!(
                "matches() should not be called on doc ID {doc}"
            )));
        }
        if self.last_doc == doc {
            return Err(assertion_error(format!(
                "matches() has been called twice on doc ID {doc}"
            )));
        }
        self.last_doc = doc;
        let v = self
            .core
            .in_spans
            .as_two_phase_iterator()
            .expect(TWO_PHASE)
            .matches()?;
        if v {
            self.core.state = State::PosStart;
        }
        Ok(v)
    }

    fn match_cost(&self) -> f32 {
        let cost = self
            .core
            .in_spans
            .as_two_phase_iterator_ref()
            .expect(TWO_PHASE)
            .match_cost();
        if cost.is_nan() {
            throw_assertion(format!(
                "matchCost()={} should not be NaN on doc ID {}",
                float_to_string(cost),
                self.core.doc_id()
            ));
        }
        if cost < 0.0 {
            throw_assertion(format!(
                "matchCost()={} should be non negative on doc ID {}",
                float_to_string(cost),
                self.core.doc_id()
            ));
        }
        cost
    }
}
