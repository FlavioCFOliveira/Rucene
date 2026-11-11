//! Port of `lucene/core/src/test/org/apache/lucene/util/TestVectorUtil.java`.
//!
//! Java's `byte[]` vectors are Rucene's `&[u8]`: each element keeps Java's
//! bit pattern (`(byte) -10` is `0xF6`); the signed helpers below read it back
//! as `i8`. `BaseVectorizationTestCase.defaultProvider()` is `new
//! DefaultVectorizationProvider()` and `maybePanamaProvider()` is
//! `VectorizationProvider.lookup(true)`, inlined here.

use std::sync::{Arc, LazyLock};

use randomizedtesting::generators::RandomNumbers;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::VectorSimilarityFunction;
use rucene::internal::vectorization::default_vectorization_provider::DefaultVectorizationProvider;
use rucene::internal::vectorization::vectorization_provider::{self, VectorizationProvider};
use rucene::search::scorer_util::DocAndScoreAccBuffer;
use rucene::util::vector_util as VectorUtil;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::test_util::TestUtil;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test};

/// `TestVectorUtil`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestVectorUtil",
        vec![
            test("testBasicDotProduct", test_basic_dot_product),
            test("testSelfDotProduct", test_self_dot_product),
            test("testOrthogonalDotProduct", test_orthogonal_dot_product),
            test(
                "testDotProductThrowsForDimensionMismatch",
                test_dot_product_throws_for_dimension_mismatch,
            ),
            test("testSelfSquareDistance", test_self_square_distance),
            test("testBasicSquareDistance", test_basic_square_distance),
            test(
                "testSquareDistanceThrowsForDimensionMismatch",
                test_square_distance_throws_for_dimension_mismatch,
            ),
            test("testRandomSquareDistance", test_random_square_distance),
            test("testBasicCosine", test_basic_cosine),
            test("testSelfCosine", test_self_cosine),
            test("testOrthogonalCosine", test_orthogonal_cosine),
            test(
                "testCosineThrowsForDimensionMismatch",
                test_cosine_throws_for_dimension_mismatch,
            ),
            test("testNormalize", test_normalize),
            test("testNormalizeZeroThrows", test_normalize_zero_throws),
            test(
                "testNormalizeToUnitInterval",
                test_normalize_to_unit_interval,
            ),
            test("testExtremeNumerics", test_extreme_numerics),
            test("testBasicDotProductBytes", test_basic_dot_product_bytes),
            test("testSelfDotProductBytes", test_self_dot_product_bytes),
            test(
                "testOrthogonalDotProductBytes",
                test_orthogonal_dot_product_bytes,
            ),
            test(
                "testSelfSquareDistanceBytes",
                test_self_square_distance_bytes,
            ),
            test(
                "testBasicSquareDistanceBytes",
                test_basic_square_distance_bytes,
            ),
            test(
                "testRandomSquareDistanceBytes",
                test_random_square_distance_bytes,
            ),
            test("testBasicDotProductUint8", test_basic_dot_product_uint8),
            test("testSelfDotProductUint8", test_self_dot_product_uint8),
            test(
                "testSelfSquareDistanceUint8",
                test_self_square_distance_uint8,
            ),
            test(
                "testBasicSquareDistanceUint8",
                test_basic_square_distance_uint8,
            ),
            test("testBasicCosineBytes", test_basic_cosine_bytes),
            test("testSelfCosineBytes", test_self_cosine_bytes),
            test("testOrthogonalCosineBytes", test_orthogonal_cosine_bytes),
            test("testBasicXorBitCount", test_basic_xor_bit_count),
            test("testXorBitCount", test_xor_bit_count),
            test("testFindNextGEQ", test_find_next_geq),
            test("testFilterByScore", test_filter_by_score),
            test(
                "testInt4BitDotProductInvariants",
                test_int4_bit_dot_product_invariants,
            ),
            test(
                "testBasicInt4BitDotProduct",
                test_basic_int4_bit_dot_product,
            ),
            test("testInt4BitDotProduct", test_int4_bit_dot_product),
        ],
    )
}

pub const DELTA: f64 = 1e-4;

/// `assertEquals(double expected, double actual, double delta)`.
fn assert_delta(expected: f64, actual: f64, delta: f64) {
    Assert::assert_equals_double_msg(None, expected, actual, delta)
}

