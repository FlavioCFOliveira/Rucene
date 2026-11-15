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

//! Port of `TestFieldMaskingSpanQuery.java`.

use std::any::Any;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{AfterClass, Assert, BeforeClass};
use rucene::analysis::Analyzer;
use rucene::document::{Document, Field, Store};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::{IndexReader, Term};
use rucene::search::doc_id_set_iterator::NO_MORE_DOCS;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::Query;
use rucene::search::query_visitor::{term_collector, QueryVisitor};
use rucene::search::score_mode::ScoreMode;
use rucene::search::weight::Weight;
use rucene_queries::spans::field_masking_span_query::FieldMaskingSpanQuery;
use rucene_queries::spans::span_near_query::SpanNearQuery;
use rucene_queries::spans::span_or_query::SpanOrQuery;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_queries::spans::span_term_query::SpanTermQuery;
use rucene_queries::spans::span_weight::{Postings, SpanWeight};
use rucene_queries::spans::spans::{Spans, NO_MORE_POSITIONS};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::check_hits::CheckHits;
use rucene_test_framework::search::query_utils::QueryUtils;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::queries_support::core_support::{class, random, test};
use crate::queries_support::search_support::static_hook;
use crate::queries_support::span_test_util::{assert_finished, assert_next};
use crate::queries_support::{assert_not_same_int, leaf_spans, non_null_spans};

/// `doc(Field[] fields)`.
fn doc(fields: Vec<Field>) -> Document {
    let mut doc = Document::new();
    for f in fields {
        doc.add(Box::new(f));
    }
    doc
}

/// `field(String name, String value)`.
fn field(name: &str, value: &str) -> Field {
    LuceneTestCase::new_text_field(name, value, Store::NO)
}

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

/// `TestFieldMaskingSpanQuery`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.queries.spans.TestFieldMaskingSpanQuery",
        vec![
            static_hook("beforeClass", BeforeClass, before_class),
            static_hook("afterClass", AfterClass, after_class),
            test("testRewrite0", test_rewrite0),
            test("testRewrite1", test_rewrite1),
            test("testRewrite2", test_rewrite2),
            test("testEquality1", test_equality1),
            test("testNoop0", test_noop0),
            test("testNoop1", test_noop1),
            test("testSimple1", test_simple1),
            test("testSimple2", test_simple2),
            test("testSpans0", test_spans0),
            test("testSpans1", test_spans1),
            test("testSpans2", test_spans2),
        ],
    )
}

fn before_class() -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::new(&*random()));
    let config = LuceneTestCase::new_index_writer_config_analyzer(Some(analyzer));
    config.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let writer = RandomIndexWriter::new_with_config(&*random(), directory.as_directory(), config)?;

    writer.add_document(&doc(vec![
        field("id", "0"),
        field("gender", "male"),
        field("first", "james"),
        field("last", "jones"),
    ]))?;

    writer.add_document(&doc(vec![
        field("id", "1"),
        field("gender", "male"),
        field("first", "james"),
        field("last", "smith"),
        field("gender", "female"),
        field("first", "sally"),
        field("last", "jones"),
    ]))?;

    writer.add_document(&doc(vec![
        field("id", "2"),
        field("gender", "female"),
        field("first", "greta"),
        field("last", "jones"),
        field("gender", "female"),
        field("first", "sally"),
        field("last", "smith"),
        field("gender", "male"),
        field("first", "james"),
        field("last", "jones"),
    ]))?;

    writer.add_document(&doc(vec![
        field("id", "3"),
        field("gender", "female"),
        field("first", "lisa"),
        field("last", "jones"),
        field("gender", "male"),
        field("first", "bob"),
        field("last", "costas"),
    ]))?;

    writer.add_document(&doc(vec![
        field("id", "4"),
        field("gender", "female"),
        field("first", "sally"),
        field("last", "smith"),
        field("gender", "female"),
        field("first", "linda"),
        field("last", "dixit"),
        field("gender", "male"),
        field("first", "bubba"),
        field("last", "jones"),
    ]))?;
    writer.force_merge(1)?;
    let reader = writer.get_reader()?;
    writer.close()?;
    let leaf = LuceneTestCase::get_only_leaf_reader(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    let searcher = Arc::new(IndexSearcher::new(leaf.as_index_reader())?);
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
        drop(searcher);
        reader.close()?;
        directory.close()?;
    }
    Ok(())
}

