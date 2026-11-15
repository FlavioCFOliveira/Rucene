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

//! Port of `TestSpanWithinQuery.java`: basic tests for SpanWithinQuery.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{AfterClass, Assert, BeforeClass};
use rucene::analysis::Analyzer;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::Term;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::Query;
use rucene_queries::spans::span_near_query::SpanNearQuery;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_queries::spans::span_term_query::SpanTermQuery;
use rucene_queries::spans::span_within_query::SpanWithinQuery;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::check_hits::CheckHits;
use rucene_test_framework::search::query_utils::QueryUtils;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::queries_support::core_support::{class, random, test, IAE};
use crate::queries_support::search_support::static_hook;
use rucene_test_framework::util::java_exceptions::expect_throws;

/// The static fields `searcher`, `directory` and `reader`.
struct Statics {
    searcher: Arc<IndexSearcher>,
    directory: TestDirectory,
    reader: Arc<dyn DirectoryReader>,
}

static STATICS: Mutex<Option<Statics>> = Mutex::new(None);

fn searcher() -> Arc<IndexSearcher> {
    let guard = STATICS.lock().unwrap_or_else(|e| e.into_inner());
    Arc::clone(&guard.as_ref().expect("beforeClass ran").searcher)
}

/// `TestSpanWithinQuery`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.queries.spans.TestSpanWithinQuery",
        vec![
            static_hook("beforeClass", BeforeClass, before_class),
            static_hook("afterClass", AfterClass, after_class),
            test("testHashcodeEquals", test_hashcode_equals),
            test("testBasicWithin", test_basic_within),
            test("testNoMatches", test_no_matches),
            test(
                "testDifferentFieldsThrowsIllegalArgumentException",
                test_different_fields_throws_illegal_argument_exception,
            ),
            test("testToString", test_to_string),
            test("testBasicNear", test_basic_near),
            test("testDescendingSequence", test_descending_sequence),
        ],
    )
}

fn before_class() -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::new(&*random()));
    let config = LuceneTestCase::new_index_writer_config_analyzer(Some(analyzer));
    config.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let writer = RandomIndexWriter::new_with_config(&*random(), directory.as_directory(), config)?;

    add_doc(&writer, "one two three four five")?;
    add_doc(&writer, "one two three four five six")?;
    add_doc(&writer, "one three five seven nine")?;
    add_doc(&writer, "two four six eight ten")?;
    add_doc(&writer, "one three five seven ten")?;
    add_doc(&writer, "one four seven eight nine")?;
    add_doc(&writer, "two one three five seven")?;
    add_doc(&writer, "three five seven nine ten")?;
    add_doc(&writer, "ten eight six four two")?;
    add_doc(&writer, "one two two three four")?;
    add_doc(&writer, "two four six six eight")?;
    add_doc(&writer, "one three five five seven")?;
    add_doc(&writer, "one two three four five six seven")?;
    add_doc(&writer, "ten nine eight seven six five four")?;
    add_doc(&writer, "one three five seven nine ten eight")?;
    add_doc(&writer, "two four six eight ten nine seven")?;

    let reader = writer.get_reader()?;
    let searcher = Arc::new(LuceneTestCase::new_searcher(Arc::clone(&reader)));
    writer.close()?;
    *STATICS.lock().unwrap_or_else(|e| e.into_inner()) = Some(Statics {
        searcher,
        directory,
        reader,
    });
    Ok(())
}

fn after_class() -> Result<()> {
    let taken = STATICS.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(Statics {
        searcher,
        directory,
        reader,
    }) = taken
    {
        reader.close()?;
        directory.close()?;
        drop(searcher);
    }
    Ok(())
}

/// `addDoc(RandomIndexWriter writer, String content)`.
fn add_doc(writer: &RandomIndexWriter, content: &str) -> Result<()> {
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "field",
        content,
        Store::YES,
    )));
    writer.add_document(&doc)?;
    Ok(())
}

/// `check(SpanQuery q, int[] docs)`.
fn check(q: Arc<dyn SpanQuery>, docs: &[i32]) -> Result<()> {
    CheckHits::check_hit_collector(&*random(), &(q as Arc<dyn Query>), None, &searcher(), docs)
}

