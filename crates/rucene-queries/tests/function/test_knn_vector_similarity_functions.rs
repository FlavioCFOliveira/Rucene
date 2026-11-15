/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `TestKnnVectorSimilarityFunctions.java`. Its test methods are
//! JUnit 4 `@Test` methods without the `test` prefix.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::{AfterClass, BeforeClass, Test};
use rucene::analysis::Analyzer;
use rucene::document::{
    Document, KnnByteVectorField, KnnFloatVectorField, SortedDocValuesField, Store, StringField,
};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::VectorSimilarityFunction;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::match_all_docs_query::MatchAllDocsQuery;
use rucene::search::query::Query;
use rucene::search::ScoreDoc;
use rucene::util::BytesRef;
use rucene_queries::function::function_query::FunctionQuery;
use rucene_queries::function::function_score_query::FunctionScoreQuery;
use rucene_queries::function::value_source::ValueSource;
use rucene_queries::function::valuesource::byte_knn_vector_field_source::ByteKnnVectorFieldSource;
use rucene_queries::function::valuesource::byte_vector_similarity_function::ByteVectorSimilarityFunction;
use rucene_queries::function::valuesource::const_knn_byte_vector_value_source::ConstKnnByteVectorValueSource;
use rucene_queries::function::valuesource::const_knn_float_value_source::ConstKnnFloatValueSource;
use rucene_queries::function::valuesource::float_knn_vector_field_source::FloatKnnVectorFieldSource;
use rucene_queries::function::valuesource::float_vector_similarity_function::FloatVectorSimilarityFunction;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::check_hits::CheckHits;
use rucene_test_framework::util::java_exceptions::{expect_throws, rethrow_java};
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::function::test_index_reader_functions::id_sort;
use crate::queries_support::core_support::{class, random};
use crate::queries_support::search_support::static_hook;

/// `documents`.
const DOCUMENTS: [&str; 2] = ["1", "2"];

/// The static fields `dir`, `analyzer`, `reader` and `searcher`.
struct Statics {
    dir: TestDirectory,
    analyzer: Arc<dyn Analyzer>,
    reader: Arc<dyn DirectoryReader>,
    searcher: Arc<IndexSearcher>,
}

static STATICS: Mutex<Option<Statics>> = Mutex::new(None);

fn searcher() -> Arc<IndexSearcher> {
    let g = STATICS.lock().unwrap_or_else(|e| e.into_inner());
    Arc::clone(&g.as_ref().expect("beforeClass ran").searcher)
}

/// A `@Test public void name()` method.
fn t(name: &str, body: fn() -> Result<()>) -> Method {
    Method::new(name)
        .throws(&["java.lang.Exception"])
        .annotate(Test::new())
        .body_any(move || {
            if let Err(e) = body() {
                rethrow_java(e)
            }
        })
}

/// `TestKnnVectorSimilarityFunctions`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.queries.function.TestKnnVectorSimilarityFunctions",
        vec![
            static_hook("beforeClass", BeforeClass, before_class),
            static_hook("afterClass", AfterClass, after_class),
            t(
                "vectorSimilarity_floatConstantVectors_shouldReturnFloatSimilarity",
                float_constant_vectors,
            ),
            t(
                "vectorSimilarity_byteConstantVectors_shouldReturnFloatSimilarity",
                byte_constant_vectors,
            ),
            t(
                "vectorSimilarity_floatFieldVectors_shouldReturnFloatSimilarity",
                float_field_vectors,
            ),
            t(
                "vectorSimilarity_byteFieldVectors_shouldReturnFloatSimilarity",
                byte_field_vectors,
            ),
            t(
                "vectorSimilarity_FloatConstAndFloatFieldVectors_shouldReturnFloatSimilarity",
                float_const_and_float_field_vectors,
            ),
            t(
                "vectorSimilarity_ByteConstAndByteFieldVectors_shouldReturnFloatSimilarity",
                byte_const_and_byte_field_vectors,
            ),
            t(
                "vectorSimilarity_missingFloatVectorField_shouldReturnZero",
                missing_float_vector_field,
            ),
            t(
                "vectorSimilarity_missingByteVectorField_shouldReturnZero",
                missing_byte_vector_field,
            ),
            t(
                "vectorSimilarity_twoVectorsWithDifferentDimensions_shouldRaiseException",
                two_vectors_with_different_dimensions,
            ),
            t(
                "vectorSimilarity_byteAndFloatVectors_shouldRaiseException",
                byte_and_float_vectors,
            ),
            t(
                "vectorSimilarity_wrongFieldType_shouldRaiseException",
                wrong_field_type,
            ),
        ],
    )
}

