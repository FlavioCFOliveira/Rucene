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

//! Port of `TestSpanTermQuery.java`: basic tests for SpanTermQuery.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::{Document, Store, StringField};
use rucene::error::Result;
use rucene::index::{IndexReader, Term};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::Query;
use rucene_queries::spans::span_term_query::SpanTermQuery;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::query_utils::QueryUtils;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::LuceneTestCase;

use crate::queries_support::core_support::{assert_contains, class, random, test};

/// `TestSpanTermQuery`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.queries.spans.TestSpanTermQuery",
        vec![
            test("testHashcodeEquals", test_hashcode_equals),
            test("testNoPositions", test_no_positions),
        ],
    )
}

fn test_hashcode_equals() -> Result<()> {
    let q1 = SpanTermQuery::new(Term::from_text("field", "foo"));
    let q2 = SpanTermQuery::new(Term::from_text("field", "bar"));
    QueryUtils::check(&q1);
    QueryUtils::check(&q2);
    QueryUtils::check_unequal(&q1, &q2);
    Ok(())
}

fn test_no_positions() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut doc = Document::new();
    doc.add(Box::new(StringField::new(
        "foo",
        "bar".to_string(),
        Store::NO,
    )?));
    iw.add_document(&doc)?;

    let ir = iw.get_reader()?;
    iw.close()?;

    let is = IndexSearcher::new(Arc::clone(&ir) as Arc<dyn IndexReader>)?;
    let query: Arc<dyn Query> = Arc::new(SpanTermQuery::new(Term::from_text("foo", "bar")));
    let expected = expect_throws("java.lang.IllegalStateException", || is.search(query, 5));
    assert_contains(
        expected.message().unwrap_or_default(),
        "was indexed without position data",
    );

    ir.close()?;
    dir.close()
}
