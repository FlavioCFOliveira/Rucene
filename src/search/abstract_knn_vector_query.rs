//! Nearest-neighbour vector search, ported from
//! `org.apache.lucene.search.AbstractKnnVectorQuery`.
//!
//! # Adaptation: base class in two halves
//!
//! Java's `AbstractKnnVectorQuery` is an abstract `Query` that holds four
//! fields, implements `rewrite` on top of two abstract methods, and lets
//! subclasses override four more. Rust has no implementation inheritance, so it
//! splits in two: [`AbstractKnnVectorQuery`], the struct carrying the fields
//! and the accessors, and [`AbstractKnnVectorQueryImpl`], the trait carrying
//! the abstract and overridable methods. `rewrite` is the free function
//! [`knn_vector_query_rewrite`], which every concrete query calls from its own
//! [`Query::rewrite`].

#![deny(unsafe_code)]

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::{DocAndFloatFeatureBuffer, FieldInfo, LeafReaderContext, QueryTimeout};
use crate::search::boolean_clause::Occur;
use crate::search::boolean_query::Builder as BooleanQueryBuilder;
use crate::search::doc_and_score_query::DocAndScoreQuery;
use crate::search::doc_id_set_iterator::{
    empty, from_iterator_supplier, from_live_docs, AcceptDocs, DocIdSetIterator, NO_MORE_DOCS,
};
use crate::search::field_exists_query::FieldExistsQuery;
use crate::search::hit_queue::HitQueue;
use crate::search::index_searcher::IndexSearcher;
use crate::search::knn::knn_collector_manager::KnnCollectorManager;
use crate::search::knn::top_knn_collector_manager::TopKnnCollectorManager;
use crate::search::knn::{KnnCollector, KnnSearchStrategy, TopDocs};
use crate::search::match_all_docs_query::MatchAllDocsQuery;
use crate::search::match_no_docs_query::MatchNoDocsQuery;
use crate::search::query::Query;
use crate::search::score_doc::ScoreDoc;
use crate::search::score_mode::ScoreMode;
use crate::search::scorer::into_scorer_iterator;
use crate::search::time_limiting_knn_collector_manager::TimeLimitingKnnCollectorManager;
use crate::search::top_docs_collector::empty_top_docs;
use crate::search::total_hits::{TotalHits, TotalHitsRelation};
use crate::search::vector_scorer::VectorScorer;
use crate::search::weight::Weight;

/// Controls the degree of additional result exploration done during the
/// pro-rata search of segments.
///
/// Equivalent to the private `AbstractKnnVectorQuery.LAMBDA`.
const LAMBDA: f64 = 16.0;

/// The state of `AbstractKnnVectorQuery`.
///
/// Equivalent to the four `protected final` fields of the abstract class
/// `org.apache.lucene.search.AbstractKnnVectorQuery` — `field`, `k`, `filter`
/// and `searchStrategy` — together with the four public accessors that read
/// them.
#[derive(Debug, Clone)]
pub struct AbstractKnnVectorQuery {
    field: String,
    k: i32,
    filter: Option<Arc<dyn Query>>,
    search_strategy: Option<KnnSearchStrategy>,
}

impl AbstractKnnVectorQuery {
    /// Creates the base state.
    ///
    /// Equivalent to the package-private
    /// `AbstractKnnVectorQuery(String, int, Query, KnnSearchStrategy)`
    /// constructor.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] — with Java's message — when
    /// `k` is less than 1.
    pub fn new(
        field: impl Into<String>,
        k: i32,
        filter: Option<Arc<dyn Query>>,
        search_strategy: Option<KnnSearchStrategy>,
    ) -> Result<Self> {
        if k < 1 {
            return Err(LuceneError::IllegalArgument(format!(
                "k must be at least 1, got: {k}"
            )));
        }
        Ok(Self {
            field: field.into(),
            k,
            filter,
            search_strategy,
        })
    }

    /// Returns the kNN vector field the search runs on.
    ///
    /// Equivalent to `AbstractKnnVectorQuery.getField()`.
    pub fn get_field(&self) -> &str {
        &self.field
    }

