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

//! Port of `Spans.java`, of the Java package `org.apache.lucene.queries.spans`,
//! together with the shared handles this port needs where Java aliases one
//! [`Spans`] object from several places.

use std::cell::RefCell;
use std::rc::Rc;

use rucene::error::Result;
use rucene::search::doc_id_set_iterator::DocIdSetIterator;
use rucene::search::two_phase_iterator::TwoPhaseIterator;
use rucene::util::FixedBitSet;

use crate::spans::span_collector::SpanCollector;

/// The value [`Spans::next_start_position`], [`Spans::start_position`] and
/// [`Spans::end_position`] return after the last start/end position at the
/// current document.
///
/// Equivalent to `Spans.NO_MORE_POSITIONS`, `Integer.MAX_VALUE`.
pub const NO_MORE_POSITIONS: i32 = i32::MAX;

/// Iterates through combinations of start/end positions per-doc.
///
/// Equivalent to `org.apache.lucene.queries.spans.Spans`.
///
/// Each start/end position represents a range of term positions within the
/// current document. These are enumerated in order, by increasing document
/// number, within that by increasing start position and finally by increasing
/// end position.
///
/// Java's `toString()` is [`DocIdSetIterator::java_to_string`], which every
/// implementation overrides with the text its Java class renders; the default
/// rendering of `Spans.toString()` is [`spans_to_string`].
///
/// **Divergence from Lucene 10.5.0.** Java's `asTwoPhaseIterator()`
/// (`Spans.java:88-90`) returns a new object that holds the approximation and
/// calls back into the spans. A
/// Rust object cannot alias the spans it views, so the view is a borrow of the
/// spans itself, as [`Scorer::two_phase_iterator`](rucene::search::Scorer::two_phase_iterator)
/// is; and, as [`TwoPhaseIterator::approximation_ref`] does, the shared-borrow
/// sibling [`as_two_phase_iterator_ref`](Self::as_two_phase_iterator_ref) is
/// declared beside it. Both must answer the same view.
pub trait Spans: DocIdSetIterator {
    /// Returns the next start position for the current doc.
    ///
    /// Equivalent to `Spans.nextStartPosition()`. There is always at least one
    /// start/end position per doc. After the last start/end position at the
    /// current doc this returns [`NO_MORE_POSITIONS`].
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading positions.
    fn next_start_position(&mut self) -> Result<i32>;

    /// Returns the start position in the current doc, or `-1` when
    /// [`next_start_position`](Self::next_start_position) was not yet called on
    /// the current doc.
    ///
    /// Equivalent to `Spans.startPosition()`. After the last start/end position
    /// at the current doc this returns [`NO_MORE_POSITIONS`].
    fn start_position(&self) -> i32;

    /// Returns the end position for the current start position, or `-1` when
    /// [`next_start_position`](Self::next_start_position) was not yet called on
    /// the current doc.
    ///
    /// Equivalent to `Spans.endPosition()`. After the last start/end position
    /// at the current doc this returns [`NO_MORE_POSITIONS`].
    fn end_position(&self) -> i32;

    /// Returns the width of the match, which is typically used to compute a
    /// sloppy frequency.
    ///
    /// Equivalent to `Spans.width()`. It is only legal to call this method when
    /// the iterator is on a valid doc ID and positioned. The return value must
    /// be positive, and lower values means that the match is better.
    fn width(&self) -> i32;

    /// Collects postings data from the leaves of the current spans.
    ///
    /// Equivalent to `Spans.collect(SpanCollector)`. This method should only be
    /// called after [`next_start_position`](Self::next_start_position), and
    /// before [`NO_MORE_POSITIONS`] has been reached.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error the collector raises.
    fn collect(&mut self, collector: &mut dyn SpanCollector) -> Result<()>;

    /// Returns an estimation of the cost of using the positions of these spans
    /// for any single document, but only after
    /// [`as_two_phase_iterator`](Self::as_two_phase_iterator) returned `None`.
    ///
    /// Equivalent to `Spans.positionsCost()`. Otherwise this method should not
    /// be called. The returned value is independent of the current document.
    fn positions_cost(&self) -> f32;

    /// Returns a [`TwoPhaseIterator`] view of these spans, or `None` when
    /// two-phase iteration is not supported.
    ///
    /// Equivalent to `Spans.asTwoPhaseIterator()`, which returns `null` by
    /// default.
    fn as_two_phase_iterator(&mut self) -> Option<&mut dyn TwoPhaseIterator> {
        None
    }

