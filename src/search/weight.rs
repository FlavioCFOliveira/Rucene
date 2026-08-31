//! Query weights, ported from `org.apache.lucene.search.Weight` and
//! `org.apache.lucene.search.FilterWeight`.

#![deny(unsafe_code)]

use std::fmt::Debug;
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::LeafReaderContext;
use crate::search::bulk_scorer::BulkScorer;
use crate::search::matches::{Matches, MatchesUtils};
use crate::search::query::Query;
use crate::search::scorer::Scorer;
use crate::search::scorer_supplier::ScorerSupplier;
use crate::search::segment_cacheable::SegmentCacheable;
use crate::search::similarities::Explanation;

/// Calculates query weights and builds query scorers.
///
/// Equivalent to the abstract class `org.apache.lucene.search.Weight`, which
/// implements [`SegmentCacheable`].
///
/// The purpose of a weight is to ensure searching does not modify a
/// [`Query`], so that a query instance can be reused.
/// [`IndexSearcher`](crate::search::IndexSearcher)-dependent state of the query
/// should reside in the weight, and
/// [`LeafReader`](crate::index::LeafReader)-dependent state should reside in
/// the [`Scorer`].
///
/// A weight is used as follows:
///
/// 1. it is constructed by a top-level query, given an index searcher
///    ([`Query::create_weight`]);
/// 2. a scorer is constructed by [`scorer_supplier`](Self::scorer_supplier).
///
/// Because a weight creates scorers for a given
/// [`LeafReaderContext`], callers must maintain the relationship between the
/// searcher's top-level reader context and the context used to create a scorer.
pub trait Weight: SegmentCacheable + Send + Sync + Debug {
    /// Returns the query this weight concerns.
    ///
    /// Equivalent to the `final Weight.getQuery()` reading the
    /// `protected final Query parentQuery` field.
    fn get_query(&self) -> Arc<dyn Query>;

    /// An explanation of the score computation for the named document.
    ///
    /// Equivalent to `Weight.explain(LeafReaderContext, int)`.
    ///
    /// * `context` — the reader's context to create the explanation for;
    /// * `doc` — the document's ID relative to the given context's reader.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while explaining.
    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation>;

    /// Returns the weight this one caches, when it is the wrapper
    /// [`LRUQueryCache`](crate::search::LRUQueryCache) puts around a weight.
    ///
    /// **Divergence from Lucene 10.5.0.** Java's `LRUQueryCache.doCache` writes
    /// `while (weight instanceof CachingWrapperWeight) weight = ((CachingWrapperWeight) weight).in;`
    /// so that a weight is never wrapped twice. Rust cannot downcast a
    /// `dyn Weight`, so the test is declared as a method whose default answers
    /// `None` — Java's `false` branch — and the caching wrapper overrides it.
    fn unwrap_cached(&self) -> Option<Arc<dyn Weight>> {
        None
    }

    /// Returns a [`ScorerSupplier`], which allows knowing the cost of the
    /// [`Scorer`] before building it, or `None` if no documents will be scored
    /// by this query.
    ///
    /// Equivalent to `Weight.scorerSupplier(LeafReaderContext)`. A supplier for
    /// the same leaf context may be requested multiple times as part of a
    /// single search call.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while preparing the supplier.
    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>>;

    /// Counts the number of live documents that match this weight's query in a
    /// leaf.
    ///
    /// Equivalent to `Weight.count(LeafReaderContext)`, which returns `-1` by
    /// default. `-1` indicates that the count could not be computed in
    /// sub-linear time; specific query classes override it to provide accurate
    /// sub-linear implementations, which is what
    /// [`IndexSearcher::count`](crate::search::IndexSearcher::count) exploits.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while counting.
    fn count(&self, _context: &LeafReaderContext) -> Result<i32> {
        Ok(-1)
    }

    /// Returns the [`Matches`] for a specific document, or `None` if the
    /// document does not match the parent query.
    ///
    /// Equivalent to `Weight.matches(LeafReaderContext, int)`. A query match
    /// that contains no position information — a point or doc-values query, for
    /// instance — returns
    /// [`MatchesUtils::match_with_no_terms`].
    ///
    /// * `context` — the reader's context to create the matches for;
    /// * `doc` — the document's ID relative to the given context's reader.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while positioning the scorer.
    fn matches(&self, context: &LeafReaderContext, doc: i32) -> Result<Option<Arc<dyn Matches>>> {
        let scorer_supplier = self.scorer_supplier(context)?;
        let Some(mut scorer_supplier) = scorer_supplier else {
            return Ok(None);
        };
        let mut scorer = scorer_supplier.get(1)?;
        if scorer.two_phase_iterator().is_some() {
            let two_phase = scorer
                .two_phase_iterator()
                .expect("INVARIANT: the two-phase view was just observed to be present");
            if two_phase.approximation().advance(doc)? != doc {
                return Ok(None);
            }
            let two_phase = scorer
                .two_phase_iterator()
                .expect("INVARIANT: the two-phase view was just observed to be present");
            if !two_phase.matches()? {
                return Ok(None);
            }
        } else if scorer.iterator().advance(doc)? != doc {
            return Ok(None);
        }
        Ok(Some(MatchesUtils::match_with_no_terms()))
    }

