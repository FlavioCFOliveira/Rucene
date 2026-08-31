//! Sum scoring for disjunctions, ported from
//! `org.apache.lucene.search.DisjunctionSumScorer`.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::search::disjunction_scorer::DisjunctionScorer;
use crate::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use crate::search::scorable::{ChildScorable, Scorable};
use crate::search::score_mode::ScoreMode;
use crate::search::scorer::Scorer;
use crate::search::two_phase_iterator::TwoPhaseIterator;
use crate::util::MathUtil;

/// A scorer for OR-like queries, the counterpart of
/// [`ConjunctionScorer`](crate::search::ConjunctionScorer).
///
/// Equivalent to the `final class
/// org.apache.lucene.search.DisjunctionSumScorer`. The shared disjunction
/// machinery lives in [`DisjunctionScorer`], which Java expresses as a
/// superclass.
#[derive(Debug)]
pub struct DisjunctionSumScorer {
    base: DisjunctionScorer,
}

impl DisjunctionSumScorer {
    /// Constructs a disjunction scorer over at least two sub-scorers.
    ///
    /// Equivalent to `DisjunctionSumScorer(List<Scorer>, ScoreMode, long)`.
    ///
    /// # Errors
    ///
    /// As [`DisjunctionScorer::new`].
    pub fn new(
        sub_scorers: Vec<Box<dyn Scorer>>,
        score_mode: ScoreMode,
        lead_cost: i64,
    ) -> Result<Self> {
        Ok(Self {
            base: DisjunctionScorer::new(sub_scorers, score_mode, lead_cost)?,
        })
    }
}

impl DocIdSetIterator for DisjunctionSumScorer {
    fn doc_id(&self) -> i32 {
        self.base.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.base.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.base.advance(target)
    }

    fn cost(&self) -> i64 {
        self.base.cost()
    }
}

impl Scorable for DisjunctionSumScorer {
    fn score(&mut self) -> Result<f32> {
        // Equivalent to `DisjunctionScorer.score()`, which is
        // `score(getSubMatches())`, and to the `score(DisiWrapper)` override.
        let mut score = 0.0f64;
        for position in self.base.sub_matches()? {
            score += f64::from(self.base.wrapper(position).scorable().score()?);
        }
        Ok(score as f32)
    }

    fn children(&mut self) -> Result<Vec<ChildScorable<'_>>> {
        self.base.children()
    }
}

impl Scorer for DisjunctionSumScorer {
    fn as_scorable(&mut self) -> &mut dyn Scorable {
        self
    }

    fn doc_id(&self) -> i32 {
        self.base.doc_id()
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        self
    }

    fn two_phase_iterator(&mut self) -> Option<&mut dyn TwoPhaseIterator> {
        if self.base.has_two_phase() {
            Some(&mut self.base)
        } else {
            None
        }
    }

    fn advance_shallow(&mut self, target: i32) -> Result<i32> {
        let mut min = NO_MORE_DOCS;
        for i in 0..self.base.num_clauses() {
            let scorer = self.base.approximation().sub_scorer(i).scorer();
            if Scorer::doc_id(scorer) <= target {
                min = min.min(scorer.advance_shallow(target)?);
            }
        }
        Ok(min)
    }

    fn get_max_score(&mut self, up_to: i32) -> Result<f32> {
        let num_clauses = self.base.num_clauses();
        let mut max_score = 0.0f64;
        for i in 0..num_clauses {
            let scorer = self.base.approximation().sub_scorer(i).scorer();
            if Scorer::doc_id(scorer) <= up_to {
                max_score += f64::from(scorer.get_max_score(up_to)?);
            }
        }
        Ok(MathUtil::sum_upper_bound(max_score, num_clauses as i32) as f32)
    }
}
