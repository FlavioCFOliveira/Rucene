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

//! Port of `TestSpanQueryParser.java`: how the new query parser can be
//! used to build `SpanQuery` objects from "field:term" pairs, optionally
//! joined by `OR`, every field converted to a unique field (see the Java
//! class's documentation).

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::search::Query;
use rucene_queries::spans::span_or_query::SpanOrQuery;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_queries::spans::span_term_query::SpanTermQuery;
use rucene_queryparser::flexible::core::config::query_config_handler::QueryConfigHandler;
use rucene_queryparser::flexible::core::parser::syntax_parser::SyntaxParser;
use rucene_queryparser::flexible::core::processors::query_node_processor::QueryNodeProcessor;
use rucene_queryparser::flexible::core::processors::query_node_processor_pipeline::QueryNodeProcessorPipeline;
use rucene_queryparser::flexible::standard::parser::standard_syntax_parser::StandardSyntaxParser;
use rucene_queryparser::flexible::standard::processors::wildcard_query_node_processor::WildcardQueryNodeProcessor;
use rucene_queryparser::jdk::char_sequence::CharSequence;

use super::spans_query_config_handler::SpansQueryConfigHandler;
use super::spans_query_tree_builder::SpansQueryTreeBuilder;
use super::spans_validator_query_node_processor::SpansValidatorQueryNodeProcessor;
use super::unique_field_query_node_processor::UniqueFieldQueryNodeProcessor;
use crate::qp_support::search_support::{fixture_class, ftest, set_up};
use crate::qp_support::{assert_str_eq, expect_throws, is, jdk_default_hooks, to_s};

const QUERY_NODE_EXCEPTION: &str = "org.apache.lucene.queryparser.flexible.core.QueryNodeException";

/// The instance of `TestSpanQueryParser`.
pub struct TestSpanQueryParser {
    span_processor_pipeline: Option<Arc<QueryNodeProcessorPipeline>>,
    span_query_config_handler: Option<Arc<QueryConfigHandler>>,
    spans_query_tree_builder: Option<SpansQueryTreeBuilder>,
    /// `private SyntaxParser queryParser = new StandardSyntaxParser()`.
    query_parser: Arc<dyn SyntaxParser>,
}

impl Default for TestSpanQueryParser {
    fn default() -> Self {
        // empty constructor
        TestSpanQueryParser {
            span_processor_pipeline: None,
            span_query_config_handler: None,
            spans_query_tree_builder: None,
            query_parser: Arc::new(Mutex::new(StandardSyntaxParser::new())),
        }
    }
}

/// `TestSpanQueryParser`.
pub fn class_def() -> Arc<Class> {
    let mut methods: Vec<Method> = jdk_default_hooks();
    methods.extend([
        set_up(test_set_up),
        ftest("testTermSpans", test_term_spans),
        ftest("testUniqueField", test_unique_field),
        ftest("testOrSpans", test_or_spans),
        ftest("testQueryValidator", test_query_validator),
    ]);
    fixture_class::<TestSpanQueryParser>(
        "org.apache.lucene.queryparser.flexible.spans.TestSpanQueryParser",
        methods,
    )
}

/// `setUp()`.
fn test_set_up(t: &mut TestSpanQueryParser) -> Result<()> {
    let span_processor_pipeline = Arc::new(QueryNodeProcessorPipeline::new());
    let span_query_config_handler = SpansQueryConfigHandler::new();
    let spans_query_tree_builder = SpansQueryTreeBuilder::new();

    // set up the processor pipeline
    span_processor_pipeline.set_query_config_handler(Some(Arc::clone(&span_query_config_handler)));
    span_processor_pipeline.add(Arc::new(WildcardQueryNodeProcessor::new()));
    span_processor_pipeline.add(Arc::new(SpansValidatorQueryNodeProcessor::new()));
    span_processor_pipeline.add(Arc::new(UniqueFieldQueryNodeProcessor::new()));

    t.span_processor_pipeline = Some(span_processor_pipeline);
    t.span_query_config_handler = Some(span_query_config_handler);
    t.spans_query_tree_builder = Some(spans_query_tree_builder);
    Ok(())
}

