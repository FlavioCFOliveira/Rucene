//! k-nearest-neighbour search strategies and collection primitives.
//!
//! This module is the port of the `org.apache.lucene.search.knn` package —
//! [`KnnSearchStrategy`], [`KnnCollectorManager`], [`TopKnnCollectorManager`]
//! and [`MultiLeafKnnCollector`].
//!
//! **Divergence from Lucene 10.5.0.** It also holds [`KnnCollector`] and
//! [`TopKnnCollector`], which Java declares in `org.apache.lucene.search`
//! itself. They were ported here before the search package existed, and moving
//! them now would break every codec and HNSW caller for no functional gain, so
//! they stay and are re-exported from [`crate::search`] under their Lucene
//! names. [`AbstractKnnCollector`], the third member of that family, does live
//! in [`crate::search::abstract_knn_collector`].

#![deny(unsafe_code)]

pub mod knn_collector_manager;
pub mod multi_leaf_knn_collector;
pub mod top_knn_collector_manager;

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::search::abstract_knn_collector::AbstractKnnCollectorState;
use crate::search::doc_id_set_iterator::{empty, DocIdSetIterator};
use crate::search::hnsw_queue_saturation_collector::HnswQueueSaturationState;
use crate::search::score_doc::ScoreDoc;
use crate::search::total_hits::{TotalHits, TotalHitsRelation};
use crate::util::hnsw::NeighborQueue;

pub use knn_collector_manager::KnnCollectorManager;
pub use multi_leaf_knn_collector::MultiLeafKnnCollector;
pub use top_knn_collector_manager::TopKnnCollectorManager;

/// The result of a kNN search.
///
/// Re-exported from [`crate::search::top_docs`]: this module used to carry an
/// empty placeholder of the same name, which is now gone.
pub use crate::search::top_docs::TopDocs;

/// The default value of [`Hnsw::filtered_search_threshold`].
///
/// Equivalent to `KnnSearchStrategy.DEFAULT_FILTERED_SEARCH_THRESHOLD`.
pub const DEFAULT_FILTERED_SEARCH_THRESHOLD: i32 = 0;

/// Additional configuration handed to a kNN search.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.search.knn.KnnSearchStrategy` and its three nested
/// subclasses.
///
/// **Divergence from Lucene 10.5.0.** Java models the family with inheritance —
/// `Hnsw`, `Seeded` and `Patience extends Hnsw`. Rust has no implementation
/// inheritance, so the family is a closed enum; because it is closed and every
/// variant is reproduced faithfully, nothing observable changes. The
/// `Patience`-is-an-`Hnsw` relationship is expressed by [`Patience`] embedding
/// an [`Hnsw`] and by [`KnnSearchStrategy::filtered_search_threshold`] and
/// [`KnnSearchStrategy::use_filtered_search`] answering for both variants.
#[derive(Debug, Clone)]
pub enum KnnSearchStrategy {
    /// A strategy for kNN search that uses HNSW.
    ///
    /// Equivalent to `KnnSearchStrategy.Hnsw`.
    Hnsw(Hnsw),
    /// A strategy for kNN search that uses a set of entry points to start the
    /// search.
    ///
    /// Equivalent to `KnnSearchStrategy.Seeded`.
    Seeded(Seeded),
    /// A strategy for kNN search on HNSW that early-exits when the nearest
    /// neighbour collection rate saturates.
    ///
    /// Equivalent to `KnnSearchStrategy.Patience`.
    Patience(Patience),
}

impl KnnSearchStrategy {
    /// The default HNSW strategy.
    ///
    /// Equivalent to `KnnSearchStrategy.Hnsw.DEFAULT`, which is
    /// `new Hnsw(DEFAULT_FILTERED_SEARCH_THRESHOLD)`.
    pub fn hnsw_default() -> Self {
        KnnSearchStrategy::Hnsw(Hnsw::DEFAULT)
    }

    /// Signals the processing of the next block of vectors.
    ///
    /// Equivalent to `KnnSearchStrategy.nextVectorsBlock()`: a no-op for
    /// [`Hnsw`], delegation to the wrapped strategy for [`Seeded`], and one
    /// candidate step of the patience counter for [`Patience`].
    ///
    /// **Divergence from Lucene 10.5.0.** Java declares the method on a
    /// mutable object reached from the collector; this port takes `&self`
    /// because [`KnnCollector::get_search_strategy`] hands out a shared
    /// borrow, and the patience counter lives behind the shared lock the
    /// collector also holds.
    pub fn next_vectors_block(&self) {
        match self {
            KnnSearchStrategy::Hnsw(_) => {}
            KnnSearchStrategy::Seeded(seeded) => seeded.original_strategy.next_vectors_block(),
            KnnSearchStrategy::Patience(patience) => patience.next_candidate(),
        }
    }

