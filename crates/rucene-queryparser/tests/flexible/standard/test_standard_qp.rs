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

//! Port of `TestStandardQP.java`: `QueryParserTestBase` run against the
//! `StandardQueryParser`.

use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::Assert;
use rucene::analysis::Analyzer;
use rucene::document::Resolution;
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query::Builder as BooleanQueryBuilder;
use rucene::search::wildcard_query::WildcardQuery;
use rucene::search::Query;
use rucene_queryparser::flexible::standard::config::standard_query_config_handler::Operator;
use rucene_queryparser::flexible::standard::standard_query_parser::StandardQueryParser;
use rucene_queryparser::jdk::char_sequence::CharSequence;
use rucene_test_framework::analysis::mock_tokenizer::{SIMPLE, WHITESPACE};
use rucene_test_framework::analysis::tokenizer_components;

use crate::qp_support::query_parser_test_base::{
    analyzer1, assert_query_equals, assert_query_equals_config, base_methods, mock_analyzer,
    mock_analyzer_default, mock_tokenizer, qp_analyzer, test_auto_generate_phrase_queries_on,
    test_qpa, FixtureTest, FnAnalyzer, QueryParserTestBase, QueryParserTestBaseState,
};
use crate::qp_support::search_support::{fixture_class, ftest, term, tq};
use crate::qp_support::{assert_query_eq, expect_throws};

const QUERY_NODE_EXCEPTION: &str = "org.apache.lucene.queryparser.flexible.core.QueryNodeException";

/// The instance of `TestStandardQP`.
#[derive(Default)]
pub struct TestStandardQP {
    base: QueryParserTestBaseState,
}

impl TestStandardQP {
    /// `getParser(Analyzer a)`.
    pub fn get_parser(&mut self, a: Option<Arc<dyn Analyzer>>) -> Result<StandardQueryParser> {
        let a = a.unwrap_or_else(|| mock_analyzer(&SIMPLE, true));
        let mut qp = StandardQueryParser::with_analyzer(Some(a));
        qp.set_default_operator(Some(Operator::Or))?;
        Ok(qp)
    }

    /// `parse(String query, StandardQueryParser qp)`.
    pub fn parse(&mut self, query: &str, qp: &StandardQueryParser) -> Result<Arc<dyn Query>> {
        let field = self.get_default_field();
        non_null(qp.parse(query, Some(&field))?)
    }
}

/// A `Query` the Java code dereferences: `null` throws
/// `NullPointerException` (the standard builders never build `null`).
fn non_null(q: Option<Arc<dyn Query>>) -> Result<Arc<dyn Query>> {
    q.ok_or_else(LuceneError::null_pointer)
}

impl QueryParserTestBase for TestStandardQP {
    type Config = StandardQueryParser;

    fn base(&self) -> &QueryParserTestBaseState {
        &self.base
    }

    fn base_mut(&mut self) -> &mut QueryParserTestBaseState {
        &mut self.base
    }

    fn get_parser_config(&mut self, a: Option<Arc<dyn Analyzer>>) -> Result<Box<Self::Config>> {
        Ok(Box::new(self.get_parser(a)?))
    }

    fn get_query_config(
        &mut self,
        query: &str,
        cqp_c: &mut Self::Config,
    ) -> Result<Arc<dyn Query>> {
        // `assert cqpC != null` and `assert cqpC instanceof
        // StandardQueryParser`: the type guarantees both.
        self.parse(query, cqp_c)
    }

    fn get_query_analyzer(
        &mut self,
        query: &str,
        a: Option<Arc<dyn Analyzer>>,
    ) -> Result<Arc<dyn Query>> {
        let qp = self.get_parser(a)?;
        self.parse(query, &qp)
    }

    fn is_query_parser_exception(&self, exception: &Throwable) -> bool {
        exception.is_instance(QUERY_NODE_EXCEPTION)
    }

