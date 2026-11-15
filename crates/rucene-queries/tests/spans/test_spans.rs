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

//! Port of `TestSpans.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::Analyzer;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::{IndexReader, Term};
use rucene::search::doc_id_set_iterator::NO_MORE_DOCS;
use rucene::search::fuzzy_query::FuzzyQuery;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::prefix_query::PrefixQuery;
use rucene::search::query::Query;
use rucene::search::term_query::TermQuery;
use rucene_queries::spans::span_multi_term_query_wrapper::SpanMultiTermQueryWrapper;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_queries::spans::span_weight::Postings;
use rucene_queries::spans::spans::{Spans, NO_MORE_POSITIONS};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::check_hits::CheckHits;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::queries_support::core_support::random;
use crate::queries_support::search_support::{fixture_class, ftest, set_up, tear_down};
use crate::queries_support::span_test_util::{
    self, assert_finished, assert_next, span_near_ordered_query_terms,
    span_near_unordered_query_terms, span_not_query_pre_post, span_or_query_terms, span_term_query,
};
use crate::queries_support::{leaf_spans, non_null_spans};

/// `TestSpans.field`.
pub const FIELD: &str = "field";

/// The instance of `TestSpans`.
pub struct SpansTest {
    searcher: Option<IndexSearcher>,
    reader: Option<Arc<dyn DirectoryReader>>,
    directory: Option<TestDirectory>,
    doc_fields: Vec<&'static str>,
}

impl Default for SpansTest {
    fn default() -> Self {
        Self {
            searcher: None,
            reader: None,
            directory: None,
            doc_fields: vec![
                "w1 w2 w3 w4 w5",
                "w1 w3 w2 w3",
                "w1 xx w2 yy w3",
                "w1 w3 xx w2 yy w3",
                "u2 u2 u1",
                "u2 xx u2 u1",
                "u2 u2 xx u1",
                "u2 xx u2 yy u1",
                "u2 xx u1 u2",
                "u2 u1 xx u2",
                "u1 u2 xx u2",
                "t1 t2 t1 t3 t2 t3",
                "s2 s1 s1 xx xx s2 xx s2 xx s1 xx xx xx xx xx s2 xx",
                "r1 s11",
                "r1 s21",
            ],
        }
    }
}

impl SpansTest {
    fn searcher(&self) -> &IndexSearcher {
        self.searcher.as_ref().expect("setUp ran")
    }

    /// `checkHits(Query query, int[] results)`.
    fn check_hits(&self, query: Arc<dyn SpanQuery>, results: &[i32]) -> Result<()> {
        CheckHits::check_hits(
            &*random(),
            &(query as Arc<dyn Query>),
            Some(FIELD),
            self.searcher(),
            results,
        )
    }

    /// `orderedSlopTest3SQ(SpanQuery q1, SpanQuery q2, SpanQuery q3, int slop,
    /// int[] expectedDocs)`.
    fn ordered_slop_test3_sq(
        &self,
        q1: Arc<dyn SpanQuery>,
        q2: Arc<dyn SpanQuery>,
        q3: Arc<dyn SpanQuery>,
        slop: i32,
        expected_docs: &[i32],
    ) -> Result<()> {
        let query = span_test_util::span_near_ordered_query(slop, vec![q1, q2, q3])?;
        self.check_hits(query, expected_docs)
    }

    /// `orderedSlopTest3(int slop, int[] expectedDocs)`.
    pub fn ordered_slop_test3(&self, slop: i32, expected_docs: &[i32]) -> Result<()> {
        self.ordered_slop_test3_sq(
            span_term_query(FIELD, "w1"),
            span_term_query(FIELD, "w2"),
            span_term_query(FIELD, "w3"),
            slop,
            expected_docs,
        )
    }

    /// `orderedSlopTest3Equal(int slop, int[] expectedDocs)`.
    pub fn ordered_slop_test3_equal(&self, slop: i32, expected_docs: &[i32]) -> Result<()> {
        self.ordered_slop_test3_sq(
            span_term_query(FIELD, "w1"),
            span_term_query(FIELD, "w3"),
            span_term_query(FIELD, "w3"),
            slop,
            expected_docs,
        )
    }

