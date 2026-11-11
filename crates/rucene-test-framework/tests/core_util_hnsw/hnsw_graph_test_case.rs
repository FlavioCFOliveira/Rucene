//! Port of `lucene/core/src/test/org/apache/lucene/util/hnsw/HnswGraphTestCase.java`:
//! tests HNSW KNN graphs, for each vector encoding.
//!
//! The abstract generic class `HnswGraphTestCase<T>` is the trait
//! [`HnswCase`] (its abstract methods, with `T` as [`HnswCase::V`]) plus the
//! per-test instance [`Fixture`] (its fields); each `public void test*()` is
//! a generic function over the case, and [`methods`] declares them for a
//! concrete subclass.
//!
//! Test-plumbing renderings (no behaviour of the code under test changes):
//! * Java's casts `(ByteVectorValues) vectors` / `(T) …` are the typed
//!   associated type [`HnswCase::Values`]; the `(MockVectorValues)` cast of
//!   `vectorValues(int, int, KnnVectorValues, int)` is its parameter type
//!   [`HnswCase::Mock`], which every caller passes.
//! * `buildScorerSupplier(vectors)` hands Java's object itself to the
//!   supplier; the supplier owns a box here, so it receives a handle over
//!   the shared object ([`SharedFloatValues`](super::mock_vector_values::SharedFloatValues)).
//! * `OnHeapHnswGraph.seek` mutates the graph's own cursor, which needs
//!   `&mut`; a graph a builder still owns (`builder.getGraph()`) is walked
//!   through [`OnHeapCursor`], which seeks exactly as `OnHeapHnswGraph.seek`.
//!   A graph `build` returned is taken from the builder with `into_graph`
//!   once the builder is no longer used.
//! * `prettyPrint` appends `hnsw.toString()`: `OnHeapHnswGraph` overrides it
//!   (its `Display`); an off-heap graph prints `Object.toString`, rendered
//!   as its Java class name and the address.
//! * `exec.submit(callable)` / `future.get(10, SECONDS)`: [`submit`] and
//!   [`Future::get`] over the port's `ThreadPoolExecutor`.
//! * The anonymous `AbstractKnnCollector` of `testFindAll` returns `null`
//!   from `topDocs()`; Rust's `top_docs` returns a value, so it returns an
//!   empty `TopDocs` (never called).

use std::collections::HashSet;
use std::marker::PhantomData;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::junit::Before;
use randomizedtesting::RandomizedTest;
use rucene::codecs::hnsw::flat_vectors::DefaultFlatVectorScorer;
use rucene::codecs::hnsw::graph_provider::HnswGraphProvider;
use rucene::codecs::knn_vectors::KnnVectorsReader;
use rucene::codecs::lucene99::hnsw_vectors_format::{
    Lucene99HnswVectorsFormat, Lucene99HnswVectorsReader,
};
use rucene::document::{
    Document, NumericDocValuesField, NumericValue, Store, StoredField, StringField,
};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::terms::Term;
use rucene::index::{
    IndexReader, IndexableField, KnnVectorValues, LeafReader, VectorEncoding,
    VectorSimilarityFunction,
};
use rucene::search::sort::{Sort, SortField, SortFieldType};
use rucene::search::{
    doc_id_set_iterator, empty_top_docs, AbstractKnnCollector, AbstractKnnCollectorState, Executor,
    IndexSearcher, KnnCollector, KnnSearchStrategy, Query, TaskExecutor, TopDocs, TopKnnCollector,
    NO_MORE_DOCS,
};
use rucene::util::bit_set::bit_set_of;
use rucene::util::concurrent::NamedThreadFactory;
use rucene::util::hnsw::{
    HnswBuilder, HnswConcurrentMergeBuilder, HnswGraph, HnswGraphBuilder, HnswGraphSearcher,
    InitializedHnswGraphBuilder, NeighborQueue, NodesIterator, OnHeapHnswGraph, RandomVectorScorer,
    RandomVectorScorerSupplier, UpdateGraphsUtils,
};
use rucene::util::{
    vector_util, Bits, BytesRef, FixedBitSet, MatchAllBits, RamUsageEstimator, SizedObject,
};
use rucene_test_framework::index::base_field_info_format_test_case::VECTOR_SIMILARITY_FUNCTION_VALUES;
use rucene_test_framework::jdk::thread_pool_executor::ThreadPoolExecutor;
use rucene_test_framework::util::java_exceptions::{expect_throws, rethrow_java};
use rucene_test_framework::util::ram_usage_tester::{ModelBuilder, RamUsageTester};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{
    assert_array_equals_msg, assert_equals, assert_equals_delta, assert_equals_msg,
    assert_not_equals, assert_not_null, random, JavaHashSet,
};

/// The abstract members of `HnswGraphTestCase<T>` a concrete subclass
/// supplies, with `T` as [`V`](Self::V).
pub trait HnswCase: Sized + Send + 'static {
    /// `T`: `float[]` or `byte[]`.
    type V: Clone + std::fmt::Debug + PartialEq + Send + Sync + 'static;
    /// The typed vector values `T` is read from (`FloatVectorValues` or
    /// `ByteVectorValues`).
    type Values: ?Sized + KnnVectorValues + 'static;
    /// The covariant return type of the subclass's `vectorValues` overrides
    /// (`MockVectorValues` or `MockByteVectorValues`).
    type Mock: KnnVectorValues + CopyMock + 'static;

    /// The Java class name of the subclass.
    const CLASS_NAME: &'static str;

    /// `getVectorEncoding()`: Java switches on it where this port's typed
    /// [`Values`](Self::Values) already selects the encoding.
    #[allow(dead_code)]
    fn get_vector_encoding() -> VectorEncoding;
    /// `knnQuery(field, vector, k)`.
    fn knn_query(field: &str, vector: &Self::V, k: i32) -> Result<Arc<dyn Query>>;
    /// `randomVector(dim)`.
    fn random_vector(dim: i32) -> Self::V;
    /// `vectorValues(size, dimension)`.
    fn vector_values(size: i32, dimension: i32) -> Arc<Self::Mock>;
    /// `vectorValues(float[][] values)`.
    fn vector_values_from(values: &[Vec<f32>]) -> Arc<Self::Mock>;
    /// `vectorValues(LeafReader reader, String fieldName)`.
    fn vector_values_reader(reader: &dyn LeafReader, field_name: &str) -> Result<Arc<Self::Mock>>;
    /// `vectorValues(size, dimension, pregeneratedVectorValues, pregeneratedOffset)`.
    fn vector_values_pregen(
        size: i32,
        dimension: i32,
        pregenerated_vector_values: &Self::Mock,
        pregenerated_offset: i32,
    ) -> Arc<Self::Mock>;
    /// `knnVectorField(name, vector, similarityFunction)`.
    fn knn_vector_field(
        name: &str,
        vector: &Self::V,
        similarity_function: VectorSimilarityFunction,
    ) -> Result<Box<dyn IndexableField>>;
    /// `circularVectorValues(nDoc)`.
    fn circular_vector_values(n_doc: i32) -> Arc<Self::Values>;
    /// `getTargetVector()`.
    fn get_target_vector() -> Self::V;

    // The casts Java's generic code makes.

    /// The upcast of a mock to the typed values.
    fn mock_values(mock: Arc<Self::Mock>) -> Arc<Self::Values>;
    /// `((XVectorValues) vectors).vectorValue(ord)` cast to `T`.
    fn value_of(vectors: &Self::Values, ord: i32) -> Result<Self::V>;
    /// `vectors.copy()` with its typed return.
    fn copy_values(vectors: &Self::Values) -> Result<Box<Self::Values>>;
    /// The handle over the shared object a supplier takes.
    fn share(vectors: &Arc<Self::Values>) -> Box<Self::Values>;
    /// The upcast to `KnnVectorValues`.
    fn as_knn(vectors: &Self::Values) -> &dyn KnnVectorValues;
    /// `similarityFunction.compare((X[]) a, (X[]) b)`.
    fn compare(sim: VectorSimilarityFunction, a: &Self::V, b: &Self::V) -> Result<f32>;
    /// `flatVectorScorer.getRandomVectorScorerSupplier(sim, vectors)`.
    fn supplier(
        flat: &DefaultFlatVectorScorer,
        sim: VectorSimilarityFunction,
        vectors: Box<Self::Values>,
    ) -> Result<Box<dyn RandomVectorScorerSupplier>>;
    /// `flatVectorScorer.getRandomVectorScorer(sim, vectors, (X[]) query)`.
    fn scorer(
        flat: &DefaultFlatVectorScorer,
        sim: VectorSimilarityFunction,
        vectors: Box<Self::Values>,
        query: &Self::V,
    ) -> Result<Box<dyn RandomVectorScorer>>;
    /// The `assertArrayEquals` of `assertVectorsEqual` for this encoding.
    fn assert_vector_equals(message: &str, expected: &Self::V, actual: &Self::V);
}

/// An instance of a concrete `HnswGraphTestCase` subclass: the fields of the
/// abstract class.
pub struct Fixture<C: HnswCase> {
    /// `VectorSimilarityFunction similarityFunction` (`null` until `setup()`).
    pub similarity_function: Option<VectorSimilarityFunction>,
    /// `DefaultFlatVectorScorer flatVectorScorer = DefaultFlatVectorScorer.INSTANCE`.
    pub flat_vector_scorer: DefaultFlatVectorScorer,
    _case: PhantomData<fn() -> C>,
}

impl<C: HnswCase> Fixture<C> {
    fn new() -> Self {
        Fixture {
            similarity_function: None,
            flat_vector_scorer: DefaultFlatVectorScorer::INSTANCE,
            _case: PhantomData,
        }
    }

    /// Reads `similarityFunction`; a `null` one dereferenced is Java's
    /// `NullPointerException`.
    pub fn sim(&self) -> VectorSimilarityFunction {
        self.similarity_function.unwrap_or_else(|| {
            Throwable::new("java.lang.NullPointerException", "similarityFunction").throw()
        })
    }

    /// `buildScorerSupplier(vectors)` (`protected`).
    pub fn build_scorer_supplier(
        &self,
        vectors: &Arc<C::Values>,
    ) -> Result<Box<dyn RandomVectorScorerSupplier>> {
        C::supplier(&self.flat_vector_scorer, self.sim(), C::share(vectors))
    }

