//! Similarity-threshold vector search, ported from
//! `org.apache.lucene.search.AbstractVectorSimilarityQuery`.
//!
//! # Adaptation: base class in two halves
//!
//! As with [`AbstractKnnVectorQuery`](crate::search::AbstractKnnVectorQuery),
//! the Java abstract class becomes a state struct —
//! [`AbstractVectorSimilarityQuery`] — plus a trait carrying the abstract
//! methods, [`AbstractVectorSimilarityQueryImpl`]. `createWeight` is the free
//! function [`vector_similarity_create_weight`], which the concrete queries
//! call from their own [`Query::create_weight`].

#![deny(unsafe_code)]

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::{LeafReaderContext, QueryTimeout};
use crate::search::conjunction_utils::ConjunctionUtils;
use crate::search::doc_id_set_iterator::{
    empty, from_iterator_supplier, from_live_docs, AcceptDocs, DocIdSetIterator, NO_MORE_DOCS,
};
use crate::search::index_searcher::IndexSearcher;
use crate::search::knn::knn_collector_manager::KnnCollectorManager;
use crate::search::knn::{Hnsw, KnnCollector, KnnSearchStrategy, TopDocs};
use crate::search::query::Query;
use crate::search::query_visitor::QueryVisitor;
use crate::search::scorable::Scorable;
use crate::search::score_doc::ScoreDoc;
use crate::search::score_mode::ScoreMode;
use crate::search::scorer::{into_scorer_iterator, Scorer};
use crate::search::scorer_supplier::ScorerSupplier;
use crate::search::segment_cacheable::SegmentCacheable;
use crate::search::similarities::Explanation;
use crate::search::time_limiting_knn_collector_manager::TimeLimitingKnnCollectorManager;
use crate::search::total_hits::TotalHitsRelation;
use crate::search::vector_scorer::{SharedVectorScorer, VectorScorer};
use crate::search::vector_similarity_collector::VectorSimilarityCollector;
use crate::search::weight::Weight;

/// The lowest decay factor: maximum approximation.
///
/// Equivalent to `AbstractVectorSimilarityQuery.DECAY_MAX_APPROXIMATION`.
pub const DECAY_MAX_APPROXIMATION: f32 = 0.0;

/// The default decay factor.
///
/// Equivalent to `AbstractVectorSimilarityQuery.DEFAULT_DECAY`.
pub const DEFAULT_DECAY: f32 = 0.5;

/// The highest decay factor: maximum quality.
///
/// Equivalent to `AbstractVectorSimilarityQuery.DECAY_MAX_QUALITY`.
pub const DECAY_MAX_QUALITY: f32 = 1.0;

/// The default search strategy of the similarity-threshold vector queries.
///
/// Equivalent to `AbstractVectorSimilarityQuery.DEFAULT_STRATEGY`, an `Hnsw`
/// with `filteredSearchThreshold == 0`, which preserves this query's own
/// filter handling by never delegating to HNSW's built-in filtered-search
/// short-circuit.
pub fn default_similarity_strategy() -> KnnSearchStrategy {
    KnnSearchStrategy::Hnsw(Hnsw::DEFAULT)
}

/// The state of `AbstractVectorSimilarityQuery`.
///
/// Equivalent to the five `protected final` fields of the abstract class
/// `org.apache.lucene.search.AbstractVectorSimilarityQuery`.
#[derive(Debug, Clone)]
pub struct AbstractVectorSimilarityQuery {
    field: String,
    result_similarity: f32,
    decay: f32,
    filter: Option<Arc<dyn Query>>,
    search_strategy: KnnSearchStrategy,
}

