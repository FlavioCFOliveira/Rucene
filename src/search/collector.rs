//! Collectors, ported from `org.apache.lucene.search.Collector`,
//! `LeafCollector`, `CollectorManager`, `SimpleCollector`, `FilterCollector`
//! and `FilterLeafCollector`.
//!
//! # Adaptation: where the `Scorable` lives
//!
//! In Java a [`LeafCollector`] keeps the [`Scorable`] handed to
//! `setScorer(Scorable)` in a field and reads it back from `collect(int)`. That
//! is an alias: the bulk scorer driving iteration and the collector reading
//! scores both hold the same live object, and both mutate it.
//!
//! Rust forbids that aliasing, so the scorable is *passed* to every collection
//! call instead of being stored: [`LeafCollector::set_scorer`] still exists and
//! is still called once before collection, exactly where Java calls it — a
//! collector that reacts to the scorer at that point (for instance by calling
//! [`Scorable::set_min_competitive_score`]) behaves identically — but
//! [`LeafCollector::collect`] takes the scorable as a second argument. Nothing
//! else about the contract changes: the scorable is the same object Java would
//! have stored, positioned on the document being collected.
//!
//! [`Scorable`]: crate::search::Scorable

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::error::Result;
use crate::index::LeafReaderContext;
use crate::search::collection_terminated_exception::CollectionResult;
use crate::search::doc_id_set_iterator::DocIdSetIterator;
use crate::search::doc_id_stream::{DocIdStream, RangeDocIdStream};
use crate::search::scorable::Scorable;
use crate::search::score_mode::ScoreMode;
use crate::search::weight::Weight;

/// Gathers raw results from a search, implementing sorting, custom result
/// filtering, collation and the like.
///
/// Equivalent to `org.apache.lucene.search.Collector`.
pub trait Collector {
    /// Creates a new [`LeafCollector`] to collect the given context.
    ///
    /// Equivalent to `Collector.getLeafCollector(LeafReaderContext)`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java's leaf collector is usually an
    /// inner class holding a live reference to its parent collector, which
    /// outlives the call. Here it borrows the parent for as long as it lives,
    /// which is what Rust requires to express the same object graph and is
    /// exactly how [`IndexSearcher`](crate::search::IndexSearcher) uses it: one
    /// leaf collector at a time, dropped before the next leaf.
    ///
    /// # Errors
    ///
    /// Returns
    /// [`CollectionTerminated`](crate::search::CollectionError::CollectionTerminated)
    /// when there is no document of interest in this reader context, so that
    /// the searcher skips the leaf; propagates any I/O error otherwise.
    fn get_leaf_collector<'a>(
        &'a mut self,
        context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>>;

    /// Indicates what features are required from the scorer.
    ///
    /// Equivalent to `Collector.scoreMode()`.
    fn score_mode(&self) -> ScoreMode;

    /// Sets the [`Weight`] that will be used to produce scorers feeding the
    /// [`LeafCollector`]s.
    ///
    /// Equivalent to `Collector.setWeight(Weight)`, a no-op by default. This is
    /// typically useful to have access to [`Weight::count`] from
    /// [`Collector::get_leaf_collector`].
    fn set_weight(&mut self, _weight: Arc<dyn Weight>) {}
}

/// Decouples the score from the collected doc: the score computation is skipped
/// entirely if it is not needed.
///
/// Equivalent to `org.apache.lucene.search.LeafCollector`. See the module
/// documentation for how the [`Scorable`] reaches the collection calls.
///
/// The doc passed to [`collect`](Self::collect) is relative to the current
/// reader. A collector that needs to resolve it into the doc ID space of the
/// top-level reader must re-base it by recording the doc base of the
/// [`LeafReaderContext`] passed to
/// [`Collector::get_leaf_collector`].
pub trait LeafCollector {
    /// Called before successive calls to [`collect`](Self::collect).
    ///
    /// Equivalent to `LeafCollector.setScorer(Scorable)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while preparing for collection.
    fn set_scorer(&mut self, scorer: &mut dyn Scorable) -> Result<()>;