    /// `buildScorer(vectors, query)` (`protected`).
    pub fn build_scorer(
        &self,
        vectors: &C::Values,
        query: &C::V,
    ) -> Result<Box<dyn RandomVectorScorer>> {
        build_scorer_with::<C>(&self.flat_vector_scorer, self.sim(), vectors, query)
    }
}

/// `buildScorer` over explicit state, for the pool threads of
/// `testOnHeapHnswGraphSearch`.
fn build_scorer_with<C: HnswCase>(
    flat: &DefaultFlatVectorScorer,
    sim: VectorSimilarityFunction,
    vectors: &C::Values,
    query: &C::V,
) -> Result<Box<dyn RandomVectorScorer>> {
    let vectors_copy = C::copy_values(vectors)?;
    C::scorer(flat, sim, vectors_copy, query)
}

/// `setup()` (`@Before`) of both subclasses:
/// `similarityFunction = RandomizedTest.randomFrom(VectorSimilarityFunction.values())`.
fn setup<C: HnswCase>(this: &mut Fixture<C>) {
    this.similarity_function = Some(*RandomizedTest::random_from(
        &VECTOR_SIMILARITY_FUNCTION_VALUES,
    ));
}

/// A `public void testX()` method over the fixture.
pub fn ti<C: HnswCase>(name: &str, body: fn(&mut Fixture<C>) -> Result<()>) -> Method {
    Method::new(name)
        .throws(&["java.lang.Exception"])
        .body(move |inst: &mut Fixture<C>| body(inst).unwrap_or_else(|e| rethrow_java(e)))
}

/// The methods `HnswGraphTestCase` declares, for the subclass `C`.
pub fn methods<C: HnswCase>() -> Vec<Method> {
    vec![
        ti::<C>(
            "testRandomReadWriteAndMerge",
            test_random_read_write_and_merge::<C>,
        ),
        ti::<C>(
            "testGraphMergeWithDeletes",
            test_graph_merge_with_deletes::<C>,
        ),
        ti::<C>("testReadWrite", test_read_write::<C>),
        ti::<C>(
            "testSortedAndUnsortedIndicesReturnSameResults",
            test_sorted_and_unsorted_indices_return_same_results::<C>,
        ),
        ti::<C>("testAknnDiverse", test_aknn_diverse::<C>),
        ti::<C>(
            "testSearchWithAcceptOrds",
            test_search_with_accept_ords::<C>,
        ),
        ti::<C>(
            "testSearchWithSelectiveAcceptOrds",
            test_search_with_selective_accept_ords::<C>,
        ),
        ti::<C>("testBuildingJoinSet", test_building_join_set::<C>),
        ti::<C>(
            "testHnswGraphBuilderInitializationFromGraph_withOffsetZero",
            test_hnsw_graph_builder_initialization_from_graph_with_offset_zero::<C>,
        ),
        ti::<C>(
            "testHnswGraphBuilderInitializationFromGraph_withNonZeroOffset",
            test_hnsw_graph_builder_initialization_from_graph_with_non_zero_offset::<C>,
        ),
        ti::<C>("testVisitedLimit", test_visited_limit::<C>),
        ti::<C>("testFindAll", test_find_all::<C>),
        ti::<C>(
            "testHnswGraphBuilderInvalid",
            test_hnsw_graph_builder_invalid::<C>,
        ),
        ti::<C>("testRamUsageEstimate", test_ram_usage_estimate::<C>),
        ti::<C>("testDiversity", test_diversity::<C>),
        ti::<C>("testDiversityFallback", test_diversity_fallback::<C>),
        ti::<C>("testDiversity3d", test_diversity3d::<C>),
        ti::<C>("testRandom", test_random::<C>),
        ti::<C>(
            "testOnHeapHnswGraphSearch",
            test_on_heap_hnsw_graph_search::<C>,
        ),
        ti::<C>(
            "testConcurrentMergeBuilder",
            test_concurrent_merge_builder::<C>,
        ),
        ti::<C>(
            "testAllNodesVisitedInSingleLevel",
            test_all_nodes_visited_in_single_level::<C>,
        ),
    ]
}

/// The class of the subclass `C` (`public class … extends
/// HnswGraphTestCase<T>`): its `@Before setup()`, the inherited methods and
/// `extra`, the subclass's own.
pub fn class_of<C: HnswCase>(extra: Vec<Method>) -> Arc<Class> {
    let mut builder = Class::builder(C::CLASS_NAME)
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(Fixture::<C>::new))
        .method(Method::new("setup").annotate(Before).body(setup::<C>));
    for m in methods::<C>().into_iter().chain(extra) {
        builder = builder.method(m);
    }
    builder.build()
}

/// `try (resource) { body }`: runs `body`, then `close`, and reports the
/// body's failure first.
fn closing<T>(body: Result<T>, close: Result<()>) -> Result<T> {
    let value = body?;
    close?;
    Ok(value)
}

/// `Integer.toString(i)` as an indexed term.
fn id_term(i: i32) -> Term {
    Term::new("id", BytesRef::new(i.to_string().into_bytes()))
}

/// `HnswGraphBuilder.create(scorerSupplier, M, beamWidth, seed)`: the graph
/// size is `-1`.
fn create(
    scorer_supplier: &dyn RandomVectorScorerSupplier,
    m: i32,
    beam_width: i32,
    seed: i64,
) -> Result<HnswGraphBuilder> {
    HnswGraphBuilder::create(scorer_supplier, m, beam_width, seed as u64, -1)
}

/// `TestUtil.alwaysKnnVectorsFormat(format)` as an `IndexWriterConfig` codec.
fn hnsw_config(format: Lucene99HnswVectorsFormat) -> Result<IndexWriterConfig> {
    let iwc = IndexWriterConfig::new();
    iwc.set_codec(TestUtil::always_knn_vectors_format(Arc::new(format)))?;
    Ok(iwc)
}

/// Tests writing segments of various sizes and merging to ensure there are
/// no errors in the HNSW graph merging logic.
fn test_random_read_write_and_merge<C: HnswCase>(this: &mut Fixture<C>) -> Result<()> {
    let dim = random().next_int_bound(100) + 1;
    let segment_sizes = [
        random().next_int_bound(20) + 1,
        random().next_int_bound(10) + 30,
        random().next_int_bound(10) + 20,
    ];
    // Randomly delete vector documents
    let add_deletes = [
        random().next_boolean(),
        random().next_boolean(),
        random().next_boolean(),
    ];
    // randomly index other documents besides vector docs
    let is_sparse = [
        random().next_boolean(),
        random().next_boolean(),
        random().next_boolean(),
    ];
    let num_vectors = segment_sizes[0] + segment_sizes[1] + segment_sizes[2];
    let m = random().next_int_bound(4) + 2;
    let beam_width = random().next_int_bound(10) + 5;
    let seed = random().next_long();
    let vectors = C::mock_values(C::vector_values(num_vectors, dim));
    HnswGraphBuilder::set_rand_seed(seed as u64);

    let dir = LuceneTestCase::new_directory();
    let outcome = (|| -> Result<()> {
        let iwc = hnsw_config(Lucene99HnswVectorsFormat::with_max_conn_beam_width(
            m, beam_width,
        ))?;
        // set a random merge policy
        iwc.set_merge_policy(Some(LuceneTestCase::new_merge_policy_with(&*random())))?;
        let iw = IndexWriter::new(dir.as_directory(), iwc)?;
        let written = (|| -> Result<()> {
            for i in 0..segment_sizes.len() {
                let size = segment_sizes[i];
                for ord in 0..size {
                    if is_sparse[i] && random().next_boolean() {
                        let d = random().next_int_bound(10) + 1;
                        for _ in 0..d {
                            let doc = Document::new();
                            iw.add_document(&doc)?;
                        }
                    }
                    let mut doc = Document::new();
                    doc.add(C::knn_vector_field(
                        "field",
                        &C::value_of(&vectors, ord)?,
                        this.sim(),
                    )?);
                    doc.add(Box::new(StringField::new(
                        "id",
                        C::as_knn(&vectors).ord_to_doc(ord).to_string(),
                        Store::NO,
                    )?));
                    iw.add_document(&doc)?;
                }
                iw.commit()?;
                if add_deletes[i] && size > 1 {
                    let mut d = 0;
                    while d < size {
                        iw.delete_documents_by_terms(vec![id_term(d)])?;
                        d += random().next_int_bound(5) + 1;
                    }
                    iw.commit()?;
                }
            }
            iw.commit()?;
            iw.force_merge(1)?;
            Ok(())
        })();
        closing(written, iw.close())?;
        let reader: Arc<dyn IndexReader> = directory_reader::open(dir.as_directory())?;
        let read = (|| -> Result<()> {
            for ctx in Arc::clone(&reader).leaves()? {
                let values = C::vector_values_reader(&*ctx.leaf_reader(), "field")?;
                assert_equals(&dim, &values.dimension());
            }
            Ok(())
        })();
        closing(read, reader.close())
    })();
    closing(outcome, dir.close())
}

/// The vectors reader of a leaf, `((CodecReader) ctx.reader()).getVectorReader()`.
fn vector_reader(leaf: Arc<dyn LeafReader>) -> Result<Option<Arc<dyn KnnVectorsReader>>> {
    let codec_reader = leaf.as_codec_reader().unwrap_or_else(|| {
        Throwable::new("java.lang.ClassCastException", "not a CodecReader").throw()
    });
    codec_reader.get_vector_reader()
}

/// `maybeReader instanceof HnswGraphProvider provider`: the providers of
/// Lucene core, a declared list (`Lucene99HnswVectorsReader`).
fn as_graph_provider(reader: &dyn KnnVectorsReader) -> Option<&dyn HnswGraphProvider> {
    reader
        .as_any()
        .and_then(|a| a.downcast_ref::<Lucene99HnswVectorsReader>())
        .map(|r| r as &dyn HnswGraphProvider)
}

