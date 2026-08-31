//! The base of exact and sloppy phrase matching, ported from
//! `org.apache.lucene.search.PhraseMatcher`.

#![deny(unsafe_code)]

use std::cell::RefCell;
use std::rc::Rc;

use crate::error::Result;
use crate::index::{
    DocAndFloatFeatureBuffer, FreqAndNormBuffer, Impacts, ImpactsEnum, ImpactsSource, PostingsEnum,
};
use crate::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use crate::search::impacts_disi::ImpactsDISI;
use crate::search::two_phase_iterator::ScorerIterator;
use crate::util::FixedBitSet;

/// A postings list shared between the conjunction that iterates it and the
/// phrase matcher that reads positions from it.
///
/// **Divergence from Lucene 10.5.0.** Java hands the same `PostingsEnum` object
/// to `ConjunctionUtils.intersectIterators`, to the impacts source and to the
/// `PostingsAndPosition` / `PhrasePositions` that reads its positions; all
/// three then advance and query one object. Rust forbids that aliasing, so the
/// enum lives in an `Rc<RefCell<..>>` that every user shares. Only one of them
/// touches it at a time, so no borrow ever overlaps.
#[derive(Clone)]
pub struct SharedPostings {
    inner: Rc<RefCell<Box<dyn ImpactsEnum>>>,
}

impl std::fmt::Debug for SharedPostings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SharedPostings")
    }
}

impl SharedPostings {
    /// Shares a postings list.
    pub fn new(postings: Box<dyn ImpactsEnum>) -> Self {
        Self {
            inner: Rc::new(RefCell::new(postings)),
        }
    }

    /// Returns the term frequency in the current document.
    ///
    /// Equivalent to `PostingsEnum.freq()`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the frequency.
    pub fn freq(&self) -> Result<i32> {
        self.inner.borrow().freq()
    }

    /// Returns the next position.
    ///
    /// Equivalent to `PostingsEnum.nextPosition()`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the position.
    pub fn next_position(&self) -> Result<i32> {
        self.inner.borrow_mut().next_position()
    }

    /// Returns the start offset of the current position.
    ///
    /// Equivalent to `PostingsEnum.startOffset()`.
    pub fn start_offset(&self) -> i32 {
        self.inner.borrow().start_offset()
    }

    /// Returns the end offset of the current position.
    ///
    /// Equivalent to `PostingsEnum.endOffset()`.
    pub fn end_offset(&self) -> i32 {
        self.inner.borrow().end_offset()
    }

    /// Returns the cost of this postings list.
    ///
    /// Equivalent to `DocIdSetIterator.cost()`.
    pub fn cost(&self) -> i64 {
        self.inner.borrow().cost()
    }
}

impl DocIdSetIterator for SharedPostings {
    fn doc_id(&self) -> i32 {
        self.inner.borrow().doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.inner.borrow_mut().next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.inner.borrow_mut().advance(target)
    }

    fn cost(&self) -> i64 {
        self.inner.borrow().cost()
    }

    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        self.inner.borrow_mut().into_bit_set(up_to, bit_set, offset)
    }
}

impl ImpactsSource for SharedPostings {
    fn advance_shallow(&mut self, target: i32) -> Result<()> {
        self.inner.borrow_mut().advance_shallow(target)
    }

    fn get_impacts(&mut self) -> Result<Box<dyn Impacts>> {
        self.inner.borrow_mut().get_impacts()
    }
}

impl PostingsEnum for SharedPostings {
    fn freq(&self) -> Result<i32> {
        self.inner.borrow().freq()
    }

    fn next_position(&mut self) -> Result<i32> {
        self.inner.borrow_mut().next_position()
    }

    fn start_offset(&self) -> i32 {
        self.inner.borrow().start_offset()
    }

    fn end_offset(&self) -> i32 {
        self.inner.borrow().end_offset()
    }

    /// **Divergence from Lucene 10.5.0.** The payload of the current position
    /// cannot travel out of the shared borrow, so this handle reports no
    /// payload. Nothing that shares a postings list this way — the phrase
    /// matchers, [`SynonymQuery`](crate::search::SynonymQuery) and
    /// [`CombinedFieldQuery`](crate::search::CombinedFieldQuery) — reads
    /// payloads; reach the underlying enum directly when they are needed.
    fn get_payload(&self) -> Result<Option<&[u8]>> {
        Ok(None)
    }

