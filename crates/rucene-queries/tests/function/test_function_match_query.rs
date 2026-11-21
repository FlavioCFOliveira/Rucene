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

//! Port of `TestFunctionMatchQuery.java`.

use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{AfterClass, Assert, BeforeClass};
use rucene::error::Result;
use rucene::index::{directory_reader, IndexReader};
use rucene::search::double_values_source::{self, DoubleValuesSource};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::Query;
use rucene::search::score_mode::ScoreMode;
use rucene_queries::function::function_match_query::{
    DoublePredicate, FunctionMatchQuery, DEFAULT_MATCH_COST,
};
use rucene_test_framework::search::query_utils::QueryUtils;
use rucene_test_framework::util::LuceneTestCase;

use crate::function::function_test_setup::{create_index, dir, subclass, FLOAT_FIELD};
use crate::queries_support::core_support::{random, test};
use crate::queries_support::search_support::static_hook;

/// The static fields `reader` and `searcher`.
struct Statics {
    reader: Arc<dyn IndexReader>,
    searcher: Arc<IndexSearcher>,
}

static STATICS: Mutex<Option<Statics>> = Mutex::new(None);

/// `in`: `DoubleValuesSource.fromFloatField(FLOAT_FIELD)`.
static IN: LazyLock<Arc<dyn DoubleValuesSource>> =
    LazyLock::new(|| double_values_source::from_float_field(FLOAT_FIELD));

fn statics() -> (Arc<dyn IndexReader>, Arc<IndexSearcher>) {
    let g = STATICS.lock().unwrap_or_else(|e| e.into_inner());
    let s = g.as_ref().expect("beforeClass ran");
    (Arc::clone(&s.reader), Arc::clone(&s.searcher))
}

/// `TestFunctionMatchQuery`.
pub fn class_def() -> Arc<Class> {
    subclass(
        "org.apache.lucene.queries.function.TestFunctionMatchQuery",
        vec![
            static_hook("beforeClass", BeforeClass, before_class),
            static_hook("afterClass", AfterClass, after_class),
            test("testRangeMatching", test_range_matching),
            test(
                "testTwoPhaseIteratorMatchCost",
                test_two_phase_iterator_match_cost,
            ),
        ],
    )
}

fn before_class() -> Result<()> {
    create_index(true)?;
    let reader: Arc<dyn IndexReader> = directory_reader::open(dir())? as Arc<dyn IndexReader>;
    let searcher = Arc::new(IndexSearcher::new(Arc::clone(&reader))?);
    *STATICS.lock().unwrap_or_else(|e| e.into_inner()) = Some(Statics { reader, searcher });
    Ok(())
}

fn after_class() -> Result<()> {
    if let Some(Statics { reader, searcher }) =
        STATICS.lock().unwrap_or_else(|e| e.into_inner()).take()
    {
        drop(searcher);
        reader.close()?;
    }
    Ok(())
}

fn test_range_matching() -> Result<()> {
    let (_, searcher) = statics();
    let fmq: Arc<dyn Query> = Arc::new(FunctionMatchQuery::new(
        Arc::clone(&IN),
        // `d -> d >= 2 && d < 4`
        Arc::new(|d: f64| (2.0..4.0).contains(&d)),
    ));
    let docs = searcher.search(Arc::clone(&fmq), 10)?;
    Assert::assert_equals(2i64, docs.total_hits.value());
    Assert::assert_equals(9, docs.score_docs[0].doc);
    Assert::assert_equals(13, docs.score_docs[1].doc);
    QueryUtils::check_searcher_wrap(&*random(), &fmq, Some(&searcher), LuceneTestCase::rarely());
    Ok(())
}

fn test_two_phase_iterator_match_cost() -> Result<()> {
    let predicate: DoublePredicate = Arc::new(|_d: f64| true);

    // should use default match cost
    let fmq = FunctionMatchQuery::new(Arc::clone(&IN), Arc::clone(&predicate));
    Assert::assert_equals_double_msg(
        None,
        f64::from(DEFAULT_MATCH_COST),
        f64::from(get_match_cost(&fmq)?),
        0.1,
    );

    // should use client defined match cost
    let fmq = FunctionMatchQuery::with_match_cost(Arc::clone(&IN), predicate, 200.0);
    Assert::assert_equals_double_msg(None, 200.0, f64::from(get_match_cost(&fmq)?), 0.1);
    Ok(())
}

/// `getMatchCost(FunctionMatchQuery)`: Java dereferences the scorer and its
/// two-phase iterator, so a `null` is a `NullPointerException`.
fn get_match_cost(fmq: &FunctionMatchQuery) -> Result<f32> {
    let (reader, searcher) = statics();
    let ctx = Arc::clone(&reader.leaves()?[0]);
    let w = fmq.create_weight(&searcher, ScoreMode::TOP_DOCS, 1.0)?;
    let mut scorer = w.scorer(&ctx)?.unwrap_or_else(|| {
        randomizedtesting::jdk::exception::throw_without_message("java.lang.NullPointerException")
    });
    let tpi = scorer.two_phase_iterator().unwrap_or_else(|| {
        randomizedtesting::jdk::exception::throw_without_message("java.lang.NullPointerException")
    });
    Ok(tpi.match_cost())
}