impl TestSpanQueryParser {
    /// `getSpanQuery(CharSequence query)`.
    fn get_span_query(&self, query: &str) -> Result<Arc<dyn Query>> {
        self.get_span_query_unique("", query)
    }

    /// `getSpanQuery(String uniqueField, CharSequence query)`.
    fn get_span_query_unique(&self, unique_field: &str, query: &str) -> Result<Arc<dyn Query>> {
        let config = self.span_query_config_handler.as_ref().expect("setUp ran");
        config.set(
            SpansQueryConfigHandler::UNIQUE_FIELD,
            Some(unique_field.to_string()),
        );
        let query_tree = self.query_parser.parse(
            &CharSequence::from(query),
            Some(&CharSequence::from("defaultField")),
        )?;
        let query_tree = self
            .span_processor_pipeline
            .as_ref()
            .expect("setUp ran")
            .process(query_tree)?;
        let span: Arc<dyn SpanQuery> = self
            .spans_query_tree_builder
            .as_ref()
            .expect("setUp ran")
            .build_span(&query_tree)?
            .unwrap_or_else(|| {
                randomizedtesting::jdk::exception::throw_without_message(
                    "java.lang.NullPointerException",
                )
            });
        Ok(span)
    }
}

fn test_term_spans(t: &mut TestSpanQueryParser) -> Result<()> {
    assert_str_eq(&to_s(&*t.get_span_query("field:term")?), "term");
    assert_str_eq(&to_s(&*t.get_span_query("term")?), "term");

    Assert::assert_true(is::<SpanTermQuery>(&*t.get_span_query("field:term")?));
    Assert::assert_true(is::<SpanTermQuery>(&*t.get_span_query("term")?));
    Ok(())
}

fn test_unique_field(t: &mut TestSpanQueryParser) -> Result<()> {
    assert_str_eq(
        &to_s(&*t.get_span_query_unique("field", "term")?),
        "field:term",
    );
    assert_str_eq(
        &to_s(&*t.get_span_query_unique("field", "field:term")?),
        "field:term",
    );
    assert_str_eq(
        &to_s(&*t.get_span_query_unique("field", "anotherField:term")?),
        "field:term",
    );
    Ok(())
}

fn test_or_spans(t: &mut TestSpanQueryParser) -> Result<()> {
    assert_str_eq(
        &to_s(&*t.get_span_query("term1 term2")?),
        "spanOr([term1, term2])",
    );
    assert_str_eq(
        &to_s(&*t.get_span_query("term1 OR term2")?),
        "spanOr([term1, term2])",
    );

    Assert::assert_true(is::<SpanOrQuery>(&*t.get_span_query("term1 term2")?));
    Assert::assert_true(is::<SpanOrQuery>(&*t.get_span_query("term1 term2")?));
    Ok(())
}

fn test_query_validator(t: &mut TestSpanQueryParser) -> Result<()> {
    // wildcard queries should not be supported
    expect_throws(QUERY_NODE_EXCEPTION, || t.get_span_query("term*"));
    // range queries should not be supported
    expect_throws(QUERY_NODE_EXCEPTION, || t.get_span_query("[a TO z]"));
    // boost queries should not be supported
    expect_throws(QUERY_NODE_EXCEPTION, || t.get_span_query("a~0.5"));
    // fuzzy queries should not be supported
    expect_throws(QUERY_NODE_EXCEPTION, || t.get_span_query("a^0.5"));
    // quoted queries should not be supported
    expect_throws(QUERY_NODE_EXCEPTION, || t.get_span_query("\"a b\""));
    // parenthesized queries should not be supported
    expect_throws(QUERY_NODE_EXCEPTION, || t.get_span_query("(a b)"));
    // AND queries should not be supported
    expect_throws(QUERY_NODE_EXCEPTION, || t.get_span_query("a AND b"));
    Ok(())
}
