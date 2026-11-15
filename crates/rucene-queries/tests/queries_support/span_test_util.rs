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

//! Port of `SpanTestUtil.java` of Lucene 10.5.0
//! `lucene/queries/src/test/org/apache/lucene/queries/spans/`: some
//! utility methods used for testing span queries.
//!
//! Java's overloads take distinct Rust names: `spanOrQuery(String,
//! String...)` is [`span_or_query_terms`], `spanOrQuery(SpanQuery...)` is
//! [`span_or_query`], and so on. The constructors that throw
//! `IllegalArgumentException` (mismatched fields) return it as an `Err`.

use std::sync::Arc;

use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::Term;
use rucene::search::doc_id_set_iterator::NO_MORE_DOCS;
use rucene_queries::spans::span_containing_query::SpanContainingQuery;
use rucene_queries::spans::span_first_query::SpanFirstQuery;
use rucene_queries::spans::span_near_query::SpanNearQuery;
use rucene_queries::spans::span_not_query::SpanNotQuery;
use rucene_queries::spans::span_or_query::SpanOrQuery;
use rucene_queries::spans::span_position_range_query::SpanPositionRangeQuery;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_queries::spans::span_term_query::SpanTermQuery;
use rucene_queries::spans::span_within_query::SpanWithinQuery;
use rucene_queries::spans::spans::{Spans, NO_MORE_POSITIONS};
use rucene_test_framework::search::query_utils::QueryUtils;

use super::asserting_span_query::AssertingSpanQuery;

/// Adds additional asserts to a spanquery. Highly recommended if you want
/// tests to actually be debuggable.
///
/// `spanQuery(SpanQuery)`.
pub fn span_query(query: Arc<dyn SpanQuery>) -> Arc<dyn SpanQuery> {
    QueryUtils::check(&*query);
    Arc::new(AssertingSpanQuery::new(query))
}

/// Makes a new SpanTermQuery (with additional asserts).
///
/// `spanTermQuery(String, String)`.
pub fn span_term_query(field: &str, term: &str) -> Arc<dyn SpanQuery> {
    span_query(Arc::new(SpanTermQuery::new(Term::from_text(field, term))))
}

/// Makes a new SpanOrQuery (with additional asserts) from the provided
/// `terms`.
///
/// `spanOrQuery(String, String...)`.
pub fn span_or_query_terms(field: &str, terms: &[&str]) -> Result<Arc<dyn SpanQuery>> {
    let subqueries: Vec<Arc<dyn SpanQuery>> =
        terms.iter().map(|t| span_term_query(field, t)).collect();
    span_or_query(subqueries)
}

/// Makes a new SpanOrQuery (with additional asserts).
///
/// `spanOrQuery(SpanQuery...)`.
pub fn span_or_query(subqueries: Vec<Arc<dyn SpanQuery>>) -> Result<Arc<dyn SpanQuery>> {
    Ok(span_query(Arc::new(SpanOrQuery::new(subqueries)?)))
}

/// Makes a new SpanNotQuery (with additional asserts).
///
/// `spanNotQuery(SpanQuery, SpanQuery)`.
pub fn span_not_query(
    include: Arc<dyn SpanQuery>,
    exclude: Arc<dyn SpanQuery>,
) -> Result<Arc<dyn SpanQuery>> {
    Ok(span_query(Arc::new(SpanNotQuery::new(include, exclude)?)))
}

/// Makes a new SpanNotQuery (with additional asserts).
///
/// `spanNotQuery(SpanQuery, SpanQuery, int, int)`.
pub fn span_not_query_pre_post(
    include: Arc<dyn SpanQuery>,
    exclude: Arc<dyn SpanQuery>,
    pre: i32,
    post: i32,
) -> Result<Arc<dyn SpanQuery>> {
    Ok(span_query(Arc::new(SpanNotQuery::with_pre_post(
        include, exclude, pre, post,
    )?)))
}

/// Makes a new SpanFirstQuery (with additional asserts).
///
/// `spanFirstQuery(SpanQuery, int)`.
pub fn span_first_query(query: Arc<dyn SpanQuery>, end: i32) -> Arc<dyn SpanQuery> {
    span_query(Arc::new(SpanFirstQuery::new(query, end)))
}

/// Makes a new SpanPositionRangeQuery (with additional asserts).
///
/// `spanPositionRangeQuery(SpanQuery, int, int)`.
pub fn span_position_range_query(
    query: Arc<dyn SpanQuery>,
    start: i32,
    end: i32,
) -> Arc<dyn SpanQuery> {
    span_query(Arc::new(SpanPositionRangeQuery::new(query, start, end)))
}

