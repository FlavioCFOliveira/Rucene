//! Port of `lucene/core/src/test/org/apache/lucene/search/TestRescoreTopNQuery.java`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{Assert, Before};
use rucene::codecs::lucene104::scalar_quantized_vectors::Lucene104HnswScalarQuantizedVectorsFormat;
use rucene::document::shape_field::LateInteractionField;
use rucene::document::{Document, IntField, KnnFloatVectorField, NumericValue, Store};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::{IndexReader, Term, VectorSimilarityFunction};
use rucene::search::double_values_source;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::late_interaction_float_values_source::LateInteractionScoreFunction;
use rucene::search::multi_vector_similarity::MultiVectorSimilarity;
use rucene::search::rescore_top_n_query::RescoreTopNQuery;
use rucene::search::term_query::TermQuery;
use rucene::search::{KnnFloatVectorQuery, Query};
use rucene::store::{ByteBuffersDirectory, Directory};
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::Nightly;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::{fixture_class, ftest, set_up};
use crate::core_search::test_late_interaction_float_values_source::{
    rows, VECTOR_SIMILARITY_FUNCTIONS,
};
use crate::core_support::{assert_equals, assert_equals_delta, random, IAE};

const FIELD: &str = "vector";
const VECTOR_SIMILARITY_FUNCTION: VectorSimilarityFunction = VectorSimilarityFunction::COSINE;
const NUM_VECTORS: i32 = 1000;
const VECTOR_DIMENSION: i32 = 128;

/// `LateInteractionFloatValuesSource.ScoreFunction.values()`.
const SCORE_FUNCTIONS: [LateInteractionScoreFunction; 1] =
    [LateInteractionScoreFunction::SumMaxSim];

/// The instance of `TestRescoreTopNQuery`.
#[derive(Default)]
pub struct RescoreTopN {
    directory: Option<Arc<dyn Directory>>,
    config: Option<IndexWriterConfig>,
}

/// `TestRescoreTopNQuery`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<RescoreTopN>(
        "org.apache.lucene.search.TestRescoreTopNQuery",
        vec![
            set_up(rescore_set_up).annotate(Before),
            ftest("testInvalidN", test_invalid_n),
            // TODO: incredibly slow
            ftest("testRescoreField", test_rescore_field).annotate(Nightly),
            ftest("testMissingDoubleValues", test_missing_double_values),
            ftest("testLateInteractionQuery", test_late_interaction_query),
        ],
    )
}

fn rescore_set_up(f: &mut RescoreTopN) -> Result<()> {
    f.directory = Some(Arc::new(ByteBuffersDirectory::default()));

    // Set up the IndexWriterConfig to use quantized vector storage
    let config = IndexWriterConfig::new();
    config.set_codec(TestUtil::always_knn_vectors_format(Arc::new(
        Lucene104HnswScalarQuantizedVectorsFormat::new(),
    )))?;
    f.config = Some(config);
    Ok(())
}

impl RescoreTopN {
    fn directory(&self) -> Arc<dyn Directory> {
        Arc::clone(self.directory.as_ref().expect("setUp ran"))
    }

    fn take_config(&mut self) -> IndexWriterConfig {
        self.config.take().expect("setUp ran")
    }
}

/// Port of `TestRescoreTopNQuery.testInvalidN`.
fn test_invalid_n(_f: &mut RescoreTopN) -> Result<()> {
    expect_throws(IAE, || {
        RescoreTopNQuery::new(
            Arc::new(TermQuery::new(Term::from_text("test", ""))),
            double_values_source::constant(0.0),
            0,
        )
    });
    Ok(())
}

