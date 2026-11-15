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

//! Port of `TestQueryParser.java`.
//!
//! The fixture [`TestQueryParserFixture`] is shared with
//! `TestExtendableQueryParser`, which extends this class and overrides
//! `getParser(Analyzer)`: `EXT` selects that override.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::Assert;
use rucene::analysis::{Analyzer, TokenFilter, TokenFilterLogic, TokenStream};
use rucene::document::{Document, Field, FieldType, Resolution};
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::index::{directory_reader, IndexOptions, IndexReader};
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query::Builder as BooleanQueryBuilder;
use rucene::search::boost_query::BoostQuery;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::match_all_docs_query::MatchAllDocsQuery;
use rucene::search::multi_phrase_query::Builder as MultiPhraseQueryBuilder;
use rucene::search::phrase_query::{Builder as PhraseQueryBuilder, PhraseQuery};
use rucene::search::synonym_query::Builder as SynonymQueryBuilder;
use rucene::search::term_query::TermQuery;
use rucene::search::Query;
use rucene::util::attribute::AttributeSource;
use rucene_queryparser::classic::parse_exception::ParseException;
use rucene_queryparser::classic::query_parser::{
    Operator, QueryParser, QueryParserOps, DEFAULT_SPLIT_ON_WHITESPACE,
};
use rucene_queryparser::classic::query_parser_base::{
    self, QueryParserBaseOps, AND_OPERATOR, OR_OPERATOR,
};
use rucene_queryparser::classic::token::TokenRef;
use rucene_queryparser::ext::extendable_query_parser::ExtendableQueryParser;
use rucene_queryparser::ext::extensions::Extensions;
use rucene_queryparser::flexible::standard::common_query_parser_configuration::CommonQueryParserConfiguration;
use rucene_test_framework::analysis::attributes::{char_term_id, pos_inc_id, Att};
use rucene_test_framework::analysis::mock_bytes_analyzer::MockBytesAnalyzer;
use rucene_test_framework::analysis::mock_lower_case_filter::MockLowerCaseFilter;
use rucene_test_framework::analysis::mock_synonym_analyzer::MockSynonymAnalyzer;
use rucene_test_framework::analysis::mock_tokenizer::{SIMPLE, WHITESPACE};
use rucene_test_framework::analysis::{components, shared, sink};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::LuceneTestCase;

use crate::qp_support::core_support::random;
use crate::qp_support::query_parser_test_base::{
    analyzer1, analyzer2, assert_query_equals, assert_query_equals_config, base_methods,
    mock_analyzer, mock_analyzer_default, mock_tokenizer, qp_analyzer, simple_cjk_tokenizer,
    FixtureTest, FnAnalyzer, QueryParserTestBase, QueryParserTestBaseState,
};
use crate::qp_support::search_support::{fixture_class, ftest, term, tq};
use crate::qp_support::{
    assert_query_eq, assert_str_eq, cast, expect_throws, CLASSIC_PARSE_EXCEPTION,
};

const FIELD: &str = "field";

/// The instance of `TestQueryParser` (`EXT == false`) or of its subclass
/// `TestExtendableQueryParser` (`EXT == true`).
pub struct TestQueryParserFixture<const EXT: bool> {
    base: QueryParserTestBaseState,
    /// `protected boolean splitOnWhitespace = QueryParser.DEFAULT_SPLIT_ON_WHITESPACE`.
    pub split_on_whitespace: bool,
}

impl<const EXT: bool> Default for TestQueryParserFixture<EXT> {
    fn default() -> Self {
        TestQueryParserFixture {
            base: QueryParserTestBaseState::default(),
            split_on_whitespace: DEFAULT_SPLIT_ON_WHITESPACE,
        }
    }
}

/// Declares the boilerplate a Java subclass of `QueryParser` inherits: the
/// state accessors and the generated members, for a struct whose field
/// `parser` is the `QueryParser` it extends.
macro_rules! classic_subclass {
    ($ty:ty) => {
        impl ::rucene_queryparser::classic::query_parser_constants::QueryParserConstants for $ty {}

        impl ::rucene::util::query_builder::QueryBuilderOps for $ty {
            fn query_builder(&self) -> &::rucene::util::query_builder::QueryBuilder {
                ::rucene::util::query_builder::QueryBuilderOps::query_builder(&self.parser)
            }
        }

        impl ::rucene_queryparser::classic::query_parser::QueryParserOps for $ty {
            fn query_parser(&self) -> &::rucene_queryparser::classic::query_parser::QueryParser {
                &self.parser
            }

            fn query_parser_mut(
                &mut self,
            ) -> &mut ::rucene_queryparser::classic::query_parser::QueryParser {
                &mut self.parser
            }
        }
    };
}

/// The members of `QueryParserBaseOps` every subclass implements the way
/// `QueryParser` does, inside the subclass's `impl QueryParserBaseOps`.
macro_rules! classic_base_required {
    () => {
        fn query_parser_base(&self) -> &::rucene_queryparser::classic::query_parser_base::QueryParserBase {
            ::rucene_queryparser::classic::query_parser_base::QueryParserBaseOps::query_parser_base(
                &self.parser,
            )
        }

        fn query_parser_base_mut(
            &mut self,
        ) -> &mut ::rucene_queryparser::classic::query_parser_base::QueryParserBase {
            ::rucene_queryparser::classic::query_parser_base::QueryParserBaseOps::query_parser_base_mut(
                &mut self.parser,
            )
        }

        fn re_init(
            &mut self,
            stream: Box<dyn ::rucene_queryparser::charstream::char_stream::CharStream>,
        ) {
            ::rucene_queryparser::classic::query_parser::re_init(self, stream);
        }

        fn top_level_query(
            &mut self,
            field: Option<&str>,
        ) -> ::rucene::error::Result<Option<::std::sync::Arc<dyn ::rucene::search::Query>>> {
            ::rucene_queryparser::classic::query_parser::top_level_query(
                self,
                field.map(str::to_string),
            )
        }

        fn set_auto_generate_phrase_queries(&mut self, value: bool) -> ::rucene::error::Result<()> {
            ::rucene_queryparser::classic::query_parser::set_auto_generate_phrase_queries(self, value)
        }
    };
}

pub(crate) use {classic_base_required, classic_subclass};

/// `TestQueryParser.QPTestParser`: fuzzy and wildcard queries not allowed.
pub struct QpTestParser {
    parser: QueryParser,
}

impl QpTestParser {
    /// `new QPTestParser(String f, Analyzer a)`.
    pub fn new(f: &str, a: Arc<dyn Analyzer>) -> Result<Self> {
        Ok(QpTestParser {
            parser: QueryParser::new(Some(f), Some(a))?,
        })
    }
}

classic_subclass!(QpTestParser);

impl QueryParserBaseOps for QpTestParser {
    classic_base_required!();

    fn get_fuzzy_query(
        &mut self,
        _field: Option<&str>,
        _term_str: &str,
        _min_similarity: f32,
    ) -> Result<Option<Arc<dyn Query>>> {
        Err(ParseException::with_message("Fuzzy queries not allowed").error())
    }

    fn get_wildcard_query(
        &mut self,
        _field: Option<&str>,
        _term_str: &str,
    ) -> Result<Option<Arc<dyn Query>>> {
        Err(ParseException::with_message("Wildcard queries not allowed").error())
    }
}

