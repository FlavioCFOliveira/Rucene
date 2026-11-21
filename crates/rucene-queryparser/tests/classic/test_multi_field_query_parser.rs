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

//! Port of `TestMultiFieldQueryParser.java`.

use std::collections::HashMap;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::TokenStreamComponents;
use rucene::analysis::{
    Analyzer, AnalyzerState, CharReader, PerFieldReuseStrategy, StringCharReader,
};
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query::Builder as BooleanQueryBuilder;
use rucene::search::regexp_query::RegexpQuery;
use rucene_queryparser::classic::multi_field_query_parser::MultiFieldQueryParser;
use rucene_queryparser::classic::query_parser::{Operator, QueryParserOps};
use rucene_queryparser::classic::query_parser_base::{QueryParserBaseOps, AND_OPERATOR};
use rucene_test_framework::analysis::mock_analyzer::MockAnalyzer;
use rucene_test_framework::analysis::mock_synonym_filter::MockSynonymFilter;
use rucene_test_framework::analysis::mock_tokenizer::WHITESPACE;
use rucene_test_framework::analysis::{components, shared, sink};
use rucene_test_framework::util::LuceneTestCase;

use crate::qp_support::core_support::{class, random, test};
use crate::qp_support::query_parser_test_base::{
    mock_analyzer_default, mock_tokenizer, qp_test_analyzer, FnAnalyzer,
};
use crate::qp_support::search_support::term;
use crate::qp_support::{assert_query_eq, assert_str_eq, expect_throws, jdk_default_hooks, to_s};

/// `TestMultiFieldQueryParser`.
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
        test("testSimpleRegex", test_simple_regex),
        test("testSynonyms", test_synonyms),
    ]);
    class(
        "org.apache.lucene.queryparser.classic.TestMultiFieldQueryParser",
        methods,
    )
}