/// Port of `TestRescoreTopNQuery.testRescoreField`.
fn test_rescore_field(f: &mut RescoreTopN) -> Result<()> {
    let mut vectors: HashMap<i32, Vec<f32>> = HashMap::new();

    let random = random();

    let num_vectors = LuceneTestCase::at_least(NUM_VECTORS);
    let num_segments = random.next_int_range(2, 10);

    // Step 1: Index random vectors in quantized format
    {
        let writer = IndexWriter::new(f.directory(), f.take_config())?;
        let written = (|| -> Result<()> {
            for j in 0..num_segments {
                for i in 0..num_vectors {
                    let vector = random_float_vector(VECTOR_DIMENSION, &*random);
                    let mut doc = Document::new();
                    let id = j * num_vectors + i;
                    doc.add(Box::new(IntField::new("id", id, Store::YES)));
                    doc.add(Box::new(KnnFloatVectorField::new(
                        FIELD,
                        &vector,
                        VECTOR_SIMILARITY_FUNCTION,
                    )?));
                    writer.add_document(&doc)?;
                    vectors.insert(id, vector);
                }
                writer.flush()?;
            }
            Ok(())
        })();
        let closed = writer.close();
        written.and(closed)?;
    }

    // Step 2: Run TwoPhaseKnnVectorQuery with a random target vector
    let reader = directory_reader::open(f.directory())?;
    let result = (|| -> Result<()> {
        let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
        let target_vector = random_float_vector(VECTOR_DIMENSION, &*random);
        let k = 10;
        let oversample = f64::from(random.next_float_range(1.5, 3.0));

        let knn_query: Arc<dyn Query> = Arc::new(KnnFloatVectorQuery::new(
            FIELD,
            target_vector.clone(),
            k + (k as f64 * oversample) as i32,
        )?);

        let query = RescoreTopNQuery::create_full_precision_rescorer_query(
            knn_query,
            target_vector.clone(),
            FIELD,
            k,
        )?;
        let top_docs = searcher.search(query, k)?;

        // Step 3: Verify that TopDocs scores match similarity with unquantized vectors
        let stored_fields = searcher.stored_fields()?;
        for score_doc in &top_docs.score_docs {
            let retrieved_doc = stored_fields.document(score_doc.doc)?;
            let id = match retrieved_doc
                .get_field("id")
                .and_then(|field| field.numeric_value())
            {
                Some(value) => int_value(value),
                None => randomizedtesting::jdk::exception::throw_without_message(
                    "java.lang.NullPointerException",
                ),
            };
            let doc_vector = vectors.get(&id);
            debug_assert!(doc_vector.is_some(), "Vector for id {id} not found");
            let doc_vector = doc_vector.unwrap_or_else(|| {
                randomizedtesting::jdk::exception::throw_without_message(
                    "java.lang.NullPointerException",
                )
            });
            let expected_score =
                VECTOR_SIMILARITY_FUNCTION.compare_f32(&target_vector, doc_vector)?;
            Assert::assert_equals_double_msg(
                Some(&format!(
                    "Score does not match expected similarity for doc ord: {}, id: {id}",
                    score_doc.doc
                )),
                f64::from(expected_score),
                f64::from(score_doc.score),
                1e-5,
            );
        }
        Ok(())
    })();
    let closed = reader.close();
    result.and(closed)
}

/// Port of `TestRescoreTopNQuery.testMissingDoubleValues`.
fn test_missing_double_values(f: &mut RescoreTopN) -> Result<()> {
    let random = random();

    {
        let writer = IndexWriter::new(f.directory(), f.take_config())?;
        let written = (|| -> Result<()> {
            let vector = random_float_vector(VECTOR_DIMENSION, &*random);
            let mut doc = Document::new();
            doc.add(Box::new(KnnFloatVectorField::new(
                FIELD,
                &vector,
                VECTOR_SIMILARITY_FUNCTION,
            )?));
            writer.add_document(&doc)?;
            Ok(())
        })();
        let closed = writer.close();
        written.and(closed)?;
    }

    // Step 2: Run TwoPhaseKnnVectorQuery with a random target vector
    let reader = directory_reader::open(f.directory())?;
    let result = (|| -> Result<()> {
        let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
        let target_vector = random_float_vector(VECTOR_DIMENSION, &*random);
        let k = 1;

        let knn_query: Arc<dyn Query> =
            Arc::new(KnnFloatVectorQuery::new(FIELD, target_vector.clone(), k)?);

        let query = RescoreTopNQuery::create_full_precision_rescorer_query(
            knn_query,
            target_vector,
            "field-1",
            k,
        )?;
        let top_docs = searcher.search(query, k)?;

        // Step 3: The rescoring field is invalid, so the score should be 0
        for score_doc in &top_docs.score_docs {
            Assert::assert_equals_double_msg(
                Some("Score must be 0 for missing DoubleValues"),
                0.0,
                f64::from(score_doc.score),
                1e-5,
            );
        }
        Ok(())
    })();
    let closed = reader.close();
    result.and(closed)
}

