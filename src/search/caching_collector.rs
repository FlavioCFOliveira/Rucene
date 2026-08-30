//! Caching and replaying a collection, ported from
//! `org.apache.lucene.search.CachingCollector`.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::LeafReaderContext;
use crate::search::collection_terminated_exception::CollectionResult;
use crate::search::collector::{Collector, LeafCollector, SimpleCollector, SimpleCollectorImpl};
use crate::search::scorable::Scorable;
use crate::search::score_mode::ScoreMode;
use crate::search::weight::Weight;
use crate::util::ArrayUtil;

/// The initial capacity of a leaf's document buffer.
///
/// Equivalent to `CachingCollector.INITIAL_ARRAY_SIZE`.
const INITIAL_ARRAY_SIZE: i32 = 128;

/// The scorable that replays cached scores.
///
/// Equivalent to the private `CachingCollector.CachedScorable`.
#[derive(Debug, Default, Clone, Copy)]
struct CachedScorable {
    score: f32,
}

impl Scorable for CachedScorable {
    fn score(&mut self) -> Result<f32> {
        Ok(self.score)
    }
}

/// The scorable handed to the replay collector when scores were not cached.
///
/// **Divergence from Lucene 10.5.0.** Java replays a no-score cache without
/// ever calling `setScorer`, so the replayed collector's scorer stays `null`
/// and reading a score raises a `NullPointerException`. The port has no null
/// scorable, so it hands over one that reports the misuse instead; the
/// documented contract — "if this instance does not cache scores, then Scorer
/// is not set on `other.setScorer` as well as scores are not replayed" — is
/// unchanged.
#[derive(Debug, Default, Clone, Copy)]
struct NoScorable;

impl Scorable for NoScorable {
    fn score(&mut self) -> Result<f32> {
        Err(LuceneError::IllegalState(
            "this CachingCollector did not cache scores, so the replayed collector must not read them"
                .to_string(),
        ))
    }
}

/// A collector that does nothing.
///
/// Equivalent to the anonymous `SimpleCollector` that
/// `CachingCollector.create(boolean, double)` wraps when no collector is
/// supplied.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoOpCollector;

impl LeafCollector for NoOpCollector {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, _doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        Ok(())
    }
}

impl SimpleCollectorImpl for NoOpCollector {
    fn score_mode(&self) -> ScoreMode {
        ScoreMode::COMPLETE
    }
}

/// The cache a [`CachingCollector`] accumulates.
///
/// Equivalent to the fields Java splits between `CachingCollector`
/// (`cached`), `NoScoreCachingCollector` (`contexts`, `docs`,
/// `maxDocsToCache`) and `ScoreCachingCollector` (`scores`).
struct CachingState {
    cached: bool,
    cache_scores: bool,
    max_docs_to_cache: i32,
    /// The ordinals of the leaves that were collected; `None` once the cache
    /// has been invalidated.
    ///
    /// **Divergence from Lucene 10.5.0.** Java keeps the `LeafReaderContext`
    /// objects themselves and re-uses them in `replay`. This port's
    /// [`Collector::get_leaf_collector`] receives a borrowed context that
    /// cannot outlive the call, and a leaf context is not clonable — cloning
    /// one would mint a new identity — so the ordinal in the top-level leaves
    /// array is recorded instead and [`CachingCollector::replay`] takes those
    /// leaves. The contexts replayed are the very same objects Java would have
    /// stored.
    ords: Option<Vec<i32>>,
    docs: Option<Vec<Vec<i32>>>,
    /// The cached scores, present only for the score-caching variant.
    scores: Option<Vec<Vec<f32>>>,
}

impl CachingState {
    /// Equivalent to `NoScoreCachingCollector.invalidate()`.
    fn invalidate(&mut self) {
        self.max_docs_to_cache = -1;
        self.ords = None;
        self.docs = None;
    }
}

