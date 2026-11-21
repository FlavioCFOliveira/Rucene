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

/// Regression test of rmp #497: Java's `MinimumMatchesIterator.getQuery()`
/// returns `null` (`MinimumShouldMatchIntervalsSource.java:406-408`), and the
/// core `MatchesIterator::get_query` answers `None` for it. Before the fix the
/// port panicked there, because the core trait returned a query.
#[test]
fn minimum_should_match_matches_iterator_has_no_query() {
    use std::sync::Arc;

    use rucene::codecs::{register_codec, Lucene104Codec};
    use rucene::document::{Document, Store, TextField};
    use rucene::index::directory_reader;
    use rucene::index::index_writer::IndexWriter;
    use rucene::index::index_writer_config::IndexWriterConfig;
    use rucene::store::{ByteBuffersDirectory, Directory};
    use rucene::util::BytesRef;
    use rucene_queries::intervals::intervals::Intervals;

    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
    let mut doc = Document::new();
    doc.add(TextField::new("field1", "pease porridge hot".to_string(), Store::NO).unwrap());
    writer.add_document(&doc).unwrap();
    writer.commit().unwrap();
    writer.close().unwrap();

    let reader = directory_reader::open(dir).unwrap();
    let leaves = reader.leaves().unwrap();
    // `Intervals.atLeast(2, term("pease"), term("porridge"), term("hot"))`: a
    // `MinimumShouldMatchIntervalsSource`, whose matches are a
    // `MinimumMatchesIterator`.
    let source = Intervals::at_least(
        2,
        ["pease", "porridge", "hot"]
            .iter()
            .map(|t| Intervals::term(BytesRef::new(t.as_bytes().to_vec())))
            .collect(),
    );
    let mut mi = source
        .matches("field1", &leaves[0], 0)
        .unwrap()
        .expect("the document matches");
    assert!(mi.next().unwrap(), "a first match");
    assert!(
        mi.get_query().is_none(),
        "MinimumMatchesIterator.getQuery() is null in Java"
    );
}