    /// Returns the filtered-search threshold of the HNSW-based strategies.
    ///
    /// Equivalent to `KnnSearchStrategy.Hnsw.filteredSearchThreshold()`, which
    /// `Patience` inherits. Returns `None` for [`KnnSearchStrategy::Seeded`],
    /// which is not an `Hnsw` in Java either.
    pub fn filtered_search_threshold(&self) -> Option<i32> {
        match self {
            KnnSearchStrategy::Hnsw(hnsw) => Some(hnsw.filtered_search_threshold()),
            KnnSearchStrategy::Seeded(_) => None,
            KnnSearchStrategy::Patience(patience) => Some(patience.filtered_search_threshold()),
        }
    }

    /// Whether to use filtered search, given the ratio of vectors that pass the
    /// filter.
    ///
    /// Equivalent to the `final KnnSearchStrategy.Hnsw.useFilteredSearch(float)`.
    /// Returns `false` for [`KnnSearchStrategy::Seeded`], which does not expose
    /// the method in Java.
    pub fn use_filtered_search(&self, ratio_passing_filter: f32) -> bool {
        debug_assert!((0.0..=1.0).contains(&ratio_passing_filter));
        match self.filtered_search_threshold() {
            Some(threshold) => ratio_passing_filter * 100.0 < threshold as f32,
            None => false,
        }
    }
}

impl PartialEq for KnnSearchStrategy {
    /// Equivalent to the `equals` overrides of the three subclasses; as in
    /// Java, two strategies of different classes are never equal.
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (KnnSearchStrategy::Hnsw(a), KnnSearchStrategy::Hnsw(b)) => a == b,
            (KnnSearchStrategy::Seeded(a), KnnSearchStrategy::Seeded(b)) => a == b,
            (KnnSearchStrategy::Patience(a), KnnSearchStrategy::Patience(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for KnnSearchStrategy {}

impl Hash for KnnSearchStrategy {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            KnnSearchStrategy::Hnsw(hnsw) => {
                state.write_u8(0);
                hnsw.hash(state);
            }
            KnnSearchStrategy::Seeded(seeded) => {
                state.write_u8(1);
                seeded.hash(state);
            }
            KnnSearchStrategy::Patience(patience) => {
                state.write_u8(2);
                patience.hash(state);
            }
        }
    }
}

/// A strategy for kNN search that uses HNSW.
///
/// Equivalent to `org.apache.lucene.search.knn.KnnSearchStrategy.Hnsw`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Hnsw {
    filtered_search_threshold: i32,
}

impl Hnsw {
    /// The default HNSW strategy.
    ///
    /// Equivalent to `KnnSearchStrategy.Hnsw.DEFAULT`.
    pub const DEFAULT: Hnsw = Hnsw {
        filtered_search_threshold: DEFAULT_FILTERED_SEARCH_THRESHOLD,
    };

    /// Creates a new HNSW strategy.
    ///
    /// Equivalent to `new KnnSearchStrategy.Hnsw(int)`. `filtered_search_threshold`
    /// is a percentage from 0 to 100, where 0 means never use filtered search
    /// and 100 means always use it.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`](crate::error::LuceneError::IllegalArgument)
    /// — with Java's message — when the threshold is outside `[0, 100]`.
    pub fn new(filtered_search_threshold: i32) -> crate::error::Result<Self> {
        if !(0..=100).contains(&filtered_search_threshold) {
            return Err(crate::error::LuceneError::IllegalArgument(
                "filteredSearchThreshold must be >= 0 and <= 100".to_string(),
            ));
        }
        Ok(Self {
            filtered_search_threshold,
        })
    }

    /// Returns the filtered-search threshold.
    ///
    /// Equivalent to `KnnSearchStrategy.Hnsw.filteredSearchThreshold()`.
    pub fn filtered_search_threshold(&self) -> i32 {
        self.filtered_search_threshold
    }

    /// Whether to use filtered search, given the ratio of vectors that pass the
    /// filter.
    ///
    /// Equivalent to the `final KnnSearchStrategy.Hnsw.useFilteredSearch(float)`.
    pub fn use_filtered_search(&self, ratio_passing_filter: f32) -> bool {
        debug_assert!((0.0..=1.0).contains(&ratio_passing_filter));
        ratio_passing_filter * 100.0 < self.filtered_search_threshold as f32
    }
}