    /// Returns the maximum number of results the search returns.
    ///
    /// Equivalent to `AbstractKnnVectorQuery.getK()`.
    pub fn get_k(&self) -> i32 {
        self.k
    }

    /// Returns the filter executed before the vector search.
    ///
    /// Equivalent to `AbstractKnnVectorQuery.getFilter()`.
    pub fn get_filter(&self) -> Option<&Arc<dyn Query>> {
        self.filter.as_ref()
    }

    /// Returns the search strategy.
    ///
    /// Equivalent to `AbstractKnnVectorQuery.getSearchStrategy()`.
    pub fn get_search_strategy(&self) -> Option<&KnnSearchStrategy> {
        self.search_strategy.as_ref()
    }

    /// Query equivalence over the base fields.
    ///
    /// Equivalent to `AbstractKnnVectorQuery.equals(Object)`, minus the class
    /// check the concrete queries perform themselves.
    pub fn base_eq(&self, other: &AbstractKnnVectorQuery) -> bool {
        self.k == other.k
            && self.field == other.field
            && match (&self.filter, &other.filter) {
                (None, None) => true,
                (Some(a), Some(b)) => a.query_eq(b.as_ref()),
                _ => false,
            }
            && self.search_strategy == other.search_strategy
    }

    /// Query hash over the base fields.
    ///
    /// Equivalent to `AbstractKnnVectorQuery.hashCode()`, which — unlike
    /// `equals` — deliberately leaves the search strategy out.
    pub fn base_hash(&self) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        self.field.hash(&mut hasher);
        self.k.hash(&mut hasher);
        self.filter
            .as_ref()
            .map(|filter| filter.query_hash())
            .hash(&mut hasher);
        hasher.finish()
    }
}

/// The abstract and overridable behaviour of `AbstractKnnVectorQuery`.
///
/// Equivalent to the methods the abstract class leaves to its subclasses:
/// `approximateSearch` and `createVectorScorer` are abstract, while
/// `getKnnCollectorManager`, `exactSearch` and `mergeLeafResults` carry the
/// bodies reproduced here as defaults.
pub trait AbstractKnnVectorQueryImpl: Query {
    /// Returns the base state of this query.
    ///
    /// Equivalent to reading the inherited `field`, `k`, `filter` and
    /// `searchStrategy` fields.
    fn knn_base(&self) -> &AbstractKnnVectorQuery;

    /// Returns this query viewed as a [`Query`].
    ///
    /// **Divergence from Lucene 10.5.0.** Java gets this for free, because
    /// `AbstractKnnVectorQuery extends Query`. Rust before 1.86 cannot coerce a
    /// `&dyn AbstractKnnVectorQueryImpl` into a `&dyn Query`, and this crate's
    /// minimum supported Rust version is 1.80, so the upcast is spelled out as
    /// a method. Every implementation writes `self`.
    fn as_query(&self) -> &dyn Query;

    /// Returns this query as a shared handle on itself.
    ///
    /// **Divergence from Lucene 10.5.0.** Java's `rewrite` hands `this` to the
    /// per-leaf tasks it submits to the `TaskExecutor`; the JVM keeps the query
    /// alive for as long as the tasks run. This port's
    /// [`TaskExecutor`](crate::search::TaskExecutor) requires `'static` tasks,
    /// so the tasks need an owning handle. Every implementation is
    /// `Arc::new(self.clone())`, which is the same shape
    /// [`Query::create_weight`] implementations already use.
    fn clone_knn_query(&self) -> Arc<dyn AbstractKnnVectorQueryImpl>;

    /// Runs the approximate (graph) search over one leaf.
    ///
    /// Equivalent to the abstract
    /// `AbstractKnnVectorQuery.approximateSearch(LeafReaderContext, AcceptDocs, int, KnnCollectorManager)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while searching.
    fn approximate_search(
        &self,
        context: &LeafReaderContext,
        accept_docs: &mut dyn AcceptDocs,
        visited_limit: i32,
        knn_collector_manager: &dyn KnnCollectorManager,
    ) -> Result<TopDocs>;