impl AbstractVectorSimilarityQuery {
    /// Creates the base state.
    ///
    /// Equivalent to the package-private
    /// `AbstractVectorSimilarityQuery(String, float, float, Query, KnnSearchStrategy)`;
    /// pass `None` for `search_strategy` to get the
    /// [`default_similarity_strategy`], as Java's `null` does.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] — with Java's messages — when
    /// `result_similarity` or `decay` is `NaN`, or when `decay` is outside
    /// `[DECAY_MAX_APPROXIMATION, DECAY_MAX_QUALITY]`.
    pub fn new(
        field: impl Into<String>,
        result_similarity: f32,
        decay: f32,
        filter: Option<Arc<dyn Query>>,
        search_strategy: Option<KnnSearchStrategy>,
    ) -> Result<Self> {
        if result_similarity.is_nan() {
            return Err(LuceneError::IllegalArgument(format!(
                "resultSimilarity must have a valid value; got {result_similarity}"
            )));
        }
        if decay.is_nan() {
            return Err(LuceneError::IllegalArgument(format!(
                "decay must have a valid value; got {decay}"
            )));
        }
        if !(DECAY_MAX_APPROXIMATION..=DECAY_MAX_QUALITY).contains(&decay) {
            return Err(LuceneError::IllegalArgument(format!(
                "decay must lie in range [DECAY_MAX_APPROXIMATION = 0, DECAY_MAX_QUALITY = 1]; got {decay}"
            )));
        }
        Ok(Self {
            field: field.into(),
            result_similarity,
            decay,
            filter,
            search_strategy: search_strategy.unwrap_or_else(default_similarity_strategy),
        })
    }

    /// Returns the vector field the search runs on.
    pub fn get_field(&self) -> &str {
        &self.field
    }

    /// Returns the similarity score a document must reach to be collected.
    pub fn result_similarity(&self) -> f32 {
        self.result_similarity
    }

    /// Returns the decay factor of the graph traversal buffer.
    pub fn decay(&self) -> f32 {
        self.decay
    }

    /// Returns the filter applied before the vector search.
    pub fn get_filter(&self) -> Option<&Arc<dyn Query>> {
        self.filter.as_ref()
    }

    /// Returns the search strategy used during graph search.
    ///
    /// Equivalent to `AbstractVectorSimilarityQuery.getSearchStrategy()`.
    pub fn get_search_strategy(&self) -> &KnnSearchStrategy {
        &self.search_strategy
    }

    /// Query equivalence over the base fields.
    ///
    /// Equivalent to `AbstractVectorSimilarityQuery.equals(Object)`, minus the
    /// class check the concrete queries perform themselves. Java compares the
    /// two float fields with `Float.compare`, so `NaN` equals `NaN` and `-0.0`
    /// differs from `0.0`; comparing the raw bits reproduces that exactly, and
    /// `NaN` cannot reach here anyway because the constructor rejects it.
    pub fn base_eq(&self, other: &AbstractVectorSimilarityQuery) -> bool {
        self.field == other.field
            && self.result_similarity.to_bits() == other.result_similarity.to_bits()
            && self.decay.to_bits() == other.decay.to_bits()
            && match (&self.filter, &other.filter) {
                (None, None) => true,
                (Some(a), Some(b)) => a.query_eq(b.as_ref()),
                _ => false,
            }
            && self.search_strategy == other.search_strategy
    }

    /// Query hash over the base fields.
    ///
    /// Equivalent to `AbstractVectorSimilarityQuery.hashCode()`.
    pub fn base_hash(&self) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        self.field.hash(&mut hasher);
        self.result_similarity.to_bits().hash(&mut hasher);
        self.decay.to_bits().hash(&mut hasher);
        self.filter
            .as_ref()
            .map(|filter| filter.query_hash())
            .hash(&mut hasher);
        self.search_strategy.hash(&mut hasher);
        hasher.finish()
    }
}

/// The abstract behaviour of `AbstractVectorSimilarityQuery`.
///
/// Equivalent to the two abstract methods `createVectorScorer` and
/// `approximateSearch`, plus the concrete `getKnnCollectorManager`.
pub trait AbstractVectorSimilarityQueryImpl: Query {
    /// Returns the base state of this query.
    fn similarity_base(&self) -> &AbstractVectorSimilarityQuery;

    /// Returns this query viewed as a [`Query`].
    ///
    /// **Divergence from Lucene 10.5.0.** Rust before 1.86 cannot coerce a
    /// `&dyn AbstractVectorSimilarityQueryImpl` into a `&dyn Query`, and this
    /// crate's minimum supported Rust version is 1.80, so the upcast is spelled
    /// out. Every implementation writes `self`.
    fn as_query(&self) -> &dyn Query;

    /// Returns this query as a shared handle on itself, so that the weight can
    /// outlive the `&self` that created it.
    ///
    /// **Divergence from Lucene 10.5.0.** Java's anonymous `Weight` captures
    /// the enclosing query; a Rust weight has to own a handle on it. Every
    /// implementation is `Arc::new(self.clone())`.
    fn clone_similarity_query(&self) -> Arc<dyn AbstractVectorSimilarityQueryImpl>;

