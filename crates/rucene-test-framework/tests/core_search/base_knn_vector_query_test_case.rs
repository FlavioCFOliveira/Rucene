//! Port of
//! `lucene/core/src/test/org/apache/lucene/search/BaseKnnVectorQueryTestCase.java`.
//!
//! Test cases for AbstractKnnVectorQuery objects.
//!
//! The abstract class becomes the trait [`KnnVectorCase`] (its abstract
//! methods and the overridable `newDirectoryForTest()`) and generic
//! functions (its test methods), which each concrete class lists in its
//! declaration through [`methods`]. The class has no instance fields.

use std::any::Any;
use std::collections::HashSet;
use std::fmt;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Method;
use randomizedtesting::junit::Assert;
use randomizedtesting::RandomizedTest;
use rucene::codecs::knn_vectors::KnnVectorsFormat;
use rucene::document::{Document, IntPoint, NumericDocValuesField, Store, StringField};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::filter_directory_reader::{
    DirectoryReaderWrapper, FilterDirectoryReader, SubReaderWrapper,
};
use rucene::index::filter_reader::FilterLeafReader;
use rucene::index::index_reader::CacheHelper;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, LiveIndexWriterConfig};
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::merge_scheduler::SerialMergeScheduler;
use rucene::index::{
    IndexReader, IndexableField, LeafReader, LeafReaderContext, QueryTimeout, VectorEncoding,
    VectorSimilarityFunction,
};
use rucene::search::constant_score_scorer::ConstantScoreScorer;
use rucene::search::constant_score_weight::{ConstantScoreWeight, ConstantScoreWeightImpl};
use rucene::search::doc_id_set_iterator::NO_MORE_DOCS;
use rucene::search::field_comparator::SortValue;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::knn::{KnnCollectorManager, KnnSearchStrategy, TopKnnCollectorManager};
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::scorer_supplier::ScorerSupplier;
use rucene::search::similarities::ExplanationValue;
use rucene::search::term_query::TermQuery;
use rucene::search::time_limiting_knn_collector_manager::{
    TimeLimitingKnnCollector, TimeLimitingKnnCollectorManager,
};
use rucene::search::total_hits::TotalHitsRelation;
use rucene::search::weight::{DefaultScorerSupplier, Weight};
use rucene::search::{
    AbstractKnnVectorQueryImpl, BoostQuery, MatchAllDocsQuery, MatchNoDocsQuery, Query, ScoreDoc,
    ScoreMode, Sort, SortField, SortFieldType, TopKnnCollector,
};
use rucene::util::{BitSet, BitSetIterator, BitSetIteratorOverrides, FixedBitSet, MatchNoBits};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::{expect_throws, rethrow_java};
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::{assert_query_not_equals, term};
use crate::core_search::test_boost_query::assert_query_equals;
use crate::core_support::{
    assert_equals, assert_equals_delta, assert_equals_msg, assert_not_null, assert_null, random,
    test,
};

pub const EPSILON: f32 = 0.001;

/// `java.lang.UnsupportedOperationException`.
pub const UOE: &str = "java.lang.UnsupportedOperationException";
/// `java.lang.IllegalArgumentException`.
pub const IAE: &str = "java.lang.IllegalArgumentException";
/// `java.lang.IllegalStateException`.
pub const ISE: &str = "java.lang.IllegalStateException";
/// `java.lang.ArrayIndexOutOfBoundsException`.
pub const AIOOBE: &str = "java.lang.ArrayIndexOutOfBoundsException";

/// The abstract (and overridable) methods of `BaseKnnVectorQueryTestCase`.
///
/// `AbstractKnnVectorQuery` is the concrete query type `Q` of the subclass;
/// the throwing queries, which Java declares as `AbstractKnnVectorQuery`
/// subclasses, are distinct types here and are answered as a `Query`.
pub trait KnnVectorCase: Send + 'static {
    /// The `AbstractKnnVectorQuery` subclass `getKnnVectorQuery` creates.
    type Q: AbstractKnnVectorQueryImpl + Clone;

    /// `abstract AbstractKnnVectorQuery getKnnVectorQuery(String field, float[] query, int k, Query queryFilter)`.
    fn get_knn_vector_query_filtered(
        field: &str,
        query: &[f32],
        k: i32,
        query_filter: Option<Arc<dyn Query>>,
    ) -> Result<Self::Q>;

    /// Ensures that query throws an exception when an exact search is executed
    fn get_throwing_knn_vector_query(
        field: &str,
        query: &[f32],
        k: i32,
        query_filter: Option<Arc<dyn Query>>,
    ) -> Result<Arc<dyn Query>>;

    /// Ensures that an approximate query returns at most maxResults results, and throws an exception
    /// when an exact search is executed
    fn get_capped_results_throwing_knn_vector_query(
        field: &str,
        vec: &[f32],
        k: i32,
        query: Option<Arc<dyn Query>>,
        max_results: i32,
    ) -> Result<Arc<dyn Query>>;

    /// `abstract float[] randomVector(int dim)`.
    fn random_vector(dim: i32) -> Vec<f32>;

    /// `abstract Field getKnnVectorField(String name, float[] vector, VectorSimilarityFunction similarityFunction)`.
    fn get_knn_vector_field_with(
        name: &str,
        vector: &[f32],
        similarity_function: VectorSimilarityFunction,
    ) -> Result<Box<dyn IndexableField>>;

    /// `abstract Field getKnnVectorField(String name, float[] vector)`.
    fn get_knn_vector_field(name: &str, vector: &[f32]) -> Result<Box<dyn IndexableField>>;

    /// Creates a new directory. Subclasses can override to test different directory implementations.
    fn new_directory_for_test() -> Result<TestDirectory> {
        Ok(LuceneTestCase::new_directory_with(&*random()))
    }
}

/// `AbstractKnnVectorQuery getKnnVectorQuery(String field, float[] query, int k)`.
pub fn get_knn_vector_query<C: KnnVectorCase>(field: &str, query: &[f32], k: i32) -> Result<C::Q> {
    C::get_knn_vector_query_filtered(field, query, k, None)
}

/// `q` as the `Query` Java's `AbstractKnnVectorQuery` is.
pub fn as_query<Q: Query>(q: Q) -> Arc<dyn Query> {
    Arc::new(q)
}

/// The 35 test methods of `BaseKnnVectorQueryTestCase`.
pub fn methods<C: KnnVectorCase>() -> Vec<Method> {
    vec![
        test("testEquals", test_equals::<C>),
        test("testGetField", test_get_field::<C>),
        test("testGetK", test_get_k::<C>),
        test("testGetFilter", test_get_filter::<C>),
        test("testEmptyIndex", test_empty_index::<C>),
        test("testFindAll", test_find_all::<C>),
        test("testFindFewer", test_find_fewer::<C>),
        test("testSearchBoost", test_search_boost::<C>),
        test("testSimpleFilter", test_simple_filter::<C>),
        test(
            "testFilterWithNoVectorMatches",
            test_filter_with_no_vector_matches::<C>,
        ),
        test("testMatchAllFilter", test_match_all_filter::<C>),
        test("testDimensionMismatch", test_dimension_mismatch::<C>),
        test("testNonVectorField", test_non_vector_field::<C>),
        test("testIllegalArguments", test_illegal_arguments::<C>),
        test("testDifferentReader", test_different_reader::<C>),
        test("testScoreEuclidean", test_score_euclidean::<C>),
        test("testScoreCosine", test_score_cosine::<C>),
        test("testScoreMIP", test_score_mip::<C>),
        test("testExplain", test_explain::<C>),
        test(
            "testExplainMultipleSegments",
            test_explain_multiple_segments::<C>,
        ),
        test("testSkewedIndex", test_skewed_index::<C>),
        test(
            "testRandomConsistencySingleThreaded",
            test_random_consistency_single_threaded::<C>,
        ),
        test(
            "testRandomConsistencyMultiThreaded",
            test_random_consistency_multi_threaded::<C>,
        ),
        test("testRandom", test_random::<C>),
        test("testRandomWithFilter", test_random_with_filter::<C>),
        test("testFilterWithSameScore", test_filter_with_same_score::<C>),
        test("testDeletes", test_deletes::<C>),
        test("testAllDeletes", test_all_deletes::<C>),
        test("testMergeAwayAllValues", test_merge_away_all_values::<C>),
        test("testNoLiveDocsReader", test_no_live_docs_reader::<C>),
        test("testBitSetQuery", test_bit_set_query::<C>),
        test(
            "testTimeLimitingKnnCollectorManager",
            test_time_limiting_knn_collector_manager::<C>,
        ),
        test("testTimeout", test_timeout::<C>),
        test(
            "testSameFieldDifferentFormats",
            test_same_field_different_formats::<C>,
        ),
        test("testStrategy", test_strategy::<C>),
    ]
}

