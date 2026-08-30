//! Bulk scoring of constant-score iterators, ported from
//! `org.apache.lucene.search.ConstantScoreBulkScorer`.

#![deny(unsafe_code)]

use crate::error::{LuceneError, Result};
use crate::search::bit_set_doc_id_stream::BitSetDocIdStream;
use crate::search::bit_set_util;
use crate::search::bulk_scorer::BulkScorer;
use crate::search::collection_terminated_exception::CollectionResult;
use crate::search::collector::LeafCollector;
use crate::search::constant_score_scorer::ConstantScoreScorer;
use crate::search::dense_conjunction_bulk_scorer::WINDOW_SIZE;
use crate::search::doc_id_set_iterator::DocIdSetIterator;
use crate::search::score_mode::ScoreMode;
use crate::search::scorer::Scorer;
use crate::search::two_phase_iterator::TwoPhaseIterator;
use crate::util::{Bits, FixedBitSet, MathUtil};

/// Message used where the two-phase view is known to be present because it was
/// observed once at construction and a scorer returns a stable view.
const TWO_PHASE_INVARIANT: &str =
    "INVARIANT: two_phase was observed at construction and a Scorer returns a stable view";

/// Bulk scorer for no-score constant-score iterators that batches doc IDs
/// through [`DocIdSetIterator::into_bit_set`].
///
/// Equivalent to `org.apache.lucene.search.ConstantScoreBulkScorer`, which is
/// package-private in Java; it is public here because Rust has no package
/// visibility and
/// [`ConstantScoreScorerSupplier`](crate::search::ConstantScoreScorerSupplier),
/// which builds it, lives in a sibling module.
///
/// **Divergence from Lucene 10.5.0.** Java keeps three aliases of the same
/// iteration — the `ConstantScoreScorer` it hands to the collector, the raw
/// approximation it drives, and the optional `TwoPhaseIterator` that confirms
/// matches. Rust cannot alias an owned iterator, so only the scorer is stored
/// and the other two are re-borrowed from it on every step, exactly as
/// [`DefaultBulkScorer`](crate::search::DefaultBulkScorer) does. The sequence of
/// calls to the iterator, to the confirmation and to the collector is unchanged.
pub struct ConstantScoreBulkScorer {
    scorer: ConstantScoreScorer,
    /// Whether matches of the approximation must be confirmed through
    /// [`TwoPhaseIterator::matches`] rather than taken as-is.
    two_phase: bool,
    cost: i64,
    window_matches: FixedBitSet,
}

impl std::fmt::Debug for ConstantScoreBulkScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConstantScoreBulkScorer")
            .field("two_phase", &self.two_phase)
            .field("cost", &self.cost)
            .finish_non_exhaustive()
    }
}

/// Returns the approximation of the given scorer: the two-phase view's when it
/// has one, its own iterator otherwise.
fn approximation(scorer: &mut ConstantScoreScorer, two_phase: bool) -> &mut dyn DocIdSetIterator {
    if two_phase {
        scorer
            .two_phase_iterator()
            .expect(TWO_PHASE_INVARIANT)
            .approximation()
    } else {
        scorer.iterator()
    }
}

/// Loads the matches below `up_to` into `bit_set`, confirming them when the
/// scorer is two-phase.
fn into_bit_set(
    scorer: &mut ConstantScoreScorer,
    two_phase: bool,
    up_to: i32,
    bit_set: &mut FixedBitSet,
    offset: i32,
) -> Result<()> {
    if two_phase {
        scorer
            .two_phase_iterator()
            .expect(TWO_PHASE_INVARIANT)
            .into_bit_set(up_to, bit_set, offset)
    } else {
        scorer.iterator().into_bit_set(up_to, bit_set, offset)
    }
}

impl ConstantScoreBulkScorer {
    /// Builds a bulk scorer over a plain iterator.
    ///
    /// Equivalent to
    /// `ConstantScoreBulkScorer(float, ScoreMode, DocIdSetIterator)`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java also rejects an iterator that is
    /// a disguised `TwoPhaseIterator`, which it detects with
    /// `TwoPhaseIterator.unwrap`. This port keeps the two shapes apart by type —
    /// [`from_two_phase`](Self::from_two_phase) is the two-phase constructor —
    /// so the check has nothing left to catch.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `score_mode` needs scores,
    /// which is the `IllegalArgumentException` Java throws.
    pub fn from_iterator(
        score: f32,
        score_mode: ScoreMode,
        iterator: Box<dyn DocIdSetIterator>,
    ) -> Result<Self> {
        Self::check_score_mode(score_mode)?;
        let cost = iterator.cost();
        Ok(Self {
            scorer: ConstantScoreScorer::from_iterator(score, score_mode, iterator),
            two_phase: false,
            cost,
            window_matches: FixedBitSet::new(WINDOW_SIZE as usize),
        })
    }

