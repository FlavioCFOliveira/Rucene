//! Required-plus-optional scoring, ported from
//! `org.apache.lucene.search.ReqOptSumScorer`.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use crate::search::scorable::{ChildScorable, Scorable};
use crate::search::score_mode::ScoreMode;
use crate::search::scorer::Scorer;
use crate::search::two_phase_iterator::TwoPhaseIterator;

/// Message used where a two-phase view is known to be present.
const TWO_PHASE_INVARIANT: &str =
    "INVARIANT: the flag was set at construction and a Scorer returns a stable view";

/// The approximation-plus-confirmation half of a [`ReqOptSumScorer`].
///
/// Equivalent to the two anonymous inner classes of
/// `org.apache.lucene.search.ReqOptSumScorer`: the `FilterDocIdSetIterator`
/// that drives iteration — plain when scores are not pruned, impact-aware under
/// [`ScoreMode::TOP_SCORES`] — and the `TwoPhaseIterator` built around it.
///
/// **Divergence from Lucene 10.5.0.** Java's inner classes read and write the
/// outer scorer's `minScore`, `optIsRequired` and `upTo` fields and hold
/// references to the very scorers the outer one holds. Rust forbids that, so
/// the state and the two scorers live here once and the outer type is a thin
/// shell over it.
struct ReqOptCore {
    req_scorer: Box<dyn Scorer>,
    opt_scorer: Box<dyn Scorer>,
    req_two_phase: bool,
    opt_two_phase: bool,
    /// Whether the impact-aware approximation is in use, that is, whether the
    /// score mode is [`ScoreMode::TOP_SCORES`].
    top_scores: bool,
    /// Whether Java's `twoPhase` field is non-`null`.
    has_two_phase: bool,
    match_cost: f32,
    req_cost: i64,

    min_score: f32,
    req_max_score: f32,
    opt_is_required: bool,

    /// State of the impact-aware approximation.
    up_to: i32,
    max_score: f32,
}

impl ReqOptCore {
    fn req_approximation(&mut self) -> &mut dyn DocIdSetIterator {
        if self.req_two_phase {
            self.req_scorer
                .two_phase_iterator()
                .expect(TWO_PHASE_INVARIANT)
                .approximation()
        } else {
            self.req_scorer.iterator()
        }
    }

    fn opt_approximation(&mut self) -> &mut dyn DocIdSetIterator {
        if self.opt_two_phase {
            self.opt_scorer
                .two_phase_iterator()
                .expect(TWO_PHASE_INVARIANT)
                .approximation()
        } else {
            self.opt_scorer.iterator()
        }
    }

    /// Equivalent to `ReqOptSumScorer.advanceShallow(int)`, which the
    /// impact-aware approximation calls on the enclosing scorer.
    fn advance_shallow(&mut self, target: i32) -> Result<i32> {
        let mut up_to = self.req_scorer.advance_shallow(target)?;
        let opt_doc = self.opt_scorer.doc_id();
        if opt_doc <= target {
            up_to = up_to.min(self.opt_scorer.advance_shallow(target)?);
        } else if opt_doc != NO_MORE_DOCS {
            up_to = up_to.min(opt_doc - 1);
        }
        Ok(up_to)
    }

    /// Equivalent to `ReqOptSumScorer.getMaxScore(int)`.
    fn get_max_score(&mut self, up_to: i32) -> Result<f32> {
        let mut max_score = self.req_scorer.get_max_score(up_to)?;
        if self.opt_scorer.doc_id() <= up_to {
            max_score += self.opt_scorer.get_max_score(up_to)?;
        }
        Ok(max_score)
    }

    /// Equivalent to the private `moveToNextBlock(int)` of the impact-aware
    /// approximation.
    fn move_to_next_block(&mut self, target: i32) -> Result<()> {
        self.up_to = self.advance_shallow(target)?;
        let req_max_score_block = self.req_scorer.get_max_score(self.up_to)?;
        self.max_score = self.get_max_score(self.up_to)?;

        // Potentially move to a conjunction
        self.opt_is_required = req_max_score_block < self.min_score;
        Ok(())
    }

    /// Equivalent to the private `advanceImpacts(int)`.
    fn advance_impacts(&mut self, mut target: i32) -> Result<i32> {
        if target > self.up_to {
            self.move_to_next_block(target)?;
        }

        loop {
            if self.max_score >= self.min_score {
                return Ok(target);
            }

            if self.up_to == NO_MORE_DOCS {
                return Ok(NO_MORE_DOCS);
            }

            target = self.up_to + 1;

            self.move_to_next_block(target)?;
        }
    }

