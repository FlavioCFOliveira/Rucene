//! Port of `lucene/core/src/test/org/apache/lucene/search/TestSeededKnnFloatVectorQuery.java`.
//!
//! `TestSeededKnnFloatVectorQuery extends BaseKnnVectorQueryTestCase`: the
//! inherited test methods are bound from `base_knn_vector_query_test_case`.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::IntPoint;
use rucene::error::Result;
use rucene::index::{directory_reader, IndexableField, VectorSimilarityFunction};
use rucene::search::seeded_knn_vector_query::SeededKnnVectorQuery;
use rucene::search::{AbstractKnnVectorQueryImpl, KnnFloatVectorQuery, MatchAllDocsQuery, Query};
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::base_knn_vector_query_test_case::{self as base, KnnVectorCase};
use crate::core_search::test_knn_float_vector_query::{
    CappedResultsThrowingKnnVectorQuery, KnnFloatCase, ThrowingKnnVectorQuery,
};
use crate::core_search::test_seeded_knn_byte_vector_query::{
    assert_descending, constant_timeout, index_random_with_seed, index_tagged_docs, match_none,
    random_filter, random_timeout, seed_count, AssertingSeededKnnVectorQuery,
};
use crate::core_support::{assert_equals, class, random, test};

/// `TestSeededKnnFloatVectorQuery extends BaseKnnVectorQueryTestCase`.
pub struct SeededKnnFloatCase;

/// `TestSeededKnnFloatVectorQuery`.
pub fn class_def() -> Arc<Class> {
    let mut methods = base::methods::<SeededKnnFloatCase>();
    methods.extend([
        test("testSeedWithTimeout", test_seed_with_timeout),
        test("testRandomWithSeed", test_random_with_seed),
    ]);
    class(
        "org.apache.lucene.search.TestSeededKnnFloatVectorQuery",
        methods,
    )
}

impl KnnVectorCase for SeededKnnFloatCase {
    type Q = SeededKnnVectorQuery;

    fn get_knn_vector_query_filtered(
        field: &str,
        query: &[f32],
        k: i32,
        query_filter: Option<Arc<dyn Query>>,
    ) -> Result<SeededKnnVectorQuery> {
        let knn_query = KnnFloatVectorQuery::with_filter(field, query.to_vec(), k, query_filter)?;
        SeededKnnVectorQuery::from_float_query(Arc::new(knn_query), match_none())
    }

    fn get_throwing_knn_vector_query(
        field: &str,
        vec: &[f32],
        k: i32,
        query: Option<Arc<dyn Query>>,
    ) -> Result<Arc<dyn Query>> {
        let knn_query = ThrowingKnnVectorQuery::new(field, vec, k, query)?;
        Ok(Arc::new(SeededKnnVectorQuery::from_float_query(
            Arc::new(knn_query),
            match_none(),
        )?))
    }

    fn get_capped_results_throwing_knn_vector_query(
        field: &str,
        vec: &[f32],
        k: i32,
        query: Option<Arc<dyn Query>>,
        max_results: i32,
    ) -> Result<Arc<dyn Query>> {
        let knn_query =
            CappedResultsThrowingKnnVectorQuery::new(field, vec, k, query, max_results)?;
        Ok(Arc::new(SeededKnnVectorQuery::from_float_query(
            Arc::new(knn_query),
            match_none(),
        )?))
    }

    fn random_vector(dim: i32) -> Vec<f32> {
        KnnFloatCase::random_vector(dim)
    }

    fn get_knn_vector_field_with(
        name: &str,
        vector: &[f32],
        similarity_function: VectorSimilarityFunction,
    ) -> Result<Box<dyn IndexableField>> {
        KnnFloatCase::get_knn_vector_field_with(name, vector, similarity_function)
    }

    fn get_knn_vector_field(name: &str, vector: &[f32]) -> Result<Box<dyn IndexableField>> {
        KnnFloatCase::get_knn_vector_field(name, vector)
    }
}

