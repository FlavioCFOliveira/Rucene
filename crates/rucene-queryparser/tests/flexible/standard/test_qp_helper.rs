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

//! Port of `TestQPHelper.java`: the `StandardQueryParser` against the
//! assertions of the classic `TestQueryParser` (this test is the
//! flexible-framework copy of it), with its own analyzers and the
//! `QPTestParser` subclass.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::{AfterClass, Assert, BeforeClass, Ignore};
use rucene::analysis::{
    Analyzer, CharReader, StringCharReader, TokenFilter, TokenFilterLogic, TokenStream, Tokenizer,
    TokenizerLogic,
};
use rucene::document::{Document, Resolution, Store};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query::{BooleanQuery, Builder as BooleanQueryBuilder};
use rucene::search::boost_query::BoostQuery;
use rucene::search::fuzzy_query::{self, FuzzyQuery};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::match_all_docs_query::MatchAllDocsQuery;
use rucene::search::match_no_docs_query::MatchNoDocsQuery;
use rucene::search::multi_phrase_query::MultiPhraseQuery;
use rucene::search::multi_term_query::{
    constant_score_blended_rewrite, scoring_boolean_rewrite, MultiTermQuery, RewriteMethod,
};
use rucene::search::phrase_query::PhraseQuery;
use rucene::search::prefix_query::PrefixQuery;
use rucene::search::regexp_query::RegexpQuery;
use rucene::search::term_query::TermQuery;
use rucene::search::term_range_query::TermRangeQuery;
use rucene::search::wildcard_query::WildcardQuery;
use rucene::search::Query;
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::attribute::AttributeSource;
use rucene::util::automaton::{
    Automata, Automaton, AutomatonProvider, CharacterRunAutomaton, RegExp,
    DEFAULT_DETERMINIZE_WORK_LIMIT,
};
use rucene_queryparser::charstream::fast_char_stream::FastCharStream;
use rucene_queryparser::flexible::core::messages::query_parser_messages::QueryParserMessages;
use rucene_queryparser::flexible::core::nodes::fuzzy_query_node::FUZZY_QUERY_NODE_CLASS;
use rucene_queryparser::flexible::core::nodes::query_node::{instance_of, QueryNodeRef};
use rucene_queryparser::flexible::core::processors::query_node_processor_impl::{
    QueryNodeProcessorImpl, QueryNodeProcessorImplOps,
};
use rucene_queryparser::flexible::core::query_node_exception::QueryNodeException;
use rucene_queryparser::flexible::messages::message_impl::MessageImpl;
use rucene_queryparser::flexible::standard::common_query_parser_configuration::CommonQueryParserConfiguration;
use rucene_queryparser::flexible::standard::config::standard_query_config_handler::Operator;
use rucene_queryparser::flexible::standard::nodes::wildcard_query_node::WILDCARD_QUERY_NODE_CLASS;
use rucene_queryparser::flexible::standard::parser::standard_syntax_parser::{
    StandardSyntaxParser, StandardSyntaxParserOps,
};
use rucene_queryparser::flexible::standard::processors::standard_query_node_processor_pipeline::StandardQueryNodeProcessorPipeline;
use rucene_queryparser::flexible::standard::query_parser_util::QueryParserUtil;
use rucene_queryparser::flexible::standard::standard_query_parser::StandardQueryParser;
use rucene_queryparser::jdk::calendar::{self, Calendar};
use rucene_queryparser::jdk::char_sequence::CharSequence;
use rucene_queryparser::jdk::locale::Locale;
use rucene_queryparser::jdk::time_zone::TimeZone;
use rucene_test_framework::analysis::attributes::{char_term_id, offset_id, pos_inc_id, Att};
use rucene_test_framework::analysis::mock_token_filter::ENGLISH_STOPSET;
use rucene_test_framework::analysis::mock_tokenizer::{SIMPLE, WHITESPACE};
use rucene_test_framework::analysis::{components, shared, sink, tokenizer_components};
use rucene_test_framework::util::LuceneTestCase;

use crate::qp_support::query_parser_test_base::{
    escape_date_string, get_date_millis, get_date_str, get_localized_date, mock_analyzer,
    mock_analyzer_default, mock_analyzer_filter, mock_tokenizer, simple_cjk_tokenizer, FixtureTest,
    FnAnalyzer,
};
use crate::qp_support::search_support::{
    fixture_class, ftest, set_up, static_hook, tear_down, term, tq,
};
use crate::qp_support::{assert_query_eq, cast, expect_throws, is, jdk_default_hooks, to_s};

const QUERY_NODE_EXCEPTION: &str = "org.apache.lucene.queryparser.flexible.core.QueryNodeException";
const BOOST_QUERY: &str = "org.apache.lucene.search.BoostQuery";

// ---------------------------------------------------------------------------
// Analyzers and the custom parser
// ---------------------------------------------------------------------------

/// `public static Analyzer qpAnalyzer`.
static QP_ANALYZER: Mutex<Option<Arc<dyn Analyzer>>> = Mutex::new(None);

/// `qpAnalyzer`.
fn qp_analyzer() -> Arc<dyn Analyzer> {
    let g = QP_ANALYZER.lock().unwrap_or_else(|e| e.into_inner());
    match g.as_ref() {
        Some(a) => Arc::clone(a),
        None => randomizedtesting::jdk::exception::throw_without_message(
            "java.lang.NullPointerException",
        ),
    }
}

/// `@BeforeClass beforeClass()`.
fn before_class() -> Result<()> {
    *QP_ANALYZER.lock().unwrap_or_else(|e| e.into_inner()) = Some(qp_test_analyzer());
    Ok(())
}

/// `@AfterClass afterClass()`.
fn after_class() -> Result<()> {
    *QP_ANALYZER.lock().unwrap_or_else(|e| e.into_inner()) = None;
    Ok(())
}

/// The logic of `TestQPHelper.QPTestFilter`: discards the token 'stop' and
/// expands the token 'phrase' into 'phrase1 phrase2'.
#[derive(Debug)]
pub struct QpTestFilterLogic {
    term_att: Att,
    offset_att: Att,
    in_phrase: bool,
    saved_start: i32,
    saved_end: i32,
}

impl TokenFilterLogic for QpTestFilterLogic {
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if self.in_phrase {
            self.in_phrase = false;
            source.clear_attributes();
            self.term_att.set_term("phrase2");
            self.offset_att
                .offset_mut()
                .set_offset(self.saved_start, self.saved_end);
            return Ok(true);
        }
        while input.increment_token()? {
            let term = self.term_att.term_string();
            if term == "phrase" {
                self.in_phrase = true;
                self.saved_start = self.offset_att.offset().start_offset();
                self.saved_end = self.offset_att.offset().end_offset();
                self.term_att.set_term("phrase1");
                self.offset_att
                    .offset_mut()
                    .set_offset(self.saved_start, self.saved_end);
                return Ok(true);
            } else if term != "stop" {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.in_phrase = false;
        self.saved_start = 0;
        self.saved_end = 0;
        Ok(())
    }
}

/// `new QPTestFilter(in)`.
fn qp_test_filter(mut input: Box<dyn TokenStream>) -> Result<TokenFilter<QpTestFilterLogic>> {
    let term_att = Att::add(input.attribute_source_mut(), char_term_id())?;
    let offset_att = Att::add(input.attribute_source_mut(), offset_id())?;
    Ok(TokenFilter::new(
        input,
        QpTestFilterLogic {
            term_att,
            offset_att,
            in_phrase: false,
            saved_start: 0,
            saved_end: 0,
        },
    ))
}

/// `new TestQPHelper.QPTestAnalyzer()`: filters `MockTokenizer` with
/// `QPTestFilter`.
pub fn qp_test_analyzer() -> Arc<dyn Analyzer> {
    FnAnalyzer::new(
        "org.apache.lucene.queryparser.flexible.standard.TestQPHelper$QPTestAnalyzer",
        |_| {
            let tokenizer = mock_tokenizer(&SIMPLE, true)?;
            let filter = qp_test_filter(shared(&tokenizer))?;
            Ok(components(tokenizer, sink(filter)))
        },
    )
    .arc()
}

/// `new QPTestParser(a)`: a `StandardQueryParser` whose processor pipeline
/// also rejects wildcard and fuzzy nodes.
fn qp_test_parser(a: Arc<dyn Analyzer>) -> Result<StandardQueryParser> {
    let mut parser = StandardQueryParser::new();
    // `((QueryNodeProcessorPipeline) getQueryNodeProcessor()).add(...)`
    let processor = parser
        .get_query_node_processor()
        .ok_or_else(LuceneError::null_pointer)?;
    let pipeline = processor
        .as_any()
        .downcast_ref::<StandardQueryNodeProcessorPipeline>()
        .ok_or_else(|| {
            LuceneError::unchecked(
                "java.lang.ClassCastException",
                "cannot be cast to class org.apache.lucene.queryparser.flexible.core.processors.QueryNodeProcessorPipeline",
            )
        })?;
    pipeline.add(Arc::new(QpTestParserQueryNodeProcessor::default()));
    parser.set_analyzer(Some(a))?;
    Ok(parser)
}

/// `QPTestParser.QPTestParserQueryNodeProcessor`.
#[derive(Debug, Default)]
struct QpTestParserQueryNodeProcessor {
    base: QueryNodeProcessorImpl,
}

impl QueryNodeProcessorImplOps for QpTestParserQueryNodeProcessor {
    fn query_node_processor_impl(&self) -> &QueryNodeProcessorImpl {
        &self.base
    }

