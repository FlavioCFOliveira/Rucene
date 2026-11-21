//! Port of `lucene/core/src/test/org/apache/lucene/codecs/hnsw/TestFlatVectorScorer.java`.
//!
//! The class extends `BaseVectorizationTestCase` and is built by its
//! `@ParametersFactory` with each of the four scorers and each of the three
//! directory suppliers; a parameter is the index of the scorer or supplier
//! in Java's list, with the `toString()` of the scorer.
//!
//! Java hands one `KnnVectorValues` to every scorer call. Rucene's scorers
//! take their values by value, so each call receives a `copy()` of them,
//! which reads the same slice (as `TestVectorScorer`'s port does).

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::annotations::ParametersFactory;
use randomizedtesting::jdk::reflect::{modifier, Class, Constructor, Method, Value};
use randomizedtesting::jdk::util::shuffle;
use randomizedtesting::junit::Assert;
use rucene::codecs::hnsw::flat_vectors::{
    DefaultFlatVectorScorer, FlatVectorScorerUtil, FlatVectorsScorer,
};
use rucene::codecs::lucene99::flat_vectors_format::{
    new_dense_off_heap_byte_vector_values, new_dense_off_heap_float_vector_values,
};
use rucene::codecs::lucene99::scalar_quantized_scorer::Lucene99ScalarQuantizedVectorScorer;
use rucene::error::Result;
use rucene::index::vector_values::{ByteVectorValues, FloatVectorValues};
use rucene::index::{from_floats, VectorSimilarityFunction};
use rucene::store::mmap::MMapDirectory;
use rucene::store::{
    Directory, IOContext, IndexInput, IndexOutput, NativeFSLockFactory, DEFAULT_IO_CONTEXT,
};
use rucene::util::hnsw::scorer::RandomVectorScorer;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use super::base_vectorization_test_case::{self, maybe_panama_provider};
use super::support::ftest;
use crate::core_support::{assert_array_equals_f32, assert_equals, random};

use VectorSimilarityFunction::{COSINE, DOT_PRODUCT, EUCLIDEAN, MAXIMUM_INNER_PRODUCT};

/// `count` (`private static final AtomicInteger`).
static COUNT: AtomicI32 = AtomicI32::new(0);

/// A `Directory` a `ThrowingSupplier<Directory>` returns: `newDirectory()`'s
/// `BaseDirectoryWrapper`, or an `MMapDirectory`.
enum SuppliedDirectory {
    Test(TestDirectory),
    MMap(MMapDirectory),
}

impl SuppliedDirectory {
    /// `createOutput(name, context)`.
    fn create_output(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        match self {
            SuppliedDirectory::Test(d) => d.as_directory().create_output(name, context),
            SuppliedDirectory::MMap(d) => d.create_output(name, context),
        }
    }

    /// `openInput(name, context)`.
    fn open_input(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        match self {
            SuppliedDirectory::Test(d) => d.as_directory().open_input(name, context),
            SuppliedDirectory::MMap(d) => d.open_input(name, context),
        }
    }

    /// `close()`.
    fn close(&mut self) -> Result<()> {
        match self {
            SuppliedDirectory::Test(d) => d.close(),
            SuppliedDirectory::MMap(d) => d.close(),
        }
    }
}

/// `ThrowingSupplier<Directory>`.
type DirectorySupplier = fn() -> Result<SuppliedDirectory>;

/// The similarity functions the tests iterate, in Java's order.
const SIMS: [VectorSimilarityFunction; 4] = [COSINE, DOT_PRODUCT, EUCLIDEAN, MAXIMUM_INNER_PRODUCT];

/// `TestFlatVectorScorer`'s instance: `flatVectorsScorer` and `newDirectory`.
pub struct TestFlatVectorScorer {
    flat_vectors_scorer: Arc<dyn FlatVectorsScorer>,
    new_directory: DirectorySupplier,
}

