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

//! Port of `QueryParserTestBase.java` of Lucene 10.5.0
//! `lucene/queryparser/src/test/org/apache/lucene/queryparser/util/`: the
//! abstract base class of the classic `TestQueryParser` (rmp #484) and of
//! the flexible `TestStandardQP` (rmp #491).
//!
//! Java's abstract class is the trait [`QueryParserTestBase`]: its abstract
//! methods are the trait's required methods, its state
//! ([`QueryParserTestBaseState`]) lives in the subclass fixture, and its test
//! methods are generic functions over the trait, declared on the subclass's
//! Java class by [`base_methods`] (minus the ones the subclass overrides).
//! `getParserConfig` returns the parser as [`QueryParserTestBase::Config`]
//! (`dyn QueryParserOps` for the classic parsers), the
//! `CommonQueryParserConfiguration` the Java methods take and cast back.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Method;
use randomizedtesting::junit::{AfterClass, Assert, BeforeClass};
use rucene::analysis::{
    new_lower_case_filter, Analyzer, AnalyzerState, CharReader, GlobalReuseStrategy, TokenFilter,
    TokenFilterLogic, TokenStream, TokenStreamComponents, Tokenizer, TokenizerLogic,
};
use rucene::document::{DateTools, Document, Resolution, Store};
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
use rucene::search::multi_term_query::{
    constant_score_blended_rewrite, scoring_boolean_rewrite, MultiTermQuery,
};
use rucene::search::phrase_query::{Builder as PhraseQueryBuilder, PhraseQuery};
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
use rucene_queryparser::classic::query_parser_base;
use rucene_queryparser::flexible::standard::common_query_parser_configuration::CommonQueryParserConfiguration;
use rucene_queryparser::jdk::calendar::{self, Calendar};
use rucene_queryparser::jdk::date_format;
use rucene_queryparser::jdk::locale::Locale;
use rucene_queryparser::jdk::time_zone::TimeZone;
use rucene_test_framework::analysis::attributes::{char_term_id, offset_id, Att};
use rucene_test_framework::analysis::mock_analyzer::MockAnalyzer;
use rucene_test_framework::analysis::mock_synonym_filter::MockSynonymFilter;
use rucene_test_framework::analysis::mock_token_filter::ENGLISH_STOPSET;
use rucene_test_framework::analysis::mock_tokenizer::{MockTokenizer, SIMPLE, WHITESPACE};
use rucene_test_framework::analysis::{components, shared, sink, tokenizer_components};
use rucene_test_framework::util::LuceneTestCase;

use super::core_support::{random, verbose};
use super::search_support::{ftest, set_up, static_hook, tear_down, term, tq};
use super::{
    assert_query_eq, cast, catch_java, expect_throws, is, jdk_default_hooks, to_s,
    CLASSIC_PARSE_EXCEPTION, STANDARD_PARSE_EXCEPTION,
};

// ---------------------------------------------------------------------------
// Analyzers
// ---------------------------------------------------------------------------

/// An analyzer whose `createComponents` (and optionally `normalize`) is a
/// closure: the anonymous and nested `Analyzer` subclasses of the tests.
pub struct FnAnalyzer {
    state: AnalyzerState,
    class: &'static str,
    create: CreateFn,
    normalize: Option<NormalizeFn>,
}

/// The body of `createComponents(String)`.
type CreateFn = Box<dyn Fn(Option<&str>) -> Result<TokenStreamComponents> + Send + Sync>;

/// The body of an overridden `normalize(String, TokenStream)`.
pub type NormalizeFn =
    Box<dyn Fn(Option<&str>, Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> + Send + Sync>;

impl std::fmt::Debug for FnAnalyzer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FnAnalyzer")
            .field("class", &self.class)
            .finish_non_exhaustive()
    }
}

impl FnAnalyzer {
    /// `new Analyzer() { createComponents = create }` of Java class `class`
    /// (the default `GLOBAL_REUSE_STRATEGY`).
    pub fn new(
        class: &'static str,
        create: impl Fn(Option<&str>) -> Result<TokenStreamComponents> + Send + Sync + 'static,
    ) -> Self {
        FnAnalyzer {
            state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
            class,
            create: Box::new(create),
            normalize: None,
        }
    }

    /// [`new`](Self::new) that also overrides `normalize`.
    pub fn with_normalize(
        class: &'static str,
        create: impl Fn(Option<&str>) -> Result<TokenStreamComponents> + Send + Sync + 'static,
        normalize: impl Fn(Option<&str>, Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        FnAnalyzer {
            normalize: Some(Box::new(normalize)),
            ..FnAnalyzer::new(class, create)
        }
    }

    /// As a shared analyzer handle.
    pub fn arc(self) -> Arc<dyn Analyzer> {
        Arc::new(self)
    }
}

impl Analyzer for FnAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        self.class
    }

    fn create_components(&self, field_name: Option<&str>) -> Result<TokenStreamComponents> {
        (self.create)(field_name)
    }

    fn normalize_token_stream(
        &self,
        field_name: Option<&str>,
        input: Box<dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        match &self.normalize {
            // A filter constructor that fails (an attribute the stream's
            // factory lacks) is an unchecked exception in Java.
            Some(f) => f(field_name, input)
                .unwrap_or_else(|e| rucene_test_framework::util::java_exceptions::rethrow_java(e)),
            None => input,
        }
    }
}

/// `new MockTokenizer(runAutomaton, lowerCase)` shared as a components'
/// source.
pub fn mock_tokenizer(
    run_automaton: &Arc<CharacterRunAutomaton>,
    lower_case: bool,
) -> Result<Arc<AtomicRefCell<MockTokenizer>>> {
    Ok(Arc::new(AtomicRefCell::new(MockTokenizer::new(
        Arc::clone(run_automaton),
        lower_case,
    )?)))
}

/// `new MockAnalyzer(random(), runAutomaton, lowerCase)`.
pub fn mock_analyzer(
    run_automaton: &Arc<CharacterRunAutomaton>,
    lower_case: bool,
) -> Arc<dyn Analyzer> {
    Arc::new(MockAnalyzer::with_automaton(
        &*random(),
        Arc::clone(run_automaton),
        lower_case,
    ))
}

/// `new MockAnalyzer(random(), runAutomaton, lowerCase, filter)`.
pub fn mock_analyzer_filter(
    run_automaton: &Arc<CharacterRunAutomaton>,
    lower_case: bool,
    filter: &Arc<CharacterRunAutomaton>,
) -> Arc<dyn Analyzer> {
    Arc::new(MockAnalyzer::with_filter(
        &*random(),
        Arc::clone(run_automaton),
        lower_case,
        Arc::clone(filter),
    ))
}

/// `new MockAnalyzer(random())`.
pub fn mock_analyzer_default() -> Arc<dyn Analyzer> {
    Arc::new(MockAnalyzer::new(&*random()))
}

