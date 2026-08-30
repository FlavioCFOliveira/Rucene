//! Block-boundary propagation for disjunctions, ported from
//! `org.apache.lucene.search.DisjunctionScoreBlockBoundaryPropagator`.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::search::doc_id_set_iterator::NO_MORE_DOCS;
use crate::search::scorer::Scorer;

/// The sub-scorers a [`DisjunctionScoreBlockBoundaryPropagator`] propagates to.
///
/// **Divergence from Lucene 10.5.0.** Java's propagator keeps its own
/// `Scorer[]`, holding a second reference to the very scorers its owner also
/// drives through their `DisiWrapper`s. Rust forbids that aliasing, so the
/// propagator stores only the *order* it sorted them into and asks its owner
/// for a scorer by index whenever it needs one. The scorers, the ordering and
/// the calls made on them are unchanged.
pub trait SubScorers {
    /// Returns the number of sub-scorers.
    fn len(&self) -> usize;

    /// Returns whether there is no sub-scorer at all.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the sub-scorer at `index`, in the order the propagator was
    /// built with.
    fn sub_scorer(&mut self, index: usize) -> &mut dyn Scorer;
}

/// Propagates block boundaries across the clauses of a disjunction.
///
/// Equivalent to the `final class
/// org.apache.lucene.search.DisjunctionScoreBlockBoundaryPropagator`.
///
/// Because a disjunction matches if any of its sub clauses matches, it is
/// tempting to return the minimum block boundary across all clauses. The
/// problem is that it might then make the query slow when the minimum
/// competitive score is high and low-scoring clauses do not drive iteration any
/// more. This class therefore computes block boundaries only across clauses
/// whose maximum score is greater than or equal to the minimum competitive
/// score, or the maximum scoring clause if there is no such clause.
#[derive(Debug)]
pub struct DisjunctionScoreBlockBoundaryPropagator {
    /// Positions of the sub-scorers, ordered as Java's sorted `scorers` array.
    order: Vec<usize>,
    max_scores: Vec<f32>,
    lead_index: usize,
}

impl DisjunctionScoreBlockBoundaryPropagator {
    /// Builds a propagator over the given scorers.
    ///
    /// Equivalent to
    /// `DisjunctionScoreBlockBoundaryPropagator(Collection<Scorer>)`, which
    /// shallow-advances every scorer to `0` and then sorts them by maximum
    /// score and, on ties, by cost.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while shallow-advancing or reading the
    /// maximum scores.
    pub fn new(scorers: &mut dyn SubScorers) -> Result<Self> {
        let len = scorers.len();
        for i in 0..len {
            scorers.sub_scorer(i).advance_shallow(0)?;
        }

        // The Java comparator reads `getMaxScore(NO_MORE_DOCS)` and, on ties,
        // `iterator().cost()`; both are read once here so that the sort has no
        // side effects.
        let mut keys: Vec<(f32, i64)> = Vec::with_capacity(len);
        for i in 0..len {
            let scorer = scorers.sub_scorer(i);
            let max_score = scorer.get_max_score(NO_MORE_DOCS)?;
            let cost = scorer.iterator().cost();
            keys.push((max_score, cost));
        }

        let mut order: Vec<usize> = (0..len).collect();
        // `Arrays.sort` on objects is stable, and so is `sort_by`.
        // `Float.compareTo` orders `-0.0` below `0.0` and `NaN` above
        // everything, which is exactly `f32::total_cmp`.
        order.sort_by(|a, b| {
            keys[*a]
                .0
                .total_cmp(&keys[*b].0)
                .then_with(|| keys[*a].1.cmp(&keys[*b].1))
        });

        let max_scores = order.iter().map(|i| keys[*i].0).collect();

        Ok(Self {
            order,
            max_scores,
            lead_index: 0,
        })
    }

    /// Propagates a shallow advance and returns the block boundary.
    ///
    /// Equivalent to the package-private
    /// `DisjunctionScoreBlockBoundaryPropagator.advanceShallow(int)`; see
    /// [`Scorer::advance_shallow`].
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while shallow-advancing.
    pub fn advance_shallow(&mut self, scorers: &mut dyn SubScorers, target: i32) -> Result<i32> {
        // For scorers that are below the lead index, just propagate.
        for i in 0..self.lead_index {
            let scorer = scorers.sub_scorer(self.order[i]);
            if Scorer::doc_id(scorer) < target {
                scorer.advance_shallow(target)?;
            }
        }

        // For scorers above the lead index, we take the minimum boundary.
        let lead_scorer = scorers.sub_scorer(self.order[self.lead_index]);
        let lead_doc = Scorer::doc_id(lead_scorer);
        let mut up_to = lead_scorer.advance_shallow(lead_doc.max(target))?;

        for i in (self.lead_index + 1)..self.order.len() {
            let scorer = scorers.sub_scorer(self.order[i]);
            if Scorer::doc_id(scorer) <= target {
                up_to = scorer.advance_shallow(target)?.min(up_to);
            }
        }

        // If the maximum scoring clauses are beyond `target`, then we use their
        // docID as a boundary. It helps not consider them when computing the
        // maximum score and get a lower score upper bound.
        for i in (self.lead_index + 1..self.order.len()).rev() {
            let scorer = scorers.sub_scorer(self.order[i]);
            let doc = Scorer::doc_id(scorer);
            if doc > target {
                up_to = up_to.min(doc - 1);
            } else {
                break;
            }
        }

        Ok(up_to)
    }

    /// Sets the minimum competitive score, filtering out clauses that score
    /// less than this threshold.
    ///
    /// Equivalent to the package-private
    /// `DisjunctionScoreBlockBoundaryPropagator.setMinCompetitiveScore(float)`;
    /// see [`Scorable::set_min_competitive_score`](crate::search::Scorable::set_min_competitive_score).
    pub fn set_min_competitive_score(&mut self, min_score: f32) {
        // Update the lead index if necessary
        while self.lead_index + 1 < self.max_scores.len()
            && min_score > self.max_scores[self.lead_index]
        {
            self.lead_index += 1;
        }
    }
}