fn test_equals<C: KnnVectorCase>() -> Result<()> {
    let q1 = as_query(get_knn_vector_query::<C>("f1", &[0.0, 1.0], 10)?);
    let filter1: Arc<dyn Query> = Arc::new(TermQuery::new(term("id", "id1")));
    let q2 = as_query(C::get_knn_vector_query_filtered(
        "f1",
        &[0.0, 1.0],
        10,
        Some(Arc::clone(&filter1)),
    )?);

    assert_query_not_equals(&q2, &q1);
    assert_query_not_equals(&q1, &q2);
    assert_query_equals(
        &q2,
        &as_query(C::get_knn_vector_query_filtered(
            "f1",
            &[0.0, 1.0],
            10,
            Some(Arc::clone(&filter1)),
        )?),
    );

    let filter2: Arc<dyn Query> = Arc::new(TermQuery::new(term("id", "id2")));
    assert_query_not_equals(
        &q2,
        &as_query(C::get_knn_vector_query_filtered(
            "f1",
            &[0.0, 1.0],
            10,
            Some(filter2),
        )?),
    );

    assert_query_equals(
        &q1,
        &as_query(get_knn_vector_query::<C>("f1", &[0.0, 1.0], 10)?),
    );

    // assertNotEquals(null, q1)
    assert_not_null(&Some(&q1));

    assert_query_not_equals(
        &q1,
        &(Arc::new(TermQuery::new(term("f1", "x"))) as Arc<dyn Query>),
    );

    assert_query_not_equals(
        &q1,
        &as_query(get_knn_vector_query::<C>("f2", &[0.0, 1.0], 10)?),
    );
    assert_query_not_equals(
        &q1,
        &as_query(get_knn_vector_query::<C>("f1", &[1.0, 1.0], 10)?),
    );
    assert_query_not_equals(
        &q1,
        &as_query(get_knn_vector_query::<C>("f1", &[0.0, 1.0], 2)?),
    );
    assert_query_not_equals(&q1, &as_query(get_knn_vector_query::<C>("f1", &[0.0], 10)?));
    Ok(())
}

fn test_get_field<C: KnnVectorCase>() -> Result<()> {
    let q1 = get_knn_vector_query::<C>("f1", &[0.0, 1.0], 10)?;
    let filter1: Arc<dyn Query> = Arc::new(TermQuery::new(term("id", "id1")));
    let q2 = C::get_knn_vector_query_filtered("f2", &[0.0, 1.0], 10, Some(filter1))?;

    assert_equals("f1", q1.knn_base().get_field());
    assert_equals("f2", q2.knn_base().get_field());
    Ok(())
}

fn test_get_k<C: KnnVectorCase>() -> Result<()> {
    let q1 = get_knn_vector_query::<C>("f1", &[0.0, 1.0], 6)?;
    let filter1: Arc<dyn Query> = Arc::new(TermQuery::new(term("id", "id1")));
    let q2 = C::get_knn_vector_query_filtered("f2", &[0.0, 1.0], 7, Some(filter1))?;

    assert_equals(&6, &q1.knn_base().get_k());
    assert_equals(&7, &q2.knn_base().get_k());
    Ok(())
}

fn test_get_filter<C: KnnVectorCase>() -> Result<()> {
    let q1 = get_knn_vector_query::<C>("f1", &[0.0, 1.0], 6)?;
    let filter1: Arc<dyn Query> = Arc::new(TermQuery::new(term("id", "id1")));
    let q2 = C::get_knn_vector_query_filtered("f2", &[0.0, 1.0], 7, Some(Arc::clone(&filter1)))?;

    assert_null(&q1.knn_base().get_filter().map(|_| ()));
    assert_query_equals(&filter1, q2.knn_base().get_filter().expect("filter"));
    Ok(())
}

/// `try (Directory indexStore = …; IndexReader reader = DirectoryReader.open(indexStore)) { body }`.
fn with_index<T>(
    mut index_store: TestDirectory,
    body: impl FnOnce(&Arc<dyn DirectoryReader>) -> Result<T>,
) -> Result<T> {
    let reader = directory_reader::open(index_store.as_directory())?;
    let result = body(&reader);
    reader.close()?;
    index_store.close()?;
    result
}

fn reader_of(reader: &Arc<dyn DirectoryReader>) -> Arc<dyn IndexReader> {
    Arc::clone(reader) as Arc<dyn IndexReader>
}

/// Tests if a AbstractKnnVectorQuery is rewritten to a MatchNoDocsQuery when there are no
/// documents to match.
fn test_empty_index<C: KnnVectorCase>() -> Result<()> {
    with_index(get_index_store::<C>("field", &[])?, |reader| {
        let searcher = LuceneTestCase::new_searcher(Arc::clone(reader));
        let kvq = as_query(get_knn_vector_query::<C>("field", &[1.0, 2.0], 10)?);
        assert_matches(&searcher, Arc::clone(&kvq), 0)?;
        let q = searcher.rewrite(kvq)?;
        Assert::assert_true(q.as_any().is::<MatchNoDocsQuery>());
        Ok(())
    })
}

/// Tests that a AbstractKnnVectorQuery whose topK &gt;= numDocs returns all the documents in score
/// order
fn test_find_all<C: KnnVectorCase>() -> Result<()> {
    with_index(
        get_index_store::<C>("field", &[&[0.0, 1.0], &[1.0, 2.0], &[0.0, 0.0]])?,
        |reader| {
            let searcher = LuceneTestCase::new_searcher(Arc::clone(reader));
            let kvq = as_query(get_knn_vector_query::<C>("field", &[0.0, 0.0], 10)?);
            assert_matches(&searcher, Arc::clone(&kvq), 3)?;
            let score_docs = searcher.search(kvq, 3)?.score_docs;
            assert_id_matches(&reader_of(reader), "id2", &score_docs[0])?;
            assert_id_matches(&reader_of(reader), "id0", &score_docs[1])?;
            assert_id_matches(&reader_of(reader), "id1", &score_docs[2])?;
            Ok(())
        },
    )
}

fn test_find_fewer<C: KnnVectorCase>() -> Result<()> {
    with_index(
        get_index_store::<C>("field", &[&[0.0, 1.0], &[1.0, 2.0], &[0.0, 0.0]])?,
        |reader| {
            let searcher = LuceneTestCase::new_searcher(Arc::clone(reader));
            let kvq = as_query(get_knn_vector_query::<C>("field", &[0.0, 0.0], 2)?);
            assert_matches(&searcher, Arc::clone(&kvq), 2)?;
            let score_docs = searcher.search(kvq, 3)?.score_docs;
            assert_equals(&score_docs.len(), &2);
            assert_top_ids_matches(&reader_of(reader), &["id2", "id0"], &score_docs)?;
            Ok(())
        },
    )
}

fn test_search_boost<C: KnnVectorCase>() -> Result<()> {
    with_index(
        get_index_store::<C>("field", &[&[0.0, 1.0], &[1.0, 2.0], &[0.0, 0.0]])?,
        |reader| {
            let searcher = LuceneTestCase::new_searcher(Arc::clone(reader));

            let vector_query = as_query(get_knn_vector_query::<C>("field", &[0.0, 0.0], 10)?);
            let score_docs = searcher.search(Arc::clone(&vector_query), 3)?.score_docs;

            let boost_query: Arc<dyn Query> = Arc::new(BoostQuery::new(vector_query, 3.0)?);
            let boost_score_docs = searcher.search(boost_query, 3)?.score_docs;
            assert_equals(&score_docs.len(), &boost_score_docs.len());

            for i in 0..score_docs.len() {
                let score_doc = &score_docs[i];
                let boost_score_doc = &boost_score_docs[i];

                assert_equals(&score_doc.doc, &boost_score_doc.doc);
                assert_equals_delta(
                    f64::from(score_doc.score * 3.0),
                    f64::from(boost_score_doc.score),
                    0.001,
                );
            }
            Ok(())
        },
    )
}

/// Tests that a AbstractKnnVectorQuery applies the filter query
fn test_simple_filter<C: KnnVectorCase>() -> Result<()> {
    with_index(
        get_index_store::<C>("field", &[&[0.0, 1.0], &[1.0, 2.0], &[0.0, 0.0]])?,
        |reader| {
            let searcher = LuceneTestCase::new_searcher(Arc::clone(reader));
            let filter: Arc<dyn Query> = Arc::new(TermQuery::new(term("id", "id2")));
            let kvq = as_query(C::get_knn_vector_query_filtered(
                "field",
                &[0.0, 0.0],
                10,
                Some(filter),
            )?);
            let top_docs = searcher.search(kvq, 3)?;
            assert_equals(&1, &top_docs.total_hits.value());
            assert_id_matches(&reader_of(reader), "id2", &top_docs.score_docs[0])?;
            Ok(())
        },
    )
}

fn test_filter_with_no_vector_matches<C: KnnVectorCase>() -> Result<()> {
    with_index(
        get_index_store::<C>("field", &[&[0.0, 1.0], &[1.0, 2.0], &[0.0, 0.0]])?,
        |reader| {
            let searcher = LuceneTestCase::new_searcher(Arc::clone(reader));

            let filter: Arc<dyn Query> = Arc::new(TermQuery::new(term("other", "value")));
            let kvq = as_query(C::get_knn_vector_query_filtered(
                "field",
                &[0.0, 0.0],
                10,
                Some(filter),
            )?);
            let top_docs = searcher.search(kvq, 3)?;
            assert_equals(&0, &top_docs.total_hits.value());
            Ok(())
        },
    )
}

fn test_match_all_filter<C: KnnVectorCase>() -> Result<()> {
    with_index(
        get_index_store::<C>("field", &[&[0.0, 1.0], &[1.0, 2.0], &[0.0, 0.0]])?,
        |reader| {
            let searcher = LuceneTestCase::new_searcher(Arc::clone(reader));

            // make sure we don't drop to exact search, even though the filter matches fewer than k docs
            let kvq = C::get_throwing_knn_vector_query(
                "field",
                &[0.0, 0.0],
                10,
                Some(Arc::new(MatchAllDocsQuery::instance())),
            )?;
            let top_docs = searcher.search(kvq, 3)?;
            assert_equals(&3, &top_docs.total_hits.value());
            Ok(())
        },
    )
}

