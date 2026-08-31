//! Block-max conjunction scoring, ported from
//! `org.apache.lucene.search.BlockMaxConjunctionScorer`.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::search::conjunction_disi::ConjunctionMember;
use crate::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use crate::search::scorable::{ChildScorable, Scorable};
use crate::search::scorer::Scorer;
use crate::search::two_phase_iterator::TwoPhaseIterator;

/// The approximation of a [`BlockMaxConjunctionScorer`], and its two-phase view.
///
/// Equivalent to the anonymous `FilterDocIdSetIterator` that
/// `BlockMaxConjunctionScorer.approximation()` returns, fused with the
/// anonymous `TwoPhaseIterator` that `twoPhaseIterator()` returns.
///
/// **Divergence from Lucene 10.5.0.** Java's anonymous iterator reads the outer
/// scorer's `minScore` field and calls back into its `advanceShallow` and
/// `getMaxScore` — an alias Rust forbids. The clauses and the block-max state
/// therefore live here, and [`BlockMaxConjunctionScorer`] delegates to them,
/// which is the same object graph with the ownership spelled out. It mirrors
/// how [`ConjunctionScorer`](crate::search::ConjunctionScorer) is built on
/// [`ConjunctionDISI`](crate::search::ConjunctionDISI).
struct BlockMaxConjunctionApproximation {
    /// The clauses, in ascending iterator-cost order. Equivalent to the sorted
    /// `Scorer[] scorers` and the `DocIdSetIterator[] approximations` derived
    /// from it, which Java keeps as two parallel arrays.
    members: Vec<ConjunctionMember>,
    /// Positions of the two-phase clauses, in ascending match-cost order.
    two_phase_order: Vec<usize>,
    match_cost: f32,
    min_score: f32,
    max_score: f32,
    up_to: i32,
}

impl BlockMaxConjunctionApproximation {
    /// Equivalent to `BlockMaxConjunctionScorer.advanceShallow(int)`.
    fn advance_shallow_all(&mut self, target: i32) -> Result<i32> {
        // We use block boundaries of the lead scorer. It is tempting to fold in
        // other clauses as well to have better bounds of the score, but then
        // there is a risk of not progressing fast enough.
        let result = self.scorer(0).advance_shallow(target)?;
        // But we still need to shallow-advance other clauses, in order to have
        // better score upper bounds.
        for i in 1..self.members.len() {
            self.scorer(i).advance_shallow(target)?;
        }
        Ok(result)
    }

    /// Equivalent to `BlockMaxConjunctionScorer.getMaxScore(int)`.
    fn max_score_all(&mut self, up_to: i32) -> Result<f32> {
        let mut sum = 0.0f64;
        for i in 0..self.members.len() {
            sum += f64::from(self.scorer(i).get_max_score(up_to)?);
        }
        Ok(sum as f32)
    }

    /// Equivalent to `BlockMaxConjunctionScorer.score()`.
    fn score_all(&mut self) -> Result<f32> {
        let mut score = 0.0f64;
        for i in 0..self.members.len() {
            score += f64::from(self.scorer(i).score()?);
        }
        Ok(score as f32)
    }

    fn scorer(&mut self, position: usize) -> &mut dyn Scorer {
        self.members[position]
            .scorer()
            .expect("INVARIANT: every clause of a BlockMaxConjunctionScorer wraps a Scorer")
    }

    /// Equivalent to the anonymous iterator's private `moveToNextBlock(int)`.
    fn move_to_next_block(&mut self, target: i32) -> Result<()> {
        if self.min_score == 0.0 {
            self.up_to = target;
            self.max_score = f32::INFINITY;
        } else {
            self.up_to = self.advance_shallow_all(target)?;
            let up_to = self.up_to;
            self.max_score = self.max_score_all(up_to)?;
        }
        Ok(())
    }

