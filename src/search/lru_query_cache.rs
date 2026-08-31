//! LRU query caching, ported from
//! `org.apache.lucene.search.LRUQueryCache`.

#![deny(unsafe_code)]

use std::collections::{BTreeMap, HashMap};
use std::fmt::Debug;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use crate::error::{LuceneError, Result};
use crate::index::{
    get_top_level_context, CacheHelper, CacheKey, ClosedListener, IndexReaderContext,
    LeafReaderContext,
};
use crate::search::bulk_scorer::BulkScorer;
use crate::search::collection_terminated_exception::{CollectionError, CollectionResult};
use crate::search::collector::LeafCollector;
use crate::search::constant_score_scorer_supplier::{
    ConstantScoreIteratorSupplier, ConstantScoreScorerSupplier,
};
use crate::search::constant_score_weight::{ConstantScoreWeight, ConstantScoreWeightImpl};
use crate::search::doc_id_set::DocIdSet;
use crate::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use crate::search::doc_id_stream::DocIdStream;
use crate::search::matches::Matches;
use crate::search::query::{Query, QueryKey};
use crate::search::query_cache::{QueryCache, QueryCachingPolicy};
use crate::search::scorable::Scorable;
use crate::search::score_mode::ScoreMode;
use crate::search::scorer::into_scorer_iterator;
use crate::search::scorer_supplier::ScorerSupplier;
use crate::search::segment_cacheable::SegmentCacheable;
use crate::search::similarities::Explanation;
use crate::search::two_phase_iterator::ScorerIterator;
use crate::search::weight::Weight;
use crate::util::bit_sets::{RoaringDocIdSet, RoaringDocIdSetBuilder};
use crate::util::doc_id_set::BitDocIdSet;
use crate::util::{Accountable, BitSet, FixedBitSet, RamUsageEstimator};

/// The estimated per-entry overhead of a hash table.
///
/// **Divergence from Lucene 10.5.0.** Java declares this constant on
/// `org.apache.lucene.util.RamUsageEstimator`. This crate's port of that class
/// does not carry it yet, and the accounting of this cache depends on it, so it
/// is declared here with Java's own definition: two references — a key and a
/// value — doubled, because hash tables are oversized to avoid collisions.
pub const HASHTABLE_RAM_BYTES_PER_ENTRY: i64 = 2 * RamUsageEstimator::NUM_BYTES_OBJECT_REF * 2;

/// The estimated per-entry overhead of a linked hash table.
///
/// See [`HASHTABLE_RAM_BYTES_PER_ENTRY`] for why the constant lives here: this
/// is Java's `RamUsageEstimator.LINKED_HASHTABLE_RAM_BYTES_PER_ENTRY`, the hash
/// table entry plus the previous and next references.
pub const LINKED_HASHTABLE_RAM_BYTES_PER_ENTRY: i64 =
    HASHTABLE_RAM_BYTES_PER_ENTRY + 2 * RamUsageEstimator::NUM_BYTES_OBJECT_REF;

/// The shallow size of a [`CacheAndCount`].
///
/// Equivalent to `CacheAndCount.BASE_RAM_BYTES_USED`, which is
/// `RamUsageEstimator.shallowSizeOfInstance(CacheAndCount.class)`: an object
/// header, one reference and one `int`, aligned.
const CACHE_AND_COUNT_BASE_RAM_BYTES_USED: i64 = 24;

/// A cached set of doc IDs together with its cardinality.
///
/// Equivalent to the protected static nested
/// `LRUQueryCache.CacheAndCount`.
#[derive(Clone)]
pub struct CacheAndCount {
    cache: Arc<dyn DocIdSet + Send + Sync>,
    count: i32,
    /// Whether this value is the `CacheAndCount.EMPTY` singleton.
    ///
    /// **Divergence from Lucene 10.5.0.** Java compares against the singleton
    /// with `==`, reference identity, to recognise the "this query matches
    /// nothing on this leaf" marker. Rust values have no identity, so the
    /// marker is a flag that only [`CacheAndCount::empty`] sets; it recognises
    /// exactly the values Java's `==` recognises.
    is_empty_marker: bool,
}

impl Debug for CacheAndCount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CacheAndCount")
            .field("count", &self.count)
            .finish_non_exhaustive()
    }
}

impl CacheAndCount {
    /// Wraps a doc ID set and its cardinality.
    ///
    /// Equivalent to `new CacheAndCount(DocIdSet, int)`.
    pub fn new(cache: Arc<dyn DocIdSet + Send + Sync>, count: i32) -> Self {
        Self {
            cache,
            count,
            is_empty_marker: false,
        }
    }

    /// Returns the marker for a query that matches nothing on a leaf.
    ///
    /// Equivalent to the `CacheAndCount.EMPTY` constant.
    pub fn empty() -> Self {
        Self {
            cache: Arc::new(crate::search::doc_id_set::EmptyDocIdSet),
            count: 0,
            is_empty_marker: true,
        }
    }

    /// Returns an iterator over the cached doc IDs.
    ///
    /// Equivalent to `CacheAndCount.iterator()`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised by the underlying set.
    pub fn iterator(&self) -> Result<Box<dyn DocIdSetIterator>> {
        self.cache.iterator()
    }

    /// Returns the number of cached doc IDs.
    ///
    /// Equivalent to `CacheAndCount.count()`.
    pub fn count(&self) -> i32 {
        self.count
    }
}

impl Accountable for CacheAndCount {
    fn ram_bytes_used(&self) -> i64 {
        CACHE_AND_COUNT_BASE_RAM_BYTES_USED + self.cache.ram_bytes_used()
    }
}

/// A `RoaringDocIdSet` seen as a [`DocIdSet`].
///
/// **Divergence from Lucene 10.5.0.** Java's `RoaringDocIdSet` *is* a
/// `DocIdSet`. The port's [`RoaringDocIdSet`] lives in `util` and exposes the
/// same two operations under different names, but does not implement the
/// search-package trait, so this newtype supplies it.
#[derive(Debug)]
struct RoaringCachedDocIdSet(RoaringDocIdSet);

impl Accountable for RoaringCachedDocIdSet {
    fn ram_bytes_used(&self) -> i64 {
        self.0.ram_bytes_used()
    }
}

