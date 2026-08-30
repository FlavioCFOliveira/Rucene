//! Block-max conjunction bulk scoring, ported from
//! `org.apache.lucene.search.BlockMaxConjunctionBulkScorer`.

#![deny(unsafe_code)]

use crate::error::{LuceneError, Result};
use crate::index::DocAndFloatFeatureBuffer;
use crate::search::bulk_scorer::BulkScorer;
use crate::search::collection_terminated_exception::CollectionResult;
use crate::search::collector::LeafCollector;
use crate::search::doc_id_set_iterator::NO_MORE_DOCS;
use crate::search::scorable::SimpleScorable;
use crate::search::scorer::Scorer;
use crate::search::scorer_util::{DocAndScoreAccBuffer, ScorerUtil};
use crate::util::{Bits, MathUtil};

/// The largest window this scorer computes score upper bounds over.
///
/// Equivalent to `BlockMaxConjunctionBulkScorer.MAX_WINDOW_SIZE`.
const MAX_WINDOW_SIZE: i32 = 65536;

/// A [`BulkScorer`] implementation of
/// [`BlockMaxConjunctionScorer`](crate::search::BlockMaxConjunctionScorer) that
/// focuses on top-level conjunctions over clauses that have no two-phase
/// iterator.
///
/// Equivalent to `org.apache.lucene.search.BlockMaxConjunctionBulkScorer`,
/// which is package-private in Java; it is public here because Rust has no
/// package visibility and
/// [`BooleanScorerSupplier`](crate::search::BooleanScorerSupplier), which
/// builds it, lives in a sibling module.
///
/// Use a [`DefaultBulkScorer`](crate::search::DefaultBulkScorer) around a
/// [`BlockMaxConjunctionScorer`](crate::search::BlockMaxConjunctionScorer) if
/// two-phase support is needed. The other difference with that scorer is that
/// this one computes scores on the fly, so that it can skip evaluating more
/// clauses when the total score would be under the minimum competitive score
/// anyway. This generally works well because computing a score is cheaper than
/// decoding a block of postings.
///
/// **Divergence from Lucene 10.5.0.** Java keeps three parallel arrays over the
/// same objects — the scorers, the scorables they are, and the iterators they
/// expose. Rust cannot alias, so the clauses are owned once and the scorable
/// and the iterator are re-borrowed from each in turn. The order of the clauses
/// and the sequence of calls are unchanged.
pub struct BlockMaxConjunctionBulkScorer {
    /// The clauses, in ascending iterator-cost order; `[0]` is Java's `lead`.
    scorers: Vec<Box<dyn Scorer>>,
    scorable: SimpleScorable,
    sum_of_other_clauses: Vec<f64>,
    max_doc: i32,
    cost: i64,
    doc_and_score_buffer: DocAndFloatFeatureBuffer,
    doc_and_score_acc_buffer: DocAndScoreAccBuffer,
}

impl std::fmt::Debug for BlockMaxConjunctionBulkScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlockMaxConjunctionBulkScorer")
            .field("clauses", &self.scorers.len())
            .field("max_doc", &self.max_doc)
            .finish_non_exhaustive()
    }
}

/// Applies a clause as required, re-borrowing the iterator and the scorable of
/// the same scorer in turn.
///
/// Equivalent to `ScorerUtil.applyRequiredClause(DocAndScoreAccBuffer,
/// DocIdSetIterator, Scorable)`, which Java can call with two aliases of the
/// same clause.
fn apply_required_clause(buffer: &mut DocAndScoreAccBuffer, scorer: &mut dyn Scorer) -> Result<()> {
    let mut intersection_size = 0;
    let mut cur_doc = scorer.iterator().doc_id();
    for i in 0..buffer.size {
        let target_doc = buffer.docs[i];
        if cur_doc < target_doc {
            cur_doc = scorer.iterator().advance(target_doc)?;
        }
        if cur_doc == target_doc {
            buffer.docs[intersection_size] = target_doc;
            buffer.scores[intersection_size] = buffer.scores[i] + f64::from(scorer.score()?);
            intersection_size += 1;
        }
    }
    buffer.size = intersection_size;
    Ok(())
}

