//! Port of
//! `lucene/core/src/test/org/apache/lucene/search/BaseVectorSimilarityQueryTestCase.java`.
//!
//! The abstract class becomes the trait [`VectorSimilarityCase`] (its
//! abstract methods), the fixture [`VectorSimilarityFixture`] (its instance
//! fields) and generic functions (its test methods), which each concrete
//! class lists in its declaration through [`methods`].

use std::any::Any;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::Assert;
use rucene::codecs::hnsw::graph_provider::HnswGraphProvider;
use rucene::codecs::lucene99::hnsw_vectors_format::{
    Lucene99HnswVectorsFormat, Lucene99HnswVectorsReader, DEFAULT_BEAM_WIDTH, DEFAULT_MAX_CONN,
    DEFAULT_NUM_MERGE_WORKER,
};
use rucene::document::{Document, IntField, Store};
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::filter_reader::FilterLeafReader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::VectorSimilarityFunction;
use rucene::index::{IndexReader, IndexableField, LeafReaderContext, QueryTimeout};
use rucene::search::abstract_vector_similarity_query::{
    default_similarity_strategy, AbstractVectorSimilarityQueryImpl, DECAY_MAX_APPROXIMATION,
    DECAY_MAX_QUALITY, DEFAULT_DECAY,
};
use rucene::search::boost_query::BoostQuery;
use rucene::search::doc_id_set_iterator::NO_MORE_DOCS;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::knn::{Hnsw, KnnSearchStrategy};
use rucene::search::match_all_docs_query::MatchAllDocsQuery;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::score_mode::ScoreMode;
use rucene::search::scorer_supplier::ScorerSupplier;
use rucene::search::segment_cacheable::SegmentCacheable;
use rucene::search::similarities::Explanation;
use rucene::search::term_query::TermQuery;
use rucene::search::weight::Weight;
use rucene::search::Query;
use rucene::util::hnsw::util::HnswUtil;
use rucene::util::hnsw::HnswGraph;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::{Monster, SuppressCodecs};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::{
    assert_query_not_equals, fixture_class_annotated, ftest, null_leaf_context, term,
};
use crate::core_search::test_boost_query::assert_query_equals;
use crate::core_support::java_number::int_value;
use crate::core_support::{assert_equals, assert_equals_delta, random, JavaHashSet, IAE};

/// `java.lang.UnsupportedOperationException`.
const UOE: &str = "java.lang.UnsupportedOperationException";

/// The abstract methods of `BaseVectorSimilarityQueryTestCase<V, F, Q>`.
///
/// `F` (the vector field class) is erased into the `Box<dyn IndexableField>`
/// [`get_vector_field`](Self::get_vector_field) answers.
pub trait VectorSimilarityCase: Sized + Send + 'static {
    /// `V`: the vector type.
    type V: Clone;
    /// `Q extends AbstractVectorSimilarityQuery`.
    type Q: AbstractVectorSimilarityQueryImpl + Clone;

    /// The concrete class's simple name (`getClass().getSimpleName()`).
    const SIMPLE_NAME: &'static str;

    /// `abstract V getRandomVector(int dim)`.
    fn get_random_vector(dim: i32) -> Self::V;

    /// `abstract float compare(V vector1, V vector2)`, which reads the
    /// instance field `function`.
    fn compare(f: &VectorSimilarityFixture<Self>, vector1: &Self::V, vector2: &Self::V) -> f32;

    /// `abstract boolean checkEquals(V vector1, V vector2)`.
    fn check_equals(vector1: &Self::V, vector2: &Self::V) -> bool;

    /// `abstract F getVectorField(String name, V vector, VectorSimilarityFunction function)`.
    fn get_vector_field(
        name: &str,
        vector: &Self::V,
        function: VectorSimilarityFunction,
    ) -> Result<Box<dyn IndexableField>>;

    /// `abstract Q getVectorQuery(String field, V vector, float resultSimilarity, float decay, Query filter)`.
    fn get_vector_query(
        field: &str,
        vector: Self::V,
        result_similarity: f32,
        decay: f32,
        filter: Option<Arc<dyn Query>>,
    ) -> Result<Self::Q>;

    /// `abstract Q getVectorQuery(String field, V vector, float resultSimilarity, float decay, Query filter, KnnSearchStrategy searchStrategy)`.
    fn get_vector_query_with_strategy(
        field: &str,
        vector: Self::V,
        result_similarity: f32,
        decay: f32,
        filter: Option<Arc<dyn Query>>,
        search_strategy: Option<KnnSearchStrategy>,
    ) -> Result<Self::Q>;

    /// `abstract Q getThrowingVectorQuery(String field, V vector, float resultSimilarity, float decay, Query filter)`.
    ///
    /// Java's anonymous subclass of `Q` is a distinct query type here, so it
    /// is answered as a `Query`.
    fn get_throwing_vector_query(
        field: &str,
        vector: Self::V,
        result_similarity: f32,
        decay: f32,
        filter: Option<Arc<dyn Query>>,
    ) -> Result<Arc<dyn Query>>;

    /// The `@Before` method of the concrete class (`setup()`).
    fn setup(f: &mut VectorSimilarityFixture<Self>);
}

/// The instance fields of `BaseVectorSimilarityQueryTestCase`
/// (`String vectorField, idField; VectorSimilarityFunction function; int
/// numDocs, dim;`).
pub struct VectorSimilarityFixture<C: VectorSimilarityCase> {
    pub vector_field: Option<String>,
    pub id_field: Option<String>,
    pub function: Option<VectorSimilarityFunction>,
    pub num_docs: i32,
    pub dim: i32,
    case: std::marker::PhantomData<fn() -> C>,
}

impl<C: VectorSimilarityCase> Default for VectorSimilarityFixture<C> {
    fn default() -> Self {
        VectorSimilarityFixture {
            vector_field: None,
            id_field: None,
            function: None,
            num_docs: 0,
            dim: 0,
            case: std::marker::PhantomData,
        }
    }
}

