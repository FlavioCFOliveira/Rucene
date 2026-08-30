//! Score caching, ported from
//! `org.apache.lucene.search.ScoreCachingWrappingScorer`.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::search::collection_terminated_exception::CollectionResult;
use crate::search::collector::LeafCollector;
use crate::search::doc_id_set_iterator::DocIdSetIterator;
use crate::search::scorable::{ChildScorable, Scorable};

/// Wraps another scorable and caches the score of the current document.
///
/// Equivalent to `org.apache.lucene.search.ScoreCachingWrappingScorer`.
/// Successive calls to [`Scorable::score`] return the same result without
/// invoking the wrapped scorable again, unless the current document has
/// changed. This is useful because the collector interface does not compute the
/// score of a document unless the collector asks for it, and some collectors
/// need to use the score in several places while holding only a scorable.
pub struct ScoreCachingWrappingScorer<'a> {
    score_is_cached: bool,
    cur_score: f32,
    inner: &'a mut dyn Scorable,
}

impl<'a> ScoreCachingWrappingScorer<'a> {
    /// Creates a new instance by wrapping the given scorable.
    ///
    /// Equivalent to the private
    /// `ScoreCachingWrappingScorer(Scorable)` constructor. It is public here
    /// because the wrapping leaf collector builds one per collected document
    /// rather than once per leaf; see [`wrap`](Self::wrap).
    pub fn new(inner: &'a mut dyn Scorable) -> Self {
        Self {
            score_is_cached: false,
            cur_score: 0.0,
            inner,
        }
    }

    /// Wraps the provided leaf collector so that scores are computed lazily and
    /// cached if accessed multiple times.
    ///
    /// Equivalent to `ScoreCachingWrappingScorer.wrap(LeafCollector)`. Java
    /// returns the collector unchanged when it already is a caching wrapper;
    /// Rust cannot test that without a downcast, and wrapping twice only costs
    /// one redundant cache layer, so this port always wraps.
    pub fn wrap<L: LeafCollector>(collector: L) -> ScoreCachingWrappingLeafCollector<L> {
        ScoreCachingWrappingLeafCollector { inner: collector }
    }
}

impl std::fmt::Debug for ScoreCachingWrappingScorer<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScoreCachingWrappingScorer")
            .field("score_is_cached", &self.score_is_cached)
            .field("cur_score", &self.cur_score)
            .finish_non_exhaustive()
    }
}

impl Scorable for ScoreCachingWrappingScorer<'_> {
    fn score(&mut self) -> Result<f32> {
        if !self.score_is_cached {
            self.cur_score = self.inner.score()?;
            self.score_is_cached = true;
        }
        Ok(self.cur_score)
    }

    fn set_min_competitive_score(&mut self, min_score: f32) -> Result<()> {
        self.inner.set_min_competitive_score(min_score)
    }

    fn children(&mut self) -> Result<Vec<ChildScorable<'_>>> {
        Ok(vec![ChildScorable::new(&mut *self.inner, "CACHED")])
    }
}

/// The leaf collector that installs a [`ScoreCachingWrappingScorer`] around the
/// scorable reaching the wrapped collector.
///
/// Equivalent to the private
/// `ScoreCachingWrappingScorer.ScoreCachingWrappingLeafCollector`, a
/// `FilterLeafCollector` that creates the caching scorer in `setScorer` and
/// invalidates the cache at the start of every `collect`.
///
/// **Divergence from Lucene 10.5.0.** Java keeps one caching scorer for the
/// whole leaf and resets its `scoreIsCached` flag on each `collect`. This port
/// builds a fresh caching scorer per call instead, because the scorable is
/// passed to each call rather than stored — see the
/// [collector module documentation](crate::search::collector). The observable
/// behaviour is identical: the wrapped scorable is asked for the score at most
/// once per collected document.
#[derive(Debug, Clone, Copy, Default)]
pub struct ScoreCachingWrappingLeafCollector<L: LeafCollector> {
    /// The wrapped leaf collector.
    pub inner: L,
}

impl<L: LeafCollector> ScoreCachingWrappingLeafCollector<L> {
    /// Unwraps this collector.
    pub fn into_inner(self) -> L {
        self.inner
    }
}

impl<L: LeafCollector> LeafCollector for ScoreCachingWrappingLeafCollector<L> {
    fn set_scorer(&mut self, scorer: &mut dyn Scorable) -> Result<()> {
        let mut caching = ScoreCachingWrappingScorer::new(scorer);
        self.inner.set_scorer(&mut caching)
    }

    fn collect(&mut self, doc: i32, scorer: &mut dyn Scorable) -> CollectionResult<()> {
        // A fresh caching scorer per document invalidates the cache exactly
        // where Java clears the `scoreIsCached` flag.
        let mut caching = ScoreCachingWrappingScorer::new(scorer);
        self.inner.collect(doc, &mut caching)
    }

    fn competitive_iterator(&mut self) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        self.inner.competitive_iterator()
    }

    fn finish(&mut self) -> Result<()> {
        self.inner.finish()
    }
}
