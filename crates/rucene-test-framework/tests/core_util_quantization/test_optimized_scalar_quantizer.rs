//! Port of
//! `lucene/core/src/test/org/apache/lucene/util/quantization/TestOptimizedScalarQuantizer.java`.
//!
//! `randomFloat()`, `randomIntBetween` and `randomBoolean()` are the
//! `RandomizedTest` helpers over `random()`.

use std::sync::Arc;

use randomizedtesting::generators::RandomNumbers;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::VectorSimilarityFunction;
use rucene::util::quantization::optimized_scalar_quantizer::MINIMUM_MSE_GRID;
use rucene::util::quantization::quantized_byte_vector_values::ScalarEncoding;
use rucene::util::quantization::{OptimizedScalarQuantizer, QuantizationResult};

use crate::core_support::{assert_array_equals, assert_equals, class, random, test};
use crate::test_scalar_quantizer::{l2normalize, SIMILARITY_FUNCTIONS};

/// `TestOptimizedScalarQuantizer`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.quantization.TestOptimizedScalarQuantizer",
        vec![
            test("testQuantizationQuality", test_quantization_quality),
            test("testAbusiveEdgeCases", test_abusive_edge_cases),
            test("testMathematicalConsistency", test_mathematical_consistency),
            test("testUnpackBinary", test_unpack_binary),
            test("testPackTransposeDibit", test_pack_transpose_dibit),
        ],
    )
}

const ALL_BITS: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

/// `RandomizedTest.randomFloat()`.
fn random_float() -> f32 {
    random().next_float()
}

/// `RandomizedTest.randomIntBetween(min, max)`.
fn random_int_between(min: i32, max: i32) -> i32 {
    RandomNumbers::random_int_between(&*random(), min, max)
}

/// `osq.multiScalarQuantize(vector, destinations, ALL_BITS, centroid)`.
fn multi(
    osq: &OptimizedScalarQuantizer,
    vector: &mut [f32],
    destinations: &mut [Vec<u8>],
    centroid: &[f32],
) -> Result<Vec<QuantizationResult>> {
    let mut dests: Vec<&mut [u8]> = destinations.iter_mut().map(|d| d.as_mut_slice()).collect();
    osq.multi_scalar_quantize(vector, &mut dests, &ALL_BITS, centroid)
}

fn test_quantization_quality() -> Result<()> {
    let dims = 16;
    let num_vectors = 32;
    let mut vectors: Vec<Vec<f32>> = Vec::with_capacity(num_vectors);
    let mut centroid = vec![0f32; dims];
    for _ in 0..num_vectors {
        let mut v = vec![0f32; dims];
        for j in 0..dims {
            v[j] = random_float();
            centroid[j] += v[j];
        }
        vectors.push(v);
    }
    for j in 0..dims {
        centroid[j] /= num_vectors as f32;
    }
    // similarity doesn't matter for this test
    let osq = OptimizedScalarQuantizer::new(VectorSimilarityFunction::DOT_PRODUCT);
    let mut scratch = vec![0f32; dims];
    for bit in ALL_BITS {
        let eps = 1.0 / (1i32 << bit) as f32;
        let mut destination = vec![0u8; dims];
        for i in 0..num_vectors {
            scratch.copy_from_slice(&vectors[i]);
            let result = osq.scalar_quantize(&mut scratch, &mut destination, bit, &centroid)?;
            assert_valid_results(&[result]);
            assert_valid_quantized_range(&destination, bit);

            let dequantized =
                OptimizedScalarQuantizer::de_quantize(&destination, bit, &result, &centroid);
            let mut mae = 0f32;
            for k in 0..dims {
                mae += (dequantized[k] - vectors[i][k]).abs();
            }
            mae /= dims as f32;
            Assert::assert_true_msg(&format!("bits: {bit} mae: {mae} > eps: {eps}"), mae <= eps);
        }
    }
    Ok(())
}

fn test_abusive_edge_cases() -> Result<()> {
    // large zero array
    for vector_similarity_function in SIMILARITY_FUNCTIONS {
        if vector_similarity_function == VectorSimilarityFunction::COSINE {
            continue;
        }
        let mut vector = vec![0f32; 4096];
        let centroid = vec![0f32; 4096];
        let osq = OptimizedScalarQuantizer::new(vector_similarity_function);
        let mut destinations = vec![vec![0u8; 4096]; MINIMUM_MSE_GRID.len()];
        let results = multi(&osq, &mut vector, &mut destinations, &centroid)?;
        assert_equals(&MINIMUM_MSE_GRID.len(), &results.len());
        assert_valid_results(&results);
        for destination in &destinations {
            assert_array_equals(&vec![0u8; 4096], destination);
        }
        let mut destination = vec![0u8; 4096];
        for bit in ALL_BITS {
            let result = osq.scalar_quantize(&mut vector, &mut destination, bit, &centroid)?;
            assert_valid_results(&[result]);
            assert_array_equals(&vec![0u8; 4096], &destination);
        }
    }

    // single value array
    for vector_similarity_function in SIMILARITY_FUNCTIONS {
        let mut vector = vec![random_float()];
        let mut centroid = vec![random_float()];
        if vector_similarity_function == VectorSimilarityFunction::COSINE {
            l2normalize(&mut vector)?;
            l2normalize(&mut centroid)?;
        }
        let osq = OptimizedScalarQuantizer::new(vector_similarity_function);
        let mut destinations = vec![vec![0u8; 1]; MINIMUM_MSE_GRID.len()];
        let results = multi(&osq, &mut vector, &mut destinations, &centroid)?;
        assert_equals(&MINIMUM_MSE_GRID.len(), &results.len());
        assert_valid_results(&results);
        for i in 0..ALL_BITS.len() {
            assert_valid_quantized_range(&destinations[i], ALL_BITS[i]);
        }
        for bit in ALL_BITS {
            vector = vec![random_float()];
            centroid = vec![random_float()];
            if vector_similarity_function == VectorSimilarityFunction::COSINE {
                l2normalize(&mut vector)?;
                l2normalize(&mut centroid)?;
            }
            let mut destination = vec![0u8; 1];
            let result = osq.scalar_quantize(&mut vector, &mut destination, bit, &centroid)?;
            assert_valid_results(&[result]);
            assert_valid_quantized_range(&destination, bit);
        }
    }
    Ok(())
}

