//! Conjunctions of iterators, ported from
//! `org.apache.lucene.search.ConjunctionDISI`.
//!
//! # Adaptation: one type instead of three
//!
//! Java splits a conjunction across `ConjunctionDISI`, which intersects the
//! *approximations*, and the private `ConjunctionTwoPhaseIterator`, whose
//! approximation **is** that `ConjunctionDISI` and whose `matches()` confirms
//! the two-phase clauses. The approximation of every two-phase clause therefore
//! lives in two places at once: inside the `ConjunctionDISI`'s iterator array
//! and inside the `TwoPhaseIterator` it belongs to.
//!
//! Rust cannot own an approximation apart from the iterator that produces it,
//! so this port fuses the two classes: [`ConjunctionDISI`] owns every clause as
//! a [`ConjunctionMember`], implements [`DocIdSetIterator`] as the intersection
//! of the approximations, and implements [`TwoPhaseIterator`] by returning
//! itself as the approximation and confirming the two-phase clauses in
//! `matches()`. The lead/tail split, the cost-ascending approximation order, the
//! match-cost-ascending confirmation order and the `doNext` loop are unchanged.
//!
//! # Divergence: no `instanceof` collapsing
//!
//! `ConjunctionDISI.addIterator` recognises three erased shapes with
//! `instanceof` — a `TwoPhaseIterator` behind `TwoPhaseIterator.unwrap`, a
//! nested `ConjunctionDISI`, and a `BitSetConjunctionDISI` — and flattens them
//! into the parent conjunction. Rust cannot downcast a
//! `dyn DocIdSetIterator`, so a nested conjunction stays nested. The same
//! documents match; only the depth of the call chain differs. For the same
//! reason the `BitSetConjunctionDISI` specialisation, which needs to recognise
//! `BitSetIterator` instances, has no counterpart: it is a throughput
//! optimisation over bit-set-backed clauses, not a behaviour.

#![deny(unsafe_code)]

use crate::error::{LuceneError, Result};
use crate::search::doc_id_set_iterator::DocIdSetIterator;
use crate::search::scorer::Scorer;
use crate::search::two_phase_iterator::{ScorerIterator, TwoPhaseIterator};
use crate::util::FixedBitSet;

/// Message used where a two-phase view is known to be present because it was
/// observed once at construction time.
const TWO_PHASE_INVARIANT: &str =
    "INVARIANT: the member was built two-phase and a Scorer returns a stable view";

/// One clause of a [`ConjunctionDISI`].
///
/// Equivalent to one entry of the `allIterators` / `twoPhaseIterators` pair
/// Java threads through `ConjunctionDISI.createConjunction`, plus — for the
/// [`from_scorer`](Self::from_scorer) shape — the `Scorer` that
/// [`ConjunctionScorer`](crate::search::ConjunctionScorer) keeps beside the
/// conjunction so that it can score the matches.
pub struct ConjunctionMember {
    kind: MemberKind,
    cost: i64,
    match_cost: f32,
    has_two_phase: bool,
}

enum MemberKind {
    Plain(Box<dyn DocIdSetIterator>),
    TwoPhase(Box<dyn TwoPhaseIterator>),
    Scorer(Box<dyn Scorer>),
}

impl std::fmt::Debug for ConjunctionMember {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConjunctionMember")
            .field("cost", &self.cost)
            .field("match_cost", &self.match_cost)
            .field("has_two_phase", &self.has_two_phase)
            .finish_non_exhaustive()
    }
}

impl ConjunctionMember {
    /// Wraps a plain iterator, which needs no confirmation.
    ///
    /// Equivalent to an entry that `ConjunctionDISI.addIterator` appends to
    /// `allIterators` without appending anything to `twoPhaseIterators`.
    pub fn from_iterator(iterator: Box<dyn DocIdSetIterator>) -> Self {
        let cost = iterator.cost();
        Self {
            kind: MemberKind::Plain(iterator),
            cost,
            match_cost: 0.0,
            has_two_phase: false,
        }
    }

    /// Wraps a two-phase iterator, whose approximation drives iteration and
    /// whose `matches()` confirms.
    ///
    /// Equivalent to what `ConjunctionDISI.addTwoPhaseIterator` records.
    pub fn from_two_phase(two_phase: Box<dyn TwoPhaseIterator>) -> Self {
        let cost = two_phase.approximation_ref().cost();
        let match_cost = two_phase.match_cost();
        Self {
            kind: MemberKind::TwoPhase(two_phase),
            cost,
            match_cost,
            has_two_phase: true,
        }
    }

