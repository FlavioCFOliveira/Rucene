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
//! The private `BitSetConjunctionDISI` is fused in too: the clauses whose
//! approximation is a [`BitSetIterator`] costlier than the cheapest clause are
//! not advanced but tested against their bit sets, and positioned with
//! `setDocId` on a match (`ConjunctionDISI.java:113-137, 226-299`).
//!
//! # Divergence: no `instanceof` collapsing
//!
//! `ConjunctionDISI.addIterator` recognises three erased shapes with
//! `instanceof` — a `TwoPhaseIterator` behind `TwoPhaseIterator.unwrap`, a
//! nested `ConjunctionDISI`, and a `BitSetConjunctionDISI` — and flattens them
//! into the parent conjunction. This port's members are owned by the
//! conjunction that holds them, so a nested conjunction stays nested. The same
//! documents match; only the depth of the call chain differs.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use crate::search::scorer::Scorer;
use crate::search::two_phase_iterator::{ScorerIterator, TwoPhaseIterator};
use crate::util::bit_set::{BitSet, BitSetIterator};
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
    /// `BitSetConjunctionDISI.bitSetIterators`: the positions of the members
    /// whose approximation is a [`BitSetIterator`] costlier than the cheapest
    /// clause, least costly first. They are not in `approx_order`.
    bit_set_order: Vec<usize>,
    /// `BitSetConjunctionDISI.bitSets`, the sets of those iterators.
    bit_sets: Vec<Arc<dyn BitSet>>,
    /// `BitSetConjunctionDISI.minLength`, the shortest of those sets.
    min_length: i32,
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
    /// Returns [`LuceneErrorKind::IllegalArgument`](crate::error::LuceneErrorKind::IllegalArgument) when fewer than two clauses are
    /// supplied, or when they are not all positioned on the same document —
    /// which is the `IllegalArgumentException` Java throws from
    /// `throwSubIteratorsNotOnSameDocument`.
    pub fn new(mut members: Vec<ConjunctionMember>) -> Result<Self> {
        if members.len() < 2 {
            return Err(LuceneError::illegal_argument(
                "Cannot make a ConjunctionDISI of less than 2 iterators".to_string(),
            ));
        }

        // check that all sub-iterators are on the same doc ID
        let cur_doc = members[0].doc_id();
        for member in &members {
            if member.doc_id() != cur_doc {
                return Err(throw_sub_iterators_not_on_same_document());
            }
        }

        // `ConjunctionDISI.createConjunction` (`ConjunctionDISI.java:113-137`):
        // every bit set iterator goes to the bit set clauses, except when it
        // has the minimum cost, since it then has to lead the iteration.
        let min_cost = members
            .iter()
            .map(ConjunctionMember::cost)
            .min()
            .unwrap_or(i64::MAX);
        let mut approx_order: Vec<usize> = Vec::with_capacity(members.len());
        let mut bit_set_order: Vec<usize> = Vec::new();
        for (i, member) in members.iter_mut().enumerate() {
            if member.cost() > min_cost && Self::bit_set_iterator(member).is_some() {
                bit_set_order.push(i);
            } else {
                approx_order.push(i);
            }
        }
        // Sort the list first to allow the sparser iterator to lead the
        // matching. Java's CollectionUtil.timSort is stable, and so is
        // `sort_by`.
        approx_order.sort_by(|a, b| members[*a].cost().cmp(&members[*b].cost()));
        // Put the least costly bit set iterators first so that we exit as soon
        // as possible (`ArrayUtil.timSort`, stable).
        bit_set_order.sort_by(|a, b| members[*a].cost().cmp(&members[*b].cost()));
        let mut bit_sets = Vec::with_capacity(bit_set_order.len());
        let mut min_length = i32::MAX;
        for &i in &bit_set_order {
            let bit_set = Arc::clone(
                Self::bit_set_iterator(&mut members[i])
                    .expect("INVARIANT: the member was narrowed to a BitSetIterator above")
                    .get_bit_set(),
            );
            min_length = min_length.min(bit_set.length() as i32);
            bit_sets.push(bit_set);
        }

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
            bit_set_order,
            bit_sets,
            min_length,
            two_phase_order,
            match_cost,
            cost,
        })
    }

    /// `iterator instanceof BitSetIterator` on the member's approximation,
    /// with the cast.
    fn bit_set_iterator(member: &mut ConjunctionMember) -> Option<&mut BitSetIterator> {
        member
            .approximation()
            .disi_as_any_mut()
            .and_then(|any| any.downcast_mut::<BitSetIterator>())
    }

    /// Positions the conjunction of the approximations other than the bit set
    /// clauses on its next doc.
    ///
    /// `lead.nextDoc()` of `BitSetConjunctionDISI`, whose lead is the
    /// `ConjunctionDISI` of the other clauses, or the one clause when only one
    /// is left (`ConjunctionDISI.java:126-133`).
    fn approximation_next_doc(&mut self) -> Result<i32> {
        let lead1 = self.approx_order[0];
        let doc = self.members[lead1].next_doc()?;
        self.do_next(doc)
    }

    /// `lead.advance(target)`; see [`Self::approximation_next_doc`].
    fn approximation_advance(&mut self, target: i32) -> Result<i32> {
        let lead1 = self.approx_order[0];
        let doc = self.members[lead1].advance(target)?;
        self.do_next(doc)
    }

    /// Equivalent to `BitSetConjunctionDISI.doNext(int)`
    /// (`ConjunctionDISI.java:265-286`): skips the docs one of the bit sets
    /// does not hold, and positions every bit set iterator on a match. With no
    /// bit set clause there is no `BitSetConjunctionDISI`, and `doc` stands.
    fn bit_set_do_next(&mut self, mut doc: i32) -> Result<i32> {
        if self.bit_set_order.is_empty() {
            return Ok(doc);
        }
        'advance_lead: loop {
            if doc >= self.min_length {
                if doc != NO_MORE_DOCS {
                    self.approximation_advance(NO_MORE_DOCS)?;
                }
                return Ok(NO_MORE_DOCS);
            }
            for bit_set in &self.bit_sets {
                if !bit_set.get(doc as usize) {
                    doc = self.approximation_next_doc()?;
                    continue 'advance_lead;
                }
            }
            for k in 0..self.bit_set_order.len() {
                let member = self.bit_set_order[k];
                Self::bit_set_iterator(&mut self.members[member])
                    .expect("INVARIANT: bit set members were narrowed at construction")
                    .set_doc_id(doc);
            }
            return Ok(doc);
        }
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
        if self.approx_order.len() == 1 {
            // A single clause leads alone: Java uses it directly rather than
            // building a `ConjunctionDISI` (`ConjunctionDISI.java:127-128`).
            return Ok(doc);
        }
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

    /// Returns `true` if all sub-iterators are on the same doc ID, `false`
    /// otherwise.
    ///
    /// Equivalent to the private `ConjunctionDISI.assertItersOnSameDoc()`,
    /// which Java only ever evaluates from inside an `assert` — hence the
    /// [`debug_assert!`] calls in [`next_doc`](DocIdSetIterator::next_doc) and
    /// [`advance`](DocIdSetIterator::advance) below.
    fn assert_iters_on_same_doc(&self) -> bool {
        // `ConjunctionDISI.assertItersOnSameDoc()` over the approximations and
        // `BitSetConjunctionDISI.assertItersOnSameDoc()` over the bit set
        // iterators: every one is on the lead's doc.
        let lead1 = self.approx_order[0];
        let cur_doc = self.members[lead1].doc_id();
        self.approx_order[1..]
            .iter()
            .chain(&self.bit_set_order)
            .all(|&i| self.members[i].doc_id() == cur_doc)
    }
}