    /// `orderedSlopTest1Equal(int slop, int[] expectedDocs)`.
    pub fn ordered_slop_test1_equal(&self, slop: i32, expected_docs: &[i32]) -> Result<()> {
        self.ordered_slop_test3_sq(
            span_term_query(FIELD, "u2"),
            span_term_query(FIELD, "u2"),
            span_term_query(FIELD, "u1"),
            slop,
            expected_docs,
        )
    }

    /// `orSpans(String[] terms)`.
    fn or_spans(&self, terms: &[&str]) -> Result<Option<Box<dyn Spans>>> {
        let q = span_or_query_terms(FIELD, terms)?;
        leaf_spans(&*q, self.searcher(), Postings::Positions)
    }

    /// `spanCount(String include, int slop, String exclude, int pre, int
    /// post)`.
    fn span_count(
        &self,
        include: &str,
        slop: i32,
        exclude: &str,
        pre: i32,
        post: i32,
    ) -> Result<i32> {
        let include_terms: Vec<&str> = include.split(' ').filter(|s| !s.is_empty()).collect();
        let iq = if include_terms.len() == 1 {
            span_term_query(FIELD, include)
        } else {
            span_near_ordered_query_terms(FIELD, slop, &include_terms)?
        };
        let eq = span_term_query(FIELD, exclude);
        let snq = span_not_query_pre_post(iq, eq, pre, post)?;
        let spans = leaf_spans(&*snq, self.searcher(), Postings::Positions)?;

        let mut i = 0;
        if let Some(mut spans) = spans {
            while spans.next_doc()? != NO_MORE_DOCS {
                while spans.next_start_position()? != NO_MORE_POSITIONS {
                    i += 1;
                }
            }
        }
        Ok(i)
    }
}

/// `TestSpans`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<SpansTest>(
        "org.apache.lucene.queries.spans.TestSpans",
        vec![
            set_up(do_set_up),
            tear_down(do_tear_down),
            ftest("testSpanNearOrdered01", test_span_near_ordered01),
            ftest("testSpanNearOrdered02", test_span_near_ordered02),
            ftest("testSpanNearOrdered03", test_span_near_ordered03),
            ftest("testSpanNearOrdered04", test_span_near_ordered04),
            ftest("testSpanNearOrdered05", test_span_near_ordered05),
            ftest("testSpanNearOrderedEqual01", test_span_near_ordered_equal01),
            ftest("testSpanNearOrderedEqual02", test_span_near_ordered_equal02),
            ftest("testSpanNearOrderedEqual03", test_span_near_ordered_equal03),
            ftest("testSpanNearOrderedEqual04", test_span_near_ordered_equal04),
            ftest("testSpanNearOrderedEqual11", test_span_near_ordered_equal11),
            ftest("testSpanNearOrderedEqual12", test_span_near_ordered_equal12),
            ftest("testSpanNearOrderedEqual13", test_span_near_ordered_equal13),
            ftest("testSpanNearOrderedEqual14", test_span_near_ordered_equal14),
            ftest("testSpanNearOrderedEqual15", test_span_near_ordered_equal15),
            ftest("testSpanNearOrderedOverlap", test_span_near_ordered_overlap),
            ftest("testSpanNearUnOrdered", test_span_near_un_ordered),
            ftest("testSpanOrEmpty", test_span_or_empty),
            ftest("testSpanOrSingle", test_span_or_single),
            ftest("testSpanOrDouble", test_span_or_double),
            ftest("testSpanOrDoubleAdvance", test_span_or_double_advance),
            ftest("testSpanOrUnused", test_span_or_unused),
            ftest("testSpanOrTripleSameDoc", test_span_or_triple_same_doc),
            ftest("testNPESpanQuery", test_npe_span_query),
            ftest("testSpanNotWithMultiterm", test_span_not_with_multiterm),
            ftest("testSpanNots", test_span_nots),
        ],
    )
}