/// The logic of `QueryParserTestBase.QPTestFilter`: discards the token
/// `stop` and expands the token `phrase` into `phrase1 phrase2`.
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
            self.term_att.char_term_mut().append_string(Some("phrase2"));
            self.offset_att
                .offset_mut()
                .set_offset(self.saved_start, self.saved_end);
            return Ok(true);
        } else {
            while input.increment_token()? {
                if self.term_att.term_string() == "phrase" {
                    self.in_phrase = true;
                    self.saved_start = self.offset_att.offset().start_offset();
                    self.saved_end = self.offset_att.offset().end_offset();
                    self.term_att.set_term("phrase1");
                    self.offset_att
                        .offset_mut()
                        .set_offset(self.saved_start, self.saved_end);
                    return Ok(true);
                } else if self.term_att.term_string() != "stop" {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
}

/// `new QPTestFilter(in)`.
pub fn qp_test_filter(mut input: Box<dyn TokenStream>) -> Result<TokenFilter<QpTestFilterLogic>> {
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

/// `new QPTestAnalyzer()`: `MockTokenizer(SIMPLE, true)` filtered by
/// `QPTestFilter`.
pub fn qp_test_analyzer() -> Arc<dyn Analyzer> {
    FnAnalyzer::new(
        "org.apache.lucene.queryparser.util.QueryParserTestBase$QPTestAnalyzer",
        |_| {
            let tokenizer = mock_tokenizer(&SIMPLE, true)?;
            let filter = qp_test_filter(shared(&tokenizer))?;
            Ok(components(tokenizer, sink(filter)))
        },
    )
    .arc()
}

/// `QueryParserTestBase.SimpleCJKTokenizer`: individual CJK chars as
/// terms, like StandardAnalyzer.
#[derive(Debug)]
pub struct SimpleCjkTokenizerLogic {
    term_att: Att,
}

impl TokenizerLogic for SimpleCjkTokenizerLogic {
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn CharReader,
    ) -> Result<bool> {
        // `int ch = input.read()`.
        let mut buf = [0u16; 1];
        let n = input.read(&mut buf)?;
        if n == 0 {
            return Ok(false);
        }
        source.clear_attributes();
        // `termAtt.setEmpty().append((char) ch)`: the port's term attribute
        // holds `char`s; no test input carries a lone surrogate.
        let c = char::from_u32(u32::from(buf[0])).unwrap_or('\u{FFFD}');
        let mut t = self.term_att.char_term_mut();
        t.set_empty();
        t.append_char(c);
        Ok(true)
    }
}

/// `new SimpleCJKTokenizer()` shared as a components' source.
pub fn simple_cjk_tokenizer() -> Result<Arc<AtomicRefCell<Tokenizer<SimpleCjkTokenizerLogic>>>> {
    let mut source =
        AttributeSource::new_with_factory(rucene::analysis::default_token_attribute_factory());
    let term_att = Att::add(&mut source, char_term_id())?;
    Ok(Arc::new(AtomicRefCell::new(Tokenizer::with_source(
        source,
        SimpleCjkTokenizerLogic { term_att },
    ))))
}

/// `new SimpleCJKAnalyzer()`.
pub fn simple_cjk_analyzer() -> Arc<dyn Analyzer> {
    FnAnalyzer::new(
        "org.apache.lucene.queryparser.util.QueryParserTestBase$SimpleCJKAnalyzer",
        |_| Ok(tokenizer_components(simple_cjk_tokenizer()?)),
    )
    .arc()
}

/// `new Analyzer1()`: whitespace+lowercase analyzer with synonyms.
pub fn analyzer1() -> Arc<dyn Analyzer> {
    FnAnalyzer::new(
        "org.apache.lucene.queryparser.util.QueryParserTestBase$Analyzer1",
        |_| {
            let tokenizer = mock_tokenizer(&WHITESPACE, true)?;
            let filter = MockSynonymFilter::new(shared(&tokenizer))?;
            Ok(components(tokenizer, sink(filter)))
        },
    )
    .arc()
}

/// `new Analyzer2()`: whitespace+lowercase analyzer without synonyms.
pub fn analyzer2() -> Arc<dyn Analyzer> {
    FnAnalyzer::new(
        "org.apache.lucene.queryparser.util.QueryParserTestBase$Analyzer2",
        |_| Ok(tokenizer_components(mock_tokenizer(&WHITESPACE, true)?)),
    )
    .arc()
}

/// The logic of `MockCollationFilter`: indexes terms as "collated" + term.
#[derive(Debug)]
struct MockCollationFilterLogic {
    term_att: Att,
}

impl TokenFilterLogic for MockCollationFilterLogic {
    fn increment_token(
        &mut self,
        _source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if input.increment_token()? {
            let term = self.term_att.term_string();
            let mut t = self.term_att.char_term_mut();
            t.set_empty();
            t.append_string(Some("collated"));
            t.append_string(Some(&term));
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

fn mock_collation_filter(
    mut input: Box<dyn TokenStream>,
) -> Result<TokenFilter<MockCollationFilterLogic>> {
    let term_att = Att::add(input.attribute_source_mut(), char_term_id())?;
    Ok(TokenFilter::new(
        input,
        MockCollationFilterLogic { term_att },
    ))
}

/// `new MockCollationAnalyzer()`.
fn mock_collation_analyzer() -> Arc<dyn Analyzer> {
    FnAnalyzer::with_normalize(
        "org.apache.lucene.queryparser.util.QueryParserTestBase$MockCollationAnalyzer",
        |_| {
            let tokenizer = mock_tokenizer(&WHITESPACE, true)?;
            let filter = mock_collation_filter(shared(&tokenizer))?;
            Ok(components(tokenizer, sink(filter)))
        },
        |_, input| {
            Ok(
                Box::new(mock_collation_filter(Box::new(new_lower_case_filter(
                    input,
                )?))?) as Box<dyn TokenStream>,
            )
        },
    )
    .arc()
}

/// `public static Analyzer qpAnalyzer`.
static QP_ANALYZER: Mutex<Option<Arc<dyn Analyzer>>> = Mutex::new(None);

/// `qpAnalyzer`.
pub fn qp_analyzer() -> Arc<dyn Analyzer> {
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

// ---------------------------------------------------------------------------
// The abstract class
// ---------------------------------------------------------------------------

/// The instance fields of `QueryParserTestBase`.
#[derive(Debug)]
pub struct QueryParserTestBaseState {
    /// `originalMaxClauses`.
    pub original_max_clauses: i32,
    /// `defaultField`.
    pub default_field: String,
}

impl Default for QueryParserTestBaseState {
    fn default() -> Self {
        QueryParserTestBaseState {
            original_max_clauses: 0,
            default_field: "field".to_string(),
        }
    }
}

/// `public abstract class QueryParserTestBase extends LuceneTestCase`.
pub trait QueryParserTestBase: Default + Send + 'static {
    /// The `CommonQueryParserConfiguration` `getParserConfig` returns: the
    /// subclass's parser type.
    type Config: CommonQueryParserConfiguration + ?Sized;

    /// The base class's fields.
    fn base(&self) -> &QueryParserTestBaseState;

    /// The base class's fields, for writing.
    fn base_mut(&mut self) -> &mut QueryParserTestBaseState;

    /// `getDefaultField()`.
    fn get_default_field(&self) -> String {
        self.base().default_field.clone()
    }

    /// `setDefaultField(String)`.
    fn set_default_field(&mut self, default_field: &str) {
        self.base_mut().default_field = default_field.to_string();
    }

    /// `getParserConfig(Analyzer)`.
    fn get_parser_config(&mut self, a: Option<Arc<dyn Analyzer>>) -> Result<Box<Self::Config>>;

    /// `setDefaultOperatorOR(CommonQueryParserConfiguration)`.
    fn set_default_operator_or(&mut self, cqp_c: &mut Self::Config) -> Result<()>;

    /// `setDefaultOperatorAND(CommonQueryParserConfiguration)`.
    fn set_default_operator_and(&mut self, cqp_c: &mut Self::Config) -> Result<()>;

    /// `setAutoGeneratePhraseQueries(CommonQueryParserConfiguration, boolean)`.
    fn set_auto_generate_phrase_queries(
        &mut self,
        cqp_c: &mut Self::Config,
        value: bool,
    ) -> Result<()>;

    /// `setDateResolution(CommonQueryParserConfiguration, CharSequence,
    /// DateTools.Resolution)`.
    fn set_date_resolution(
        &mut self,
        cqp_c: &mut Self::Config,
        field: &str,
        value: Resolution,
    ) -> Result<()>;

    /// `getQuery(String, CommonQueryParserConfiguration)`.
    fn get_query_config(&mut self, query: &str, cqp_c: &mut Self::Config)
        -> Result<Arc<dyn Query>>;

    /// `getQuery(String, Analyzer)`.
    fn get_query_analyzer(
        &mut self,
        query: &str,
        a: Option<Arc<dyn Analyzer>>,
    ) -> Result<Arc<dyn Query>>;

    /// `isQueryParserException(Exception)`.
    fn is_query_parser_exception(
        &self,
        exception: &randomizedtesting::jdk::exception::Throwable,
    ) -> bool;

    /// `this instanceof TestQueryParser` (`testRangeQueryRequiresTO`).
    fn is_test_query_parser(&self) -> bool;

    /// `if (qp instanceof QueryParser) ((QueryParser) qp).setSplitOnWhitespace(true)`
    /// (`testAutoGeneratePhraseQueriesOn`).
    fn set_split_on_whitespace_if_classic(&mut self, qp: &mut Self::Config) -> Result<()>;

    /// `getQuery(String)`: `getQuery(query, (Analyzer) null)`.
    fn get_query(&mut self, query: &str) -> Result<Arc<dyn Query>> {
        self.get_query_analyzer(query, None)
    }
}

/// A test method of a fixture `T`.
pub type FixtureTest<T> = fn(&mut T) -> Result<()>;

/// The test methods of `QueryParserTestBase`, its `setUp`/`tearDown` and
/// class hooks, as methods of the subclass whose fixture is `T`, without the
/// ones named in `overridden` (the subclass declares those).
pub fn base_methods<T: QueryParserTestBase>(overridden: &[&str]) -> Vec<Method> {
    let mut methods = vec![
        static_hook("beforeClass", BeforeClass, before_class),
        static_hook("afterClass", AfterClass, after_class),
        set_up(base_set_up::<T>),
        tear_down(base_tear_down::<T>),
    ];
    methods.extend(jdk_default_hooks());
    let tests: Vec<(&str, FixtureTest<T>)> = vec![
        ("testCJK", test_cjk::<T>),
        ("testCJKTerm", test_cjk_term::<T>),
        ("testCJKBoostedTerm", test_cjk_boosted_term::<T>),
        ("testCJKPhrase", test_cjk_phrase::<T>),
        ("testCJKBoostedPhrase", test_cjk_boosted_phrase::<T>),
        ("testCJKSloppyPhrase", test_cjk_sloppy_phrase::<T>),
        (
            "testAutoGeneratePhraseQueriesOn",
            test_auto_generate_phrase_queries_on::<T>,
        ),
        ("testSimple", test_simple::<T>),
        ("testOperatorVsWhitespace", test_operator_vs_whitespace::<T>),
        ("testPunct", test_punct::<T>),
        ("testSlop", test_slop::<T>),
        ("testNumber", test_number::<T>),
        ("testWildcard", test_wildcard::<T>),
        ("testLeadingWildcardType", test_leading_wildcard_type::<T>),
        ("testQPA", test_qpa::<T>),
        ("testRange", test_range::<T>),
        ("testRangeWithPhrase", test_range_with_phrase::<T>),
        (
            "testRangeQueryEndpointTO",
            test_range_query_endpoint_to::<T>,
        ),
        (
            "testRangeQueryRequiresTO",
            test_range_query_requires_to::<T>,
        ),
        ("testDateRange", test_date_range::<T>),
        ("testEscaped", test_escaped::<T>),
        (
            "testEscapedVsQuestionMarkAsWildcard",
            test_escaped_vs_question_mark_as_wildcard::<T>,
        ),
        ("testQueryStringEscaping", test_query_string_escaping::<T>),
        (
            "testTabNewlineCarriageReturn",
            test_tab_newline_carriage_return::<T>,
        ),
        ("testSimpleDAO", test_simple_dao::<T>),
        ("testBoost", test_boost::<T>),
        ("testException", test_exception::<T>),
        ("testBooleanQuery", test_boolean_query::<T>),
        ("testPrecedence", test_precedence::<T>),
        (
            "testParsesBracketsIfQuoted",
            test_parses_brackets_if_quoted::<T>,
        ),
        ("testEscapedWildcard", test_escaped_wildcard::<T>),
        ("testRegexps", test_regexps::<T>),
        ("testStopwords", test_stopwords::<T>),
        ("testPositionIncrement", test_position_increment::<T>),
        ("testMatchAllDocs", test_match_all_docs::<T>),
        ("testPositionIncrements", test_position_increments::<T>),
        ("testCollatedRange", test_collated_range::<T>),
        (
            "testDistanceAsEditsParsing",
            test_distance_as_edits_parsing::<T>,
        ),
        ("testPhraseQueryToString", test_phrase_query_to_string::<T>),
        (
            "testParseWildcardAndPhraseQueries",
            test_parse_wildcard_and_phrase_queries::<T>,
        ),
        (
            "testPhraseQueryPositionIncrements",
            test_phrase_query_position_increments::<T>,
        ),
        (
            "testMatchAllQueryParsing",
            test_match_all_query_parsing::<T>,
        ),
        ("testNestedAndClausesFoo", test_nested_and_clauses_foo::<T>),
    ];
    for (name, body) in tests {
        if !overridden.contains(&name) {
            methods.push(ftest(name, body));
        }
    }
    methods
}

/// `setUp()`: `super.setUp(); originalMaxClauses = IndexSearcher.getMaxClauseCount();`.
fn base_set_up<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    t.base_mut().original_max_clauses = IndexSearcher::get_max_clause_count();
    Ok(())
}

/// `tearDown()`: `IndexSearcher.setMaxClauseCount(originalMaxClauses); super.tearDown();`.
fn base_tear_down<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    IndexSearcher::set_max_clause_count(t.base().original_max_clauses)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// `assertQueryEquals(String query, Analyzer a, String result)`.
pub fn assert_query_equals<T: QueryParserTestBase>(
    t: &mut T,
    query: &str,
    a: Option<Arc<dyn Analyzer>>,
    result: &str,
) -> Result<()> {
    let q = t.get_query_analyzer(query, a)?;
    let s = q.to_query_string("field");
    if s != result {
        Assert::fail(&format!(
            "Query /{query}/ yielded /{s}/, expecting /{result}/"
        ));
    }
    Ok(())
}

/// `assertMatchNoDocsQuery(String queryString, Analyzer a)`.
pub fn assert_match_no_docs_query_str<T: QueryParserTestBase>(
    t: &mut T,
    query_string: &str,
    a: Option<Arc<dyn Analyzer>>,
) -> Result<()> {
    let q = t.get_query_analyzer(query_string, a)?;
    assert_match_no_docs_query(&*q);
    Ok(())
}

/// `assertMatchNoDocsQuery(Query query)`.
pub fn assert_match_no_docs_query(query: &dyn Query) {
    // good: a `MatchNoDocsQuery`, or a `BooleanQuery` without clauses
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

/// `assertQueryEquals(CommonQueryParserConfiguration cqpC, String field,
/// String query, String result)`.
pub fn assert_query_equals_config<T: QueryParserTestBase>(
    t: &mut T,
    cqp_c: &mut T::Config,
    field: &str,
    query: &str,
    result: &str,
) -> Result<()> {
    let q = t.get_query_config(query, cqp_c)?;
    let s = q.to_query_string(field);
    if s != result {
        Assert::fail(&format!(
            "Query /{query}/ yielded /{s}/, expecting /{result}/"
        ));
    }
    Ok(())
}

/// `assertEscapedQueryEquals(String query, Analyzer a, String result)`.
pub fn assert_escaped_query_equals(query: &str, _a: Option<Arc<dyn Analyzer>>, result: &str) {
    let escaped_query = query_parser_base::escape(query);
    if escaped_query != result {
        Assert::fail(&format!(
            "Query /{query}/ yielded /{escaped_query}/, expecting /{result}/"
        ));
    }
}

/// `assertWildcardQueryEquals(String query, String result, boolean
/// allowLeadingWildcard)`.
pub fn assert_wildcard_query_equals_allow<T: QueryParserTestBase>(
    t: &mut T,
    query: &str,
    result: &str,
    allow_leading_wildcard: bool,
) -> Result<()> {
    let mut cqp_c = t.get_parser_config(None)?;
    cqp_c.set_allow_leading_wildcard(allow_leading_wildcard)?;
    let q = t.get_query_config(query, &mut cqp_c)?;
    let s = q.to_query_string("field");
    if s != result {
        Assert::fail(&format!(
            "WildcardQuery /{query}/ yielded /{s}/, expecting /{result}/"
        ));
    }
    Ok(())
}

/// `assertWildcardQueryEquals(String query, String result)`.
pub fn assert_wildcard_query_equals<T: QueryParserTestBase>(
    t: &mut T,
    query: &str,
    result: &str,
) -> Result<()> {
    assert_wildcard_query_equals_allow(t, query, result, false)
}

/// `getQueryDOA(String query, Analyzer a)`.
pub fn get_query_doa<T: QueryParserTestBase>(
    t: &mut T,
    query: &str,
    a: Option<Arc<dyn Analyzer>>,
) -> Result<Arc<dyn Query>> {
    let a = a.unwrap_or_else(|| mock_analyzer(&SIMPLE, true));
    let mut qp = t.get_parser_config(Some(a))?;
    t.set_default_operator_and(&mut qp)?;
    t.get_query_config(query, &mut qp)
}

/// `assertQueryEqualsDOA(String query, Analyzer a, String result)`.
pub fn assert_query_equals_doa<T: QueryParserTestBase>(
    t: &mut T,
    query: &str,
    a: Option<Arc<dyn Analyzer>>,
    result: &str,
) -> Result<()> {
    let q = get_query_doa(t, query, a)?;
    let s = q.to_query_string("field");
    if s != result {
        Assert::fail(&format!(
            "Query /{query}/ yielded /{s}/, expecting /{result}/"
        ));
    }
    Ok(())
}

/// `assertParseException(String queryString)`.
pub fn assert_parse_exception<T: QueryParserTestBase>(t: &mut T, query_string: &str) -> Result<()> {
    assert_parse_exception_analyzer(t, query_string, None)
}

/// `assertParseException(String queryString, Analyzer a)`: `try {
/// getQuery(queryString, a); } catch (Exception expected) { if
/// (isQueryParserException(expected)) return; } fail(...)`; a throwable
/// that is no `Exception` propagates.
pub fn assert_parse_exception_analyzer<T: QueryParserTestBase>(
    t: &mut T,
    query_string: &str,
    a: Option<Arc<dyn Analyzer>>,
) -> Result<()> {
    match catch_java(|| t.get_query_analyzer(query_string, a)) {
        Ok(_) => {}
        Err(expected) if expected.is_instance("java.lang.Exception") => {
            if t.is_query_parser_exception(&expected) {
                return Ok(());
            }
        }
        Err(other) => other.throw(),
    }
    Assert::fail("ParseException expected, not thrown");
}

/// `assertEquals(X.class, q.getClass())`.
pub fn assert_class(expected: &str, q: &dyn Query) {
    Assert::assert_equals(
        format!("class {expected}"),
        format!("class {}", q.java_class_name()),
    );
}

fn bq(clauses: Vec<(Arc<dyn Query>, Occur)>) -> Result<Arc<dyn Query>> {
    let mut b = BooleanQueryBuilder::new();
    for (q, o) in clauses {
        b.add(q, o)?;
    }
    Ok(Arc::new(b.build()))
}

// ---------------------------------------------------------------------------
// Test methods
// ---------------------------------------------------------------------------

fn test_cjk<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    // Test Ideographic Space - As wide as a CJK character cell (fullwidth)
    // used google to translate the word "term" to japanese -> 用語
    assert_query_equals(
        t,
        "term\u{3000}term\u{3000}term",
        None,
        "term\u{0020}term\u{0020}term",
    )?;
    assert_query_equals(
        t,
        "用語\u{3000}用語\u{3000}用語",
        None,
        "用語\u{0020}用語\u{0020}用語",
    )?;
    Ok(())
}

fn test_cjk_term<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    // individual CJK chars as terms
    let analyzer = simple_cjk_analyzer();
    let expected = bq(vec![
        (tq("field", "中"), Occur::SHOULD),
        (tq("field", "国"), Occur::SHOULD),
    ])?;
    assert_query_eq(&*expected, &*t.get_query_analyzer("中国", Some(analyzer))?);
    Ok(())
}

fn test_cjk_boosted_term<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    // individual CJK chars as terms
    let analyzer = simple_cjk_analyzer();
    let expected = bq(vec![
        (tq("field", "中"), Occur::SHOULD),
        (tq("field", "国"), Occur::SHOULD),
    ])?;
    let expected: Arc<dyn Query> = Arc::new(BoostQuery::new(expected, 0.5)?);
    assert_query_eq(
        &*expected,
        &*t.get_query_analyzer("中国^0.5", Some(analyzer))?,
    );
    Ok(())
}

fn test_cjk_phrase<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    // individual CJK chars as terms
    let analyzer = simple_cjk_analyzer();
    let expected = PhraseQuery::from_strings("field", &["中", "国"])?;
    assert_query_eq(
        &expected,
        &*t.get_query_analyzer("\"中国\"", Some(analyzer))?,
    );
    Ok(())
}

fn test_cjk_boosted_phrase<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    // individual CJK chars as terms
    let analyzer = simple_cjk_analyzer();
    let expected: Arc<dyn Query> = Arc::new(PhraseQuery::from_strings("field", &["中", "国"])?);
    let expected = BoostQuery::new(expected, 0.5)?;
    assert_query_eq(
        &expected,
        &*t.get_query_analyzer("\"中国\"^0.5", Some(analyzer))?,
    );
    Ok(())
}

