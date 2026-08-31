//! Running several collectors in one search, ported from
//! `org.apache.lucene.search.MultiCollector`.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::LeafReaderContext;
use crate::search::collection_terminated_exception::{CollectionError, CollectionResult};
use crate::search::collector::{Collector, LeafCollector};
use crate::search::doc_id_stream::DocIdStream;
use crate::search::scorable::{ChildScorable, FilterScorable, Scorable};
use crate::search::score_caching_wrapping_scorer::ScoreCachingWrappingScorer;
use crate::search::score_mode::ScoreMode;
use crate::search::weight::Weight;

/// A [`Collector`] which allows running a search with several collectors.
///
/// Equivalent to `org.apache.lucene.search.MultiCollector`. Use
/// [`wrap`](MultiCollector::wrap), which filters out the absent collectors and
/// returns the single remaining one unchanged when there is only one.
///
/// **NOTE:** when mixing collectors that want to skip low-scoring hits
/// ([`ScoreMode::TOP_SCORES`]) with ones that require seeing all hits — mixing
/// [`TopScoreDocCollector`](crate::search::TopScoreDocCollector) and
/// [`TotalHitCountCollector`](crate::search::TotalHitCountCollector), for
/// instance — it should be faster to run the query twice, once per collector,
/// than to use this wrapper on a single search.
///
/// **Divergence from Lucene 10.5.0.** Java's `MultiCollector` always holds
/// `Collector[]`, that is, erased collectors. This port makes the element type
/// a parameter, defaulting to `Box<dyn Collector>` so that
/// [`wrap`](MultiCollector::wrap) reads exactly like Java's;
/// [`MultiCollectorManager`](crate::search::MultiCollectorManager) instantiates
/// it over a concrete element type instead, because it has to recover the
/// sub-collectors by value in `reduce` and Rust cannot downcast a
/// `Box<dyn Collector>`.
pub struct MultiCollector<C: Collector = Box<dyn Collector>> {
    cache_scores: bool,
    collectors: Vec<C>,
}

impl<C: Collector> std::fmt::Debug for MultiCollector<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MultiCollector")
            .field("cache_scores", &self.cache_scores)
            .field("collectors", &self.collectors.len())
            .finish()
    }
}

impl MultiCollector<Box<dyn Collector>> {
    /// Wraps a list of collectors with a [`MultiCollector`].
    ///
    /// Equivalent to `MultiCollector.wrap(Iterable<? extends Collector>)`,
    /// which:
    ///
    /// * filters out the absent collectors, so they are not used at search
    ///   time — Java's `null`s become [`None`] here;
    /// * returns the single remaining collector when the input holds exactly
    ///   one;
    /// * otherwise returns a [`MultiCollector`] over the remaining ones.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the input is empty or
    /// every entry is absent, which is the `IllegalArgumentException` Java
    /// throws.
    pub fn wrap(
        collectors: impl IntoIterator<Item = Option<Box<dyn Collector>>>,
    ) -> Result<Box<dyn Collector>> {
        let mut present: Vec<Box<dyn Collector>> = collectors.into_iter().flatten().collect();
        match present.len() {
            0 => Err(LuceneError::IllegalArgument(
                "At least 1 collector must not be null".to_string(),
            )),
            // only 1 Collector - return it.
            1 => Ok(present
                .pop()
                .expect("INVARIANT: the vector was just observed to hold one element")),
            _ => Ok(Box::new(MultiCollector::new(present))),
        }
    }
}

impl<C: Collector> MultiCollector<C> {
    /// Wraps the given collectors, unconditionally.
    ///
    /// Equivalent to the private `MultiCollector(Collector...)` constructor,
    /// which [`wrap`](MultiCollector::wrap) calls once it has filtered the
    /// input. It is public here because
    /// [`MultiCollectorManager`](crate::search::MultiCollectorManager)
    /// instantiates this type over its own element type.
    pub fn new(collectors: Vec<C>) -> Self {
        let num_needs_scores = collectors
            .iter()
            .filter(|collector| collector.score_mode().needs_scores())
            .count();
        Self {
            cache_scores: num_needs_scores >= 2,
            collectors,
        }
    }