/// testDimensionMismatch
fn test_dimension_mismatch<C: KnnVectorCase>() -> Result<()> {
    with_index(
        get_index_store::<C>("field", &[&[0.0, 1.0], &[1.0, 2.0], &[0.0, 0.0]])?,
        |reader| {
            let searcher = LuceneTestCase::new_searcher(Arc::clone(reader));
            let kvq = as_query(get_knn_vector_query::<C>("field", &[0.0], 1)?);
            let e = expect_throws(IAE, || searcher.search(kvq, 10));
            assert_equals(
                "vector query dimension: 1 differs from field dimension: 2",
                e.message().unwrap_or_default(),
            );
            Ok(())
        },
    )
}

/// testNonVectorField
fn test_non_vector_field<C: KnnVectorCase>() -> Result<()> {
    with_index(
        get_index_store::<C>("field", &[&[0.0, 1.0], &[1.0, 2.0], &[0.0, 0.0]])?,
        |reader| {
            let searcher = LuceneTestCase::new_searcher(Arc::clone(reader));
            assert_matches(
                &searcher,
                as_query(get_knn_vector_query::<C>("xyzzy", &[0.0], 10)?),
                0,
            )?;
            assert_matches(
                &searcher,
                as_query(get_knn_vector_query::<C>("id", &[0.0], 10)?),
                0,
            )?;
            Ok(())
        },
    )
}

/// Test bad parameters
fn test_illegal_arguments<C: KnnVectorCase>() -> Result<()> {
    expect_throws(IAE, || get_knn_vector_query::<C>("xx", &[1.0], 0));
    Ok(())
}

fn test_different_reader<C: KnnVectorCase>() -> Result<()> {
    with_index(
        get_index_store::<C>("field", &[&[0.0, 1.0], &[1.0, 2.0], &[0.0, 0.0]])?,
        |reader| {
            let query = get_knn_vector_query::<C>("field", &[2.0, 3.0], 3)?;
            let dasq = rewrite_once(&query, &LuceneTestCase::new_searcher(Arc::clone(reader)))?;
            let leaves = reader_of(reader).leaves()?;
            let leaf_searcher = LuceneTestCase::new_searcher(leaves[0].leaf_reader());
            expect_throws(ISE, || {
                dasq.create_weight(&leaf_searcher, ScoreMode::COMPLETE, 1.0)
            });
            Ok(())
        },
    )
}

/// `query.rewrite(searcher)`: Rucene's `None` is Java's `this`.
fn rewrite_once<Q: Query + Clone>(query: &Q, searcher: &IndexSearcher) -> Result<Arc<dyn Query>> {
    Ok(query
        .rewrite(searcher)?
        .unwrap_or_else(|| Arc::new(query.clone())))
}

fn test_score_euclidean<C: KnnVectorCase>() -> Result<()> {
    let mut vectors: Vec<Vec<f32>> = Vec::with_capacity(5);
    for j in 0..5 {
        vectors.push(vec![j as f32, j as f32]);
    }
    let contents: Vec<&[f32]> = vectors.iter().map(Vec::as_slice).collect();
    with_index(get_stable_index_store::<C>("field", &contents)?, |reader| {
        let searcher = IndexSearcher::new(reader_of(reader))?;
        let query = get_knn_vector_query::<C>("field", &[2.0, 3.0], 3)?;
        let rewritten = rewrite_once(&query, &searcher)?;
        let weight = searcher.create_weight(rewritten, ScoreMode::COMPLETE, 1.0)?;
        let leaves = reader_of(reader).leaves()?;
        let mut scorer = weight.scorer(&leaves[0])?.expect("scorer");

        // prior to advancing, score is 0
        assert_equals(&-1, &scorer.doc_id());
        expect_throws(AIOOBE, || scorer.score());

        // This is 1 / ((l2distance((2,3), (2, 2)) = 1) + 1) = 0.5
        assert_equals_delta(
            f64::from(1.0f32 / 2.0),
            f64::from(scorer.get_max_score(2)?),
            0.0,
        );
        assert_equals_delta(
            f64::from(1.0f32 / 2.0),
            f64::from(scorer.get_max_score(i32::MAX)?),
            0.0,
        );

        assert_equals(&3, &scorer.iterator().cost());
        let first_doc = scorer.iterator().next_doc()?;
        if first_doc == 1 {
            assert_equals_delta(f64::from(1.0f32 / 6.0), f64::from(scorer.score()?), 0.0);
            assert_equals(&3, &scorer.iterator().advance(3)?);
            assert_equals_delta(f64::from(1.0f32 / 2.0), f64::from(scorer.score()?), 0.0);
            assert_equals(&NO_MORE_DOCS, &scorer.iterator().advance(4)?);
        } else {
            assert_equals(&2, &first_doc);
            assert_equals_delta(f64::from(1.0f32 / 2.0), f64::from(scorer.score()?), 0.0);
            assert_equals(&4, &scorer.iterator().advance(4)?);
            assert_equals_delta(f64::from(1.0f32 / 6.0), f64::from(scorer.score()?), 0.0);
            assert_equals(&NO_MORE_DOCS, &scorer.iterator().advance(5)?);
        }
        expect_throws(AIOOBE, || scorer.score());
        Ok(())
    })
}

/// `new IndexWriter(d, new IndexWriterConfig())`, closed after `body`.
fn with_plain_writer(
    d: &TestDirectory,
    body: impl FnOnce(&IndexWriter) -> Result<()>,
) -> Result<()> {
    let w = IndexWriter::new(d.as_directory(), IndexWriterConfig::new())?;
    let result = body(&w);
    w.close()?;
    result
}

fn test_score_cosine<C: KnnVectorCase>() -> Result<()> {
    let mut d = C::new_directory_for_test()?;
    with_plain_writer(&d, |w| {
        for j in 1..=5 {
            let mut doc = Document::new();
            doc.add(C::get_knn_vector_field_with(
                "field",
                &[j as f32, (j * j) as f32],
                VectorSimilarityFunction::COSINE,
            )?);
            w.add_document(&doc)?;
        }
        Ok(())
    })?;
    let reader = directory_reader::open(d.as_directory())?;
    let result = (|| -> Result<()> {
        assert_equals(&1, &reader_of(&reader).leaves()?.len());
        let searcher = IndexSearcher::new(reader_of(&reader))?;
        let query = get_knn_vector_query::<C>("field", &[2.0, 3.0], 3)?;
        let rewritten = rewrite_once(&query, &searcher)?;
        let weight = searcher.create_weight(rewritten, ScoreMode::COMPLETE, 1.0)?;
        let leaves = reader_of(&reader).leaves()?;
        let mut scorer = weight.scorer(&leaves[0])?.expect("scorer");

        // prior to advancing, score is undefined
        assert_equals(&-1, &scorer.doc_id());
        expect_throws(AIOOBE, || scorer.score());

        /* score0 = ((2,3) * (1, 1) = 5) / (||2, 3|| * ||1, 1|| = sqrt(26)), then
         * normalized by (1 + x) /2.
         */
        let score0 = ((1.0
            + f64::from(2 * 1 + 3 * 1) / f64::from((2 * 2 + 3 * 3) * (1 * 1 + 1 * 1)).sqrt())
            / 2.0) as f32;

        /* score1 = ((2,3) * (2, 4) = 16) / (||2, 3|| * ||2, 4|| = sqrt(260)), then
         * normalized by (1 + x) /2
         */
        let score1 = ((1.0
            + f64::from(2 * 2 + 3 * 4) / f64::from((2 * 2 + 3 * 3) * (2 * 2 + 4 * 4)).sqrt())
            / 2.0) as f32;

        // doc 1 happens to have the maximum score
        assert_equals_delta(
            f64::from(score1),
            f64::from(scorer.get_max_score(2)?),
            0.0001,
        );
        assert_equals_delta(
            f64::from(score1),
            f64::from(scorer.get_max_score(i32::MAX)?),
            0.0001,
        );

        assert_equals(&3, &scorer.iterator().cost());
        assert_equals(&0, &scorer.iterator().next_doc()?);
        // doc 0 has (1, 1)
        assert_equals_delta(f64::from(score0), f64::from(scorer.score()?), 0.0001);
        assert_equals(&1, &scorer.iterator().advance(1)?);
        assert_equals_delta(f64::from(score1), f64::from(scorer.score()?), 0.0001);

        // since topK was 3
        assert_equals(&NO_MORE_DOCS, &scorer.iterator().advance(4)?);
        expect_throws(AIOOBE, || scorer.score());
        Ok(())
    })();
    reader.close()?;
    d.close()?;
    result
}

fn test_score_mip<C: KnnVectorCase>() -> Result<()> {
    with_index(
        get_index_store_with::<C>(
            "field",
            VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT,
            &[&[0.0, 1.0], &[1.0, 2.0], &[0.0, 0.0]],
        )?,
        |reader| {
            let searcher = LuceneTestCase::new_searcher(Arc::clone(reader));
            let kvq = as_query(get_knn_vector_query::<C>("field", &[0.0, -1.0], 10)?);
            assert_matches(&searcher, Arc::clone(&kvq), 3)?;
            let score_docs = searcher.search(kvq, 3)?.score_docs;
            assert_id_matches(&reader_of(reader), "id2", &score_docs[0])?;
            assert_id_matches(&reader_of(reader), "id0", &score_docs[1])?;
            assert_id_matches(&reader_of(reader), "id1", &score_docs[2])?;

            assert_equals_delta(1.0, f64::from(score_docs[0].score), 1e-7);
            assert_equals_delta(
                f64::from(1.0f32 / 2.0),
                f64::from(score_docs[1].score),
                1e-7,
            );
            assert_equals_delta(
                f64::from(1.0f32 / 3.0),
                f64::from(score_docs[2].score),
                1e-7,
            );
            Ok(())
        },
    )
}