/// Java `byte[]` literal: each `int` narrowed to its byte.
fn b(v: &[i32]) -> Vec<u8> {
    v.iter().map(|&x| x as i8 as u8).collect()
}

fn test_basic_dot_product() -> Result<()> {
    assert_delta(
        5.0,
        VectorUtil::dot_product_f32(&[1.0, 2.0, 3.0], &[-10.0, 0.0, 5.0])? as f64,
        0.0,
    );
    Ok(())
}

fn test_self_dot_product() -> Result<()> {
    // the dot product of a vector with itself is equal to the sum of the
    // squares of its components
    let v = random_vector();
    assert_delta(
        l2(&v) as f64,
        VectorUtil::dot_product_f32(&v, &v)? as f64,
        DELTA,
    );
    Ok(())
}

fn test_orthogonal_dot_product() -> Result<()> {
    // the dot product of two perpendicular vectors is 0
    let mut v = [0f32; 2];
    v[0] = random().next_int_bound(100) as f32;
    v[1] = random().next_int_bound(100) as f32;
    let u = [v[1], -v[0]];
    assert_delta(0.0, VectorUtil::dot_product_f32(&u, &v)? as f64, DELTA);
    Ok(())
}

fn test_dot_product_throws_for_dimension_mismatch() -> Result<()> {
    let v = [1f32, 0.0, 0.0];
    let u = [0f32, 1.0];
    expect_throws("java.lang.IllegalArgumentException", || {
        VectorUtil::dot_product_f32(&u, &v)
    });
    Ok(())
}

fn test_self_square_distance() -> Result<()> {
    // the l2 distance of a vector with itself is zero
    let v = random_vector();
    assert_delta(0.0, VectorUtil::square_distance_f32(&v, &v)? as f64, DELTA);
    Ok(())
}

fn test_basic_square_distance() -> Result<()> {
    assert_delta(
        12.0,
        VectorUtil::square_distance_f32(&[1.0, 2.0, 3.0], &[-1.0, 0.0, 5.0])? as f64,
        0.0,
    );
    Ok(())
}

fn test_square_distance_throws_for_dimension_mismatch() -> Result<()> {
    let v = [1f32, 0.0, 0.0];
    let u = [0f32, 1.0];
    expect_throws("java.lang.IllegalArgumentException", || {
        VectorUtil::square_distance_f32(&u, &v)
    });
    Ok(())
}

fn test_random_square_distance() -> Result<()> {
    // the square distance of a vector with its inverse is equal to four times
    // the sum of squares of its components
    let v = random_vector();
    let u = negative(&v);
    assert_delta(
        (4.0 * l2(&v)) as f64,
        VectorUtil::square_distance_f32(&u, &v)? as f64,
        DELTA,
    );
    Ok(())
}

fn test_basic_cosine() -> Result<()> {
    assert_delta(
        0.11952f32 as f64,
        VectorUtil::cosine_f32(&[1.0, 2.0, 3.0], &[-10.0, 0.0, 5.0])? as f64,
        DELTA,
    );
    Ok(())
}

fn test_self_cosine() -> Result<()> {
    // the dot product of a vector with itself is always equal to 1
    let mut v = random_vector();
    // ensure the vector is non-zero so that cosine is defined
    v[0] = random().next_float() + 0.01;
    assert_delta(1.0, VectorUtil::cosine_f32(&v, &v)? as f64, DELTA);
    Ok(())
}

fn test_orthogonal_cosine() -> Result<()> {
    // the cosine of two perpendicular vectors is 0
    let mut v = [0f32; 2];
    v[0] = random().next_int_bound(100) as f32;
    // ensure the vector is non-zero so that cosine is defined
    v[1] = random().next_int_range(1, 100) as f32;
    let u = [v[1], -v[0]];
    assert_delta(0.0, VectorUtil::cosine_f32(&u, &v)? as f64, DELTA);
    Ok(())
}

fn test_cosine_throws_for_dimension_mismatch() -> Result<()> {
    let v = [1f32, 0.0, 0.0];
    let u = [0f32, 1.0];
    expect_throws("java.lang.IllegalArgumentException", || {
        VectorUtil::cosine_f32(&u, &v)
    });
    Ok(())
}

