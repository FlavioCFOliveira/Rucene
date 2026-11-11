//! Port of `lucene/core/src/test/org/apache/lucene/search/TestLateInteractionRescorer.java`.

use std::collections::HashSet;
use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::shape_field::LateInteractionField;
use rucene::document::{Document, IntField, KnnFloatVectorField, Store};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::IndexReader;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::late_interaction_float_values_source::LateInteractionScoreFunction;
use rucene::search::multi_vector_similarity::MultiVectorSimilarity;
use rucene::search::rescorer::Rescorer;
use rucene::search::{KnnFloatVectorQuery, LateInteractionRescorer, Query};
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::test_late_interaction_float_values_source::{
    rows, VECTOR_SIMILARITY_FUNCTIONS,
};
use crate::core_support::{assert_equals, assert_equals_delta, class, random, test};

const LATE_I_FIELD: &str = "li_vector";
const KNN_FIELD: &str = "knn_vector";
const DIMENSION: i32 = 16;

/// `TestLateInteractionRescorer`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestLateInteractionRescorer",
        vec![
            test("testBasic", test_basic),
            test("testMissingLateIValues", test_missing_late_i_values),
        ],
    )
}

fn test_basic() -> Result<()> {
    let mut corpus: Vec<Vec<Vec<f32>>> = Vec::new();
    let vector_similarity_function = VECTOR_SIMILARITY_FUNCTIONS
        [random().next_int_bound(VECTOR_SIMILARITY_FUNCTIONS.len() as i32) as usize];
    let score_function = LateInteractionScoreFunction::SumMaxSim;

    let dir = LuceneTestCase::new_directory();
    let result = (|| -> Result<()> {
        index_multi_vectors(&dir, &mut corpus)?;
        let late_i_query_vector = create_multi_vector(DIMENSION);
        let knn_query_vector = random_float_vector(DIMENSION, &*random());
        let knn_query: Arc<dyn Query> =
            Arc::new(KnnFloatVectorQuery::new(KNN_FIELD, knn_query_vector, 50)?);

        let reader = directory_reader::open(dir.as_directory())?;
        let r = (|| -> Result<()> {
            let top_n = 10;
            let s = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
            let knn_hits = s.search(Arc::clone(&knn_query), 5 * top_n)?;
            let rescorer = LateInteractionRescorer::create_with_similarity(
                LATE_I_FIELD,
                late_i_query_vector.clone(),
                vector_similarity_function,
            )?;
            let reranked_hits = rescorer.rescore(&s, &knn_hits, top_n)?;
            let knn_hit_docs: HashSet<i32> = knn_hits.score_docs.iter().map(|k| k.doc).collect();
            assert_equals(&(top_n as usize), &reranked_hits.score_docs.len());
            let stored_fields = reader.stored_fields()?;
            for i in 0..reranked_hits.score_docs.len() {
                Assert::assert_true(knn_hit_docs.contains(&reranked_hits.score_docs[i].doc));
                let id_value: usize = stored_fields
                    .document(reranked_hits.score_docs[i].doc)?
                    .get("id")
                    .expect("id")
                    .parse()
                    .expect("Integer.parseInt");
                let doc_vector = &corpus[id_value];
                let expected = score_function.compare(
                    &late_i_query_vector,
                    doc_vector,
                    vector_similarity_function,
                )?;
                assert_equals_delta(
                    f64::from(expected),
                    f64::from(reranked_hits.score_docs[i].score),
                    f64::from(1e-4f32),
                );
                if i > 0 {
                    Assert::assert_true(
                        reranked_hits.score_docs[i].score <= reranked_hits.score_docs[i - 1].score,
                    );
                }
            }
            Ok(())
        })();
        reader.close()?;
        r
    })();
    dir.close()?;
    result
}