    fn post_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef> {
        Ok(node)
    }

    fn pre_process_node(&self, node: QueryNodeRef) -> Result<QueryNodeRef> {
        if instance_of(node.as_ref(), &WILDCARD_QUERY_NODE_CLASS)
            || instance_of(node.as_ref(), &FUZZY_QUERY_NODE_CLASS)
        {
            return Err(QueryNodeException::new(Arc::new(MessageImpl::new(
                QueryParserMessages::EMPTY_MESSAGE.get(),
            )))
            .error());
        }
        Ok(node)
    }

    fn set_children_order(&self, children: Vec<QueryNodeRef>) -> Result<Vec<QueryNodeRef>> {
        Ok(children)
    }
}

/// `new SimpleCJKAnalyzer()` (TestQPHelper's: individual CJK chars as
/// terms, like StandardAnalyzer).
fn simple_cjk_analyzer() -> Arc<dyn Analyzer> {
    FnAnalyzer::new(
        "org.apache.lucene.queryparser.flexible.standard.TestQPHelper$SimpleCJKAnalyzer",
        |_| Ok(tokenizer_components(simple_cjk_tokenizer()?)),
    )
    .arc()
}

/// `TestQPHelper.CannedTokenizer`.
#[derive(Debug)]
struct CannedTokenizer {
    upto: i32,
    pos_incr: Att,
    term: Att,
}

impl TokenizerLogic for CannedTokenizer {
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        _input: &mut dyn CharReader,
    ) -> Result<bool> {
        source.clear_attributes();
        if self.upto == 4 {
            return Ok(false);
        }
        let (inc, text) = match self.upto {
            0 => (1, "a"),
            1 => (1, "b"),
            2 => (0, "c"),
            _ => (0, "d"),
        };
        self.pos_incr.pos_inc_mut().set_position_increment(inc);
        self.term.set_term(text);
        self.upto += 1;
        Ok(true)
    }

    fn reset(&mut self, _source: &mut AttributeSource, _reader: &mut dyn CharReader) -> Result<()> {
        self.upto = 0;
        Ok(())
    }
}

/// `new CannedAnalyzer()`.
fn canned_analyzer() -> Arc<dyn Analyzer> {
    FnAnalyzer::new(
        "org.apache.lucene.queryparser.flexible.standard.TestQPHelper$CannedAnalyzer",
        |_| {
            let mut source = AttributeSource::new_with_factory(
                rucene::analysis::default_token_attribute_factory(),
            );
            let pos_incr = Att::add(&mut source, pos_inc_id())?;
            let term = Att::add(&mut source, char_term_id())?;
            let t = Tokenizer::with_source(
                source,
                CannedTokenizer {
                    upto: 0,
                    pos_incr,
                    term,
                },
            );
            Ok(tokenizer_components(Arc::new(AtomicRefCell::new(t))))
        },
    )
    .arc()
}

// ---------------------------------------------------------------------------
// The class and its helpers
// ---------------------------------------------------------------------------

/// The instance of `TestQPHelper`.
#[derive(Default)]
pub struct TestQPHelper {
    original_max_clauses: i32,
}

/// `TestQPHelper`.
pub fn class_def() -> Arc<Class> {
    let mut methods: Vec<Method> = vec![
        static_hook("beforeClass", BeforeClass, before_class),
        static_hook("afterClass", AfterClass, after_class),
        set_up(test_set_up),
        tear_down(test_tear_down),
    ];
    methods.extend(jdk_default_hooks());
    type F = TestQPHelper;
    let tests: Vec<(&str, FixtureTest<F>)> = vec![
        (
            "testConstantScoreAutoRewrite",
            test_constant_score_auto_rewrite,
        ),
        ("testCJK", test_cjk),
        ("testCJKTerm", test_cjk_term),
        ("testCJKBoostedTerm", test_cjk_boosted_term),
        ("testCJKPhrase", test_cjk_phrase),
        ("testCJKBoostedPhrase", test_cjk_boosted_phrase),
        ("testCJKSloppyPhrase", test_cjk_sloppy_phrase),
        ("testSimple", test_simple),
        ("testParse", test_parse),
        ("testPunct", test_punct),
        ("testGroup", test_group),
        ("testSlop", test_slop),
        ("testNumber", test_number),
        ("testLeadingNegation", test_leading_negation),
        ("testNegationInParentheses", test_negation_in_parentheses),
        ("testWildcard", test_wildcard),
        ("testLeadingWildcardType", test_leading_wildcard_type),
        ("testQPA", test_qpa),
        ("testRange", test_range),
        ("testDateRange", test_date_range),
        ("testEscaped", test_escaped),
        ("testQueryStringEscaping", test_query_string_escaping),
        (
            "testTabNewlineCarriageReturn",
            test_tab_newline_carriage_return,
        ),
        ("testSimpleDAO", test_simple_dao),
        ("testBoost", test_boost),
        ("testException", test_exception),
        (
            "testCustomQueryParserWildcard",
            test_custom_query_parser_wildcard,
        ),
        ("testCustomQueryParserFuzzy", test_custom_query_parser_fuzzy),
        ("testBooleanQuery", test_boolean_query),
        ("testPrecedence", test_precedence),
        ("testStarParsing", test_star_parsing),
        ("testRegexps", test_regexps),
        ("testStopwords", test_stopwords),
        ("testPositionIncrement", test_position_increment),
        ("testMatchAllDocs", test_match_all_docs),
        ("testMultiPhraseQuery", test_multi_phrase_query),
        ("testRegexQueryParsing", test_regex_query_parsing),
    ];
    for (name, body) in tests {
        methods.push(ftest(name, body));
    }
    methods.push(
        ftest("testEscapedWildcard", test_escaped_wildcard).annotate(Ignore {
            value: "flexible queryparser shouldn't escape wildcard terms".to_string(),
        }),
    );
    fixture_class::<TestQPHelper>(
        "org.apache.lucene.queryparser.flexible.standard.TestQPHelper",
        methods,
    )
}

/// `setUp()`.
fn test_set_up(t: &mut TestQPHelper) -> Result<()> {
    t.original_max_clauses = IndexSearcher::get_max_clause_count();
    Ok(())
}

/// `tearDown()`.
fn test_tear_down(t: &mut TestQPHelper) -> Result<()> {
    IndexSearcher::set_max_clause_count(t.original_max_clauses)
}

/// A `Query` the Java code dereferences: `null` throws
/// `NullPointerException` (the standard builders never build `null`).
fn non_null(q: Option<Arc<dyn Query>>) -> Result<Arc<dyn Query>> {
    q.ok_or_else(LuceneError::null_pointer)
}

/// `qp.parse(query, field)`.
fn parse(qp: &StandardQueryParser, query: &str, field: &str) -> Result<Arc<dyn Query>> {
    non_null(qp.parse(query, Some(field))?)
}

/// `getParser(Analyzer a)`.
fn get_parser(a: Option<Arc<dyn Analyzer>>) -> Result<StandardQueryParser> {
    let a = a.unwrap_or_else(|| mock_analyzer(&SIMPLE, true));
    let mut qp = StandardQueryParser::new();
    qp.set_analyzer(Some(a))?;
    qp.set_default_operator(Some(Operator::Or))?;
    Ok(qp)
}

/// `getQuery(String query, Analyzer a)`.
fn get_query(query: &str, a: Option<Arc<dyn Analyzer>>) -> Result<Arc<dyn Query>> {
    parse(&get_parser(a)?, query, "field")
}

/// `getQueryAllowLeadingWildcard(String query, Analyzer a)`.
fn get_query_allow_leading_wildcard(
    query: &str,
    a: Option<Arc<dyn Analyzer>>,
) -> Result<Arc<dyn Query>> {
    let mut parser = get_parser(a)?;
    parser.set_allow_leading_wildcard(true)?;
    parse(&parser, query, "field")
}