    /// Returns a [`Scorer`] which can iterate in order over all matching
    /// documents and assign them a score, or `None` if no documents will be
    /// scored by this query.
    ///
    /// Equivalent to the `final Weight.scorer(LeafReaderContext)`, which
    /// delegates to [`scorer_supplier`](Self::scorer_supplier) with a lead cost
    /// of [`i64::MAX`]. A scorer for the same leaf context may be requested
    /// multiple times as part of a single search call.
    ///
    /// The returned scorer does not have
    /// [`LeafReader::get_live_docs`](crate::index::LeafReader::get_live_docs)
    /// applied; they need to be checked on top.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while building the scorer.
    fn scorer(&self, context: &LeafReaderContext) -> Result<Option<Box<dyn Scorer>>> {
        let scorer_supplier = self.scorer_supplier(context)?;
        match scorer_supplier {
            None => Ok(None),
            Some(mut supplier) => Ok(Some(supplier.get(i64::MAX)?)),
        }
    }

    /// Returns a [`BulkScorer`] for this weight, or `None` if no documents
    /// match.
    ///
    /// Equivalent to the `final Weight.bulkScorer(LeafReaderContext)`, which
    /// obtains the supplier, marks it as the top-level scoring clause and asks
    /// it for a bulk scorer. A bulk scorer for the same leaf context may be
    /// requested multiple times as part of a single search call.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while building the bulk scorer.
    fn bulk_scorer(&self, context: &LeafReaderContext) -> Result<Option<Box<dyn BulkScorer>>> {
        let scorer_supplier = self.scorer_supplier(context)?;
        let Some(mut scorer_supplier) = scorer_supplier else {
            // No docs match
            return Ok(None);
        };

        scorer_supplier.set_top_level_scoring_clause()?;
        Ok(Some(scorer_supplier.bulk_scorer()?))
    }
}

/// A wrapper for a supplier that always returns the same, already-built
/// [`Scorer`].
///
/// Equivalent to `org.apache.lucene.search.Weight.DefaultScorerSupplier`.
///
/// **Divergence from Lucene 10.5.0.** Java keeps the scorer in a field and
/// hands out the same reference on every `get(long)` call, relying on the
/// documented "must be called at most once" contract to keep that safe. Rust
/// cannot hand out an owned scorer twice, so the second call fails with
/// [`LuceneError::IllegalState`] — turning a documented misuse into a reported
/// one. For the same reason the cost is computed once, at construction, rather
/// than on every `cost()` call; the value is identical because a scorer's
/// iterator cost is stable.
pub struct DefaultScorerSupplier {
    scorer: Option<Box<dyn Scorer>>,
    cost: i64,
}

impl std::fmt::Debug for DefaultScorerSupplier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DefaultScorerSupplier")
            .field("cost", &self.cost)
            .field("consumed", &self.scorer.is_none())
            .finish()
    }
}

impl DefaultScorerSupplier {
    /// Wraps an already-built scorer.
    ///
    /// Equivalent to `new DefaultScorerSupplier(Scorer)`; Java's
    /// `Objects.requireNonNull` is unnecessary because `Box<dyn Scorer>` cannot
    /// be null.
    pub fn new(mut scorer: Box<dyn Scorer>) -> Self {
        let cost = scorer.iterator().cost();
        Self {
            scorer: Some(scorer),
            cost,
        }
    }
}

impl ScorerSupplier for DefaultScorerSupplier {
    fn get(&mut self, _lead_cost: i64) -> Result<Box<dyn Scorer>> {
        self.scorer.take().ok_or_else(|| {
            LuceneError::IllegalState(
                "ScorerSupplier.get(long) must be called at most once".to_string(),
            )
        })
    }

    fn cost(&self) -> i64 {
        self.cost
    }
}

/// A weight that contains another weight and implements every method by calling
/// the contained weight's method.
///
/// Equivalent to `org.apache.lucene.search.FilterWeight`. Java leaves the class
/// abstract so that a subclass must exist to change something; Rust composition
/// makes the wrapper concrete and a "subclass" is a type that holds one.
#[derive(Debug)]
pub struct FilterWeight {
    query: Arc<dyn Query>,
    /// The wrapped weight.
    ///
    /// Equivalent to the `protected final Weight in` field.
    pub inner: Arc<dyn Weight>,
}

impl FilterWeight {
    /// Wraps the given weight, taking the parent query from it.
    ///
    /// Equivalent to `FilterWeight(Weight)`, which is
    /// `this(weight.getQuery(), weight)`.
    pub fn new(weight: Arc<dyn Weight>) -> Self {
        let query = weight.get_query();
        Self::with_query(query, weight)
    }

    /// Wraps the given weight under a different parent query.
    ///
    /// Equivalent to `FilterWeight(Query, Weight)`. Use this variant only if
    /// the weight was not obtained through
    /// [`Query::create_weight`] on that query.
    pub fn with_query(query: Arc<dyn Query>, weight: Arc<dyn Weight>) -> Self {
        Self {
            query,
            inner: weight,
        }
    }
}

impl SegmentCacheable for FilterWeight {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.inner.is_cacheable(ctx)
    }
}

impl Weight for FilterWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        Arc::clone(&self.query)
    }

    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation> {
        self.inner.explain(context, doc)
    }

    fn matches(&self, context: &LeafReaderContext, doc: i32) -> Result<Option<Arc<dyn Matches>>> {
        self.inner.matches(context, doc)
    }

    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        self.inner.scorer_supplier(context)
    }

    fn count(&self, context: &LeafReaderContext) -> Result<i32> {
        self.inner.count(context)
    }
}