fn test_normalize() -> Result<()> {
    let mut v = random_vector();
    let i = random().next_int_bound(v.len() as i32) as usize;
    v[i] = 1.0; // ensure vector is not all zeroes
                // `VectorUtil.l2normalize(float[])`: `l2normalize(v, true)`.
    VectorUtil::l2normalize(&mut v, true)?;
    assert_delta(1.0, l2(&v) as f64, DELTA);
    Ok(())
}

fn test_normalize_zero_throws() -> Result<()> {
    let mut v = [0f32, 0.0, 0.0];
    expect_throws("java.lang.IllegalArgumentException", || {
        VectorUtil::l2normalize(&mut v, true)
    });
    Ok(())
}

fn test_normalize_to_unit_interval() -> Result<()> {
    for _ in 0..100 {
        let f = random().next_float() * 2.0 - 1.0;
        let v = VectorUtil::normalize_to_unit_interval(f);
        Assert::assert_true(v >= 0.0);
        Assert::assert_true(v <= 1.0);
        Assert::assert_equals_float_msg(None, ((1.0 + f) / 2.0).max(0.0), v, 0.0);
    }
    Ok(())
}

fn test_extreme_numerics() -> Result<()> {
    let v1 = vec![0.888888f32; 1536];
    let v2 = vec![-0.777777f32; 1536];
    for vector_similarity_function in [
        VectorSimilarityFunction::EUCLIDEAN,
        VectorSimilarityFunction::DOT_PRODUCT,
        VectorSimilarityFunction::COSINE,
        VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT,
    ] {
        let v = vector_similarity_function.compare_f32(&v1, &v2)?;
        Assert::assert_true_msg(
            &format!("{vector_similarity_function:?} expected >=0 got:{v}"),
            v >= 0.0,
        );
    }
    Ok(())
}

fn l2(v: &[f32]) -> f32 {
    let mut l2 = 0f32;
    for &x in v {
        l2 += x * x;
    }
    l2
}

fn negative(v: &[f32]) -> Vec<f32> {
    v.iter().map(|&x| -x).collect()
}

/// `negative(byte[])`.
fn negative_bytes(v: &[u8]) -> Vec<u8> {
    v.iter().map(|&x| (x as i8).wrapping_neg() as u8).collect()
}

/// `l2(byte[])`.
fn l2_bytes(v: &[u8]) -> f32 {
    let mut l2 = 0f32;
    for &x in v {
        let x = x as i8 as i32;
        l2 += (x * x) as f32;
    }
    l2
}

fn uint8_l2(v: &[u8]) -> f32 {
    let mut l2 = 0f32;
    for &x in v {
        l2 += (x as i32 * x as i32) as f32;
    }
    l2
}

fn random_vector() -> Vec<f32> {
    random_vector_dim(random().next_int_bound(100) + 1)
}

/// `randomVector(int dim)`.
pub fn random_vector_dim(dim: i32) -> Vec<f32> {
    let random = random();
    (0..dim).map(|_| random.next_float()).collect()
}

fn random_vector_bytes() -> Vec<u8> {
    let len = TestUtil::next_int(&*random(), 1, 100);
    let mut v = TestUtil::random_binary_term_with(&*random(), len);
    for i in v.offset..v.offset + v.length {
        if v.bytes[i] as i8 == -128 {
            v.bytes[i] = (-127i8) as u8;
        }
    }
    Assert::assert_true(v.offset == 0);
    v.bytes
}

/// `randomVectorBytes(int dim)` (public; used by other Lucene tests).
#[allow(dead_code)]
pub fn random_vector_bytes_dim(dim: i32) -> Vec<u8> {
    let mut v = TestUtil::random_binary_term_with(&*random(), dim);
    for i in v.offset..v.offset + v.length {
        if v.bytes[i] as i8 == -128 {
            v.bytes[i] = (-127i8) as u8;
        }
    }
    v.bytes
}

