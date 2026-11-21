//! Port of
//! `lucene/core/src/test/org/apache/lucene/util/quantization/TestScalarQuantizer.java`,
//! with its package-private helpers (`shuffleArray`, `randomFloatArray`,
//! `randomFloats`, `fromFloats`, `fromFloatsWithRandomDeletions`,
//! `TestSimpleFloatVectorValues`) that `TestScalarQuantizedVectorSimilarity`
//! uses.
//!
//! `TestSimpleFloatVectorValues.copy()` returns `this`; Rucene's `copy`
//! returns a new box, which shares the vectors (`Arc`) and the deletions.

use std::collections::HashSet;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::vector_values::{DocIndexIterator, FloatVectorValues, KnnVectorValues};
use rucene::index::{VectorEncoding, VectorSimilarityFunction};
use rucene::search::doc_id_set_iterator::DocIdSetIterator;
use rucene::search::NO_MORE_DOCS;
use rucene::util::quantization::scalar_quantizer::SCRATCH_SIZE;
use rucene::util::quantization::ScalarQuantizer;
use rucene::util::vector_util;
use rucene_test_framework::util::java_exceptions::expect_throws;

use crate::core_support::{assert_equals, class, random, test};

/// `TestScalarQuantizer`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.quantization.TestScalarQuantizer",
        vec![
            test("testTinyVectors", test_tiny_vectors),
            test("testNanAndInfValueFailure", test_nan_and_inf_value_failure),
            test(
                "testQuantizeAndDeQuantize7Bit",
                test_quantize_and_de_quantize7_bit,
            ),
            test(
                "testQuantizeAndDeQuantize8Bit",
                test_quantize_and_de_quantize8_bit,
            ),
            test("testQuantiles", test_quantiles),
            test("testEdgeCase", test_edge_case),
            test("testScalarWithSampling", test_scalar_with_sampling),
            test(
                "testFromVectorsAutoInterval4Bit",
                test_from_vectors_auto_interval4_bit,
            ),
        ],
    )
}

/// `VectorSimilarityFunction.values()`.
pub const SIMILARITY_FUNCTIONS: [VectorSimilarityFunction; 4] = [
    VectorSimilarityFunction::EUCLIDEAN,
    VectorSimilarityFunction::DOT_PRODUCT,
    VectorSimilarityFunction::COSINE,
    VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT,
];

/// `VectorUtil.l2normalize(float[])`.
pub fn l2normalize(v: &mut [f32]) -> Result<()> {
    vector_util::l2normalize(v, true)
}

fn test_tiny_vectors() -> Result<()> {
    for function in SIMILARITY_FUNCTIONS {
        let mut function = function;
        let dims = random().next_int_bound(9) + 1;
        let num_vecs = random().next_int_bound(9) + 10;
        let mut floats = random_floats(num_vecs, dims);
        if function == VectorSimilarityFunction::COSINE {
            for v in floats.iter_mut() {
                l2normalize(v)?;
            }
        }
        let floats = Arc::new(floats);
        for bits in [4u8, 7] {
            let float_vector_values = from_floats(&floats);
            if function == VectorSimilarityFunction::COSINE {
                function = VectorSimilarityFunction::DOT_PRODUCT;
            }
            let scalar_quantizer = if random().next_boolean() {
                ScalarQuantizer::from_vectors(&float_vector_values, 0.9, num_vecs, bits)
            } else {
                ScalarQuantizer::from_vectors_auto_interval(
                    &float_vector_values,
                    function,
                    num_vecs,
                    bits,
                )
            };
            // assertNotNull(scalarQuantizer)
            scalar_quantizer?;
        }
    }
    Ok(())
}

fn test_nan_and_inf_value_failure() -> Result<()> {
    for function in SIMILARITY_FUNCTIONS {
        let dims = random().next_int_bound(9) + 1;
        let num_vecs = random().next_int_bound(9) + 10;
        let mut floats = vec![vec![0f32; dims as usize]; num_vecs as usize];
        for i in 0..num_vecs as usize {
            for j in 0..dims as usize {
                floats[i][j] = if random().next_boolean() {
                    f32::NAN
                } else {
                    f32::INFINITY
                };
            }
        }
        let floats = Arc::new(floats);
        for bits in [4u8, 7] {
            let float_vector_values = from_floats(&floats);
            expect_throws("java.lang.IllegalStateException", || {
                ScalarQuantizer::from_vectors(&float_vector_values, 0.9, num_vecs, bits)
            });
            let actual_function = if function == VectorSimilarityFunction::COSINE {
                VectorSimilarityFunction::DOT_PRODUCT
            } else {
                function
            };
            expect_throws("java.lang.IllegalStateException", || {
                ScalarQuantizer::from_vectors_auto_interval(
                    &float_vector_values,
                    actual_function,
                    num_vecs,
                    bits,
                )
            });
        }
    }
    Ok(())
}