fn before_class() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::new(&*random()));
    let iw_config = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::clone(&analyzer)));
    iw_config.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let iw = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iw_config)?;
    let mut document = Document::new();
    document.add(Box::new(StringField::new(
        "id",
        "1".to_string(),
        Store::NO,
    )?));
    document.add(Box::new(SortedDocValuesField::new(
        "id",
        BytesRef::from_char_sequence("1"),
    )));
    document.add(Box::new(KnnFloatVectorField::with_euclidean(
        "knnFloatField1",
        &[1.0, 2.0, 3.0],
    )?));
    document.add(Box::new(KnnFloatVectorField::with_euclidean(
        "knnFloatField2",
        &[5.2, 3.2, 3.1],
    )?));

    // add only to the first document
    document.add(Box::new(KnnFloatVectorField::with_euclidean(
        "knnFloatField3",
        &[1.0, 1.0, 1.0],
    )?));
    document.add(Box::new(KnnByteVectorField::with_euclidean(
        "knnByteField3",
        &[1, 1, 1],
    )?));

    document.add(Box::new(KnnByteVectorField::with_euclidean(
        "knnByteField1",
        &[1, 2, 3],
    )?));
    document.add(Box::new(KnnByteVectorField::with_euclidean(
        "knnByteField2",
        &[4, 2, 3],
    )?));
    iw.add_document(&document)?;

    if LuceneTestCase::usually_with(&*random()) {
        iw.commit()?;
    }

    let mut document2 = Document::new();
    document2.add(Box::new(StringField::new(
        "id",
        "2".to_string(),
        Store::NO,
    )?));
    document2.add(Box::new(SortedDocValuesField::new(
        "id",
        BytesRef::from_char_sequence("2"),
    )));
    document2.add(Box::new(KnnFloatVectorField::with_euclidean(
        "knnFloatField1",
        &[1.0, 2.0, 3.0],
    )?));
    document2.add(Box::new(KnnFloatVectorField::with_euclidean(
        "knnFloatField2",
        &[5.2, 3.2, 3.1],
    )?));

    document2.add(Box::new(KnnByteVectorField::with_euclidean(
        "knnByteField1",
        &[1, 2, 3],
    )?));
    document2.add(Box::new(KnnByteVectorField::with_euclidean(
        "knnByteField2",
        &[4, 2, 3],
    )?));
    iw.add_document(&document2)?;

    let reader = iw.get_reader()?;
    let searcher = Arc::new(LuceneTestCase::new_searcher(Arc::clone(&reader)));
    iw.close()?;
    *STATICS.lock().unwrap_or_else(|e| e.into_inner()) = Some(Statics {
        dir,
        analyzer,
        reader,
        searcher,
    });
    Ok(())
}

fn after_class() -> Result<()> {
    if let Some(Statics {
        dir,
        analyzer,
        reader,
        searcher,
    }) = STATICS.lock().unwrap_or_else(|e| e.into_inner()).take()
    {
        drop(searcher);
        reader.close()?;
        dir.close()?;
        analyzer.close();
    }
    Ok(())
}

const EUCLIDEAN: VectorSimilarityFunction = VectorSimilarityFunction::EUCLIDEAN;

fn const_f(v: &[f32]) -> Result<Arc<dyn ValueSource>> {
    Ok(Arc::new(ConstKnnFloatValueSource::new(v.to_vec())?))
}

fn const_b(v: &[u8]) -> Arc<dyn ValueSource> {
    Arc::new(ConstKnnByteVectorValueSource::new(v.to_vec()))
}

fn field_f(name: &str) -> Arc<dyn ValueSource> {
    Arc::new(FloatKnnVectorFieldSource::new(name))
}

fn field_b(name: &str) -> Arc<dyn ValueSource> {
    Arc::new(ByteKnnVectorFieldSource::new(name))
}

fn fsim(v1: Arc<dyn ValueSource>, v2: Arc<dyn ValueSource>) -> Arc<dyn ValueSource> {
    Arc::new(FloatVectorSimilarityFunction::new(EUCLIDEAN, v1, v2))
}

fn bsim(v1: Arc<dyn ValueSource>, v2: Arc<dyn ValueSource>) -> Arc<dyn ValueSource> {
    Arc::new(ByteVectorSimilarityFunction::new(EUCLIDEAN, v1, v2))
}

fn float_constant_vectors() -> Result<()> {
    let v1 = const_f(&[1.0, 2.0, 3.0])?;
    let v2 = const_f(&[5.0, 4.0, 1.0])?;
    assert_hits_vs(fsim(v1, v2), &[0.04, 0.04])
}

fn byte_constant_vectors() -> Result<()> {
    let v1 = const_b(&[1, 2, 3]);
    let v2 = const_b(&[2, 5, 6]);
    assert_hits_vs(bsim(v1, v2), &[0.05, 0.05])
}