impl<C: VectorSimilarityCase> VectorSimilarityFixture<C> {
    fn vector_field(&self) -> &str {
        self.vector_field.as_deref().expect("setup ran")
    }

    fn id_field(&self) -> &str {
        self.id_field.as_deref().expect("setup ran")
    }

    /// The instance field `function`, set by `setup()`.
    pub fn function(&self) -> VectorSimilarityFunction {
        self.function.expect("setup ran")
    }
}

/// The class `name`, extending `BaseVectorSimilarityQueryTestCase`
/// (`@LuceneTestCase.SuppressCodecs("SimpleText")`, an `@Inherited`
/// annotation), with the inherited test methods bound to `C`.
pub fn class_def<C: VectorSimilarityCase>(name: &str) -> Arc<Class> {
    fixture_class_annotated::<VectorSimilarityFixture<C>>(name, methods::<C>(), |b| {
        b.annotate(SuppressCodecs {
            value: vec!["SimpleText".to_string()],
        })
    })
}

/// The `@Before setup()` of the concrete class and the 19 test methods of
/// `BaseVectorSimilarityQueryTestCase`.
pub fn methods<C: VectorSimilarityCase>() -> Vec<Method> {
    vec![
        Method::new("setup")
            .annotate(randomizedtesting::junit::Before)
            .body(|f: &mut VectorSimilarityFixture<C>| C::setup(f)),
        ftest("testEquals", test_equals::<C>),
        ftest(
            "testEqualsWithSearchStrategy",
            test_equals_with_search_strategy::<C>,
        ),
        ftest("testGetSearchStrategy", test_get_search_strategy::<C>),
        ftest(
            "testNullSearchStrategyDefaultsToDefault",
            test_null_search_strategy_defaults_to_default::<C>,
        ),
        ftest(
            "testSearchStrategyReachesCollector",
            test_search_strategy_reaches_collector::<C>,
        ),
        ftest("testIllegalParams", test_illegal_params::<C>),
        ftest("testEmptyIndex", test_empty_index::<C>),
        ftest("testExtremes", test_extremes::<C>),
        ftest("testRandomFilter", test_random_filter::<C>),
        ftest("testFilterWithNoMatches", test_filter_with_no_matches::<C>),
        ftest("testDimensionMismatch", test_dimension_mismatch::<C>),
        ftest("testNonVectorsField", test_non_vectors_field::<C>),
        ftest("testSomeDeletes", test_some_deletes::<C>),
        ftest("testAllDeletes", test_all_deletes::<C>),
        ftest("testBoostQuery", test_boost_query::<C>),
        ftest(
            "testVectorsAboveSimilarity",
            test_vectors_above_similarity::<C>,
        ),
        ftest("testFallbackToExact", test_fallback_to_exact::<C>),
        ftest("testApproximate", test_approximate::<C>).annotate(Monster {
            value: "indexes and searches a large number of vectors".to_string(),
        }),
        ftest("testTimeout", test_timeout::<C>),
    ]
}

fn as_query<Q: Query>(q: Q) -> Arc<dyn Query> {
    Arc::new(q)
}

fn test_equals<C: VectorSimilarityCase>(f: &mut VectorSimilarityFixture<C>) -> Result<()> {
    let (field1, field2) = ("f1", "f2");

    let vector1 = C::get_random_vector(f.dim);
    let mut vector2;
    loop {
        vector2 = C::get_random_vector(f.dim);
        if !C::check_equals(&vector1, &vector2) {
            break;
        }
    }

    let (result_similarity1, result_similarity2) = (0.4f32, 0.5f32);
    let (decay1, decay2) = (0.5f32, 0.6f32);

    let filter1: Arc<dyn Query> = Arc::new(TermQuery::new(term("t1", "v1")));
    let filter2: Arc<dyn Query> = Arc::new(TermQuery::new(term("t2", "v2")));

    let query = as_query(C::get_vector_query(
        field1,
        vector1.clone(),
        result_similarity1,
        decay1,
        Some(Arc::clone(&filter1)),
    )?);

    // Everything is equal
    assert_query_equals(
        &query,
        &as_query(C::get_vector_query(
            field1,
            vector1.clone(),
            result_similarity1,
            decay1,
            Some(Arc::clone(&filter1)),
        )?),
    );

    // Null check
    // `assertNotEquals(null, query)`: `query` is a value, never `null`, so
    // `Objects.equals(null, query)` is false by construction.
    Assert::assert_false(false);

    // Different field
    assert_query_not_equals(
        &query,
        &as_query(C::get_vector_query(
            field2,
            vector1.clone(),
            result_similarity1,
            decay1,
            Some(Arc::clone(&filter1)),
        )?),
    );

    // Different vector
    assert_query_not_equals(
        &query,
        &as_query(C::get_vector_query(
            field1,
            vector2,
            result_similarity1,
            decay1,
            Some(Arc::clone(&filter1)),
        )?),
    );

    // Different decay
    assert_query_not_equals(
        &query,
        &as_query(C::get_vector_query(
            field1,
            vector1.clone(),
            result_similarity1,
            decay2,
            Some(Arc::clone(&filter1)),
        )?),
    );

    // Different resultSimilarity
    assert_query_not_equals(
        &query,
        &as_query(C::get_vector_query(
            field1,
            vector1.clone(),
            result_similarity2,
            decay1,
            Some(Arc::clone(&filter1)),
        )?),
    );

    // Different filter
    assert_query_not_equals(
        &query,
        &as_query(C::get_vector_query(
            field1,
            vector1,
            result_similarity1,
            decay1,
            Some(filter2),
        )?),
    );
    Ok(())
}