    /// The shared-borrow sibling of
    /// [`as_two_phase_iterator`](Self::as_two_phase_iterator); see the trait
    /// documentation for why it exists.
    fn as_two_phase_iterator_ref(&self) -> Option<&dyn TwoPhaseIterator> {
        None
    }

    /// Called before the current doc's frequency is calculated.
    ///
    /// Equivalent to the protected `Spans.doStartCurrentDoc()`, which does
    /// nothing by default.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error an implementation raises.
    fn do_start_current_doc(&mut self) -> Result<()> {
        Ok(())
    }

    /// Called each time the scorer's [`SpanScorer`](crate::spans::span_scorer::SpanScorer)
    /// is advanced during frequency calculation.
    ///
    /// Equivalent to the protected `Spans.doCurrentSpans()`, which does nothing
    /// by default.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error an implementation raises.
    fn do_current_spans(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Renders spans the way `Spans.toString()` does:
/// `name(doc=…,start=…,end=…)`.
///
/// Equivalent to `Spans.toString()`, where `name` is
/// `clazz.isAnonymousClass() ? clazz.getName() : clazz.getSimpleName()` of the
/// spans' Java class; each implementation passes the name of its own class.
pub fn spans_to_string<S: Spans + ?Sized>(name: &str, spans: &S) -> String {
    format!(
        "{name}(doc={},start={},end={})",
        spans.doc_id(),
        spans.start_position(),
        spans.end_position()
    )
}

/// Throws the `UnsupportedOperationException` with which the spans that always
/// support two-phase iteration answer [`Spans::positions_cost`]
/// (`ConjunctionSpans.java:106-108`, `FilterSpans.java:175-177`).
pub(crate) fn positions_cost_unsupported() -> ! {
    rucene::store::java_nio_file::throw_unchecked("java.lang.UnsupportedOperationException", None)
}

impl Spans for Box<dyn Spans> {
    fn next_start_position(&mut self) -> Result<i32> {
        (**self).next_start_position()
    }

    fn start_position(&self) -> i32 {
        (**self).start_position()
    }

    fn end_position(&self) -> i32 {
        (**self).end_position()
    }

    fn width(&self) -> i32 {
        (**self).width()
    }

    fn collect(&mut self, collector: &mut dyn SpanCollector) -> Result<()> {
        (**self).collect(collector)
    }

    fn positions_cost(&self) -> f32 {
        (**self).positions_cost()
    }

    fn as_two_phase_iterator(&mut self) -> Option<&mut dyn TwoPhaseIterator> {
        (**self).as_two_phase_iterator()
    }

    fn as_two_phase_iterator_ref(&self) -> Option<&dyn TwoPhaseIterator> {
        (**self).as_two_phase_iterator_ref()
    }

    fn do_start_current_doc(&mut self) -> Result<()> {
        (**self).do_start_current_doc()
    }

    fn do_current_spans(&mut self) -> Result<()> {
        (**self).do_current_spans()
    }
}

/// Message used where a two-phase view is known to exist because it was
/// observed before the view was handed out.
const TWO_PHASE_INVARIANT: &str =
    "INVARIANT: a SharedSpansTwoPhase is only built over spans that support two-phase iteration";

/// A shared handle on one [`Spans`] object.
///
/// **This has no counterpart in Lucene 10.5.0.** Java keeps several references
/// to the same spans: a `ConjunctionSpans`
/// holds its sub-spans both in its `subSpans` array and inside the
/// `ConjunctionDISI` that intersects them, and the disjunction of
/// [`SpanOrQuery`](crate::spans::span_or_query::SpanOrQuery) holds each
/// sub-spans in a [`SpanDisiWrapper`](crate::spans::span_disi_wrapper::SpanDisiWrapper)
/// and in its position queue. Rust forbids that aliasing, so the spans is put
/// behind this handle — the shape of the core's
/// [`SharedPostings`](rucene::search::phrase_matcher::SharedPostings) — and every
/// clone reaches exactly the object Java's references point at.
#[derive(Clone)]
pub struct SharedSpans {
    inner: Rc<RefCell<Box<dyn Spans>>>,
    view: SharedSpansTwoPhase,
}

impl std::fmt::Debug for SharedSpans {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.inner.borrow().java_to_string())
    }
}

impl SharedSpans {
    /// Shares the given spans.
    pub fn new(spans: Box<dyn Spans>) -> Self {
        let inner = Rc::new(RefCell::new(spans));
        let view = SharedSpansTwoPhase::new(Rc::clone(&inner));
        Self { inner, view }
    }