    /// Creates the exact vector scorer of one leaf.
    ///
    /// Equivalent to the abstract
    /// `AbstractKnnVectorQuery.createVectorScorer(LeafReaderContext, FieldInfo)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while opening the vector values.
    fn create_vector_scorer(
        &self,
        context: &LeafReaderContext,
        field_info: &FieldInfo,
    ) -> Result<Option<Box<dyn VectorScorer>>>;

    /// Returns the collector manager the search fans out with.
    ///
    /// Equivalent to
    /// `AbstractKnnVectorQuery.getKnnCollectorManager(int, IndexSearcher)`.
    fn get_knn_collector_manager(
        &self,
        k: i32,
        searcher: &IndexSearcher,
    ) -> Arc<dyn KnnCollectorManager> {
        Arc::new(TopKnnCollectorManager::new(k, searcher))
    }

    /// Runs the exact search over one leaf.
    ///
    /// Equivalent to
    /// `AbstractKnnVectorQuery.exactSearch(LeafReaderContext, DocIdSetIterator, QueryTimeout)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while scoring.
    fn exact_search(
        &self,
        context: &LeafReaderContext,
        accept_iterator: Box<dyn DocIdSetIterator>,
        query_timeout: Option<&Arc<dyn QueryTimeout>>,
    ) -> Result<TopDocs> {
        let base = self.knn_base();
        let infos = context.leaf_reader().get_field_infos();
        let Some(field_info) = infos.field_info(base.get_field()) else {
            // The field does not exist.
            return Ok(empty_top_docs());
        };
        if field_info.vector_dimension == 0 {
            // The field does not index vectors.
            return Ok(empty_top_docs());
        }
        let vector_scorer = self.create_vector_scorer(context, field_info)?;
        default_exact_search(base, vector_scorer, accept_iterator, query_timeout)
    }

    /// Merges the segment-level results into the index-level results.
    ///
    /// Equivalent to `AbstractKnnVectorQuery.mergeLeafResults(TopDocs[])`,
    /// which delegates to `TopDocs.merge(k, perLeafResults)` and therefore
    /// requires the inputs to be sorted.
    ///
    /// # Errors
    ///
    /// Propagates the merge error.
    fn merge_leaf_results(&self, per_leaf_results: &[TopDocs]) -> Result<TopDocs> {
        TopDocs::merge(self.knn_base().get_k() as usize, per_leaf_results)
    }
}

/// The scoring half of `AbstractKnnVectorQuery.exactSearch`.
///
/// It is a free function over the base state and an already-created
/// [`VectorScorer`] — rather than over the query — so that
/// [`AbstractKnnVectorQueryImpl::exact_search`] can reach it from its default
/// body without needing `Self: Sized`. The field lookup and the
/// `createVectorScorer` call that precede it in Java live in that default body.
///
/// # Errors
///
/// Propagates any I/O error raised while scoring, and
/// [`LuceneError::IllegalArgument`] when the accept iterator's cost does not
/// fit in an `i32`, which is Java's `Math.toIntExact` throwing
/// `ArithmeticException`.
pub fn default_exact_search(
    base: &AbstractKnnVectorQuery,
    vector_scorer: Option<Box<dyn VectorScorer>>,
    accept_iterator: Box<dyn DocIdSetIterator>,
    query_timeout: Option<&Arc<dyn QueryTimeout>>,
) -> Result<TopDocs> {
    let Some(vector_scorer) = vector_scorer else {
        return Ok(empty_top_docs());
    };

    let cost = accept_iterator.cost();
    let cost_as_int = i32::try_from(cost).map_err(|_| {
        LuceneError::IllegalArgument(format!(
            "integer overflow on the accepted doc count: {cost}"
        ))
    })?;
    let queue_size = base.get_k().min(cost_as_int) as usize;
    let mut queue = HitQueue::new(queue_size, true)?;
    let mut relation = TotalHitsRelation::EQUAL_TO;
    let mut buffer = DocAndFloatFeatureBuffer::new();
    let mut bulk_scorer = vector_scorer.bulk(Some(accept_iterator))?;

    loop {
        let max_score = bulk_scorer.next_docs_and_scores(NO_MORE_DOCS, None, &mut buffer)?;
        if buffer.size == 0 {
            break;
        }
        // Mark the results as partial if the timeout is met.
        if query_timeout.is_some_and(|timeout| timeout.should_exit()) {
            relation = TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO;
            break;
        }
        let top_score = queue.top().map(|top| top.score).unwrap_or(f32::NAN);
        if max_score < top_score {
            // All the scores in this batch are too low, skip.
            continue;
        }
        for i in 0..buffer.size {
            let score = buffer.features[i];
            let doc = buffer.docs[i];
            if queue.top().is_some_and(|top| score > top.score) {
                queue.update_top_with(ScoreDoc::new(doc, score));
            }
        }
    }

    // Remove any remaining sentinel values.
    while queue.size() > 0 && queue.top().is_some_and(|top| top.score < 0.0) {
        queue.pop();
    }

    let mut top_score_docs = vec![ScoreDoc::new(0, 0.0); queue.size()];
    for slot in top_score_docs.iter_mut().rev() {
        *slot = queue
            .pop()
            .expect("INVARIANT: the queue holds exactly as many hits as the array");
    }

    let total_hits = TotalHits::new(cost, relation)?;
    Ok(TopDocs {
        total_hits,
        score_docs: top_score_docs,
    })
}

