//! Port of
//! `lucene/core/src/test/org/apache/lucene/util/quantization/TestScalarQuantizedVectorSimilarity.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::VectorSimilarityFunction;
use rucene::util::quantization::{ScalarQuantizedVectorSimilarity, ScalarQuantizer};

use crate::core_support::{class, random, test};
use crate::test_scalar_quantizer::{
    from_floats, l2normalize, random_float_array, random_floats, TestSimpleFloatVectorValues,
    SIMILARITY_FUNCTIONS,
};

/// `TestScalarQuantizedVectorSimilarity`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.quantization.TestScalarQuantizedVectorSimilarity",
        vec![
            test("testNonZeroScores", test_non_zero_scores),
            test("testToEuclidean", test_to_euclidean),
            test("testToCosine", test_to_cosine),
            test("testToDotProduct", test_to_dot_product),
            test("testToMaxInnerProduct", test_to_max_inner_product),
        ],
    )
}

fn test_non_zero_scores() -> Result<()> {
    let quantized = vec![vec![0u8; 32]; 2];
    for similarity_function in SIMILARITY_FUNCTIONS {
        let mut multiplier = random().next_float();
        if random().next_boolean() {
            multiplier = -multiplier;
        }
        for bits in [4u8, 7, 8] {
            let quantized_similarity =
                <dyn ScalarQuantizedVectorSimilarity>::from_vector_similarity(
                    similarity_function,
                    multiplier,
                    bits,
                );
            let negative_offset_a =
                -(random().next_float() * (random().next_int_bound(10) + 1) as f32);
            let negative_offset_b =
                -(random().next_float() * (random().next_int_bound(10) + 1) as f32);
            let score = quantized_similarity.score(
                &quantized[0],
                negative_offset_a,
                &quantized[1],
                negative_offset_b,
            )?;
            Assert::assert_true(score >= 0.0);
        }
    }
    Ok(())
}

/// The confidence intervals the `testTo*` methods iterate.
fn confidence_intervals(dims: i32) -> [f32; 5] {
    [0.9, 0.95, 0.99, 1.0 - 1.0 / (dims + 1) as f32, 1.0]
}

fn test_to_euclidean() -> Result<()> {
    let dims = 128;
    let num_vecs = 100;

    let floats = Arc::new(random_floats(num_vecs, dims));
    for confidence_interval in confidence_intervals(dims) {
        let error = ((100.0 - confidence_interval) * 0.01f32).max(0.01);
        let float_vector_values = from_floats(&floats);
        let scalar_quantizer =
            ScalarQuantizer::from_vectors(&float_vector_values, confidence_interval, num_vecs, 7)?;
        let (quantized, offsets) = quantize_vectors(
            &scalar_quantizer,
            &floats,
            VectorSimilarityFunction::EUCLIDEAN,
        )?;
        let query = floats[0][0..dims as usize].to_vec();
        let quantized_similarity = <dyn ScalarQuantizedVectorSimilarity>::from_vector_similarity(
            VectorSimilarityFunction::EUCLIDEAN,
            scalar_quantizer.get_constant_multiplier(),
            scalar_quantizer.get_bits(),
        );
        assert_quantized_scores(
            &floats,
            &quantized,
            &offsets,
            &query,
            error,
            VectorSimilarityFunction::EUCLIDEAN,
            &*quantized_similarity,
            &scalar_quantizer,
        )?;
    }
    Ok(())
}

fn test_to_cosine() -> Result<()> {
    let dims = 128;
    let num_vecs = 100;

    let floats = Arc::new(random_floats(num_vecs, dims));

    for confidence_interval in confidence_intervals(dims) {
        let error = ((100.0 - confidence_interval) * 0.01f32).max(0.01);
        let float_vector_values = from_floats_normalized(&floats);
        let scalar_quantizer =
            ScalarQuantizer::from_vectors(&float_vector_values, confidence_interval, num_vecs, 7)?;
        let (quantized, offsets) = quantize_vectors_normalized(
            &scalar_quantizer,
            &floats,
            VectorSimilarityFunction::COSINE,
        )?;
        let mut query = floats[0][0..dims as usize].to_vec();
        l2normalize(&mut query)?;
        let quantized_similarity = <dyn ScalarQuantizedVectorSimilarity>::from_vector_similarity(
            VectorSimilarityFunction::COSINE,
            scalar_quantizer.get_constant_multiplier(),
            scalar_quantizer.get_bits(),
        );
        assert_quantized_scores(
            &floats,
            &quantized,
            &offsets,
            &query,
            error,
            VectorSimilarityFunction::COSINE,
            &*quantized_similarity,
            &scalar_quantizer,
        )?;
    }
    Ok(())
}

