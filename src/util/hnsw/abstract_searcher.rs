//! Port of `org.apache.lucene.util.hnsw.AbstractHnswGraphSearcher`.

use crate::error::Result;
use crate::search::knn::KnnCollector;
use crate::util::bit_set::BitSet;
use crate::util::Bits;

use super::neighbor::NeighborQueue;
use super::scorer::RandomVectorScorer;
use super::HnswGraph;

/// Marks that the graph entry node is not set, or that the visitation limit was
/// exceeded.
///
/// Equivalent to `AbstractHnswGraphSearcher.UNK_EP`.
pub const UNK_EP: i32 = -1;

/// Base contract for the HNSW graph searchers.
///
/// Equivalent to `org.apache.lucene.util.hnsw.AbstractHnswGraphSearcher`, which Java
/// declares as an abstract class; Rust expresses the same two abstract methods plus
/// the concrete `search` as a trait with a default method.
pub trait AbstractHnswGraphSearcher {
    /// Searches a given level of the graph, starting at the given entry points.
    ///
    /// # Errors
    ///
    /// Returns an error when accessing the vectors or the graph fails.
    fn search_level(
        &mut self,
        results: &mut dyn KnnCollector,
        scorer: &mut dyn RandomVectorScorer,
        level: i32,
        eps: &[i32],
        graph: &mut dyn HnswGraph,
        accept_ords: Option<&dyn Bits>,
    ) -> Result<()>;

    /// Finds the best entry points from which to search the zeroth graph layer.
    ///
    /// A single entry of [`UNK_EP`] means the graph entry node is not set, or that
    /// the visitation limit was exceeded.
    ///
    /// # Errors
    ///
    /// Returns an error when accessing the vectors or the graph fails.
    fn find_best_entry_point(
        &mut self,
        scorer: &mut dyn RandomVectorScorer,
        graph: &mut dyn HnswGraph,
        collector: &mut dyn KnnCollector,
    ) -> Result<Vec<i32>>;

    /// Searches the graph for the given scorer, gathering results in the provided
    /// collector that pass `accept_ords`.
    ///
    /// # Errors
    ///
    /// Returns an error when accessing the vectors or the graph fails.
    fn search(
        &mut self,
        results: &mut dyn KnnCollector,
        scorer: &mut dyn RandomVectorScorer,
        graph: &mut dyn HnswGraph,
        accept_ords: Option<&dyn Bits>,
    ) -> Result<()> {
        let eps = self.find_best_entry_point(scorer, graph, results)?;
        debug_assert!(!eps.is_empty());
        if eps[0] == UNK_EP {
            return Ok(());
        }
        self.search_level(results, scorer, 0, &eps, graph, accept_ords)
    }
}

/// Scores the entry points and seeds the candidate queue with them.
///
/// Equivalent to the `protected static`
/// `org.apache.lucene.util.hnsw.AbstractHnswGraphSearcher#scoreEntryPoints`,
/// which Rust expresses as a free function in the same module, since a trait
/// cannot carry a static method that its implementors share.
///
/// # Errors
///
/// Returns an error when scoring the entry points fails.
pub(crate) fn score_entry_points(
    results: &mut dyn KnnCollector,
    scorer: &mut dyn RandomVectorScorer,
    visited: &mut dyn BitSet,
    eps: &[i32],
    accept_ords: Option<&dyn Bits>,
    candidates: &mut NeighborQueue,
    scores: &mut [f32],
) -> Result<()> {
    debug_assert!(!eps.is_empty());
    debug_assert!(scores.len() >= eps.len());
    scorer.bulk_score(eps, &mut scores[..eps.len()], eps.len() as i32)?;
    results.inc_visited_count(eps.len() as i32);
    for i in 0..eps.len() {
        let score = scores[i];
        let ep = eps[i];
        visited.set(ep as usize);
        candidates.add(ep, score);
        if accept_ords.map_or(Ok(true), |b| b.try_get(ep as usize))? {
            results.collect(ep, score);
        }
    }
    Ok(())
}
