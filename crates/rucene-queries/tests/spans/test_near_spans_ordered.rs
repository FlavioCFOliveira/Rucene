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

//! Port of `TestNearSpansOrdered.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::Analyzer;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::{IndexReader, Term};
use rucene::search::doc_id_set_iterator::NO_MORE_DOCS;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::Query;
use rucene::search::score_mode::ScoreMode;
use rucene_queries::spans::span_near_query::SpanNearQuery;
use rucene_queries::spans::span_or_query::SpanOrQuery;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_queries::spans::span_term_query::SpanTermQuery;
use rucene_queries::spans::span_weight::Postings;
use rucene_queries::spans::spans::Spans;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::check_hits::CheckHits;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::queries_support::core_support::random;
use crate::queries_support::search_support::{fixture_class, ftest, set_up, tear_down};
use crate::queries_support::span_test_util::{assert_finished, assert_next};
use crate::queries_support::{assert_not_same_int, first_leaf, leaf_spans, non_null_spans};

/// `TestNearSpansOrdered.FIELD`.
pub const FIELD: &str = "field";

/// The instance of `TestNearSpansOrdered`.
pub struct NearSpansOrdered {
    searcher: Option<IndexSearcher>,
    directory: Option<TestDirectory>,
    reader: Option<Arc<dyn DirectoryReader>>,
    doc_fields: Vec<&'static str>,
}

impl Default for NearSpansOrdered {
    fn default() -> Self {
        Self {
            searcher: None,
            directory: None,
            reader: None,
            doc_fields: vec![
                "w1 w2 w3 w4 w5",
                "w1 w3 w2 w3 zz",
                "w1 xx w2 yy w3",
                "w1 w3 xx w2 yy w3 zz",
                "t1 t2 t2 t1",
                "g x x g g x x x g g x x g",
                "go to webpage",
            ],
        }
    }
}

impl NearSpansOrdered {
    fn searcher(&self) -> &IndexSearcher {
        self.searcher.as_ref().expect("setUp ran")
    }

    /// `q.createWeight(searcher, COMPLETE_NO_SCORES, 1f).getSpans(leaves().get(0),
    /// POSITIONS)`, dereferenced as Java does.
    fn spans(&self, q: &dyn SpanQuery) -> Result<Box<dyn Spans>> {
        Ok(non_null_spans(leaf_spans(
            q,
            self.searcher(),
            Postings::Positions,
        )?))
    }
}

/// `TestNearSpansOrdered`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<NearSpansOrdered>(
        "org.apache.lucene.queries.spans.TestNearSpansOrdered",
        vec![
            tear_down(do_tear_down),
            set_up(do_set_up),
            ftest("testSpanNearQuery", test_span_near_query),
            ftest("testNearSpansNext", test_near_spans_next),
            ftest(
                "testNearSpansAdvanceLikeNext",
                test_near_spans_advance_like_next,
            ),
            ftest(
                "testNearSpansNextThenAdvance",
                test_near_spans_next_then_advance,
            ),
            ftest(
                "testNearSpansNextThenAdvancePast",
                test_near_spans_next_then_advance_past,
            ),
            ftest("testNearSpansAdvancePast", test_near_spans_advance_past),
            ftest("testNearSpansAdvanceTo0", test_near_spans_advance_to0),
            ftest("testNearSpansAdvanceTo1", test_near_spans_advance_to1),
            ftest("testSpanNearScorerSkipTo1", test_span_near_scorer_skip_to1),
            ftest("testOverlappedOrderedSpan", test_overlapped_ordered_span),
            ftest(
                "testOverlappedNonOrderedSpan",
                test_overlapped_non_ordered_span,
            ),
            ftest(
                "testNonOverlappedOrderedSpan",
                test_non_overlapped_ordered_span,
            ),
            ftest("testOrderedSpanIteration", test_ordered_span_iteration),
            ftest(
                "testOrderedSpanIterationSameTerms1",
                test_ordered_span_iteration_same_terms1,
            ),
            ftest(
                "testOrderedSpanIterationSameTerms2",
                test_ordered_span_iteration_same_terms2,
            ),
            ftest("testSpanNearScorerExplain", test_span_near_scorer_explain),
            ftest("testGaps", test_gaps),
            ftest("testMultipleGaps", test_multiple_gaps),
            ftest("testNestedGaps", test_nested_gaps),
        ],
    )
}