    /// Called once for every document matching a query, with the unbased
    /// document number.
    ///
    /// Equivalent to `LeafCollector.collect(int)`, plus the scorable that Java
    /// would have stored in [`set_scorer`](Self::set_scorer).
    ///
    /// This is called in an inner search loop; for good search performance
    /// implementations of this method should not read stored fields on every
    /// hit, which can slow searches by an order of magnitude or more.
    ///
    /// # Errors
    ///
    /// Returns
    /// [`CollectionTerminated`](crate::search::CollectionError::CollectionTerminated)
    /// to end collection of the current leaf early; propagates any I/O error
    /// otherwise.
    fn collect(&mut self, doc: i32, scorer: &mut dyn Scorable) -> CollectionResult<()>;

    /// Collects a range of doc IDs, between `min` inclusive and `max`
    /// exclusive. `max` is guaranteed to be greater than `min`.
    ///
    /// Equivalent to `LeafCollector.collectRange(int, int)`, whose default
    /// delegates to [`collect_stream`](Self::collect_stream) over a range
    /// stream. Overriding it is typically useful to take advantage of
    /// pre-aggregated data exposed in a
    /// [`DocValuesSkipper`](crate::index::DocValuesSkipper).
    ///
    /// The position of `scorer` is undefined within this method. Overrides must
    /// not call [`Scorable::score`], and if the scorable is a
    /// [`Scorer`](crate::search::Scorer) must not assume its doc ID corresponds
    /// to any document being collected. Use [`collect`](Self::collect) if
    /// per-document scores are needed.
    ///
    /// # Errors
    ///
    /// As [`collect`](Self::collect).
    fn collect_range(
        &mut self,
        min: i32,
        max: i32,
        scorer: &mut dyn Scorable,
    ) -> CollectionResult<()> {
        let mut stream = RangeDocIdStream::new(min, max)?;
        self.collect_stream(&mut stream, scorer)
    }

    /// Bulk-collects doc IDs.
    ///
    /// Equivalent to `LeafCollector.collect(DocIdStream)`, whose default calls
    /// `stream.forEach(this::collect)`. Rust has no overloading, so the name
    /// carries a `_stream` suffix.
    ///
    /// The provided stream may be reused across calls and should be consumed
    /// immediately. It typically only holds a small subset of the query
    /// matches, so this method may be called multiple times per segment. As
    /// with [`collect`](Self::collect), doc IDs are collected in order, and
    /// callers may freely mix calls to the two methods.
    ///
    /// The position of `scorer` is undefined within this method; see
    /// [`collect_range`](Self::collect_range).
    ///
    /// # Errors
    ///
    /// As [`collect`](Self::collect).
    fn collect_stream(
        &mut self,
        stream: &mut dyn DocIdStream,
        scorer: &mut dyn Scorable,
    ) -> CollectionResult<()> {
        let mut consumer = |doc: i32| self.collect(doc, &mut *scorer);
        stream.for_each(&mut consumer)
    }

    /// Optionally returns an iterator over competitive documents.
    ///
    /// Equivalent to `LeafCollector.competitiveIterator()`, which returns
    /// `null` by default — interpreted as "this collector provides no
    /// competitive iterator". Collectors should delegate this method to their
    /// comparators when those provide skipping over non-competitive docs.
    ///
    /// **Divergence from Lucene 10.5.0.** Java returns a live view onto the
    /// collector's own state, which the caller then advances while also calling
    /// back into the collector. Rust cannot hand out that alias, so the
    /// iterator is returned by value; a collector whose competitive iterator
    /// must observe its comparators has to share that state explicitly.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while building the iterator.
    fn competitive_iterator(&mut self) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        Ok(None)
    }

    /// Hook called once the leaf associated with this collector has finished
    /// collecting successfully, including when collection was terminated early.
    ///
    /// Equivalent to `LeafCollector.finish()`, which does nothing by default.
    /// It is typically useful to compile data collected on this leaf, for
    /// instance to convert facet counts on leaf ordinals into facet counts on
    /// global ordinals. It is called at most once per leaf collector instance.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while finishing the leaf.
    fn finish(&mut self) -> Result<()> {
        Ok(())
    }
}