/// The quantize / de-quantize / re-quantize round trip of the
/// `testQuantizeAndDeQuantize*` and `testFromVectorsAutoInterval4Bit` tests:
/// the minimum and maximum quantized values seen (as Java's `byte`).
fn round_trip(
    scalar_quantizer: &ScalarQuantizer,
    floats: &[Vec<f32>],
    dims: usize,
    similarity_function: VectorSimilarityFunction,
    delta: f64,
) -> Result<(i8, i8)> {
    let mut dequantized = vec![0f32; dims];
    let mut quantized = vec![0u8; dims];
    let mut requantized = vec![0u8; dims];
    let mut max_dim_value: i8 = -128;
    let mut min_dim_value: i8 = 127;
    for v in floats {
        scalar_quantizer.quantize(v, &mut quantized, similarity_function)?;
        scalar_quantizer.de_quantize(&quantized, &mut dequantized)?;
        scalar_quantizer.quantize(&dequantized, &mut requantized, similarity_function)?;
        for j in 0..dims {
            let q = quantized[j] as i8;
            if q > max_dim_value {
                max_dim_value = q;
            }
            if q < min_dim_value {
                min_dim_value = q;
            }
            Assert::assert_equals_double_msg(None, dequantized[j] as f64, v[j] as f64, delta);
            assert_equals(&quantized[j], &requantized[j]);
        }
    }
    Ok((min_dim_value, max_dim_value))
}

fn test_quantize_and_de_quantize7_bit() -> Result<()> {
    let dims = 128;
    let num_vecs = 100;
    let similarity_function = VectorSimilarityFunction::DOT_PRODUCT;

    let floats = Arc::new(random_floats(num_vecs, dims));
    let float_vector_values = from_floats(&floats);
    let scalar_quantizer = ScalarQuantizer::from_vectors(&float_vector_values, 1.0, num_vecs, 7)?;
    let (min_dim_value, _max) = round_trip(
        &scalar_quantizer,
        &floats,
        dims as usize,
        similarity_function,
        0.02,
    )?;
    // int7 should always quantize to 0-127
    Assert::assert_true(min_dim_value >= 0);
    Ok(())
}

fn test_quantize_and_de_quantize8_bit() -> Result<()> {
    let dims = 128;
    let num_vecs = 100;
    let similarity_function = VectorSimilarityFunction::DOT_PRODUCT;

    let floats = Arc::new(random_floats(num_vecs, dims));
    let float_vector_values = from_floats(&floats);
    let scalar_quantizer = ScalarQuantizer::from_vectors(&float_vector_values, 1.0, num_vecs, 8)?;
    round_trip(
        &scalar_quantizer,
        &floats,
        dims as usize,
        similarity_function,
        0.02,
    )?;
    Ok(())
}

fn test_quantiles() -> Result<()> {
    let mut percs = vec![0f32; 1000];
    for i in 0..1000 {
        percs[i] = i as f32;
    }
    shuffle_array(&mut percs);
    let upper_and_lower = ScalarQuantizer::get_upper_and_lower_quantile(&mut percs, 0.9);
    Assert::assert_equals_double_msg(None, 50.0, upper_and_lower[0] as f64, 1e-7);
    Assert::assert_equals_double_msg(None, 949.0, upper_and_lower[1] as f64, 1e-7);
    shuffle_array(&mut percs);
    let upper_and_lower = ScalarQuantizer::get_upper_and_lower_quantile(&mut percs, 0.95);
    Assert::assert_equals_double_msg(None, 25.0, upper_and_lower[0] as f64, 1e-7);
    Assert::assert_equals_double_msg(None, 974.0, upper_and_lower[1] as f64, 1e-7);
    shuffle_array(&mut percs);
    let upper_and_lower = ScalarQuantizer::get_upper_and_lower_quantile(&mut percs, 0.99);
    Assert::assert_equals_double_msg(None, 5.0, upper_and_lower[0] as f64, 1e-7);
    Assert::assert_equals_double_msg(None, 994.0, upper_and_lower[1] as f64, 1e-7);
    Ok(())
}