/// `check(SpanQuery q, int[] docs)`.
fn check(q: Arc<dyn SpanQuery>, docs: &[i32]) -> Result<()> {
    CheckHits::check_hit_collector(&*random(), &(q as Arc<dyn Query>), None, &searcher(), docs)
}

fn term(field: &str, text: &str) -> Term {
    Term::from_text(field, text)
}

fn stq(field: &str, text: &str) -> Arc<dyn SpanQuery> {
    Arc::new(SpanTermQuery::new(term(field, text)))
}

fn mask(q: &Arc<dyn SpanQuery>, field: &str) -> Arc<dyn SpanQuery> {
    Arc::new(FieldMaskingSpanQuery::new(Arc::clone(q), field))
}

/// `qr.visit(QueryVisitor.termCollector(terms))` and the number of terms.
fn collected_terms(qr: &dyn Query) -> usize {
    let mut terms: HashSet<Term> = HashSet::new();
    qr.visit(&mut term_collector(&mut terms));
    terms.len()
}

fn test_rewrite0() -> Result<()> {
    let q: Arc<dyn Query> = Arc::new(FieldMaskingSpanQuery::new(stq("last", "sally"), "first"));
    let qr = searcher().rewrite(Arc::clone(&q))?;

    QueryUtils::check_equal(&*q, &*qr);

    Assert::assert_equals(1i64, collected_terms(&*qr) as i64);
    Ok(())
}

/// The anonymous `SpanTermQuery` subclass of `testRewrite1`, which rewrites
/// to a `SpanOrQuery`.
#[derive(Debug, Clone)]
struct RewritingSpanTermQuery {
    inner: SpanTermQuery,
}

impl Query for RewritingSpanTermQuery {
    fn to_query_string(&self, field: &str) -> String {
        self.inner.to_query_string(field)
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        self.inner.visit(visitor);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        Ok(self.create_span_weight(searcher, score_mode, boost)?)
    }

    fn rewrite(&self, searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        Ok(self.rewrite_span(searcher)?.map(|q| q as Arc<dyn Query>))
    }

    /// `SpanTermQuery.equals`: `sameClassAs(other)` and the same term.
    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
            && other
                .as_any()
                .downcast_ref::<RewritingSpanTermQuery>()
                .is_some_and(|other| self.inner.get_term() == other.inner.get_term())
    }

    /// `SpanTermQuery.hashCode`: `classHash() ^ term.hashCode()`.
    fn query_hash(&self) -> i32 {
        self.class_hash() ^ self.inner.get_term().hash_code()
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.queries.spans.TestFieldMaskingSpanQuery$1"
    }
}

impl SpanQuery for RewritingSpanTermQuery {
    fn get_field(&self) -> Option<&str> {
        self.inner.get_field()
    }

    fn create_span_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn SpanWeight>> {
        self.inner.create_span_weight(searcher, score_mode, boost)
    }

    fn rewrite_span(&self, _searcher: &IndexSearcher) -> Result<Option<Arc<dyn SpanQuery>>> {
        Ok(Some(Arc::new(SpanOrQuery::new(vec![
            stq("first", "sally"),
            stq("first", "james"),
        ])?)))
    }
}

fn test_rewrite1() -> Result<()> {
    // mask an anon SpanQuery class that rewrites to something else.
    let q: Arc<dyn Query> = Arc::new(FieldMaskingSpanQuery::new(
        Arc::new(RewritingSpanTermQuery {
            inner: SpanTermQuery::new(term("last", "sally")),
        }),
        "first",
    ));

    let qr = searcher().rewrite(Arc::clone(&q))?;

    QueryUtils::check_unequal(&*q, &*qr);

    Assert::assert_equals(2i64, collected_terms(&*qr) as i64);
    Ok(())
}

