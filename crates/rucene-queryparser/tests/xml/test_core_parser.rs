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

//! Port of `TestCoreParser.java`, `TestCorePlusQueriesParser.java` and
//! `TestCorePlusExtensionsParser.java`.
//!
//! The two subclasses override only `newCoreParser(String, Analyzer)` and
//! add one test method each: `LEVEL` selects the class (`0`
//! `TestCoreParser`, `1` `TestCorePlusQueriesParser`, `2`
//! `TestCorePlusExtensionsParser`). The static fields `analyzer`,
//! `coreParser` and `indexData` are `TestCoreParser`'s, shared by the
//! subclasses as in Java.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::{AfterClass, Assert};
use rucene::analysis::Analyzer;
use rucene::error::Result;
use rucene::index::IndexReader;
use rucene::search::boost_query::BoostQuery;
use rucene::search::disjunction_max_query::DisjunctionMaxQuery;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::Query;
use rucene_queries::spans::span_near_query::SpanNearQuery;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_queries::spans::span_term_query::SpanTermQuery;
use rucene_queryparser::xml::core_parser::{CoreParser, CoreParserOps};
use rucene_queryparser::xml::core_plus_extensions_parser::CorePlusExtensionsParser;
use rucene_queryparser::xml::core_plus_queries_parser::CorePlusQueriesParser;
use rucene_test_framework::analysis::mock_token_filter::ENGLISH_STOPSET;
use rucene_test_framework::analysis::mock_tokenizer::WHITESPACE;
use rucene_test_framework::util::lucene_test_case::VERBOSE;
use rucene_test_framework::util::LuceneTestCase;

use super::core_parser_test_index_data::{resource, CoreParserTestIndexData};
use crate::qp_support::core_support::{class, test};
use crate::qp_support::query_parser_test_base::mock_analyzer_filter;
use crate::qp_support::search_support::{static_hook, term, tq};
use crate::qp_support::{
    assert_query_eq, catch_java, expect_throws, jdk_default_hooks, to_s, PARSER_EXCEPTION,
};

/// `private static final String defaultField = "contents"`.
const DEFAULT_FIELD: &str = "contents";

/// `private static Analyzer analyzer`.
static ANALYZER: Mutex<Option<Arc<dyn Analyzer>>> = Mutex::new(None);
/// `private static CoreParser coreParser`.
static CORE_PARSER: Mutex<Option<Arc<dyn CoreParserOps>>> = Mutex::new(None);
/// `private static CoreParserTestIndexData indexData`.
static INDEX_DATA: Mutex<Option<Arc<CoreParserTestIndexData>>> = Mutex::new(None);

/// The Java class of `LEVEL`.
fn class_name<const LEVEL: u8>() -> &'static str {
    match LEVEL {
        0 => "org.apache.lucene.queryparser.xml.TestCoreParser",
        1 => "org.apache.lucene.queryparser.xml.TestCorePlusQueriesParser",
        _ => "org.apache.lucene.queryparser.xml.TestCorePlusExtensionsParser",
    }
}

