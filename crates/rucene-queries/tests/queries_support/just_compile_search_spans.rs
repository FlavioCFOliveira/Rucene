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

//! Port of `JustCompileSearchSpans.java` of Lucene 10.5.0
//! `lucene/queries/src/test/org/apache/lucene/queries/spans/`.
//!
//! Holds all implementations of classes in the o.a.l.s.spans package as a
//! back-compatibility test. It does not run any tests per-se, however if
//! someone adds a method to an interface or abstract method to an abstract
//! class, one of the implementations here will fail to compile and so we
//! know back-compat policy was violated.
//!
//! In Rust the abstract classes are the traits [`Spans`] and [`SpanQuery`]
//! (with [`Query`] and [`DocIdSetIterator`] above them): a required method
//! added to any of them fails the compilation of this module.

use std::any::Any;
use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use rucene::error::{LuceneError, Result};
use rucene::search::doc_id_set_iterator::DocIdSetIterator;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::Query;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::score_mode::ScoreMode;
use rucene_queries::spans::span_collector::SpanCollector;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_queries::spans::span_weight::SpanWeight;
use rucene_queries::spans::spans::Spans;

/// `UNSUPPORTED_MSG`.
const UNSUPPORTED_MSG: &str = "unsupported: used for back-compat testing only !";

/// `throw new UnsupportedOperationException(UNSUPPORTED_MSG)` from a method
/// whose Rust signature cannot fail.
fn unsupported() -> ! {
    Throwable::new("java.lang.UnsupportedOperationException", UNSUPPORTED_MSG).throw()
}

/// The same exception from a method that returns a `Result`.
fn unsupported_err() -> LuceneError {
    LuceneError::unsupported_operation(UNSUPPORTED_MSG.to_string())
}

/// `JustCompileSearchSpans.JustCompileSpans`.
#[derive(Debug)]
pub struct JustCompileSpans;

impl DocIdSetIterator for JustCompileSpans {
    fn doc_id(&self) -> i32 {
        unsupported()
    }

    fn next_doc(&mut self) -> Result<i32> {
        Err(unsupported_err())
    }

    fn advance(&mut self, _target: i32) -> Result<i32> {
        Err(unsupported_err())
    }

    fn cost(&self) -> i64 {
        unsupported()
    }
}

impl Spans for JustCompileSpans {
    fn start_position(&self) -> i32 {
        unsupported()
    }

    fn end_position(&self) -> i32 {
        unsupported()
    }

    fn width(&self) -> i32 {
        unsupported()
    }

    fn collect(&mut self, _collector: &mut dyn SpanCollector) -> Result<()> {
        Ok(())
    }

    fn next_start_position(&mut self) -> Result<i32> {
        Err(unsupported_err())
    }

    fn positions_cost(&self) -> f32 {
        unsupported()
    }
}

/// `JustCompileSearchSpans.JustCompileSpanQuery`.
#[derive(Debug)]
pub struct JustCompileSpanQuery;

impl Query for JustCompileSpanQuery {
    fn to_query_string(&self, _field: &str) -> String {
        unsupported()
    }

    fn visit(&self, _visitor: &mut dyn QueryVisitor) {
        unsupported()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn query_eq(&self, _other: &dyn Query) -> bool {
        unsupported()
    }

    fn query_hash(&self) -> i32 {
        unsupported()
    }
}

impl SpanQuery for JustCompileSpanQuery {
    fn get_field(&self) -> Option<&str> {
        unsupported()
    }

    fn create_span_weight(
        &self,
        _searcher: &IndexSearcher,
        _score_mode: ScoreMode,
        _boost: f32,
    ) -> Result<Arc<dyn SpanWeight>> {
        Err(unsupported_err())
    }
}
