//! Scoring phrase matches, ported from
//! `org.apache.lucene.search.PhraseScorer`.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::index::NumericDocValues;
use crate::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use crate::search::phrase_matcher::PhraseMatcher;
use crate::search::scorable::Scorable;
use crate::search::score_mode::ScoreMode;
use crate::search::scorer::Scorer;
use crate::search::sim_scorer_source::SharedSimScorer;
use crate::search::two_phase_iterator::TwoPhaseIterator;
use crate::util::FixedBitSet;

/// A [`Scorer`] over the matches of a [`PhraseMatcher`].
///
/// Equivalent to the package-private
/// `org.apache.lucene.search.PhraseScorer`; it is public here because Rust has
/// no package visibility and [`PhraseWeight`](crate::search::PhraseWeight)
/// lives in a sibling module.
pub struct PhraseScorer {
    /// The matcher, which owns the approximation, the impacts-aware
    /// approximation and the score cache Java keeps as three aliasing fields
    /// beside it.
    matcher: Box<dyn PhraseMatcher>,
    score_mode: ScoreMode,
    sim_scorer: SharedSimScorer,
    norms: Option<Box<dyn NumericDocValues>>,
    match_cost: f32,
    min_competitive_score: f32,
    freq: f32,
}

impl std::fmt::Debug for PhraseScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PhraseScorer")
            .field("score_mode", &self.score_mode)
            .field("match_cost", &self.match_cost)
            .finish_non_exhaustive()
    }
}

impl PhraseScorer {
    /// Creates a scorer over the given matcher.
    ///
    /// Equivalent to
    /// `PhraseScorer(PhraseMatcher, ScoreMode, SimScorer, NumericDocValues)`.
    pub fn new(
        matcher: Box<dyn PhraseMatcher>,
        score_mode: ScoreMode,
        sim_scorer: SharedSimScorer,
        norms: Option<Box<dyn NumericDocValues>>,
    ) -> Self {
        let match_cost = matcher.get_match_cost();
        Self {
            matcher,
            score_mode,
            sim_scorer,
            norms,
            match_cost,
            min_competitive_score: 0.0,
            freq: 0.0,
        }
    }

    /// Reads the norm of `doc`, or `1` when the field has no norms.
    fn norm(&mut self, doc: i32) -> Result<i64> {
        if let Some(norms) = self.norms.as_mut() {
            if norms.advance_exact(doc)? {
                return norms.long_value();
            }
        }
        Ok(1)
    }

    /// Confirms the candidates of the approximation, which is what
    /// `TwoPhaseIterator.asDocIdSetIterator(twoPhaseIterator())` does for
    /// Java's `iterator()`.
    fn confirm(&mut self, mut doc: i32) -> Result<i32> {
        loop {
            if doc == NO_MORE_DOCS {
                return Ok(NO_MORE_DOCS);
            }
            if TwoPhaseIterator::matches(self)? {
                return Ok(doc);
            }
            doc = self.matcher.approximation().next_doc()?;
        }
    }
}

impl DocIdSetIterator for PhraseScorer {
    fn doc_id(&self) -> i32 {
        self.matcher.approximation_ref().doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        let doc = self.matcher.approximation().next_doc()?;
        self.confirm(doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let doc = self.matcher.approximation().advance(target)?;
        self.confirm(doc)
    }

    fn cost(&self) -> i64 {
        self.matcher.approximation_ref().cost()
    }
}

impl TwoPhaseIterator for PhraseScorer {
    fn approximation(&mut self) -> &mut dyn DocIdSetIterator {
        self.matcher.approximation()
    }

    fn approximation_ref(&self) -> &dyn DocIdSetIterator {
        self.matcher.approximation_ref()
    }

    fn matches(&mut self) -> Result<bool> {
        if self.score_mode == ScoreMode::TOP_SCORES && self.min_competitive_score > 0.0 {
            let max_freq = self.matcher.max_freq()?;
            let doc = self.matcher.approximation_ref().doc_id();
            let norm = self.norm(doc)?;
            if self.sim_scorer.score(max_freq, norm) < self.min_competitive_score {
                // The maximum score we could get is less than the minimum
                // competitive score.
                return Ok(false);
            }
        }
        self.matcher.reset_positions()?;
        self.freq = 0.0;
        self.matcher.next_match()
    }

    fn match_cost(&self) -> f32 {
        self.match_cost
    }

    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        let mut doc = self.matcher.approximation_ref().doc_id();
        while doc < up_to {
            if TwoPhaseIterator::matches(self)? {
                bit_set.set((doc - offset) as usize);
            }
            doc = self.matcher.approximation().next_doc()?;
        }
        Ok(())
    }
}

impl Scorable for PhraseScorer {
    fn score(&mut self) -> Result<f32> {
        if self.freq == 0.0 {
            self.freq = self.matcher.sloppy_weight();
            while self.matcher.next_match()? {
                self.freq += self.matcher.sloppy_weight();
            }
        }
        let doc = self.matcher.approximation_ref().doc_id();
        let norm = self.norm(doc)?;
        Ok(self.sim_scorer.score(self.freq, norm))
    }

    fn set_min_competitive_score(&mut self, min_score: f32) -> Result<()> {
        self.min_competitive_score = min_score;
        self.matcher.set_min_competitive_score(min_score);
        Ok(())
    }
}

impl Scorer for PhraseScorer {
    fn as_scorable(&mut self) -> &mut dyn Scorable {
        self
    }

    fn doc_id(&self) -> i32 {
        self.matcher.approximation_ref().doc_id()
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        self
    }

    fn two_phase_iterator(&mut self) -> Option<&mut dyn TwoPhaseIterator> {
        Some(self)
    }

    fn advance_shallow(&mut self, target: i32) -> Result<i32> {
        self.matcher.advance_shallow(target)
    }

    fn get_max_score(&mut self, up_to: i32) -> Result<f32> {
        self.matcher.get_max_score(up_to)
    }
}