/// The test class of `LEVEL`, with the methods it declares and inherits.
pub fn class_def<const LEVEL: u8>() -> Arc<Class> {
    let mut methods: Vec<Method> = jdk_default_hooks();
    methods.push(static_hook("afterClass", AfterClass, after_class));
    let tests: Vec<(&str, crate::qp_support::core_support::TestFn)> = vec![
        ("testTermQueryXML", test_term_query_xml::<LEVEL>),
        (
            "test_DOCTYPE_TermQueryXML",
            test_doctype_term_query_xml::<LEVEL>,
        ),
        (
            "test_ENTITY_TermQueryXML",
            test_entity_term_query_xml::<LEVEL>,
        ),
        ("testTermQueryEmptyXML", test_term_query_empty_xml::<LEVEL>),
        ("testTermsQueryXML", test_terms_query_xml::<LEVEL>),
        ("testBooleanQueryXML", test_boolean_query_xml::<LEVEL>),
        (
            "testDisjunctionMaxQueryXML",
            test_disjunction_max_query_xml::<LEVEL>,
        ),
        ("testRangeQueryXML", test_range_query_xml::<LEVEL>),
        ("testUserQueryXML", test_user_query_xml::<LEVEL>),
        (
            "testCustomFieldUserQueryXML",
            test_custom_field_user_query_xml::<LEVEL>,
        ),
        (
            "testBoostingTermQueryXML",
            test_boosting_term_query_xml::<LEVEL>,
        ),
        ("testSpanTermXML", test_span_term_xml::<LEVEL>),
        (
            "testSpanPositionRangeQueryXML",
            test_span_position_range_query_xml::<LEVEL>,
        ),
        (
            "testSpanNearQueryWithoutSlopXML",
            test_span_near_query_without_slop_xml::<LEVEL>,
        ),
        (
            "testConstantScoreQueryXML",
            test_constant_score_query_xml::<LEVEL>,
        ),
        (
            "testMatchAllDocsPlusFilterXML",
            test_match_all_docs_plus_filter_xml::<LEVEL>,
        ),
        ("testNestedBooleanQuery", test_nested_boolean_query::<LEVEL>),
        ("testPointRangeQuery", test_point_range_query::<LEVEL>),
        (
            "testPointRangeQueryWithoutLowerTerm",
            test_point_range_query_without_lower_term::<LEVEL>,
        ),
        (
            "testPointRangeQueryWithoutUpperTerm",
            test_point_range_query_without_upper_term::<LEVEL>,
        ),
        (
            "testPointRangeQueryWithoutRange",
            test_point_range_query_without_range::<LEVEL>,
        ),
        ("testSpanBoosts", test_span_boosts::<LEVEL>),
    ];
    for (name, body) in tests {
        methods.push(test(name, body));
    }
    if LEVEL >= 1 {
        methods.push(test(
            "testLikeThisQueryXML",
            test_like_this_query_xml::<LEVEL>,
        ));
    }
    if LEVEL >= 2 {
        methods.push(test(
            "testFuzzyLikeThisQueryXML",
            test_fuzzy_like_this_query_xml::<LEVEL>,
        ));
    }
    class(class_name::<LEVEL>(), methods)
}

/// `newAnalyzer()`.
fn new_analyzer() -> Arc<dyn Analyzer> {
    // TODO: rewrite test (this needs to set QueryParser.enablePositionIncrements, too, for work
    // with CURRENT):
    mock_analyzer_filter(&WHITESPACE, true, &ENGLISH_STOPSET)
}

/// `newCoreParser(String defaultField, Analyzer analyzer)`, as `LEVEL`'s
/// class overrides it.
fn new_core_parser<const LEVEL: u8>(
    default_field: &str,
    analyzer: Arc<dyn Analyzer>,
) -> Arc<dyn CoreParserOps> {
    let field = Some(default_field.to_string());
    match LEVEL {
        0 => Arc::new(CoreParser::new(field, Some(analyzer))),
        1 => Arc::new(CorePlusQueriesParser::new(field, Some(analyzer))),
        _ => Arc::new(CorePlusExtensionsParser::new(field, Some(analyzer))),
    }
}

/// `@AfterClass afterClass()`.
fn after_class() -> Result<()> {
    let index_data = INDEX_DATA.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(index_data) = index_data {
        index_data.close()?;
    }
    *CORE_PARSER.lock().unwrap_or_else(|e| e.into_inner()) = None;
    *ANALYZER.lock().unwrap_or_else(|e| e.into_inner()) = None;
    Ok(())
}

// ================= Helper methods ===================================

/// `analyzer()`.
fn analyzer() -> Arc<dyn Analyzer> {
    let mut g = ANALYZER.lock().unwrap_or_else(|e| e.into_inner());
    Arc::clone(g.get_or_insert_with(new_analyzer))
}

/// `coreParser()`.
fn core_parser<const LEVEL: u8>() -> Arc<dyn CoreParserOps> {
    let existing = CORE_PARSER
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    if let Some(p) = existing {
        return p;
    }
    let p = new_core_parser::<LEVEL>(DEFAULT_FIELD, analyzer());
    *CORE_PARSER.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::clone(&p));
    p
}

/// `indexData()`: an exception fails the test with its `toString()`.
fn index_data() -> Arc<CoreParserTestIndexData> {
    let existing = INDEX_DATA.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if let Some(d) = existing {
        return d;
    }
    let d = match catch_java(|| CoreParserTestIndexData::new(analyzer())) {
        Ok(d) => Arc::new(d),
        Err(e) => Assert::fail(&format!("caught Exception {}", throwable_to_string(&e))),
    };
    *INDEX_DATA.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::clone(&d));
    d
}

/// `Throwable.toString()`.
fn throwable_to_string(t: &randomizedtesting::jdk::exception::Throwable) -> String {
    match t.message() {
        Some(m) => format!("{}: {m}", t.class_name()),
        None => t.class_name().to_string(),
    }
}