fn check(query: &str, s: &str, result: &str) {
    if s != result {
        Assert::fail(&format!(
            "Query /{query}/ yielded /{s}/, expecting /{result}/"
        ));
    }
}

/// `assertQueryEquals(String query, Analyzer a, String result)`.
fn assert_query_equals(query: &str, a: Option<Arc<dyn Analyzer>>, result: &str) -> Result<()> {
    let q = get_query(query, a)?;
    check(query, &q.to_query_string("field"), result);
    Ok(())
}

/// `assertMatchNoDocsQuery(String queryString, Analyzer a)`.
fn assert_match_no_docs_query_str(query_string: &str, a: Option<Arc<dyn Analyzer>>) -> Result<()> {
    assert_match_no_docs_query(&*get_query(query_string, a)?);
    Ok(())
}

/// `assertMatchNoDocsQuery(Query query)`.
fn assert_match_no_docs_query(query: &dyn Query) {
    let good = is::<MatchNoDocsQuery>(query)
        || (is::<BooleanQuery>(query)
            && cast::<BooleanQuery>(query, "BooleanQuery")
                .clauses()
                .is_empty());
    if !good {
        Assert::fail(&format!(
            "expected MatchNoDocsQuery or an empty BooleanQuery but got: {}",
            to_s(query)
        ));
    }
}

/// `assertQueryEqualsAllowLeadingWildcard(String query, Analyzer a, String
/// result)`.
fn assert_query_equals_allow_leading_wildcard(
    query: &str,
    a: Option<Arc<dyn Analyzer>>,
    result: &str,
) -> Result<()> {
    let q = get_query_allow_leading_wildcard(query, a)?;
    check(query, &q.to_query_string("field"), result);
    Ok(())
}

/// `assertQueryEquals(StandardQueryParser qp, String field, String query,
/// String result)`.
fn assert_query_equals_qp(
    qp: &StandardQueryParser,
    field: &str,
    query: &str,
    result: &str,
) -> Result<()> {
    let q = parse(qp, query, field)?;
    check(query, &q.to_query_string(field), result);
    Ok(())
}

/// `assertEscapedQueryEquals(String query, Analyzer a, String result)`.
fn assert_escaped_query_equals(query: &str, _a: Option<Arc<dyn Analyzer>>, result: &str) {
    let escaped_query = QueryParserUtil::escape(query);
    check(query, &escaped_query, result);
}

/// `assertWildcardQueryEquals(String query, String result, boolean
/// allowLeadingWildcard)`.
fn assert_wildcard_query_equals_allow(
    query: &str,
    result: &str,
    allow_leading_wildcard: bool,
) -> Result<()> {
    let mut qp = get_parser(None)?;
    qp.set_allow_leading_wildcard(allow_leading_wildcard)?;
    let q = parse(&qp, query, "field")?;
    let s = q.to_query_string("field");
    if s != result {
        Assert::fail(&format!(
            "WildcardQuery /{query}/ yielded /{s}/, expecting /{result}/"
        ));
    }
    Ok(())
}

/// `assertWildcardQueryEquals(String query, String result)`.
fn assert_wildcard_query_equals(query: &str, result: &str) -> Result<()> {
    assert_wildcard_query_equals_allow(query, result, false)
}

/// `getQueryDOA(String query, Analyzer a)`.
fn get_query_doa(query: &str, a: Option<Arc<dyn Analyzer>>) -> Result<Arc<dyn Query>> {
    let a = a.unwrap_or_else(|| mock_analyzer(&SIMPLE, true));
    let mut qp = StandardQueryParser::new();
    qp.set_analyzer(Some(a))?;
    qp.set_default_operator(Some(Operator::And))?;
    parse(&qp, query, "field")
}

/// `assertQueryEqualsDOA(String query, Analyzer a, String result)`.
fn assert_query_equals_doa(query: &str, a: Option<Arc<dyn Analyzer>>, result: &str) -> Result<()> {
    let q = get_query_doa(query, a)?;
    check(query, &q.to_query_string("field"), result);
    Ok(())
}

/// `assertQueryNodeException(String queryString)`.
fn assert_query_node_exception(query_string: &str) -> Result<()> {
    expect_throws(QUERY_NODE_EXCEPTION, || get_query(query_string, None));
    Ok(())
}

fn bq(clauses: Vec<(Arc<dyn Query>, Occur)>) -> Result<Arc<dyn Query>> {
    let mut b = BooleanQueryBuilder::new();
    for (q, o) in clauses {
        b.add(q, o)?;
    }
    Ok(Arc::new(b.build()))
}

/// `new RegexpQuery(new Term(field, text))`.
fn regexp(field: &str, text: &str) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(RegexpQuery::new(term(field, text))?))
}

/// `((MultiTermQuery) q).getRewriteMethod()`: the cast is a downcast to
/// the multi-term classes the parser builds.
fn rewrite_method_of(q: &dyn Query) -> Arc<dyn RewriteMethod> {
    let any = q.as_any();
    if let Some(m) = any.downcast_ref::<WildcardQuery>() {
        return m.get_rewrite_method();
    }
    if let Some(m) = any.downcast_ref::<PrefixQuery>() {
        return m.get_rewrite_method();
    }
    if let Some(m) = any.downcast_ref::<TermRangeQuery>() {
        return m.get_rewrite_method();
    }
    if let Some(m) = any.downcast_ref::<RegexpQuery>() {
        return m.get_rewrite_method();
    }
    if let Some(m) = any.downcast_ref::<FuzzyQuery>() {
        return m.get_rewrite_method();
    }
    randomizedtesting::jdk::exception::throw(
        "java.lang.ClassCastException",
        format!(
            "class {} cannot be cast to class org.apache.lucene.search.MultiTermQuery",
            q.java_class_name()
        ),
    )
}

/// `assertEquals(expected, ((MultiTermQuery) q).getRewriteMethod())`.
fn assert_rewrite_method(expected: &Arc<dyn RewriteMethod>, q: &dyn Query) {
    Assert::assert_true(expected.method_eq(&*rewrite_method_of(q)));
}

// ---------------------------------------------------------------------------
// Test methods
// ---------------------------------------------------------------------------

fn test_constant_score_auto_rewrite(_t: &mut TestQPHelper) -> Result<()> {
    let qp = StandardQueryParser::with_analyzer(Some(mock_analyzer(&WHITESPACE, false)));
    let q = parse(&qp, "foo*bar", "field")?;
    Assert::assert_true(is::<WildcardQuery>(&*q));
    assert_rewrite_method(&constant_score_blended_rewrite(), &*q);

    let q = parse(&qp, "foo*", "field")?;
    Assert::assert_true(is::<PrefixQuery>(&*q));
    assert_rewrite_method(&constant_score_blended_rewrite(), &*q);

    let q = parse(&qp, "[a TO z]", "field")?;
    Assert::assert_true(is::<TermRangeQuery>(&*q));
    assert_rewrite_method(&constant_score_blended_rewrite(), &*q);
    Ok(())
}

fn test_cjk(_t: &mut TestQPHelper) -> Result<()> {
    // Test Ideographic Space - As wide as a CJK character cell (fullwidth)
    // used google to translate the word "term" to japanese -> ??
    assert_query_equals(
        "term\u{3000}term\u{3000}term",
        None,
        "term\u{0020}term\u{0020}term",
    )?;
    assert_query_equals_allow_leading_wildcard(
        "??\u{3000}??\u{3000}??",
        None,
        "??\u{0020}??\u{0020}??",
    )?;
    Ok(())
}

fn test_cjk_term(_t: &mut TestQPHelper) -> Result<()> {
    // individual CJK chars as terms
    let analyzer = simple_cjk_analyzer();

    let expected = bq(vec![
        (tq("field", "中"), Occur::SHOULD),
        (tq("field", "国"), Occur::SHOULD),
    ])?;
    assert_query_eq(&*expected, &*get_query("中国", Some(analyzer))?);

    let inner = bq(vec![
        (tq("field", "中"), Occur::SHOULD),
        (tq("field", "国"), Occur::SHOULD),
    ])?;
    let expected = bq(vec![(tq("field", "中"), Occur::MUST), (inner, Occur::MUST)])?;
    assert_query_eq(
        &*expected,
        &*get_query("中 AND 中国", Some(simple_cjk_analyzer()))?,
    );
    Ok(())
}

fn test_cjk_boosted_term(_t: &mut TestQPHelper) -> Result<()> {
    // individual CJK chars as terms
    let analyzer = simple_cjk_analyzer();

    let expected = bq(vec![
        (tq("field", "中"), Occur::SHOULD),
        (tq("field", "国"), Occur::SHOULD),
    ])?;
    let expected = BoostQuery::new(expected, 0.5)?;
    assert_query_eq(&expected, &*get_query("中国^0.5", Some(analyzer))?);
    Ok(())
}

