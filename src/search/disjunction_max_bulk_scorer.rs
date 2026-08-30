//! Disjunction-max bulk scoring, ported from
//! `org.apache.lucene.search.DisjunctionMaxBulkScorer`.

#![deny(unsafe_code)]

use crate::error::{LuceneError, Result};
use crate::search::bit_set_util;
use crate::search::bulk_scorer::BulkScorer;
use crate::search::collection_terminated_exception::CollectionResult;
use crate::search::collector::LeafCollector;
use crate::search::doc_id_set_iterator::NO_MORE_DOCS;
use crate::search::index_priority_queue::{IndexOrder, IndexPriorityQueue};
use crate::search::scorable::{Scorable, SimpleScorable};
use crate::util::{Bits, FixedBitSet, MathUtil};

/// The window this scorer computes matches over; the same size
/// [`BooleanScorer`](crate::search::BooleanScorer) uses.
///
/// Equivalent to `DisjunctionMaxBulkScorer.WINDOW_SIZE`.
const WINDOW_SIZE: i32 = 4096;

/// Reproduces `java.lang.Math.max(float, float)`.
///
/// Rust's [`f32::max`] returns the non-`NaN` operand when one of them is `NaN`,
/// where Java propagates it, and the two also disagree on `max(-0.0, +0.0)`.
/// `DisjunctionMaxBulkScorer` keeps the greatest score seen for a document with
/// `Math.max`, so the Java semantics are reproduced exactly.
fn java_math_max(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && a.to_bits() == (-0.0f32).to_bits() {
        return b;
    }
    if a >= b {
        a
    } else {
        b
    }
}

/// A sub bulk scorer paired with the doc ID it will produce matches from next.
///
/// Equivalent to the private `DisjunctionMaxBulkScorer.BulkScorerAndNext`.
struct BulkScorerAndNext {
    scorer: Box<dyn BulkScorer>,
    next: i32,
}

/// Orders [`BulkScorerAndNext`] entries by the doc ID they resume from.
///
/// Equivalent to the `lessThan` of the anonymous `PriorityQueue` in
/// `DisjunctionMaxBulkScorer`'s constructor.
struct ByNext;

impl IndexOrder<BulkScorerAndNext> for ByNext {
    fn less_than(a: &BulkScorerAndNext, b: &BulkScorerAndNext) -> bool {
        a.next < b.next
    }
}

/// The [`LeafCollector`] the sub bulk scorers feed, which records matches into
/// the window bit set and the window scores.
///
/// Equivalent to the reusable `innerCollector` field of
/// `DisjunctionMaxBulkScorer`.
struct WindowCollector<'a> {
    window_matches: &'a mut FixedBitSet,
    window_scores: &'a mut [f32],
    window_min: i32,
    min_competitive_score: f32,
}

impl LeafCollector for WindowCollector<'_> {
    fn set_scorer(&mut self, scorer: &mut dyn Scorable) -> Result<()> {
        if self.min_competitive_score != 0.0 {
            scorer.set_min_competitive_score(self.min_competitive_score)?;
        }
        Ok(())
    }

    fn collect(&mut self, doc: i32, scorer: &mut dyn Scorable) -> CollectionResult<()> {
        let delta = (doc - self.window_min) as usize;
        self.window_matches.set(delta);
        self.window_scores[delta] = java_math_max(self.window_scores[delta], scorer.score()?);
        Ok(())
    }
}

/// Bulk scorer for [`DisjunctionMaxQuery`](crate::search::DisjunctionMaxQuery)
/// when the tie-break multiplier is zero.
///
/// Equivalent to `org.apache.lucene.search.DisjunctionMaxBulkScorer`, which is
/// package-private in Java; it is public here because Rust has no package
/// visibility and the query that builds it lives in a sibling module.
///
/// **Divergence from Lucene 10.5.0.** Java's priority queue holds references to
/// the sub scorers whose `next` field it orders by, and the code mutates that
/// field while the entry sits in the heap. Rust forbids that aliasing, so the
/// entries live in an array owned by this scorer and the heap holds positions
/// into it — see [`IndexPriorityQueue`]. The heap layout, and therefore the
/// order in which equal entries come out, is Lucene's.
pub struct DisjunctionMaxBulkScorer {
    // WINDOW_SIZE + 1 to ease iteration on the bit set.
    window_matches: FixedBitSet,
    window_scores: Vec<f32>,
    entries: Vec<BulkScorerAndNext>,
    queue: IndexPriorityQueue<BulkScorerAndNext, ByNext>,
    top_level_scorable: SimpleScorable,
    cost: i64,
}

