//! Port of the `lucene/core/src/test/org/apache/lucene/internal/vectorization/`
//! test classes of Lucene 10.5.0, each run as a suite by the ported runner:
//! `BaseVectorizationTestCase`, `TestVectorizationProvider`,
//! `TestPostingDecodingUtil`, `TestVectorUtilSupport` and `TestVectorScorer`.

mod lucene_core_shims;
mod suite_seeds;

use std::sync::Arc;

use lucene_core_shims::{assert_bytes_equals, lucene_class, run, t, ti};
use randomizedtesting::annotations::ParametersFactory;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::{modifier, Class, Constructor, Method, Value};
use randomizedtesting::junit::{Assert, BeforeClass};
use rucene::codecs::lucene104::for_util::BLOCK_SIZE;
use rucene::error::Result;
use rucene::internal::vectorization::posting_decoding_util::PostingDecodingUtil;
use rucene::internal::vectorization::vector_util_support::VectorUtilSupport;
use rucene::internal::vectorization::vectorization_provider::{self, VectorizationProvider};
use rucene::internal::vectorization::DefaultVectorizationProvider;
use rucene::store::mmap::MMapDirectory;
use rucene::store::{Directory, DEFAULT_IO_CONTEXT};
use rucene_test_framework::index::base_index_file_format_test_case::assert_array_equals;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

fn random() -> Arc<dyn Random> {
    LuceneTestCase::random()
}

// -----------------------------------------------------------------------------
// BaseVectorizationTestCase
// -----------------------------------------------------------------------------

/// `LUCENE_PROVIDER` (`protected static final`): `defaultProvider()`.
fn lucene_provider() -> Arc<dyn VectorizationProvider> {
    static P: std::sync::LazyLock<Arc<dyn VectorizationProvider>> =
        std::sync::LazyLock::new(default_provider);
    Arc::clone(&P)
}

/// `PANAMA_PROVIDER` (`protected static final`): `maybePanamaProvider()`.
fn panama_provider() -> Arc<dyn VectorizationProvider> {
    static P: std::sync::LazyLock<Arc<dyn VectorizationProvider>> =
        std::sync::LazyLock::new(maybe_panama_provider);
    Arc::clone(&P)
}

/// `defaultProvider()` (`public static`).
fn default_provider() -> Arc<dyn VectorizationProvider> {
    Arc::new(DefaultVectorizationProvider::new())
}

/// `maybePanamaProvider()` (`public static`).
fn maybe_panama_provider() -> Arc<dyn VectorizationProvider> {
    vectorization_provider::lookup(true)
}

/// `BaseVectorizationTestCase.beforeClass()` (`@BeforeClass`).
///
/// Java compares the two providers' runtime classes
/// (`PANAMA_PROVIDER.getClass() != LUCENE_PROVIDER.getClass()`); the
/// providers' `Debug` names stand for the classes.
fn base_vectorization_before_class() {
    let panama = format!("{:?}", panama_provider());
    let lucene = format!("{:?}", lucene_provider());
    let class_of = |s: &str| s.split([' ', '{', '(']).next().unwrap_or("").to_string();
    LuceneTestCase::assume_true(
        "Test only works when JDK's vector incubator module is enabled.",
        class_of(&panama) != class_of(&lucene),
    );
}

/// The `BaseVectorizationTestCase` class, with its `@BeforeClass`.
fn base_vectorization_test_case() -> Arc<Class> {
    static CLASS: std::sync::LazyLock<Arc<Class>> = std::sync::LazyLock::new(|| {
        Class::builder("org.apache.lucene.internal.vectorization.BaseVectorizationTestCase")
            .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
            .extends(&LuceneTestCase::class())
            .method(
                Method::new("beforeClass")
                    .annotate(BeforeClass)
                    .throws(&["java.lang.Exception"])
                    .static_body(base_vectorization_before_class),
            )
            .build()
    });
    Arc::clone(&CLASS)
}

// -----------------------------------------------------------------------------
// TestVectorizationProvider
// -----------------------------------------------------------------------------

/// `TestVectorizationProvider.testCallerOfGetter()`.
fn vectorization_provider_test_caller_of_getter() -> Result<()> {
    LuceneTestCase::expect_throws("java.lang.IllegalCallerException", || {
        illegal_caller();
        Ok(())
    });
    Ok(())
}

/// `illegalCaller()` (`private static`).
fn illegal_caller() {
    vectorization_provider::get_instance();
}

fn vectorization_provider_class() -> Arc<Class> {
    lucene_class("org.apache.lucene.internal.vectorization.TestVectorizationProvider")
        .method(t(
            "testCallerOfGetter",
            vectorization_provider_test_caller_of_getter,
        ))
        .build()
}

// -----------------------------------------------------------------------------
// TestPostingDecodingUtil
// -----------------------------------------------------------------------------