fn test_cjk_sloppy_phrase<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    // individual CJK chars as terms
    let analyzer = simple_cjk_analyzer();
    let expected = PhraseQuery::with_slop(3, "field", &["中", "国"])?;
    assert_query_eq(
        &expected,
        &*t.get_query_analyzer("\"中国\"~3", Some(analyzer))?,
    );
    Ok(())
}

pub fn test_auto_generate_phrase_queries_on<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    // individual CJK chars as terms
    let analyzer = simple_cjk_analyzer();
    let expected = PhraseQuery::from_strings("field", &["中", "国"])?;
    let mut qp = t.get_parser_config(Some(analyzer))?;
    // Always true, since TestStandardQP overrides this method
    t.set_split_on_whitespace_if_classic(&mut qp)?; // LUCENE-7533
    t.set_auto_generate_phrase_queries(&mut qp, true)?;
    assert_query_eq(&expected, &*t.get_query_config("中国", &mut qp)?);
    Ok(())
}

fn test_simple<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    assert_query_equals(t, "term term term", None, "term term term")?;
    assert_query_equals(
        t,
        "türm term term",
        Some(mock_analyzer_default()),
        "türm term term",
    )?;
    assert_query_equals(t, "ümlaut", Some(mock_analyzer_default()), "ümlaut")?;

    // FIXME: enhance MockAnalyzer to be able to support this
    // it must no longer extend CharTokenizer
    // assertQueryEquals("\"\"", new KeywordAnalyzer(), "");
    // assertQueryEquals("foo:\"\"", new KeywordAnalyzer(), "foo:");

    assert_query_equals(t, "a AND b", None, "+a +b")?;
    assert_query_equals(t, "(a AND b)", None, "+a +b")?;
    assert_query_equals(t, "c OR (a AND b)", None, "c (+a +b)")?;
    assert_query_equals(t, "a AND NOT b", None, "+a -b")?;
    assert_query_equals(t, "a AND -b", None, "+a -b")?;
    assert_query_equals(t, "a AND !b", None, "+a -b")?;
    assert_query_equals(t, "a && b", None, "+a +b")?;
    //    assertQueryEquals("a && ! b", null, "+a -b");

    assert_query_equals(t, "a OR b", None, "a b")?;
    assert_query_equals(t, "a || b", None, "a b")?;
    assert_query_equals(t, "a OR !b", None, "a -b")?;
    //    assertQueryEquals("a OR ! b", null, "a -b");
    assert_query_equals(t, "a OR -b", None, "a -b")?;

    assert_query_equals(t, "+term -term term", None, "+term -term term")?;
    assert_query_equals(
        t,
        "foo:term AND field:anotherTerm",
        None,
        "+foo:term +anotherterm",
    )?;
    assert_query_equals(
        t,
        "term AND \"phrase phrase\"",
        None,
        "+term +\"phrase phrase\"",
    )?;
    assert_query_equals(t, "\"hello there\"", None, "\"hello there\"")?;
    Assert::assert_true(is::<BooleanQuery>(&*t.get_query("a AND b")?));
    Assert::assert_true(is::<TermQuery>(&*t.get_query("hello")?));
    Assert::assert_true(is::<PhraseQuery>(&*t.get_query("\"hello there\"")?));

    assert_query_equals(t, "germ term^2.0", None, "germ (term)^2.0")?;
    assert_query_equals(t, "(term)^2.0", None, "(term)^2.0")?;
    assert_query_equals(t, "(germ term)^2.0", None, "(germ term)^2.0")?;
    assert_query_equals(t, "term^2.0", None, "(term)^2.0")?;
    assert_query_equals(t, "term^2", None, "(term)^2.0")?;
    assert_query_equals(t, "\"germ term\"^2.0", None, "(\"germ term\")^2.0")?;
    assert_query_equals(t, "\"term germ\"^2", None, "(\"term germ\")^2.0")?;

    assert_query_equals(
        t,
        "(foo OR bar) AND (baz OR boo)",
        None,
        "+(foo bar) +(baz boo)",
    )?;
    assert_query_equals(t, "((a OR b) AND NOT c) OR d", None, "(+(a b) -c) d")?;
    assert_query_equals(
        t,
        "+(apple \"steve jobs\") -(foo bar baz)",
        None,
        "+(apple \"steve jobs\") -(foo bar baz)",
    )?;
    assert_query_equals(
        t,
        "+title:(dog OR cat) -author:\"bob dole\"",
        None,
        "+(title:dog title:cat) -author:\"bob dole\"",
    )?;
    Ok(())
}