/// The body shared by `testExplain` and `testExplainMultipleSegments`.
fn explain_body<C: KnnVectorCase>(commit_each: bool, nomatch_doc: i32) -> Result<()> {
    let mut d = C::new_directory_for_test()?;
    with_plain_writer(&d, |w| {
        for j in 0..5 {
            let mut doc = Document::new();
            doc.add(C::get_knn_vector_field("field", &[j as f32, j as f32])?);
            w.add_document(&doc)?;
            if commit_each {
                w.commit()?;
            }
        }
        Ok(())
    })?;
    let reader = directory_reader::open(d.as_directory())?;
    let result = (|| -> Result<()> {
        let searcher = IndexSearcher::new(reader_of(&reader))?;
        let query = as_query(get_knn_vector_query::<C>("field", &[2.0, 3.0], 3)?);
        let matched = searcher.explain(Arc::clone(&query), 2)?;
        Assert::assert_true(matched.is_match());
        assert_equals(&ExplanationValue::Float(1.0 / 2.0), &matched.value());
        assert_equals(&0, &matched.details().len());
        assert_equals("within top 3 docs", matched.description());

        let nomatch = searcher.explain(query, nomatch_doc)?;
        Assert::assert_false(nomatch.is_match());
        assert_equals(&ExplanationValue::Float(0.0), &nomatch.value());
        assert_equals(&0, &matched.details().len());
        assert_equals("not in top 3 docs", nomatch.description());
        Ok(())
    })();
    reader.close()?;
    d.close()?;
    result
}

fn test_explain<C: KnnVectorCase>() -> Result<()> {
    explain_body::<C>(false, 5)
}

fn test_explain_multiple_segments<C: KnnVectorCase>() -> Result<()> {
    explain_body::<C>(true, 4)
}

/// Test that when vectors are abnormally distributed among segments, we still find the top K
fn test_skewed_index<C: KnnVectorCase>() -> Result<()> {
    /* We have to choose the numbers carefully here so that some segment has more than the expected
     * number of top K documents, but no more than K documents in total (otherwise we might occasionally
     * randomly fail to find one).
     */
    let mut d = C::new_directory_for_test()?;
    with_plain_writer(&d, |w| {
        let mut r = 0;
        for _i in 0..5 {
            for _j in 0..5 {
                let mut doc = Document::new();
                doc.add(C::get_knn_vector_field("field", &[r as f32, r as f32])?);
                doc.add(Box::new(StringField::new(
                    "id",
                    format!("id{r}"),
                    Store::YES,
                )?));
                w.add_document(&doc)?;
                r += 1;
            }
            w.flush()?;
        }
        Ok(())
    })?;
    let reader = directory_reader::open(d.as_directory())?;
    let result = (|| -> Result<()> {
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
        let mut results = searcher.search(
            as_query(get_knn_vector_query::<C>("field", &[0.0, 0.0], 8)?),
            10,
        )?;
        assert_equals(&8, &results.score_docs.len());
        assert_id_matches(&reader_of(&reader), "id0", &results.score_docs[0])?;
        assert_id_matches(&reader_of(&reader), "id7", &results.score_docs[7])?;

        // test some results in the middle of the sequence - also tests docid tiebreaking
        results = searcher.search(
            as_query(get_knn_vector_query::<C>("field", &[10.0, 10.0], 8)?),
            10,
        )?;
        assert_equals(&8, &results.score_docs.len());
        assert_id_matches(&reader_of(&reader), "id10", &results.score_docs[0])?;
        assert_id_matches(&reader_of(&reader), "id6", &results.score_docs[7])?;
        Ok(())
    })();
    reader.close()?;
    d.close()?;
    result
}

/// Tests with random vectors, number of documents, etc. Uses RandomIndexWriter.
fn test_random_consistency_single_threaded<C: KnnVectorCase>() -> Result<()> {
    assert_random_consistency::<C>(false)
}

// @AwaitsFix(bugUrl = "https://github.com/apache/lucene/issues/14180")
fn test_random_consistency_multi_threaded<C: KnnVectorCase>() -> Result<()> {
    assert_random_consistency::<C>(true)
}

fn assert_random_consistency<C: KnnVectorCase>(multi_threaded: bool) -> Result<()> {
    let num_docs = 100;
    let dimension = 4;
    let num_iters = 10;
    let every_doc_has_a_vector = random().next_boolean();
    let r = random();
    let mut d = C::new_directory_for_test()?;
    // To ensure consistency between seeded runs, remove some randomness
    let mut iwc = IndexWriterConfig::with_analyzer(Arc::new(MockAnalyzer::new(&*random())));
    iwc.set_merge_scheduler(Arc::new(SerialMergeScheduler::new()))?;
    iwc.set_merge_policy(Arc::new(NoMergePolicy::new()))?;
    iwc.set_max_buffered_docs(num_docs)?;
    iwc.set_ram_buffer_size_mb(f64::from(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH))?;
    let w = IndexWriter::new(d.as_directory(), iwc)?;
    let written = (|| -> Result<()> {
        for i in 0..num_docs {
            let mut doc = Document::new();
            if every_doc_has_a_vector || random().next_int_bound(10) != 2 {
                doc.add(C::get_knn_vector_field(
                    "field",
                    &C::random_vector(dimension),
                )?);
            }
            w.add_document(&doc)?;
            if r.next_boolean() && i % 50 == 0 {
                w.flush()?;
            }
        }
        Ok(())
    })();
    w.close()?;
    written?;
    let reader = directory_reader::open(d.as_directory())?;
    let result = (|| -> Result<()> {
        let searcher =
            LuceneTestCase::new_searcher_threads(Arc::clone(&reader), true, true, multi_threaded);
        // first get the initial set of docs, and we expect all future queries to be exactly the
        // same
        let k = random().next_int_bound(80) + 1;
        let query = as_query(get_knn_vector_query::<C>(
            "field",
            &C::random_vector(dimension),
            k,
        )?);
        let n = random().next_int_bound(100) + 1;
        let expected_results = searcher.search(Arc::clone(&query), n)?;
        for _i in 0..num_iters {
            let results = searcher.search(Arc::clone(&query), n)?;
            assert_equals(
                &expected_results.total_hits.value(),
                &results.total_hits.value(),
            );
            assert_equals(
                &expected_results.score_docs.len(),
                &results.score_docs.len(),
            );
            for j in 0..results.score_docs.len() {
                assert_equals(
                    &expected_results.score_docs[j].doc,
                    &results.score_docs[j].doc,
                );
                assert_equals_delta(
                    f64::from(expected_results.score_docs[j].score),
                    f64::from(results.score_docs[j].score),
                    f64::from(EPSILON),
                );
            }
        }
        Ok(())
    })();
    reader.close()?;
    d.close()?;
    result
}

/// Tests with random vectors, number of documents, etc. Uses RandomIndexWriter.
fn test_random<C: KnnVectorCase>() -> Result<()> {
    let num_docs = LuceneTestCase::at_least(100);
    let dimension = LuceneTestCase::at_least(5);
    let num_iters = LuceneTestCase::at_least(10);
    let every_doc_has_a_vector = random().next_boolean();
    let mut d = C::new_directory_for_test()?;
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), d.as_directory())?;
    for _i in 0..num_docs {
        let mut doc = Document::new();
        if every_doc_has_a_vector || random().next_int_bound(10) != 2 {
            doc.add(C::get_knn_vector_field(
                "field",
                &C::random_vector(dimension),
            )?);
        }
        w.add_document(&doc)?;
    }
    w.close()?;
    let reader = directory_reader::open(d.as_directory())?;
    let result = (|| -> Result<()> {
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
        for _i in 0..num_iters {
            let k = random().next_int_bound(80) + 1;
            let query = as_query(get_knn_vector_query::<C>(
                "field",
                &C::random_vector(dimension),
                k,
            )?);
            let n = random().next_int_bound(100) + 1;
            let results = searcher.search(query, n)?;
            let expected = n.min(k).min(reader.num_docs());
            // we may get fewer results than requested if there are deletions, but this test doesn't
            // test that
            debug_assert!(!reader.has_deletions());
            assert_equals(&(expected as usize), &results.score_docs.len());
            Assert::assert_true(results.total_hits.value() >= results.score_docs.len() as i64);
            // verify the results are in descending score order
            let mut last = f32::MAX;
            for score_doc in &results.score_docs {
                Assert::assert_true(score_doc.score <= last);
                last = score_doc.score;
            }
        }
        Ok(())
    })();
    reader.close()?;
    d.close()?;
    result
}

/// `new IndexWriterConfig().setCodec(TestUtil.getDefaultCodec())`.
fn default_codec_iwc() -> Result<IndexWriterConfig> {
    let mut iwc = IndexWriterConfig::new();
    iwc.set_codec(TestUtil::get_default_codec())?;
    Ok(iwc)
}