    /// Returns this query as a shared handle typed as a [`Query`].
    ///
    /// **Divergence from Lucene 10.5.0.** It exists for the same reason
    /// [`as_query`](Self::as_query) does: an `Arc<dyn AbstractVectorSimilarityQueryImpl>`
    /// cannot be coerced into an `Arc<dyn Query>` before Rust 1.86, and
    /// [`Weight::get_query`] must hand one out. Every implementation is
    /// `Arc::new(self.clone())`.
    fn clone_as_query(&self) -> Arc<dyn Query>;

    /// Creates the exact vector scorer of one leaf.
    ///
    /// Equivalent to the abstract
    /// `AbstractVectorSimilarityQuery.createVectorScorer(LeafReaderContext)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while opening the vector values.
    fn create_vector_scorer(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn VectorScorer>>>;

    /// Runs the approximate (graph) search over one leaf.
    ///
    /// Equivalent to the abstract
    /// `AbstractVectorSimilarityQuery.approximateSearch(LeafReaderContext, AcceptDocs, int, KnnCollectorManager)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while searching.
    fn approximate_search(
        &self,
        context: &LeafReaderContext,
        accept_docs: &mut dyn AcceptDocs,
        visit_limit: i32,
        knn_collector_manager: &dyn KnnCollectorManager,
    ) -> Result<TopDocs>;

    /// Returns a manager that always builds a [`VectorSimilarityCollector`]
    /// configured with this query's own values.
    ///
    /// Equivalent to `AbstractVectorSimilarityQuery.getKnnCollectorManager()`,
    /// which returns a lambda.
    fn get_knn_collector_manager(&self) -> Arc<dyn KnnCollectorManager> {
        let base = self.similarity_base();
        Arc::new(VectorSimilarityCollectorManager {
            result_similarity: base.result_similarity(),
            decay: base.decay(),
        })
    }
}

/// The manager `AbstractVectorSimilarityQuery.getKnnCollectorManager()` returns.
///
/// Equivalent to the lambda implementing `KnnCollectorManager` there; Rust has
/// no closure that also carries a `Debug` implementation, so it is a struct.
#[derive(Debug, Clone, Copy)]
struct VectorSimilarityCollectorManager {
    result_similarity: f32,
    decay: f32,
}

impl KnnCollectorManager for VectorSimilarityCollectorManager {
    fn new_collector(
        &self,
        visit_limit: i32,
        search_strategy: Option<&KnnSearchStrategy>,
        _context: &LeafReaderContext,
    ) -> Result<Box<dyn KnnCollector>> {
        Ok(Box::new(VectorSimilarityCollector::with_strategy(
            self.result_similarity,
            self.decay,
            visit_limit as i64,
            search_strategy.cloned(),
        )))
    }
}

/// Visits a similarity-threshold vector query.
///
/// Equivalent to `AbstractVectorSimilarityQuery.visit(QueryVisitor)`.
pub fn vector_similarity_query_visit(
    base: &AbstractVectorSimilarityQuery,
    query: &dyn Query,
    visitor: &mut dyn QueryVisitor,
) {
    if visitor.accept_field(base.get_field()) {
        visitor.visit_leaf(query);
    }
}

/// Builds the weight of a similarity-threshold vector query.
///
/// Equivalent to
/// `AbstractVectorSimilarityQuery.createWeight(IndexSearcher, ScoreMode, float)`.
///
/// # Errors
///
/// Propagates any error raised while rewriting or weighting the filter.
pub fn vector_similarity_create_weight(
    query: Arc<dyn AbstractVectorSimilarityQueryImpl>,
    searcher: &IndexSearcher,
    _score_mode: ScoreMode,
    boost: f32,
) -> Result<Arc<dyn Weight>> {
    let base = query.similarity_base();
    let filter_weight = match base.get_filter() {
        None => None,
        Some(filter) => {
            let rewritten = searcher.rewrite(Arc::clone(filter))?;
            Some(searcher.create_weight(rewritten, ScoreMode::COMPLETE_NO_SCORES, 1.0)?)
        }
    };
    let query_timeout = searcher.get_timeout().cloned();
    let time_limiting_knn_collector_manager = TimeLimitingKnnCollectorManager::new(
        query.get_knn_collector_manager(),
        query_timeout.clone(),
    );
    let parent_query = query.clone_as_query();
    Ok(Arc::new(VectorSimilarityWeight {
        parent_query,
        query,
        filter_weight,
        query_timeout,
        time_limiting_knn_collector_manager,
        boost,
    }))
}

/// The weight a similarity-threshold vector query creates.
///
/// Equivalent to the anonymous `Weight` of
/// `AbstractVectorSimilarityQuery.createWeight`.
#[derive(Debug)]
struct VectorSimilarityWeight {
    parent_query: Arc<dyn Query>,
    query: Arc<dyn AbstractVectorSimilarityQueryImpl>,
    filter_weight: Option<Arc<dyn Weight>>,
    query_timeout: Option<Arc<dyn QueryTimeout>>,
    time_limiting_knn_collector_manager: TimeLimitingKnnCollectorManager,
    boost: f32,
}

impl SegmentCacheable for VectorSimilarityWeight {
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        true
    }
}

