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

//! Port of `TestMultiFieldQPHelper.java`: tests multi field query parsing
//! using the `StandardQueryParser`.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::{
    Analyzer, AnalyzerState, CharReader, PerFieldReuseStrategy, StringCharReader,
    TokenStreamComponents,
};
use rucene::document::{Document, Store};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::search::boolean_clause::Occur;
use rucene::search::match_no_docs_query::MatchNoDocsQuery;
use rucene::search::Query;
use rucene_queryparser::flexible::standard::config::standard_query_config_handler::{
    MultiFields, Operator,
};
use rucene_queryparser::flexible::standard::query_parser_util::QueryParserUtil;
use rucene_queryparser::flexible::standard::standard_query_parser::StandardQueryParser;
use rucene_queryparser::jdk::char_sequence::CharSequence;
use rucene_test_framework::analysis::mock_analyzer::MockAnalyzer;
use rucene_test_framework::util::LuceneTestCase;

use super::test_qp_helper::qp_test_analyzer;
use crate::qp_support::core_support::{class, random, test};
use crate::qp_support::query_parser_test_base::mock_analyzer_default;
use crate::qp_support::{assert_str_eq, expect_throws, is, jdk_default_hooks, to_s};

/// `TestMultiFieldQPHelper`.
pub fn class_def() -> Arc<Class> {
    let mut methods = jdk_default_hooks();
    methods.extend([
        test("testStopwordsParsing", test_stopwords_parsing),
        test("testSimple", test_simple),
        test("testBoostsSimple", test_boosts_simple),
        test("testStaticMethod1", test_static_method1),
        test("testStaticMethod2", test_static_method2),
        test("testStaticMethod2Old", test_static_method2_old),
        test("testStaticMethod3", test_static_method3),
        test("testStaticMethod3Old", test_static_method3_old),
        test("testAnalyzerReturningNull", test_analyzer_returning_null),
        test("testStopWordSearching", test_stop_word_searching),
    ]);
    class(
        "org.apache.lucene.queryparser.flexible.standard.TestMultiFieldQPHelper",
        methods,
    )
}

/// `String[] fields = {...}` as the `CharSequence[]` `setMultiFields`
/// holds.
fn multi_fields(fields: &[&str]) -> MultiFields {
    Arc::new(RwLock::new(
        fields
            .iter()
            .map(|f| Some(CharSequence::from(*f)))
            .collect(),
    ))
}

/// `String[] fields` as the static `parse` methods take them.
fn field_refs<'a>(fields: &[&'a str]) -> Vec<Option<&'a str>> {
    fields.iter().map(|f| Some(*f)).collect()
}

/// `mfqp.parse(query, null)`.
fn parse_null(mfqp: &StandardQueryParser, query: &str) -> Result<Arc<dyn Query>> {
    mfqp.parse(query, None)?
        .ok_or_else(LuceneError::null_pointer)
}

/// test stop words parsing for both the non static form, and for the
/// corresponding static form (qtxt, fields[]).
fn test_stopwords_parsing() -> Result<()> {
    assert_stop_query_equals("one", "b:one t:one")?;
    assert_stop_query_equals("one stop", "b:one t:one")?;
    assert_stop_query_equals("one (stop)", "b:one t:one")?;
    assert_stop_query_equals("one ((stop))", "b:one t:one")?;
    assert_stop_query_is_match_no_docs_query("stop")?;
    assert_stop_query_is_match_no_docs_query("(stop)")?;
    assert_stop_query_is_match_no_docs_query("((stop))")?;
    Ok(())
}

// verify parsing of query using a stopping analyzer
fn assert_stop_query_is_match_no_docs_query(qtxt: &str) -> Result<()> {
    let fields = ["b", "t"];
    let a = qp_test_analyzer();
    let mut mfqp = StandardQueryParser::new();
    mfqp.set_multi_fields(Some(multi_fields(&fields)))?;
    mfqp.set_analyzer(Some(a))?;

    let q = parse_null(&mfqp, qtxt)?;
    Assert::assert_true(is::<MatchNoDocsQuery>(&*q));
    Ok(())
}

// verify parsing of query using a stopping analyzer
fn assert_stop_query_equals(qtxt: &str, expected_res: &str) -> Result<()> {
    let fields = ["b", "t"];
    let occur = [Occur::SHOULD, Occur::SHOULD];
    let a = qp_test_analyzer();
    let mut mfqp = StandardQueryParser::new();
    mfqp.set_multi_fields(Some(multi_fields(&fields)))?;
    mfqp.set_analyzer(Some(Arc::clone(&a)))?;

    let q = parse_null(&mfqp, qtxt)?;
    assert_str_eq(expected_res, to_s(&*q).trim());

    let q = QueryParserUtil::parse_with_flags(qtxt, &field_refs(&fields), &occur, Some(a))?;
    assert_str_eq(expected_res, to_s(&*q).trim());
    Ok(())
}

