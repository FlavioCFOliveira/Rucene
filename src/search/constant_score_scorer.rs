//! Constant scoring, ported from
//! `org.apache.lucene.search.ConstantScoreScorer`.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::index::DocAndFloatFeatureBuffer;
use crate::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use crate::search::scorable::Scorable;
use crate::search::score_mode::ScoreMode;
use crate::search::scorer::Scorer;
use crate::search::two_phase_iterator::{TwoPhaseIterator, TwoPhaseIteratorAsDocIdSetIterator};
use crate::util::{Bits, FixedBitSet};

/// Tracks the current doc ID of a wrapped iterator and can be short-circuited,
/// so that a minimum competitive score above the constant score exhausts
/// iteration.
///
/// Equivalent to the private
/// `ConstantScoreScorer.DocIdSetIteratorWrapper`.
///
/// **Divergence from Lucene 10.5.0.** Java short-circuits by replacing the
/// wrapper's `delegate` field with `DocIdSetIterator.empty()`. Rust cannot swap
/// an owned trait object out from under an existing borrow chain, so the
/// wrapper carries an `exhausted` flag instead. The observable behaviour is
/// identical: once set, `next_doc` and `advance` answer
/// [`NO_MORE_DOCS`] and `into_bit_set` writes nothing, which is exactly what a
/// freshly-created empty iterator does.
struct DocIdSetIteratorWrapper {
    doc: i32,
    delegate: Box<dyn DocIdSetIterator>,
    exhausted: bool,
}

impl std::fmt::Debug for DocIdSetIteratorWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocIdSetIteratorWrapper")
            .field("doc", &self.doc)
            .field("exhausted", &self.exhausted)
            .finish_non_exhaustive()
    }
}

impl DocIdSetIteratorWrapper {
    fn new(delegate: Box<dyn DocIdSetIterator>) -> Self {
        Self {
            doc: -1,
            delegate,
            exhausted: false,
        }
    }

    fn exhaust(&mut self) {
        self.exhausted = true;
    }
}

impl DocIdSetIterator for DocIdSetIteratorWrapper {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.doc = if self.exhausted {
            NO_MORE_DOCS
        } else {
            self.delegate.next_doc()?
        };
        Ok(self.doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.doc = if self.exhausted {
            NO_MORE_DOCS
        } else {
            self.delegate.advance(target)?
        };
        Ok(self.doc)
    }

    fn cost(&self) -> i64 {
        self.delegate.cost()
    }

    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        if self.exhausted {
            self.doc = NO_MORE_DOCS;
            return Ok(());
        }
        self.delegate.into_bit_set(up_to, bit_set, offset)?;
        self.doc = self.delegate.doc_id();
        Ok(())
    }
}

/// A two-phase iterator whose approximation is a
/// [`DocIdSetIteratorWrapper`] over the wrapped iterator's own approximation.
///
/// Equivalent to the anonymous `TwoPhaseIterator` that
/// `ConstantScoreScorer(float, ScoreMode, TwoPhaseIterator)` builds when the
/// score mode is [`ScoreMode::TOP_SCORES`]: it confirms matches through the
/// wrapped iterator, but iterates through a wrapper that can be short-circuited.
///
/// **Divergence from Lucene 10.5.0.** Java wraps `twoPhaseIterator.approximation()`
/// — an object it can reach and re-wrap — and keeps the original two-phase
/// iterator alongside. Rust cannot move an approximation out of the iterator
/// that owns it, so this type *is* the approximation: it implements
/// [`DocIdSetIterator`] over the wrapped iterator's approximation and returns
/// itself from [`TwoPhaseIterator::approximation`]. The call sequence is the
/// same.
struct GatedTwoPhaseIterator {
    doc: i32,
    inner: Box<dyn TwoPhaseIterator>,
    exhausted: bool,
}

impl std::fmt::Debug for GatedTwoPhaseIterator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatedTwoPhaseIterator")
            .field("doc", &self.doc)
            .field("exhausted", &self.exhausted)
            .finish_non_exhaustive()
    }
}