/// A strategy for kNN search that uses a set of entry points to start the
/// search.
///
/// Equivalent to `org.apache.lucene.search.knn.KnnSearchStrategy.Seeded`.
///
/// **Divergence from Lucene 10.5.0.** Java holds the entry points in a bare
/// `DocIdSetIterator` field, aliased by whoever consumes them. A
/// [`Query`](crate::search::Query) must be `Send + Sync` in this port, and the
/// iterator is advanced while it is read, so the entry points live behind an
/// [`Arc<Mutex<_>>`]. Equality and hashing keep Java's semantics — the
/// iterator is compared by identity, which is what `Objects.equals` on a
/// `DocIdSetIterator` does.
#[derive(Clone)]
pub struct Seeded {
    entry_points: Arc<Mutex<Box<dyn DocIdSetIterator + Send>>>,
    number_of_entry_points: i32,
    original_strategy: Arc<KnnSearchStrategy>,
}

impl fmt::Debug for Seeded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Seeded")
            .field("numberOfEntryPoints", &self.number_of_entry_points)
            .field("originalStrategy", &self.original_strategy)
            .finish()
    }
}

impl Seeded {
    /// Creates a seeded strategy.
    ///
    /// Equivalent to
    /// `new KnnSearchStrategy.Seeded(DocIdSetIterator, int, KnnSearchStrategy)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`](crate::error::LuceneError::IllegalArgument)
    /// — with Java's messages — when `number_of_entry_points` is negative, or
    /// when it is positive and no entry points are supplied.
    pub fn new(
        entry_points: Option<Box<dyn DocIdSetIterator + Send>>,
        number_of_entry_points: i32,
        original_strategy: KnnSearchStrategy,
    ) -> crate::error::Result<Self> {
        if number_of_entry_points < 0 {
            return Err(crate::error::LuceneError::IllegalArgument(
                "numberOfEntryPoints must be >= 0".to_string(),
            ));
        }
        if number_of_entry_points > 0 && entry_points.is_none() {
            return Err(crate::error::LuceneError::IllegalArgument(
                "entryPoints must not be null".to_string(),
            ));
        }
        let entry_points: Box<dyn DocIdSetIterator + Send> = match entry_points {
            Some(iterator) => iterator,
            None => Box::new(empty()),
        };
        Ok(Self {
            entry_points: Arc::new(Mutex::new(entry_points)),
            number_of_entry_points,
            original_strategy: Arc::new(original_strategy),
        })
    }

    /// Locks and returns the iterator of valid entry points for the kNN search.
    ///
    /// Equivalent to `KnnSearchStrategy.Seeded.entryPoints()`.
    pub fn entry_points(&self) -> MutexGuard<'_, Box<dyn DocIdSetIterator + Send>> {
        self.entry_points
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Returns the number of valid entry points for the kNN search.
    ///
    /// Equivalent to `KnnSearchStrategy.Seeded.numberOfEntryPoints()`.
    pub fn number_of_entry_points(&self) -> i32 {
        self.number_of_entry_points
    }

    /// Returns the strategy to use after seeding.
    ///
    /// Equivalent to `KnnSearchStrategy.Seeded.originalStrategy()`.
    pub fn original_strategy(&self) -> &KnnSearchStrategy {
        &self.original_strategy
    }

    /// Returns a copy of this strategy over the same entry points but with a
    /// different strategy to use after seeding.
    ///
    /// Equivalent to Java's
    /// `new KnnSearchStrategy.Seeded(seeded.entryPoints(), seeded.numberOfEntryPoints(), other)`,
    /// which reuses the very same `DocIdSetIterator` instance; this port shares
    /// the same [`Arc`] so that the entry points stay one object, as they are
    /// in Java.
    pub fn with_original_strategy(&self, original_strategy: KnnSearchStrategy) -> Self {
        Self {
            entry_points: Arc::clone(&self.entry_points),
            number_of_entry_points: self.number_of_entry_points,
            original_strategy: Arc::new(original_strategy),
        }
    }
}

impl PartialEq for Seeded {
    fn eq(&self, other: &Self) -> bool {
        self.number_of_entry_points == other.number_of_entry_points
            && Arc::ptr_eq(&self.entry_points, &other.entry_points)
            && self.original_strategy == other.original_strategy
    }
}

impl Eq for Seeded {}

impl Hash for Seeded {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_usize(Arc::as_ptr(&self.entry_points) as *const () as usize);
        state.write_i32(self.number_of_entry_points);
        self.original_strategy.hash(state);
    }
}