impl<T: LeafCollector + ?Sized> LeafCollector for &mut T {
    fn set_scorer(&mut self, scorer: &mut dyn Scorable) -> Result<()> {
        (**self).set_scorer(scorer)
    }

    fn collect(&mut self, doc: i32, scorer: &mut dyn Scorable) -> CollectionResult<()> {
        (**self).collect(doc, scorer)
    }

    fn collect_range(
        &mut self,
        min: i32,
        max: i32,
        scorer: &mut dyn Scorable,
    ) -> CollectionResult<()> {
        (**self).collect_range(min, max, scorer)
    }

    fn collect_stream(
        &mut self,
        stream: &mut dyn DocIdStream,
        scorer: &mut dyn Scorable,
    ) -> CollectionResult<()> {
        (**self).collect_stream(stream, scorer)
    }

    fn competitive_iterator(&mut self) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        (**self).competitive_iterator()
    }

    fn finish(&mut self) -> Result<()> {
        (**self).finish()
    }
}

/// A manager of collectors, used to parallelise the execution of a search
/// request.
///
/// Equivalent to `org.apache.lucene.search.CollectorManager<C extends
/// Collector, T>`; the two Java type parameters become the associated types
/// [`Collector`](CollectorManager::Collector) and
/// [`Output`](CollectorManager::Output).
///
/// Multiple [`LeafCollector`]s may be requested for the same
/// [`LeafReaderContext`] across the different collectors returned by
/// [`new_collector`](Self::new_collector). Any computation that must happen
/// once per segment therefore requires specific handling in the manager,
/// because the collection of an entire segment may be split across threads.
pub trait CollectorManager {
    /// The collector type this manager produces.
    type Collector: Collector;

    /// The type the individual collections reduce to.
    type Output;

    /// Returns a new collector. This must return a different instance on each
    /// call.
    ///
    /// Equivalent to `CollectorManager.newCollector()`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while creating the collector.
    fn new_collector(&self) -> Result<Self::Collector>;

    /// Reduces the results of the individual collectors into a meaningful
    /// result. This must be called after collection has finished on all the
    /// provided collectors.
    ///
    /// Equivalent to `CollectorManager.reduce(Collection<C>)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reducing.
    fn reduce(&self, collectors: Vec<Self::Collector>) -> Result<Self::Output>;
}

/// The behaviour a [`SimpleCollector`] is built from.
///
/// Equivalent to what a Java subclass of
/// `org.apache.lucene.search.SimpleCollector` overrides. Java expresses
/// "collector and leaf collector in one object" by having the abstract class
/// implement both interfaces and return `this` from `getLeafCollector`; Rust
/// has no implementation inheritance, so the collecting half is this trait and
/// [`SimpleCollector`] supplies the `Collector` half around it.
pub trait SimpleCollectorImpl: LeafCollector {
    /// Indicates what features are required from the scorer.
    ///
    /// Equivalent to `Collector.scoreMode()`, which `SimpleCollector` leaves
    /// abstract.
    fn score_mode(&self) -> ScoreMode;

    /// Called before collecting `context`.
    ///
    /// Equivalent to `SimpleCollector.doSetNextReader(LeafReaderContext)`,
    /// which does nothing by default.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while preparing for the leaf.
    fn do_set_next_reader(&mut self, _context: &LeafReaderContext) -> Result<()> {
        Ok(())
    }

    /// Sets the weight that will produce the scorers feeding this collector.
    ///
    /// Equivalent to `Collector.setWeight(Weight)`.
    fn set_weight(&mut self, _weight: Arc<dyn Weight>) {}
}