/// Returns `perLeafTopK`: the expected number of hits (`k * leafProportion`) in
/// a leaf with the given proportion of the whole index, plus three standard
/// deviations of a binomial distribution.
///
/// Equivalent to the private
/// `AbstractKnnVectorQuery.perLeafTopKCalculation(int, float)`. There is a 95%
/// probability that this segment's contribution to the global top `k` hits is
/// at most the returned value.
fn per_leaf_top_k_calculation(k: i32, leaf_proportion: f32) -> i32 {
    let k = k as f64;
    let leaf_proportion = leaf_proportion as f64;
    let value = (k * leaf_proportion
        + LAMBDA * (k * leaf_proportion * (1.0 - leaf_proportion)).sqrt())
    .max(1.0);
    // Java's `(int) double` is a narrowing conversion that truncates towards
    // zero and maps NaN to 0.
    if value.is_nan() {
        0
    } else if value >= i32::MAX as f64 {
        i32::MAX
    } else if value <= i32::MIN as f64 {
        i32::MIN
    } else {
        value as i32
    }
}

/// A manager that asks its delegate for a per-leaf, pro-rata `k`.
///
/// Equivalent to the static nested
/// `AbstractKnnVectorQuery.OptimisticKnnCollectorManager`.
#[derive(Debug)]
pub struct OptimisticKnnCollectorManager {
    k: i32,
    delegate: Arc<dyn KnnCollectorManager>,
}

impl OptimisticKnnCollectorManager {
    /// Wraps `delegate`, scaling `k` per leaf.
    ///
    /// Equivalent to
    /// `new AbstractKnnVectorQuery.OptimisticKnnCollectorManager(int, KnnCollectorManager)`.
    pub fn new(k: i32, delegate: Arc<dyn KnnCollectorManager>) -> Self {
        Self { k, delegate }
    }
}

impl KnnCollectorManager for OptimisticKnnCollectorManager {
    fn new_collector(
        &self,
        visited_limit: i32,
        search_strategy: Option<&KnnSearchStrategy>,
        context: &LeafReaderContext,
    ) -> Result<Box<dyn KnnCollector>> {
        // The delegate supports optimistic collection; `context.parent` can be
        // absent if this is a memory index.
        if self.delegate.is_optimistic() {
            if let Some(parent_max_doc) = parent_max_doc(context) {
                let leaf_proportion =
                    context.leaf_reader().max_doc() as f32 / parent_max_doc as f32;
                let per_leaf_top_k = per_leaf_top_k_calculation(self.k, leaf_proportion);
                // If we divided by zero above, leaf_proportion can be NaN and
                // then this would be 0.
                debug_assert!(per_leaf_top_k > 0);
                if let Some(collector) = self.delegate.new_optimistic_collector(
                    visited_limit,
                    search_strategy,
                    context,
                    per_leaf_top_k,
                )? {
                    return Ok(collector);
                }
            }
        }
        // We do not support optimistic collection, so just do the regular
        // execution path.
        self.delegate
            .new_collector(visited_limit, search_strategy, context)
    }
}