fn float_field_vectors() -> Result<()> {
    let v1 = field_f("knnFloatField1");
    let v2 = field_f("knnFloatField2");
    assert_hits_vs(fsim(v1, v2), &[0.049776014, 0.049776014])
}

fn byte_field_vectors() -> Result<()> {
    let v1 = field_b("knnByteField1");
    let v2 = field_b("knnByteField2");
    assert_hits_vs(bsim(v1, v2), &[0.1, 0.1])
}

fn float_const_and_float_field_vectors() -> Result<()> {
    let v1 = const_f(&[1.0, 2.0, 4.0])?;
    let v2 = field_f("knnFloatField1");
    assert_hits_vs(fsim(v1, v2), &[0.5, 0.5])
}

fn byte_const_and_byte_field_vectors() -> Result<()> {
    let v1 = const_b(&[1, 2, 4]);
    let v2 = field_b("knnByteField1");
    assert_hits_vs(bsim(v1, v2), &[0.5, 0.5])
}

fn missing_float_vector_field() -> Result<()> {
    let v1 = const_f(&[2.0, 1.0, 1.0])?;
    let v2 = field_f("knnFloatField3");
    assert_hits_vs(fsim(v1, v2), &[0.5, 0.0])
}

fn missing_byte_vector_field() -> Result<()> {
    let v1 = const_b(&[2, 1, 1]);
    let v2 = field_b("knnByteField3");
    assert_hits_vs(bsim(v1, v2), &[0.5, 0.0])
}

/// `assertThrows(class, () -> searcher.search(new FunctionQuery(func), 10))`.
fn assert_search_throws(class: &str, func: Arc<dyn ValueSource>) {
    let searcher = searcher();
    expect_throws(class, || {
        searcher.search(Arc::new(FunctionQuery::new(func)), 10)
    });
}

fn two_vectors_with_different_dimensions() -> Result<()> {
    let v1 = const_b(&[1, 2, 3, 4]);
    let v2 = field_b("knnByteField1");
    assert_search_throws("java.lang.AssertionError", bsim(v1, v2));

    let v1 = const_f(&[1.0, 2.0])?;
    let v2 = field_f("knnFloatField1");
    assert_search_throws("java.lang.AssertionError", fsim(v1, v2));
    Ok(())
}

fn byte_and_float_vectors() -> Result<()> {
    let v1 = const_b(&[1, 2, 3]);
    let v2 = field_b("knnByteField1");
    assert_search_throws("java.lang.UnsupportedOperationException", fsim(v1, v2));

    let v1 = const_b(&[1, 2, 3]);
    let v2 = field_f("knnFloatField1");
    assert_search_throws("java.lang.UnsupportedOperationException", bsim(v1, v2));
    Ok(())
}

fn wrong_field_type() -> Result<()> {
    let v1 = field_b("knnByteField1");
    let v2 = field_b("knnFloatField2");
    assert_search_throws("java.lang.IllegalStateException", bsim(v1, v2));

    let v1 = field_f("knnByteField1");
    let v2 = field_f("knnFloatField2");
    assert_search_throws("java.lang.IllegalStateException", fsim(v1, Arc::clone(&v2)));

    let v1 = field_f("id");
    assert_search_throws("java.lang.IllegalStateException", fsim(v1, v2));
    Ok(())
}

/// `assertHits(ValueSource, float[])`.
fn assert_hits_vs(func: Arc<dyn ValueSource>, scores: &[f32]) -> Result<()> {
    assert_hits(Arc::new(FunctionQuery::new(Arc::clone(&func))), scores)?;
    assert_hits(
        Arc::new(FunctionScoreQuery::new(
            Arc::new(MatchAllDocsQuery::instance()),
            func.as_double_values_source(),
        )),
        scores,
    )
}

/// `assertHits(Query, float[])`.
fn assert_hits(q: Arc<dyn Query>, scores: &[f32]) -> Result<()> {
    let mut expected: Vec<ScoreDoc> = Vec::with_capacity(scores.len());
    let mut expected_docs = vec![0i32; scores.len()];
    for i in 0..scores.len() {
        expected_docs[i] = i as i32;
        expected.push(ScoreDoc::new(i as i32, scores[i]));
    }
    let docs = searcher().search_with_sort_and_scores(
        Arc::clone(&q),
        DOCUMENTS.len() as i32,
        &id_sort()?,
        true,
    )?;
    let score_docs: Vec<ScoreDoc> = docs.score_docs.iter().map(|fd| fd.score_doc).collect();
    CheckHits::check_hits_query(&*q, &expected, &score_docs, &expected_docs);
    Ok(())
}
