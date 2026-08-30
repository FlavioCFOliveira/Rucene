//! Port of `org.apache.lucene.util.hnsw.SeededHnswGraphSearcher`.

use crate::error::{LuceneError, Result};
use crate::search::knn::KnnCollector;
use crate::search::{DocIdSetIterator, NO_MORE_DOCS};
use crate::util::Bits;

use super::abstract_searcher::AbstractHnswGraphSearcher;
use super::scorer::RandomVectorScorer;
use super::HnswGraph;

/// An HNSW graph searcher that uses a set of seed ordinals to initiate the search.
///
/// Equivalent to `org.apache.lucene.util.hnsw.SeededHnswGraphSearcher`.
pub struct SeededHnswGraphSearcher {
    delegate: Box<dyn AbstractHnswGraphSearcher>,
    seed_ords: Vec<i32>,
}

impl SeededHnswGraphSearcher {
    /// Wraps `delegate`, entering the graph at `seed_ords`.
    pub fn new(delegate: Box<dyn AbstractHnswGraphSearcher>, seed_ords: Vec<i32>) -> Self {
        Self {
            delegate,
            seed_ords,
        }
    }

    /// Builds a seeded searcher whose entry points are the first `num_eps` documents
    /// of `eps`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if `num_eps` is not positive, or if
    /// `eps` yields fewer than `num_eps` entry points.
    pub fn from_entry_points(
        delegate: Box<dyn AbstractHnswGraphSearcher>,
        num_eps: i32,
        eps: &mut dyn DocIdSetIterator,
        graph_size: i32,
    ) -> Result<Self> {
        if num_eps <= 0 {
            return Err(LuceneError::IllegalArgument(
                "The number of entry points must be > 0".to_string(),
            ));
        }
        let mut entry_points = vec![0i32; num_eps as usize];
        let mut idx = 0usize;
        while idx < entry_points.len() {
            let entry_point_ord = eps.next_doc()?;
            if entry_point_ord == NO_MORE_DOCS {
                return Err(LuceneError::IllegalArgument(
                    "The number of entry points provided is less than the number of entry points requested"
                        .to_string(),
                ));
            }
            debug_assert!(entry_point_ord < graph_size);
            entry_points[idx] = entry_point_ord;
            idx += 1;
        }
        Ok(Self::new(delegate, entry_points))
    }
}

impl AbstractHnswGraphSearcher for SeededHnswGraphSearcher {
    fn search_level(
        &mut self,
        results: &mut dyn KnnCollector,
        scorer: &mut dyn RandomVectorScorer,
        level: i32,
        eps: &[i32],
        graph: &mut dyn HnswGraph,
        accept_ords: Option<&dyn Bits>,
    ) -> Result<()> {
        self.delegate
            .search_level(results, scorer, level, eps, graph, accept_ords)
    }

    fn find_best_entry_point(
        &mut self,
        _scorer: &mut dyn RandomVectorScorer,
        _graph: &mut dyn HnswGraph,
        _collector: &mut dyn KnnCollector,
    ) -> Result<Vec<i32>> {
        Ok(self.seed_ords.clone())
    }
}