    /// Wraps a scorer, splitting it the way `ConjunctionDISI.addScorer` does:
    /// by its two-phase view when it has one, by its iterator otherwise.
    pub fn from_scorer(mut scorer: Box<dyn Scorer>) -> Self {
        let has_two_phase = scorer.two_phase_iterator().is_some();
        let match_cost = if has_two_phase {
            scorer
                .two_phase_iterator()
                .expect(TWO_PHASE_INVARIANT)
                .match_cost()
        } else {
            0.0
        };
        let cost = if has_two_phase {
            scorer
                .two_phase_iterator()
                .expect(TWO_PHASE_INVARIANT)
                .approximation_ref()
                .cost()
        } else {
            scorer.iterator().cost()
        };
        Self {
            kind: MemberKind::Scorer(scorer),
            cost,
            match_cost,
            has_two_phase,
        }
    }

    /// Returns the cost of this member's approximation, read once at
    /// construction.
    ///
    /// Equivalent to `DocIdSetIterator.cost()` on the approximation Java stores.
    pub fn cost(&self) -> i64 {
        self.cost
    }

    /// Returns the match cost, or `0` when this member needs no confirmation.
    ///
    /// Equivalent to `TwoPhaseIterator.matchCost()`.
    pub fn match_cost(&self) -> f32 {
        self.match_cost
    }

    /// Returns whether this member needs confirming through
    /// [`matches`](Self::matches).
    pub fn has_two_phase(&self) -> bool {
        self.has_two_phase
    }

    /// Returns the wrapped scorer, when this member was built from one.
    ///
    /// Equivalent to the reference `ConjunctionScorer` keeps in its `scorers`
    /// and `required` collections.
    pub fn scorer(&mut self) -> Option<&mut dyn Scorer> {
        match &mut self.kind {
            MemberKind::Scorer(scorer) => Some(&mut **scorer),
            _ => None,
        }
    }

    /// Returns the current doc ID of this member's approximation.
    ///
    /// Equivalent to `approximation.docID()`.
    pub fn doc_id(&self) -> i32 {
        match &self.kind {
            MemberKind::Plain(iterator) => iterator.doc_id(),
            MemberKind::TwoPhase(two_phase) => two_phase.approximation_ref().doc_id(),
            MemberKind::Scorer(scorer) => scorer.doc_id(),
        }
    }

    /// Advances this member's approximation to `target`.
    ///
    /// Equivalent to `approximation.advance(int)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while advancing.
    pub fn advance(&mut self, target: i32) -> Result<i32> {
        self.approximation().advance(target)
    }

    /// Advances this member's approximation to the next document.
    ///
    /// Equivalent to `approximation.nextDoc()`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while advancing.
    pub fn next_doc(&mut self) -> Result<i32> {
        self.approximation().next_doc()
    }

    /// Returns this member's approximation.
    ///
    /// Equivalent to reading the `approximation` field Java stores beside the
    /// two-phase view.
    pub fn approximation(&mut self) -> &mut dyn DocIdSetIterator {
        match &mut self.kind {
            MemberKind::Plain(iterator) => &mut **iterator,
            MemberKind::TwoPhase(two_phase) => two_phase.approximation(),
            MemberKind::Scorer(scorer) => {
                if self.has_two_phase {
                    scorer
                        .two_phase_iterator()
                        .expect(TWO_PHASE_INVARIANT)
                        .approximation()
                } else {
                    scorer.iterator()
                }
            }
        }
    }

    /// Returns the two-phase view, or `None` when this member needs no
    /// confirmation.
    pub fn two_phase(&mut self) -> Option<&mut dyn TwoPhaseIterator> {
        match &mut self.kind {
            MemberKind::Plain(_) => None,
            MemberKind::TwoPhase(two_phase) => Some(&mut **two_phase),
            MemberKind::Scorer(scorer) => scorer.two_phase_iterator(),
        }
    }

    /// Confirms the current document, or returns `true` when this member needs
    /// no confirmation.
    ///
    /// Equivalent to `twoPhase == null || twoPhase.matches()`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while confirming.
    pub fn matches(&mut self) -> Result<bool> {
        match self.two_phase() {
            None => Ok(true),
            Some(two_phase) => two_phase.matches(),
        }
    }

