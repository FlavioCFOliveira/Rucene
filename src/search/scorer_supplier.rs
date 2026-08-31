//! Deferred scorer construction, ported from
//! `org.apache.lucene.search.ScorerSupplier`.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::search::bulk_scorer::{BulkScorer, DefaultBulkScorer};
use crate::search::scorer::Scorer;

/// A supplier of [`Scorer`]s, allowing the cost of a scorer to be known before
/// it is built.
///
/// Equivalent to the abstract class `org.apache.lucene.search.ScorerSupplier`.
pub trait ScorerSupplier {
    /// Builds the scorer. This must be called at most once.
    ///
    /// Equivalent to `ScorerSupplier.get(long)`, which may not return `null`.
    ///
    /// `lead_cost` is the cost of the scorer that will be used to lead
    /// iteration. It can be interpreted as an upper bound on the number of
    /// times [`DocIdSetIterator::next_doc`](crate::search::DocIdSetIterator::next_doc),
    /// [`DocIdSetIterator::advance`](crate::search::DocIdSetIterator::advance)
    /// and [`TwoPhaseIterator::matches`](crate::search::TwoPhaseIterator::matches)
    /// will be called. Under doubt, pass [`i64::MAX`], which produces a scorer
    /// with good iteration capabilities.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while building the scorer.
    fn get(&mut self, lead_cost: i64) -> Result<Box<dyn Scorer>>;

    /// Returns a scorer optimised for bulk scoring.
    ///
    /// Equivalent to `ScorerSupplier.bulkScorer()`, whose default iterates
    /// matches from the [`Scorer`] through [`DefaultBulkScorer`]. Some queries
    /// have more efficient approaches for matching all hits and override it.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while building the bulk scorer.
    fn bulk_scorer(&mut self) -> Result<Box<dyn BulkScorer>> {
        Ok(Box::new(DefaultBulkScorer::new(self.get(i64::MAX)?)))
    }

    /// Returns an estimate of the cost of the scorer that
    /// [`get`](Self::get) would return.
    ///
    /// Equivalent to `ScorerSupplier.cost()`. This may be a costly operation,
    /// so it should only be called when necessary.
    fn cost(&self) -> i64;

    /// Informs this supplier that its scorers produce scores that get passed to
    /// the collector, as opposed to partial scores that then need to be
    /// combined.
    ///
    /// Equivalent to `ScorerSupplier.setTopLevelScoringClause()`, a no-op by
    /// default. Note that this method also gets called when scores are not
    /// requested — for instance because the score mode is
    /// [`ScoreMode::COMPLETE_NO_SCORES`](crate::search::ScoreMode::COMPLETE_NO_SCORES) —
    /// so implementations should look at both the score mode and this call to
    /// know whether to prepare for reacting to
    /// [`Scorable::set_min_competitive_score`](crate::search::Scorable::set_min_competitive_score).
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reconfiguring.
    fn set_top_level_scoring_clause(&mut self) -> Result<()> {
        Ok(())
    }
}