/// The `scorers` of `parametersFactory()`, in Java's order.
fn scorers() -> Vec<Arc<dyn FlatVectorsScorer>> {
    vec![
        Arc::new(DefaultFlatVectorScorer::INSTANCE),
        Arc::new(Lucene99ScalarQuantizedVectorScorer::new(Arc::new(
            DefaultFlatVectorScorer::INSTANCE,
        ))),
        Arc::new(FlatVectorScorerUtil::get_lucene99_flat_vectors_scorer()),
        maybe_panama_provider().get_lucene99_flat_vectors_scorer(),
    ]
}

/// `TestFlatVectorScorer::newDirectory`.
fn new_directory() -> Result<SuppliedDirectory> {
    Ok(SuppliedDirectory::Test(LuceneTestCase::new_directory()))
}

/// `() -> new MMapDirectory(createTempDir(count.getAndIncrement() + "-"))`.
fn new_mmap_directory() -> Result<SuppliedDirectory> {
    let prefix = format!("{}-", COUNT.fetch_add(1, Ordering::SeqCst));
    Ok(SuppliedDirectory::MMap(MMapDirectory::open(
        LuceneTestCase::create_temp_dir_prefix(&prefix),
    )?))
}

/// `() -> new MMapDirectory(createTempDir(count.getAndIncrement() + "-"), 1024)`:
/// the default lock factory is `FSLockFactory.getDefault()`, a
/// `NativeFSLockFactory`.
fn new_mmap_directory_1024() -> Result<SuppliedDirectory> {
    let prefix = format!("{}-", COUNT.fetch_add(1, Ordering::SeqCst));
    Ok(SuppliedDirectory::MMap(MMapDirectory::with_options(
        LuceneTestCase::create_temp_dir_prefix(&prefix),
        Box::new(NativeFSLockFactory),
        1024,
    )?))
}

/// The `dirs` of `parametersFactory()`, in Java's order.
const DIRS: [DirectorySupplier; 3] = [new_directory, new_mmap_directory, new_mmap_directory_1024];

/// `TestFlatVectorScorer`.
pub fn class_def() -> Arc<Class> {
    type T = TestFlatVectorScorer;
    Class::builder("org.apache.lucene.codecs.hnsw.TestFlatVectorScorer")
        .extends(&base_vectorization_test_case::class())
        .constructor(Constructor::with_params(
            &[
                ("org.apache.lucene.codecs.hnsw.FlatVectorsScorer", None),
                (
                    "org.apache.lucene.codecs.hnsw.TestFlatVectorScorer$ThrowingSupplier",
                    None,
                ),
            ],
            |args: &[Value]| {
                let index = |v: Option<&Value>| match v {
                    Some(Value::Object(o, _)) => o.downcast_ref::<usize>().copied().unwrap_or(0),
                    _ => 0,
                };
                TestFlatVectorScorer {
                    flat_vectors_scorer: Arc::clone(&scorers()[index(args.first())]),
                    new_directory: DIRS[index(args.get(1))],
                }
            },
        ))
        .method(
            Method::new("parametersFactory")
                .modifiers(modifier::PUBLIC | modifier::STATIC)
                .returns("java.lang.Iterable<java.lang.Object[]>")
                .annotate(ParametersFactory::default())
                .static_value(|_| parameters_factory()),
        )
        .method(ftest("testDefaultOrMemSegScorer", |_: &mut T| {
            test_default_or_mem_seg_scorer()
        }))
        .method(ftest(
            "testMultipleByteScorers",
            T::test_multiple_byte_scorers,
        ))
        .method(ftest(
            "testMultipleFloatScorers",
            T::test_multiple_float_scorers,
        ))
        .method(ftest(
            "testCheckByteDimensions",
            T::test_check_byte_dimensions,
        ))
        .method(ftest(
            "testCheckFloatDimensions",
            T::test_check_float_dimensions,
        ))
        .method(ftest("testBulkScorerBytes", T::test_bulk_scorer_bytes))
        .method(ftest("testBulkScorerFloats", T::test_bulk_scorer_floats))
        .method(ftest(
            "testOnHeapBulkScorerFloats",
            T::test_on_heap_bulk_scorer_floats,
        ))
        .build()
}