impl<const EXT: bool> TestQueryParserFixture<EXT> {
    /// `getParser(Analyzer a)`; `TestExtendableQueryParser` overrides it
    /// as `getParser(a, null)`.
    pub fn get_parser(&mut self, a: Option<Arc<dyn Analyzer>>) -> Result<Box<dyn QueryParserOps>> {
        if EXT {
            return self.get_ext_parser(a, None);
        }
        let a = a.unwrap_or_else(|| mock_analyzer(&SIMPLE, true));
        let mut qp = QueryParser::new(Some(&self.get_default_field()), Some(a))?;
        qp.set_default_operator(OR_OPERATOR);
        qp.set_split_on_whitespace(self.split_on_whitespace)?;
        Ok(Box::new(qp))
    }

    /// `TestExtendableQueryParser.getParser(Analyzer a, Extensions extensions)`.
    pub fn get_ext_parser(
        &mut self,
        a: Option<Arc<dyn Analyzer>>,
        extensions: Option<Arc<Extensions>>,
    ) -> Result<Box<dyn QueryParserOps>> {
        let a = a.unwrap_or_else(|| mock_analyzer(&SIMPLE, true));
        let field = self.get_default_field();
        let mut qp: Box<dyn QueryParserOps> = match extensions {
            None => Box::new(ExtendableQueryParser::new(Some(&field), Some(a))?),
            Some(extensions) => Box::new(ExtendableQueryParser::with_extensions(
                Some(&field),
                Some(a),
                extensions,
            )?),
        };
        qp.set_default_operator(OR_OPERATOR);
        qp.set_split_on_whitespace(self.split_on_whitespace)?;
        Ok(qp)
    }
}

impl<const EXT: bool> QueryParserTestBase for TestQueryParserFixture<EXT> {
    type Config = dyn QueryParserOps;

    fn base(&self) -> &QueryParserTestBaseState {
        &self.base
    }

    fn base_mut(&mut self) -> &mut QueryParserTestBaseState {
        &mut self.base
    }

    fn get_parser_config(&mut self, a: Option<Arc<dyn Analyzer>>) -> Result<Box<Self::Config>> {
        self.get_parser(a)
    }

    fn get_query_config(
        &mut self,
        query: &str,
        cqp_c: &mut Self::Config,
    ) -> Result<Arc<dyn Query>> {
        cqp_c.parse(query)
    }

    fn get_query_analyzer(
        &mut self,
        query: &str,
        a: Option<Arc<dyn Analyzer>>,
    ) -> Result<Arc<dyn Query>> {
        self.get_parser(a)?.parse(query)
    }

    fn is_query_parser_exception(&self, exception: &Throwable) -> bool {
        exception.is_instance(CLASSIC_PARSE_EXCEPTION)
    }

    fn set_default_operator_or(&mut self, cqp_c: &mut Self::Config) -> Result<()> {
        cqp_c.set_default_operator(Operator::Or);
        Ok(())
    }

    fn set_default_operator_and(&mut self, cqp_c: &mut Self::Config) -> Result<()> {
        cqp_c.set_default_operator(Operator::And);
        Ok(())
    }

    fn set_auto_generate_phrase_queries(
        &mut self,
        cqp_c: &mut Self::Config,
        value: bool,
    ) -> Result<()> {
        cqp_c.set_auto_generate_phrase_queries(value)
    }

    fn set_date_resolution(
        &mut self,
        cqp_c: &mut Self::Config,
        field: &str,
        value: Resolution,
    ) -> Result<()> {
        cqp_c.set_date_resolution_for_field(Some(field), Some(value))
    }

    fn is_test_query_parser(&self) -> bool {
        true
    }

    fn set_split_on_whitespace_if_classic(&mut self, qp: &mut Self::Config) -> Result<()> {
        qp.set_split_on_whitespace(true)
    }
}

/// The methods of `TestQueryParser` (with those it inherits), as methods
/// of a class whose fixture is `TestQueryParserFixture<EXT>`.
pub fn methods<const EXT: bool>() -> Vec<Method> {
    let mut m = base_methods::<TestQueryParserFixture<EXT>>(&["testQPA"]);
    type F<const E: bool> = TestQueryParserFixture<E>;
    let own: Vec<(&str, FixtureTest<F<EXT>>)> = vec![
        ("testDefaultOperator", test_default_operator::<EXT>),
        ("testProtectedCtors", test_protected_ctors::<EXT>),
        (
            "testFuzzySlopeExtendability",
            test_fuzzy_slope_extendability::<EXT>,
        ),
        (
            "testFuzzyDistanceExtendability",
            test_fuzzy_distance_extendability::<EXT>,
        ),
        ("testStarParsing", test_star_parsing::<EXT>),
        (
            "testCustomQueryParserWildcard",
            test_custom_query_parser_wildcard::<EXT>,
        ),
        (
            "testCustomQueryParserFuzzy",
            test_custom_query_parser_fuzzy::<EXT>,
        ),
        ("testNewFieldQuery", test_new_field_query::<EXT>),
        ("testSynonyms", test_synonyms::<EXT>),
        ("testSynonymsPhrase", test_synonyms_phrase::<EXT>),
        ("testCJKSynonym", test_cjk_synonym::<EXT>),
        ("testCJKSynonymsOR", test_cjk_synonyms_or::<EXT>),
        ("testCJKSynonymsOR2", test_cjk_synonyms_or2::<EXT>),
        ("testCJKSynonymsAND", test_cjk_synonyms_and::<EXT>),
        ("testCJKSynonymsAND2", test_cjk_synonyms_and2::<EXT>),
        ("testCJKSynonymsPhrase", test_cjk_synonyms_phrase::<EXT>),
        (
            "testWildcardDeterminizeWorkLimit",
            test_wildcard_determinize_work_limit::<EXT>,
        ),
        ("testQPA", test_qpa::<EXT>),
        ("testMultiWordSynonyms", test_multi_word_synonyms::<EXT>),
        ("testEnableGraphQueries", test_enable_graph_queries::<EXT>),
        (
            "testOperatorsAndMultiWordSynonyms",
            test_operators_and_multi_word_synonyms::<EXT>,
        ),
        (
            "testOperatorsAndMultiWordSynonymsSplitOnWhitespace",
            test_operators_and_multi_word_synonyms_split_on_whitespace::<EXT>,
        ),
        (
            "testDefaultSplitOnWhitespace",
            test_default_split_on_whitespace::<EXT>,
        ),
        ("testWildcardAlone", test_wildcard_alone::<EXT>),
        ("testWildCardEscapes", test_wild_card_escapes::<EXT>),
        (
            "testWildcardDoesNotNormalizeEscapedChars",
            test_wildcard_does_not_normalize_escaped_chars::<EXT>,
        ),
        ("testWildCardQuery", test_wild_card_query::<EXT>),
        ("testPrefixQuery", test_prefix_query::<EXT>),
        ("testRangeQuery", test_range_query::<EXT>),
        ("testFuzzyQuery", test_fuzzy_query::<EXT>),
        ("testByteTerms", test_byte_terms::<EXT>),
        (
            "test_splitOnWhitespace_with_autoGeneratePhraseQueries",
            test_split_on_whitespace_with_auto_generate_phrase_queries::<EXT>,
        ),
    ];
    for (name, body) in own {
        m.push(ftest(name, body));
    }
    m
}

/// `TestQueryParser`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<TestQueryParserFixture<false>>(
        "org.apache.lucene.queryparser.classic.TestQueryParser",
        methods::<false>(),
    )
}

fn assert_operator(expected: Operator, actual: Operator) {
    // `Operator.toString()` is the constant's name.
    let name = |o: Operator| format!("{o:?}").to_uppercase();
    Assert::assert_equals(name(expected), name(actual));
}

fn test_default_operator<const EXT: bool>(t: &mut TestQueryParserFixture<EXT>) -> Result<()> {
    let mut qp = t.get_parser(Some(mock_analyzer_default()))?;
    // make sure OR is the default:
    assert_operator(OR_OPERATOR, qp.get_default_operator());
    t.set_default_operator_and(&mut *qp)?;
    assert_operator(AND_OPERATOR, qp.get_default_operator());
    t.set_default_operator_or(&mut *qp)?;
    assert_operator(OR_OPERATOR, qp.get_default_operator());
    Ok(())
}