    fn set_default_operator_or(&mut self, cqp_c: &mut Self::Config) -> Result<()> {
        cqp_c.set_default_operator(Some(Operator::Or))
    }

    fn set_default_operator_and(&mut self, cqp_c: &mut Self::Config) -> Result<()> {
        cqp_c.set_default_operator(Some(Operator::And))
    }

    fn set_auto_generate_phrase_queries(
        &mut self,
        _cqp_c: &mut Self::Config,
        _value: bool,
    ) -> Result<()> {
        // `throw new UnsupportedOperationException()`
        Err(LuceneError::from(LuceneErrorKind::Unchecked {
            class: "java.lang.UnsupportedOperationException",
            message: None,
            cause: None,
        }))
    }

    fn set_date_resolution(
        &mut self,
        cqp_c: &mut Self::Config,
        field: &str,
        value: Resolution,
    ) -> Result<()> {
        // `qp.getDateResolutionMap().put(field, value)`
        let map = cqp_c
            .get_date_resolution_map()?
            .ok_or_else(LuceneError::null_pointer)?;
        map.write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(CharSequence::from(field), value);
        Ok(())
    }

    fn is_test_query_parser(&self) -> bool {
        false
    }

    fn set_split_on_whitespace_if_classic(&mut self, _qp: &mut Self::Config) -> Result<()> {
        Ok(())
    }
}

/// The methods of `TestStandardQP` (with those it inherits).
pub fn methods() -> Vec<Method> {
    let own: Vec<(&str, FixtureTest<TestStandardQP>)> = vec![
        ("testOperatorVsWhitespace", test_operator_vs_whitespace),
        ("testRangeWithPhrase", test_range_with_phrase),
        (
            "testEscapedVsQuestionMarkAsWildcard",
            test_escaped_vs_question_mark_as_wildcard,
        ),
        ("testEscapedWildcard", test_escaped_wildcard),
        (
            "testAutoGeneratePhraseQueriesOn",
            test_auto_generate_phrase_queries_on_override,
        ),
        ("testStarParsing", test_star_parsing),
        ("testDefaultOperator", test_default_operator),
        ("testNewFieldQuery", test_new_field_query),
        ("testQPA", test_qpa_override),
    ];
    let overridden: Vec<&str> = own.iter().map(|(n, _)| *n).collect();
    let mut m = base_methods::<TestStandardQP>(&overridden);
    for (name, body) in own {
        m.push(ftest(name, body));
    }
    m
}

/// `TestStandardQP`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<TestStandardQP>(
        "org.apache.lucene.queryparser.flexible.standard.TestStandardQP",
        methods(),
    )
}

fn test_operator_vs_whitespace(t: &mut TestStandardQP) -> Result<()> {
    // LUCENE-2566 is not implemented for StandardQueryParser
    // TODO implement LUCENE-2566 and remove this (override)method
    let a = FnAnalyzer::new(
        "org.apache.lucene.queryparser.flexible.standard.TestStandardQP$1",
        |_| Ok(tokenizer_components(mock_tokenizer(&WHITESPACE, false)?)),
    )
    .arc();
    assert_query_equals(t, "a - b", Some(Arc::clone(&a)), "a -b")?;
    assert_query_equals(t, "a + b", Some(Arc::clone(&a)), "a +b")?;
    assert_query_equals(t, "a ! b", Some(a), "a -b")?;
    Ok(())
}

fn test_range_with_phrase(t: &mut TestStandardQP) -> Result<()> {
    // StandardSyntaxParser does not differentiate between a term and a
    // one-term-phrase in a range query.
    // Is this an issue? Should StandardSyntaxParser mark the text as
    // wasEscaped=true ?
    assert_query_equals(t, "[\\* TO \"*\"]", None, "[\\* TO *]")
}