impl DocIdSetIterator for ConjunctionDISI {
    fn doc_id(&self) -> i32 {
        self.members[self.approx_order[0]].doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        debug_assert!(
            self.assert_iters_on_same_doc(),
            "Sub-iterators of ConjunctionDISI are not on the same document!"
        );
        let doc = self.approximation_next_doc()?;
        self.bit_set_do_next(doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        debug_assert!(
            self.assert_iters_on_same_doc(),
            "Sub-iterators of ConjunctionDISI are not one the same document!"
        );
        let doc = self.approximation_advance(target)?;
        self.bit_set_do_next(doc)
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

/// Adds `scorer` to `members`, splitting it into two phases when it supports
/// an approximation.
///
/// Equivalent to `ConjunctionDISI.addScorer(Scorer, List<DocIdSetIterator>,
/// List<TwoPhaseIterator>)`, which dispatches to
/// [`add_two_phase_iterator`] when `scorer.twoPhaseIterator()` is non-`null`
/// and to [`add_iterator`] otherwise. Here the split is performed by
/// [`ConjunctionMember::from_scorer`], because this port keeps the `Scorer`
/// itself so that [`ConjunctionScorer`](crate::search::ConjunctionScorer) can
/// score the matches; see the [module documentation](self).
pub fn add_scorer(scorer: Box<dyn Scorer>, members: &mut Vec<ConjunctionMember>) {
    members.push(ConjunctionMember::from_scorer(scorer));
}

/// Adds `disi` to `members` as a clause that needs no confirmation.
///
/// Equivalent to `ConjunctionDISI.addIterator(DocIdSetIterator,
/// List<DocIdSetIterator>, List<TwoPhaseIterator>)`.
///
/// **Divergence from Lucene 10.5.0.** Java first probes the erased iterator
/// with `TwoPhaseIterator.unwrap` and then with two `getClass()` tests, so
/// that a two-phase iterator, a nested `ConjunctionDISI` or a
/// `BitSetConjunctionDISI` is collapsed into the parent conjunction
/// (`ConjunctionDISI.java` lines 53-76). Rust cannot downcast a
/// `dyn DocIdSetIterator`, so the clause is appended as it is and a nested
/// conjunction stays nested; the same documents match, only the depth of the
/// call chain differs. Callers that hold the unerased shape should call
/// [`add_two_phase_iterator`] instead. See the [module documentation](self).
pub fn add_iterator(disi: Box<dyn DocIdSetIterator>, members: &mut Vec<ConjunctionMember>) {
    members.push(ConjunctionMember::from_iterator(disi));
}

/// Adds `two_phase` to `members` as a clause whose candidates still need
/// confirming.
///
/// Equivalent to `ConjunctionDISI.addTwoPhaseIterator(TwoPhaseIterator,
/// List<DocIdSetIterator>, List<TwoPhaseIterator>)`, which appends the
/// approximation to `allIterators` and the two-phase iterator itself to
/// `twoPhaseIterators`. Here a single [`ConjunctionMember`] carries both,
/// because Rust cannot own an approximation apart from the iterator that
/// produces it; see the [module documentation](self).
///
/// **Divergence from Lucene 10.5.0.** Java also collapses a nested
/// `ConjunctionTwoPhaseIterator` into the parent by a `getClass()` test
/// (`ConjunctionDISI.java` lines 84-88), which Rust cannot reproduce for the
/// reason [`add_iterator`] records.
pub fn add_two_phase_iterator(
    two_phase: Box<dyn TwoPhaseIterator>,
    members: &mut Vec<ConjunctionMember>,
) {
    members.push(ConjunctionMember::from_two_phase(two_phase));
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

/// Returns the error Java throws when the clauses are not all positioned on the
/// same document.
///
/// Equivalent to the private static
/// `ConjunctionDISI.throwSubIteratorsNotOnSameDocument()`, which throws an
/// `IllegalArgumentException` where Rust returns one for the caller to
/// propagate.
fn throw_sub_iterators_not_on_same_document() -> LuceneError {
    LuceneError::illegal_argument(
        "Sub-iterators of ConjunctionDISI are not on the same document!".to_string(),
    )
}

#[cfg(test)]
mod bit_set_conjunction_tests {
    use super::*;
    use crate::util::FixedBitSet;

    struct Docs {
        docs: Vec<i32>,
        i: usize,
        doc: i32,
    }

    impl DocIdSetIterator for Docs {
        fn doc_id(&self) -> i32 {
            self.doc
        }
        fn next_doc(&mut self) -> Result<i32> {
            self.doc = self.docs.get(self.i).copied().unwrap_or(NO_MORE_DOCS);
            self.i += 1;
            Ok(self.doc)
        }
        fn advance(&mut self, target: i32) -> Result<i32> {
            while self.doc < target {
                self.next_doc()?;
            }
            Ok(self.doc)
        }
        fn cost(&self) -> i64 {
            self.docs.len() as i64
        }
    }

    /// rmp #297: `createConjunction` moves a `BitSetIterator` costlier than
    /// the cheapest clause into a `BitSetConjunctionDISI`, which tests its bit
    /// set instead of advancing it and only positions it, with `setDocId`, on
    /// a match (`ConjunctionDISI.java:113-137, 265-286`): after the lead runs
    /// out, the bit set iterator is still on the last match.
    #[test]
    fn a_costlier_bit_set_iterator_is_tested_not_advanced() {
        let mut set = FixedBitSet::new(6);
        set.set(1);
        set.set(3);
        let lead = Docs {
            docs: vec![1, 5],
            i: 0,
            doc: -1,
        };
        let bit_set_iterator = BitSetIterator::new(Arc::new(set), 10).unwrap();
        let mut conjunction = ConjunctionDISI::new(vec![
            ConjunctionMember::from_iterator(Box::new(lead)),
            ConjunctionMember::from_iterator(Box::new(bit_set_iterator)),
        ])
        .unwrap();
        assert_eq!(conjunction.next_doc().unwrap(), 1);
        assert_eq!(conjunction.member(1).doc_id(), 1);
        assert_eq!(conjunction.next_doc().unwrap(), NO_MORE_DOCS);
        assert_eq!(conjunction.member(1).doc_id(), 1);
        assert_eq!(conjunction.member(0).doc_id(), NO_MORE_DOCS);
        assert_eq!(conjunction.cost(), 2);
    }
}