impl DocIdSet for RoaringCachedDocIdSet {
    fn iterator(&self) -> Result<Box<dyn DocIdSetIterator>> {
        Ok(Box::new(self.0.iter()))
    }
}

/// The access-ordered map of the queries the cache holds.
///
/// Equivalent to the `LinkedHashMap<Query, Query>` — created with
/// `accessOrder = true` — that Java calls `uniqueQueries`, and to its key-set
/// view `mostRecentlyUsedQueries`. It maps a query to the single instance the
/// cache keeps, so that several copies of the same query are not stored, and
/// its iteration order is least-recently-used first.
#[derive(Debug, Default)]
struct LruQueryMap {
    entries: HashMap<QueryKey, (Arc<dyn Query>, u64)>,
    order: BTreeMap<u64, QueryKey>,
    counter: u64,
}

impl LruQueryMap {
    fn new() -> Self {
        Self::default()
    }

    fn touch(&mut self, key: &QueryKey, previous_seq: u64) -> u64 {
        self.order.remove(&previous_seq);
        self.counter += 1;
        let seq = self.counter;
        self.order.insert(seq, key.clone());
        seq
    }

    /// Equivalent to `LinkedHashMap.get(Object)` on an access-ordered map: it
    /// moves the entry to the most-recently-used position.
    fn get(&mut self, key: &QueryKey) -> Option<Arc<dyn Query>> {
        let (query, seq) = self.entries.get(key)?;
        let (query, previous_seq) = (Arc::clone(query), *seq);
        let seq = self.touch(key, previous_seq);
        self.entries.insert(key.clone(), (Arc::clone(&query), seq));
        Some(query)
    }

    /// Equivalent to `Map.putIfAbsent(K, V)`: returns the existing singleton
    /// when there is one, and `None` when the query was inserted. As in Java,
    /// an existing entry is moved to the most-recently-used position.
    fn put_if_absent(&mut self, query: Arc<dyn Query>) -> Option<Arc<dyn Query>> {
        let key = QueryKey::new(Arc::clone(&query));
        if let Some((existing, seq)) = self.entries.get(&key) {
            let (existing, previous_seq) = (Arc::clone(existing), *seq);
            let seq = self.touch(&key, previous_seq);
            self.entries.insert(key, (Arc::clone(&existing), seq));
            return Some(existing);
        }
        self.counter += 1;
        let seq = self.counter;
        self.order.insert(seq, key.clone());
        self.entries.insert(key, (query, seq));
        None
    }

    /// Equivalent to `Map.remove(Object)`.
    fn remove(&mut self, key: &QueryKey) -> Option<Arc<dyn Query>> {
        let (query, seq) = self.entries.remove(key)?;
        self.order.remove(&seq);
        Some(query)
    }

    /// Removes and returns the least-recently-used query.
    ///
    /// Equivalent to `mostRecentlyUsedQueries.iterator().next()` followed by
    /// `iterator.remove()`.
    fn remove_eldest(&mut self) -> Option<Arc<dyn Query>> {
        let (seq, key) = self.order.iter().next().map(|(s, k)| (*s, k.clone()))?;
        self.order.remove(&seq);
        self.entries.remove(&key).map(|(query, _)| query)
    }

    fn len(&self) -> usize {
        self.entries.len()
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
    }

    /// Returns the cached queries in least-recently-used order.
    ///
    /// Equivalent to iterating `mostRecentlyUsedQueries`.
    fn queries(&self) -> Vec<Arc<dyn Query>> {
        self.order
            .values()
            .filter_map(|key| self.entries.get(key).map(|(query, _)| Arc::clone(query)))
            .collect()
    }
}

/// The per-leaf half of the cache.
///
/// Equivalent to the inner class `LRUQueryCache.LeafCache`. It is not
/// thread-safe: everything but the RAM accounting must be called under the
/// cache's write lock, which the port enforces by keeping it inside the
/// [`RwLock`].
#[derive(Debug)]
struct LeafCache {
    cache: HashMap<QueryKey, CacheAndCount>,
    ram_bytes_used: i64,
}

impl LeafCache {
    fn new() -> Self {
        Self {
            cache: HashMap::new(),
            ram_bytes_used: 0,
        }
    }
}

/// Whether a leaf is worth caching on.
///
/// Equivalent to the `Predicate<LeafReaderContext> leavesToCache` field.
pub type LeavesToCache = Arc<dyn Fn(&LeafReaderContext) -> bool + Send + Sync>;

/// The predicate the two-argument constructor installs.
///
/// Equivalent to the package-private
/// `LRUQueryCache.MinSegmentSizePredicate`: a leaf is cached on when it has at
/// least `min_size` documents and more than half the average number of
/// documents per leaf of the index. That guarantees that every leaf of the
/// upper [tier](crate::index::TieredMergePolicy) is cached.
pub fn min_segment_size_predicate(min_size: i32) -> LeavesToCache {
    Arc::new(move |context: &LeafReaderContext| {
        let max_doc = context.leaf_reader().max_doc();
        if max_doc < min_size {
            return false;
        }
        let (top_level_max_doc, num_leaves) = top_level_stats(context);
        let average_total_docs = top_level_max_doc / num_leaves.max(1) as i32;
        max_doc * 2 > average_total_docs
    })
}

/// Returns the top-level reader's `maxDoc` and its number of leaves.
///
/// Equivalent to
/// `ReaderUtil.getTopLevelContext(context).reader().maxDoc()` and
/// `.leaves().size()`. A leaf with no parent is its own top level and has a
/// single leaf, which is what `LeafReaderContext.leaves()` returns for it.
fn top_level_stats(context: &LeafReaderContext) -> (i32, usize) {
    match IndexReaderContext::parent(context).and_then(|parent| parent.upgrade()) {
        None => (context.leaf_reader().max_doc(), 1),
        Some(parent) => {
            let top = get_top_level_context(parent);
            let max_doc = top.reader().max_doc();
            let leaves = Arc::clone(&top).leaves().len();
            (max_doc, leaves)
        }
    }
}

