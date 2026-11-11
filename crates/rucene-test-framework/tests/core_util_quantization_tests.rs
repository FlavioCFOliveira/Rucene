//! Port of tests of
//! `lucene/core/src/test/org/apache/lucene/util/quantization/` (sprint 9,
//! task #402): `TestOptimizedScalarQuantizer`,
//! `TestScalarQuantizedVectorSimilarity` and `TestScalarQuantizer`. Each
//! class runs as a suite under `LuceneTestCase`'s rule chain.

// The ports keep Java's indexed loops, counters and unit bindings line by
// line.
#![allow(
    clippy::needless_range_loop,
    clippy::explicit_counter_loop,
    clippy::let_unit_value
)]

mod core_support;

#[path = "core_util_quantization/test_optimized_scalar_quantizer.rs"]
mod test_optimized_scalar_quantizer;
#[path = "core_util_quantization/test_scalar_quantized_vector_similarity.rs"]
mod test_scalar_quantized_vector_similarity;
#[path = "core_util_quantization/test_scalar_quantizer.rs"]
mod test_scalar_quantizer;

use core_support::run;

#[test]
fn test_optimized_scalar_quantizer() {
    run(test_optimized_scalar_quantizer::class_def);
}

#[test]
fn test_scalar_quantized_vector_similarity() {
    run(test_scalar_quantized_vector_similarity::class_def);
}

#[test]
fn test_scalar_quantizer() {
    run(test_scalar_quantizer::class_def);
}
