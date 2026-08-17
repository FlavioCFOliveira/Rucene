//! HNSW graph search implementation.
//!
//! Equivalent to `org.apache.lucene.util.hnsw.HnswGraphSearcher` and
//! `org.apache.lucene.util.hnsw.AbstractHnswGraphSearcher`.

#![deny(unsafe_code)]

use std::f32;

use crate::search::knn::{KnnCollector, TopKnnCollector};
use crate::search::NO_MORE_DOCS;
use crate::util::{Bits, FixedBitSet};

use super::neighbor::{NeighborArray, NeighborQueue};
use super::on_heap::OnHeapHnswGraph;
use super::scorer::RandomVectorScorer;
use super::{HnswGraph, Result};

const UNK_EP: i32 = -1;

/// Returns the next representable float after `x`, matching Java's
/// `Math.nextUp(float)` and safe for negative infinity / zero / NaN.
fn next_up_f32(x: f32) -> f32 {
    if x.is_nan() || x == f32::INFINITY {
        return x;
    }
    if x == 0.0 {
        return f32::from_bits(1);
    }
    let bits = x.to_bits();
    if x.is_sign_negative() {
        f32::from_bits(bits - 1)
    } else {
        f32::from_bits(bits + 1)
    }
}

/// Searches an HNSW graph for nearest neighbors.
///
/// Equivalent to `org.apache.lucene.util.hnsw.HnswGraphSearcher`.
#[derive(Debug)]
pub struct HnswGraphSearcher {
    candidates: NeighborQueue,
    visited: FixedBitSet,
    bulk_nodes: Vec<i32>,
    bulk_scores: Vec<f32>,
}

impl HnswGraphSearcher {
    /// Creates a new searcher. `candidates` is a max-heap used to track nodes
    /// to explore; `visited` tracks already-visited nodes.
    pub fn new(candidates: NeighborQueue, visited: FixedBitSet) -> Self {
        Self {
            candidates,
            visited,
            bulk_nodes: Vec::new(),
            bulk_scores: Vec::new(),
        }
    }

    /// Convenience search over an on-heap graph, returning a collector with
    /// the top `top_k` results.
    pub fn search_on_heap(
        scorer: &mut dyn RandomVectorScorer,
        top_k: i32,
        graph: &OnHeapHnswGraph,
        accept_ords: Option<&dyn Bits>,
        visited_limit: i64,
    ) -> Result<TopKnnCollector> {
        let mut collector = TopKnnCollector::new(top_k, visited_limit);
        let mut searcher = OnHeapSearcher::new(
            NeighborQueue::new(top_k, true),
            FixedBitSet::new(graph.max_node_id().max(0) as usize + 1),
        );
        searcher.search(&mut collector, scorer, graph, accept_ords)?;
        Ok(collector)
    }

    /// Searches a generic `HnswGraph`.
    pub fn search_graph(
        &mut self,
        results: &mut dyn KnnCollector,
        scorer: &mut dyn RandomVectorScorer,
        graph: &mut dyn HnswGraph,
        accept_ords: Option<&dyn Bits>,
    ) -> Result<()> {
        let eps = self.find_best_entry_point(scorer, graph, results)?;
        if eps.is_empty() || eps[0] == UNK_EP {
            return Ok(());
        }
        self.search_level(results, scorer, 0, &eps, graph, accept_ords)
    }

