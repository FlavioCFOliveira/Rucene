//! Patience-based early exit, ported from
//! `org.apache.lucene.search.HnswQueueSaturationCollector`.

#![deny(unsafe_code)]

use std::sync::{Arc, Mutex, PoisonError};

use crate::search::knn::{KnnCollector, KnnSearchStrategy, Patience, Seeded, TopDocs};
use crate::search::total_hits::{TotalHits, TotalHitsRelation};

/// The saturation counters of a
/// [`HnswQueueSaturationCollector`], shared with the
/// [`Patience`] strategy that steps them.
///
/// **Divergence from Lucene 10.5.0.** Java keeps these as private fields of the
/// collector, and `KnnSearchStrategy.Patience` holds a reference to the
/// collector so that `nextVectorsBlock()` can mutate them while the search also
/// drives the collector. Rust forbids that alias, so the counters are extracted
/// into this struct and shared through an [`Arc<Mutex<_>>`]. The counters, the
/// saturation rule and the resulting early exit are byte-for-byte Java's.
#[derive(Debug)]
pub struct HnswQueueSaturationState {
    saturation_threshold: f64,
    patience: i32,
    patience_finished: bool,
    count_saturated: i32,
    previous_queue_size: i32,
    current_queue_size: i32,
}

impl HnswQueueSaturationState {
    /// Creates the counters, as the collector's constructor does.
    fn new(saturation_threshold: f64, patience: i32) -> Self {
        Self {
            saturation_threshold,
            patience,
            patience_finished: false,
            count_saturated: 0,
            previous_queue_size: 0,
            current_queue_size: 0,
        }
    }

    /// Returns whether patience has run out.
    ///
    /// Equivalent to reading the `patienceFinished` field.
    pub fn patience_finished(&self) -> bool {
        self.patience_finished
    }

    /// Records one more collected result.
    ///
    /// Equivalent to the `currentQueueSize++` of
    /// `HnswQueueSaturationCollector.collect(int, float)`.
    fn record_collected(&mut self) {
        self.current_queue_size += 1;
    }

    /// Records the visit of the next HNSW node candidate.
    ///
    /// Equivalent to `HnswQueueSaturationCollector.nextCandidate()`. Note that
    /// the queue saturation is `NaN` on the very first candidate, when
    /// `currentQueueSize` is still zero; `NaN >= threshold` is false in Java and
    /// in Rust alike, so the counter resets, exactly as Java does.
    pub fn next_candidate(&mut self) {
        let queue_saturation = self.current_queue_size.min(self.previous_queue_size) as f64
            / self.current_queue_size as f64;
        self.previous_queue_size = self.current_queue_size;
        if queue_saturation >= self.saturation_threshold {
            self.count_saturated += 1;
        } else {
            self.count_saturated = 0;
        }
        if self.count_saturated > self.patience {
            self.patience_finished = true;
        }
    }
}

/// A [`KnnCollector`] decorator that early-exits when the nearest neighbour
/// queue keeps saturating beyond a "patience" parameter.
///
/// Equivalent to `org.apache.lucene.search.HnswQueueSaturationCollector`, which
/// extends `KnnCollector.Decorator`; Rust has no implementation inheritance, so
/// the decorator's delegation is written out.
///
/// It records the rate of collection of new nearest neighbours in the delegate
/// collector's queue at each HNSW node candidate visit. Once it saturates for
/// `patience` consecutive node visits, the search terminates early.
///
/// See "Patience in Proximity: A Simple Early Termination Strategy for HNSW
/// Graph Traversal in Approximate k-Nearest Neighbor Search" (Teofili and Lin),
/// ECIR '25.
pub struct HnswQueueSaturationCollector {
    delegate: Box<dyn KnnCollector>,
    state: Arc<Mutex<HnswQueueSaturationState>>,
    search_strategy: Option<KnnSearchStrategy>,
}

impl std::fmt::Debug for HnswQueueSaturationCollector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HnswQueueSaturationCollector")
            .field("state", &self.state)
            .finish()
    }
}