/// Tests with random vectors and a random filter. Uses RandomIndexWriter.
fn test_random_with_filter<C: KnnVectorCase>() -> Result<()> {
    let num_docs = 1000;
    let dimension = LuceneTestCase::at_least(5);
    let num_iters = LuceneTestCase::at_least(10);
    let mut d = C::new_directory_for_test()?;
    // Always use the default kNN format to have predictable behavior around when it hits
    // visitedLimit. This is fine since the test targets AbstractKnnVectorQuery logic, not the kNN
    // format
    // implementation.
    let iwc = default_codec_iwc()?;
    let w = RandomIndexWriter::new_with_config(&*random(), d.as_directory(), iwc)?;
    for i in 0..num_docs {
        let mut doc = Document::new();
        doc.add(C::get_knn_vector_field(
            "field",
            &C::random_vector(dimension),
        )?);
        doc.add(Box::new(NumericDocValuesField::new("tag", i64::from(i))));
        doc.add(Box::new(IntPoint::new("tag", &[i])?));
        w.add_document(&doc)?;
    }
    w.force_merge(1)?;
    w.close()?;

    let reader = directory_reader::open(d.as_directory())?;
    let result = (|| -> Result<()> {
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
        for _i in 0..num_iters {
            let lower = random().next_int_bound(500);

            // Test a filter with cost less than k and check we use exact search
            let filter1 = IntPoint::new_range_query("tag", &[lower], &[lower + 8])?;
            let mut results = searcher.search(
                as_query(C::get_knn_vector_query_filtered(
                    "field",
                    &C::random_vector(dimension),
                    10,
                    Some(Arc::clone(&filter1)),
                )?),
                num_docs,
            )?;
            assert_equals(&9, &results.total_hits.value());
            assert_equals(
                &results.total_hits.value(),
                &(results.score_docs.len() as i64),
            );
            expect_throws(UOE, || {
                searcher.search(
                    C::get_throwing_knn_vector_query(
                        "field",
                        &C::random_vector(dimension),
                        10,
                        Some(Arc::clone(&filter1)),
                    )?,
                    num_docs,
                )
            });

            // Test an unrestrictive filter and check we use approximate search
            let filter3 = IntPoint::new_range_query("tag", &[lower], &[num_docs])?;
            let sorted = searcher.search_with_sort(
                C::get_throwing_knn_vector_query(
                    "field",
                    &C::random_vector(dimension),
                    5,
                    Some(filter3),
                )?,
                num_docs,
                &Sort::new_fields(vec![SortField::new(
                    Some("tag".to_string()),
                    SortFieldType::Int,
                )?])?,
            )?;
            assert_equals(&5, &sorted.total_hits.value());
            assert_equals(
                &sorted.total_hits.value(),
                &(sorted.score_docs.len() as i64),
            );

            for field_doc in &sorted.score_docs {
                let fields = field_doc.fields.as_ref().expect("sorted hit fields");
                assert_equals(&1, &fields.len());

                let tag = match fields[0] {
                    SortValue::Int(tag) => tag,
                    ref other => {
                        return Err(LuceneError::illegal_state(format!(
                            "java.lang.ClassCastException: {other:?}"
                        )))
                    }
                };
                Assert::assert_true(lower <= tag && tag <= num_docs);
            }
            // Test a filter with cost slightly more than k, and check we use exact search as k
            // results are not retrieved from approximate search
            let filter5 = IntPoint::new_range_query("tag", &[lower], &[lower + 11])?;
            results = searcher.search(
                as_query(C::get_knn_vector_query_filtered(
                    "field",
                    &C::random_vector(dimension),
                    10,
                    Some(Arc::clone(&filter5)),
                )?),
                num_docs,
            )?;
            assert_equals(&10, &results.total_hits.value());
            assert_equals(
                &results.total_hits.value(),
                &(results.score_docs.len() as i64),
            );
            expect_throws(UOE, || {
                searcher.search(
                    C::get_capped_results_throwing_knn_vector_query(
                        "field",
                        &C::random_vector(dimension),
                        10,
                        Some(Arc::clone(&filter5)),
                        5,
                    )?,
                    num_docs,
                )
            });
            assert_equals(&10, &results.total_hits.value());
            assert_equals(
                &results.total_hits.value(),
                &(results.score_docs.len() as i64),
            );
        }
        // Test a filter that exhausts visitedLimit in upper levels, and switches to exact search
        // due to extreme edge cases, removing the randomness
        let mut vector = vec![0f32; dimension as usize];
        for i in 0..dimension as usize {
            vector[i] = if i % 2 == 0 { 42.0 } else { 7.0 };
        }
        let filter4 = IntPoint::new_range_query("tag", &[250], &[256])?;
        expect_throws(UOE, || {
            searcher.search(
                C::get_throwing_knn_vector_query("field", &vector, 1, Some(filter4))?,
                num_docs,
            )
        });
        Ok(())
    })();
    reader.close()?;
    d.close()?;
    result
}

/// Tests filtering when all vectors have the same score.
fn test_filter_with_same_score<C: KnnVectorCase>() -> Result<()> {
    let num_docs = 100;
    let dimension = LuceneTestCase::at_least(5);
    let mut d = C::new_directory_for_test()?;
    // Always use the default kNN format to have predictable behavior around when it hits
    // visitedLimit. This is fine since the test targets AbstractKnnVectorQuery logic, not the kNN
    // format
    // implementation.
    let iwc = default_codec_iwc()?;
    let w = IndexWriter::new(d.as_directory(), iwc)?;
    let vector = C::random_vector(dimension);
    for i in 0..num_docs {
        let mut doc = Document::new();
        doc.add(C::get_knn_vector_field("field", &vector)?);
        doc.add(Box::new(IntPoint::new("tag", &[i])?));
        w.add_document(&doc)?;
    }
    w.force_merge(1)?;
    w.close()?;

    let reader = directory_reader::open(d.as_directory())?;
    let result = (|| -> Result<()> {
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
        let lower = random().next_int_bound(50);
        let size = 5;

        // Test a restrictive filter, which usually performs exact search
        let filter1 = IntPoint::new_range_query("tag", &[lower], &[lower + 6])?;
        let mut results = searcher.search(
            as_query(C::get_knn_vector_query_filtered(
                "field",
                &C::random_vector(dimension),
                size,
                Some(filter1),
            )?),
            size,
        )?;
        assert_equals(&(size as usize), &results.score_docs.len());

        // Test an unrestrictive filter, which usually performs approximate search
        let filter2 = IntPoint::new_range_query("tag", &[lower], &[num_docs])?;
        results = searcher.search(
            as_query(C::get_knn_vector_query_filtered(
                "field",
                &C::random_vector(dimension),
                size,
                Some(filter2),
            )?),
            size,
        )?;
        assert_equals(&(size as usize), &results.score_docs.len());
        Ok(())
    })();
    reader.close()?;
    d.close()?;
    result
}

/// `try (Directory dir = newDirectoryForTest(); IndexWriter w = new IndexWriter(dir, iwc)) { body }`.
fn with_dir_and_writer<C: KnnVectorCase>(
    iwc: IndexWriterConfig,
    body: impl FnOnce(&TestDirectory, &Arc<IndexWriter>) -> Result<()>,
) -> Result<()> {
    let mut dir = C::new_directory_for_test()?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let result = body(&dir, &w);
    w.close()?;
    dir.close()?;
    result
}

fn test_deletes<C: KnnVectorCase>() -> Result<()> {
    with_dir_and_writer::<C>(LuceneTestCase::new_index_writer_config(), |dir, w| {
        let num_docs = LuceneTestCase::at_least(100);
        let dim = 30;
        for i in 0..num_docs {
            let mut d = Document::new();
            d.add(Box::new(StringField::new(
                "index",
                i.to_string(),
                Store::YES,
            )?));
            if RandomizedTest::frequently() {
                d.add(C::get_knn_vector_field("vector", &C::random_vector(dim))?);
            }
            w.add_document(&d)?;
        }
        w.commit()?;

        // Delete some documents at random, both those with and without vectors
        let mut to_delete: HashSet<String> = HashSet::new();
        let mut to_delete_terms = Vec::new();
        for _i in 0..25 {
            let index = random().next_int_bound(num_docs);
            if to_delete.insert(index.to_string()) {
                to_delete_terms.push(term("index", &index.to_string()));
            }
        }
        w.delete_documents_by_terms(to_delete_terms)?;
        w.commit()?;

        let hits = 50;
        let reader = directory_reader::open(dir.as_directory())?;
        let result = (|| -> Result<()> {
            let mut all_ids: HashSet<String> = HashSet::new();
            let searcher = IndexSearcher::new(reader_of(&reader))?;
            let query = as_query(get_knn_vector_query::<C>(
                "vector",
                &C::random_vector(dim),
                hits,
            )?);
            let top_docs = searcher.search(query, num_docs)?;
            let stored_fields = reader.stored_fields()?;
            let fields_to_load: HashSet<String> = ["index".to_string()].into_iter().collect();
            for score_doc in &top_docs.score_docs {
                let doc = stored_fields.document_fields(score_doc.doc, &fields_to_load)?;
                let index = doc.get("index").unwrap_or_else(|| "null".into());
                Assert::assert_false_msg(
                    &format!("search returned a deleted document: {index}"),
                    to_delete.contains(index.as_str()),
                );
                all_ids.insert(index.to_string());
            }
            assert_equals_msg(
                Some("search missed some documents"),
                &(hits as usize),
                &all_ids.len(),
            );
            Ok(())
        })();
        reader.close()?;
        result
    })
}