    /// Provides access to the wrapped collectors, for advanced use cases.
    ///
    /// Equivalent to `MultiCollector.getCollectors()`.
    pub fn get_collectors(&self) -> &[C] {
        &self.collectors
    }

    /// Unwraps this collector, returning the collectors it was built from.
    ///
    /// **Divergence from Lucene 10.5.0.** Java's `getCollectors()` hands out
    /// live references that `MultiCollectorManager.reduce` then passes to the
    /// sub-managers. Rust's `reduce` consumes its collectors, so the wrapper
    /// has to give them up by value.
    pub fn into_collectors(self) -> Vec<C> {
        self.collectors
    }
}

impl<C: Collector> Collector for MultiCollector<C> {
    fn score_mode(&self) -> ScoreMode {
        let mut score_mode: Option<ScoreMode> = None;
        for collector in &self.collectors {
            match score_mode {
                None => score_mode = Some(collector.score_mode()),
                Some(current) if current != collector.score_mode() => {
                    // If score modes disagree, we don't try to be smart and
                    // just use one of the COMPLETE score modes depending on
                    // whether scores are needed or not.
                    score_mode = Some(
                        if current.needs_scores() || collector.score_mode().needs_scores() {
                            ScoreMode::COMPLETE
                        } else {
                            ScoreMode::COMPLETE_NO_SCORES
                        },
                    );
                }
                Some(_) => {}
            }
        }
        score_mode.expect("INVARIANT: a MultiCollector is only built over one or more collectors")
    }

    fn get_leaf_collector<'a>(
        &'a mut self,
        context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        // Java reads `scoreMode()` after building the leaf collectors; it is a
        // pure function of the collectors' configuration, so it is read here
        // instead, where nothing is borrowed yet.
        let overall_score_mode = self.score_mode();
        let cache_scores = self.cache_scores;

        let mut leaf_collectors: Vec<Box<dyn LeafCollector + 'a>> =
            Vec::with_capacity(self.collectors.len());
        let mut leaf_score_mode: Option<ScoreMode> = None;
        for collector in self.collectors.iter_mut() {
            let collector_score_mode = collector.score_mode();
            let leaf_collector = match collector.get_leaf_collector(context) {
                Ok(leaf_collector) => leaf_collector,
                // this leaf collector does not need this segment
                Err(CollectionError::CollectionTerminated) => continue,
                Err(error) => return Err(error),
            };
            leaf_score_mode = Some(match leaf_score_mode {
                None => collector_score_mode,
                Some(current) if current != collector_score_mode => ScoreMode::COMPLETE,
                Some(current) => current,
            });
            leaf_collectors.push(leaf_collector);
        }

        if leaf_collectors.is_empty() {
            return Err(CollectionError::CollectionTerminated);
        }

        // Wraps a single leaf collector that wants to skip low-scoring hits
        // (ScoreMode.TOP_SCORES) but the global score mode doesn't allow it.
        if leaf_collectors.len() == 1
            && (overall_score_mode == ScoreMode::TOP_SCORES
                || leaf_score_mode != Some(ScoreMode::TOP_SCORES))
        {
            return Ok(leaf_collectors
                .pop()
                .expect("INVARIANT: the vector was just observed to hold one element"));
        }

        let collector =
            MultiLeafCollector::new(leaf_collectors, overall_score_mode == ScoreMode::TOP_SCORES);
        if cache_scores {
            Ok(Box::new(ScoreCachingWrappingScorer::wrap(collector)))
        } else {
            Ok(Box::new(collector))
        }
    }

    fn set_weight(&mut self, weight: Arc<dyn Weight>) {
        for collector in &mut self.collectors {
            collector.set_weight(Arc::clone(&weight));
        }
    }
}

