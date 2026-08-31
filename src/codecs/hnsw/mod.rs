//! Flat and HNSW vector codec helpers.
//!
//! Equivalent to `org.apache.lucene.codecs.hnsw`.
//!
//! This module contains the shared abstractions used by vector formats:
//! flat field writers/readers, vector scoring, and the `DefaultFlatVectorScorer`.

pub mod flat_vectors;
pub mod graph_provider;
pub mod scalar_quantized_scorer;

pub use flat_vectors::{
    DefaultFlatVectorScorer, DocsWithFieldSet, FlatFieldVectorsWriter, FlatVectorScorerUtil,
    FlatVectorsFormat, FlatVectorsReader, FlatVectorsScorer, FlatVectorsWriter,
};
