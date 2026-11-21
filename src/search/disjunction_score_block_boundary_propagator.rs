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

        // `Arrays.sort(this.scorers, MAX_SCORE_COMPARATOR)`
        // (`DisjunctionScoreBlockBoundaryPropagator.java:33-42`, `54`): the
        // comparator reads `getMaxScore(NO_MORE_DOCS)` of both scorers on
        // every comparison and `iterator().cost()` only on a tie. Java wraps
        // an `IOException` thrown inside the comparator; the first error is
        // kept here and propagated once the sort returns.
        let mut order: Vec<usize> = (0..len).collect();
        let mut error: Option<crate::error::LuceneError> = None;
        // `Arrays.sort` on objects is stable, and so is `sort_by`.
        // `Float.compareTo` orders `-0.0` below `0.0` and `NaN` above
        // everything, which is exactly `f32::total_cmp`.
        order.sort_by(|&a, &b| {
            if error.is_some() {
                return std::cmp::Ordering::Equal;
            }
            let max_a = scorers.sub_scorer(a).get_max_score(NO_MORE_DOCS);
            let max_b = scorers.sub_scorer(b).get_max_score(NO_MORE_DOCS);
            let (max_a, max_b) = match (max_a, max_b) {
                (Ok(x), Ok(y)) => (x, y),
                (Err(e), _) | (_, Err(e)) => {
                    error = Some(e);
                    return std::cmp::Ordering::Equal;
                }
            };
            max_a.total_cmp(&max_b).then_with(|| {
                let cost_a = scorers.sub_scorer(a).iterator().cost();
                let cost_b = scorers.sub_scorer(b).iterator().cost();
                cost_a.cmp(&cost_b)
            })
        });
        if let Some(e) = error {
            return Err(e);
        }

        let mut max_scores = Vec::with_capacity(len);
        for &i in &order {
            max_scores.push(scorers.sub_scorer(i).get_max_score(NO_MORE_DOCS)?);
        }

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