fn test_graph_merge_with_deletes<C: HnswCase>(this: &mut Fixture<C>) -> Result<()> {
    let m = 4;
    let beam_width = 20;
    let vector_field_name = "vec1";
    let num_vectors = random().next_int_bound(1000) + 1;
    let deletion_probaility = random().next_int_bound(100);
    let mut dim = random().next_int_bound(64) + 1;
    if dim % 2 == 1 {
        dim += 1;
    }
    let vectors = C::mock_values(C::vector_values(num_vectors, dim));
    let mut delete_count = 0;

    let dir = LuceneTestCase::new_directory();
    let outcome = (|| -> Result<()> {
        let iwc = hnsw_config(
            Lucene99HnswVectorsFormat::with_max_conn_beam_width_threshold(m, beam_width, 0),
        )?;
        // set a random merge policy
        iwc.set_merge_policy(Some(LuceneTestCase::new_merge_policy_with(&*random())))?;
        let w = IndexWriter::new(dir.as_directory(), iwc)?;
        let written = (|| -> Result<()> {
            for i in 0..num_vectors {
                let mut doc = Document::new();
                doc.add(C::knn_vector_field(
                    vector_field_name,
                    &C::value_of(&vectors, i)?,
                    this.sim(),
                )?);
                doc.add(Box::new(StringField::new("id", i.to_string(), Store::NO)?));
                w.add_document(&doc)?;
            }
            w.commit()?;

            for d in 0..num_vectors {
                if random().next_int_bound(100) < deletion_probaility {
                    delete_count += 1;
                    w.delete_documents_by_terms(vec![id_term(d)])?;
                }
            }
            w.commit()?;
            w.force_merge(1)?;
            Ok(())
        })();
        closing(written, w.close())?;

        let reader: Arc<dyn IndexReader> = directory_reader::open(dir.as_directory())?;
        let read = (|| -> Result<()> {
            for ctx in Arc::clone(&reader).leaves()? {
                let Some(vectors_reader) = vector_reader(ctx.leaf_reader())? else {
                    continue;
                };
                let mut maybe_reader: &dyn KnnVectorsReader = &*vectors_reader;
                // `maybeReader instanceof PerFieldKnnVectorsFormat.FieldsReader`
                if let Some(field_reader) = maybe_reader.get_field_reader(vector_field_name) {
                    maybe_reader = field_reader;
                }
                if let Some(provider) = as_graph_provider(maybe_reader) {
                    let graph_values = provider.get_graph(vector_field_name)?;
                    assert_equals(&(num_vectors - delete_count), &graph_values.size());
                }
            }
            Ok(())
        })();
        closing(read, reader.close())
    })();
    closing(outcome, dir.close())
}

/// `vectorValue(vectors, ord)` (`private`).
fn vector_value<C: HnswCase>(vectors: &C::Values, ord: i32) -> Result<C::V> {
    C::value_of(vectors, ord)
}

/// Test writing out and reading in a graph gives the expected graph.
fn test_read_write<C: HnswCase>(this: &mut Fixture<C>) -> Result<()> {
    let dim = random().next_int_bound(100) + 1;
    let n_doc = random().next_int_bound(100) + 1;
    let m = random().next_int_bound(4) + 2;
    let beam_width = random().next_int_bound(10) + 5;
    let seed = random().next_long();
    let vectors = C::mock_values(C::vector_values(n_doc, dim));
    let v2: Arc<C::Values> = Arc::from(C::copy_values(&vectors)?);
    let v3: Arc<C::Values> = Arc::from(C::copy_values(&vectors)?);
    let scorer_supplier = this.build_scorer_supplier(&vectors)?;
    let mut builder = create(&*scorer_supplier, m, beam_width, seed)?;
    builder.build(C::as_knn(&vectors).size())?;
    expect_throws("java.lang.IllegalStateException", || {
        builder.add_graph_node(0)
    });
    let mut hnsw = builder.into_graph()?;

    // Recreate the graph while indexing with the same random seed and write it out
    HnswGraphBuilder::set_rand_seed(seed as u64);
    let dir = LuceneTestCase::new_directory();
    let outcome = (|| -> Result<()> {
        let mut n_vec = 0;
        let mut indexed_doc = 0;
        // Don't merge randomly, create a single segment because we rely on the docid ordering for
        // this test
        let iwc = hnsw_config(
            Lucene99HnswVectorsFormat::with_max_conn_beam_width_threshold(m, beam_width, 0),
        )?;
        let iw = IndexWriter::new(dir.as_directory(), iwc)?;
        let written = (|| -> Result<()> {
            let mut it2 = C::as_knn(&v2).iterator()?;
            while it2.next_doc()? != NO_MORE_DOCS {
                while indexed_doc < it2.doc_id() {
                    // increment docId in the index by adding empty documents
                    iw.add_document(&Document::new())?;
                    indexed_doc += 1;
                }
                let mut doc = Document::new();
                doc.add(C::knn_vector_field(
                    "field",
                    &vector_value::<C>(&v2, it2.index())?,
                    this.sim(),
                )?);
                doc.add(Box::new(StoredField::new_number(
                    "id",
                    NumericValue::Int(it2.doc_id()),
                )?));
                iw.add_document(&doc)?;
                n_vec += 1;
                indexed_doc += 1;
            }
            Ok(())
        })();
        closing(written, iw.close())?;
        let reader: Arc<dyn IndexReader> = directory_reader::open(dir.as_directory())?;
        let read = (|| -> Result<()> {
            for ctx in Arc::clone(&reader).leaves()? {
                let leaf = ctx.leaf_reader();
                let values = C::mock_values(C::vector_values_reader(&*leaf, "field")?);
                assert_equals(&dim, &C::as_knn(&values).dimension());
                assert_equals(&n_vec, &C::as_knn(&values).size());
                assert_equals(&indexed_doc, &leaf.max_doc());
                assert_equals(&indexed_doc, &leaf.num_docs());
                assert_vectors_equal::<C>(&v3, &values)?;
                let vectors_reader = vector_reader(Arc::clone(&leaf))?.unwrap_or_else(|| {
                    Throwable::new("java.lang.NullPointerException", "getVectorReader()").throw()
                });
                let field_reader = vectors_reader.get_field_reader("field").unwrap_or_else(|| {
                    Throwable::new(
                        "java.lang.ClassCastException",
                        "not a PerFieldKnnVectorsFormat.FieldsReader",
                    )
                    .throw()
                });
                let hnsw_reader = field_reader
                    .as_any()
                    .and_then(|a| a.downcast_ref::<Lucene99HnswVectorsReader>())
                    .unwrap_or_else(|| {
                        Throwable::new(
                            "java.lang.ClassCastException",
                            "not a Lucene99HnswVectorsReader",
                        )
                        .throw()
                    });
                let mut graph_values = hnsw_reader.get_graph("field")?;
                let h_str = object_to_string(
                    "org.apache.lucene.codecs.lucene99.Lucene99HnswVectorsReader$OffHeapHnswGraph",
                    &*graph_values,
                );
                let g_str = hnsw.to_string();
                assert_graph_equal(&mut hnsw, &g_str, graph_values.as_mut(), &h_str)?;
            }
            Ok(())
        })();
        closing(read, reader.close())
    })();
    closing(outcome, dir.close())
}

/// `Object.toString()` of a graph whose class does not override it:
/// `getClass().getName() + "@" + hashCode`, with the address as the hash.
fn object_to_string(class_name: &str, graph: &dyn HnswGraph) -> String {
    format!(
        "{class_name}@{:x}",
        graph as *const dyn HnswGraph as *const () as usize
    )
}

/// Test that sorted index returns the same search results as unsorted.
fn test_sorted_and_unsorted_indices_return_same_results<C: HnswCase>(
    _this: &mut Fixture<C>,
) -> Result<()> {
    let dim = random().next_int_bound(10) + 3;
    let n_doc = random().next_int_bound(200) + 100;
    let vectors = C::mock_values(C::vector_values(n_doc, dim));

    let m = random().next_int_bound(10) + 5;
    let beam_width = random().next_int_bound(10) + 10;
    let similarity_function = *RandomizedTest::random_from(&VECTOR_SIMILARITY_FUNCTION_VALUES);
    let seed = random().next_long();
    HnswGraphBuilder::set_rand_seed(seed as u64);
    let iwc = hnsw_config(Lucene99HnswVectorsFormat::with_max_conn_beam_width(
        m, beam_width,
    ))?;
    let iwc2 = hnsw_config(Lucene99HnswVectorsFormat::with_max_conn_beam_width(
        m, beam_width,
    ))?;
    iwc2.set_index_sort(Sort::new_fields(vec![SortField::new(
        Some("sortkey".to_string()),
        SortFieldType::Long,
    )?])?)?;

    let dir = LuceneTestCase::new_directory();
    let dir2 = LuceneTestCase::new_directory();
    let outcome = (|| -> Result<()> {
        let mut indexed_doc = 0;
        let iw = IndexWriter::new(dir.as_directory(), iwc)?;
        let iw2 = IndexWriter::new(dir2.as_directory(), iwc2)?;
        let written = (|| -> Result<()> {
            for ord in 0..C::as_knn(&vectors).size() {
                while indexed_doc < C::as_knn(&vectors).ord_to_doc(ord) {
                    // increment docId in the index by adding empty documents
                    iw.add_document(&Document::new())?;
                    indexed_doc += 1;
                }
                let mut doc = Document::new();
                doc.add(C::knn_vector_field(
                    "vector",
                    &vector_value::<C>(&vectors, ord)?,
                    similarity_function,
                )?);
                doc.add(Box::new(StoredField::new_number(
                    "id",
                    NumericValue::Int(C::as_knn(&vectors).ord_to_doc(ord)),
                )?));
                doc.add(Box::new(NumericDocValuesField::new(
                    "sortkey",
                    random().next_long(),
                )));
                iw.add_document(&doc)?;
                iw2.add_document(&doc)?;
                indexed_doc += 1;
            }
            Ok(())
        })();
        let closed2 = iw2.close();
        closing(closing(written, iw.close()), closed2)?;
        let reader: Arc<dyn IndexReader> = directory_reader::open(dir.as_directory())?;
        let reader2: Arc<dyn IndexReader> = directory_reader::open(dir2.as_directory())?;
        let read = (|| -> Result<()> {
            let searcher = IndexSearcher::new(Arc::clone(&reader))?;
            let searcher2 = IndexSearcher::new(Arc::clone(&reader2))?;
            'outer: for _ in 0..10 {
                // ask to explore a lot of candidates to ensure the same returned hits,
                // as graphs of 2 indices are organized differently
                let query = C::knn_query("vector", &C::random_vector(dim), 60)?;
                let search_size = 5;
                let mut ids1: Vec<Option<String>> = Vec::with_capacity(search_size);
                let mut docs1: Vec<i32> = Vec::with_capacity(search_size);
                let mut ids2: Vec<Option<String>> = Vec::with_capacity(search_size);
                let mut docs2: Vec<i32> = Vec::with_capacity(search_size);

                // Check if a duplicate score exists in n+1, if so, this test is invalid
                // Else, continue to fail on ID equality as this test failed
                let top_docs = searcher.search(Arc::clone(&query), search_size as i32 + 1)?;
                let mut last_score = -1f32;
                let stored_fields = reader.stored_fields()?;
                let fields: HashSet<String> = HashSet::from(["id".to_string()]);
                for j in 0..search_size + 1 {
                    let score_doc = &top_docs.score_docs[j];
                    if score_doc.score == last_score {
                        // if we have repeated score this test is invalid
                        continue 'outer;
                    } else {
                        last_score = score_doc.score;
                    }
                    if j < search_size {
                        let doc = stored_fields.document_fields(score_doc.doc, &fields)?;
                        ids1.push(doc.get("id"));
                        docs1.push(score_doc.doc);
                    }
                }
                let top_docs2 = searcher2.search(Arc::clone(&query), search_size as i32)?;
                let stored_fields2 = reader2.stored_fields()?;
                for score_doc in &top_docs2.score_docs {
                    let doc = stored_fields2.document_fields(score_doc.doc, &fields)?;
                    ids2.push(doc.get("id"));
                    docs2.push(score_doc.doc);
                }
                assert_equals(&ids1, &ids2);
                // doc IDs are not equal, as in the second sorted index docs are organized
                // differently
                assert_not_equals(&docs1, &docs2);
            }
            Ok(())
        })();
        let closed2 = reader2.close();
        closing(closing(read, reader.close()), closed2)
    })();
    let closed2 = dir2.close();
    closing(closing(outcome, dir.close()), closed2)
}