fn test_operator_vs_whitespace<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    // LUCENE-2566
    // +,-,! should be directly adjacent to operand (i.e. not separated by whitespace) to be treated
    // as an operator
    let a = FnAnalyzer::new(
        "org.apache.lucene.queryparser.util.QueryParserTestBase$1",
        |_| Ok(tokenizer_components(mock_tokenizer(&WHITESPACE, false)?)),
    )
    .arc();
    assert_query_equals(t, "a - b", Some(Arc::clone(&a)), "a - b")?;
    assert_query_equals(t, "a + b", Some(Arc::clone(&a)), "a + b")?;
    assert_query_equals(t, "a ! b", Some(a), "a ! b")?;
    Ok(())
}

fn test_punct<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    let a = mock_analyzer(&WHITESPACE, false);
    assert_query_equals(t, "a&b", Some(Arc::clone(&a)), "a&b")?;
    assert_query_equals(t, "a&&b", Some(Arc::clone(&a)), "a&&b")?;
    assert_query_equals(t, ".NET", Some(a), ".NET")?;
    Ok(())
}

fn test_slop<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    assert_query_equals(t, "\"term germ\"~2", None, "\"term germ\"~2")?;
    assert_query_equals(t, "\"term germ\"~2 flork", None, "\"term germ\"~2 flork")?;
    assert_query_equals(t, "\"term\"~2", None, "term")?;
    assert_query_equals(t, "\" \"~2 germ", None, "germ")?;
    assert_query_equals(t, "\"term germ\"~2^2", None, "(\"term germ\"~2)^2.0")?;
    Ok(())
}

fn test_number<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    // The numbers go away because SimpleAnalzyer ignores them
    assert_match_no_docs_query_str(t, "3", None)?;
    assert_query_equals(t, "term 1.0 1 2", None, "term")?;
    assert_query_equals(t, "term term1 term2", None, "term term term")?;

    let a = mock_analyzer(&WHITESPACE, true);
    assert_query_equals(t, "3", Some(Arc::clone(&a)), "3")?;
    assert_query_equals(t, "term 1.0 1 2", Some(Arc::clone(&a)), "term 1.0 1 2")?;
    assert_query_equals(t, "term term1 term2", Some(a), "term term1 term2")?;
    Ok(())
}

fn test_wildcard<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    assert_query_equals(t, "term*", None, "term*")?;
    assert_query_equals(t, "term*^2", None, "(term*)^2.0")?;
    assert_query_equals(t, "term~", None, "term~2")?;
    assert_query_equals(t, "term~1", None, "term~1")?;
    assert_query_equals(t, "term~0.7", None, "term~1")?;
    assert_query_equals(t, "term~^3", None, "(term~2)^3.0")?;
    assert_query_equals(t, "term*germ", None, "term*germ")?;
    assert_query_equals(t, "term*germ^3", None, "(term*germ)^3.0")?;

    Assert::assert_true(is::<PrefixQuery>(&*t.get_query("term*")?));
    Assert::assert_true(is::<BoostQuery>(&*t.get_query("term*^2")?));
    let q = t.get_query("term*^2")?;
    Assert::assert_true(is::<PrefixQuery>(
        &*cast::<BoostQuery>(&*q, "org.apache.lucene.search.BoostQuery").get_query(),
    ));
    Assert::assert_true(is::<FuzzyQuery>(&*t.get_query("term~")?));
    Assert::assert_true(is::<FuzzyQuery>(&*t.get_query("term~0.7")?));
    let q = t.get_query("term~0.7")?;
    let fq = cast::<FuzzyQuery>(&*q, "org.apache.lucene.search.FuzzyQuery");
    Assert::assert_equals(1, fq.get_max_edits());
    Assert::assert_equals(fuzzy_query::DEFAULT_PREFIX_LENGTH, fq.get_prefix_length());
    let q = t.get_query("term~")?;
    let fq = cast::<FuzzyQuery>(&*q, "org.apache.lucene.search.FuzzyQuery");
    Assert::assert_equals(2, fq.get_max_edits());
    Assert::assert_equals(fuzzy_query::DEFAULT_PREFIX_LENGTH, fq.get_prefix_length());

    assert_parse_exception(t, "term~1.1")?; // value > 1, throws exception

    Assert::assert_true(is::<WildcardQuery>(&*t.get_query("term*germ")?));

    /* Tests to see that wild card terms are (or are not) properly
     * lower-cased with propery parser configuration
     */
    // First prefix queries:
    // by default, convert to lowercase:
    assert_wildcard_query_equals(t, "Term*", "term*")?;
    // explicitly set lowercase:
    assert_wildcard_query_equals(t, "term*", "term*")?;
    assert_wildcard_query_equals(t, "Term*", "term*")?;
    assert_wildcard_query_equals(t, "TERM*", "term*")?;
    // Then 'full' wildcard queries:
    // by default, convert to lowercase:
    assert_wildcard_query_equals(t, "Te?m", "te?m")?;
    // explicitly set lowercase:
    assert_wildcard_query_equals(t, "te?m", "te?m")?;
    assert_wildcard_query_equals(t, "Te?m", "te?m")?;
    assert_wildcard_query_equals(t, "TE?M", "te?m")?;
    assert_wildcard_query_equals(t, "Te?m*gerM", "te?m*germ")?;
    //  Fuzzy queries:
    assert_wildcard_query_equals(t, "Term~", "term~2")?;
    //  Range queries:
    assert_wildcard_query_equals(t, "[A TO C]", "[a TO c]")?;
    // Test suffix queries: first disallow
    let ex = expect_throws("java.lang.Exception", || {
        assert_wildcard_query_equals_allow(t, "*Term", "*term", false)
    });
    Assert::assert_true(t.is_query_parser_exception(&ex));

    let ex = expect_throws("java.lang.Exception", || {
        assert_wildcard_query_equals(t, "?Term", "?term")
    });
    Assert::assert_true(t.is_query_parser_exception(&ex));

    // Test suffix queries: then allow
    assert_wildcard_query_equals_allow(t, "*Term", "*term", true)?;
    assert_wildcard_query_equals_allow(t, "?Term", "?term", true)?;
    Ok(())
}