fn test_all_deletes<C: KnnVectorCase>() -> Result<()> {
    with_dir_and_writer::<C>(LuceneTestCase::new_index_writer_config(), |dir, w| {
        let num_docs = LuceneTestCase::at_least(100);
        let dim = 30;
        for _i in 0..num_docs {
            let mut d = Document::new();
            d.add(C::get_knn_vector_field("vector", &C::random_vector(dim))?);
            w.add_document(&d)?;
        }
        w.commit()?;

        w.delete_documents_by_queries(vec![Arc::new(MatchAllDocsQuery::instance())])?;
        w.commit()?;

        let reader = directory_reader::open(dir.as_directory())?;
        let result = (|| -> Result<()> {
            let searcher = IndexSearcher::new(reader_of(&reader))?;
            let query = as_query(get_knn_vector_query::<C>(
                "vector",
                &C::random_vector(dim),
                num_docs,
            )?);
            let top_docs = searcher.search(query, num_docs)?;
            assert_equals(&0, &top_docs.score_docs.len());
            Ok(())
        })();
        reader.close()?;
        result
    })
}

// Test ghost fields, that have a field info but no values
fn test_merge_away_all_values<C: KnnVectorCase>() -> Result<()> {
    let dim = 30;
    with_dir_and_writer::<C>(LuceneTestCase::new_index_writer_config(), |_dir, w| {
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new("id", "0".into(), Store::NO)?));
        w.add_document(&doc)?;
        doc = Document::new();
        doc.add(Box::new(StringField::new("id", "1".into(), Store::NO)?));
        doc.add(C::get_knn_vector_field("field", &C::random_vector(dim))?);
        w.add_document(&doc)?;
        w.commit()?;
        w.delete_documents_by_terms(vec![term("id", "1")])?;
        w.force_merge(1)?;

        let reader = directory_reader::open_with_writer(&**w)?;
        let result = (|| -> Result<()> {
            let leaf_reader = LuceneTestCase::get_only_leaf_reader(reader_of(&reader))?;
            let infos = leaf_reader.get_field_infos();
            let fi = infos.field_info("field");
            assert_not_null(&fi);
            let fi = fi.expect("checked");
            let has_no_docs = match fi.get_vector_encoding() {
                VectorEncoding::BYTE => {
                    let vector_values = leaf_reader.get_byte_vector_values("field")?;
                    assert_not_null(&vector_values);
                    vector_values.expect("checked").iterator()?.next_doc()?
                }
                VectorEncoding::FLOAT32 => {
                    let vector_values = leaf_reader.get_float_vector_values("field")?;
                    assert_not_null(&vector_values);
                    vector_values.expect("checked").iterator()?.next_doc()?
                }
            };
            assert_equals(&NO_MORE_DOCS, &has_no_docs);
            Ok(())
        })();
        reader.close()?;
        result
    })
}

/// Check that the query behaves reasonably when using a custom filter reader where there are no
/// live docs.
fn test_no_live_docs_reader<C: KnnVectorCase>() -> Result<()> {
    let iwc = LuceneTestCase::new_index_writer_config();
    with_dir_and_writer::<C>(iwc, |dir, w| {
        let num_docs = 10;
        let dim = 30;
        for i in 0..num_docs {
            let mut d = Document::new();
            d.add(Box::new(StringField::new(
                "index",
                i.to_string(),
                Store::NO,
            )?));
            d.add(C::get_knn_vector_field("vector", &C::random_vector(dim))?);
            w.add_document(&d)?;
        }
        w.commit()?;

        let reader = directory_reader::open(dir.as_directory())?;
        let result = (|| -> Result<()> {
            let wrapped_reader = no_live_docs_directory_reader(Arc::clone(&reader))?;
            let searcher = IndexSearcher::new(wrapped_reader as Arc<dyn IndexReader>)?;
            let query = as_query(get_knn_vector_query::<C>(
                "vector",
                &C::random_vector(dim),
                num_docs,
            )?);
            let top_docs = searcher.search(query, num_docs)?;
            assert_equals(&0, &top_docs.score_docs.len());
            Ok(())
        })();
        reader.close()?;
        result
    })
}

/// Test that AbstractKnnVectorQuery optimizes the case where the filter query is backed by {@link
/// BitSetIterator}.
fn test_bit_set_query<C: KnnVectorCase>() -> Result<()> {
    let iwc = LuceneTestCase::new_index_writer_config();
    with_dir_and_writer::<C>(iwc, |dir, w| {
        let num_docs = 100;
        let dim = 30;
        for _i in 0..num_docs {
            let mut d = Document::new();
            d.add(C::get_knn_vector_field("vector", &C::random_vector(dim))?);
            w.add_document(&d)?;
        }
        w.commit()?;

        let reader = directory_reader::open(dir.as_directory())?;
        let result = (|| -> Result<()> {
            let searcher = IndexSearcher::new(reader_of(&reader))?;

            let filter: Arc<dyn Query> = Arc::new(ThrowingBitSetQuery::new(FixedBitSet::new(
                num_docs as usize,
            )));
            expect_throws(UOE, || {
                searcher.search(
                    as_query(C::get_knn_vector_query_filtered(
                        "vector",
                        &C::random_vector(dim),
                        10,
                        Some(filter),
                    )?),
                    num_docs,
                )
            });
            Ok(())
        })();
        reader.close()?;
        result
    })
}

/// Test functionality of {@link TimeLimitingKnnCollectorManager}.
fn test_time_limiting_knn_collector_manager<C: KnnVectorCase>() -> Result<()> {
    with_index(
        get_index_store::<C>("field", &[&[0.0, 1.0], &[1.0, 2.0], &[0.0, 0.0]])?,
        |reader| {
            let searcher = LuceneTestCase::new_searcher(Arc::clone(reader));

            let delegate: Arc<dyn KnnCollectorManager> =
                Arc::new(TopKnnCollectorManager::new(3, &searcher));

            // A collector manager with no timeout
            let no_timeout_manager =
                TimeLimitingKnnCollectorManager::new(Arc::clone(&delegate), None);
            let mut no_timeout_collector = no_timeout_manager.new_collector(
                i32::MAX,
                None,
                &searcher.get_leaf_contexts()[0],
            )?;

            // Check that a normal collector is created without timeout
            Assert::assert_false(
                no_timeout_collector
                    .knn_as_any()
                    .is_some_and(|any| any.is::<TimeLimitingKnnCollector>()),
            );
            no_timeout_collector.collect(0, 0.0);
            Assert::assert_false(no_timeout_collector.early_terminated());

            // Check that results are complete
            let no_timeout_top_docs = no_timeout_collector.top_docs();
            assert_equals(
                &TotalHitsRelation::EQUAL_TO,
                &no_timeout_top_docs.total_hits.relation(),
            );
            assert_equals(&1, &no_timeout_top_docs.score_docs.len());

            // A collector manager that immediately times out
            let timeout_manager = TimeLimitingKnnCollectorManager::new(
                delegate,
                Some(Arc::new(ConstantTimeout(true))),
            );
            let mut timeout_collector =
                timeout_manager.new_collector(i32::MAX, None, &searcher.get_leaf_contexts()[0])?;

            // Check that a time limiting collector is created, which returns partial results
            Assert::assert_false(
                timeout_collector
                    .knn_as_any()
                    .is_some_and(|any| any.is::<TopKnnCollector>()),
            );
            timeout_collector.collect(0, 0.0);
            Assert::assert_true(timeout_collector.early_terminated());

            // Check that partial results are returned
            let timeout_top_docs = timeout_collector.top_docs();
            assert_equals(
                &TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO,
                &timeout_top_docs.total_hits.relation(),
            );
            assert_equals(&1, &timeout_top_docs.score_docs.len());
            Ok(())
        },
    )
}

/// `QueryTimeout` lambda `() -> value`.
#[derive(Debug)]
pub struct ConstantTimeout(pub bool);

impl QueryTimeout for ConstantTimeout {
    fn should_exit(&self) -> bool {
        self.0
    }
}

/// Test that the query times out correctly.
fn test_timeout<C: KnnVectorCase>() -> Result<()> {
    with_index(
        get_index_store::<C>("field", &[&[0.0, 1.0], &[1.0, 2.0], &[0.0, 0.0]])?,
        |reader| {
            let mut searcher = LuceneTestCase::new_searcher(Arc::clone(reader));

            let query = as_query(get_knn_vector_query::<C>("field", &[0.0, 1.0], 2)?);
            let exact_query = as_query(C::get_knn_vector_query_filtered(
                "field",
                &[0.0, 1.0],
                10,
                Some(Arc::new(MatchAllDocsQuery::instance())),
            )?);

            assert_equals(&2, &searcher.count(Arc::clone(&query))?); // Expect some results without timeout
            assert_equals(&3, &searcher.count(Arc::clone(&exact_query))?); // Same for exact search

            searcher.set_timeout(Some(Arc::new(ConstantTimeout(true)))); // Immediately timeout
            assert_equals(&0, &searcher.count(Arc::clone(&query))?); // Expect no results with the timeout
            assert_equals(&0, &searcher.count(Arc::clone(&exact_query))?); // Same for exact search

            searcher.set_timeout(Some(Arc::new(CountingQueryTimeout::new(1)))); // Only score 1 doc
                                                                                // Note: We get partial results when the HNSW graph has 1 layer, but no results for > 1 layer
                                                                                // because the timeout is exhausted while finding the best entry node for the last level
                                                                                // assertThat(searcher.count(query), lessThanOrEqualTo(1))
            let count = searcher.count(Arc::clone(&query))?;
            Assert::assert_true_msg(
                &format!("Expected: a value less than or equal to <1>\n     but: <{count}> was greater than <1>"),
                count <= 1,
            );

            searcher.set_timeout(Some(Arc::new(CountingQueryTimeout::new(1)))); // Only score 1 doc
            let count = searcher.count(exact_query)?;
            Assert::assert_true_msg(
                &format!("Expected: a value less than or equal to <1>\n     but: <{count}> was greater than <1>"),
                count <= 1,
            );
            Ok(())
        },
    )
}