fn do_set_up(f: &mut SpansTest) -> Result<()> {
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
            Store::YES,
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

fn do_tear_down(f: &mut SpansTest) -> Result<()> {
    f.searcher = None;
    f.reader.take().expect("setUp ran").close()?;
    f.directory.take().expect("setUp ran").close()
}

fn test_span_near_ordered01(f: &mut SpansTest) -> Result<()> {
    f.ordered_slop_test3(0, &[0])
}

fn test_span_near_ordered02(f: &mut SpansTest) -> Result<()> {
    f.ordered_slop_test3(1, &[0, 1])
}

fn test_span_near_ordered03(f: &mut SpansTest) -> Result<()> {
    f.ordered_slop_test3(2, &[0, 1, 2])
}

fn test_span_near_ordered04(f: &mut SpansTest) -> Result<()> {
    f.ordered_slop_test3(3, &[0, 1, 2, 3])
}

fn test_span_near_ordered05(f: &mut SpansTest) -> Result<()> {
    f.ordered_slop_test3(4, &[0, 1, 2, 3])
}

fn test_span_near_ordered_equal01(f: &mut SpansTest) -> Result<()> {
    f.ordered_slop_test3_equal(0, &[])
}

fn test_span_near_ordered_equal02(f: &mut SpansTest) -> Result<()> {
    f.ordered_slop_test3_equal(1, &[1])
}

fn test_span_near_ordered_equal03(f: &mut SpansTest) -> Result<()> {
    f.ordered_slop_test3_equal(2, &[1])
}

fn test_span_near_ordered_equal04(f: &mut SpansTest) -> Result<()> {
    f.ordered_slop_test3_equal(3, &[1, 3])
}

fn test_span_near_ordered_equal11(f: &mut SpansTest) -> Result<()> {
    f.ordered_slop_test1_equal(0, &[4])
}

fn test_span_near_ordered_equal12(f: &mut SpansTest) -> Result<()> {
    f.ordered_slop_test1_equal(0, &[4])
}

fn test_span_near_ordered_equal13(f: &mut SpansTest) -> Result<()> {
    f.ordered_slop_test1_equal(1, &[4, 5, 6])
}

fn test_span_near_ordered_equal14(f: &mut SpansTest) -> Result<()> {
    f.ordered_slop_test1_equal(2, &[4, 5, 6, 7])
}

fn test_span_near_ordered_equal15(f: &mut SpansTest) -> Result<()> {
    f.ordered_slop_test1_equal(3, &[4, 5, 6, 7])
}

fn assert_long_msg(message: &str, expected: i32, actual: i32) {
    Assert::assert_equals_msg(Some(message), i64::from(expected), i64::from(actual));
}

fn test_span_near_ordered_overlap(f: &mut SpansTest) -> Result<()> {
    let query = span_near_ordered_query_terms(FIELD, 1, &["t1", "t2", "t3"])?;

    let mut spans = non_null_spans(leaf_spans(&*query, f.searcher(), Postings::Positions)?);

    assert_long_msg("first doc", 11, spans.next_doc()?);
    assert_long_msg("first start", 0, spans.next_start_position()?);
    assert_long_msg("first end", 4, spans.end_position());

    assert_long_msg("second start", 2, spans.next_start_position()?);
    assert_long_msg("second end", 6, spans.end_position());

    assert_finished(Some(&mut *spans))
}

fn test_span_near_un_ordered(f: &mut SpansTest) -> Result<()> {
    // See http://www.gossamer-threads.com/lists/lucene/java-dev/52270 for discussion about this
    // test
    let senq = span_near_unordered_query_terms(FIELD, 0, &["u1", "u2"])?;
    let mut spans = non_null_spans(leaf_spans(&*senq, f.searcher(), Postings::Positions)?);
    assert_next(&mut *spans, 4, 1, 3)?;
    assert_next(&mut *spans, 5, 2, 4)?;
    assert_next(&mut *spans, 8, 2, 4)?;
    assert_next(&mut *spans, 9, 0, 2)?;
    assert_next(&mut *spans, 10, 0, 2)?;
    assert_finished(Some(&mut *spans))?;

    let senq =
        span_test_util::span_near_unordered_query(1, vec![senq, span_term_query(FIELD, "u2")])?;
    let mut spans = non_null_spans(leaf_spans(&*senq, f.searcher(), Postings::Positions)?);
    assert_next(&mut *spans, 4, 0, 3)?;
    assert_next(&mut *spans, 4, 1, 3)?; // unordered spans can be subsets
    assert_next(&mut *spans, 5, 0, 4)?;
    assert_next(&mut *spans, 5, 2, 4)?;
    assert_next(&mut *spans, 8, 0, 4)?;
    assert_next(&mut *spans, 8, 2, 4)?;
    assert_next(&mut *spans, 9, 0, 2)?;
    assert_next(&mut *spans, 9, 0, 4)?;
    assert_next(&mut *spans, 10, 0, 2)?;
    assert_finished(Some(&mut *spans))
}

fn test_span_or_empty(f: &mut SpansTest) -> Result<()> {
    match f.or_spans(&[])? {
        Some(mut spans) => assert_finished(Some(&mut *spans)),
        None => assert_finished(None),
    }
}

fn test_span_or_single(f: &mut SpansTest) -> Result<()> {
    let mut spans = non_null_spans(f.or_spans(&["w5"])?);
    assert_next(&mut *spans, 0, 4, 5)?;
    assert_finished(Some(&mut *spans))
}

fn test_span_or_double(f: &mut SpansTest) -> Result<()> {
    let mut spans = non_null_spans(f.or_spans(&["w5", "yy"])?);
    assert_next(&mut *spans, 0, 4, 5)?;
    assert_next(&mut *spans, 2, 3, 4)?;
    assert_next(&mut *spans, 3, 4, 5)?;
    assert_next(&mut *spans, 7, 3, 4)?;
    assert_finished(Some(&mut *spans))
}

fn test_span_or_double_advance(f: &mut SpansTest) -> Result<()> {
    let mut spans = non_null_spans(f.or_spans(&["w5", "yy"])?);
    assert_long_msg("initial advance", 3, spans.advance(3)?);
    assert_next(&mut *spans, 3, 4, 5)?;
    assert_next(&mut *spans, 7, 3, 4)?;
    assert_finished(Some(&mut *spans))
}

fn test_span_or_unused(f: &mut SpansTest) -> Result<()> {
    let mut spans = non_null_spans(f.or_spans(&["w5", "unusedTerm", "yy"])?);
    assert_next(&mut *spans, 0, 4, 5)?;
    assert_next(&mut *spans, 2, 3, 4)?;
    assert_next(&mut *spans, 3, 4, 5)?;
    assert_next(&mut *spans, 7, 3, 4)?;
    assert_finished(Some(&mut *spans))
}

fn test_span_or_triple_same_doc(f: &mut SpansTest) -> Result<()> {
    let mut spans = non_null_spans(f.or_spans(&["t1", "t2", "t3"])?);
    assert_next(&mut *spans, 11, 0, 1)?;
    assert_next(&mut *spans, 11, 1, 2)?;
    assert_next(&mut *spans, 11, 2, 3)?;
    assert_next(&mut *spans, 11, 3, 4)?;
    assert_next(&mut *spans, 11, 4, 5)?;
    assert_next(&mut *spans, 11, 5, 6)?;
    assert_finished(Some(&mut *spans))
}

// LUCENE-1404
fn add_doc(writer: &IndexWriter, id: &str, text: &str) -> Result<()> {
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_string_field(
        "id",
        id,
        Store::YES,
    )));
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "text",
        text,
        Store::YES,
    )));
    writer.add_document(&doc)?;
    Ok(())
}

