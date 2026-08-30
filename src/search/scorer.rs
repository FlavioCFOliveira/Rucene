//! Scorers, ported from `org.apache.lucene.search.Scorer` and
//! `org.apache.lucene.search.FilterScorer`.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::index::DocAndFloatFeatureBuffer;
use crate::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use crate::search::scorable::{ChildScorable, Scorable};
use crate::search::two_phase_iterator::{ScorerIterator, TwoPhaseIterator};
use crate::util::Bits;

/// Common scoring functionality for the different types of queries.
///
/// Equivalent to the abstract class `org.apache.lucene.search.Scorer`, which
/// extends `Scorable`. A scorer exposes an [`iterator`](Self::iterator) over
/// the documents matching a query in increasing order of doc ID.
pub trait Scorer: Scorable {
    /// Returns this scorer viewed as a [`Scorable`].
    ///
    /// **Divergence from Lucene 10.5.0.** Java gets this for free, because
    /// `Scorer extends Scorable`. Rust before 1.86 cannot coerce a
    /// `&mut dyn Scorer` into a `&mut dyn Scorable`, and this crate's minimum
    /// supported Rust version is 1.80, so the upcast is spelled out as a
    /// method. Every implementation writes `self`.
    fn as_scorable(&mut self) -> &mut dyn Scorable;

    /// Returns the doc ID that is currently being scored.
    ///
    /// Equivalent to `Scorer.docID()`.
    fn doc_id(&self) -> i32;

    /// Returns an iterator over the matching documents.
    ///
    /// Equivalent to `Scorer.iterator()`. The returned iterator is positioned
    /// on `-1` if no documents have been scored yet, on [`NO_MORE_DOCS`] if all
    /// documents have been scored already, and on the last scored document
    /// otherwise.
    ///
    /// As in Java the returned iterator is a *view*: its state is the scorer's
    /// state, so calling this method several times returns iterators that agree
    /// with each other. Rust expresses the view as a borrow of the scorer
    /// rather than as an independent object.
    fn iterator(&mut self) -> &mut dyn DocIdSetIterator;

    /// Optionally returns a [`TwoPhaseIterator`] view of this scorer, or `None`
    /// when two-phase iteration is not supported.
    ///
    /// Equivalent to `Scorer.twoPhaseIterator()`, which returns `null` by
    /// default. The returned iterator's
    /// [`approximation`](TwoPhaseIterator::approximation) must advance
    /// synchronously with [`iterator`](Self::iterator): advancing one advances
    /// the other. Implementing this method is typically useful on scorers with
    /// a high per-document overhead for confirming matches.
    fn two_phase_iterator(&mut self) -> Option<&mut dyn TwoPhaseIterator> {
        None
    }

    /// Advances to the block of documents that contains `target` in order to
    /// get scoring information about that block.
    ///
    /// Equivalent to `Scorer.advanceShallow(int)`, which returns
    /// [`NO_MORE_DOCS`] by default. This method is implicitly called by
    /// [`DocIdSetIterator::advance`] and [`DocIdSetIterator::next_doc`] on the
    /// returned doc ID. Calling it does not modify the current
    /// [`doc_id`](Self::doc_id). It returns a number that is greater than or
    /// equal to all documents contained in the current block, but less than any
    /// doc ID of the next block. `target` must be `>=` [`doc_id`](Self::doc_id)
    /// as well as all targets passed to this method so far.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the block metadata.
    fn advance_shallow(&mut self, _target: i32) -> Result<i32> {
        Ok(NO_MORE_DOCS)
    }

    /// Returns the maximum score that documents between the last `target` this
    /// scorer was [shallow-advanced](Self::advance_shallow) to, included, and
    /// `up_to`, included, may have.
    ///
    /// Equivalent to `Scorer.getMaxScore(int)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the impacts.
    fn get_max_score(&mut self, up_to: i32) -> Result<f32>;