/// Creates a new directory and adds documents with the given vectors as kNN vector fields
pub fn get_index_store<C: KnnVectorCase>(
    field: &str,
    contents: &[&[f32]],
) -> Result<TestDirectory> {
    get_index_store_with::<C>(field, VectorSimilarityFunction::EUCLIDEAN, contents)
}

/// Creates a new directory and adds documents with the given vectors with similarity as kNN vector
/// fields
pub fn get_index_store_with<C: KnnVectorCase>(
    field: &str,
    vector_similarity_function: VectorSimilarityFunction,
    contents: &[&[f32]],
) -> Result<TestDirectory> {
    let index_store = C::new_directory_for_test()?;
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), index_store.as_directory())?;
    for (i, content) in contents.iter().enumerate() {
        let mut doc = Document::new();
        doc.add(C::get_knn_vector_field_with(
            field,
            content,
            vector_similarity_function,
        )?);
        doc.add(Box::new(StringField::new(
            "id",
            format!("id{i}"),
            Store::YES,
        )?));
        writer.add_document(&doc)?;
        if RandomizedTest::random_boolean() {
            // Add some documents without a vector
            let mut j = 0;
            while j < RandomizedTest::random_int_between(1, 5) {
                doc = Document::new();
                doc.add(Box::new(StringField::new(
                    "other",
                    "value".into(),
                    Store::NO,
                )?));
                // Add fields that will be matched by our test filters but won't have vectors
                doc.add(Box::new(StringField::new(
                    "id",
                    format!("id{j}"),
                    Store::YES,
                )?));
                writer.add_document(&doc)?;
                j += 1;
            }
        }
    }
    // Add some documents without a vector
    for _i in 0..5 {
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new(
            "other",
            "value".into(),
            Store::NO,
        )?));
        writer.add_document(&doc)?;
    }
    writer.close()?;
    Ok(index_store)
}

/// Creates a new directory and adds documents with the given vectors as kNN vector fields,
/// preserving the order of the added documents.
fn get_stable_index_store<C: KnnVectorCase>(
    field: &str,
    contents: &[&[f32]],
) -> Result<TestDirectory> {
    let index_store = C::new_directory_for_test()?;
    with_plain_writer(&index_store, |writer| {
        for (i, content) in contents.iter().enumerate() {
            let mut doc = Document::new();
            doc.add(C::get_knn_vector_field(field, content)?);
            doc.add(Box::new(StringField::new(
                "id",
                format!("id{i}"),
                Store::YES,
            )?));
            writer.add_document(&doc)?;
        }
        // Add some documents without a vector
        for _i in 0..5 {
            let mut doc = Document::new();
            doc.add(Box::new(StringField::new(
                "other",
                "value".into(),
                Store::NO,
            )?));
            writer.add_document(&doc)?;
        }
        Ok(())
    })?;
    Ok(index_store)
}

fn assert_matches(
    searcher: &IndexSearcher,
    q: Arc<dyn Query>,
    expected_matches: usize,
) -> Result<()> {
    let result = searcher.search(q, 1000)?.score_docs;
    assert_equals(&expected_matches, &result.len());
    Ok(())
}

pub fn assert_id_matches(
    reader: &Arc<dyn IndexReader>,
    expected_id: &str,
    score_doc: &ScoreDoc,
) -> Result<()> {
    let actual_id = reader.stored_fields()?.document(score_doc.doc)?.get("id");
    assert_equals(
        &Some(expected_id.to_string()),
        &actual_id.map(|s| s.to_string()),
    );
    Ok(())
}

pub fn assert_top_ids_matches(
    reader: &Arc<dyn IndexReader>,
    expected_ids: &[&str],
    score_docs: &[ScoreDoc],
) -> Result<()> {
    let expected_ids: HashSet<Option<String>> =
        expected_ids.iter().map(|s| Some(s.to_string())).collect();
    let mut actual_ids: HashSet<Option<String>> = HashSet::new();
    for score_doc in score_docs {
        actual_ids.insert(
            reader
                .stored_fields()?
                .document(score_doc.doc)?
                .get("id")
                .map(|s| s.to_string()),
        );
    }
    assert_equals(&expected_ids.len(), &actual_ids.len());
    assert_equals(&expected_ids, &actual_ids);
    Ok(())
}

/// `assertDocScoreQueryToString(Query query)`.
pub fn assert_doc_score_query_to_string(query: &dyn Query) {
    let query_string = query.to_query_string("ignored");
    // The string should contain matching docIds and their score.
    // Since a forceMerge could occur in this test, we must not assert that a specific doc_id is
    // matched
    // But that instead the string format is expected and that the max score is 1.0
    Assert::assert_true(matches_doc_score_query_string(&query_string));
}

/// `queryString.matches("DocAndScoreQuery\\[\\d+,...]\\[\\d+.\\d+,...],1.0")`.
fn matches_doc_score_query_string(s: &str) -> bool {
    fn digits(s: &str) -> Option<&str> {
        let n = s.bytes().take_while(u8::is_ascii_digit).count();
        (n > 0).then(|| &s[n..])
    }
    // `.` in the Java pattern matches any character but a line terminator.
    fn any_char(s: &str) -> Option<&str> {
        let mut chars = s.chars();
        match chars.next() {
            Some(c)
                if c != '\n'
                    && c != '\r'
                    && c != '\u{85}'
                    && c != '\u{2028}'
                    && c != '\u{2029}' =>
            {
                Some(chars.as_str())
            }
            _ => None,
        }
    }
    let parse = || -> Option<()> {
        let s = s.strip_prefix("DocAndScoreQuery[")?;
        let s = digits(s)?;
        let s = s.strip_prefix(',')?;
        let s = any_char(any_char(any_char(s)?)?)?;
        let s = s.strip_prefix("][")?;
        let s = digits(s)?;
        let s = any_char(s)?;
        let s = digits(s)?;
        let s = s.strip_prefix(',')?;
        let s = any_char(any_char(any_char(s)?)?)?;
        let s = s.strip_prefix("],1")?;
        let s = any_char(s)?;
        (s == "0").then_some(())
    };
    // `\d+` is greedy with backtracking; the inputs are digit runs followed by
    // a non-digit separator, so the greedy match is the only one.
    parse().is_some()
}

// -----------------------------------------------------------------------------
// NoLiveDocsDirectoryReader / NoLiveDocsLeafReader
// -----------------------------------------------------------------------------

/// The anonymous `SubReaderWrapper` of `NoLiveDocsDirectoryReader`.
struct NoLiveDocsSubReaderWrapper;

impl SubReaderWrapper for NoLiveDocsSubReaderWrapper {
    fn wrap(&self, reader: Arc<dyn LeafReader>) -> Result<Arc<dyn LeafReader>> {
        Ok(Arc::new(NoLiveDocsLeafReader::new(reader)?))
    }
}

/// `NoLiveDocsDirectoryReader.doWrapDirectoryReader`.
struct NoLiveDocsDirectoryReaderWrapper;

impl DirectoryReaderWrapper for NoLiveDocsDirectoryReaderWrapper {
    fn wrap(&self, inner: Arc<dyn DirectoryReader>) -> Result<Arc<dyn DirectoryReader>> {
        no_live_docs_directory_reader(inner)
    }

    fn subclass_simple_name(&self) -> &'static str {
        "NoLiveDocsDirectoryReader"
    }
}

/// `new NoLiveDocsDirectoryReader(in)`: a `FilterDirectoryReader` whose
/// `getReaderCacheHelper()` is `in.getReaderCacheHelper()` (the forwarding
/// choice of [`DirectoryReaderWrapper::uses_delegating_cache_helper`]).
fn no_live_docs_directory_reader(
    r#in: Arc<dyn DirectoryReader>,
) -> Result<Arc<dyn DirectoryReader>> {
    Ok(Arc::new(FilterDirectoryReader::new(
        r#in,
        Arc::new(NoLiveDocsSubReaderWrapper),
        Arc::new(NoLiveDocsDirectoryReaderWrapper),
    )?))
}

/// `private static class NoLiveDocsLeafReader extends FilterLeafReader`.
struct NoLiveDocsLeafReader {
    /// The `FilterLeafReader` this class extends.
    inner: FilterLeafReader,
    /// Java's protected `in`.
    r#in: Arc<dyn LeafReader>,
}

impl NoLiveDocsLeafReader {
    fn new(r#in: Arc<dyn LeafReader>) -> Result<Self> {
        Ok(NoLiveDocsLeafReader {
            inner: FilterLeafReader::new(Arc::clone(&r#in))?,
            r#in,
        })
    }
}

impl fmt::Debug for NoLiveDocsLeafReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NoLiveDocsLeafReader")
            .finish_non_exhaustive()
    }
}

impl fmt::Display for NoLiveDocsLeafReader {
    /// The inherited `FilterLeafReader.toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.inner, f)
    }
}

