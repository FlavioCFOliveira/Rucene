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

//! Port of `TestStandardQPEnhancements.java`: interval sub-query support in
//! `StandardQueryParser`.
//!
//! Hamcrest's `assertThat(x, equalTo(y))`, `instanceOf` and `notNullValue`
//! are the JUnit assertions of the same conditions.

use std::sync::{Arc, RwLock};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::StringCharReader;
use rucene::error::{LuceneError, Result};
use rucene::search::boolean_query::BooleanQuery;
use rucene::search::Query;
use rucene_queryparser::charstream::fast_char_stream::FastCharStream;
use rucene_queryparser::flexible::core::nodes::query_node::class_cast;
use rucene_queryparser::flexible::standard::common_query_parser_configuration::CommonQueryParserConfiguration;
use rucene_queryparser::flexible::standard::config::standard_query_config_handler::Operator;
use rucene_queryparser::flexible::standard::nodes::interval_query_node::{
    IntervalQueryNode, INTERVAL_QUERY_NODE_CLASS,
};
use rucene_queryparser::flexible::standard::parser::standard_syntax_parser::{
    StandardSyntaxParser, StandardSyntaxParserOps,
};
use rucene_queryparser::flexible::standard::standard_query_parser::StandardQueryParser;
use rucene_queryparser::jdk::char_sequence::CharSequence;
use rucene_test_framework::analysis::mock_tokenizer::WHITESPACE;
use rucene_test_framework::analysis::tokenizer_components;

use crate::qp_support::core_support::{class, random, test};
use crate::qp_support::query_parser_test_base::{mock_tokenizer, FnAnalyzer};
use crate::qp_support::{cast, jdk_default_hooks, to_s};

/// `protected static final String FLD_DEFAULT = "defaultField"`.
const FLD_DEFAULT: &str = "defaultField";
/// `protected static final String FLD_WHITESPACE = "whitespaceField"`.
const FLD_WHITESPACE: &str = "whitespaceField";

/// `TestStandardQPEnhancements`.
pub fn class_def() -> Arc<Class> {
    let mut methods = jdk_default_hooks();
    methods.extend([
        test("testMinShouldMatchOperator", test_min_should_match_operator),
        test("testAtLeast", test_at_least),
        test("testMaxWidth", test_max_width),
        test("testQuotedTerm", test_quoted_term),
        test("testMaxGaps", test_max_gaps),
        test("testOrdered", test_ordered),
        test("testUnordered", test_unordered),
        test("testOr", test_or),
        test("testWildcard", test_wildcard),
        test("testPhrase", test_phrase),
        test("testBefore", test_before),
        test("testAfter", test_after),
        test("testContaining", test_containing),
        test("testContainedBy", test_contained_by),
        test("testNotContaining", test_not_containing),
        test("testNotContainedBy", test_not_contained_by),
        test("testWithin", test_within),
        test("testNotWithin", test_not_within),
        test("testOverlapping", test_overlapping),
        test("testNonOverlapping", test_non_overlapping),
        test("testUnorderedNoOverlaps", test_unordered_no_overlaps),
        test("testExtend", test_extend),
        test("testFuzzy", test_fuzzy),
    ]);
    class(
        "org.apache.lucene.queryparser.flexible.standard.TestStandardQPEnhancements",
        methods,
    )
}

/// `getQueryParser()`.
fn get_query_parser() -> Result<StandardQueryParser> {
    let analyzer = FnAnalyzer::new(
        "org.apache.lucene.queryparser.flexible.standard.TestStandardQPEnhancements$1",
        |_| Ok(tokenizer_components(mock_tokenizer(&WHITESPACE, true)?)),
    )
    .arc();

    let mut qp = StandardQueryParser::with_analyzer(Some(analyzer));
    qp.set_default_operator(Some(Operator::And))?;
    qp.set_multi_fields(Some(Arc::new(RwLock::new(Vec::new()))))?;
    Ok(qp)
}

fn test_min_should_match_operator() -> Result<()> {
    let parsed = parsed_query(&format!(
        "({FLD_WHITESPACE}:foo OR {FLD_WHITESPACE}:bar OR {FLD_WHITESPACE}:baz)@2"
    ))?;

    Assert::assert_equals(
        2,
        cast::<BooleanQuery>(&*parsed, "org.apache.lucene.search.BooleanQuery")
            .get_minimum_number_should_match(),
    );
    Ok(())
}

fn test_at_least() -> Result<()> {
    check_interval_query_node("fn:atleast(3 FOO BAR baz)")
}

fn test_max_width() -> Result<()> {
    check_interval_query_node("fn:maxwidth(3 fn:atleast(2 foo bar baz))")
}

fn test_quoted_term() -> Result<()> {
    check_interval_query_node("fn:atleast(2 \"foo\" \"BAR baz\")")
}

