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

//! Port of `TestFunctionRangeQuery.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::junit::{After, Assert, Before, BeforeClass};
use rucene::error::Result;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::{directory_reader, IndexReader};
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query::Builder as BooleanQueryBuilder;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::Query;
use rucene::search::similarities::ExplanationValue;
use rucene::search::ScoreDoc;
use rucene_queries::function::function_range_query::FunctionRangeQuery;
use rucene_queries::function::value_source::ValueSource;
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::LuceneTestCase;

use crate::function::function_test_setup::{
    self, create_index, dir, float_mv_max_valuesource, float_mv_min_valuesource, float_valuesource,
    int_mv_max_valuesource, int_mv_min_valuesource, int_valuesource, INT_FIELD, INT_FIELD_MV_MIN,
    N_DOCS,
};
use crate::queries_support::search_support::{ftest, static_hook};

/// The instance of `TestFunctionRangeQuery`: `indexReader`, `indexSearcher`.
#[derive(Default)]
pub struct RangeTest {
    index_reader: Option<Arc<dyn rucene::index::directory_reader::DirectoryReader>>,
    index_searcher: Option<IndexSearcher>,
}

impl RangeTest {
    fn searcher(&self) -> &IndexSearcher {
        self.index_searcher.as_ref().expect("before() ran")
    }
}

/// `TestFunctionRangeQuery`.
pub fn class_def() -> Arc<Class> {
    Class::builder("org.apache.lucene.queries.function.TestFunctionRangeQuery")
        .extends(&function_test_setup::class())
        .constructor(Constructor::no_args(RangeTest::default))
        .method(static_hook("beforeClass", BeforeClass, || {
            create_index(true)
        }))
        .method(
            Method::new("before")
                .throws(&["java.io.IOException"])
                .annotate(Before)
                .body(|f: &mut RangeTest| before(f).unwrap_or_else(|e| rethrow_java(e))),
        )
        .method(
            Method::new("after")
                .throws(&["java.io.IOException"])
                .annotate(After)
                .body(|f: &mut RangeTest| after(f).unwrap_or_else(|e| rethrow_java(e))),
        )
        .method(ftest("testRangeInt", test_range_int))
        .method(ftest(
            "testRangeIntMultiValued",
            test_range_int_multi_valued,
        ))
        .method(ftest("testRangeFloat", test_range_float))
        .method(ftest(
            "testRangeFloatMultiValued",
            test_range_float_multi_valued,
        ))
        .method(ftest("testDeleted", test_deleted))
        .method(ftest("testDeletedMultiValued", test_deleted_multi_valued))
        .method(ftest("testExplain", test_explain))
        .method(ftest("testExplainMultiValued", test_explain_multi_valued))
        .method(ftest("testTwoRangeQueries", test_two_range_queries))
        .build()
}

fn before(f: &mut RangeTest) -> Result<()> {
    let index_reader = directory_reader::open(dir())?;
    f.index_searcher = Some(LuceneTestCase::new_searcher(Arc::clone(&index_reader)));
    f.index_reader = Some(index_reader);
    Ok(())
}

fn after(f: &mut RangeTest) -> Result<()> {
    f.index_searcher = None;
    f.index_reader.take().expect("before() ran").close()
}

/// `new FunctionRangeQuery(valueSource, (Integer) lower, (Integer) upper, includeLower,
/// includeUpper)`.
fn frange(
    value_source: Arc<dyn ValueSource>,
    lower: i32,
    upper: i32,
    include_lower: bool,
    include_upper: bool,
) -> Arc<dyn Query> {
    Arc::new(FunctionRangeQuery::from_numbers(
        value_source,
        Some(ExplanationValue::Int(lower)),
        Some(ExplanationValue::Int(upper)),
        include_lower,
        include_upper,
    ))
}

fn test_range_int(f: &mut RangeTest) -> Result<()> {
    do_test_range(f, int_valuesource())
}

fn test_range_int_multi_valued(f: &mut RangeTest) -> Result<()> {
    do_test_range(f, int_mv_max_valuesource())?;
    do_test_range(f, int_mv_min_valuesource())
}

fn test_range_float(f: &mut RangeTest) -> Result<()> {
    do_test_range(f, float_valuesource())
}

fn test_range_float_multi_valued(f: &mut RangeTest) -> Result<()> {
    do_test_range(f, float_mv_max_valuesource())?;
    do_test_range(f, float_mv_min_valuesource())
}

