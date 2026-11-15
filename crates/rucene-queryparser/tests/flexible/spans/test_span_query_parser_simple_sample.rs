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

//! Port of `TestSpanQueryParserSimpleSample.java` (see
//! `TestSpanQueryParser` for a more advanced example).

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::search::Query;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_queries::spans::span_term_query::SpanTermQuery;
use rucene_queryparser::flexible::core::parser::syntax_parser::SyntaxParser;
use rucene_queryparser::flexible::core::processors::query_node_processor::QueryNodeProcessor;
use rucene_queryparser::flexible::core::processors::query_node_processor_pipeline::QueryNodeProcessorPipeline;
use rucene_queryparser::flexible::standard::parser::standard_syntax_parser::StandardSyntaxParser;
use rucene_queryparser::jdk::char_sequence::CharSequence;

use super::spans_query_config_handler::SpansQueryConfigHandler;
use super::spans_query_tree_builder::SpansQueryTreeBuilder;
use super::spans_validator_query_node_processor::SpansValidatorQueryNodeProcessor;
use super::unique_field_query_node_processor::UniqueFieldQueryNodeProcessor;
use crate::qp_support::core_support::{class, test, verbose};
use crate::qp_support::{assert_str_eq, is, jdk_default_hooks, to_s};

/// `TestSpanQueryParserSimpleSample`.
pub fn class_def() -> Arc<Class> {
    let mut methods = jdk_default_hooks();
    methods.push(test("testBasicDemo", test_basic_demo));
    class(
        "org.apache.lucene.queryparser.flexible.spans.TestSpanQueryParserSimpleSample",
        methods,
    )
}

fn test_basic_demo() -> Result<()> {
    let query_parser: Arc<dyn SyntaxParser> = Arc::new(Mutex::new(StandardSyntaxParser::new()));

    // convert the CharSequence into a QueryNode tree
    let query_tree = query_parser.parse(&CharSequence::from("body:text"), None)?;

    // create a config handler with a attribute used in
    // UniqueFieldQueryNodeProcessor
    let span_query_config_handler = SpansQueryConfigHandler::new();
    span_query_config_handler.set(
        SpansQueryConfigHandler::UNIQUE_FIELD,
        Some("index".to_string()),
    );

    // set up the processor pipeline with the ConfigHandler
    // and create the pipeline for this simple demo
    let span_processor_pipeline =
        QueryNodeProcessorPipeline::with_query_config_handler(Some(span_query_config_handler));
    // @see SpansValidatorQueryNodeProcessor
    span_processor_pipeline.add(Arc::new(SpansValidatorQueryNodeProcessor::new()));
    // @see UniqueFieldQueryNodeProcessor
    span_processor_pipeline.add(Arc::new(UniqueFieldQueryNodeProcessor::new()));

    // print to show out the QueryNode tree before being processed
    if verbose() {
        println!("{}", query_tree.to_java_string()?);
    }

    // Process the QueryTree using our new Processors
    let query_tree = span_processor_pipeline.process(query_tree)?;

    // print to show out the QueryNode tree after being processed
    if verbose() {
        println!("{}", query_tree.to_java_string()?);
    }

    // create a instance off the Builder
    let spans_query_tree_builder = SpansQueryTreeBuilder::new();

    // convert QueryNode tree to span query Objects
    let spanquery: Arc<dyn SpanQuery> = spans_query_tree_builder
        .build_span(&query_tree)?
        .unwrap_or_else(|| {
            randomizedtesting::jdk::exception::throw_without_message(
                "java.lang.NullPointerException",
            )
        });
    let spanquery: Arc<dyn Query> = spanquery;

    Assert::assert_true(is::<SpanTermQuery>(&*spanquery));
    assert_str_eq(&to_s(&*spanquery), "index:text");
    Ok(())
}