/// The state a [`LRUQueryCache`] shares with the weights it wraps.
///
/// **Divergence from Lucene 10.5.0.** Java's `CachingWrapperWeight` is an inner
/// class and reaches the cache through the implicit outer reference. A Rust
/// weight has to own a handle, and [`QueryCache::do_cache`] only has `&self`,
/// so the cache's whole state lives behind an [`Arc`] that both halves share.
struct CacheState {
    max_size: usize,
    max_ram_bytes_used: i64,
    leaves_to_cache: LeavesToCache,
    /// Guards the per-leaf caches, and — through the lock ordering below — the
    /// invariant that a leaf cache only ever holds a subset of
    /// `unique_queries`.
    ///
    /// Equivalent to `cache` plus the `ReentrantReadWriteLock` that guards it.
    cache: RwLock<HashMap<usize, LeafCache>>,
    /// Equivalent to `uniqueQueries`, which Java wraps in
    /// `Collections.synchronizedMap` because reading an access-ordered
    /// `LinkedHashMap` mutates it.
    unique_queries: Mutex<LruQueryMap>,
    skip_cache_factor: AtomicU32,
    hit_count: AtomicI64,
    miss_count: AtomicI64,
    ram_bytes_used: AtomicI64,
    cache_count: AtomicI64,
    cache_size: AtomicI64,
}

impl Debug for CacheState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LRUQueryCache")
            .field("maxSize", &self.max_size)
            .field("maxRamBytesUsed", &self.max_ram_bytes_used)
            .field("ramBytesUsed", &self.ram_bytes_used.load(Ordering::Acquire))
            .field("cacheSize", &self.cache_size.load(Ordering::Acquire))
            .finish_non_exhaustive()
    }
}

impl CacheState {
    fn skip_cache_factor(&self) -> f32 {
        f32::from_bits(self.skip_cache_factor.load(Ordering::Acquire))
    }

    /// Equivalent to `LRUQueryCache.onHit(Object, Query)`.
    fn on_hit(&self) {
        self.hit_count.fetch_add(1, Ordering::AcqRel);
    }

    /// Equivalent to `LRUQueryCache.onMiss(Object, Query)`.
    fn on_miss(&self) {
        self.miss_count.fetch_add(1, Ordering::AcqRel);
    }

    /// Equivalent to `LRUQueryCache.onQueryCache(Query, long)`.
    fn on_query_cache(&self, ram_bytes_used: i64) {
        self.ram_bytes_used
            .fetch_add(ram_bytes_used, Ordering::AcqRel);
    }

    /// Equivalent to `LRUQueryCache.onQueryEviction(Query, long)`.
    fn on_query_eviction(&self, ram_bytes_used: i64) {
        self.ram_bytes_used
            .fetch_sub(ram_bytes_used, Ordering::AcqRel);
    }

    /// Equivalent to `LRUQueryCache.onDocIdSetCache(Object, long)`.
    fn on_doc_id_set_cache(&self, ram_bytes_used: i64) {
        self.cache_size.fetch_add(1, Ordering::AcqRel);
        self.cache_count.fetch_add(1, Ordering::AcqRel);
        self.ram_bytes_used
            .fetch_add(ram_bytes_used, Ordering::AcqRel);
    }

    /// Equivalent to
    /// `LRUQueryCache.onDocIdSetEviction(Object, int, long)`.
    fn on_doc_id_set_eviction(&self, num_entries: i64, sum_ram_bytes_used: i64) {
        self.ram_bytes_used
            .fetch_sub(sum_ram_bytes_used, Ordering::AcqRel);
        self.cache_size.fetch_sub(num_entries, Ordering::AcqRel);
    }

    /// Equivalent to `LRUQueryCache.onClear()`.
    fn on_clear(&self) {
        self.ram_bytes_used.store(0, Ordering::Release);
        self.cache_size.store(0, Ordering::Release);
    }

    /// Equivalent to the package-private
    /// `LRUQueryCache.requiresEviction()`.
    fn requires_eviction(&self, queries: &LruQueryMap) -> bool {
        let size = queries.len();
        if size == 0 {
            false
        } else {
            size > self.max_size
                || self.ram_bytes_used.load(Ordering::Acquire) > self.max_ram_bytes_used
        }
    }

    /// Equivalent to `LRUQueryCache.get(Query, IndexReader.CacheHelper)`.
    ///
    /// The caller holds the read lock, exactly as Java's does: the lock is
    /// taken in `CachingWrapperWeight` with `tryLock`, so that a busy cache
    /// yields an uncached scorer rather than a wait.
    fn get(
        &self,
        cache: &HashMap<usize, LeafCache>,
        key: &Arc<dyn Query>,
        reader_key: usize,
    ) -> Option<CacheAndCount> {
        let Some(leaf_cache) = cache.get(&reader_key) else {
            self.on_miss();
            return None;
        };
        // This get call moves the query to the most-recently-used position.
        let singleton = {
            let mut queries = self
                .unique_queries
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            queries.get(&QueryKey::new(Arc::clone(key)))
        };
        let Some(singleton) = singleton else {
            self.on_miss();
            return None;
        };
        let cached = leaf_cache.cache.get(&QueryKey::new(singleton)).cloned();
        match &cached {
            None => self.on_miss(),
            Some(_) => self.on_hit(),
        }
        cached
    }

    /// Equivalent to the private
    /// `LRUQueryCache.putIfAbsent(Query, CacheAndCount, IndexReader.CacheHelper)`.
    fn put_if_absent(
        self: &Arc<Self>,
        query: Arc<dyn Query>,
        cached: CacheAndCount,
        cache_helper: &dyn CacheHelper,
    ) {
        // Under the write lock, so that the query map and the leaf caches stay
        // in sync.
        let mut cache = self.cache.write().unwrap_or_else(PoisonError::into_inner);
        let query = {
            let mut queries = self
                .unique_queries
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            match queries.put_if_absent(Arc::clone(&query)) {
                None => {
                    self.on_query_cache(query_ram_bytes_used(query.as_ref()));
                    query
                }
                Some(singleton) => singleton,
            }
        };
        let key = cache_key_identity(cache_helper);
        if !cache.contains_key(&key) {
            cache.insert(key, LeafCache::new());
            self.ram_bytes_used
                .fetch_add(HASHTABLE_RAM_BYTES_PER_ENTRY, Ordering::AcqRel);
            // We just created a new leaf cache, so we need to register a close
            // listener.
            cache_helper.add_closed_listener(Box::new(ClearCoreCacheKey {
                state: Arc::clone(self),
                key,
            }));
        }
        let leaf_cache = cache
            .get_mut(&key)
            .expect("INVARIANT: the leaf cache was just inserted if it was missing");
        let query_key = QueryKey::new(query);
        if !leaf_cache.cache.contains_key(&query_key) {
            let ram = HASHTABLE_RAM_BYTES_PER_ENTRY + cached.ram_bytes_used();
            leaf_cache.cache.insert(query_key, cached);
            leaf_cache.ram_bytes_used += ram;
            self.on_doc_id_set_cache(ram);
        }
        self.evict_if_necessary(&mut cache);
    }

