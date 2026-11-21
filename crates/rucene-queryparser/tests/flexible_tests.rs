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
//! `lucene/queryparser/src/test/org/apache/lucene/queryparser/flexible/`
//! that rmp tasks #486, #487 and #491 list, each run as a suite by the
//! ported runner: `TestNLS` (with `MessagesTestBundle`), `TestQueryNode`,
//! `TestQueryTreeBuilder`, `TestUnescapedCharSequence`,
//! `TestMultiAnalyzerQPHelper`, `TestMultiFieldQPHelper`,
//! `TestPointQueryParser`, `TestPrecedenceQueryParser`, `TestQPHelper`,
//! `TestSpanQueryParser`, `TestSpanQueryParserSimpleSample`,
//! `TestStandardQP`, `TestStandardQPEnhancements`, and the test-only
//! package `flexible.spans` (`SpanOrQueryNodeBuilder`,
//! `SpanTermQueryNodeBuilder`, `SpansQueryConfigHandler`,
//! `SpansQueryTreeBuilder`, `SpansValidatorQueryNodeProcessor`,
//! `UniqueFieldAttribute`, `UniqueFieldAttributeImpl`,
//! `UniqueFieldQueryNodeProcessor`).

// The ports keep Java's indexed loops line by line.
#![allow(clippy::needless_range_loop)]

mod qp_support;

mod flexible {
    pub mod messages {
        pub mod messages_test_bundle;
        pub mod test_nls;
    }

    pub mod core {
        pub mod test_query_node;
        pub mod test_query_tree_builder;
        pub mod test_unescaped_char_sequence;
    }

    pub mod standard {
        pub mod test_multi_analyzer_qp_helper;
        pub mod test_multi_field_qp_helper;
        pub mod test_point_query_parser;
        pub mod test_qp_helper;
        pub mod test_standard_qp;
        pub mod test_standard_qp_enhancements;
    }

    pub mod precedence {
        pub mod test_precedence_query_parser;
    }

    pub mod spans {
        pub mod span_or_query_node_builder;
        pub mod span_query_cast;
        pub mod span_term_query_node_builder;
        pub mod spans_query_config_handler;
        pub mod spans_query_tree_builder;
        pub mod spans_validator_query_node_processor;
        pub mod test_span_query_parser;
        pub mod test_span_query_parser_simple_sample;
        // Ported with the package (rmp #491); no Lucene test uses them.
        #[allow(dead_code)]
        pub mod unique_field_attribute;
        #[allow(dead_code)]
        pub mod unique_field_attribute_impl;
        pub mod unique_field_query_node_processor;
    }
}

use flexible::{core, messages, precedence, spans, standard};
use qp_support::core_support::run;

#[test]
fn test_nls() {
    run(messages::test_nls::class_def);
}

#[test]
fn test_query_node() {
    run(core::test_query_node::class_def);
}

#[test]
fn test_query_tree_builder() {
    run(core::test_query_tree_builder::class_def);
}

#[test]
fn test_unescaped_char_sequence() {
    run(core::test_unescaped_char_sequence::class_def);
}

#[test]
fn test_multi_analyzer_qp_helper() {
    run(standard::test_multi_analyzer_qp_helper::class_def);
}

#[test]
fn test_multi_field_qp_helper() {
    run(standard::test_multi_field_qp_helper::class_def);
}

#[test]
fn test_point_query_parser() {
    run(standard::test_point_query_parser::class_def);
}

#[test]
fn test_qp_helper() {
    run(standard::test_qp_helper::class_def);
}

#[test]
fn test_standard_qp() {
    run(standard::test_standard_qp::class_def);
}

#[test]
fn test_standard_qp_enhancements() {
    run(standard::test_standard_qp_enhancements::class_def);
}

#[test]
fn test_precedence_query_parser() {
    run(precedence::test_precedence_query_parser::class_def);
}

#[test]
fn test_span_query_parser() {
    run(spans::test_span_query_parser::class_def);
}

#[test]
fn test_span_query_parser_simple_sample() {
    run(spans::test_span_query_parser_simple_sample::class_def);
}