fn test_equals_with_search_strategy<C: VectorSimilarityCase>(
    f: &mut VectorSimilarityFixture<C>,
) -> Result<()> {
    let field = "f";
    let vector = C::get_random_vector(f.dim);
    let result_similarity = 0.4f32;
    let decay = 0.5f32;
    let filter: Arc<dyn Query> = Arc::new(TermQuery::new(term("t", "v")));

    let strategy_a = KnnSearchStrategy::Hnsw(Hnsw::new(10)?);
    let strategy_b = KnnSearchStrategy::Hnsw(Hnsw::new(20)?);

    let query_a = C::get_vector_query_with_strategy(
        field,
        vector.clone(),
        result_similarity,
        decay,
        Some(Arc::clone(&filter)),
        Some(strategy_a.clone()),
    )?;
    let query_b = C::get_vector_query_with_strategy(
        field,
        vector.clone(),
        result_similarity,
        decay,
        Some(Arc::clone(&filter)),
        Some(strategy_b),
    )?;
    let query_a_dup = C::get_vector_query_with_strategy(
        field,
        vector.clone(),
        result_similarity,
        decay,
        Some(Arc::clone(&filter)),
        Some(strategy_a),
    )?;
    let (query_a_hash, query_a_dup_hash) = (query_a.query_hash(), query_a_dup.query_hash());
    let query_a = as_query(query_a);
    let query_b = as_query(query_b);
    let query_a_dup = as_query(query_a_dup);

    // Queries with the same non-default strategy are equal.
    assert_query_equals(&query_a, &query_a_dup);
    assert_equals(&query_a_hash, &query_a_dup_hash);

    // Queries that differ only in strategy are not equal.
    assert_query_not_equals(&query_a, &query_b);

    // Query with default strategy is not equal to one with non-default strategy.
    let query_default = as_query(C::get_vector_query(
        field,
        vector,
        result_similarity,
        decay,
        Some(filter),
    )?);
    assert_query_not_equals(&query_default, &query_a);
    Ok(())
}

fn test_get_search_strategy<C: VectorSimilarityCase>(
    f: &mut VectorSimilarityFixture<C>,
) -> Result<()> {
    let field = "f";
    let vector = C::get_random_vector(f.dim);
    let strategy = KnnSearchStrategy::Hnsw(Hnsw::new(42)?);

    // Default strategy is exposed via getter.
    let default_query = C::get_vector_query(field, vector.clone(), 0.4, 0.5, None)?;
    assert_equals(
        &default_similarity_strategy(),
        default_query.similarity_base().get_search_strategy(),
    );

    // Custom strategy is propagated through to the getter.
    let custom_query =
        C::get_vector_query_with_strategy(field, vector, 0.4, 0.5, None, Some(strategy.clone()))?;
    // **Divergence from Lucene 10.5.0.** Java: `assertSame(strategy,
    // customQuery.getSearchStrategy())`. Rucene: `KnnSearchStrategy` is a
    // value type the query stores by value, so the strategy is compared by
    // value. Why: the query owns its strategy; no instance is shared.
    assert_equals(
        &strategy,
        custom_query.similarity_base().get_search_strategy(),
    );
    Ok(())
}

fn test_null_search_strategy_defaults_to_default<C: VectorSimilarityCase>(
    f: &mut VectorSimilarityFixture<C>,
) -> Result<()> {
    let field = "f";
    let vector = C::get_random_vector(f.dim);
    let query = C::get_vector_query_with_strategy(
        field, vector, 0.4, 0.5, None, /* searchStrategy= */ None,
    )?;
    // **Divergence from Lucene 10.5.0.** Java: `assertSame(
    // AbstractVectorSimilarityQuery.DEFAULT_STRATEGY,
    // query.getSearchStrategy())`. Rucene: `DEFAULT_STRATEGY` is the value
    // `default_similarity_strategy()` builds, compared by value. Why:
    // `KnnSearchStrategy` is a value type with no shared static instance.
    assert_equals(
        &default_similarity_strategy(),
        query.similarity_base().get_search_strategy(),
    );
    Ok(())
}

fn test_search_strategy_reaches_collector<C: VectorSimilarityCase>(
    f: &mut VectorSimilarityFixture<C>,
) -> Result<()> {
    let strategy = KnnSearchStrategy::Hnsw(Hnsw::new(7)?);
    let query = C::get_vector_query_with_strategy(
        "field",
        C::get_random_vector(f.dim),
        /* resultSimilarity= */ 0.5,
        /* decay= */ 0.5,
        /* filter= */ None,
        Some(strategy.clone()),
    )?;

    // **Divergence from Lucene 10.5.0.** Java passes a `null` context to
    // `newCollector`; Rucene's `KnnCollectorManager::new_collector` takes a
    // context by reference, so the one leaf of a fixed single-document
    // index stands for it. The similarity collector manager ignores it.
    let (_reader, leaves) = null_leaf_context()?;
    let collector = query.get_knn_collector_manager().new_collector(
        i32::MAX,
        Some(query.similarity_base().get_search_strategy()),
        &leaves[0],
    )?;

    // **Divergence from Lucene 10.5.0.** Java: `assertSame(strategy,
    // collector.getSearchStrategy())`. Rucene: the strategy is a value the
    // collector holds a copy of, compared by value (see
    // `test_get_search_strategy`).
    assert_equals(&Some(&strategy), &collector.get_search_strategy());
    Ok(())
}