fn test_basic_dot_product_bytes() -> Result<()> {
    let a = b(&[1, 2, 3]);
    let bb = b(&[-10, 0, 5]);
    assert_delta(5.0, VectorUtil::dot_product_bytes(&a, &bb)? as f64, 0.0);
    let denom = (a.len() * (1 << 15)) as f32;
    assert_delta(
        (0.5 + 5.0 / denom) as f64,
        VectorUtil::dot_product_score(&a, &bb)? as f64,
        DELTA,
    );

    // dot product 0 maps to 0.5
    let zero = b(&[0, 0, 0]);
    assert_delta(0.5, VectorUtil::dot_product_score(&a, &zero)? as f64, DELTA);

    // minimum dot product
    let min = b(&[-128, -128]);
    let max = b(&[127, 127]);
    // minimum dot product (-128 * 127 * 2) scales to 0.0039, not 0, due to
    // asymmetry in the range of byte values
    assert_delta(
        0.0039,
        VectorUtil::dot_product_score(&min, &max)? as f64,
        DELTA,
    );

    // maximum dot product
    assert_delta(
        1.0,
        VectorUtil::dot_product_score(&min, &min)? as f64,
        DELTA,
    );
    Ok(())
}

fn test_self_dot_product_bytes() -> Result<()> {
    // the dot product of a vector with itself is equal to the sum of the
    // squares of its components
    let v = random_vector_bytes();
    assert_delta(
        l2_bytes(&v) as f64,
        VectorUtil::dot_product_bytes(&v, &v)? as f64,
        DELTA,
    );
    Ok(())
}

fn test_orthogonal_dot_product_bytes() -> Result<()> {
    // the dot product of two perpendicular vectors is 0
    let mut a = [0u8; 2];
    a[0] = random().next_int_bound(100) as u8;
    a[1] = random().next_int_bound(100) as u8;
    let bb = [a[1], (a[0] as i8).wrapping_neg() as u8];
    assert_delta(0.0, VectorUtil::dot_product_bytes(&a, &bb)? as f64, DELTA);
    Ok(())
}

fn test_self_square_distance_bytes() -> Result<()> {
    // the l2 distance of a vector with itself is zero
    let v = random_vector_bytes();
    assert_delta(
        0.0,
        VectorUtil::square_distance_bytes(&v, &v)? as f64,
        DELTA,
    );
    Ok(())
}

fn test_basic_square_distance_bytes() -> Result<()> {
    assert_delta(
        12.0,
        VectorUtil::square_distance_bytes(&b(&[1, 2, 3]), &b(&[-1, 0, 5]))? as f64,
        0.0,
    );
    Ok(())
}

fn test_random_square_distance_bytes() -> Result<()> {
    // the square distance of a vector with its inverse is equal to four times
    // the sum of squares of its components
    let v = random_vector_bytes();
    let u = negative_bytes(&v);
    assert_delta(
        (4.0 * l2_bytes(&v)) as f64,
        VectorUtil::square_distance_bytes(&u, &v)? as f64,
        DELTA,
    );
    Ok(())
}

fn test_basic_dot_product_uint8() -> Result<()> {
    let a = b(&[1, 2, 3]);
    let bb = b(&[-10, 0, 5]);
    assert_delta(261.0, VectorUtil::uint8_dot_product(&a, &bb)? as f64, 0.0);
    let min = b(&[-128, -128]);
    let max = b(&[127, 127]);
    assert_delta(
        32512.0,
        VectorUtil::uint8_dot_product(&min, &max)? as f64,
        DELTA,
    );
    Ok(())
}

fn test_self_dot_product_uint8() -> Result<()> {
    // the dot product of a vector with itself is equal to the sum of the
    // squares of its components
    let v = random_vector_bytes();
    assert_delta(
        uint8_l2(&v) as f64,
        VectorUtil::uint8_dot_product(&v, &v)? as f64,
        DELTA,
    );
    Ok(())
}

fn test_self_square_distance_uint8() -> Result<()> {
    // the l2 distance of a vector with itself is zero
    let v = random_vector_bytes();
    assert_delta(
        0.0,
        VectorUtil::uint8_square_distance(&v, &v)? as f64,
        DELTA,
    );
    Ok(())
}

fn test_basic_square_distance_uint8() -> Result<()> {
    assert_delta(
        64524.0,
        VectorUtil::uint8_square_distance(&b(&[1, 2, 3]), &b(&[-1, 0, 5]))? as f64,
        0.0,
    );
    Ok(())
}

fn test_basic_cosine_bytes() -> Result<()> {
    assert_delta(
        0.11952f32 as f64,
        VectorUtil::cosine_bytes(&b(&[1, 2, 3]), &b(&[-10, 0, 5]))? as f64,
        DELTA,
    );
    Ok(())
}

