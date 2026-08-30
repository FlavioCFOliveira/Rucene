//! Hit counting, ported from `org.apache.lucene.search.TotalHitCountCollector`
//! and `org.apache.lucene.search.TotalHitCountCollectorManager`.
//!
//! These two are not part of the 40-type execution spine by themselves, but
//! [`IndexSearcher::count`](crate::search::IndexSearcher::count) is defined in
//! terms of them, so they are ported here to keep that method complete.

#![deny(unsafe_code)]

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};

use crate::error::{LuceneError, Result};
use crate::index::{IndexReaderContext, LeafReaderContext};
use crate::search::collection_terminated_exception::{CollectionError, CollectionResult};
use crate::search::collector::{Collector, CollectorManager, LeafCollector};
use crate::search::doc_id_stream::DocIdStream;
use crate::search::index_searcher::LeafSlice;
use crate::search::scorable::Scorable;
use crate::search::score_mode::ScoreMode;
use crate::search::weight::Weight;

/// Counts the total number of hits.
///
/// Equivalent to `org.apache.lucene.search.TotalHitCountCollector`, the
/// collector behind [`IndexSearcher::count`](crate::search::IndexSearcher::count).
/// When the [`Weight`] implements [`Weight::count`], this collector skips
/// collecting whole segments.
///
/// **Divergence from Lucene 10.5.0.** Java expresses the intra-segment-partition
/// variant as a private subclass,
/// `TotalHitCountCollectorManager.LeafPartitionAwareTotalHitCountCollector`.
/// Rust has no implementation inheritance, so the shared coordination map is an
/// optional field here and the subclass's `getLeafCollector` override is the
/// branch it enables.
#[derive(Debug)]
pub struct TotalHitCountCollector {
    weight: Option<Arc<dyn Weight>>,
    total_hits: i32,
    early_terminated_map: Option<Arc<EarlyTerminatedMap>>,
}

impl Default for TotalHitCountCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl TotalHitCountCollector {
    /// Creates a collector that counts every leaf it is given.
    ///
    /// Equivalent to `new TotalHitCountCollector()`.
    pub fn new() -> Self {
        Self {
            weight: None,
            total_hits: 0,
            early_terminated_map: None,
        }
    }

    /// Creates a collector that coordinates with the sibling collectors
    /// targeting the same leaf, for searches whose slices hold partitions of a
    /// segment rather than whole segments.
    ///
    /// Equivalent to
    /// `new TotalHitCountCollectorManager.LeafPartitionAwareTotalHitCountCollector(Map)`.
    pub fn leaf_partition_aware(early_terminated_map: Arc<EarlyTerminatedMap>) -> Self {
        Self {
            weight: None,
            total_hits: 0,
            early_terminated_map: Some(early_terminated_map),
        }
    }

    /// Returns how many hits matched the search.
    ///
    /// Equivalent to `TotalHitCountCollector.getTotalHits()`.
    pub fn get_total_hits(&self) -> i32 {
        self.total_hits
    }

    /// Builds the counting leaf collector.
    ///
    /// Equivalent to the `protected final
    /// TotalHitCountCollector.createLeafCollector()`.
    pub fn create_leaf_collector(&mut self) -> Box<dyn LeafCollector + '_> {
        Box::new(TotalHitCountLeafCollector {
            total_hits: &mut self.total_hits,
        })
    }

    /// The body of the base class's `getLeafCollector`: ask the weight for a
    /// sub-linear count and terminate the leaf when it answers.
    fn count_or_collect(
        &mut self,
        context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + '_>> {
        let leaf_count = match self.weight.as_ref() {
            None => -1,
            Some(weight) => weight.count(context)?,
        };
        if leaf_count != -1 {
            self.total_hits += leaf_count;
            return Err(CollectionError::CollectionTerminated);
        }
        Ok(self.create_leaf_collector())
    }
}