fn test_cjk_phrase(_t: &mut TestQPHelper) -> Result<()> {
    // individual CJK chars as terms
    let analyzer = simple_cjk_analyzer();

    let expected = PhraseQuery::from_strings("field", &["中", "国"])?;

    assert_query_eq(&expected, &*get_query("\"中国\"", Some(analyzer))?);
    Ok(())
}

fn test_cjk_boosted_phrase(_t: &mut TestQPHelper) -> Result<()> {
    // individual CJK chars as terms
    let analyzer = simple_cjk_analyzer();

    let expected: Arc<dyn Query> = Arc::new(PhraseQuery::from_strings("field", &["中", "国"])?);
    let expected = BoostQuery::new(expected, 0.5)?;

    assert_query_eq(&expected, &*get_query("\"中国\"^0.5", Some(analyzer))?);
    Ok(())
}

fn test_cjk_sloppy_phrase(_t: &mut TestQPHelper) -> Result<()> {
    // individual CJK chars as terms
    let analyzer = simple_cjk_analyzer();

    let expected = PhraseQuery::with_slop(3, "field", &["中", "国"])?;

    assert_query_eq(&expected, &*get_query("\"中国\"~3", Some(analyzer))?);
    Ok(())
}

fn test_simple(_t: &mut TestQPHelper) -> Result<()> {
    assert_query_equals("field=a", None, "a")?;
    assert_query_equals("\"term germ\"~2", None, "\"term germ\"~2")?;
    assert_query_equals("term term term", None, "term term term")?;
    assert_query_equals(
        "türm term term",
        Some(mock_analyzer(&WHITESPACE, false)),
        "türm term term",
    )?;
    assert_query_equals("ümlaut", Some(mock_analyzer(&WHITESPACE, false)), "ümlaut")?;

    // FIXME: change MockAnalyzer to not extend CharTokenizer for this test
    // assertQueryEquals("\"\"", new KeywordAnalyzer(), "");
    // assertQueryEquals("foo:\"\"", new KeywordAnalyzer(), "foo:");

    assert_query_equals("a AND b", None, "+a +b")?;
    assert_query_equals("(a AND b)", None, "+a +b")?;
    assert_query_equals("c OR (a AND b)", None, "c (+a +b)")?;

    assert_query_equals("a AND NOT b", None, "+a -b")?;

    assert_query_equals("a AND -b", None, "+a -b")?;

    assert_query_equals("a AND !b", None, "+a -b")?;

    assert_query_equals("a && b", None, "+a +b")?;

    assert_query_equals("a && ! b", None, "+a -b")?;

    assert_query_equals("a OR b", None, "a b")?;
    assert_query_equals("a || b", None, "a b")?;

    assert_query_equals("a OR !b", None, "a -b")?;

    assert_query_equals("a OR ! b", None, "a -b")?;

    assert_query_equals("a OR -b", None, "a -b")?;

    assert_query_equals("+term -term term", None, "+term -term term")?;
    assert_query_equals(
        "foo:term AND field:anotherTerm",
        None,
        "+foo:term +anotherterm",
    )?;
    assert_query_equals(
        "term AND \"phrase phrase\"",
        None,
        "+term +\"phrase phrase\"",
    )?;
    assert_query_equals("\"hello there\"", None, "\"hello there\"")?;
    Assert::assert_true(is::<BooleanQuery>(&*get_query("a AND b", None)?));
    Assert::assert_true(is::<TermQuery>(&*get_query("hello", None)?));
    Assert::assert_true(is::<PhraseQuery>(&*get_query("\"hello there\"", None)?));

    assert_query_equals("germ term^2.0", None, "germ (term)^2.0")?;
    assert_query_equals("(term)^2.0", None, "(term)^2.0")?;
    assert_query_equals("(germ term)^2.0", None, "(germ term)^2.0")?;
    assert_query_equals("term^2.0", None, "(term)^2.0")?;
    assert_query_equals("term^2", None, "(term)^2.0")?;
    assert_query_equals("\"germ term\"^2.0", None, "(\"germ term\")^2.0")?;
    assert_query_equals("\"term germ\"^2", None, "(\"term germ\")^2.0")?;

    assert_query_equals(
        "(foo OR bar) AND (baz OR boo)",
        None,
        "+(foo bar) +(baz boo)",
    )?;
    assert_query_equals("((a OR b) AND NOT c) OR d", None, "(+(a b) -c) d")?;
    assert_query_equals(
        "+(apple \"steve jobs\") -(foo bar baz)",
        None,
        "+(apple \"steve jobs\") -(foo bar baz)",
    )?;
    assert_query_equals(
        "+title:(dog OR cat) -author:\"bob dole\"",
        None,
        "+(title:dog title:cat) -author:\"bob dole\"",
    )?;
    Ok(())
}

fn test_parse(_t: &mut TestQPHelper) -> Result<()> {
    let mut p = StandardSyntaxParser::with_char_stream(Box::new(FastCharStream::new(Box::new(
        StringCharReader::new(""),
    ))));
    p.re_init(Box::new(FastCharStream::new(Box::new(
        StringCharReader::new("title:(dog OR cat)"),
    ))));
    // `System.out.println(p.TopLevelQuery("_fld_"))`
    let node = p.top_level_query(Some(CharSequence::from("_fld_")))?;
    match node {
        None => println!("null"),
        Some(node) => println!("{}", node.to_java_string()?),
    }
    Ok(())
}

fn test_punct(_t: &mut TestQPHelper) -> Result<()> {
    let a = mock_analyzer(&WHITESPACE, false);
    assert_query_equals("a&b", Some(Arc::clone(&a)), "a&b")?;
    assert_query_equals("a&&b", Some(Arc::clone(&a)), "a&&b")?;
    assert_query_equals(".NET", Some(a), ".NET")?;
    Ok(())
}

fn test_group(_t: &mut TestQPHelper) -> Result<()> {
    assert_query_equals("!(a AND b) OR c", None, "-(+a +b) c")?;
    assert_query_equals("!(a AND b) AND c", None, "-(+a +b) +c")?;
    assert_query_equals("((a AND b) AND c)", None, "+(+a +b) +c")?;
    assert_query_equals("(a AND b) AND c", None, "+(+a +b) +c")?;
    assert_query_equals("b !(a AND b)", None, "b -(+a +b)")?;
    assert_query_equals("(a AND b)^4 OR c", None, "(+a +b)^4.0 c")?;
    Ok(())
}

fn test_slop(_t: &mut TestQPHelper) -> Result<()> {
    assert_query_equals("\"term germ\"~2", None, "\"term germ\"~2")?;
    assert_query_equals("\"term germ\"~2 flork", None, "\"term germ\"~2 flork")?;
    assert_query_equals("\"term\"~2", None, "term")?;
    assert_query_equals("\" \"~2 germ", None, "germ")?;
    assert_query_equals("\"term germ\"~2^2", None, "(\"term germ\"~2)^2.0")?;
    Ok(())
}

fn test_number(_t: &mut TestQPHelper) -> Result<()> {
    // The numbers go away because SimpleAnalyzer ignores them
    assert_match_no_docs_query_str("3", None)?;
    assert_query_equals("term 1.0 1 2", None, "term")?;
    assert_query_equals("term term1 term2", None, "term term term")?;

    let a = mock_analyzer(&WHITESPACE, false);
    assert_query_equals("3", Some(Arc::clone(&a)), "3")?;
    assert_query_equals("term 1.0 1 2", Some(Arc::clone(&a)), "term 1.0 1 2")?;
    assert_query_equals("term term1 term2", Some(a), "term term1 term2")?;
    Ok(())
}

fn test_leading_negation(_t: &mut TestQPHelper) -> Result<()> {
    assert_query_equals("-term", None, "-term")?;
    assert_query_equals("!term", None, "-term")?;
    assert_query_equals("NOT term", None, "-term")?;
    Ok(())
}

fn test_negation_in_parentheses(_t: &mut TestQPHelper) -> Result<()> {
    assert_query_equals("(-a)", None, "-a")?;
    assert_query_equals("(!a)", None, "-a")?;
    assert_query_equals("(NOT a)", None, "-a")?;
    assert_query_equals("a (!b)", None, "a (-b)")?;
    assert_query_equals("+a +(!b)", None, "+a +(-b)")?;
    assert_query_equals("a AND (!b)", None, "+a +(-b)")?;
    assert_query_equals("a (NOT b)", None, "a (-b)")?;
    assert_query_equals("a AND (NOT b)", None, "+a +(-b)")?;
    Ok(())
}