/// The leaf collector a [`MultiCollector`] hands out.
///
/// Equivalent to the private `MultiCollector.MultiLeafCollector`.
///
/// **Divergence from Lucene 10.5.0.** Java wraps the scorable once, in
/// `setScorer`, because every sub-collector stores it. This port passes the
/// scorable to each collection call instead — see the
/// [collector module documentation](crate::search::collector) — so the wrapper
/// is rebuilt around the scorable of every call. The wrapper is the same one
/// Java installs, and its effect on
/// [`Scorable::set_min_competitive_score`] is unchanged.
pub struct MultiLeafCollector<'a> {
    /// The sub collectors; an entry becomes [`None`] once it has terminated,
    /// which is Java's `collectors[i] = null`.
    collectors: Vec<Option<Box<dyn LeafCollector + 'a>>>,
    min_scores: Vec<f32>,
    skip_non_competitive_scores: bool,
}

impl std::fmt::Debug for MultiLeafCollector<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MultiLeafCollector")
            .field("collectors", &self.collectors.len())
            .field(
                "skip_non_competitive_scores",
                &self.skip_non_competitive_scores,
            )
            .finish()
    }
}

impl<'a> MultiLeafCollector<'a> {
    /// Wraps the given leaf collectors.
    ///
    /// Equivalent to the private
    /// `MultiLeafCollector(List<LeafCollector>, boolean)`.
    fn new(collectors: Vec<Box<dyn LeafCollector + 'a>>, skip_non_competitive: bool) -> Self {
        let min_scores = if skip_non_competitive {
            vec![0.0; collectors.len()]
        } else {
            Vec::new()
        };
        Self {
            collectors: collectors.into_iter().map(Some).collect(),
            min_scores,
            skip_non_competitive_scores: skip_non_competitive,
        }
    }

    /// Equivalent to the private
    /// `MultiLeafCollector.allCollectorsTerminated()`.
    fn all_collectors_terminated(&self) -> bool {
        self.collectors.iter().all(Option::is_none)
    }