/// The public constructors of `QueryParser` as Java's
/// `Class.getConstructor` finds them: Rust has no reflection, so they are
/// declared here (the precedent of the sprint-9 reflection tests). Lucene's
/// build makes the generated `QueryParser(CharStream)` and
/// `QueryParser(QueryParserTokenManager)` protected; their Rust
/// counterparts `QueryParser::with_char_stream` and
/// `QueryParser::with_token_manager` are `pub` only because Rust has no
/// `protected` for the subclasses that embed a parser.
const QUERY_PARSER_PUBLIC_CONSTRUCTORS: &[&[&str]] =
    &[&["java.lang.String", "org.apache.lucene.analysis.Analyzer"]];

/// `QueryParser.class.getConstructor(parameterTypes)`.
fn get_constructor(parameter_types: &[&str]) -> Result<()> {
    if QUERY_PARSER_PUBLIC_CONSTRUCTORS.contains(&parameter_types) {
        Ok(())
    } else {
        Err(LuceneError::from(LuceneErrorKind::NoSuchMethod(format!(
            "org.apache.lucene.queryparser.classic.QueryParser.<init>({})",
            parameter_types.join(", ")
        ))))
    }
}

// LUCENE-2002: when we run javacc to regen QueryParser,
// we also run a replaceregexp step to fix 2 of the public
// ctors (change them to protected):
//
// protected QueryParser(CharStream stream)
//
// protected QueryParser(QueryParserTokenManager tm)
//
// This test is here as a safety, in case that ant step
// doesn't work for some reason.
fn test_protected_ctors<const EXT: bool>(_t: &mut TestQueryParserFixture<EXT>) -> Result<()> {
    expect_throws("java.lang.NoSuchMethodException", || {
        get_constructor(&["org.apache.lucene.queryparser.charstream.CharStream"])
    });
    expect_throws("java.lang.NoSuchMethodException", || {
        get_constructor(&["org.apache.lucene.queryparser.classic.QueryParserTokenManager"])
    });
    Ok(())
}

/// `Float.parseFloat(String)`.
fn parse_float(s: &str) -> Result<f32> {
    rucene::util::java_fmt::parse_float(s)
        .ok_or_else(|| LuceneError::number_format(format!("For input string: \"{s}\"")))
}

/// `Float.toString(float)`.
fn float_to_string(f: f32) -> String {
    rucene::util::java_fmt::float_to_string(f)
}

/// The image of a token as UTF-16 units.
fn image_units(token: &TokenRef) -> Vec<u16> {
    token
        .lock()
        .image
        .clone()
        .unwrap_or_default()
        .encode_utf16()
        .collect()
}

/// The anonymous subclass of `testFuzzySlopeExtendability`.
struct FuzzySlopeParser {
    parser: QueryParser,
}

classic_subclass!(FuzzySlopeParser);

impl QueryParserBaseOps for FuzzySlopeParser {
    classic_base_required!();

    fn handle_bare_fuzzy(
        &mut self,
        qfield: Option<&str>,
        fuzzy_slop: Option<&TokenRef>,
        term_image: &str,
    ) -> Result<Option<Arc<dyn Query>>> {
        let slop =
            crate::qp_support::catch_java(|| fuzzy_slop.ok_or_else(LuceneError::null_pointer))
                .unwrap_or_else(|t| t.throw());
        let image = image_units(slop);
        if image.last() == Some(&u16::from_str_radix("20AC", 16).unwrap_or(0)) {
            let mut fms = self.get_fuzzy_min_sim()?;
            // `try { fms = Float.parseFloat(image.substring(1, length - 1)); }
            // catch (Exception ignored) {}`
            if image.len() >= 2 {
                if let Ok(v) = parse_float(&String::from_utf16_lossy(&image[1..image.len() - 1])) {
                    fms = v;
                }
            }
            let value = parse_float(term_image)?;
            return self.get_range_query(
                qfield,
                Some(&float_to_string(value - fms / 2.0)),
                Some(&float_to_string(value + fms / 2.0)),
                true,
                true,
            );
        }
        query_parser_base::handle_bare_fuzzy(self, qfield, fuzzy_slop, term_image)
    }
}

fn test_fuzzy_slope_extendability<const EXT: bool>(
    _t: &mut TestQueryParserFixture<EXT>,
) -> Result<()> {
    let mut qp = FuzzySlopeParser {
        parser: QueryParser::new(Some("a"), Some(mock_analyzer(&WHITESPACE, false)))?,
    };
    let expected = qp.parse("a:[11.95 TO 12.95]")?;
    assert_query_eq(&*expected, &*qp.parse("12.45~1€")?);
    Ok(())
}

/// The anonymous subclasses of `testFuzzyDistanceExtendability`.
struct FuzzyDistanceParser {
    parser: QueryParser,
    by_length: bool,
}

classic_subclass!(FuzzyDistanceParser);

impl QueryParserBaseOps for FuzzyDistanceParser {
    classic_base_required!();

    fn get_fuzzy_distance(&mut self, fuzzy_slop: Option<&TokenRef>, term_str: &str) -> f32 {
        if self.by_length {
            return term_str.encode_utf16().count() as f32; // distance based on the term length
        }
        // `try { return Float.parseFloat(fuzzySlop.image.substring(1)); }
        // catch (Exception ignored) {}`
        if let Some(slop) = fuzzy_slop {
            let image = image_units(slop);
            if !image.is_empty() {
                if let Ok(v) = parse_float(&String::from_utf16_lossy(&image[1..])) {
                    return v;
                }
            }
        }
        1.0 // alternative value to the default min similarity
    }
}

fn test_fuzzy_distance_extendability<const EXT: bool>(
    _t: &mut TestQueryParserFixture<EXT>,
) -> Result<()> {
    let mut qp = FuzzyDistanceParser {
        parser: QueryParser::new(Some("a"), Some(mock_analyzer(&WHITESPACE, false)))?,
        by_length: false,
    };
    assert_query_eq(&*qp.parse("term~")?, &*qp.parse("term~1")?);
    assert_query_eq(&*qp.parse("term~XXX")?, &*qp.parse("term~1")?);

    let mut qp2 = FuzzyDistanceParser {
        parser: QueryParser::new(Some("a"), Some(mock_analyzer(&WHITESPACE, false)))?,
        by_length: true,
    };
    assert_query_eq(&*qp2.parse("a~")?, &*qp2.parse("a~1")?);
    assert_query_eq(&*qp2.parse("ab~")?, &*qp2.parse("ab~2")?);
    Ok(())
}

/// The anonymous subclass of `testStarParsing`: `type` records which
/// factory ran.
struct StarParser {
    parser: QueryParser,
    type_: Arc<AtomicI32>,
}

classic_subclass!(StarParser);

impl QueryParserBaseOps for StarParser {
    classic_base_required!();

    fn get_wildcard_query(
        &mut self,
        field: Option<&str>,
        term_str: &str,
    ) -> Result<Option<Arc<dyn Query>>> {
        // override error checking of superclass
        self.type_.store(1, Ordering::Relaxed);
        Ok(Some(Arc::new(TermQuery::new(
            rucene::index::Term::from_text_nullable_field(field, term_str),
        ))))
    }

    fn get_prefix_query(
        &mut self,
        field: Option<&str>,
        term_str: &str,
    ) -> Result<Option<Arc<dyn Query>>> {
        // override error checking of superclass
        self.type_.store(2, Ordering::Relaxed);
        Ok(Some(Arc::new(TermQuery::new(
            rucene::index::Term::from_text_nullable_field(field, term_str),
        ))))
    }

