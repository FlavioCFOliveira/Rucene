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
//! `lucene/queryparser/src/test/org/apache/lucene/queryparser/{classic,complexPhrase,ext}/`
//! that rmp task #484 lists, each run as a suite by the ported runner:
//! `TestComplexPhraseQuery`, `TestExtendableQueryParser`, `TestExtensions`,
//! `TestMultiAnalyzer`, `TestMultiFieldQueryParser`,
//! `TestMultiPhraseQueryParsing`, `TestQueryParser`; the test helpers
//! `ExtensionStub` ([`ext::extension_stub`]) and `QueryParserTestBase`
//! (`qp_support::query_parser_test_base`, shared with the flexible tests).

// The ports keep Java's indexed loops line by line.
#![allow(clippy::needless_range_loop)]

mod qp_support;

mod classic {
    pub mod test_multi_analyzer;
    pub mod test_multi_field_query_parser;
    pub mod test_multi_phrase_query_parsing;
    pub mod test_query_parser;
}

mod complex_phrase {
    pub mod test_complex_phrase_query;
}

mod ext {
    pub mod extension_stub;
    pub mod test_extendable_query_parser;
    pub mod test_extensions;
}

use qp_support::core_support::run;

#[test]
fn test_complex_phrase_query() {
    run(complex_phrase::test_complex_phrase_query::class_def);
}

#[test]
fn test_extendable_query_parser() {
    run(ext::test_extendable_query_parser::class_def);
}

#[test]
fn test_extensions() {
    run(ext::test_extensions::class_def);
}

#[test]
fn test_multi_analyzer() {
    run(classic::test_multi_analyzer::class_def);
}

#[test]
fn test_multi_field_query_parser() {
    run(classic::test_multi_field_query_parser::class_def);
}

#[test]
fn test_multi_phrase_query_parsing() {
    run(classic::test_multi_phrase_query_parsing::class_def);
}

#[test]
fn test_query_parser() {
    run(classic::test_query_parser::class_def);
}