fn test_max_gaps() -> Result<()> {
    check_interval_query_node("fn:maxgaps(2 fn:unordered(foo BAR baz))")
}

fn test_ordered() -> Result<()> {
    check_interval_query_node("fn:ordered(foo BAR baz)")
}

fn test_unordered() -> Result<()> {
    check_interval_query_node("fn:unordered(foo BAR baz)")
}

fn test_or() -> Result<()> {
    check_interval_query_node("fn:or(foo baz)")
}

fn test_wildcard() -> Result<()> {
    check_interval_query_node("fn:wildcard(foo*)")?;

    // Explicit maxExpansions.
    check_interval_query_node("fn:wildcard(foo* 128)")
}

fn test_phrase() -> Result<()> {
    check_interval_query_node("fn:phrase(abc def fn:or(baz boo))")
}

fn test_before() -> Result<()> {
    check_interval_query_node("fn:before(abc fn:ordered(foo bar))")
}

fn test_after() -> Result<()> {
    check_interval_query_node("fn:after(abc fn:ordered(foo bar))")
}

fn test_containing() -> Result<()> {
    check_interval_query_node("fn:containing(big small)")
}

fn test_contained_by() -> Result<()> {
    check_interval_query_node("fn:containedBy(small big)")
}

fn test_not_containing() -> Result<()> {
    check_interval_query_node("fn:notContaining(minuend subtrahend)")
}

fn test_not_contained_by() -> Result<()> {
    check_interval_query_node("fn:notContainedBy(small big)")
}

fn test_within() -> Result<()> {
    check_interval_query_node("fn:within(small 2 fn:ordered(big foo))")
}

fn test_not_within() -> Result<()> {
    check_interval_query_node("fn:notWithin(small 2 fn:ordered(big foo))")
}

fn test_overlapping() -> Result<()> {
    check_interval_query_node("fn:overlapping(fn:ordered(big foo) small)")
}

fn test_non_overlapping() -> Result<()> {
    check_interval_query_node("fn:nonOverlapping(fn:ordered(big foo) small)")
}

fn test_unordered_no_overlaps() -> Result<()> {
    check_interval_query_node("fn:unorderedNoOverlaps(fn:ordered(big foo) small)")
}

fn test_extend() -> Result<()> {
    check_interval_query_node("fn:extend(fn:ordered(big foo) 2 5)")
}

fn test_fuzzy() -> Result<()> {
    check_interval_query_node("fn:fuzzyTerm(dfe)")?;
    // Explicit maxEdits
    check_interval_query_node("fn:fuzzyTerm(dfe 2)")?;
    // Explicit maxExpansions
    check_interval_query_node("fn:fuzzyTerm(dfe 2 128)")
}

/// `checkIntervalQueryNode(String query)`.
fn check_interval_query_node(query: &str) -> Result<()> {
    // Check raw parser first.
    let mut syntax_parser = StandardSyntaxParser::with_char_stream(Box::new(FastCharStream::new(
        Box::new(StringCharReader::new(query)),
    )));
    let query_node = syntax_parser
        .top_level_query(Some(CharSequence::from(FLD_DEFAULT)))?
        .ok_or_else(LuceneError::null_pointer)?;
    Assert::assert_true_msg(
        &format!(
            "Expected: an instance of {}",
            INTERVAL_QUERY_NODE_CLASS.get_name()
        ),
        query_node.as_any().is::<IntervalQueryNode>(),
    );

    let mut query_parser = get_query_parser()?;
    let parsed_query: Option<Arc<dyn Query>> = if random().next_boolean() {
        query_parser.set_multi_fields(Some(Arc::new(RwLock::new(vec![Some(
            CharSequence::from(FLD_DEFAULT),
        )]))))?;
        query_parser.parse(query, None)?
    } else {
        query_parser.parse(query, Some(FLD_DEFAULT))?
    };
    Assert::assert_true_msg("Expected: not null", parsed_query.is_some());

    // Emit toString() for visual diagnostics.
    let interval_query_node = query_node
        .as_any()
        .downcast_ref::<IntervalQueryNode>()
        .ok_or_else(|| class_cast(query_node.as_ref(), &INTERVAL_QUERY_NODE_CLASS))?;
    interval_query_node.set_analyzer(query_parser.get_analyzer()?)?;
    print!(
        "query: {query}\n  node: {}\n  query: {}\n",
        query_node.to_java_string()?,
        parsed_query
            .as_deref()
            .map_or_else(|| "null".to_string(), to_s)
    );
    Ok(())
}

/// `parsed(String query)`.
#[allow(dead_code)]
fn parsed(query: &str) -> Result<String> {
    Ok(parsed_query(query)?.to_query_string("<no-default>"))
}

/// `parsedQuery(String query)`.
fn parsed_query(query: &str) -> Result<Arc<dyn Query>> {
    get_query_parser()?
        .parse(query, /* no default field. */ None)?
        .ok_or_else(LuceneError::null_pointer)
}