fn test_illegal_params<C: VectorSimilarityCase>(f: &mut VectorSimilarityFixture<C>) -> Result<()> {
    let vector_field = f.vector_field().to_string();
    let dim = f.dim;
    expect_throws(IAE, || {
        C::get_vector_query(
            &vector_field,
            C::get_random_vector(dim),
            f32::NAN, // illegal resultSimilarity
            DEFAULT_DECAY,
            None,
        )
    });

    expect_throws(IAE, || {
        C::get_vector_query(
            &vector_field,
            C::get_random_vector(dim),
            0.0,
            f32::NAN, // illegal decay
            None,
        )
    });

    expect_throws(IAE, || {
        C::get_vector_query(
            &vector_field,
            C::get_random_vector(dim),
            0.0,
            next_down(DECAY_MAX_APPROXIMATION), // illegal decay
            None,
        )
    });

    expect_throws(IAE, || {
        C::get_vector_query(
            &vector_field,
            C::get_random_vector(dim),
            0.0,
            next_up(DECAY_MAX_QUALITY), // illegal decay
            None,
        )
    });
    Ok(())
}

/// `Math.nextDown(float)`.
fn next_down(v: f32) -> f32 {
    -next_up(-v)
}

/// `Math.nextUp(float)`.
fn next_up(v: f32) -> f32 {
    if v.is_nan() || v == f32::INFINITY {
        v
    } else if v == 0.0 {
        f32::from_bits(1)
    } else if v > 0.0 {
        f32::from_bits(v.to_bits() + 1)
    } else {
        f32::from_bits(v.to_bits() - 1)
    }
}

/// `try (Directory indexStore = getIndexStore(vectors); IndexReader reader
/// = DirectoryReader.open(indexStore)) { body }`.
fn with_index<C: VectorSimilarityCase, T>(
    f: &VectorSimilarityFixture<C>,
    vectors: &[C::V],
    body: impl FnOnce(&Arc<dyn DirectoryReader>) -> Result<T>,
) -> Result<T> {
    let mut index_store = get_index_store(f, vectors)?;
    let reader = directory_reader::open(index_store.as_directory())?;
    let result = body(&reader);
    reader.close()?;
    index_store.close()?;
    result
}

fn test_empty_index<C: VectorSimilarityCase>(f: &mut VectorSimilarityFixture<C>) -> Result<()> {
    // Do not index any vectors
    f.num_docs = 0;

    let vectors = get_random_vectors::<C>(f.num_docs, f.dim);
    with_index(f, &vectors, |reader| {
        let searcher = LuceneTestCase::new_searcher(Arc::clone(reader));

        let query = as_query(C::get_vector_query(
            f.vector_field(),
            C::get_random_vector(f.dim),
            f32::NEG_INFINITY,
            DECAY_MAX_QUALITY,
            None,
        )?);

        // Check that no vectors are found
        assert_equals(&0, &searcher.count(query)?);
        Ok(())
    })
}

/// `HnswUtil.graphIsRooted(reader, vectorField)`, with the cast chain Java
/// writes inside it (`(HnswGraphProvider) ((PerFieldKnnVectorsFormat.FieldsReader)
/// ((CodecReader) FilterLeafReader.unwrap(ctx.reader())).getVectorReader())
/// .getFieldReader(vectorField)`) supplied as the leaf-to-graph function
/// Rucene's `HnswUtil::graph_is_rooted` takes.
fn graph_is_rooted(reader: &Arc<dyn DirectoryReader>, vector_field: &str) -> Result<bool> {
    HnswUtil::graph_is_rooted(
        Arc::clone(reader) as Arc<dyn IndexReader>,
        vector_field,
        &hnsw_graph_of_leaf,
    )
}

fn class_cast(what: &str) -> LuceneError {
    // `ClassCastException` has no `LuceneError` variant; the failed cast is
    // reported as an unchecked error naming it.
    LuceneError::from(LuceneErrorKind::Other(format!(
        "java.lang.ClassCastException: {what}"
    )))
}

fn hnsw_graph_of_leaf(ctx: &LeafReaderContext, vector_field: &str) -> Result<Box<dyn HnswGraph>> {
    let leaf = FilterLeafReader::unwrap(&ctx.leaf_reader());
    let codec_reader = leaf
        .as_codec_reader()
        .ok_or_else(|| class_cast("not a CodecReader"))?;
    let vectors_reader = codec_reader
        .get_vector_reader()?
        .ok_or_else(|| class_cast("null vector reader"))?;
    let field_reader = vectors_reader
        .get_field_reader(vector_field)
        .ok_or_else(|| class_cast("not a PerFieldKnnVectorsFormat.FieldsReader"))?;
    let provider = field_reader
        .as_any()
        .and_then(|a| a.downcast_ref::<Lucene99HnswVectorsReader>())
        .ok_or_else(|| class_cast("not a HnswGraphProvider"))?;
    HnswGraphProvider::get_graph(provider, vector_field)
}

fn test_extremes<C: VectorSimilarityCase>(f: &mut VectorSimilarityFixture<C>) -> Result<()> {
    let vectors = get_random_vectors::<C>(f.num_docs, f.dim);
    with_index(f, &vectors, |reader| {
        let searcher = LuceneTestCase::new_searcher(Arc::clone(reader));
        LuceneTestCase::assume_true(
            "graph is disconnected",
            graph_is_rooted(reader, f.vector_field())?,
        );

        // All vectors are above -Infinity
        let query1 = as_query(C::get_vector_query(
            f.vector_field(),
            C::get_random_vector(f.dim),
            f32::NEG_INFINITY,
            DECAY_MAX_QUALITY,
            None,
        )?);

        // Check that all vectors are found
        assert_equals(&f.num_docs, &searcher.count(query1)?);

        // No vectors are above +Infinity
        let query2 = as_query(C::get_vector_query(
            f.vector_field(),
            C::get_random_vector(f.dim),
            f32::INFINITY,
            DECAY_MAX_QUALITY,
            None,
        )?);

        // Check that no vectors are found
        assert_equals(&0, &searcher.count(query2)?);
        Ok(())
    })
}