/// Returns `ctx.parent.reader().maxDoc()`, or `None` when the leaf has no
/// parent — which happens for a memory index.
fn parent_max_doc(context: &LeafReaderContext) -> Option<i32> {
    use crate::index::IndexReaderContext;
    let parent = IndexReaderContext::parent(context)?;
    let parent = parent.upgrade()?;
    Some(parent.reader().max_doc())
}

/// Rewrites a kNN vector query into the hits it matches.
///
/// Equivalent to `AbstractKnnVectorQuery.rewrite(IndexSearcher)`.
///
/// The query first executes its filter for each leaf, then chooses a strategy
/// dynamically:
///
/// * if the filter cost is less than `k`, run an exact search;
/// * otherwise run a kNN search subject to the filter;
/// * if the kNN search visits too many vectors without completing, stop and run
///   an exact search.
///
/// # Errors
///
/// Propagates any I/O error raised while filtering, searching or merging.
pub fn knn_vector_query_rewrite(
    query: Arc<dyn AbstractKnnVectorQueryImpl>,
    searcher: &IndexSearcher,
) -> Result<Option<Arc<dyn Query>>> {
    let base = query.knn_base().clone();

    let filter_weight: Option<Arc<dyn Weight>> = match base.get_filter() {
        None => None,
        Some(filter) => {
            // Rewrite the inner filter query first, to find out whether it is a
            // match-all or a match-no-docs query, so that we can skip the kNN
            // search.
            let rewritten_filter = filter
                .rewrite(searcher)?
                .unwrap_or_else(|| Arc::clone(filter));
            if rewritten_filter.as_any().is::<MatchNoDocsQuery>() {
                // If the filter is a match-no-docs query, we can also skip it.
                return Ok(Some(rewritten_filter));
            }
            if rewritten_filter.as_any().is::<MatchAllDocsQuery>() {
                // If the filter is a match-all-docs query, we can skip it.
                None
            } else {
                let mut builder = BooleanQueryBuilder::new();
                builder.add(Arc::clone(filter), Occur::FILTER)?;
                builder.add(
                    Arc::new(FieldExistsQuery::new(base.get_field())),
                    Occur::FILTER,
                )?;
                let boolean_query: Arc<dyn Query> = Arc::new(builder.build());
                let rewritten = searcher.rewrite(boolean_query)?;
                if rewritten.as_any().is::<MatchNoDocsQuery>() {
                    return Ok(Some(rewritten));
                }
                Some(rewritten.create_weight(searcher, ScoreMode::COMPLETE_NO_SCORES, 1.0)?)
            }
        }
    };

    let knn_collector_manager = query.get_knn_collector_manager(base.get_k(), searcher);
    let is_optimistic = knn_collector_manager.is_optimistic();
    let optimistic_collector_manager: Arc<dyn KnnCollectorManager> = Arc::new(
        OptimisticKnnCollectorManager::new(base.get_k(), knn_collector_manager),
    );
    let time_limiting_knn_collector_manager = Arc::new(TimeLimitingKnnCollectorManager::new(
        optimistic_collector_manager,
        searcher.get_timeout().cloned(),
    ));

    let mut leaf_reader_contexts: Vec<Arc<LeafReaderContext>> =
        searcher.get_leaf_contexts().to_vec();
    let mut tasks = build_tasks(
        &query,
        &leaf_reader_contexts,
        &filter_weight,
        &time_limiting_knn_collector_manager,
    );

    // Equivalent to Java's `Map<Integer, TopDocs> perLeafResults`, a `HashMap`
    // over the leaf ordinals. It is a `BTreeMap` here so that
    // `mergeLeafResults` sees the leaves in the same ascending-ordinal order a
    // Java `HashMap` keyed by `0..leaves-1` iterates in; the order decides the
    // shard index a merged hit is tagged with, and therefore how ties break.
    let mut per_leaf_results: BTreeMap<i32, TopDocs> = BTreeMap::new();
    let mut top_k = run_search_tasks(
        &query,
        tasks,
        searcher,
        &mut per_leaf_results,
        &leaf_reader_contexts,
    )?;

    if !top_k.score_docs.is_empty()
        && per_leaf_results.len() > 1
        // Only re-enter if we used optimistic collection.
        && is_optimistic
        // Do not re-enter the search if we terminated early.
        && top_k.total_hits.relation() == TotalHitsRelation::EQUAL_TO
    {
        let min_top_k_score = top_k.score_docs[top_k.score_docs.len() - 1].score;
        let reentrant: Arc<dyn KnnCollectorManager> = Arc::new(ReentrantKnnCollectorManager::new(
            query.get_knn_collector_manager(base.get_k(), searcher),
            Arc::clone(&query),
            per_leaf_results.clone(),
        ));
        let knn_collector_manager_phase2 = Arc::new(TimeLimitingKnnCollectorManager::new(
            reentrant,
            searcher.get_timeout().cloned(),
        ));

        let mut active_contexts = Vec::with_capacity(leaf_reader_contexts.len());
        for ctx in leaf_reader_contexts {
            let per_leaf = per_leaf_results
                .get(&ctx.ord())
                .expect("INVARIANT: every leaf was searched in the first phase");
            if !per_leaf.score_docs.is_empty()
                && per_leaf.score_docs[per_leaf.score_docs.len() - 1].score >= min_top_k_score
            {
                // All this leaf's hits are at or above the global top-k minimum
                // score; explore it further.
                active_contexts.push(ctx);
            }
            // Otherwise this leaf is tapped out; drop the context from the
            // active list so that tasks and leaves stay in correspondence.
        }
        leaf_reader_contexts = active_contexts;
        tasks = build_tasks(
            &query,
            &leaf_reader_contexts,
            &filter_weight,
            &knn_collector_manager_phase2,
        );
        top_k = run_search_tasks(
            &query,
            tasks,
            searcher,
            &mut per_leaf_results,
            &leaf_reader_contexts,
        )?;
    }

    if top_k.score_docs.is_empty() {
        return Ok(Some(Arc::new(MatchNoDocsQuery::instance())));
    }
    Ok(Some(DocAndScoreQuery::create_doc_and_score_query(
        searcher, top_k,
    )?))
}

