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

//! Port of `TestSpanContainQuery.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::Analyzer;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::IndexReader;
use rucene::search::doc_id_set_iterator::NO_MORE_DOCS;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::Query;
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
    assert_finished, assert_next, span_containing_query, span_near_ordered_query_terms,
    span_or_query_terms, span_term_query, span_within_query,
};
use crate::queries_support::{leaf_spans, non_null_spans};

/// `TestSpanContainQuery.field`.
const FIELD: &str = "field";

/// The instance of `TestSpanContainQuery`.
pub struct SpanContainQuery {
    searcher: Option<IndexSearcher>,
    reader: Option<Arc<dyn DirectoryReader>>,
    directory: Option<TestDirectory>,
    doc_fields: Vec<&'static str>,
}

impl Default for SpanContainQuery {
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
            ],
        }
    }
}

impl SpanContainQuery {
    fn searcher(&self) -> &IndexSearcher {
        self.searcher.as_ref().expect("setUp ran")
    }

    /// `checkHits(Query query, int[] results)` (declared by the Java class,
    /// called by none of its tests).
    #[allow(dead_code)]
    fn check_hits(&self, query: Arc<dyn Query>, results: &[i32]) -> Result<()> {
        CheckHits::check_hits(&*random(), &query, Some(FIELD), self.searcher(), results)
    }

    /// `makeSpans(SpanQuery sq)`.
    fn make_spans(&self, sq: &dyn SpanQuery) -> Result<Option<Box<dyn Spans>>> {
        leaf_spans(sq, self.searcher(), Postings::Positions)
    }

    /// `tstEqualSpans(String mes, SpanQuery expectedQ, SpanQuery actualQ)`.
    fn tst_equal_spans_q(
        &self,
        mes: &str,
        expected_q: &dyn SpanQuery,
        actual_q: &dyn SpanQuery,
    ) -> Result<()> {
        let mut expected = non_null_spans(self.make_spans(expected_q)?);
        let mut actual = non_null_spans(self.make_spans(actual_q)?);
        tst_equal_spans(mes, &mut *expected, &mut *actual)
    }
}

/// `tstEqualSpans(String mes, Spans expected, Spans actual)`.
fn tst_equal_spans(_mes: &str, expected: &mut dyn Spans, actual: &mut dyn Spans) -> Result<()> {
    while expected.next_doc()? != NO_MORE_DOCS {
        Assert::assert_equals(i64::from(expected.doc_id()), i64::from(actual.next_doc()?));
        Assert::assert_equals(i64::from(expected.doc_id()), i64::from(actual.doc_id()));
        while expected.next_start_position()? != NO_MORE_POSITIONS {
            Assert::assert_equals(
                i64::from(expected.start_position()),
                i64::from(actual.next_start_position()?),
            );
            Assert::assert_equals_msg(
                Some("start"),
                i64::from(expected.start_position()),
                i64::from(actual.start_position()),
            );
            Assert::assert_equals_msg(
                Some("end"),
                i64::from(expected.end_position()),
                i64::from(actual.end_position()),
            );
        }
    }
    Ok(())
}

/// `TestSpanContainQuery`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<SpanContainQuery>(
        "org.apache.lucene.queries.spans.TestSpanContainQuery",
        vec![
            set_up(do_set_up),
            tear_down(do_tear_down),
            ftest("testSpanContainTerm", test_span_contain_term),
            ftest(
                "testSpanContainPhraseBothWords",
                test_span_contain_phrase_both_words,
            ),
            ftest(
                "testSpanContainPhraseFirstWord",
                test_span_contain_phrase_first_word,
            ),
            ftest(
                "testSpanContainPhraseSecondWord",
                test_span_contain_phrase_second_word,
            ),
        ],
    )
}