// LUCENE-1404
fn hit_count(searcher: &IndexSearcher, word: &str) -> Result<i64> {
    Ok(i64::from(searcher.count(Arc::new(TermQuery::new(
        Term::from_text("text", word),
    )))?))
}

// LUCENE-1404
fn create_span(value: &str) -> Arc<dyn SpanQuery> {
    span_term_query("text", value)
}

// LUCENE-1404
fn create_span_clauses(
    slop: i32,
    ordered: bool,
    clauses: Vec<Arc<dyn SpanQuery>>,
) -> Result<Arc<dyn SpanQuery>> {
    if ordered {
        span_test_util::span_near_ordered_query(slop, clauses)
    } else {
        span_test_util::span_near_unordered_query(slop, clauses)
    }
}

// LUCENE-1404
fn create_span_terms(
    slop: i32,
    ordered: bool,
    term1: &str,
    term2: &str,
) -> Result<Arc<dyn SpanQuery>> {
    create_span_clauses(slop, ordered, vec![create_span(term1), create_span(term2)])
}

// LUCENE-1404
fn test_npe_span_query(_f: &mut SpansTest) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::new(&*random()));
    let writer = IndexWriter::new(
        dir.as_directory(),
        IndexWriterConfig::with_analyzer(Some(analyzer)),
    )?;

    // Add documents
    add_doc(&writer, "1", "the big dogs went running to the market")?;
    add_doc(
        &writer,
        "2",
        "the cat chased the mouse, then the cat ate the mouse quickly",
    )?;

    // Commit
    writer.close()?;

    // Get searcher
    let reader = directory_reader::open(dir.as_directory())?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    // Control (make sure docs indexed)
    Assert::assert_equals(2i64, hit_count(&searcher, "the")?);
    Assert::assert_equals(1i64, hit_count(&searcher, "cat")?);
    Assert::assert_equals(1i64, hit_count(&searcher, "dogs")?);
    Assert::assert_equals(0i64, hit_count(&searcher, "rabbit")?);

    // This throws exception (it shouldn't)
    let q = create_span_clauses(
        0,
        true,
        vec![
            create_span_terms(4, false, "chased", "cat")?,
            create_span("ate"),
        ],
    )?;
    Assert::assert_equals(
        1i64,
        searcher.search(q as Arc<dyn Query>, 10)?.total_hits.value(),
    );
    reader.close()?;
    dir.close()
}

