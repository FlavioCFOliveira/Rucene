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
//! `lucene/queries/src/test/org/apache/lucene/queries/intervals/` that rmp
//! task #482 lists, each run as a suite by the ported runner:
//! `TestComplexMatches`, `TestDisjunctionRewrites`, `TestIntervalBuilder`,
//! `TestIntervalQuery`, `TestIntervals`, `TestPayloadFilteredInterval` and
//! `TestSimplifications`.
//!
//! The test helper of the package, `OneTimeIntervalSource`, is
//! [`intervals::one_time_interval_source`].

// The ports keep Java's indexed loops and counters line by line.
#![allow(clippy::needless_range_loop)]

#[macro_use]
mod queries_support;

mod intervals {
    pub mod one_time_interval_source;
    pub mod support;
    pub mod test_complex_matches;
    pub mod test_disjunction_rewrites;
    pub mod test_interval_builder;
    pub mod test_interval_query;
    pub mod test_intervals;
    pub mod test_payload_filtered_interval;
    pub mod test_simplifications;
}

use queries_support::core_support::run;

#[test]
fn test_complex_matches() {
    run(intervals::test_complex_matches::class_def);
}

#[test]
fn test_disjunction_rewrites() {
    run(intervals::test_disjunction_rewrites::class_def);
}

#[test]
fn test_interval_builder() {
    run(intervals::test_interval_builder::class_def);
}

#[test]
fn test_interval_query() {
    run(intervals::test_interval_query::class_def);
}

#[test]
fn test_intervals() {
    run(intervals::test_intervals::class_def);
}

#[test]
fn test_payload_filtered_interval() {
    run(intervals::test_payload_filtered_interval::class_def);
}

#[test]
fn test_simplifications() {
    run(intervals::test_simplifications::class_def);
}
