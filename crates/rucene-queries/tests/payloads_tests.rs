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
//! `lucene/queries/src/test/org/apache/lucene/queries/payloads/` that rmp
//! task #481 lists, each run as a suite by the ported runner:
//! `TestPayloadCheckQuery`, `TestPayloadExplanations`,
//! `TestPayloadScoreQuery`, `TestPayloadSpanPositions`, `TestPayloadSpans`
//! and `TestPayloadTermQuery`.
//!
//! The test helper `PayloadHelper` lives in the shared `queries_support`
//! module.

// The ports keep Java's indexed loops and counters line by line.
#![allow(clippy::needless_range_loop)]

#[macro_use]
mod queries_support;

mod payloads {
    pub mod test_payload_check_query;
    pub mod test_payload_explanations;
    pub mod test_payload_score_query;
    pub mod test_payload_span_positions;
    pub mod test_payload_spans;
    pub mod test_payload_term_query;
}

use queries_support::core_support::run;

#[test]
fn test_payload_check_query() {
    run(payloads::test_payload_check_query::class_def);
}

#[test]
fn test_payload_explanations() {
    run(payloads::test_payload_explanations::class_def);
}

#[test]
fn test_payload_score_query() {
    run(payloads::test_payload_score_query::class_def);
}

#[test]
fn test_payload_span_positions() {
    run(payloads::test_payload_span_positions::class_def);
}

#[test]
fn test_payload_spans() {
    run(payloads::test_payload_spans::class_def);
}

#[test]
fn test_payload_term_query() {
    run(payloads::test_payload_term_query::class_def);
}
