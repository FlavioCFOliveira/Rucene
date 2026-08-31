//! Seeded nearest-neighbour search, ported from
//! `org.apache.lucene.search.SeededKnnVectorQuery`.

#![deny(unsafe_code)]

use std::any::Any;
use std::sync::Arc;

use crate::error::Result;
use crate::index::{DocIndexIterator, FieldInfo, LeafReaderContext, QueryTimeout};
use crate::search::abstract_knn_vector_query::{
    knn_vector_query_rewrite, AbstractKnnVectorQuery, AbstractKnnVectorQueryImpl,
};
use crate::search::boolean_clause::Occur;
use crate::search::boolean_query::Builder as BooleanQueryBuilder;
use crate::search::collection_terminated_exception::CollectionError;
use crate::search::collector::{Collector, CollectorManager};
use crate::search::doc_id_set_iterator::{AcceptDocs, DocIdSetIterator, NO_MORE_DOCS};
use crate::search::field_exists_query::FieldExistsQuery;
use crate::search::index_searcher::IndexSearcher;
use crate::search::knn::knn_collector_manager::KnnCollectorManager;
use crate::search::knn::{KnnCollector, KnnSearchStrategy, Seeded, TopDocs};
use crate::search::query::Query;
use crate::search::query_visitor::QueryVisitor;
use crate::search::score_mode::ScoreMode;
use crate::search::top_docs_collector::TopDocsCollector;
use crate::search::top_score_doc_collector_manager::TopScoreDocCollectorManager;
use crate::search::vector_scorer::VectorScorer;
use crate::search::weight::Weight;

/// A kNN vector query that provides a query seed to initiate the vector search.
///
/// Equivalent to `org.apache.lucene.search.SeededKnnVectorQuery`. The
/// underlying format is free to ignore the provided seed.
///
/// See ["Lexically-Accelerated Dense Retrieval"](https://dl.acm.org/doi/10.1145/3539618.3591715)
/// (Kulkarni, MacAvaney, Goharian and Frieder), SIGIR '23, pp. 152-162.
#[derive(Debug, Clone)]
pub struct SeededKnnVectorQuery {
    base: AbstractKnnVectorQuery,
    seed: Arc<dyn Query>,
    seed_weight: Option<Arc<dyn Weight>>,
    delegate: Arc<dyn AbstractKnnVectorQueryImpl>,
}

impl SeededKnnVectorQuery {
    /// Wraps a kNN query with a seed query.
    ///
    /// Equivalent to the package-private
    /// `SeededKnnVectorQuery(AbstractKnnVectorQuery, Query, Weight, String, int, Query, KnnSearchStrategy)`,
    /// and to the two public constructors that read the delegate's own fields
    /// — which is what [`from_query`](Self::from_query) does.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`](crate::error::LuceneError::IllegalArgument)
    /// when `k` is less than 1.
    pub fn new(
        knn_query: Arc<dyn AbstractKnnVectorQueryImpl>,
        seed: Arc<dyn Query>,
        seed_weight: Option<Arc<dyn Weight>>,
        field: impl Into<String>,
        k: i32,
        filter: Option<Arc<dyn Query>>,
        search_strategy: Option<KnnSearchStrategy>,
    ) -> Result<Self> {
        Ok(Self {
            base: AbstractKnnVectorQuery::new(field, k, filter, search_strategy)?,
            seed,
            seed_weight,
            delegate: knn_query,
        })
    }