impl Weight for VectorSimilarityWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        Arc::clone(&self.parent_query)
    }

    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation> {
        if let Some(filter_weight) = &self.filter_weight {
            let filter_scorer = filter_weight.scorer(context)?;
            let matches = match filter_scorer {
                None => false,
                Some(scorer) => {
                    let mut iterator = into_scorer_iterator(scorer).into_doc_id_set_iterator();
                    iterator.advance(doc)? <= doc
                }
            };
            if !matches {
                return Ok(Explanation::no_match(
                    "Doc does not match the filter",
                    Vec::new(),
                ));
            }
        }

        let Some(scorer) = self.query.create_vector_scorer(context)? else {
            return Ok(Explanation::no_match(
                "Not indexed as the correct vector field",
                Vec::new(),
            ));
        };
        let scorer = SharedVectorScorer::new(scorer);
        let mut iterator = scorer.iterator();
        let doc_id = iterator.advance(doc)?;
        if doc_id != doc {
            return Ok(Explanation::no_match("No vector found for doc", Vec::new()));
        }
        let score = scorer.score()?;
        if score >= self.query.similarity_base().result_similarity() {
            Ok(Explanation::matched(
                self.boost * score,
                "Score above threshold",
                Vec::new(),
            ))
        } else {
            Ok(Explanation::no_match("Score below threshold", Vec::new()))
        }
    }

    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let base = self.query.similarity_base();
        let leaf_reader = context.leaf_reader();
        let live_docs = leaf_reader.get_live_docs();
        let max_doc = leaf_reader.max_doc();

        // If there is no filter.
        let Some(filter_weight) = &self.filter_weight else {
            if base.decay() == DECAY_MAX_QUALITY {
                // With DECAY_MAX_QUALITY the intent is to find every vector
                // above result_similarity. The approximate graph search may
                // miss nodes, so use exact search to guarantee completeness.
                let mut accept_docs = from_live_docs(live_docs, max_doc)?;
                return Ok(from_accept_docs(
                    self.boost,
                    self.query.create_vector_scorer(context)?,
                    accept_docs.iterator()?,
                    base.result_similarity(),
                )?
                .map(|supplier| Box::new(supplier) as Box<dyn ScorerSupplier>));
            }
            // Return results via approximate graph search.
            let mut accept_docs = from_live_docs(live_docs, max_doc)?;
            let results = self.query.approximate_search(
                context,
                &mut accept_docs,
                i32::MAX,
                &self.time_limiting_knn_collector_manager,
            )?;
            return Ok(from_score_docs(self.boost, results.score_docs)
                .map(|supplier| Box::new(supplier) as Box<dyn ScorerSupplier>));
        };

        let supplier = || -> Result<Box<dyn DocIdSetIterator>> {
            match filter_weight.scorer(context)? {
                None => Ok(Box::new(empty())),
                Some(scorer) => Ok(into_scorer_iterator(scorer).into_doc_id_set_iterator()),
            }
        };
        let mut accept_docs = from_iterator_supplier(supplier, live_docs, max_doc)?;
        let cardinality = accept_docs.cost()?;
        if cardinality == 0 {
            // There are no live matching docs.
            return Ok(None);
        }

        if base.decay() == DECAY_MAX_QUALITY {
            // With DECAY_MAX_QUALITY, skip the approximate search and go
            // straight to exact search over the filtered docs.
            return Ok(from_accept_docs(
                self.boost,
                self.query.create_vector_scorer(context)?,
                accept_docs.iterator()?,
                base.result_similarity(),
            )?
            .map(|supplier| Box::new(supplier) as Box<dyn ScorerSupplier>));
        }

        // Perform an approximate search.
        let visit_limit = i32::try_from(cardinality).unwrap_or(i32::MAX);
        let results = self.query.approximate_search(
            context,
            &mut accept_docs,
            visit_limit,
            &self.time_limiting_knn_collector_manager,
        )?;

        if results.total_hits.relation() == TotalHitsRelation::EQUAL_TO
            // Return partial results only when the timeout is met.
            || self
                .query_timeout
                .as_ref()
                .is_some_and(|timeout| timeout.should_exit())
        {
            // Return an iterator over the collected results.
            Ok(from_score_docs(self.boost, results.score_docs)
                .map(|supplier| Box::new(supplier) as Box<dyn ScorerSupplier>))
        } else {
            // Return a lazy-loading iterator.
            Ok(from_accept_docs(
                self.boost,
                self.query.create_vector_scorer(context)?,
                accept_docs.iterator()?,
                base.result_similarity(),
            )?
            .map(|supplier| Box::new(supplier) as Box<dyn ScorerSupplier>))
        }
    }
}

