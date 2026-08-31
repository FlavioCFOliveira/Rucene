//! Required-except-prohibited scoring, ported from
//! `org.apache.lucene.search.ReqExclScorer`.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use crate::search::scorable::{ChildScorable, Scorable};
use crate::search::scorer::Scorer;
use crate::search::two_phase_iterator::TwoPhaseIterator;

/// Estimation of the number of operations required to call
/// [`DocIdSetIterator::advance`].
///
/// Equivalent to the private `ReqExclScorer.ADVANCE_COST` constant. This is
/// likely completely wrong, especially given that the cost of that method
/// usually depends on how far you want to advance, but it is probably better
/// than nothing.
const ADVANCE_COST: f32 = 10.0;

/// Message used where a two-phase view is known to be present.
const TWO_PHASE_INVARIANT: &str =
    "INVARIANT: the flag was set at construction and a Scorer returns a stable view";

/// The approximation-plus-confirmation half of a [`ReqExclScorer`].
///
/// Equivalent to the anonymous `TwoPhaseIterator` that
/// `ReqExclScorer.twoPhaseIterator()` returns, whose approximation is the
/// required clause's approximation.
///
/// **Divergence from Lucene 10.5.0.** Java's anonymous class holds the required
/// clause's approximation, which the enclosing scorer also holds. Rust forbids
/// that aliasing, so this type owns both scorers and *is* the approximation:
/// its [`DocIdSetIterator`] implementation forwards to the required clause's
/// approximation, and [`TwoPhaseIterator::approximation`] returns itself. The
/// sequence of calls is unchanged.
struct ReqExclTwoPhase {
    req_scorer: Box<dyn Scorer>,
    excl_scorer: Box<dyn Scorer>,
    req_two_phase: bool,
    excl_two_phase: bool,
    /// Whether the required clause is confirmed before the prohibited one.
    check_req_first: bool,
    match_cost: f32,
    req_cost: i64,
}

impl ReqExclTwoPhase {
    fn req_approximation(&mut self) -> &mut dyn DocIdSetIterator {
        if self.req_two_phase {
            self.req_scorer
                .two_phase_iterator()
                .expect(TWO_PHASE_INVARIANT)
                .approximation()
        } else {
            self.req_scorer.iterator()
        }
    }

    fn excl_approximation(&mut self) -> &mut dyn DocIdSetIterator {
        if self.excl_two_phase {
            self.excl_scorer
                .two_phase_iterator()
                .expect(TWO_PHASE_INVARIANT)
                .approximation()
        } else {
            self.excl_scorer.iterator()
        }
    }

    /// Equivalent to the private static
    /// `ReqExclScorer.matchesOrNull(TwoPhaseIterator)` applied to the required
    /// clause.
    fn req_matches_or_null(&mut self) -> Result<bool> {
        if self.req_two_phase {
            self.req_scorer
                .two_phase_iterator()
                .expect(TWO_PHASE_INVARIANT)
                .matches()
        } else {
            Ok(true)
        }
    }

    /// Equivalent to `ReqExclScorer.matchesOrNull(TwoPhaseIterator)` applied to
    /// the prohibited clause.
    fn excl_matches_or_null(&mut self) -> Result<bool> {
        if self.excl_two_phase {
            self.excl_scorer
                .two_phase_iterator()
                .expect(TWO_PHASE_INVARIANT)
                .matches()
        } else {
            Ok(true)
        }
    }
}

impl DocIdSetIterator for ReqExclTwoPhase {
    fn doc_id(&self) -> i32 {
        self.req_scorer.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.req_approximation().next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.req_approximation().advance(target)
    }

    fn cost(&self) -> i64 {
        self.req_cost
    }
}

impl TwoPhaseIterator for ReqExclTwoPhase {
    fn approximation(&mut self) -> &mut dyn DocIdSetIterator {
        self
    }

    fn approximation_ref(&self) -> &dyn DocIdSetIterator {
        self
    }

    fn matches(&mut self) -> Result<bool> {
        let doc = self.req_scorer.doc_id();
        // check if the doc is not excluded
        let mut excl_doc = self.excl_scorer.doc_id();
        if excl_doc < doc {
            excl_doc = self.excl_approximation().advance(doc)?;
        }
        if excl_doc != doc {
            return self.req_matches_or_null();
        }
        if self.check_req_first {
            // reqTwoPhaseIterator is LESS costly than exclTwoPhaseIterator,
            // check it first
            Ok(self.req_matches_or_null()? && !self.excl_matches_or_null()?)
        } else {
            // reqTwoPhaseIterator is MORE costly than exclTwoPhaseIterator,
            // check it last
            Ok(!self.excl_matches_or_null()? && self.req_matches_or_null()?)
        }
    }

    fn match_cost(&self) -> f32 {
        self.match_cost
    }
}

/// A scorer for queries with a required sub-scorer and an excluding
/// (prohibited) sub-scorer.
///
/// Equivalent to `org.apache.lucene.search.ReqExclScorer`.
pub struct ReqExclScorer {
    inner: ReqExclTwoPhase,
}

impl std::fmt::Debug for ReqExclScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReqExclScorer")
            .field("match_cost", &self.inner.match_cost)
            .finish_non_exhaustive()
    }
}