    fn get_field_query(
        &mut self,
        field: Option<&str>,
        query_text: &str,
        quoted: bool,
    ) -> Result<Option<Arc<dyn Query>>> {
        self.type_.store(3, Ordering::Relaxed);
        query_parser_base::get_field_query(self, field, query_text, quoted)
    }
}

fn tq_of(q: &dyn Query) -> &TermQuery {
    cast::<TermQuery>(q, "org.apache.lucene.search.TermQuery")
}

fn test_star_parsing<const EXT: bool>(_t: &mut TestQueryParserFixture<EXT>) -> Result<()> {
    let type_ = Arc::new(AtomicI32::new(0));
    let mut qp = StarParser {
        parser: QueryParser::new(Some(FIELD), Some(mock_analyzer(&WHITESPACE, false)))?,
        type_: Arc::clone(&type_),
    };
    let ty = || type_.load(Ordering::Relaxed);

    let q = qp.parse("foo:zoo*")?;
    Assert::assert_equals("zoo".to_string(), tq_of(&*q).get_term().text());
    Assert::assert_equals(2, ty());

    let q = qp.parse("foo:zoo*^2")?;
    let bq = cast::<BoostQuery>(&*q, "org.apache.lucene.search.BoostQuery");
    let inner = bq.get_query();
    Assert::assert_equals("zoo".to_string(), tq_of(&*inner).get_term().text());
    Assert::assert_equals(2, ty());
    Assert::assert_equals_float_msg(None, bq.get_boost(), 2.0, 0.0);

    let q = qp.parse("foo:*")?;
    Assert::assert_equals("*".to_string(), tq_of(&*q).get_term().text());
    Assert::assert_equals(1, ty()); // could be a valid prefix query in the future too

    let q = qp.parse("foo:*^2")?;
    let bq = cast::<BoostQuery>(&*q, "org.apache.lucene.search.BoostQuery");
    let inner = bq.get_query();
    Assert::assert_equals("*".to_string(), tq_of(&*inner).get_term().text());
    Assert::assert_equals(1, ty());
    Assert::assert_equals_float_msg(None, bq.get_boost(), 2.0, 0.0);

    let q = qp.parse("*:foo")?;
    Assert::assert_equals("*".to_string(), tq_of(&*q).get_term().field().to_string());
    Assert::assert_equals("foo".to_string(), tq_of(&*q).get_term().text());
    Assert::assert_equals(3, ty());

    let q = qp.parse("*:*")?;
    Assert::assert_equals("*".to_string(), tq_of(&*q).get_term().field().to_string());
    Assert::assert_equals("*".to_string(), tq_of(&*q).get_term().text());
    Assert::assert_equals(1, ty()); // could be handled as a prefix query in the
                                    // future

    let q = qp.parse("(*:*)")?;
    Assert::assert_equals("*".to_string(), tq_of(&*q).get_term().field().to_string());
    Assert::assert_equals("*".to_string(), tq_of(&*q).get_term().text());
    Assert::assert_equals(1, ty());
    Ok(())
}

// Wildcard queries should not be allowed
fn test_custom_query_parser_wildcard<const EXT: bool>(
    _t: &mut TestQueryParserFixture<EXT>,
) -> Result<()> {
    expect_throws(CLASSIC_PARSE_EXCEPTION, || {
        QpTestParser::new("contents", mock_analyzer(&WHITESPACE, false))?.parse("a?t")
    });
    Ok(())
}

// Fuzzy queries should not be allowed
fn test_custom_query_parser_fuzzy<const EXT: bool>(
    _t: &mut TestQueryParserFixture<EXT>,
) -> Result<()> {
    expect_throws(CLASSIC_PARSE_EXCEPTION, || {
        QpTestParser::new("contents", mock_analyzer(&WHITESPACE, false))?.parse("xunit~")
    });
    Ok(())
}

/// query parser that doesn't expand synonyms when users use double quotes
struct SmartQueryParser {
    parser: QueryParser,
    more_precise: Arc<dyn Analyzer>,
}

impl SmartQueryParser {
    fn new() -> Result<Self> {
        Ok(SmartQueryParser {
            more_precise: analyzer2(),
            parser: QueryParser::new(Some(FIELD), Some(analyzer1()))?,
        })
    }
}

classic_subclass!(SmartQueryParser);

impl QueryParserBaseOps for SmartQueryParser {
    classic_base_required!();

    fn get_field_query(
        &mut self,
        field: Option<&str>,
        query_text: &str,
        quoted: bool,
    ) -> Result<Option<Arc<dyn Query>>> {
        if quoted {
            let a = Arc::clone(&self.more_precise);
            self.new_field_query(Some(a), field, query_text, quoted)
        } else {
            query_parser_base::get_field_query(self, field, query_text, quoted)
        }
    }
}

fn synonym(field: &str, terms: &[&str]) -> Result<Arc<dyn Query>> {
    let mut b = SynonymQueryBuilder::new(Some(field));
    for t in terms {
        b.add_term(&term(field, t))?;
    }
    Ok(Arc::new(b.build()))
}

fn boost(q: Arc<dyn Query>, b: f32) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(BoostQuery::new(q, b)?))
}

fn bq(clauses: Vec<(Arc<dyn Query>, Occur)>) -> Result<Arc<dyn Query>> {
    let mut b = BooleanQueryBuilder::new();
    for (q, o) in clauses {
        b.add(q, o)?;
    }
    Ok(Arc::new(b.build()))
}

fn test_new_field_query<const EXT: bool>(_t: &mut TestQueryParserFixture<EXT>) -> Result<()> {
    /* ordinary behavior, synonyms form uncoordinated boolean query */
    let mut dumb = QueryParser::new(Some(FIELD), Some(analyzer1()))?;
    let expanded = synonym(FIELD, &["dogs", "dog"])?;
    assert_query_eq(&*expanded, &*dumb.parse("\"dogs\"")?);
    /* even with the phrase operator the behavior is the same */
    assert_query_eq(&*expanded, &*dumb.parse("dogs")?);

    /* custom behavior, the synonyms are expanded, unless you use quote operator */
    let mut smart = SmartQueryParser::new()?;
    assert_query_eq(&*expanded, &*smart.parse("dogs")?);

    let unexpanded = tq(FIELD, "dogs");
    assert_query_eq(&*unexpanded, &*smart.parse("\"dogs\"")?);
    Ok(())
}

/// simple synonyms test
fn test_synonyms<const EXT: bool>(_t: &mut TestQueryParserFixture<EXT>) -> Result<()> {
    let expected = synonym(FIELD, &["dogs", "dog"])?;
    let mut qp = QueryParser::new(Some(FIELD), Some(Arc::new(MockSynonymAnalyzer::new())))?;
    assert_query_eq(&*expected, &*qp.parse("dogs")?);
    assert_query_eq(&*expected, &*qp.parse("\"dogs\"")?);
    qp.set_default_operator(Operator::And);
    assert_query_eq(&*expected, &*qp.parse("dogs")?);
    assert_query_eq(&*expected, &*qp.parse("\"dogs\"")?);
    let expected = boost(expected, 2.0)?;
    assert_query_eq(&*expected, &*qp.parse("dogs^2")?);
    assert_query_eq(&*expected, &*qp.parse("\"dogs\"^2")?);
    Ok(())
}