/// One per-leaf search task.
type LeafTask = Box<dyn FnOnce() -> Result<TopDocs> + Send + 'static>;

/// Builds one [`search_leaf`] task per leaf.
///
/// Equivalent to the loop that fills Java's `List<Callable<TopDocs>> tasks`.
fn build_tasks(
    query: &Arc<dyn AbstractKnnVectorQueryImpl>,
    leaf_reader_contexts: &[Arc<LeafReaderContext>],
    filter_weight: &Option<Arc<dyn Weight>>,
    manager: &Arc<TimeLimitingKnnCollectorManager>,
) -> Vec<LeafTask> {
    leaf_reader_contexts
        .iter()
        .map(|context| {
            let query = Arc::clone(query);
            let context = Arc::clone(context);
            let filter_weight = filter_weight.clone();
            let manager = Arc::clone(manager);
            let task: LeafTask = Box::new(move || {
                search_leaf(
                    query.as_ref(),
                    context.as_ref(),
                    filter_weight.as_ref(),
                    manager.as_ref(),
                )
            });
            task
        })
        .collect()
}

/// Runs the per-leaf tasks, records their results and merges them.
///
/// Equivalent to the private
/// `AbstractKnnVectorQuery.runSearchTasks(List, TaskExecutor, Map, List)`.
fn run_search_tasks(
    query: &Arc<dyn AbstractKnnVectorQueryImpl>,
    tasks: Vec<LeafTask>,
    searcher: &IndexSearcher,
    per_leaf_results: &mut BTreeMap<i32, TopDocs>,
    leaf_reader_contexts: &[Arc<LeafReaderContext>],
) -> Result<TopDocs> {
    let task_results = searcher.get_task_executor().invoke_all(tasks)?;
    for (i, result) in task_results.into_iter().enumerate() {
        per_leaf_results.insert(leaf_reader_contexts[i].ord(), result);
    }
    // Merge-sort the results.
    let merged: Vec<TopDocs> = per_leaf_results.values().cloned().collect();
    query.merge_leaf_results(&merged)
}