    /// Builds a bulk scorer that confirms the matches of a two-phase iterator.
    ///
    /// Equivalent to
    /// `ConstantScoreBulkScorer(float, ScoreMode, TwoPhaseIterator)`. It is
    /// worthwhile when the two-phase iterator overrides
    /// [`TwoPhaseIterator::into_bit_set`] with a bulk implementation, so that a
    /// single clause can be collected window by window instead of confirmed one
    /// document at a time.
    ///
    /// # Errors
    ///
    /// As [`from_iterator`](Self::from_iterator).
    pub fn from_two_phase(
        score: f32,
        score_mode: ScoreMode,
        two_phase: Box<dyn TwoPhaseIterator>,
    ) -> Result<Self> {
        Self::check_score_mode(score_mode)?;
        let cost = two_phase.approximation_ref().cost();
        Ok(Self {
            scorer: ConstantScoreScorer::from_two_phase(score, score_mode, two_phase),
            two_phase: true,
            cost,
            window_matches: FixedBitSet::new(WINDOW_SIZE as usize),
        })
    }

    fn check_score_mode(score_mode: ScoreMode) -> Result<()> {
        if score_mode.needs_scores() {
            return Err(LuceneError::IllegalArgument(format!(
                "ScoreMode must not need scores: {score_mode:?}"
            )));
        }
        Ok(())
    }

    /// Equivalent to the private static
    /// `ConstantScoreBulkScorer.scoreIterator`.
    fn score_iterator(
        &mut self,
        collector: &mut dyn LeafCollector,
        accept_docs: Option<&dyn Bits>,
        competitive_iterator: &mut dyn DocIdSetIterator,
        min: i32,
        max: i32,
    ) -> CollectionResult<()> {
        let mut min = min;
        if competitive_iterator.doc_id() > min {
            min = competitive_iterator.doc_id().min(max);
        }
        let two_phase = self.two_phase;
        if approximation(&mut self.scorer, two_phase).doc_id() < min {
            if approximation(&mut self.scorer, two_phase).doc_id() == min - 1 {
                approximation(&mut self.scorer, two_phase).next_doc()?;
            } else {
                approximation(&mut self.scorer, two_phase).advance(min)?;
            }
        }
        let mut doc = approximation(&mut self.scorer, two_phase).doc_id();
        while doc < max {
            debug_assert!(competitive_iterator.doc_id() <= doc); // invariant
            if competitive_iterator.doc_id() < doc {
                let competitive_next = competitive_iterator.advance(doc)?;
                if competitive_next != doc {
                    doc = approximation(&mut self.scorer, two_phase).advance(competitive_next)?;
                    continue;
                }
            }

            let accepted = accept_docs.map_or(true, |bits| bits.get(doc as usize));
            let confirmed = if !accepted {
                false
            } else if two_phase {
                self.scorer
                    .two_phase_iterator()
                    .expect(TWO_PHASE_INVARIANT)
                    .matches()?
            } else {
                true
            };
            if confirmed {
                collector.collect(doc, self.scorer.as_scorable())?;
            }

            doc = approximation(&mut self.scorer, two_phase).next_doc()?;
        }
        Ok(())
    }

    /// Equivalent to the private
    /// `ConstantScoreBulkScorer.scoreIteratorIntoBitSet`.
    fn score_iterator_into_bit_set(
        &mut self,
        collector: &mut dyn LeafCollector,
        accept_docs: Option<&dyn Bits>,
        min: i32,
        max: i32,
    ) -> CollectionResult<()> {
        let two_phase = self.two_phase;
        if approximation(&mut self.scorer, two_phase).doc_id() < min {
            if approximation(&mut self.scorer, two_phase).doc_id() == min - 1 {
                approximation(&mut self.scorer, two_phase).next_doc()?;
            } else {
                approximation(&mut self.scorer, two_phase).advance(min)?;
            }
        }
        let mut doc = approximation(&mut self.scorer, two_phase).doc_id();
        while doc < max {
            let window_base = doc;
            let window_max = MathUtil::unsigned_min(max, window_base.wrapping_add(WINDOW_SIZE));

            let Self {
                scorer,
                window_matches,
                ..
            } = self;
            debug_assert!(bit_set_util::scan_is_empty(window_matches));
            into_bit_set(scorer, two_phase, window_max, window_matches, window_base)?;

            if let Some(accept_docs) = accept_docs {
                bit_set_util::apply_mask(accept_docs, window_matches, window_base);
            }

            {
                let mut stream = BitSetDocIdStream::new(window_matches, window_base);
                collector.collect_stream(&mut stream, scorer.as_scorable())?;
            }
            window_matches.clear_all();

            doc = approximation(&mut self.scorer, two_phase).doc_id();
        }
        Ok(())
    }
}

impl BulkScorer for ConstantScoreBulkScorer {
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
        collector.set_scorer(self.scorer.as_scorable())?;
        let competitive_iterator = collector.competitive_iterator()?;
        match competitive_iterator {
            Some(mut competitive_iterator) => {
                self.score_iterator(collector, accept_docs, &mut *competitive_iterator, min, max)?
            }
            None => self.score_iterator_into_bit_set(collector, accept_docs, min, max)?,
        }
        let two_phase = self.two_phase;
        Ok(approximation(&mut self.scorer, two_phase).doc_id())
    }
}
