//! Search entry point, ported from `org.apache.lucene.search.IndexSearcher`.

#![deny(unsafe_code)]

use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, LazyLock, OnceLock, RwLock};

use crate::error::{LuceneError, Result};
use crate::index::{
    sub_index_from_leaves, IndexReader, IndexReaderContext, LeafReaderContext, QueryTimeout,
    StoredFields, Term,
};
use crate::search::boolean_clause::Occur;
use crate::search::collection_terminated_exception::CollectionError;
use crate::search::collector::{Collector, CollectorManager};
use crate::search::doc_id_set_iterator::NO_MORE_DOCS;
use crate::search::query::Query;
use crate::search::query_cache::{QueryCache, QueryCachingPolicy};
use crate::search::query_visitor::QueryVisitor;
use crate::search::score_doc::ScoreDoc;
use crate::search::score_mode::ScoreMode;
use crate::search::scorer_util::ScorerUtil;
use crate::search::similarities::{
    BM25Similarity, CollectionStatistics, Explanation, Similarity, TermStatistics,
};
use crate::search::task_executor::{Executor, TaskExecutor};
use crate::search::time_limiting_bulk_scorer::TimeLimitingBulkScorer;
use crate::search::top_docs::TopDocs;
use crate::search::top_score_doc_collector_manager::TopScoreDocCollectorManager;
use crate::search::total_hit_count_collector::TotalHitCountCollectorManager;
use crate::search::weight::Weight;
use crate::util::ByteRunAutomaton;

/// The maximum number of clauses permitted per query.
///
/// Equivalent to the mutable static `IndexSearcher.maxClauseCount` field,
/// which starts at 1024.
static MAX_CLAUSE_COUNT: AtomicI32 = AtomicI32::new(1024);

/// The process-wide default query cache.
///
/// Equivalent to the mutable static `IndexSearcher.DEFAULT_QUERY_CACHE`, which
/// Java seeds with an `LRUQueryCache` holding up to 1000 queries or 5% of the
/// heap. That cache is not part of the query-execution spine and is not ported
/// yet, so the default is "no cache"; see [`crate::search::query_cache`].
static DEFAULT_QUERY_CACHE: LazyLock<RwLock<Option<Arc<dyn QueryCache>>>> =
    LazyLock::new(|| RwLock::new(None));

/// The process-wide default query caching policy.
///
/// Equivalent to the mutable static `IndexSearcher.DEFAULT_CACHING_POLICY`,
/// which Java seeds with a `UsageTrackingQueryCachingPolicy`. That policy is
/// not ported yet, and it would never be consulted anyway while
/// [`DEFAULT_QUERY_CACHE`] is absent.
static DEFAULT_CACHING_POLICY: LazyLock<RwLock<Option<Arc<dyn QueryCachingPolicy>>>> =
    LazyLock::new(|| RwLock::new(None));

/// The default similarity, shared by every searcher that does not set its own.
///
/// Equivalent to the `private static final Similarity defaultSimilarity`
/// field.
static DEFAULT_SIMILARITY: LazyLock<Arc<dyn Similarity>> =
    LazyLock::new(|| Arc::new(BM25Similarity::new()));

/// Hit counts are accurate up to this many hits by default, so that most of the
/// time is not spent computing them.
///
/// Equivalent to `IndexSearcher.TOTAL_HITS_THRESHOLD`.
const TOTAL_HITS_THRESHOLD: i32 = 1000;

/// Threshold for the index-slice allocation logic.
///
/// Equivalent to `IndexSearcher.MAX_DOCS_PER_SLICE`.
const MAX_DOCS_PER_SLICE: i32 = 250_000;

/// Threshold for the index-slice allocation logic.
///
/// Equivalent to `IndexSearcher.MAX_SEGMENTS_PER_SLICE`.
const MAX_SEGMENTS_PER_SLICE: usize = 5;

/// Raised when an attempt is made to add more than
/// [`IndexSearcher::get_max_clause_count`] clauses.
///
/// Equivalent to `org.apache.lucene.search.IndexSearcher.TooManyClauses`, a
/// `RuntimeException`. This typically happens when a prefix, fuzzy, wildcard or
/// term-range query is expanded to many terms during search.
///
/// **Divergence from Lucene 10.5.0.** Rust has no exceptions, so this is a
/// value. Where Java throws it, this port returns
/// [`LuceneError::ResourceLimit`] carrying [`Self::to_string`]; the type is
/// still available so that callers can build the same message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TooManyClauses {
    message: String,
    max_clause_count: i32,
}

impl Default for TooManyClauses {
    fn default() -> Self {
        Self::new()
    }
}

impl TooManyClauses {
    /// Builds the error with Java's default message.
    ///
    /// Equivalent to `new TooManyClauses()`.
    pub fn new() -> Self {
        let max_clause_count = IndexSearcher::get_max_clause_count();
        Self {
            message: format!("maxClauseCount is set to {max_clause_count}"),
            max_clause_count,
        }
    }

    /// Builds the error with a custom message.
    ///
    /// Equivalent to `new TooManyClauses(String)`.
    pub fn with_message(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            max_clause_count: IndexSearcher::get_max_clause_count(),
        }
    }

    /// The value of [`IndexSearcher::get_max_clause_count`] when this error was
    /// created.
    ///
    /// Equivalent to `TooManyClauses.getMaxClauseCount()`.
    pub fn get_max_clause_count(&self) -> i32 {
        self.max_clause_count
    }
}

impl fmt::Display for TooManyClauses {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for TooManyClauses {}

impl From<TooManyClauses> for LuceneError {
    fn from(err: TooManyClauses) -> Self {
        LuceneError::ResourceLimit(err.to_string())
    }
}

/// Raised when a query has more than [`IndexSearcher::get_max_clause_count`]
/// clauses cumulatively across all of its children.
///
/// Equivalent to
/// `org.apache.lucene.search.IndexSearcher.TooManyNestedClauses`, which
/// `extends TooManyClauses` in Java. Rust has no inheritance, so this type
/// holds the parent error rather than deriving from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TooManyNestedClauses {
    inner: TooManyClauses,
}

impl Default for TooManyNestedClauses {
    fn default() -> Self {
        Self::new()
    }
}

impl TooManyNestedClauses {
    /// Builds the error with Java's message.
    ///
    /// Equivalent to `new TooManyNestedClauses()`.
    pub fn new() -> Self {
        let max_clause_count = IndexSearcher::get_max_clause_count();
        Self {
            inner: TooManyClauses::with_message(format!(
                "Query contains too many nested clauses; maxClauseCount is set to {max_clause_count}"
            )),
        }
    }