    /// Equivalent to the private `LRUQueryCache.evictIfNecessary()`.
    fn evict_if_necessary(&self, cache: &mut HashMap<usize, LeafCache>) {
        loop {
            let query = {
                let mut queries = self
                    .unique_queries
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                if !self.requires_eviction(&queries) {
                    return;
                }
                queries.remove_eldest()
            };
            let Some(query) = query else {
                return;
            };
            self.on_eviction(cache, &query);
        }
    }

    /// Equivalent to the private `LRUQueryCache.onEviction(Query)`.
    fn on_eviction(&self, cache: &mut HashMap<usize, LeafCache>, singleton: &Arc<dyn Query>) {
        self.on_query_eviction(query_ram_bytes_used(singleton.as_ref()));
        let key = QueryKey::new(Arc::clone(singleton));
        for leaf_cache in cache.values_mut() {
            if let Some(removed) = leaf_cache.cache.remove(&key) {
                let ram = HASHTABLE_RAM_BYTES_PER_ENTRY + removed.ram_bytes_used();
                leaf_cache.ram_bytes_used -= ram;
                self.on_doc_id_set_eviction(1, ram);
            }
        }
    }
}

/// The listener that drops a leaf's entries when its reader core closes.
///
/// Equivalent to the `this::clearCoreCacheKey` method reference Java registers
/// with `IndexReader.CacheHelper.addClosedListener`.
struct ClearCoreCacheKey {
    state: Arc<CacheState>,
    key: usize,
}

impl ClosedListener for ClearCoreCacheKey {
    fn on_close(&self, _key: CacheKey) -> Result<()> {
        clear_core_cache_key(&self.state, self.key);
        Ok(())
    }
}

/// Equivalent to `LRUQueryCache.clearCoreCacheKey(Object)`.
fn clear_core_cache_key(state: &Arc<CacheState>, core_key: usize) {
    let mut cache = state.cache.write().unwrap_or_else(PoisonError::into_inner);
    if let Some(leaf_cache) = cache.remove(&core_key) {
        state
            .ram_bytes_used
            .fetch_sub(HASHTABLE_RAM_BYTES_PER_ENTRY, Ordering::AcqRel);
        let num_entries = leaf_cache.cache.len();
        if num_entries > 0 {
            state.on_doc_id_set_eviction(num_entries as i64, leaf_cache.ram_bytes_used);
        } else {
            debug_assert_eq!(leaf_cache.ram_bytes_used, 0);
        }
    }
}

/// Equivalent to the private static
/// `LRUQueryCache.getRamBytesUsed(Query)`.
///
/// **Divergence from Lucene 10.5.0.** Java asks the query for its own footprint
/// when it implements `Accountable`, and otherwise assumes
/// `QUERY_DEFAULT_RAM_BYTES_USED`. This port's [`Query`] does not extend
/// `Accountable`, so the default always applies; the accounting is therefore
/// coarser for the few queries that report their own size, and identical for
/// every other one.
fn query_ram_bytes_used(_query: &dyn Query) -> i64 {
    LINKED_HASHTABLE_RAM_BYTES_PER_ENTRY + RamUsageEstimator::QUERY_DEFAULT_RAM_BYTES_USED
}

/// Returns the identity of a cache helper's key.
///
/// **Divergence from Lucene 10.5.0.** Java keys the per-leaf caches on an
/// `IdentityHashMap<IndexReader.CacheKey, LeafCache>`. This port's
/// [`CacheKey`] carries no data, so identity is the address of the key the
/// helper hands out — which every reader keeps in one place for the life of its
/// core, exactly as Java's instance is.
fn cache_key_identity(cache_helper: &dyn CacheHelper) -> usize {
    cache_helper.get_key() as *const CacheKey as usize
}

/// A [`QueryCache`] that evicts queries with a least-recently-used policy, in
/// order to stay under a maximum size and number of bytes used.
///
/// Equivalent to `org.apache.lucene.search.LRUQueryCache`. This type is
/// thread-safe.
///
/// Query eviction runs in linear time with the total number of segments that
/// have cache entries, so the cache works best with a
/// [caching policy](QueryCachingPolicy) that only caches on "large" segments,
/// and it is advisable not to share one cache across too many indexes.
///
/// The cache exposes global statistics — [`get_hit_count`](Self::get_hit_count),
/// [`get_miss_count`](Self::get_miss_count),
/// [`get_cache_size`](Self::get_cache_size),
/// [`get_cache_count`](Self::get_cache_count) and
/// [`get_eviction_count`](Self::get_eviction_count).
///
/// **Divergence from Lucene 10.5.0.** Java declares the statistics callbacks —
/// `onHit`, `onMiss`, `onQueryCache`, `onQueryEviction`, `onDocIdSetCache`,
/// `onDocIdSetEviction`, `onClear` — and the caching strategy hooks `cacheImpl`
/// and `tryPopulateCache` as `protected`, so that a subclass can gather
/// finer-grained statistics or defer the population. Rust has no subclassing,
/// so they are internal to this type and behave exactly as the base class does;
/// a caller who needs different behaviour writes their own [`QueryCache`].
#[derive(Debug)]
pub struct LRUQueryCache {
    state: Arc<CacheState>,
}