    /// Returns the end of the run of consecutive matching doc IDs containing
    /// the current one.
    ///
    /// Equivalent to `DenseConjunctionBulkScorer.DisiWrapper.docIDRunEnd()`,
    /// which reads the run end of the two-phase view when there is one and of
    /// the approximation otherwise.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while computing the run end.
    pub fn doc_id_run_end(&mut self) -> Result<i32> {
        if self.has_two_phase {
            let two_phase = self
                .two_phase()
                .expect("INVARIANT: has_two_phase implies a two-phase view");
            two_phase.doc_id_run_end()
        } else {
            self.approximation().doc_id_run_end()
        }
    }

    /// Loads the matching doc IDs below `up_to` into `bit_set`.
    ///
    /// Equivalent to `DenseConjunctionBulkScorer.DisiWrapper.intoBitSet(int,
    /// FixedBitSet, int)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while iterating or confirming.
    #[allow(clippy::wrong_self_convention)]
    pub fn into_bit_set(
        &mut self,
        up_to: i32,
        bit_set: &mut FixedBitSet,
        offset: i32,
    ) -> Result<()> {
        if self.has_two_phase {
            let two_phase = self
                .two_phase()
                .expect("INVARIANT: has_two_phase implies a two-phase view");
            two_phase.into_bit_set(up_to, bit_set, offset)
        } else {
            self.approximation().into_bit_set(up_to, bit_set, offset)
        }
    }
}

/// A conjunction of [`DocIdSetIterator`]s.
///
/// Equivalent to the `final class org.apache.lucene.search.ConjunctionDISI`
/// fused with its private `ConjunctionTwoPhaseIterator`; see the
/// [module documentation](self). All sub-iterators must be on the same document
/// at all times; this iterates the doc IDs present in every one of them.
pub struct ConjunctionDISI {
    members: Vec<ConjunctionMember>,
    /// Positions of the members in ascending cost order: `[0]` is Java's
    /// `lead1`, `[1]` its `lead2`, the rest its `others`.
    approx_order: Vec<usize>,
    /// Positions of the two-phase members, in ascending match-cost order.
    two_phase_order: Vec<usize>,
    match_cost: f32,
    cost: i64,
}

impl std::fmt::Debug for ConjunctionDISI {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConjunctionDISI")
            .field("members", &self.members.len())
            .field("two_phase", &self.two_phase_order.len())
            .field("cost", &self.cost)
            .finish()
    }
}

impl ConjunctionDISI {
    /// Builds a conjunction over the given clauses.
    ///
    /// Equivalent to `ConjunctionDISI.createConjunction(List<DocIdSetIterator>,
    /// List<TwoPhaseIterator>)`, with the clauses already collected into
    /// [`ConjunctionMember`]s.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when fewer than two clauses are
    /// supplied, or when they are not all positioned on the same document —
    /// which is the `IllegalArgumentException` Java throws from
    /// `throwSubIteratorsNotOnSameDocument`.
    pub fn new(members: Vec<ConjunctionMember>) -> Result<Self> {
        if members.len() < 2 {
            return Err(LuceneError::IllegalArgument(
                "Cannot make a ConjunctionDISI of less than 2 iterators".to_string(),
            ));
        }

        // check that all sub-iterators are on the same doc ID
        let cur_doc = members[0].doc_id();
        for member in &members {
            if member.doc_id() != cur_doc {
                return Err(LuceneError::IllegalArgument(
                    "Sub-iterators of ConjunctionDISI are not on the same document!".to_string(),
                ));
            }
        }

        // Sort the list first to allow the sparser iterator to lead the
        // matching. Java's CollectionUtil.timSort is stable, and so is
        // `sort_by`.
        let mut approx_order: Vec<usize> = (0..members.len()).collect();
        approx_order.sort_by(|a, b| members[*a].cost().cmp(&members[*b].cost()));

        let mut two_phase_order: Vec<usize> = (0..members.len())
            .filter(|i| members[*i].has_two_phase())
            .collect();
        two_phase_order.sort_by(|a, b| {
            members[*a]
                .match_cost()
                .total_cmp(&members[*b].match_cost())
        });

        // Compute the matchCost as the total matchCost of the sub iterators.
        let mut match_cost = 0.0f32;
        for i in &two_phase_order {
            match_cost += members[*i].match_cost();
        }

        let cost = members[approx_order[0]].cost();

        Ok(Self {
            members,
            approx_order,
            two_phase_order,
            match_cost,
            cost,
        })
    }