fn test_simple() -> Result<()> {
    let fields = ["b", "t"];
    let mut mfqp = StandardQueryParser::new();
    mfqp.set_multi_fields(Some(multi_fields(&fields)))?;
    mfqp.set_analyzer(Some(mock_analyzer_default()))?;

    let q = parse_null(&mfqp, "one")?;
    assert_str_eq("b:one t:one", &to_s(&*q));

    let q = parse_null(&mfqp, "one two")?;
    assert_str_eq("(b:one t:one) (b:two t:two)", &to_s(&*q));

    let q = parse_null(&mfqp, "+one +two")?;
    assert_str_eq("+(b:one t:one) +(b:two t:two)", &to_s(&*q));

    let q = parse_null(&mfqp, "+one -two -three")?;
    assert_str_eq(
        "+(b:one t:one) -(b:two t:two) -(b:three t:three)",
        &to_s(&*q),
    );

    let q = parse_null(&mfqp, "one^2 two")?;
    assert_str_eq("(b:one t:one)^2.0 (b:two t:two)", &to_s(&*q));

    let q = parse_null(&mfqp, "one~ two")?;
    assert_str_eq("(b:one~2 t:one~2) (b:two t:two)", &to_s(&*q));

    let q = parse_null(&mfqp, "one~0.8 two^2")?;
    assert_str_eq("(b:one~0 t:one~0) (b:two t:two)^2.0", &to_s(&*q));

    let q = parse_null(&mfqp, "one* two*")?;
    assert_str_eq("(b:one* t:one*) (b:two* t:two*)", &to_s(&*q));

    let q = parse_null(&mfqp, "[a TO c] two")?;
    assert_str_eq("(b:[a TO c] t:[a TO c]) (b:two t:two)", &to_s(&*q));

    let q = parse_null(&mfqp, "w?ldcard")?;
    assert_str_eq("b:w?ldcard t:w?ldcard", &to_s(&*q));

    let q = parse_null(&mfqp, "\"foo bar\"")?;
    assert_str_eq("b:\"foo bar\" t:\"foo bar\"", &to_s(&*q));

    let q = parse_null(&mfqp, "\"aa bb cc\" \"dd ee\"")?;
    assert_str_eq(
        "(b:\"aa bb cc\" t:\"aa bb cc\") (b:\"dd ee\" t:\"dd ee\")",
        &to_s(&*q),
    );

    let q = parse_null(&mfqp, "\"foo bar\"~4")?;
    assert_str_eq("b:\"foo bar\"~4 t:\"foo bar\"~4", &to_s(&*q));

    // LUCENE-1213: QueryParser was ignoring slop when phrase
    // had a field.
    let q = parse_null(&mfqp, "b:\"foo bar\"~4")?;
    assert_str_eq("b:\"foo bar\"~4", &to_s(&*q));

    // make sure that terms which have a field are not touched:
    let q = parse_null(&mfqp, "one f:two")?;
    assert_str_eq("(b:one t:one) f:two", &to_s(&*q));

    // AND mode:
    mfqp.set_default_operator(Some(Operator::And))?;
    let q = parse_null(&mfqp, "one two")?;
    assert_str_eq("+(b:one t:one) +(b:two t:two)", &to_s(&*q));
    let q = parse_null(&mfqp, "\"aa bb cc\" \"dd ee\"")?;
    assert_str_eq(
        "+(b:\"aa bb cc\" t:\"aa bb cc\") +(b:\"dd ee\" t:\"dd ee\")",
        &to_s(&*q),
    );
    Ok(())
}

fn test_boosts_simple() -> Result<()> {
    let mut boosts = HashMap::new();
    boosts.insert("b".to_string(), 5f32);
    boosts.insert("t".to_string(), 10f32);
    let fields = ["b", "t"];
    let mut mfqp = StandardQueryParser::new();
    mfqp.set_multi_fields(Some(multi_fields(&fields)))?;
    mfqp.set_fields_boost(Some(Arc::new(RwLock::new(boosts))))?;
    mfqp.set_analyzer(Some(mock_analyzer_default()))?;

    // Check for simple
    let q = parse_null(&mfqp, "one")?;
    assert_str_eq("(b:one)^5.0 (t:one)^10.0", &to_s(&*q));

    // Check for AND
    let q = parse_null(&mfqp, "one AND two")?;
    assert_str_eq(
        "+((b:one)^5.0 (t:one)^10.0) +((b:two)^5.0 (t:two)^10.0)",
        &to_s(&*q),
    );

    // Check for OR
    let q = parse_null(&mfqp, "one OR two")?;
    assert_str_eq(
        "((b:one)^5.0 (t:one)^10.0) ((b:two)^5.0 (t:two)^10.0)",
        &to_s(&*q),
    );

    // Check for AND and a field
    let q = parse_null(&mfqp, "one AND two AND foo:test")?;
    assert_str_eq(
        "+((b:one)^5.0 (t:one)^10.0) +((b:two)^5.0 (t:two)^10.0) +foo:test",
        &to_s(&*q),
    );

    let q = parse_null(&mfqp, "one^3 AND two^4")?;
    assert_str_eq(
        "+((b:one)^5.0 (t:one)^10.0)^3.0 +((b:two)^5.0 (t:two)^10.0)^4.0",
        &to_s(&*q),
    );
    Ok(())
}