/// Caches all docs, and optionally also scores, coming from a search, and is
/// then able to replay them to another collector.
///
/// Equivalent to the abstract `org.apache.lucene.search.CachingCollector` and
/// its two concrete subclasses, `NoScoreCachingCollector` and
/// `ScoreCachingCollector`. You specify the maximum RAM this collector may use;
/// once collection is done, call [`is_cached`](Self::is_cached). If it returns
/// `true`, [`replay`](Self::replay) can drive a new collector; if it returns
/// `false`, too much RAM was required and the original search must be re-run.
///
/// **NOTE:** this type consumes 4 bytes per collected document, or 8 when
/// scores are cached. If the result set is large this can easily be a very
/// substantial amount of RAM.
///
/// **Divergence from Lucene 10.5.0.** Java models the two variants as a
/// subclass pair that override three methods — `scoreMode()`, the per-leaf
/// buffering and the replay. Rust has no implementation inheritance, so both
/// live in this one type and the variant is the presence of the score cache.
/// The buffering policy, the invalidation thresholds and the replay sequences
/// are unchanged.
pub struct CachingCollector<C: Collector> {
    inner: C,
    state: CachingState,
}

impl<C: Collector> std::fmt::Debug for CachingCollector<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CachingCollector")
            .field("cached", &self.state.cached)
            .field("cache_scores", &self.state.cache_scores)
            .field("max_docs_to_cache", &self.state.max_docs_to_cache)
            .finish_non_exhaustive()
    }
}

/// Converts a RAM budget in megabytes into a maximum number of cached
/// documents.
///
/// Equivalent to the arithmetic of
/// `CachingCollector.create(Collector, boolean, double)`.
fn max_docs_for_ram(cache_scores: bool, max_ram_mb: f64) -> i32 {
    let mut bytes_per_doc = std::mem::size_of::<i32>() as f64;
    if cache_scores {
        bytes_per_doc += std::mem::size_of::<f32>() as f64;
    }
    // Java's `(int)` cast of a double truncates towards zero and saturates;
    // Rust's `as i32` does the same.
    ((max_ram_mb * 1024.0 * 1024.0) / bytes_per_doc) as i32
}

impl CachingCollector<SimpleCollector<NoOpCollector>> {
    /// Creates a caching collector which does not wrap another collector.
    ///
    /// Equivalent to `CachingCollector.create(boolean, double)`. The cached
    /// documents and scores can later be [`replay`](Self::replay)ed.
    pub fn create_standalone(cache_scores: bool, max_ram_mb: f64) -> Self {
        Self::create_with_max_ram(
            SimpleCollector::new(NoOpCollector),
            cache_scores,
            max_ram_mb,
        )
    }
}

impl<C: Collector> CachingCollector<C> {
    /// Creates a caching collector that wraps `other` and caches documents, and
    /// optionally scores, up to the given RAM threshold.
    ///
    /// Equivalent to `CachingCollector.create(Collector, boolean, double)`.
    ///
    /// * `other` — the collector to wrap and delegate calls to;
    /// * `cache_scores` — whether to cache scores in addition to document IDs,
    ///   which increases the RAM consumed per document;
    /// * `max_ram_mb` — the maximum RAM, in megabytes, to consume for caching.
    ///   If the collector exceeds the threshold, nothing is cached.
    pub fn create_with_max_ram(other: C, cache_scores: bool, max_ram_mb: f64) -> Self {
        Self::create(
            other,
            cache_scores,
            max_docs_for_ram(cache_scores, max_ram_mb),
        )
    }

    /// Creates a caching collector that wraps `other` and caches documents, and
    /// optionally scores, up to the given document threshold.
    ///
    /// Equivalent to `CachingCollector.create(Collector, boolean, int)`.
    pub fn create(other: C, cache_scores: bool, max_docs_to_cache: i32) -> Self {
        Self {
            inner: other,
            state: CachingState {
                cached: true,
                cache_scores,
                max_docs_to_cache,
                ords: Some(Vec::new()),
                docs: Some(Vec::new()),
                scores: if cache_scores { Some(Vec::new()) } else { None },
            },
        }
    }

    /// Returns `true` if this collector is able to replay collection.
    ///
    /// Equivalent to the `final CachingCollector.isCached()`.
    pub fn is_cached(&self) -> bool {
        self.state.cached
    }

    /// Unwraps this collector.
    ///
    /// Equivalent to reading `FilterCollector`'s
    /// `protected final Collector in` field.
    pub fn into_inner(self) -> C {
        self.inner
    }