fn test_mathematical_consistency() -> Result<()> {
    let dims = random_int_between(1, 4096) as usize;
    let mut vector = vec![0f32; dims];
    for i in 0..dims {
        vector[i] = random_float();
    }
    let mut centroid = vec![0f32; dims];
    for i in 0..dims {
        centroid[i] = random_float();
    }
    let mut copy = vec![0f32; dims];
    for vector_similarity_function in SIMILARITY_FUNCTIONS {
        // copy the vector to avoid modifying it
        copy.copy_from_slice(&vector);
        if vector_similarity_function == VectorSimilarityFunction::COSINE {
            l2normalize(&mut copy)?;
            l2normalize(&mut centroid)?;
        }
        let osq = OptimizedScalarQuantizer::new(vector_similarity_function);
        let mut destinations = vec![vec![0u8; dims]; MINIMUM_MSE_GRID.len()];
        let results = multi(&osq, &mut copy, &mut destinations, &centroid)?;
        assert_equals(&MINIMUM_MSE_GRID.len(), &results.len());
        assert_valid_results(&results);
        for i in 0..ALL_BITS.len() {
            assert_valid_quantized_range(&destinations[i], ALL_BITS[i]);
        }
        for bit in ALL_BITS {
            let mut destination = vec![0u8; dims];
            copy.copy_from_slice(&vector);
            if vector_similarity_function == VectorSimilarityFunction::COSINE {
                l2normalize(&mut copy)?;
                l2normalize(&mut centroid)?;
            }
            let result = osq.scalar_quantize(&mut copy, &mut destination, bit, &centroid)?;
            assert_valid_results(&[result]);
            assert_valid_quantized_range(&destination, bit);
        }
    }
    Ok(())
}

fn test_unpack_binary() -> Result<()> {
    let dim = random_int_between(1, 4096);
    let encoding = ScalarEncoding::SingleBitQueryNibble;
    let mut scratch = vec![0u8; encoding.get_discrete_dimensions(dim) as usize];
    for i in 0..scratch.len() {
        scratch[i] = if random().next_boolean() { 1 } else { 0 };
    }
    let mut packed = vec![0u8; encoding.get_doc_packed_length(scratch.len() as i32) as usize];
    let mut unpacked = vec![0u8; scratch.len()];
    OptimizedScalarQuantizer::pack_as_binary(&scratch, &mut packed);
    OptimizedScalarQuantizer::unpack_binary(&packed, &mut unpacked);
    assert_array_equals(&scratch, &unpacked);
    Ok(())
}

fn test_pack_transpose_dibit() -> Result<()> {
    let dim = random_int_between(1, 4096);
    let encoding = ScalarEncoding::DibitQueryNibble;
    let mut scratch = vec![0u8; encoding.get_discrete_dimensions(dim) as usize];
    for i in 0..scratch.len() {
        scratch[i] = random_int_between(0, 3) as u8;
    }
    let mut packed = vec![0u8; encoding.get_doc_packed_length(scratch.len() as i32) as usize];
    let mut unpacked = vec![0u8; scratch.len()];
    OptimizedScalarQuantizer::transpose_dibit(&scratch, &mut packed);
    OptimizedScalarQuantizer::untranspose_dibit(&packed, &mut unpacked);
    assert_array_equals(&scratch, &unpacked);
    Ok(())
}

/// `assertValidQuantizedRange(byte[] quantized, byte bits)`: Java's signed
/// `byte` comparisons.
fn assert_valid_quantized_range(quantized: &[u8], bits: u8) {
    for &b in quantized {
        let b = b as i8 as i32;
        if bits < 8 {
            Assert::assert_true(b >= 0);
        }
        Assert::assert_true(b < 1 << bits);
    }
}

/// `assertValidResults(QuantizationResult... results)`.
fn assert_valid_results(results: &[QuantizationResult]) {
    for result in results {
        Assert::assert_true(result.lower_interval.is_finite());
        Assert::assert_true(result.upper_interval.is_finite());
        Assert::assert_true(result.lower_interval <= result.upper_interval);
        Assert::assert_true(result.additional_correction.is_finite());
        Assert::assert_true(result.quantized_component_sum >= 0);
    }
}
