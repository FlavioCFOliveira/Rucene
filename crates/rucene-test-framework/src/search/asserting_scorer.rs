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

//! Port of `org.apache.lucene.tests.search.AssertingScorer`.
//!
//! # Layout
//!
//! Java's scorer hands out two views that share its state: the
//! `FilterDocIdSetIterator` of `iterator()` and the `TwoPhaseIterator` of
//! `twoPhaseIterator()`, whose approximation is another filter. Rust's
//! [`Scorer::iterator`] and [`Scorer::two_phase_iterator`] return borrows, so
//! the views are nested fields: [`AssertingScorer`] is its own iterator view,
//! it owns the two-phase view, which owns the approximation view, which owns
//! the shared state ([`Core`]). Each view reaches the state through the fields
//! it owns, which is the same state Java's inner classes reach through
//! `AssertingScorer.this`.
//!
//! # Divergences
//!
//! Rust's [`DocIdSetIterator::doc_id`], [`DocIdSetIterator::cost`],
//! [`DocIdSetIterator::doc_id_run_end`] and [`TwoPhaseIterator::match_cost`]
//! take `&self`, while the wrapped scorer only lends its iterator and its
//! two-phase view through `&mut self`. So:
//!
//! * the iterator view's `docID()` returns the wrapped scorer's
//!   [`Scorer::doc_id`] and cannot assert Java's
//!   `AssertingScorer.this.in.docID() == in.docID()`; the two are the same
//!   position under the `Scorer` contract, and `nextDoc`/`advance` still
//!   assert it;
//! * the costs and the match cost are read from the wrapped iterator when the
//!   view is handed out (`wrap` for the iterator, `two_phase_iterator` for the
//!   approximation and the match cost), and the match cost assertions run when
//!   the cached value is read;
//! * the iterator view's `docIDRunEnd()` asserts the state as Java does, then
//!   returns [`DocIdSetIterator`]'s default `docID() + 1` instead of the
//!   wrapped iterator's run end, as the core's `ScorerAsIterator` does; the
//!   value is a valid run end, only a shorter one.
//!
//! The messages that embed the wrapped scorer's `toString()` (`" " + in`) end
//! with `<scorer>`: Rust's [`Scorer`] has no string form.

use std::any::TypeId;
use std::fmt;

use rucene::error::Result;
use rucene::index::DocAndFloatFeatureBuffer;
use rucene::search::{
    ChildScorable, DocIdSetIterator, Scorable, Scorer, TwoPhaseIterator, NO_MORE_DOCS,
};
use rucene::util::java_fmt::float_to_string;
use rucene::util::{Bits, FixedBitSet};

/// Rendering of the wrapped scorer in messages (see the module divergences).
const IN: &str = "<scorer>";

/// `AssertingScorer.IteratorState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IteratorState {
    /// `APPROXIMATING`.
    Approximating,
    /// `ITERATING`.
    Iterating,
    /// `SHALLOW_ADVANCING`.
    ShallowAdvancing,
    /// `FINISHED`.
    Finished,
}

impl fmt::Display for IteratorState {
    /// The Java constant name, as `assert …: state` prints it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            IteratorState::Approximating => "APPROXIMATING",
            IteratorState::Iterating => "ITERATING",
            IteratorState::ShallowAdvancing => "SHALLOW_ADVANCING",
            IteratorState::Finished => "FINISHED",
        })
    }
}

/// The fields of Java's `AssertingScorer`, shared by its views.
struct Core<'a> {
    /// `in`.
    inner: Box<dyn Scorer + 'a>,
    /// `canScore`.
    can_score: bool,
    /// `canSetMinCompetitiveScore`.
    can_set_min_competitive_score: bool,
    /// `state`.
    state: IteratorState,
    /// `doc`.
    doc: i32,
    /// `minCompetitiveScore`.
    min_competitive_score: f32,
    /// `lastShallowTarget`.
    last_shallow_target: i32,
    /// `in.iterator().cost()`, read by `wrap`.
    cost: i64,
}

/// The mutable fields of Java's `AssertingScorer` that a collector carries
/// from one loan of its scorer to the next (see
/// [`AssertingScorer::wrap_borrowed`]).
#[derive(Debug, Clone, Copy)]
pub(crate) struct AssertingScorerState {
    state: IteratorState,
    doc: i32,
    min_competitive_score: f32,
    last_shallow_target: i32,
}

/// A borrowed [`Scorer`] lent to an [`AssertingScorer`] for one collection
/// call: every call is forwarded.
struct BorrowedScorer<'a> {
    inner: &'a mut dyn Scorer,
}