    fn next_postings(&mut self, up_to: i32, buffer: &mut DocAndFloatFeatureBuffer) -> Result<()> {
        self.inner.borrow_mut().next_postings(up_to, buffer)
    }
}

impl ImpactsEnum for SharedPostings {}

/// A [`DocIdSetIterator`] paired with the [`ImpactsSource`] that describes it.
///
/// **This has no counterpart in Lucene 10.5.0.** Java passes the iterator and
/// the impacts source to `new ImpactsDISI(iterator, new MaxScoreCache(source,
/// scorer))` as two separate objects. This port's
/// [`MaxScoreCache`](crate::search::MaxScoreCache) does not hold its source —
/// see its own documentation — so [`ImpactsDISI`] holds one value that is
/// both, which is what this type is. The phrase matchers,
/// [`SynonymQuery`](crate::search::SynonymQuery) and
/// [`CombinedFieldQuery`](crate::search::CombinedFieldQuery) all build one.
pub struct IteratorWithImpacts<I: DocIdSetIterator = Box<dyn DocIdSetIterator>> {
    iterator: I,
    impacts: Box<dyn ImpactsSource>,
}

impl<I: DocIdSetIterator> std::fmt::Debug for IteratorWithImpacts<I> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("IteratorWithImpacts")
    }
}

impl<I: DocIdSetIterator> IteratorWithImpacts<I> {
    /// Pairs an iterator with the impacts that describe it.
    pub fn new(iterator: I, impacts: Box<dyn ImpactsSource>) -> Self {
        Self { iterator, impacts }
    }

    /// Returns the wrapped iterator.
    pub fn iterator_ref(&self) -> &I {
        &self.iterator
    }

    /// Returns the wrapped iterator for mutation.
    pub fn iterator_mut(&mut self) -> &mut I {
        &mut self.iterator
    }
}

impl IteratorWithImpacts<Box<dyn DocIdSetIterator>> {
    /// Pairs a [`ScorerIterator`] with the impacts that describe it.
    pub fn from_scorer_iterator(iterator: ScorerIterator, impacts: Box<dyn ImpactsSource>) -> Self {
        Self::new(iterator.into_doc_id_set_iterator(), impacts)
    }
}

impl<I: DocIdSetIterator> DocIdSetIterator for IteratorWithImpacts<I> {
    fn doc_id(&self) -> i32 {
        self.iterator.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.iterator.next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.iterator.advance(target)
    }

    fn cost(&self) -> i64 {
        self.iterator.cost()
    }

    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        self.iterator.into_bit_set(up_to, bit_set, offset)
    }
}

impl<I: DocIdSetIterator> ImpactsSource for IteratorWithImpacts<I> {
    fn advance_shallow(&mut self, target: i32) -> Result<()> {
        self.impacts.advance_shallow(target)
    }

    fn get_impacts(&mut self) -> Result<Box<dyn Impacts>> {
        self.impacts.get_impacts()
    }
}

/// The impacts-aware approximation both phrase matchers expose.
pub type PhraseImpactsDISI = ImpactsDISI<IteratorWithImpacts>;

/// The base of exact and sloppy phrase matching.
///
/// Equivalent to the abstract class `org.apache.lucene.search.PhraseMatcher`.
/// To find matches on a document, first advance
/// [`approximation`](Self::approximation) to the relevant document, then call
/// [`reset_positions`](Self::reset_positions); the matches are then iterated
/// with [`next_match`](Self::next_match).
///
/// **Divergence from Lucene 10.5.0.** Java's `impactsApproximation()` hands the
/// `ImpactsDISI` itself to `PhraseScorer`, which then reads its `MaxScoreCache`
/// and pushes the minimum competitive score into it. That object owns the
/// approximation in this port, and the two matchers wrap different iterators,
/// so the three operations the scorer performs on it —
/// [`set_min_competitive_score`](Self::set_min_competitive_score),
/// [`advance_shallow`](Self::advance_shallow) and
/// [`get_max_score`](Self::get_max_score) — are declared here instead.
pub trait PhraseMatcher {
    /// An approximation that only matches documents that have all terms.
    ///
    /// Equivalent to `PhraseMatcher.approximation()`.
    fn approximation(&mut self) -> &mut dyn DocIdSetIterator;

    /// The same approximation, for inspection.
    ///
    /// The shared-borrow sibling of [`approximation`](Self::approximation); see
    /// [`TwoPhaseIterator::approximation_ref`](crate::search::TwoPhaseIterator::approximation_ref)
    /// for why this port needs the two uses kept apart.
    fn approximation_ref(&self) -> &dyn DocIdSetIterator;