fn test_self_cosine_bytes() -> Result<()> {
    // the dot product of a vector with itself is always equal to 1
    let mut v = random_vector_bytes();
    // ensure the vector is non-zero so that cosine is defined
    v[0] = (random().next_int_bound(126) + 1) as u8;
    assert_delta(1.0, VectorUtil::cosine_bytes(&v, &v)? as f64, DELTA);
    Ok(())
}

fn test_orthogonal_cosine_bytes() -> Result<()> {
    // the cosine of two perpendicular vectors is 0
    let mut v = [0f32; 2];
    v[0] = random().next_int_bound(100) as f32;
    // ensure the vector is non-zero so that cosine is defined
    v[1] = random().next_int_range(1, 100) as f32;
    let u = [v[1], -v[0]];
    assert_delta(0.0, VectorUtil::cosine_f32(&u, &v)? as f64, DELTA);
    Ok(())
}

/// `interface ToIntBiFunction { int apply(byte[] a, byte[] b); }`.
type ToIntBiFunction = fn(&[u8], &[u8]) -> i32;

fn test_basic_xor_bit_count() -> Result<()> {
    test_basic_xor_bit_count_impl(|a, b| match VectorUtil::xor_bit_count(a, b) {
        Ok(v) => v,
        Err(e) => rucene_test_framework::util::java_exceptions::rethrow_java(e),
    });
    test_basic_xor_bit_count_impl(VectorUtil::xor_bit_count_int);
    test_basic_xor_bit_count_impl(VectorUtil::xor_bit_count_long);
    // test sanity
    test_basic_xor_bit_count_impl(xor_bit_count);
    Ok(())
}

/// `testBasicXorBitCountImpl(ToIntBiFunction xorBitCount)`.
fn test_basic_xor_bit_count_impl(xor_bit_count: ToIntBiFunction) {
    let cases: &[(i32, &[i32], &[i32])] = &[
        (0, &[1], &[1]),
        (0, &[1, 2, 3], &[1, 2, 3]),
        (1, &[1, 2, 3], &[0, 2, 3]),
        (2, &[1, 2, 3], &[0, 6, 3]),
        (3, &[1, 2, 3], &[0, 6, 7]),
        (4, &[1, 2, 3], &[2, 6, 7]),
        // 32-bit / int boundary
        (0, &[1, 2, 3, 4], &[1, 2, 3, 4]),
        (1, &[1, 2, 3, 4], &[0, 2, 3, 4]),
        (0, &[1, 2, 3, 4, 5], &[1, 2, 3, 4, 5]),
        (1, &[1, 2, 3, 4, 5], &[0, 2, 3, 4, 5]),
        // 64-bit / long boundary
        (0, &[1, 2, 3, 4, 5, 6, 7, 8], &[1, 2, 3, 4, 5, 6, 7, 8]),
        (1, &[1, 2, 3, 4, 5, 6, 7, 8], &[0, 2, 3, 4, 5, 6, 7, 8]),
        (
            0,
            &[1, 2, 3, 4, 5, 6, 7, 8, 9],
            &[1, 2, 3, 4, 5, 6, 7, 8, 9],
        ),
        (
            1,
            &[1, 2, 3, 4, 5, 6, 7, 8, 9],
            &[0, 2, 3, 4, 5, 6, 7, 8, 9],
        ),
    ];
    for (expected, a, bb) in cases {
        assert_equals(expected, &xor_bit_count(&b(a), &b(bb)));
    }
}

fn test_xor_bit_count() -> Result<()> {
    let iterations = LuceneTestCase::at_least(100);
    for _ in 0..iterations {
        let size = random().next_int_bound(1024) as usize;
        let mut a = vec![0u8; size];
        let mut bb = vec![0u8; size];
        random().next_bytes(&mut a);
        random().next_bytes(&mut bb);

        let expected = xor_bit_count(&a, &bb);
        assert_equals(&expected, &VectorUtil::xor_bit_count(&a, &bb)?);
        assert_equals(&expected, &VectorUtil::xor_bit_count_int(&a, &bb));
        assert_equals(&expected, &VectorUtil::xor_bit_count_long(&a, &bb));
    }
    Ok(())
}

fn xor_bit_count(a: &[u8], b: &[u8]) -> i32 {
    let mut res = 0;
    for i in 0..a.len() {
        let mut x = a[i];
        let mut y = b[i];
        for _ in 0..8 {
            if x == y {
                break;
            }
            if (x & 0x01) != (y & 0x01) {
                res += 1;
            }
            x >>= 1;
            y >>= 1;
        }
    }
    res
}