fn do_test_range(f: &RangeTest, value_source: Arc<dyn ValueSource>) -> Result<()> {
    let range_query = frange(Arc::clone(&value_source), 2, 4, true, false);
    let score_docs = f.searcher().search(range_query, N_DOCS)?.score_docs;
    expect_scores(&score_docs, &[3, 2]);

    let range_query = frange(value_source, 2, 4, false, true);
    let score_docs = f.searcher().search(range_query, N_DOCS)?.score_docs;
    expect_scores(&score_docs, &[4, 3]);
    Ok(())
}

fn test_deleted(_f: &mut RangeTest) -> Result<()> {
    do_test_deleted(int_valuesource())
}

fn test_deleted_multi_valued(_f: &mut RangeTest) -> Result<()> {
    do_test_deleted(int_mv_max_valuesource())?;
    do_test_deleted(int_mv_min_valuesource())
}

fn do_test_deleted(value_source: Arc<dyn ValueSource>) -> Result<()> {
    // We delete doc with #3. Note we don't commit it to disk; we search using a near real-time
    // reader.
    let writer = IndexWriter::new(dir(), IndexWriterConfig::with_analyzer(None))?;
    let body = || -> Result<()> {
        writer.delete_documents_by_queries(vec![frange(
            Arc::clone(&value_source),
            3,
            3,
            true,
            true,
        )])?; // delete the one with #3
        java_assert!(writer.has_deletions()?);
        let index_reader2: Arc<dyn IndexReader> =
            directory_reader::open_with_writer(&*writer)? as Arc<dyn IndexReader>;
        let result = (|| -> Result<()> {
            let index_searcher2 = IndexSearcher::new(Arc::clone(&index_reader2))?;
            let top_docs = index_searcher2
                .search(frange(Arc::clone(&value_source), 3, 4, true, true), N_DOCS)?;
            expect_scores(&top_docs.score_docs, &[4]); // missing #3 because it's deleted
            Ok(())
        })();
        let closed = index_reader2.close();
        result.and(closed)
    };
    let result = body();
    let rolled = writer.rollback();
    let closed = writer.close();
    result.and(rolled).and(closed)
}

fn test_explain(f: &mut RangeTest) -> Result<()> {
    let range_query = frange(int_valuesource(), 2, 2, true, true);
    let score_docs = f
        .searcher()
        .search(Arc::clone(&range_query), N_DOCS)?
        .score_docs;
    let explain = f.searcher().explain(range_query, score_docs[0].doc)?;
    // Just validate it looks reasonable
    Assert::assert_equals_str(
        None,
        Some(&format!(
            "2.0 = frange(int({INT_FIELD})):[2 TO 2]\n  2.0 = int({INT_FIELD})=2\n"
        )),
        Some(&explain.to_string()),
    );
    Ok(())
}

fn test_explain_multi_valued(f: &mut RangeTest) -> Result<()> {
    let range_query = frange(int_mv_min_valuesource(), 2, 2, true, true);
    let score_docs = f
        .searcher()
        .search(Arc::clone(&range_query), N_DOCS)?
        .score_docs;
    let explain = f.searcher().explain(range_query, score_docs[0].doc)?;
    // Just validate it looks reasonable
    Assert::assert_equals_str(
        None,
        Some(&format!(
            "2.0 = frange(int({INT_FIELD_MV_MIN},MIN)):[2 TO 2]\n  2.0 = int({INT_FIELD_MV_MIN},MIN)=2\n"
        )),
        Some(&explain.to_string()),
    );
    Ok(())
}

fn test_two_range_queries(f: &mut RangeTest) -> Result<()> {
    let rq1 = frange(int_valuesource(), 2, 4, true, true);
    let rq2 = frange(int_valuesource(), 8, 10, true, true);
    let mut b = BooleanQueryBuilder::new();
    b.add(rq1, Occur::SHOULD)?;
    b.add(rq2, Occur::SHOULD)?;
    let bq: Arc<dyn Query> = Arc::new(b.build());
    let score_docs = f.searcher().search(bq, N_DOCS)?.score_docs;
    expect_scores(&score_docs, &[10, 9, 8, 4, 3, 2]);
    Ok(())
}

fn expect_scores(score_docs: &[ScoreDoc], doc_scores: &[i32]) {
    Assert::assert_equals(doc_scores.len(), score_docs.len());
    for i in 0..doc_scores.len() {
        Assert::assert_equals_float_msg(None, doc_scores[i] as f32, score_docs[i].score, 0.0);
    }
}