    /// This error viewed as its parent type.
    ///
    /// The Rust stand-in for Java's `extends TooManyClauses`.
    pub fn as_too_many_clauses(&self) -> &TooManyClauses {
        &self.inner
    }
}

impl fmt::Display for TooManyNestedClauses {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.inner, f)
    }
}

impl std::error::Error for TooManyNestedClauses {}

impl From<TooManyNestedClauses> for LuceneError {
    fn from(err: TooManyNestedClauses) -> Self {
        LuceneError::ResourceLimit(err.to_string())
    }
}

/// Holds a specific leaf context and the range of doc IDs to search within it.
///
/// Equivalent to the `final
/// org.apache.lucene.search.IndexSearcher.LeafReaderContextPartition`, used to
/// optionally search across partitions of the same segment concurrently.
///
/// Build one with [`create_for_entire_segment`](Self::create_for_entire_segment)
/// to target the whole leaf, or with
/// [`create_from_and_to`](Self::create_from_and_to) for a true partition.
#[derive(Debug, Clone)]
pub struct LeafReaderContextPartition {
    /// The lowest doc ID to search, included.
    pub min_doc_id: i32,
    /// The highest doc ID to search, excluded.
    pub max_doc_id: i32,
    /// The leaf being searched.
    pub ctx: Arc<LeafReaderContext>,
    /// The number of docs this partition targets.
    ///
    /// Tracked separately because [`NO_MORE_DOCS`] is used as the upper bound
    /// when the partition targets the entire segment.
    max_docs: i32,
}

impl LeafReaderContextPartition {
    fn new(
        leaf_reader_context: Arc<LeafReaderContext>,
        min_doc_id: i32,
        max_doc_id: i32,
        max_docs: i32,
    ) -> Result<Self> {
        if min_doc_id >= max_doc_id {
            return Err(LuceneError::IllegalArgument(format!(
                "minDocId is greater than or equal to maxDocId: [{min_doc_id}] > [{max_doc_id}]"
            )));
        }
        if min_doc_id < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "minDocId is lower than 0: [{min_doc_id}]"
            )));
        }
        let leaf_max_doc = leaf_reader_context.leaf_reader().max_doc();
        if min_doc_id >= leaf_max_doc {
            return Err(LuceneError::IllegalArgument(format!(
                "minDocId is greater than than maxDoc: [{min_doc_id}] > [{leaf_max_doc}]"
            )));
        }

        Ok(Self {
            ctx: leaf_reader_context,
            min_doc_id,
            max_doc_id,
            max_docs,
        })
    }

    /// Creates a partition of the provided leaf context targeting the entire
    /// segment.
    ///
    /// Equivalent to
    /// `LeafReaderContextPartition.createForEntireSegment(LeafReaderContext)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] for an empty leaf, which Java
    /// rejects with the same `minDocId >= maxDocId` check.
    pub fn create_for_entire_segment(ctx: Arc<LeafReaderContext>) -> Result<Self> {
        let max_docs = ctx.leaf_reader().max_doc();
        Self::new(ctx, 0, NO_MORE_DOCS, max_docs)
    }

    /// Creates a partition of the provided leaf context targeting a subset of
    /// the segment, from `min_doc_id` included to `max_doc_id` excluded.
    ///
    /// Equivalent to
    /// `LeafReaderContextPartition.createFromAndTo(LeafReaderContext, int, int)`.
    ///
    /// # Errors
    ///
    /// As [`create_for_entire_segment`](Self::create_for_entire_segment).
    pub fn create_from_and_to(
        ctx: Arc<LeafReaderContext>,
        min_doc_id: i32,
        max_doc_id: i32,
    ) -> Result<Self> {
        debug_assert!(max_doc_id != NO_MORE_DOCS);
        Self::new(ctx, min_doc_id, max_doc_id, max_doc_id - min_doc_id)
    }

    /// The number of docs this partition targets.
    pub fn max_docs(&self) -> i32 {
        self.max_docs
    }
}

/// A subset of an [`IndexSearcher`]'s leaf contexts, to be executed within a
/// single thread.
///
/// Equivalent to `org.apache.lucene.search.IndexSearcher.LeafSlice`. A slice
/// holds one or more [`LeafReaderContextPartition`]s, each targeting a specific
/// doc ID range of a leaf.
#[derive(Debug, Clone)]
pub struct LeafSlice {
    partitions: Vec<LeafReaderContextPartition>,
    max_docs: i32,
}

impl LeafSlice {
    /// Creates a slice over the given partitions, ordering them by doc base and
    /// then by lowest doc ID.
    ///
    /// Equivalent to `new LeafSlice(List<LeafReaderContextPartition>)`, which
    /// sorts with `LeafSlice.COMPARATOR`.
    pub fn new(mut partitions: Vec<LeafReaderContextPartition>) -> Self {
        partitions.sort_by(|a, b| {
            a.ctx
                .doc_base()
                .cmp(&b.ctx.doc_base())
                .then_with(|| a.min_doc_id.cmp(&b.min_doc_id))
        });
        let max_docs = partitions
            .iter()
            .fold(0i32, |acc, partition| acc.wrapping_add(partition.max_docs));
        Self {
            partitions,
            max_docs,
        }
    }

    /// Creates a slice targeting each of the given leaves entirely.
    ///
    /// Equivalent to the private
    /// `LeafSlice.entireSegments(List<LeafReaderContext>)`.
    ///
    /// # Errors
    ///
    /// Propagates the partition construction error for an empty leaf.
    pub fn entire_segments(contexts: &[Arc<LeafReaderContext>]) -> Result<Self> {
        let mut parts = Vec::with_capacity(contexts.len());
        for ctx in contexts {
            parts.push(LeafReaderContextPartition::create_for_entire_segment(
                Arc::clone(ctx),
            )?);
        }
        Ok(Self::new(parts))
    }

    /// The leaf partitions that make up this slice.
    ///
    /// Equivalent to the `public final LeafReaderContextPartition[] partitions`
    /// field.
    pub fn partitions(&self) -> &[LeafReaderContextPartition] {
        &self.partitions
    }

    /// The total number of docs this slice targets, the sum over its
    /// partitions.
    ///
    /// Equivalent to `LeafSlice.getMaxDocs()`.
    pub fn get_max_docs(&self) -> i32 {
        self.max_docs
    }
}

