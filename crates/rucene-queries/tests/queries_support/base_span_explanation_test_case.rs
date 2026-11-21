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

//! Port of `BaseSpanExplanationTestCase.java` of Lucene 10.5.0
//! `lucene/queries/src/test/org/apache/lucene/queries/spans/`: the
//! `BaseExplanationTestCase` subclass whose macros build span queries.
//!
//! Java's instance macros are free functions here (they read no instance
//! state); its overloads take distinct names (`sor(String, String)` is
//! [`sor`], `sor(SpanQuery, SpanQuery)` is [`sor_q`], ...).

use std::sync::{Arc, LazyLock};

use randomizedtesting::jdk::reflect::{modifier, Class};
use rucene::error::Result;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_test_framework::search::base_explanation_test_case::{self, FIELD};

use super::span_test_util::{
    span_first_query, span_near_ordered_query, span_near_unordered_query, span_not_query,
    span_or_query, span_or_query_terms, span_term_query,
};

/// `public abstract class BaseSpanExplanationTestCase extends
/// BaseExplanationTestCase` (no members of its own besides the macros).
pub fn class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        Class::builder("org.apache.lucene.queries.spans.BaseSpanExplanationTestCase")
            .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
            .extends(&base_explanation_test_case::class())
            .build()
    });
    Arc::clone(&CLASS)
}

/// MACRO for SpanTermQuery.
pub fn st(s: &str) -> Arc<dyn SpanQuery> {
    span_term_query(FIELD, s)
}

/// MACRO for SpanNotQuery.
pub fn snot(i: Arc<dyn SpanQuery>, e: Arc<dyn SpanQuery>) -> Result<Arc<dyn SpanQuery>> {
    span_not_query(i, e)
}

/// MACRO for SpanOrQuery containing two SpanTerm queries.
pub fn sor(s: &str, e: &str) -> Result<Arc<dyn SpanQuery>> {
    span_or_query_terms(FIELD, &[s, e])
}

/// MACRO for SpanOrQuery containing two SpanQueries.
pub fn sor_q(s: Arc<dyn SpanQuery>, e: Arc<dyn SpanQuery>) -> Result<Arc<dyn SpanQuery>> {
    span_or_query(vec![s, e])
}

/// MACRO for SpanOrQuery containing three SpanTerm queries.
pub fn sor3(s: &str, m: &str, e: &str) -> Result<Arc<dyn SpanQuery>> {
    span_or_query_terms(FIELD, &[s, m, e])
}

/// MACRO for SpanOrQuery containing two SpanQueries (three, as Java's
/// overload `sor(SpanQuery, SpanQuery, SpanQuery)`).
pub fn sor_q3(
    s: Arc<dyn SpanQuery>,
    m: Arc<dyn SpanQuery>,
    e: Arc<dyn SpanQuery>,
) -> Result<Arc<dyn SpanQuery>> {
    span_or_query(vec![s, m, e])
}

/// MACRO for SpanNearQuery containing two SpanTerm queries.
pub fn snear(s: &str, e: &str, slop: i32, in_order: bool) -> Result<Arc<dyn SpanQuery>> {
    snear_q(st(s), st(e), slop, in_order)
}

/// MACRO for SpanNearQuery containing two SpanQueries.
pub fn snear_q(
    s: Arc<dyn SpanQuery>,
    e: Arc<dyn SpanQuery>,
    slop: i32,
    in_order: bool,
) -> Result<Arc<dyn SpanQuery>> {
    if in_order {
        span_near_ordered_query(slop, vec![s, e])
    } else {
        span_near_unordered_query(slop, vec![s, e])
    }
}

/// MACRO for SpanNearQuery containing three SpanTerm queries.
pub fn snear3(s: &str, m: &str, e: &str, slop: i32, in_order: bool) -> Result<Arc<dyn SpanQuery>> {
    snear_q3(st(s), st(m), st(e), slop, in_order)
}

/// MACRO for SpanNearQuery containing three SpanQueries.
pub fn snear_q3(
    s: Arc<dyn SpanQuery>,
    m: Arc<dyn SpanQuery>,
    e: Arc<dyn SpanQuery>,
    slop: i32,
    in_order: bool,
) -> Result<Arc<dyn SpanQuery>> {
    if in_order {
        span_near_ordered_query(slop, vec![s, m, e])
    } else {
        span_near_unordered_query(slop, vec![s, m, e])
    }
}

/// MACRO for SpanFirst(SpanTermQuery).
pub fn sf(s: &str, b: i32) -> Arc<dyn SpanQuery> {
    span_first_query(st(s), b)
}