fn test_to_dot_product() -> Result<()> {
    let dims = 128;
    let num_vecs = 100;

    let mut floats = random_floats(num_vecs, dims);
    for fs in floats.iter_mut() {
        l2normalize(fs)?;
    }
    let floats = Arc::new(floats);
    for confidence_interval in confidence_intervals(dims) {
        let error = ((100.0 - confidence_interval) * 0.01f32).max(0.01);
        let float_vector_values = from_floats(&floats);
        let scalar_quantizer =
            ScalarQuantizer::from_vectors(&float_vector_values, confidence_interval, num_vecs, 7)?;
        let (quantized, offsets) = quantize_vectors(
            &scalar_quantizer,
            &floats,
            VectorSimilarityFunction::DOT_PRODUCT,
        )?;
        let mut query = random_float_array(dims);
        l2normalize(&mut query)?;
        let quantized_similarity = <dyn ScalarQuantizedVectorSimilarity>::from_vector_similarity(
            VectorSimilarityFunction::DOT_PRODUCT,
            scalar_quantizer.get_constant_multiplier(),
            scalar_quantizer.get_bits(),
        );
        assert_quantized_scores(
            &floats,
            &quantized,
            &offsets,
            &query,
            error,
            VectorSimilarityFunction::DOT_PRODUCT,
            &*quantized_similarity,
            &scalar_quantizer,
        )?;
    }
    Ok(())
}

fn test_to_max_inner_product() -> Result<()> {
    let dims = 128;
    let num_vecs = 100;

    let floats = Arc::new(random_floats(num_vecs, dims));
    for confidence_interval in confidence_intervals(dims) {
        let error = ((100.0 - confidence_interval) * 0.5f32).max(0.5);
        let float_vector_values = from_floats(&floats);
        let scalar_quantizer =
            ScalarQuantizer::from_vectors(&float_vector_values, confidence_interval, num_vecs, 7)?;
        let (quantized, offsets) = quantize_vectors(
            &scalar_quantizer,
            &floats,
            VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT,
        )?;
        let query = random_float_array(dims);
        let quantized_similarity = <dyn ScalarQuantizedVectorSimilarity>::from_vector_similarity(
            VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT,
            scalar_quantizer.get_constant_multiplier(),
            scalar_quantizer.get_bits(),
        );
        assert_quantized_scores(
            &floats,
            &quantized,
            &offsets,
            &query,
            error,
            VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT,
            &*quantized_similarity,
            &scalar_quantizer,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn assert_quantized_scores(
    floats: &[Vec<f32>],
    quantized: &[Vec<u8>],
    stored_offsets: &[f32],
    query: &[f32],
    error: f32,
    similarity_function: VectorSimilarityFunction,
    quantized_similarity: &dyn ScalarQuantizedVectorSimilarity,
    scalar_quantizer: &ScalarQuantizer,
) -> Result<()> {
    for i in 0..floats.len() {
        let stored_offset = stored_offsets[i];
        let mut quantized_query = vec![0u8; query.len()];
        let query_offset =
            scalar_quantizer.quantize(query, &mut quantized_query, similarity_function)?;
        let original = similarity_function.compare_f32(query, &floats[i])?;
        let quantized_score = quantized_similarity.score(
            &quantized_query,
            query_offset,
            &quantized[i],
            stored_offset,
        )?;
        Assert::assert_equals_float_msg(
            Some(&format!("Not within acceptable error [{error}]")),
            original,
            quantized_score,
            error,
        );
    }
    Ok(())
}

/// `quantizeVectors(...)`: the quantized vectors and their offsets.
fn quantize_vectors(
    scalar_quantizer: &ScalarQuantizer,
    floats: &[Vec<f32>],
    similarity_function: VectorSimilarityFunction,
) -> Result<(Vec<Vec<u8>>, Vec<f32>)> {
    let mut quantized = Vec::with_capacity(floats.len());
    let mut offsets = vec![0f32; floats.len()];
    for (i, v) in floats.iter().enumerate() {
        let mut q = vec![0u8; v.len()];
        offsets[i] = scalar_quantizer.quantize(v, &mut q, similarity_function)?;
        quantized.push(q);
    }
    Ok((quantized, offsets))
}

/// `quantizeVectorsNormalized(...)`.
fn quantize_vectors_normalized(
    scalar_quantizer: &ScalarQuantizer,
    floats: &[Vec<f32>],
    similarity_function: VectorSimilarityFunction,
) -> Result<(Vec<Vec<u8>>, Vec<f32>)> {
    let mut quantized = Vec::with_capacity(floats.len());
    let mut offsets = vec![0f32; floats.len()];
    for (i, f) in floats.iter().enumerate() {
        let mut v = f.clone();
        l2normalize(&mut v)?;
        let mut q = vec![0u8; v.len()];
        offsets[i] = scalar_quantizer.quantize(&v, &mut q, similarity_function)?;
        quantized.push(q);
    }
    Ok((quantized, offsets))
}

/// `fromFloatsNormalized(float[][] floats, Set<Integer> deletedVectors)`
/// with `deletedVectors == null`.
fn from_floats_normalized(floats: &Arc<Vec<Vec<f32>>>) -> TestSimpleFloatVectorValues {
    TestSimpleFloatVectorValues::new(Arc::clone(floats), None, true)
}