fn test_find_next_geq() -> Result<()> {
    let padding = TestUtil::next_int(&*random(), 0, 5) as usize;
    let mut values = vec![0i32; 128 + padding];
    let mut v = 0;
    for i in 0..128 {
        v += TestUtil::next_int(&*random(), 1, 1000);
        values[i] = v;
    }

    for _ in 0..1_000 {
        let from = TestUtil::next_int(&*random(), 0, 127) as usize;
        let target = TestUtil::next_int(&*random(), values[from], values[from].max(values[127]))
            + random().next_int_bound(10)
            - 5;
        assert_equals(
            &slow_find_next_geq(&values, 128, target, from),
            &VectorUtil::find_next_geq(&values, target, from, 128),
        );
    }
    Ok(())
}

fn slow_find_next_geq(buffer: &[i32], length: usize, target: i32, from: usize) -> usize {
    for i in from..length {
        if buffer[i] >= target {
            return i;
        }
    }
    length
}

fn test_filter_by_score() -> Result<()> {
    for _ in 0..1_000 {
        let padding = TestUtil::next_int(&*random(), 0, 5) as usize;
        let mut b1 = DocAndScoreAccBuffer::new();
        let mut b2 = DocAndScoreAccBuffer::new();
        b1.grow_no_copy(128 + padding);
        b2.grow_no_copy(128 + padding);
        let mut doc = 0;
        for i in 0..128 + padding {
            doc += TestUtil::next_int(&*random(), 1, 1000);
            b1.docs[i] = doc;
            b2.docs[i] = doc;
            let s = random().next_double();
            b1.scores[i] = s;
            b2.scores[i] = s;
        }

        let min_score_inclusive = random().next_double();
        let up_to = TestUtil::next_int(&*random(), 0, 127) as usize;
        b1.size = slow_filter_by_score(&mut b1.docs, &mut b1.scores, min_score_inclusive, up_to);
        b2.size =
            VectorUtil::filter_by_score(&mut b2.docs, &mut b2.scores, min_score_inclusive, up_to)?;

        assert_equals(&b1.size, &b2.size);
        Assert::assert_true(b1.docs[..b1.size] == b2.docs[..b2.size]);
        Assert::assert_true(b1.scores[..b1.size] == b2.scores[..b2.size]);
    }
    Ok(())
}

fn slow_filter_by_score(
    doc_buffer: &mut [i32],
    score_buffer: &mut [f64],
    min_score_inclusive: f64,
    up_to: usize,
) -> usize {
    let mut new_size = 0;
    for i in 0..up_to {
        if score_buffer[i] >= min_score_inclusive {
            doc_buffer[new_size] = doc_buffer[i];
            score_buffer[new_size] = score_buffer[i];
            new_size += 1;
        }
    }
    new_size
}

fn test_int4_bit_dot_product_invariants() -> Result<()> {
    let iterations = LuceneTestCase::at_least(10);
    for _ in 0..iterations {
        let size = RandomNumbers::random_int_between(&*random(), 1, 10) as usize;
        let d = vec![0u8; size];
        let q = vec![0u8; size * 4 - 1];
        expect_throws("java.lang.IllegalArgumentException", || {
            VectorUtil::int4_bit_dot_product(&q, &d)
        });
    }
    Ok(())
}

/// `static final VectorizationProvider defaultedProvider`.
static DEFAULTED_PROVIDER: LazyLock<Arc<dyn VectorizationProvider>> =
    LazyLock::new(|| Arc::new(DefaultVectorizationProvider::new()));

/// `static final VectorizationProvider defOrPanamaProvider`.
static DEF_OR_PANAMA_PROVIDER: LazyLock<Arc<dyn VectorizationProvider>> =
    LazyLock::new(|| vectorization_provider::lookup(true));

/// `interface Int4BitDotProduct { long apply(byte[] q, byte[] d); }`.
type Int4BitDotProduct<'a> = &'a dyn Fn(&[u8], &[u8]) -> Result<i64>;