impl Scorable for BorrowedScorer<'_> {
    fn score(&mut self) -> Result<f32> {
        self.inner.score()
    }

    fn smoothing_score(&mut self, doc_id: i32) -> Result<f32> {
        self.inner.smoothing_score(doc_id)
    }

    fn set_min_competitive_score(&mut self, min_score: f32) -> Result<()> {
        self.inner.set_min_competitive_score(min_score)
    }

    fn children(&mut self) -> Result<Vec<ChildScorable<'_>>> {
        self.inner.children()
    }

    fn as_scorer(&mut self) -> Option<&mut dyn Scorer> {
        Some(self)
    }

    fn class_id(&self) -> Option<TypeId> {
        self.inner.class_id()
    }
}

impl Scorer for BorrowedScorer<'_> {
    fn as_indri_scorer(&mut self) -> Option<&mut dyn rucene::search::indri_scorer::IndriScorer> {
        self.inner.as_indri_scorer()
    }

    fn doc_id(&self) -> i32 {
        self.inner.doc_id()
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        self.inner.iterator()
    }

    fn two_phase_iterator(&mut self) -> Option<&mut dyn TwoPhaseIterator> {
        self.inner.two_phase_iterator()
    }

    fn advance_shallow(&mut self, target: i32) -> Result<i32> {
        self.inner.advance_shallow(target)
    }

    fn get_max_score(&mut self, up_to: i32) -> Result<f32> {
        self.inner.get_max_score(up_to)
    }

    fn next_docs_and_scores(
        &mut self,
        up_to: i32,
        live_docs: Option<&dyn Bits>,
        buffer: &mut DocAndFloatFeatureBuffer,
    ) -> Result<()> {
        self.inner.next_docs_and_scores(up_to, live_docs, buffer)
    }
}

/// The approximation of the two-phase view: Java's anonymous
/// `FilterDocIdSetIterator` over `in.twoPhaseIterator().approximation()`.
struct AssertingApproximation<'a> {
    core: Core<'a>,
    /// `inApproximation.cost()`, read by `two_phase_iterator()`.
    cost: i64,
}

impl AssertingApproximation<'_> {
    fn in_approximation(&mut self) -> &mut dyn DocIdSetIterator {
        self.core
            .inner
            .two_phase_iterator()
            .expect("INVARIANT: the two-phase view is only handed out over a two-phase scorer")
            .approximation()
    }
}

impl DocIdSetIterator for AssertingApproximation<'_> {
    fn doc_id(&self) -> i32 {
        // `inApproximation.docID()`: the wrapped scorer's position, which a
        // two-phase scorer shares with its approximation.
        self.core.inner.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        java_assert!(
            self.core.state != IteratorState::Finished,
            "advance() called after NO_MORE_DOCS"
        );
        java_assert!(self.doc_id() + 1 >= self.core.last_shallow_target);
        let next_doc = self.in_approximation().next_doc()?;
        java_assert!(
            next_doc > self.core.doc,
            "backwards advance from: {} to: {}",
            self.core.doc,
            next_doc
        );
        self.core.state = if next_doc == NO_MORE_DOCS {
            IteratorState::Finished
        } else {
            IteratorState::Approximating
        };
        java_assert!(self.in_approximation().doc_id() == next_doc);
        self.core.doc = next_doc;
        Ok(next_doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        java_assert!(
            self.core.state != IteratorState::Finished,
            "advance() called after NO_MORE_DOCS"
        );
        java_assert!(
            target > self.core.doc,
            "target must be > docID(), got {} <= {}",
            target,
            self.core.doc
        );
        java_assert!(target >= self.core.last_shallow_target);
        let advanced = self.in_approximation().advance(target)?;
        java_assert!(
            advanced >= target,
            "backwards advance from: {} to: {}",
            target,
            advanced
        );
        self.core.state = if advanced == NO_MORE_DOCS {
            IteratorState::Finished
        } else {
            IteratorState::Approximating
        };
        java_assert!(self.in_approximation().doc_id() == advanced);
        self.core.doc = advanced;
        Ok(advanced)
    }

    fn cost(&self) -> i64 {
        self.cost
    }
}

/// Java's anonymous `TwoPhaseIterator` of `twoPhaseIterator()`.
struct AssertingTwoPhase<'a> {
    approximation: AssertingApproximation<'a>,
    /// `in.matchCost()`, read by `two_phase_iterator()`.
    match_cost: f32,
}