    /// Replays the cached doc IDs, and their scores when they were cached, to
    /// the given collector.
    ///
    /// Equivalent to the abstract `CachingCollector.replay(Collector)`. If this
    /// instance does not cache scores, then no scorer is set on `other` and
    /// scores are not replayed.
    ///
    /// `leaves` must be the top-level leaves of the reader this collector
    /// collected — [`IndexSearcher::get_leaf_contexts`](crate::search::IndexSearcher::get_leaf_contexts)
    /// — because the leaves that were collected are recalled by ordinal; see the
    /// divergence noted on the cache.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when this collector is not cached,
    /// that is, when the RAM limits were too low for the number of documents
    /// and scores to cache; that is the `IllegalStateException` Java throws.
    /// Returns [`LuceneError::IllegalArgument`] when `leaves` does not hold a
    /// leaf that was collected. Propagates whatever the replayed collector
    /// fails with, including its early-termination signal.
    pub fn replay(
        &mut self,
        other: &mut dyn Collector,
        leaves: &[Arc<LeafReaderContext>],
    ) -> CollectionResult<()> {
        if !self.is_cached() {
            return Err(LuceneError::IllegalState(
                "cannot replay: cache was cleared because too much RAM was required".to_string(),
            )
            .into());
        }
        let ords = self
            .state
            .ords
            .as_ref()
            .expect("INVARIANT: a cached CachingCollector has not been invalidated");
        let docs = self
            .state
            .docs
            .as_ref()
            .expect("INVARIANT: a cached CachingCollector has not been invalidated");
        debug_assert_eq!(docs.len(), ords.len());

        for i in 0..ords.len() {
            let ord = ords[i];
            let context = leaves.get(ord as usize).ok_or_else(|| {
                LuceneError::IllegalArgument(format!(
                    "leaf ordinal {ord} was collected but is absent from the leaves given to replay"
                ))
            })?;
            let mut collector = other.get_leaf_collector(context)?;
            match self.state.scores.as_ref() {
                Some(scores) => {
                    // Equivalent to `ScoreCachingCollector.collect(LeafCollector,
                    // int)`, which — unlike the no-score variant — does not call
                    // `finish()`.
                    let cached_docs = &docs[i];
                    let cached_scores = &scores[i];
                    debug_assert_eq!(cached_docs.len(), cached_scores.len());
                    let mut scorer = CachedScorable::default();
                    collector.set_scorer(&mut scorer)?;
                    for j in 0..cached_docs.len() {
                        scorer.score = cached_scores[j];
                        collector.collect(cached_docs[j], &mut scorer)?;
                    }
                }
                None => {
                    // Equivalent to
                    // `NoScoreCachingCollector.collect(LeafCollector, int)`.
                    let mut scorer = NoScorable;
                    for doc in &docs[i] {
                        collector.collect(*doc, &mut scorer)?;
                    }
                    collector.finish()?;
                }
            }
        }
        Ok(())
    }
}

impl<C: Collector> Collector for CachingCollector<C> {
    fn get_leaf_collector<'a>(
        &'a mut self,
        context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        let Self { inner, state } = self;
        let inner = inner.get_leaf_collector(context)?;
        if state.max_docs_to_cache >= 0 {
            if let Some(ords) = state.ords.as_mut() {
                ords.push(context.ord());
            }
            let max_docs_to_cache = state.max_docs_to_cache;
            let cache_scores = state.cache_scores;
            Ok(Box::new(CachingLeafCollector::new(
                inner,
                state,
                max_docs_to_cache,
                cache_scores,
            )))
        } else {
            Ok(inner)
        }
    }

    fn score_mode(&self) -> ScoreMode {
        if self.state.cache_scores {
            // Ensure the scores are collected so they can be replayed, even if
            // the wrapped collector doesn't need them.
            ScoreMode::COMPLETE
        } else {
            // Note: do *not* say the scores are not needed. Just because we
            // aren't caching the score doesn't mean the wrapped collector
            // doesn't need it to do its job.
            self.inner.score_mode()
        }
    }

    fn set_weight(&mut self, weight: Arc<dyn Weight>) {
        self.inner.set_weight(weight);
    }
}

/// The leaf collector a [`CachingCollector`] hands out.
///
/// Equivalent to `CachingCollector.NoScoreCachingLeafCollector` and its
/// subclass `ScoreCachingLeafCollector`, which Java declares as inner classes so
/// that they can write into the outer collector's cache; here that is an
/// explicit borrow.
struct CachingLeafCollector<'a> {
    inner: Box<dyn LeafCollector + 'a>,
    state: &'a mut CachingState,
    max_docs_to_cache: i32,
    /// The buffer of collected doc IDs; `None` once this leaf overflowed.
    docs: Option<Vec<i32>>,
    /// The buffer of collected scores, present only for the score-caching
    /// variant.
    scores: Option<Vec<f32>>,
    doc_count: i32,
}