fn test_leading_wildcard_type<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    let mut cqp_c = t.get_parser_config(None)?;
    cqp_c.set_allow_leading_wildcard(true)?;
    let wildcard = "org.apache.lucene.search.WildcardQuery";
    assert_class(wildcard, &*t.get_query_config("t*erm*", &mut cqp_c)?);
    assert_class(wildcard, &*t.get_query_config("?term*", &mut cqp_c)?);
    assert_class(wildcard, &*t.get_query_config("*term*", &mut cqp_c)?);
    Ok(())
}

/// `testQPA()` of the base class (the classic subclass overrides it).
pub fn test_qpa<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    let qpa = Some(qp_analyzer());
    assert_query_equals(t, "term term^3.0 term", qpa.clone(), "term (term)^3.0 term")?;
    assert_query_equals(t, "term stop^3.0 term", qpa.clone(), "term term")?;

    assert_query_equals(t, "term term term", qpa.clone(), "term term term")?;
    assert_query_equals(t, "term +stop term", qpa.clone(), "term term")?;
    assert_query_equals(t, "term -stop term", qpa.clone(), "term term")?;

    assert_query_equals(t, "drop AND (stop) AND roll", qpa.clone(), "+drop +roll")?;
    assert_query_equals(t, "term +(stop) term", qpa.clone(), "term term")?;
    assert_query_equals(t, "term -(stop) term", qpa.clone(), "term term")?;

    assert_query_equals(t, "drop AND stop AND roll", qpa.clone(), "+drop +roll")?;

    // TODO: Re-enable once flexible standard parser gets multi-word synonym support
    //    assertQueryEquals("term phrase term", qpAnalyzer,
    //                      "term phrase1 phrase2 term");
    assert_query_equals(
        t,
        "term AND NOT phrase term",
        qpa.clone(),
        "+term -(phrase1 phrase2) term",
    )?;
    assert_match_no_docs_query_str(t, "stop^3", qpa.clone())?;
    assert_match_no_docs_query_str(t, "stop", qpa.clone())?;
    assert_match_no_docs_query_str(t, "(stop)^3", qpa.clone())?;
    assert_match_no_docs_query_str(t, "((stop))^3", qpa.clone())?;
    assert_match_no_docs_query_str(t, "(stop^3)", qpa.clone())?;
    assert_match_no_docs_query_str(t, "((stop)^3)", qpa.clone())?;
    assert_match_no_docs_query_str(t, "(stop)", qpa.clone())?;
    assert_match_no_docs_query_str(t, "((stop))", qpa.clone())?;
    Assert::assert_true(is::<BooleanQuery>(
        &*t.get_query_analyzer("term term term", qpa.clone())?,
    ));
    Assert::assert_true(is::<TermQuery>(
        &*t.get_query_analyzer("term +stop", qpa.clone())?,
    ));

    let mut cqpc = t.get_parser_config(qpa)?;
    t.set_default_operator_and(&mut cqpc)?;
    // TODO: Re-enable once flexible standard parser gets multi-word synonym support
    //    assertQueryEquals(cqpc, "field", "term phrase term",
    //        "+term +phrase1 +phrase2 +term");
    assert_query_equals_config(t, &mut cqpc, "field", "phrase", "+phrase1 +phrase2")?;
    Ok(())
}

fn test_range<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    assert_query_equals(t, "[ a TO z]", None, "[a TO z]")?;
    assert_query_equals(t, "[ a TO z}", None, "[a TO z}")?;
    assert_query_equals(t, "{ a TO z]", None, "{a TO z]")?;

    let q = t.get_query("[ a TO z]")?;
    Assert::assert_true(
        constant_score_blended_rewrite().method_eq(
            &*cast::<TermRangeQuery>(&*q, "org.apache.lucene.search.TermRangeQuery")
                .get_rewrite_method(),
        ),
    );

    let mut qp = t.get_parser_config(Some(mock_analyzer(&SIMPLE, true)))?;

    qp.set_multi_term_rewrite_method(Some(scoring_boolean_rewrite()))?;
    let q = t.get_query_config("[ a TO z]", &mut qp)?;
    Assert::assert_true(
        scoring_boolean_rewrite().method_eq(
            &*cast::<TermRangeQuery>(&*q, "org.apache.lucene.search.TermRangeQuery")
                .get_rewrite_method(),
        ),
    );

    // test open ranges
    assert_query_equals(t, "[ a TO * ]", None, "[a TO *]")?;
    assert_query_equals(t, "[ * TO z ]", None, "[* TO z]")?;
    assert_query_equals(t, "[ * TO * ]", None, "[* TO *]")?;

    // mixing exclude and include bounds
    assert_query_equals(t, "{ a TO z ]", None, "{a TO z]")?;
    assert_query_equals(t, "[ a TO z }", None, "[a TO z}")?;
    assert_query_equals(t, "{ a TO * ]", None, "{a TO *]")?;
    assert_query_equals(t, "[ * TO z }", None, "[* TO z}")?;

    assert_query_equals(t, "[ a TO z ]", None, "[a TO z]")?;
    assert_query_equals(t, "{ a TO z}", None, "{a TO z}")?;
    assert_query_equals(t, "{ a TO z }", None, "{a TO z}")?;
    assert_query_equals(t, "{ a TO z }^2.0", None, "({a TO z})^2.0")?;
    assert_query_equals(t, "[ a TO z] OR bar", None, "[a TO z] bar")?;
    assert_query_equals(t, "[ a TO z] AND bar", None, "+[a TO z] +bar")?;
    assert_query_equals(t, "( bar blar { a TO z}) ", None, "bar blar {a TO z}")?;
    assert_query_equals(
        t,
        "gack ( bar blar { a TO z}) ",
        None,
        "gack (bar blar {a TO z})",
    )?;

    assert_query_equals(t, "[* TO Z]", None, "[* TO z]")?;
    assert_query_equals(t, "[A TO *]", None, "[a TO *]")?;
    assert_query_equals(t, "[* TO *]", None, "[* TO *]")?;
    Ok(())
}

fn test_range_with_phrase<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    assert_query_equals(t, "[\\* TO \"*\"]", None, "[\\* TO \\*]")?;
    assert_query_equals(t, "[\"*\" TO *]", None, "[\\* TO *]")?;
    Ok(())
}

fn test_range_query_endpoint_to<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    let a = Some(mock_analyzer_default());
    assert_query_equals(t, "[to TO to]", a.clone(), "[to TO to]")?;
    assert_query_equals(t, "[to TO TO]", a.clone(), "[to TO to]")?;
    assert_query_equals(t, "[TO TO to]", a.clone(), "[to TO to]")?;
    assert_query_equals(t, "[TO TO TO]", a.clone(), "[to TO to]")?;

    assert_query_equals(t, "[\"TO\" TO \"TO\"]", a.clone(), "[to TO to]")?;
    assert_query_equals(t, "[\"TO\" TO TO]", a.clone(), "[to TO to]")?;
    assert_query_equals(t, "[TO TO \"TO\"]", a.clone(), "[to TO to]")?;

    assert_query_equals(t, "[to TO xx]", a.clone(), "[to TO xx]")?;
    assert_query_equals(t, "[\"TO\" TO xx]", a.clone(), "[to TO xx]")?;
    assert_query_equals(t, "[TO TO xx]", a.clone(), "[to TO xx]")?;

    assert_query_equals(t, "[xx TO to]", a.clone(), "[xx TO to]")?;
    assert_query_equals(t, "[xx TO \"TO\"]", a.clone(), "[xx TO to]")?;
    assert_query_equals(t, "[xx TO TO]", a, "[xx TO to]")?;
    Ok(())
}

fn test_range_query_requires_to<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    let a = Some(mock_analyzer_default());

    assert_query_equals(t, "{A TO B}", a.clone(), "{a TO b}")?;
    assert_query_equals(t, "[A TO B}", a.clone(), "[a TO b}")?;
    assert_query_equals(t, "{A TO B]", a.clone(), "{a TO b]")?;
    assert_query_equals(t, "[A TO B]", a, "[a TO b]")?;

    // " TO " is required between range endpoints

    let exception_class = if t.is_test_query_parser() {
        CLASSIC_PARSE_EXCEPTION
    } else {
        STANDARD_PARSE_EXCEPTION
    };

    for q in [
        "{A B}", "[A B}", "{A B]", "[A B]", "{TO B}", "[TO B}", "{TO B]", "[TO B]", "{A TO}",
        "[A TO}", "{A TO]", "[A TO]",
    ] {
        expect_throws(exception_class, || t.get_query(q));
    }
    Ok(())
}

