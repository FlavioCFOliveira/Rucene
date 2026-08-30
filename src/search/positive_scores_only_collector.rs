//! Filtering out non-positive scores, ported from
//! `org.apache.lucene.search.PositiveScoresOnlyCollector`.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::error::Result;
use crate::index::LeafReaderContext;
use crate::search::collection_terminated_exception::CollectionResult;
use crate::search::collector::{Collector, FilterCollector, LeafCollector};
use crate::search::scorable::Scorable;
use crate::search::score_caching_wrapping_scorer::ScoreCachingWrappingScorer;
use crate::search::score_mode::ScoreMode;
use crate::search::weight::Weight;

/// A [`Collector`] implementation which wraps another collector and makes sure
/// only documents with scores greater than zero are collected.
///
/// Equivalent to `org.apache.lucene.search.PositiveScoresOnlyCollector`, which
/// extends `FilterCollector`; Rust has no implementation inheritance, so this
/// port holds a [`FilterCollector`] instead.
#[derive(Debug, Clone, Copy, Default)]
pub struct PositiveScoresOnlyCollector<C: Collector> {
    inner: FilterCollector<C>,
}

impl<C: Collector> PositiveScoresOnlyCollector<C> {
    /// Wraps the given collector.
    ///
    /// Equivalent to `new PositiveScoresOnlyCollector(Collector)`.
    pub fn new(inner: C) -> Self {
        Self {
            inner: FilterCollector::new(inner),
        }
    }

    /// Unwraps this collector.
    ///
    /// Equivalent to reading `FilterCollector`'s `protected final Collector in`
    /// field.
    pub fn into_inner(self) -> C {
        self.inner.into_inner()
    }
}

impl<C: Collector> Collector for PositiveScoresOnlyCollector<C> {
    fn get_leaf_collector<'a>(
        &'a mut self,
        context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        let inner = self.inner.get_leaf_collector(context)?;
        Ok(Box::new(ScoreCachingWrappingScorer::wrap(
            PositiveScoresOnlyLeafCollector { inner },
        )))
    }

    fn score_mode(&self) -> ScoreMode {
        self.inner.score_mode()
    }

    fn set_weight(&mut self, weight: Arc<dyn Weight>) {
        self.inner.set_weight(weight);
    }
}

/// The leaf collector a [`PositiveScoresOnlyCollector`] hands out.
///
/// Equivalent to the anonymous `FilterLeafCollector` that
/// `PositiveScoresOnlyCollector.getLeafCollector` wraps in a
/// [`ScoreCachingWrappingScorer`]. As in Java, the bulk collection paths and
/// `competitiveIterator()` keep [`LeafCollector`]'s defaults, which route
/// through [`collect`](LeafCollector::collect) and therefore apply the same
/// filter.
struct PositiveScoresOnlyLeafCollector<'a> {
    inner: Box<dyn LeafCollector + 'a>,
}

impl LeafCollector for PositiveScoresOnlyLeafCollector<'_> {
    fn set_scorer(&mut self, scorer: &mut dyn Scorable) -> Result<()> {
        self.inner.set_scorer(scorer)
    }

    fn collect(&mut self, doc: i32, scorer: &mut dyn Scorable) -> CollectionResult<()> {
        if scorer.score()? > 0.0 {
            self.inner.collect(doc, scorer)?;
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        self.inner.finish()
    }
}