impl GatedTwoPhaseIterator {
    fn new(inner: Box<dyn TwoPhaseIterator>) -> Self {
        Self {
            doc: -1,
            inner,
            exhausted: false,
        }
    }

    fn exhaust(&mut self) {
        self.exhausted = true;
    }
}

impl DocIdSetIterator for GatedTwoPhaseIterator {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.doc = if self.exhausted {
            NO_MORE_DOCS
        } else {
            self.inner.approximation().next_doc()?
        };
        Ok(self.doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.doc = if self.exhausted {
            NO_MORE_DOCS
        } else {
            self.inner.approximation().advance(target)?
        };
        Ok(self.doc)
    }

    fn cost(&self) -> i64 {
        self.inner.approximation_ref().cost()
    }

    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        if self.exhausted {
            self.doc = NO_MORE_DOCS;
            return Ok(());
        }
        self.inner
            .approximation()
            .into_bit_set(up_to, bit_set, offset)?;
        self.doc = self.inner.approximation_ref().doc_id();
        Ok(())
    }
}

impl TwoPhaseIterator for GatedTwoPhaseIterator {
    fn approximation(&mut self) -> &mut dyn DocIdSetIterator {
        self
    }

    fn approximation_ref(&self) -> &dyn DocIdSetIterator {
        self
    }

    fn matches(&mut self) -> Result<bool> {
        self.inner.matches()
    }

    fn match_cost(&self) -> f32 {
        self.inner.match_cost()
    }
}

/// The four iteration shapes a [`ConstantScoreScorer`] can take: with or
/// without two-phase confirmation, and with or without the short-circuiting
/// wrapper that [`ScoreMode::TOP_SCORES`] requires.
enum Iteration {
    Plain(Box<dyn DocIdSetIterator>),
    PlainGated(DocIdSetIteratorWrapper),
    TwoPhase(TwoPhaseIteratorAsDocIdSetIterator<dyn TwoPhaseIterator>),
    TwoPhaseGated(TwoPhaseIteratorAsDocIdSetIterator<GatedTwoPhaseIterator>),
}

impl std::fmt::Debug for Iteration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Plain(_) => "Plain",
            Self::PlainGated(_) => "PlainGated",
            Self::TwoPhase(_) => "TwoPhase",
            Self::TwoPhaseGated(_) => "TwoPhaseGated",
        };
        f.write_str(name)
    }
}

/// A constant-scoring [`Scorer`].
///
/// Equivalent to the `final org.apache.lucene.search.ConstantScoreScorer`.
///
/// Java overloads the constructor on whether iteration is driven by a plain
/// iterator or by a two-phase one; Rust has no overloading, so the two are
/// [`from_iterator`](Self::from_iterator) and
/// [`from_two_phase`](Self::from_two_phase).
#[derive(Debug)]
pub struct ConstantScoreScorer {
    score: f32,
    score_mode: ScoreMode,
    iteration: Iteration,
}

impl ConstantScoreScorer {
    /// Builds a scorer driven by a plain iterator; two-phase iteration will not
    /// be supported.
    ///
    /// Equivalent to
    /// `ConstantScoreScorer(float, ScoreMode, DocIdSetIterator)`.
    ///
    /// * `score` — the score to return on each document;
    /// * `score_mode` — the score mode;
    /// * `disi` — the iterator that defines matching documents.
    pub fn from_iterator(
        score: f32,
        score_mode: ScoreMode,
        disi: Box<dyn DocIdSetIterator>,
    ) -> Self {
        // TODO(lucene): Lucene only wraps when this is the top-level scoring
        // clause; see ScorerSupplier#setTopLevelScoringClause. This port keeps
        // Lucene's current, unconditional behaviour.
        let iteration = if score_mode == ScoreMode::TOP_SCORES {
            Iteration::PlainGated(DocIdSetIteratorWrapper::new(disi))
        } else {
            Iteration::Plain(disi)
        };
        Self {
            score,
            score_mode,
            iteration,
        }
    }

