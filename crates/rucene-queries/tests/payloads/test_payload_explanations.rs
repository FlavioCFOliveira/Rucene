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

//! Port of `TestPayloadExplanations.java`: TestExplanations subclass
//! focusing on payload queries.
//!
//! `testAllFunctions(SpanQuery, int[])` takes arguments, so it is a helper
//! and not a test method (Lucene's JUnit 3 method provider runs the public
//! no-argument `test*` methods only).

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor};
use rucene::error::Result;
use rucene::index::Term;
use rucene_queries::payloads::average_payload_function::AveragePayloadFunction;
use rucene_queries::payloads::max_payload_function::MaxPayloadFunction;
use rucene_queries::payloads::min_payload_function::MinPayloadFunction;
use rucene_queries::payloads::payload_decoder::float_decoder;
use rucene_queries::payloads::payload_function::PayloadFunction;
use rucene_queries::payloads::payload_score_query::PayloadScoreQuery;
use rucene_queries::spans::span_near_query::SpanNearQuery;
use rucene_queries::spans::span_or_query::SpanOrQuery;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_queries::spans::span_term_query::SpanTermQuery;
use rucene_test_framework::search::base_explanation_test_case::{
    self, qtest, ExplanationTestInstance, FIELD,
};

use crate::queries_support::core_support::random;
use crate::queries_support::search_support::ftest;

type F = ExplanationTestInstance;

/// `functions` (`private static final`).
fn functions() -> Vec<Arc<dyn PayloadFunction>> {
    vec![
        Arc::new(AveragePayloadFunction::new()),
        Arc::new(MinPayloadFunction::new()),
        Arc::new(MaxPayloadFunction::new()),
    ]
}

/// `TestPayloadExplanations extends BaseExplanationTestCase`.
pub fn class_def() -> Arc<Class> {
    Class::builder("org.apache.lucene.queries.payloads.TestPayloadExplanations")
        .extends(&base_explanation_test_case::class())
        .constructor(Constructor::no_args(ExplanationTestInstance::default))
        .method(ftest::<F>("testPT1", test_pt1))
        .method(ftest::<F>("testPT4", test_pt4))
        .method(ftest::<F>("testSimpleTerm", test_simple_term))
        .method(ftest::<F>("testOrTerm", test_or_term))
        .method(ftest::<F>("testOrderedNearQuery", test_ordered_near_query))
        .method(ftest::<F>(
            "testUnorderedNearQuery",
            test_unordered_near_query,
        ))
        .build()
}

fn stq(text: &str) -> Arc<dyn SpanQuery> {
    Arc::new(SpanTermQuery::new(Term::from_text(FIELD, text)))
}

/// macro for payloadscorequery
fn pt(s: &str, f: &Arc<dyn PayloadFunction>) -> Arc<dyn SpanQuery> {
    Arc::new(PayloadScoreQuery::new(
        stq(s),
        Arc::clone(f),
        float_decoder(),
        random().next_boolean(),
    ))
}

/* simple PayloadTermQueries */

fn test_pt1(_f: &mut F) -> Result<()> {
    for f in functions() {
        qtest(pt("w1", &f), &[0, 1, 2, 3])?;
    }
    Ok(())
}

fn test_pt4(_f: &mut F) -> Result<()> {
    for f in functions() {
        qtest(pt("xx", &f), &[2, 3])?;
    }
    Ok(())
}

/*
  protected static final String[] docFields = {
  "w1 w2 w3 w4 w5",
  "w1 w3 w2 w3 zz",
  "w1 xx w2 yy w3",
  "w1 w3 xx w2 yy w3 zz"
};
 */

/// `testAllFunctions(SpanQuery query, int[] expected)`.
pub fn test_all_functions(query: Arc<dyn SpanQuery>, expected: &[i32]) -> Result<()> {
    for f in functions() {
        qtest(
            Arc::new(PayloadScoreQuery::new(
                Arc::clone(&query),
                f,
                float_decoder(),
                random().next_boolean(),
            )),
            expected,
        )?;
    }
    Ok(())
}

fn test_simple_term(_f: &mut F) -> Result<()> {
    let q = stq("w2");
    test_all_functions(q, &[0, 1, 2, 3])
}

fn test_or_term(_f: &mut F) -> Result<()> {
    let q: Arc<dyn SpanQuery> = Arc::new(SpanOrQuery::new(vec![stq("xx"), stq("yy")])?);
    test_all_functions(q, &[2, 3])
}

fn test_ordered_near_query(_f: &mut F) -> Result<()> {
    let q: Arc<dyn SpanQuery> = Arc::new(SpanNearQuery::new(vec![stq("w3"), stq("w2")], 1, true)?);
    test_all_functions(q, &[1, 3])
}

fn test_unordered_near_query(_f: &mut F) -> Result<()> {
    let q: Arc<dyn SpanQuery> = Arc::new(SpanNearQuery::new(vec![stq("w2"), stq("w3")], 1, false)?);
    test_all_functions(q, &[0, 1, 2, 3])
}