/// Makes a new SpanContainingQuery (with additional asserts).
///
/// `spanContainingQuery(SpanQuery, SpanQuery)`.
pub fn span_containing_query(
    big: Arc<dyn SpanQuery>,
    little: Arc<dyn SpanQuery>,
) -> Result<Arc<dyn SpanQuery>> {
    Ok(span_query(Arc::new(SpanContainingQuery::new(big, little)?)))
}

/// Makes a new SpanWithinQuery (with additional asserts).
///
/// `spanWithinQuery(SpanQuery, SpanQuery)`.
pub fn span_within_query(
    big: Arc<dyn SpanQuery>,
    little: Arc<dyn SpanQuery>,
) -> Result<Arc<dyn SpanQuery>> {
    Ok(span_query(Arc::new(SpanWithinQuery::new(big, little)?)))
}

/// Makes a new ordered SpanNearQuery (with additional asserts) from the
/// provided `terms`.
///
/// `spanNearOrderedQuery(String, int, String...)`.
pub fn span_near_ordered_query_terms(
    field: &str,
    slop: i32,
    terms: &[&str],
) -> Result<Arc<dyn SpanQuery>> {
    let subqueries: Vec<Arc<dyn SpanQuery>> =
        terms.iter().map(|t| span_term_query(field, t)).collect();
    span_near_ordered_query(slop, subqueries)
}

/// Makes a new ordered SpanNearQuery (with additional asserts).
///
/// `spanNearOrderedQuery(int, SpanQuery...)`.
pub fn span_near_ordered_query(
    slop: i32,
    subqueries: Vec<Arc<dyn SpanQuery>>,
) -> Result<Arc<dyn SpanQuery>> {
    Ok(span_query(Arc::new(SpanNearQuery::new(
        subqueries, slop, true,
    )?)))
}

/// Makes a new unordered SpanNearQuery (with additional asserts) from the
/// provided `terms`.
///
/// `spanNearUnorderedQuery(String, int, String...)`.
pub fn span_near_unordered_query_terms(
    field: &str,
    slop: i32,
    terms: &[&str],
) -> Result<Arc<dyn SpanQuery>> {
    let mut builder = SpanNearQuery::new_unordered_near_query(field);
    builder.set_slop(slop);
    for term in terms {
        builder.add_clause(Arc::new(SpanTermQuery::new(Term::from_text(field, term))))?;
    }
    Ok(span_query(Arc::new(builder.build()?)))
}

/// Makes a new unordered SpanNearQuery (with additional asserts).
///
/// `spanNearUnorderedQuery(int, SpanQuery...)`.
pub fn span_near_unordered_query(
    slop: i32,
    subqueries: Vec<Arc<dyn SpanQuery>>,
) -> Result<Arc<dyn SpanQuery>> {
    Ok(span_query(Arc::new(SpanNearQuery::new(
        subqueries, slop, false,
    )?)))
}

/// `assertEquals(String message, long expected, long actual)`.
fn assert_equals_long(message: &str, expected: i32, actual: i32) {
    Assert::assert_equals_msg(Some(message), i64::from(expected), i64::from(actual));
}

/// Assert the next iteration from `spans` is a match from `start` to `end`
/// in `doc`.
///
/// `assertNext(Spans, int, int, int)`.
///
/// # Errors
///
/// Propagates any error raised while iterating the spans.
pub fn assert_next(spans: &mut dyn Spans, doc: i32, start: i32, end: i32) -> Result<()> {
    if spans.doc_id() >= doc {
        assert_equals_long("docId", doc, spans.doc_id());
    } else {
        // nextDoc needed before testing start/end
        if spans.doc_id() >= 0 {
            assert_equals_long(
                "nextStartPosition of previous doc",
                NO_MORE_POSITIONS,
                spans.next_start_position()?,
            );
            assert_equals_long(
                "endPosition of previous doc",
                NO_MORE_POSITIONS,
                spans.end_position(),
            );
        }
        assert_equals_long("nextDoc", doc, spans.next_doc()?);
        if doc != NO_MORE_DOCS {
            assert_equals_long("first startPosition", -1, spans.start_position());
            assert_equals_long("first endPosition", -1, spans.end_position());
        }
    }
    if doc != NO_MORE_DOCS {
        assert_equals_long("nextStartPosition", start, spans.next_start_position()?);
        assert_equals_long("startPosition", start, spans.start_position());
        assert_equals_long("endPosition", end, spans.end_position());
    }
    Ok(())
}

/// Assert that `spans` is exhausted.
///
/// `assertFinished(Spans)`: a `null` (`None`) spans is empty.
///
/// # Errors
///
/// Propagates any error raised while iterating the spans.
pub fn assert_finished(spans: Option<&mut dyn Spans>) -> Result<()> {
    if let Some(spans) = spans {
        // null Spans is empty
        assert_next(spans, NO_MORE_DOCS, -2, -2)?; // start and end positions will be ignored
    }
    Ok(())
}