impl LRUQueryCache {
    /// Expert: creates a cache holding at most `max_size` queries and at most
    /// `max_ram_bytes_used` bytes, caching only on the leaves that satisfy
    /// `leaves_to_cache`.
    ///
    /// Equivalent to
    /// `new LRUQueryCache(int, long, Predicate<LeafReaderContext>, float)`.
    /// A clause whose cost is `skip_cache_factor` times more than the cost of
    /// the top-level query is not cached, so that queries are not slowed down
    /// too much.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] — with Java's message — when
    /// `skip_cache_factor` is less than 1, `NaN` included.
    pub fn with_predicate(
        max_size: i32,
        max_ram_bytes_used: i64,
        leaves_to_cache: LeavesToCache,
        skip_cache_factor: f32,
    ) -> Result<Self> {
        // Spelled as Lucene spells it: `>= 1 == false` lets a NaN through the
        // guard, because every comparison against NaN is false.
        if !(skip_cache_factor >= 1.0) {
            return Err(LuceneError::IllegalArgument(format!(
                "skipCacheFactor must be no less than 1, get {skip_cache_factor}"
            )));
        }
        Ok(Self {
            state: Arc::new(CacheState {
                max_size: max_size.max(0) as usize,
                max_ram_bytes_used,
                leaves_to_cache,
                cache: RwLock::new(HashMap::new()),
                unique_queries: Mutex::new(LruQueryMap::new()),
                skip_cache_factor: AtomicU32::new(skip_cache_factor.to_bits()),
                hit_count: AtomicI64::new(0),
                miss_count: AtomicI64::new(0),
                ram_bytes_used: AtomicI64::new(0),
                cache_count: AtomicI64::new(0),
                cache_size: AtomicI64::new(0),
            }),
        })
    }

    /// Creates a cache holding at most `max_size` queries and at most
    /// `max_ram_bytes_used` bytes.
    ///
    /// Equivalent to `new LRUQueryCache(int, long)`. Queries are only cached on
    /// leaves that have more than 10k documents and more than half of the
    /// average number of documents per leaf, which guarantees that every leaf
    /// of the upper tier is cached, and only clauses whose cost is at most 100x
    /// the cost of the top-level query are cached, so that latency does not
    /// suffer too much from caching.
    ///
    /// # Errors
    ///
    /// As [`with_predicate`](Self::with_predicate); the defaults are valid.
    pub fn new(max_size: i32, max_ram_bytes_used: i64) -> Result<Self> {
        Self::with_predicate(
            max_size,
            max_ram_bytes_used,
            min_segment_size_predicate(10_000),
            10.0,
        )
    }

    /// Returns the skip-cache factor.
    ///
    /// Equivalent to `LRUQueryCache.getSkipCacheFactor()`.
    pub fn get_skip_cache_factor(&self) -> f32 {
        self.state.skip_cache_factor()
    }

    /// Updates the skip-cache factor dynamically.
    ///
    /// Equivalent to `LRUQueryCache.setSkipCacheFactor(float)`.
    pub fn set_skip_cache_factor(&self, skip_cache_factor: f32) {
        self.state
            .skip_cache_factor
            .store(skip_cache_factor.to_bits(), Ordering::Release);
    }

    /// Removes every cache entry of the given core cache key.
    ///
    /// Equivalent to `LRUQueryCache.clearCoreCacheKey(Object)`; the key is the
    /// identity [`cache_key_identity`] computes for a leaf's core cache helper.
    pub fn clear_core_cache_key(&self, core_key: usize) {
        clear_core_cache_key(&self.state, core_key);
    }

    /// Removes every cache entry of the given query.
    ///
    /// Equivalent to `LRUQueryCache.clearQuery(Query)`.
    pub fn clear_query(&self, query: Arc<dyn Query>) {
        let mut cache = self
            .state
            .cache
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        let singleton = {
            let mut queries = self
                .state
                .unique_queries
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            queries.remove(&QueryKey::new(query))
        };
        if let Some(singleton) = singleton {
            self.state.on_eviction(&mut cache, &singleton);
        }
    }

    /// Clears the content of this cache.
    ///
    /// Equivalent to `LRUQueryCache.clear()`.
    pub fn clear(&self) {
        let mut cache = self
            .state
            .cache
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        cache.clear();
        self.state
            .unique_queries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        self.state.on_clear();
    }

    /// Returns the cached queries, least-recently-used first.
    ///
    /// Equivalent to the package-private `LRUQueryCache.cachedQueries()`, which
    /// exists for testing; it is `pub` here because the port has no package
    /// visibility.
    pub fn cached_queries(&self) -> Vec<Arc<dyn Query>> {
        let _guard = self
            .state
            .cache
            .read()
            .unwrap_or_else(PoisonError::into_inner);
        self.state
            .unique_queries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .queries()
    }

    /// Checks the cache's internal invariants.
    ///
    /// Equivalent to the package-private
    /// `LRUQueryCache.assertConsistent()`, which exists for testing and throws
    /// an `AssertionError`; this port reports the same conditions as
    /// [`LuceneError::IllegalState`].
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when an eviction is still
    /// required, when a leaf cache holds a key the top-level cache does not,
    /// or when the recomputed RAM usage or cache size disagrees with the
    /// tracked one.
    pub fn assert_consistent(&self) -> Result<()> {
        let cache = self
            .state
            .cache
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        let queries = self
            .state
            .unique_queries
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if self.state.requires_eviction(&queries) {
            return Err(LuceneError::IllegalState(format!(
                "requires evictions: size={}, maxSize={}, ramBytesUsed={}, maxRamBytesUsed={}",
                queries.len(),
                self.state.max_size,
                self.ram_bytes_used(),
                self.state.max_ram_bytes_used
            )));
        }
        for leaf_cache in cache.values() {
            for key in leaf_cache.cache.keys() {
                if !queries.entries.contains_key(key) {
                    return Err(LuceneError::IllegalState(
                        "One leaf cache contains more keys than the top-level cache".to_string(),
                    ));
                }
            }
        }
        let mut recomputed_ram_bytes_used = HASHTABLE_RAM_BYTES_PER_ENTRY * cache.len() as i64;
        for query in queries.queries() {
            recomputed_ram_bytes_used += query_ram_bytes_used(query.as_ref());
        }
        for leaf_cache in cache.values() {
            recomputed_ram_bytes_used +=
                HASHTABLE_RAM_BYTES_PER_ENTRY * leaf_cache.cache.len() as i64;
            for cached in leaf_cache.cache.values() {
                recomputed_ram_bytes_used += cached.ram_bytes_used();
            }
        }
        if recomputed_ram_bytes_used != self.ram_bytes_used() {
            return Err(LuceneError::IllegalState(format!(
                "ramBytesUsed mismatch : {} != {recomputed_ram_bytes_used}",
                self.ram_bytes_used()
            )));
        }

        let recomputed_cache_size: i64 = cache
            .values()
            .map(|leaf_cache| leaf_cache.cache.len() as i64)
            .sum();
        if recomputed_cache_size != self.get_cache_size() {
            return Err(LuceneError::IllegalState(format!(
                "cacheSize mismatch : {} != {recomputed_cache_size}",
                self.get_cache_size()
            )));
        }
        Ok(())
    }

