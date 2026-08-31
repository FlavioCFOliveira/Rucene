//! Port of `org.apache.lucene.util.hnsw.FilteredHnswGraphSearcher`.

use crate::error::{LuceneError, Result};
use crate::search::knn::KnnCollector;
use crate::search::NO_MORE_DOCS;
use crate::util::bit_set::BitSet;
use crate::util::{Bits, FixedBitSet, SparseFixedBitSet};

use super::abstract_searcher::AbstractHnswGraphSearcher;
use super::neighbor::NeighborQueue;
use super::scorer::RandomVectorScorer;
use super::searcher::{next_up_f32, score_entry_points, HnswGraphSearcher};
use super::HnswGraph;

/// How many filtered candidates must be found to consider N-hop neighbours.
///
/// Equivalent to `FilteredHnswGraphSearcher.EXPANDED_EXPLORATION_LAMBDA`.
const EXPANDED_EXPLORATION_LAMBDA: f32 = 0.10;

/// Searches an HNSW graph to find nearest neighbours to a query vector, optimised
/// for a filtered search.
///
/// Equivalent to `org.apache.lucene.util.hnsw.FilteredHnswGraphSearcher`, inspired
/// by the [ACORN-1 algorithm](https://arxiv.org/abs/2403.04871) and augmented in two
/// ways: the optimised filter step is triggered dynamically per small world based on
/// a filtered lambda, and the number of additional candidates explored is predicated
/// on the original candidate's filtered percentage.
///
/// # Divergences from Lucene 10.5.0
///
/// * Java derives this class from `HnswGraphSearcher` and reuses the inherited
///   `candidates`, `visited` and `bulkScores` fields for both `findBestEntryPoint`
///   and the overridden `searchLevel`. Rust has no implementation inheritance, so
///   this port holds a [`HnswGraphSearcher`] to supply `find_best_entry_point` and
///   keeps its own filtered-search state. Both methods reset that state before use,
///   so the results are the same.
/// * Java calls `results.getSearchStrategy().nextVectorsBlock()` at the end of every
///   candidate expansion. The crate's `KnnSearchStrategy` is still a placeholder with
///   no such method, so that call has no counterpart yet.
pub struct FilteredHnswGraphSearcher {
    entry_point_searcher: HnswGraphSearcher,
    candidates: NeighborQueue,
    visited: Box<dyn BitSet>,
    bulk_scores: Vec<f32>,
    /// How many extra neighbours to explore, as a multiple of the candidate's
    /// neighbour count.
    max_exploration_multiplier: i32,
    min_to_score: i32,
}

impl FilteredHnswGraphSearcher {
    /// Creates a new filtered graph searcher.
    ///
    /// `filter_size` is the number of vectors that pass the accepted-ords filter.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if `filter_size` is not in
    /// `(0, graph size)`.
    pub fn create(k: i32, graph: &dyn HnswGraph, filter_size: i32) -> Result<Self> {
        let graph_size = graph.max_node_id() + 1;
        if filter_size <= 0 || filter_size >= graph_size {
            return Err(LuceneError::IllegalArgument(
                "filterSize must be > 0 and < graph size".to_string(),
            ));
        }
        let visited = Self::bit_set_for(i64::from(filter_size), graph_size, k);
        Ok(Self::new(
            k,
            NeighborQueue::new(k, true),
            visited,
            filter_size,
            graph,
        ))
    }

    fn new(
        k: i32,
        candidates: NeighborQueue,
        visited: Box<dyn BitSet>,
        filter_size: i32,
        graph: &dyn HnswGraph,
    ) -> Self {
        debug_assert!(
            graph.max_conn() > 0,
            "graph must have known max connections"
        );
        let filter_ratio = filter_size as f32 / graph.size() as f32;
        let max_exploration_multiplier = java_round_double(f64::min(
            1.0 / f64::from(filter_ratio),
            f64::from(graph.max_conn()) / 2.0,
        )) as i32;
        // As the filter gets exceptionally restrictive, we must spread out the
        // exploration.
        let min_to_score = java_round_double(f64::min(
            f64::max(
                0.0,
                1.0 / f64::from(filter_ratio) - (2.0 * f64::from(graph.max_conn())),
            ),
            f64::from(graph.max_conn()),
        )) as i32;
        let entry_point_searcher = HnswGraphSearcher::new(
            NeighborQueue::new(k, true),
            FixedBitSet::new(graph.max_node_id().max(0) as usize + 1),
        );
        Self {
            entry_point_searcher,
            candidates,
            visited,
            bulk_scores: Vec::new(),
            max_exploration_multiplier,
            min_to_score,
        }
    }

