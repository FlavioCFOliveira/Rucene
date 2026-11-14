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

//! Port of `FilterSpans.java`, of the Java package
//! `org.apache.lucene.queries.spans`.

use rucene::error::Result;
use rucene::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use rucene::search::two_phase_iterator::TwoPhaseIterator;

use crate::spans::span_collector::SpanCollector;
use crate::spans::spans::{positions_cost_unsupported, Spans, NO_MORE_POSITIONS};

/// Message used where the wrapped spans' two-phase view was observed to exist
/// a moment before.
const IN_TWO_PHASE_INVARIANT: &str =
    "INVARIANT: the wrapped spans answered a two-phase view on the previous call";

/// The members a subclass of [`FilterSpans`] implements.
///
/// Equivalent to the abstract method `FilterSpans.accept(Spans)` together with
/// the two protected hooks of [`Spans`] a subclass may override —
/// `doStartCurrentDoc()` and `doCurrentSpans()`, which
/// `PayloadScoreQuery.PayloadSpans` overrides. Java's anonymous subclasses
/// become implementations of this trait, which [`FilterSpans`] holds.
pub trait FilterSpansAccept {
    /// Returns [`AcceptStatus::Yes`] if the candidate should be an accepted
    /// match, [`AcceptStatus::No`] if it should not, and
    /// [`AcceptStatus::NoMoreInCurrentDoc`] if iteration should move on to the
    /// next document.
    ///
    /// Equivalent to the protected abstract `FilterSpans.accept(Spans)`; the
    /// candidate is always the wrapped spans.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while inspecting the candidate.
    fn accept(&mut self, candidate: &mut dyn Spans) -> Result<AcceptStatus>;

    /// Equivalent to an override of `Spans.doStartCurrentDoc()` in a subclass.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error an implementation raises.
    fn do_start_current_doc(&mut self) -> Result<()> {
        Ok(())
    }

    /// Equivalent to an override of `Spans.doCurrentSpans()` in a subclass,
    /// which reaches the wrapped spans through the `in` field; it is passed
    /// here.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error an implementation raises.
    fn do_current_spans(&mut self, _in_spans: &mut dyn Spans) -> Result<()> {
        Ok(())
    }
}

/// A [`Spans`] implementation wrapping another spans instance, allowing to
/// filter spans matches easily by implementing
/// [`FilterSpansAccept::accept`].
///
/// Equivalent to the abstract class `org.apache.lucene.queries.spans.FilterSpans`.
/// The abstract members are the [`FilterSpansAccept`] the filter holds.
pub struct FilterSpans<A: FilterSpansAccept> {
    /// The wrapped spans instance; the protected final `in` field.
    in_spans: Box<dyn Spans>,
    at_first_in_current_doc: bool,
    start_pos: i32,
    filter: A,
}

impl<A: FilterSpansAccept> FilterSpans<A> {
    /// Wraps the given [`Spans`].
    ///
    /// Equivalent to the protected `FilterSpans(Spans)`.
    pub fn new(in_spans: Box<dyn Spans>, filter: A) -> Self {
        Self {
            in_spans,
            at_first_in_current_doc: false,
            start_pos: -1,
            filter,
        }
    }

    /// Returns the wrapped spans; the protected `in` field.
    pub fn in_spans(&self) -> &dyn Spans {
        &*self.in_spans
    }

    /// Returns the subclass members this filter holds.
    pub fn filter(&self) -> &A {
        &self.filter
    }

    /// Returns the subclass members this filter holds, mutably.
    pub fn filter_mut(&mut self) -> &mut A {
        &mut self.filter
    }

    /// Returns `true` if the current document matches.
    ///
    /// Equivalent to the private final `FilterSpans.twoPhaseCurrentDocMatches()`,
    /// called during two-phase processing.
    fn two_phase_current_doc_matches(&mut self) -> Result<bool> {
        self.at_first_in_current_doc = false;
        self.start_pos = self.in_spans.next_start_position()?;
        debug_assert!(self.start_pos != NO_MORE_POSITIONS);
        loop {
            match self.filter.accept(&mut *self.in_spans)? {
                AcceptStatus::Yes => {
                    self.at_first_in_current_doc = true;
                    return Ok(true);
                }
                AcceptStatus::No => {
                    self.start_pos = self.in_spans.next_start_position()?;
                    if self.start_pos != NO_MORE_POSITIONS {
                        continue;
                    }
                    // else fallthrough
                    self.start_pos = -1;
                    return Ok(false);
                }
                AcceptStatus::NoMoreInCurrentDoc => {
                    self.start_pos = -1;
                    return Ok(false);
                }
            }
        }
    }
}

impl<A: FilterSpansAccept> DocIdSetIterator for FilterSpans<A> {
    /// Equivalent to `FilterSpans.toString()`, `"Filter(" + in + ")"`.
    fn java_to_string(&self) -> String {
        format!("Filter({})", self.in_spans.java_to_string())
    }

    fn doc_id(&self) -> i32 {
        self.in_spans.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        loop {
            let doc = self.in_spans.next_doc()?;
            if doc == NO_MORE_DOCS {
                return Ok(NO_MORE_DOCS);
            } else if self.two_phase_current_doc_matches()? {
                return Ok(doc);
            }
        }
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let mut doc = self.in_spans.advance(target)?;
        while doc != NO_MORE_DOCS {
            if self.two_phase_current_doc_matches()? {
                break;
            }
            doc = self.in_spans.next_doc()?;
        }

        Ok(doc)
    }