    /// Returns how many times a query has been looked up in this cache.
    ///
    /// Equivalent to the `final LRUQueryCache.getTotalCount()`. The counter is
    /// incremented once per segment, so running a cached query once increments
    /// it by the number of segments the searcher wraps. By definition it is the
    /// sum of [`get_hit_count`](Self::get_hit_count) and
    /// [`get_miss_count`](Self::get_miss_count).
    pub fn get_total_count(&self) -> i64 {
        self.get_hit_count() + self.get_miss_count()
    }

    /// Returns how many of the lookups found a cached doc ID set.
    ///
    /// Equivalent to the `final LRUQueryCache.getHitCount()`.
    pub fn get_hit_count(&self) -> i64 {
        self.state.hit_count.load(Ordering::Acquire)
    }

    /// Returns how many of the lookups did not find the query in the cache.
    ///
    /// Equivalent to the `final LRUQueryCache.getMissCount()`.
    pub fn get_miss_count(&self) -> i64 {
        self.state.miss_count.load(Ordering::Acquire)
    }

    /// Returns the number of doc ID sets currently stored in the cache.
    ///
    /// Equivalent to the `final LRUQueryCache.getCacheSize()`.
    pub fn get_cache_size(&self) -> i64 {
        self.state.cache_size.load(Ordering::Acquire)
    }

    /// Returns the total number of cache entries ever generated and put in the
    /// cache.
    ///
    /// Equivalent to the `final LRUQueryCache.getCacheCount()`. It is highly
    /// desirable for the [hit count](Self::get_hit_count) to be much higher
    /// than this, since the opposite indicates that the cache makes efforts to
    /// cache queries that then do not get reused.
    pub fn get_cache_count(&self) -> i64 {
        self.state.cache_count.load(Ordering::Acquire)
    }

    /// Returns the number of cache entries removed, either to stay under the
    /// configured limits or because a segment was closed.
    ///
    /// Equivalent to the `final LRUQueryCache.getEvictionCount()`. A high
    /// number of evictions may mean that queries are not reused, or that the
    /// [caching policy](QueryCachingPolicy) caches too aggressively on
    /// near-real-time segments that get merged early.
    pub fn get_eviction_count(&self) -> i64 {
        self.get_cache_count() - self.get_cache_size()
    }
}

impl Accountable for LRUQueryCache {
    fn ram_bytes_used(&self) -> i64 {
        self.state.ram_bytes_used.load(Ordering::Acquire)
    }
}

impl QueryCache for LRUQueryCache {
    fn do_cache(
        &self,
        weight: Arc<dyn Weight>,
        policy: Arc<dyn QueryCachingPolicy>,
    ) -> Arc<dyn Weight> {
        let mut weight = weight;
        while let Some(inner) = weight.unwrap_cached() {
            weight = inner;
        }
        let query = weight.get_query();
        Arc::new(CachingWrapperWeight {
            base: ConstantScoreWeight::new(
                Arc::clone(&query),
                1.0,
                CachingWrapperWeightImpl {
                    state: Arc::clone(&self.state),
                    inner: Arc::clone(&weight),
                    policy,
                    used: AtomicBool::new(false),
                    query,
                },
            ),
            inner: weight,
        })
    }
}

/// The weight [`LRUQueryCache::do_cache`] wraps a weight in.
///
/// Equivalent to the inner class `LRUQueryCache.CachingWrapperWeight`, which
/// extends `ConstantScoreWeight`. Rust has no implementation inheritance, so
/// the base class is held rather than extended and its methods are forwarded.
#[derive(Debug)]
struct CachingWrapperWeight {
    base: ConstantScoreWeight<CachingWrapperWeightImpl>,
    inner: Arc<dyn Weight>,
}

impl SegmentCacheable for CachingWrapperWeight {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.base.is_cacheable(ctx)
    }
}

impl Weight for CachingWrapperWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        self.base.get_query()
    }

    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation> {
        self.base.explain(context, doc)
    }

    fn matches(&self, context: &LeafReaderContext, doc: i32) -> Result<Option<Arc<dyn Matches>>> {
        self.base.matches(context, doc)
    }

    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        self.base.scorer_supplier(context)
    }

    fn count(&self, context: &LeafReaderContext) -> Result<i32> {
        self.base.count(context)
    }

    fn unwrap_cached(&self) -> Option<Arc<dyn Weight>> {
        Some(Arc::clone(&self.inner))
    }
}

/// The leaf-level behaviour of a [`CachingWrapperWeight`].
///
/// Equivalent to the overrides `LRUQueryCache.CachingWrapperWeight` supplies
/// on top of `ConstantScoreWeight`.
#[derive(Debug)]
struct CachingWrapperWeightImpl {
    state: Arc<CacheState>,
    inner: Arc<dyn Weight>,
    policy: Arc<dyn QueryCachingPolicy>,
    /// Equivalent to the `AtomicBoolean used` field: `Weight.scorer` may be
    /// called from several threads when the searcher has an executor.
    used: AtomicBool,
    query: Arc<dyn Query>,
}

impl CachingWrapperWeightImpl {
    /// Equivalent to the private
    /// `CachingWrapperWeight.cacheEntryHasReasonableWorstCaseSize(int)`.
    fn cache_entry_has_reasonable_worst_case_size(&self, max_doc: i32) -> bool {
        // The worst case — a dense set — is a bit set that needs one bit per
        // document.
        let worst_case_ram_usage = (max_doc / 8) as i64;
        // Imagine the worst case, where a cache entry is larger than the cache
        // itself: not only would this entry be trashed immediately, it would
        // also evict every current entry. We therefore only cache on a reader
        // when there is room for five different filters on it, to avoid
        // excessive trashing.
        worst_case_ram_usage * 5 < self.state.max_ram_bytes_used
    }

    /// Whether this segment is eligible for caching, regardless of the query.
    ///
    /// Equivalent to the private
    /// `CachingWrapperWeight.shouldCache(LeafReaderContext)`.
    fn should_cache(&self, context: &LeafReaderContext) -> bool {
        let (top_level_max_doc, _) = top_level_stats(context);
        self.cache_entry_has_reasonable_worst_case_size(top_level_max_doc)
            && (self.state.leaves_to_cache)(context)
    }