impl Collector for TotalHitCountCollector {
    fn score_mode(&self) -> ScoreMode {
        ScoreMode::COMPLETE_NO_SCORES
    }

    fn set_weight(&mut self, weight: Arc<dyn Weight>) {
        self.weight = Some(weight);
    }

    fn get_leaf_collector<'a>(
        &'a mut self,
        context: &LeafReaderContext,
    ) -> CollectionResult<Box<dyn LeafCollector + 'a>> {
        let Some(map) = self.early_terminated_map.clone() else {
            return self.count_or_collect(context);
        };

        match map.claim(context.id()) {
            Claim::First(cell) => match self.count_or_collect(context) {
                Ok(leaf_collector) => {
                    cell.complete(false);
                    Ok(leaf_collector)
                }
                Err(CollectionError::CollectionTerminated) => {
                    cell.complete(true);
                    Err(CollectionError::CollectionTerminated)
                }
                Err(err) => {
                    // Java leaves the future incomplete on a non-terminating
                    // failure, which would block every other partition of the
                    // same leaf for ever. Completing it as "did not terminate"
                    // keeps the siblings live; the failure still propagates and
                    // aborts the search.
                    cell.complete(false);
                    Err(err)
                }
            },
            Claim::Existing(cell) => {
                if cell.wait()? {
                    // The first partition of the same leaf terminated early; do
                    // the same for the subsequent ones.
                    Err(CollectionError::CollectionTerminated)
                } else {
                    // The first partition of the same leaf computed hit counts;
                    // do the same for the subsequent ones.
                    Ok(self.create_leaf_collector())
                }
            }
        }
    }
}

/// The per-leaf half of [`TotalHitCountCollector`].
///
/// Equivalent to the anonymous `LeafCollector` that
/// `TotalHitCountCollector.createLeafCollector()` returns.
#[derive(Debug)]
struct TotalHitCountLeafCollector<'a> {
    total_hits: &'a mut i32,
}

impl LeafCollector for TotalHitCountLeafCollector<'_> {
    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }

    fn collect(&mut self, _doc: i32, _scorer: &mut dyn Scorable) -> CollectionResult<()> {
        *self.total_hits += 1;
        Ok(())
    }

    fn collect_stream(
        &mut self,
        stream: &mut dyn DocIdStream,
        _scorer: &mut dyn Scorable,
    ) -> CollectionResult<()> {
        *self.total_hits += stream.count()?;
        Ok(())
    }
}

/// Whether the caller is the first collector to reach a given leaf.
enum Claim {
    /// The caller must decide, and then publish its decision.
    First(Arc<EarlyTerminatedCell>),
    /// Another collector is deciding, or has decided.
    Existing(Arc<EarlyTerminatedCell>),
}

/// One leaf's decision: whether the first partition to reach it terminated
/// early.
///
/// Equivalent to the `CompletableFuture<Boolean>` Java stores per leaf.
#[derive(Debug, Default)]
struct EarlyTerminatedCell {
    state: Mutex<Option<bool>>,
    published: Condvar,
}

impl EarlyTerminatedCell {
    fn complete(&self, early_terminated: bool) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.is_none() {
            *state = Some(early_terminated);
        }
        self.published.notify_all();
    }

    fn wait(&self) -> Result<bool> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while state.is_none() {
            state = self
                .published
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        state.ok_or_else(|| {
            LuceneError::IllegalState("early-termination decision is missing".to_string())
        })
    }
}

/// The state shared across the collectors a
/// [`TotalHitCountCollectorManager`] creates when the searcher's slices hold
/// partitions of a segment.
///
/// Equivalent to the `Map<Object, Future<Boolean>> earlyTerminatedMap` field of
/// `TotalHitCountCollectorManager`.
///
/// It is necessary for correctness: if the first partition of a segment
/// terminates early, the count has already been retrieved for the entire
/// segment, so subsequent partitions of the same segment must also terminate
/// early without incrementing the hit count. Conversely, if the first partition
/// computes hit counts, subsequent partitions must do the same, to prevent
/// their counts from being retrieved from a query cache that would return the
/// count for the entire segment.
#[derive(Debug, Default)]
pub struct EarlyTerminatedMap {
    cells: Mutex<HashMap<usize, Arc<EarlyTerminatedCell>>>,
}