    fn cost(&self) -> i64 {
        self.in_spans.cost()
    }
}

impl<A: FilterSpansAccept> Spans for FilterSpans<A> {
    fn next_start_position(&mut self) -> Result<i32> {
        if self.at_first_in_current_doc {
            self.at_first_in_current_doc = false;
            return Ok(self.start_pos);
        }

        loop {
            self.start_pos = self.in_spans.next_start_position()?;
            if self.start_pos == NO_MORE_POSITIONS {
                return Ok(NO_MORE_POSITIONS);
            }
            match self.filter.accept(&mut *self.in_spans)? {
                AcceptStatus::Yes => return Ok(self.start_pos),
                AcceptStatus::No => {}
                AcceptStatus::NoMoreInCurrentDoc => {
                    self.start_pos = NO_MORE_POSITIONS; // startPos ahead for the current doc.
                    return Ok(self.start_pos);
                }
            }
        }
    }

    fn start_position(&self) -> i32 {
        if self.at_first_in_current_doc {
            -1
        } else {
            self.start_pos
        }
    }

    fn end_position(&self) -> i32 {
        if self.at_first_in_current_doc {
            -1
        } else if self.start_pos != NO_MORE_POSITIONS {
            self.in_spans.end_position()
        } else {
            NO_MORE_POSITIONS
        }
    }

    fn width(&self) -> i32 {
        self.in_spans.width()
    }

    fn collect(&mut self, collector: &mut dyn SpanCollector) -> Result<()> {
        self.in_spans.collect(collector)
    }

    fn positions_cost(&self) -> f32 {
        positions_cost_unsupported() // asTwoPhaseIterator never returns null
    }

    /// Equivalent to the final `FilterSpans.asTwoPhaseIterator()`, which never
    /// returns `null`. The view's approximation is the wrapped spans'
    /// approximation when they have one ("underestimate" match cost), and the
    /// wrapped spans themselves otherwise ("overestimate"); see the
    /// [`TwoPhaseIterator`] implementation of this type.
    fn as_two_phase_iterator(&mut self) -> Option<&mut dyn TwoPhaseIterator> {
        Some(self)
    }

    fn as_two_phase_iterator_ref(&self) -> Option<&dyn TwoPhaseIterator> {
        Some(self)
    }

    fn do_start_current_doc(&mut self) -> Result<()> {
        self.filter.do_start_current_doc()
    }

    fn do_current_spans(&mut self) -> Result<()> {
        self.filter.do_current_spans(&mut *self.in_spans)
    }
}

/// The two anonymous `TwoPhaseIterator`s of `FilterSpans.asTwoPhaseIterator()`:
/// the first when the wrapped spans have an approximation, the second when they
/// do not, decided — as in Java — by asking the wrapped spans.
impl<A: FilterSpansAccept> TwoPhaseIterator for FilterSpans<A> {
    fn approximation(&mut self) -> &mut dyn DocIdSetIterator {
        if self.in_spans.as_two_phase_iterator().is_some() {
            // wrapped instance has an approximation
            self.in_spans
                .as_two_phase_iterator()
                .expect(IN_TWO_PHASE_INVARIANT)
                .approximation()
        } else {
            // wrapped instance has no approximation, but
            // we can still defer matching until absolutely needed.
            &mut self.in_spans
        }
    }

    fn approximation_ref(&self) -> &dyn DocIdSetIterator {
        match self.in_spans.as_two_phase_iterator_ref() {
            Some(inner) => inner.approximation_ref(),
            None => &self.in_spans,
        }
    }

    fn matches(&mut self) -> Result<bool> {
        let inner_matches = match self.in_spans.as_two_phase_iterator() {
            Some(inner) => Some(inner.matches()?),
            None => None,
        };
        match inner_matches {
            Some(inner_matches) => Ok(inner_matches && self.two_phase_current_doc_matches()?),
            None => self.two_phase_current_doc_matches(),
        }
    }

    fn match_cost(&self) -> f32 {
        match self.in_spans.as_two_phase_iterator_ref() {
            Some(inner) => inner.match_cost(),      // underestimate
            None => self.in_spans.positions_cost(), // overestimate
        }
    }
}

/// Status returned from [`FilterSpansAccept::accept`] that indicates whether a
/// candidate match should be accepted, rejected, or rejected and move on to the
/// next document.
///
/// Equivalent to `org.apache.lucene.queries.spans.FilterSpans.AcceptStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AcceptStatus {
    /// Java constant `YES`: the match should be accepted.
    Yes,
    /// Java constant `NO`: the match should be rejected.
    No,
    /// Java constant `NO_MORE_IN_CURRENT_DOC`: the match should be rejected,
    /// and the enumeration may continue with the next document.
    NoMoreInCurrentDoc,
}

impl AcceptStatus {
    /// Java's `AcceptStatus.toString()`, the constant's name.
    pub fn name(self) -> &'static str {
        match self {
            AcceptStatus::Yes => "YES",
            AcceptStatus::No => "NO",
            AcceptStatus::NoMoreInCurrentDoc => "NO_MORE_IN_CURRENT_DOC",
        }
    }
}