fn test_rewrite2() -> Result<()> {
    let q1 = stq("last", "smith");
    let q2 = stq("last", "jones");
    let q: Arc<dyn Query> = Arc::new(SpanNearQuery::new(vec![q1, mask(&q2, "last")], 1, true)?);
    let qr = searcher().rewrite(Arc::clone(&q))?;

    QueryUtils::check_equal(&*q, &*qr);

    Assert::assert_equals(2i64, collected_terms(&*qr) as i64);
    Ok(())
}

fn test_equality1() -> Result<()> {
    let q1 = mask(&stq("last", "sally"), "first");
    let q2 = mask(&stq("last", "sally"), "first");
    let q3 = mask(&stq("last", "sally"), "XXXXX");
    let q4 = mask(&stq("last", "XXXXX"), "first");
    let q5 = mask(&stq("xXXX", "sally"), "first");
    QueryUtils::check_equal(&*q1, &*q2);
    QueryUtils::check_unequal(&*q1, &*q3);
    QueryUtils::check_unequal(&*q1, &*q4);
    QueryUtils::check_unequal(&*q1, &*q5);
    Ok(())
}

fn test_noop0() -> Result<()> {
    let q1 = stq("last", "sally");
    let q = mask(&q1, "first");
    check(
        q,
        &[
            /* :EMPTY: */
        ],
    )
}

fn test_noop1() -> Result<()> {
    let q1 = stq("last", "smith");
    let q2 = stq("last", "jones");
    let q: Arc<dyn SpanQuery> = Arc::new(SpanNearQuery::new(
        vec![Arc::clone(&q1), mask(&q2, "last")],
        0,
        true,
    )?);
    check(q, &[1, 2])?;
    let q: Arc<dyn SpanQuery> = Arc::new(SpanNearQuery::new(
        vec![mask(&q1, "last"), mask(&q2, "last")],
        0,
        true,
    )?);
    check(q, &[1, 2])
}

fn test_simple1() -> Result<()> {
    let q1 = stq("first", "james");
    let q2 = stq("last", "jones");
    let q: Arc<dyn SpanQuery> = Arc::new(SpanNearQuery::new(
        vec![Arc::clone(&q1), mask(&q2, "first")],
        -1,
        false,
    )?);
    check(q, &[0, 2])?;
    let q: Arc<dyn SpanQuery> = Arc::new(SpanNearQuery::new(
        vec![mask(&q2, "first"), Arc::clone(&q1)],
        -1,
        false,
    )?);
    check(q, &[0, 2])?;
    let q: Arc<dyn SpanQuery> = Arc::new(SpanNearQuery::new(
        vec![Arc::clone(&q2), mask(&q1, "last")],
        -1,
        false,
    )?);
    check(q, &[0, 2])?;
    let q: Arc<dyn SpanQuery> = Arc::new(SpanNearQuery::new(
        vec![mask(&q1, "last"), Arc::clone(&q2)],
        -1,
        false,
    )?);
    check(q, &[0, 2])
}

/// `assumeTrue("Broken scoring: LUCENE-3723", searcher.getSimilarity()
/// instanceof TFIDFSimilarity)`.
fn assume_tfidf() {
    LuceneTestCase::assume_true(
        "Broken scoring: LUCENE-3723",
        searcher().get_similarity().as_tfidf_similarity().is_some(),
    );
}

fn test_simple2() -> Result<()> {
    assume_tfidf();
    let q1 = stq("gender", "female");
    let q2 = stq("last", "smith");
    let q: Arc<dyn SpanQuery> = Arc::new(SpanNearQuery::new(
        vec![Arc::clone(&q1), mask(&q2, "gender")],
        -1,
        false,
    )?);
    check(q, &[2, 4])?;
    let q: Arc<dyn SpanQuery> = Arc::new(SpanNearQuery::new(
        vec![mask(&q1, "id"), mask(&q2, "id")],
        -1,
        false,
    )?);
    check(q, &[2, 4])
}

