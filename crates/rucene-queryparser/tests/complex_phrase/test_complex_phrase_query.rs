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

//! Port of `TestComplexPhraseQuery.java`.

use std::collections::HashSet;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::standard::StandardAnalyzer;
use rucene::analysis::Analyzer;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_writer::IndexWriter;
use rucene::search::boost_query::BoostQuery;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::Query;
use rucene_queries::spans::span_near_query::SpanNearQuery;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_queries::spans::span_term_query::SpanTermQuery;
use rucene_queryparser::classic::query_parser_base::QueryParserBaseOps;
use rucene_queryparser::complex_phrase::complex_phrase_query_parser::ComplexPhraseQueryParser;
use rucene_queryparser::flexible::standard::common_query_parser_configuration::CommonQueryParserConfiguration;
use rucene_test_framework::analysis::mock_synonym_analyzer::MockSynonymAnalyzer;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::qp_support::core_support::random;
use crate::qp_support::query_parser_test_base::mock_analyzer_default;
use crate::qp_support::search_support::{fixture_class, ftest, set_up, tear_down, term};
use crate::qp_support::{
    assert_query_eq, assert_str_eq_msg, expect_throws, jdk_default_hooks, to_s,
};

/// `static class DocData`.
struct DocData {
    name: &'static str,
    id: &'static str,
    role: &'static str,
}

/// `docsContent`.
const DOCS_CONTENT: [DocData; 8] = [
    DocData {
        name: "john smith",
        id: "1",
        role: "developer",
    },
    DocData {
        name: "johathon smith",
        id: "2",
        role: "developer",
    },
    DocData {
        name: "john percival smith",
        id: "3",
        role: "designer",
    },
    DocData {
        name: "jackson waits tom",
        id: "4",
        role: "project manager",
    },
    DocData {
        name: "johny perkins",
        id: "5",
        role: "orders pizza",
    },
    DocData {
        name: "hapax neverson",
        id: "6",
        role: "never matches",
    },
    DocData {
        name: "dog cigar",
        id: "7",
        role: "just for synonyms",
    },
    DocData {
        name: "dogs don't smoke cigarettes",
        id: "8",
        role: "just for synonyms",
    },
];

/// The instance of `TestComplexPhraseQuery`.
pub struct TestComplexPhraseQuery {
    rd: Option<TestDirectory>,
    analyzer: Option<Arc<dyn Analyzer>>,
    searcher: Option<IndexSearcher>,
    reader: Option<Arc<dyn DirectoryReader>>,
    default_field_name: String,
    in_order: bool,
}

impl Default for TestComplexPhraseQuery {
    fn default() -> Self {
        TestComplexPhraseQuery {
            rd: None,
            analyzer: None,
            searcher: None,
            reader: None,
            default_field_name: "name".to_string(),
            in_order: true,
        }
    }
}

/// `TestComplexPhraseQuery`.
pub fn class_def() -> Arc<Class> {
    let mut methods = jdk_default_hooks();
    methods.extend([
        set_up(set_up_impl),
        tear_down(tear_down_impl),
        ftest("testComplexPhrases", test_complex_phrases),
        ftest("testSingleTermPhrase", test_single_term_phrase),
        ftest("testSynonyms", test_synonyms),
        ftest(
            "testUnOrderedProximitySearches",
            test_un_ordered_proximity_searches,
        ),
        ftest("testFieldedQuery", test_fielded_query),
        ftest("testToStringContainsSlop", test_to_string_contains_slop),
        ftest("testHashcodeEquals", test_hashcode_equals),
        ftest("testBoosts", test_boosts),
    ]);
    fixture_class::<TestComplexPhraseQuery>(
        "org.apache.lucene.queryparser.complexPhrase.TestComplexPhraseQuery",
        methods,
    )
}

impl TestComplexPhraseQuery {
    fn analyzer(&self) -> Arc<dyn Analyzer> {
        Arc::clone(self.analyzer.as_ref().expect("setUp ran"))
    }

    fn searcher(&self) -> &IndexSearcher {
        self.searcher.as_ref().expect("setUp ran")
    }

    fn check_bad_query(&self, q_string: &str) -> Result<()> {
        let mut qp =
            ComplexPhraseQueryParser::new(Some(&self.default_field_name), Some(self.analyzer()))?;
        qp.set_in_order(self.in_order);
        expect_throws("java.lang.Throwable", || qp.parse(q_string));
        Ok(())
    }

    fn check_matches(&self, q_string: &str, expected_vals: &str) -> Result<()> {
        self.check_matches_with(q_string, expected_vals, self.analyzer())
    }