/// Searches one leaf and re-bases its hits into the top-level doc ID space.
///
/// Equivalent to
/// `AbstractKnnVectorQuery.searchLeaf(LeafReaderContext, Weight, TimeLimitingKnnCollectorManager)`.
///
/// # Errors
///
/// Propagates any I/O error raised while searching.
pub fn search_leaf(
    query: &dyn AbstractKnnVectorQueryImpl,
    ctx: &LeafReaderContext,
    filter_weight: Option<&Arc<dyn Weight>>,
    time_limiting_knn_collector_manager: &TimeLimitingKnnCollectorManager,
) -> Result<TopDocs> {
    let mut results = get_leaf_results(
        query,
        ctx,
        filter_weight,
        time_limiting_knn_collector_manager,
    )?;
    if ctx.doc_base() > 0 {
        for score_doc in &mut results.score_docs {
            score_doc.doc += ctx.doc_base();
        }
    }
    Ok(results)
}

/// Chooses between the approximate and the exact search for one leaf.
///
/// Equivalent to the private
/// `AbstractKnnVectorQuery.getLeafResults(LeafReaderContext, Weight, TimeLimitingKnnCollectorManager)`.
fn get_leaf_results(
    query: &dyn AbstractKnnVectorQueryImpl,
    ctx: &LeafReaderContext,
    filter_weight: Option<&Arc<dyn Weight>>,
    time_limiting_knn_collector_manager: &TimeLimitingKnnCollectorManager,
) -> Result<TopDocs> {
    let base = query.knn_base();
    let reader = ctx.leaf_reader();
    let live_docs = reader.get_live_docs();
    let max_doc = reader.max_doc();

    let Some(filter_weight) = filter_weight else {
        let mut accept_docs = from_live_docs(live_docs, max_doc)?;
        return query.approximate_search(
            ctx,
            &mut accept_docs,
            i32::MAX,
            time_limiting_knn_collector_manager,
        );
    };

    let supplier = || -> Result<Box<dyn DocIdSetIterator>> {
        match filter_weight.scorer(ctx)? {
            None => Ok(Box::new(empty())),
            Some(scorer) => Ok(into_scorer_iterator(scorer).into_doc_id_set_iterator()),
        }
    };
    let mut accept_docs = from_iterator_supplier(supplier, live_docs, max_doc)?;
    let cost = accept_docs.cost()?;
    let query_timeout = time_limiting_knn_collector_manager
        .get_query_timeout()
        .cloned();

    // `ctx.parent` can be absent if this is a memory index; we have no good way
    // to estimate per_leaf_top_k then, so just do an approximate search.
    let per_leaf_top_k = match parent_max_doc(ctx) {
        Some(parent_max_doc) => {
            let leaf_proportion = max_doc as f32 / parent_max_doc as f32;
            per_leaf_top_k_calculation(base.get_k(), leaf_proportion)
        }
        None => base.get_k(),
    };

    if cost <= per_leaf_top_k as i64 {
        // If there are at most per_leaf_top_k possible matches, short-circuit
        // and perform an exact search, since HNSW must always visit at least
        // per_leaf_top_k documents.
        return query.exact_search(ctx, accept_docs.iterator()?, query_timeout.as_ref());
    }

    // Perform the approximate kNN search. We pass cost + 1 here to account for
    // the edge case where we explore exactly `cost` vectors.
    let visited_limit = i32::try_from(cost.saturating_add(1)).unwrap_or(i32::MAX);
    let results = query.approximate_search(
        ctx,
        &mut accept_docs,
        visited_limit,
        time_limiting_knn_collector_manager,
    )?;

    if (results.total_hits.relation() == TotalHitsRelation::EQUAL_TO
        // We know that there are more than per_leaf_top_k available docs; if we
        // did not even get per_leaf_top_k something weird happened, and we need
        // to drop to exact search.
        && results.score_docs.len() >= per_leaf_top_k as usize)
        // Return partial results only when the timeout is met.
        || query_timeout
            .as_ref()
            .is_some_and(|timeout| timeout.should_exit())
    {
        Ok(results)
    } else {
        // We stopped the kNN search because it visited too many nodes, so fall
        // back to exact search.
        query.exact_search(ctx, accept_docs.iterator()?, query_timeout.as_ref())
    }
}