/// `sortedNodesOnLevel(h, level)`.
fn sorted_nodes_on_level(h: &dyn HnswGraph, level: i32) -> Result<Vec<i32>> {
    let mut nodes_on_level = h.get_nodes_on_level(level)?;
    let mut nodes = Vec::new();
    while nodes_on_level.has_next() {
        nodes.push(nodes_on_level.next_int());
    }
    nodes.sort();
    Ok(nodes)
}

/// `assertGraphEqual(g, h)`; `g_str`/`h_str` are `g.toString()`/`h.toString()`.
pub fn assert_graph_equal(
    g: &mut dyn HnswGraph,
    g_str: &str,
    h: &mut dyn HnswGraph,
    h_str: &str,
) -> Result<()> {
    // construct these up front since they call seek which will mess up our test loop
    let pretty_g = pretty_print(g, g_str);
    let pretty_h = pretty_print(h, h_str);
    assert_equals_msg(
        Some(&format!(
            "the number of levels in the graphs are different:\n{pretty_g}\n{pretty_h}"
        )),
        &g.num_levels()?,
        &h.num_levels()?,
    );
    assert_equals_msg(
        Some(&format!(
            "the number of nodes in the graphs are different:\n{pretty_g}\n{pretty_h}"
        )),
        &g.size(),
        &h.size(),
    );

    // assert equal nodes on each level
    for level in 0..g.num_levels()? {
        let h_nodes = sorted_nodes_on_level(h, level)?;
        let g_nodes = sorted_nodes_on_level(g, level)?;
        assert_equals_msg(
            Some(&format!(
                "nodes in the graphs are different on level {level}:\n{pretty_g}\n{pretty_h}"
            )),
            &g_nodes,
            &h_nodes,
        );
    }

    // assert equal nodes' neighbours on each level
    for level in 0..g.num_levels()? {
        let mut nodes_on_level = g.get_nodes_on_level(level)?;
        while nodes_on_level.has_next() {
            let node = nodes_on_level.next_int();
            g.seek(level, node)?;
            h.seek(level, node)?;
            assert_equals_msg(
                Some(&format!(
                    "arcs differ for node {node} on level {level}:\n{pretty_g}\n{pretty_h}"
                )),
                &get_neighbor_nodes(g)?,
                &get_neighbor_nodes(h)?,
            );
        }
    }
    Ok(())
}

/// Make sure we actually approximately find the closest k elements. Mostly
/// this is about ensuring that we have all the distance functions,
/// comparators, priority queues and so on oriented in the right directions.
fn test_aknn_diverse<C: HnswCase>(this: &mut Fixture<C>) -> Result<()> {
    let n_doc = 100;
    this.similarity_function = Some(VectorSimilarityFunction::DOT_PRODUCT);
    let vectors = C::circular_vector_values(n_doc);
    let scorer_supplier = this.build_scorer_supplier(&vectors)?;
    let mut builder = create(&*scorer_supplier, 10, 100, random().next_int() as i64)?;
    builder.build(C::as_knn(&vectors).size())?;
    let hnsw = builder.into_graph()?;

    // run some searches
    let mut nn = HnswGraphSearcher::search_on_heap(
        this.build_scorer(&vectors, &C::get_target_vector())?
            .as_mut(),
        10,
        &hnsw,
        None,
        i32::MAX as i64,
    )?;
    let top_docs = nn.top_docs()?;
    assert_equals_msg(
        Some("Number of found results is not equal to [10]."),
        &10,
        &top_docs.score_docs.len(),
    );
    let mut sum = 0;
    for node in &top_docs.score_docs {
        sum += node.doc;
    }
    // We expect to get approximately 100% recall;
    // the lowest docIds are closest to zero; sum(0,9) = 45
    assert_true(&format!("sum(result docs)={sum}"), sum < 75);

    for i in 0..n_doc {
        let neighbors = hnsw.get_neighbors(0, i)?;
        let nnodes = neighbors.nodes();
        for j in 0..neighbors.size() {
            // all neighbors should be valid node ids.
            assert_true("", nnodes[j as usize] < n_doc);
        }
    }
    Ok(())
}

/// `assertTrue(message, condition)`.
fn assert_true(message: &str, condition: bool) {
    randomizedtesting::junit::Assert::assert_true_msg(message, condition);
}

fn test_search_with_accept_ords<C: HnswCase>(this: &mut Fixture<C>) -> Result<()> {
    let n_doc = 100;
    let vectors = C::circular_vector_values(n_doc);
    this.similarity_function = Some(VectorSimilarityFunction::DOT_PRODUCT);
    let scorer_supplier = this.build_scorer_supplier(&vectors)?;
    let mut builder = create(&*scorer_supplier, 16, 100, random().next_int() as i64)?;
    builder.build(C::as_knn(&vectors).size())?;
    let hnsw = builder.into_graph()?;
    // the first 10 docs must not be deleted to ensure the expected recall
    let accept_ords = create_random_accept_ords(10, n_doc);
    let mut nn = HnswGraphSearcher::search_on_heap(
        this.build_scorer(&vectors, &C::get_target_vector())?
            .as_mut(),
        10,
        &hnsw,
        Some(&accept_ords),
        i32::MAX as i64,
    )?;
    let nodes = nn.top_docs()?;
    assert_equals_msg(
        Some("Number of found results is not equal to [10]."),
        &10,
        &nodes.score_docs.len(),
    );
    let mut sum = 0;
    for node in &nodes.score_docs {
        assert_true(
            &format!("the results include a deleted document: {node}"),
            accept_ords.get(node.doc as usize),
        );
        sum += node.doc;
    }
    // We expect to get approximately 100% recall;
    // the lowest docIds are closest to zero; sum(0,9) = 45
    assert_true(&format!("sum(result docs)={sum}"), sum < 75);
    Ok(())
}

fn test_search_with_selective_accept_ords<C: HnswCase>(this: &mut Fixture<C>) -> Result<()> {
    let n_doc = 100;
    let vectors = C::circular_vector_values(n_doc);
    this.similarity_function = Some(VectorSimilarityFunction::DOT_PRODUCT);
    let scorer_supplier = this.build_scorer_supplier(&vectors)?;
    let mut builder = create(&*scorer_supplier, 16, 100, random().next_int() as i64)?;
    builder.build(C::as_knn(&vectors).size())?;
    let hnsw = builder.into_graph()?;
    // Only mark a few vectors as accepted
    let mut accept_ords = FixedBitSet::new(n_doc as usize);
    let mut i = 0;
    while i < n_doc {
        accept_ords.set(i as usize);
        i += random().next_int_range(15, 20);
    }

    // Check the search finds all accepted vectors
    let num_accepted = accept_ords.cardinality() as i32;
    let mut nn = HnswGraphSearcher::search_on_heap(
        this.build_scorer(&vectors, &C::get_target_vector())?
            .as_mut(),
        num_accepted,
        &hnsw,
        Some(&accept_ords),
        i32::MAX as i64,
    )?;
    let nodes = nn.top_docs()?;
    assert_equals(&(num_accepted as usize), &nodes.score_docs.len());
    for node in &nodes.score_docs {
        assert_true(
            &format!("the results include a deleted document: {node}"),
            accept_ords.get(node.doc as usize),
        );
    }
    Ok(())
}

fn test_building_join_set<C: HnswCase>(this: &mut Fixture<C>) -> Result<()> {
    let dim = random().next_int_bound(100) + 1;
    let n_doc = random().next_int_bound(500) + 100;
    let m = random().next_int_bound(16) + 16;
    let beam_width = random().next_int_bound(100) + 16;
    let seed = random().next_long();
    let vectors = C::mock_values(C::vector_values(n_doc, dim));
    let scorer_supplier = this.build_scorer_supplier(&vectors)?;
    let mut builder = create(&*scorer_supplier, m, beam_width, seed)?;
    builder.build(C::as_knn(&vectors).size())?;
    let mut graph = builder.into_graph()?;

    let j = UpdateGraphsUtils::compute_join_set(&mut graph)?;
    assert_true(
        &format!(
            "Join set size [{}] is not less than graph size [{}]",
            j.size(),
            graph.size()
        ),
        j.size() < graph.size(),
    );
    Ok(())
}