impl BlockMaxConjunctionBulkScorer {
    /// Builds a block-max conjunction bulk scorer.
    ///
    /// Equivalent to `BlockMaxConjunctionBulkScorer(int, List<Scorer>)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when fewer than two clauses are
    /// supplied, which is the `IllegalArgumentException` Java throws.
    pub fn new(max_doc: i32, scorers: Vec<Box<dyn Scorer>>) -> Result<Self> {
        if scorers.len() <= 1 {
            return Err(LuceneError::IllegalArgument(format!(
                "Expected 2 or more scorers, got {}",
                scorers.len()
            )));
        }
        let mut scorers = scorers;
        let mut costs = Vec::with_capacity(scorers.len());
        for scorer in &mut scorers {
            costs.push(scorer.iterator().cost());
        }
        // `Arrays.sort` on objects is stable, and so is `sort_by_cached_key`
        // over the costs read once above.
        let mut order: Vec<usize> = (0..scorers.len()).collect();
        order.sort_by(|a, b| costs[*a].cmp(&costs[*b]));
        let mut sorted: Vec<Option<Box<dyn Scorer>>> = scorers.into_iter().map(Some).collect();
        let scorers: Vec<Box<dyn Scorer>> = order
            .iter()
            .map(|position| {
                sorted[*position]
                    .take()
                    .expect("INVARIANT: a sort permutation visits every position exactly once")
            })
            .collect();

        let cost = costs[order[0]];
        let sum_of_other_clauses = vec![f64::INFINITY; scorers.len()];
        Ok(Self {
            scorers,
            scorable: SimpleScorable::new(),
            sum_of_other_clauses,
            max_doc,
            cost,
            doc_and_score_buffer: DocAndFloatFeatureBuffer::new(),
            doc_and_score_acc_buffer: DocAndScoreAccBuffer::new(),
        })
    }

    /// Equivalent to the private
    /// `BlockMaxConjunctionBulkScorer.computeMaxScore(int, int)`.
    fn compute_max_score(&mut self, window_min: i32, window_max: i32) -> Result<f32> {
        for i in 0..self.scorers.len() {
            self.scorers[i].advance_shallow(window_min)?;
        }

        let mut max_window_score = 0.0f64;
        for i in 0..self.scorers.len() {
            let max_clause_score = self.scorers[i].get_max_score(window_max)?;
            self.sum_of_other_clauses[i] = f64::from(max_clause_score);
            max_window_score += f64::from(max_clause_score);
        }
        for i in (0..self.sum_of_other_clauses.len().saturating_sub(1)).rev() {
            self.sum_of_other_clauses[i] += self.sum_of_other_clauses[i + 1];
        }
        Ok(max_window_score as f32)
    }