impl TwoPhaseIterator for AssertingTwoPhase<'_> {
    fn approximation(&mut self) -> &mut dyn DocIdSetIterator {
        &mut self.approximation
    }

    fn approximation_ref(&self) -> &dyn DocIdSetIterator {
        &self.approximation
    }

    fn matches(&mut self) -> Result<bool> {
        let state = self.approximation.core.state;
        java_assert!(state == IteratorState::Approximating, "{state}");
        let matches = self
            .approximation
            .core
            .inner
            .two_phase_iterator()
            .expect("INVARIANT: the two-phase view is only handed out over a two-phase scorer")
            .matches()?;
        if matches {
            let in_approximation_doc = self.approximation.in_approximation().doc_id();
            java_assert!(
                self.approximation.core.inner.iterator().doc_id() == in_approximation_doc,
                "Approximation and scorer don't advance synchronously"
            );
            self.approximation.core.doc = in_approximation_doc;
            self.approximation.core.state = IteratorState::Iterating;
        }
        Ok(matches)
    }

    fn match_cost(&self) -> f32 {
        let match_cost = self.match_cost;
        java_assert_unchecked!(!match_cost.is_nan());
        java_assert_unchecked!(match_cost >= 0.0);
        match_cost
    }
}

impl fmt::Debug for AssertingTwoPhase<'_> {
    /// `"AssertingScorer@asTwoPhaseIterator(" + in + ")"`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AssertingScorer@asTwoPhaseIterator({IN})")
    }
}

/// Wraps a [`Scorer`] with additional checks: the state machine of its
/// iterator and two-phase views, the order of `advanceShallow` targets, the
/// minimum competitive score and the scores themselves.
///
/// Equivalent to `org.apache.lucene.tests.search.AssertingScorer`. The scorer
/// is its own iterator view (`iterator()`, Java's `FilterDocIdSetIterator`);
/// see the module documentation for the layout and the divergences.
pub struct AssertingScorer<'a> {
    two_phase: AssertingTwoPhase<'a>,
}

impl fmt::Debug for AssertingScorer<'_> {
    /// `"AssertingScorer(" + in + ")"`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AssertingScorer({IN})")
    }
}

impl<'a> AssertingScorer<'a> {
    /// `AssertingScorer.wrap(other, canScore, canSetMinCompetitiveScore)`.
    ///
    /// Java's `wrap(null, …)` returns `null`; a caller holding an
    /// `Option<Box<dyn Scorer>>` maps it.
    pub fn wrap(
        other: Box<dyn Scorer + 'a>,
        can_score: bool,
        can_set_min_competitive_score: bool,
    ) -> Box<dyn Scorer + 'a> {
        Box::new(Self::new(other, can_score, can_set_min_competitive_score))
    }

    /// The private constructor: `doc` starts at `in.docID()`.
    fn new(
        mut inner: Box<dyn Scorer + 'a>,
        can_score: bool,
        can_set_min_competitive_score: bool,
    ) -> AssertingScorer<'a> {
        let doc = inner.doc_id();
        let cost = inner.iterator().cost();
        AssertingScorer {
            two_phase: AssertingTwoPhase {
                approximation: AssertingApproximation {
                    core: Core {
                        inner,
                        can_score,
                        can_set_min_competitive_score,
                        state: IteratorState::Iterating,
                        doc,
                        min_competitive_score: 0.0,
                        last_shallow_target: -1,
                        cost,
                    },
                    cost: 0,
                },
                match_cost: 0.0,
            },
        }
    }

    fn core(&self) -> &Core<'a> {
        &self.two_phase.approximation.core
    }

    fn core_mut(&mut self) -> &mut Core<'a> {
        &mut self.two_phase.approximation.core
    }

    /// Wraps a borrowed scorer with the state machine `state` carries over
    /// from an earlier wrapper of the same scorer.
    ///
    /// Java's `AssertingLeafCollector.setScorer` builds one
    /// `AssertingScorer.wrap(scorer, true, true)` that the wrapped collector
    /// keeps; this port lends the scorer to every collection call, so the
    /// collector keeps the wrapper's state ([`AssertingScorerState`]) and
    /// rebuilds the wrapper around each loan.
    pub(crate) fn wrap_borrowed(
        other: &'a mut dyn Scorer,
        state: Option<AssertingScorerState>,
    ) -> AssertingScorer<'a> {
        let mut scorer = Self::new(Box::new(BorrowedScorer { inner: other }), true, true);
        if let Some(state) = state {
            let core = scorer.core_mut();
            core.state = state.state;
            core.doc = state.doc;
            core.min_competitive_score = state.min_competitive_score;
            core.last_shallow_target = state.last_shallow_target;
        }
        scorer
    }

    /// The state machine of this wrapper (see
    /// [`wrap_borrowed`](Self::wrap_borrowed)).
    pub(crate) fn state_snapshot(&self) -> AssertingScorerState {
        let core = self.core();
        AssertingScorerState {
            state: core.state,
            doc: core.doc,
            min_competitive_score: core.min_competitive_score,
            last_shallow_target: core.last_shallow_target,
        }
    }

    /// `getIn()`.
    pub fn get_in(&self) -> &dyn Scorer {
        &*self.core().inner
    }

    /// The current state of the iterator views (Java's package-private
    /// `state` field).
    pub fn state(&self) -> IteratorState {
        self.core().state
    }

    /// `iterating()`: whether the scorer is positioned on a document it
    /// iterated to. The state cannot be required to be `ITERATING` alone,
    /// because of caching scorer wrappers.
    fn iterating(&self) -> bool {
        match Scorer::doc_id(self) {
            -1 | NO_MORE_DOCS => false,
            _ => self.core().state == IteratorState::Iterating,
        }
    }
}

