//! Disjunction scoring machinery, ported from
//! `org.apache.lucene.search.DisjunctionScorer`.

#![deny(unsafe_code)]

use crate::error::{LuceneError, Result};
use crate::search::disi_wrapper::DisiWrapper;
use crate::search::disjunction_disi_approximation::DisjunctionDISIApproximation;
use crate::search::disjunction_score_block_boundary_propagator::SubScorers;
use crate::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use crate::search::index_priority_queue::{IndexOrder, IndexPriorityQueue};
use crate::search::scorable::ChildScorable;
use crate::search::score_mode::ScoreMode;
use crate::search::scorer::Scorer;
use crate::search::two_phase_iterator::TwoPhaseIterator;

/// Orders unverified matches by ascending match cost, so that the cheapest
/// confirmation runs first.
///
/// Equivalent to the anonymous `PriorityQueue<DisiWrapper>` of
/// `DisjunctionScorer.TwoPhase`, whose `lessThan` is
/// `a.matchCost < b.matchCost`.
#[derive(Debug)]
pub struct ByMatchCost;

impl IndexOrder<DisiWrapper> for ByMatchCost {
    fn less_than(a: &DisiWrapper, b: &DisiWrapper) -> bool {
        a.match_cost < b.match_cost
    }
}

/// The machinery shared by the scorers that score disjunctions.
///
/// Equivalent to the `abstract class
/// org.apache.lucene.search.DisjunctionScorer` fused with its private inner
/// `TwoPhase` class: in Java the two-phase iterator is an inner object that
/// reads and writes the outer scorer's state, which Rust cannot express, so the
/// state lives here once and this type is both.
///
/// **Divergence from Lucene 10.5.0.** Java expresses "score the linked list of
/// clauses positioned on the current doc" as an abstract method that a subclass
/// overrides. Rust has no implementation inheritance, so
/// [`DisjunctionSumScorer`](crate::search::DisjunctionSumScorer) and
/// [`DisjunctionMaxScorer`](crate::search::DisjunctionMaxScorer) hold one of
/// these and implement their own `score()` over the positions
/// [`sub_matches`](Self::sub_matches) returns. The linked list itself becomes a
/// vector of positions in the very order Java's list is traversed, so the order
/// in which sub-scores are summed is unchanged.
pub struct DisjunctionScorer {
    approximation: DisjunctionDISIApproximation,
    num_clauses: usize,
    needs_scores: bool,
    /// Whether any sub-scorer supports approximations; Java's `twoPhase` field
    /// is `null` when none does.
    has_two_phase: bool,
    match_cost: f32,
    /// Verified matches on the current doc, in the order they were prepended to
    /// Java's linked list; the traversal order is the reverse.
    verified_matches: Vec<usize>,
    /// Approximations on the current doc that have not been verified yet.
    unverified_matches: IndexPriorityQueue<DisiWrapper, ByMatchCost>,
}

impl std::fmt::Debug for DisjunctionScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DisjunctionScorer")
            .field("num_clauses", &self.num_clauses)
            .field("has_two_phase", &self.has_two_phase)
            .finish_non_exhaustive()
    }
}