fn test_wildcard(_t: &mut TestQPHelper) -> Result<()> {
    assert_query_equals("term*", None, "term*")?;
    assert_query_equals("term*^2", None, "(term*)^2.0")?;
    assert_query_equals("term~", None, "term~2")?;
    assert_query_equals("term~0.7", None, "term~1")?;

    assert_query_equals("term~^3", None, "(term~2)^3.0")?;
    assert_query_equals("term*germ", None, "term*germ")?;
    assert_query_equals("term*germ^3", None, "(term*germ)^3.0")?;

    Assert::assert_true(is::<PrefixQuery>(&*get_query("term*", None)?));
    Assert::assert_true(is::<BoostQuery>(&*get_query("term*^2", None)?));
    let q = get_query("term*^2", None)?;
    Assert::assert_true(is::<PrefixQuery>(
        &*cast::<BoostQuery>(&*q, BOOST_QUERY).get_query(),
    ));
    Assert::assert_true(is::<FuzzyQuery>(&*get_query("term~", None)?));
    Assert::assert_true(is::<FuzzyQuery>(&*get_query("term~0.7", None)?));
    let q = get_query("term~0.7", None)?;
    let fq = cast::<FuzzyQuery>(&*q, "org.apache.lucene.search.FuzzyQuery");
    Assert::assert_equals(1, fq.get_max_edits());
    Assert::assert_equals(fuzzy_query::DEFAULT_PREFIX_LENGTH, fq.get_prefix_length());
    let q = get_query("term~", None)?;
    let fq = cast::<FuzzyQuery>(&*q, "org.apache.lucene.search.FuzzyQuery");
    Assert::assert_equals(2, fq.get_max_edits());
    Assert::assert_equals(fuzzy_query::DEFAULT_PREFIX_LENGTH, fq.get_prefix_length());

    assert_query_node_exception("term~1.1")?; // value > 1, throws exception
    assert_query_node_exception("term^3~")?; // Boost must be applied to FuzzyOp.

    Assert::assert_true(is::<WildcardQuery>(&*get_query("term*germ", None)?));

    /*
     * Tests to see that wild card terms are (or are not) properly lower-cased
     * with propery parser configuration
     */
    // First prefix queries:
    // by default, convert to lowercase:
    assert_wildcard_query_equals("Term*", "term*")?;
    // explicitly set lowercase:
    assert_wildcard_query_equals("term*", "term*")?;
    assert_wildcard_query_equals("Term*", "term*")?;
    assert_wildcard_query_equals("TERM*", "term*")?;
    // Then 'full' wildcard queries:
    // by default, convert to lowercase:
    assert_wildcard_query_equals("Te?m", "te?m")?;
    // explicitly set lowercase:
    assert_wildcard_query_equals("te?m", "te?m")?;
    assert_wildcard_query_equals("Te?m", "te?m")?;
    assert_wildcard_query_equals("TE?M", "te?m")?;
    assert_wildcard_query_equals("Te?m*gerM", "te?m*germ")?;
    // Fuzzy queries:
    assert_wildcard_query_equals("Term~", "term~2")?;
    // Range queries:

    // TODO: implement this on QueryParser
    // Q0002E_INVALID_SYNTAX_CANNOT_PARSE: Syntax Error, cannot parse '[A TO
    // C]': Lexical error at line 1, column 1. Encountered: "[" (91), after
    // : ""
    assert_wildcard_query_equals("[A TO C]", "[a TO c]")?;
    // Test suffix queries: first disallow
    expect_throws(QUERY_NODE_EXCEPTION, || {
        assert_wildcard_query_equals("*Term", "*term")
    });

    expect_throws(QUERY_NODE_EXCEPTION, || {
        assert_wildcard_query_equals("?Term", "?term")
    });

    // Test suffix queries: then allow
    assert_wildcard_query_equals_allow("*Term", "*term", true)?;
    assert_wildcard_query_equals_allow("?Term", "?term", true)?;
    Ok(())
}

fn test_leading_wildcard_type(_t: &mut TestQPHelper) -> Result<()> {
    let mut qp = get_parser(None)?;
    qp.set_allow_leading_wildcard(true)?;
    let wildcard = "class org.apache.lucene.search.WildcardQuery".to_string();
    let class_of = |q: Arc<dyn Query>| format!("class {}", q.java_class_name());
    Assert::assert_equals(wildcard.clone(), class_of(parse(&qp, "t*erm*", "field")?));
    Assert::assert_equals(wildcard.clone(), class_of(parse(&qp, "?term*", "field")?));
    Assert::assert_equals(wildcard, class_of(parse(&qp, "*term*", "field")?));
    Ok(())
}

fn test_qpa(_t: &mut TestQPHelper) -> Result<()> {
    let qpa = || Some(qp_analyzer());
    assert_query_equals("term term^3.0 term", qpa(), "term (term)^3.0 term")?;
    assert_query_equals("term stop^3.0 term", qpa(), "term term")?;

    assert_query_equals("term term term", qpa(), "term term term")?;
    assert_query_equals("term +stop term", qpa(), "term term")?;
    assert_query_equals("term -stop term", qpa(), "term term")?;

    assert_query_equals("drop AND (stop) AND roll", qpa(), "+drop +roll")?;
    assert_query_equals("term +(stop) term", qpa(), "term term")?;
    assert_query_equals("term -(stop) term", qpa(), "term term")?;

    assert_query_equals("drop AND stop AND roll", qpa(), "+drop +roll")?;
    assert_query_equals("term phrase term", qpa(), "term (phrase1 phrase2) term")?;

    assert_query_equals(
        "term AND NOT phrase term",
        qpa(),
        "+term -(phrase1 phrase2) term",
    )?;

    assert_match_no_docs_query_str("stop^3", qpa())?;
    assert_match_no_docs_query_str("stop", qpa())?;
    assert_match_no_docs_query_str("(stop)^3", qpa())?;
    assert_match_no_docs_query_str("((stop))^3", qpa())?;
    assert_match_no_docs_query_str("(stop^3)", qpa())?;
    assert_match_no_docs_query_str("((stop)^3)", qpa())?;
    assert_match_no_docs_query_str("(stop)", qpa())?;
    assert_match_no_docs_query_str("((stop))", qpa())?;
    Assert::assert_true(is::<BooleanQuery>(&*get_query("term term term", qpa())?));
    Assert::assert_true(is::<TermQuery>(&*get_query("term +stop", qpa())?));
    Ok(())
}

fn test_range(_t: &mut TestQPHelper) -> Result<()> {
    assert_query_equals("[ a TO z]", None, "[a TO z]")?;
    let q = get_query("[ a TO z]", None)?;
    assert_rewrite_method(
        &constant_score_blended_rewrite(),
        cast::<TermRangeQuery>(&*q, "org.apache.lucene.search.TermRangeQuery"),
    );

    let mut qp = StandardQueryParser::new();

    qp.set_multi_term_rewrite_method(Some(scoring_boolean_rewrite()))?;
    let q = parse(&qp, "[ a TO z]", "field")?;
    assert_rewrite_method(
        &scoring_boolean_rewrite(),
        cast::<TermRangeQuery>(&*q, "org.apache.lucene.search.TermRangeQuery"),
    );

    // test open ranges
    assert_query_equals("[ a TO * ]", None, "[a TO *]")?;
    assert_query_equals("[ * TO z ]", None, "[* TO z]")?;
    assert_query_equals("[ * TO * ]", None, "[* TO *]")?;

    assert_query_equals("field>=a", None, "[a TO *]")?;
    assert_query_equals("field>a", None, "{a TO *]")?;
    assert_query_equals("field<=a", None, "[* TO a]")?;
    assert_query_equals("field<a", None, "[* TO a}")?;

    // mixing exclude and include bounds
    assert_query_equals("{ a TO z ]", None, "{a TO z]")?;
    assert_query_equals("[ a TO z }", None, "[a TO z}")?;
    assert_query_equals("{ a TO * ]", None, "{a TO *]")?;
    assert_query_equals("[ * TO z }", None, "[* TO z}")?;

    assert_query_equals("[ a TO z ]", None, "[a TO z]")?;
    assert_query_equals("{ a TO z}", None, "{a TO z}")?;
    assert_query_equals("{ a TO z }", None, "{a TO z}")?;
    assert_query_equals("{ a TO z }^2.0", None, "({a TO z})^2.0")?;
    assert_query_equals("[ a TO z] OR bar", None, "[a TO z] bar")?;
    assert_query_equals("[ a TO z] AND bar", None, "+[a TO z] +bar")?;
    assert_query_equals("( bar blar { a TO z}) ", None, "bar blar {a TO z}")?;
    assert_query_equals(
        "gack ( bar blar { a TO z}) ",
        None,
        "gack (bar blar {a TO z})",
    )?;
    Ok(())
}

