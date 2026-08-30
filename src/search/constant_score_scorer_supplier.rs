//! Constant-score supplier, ported from
//! `org.apache.lucene.search.ConstantScoreScorerSupplier`.

#![deny(unsafe_code)]

use std::fmt::Debug;

use crate::error::{LuceneError, Result};
use crate::search::bulk_scorer::{BulkScorer, DefaultBulkScorer};
use crate::search::constant_score_scorer::ConstantScoreScorer;
use crate::search::doc_id_set_iterator::{self, DocIdSetIterator};
use crate::search::score_mode::ScoreMode;
use crate::search::scorer::Scorer;
use crate::search::scorer_supplier::ScorerSupplier;
use crate::search::two_phase_iterator::ScorerIterator;

/// The iteration a [`ConstantScoreScorerSupplier`] is built from.
///
/// Equivalent to what a Java subclass of
/// `org.apache.lucene.search.ConstantScoreScorerSupplier` overrides: the
/// abstract `iterator(long)` and the abstract `cost()` inherited from
/// `ScorerSupplier`.
pub trait ConstantScoreIteratorSupplier: Debug {
    /// Returns the iterator given the cost of the leading clause.
    ///
    /// Equivalent to `ConstantScoreScorerSupplier.iterator(long)`. The return
    /// type keeps the plain and the two-phase shapes apart, because Rust cannot
    /// recover a two-phase iterator from an erased one; see [`ScorerIterator`].
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while building the iterator.
    fn iterator(&mut self, lead_cost: i64) -> Result<ScorerIterator>;

    /// Returns an estimate of the cost of the iterator.
    ///
    /// Equivalent to `ScorerSupplier.cost()`.
    fn cost(&self) -> i64;
}

/// Specialisation of [`ScorerSupplier`] for queries that produce constant
/// scores.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.search.ConstantScoreScorerSupplier`. Supply the iteration
/// as a [`ConstantScoreIteratorSupplier`] and wrap it here.
///
/// **Divergence from Lucene 10.5.0.** Java's `bulkScorer()` picks between
/// `DenseConjunctionBulkScorer`, `ConstantScoreBulkScorer` and
/// `Weight.DefaultBulkScorer` depending on the density of the iterator and on
/// whether scores are needed. The first two are bulk-scoring specialisations
/// that are not part of the query-execution spine and are not ported yet, so
/// this port always uses [`DefaultBulkScorer`]. The hits collected and their
/// scores are identical; only the throughput on very dense iterators differs.
#[derive(Debug)]
pub struct ConstantScoreScorerSupplier<I: ConstantScoreIteratorSupplier> {
    score_mode: ScoreMode,
    score: f32,
    max_doc: i32,
    inner: I,
}

impl<I: ConstantScoreIteratorSupplier> ConstantScoreScorerSupplier<I> {
    /// Wraps the given iteration.
    ///
    /// Equivalent to the protected
    /// `ConstantScoreScorerSupplier(float, ScoreMode, int)` constructor,
    /// invoked by sub-classes.
    pub fn new(score: f32, score_mode: ScoreMode, max_doc: i32, inner: I) -> Self {
        Self {
            score_mode,
            score,
            max_doc,
            inner,
        }
    }

    /// Returns the number of documents in the segment this supplier scores.
    ///
    /// Equivalent to reading the private `maxDoc` field, which Java's
    /// `bulkScorer()` uses to pick a bulk-scoring strategy.
    pub fn max_doc(&self) -> i32 {
        self.max_doc
    }
}

/// A [`ConstantScoreIteratorSupplier`] that hands out one already-built
/// iterator.
///
/// Equivalent to the anonymous subclass that
/// `ConstantScoreScorerSupplier.fromIterator(DocIdSetIterator, float,
/// ScoreMode, int)` returns.
///
/// **Divergence from Lucene 10.5.0.** Java's lambda captures the iterator and
/// returns the same reference on every call. Rust cannot hand out an owned
/// iterator twice, so a second call fails with
/// [`LuceneError::IllegalState`] — turning `ScorerSupplier`'s documented
/// "must be called at most once" contract into a reported one. The cost is
/// captured at construction for the same reason.
#[derive(Debug)]
pub struct SingleIteratorSupplier {
    iterator: Option<ScorerIterator>,
    cost: i64,
}

impl SingleIteratorSupplier {
    /// Captures the given iterator and its cost.
    fn new(iterator: ScorerIterator) -> Self {
        let cost = iterator.cost();
        Self {
            iterator: Some(iterator),
            cost,
        }
    }
}

impl ConstantScoreIteratorSupplier for SingleIteratorSupplier {
    fn iterator(&mut self, _lead_cost: i64) -> Result<ScorerIterator> {
        self.iterator.take().ok_or_else(|| {
            LuceneError::IllegalState(
                "ScorerSupplier.get(long) must be called at most once".to_string(),
            )
        })
    }

    fn cost(&self) -> i64 {
        self.cost
    }
}

impl ConstantScoreScorerSupplier<SingleIteratorSupplier> {
    /// Creates a supplier that matches all docs in `[0, max_doc)`.
    ///
    /// Equivalent to
    /// `ConstantScoreScorerSupplier.matchAll(float, ScoreMode, int)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `max_doc` is negative,
    /// which is what `DocIdSetIterator.all(int)` rejects.
    pub fn match_all(score: f32, score_mode: ScoreMode, max_doc: i32) -> Result<Self> {
        let iterator: Box<dyn DocIdSetIterator> = Box::new(doc_id_set_iterator::all(max_doc)?);
        Ok(Self::from_iterator(
            ScorerIterator::Simple(iterator),
            score,
            score_mode,
            max_doc,
        ))
    }

    /// Creates a supplier for the given iterator.
    ///
    /// Equivalent to
    /// `ConstantScoreScorerSupplier.fromIterator(DocIdSetIterator, float,
    /// ScoreMode, int)`.
    pub fn from_iterator(
        iterator: ScorerIterator,
        score: f32,
        score_mode: ScoreMode,
        max_doc: i32,
    ) -> Self {
        Self::new(
            score,
            score_mode,
            max_doc,
            SingleIteratorSupplier::new(iterator),
        )
    }
}

impl<I: ConstantScoreIteratorSupplier> ScorerSupplier for ConstantScoreScorerSupplier<I> {
    fn cost(&self) -> i64 {
        self.inner.cost()
    }

    fn get(&mut self, lead_cost: i64) -> Result<Box<dyn Scorer>> {
        match self.inner.iterator(lead_cost)? {
            ScorerIterator::Simple(iterator) => Ok(Box::new(ConstantScoreScorer::from_iterator(
                self.score,
                self.score_mode,
                iterator,
            ))),
            ScorerIterator::TwoPhase(two_phase) => Ok(Box::new(
                ConstantScoreScorer::from_two_phase(self.score, self.score_mode, two_phase),
            )),
        }
    }

    fn bulk_scorer(&mut self) -> Result<Box<dyn BulkScorer>> {
        Ok(Box::new(DefaultBulkScorer::new(self.get(i64::MAX)?)))
    }
}