/// A strategy for kNN search on HNSW that early-exits when the nearest
/// neighbour collection rate saturates.
///
/// Equivalent to `org.apache.lucene.search.knn.KnnSearchStrategy.Patience`,
/// which extends `Hnsw`.
///
/// **Divergence from Lucene 10.5.0.** Java's `Patience` holds the whole
/// [`HnswQueueSaturationCollector`](crate::search::HnswQueueSaturationCollector)
/// and calls `nextCandidate()` on it, which mutates the collector while the
/// search also drives it. Rust forbids that alias, so the strategy and the
/// collector share the saturation counters through an [`Arc<Mutex<_>>`]; the
/// counters, their updates and the resulting early exit are unchanged. Java's
/// identity comparison of the collector becomes pointer equality of the shared
/// state, which discriminates exactly the same instances.
#[derive(Debug, Clone)]
pub struct Patience {
    hnsw: Hnsw,
    state: Arc<Mutex<HnswQueueSaturationState>>,
}

impl Patience {
    /// Creates a patience strategy over the saturation state of a collector.
    ///
    /// Equivalent to
    /// `new KnnSearchStrategy.Patience(HnswQueueSaturationCollector, int)`.
    pub fn new(
        state: Arc<Mutex<HnswQueueSaturationState>>,
        filtered_search_threshold: i32,
    ) -> Self {
        Self {
            hnsw: Hnsw {
                filtered_search_threshold,
            },
            state,
        }
    }

    /// Returns the filtered-search threshold inherited from `Hnsw`.
    ///
    /// Equivalent to `KnnSearchStrategy.Hnsw.filteredSearchThreshold()`.
    pub fn filtered_search_threshold(&self) -> i32 {
        self.hnsw.filtered_search_threshold()
    }

    /// Whether to use filtered search, given the ratio of vectors that pass the
    /// filter.
    ///
    /// Equivalent to the inherited
    /// `final KnnSearchStrategy.Hnsw.useFilteredSearch(float)`.
    pub fn use_filtered_search(&self, ratio_passing_filter: f32) -> bool {
        self.hnsw.use_filtered_search(ratio_passing_filter)
    }

    /// Records the visit of one more HNSW node candidate.
    ///
    /// Equivalent to `KnnSearchStrategy.Patience.nextVectorsBlock()`, which is
    /// `collector.nextCandidate()`.
    pub fn next_candidate(&self) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .next_candidate();
    }

    /// Returns the shared saturation state.
    pub fn state(&self) -> &Arc<Mutex<HnswQueueSaturationState>> {
        &self.state
    }
}

impl PartialEq for Patience {
    fn eq(&self, other: &Self) -> bool {
        self.hnsw == other.hnsw && Arc::ptr_eq(&self.state, &other.state)
    }
}

impl Eq for Patience {}

impl Hash for Patience {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hnsw.hash(state);
        state.write_usize(Arc::as_ptr(&self.state) as *const () as usize);
    }
}

/// Collects the results of a kNN search and provides the top documents from
/// the gathered neighbours.
///
/// Equivalent to `org.apache.lucene.search.KnnCollector`.
pub trait KnnCollector: Send {
    /// Returns whether the current result set is marked incomplete because the
    /// search visited too many documents.
    ///
    /// Equivalent to `KnnCollector.earlyTerminated()`. When collection was
    /// terminated early the results are not a correct representation of the `k`
    /// nearest neighbours.
    fn early_terminated(&self) -> bool;

    /// Increments the visited-vector count by `count`, which must be positive.
    ///
    /// Equivalent to `KnnCollector.incVisitedCount(int)`.
    fn inc_visited_count(&mut self, count: i32);

    /// Returns the current visited-vector count.
    ///
    /// Equivalent to `KnnCollector.visitedCount()`.
    fn visited_count(&self) -> i64;

    /// Returns the visited-vector limit.
    ///
    /// Equivalent to `KnnCollector.visitLimit()`.
    fn visit_limit(&self) -> i64;

    /// Returns the expected number of collected results.
    ///
    /// Equivalent to `KnnCollector.k()`.
    fn k(&self) -> i32;

    /// Collects the given doc ID with its similarity, returning `true` if the
    /// vector was collected.
    ///
    /// Equivalent to `KnnCollector.collect(int, float)`.
    fn collect(&mut self, doc_id: i32, similarity: f32) -> bool;

    /// Returns the current minimum competitive similarity, so that only
    /// competitive results are explored.
    ///
    /// Equivalent to `KnnCollector.minCompetitiveSimilarity()`. A collector
    /// that wants to collect `k` results returns [`f32::NEG_INFINITY`] until it
    /// is full, and the minimum collected score afterwards.
    fn min_competitive_similarity(&self) -> f32;