    /// Wraps a kNN query with a seed query, taking the field, `k`, filter and
    /// strategy from the wrapped query.
    ///
    /// Equivalent to the public
    /// `SeededKnnVectorQuery(KnnFloatVectorQuery, Query, Weight)` and
    /// `SeededKnnVectorQuery(KnnByteVectorQuery, Query, Weight)` constructors,
    /// which differ only in the static type they accept, and to the
    /// `fromFloatQuery` / `fromByteQuery` factories when `seed_weight` is
    /// `None`.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn from_query(
        knn_query: Arc<dyn AbstractKnnVectorQueryImpl>,
        seed: Arc<dyn Query>,
        seed_weight: Option<Arc<dyn Weight>>,
    ) -> Result<Self> {
        let base = knn_query.knn_base().clone();
        Self::new(
            knn_query,
            seed,
            seed_weight,
            base.get_field(),
            base.get_k(),
            base.get_filter().cloned(),
            base.get_search_strategy().cloned(),
        )
    }

    /// Returns the seed query.
    pub fn seed(&self) -> &Arc<dyn Query> {
        &self.seed
    }

    /// Returns the weight of the seed query, once the query has been rewritten.
    pub fn seed_weight(&self) -> Option<&Arc<dyn Weight>> {
        self.seed_weight.as_ref()
    }

    /// Returns the wrapped kNN query.
    pub fn delegate(&self) -> &Arc<dyn AbstractKnnVectorQueryImpl> {
        &self.delegate
    }

    /// Builds the weight the seed query is scored with.
    ///
    /// Equivalent to `SeededKnnVectorQuery.createSeedWeight(IndexSearcher)`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while rewriting or weighting the seed query.
    pub fn create_seed_weight(&self, index_searcher: &IndexSearcher) -> Result<Arc<dyn Weight>> {
        let mut builder = BooleanQueryBuilder::new();
        builder.add(Arc::clone(&self.seed), Occur::MUST)?;
        builder.add(
            Arc::new(FieldExistsQuery::new(self.base.get_field())),
            Occur::FILTER,
        )?;
        if let Some(filter) = self.base.get_filter() {
            builder.add(Arc::clone(filter), Occur::FILTER)?;
        }
        let boolean_query: Arc<dyn Query> = Arc::new(builder.build());
        let seed_rewritten = index_searcher.rewrite(boolean_query)?;
        index_searcher.create_weight(seed_rewritten, ScoreMode::TOP_SCORES, 1.0)
    }

    /// Returns the kNN vector field the search runs on.
    ///
    /// Equivalent to `SeededKnnVectorQuery.getField()`, which delegates.
    pub fn get_field(&self) -> &str {
        self.delegate.knn_base().get_field()
    }

    /// Returns the maximum number of results the search returns.
    ///
    /// Equivalent to `SeededKnnVectorQuery.getK()`, which delegates.
    pub fn get_k(&self) -> i32 {
        self.delegate.knn_base().get_k()
    }

    /// Returns the filter executed before the vector search.
    ///
    /// Equivalent to `SeededKnnVectorQuery.getFilter()`, which delegates.
    pub fn get_filter(&self) -> Option<&Arc<dyn Query>> {
        self.delegate.knn_base().get_filter()
    }
}

impl AbstractKnnVectorQueryImpl for SeededKnnVectorQuery {
    fn knn_base(&self) -> &AbstractKnnVectorQuery {
        &self.base
    }

    fn as_query(&self) -> &dyn Query {
        self
    }

    fn clone_knn_query(&self) -> Arc<dyn AbstractKnnVectorQueryImpl> {
        Arc::new(self.clone())
    }

    fn approximate_search(
        &self,
        context: &LeafReaderContext,
        accept_docs: &mut dyn AcceptDocs,
        visited_limit: i32,
        knn_collector_manager: &dyn KnnCollectorManager,
    ) -> Result<TopDocs> {
        let manager = SeededCollectorManager {
            knn_collector_manager: ErasedManager(knn_collector_manager),
            seed_weight: self.seed_weight.clone(),
            delegate: Arc::clone(&self.delegate),
            k: self.base.get_k(),
            field: self.base.get_field().to_string(),
        };
        self.delegate
            .approximate_search(context, accept_docs, visited_limit, &manager)
    }

    fn create_vector_scorer(
        &self,
        context: &LeafReaderContext,
        field_info: &FieldInfo,
    ) -> Result<Option<Box<dyn VectorScorer>>> {
        self.delegate.create_vector_scorer(context, field_info)
    }

    fn get_knn_collector_manager(
        &self,
        k: i32,
        searcher: &IndexSearcher,
    ) -> Arc<dyn KnnCollectorManager> {
        self.delegate.get_knn_collector_manager(k, searcher)
    }

    fn exact_search(
        &self,
        context: &LeafReaderContext,
        accept_iterator: Box<dyn DocIdSetIterator>,
        query_timeout: Option<&Arc<dyn QueryTimeout>>,
    ) -> Result<TopDocs> {
        self.delegate
            .exact_search(context, accept_iterator, query_timeout)
    }