/// forms multiphrase query
fn test_synonyms_phrase<const EXT: bool>(_t: &mut TestQueryParserFixture<EXT>) -> Result<()> {
    let mut expected_q_builder = MultiPhraseQueryBuilder::new();
    expected_q_builder.add(term(FIELD, "old"))?;
    expected_q_builder.add_terms(vec![term(FIELD, "dogs"), term(FIELD, "dog")])?;
    let mut qp = QueryParser::new(Some(FIELD), Some(Arc::new(MockSynonymAnalyzer::new())))?;
    assert_query_eq(&expected_q_builder.build(), &*qp.parse("\"old dogs\"")?);
    qp.set_default_operator(Operator::And);
    assert_query_eq(&expected_q_builder.build(), &*qp.parse("\"old dogs\"")?);
    let expected = boost(Arc::new(expected_q_builder.build()), 2.0)?;
    assert_query_eq(&*expected, &*qp.parse("\"old dogs\"^2")?);
    expected_q_builder.set_slop(3)?;
    let expected = boost(Arc::new(expected_q_builder.build()), 2.0)?;
    assert_query_eq(&*expected, &*qp.parse("\"old dogs\"~3^2")?);
    Ok(())
}

/// The logic of `TestQueryParser.MockCJKSynonymFilter`: adds synonym of
/// "國" for "国".
#[derive(Debug)]
struct MockCjkSynonymFilterLogic {
    term_att: Att,
    pos_inc_att: Att,
    add_synonym: bool,
}

impl TokenFilterLogic for MockCjkSynonymFilterLogic {
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if self.add_synonym {
            // inject our synonym
            source.clear_attributes();
            self.term_att.set_term("國");
            self.pos_inc_att.pos_inc_mut().set_position_increment(0);
            self.add_synonym = false;
            return Ok(true);
        }