    /// Equivalent to the anonymous iterator's private `advanceTarget(int)`.
    fn advance_target(&mut self, target: i32) -> Result<i32> {
        let mut target = target;
        if target > self.up_to {
            self.move_to_next_block(target)?;
        }

        loop {
            debug_assert!(self.up_to >= target);

            if self.max_score >= self.min_score {
                return Ok(target);
            }

            if self.up_to == NO_MORE_DOCS {
                return Ok(NO_MORE_DOCS);
            }

            target = self.up_to + 1;

            self.move_to_next_block(target)?;
        }
    }

    /// Equivalent to the anonymous iterator's private `doNext(int)`.
    fn do_next(&mut self, mut doc: i32) -> Result<i32> {
        'advance_head: loop {
            debug_assert_eq!(doc, self.members[0].doc_id());

            if doc == NO_MORE_DOCS {
                return Ok(NO_MORE_DOCS);
            }

            if doc > self.up_to {
                // This check is useful when scorers return information about
                // blocks that do not actually have any matches. Otherwise `doc`
                // will always be in the current block already since it is
                // always the result of lead.advance(advanceTarget(some_doc_id)).
                let next_target = self.advance_target(doc)?;
                if next_target != doc {
                    doc = self.members[0].advance(next_target)?;
                    continue;
                }
            }

            debug_assert!(doc <= self.up_to);

            // then find agreement with other iterators
            for i in 1..self.members.len() {
                // other.doc may already be equal to doc if we "continued
                // advanceHead" on the previous iteration and the advance on the
                // lead scorer exactly matched.
                if self.members[i].doc_id() < doc {
                    let next = self.members[i].advance(doc)?;

                    if next > doc {
                        // iterator beyond the current doc - advance lead and
                        // continue to the new highest doc.
                        let target = self.advance_target(next)?;
                        doc = self.members[0].advance(target)?;
                        continue 'advance_head;
                    }
                }

                debug_assert_eq!(self.members[i].doc_id(), doc);
            }

            // success - all iterators are on the same doc and the score is
            // competitive
            return Ok(doc);
        }
    }

    fn has_two_phase(&self) -> bool {
        !self.two_phase_order.is_empty()
    }
}

impl DocIdSetIterator for BlockMaxConjunctionApproximation {
    fn doc_id(&self) -> i32 {
        self.members[0].doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        let target = self.doc_id() + 1;
        self.advance(target)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let target = self.advance_target(target)?;
        let doc = self.members[0].advance(target)?;
        self.do_next(doc)
    }

    fn cost(&self) -> i64 {
        self.members[0].cost()
    }
}

impl TwoPhaseIterator for BlockMaxConjunctionApproximation {
    fn approximation(&mut self) -> &mut dyn DocIdSetIterator {
        self
    }

    fn approximation_ref(&self) -> &dyn DocIdSetIterator {
        self
    }

    fn matches(&mut self) -> Result<bool> {
        for k in 0..self.two_phase_order.len() {
            let position = self.two_phase_order[k];
            debug_assert_eq!(self.members[position].doc_id(), self.members[0].doc_id());
            if !self.members[position].matches()? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn match_cost(&self) -> f32 {
        self.match_cost
    }
}

/// Scorer for conjunctions that checks the maximum score of each clause in
/// order to potentially skip over blocks that cannot have competitive matches.
///
/// Equivalent to the `final org.apache.lucene.search.BlockMaxConjunctionScorer`,
/// which is package-private in Java; it is public here because Rust has no
/// package visibility and
/// [`BooleanScorerSupplier`](crate::search::BooleanScorerSupplier), which
/// builds it, lives in a sibling module.
pub struct BlockMaxConjunctionScorer {
    approximation: BlockMaxConjunctionApproximation,
}

impl std::fmt::Debug for BlockMaxConjunctionScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlockMaxConjunctionScorer")
            .field("clauses", &self.approximation.members.len())
            .finish_non_exhaustive()
    }
}