    /// Returns the number of clauses.
    pub fn len(&self) -> usize {
        self.members.len()
    }

    /// Returns whether this conjunction has no clause; it never has, because
    /// [`new`](Self::new) rejects fewer than two.
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// Returns whether any clause needs two-phase confirmation, that is,
    /// whether Java's `TwoPhaseIterator.unwrap` on the created conjunction
    /// would return non-`null`.
    pub fn has_two_phase(&self) -> bool {
        !self.two_phase_order.is_empty()
    }

    /// Returns the clause at `position`, in construction order.
    pub fn member(&mut self, position: usize) -> &mut ConjunctionMember {
        &mut self.members[position]
    }

    /// Returns the clauses, in construction order.
    pub fn members(&mut self) -> &mut [ConjunctionMember] {
        &mut self.members
    }

    /// Equivalent to the private `ConjunctionDISI.doNext(int)`.
    fn do_next(&mut self, mut doc: i32) -> Result<i32> {
        let lead1 = self.approx_order[0];
        let lead2 = self.approx_order[1];
        'advance_head: loop {
            debug_assert_eq!(doc, self.members[lead1].doc_id());

            // find agreement between the two iterators with the lower costs; we
            // special case them because they do not need the
            // `other.docID() < doc` check that the `others` iterators need
            let next2 = self.members[lead2].advance(doc)?;
            if next2 != doc {
                doc = self.members[lead1].advance(next2)?;
                if next2 != doc {
                    continue 'advance_head;
                }
            }

            // then find agreement with other iterators
            for k in 2..self.approx_order.len() {
                let other = self.approx_order[k];
                // other.doc may already be equal to doc if we "continued
                // advanceHead" on the previous iteration and the advance on the
                // lead scorer exactly matched.
                if self.members[other].doc_id() < doc {
                    let next = self.members[other].advance(doc)?;
                    if next > doc {
                        // iterator beyond the current doc - advance lead and
                        // continue to the new highest doc.
                        doc = self.members[lead1].advance(next)?;
                        continue 'advance_head;
                    }
                }
            }

            // success - all iterators are on the same doc
            return Ok(doc);
        }
    }
}

impl DocIdSetIterator for ConjunctionDISI {
    fn doc_id(&self) -> i32 {
        self.members[self.approx_order[0]].doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        let lead1 = self.approx_order[0];
        let doc = self.members[lead1].next_doc()?;
        self.do_next(doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let lead1 = self.approx_order[0];
        let doc = self.members[lead1].advance(target)?;
        self.do_next(doc)
    }

    fn cost(&self) -> i64 {
        self.cost
    }
}

impl TwoPhaseIterator for ConjunctionDISI {
    fn approximation(&mut self) -> &mut dyn DocIdSetIterator {
        self
    }

    fn approximation_ref(&self) -> &dyn DocIdSetIterator {
        self
    }

    fn matches(&mut self) -> Result<bool> {
        // match cheapest first
        for k in 0..self.two_phase_order.len() {
            let member = self.two_phase_order[k];
            if !self.members[member].matches()? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn match_cost(&self) -> f32 {
        self.match_cost
    }
}

/// Builds a conjunction over the given clauses and returns it in the shape a
/// caller can tell apart.
///
/// Equivalent to `ConjunctionDISI.createConjunction(List<DocIdSetIterator>,
/// List<TwoPhaseIterator>)`.
///
/// **Divergence from Lucene 10.5.0.** Java's `allIterators` also holds the
/// approximation of every entry of `twoPhaseIterators`; here each two-phase
/// clause carries its own approximation, because Rust cannot own an
/// approximation apart from the iterator that produces it. The result is
/// returned as a [`ScorerIterator`] rather than as an erased
/// `DocIdSetIterator`, for the reason that type documents.
///
/// # Errors
///
/// As [`ConjunctionDISI::new`].
pub fn create_conjunction(members: Vec<ConjunctionMember>) -> Result<ScorerIterator> {
    let conjunction = ConjunctionDISI::new(members)?;
    if conjunction.has_two_phase() {
        Ok(ScorerIterator::TwoPhase(Box::new(conjunction)))
    } else {
        Ok(ScorerIterator::Simple(Box::new(conjunction)))
    }
}
