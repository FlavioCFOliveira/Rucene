//! Impact-driven skipping, ported from
//! `org.apache.lucene.search.ImpactsDISI`.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::index::ImpactsSource;
use crate::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use crate::search::max_score_cache::MaxScoreCache;

/// A [`DocIdSetIterator`] that skips non-competitive documents thanks to the
/// indexed impacts.
///
/// Equivalent to the `final class org.apache.lucene.search.ImpactsDISI`, which
/// extends `FilterDocIdSetIterator`. Call
/// [`set_min_competitive_score`](Self::set_min_competitive_score) to give this
/// iterator the ability to skip low-scoring documents.
///
/// **Divergence from Lucene 10.5.0.** Java takes the iterator and a
/// [`MaxScoreCache`] built from the *same* `ImpactsEnum`, holding it twice.
/// Rust forbids that aliasing, so this type owns the source once and hands it to
/// the cache on every call. It is generic over the concrete type because,
/// below this crate's minimum supported Rust version of 1.86, a
/// `dyn ImpactsEnum` cannot be coerced to the `dyn ImpactsSource` the cache
/// takes.
///
/// The bound is `DocIdSetIterator + ImpactsSource` rather than `ImpactsEnum`,
/// because Java's constructor takes the iterator and the cache separately and
/// the two are not always the same object: `TermScorer` passes one
/// `ImpactsEnum` for both, while `ExactPhraseMatcher` passes a conjunction of
/// postings as the iterator and a *merged* impacts source to the cache. A type
/// that is both is what this port needs in either case.
pub struct ImpactsDISI<I: DocIdSetIterator + ImpactsSource> {
    inner: I,
    max_score_cache: MaxScoreCache,
    min_competitive_score: f32,
    up_to: i32,
    max_score: f32,
}

impl<I: DocIdSetIterator + ImpactsSource> std::fmt::Debug for ImpactsDISI<I> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImpactsDISI")
            .field("min_competitive_score", &self.min_competitive_score)
            .field("up_to", &self.up_to)
            .field("max_score", &self.max_score)
            .finish_non_exhaustive()
    }
}

impl<I: DocIdSetIterator + ImpactsSource> ImpactsDISI<I> {
    /// Wraps an impacts enum and the cache of maximum scores computed from it.
    ///
    /// Equivalent to `new ImpactsDISI(DocIdSetIterator, MaxScoreCache)`.
    pub fn new(inner: I, max_score_cache: MaxScoreCache) -> Self {
        Self {
            inner,
            max_score_cache,
            min_competitive_score: 0.0,
            up_to: NO_MORE_DOCS,
            max_score: f32::MAX,
        }
    }

    /// Returns the cache of maximum scores.
    ///
    /// Equivalent to `ImpactsDISI.getMaxScoreCache()`.
    pub fn get_max_score_cache(&mut self) -> &mut MaxScoreCache {
        &mut self.max_score_cache
    }

    /// Returns the wrapped impacts enum.
    ///
    /// Equivalent to reading the `protected final DocIdSetIterator in` field of
    /// `FilterDocIdSetIterator`.
    pub fn inner(&mut self) -> &mut I {
        &mut self.inner
    }

    /// Returns the wrapped impacts source for inspection.
    ///
    /// The shared-borrow sibling of [`inner`](Self::inner), needed wherever the
    /// port declares an accessor on `&self` — such as
    /// [`TwoPhaseIterator::approximation_ref`](crate::search::TwoPhaseIterator::approximation_ref).
    pub fn inner_ref(&self) -> &I {
        &self.inner
    }

    /// Returns the wrapped impacts enum and the cache of maximum scores at the
    /// same time.
    ///
    /// **Divergence from Lucene 10.5.0.** Java's `TermScorer` holds its own
    /// reference to the very `MaxScoreCache` it handed to `ImpactsDISI`, and
    /// answers `advanceShallow` and `getMaxScore` through it. This port had to
    /// move the cache inside `ImpactsDISI`, because the cache and the iterator
    /// need the same `ImpactsEnum`; a scorer therefore reaches both halves
    /// through this accessor, which borrows the two disjoint fields at once.
    pub fn split_mut(&mut self) -> (&mut I, &mut MaxScoreCache) {
        (&mut self.inner, &mut self.max_score_cache)
    }

    /// Sets the minimum competitive score.
    ///
    /// Equivalent to `ImpactsDISI.setMinCompetitiveScore(float)`; see
    /// [`Scorable::set_min_competitive_score`](crate::search::Scorable::set_min_competitive_score).
    pub fn set_min_competitive_score(&mut self, min_competitive_score: f32) {
        debug_assert!(min_competitive_score >= self.min_competitive_score);
        if min_competitive_score > self.min_competitive_score {
            self.min_competitive_score = min_competitive_score;
            // force upTo and maxScore to be recomputed so that we will skip
            // documents if the current block of documents is not competitive -
            // only if the min competitive score actually increased
            self.up_to = -1;
        }
    }

    /// Equivalent to the private `ImpactsDISI.advanceTarget(int)`.
    fn advance_target(&mut self, mut target: i32) -> Result<i32> {
        if target <= self.up_to {
            // we are still in the current block, which is considered
            // competitive according to impacts, no skipping
            return Ok(target);
        }

        self.up_to = self
            .max_score_cache
            .advance_shallow(&mut self.inner, target)?;
        self.max_score = self
            .max_score_cache
            .get_max_score_for_level_zero(&mut self.inner)?;

        loop {
            debug_assert!(self.up_to >= target);

            if self.max_score >= self.min_competitive_score {
                return Ok(target);
            }

            if self.up_to == NO_MORE_DOCS {
                return Ok(NO_MORE_DOCS);
            }

            let skip_up_to = self
                .max_score_cache
                .get_skip_up_to(&mut self.inner, self.min_competitive_score)?;
            if skip_up_to == -1 {
                // no further skipping
                target = self.up_to + 1;
            } else if skip_up_to == NO_MORE_DOCS {
                return Ok(NO_MORE_DOCS);
            } else {
                target = skip_up_to + 1;
            }
            self.up_to = self
                .max_score_cache
                .advance_shallow(&mut self.inner, target)?;
            self.max_score = self
                .max_score_cache
                .get_max_score_for_level_zero(&mut self.inner)?;
        }
    }

    /// If the current doc is not competitive, moves to a competitive one.
    ///
    /// Equivalent to the package-private `ImpactsDISI.ensureCompetitive()`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while advancing.
    pub fn ensure_competitive(&mut self) -> Result<()> {
        let doc = self.inner.doc_id();
        let advance_target = self.advance_target(doc)?;
        if advance_target != doc {
            self.inner.advance(advance_target)?;
        }
        Ok(())
    }
}

impl<I: DocIdSetIterator + ImpactsSource> DocIdSetIterator for ImpactsDISI<I> {
    fn doc_id(&self) -> i32 {
        self.inner.doc_id()
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let target = self.advance_target(target)?;
        self.inner.advance(target)
    }

    fn next_doc(&mut self) -> Result<i32> {
        if self.inner.doc_id() < self.up_to {
            return self.inner.next_doc();
        }
        let target = self.inner.doc_id() + 1;
        DocIdSetIterator::advance(self, target)
    }

    fn cost(&self) -> i64 {
        self.inner.cost()
    }
}