    /// Equivalent to the private `advanceInternal(int)` of the impact-aware
    /// approximation.
    fn advance_internal(&mut self, target: i32) -> Result<i32> {
        if target == NO_MORE_DOCS {
            self.req_approximation().advance(target)?;
            return Ok(NO_MORE_DOCS);
        }
        let mut req_doc = target;
        'advance_head: loop {
            if self.min_score != 0.0 {
                req_doc = self.advance_impacts(req_doc)?;
            }
            if self.req_scorer.doc_id() < req_doc {
                req_doc = self.req_approximation().advance(req_doc)?;
            }
            if req_doc == NO_MORE_DOCS || !self.opt_is_required {
                return Ok(req_doc);
            }

            let upper_bound = if self.req_max_score < self.min_score {
                NO_MORE_DOCS
            } else {
                self.up_to
            };
            if req_doc > upper_bound {
                continue 'advance_head;
            }

            // Find the next common doc within the current block
            loop {
                // invariant: req_doc >= opt_doc
                let mut opt_doc = self.opt_scorer.doc_id();
                if opt_doc < req_doc {
                    opt_doc = self.opt_approximation().advance(req_doc)?;
                }
                if opt_doc > upper_bound {
                    req_doc = upper_bound + 1;
                    continue 'advance_head;
                }

                if opt_doc != req_doc {
                    req_doc = self.req_approximation().advance(opt_doc)?;
                    if req_doc > upper_bound {
                        continue 'advance_head;
                    }
                }

                if req_doc == NO_MORE_DOCS || opt_doc == req_doc {
                    return Ok(req_doc);
                }
            }
        }
    }
}

impl DocIdSetIterator for ReqOptCore {
    fn doc_id(&self) -> i32 {
        self.req_scorer.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        if self.top_scores {
            let target = self.req_scorer.doc_id() + 1;
            self.advance_internal(target)
        } else {
            self.req_approximation().next_doc()
        }
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        if self.top_scores {
            self.advance_internal(target)
        } else {
            self.req_approximation().advance(target)
        }
    }

    fn cost(&self) -> i64 {
        self.req_cost
    }
}

impl TwoPhaseIterator for ReqOptCore {
    fn approximation(&mut self) -> &mut dyn DocIdSetIterator {
        self
    }

    fn approximation_ref(&self) -> &dyn DocIdSetIterator {
        self
    }

    fn matches(&mut self) -> Result<bool> {
        if self.req_two_phase {
            let matched = self
                .req_scorer
                .two_phase_iterator()
                .expect(TWO_PHASE_INVARIANT)
                .matches()?;
            if !matched {
                return Ok(false);
            }
        }
        if self.opt_two_phase {
            if self.opt_is_required {
                // The below condition is rare and can only happen if we
                // transitioned to optIsRequired=true after the opt approximation
                // was advanced and before it was confirmed.
                if self.req_scorer.doc_id() != self.opt_scorer.doc_id() {
                    if self.opt_scorer.doc_id() < self.req_scorer.doc_id() {
                        let target = self.req_scorer.doc_id();
                        self.opt_approximation().advance(target)?;
                    }
                    if self.req_scorer.doc_id() != self.opt_scorer.doc_id() {
                        return Ok(false);
                    }
                }
                let matched = self
                    .opt_scorer
                    .two_phase_iterator()
                    .expect(TWO_PHASE_INVARIANT)
                    .matches()?;
                if !matched {
                    // Advance the iterator to make it clear it doesn't match the
                    // current doc id
                    self.opt_approximation().next_doc()?;
                    return Ok(false);
                }
            } else if self.opt_scorer.doc_id() == self.req_scorer.doc_id() {
                let matched = self
                    .opt_scorer
                    .two_phase_iterator()
                    .expect(TWO_PHASE_INVARIANT)
                    .matches()?;
                if !matched {
                    // Advance the iterator to make it clear it doesn't match the
                    // current doc id
                    self.opt_approximation().next_doc()?;
                }
            }
        }
        Ok(true)
    }

    fn match_cost(&self) -> f32 {
        self.match_cost
    }
}

/// A scorer for queries with a required part and an optional part.
///
/// Equivalent to `org.apache.lucene.search.ReqOptSumScorer`. It delays
/// advancing the optional part until a score is needed.
pub struct ReqOptSumScorer {
    core: ReqOptCore,
}

impl std::fmt::Debug for ReqOptSumScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReqOptSumScorer")
            .field("top_scores", &self.core.top_scores)
            .field("opt_is_required", &self.core.opt_is_required)
            .finish_non_exhaustive()
    }
}