fn test_edge_case() -> Result<()> {
    let upper_and_lower =
        ScalarQuantizer::get_upper_and_lower_quantile(&mut [1.0, 1.0, 1.0, 1.0, 1.0], 0.9);
    Assert::assert_equals_float_msg(None, 1.0, upper_and_lower[0], 1e-7);
    Assert::assert_equals_float_msg(None, 1.0, upper_and_lower[1], 1e-7);
    Ok(())
}

fn test_scalar_with_sampling() -> Result<()> {
    let num_vecs = random().next_int_bound(128) + 5;
    let dims = 64;
    let floats = Arc::new(random_floats(num_vecs, dims));
    // Should be able to handle empty vectors
    for _ in 0..3 {
        let n = random().next_int_bound(num_vecs - 1) + 1;
        let float_vector_values = from_floats_with_random_deletions(&floats, n);
        ScalarQuantizer::from_vectors_with_sample_size(
            &float_vector_values,
            0.99,
            float_vector_values.num_live_vectors,
            7,
            (float_vector_values.num_live_vectors - 1).max(SCRATCH_SIZE as i32 + 1),
        )?;
    }
    {
        let n = random().next_int_bound(num_vecs - 1) + 1;
        let float_vector_values = from_floats_with_random_deletions(&floats, n);
        let sample = (random().next_int_bound(float_vector_values.floats.len() as i32 - 1) + 1)
            .max(SCRATCH_SIZE as i32 + 1);
        ScalarQuantizer::from_vectors_with_sample_size(
            &float_vector_values,
            0.99,
            float_vector_values.num_live_vectors,
            7,
            sample,
        )?;
    }
    Ok(())
}

fn test_from_vectors_auto_interval4_bit() -> Result<()> {
    let dims = 128;
    let num_vecs = 100;
    let similarity_function = VectorSimilarityFunction::DOT_PRODUCT;

    let mut floats = random_floats(num_vecs, dims);
    for v in floats.iter_mut() {
        l2normalize(v)?;
    }
    let floats = Arc::new(floats);

    let float_vector_values = from_floats(&floats);
    let scalar_quantizer = ScalarQuantizer::from_vectors_auto_interval(
        &float_vector_values,
        similarity_function,
        num_vecs,
        4,
    )?;
    let (min_dim_value, max_dim_value) = round_trip(
        &scalar_quantizer,
        &floats,
        dims as usize,
        similarity_function,
        0.2,
    )?;
    Assert::assert_true(min_dim_value >= 0);
    Assert::assert_true(max_dim_value <= 15);
    Ok(())
}

/// `shuffleArray(float[] ar)`.
pub fn shuffle_array(ar: &mut [f32]) {
    let mut i = ar.len() as i32 - 1;
    while i > 0 {
        let index = random().next_int_bound(i + 1) as usize;
        ar.swap(index, i as usize);
        i -= 1;
    }
}

/// `randomFloatArray(int dims)`.
pub fn random_float_array(dims: i32) -> Vec<f32> {
    (0..dims)
        .map(|_| random().next_float_range(-1.0, 1.0))
        .collect()
}

/// `randomFloats(int num, int dims)`.
pub fn random_floats(num: i32, dims: i32) -> Vec<Vec<f32>> {
    (0..num).map(|_| random_float_array(dims)).collect()
}

/// `fromFloats(float[][] floats)`.
pub fn from_floats(floats: &Arc<Vec<Vec<f32>>>) -> TestSimpleFloatVectorValues {
    TestSimpleFloatVectorValues::new(Arc::clone(floats), None, false)
}

/// `fromFloatsWithRandomDeletions(float[][] floats, int numDeleted)`.
pub fn from_floats_with_random_deletions(
    floats: &Arc<Vec<Vec<f32>>>,
    num_deleted: i32,
) -> TestSimpleFloatVectorValues {
    // Only `contains` and `size` are read: iteration order is not observed.
    let mut deleted_vectors: HashSet<i32> = HashSet::new();
    for _ in 0..num_deleted {
        deleted_vectors.insert(random().next_int_bound(floats.len() as i32));
    }
    TestSimpleFloatVectorValues::new(Arc::clone(floats), Some(Arc::new(deleted_vectors)), false)
}

/// `static class TestSimpleFloatVectorValues extends FloatVectorValues`;
/// `normalized` is the anonymous subclass of
/// `TestScalarQuantizedVectorSimilarity.fromFloatsNormalized`, whose
/// `vectorValue` returns an l2-normalized copy.
#[derive(Clone)]
pub struct TestSimpleFloatVectorValues {
    pub floats: Arc<Vec<Vec<f32>>>,
    pub deleted_vectors: Option<Arc<HashSet<i32>>>,
    pub ord_to_doc: Arc<Vec<i32>>,
    pub num_live_vectors: i32,
    pub normalized: bool,
}