fn test_hashcode_equals() -> Result<()> {
    let q1 = span_query("field", "one", "two")?;
    let q2 = span_query("field", "one", "thre")?;
    let q3 = span_query("field", "one", "two")?;
    let q4 = span_query("field", "one", "four")?;

    let query1 = SpanWithinQuery::new(q1, q2)?;
    let query2 = SpanWithinQuery::new(q3, q4)?;

    QueryUtils::check(&query1);
    QueryUtils::check(&query2);
    QueryUtils::check_unequal(&query1, &query2);
    Ok(())
}

fn test_basic_within() -> Result<()> {
    let big = span_query("field", "one", "five")?;
    let little = span_query("field", "two", "three")?;
    let query = Arc::new(SpanWithinQuery::new(big, little)?);

    let expected_docs = [0, 1, 12];

    check(query, &expected_docs)
}

fn test_no_matches() -> Result<()> {
    let big = span_query("field", "ten", "one")?;
    let little = span_query("field", "two", "three")?;
    let query = Arc::new(SpanWithinQuery::new(big, little)?);

    let expected_docs: [i32; 0] = [];

    check(query, &expected_docs)
}

fn test_different_fields_throws_illegal_argument_exception() -> Result<()> {
    let big: Arc<dyn SpanQuery> = Arc::new(SpanTermQuery::new(Term::from_text("field1", "one")));
    let little: Arc<dyn SpanQuery> = Arc::new(SpanTermQuery::new(Term::from_text("field2", "two")));

    expect_throws(IAE, || SpanWithinQuery::new(big, little));
    Ok(())
}

fn test_to_string() -> Result<()> {
    let big = span_query("field", "one", "four")?;
    let little = span_query("field", "two", "three")?;
    let query = SpanWithinQuery::new(big, little)?;

    let query_string = query.to_query_string("field");

    Assert::assert_true(query_string.contains("SpanWithin"));
    Assert::assert_true(query_string.contains("one"));
    Assert::assert_true(query_string.contains("two"));
    Assert::assert_true(query_string.contains("three"));
    Assert::assert_true(query_string.contains("four"));
    Ok(())
}

fn test_basic_near() -> Result<()> {
    let big = span_near_query("field", &["one", "two", "three", "four"])?;
    let little = span_near_query("field", &["two", "three"])?;
    let query: Arc<dyn Query> = Arc::new(SpanWithinQuery::new(big, little)?);

    let expected_docs = [0, 1, 9, 12];

    CheckHits::check_hit_collector(&*random(), &query, None, &searcher(), &expected_docs)
}

fn test_descending_sequence() -> Result<()> {
    let big = span_near_query("field", &["ten", "nine", "eight"])?;
    let little = span_near_query("field", &["nine", "eight"])?;
    let query: Arc<dyn Query> = Arc::new(SpanWithinQuery::new(big, little)?);

    let expected_docs = [13];

    CheckHits::check_hit_collector(&*random(), &query, None, &searcher(), &expected_docs)
}

/// `spanQuery(String field, String term1, String term2)`.
fn span_query(field: &str, term1: &str, term2: &str) -> Result<Arc<dyn SpanQuery>> {
    let t1: Arc<dyn SpanQuery> = Arc::new(SpanTermQuery::new(Term::from_text(field, term1)));
    let t2: Arc<dyn SpanQuery> = Arc::new(SpanTermQuery::new(Term::from_text(field, term2)));
    Ok(Arc::new(SpanNearQuery::new(vec![t1, t2], 5, true)?))
}

/// `spanNearQuery(String field, String[] terms)`.
fn span_near_query(field: &str, terms: &[&str]) -> Result<Arc<dyn SpanQuery>> {
    let mut clauses: Vec<Arc<dyn SpanQuery>> = Vec::with_capacity(terms.len());
    for i in 0..terms.len() {
        clauses.push(Arc::new(SpanTermQuery::new(Term::from_text(
            field, terms[i],
        ))));
    }
    Ok(Arc::new(SpanNearQuery::new(clauses, 5, true)?))
}
