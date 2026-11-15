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

//! Port of `TestFieldScoreQuery.java`: test FieldScoreQuery search.
//!
//! Tests here create an index with a few documents, each having an int value
//! indexed field and a float value indexed field. The values of these fields
//! are later used for scoring.
//!
//! The rank tests use Hits to verify that docs are ordered (by score) as
//! expected.
//!
//! The exact score tests use TopDocs top to verify the exact score.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{Assert, BeforeClass};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::search::query::{query_to_string, Query};
use rucene::search::similarities::ExplanationValue;
use rucene_queries::function::function_query::FunctionQuery;
use rucene_queries::function::function_range_query::FunctionRangeQuery;
use rucene_queries::function::value_source::ValueSource;
use rucene_test_framework::search::query_utils::QueryUtils;
use rucene_test_framework::util::LuceneTestCase;

use crate::function::function_test_setup::{
    create_index, dir, expected_field_score, float_mv_max_valuesource, float_mv_min_valuesource,
    float_valuesource, int_mv_max_valuesource, int_mv_min_valuesource, int_valuesource, log,
    subclass, ID_FIELD, N_DOCS, TEST_SCORE_TOLERANCE_DELTA,
};
use crate::queries_support::core_support::{random, test};
use crate::queries_support::search_support::static_hook;

/// `TestFieldScoreQuery`.
pub fn class_def() -> Arc<Class> {
    subclass(
        "org.apache.lucene.queries.function.TestFieldScoreQuery",
        vec![
            static_hook("beforeClass", BeforeClass, || create_index(true)),
            test("testRankInt", test_rank_int),
            test("testRankIntMultiValued", test_rank_int_multi_valued),
            test("testRankFloat", test_rank_float),
            test("testRankFloatMultiValued", test_rank_float_multi_valued),
            test("testExactScoreInt", test_exact_score_int),
            test(
                "testExactScoreIntMultiValued",
                test_exact_score_int_multi_valued,
            ),
            test("testExactScoreFloat", test_exact_score_float),
            test(
                "testExactScoreFloatMultiValued",
                test_exact_score_float_multi_valued,
            ),
        ],
    )
}

/// Test that FieldScoreQuery of Type.INT returns docs in expected order.
fn test_rank_int() -> Result<()> {
    do_test_rank(int_valuesource())
}

fn test_rank_int_multi_valued() -> Result<()> {
    do_test_rank(int_mv_max_valuesource())?;
    do_test_rank(int_mv_min_valuesource())
}

/// Test that FieldScoreQuery of Type.FLOAT returns docs in expected order.
fn test_rank_float() -> Result<()> {
    // same values, but in flot format
    do_test_rank(float_valuesource())
}

fn test_rank_float_multi_valued() -> Result<()> {
    // same values, but in flot format
    do_test_rank(float_mv_max_valuesource())?;
    do_test_rank(float_mv_min_valuesource())
}

// Test that FieldScoreQuery returns docs in expected order.
fn do_test_rank(value_source: Arc<dyn ValueSource>) -> Result<()> {
    let function_query = get_function_query(value_source);
    let r = directory_reader::open(dir())?;
    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    log(|| format!("test: {}", query_to_string(&*function_query)));
    QueryUtils::check_searcher(&*random(), &function_query, Some(&s));
    let h = s.search(Arc::clone(&function_query), 1000)?.score_docs;
    Assert::assert_equals_msg(
        Some("All docs should be matched!"),
        N_DOCS as usize,
        h.len(),
    );
    let mut prev_id = format!("ID{}", N_DOCS + 1); // greater than all ids of docs in this test
    for i in 0..h.len() {
        let res_id = s
            .stored_fields()?
            .document(h[i].doc)?
            .get(ID_FIELD)
            .unwrap_or_default();
        log(|| format!("{i}.   score={}  -  {res_id}", h[i].score));
        if crate::queries_support::core_support::verbose() {
            log(|| {
                s.explain(Arc::clone(&function_query), h[i].doc)
                    .map(|e| e.to_string())
                    .unwrap_or_default()
            });
        }
        Assert::assert_true_msg(
            &format!("res id {res_id} should be < prev res id {prev_id}"),
            res_id < prev_id,
        );
        prev_id = res_id;
    }
    r.close()
}

/// Test that FieldScoreQuery of Type.INT returns the expected scores.
fn test_exact_score_int() -> Result<()> {
    do_test_exact_score(int_valuesource())
}

fn test_exact_score_int_multi_valued() -> Result<()> {
    do_test_exact_score(int_mv_max_valuesource())?;
    do_test_exact_score(int_mv_min_valuesource())
}

/// Test that FieldScoreQuery of Type.FLOAT returns the expected scores.
fn test_exact_score_float() -> Result<()> {
    // same values, but in flot format
    do_test_exact_score(float_valuesource())
}

fn test_exact_score_float_multi_valued() -> Result<()> {
    // same values, but in flot format
    do_test_exact_score(float_mv_max_valuesource())?;
    do_test_exact_score(float_mv_min_valuesource())
}

// Test that FieldScoreQuery returns docs with expected score.
fn do_test_exact_score(value_source: Arc<dyn ValueSource>) -> Result<()> {
    let function_query = get_function_query(value_source);
    let r = directory_reader::open(dir())?;
    let s = LuceneTestCase::new_searcher(Arc::clone(&r));
    let td = s.search(Arc::clone(&function_query), 1000)?;
    Assert::assert_equals_msg(
        Some("All docs should be matched!"),
        i64::from(N_DOCS),
        td.total_hits.value(),
    );
    for a_sd in &td.score_docs {
        let score = a_sd.score;
        if crate::queries_support::core_support::verbose() {
            log(|| {
                s.explain(Arc::clone(&function_query), a_sd.doc)
                    .map(|e| e.to_string())
                    .unwrap_or_default()
            });
        }
        let id = s
            .get_index_reader()
            .stored_fields()?
            .document(a_sd.doc)?
            .get(ID_FIELD)
            .unwrap_or_default();
        let expected_score = expected_field_score(&id); // "ID7" --> 7.0
        Assert::assert_equals_float_msg(
            Some(&format!(
                "score of {id} shuould be {} != {}",
                rucene::util::java_fmt::float_to_string(expected_score),
                rucene::util::java_fmt::float_to_string(score)
            )),
            expected_score,
            score,
            TEST_SCORE_TOLERANCE_DELTA,
        );
    }
    r.close()
}

/// `getFunctionQuery(ValueSource)`.
fn get_function_query(value_source: Arc<dyn ValueSource>) -> Arc<dyn Query> {
    if random().next_boolean() {
        Arc::new(FunctionQuery::new(value_source))
    } else {
        let lower = if random().next_boolean() {
            None
        } else {
            Some(ExplanationValue::Int(1))
        }; // 1 is the lowest value
        let upper = if random().next_boolean() {
            None
        } else {
            Some(ExplanationValue::Int(N_DOCS))
        }; // N_DOCS is the highest value
        Arc::new(FunctionRangeQuery::from_numbers(
            value_source,
            lower,
            upper,
            true,
            true,
        )) // will match all docs based on the indexed data
    }
}
