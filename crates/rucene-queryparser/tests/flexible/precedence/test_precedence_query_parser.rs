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

//! Port of `TestPrecedenceQueryParser.java`: the `PrecedenceQueryParser`
//! against the tests of `QueryParserTestBase`, some adjusted to fit the
//! precedence requirement, plus some precedence test cases.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::{AfterClass, Assert, BeforeClass};
use rucene::analysis::{Analyzer, TokenFilter, TokenFilterLogic, TokenStream};
use rucene::document::{DateTools, Resolution};
use rucene::error::{LuceneError, Result};
use rucene::search::boolean_query::BooleanQuery;
use rucene::search::boost_query::BoostQuery;
use rucene::search::fuzzy_query::{self, FuzzyQuery};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::match_no_docs_query::MatchNoDocsQuery;
use rucene::search::phrase_query::PhraseQuery;
use rucene::search::prefix_query::PrefixQuery;
use rucene::search::term_query::TermQuery;
use rucene::search::term_range_query::TermRangeQuery;
use rucene::search::wildcard_query::WildcardQuery;
use rucene::search::Query;
use rucene::util::attribute::AttributeSource;
use rucene::util::automaton::{Automata, CharacterRunAutomaton};
use rucene_queryparser::flexible::precedence::precedence_query_parser::PrecedenceQueryParser;
use rucene_queryparser::flexible::standard::common_query_parser_configuration::CommonQueryParserConfiguration;
use rucene_queryparser::flexible::standard::config::standard_query_config_handler::Operator;
use rucene_queryparser::jdk::calendar::{self, Calendar};
use rucene_queryparser::jdk::char_sequence::CharSequence;
use rucene_queryparser::jdk::date_format;
use rucene_queryparser::jdk::locale::Locale;
use rucene_queryparser::jdk::time_zone::TimeZone;
use rucene_test_framework::analysis::attributes::{char_term_id, offset_id, Att};
use rucene_test_framework::analysis::mock_token_filter::ENGLISH_STOPSET;
use rucene_test_framework::analysis::mock_tokenizer::{SIMPLE, WHITESPACE};
use rucene_test_framework::analysis::{components, shared, sink};

use crate::flexible::standard::test_standard_qp::assert_operator;
use crate::qp_support::query_parser_test_base::{
    escape_date_string, get_date_millis, get_date_str, mock_analyzer, mock_analyzer_default,
    mock_analyzer_filter, mock_tokenizer, FixtureTest, FnAnalyzer,
};
use crate::qp_support::search_support::{fixture_class, ftest, set_up, static_hook, tear_down};
use crate::qp_support::{
    assert_query_eq, cast, expect_throws, is, jdk_default_hooks, to_s, STANDARD_PARSE_EXCEPTION,
};

const QUERY_NODE_EXCEPTION: &str = "org.apache.lucene.queryparser.flexible.core.QueryNodeException";
const QUERY_NODE_PARSE_EXCEPTION: &str =
    "org.apache.lucene.queryparser.flexible.core.QueryNodeParseException";
const BOOST_QUERY: &str = "org.apache.lucene.search.BoostQuery";

// ---------------------------------------------------------------------------
// Analyzers
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

/// The logic of `TestPrecedenceQueryParser.QPTestFilter`: discards the token
/// 'stop' and expands the token 'phrase' into 'phrase1 phrase2'. Unlike
/// `TestQPHelper`'s, it does not clear the attributes before `phrase2`.
#[derive(Debug)]
struct QpTestFilterLogic {
    in_phrase: bool,
    saved_start: i32,
    saved_end: i32,
    term_att: Att,
    offset_att: Att,
}