fn test_escaped_vs_question_mark_as_wildcard(t: &mut TestStandardQP) -> Result<()> {
    let a = mock_analyzer(&WHITESPACE, false);
    assert_query_equals(t, "a:b\\-?c", Some(Arc::clone(&a)), "a:b-?c")?;
    assert_query_equals(t, "a:b\\+?c", Some(Arc::clone(&a)), "a:b+?c")?;
    assert_query_equals(t, "a:b\\:?c", Some(Arc::clone(&a)), "a:b:?c")?;

    assert_query_equals(t, "a:b\\\\?c", Some(a), "a:b\\?c")?;
    Ok(())
}

fn test_escaped_wildcard(t: &mut TestStandardQP) -> Result<()> {
    let mut qp = t.get_parser_config(Some(mock_analyzer(&WHITESPACE, false)))?;
    let q = WildcardQuery::new(term("field", "foo?ba?r"))?; // TODO not correct!!
    assert_query_eq(&q, &*t.get_query_config("foo\\?ba?r", &mut qp)?);
    Ok(())
}

fn test_auto_generate_phrase_queries_on_override(t: &mut TestStandardQP) -> Result<()> {
    expect_throws("java.lang.UnsupportedOperationException", || {
        let mut qp = t.get_parser(None)?;
        t.set_auto_generate_phrase_queries(&mut qp, true)?;
        test_auto_generate_phrase_queries_on(t)
    });
    Ok(())
}

fn test_star_parsing(_t: &mut TestStandardQP) -> Result<()> {
    Ok(())
}

fn test_default_operator(t: &mut TestStandardQP) -> Result<()> {
    let mut qp = t.get_parser(Some(mock_analyzer_default()))?;
    // make sure OR is the default:
    assert_operator(Operator::Or, qp.get_default_operator()?);
    t.set_default_operator_and(&mut qp)?;
    assert_operator(Operator::And, qp.get_default_operator()?);
    t.set_default_operator_or(&mut qp)?;
    assert_operator(Operator::Or, qp.get_default_operator()?);
    Ok(())
}

/// `assertEquals(expected, actual)` over two `Operator` constants (their
/// `toString()` is the constant's name).
pub fn assert_operator(expected: Operator, actual: Option<Operator>) {
    let name = |o: Option<Operator>| match o {
        None => "null".to_string(),
        Some(o) => format!("{o:?}").to_uppercase(),
    };
    Assert::assert_equals(name(Some(expected)), name(actual));
}

fn test_new_field_query(t: &mut TestStandardQP) -> Result<()> {
    /* ordinary behavior, synonyms form uncoordinated boolean query */
    let dumb = t.get_parser(Some(analyzer1()))?;
    let mut expanded = BooleanQueryBuilder::new();
    expanded.add(tq("field", "dogs"), Occur::SHOULD)?;
    expanded.add(tq("field", "dog"), Occur::SHOULD)?;
    let expanded = expanded.build();
    assert_query_eq(
        &expanded,
        &*non_null(dumb.parse("\"dogs\"", Some("field"))?)?,
    );
    /* even with the phrase operator the behavior is the same */
    assert_query_eq(&expanded, &*non_null(dumb.parse("dogs", Some("field"))?)?);

    /* custom behavior, the synonyms are expanded, unless you use quote operator */
    // TODO test something like "SmartQueryParser()"
    Ok(())
}

// TODO: Remove this specialization once the flexible standard parser gets
// multi-word synonym support
fn test_qpa_override(t: &mut TestStandardQP) -> Result<()> {
    test_qpa(t)?;

    assert_query_equals(
        t,
        "term phrase term",
        Some(qp_analyzer()),
        "term (phrase1 phrase2) term",
    )?;

    let mut cqpc = t.get_parser_config(Some(qp_analyzer()))?;
    t.set_default_operator_and(&mut cqpc)?;
    assert_query_equals_config(
        t,
        &mut cqpc,
        "field",
        "term phrase term",
        "+term +(+phrase1 +phrase2) +term",
    )?;
    Ok(())
}