/// Port of `TestRescoreTopNQuery.testLateInteractionQuery`.
fn test_late_interaction_query(_f: &mut RescoreTopN) -> Result<()> {
    const LATE_I_FIELD: &str = "li_vector";
    const KNN_FIELD: &str = "knn_vector";
    let mut corpus: Vec<Vec<Vec<f32>>> = Vec::new();
    let num_docs = LuceneTestCase::at_least(1000);
    let num_segments = random().next_int_range(2, 10);
    let dim = 128;
    let vector_similarity_function = VECTOR_SIMILARITY_FUNCTIONS
        [random().next_int_bound(VECTOR_SIMILARITY_FUNCTIONS.len() as i32) as usize];
    let score_function =
        SCORE_FUNCTIONS[random().next_int_bound(SCORE_FUNCTIONS.len() as i32) as usize];

    let dir = LuceneTestCase::new_directory();
    let result = (|| -> Result<()> {
        let mut id = 0;
        {
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
                            let value = create_multi_vector(dim);
                            doc.add(Box::new(IntField::new("id", id, Store::YES)));
                            id += 1;
                            doc.add(Box::new(LateInteractionField::new(
                                LATE_I_FIELD,
                                Some(&rows(&value)),
                            )?));
                            corpus.push(value);
                            doc.add(Box::new(KnnFloatVectorField::with_euclidean(
                                KNN_FIELD,
                                &random_float_vector(dim, &*random()),
                            )?));
                            doc.add(Box::new(IntField::new("has_li_vector", 1, Store::YES)));
                        }
                        w.add_document(&doc)?;
                    }
                    w.flush()?;
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
            let closed = w.close();
            written.and(closed)?;
        }

        let late_i_query_vector = create_multi_vector(dim);
        let knn_query_vector = random_float_vector(dim, &*random());
        let knn_query: Arc<dyn Query> =
            Arc::new(KnnFloatVectorQuery::new(KNN_FIELD, knn_query_vector, 50)?);

        let reader = directory_reader::open(dir.as_directory())?;
        let r = (|| -> Result<()> {
            let top_n = 10;
            let s = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
            let knn_hits = s.search(Arc::clone(&knn_query), 5 * top_n)?;
            let knn_hit_docs: HashSet<i32> = knn_hits.score_docs.iter().map(|k| k.doc).collect();
            let late_i_query = RescoreTopNQuery::create_late_interaction_query(
                Arc::clone(&knn_query),
                top_n,
                LATE_I_FIELD,
                late_i_query_vector.clone(),
                vector_similarity_function,
            )?;
            let late_i_hits = s.search(late_i_query, 3 * top_n)?;
            // total match-set for RescoreTopNQuery is topN
            assert_equals(&(top_n as usize), &late_i_hits.score_docs.len());
            let stored_fields = reader.stored_fields()?;
            for hit in &late_i_hits.score_docs {
                Assert::assert_true(knn_hit_docs.contains(&hit.doc));
                let id_value: usize = stored_fields
                    .document(hit.doc)?
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
                assert_equals_delta(f64::from(expected), f64::from(hit.score), 1e-5);
            }
            Ok(())
        })();
        reader.close()?;
        r
    })();
    dir.close()?;
    result
}

/// `Number.intValue()`.
fn int_value(value: NumericValue) -> i32 {
    match value {
        NumericValue::Int(v) => v,
        NumericValue::Long(v) => v as i32,
        NumericValue::Float(v) => v as i32,
        NumericValue::Double(v) => v as i32,
    }
}

/// `private float[][] createMultiVector(int dimension)`.
fn create_multi_vector(dimension: i32) -> Vec<Vec<f32>> {
    let n = random().next_int_range(3, 12);
    let mut value = Vec::with_capacity(n as usize);
    for _i in 0..n {
        value.push(random_float_vector(dimension, &*random()));
    }
    value
}

/// `private float[] randomFloatVector(int dimension, Random random)`.
fn random_float_vector(dimension: i32, random: &dyn Random) -> Vec<f32> {
    let mut vector = vec![0f32; dimension as usize];
    for i in 0..dimension as usize {
        vector[i] = random.next_float();
    }
    vector
}