fn do_tear_down(f: &mut NearSpansOrdered) -> Result<()> {
    f.searcher = None;
    f.reader.take().expect("setUp ran").close()?;
    f.directory.take().expect("setUp ran").close()
}

fn do_set_up(f: &mut NearSpansOrdered) -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::new(&*random()));
    let config = LuceneTestCase::new_index_writer_config_analyzer(Some(analyzer));
    config.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let writer = RandomIndexWriter::new_with_config(&*random(), directory.as_directory(), config)?;
    for i in 0..f.doc_fields.len() {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_text_field(
            FIELD,
            f.doc_fields[i],
            Store::NO,
        )));
        writer.add_document(&doc)?;
    }
    writer.force_merge(1)?;
    let reader = writer.get_reader()?;
    writer.close()?;
    f.searcher = Some(LuceneTestCase::new_searcher(
        LuceneTestCase::get_only_leaf_reader(Arc::clone(&reader) as Arc<dyn IndexReader>)?,
    ));
    f.reader = Some(reader);
    f.directory = Some(directory);
    Ok(())
}

fn stq(text: &str) -> Arc<dyn SpanQuery> {
    Arc::new(SpanTermQuery::new(Term::from_text(FIELD, text)))
}

/// `makeQuery(String s1, String s2, String s3, int slop, boolean inOrder)`.
fn make_query3(
    s1: &str,
    s2: &str,
    s3: &str,
    slop: i32,
    in_order: bool,
) -> Result<Arc<dyn SpanQuery>> {
    Ok(Arc::new(SpanNearQuery::new(
        vec![stq(s1), stq(s2), stq(s3)],
        slop,
        in_order,
    )?))
}

/// `makeQuery()`.
fn make_query() -> Result<Arc<dyn SpanQuery>> {
    make_query3("w1", "w2", "w3", 1, true)
}

/// `makeOverlappedQuery(String sqt1, String sqt2, boolean sqOrdered, String
/// t3, boolean ordered)`.
fn make_overlapped_query(
    sqt1: &str,
    sqt2: &str,
    sq_ordered: bool,
    t3: &str,
    ordered: bool,
) -> Result<Arc<dyn SpanQuery>> {
    Ok(Arc::new(SpanNearQuery::new(
        vec![
            Arc::new(SpanNearQuery::new(
                vec![stq(sqt1), stq(sqt2)],
                1,
                sq_ordered,
            )?),
            stq(t3),
        ],
        0,
        ordered,
    )?))
}

fn check_hits(f: &NearSpansOrdered, q: Arc<dyn SpanQuery>, results: &[i32]) -> Result<()> {
    CheckHits::check_hits(
        &*random(),
        &(q as Arc<dyn Query>),
        Some(FIELD),
        f.searcher(),
        results,
    )
}

fn test_span_near_query(f: &mut NearSpansOrdered) -> Result<()> {
    let q = make_query()?;
    check_hits(f, q, &[0, 1])
}

/// `s(Spans span)`.
fn s_of(span: &dyn Spans) -> String {
    s(span.doc_id(), span.start_position(), span.end_position())
}

/// `s(int doc, int start, int end)`.
fn s(doc: i32, start: i32, end: i32) -> String {
    format!("s({doc},{start},{end})")
}

fn assert_long(expected: i32, actual: i32) {
    Assert::assert_equals(i64::from(expected), i64::from(actual));
}

fn test_near_spans_next(f: &mut NearSpansOrdered) -> Result<()> {
    let q = make_query()?;
    let mut span = f.spans(&*q)?;
    assert_next(&mut *span, 0, 0, 3)?;
    assert_next(&mut *span, 1, 0, 4)?;
    assert_finished(Some(&mut *span))
}