impl HnswQueueSaturationCollector {
    /// Wraps `delegate` with a patience-based early exit.
    ///
    /// Equivalent to
    /// `new HnswQueueSaturationCollector(KnnCollector, double, int)`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java's `getSearchStrategy()` builds a
    /// fresh `KnnSearchStrategy.Patience` on every call; this port builds it
    /// once, here, because the trait hands out a borrow rather than a value.
    /// The delegate's own strategy never changes, so every call would have
    /// produced an equal object anyway.
    pub fn new(delegate: Box<dyn KnnCollector>, saturation_threshold: f64, patience: i32) -> Self {
        let state = Arc::new(Mutex::new(HnswQueueSaturationState::new(
            saturation_threshold,
            patience,
        )));
        let search_strategy = Self::patience_strategy(delegate.get_search_strategy(), &state);
        Self {
            delegate,
            state,
            search_strategy,
        }
    }

    /// Rewraps the delegate's strategy with patience.
    ///
    /// Equivalent to the body of
    /// `HnswQueueSaturationCollector.getSearchStrategy()`.
    fn patience_strategy(
        delegate_strategy: Option<&KnnSearchStrategy>,
        state: &Arc<Mutex<HnswQueueSaturationState>>,
    ) -> Option<KnnSearchStrategy> {
        match delegate_strategy {
            Some(KnnSearchStrategy::Hnsw(hnsw)) => Some(KnnSearchStrategy::Patience(
                Patience::new(Arc::clone(state), hnsw.filtered_search_threshold()),
            )),
            Some(KnnSearchStrategy::Seeded(seeded)) => {
                if let KnnSearchStrategy::Hnsw(hnsw) = seeded.original_strategy() {
                    // Rewrap the underlying HNSW strategy with patience: this
                    // way we still use the seeded entry points and the filter
                    // threshold, and can utilise patience thresholds.
                    let patience = KnnSearchStrategy::Patience(Patience::new(
                        Arc::clone(state),
                        hnsw.filtered_search_threshold(),
                    ));
                    Some(KnnSearchStrategy::Seeded(Seeded::with_original_strategy(
                        seeded, patience,
                    )))
                } else {
                    Some(KnnSearchStrategy::Seeded(seeded.clone()))
                }
            }
            other => other.cloned(),
        }
    }

    /// Returns the shared saturation counters.
    pub fn state(&self) -> &Arc<Mutex<HnswQueueSaturationState>> {
        &self.state
    }

    /// Records the visit of the next HNSW node candidate.
    ///
    /// Equivalent to `HnswQueueSaturationCollector.nextCandidate()`.
    pub fn next_candidate(&self) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .next_candidate();
    }

    fn patience_finished(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .patience_finished()
    }
}

impl KnnCollector for HnswQueueSaturationCollector {
    fn early_terminated(&self) -> bool {
        self.delegate.early_terminated() || self.patience_finished()
    }

    fn inc_visited_count(&mut self, count: i32) {
        self.delegate.inc_visited_count(count);
    }

    fn visited_count(&self) -> i64 {
        self.delegate.visited_count()
    }

    fn visit_limit(&self) -> i64 {
        self.delegate.visit_limit()
    }

    fn k(&self) -> i32 {
        self.delegate.k()
    }

    fn collect(&mut self, doc_id: i32, similarity: f32) -> bool {
        let collect = self.delegate.collect(doc_id, similarity);
        if collect {
            self.state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .record_collected();
        }
        collect
    }

    fn min_competitive_similarity(&self) -> f32 {
        self.delegate.min_competitive_similarity()
    }

    fn top_docs(&mut self) -> TopDocs {
        if self.patience_finished() && !self.delegate.early_terminated() {
            // This avoids re-running exact search in the filtered scenario when
            // patience is exhausted.
            let delegate_docs = self.delegate.top_docs();
            let total_hits = TotalHits::new(
                delegate_docs.total_hits.value(),
                TotalHitsRelation::EQUAL_TO,
            )
            .expect("INVARIANT: a hit count read back from a TopDocs is never negative");
            TopDocs {
                total_hits,
                score_docs: delegate_docs.score_docs,
            }
        } else {
            self.delegate.top_docs()
        }
    }

    fn get_search_strategy(&self) -> Option<&KnnSearchStrategy> {
        self.search_strategy.as_ref()
    }
}