impl LeafReader for NoLiveDocsLeafReader {
    fn core(&self) -> &rucene::index::IndexReaderCore {
        LeafReader::core(&self.inner)
    }
    fn term_vectors(&self) -> Result<Box<dyn rucene::index::TermVectors>> {
        LeafReader::term_vectors(&self.inner)
    }
    fn num_docs(&self) -> i32 {
        0
    }
    fn max_doc(&self) -> i32 {
        LeafReader::max_doc(&self.inner)
    }
    fn stored_fields(&self) -> Result<Box<dyn rucene::index::StoredFields>> {
        LeafReader::stored_fields(&self.inner)
    }
    fn do_close(&self) -> Result<()> {
        LeafReader::do_close(&self.inner)
    }
    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        self.r#in.get_reader_cache_helper()
    }
    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        self.r#in.get_core_cache_helper()
    }
    fn terms(&self, field: &str) -> Result<Option<Box<dyn rucene::index::Terms>>> {
        self.inner.terms(field)
    }
    fn get_numeric_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::NumericDocValues>>> {
        self.inner.get_numeric_doc_values(field)
    }
    fn get_binary_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::BinaryDocValues>>> {
        self.inner.get_binary_doc_values(field)
    }
    fn get_sorted_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::SortedDocValues>>> {
        self.inner.get_sorted_doc_values(field)
    }
    fn get_sorted_numeric_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::SortedNumericDocValues>>> {
        self.inner.get_sorted_numeric_doc_values(field)
    }
    fn get_sorted_set_doc_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::SortedSetDocValues>>> {
        self.inner.get_sorted_set_doc_values(field)
    }
    fn get_norm_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::NumericDocValues>>> {
        self.inner.get_norm_values(field)
    }
    fn get_doc_values_skipper(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::DocValuesSkipper>>> {
        self.inner.get_doc_values_skipper(field)
    }
    fn get_float_vector_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::FloatVectorValues>>> {
        self.inner.get_float_vector_values(field)
    }
    fn get_byte_vector_values(
        &self,
        field: &str,
    ) -> Result<Option<Box<dyn rucene::index::ByteVectorValues>>> {
        self.inner.get_byte_vector_values(field)
    }
    fn search_nearest_vectors(
        &self,
        field: &str,
        target: &[f32],
        collector: &mut dyn rucene::search::knn::KnnCollector,
        accept_docs: &mut dyn rucene::search::AcceptDocs,
    ) -> Result<()> {
        self.inner
            .search_nearest_vectors(field, target, collector, accept_docs)
    }
    fn search_nearest_vectors_byte(
        &self,
        field: &str,
        target: &[u8],
        collector: &mut dyn rucene::search::knn::KnnCollector,
        accept_docs: &mut dyn rucene::search::AcceptDocs,
    ) -> Result<()> {
        self.inner
            .search_nearest_vectors_byte(field, target, collector, accept_docs)
    }
    fn get_field_infos(&self) -> rucene::index::FieldInfos {
        self.inner.get_field_infos()
    }
    fn get_live_docs(&self) -> Option<Box<dyn rucene::util::Bits>> {
        Some(Box::new(MatchNoBits::new(self.r#in.max_doc() as usize)))
    }
    fn get_point_values(&self, field: &str) -> Result<Option<Box<dyn rucene::index::PointValues>>> {
        self.inner.get_point_values(field)
    }
    fn check_integrity(&self) -> Result<()> {
        self.inner.check_integrity()
    }
    fn get_meta_data(&self) -> rucene::index::LeafMetaData {
        self.inner.get_meta_data()
    }
}

// -----------------------------------------------------------------------------
// ThrowingBitSetQuery
// -----------------------------------------------------------------------------

/// `static class ThrowingBitSetQuery extends Query`.
#[derive(Debug, Clone)]
pub struct ThrowingBitSetQuery {
    docs: Arc<FixedBitSet>,
}

impl ThrowingBitSetQuery {
    pub fn new(docs: FixedBitSet) -> Self {
        ThrowingBitSetQuery {
            docs: Arc::new(docs),
        }
    }
}

/// The anonymous `ConstantScoreWeight` of `ThrowingBitSetQuery.createWeight`.
#[derive(Debug)]
struct ThrowingBitSetWeight {
    score_mode: ScoreMode,
    docs: Arc<FixedBitSet>,
}

/// The anonymous `BitSetIterator` subclass whose `getBitSet()` throws.
struct ThrowingGetBitSet;

impl BitSetIteratorOverrides for ThrowingGetBitSet {
    fn get_bit_set<'a>(&'a self, _iterator: &'a BitSetIterator) -> &'a Arc<dyn BitSet> {
        rethrow_java(LuceneError::UnsupportedOperation(
            "reusing BitSet is not supported".to_string(),
        ))
    }
}

impl ConstantScoreWeightImpl for ThrowingBitSetWeight {
    fn scorer_supplier(
        &self,
        _context: &LeafReaderContext,
        score: f32,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let mut bit_set_iterator = BitSetIterator::new(
            Arc::clone(&self.docs) as Arc<dyn BitSet>,
            self.docs.approximate_cardinality() as i64,
        )?;
        bit_set_iterator.set_overrides(Arc::new(ThrowingGetBitSet));
        let scorer =
            ConstantScoreScorer::from_iterator(score, self.score_mode, Box::new(bit_set_iterator));
        Ok(Some(Box::new(DefaultScorerSupplier::new(Box::new(scorer)))))
    }

    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        false
    }
}

impl Query for ThrowingBitSetQuery {
    fn create_weight(
        &self,
        _searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        Ok(Arc::new(ConstantScoreWeight::new(
            Arc::new(self.clone()),
            boost,
            ThrowingBitSetWeight {
                score_mode,
                docs: Arc::clone(&self.docs),
            },
        )))
    }

    fn visit(&self, _visitor: &mut dyn QueryVisitor) {}

    fn to_query_string(&self, _field: &str) -> String {
        "throwingBitSetQuery".to_string()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
            && other
                .as_any()
                .downcast_ref::<ThrowingBitSetQuery>()
                .is_some_and(|o| *self.docs == *o.docs)
    }

    fn query_hash(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.docs.hash(&mut hasher);
        31u64
            .wrapping_mul(self.class_hash())
            .wrapping_add(hasher.finish())
    }
}

fn test_same_field_different_formats<C: KnnVectorCase>() -> Result<()> {
    let mut directory = C::new_directory_for_test()?;
    let result = (|| -> Result<()> {
        let mock_analyzer = Arc::new(MockAnalyzer::new(&*random()));
        let mut iwc = LuceneTestCase::new_index_writer_config_analyzer(
            Arc::clone(&mock_analyzer) as Arc<dyn rucene::analysis::Analyzer>
        );
        let format1: Arc<dyn KnnVectorsFormat> =
            LuceneTestCase::random_vector_format(VectorEncoding::FLOAT32);
        let format2: Arc<dyn KnnVectorsFormat> =
            LuceneTestCase::random_vector_format(VectorEncoding::FLOAT32);
        iwc.set_codec(TestUtil::always_knn_vectors_format(format1))?;

        {
            let iwriter = IndexWriter::new(directory.as_directory(), iwc)?;
            let r = (|| -> Result<()> {
                let mut doc = Document::new();
                doc.add(C::get_knn_vector_field("field1", &[1.0, 1.0, 1.0])?);
                iwriter.add_document(&doc)?;

                doc.clear();
                doc.add(C::get_knn_vector_field("field1", &[1.0, 2.0, 3.0])?);
                iwriter.add_document(&doc)?;
                iwriter.commit()?;
                Ok(())
            })();
            iwriter.close()?;
            r?;
        }

        let mut iwc = LuceneTestCase::new_index_writer_config_analyzer(
            mock_analyzer as Arc<dyn rucene::analysis::Analyzer>,
        );
        iwc.set_codec(TestUtil::always_knn_vectors_format(format2))?;

        {
            let iwriter = IndexWriter::new(directory.as_directory(), iwc)?;
            let r = (|| -> Result<()> {
                let mut doc = Document::new();
                doc.clear();
                doc.add(C::get_knn_vector_field("field1", &[1.0, 1.0, 2.0])?);
                iwriter.add_document(&doc)?;

                doc.clear();
                doc.add(C::get_knn_vector_field("field1", &[4.0, 5.0, 6.0])?);
                iwriter.add_document(&doc)?;
                iwriter.commit()?;
                Ok(())
            })();
            iwriter.close()?;
            r?;
        }

        let ireader = directory_reader::open(directory.as_directory())?;
        let r = (|| -> Result<()> {
            let vector_query = as_query(get_knn_vector_query::<C>("field1", &[1.0, 2.0, 3.0], 10)?);
            let hits1 = IndexSearcher::new(reader_of(&ireader))?.search(vector_query, 4)?;
            assert_equals(&4, &hits1.score_docs.len());
            Ok(())
        })();
        ireader.close()?;
        r
    })();
    directory.close()?;
    result
}

/// `private static class CountingQueryTimeout implements QueryTimeout`.
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

fn test_strategy<C: KnnVectorCase>() -> Result<()> {
    let vector = get_knn_vector_query::<C>("vector", &C::random_vector(10), 3)?;
    assert_not_null(&vector.knn_base().get_search_strategy());
    Assert::assert_true(matches!(
        vector.knn_base().get_search_strategy(),
        Some(KnnSearchStrategy::Hnsw(_))
    ));
    Ok(())
}