fn strings(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

/// test stop words parsing for both the non static form, and for the
/// corresponding static form (qtxt, fields[]).
fn test_stopwords_parsing() -> Result<()> {
    assert_stop_query_equals("one", "b:one t:one")?;
    assert_stop_query_equals("one stop", "b:one t:one")?;
    assert_stop_query_equals("one (stop)", "b:one t:one")?;
    assert_stop_query_equals("one ((stop))", "b:one t:one")?;
    assert_stop_query_equals("stop", "")?;
    assert_stop_query_equals("(stop)", "")?;
    assert_stop_query_equals("((stop))", "")?;
    Ok(())
}

// verify parsing of query using a stopping analyzer
fn assert_stop_query_equals(qtxt: &str, expected_res: &str) -> Result<()> {
    let fields = ["b", "t"];
    let occur = [Occur::SHOULD, Occur::SHOULD];
    let a = qp_test_analyzer();
    let mut mfqp = MultiFieldQueryParser::new(strings(&fields), Some(Arc::clone(&a)))?;

    let q = mfqp.parse(qtxt)?;
    assert_str_eq(expected_res, &to_s(&*q));

    let q = MultiFieldQueryParser::parse_with_flags(qtxt, &fields, &occur, Some(a))?;
    assert_str_eq(expected_res, &to_s(&*q));
    Ok(())
}

fn test_simple() -> Result<()> {
    let fields = ["b", "t"];
    let mut mfqp = MultiFieldQueryParser::new(strings(&fields), Some(mock_analyzer_default()))?;

    for (q, expected) in [
        ("one", "b:one t:one"),
        ("one two", "(b:one t:one) (b:two t:two)"),
        ("+one +two", "+(b:one t:one) +(b:two t:two)"),
        (
            "+one -two -three",
            "+(b:one t:one) -(b:two t:two) -(b:three t:three)",
        ),
        ("one^2 two", "(b:one t:one)^2.0 (b:two t:two)"),
        ("one~ two", "(b:one~2 t:one~2) (b:two t:two)"),
        ("one~0.8 two^2", "(b:one~0 t:one~0) (b:two t:two)^2.0"),
        ("one* two*", "(b:one* t:one*) (b:two* t:two*)"),
        ("[a TO c] two", "(b:[a TO c] t:[a TO c]) (b:two t:two)"),
        ("w?ldcard", "b:w?ldcard t:w?ldcard"),
        ("\"foo bar\"", "b:\"foo bar\" t:\"foo bar\""),
        (
            "\"aa bb cc\" \"dd ee\"",
            "(b:\"aa bb cc\" t:\"aa bb cc\") (b:\"dd ee\" t:\"dd ee\")",
        ),
        ("\"foo bar\"~4", "b:\"foo bar\"~4 t:\"foo bar\"~4"),
        // LUCENE-1213: MultiFieldQueryParser was ignoring slop when phrase had a field.
        ("b:\"foo bar\"~4", "b:\"foo bar\"~4"),
        // make sure that terms which have a field are not touched:
        ("one f:two", "(b:one t:one) f:two"),
    ] {
        assert_str_eq(expected, &to_s(&*mfqp.parse(q)?));
    }

    // AND mode:
    mfqp.set_default_operator(AND_OPERATOR);
    assert_str_eq(
        "+(b:one t:one) +(b:two t:two)",
        &to_s(&*mfqp.parse("one two")?),
    );
    assert_str_eq(
        "+(b:\"aa bb cc\" t:\"aa bb cc\") +(b:\"dd ee\" t:\"dd ee\")",
        &to_s(&*mfqp.parse("\"aa bb cc\" \"dd ee\"")?),
    );
    Ok(())
}

fn test_boosts_simple() -> Result<()> {
    let mut boosts = HashMap::new();
    boosts.insert("b".to_string(), 5.0f32);
    boosts.insert("t".to_string(), 10.0f32);
    let fields = ["b", "t"];
    let mut mfqp = MultiFieldQueryParser::with_boosts(
        strings(&fields),
        Some(mock_analyzer_default()),
        Some(boosts),
    )?;

    for (q, expected) in [
        // Check for simple
        ("one", "(b:one)^5.0 (t:one)^10.0"),
        // Check for AND
        (
            "one AND two",
            "+((b:one)^5.0 (t:one)^10.0) +((b:two)^5.0 (t:two)^10.0)",
        ),
        // Check for OR
        (
            "one OR two",
            "((b:one)^5.0 (t:one)^10.0) ((b:two)^5.0 (t:two)^10.0)",
        ),
        // Check for AND and a field
        (
            "one AND two AND foo:test",
            "+((b:one)^5.0 (t:one)^10.0) +((b:two)^5.0 (t:two)^10.0) +foo:test",
        ),
        // Check boost with slop
        // See https://github.com/apache/lucene/issues/12195
        (
            "\"one two\"~2",
            "(b:\"one two\"~2)^5.0 (t:\"one two\"~2)^10.0",
        ),
        // check boost with fuzzy
        ("one~", "(b:one~2)^5.0 (t:one~2)^10.0"),
        // check boost with prefix
        ("one*", "(b:one*)^5.0 (t:one*)^10.0"),
        // check boost with wildcard
        ("o?n*e", "(b:o?n*e)^5.0 (t:o?n*e)^10.0"),
        // check boost with regex
        ("/[a-z][123]/", "(b:/[a-z][123]/)^5.0 (t:/[a-z][123]/)^10.0"),
        // check boost with range
        ("[one TO two]", "(b:[one TO two])^5.0 (t:[one TO two])^10.0"),
        (
            "one^3 AND two^4",
            "+((b:one)^5.0 (t:one)^10.0)^3.0 +((b:two)^5.0 (t:two)^10.0)^4.0",
        ),
    ] {
        assert_str_eq(expected, &to_s(&*mfqp.parse(q)?));
    }
    Ok(())
}

fn test_static_method1() -> Result<()> {
    let fields = ["b", "t"];
    let q = MultiFieldQueryParser::parse_queries(
        &["one", "two"],
        &fields,
        Some(mock_analyzer_default()),
    )?;
    assert_str_eq("b:one t:two", &to_s(&*q));

    let q = MultiFieldQueryParser::parse_queries(
        &["+one", "+two"],
        &fields,
        Some(mock_analyzer_default()),
    )?;
    assert_str_eq("(+b:one) (+t:two)", &to_s(&*q));

    let q = MultiFieldQueryParser::parse_queries(
        &["one", "+two"],
        &fields,
        Some(mock_analyzer_default()),
    )?;
    assert_str_eq("b:one (+t:two)", &to_s(&*q));

    let q = MultiFieldQueryParser::parse_queries(
        &["one +more", "+two"],
        &fields,
        Some(mock_analyzer_default()),
    )?;
    assert_str_eq("(b:one +b:more) (+t:two)", &to_s(&*q));

    let queries5 = ["blah"];
    // expected exception, array length differs
    expect_throws("java.lang.IllegalArgumentException", || {
        MultiFieldQueryParser::parse_queries(&queries5, &fields, Some(mock_analyzer_default()))
    });

    // check also with stop words for this static form (qtxts[], fields[]).
    let stop_a = qp_test_analyzer();

    let q = MultiFieldQueryParser::parse_queries(
        &["((+stop))", "+((stop))"],
        &fields,
        Some(Arc::clone(&stop_a)),
    )?;
    assert_str_eq("", &to_s(&*q));

    let q = MultiFieldQueryParser::parse_queries(
        &["one ((+stop)) +more", "+((stop)) +two"],
        &fields,
        Some(stop_a),
    )?;
    assert_str_eq("(b:one +b:more) (+t:two)", &to_s(&*q));
    Ok(())
}

fn test_static_method2() -> Result<()> {
    let fields = ["b", "t"];
    let flags = [Occur::MUST, Occur::MUST_NOT];
    let q = MultiFieldQueryParser::parse_with_flags(
        "one",
        &fields,
        &flags,
        Some(mock_analyzer_default()),
    )?;
    assert_str_eq("+b:one -t:one", &to_s(&*q));

    let q = MultiFieldQueryParser::parse_with_flags(
        "one two",
        &fields,
        &flags,
        Some(mock_analyzer_default()),
    )?;
    assert_str_eq("+(b:one b:two) -(t:one t:two)", &to_s(&*q));

    // expected exception, array length differs
    expect_throws("java.lang.IllegalArgumentException", || {
        let flags2 = [Occur::MUST];
        MultiFieldQueryParser::parse_with_flags(
            "blah",
            &fields,
            &flags2,
            Some(mock_analyzer_default()),
        )
    });
    Ok(())
}

fn test_static_method2_old() -> Result<()> {
    let fields = ["b", "t"];
    // int[] flags = {MultiFieldQueryParser.REQUIRED_FIELD, MultiFieldQueryParser.PROHIBITED_FIELD};
    let flags = [Occur::MUST, Occur::MUST_NOT];

    let q = MultiFieldQueryParser::parse_with_flags(
        "one",
        &fields,
        &flags,
        Some(mock_analyzer_default()),
    )?; // , fields, flags, new MockAnalyzer(random));
    assert_str_eq("+b:one -t:one", &to_s(&*q));

    let q = MultiFieldQueryParser::parse_with_flags(
        "one two",
        &fields,
        &flags,
        Some(mock_analyzer_default()),
    )?;
    assert_str_eq("+(b:one b:two) -(t:one t:two)", &to_s(&*q));

    // expected exception, array length differs
    expect_throws("java.lang.IllegalArgumentException", || {
        let flags2 = [Occur::MUST];
        MultiFieldQueryParser::parse_with_flags(
            "blah",
            &fields,
            &flags2,
            Some(mock_analyzer_default()),
        )
    });
    Ok(())
}

fn test_static_method3() -> Result<()> {
    let queries = ["one", "two", "three"];
    let fields = ["f1", "f2", "f3"];
    let flags = [Occur::MUST, Occur::MUST_NOT, Occur::SHOULD];
    let q = MultiFieldQueryParser::parse_queries_with_flags(
        &queries,
        &fields,
        &flags,
        Some(mock_analyzer_default()),
    )?;
    assert_str_eq("+f1:one -f2:two f3:three", &to_s(&*q));

    // expected exception, array length differs
    expect_throws("java.lang.IllegalArgumentException", || {
        let flags2 = [Occur::MUST];
        MultiFieldQueryParser::parse_queries_with_flags(
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
    let fields = ["b", "t"];
    let flags = [Occur::MUST, Occur::MUST_NOT];
    let q = MultiFieldQueryParser::parse_queries_with_flags(
        &queries,
        &fields,
        &flags,
        Some(mock_analyzer_default()),
    )?;
    assert_str_eq("+b:one -t:two", &to_s(&*q));

    // expected exception, array length differs
    expect_throws("java.lang.IllegalArgumentException", || {
        let flags2 = [Occur::MUST];
        MultiFieldQueryParser::parse_queries_with_flags(
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
    let mut parser = MultiFieldQueryParser::new(
        strings(&fields),
        Some(Arc::new(AnalyzerReturningNull::new())),
    )?;
    let q = parser.parse("bla AND blo")?;
    assert_str_eq("+(f2:bla f3:bla) +(f2:blo f3:blo)", &to_s(&*q));
    // the following queries are not affected as their terms are not analyzed anyway:
    let q = parser.parse("bla*")?;
    assert_str_eq("f1:bla* f2:bla* f3:bla*", &to_s(&*q));
    let q = parser.parse("bla~")?;
    assert_str_eq("f1:bla~2 f2:bla~2 f3:bla~2", &to_s(&*q));
    let q = parser.parse("[a TO c]")?;
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
    doc.add(LuceneTestCase::new_text_field(
        "body",
        "blah the footest blah",
        Store::NO,
    ));
    iw.add_document(&doc)?;
    iw.close()?;

    let mut mfqp = MultiFieldQueryParser::new(vec!["body".to_string()], Some(analyzer))?;
    mfqp.set_default_operator(Operator::And);
    let q = mfqp.parse("the footest")?;
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
        "org.apache.lucene.queryparser.classic.TestMultiFieldQueryParser$AnalyzerReturningNull"
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

fn test_simple_regex() -> Result<()> {
    let fields = ["a", "b"];
    let mut mfqp = MultiFieldQueryParser::new(strings(&fields), Some(mock_analyzer_default()))?;

    let mut bq = BooleanQueryBuilder::new();
    bq.add(
        Arc::new(RegexpQuery::new(term("a", "[a-z][123]"))?),
        Occur::SHOULD,
    )?;
    bq.add(
        Arc::new(RegexpQuery::new(term("b", "[a-z][123]"))?),
        Occur::SHOULD,
    )?;
    assert_query_eq(&bq.build(), &*mfqp.parse("/[a-z][123]/")?);
    Ok(())
}

/// whitespace+lowercase analyzer with synonyms (dogs,dog) and (guinea
/// pig,cavy)
fn mock_synonym_analyzer() -> Arc<dyn Analyzer> {
    FnAnalyzer::new(
        "org.apache.lucene.queryparser.classic.TestMultiFieldQueryParser$MockSynonymAnalyzer",
        |_| {
            let tokenizer = mock_tokenizer(&WHITESPACE, true)?;
            let filter = MockSynonymFilter::new(shared(&tokenizer))?;
            Ok(components(tokenizer, sink(filter)))
        },
    )
    .arc()
}

fn test_synonyms() -> Result<()> {
    let fields = ["b", "t"];
    let mut parser = MultiFieldQueryParser::new(strings(&fields), Some(mock_synonym_analyzer()))?;
    let q = parser.parse("dogs")?;
    assert_str_eq("Synonym(b:dog b:dogs) Synonym(t:dog t:dogs)", &to_s(&*q));
    let q = parser.parse("guinea pig")?;
    Assert::assert_false(parser.get_split_on_whitespace());
    assert_str_eq(
        "((+b:guinea +b:pig) b:cavy) ((+t:guinea +t:pig) t:cavy)",
        &to_s(&*q),
    );
    parser.set_split_on_whitespace(true)?;
    let q = parser.parse("guinea pig")?;
    assert_str_eq("(b:guinea t:guinea) (b:pig t:pig)", &to_s(&*q));
    Ok(())
}
