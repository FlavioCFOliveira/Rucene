//! Port of `lucene/core/src/test/org/apache/lucene/search/TestFullPrecisionFloatVectorSimilarityValuesSource.java`.
//!
//! `fpSimValues == DoubleValues.EMPTY` is Java's identity test with the
//! constant; it is `DoubleValues::is_empty_constant` here (declared at
//! `src/search/double_values.rs`).

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{Assert, Before, Test};
use rucene::codecs::lucene104::scalar_quantized_vectors::Lucene104HnswScalarQuantizedVectorsFormat;
use rucene::codecs::lucene99::hnsw_vectors_format::{
    DEFAULT_BEAM_WIDTH, DEFAULT_MAX_CONN, HNSW_GRAPH_THRESHOLD,
};
use rucene::codecs::{self, Codec, KnnVectorsFormat};
use rucene::document::{Document, IntField, KnnFloatVectorField, Store};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::{IndexReader, VectorSimilarityFunction};
use rucene::search::double_values::DoubleValues;
use rucene::search::double_values_source::similarity_to_float_query_vector;
use rucene::search::full_precision_float_vector_similarity_values_source::FullPrecisionFloatVectorSimilarityValuesSource;
use rucene::search::NO_MORE_DOCS;
use rucene::util::quantization::quantized_byte_vector_values::ScalarEncoding;
use rucene_test_framework::util::lucene_test_case::Nightly;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::{fixture_class, ftest};
use crate::core_search::test_float_vector_similarity_query::random_vector;
use crate::core_support::{assert_equals_delta, assert_null, random};

const KNN_FIELD: &str = "knnField";
const NUM_VECTORS: i32 = 1000;
const VECTOR_DIMENSION: i32 = 128;

/// `VectorSimilarityFunction.values()`.
const SIMILARITY_FUNCTIONS: [VectorSimilarityFunction; 4] = [
    VectorSimilarityFunction::EUCLIDEAN,
    VectorSimilarityFunction::DOT_PRODUCT,
    VectorSimilarityFunction::COSINE,
    VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT,
];

/// The instance of `TestFullPrecisionFloatVectorSimilarityValuesSource`.
#[derive(Default)]
pub struct FullPrecision {
    saved_codec: Option<Arc<dyn Codec>>,
    format: Option<Arc<dyn KnnVectorsFormat>>,
    bits: i32,
}

/// `TestFullPrecisionFloatVectorSimilarityValuesSource`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<FullPrecision>(
        "org.apache.lucene.search.TestFullPrecisionFloatVectorSimilarityValuesSource",
        vec![
            crate::core_search::support::set_up(full_precision_set_up).annotate(Before),
            crate::core_search::support::tear_down(|f: &mut FullPrecision| {
                if let Some(saved) = f.saved_codec.take() {
                    codecs::set_default(saved); // restore
                }
                Ok(())
            }),
            // TODO: incredibly slow
            ftest(
                "testFullPrecisionVectorSimilarityDVS",
                test_full_precision_vector_similarity_dvs,
            )
            .annotate(Nightly)
            .annotate(Test::new()),
        ],
    )
}

fn full_precision_set_up(f: &mut FullPrecision) -> Result<()> {
    f.bits = if random().next_boolean() { 4 } else { 8 };
    f.format = Some(get_knn_format(f.bits)?);
    f.saved_codec = codecs::get_default();
    codecs::set_default(get_codec(f));
    Ok(())
}

/// `protected Codec getCodec()`.
fn get_codec(f: &FullPrecision) -> Arc<dyn Codec> {
    TestUtil::always_knn_vectors_format(Arc::clone(f.format.as_ref().expect("setUp ran")))
}

fn get_knn_format(bits: i32) -> Result<Arc<dyn KnnVectorsFormat>> {
    Ok(Arc::new(
        Lucene104HnswScalarQuantizedVectorsFormat::with_merge_exec(
            ScalarEncoding::from_num_bits(bits)?,
            DEFAULT_MAX_CONN,
            DEFAULT_BEAM_WIDTH,
            1,
            None,
            HNSW_GRAPH_THRESHOLD,
        )?,
    ))
}

/// Indexes `numSegments` segments of `numVectors` documents (some without a
/// vector) and one segment with no vectors, with `bits`-bit quantization.
fn index_segments(
    dir: &rucene_test_framework::util::lucene_test_case::TestDirectory,
    bits: i32,
    num_segments: i32,
    num_vectors: i32,
    indexing_similarity_function: VectorSimilarityFunction,
    id: &mut i32,
    vectors: &mut Vec<Vec<f32>>,
) -> Result<()> {
    let config = LuceneTestCase::new_index_writer_config();
    config.set_codec(TestUtil::always_knn_vectors_format(get_knn_format(bits)?))?;
    let w = IndexWriter::new(dir.as_directory(), config)?;
    for _ in 0..num_segments {
        for _ in 0..num_vectors {
            let mut doc = Document::new();
            if random().next_int_bound(100) < 30 {
                // skip vector for some docs to create sparse vector field
                doc.add(Box::new(IntField::new("has_vector", 0, Store::YES)));
            } else {
                let vector = random_vector(VECTOR_DIMENSION);
                doc.add(Box::new(IntField::new("id", *id, Store::YES)));
                *id += 1;
                doc.add(Box::new(KnnFloatVectorField::new(
                    KNN_FIELD,
                    &vector,
                    indexing_similarity_function,
                )?));
                vectors.push(vector);
                doc.add(Box::new(IntField::new("has_vector", 1, Store::YES)));
            }
            w.add_document(&doc)?;
            w.flush()?;
        }
    }
    // add a segment with no vectors
    for _ in 0..100 {
        let mut doc = Document::new();
        doc.add(Box::new(IntField::new("has_vector", 0, Store::YES)));
        w.add_document(&doc)?;
    }
    w.flush()?;
    w.close()?;
    Ok(())
}