    /// Builds a scorer driven by a two-phase iterator, so that the scorer
    /// supports two-phase iteration.
    ///
    /// Equivalent to
    /// `ConstantScoreScorer(float, ScoreMode, TwoPhaseIterator)`.
    ///
    /// * `score` — the score to return on each document;
    /// * `score_mode` — the score mode;
    /// * `two_phase_iterator` — the iterator that defines matching documents.
    pub fn from_two_phase(
        score: f32,
        score_mode: ScoreMode,
        two_phase_iterator: Box<dyn TwoPhaseIterator>,
    ) -> Self {
        let iteration = if score_mode == ScoreMode::TOP_SCORES {
            Iteration::TwoPhaseGated(TwoPhaseIteratorAsDocIdSetIterator::new(Box::new(
                GatedTwoPhaseIterator::new(two_phase_iterator),
            )))
        } else {
            Iteration::TwoPhase(TwoPhaseIteratorAsDocIdSetIterator::new(two_phase_iterator))
        };
        Self {
            score,
            score_mode,
            iteration,
        }
    }
}

impl Scorable for ConstantScoreScorer {
    fn score(&mut self) -> Result<f32> {
        Ok(self.score)
    }

    fn set_min_competitive_score(&mut self, min_score: f32) -> Result<()> {
        if self.score_mode == ScoreMode::TOP_SCORES && min_score > self.score {
            match &mut self.iteration {
                Iteration::PlainGated(wrapper) => wrapper.exhaust(),
                Iteration::TwoPhaseGated(disi) => disi.two_phase_mut().exhaust(),
                // Unreachable: the gated shapes are the only ones built for
                // TOP_SCORES, which Java relies on when it casts the
                // approximation to its wrapper type.
                Iteration::Plain(_) | Iteration::TwoPhase(_) => {
                    debug_assert!(false, "TOP_SCORES always builds a gated iteration");
                }
            }
        }
        Ok(())
    }
}

impl Scorer for ConstantScoreScorer {
    fn as_scorable(&mut self) -> &mut dyn Scorable {
        self
    }

    fn doc_id(&self) -> i32 {
        match &self.iteration {
            Iteration::Plain(disi) => disi.doc_id(),
            Iteration::PlainGated(disi) => disi.doc_id(),
            Iteration::TwoPhase(disi) => disi.doc_id(),
            Iteration::TwoPhaseGated(disi) => disi.doc_id(),
        }
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        match &mut self.iteration {
            Iteration::Plain(disi) => &mut **disi,
            Iteration::PlainGated(disi) => disi,
            Iteration::TwoPhase(disi) => disi,
            Iteration::TwoPhaseGated(disi) => disi,
        }
    }

    fn two_phase_iterator(&mut self) -> Option<&mut dyn TwoPhaseIterator> {
        match &mut self.iteration {
            Iteration::Plain(_) | Iteration::PlainGated(_) => None,
            Iteration::TwoPhase(disi) => Some(disi.two_phase_mut()),
            Iteration::TwoPhaseGated(disi) => Some(disi.two_phase_mut()),
        }
    }

    fn get_max_score(&mut self, _up_to: i32) -> Result<f32> {
        Ok(self.score)
    }

    fn next_docs_and_scores(
        &mut self,
        up_to: i32,
        live_docs: Option<&dyn Bits>,
        buffer: &mut DocAndFloatFeatureBuffer,
    ) -> Result<()> {
        let batch_size = 64;
        buffer.grow_no_copy(batch_size);
        let mut size = 0;
        let mut doc = self.iterator().doc_id();
        while doc < up_to && size < batch_size {
            if live_docs.map_or(true, |bits| bits.get(doc as usize)) {
                buffer.docs[size] = doc;
                size += 1;
            }
            doc = self.iterator().next_doc()?;
        }
        let score = self.score;
        buffer.features[..size].fill(score);
        buffer.size = size;
        Ok(())
    }
}