fn test_spans0() -> Result<()> {
    let q1 = stq("gender", "female");
    let q2 = stq("first", "james");
    let q: Arc<dyn SpanQuery> = Arc::new(SpanOrQuery::new(vec![q1, mask(&q2, "gender")])?);
    check(Arc::clone(&q), &[0, 1, 2, 3, 4])?;

    let mut span = non_null_spans(leaf_spans(&*q, &searcher(), Postings::Positions)?);
    assert_next(&mut *span, 0, 0, 1)?;
    assert_next(&mut *span, 1, 0, 1)?;
    assert_next(&mut *span, 1, 1, 2)?;
    assert_next(&mut *span, 2, 0, 1)?;
    assert_next(&mut *span, 2, 1, 2)?;
    assert_next(&mut *span, 2, 2, 3)?;
    assert_next(&mut *span, 3, 0, 1)?;
    assert_next(&mut *span, 4, 0, 1)?;
    assert_next(&mut *span, 4, 1, 2)?;
    assert_finished(Some(&mut *span))
}

fn test_spans1() -> Result<()> {
    let q1 = stq("first", "sally");
    let q2 = stq("first", "james");
    let q_a: Arc<dyn SpanQuery> = Arc::new(SpanOrQuery::new(vec![q1, q2])?);
    let q_b = mask(&q_a, "id");

    check(Arc::clone(&q_a), &[0, 1, 2, 4])?;
    check(Arc::clone(&q_b), &[0, 1, 2, 4])?;

    let mut span_a = non_null_spans(leaf_spans(&*q_a, &searcher(), Postings::Positions)?);
    let mut span_b = non_null_spans(leaf_spans(&*q_b, &searcher(), Postings::Positions)?);

    while span_a.next_doc()? != NO_MORE_DOCS {
        assert_not_same_int(
            Some("spanB not still going"),
            NO_MORE_DOCS,
            span_b.next_doc()?,
        );
        while span_a.next_start_position()? != NO_MORE_POSITIONS {
            Assert::assert_equals_msg(
                Some("spanB start position"),
                i64::from(span_a.start_position()),
                i64::from(span_b.next_start_position()?),
            );
            Assert::assert_equals_msg(
                Some("spanB end position"),
                i64::from(span_a.end_position()),
                i64::from(span_b.end_position()),
            );
        }
        Assert::assert_equals_msg(
            Some("spanB start position"),
            i64::from(NO_MORE_POSITIONS),
            i64::from(span_b.next_start_position()?),
        );
    }
    Assert::assert_equals_msg(
        Some("spanB end doc"),
        i64::from(NO_MORE_DOCS),
        i64::from(span_b.next_doc()?),
    );
    Ok(())
}

fn test_spans2() -> Result<()> {
    assume_tfidf();
    let q_a1 = stq("gender", "female");
    let q_a2 = stq("first", "james");
    let q_a: Arc<dyn SpanQuery> = Arc::new(SpanOrQuery::new(vec![q_a1, mask(&q_a2, "gender")])?);
    let q_b = stq("last", "jones");
    let q: Arc<dyn SpanQuery> = Arc::new(SpanNearQuery::new(
        vec![mask(&q_a, "id"), mask(&q_b, "id")],
        -1,
        false,
    )?);
    check(Arc::clone(&q), &[0, 1, 2, 3])?;

    let mut span = non_null_spans(leaf_spans(&*q, &searcher(), Postings::Positions)?);
    assert_next(&mut *span, 0, 0, 1)?;
    assert_next(&mut *span, 1, 1, 2)?;
    assert_next(&mut *span, 2, 0, 1)?;
    assert_next(&mut *span, 2, 2, 3)?;
    assert_next(&mut *span, 3, 0, 1)?;
    assert_finished(Some(&mut *span))
}

/// `s(int doc, int start, int end)`.
#[allow(dead_code)]
pub fn s(doc: i32, start: i32, end: i32) -> String {
    format!("s({doc},{start},{end})")
}