fn test_full_precision_vector_similarity_dvs(_f: &mut FullPrecision) -> Result<()> {
    let mut vectors: Vec<Vec<f32>> = Vec::new();
    let num_vectors = LuceneTestCase::at_least(NUM_VECTORS);
    let num_segments = random().next_int_range(2, 10);
    let indexing_similarity_function =
        SIMILARITY_FUNCTIONS[random().next_int_bound(SIMILARITY_FUNCTIONS.len() as i32) as usize];

    let dir = LuceneTestCase::new_directory();
    {
        let mut id = 0;

        // index some 4 bit quantized vectors
        index_segments(
            &dir,
            4,
            num_segments,
            num_vectors,
            indexing_similarity_function,
            &mut id,
            &mut vectors,
        )?;

        // index some 8 bit quantized vectors
        index_segments(
            &dir,
            8,
            num_segments,
            num_vectors,
            indexing_similarity_function,
            &mut id,
            &mut vectors,
        )?;

        let query_vector = random_vector(VECTOR_DIMENSION);
        let mut rerank_similarity_function;
        {
            let reader = directory_reader::open(dir.as_directory())?;
            for ctx in (Arc::clone(&reader) as Arc<dyn IndexReader>).leaves()? {
                let mut fp_sim_values: Box<dyn DoubleValues>;
                if random().next_boolean() {
                    rerank_similarity_function = SIMILARITY_FUNCTIONS
                        [random().next_int_bound(SIMILARITY_FUNCTIONS.len() as i32) as usize];
                    fp_sim_values = Arc::new(FullPrecisionFloatVectorSimilarityValuesSource::new(
                        query_vector.clone(),
                        KNN_FIELD,
                        rerank_similarity_function,
                    ))
                    .get_similarity_scores(&ctx)?;
                } else {
                    fp_sim_values = Arc::new(
                        FullPrecisionFloatVectorSimilarityValuesSource::with_field_similarity(
                            query_vector.clone(),
                            KNN_FIELD,
                        ),
                    )
                    .get_similarity_scores(&ctx)?;
                    rerank_similarity_function = indexing_similarity_function;
                }
                let mut quantized_sim_values =
                    similarity_to_float_query_vector(&ctx, query_vector.clone(), KNN_FIELD)?;
                // validate when segment has no vectors
                if fp_sim_values.is_empty_constant() || quantized_sim_values.is_empty_constant() {
                    // `assertEquals(fpSimValues, quantizedSimValues)`: identity.
                    Assert::assert_true(
                        fp_sim_values.is_empty_constant()
                            && quantized_sim_values.is_empty_constant(),
                    );
                    assert_null(
                        &ctx.leaf_reader()
                            .get_float_vector_values(KNN_FIELD)?
                            .map(|_| ()),
                    );
                    continue;
                }
                let stored_fields = ctx.leaf_reader().stored_fields()?;
                let mut quantized_scorer = ctx
                    .leaf_reader()
                    .get_float_vector_values(KNN_FIELD)?
                    .expect("NullPointerException")
                    .scorer(&query_vector)?
                    .expect("NullPointerException");
                while quantized_scorer.iterator().next_doc()? != NO_MORE_DOCS {
                    let doc = quantized_scorer.iterator().doc_id();
                    fp_sim_values.advance_exact(doc)?;
                    quantized_sim_values.advance_exact(doc)?;
                    let id_value: i32 = stored_fields
                        .document(doc)?
                        .get("id")
                        .expect("NullPointerException")
                        .parse()
                        .unwrap_or_else(|_| {
                            randomizedtesting::jdk::exception::throw_without_message(
                                "java.lang.NumberFormatException",
                            )
                        });
                    let doc_vector = &vectors[id_value as usize];
                    // validate full precision vector scores
                    let expected_fp_score = f64::from(
                        rerank_similarity_function.compare_f32(&query_vector, doc_vector)?,
                    );
                    let actual_fp_score = fp_sim_values.double_value()?;
                    assert_equals_delta(expected_fp_score, actual_fp_score, 1e-5);
                    // validate quantized vector scores
                    let expected_q_score = f64::from(quantized_scorer.score()?);
                    let actual_q_score = quantized_sim_values.double_value()?;
                    assert_equals_delta(expected_q_score, actual_q_score, 1e-5);
                }
            }
            reader.close()?;
        }
    }
    dir.close()?;
    Ok(())
}
