//! k-nearest-neighbor collection primitives.
//!
//! Equivalent to `org.apache.lucene.search.KnnCollector` and
//! `org.apache.lucene.search.TopKnnCollector`.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::util::hnsw::NeighborQueue;

/// Collects the results of a kNN search.
///
/// Equivalent to `org.apache.lucene.search.KnnCollector`.
pub trait KnnCollector: Send {
    /// Returns true if the search was terminated early.
    fn early_terminated(&self) -> bool;

    /// Increments the visited-vector count.
    fn inc_visited_count(&mut self, count: i32);

    /// Returns the number of vectors visited so far.
    fn visited_count(&self) -> i64;

    /// Returns the maximum number of vectors the search may visit.
    fn visit_limit(&self) -> i64;

    /// Returns the number of results to collect.
    fn k(&self) -> i32;

    /// Collects the given doc id and similarity. Returns true if the result
    /// was accepted.
    fn collect(&mut self, doc_id: i32, similarity: f32) -> bool;

    /// Returns the minimum similarity required to remain competitive.
    fn min_competitive_similarity(&self) -> f32;
}

/// A simple kNN collector that keeps the top `k` results by similarity.
///
/// Equivalent to `org.apache.lucene.search.TopKnnCollector`.
#[derive(Debug, Clone)]
pub struct TopKnnCollector {
    k: i32,
    visit_limit: i64,
    visited_count: i64,
    queue: NeighborQueue,
}

impl TopKnnCollector {
    /// Creates a new collector.
    pub fn new(k: i32, visit_limit: i64) -> Self {
        Self {
            k,
            visit_limit,
            visited_count: 0,
            queue: NeighborQueue::new(k, false),
        }
    }
}

impl KnnCollector for TopKnnCollector {
    fn early_terminated(&self) -> bool {
        self.visited_count >= self.visit_limit
    }

    fn inc_visited_count(&mut self, count: i32) {
        self.visited_count += count as i64;
    }

    fn visited_count(&self) -> i64 {
        self.visited_count
    }

    fn visit_limit(&self) -> i64 {
        self.visit_limit
    }

    fn k(&self) -> i32 {
        self.k
    }

    fn collect(&mut self, doc_id: i32, similarity: f32) -> bool {
        self.queue.insert_with_overflow(doc_id, similarity)
    }

    fn min_competitive_similarity(&self) -> f32 {
        if self.queue.size() >= self.k {
            self.queue.top_score()
        } else {
            f32::NEG_INFINITY
        }
    }
}

impl TopKnnCollector {
    /// Returns the collected document ids and scores, best first.
    pub fn top_docs(&self) -> Result<Vec<(i32, f32)>> {
        let mut results = Vec::with_capacity(self.queue.size() as usize);
        let mut clone = self.queue.clone();
        while clone.size() > 0 {
            results.push((clone.top_node(), clone.top_score()));
            clone.pop();
        }
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        Ok(results)
    }

    /// Returns the collected node ids in arbitrary order.
    #[cfg(test)]
    pub fn nodes(&self) -> Vec<i32> {
        self.queue.nodes()
    }
}