/// `reader()`.
#[allow(dead_code)]
fn reader() -> Arc<dyn IndexReader> {
    Arc::clone(&index_data().reader) as Arc<dyn IndexReader>
}

/// `searcher()`.
fn searcher() -> Arc<IndexSearcher> {
    Arc::clone(&index_data().searcher)
}

/// `parseShouldFail(String xmlFileName, String expectedParserExceptionMessage)`.
fn parse_should_fail<const LEVEL: u8>(xml_file_name: &str, expected: &str) -> Result<()> {
    let mut q = None;
    let mut pe = None;
    match catch_java(|| parse::<LEVEL>(xml_file_name)) {
        Ok(query) => q = Some(query),
        Err(e) if e.is_instance(PARSER_EXCEPTION) => pe = Some(e),
        Err(other) => other.throw(),
    }
    Assert::assert_true_msg(
        &format!(
            "for {xml_file_name} unexpectedly got {}",
            q.as_ref()
                .map_or_else(|| "null".to_string(), |q| to_s(&**q))
        ),
        q.is_none(),
    );
    Assert::assert_true_msg(
        &format!("expected a ParserException for {xml_file_name}"),
        pe.is_some(),
    );
    Assert::assert_equals_str(
        Some(&format!(
            "expected different ParserException for {xml_file_name}"
        )),
        Some(expected),
        pe.as_ref().and_then(|e| e.message()),
    );
    Ok(())
}

/// `parse(String xmlFileName)`.
fn parse<const LEVEL: u8>(xml_file_name: &str) -> Result<Arc<dyn Query>> {
    let bytes = impl_resource(xml_file_name);
    core_parser::<LEVEL>().parse(&mut &bytes[..])
}

/// `parseAsSpan(String xmlFileName)`.
fn parse_as_span<const LEVEL: u8>(xml_file_name: &str) -> Result<Arc<dyn SpanQuery>> {
    let bytes = impl_resource(xml_file_name);
    core_parser::<LEVEL>().parse_as_span_query(&mut &bytes[..])
}

/// The stream `implParse` opens: `assertNotNull("Test XML file " + name +
/// " cannot be found", xmlStream)`.
fn impl_resource(xml_file_name: &str) -> Vec<u8> {
    let bytes = resource(xml_file_name);
    Assert::assert_true_msg(
        &format!("Test XML file {xml_file_name} cannot be found"),
        bytes.is_some(),
    );
    bytes.unwrap_or_default()
}

/// `rewrite(Query q)`: `q.rewrite(searcher())`.
fn rewrite(q: &Arc<dyn Query>) -> Result<Arc<dyn Query>> {
    Ok(q.rewrite(&searcher())?.unwrap_or_else(|| Arc::clone(q)))
}

/// `q.getClass().getCanonicalName()`.
fn canonical_name(q: &dyn Query) -> String {
    q.java_class_name().replace('$', ".")
}

/// `dumpResults(String qType, Query q, int numDocs)`.
fn dump_results(q_type: &str, q: Arc<dyn Query>, num_docs: i32) -> Result<()> {
    if *VERBOSE {
        println!(
            "TEST: qType={q_type} numDocs={num_docs} {} query={}",
            canonical_name(&*q),
            to_s(&*q)
        );
    }
    let searcher = searcher();
    let hits = searcher.search(Arc::clone(&q), num_docs)?;
    let produced_results = hits.total_hits.value() > 0;
    if !produced_results {
        println!(
            "TEST: qType={q_type} numDocs={num_docs} {} query={}",
            canonical_name(&*q),
            to_s(&*q)
        );
    }
    if *VERBOSE {
        let score_docs = &hits.score_docs;
        let stored_fields = searcher.stored_fields()?;
        let n = i64::from(num_docs).min(hits.total_hits.value());
        for i in 0..n as usize {
            let ldoc = stored_fields.document(score_docs[i].doc)?;
            println!(
                "[{}]{}",
                ldoc.get("date").unwrap_or_else(|| "null".into()),
                ldoc.get("contents").unwrap_or_else(|| "null".into())
            );
        }
        println!();
    }
    Assert::assert_true_msg(&format!("{q_type} produced no results"), produced_results);
    Ok(())
}

fn test_term_query_xml<const LEVEL: u8>() -> Result<()> {
    let q = parse::<LEVEL>("TermQuery.xml")?;
    dump_results("TermQuery", q, 5)
}