    /// Equivalent to the `used.compareAndSet(false, true)` guard around
    /// `policy.onUse(getQuery())`.
    fn mark_used(&self) {
        if self
            .used
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            self.policy.on_use(self.query.as_ref());
        }
    }
}

impl ConstantScoreWeightImpl for CachingWrapperWeightImpl {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.inner.is_cacheable(ctx)
    }

    fn matches(&self, context: &LeafReaderContext, doc: i32) -> Result<Option<Arc<dyn Matches>>> {
        self.inner.matches(context, doc)
    }

    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        self.mark_used();

        if !self.inner.is_cacheable(context) {
            // This segment is not suitable for caching.
            return self.inner.scorer_supplier(context);
        }

        // Short-circuit: check whether this segment is eligible for caching
        // before taking a lock in `get`.
        if !self.should_cache(context) {
            return self.inner.scorer_supplier(context);
        }

        let Some(cache_helper) = context.leaf_reader().get_core_cache_helper() else {
            // This reader has no cache helper.
            return self.inner.scorer_supplier(context);
        };
        let reader_key = cache_key_identity(cache_helper.as_ref());

        // If the lock is already busy, prefer the uncached version over
        // waiting.
        let cached = match self.state.cache.try_read() {
            Err(_) => return self.inner.scorer_supplier(context),
            Ok(guard) => self.state.get(&guard, &self.query, reader_key),
        };

        let max_doc = context.leaf_reader().max_doc();
        let Some(cached) = cached else {
            if !self.policy.should_cache(self.query.as_ref())? {
                return self.inner.scorer_supplier(context);
            }
            let Some(supplier) = self.inner.scorer_supplier(context)? else {
                self.state.put_if_absent(
                    Arc::clone(&self.query),
                    CacheAndCount::empty(),
                    cache_helper.as_ref(),
                );
                return Ok(None);
            };
            let cost = supplier.cost();
            return Ok(Some(Box::new(ConstantScoreScorerSupplier::new(
                0.0,
                ScoreMode::COMPLETE_NO_SCORES,
                max_doc,
                PopulatingIteratorSupplier {
                    state: Arc::clone(&self.state),
                    inner: Arc::clone(&self.inner),
                    cache_helper,
                    supplier,
                    cost,
                    max_doc,
                },
            ))));
        };

        if cached.is_empty_marker {
            return Ok(None);
        }
        let disi = cached.iterator()?;
        Ok(Some(Box::new(ConstantScoreScorerSupplier::from_iterator(
            ScorerIterator::Simple(disi),
            0.0,
            ScoreMode::COMPLETE_NO_SCORES,
            max_doc,
        ))))
    }

    fn count(&self, context: &LeafReaderContext) -> Result<i32> {
        // Our cache will not have an accurate count when there are deletions.
        // `IndexReader.hasDeletions()` is `maxDoc() - numDocs() > 0`; the port
        // reaches `IndexReader` through a blanket implementation that a
        // `dyn LeafReader` cannot satisfy, so the two counts are read directly.
        let reader = context.leaf_reader();
        if reader.max_doc() - reader.num_docs() > 0 {
            return self.inner.count(context);
        }

        // Otherwise check whether the count is in the cache.
        self.mark_used();

        if !self.inner.is_cacheable(context) {
            // This segment is not suitable for caching.
            return self.inner.count(context);
        }

        // Short-circuit: check whether this segment is eligible for caching
        // before taking a lock in `get`.
        if !self.should_cache(context) {
            return self.inner.count(context);
        }

        let Some(cache_helper) = context.leaf_reader().get_core_cache_helper() else {
            // This reader has no cache helper.
            return self.inner.count(context);
        };
        let reader_key = cache_key_identity(cache_helper.as_ref());

        // If the lock is already busy, prefer the uncached version over
        // waiting.
        let cached = match self.state.cache.try_read() {
            Err(_) => return self.inner.count(context),
            Ok(guard) => self.state.get(&guard, &self.query, reader_key),
        };
        match cached {
            // Cached.
            Some(cached) => Ok(cached.count()),
            // Not cached: check whether the wrapped weight can count quickly,
            // and use that.
            None => self.inner.count(context),
        }
    }
}

/// The iteration of the supplier a cache miss produces.
///
/// Equivalent to the anonymous `ConstantScoreScorerSupplier` of
/// `LRUQueryCache.CachingWrapperWeight.scorerSupplier`.
struct PopulatingIteratorSupplier {
    state: Arc<CacheState>,
    inner: Arc<dyn Weight>,
    cache_helper: Box<dyn CacheHelper>,
    supplier: Box<dyn ScorerSupplier>,
    cost: i64,
    max_doc: i32,
}

impl Debug for PopulatingIteratorSupplier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PopulatingIteratorSupplier")
            .field("cost", &self.cost)
            .field("maxDoc", &self.max_doc)
            .finish_non_exhaustive()
    }
}

impl ConstantScoreIteratorSupplier for PopulatingIteratorSupplier {
    fn iterator(&mut self, lead_cost: i64) -> Result<ScorerIterator> {
        // Skip the cache operation, which would slow the query down too much.
        if self.cost as f32 / self.state.skip_cache_factor() > lead_cost as f32 {
            return Ok(into_scorer_iterator(self.supplier.get(lead_cost)?));
        }
        let cached = try_populate_cache(
            &self.state,
            self.cache_helper.as_ref(),
            &self.inner,
            self.supplier.as_mut(),
            self.max_doc,
        )?;
        // The cache is not available, so use the uncached iterator.
        let Some(cached) = cached else {
            return Ok(into_scorer_iterator(self.supplier.get(lead_cost)?));
        };
        // `DocIdSet.iterator()` may return an empty iterator; a non-null
        // iterator is what is wanted here, which the port's sets always give.
        Ok(ScorerIterator::Simple(cached.iterator()?))
    }

    fn cost(&self) -> i64 {
        self.cost
    }
}

