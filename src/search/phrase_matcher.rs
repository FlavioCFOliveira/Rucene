//! The base of exact and sloppy phrase matching, ported from
//! `org.apache.lucene.search.PhraseMatcher`.

#![deny(unsafe_code)]

use std::cell::RefCell;
use std::rc::Rc;

use crate::codecs::lucene104::postings_reader::BlockPostingsEnum;
use crate::error::Result;
use crate::index::{
    DocAndFloatFeatureBuffer, FreqAndNormBuffer, Impacts, ImpactsEnum, ImpactsRef, ImpactsSource,
    PostingsEnum,
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
///
/// **Static fast path (rmp #679–#682).** Java's JIT inlines the postings calls
/// of the phrase matchers, which are monomorphic in practice
/// (`BlockPostingsEnum`). The port reaches the enum through a `dyn` call per
/// `freq`, `nextPosition`, `advance`, `nextDoc` and `docID`, so a
/// `BlockPostingsEnum` handed in is moved out of its box once, through the
/// hidden [`PostingsEnum::into_any`] hook, and every call on it is then
/// static and inlinable. Any other enum keeps the `dyn` path. The calls and
/// their results are the same either way.
#[derive(Clone)]
pub struct SharedPostings {
    inner: Rc<RefCell<SharedEnum>>,
}

/// The enum a [`SharedPostings`] holds: the core's block postings enum by its
/// concrete type, or any other enum.
enum SharedEnum {
    Block(Box<BlockPostingsEnum>),
    Dyn(Box<dyn ImpactsEnum>),
}

/// Calls `$body` with `$e` bound to the enum, statically for the block enum.
macro_rules! with_enum {
    ($shared:expr, $e:ident => $body:expr) => {
        match $shared {
            SharedEnum::Block($e) => $body,
            SharedEnum::Dyn($e) => $body,
        }
    };
}

impl std::fmt::Debug for SharedPostings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SharedPostings")
    }
}

impl SharedPostings {
    /// Shares a postings list.
    pub fn new(postings: Box<dyn ImpactsEnum>) -> Self {
        let is_block = postings
            .disi_as_any()
            .is_some_and(|any| any.is::<BlockPostingsEnum>());
        let shared = if is_block {
            SharedEnum::Block(
                postings
                    .into_any()
                    .and_then(|any| any.downcast::<BlockPostingsEnum>().ok())
                    .expect("INVARIANT: disi_as_any named a BlockPostingsEnum"),
            )
        } else {
            SharedEnum::Dyn(postings)
        };
        Self {
            inner: Rc::new(RefCell::new(shared)),
        }
    }

    /// Returns the term frequency in the current document.
    ///
    /// Equivalent to `PostingsEnum.freq()`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the frequency.
    #[inline]
    pub fn freq(&self) -> Result<i32> {
        with_enum!(&*self.inner.borrow(), e => e.freq())
    }

    /// Returns the next position.
    ///
    /// Equivalent to `PostingsEnum.nextPosition()`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the position.
    #[inline]
    pub fn next_position(&self) -> Result<i32> {
        with_enum!(&mut *self.inner.borrow_mut(), e => e.next_position())
    }

    /// Returns the start offset of the current position.
    ///
    /// Equivalent to `PostingsEnum.startOffset()`.
    pub fn start_offset(&self) -> i32 {
        with_enum!(&*self.inner.borrow(), e => e.start_offset())
    }

    /// Returns the end offset of the current position.
    ///
    /// Equivalent to `PostingsEnum.endOffset()`.
    pub fn end_offset(&self) -> i32 {
        with_enum!(&*self.inner.borrow(), e => e.end_offset())
    }

    /// Returns the cost of this postings list.
    ///
    /// Equivalent to `DocIdSetIterator.cost()`.
    pub fn cost(&self) -> i64 {
        with_enum!(&*self.inner.borrow(), e => e.cost())
    }
}

impl DocIdSetIterator for SharedPostings {
    #[inline]
    fn doc_id(&self) -> i32 {
        with_enum!(&*self.inner.borrow(), e => e.doc_id())
    }