    fn merge_leaf_results(&self, per_leaf_results: &[TopDocs]) -> Result<TopDocs> {
        self.delegate.merge_leaf_results(per_leaf_results)
    }
}

impl Query for SeededKnnVectorQuery {
    fn to_query_string(&self, _field: &str) -> String {
        format!(
            "SeededKnnVectorQuery{{seed={}, seedWeight={}, delegate={}}}",
            crate::search::query_to_string(self.seed.as_ref()),
            match &self.seed_weight {
                None => "null".to_string(),
                Some(weight) => format!("{weight:?}"),
            },
            crate::search::query_to_string(self.delegate.as_query())
        )
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        self.delegate.visit(visitor);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn rewrite(&self, index_searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        if self.seed_weight.is_some() {
            return knn_vector_query_rewrite(self.clone_knn_query(), index_searcher);
        }
        let delegate_base = self.delegate.knn_base().clone();
        let rewritten = SeededKnnVectorQuery::new(
            Arc::clone(&self.delegate),
            Arc::clone(&self.seed),
            Some(self.create_seed_weight(index_searcher)?),
            delegate_base.get_field(),
            delegate_base.get_k(),
            delegate_base.get_filter().cloned(),
            delegate_base.get_search_strategy().cloned(),
        )?;
        knn_vector_query_rewrite(rewritten.clone_knn_query(), index_searcher)
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        match other.as_any().downcast_ref::<SeededKnnVectorQuery>() {
            Some(other) => {
                self.base.base_eq(&other.base)
                    && self.seed.query_eq(other.seed.as_ref())
                    && match (&self.seed_weight, &other.seed_weight) {
                        (None, None) => true,
                        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                        _ => false,
                    }
                    && self.delegate.query_eq(other.delegate.as_query())
            }
            None => false,
        }
    }

    fn query_hash(&self) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        self.base.base_hash().hash(&mut hasher);
        self.seed.query_hash().hash(&mut hasher);
        self.seed_weight
            .as_ref()
            .map(|weight| Arc::as_ptr(weight) as *const () as usize)
            .hash(&mut hasher);
        self.delegate.query_hash().hash(&mut hasher);
        hasher.finish()
    }
}

/// Borrows a [`KnnCollectorManager`] so that it can be nested inside another
/// manager without being cloned.
///
/// Equivalent to the field `SeededCollectorManager.knnCollectorManager`, which
/// Java holds as a plain reference.
struct ErasedManager<'a>(&'a dyn KnnCollectorManager);

impl std::fmt::Debug for ErasedManager<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// A collector manager that seeds the graph search with the hits of the seed
/// query.
///
/// Equivalent to the inner class
/// `SeededKnnVectorQuery.SeededCollectorManager`.
#[derive(Debug)]
struct SeededCollectorManager<'a> {
    knn_collector_manager: ErasedManager<'a>,
    seed_weight: Option<Arc<dyn Weight>>,
    delegate: Arc<dyn AbstractKnnVectorQueryImpl>,
    k: i32,
    field: String,
}

