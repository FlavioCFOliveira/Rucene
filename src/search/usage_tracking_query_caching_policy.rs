//! Usage-based caching policy, ported from
//! `org.apache.lucene.search.UsageTrackingQueryCachingPolicy`.

#![deny(unsafe_code)]

use std::sync::{Mutex, PoisonError};

use crate::error::Result;
use crate::search::boolean_query::BooleanQuery;
use crate::search::disjunction_max_query::DisjunctionMaxQuery;
use crate::search::field_exists_query::FieldExistsQuery;
use crate::search::match_all_docs_query::MatchAllDocsQuery;
use crate::search::match_no_docs_query::MatchNoDocsQuery;
use crate::search::point_in_set_query::PointInSetQuery;
use crate::search::point_range_query::PointRangeQuery;
use crate::search::query::Query;
use crate::search::query_cache::QueryCachingPolicy;
use crate::search::term_in_set_query::TermInSetQuery;
use crate::search::term_query::TermQuery;
use crate::util::FrequencyTrackingRingBuffer;

/// The hash code used as a sentinel in the ring buffer.
///
/// Equivalent to `UsageTrackingQueryCachingPolicy.SENTINEL`.
const SENTINEL: i32 = i32::MIN;

/// Whether the query is one of the point queries.
///
/// Equivalent to the private
/// `UsageTrackingQueryCachingPolicy.isPointQuery(Query)`.
///
/// **Divergence from Lucene 10.5.0.** Java walks the class hierarchy looking
/// for a class whose simple name starts with `Point` and ends with `Query`,
/// because `IntPoint.newRangeQuery` and friends return *anonymous subclasses*
/// of `PointRangeQuery`. Rust has no subclassing — the port's point queries
/// hold a formatter instead of overriding a method — so the two concrete types
/// are tested directly, which recognises exactly the same queries.
fn is_point_query(query: &dyn Query) -> bool {
    query.as_any().is::<PointRangeQuery>() || query.as_any().is::<PointInSetQuery>()
}

/// Whether the query is costly to turn into a doc ID set.
///
/// Equivalent to the package-private
/// `UsageTrackingQueryCachingPolicy.isCostly(Query)`. This does not measure the
/// cost of iterating the filter — [`DocIdSetIterator::cost`] already answers
/// that — but the cost of building the doc ID set in the first place.
///
/// [`DocIdSetIterator::cost`]: crate::search::DocIdSetIterator::cost
pub fn is_costly(query: &dyn Query) -> bool {
    query.is_multi_term_query()
        || query
            .as_any()
            .is::<crate::search::MultiTermQueryConstantScoreBlendedWrapper>()
        || query
            .as_any()
            .is::<crate::search::MultiTermQueryConstantScoreWrapper>()
        || query.as_any().is::<TermInSetQuery>()
        || is_point_query(query)
}

/// Whether the query should never be cached.
///
/// Equivalent to the private
/// `UsageTrackingQueryCachingPolicy.shouldNeverCache(Query)`.
fn should_never_cache(query: &dyn Query) -> bool {
    // We do not bother caching term queries, since they are already plenty
    // fast.
    if query.as_any().is::<TermQuery>() {
        return true;
    }

    // We do not bother caching field-exists queries either, for the same
    // reason.
    if query.as_any().is::<FieldExistsQuery>() {
        return true;
    }

    // MatchAllDocsQuery has an iterator that is faster than what a bit set
    // could do.
    if query.as_any().is::<MatchAllDocsQuery>() {
        return true;
    }

    // For the queries below it is cheap to notice that they cannot match any
    // doc, so we do not bother caching them.
    if query.as_any().is::<MatchNoDocsQuery>() {
        return true;
    }

    if let Some(bq) = query.as_any().downcast_ref::<BooleanQuery>() {
        if bq.clauses().is_empty() {
            return true;
        }
    }

    if let Some(dmq) = query.as_any().downcast_ref::<DisjunctionMaxQuery>() {
        if dmq.get_disjuncts().is_empty() {
            return true;
        }
    }

    false
}

/// A [`QueryCachingPolicy`] that tracks the usage statistics of the
/// recently-used filters in order to decide which are worth caching.
///
/// Equivalent to
/// `org.apache.lucene.search.UsageTrackingQueryCachingPolicy`.
#[derive(Debug)]
pub struct UsageTrackingQueryCachingPolicy {
    recently_used_filters: Mutex<FrequencyTrackingRingBuffer>,
}

impl Default for UsageTrackingQueryCachingPolicy {
    fn default() -> Self {
        Self::new().expect("INVARIANT: the default history size of 256 is valid")
    }
}