/// Populates the cache for the given scorer supplier and leaf.
///
/// Equivalent to the protected
/// `LRUQueryCache.tryPopulateCache(IndexReader.CacheHelper, Weight, ScorerSupplier, LeafReaderContext)`,
/// whose base implementation always populates and therefore always returns a
/// value.
fn try_populate_cache(
    state: &Arc<CacheState>,
    cache_helper: &dyn CacheHelper,
    weight: &Arc<dyn Weight>,
    scorer_supplier: &mut dyn ScorerSupplier,
    max_doc: i32,
) -> Result<Option<CacheAndCount>> {
    let cached = cache_impl(scorer_supplier.bulk_scorer()?, max_doc)?;
    state.put_if_absent(weight.get_query(), cached.clone(), cache_helper);
    Ok(Some(cached))
}

/// Builds the cached doc ID set of one leaf.
///
/// Equivalent to the protected
/// `LRUQueryCache.cacheImpl(BulkScorer, int)`, which uses a roaring set below
/// 1% density and a bit set otherwise.
fn cache_impl(mut scorer: Box<dyn BulkScorer>, max_doc: i32) -> Result<CacheAndCount> {
    if scorer.cost() * 100 >= max_doc as i64 {
        // A fixed bit set is faster for dense sets and enables the
        // random-access optimisation in the conjunction machinery.
        cache_into_bit_set(scorer.as_mut(), max_doc)
    } else {
        cache_into_roaring_doc_id_set(scorer.as_mut(), max_doc)
    }
}

/// Equivalent to the private static
/// `LRUQueryCache.cacheIntoBitSet(BulkScorer, int)`.
fn cache_into_bit_set(scorer: &mut dyn BulkScorer, max_doc: i32) -> Result<CacheAndCount> {
    let mut collector = BitSetCollector {
        bit_set: FixedBitSet::new(max_doc.max(0) as usize),
        count: 0,
        buffer: Vec::new(),
    };
    let outcome = scorer.score(&mut collector, None, 0, NO_MORE_DOCS);
    collection_outcome(outcome)?;
    let count = collector.count;
    let set: Arc<dyn BitSet> = Arc::new(collector.bit_set);
    Ok(CacheAndCount::new(
        Arc::new(BitDocIdSet::new(set, count as i64)?),
        count,
    ))
}

/// Equivalent to the private static
/// `LRUQueryCache.cacheIntoRoaringDocIdSet(BulkScorer, int)`.
fn cache_into_roaring_doc_id_set(
    scorer: &mut dyn BulkScorer,
    max_doc: i32,
) -> Result<CacheAndCount> {
    let mut collector = RoaringCollector {
        builder: RoaringDocIdSet::builder(max_doc.max(0) as usize),
        error: None,
    };
    let outcome = scorer.score(&mut collector, None, 0, NO_MORE_DOCS);
    collection_outcome(outcome)?;
    if let Some(error) = collector.error {
        return Err(error);
    }
    let cache = collector.builder.build();
    let cardinality = cache.cardinality() as i32;
    Ok(CacheAndCount::new(
        Arc::new(RoaringCachedDocIdSet(cache)),
        cardinality,
    ))
}

/// Turns a bulk-scoring outcome into the error type the cache reports.
///
/// A `CollectionTerminatedException` cannot escape here — the collectors below
/// never throw one — and a timeout aborts the search in Java too.
fn collection_outcome(outcome: CollectionResult<i32>) -> Result<()> {
    match outcome {
        Ok(_) => Ok(()),
        Err(CollectionError::Lucene(error)) => Err(error),
        Err(other) => Err(LuceneError::IllegalState(other.to_string())),
    }
}

/// The collector that fills a bit set.
///
/// Equivalent to the anonymous `LeafCollector` of `cacheIntoBitSet`.
struct BitSetCollector {
    bit_set: FixedBitSet,
    count: i32,
    buffer: Vec<i32>,
}

impl LeafCollector for BitSetCollector {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        self.count += 1;
        self.bit_set.set(doc as usize);
        Ok(())
    }

    fn collect_range(
        &mut self,
        min: i32,
        max: i32,
        _scorer: &mut dyn Scorable,
    ) -> CollectionResult<()> {
        self.count += max - min;
        self.bit_set.set_range(min as usize, max as usize);
        Ok(())
    }

    fn collect_stream(
        &mut self,
        stream: &mut dyn DocIdStream,
        _scorer: &mut dyn Scorable,
    ) -> CollectionResult<()> {
        if self.buffer.is_empty() {
            self.buffer = vec![0; 128];
        }
        loop {
            let c = stream.into_array(&mut self.buffer);
            if c == 0 {
                return Ok(());
            }
            for doc in &self.buffer[..c] {
                self.bit_set.set(*doc as usize);
            }
            self.count += c as i32;
        }
    }
}

/// The collector that fills a roaring set.
///
/// Equivalent to the anonymous `LeafCollector` of
/// `cacheIntoRoaringDocIdSet`.
struct RoaringCollector {
    builder: RoaringDocIdSetBuilder,
    /// The first error the builder raised.
    ///
    /// **Divergence from Lucene 10.5.0.** Java's `RoaringDocIdSet.Builder.add`
    /// returns the builder and throws nothing; this port's returns a
    /// [`Result`], and the collector contract has no channel for it, so the
    /// first failure is recorded and reported once collection ends.
    error: Option<LuceneError>,
}

impl RoaringCollector {
    fn record(&mut self, outcome: Result<()>) {
        if let Err(error) = outcome {
            if self.error.is_none() {
                self.error = Some(error);
            }
        }
    }
}

impl LeafCollector for RoaringCollector {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        let outcome = self.builder.add(doc as usize);
        self.record(outcome);
        Ok(())
    }

    fn collect_range(
        &mut self,
        min: i32,
        max: i32,
        _scorer: &mut dyn Scorable,
    ) -> CollectionResult<()> {
        let outcome = self.builder.add_range(min as usize, max as usize);
        self.record(outcome);
        Ok(())
    }

    fn collect_stream(
        &mut self,
        stream: &mut dyn DocIdStream,
        _scorer: &mut dyn Scorable,
    ) -> CollectionResult<()> {
        let mut errors: Vec<LuceneError> = Vec::new();
        {
            let builder = &mut self.builder;
            let mut consumer = |doc: i32| {
                if let Err(error) = builder.add(doc as usize) {
                    errors.push(error);
                }
                Ok(())
            };
            stream.for_each(&mut consumer)?;
        }
        for error in errors {
            self.record(Err(error));
        }
        Ok(())
    }
}