impl Scorable for AssertingScorer<'_> {
    fn score(&mut self) -> Result<f32> {
        java_assert!(self.core().can_score);
        let state = self.core().state;
        java_assert!(self.iterating(), "{state}");
        let score = self.core_mut().inner.score()?;
        java_assert!(!score.is_nan(), "NaN score for in={IN}");
        if cfg!(debug_assertions) && self.core().last_shallow_target != -1 {
            let doc = Scorer::doc_id(self);
            let max_score = self.get_max_score(doc)?;
            let within = score <= max_score;
            java_assert!(within);
        }
        // `Float.compare(score, 0f) >= 0`: -0f compares below 0f.
        java_assert!(
            score.total_cmp(&0.0f32) != std::cmp::Ordering::Less,
            "{}",
            float_to_string(score)
        );
        Ok(score)
    }

    fn set_min_competitive_score(&mut self, score: f32) -> Result<()> {
        java_assert!(self.core().can_score);
        java_assert!(self.core().can_set_min_competitive_score);
        java_assert!(!score.is_nan());
        let not_decreasing = score >= self.core().min_competitive_score;
        java_assert!(not_decreasing);
        self.core_mut().inner.set_min_competitive_score(score)?;
        self.core_mut().min_competitive_score = score;
        Ok(())
    }

    fn children(&mut self) -> Result<Vec<ChildScorable<'_>>> {
        // We cannot hide that we hold a single child, else collectors (e.g.
        // ToParentBlockJoinCollector) that need to walk the scorer tree will
        // miss/skip the Scorer we wrap.
        Ok(vec![ChildScorable::new(
            &mut *self.core_mut().inner,
            "SHOULD",
        )])
    }

    /// An `AssertingScorer` is a `Scorer` (`instanceof Scorer`).
    fn as_scorer(&mut self) -> Option<&mut dyn Scorer> {
        Some(self)
    }

    /// `instanceof AssertingScorer` (see [`Scorable::class_id`]).
    fn class_id(&self) -> Option<TypeId> {
        Some(TypeId::of::<AssertingScorer<'static>>())
    }
}

