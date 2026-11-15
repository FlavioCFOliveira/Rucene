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
//! `lucene/queryparser/src/test/org/apache/lucene/queryparser/xml/` that
//! rmp task #493 lists, each run as a suite by the ported runner:
//! `TestCoreParser`, `TestCorePlusExtensionsParser`,
//! `TestCorePlusQueriesParser`; the test helper `CoreParserTestIndexData`.
//! The test resources are byte-identical copies under
//! `tests/resources/org/apache/lucene/queryparser/xml/`.

// The ports keep Java's indexed loops line by line.
#![allow(clippy::needless_range_loop)]

mod qp_support;

mod xml {
    pub mod core_parser_test_index_data;
    pub mod test_core_parser;
}

use qp_support::core_support::run;

#[test]
fn test_core_parser() {
    run(xml::test_core_parser::class_def::<0>);
}

#[test]
fn test_core_plus_queries_parser() {
    run(xml::test_core_parser::class_def::<1>);
}

#[test]
fn test_core_plus_extensions_parser() {
    run(xml::test_core_parser::class_def::<2>);
}