impl KnnCollectorManager for SeededCollectorManager<'_> {
    fn new_collector(
        &self,
        visit_limit: i32,
        search_strategy: Option<&KnnSearchStrategy>,
        ctx: &LeafReaderContext,
    ) -> Result<Box<dyn KnnCollector>> {
        let mut seed_collector =
            TopScoreDocCollectorManager::new(self.k, None, i32::MAX)?.new_collector()?;
        let leaf_reader = ctx.leaf_reader();
        {
            let mut leaf_collector = match seed_collector.get_leaf_collector(ctx) {
                Ok(leaf_collector) => leaf_collector,
                Err(CollectionError::CollectionTerminated) => {
                    // Java's `getLeafCollector` returns a collector here; the
                    // port signals "no document of interest in this leaf" with
                    // the same exception the searcher catches, so there is
                    // nothing to collect.
                    return self.knn_collector_manager.0.new_collector(
                        visit_limit,
                        search_strategy,
                        ctx,
                    );
                }
                Err(CollectionError::Lucene(error)) => return Err(error),
                Err(CollectionError::TimeExceeded) => {
                    // The searcher aborts the whole query on a timeout; there
                    // is nothing to seed with, so fall back to the plain
                    // collector.
                    return self.knn_collector_manager.0.new_collector(
                        visit_limit,
                        search_strategy,
                        ctx,
                    );
                }
            };
            if let Some(seed_weight) = &self.seed_weight {
                if let Some(mut scorer) = seed_weight.bulk_scorer(ctx)? {
                    let live_docs = leaf_reader.get_live_docs();
                    match scorer.score(
                        leaf_collector.as_mut(),
                        live_docs.as_deref(),
                        0,
                        NO_MORE_DOCS,
                    ) {
                        Ok(_) => {}
                        Err(CollectionError::CollectionTerminated)
                        | Err(CollectionError::TimeExceeded) => {}
                        Err(CollectionError::Lucene(error)) => return Err(error),
                    }
                }
            }
            leaf_collector.finish()?;
        }

        let delegate_collector =
            self.knn_collector_manager
                .0
                .new_collector(visit_limit, search_strategy, ctx)?;
        let seed_top_docs = seed_collector.top_docs()?;
        let infos = leaf_reader.get_field_infos();
        let scorer = match infos.field_info(&self.field) {
            Some(field_info) => self.delegate.create_vector_scorer(ctx, field_info)?,
            None => None,
        };
        let (Some(scorer), true) = (scorer, seed_top_docs.total_hits.value() != 0) else {
            return Ok(delegate_collector);
        };
        let Some(seed_docs) = mapped_seed_ordinals(scorer, &seed_top_docs, ctx)? else {
            return Ok(delegate_collector);
        };
        let strategy = KnnSearchStrategy::Seeded(Seeded::new(
            Some(seed_docs),
            seed_top_docs.score_docs.len() as i32,
            search_strategy
                .cloned()
                .unwrap_or_else(KnnSearchStrategy::hnsw_default),
        )?);
        self.knn_collector_manager
            .0
            .new_collector(visit_limit, Some(&strategy), ctx)
    }
}

/// Maps the doc IDs of a seed [`TopDocs`] onto the vector ordinals of a leaf.
///
/// Equivalent to what `new MappedDISI(indexIterator, new TopDocsDISI(seedTopDocs, ctx))`
/// yields when it is drained, and returns `None` where Java's
/// `vectorIterator instanceof KnnVectorValues.DocIndexIterator` test fails.
///
/// **Divergence from Lucene 10.5.0.** Java hands the lazy `MappedDISI` to
/// `KnnSearchStrategy.Seeded`, which the consumer drains. A
/// [`KnnSearchStrategy`] must be `Send + Sync` in this port, because a
/// [`Query`] is, while a vector scorer is not; the mapping is therefore
/// performed eagerly here and the strategy carries the resulting ordinals. The
/// sequence handed to the consumer is identical — `SeededHnswGraphSearcher`
/// only ever calls `nextDoc()` on it — and Java's own
/// `numberOfEntryPoints` is the full seed count, so no extra document is
/// mapped in the common case. [`MappedDISI`] is still ported, for a caller that
/// owns the vector iterator outright.
///
/// # Errors
///
/// Propagates any I/O error raised while advancing the vector iterator.
pub fn mapped_seed_ordinals(
    mut scorer: Box<dyn VectorScorer>,
    seed_top_docs: &TopDocs,
    ctx: &LeafReaderContext,
) -> Result<Option<Box<dyn DocIdSetIterator + Send>>> {
    let mut source = TopDocsDISI::new(seed_top_docs, ctx);
    let Some(indexed) = scorer.doc_index_iterator() else {
        return Ok(None);
    };
    let mut ordinals = Vec::with_capacity(seed_top_docs.score_docs.len());
    loop {
        let new_target = source.next_doc()?;
        if new_target == NO_MORE_DOCS {
            break;
        }
        indexed.advance(new_target)?;
        if indexed.doc_id() == NO_MORE_DOCS {
            break;
        }
        ordinals.push(indexed.index());
    }
    Ok(Some(Box::new(OrdinalIterator::new(ordinals))))
}