impl UsageTrackingQueryCachingPolicy {
    /// Expert: creates an instance with a configurable history size.
    ///
    /// Equivalent to
    /// `new UsageTrackingQueryCachingPolicy(int)`. Beware of passing too large
    /// a history: either
    /// [`min_frequency_to_cache`](Self::min_frequency_to_cache) returns low
    /// values, and rarely-used filters get cached, which hurts performance, or
    /// it returns high values that grow with the history, and filters are slow
    /// to make it into the cache.
    ///
    /// # Errors
    ///
    /// Propagates the [`FrequencyTrackingRingBuffer`] construction error for an
    /// invalid history size.
    pub fn with_history_size(history_size: i32) -> Result<Self> {
        Ok(Self {
            recently_used_filters: Mutex::new(FrequencyTrackingRingBuffer::new(
                history_size.max(0) as usize,
                SENTINEL,
            )?),
        })
    }

    /// Creates an instance with a history size of 256, a good default for most
    /// cases.
    ///
    /// Equivalent to `new UsageTrackingQueryCachingPolicy()`.
    ///
    /// # Errors
    ///
    /// As [`with_history_size`](Self::with_history_size).
    pub fn new() -> Result<Self> {
        Self::with_history_size(256)
    }

    /// Returns how many times a filter must appear in the history before it is
    /// cached.
    ///
    /// Equivalent to
    /// `UsageTrackingQueryCachingPolicy.minFrequencyToCache(Query)`. The
    /// implementation returns 2 for the filters that have to evaluate against
    /// the entire index to build a
    /// [`DocIdSetIterator`](crate::search::DocIdSetIterator) — a multi-term
    /// query, a point query or a term-in-set query — and 5 for the others.
    ///
    /// **Divergence from Lucene 10.5.0.** Java declares the method `protected`
    /// so that a subclass can change the thresholds. Rust has no subclassing,
    /// so it is a public method of this type; a caller who needs different
    /// thresholds writes their own [`QueryCachingPolicy`].
    pub fn min_frequency_to_cache(&self, query: &dyn Query) -> i32 {
        if is_costly(query) {
            2
        } else {
            // Default: cache after the filter has been seen 5 times.
            let mut min_frequency = 5;
            if query.as_any().is::<BooleanQuery>() || query.as_any().is::<DisjunctionMaxQuery>() {
                // Say you keep reusing a boolean query that looks like "A OR B"
                // and never use A and B outside that context. Five uses later
                // we would cache A, B and "A OR B", which is wasteful, so
                // compound queries are cached a bit earlier and only "A OR B"
                // ends up in the cache.
                min_frequency -= 1;
            }
            min_frequency
        }
    }

    /// Returns how many times the query appears in the tracked history.
    ///
    /// Equivalent to the package-private
    /// `UsageTrackingQueryCachingPolicy.frequency(Query)`.
    pub fn frequency(&self, query: &dyn Query) -> i32 {
        debug_assert!(!query.as_any().is::<crate::search::BoostQuery>());
        debug_assert!(!query.as_any().is::<crate::search::ConstantScoreQuery>());

        // Compute the hash outside the lock, in case it is somewhat expensive.
        let hash_code = query_hash_code(query);

        self.recently_used_filters
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .frequency(hash_code)
    }
}

/// Narrows a query hash to the `int` the ring buffer tracks.
///
/// **Divergence from Lucene 10.5.0.** Java's `Query.hashCode()` is an `int`;
/// this port's [`Query::query_hash`] is a `u64`, because Rust's hashers produce
/// one. The buffer tracks 32-bit keys, so the hash is folded the way Java folds
/// a `long` into an `int` in `Long.hashCode`. A fold can only ever create a
/// collision, which the class already documents as acceptable: "this may cause
/// rare false positives, but at worst this just means we cache a query that was
/// not in fact used enough".
fn query_hash_code(query: &dyn Query) -> i32 {
    let hash = query.query_hash();
    ((hash ^ (hash >> 32)) as u32) as i32
}

impl QueryCachingPolicy for UsageTrackingQueryCachingPolicy {
    fn on_use(&self, query: &dyn Query) {
        debug_assert!(!query.as_any().is::<crate::search::BoostQuery>());
        debug_assert!(!query.as_any().is::<crate::search::ConstantScoreQuery>());

        if should_never_cache(query) {
            return;
        }

        // Compute the hash outside the lock, in case it is somewhat expensive.
        let hash_code = query_hash_code(query);

        // We only track hash codes, to avoid holding references to possibly
        // large queries; this may cause rare false positives, but at worst it
        // just means we cache a query that was not in fact used enough.
        self.recently_used_filters
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .add(hash_code);
    }

    fn should_cache(&self, query: &dyn Query) -> Result<bool> {
        if should_never_cache(query) {
            return Ok(false);
        }
        let frequency = self.frequency(query);
        let min_frequency = self.min_frequency_to_cache(query);
        Ok(frequency >= min_frequency)
    }
}