fn test_random_filter<C: VectorSimilarityCase>(f: &mut VectorSimilarityFixture<C>) -> Result<()> {
    // Filter a sub-range from 0 to numDocs
    let start_index = random().next_int_bound(f.num_docs);
    let end_index = random().next_int_range(start_index, f.num_docs);
    let filter = IntField::new_range_query(f.id_field(), start_index, end_index)?;

    let vectors = get_random_vectors::<C>(f.num_docs, f.dim);
    with_index(f, &vectors, |reader| {
        LuceneTestCase::assume_true(
            "graph is disconnected",
            graph_is_rooted(reader, f.vector_field())?,
        );
        let searcher = LuceneTestCase::new_searcher(Arc::clone(reader));

        let query = as_query(C::get_vector_query(
            f.vector_field(),
            C::get_random_vector(f.dim),
            f32::NEG_INFINITY,
            DECAY_MAX_QUALITY,
            Some(filter),
        )?);

        let score_docs = searcher.search(query, f.num_docs)?.score_docs;
        for score_doc in &score_docs {
            let id = get_id(f, &searcher, score_doc.doc)?;

            // Check that returned document is in selected range
            Assert::assert_true(id >= start_index && id <= end_index);
        }
        // Check that all filtered vectors are found
        assert_equals(&(end_index - start_index + 1), &(score_docs.len() as i32));
        Ok(())
    })
}

fn test_filter_with_no_matches<C: VectorSimilarityCase>(
    f: &mut VectorSimilarityFixture<C>,
) -> Result<()> {
    let vectors = get_random_vectors::<C>(f.num_docs, f.dim);
    with_index(f, &vectors, |reader| {
        let searcher = LuceneTestCase::new_searcher(Arc::clone(reader));

        // Non-existent field
        let filter1: Arc<dyn Query> =
            Arc::new(TermQuery::new(term("random_field", "random_value")));
        let query1 = as_query(C::get_vector_query(
            f.vector_field(),
            C::get_random_vector(f.dim),
            f32::NEG_INFINITY,
            DECAY_MAX_QUALITY,
            Some(filter1),
        )?);

        // Check that no vectors are found
        assert_equals(&0, &searcher.count(query1)?);

        // Field exists, but value of -1 is not indexed
        let filter2 = IntField::new_exact_query(f.id_field(), -1)?;
        let query2 = as_query(C::get_vector_query(
            f.vector_field(),
            C::get_random_vector(f.dim),
            f32::NEG_INFINITY,
            DECAY_MAX_QUALITY,
            Some(filter2),
        )?);

        // Check that no vectors are found
        assert_equals(&0, &searcher.count(query2)?);
        Ok(())
    })
}

fn test_dimension_mismatch<C: VectorSimilarityCase>(
    f: &mut VectorSimilarityFixture<C>,
) -> Result<()> {
    // Different dimension
    let new_dim = LuceneTestCase::at_least(f.dim + 1);

    let vectors = get_random_vectors::<C>(f.num_docs, f.dim);
    with_index(f, &vectors, |reader| {
        let searcher = LuceneTestCase::new_searcher(Arc::clone(reader));

        let query = as_query(C::get_vector_query(
            f.vector_field(),
            C::get_random_vector(new_dim),
            f32::NEG_INFINITY,
            DECAY_MAX_QUALITY,
            None,
        )?);

        // Check that an exception for differing dimensions is thrown
        let e = expect_throws(IAE, || searcher.count(Arc::clone(&query)));
        assert_equals(
            format!(
                "vector query dimension: {} differs from field dimension: {}",
                new_dim, f.dim
            )
            .as_str(),
            e.message().unwrap_or_default(),
        );
        Ok(())
    })
}

fn test_non_vectors_field<C: VectorSimilarityCase>(
    f: &mut VectorSimilarityFixture<C>,
) -> Result<()> {
    let vectors = get_random_vectors::<C>(f.num_docs, f.dim);
    with_index(f, &vectors, |reader| {
        let searcher = LuceneTestCase::new_searcher(Arc::clone(reader));

        // Non-existent field
        let query1 = as_query(C::get_vector_query(
            "random_field",
            C::get_random_vector(f.dim),
            f32::NEG_INFINITY,
            DECAY_MAX_QUALITY,
            None,
        )?);
        assert_equals(&0, &searcher.count(query1)?);

        // Indexed as int field
        let query2 = as_query(C::get_vector_query(
            f.id_field(),
            C::get_random_vector(f.dim),
            f32::NEG_INFINITY,
            DECAY_MAX_QUALITY,
            None,
        )?);
        assert_equals(&0, &searcher.count(query2)?);
        Ok(())
    })
}

fn test_some_deletes<C: VectorSimilarityCase>(f: &mut VectorSimilarityFixture<C>) -> Result<()> {
    // Delete a sub-range from 0 to numDocs
    let start_index = random().next_int_bound(f.num_docs);
    let end_index = random().next_int_range(start_index, f.num_docs);
    let delete = IntField::new_range_query(f.id_field(), start_index, end_index)?;

    let vectors = get_random_vectors::<C>(f.num_docs, f.dim);
    let mut index_store = get_index_store(f, &vectors)?;
    let w = IndexWriter::new(
        index_store.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )?;

    w.delete_documents_by_queries(vec![delete])?;
    w.commit()?;

    let reader = directory_reader::open(index_store.as_directory())?;
    let result = (|| -> Result<()> {
        LuceneTestCase::assume_true(
            "graph is disconnected",
            graph_is_rooted(&reader, f.vector_field())?,
        );
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

        let query = as_query(C::get_vector_query(
            f.vector_field(),
            C::get_random_vector(f.dim),
            f32::NEG_INFINITY,
            DECAY_MAX_QUALITY,
            None,
        )?);

        let score_docs = searcher.search(query, f.num_docs)?.score_docs;
        for score_doc in &score_docs {
            let id = get_id(f, &searcher, score_doc.doc)?;

            // Check that returned document is not deleted
            Assert::assert_false(id >= start_index && id <= end_index);
        }
        // Check that all live docs are returned
        assert_equals(
            &(f.num_docs - end_index + start_index - 1),
            &(score_docs.len() as i32),
        );
        Ok(())
    })();
    reader.close()?;
    w.close()?;
    index_store.close()?;
    result
}