/// A manager that seeds the second search phase with the hits of the first.
///
/// Equivalent to the inner class
/// `AbstractKnnVectorQuery.ReentrantKnnCollectorManager`, itself forked from
/// `SeededKnnVectorQuery.SeededCollectorManager`.
#[derive(Debug)]
struct ReentrantKnnCollectorManager {
    knn_collector_manager: Arc<dyn KnnCollectorManager>,
    query: Arc<dyn AbstractKnnVectorQueryImpl>,
    per_leaf_results: BTreeMap<i32, TopDocs>,
}

impl ReentrantKnnCollectorManager {
    fn new(
        knn_collector_manager: Arc<dyn KnnCollectorManager>,
        query: Arc<dyn AbstractKnnVectorQueryImpl>,
        per_leaf_results: BTreeMap<i32, TopDocs>,
    ) -> Self {
        Self {
            knn_collector_manager,
            query,
            per_leaf_results,
        }
    }
}

impl KnnCollectorManager for ReentrantKnnCollectorManager {
    fn new_collector(
        &self,
        visit_limit: i32,
        search_strategy: Option<&KnnSearchStrategy>,
        ctx: &LeafReaderContext,
    ) -> Result<Box<dyn KnnCollector>> {
        let delegate_collector =
            self.knn_collector_manager
                .new_collector(visit_limit, search_strategy, ctx)?;
        let Some(seed_top_docs) = self.per_leaf_results.get(&ctx.ord()) else {
            return Ok(delegate_collector);
        };
        let base = self.query.knn_base();
        let infos = ctx.leaf_reader().get_field_infos();
        let scorer = match infos.field_info(base.get_field()) {
            Some(field_info) => self.query.create_vector_scorer(ctx, field_info)?,
            None => None,
        };
        let Some(scorer) = scorer else {
            // Should not happen: we only come here when there are results. It
            // is safe to return no seeds.
            return Ok(delegate_collector);
        };
        if seed_top_docs.total_hits.value() == 0 {
            return Ok(delegate_collector);
        }
        let Some(seed_docs) = crate::search::seeded_knn_vector_query::mapped_seed_ordinals(
            scorer,
            seed_top_docs,
            ctx,
        )?
        else {
            // The vector iterator is not indexed, so the seed docs cannot be
            // mapped onto vector ordinals; continuing would loop forever.
            return Ok(delegate_collector);
        };
        let strategy = KnnSearchStrategy::Seeded(crate::search::knn::Seeded::new(
            Some(seed_docs),
            seed_top_docs.score_docs.len() as i32,
            search_strategy
                .cloned()
                .unwrap_or_else(KnnSearchStrategy::hnsw_default),
        )?);
        self.knn_collector_manager
            .new_collector(visit_limit, Some(&strategy), ctx)
    }
}

/// Visits a kNN vector query.
///
/// Equivalent to `AbstractKnnVectorQuery.visit(QueryVisitor)`.
pub fn knn_vector_query_visit(
    base: &AbstractKnnVectorQuery,
    query: &dyn Query,
    visitor: &mut dyn crate::search::query_visitor::QueryVisitor,
) {
    if visitor.accept_field(base.get_field()) {
        visitor.visit_leaf(query);
    }
}