    /// Returns a new batch of doc IDs and scores, starting at the current doc
    /// ID and ending before `up_to`.
    ///
    /// Equivalent to `Scorer.nextDocsAndScores(int, Bits,
    /// DocAndFloatFeatureBuffer)`. Because it starts on the current doc ID, it
    /// is illegal to call this method when [`doc_id`](Self::doc_id) is `-1`. An
    /// empty result indicates that there are no postings left between the
    /// current doc ID and `up_to`.
    ///
    /// Implementations should ideally fill the buffer with between 8 and a
    /// couple of hundred entries, to keep heap requirements contained while
    /// still being large enough for operations on the buffer to vectorise. When
    /// this scorer exposes a [`two_phase_iterator`](Self::two_phase_iterator),
    /// it must be positioned on a matching document before this method is
    /// called.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while iterating or scoring.
    fn next_docs_and_scores(
        &mut self,
        up_to: i32,
        live_docs: Option<&dyn Bits>,
        buffer: &mut DocAndFloatFeatureBuffer,
    ) -> Result<()> {
        let batch_size = 64; // arbitrary
        buffer.grow_no_copy(batch_size);
        let mut size = 0;
        let mut doc = self.doc_id();
        while doc < up_to && size < batch_size {
            if live_docs.map_or(true, |bits| bits.get(doc as usize)) {
                buffer.docs[size] = doc;
                buffer.features[size] = self.score()?;
                size += 1;
            }
            doc = self.iterator().next_doc()?;
        }
        buffer.size = size;
        Ok(())
    }
}

/// A [`DocIdSetIterator`] that owns the [`Scorer`] it iterates.
///
/// **Divergence from Lucene 10.5.0.** Java writes `scorer.iterator()` and hands
/// the result to a `ConstantScoreScorer`, because the scorer stays alive
/// independently of the iterator it lent out. Rust needs whoever iterates to own
/// the iteration, so this adapter takes the scorer and forwards every call to
/// `scorer.iterator()`. The one behaviour it cannot forward is
/// [`DocIdSetIterator::doc_id_run_end`], which the port declares on `&self`
/// while [`Scorer::iterator`] needs `&mut self`; it therefore falls back to the
/// trait's default, `doc_id() + 1`. That is always a legal answer — the contract
/// only requires the end of *a* run of matching doc IDs containing the current
/// one — so it can cost a bulk scorer a larger window, never a different set of
/// matches.
pub struct ScorerAsIterator {
    scorer: Box<dyn Scorer>,
    cost: i64,
}

impl std::fmt::Debug for ScorerAsIterator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScorerAsIterator")
            .field("cost", &self.cost)
            .finish_non_exhaustive()
    }
}

impl ScorerAsIterator {
    /// Takes ownership of the given scorer and iterates it.
    ///
    /// Equivalent to `scorer.iterator()`.
    pub fn new(mut scorer: Box<dyn Scorer>) -> Self {
        let cost = scorer.iterator().cost();
        Self { scorer, cost }
    }

    /// Unwraps this view, returning the scorer it was built from.
    pub fn into_scorer(self) -> Box<dyn Scorer> {
        self.scorer
    }
}

impl DocIdSetIterator for ScorerAsIterator {
    fn doc_id(&self) -> i32 {
        self.scorer.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.scorer.iterator().next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.scorer.iterator().advance(target)
    }

    fn cost(&self) -> i64 {
        self.cost
    }

    fn into_bit_set(
        &mut self,
        up_to: i32,
        bit_set: &mut crate::util::FixedBitSet,
        offset: i32,
    ) -> Result<()> {
        self.scorer.iterator().into_bit_set(up_to, bit_set, offset)
    }
}

