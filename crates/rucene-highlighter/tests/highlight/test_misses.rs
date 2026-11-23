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

//! Port of `TestMisses.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::Analyzer;
use rucene::error::Result;
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query::Builder as BooleanQueryBuilder;
use rucene::search::phrase_query::PhraseQuery;
use rucene::search::query::Query;
use rucene::search::TermQuery;
use rucene_highlighter::highlight::highlighter::Highlighter;
use rucene_highlighter::highlight::query_scorer::QueryScorer;
use rucene_highlighter::highlight::simple_html_formatter::SimpleHTMLFormatter;
use rucene_queries::spans::span_near_query::SpanNearQuery;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_queries::spans::span_term_query::SpanTermQuery;
use rucene_test_framework::analysis::mock_tokenizer::WHITESPACE;
use rucene_test_framework::analysis::MockAnalyzer;

use crate::highlighter_support::core_support::{class, random, test};
use crate::highlighter_support::search_support::term;

/// `TestMisses`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.highlight.TestMisses",
        vec![
            test("testTermQuery", test_term_query),
            test("testBooleanQuery", test_boolean_query),
            test("testPhraseQuery", test_phrase_query),
            test("testSpanNearQuery", test_span_near_query),
        ],
    )
}

/// The common body: `try (Analyzer analyzer = new MockAnalyzer(random(),
/// MockTokenizer.WHITESPACE, false)) { ... }` over `query`.
fn check(query: Arc<dyn Query>, expected: &str) -> Result<()> {
    let analyzer = MockAnalyzer::with_automaton(&*random(), Arc::clone(&WHITESPACE), false);
    let mut highlighter = Highlighter::with_formatter(
        Box::new(SimpleHTMLFormatter::new()),
        Box::new(QueryScorer::new(query)),
    );
    Assert::assert_equals_str(
        None,
        Some(expected),
        highlighter
            .get_best_fragment(&analyzer, "test", "this is a foo bar example")?
            .as_deref(),
    );
    Assert::assert_true(
        highlighter
            .get_best_fragment(&analyzer, "test", "this does not match")?
            .is_none(),
    );
    analyzer.close();
    Ok(())
}

fn test_term_query() -> Result<()> {
    let query: Arc<dyn Query> = Arc::new(TermQuery::new(term("test", "foo")));
    check(query, "this is a <B>foo</B> bar example")
}

fn test_boolean_query() -> Result<()> {
    let mut query = BooleanQueryBuilder::new();
    query.add(Arc::new(TermQuery::new(term("test", "foo"))), Occur::MUST)?;
    query.add(Arc::new(TermQuery::new(term("test", "bar"))), Occur::MUST)?;
    check(
        Arc::new(query.build()),
        "this is a <B>foo</B> <B>bar</B> example",
    )
}

fn test_phrase_query() -> Result<()> {
    let query: Arc<dyn Query> = Arc::new(PhraseQuery::with_slop(0, "test", &["foo", "bar"])?);
    check(query, "this is a <B>foo</B> <B>bar</B> example")
}

fn test_span_near_query() -> Result<()> {
    let clauses: Vec<Arc<dyn SpanQuery>> = vec![
        Arc::new(SpanTermQuery::new(term("test", "foo"))),
        Arc::new(SpanTermQuery::new(term("test", "bar"))),
    ];
    let query: Arc<dyn Query> = Arc::new(SpanNearQuery::new(clauses, 0, true)?);
    check(query, "this is a <B>foo</B> <B>bar</B> example")
}