fn test_date_range(_t: &mut TestQPHelper) -> Result<()> {
    let start_date = get_localized_date(2002, 1, 1)?;
    let end_date = get_localized_date(2002, 1, 4)?;
    // we use the default Locale/TZ since LuceneTestCase randomizes it
    let mut end_date_expected =
        Calendar::new_gregorian(&TimeZone::get_default(), &Locale::get_default())?;
    end_date_expected.clear();
    end_date_expected.set_date_time_seconds(2002, 1, 4, 23, 59, 59);
    end_date_expected.set(calendar::MILLISECOND, 999);
    let default_field = "default";
    let month_field = "month";
    let hour_field = "hour";
    let mut qp = StandardQueryParser::new();

    let date_res: Arc<RwLock<HashMap<CharSequence, Resolution>>> =
        Arc::new(RwLock::new(HashMap::new()));

    // set a field specific date resolution
    date_res
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .insert(CharSequence::from(month_field), Resolution::MONTH);
    qp.set_date_resolution_map(Some(Arc::clone(&date_res)))?;

    // set default date resolution to MILLISECOND
    qp.set_date_resolution(Some(Resolution::MILLISECOND))?;

    // set second field specific date resolution
    date_res
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .insert(CharSequence::from(hour_field), Resolution::HOUR);
    qp.set_date_resolution_map(Some(Arc::clone(&date_res)))?;

    let end_inclusive = end_date_expected.get_time()?.get_time();
    // for this field no field specific date resolution has been set,
    // so verify if the default resolution is used
    assert_date_range_query_equals(
        &qp,
        default_field,
        &start_date,
        &end_date,
        end_inclusive,
        Resolution::MILLISECOND,
    )?;

    // verify if field specific date resolutions are used for these two
    // fields
    assert_date_range_query_equals(
        &qp,
        month_field,
        &start_date,
        &end_date,
        end_inclusive,
        Resolution::MONTH,
    )?;

    assert_date_range_query_equals(
        &qp,
        hour_field,
        &start_date,
        &end_date,
        end_inclusive,
        Resolution::HOUR,
    )?;
    Ok(())
}

/// `assertDateRangeQueryEquals(StandardQueryParser qp, String field, String
/// startDate, String endDate, Date endDateInclusive, Resolution
/// resolution)`.
fn assert_date_range_query_equals(
    qp: &StandardQueryParser,
    field: &str,
    start_date: &str,
    end_date: &str,
    end_date_inclusive: i64,
    resolution: Resolution,
) -> Result<()> {
    assert_query_equals_qp(
        qp,
        field,
        &format!(
            "{field}:[{} TO {}]",
            escape_date_string(start_date),
            escape_date_string(end_date)
        ),
        &format!(
            "[{} TO {}]",
            get_date_str(start_date, resolution)?,
            get_date_millis(end_date_inclusive, resolution)
        ),
    )?;
    assert_query_equals_qp(
        qp,
        field,
        &format!(
            "{field}:{{{} TO {}}}",
            escape_date_string(start_date),
            escape_date_string(end_date)
        ),
        &format!(
            "{{{} TO {}}}",
            get_date_str(start_date, resolution)?,
            get_date_str(end_date, resolution)?
        ),
    )?;
    Ok(())
}

fn test_escaped(_t: &mut TestQPHelper) -> Result<()> {
    let a = mock_analyzer(&WHITESPACE, false);
    let aa = || Some(Arc::clone(&a));

    assert_query_equals("\\*", aa(), "*")?;

    assert_query_equals("\\a", aa(), "a")?;

    assert_query_equals("a\\-b:c", aa(), "a-b:c")?;
    assert_query_equals("a\\+b:c", aa(), "a+b:c")?;
    assert_query_equals("a\\:b:c", aa(), "a:b:c")?;
    assert_query_equals("a\\\\b:c", aa(), "a\\b:c")?;

    assert_query_equals("a:b\\-c", aa(), "a:b-c")?;
    assert_query_equals("a:b\\+c", aa(), "a:b+c")?;
    assert_query_equals("a:b\\:c", aa(), "a:b:c")?;
    assert_query_equals("a:b\\\\c", aa(), "a:b\\c")?;

    assert_query_equals("a:b\\-c*", aa(), "a:b-c*")?;
    assert_query_equals("a:b\\+c*", aa(), "a:b+c*")?;
    assert_query_equals("a:b\\:c*", aa(), "a:b:c*")?;

    assert_query_equals("a:b\\\\c*", aa(), "a:b\\c*")?;

    assert_query_equals("a:b\\-?c", aa(), "a:b-?c")?;
    assert_query_equals("a:b\\+?c", aa(), "a:b+?c")?;
    assert_query_equals("a:b\\:?c", aa(), "a:b:?c")?;

    assert_query_equals("a:b\\\\?c", aa(), "a:b\\?c")?;

    assert_query_equals("a:b\\-c~", aa(), "a:b-c~2")?;
    assert_query_equals("a:b\\+c~", aa(), "a:b+c~2")?;
    assert_query_equals("a:b\\:c~", aa(), "a:b:c~2")?;
    assert_query_equals("a:b\\\\c~", aa(), "a:b\\c~2")?;

    // TODO: implement Range queries on QueryParser
    assert_query_equals("[ a\\- TO a\\+ ]", None, "[a- TO a+]")?;
    assert_query_equals("[ a\\: TO a\\~ ]", None, "[a: TO a~]")?;
    assert_query_equals("[ a\\\\ TO a\\* ]", None, "[a\\ TO a*]")?;

    assert_query_equals(
        "[\"c\\:\\\\temp\\\\\\~foo0.txt\" TO \"c\\:\\\\temp\\\\\\~foo9.txt\"]",
        aa(),
        "[c:\\temp\\~foo0.txt TO c:\\temp\\~foo9.txt]",
    )?;

    assert_query_equals("a\\\\\\+b", aa(), "a\\+b")?;

    assert_query_equals("a \\\"b c\\\" d", aa(), "a \"b c\" d")?;
    assert_query_equals("\"a \\\"b c\\\" d\"", aa(), "\"a \"b c\" d\"")?;
    assert_query_equals("\"a \\+b c d\"", aa(), "\"a +b c d\"")?;

    assert_query_equals("c\\:\\\\temp\\\\\\~foo.txt", aa(), "c:\\temp\\~foo.txt")?;

    assert_query_node_exception("XY\\")?; // there must be a character after the
                                          // escape char

    // test unicode escaping
    assert_query_equals("a\\u0062c", aa(), "abc")?;
    assert_query_equals("XY\\u005a", aa(), "XYZ")?;
    assert_query_equals("XY\\u005A", aa(), "XYZ")?;
    assert_query_equals("\"a \\\\\\u0028\\u0062\\\" c\"", aa(), "\"a \\(b\" c\"")?;

    assert_query_node_exception("XY\\u005G")?; // test non-hex character in escaped
                                               // unicode sequence
    assert_query_node_exception("XY\\u005")?; // test incomplete escaped unicode
                                              // sequence

    // Tests bug LUCENE-800
    assert_query_equals("(item:\\\\ item:ABCD\\\\)", aa(), "item:\\ item:ABCD\\")?;
    assert_query_node_exception("(item:\\\\ item:ABCD\\\\))")?; // unmatched closing
                                                                // paranthesis
    assert_query_equals("\\*", aa(), "*")?;
    assert_query_equals("\\\\", aa(), "\\")?; // escaped backslash

    assert_query_node_exception("\\")?; // a backslash must always be escaped

    // LUCENE-1189
    assert_query_equals("(\"a\\\\\") or (\"b\")", aa(), "a\\ or b")?;
    Ok(())
}