/// A [`TwoPhaseIterator`] that owns the [`Scorer`] whose two-phase view it
/// exposes.
///
/// **Divergence from Lucene 10.5.0.** The counterpart of [`ScorerAsIterator`]
/// for `scorer.twoPhaseIterator()`; see that type for why the scorer has to be
/// owned. This adapter is its own approximation, so that
/// [`TwoPhaseIterator::approximation_ref`], which the port declares on `&self`,
/// can be answered without borrowing the scorer mutably.
pub struct ScorerAsTwoPhaseIterator {
    scorer: Box<dyn Scorer>,
    cost: i64,
    match_cost: f32,
}

impl std::fmt::Debug for ScorerAsTwoPhaseIterator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScorerAsTwoPhaseIterator")
            .field("cost", &self.cost)
            .field("match_cost", &self.match_cost)
            .finish_non_exhaustive()
    }
}

/// Message used where the two-phase view is known to be present because it was
/// observed once at construction and a scorer returns a stable view.
const TWO_PHASE_INVARIANT: &str =
    "INVARIANT: the two-phase view was observed at construction and a Scorer returns a stable view";

impl ScorerAsTwoPhaseIterator {
    /// Takes ownership of the given scorer and exposes its two-phase view.
    ///
    /// Equivalent to `scorer.twoPhaseIterator()`.
    ///
    /// # Panics
    ///
    /// Panics when the scorer has no two-phase view; use
    /// [`into_scorer_iterator`] rather than calling this directly.
    pub fn new(mut scorer: Box<dyn Scorer>) -> Self {
        let view = scorer
            .two_phase_iterator()
            .expect("a ScorerAsTwoPhaseIterator requires a scorer with a two-phase view");
        let match_cost = view.match_cost();
        let cost = view.approximation_ref().cost();
        Self {
            scorer,
            cost,
            match_cost,
        }
    }

    /// Unwraps this view, returning the scorer it was built from.
    pub fn into_scorer(self) -> Box<dyn Scorer> {
        self.scorer
    }
}

impl DocIdSetIterator for ScorerAsTwoPhaseIterator {
    fn doc_id(&self) -> i32 {
        self.scorer.doc_id()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.scorer
            .two_phase_iterator()
            .expect(TWO_PHASE_INVARIANT)
            .approximation()
            .next_doc()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.scorer
            .two_phase_iterator()
            .expect(TWO_PHASE_INVARIANT)
            .approximation()
            .advance(target)
    }

    fn cost(&self) -> i64 {
        self.cost
    }

    fn into_bit_set(
        &mut self,
        up_to: i32,
        bit_set: &mut crate::util::FixedBitSet,
        offset: i32,
    ) -> Result<()> {
        self.scorer
            .two_phase_iterator()
            .expect(TWO_PHASE_INVARIANT)
            .approximation()
            .into_bit_set(up_to, bit_set, offset)
    }
}

impl TwoPhaseIterator for ScorerAsTwoPhaseIterator {
    fn approximation(&mut self) -> &mut dyn DocIdSetIterator {
        self
    }

    fn approximation_ref(&self) -> &dyn DocIdSetIterator {
        self
    }

    fn matches(&mut self) -> Result<bool> {
        self.scorer
            .two_phase_iterator()
            .expect(TWO_PHASE_INVARIANT)
            .matches()
    }

    fn match_cost(&self) -> f32 {
        self.match_cost
    }

    fn doc_id_run_end(&mut self) -> Result<i32> {
        self.scorer
            .two_phase_iterator()
            .expect(TWO_PHASE_INVARIANT)
            .doc_id_run_end()
    }

    fn into_bit_set(
        &mut self,
        up_to: i32,
        bit_set: &mut crate::util::FixedBitSet,
        offset: i32,
    ) -> Result<()> {
        self.scorer
            .two_phase_iterator()
            .expect(TWO_PHASE_INVARIANT)
            .into_bit_set(up_to, bit_set, offset)
    }
}

