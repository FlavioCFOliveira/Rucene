//! Scalar quantization ported from `org.apache.lucene.util.quantization`.
//!
//! Compresses a float vector into one byte per dimension (or half a byte at four
//! bits), so a KNN index holds four to eight times fewer bytes, and scores the
//! compressed form with the corrections that keep the ranking close to the
//! original.

pub mod optimized_scalar_quantizer;
pub mod quantized_byte_vector_values;
pub mod scalar_quantized_vector_similarity;
pub mod scalar_quantizer;

pub use optimized_scalar_quantizer::{OptimizedScalarQuantizer, QuantizationResult};
pub use quantized_byte_vector_values::{
    BaseQuantizedByteVectorValues, LegacyQuantizedByteVectorValues, QuantizedByteVectorValues,
    QuantizedVectorsReader, ScalarEncoding,
};
pub use scalar_quantized_vector_similarity::ScalarQuantizedVectorSimilarity;
pub use scalar_quantizer::{ScalarQuantizer, SCALAR_QUANTIZATION_SAMPLE_SIZE};
