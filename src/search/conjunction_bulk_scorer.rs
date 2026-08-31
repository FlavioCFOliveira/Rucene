//! Conjunction bulk scoring, ported from
//! `org.apache.lucene.search.ConjunctionBulkScorer`.

#![deny(unsafe_code)]

use crate::error::{LuceneError, Result};
use crate::search::bulk_scorer::BulkScorer;
use crate::search::collection_terminated_exception::CollectionResult;
use crate::search::collector::LeafCollector;
use crate::search::doc_id_set_iterator::DocIdSetIterator;
use crate::search::scorable::{ChildScorable, Scorable};
use crate::search::scorer::Scorer;
use crate::util::Bits;

/// A [`BulkScorer`] implementation of
/// [`ConjunctionScorer`](crate::search::ConjunctionScorer).
///
/// Equivalent to `org.apache.lucene.search.ConjunctionBulkScorer`, which is
/// package-private in Java; it is public here because Rust has no package
/// visibility and
/// [`BooleanScorerSupplier`](crate::search::BooleanScorerSupplier), which
/// builds it, lives in a sibling module. For simplicity it focuses on scorers
/// that produce regular [`DocIdSetIterator`]s and not
/// [`TwoPhaseIterator`](crate::search::TwoPhaseIterator)s.
///
/// **Divergence from Lucene 10.5.0.** Java holds each clause three times — in
/// `allScorers`, in the `scoringScorers` subset, and as the iterator it pulled
/// out of it — and hands the collector an anonymous `Scorable` that reads the
/// first two. Rust cannot alias, so the clauses are owned once and the two
/// subsets become lists of *positions* into them; the scorer handed to the
/// collector is this bulk scorer itself, whose
/// [`Scorable`] implementation is Java's anonymous class. The order in which
/// the clause scores are summed, and the order in which the iterators are
/// visited, are unchanged.
pub struct ConjunctionBulkScorer {
    scorers: Vec<Box<dyn Scorer>>,
    /// Positions, within `scorers`, of the clauses that contribute to the
    /// score. Equivalent to the `Scorable[] scoringScorers` field.
    scoring: Vec<usize>,
    /// Positions, within `scorers`, in ascending iterator-cost order: `[0]` is
    /// Java's `lead1`, `[1]` its `lead2`, the rest its `others`.
    order: Vec<usize>,
    cost: i64,
}

impl std::fmt::Debug for ConjunctionBulkScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConjunctionBulkScorer")
            .field("clauses", &self.scorers.len())
            .field("scoring", &self.scoring.len())
            .field("cost", &self.cost)
            .finish()
    }
}

impl ConjunctionBulkScorer {
    /// Builds a conjunction bulk scorer.
    ///
    /// Equivalent to `ConjunctionBulkScorer(List<Scorer>, List<Scorer>)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when fewer than two clauses are
    /// supplied, which is the `IllegalArgumentException` Java throws.
    pub fn new(
        required_scoring: Vec<Box<dyn Scorer>>,
        required_no_scoring: Vec<Box<dyn Scorer>>,
    ) -> Result<Self> {
        let num_clauses = required_scoring.len() + required_no_scoring.len();
        if num_clauses <= 1 {
            return Err(LuceneError::IllegalArgument(format!(
                "Expected 2 or more clauses, got {num_clauses}"
            )));
        }
        let scoring: Vec<usize> = (0..required_scoring.len()).collect();
        let mut scorers = required_scoring;
        scorers.extend(required_no_scoring);

        let mut costs = Vec::with_capacity(scorers.len());
        for scorer in &mut scorers {
            costs.push(scorer.iterator().cost());
        }
        // `Collections.sort` is stable, and so is `sort_by`.
        let mut order: Vec<usize> = (0..scorers.len()).collect();
        order.sort_by(|a, b| costs[*a].cmp(&costs[*b]));

        let cost = costs[order[0]];
        Ok(Self {
            scorers,
            scoring,
            order,
            cost,
        })
    }

    fn iterator(&mut self, position: usize) -> &mut dyn DocIdSetIterator {
        self.scorers[position].iterator()
    }

    fn doc_id(&self, position: usize) -> i32 {
        self.scorers[position].doc_id()
    }
}