impl std::fmt::Debug for DisjunctionMaxBulkScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DisjunctionMaxBulkScorer")
            .field("scorers", &self.entries.len())
            .field("cost", &self.cost)
            .finish()
    }
}

impl DisjunctionMaxBulkScorer {
    /// Builds a disjunction-max bulk scorer over at least two sub scorers.
    ///
    /// Equivalent to `DisjunctionMaxBulkScorer(List<BulkScorer>)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when fewer than two sub scorers
    /// are supplied, which is the bare `IllegalArgumentException` Java throws.
    pub fn new(scorers: Vec<Box<dyn BulkScorer>>) -> Result<Self> {
        if scorers.len() < 2 {
            return Err(LuceneError::IllegalArgument(format!(
                "Expected 2 or more bulk scorers, got {}",
                scorers.len()
            )));
        }
        let cost = scorers
            .iter()
            .fold(0i64, |acc, scorer| acc.wrapping_add(scorer.cost()));
        let entries: Vec<BulkScorerAndNext> = scorers
            .into_iter()
            .map(|scorer| BulkScorerAndNext { scorer, next: 0 })
            .collect();
        let mut queue = IndexPriorityQueue::new(entries.len());
        queue.add_all(&entries, 0..entries.len());
        Ok(Self {
            window_matches: FixedBitSet::new((WINDOW_SIZE + 1) as usize),
            window_scores: vec![0.0; WINDOW_SIZE as usize],
            entries,
            queue,
            top_level_scorable: SimpleScorable::new(),
            cost,
        })
    }
}

/// Message used where the heap is known to be non-empty because it was filled
/// with at least two entries at construction and entries are never removed.
const NON_EMPTY_INVARIANT: &str =
    "INVARIANT: the queue is filled at construction and entries are never removed";

impl BulkScorer for DisjunctionMaxBulkScorer {
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
        let mut top = self.queue.top().expect(NON_EMPTY_INVARIANT);

        while self.entries[top].next < max {
            let window_min = self.entries[top].next.max(min);
            let window_max = MathUtil::unsigned_min(max, window_min.wrapping_add(WINDOW_SIZE));

            // First compute matches / scores in the window.
            loop {
                {
                    let Self {
                        window_matches,
                        window_scores,
                        entries,
                        top_level_scorable,
                        ..
                    } = self;
                    let mut inner_collector = WindowCollector {
                        window_matches,
                        window_scores,
                        window_min,
                        min_competitive_score: top_level_scorable.min_competitive_score(),
                    };
                    entries[top].next = entries[top].scorer.score(
                        &mut inner_collector,
                        accept_docs,
                        window_min,
                        window_max,
                    )?;
                }
                top = self
                    .queue
                    .update_top(&self.entries)
                    .expect(NON_EMPTY_INVARIANT);
                if self.entries[top].next >= window_max {
                    break;
                }
            }

            // Then replay, resetting window_scores entries inline to avoid a
            // full fill.
            collector.set_scorer(&mut self.top_level_scorable)?;
            let mut window_doc = bit_set_util::next_set_bit(&self.window_matches, 0);
            while window_doc != NO_MORE_DOCS {
                self.top_level_scorable
                    .set_score(self.window_scores[window_doc as usize]);
                self.window_scores[window_doc as usize] = 0.0;
                collector.collect(window_min + window_doc, &mut self.top_level_scorable)?;
                window_doc =
                    bit_set_util::next_set_bit(&self.window_matches, window_doc as usize + 1);
            }

            // Only the bit set needs clearing; window_scores entries were reset
            // during replay above.
            self.window_matches.clear_all();
        }

        Ok(self.entries[top].next)
    }
}