/// `parametersFactory()` (`@ParametersFactory`).
fn parameters_factory() -> Value {
    let scorers = scorers();
    let mut objs = Vec::new();
    for (s, scorer) in scorers.iter().enumerate() {
        for d in 0..DIRS.len() {
            objs.push(Value::Array(vec![
                Value::Object(Arc::new(s), scorer.to_string()),
                Value::Object(
                    Arc::new(d),
                    format!("org.apache.lucene.codecs.hnsw.TestFlatVectorScorer$$Lambda/{d}"),
                ),
            ]));
        }
    }
    Value::List(objs)
}

/// Port of `TestFlatVectorScorer.testDefaultOrMemSegScorer`.
fn test_default_or_mem_seg_scorer() -> Result<()> {
    let scorer = FlatVectorScorerUtil::get_lucene99_flat_vectors_scorer();
    let s = scorer.to_string();
    // `is(oneOf("DefaultFlatVectorScorer()", "Lucene99MemorySegmentFlatVectorsScorer()"))`.
    Assert::assert_true_msg(
        &format!(
            "\nExpected: is one of {{\"DefaultFlatVectorScorer()\", \
             \"Lucene99MemorySegmentFlatVectorsScorer()\"}}\n     but: was \"{s}\""
        ),
        s == "DefaultFlatVectorScorer()" || s == "Lucene99MemorySegmentFlatVectorsScorer()",
    );
    Ok(())
}

/// `KnnVectorValues` of either encoding, as the `assert*` helpers take them
/// (Java dispatches on `values.getEncoding()`).
enum Values {
    Byte(Box<dyn ByteVectorValues>),
    Float(Box<dyn FloatVectorValues>),
}

impl Values {
    fn size(&self) -> i32 {
        match self {
            Values::Byte(v) => v.size(),
            Values::Float(v) => v.size(),
        }
    }

    fn dimension(&self) -> i32 {
        match self {
            Values::Byte(v) => v.dimension(),
            Values::Float(v) => v.dimension(),
        }
    }
}