impl EarlyTerminatedMap {
    /// Creates an empty map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Removes every recorded decision, so that the owning manager can be
    /// reused across searches.
    pub fn clear(&self) {
        self.cells
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }

    fn is_empty(&self) -> bool {
        self.cells
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
    }

    fn claim(&self, leaf_id: usize) -> Claim {
        let mut cells = self
            .cells
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match cells.get(&leaf_id) {
            Some(cell) => Claim::Existing(Arc::clone(cell)),
            None => {
                let cell = Arc::new(EarlyTerminatedCell::default());
                cells.insert(leaf_id, Arc::clone(&cell));
                Claim::First(cell)
            }
        }
    }
}

/// Collector manager that parallelises counting the number of hits.
///
/// Equivalent to `org.apache.lucene.search.TotalHitCountCollectorManager`. When
/// this is the only manager used,
/// [`IndexSearcher::count`](crate::search::IndexSearcher::count) should be
/// called rather than
/// [`IndexSearcher::search`](crate::search::IndexSearcher::search), because the
/// former is faster whenever the count can be returned directly from the index
/// statistics.
#[derive(Debug)]
pub struct TotalHitCountCollectorManager {
    has_segment_partitions: bool,
    early_terminated_map: Arc<EarlyTerminatedMap>,
}

impl TotalHitCountCollectorManager {
    /// Creates a manager for the given leaf slices, which are used to decide
    /// whether the collectors have to coordinate across partitions of the same
    /// segment.
    ///
    /// Equivalent to
    /// `new TotalHitCountCollectorManager(IndexSearcher.LeafSlice[])`; obtain
    /// the slices from
    /// [`IndexSearcher::get_slices`](crate::search::IndexSearcher::get_slices).
    pub fn new(leaf_slices: &[LeafSlice]) -> Self {
        Self {
            has_segment_partitions: Self::has_segment_partitions(leaf_slices),
            early_terminated_map: Arc::new(EarlyTerminatedMap::new()),
        }
    }

    /// Equivalent to the private
    /// `TotalHitCountCollectorManager.hasSegmentPartitions`.
    fn has_segment_partitions(leaf_slices: &[LeafSlice]) -> bool {
        for leaf_slice in leaf_slices {
            for leaf_partition in leaf_slice.partitions() {
                if leaf_partition.min_doc_id > 0
                    || leaf_partition.max_doc_id < leaf_partition.ctx.leaf_reader().max_doc()
                {
                    return true;
                }
            }
        }
        false
    }
}

impl CollectorManager for TotalHitCountCollectorManager {
    type Collector = TotalHitCountCollector;
    type Output = i32;

    fn new_collector(&self) -> Result<TotalHitCountCollector> {
        if self.has_segment_partitions {
            Ok(TotalHitCountCollector::leaf_partition_aware(Arc::clone(
                &self.early_terminated_map,
            )))
        } else {
            Ok(TotalHitCountCollector::new())
        }
    }

    fn reduce(&self, collectors: Vec<TotalHitCountCollector>) -> Result<i32> {
        // Make the same collector manager instance reusable across multiple
        // searches. It is not a strict requirement, but it is generally
        // supported, as collector managers normally hold no state.
        debug_assert!(self.has_segment_partitions || self.early_terminated_map.is_empty());
        if self.has_segment_partitions {
            self.early_terminated_map.clear();
        }
        let mut total_hits = 0i32;
        for collector in &collectors {
            total_hits = total_hits.wrapping_add(collector.get_total_hits());
        }
        Ok(total_hits)
    }
}
