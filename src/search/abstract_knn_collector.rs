//! The default kNN collector base, ported from
//! `org.apache.lucene.search.AbstractKnnCollector`.

#![deny(unsafe_code)]

use crate::search::knn::{KnnCollector, KnnSearchStrategy};

/// The state and the `final` behaviour of `AbstractKnnCollector`.
///
/// Equivalent to the fields of the abstract class
/// `org.apache.lucene.search.AbstractKnnCollector` — `visitedCount`,
/// `visitLimit`, `searchStrategy` and `k` — together with the five `final`
/// methods that read them.
///
/// **Divergence from Lucene 10.5.0.** Rust has no implementation inheritance,
/// so the base class splits in two: this struct, which an implementation
/// embeds and forwards to, and the [`AbstractKnnCollector`] trait, which is the
/// type callers name. Neither the state nor the behaviour changes.
#[derive(Debug, Clone)]
pub struct AbstractKnnCollectorState {
    /// The number of vectors visited so far.
    ///
    /// Equivalent to the `protected long visitedCount` field.
    pub visited_count: i64,
    visit_limit: i64,
    search_strategy: Option<KnnSearchStrategy>,
    k: i32,
}

impl AbstractKnnCollectorState {
    /// Creates the base state.
    ///
    /// Equivalent to the protected
    /// `AbstractKnnCollector(int, long, KnnSearchStrategy)` constructor; pass
    /// `None` for the two-argument form, which passes a `null` strategy.
    pub fn new(k: i32, visit_limit: i64, search_strategy: Option<KnnSearchStrategy>) -> Self {
        Self {
            visited_count: 0,
            visit_limit,
            search_strategy,
            k,
        }
    }

    /// Returns whether the visit limit has been reached.
    ///
    /// Equivalent to the `final AbstractKnnCollector.earlyTerminated()`.
    pub fn early_terminated(&self) -> bool {
        self.visited_count >= self.visit_limit
    }

    /// Increments the visited-vector count.
    ///
    /// Equivalent to the `final AbstractKnnCollector.incVisitedCount(int)`,
    /// whose assertion that `count >= 0` is reproduced as a debug assertion.
    pub fn inc_visited_count(&mut self, count: i32) {
        debug_assert!(count >= 0);
        self.visited_count += count as i64;
    }

    /// Returns the current visited-vector count.
    ///
    /// Equivalent to the `final AbstractKnnCollector.visitedCount()`.
    pub fn visited_count(&self) -> i64 {
        self.visited_count
    }

    /// Returns the visited-vector limit.
    ///
    /// Equivalent to the `final AbstractKnnCollector.visitLimit()`.
    pub fn visit_limit(&self) -> i64 {
        self.visit_limit
    }

    /// Returns the expected number of collected results.
    ///
    /// Equivalent to the `final AbstractKnnCollector.k()`.
    pub fn k(&self) -> i32 {
        self.k
    }

    /// Returns the search strategy, if one was supplied.
    ///
    /// Equivalent to `AbstractKnnCollector.getSearchStrategy()`.
    pub fn get_search_strategy(&self) -> Option<&KnnSearchStrategy> {
        self.search_strategy.as_ref()
    }
}

/// The default implementation of a kNN collector: one that knows how many
/// results it has collected so far.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.search.AbstractKnnCollector`, as seen by its callers —
/// [`MultiLeafKnnCollector`](crate::search::knn::MultiLeafKnnCollector) takes
/// one and reads `numCollected()` from it. The state and the `final` methods
/// live in [`AbstractKnnCollectorState`].
pub trait AbstractKnnCollector: KnnCollector {
    /// Returns the number of results collected so far.
    ///
    /// Equivalent to the abstract `AbstractKnnCollector.numCollected()`.
    fn num_collected(&self) -> i32;
}