    /// Returns whether both handles share the same spans: Java's `==`.
    pub fn ptr_eq(&self, other: &SharedSpans) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }

    /// Runs `f` with the shared spans borrowed mutably.
    pub fn with_mut<R>(&self, f: impl FnOnce(&mut dyn Spans) -> R) -> R {
        f(&mut **self.inner.borrow_mut())
    }

    /// Runs `f` with the shared spans borrowed immutably.
    pub fn with_ref<R>(&self, f: impl FnOnce(&dyn Spans) -> R) -> R {
        f(&**self.inner.borrow())
    }

    /// The shared spans' current doc ID; `Spans.docID()`.
    pub fn doc_id(&self) -> i32 {
        self.inner.borrow().doc_id()
    }

    /// Advances the shared spans to the next doc; `Spans.nextDoc()`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while advancing.
    pub fn next_doc(&self) -> Result<i32> {
        self.inner.borrow_mut().next_doc()
    }

    /// Advances the shared spans to `target`; `Spans.advance(int)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while advancing.
    pub fn advance(&self, target: i32) -> Result<i32> {
        self.inner.borrow_mut().advance(target)
    }

    /// The shared spans' cost; `Spans.cost()`.
    pub fn cost(&self) -> i64 {
        self.inner.borrow().cost()
    }

    /// `Spans.nextStartPosition()` on the shared spans.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading positions.
    pub fn next_start_position(&self) -> Result<i32> {
        self.inner.borrow_mut().next_start_position()
    }

    /// `Spans.startPosition()` on the shared spans.
    pub fn start_position(&self) -> i32 {
        self.inner.borrow().start_position()
    }

    /// `Spans.endPosition()` on the shared spans.
    pub fn end_position(&self) -> i32 {
        self.inner.borrow().end_position()
    }

    /// `Spans.width()` on the shared spans.
    pub fn width(&self) -> i32 {
        self.inner.borrow().width()
    }

    /// `Spans.collect(SpanCollector)` on the shared spans.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error the collector raises.
    pub fn collect(&self, collector: &mut dyn SpanCollector) -> Result<()> {
        self.inner.borrow_mut().collect(collector)
    }

    /// `Spans.positionsCost()` on the shared spans.
    pub fn positions_cost(&self) -> f32 {
        self.inner.borrow().positions_cost()
    }

    /// Returns the two-phase view of the shared spans, or `None` when they do
    /// not support two-phase iteration.
    ///
    /// Equivalent to keeping the `TwoPhaseIterator` that
    /// `spans.asTwoPhaseIterator()` returns: the view is itself a shared
    /// handle, so it can be stored beside the spans, as Java stores it.
    pub fn two_phase_view(&self) -> Option<SharedSpansTwoPhase> {
        if self.inner.borrow_mut().as_two_phase_iterator().is_some() {
            Some(self.view.clone())
        } else {
            None
        }
    }

    /// `spans.asTwoPhaseIterator() != null`, asked without a mutable borrow.
    pub fn has_two_phase_iterator(&self) -> bool {
        self.inner.borrow().as_two_phase_iterator_ref().is_some()
    }

    /// `Spans.toString()` of the shared spans.
    pub fn spans_to_string(&self) -> String {
        self.inner.borrow().java_to_string()
    }
}

impl DocIdSetIterator for SharedSpans {
    fn java_to_string(&self) -> String {
        self.inner.borrow().java_to_string()
    }

    fn doc_id(&self) -> i32 {
        SharedSpans::doc_id(self)
    }

    fn next_doc(&mut self) -> Result<i32> {
        SharedSpans::next_doc(self)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        SharedSpans::advance(self, target)
    }

    fn cost(&self) -> i64 {
        SharedSpans::cost(self)
    }

    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        self.inner.borrow_mut().into_bit_set(up_to, bit_set, offset)
    }

    fn doc_id_run_end(&mut self) -> Result<i32> {
        self.inner.borrow_mut().doc_id_run_end()
    }
}

impl Spans for SharedSpans {
    fn next_start_position(&mut self) -> Result<i32> {
        SharedSpans::next_start_position(self)
    }

    fn start_position(&self) -> i32 {
        SharedSpans::start_position(self)
    }

    fn end_position(&self) -> i32 {
        SharedSpans::end_position(self)
    }

    fn width(&self) -> i32 {
        SharedSpans::width(self)
    }

    fn collect(&mut self, collector: &mut dyn SpanCollector) -> Result<()> {
        SharedSpans::collect(self, collector)
    }

    fn positions_cost(&self) -> f32 {
        SharedSpans::positions_cost(self)
    }