fn test_all_deletes<C: VectorSimilarityCase>(f: &mut VectorSimilarityFixture<C>) -> Result<()> {
    let vectors = get_random_vectors::<C>(f.num_docs, f.dim);
    let mut dir = get_index_store(f, &vectors)?;
    let w = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )?;
    // Delete all documents
    w.delete_documents_by_queries(vec![Arc::new(MatchAllDocsQuery::instance())])?;
    w.commit()?;

    let reader = directory_reader::open(dir.as_directory())?;
    let result = (|| -> Result<()> {
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

        let query = as_query(C::get_vector_query(
            f.vector_field(),
            C::get_random_vector(f.dim),
            f32::NEG_INFINITY,
            DECAY_MAX_QUALITY,
            None,
        )?);

        // Check that no vectors are found
        assert_equals(&0, &searcher.count(query)?);
        Ok(())
    })();
    reader.close()?;
    w.close()?;
    dir.close()?;
    result
}

fn test_boost_query<C: VectorSimilarityCase>(f: &mut VectorSimilarityFixture<C>) -> Result<()> {
    // Define the boost and allowed delta
    let boost = random().next_float_range(5.0, 10.0);
    let delta = 1e-3f32;

    let vectors = get_random_vectors::<C>(f.num_docs, f.dim);
    with_index(f, &vectors, |reader| {
        let searcher = LuceneTestCase::new_searcher(Arc::clone(reader));

        let query1 = as_query(C::get_vector_query(
            f.vector_field(),
            C::get_random_vector(f.dim),
            f32::NEG_INFINITY,
            DECAY_MAX_QUALITY,
            None,
        )?);
        let score_docs1 = searcher.search(Arc::clone(&query1), f.num_docs)?.score_docs;

        let query2: Arc<dyn Query> = Arc::new(BoostQuery::new(query1, boost)?);
        let score_docs2 = searcher.search(query2, f.num_docs)?.score_docs;

        // Check that original scores and boosted scores are equal considering the delta to account
        // for floating point precision limitations. Don't take the exact result order into
        // consideration as for small original scores with tiny differences
        // the boosted scores can be become the same, which might affect the result order.
        assert_equals(&score_docs1.len(), &score_docs2.len());
        for i in 0..score_docs1.len() {
            let idx = i;
            let boosted_doc = score_docs2.iter().find(|d| d.doc == score_docs1[idx].doc);

            Assert::assert_true(boosted_doc.is_some());
            assert_equals_delta(
                (boost * score_docs1[i].score) as f64,
                boosted_doc.expect("present").score as f64,
                delta as f64,
            );
        }
        Ok(())
    })
}

fn test_vectors_above_similarity<C: VectorSimilarityCase>(
    f: &mut VectorSimilarityFixture<C>,
) -> Result<()> {
    // Pick number of docs to accept
    let num_accepted = random().next_int_range(f.num_docs / 3, f.num_docs / 2);
    let delta = 1e-3f32;

    let vectors = get_random_vectors::<C>(f.num_docs, f.dim);
    let query_vector = C::get_random_vector(f.dim);

    // Find score above which we get (at least) numAccepted vectors
    let result_similarity = get_similarity(f, &vectors, &query_vector, num_accepted);

    // Cache scores of vectors
    // `Map<Integer, Float>`: only `containsKey`, `get` and `size` are used,
    // so iteration order is irrelevant.
    let mut scores: std::collections::HashMap<i32, f32> = std::collections::HashMap::new();
    for i in 0..f.num_docs {
        let score = C::compare(f, &query_vector, &vectors[i as usize]);
        if score >= result_similarity {
            scores.insert(i, score);
        }
    }

    with_index(f, &vectors, |reader| {
        let searcher = LuceneTestCase::new_searcher(Arc::clone(reader));

        let query = as_query(C::get_vector_query(
            f.vector_field(),
            query_vector.clone(),
            result_similarity,
            DECAY_MAX_QUALITY,
            None,
        )?);

        let score_docs = searcher.search(query, f.num_docs)?.score_docs;
        for score_doc in &score_docs {
            let id = get_id(f, &searcher, score_doc.doc)?;

            // Check that the collected result is above accepted similarity
            Assert::assert_true(scores.contains_key(&id));

            // Check that the score is correct
            assert_equals_delta(scores[&id] as f64, score_doc.score as f64, delta as f64);
        }

        // Check that all results are collected
        assert_equals(&scores.len(), &score_docs.len());
        Ok(())
    })
}

fn test_fallback_to_exact<C: VectorSimilarityCase>(
    f: &mut VectorSimilarityFixture<C>,
) -> Result<()> {
    // Restrictive filter, along with similarity to visit a large number of nodes
    let num_filtered = f.num_docs / 5;
    let target_visited = f.num_docs;

    let vectors = get_random_vectors::<C>(f.num_docs, f.dim);
    let query_vector = C::get_random_vector(f.dim);

    let result_similarity = get_similarity(f, &vectors, &query_vector, target_visited);
    let filter = IntField::new_set_query(f.id_field(), &get_filtered(f, num_filtered))?;

    with_index(f, &vectors, |reader| {
        let searcher = LuceneTestCase::new_searcher(Arc::clone(reader));

        let query = C::get_throwing_vector_query(
            f.vector_field(),
            query_vector.clone(),
            result_similarity,
            DECAY_MAX_APPROXIMATION,
            Some(filter),
        )?;

        // Falls back to exact search, even with DECAY_MAX_APPROXIMATION
        expect_throws(UOE, || searcher.count(Arc::clone(&query)));

        let exact_query = C::get_throwing_vector_query(
            f.vector_field(),
            query_vector.clone(),
            f32::NEG_INFINITY,
            DECAY_MAX_QUALITY,
            None,
        )?;

        // Exact search should be used directly with DECAY_MAX_QUALITY
        expect_throws(UOE, || searcher.count(Arc::clone(&exact_query)));
        Ok(())
    })
}