impl Scorer for AssertingScorer<'_> {
    fn doc_id(&self) -> i32 {
        self.core().inner.doc_id()
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        self
    }

    fn two_phase_iterator(&mut self) -> Option<&mut dyn TwoPhaseIterator> {
        let core = &mut self.two_phase.approximation.core;
        let (in_approximation_doc, approximation_cost, match_cost) = {
            let in_two_phase = core.inner.two_phase_iterator()?;
            let match_cost = in_two_phase.match_cost();
            let in_approximation = in_two_phase.approximation();
            (
                in_approximation.doc_id(),
                in_approximation.cost(),
                match_cost,
            )
        };
        java_assert_unchecked!(in_approximation_doc == core.doc);
        self.two_phase.approximation.cost = approximation_cost;
        self.two_phase.match_cost = match_cost;
        Some(&mut self.two_phase)
    }

    fn advance_shallow(&mut self, target: i32) -> Result<i32> {
        java_assert!(self.core().can_score);
        let last_shallow_target = self.core().last_shallow_target;
        java_assert!(
            target >= last_shallow_target,
            "called on decreasing targets: target = {} < last target = {}",
            target,
            last_shallow_target
        );
        let doc_id = Scorer::doc_id(self);
        java_assert!(target >= doc_id, "target = {} < docID = {}", target, doc_id);
        let up_to = self.core_mut().inner.advance_shallow(target)?;
        java_assert!(up_to >= target, "upTo = {} < target = {}", up_to, target);
        let core = self.core_mut();
        core.last_shallow_target = target;
        if target != core.doc {
            core.state = IteratorState::ShallowAdvancing;
        }
        Ok(up_to)
    }

    fn get_max_score(&mut self, up_to: i32) -> Result<f32> {
        java_assert!(self.core().can_score);
        let last_shallow_target = self.core().last_shallow_target;
        java_assert!(
            up_to >= last_shallow_target,
            "uTo = {} < last target = {}",
            up_to,
            last_shallow_target
        );
        java_assert!(
            Scorer::doc_id(self) >= 0 || last_shallow_target >= 0,
            "Cannot get max scores until the iterator is positioned or advanceShallow has been called"
        );
        self.core_mut().inner.get_max_score(up_to)
    }

    fn next_docs_and_scores(
        &mut self,
        up_to: i32,
        live_docs: Option<&dyn Bits>,
        buffer: &mut DocAndFloatFeatureBuffer,
    ) -> Result<()> {
        java_assert!(self.core().doc != -1);
        let core = self.core_mut();
        core.inner.next_docs_and_scores(up_to, live_docs, buffer)?;
        let in_doc = core.inner.iterator().doc_id();
        if core.doc != in_doc {
            core.doc = in_doc;
            core.state = if in_doc == NO_MORE_DOCS {
                IteratorState::Finished
            } else {
                IteratorState::Iterating
            };
        }
        Ok(())
    }
}

/// The iterator view: Java's anonymous `FilterDocIdSetIterator` of
/// `iterator()`.
impl DocIdSetIterator for AssertingScorer<'_> {
    fn doc_id(&self) -> i32 {
        Scorer::doc_id(self)
    }

    fn next_doc(&mut self) -> Result<i32> {
        java_assert!(
            self.core().state != IteratorState::Finished,
            "nextDoc() called after NO_MORE_DOCS"
        );
        let in_doc = self.core_mut().inner.iterator().doc_id();
        java_assert!(in_doc + 1 >= self.core().last_shallow_target);
        let core = self.core_mut();
        let next_doc = core.inner.iterator().next_doc()?;
        java_assert!(
            next_doc > core.doc,
            "backwards nextDoc from {} to {} {IN}",
            core.doc,
            next_doc
        );
        core.state = if next_doc == NO_MORE_DOCS {
            IteratorState::Finished
        } else {
            IteratorState::Iterating
        };
        let in_doc = core.inner.iterator().doc_id();
        java_assert!(in_doc == next_doc);
        java_assert!(core.inner.doc_id() == in_doc);
        core.doc = next_doc;
        Ok(next_doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let core = self.core_mut();
        java_assert!(
            core.state != IteratorState::Finished,
            "advance() called after NO_MORE_DOCS"
        );
        java_assert!(
            target > core.doc,
            "target must be > docID(), got {} <= {}",
            target,
            core.doc
        );
        java_assert!(target >= core.last_shallow_target);
        let advanced = core.inner.iterator().advance(target)?;
        java_assert!(
            advanced >= target,
            "backwards advance from: {} to: {}",
            target,
            advanced
        );
        core.state = if advanced == NO_MORE_DOCS {
            IteratorState::Finished
        } else {
            IteratorState::Iterating
        };
        let in_doc = core.inner.iterator().doc_id();
        java_assert!(in_doc == advanced, "{} != {} in {IN}", in_doc, advanced);
        java_assert!(core.inner.doc_id() == in_doc);
        core.doc = advanced;
        Ok(advanced)
    }

    fn cost(&self) -> i64 {
        self.core().cost
    }

    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        let doc_id = DocIdSetIterator::doc_id(self);
        java_assert!(doc_id != -1);
        java_assert!(offset <= doc_id);
        self.core_mut()
            .inner
            .iterator()
            .into_bit_set(up_to, bit_set, offset)?;
        java_assert!(DocIdSetIterator::doc_id(self) >= up_to);
        Ok(())
    }

    fn doc_id_run_end(&mut self) -> Result<i32> {
        java_assert!(self.core().state == IteratorState::Iterating);
        // See the module divergences: the wrapped iterator's run end is not
        // reachable from `&self`; `docID() + 1` is the default run end.
        let next_non_matching_doc_id = DocIdSetIterator::doc_id(self) + 1;
        java_assert!(next_non_matching_doc_id > DocIdSetIterator::doc_id(self));
        Ok(next_non_matching_doc_id)
    }
}