/// Counts the clauses of a query tree and reports when the total exceeds
/// [`IndexSearcher::get_max_clause_count`].
///
/// Equivalent to the anonymous visitor returned by the private
/// `IndexSearcher.getNumClausesCheckVisitor()`.
///
/// **Divergence from Lucene 10.5.0.** Java aborts the traversal by throwing
/// `TooManyNestedClauses` from the visiting methods. A Rust visitor method
/// cannot fail, so the violation is recorded and checked once the traversal
/// finishes; the outcome is the same error, only reached after visiting the
/// remaining nodes.
#[derive(Debug, Default)]
struct NumClausesCheckVisitor {
    num_clauses: i32,
    exceeded: bool,
}

impl NumClausesCheckVisitor {
    fn count_one(&mut self) {
        if self.num_clauses > IndexSearcher::get_max_clause_count() {
            self.exceeded = true;
            return;
        }
        self.num_clauses += 1;
    }

    fn check(&self) -> Result<()> {
        if self.exceeded {
            return Err(TooManyNestedClauses::new().into());
        }
        Ok(())
    }
}

impl QueryVisitor for NumClausesCheckVisitor {
    fn get_sub_visitor<'a>(
        &'a mut self,
        _occur: Occur,
        _parent: &dyn Query,
    ) -> Box<dyn QueryVisitor + 'a>
    where
        Self: 'a,
    {
        // Return this instance even for MUST_NOT, and not the empty visitor.
        Box::new(self)
    }

    fn visit_leaf(&mut self, _query: &dyn Query) {
        self.count_one();
    }

    fn consume_terms(&mut self, _query: &dyn Query, _terms: &[Term]) {
        self.count_one();
    }

    fn consume_terms_matching(
        &mut self,
        _query: &dyn Query,
        _field: &str,
        _automaton: &dyn Fn() -> ByteRunAutomaton,
    ) {
        self.count_one();
    }
}

/// Implements search over a single [`IndexReader`].
///
/// Equivalent to `org.apache.lucene.search.IndexSearcher`.
///
/// Applications usually only need [`search`](Self::search). For performance
/// reasons, if the index is unchanging, a single searcher should be shared
/// across multiple searches instead of creating one per search. When the index
/// has changed, obtain a new reader with
/// [`open_if_changed`](crate::index::open_if_changed) and create a new searcher
/// from it, which is relatively cheap.
///
/// [`search`](Self::search) and [`search_after`](Self::search_after) only count
/// hits accurately up to 1000 and may return a
/// [lower bound](crate::search::TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO) of
/// the hit count beyond that. On queries matching many documents, counting hits
/// may take much longer than computing the top hits, so this trade-off gives
/// some information about the hit count without slowing search down too much.
/// The hits themselves are always accurate. Applications that need an exact
/// count should build a collector manager and call
/// [`search_with_manager`](Self::search_with_manager).
///
/// **Divergence from Lucene 10.5.0.** Java's class is designed for subclassing:
/// `slices`, `searchLeaf`, `explain`, `termStatistics` and
/// `collectionStatistics` are `protected` and meant to be overridden — for
/// instance to return statistics across a distributed collection. Rust has no
/// implementation inheritance, so those are inherent methods here and the
/// customisation points they offer are not available; a caller that needs them
/// composes an `IndexSearcher` rather than extending it. The setters, which
/// Java documents as "call before starting to use this searcher", take
/// `&mut self` for the same reason no lock is needed around them.
pub struct IndexSearcher {
    reader: Arc<dyn IndexReader>,
    reader_context: Arc<dyn IndexReaderContext>,
    leaf_contexts: Vec<Arc<LeafReaderContext>>,
    leaf_slices: OnceLock<std::result::Result<Vec<LeafSlice>, String>>,
    /// Used internally to load-balance the threads executing a query.
    task_executor: Arc<TaskExecutor>,
    similarity: Arc<dyn Similarity>,
    query_cache: Option<Arc<dyn QueryCache>>,
    query_caching_policy: Option<Arc<dyn QueryCachingPolicy>>,
    query_timeout: Option<Arc<dyn QueryTimeout>>,
    /// Set on whichever thread ran past the timeout.
    partial_result: Arc<AtomicBool>,
}

impl fmt::Debug for IndexSearcher {
    /// Renders as `IndexSearcher.toString()` does.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "IndexSearcher({:?}; taskExecutor={:?})",
            self.reader, self.task_executor
        )
    }
}

impl IndexSearcher {
    /// Creates a searcher over the provided index.
    ///
    /// Equivalent to `new IndexSearcher(IndexReader)`.
    ///
    /// # Errors
    ///
    /// Propagates the slice construction error for an empty leaf.
    pub fn new(reader: Arc<dyn IndexReader>) -> Result<Self> {
        Self::with_executor(reader, None)
    }

