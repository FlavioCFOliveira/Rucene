//! The Indri conjunction scorer, ported from
//! `org.apache.lucene.search.IndriAndScorer`.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::search::disjunction_disi_approximation::DisjunctionDISIApproximation;
use crate::search::indri_disjunction_scorer::{IndriDisjunctionScorer, IndriDisjunctionScoring};
use crate::search::score_mode::ScoreMode;
use crate::search::scorer::Scorer;

/// Combines the scores of the sub-scorers. When a sub-scorer does not contain
/// the doc ID, a smoothing score is computed for that document/sub-scorer
/// combination.
///
/// Equivalent to `org.apache.lucene.search.IndriAndScorer`, which extends
/// `IndriDisjunctionScorer` and only supplies the two scoring methods; this
/// port therefore expresses it as that scorer parameterised by
/// [`IndriAndScoring`].
pub type IndriAndScorer = IndriDisjunctionScorer<IndriAndScoring>;

/// The scoring of an [`IndriAndScorer`].
///
/// Equivalent to `IndriAndScorer.score(List<Scorer>)`,
/// `IndriAndScorer.smoothingScore(List<Scorer>, int)` and the private
/// `scoreDoc` they both delegate to.
#[derive(Debug, Default, Clone, Copy)]
pub struct IndriAndScoring;

impl IndriAndScoring {
    /// Equivalent to the private `IndriAndScorer.scoreDoc(List<Scorer>, int)`.
    fn score_doc(
        &self,
        sub_scorers: &mut DisjunctionDISIApproximation,
        doc_id: i32,
    ) -> Result<f32> {
        let mut score = 0f64;
        let mut boost_sum = 0f64;
        for i in 0..sub_scorers.len() {
            let scorer = sub_scorers.sub_scorer(i).scorer();
            let Some(indri_scorer) = scorer.as_indri_scorer() else {
                continue;
            };
            let scorer_doc_id = indri_scorer.doc_id();
            let boost = indri_scorer.get_boost();
            // If the query exists in the document, score the document.
            // Otherwise compute a smoothing score, which acts like an idf for
            // sub-queries and terms.
            let mut temp_score = if doc_id == scorer_doc_id {
                indri_scorer.score()? as f64
            } else {
                indri_scorer.smoothing_score(doc_id)? as f64
            };
            temp_score *= boost as f64;
            score += temp_score;
            boost_sum += boost as f64;
        }
        if boost_sum == 0.0 {
            Ok(0.0)
        } else {
            Ok((score / boost_sum) as f32)
        }
    }
}

impl IndriDisjunctionScoring for IndriAndScoring {
    fn score(&self, sub_scorers: &mut DisjunctionDISIApproximation, doc_id: i32) -> Result<f32> {
        self.score_doc(sub_scorers, doc_id)
    }

    fn smoothing_score(
        &self,
        sub_scorers: &mut DisjunctionDISIApproximation,
        doc_id: i32,
    ) -> Result<f32> {
        self.score_doc(sub_scorers, doc_id)
    }
}

/// Creates an [`IndriAndScorer`] over the sub-scorers of the child queries.
///
/// Equivalent to the protected
/// `IndriAndScorer(List<Scorer>, ScoreMode, float)` constructor.
pub fn new_indri_and_scorer(
    sub_scorers: Vec<Box<dyn Scorer>>,
    score_mode: ScoreMode,
    boost: f32,
) -> IndriAndScorer {
    IndriDisjunctionScorer::new(sub_scorers, score_mode, boost, IndriAndScoring)
}
