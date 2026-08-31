//! Query caching hooks, ported from `org.apache.lucene.search.QueryCache` and
//! `org.apache.lucene.search.QueryCachingPolicy`.
//!
//! **Divergence from Lucene 10.5.0.** Java installs concrete defaults —
//! `LRUQueryCache` holding up to 1000 queries or 5% of the heap, and
//! `UsageTrackingQueryCachingPolicy`. Neither is part of the query-execution
//! spine and neither is ported yet, so
//! [`IndexSearcher`](crate::search::IndexSearcher) starts with no cache and no
//! policy, and [`IndexSearcher::create_weight`](crate::search::IndexSearcher::create_weight)
//! simply returns the uncached weight. The hook itself is faithful: install a
//! cache and a policy and the weight is wrapped exactly where Java wraps it.

#![deny(unsafe_code)]

use std::fmt::Debug;
use std::sync::Arc;

use crate::error::Result;
use crate::search::query::Query;
use crate::search::weight::Weight;

/// A cache for queries.
///
/// Equivalent to `org.apache.lucene.search.QueryCache`.
pub trait QueryCache: Send + Sync + Debug {
    /// Returns a wrapper around `weight` that will cache matching docs
    /// per-segment according to `policy`.
    ///
    /// Equivalent to `QueryCache.doCache(Weight, QueryCachingPolicy)`. The
    /// returned weight is only equivalent to the original when scores are not
    /// needed; see [`Collector::score_mode`](crate::search::Collector::score_mode).
    fn do_cache(
        &self,
        weight: Arc<dyn Weight>,
        policy: Arc<dyn QueryCachingPolicy>,
    ) -> Arc<dyn Weight>;
}

/// A policy defining which filters should be cached.
///
/// Equivalent to `org.apache.lucene.search.QueryCachingPolicy`.
/// Implementations must be thread-safe.
pub trait QueryCachingPolicy: Send + Sync + Debug {
    /// Callback invoked every time a cached filter is used.
    ///
    /// Equivalent to `QueryCachingPolicy.onUse(Query)`. This is typically
    /// useful when the policy wants to track usage statistics in order to make
    /// decisions.
    fn on_use(&self, query: &dyn Query);

    /// Whether the given query is worth caching.
    ///
    /// Equivalent to `QueryCachingPolicy.shouldCache(Query)`. The
    /// [`QueryCache`] calls it to know whether to cache: it first attempts to
    /// load a doc ID set from the cache, and if the set is not cached yet and
    /// this method returns `true`, a cache entry is generated. Otherwise an
    /// uncached scorer is returned.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while deciding.
    fn should_cache(&self, query: &dyn Query) -> Result<bool>;
}