/// test does not imply that skipTo(doc+1) should work exactly the same as
/// next -- it's only applicable in this case since we know doc does not
/// contain more than one span
fn test_near_spans_advance_like_next(f: &mut NearSpansOrdered) -> Result<()> {
    let q = make_query()?;
    let mut span = f.spans(&*q)?;
    assert_long(0, span.advance(0)?);
    assert_long(0, span.next_start_position()?);
    Assert::assert_equals(s(0, 0, 3), s_of(&*span));
    assert_long(1, span.advance(1)?);
    assert_long(0, span.next_start_position()?);
    Assert::assert_equals(s(1, 0, 4), s_of(&*span));
    assert_long(NO_MORE_DOCS, span.advance(2)?);
    Ok(())
}

fn test_near_spans_next_then_advance(f: &mut NearSpansOrdered) -> Result<()> {
    let q = make_query()?;
    let mut span = f.spans(&*q)?;
    assert_not_same_int(None, NO_MORE_DOCS, span.next_doc()?);
    assert_long(0, span.next_start_position()?);
    Assert::assert_equals(s(0, 0, 3), s_of(&*span));
    assert_not_same_int(None, NO_MORE_DOCS, span.advance(1)?);
    assert_long(0, span.next_start_position()?);
    Assert::assert_equals(s(1, 0, 4), s_of(&*span));
    assert_long(NO_MORE_DOCS, span.next_doc()?);
    Ok(())
}

fn test_near_spans_next_then_advance_past(f: &mut NearSpansOrdered) -> Result<()> {
    let q = make_query()?;
    let mut span = f.spans(&*q)?;
    assert_not_same_int(None, NO_MORE_DOCS, span.next_doc()?);
    assert_long(0, span.next_start_position()?);
    Assert::assert_equals(s(0, 0, 3), s_of(&*span));
    assert_long(NO_MORE_DOCS, span.advance(2)?);
    Ok(())
}

fn test_near_spans_advance_past(f: &mut NearSpansOrdered) -> Result<()> {
    let q = make_query()?;
    let mut span = f.spans(&*q)?;
    assert_long(NO_MORE_DOCS, span.advance(2)?);
    Ok(())
}

fn test_near_spans_advance_to0(f: &mut NearSpansOrdered) -> Result<()> {
    let q = make_query()?;
    let mut span = f.spans(&*q)?;
    assert_long(0, span.advance(0)?);
    assert_long(0, span.next_start_position()?);
    Assert::assert_equals(s(0, 0, 3), s_of(&*span));
    Ok(())
}

fn test_near_spans_advance_to1(f: &mut NearSpansOrdered) -> Result<()> {
    let q = make_query()?;
    let mut span = f.spans(&*q)?;
    assert_long(1, span.advance(1)?);
    assert_long(0, span.next_start_position()?);
    Assert::assert_equals(s(1, 0, 4), s_of(&*span));
    Ok(())
}

/// not a direct test of NearSpans, but a demonstration of how/when this
/// causes problems
fn test_span_near_scorer_skip_to1(f: &mut NearSpansOrdered) -> Result<()> {
    let q = make_query()?;
    let searcher = f.searcher();
    let w = searcher.create_weight(
        searcher.rewrite(q as Arc<dyn Query>)?,
        ScoreMode::COMPLETE,
        1.0,
    )?;
    let leave = first_leaf(searcher);
    let mut s = w.scorer(&leave)?.unwrap_or_else(|| {
        randomizedtesting::jdk::exception::throw_without_message("java.lang.NullPointerException")
    });
    assert_long(1, s.iterator().advance(1)?);
    Ok(())
}

fn test_overlapped_ordered_span(f: &mut NearSpansOrdered) -> Result<()> {
    let q = make_overlapped_query("w5", "w3", false, "w4", true)?;
    check_hits(f, q, &[])
}

fn test_overlapped_non_ordered_span(f: &mut NearSpansOrdered) -> Result<()> {
    let q = make_overlapped_query("w3", "w5", true, "w4", false)?;
    check_hits(f, q, &[0])
}

fn test_non_overlapped_ordered_span(f: &mut NearSpansOrdered) -> Result<()> {
    let q = make_overlapped_query("w3", "w4", true, "w5", true)?;
    check_hits(f, q, &[0])
}