impl ReqOptSumScorer {
    /// Constructs a required-plus-optional scorer.
    ///
    /// Equivalent to `new ReqOptSumScorer(Scorer, Scorer, ScoreMode)`.
    ///
    /// * `req_scorer` — the required scorer, which must match;
    /// * `opt_scorer` — the optional scorer, used for scoring only;
    /// * `score_mode` — how the produced scorer will be consumed.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while priming the score bounds.
    pub fn new(
        mut req_scorer: Box<dyn Scorer>,
        mut opt_scorer: Box<dyn Scorer>,
        score_mode: ScoreMode,
    ) -> Result<Self> {
        let req_two_phase = req_scorer.two_phase_iterator().is_some();
        let opt_two_phase = opt_scorer.two_phase_iterator().is_some();

        let req_match_cost = if req_two_phase {
            req_scorer
                .two_phase_iterator()
                .expect(TWO_PHASE_INVARIANT)
                .match_cost()
        } else {
            0.0
        };
        let opt_match_cost = if opt_two_phase {
            opt_scorer
                .two_phase_iterator()
                .expect(TWO_PHASE_INVARIANT)
                .match_cost()
        } else {
            0.0
        };
        let req_cost = if req_two_phase {
            req_scorer
                .two_phase_iterator()
                .expect(TWO_PHASE_INVARIANT)
                .approximation_ref()
                .cost()
        } else {
            req_scorer.iterator().cost()
        };

        let top_scores = score_mode == ScoreMode::TOP_SCORES;
        let req_max_score = if top_scores {
            req_scorer.advance_shallow(0)?;
            opt_scorer.advance_shallow(0)?;
            req_scorer.get_max_score(NO_MORE_DOCS)?
        } else {
            f32::INFINITY
        };

        let has_two_phase = req_two_phase || opt_two_phase;
        let mut match_cost = 1.0f32;
        if req_two_phase {
            match_cost += req_match_cost;
        }
        if opt_two_phase {
            match_cost += opt_match_cost;
        }

        Ok(Self {
            core: ReqOptCore {
                req_scorer,
                opt_scorer,
                req_two_phase,
                opt_two_phase,
                top_scores,
                has_two_phase,
                match_cost,
                req_cost,
                min_score: 0.0,
                req_max_score,
                opt_is_required: false,
                up_to: -1,
                max_score: 0.0,
            },
        })
    }

    fn confirm(&mut self, mut doc: i32) -> Result<i32> {
        loop {
            if doc == NO_MORE_DOCS {
                return Ok(NO_MORE_DOCS);
            }
            if self.core.matches()? {
                return Ok(doc);
            }
            doc = self.core.next_doc()?;
        }
    }
}

impl DocIdSetIterator for ReqOptSumScorer {
    fn doc_id(&self) -> i32 {
        self.core.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        let doc = self.core.next_doc()?;
        self.confirm(doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let doc = self.core.advance(target)?;
        self.confirm(doc)
    }

    fn cost(&self) -> i64 {
        self.core.req_cost
    }
}

impl Scorable for ReqOptSumScorer {
    fn score(&mut self) -> Result<f32> {
        // TODO(lucene): sum into a double and cast to float if we ever send
        // required clauses to BS1
        let cur_doc = self.core.req_scorer.doc_id();
        let mut score = self.core.req_scorer.score()?;

        let mut opt_scorer_doc = self.core.opt_scorer.doc_id();
        if opt_scorer_doc < cur_doc {
            opt_scorer_doc = self.core.opt_approximation().advance(cur_doc)?;
            if self.core.opt_two_phase && opt_scorer_doc == cur_doc {
                let matched = self
                    .core
                    .opt_scorer
                    .two_phase_iterator()
                    .expect(TWO_PHASE_INVARIANT)
                    .matches()?;
                if !matched {
                    opt_scorer_doc = self.core.opt_approximation().next_doc()?;
                }
            }
        }
        if opt_scorer_doc == cur_doc {
            score += self.core.opt_scorer.score()?;
        }

        Ok(score)
    }

    fn set_min_competitive_score(&mut self, min_score: f32) -> Result<()> {
        self.core.min_score = min_score;
        // Potentially move to a conjunction
        if self.core.req_max_score < min_score {
            self.core.opt_is_required = true;
            if self.core.req_max_score == 0.0 {
                // If the required clause doesn't contribute scores, we can
                // propagate the minimum competitive score to the optional
                // clause. This happens when the required clause is a FILTER
                // clause.
                self.core.opt_scorer.set_min_competitive_score(min_score)?;
            }
        }
        Ok(())
    }

    fn children(&mut self) -> Result<Vec<ChildScorable<'_>>> {
        Ok(vec![
            ChildScorable::new(self.core.req_scorer.as_scorable(), "MUST"),
            ChildScorable::new(self.core.opt_scorer.as_scorable(), "SHOULD"),
        ])
    }
}

impl Scorer for ReqOptSumScorer {
    fn as_scorable(&mut self) -> &mut dyn Scorable {
        self
    }

    fn doc_id(&self) -> i32 {
        self.core.req_scorer.doc_id()
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        if self.core.has_two_phase {
            self
        } else {
            &mut self.core
        }
    }

    fn two_phase_iterator(&mut self) -> Option<&mut dyn TwoPhaseIterator> {
        if self.core.has_two_phase {
            Some(&mut self.core)
        } else {
            None
        }
    }

    fn advance_shallow(&mut self, target: i32) -> Result<i32> {
        self.core.advance_shallow(target)
    }

    fn get_max_score(&mut self, up_to: i32) -> Result<f32> {
        self.core.get_max_score(up_to)
    }
}