    /// Drains the collected nearest neighbours into a [`TopDocs`], ordered by
    /// score descending.
    ///
    /// Equivalent to `KnnCollector.topDocs()`. This is generally a destructive
    /// action and the collector should not be used afterwards, which is why —
    /// unlike Java, whose `topDocs()` merely looks `const` — the receiver is
    /// `&mut self`.
    fn top_docs(&mut self) -> TopDocs;

    /// Returns the search strategy used by this collector, if any.
    ///
    /// Equivalent to `KnnCollector.getSearchStrategy()`, which may return
    /// `null`.
    fn get_search_strategy(&self) -> Option<&KnnSearchStrategy>;
}

/// A kNN collector that keeps the top `k` results by similarity in a min-heap.
///
/// Equivalent to `org.apache.lucene.search.TopKnnCollector`.
#[derive(Debug, Clone)]
pub struct TopKnnCollector {
    base: AbstractKnnCollectorState,
    /// The min-heap of the currently collected vectors.
    ///
    /// Equivalent to the `protected final NeighborQueue queue` field.
    pub queue: NeighborQueue,
}

impl TopKnnCollector {
    /// Creates a collector with no search strategy.
    ///
    /// Equivalent to `new TopKnnCollector(int, int)`.
    pub fn new(k: i32, visit_limit: i64) -> Self {
        Self::with_strategy(k, visit_limit, None)
    }

    /// Creates a collector with an explicit search strategy.
    ///
    /// Equivalent to
    /// `new TopKnnCollector(int, int, KnnSearchStrategy)`.
    pub fn with_strategy(
        k: i32,
        visit_limit: i64,
        search_strategy: Option<KnnSearchStrategy>,
    ) -> Self {
        Self {
            base: AbstractKnnCollectorState::new(k, visit_limit, search_strategy),
            queue: NeighborQueue::new(k, false),
        }
    }

    /// Returns the collected document ids and scores, best first, without
    /// draining the queue.
    ///
    /// **Divergence from Lucene 10.5.0.** Java has no such method; it exists so
    /// that HNSW callers that only need the raw pairs do not have to build a
    /// [`TopDocs`].
    pub fn top_docs_with_scores(&self) -> Vec<(i32, f32)> {
        let mut results = Vec::with_capacity(self.queue.size() as usize);
        let mut clone = self.queue.clone();
        while clone.size() > 0 {
            results.push((clone.top_node(), clone.top_score()));
            clone.pop();
        }
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        results
    }

    /// Returns the collected node ids in arbitrary order.
    #[cfg(test)]
    pub fn nodes(&self) -> Vec<i32> {
        self.queue.nodes()
    }
}

impl fmt::Display for TopKnnCollector {
    /// Renders the collector exactly as `TopKnnCollector.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "TopKnnCollector[k={}, size={}]",
            self.k(),
            self.queue.size()
        )
    }
}

impl KnnCollector for TopKnnCollector {
    fn early_terminated(&self) -> bool {
        self.base.early_terminated()
    }

    fn inc_visited_count(&mut self, count: i32) {
        self.base.inc_visited_count(count);
    }

    fn visited_count(&self) -> i64 {
        self.base.visited_count()
    }

    fn visit_limit(&self) -> i64 {
        self.base.visit_limit()
    }

    fn k(&self) -> i32 {
        self.base.k()
    }

    fn collect(&mut self, doc_id: i32, similarity: f32) -> bool {
        self.queue.insert_with_overflow(doc_id, similarity)
    }

    fn min_competitive_similarity(&self) -> f32 {
        if self.queue.size() >= self.base.k() {
            self.queue.top_score()
        } else {
            f32::NEG_INFINITY
        }
    }

    fn top_docs(&mut self) -> TopDocs {
        debug_assert!(
            self.queue.size() <= self.base.k(),
            "Tried to collect more results than the maximum number allowed"
        );
        let len = self.queue.size() as usize;
        let mut score_docs = vec![ScoreDoc::new(0, 0.0); len];
        for i in 1..=len {
            score_docs[len - i] = ScoreDoc::new(self.queue.top_node(), self.queue.top_score());
            self.queue.pop();
        }
        let relation = if self.early_terminated() {
            TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO
        } else {
            TotalHitsRelation::EQUAL_TO
        };
        TopDocs {
            total_hits: TotalHits::new(self.visited_count(), relation)
                .expect("INVARIANT: a visited count is never negative"),
            score_docs,
        }
    }

    fn get_search_strategy(&self) -> Option<&KnnSearchStrategy> {
        self.base.get_search_strategy()
    }
}

impl crate::search::abstract_knn_collector::AbstractKnnCollector for TopKnnCollector {
    fn num_collected(&self) -> i32 {
        self.queue.size()
    }
}