        if input.increment_token()? {
            self.add_synonym = self.term_att.term_string() == "国";
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// `new MockCJKSynonymAnalyzer()`.
fn mock_cjk_synonym_analyzer() -> Arc<dyn Analyzer> {
    FnAnalyzer::new(
        "org.apache.lucene.queryparser.classic.TestQueryParser$MockCJKSynonymAnalyzer",
        |_| {
            let tokenizer = simple_cjk_tokenizer()?;
            let mut input = shared(&tokenizer);
            let term_att = Att::add(input.attribute_source_mut(), char_term_id())?;
            let pos_inc_att = Att::add(input.attribute_source_mut(), pos_inc_id())?;
            let filter = TokenFilter::new(
                input,
                MockCjkSynonymFilterLogic {
                    term_att,
                    pos_inc_att,
                    add_synonym: false,
                },
            );
            Ok(components(tokenizer, sink(filter)))
        },
    )
    .arc()
}

/// simple CJK synonym test
fn test_cjk_synonym<const EXT: bool>(_t: &mut TestQueryParserFixture<EXT>) -> Result<()> {
    let expected = synonym(FIELD, &["国", "國"])?;
    let mut qp = QueryParser::new(Some(FIELD), Some(mock_cjk_synonym_analyzer()))?;
    assert_query_eq(&*expected, &*qp.parse("国")?);
    qp.set_default_operator(Operator::And);
    assert_query_eq(&*expected, &*qp.parse("国")?);
    let expected = boost(expected, 2.0)?;
    assert_query_eq(&*expected, &*qp.parse("国^2")?);
    Ok(())
}

/// synonyms with default OR operator
fn test_cjk_synonyms_or<const EXT: bool>(_t: &mut TestQueryParserFixture<EXT>) -> Result<()> {
    let inner = synonym(FIELD, &["国", "國"])?;
    let expected = bq(vec![
        (tq(FIELD, "中"), Occur::SHOULD),
        (inner, Occur::SHOULD),
    ])?;
    let mut qp = QueryParser::new(Some(FIELD), Some(mock_cjk_synonym_analyzer()))?;
    assert_query_eq(&*expected, &*qp.parse("中国")?);
    let expected = boost(expected, 2.0)?;
    assert_query_eq(&*expected, &*qp.parse("中国^2")?);
    Ok(())
}

/// more complex synonyms with default OR operator
fn test_cjk_synonyms_or2<const EXT: bool>(_t: &mut TestQueryParserFixture<EXT>) -> Result<()> {
    let inner = synonym(FIELD, &["国", "國"])?;
    let inner2 = synonym(FIELD, &["国", "國"])?;
    let expected = bq(vec![
        (tq(FIELD, "中"), Occur::SHOULD),
        (inner, Occur::SHOULD),
        (inner2, Occur::SHOULD),
    ])?;
    let mut qp = QueryParser::new(Some(FIELD), Some(mock_cjk_synonym_analyzer()))?;
    assert_query_eq(&*expected, &*qp.parse("中国国")?);
    let expected = boost(expected, 2.0)?;
    assert_query_eq(&*expected, &*qp.parse("中国国^2")?);
    Ok(())
}

/// synonyms with default AND operator
fn test_cjk_synonyms_and<const EXT: bool>(_t: &mut TestQueryParserFixture<EXT>) -> Result<()> {
    let inner = synonym(FIELD, &["国", "國"])?;
    let expected = bq(vec![(tq(FIELD, "中"), Occur::MUST), (inner, Occur::MUST)])?;
    let mut qp = QueryParser::new(Some(FIELD), Some(mock_cjk_synonym_analyzer()))?;
    qp.set_default_operator(Operator::And);
    assert_query_eq(&*expected, &*qp.parse("中国")?);
    let expected = boost(expected, 2.0)?;
    assert_query_eq(&*expected, &*qp.parse("中国^2")?);
    Ok(())
}

/// more complex synonyms with default AND operator
fn test_cjk_synonyms_and2<const EXT: bool>(_t: &mut TestQueryParserFixture<EXT>) -> Result<()> {
    let inner = synonym(FIELD, &["国", "國"])?;
    let inner2 = synonym(FIELD, &["国", "國"])?;
    let expected = bq(vec![
        (tq(FIELD, "中"), Occur::MUST),
        (inner, Occur::MUST),
        (inner2, Occur::MUST),
    ])?;
    let mut qp = QueryParser::new(Some(FIELD), Some(mock_cjk_synonym_analyzer()))?;
    qp.set_default_operator(Operator::And);
    assert_query_eq(&*expected, &*qp.parse("中国国")?);
    let expected = boost(expected, 2.0)?;
    assert_query_eq(&*expected, &*qp.parse("中国国^2")?);
    Ok(())
}

/// forms multiphrase query
fn test_cjk_synonyms_phrase<const EXT: bool>(_t: &mut TestQueryParserFixture<EXT>) -> Result<()> {
    let mut expected_q_builder = MultiPhraseQueryBuilder::new();
    expected_q_builder.add(term(FIELD, "中"))?;
    expected_q_builder.add_terms(vec![term(FIELD, "国"), term(FIELD, "國")])?;
    let mut qp = QueryParser::new(Some(FIELD), Some(mock_cjk_synonym_analyzer()))?;
    qp.set_default_operator(Operator::And);
    assert_query_eq(&expected_q_builder.build(), &*qp.parse("\"中国\"")?);
    let expected = boost(Arc::new(expected_q_builder.build()), 2.0)?;
    assert_query_eq(&*expected, &*qp.parse("\"中国\"^2")?);
    expected_q_builder.set_slop(3)?;
    let expected = boost(Arc::new(expected_q_builder.build()), 2.0)?;
    assert_query_eq(&*expected, &*qp.parse("\"中国\"~3^2")?);
    Ok(())
}

/// LUCENE-6677: make sure wildcard query respects determinizeWorkLimit.
fn test_wildcard_determinize_work_limit<const EXT: bool>(
    _t: &mut TestQueryParserFixture<EXT>,
) -> Result<()> {
    let mut qp = QueryParser::new(Some(FIELD), Some(mock_analyzer_default()))?;
    qp.set_determinize_work_limit(1);
    expect_throws(
        "org.apache.lucene.util.automaton.TooComplexToDeterminizeException",
        || qp.parse("a*aaaaaaa"),
    );
    Ok(())
}

// TODO: Remove this specialization once the flexible standard parser gets multi-word synonym
// support
fn test_qpa<const EXT: bool>(t: &mut TestQueryParserFixture<EXT>) -> Result<()> {
    let old_split_on_whitespace = t.split_on_whitespace;
    t.split_on_whitespace = false;

    assert_query_equals(
        t,
        "term phrase term",
        Some(qp_analyzer()),
        "term phrase1 phrase2 term",
    )?;

    let mut cqpc = t.get_parser_config(Some(qp_analyzer()))?;
    t.set_default_operator_and(&mut *cqpc)?;
    assert_query_equals_config(
        t,
        &mut *cqpc,
        "field",
        "term phrase term",
        "+term +phrase1 +phrase2 +term",
    )?;

    t.split_on_whitespace = old_split_on_whitespace;
    Ok(())
}

// TODO: Move to QueryParserTestBase once standard flexible parser gets this capability
fn test_multi_word_synonyms<const EXT: bool>(_t: &mut TestQueryParserFixture<EXT>) -> Result<()> {
    let mut dumb = QueryParser::new(Some("field"), Some(analyzer1()))?;
    dumb.set_split_on_whitespace(false)?;

    let guinea = tq("field", "guinea");
    let pig = tq("field", "pig");
    let cavy = tq("field", "cavy");

    // A multi-word synonym source will form a graph query for synonyms that formed the graph token
    // stream
    let guinea_pig = bq(vec![
        (Arc::clone(&guinea), Occur::MUST),
        (Arc::clone(&pig), Occur::MUST),
    ])?;

    let mut pqb = PhraseQueryBuilder::new();
    pqb.add(term("field", "guinea"))?;
    pqb.add(term("field", "pig"))?;
    let phrase_guinea_pig = pqb.build()?;

    let graph_query = bq(vec![(
        bq(vec![
            (Arc::clone(&guinea_pig), Occur::SHOULD),
            (Arc::clone(&cavy), Occur::SHOULD),
        ])?,
        Occur::SHOULD,
    )])?;
    assert_query_eq(&*graph_query, &*dumb.parse("guinea pig")?);

    let synonyms = bq(vec![
        (
            Arc::new(PhraseQuery::from_strings("field", &["guinea", "pig"])?),
            Occur::SHOULD,
        ),
        (tq("field", "cavy"), Occur::SHOULD),
    ])?;
    assert_query_eq(&*synonyms, &*dumb.parse("\"guinea pig\"")?);

    // custom behavior, the synonyms are expanded, unless you use quote operator
    let mut smart = SmartQueryParser::new()?;
    smart.set_split_on_whitespace(false)?;
    let graph_query = bq(vec![(
        bq(vec![
            (Arc::clone(&guinea_pig), Occur::SHOULD),
            (Arc::clone(&cavy), Occur::SHOULD),
        ])?,
        Occur::SHOULD,
    )])?;
    assert_query_eq(&*graph_query, &*smart.parse("guinea pig")?);
    assert_query_eq(&phrase_guinea_pig, &*smart.parse("\"guinea pig\"")?);

    // with the AND operator
    dumb.set_default_operator(Operator::And);
    let graph_and_query = bq(vec![(
        bq(vec![
            (Arc::clone(&guinea_pig), Occur::SHOULD),
            (Arc::clone(&cavy), Occur::SHOULD),
        ])?,
        Occur::MUST,
    )])?;
    assert_query_eq(&*graph_and_query, &*dumb.parse("guinea pig")?);

    let graph_and_query = bq(vec![
        (
            bq(vec![
                (Arc::clone(&guinea_pig), Occur::SHOULD),
                (Arc::clone(&cavy), Occur::SHOULD),
            ])?,
            Occur::MUST,
        ),
        (Arc::clone(&cavy), Occur::MUST),
    ])?;
    assert_query_eq(&*graph_and_query, &*dumb.parse("guinea pig cavy")?);
    Ok(())
}

fn test_enable_graph_queries<const EXT: bool>(_t: &mut TestQueryParserFixture<EXT>) -> Result<()> {
    let mut dumb = QueryParser::new(Some("field"), Some(analyzer1()))?;
    dumb.set_split_on_whitespace(false)?;
    dumb.query_parser_base_mut()
        .query_builder_mut()
        .set_enable_graph_queries(false);

    let pig = tq("field", "pig");

    // A multi-word synonym source will just form a boolean query when graph queries are disabled:
    let inner = synonym("field", &["cavy", "guinea"])?;
    let query = bq(vec![(inner, Occur::SHOULD), (pig, Occur::SHOULD)])?;
    assert_query_eq(&*query, &*dumb.parse("guinea pig")?);
    Ok(())
}

/// The `(query, result)` pairs of the two multi-word synonym tests that do
/// not depend on `splitOnWhitespace`.
const OPERATORS_COMMON: &[(&str, &str)] = &[
    ("+guinea pig", "+guinea pig"),
    ("-guinea pig", "-guinea pig"),
    ("!guinea pig", "-guinea pig"),
    ("guinea* pig", "guinea* pig"),
    ("guinea? pig", "guinea? pig"),
    ("guinea~2 pig", "guinea~2 pig"),
    ("guinea^2 pig", "(guinea)^2.0 pig"),
    ("guinea +pig", "guinea +pig"),
    ("guinea -pig", "guinea -pig"),
    ("guinea !pig", "guinea -pig"),
    ("guinea pig*", "guinea pig*"),
    ("guinea pig?", "guinea pig?"),
    ("guinea pig~2", "guinea pig~2"),
    ("guinea pig^2", "guinea (pig)^2.0"),
    ("field:guinea pig", "guinea pig"),
    ("guinea field:pig", "guinea pig"),
    ("NOT guinea pig", "-guinea pig"),
    ("guinea NOT pig", "guinea -pig"),
    ("guinea pig AND dogs", "guinea +pig +Synonym(dog dogs)"),
    ("dogs AND guinea pig", "+Synonym(dog dogs) +guinea pig"),
    ("guinea pig && dogs", "guinea +pig +Synonym(dog dogs)"),
    ("dogs && guinea pig", "+Synonym(dog dogs) +guinea pig"),
    ("guinea pig OR dogs", "guinea pig Synonym(dog dogs)"),
    ("dogs OR guinea pig", "Synonym(dog dogs) guinea pig"),
    ("guinea pig || dogs", "guinea pig Synonym(dog dogs)"),
    ("dogs || guinea pig", "Synonym(dog dogs) guinea pig"),
    ("\"guinea\" pig", "guinea pig"),
    ("guinea \"pig\"", "guinea pig"),
    ("(guinea) pig", "guinea pig"),
    ("guinea (pig)", "guinea pig"),
    ("/guinea/ pig", "/guinea/ pig"),
    ("guinea /pig/", "guinea /pig/"),
];

// TODO: Move to QueryParserTestBase once standard flexible parser gets this capability
fn test_operators_and_multi_word_synonyms<const EXT: bool>(
    t: &mut TestQueryParserFixture<EXT>,
) -> Result<()> {
    let a: Arc<dyn Analyzer> = Arc::new(MockSynonymAnalyzer::new());

    let old_split_on_whitespace = t.split_on_whitespace;
    t.split_on_whitespace = false;

    // Operators should interrupt multiword analysis of adjacent words if they associate
    for (q, r) in OPERATORS_COMMON {
        assert_query_equals(t, q, Some(Arc::clone(&a)), r)?;
    }

    // Operators should not interrupt multiword analysis if not don't associate
    for (q, r) in [
        ("(guinea pig)", "((+guinea +pig) cavy)"),
        ("+(guinea pig)", "+(((+guinea +pig) cavy))"),
        ("-(guinea pig)", "-(((+guinea +pig) cavy))"),
        ("!(guinea pig)", "-(((+guinea +pig) cavy))"),
        ("NOT (guinea pig)", "-(((+guinea +pig) cavy))"),
        ("(guinea pig)^2", "(((+guinea +pig) cavy))^2.0"),
        ("field:(guinea pig)", "((+guinea +pig) cavy)"),
        ("+small guinea pig", "+small ((+guinea +pig) cavy)"),
        ("-small guinea pig", "-small ((+guinea +pig) cavy)"),
        ("!small guinea pig", "-small ((+guinea +pig) cavy)"),
        ("NOT small guinea pig", "-small ((+guinea +pig) cavy)"),
        ("small* guinea pig", "small* ((+guinea +pig) cavy)"),
        ("small? guinea pig", "small? ((+guinea +pig) cavy)"),
        ("\"small\" guinea pig", "small ((+guinea +pig) cavy)"),
        ("guinea pig +running", "((+guinea +pig) cavy) +running"),
        ("guinea pig -running", "((+guinea +pig) cavy) -running"),
        ("guinea pig !running", "((+guinea +pig) cavy) -running"),
        ("guinea pig NOT running", "((+guinea +pig) cavy) -running"),
        ("guinea pig running*", "((+guinea +pig) cavy) running*"),
        ("guinea pig running?", "((+guinea +pig) cavy) running?"),
        ("guinea pig \"running\"", "((+guinea +pig) cavy) running"),
        ("\"guinea pig\"~2", "\"guinea pig\" cavy"),
        ("field:\"guinea pig\"", "\"guinea pig\" cavy"),
    ] {
        assert_query_equals(t, q, Some(Arc::clone(&a)), r)?;
    }

    t.split_on_whitespace = old_split_on_whitespace;
    Ok(())
}

fn test_operators_and_multi_word_synonyms_split_on_whitespace<const EXT: bool>(
    t: &mut TestQueryParserFixture<EXT>,
) -> Result<()> {
    let a: Arc<dyn Analyzer> = Arc::new(MockSynonymAnalyzer::new());

    let old_split_on_whitespace = t.split_on_whitespace;
    t.split_on_whitespace = true;

    for (q, r) in OPERATORS_COMMON {
        assert_query_equals(t, q, Some(Arc::clone(&a)), r)?;
    }

    for (q, r) in [
        ("(guinea pig)", "guinea pig"),
        ("+(guinea pig)", "+(guinea pig)"),
        ("-(guinea pig)", "-(guinea pig)"),
        ("!(guinea pig)", "-(guinea pig)"),
        ("NOT (guinea pig)", "-(guinea pig)"),
        ("(guinea pig)^2", "(guinea pig)^2.0"),
        ("field:(guinea pig)", "guinea pig"),
        ("+small guinea pig", "+small guinea pig"),
        ("-small guinea pig", "-small guinea pig"),
        ("!small guinea pig", "-small guinea pig"),
        ("NOT small guinea pig", "-small guinea pig"),
        ("small* guinea pig", "small* guinea pig"),
        ("small? guinea pig", "small? guinea pig"),
        ("\"small\" guinea pig", "small guinea pig"),
        ("guinea pig +running", "guinea pig +running"),
        ("guinea pig -running", "guinea pig -running"),
        ("guinea pig !running", "guinea pig -running"),
        ("guinea pig NOT running", "guinea pig -running"),
        ("guinea pig running*", "guinea pig running*"),
        ("guinea pig running?", "guinea pig running?"),
        ("guinea pig \"running\"", "guinea pig running"),
        ("\"guinea pig\"~2", "\"guinea pig\" cavy"),
        ("field:\"guinea pig\"", "\"guinea pig\" cavy"),
    ] {
        assert_query_equals(t, q, Some(Arc::clone(&a)), r)?;
    }

    t.split_on_whitespace = old_split_on_whitespace;
    Ok(())
}

fn test_default_split_on_whitespace<const EXT: bool>(
    t: &mut TestQueryParserFixture<EXT>,
) -> Result<()> {
    let mut parser = QueryParser::new(Some("field"), Some(analyzer1()))?;

    Assert::assert_false(parser.get_split_on_whitespace()); // default is false

    // A multi-word synonym source will form a synonym query for the same-starting-position tokens
    let guinea = tq("field", "guinea");
    let pig = tq("field", "pig");
    let cavy = tq("field", "cavy");

    // A multi-word synonym source will form a graph query for synonyms that formed the graph token
    // stream
    let guinea_pig = bq(vec![(guinea, Occur::MUST), (pig, Occur::MUST)])?;

    let graph_query = bq(vec![(
        bq(vec![(guinea_pig, Occur::SHOULD), (cavy, Occur::SHOULD)])?,
        Occur::SHOULD,
    )])?;
    assert_query_eq(&*graph_query, &*parser.parse("guinea pig")?);

    let old_split_on_whitespace = t.split_on_whitespace;
    t.split_on_whitespace = DEFAULT_SPLIT_ON_WHITESPACE;
    assert_query_equals(
        t,
        "guinea pig",
        Some(Arc::new(MockSynonymAnalyzer::new())),
        "((+guinea +pig) cavy)",
    )?;
    t.split_on_whitespace = old_split_on_whitespace;
    Ok(())
}

fn test_wildcard_alone<const EXT: bool>(_t: &mut TestQueryParserFixture<EXT>) -> Result<()> {
    // seems like crazy edge case, but can be useful in concordance
    let mut parser = QueryParser::new(Some(FIELD), Some(ascii_analyzer()))?;
    parser.set_allow_leading_wildcard(false)?;
    expect_throws(CLASSIC_PARSE_EXCEPTION, || parser.parse("*"));

    let mut parser2 = QueryParser::new(Some("*"), Some(ascii_analyzer()))?;
    parser2.set_allow_leading_wildcard(false)?;
    assert_query_eq(&MatchAllDocsQuery::instance(), &*parser2.parse("*")?);
    Ok(())
}

fn test_wild_card_escapes<const EXT: bool>(_t: &mut TestQueryParserFixture<EXT>) -> Result<()> {
    let a = ascii_analyzer();
    let mut parser = QueryParser::new(Some(FIELD), Some(Arc::clone(&a)))?;
    Assert::assert_true(is_a_hit(parser.parse("mö*tley")?, "moatley", &a)?);
    // need to have at least one genuine wildcard to trigger the wildcard analysis
    // hence the * before the y
    Assert::assert_true(is_a_hit(parser.parse("mö\\*tl*y")?, "mo*tley", &a)?);
    // escaped backslash then true wildcard
    Assert::assert_true(is_a_hit(parser.parse("mö\\\\*tley")?, "mo\\atley", &a)?);
    // escaped wildcard then true wildcard
    Assert::assert_true(is_a_hit(parser.parse("mö\\??ley")?, "mo?tley", &a)?);

    // the first is an escaped * which should yield a miss
    Assert::assert_false(is_a_hit(parser.parse("mö\\*tl*y")?, "moatley", &a)?);
    Ok(())
}

fn test_wildcard_does_not_normalize_escaped_chars<const EXT: bool>(
    _t: &mut TestQueryParserFixture<EXT>,
) -> Result<()> {
    let ascii = ascii_analyzer();
    let keyword_analyzer = mock_analyzer_default();
    let mut parser = QueryParser::new(Some(FIELD), Some(Arc::clone(&ascii)))?;

    Assert::assert_true(is_a_hit(parser.parse("e*e")?, "étude", &ascii)?);
    Assert::assert_true(is_a_hit(parser.parse("é*e")?, "etude", &ascii)?);
    Assert::assert_false(is_a_hit(parser.parse("\\é*e")?, "etude", &ascii)?);
    Assert::assert_true(is_a_hit(
        parser.parse("\\é*e")?,
        "étude",
        &keyword_analyzer,
    )?);
    Ok(())
}

fn test_wild_card_query<const EXT: bool>(_t: &mut TestQueryParserFixture<EXT>) -> Result<()> {
    let mut parser = QueryParser::new(Some(FIELD), Some(ascii_analyzer()))?;
    parser.set_allow_leading_wildcard(true)?;
    assert_str_eq(
        "*bersetzung uber*ung",
        &parser.parse("*bersetzung über*ung")?.to_query_string(FIELD),
    );
    parser.set_allow_leading_wildcard(false)?;
    assert_str_eq(
        "motley crue motl?* cru?",
        &parser
            .parse("Mötley Cr\u{00fc}e Mötl?* Crü?")?
            .to_query_string(FIELD),
    );
    assert_str_eq(
        "renee zellweger ren?? zellw?ger",
        &parser
            .parse("Renée Zellweger Ren?? Zellw?ger")?
            .to_query_string(FIELD),
    );
    Ok(())
}

fn test_prefix_query<const EXT: bool>(_t: &mut TestQueryParserFixture<EXT>) -> Result<()> {
    let mut parser = QueryParser::new(Some(FIELD), Some(ascii_analyzer()))?;
    assert_str_eq(
        "ubersetzung ubersetz*",
        &parser
            .parse("übersetzung übersetz*")?
            .to_query_string(FIELD),
    );
    assert_str_eq(
        "motley crue motl* cru*",
        &parser
            .parse("Mötley Crüe Mötl* crü*")?
            .to_query_string(FIELD),
    );
    assert_str_eq(
        "rene? zellw*",
        &parser.parse("René? Zellw*")?.to_query_string(FIELD),
    );
    Ok(())
}

fn test_range_query<const EXT: bool>(_t: &mut TestQueryParserFixture<EXT>) -> Result<()> {
    let mut parser = QueryParser::new(Some(FIELD), Some(ascii_analyzer()))?;
    assert_str_eq(
        "[aa TO bb]",
        &parser.parse("[aa TO bb]")?.to_query_string(FIELD),
    );
    assert_str_eq(
        "{anais TO zoe}",
        &parser.parse("{Anaïs TO Zoé}")?.to_query_string(FIELD),
    );
    Ok(())
}

fn test_fuzzy_query<const EXT: bool>(_t: &mut TestQueryParserFixture<EXT>) -> Result<()> {
    let mut parser = QueryParser::new(Some(FIELD), Some(ascii_analyzer()))?;
    assert_str_eq(
        "ubersetzung ubersetzung~1",
        &parser
            .parse("Übersetzung Übersetzung~0.9")?
            .to_query_string(FIELD),
    );
    assert_str_eq(
        "motley crue motley~1 crue~2",
        &parser
            .parse("Mötley Crüe Mötley~0.75 Crüe~0.5")?
            .to_query_string(FIELD),
    );
    assert_str_eq(
        "renee zellweger renee~0 zellweger~2",
        &parser
            .parse("Renée Zellweger Renée~0.9 Zellweger~")?
            .to_query_string(FIELD),
    );
    Ok(())
}

/// The logic of `TestQueryParser.FoldingFilter`.
#[derive(Debug)]
struct FoldingFilterLogic {
    term_att: Att,
}

impl TokenFilterLogic for FoldingFilterLogic {
    fn increment_token(
        &mut self,
        _source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if input.increment_token()? {
            // `char[] term = termAtt.buffer()`: the whole buffer, not only the
            // term's length.
            let mut t = self.term_att.char_term_mut();
            for c in t.buffer_mut().iter_mut() {
                match *c {
                    'ü' => *c = 'u',
                    'ö' => *c = 'o',
                    'é' => *c = 'e',
                    'ï' => *c = 'i',
                    _ => {}
                }
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

fn folding_filter(mut input: Box<dyn TokenStream>) -> Result<TokenFilter<FoldingFilterLogic>> {
    let term_att = Att::add(input.attribute_source_mut(), char_term_id())?;
    Ok(TokenFilter::new(input, FoldingFilterLogic { term_att }))
}

/// `new ASCIIAnalyzer()`.
fn ascii_analyzer() -> Arc<dyn Analyzer> {
    FnAnalyzer::with_normalize(
        "org.apache.lucene.queryparser.classic.TestQueryParser$ASCIIAnalyzer",
        |_| {
            let result = mock_tokenizer(&WHITESPACE, true)?;
            let filter = folding_filter(shared(&result))?;
            Ok(components(result, sink(filter)))
        },
        |_, input| {
            Ok(
                Box::new(folding_filter(Box::new(MockLowerCaseFilter::new(input)?))?)
                    as Box<dyn TokenStream>,
            )
        },
    )
    .arc()
}

// LUCENE-4176
fn test_byte_terms<const EXT: bool>(_t: &mut TestQueryParserFixture<EXT>) -> Result<()> {
    let s = "เข";
    let analyzer: Arc<dyn Analyzer> = Arc::new(MockBytesAnalyzer::new());
    let mut qp = QueryParser::new(Some(FIELD), Some(Arc::clone(&analyzer)))?;

    Assert::assert_true(is_a_hit(qp.parse("[เข TO เข]")?, s, &analyzer)?);
    Assert::assert_true(is_a_hit(qp.parse("เข~1")?, s, &analyzer)?);
    Assert::assert_true(is_a_hit(qp.parse("เข*")?, s, &analyzer)?);
    Assert::assert_true(is_a_hit(qp.parse("เ*")?, s, &analyzer)?);
    Assert::assert_true(is_a_hit(qp.parse("เ??")?, s, &analyzer)?);
    Ok(())
}

// LUCENE-7533
fn test_split_on_whitespace_with_auto_generate_phrase_queries<const EXT: bool>(
    _t: &mut TestQueryParserFixture<EXT>,
) -> Result<()> {
    let mut qp = QueryParser::new(Some(FIELD), Some(mock_analyzer_default()))?;
    expect_throws("java.lang.IllegalArgumentException", || {
        qp.set_split_on_whitespace(false)?;
        qp.set_auto_generate_phrase_queries(true)
    });
    let mut qp2 = QueryParser::new(Some(FIELD), Some(mock_analyzer_default()))?;
    expect_throws("java.lang.IllegalArgumentException", || {
        qp2.set_split_on_whitespace(true)?;
        qp2.set_auto_generate_phrase_queries(true)?;
        qp2.set_split_on_whitespace(false)
    });
    Ok(())
}

/// `isAHit(Query q, String content, Analyzer analyzer)`.
fn is_a_hit(q: Arc<dyn Query>, content: &str, analyzer: &Arc<dyn Analyzer>) -> Result<bool> {
    let ram_dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new(&*random(), ram_dir.as_directory(), Arc::clone(analyzer))?;
    let mut doc = Document::new();
    let field_type = FieldType::new();
    field_type.set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)?;
    field_type.set_tokenized(true)?;
    field_type.set_stored(true)?;
    let field = Field::new(FIELD, content.to_string(), field_type)?;
    doc.add(Box::new(field));
    writer.add_document(&doc)?;
    writer.close()?;
    let ir = directory_reader::open(ram_dir.as_directory())?;
    let is = IndexSearcher::new(Arc::clone(&ir) as Arc<dyn IndexReader>)?;

    let hits = is.count(q)?;
    ir.close()?;
    ram_dir.close()?;
    Ok(hits == 1)
}