/// `BitSet.of(DocIdSetIterator.range(docIdOffset, initializerSize +
/// docIdOffset), totalSize + 1)` as the `FixedBitSet` the port's
/// `InitializedHnswGraphBuilder::from_graph` takes: the same bits (Java may
/// choose a `SparseFixedBitSet`; see the pre-existing divergence of that API).
fn initialized_bits(
    doc_id_offset: i32,
    initializer_size: i32,
    max_doc: i32,
) -> Result<FixedBitSet> {
    let mut range = doc_id_set_iterator::range(doc_id_offset, initializer_size + doc_id_offset)?;
    let set = bit_set_of(&mut range, max_doc)?;
    let mut fixed = FixedBitSet::new(max_doc as usize);
    for i in 0..max_doc as usize {
        if set.get(i) {
            fixed.set(i);
        }
    }
    Ok(fixed)
}

fn test_hnsw_graph_builder_initialization_from_graph_with_offset_zero<C: HnswCase>(
    this: &mut Fixture<C>,
) -> Result<()> {
    let total_size = LuceneTestCase::at_least(100);
    let initializer_size = random().next_int_range(5, total_size);
    let doc_id_offset = 0;
    let dim = LuceneTestCase::at_least(10);
    let beam_width = 30;
    let seed = random().next_long();

    let initializer_mock = C::vector_values(initializer_size, dim);
    let initializer_vectors = C::mock_values(Arc::clone(&initializer_mock));
    let initialscorer_supplier = this.build_scorer_supplier(&initializer_vectors)?;
    let mut initializer_builder = create(&*initialscorer_supplier, 10, beam_width, seed)?;

    initializer_builder.build(C::as_knn(&initializer_vectors).size())?;
    let mut initializer_graph = initializer_builder.into_graph()?;
    let final_vector_values = C::mock_values(C::vector_values_pregen(
        total_size,
        dim,
        &initializer_mock,
        doc_id_offset,
    ));
    let initializer_ord_map = create_offset_ordinal_map(
        initializer_size,
        C::as_knn(&final_vector_values),
        doc_id_offset,
    )?;

    let finalscorer_supplier = this.build_scorer_supplier(&final_vector_values)?;

    // we cannot call getNodesOnLevel before the graph reaches the size it claimed, so here we
    // create
    // another graph to do the assertion
    let initializer_graph_size = initializer_graph.size();
    let mut graph_after_init = InitializedHnswGraphBuilder::init_graph(
        &mut initializer_graph,
        &initializer_ord_map,
        initializer_graph_size,
        beam_width,
        &*initialscorer_supplier,
    )?;

    let mut final_builder = InitializedHnswGraphBuilder::from_graph(
        &*finalscorer_supplier,
        30,
        seed as u64,
        &mut initializer_graph,
        &initializer_ord_map,
        Some(initialized_bits(
            doc_id_offset,
            initializer_size,
            total_size + 1,
        )?),
        total_size,
    )?;

    // When offset is 0, the graphs should be identical before vectors are added
    let g_str = initializer_graph.to_string();
    let h_str = graph_after_init.to_string();
    assert_graph_equal(
        &mut initializer_graph,
        &g_str,
        &mut graph_after_init,
        &h_str,
    )?;

    final_builder.build(C::as_knn(&final_vector_values).size())?;
    let mut final_graph = Box::new(final_builder).into_completed_graph()?;
    assert_graph_contains_graph(
        &mut final_graph,
        &mut initializer_graph,
        &initializer_ord_map,
    )
}

fn test_hnsw_graph_builder_initialization_from_graph_with_non_zero_offset<C: HnswCase>(
    this: &mut Fixture<C>,
) -> Result<()> {
    let total_size = LuceneTestCase::at_least(100);
    let initializer_size = random().next_int_range(5, total_size);
    let doc_id_offset = random().next_int_range(1, total_size - initializer_size + 1);
    let dim = LuceneTestCase::at_least(10);
    let seed = random().next_long();

    let initializer_mock = C::vector_values(initializer_size, dim);
    let initializer_vectors = C::mock_values(Arc::clone(&initializer_mock));
    let initialscorer_supplier = this.build_scorer_supplier(&initializer_vectors)?;
    let mut initializer_builder = create(&*initialscorer_supplier, 10, 30, seed)?;

    initializer_builder.build(C::as_knn(&initializer_vectors).size())?;
    let mut initializer_graph = initializer_builder.into_graph()?;
    // `initializerVectors.copy()`
    let initializer_copy = initializer_mock.copy_mock_values();
    let final_vector_values = C::mock_values(C::vector_values_pregen(
        total_size,
        dim,
        &initializer_copy,
        doc_id_offset,
    ));
    let initializer_ord_map = create_offset_ordinal_map(
        initializer_size,
        C::as_knn(&final_vector_values),
        doc_id_offset,
    )?;

    let finalscorer_supplier = this.build_scorer_supplier(&final_vector_values)?;
    let mut final_builder = InitializedHnswGraphBuilder::from_graph(
        &*finalscorer_supplier,
        30,
        seed as u64,
        &mut initializer_graph,
        &initializer_ord_map,
        Some(initialized_bits(
            doc_id_offset,
            initializer_size,
            total_size + 1,
        )?),
        total_size,
    )?;

    assert_graph_initialized_from_graph(
        &mut OnHeapCursor::new(final_builder.get_graph()),
        &mut initializer_graph,
        &initializer_ord_map,
    )?;

    // Confirm that the graph is appropriately constructed by checking that the nodes in the old
    // graph are present in the levels of the new graph
    final_builder.build(C::as_knn(&final_vector_values).size())?;
    let mut final_graph = Box::new(final_builder).into_completed_graph()?;
    assert_graph_contains_graph(
        &mut final_graph,
        &mut initializer_graph,
        &initializer_ord_map,
    )
}

/// The covariant `copy()` of the mocks.
pub trait CopyMock {
    /// `copy()`.
    fn copy_mock_values(&self) -> Self;
}

fn assert_graph_contains_graph(
    g: &mut dyn HnswGraph,
    initializer: &mut dyn HnswGraph,
    new_ordinals: &[i32],
) -> Result<()> {
    for i in 0..initializer.num_levels()? {
        let mut final_graph_nodes_on_level = nodes_iterator_to_array(g.get_nodes_on_level(i)?);
        let mut initializer_graph_nodes_on_level = map_array_and_sort(
            &nodes_iterator_to_array(initializer.get_nodes_on_level(i)?),
            new_ordinals,
        );
        let overlap = compute_overlap(
            &mut final_graph_nodes_on_level,
            &mut initializer_graph_nodes_on_level,
        );
        assert_equals(&(initializer_graph_nodes_on_level.len() as i32), &overlap);
    }
    Ok(())
}

fn assert_graph_initialized_from_graph(
    g: &mut dyn HnswGraph,
    initializer: &mut dyn HnswGraph,
    new_ordinals: &[i32],
) -> Result<()> {
    assert_equals_msg(
        Some("the number of levels in the graphs are different!"),
        &initializer.num_levels()?,
        &g.num_levels()?,
    );
    // Confirm that the size of the new graph includes all nodes up to an including the max new
    // ordinal in the old to
    // new ordinal mapping
    assert_equals_msg(
        Some("the number of nodes in the graphs are different!"),
        &initializer.size(),
        &g.size(),
    );

    // assert that the neighbors from the old graph are successfully transferred to the new graph
    for level in 0..g.num_levels()? {
        let mut nodes_on_level = initializer.get_nodes_on_level(level)?;
        while nodes_on_level.has_next() {
            let node = nodes_on_level.next_int();
            g.seek(level, new_ordinals[node as usize])?;
            initializer.seek(level, node)?;
            let expected = get_neighbor_nodes(g)?;
            let mut mapped = JavaHashSet::new();
            for n in get_neighbor_nodes(initializer)?.iter() {
                mapped.add(new_ordinals[*n as usize]);
            }
            assert_equals_msg(
                Some(&format!("arcs differ for node {node}")),
                &expected,
                &mapped,
            );
        }
    }
    Ok(())
}

fn nodes_iterator_to_array(mut nodes_iterator: Box<dyn NodesIterator>) -> Vec<i32> {
    let mut arr = vec![0; nodes_iterator.size() as usize];
    let mut i = 0;
    while nodes_iterator.has_next() {
        arr[i] = nodes_iterator.next_int();
        i += 1;
    }
    arr
}

fn map_array_and_sort(arr: &[i32], offset: &[i32]) -> Vec<i32> {
    let mut mapped_a = vec![0; arr.len()];
    for i in 0..arr.len() {
        mapped_a[i] = offset[arr[i] as usize];
    }
    mapped_a.sort();
    mapped_a
}

fn create_offset_ordinal_map(
    doc_id_size: i32,
    total_vector_values: &dyn KnnVectorValues,
    doc_id_offset: i32,
) -> Result<Vec<i32>> {
    // Compute the offset for the ordinal map to be the number of non-null vectors in the total
    // vector values before the docIdOffset
    let mut ordinal_offset = 0;
    let mut it = total_vector_values.iterator()?;
    while it.next_doc()? < doc_id_offset {
        ordinal_offset += 1;
    }
    let mut offset_ordinal_map = vec![0; doc_id_size as usize];

    let mut curr = 0;
    while it.doc_id() < doc_id_offset + doc_id_size {
        offset_ordinal_map[curr as usize] = ordinal_offset + curr;
        curr += 1;
        it.next_doc()?;
    }

    Ok(offset_ordinal_map)
}

fn test_visited_limit<C: HnswCase>(this: &mut Fixture<C>) -> Result<()> {
    let n_doc = 500;
    this.similarity_function = Some(VectorSimilarityFunction::DOT_PRODUCT);
    let vectors = C::circular_vector_values(n_doc);
    let scorer_supplier = this.build_scorer_supplier(&vectors)?;
    let mut builder = create(&*scorer_supplier, 16, 100, random().next_int() as i64)?;
    builder.build(C::as_knn(&vectors).size())?;
    let hnsw = builder.into_graph()?;

    let top_k = 50;
    let visited_limit = top_k + random().next_int_bound(5);
    let accept = create_random_accept_ords(0, n_doc);
    let nn = HnswGraphSearcher::search_on_heap(
        this.build_scorer(&vectors, &C::get_target_vector())?
            .as_mut(),
        top_k,
        &hnsw,
        Some(&accept),
        visited_limit as i64,
    )?;
    assert_true("", nn.early_terminated());
    // The visited count shouldn't exceed the limit
    assert_true("", nn.visited_count() <= visited_limit as i64);
    Ok(())
}