impl TestSimpleFloatVectorValues {
    pub fn new(
        values: Arc<Vec<Vec<f32>>>,
        deleted_vectors: Option<Arc<HashSet<i32>>>,
        normalized: bool,
    ) -> Self {
        let num_live_vectors = match &deleted_vectors {
            None => values.len() as i32,
            Some(d) => (values.len() - d.len()) as i32,
        };
        let mut ord_to_doc = vec![0i32; num_live_vectors as usize];
        match &deleted_vectors {
            None => {
                for i in 0..num_live_vectors as usize {
                    ord_to_doc[i] = i as i32;
                }
            }
            Some(d) => {
                let mut ord = 0;
                for doc in 0..values.len() as i32 {
                    if !d.contains(&doc) {
                        ord_to_doc[ord] = doc;
                        ord += 1;
                    }
                }
            }
        }
        TestSimpleFloatVectorValues {
            floats: values,
            deleted_vectors,
            ord_to_doc: Arc::new(ord_to_doc),
            num_live_vectors,
            normalized,
        }
    }
}

impl KnnVectorValues for TestSimpleFloatVectorValues {
    fn dimension(&self) -> i32 {
        self.floats[0].len() as i32
    }

    fn size(&self) -> i32 {
        self.floats.len() as i32
    }

    fn ord_to_doc(&self, ord: i32) -> i32 {
        self.ord_to_doc[ord as usize]
    }

    fn copy(&self) -> Result<Box<dyn KnnVectorValues>> {
        Ok(Box::new(self.clone()))
    }

    fn encoding(&self) -> VectorEncoding {
        VectorEncoding::FLOAT32
    }

    fn iterator(&self) -> Result<Box<dyn DocIndexIterator>> {
        Ok(Box::new(SimpleDocIndexIterator {
            floats_len: self.floats.len() as i32,
            deleted_vectors: self.deleted_vectors.clone(),
            ord: -1,
            doc: -1,
        }))
    }
}

impl FloatVectorValues for TestSimpleFloatVectorValues {
    fn vector_value(&self, ord: i32) -> Result<Vec<f32>> {
        let v = self.floats[KnnVectorValues::ord_to_doc(self, ord) as usize].clone();
        if self.normalized {
            // `ArrayUtil.copyArray(floats[ordToDoc[ord]])` normalized.
            let mut v = v;
            l2normalize(&mut v)?;
            return Ok(v);
        }
        Ok(v)
    }

    fn copy_float(&self) -> Result<Box<dyn FloatVectorValues>> {
        Ok(Box::new(self.clone()))
    }

    fn scorer(
        &self,
        _target: &[f32],
    ) -> Result<Option<Box<dyn rucene::search::vector_scorer::VectorScorer>>> {
        randomizedtesting::jdk::exception::Throwable::new(
            "java.lang.UnsupportedOperationException",
            String::new(),
        )
        .throw()
    }
}

/// The anonymous `DocIndexIterator` of `TestSimpleFloatVectorValues.iterator()`.
struct SimpleDocIndexIterator {
    floats_len: i32,
    deleted_vectors: Option<Arc<HashSet<i32>>>,
    ord: i32,
    doc: i32,
}

impl DocIdSetIterator for SimpleDocIndexIterator {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        while self.doc < self.floats_len - 1 {
            self.doc += 1;
            let deleted = self
                .deleted_vectors
                .as_ref()
                .is_some_and(|d| d.contains(&self.doc));
            if !deleted {
                self.ord += 1;
                return Ok(self.doc);
            }
        }
        self.doc = NO_MORE_DOCS;
        Ok(self.doc)
    }

    fn advance(&mut self, _target: i32) -> Result<i32> {
        randomizedtesting::jdk::exception::Throwable::new(
            "java.lang.UnsupportedOperationException",
            String::new(),
        )
        .throw()
    }

    fn cost(&self) -> i64 {
        // `floats.length - deletedVectors.size()`: a `NullPointerException`
        // when there are no deletions.
        match &self.deleted_vectors {
            Some(d) => (self.floats_len - d.len() as i32) as i64,
            None => randomizedtesting::jdk::exception::Throwable::new(
                "java.lang.NullPointerException",
                "deletedVectors".to_string(),
            )
            .throw(),
        }
    }
}

impl DocIndexIterator for SimpleDocIndexIterator {
    fn index(&self) -> i32 {
        self.ord
    }
}