/// Base [`Collector`] implementation that collects every context with a single
/// leaf collector: itself.
///
/// Equivalent to `org.apache.lucene.search.SimpleCollector`. Supply the
/// collecting behaviour as a [`SimpleCollectorImpl`] and wrap it here.
#[derive(Debug, Clone, Copy, Default)]
pub struct SimpleCollector<T: SimpleCollectorImpl> {
    inner: T,
}

impl<T: SimpleCollectorImpl> SimpleCollector<T> {
    /// Wraps the given collecting behaviour.
    pub fn new(inner: T) -> Self {
        Self { inner }
    }

    /// Returns the wrapped behaviour.
    pub fn inner(&self) -> &T {
        &self.inner
    }

    /// Returns the wrapped behaviour for mutation.
    pub fn inner_mut(&mut self) -> &mut T {
        &mut self.inner
    }

    /// Unwraps this collector, returning the behaviour it was built from.
    pub fn into_inner(self) -> T {
        self.inner
    }
}

impl<T: SimpleCollectorImpl> Collector for SimpleCollector<T> {
    fn get_leaf_collector<'a>(
        &'a mut self,
        context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        self.inner.do_set_next_reader(context)?;
        Ok(Box::new(&mut self.inner))
    }

    fn score_mode(&self) -> ScoreMode {
        self.inner.score_mode()
    }

    fn set_weight(&mut self, weight: Arc<dyn Weight>) {
        self.inner.set_weight(weight);
    }
}

/// [`Collector`] delegator.
///
/// Equivalent to `org.apache.lucene.search.FilterCollector`, which delegates
/// every method to the wrapped collector. Java leaves the class abstract so
/// that a subclass must exist to change something; Rust composition makes the
/// wrapper concrete and a "subclass" is a type that holds one.
#[derive(Debug, Clone, Copy, Default)]
pub struct FilterCollector<C: Collector> {
    /// The wrapped collector.
    ///
    /// Equivalent to the `protected final Collector in` field.
    pub inner: C,
}

impl<C: Collector> FilterCollector<C> {
    /// Wraps the given collector.
    ///
    /// Equivalent to `new FilterCollector(Collector)`.
    pub fn new(inner: C) -> Self {
        Self { inner }
    }

    /// Unwraps this collector.
    pub fn into_inner(self) -> C {
        self.inner
    }
}

impl<C: Collector> Collector for FilterCollector<C> {
    fn get_leaf_collector<'a>(
        &'a mut self,
        context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        self.inner.get_leaf_collector(context)
    }

    fn score_mode(&self) -> ScoreMode {
        self.inner.score_mode()
    }

    fn set_weight(&mut self, weight: Arc<dyn Weight>) {
        self.inner.set_weight(weight);
    }
}

/// [`LeafCollector`] delegator.
///
/// Equivalent to `org.apache.lucene.search.FilterLeafCollector`. Note that Java
/// overrides only `setScorer`, `collect` and `finish`: the bulk collection
/// paths and `competitiveIterator()` keep [`LeafCollector`]'s defaults rather
/// than delegating, and this port reproduces that exactly.
#[derive(Debug, Clone, Copy, Default)]
pub struct FilterLeafCollector<L: LeafCollector> {
    /// The wrapped leaf collector.
    ///
    /// Equivalent to the `protected final LeafCollector in` field.
    pub inner: L,
}

impl<L: LeafCollector> FilterLeafCollector<L> {
    /// Wraps the given leaf collector.
    ///
    /// Equivalent to `new FilterLeafCollector(LeafCollector)`.
    pub fn new(inner: L) -> Self {
        Self { inner }
    }

    /// Unwraps this collector.
    pub fn into_inner(self) -> L {
        self.inner
    }
}

impl<L: LeafCollector> LeafCollector for FilterLeafCollector<L> {
    fn set_scorer(&mut self, scorer: &mut dyn Scorable) -> Result<()> {
        self.inner.set_scorer(scorer)
    }

    fn collect(&mut self, doc: i32, scorer: &mut dyn Scorable) -> CollectionResult<()> {
        self.inner.collect(doc, scorer)
    }

    fn finish(&mut self) -> Result<()> {
        self.inner.finish()
    }
}