fn test_doctype_term_query_xml<const LEVEL: u8>() -> Result<()> {
    let saxe =
        LuceneTestCase::expect_throws_wrapped(PARSER_EXCEPTION, "org.xml.sax.SAXException", || {
            crate::qp_support::register_queryparser_exceptions();
            parse::<LEVEL>("DOCTYPE_TermQuery.xml")
                .map(|_| ())
                .map_err(|e| rucene_test_framework::util::java_exceptions::java_throwable(&e))
        });
    Assert::assert_true(
        saxe.message()
            .unwrap_or_default()
            .starts_with("External Entity resolving unsupported:"),
    );
    Ok(())
}

fn test_entity_term_query_xml<const LEVEL: u8>() -> Result<()> {
    let saxe =
        LuceneTestCase::expect_throws_wrapped(PARSER_EXCEPTION, "org.xml.sax.SAXException", || {
            crate::qp_support::register_queryparser_exceptions();
            parse::<LEVEL>("ENTITY_TermQuery.xml")
                .map(|_| ())
                .map_err(|e| rucene_test_framework::util::java_exceptions::java_throwable(&e))
        });
    Assert::assert_true(
        saxe.message()
            .unwrap_or_default()
            .starts_with("External Entity resolving unsupported:"),
    );
    Ok(())
}

fn test_term_query_empty_xml<const LEVEL: u8>() -> Result<()> {
    parse_should_fail::<LEVEL>("TermQueryEmpty.xml", "TermQuery has no text")
}

fn test_terms_query_xml<const LEVEL: u8>() -> Result<()> {
    let q = parse::<LEVEL>("TermsQuery.xml")?;
    dump_results("TermsQuery", q, 5)
}

fn test_boolean_query_xml<const LEVEL: u8>() -> Result<()> {
    let q = parse::<LEVEL>("BooleanQuery.xml")?;
    dump_results("BooleanQuery", q, 5)
}

fn test_disjunction_max_query_xml<const LEVEL: u8>() -> Result<()> {
    let q = parse::<LEVEL>("DisjunctionMaxQuery.xml")?;
    let inner: Arc<dyn Query> = Arc::new(DisjunctionMaxQuery::new(vec![tq("b", "verger")], 0.3)?);
    let expected = DisjunctionMaxQuery::new(vec![tq("a", "merger"), inner], 0.0)?;
    assert_query_eq(&expected, &*q);
    Ok(())
}

fn test_range_query_xml<const LEVEL: u8>() -> Result<()> {
    let q = parse::<LEVEL>("RangeQuery.xml")?;
    dump_results("RangeQuery", q, 5)
}

fn test_user_query_xml<const LEVEL: u8>() -> Result<()> {
    let q = parse::<LEVEL>("UserInputQuery.xml")?;
    dump_results("UserInput with Filter", q, 5)
}

fn test_custom_field_user_query_xml<const LEVEL: u8>() -> Result<()> {
    let q = parse::<LEVEL>("UserInputQueryCustomField.xml")?;
    let h = searcher().search(q, 1000)?.total_hits.value();
    Assert::assert_equals_msg(
        Some("UserInputQueryCustomField should produce 0 result "),
        0,
        h,
    );
    Ok(())
}

fn test_boosting_term_query_xml<const LEVEL: u8>() -> Result<()> {
    let q = parse::<LEVEL>("BoostingTermQuery.xml")?;
    dump_results("BoostingTermQuery", q, 5)
}

fn test_span_term_xml<const LEVEL: u8>() -> Result<()> {
    let q = parse::<LEVEL>("SpanQuery.xml")?;
    dump_results("Span Query", Arc::clone(&q), 5)?;
    let sq = parse_as_span::<LEVEL>("SpanQuery.xml")?;
    let sq_q: Arc<dyn Query> = sq;
    dump_results("Span Query", Arc::clone(&sq_q), 5)?;
    assert_query_eq(&*q, &*sq_q);
    Ok(())
}

fn test_span_position_range_query_xml<const LEVEL: u8>() -> Result<()> {
    let q = parse::<LEVEL>("SpanPositionRangeQuery.xml")?;
    let h = searcher().search(Arc::clone(&q), 10)?.total_hits.value();
    Assert::assert_equals_msg(
        Some("SpanPositionRangeQuery should produce 2 result "),
        2,
        h,
    );
    let sq = parse_as_span::<LEVEL>("SpanPositionRangeQuery.xml")?;
    let sq_q: Arc<dyn Query> = sq;
    dump_results("SpanPositionRangeQuery", Arc::clone(&sq_q), 5)?;
    assert_query_eq(&*q, &*sq_q);
    Ok(())
}

