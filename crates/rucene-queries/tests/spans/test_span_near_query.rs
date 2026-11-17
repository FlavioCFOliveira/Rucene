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

//! Port of `TestSpanNearQuery.java`: basic tests for SpanNearQuery.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store, StringField};
use rucene::error::Result;
use rucene::index::{IndexReader, Term};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::Query;
use rucene_queries::spans::span_near_query::SpanNearQuery;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_queries::spans::span_term_query::SpanTermQuery;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::query_utils::QueryUtils;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::LuceneTestCase;

use crate::queries_support::core_support::{
    assert_contains, class, iae_message, random, test, IAE,
};

/// `TestSpanNearQuery`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.queries.spans.TestSpanNearQuery",
        vec![
            test("testHashcodeEquals", test_hashcode_equals),
            test("testDifferentField", test_different_field),
            test("testNoPositions", test_no_positions),
            test("testBuilder", test_builder),
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

    let near1 = SpanNearQuery::new(vec![Arc::clone(&q1), Arc::clone(&q2)], 10, true)?;
    let near2 = SpanNearQuery::new(vec![q2, q3], 10, true)?;

    QueryUtils::check(&near1);
    QueryUtils::check(&near2);
    QueryUtils::check_unequal(&near1, &near2);
    Ok(())
}

fn test_different_field() -> Result<()> {
    let q1 = stq("field1", "foo");
    let q2 = stq("field2", "bar");
    let expected = iae_message(|| SpanNearQuery::new(vec![q1, q2], 10, true));
    Assert::assert_true(expected.contains("must have same field"));
    Ok(())
}

fn test_no_positions() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut doc = Document::new();
    doc.add(StringField::new("foo", "bar".to_string(), Store::NO)?);
    iw.add_document(&doc)?;

    let ir = iw.get_reader()?;
    iw.close()?;

    let is = IndexSearcher::new(Arc::clone(&ir) as Arc<dyn IndexReader>)?;
    let query = stq("foo", "bar");
    let query2 = stq("foo", "baz");

    let expected = expect_throws("java.lang.IllegalStateException", || {
        is.search(
            Arc::new(SpanNearQuery::new(vec![query, query2], 10, true)?) as Arc<dyn Query>,
            5,
        )
    });
    assert_contains(
        expected.message().unwrap_or_default(),
        "was indexed without position data",
    );

    ir.close()?;
    dir.close()
}

fn test_builder() -> Result<()> {
    // Can't add subclauses from different fields
    expect_throws(IAE, || {
        SpanNearQuery::new_ordered_near_query("field1")
            .add_clause(stq("field2", "term"))
            .map(|_| ())
    });

    // Can't add gaps to unordered queries
    expect_throws(IAE, || {
        SpanNearQuery::new_unordered_near_query("field1")
            .add_gap(1)
            .map(|_| ())
    });
    Ok(())
}