/// `TestPostingDecodingUtil.testDuelSplitInts()`.
///
/// **Divergence from Lucene 10.5.0.** Java builds both utilities over one
/// `slice` and seeks that shared slice between the two calls. Rucene's
/// `PostingDecodingUtil` owns its input, so each utility gets its own slice
/// of the same range, and the seeks and file pointers are taken on the input
/// each utility owns.
fn posting_decoding_util_test_duel_split_ints() -> Result<()> {
    let iterations = LuceneTestCase::at_least(100);
    let mut dir = MMapDirectory::open(LuceneTestCase::create_temp_dir())?;
    let result = (|| -> Result<()> {
        {
            let mut out = dir.create_output("tests.bin", &*DEFAULT_IO_CONTEXT)?;
            out.write_int(random().next_int())?;
            for _ in 0..BLOCK_SIZE {
                out.write_long(random().next_int() as i64)?;
            }
            out.close()?;
        }
        let vectorization_provider = vectorization_provider::lookup(true);
        let mut r#in = dir.open_input("tests.bin", &*DEFAULT_IO_CONTEXT)?;
        let mut expected_b = vec![0i32; BLOCK_SIZE];
        let mut expected_c = vec![0i32; BLOCK_SIZE];
        let mut actual_b = vec![0i32; BLOCK_SIZE];
        let mut actual_c = vec![0i32; BLOCK_SIZE];
        for _ in 0..iterations {
            // Initialize arrays with random content.
            for i in 0..expected_b.len() {
                expected_b[i] = random().next_int();
                actual_b[i] = expected_b[i];
                expected_c[i] = random().next_int();
                actual_c[i] = expected_c[i];
            }
            let b_shift = TestUtil::next_int(&*random(), 1, 31);
            let dec = TestUtil::next_int(&*random(), 1, b_shift);
            let num_iters = (b_shift + dec - 1) / dec;
            let count = TestUtil::next_int(&*random(), 1, 64 / num_iters);
            let b_mask = random().next_int();
            let c_index = random().next_int_bound(64);
            let c_mask = random().next_int();
            let start_fp = random().next_int_bound(4) as i64;

            // Work on a slice that has just the right number of bytes to make the test fail with
            // an index-out-of-bounds in case the implementation reads more than the allowed
            // number of padding bytes.
            let slice_len = start_fp + count as i64 * 8;
            let mut default_util = PostingDecodingUtil::new(r#in.slice("test", 0, slice_len)?);
            let mut optimized_util = vectorization_provider
                .new_posting_decoding_util(r#in.slice("test", 0, slice_len)?)?;

            default_util.input_mut().seek(start_fp)?;
            default_util.split_ints(
                count as usize,
                &mut expected_b,
                b_shift,
                dec,
                b_mask,
                &mut expected_c,
                c_index as usize,
                c_mask,
            )?;
            let expected_end_fp = default_util.input().file_pointer();
            optimized_util.input_mut().seek(start_fp)?;
            optimized_util.split_ints(
                count as usize,
                &mut actual_b,
                b_shift,
                dec,
                b_mask,
                &mut actual_c,
                c_index as usize,
                c_mask,
            )?;
            Assert::assert_equals(expected_end_fp, optimized_util.input().file_pointer());
            assert_array_equals(&expected_b, &actual_b);
            assert_array_equals(&expected_c, &actual_c);
        }
        r#in.close()
    })();
    result.and(dir.close())
}

fn posting_decoding_util_class() -> Arc<Class> {
    lucene_class("org.apache.lucene.internal.vectorization.TestPostingDecodingUtil")
        .method(t(
            "testDuelSplitInts",
            posting_decoding_util_test_duel_split_ints,
        ))
        .build()
}

// -----------------------------------------------------------------------------
// TestVectorUtilSupport
// -----------------------------------------------------------------------------

/// `VECTOR_SIZES` (`private static final int[]`).
const VECTOR_SIZES: [i32; 22] = [
    1, 4, 6, 8, 13, 16, 25, 32, 64, 100, 128, 207, 256, 300, 512, 702, 1024, 1536, 2046, 2048,
    4096, 4098,
];

/// `TestVectorUtilSupport extends BaseVectorizationTestCase`, built by its
/// `@ParametersFactory` with one size.
struct TestVectorUtilSupport {
    size: usize,
    delta: f64,
}

/// `Byte.MIN_VALUE` and `Byte.MAX_VALUE` as the bytes Rucene's `byte[]`
/// slices hold.
const BYTE_MIN: u8 = i8::MIN as u8;
const BYTE_MAX: u8 = i8::MAX as u8;

impl TestVectorUtilSupport {
    /// `TestVectorUtilSupport(int size)`.
    fn new(size: i32) -> TestVectorUtilSupport {
        TestVectorUtilSupport {
            size: size as usize,
            // scale the delta with the size
            delta: 1e-5 * size as f64,
        }
    }

    /// `testFloatVectors()`.
    fn test_float_vectors(&mut self) -> Result<()> {
        let mut a = vec![0f32; self.size];
        let mut b = vec![0f32; self.size];
        for i in 0..self.size {
            a[i] = random().next_float();
            b[i] = random().next_float();
        }
        self.assert_float_returning_providers(|p| p.dot_product_f32(&a, &b))?;
        self.assert_float_returning_providers(|p| p.square_distance_f32(&a, &b))?;
        self.assert_float_returning_providers(|p| p.cosine_f32(&a, &b))
    }

    /// `testBinaryVectors()`.
    fn test_binary_vectors(&mut self) -> Result<()> {
        let mut a = vec![0u8; self.size];
        let mut b = vec![0u8; self.size];
        random().next_bytes(&mut a);
        random().next_bytes(&mut b);
        self.assert_int_returning_providers(|p| p.dot_product_bytes(&a, &b))?;
        self.assert_int_returning_providers(|p| p.square_distance_bytes(&a, &b))?;
        self.assert_float_returning_providers(|p| p.cosine_bytes(&a, &b))
    }

    /// `testBinaryVectorsBoundaries()`.
    fn test_binary_vectors_boundaries(&mut self) -> Result<()> {
        let mut a = vec![0u8; self.size];
        let mut b = vec![0u8; self.size];
        for (fa, fb) in [
            (BYTE_MIN, BYTE_MIN),
            (BYTE_MAX, BYTE_MAX),
            (BYTE_MIN, BYTE_MAX),
            (BYTE_MAX, BYTE_MIN),
        ] {
            a.fill(fa);
            b.fill(fb);
            self.assert_int_returning_providers(|p| p.dot_product_bytes(&a, &b))?;
            self.assert_int_returning_providers(|p| p.square_distance_bytes(&a, &b))?;
            self.assert_float_returning_providers(|p| p.cosine_bytes(&a, &b))?;
        }
        Ok(())
    }

    /// The four `int4DotProduct` duels of `testInt4DotProduct` and its
    /// boundary variants, with the four assertions against `dotProduct`.
    fn int4_dot_product_duels(&self, a: &[u8], b: &[u8]) -> Result<()> {
        self.assert_int_returning_providers(|p| p.int4_dot_product(a, b))?;
        self.assert_int_returning_providers(|p| p.int4_dot_product_single_packed(a, &pack(b)))?;
        self.assert_int_returning_providers(|p| p.int4_dot_product_single_packed(b, &pack(a)))?;
        self.assert_int_returning_providers(|p| {
            p.int4_dot_product_both_packed(&pack(a), &pack(b))
        })?;
        let lucene = lucene_provider();
        let panama = panama_provider();
        let l = lucene.get_vector_util_support();
        let pa = panama.get_vector_util_support();
        Assert::assert_equals(l.dot_product_bytes(a, b)?, pa.int4_dot_product(a, b)?);
        Assert::assert_equals(
            l.dot_product_bytes(a, b)?,
            pa.int4_dot_product_single_packed(a, &pack(b))?,
        );
        Assert::assert_equals(
            l.dot_product_bytes(a, b)?,
            pa.int4_dot_product_single_packed(b, &pack(a))?,
        );
        Assert::assert_equals(
            l.dot_product_bytes(a, b)?,
            pa.int4_dot_product_both_packed(&pack(a), &pack(b))?,
        );
        Ok(())
    }

    /// `testInt4DotProduct()`.
    fn test_int4_dot_product(&mut self) -> Result<()> {
        LuceneTestCase::assume_true("even sizes only", self.size.is_multiple_of(2));
        let mut a = vec![0u8; self.size];
        let mut b = vec![0u8; self.size];
        for i in 0..self.size {
            a[i] = random().next_int_bound(16) as u8;
            b[i] = random().next_int_bound(16) as u8;
        }
        self.int4_dot_product_duels(&a, &b)
    }

    /// `testInt4DotProductBoundaries()`.
    fn test_int4_dot_product_boundaries(&mut self) -> Result<()> {
        LuceneTestCase::assume_true("even sizes only", self.size.is_multiple_of(2));
        let max_value: u8 = 15;
        let mut a = vec![0u8; self.size];
        let mut b = vec![0u8; self.size];
        a.fill(max_value);
        b.fill(max_value);
        self.int4_dot_product_duels(&a, &b)?;

        let min_value: u8 = 0;
        a.fill(min_value);
        b.fill(min_value);
        self.int4_dot_product_duels(&a, &b)
    }

    /// `testInt4SquareDistance()`.
    fn test_int4_square_distance(&mut self) -> Result<()> {
        LuceneTestCase::assume_true("even sizes only", self.size.is_multiple_of(2));
        let mut a = vec![0u8; self.size];
        let mut b = vec![0u8; self.size];
        for i in 0..self.size {
            a[i] = random().next_int_bound(16) as u8;
            b[i] = random().next_int_bound(16) as u8;
        }
        self.assert_int_returning_providers(|p| p.int4_square_distance(&a, &b))?;
        self.assert_int_returning_providers(|p| {
            p.int4_square_distance_single_packed(&a, &pack(&b))
        })?;
        self.assert_int_returning_providers(|p| {
            p.int4_square_distance_single_packed(&b, &pack(&a))
        })?;
        self.assert_int_returning_providers(|p| {
            p.int4_square_distance_both_packed(&pack(&a), &pack(&b))
        })?;
        let lucene = lucene_provider();
        let panama = panama_provider();
        let l = lucene.get_vector_util_support();
        let pa = panama.get_vector_util_support();
        Assert::assert_equals(
            l.square_distance_bytes(&a, &b)?,
            pa.int4_square_distance(&a, &b)?,
        );
        Assert::assert_equals(
            l.square_distance_bytes(&a, &b)?,
            pa.int4_square_distance_single_packed(&a, &pack(&b))?,
        );
        Assert::assert_equals(
            l.square_distance_bytes(&a, &b)?,
            pa.int4_square_distance_single_packed(&b, &pack(&a))?,
        );
        Assert::assert_equals(
            l.square_distance_bytes(&a, &b)?,
            pa.int4_square_distance_both_packed(&pack(&a), &pack(&b))?,
        );
        Ok(())
    }

    /// `testInt4SquareDistanceBoundaries()`: squareDistance is maximized when
    /// the points are farther away.
    fn test_int4_square_distance_boundaries(&mut self) -> Result<()> {
        LuceneTestCase::assume_true("even sizes only", self.size.is_multiple_of(2));
        let max_value: u8 = 15;
        let a = vec![max_value; self.size];
        let min_value: u8 = 0;
        let b = vec![min_value; self.size];
        self.int4_dot_product_duels(&a, &b)
    }

    /// `testInt4BitDotProduct()`.
    fn test_int4_bit_dot_product(&mut self) -> Result<()> {
        let mut binary_quantized = vec![0u8; self.size];
        let mut int4_quantized = vec![0u8; self.size * 4];
        random().next_bytes(&mut binary_quantized);
        random().next_bytes(&mut int4_quantized);
        self.assert_long_returning_providers(|p| {
            p.int4_bit_dot_product(&int4_quantized, &binary_quantized)
        })
    }

    /// `testInt4BitDotProductBoundaries()`.
    fn test_int4_bit_dot_product_boundaries(&mut self) -> Result<()> {
        let mut binary_quantized = vec![0u8; self.size];
        let mut int4_quantized = vec![0u8; self.size * 4];

        binary_quantized.fill(BYTE_MAX);
        int4_quantized.fill(BYTE_MAX);
        self.assert_long_returning_providers(|p| {
            p.int4_bit_dot_product(&int4_quantized, &binary_quantized)
        })?;

        binary_quantized.fill(BYTE_MIN);
        int4_quantized.fill(BYTE_MIN);
        self.assert_long_returning_providers(|p| {
            p.int4_bit_dot_product(&int4_quantized, &binary_quantized)
        })
    }

    /// `testMinMaxScalarQuantize()`.
    fn test_min_max_scalar_quantize(&mut self) -> Result<()> {
        let r = random();
        let min = r.next_float_range(-1.0, 1.0);
        let max = r.next_float_range(min, 1.0);
        let divisor = ((1 << 8) - 1) as f32; // 8 bit quantization here
        let scale = divisor / (max - min);
        let alpha = (max - min) / divisor;
        let mut vector = vec![0f32; self.size];
        for v in vector.iter_mut() {
            *v = (r.next_float() * (max - min)) + min;
        }

        let outputs = std::cell::RefCell::new(Vec::<Vec<u8>>::new());
        let size = self.size;
        self.assert_float_returning_providers(|p| {
            let mut output = vec![0u8; size];
            let result = p.min_max_scalar_quantize(&vector, &mut output, scale, alpha, min, max);
            outputs.borrow_mut().push(output);
            result
        })?;
        let outputs = outputs.into_inner();
        // check the outputs are identical
        for o in 1..outputs.len() {
            assert_bytes_equals(&outputs[0], &outputs[o]);
        }

        // check recalculation too
        let new_max = max * 2.0;
        let new_min = min / 2.0;
        let new_scale = divisor / (new_max - new_min);
        let new_alpha = (new_max - new_min) / divisor;
        self.assert_float_returning_providers(|p| {
            Ok(p.recalculate_scalar_quantization_offset(
                &outputs[0],
                alpha,
                min,
                new_scale,
                new_alpha,
                new_min,
                new_max,
            ))
        })
    }

    /// `assertFloatReturningProviders(ToDoubleFunction<VectorUtilSupport>)`
    /// (`private`).
    fn assert_float_returning_providers(
        &self,
        func: impl Fn(&dyn VectorUtilSupport) -> Result<f32>,
    ) -> Result<()> {
        let lucene_provider_results = func(lucene_provider().get_vector_util_support())? as f64;
        let panama_provider_results = func(panama_provider().get_vector_util_support())? as f64;
        let delta = self
            .delta
            .max(self.delta * lucene_provider_results.max(panama_provider_results));
        Assert::assert_equals_double_msg(
            None,
            lucene_provider_results,
            panama_provider_results,
            delta,
        );
        Ok(())
    }

    /// `assertIntReturningProviders(ToIntFunction<VectorUtilSupport>)`
    /// (`private`).
    fn assert_int_returning_providers(
        &self,
        func: impl Fn(&dyn VectorUtilSupport) -> Result<i32>,
    ) -> Result<()> {
        Assert::assert_equals(
            func(lucene_provider().get_vector_util_support())?,
            func(panama_provider().get_vector_util_support())?,
        );
        Ok(())
    }

    /// `assertLongReturningProviders(ToLongFunction<VectorUtilSupport>)`
    /// (`private`).
    fn assert_long_returning_providers(
        &self,
        func: impl Fn(&dyn VectorUtilSupport) -> Result<i64>,
    ) -> Result<()> {
        Assert::assert_equals(
            func(lucene_provider().get_vector_util_support())?,
            func(panama_provider().get_vector_util_support())?,
        );
        Ok(())
    }
}

/// `pack(byte[] unpacked)` (`static`).
fn pack(unpacked: &[u8]) -> Vec<u8> {
    let len = unpacked.len().div_ceil(2);
    let mut packed = vec![0u8; len];
    for i in 0..len {
        packed[i] = ((unpacked[i] as i8 as i32) << 4 | (unpacked[len + i] as i8 as i32)) as u8;
    }
    packed
}

fn vector_util_support_class() -> Arc<Class> {
    type T = TestVectorUtilSupport;
    Class::builder("org.apache.lucene.internal.vectorization.TestVectorUtilSupport")
        .extends(&base_vectorization_test_case())
        .constructor(Constructor::with_params(
            &[("int", None)],
            |args: &[Value]| {
                let size = match args.first() {
                    Some(Value::Int(i)) => *i,
                    _ => 0,
                };
                TestVectorUtilSupport::new(size)
            },
        ))
        .method(
            Method::new("parametersFactory")
                .modifiers(modifier::PUBLIC | modifier::STATIC)
                .returns("java.lang.Iterable<java.lang.Object[]>")
                .annotate(ParametersFactory::default())
                .static_value(|_| {
                    Value::List(
                        VECTOR_SIZES
                            .iter()
                            .map(|&i| Value::Array(vec![Value::Int(i)]))
                            .collect(),
                    )
                }),
        )
        .method(ti("testFloatVectors", T::test_float_vectors))
        .method(ti("testBinaryVectors", T::test_binary_vectors))
        .method(ti(
            "testBinaryVectorsBoundaries",
            T::test_binary_vectors_boundaries,
        ))
        .method(ti("testInt4DotProduct", T::test_int4_dot_product))
        .method(ti(
            "testInt4DotProductBoundaries",
            T::test_int4_dot_product_boundaries,
        ))
        .method(ti("testInt4SquareDistance", T::test_int4_square_distance))
        .method(ti(
            "testInt4SquareDistanceBoundaries",
            T::test_int4_square_distance_boundaries,
        ))
        .method(ti("testInt4BitDotProduct", T::test_int4_bit_dot_product))
        .method(ti(
            "testInt4BitDotProductBoundaries",
            T::test_int4_bit_dot_product_boundaries,
        ))
        .method(ti(
            "testMinMaxScalarQuantize",
            T::test_min_max_scalar_quantize,
        ))
        .build()
}

// -----------------------------------------------------------------------------
// TestVectorScorer
// -----------------------------------------------------------------------------

mod vector_scorer {
    //! `TestVectorScorer`.
    //!
    //! Java's `vectorValues(...)` answers one `KnnVectorValues` that the test
    //! hands to every scorer call. Rucene's scorers take their values by
    //! value, so each call receives a `copy()` of them, which reads the same
    //! slice.

    use std::sync::{Arc, Mutex};

    use randomizedtesting::generators::RandomNumbers;
    use randomizedtesting::jdk::random::{JdkRandom, Random};
    use randomizedtesting::jdk::reflect::{Class, Method};
    use randomizedtesting::junit::{Assert, BeforeClass};
    use rucene::codecs::hnsw::flat_vectors::{DefaultFlatVectorScorer, FlatVectorsScorer};
    use rucene::codecs::lucene99::flat_vectors_format::{
        new_dense_off_heap_byte_vector_values, new_dense_off_heap_float_vector_values,
    };
    use rucene::error::Result;
    use rucene::index::vector_values::{ByteVectorValues, FloatVectorValues};
    use rucene::index::{VectorEncoding, VectorSimilarityFunction};
    use rucene::internal::vectorization::vectorization_provider;
    use rucene::store::mmap::{MMapDirectory, DEFAULT_MAX_CHUNK_SIZE};
    use rucene::store::{Directory, IndexInput, NativeFSLockFactory, DEFAULT_IO_CONTEXT};
    use rucene::util::hnsw::scorer::{RandomVectorScorer, RandomVectorScorerSupplier};
    use rucene::util::NamedThreadFactory;
    use rucene_test_framework::util::lucene_test_case::Monster;
    use rucene_test_framework::util::LuceneTestCase;

    use crate::lucene_core_shims::{lucene_class, t};

    use VectorSimilarityFunction::{COSINE, DOT_PRODUCT, EUCLIDEAN, MAXIMUM_INNER_PRODUCT};

    const DELTA: f64 = 1e-5;

    /// The similarity functions every test iterates, in Java's order.
    const SIMS: [VectorSimilarityFunction; 4] =
        [COSINE, EUCLIDEAN, DOT_PRODUCT, MAXIMUM_INNER_PRODUCT];

    /// `DEFAULT_SCORER` (`private static final`):
    /// `DefaultFlatVectorScorer.INSTANCE`.
    fn default_scorer() -> Arc<dyn FlatVectorsScorer> {
        static S: std::sync::LazyLock<Arc<dyn FlatVectorsScorer>> =
            std::sync::LazyLock::new(|| Arc::new(DefaultFlatVectorScorer::INSTANCE));
        Arc::clone(&S)
    }

    /// `MEMSEG_SCORER` (`private static final`):
    /// `VectorizationProvider.lookup(true).getLucene99FlatVectorsScorer()`.
    fn memseg_scorer() -> Arc<dyn FlatVectorsScorer> {
        static S: std::sync::LazyLock<Arc<dyn FlatVectorsScorer>> =
            std::sync::LazyLock::new(|| {
                vectorization_provider::lookup(true).get_lucene99_flat_vectors_scorer()
            });
        Arc::clone(&S)
    }

    /// `beforeClass()` (`@BeforeClass`).
    ///
    /// Java compares the two scorers' runtime classes
    /// (`MEMSEG_SCORER.getClass() != DEFAULT_SCORER.getClass()`); the
    /// scorers' `Debug` type names stand for the classes, as in
    /// `BaseVectorizationTestCase.beforeClass()`.
    fn before_class() {
        let class_of = |s: String| s.split([' ', '{', '(']).next().unwrap_or("").to_string();
        LuceneTestCase::assume_true(
            "Test only works when the Memory segment scorer is present.",
            class_of(format!("{:?}", memseg_scorer()))
                != class_of(format!("{:?}", default_scorer())),
        );
    }

    /// `new MMapDirectory(path, maxChunkSize)`: the default lock factory is
    /// `FSLockFactory.getDefault()`, a `NativeFSLockFactory`.
    fn mmap_directory(prefix: &str, max_chunk_size: i64) -> Result<MMapDirectory> {
        MMapDirectory::with_options(
            LuceneTestCase::create_temp_dir_prefix(prefix),
            Box::new(NativeFSLockFactory),
            max_chunk_size as usize,
        )
    }

    fn assert_score(actual: f32, expected: f32) {
        Assert::assert_equals_double_msg(None, actual as f64, expected as f64, DELTA);
    }

    pub fn test_simple_scorer() -> Result<()> {
        test_simple_scorer_impl(*DEFAULT_MAX_CHUNK_SIZE as i64)
    }

    pub fn test_simple_scorer_small_chunk_size() -> Result<()> {
        let max_chunk_size = LuceneTestCase::random().next_long_range(4, 16);
        test_simple_scorer_impl(max_chunk_size)
    }

    pub fn test_simple_scorer_med_chunk_size() -> Result<()> {
        // a chunk size where in some vectors will be copied on-heap, while others remain off-heap
        test_simple_scorer_impl(64)
    }

    /// `testSimpleScorer(long maxChunkSize)`.
    fn test_simple_scorer_impl(max_chunk_size: i64) -> Result<()> {
        let mut dir = mmap_directory("testSimpleScorer", max_chunk_size)?;
        let result = (|| -> Result<()> {
            for dims in [31, 32, 33] {
                // dimensions that, in some scenarios, cross the mmap chunk sizes
                let mut vectors = vec![vec![0u8; dims as usize]; 2];
                let file_name = format!("bar-{dims}");
                {
                    let mut out = dir.create_output(&file_name, &*DEFAULT_IO_CONTEXT)?;
                    for i in 0..dims {
                        vectors[0][i as usize] = i as u8;
                        vectors[1][i as usize] = (dims - i) as u8;
                    }
                    let bytes = concat(&[&vectors[0], &vectors[1]]);
                    out.write_bytes(&bytes, 0, bytes.len())?;
                    out.close()?;
                }
                let mut input = dir.open_input(&file_name, &*DEFAULT_IO_CONTEXT)?;
                for sim in SIMS {
                    let vector_values = vector_values(dims, 2, &*input, sim)?;
                    for ords in [[0, 1], [1, 0]] {
                        let idx0 = ords[0];
                        let idx1 = ords[1];
                        assert_scorers(sim, &*vector_values, idx0, idx1, &vectors[idx0 as usize])?;
                    }
                }
                input.close()?;
            }
            Ok(())
        })();
        result.and(dir.close())
    }

    /// The scorer comparisons each Java test repeats verbatim: the
    /// `getRandomVectorScorerSupplier` and `getRandomVectorScorer` results of
    /// `MEMSEG_SCORER` against those of `DEFAULT_SCORER`.
    fn assert_scorers(
        sim: VectorSimilarityFunction,
        vector_values: &dyn ByteVectorValues,
        idx0: i32,
        idx1: i32,
        query: &[u8],
    ) -> Result<()> {
        // getRandomVectorScorerSupplier
        let scorer1 = default_scorer()
            .get_random_vector_scorer_supplier_byte(sim, vector_values.copy_byte()?)?;
        let mut scorer1idx0 = scorer1.scorer()?;
        scorer1idx0.set_scoring_ordinal(idx0)?;
        let expected = scorer1idx0.score(idx1)?;
        let scorer2 = memseg_scorer()
            .get_random_vector_scorer_supplier_byte(sim, vector_values.copy_byte()?)?;
        scorer1idx0 = scorer2.scorer()?;
        scorer1idx0.set_scoring_ordinal(idx0)?;
        assert_score(scorer1idx0.score(idx1)?, expected);

        // getRandomVectorScorer
        let mut scorer3 = default_scorer().get_random_vector_scorer_byte(
            sim,
            vector_values.copy_byte()?,
            query,
        )?;
        assert_score(scorer3.score(idx1)?, expected);
        let mut scorer4 = memseg_scorer().get_random_vector_scorer_byte(
            sim,
            vector_values.copy_byte()?,
            query,
        )?;
        assert_score(scorer4.score(idx1)?, expected);
        Ok(())
    }

    pub fn test_random_scorer() -> Result<()> {
        test_random_scorer_impl(*DEFAULT_MAX_CHUNK_SIZE as i64, byte_array_random_func)
    }

    pub fn test_random_scorer_max() -> Result<()> {
        test_random_scorer_impl(*DEFAULT_MAX_CHUNK_SIZE as i64, byte_array_max_func)
    }

    pub fn test_random_scorer_min() -> Result<()> {
        test_random_scorer_impl(*DEFAULT_MAX_CHUNK_SIZE as i64, byte_array_min_func)
    }

    pub fn test_random_small_chunk_size() -> Result<()> {
        let max_chunk_size = random_long_between(32, 128);
        test_random_scorer_impl(max_chunk_size, byte_array_random_func)
    }

    /// `testRandomScorer(long maxChunkSize, Function<Integer, byte[]>
    /// byteArraySupplier)`.
    fn test_random_scorer_impl(
        max_chunk_size: i64,
        byte_array_supplier: fn(i32) -> Vec<u8>,
    ) -> Result<()> {
        let mut dir = mmap_directory("testRandomScorer", max_chunk_size)?;
        let result = (|| -> Result<()> {
            let dims = random_int_between(1, 4096);
            let size = random_int_between(2, 100);
            let mut vectors: Vec<Vec<u8>> = Vec::with_capacity(size as usize);
            let file_name = format!("foo-{dims}");
            {
                let mut out = dir.create_output(&file_name, &*DEFAULT_IO_CONTEXT)?;
                for _ in 0..size {
                    let vec = byte_array_supplier(dims);
                    out.write_bytes(&vec, 0, vec.len())?;
                    vectors.push(vec);
                }
                out.close()?;
            }

            let mut input = dir.open_input(&file_name, &*DEFAULT_IO_CONTEXT)?;
            for _times in 0..TIMES {
                for sim in SIMS {
                    let vector_values = vector_values(dims, size, &*input, sim)?;
                    let idx0 = random_int_between(0, size - 1);
                    let idx1 = random_int_between(0, size - 1); // may be the same as idx0 - which is ok.
                    assert_scorers(sim, &*vector_values, idx0, idx1, &vectors[idx0 as usize])?;
                }
            }
            input.close()
        })();
        result.and(dir.close())
    }

    pub fn test_random_slice_small() -> Result<()> {
        test_random_slice_impl(30, 64, 1, byte_array_random_func)
    }

    pub fn test_random_slice() -> Result<()> {
        let dims = random_int_between(1, 4096);
        let max_chunk_size = random_long_between(32, 128);
        let initial_offset = random_int_between(1, 129);
        test_random_slice_impl(dims, max_chunk_size, initial_offset, byte_array_random_func)
    }

    /// `testRandomSliceImpl(...)`: tests with a slice that has a non-zero
    /// initial offset.
    fn test_random_slice_impl(
        dims: i32,
        max_chunk_size: i64,
        initial_offset: i32,
        byte_array_supplier: fn(i32) -> Vec<u8>,
    ) -> Result<()> {
        let mut dir = mmap_directory("testRandomSliceImpl", max_chunk_size)?;
        let result = (|| -> Result<()> {
            let size = random_int_between(2, 100);
            let mut vectors: Vec<Vec<u8>> = Vec::with_capacity(size as usize);
            let file_name = format!("baz-{dims}");
            {
                let mut out = dir.create_output(&file_name, &*DEFAULT_IO_CONTEXT)?;
                let ba = vec![0u8; initial_offset as usize];
                out.write_bytes(&ba, 0, ba.len())?;
                for _ in 0..size {
                    let vec = byte_array_supplier(dims);
                    out.write_bytes(&vec, 0, vec.len())?;
                    vectors.push(vec);
                }
                out.close()?;
            }

            let mut outter = dir.open_input(&file_name, &*DEFAULT_IO_CONTEXT)?;
            let mut input = outter.slice(
                "slice",
                initial_offset as i64,
                outter.length() - initial_offset as i64,
            )?;
            for _times in 0..TIMES {
                for sim in SIMS {
                    let vector_values = vector_values(dims, size, &*input, sim)?;
                    let idx0 = random_int_between(0, size - 1);
                    let idx1 = random_int_between(0, size - 1); // may be the same as idx0 - which is ok.
                    assert_scorers(sim, &*vector_values, idx0, idx1, &vectors[idx0 as usize])?;
                }
            }
            input.close()?;
            outter.close()
        })();
        result.and(dir.close())
    }

    /// `testCopiesAcrossThreads()`: tests that copies in threads do not
    /// interfere with each other.
    ///
    /// `Executors.newFixedThreadPool(2, new NamedThreadFactory("copiesThreads"))`
    /// with `invokeAll` runs each task on its own named thread, joined before
    /// the results are read. Java builds each task's scorer
    /// (`scorer.copy().scorer()`) on the test thread; Rucene's scorers are
    /// not `Send`, so the copied supplier, which is, moves to the task, and
    /// the task calls `scorer()` first.
    pub fn test_copies_across_threads() -> Result<()> {
        let max_chunk_size: i64 = 32;
        let dims = 34; // dimensions that are larger than the chunk size, to force fallback
        let vec1 = vec![1u8; dims as usize];
        let vec2 = vec![2u8; dims as usize];
        let mut dir = mmap_directory("testRace", max_chunk_size)?;
        let result = (|| -> Result<()> {
            let file_name = format!("biz-{dims}");
            {
                let mut out = dir.create_output(&file_name, &*DEFAULT_IO_CONTEXT)?;
                let bytes = concat(&[&vec1, &vec1, &vec2, &vec2]);
                out.write_bytes(&bytes, 0, bytes.len())?;
                out.close()?;
            }
            let mut input = dir.open_input(&file_name, &*DEFAULT_IO_CONTEXT)?;
            for sim in SIMS {
                let vector_values = vector_values(dims, 4, &*input, sim)?;
                let score_supplier = default_scorer()
                    .get_random_vector_scorer_supplier_byte(sim, vector_values.copy_byte()?)?;
                let mut scorer1 = score_supplier.scorer()?;
                scorer1.set_scoring_ordinal(0)?;
                let expected_score1 = scorer1.score(1)?;
                scorer1.set_scoring_ordinal(2)?;
                let expected_score2 = scorer1.score(3)?;

                let scorer = memseg_scorer()
                    .get_random_vector_scorer_supplier_byte(sim, vector_values.copy_byte()?)?;
                let tasks = vec![
                    AssertingScoreCallable {
                        scorer: scorer.copy()?,
                        target: 0,
                        ord: 1,
                        expected_score: expected_score1,
                    },
                    AssertingScoreCallable {
                        scorer: scorer.copy()?,
                        target: 2,
                        ord: 3,
                        expected_score: expected_score2,
                    },
                ];
                let executor = NamedThreadFactory::new("copiesThreads");
                let mut handles = Vec::new();
                let mut results: Vec<Arc<Mutex<Option<Option<String>>>>> = Vec::new();
                for task in tasks {
                    let slot = Arc::new(Mutex::new(None));
                    let sink = Arc::clone(&slot);
                    handles.push(executor.new_thread(move || {
                        let outcome = task.call();
                        *sink.lock().unwrap_or_else(|e| e.into_inner()) = Some(outcome);
                    })?);
                    results.push(slot);
                }
                // `executor.shutdown()`; `executor.awaitTermination(30, SECONDS)`.
                let mut terminated = true;
                for handle in handles {
                    terminated &= handle.join().is_ok();
                }
                Assert::assert_true(terminated);
                let not_done = results
                    .iter()
                    .filter(|r| r.lock().unwrap_or_else(|e| e.into_inner()).is_none())
                    .count() as i64;
                Assert::assert_equals(not_done, 0i64);
                for res in &results {
                    let res = res
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .clone()
                        .expect("isDone");
                    let shown = match &res {
                        Some(t) => format!("Optional[{t}]"),
                        None => "Optional.empty".to_string(),
                    };
                    Assert::assert_true_msg(&format!("Unexpected exception{shown}"), res.is_none());
                }
            }
            input.close()
        })();
        result.and(dir.close())
    }

    /// `record AssertingScoreCallable(UpdateableRandomVectorScorer scorer, int
    /// target, int ord, float expectedScore) implements
    /// Callable<Optional<Throwable>>`: a callable that scores the given ord
    /// and scorer and asserts the expected result. It holds the supplier
    /// copy its scorer comes from (see [`test_copies_across_threads`]).
    struct AssertingScoreCallable {
        scorer: Box<dyn RandomVectorScorerSupplier>,
        target: i32,
        ord: i32,
        expected_score: f32,
    }

    impl AssertingScoreCallable {
        /// `call()`: the `Throwable` caught, as its string form, or `None`.
        fn call(self) -> Option<String> {
            let outcome =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
                    let mut scorer = self.scorer.scorer()?;
                    scorer.set_scoring_ordinal(self.target)?;
                    for _ in 0..100 {
                        assert_score(scorer.score(self.ord)?, self.expected_score);
                    }
                    Ok(())
                }));
            match outcome {
                Ok(Ok(())) => None,
                Ok(Err(e)) => Some(e.to_string()),
                Err(payload) => Some(
                    payload
                        .downcast_ref::<String>()
                        .cloned()
                        .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
                        .unwrap_or_default(),
                ),
            }
        }
    }

    /// `testLarge()` (`@Monster("requires gigabytes of disk space")`): tests
    /// with a large amount of data (> 2GB), which ensures that data offsets
    /// do not overflow.
    pub fn test_large() -> Result<()> {
        let mut dir = MMapDirectory::open(LuceneTestCase::create_temp_dir_prefix("testLarge"))?;
        let result = (|| -> Result<()> {
            let dims = 8192;
            let size = 262500;
            let file_name = format!("large-{dims}");
            {
                let mut out = dir.create_output(&file_name, &*DEFAULT_IO_CONTEXT)?;
                for i in 0..size {
                    let vec = vector(i, dims);
                    out.write_bytes(&vec, 0, vec.len())?;
                }
                out.close()?;
            }

            let mut input = dir.open_input(&file_name, &*DEFAULT_IO_CONTEXT)?;
            randomizedtesting::jdk::exception::java_assert_no_message(
                input.length() > i32::MAX as i64,
            );
            for _times in 0..TIMES {
                for sim in SIMS {
                    let vector_values = vector_values(dims, size, &*input, sim)?;
                    let ord1 = random_int_between(0, size - 1);
                    let ord2 = size - 1;
                    for ords in [[ord1, ord2], [ord2, ord1]] {
                        let idx0 = ords[0];
                        let idx1 = ords[ords.len() - 1];
                        let query = vector(idx0, dims);
                        assert_scorers(sim, &*vector_values, idx0, idx1, &query)?;
                    }
                }
            }
            input.close()
        })();
        result.and(dir.close())
    }

    /// `testWithFloatValues()`: tests that the FlatVectorsScorer handles float
    /// vectors correctly.
    ///
    /// Not ported: the two closing `expectThrows(Throwable.class, () ->
    /// SCORER.getRandomVectorScorer(sim, vectorValues, new byte[] {1}))`
    /// (`TestVectorScorer.java:376-381`) pass float vector values with a byte
    /// target, which Java rejects at run time; Rucene's typed
    /// `get_random_vector_scorer_byte` takes `ByteVectorValues` only, so the
    /// call does not compile.
    pub fn test_with_float_values() -> Result<()> {
        let mut dir = MMapDirectory::open(LuceneTestCase::create_temp_dir_prefix(
            "testWithFloatValues",
        ))?;
        let result = (|| -> Result<()> {
            let file_name = "floatvalues";
            {
                let mut out = dir.create_output(file_name, &*DEFAULT_IO_CONTEXT)?;
                let vec = float_to_byte_array(1f32); // single vector, with one dimension
                out.write_bytes(&vec, 0, vec.len())?;
                out.close()?;
            }

            let mut input = dir.open_input(file_name, &*DEFAULT_IO_CONTEXT)?;
            for _times in 0..TIMES {
                for sim in SIMS {
                    let vector_values = float_vector_values(1, 1, &*input, sim)?;
                    // `vectorValues.getEncoding()`: float vector values encode FLOAT32.
                    randomizedtesting::jdk::exception::java_assert_no_message(
                        VectorEncoding::FLOAT32.byte_size() == 4,
                    );

                    let supplier1 = default_scorer().get_random_vector_scorer_supplier_float(
                        sim,
                        vector_values.copy_float()?,
                    )?;
                    let supplier2 = memseg_scorer().get_random_vector_scorer_supplier_float(
                        sim,
                        vector_values.copy_float()?,
                    )?;
                    // these assertion assumes that the supplier and scorer's toString will have float
                    // in it, since it's based on float vectors.
                    Assert::assert_true(
                        RandomVectorScorerSupplier::to_string(&*supplier1)
                            .to_lowercase()
                            .contains("float"),
                    );
                    Assert::assert_true(
                        RandomVectorScorerSupplier::to_string(&*supplier2)
                            .to_lowercase()
                            .contains("float"),
                    );
                    Assert::assert_true(
                        RandomVectorScorer::to_string(&*supplier1.scorer()?)
                            .to_lowercase()
                            .contains("float"),
                    );
                    Assert::assert_true(
                        RandomVectorScorer::to_string(&*supplier2.scorer()?)
                            .to_lowercase()
                            .contains("float"),
                    );
                    let mut scorer1idx0 = supplier1.scorer()?;
                    scorer1idx0.set_scoring_ordinal(0)?;
                    let mut expected = scorer1idx0.score(0)?;
                    scorer1idx0 = supplier2.scorer()?;
                    scorer1idx0.set_scoring_ordinal(0)?;
                    assert_score(scorer1idx0.score(0)?, expected);

                    let mut scorer1 = default_scorer().get_random_vector_scorer_float(
                        sim,
                        vector_values.copy_float()?,
                        &[1f32],
                    )?;
                    let mut scorer2 = memseg_scorer().get_random_vector_scorer_float(
                        sim,
                        vector_values.copy_float()?,
                        &[1f32],
                    )?;
                    Assert::assert_true(
                        RandomVectorScorer::to_string(&*scorer1)
                            .to_lowercase()
                            .contains("float"),
                    );
                    Assert::assert_true(
                        RandomVectorScorer::to_string(&*scorer2)
                            .to_lowercase()
                            .contains("float"),
                    );
                    expected = scorer1.score(0)?;
                    assert_score(scorer2.score(0)?, expected);
                }
            }
            input.close()
        })();
        result.and(dir.close())
    }

    /// `vectorValues(int dims, int size, IndexInput in, VectorSimilarityFunction sim)`.
    fn vector_values(
        dims: i32,
        size: i32,
        input: &dyn IndexInput,
        sim: VectorSimilarityFunction,
    ) -> Result<Box<dyn ByteVectorValues>> {
        Ok(new_dense_off_heap_byte_vector_values(
            dims,
            size,
            input.slice("byteValues", 0, input.length())?,
            dims,
            memseg_scorer(),
            sim,
        ))
    }

    /// `floatVectorValues(int dims, int size, IndexInput in, VectorSimilarityFunction sim)`.
    fn float_vector_values(
        dims: i32,
        size: i32,
        input: &dyn IndexInput,
        sim: VectorSimilarityFunction,
    ) -> Result<Box<dyn FloatVectorValues>> {
        Ok(new_dense_off_heap_float_vector_values(
            dims,
            size,
            input.slice("floatValues", 0, input.length())?,
            dims,
            memseg_scorer(),
            sim,
        ))
    }

    /// `vector(int ord, int dims)` (`static`): creates the vector based on the
    /// given ordinal, which is reproducible given the ord and dims.
    fn vector(ord: i32, dims: i32) -> Vec<u8> {
        // `Objects.hash(ord, dims)`: `31 * (31 * 1 + ord) + dims`.
        let random = JdkRandom::new(objects_hash(ord, dims) as i64);
        let mut ba = vec![0u8; dims as usize];
        for i in 0..dims as usize {
            ba[i] =
                RandomNumbers::random_int_between(&random, i8::MIN as i32, i8::MAX as i32) as u8;
        }
        ba
    }

    /// `Objects.hash(Object...)` over two `Integer`s (`Arrays.hashCode`).
    fn objects_hash(a: i32, b: i32) -> i32 {
        let mut result: i32 = 1;
        result = result.wrapping_mul(31).wrapping_add(a);
        result = result.wrapping_mul(31).wrapping_add(b);
        result
    }

    /// `concat(byte[]... arrays)` (`static`): concatenates byte arrays.
    fn concat(arrays: &[&[u8]]) -> Vec<u8> {
        let mut baos = Vec::new();
        for ba in arrays {
            baos.extend_from_slice(ba);
        }
        baos
    }

    /// `floatToByteArray(float value)` (`public static`): converts a float
    /// value to a byte array.
    fn float_to_byte_array(value: f32) -> Vec<u8> {
        value.to_le_bytes().to_vec()
    }

    /// `randomIntBetween(int, int)` (`static`).
    fn random_int_between(min_inclusive: i32, max_inclusive: i32) -> i32 {
        RandomNumbers::random_int_between(&*LuceneTestCase::random(), min_inclusive, max_inclusive)
    }

    /// `randomLongBetween(long, long)` (`static`).
    fn random_long_between(min_inclusive: i64, max_inclusive: i64) -> i64 {
        RandomNumbers::random_long_between(&*LuceneTestCase::random(), min_inclusive, max_inclusive)
    }

    /// `BYTE_ARRAY_RANDOM_FUNC` (`private static final`).
    fn byte_array_random_func(size: i32) -> Vec<u8> {
        let mut ba = vec![0u8; size as usize];
        for b in ba.iter_mut() {
            *b = LuceneTestCase::random().next_int() as u8;
        }
        ba
    }

    /// `BYTE_ARRAY_MAX_FUNC` (`private static final`).
    fn byte_array_max_func(size: i32) -> Vec<u8> {
        vec![i8::MAX as u8; size as usize]
    }

    /// `BYTE_ARRAY_MIN_FUNC` (`private static final`).
    fn byte_array_min_func(size: i32) -> Vec<u8> {
        vec![i8::MIN as u8; size as usize]
    }

    /// `TIMES` (`private static final`): a loop iteration times.
    const TIMES: i32 = 100;

    pub fn class() -> Arc<Class> {
        lucene_class("org.apache.lucene.internal.vectorization.TestVectorScorer")
            .method(
                Method::new("beforeClass")
                    .annotate(BeforeClass)
                    .throws(&["java.lang.Exception"])
                    .static_body(before_class),
            )
            .method(t("testSimpleScorer", test_simple_scorer))
            .method(t(
                "testSimpleScorerSmallChunkSize",
                test_simple_scorer_small_chunk_size,
            ))
            .method(t(
                "testSimpleScorerMedChunkSize",
                test_simple_scorer_med_chunk_size,
            ))
            .method(t("testRandomScorer", test_random_scorer))
            .method(t("testRandomScorerMax", test_random_scorer_max))
            .method(t("testRandomScorerMin", test_random_scorer_min))
            .method(t("testRandomSmallChunkSize", test_random_small_chunk_size))
            .method(t("testRandomSliceSmall", test_random_slice_small))
            .method(t("testRandomSlice", test_random_slice))
            .method(t("testCopiesAcrossThreads", test_copies_across_threads))
            .method(t("testLarge", test_large).annotate(Monster {
                value: "requires gigabytes of disk space".to_string(),
            }))
            .method(t("testWithFloatValues", test_with_float_values))
            .build()
    }
}

// -----------------------------------------------------------------------------
// The suites
// -----------------------------------------------------------------------------

#[test]
fn test_vectorization_provider() {
    run(vectorization_provider_class);
}

#[test]
fn test_posting_decoding_util() {
    run(posting_decoding_util_class);
}

#[test]
fn test_vector_util_support() {
    run(vector_util_support_class);
}

#[test]
fn test_vector_scorer() {
    run(vector_scorer::class);
}