fn test_approximate<C: VectorSimilarityCase>(f: &mut VectorSimilarityFixture<C>) -> Result<()> {
    f.num_docs = 1000;

    // Non-restrictive filter, along with similarity to visit a small number of nodes
    let num_filtered = f.num_docs - 1;
    let target_visited = random().next_int_range(1, num_filtered / 10);

    let vectors = get_random_vectors::<C>(f.num_docs, f.dim);
    let query_vector = C::get_random_vector(f.dim);

    let result_similarity = get_similarity(f, &vectors, &query_vector, target_visited);
    let filter = IntField::new_set_query(f.id_field(), &get_filtered(f, num_filtered))?;

    let mut index_store = get_index_store(f, &vectors)?;
    let iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_codec(hnsw_codec())?;
    let w = IndexWriter::new(index_store.as_directory(), iwc)?;
    // Force merge because smaller segments have few filtered docs and often fall back to exact
    // search, making this test flaky
    w.force_merge(1)?;
    w.commit()?;

    let reader = directory_reader::open(index_store.as_directory())?;
    let result = (|| -> Result<()> {
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

        // Use DECAY_MAX_APPROXIMATION to avoid fallback to exact search
        let query = C::get_throwing_vector_query(
            f.vector_field(),
            query_vector,
            result_similarity,
            DECAY_MAX_APPROXIMATION,
            Some(filter),
        )?;

        // Does not fall back to exact search
        Assert::assert_true(searcher.count(query)? <= num_filtered);
        Ok(())
    })();
    reader.close()?;
    w.close()?;
    index_store.close()?;
    result
}

/// `QueryTimeout` lambda `() -> value`.
#[derive(Debug)]
struct ConstantTimeout(bool);

impl QueryTimeout for ConstantTimeout {
    fn should_exit(&self) -> bool {
        self.0
    }
}

/// Test that the query times out correctly.
fn test_timeout<C: VectorSimilarityCase>(f: &mut VectorSimilarityFixture<C>) -> Result<()> {
    let vectors = get_random_vectors::<C>(f.num_docs, f.dim);
    let query_vector = C::get_random_vector(f.dim);

    with_index(f, &vectors, |reader| {
        let mut searcher = LuceneTestCase::new_searcher(Arc::clone(reader));

        // This query is cacheable, explicitly prevent it
        searcher.set_query_cache(None);

        // Use Math.nextDown(DECAY_MAX_QUALITY) to ensure approximate graph search is used,
        // since DECAY_MAX_QUALITY now triggers exact search which bypasses graph traversal
        let high_quality_no_exact = next_down(DECAY_MAX_QUALITY);

        let query: Arc<dyn Query> = Arc::new(CountingQuery::new(as_query(C::get_vector_query(
            f.vector_field(),
            query_vector.clone(),
            f32::NEG_INFINITY,
            high_quality_no_exact,
            None,
        )?)));
        assert_equals(&f.num_docs, &searcher.count(Arc::clone(&query))?); // Expect some results without timeout

        searcher.set_timeout(Some(Arc::new(ConstantTimeout(true)))); // Immediately timeout
        assert_equals(&0, &searcher.count(Arc::clone(&query))?); // Expect no results with the timeout

        searcher.set_timeout(Some(Arc::new(CountingQueryTimeout::new(f.num_docs - 1)))); // Do not score all docs
        let count = searcher.count(Arc::clone(&query))?;
        Assert::assert_true_msg(
            &format!("0 < count={} < numDocs={}", count, f.num_docs),
            count > 0 && count < f.num_docs,
        ); // Expect partial results

        // Test timeout with filter
        let num_filtered = random().next_int_range(f.num_docs / 2, f.num_docs);
        let filter = IntField::new_set_query(f.id_field(), &get_filtered(f, num_filtered))?;
        let filtered_query: Arc<dyn Query> =
            Arc::new(CountingQuery::new(as_query(C::get_vector_query(
                f.vector_field(),
                query_vector.clone(),
                f32::NEG_INFINITY,
                high_quality_no_exact,
                Some(filter),
            )?)));

        searcher.set_timeout(Some(Arc::new(ConstantTimeout(false)))); // Set a timeout which is never met
        assert_equals(&num_filtered, &searcher.count(Arc::clone(&filtered_query))?);

        searcher.set_timeout(Some(Arc::new(CountingQueryTimeout::new(num_filtered - 1)))); // Timeout before scoring all filtered docs
        let filtered_count = searcher.count(Arc::clone(&filtered_query))?;
        Assert::assert_true_msg(
            &format!(
                "0 < filteredCount={} < numFiltered={}",
                filtered_count, num_filtered
            ),
            filtered_count > 0 && filtered_count < num_filtered,
        ); // Expect partial results
        Ok(())
    })
}

fn get_similarity<C: VectorSimilarityCase>(
    f: &VectorSimilarityFixture<C>,
    vectors: &[C::V],
    query_vector: &C::V,
    target_visited: i32,
) -> f32 {
    Assert::assert_true(target_visited >= 0 && target_visited <= f.num_docs);
    if target_visited == 0 {
        return f32::INFINITY;
    }

    let mut scores = vec![0f32; f.num_docs as usize];
    for i in 0..f.num_docs as usize {
        scores[i] = C::compare(f, query_vector, &vectors[i]);
    }
    // `Arrays.sort(float[])`: `Float.compare` order.
    scores.sort_by(|a, b| a.total_cmp(b));

    scores[(f.num_docs - target_visited) as usize]
}