    /// Runs `action` on every live sub collector, terminating and dropping the
    /// ones that signal early termination.
    ///
    /// Equivalent to the `try`/`catch CollectionTerminatedException` body Java
    /// repeats in `collect(int)` and `collectRange(int, int)`.
    fn for_each_live<F>(&mut self, mut action: F) -> CollectionResult<()>
    where
        F: FnMut(&mut dyn LeafCollector, usize, &mut [f32], bool) -> CollectionResult<()>,
    {
        for i in 0..self.collectors.len() {
            let Self {
                collectors,
                min_scores,
                skip_non_competitive_scores,
            } = self;
            let Some(collector) = collectors[i].as_mut() else {
                continue;
            };
            let outcome = action(
                &mut **collector,
                i,
                min_scores,
                *skip_non_competitive_scores,
            );
            match outcome {
                Ok(()) => {}
                Err(CollectionError::CollectionTerminated) => {
                    let collector = self.collectors[i]
                        .as_mut()
                        .expect("INVARIANT: the entry was just observed to be present");
                    collector.finish()?;
                    self.collectors[i] = None;
                    if self.all_collectors_terminated() {
                        return Err(CollectionError::CollectionTerminated);
                    }
                }
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
}

impl LeafCollector for MultiLeafCollector<'_> {
    fn set_scorer(&mut self, scorer: &mut dyn Scorable) -> Result<()> {
        let Self {
            collectors,
            min_scores,
            skip_non_competitive_scores,
        } = self;
        if *skip_non_competitive_scores {
            for (i, entry) in collectors.iter_mut().enumerate() {
                if let Some(collector) = entry.as_mut() {
                    let mut wrapper = MinCompetitiveScoreAwareScorable {
                        inner: scorer,
                        idx: i,
                        min_scores,
                    };
                    collector.set_scorer(&mut wrapper)?;
                }
            }
        } else {
            for collector in collectors.iter_mut().flatten() {
                // Ignore calls to setMinCompetitiveScore so that if we wrap two
                // collectors and one of them wants to skip low-scoring hits,
                // then the other collector still sees all hits. `FilterScorable`
                // does not override it, so it inherits `Scorable`'s no-op —
                // exactly as in Java.
                let mut wrapper = FilterScorable::new(scorer);
                collector.set_scorer(&mut wrapper)?;
            }
        }
        Ok(())
    }

    // NOTE: not propagating collect(DocIdStream) since DocIdStreams may only be
    // consumed once; the inherited default replays it document by document.
    fn collect(&mut self, doc: i32, scorer: &mut dyn Scorable) -> CollectionResult<()> {
        self.for_each_live(|collector, idx, min_scores, skip| {
            if skip {
                let mut wrapper = MinCompetitiveScoreAwareScorable {
                    inner: scorer,
                    idx,
                    min_scores,
                };
                collector.collect(doc, &mut wrapper)
            } else {
                let mut wrapper = FilterScorable::new(scorer);
                collector.collect(doc, &mut wrapper)
            }
        })
    }

    fn collect_range(
        &mut self,
        min: i32,
        max: i32,
        scorer: &mut dyn Scorable,
    ) -> CollectionResult<()> {
        self.for_each_live(|collector, idx, min_scores, skip| {
            if skip {
                let mut wrapper = MinCompetitiveScoreAwareScorable {
                    inner: scorer,
                    idx,
                    min_scores,
                };
                collector.collect_range(min, max, &mut wrapper)
            } else {
                let mut wrapper = FilterScorable::new(scorer);
                collector.collect_range(min, max, &mut wrapper)
            }
        })
    }

    fn collect_stream(
        &mut self,
        stream: &mut dyn DocIdStream,
        scorer: &mut dyn Scorable,
    ) -> CollectionResult<()> {
        // Reproduces `LeafCollector`'s default, which Java's MultiLeafCollector
        // does not override: a doc-ID stream may only be consumed once, so it is
        // replayed document by document instead of being handed to the sub
        // collectors.
        let mut consumer = |doc: i32| self.collect(doc, &mut *scorer);
        stream.for_each(&mut consumer)
    }

    fn finish(&mut self) -> Result<()> {
        for collector in self.collectors.iter_mut().flatten() {
            collector.finish()?;
        }
        Ok(())
    }
}

/// A [`Scorable`] that only propagates a minimum competitive score once every
/// sub collector agrees on one.
///
/// Equivalent to the static `MultiCollector.MinCompetitiveScoreAwareScorable`,
/// which extends `FilterScorable`.
pub struct MinCompetitiveScoreAwareScorable<'a> {
    inner: &'a mut dyn Scorable,
    idx: usize,
    min_scores: &'a mut [f32],
}

impl std::fmt::Debug for MinCompetitiveScoreAwareScorable<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MinCompetitiveScoreAwareScorable")
            .field("idx", &self.idx)
            .finish_non_exhaustive()
    }
}

impl MinCompetitiveScoreAwareScorable<'_> {
    /// Equivalent to the private
    /// `MinCompetitiveScoreAwareScorable.minScore()`.
    fn min_score(&self) -> f32 {
        let mut min = f32::MAX;
        for score in self.min_scores.iter() {
            if *score < min {
                min = *score;
            }
        }
        min
    }
}

impl Scorable for MinCompetitiveScoreAwareScorable<'_> {
    fn score(&mut self) -> Result<f32> {
        self.inner.score()
    }

    fn set_min_competitive_score(&mut self, min_score: f32) -> Result<()> {
        if min_score > self.min_scores[self.idx] {
            self.min_scores[self.idx] = min_score;
            let min = self.min_score();
            self.inner.set_min_competitive_score(min)?;
        }
        Ok(())
    }

    fn children(&mut self) -> Result<Vec<ChildScorable<'_>>> {
        // Inherited from `FilterScorable`.
        Ok(vec![ChildScorable::new(&mut *self.inner, "FILTER")])
    }
}
