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

//! Port of the test classes of Lucene 10.5.0
//! `lucene/queries/src/test/org/apache/lucene/queries/spans/` that rmp task
//! #480 lists, each run as a suite by the ported runner: `TestBasics`,
//! `TestFieldMaskingSpanQuery`, `TestFilterSpans`, `TestNearSpansOrdered`,
//! `TestQueryRescorerWithSpans`, `TestSpanCollection`,
//! `TestSpanContainQuery`, `TestSpanExplanations`,
//! `TestSpanExplanationsOfNonMatches`, `TestSpanFirstQuery`,
//! `TestSpanMatches`, `TestSpanMultiTermQueryWrapper`, `TestSpanNearQuery`,
//! `TestSpanNotQuery`, `TestSpanOrQuery`, `TestSpanQueryVisitor`,
//! `TestSpanSearchEquivalence`, `TestSpanSimilarity`, `TestSpanTermQuery`,
//! `TestSpanWithinQuery`, `TestSpans` and `TestSpansEnum`.
//!
//! The test helpers of the package (`AssertingSpanQuery`,
//! `AssertingSpanWeight`, `AssertingSpans`, `BaseSpanExplanationTestCase`,
//! `JustCompileSearchSpans`, `SpanTestUtil`) live in the shared
//! `queries_support` module.

// The ports keep Java's indexed loops and counters line by line.
#![allow(clippy::needless_range_loop)]

#[macro_use]
mod queries_support;

mod spans {
    pub mod test_basics;
    pub mod test_field_masking_span_query;
    pub mod test_filter_spans;
    pub mod test_near_spans_ordered;
    pub mod test_query_rescorer_with_spans;
    pub mod test_span_collection;
    pub mod test_span_contain_query;
    pub mod test_span_explanations;
    pub mod test_span_first_query;
    pub mod test_span_matches;
    pub mod test_span_multi_term_query_wrapper;
    pub mod test_span_near_query;
    pub mod test_span_not_query;
    pub mod test_span_or_query;
    pub mod test_span_query_visitor;
    pub mod test_span_search_equivalence;
    pub mod test_span_similarity;
    pub mod test_span_term_query;
    pub mod test_span_within_query;
    pub mod test_spans;
    pub mod test_spans_enum;
}

use queries_support::core_support::run;

#[test]
fn test_basics() {
    run(spans::test_basics::class_def);
}

#[test]
fn test_field_masking_span_query() {
    run(spans::test_field_masking_span_query::class_def);
}

#[test]
fn test_filter_spans() {
    run(spans::test_filter_spans::class_def);
}

#[test]
fn test_near_spans_ordered() {
    run(spans::test_near_spans_ordered::class_def);
}

#[test]
fn test_query_rescorer_with_spans() {
    run(spans::test_query_rescorer_with_spans::class_def);
}

#[test]
fn test_span_collection() {
    run(spans::test_span_collection::class_def);
}

#[test]
fn test_span_contain_query() {
    run(spans::test_span_contain_query::class_def);
}

#[test]
fn test_span_explanations() {
    run(spans::test_span_explanations::class_def);
}

#[test]
fn test_span_explanations_of_non_matches() {
    run(spans::test_span_explanations::non_matches_class_def);
}

#[test]
fn test_span_first_query() {
    run(spans::test_span_first_query::class_def);
}

#[test]
fn test_span_matches() {
    run(spans::test_span_matches::class_def);
}

#[test]
fn test_span_multi_term_query_wrapper() {
    run(spans::test_span_multi_term_query_wrapper::class_def);
}

#[test]
fn test_span_near_query() {
    run(spans::test_span_near_query::class_def);
}

#[test]
fn test_span_not_query() {
    run(spans::test_span_not_query::class_def);
}

#[test]
fn test_span_or_query() {
    run(spans::test_span_or_query::class_def);
}

#[test]
fn test_span_query_visitor() {
    run(spans::test_span_query_visitor::class_def);
}

#[test]
fn test_span_search_equivalence() {
    run(spans::test_span_search_equivalence::class_def);
}

#[test]
fn test_span_similarity() {
    run(spans::test_span_similarity::class_def);
}

#[test]
fn test_span_term_query() {
    run(spans::test_span_term_query::class_def);
}

#[test]
fn test_span_within_query() {
    run(spans::test_span_within_query::class_def);
}

#[test]
fn test_spans() {
    run(spans::test_spans::class_def);
}

#[test]
fn test_spans_enum() {
    run(spans::test_spans_enum::class_def);
}