    fn bit_set_for(filter_size: i64, graph_size: i32, topk: i32) -> Box<dyn BitSet> {
        let percent_filtered = filter_size as f32 / graph_size as f32;
        debug_assert!(percent_filtered > 0.0 && percent_filtered < 1.0);
        let total_ops = f64::from(graph_size).ln() * f64::from(topk);
        let approximate_visitation = (total_ops / f64::from(percent_filtered)) as i32;
        Self::bit_set(approximate_visitation, graph_size)
    }

    fn bit_set(expected_bits: i32, total_bits: i32) -> Box<dyn BitSet> {
        if expected_bits < (total_bits >> 7) {
            Box::new(SparseFixedBitSet::new(total_bits.max(1) as usize))
        } else {
            Box::new(FixedBitSet::new(total_bits.max(0) as usize))
        }
    }

    fn prepare_scratch_state(&mut self) {
        self.candidates.clear();
        self.visited.clear_all();
    }
}

impl AbstractHnswGraphSearcher for FilteredHnswGraphSearcher {
    fn search_level(
        &mut self,
        results: &mut dyn KnnCollector,
        scorer: &mut dyn RandomVectorScorer,
        level: i32,
        eps: &[i32],
        graph: &mut dyn HnswGraph,
        accept_ords: Option<&dyn Bits>,
    ) -> Result<()> {
        debug_assert!(level == 0, "Filtered search only works on the base level");
        let accept_ords = accept_ords.ok_or_else(|| {
            LuceneError::IllegalArgument(
                "acceptOrds must not be null to used filtered search".to_string(),
            )
        })?;

        let size = graph.max_node_id() + 1;

        self.prepare_scratch_state();

        if self.bulk_scores.len() < eps.len() {
            self.bulk_scores.resize(eps.len(), 0.0);
        }
        if results.early_terminated() {
            return Ok(());
        }
        score_entry_points(
            results,
            scorer,
            self.visited.as_mut(),
            eps,
            Some(accept_ords),
            &mut self.candidates,
            &mut self.bulk_scores,
        )?;
        if results.early_terminated() {
            return Ok(());
        }
        // Collect the vectors to score and potentially add as candidates.
        let queue_capacity =
            (graph.max_conn() * 2 * self.max_exploration_multiplier).max(0) as usize;
        let mut to_score = IntArrayQueue::new(queue_capacity);
        let mut to_explore = IntArrayQueue::new(queue_capacity);
        // A bound that holds the minimum similarity to the query vector that a
        // candidate vector must have to be considered.
        let mut min_accepted_similarity = next_up_f32(results.min_competitive_similarity());
        while self.candidates.size() > 0 && !results.early_terminated() {
            // Get the best candidate (closest or best scoring).
            let top_candidate_similarity = self.candidates.top_score();
            if min_accepted_similarity > top_candidate_similarity {
                break;
            }
            let top_candidate_node = self.candidates.pop();
            graph.seek(level, top_candidate_node)?;
            let neighbor_count = graph.neighbor_count();
            to_score.clear();
            to_explore.clear();
            loop {
                let friend_ord = graph.next_neighbor()?;
                if friend_ord == NO_MORE_DOCS || to_score.is_full() {
                    break;
                }
                debug_assert!(friend_ord < size);
                if self.visited.get_and_set(friend_ord as usize) {
                    continue;
                }
                if accept_ords.get(friend_ord as usize) {
                    to_score.add(friend_ord);
                } else {
                    to_explore.add(friend_ord);
                }
            }
            // Adjust locally the number of filtered candidates to explore.
            let filtered_amount = to_explore.count() as f32 / neighbor_count as f32;
            let max_to_score_count = (neighbor_count as f32
                * f32::min(
                    self.max_exploration_multiplier as f32,
                    1.0 / (1.0 - filtered_amount),
                )) as i32;
            let max_additional_to_explore_count = to_explore.capacity() as i32 - 1;
            // There is enough filtered, or we don't have enough candidates to score
            // and explore.
            let mut total_explored = to_score.count() + to_explore.count();
            if to_score.count() < max_to_score_count
                && filtered_amount > EXPANDED_EXPLORATION_LAMBDA
            {
                // Now we need to explore the neighbours of the neighbours.
                loop {
                    let explore_friend = to_explore.poll();
                    if explore_friend == NO_MORE_DOCS
                        // only explore the initial additional neighbourhood
                        || total_explored >= max_additional_to_explore_count
                        || to_score.count() >= max_to_score_count
                    {
                        break;
                    }
                    graph.seek(level, explore_friend)?;
                    loop {
                        let friend_of_a_friend_ord = graph.next_neighbor()?;
                        if friend_of_a_friend_ord == NO_MORE_DOCS
                            || to_score.count() >= max_to_score_count
                        {
                            break;
                        }
                        if self.visited.get_and_set(friend_of_a_friend_ord as usize) {
                            continue;
                        }
                        total_explored += 1;
                        if accept_ords.get(friend_of_a_friend_ord as usize) {
                            to_score.add(friend_of_a_friend_ord);
                        // If we have YET to find a minimum number of candidates, we
                        // will continue to explore until our max.
                        } else if total_explored < max_additional_to_explore_count
                            && to_score.count() < self.min_to_score
                        {
                            to_explore.add(friend_of_a_friend_ord);
                        }
                    }
                }
            }
            // Score the vectors and add them to the candidate list.
            if self.bulk_scores.len() < to_score.count() as usize {
                self.bulk_scores.resize(to_score.count() as usize, 0.0);
            }
            debug_assert!(to_score.upto == 0);
            let max_score = if to_score.count() > 0 {
                scorer.bulk_score(&to_score.nodes, &mut self.bulk_scores, to_score.size)?
            } else {
                f32::NEG_INFINITY
            };
            results.inc_visited_count(to_score.count());
            if max_score > min_accepted_similarity {
                for i in 0..to_score.count() {
                    let idx = (i + to_score.upto) as usize;
                    let friend_similarity = self.bulk_scores[idx];
                    if friend_similarity > min_accepted_similarity {
                        let ord = to_score.nodes[idx];
                        self.candidates.add(ord, friend_similarity);
                        if results.collect(ord, friend_similarity) {
                            min_accepted_similarity =
                                next_up_f32(results.min_competitive_similarity());
                        }
                    }
                }
            }
            to_score.upto = to_score.size; // all scored
        }
        Ok(())
    }