fn test_missing_late_i_values() -> Result<()> {
    let mut corpus: Vec<Vec<Vec<f32>>> = Vec::new();
    let vector_similarity_function = VECTOR_SIMILARITY_FUNCTIONS
        [random().next_int_bound(VECTOR_SIMILARITY_FUNCTIONS.len() as i32) as usize];

    let dir = LuceneTestCase::new_directory();
    let result = (|| -> Result<()> {
        index_multi_vectors(&dir, &mut corpus)?;
        let late_i_query_vector = create_multi_vector(DIMENSION);
        let knn_query_vector = random_float_vector(DIMENSION, &*random());
        let knn_query: Arc<dyn Query> =
            Arc::new(KnnFloatVectorQuery::new(KNN_FIELD, knn_query_vector, 50)?);

        let reader = directory_reader::open(dir.as_directory())?;
        let r = (|| -> Result<()> {
            let top_n = 10;
            let s = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
            let mut knn_hits = s.search(Arc::clone(&knn_query), 5 * top_n)?;
            let rescorer = LateInteractionRescorer::create_with_similarity(
                "bad-test-field",
                late_i_query_vector.clone(),
                vector_similarity_function,
            )?;
            let mut reranked_hits = rescorer.rescore(&s, &knn_hits, top_n)?;
            let mut knn_hit_docs: HashSet<i32> =
                knn_hits.score_docs.iter().map(|k| k.doc).collect();
            assert_equals(&(top_n as usize), &reranked_hits.score_docs.len());
            for i in 0..reranked_hits.score_docs.len() {
                Assert::assert_true(knn_hit_docs.contains(&reranked_hits.score_docs[i].doc));
                assert_equals_delta(0.0, f64::from(reranked_hits.score_docs[i].score), 1e-5);
            }

            let rescorer_with_fallback =
                LateInteractionRescorer::with_fallback_to_first_pass_score(
                    "bad-test-field",
                    late_i_query_vector,
                    vector_similarity_function,
                )?;
            knn_hits = s.search(Arc::clone(&knn_query), 5 * top_n)?;
            reranked_hits = rescorer_with_fallback.rescore(&s, &knn_hits, top_n)?;
            knn_hit_docs = knn_hits.score_docs.iter().map(|k| k.doc).collect();
            assert_equals(&(top_n as usize), &reranked_hits.score_docs.len());
            for i in 0..reranked_hits.score_docs.len() {
                Assert::assert_true(knn_hit_docs.contains(&reranked_hits.score_docs[i].doc));
                assert_equals_delta(
                    f64::from(knn_hits.score_docs[i].score),
                    f64::from(reranked_hits.score_docs[i].score),
                    1e-5,
                );
            }
            Ok(())
        })();
        reader.close()?;
        r
    })();
    dir.close()?;
    result
}

fn index_multi_vectors(dir: &TestDirectory, corpus: &mut Vec<Vec<Vec<f32>>>) -> Result<()> {
    let num_docs = LuceneTestCase::at_least(1000);
    let num_segments = random().next_int_range(2, 5);
    let mut id = 0;
    let w = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )?;
    let written = (|| -> Result<()> {
        for _j in 0..num_segments {
            for _i in 0..num_docs {
                let mut doc = Document::new();
                if random().next_int_bound(100) < 30 {
                    // skip value for some docs to create sparse field
                    doc.add(Box::new(IntField::new("has_li_vector", 0, Store::YES)));
                } else {
                    let value = create_multi_vector(DIMENSION);
                    doc.add(Box::new(IntField::new("id", id, Store::YES)));
                    id += 1;
                    doc.add(Box::new(LateInteractionField::new(
                        LATE_I_FIELD,
                        Some(&rows(&value)),
                    )?));
                    corpus.push(value);
                    doc.add(Box::new(KnnFloatVectorField::with_euclidean(
                        KNN_FIELD,
                        &random_float_vector(DIMENSION, &*random()),
                    )?));
                    doc.add(Box::new(IntField::new("has_li_vector", 1, Store::YES)));
                }
                w.add_document(&doc)?;
                w.flush()?;
            }
        }
        // add a segment with no vectors
        for _i in 0..100 {
            let mut doc = Document::new();
            doc.add(Box::new(IntField::new("has_li_vector", 0, Store::YES)));
            w.add_document(&doc)?;
        }
        w.flush()?;
        Ok(())
    })();
    w.close()?;
    written
}

fn create_multi_vector(dimension: i32) -> Vec<Vec<f32>> {
    let n = random().next_int_range(2, 5);
    let mut value = Vec::with_capacity(n as usize);
    for _i in 0..n {
        value.push(random_float_vector(dimension, &*random()));
    }
    value
}

fn random_float_vector(dimension: i32, random: &dyn Random) -> Vec<f32> {
    let mut vector = vec![0f32; dimension as usize];
    for i in 0..dimension as usize {
        vector[i] = random.next_float();
    }
    vector
}