/// The anonymous `AbstractKnnCollector` of `testFindAll`.
struct FindAllCollector {
    state: AbstractKnnCollectorState,
    collected: i32,
    final_min_score: f32,
}

impl KnnCollector for FindAllCollector {
    fn early_terminated(&self) -> bool {
        self.state.early_terminated()
    }
    fn inc_visited_count(&mut self, count: i32) {
        self.state.inc_visited_count(count)
    }
    fn visited_count(&self) -> i64 {
        self.state.visited_count()
    }
    fn visit_limit(&self) -> i64 {
        self.state.visit_limit()
    }
    fn k(&self) -> i32 {
        self.state.k()
    }
    fn collect(&mut self, _doc_id: i32, _similarity: f32) -> bool {
        self.collected += 1;
        true
    }
    fn min_competitive_similarity(&self) -> f32 {
        self.final_min_score
    }
    fn top_docs(&mut self) -> rucene::error::Result<TopDocs> {
        Ok({
            // Java returns `null`.
            empty_top_docs()
        })
    }
    fn get_search_strategy(&self) -> Option<&KnnSearchStrategy> {
        self.state.get_search_strategy()
    }
}

impl AbstractKnnCollector for FindAllCollector {
    fn num_collected(&self) -> i32 {
        self.collected
    }
}

fn test_find_all<C: HnswCase>(this: &mut Fixture<C>) -> Result<()> {
    let num_vectors = 10;
    let vector_values = C::circular_vector_values(num_vectors);
    let target = C::get_target_vector();
    let mut min_score = f32::INFINITY;
    for i in 0..num_vectors {
        let score = C::compare(this.sim(), &C::value_of(&vector_values, i)?, &target)?;
        min_score = min_score.min(score);
    }
    let scorer_supplier = this.build_scorer_supplier(&vector_values)?;
    let mut builder = create(&*scorer_supplier, 16, 100, random().next_int() as i64)?;
    builder.build(num_vectors)?;
    let mut hnsw = builder.into_graph()?;
    let final_min_score = min_score.next_down();

    let mut collector = FindAllCollector {
        state: AbstractKnnCollectorState::new(
            num_vectors,
            i64::MAX,
            Some(KnnSearchStrategy::hnsw_default()),
        ),
        collected: 0,
        final_min_score,
    };
    let accept = MatchAllBits::new(num_vectors as usize);
    HnswGraphSearcher::search_accept_ords(
        this.build_scorer(&vector_values, &target)?.as_mut(),
        &mut collector,
        &mut hnsw,
        Some(&accept),
    )?;
    assert_equals(&num_vectors, &collector.num_collected());
    Ok(())
}

fn test_hnsw_graph_builder_invalid<C: HnswCase>(this: &mut Fixture<C>) -> Result<()> {
    let scorer_supplier = this.build_scorer_supplier(&C::mock_values(C::vector_values(1, 1)))?;
    // M must be > 0
    expect_throws("java.lang.IllegalArgumentException", || {
        create(&*scorer_supplier, 0, 10, 0)
    });
    // beamWidth must be > 0
    expect_throws("java.lang.IllegalArgumentException", || {
        create(&*scorer_supplier, 10, 0, 0)
    });
    Ok(())
}

fn test_ram_usage_estimate<C: HnswCase>(this: &mut Fixture<C>) -> Result<()> {
    let size = LuceneTestCase::at_least(2000);
    let dim = RandomizedTest::random_int_between(100, 1024);
    let m = RandomizedTest::random_int_between(4, 96);

    this.similarity_function = Some(*RandomizedTest::random_from(
        &VECTOR_SIMILARITY_FUNCTION_VALUES,
    ));
    let vectors = C::mock_values(C::vector_values(size, dim));

    let scorer_supplier = this.build_scorer_supplier(&vectors)?;
    let mut builder = create(&*scorer_supplier, m, m * 2, random().next_long())?;
    builder.build(C::as_knn(&vectors).size())?;
    let hnsw = builder.into_graph()?;
    let estimated = RamUsageEstimator::size_of_object(Some(SizedObject::Accountable(&hnsw)));
    let actual = RamUsageTester::ram_used(&*ModelBuilder::new().on_heap_hnsw_graph(&hnsw));

    assert_equals_delta(actual as f64, estimated as f64, actual as f64 * 0.3);
    Ok(())
}

fn test_diversity<C: HnswCase>(this: &mut Fixture<C>) -> Result<()> {
    this.similarity_function = Some(VectorSimilarityFunction::DOT_PRODUCT);
    // Some carefully checked test cases with simple 2d vectors on the unit circle:
    let values = vec![
        unit_vector2d(0.5),
        unit_vector2d(0.75),
        unit_vector2d(0.2),
        unit_vector2d(0.9),
        unit_vector2d(0.8),
        unit_vector2d(0.77),
        unit_vector2d(0.6),
    ];
    let vectors = C::mock_values(C::vector_values_from(&values));
    // First add nodes until everybody gets a full neighbor list
    let scorer_supplier = this.build_scorer_supplier(&vectors)?;
    let mut builder = create(&*scorer_supplier, 2, 10, random().next_int() as i64)?;
    // node 0 is added by the builder constructor
    builder.add_graph_node(0)?;
    builder.add_graph_node(1)?;
    builder.add_graph_node(2)?;
    // now every node has tried to attach every other node as a neighbor, but
    // some were excluded based on diversity check.
    assert_level0_neighbors(builder.get_graph(), 0, &[1, 2])?;
    assert_level0_neighbors(builder.get_graph(), 1, &[0])?;
    assert_level0_neighbors(builder.get_graph(), 2, &[0])?;

    builder.add_graph_node(3)?;
    assert_level0_neighbors(builder.get_graph(), 0, &[1, 2])?;
    // we added 3 here
    assert_level0_neighbors(builder.get_graph(), 1, &[0, 3])?;
    assert_level0_neighbors(builder.get_graph(), 2, &[0])?;
    assert_level0_neighbors(builder.get_graph(), 3, &[1])?;

    // supplant an existing neighbor
    builder.add_graph_node(4)?;
    // 4 is the same distance from 0 that 2 is; we leave the existing node in place
    assert_level0_neighbors(builder.get_graph(), 0, &[1, 2])?;
    assert_level0_neighbors(builder.get_graph(), 1, &[0, 3, 4])?;
    assert_level0_neighbors(builder.get_graph(), 2, &[0])?;
    // 1 survives the diversity check
    assert_level0_neighbors(builder.get_graph(), 3, &[1, 4])?;
    assert_level0_neighbors(builder.get_graph(), 4, &[1, 3])?;

    builder.add_graph_node(5)?;
    assert_level0_neighbors(builder.get_graph(), 0, &[1, 2])?;
    assert_level0_neighbors(builder.get_graph(), 1, &[0, 3, 4, 5])?;
    assert_level0_neighbors(builder.get_graph(), 2, &[0])?;
    // even though 5 is closer, 3 is not a neighbor of 5, so no update to *its* neighbors occurs
    assert_level0_neighbors(builder.get_graph(), 3, &[1, 4])?;
    assert_level0_neighbors(builder.get_graph(), 4, &[1, 3, 5])?;
    assert_level0_neighbors(builder.get_graph(), 5, &[1, 4])?;
    Ok(())
}

fn test_diversity_fallback<C: HnswCase>(this: &mut Fixture<C>) -> Result<()> {
    this.similarity_function = Some(VectorSimilarityFunction::EUCLIDEAN);
    // Some test cases can't be exercised in two dimensions;
    // in particular if a new neighbor displaces an existing neighbor
    // by being closer to the target, yet none of the existing neighbors is closer to the new vector
    // than to the target -- ie they all remain diverse, so we simply drop the farthest one.
    let values = vec![
        vec![0.0, 0.0, 0.0],
        vec![0.0, 10.0, 0.0],
        vec![0.0, 0.0, 20.0],
        vec![10.0, 0.0, 0.0],
        vec![0.0, 4.0, 0.0],
    ];
    let vectors = C::mock_values(C::vector_values_from(&values));
    // First add nodes until everybody gets a full neighbor list
    let scorer_supplier = this.build_scorer_supplier(&vectors)?;
    let mut builder = create(&*scorer_supplier, 1, 10, random().next_int() as i64)?;
    builder.add_graph_node(0)?;
    builder.add_graph_node(1)?;
    builder.add_graph_node(2)?;
    assert_level0_neighbors(builder.get_graph(), 0, &[1, 2])?;
    // 2 is closer to 0 than 1, so it is excluded as non-diverse
    assert_level0_neighbors(builder.get_graph(), 1, &[0])?;
    // 1 is closer to 0 than 2, so it is excluded as non-diverse
    assert_level0_neighbors(builder.get_graph(), 2, &[0])?;

    builder.add_graph_node(3)?;
    // this is one case we are testing; 2 has been displaced by 3
    assert_level0_neighbors(builder.get_graph(), 0, &[1, 3])?;
    assert_level0_neighbors(builder.get_graph(), 1, &[0])?;
    assert_level0_neighbors(builder.get_graph(), 2, &[0])?;
    assert_level0_neighbors(builder.get_graph(), 3, &[0])?;
    Ok(())
}

fn test_diversity3d<C: HnswCase>(this: &mut Fixture<C>) -> Result<()> {
    this.similarity_function = Some(VectorSimilarityFunction::EUCLIDEAN);
    // test the case when a neighbor *becomes* non-diverse when a newer better neighbor arrives
    let values = vec![
        vec![0.0, 0.0, 0.0],
        vec![0.0, 10.0, 0.0],
        vec![0.0, 0.0, 20.0],
        vec![0.0, 9.0, 0.0],
    ];
    let vectors = C::mock_values(C::vector_values_from(&values));
    // First add nodes until everybody gets a full neighbor list
    let scorer_supplier = this.build_scorer_supplier(&vectors)?;
    let mut builder = create(&*scorer_supplier, 1, 10, random().next_int() as i64)?;
    builder.add_graph_node(0)?;
    builder.add_graph_node(1)?;
    builder.add_graph_node(2)?;
    assert_level0_neighbors(builder.get_graph(), 0, &[1, 2])?;
    // 2 is closer to 0 than 1, so it is excluded as non-diverse
    assert_level0_neighbors(builder.get_graph(), 1, &[0])?;
    // 1 is closer to 0 than 2, so it is excluded as non-diverse
    assert_level0_neighbors(builder.get_graph(), 2, &[0])?;

    builder.add_graph_node(3)?;
    // this is one case we are testing; 1 has been displaced by 3
    assert_level0_neighbors(builder.get_graph(), 0, &[2, 3])?;
    assert_level0_neighbors(builder.get_graph(), 1, &[0, 3])?;
    assert_level0_neighbors(builder.get_graph(), 2, &[0])?;
    assert_level0_neighbors(builder.get_graph(), 3, &[0, 1])?;
    Ok(())
}