impl DisjunctionScorer {
    /// Builds the machinery over the given sub-scorers.
    ///
    /// Equivalent to `DisjunctionScorer(List<Scorer>, ScoreMode, long)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when fewer than two sub-scorers
    /// are supplied, matching Java's `IllegalArgumentException`.
    pub fn new(
        sub_scorers: Vec<Box<dyn Scorer>>,
        score_mode: ScoreMode,
        lead_cost: i64,
    ) -> Result<Self> {
        if sub_scorers.len() <= 1 {
            return Err(LuceneError::IllegalArgument(
                "There must be at least 2 subScorers".to_string(),
            ));
        }
        let num_clauses = sub_scorers.len();
        let needs_scores = score_mode != ScoreMode::COMPLETE_NO_SCORES;
        let mut has_approximation = false;
        let mut sum_match_cost = 0.0f32;
        let mut sum_approx_cost: i64 = 0;
        let mut wrappers = Vec::with_capacity(num_clauses);
        for scorer in sub_scorers {
            let w = DisiWrapper::new(scorer, false);
            let cost_weight = if w.cost <= 1 { 1 } else { w.cost };
            sum_approx_cost = sum_approx_cost.wrapping_add(cost_weight);
            if w.has_two_phase() {
                has_approximation = true;
                sum_match_cost += w.match_cost * cost_weight as f32;
            }
            wrappers.push(w);
        }
        let approximation = DisjunctionDISIApproximation::new(wrappers, lead_cost);

        let match_cost = if has_approximation {
            sum_match_cost / sum_approx_cost as f32
        } else {
            0.0
        };

        Ok(Self {
            approximation,
            num_clauses,
            needs_scores,
            has_two_phase: has_approximation,
            match_cost,
            verified_matches: Vec::with_capacity(num_clauses),
            unverified_matches: IndexPriorityQueue::new(num_clauses),
        })
    }

    /// Returns the number of clauses.
    ///
    /// Equivalent to reading the `private final int numClauses` field.
    pub fn num_clauses(&self) -> usize {
        self.num_clauses
    }

    /// Returns whether any clause supports two-phase iteration, that is,
    /// whether Java's `twoPhase` field is non-`null`.
    pub fn has_two_phase(&self) -> bool {
        self.has_two_phase
    }

    /// Returns the disjunction of the clause approximations.
    ///
    /// Equivalent to reading the `private final DisjunctionDISIApproximation
    /// approximation` field.
    pub fn approximation(&mut self) -> &mut DisjunctionDISIApproximation {
        &mut self.approximation
    }

    /// Returns the clause at `position`, where the positions are those
    /// [`sub_matches`](Self::sub_matches) hands out.
    pub fn wrapper(&mut self, position: usize) -> &mut DisiWrapper {
        self.approximation.wrapper(position)
    }

    /// Returns the current doc ID.
    ///
    /// Equivalent to the `final DisjunctionScorer.docID()`.
    pub fn doc_id(&self) -> i32 {
        self.approximation.doc_id()
    }

    /// Returns the cost of the disjunction.
    pub fn cost(&self) -> i64 {
        self.approximation.cost()
    }

    /// Returns the match cost of the two-phase view.
    ///
    /// Equivalent to `DisjunctionScorer.TwoPhase.matchCost()`.
    pub fn match_cost(&self) -> f32 {
        self.match_cost
    }

    /// Returns the positions of the clauses that match the current doc, in the
    /// order in which Java traverses its linked list.
    ///
    /// Equivalent to the package-private `DisjunctionScorer.getSubMatches()`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while confirming matches.
    pub fn sub_matches(&mut self) -> Result<Vec<usize>> {
        if !self.has_two_phase {
            Ok(self.approximation.top_list())
        } else {
            self.two_phase_sub_matches()
        }
    }

    /// Equivalent to `DisjunctionScorer.TwoPhase.getSubMatches()`.
    fn two_phase_sub_matches(&mut self) -> Result<Vec<usize>> {
        // iteration order does not matter
        let unverified: Vec<usize> = self.unverified_matches.entries().to_vec();
        for position in unverified {
            if self.approximation.wrapper(position).matches()? {
                self.verified_matches.push(position);
            }
        }
        self.unverified_matches.clear();
        debug_assert!(self
            .verified_matches
            .iter()
            .all(|position| self.approximation.wrappers()[*position].doc == self.doc_id()));
        let mut matches = self.verified_matches.clone();
        matches.reverse();
        Ok(matches)
    }