/// Advances every clause of `others`, and then the collector's competitive
/// iterator, onto `doc`.
///
/// Returns the doc ID iteration must resume from when one of them is beyond
/// `doc`, or `None` when they all agree — which is what Java's `break` out of
/// the `others` loop and its `continue advanceHead` express.
fn align_others(
    scorers: &mut [Box<dyn Scorer>],
    others: &[usize],
    competitive: &mut Option<Box<dyn DocIdSetIterator>>,
    doc: i32,
) -> Result<Option<i32>> {
    for position in others {
        let iterator = scorers[*position].iterator();
        if iterator.doc_id() < doc {
            let next = iterator.advance(doc)?;
            if next != doc {
                return Ok(Some(next));
            }
        }
        debug_assert_eq!(scorers[*position].doc_id(), doc);
    }
    if let Some(competitive) = competitive.as_deref_mut() {
        if competitive.doc_id() < doc {
            let next = competitive.advance(doc)?;
            if next != doc {
                return Ok(Some(next));
            }
        }
        debug_assert_eq!(competitive.doc_id(), doc);
    }
    Ok(None)
}

impl Scorable for ConjunctionBulkScorer {
    fn score(&mut self) -> Result<f32> {
        let mut score = 0.0f64;
        for i in 0..self.scoring.len() {
            let position = self.scoring[i];
            score += f64::from(self.scorers[position].score()?);
        }
        Ok(score as f32)
    }

    fn children(&mut self) -> Result<Vec<ChildScorable<'_>>> {
        let mut children = Vec::with_capacity(self.scorers.len());
        for scorer in &mut self.scorers {
            children.push(ChildScorable::new(scorer.as_scorable(), "MUST"));
        }
        Ok(children)
    }
}

impl BulkScorer for ConjunctionBulkScorer {
    fn cost(&self) -> i64 {
        self.cost
    }

    fn score(
        &mut self,
        collector: &mut dyn LeafCollector,
        accept_docs: Option<&dyn Bits>,
        min: i32,
        max: i32,
    ) -> CollectionResult<i32> {
        let lead1 = self.order[0];
        let lead2 = self.order[1];
        debug_assert!(self.doc_id(lead1) >= self.doc_id(lead2));

        if self.doc_id(lead1) < min {
            self.iterator(lead1).advance(min)?;
        }

        if self.doc_id(lead1) >= max {
            return Ok(self.doc_id(lead1));
        }

        collector.set_scorer(self)?;

        let mut competitive_iterator = collector.competitive_iterator()?;

        // In the main loop, we want to be able to rely on the invariant that
        // lead1.docID() > lead2.docID(). However it's possible that these two
        // are equal on the first document in a scoring window. So we treat this
        // case separately here.
        if self.doc_id(lead1) == self.doc_id(lead2) {
            let doc = self.doc_id(lead1);
            if accept_docs.map_or(true, |bits| bits.get(doc as usize)) {
                let next = {
                    let Self { scorers, order, .. } = &mut *self;
                    align_others(scorers, &order[2..], &mut competitive_iterator, doc)?
                };
                match next {
                    Some(next) => {
                        self.iterator(lead1).advance(next)?;
                    }
                    None => {
                        collector.collect(doc, self)?;
                        self.iterator(lead1).next_doc()?;
                    }
                }
            } else {
                self.iterator(lead1).next_doc()?;
            }
        }

        let mut doc = self.doc_id(lead1);
        'advance_head: while doc < max {
            debug_assert!(self.doc_id(lead2) < doc);

            if accept_docs.is_some_and(|bits| !bits.get(doc as usize)) {
                doc = self.iterator(lead1).next_doc()?;
                continue;
            }

            // We maintain the invariant that lead2.docID() < lead1.docID() so
            // that we don't need to check if lead2 is already on the same doc
            // as lead1 here.
            let next2 = self.iterator(lead2).advance(doc)?;
            if next2 != doc {
                doc = self.iterator(lead1).advance(next2)?;
                if doc != next2 {
                    continue;
                } else if doc >= max {
                    break;
                } else if accept_docs.is_some_and(|bits| !bits.get(doc as usize)) {
                    doc = self.iterator(lead1).next_doc()?;
                    continue;
                }
            }
            debug_assert_eq!(self.doc_id(lead2), doc);

            let next = {
                let Self { scorers, order, .. } = &mut *self;
                align_others(scorers, &order[2..], &mut competitive_iterator, doc)?
            };
            if let Some(next) = next {
                doc = self.iterator(lead1).advance(next)?;
                continue 'advance_head;
            }

            collector.collect(doc, self)?;
            doc = self.iterator(lead1).next_doc()?;
        }

        Ok(self.doc_id(lead1))
    }
}
