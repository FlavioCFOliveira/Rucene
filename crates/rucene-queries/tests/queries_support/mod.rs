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

//! Helpers shared by the test targets of `rucene-queries` that port the
//! tests of Lucene 10.5.0 `lucene/queries/src/test` (sprint 13, test passes
//! T2a and T2b).
//!
//! * [`core_support`] and [`search_support`]: the suite plumbing of the
//!   sprint-9 core test ports (`rucene-test-framework/tests/core_support`
//!   and `core_search/support.rs`), reused, not copied: declaring a Java
//!   test class whose methods are Rust functions, running it as a suite
//!   through the ported `randomizedtesting` runner, and the JUnit
//!   assertions the ported bodies use beyond `Assert`.
//! * The test helpers of the Java package `org.apache.lucene.queries.spans`
//!   ([`asserting_span_query`], [`asserting_span_weight`],
//!   [`asserting_spans`], [`span_test_util`],
//!   [`base_span_explanation_test_case`], [`just_compile_search_spans`]) and
//!   of `org.apache.lucene.queries.payloads` ([`payload_helper`]).
//!
//! A target uses it as `#[macro_use] mod queries_support;`: the
//! [`java_assert`] macros are textually scoped.

#![allow(dead_code, unused_macros)]

/// Java's `assert condition [: message];` under `-ea` in a method that
/// returns [`rucene::error::Result`]: a failed assertion returns
/// `Err(LuceneErrorKind::AssertionError(message))`, the error a Java
/// `AssertionError` stands for in this port (the message is empty for an
/// `assert` without one).
///
/// The same macro as the test framework's (crate-private there).
///
/// **Divergence.** Java evaluates `assert` only with `-ea` (as Lucene's
/// builds run tests); the port evaluates it when `debug_assertions` are on.
macro_rules! java_assert {
    ($cond:expr) => {
        if cfg!(debug_assertions) && !($cond) {
            return Err(rucene::error::LuceneError::from(
                rucene::error::LuceneErrorKind::AssertionError(String::new()),
            )
            .into());
        }
    };
    ($cond:expr, $($msg:tt)+) => {
        if cfg!(debug_assertions) && !($cond) {
            return Err(rucene::error::LuceneError::from(
                rucene::error::LuceneErrorKind::AssertionError(format!($($msg)+)),
            )
            .into());
        }
    };
}

/// Java's `assert condition [: message];` under `-ea` in a method whose Rust
/// signature cannot fail (`docID()`, `startPosition()`, `width()`, ...): a
/// failed assertion throws `java.lang.AssertionError` (panics with it).
macro_rules! java_assert_unchecked {
    ($cond:expr) => {
        randomizedtesting::jdk::exception::java_assert_no_message($cond)
    };
    ($cond:expr, $($msg:tt)+) => {
        randomizedtesting::jdk::exception::java_assert($cond, || format!($($msg)+))
    };
}

#[path = "../../../rucene-test-framework/tests/core_support/mod.rs"]
pub mod core_support;

#[path = "../../../rucene-test-framework/tests/core_search/support.rs"]
pub mod search_support;

pub mod asserting_span_query;
pub mod asserting_span_weight;
pub mod asserting_spans;
pub mod base_span_explanation_test_case;
pub mod just_compile_search_spans;
pub mod payload_helper;
pub mod span_test_util;

use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::index::LeafReaderContext;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::score_mode::ScoreMode;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_queries::spans::span_weight::Postings;
use rucene_queries::spans::spans::Spans;

/// `new AssertionError(message)` as a `Result` error.
pub fn assertion_error(message: impl Into<String>) -> LuceneError {
    LuceneError::from(LuceneErrorKind::AssertionError(message.into()))
}

/// `throw new AssertionError(message)` from a method whose Rust signature
/// cannot fail.
pub fn throw_assertion(message: impl Into<String>) -> ! {
    Throwable::new("java.lang.AssertionError", message.into()).throw()
}

/// The `Spans` a test dereferences: Java's `spans.nextDoc()` on a `null`
/// `Spans` throws `NullPointerException`.
pub fn non_null_spans(spans: Option<Box<dyn Spans>>) -> Box<dyn Spans> {
    spans.unwrap_or_else(|| {
        randomizedtesting::jdk::exception::throw_without_message("java.lang.NullPointerException")
    })
}

/// `assertNotSame([message,] unexpected, actual)` over two `int`s, which Java
/// autoboxes: `Integer.valueOf` shares one object per value only in
/// `-128..=127` (the `IntegerCache` of JDK 25), so two equal values outside it
/// are never the same object and the assertion passes.
pub fn assert_not_same_int(message: Option<&str>, unexpected: i32, actual: i32) {
    if unexpected == actual && (-128..=127).contains(&actual) {
        let formatted = message.map_or_else(String::new, |m| format!("{m} "));
        randomizedtesting::junit::Assert::fail(&format!("{formatted}expected not same"));
    }
}

/// `searcher.getIndexReader().leaves().get(0)`.
pub fn first_leaf(searcher: &IndexSearcher) -> Arc<LeafReaderContext> {
    Arc::clone(&searcher.get_leaf_contexts()[0])
}

/// `q.createWeight(searcher, ScoreMode.COMPLETE_NO_SCORES, 1f)
/// .getSpans(searcher.getIndexReader().leaves().get(0), postings)`, the call
/// the span tests make to iterate a query's spans on the first leaf.
pub fn leaf_spans(
    q: &dyn SpanQuery,
    searcher: &IndexSearcher,
    postings: Postings,
) -> Result<Option<Box<dyn Spans>>> {
    q.create_span_weight(searcher, ScoreMode::COMPLETE_NO_SCORES, 1.0)?
        .get_spans(&first_leaf(searcher), postings)
}