impl TestFlatVectorScorer {
    /// Tests that the creation of another scorer does not disturb previous scorers
    ///
    /// Port of `TestFlatVectorScorer.testMultipleByteScorers`.
    fn test_multiple_byte_scorers(&mut self) -> Result<()> {
        let vec0: Vec<u8> = vec![0, 0, 0, 0];
        let vec1: Vec<u8> = vec![1, 1, 1, 1];
        let vec2: Vec<u8> = vec![15, 15, 15, 15];

        let file_name = "testMultipleByteScorers";
        let mut dir = (self.new_directory)()?;
        {
            let mut out = dir.create_output(file_name, &*DEFAULT_IO_CONTEXT)?;
            out.write_bytes(&concat_bytes(&[&vec0, &vec1, &vec2]), 0, vec0.len() * 3)?;
            out.close()?;
        }
        {
            let mut r#in = dir.open_input(file_name, &*DEFAULT_IO_CONTEXT)?;
            let vector_values = self.byte_vector_values(4, 3, &*r#in, EUCLIDEAN)?;
            let ss = self
                .flat_vectors_scorer
                .get_random_vector_scorer_supplier_byte(EUCLIDEAN, vector_values)?;

            let mut scorer_against_ord0 = ss.scorer()?;
            scorer_against_ord0.set_scoring_ordinal(0)?;
            let first_score = scorer_against_ord0.score(1)?;
            let mut scorer_against_ord2 = ss.scorer()?;
            scorer_against_ord2.set_scoring_ordinal(2)?;
            let score_again = scorer_against_ord0.score(1)?;

            // `assertThat(scoreAgain, equalTo(firstScore))`: `Float.equals`.
            assert_equals(&first_score.to_bits(), &score_again.to_bits());
            r#in.close()?;
        }
        dir.close()
    }

    /// Tests that the creation of another scorer does not perturb previous scorers
    ///
    /// Port of `TestFlatVectorScorer.testMultipleFloatScorers`.
    fn test_multiple_float_scorers(&mut self) -> Result<()> {
        let vec0: Vec<f32> = vec![0.0, 0.0, 0.0, 0.0];
        let vec1: Vec<f32> = vec![1.0, 1.0, 1.0, 1.0];
        let vec2: Vec<f32> = vec![15.0, 15.0, 15.0, 15.0];

        let file_name = "testMultipleFloatScorers";
        let mut dir = (self.new_directory)()?;
        {
            let mut out = dir.create_output(file_name, &*DEFAULT_IO_CONTEXT)?;
            out.write_bytes(
                &concat_floats(&[&vec0, &vec1, &vec2]),
                0,
                vec0.len() * 4 * 3,
            )?;
            out.close()?;
        }
        {
            let mut r#in = dir.open_input(file_name, &*DEFAULT_IO_CONTEXT)?;
            let vector_values = self.float_vector_values(4, 3, &*r#in, EUCLIDEAN)?;
            let ss = self
                .flat_vectors_scorer
                .get_random_vector_scorer_supplier_float(EUCLIDEAN, vector_values)?;

            let mut scorer_against_ord0 = ss.scorer()?;
            scorer_against_ord0.set_scoring_ordinal(0)?;
            let first_score = scorer_against_ord0.score(1)?;
            let mut scorer_against_ord2 = ss.scorer()?;
            scorer_against_ord2.set_scoring_ordinal(2)?;
            let score_again = scorer_against_ord0.score(1)?;

            // `assertThat(scoreAgain, equalTo(firstScore))`: `Float.equals`.
            assert_equals(&first_score.to_bits(), &score_again.to_bits());
            r#in.close()?;
        }
        dir.close()
    }

    /// Port of `TestFlatVectorScorer.testCheckByteDimensions`.
    fn test_check_byte_dimensions(&mut self) -> Result<()> {
        let vec0 = vec![0u8; 4];
        let file_name = "testCheckByteDimensions";
        let mut dir = (self.new_directory)()?;
        {
            let mut out = dir.create_output(file_name, &*DEFAULT_IO_CONTEXT)?;
            out.write_bytes(&vec0, 0, vec0.len())?;
            out.close()?;
        }
        {
            let mut r#in = dir.open_input(file_name, &*DEFAULT_IO_CONTEXT)?;
            for sim in SIMS {
                let vector_values = self.byte_vector_values(4, 1, &*r#in, sim)?;
                expect_throws("java.lang.IllegalArgumentException", || {
                    self.flat_vectors_scorer.get_random_vector_scorer_byte(
                        sim,
                        vector_values,
                        &[0u8; 5],
                    )
                });
            }
            r#in.close()?;
        }
        dir.close()
    }

    /// Port of `TestFlatVectorScorer.testCheckFloatDimensions`.
    fn test_check_float_dimensions(&mut self) -> Result<()> {
        let vec0 = vec![0f32; 4];
        let file_name = "testCheckFloatDimensions";
        let mut dir = (self.new_directory)()?;
        {
            let mut out = dir.create_output(file_name, &*DEFAULT_IO_CONTEXT)?;
            out.write_bytes(&concat_floats(&[&vec0]), 0, vec0.len() * 4)?;
            out.close()?;
        }
        {
            let mut r#in = dir.open_input(file_name, &*DEFAULT_IO_CONTEXT)?;
            for sim in SIMS {
                let vector_values = self.float_vector_values(4, 1, &*r#in, sim)?;
                expect_throws("java.lang.IllegalArgumentException", || {
                    self.flat_vectors_scorer.get_random_vector_scorer_float(
                        sim,
                        vector_values,
                        &[0f32; 5],
                    )
                });
            }
            r#in.close()?;
        }
        dir.close()
    }

    /// Port of `TestFlatVectorScorer.testBulkScorerBytes`.
    fn test_bulk_scorer_bytes(&mut self) -> Result<()> {
        let dims = random().next_int_range(1, 1024);
        let size = random().next_int_range(2, 255);
        let file_name = "testBulkScorerBytes";
        let mut dir = (self.new_directory)()?;
        {
            let mut out = dir.create_output(file_name, &*DEFAULT_IO_CONTEXT)?;
            for _ in 0..size {
                let ba = random_byte_vector(dims);
                out.write_bytes(&ba, 0, ba.len())?;
            }
            out.close()?;
        }
        {
            let mut r#in = dir.open_input(file_name, &*DEFAULT_IO_CONTEXT)?;
            debug_assert!(r#in.length() == dims as i64 * size as i64);
            for sim in SIMS {
                let values = Values::Byte(self.byte_vector_values(dims, size, &*r#in, sim)?);
                self.assert_bulk_equals_non_bulk(&values, sim)?;
                self.assert_bulk_equals_non_bulk_supplier(&values, sim)?;
                self.assert_scores_against_default_flat_scorer(&values, sim)?;
            }
            r#in.close()?;
        }
        dir.close()
    }

    /// Port of `TestFlatVectorScorer.testBulkScorerFloats`.
    fn test_bulk_scorer_floats(&mut self) -> Result<()> {
        let dims = random().next_int_range(1, 1024);
        let size = random().next_int_range(2, 255);
        let file_name = "testBulkScorerFloats";
        let mut dir = (self.new_directory)()?;
        {
            let mut out = dir.create_output(file_name, &*DEFAULT_IO_CONTEXT)?;
            for _ in 0..size {
                let ba = concat_floats(&[&random_float_vector(dims)]);
                out.write_bytes(&ba, 0, ba.len())?;
            }
            out.close()?;
        }
        {
            let mut r#in = dir.open_input(file_name, &*DEFAULT_IO_CONTEXT)?;
            debug_assert!(r#in.length() == dims as i64 * size as i64 * 4);
            for sim in SIMS {
                let values = Values::Float(self.float_vector_values(dims, size, &*r#in, sim)?);
                self.assert_bulk_equals_non_bulk(&values, sim)?;
                self.assert_bulk_equals_non_bulk_supplier(&values, sim)?;
                self.assert_scores_against_default_flat_scorer(&values, sim)?;
            }
            r#in.close()?;
        }
        dir.close()
    }

    /// Port of `TestFlatVectorScorer.testOnHeapBulkScorerFloats`.
    fn test_on_heap_bulk_scorer_floats(&mut self) -> Result<()> {
        let dims = random().next_int_range(1, 1024);
        let size = random().next_int_range(2, 255);
        let mut vectors = Vec::new();
        for _ in 0..size {
            vectors.push(random_float_vector(dims));
        }
        let values = Values::Float(Box::new(from_floats(vectors, dims)));
        for sim in SIMS {
            self.assert_bulk_equals_non_bulk(&values, sim)?;
            self.assert_bulk_equals_non_bulk_supplier(&values, sim)?;
            self.assert_scores_against_default_flat_scorer(&values, sim)?;
        }
        Ok(())
    }

    /// `flatVectorsScorer.getRandomVectorScorerSupplier(sim, values)`.
    fn supplier(
        scorer: &dyn FlatVectorsScorer,
        values: &Values,
        sim: VectorSimilarityFunction,
    ) -> Result<Box<dyn rucene::util::hnsw::scorer::RandomVectorScorerSupplier>> {
        match values {
            Values::Byte(v) => scorer.get_random_vector_scorer_supplier_byte(sim, v.copy_byte()?),
            Values::Float(v) => {
                scorer.get_random_vector_scorer_supplier_float(sim, v.copy_float()?)
            }
        }
    }

    /// Port of `TestFlatVectorScorer.assertBulkEqualsNonBulk`.
    fn assert_bulk_equals_non_bulk(
        &self,
        values: &Values,
        sim: VectorSimilarityFunction,
    ) -> Result<()> {
        let dims = values.dimension();
        let size = values.size();
        let delta = 1e-3f32 * size as f32;
        let mut scorer = match values {
            Values::Byte(v) => self.flat_vectors_scorer.get_random_vector_scorer_byte(
                sim,
                v.copy_byte()?,
                &random_byte_vector(dims),
            )?,
            Values::Float(v) => self.flat_vectors_scorer.get_random_vector_scorer_float(
                sim,
                v.copy_float()?,
                &random_float_vector(dims),
            )?,
        };
        let indices = random_indices(size);
        let mut expected_scores = vec![0f32; size as usize];
        let mut expected_max_score = f32::NEG_INFINITY;
        for i in 0..size as usize {
            expected_scores[i] = scorer.score(indices[i])?;
            expected_max_score = expected_max_score.max(expected_scores[i]);
        }
        let mut bulk_scores = vec![0f32; size as usize];
        Assert::assert_equals_double_msg(
            None,
            expected_max_score as f64,
            scorer.bulk_score(&indices, &mut bulk_scores, size)? as f64,
            0.001,
        );
        assert_array_equals_f32(&expected_scores, &bulk_scores, delta);
        assert_no_score_beyond_num_nodes(&mut *scorer, size)
    }

    /// score through the supplier/updatableScorer interface
    ///
    /// Port of `TestFlatVectorScorer.assertBulkEqualsNonBulkSupplier`.
    fn assert_bulk_equals_non_bulk_supplier(
        &self,
        values: &Values,
        sim: VectorSimilarityFunction,
    ) -> Result<()> {
        let size = values.size();
        let delta = 1e-3f32 * size as f32;
        let supplier = Self::supplier(&*self.flat_vectors_scorer, values, sim)?;
        let copy = supplier.copy()?;
        for ss in [supplier, copy] {
            let mut updatable_scorer = ss.scorer()?;
            let target_node = random().next_int_bound(size);
            updatable_scorer.set_scoring_ordinal(target_node)?;
            let indices = random_indices(size);
            let mut expected_scores = vec![0f32; size as usize];
            for i in 0..size as usize {
                expected_scores[i] = updatable_scorer.score(indices[i])?;
            }
            let mut bulk_scores = vec![0f32; size as usize];
            updatable_scorer.bulk_score(&indices, &mut bulk_scores, size)?;
            assert_array_equals_f32(&expected_scores, &bulk_scores, delta);
            assert_no_score_beyond_num_nodes(&mut *updatable_scorer, size)?;
        }
        Ok(())
    }

    /// asserts scores against the default scorer.
    ///
    /// Port of `TestFlatVectorScorer.assertScoresAgainstDefaultFlatScorer`.
    fn assert_scores_against_default_flat_scorer(
        &self,
        values: &Values,
        sim: VectorSimilarityFunction,
    ) -> Result<()> {
        let size = values.size();
        let delta = 1e-3f32 * size as f32;
        let target_node = random().next_int_bound(size);
        let indices = random_indices(size);
        let mut default_scorer =
            Self::supplier(&DefaultFlatVectorScorer::INSTANCE, values, sim)?.scorer()?;
        default_scorer.set_scoring_ordinal(target_node)?;
        let mut expected_scores = vec![0f32; size as usize];
        let mut expected_max_score = f32::NEG_INFINITY;
        for i in 0..size as usize {
            expected_scores[i] = default_scorer.score(indices[i])?;
            expected_max_score = expected_max_score.max(expected_scores[i]);
        }

        let supplier = Self::supplier(&*self.flat_vectors_scorer, values, sim)?;
        let copy = supplier.copy()?;
        for ss in [supplier, copy] {
            let mut updatable_scorer = ss.scorer()?;
            updatable_scorer.set_scoring_ordinal(target_node)?;
            let mut bulk_scores = vec![0f32; size as usize];
            Assert::assert_equals_double_msg(
                None,
                expected_max_score as f64,
                updatable_scorer.bulk_score(&indices, &mut bulk_scores, size)? as f64,
                0.001,
            );
            assert_array_equals_f32(&expected_scores, &bulk_scores, delta);
        }
        Ok(())
    }

    /// `byteVectorValues(dims, size, in, sim)`.
    fn byte_vector_values(
        &self,
        dims: i32,
        size: i32,
        r#in: &dyn IndexInput,
        sim: VectorSimilarityFunction,
    ) -> Result<Box<dyn ByteVectorValues>> {
        Ok(new_dense_off_heap_byte_vector_values(
            dims,
            size,
            r#in.slice("byteValues", 0, r#in.length())?,
            dims,
            Arc::clone(&self.flat_vectors_scorer),
            sim,
        ))
    }

    /// `floatVectorValues(dims, size, in, sim)`.
    fn float_vector_values(
        &self,
        dims: i32,
        size: i32,
        r#in: &dyn IndexInput,
        sim: VectorSimilarityFunction,
    ) -> Result<Box<dyn FloatVectorValues>> {
        Ok(new_dense_off_heap_float_vector_values(
            dims,
            size,
            r#in.slice("floatValues", 0, r#in.length())?,
            dims * 4,
            Arc::clone(&self.flat_vectors_scorer),
            sim,
        ))
    }
}

/// Port of `TestFlatVectorScorer.assertNoScoreBeyondNumNodes`.
fn assert_no_score_beyond_num_nodes(
    scorer: &mut dyn RandomVectorScorer,
    max_size: i32,
) -> Result<()> {
    let num_nodes = random().next_int_range(0, max_size);
    let indices = vec![0i32; num_nodes as usize + 1];
    let mut bulk_scores = vec![0f32; num_nodes as usize + 1];
    let last = bulk_scores.len() - 1;
    bulk_scores[last] = f32::NAN;
    scorer.bulk_score(&indices, &mut bulk_scores, num_nodes)?;
    Assert::assert_equals_float_msg(None, f32::NAN, bulk_scores[last], 0.0);
    Ok(())
}

/// `randomByteVector(int dims)`.
fn random_byte_vector(dims: i32) -> Vec<u8> {
    let mut ba = vec![0u8; dims as usize];
    random().next_bytes(&mut ba);
    ba
}

/// `randomFloatVector(int dims)`.
fn random_float_vector(dims: i32) -> Vec<f32> {
    let mut fa = vec![0f32; dims as usize];
    for f in fa.iter_mut() {
        *f = random().next_float();
    }
    fa
}

/// Concatenates float arrays as byte[].
///
/// Port of `TestFlatVectorScorer.concat(float[]...)`.
pub fn concat_floats(arrays: &[&[f32]]) -> Vec<u8> {
    let mut baos = Vec::new();
    for fa in arrays {
        for f in fa.iter() {
            baos.extend_from_slice(&f.to_le_bytes());
        }
    }
    baos
}

/// Concatenates byte arrays.
///
/// Port of `TestFlatVectorScorer.concat(byte[]...)`.
pub fn concat_bytes(arrays: &[&[u8]]) -> Vec<u8> {
    let mut baos = Vec::new();
    for ba in arrays {
        baos.extend_from_slice(ba);
    }
    baos
}

/// Returns an int[] of the given size with valued from 0 to size shuffled.
///
/// Port of `TestFlatVectorScorer.randomIndices`.
pub fn random_indices(size: i32) -> Vec<i32> {
    let mut list: Vec<i32> = (0..size).collect();
    shuffle(&mut list, &*random());
    list
}