    fn as_two_phase_iterator(&mut self) -> Option<&mut dyn TwoPhaseIterator> {
        if self.inner.borrow_mut().as_two_phase_iterator().is_some() {
            Some(&mut self.view)
        } else {
            None
        }
    }

    fn as_two_phase_iterator_ref(&self) -> Option<&dyn TwoPhaseIterator> {
        if self.has_two_phase_iterator() {
            Some(&self.view)
        } else {
            None
        }
    }

    fn do_start_current_doc(&mut self) -> Result<()> {
        self.inner.borrow_mut().do_start_current_doc()
    }

    fn do_current_spans(&mut self) -> Result<()> {
        self.inner.borrow_mut().do_current_spans()
    }
}

/// The approximation of the two-phase view of a [`SharedSpans`].
///
/// **This has no counterpart in Lucene 10.5.0.** It is the object
/// `spans.asTwoPhaseIterator().approximation()` returns, reached through the
/// shared handle: every call borrows the spans and forwards to the
/// approximation of their two-phase view.
#[derive(Clone)]
pub struct SharedSpansApproximation {
    inner: Rc<RefCell<Box<dyn Spans>>>,
}

impl std::fmt::Debug for SharedSpansApproximation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SharedSpansApproximation")
    }
}

impl DocIdSetIterator for SharedSpansApproximation {
    fn java_to_string(&self) -> String {
        self.inner
            .borrow_mut()
            .as_two_phase_iterator()
            .expect(TWO_PHASE_INVARIANT)
            .approximation_ref()
            .java_to_string()
    }

    fn doc_id(&self) -> i32 {
        self.inner
            .borrow_mut()
            .as_two_phase_iterator()
            .expect(TWO_PHASE_INVARIANT)
            .approximation_ref()
            .doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.inner
            .borrow_mut()
            .as_two_phase_iterator()
            .expect(TWO_PHASE_INVARIANT)
            .approximation()
            .next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.inner
            .borrow_mut()
            .as_two_phase_iterator()
            .expect(TWO_PHASE_INVARIANT)
            .approximation()
            .advance(target)
    }

    fn cost(&self) -> i64 {
        self.inner
            .borrow_mut()
            .as_two_phase_iterator()
            .expect(TWO_PHASE_INVARIANT)
            .approximation_ref()
            .cost()
    }
}

/// The two-phase view of a [`SharedSpans`].
///
/// **This has no counterpart in Lucene 10.5.0.** It is the
/// `TwoPhaseIterator` that `spans.asTwoPhaseIterator()` returns, reached
/// through the shared handle, so that it can be stored — in a conjunction, in a
/// [`SpanDisiWrapper`](crate::spans::span_disi_wrapper::SpanDisiWrapper) — beside
/// the spans it views.
#[derive(Clone)]
pub struct SharedSpansTwoPhase {
    inner: Rc<RefCell<Box<dyn Spans>>>,
    approximation: SharedSpansApproximation,
}

impl std::fmt::Debug for SharedSpansTwoPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SharedSpansTwoPhase")
    }
}

impl SharedSpansTwoPhase {
    fn new(inner: Rc<RefCell<Box<dyn Spans>>>) -> Self {
        let approximation = SharedSpansApproximation {
            inner: Rc::clone(&inner),
        };
        Self {
            inner,
            approximation,
        }
    }

    /// Returns the approximation as an owned shared handle, for the callers
    /// that store it (`SpanDisiWrapper.approximation`).
    pub fn approximation_handle(&self) -> SharedSpansApproximation {
        self.approximation.clone()
    }
}

impl TwoPhaseIterator for SharedSpansTwoPhase {
    fn approximation(&mut self) -> &mut dyn DocIdSetIterator {
        &mut self.approximation
    }

    fn approximation_ref(&self) -> &dyn DocIdSetIterator {
        &self.approximation
    }

    fn matches(&mut self) -> Result<bool> {
        self.inner
            .borrow_mut()
            .as_two_phase_iterator()
            .expect(TWO_PHASE_INVARIANT)
            .matches()
    }

    fn match_cost(&self) -> f32 {
        self.inner
            .borrow()
            .as_two_phase_iterator_ref()
            .expect(TWO_PHASE_INVARIANT)
            .match_cost()
    }

    fn doc_id_run_end(&mut self) -> Result<i32> {
        self.inner
            .borrow_mut()
            .as_two_phase_iterator()
            .expect(TWO_PHASE_INVARIANT)
            .doc_id_run_end()
    }
}