/// The supplier of the scorers a similarity-threshold query produces.
///
/// Equivalent to the private static
/// `AbstractVectorSimilarityQuery.VectorSimilarityScorerSupplier`.
struct VectorSimilarityScorerSupplier {
    iterator: Option<Box<dyn DocIdSetIterator>>,
    cost: i64,
    cached_score: Rc<Cell<f32>>,
}

impl std::fmt::Debug for VectorSimilarityScorerSupplier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VectorSimilarityScorerSupplier")
            .field("cost", &self.cost)
            .finish()
    }
}

impl ScorerSupplier for VectorSimilarityScorerSupplier {
    fn get(&mut self, _lead_cost: i64) -> Result<Box<dyn Scorer>> {
        let iterator = self.iterator.take().ok_or_else(|| {
            LuceneError::IllegalState(
                "ScorerSupplier.get(long) must be called at most once".to_string(),
            )
        })?;
        Ok(Box::new(VectorSimilarityScorer {
            iterator,
            cached_score: Rc::clone(&self.cached_score),
        }))
    }

    fn cost(&self) -> i64 {
        self.cost
    }
}

/// Builds a supplier over precollected hits.
///
/// Equivalent to
/// `VectorSimilarityScorerSupplier.fromScoreDocs(float, ScoreDoc[])`, which
/// returns `null` when there are no hits.
fn from_score_docs(
    boost: f32,
    mut score_docs: Vec<ScoreDoc>,
) -> Option<VectorSimilarityScorerSupplier> {
    if score_docs.is_empty() {
        return None;
    }
    // Sort in ascending order of doc ID.
    score_docs.sort_by_key(|score_doc| score_doc.doc);
    let cached_score = Rc::new(Cell::new(0.0));
    let cost = score_docs.len() as i64;
    let iterator = ScoreDocIterator {
        score_docs,
        boost,
        index: -1,
        cached_score: Rc::clone(&cached_score),
    };
    Some(VectorSimilarityScorerSupplier {
        iterator: Some(Box::new(iterator)),
        cost,
        cached_score,
    })
}

/// Builds a supplier that scores lazily over the accepted documents.
///
/// Equivalent to
/// `VectorSimilarityScorerSupplier.fromAcceptDocs(float, VectorScorer, DocIdSetIterator, float)`,
/// which returns `null` when there is no scorer.
///
/// # Errors
///
/// Propagates any error raised while building the conjunction.
fn from_accept_docs(
    boost: f32,
    scorer: Option<Box<dyn VectorScorer>>,
    accept_docs: Box<dyn DocIdSetIterator>,
    threshold: f32,
) -> Result<Option<VectorSimilarityScorerSupplier>> {
    let Some(scorer) = scorer else {
        return Ok(None);
    };
    let cached_score = Rc::new(Cell::new(0.0));
    let scorer = SharedVectorScorer::new(scorer);
    let vector_iterator = scorer.iterator();
    let conjunction =
        ConjunctionUtils::intersect_iterators(vec![Box::new(vector_iterator), accept_docs])?
            .into_doc_id_set_iterator();
    let cost = conjunction.cost();
    let iterator = ThresholdFilteredIterator {
        inner: conjunction,
        doc: -1,
        scorer,
        boost,
        threshold,
        cached_score: Rc::clone(&cached_score),
    };
    Ok(Some(VectorSimilarityScorerSupplier {
        iterator: Some(Box::new(iterator)),
        cost,
        cached_score,
    }))
}