    /// Creates a searcher running searches for each slice on the provided
    /// executor.
    ///
    /// Equivalent to `new IndexSearcher(IndexReader, Executor)`.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn with_executor(
        reader: Arc<dyn IndexReader>,
        executor: Option<Arc<dyn Executor>>,
    ) -> Result<Self> {
        let context = Arc::clone(&reader).get_context();
        Self::from_context(context, executor)
    }

    /// Creates a searcher over the provided top-level reader context.
    ///
    /// Equivalent to `new IndexSearcher(IndexReaderContext, Executor)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the context is not
    /// top-level, which Java asserts, and propagates the slice construction
    /// error for an empty leaf.
    pub fn from_context(
        context: Arc<dyn IndexReaderContext>,
        executor: Option<Arc<dyn Executor>>,
    ) -> Result<Self> {
        if !context.is_top_level() {
            return Err(LuceneError::IllegalArgument(format!(
                "IndexSearcher's ReaderContext must be topLevel for reader {:?}",
                context.reader()
            )));
        }
        let reader = context.reader();
        let has_executor = executor.is_some();
        let task_executor = match executor {
            Some(executor) => Arc::new(TaskExecutor::new(executor)),
            None => Arc::new(TaskExecutor::same_thread()),
        };
        let leaf_contexts = Arc::clone(&context).leaves();

        let leaf_slices = OnceLock::new();
        if !has_executor {
            let slices = if leaf_contexts.is_empty() {
                Vec::new()
            } else {
                vec![LeafSlice::entire_segments(&leaf_contexts)?]
            };
            let _ = leaf_slices.set(Ok(slices));
        }

        Ok(Self {
            reader,
            reader_context: context,
            leaf_contexts,
            leaf_slices,
            task_executor,
            similarity: Arc::clone(&DEFAULT_SIMILARITY),
            query_cache: DEFAULT_QUERY_CACHE
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone(),
            query_caching_policy: DEFAULT_CACHING_POLICY
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone(),
            query_timeout: None,
            partial_result: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Returns the default similarity instance.
    ///
    /// Equivalent to `IndexSearcher.getDefaultSimilarity()`. In general this is
    /// only called to initialise searchers and writers; user code and query
    /// implementations should respect [`get_similarity`](Self::get_similarity).
    pub fn get_default_similarity() -> Arc<dyn Similarity> {
        Arc::clone(&DEFAULT_SIMILARITY)
    }

    /// Returns the leaf contexts associated with this searcher.
    ///
    /// Equivalent to `IndexSearcher.getLeafContexts()`.
    pub fn get_leaf_contexts(&self) -> &[Arc<LeafReaderContext>] {
        &self.leaf_contexts
    }

    /// Returns the default query cache, or `None` if caching is disabled.
    ///
    /// Equivalent to `IndexSearcher.getDefaultQueryCache()`.
    pub fn get_default_query_cache() -> Option<Arc<dyn QueryCache>> {
        DEFAULT_QUERY_CACHE
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Sets the default query cache.
    ///
    /// Equivalent to `IndexSearcher.setDefaultQueryCache(QueryCache)`.
    pub fn set_default_query_cache(default_query_cache: Option<Arc<dyn QueryCache>>) {
        *DEFAULT_QUERY_CACHE
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = default_query_cache;
    }

    /// Returns the default query caching policy.
    ///
    /// Equivalent to `IndexSearcher.getDefaultQueryCachingPolicy()`.
    pub fn get_default_query_caching_policy() -> Option<Arc<dyn QueryCachingPolicy>> {
        DEFAULT_CACHING_POLICY
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Sets the default query caching policy.
    ///
    /// Equivalent to
    /// `IndexSearcher.setDefaultQueryCachingPolicy(QueryCachingPolicy)`.
    pub fn set_default_query_caching_policy(policy: Option<Arc<dyn QueryCachingPolicy>>) {
        *DEFAULT_CACHING_POLICY
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = policy;
    }

    /// Returns the maximum number of clauses permitted, 1024 by default.
    ///
    /// Equivalent to `IndexSearcher.getMaxClauseCount()`.
    pub fn get_max_clause_count() -> i32 {
        MAX_CLAUSE_COUNT.load(Ordering::Relaxed)
    }

    /// Sets the maximum number of clauses permitted per query.
    ///
    /// Equivalent to `IndexSearcher.setMaxClauseCount(int)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `value` is below 1.
    pub fn set_max_clause_count(value: i32) -> Result<()> {
        if value < 1 {
            return Err(LuceneError::IllegalArgument(
                "maxClauseCount must be >= 1".to_string(),
            ));
        }
        MAX_CLAUSE_COUNT.store(value, Ordering::Relaxed);
        Ok(())
    }

    /// Sets the query cache to use when scores are not needed, or `None` to
    /// never cache query matches.
    ///
    /// Equivalent to `IndexSearcher.setQueryCache(QueryCache)`. It should be
    /// called *before* starting to use this searcher, and queries should not be
    /// modified after they have been passed to a caching searcher.
    pub fn set_query_cache(&mut self, query_cache: Option<Arc<dyn QueryCache>>) {
        self.query_cache = query_cache;
    }

    /// Returns this searcher's query cache.
    ///
    /// Equivalent to `IndexSearcher.getQueryCache()`.
    pub fn get_query_cache(&self) -> Option<&Arc<dyn QueryCache>> {
        self.query_cache.as_ref()
    }

    /// Sets the query caching policy.
    ///
    /// Equivalent to
    /// `IndexSearcher.setQueryCachingPolicy(QueryCachingPolicy)`. It should be
    /// called *before* starting to use this searcher.
    pub fn set_query_caching_policy(&mut self, policy: Arc<dyn QueryCachingPolicy>) {
        self.query_caching_policy = Some(policy);
    }

    /// Returns this searcher's query caching policy.
    ///
    /// Equivalent to `IndexSearcher.getQueryCachingPolicy()`.
    pub fn get_query_caching_policy(&self) -> Option<&Arc<dyn QueryCachingPolicy>> {
        self.query_caching_policy.as_ref()
    }

    /// Creates the leaf slices, each holding a subset of the given leaves and
    /// each executed in a single thread.
    ///
    /// Equivalent to the `protected IndexSearcher.slices(List<LeafReaderContext>)`,
    /// which delegates to
    /// [`compute_slices`](Self::compute_slices) with the default thresholds and
    /// without segment partitions. By default, segments with more than 250,000
    /// documents get their own thread.
    ///
    /// # Errors
    ///
    /// Propagates the partition construction error for an empty leaf.
    pub fn default_slices(leaves: &[Arc<LeafReaderContext>]) -> Result<Vec<LeafSlice>> {
        Self::compute_slices(leaves, MAX_DOCS_PER_SLICE, MAX_SEGMENTS_PER_SLICE, false)
    }

    /// Segregates leaf contexts amongst multiple slices, according to the
    /// provided maximum number of documents and segments per slice.
    ///
    /// Equivalent to the `public static IndexSearcher.slices(List, int, int,
    /// boolean)`; the name differs because Rust has no overloading.
    ///
    /// * `leaves` — the leaves to slice;
    /// * `max_docs_per_slice` — the maximum number of documents in a slice;
    /// * `max_segments_per_slice` — the maximum number of segments in a slice;
    /// * `allow_segment_partitions` — whether a segment holding more documents
    ///   than `max_docs_per_slice` may be split into equally-sized partitions,
    ///   each getting its own slice.
    ///
    /// Intra-segment concurrency is not enabled by default, because there is
    /// still a performance penalty for queries requiring segment-level
    /// computation ahead of time, such as points and range queries.
    ///
    /// # Errors
    ///
    /// Propagates the partition construction error for an empty leaf.
    pub fn compute_slices(
        leaves: &[Arc<LeafReaderContext>],
        max_docs_per_slice: i32,
        max_segments_per_slice: usize,
        allow_segment_partitions: bool,
    ) -> Result<Vec<LeafSlice>> {
        // Make a copy so we can sort by maxDoc, descending.
        let mut sorted_leaves: Vec<Arc<LeafReaderContext>> = leaves.to_vec();
        sorted_leaves.sort_by_key(|leaf| std::cmp::Reverse(leaf.leaf_reader().max_doc()));

        if allow_segment_partitions {
            return Self::slices_with_segment_partitions(
                max_docs_per_slice,
                max_segments_per_slice,
                &sorted_leaves,
            );
        }

        let mut grouped_leaves: Vec<Vec<Arc<LeafReaderContext>>> = Vec::new();
        let mut doc_sum: i64 = 0;
        let mut open_group: Option<usize> = None;
        for ctx in &sorted_leaves {
            if ctx.leaf_reader().max_doc() > max_docs_per_slice {
                debug_assert!(open_group.is_none());
                grouped_leaves.push(vec![Arc::clone(ctx)]);
            } else {
                match open_group {
                    None => {
                        grouped_leaves.push(vec![Arc::clone(ctx)]);
                        open_group = Some(grouped_leaves.len() - 1);
                    }
                    Some(index) => grouped_leaves[index].push(Arc::clone(ctx)),
                }

                doc_sum += i64::from(ctx.leaf_reader().max_doc());
                let group_len = grouped_leaves[open_group.unwrap_or(0)].len();
                if group_len >= max_segments_per_slice || doc_sum > i64::from(max_docs_per_slice) {
                    open_group = None;
                    doc_sum = 0;
                }
            }
        }

        let mut slices = Vec::with_capacity(grouped_leaves.len());
        for current_leaf in &grouped_leaves {
            slices.push(LeafSlice::entire_segments(current_leaf)?);
        }
        Ok(slices)
    }

    /// Equivalent to the private
    /// `IndexSearcher.slicesWithSegmentPartitions(int, int, List)`.
    fn slices_with_segment_partitions(
        max_docs_per_slice: i32,
        max_segments_per_slice: usize,
        sorted_leaves: &[Arc<LeafReaderContext>],
    ) -> Result<Vec<LeafSlice>> {
        let mut grouped_leaf_partitions: Vec<Vec<LeafReaderContextPartition>> = Vec::new();
        let mut current_slice_num_docs: i32 = 0;
        let mut open_group: Option<usize> = None;
        for ctx in sorted_leaves {
            let max_doc = ctx.leaf_reader().max_doc();
            if max_doc > max_docs_per_slice {
                debug_assert!(open_group.is_none());
                // If the segment does not fit in a single slice, split it into
                // at most 5 partitions of equal size.
                let num_slices = 5.min(
                    max_doc.div_euclid(max_docs_per_slice)
                        + i32::from(max_doc.rem_euclid(max_docs_per_slice) != 0),
                );
                let num_docs = max_doc / num_slices;
                let mut max_doc_id = num_docs;
                let mut min_doc_id = 0;
                for _ in 0..num_slices - 1 {
                    grouped_leaf_partitions.push(vec![
                        LeafReaderContextPartition::create_from_and_to(
                            Arc::clone(ctx),
                            min_doc_id,
                            max_doc_id,
                        )?,
                    ]);
                    min_doc_id = max_doc_id;
                    max_doc_id += num_docs;
                }
                // The last slice gets all the remaining docs.
                grouped_leaf_partitions.push(vec![LeafReaderContextPartition::create_from_and_to(
                    Arc::clone(ctx),
                    min_doc_id,
                    max_doc,
                )?]);
            } else {
                let index = match open_group {
                    Some(index) => index,
                    None => {
                        grouped_leaf_partitions.push(Vec::new());
                        let index = grouped_leaf_partitions.len() - 1;
                        open_group = Some(index);
                        index
                    }
                };
                grouped_leaf_partitions[index].push(
                    LeafReaderContextPartition::create_for_entire_segment(Arc::clone(ctx))?,
                );

                current_slice_num_docs = current_slice_num_docs.wrapping_add(max_doc);
                // We only split a segment when it does not fit entirely in a
                // slice. We do not partition the segment that makes the current
                // slice go over maxDocsPerSlice, so a slice either contains
                // multiple entire segments, or a single partition of a segment.
                if grouped_leaf_partitions[index].len() >= max_segments_per_slice
                    || current_slice_num_docs > max_docs_per_slice
                {
                    open_group = None;
                    current_slice_num_docs = 0;
                }
            }
        }

        Ok(grouped_leaf_partitions
            .into_iter()
            .map(LeafSlice::new)
            .collect())
    }

    /// Returns the index reader this searches.
    ///
    /// Equivalent to `IndexSearcher.getIndexReader()`.
    pub fn get_index_reader(&self) -> &Arc<dyn IndexReader> {
        &self.reader
    }

    /// Returns a reader for the stored fields of this index.
    ///
    /// Equivalent to `IndexSearcher.storedFields()`, sugar for
    /// `getIndexReader().storedFields()`. It never returns nothing, even if no
    /// stored fields were indexed, and the returned instance should only be
    /// used by a single thread.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while opening the stored fields.
    pub fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
        self.reader.stored_fields()
    }

    /// Sets the similarity used by this searcher.
    ///
    /// Equivalent to `IndexSearcher.setSimilarity(Similarity)`.
    pub fn set_similarity(&mut self, similarity: Arc<dyn Similarity>) {
        self.similarity = similarity;
    }

    /// Returns the similarity used to compute scores.
    ///
    /// Equivalent to `IndexSearcher.getSimilarity()`.
    pub fn get_similarity(&self) -> &Arc<dyn Similarity> {
        &self.similarity
    }

    /// Returns the leaf slices used for concurrent searching.
    ///
    /// Equivalent to the `final IndexSearcher.getSlices()`, which computes them
    /// once and caches them.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when a slice targets several
    /// partitions of the same leaf, and propagates the slice construction
    /// error.
    pub fn get_slices(&self) -> Result<&[LeafSlice]> {
        let cached = self.leaf_slices.get_or_init(|| {
            let slices = match Self::default_slices(&self.leaf_contexts) {
                Ok(slices) => slices,
                Err(err) => return Err(err.to_string()),
            };
            // Enforce that there are not multiple leaf partitions within the
            // same leaf slice pointing to the same leaf context. It is a
            // requirement that Collector::get_leaf_collector is called once per
            // leaf context; it also makes no sense to partition a segment and
            // then search those partitions as part of the same slice, because
            // the goal of partitioning is parallel searching, which happens at
            // the slice level.
            for leaf_slice in &slices {
                if leaf_slice.partitions.len() <= 1 {
                    continue;
                }
                if let Err(err) = Self::enforce_distinct_leaves(leaf_slice) {
                    return Err(err.to_string());
                }
            }
            Ok(slices)
        });
        match cached {
            Ok(slices) => Ok(slices),
            Err(message) => Err(LuceneError::IllegalState(message.clone())),
        }
    }

    /// Equivalent to the private `IndexSearcher.enforceDistinctLeaves`.
    fn enforce_distinct_leaves(leaf_slice: &LeafSlice) -> Result<()> {
        let mut distinct_leaves = std::collections::HashSet::new();
        for leaf_partition in &leaf_slice.partitions {
            if !distinct_leaves.insert(leaf_partition.ctx.id()) {
                return Err(LuceneError::IllegalState(
                    "The same slice targets multiple leaf partitions of the same leaf reader context. A physical segment should rather get partitioned to be searched concurrently from as many slices as the number of leaf partitions it is split into."
                        .to_string(),
                ));
            }
        }
        Ok(())
    }

    /// Counts how many documents match the given query.
    ///
    /// Equivalent to `IndexSearcher.count(Query)`. It may be faster than
    /// counting the number of hits by collecting all matches, because the count
    /// is retrieved from the index statistics whenever possible.
    ///
    /// **Divergence from Lucene 10.5.0.** Java first rewrites
    /// `new ConstantScoreQuery(query)` to pick up that query's extra rewrite
    /// rules, and then applies a two-clause pure-disjunction optimisation that
    /// inspects a `BooleanQuery`. Neither `ConstantScoreQuery` nor
    /// `BooleanQuery` is part of the query-execution spine and neither is
    /// ported yet, so this port rewrites the query as given and always counts
    /// through [`TotalHitCountCollectorManager`]. The count returned is the
    /// same; only the two optimisations are missing.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while rewriting, weighting or counting.
    pub fn count(&self, query: Arc<dyn Query>) -> Result<i32> {
        let query = self.rewrite(query)?;
        let collector_manager = TotalHitCountCollectorManager::new(self.get_slices()?);
        let first_collector = collector_manager.new_collector()?;
        let weight = self.create_weight(query, first_collector.score_mode(), 1.0)?;
        self.search_weight(weight, &collector_manager, first_collector)
    }

    /// Finds the top `num_hits` hits for `query` where all results come after a
    /// previous result.
    ///
    /// Equivalent to `IndexSearcher.searchAfter(ScoreDoc, Query, int)`. Passing
    /// the bottom result of a previous page as `after` makes this an efficient
    /// way to page deeply through large result sets.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `after.doc` exceeds the
    /// number of documents in the reader, and propagates any I/O error.
    pub fn search_after(
        &self,
        after: Option<ScoreDoc>,
        query: Arc<dyn Query>,
        num_hits: i32,
    ) -> Result<TopDocs> {
        let limit = self.reader.max_doc().max(1);
        if let Some(after) = after.as_ref() {
            if after.doc >= limit {
                return Err(LuceneError::IllegalArgument(format!(
                    "after.doc exceeds the number of documents in the reader: after.doc={} limit={limit}",
                    after.doc
                )));
            }
        }

        let capped_num_hits = num_hits.min(limit);
        let manager =
            TopScoreDocCollectorManager::new(capped_num_hits, after, TOTAL_HITS_THRESHOLD)?;

        self.search_with_manager(query, &manager)
    }

    /// Returns the configured timeout for the searches running through this
    /// searcher, if any.
    ///
    /// Equivalent to `IndexSearcher.getTimeout()`.
    pub fn get_timeout(&self) -> Option<&Arc<dyn QueryTimeout>> {
        self.query_timeout.as_ref()
    }

    /// Sets a timeout for the searches running through this searcher.
    ///
    /// Equivalent to `IndexSearcher.setTimeout(QueryTimeout)`.
    pub fn set_timeout(&mut self, query_timeout: Option<Arc<dyn QueryTimeout>>) {
        self.query_timeout = query_timeout;
    }

    /// Finds the top `n` hits for `query`.
    ///
    /// Equivalent to `IndexSearcher.search(Query, int)`.
    ///
    /// # Errors
    ///
    /// As [`search_after`](Self::search_after).
    pub fn search(&self, query: Arc<dyn Query>, n: i32) -> Result<TopDocs> {
        self.search_after(None, query, n)
    }

    /// Lower-level search API, calling
    /// [`LeafCollector::collect`](crate::search::LeafCollector::collect) for
    /// every matching document.
    ///
    /// Equivalent to the deprecated `IndexSearcher.search(Query, Collector)`,
    /// deprecated in favour of
    /// [`search_with_manager`](Self::search_with_manager) because that one
    /// supports concurrency. The name differs because Rust has no overloading.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while rewriting, weighting or
    /// collecting.
    pub fn search_with_collector(
        &self,
        query: Arc<dyn Query>,
        collector: &mut dyn Collector,
    ) -> Result<()> {
        let query = self.rewrite_for_score_mode(query, collector.score_mode().needs_scores())?;
        let weight = self.create_weight(query, collector.score_mode(), 1.0)?;
        collector.set_weight(Arc::clone(&weight));
        for ctx in &self.leaf_contexts {
            // Search each subreader.
            self.search_leaf(ctx, 0, NO_MORE_DOCS, &weight, collector)?;
        }
        Ok(())
    }

    /// Returns `true` if any search hit the configured timeout.
    ///
    /// Equivalent to `IndexSearcher.timedOut()`.
    pub fn timed_out(&self) -> bool {
        self.partial_result.load(Ordering::SeqCst)
    }

    /// Lower-level search API: searches all leaves with the given collector
    /// manager, parallelising the collection over
    /// [`get_slices`](Self::get_slices) through this searcher's executor.
    ///
    /// Equivalent to
    /// `IndexSearcher.search(Query, CollectorManager<C, T>)`; the name differs
    /// because Rust has no overloading.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the manager produces
    /// collectors with differing score modes, and propagates any I/O error.
    pub fn search_with_manager<M>(&self, query: Arc<dyn Query>, manager: &M) -> Result<M::Output>
    where
        M: CollectorManager,
        M::Collector: Send + 'static,
    {
        let first_collector = manager.new_collector()?;
        let query =
            self.rewrite_for_score_mode(query, first_collector.score_mode().needs_scores())?;
        let weight = self.create_weight(query, first_collector.score_mode(), 1.0)?;
        self.search_weight(weight, manager, first_collector)
    }

    /// Equivalent to the private
    /// `IndexSearcher.search(Weight, CollectorManager, C)`.
    fn search_weight<M>(
        &self,
        weight: Arc<dyn Weight>,
        manager: &M,
        first_collector: M::Collector,
    ) -> Result<M::Output>
    where
        M: CollectorManager,
        M::Collector: Send + 'static,
    {
        let leaf_slices = self.get_slices()?;
        if leaf_slices.is_empty() {
            // There are no segments, nothing to offload to the executor, but we
            // do need to reduce in order to create some kind of empty result.
            debug_assert!(self.leaf_contexts.is_empty());
            return manager.reduce(vec![first_collector]);
        }

        let mut collectors = Vec::with_capacity(leaf_slices.len());
        collectors.push(first_collector);
        let score_mode = collectors[0].score_mode();
        for _ in 1..leaf_slices.len() {
            let collector = manager.new_collector()?;
            if score_mode != collector.score_mode() {
                return Err(LuceneError::IllegalState(
                    "CollectorManager does not always produce collectors with the same score mode"
                        .to_string(),
                ));
            }
            collectors.push(collector);
        }

        let mut tasks: Vec<Box<dyn FnOnce() -> Result<M::Collector> + Send + 'static>> =
            Vec::with_capacity(leaf_slices.len());
        for (slice, collector) in leaf_slices.iter().zip(collectors) {
            let partitions: Vec<LeafReaderContextPartition> = slice.partitions.to_vec();
            let weight = Arc::clone(&weight);
            let query_timeout = self.query_timeout.clone();
            let partial_result = Arc::clone(&self.partial_result);
            let mut collector = collector;
            tasks.push(Box::new(move || {
                Self::search_partitions_inner(
                    &partitions,
                    &weight,
                    &mut collector,
                    query_timeout.as_ref(),
                    &partial_result,
                )?;
                Ok(collector)
            }));
        }

        let results = self.task_executor.invoke_all(tasks)?;
        manager.reduce(results)
    }

    /// Lower-level search API: searches the given leaf partitions exclusively.
    ///
    /// Equivalent to the `protected IndexSearcher.search(
    /// LeafReaderContextPartition[], Weight, Collector)`. To search across all
    /// of this searcher's leaves, use [`get_leaf_contexts`](Self::get_leaf_contexts).
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while collecting.
    pub fn search_partitions(
        &self,
        partitions: &[LeafReaderContextPartition],
        weight: &Arc<dyn Weight>,
        collector: &mut dyn Collector,
    ) -> Result<()> {
        Self::search_partitions_inner(
            partitions,
            weight,
            collector,
            self.query_timeout.as_ref(),
            &self.partial_result,
        )
    }

    /// The body of [`search_partitions`](Self::search_partitions), free of
    /// `&self` so that it can be moved into a task.
    fn search_partitions_inner(
        partitions: &[LeafReaderContextPartition],
        weight: &Arc<dyn Weight>,
        collector: &mut dyn Collector,
        query_timeout: Option<&Arc<dyn QueryTimeout>>,
        partial_result: &AtomicBool,
    ) -> Result<()> {
        collector.set_weight(Arc::clone(weight));

        for partition in partitions {
            // Search each subreader partition.
            Self::search_leaf_inner(
                &partition.ctx,
                partition.min_doc_id,
                partition.max_doc_id,
                weight,
                collector,
                query_timeout,
                partial_result,
            )?;
        }
        Ok(())
    }

    /// Lower-level search API, calling
    /// [`LeafCollector::collect`](crate::search::LeafCollector::collect) for
    /// every matching document of one leaf.
    ///
    /// Equivalent to the `protected IndexSearcher.searchLeaf(LeafReaderContext,
    /// int, int, Weight, Collector)`.
    ///
    /// * `ctx` — the leaf to execute the search against;
    /// * `min_doc_id` — the lower bound of the doc ID range to search;
    /// * `max_doc_id` — the upper bound of the doc ID range to search;
    /// * `weight` — the weight matching documents;
    /// * `collector` — the collector receiving hits.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while collecting. The early-termination
    /// and timeout signals are handled here, exactly where Java catches
    /// `CollectionTerminatedException` and `TimeExceededException`.
    pub fn search_leaf(
        &self,
        ctx: &Arc<LeafReaderContext>,
        min_doc_id: i32,
        max_doc_id: i32,
        weight: &Arc<dyn Weight>,
        collector: &mut dyn Collector,
    ) -> Result<()> {
        Self::search_leaf_inner(
            ctx,
            min_doc_id,
            max_doc_id,
            weight,
            collector,
            self.query_timeout.as_ref(),
            &self.partial_result,
        )
    }

    /// The body of [`search_leaf`](Self::search_leaf), free of `&self` so that
    /// it can be moved into a task.
    fn search_leaf_inner(
        ctx: &Arc<LeafReaderContext>,
        min_doc_id: i32,
        max_doc_id: i32,
        weight: &Arc<dyn Weight>,
        collector: &mut dyn Collector,
        query_timeout: Option<&Arc<dyn QueryTimeout>>,
        partial_result: &AtomicBool,
    ) -> Result<()> {
        let mut leaf_collector = match collector.get_leaf_collector(ctx) {
            Ok(leaf_collector) => leaf_collector,
            Err(CollectionError::CollectionTerminated) => {
                // There is no doc of interest in this reader context; continue
                // with the following leaf.
                return Ok(());
            }
            Err(err) => return Err(err.into_lucene_error()),
        };

        let scorer_supplier = weight.scorer_supplier(ctx)?;
        if let Some(mut scorer_supplier) = scorer_supplier {
            scorer_supplier.set_top_level_scoring_clause()?;
            let mut scorer = scorer_supplier.bulk_scorer()?;
            if let Some(query_timeout) = query_timeout {
                scorer = Box::new(TimeLimitingBulkScorer::new(
                    scorer,
                    Arc::clone(query_timeout),
                ));
            }
            let live_docs = ctx.leaf_reader().get_live_docs();
            // Optimise for the case when live docs are stored in a bit set.
            let accept_docs = ScorerUtil::likely_live_docs(live_docs.as_deref());
            match scorer.score(&mut *leaf_collector, accept_docs, min_doc_id, max_doc_id) {
                Ok(_) => {}
                // Collection was terminated prematurely; continue with the
                // following leaf.
                Err(CollectionError::CollectionTerminated) => {}
                Err(CollectionError::TimeExceeded) => {
                    partial_result.store(true, Ordering::SeqCst);
                }
                Err(CollectionError::Lucene(err)) => return Err(err),
            }
        }
        // Note: this is called if collection ran successfully, including the
        // above special cases of early termination and timeout, but no other
        // failure.
        leaf_collector.finish()
    }

    /// Rewrites a query into primitive queries.
    ///
    /// Equivalent to `IndexSearcher.rewrite(Query)`, which loops until the
    /// query stops changing and then validates the total clause count.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::ResourceLimit`] carrying a
    /// [`TooManyNestedClauses`] message when the query has more than
    /// [`get_max_clause_count`](Self::get_max_clause_count) clauses
    /// cumulatively, and propagates any I/O error.
    pub fn rewrite(&self, original: Arc<dyn Query>) -> Result<Arc<dyn Query>> {
        let mut query = original;
        while let Some(rewritten) = query.rewrite(self)? {
            query = rewritten;
        }
        let mut visitor = NumClausesCheckVisitor::default();
        query.visit(&mut visitor);
        visitor.check()?;
        Ok(query)
    }

    /// Rewrites a query, taking advantage of the extra rewrite rules that apply
    /// when scores are not needed.
    ///
    /// Equivalent to the private `IndexSearcher.rewrite(Query, boolean)`.
    ///
    /// **Divergence from Lucene 10.5.0.** When scores are not needed Java
    /// rewrites `new ConstantScoreQuery(original)` instead of `original`, which
    /// unlocks that query's extra rewrite rules. `ConstantScoreQuery` belongs to
    /// the query package and is not ported yet, so this port rewrites the query
    /// as given. The rewritten query matches the same documents; only some
    /// simplifications are missed.
    fn rewrite_for_score_mode(
        &self,
        original: Arc<dyn Query>,
        _needs_scores: bool,
    ) -> Result<Arc<dyn Query>> {
        self.rewrite(original)
    }

    /// Returns an explanation of how `doc` scored against `query`.
    ///
    /// Equivalent to `IndexSearcher.explain(Query, int)`. It is intended for
    /// developing similarity implementations and, for good performance, should
    /// not be displayed with every hit: computing an explanation is as
    /// expensive as executing the query over the entire index.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while rewriting, weighting or
    /// explaining.
    pub fn explain(&self, query: Arc<dyn Query>, doc: i32) -> Result<Explanation> {
        let query = self.rewrite(query)?;
        let weight = self.create_weight(query, ScoreMode::COMPLETE, 1.0)?;
        self.explain_weight(&weight, doc)
    }

    /// Returns an explanation of how `doc` scored against `weight`.
    ///
    /// Equivalent to the `protected IndexSearcher.explain(Weight, int)`;
    /// applications should call [`explain`](Self::explain) instead.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while explaining.
    pub fn explain_weight(&self, weight: &Arc<dyn Weight>, doc: i32) -> Result<Explanation> {
        let n = sub_index_from_leaves(doc, &self.leaf_contexts);
        let ctx = &self.leaf_contexts[n];
        let de_based_doc = doc - ctx.doc_base();
        let live_docs = ctx.leaf_reader().get_live_docs();
        if let Some(live_docs) = live_docs.as_ref() {
            if !live_docs.get(de_based_doc as usize) {
                return Ok(Explanation::no_match(
                    format!("Document {doc} is deleted"),
                    Vec::new(),
                ));
            }
        }
        weight.explain(ctx, de_based_doc)
    }

    /// Creates a [`Weight`] for the given query, adding caching when possible
    /// and configured.
    ///
    /// Equivalent to `IndexSearcher.createWeight(Query, ScoreMode, float)`. The
    /// query is assumed to have been [`rewrite`](Self::rewrite)n already.
    ///
    /// **Divergence from Lucene 10.5.0.** Java caches whenever a cache is
    /// installed, because the default caching policy is never null. Here the
    /// policy is optional — its default implementation is not ported, see
    /// [`crate::search::query_cache`] — so caching applies only when both a
    /// cache and a policy are present.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while building the weight, including the
    /// [`LuceneError::UnsupportedOperation`] a query that does not implement
    /// `create_weight` returns.
    pub fn create_weight(
        &self,
        query: Arc<dyn Query>,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        let mut weight = query.create_weight(self, score_mode, boost)?;
        if !score_mode.needs_scores() {
            if let (Some(query_cache), Some(policy)) = (
                self.query_cache.as_ref(),
                self.query_caching_policy.as_ref(),
            ) {
                weight = query_cache.do_cache(weight, Arc::clone(policy));
            }
        }
        Ok(weight)
    }

    /// Returns this searcher's top-level reader context.
    ///
    /// Equivalent to `IndexSearcher.getTopReaderContext()`.
    pub fn get_top_reader_context(&self) -> &Arc<dyn IndexReaderContext> {
        &self.reader_context
    }

    /// Returns the statistics for a term.
    ///
    /// Equivalent to `IndexSearcher.termStatistics(Term, int, long)`. Java
    /// makes it overridable so that, for instance, a subclass can return a
    /// term's statistics across a distributed collection.
    ///
    /// * `doc_freq` — the document frequency of the term, at least 1;
    /// * `total_term_freq` — the total term frequency.
    ///
    /// # Errors
    ///
    /// Propagates the [`TermStatistics`] validation error, which triggers when
    /// `doc_freq` is not positive.
    pub fn term_statistics(
        &self,
        term: &Term,
        doc_freq: i32,
        total_term_freq: i64,
    ) -> Result<TermStatistics> {
        // This constructor fails if docFreq <= 0.
        TermStatistics::new(term.bytes().clone(), i64::from(doc_freq), total_term_freq)
    }

    /// Returns the statistics for a field, or `None` if the field does not
    /// exist — that is, has no indexed terms.
    ///
    /// Equivalent to `IndexSearcher.collectionStatistics(String)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the terms, and the
    /// [`CollectionStatistics`] validation error.
    pub fn collection_statistics(&self, field: &str) -> Result<Option<CollectionStatistics>> {
        let mut doc_count: i64 = 0;
        let mut sum_total_term_freq: i64 = 0;
        let mut sum_doc_freq: i64 = 0;
        for leaf in &self.leaf_contexts {
            // Terms.getTerms(LeafReader, String) answers Terms.EMPTY when the
            // field is absent, and every statistic of Terms.EMPTY is 0.
            if let Some(terms) = leaf.leaf_reader().terms(field)? {
                doc_count += i64::from(terms.doc_count());
                sum_total_term_freq += terms.sum_total_term_freq();
                sum_doc_freq += terms.sum_doc_freq();
            }
        }
        if doc_count == 0 {
            return Ok(None);
        }
        Ok(Some(CollectionStatistics::new(
            field,
            i64::from(self.reader.max_doc()),
            doc_count,
            sum_total_term_freq,
            sum_doc_freq,
        )?))
    }

    /// Returns the task executor this searcher relies on to execute concurrent
    /// operations.
    ///
    /// Equivalent to `IndexSearcher.getTaskExecutor()`.
    pub fn get_task_executor(&self) -> &Arc<TaskExecutor> {
        &self.task_executor
    }
}