    #[inline]
    fn next_doc(&mut self) -> Result<i32> {
        with_enum!(&mut *self.inner.borrow_mut(), e => e.next_doc())
    }

    #[inline]
    fn advance(&mut self, target: i32) -> Result<i32> {
        with_enum!(&mut *self.inner.borrow_mut(), e => e.advance(target))
    }

    fn cost(&self) -> i64 {
        with_enum!(&*self.inner.borrow(), e => e.cost())
    }

    fn into_bit_set(&mut self, up_to: i32, bit_set: &mut FixedBitSet, offset: i32) -> Result<()> {
        with_enum!(&mut *self.inner.borrow_mut(), e => e.into_bit_set(up_to, bit_set, offset))
    }
}

impl ImpactsSource for SharedPostings {
    fn advance_shallow(&mut self, target: i32) -> Result<()> {
        with_enum!(&mut *self.inner.borrow_mut(), e => e.advance_shallow(target))
    }

    /// The shared enum's own view, lent with the `RefCell` borrow held for as
    /// long as the view lives ([`ImpactsRef::Guarded`]): Java returns the
    /// enum's view, which reads the enum's fields until it is advanced. A
    /// view the enum builds per call (Java's `new Impacts()`) cannot outlive
    /// that borrow here, so it is copied, level by level, into an owned
    /// [`CopiedImpacts`].
    fn get_impacts(&mut self) -> Result<ImpactsRef<'_>> {
        let mut built: Option<Result<CopiedImpacts>> = None;
        let lent = std::cell::RefMut::filter_map(self.inner.borrow_mut(), |inner| {
            let impacts = match inner {
                SharedEnum::Block(e) => e.get_impacts(),
                SharedEnum::Dyn(e) => e.get_impacts(),
            };
            match impacts {
                Ok(ImpactsRef::Borrowed(view)) => Some(view),
                Ok(mut view) => {
                    built = Some(Ok(CopiedImpacts::copy_of(&mut *view)));
                    None
                }
                Err(error) => {
                    built = Some(Err(error));
                    None
                }
            }
        });
        match lent {
            Ok(view) => Ok(ImpactsRef::Guarded(view)),
            Err(_) => Ok(ImpactsRef::Owned(Box::new(built.expect(
                "INVARIANT: the closure records a view or an error when it lends none",
            )?))),
        }
    }
}

impl std::fmt::Display for SharedPostings {
    /// Port-only sharing wrapper with no Lucene counterpart: Java hands the
    /// same `ImpactsEnum` object to three collaborators, so what would be
    /// rendered there is the enumeration itself. This forwards to it.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        with_enum!(&*self.inner.borrow(), e => std::fmt::Display::fmt(&**e, f))
    }
}

impl PostingsEnum for SharedPostings {
    #[inline]
    fn freq(&self) -> Result<i32> {
        SharedPostings::freq(self)
    }

    #[inline]
    fn next_position(&mut self) -> Result<i32> {
        SharedPostings::next_position(self)
    }

    fn start_offset(&self) -> i32 {
        SharedPostings::start_offset(self)
    }