/// Iterates a precomputed list of vector ordinals.
///
/// It stands in for the drained [`MappedDISI`]; see [`mapped_seed_ordinals`].
#[derive(Debug)]
struct OrdinalIterator {
    ordinals: Vec<i32>,
    idx: i32,
}

impl OrdinalIterator {
    fn new(ordinals: Vec<i32>) -> Self {
        Self { ordinals, idx: -1 }
    }
}

impl DocIdSetIterator for OrdinalIterator {
    fn doc_id(&self) -> i32 {
        if self.idx < 0 {
            -1
        } else if self.idx as usize >= self.ordinals.len() {
            NO_MORE_DOCS
        } else {
            self.ordinals[self.idx as usize]
        }
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.idx += 1;
        Ok(self.doc_id())
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.slow_advance(target)
    }

    fn cost(&self) -> i64 {
        self.ordinals.len() as i64
    }
}

/// Translates the doc IDs of a source iterator into vector ordinals.
///
/// Equivalent to the static nested `SeededKnnVectorQuery.MappedDISI`.
pub struct MappedDISI {
    indexed_disi: Box<dyn DocIndexIterator>,
    source_disi: Box<dyn DocIdSetIterator>,
}

impl std::fmt::Debug for MappedDISI {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MappedDISI").finish_non_exhaustive()
    }
}

impl MappedDISI {
    /// Wraps a vector iterator and the source of the doc IDs to translate.
    ///
    /// Equivalent to
    /// `new MappedDISI(KnnVectorValues.DocIndexIterator, DocIdSetIterator)`.
    pub fn new(
        indexed_disi: Box<dyn DocIndexIterator>,
        source_disi: Box<dyn DocIdSetIterator>,
    ) -> Self {
        Self {
            indexed_disi,
            source_disi,
        }
    }
}

impl DocIdSetIterator for MappedDISI {
    fn doc_id(&self) -> i32 {
        if self.indexed_disi.doc_id() == NO_MORE_DOCS || self.source_disi.doc_id() == NO_MORE_DOCS {
            return NO_MORE_DOCS;
        }
        self.indexed_disi.index()
    }

    fn next_doc(&mut self) -> Result<i32> {
        let new_target = self.source_disi.next_doc()?;
        if new_target != NO_MORE_DOCS {
            self.indexed_disi.advance(new_target)?;
        }
        Ok(self.doc_id())
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let new_target = self.source_disi.advance(target)?;
        if new_target != NO_MORE_DOCS {
            self.indexed_disi.advance(new_target)?;
        }
        Ok(self.doc_id())
    }

    fn cost(&self) -> i64 {
        self.source_disi.cost()
    }
}

/// Iterates the doc IDs of a [`TopDocs`], re-based into a leaf and sorted.
///
/// Equivalent to the static nested `SeededKnnVectorQuery.TopDocsDISI`.
#[derive(Debug)]
pub struct TopDocsDISI {
    sorted_doc_ids: Vec<i32>,
    idx: i32,
}

impl TopDocsDISI {
    /// Collects and sorts the leaf-local doc IDs of `top_docs`.
    ///
    /// Equivalent to `new TopDocsDISI(TopDocs, LeafReaderContext)`, which
    /// removes the doc base the collector added.
    pub fn new(top_docs: &TopDocs, ctx: &LeafReaderContext) -> Self {
        let mut sorted_doc_ids: Vec<i32> = top_docs
            .score_docs
            .iter()
            .map(|score_doc| score_doc.doc - ctx.doc_base())
            .collect();
        sorted_doc_ids.sort_unstable();
        Self {
            sorted_doc_ids,
            idx: -1,
        }
    }
}

impl DocIdSetIterator for TopDocsDISI {
    fn doc_id(&self) -> i32 {
        if self.idx == -1 {
            -1
        } else if self.idx as usize >= self.sorted_doc_ids.len() {
            NO_MORE_DOCS
        } else {
            self.sorted_doc_ids[self.idx as usize]
        }
    }

    fn next_doc(&mut self) -> Result<i32> {
        self.idx += 1;
        Ok(self.doc_id())
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.slow_advance(target)
    }

    fn cost(&self) -> i64 {
        self.sorted_doc_ids.len() as i64
    }
}