fn get_filtered<C: VectorSimilarityCase>(
    f: &VectorSimilarityFixture<C>,
    num_filtered: i32,
) -> Vec<i32> {
    let mut accepted: JavaHashSet<i32> = JavaHashSet::new();
    let mut i = 0;
    while i < num_filtered {
        let index = random().next_int_bound(f.num_docs);
        if !accepted.contains(&index) {
            accepted.add(index);
            i += 1;
        }
    }
    accepted.to_vec()
}

fn get_id<C: VectorSimilarityCase>(
    f: &VectorSimilarityFixture<C>,
    searcher: &IndexSearcher,
    doc: i32,
) -> Result<i32> {
    let document = searcher.stored_fields()?.document(doc)?;
    // `Objects.requireNonNull(...getField(idField)).numericValue().intValue()`
    let field = match document.get_field(f.id_field()) {
        Some(field) => field,
        None => {
            randomizedtesting::jdk::exception::Throwable::new("java.lang.NullPointerException", "")
                .throw()
        }
    };
    Ok(int_value(field.numeric_value()))
}

/// `V[] getRandomVectors(int numDocs, int dim)`.
pub fn get_random_vectors<C: VectorSimilarityCase>(num_docs: i32, dim: i32) -> Vec<C::V> {
    (0..num_docs).map(|_| C::get_random_vector(dim)).collect()
}

/// `TestUtil.alwaysKnnVectorsFormat(new Lucene99HnswVectorsFormat(DEFAULT_MAX_CONN, DEFAULT_BEAM_WIDTH, 0))`.
fn hnsw_codec() -> Arc<dyn rucene::codecs::Codec> {
    TestUtil::always_knn_vectors_format(Arc::new(
        Lucene99HnswVectorsFormat::with_merge_exec_and_threshold(
            DEFAULT_MAX_CONN,
            DEFAULT_BEAM_WIDTH,
            DEFAULT_NUM_MERGE_WORKER,
            None,
            0,
        ),
    ))
}

/// `final Directory getIndexStore(V... vectors)`.
pub fn get_index_store<C: VectorSimilarityCase>(
    f: &VectorSimilarityFixture<C>,
    vectors: &[C::V],
) -> Result<rucene_test_framework::util::lucene_test_case::TestDirectory> {
    let dir = LuceneTestCase::new_directory();
    let iwc: IndexWriterConfig = LuceneTestCase::new_index_writer_config();
    iwc.set_codec(hnsw_codec())?;
    let writer = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;
    let result = (|| -> Result<()> {
        for i in 0..vectors.len() {
            let mut doc = Document::new();
            doc.add(C::get_vector_field(
                f.vector_field(),
                &vectors[i],
                f.function(),
            )?);
            doc.add(Box::new(IntField::new(f.id_field(), i as i32, Store::YES)));
            writer.add_document(&doc)?;
        }
        Ok(())
    })();
    writer.close()?;
    result?;
    Ok(dir)
}

/// `CountingQueryTimeout implements QueryTimeout`.
#[derive(Debug)]
struct CountingQueryTimeout {
    remaining: AtomicI32,
}

impl CountingQueryTimeout {
    fn new(count: i32) -> Self {
        CountingQueryTimeout {
            remaining: AtomicI32::new(count),
        }
    }
}

impl QueryTimeout for CountingQueryTimeout {
    fn should_exit(&self) -> bool {
        // Java mutates a plain `int` field; the query runs on one thread at
        // a time, so a relaxed counter is the same read-modify-write.
        let remaining = self.remaining.load(Ordering::Relaxed);
        if remaining > 0 {
            self.remaining.store(remaining - 1, Ordering::Relaxed);
            return false;
        }
        true
    }
}

/// A [`Query`] that emulates `Weight#count(LeafReaderContext)` by counting
/// number of docs of underlying `Scorer#iterator()`. TODO: This is a
/// workaround to count partial results of `delegate` because
/// `TimeLimitingBulkScorer` immediately discards results after timeout.
#[derive(Debug, Clone)]
struct CountingQuery {
    delegate: Arc<dyn Query>,
}

impl CountingQuery {
    fn new(delegate: Arc<dyn Query>) -> Self {
        CountingQuery { delegate }
    }
}

impl Query for CountingQuery {
    fn to_query_string(&self, field: &str) -> String {
        format!(
            "{}[{}]",
            "CountingQuery",
            self.delegate.to_query_string(field)
        )
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        visitor.visit_leaf(self);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        let delegate_weight = self.delegate.create_weight(searcher, score_mode, boost)?;
        Ok(Arc::new(CountingWeight {
            query: Arc::new(self.clone()),
            delegate_weight,
        }))
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
            && other
                .as_any()
                .downcast_ref::<CountingQuery>()
                .is_some_and(|o| self.delegate.query_eq(&*o.delegate))
    }

    fn query_hash(&self) -> u64 {
        self.delegate.query_hash()
    }
}

/// The anonymous `Weight` of `CountingQuery.createWeight`.
#[derive(Debug)]
struct CountingWeight {
    query: Arc<dyn Query>,
    delegate_weight: Arc<dyn Weight>,
}

impl SegmentCacheable for CountingWeight {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.delegate_weight.is_cacheable(ctx)
    }
}

impl Weight for CountingWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        Arc::clone(&self.query)
    }

    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation> {
        self.delegate_weight.explain(context, doc)
    }

    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        self.delegate_weight.scorer_supplier(context)
    }

    fn count(&self, context: &LeafReaderContext) -> Result<i32> {
        let Some(mut scorer) = self.scorer(context)? else {
            return Ok(0);
        };

        let mut count = 0;
        let iterator = scorer.iterator();
        while iterator.next_doc()? != NO_MORE_DOCS {
            count += 1;
        }
        Ok(count)
    }
}