    /// Scores a window of doc IDs by first finding agreement between all
    /// iterators and only then computing scores and calling the collector,
    /// until dynamic pruning kicks in.
    ///
    /// Equivalent to the private
    /// `BlockMaxConjunctionBulkScorer.scoreDocFirstUntilDynamicPruning`.
    fn score_doc_first_until_dynamic_pruning(
        &mut self,
        collector: &mut dyn LeafCollector,
        accept_docs: Option<&dyn Bits>,
        min: i32,
        max: i32,
    ) -> CollectionResult<i32> {
        let mut doc = self.scorers[0].doc_id();
        if doc < min {
            doc = self.scorers[0].iterator().advance(min)?;
        }

        'outer: while doc < max {
            if accept_docs.map_or(true, |bits| bits.get(doc as usize)) {
                for i in 1..self.scorers.len() {
                    let iterator = self.scorers[i].iterator();
                    let mut other_doc = iterator.doc_id();
                    if other_doc < doc {
                        other_doc = iterator.advance(doc)?;
                    }
                    if doc != other_doc {
                        doc = self.scorers[0].iterator().advance(other_doc)?;
                        continue 'outer;
                    }
                }

                let mut score = 0.0f64;
                for i in 0..self.scorers.len() {
                    score += f64::from(self.scorers[i].score()?);
                }
                self.scorable.set_score(score as f32);
                collector.collect(doc, &mut self.scorable)?;
                if self.scorable.min_competitive_score() > 0.0 {
                    return Ok(self.scorers[0].iterator().next_doc()?);
                }
            }
            doc = self.scorers[0].iterator().next_doc()?;
        }
        Ok(doc)
    }

    /// Scores a window of doc IDs by computing matches and scores on the lead
    /// costly clause, then iterating the other clauses one by one to remove
    /// documents that do not match and increase the global score by the score
    /// of the current clause.
    ///
    /// Equivalent to the private
    /// `BlockMaxConjunctionBulkScorer.scoreWindowScoreFirst`. This is often
    /// faster when a minimum competitive score is set, as score computations
    /// can be more efficient and because advancing other clauses can be skipped
    /// if the global score so far is not high enough for a document to have a
    /// chance of being competitive.
    fn score_window_score_first(
        &mut self,
        collector: &mut dyn LeafCollector,
        accept_docs: Option<&dyn Bits>,
        min: i32,
        max: i32,
        max_window_score: f32,
    ) -> CollectionResult<()> {
        if max_window_score < self.scorable.min_competitive_score() {
            // no hits are competitive
            return Ok(());
        }

        if self.scorers[0].doc_id() < min {
            self.scorers[0].iterator().advance(min)?;
        }
        if self.scorers[0].doc_id() >= max {
            return Ok(());
        }

        let num_scorers = self.scorers.len() as i32;
        loop {
            {
                let Self {
                    scorers,
                    doc_and_score_buffer,
                    ..
                } = self;
                scorers[0].next_docs_and_scores(max, accept_docs, doc_and_score_buffer)?;
            }
            if self.doc_and_score_buffer.size == 0 {
                break;
            }

            self.doc_and_score_acc_buffer
                .copy_from(&self.doc_and_score_buffer);

            for i in 1..self.scorers.len() {
                let sum_of_other_clause = self.sum_of_other_clauses[i];
                if sum_of_other_clause != self.sum_of_other_clauses[i - 1] {
                    // two equal consecutive values mean that the first clause
                    // always returns a score of zero, so we don't need to filter
                    // hits by score again.
                    ScorerUtil::filter_competitive_hits(
                        &mut self.doc_and_score_acc_buffer,
                        sum_of_other_clause,
                        self.scorable.min_competitive_score(),
                        num_scorers,
                    );
                }

                let Self {
                    scorers,
                    doc_and_score_acc_buffer,
                    ..
                } = self;
                apply_required_clause(doc_and_score_acc_buffer, &mut *scorers[i])?;
            }

            for i in 0..self.doc_and_score_acc_buffer.size {
                self.scorable
                    .set_score(self.doc_and_score_acc_buffer.scores[i] as f32);
                let doc = self.doc_and_score_acc_buffer.docs[i];
                collector.collect(doc, &mut self.scorable)?;
            }
        }

        let mut max_other_doc = -1;
        for i in 1..self.scorers.len() {
            max_other_doc = max_other_doc.max(self.scorers[i].doc_id());
        }
        if self.scorers[0].doc_id() < max_other_doc {
            self.scorers[0].iterator().advance(max_other_doc)?;
        }
        Ok(())
    }
}

impl BulkScorer for BlockMaxConjunctionBulkScorer {
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
        collector.set_scorer(&mut self.scorable)?;

        let mut window_min = self.scorers[0].doc_id().max(min);
        if self.scorable.min_competitive_score() == 0.0 {
            window_min =
                self.score_doc_first_until_dynamic_pruning(collector, accept_docs, min, max)?;
        }

        while window_min < max {
            // Use impacts of the least costly scorer to compute windows.
            // NOTE: window_max is inclusive.
            let mut window_max = self.scorers[0].advance_shallow(window_min)?.min(max - 1);
            // Ensure the scoring window is not too big; this especially works
            // for the default implementation of `Scorer::advance_shallow`, which
            // may return NO_MORE_DOCS.
            window_max =
                MathUtil::unsigned_min(window_max, window_min.wrapping_add(MAX_WINDOW_SIZE));

            let max_window_score = self.compute_max_score(window_min, window_max)?;
            self.score_window_score_first(
                collector,
                accept_docs,
                window_min,
                window_max + 1,
                max_window_score,
            )?;
            window_min = self.scorers[0].doc_id().max(window_max + 1);
        }

        Ok(if window_min >= self.max_doc {
            NO_MORE_DOCS
        } else {
            window_min
        })
    }
}
