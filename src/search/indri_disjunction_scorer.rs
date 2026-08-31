//! The Indri disjunction scorer, ported from
//! `org.apache.lucene.search.IndriDisjunctionScorer`.

#![deny(unsafe_code)]

use std::fmt::Debug;

use crate::error::Result;
use crate::search::disi_wrapper::DisiWrapper;
use crate::search::disjunction_disi_approximation::DisjunctionDISIApproximation;
use crate::search::doc_id_set_iterator::DocIdSetIterator;
use crate::search::indri_scorer::IndriScorer;
use crate::search::scorable::Scorable;
use crate::search::score_mode::ScoreMode;
use crate::search::scorer::Scorer;

/// The scoring behaviour an [`IndriDisjunctionScorer`] delegates to.
///
/// Equivalent to the two abstract methods of
/// `org.apache.lucene.search.IndriDisjunctionScorer`,
/// `score(List<Scorer>)` and `smoothingScore(List<Scorer>, int)`. Rust has no
/// implementation inheritance, so they become this trait; the sub-scorer list
/// is reached through the disjunction approximation, which owns it.
pub trait IndriDisjunctionScoring: Debug + Send {
    /// Scores the current document from every sub-scorer.
    ///
    /// Equivalent to `IndriDisjunctionScorer.score(List<Scorer>)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while scoring a sub-scorer.
    fn score(&self, sub_scorers: &mut DisjunctionDISIApproximation, doc_id: i32) -> Result<f32>;

    /// Computes the smoothing score of `doc_id` from every sub-scorer.
    ///
    /// Equivalent to
    /// `IndriDisjunctionScorer.smoothingScore(List<Scorer>, int)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while scoring a sub-scorer.
    fn smoothing_score(
        &self,
        sub_scorers: &mut DisjunctionDISIApproximation,
        doc_id: i32,
    ) -> Result<f32>;
}

/// An Indri disjunction scorer, which stores the sub-scorers of the child
/// queries.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.search.IndriDisjunctionScorer`. The `score` and
/// `smoothingScore` methods use the list of *all* sub-scorers, not only the
/// matching ones, so that a smoothing score can be computed when there is no
/// exact match.
///
/// **Divergence from Lucene 10.5.0.** Java keeps the sub-scorers twice — in
/// `subScorersList` and inside the `DisiWrapper`s of the approximation — and
/// both views point at the same objects. Rust cannot alias them, so this port
/// keeps only the approximation, which owns the wrappers, and
/// [`get_sub_matches`](Self::get_sub_matches) reaches the scorers through it in
/// the order they were supplied. The list contents and their order are
/// unchanged.
#[derive(Debug)]
pub struct IndriDisjunctionScorer<I: IndriDisjunctionScoring> {
    approximation: DisjunctionDISIApproximation,
    boost: f32,
    inner: I,
}

impl<I: IndriDisjunctionScoring> IndriDisjunctionScorer<I> {
    /// Wraps the sub-scorers of the child queries.
    ///
    /// Equivalent to the protected
    /// `IndriDisjunctionScorer(List<Scorer>, ScoreMode, float)`, whose score
    /// mode is unused — as it is here — and which builds a
    /// `DisjunctionDISIApproximation` with a lead cost of [`i64::MAX`].
    pub fn new(
        sub_scorers_list: Vec<Box<dyn Scorer>>,
        _score_mode: ScoreMode,
        boost: f32,
        inner: I,
    ) -> Self {
        let wrappers: Vec<DisiWrapper> = sub_scorers_list
            .into_iter()
            .map(|scorer| DisiWrapper::new(scorer, false))
            .collect();
        Self {
            approximation: DisjunctionDISIApproximation::new(wrappers, i64::MAX),
            boost,
            inner,
        }
    }

    /// Returns the sub-scorers, in the order they were supplied.
    ///
    /// Equivalent to `IndriDisjunctionScorer.getSubMatches()`, which returns
    /// the whole list rather than only the matches.
    pub fn get_sub_matches(&mut self) -> &mut DisjunctionDISIApproximation {
        &mut self.approximation
    }
}

impl<I: IndriDisjunctionScoring> Scorable for IndriDisjunctionScorer<I> {
    fn score(&mut self) -> Result<f32> {
        let doc_id = self.approximation.doc_id();
        let Self {
            approximation,
            inner,
            ..
        } = self;
        inner.score(approximation, doc_id)
    }

    fn smoothing_score(&mut self, doc_id: i32) -> Result<f32> {
        let Self {
            approximation,
            inner,
            ..
        } = self;
        inner.smoothing_score(approximation, doc_id)
    }
}

impl<I: IndriDisjunctionScoring> Scorer for IndriDisjunctionScorer<I> {
    fn as_scorable(&mut self) -> &mut dyn Scorable {
        self
    }

    fn as_indri_scorer(&mut self) -> Option<&mut dyn IndriScorer> {
        Some(self)
    }

    fn doc_id(&self) -> i32 {
        self.approximation.doc_id()
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        &mut self.approximation
    }

    fn get_max_score(&mut self, _up_to: i32) -> Result<f32> {
        Ok(0.0)
    }
}

impl<I: IndriDisjunctionScoring> IndriScorer for IndriDisjunctionScorer<I> {
    fn get_boost(&self) -> f32 {
        self.boost
    }
}
