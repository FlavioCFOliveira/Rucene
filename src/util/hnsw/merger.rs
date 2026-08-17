//! Graph merging utilities.
//!
//! Equivalent to `org.apache.lucene.util.hnsw.HnswGraphMerger` and
//! `org.apache.lucene.util.hnsw.ConcurrentHnswMerger`.

#![deny(unsafe_code)]

use crate::error::Result;

use super::builder::{HnswBuilder, HnswGraphBuilder, DEFAULT_RAND_SEED};
use super::on_heap::OnHeapHnswGraph;
use super::scorer::RandomVectorScorerSupplier;

/// Merges multiple HNSW graphs into a single on-heap graph.
///
/// Equivalent to `org.apache.lucene.util.hnsw.HnswGraphMerger`.
pub trait HnswGraphMerger {
    /// Merges the provided vectors into a new graph.
    ///
    /// `total_vector_count` is the total number of vectors after merging.
    fn merge(
        &mut self,
        scorer_supplier: &dyn RandomVectorScorerSupplier,
        total_vector_count: i32,
    ) -> Result<OnHeapHnswGraph>;
}

/// A basic merger that rebuilds the graph from the merged vectors.
///
/// This is a simplified stand-in for the full Lucene concurrent merge path.
/// It simply runs a standard `HnswGraphBuilder` over the full merged vector
/// set.
///
/// Equivalent in spirit to `org.apache.lucene.util.hnsw.ConcurrentHnswMerger`.
#[derive(Debug, Default)]
pub struct ConcurrentHnswMerger {
    m: i32,
    beam_width: i32,
}

impl ConcurrentHnswMerger {
    /// Creates a merger with the given graph hyperparameters.
    pub fn new(m: i32, beam_width: i32) -> Self {
        Self { m, beam_width }
    }
}

impl HnswGraphMerger for ConcurrentHnswMerger {
    fn merge(
        &mut self,
        scorer_supplier: &dyn RandomVectorScorerSupplier,
        total_vector_count: i32,
    ) -> Result<OnHeapHnswGraph> {
        let mut builder = HnswGraphBuilder::create(
            scorer_supplier,
            self.m,
            self.beam_width,
            DEFAULT_RAND_SEED,
            total_vector_count,
        )?;
        builder.build(total_vector_count)?;
        Ok(builder.into_graph())
    }
}