fn test_query_string_escaping(_t: &mut TestQPHelper) -> Result<()> {
    let a = mock_analyzer(&WHITESPACE, false);
    let aa = || Some(Arc::clone(&a));

    assert_escaped_query_equals("a-b:c", aa(), "a\\-b\\:c");
    assert_escaped_query_equals("a+b:c", aa(), "a\\+b\\:c");
    assert_escaped_query_equals("a:b:c", aa(), "a\\:b\\:c");
    assert_escaped_query_equals("a\\b:c", aa(), "a\\\\b\\:c");

    assert_escaped_query_equals("a:b-c", aa(), "a\\:b\\-c");
    assert_escaped_query_equals("a:b+c", aa(), "a\\:b\\+c");
    assert_escaped_query_equals("a:b:c", aa(), "a\\:b\\:c");
    assert_escaped_query_equals("a:b\\c", aa(), "a\\:b\\\\c");

    assert_escaped_query_equals("a:b-c*", aa(), "a\\:b\\-c\\*");
    assert_escaped_query_equals("a:b+c*", aa(), "a\\:b\\+c\\*");
    assert_escaped_query_equals("a:b:c*", aa(), "a\\:b\\:c\\*");

    assert_escaped_query_equals("a:b\\\\c*", aa(), "a\\:b\\\\\\\\c\\*");

    assert_escaped_query_equals("a:b-?c", aa(), "a\\:b\\-\\?c");
    assert_escaped_query_equals("a:b+?c", aa(), "a\\:b\\+\\?c");
    assert_escaped_query_equals("a:b:?c", aa(), "a\\:b\\:\\?c");

    assert_escaped_query_equals("a:b?c", aa(), "a\\:b\\?c");

    assert_escaped_query_equals("a:b-c~", aa(), "a\\:b\\-c\\~");
    assert_escaped_query_equals("a:b+c~", aa(), "a\\:b\\+c\\~");
    assert_escaped_query_equals("a:b:c~", aa(), "a\\:b\\:c\\~");
    assert_escaped_query_equals("a:b\\c~", aa(), "a\\:b\\\\c\\~");

    assert_escaped_query_equals("[ a - TO a+ ]", None, "\\[ a \\- TO a\\+ \\]");
    assert_escaped_query_equals("[ a : TO a~ ]", None, "\\[ a \\: TO a\\~ \\]");
    assert_escaped_query_equals("[ a\\ TO a* ]", None, "\\[ a\\\\ TO a\\* \\]");

    // LUCENE-881
    assert_escaped_query_equals("|| abc ||", aa(), "\\|\\| abc \\|\\|");
    assert_escaped_query_equals("&& abc &&", aa(), "\\&\\& abc \\&\\&");
    Ok(())
}

/// `@Ignore("flexible queryparser shouldn't escape wildcard terms")`.
fn test_escaped_wildcard(_t: &mut TestQPHelper) -> Result<()> {
    let mut qp = StandardQueryParser::new();
    qp.set_analyzer(Some(mock_analyzer(&WHITESPACE, false)))?;

    let q = WildcardQuery::new(term("field", "foo\\?ba?r"))?;
    assert_query_eq(&q, &*parse(&qp, "foo\\?ba?r", "field")?);
    Ok(())
}

fn test_tab_newline_carriage_return(_t: &mut TestQPHelper) -> Result<()> {
    assert_query_equals_doa("+weltbank +worlbank", None, "+weltbank +worlbank")?;

    assert_query_equals_doa("+weltbank\n+worlbank", None, "+weltbank +worlbank")?;
    assert_query_equals_doa("weltbank \n+worlbank", None, "+weltbank +worlbank")?;
    assert_query_equals_doa("weltbank \n +worlbank", None, "+weltbank +worlbank")?;

    assert_query_equals_doa("+weltbank\r+worlbank", None, "+weltbank +worlbank")?;
    assert_query_equals_doa("weltbank \r+worlbank", None, "+weltbank +worlbank")?;
    assert_query_equals_doa("weltbank \r +worlbank", None, "+weltbank +worlbank")?;

    assert_query_equals_doa("+weltbank\r\n+worlbank", None, "+weltbank +worlbank")?;
    assert_query_equals_doa("weltbank \r\n+worlbank", None, "+weltbank +worlbank")?;
    assert_query_equals_doa("weltbank \r\n +worlbank", None, "+weltbank +worlbank")?;
    assert_query_equals_doa("weltbank \r \n +worlbank", None, "+weltbank +worlbank")?;

    assert_query_equals_doa("+weltbank\t+worlbank", None, "+weltbank +worlbank")?;
    assert_query_equals_doa("weltbank \t+worlbank", None, "+weltbank +worlbank")?;
    assert_query_equals_doa("weltbank \t +worlbank", None, "+weltbank +worlbank")?;
    Ok(())
}

fn test_simple_dao(_t: &mut TestQPHelper) -> Result<()> {
    assert_query_equals_doa("term term term", None, "+term +term +term")?;
    assert_query_equals_doa("term +term term", None, "+term +term +term")?;
    assert_query_equals_doa("term term +term", None, "+term +term +term")?;
    assert_query_equals_doa("term +term +term", None, "+term +term +term")?;
    assert_query_equals_doa("-term term term", None, "-term +term +term")?;
    Ok(())
}

fn test_boost(_t: &mut TestQPHelper) -> Result<()> {
    let stop_set = Arc::new(CharacterRunAutomaton::new(Automata::make_string("on"))?);
    let one_stop_analyzer = mock_analyzer_filter(&SIMPLE, true, &stop_set);
    let mut qp = StandardQueryParser::new();
    qp.set_analyzer(Some(one_stop_analyzer))?;

    let q = qp.parse("on^1.0", Some("field"))?;
    Assert::assert_true(q.is_some()); // assertNotNull(q)
    let q = parse(&qp, "\"hello\"^2.0", "field")?;
    Assert::assert_equals_float_msg(
        None,
        cast::<BoostQuery>(&*q, BOOST_QUERY).get_boost(),
        2.0,
        0.5,
    );
    let q = parse(&qp, "hello^2.0", "field")?;
    Assert::assert_equals_float_msg(
        None,
        cast::<BoostQuery>(&*q, BOOST_QUERY).get_boost(),
        2.0,
        0.5,
    );
    let q = qp.parse("\"on\"^1.0", Some("field"))?;
    Assert::assert_true(q.is_some()); // assertNotNull(q)

    let mut qp2 = StandardQueryParser::new();
    qp2.set_analyzer(Some(mock_analyzer_filter(&SIMPLE, true, &ENGLISH_STOPSET)))?;

    let q = qp2.parse("the^3", Some("field"))?;
    // "the" is a stop word so the result is an empty query:
    Assert::assert_true(q.is_some()); // assertNotNull(q)
    let q = non_null(q)?;
    assert_match_no_docs_query(&*q);
    Assert::assert_false(is::<BoostQuery>(&*q));
    Ok(())
}

fn test_exception(_t: &mut TestQPHelper) -> Result<()> {
    assert_query_node_exception("*leadingWildcard")?; // disallowed by default
    assert_query_node_exception("\"some phrase")?;
    assert_query_node_exception("(foo bar")?;
    assert_query_node_exception("foo bar))")?;
    assert_query_node_exception("field:term:with:colon some more terms")?;
    assert_query_node_exception("(sub query)^5.0^2.0 plus more")?;
    assert_query_node_exception("secret AND illegal) AND access:confidential")?;
    Ok(())
}

// Wildcard queries should not be allowed
fn test_custom_query_parser_wildcard(_t: &mut TestQPHelper) -> Result<()> {
    expect_throws(QUERY_NODE_EXCEPTION, || {
        qp_test_parser(mock_analyzer(&WHITESPACE, false))?.parse("a?t", Some("contents"))
    });
    Ok(())
}

// Fuzzy queries should not be allowed"
fn test_custom_query_parser_fuzzy(_t: &mut TestQPHelper) -> Result<()> {
    expect_throws(QUERY_NODE_EXCEPTION, || {
        qp_test_parser(mock_analyzer(&WHITESPACE, false))?.parse("xunit~", Some("contents"))
    });
    Ok(())
}

// too many boolean clauses, so ParseException is expected
fn test_boolean_query(_t: &mut TestQPHelper) -> Result<()> {
    IndexSearcher::set_max_clause_count(2)?;
    expect_throws(QUERY_NODE_EXCEPTION, || {
        let mut qp = StandardQueryParser::new();
        qp.set_analyzer(Some(mock_analyzer(&WHITESPACE, false)))?;

        qp.parse("one two three", Some("field"))
    });
    Ok(())
}

/// This test differs from TestPrecedenceQueryParser
fn test_precedence(_t: &mut TestQPHelper) -> Result<()> {
    let mut qp = StandardQueryParser::new();
    qp.set_analyzer(Some(mock_analyzer(&WHITESPACE, false)))?;

    let query1 = parse(&qp, "A AND B OR C AND D", "field")?;
    let query2 = parse(&qp, "+A +B +C +D", "field")?;

    assert_query_eq(&*query1, &*query2);
    Ok(())
}

fn test_star_parsing(_t: &mut TestQPHelper) -> Result<()> {
    // (the Java body is commented out)
    Ok(())
}

/// `name -> null`: no regular expression of the test names an automaton.
struct NullProvider;

impl AutomatonProvider for NullProvider {
    fn get_automaton(&self, _name: &str) -> Result<Automaton> {
        Err(LuceneError::null_pointer())
    }
}

