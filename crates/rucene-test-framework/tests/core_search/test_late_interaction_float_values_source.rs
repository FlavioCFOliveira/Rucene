//! Port of `lucene/core/src/test/org/apache/lucene/search/TestLateInteractionFloatValuesSource.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::shape_field::LateInteractionField;
use rucene::document::{Document, IntField, Store};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::{IndexReader, VectorSimilarityFunction};
use rucene::search::doc_id_set_iterator::NO_MORE_DOCS;
use rucene::search::double_values_source::DoubleValuesSource;
use rucene::search::late_interaction_float_values_source::{
    LateInteractionFloatValuesSource, LateInteractionScoreFunction,
};
use rucene::search::multi_vector_similarity::MultiVectorSimilarity;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::test_float_vector_similarity_query::random_vector;
use crate::core_support::{assert_equals_delta, class, random, test, IAE};

const DIMENSION: i32 = 16;
const LATE_I_FIELD: &str = "lateIF";

/// `VectorSimilarityFunction.values()`.
pub const VECTOR_SIMILARITY_FUNCTIONS: [VectorSimilarityFunction; 4] = [
    VectorSimilarityFunction::EUCLIDEAN,
    VectorSimilarityFunction::DOT_PRODUCT,
    VectorSimilarityFunction::COSINE,
    VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT,
];

/// `LateInteractionFloatValuesSource.ScoreFunction.values()`.
const SCORE_FUNCTIONS: [LateInteractionScoreFunction; 1] =
    [LateInteractionScoreFunction::SumMaxSim];

/// `TestLateInteractionFloatValuesSource`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestLateInteractionFloatValuesSource",
        vec![
            test("testValidations", test_validations),
            test("testValues", test_values),
            test("testSumMaxSim", test_sum_max_sim),
        ],
    )
}

fn test_validations() -> Result<()> {
    expect_throws(IAE, || {
        LateInteractionFloatValuesSource::with_similarity("fieldName", None, None)
    });
    expect_throws(IAE, || {
        LateInteractionFloatValuesSource::with_similarity("fieldName", Some(Vec::new()), None)
    });
    let empty_tokens: Vec<Option<Vec<f32>>> = vec![Some(Vec::new())];
    expect_throws(IAE, || {
        LateInteractionFloatValuesSource::with_similarity("fieldName", Some(empty_tokens), None)
    });

    let n = random().next_int_range(3, 12);
    let mut value_bad: Vec<Vec<f32>> = Vec::with_capacity(n as usize);
    for _i in 0..n {
        if random().next_boolean() {
            value_bad.push(random_vector(DIMENSION));
        } else {
            value_bad.push(random_vector(DIMENSION + 1));
        }
    }
    Ok(())
}

fn test_values() -> Result<()> {
    let mut corpus: Vec<Vec<Vec<f32>>> = Vec::new();
    let num_docs = LuceneTestCase::at_least(1000);
    let num_segments = random().next_int_range(2, 5);
    let vector_similarity_function = VECTOR_SIMILARITY_FUNCTIONS
        [random().next_int_bound(VECTOR_SIMILARITY_FUNCTIONS.len() as i32) as usize];
    let score_function =
        SCORE_FUNCTIONS[random().next_int_bound(SCORE_FUNCTIONS.len() as i32) as usize];

    let mut dir = LuceneTestCase::new_directory();
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
                            let value = create_multi_vector();
                            doc.add(Box::new(IntField::new("id", id, Store::YES)));
                            id += 1;
                            doc.add(Box::new(LateInteractionField::new(
                                LATE_I_FIELD,
                                Some(&rows(&value)),
                            )?));
                            corpus.push(value);
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
            written?;
        }

        let query_vector = create_multi_vector();
        let source = Arc::new(LateInteractionFloatValuesSource::with_score_function(
            LATE_I_FIELD,
            Some(rows(&query_vector)),
            Some(vector_similarity_function),
            Arc::new(score_function),
        )?);
        let reader = directory_reader::open(dir.as_directory())?;
        let r = (|| -> Result<()> {
            for ctx in (Arc::clone(&reader) as Arc<dyn IndexReader>).leaves()? {
                let mut values = Arc::clone(&source).get_values(&ctx, None)?;
                let infos = ctx.leaf_reader().get_field_infos();
                let fi = infos.field_info(LATE_I_FIELD);
                if fi.is_none() {
                    // assertEquals(values, DoubleValues.EMPTY)
                    Assert::assert_true(values.is_empty_constant());
                    continue;
                }
                let mut disi = ctx
                    .leaf_reader()
                    .get_binary_doc_values(LATE_I_FIELD)?
                    .expect("binary doc values");
                let stored_fields = ctx.leaf_reader().stored_fields()?;
                while disi.next_doc()? != NO_MORE_DOCS {
                    let doc = disi.doc_id();
                    let id_value: usize = stored_fields
                        .document(doc)?
                        .get("id")
                        .expect("id")
                        .parse()
                        .expect("Integer.parseInt");
                    let doc_vector = &corpus[id_value];
                    let expected = score_function.compare(
                        &query_vector,
                        doc_vector,
                        vector_similarity_function,
                    )?;
                    values.advance_exact(doc)?;
                    assert_equals_delta(f64::from(expected), values.double_value()?, 0.0001);
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

fn test_sum_max_sim() -> Result<()> {
    let query_vector = create_multi_vector();
    // docVector has all the token vectors in the query and some random token vectors
    let mut doc_vector: Vec<Vec<f32>> = Vec::with_capacity(query_vector.len() + 4);
    for q in &query_vector {
        doc_vector.push(q.clone());
    }
    for _i in query_vector.len()..query_vector.len() + 4 {
        doc_vector.push(random_vector(query_vector[0].len() as i32));
    }
    let score = LateInteractionScoreFunction::SumMaxSim.compare(
        &query_vector,
        &doc_vector,
        VectorSimilarityFunction::COSINE,
    )?;
    assert_equals_delta(query_vector.len() as f64, f64::from(score), 1e-5);
    Ok(())
}

/// `float[][]` of non-null rows.
pub fn rows(v: &[Vec<f32>]) -> Vec<Option<Vec<f32>>> {
    v.iter().cloned().map(Some).collect()
}

fn create_multi_vector() -> Vec<Vec<f32>> {
    let n = random().next_int_range(3, 5);
    let mut value = Vec::with_capacity(n as usize);
    for _i in 0..n {
        value.push(random_vector(DIMENSION));
    }
    value
}