    /// An upper bound on the number of possible matches on this document.
    ///
    /// Equivalent to `PhraseMatcher.maxFreq()`. It may be called before
    /// [`reset_positions`](Self::reset_positions), to enable early termination
    /// of non-competitive documents in
    /// [`ScoreMode::TOP_SCORES`](crate::search::ScoreMode::TOP_SCORES) mode, as
    /// long as the approximation has been advanced to the target document.
    /// Implementations lazily load any required state — term frequencies, for
    /// instance — on first access per document.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the frequencies.
    fn max_freq(&mut self) -> Result<f32>;

    /// Called after the approximation has been advanced, to load the positions
    /// for matching.
    ///
    /// Equivalent to `PhraseMatcher.resetPositions()`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading positions.
    fn reset_positions(&mut self) -> Result<()>;

    /// Finds the next match on the current document, returning `false` if there
    /// are none.
    ///
    /// Equivalent to `PhraseMatcher.nextMatch()`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading positions.
    fn next_match(&mut self) -> Result<bool>;

    /// The slop-adjusted weight of the current match; the sum of these weights
    /// is the frequency used for scoring.
    ///
    /// Equivalent to `PhraseMatcher.sloppyWeight()`.
    fn sloppy_weight(&self) -> f32;

    /// The start position of the current match.
    ///
    /// Equivalent to `PhraseMatcher.startPosition()`.
    fn start_position(&self) -> i32;

    /// The end position of the current match.
    ///
    /// Equivalent to `PhraseMatcher.endPosition()`.
    fn end_position(&self) -> i32;

    /// The start offset of the current match.
    ///
    /// Equivalent to `PhraseMatcher.startOffset()`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the offset.
    fn start_offset(&self) -> Result<i32>;

    /// The end offset of the current match.
    ///
    /// Equivalent to `PhraseMatcher.endOffset()`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the offset.
    fn end_offset(&self) -> Result<i32>;

    /// An estimate of the average cost of finding all matches on a document.
    ///
    /// Equivalent to `PhraseMatcher.getMatchCost()`; see
    /// [`TwoPhaseIterator::match_cost`](crate::search::TwoPhaseIterator::match_cost).
    fn get_match_cost(&self) -> f32;

    /// Pushes a minimum competitive score into the impacts approximation.
    ///
    /// Equivalent to `impactsApproximation().setMinCompetitiveScore(float)`.
    fn set_min_competitive_score(&mut self, min_score: f32);

    /// Shallow-advances the impacts approximation.
    ///
    /// Equivalent to
    /// `impactsApproximation().getMaxScoreCache().advanceShallow(int)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the impacts.
    fn advance_shallow(&mut self, target: i32) -> Result<i32>;

    /// Returns the maximum score up to `up_to`, included.
    ///
    /// Equivalent to
    /// `impactsApproximation().getMaxScoreCache().getMaxScore(int)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the impacts.
    fn get_max_score(&mut self, up_to: i32) -> Result<f32>;
}

/// An [`ImpactsSource`] that reports a single, unbounded dummy impact.
///
/// Equivalent to the anonymous `ImpactsSource`
/// [`SloppyPhraseMatcher`](crate::search::SloppyPhraseMatcher) installs: a good
/// upper bound of the sloppy frequency would be the sum of the sub-frequencies,
/// which is correct but usually so much higher than the actual sloppy frequency
/// that it does not help skip irrelevant documents, so sloppy phrase queries
/// use dummy impacts.
#[derive(Debug, Default, Clone, Copy)]
pub struct DummyImpactsSource;

impl ImpactsSource for DummyImpactsSource {
    fn advance_shallow(&mut self, _target: i32) -> Result<()> {
        Ok(())
    }

    fn get_impacts(&mut self) -> Result<Box<dyn Impacts>> {
        Ok(Box::new(DummyImpacts))
    }
}

/// The single dummy impact [`DummyImpactsSource`] reports.
#[derive(Debug, Default, Clone, Copy)]
struct DummyImpacts;

impl Impacts for DummyImpacts {
    fn num_levels(&self) -> i32 {
        1
    }

    fn doc_id_up_to(&self, _level: i32) -> i32 {
        NO_MORE_DOCS
    }

    fn get_impacts(&self, _level: i32) -> FreqAndNormBuffer {
        let mut buffer = FreqAndNormBuffer::new();
        buffer.add(i32::MAX, 1);
        buffer
    }
}