fn test_span_not_with_multiterm(f: &mut SpansTest) -> Result<()> {
    let q = span_not_query_pre_post(
        span_term_query(FIELD, "r1"),
        Arc::new(SpanMultiTermQueryWrapper::new(Arc::new(PrefixQuery::new(
            Term::from_text(FIELD, "s1"),
        )?))),
        3,
        3,
    )?;
    f.check_hits(q, &[14])?;

    let q = span_not_query_pre_post(
        span_term_query(FIELD, "r1"),
        Arc::new(SpanMultiTermQueryWrapper::new(Arc::new(
            FuzzyQuery::with_prefix_length(Term::from_text(FIELD, "s12"), 1, 2)?,
        ))),
        3,
        3,
    )?;
    f.check_hits(q, &[14])?;

    let q = span_not_query_pre_post(
        Arc::new(SpanMultiTermQueryWrapper::new(Arc::new(PrefixQuery::new(
            Term::from_text(FIELD, "r"),
        )?))),
        span_term_query(FIELD, "s21"),
        3,
        3,
    )?;
    f.check_hits(q, &[13])
}

/// `assertEquals(String message, double expected, double actual, double
/// delta)` over the `int` count, widened.
fn assert_double(message: &str, expected: i32, actual: i32, delta: f64) {
    Assert::assert_equals_double_msg(Some(message), f64::from(expected), f64::from(actual), delta);
}