fn test_basic_int4_bit_dot_product() -> Result<()> {
    test_basic_int4_bit_dot_product_impl(&VectorUtil::int4_bit_dot_product)?;
    test_basic_int4_bit_dot_product_impl(&|q, d| {
        DEFAULTED_PROVIDER
            .get_vector_util_support()
            .int4_bit_dot_product(q, d)
    })?;
    test_basic_int4_bit_dot_product_impl(&|q, d| {
        DEF_OR_PANAMA_PROVIDER
            .get_vector_util_support()
            .int4_bit_dot_product(q, d)
    })
}

/// `testBasicInt4BitDotProductImpl(Int4BitDotProduct)`.
fn test_basic_int4_bit_dot_product_impl(f: Int4BitDotProduct<'_>) -> Result<()> {
    assert_equals(&15i64, &f(&b(&[1, 1, 1, 1]), &b(&[1]))?);
    assert_equals(&30i64, &f(&b(&[1, 2, 1, 2, 1, 2, 1, 2]), &b(&[1, 2]))?);

    let cases: &[(i64, &[i32])] = &[
        (60, &[1, 2, 3]),                    // 4 + 8 + 16 + 32
        (75, &[1, 2, 3, 4]),                 // 5 + 10 + 20 + 40
        (105, &[1, 2, 3, 4, 5]),             // 7 + 14 + 28 + 56
        (135, &[1, 2, 3, 4, 5, 6]),          // 9 + 18 + 36 + 72
        (180, &[1, 2, 3, 4, 5, 6, 7]),       // 12 + 24 + 48 + 96
        (195, &[1, 2, 3, 4, 5, 6, 7, 8]),    // 13 + 26 + 52 + 104
        (225, &[1, 2, 3, 4, 5, 6, 7, 8, 9]), // 15 + 30 + 60 + 120
    ];
    for (expected, dv) in cases {
        let d = b(dv);
        // q is d repeated four times
        let q: Vec<u8> = d.iter().cycle().take(d.len() * 4).copied().collect();
        // Java `assert`.
        Assert::assert_true(scalar_int4_bit_dot_product(&q, &d) as i64 == *expected);
        assert_equals(expected, &f(&q, &d)?);
    }
    Ok(())
}

fn test_int4_bit_dot_product() -> Result<()> {
    test_int4_bit_dot_product_impl(&VectorUtil::int4_bit_dot_product)?;
    test_int4_bit_dot_product_impl(&|q, d| {
        DEFAULTED_PROVIDER
            .get_vector_util_support()
            .int4_bit_dot_product(q, d)
    })?;
    test_int4_bit_dot_product_impl(&|q, d| {
        DEF_OR_PANAMA_PROVIDER
            .get_vector_util_support()
            .int4_bit_dot_product(q, d)
    })
}

/// `testInt4BitDotProductImpl(Int4BitDotProduct)`.
fn test_int4_bit_dot_product_impl(f: Int4BitDotProduct<'_>) -> Result<()> {
    let iterations = LuceneTestCase::at_least(50);
    for _ in 0..iterations {
        let size = random().next_int_bound(5000) as usize;
        let mut d = vec![0u8; size];
        let mut q = vec![0u8; size * 4];
        random().next_bytes(&mut d);
        random().next_bytes(&mut q);
        assert_equals(&(scalar_int4_bit_dot_product(&q, &d) as i64), &f(&q, &d)?);

        d.fill(i8::MAX as u8);
        q.fill(i8::MAX as u8);
        assert_equals(&(scalar_int4_bit_dot_product(&q, &d) as i64), &f(&q, &d)?);

        d.fill(i8::MIN as u8);
        q.fill(i8::MIN as u8);
        assert_equals(&(scalar_int4_bit_dot_product(&q, &d) as i64), &f(&q, &d)?);
    }
    Ok(())
}

/// `scalarInt4BitDotProduct(byte[] q, byte[] d)`.
pub fn scalar_int4_bit_dot_product(q: &[u8], d: &[u8]) -> i32 {
    let mut res = 0;
    for i in 0..4 {
        res += popcount(q, i * d.len(), d, d.len()) << i;
    }
    res
}

/// `popcount(byte[] a, int aOffset, byte[] b, int length)`.
pub fn popcount(a: &[u8], a_offset: usize, b: &[u8], length: usize) -> i32 {
    let mut res = 0;
    for j in 0..length {
        let value = (a[a_offset + j] & b[j]) as i32;
        for k in 0..8 {
            if (value & (1 << k)) != 0 {
                res += 1;
            }
        }
    }
    res
}