/// Iterates precollected hits, caching each hit's boosted score.
///
/// Equivalent to the anonymous `DocIdSetIterator` of
/// `VectorSimilarityScorerSupplier.fromScoreDocs`.
struct ScoreDocIterator {
    score_docs: Vec<ScoreDoc>,
    boost: f32,
    index: i32,
    cached_score: Rc<Cell<f32>>,
}

impl ScoreDocIterator {
    fn current(&self) -> i32 {
        if self.index < 0 {
            -1
        } else if self.index as usize >= self.score_docs.len() {
            NO_MORE_DOCS
        } else {
            let score_doc = &self.score_docs[self.index as usize];
            self.cached_score.set(self.boost * score_doc.score);
            score_doc.doc
        }
    }
}

impl DocIdSetIterator for ScoreDocIterator {
    fn doc_id(&self) -> i32 {
        self.current()
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.index += 1;
        Ok(self.current())
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.index = match self
            .score_docs
            .binary_search_by_key(&target, |score_doc| score_doc.doc)
        {
            Ok(found) => found as i32,
            Err(insertion) => insertion as i32,
        };
        Ok(self.current())
    }

    fn cost(&self) -> i64 {
        self.score_docs.len() as i64
    }
}

/// Confirms the candidates of a conjunction against the similarity threshold.
///
/// Equivalent to the anonymous `FilteredDocIdSetIterator` of
/// `VectorSimilarityScorerSupplier.fromAcceptDocs`, whose `match(int)` computes
/// the vector score, caches it and compares it with the threshold.
///
/// **Divergence from Lucene 10.5.0.** `FilteredDocIdSetIterator` itself is not
/// part of this port yet, so its two-line `nextDoc`/`advance` loop is inlined
/// here.
struct ThresholdFilteredIterator {
    inner: Box<dyn DocIdSetIterator>,
    doc: i32,
    scorer: SharedVectorScorer,
    boost: f32,
    threshold: f32,
    cached_score: Rc<Cell<f32>>,
}

impl ThresholdFilteredIterator {
    fn matches(&mut self) -> Result<bool> {
        // Compute the dot product.
        let score = self.scorer.score()?;
        self.cached_score.set(score * self.boost);
        Ok(score >= self.threshold)
    }
}

impl DocIdSetIterator for ThresholdFilteredIterator {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        loop {
            self.doc = self.inner.next_doc()?;
            if self.doc == NO_MORE_DOCS {
                return Ok(self.doc);
            }
            if self.matches()? {
                return Ok(self.doc);
            }
        }
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.doc = self.inner.advance(target)?;
        if self.doc == NO_MORE_DOCS {
            return Ok(self.doc);
        }
        if self.matches()? {
            return Ok(self.doc);
        }
        self.next_doc()
    }

    fn cost(&self) -> i64 {
        self.inner.cost()
    }
}

/// The scorer a [`VectorSimilarityScorerSupplier`] produces.
///
/// Equivalent to the anonymous `Scorer` of
/// `VectorSimilarityScorerSupplier.get(long)`.
struct VectorSimilarityScorer {
    iterator: Box<dyn DocIdSetIterator>,
    cached_score: Rc<Cell<f32>>,
}

impl std::fmt::Debug for VectorSimilarityScorer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VectorSimilarityScorer").finish()
    }
}

impl Scorable for VectorSimilarityScorer {
    fn score(&mut self) -> Result<f32> {
        Ok(self.cached_score.get())
    }
}

impl Scorer for VectorSimilarityScorer {
    fn as_scorable(&mut self) -> &mut dyn Scorable {
        self
    }

    fn doc_id(&self) -> i32 {
        self.iterator.doc_id()
    }

    fn iterator(&mut self) -> &mut dyn DocIdSetIterator {
        &mut *self.iterator
    }

    fn get_max_score(&mut self, _up_to: i32) -> Result<f32> {
        Ok(f32::INFINITY)
    }
}