/// Port of `TestSeededKnnFloatVectorQuery.testSeedWithTimeout`.
fn test_seed_with_timeout() -> Result<()> {
    let num_docs = LuceneTestCase::at_least(50);
    let dimension = LuceneTestCase::at_least(5);
    let num_iters = LuceneTestCase::at_least(5);
    let d = SeededKnnFloatCase::new_directory_for_test()?;
    let result = (|| -> Result<()> {
        index_tagged_docs::<SeededKnnFloatCase>(&d, num_docs, dimension)?;

        let reader = directory_reader::open(d.as_directory())?;
        let r = (|| -> Result<()> {
            let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
            searcher.set_timeout(constant_timeout(true));
            let k = random().next_int_bound(80) + 1;
            for _i in 0..num_iters {
                // All documents as seeds
                let seed: Arc<dyn Query> = if random().next_boolean() {
                    IntPoint::new_range_query("tag", &[1], &[6])?
                } else {
                    Arc::new(MatchAllDocsQuery::instance())
                };
                let filter = random_filter();
                let knn_float_vector_query: Arc<dyn AbstractKnnVectorQueryImpl> =
                    Arc::new(KnnFloatVectorQuery::with_filter(
                        "field",
                        SeededKnnFloatCase::random_vector(dimension),
                        k,
                        filter,
                    )?);
                let knn_query: Arc<dyn Query> = Arc::new(SeededKnnVectorQuery::from_float_query(
                    Arc::clone(&knn_float_vector_query),
                    seed,
                )?);
                assert_equals(&0, &searcher.count(knn_query)?);
                // No seed documents -- falls back on full approx search
                let seed = match_none();
                let knn_query: Arc<dyn Query> = Arc::new(SeededKnnVectorQuery::from_float_query(
                    Arc::clone(&knn_float_vector_query),
                    seed,
                )?);
                assert_equals(&0, &searcher.count(knn_query)?);
            }
            Ok(())
        })();
        reader.close()?;
        r
    })();
    d.close()?;
    result
}

/// Tests with random vectors and a random seed. Uses RandomIndexWriter.
///
/// Port of `TestSeededKnnFloatVectorQuery.testRandomWithSeed`.
fn test_random_with_seed() -> Result<()> {
    let num_docs = 1000;
    let dimension = LuceneTestCase::at_least(5);
    let num_iters = LuceneTestCase::at_least(10);
    let d = SeededKnnFloatCase::new_directory_for_test()?;
    let result = (|| -> Result<()> {
        let num_docs_with_vector =
            index_random_with_seed::<SeededKnnFloatCase>(&d, num_docs, dimension)?;

        let reader = directory_reader::open(d.as_directory())?;
        let r = (|| -> Result<()> {
            let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
            for _i in 0..num_iters {
                random_timeout(&mut searcher);
                let k = random().next_int_bound(10) + 1;
                let n = random().next_int_bound(100) + 1;
                // we may get fewer results than requested if there are deletions, but this test doesn't
                // check that
                debug_assert!(!reader.has_deletions());

                // All documents as seeds
                let seed_calls = Arc::new(AtomicI32::new(0));
                let seed1: Arc<dyn Query> = Arc::new(MatchAllDocsQuery::instance());
                let filter = random_filter();
                let knn_float_vector_query: Arc<dyn AbstractKnnVectorQueryImpl> =
                    Arc::new(KnnFloatVectorQuery::with_filter(
                        "field",
                        SeededKnnFloatCase::random_vector(dimension),
                        k,
                        filter,
                    )?);
                let query = AssertingSeededKnnVectorQuery::new(
                    Arc::clone(&knn_float_vector_query),
                    seed1,
                    None,
                    Some(Arc::clone(&seed_calls)),
                )?;
                let results = searcher.search(Arc::new(query), n)?;
                assert_equals(&seed_calls.load(Ordering::SeqCst), &1);
                let expected = n.min(k).min(num_docs_with_vector);

                assert_equals(&(expected as usize), &results.score_docs.len());
                Assert::assert_true(results.total_hits.value() >= results.score_docs.len() as i64);
                // verify the results are in descending score order
                assert_descending(&results);

                // Restrictive seed query -- 6 documents
                let seed2 = IntPoint::new_range_query("tag", &[1], &[6])?;
                let seed_count = seed_count(&searcher, &seed2)?;
                let query = AssertingSeededKnnVectorQuery::new(
                    Arc::clone(&knn_float_vector_query),
                    seed2,
                    None,
                    if seed_count > 0 {
                        Some(Arc::clone(&seed_calls))
                    } else {
                        None
                    },
                )?;
                let results = searcher.search(Arc::new(query), n)?;
                assert_equals(
                    &seed_calls.load(Ordering::SeqCst),
                    &(if seed_count > 0 { 2 } else { 1 }),
                );
                let expected = n.min(k).min(reader.num_docs());
                assert_equals(&(expected as usize), &results.score_docs.len());
                Assert::assert_true(results.total_hits.value() >= results.score_docs.len() as i64);
                // verify the results are in descending score order
                assert_descending(&results);

                // No seed documents -- falls back on full approx search
                let seed3 = match_none();
                let query = AssertingSeededKnnVectorQuery::new(
                    Arc::clone(&knn_float_vector_query),
                    seed3,
                    None,
                    None,
                )?;
                let results = searcher.search(Arc::new(query), n)?;
                let expected = n.min(k).min(reader.num_docs());
                assert_equals(&(expected as usize), &results.score_docs.len());
                Assert::assert_true(results.total_hits.value() >= results.score_docs.len() as i64);
                // verify the results are in descending score order
                assert_descending(&results);
            }
            Ok(())
        })();
        reader.close()?;
        r
    })();
    d.close()?;
    result
}