impl TokenFilterLogic for QpTestFilterLogic {
    fn increment_token(
        &mut self,
        _source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if self.in_phrase {
            self.in_phrase = false;
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

/// `new QPTestAnalyzer()`: filters `MockTokenizer` with `QPTestFilter`.
fn qp_test_analyzer() -> Arc<dyn Analyzer> {
    FnAnalyzer::new(
        "org.apache.lucene.queryparser.flexible.precedence.TestPrecedenceQueryParser$QPTestAnalyzer",
        |_| {
            let tokenizer = mock_tokenizer(&SIMPLE, true)?;
            let mut input = shared(&tokenizer);
            let term_att = Att::add(input.attribute_source_mut(), char_term_id())?;
            let offset_att = Att::add(input.attribute_source_mut(), offset_id())?;
            let filter = TokenFilter::new(
                input,
                QpTestFilterLogic {
                    in_phrase: false,
                    saved_start: 0,
                    saved_end: 0,
                    term_att,
                    offset_att,
                },
            );
            Ok(components(tokenizer, sink(filter)))
        },
    )
    .arc()
}

// ---------------------------------------------------------------------------
// The class and its helpers
// ---------------------------------------------------------------------------

/// The instance of `TestPrecedenceQueryParser`.
#[derive(Default)]
pub struct TestPrecedenceQueryParser {
    original_max_clauses: i32,
}

/// `TestPrecedenceQueryParser`.
pub fn class_def() -> Arc<Class> {
    let mut methods: Vec<Method> = vec![
        static_hook("beforeClass", BeforeClass, before_class),
        static_hook("afterClass", AfterClass, after_class),
        set_up(test_set_up),
        tear_down(test_tear_down),
    ];
    methods.extend(jdk_default_hooks());
    type F = TestPrecedenceQueryParser;
    let tests: Vec<(&str, FixtureTest<F>)> = vec![
        ("testSimple", test_simple),
        ("testPunct", test_punct),
        ("testSlop", test_slop),
        ("testNumber", test_number),
        ("testWildcard", test_wildcard),
        ("testQPA", test_qpa),
        ("testRange", test_range),
        ("testDateRange", test_date_range),
        ("testEscaped", test_escaped),
        (
            "testTabNewlineCarriageReturn",
            test_tab_newline_carriage_return,
        ),
        ("testSimpleDAO", test_simple_dao),
        ("testBoost", test_boost),
        ("testException", test_exception),
        ("testBooleanQuery", test_boolean_query),
        ("testNOT", test_not),
        ("testPrecedence", test_precedence),
    ];
    for (name, body) in tests {
        methods.push(ftest(name, body));
    }
    fixture_class::<TestPrecedenceQueryParser>(
        "org.apache.lucene.queryparser.flexible.precedence.TestPrecedenceQueryParser",
        methods,
    )
}

/// `setUp()`.
fn test_set_up(t: &mut TestPrecedenceQueryParser) -> Result<()> {
    t.original_max_clauses = IndexSearcher::get_max_clause_count();
    Ok(())
}

/// `tearDown()`.
fn test_tear_down(t: &mut TestPrecedenceQueryParser) -> Result<()> {
    IndexSearcher::set_max_clause_count(t.original_max_clauses)
}

/// `qp.parse(query, field)` dereferenced.
fn parse(qp: &PrecedenceQueryParser, query: &str, field: &str) -> Result<Arc<dyn Query>> {
    qp.parse(query, Some(field))?
        .ok_or_else(LuceneError::null_pointer)
}

/// `getParser(Analyzer a)`.
fn get_parser(a: Option<Arc<dyn Analyzer>>) -> Result<PrecedenceQueryParser> {
    let a = a.unwrap_or_else(|| mock_analyzer(&SIMPLE, true));
    let mut qp = PrecedenceQueryParser::new()?;
    qp.set_analyzer(Some(a))?;
    qp.set_default_operator(Some(Operator::Or))?;
    Ok(qp)
}

/// `getQuery(String query, Analyzer a)`.
fn get_query(query: &str, a: Option<Arc<dyn Analyzer>>) -> Result<Arc<dyn Query>> {
    parse(&get_parser(a)?, query, "field")
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

/// `assertWildcardQueryEquals(String query, String result)`.
fn assert_wildcard_query_equals(query: &str, result: &str) -> Result<()> {
    let qp = get_parser(None)?;
    let q = parse(&qp, query, "field")?;
    let s = q.to_query_string("field");
    if s != result {
        Assert::fail(&format!(
            "WildcardQuery /{query}/ yielded /{s}/, expecting /{result}/"
        ));
    }
    Ok(())
}

/// `getQueryDOA(String query, Analyzer a)`.
fn get_query_doa(query: &str, a: Option<Arc<dyn Analyzer>>) -> Result<Arc<dyn Query>> {
    let a = a.unwrap_or_else(|| mock_analyzer(&SIMPLE, true));
    let mut qp = PrecedenceQueryParser::new()?;
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

// ---------------------------------------------------------------------------
// Test methods
// ---------------------------------------------------------------------------

fn test_simple(_t: &mut TestPrecedenceQueryParser) -> Result<()> {
    assert_query_equals("term term term", None, "term term term")?;
    assert_query_equals("türm term term", None, "türm term term")?;
    assert_query_equals("ümlaut", None, "ümlaut")?;

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

    let mut qp = PrecedenceQueryParser::new()?;
    qp.set_analyzer(Some(mock_analyzer_default()))?;
    // make sure OR is the default:
    assert_operator(Operator::Or, qp.get_default_operator()?);
    qp.set_default_operator(Some(Operator::And))?;
    assert_operator(Operator::And, qp.get_default_operator()?);
    qp.set_default_operator(Some(Operator::Or))?;
    assert_operator(Operator::Or, qp.get_default_operator()?);

    assert_query_equals("a OR !b", None, "a -b")?;
    assert_query_equals("a OR ! b", None, "a -b")?;
    assert_query_equals("a OR -b", None, "a -b")?;
    Ok(())
}

fn test_punct(_t: &mut TestPrecedenceQueryParser) -> Result<()> {
    let a = mock_analyzer(&WHITESPACE, false);
    assert_query_equals("a&b", Some(Arc::clone(&a)), "a&b")?;
    assert_query_equals("a&&b", Some(Arc::clone(&a)), "a&&b")?;
    assert_query_equals(".NET", Some(a), ".NET")?;
    Ok(())
}

fn test_slop(_t: &mut TestPrecedenceQueryParser) -> Result<()> {
    assert_query_equals("\"term germ\"~2", None, "\"term germ\"~2")?;
    assert_query_equals("\"term germ\"~2 flork", None, "\"term germ\"~2 flork")?;
    assert_query_equals("\"term\"~2", None, "term")?;
    assert_query_equals("\" \"~2 germ", None, "germ")?;
    assert_query_equals("\"term germ\"~2^2", None, "(\"term germ\"~2)^2.0")?;
    Ok(())
}

fn test_number(_t: &mut TestPrecedenceQueryParser) -> Result<()> {
    // The numbers go away because SimpleAnalzyer ignores them
    assert_match_no_docs_query_str("3", None)?;
    assert_query_equals("term 1.0 1 2", None, "term")?;
    assert_query_equals("term term1 term2", None, "term term term")?;

    let a = mock_analyzer_default();
    assert_query_equals("3", Some(Arc::clone(&a)), "3")?;
    assert_query_equals("term 1.0 1 2", Some(Arc::clone(&a)), "term 1.0 1 2")?;
    assert_query_equals("term term1 term2", Some(a), "term term1 term2")?;
    Ok(())
}

fn test_wildcard(_t: &mut TestPrecedenceQueryParser) -> Result<()> {
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
    expect_throws(STANDARD_PARSE_EXCEPTION, || {
        get_query("term~1.1", None) // value > 1, throws exception
    });
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
    assert_wildcard_query_equals("[A TO C]", "[a TO c]")?;
    Ok(())
}

fn test_qpa(_t: &mut TestPrecedenceQueryParser) -> Result<()> {
    let qpa = || Some(qp_analyzer());
    assert_query_equals("term term term", qpa(), "term term term")?;
    assert_query_equals("term +stop term", qpa(), "term term")?;
    assert_query_equals("term -stop term", qpa(), "term term")?;
    assert_query_equals("drop AND stop AND roll", qpa(), "+drop +roll")?;
    assert_query_equals("term phrase term", qpa(), "term (phrase1 phrase2) term")?;
    // note the parens in this next assertion differ from the original
    // QueryParser behavior
    assert_query_equals(
        "term AND NOT phrase term",
        qpa(),
        "(+term -(phrase1 phrase2)) term",
    )?;
    assert_match_no_docs_query_str("stop", qpa())?;
    assert_match_no_docs_query_str("stop OR stop AND stop", qpa())?;
    Assert::assert_true(is::<BooleanQuery>(&*get_query("term term term", qpa())?));
    Assert::assert_true(is::<TermQuery>(&*get_query("term +stop", qpa())?));
    Ok(())
}

fn test_range(_t: &mut TestPrecedenceQueryParser) -> Result<()> {
    assert_query_equals("[ a TO z]", None, "[a TO z]")?;
    Assert::assert_true(is::<TermRangeQuery>(&*get_query("[ a TO z]", None)?));
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

/// `public String getDate(String s)`: the date of `s` (the default
/// locale's `SHORT` format) at `DAY` resolution (unused by the tests, as in
/// Lucene).
#[allow(dead_code)]
fn get_date_day(s: &str) -> Result<String> {
    // we use the default Locale since LuceneTestCase randomizes it
    let df = date_format::get_date_instance(date_format::SHORT, &Locale::get_default())?;
    Ok(DateTools::time_to_string(
        df.parse(s)?.get_time(),
        Resolution::DAY,
    ))
}

/// `getLocalizedDate(int year, int month, int day, boolean
/// extendLastDate)`: unlike `TestQPHelper`'s, the calendar is not cleared,
/// so its time of day is the current one unless extended.
fn get_localized_date(year: i32, month: i32, day: i32, extend_last_date: bool) -> Result<String> {
    // we use the default Locale/TZ since LuceneTestCase randomizes it
    let locale = Locale::get_default();
    let df = date_format::get_date_instance(date_format::SHORT, &locale)?;
    let mut calendar = Calendar::new_gregorian(&TimeZone::get_default(), &locale)?;
    calendar.set_date(year, month, day);
    if extend_last_date {
        calendar.set(calendar::HOUR_OF_DAY, 23);
        calendar.set(calendar::MINUTE, 59);
        calendar.set(calendar::SECOND, 59);
        calendar.set(calendar::MILLISECOND, 999);
    }
    df.format(calendar.get_time()?)
}

fn test_date_range(_t: &mut TestPrecedenceQueryParser) -> Result<()> {
    let start_date = get_localized_date(2002, 1, 1, false)?;
    let end_date = get_localized_date(2002, 1, 4, false)?;
    // we use the default Locale/TZ since LuceneTestCase randomizes it
    let mut end_date_expected =
        Calendar::new_gregorian(&TimeZone::get_default(), &Locale::get_default())?;
    end_date_expected.set_date_time_seconds(2002, 1, 4, 23, 59, 59);
    end_date_expected.set(calendar::MILLISECOND, 999);
    let default_field = "default";
    let month_field = "month";
    let hour_field = "hour";
    let mut qp = PrecedenceQueryParser::with_analyzer(Some(mock_analyzer_default()))?;

    let field_map: Arc<RwLock<HashMap<CharSequence, Resolution>>> =
        Arc::new(RwLock::new(HashMap::new()));
    // set a field specific date resolution
    field_map
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .insert(CharSequence::from(month_field), Resolution::MONTH);
    qp.set_date_resolution_map(Some(Arc::clone(&field_map)))?;

    // set default date resolution to MILLISECOND
    qp.set_date_resolution(Some(Resolution::MILLISECOND))?;

    // set second field specific date resolution
    field_map
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .insert(CharSequence::from(hour_field), Resolution::HOUR);
    qp.set_date_resolution_map(Some(Arc::clone(&field_map)))?;

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

    // verify if field specific date resolutions are used for these two fields
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

/// `assertQueryEquals(PrecedenceQueryParser qp, String field, String query,
/// String result)`.
fn assert_query_equals_qp(
    qp: &PrecedenceQueryParser,
    field: &str,
    query: &str,
    result: &str,
) -> Result<()> {
    let q = parse(qp, query, field)?;
    check(query, &q.to_query_string(field), result);
    Ok(())
}

/// `assertDateRangeQueryEquals(PrecedenceQueryParser qp, String field,
/// String startDate, String endDate, Date endDateInclusive, Resolution
/// resolution)`.
fn assert_date_range_query_equals(
    qp: &PrecedenceQueryParser,
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

fn test_escaped(_t: &mut TestPrecedenceQueryParser) -> Result<()> {
    let a = mock_analyzer(&WHITESPACE, false);
    let aa = || Some(Arc::clone(&a));

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

    assert_query_equals("[ a\\- TO a\\+ ]", None, "[a- TO a+]")?;
    assert_query_equals("[ a\\: TO a\\~ ]", None, "[a: TO a~]")?;
    assert_query_equals("[ a\\\\ TO a\\* ]", None, "[a\\ TO a*]")?;
    Ok(())
}

fn test_tab_newline_carriage_return(_t: &mut TestPrecedenceQueryParser) -> Result<()> {
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

fn test_simple_dao(_t: &mut TestPrecedenceQueryParser) -> Result<()> {
    assert_query_equals_doa("term term term", None, "+term +term +term")?;
    assert_query_equals_doa("term +term term", None, "+term +term +term")?;
    assert_query_equals_doa("term term +term", None, "+term +term +term")?;
    assert_query_equals_doa("term +term +term", None, "+term +term +term")?;
    assert_query_equals_doa("-term term term", None, "-term +term +term")?;
    Ok(())
}

fn test_boost(_t: &mut TestPrecedenceQueryParser) -> Result<()> {
    let stop_set = Arc::new(CharacterRunAutomaton::new(Automata::make_string("on"))?);
    let one_stop_analyzer = mock_analyzer_filter(&SIMPLE, true, &stop_set);

    let mut qp = PrecedenceQueryParser::new()?;
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

    let q = get_parser(Some(mock_analyzer_filter(&SIMPLE, true, &ENGLISH_STOPSET)))?
        .parse("the^3", Some("field"))?;
    Assert::assert_true(q.is_some()); // assertNotNull(q)
    Ok(())
}

fn test_exception(_t: &mut TestPrecedenceQueryParser) -> Result<()> {
    expect_throws(QUERY_NODE_PARSE_EXCEPTION, || {
        assert_query_equals("\"some phrase", None, "abc")
    });
    Ok(())
}

// ParseException expected due to too many boolean clauses
fn test_boolean_query(_t: &mut TestPrecedenceQueryParser) -> Result<()> {
    IndexSearcher::set_max_clause_count(2)?;
    expect_throws(QUERY_NODE_EXCEPTION, || {
        get_parser(Some(mock_analyzer(&WHITESPACE, false)))?.parse("one two three", Some("field"))
    });
    Ok(())
}

// LUCENE-792
fn test_not(_t: &mut TestPrecedenceQueryParser) -> Result<()> {
    let a = mock_analyzer(&WHITESPACE, false);
    assert_query_equals("NOT foo AND bar", Some(a), "-foo +bar")
}

/// This test differs from the original QueryParser, showing how the
/// precedence issue has been corrected.
fn test_precedence(_t: &mut TestPrecedenceQueryParser) -> Result<()> {
    let mut parser = get_parser(Some(mock_analyzer(&WHITESPACE, false)))?;
    let pairs = |parser: &PrecedenceQueryParser, a: &str, b: &str| -> Result<()> {
        let query1 = parse(parser, a, "field")?;
        let query2 = parse(parser, b, "field")?;
        assert_query_eq(&*query1, &*query2);
        Ok(())
    };
    pairs(&parser, "A AND B OR C AND D", "(A AND B) OR (C AND D)")?;

    pairs(&parser, "A OR B C", "(A B) C")?;

    pairs(&parser, "A AND B C", "(+A +B) C")?;

    pairs(&parser, "A AND NOT B", "+A -B")?;

    pairs(&parser, "A OR NOT B", "A -B")?;

    pairs(&parser, "A OR NOT B AND C", "A (-B +C)")?;

    parser.set_default_operator(Some(Operator::And))?;
    pairs(&parser, "A AND B OR C AND D", "(A AND B) OR (C AND D)")?;

    pairs(&parser, "A AND B C", "(A B) C")?;

    pairs(&parser, "A AND B C", "(+A +B) C")?;

    pairs(&parser, "A AND NOT B", "+A -B")?;

    pairs(&parser, "A AND NOT B OR C", "(+A -B) OR C")?;
    Ok(())
}
