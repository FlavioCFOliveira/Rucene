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

//! Port of `TestSpanOrQuery.java`: basic tests for SpanOrQuery.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::Term;
use rucene::search::query::Query;
use rucene_queries::spans::span_or_query::SpanOrQuery;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_queries::spans::span_term_query::SpanTermQuery;
use rucene_test_framework::search::query_utils::QueryUtils;

use crate::queries_support::core_support::{class, iae_message, test};

/// `TestSpanOrQuery`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.queries.spans.TestSpanOrQuery",
        vec![
            test("testHashcodeEquals", test_hashcode_equals),
            test("testSpanOrEmpty", test_span_or_empty),
            test("testDifferentField", test_different_field),
        ],
    )
}

fn stq(field: &str, text: &str) -> Arc<dyn SpanQuery> {
    Arc::new(SpanTermQuery::new(Term::from_text(field, text)))
}

fn test_hashcode_equals() -> Result<()> {
    let q1 = stq("field", "foo");
    let q2 = stq("field", "bar");
    let q3 = stq("field", "baz");

    let or1 = SpanOrQuery::new(vec![q1, Arc::clone(&q2)])?;
    let or2 = SpanOrQuery::new(vec![q2, q3])?;

    QueryUtils::check(&or1);
    QueryUtils::check(&or2);
    QueryUtils::check_unequal(&or1, &or2);
    Ok(())
}

fn test_span_or_empty() -> Result<()> {
    let a = SpanOrQuery::new(vec![])?;
    let b = SpanOrQuery::new(vec![])?;
    Assert::assert_true_msg("empty should equal", a.query_eq(&b));
    Ok(())
}

fn test_different_field() -> Result<()> {
    let q1 = stq("field1", "foo");
    let q2 = stq("field2", "bar");
    let expected = iae_message(|| SpanOrQuery::new(vec![q1, q2]));
    Assert::assert_true(expected.contains("must have same field"));
    Ok(())
}