    fn check_matches_with(
        &self,
        q_string: &str,
        expected_vals: &str,
        an_analyzer: Arc<dyn Analyzer>,
    ) -> Result<()> {
        let mut qp =
            ComplexPhraseQueryParser::new(Some(&self.default_field_name), Some(an_analyzer))?;
        qp.set_in_order(self.in_order);
        qp.set_fuzzy_prefix_length(1)?; // usually a good idea

        let q = qp.parse(q_string)?;

        let mut expecteds: HashSet<String> = HashSet::new();
        // `expectedVals.split(",")`: trailing empty strings removed.
        for v in java_split_comma(expected_vals) {
            if !v.is_empty() {
                expecteds.insert(v);
            }
        }

        let td = self.searcher().search(q, 10)?;
        let sd = td.score_docs;
        let stored_fields = self.searcher().stored_fields()?;
        for s in &sd {
            let doc = stored_fields.document(s.doc)?;
            let id = doc.get("id");
            let id_str = id.clone().unwrap_or_else(|| "null".to_string());
            Assert::assert_true_msg(
                &format!("{q_string}matched doc#{id_str} not expected"),
                id.as_ref().is_some_and(|i| expecteds.contains(i)),
            );
            if let Some(i) = id {
                expecteds.remove(&i);
            }
        }

        Assert::assert_equals_msg(
            Some(&format!("{q_string} missing some matches ")),
            0,
            expecteds.len(),
        );
        Ok(())
    }
}

/// `String.split(",")`: the parts, trailing empty strings removed.
fn java_split_comma(s: &str) -> Vec<String> {
    let mut parts: Vec<String> = s.split(',').map(str::to_string).collect();
    while parts.last().is_some_and(String::is_empty) {
        parts.pop();
    }
    parts
}

fn test_complex_phrases(t: &mut TestComplexPhraseQuery) -> Result<()> {
    t.check_matches("\"john smith\"", "1")?; // Simple multi-term still works
    t.check_matches("\"j*   smyth~\"", "1,2")?; // wildcards and fuzzies are OK in
                                                // phrases
    t.check_matches("\"(jo* -john)  smith\"", "2")?; // boolean logic works
    t.check_matches("\"jo*  smith\"~2", "1,2,3")?; // position logic works.
    t.check_matches("\"jo* [sma TO smZ]\" ", "1,2")?; // range queries supported
    t.check_matches("\"john\"", "1,3")?; // Simple single-term still works
    t.check_matches("\"(john OR johathon)  smith\"", "1,2")?; // boolean logic with
                                                              // brackets works.
    t.check_matches("\"(john OR nosuchword*)  smith\"", "1")?; // boolean logic with
                                                               // brackets works when one of the terms in BooleanQuery does not exist (LUCENE-8305).
    t.check_matches("\"(jo* -john) smyth~\"", "2")?; // boolean logic with
                                                     // brackets works.

    // checkMatches("\"john -percival\"", "1"); // not logic doesn't work
    // currently :(.

    t.check_matches("\"john  nosuchword*\"", "")?; // phrases with clauses producing
                                                   // empty sets

    t.check_bad_query("\"jo*  id:1 smith\"")?; // mixing fields in a phrase is bad
    t.check_bad_query("\"jo* \"smith\" \"")?; // phrases inside phrases is bad
    Ok(())
}

fn test_single_term_phrase(t: &mut TestComplexPhraseQuery) -> Result<()> {
    t.check_matches("\"joh*\"", "1,2,3,5")?;
    t.check_matches("\"joh~\"", "1,3,5")?;
    t.check_matches("\"joh*\" \"tom\"", "1,2,3,4,5")?;
    t.check_matches("+\"j*\" +\"tom\"", "4")?;
    t.check_matches("\"jo*\" \"[sma TO smZ]\" ", "1,2,3,5,8")?;
    t.check_matches("+\"j*hn\" +\"sm*h\"", "1,3")?;
    Ok(())
}

fn test_synonyms(t: &mut TestComplexPhraseQuery) -> Result<()> {
    t.check_matches("\"dogs\"", "8")?;
    let synonym: Arc<dyn Analyzer> = Arc::new(MockSynonymAnalyzer::new());
    t.check_matches_with("\"dogs\"", "7,8", Arc::clone(&synonym))?;
    // synonym is unidirectional
    t.check_matches_with("\"dog\"", "7", Arc::clone(&synonym))?;
    t.check_matches("\"dogs cigar*\"", "")?;
    t.check_matches("\"dog cigar*\"", "7")?;
    t.check_matches_with("\"dogs cigar*\"", "7", Arc::clone(&synonym))?;
    t.check_matches_with("\"dog cigar*\"", "7", Arc::clone(&synonym))?;
    t.check_matches_with("\"dogs cigar*\"~2", "7,8", Arc::clone(&synonym))?;
    // synonym is unidirectional
    t.check_matches_with("\"dog cigar*\"~2", "7", synonym)?;
    Ok(())
}

fn test_un_ordered_proximity_searches(t: &mut TestComplexPhraseQuery) -> Result<()> {
    t.in_order = true;
    t.check_matches("\"smith jo*\"~2", "")?; // ordered proximity produces empty set

    t.in_order = false;
    t.check_matches("\"smith jo*\"~2", "1,2,3")?; // un-ordered proximity
    Ok(())
}