pub fn escape_date_string(s: &str) -> String {
    if s.contains(' ') {
        format!("\"{s}\"")
    } else {
        s.to_string()
    }
}

/// for testing DateTools support: `getDate(String s, Resolution)`.
pub fn get_date_str(s: &str, resolution: Resolution) -> Result<String> {
    // we use the default Locale since LuceneTestCase randomizes it
    let df = date_format::get_date_instance(date_format::SHORT, &Locale::get_default())?;
    Ok(get_date_millis(df.parse(s)?.get_time(), resolution))
}

/// for testing DateTools support: `getDate(Date d, Resolution)`.
pub fn get_date_millis(d: i64, resolution: Resolution) -> String {
    DateTools::time_to_string(d, resolution)
}

pub fn get_localized_date(year: i32, month: i32, day: i32) -> Result<String> {
    // we use the default Locale/TZ since LuceneTestCase randomizes it
    let locale = Locale::get_default();
    let df = date_format::get_date_instance(date_format::SHORT, &locale)?;
    let mut calendar = Calendar::new_gregorian(&TimeZone::get_default(), &locale)?;
    calendar.clear();
    calendar.set_date(year, month, day);
    calendar.set(calendar::HOUR_OF_DAY, 23);
    calendar.set(calendar::MINUTE, 59);
    calendar.set(calendar::SECOND, 59);
    calendar.set(calendar::MILLISECOND, 999);
    df.format(calendar.get_time()?)
}

fn test_date_range<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
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
    let a = mock_analyzer(&SIMPLE, true);
    let mut qp = t.get_parser_config(Some(a))?;

    // set a field specific date resolution
    t.set_date_resolution(&mut qp, month_field, Resolution::MONTH)?;

    // set default date resolution to MILLISECOND
    qp.set_date_resolution(Some(Resolution::MILLISECOND))?;

    // set second field specific date resolution
    t.set_date_resolution(&mut qp, hour_field, Resolution::HOUR)?;

    let end_inclusive = end_date_expected.get_time()?.get_time();
    // for this field no field specific date resolution has been set,
    // so verify if the default resolution is used
    assert_date_range_query_equals(
        t,
        &mut qp,
        default_field,
        &start_date,
        &end_date,
        end_inclusive,
        Resolution::MILLISECOND,
    )?;

    // verify if field specific date resolutions are used for these two fields
    assert_date_range_query_equals(
        t,
        &mut qp,
        month_field,
        &start_date,
        &end_date,
        end_inclusive,
        Resolution::MONTH,
    )?;

    assert_date_range_query_equals(
        t,
        &mut qp,
        hour_field,
        &start_date,
        &end_date,
        end_inclusive,
        Resolution::HOUR,
    )?;
    Ok(())
}

/// `assertDateRangeQueryEquals(CommonQueryParserConfiguration, String field,
/// String startDate, String endDate, Date endDateInclusive, Resolution)`.
pub fn assert_date_range_query_equals<T: QueryParserTestBase>(
    t: &mut T,
    cqp_c: &mut T::Config,
    field: &str,
    start_date: &str,
    end_date: &str,
    end_date_inclusive: i64,
    resolution: Resolution,
) -> Result<()> {
    assert_query_equals_config(
        t,
        cqp_c,
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
    assert_query_equals_config(
        t,
        cqp_c,
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

fn test_escaped<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    let a = mock_analyzer(&WHITESPACE, false);
    let aa = || Some(Arc::clone(&a));

    /*assertQueryEquals("\\[brackets", a, "\\[brackets");
    ... (commented out in Lucene) */

    assert_query_equals(t, "\\a", aa(), "a")?;

    assert_query_equals(t, "a\\-b:c", aa(), "a-b:c")?;
    assert_query_equals(t, "a\\+b:c", aa(), "a+b:c")?;
    assert_query_equals(t, "a\\:b:c", aa(), "a:b:c")?;
    assert_query_equals(t, "a\\\\b:c", aa(), "a\\b:c")?;

    assert_query_equals(t, "a:b\\-c", aa(), "a:b-c")?;
    assert_query_equals(t, "a:b\\+c", aa(), "a:b+c")?;
    assert_query_equals(t, "a:b\\:c", aa(), "a:b:c")?;
    assert_query_equals(t, "a:b\\\\c", aa(), "a:b\\c")?;

    assert_query_equals(t, "a:b\\-c*", aa(), "a:b-c*")?;
    assert_query_equals(t, "a:b\\+c*", aa(), "a:b+c*")?;
    assert_query_equals(t, "a:b\\:c*", aa(), "a:b:c*")?;

    assert_query_equals(t, "a:b\\\\c*", aa(), "a:b\\c*")?;

    assert_query_equals(t, "a:b\\-c~", aa(), "a:b-c~2")?;
    assert_query_equals(t, "a:b\\+c~", aa(), "a:b+c~2")?;
    assert_query_equals(t, "a:b\\:c~", aa(), "a:b:c~2")?;
    assert_query_equals(t, "a:b\\\\c~", aa(), "a:b\\c~2")?;

    assert_query_equals(t, "[ a\\- TO a\\+ ]", None, "[a- TO a+]")?;
    assert_query_equals(t, "[ a\\: TO a\\~ ]", None, "[a: TO a~]")?;
    assert_query_equals(t, "[ a\\\\ TO a\\* ]", None, "[a\\ TO a*]")?;

    assert_query_equals(
        t,
        "[\"c\\:\\\\temp\\\\\\~foo0.txt\" TO \"c\\:\\\\temp\\\\\\~foo9.txt\"]",
        aa(),
        "[c:\\temp\\~foo0.txt TO c:\\temp\\~foo9.txt]",
    )?;

    assert_query_equals(t, "a\\\\\\+b", aa(), "a\\+b")?;

    assert_query_equals(t, "a \\\"b c\\\" d", aa(), "a \"b c\" d")?;
    assert_query_equals(t, "\"a \\\"b c\\\" d\"", aa(), "\"a \"b c\" d\"")?;
    assert_query_equals(t, "\"a \\+b c d\"", aa(), "\"a +b c d\"")?;

    assert_query_equals(t, "c\\:\\\\temp\\\\\\~foo.txt", aa(), "c:\\temp\\~foo.txt")?;

    assert_parse_exception(t, "XY\\")?; // there must be a character after the escape char

    // test unicode escaping
    assert_query_equals(t, "a\\u0062c", aa(), "abc")?;
    assert_query_equals(t, "XY\\u005a", aa(), "XYZ")?;
    assert_query_equals(t, "XY\\u005A", aa(), "XYZ")?;
    assert_query_equals(t, "\"a \\\\\\u0028\\u0062\\\" c\"", aa(), "\"a \\(b\" c\"")?;

    assert_parse_exception(t, "XY\\u005G")?; // test non-hex character in escaped unicode sequence
    assert_parse_exception(t, "XY\\u005")?; // test incomplete escaped unicode sequence

    // Tests bug LUCENE-800
    assert_query_equals(t, "(item:\\\\ item:ABCD\\\\)", aa(), "item:\\ item:ABCD\\")?;
    assert_parse_exception(t, "(item:\\\\ item:ABCD\\\\))")?; // unmatched closing paranthesis
    assert_query_equals(t, "\\*", aa(), "*")?;
    assert_query_equals(t, "\\\\", aa(), "\\")?; // escaped backslash

    assert_parse_exception(t, "\\")?; // a backslash must always be escaped

    // LUCENE-1189
    assert_query_equals(t, "(\"a\\\\\") or (\"b\")", aa(), "a\\ or b")?;
    Ok(())
}

fn test_escaped_vs_question_mark_as_wildcard<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    let a = mock_analyzer(&WHITESPACE, false);
    assert_query_equals(t, "a:b\\-?c", Some(Arc::clone(&a)), "a:b\\-?c")?;
    assert_query_equals(t, "a:b\\+?c", Some(Arc::clone(&a)), "a:b\\+?c")?;
    assert_query_equals(t, "a:b\\:?c", Some(Arc::clone(&a)), "a:b\\:?c")?;

    assert_query_equals(t, "a:b\\\\?c", Some(a), "a:b\\\\?c")?;
    Ok(())
}

fn test_query_string_escaping<T: QueryParserTestBase>(_t: &mut T) -> Result<()> {
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

fn test_tab_newline_carriage_return<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    for q in [
        "+weltbank +worlbank",
        "+weltbank\n+worlbank",
        "weltbank \n+worlbank",
        "weltbank \n +worlbank",
        "+weltbank\r+worlbank",
        "weltbank \r+worlbank",
        "weltbank \r +worlbank",
        "+weltbank\r\n+worlbank",
        "weltbank \r\n+worlbank",
        "weltbank \r\n +worlbank",
        "weltbank \r \n +worlbank",
        "+weltbank\t+worlbank",
        "weltbank \t+worlbank",
        "weltbank \t +worlbank",
    ] {
        assert_query_equals_doa(t, q, None, "+weltbank +worlbank")?;
    }
    Ok(())
}

fn test_simple_dao<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    assert_query_equals_doa(t, "term term term", None, "+term +term +term")?;
    assert_query_equals_doa(t, "term +term term", None, "+term +term +term")?;
    assert_query_equals_doa(t, "term term +term", None, "+term +term +term")?;
    assert_query_equals_doa(t, "term +term +term", None, "+term +term +term")?;
    assert_query_equals_doa(t, "-term term term", None, "-term +term +term")?;
    Ok(())
}