impl ReqExclScorer {
    /// Constructs a scorer that matches `req_scorer` except where `excl_scorer`
    /// indicates exclusion.
    ///
    /// Equivalent to `new ReqExclScorer(Scorer, Scorer)`.
    pub fn new(mut req_scorer: Box<dyn Scorer>, mut excl_scorer: Box<dyn Scorer>) -> Self {
        let req_two_phase = req_scorer.two_phase_iterator().is_some();
        let excl_two_phase = excl_scorer.two_phase_iterator().is_some();

        let req_match_cost = if req_two_phase {
            req_scorer
                .two_phase_iterator()
                .expect(TWO_PHASE_INVARIANT)
                .match_cost()
        } else {
            0.0
        };
        let excl_match_cost = if excl_two_phase {
            excl_scorer
                .two_phase_iterator()
                .expect(TWO_PHASE_INVARIANT)
                .match_cost()
        } else {
            0.0
        };
        let req_cost = if req_two_phase {
            req_scorer
                .two_phase_iterator()
                .expect(TWO_PHASE_INVARIANT)
                .approximation_ref()
                .cost()
        } else {
            req_scorer.iterator().cost()
        };
        let excl_cost = if excl_two_phase {
            excl_scorer
                .two_phase_iterator()
                .expect(TWO_PHASE_INVARIANT)
                .approximation_ref()
                .cost()
        } else {
            excl_scorer.iterator().cost()
        };

        let match_cost = Self::match_cost(
            req_cost,
            req_two_phase,
            req_match_cost,
            excl_cost,
            excl_two_phase,
            excl_match_cost,
        );

        // Equivalent to the branch `ReqExclScorer.twoPhaseIterator()` takes to
        // pick which clause it confirms first.
        let check_req_first =
            !req_two_phase || (excl_two_phase && req_match_cost <= excl_match_cost);

        Self {
            inner: ReqExclTwoPhase {
                req_scorer,
                excl_scorer,
                req_two_phase,
                excl_two_phase,
                check_req_first,
                match_cost,
                req_cost,
            },
        }
    }

    /// Equivalent to the private static `ReqExclScorer.matchCost(
    /// DocIdSetIterator, TwoPhaseIterator, DocIdSetIterator,
    /// TwoPhaseIterator)`.
    fn match_cost(
        req_cost: i64,
        req_two_phase: bool,
        req_match_cost: f32,
        excl_cost: i64,
        excl_two_phase: bool,
        excl_match_cost: f32,
    ) -> f32 {
        // we perform 2 comparisons to advance exclApproximation
        let mut match_cost = 2.0f32;
        if req_two_phase {
            // this two-phase iterator must always be matched
            match_cost += req_match_cost;
        }

        // match cost of the prohibited clause: we need to advance the
        // approximation and match the two-phased iterator
        let excl_match_cost = ADVANCE_COST + if excl_two_phase { excl_match_cost } else { 0.0 };

        // upper value for the ratio of documents that reqApproximation matches
        // that exclApproximation also matches
        let ratio = if req_cost <= 0 {
            1.0f32
        } else if excl_cost <= 0 {
            0.0f32
        } else {
            (req_cost.min(excl_cost) as f32) / (req_cost as f32)
        };
        match_cost += ratio * excl_match_cost;

        match_cost
    }

    fn confirm(&mut self, mut doc: i32) -> Result<i32> {
        loop {
            if doc == NO_MORE_DOCS {
                return Ok(NO_MORE_DOCS);
            }
            if self.inner.matches()? {
                return Ok(doc);
            }
            doc = self.inner.next_doc()?;
        }
    }
}

impl DocIdSetIterator for ReqExclScorer {
    fn doc_id(&self) -> i32 {
        self.inner.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        let doc = self.inner.next_doc()?;
        self.confirm(doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let doc = self.inner.advance(target)?;
        self.confirm(doc)
    }

    fn cost(&self) -> i64 {
        self.inner.req_cost
    }
}

impl Scorable for ReqExclScorer {
    fn score(&mut self) -> Result<f32> {
        // reqScorer may be null when next() or skipTo() already returned false
        self.inner.req_scorer.score()
    }

    fn set_min_competitive_score(&mut self, min_score: f32) -> Result<()> {
        // The score of this scorer is the same as the score of 'reqScorer'.
        self.inner.req_scorer.set_min_competitive_score(min_score)
    }

    fn children(&mut self) -> Result<Vec<ChildScorable<'_>>> {
        Ok(vec![ChildScorable::new(
            self.inner.req_scorer.as_scorable(),
            "MUST",
        )])
    }
}

impl Scorer for ReqExclScorer {
    fn as_scorable(&mut self) -> &mut dyn Scorable {
        self
    }

    fn doc_id(&self) -> i32 {
        self.inner.doc_id()
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        self
    }

    fn two_phase_iterator(&mut self) -> Option<&mut dyn TwoPhaseIterator> {
        Some(&mut self.inner)
    }

    fn advance_shallow(&mut self, target: i32) -> Result<i32> {
        self.inner.req_scorer.advance_shallow(target)
    }

    fn get_max_score(&mut self, up_to: i32) -> Result<f32> {
        self.inner.req_scorer.get_max_score(up_to)
    }
}