fn test_span_nots(f: &mut SpansTest) -> Result<()> {
    assert_double(
        "SpanNotIncludeExcludeSame1",
        0,
        f.span_count("s2", 0, "s2", 0, 0)?,
        0.0,
    );
    assert_double(
        "SpanNotIncludeExcludeSame2",
        0,
        f.span_count("s2", 0, "s2", 10, 10)?,
        0.0,
    );

    // focus on behind
    assert_long_msg("SpanNotS2NotS1_6_0", 1, f.span_count("s2", 0, "s1", 6, 0)?);
    assert_long_msg("SpanNotS2NotS1_5_0", 2, f.span_count("s2", 0, "s1", 5, 0)?);
    assert_long_msg("SpanNotS2NotS1_3_0", 3, f.span_count("s2", 0, "s1", 3, 0)?);
    assert_long_msg("SpanNotS2NotS1_2_0", 4, f.span_count("s2", 0, "s1", 2, 0)?);
    assert_long_msg("SpanNotS2NotS1_0_0", 4, f.span_count("s2", 0, "s1", 0, 0)?);

    // focus on both
    assert_long_msg("SpanNotS2NotS1_3_1", 2, f.span_count("s2", 0, "s1", 3, 1)?);
    assert_long_msg("SpanNotS2NotS1_2_1", 3, f.span_count("s2", 0, "s1", 2, 1)?);
    assert_long_msg("SpanNotS2NotS1_1_1", 3, f.span_count("s2", 0, "s1", 1, 1)?);
    assert_long_msg(
        "SpanNotS2NotS1_10_10",
        0,
        f.span_count("s2", 0, "s1", 10, 10)?,
    );

    // focus on ahead
    assert_long_msg(
        "SpanNotS1NotS2_10_10",
        0,
        f.span_count("s1", 0, "s2", 10, 10)?,
    );
    assert_long_msg("SpanNotS1NotS2_0_1", 3, f.span_count("s1", 0, "s2", 0, 1)?);
    assert_long_msg("SpanNotS1NotS2_0_2", 3, f.span_count("s1", 0, "s2", 0, 2)?);
    assert_long_msg("SpanNotS1NotS2_0_3", 2, f.span_count("s1", 0, "s2", 0, 3)?);
    assert_long_msg("SpanNotS1NotS2_0_4", 1, f.span_count("s1", 0, "s2", 0, 4)?);
    assert_long_msg("SpanNotS1NotS2_0_8", 0, f.span_count("s1", 0, "s2", 0, 8)?);

    // exclude doesn't exist
    assert_long_msg("SpanNotS1NotS3_8_8", 3, f.span_count("s1", 0, "s3", 8, 8)?);

    // include doesn't exist
    assert_long_msg("SpanNotS3NotS1_8_8", 0, f.span_count("s3", 0, "s1", 8, 8)?);

    // Negative values
    assert_long_msg(
        "SpanNotS2S1NotXXNeg_0_0",
        1,
        f.span_count("s2 s1", 10, "xx", 0, 0)?,
    );
    assert_long_msg(
        "SpanNotS2S1NotXXNeg_1_1",
        1,
        f.span_count("s2 s1", 10, "xx", -1, -1)?,
    );
    assert_long_msg(
        "SpanNotS2S1NotXXNeg_0_2",
        2,
        f.span_count("s2 s1", 10, "xx", 0, -2)?,
    );
    assert_long_msg(
        "SpanNotS2S1NotXXNeg_1_2",
        2,
        f.span_count("s2 s1", 10, "xx", -1, -2)?,
    );
    assert_long_msg(
        "SpanNotS2S1NotXXNeg_2_1",
        2,
        f.span_count("s2 s1", 10, "xx", -2, -1)?,
    );
    assert_long_msg(
        "SpanNotS2S1NotXXNeg_3_1",
        2,
        f.span_count("s2 s1", 10, "xx", -3, -1)?,
    );
    assert_long_msg(
        "SpanNotS2S1NotXXNeg_1_3",
        2,
        f.span_count("s2 s1", 10, "xx", -1, -3)?,
    );
    assert_long_msg(
        "SpanNotS2S1NotXXNeg_2_2",
        3,
        f.span_count("s2 s1", 10, "xx", -2, -2)?,
    );
    Ok(())
}