    /// Equivalent to `DisjunctionScorer.TwoPhase.matches()`.
    fn two_phase_matches(&mut self) -> Result<bool> {
        self.verified_matches.clear();
        self.unverified_matches.clear();

        for position in self.approximation.top_list() {
            if !self.approximation.wrappers()[position].has_two_phase() {
                // implicitly verified, move it to verifiedMatches
                self.verified_matches.push(position);

                if !self.needs_scores {
                    // we can stop here
                    return Ok(true);
                }
            } else {
                self.unverified_matches
                    .add(self.approximation.wrappers(), position);
            }
        }

        if !self.verified_matches.is_empty() {
            return Ok(true);
        }

        // verify subs that have a two-phase iterator, least-costly ones first
        while self.unverified_matches.size() > 0 {
            let position = self
                .unverified_matches
                .pop(self.approximation.wrappers())
                .expect("INVARIANT: the queue was just observed to be non-empty");
            if self.approximation.wrapper(position).matches()? {
                self.verified_matches.clear();
                self.verified_matches.push(position);
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Advances to the next matching document.
    ///
    /// Equivalent to iterating the value `DisjunctionScorer.iterator()`
    /// returns: the approximation itself when no clause is two-phase, the
    /// confirming view otherwise.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while iterating or confirming.
    pub fn next_doc(&mut self) -> Result<i32> {
        let doc = self.approximation.next_doc()?;
        if self.has_two_phase {
            self.confirm(doc)
        } else {
            Ok(doc)
        }
    }

    /// Advances to the first matching document on or after `target`.
    ///
    /// See [`next_doc`](Self::next_doc).
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while iterating or confirming.
    pub fn advance(&mut self, target: i32) -> Result<i32> {
        let doc = self.approximation.advance(target)?;
        if self.has_two_phase {
            self.confirm(doc)
        } else {
            Ok(doc)
        }
    }

    fn confirm(&mut self, mut doc: i32) -> Result<i32> {
        loop {
            if doc == NO_MORE_DOCS {
                return Ok(NO_MORE_DOCS);
            }
            if self.two_phase_matches()? {
                return Ok(doc);
            }
            doc = self.approximation.next_doc()?;
        }
    }

    /// Returns the child sub-scorers positioned on the current document.
    ///
    /// Equivalent to the `final DisjunctionScorer.getChildren()`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java walks its linked list and hands
    /// out a reference per node. Rust cannot produce several exclusive borrows
    /// by repeated indexing, so the clauses are taken from one `iter_mut` pass
    /// and then put back into the traversal order Java's list has. The set of
    /// children and their order are identical.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while confirming matches.
    pub fn children(&mut self) -> Result<Vec<ChildScorable<'_>>> {
        let matches = self.sub_matches()?;
        let mut rank = vec![usize::MAX; self.num_clauses];
        for (position_in_list, position) in matches.iter().enumerate() {
            rank[*position] = position_in_list;
        }
        let mut selected: Vec<(usize, &mut DisiWrapper)> = self
            .approximation
            .wrappers_mut()
            .iter_mut()
            .enumerate()
            .filter(|(position, _)| rank[*position] != usize::MAX)
            .collect();
        selected.sort_by_key(|(position, _)| rank[*position]);
        Ok(selected
            .into_iter()
            .map(|(_, wrapper)| ChildScorable::new(wrapper.scorable(), "SHOULD"))
            .collect())
    }
}

impl TwoPhaseIterator for DisjunctionScorer {
    fn approximation(&mut self) -> &mut dyn DocIdSetIterator {
        &mut self.approximation
    }

    fn approximation_ref(&self) -> &dyn DocIdSetIterator {
        &self.approximation
    }

    fn matches(&mut self) -> Result<bool> {
        self.two_phase_matches()
    }

    fn match_cost(&self) -> f32 {
        self.match_cost
    }
}

impl SubScorers for DisjunctionScorer {
    fn len(&self) -> usize {
        self.num_clauses
    }

    fn sub_scorer(&mut self, index: usize) -> &mut dyn Scorer {
        self.approximation.sub_scorer(index).scorer()
    }
}