fn test_boost<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    let stop_words = Arc::new(CharacterRunAutomaton::new(Automata::make_string("on"))?);
    let one_stop_analyzer = mock_analyzer_filter(&SIMPLE, true, &stop_words);
    let mut qp = t.get_parser_config(Some(one_stop_analyzer))?;
    let q = t.get_query_config("on^1.0", &mut qp)?;
    let _ = q; // assertNotNull(q)
    let q = t.get_query_config("\"hello\"^2.0", &mut qp)?;
    Assert::assert_equals_float_msg(
        None,
        cast::<BoostQuery>(&*q, "org.apache.lucene.search.BoostQuery").get_boost(),
        2.0,
        0.5,
    );
    let q = t.get_query_config("hello^2.0", &mut qp)?;
    Assert::assert_equals_float_msg(
        None,
        cast::<BoostQuery>(&*q, "org.apache.lucene.search.BoostQuery").get_boost(),
        2.0,
        0.5,
    );
    let q = t.get_query_config("\"on\"^1.0", &mut qp)?;
    let _ = q; // assertNotNull(q)

    let a2 = mock_analyzer_filter(&SIMPLE, true, &ENGLISH_STOPSET);
    let mut qp2 = t.get_parser_config(Some(a2))?;
    let q = t.get_query_config("the^3", &mut qp2)?;
    // "the" is a stop word so the result is an empty query:
    assert_match_no_docs_query(&*q);
    Assert::assert_false(is::<BoostQuery>(&*q));
    Ok(())
}

fn test_exception<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    assert_parse_exception(t, "\"some phrase")?;
    assert_parse_exception(t, "(foo bar")?;
    assert_parse_exception(t, "foo bar))")?;
    assert_parse_exception(t, "field:term:with:colon some more terms")?;
    assert_parse_exception(t, "(sub query)^5.0^2.0 plus more")?;
    assert_parse_exception(t, "secret AND illegal) AND access:confidential")?;
    Ok(())
}

fn test_boolean_query<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    IndexSearcher::set_max_clause_count(2)?;
    let pur_whitespace_analyzer = mock_analyzer(&WHITESPACE, false);
    assert_parse_exception_analyzer(t, "one two three", Some(pur_whitespace_analyzer))?;
    Ok(())
}

/// This test differs from TestPrecedenceQueryParser
fn test_precedence<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    let mut qp = t.get_parser_config(Some(mock_analyzer(&WHITESPACE, false)))?;
    let query1 = t.get_query_config("A AND B OR C AND D", &mut qp)?;
    let query2 = t.get_query_config("+A +B +C +D", &mut qp)?;
    assert_query_eq(&*query1, &*query2);
    Ok(())
}

fn test_parses_brackets_if_quoted<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    let a = mock_analyzer(&WHITESPACE, false);
    let aa = || Some(Arc::clone(&a));

    assert_query_equals(t, "[\"a[i]\" TO \"b[i]\"]", aa(), "[a[i] TO b[i]]")?;
    assert_query_equals(t, "{\"a[i]\" TO \"b[i]\"}", aa(), "{a[i] TO b[i]}")?;
    assert_query_equals(t, "[\"a[i]\" TO \"b[i]\"}", aa(), "[a[i] TO b[i]}")?;
    assert_query_equals(t, "{\"a[i]\" TO \"b[i]\"]", aa(), "{a[i] TO b[i]]")?;

    assert_query_equals(t, "[\"a[i\\]\" TO \"b[i\\]\"]", aa(), "[a[i] TO b[i]]")?;
    assert_query_equals(t, "[\"a\\[i\\]\" TO \"b\\[i\\]\"]", aa(), "[a[i] TO b[i]]")?;

    assert_query_equals(
        t,
        "[\"a[i][j]\" TO \"b[i][j]\"]",
        aa(),
        "[a[i][j] TO b[i][j]]",
    )?;

    assert_query_equals(
        t,
        "[ \"2024-01-01T01:01:01+01:00[Europe/Warsaw]\" TO \"2025-01-01T01:01:01+01:00[Europe/Warsaw]\" ]",
        None,
        "[2024-01-01t01:01:01+01:00[europe/warsaw] TO 2025-01-01t01:01:01+01:00[europe/warsaw]]",
    )?;

    // If the range terms aren't wrapped in quotes, a closing bracket will throw
    assert_parse_exception(t, "[a[i] TO b[i]]")?;
    assert_parse_exception(t, "[a\\[i\\] TO b\\[i\\]]")?;
    Ok(())
}

fn test_escaped_wildcard<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    let mut qp = t.get_parser_config(Some(mock_analyzer(&WHITESPACE, false)))?;
    let q = WildcardQuery::new(term("field", "foo\\?ba?r"))?;
    assert_query_eq(&q, &*t.get_query_config("foo\\?ba?r", &mut qp)?);
    Ok(())
}

/// `name -> null`: no regular expression of the test names an automaton.
struct NullProvider;

impl AutomatonProvider for NullProvider {
    fn get_automaton(&self, _name: &str) -> Result<Automaton> {
        Err(LuceneError::null_pointer())
    }
}

fn test_regexps<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    let mut qp = t.get_parser_config(Some(mock_analyzer(&WHITESPACE, true)))?;
    let q: Arc<dyn Query> = Arc::new(RegexpQuery::new(term("field", "[a-z][123]"))?);
    assert_query_eq(&*q, &*t.get_query_config("/[a-z][123]/", &mut qp)?);
    assert_query_eq(&*q, &*t.get_query_config("/[A-Z][123]/", &mut qp)?);
    assert_query_eq(
        &BoostQuery::new(Arc::clone(&q), 0.5)?,
        &*t.get_query_config("/[A-Z][123]/^0.5", &mut qp)?,
    );

    let escaped = RegexpQuery::new(term("field", "[a-z]\\/[123]"))?;
    assert_query_eq(&escaped, &*t.get_query_config("/[a-z]\\/[123]/", &mut qp)?);
    let escaped2 = RegexpQuery::new(term("field", "[a-z]\\*[123]"))?;
    assert_query_eq(&escaped2, &*t.get_query_config("/[a-z]\\*[123]/", &mut qp)?);

    let complex = bq(vec![
        (
            Arc::new(RegexpQuery::new(term("field", "[a-z]\\/[123]"))?),
            Occur::MUST,
        ),
        (tq("path", "/etc/init.d/"), Occur::MUST),
        (tq("field", "/etc/init[.]d/lucene/"), Occur::SHOULD),
    ])?;
    assert_query_eq(
        &*complex,
        &*t.get_query_config(
            "/[a-z]\\/[123]/ AND path:\"/etc/init.d/\" OR \"/etc\\/init\\[.\\]d/lucene/\" ",
            &mut qp,
        )?,
    );

    let re = RegexpQuery::new(term("field", "http.*"))?;
    assert_query_eq(&re, &*t.get_query_config("field:/http.*/", &mut qp)?);
    assert_query_eq(&re, &*t.get_query_config("/http.*/", &mut qp)?);

    let re = RegexpQuery::new(term("field", "http~0.5"))?;
    assert_query_eq(&re, &*t.get_query_config("field:/http~0.5/", &mut qp)?);
    assert_query_eq(&re, &*t.get_query_config("/http~0.5/", &mut qp)?);

    let re = RegexpQuery::new(term("field", "boo"))?;
    assert_query_eq(&re, &*t.get_query_config("field:/boo/", &mut qp)?);
    assert_query_eq(&re, &*t.get_query_config("/boo/", &mut qp)?);

    assert_query_eq(
        &*tq("field", "/boo/"),
        &*t.get_query_config("\"/boo/\"", &mut qp)?,
    );
    assert_query_eq(
        &*tq("field", "/boo/"),
        &*t.get_query_config("\\/boo\\/", &mut qp)?,
    );

    let two = bq(vec![
        (
            Arc::new(RegexpQuery::new(term("field", "foo"))?),
            Occur::SHOULD,
        ),
        (
            Arc::new(RegexpQuery::new(term("field", "bar"))?),
            Occur::SHOULD,
        ),
    ])?;
    assert_query_eq(
        &*two,
        &*t.get_query_config("field:/foo/ field:/bar/", &mut qp)?,
    );
    assert_query_eq(&*two, &*t.get_query_config("/foo/ /bar/", &mut qp)?);

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
    let parsed = t.get_query_config("/[A-Z][123]/^0.5", &mut qp)?;
    Assert::assert_true(is::<BoostQuery>(&*parsed));
    let parsed = t.get_query_config("/[A-Z][123]/^0.5", &mut qp)?;
    Assert::assert_true(is::<RegexpQuery>(
        &*cast::<BoostQuery>(&*parsed, "org.apache.lucene.search.BoostQuery").get_query(),
    ));
    let parsed = t.get_query_config("/[A-Z][123]/^0.5", &mut qp)?;
    let inner = cast::<BoostQuery>(&*parsed, "org.apache.lucene.search.BoostQuery").get_query();
    Assert::assert_true(scoring_boolean_rewrite().method_eq(
        &*cast::<RegexpQuery>(&*inner, "org.apache.lucene.search.RegexpQuery").get_rewrite_method(),
    ));
    assert_query_eq(
        &BoostQuery::new(q, 0.5)?,
        &*t.get_query_config("/[A-Z][123]/^0.5", &mut qp)?,
    );
    Ok(())
}

