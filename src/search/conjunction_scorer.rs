//! Conjunction scoring, ported from
//! `org.apache.lucene.search.ConjunctionScorer`.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::search::conjunction_disi::{ConjunctionDISI, ConjunctionMember};
use crate::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use crate::search::scorable::{ChildScorable, Scorable};
use crate::search::scorer::Scorer;
use crate::search::two_phase_iterator::TwoPhaseIterator;

/// Scorer for conjunctions: sets of queries, all of which are required.
///
/// Equivalent to `org.apache.lucene.search.ConjunctionScorer`.
///
/// **Divergence from Lucene 10.5.0.** Java's constructor takes two overlapping
/// collections, `required` and the `scorers` subset of it that participates in
/// scoring, and holds a reference to every scorer twice — once in those
/// collections and once inside the `ConjunctionDISI` built from them. Rust
/// cannot alias, so the constructor takes the required scorers once and the
/// *positions* of the scoring subset within them. The clauses, their order, and
/// therefore the order in which their scores are summed are identical.
pub struct ConjunctionScorer {
    disi: ConjunctionDISI,
    /// Positions, within the conjunction's clauses, of the scorers that
    /// contribute to the score. Equivalent to the `Scorer[] scorers` field.
    scoring: Vec<usize>,
}

impl std::fmt::Debug for ConjunctionScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConjunctionScorer")
            .field("scoring", &self.scoring.len())
            .finish_non_exhaustive()
    }
}

impl ConjunctionScorer {
    /// Creates a new conjunction scorer.
    ///
    /// Equivalent to `ConjunctionScorer(Collection<Scorer>,
    /// Collection<Scorer>)`, where `scoring` names the positions within
    /// `required` of the scorers that Java passes as the second argument.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`](crate::error::LuceneError::IllegalArgument)
    /// when fewer than two clauses are supplied or when they are not all on the
    /// same document.
    pub fn new(required: Vec<Box<dyn Scorer>>, scoring: Vec<usize>) -> Result<Self> {
        debug_assert!(
            scoring.iter().all(|position| *position < required.len()),
            "scorers must be a subset of required"
        );
        let members = required
            .into_iter()
            .map(ConjunctionMember::from_scorer)
            .collect();
        Ok(Self {
            disi: ConjunctionDISI::new(members)?,
            scoring,
        })
    }

    /// Returns the clause at `position`.
    fn clause(&mut self, position: usize) -> &mut ConjunctionMember {
        self.disi.member(position)
    }

    /// Returns the scorer of the clause at `position`.
    fn clause_scorer(&mut self, position: usize) -> &mut dyn Scorer {
        self.clause(position)
            .scorer()
            .expect("INVARIANT: every clause of a ConjunctionScorer wraps a Scorer")
    }

    /// Confirms the current candidate, walking forward until one matches.
    ///
    /// Equivalent to what
    /// `TwoPhaseIterator.asDocIdSetIterator(ConjunctionTwoPhaseIterator)` does
    /// around the conjunction when at least one clause is two-phase.
    fn confirm(&mut self, mut doc: i32) -> Result<i32> {
        loop {
            if doc == NO_MORE_DOCS {
                return Ok(NO_MORE_DOCS);
            }
            if self.disi.matches()? {
                return Ok(doc);
            }
            doc = DocIdSetIterator::next_doc(&mut self.disi)?;
        }
    }
}

impl DocIdSetIterator for ConjunctionScorer {
    fn doc_id(&self) -> i32 {
        self.disi.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        let doc = DocIdSetIterator::next_doc(&mut self.disi)?;
        if self.disi.has_two_phase() {
            self.confirm(doc)
        } else {
            Ok(doc)
        }
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let doc = DocIdSetIterator::advance(&mut self.disi, target)?;
        if self.disi.has_two_phase() {
            self.confirm(doc)
        } else {
            Ok(doc)
        }
    }

    fn cost(&self) -> i64 {
        DocIdSetIterator::cost(&self.disi)
    }
}

impl Scorable for ConjunctionScorer {
    fn score(&mut self) -> Result<f32> {
        let mut sum = 0.0f64;
        for i in 0..self.scoring.len() {
            let position = self.scoring[i];
            sum += f64::from(self.clause_scorer(position).score()?);
        }
        Ok(sum as f32)
    }

    fn set_min_competitive_score(&mut self, min_score: f32) -> Result<()> {
        // This scorer is only used for TOP_SCORES when there is a single
        // scoring clause.
        if self.scoring.len() == 1 {
            let position = self.scoring[0];
            self.clause_scorer(position)
                .set_min_competitive_score(min_score)?;
        }
        Ok(())
    }

    fn children(&mut self) -> Result<Vec<ChildScorable<'_>>> {
        let mut children = Vec::with_capacity(self.disi.len());
        for member in self.disi.members() {
            let scorer = member
                .scorer()
                .expect("INVARIANT: every clause of a ConjunctionScorer wraps a Scorer");
            children.push(ChildScorable::new(scorer.as_scorable(), "MUST"));
        }
        Ok(children)
    }
}

impl Scorer for ConjunctionScorer {
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
        if self.disi.has_two_phase() {
            Some(&mut self.disi)
        } else {
            None
        }
    }

    fn advance_shallow(&mut self, target: i32) -> Result<i32> {
        if self.scoring.len() == 1 {
            let position = self.scoring[0];
            return self.clause_scorer(position).advance_shallow(target);
        }
        for i in 0..self.scoring.len() {
            let position = self.scoring[i];
            self.clause_scorer(position).advance_shallow(target)?;
        }
        Ok(NO_MORE_DOCS)
    }

    fn get_max_score(&mut self, up_to: i32) -> Result<f32> {
        let mut max_score = 0.0f64;
        for i in 0..self.scoring.len() {
            let position = self.scoring[i];
            let scorer = self.clause_scorer(position);
            if Scorer::doc_id(scorer) <= up_to {
                max_score += f64::from(scorer.get_max_score(up_to)?);
            }
        }
        Ok(max_score as f32)
    }
}