fn test_fielded_query(t: &mut TestComplexPhraseQuery) -> Result<()> {
    t.check_matches("name:\"john smith\"", "1")?;
    t.check_matches("name:\"j*   smyth~\"", "1,2")?;
    t.check_matches("role:\"developer\"", "1,2")?;
    t.check_matches("role:\"p* manager\"", "4")?;
    t.check_matches("role:de*", "1,2,3")?;
    t.check_matches("name:\"j* smyth~\"~5", "1,2,3")?;
    t.check_matches("role:\"p* manager\" AND name:jack*", "4")?;
    t.check_matches("+role:developer +name:jack*", "")?;
    t.check_matches("name:\"john smith\"~2 AND role:designer AND id:3", "3")?;
    Ok(())
}

fn test_to_string_contains_slop(t: &mut TestComplexPhraseQuery) -> Result<()> {
    let mut qp = ComplexPhraseQueryParser::new(Some(""), Some(t.analyzer()))?;
    let slop = random().next_int_bound(31) + 1;

    let q_string = format!("name:\"j* smyth~\"~{slop}");
    let query = qp.parse(&q_string)?;
    let actual_q_str = to_s(&*query);
    Assert::assert_true_msg(
        "Slop is not shown in toString()",
        actual_q_str.ends_with(&format!("~{slop}")),
    );
    crate::qp_support::assert_str_eq(&q_string, &actual_q_str);

    let string = "\"j* smyth~\"";
    let q = qp.parse(string)?;
    assert_str_eq_msg("Don't show implicit slop of zero", &to_s(&*q), string);
    Ok(())
}

fn test_hashcode_equals(t: &mut TestComplexPhraseQuery) -> Result<()> {
    let mut qp = ComplexPhraseQueryParser::new(Some(&t.default_field_name), Some(t.analyzer()))?;
    qp.set_in_order(true);
    qp.set_fuzzy_prefix_length(1)?;

    let q_string = "\"aaa* bbb*\"";

    let q = qp.parse(q_string)?;
    let q2 = qp.parse(q_string)?;

    Assert::assert_equals(q.query_hash(), q2.query_hash());
    assert_query_eq(&*q, &*q2);

    qp.set_in_order(false); // SOLR-6011

    let q2 = qp.parse(q_string)?;

    // although the general contract of hashCode can't guarantee different values, if we only change
    // one thing
    // about a single query, it normally should result in a different value (and will with the
    // current
    // implementation in ComplexPhraseQuery)
    Assert::assert_true(q.query_hash() != q2.query_hash());
    Assert::assert_true(!q.query_eq(&*q2));
    Assert::assert_true(!q2.query_eq(&*q));
    Ok(())
}

fn test_boosts(t: &mut TestComplexPhraseQuery) -> Result<()> {
    // top-level boosts should be preserved, interior boosts are ignored as they don't apply to
    // spans
    let top_level = "(\"john^3 smit*\"~4)^2";
    let mut parser =
        ComplexPhraseQueryParser::new(Some("name"), Some(Arc::new(StandardAnalyzer::new())))?;
    parser.set_in_order(true);
    let actual = t.searcher().rewrite(parser.parse(top_level)?)?;
    let clauses: Vec<Arc<dyn SpanQuery>> = vec![
        Arc::new(SpanTermQuery::new(term("name", "john"))),
        Arc::new(SpanTermQuery::new(term("name", "smith"))),
    ];
    let expected: Arc<dyn Query> = Arc::new(SpanNearQuery::new(clauses, 4, true)?);
    let expected = BoostQuery::new(expected, 2.0)?;
    assert_query_eq(&expected, &*actual);
    Ok(())
}

fn set_up_impl(t: &mut TestComplexPhraseQuery) -> Result<()> {
    let analyzer = mock_analyzer_default();
    let rd = LuceneTestCase::new_directory();
    let w = IndexWriter::new(
        rd.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::clone(&analyzer))),
    )?;
    for d in &DOCS_CONTENT {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_text_field(
            "name",
            d.name,
            Store::YES,
        )));
        doc.add(Box::new(LuceneTestCase::new_text_field(
            "id",
            d.id,
            Store::YES,
        )));
        doc.add(Box::new(LuceneTestCase::new_text_field(
            "role",
            d.role,
            Store::YES,
        )));
        w.add_document(&doc)?;
    }
    w.close()?;
    let reader = directory_reader::open(rd.as_directory())?;
    t.searcher = Some(LuceneTestCase::new_searcher(Arc::clone(&reader)));
    t.reader = Some(reader);
    t.analyzer = Some(analyzer);
    t.rd = Some(rd);
    Ok(())
}

fn tear_down_impl(t: &mut TestComplexPhraseQuery) -> Result<()> {
    t.searcher = None;
    if let Some(reader) = t.reader.take() {
        reader.close()?;
    }
    if let Some(rd) = t.rd.take() {
        rd.close()?;
    }
    Ok(())
}