impl<'a> CachingLeafCollector<'a> {
    /// Equivalent to
    /// `NoScoreCachingLeafCollector(LeafCollector, int, NoScoreCachingCollector)`
    /// and to its score-caching subclass's constructor.
    fn new(
        inner: Box<dyn LeafCollector + 'a>,
        state: &'a mut CachingState,
        max_docs_to_cache: i32,
        cache_scores: bool,
    ) -> Self {
        let capacity = max_docs_to_cache.clamp(0, INITIAL_ARRAY_SIZE) as usize;
        Self {
            inner,
            state,
            max_docs_to_cache,
            docs: Some(vec![0; capacity]),
            scores: if cache_scores {
                Some(vec![0.0; capacity])
            } else {
                None
            },
            doc_count: 0,
        }
    }

    /// Equivalent to `NoScoreCachingLeafCollector.grow(int)` and its override.
    fn grow(&mut self, new_len: usize) {
        if let Some(docs) = self.docs.as_mut() {
            docs.resize(new_len, 0);
        }
        if let Some(scores) = self.scores.as_mut() {
            scores.resize(new_len, 0.0);
        }
    }

    /// Equivalent to `NoScoreCachingLeafCollector.invalidate()` and its
    /// override.
    fn invalidate(&mut self) {
        self.docs = None;
        self.scores = None;
        self.doc_count = -1;
        self.state.cached = false;
    }

    /// Equivalent to `NoScoreCachingLeafCollector.buffer(int)` and its override.
    fn buffer(&mut self, doc: i32, scorer: &mut dyn Scorable) -> Result<()> {
        let index = self.doc_count as usize;
        if let Some(docs) = self.docs.as_mut() {
            docs[index] = doc;
        }
        if self.scores.is_some() {
            let score = scorer.score()?;
            if let Some(scores) = self.scores.as_mut() {
                scores[index] = score;
            }
        }
        Ok(())
    }

    /// Equivalent to `NoScoreCachingLeafCollector.hasCache()`.
    fn has_cache(&self) -> bool {
        self.docs.is_some()
    }

    /// Equivalent to `NoScoreCachingLeafCollector.postCollect()` and its
    /// override.
    fn post_collect(&mut self) {
        let doc_count = self.doc_count.max(0) as usize;
        let cached_docs = self
            .docs
            .as_ref()
            .map(|docs| docs[..doc_count].to_vec())
            .unwrap_or_default();
        self.state.max_docs_to_cache -= cached_docs.len() as i32;
        if let Some(docs) = self.state.docs.as_mut() {
            docs.push(cached_docs);
        }
        if let Some(cached_scores) = self
            .scores
            .as_ref()
            .map(|scores| scores[..doc_count].to_vec())
        {
            if let Some(scores) = self.state.scores.as_mut() {
                scores.push(cached_scores);
            }
        }
    }
}

impl LeafCollector for CachingLeafCollector<'_> {
    fn set_scorer(&mut self, scorer: &mut dyn Scorable) -> Result<()> {
        self.inner.set_scorer(scorer)
    }

    fn collect(&mut self, doc: i32, scorer: &mut dyn Scorable) -> CollectionResult<()> {
        if let Some(capacity) = self.docs.as_ref().map(Vec::len) {
            if self.doc_count as usize >= capacity {
                if self.doc_count >= self.max_docs_to_cache {
                    self.invalidate();
                } else {
                    let new_len = ArrayUtil::oversize(self.doc_count as usize + 1, 4)
                        .min(self.max_docs_to_cache.max(0) as usize);
                    self.grow(new_len);
                }
            }
            if self.docs.is_some() {
                self.buffer(doc, scorer)?;
                self.doc_count += 1;
            }
        }
        self.inner.collect(doc, scorer)
    }

    fn finish(&mut self) -> Result<()> {
        if !self.has_cache() {
            self.state.invalidate();
        } else {
            self.post_collect();
        }
        Ok(())
    }
}