fn assert_level0_neighbors(graph: &OnHeapHnswGraph, node: i32, expected: &[i32]) -> Result<()> {
    let mut expected = expected.to_vec();
    expected.sort();
    let nn = graph.get_neighbors(0, node)?;
    let mut actual = nn.nodes()[..nn.size() as usize].to_vec();
    actual.sort();
    assert_array_equals_msg(
        Some(&format!(
            "expected: {} actual: {}",
            crate::core_support::arrays_to_string(&expected),
            crate::core_support::arrays_to_string(&actual)
        )),
        &expected,
        &actual,
    );
    Ok(())
}

fn test_random<C: HnswCase>(this: &mut Fixture<C>) -> Result<()> {
    let size = LuceneTestCase::at_least(100);
    let dim = LuceneTestCase::at_least(10);
    let vectors = C::mock_values(C::vector_values(size, dim));
    let top_k = 5;
    let scorer_supplier = this.build_scorer_supplier(&vectors)?;
    let mut builder = create(&*scorer_supplier, 10, 30, random().next_long())?;
    builder.build(C::as_knn(&vectors).size())?;
    let hnsw = builder.into_graph()?;
    let accept_ords = if random().next_boolean() {
        None
    } else {
        Some(create_random_accept_ords(0, size))
    };

    let mut total_matches = 0;
    for _ in 0..100 {
        let query = C::random_vector(dim);
        let mut actual = HnswGraphSearcher::search_on_heap(
            this.build_scorer(&vectors, &query)?.as_mut(),
            100,
            &hnsw,
            accept_ords.as_ref().map(|b| b as &dyn Bits),
            i32::MAX as i64,
        )?;
        let top_docs = actual.top_docs()?;
        let mut expected = NeighborQueue::new(top_k, false)?;
        for j in 0..size {
            // `vectorValue(vectors, j) != null`: a null row raises before the comparison here.
            let _ = vector_value::<C>(&vectors, j)?;
            if accept_ords.as_ref().is_none_or(|a| a.get(j as usize)) {
                expected.add(
                    j,
                    C::compare(this.sim(), &query, &vector_value::<C>(&vectors, j)?)?,
                );
                if expected.size() > top_k {
                    expected.pop();
                }
            }
        }
        let mut actual_top_k_docs = vec![0; top_k as usize];
        for j in 0..top_k as usize {
            actual_top_k_docs[j] = top_docs.score_docs[j].doc;
        }
        total_matches += compute_overlap(&mut actual_top_k_docs, &mut expected.nodes());
    }
    let overlap = total_matches as f64 / (100 * top_k) as f64;
    println!("overlap={overlap} totalMatches={total_matches}");
    assert_true(&format!("overlap={overlap}"), overlap > 0.9);
    Ok(())
}

/// A submitted task's `Future`.
pub struct Future<T> {
    rx: mpsc::Receiver<std::thread::Result<Result<T>>>,
}

impl<T> Future<T> {
    /// `future.get(timeout, unit)`: `TimeoutException` when the task has not
    /// completed in time; the task's failure is rethrown (Java wraps it in an
    /// `ExecutionException`).
    pub fn get(&self, timeout: Duration) -> Result<T> {
        match self.rx.recv_timeout(timeout) {
            Ok(Ok(result)) => result,
            Ok(Err(panic)) => resume_unwind(panic),
            Err(_) => Throwable::new("java.util.concurrent.TimeoutException", "").throw(),
        }
    }
}

/// `exec.submit(callable)`.
pub fn submit<T: Send + 'static>(
    exec: &ThreadPoolExecutor,
    task: impl FnOnce() -> Result<T> + Send + 'static,
) -> Future<T> {
    let (tx, rx) = mpsc::channel();
    let command = Box::new(move || {
        let _ = tx.send(catch_unwind(AssertUnwindSafe(task)));
    });
    if exec.execute(command).is_err() {
        Throwable::new("java.util.concurrent.RejectedExecutionException", "").throw()
    }
    Future { rx }
}

/// `Executors.newFixedThreadPool(n, new NamedThreadFactory(prefix))`.
fn new_fixed_thread_pool(n: i32, prefix: &str) -> ThreadPoolExecutor {
    ThreadPoolExecutor::new(n, n, Duration::ZERO, NamedThreadFactory::new(prefix))
}

/// Test thread-safety of searching `OnHeapHnswGraph`.
fn test_on_heap_hnsw_graph_search<C: HnswCase>(this: &mut Fixture<C>) -> Result<()> {
    let size = LuceneTestCase::at_least(100);
    let dim = LuceneTestCase::at_least(10);
    let vectors = C::mock_values(C::vector_values(size, dim));
    let scorer_supplier = this.build_scorer_supplier(&vectors)?;
    let mut builder = create(&*scorer_supplier, 10, 30, random().next_long())?;
    builder.build(C::as_knn(&vectors).size())?;
    let hnsw = Arc::new(builder.into_graph()?);
    let accept_ords: Option<Arc<FixedBitSet>> = if random().next_boolean() {
        None
    } else {
        Some(Arc::new(create_random_accept_ords(0, size)))
    };

    let mut queries: Vec<C::V> = Vec::new();
    let mut expects: Vec<TopKnnCollector> = Vec::new();
    for _ in 0..100 {
        let query = C::random_vector(dim);
        queries.push(query.clone());
        let expect = HnswGraphSearcher::search_on_heap(
            this.build_scorer(&vectors, &query)?.as_mut(),
            100,
            &hnsw,
            accept_ords.as_deref().map(|b| b as &dyn Bits),
            i32::MAX as i64,
        )?;
        expects.push(expect);
    }

    let exec = new_fixed_thread_pool(4, "onHeapHnswSearch");
    let mut futures: Vec<Future<TopKnnCollector>> = Vec::new();
    let flat = this.flat_vector_scorer;
    let sim = this.sim();
    for query in queries {
        let vectors = Arc::clone(&vectors);
        let hnsw = Arc::clone(&hnsw);
        let accept_ords = accept_ords.clone();
        futures.push(submit(&exec, move || {
            // Java rethrows the IOException as a RuntimeException.
            HnswGraphSearcher::search_on_heap(
                build_scorer_with::<C>(&flat, sim, &vectors, &query)?.as_mut(),
                100,
                &hnsw,
                accept_ords.as_deref().map(|b| b as &dyn Bits),
                i32::MAX as i64,
            )
        }));
    }
    let mut actuals: Vec<TopKnnCollector> = Vec::new();
    for future in &futures {
        actuals.push(future.get(Duration::from_secs(10))?);
    }
    exec.shutdown_now();
    for i in 0..expects.len() {
        let expect = expects[i].top_docs()?;
        let actual = actuals[i].top_docs()?;
        let mut expected_docs = vec![0; expect.score_docs.len()];
        for j in 0..expect.score_docs.len() {
            expected_docs[j] = expect.score_docs[j].doc;
        }
        let mut actual_docs = vec![0; actual.score_docs.len()];
        for j in 0..actual.score_docs.len() {
            actual_docs[j] = actual.score_docs[j].doc;
        }
        crate::core_support::assert_array_equals(&expected_docs, &actual_docs);
    }
    Ok(())
}

/// A very basic test ensure the concurrent merge does not throw exceptions,
/// it by no means guarantees the true correctness of the concurrent merge and
/// that must be checked manually by running a KNN benchmark and comparing
/// the recall.
fn test_concurrent_merge_builder<C: HnswCase>(this: &mut Fixture<C>) -> Result<()> {
    let size = LuceneTestCase::at_least(1000);
    let dim = LuceneTestCase::at_least(10);
    let vectors = C::mock_values(C::vector_values(size, dim));
    let scorer_supplier = this.build_scorer_supplier(&vectors)?;
    let exec = new_fixed_thread_pool(4, "hnswMerge");
    let task_executor = Arc::new(TaskExecutor::new(
        Arc::new(exec.clone()) as Arc<dyn Executor>
    ));
    HnswGraphBuilder::set_rand_seed(random().next_long() as u64);
    let mut builder = HnswConcurrentMergeBuilder::new(
        task_executor,
        4,
        &*scorer_supplier,
        10,
        30,
        OnHeapHnswGraph::new(10, size),
        None,
    )?;
    builder.set_batch_size(100);
    builder.build(size)?;
    exec.shutdown_now();
    {
        let graph = builder.get_completed_graph()?;
        assert_true("", graph.entry_node()? != -1);
        assert_equals(&size, &graph.size());
        assert_equals(&(size - 1), &graph.max_node_id());
        for l in 0..graph.num_levels()? {
            assert_not_null(&Some(graph.get_nodes_on_level(l)?));
        }
    }
    // cannot build twice
    expect_throws("java.lang.IllegalStateException", || {
        builder.build(size).map(|_| ())
    });
    Ok(())
}

fn test_all_nodes_visited_in_single_level<C: HnswCase>(this: &mut Fixture<C>) -> Result<()> {
    let size = LuceneTestCase::at_least(100);
    let dim = LuceneTestCase::at_least(50);

    // Search for a large number of results
    let top_k = size - 1;

    let doc_vectors = C::mock_values(C::vector_values(size, dim));
    let mut builder = create(
        &*this.build_scorer_supplier(&doc_vectors)?,
        10,
        30,
        random().next_long(),
    )?;
    builder.build(size)?;
    let mut graph = builder.into_graph()?;

    let query_vectors = C::mock_values(C::vector_values(1, dim));
    let mut query_scorer =
        this.build_scorer(&doc_vectors, &vector_value::<C>(&query_vectors, 0)?)?;

    let mut collector = TopKnnCollector::new(top_k, i32::MAX as i64);
    {
        let mut single_level_graph = SingleLevelGraph(DelegateHnswGraph::new(&mut graph));
        HnswGraphSearcher::search_accept_ords(
            query_scorer.as_mut(),
            &mut collector,
            &mut single_level_graph,
            None,
        )?;
    }

    // Check that we visit all nodes
    assert_equals(&(graph.size() as i64), &collector.visited_count());
    Ok(())
}