    fn end_offset(&self) -> i32 {
        SharedPostings::end_offset(self)
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
        with_enum!(&mut *self.inner.borrow_mut(), e => e.next_postings(up_to, buffer))
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

    fn get_impacts(&mut self) -> Result<ImpactsRef<'_>> {
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

    /// An approximation that is aware of impacts, used for early termination
    /// in [`ScoreMode::TOP_SCORES`](crate::search::ScoreMode::TOP_SCORES) mode.
    ///
    /// Equivalent to the abstract `PhraseMatcher#impactsApproximation`
    /// (`PhraseMatcher.java:43`). Java's `PhraseScorer` keeps the returned
    /// object in a field; here the matcher owns it and lends it out, so the
    /// three methods below reach it through this accessor every time.
    fn impacts_approximation(&mut self) -> &mut PhraseImpactsDISI;

    /// Pushes a minimum competitive score into the impacts approximation.
    ///
    /// Equivalent to `impactsApproximation().setMinCompetitiveScore(float)`.
    fn set_min_competitive_score(&mut self, min_score: f32) {
        self.impacts_approximation()
            .set_min_competitive_score(min_score);
    }

    /// Shallow-advances the impacts approximation.
    ///
    /// Equivalent to
    /// `impactsApproximation().getMaxScoreCache().advanceShallow(int)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the impacts.
    fn advance_shallow(&mut self, target: i32) -> Result<i32> {
        let (source, cache) = self.impacts_approximation().split_mut();
        cache.advance_shallow(source, target)
    }

    /// Returns the maximum score up to `up_to`, included.
    ///
    /// Equivalent to
    /// `impactsApproximation().getMaxScoreCache().getMaxScore(int)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the impacts.
    fn get_max_score(&mut self, up_to: i32) -> Result<f32> {
        let (source, cache) = self.impacts_approximation().split_mut();
        cache.get_max_score(source, up_to)
    }
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

    fn get_impacts(&mut self) -> Result<ImpactsRef<'_>> {
        Ok(ImpactsRef::Owned(Box::new(DummyImpacts::new())))
    }
}

/// The single dummy impact [`DummyImpactsSource`] reports: the `new Impacts()`
/// built per call, whose `impactBuffer` holds `(Integer.MAX_VALUE, 1)`
/// (`SloppyPhraseMatcher.java:111-131`), the constant
/// [`DUMMY_IMPACT`](crate::index::postings_enum::DUMMY_IMPACT).
#[derive(Debug, Default, Clone, Copy)]
struct DummyImpacts;

impl DummyImpacts {
    fn new() -> Self {
        Self
    }
}

impl Impacts for DummyImpacts {
    fn num_levels(&self) -> i32 {
        1
    }

    fn doc_id_up_to(&self, _level: i32) -> i32 {
        NO_MORE_DOCS
    }

    fn get_impacts(&mut self, _level: i32) -> &FreqAndNormBuffer {
        &crate::index::postings_enum::DUMMY_IMPACT
    }
}

/// A copy of every level of an [`Impacts`] view.
///
/// **This has no counterpart in Lucene 10.5.0.** It exists only for the
/// [`SharedPostings`] divergence: a view the shared enum builds per call
/// (Java's `new Impacts()`, such as `SlowImpactsEnum`'s) may borrow the enum,
/// and cannot outlive the `RefCell` borrow that reached it, so
/// [`SharedPostings`] answers with a copy of what it reports. The enum is not
/// advanced while the copy is read (Java's contract for the view), so every
/// answer is the one the view would give.
struct CopiedImpacts {
    doc_id_up_to: Vec<i32>,
    levels: Vec<FreqAndNormBuffer>,
}

impl CopiedImpacts {
    fn copy_of(view: &mut dyn Impacts) -> Self {
        let num_levels = view.num_levels().max(0) as usize;
        let mut doc_id_up_to = Vec::with_capacity(num_levels);
        let mut levels = Vec::with_capacity(num_levels);
        for level in 0..num_levels as i32 {
            doc_id_up_to.push(view.doc_id_up_to(level));
            let impacts = view.get_impacts(level);
            let mut copy = FreqAndNormBuffer::new();
            copy.grow_no_copy(impacts.size);
            copy.freqs[..impacts.size].copy_from_slice(&impacts.freqs[..impacts.size]);
            copy.norms[..impacts.size].copy_from_slice(&impacts.norms[..impacts.size]);
            copy.size = impacts.size;
            levels.push(copy);
        }
        Self {
            doc_id_up_to,
            levels,
        }
    }
}

impl Impacts for CopiedImpacts {
    fn num_levels(&self) -> i32 {
        self.levels.len() as i32
    }

    fn doc_id_up_to(&self, level: i32) -> i32 {
        self.doc_id_up_to[level as usize]
    }

    fn get_impacts(&mut self, level: i32) -> &FreqAndNormBuffer {
        &self.levels[level as usize]
    }
}