impl BlockMaxConjunctionScorer {
    /// Creates a new block-max conjunction scorer from scoring clauses.
    ///
    /// Equivalent to `BlockMaxConjunctionScorer(Collection<Scorer>)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while priming the score bounds, which is
    /// what Java's `scorer.advanceShallow(0)` may throw.
    pub fn new(scorers: Vec<Box<dyn Scorer>>) -> Result<Self> {
        let mut members: Vec<ConjunctionMember> = scorers
            .into_iter()
            .map(ConjunctionMember::from_scorer)
            .collect();
        // Sort scorers by cost. `Arrays.sort` on objects is stable, and so is
        // `sort_by`.
        members.sort_by_key(ConjunctionMember::cost);

        for member in &mut members {
            let scorer = member
                .scorer()
                .expect("INVARIANT: every clause of a BlockMaxConjunctionScorer wraps a Scorer");
            scorer.advance_shallow(0)?;
        }

        let mut two_phase_order: Vec<usize> = (0..members.len())
            .filter(|i| members[*i].has_two_phase())
            .collect();
        two_phase_order.sort_by(|a, b| {
            members[*a]
                .match_cost()
                .total_cmp(&members[*b].match_cost())
        });

        let mut match_cost = 0.0f64;
        for position in &two_phase_order {
            match_cost += f64::from(members[*position].match_cost());
        }

        Ok(Self {
            approximation: BlockMaxConjunctionApproximation {
                members,
                two_phase_order,
                match_cost: match_cost as f32,
                min_score: 0.0,
                max_score: 0.0,
                up_to: -1,
            },
        })
    }

    /// Confirms the current candidate, walking forward until one matches.
    ///
    /// Equivalent to what
    /// `TwoPhaseIterator.asDocIdSetIterator(twoPhaseIterator())` does around the
    /// approximation when at least one clause is two-phase.
    fn confirm(&mut self, mut doc: i32) -> Result<i32> {
        loop {
            if doc == NO_MORE_DOCS {
                return Ok(NO_MORE_DOCS);
            }
            if self.approximation.matches()? {
                return Ok(doc);
            }
            doc = DocIdSetIterator::next_doc(&mut self.approximation)?;
        }
    }
}

impl DocIdSetIterator for BlockMaxConjunctionScorer {
    fn doc_id(&self) -> i32 {
        self.approximation.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        let doc = DocIdSetIterator::next_doc(&mut self.approximation)?;
        if self.approximation.has_two_phase() {
            self.confirm(doc)
        } else {
            Ok(doc)
        }
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let doc = DocIdSetIterator::advance(&mut self.approximation, target)?;
        if self.approximation.has_two_phase() {
            self.confirm(doc)
        } else {
            Ok(doc)
        }
    }

    fn cost(&self) -> i64 {
        DocIdSetIterator::cost(&self.approximation)
    }
}

impl Scorable for BlockMaxConjunctionScorer {
    fn score(&mut self) -> Result<f32> {
        self.approximation.score_all()
    }

    fn set_min_competitive_score(&mut self, min_score: f32) -> Result<()> {
        self.approximation.min_score = min_score;
        Ok(())
    }

    fn children(&mut self) -> Result<Vec<ChildScorable<'_>>> {
        let mut children = Vec::with_capacity(self.approximation.members.len());
        for member in &mut self.approximation.members {
            let scorer = member
                .scorer()
                .expect("INVARIANT: every clause of a BlockMaxConjunctionScorer wraps a Scorer");
            children.push(ChildScorable::new(scorer.as_scorable(), "MUST"));
        }
        Ok(children)
    }
}

impl Scorer for BlockMaxConjunctionScorer {
    fn as_scorable(&mut self) -> &mut dyn Scorable {
        self
    }

    fn doc_id(&self) -> i32 {
        DocIdSetIterator::doc_id(self)
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        self
    }

    fn two_phase_iterator(&mut self) -> Option<&mut dyn TwoPhaseIterator> {
        if self.approximation.has_two_phase() {
            Some(&mut self.approximation)
        } else {
            None
        }
    }

    fn advance_shallow(&mut self, target: i32) -> Result<i32> {
        self.approximation.advance_shallow_all(target)
    }

    fn get_max_score(&mut self, up_to: i32) -> Result<f32> {
        self.approximation.max_score_all(up_to)
    }
}