fn test_ordered_span_iteration(f: &mut NearSpansOrdered) -> Result<()> {
    let q: Arc<dyn SpanQuery> = Arc::new(SpanNearQuery::new(
        vec![
            Arc::new(SpanOrQuery::new(vec![stq("w1"), stq("w2")])?),
            stq("w4"),
        ],
        10,
        true,
    )?);
    let mut spans = f.spans(&*q)?;
    assert_next(&mut *spans, 0, 0, 4)?;
    assert_next(&mut *spans, 0, 1, 4)?;
    assert_finished(Some(&mut *spans))
}

fn test_ordered_span_iteration_same_terms1(f: &mut NearSpansOrdered) -> Result<()> {
    let q: Arc<dyn SpanQuery> = Arc::new(SpanNearQuery::new(vec![stq("t1"), stq("t2")], 1, true)?);
    let mut spans = f.spans(&*q)?;
    assert_next(&mut *spans, 4, 0, 2)?;
    assert_finished(Some(&mut *spans))
}

fn test_ordered_span_iteration_same_terms2(f: &mut NearSpansOrdered) -> Result<()> {
    let q: Arc<dyn SpanQuery> = Arc::new(SpanNearQuery::new(vec![stq("t2"), stq("t1")], 1, true)?);
    let mut spans = f.spans(&*q)?;
    assert_next(&mut *spans, 4, 1, 4)?;
    assert_next(&mut *spans, 4, 2, 4)?;
    assert_finished(Some(&mut *spans))
}

/// not a direct test of NearSpans, but a demonstration of how/when this
/// causes problems
fn test_span_near_scorer_explain(f: &mut NearSpansOrdered) -> Result<()> {
    let q = make_query()?;
    let e = f.searcher().explain(q as Arc<dyn Query>, 1)?;
    Assert::assert_true_msg(
        &format!("Scorer explanation value for doc#1 isn't positive: {e}"),
        0.0 <= e.value().double_value(),
    );
    Ok(())
}

fn test_gaps(f: &mut NearSpansOrdered) -> Result<()> {
    let mut b = SpanNearQuery::new_ordered_near_query(FIELD);
    b.add_clause(stq("w1"))?.add_gap(1)?.add_clause(stq("w2"))?;
    let q: Arc<dyn SpanQuery> = Arc::new(b.build()?);
    let mut spans = f.spans(&*q)?;
    assert_next(&mut *spans, 1, 0, 3)?;
    assert_next(&mut *spans, 2, 0, 3)?;
    assert_finished(Some(&mut *spans))?;

    let mut b = SpanNearQuery::new_ordered_near_query(FIELD);
    b.add_clause(stq("w1"))?
        .add_gap(1)?
        .add_clause(stq("w2"))?
        .add_gap(1)?
        .add_clause(stq("w3"))?
        .set_slop(1);
    let q: Arc<dyn SpanQuery> = Arc::new(b.build()?);
    let mut spans = f.spans(&*q)?;
    assert_next(&mut *spans, 2, 0, 5)?;
    assert_next(&mut *spans, 3, 0, 6)?;
    assert_finished(Some(&mut *spans))
}

fn test_multiple_gaps(f: &mut NearSpansOrdered) -> Result<()> {
    let mut b = SpanNearQuery::new_ordered_near_query(FIELD);
    b.add_clause(stq("g"))?.add_gap(2)?.add_clause(stq("g"))?;
    let q: Arc<dyn SpanQuery> = Arc::new(b.build()?);
    let mut spans = f.spans(&*q)?;
    assert_next(&mut *spans, 5, 0, 4)?;
    assert_next(&mut *spans, 5, 9, 13)?;
    assert_finished(Some(&mut *spans))
}

fn test_nested_gaps(f: &mut NearSpansOrdered) -> Result<()> {
    let mut inner = SpanNearQuery::new_ordered_near_query(FIELD);
    inner.add_clause(stq("go"))?.add_gap(1)?;
    let mut b = SpanNearQuery::new_ordered_near_query(FIELD);
    b.add_clause(Arc::new(SpanOrQuery::new(vec![
        stq("open"),
        Arc::new(inner.build()?),
    ])?))?
    .add_clause(stq("webpage"))?;
    let q: Arc<dyn Query> = Arc::new(b.build()?);
    let top_docs = f.searcher().search(q, 1)?;
    assert_long(6, top_docs.score_docs[0].doc);
    Ok(())
}