fn test_span_near_query_without_slop_xml<const LEVEL: u8>() -> Result<()> {
    // expected NumberFormatException from empty "slop" string
    expect_throws("java.lang.NumberFormatException", || {
        parse::<LEVEL>("SpanNearQueryWithoutSlop.xml")
    });
    expect_throws("java.lang.NumberFormatException", || {
        parse_as_span::<LEVEL>("SpanNearQueryWithoutSlop.xml")
    });
    Ok(())
}

fn test_constant_score_query_xml<const LEVEL: u8>() -> Result<()> {
    let q = parse::<LEVEL>("ConstantScoreQuery.xml")?;
    dump_results("ConstantScoreQuery", q, 5)
}

fn test_match_all_docs_plus_filter_xml<const LEVEL: u8>() -> Result<()> {
    let q = parse::<LEVEL>("MatchAllDocsQuery.xml")?;
    dump_results("MatchAllDocsQuery with range filter", q, 5)
}

fn test_nested_boolean_query<const LEVEL: u8>() -> Result<()> {
    let q = parse::<LEVEL>("NestedBooleanQuery.xml")?;
    dump_results("Nested Boolean query", q, 5)
}

fn test_point_range_query<const LEVEL: u8>() -> Result<()> {
    let q = parse::<LEVEL>("PointRangeQuery.xml")?;
    dump_results("PointRangeQuery", q, 5)
}

fn test_point_range_query_without_lower_term<const LEVEL: u8>() -> Result<()> {
    let q = parse::<LEVEL>("PointRangeQueryWithoutLowerTerm.xml")?;
    dump_results("PointRangeQueryWithoutLowerTerm", q, 5)
}

fn test_point_range_query_without_upper_term<const LEVEL: u8>() -> Result<()> {
    let q = parse::<LEVEL>("PointRangeQueryWithoutUpperTerm.xml")?;
    dump_results("PointRangeQueryWithoutUpperTerm", q, 5)
}

fn test_point_range_query_without_range<const LEVEL: u8>() -> Result<()> {
    let q = parse::<LEVEL>("PointRangeQueryWithoutRange.xml")?;
    dump_results("PointRangeQueryWithoutRange", q, 5)
}

fn test_span_boosts<const LEVEL: u8>() -> Result<()> {
    let top_level = "<SpanTerm fieldName=\"field\" boost=\"2\">value</SpanTerm>";
    {
        let actual = core_parser::<LEVEL>().parse(&mut top_level.as_bytes())?;
        let expected: Arc<dyn Query> = Arc::new(SpanTermQuery::new(term("field", "value")));
        let expected = BoostQuery::new(expected, 2.0)?;
        assert_query_eq(&expected, &*actual);
    }

    let nested = concat!(
        "<SpanNear fieldName=\"field\" boost=\"2\" slop=\"8\" inOrder=\"false\">",
        // top level boost is preserved
        " <SpanTerm boost=\"4\">value1</SpanTerm>",
        // interior boost is ignored
        " <SpanTerm>value2</SpanTerm>",
        "</SpanNear>"
    );
    {
        let actual = core_parser::<LEVEL>().parse(&mut nested.as_bytes())?;
        let clauses: Vec<Arc<dyn SpanQuery>> = vec![
            Arc::new(SpanTermQuery::new(term("field", "value1"))),
            Arc::new(SpanTermQuery::new(term("field", "value2"))),
        ];
        let expected: Arc<dyn Query> = Arc::new(SpanNearQuery::new(clauses, 8, false)?);
        let expected = BoostQuery::new(expected, 2.0)?;
        assert_query_eq(&expected, &*actual);
    }
    Ok(())
}

/// `TestCorePlusQueriesParser.testLikeThisQueryXML()`.
fn test_like_this_query_xml<const LEVEL: u8>() -> Result<()> {
    let q = parse::<LEVEL>("LikeThisQuery.xml")?;
    dump_results("like this", q, 5)
}

/// `TestCorePlusExtensionsParser.testFuzzyLikeThisQueryXML()`.
fn test_fuzzy_like_this_query_xml<const LEVEL: u8>() -> Result<()> {
    let q = parse::<LEVEL>("FuzzyLikeThisQuery.xml")?;
    // show rewritten fuzzyLikeThisQuery - see what is being matched on
    if *VERBOSE {
        println!("{}", to_s(&*rewrite(&q)?));
    }
    dump_results("FuzzyLikeThis", q, 5)
}