    fn find_best_entry_point(
        &mut self,
        scorer: &mut dyn RandomVectorScorer,
        graph: &mut dyn HnswGraph,
        collector: &mut dyn KnnCollector,
    ) -> Result<Vec<i32>> {
        let mut current_ep = graph.entry_node()?;
        if current_ep == -1 || graph.num_levels()? == 1 {
            return Ok(vec![current_ep]);
        }
        let size = graph.max_node_id() + 1;
        self.prepare_scratch_state(size, graph.max_conn() * 2);
        let mut current_score = scorer.score(current_ep)?;
        collector.inc_visited_count(1);
        for level in (1..graph.num_levels()?).rev() {
            let mut found_better = true;
            self.visited.set(current_ep as usize);
            while found_better {
                found_better = false;
                graph.seek(level, current_ep)?;
                let mut friend_ord;
                let mut num_nodes = 0i32;
                while {
                    friend_ord = graph.next_neighbor()?;
                    friend_ord != NO_MORE_DOCS
                } {
                    if self.visited.get_and_set(friend_ord as usize) {
                        continue;
                    }
                    if collector.early_terminated() {
                        return Ok(vec![UNK_EP]);
                    }
                    self.bulk_nodes[num_nodes as usize] = friend_ord;
                    num_nodes += 1;
                }
                let max_score = if num_nodes > 0 {
                    scorer.bulk_score(&self.bulk_nodes, &mut self.bulk_scores, num_nodes)?
                } else {
                    f32::NEG_INFINITY
                };
                collector.inc_visited_count(num_nodes);
                if max_score > current_score {
                    for i in 0..num_nodes as usize {
                        let score = self.bulk_scores[i];
                        if score > current_score {
                            current_score = score;
                            current_ep = self.bulk_nodes[i];
                            found_better = true;
                        }
                    }
                }
            }
        }
        Ok(if collector.early_terminated() {
            vec![UNK_EP]
        } else {
            vec![current_ep]
        })
    }