fn test_static_method1() -> Result<()> {
    let fields = field_refs(&["b", "t"]);
    let queries = ["one", "two"];
    let q = QueryParserUtil::parse_queries(&queries, &fields, Some(mock_analyzer_default()))?;
    assert_str_eq("b:one t:two", &to_s(&*q));

    let queries2 = ["+one", "+two"];
    let q = QueryParserUtil::parse_queries(&queries2, &fields, Some(mock_analyzer_default()))?;
    assert_str_eq("b:one t:two", &to_s(&*q));

    let queries3 = ["one", "+two"];
    let q = QueryParserUtil::parse_queries(&queries3, &fields, Some(mock_analyzer_default()))?;
    assert_str_eq("b:one t:two", &to_s(&*q));

    let queries4 = ["one +more", "+two"];
    let q = QueryParserUtil::parse_queries(&queries4, &fields, Some(mock_analyzer_default()))?;
    assert_str_eq("(b:one +b:more) t:two", &to_s(&*q));

    let queries5 = ["blah"];
    // expected exception, array length differs
    expect_throws("java.lang.IllegalArgumentException", || {
        QueryParserUtil::parse_queries(&queries5, &fields, Some(mock_analyzer_default()))
    });

    // check also with stop words for this static form (qtxts[], fields[]).
    let stop_a = qp_test_analyzer();

    let queries6 = ["((+stop))", "+((stop))"];
    let q = QueryParserUtil::parse_queries(&queries6, &fields, Some(Arc::clone(&stop_a)))?;
    assert_str_eq("MatchNoDocsQuery(\"\") MatchNoDocsQuery(\"\")", &to_s(&*q));
    // assertEquals(" ", q.toString());

    let queries7 = ["one ((+stop)) +more", "+((stop)) +two"];
    let q = QueryParserUtil::parse_queries(&queries7, &fields, Some(stop_a))?;
    assert_str_eq("(b:one +b:more) (+t:two)", &to_s(&*q));
    Ok(())
}

fn test_static_method2() -> Result<()> {
    let fields = field_refs(&["b", "t"]);
    let flags = [Occur::MUST, Occur::MUST_NOT];
    let q =
        QueryParserUtil::parse_with_flags("one", &fields, &flags, Some(mock_analyzer_default()))?;
    assert_str_eq("+b:one -t:one", &to_s(&*q));

    let q = QueryParserUtil::parse_with_flags(
        "one two",
        &fields,
        &flags,
        Some(mock_analyzer_default()),
    )?;
    assert_str_eq("+(b:one b:two) -(t:one t:two)", &to_s(&*q));

    // expected exception, array length differs
    expect_throws("java.lang.IllegalArgumentException", || {
        let flags2 = [Occur::MUST];
        QueryParserUtil::parse_with_flags("blah", &fields, &flags2, Some(mock_analyzer_default()))
    });
    Ok(())
}

fn test_static_method2_old() -> Result<()> {
    let names = ["b", "t"];
    let fields = field_refs(&names);
    let flags = [Occur::MUST, Occur::MUST_NOT];
    let mut parser = StandardQueryParser::new();
    parser.set_multi_fields(Some(multi_fields(&names)))?;
    parser.set_analyzer(Some(mock_analyzer_default()))?;

    let q =
        QueryParserUtil::parse_with_flags("one", &fields, &flags, Some(mock_analyzer_default()))?; // , fields, flags, new
                                                                                                   // MockAnalyzer());
    assert_str_eq("+b:one -t:one", &to_s(&*q));

    let q = QueryParserUtil::parse_with_flags(
        "one two",
        &fields,
        &flags,
        Some(mock_analyzer_default()),
    )?;
    assert_str_eq("+(b:one b:two) -(t:one t:two)", &to_s(&*q));

    // expected exception, array length differs
    expect_throws("java.lang.IllegalArgumentException", || {
        let flags2 = [Occur::MUST];
        QueryParserUtil::parse_with_flags("blah", &fields, &flags2, Some(mock_analyzer_default()))
    });
    Ok(())
}