fn test_stopwords<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    let stop_set = Arc::new(CharacterRunAutomaton::new(
        RegExp::new("the|foo")?.to_automaton()?,
    )?);
    let mut qp = t.get_parser_config(Some(mock_analyzer_filter(&SIMPLE, true, &stop_set)))?;
    let result = t.get_query_config("field:the OR field:foo", &mut qp)?;
    Assert::assert_true_msg(
        "result is not a BooleanQuery",
        is::<BooleanQuery>(&*result) || is::<MatchNoDocsQuery>(&*result),
    );
    if is::<BooleanQuery>(&*result) {
        Assert::assert_equals(
            0,
            cast::<BooleanQuery>(&*result, "BooleanQuery")
                .clauses()
                .len(),
        );
    }
    let result = t.get_query_config("field:woo OR field:the", &mut qp)?;
    Assert::assert_true_msg("result is not a TermQuery", is::<TermQuery>(&*result));
    let result = t.get_query_config(
        "(fieldX:xxxxx OR fieldy:xxxxxxxx)^2 AND (fieldx:the OR fieldy:foo)",
        &mut qp,
    )?;
    Assert::assert_true_msg("result is not a BoostQuery", is::<BoostQuery>(&*result));
    let result = cast::<BoostQuery>(&*result, "org.apache.lucene.search.BoostQuery").get_query();
    Assert::assert_true_msg("result is not a BooleanQuery", is::<BooleanQuery>(&*result));
    if verbose() {
        println!("Result: {}", to_s(&*result));
    }
    let n = cast::<BooleanQuery>(&*result, "BooleanQuery")
        .clauses()
        .len();
    Assert::assert_true_msg(&format!("{n} does not equal: 2"), n == 2);
    Ok(())
}

fn test_position_increment<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    let mut qp =
        t.get_parser_config(Some(mock_analyzer_filter(&SIMPLE, true, &ENGLISH_STOPSET)))?;
    qp.set_enable_position_increments(true)?;
    let qtxt = "\"the words in poisitions pos02578 are stopped in this phrasequery\"";
    //               0         2                      5           7  8
    let expected_positions = [1, 3, 4, 6, 9];
    let q = t.get_query_config(qtxt, &mut qp)?;
    let pq = cast::<PhraseQuery>(&*q, "org.apache.lucene.search.PhraseQuery");
    let terms = pq.get_terms();
    let pos = pq.get_positions();
    for i in 0..terms.len() {
        Assert::assert_equals_msg(
            Some(&format!("term {i} = {} has wrong term-position!", terms[i])),
            expected_positions[i],
            pos[i],
        );
    }
    Ok(())
}

fn test_match_all_docs<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    let mut qp = t.get_parser_config(Some(mock_analyzer(&WHITESPACE, false)))?;
    assert_query_eq(
        &MatchAllDocsQuery::instance(),
        &*t.get_query_config("*:*", &mut qp)?,
    );
    assert_query_eq(
        &MatchAllDocsQuery::instance(),
        &*t.get_query_config("(*:*)", &mut qp)?,
    );
    let q = t.get_query_config("+*:* -*:*", &mut qp)?;
    let bq = cast::<BooleanQuery>(&*q, "org.apache.lucene.search.BooleanQuery");
    Assert::assert_equals(2, bq.clauses().len());
    for clause in bq.clauses() {
        Assert::assert_true(is::<MatchAllDocsQuery>(&**clause.query()));
    }
    Ok(())
}

/// `assertHits(int expected, String query, IndexSearcher is)` (unused in
/// Lucene too: `@SuppressWarnings("unused")`).
#[allow(dead_code)]
fn assert_hits<T: QueryParserTestBase>(
    t: &mut T,
    expected: usize,
    query: &str,
    is: &IndexSearcher,
) -> Result<()> {
    let old_default_field = t.get_default_field();
    t.set_default_field("date");
    let mut qp = t.get_parser_config(Some(mock_analyzer(&WHITESPACE, false)))?;
    qp.set_locale(Some(Locale::english()))?;
    let q = t.get_query_config(query, &mut qp)?;
    let hits = is.search(q, 1000)?.score_docs;
    Assert::assert_equals(expected, hits.len());
    t.set_default_field(&old_default_field);
    Ok(())
}

// LUCENE-2002: make sure defaults for StandardAnalyzer's
// enableStopPositionIncr & QueryParser's enablePosIncr
// "match"
fn test_position_increments<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let a = mock_analyzer_filter(&SIMPLE, true, &ENGLISH_STOPSET);
    let w = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::clone(&a))),
    )?;
    let mut doc = Document::new();
    doc.add(LuceneTestCase::new_text_field(
        "field",
        "the wizard of ozzy",
        Store::NO,
    ));
    w.add_document(&doc)?;
    let r = directory_reader::open_with_writer(&*w)?;
    w.close()?;
    let s = LuceneTestCase::new_searcher(Arc::clone(&r));

    let q = t.get_query_analyzer("\"wizard of ozzy\"", Some(a))?;
    Assert::assert_equals(1i64, s.search(q, 1)?.total_hits.value());
    r.close()?;
    dir.close()?;
    Ok(())
}

fn test_collated_range<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    let mut qp = t.get_parser_config(Some(mock_collation_analyzer()))?;
    let expected = TermRangeQuery::new_string_range(
        t.get_default_field(),
        Some("collatedabc"),
        Some("collateddef"),
        true,
        true,
    )?;
    let actual = t.get_query_config("[abc TO def]", &mut qp)?;
    assert_query_eq(&expected, &*actual);
    Ok(())
}

fn test_distance_as_edits_parsing<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    let q = t.get_query_analyzer("foobar~2", Some(mock_analyzer_default()))?;
    let q = cast::<FuzzyQuery>(&*q, "org.apache.lucene.search.FuzzyQuery");
    Assert::assert_equals(2, q.get_max_edits());
    Ok(())
}

fn test_phrase_query_to_string<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    let analyzer = mock_analyzer_filter(&SIMPLE, true, &ENGLISH_STOPSET);
    let mut qp = t.get_parser_config(Some(analyzer))?;
    qp.set_enable_position_increments(true)?;
    let q = t.get_query_config("\"this hi this is a test is\"", &mut qp)?;
    let q = cast::<PhraseQuery>(&*q, "org.apache.lucene.search.PhraseQuery");
    super::assert_str_eq("field:\"? hi ? ? ? test\"", &to_s(q));
    Ok(())
}

fn test_parse_wildcard_and_phrase_queries<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    let field = "content";
    let old_default_field = t.get_default_field();
    t.set_default_field(field);
    let mut qp = t.get_parser_config(Some(mock_analyzer_default()))?;
    qp.set_allow_leading_wildcard(true)?;

    let prefix_queries: [&[&str]; 3] = [
        &["a*", "ab*", "abc*"],
        &["h*", "hi*", "hij*", "\\\\7*"],
        &["o*", "op*", "opq*", "\\\\\\\\*"],
    ];

    let wildcard_queries: [&[&str]; 3] = [
        &["*a*", "*ab*", "*abc**", "ab*e*", "*g?", "*f?1", "abc**"],
        &["*h*", "*hi*", "*hij**", "hi*k*", "*n?", "*m?1", "hij**"],
        &["*o*", "*op*", "*opq**", "op*q*", "*u?", "*t?1", "opq**"],
    ];

    // test queries that must be prefix queries
    for row in prefix_queries {
        for query_string in row {
            let q = t.get_query_config(query_string, &mut qp)?;
            assert_class("org.apache.lucene.search.PrefixQuery", &*q);
        }
    }

    // test queries that must be wildcard queries
    for row in wildcard_queries {
        for qtxt in row {
            let q = t.get_query_config(qtxt, &mut qp)?;
            assert_class("org.apache.lucene.search.WildcardQuery", &*q);
        }
    }
    t.set_default_field(&old_default_field);
    Ok(())
}

fn test_phrase_query_position_increments<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    let stop_stop_list = Arc::new(CharacterRunAutomaton::new(
        RegExp::new("[sS][tT][oO][pP]")?.to_automaton()?,
    )?);

    let mut qp = t.get_parser_config(Some(mock_analyzer_filter(
        &WHITESPACE,
        false,
        &stop_stop_list,
    )))?;
    qp.set_enable_position_increments(true)?;

    let mut phrase_query = PhraseQueryBuilder::new();
    phrase_query.add(term("field", "1"))?;
    phrase_query.add_at(term("field", "2"), 2)?;
    assert_query_eq(
        &phrase_query.build()?,
        &*t.get_query_config("\"1 stop 2\"", &mut qp)?,
    );
    Ok(())
}

fn test_match_all_query_parsing<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    // test simple parsing of MatchAllDocsQuery
    let old_default_field = t.get_default_field();
    t.set_default_field("key");
    let mut qp = t.get_parser_config(Some(mock_analyzer_default()))?;
    assert_query_eq(
        &MatchAllDocsQuery::instance(),
        &*t.get_query_config(&to_s(&MatchAllDocsQuery::instance()), &mut qp)?,
    );

    // test parsing with non-default boost
    let query: Arc<dyn Query> = Arc::new(MatchAllDocsQuery::instance());
    let query = BoostQuery::new(query, 2.3)?;
    assert_query_eq(&query, &*t.get_query_config(&to_s(&query), &mut qp)?);
    t.set_default_field(&old_default_field);
    Ok(())
}

fn test_nested_and_clauses_foo<T: QueryParserTestBase>(t: &mut T) -> Result<()> {
    let query = "(field1:[1 TO *] AND field1:[* TO 2]) AND field2:(z)";
    let inner = bq(vec![
        (
            Arc::new(TermRangeQuery::new_string_range(
                "field1",
                Some("1"),
                None,
                true,
                true,
            )?),
            Occur::MUST,
        ),
        (
            Arc::new(TermRangeQuery::new_string_range(
                "field1",
                None,
                Some("2"),
                true,
                true,
            )?),
            Occur::MUST,
        ),
    ])?;
    let q = bq(vec![(inner, Occur::MUST), (tq("field2", "z"), Occur::MUST)])?;
    assert_query_eq(
        &*q,
        &*t.get_query_analyzer(query, Some(mock_analyzer_default()))?,
    );
    Ok(())
}