fn do_set_up(f: &mut SpanContainQuery) -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::new(&*random()));
    let config = LuceneTestCase::new_index_writer_config_analyzer(Some(analyzer));
    config.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let writer = RandomIndexWriter::new_with_config(&*random(), directory.as_directory(), config)?;
    for i in 0..f.doc_fields.len() {
        let mut doc = Document::new();
        doc.add(LuceneTestCase::new_text_field(
            FIELD,
            f.doc_fields[i],
            Store::YES,
        ));
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

fn do_tear_down(f: &mut SpanContainQuery) -> Result<()> {
    f.searcher = None;
    f.reader.take().expect("setUp ran").close()?;
    f.directory.take().expect("setUp ran").close()
}

fn test_span_contain_term(f: &mut SpanContainQuery) -> Result<()> {
    let stq = span_term_query(FIELD, "w3");
    let containing_q = span_containing_query(Arc::clone(&stq), Arc::clone(&stq))?;
    let contained_q = span_within_query(Arc::clone(&stq), Arc::clone(&stq))?;
    f.tst_equal_spans_q("containing", &*stq, &*containing_q)?;
    f.tst_equal_spans_q("containing", &*stq, &*contained_q)
}

fn test_span_contain_phrase_both_words(f: &mut SpanContainQuery) -> Result<()> {
    let w2 = "w2";
    let w3 = "w3";
    let phrase_q = span_near_ordered_query_terms(FIELD, 0, &[w2, w3])?;
    let w23 = span_or_query_terms(FIELD, &[w2, w3])?;
    let containing_phrase_or = span_containing_query(Arc::clone(&phrase_q), Arc::clone(&w23))?;
    let contained_phrase_or = span_within_query(phrase_q.clone(), w23)?;
    f.tst_equal_spans_q("containing phrase or", &*phrase_q, &*containing_phrase_or)?;
    let mut spans = non_null_spans(f.make_spans(&*contained_phrase_or)?);
    assert_next(&mut *spans, 0, 1, 2)?;
    assert_next(&mut *spans, 0, 2, 3)?;
    assert_next(&mut *spans, 1, 2, 3)?;
    assert_next(&mut *spans, 1, 3, 4)?;
    assert_finished(Some(&mut *spans))
}

fn test_span_contain_phrase_first_word(f: &mut SpanContainQuery) -> Result<()> {
    let w2 = "w2";
    let w3 = "w3";
    let stqw2 = span_term_query(FIELD, w2);
    let phrase_q = span_near_ordered_query_terms(FIELD, 0, &[w2, w3])?;
    let containing_phrase_w2 = span_containing_query(Arc::clone(&phrase_q), Arc::clone(&stqw2))?;
    let contained_phrase_w2 = span_within_query(Arc::clone(&phrase_q), stqw2)?;
    f.tst_equal_spans_q("containing phrase w2", &*phrase_q, &*containing_phrase_w2)?;
    let mut spans = non_null_spans(f.make_spans(&*contained_phrase_w2)?);
    assert_next(&mut *spans, 0, 1, 2)?;
    assert_next(&mut *spans, 1, 2, 3)?;
    assert_finished(Some(&mut *spans))
}

fn test_span_contain_phrase_second_word(f: &mut SpanContainQuery) -> Result<()> {
    let w2 = "w2";
    let w3 = "w3";
    let stqw3 = span_term_query(FIELD, w3);
    let phrase_q = span_near_ordered_query_terms(FIELD, 0, &[w2, w3])?;
    let containing_phrase_w3 = span_containing_query(Arc::clone(&phrase_q), Arc::clone(&stqw3))?;
    let contained_phrase_w3 = span_within_query(Arc::clone(&phrase_q), stqw3)?;
    f.tst_equal_spans_q("containing phrase w3", &*phrase_q, &*containing_phrase_w3)?;
    let mut spans = non_null_spans(f.make_spans(&*contained_phrase_w3)?);
    assert_next(&mut *spans, 0, 2, 3)?;
    assert_next(&mut *spans, 1, 3, 4)?;
    assert_finished(Some(&mut *spans))
}