fn test_regexps(_t: &mut TestQPHelper) -> Result<()> {
    let mut qp = StandardQueryParser::new();
    qp.set_analyzer(Some(mock_analyzer(&WHITESPACE, true)))?;
    let df = "field";
    let q = regexp("field", "[a-z][123]")?;
    assert_query_eq(&*q, &*parse(&qp, "/[a-z][123]/", df)?);
    assert_query_eq(&*q, &*parse(&qp, "/[A-Z][123]/", df)?);
    assert_query_eq(
        &BoostQuery::new(Arc::clone(&q), 0.5)?,
        &*parse(&qp, "/[A-Z][123]/^0.5", df)?,
    );

    let escaped = regexp("field", "[a-z]\\/[123]")?;
    assert_query_eq(&*escaped, &*parse(&qp, "/[a-z]\\/[123]/", df)?);
    let escaped2 = regexp("field", "[a-z]\\*[123]")?;
    assert_query_eq(&*escaped2, &*parse(&qp, "/[a-z]\\*[123]/", df)?);

    let complex = bq(vec![
        (regexp("field", "[a-z]\\/[123]")?, Occur::MUST),
        (tq("path", "/etc/init.d/"), Occur::MUST),
        (tq("field", "/etc/init[.]d/lucene/"), Occur::SHOULD),
    ])?;
    assert_query_eq(
        &*complex,
        &*parse(
            &qp,
            "/[a-z]\\/[123]/ AND path:\"/etc/init.d/\" OR \"/etc\\/init\\[.\\]d/lucene/\" ",
            df,
        )?,
    );

    let re = regexp("field", "http.*")?;
    assert_query_eq(&*re, &*parse(&qp, "field:/http.*/", df)?);
    assert_query_eq(&*re, &*parse(&qp, "/http.*/", df)?);

    let re = regexp("field", "http~0.5")?;
    assert_query_eq(&*re, &*parse(&qp, "field:/http~0.5/", df)?);
    assert_query_eq(&*re, &*parse(&qp, "/http~0.5/", df)?);

    // fuzzy op doesn't apply to regexps.
    assert_query_node_exception("/http/~2")?;

    let re = regexp("field", "boo")?;
    assert_query_eq(&*re, &*parse(&qp, "field:/boo/", df)?);
    assert_query_eq(&*re, &*parse(&qp, "/boo/", df)?);

    assert_query_eq(&*tq("field", "/boo/"), &*parse(&qp, "\"/boo/\"", df)?);
    assert_query_eq(&*tq("field", "/boo/"), &*parse(&qp, "\\/boo\\/", df)?);

    let two = bq(vec![
        (regexp("field", "foo")?, Occur::SHOULD),
        (regexp("field", "bar")?, Occur::SHOULD),
    ])?;
    assert_query_eq(&*two, &*parse(&qp, "field:/foo/ field:/bar/", df)?);
    assert_query_eq(&*two, &*parse(&qp, "/foo/ /bar/", df)?);

    qp.set_multi_term_rewrite_method(Some(scoring_boolean_rewrite()))?;
    let q: Arc<dyn Query> = Arc::new(RegexpQuery::with_everything(
        term("field", "[a-z][123]"),
        RegExp::ALL,
        0,
        &NullProvider,
        DEFAULT_DETERMINIZE_WORK_LIMIT,
        scoring_boolean_rewrite(),
        true,
    )?);
    assert_query_eq(
        &BoostQuery::new(q, 0.5)?,
        &*parse(&qp, "/[A-Z][123]/^0.5", df)?,
    );
    let parsed = parse(&qp, "/[A-Z][123]/^0.5", df)?;
    let inner = cast::<BoostQuery>(&*parsed, BOOST_QUERY).get_query();
    assert_rewrite_method(
        &scoring_boolean_rewrite(),
        cast::<RegexpQuery>(&*inner, "org.apache.lucene.search.RegexpQuery"),
    );
    Ok(())
}

fn test_stopwords(_t: &mut TestQPHelper) -> Result<()> {
    let mut qp = StandardQueryParser::new();
    let stop_set = Arc::new(CharacterRunAutomaton::new(
        RegExp::new("the|foo")?.to_automaton()?,
    )?);
    qp.set_analyzer(Some(mock_analyzer_filter(&SIMPLE, true, &stop_set)))?;

    let result = qp.parse("a:the OR a:foo", Some("a"))?;
    Assert::assert_true_msg("result is null and it shouldn't be", result.is_some());
    let result = non_null(result)?;
    Assert::assert_true_msg(
        "result is not a MatchNoDocsQuery",
        is::<MatchNoDocsQuery>(&*result),
    );
    let result = qp.parse("a:woo OR a:the", Some("a"))?;
    Assert::assert_true_msg("result is null and it shouldn't be", result.is_some());
    let result = non_null(result)?;
    Assert::assert_true_msg("result is not a TermQuery", is::<TermQuery>(&*result));
    let result = parse(
        &qp,
        "(fieldX:xxxxx OR fieldy:xxxxxxxx)^2 AND (fieldx:the OR fieldy:foo)",
        "a",
    )?;
    let expected = bq(vec![
        (tq("fieldX", "xxxxx"), Occur::SHOULD),
        (tq("fieldy", "xxxxxxxx"), Occur::SHOULD),
    ])?;
    let expected = BoostQuery::new(expected, 2.0)?;
    assert_query_eq(&expected, &*result);
    Ok(())
}

fn test_position_increment(_t: &mut TestQPHelper) -> Result<()> {
    let mut qp = StandardQueryParser::new();
    qp.set_analyzer(Some(mock_analyzer_filter(&SIMPLE, true, &ENGLISH_STOPSET)))?;

    qp.set_enable_position_increments(true)?;

    let qtxt = "\"the words in poisitions pos02578 are stopped in this phrasequery\"";
    // 0 2 5 7 8
    let expected_positions = [1, 3, 4, 6, 9];
    let q = parse(&qp, qtxt, "a")?;
    let pq = cast::<PhraseQuery>(&*q, "org.apache.lucene.search.PhraseQuery");
    let t = pq.get_terms();
    let pos = pq.get_positions();
    for i in 0..t.len() {
        Assert::assert_equals_msg(
            Some(&format!("term {i} = {} has wrong term-position!", t[i])),
            expected_positions[i],
            pos[i],
        );
    }
    Ok(())
}

fn test_match_all_docs(_t: &mut TestQPHelper) -> Result<()> {
    let mut qp = StandardQueryParser::new();
    qp.set_analyzer(Some(mock_analyzer(&WHITESPACE, false)))?;

    assert_query_eq(
        &MatchAllDocsQuery::instance(),
        &*parse(&qp, "*:*", "field")?,
    );
    assert_query_eq(
        &MatchAllDocsQuery::instance(),
        &*parse(&qp, "(*:*)", "field")?,
    );
    let q = parse(&qp, "+*:* -*:*", "field")?;
    let bq = cast::<BooleanQuery>(&*q, "org.apache.lucene.search.BooleanQuery");
    for c in bq.clauses() {
        Assert::assert_true(
            c.query().java_class_name() == "org.apache.lucene.search.MatchAllDocsQuery",
        );
    }
    Ok(())
}

fn test_multi_phrase_query(_t: &mut TestQPHelper) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(Some(canned_analyzer())),
    )?;
    let mut doc = Document::new();
    doc.add(LuceneTestCase::new_text_field("field", "", Store::NO));
    w.add_document(&doc)?;
    let r = directory_reader::open_with_writer(&*w)?;
    let s = LuceneTestCase::new_searcher(Arc::clone(&r));

    let q = parse(
        &StandardQueryParser::with_analyzer(Some(canned_analyzer())),
        "\"a\"",
        "field",
    )?;
    Assert::assert_true(is::<MultiPhraseQuery>(&*q));
    Assert::assert_equals(1i64, s.search(q, 10)?.total_hits.value());
    r.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

fn test_regex_query_parsing(_t: &mut TestQPHelper) -> Result<()> {
    let fields = [Some(CharSequence::from("b")), Some(CharSequence::from("t"))];

    let mut parser = StandardQueryParser::new();
    parser.set_multi_fields(Some(Arc::new(RwLock::new(fields.to_vec()))))?;
    parser.set_default_operator(Some(Operator::And))?;
    parser.set_analyzer(Some(mock_analyzer_default()))?;

    let exp = bq(vec![
        // TODO spezification? was "MUST"
        (regexp("b", "ab.+")?, Occur::SHOULD),
        // TODO spezification? was "MUST"
        (regexp("t", "ab.+")?, Occur::SHOULD),
    ])?;

    assert_query_eq(&*exp, &*non_null(parser.parse("/ab.+/", None)?)?);

    let regexp_query_exp = regexp("test", "[abc]?[0-9]")?;

    assert_query_eq(
        &*regexp_query_exp,
        &*non_null(parser.parse("test:/[abc]?[0-9]/", None)?)?,
    );
    Ok(())
}