/// `computeOverlap(a, b)`: sorts both arrays.
fn compute_overlap(a: &mut [i32], b: &mut [i32]) -> i32 {
    a.sort();
    b.sort();
    let mut overlap = 0;
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i] == b[j] {
            overlap += 1;
            i += 1;
            j += 1;
        } else if a[i] > b[j] {
            j += 1;
        } else {
            i += 1;
        }
    }
    overlap
}

/// `unitVector2d(piRadians)`.
pub fn unit_vector2d(pi_radians: f64) -> Vec<f32> {
    let mut value = vec![0f32; 2];
    unit_vector2d_into(pi_radians, &mut value);
    value
}

/// `unitVector2d(piRadians, value)`.
pub fn unit_vector2d_into(pi_radians: f64, value: &mut [f32]) {
    value[0] = (std::f64::consts::PI * pi_radians).cos() as f32;
    value[1] = (std::f64::consts::PI * pi_radians).sin() as f32;
}

/// `getNeighborNodes(g)`: a `HashSet<Integer>`.
fn get_neighbor_nodes(g: &mut dyn HnswGraph) -> Result<JavaHashSet<i32>> {
    let mut neighbors = JavaHashSet::new();
    let mut n = g.next_neighbor()?;
    while n != NO_MORE_DOCS {
        neighbors.add(n);
        n = g.next_neighbor()?;
    }
    Ok(neighbors)
}

/// `assertVectorsEqual(u, v)`.
fn assert_vectors_equal<C: HnswCase>(u: &C::Values, v: &C::Values) -> Result<()> {
    assert_equals(&C::as_knn(u).size(), &C::as_knn(v).size());
    for ord in 0..C::as_knn(u).size() {
        let u_doc = C::as_knn(u).ord_to_doc(ord);
        let v_doc = C::as_knn(v).ord_to_doc(ord);
        assert_equals(&u_doc, &v_doc);
        assert_not_equals(&NO_MORE_DOCS, &u_doc);
        C::assert_vector_equals(
            &format!("vectors do not match for doc={u_doc}"),
            &vector_value::<C>(u, ord)?,
            &vector_value::<C>(v, ord)?,
        );
    }
    Ok(())
}

/// `createRandomFloatVectors(size, dimension, random)`.
pub fn create_random_float_vectors(size: i32, dimension: i32) -> Vec<Option<Vec<f32>>> {
    let mut vectors = Vec::with_capacity(size as usize);
    for _ in 0..size {
        vectors.push(Some(random_vector(dimension)));
    }
    vectors
}

/// `createRandomByteVectors(size, dimension, random)`.
pub fn create_random_byte_vectors(size: i32, dimension: i32) -> Vec<Option<Vec<u8>>> {
    let mut vectors = Vec::with_capacity(size as usize);
    for _ in 0..size {
        vectors.push(Some(random_vector8(dimension)));
    }
    vectors
}

/// Generate a random bitset where before startIndex all bits are set, and
/// after startIndex each entry has a 2/3 probability of being set.
fn create_random_accept_ords(start_index: i32, length: i32) -> FixedBitSet {
    let mut bits = FixedBitSet::new(length as usize);
    // all bits are set before startIndex
    for i in 0..start_index {
        bits.set(i as usize);
    }
    // after startIndex, bits are set with 2/3 probability
    for i in start_index as usize..bits.length() {
        if random().next_float() < 0.667f32 {
            bits.set(i);
        }
    }
    bits
}

/// `randomVector(random, dim)` (static), on `random()`.
pub fn random_vector(dim: i32) -> Vec<f32> {
    let random = random();
    let mut vec = vec![0f32; dim as usize];
    for i in 0..dim as usize {
        vec[i] = random.next_float();
        if random.next_boolean() {
            vec[i] = -vec[i];
        }
    }
    vector_util::l2normalize(&mut vec, true).unwrap_or_else(|e| rethrow_java(e));
    vec
}

/// Java's `(byte) f` of a `float`: to `int` (truncating), then to `byte`.
pub fn float_to_byte(f: f32) -> u8 {
    (f as i32) as i8 as u8
}

/// `randomVector8(random, dim)` (static), on `random()`.
pub fn random_vector8(dim: i32) -> Vec<u8> {
    let fvec = random_vector(dim);
    let mut bvec = vec![0u8; dim as usize];
    for i in 0..dim as usize {
        bvec[i] = float_to_byte(fvec[i] * 127.0);
    }
    bvec
}

/// `prettyPrint(hnsw)`; `to_string` is `hnsw.toString()`.
pub fn pretty_print(hnsw: &mut dyn HnswGraph, to_string: &str) -> String {
    let mut sb = String::new();
    sb.push_str(to_string);
    sb.push('\n');

    let walk = (|| -> Result<()> {
        for level in 0..hnsw.num_levels()? {
            sb.push_str(&format!("# Level {level}\n"));
            let mut it = hnsw.get_nodes_on_level(level)?;
            while it.has_next() {
                let node = it.next_int();
                sb.push_str(&format!("  {node} -> "));
                hnsw.seek(level, node)?;
                loop {
                    let neighbor = hnsw.next_neighbor()?;
                    if neighbor == NO_MORE_DOCS {
                        break;
                    }
                    sb.push_str(&format!(" {neighbor}"));
                }
                sb.push('\n');
            }
        }
        Ok(())
    })();
    // `throw new RuntimeException(e)`.
    if let Err(e) = walk {
        rethrow_java(e)
    }

    sb
}

/// `private static class DelegateHnswGraph extends HnswGraph`.
pub struct DelegateHnswGraph<'a> {
    delegate: &'a mut dyn HnswGraph,
}

impl<'a> DelegateHnswGraph<'a> {
    fn new(delegate: &'a mut dyn HnswGraph) -> Self {
        DelegateHnswGraph { delegate }
    }
}

impl HnswGraph for DelegateHnswGraph<'_> {
    fn seek(&mut self, level: i32, target: i32) -> Result<()> {
        self.delegate.seek(level, target)
    }
    fn size(&self) -> i32 {
        self.delegate.size()
    }
    fn next_neighbor(&mut self) -> Result<i32> {
        self.delegate.next_neighbor()
    }
    fn num_levels(&self) -> Result<i32> {
        self.delegate.num_levels()
    }
    fn entry_node(&self) -> Result<i32> {
        self.delegate.entry_node()
    }
    fn neighbor_count(&self) -> i32 {
        self.delegate.neighbor_count()
    }
    fn max_conn(&self) -> i32 {
        self.delegate.max_conn()
    }
    fn get_nodes_on_level(&self, level: i32) -> Result<Box<dyn NodesIterator>> {
        self.delegate.get_nodes_on_level(level)
    }
}

/// The anonymous `new DelegateHnswGraph(graph) { numLevels() { return 1; } }`
/// of `testAllNodesVisitedInSingleLevel`.
struct SingleLevelGraph<'a>(DelegateHnswGraph<'a>);

impl HnswGraph for SingleLevelGraph<'_> {
    fn seek(&mut self, level: i32, target: i32) -> Result<()> {
        self.0.seek(level, target)
    }
    fn size(&self) -> i32 {
        self.0.size()
    }
    fn next_neighbor(&mut self) -> Result<i32> {
        self.0.next_neighbor()
    }
    fn num_levels(&self) -> Result<i32> {
        // Only retain the last level
        Ok(1)
    }
    fn entry_node(&self) -> Result<i32> {
        self.0.entry_node()
    }
    fn neighbor_count(&self) -> i32 {
        self.0.neighbor_count()
    }
    fn max_conn(&self) -> i32 {
        self.0.max_conn()
    }
    fn get_nodes_on_level(&self, level: i32) -> Result<Box<dyn NodesIterator>> {
        self.0.get_nodes_on_level(level)
    }
}

/// The cursor `OnHeapHnswGraph.seek`/`nextNeighbor` keep, over a graph a
/// builder still owns: `seek` takes `getNeighbors(level, target)` and resets
/// `upto` to `-1`, as Java's `OnHeapHnswGraph.seek` does.
pub struct OnHeapCursor<'a> {
    graph: &'a OnHeapHnswGraph,
    cur: Option<Vec<i32>>,
    upto: i32,
}

impl<'a> OnHeapCursor<'a> {
    /// A cursor over `graph`, not yet positioned.
    pub fn new(graph: &'a OnHeapHnswGraph) -> Self {
        OnHeapCursor {
            graph,
            cur: None,
            upto: -1,
        }
    }
}

impl HnswGraph for OnHeapCursor<'_> {
    fn seek(&mut self, level: i32, target: i32) -> Result<()> {
        let neighbors = self.graph.get_neighbors(level, target)?;
        self.cur = Some(neighbors.nodes());
        self.upto = -1;
        Ok(())
    }
    fn size(&self) -> i32 {
        self.graph.size()
    }
    fn max_node_id(&self) -> i32 {
        self.graph.max_node_id()
    }
    fn next_neighbor(&mut self) -> Result<i32> {
        let cur = self
            .cur
            .as_ref()
            .unwrap_or_else(|| Throwable::new("java.lang.NullPointerException", "cur").throw());
        self.upto += 1;
        if (self.upto as usize) < cur.len() {
            Ok(cur[self.upto as usize])
        } else {
            Ok(NO_MORE_DOCS)
        }
    }
    fn num_levels(&self) -> Result<i32> {
        self.graph.num_levels()
    }
    fn entry_node(&self) -> Result<i32> {
        self.graph.entry_node()
    }
    fn neighbor_count(&self) -> i32 {
        self.cur.as_ref().map_or(0, |c| c.len() as i32)
    }
    fn max_conn(&self) -> i32 {
        self.graph.max_conn()
    }
    fn get_nodes_on_level(&self, level: i32) -> Result<Box<dyn NodesIterator>> {
        self.graph.get_nodes_on_level(level)
    }
}