    fn find_best_entry_point(
        &mut self,
        scorer: &mut dyn RandomVectorScorer,
        graph: &mut dyn HnswGraph,
        collector: &mut dyn KnnCollector,
    ) -> Result<Vec<i32>> {
        self.entry_point_searcher
            .find_best_entry_point(scorer, graph, collector)
    }
}

/// `java.lang.Math.round(double)`: floor of `x + 0.5`, which differs from Rust's
/// round-half-away-from-zero for negative halves.
fn java_round_double(x: f64) -> i64 {
    if x.is_nan() {
        return 0;
    }
    (x + 0.5).floor() as i64
}

/// A fixed-capacity FIFO of node ordinals.
///
/// Equivalent to the private `FilteredHnswGraphSearcher.IntArrayQueue`.
struct IntArrayQueue {
    nodes: Vec<i32>,
    upto: i32,
    size: i32,
}

impl IntArrayQueue {
    fn new(capacity: usize) -> Self {
        Self {
            nodes: vec![0; capacity],
            upto: 0,
            size: 0,
        }
    }

    fn capacity(&self) -> usize {
        self.nodes.len()
    }

    fn count(&self) -> i32 {
        self.size - self.upto
    }

    /// # Panics
    ///
    /// Panics when the queue is full, matching Java's
    /// `UnsupportedOperationException`.
    fn add(&mut self, node: i32) {
        assert!(!self.is_full(), "Initial capacity should remain unchanged");
        self.nodes[self.size as usize] = node;
        self.size += 1;
    }

    fn is_full(&self) -> bool {
        self.size as usize == self.nodes.len()
    }

    fn poll(&mut self) -> i32 {
        if self.upto == self.size {
            return NO_MORE_DOCS;
        }
        let node = self.nodes[self.upto as usize];
        self.upto += 1;
        node
    }

    fn clear(&mut self) {
        self.upto = 0;
        self.size = 0;
    }
}