/// Splits an owned scorer into the iteration shape it exposes.
///
/// Equivalent to the `scorer.twoPhaseIterator() != null ? scorer.twoPhaseIterator()
/// : scorer.iterator()` choice Lucene writes wherever it builds a
/// [`ConstantScoreScorer`](crate::search::ConstantScoreScorer) around another
/// scorer's iteration.
pub fn into_scorer_iterator(mut scorer: Box<dyn Scorer>) -> ScorerIterator {
    if scorer.two_phase_iterator().is_some() {
        ScorerIterator::TwoPhase(Box::new(ScorerAsTwoPhaseIterator::new(scorer)))
    } else {
        ScorerIterator::Simple(Box::new(ScorerAsIterator::new(scorer)))
    }
}

/// A scorer that contains another scorer, which it uses as its basic source of
/// data, possibly transforming the data along the way.
///
/// Equivalent to `org.apache.lucene.search.FilterScorer`, whose `docID()`,
/// `iterator()` and `twoPhaseIterator()` are `final` delegations and whose
/// `unwrap()` comes from `Unwrappable<Scorer>` — reproduced here as
/// [`inner`](Self::inner) and [`into_inner`](Self::into_inner).
///
/// **Divergence from Lucene 10.5.0.** Java leaves `getMaxScore(int)` abstract
/// on purpose, since the point of the class is to change how the score is
/// computed. Rust has no abstract methods on a concrete struct, so this wrapper
/// is the identity filter: it delegates `score()` and `getMaxScore()` too. A
/// "subclass" is a type that holds a `FilterScorer` and overrides those two by
/// composition.
pub struct FilterScorer {
    inner: Box<dyn Scorer>,
}

impl FilterScorer {
    /// Wraps the given scorer.
    ///
    /// Equivalent to `new FilterScorer(Scorer)`; Java's null check is not
    /// needed because `Box<dyn Scorer>` cannot be null.
    pub fn new(inner: Box<dyn Scorer>) -> Self {
        Self { inner }
    }

    /// Returns the wrapped scorer.
    ///
    /// Equivalent to the `protected final Scorer in` field.
    pub fn inner(&self) -> &dyn Scorer {
        &*self.inner
    }

    /// Returns the wrapped scorer for mutation.
    pub fn inner_mut(&mut self) -> &mut dyn Scorer {
        &mut *self.inner
    }

    /// Unwraps this scorer.
    ///
    /// Equivalent to `FilterScorer.unwrap()`.
    pub fn into_inner(self) -> Box<dyn Scorer> {
        self.inner
    }
}

impl std::fmt::Debug for FilterScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FilterScorer")
    }
}

impl Scorable for FilterScorer {
    fn score(&mut self) -> Result<f32> {
        self.inner.score()
    }

    fn smoothing_score(&mut self, doc_id: i32) -> Result<f32> {
        self.inner.smoothing_score(doc_id)
    }

    fn set_min_competitive_score(&mut self, min_score: f32) -> Result<()> {
        self.inner.set_min_competitive_score(min_score)
    }

    fn children(&mut self) -> Result<Vec<ChildScorable<'_>>> {
        self.inner.children()
    }
}

impl Scorer for FilterScorer {
    fn as_scorable(&mut self) -> &mut dyn Scorable {
        self
    }

    fn doc_id(&self) -> i32 {
        self.inner.doc_id()
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        self.inner.iterator()
    }

    fn two_phase_iterator(&mut self) -> Option<&mut dyn TwoPhaseIterator> {
        self.inner.two_phase_iterator()
    }

    fn advance_shallow(&mut self, target: i32) -> Result<i32> {
        self.inner.advance_shallow(target)
    }

    fn get_max_score(&mut self, up_to: i32) -> Result<f32> {
        self.inner.get_max_score(up_to)
    }

    fn next_docs_and_scores(
        &mut self,
        up_to: i32,
        live_docs: Option<&dyn Bits>,
        buffer: &mut DocAndFloatFeatureBuffer,
    ) -> Result<()> {
        self.inner.next_docs_and_scores(up_to, live_docs, buffer)
    }
}