fn test_static_method3() -> Result<()> {
    let queries = ["one", "two", "three"];
    let fields = field_refs(&["f1", "f2", "f3"]);
    let flags = [Occur::MUST, Occur::MUST_NOT, Occur::SHOULD];
    let q = QueryParserUtil::parse_queries_with_flags(
        &queries,
        &fields,
        &flags,
        Some(mock_analyzer_default()),
    )?;
    assert_str_eq("+f1:one -f2:two f3:three", &to_s(&*q));

    // expected exception, array length differs
    expect_throws("java.lang.IllegalArgumentException", || {
        let flags2 = [Occur::MUST];
        QueryParserUtil::parse_queries_with_flags(
            &queries,
            &fields,
            &flags2,
            Some(mock_analyzer_default()),
        )
    });
    Ok(())
}

fn test_static_method3_old() -> Result<()> {
    let queries = ["one", "two"];
    let fields = field_refs(&["b", "t"]);
    let flags = [Occur::MUST, Occur::MUST_NOT];
    let q = QueryParserUtil::parse_queries_with_flags(
        &queries,
        &fields,
        &flags,
        Some(mock_analyzer_default()),
    )?;
    assert_str_eq("+b:one -t:two", &to_s(&*q));

    // expected exception, array length differs
    expect_throws("java.lang.IllegalArgumentException", || {
        let flags2 = [Occur::MUST];
        QueryParserUtil::parse_queries_with_flags(
            &queries,
            &fields,
            &flags2,
            Some(mock_analyzer_default()),
        )
    });
    Ok(())
}

fn test_analyzer_returning_null() -> Result<()> {
    let fields = ["f1", "f2", "f3"];
    let mut parser = StandardQueryParser::new();
    parser.set_multi_fields(Some(multi_fields(&fields)))?;
    parser.set_analyzer(Some(Arc::new(AnalyzerReturningNull::new())))?;

    let q = parse_null(&parser, "bla AND blo")?;
    assert_str_eq("+(f2:bla f3:bla) +(f2:blo f3:blo)", &to_s(&*q));
    // the following queries are not affected as their terms are not
    // analyzed anyway:
    let q = parse_null(&parser, "bla*")?;
    assert_str_eq("f1:bla* f2:bla* f3:bla*", &to_s(&*q));
    let q = parse_null(&parser, "bla~")?;
    assert_str_eq("f1:bla~2 f2:bla~2 f3:bla~2", &to_s(&*q));
    let q = parse_null(&parser, "[a TO c]")?;
    assert_str_eq("f1:[a TO c] f2:[a TO c] f3:[a TO c]", &to_s(&*q));
    Ok(())
}

fn test_stop_word_searching() -> Result<()> {
    let analyzer = mock_analyzer_default();
    let ram_dir = LuceneTestCase::new_directory();
    let iw = IndexWriter::new(
        ram_dir.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::clone(&analyzer))),
    )?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "body",
        "blah the footest blah",
        Store::NO,
    )));
    iw.add_document(&doc)?;
    iw.close()?;

    let mut mfqp = StandardQueryParser::new();

    mfqp.set_multi_fields(Some(multi_fields(&["body"])))?;
    mfqp.set_analyzer(Some(analyzer))?;
    mfqp.set_default_operator(Some(Operator::And))?;
    let q = parse_null(&mfqp, "the footest")?;
    let ir = directory_reader::open(ram_dir.as_directory())?;
    let is = LuceneTestCase::new_searcher(Arc::clone(&ir));
    let hits = is.search(q, 1000)?.score_docs;
    Assert::assert_equals(1, hits.len());
    ir.close()?;
    ram_dir.close()?;
    Ok(())
}

/// Return no tokens for field "f1".
#[derive(Debug)]
struct AnalyzerReturningNull {
    state: AnalyzerState,
    std_analyzer: MockAnalyzer,
}

impl AnalyzerReturningNull {
    fn new() -> Self {
        AnalyzerReturningNull {
            std_analyzer: MockAnalyzer::new(&*random()),
            // `super(PER_FIELD_REUSE_STRATEGY)`
            state: AnalyzerState::new(Box::new(PerFieldReuseStrategy)),
        }
    }
}

impl Analyzer for AnalyzerReturningNull {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.queryparser.flexible.standard.TestMultiFieldQPHelper$AnalyzerReturningNull"
    }

    fn init_reader(
        &self,
        field_name: Option<&str>,
        reader: Box<dyn CharReader>,
    ) -> Box<dyn CharReader> {
        if field_name == Some("f1") {
            // we don't use the reader, so close it:
            drop(reader);
            // return empty reader, so MockTokenizer returns no tokens:
            Box::new(StringCharReader::new(""))
        } else {
            reader
        }
    }

    fn create_components(&self, field_name: Option<&str>) -> Result<TokenStreamComponents> {
        self.std_analyzer.create_components(field_name)
    }
}