    /// Searches a single graph level, collecting into `results`.
    pub fn search_level(
        &mut self,
        results: &mut dyn KnnCollector,
        scorer: &mut dyn RandomVectorScorer,
        level: i32,
        eps: &[i32],
        graph: &mut dyn HnswGraph,
        accept_ords: Option<&dyn Bits>,
    ) -> Result<()> {
        let size = graph.max_node_id() + 1;
        self.prepare_scratch_state(size, graph.max_conn() * 2);
        if self.bulk_scores.len() < eps.len() {
            self.bulk_scores.resize(eps.len(), 0.0);
        }
        if results.early_terminated() {
            return Ok(());
        }
        score_entry_points(
            results,
            scorer,
            &mut self.visited,
            eps,
            accept_ords,
            &mut self.candidates,
            &mut self.bulk_scores,
        )?;
        if results.early_terminated() {
            return Ok(());
        }

        let mut min_accepted_similarity = next_up_f32(results.min_competitive_similarity());
        let mut should_explore_min_sim = true;
        while self.candidates.size() > 0 && !results.early_terminated() {
            let top_candidate_similarity = self.candidates.top_score();
            if top_candidate_similarity < min_accepted_similarity {
                if should_explore_min_sim
                    && next_up_f32(top_candidate_similarity) == min_accepted_similarity
                {
                    should_explore_min_sim = false;
                } else {
                    break;
                }
            }

            let top_candidate_node = self.candidates.pop();
            graph.seek(level, top_candidate_node)?;
            let mut friend_ord;
            let mut num_nodes = 0i32;
            while {
                friend_ord = graph.next_neighbor()?;
                friend_ord != NO_MORE_DOCS
            } {
                if self.visited.get_and_set(friend_ord as usize) {
                    continue;
                }
                if results.early_terminated() {
                    break;
                }
                self.bulk_nodes[num_nodes as usize] = friend_ord;
                num_nodes += 1;
            }

            let limit =
                (results.visit_limit() - results.visited_count()).min(num_nodes as i64) as i32;
            results.inc_visited_count(limit);
            if limit > 0
                && scorer.bulk_score(&self.bulk_nodes, &mut self.bulk_scores, limit)?
                    > results.min_competitive_similarity()
            {
                for i in 0..limit as usize {
                    let node = self.bulk_nodes[i];
                    let score = self.bulk_scores[i];
                    if score >= min_accepted_similarity {
                        self.candidates.add(node, score);
                        if accept_ords.map_or(true, |b| b.get(node as usize))
                            && results.collect(node, score)
                        {
                            let old_min = min_accepted_similarity;
                            min_accepted_similarity =
                                next_up_f32(results.min_competitive_similarity());
                            if min_accepted_similarity > old_min {
                                should_explore_min_sim = true;
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn prepare_scratch_state(&mut self, capacity: i32, bulk_score_size: i32) {
        self.candidates.clear();
        let cap = capacity.max(0) as usize;
        if self.visited.length() < cap {
            self.visited = FixedBitSet::new(cap);
        } else {
            self.visited.clear_all();
        }
        let bulk = bulk_score_size.max(0) as usize;
        if self.bulk_nodes.len() < bulk {
            self.bulk_nodes.resize(bulk, 0);
            self.bulk_scores.resize(bulk, 0.0);
        }
    }
}

fn score_entry_points(
    results: &mut dyn KnnCollector,
    scorer: &mut dyn RandomVectorScorer,
    visited: &mut FixedBitSet,
    eps: &[i32],
    accept_ords: Option<&dyn Bits>,
    candidates: &mut NeighborQueue,
    scores: &mut [f32],
) -> Result<()> {
    scorer.bulk_score(eps, &mut scores[..eps.len()], eps.len() as i32)?;
    results.inc_visited_count(eps.len() as i32);
    for i in 0..eps.len() {
        let score = scores[i];
        let ep = eps[i];
        visited.set(ep as usize);
        candidates.add(ep, score);
        if accept_ords.map_or(true, |b| b.get(ep as usize)) {
            results.collect(ep, score);
        }
    }
    Ok(())
}

/// Thread-safe searcher for `OnHeapHnswGraph` that avoids mutating the graph.
struct OnHeapSearcher {
    candidates: NeighborQueue,
    visited: FixedBitSet,
    bulk_nodes: Vec<i32>,
    bulk_scores: Vec<f32>,
    cur_level: i32,
    cur_node: i32,
    cur_neighbors: Option<NeighborArray>,
    cur_upto: i32,
}

impl OnHeapSearcher {
    fn new(candidates: NeighborQueue, visited: FixedBitSet) -> Self {
        Self {
            candidates,
            visited,
            bulk_nodes: Vec::new(),
            bulk_scores: Vec::new(),
            cur_level: -1,
            cur_node: -1,
            cur_neighbors: None,
            cur_upto: -1,
        }
    }

    fn search(
        &mut self,
        results: &mut dyn KnnCollector,
        scorer: &mut dyn RandomVectorScorer,
        graph: &OnHeapHnswGraph,
        accept_ords: Option<&dyn Bits>,
    ) -> Result<()> {
        let eps = self.find_best_entry_point(scorer, graph, results)?;
        if eps.is_empty() || eps[0] == UNK_EP {
            return Ok(());
        }
        self.search_level(results, scorer, 0, &eps, graph, accept_ords)
    }

    fn graph_seek(&mut self, graph: &OnHeapHnswGraph, level: i32, target: i32) -> Result<()> {
        self.cur_level = level;
        self.cur_node = target;
        self.cur_neighbors = Some(graph.get_neighbors(level, target)?.clone());
        self.cur_upto = -1;
        Ok(())
    }

    fn graph_next_neighbor(&mut self) -> i32 {
        let cur = self.cur_neighbors.as_ref().expect("graph_seek not called");
        self.cur_upto += 1;
        if self.cur_upto < cur.size() {
            cur.nodes()[self.cur_upto as usize]
        } else {
            NO_MORE_DOCS
        }
    }

    fn prepare_scratch_state(&mut self, capacity: i32, bulk_score_size: i32) {
        self.candidates.clear();
        let cap = capacity.max(0) as usize;
        if self.visited.length() < cap {
            self.visited = FixedBitSet::new(cap);
        } else {
            self.visited.clear_all();
        }
        let bulk = bulk_score_size.max(0) as usize;
        if self.bulk_nodes.len() < bulk {
            self.bulk_nodes.resize(bulk, 0);
            self.bulk_scores.resize(bulk, 0.0);
        }
    }

    fn find_best_entry_point(
        &mut self,
        scorer: &mut dyn RandomVectorScorer,
        graph: &OnHeapHnswGraph,
        collector: &mut dyn KnnCollector,
    ) -> Result<Vec<i32>> {
        let mut current_ep = graph.entry_node()?;
        if current_ep == -1 || graph.num_levels()? == 1 {
            return Ok(vec![current_ep]);
        }
        let size = graph.max_node_id() + 1;
        self.prepare_scratch_state(size, graph.max_conn() * 2);
        let mut current_score = scorer.score(current_ep)?;
        collector.inc_visited_count(1);
        for level in (1..graph.num_levels()?).rev() {
            let mut found_better = true;
            self.visited.set(current_ep as usize);
            while found_better {
                found_better = false;
                self.graph_seek(graph, level, current_ep)?;
                let mut friend_ord;
                let mut num_nodes = 0i32;
                while {
                    friend_ord = self.graph_next_neighbor();
                    friend_ord != NO_MORE_DOCS
                } {
                    if self.visited.get_and_set(friend_ord as usize) {
                        continue;
                    }
                    if collector.early_terminated() {
                        return Ok(vec![UNK_EP]);
                    }
                    self.bulk_nodes[num_nodes as usize] = friend_ord;
                    num_nodes += 1;
                }
                let max_score = if num_nodes > 0 {
                    scorer.bulk_score(&self.bulk_nodes, &mut self.bulk_scores, num_nodes)?
                } else {
                    f32::NEG_INFINITY
                };
                collector.inc_visited_count(num_nodes);
                if max_score > current_score {
                    for i in 0..num_nodes as usize {
                        let score = self.bulk_scores[i];
                        if score > current_score {
                            current_score = score;
                            current_ep = self.bulk_nodes[i];
                            found_better = true;
                        }
                    }
                }
            }
        }
        Ok(if collector.early_terminated() {
            vec![UNK_EP]
        } else {
            vec![current_ep]
        })
    }

    fn search_level(
        &mut self,
        results: &mut dyn KnnCollector,
        scorer: &mut dyn RandomVectorScorer,
        level: i32,
        eps: &[i32],
        graph: &OnHeapHnswGraph,
        accept_ords: Option<&dyn Bits>,
    ) -> Result<()> {
        let size = graph.max_node_id() + 1;
        self.prepare_scratch_state(size, graph.max_conn() * 2);
        if self.bulk_scores.len() < eps.len() {
            self.bulk_scores.resize(eps.len(), 0.0);
        }
        if results.early_terminated() {
            return Ok(());
        }
        score_entry_points(
            results,
            scorer,
            &mut self.visited,
            eps,
            accept_ords,
            &mut self.candidates,
            &mut self.bulk_scores,
        )?;
        if results.early_terminated() {
            return Ok(());
        }

        let mut min_accepted_similarity = next_up_f32(results.min_competitive_similarity());
        let mut should_explore_min_sim = true;
        while self.candidates.size() > 0 && !results.early_terminated() {
            let top_candidate_similarity = self.candidates.top_score();
            if top_candidate_similarity < min_accepted_similarity {
                if should_explore_min_sim
                    && next_up_f32(top_candidate_similarity) == min_accepted_similarity
                {
                    should_explore_min_sim = false;
                } else {
                    break;
                }
            }

            let top_candidate_node = self.candidates.pop();
            self.graph_seek(graph, level, top_candidate_node)?;
            let mut friend_ord;
            let mut num_nodes = 0i32;
            while {
                friend_ord = self.graph_next_neighbor();
                friend_ord != NO_MORE_DOCS
            } {
                if self.visited.get_and_set(friend_ord as usize) {
                    continue;
                }
                if results.early_terminated() {
                    break;
                }
                self.bulk_nodes[num_nodes as usize] = friend_ord;
                num_nodes += 1;
            }

            let limit =
                (results.visit_limit() - results.visited_count()).min(num_nodes as i64) as i32;
            results.inc_visited_count(limit);
            if limit > 0
                && scorer.bulk_score(&self.bulk_nodes, &mut self.bulk_scores, limit)?
                    > results.min_competitive_similarity()
            {
                for i in 0..limit as usize {
                    let node = self.bulk_nodes[i];
                    let score = self.bulk_scores[i];
                    if score >= min_accepted_similarity {
                        self.candidates.add(node, score);
                        if accept_ords.map_or(true, |b| b.get(node as usize))
                            && results.collect(node, score)
                        {
                            let old_min = min_accepted_similarity;
                            min_accepted_similarity =
                                next_up_f32(results.min_competitive_similarity());
                            if min_accepted_similarity > old_min {
                                should_explore_min_sim = true;
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
