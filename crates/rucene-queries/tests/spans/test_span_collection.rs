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

//! Port of `TestSpanCollection.java`.

use std::collections::HashSet;
use std::sync::{Arc, LazyLock};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::Analyzer;
use rucene::document::{Document, FieldType, TextField};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::{IndexOptions, IndexReader, PostingsEnum, Term};
use rucene::search::index_searcher::IndexSearcher;
use rucene_queries::spans::span_collector::SpanCollector;
use rucene_queries::spans::span_near_query::SpanNearQuery;
use rucene_queries::spans::span_not_query::SpanNotQuery;
use rucene_queries::spans::span_or_query::SpanOrQuery;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_queries::spans::span_term_query::SpanTermQuery;
use rucene_queries::spans::span_weight::Postings;
use rucene_queries::spans::spans::Spans;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::queries_support::core_support::random;
use crate::queries_support::search_support::{fixture_class, ftest, set_up, tear_down};
use crate::queries_support::{leaf_spans, non_null_spans};

/// `TestSpanCollection.FIELD`.
pub const FIELD: &str = "field";

/// `TestSpanCollection.OFFSETS`: `TextField.TYPE_STORED` indexed with
/// offsets (the static initializer).
pub static OFFSETS: LazyLock<FieldType> = LazyLock::new(|| {
    let offsets = FieldType::new_from(TextField::type_stored());
    offsets
        .set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS)
        .expect("INVARIANT: the field type is not frozen");
    offsets
});

/// The instance of `TestSpanCollection`.
pub struct SpanCollection {
    searcher: Option<IndexSearcher>,
    directory: Option<TestDirectory>,
    reader: Option<Arc<dyn DirectoryReader>>,
    doc_fields: Vec<&'static str>,
}

impl Default for SpanCollection {
    fn default() -> Self {
        Self {
            searcher: None,
            directory: None,
            reader: None,
            doc_fields: vec![
                "w1 w2 w3 w4 w5",
                "w1 w3 w2 w3 zz",
                "w1 xx w2 yy w4",
                "w1 w2 w1 w4 w2 w3",
            ],
        }
    }
}

impl SpanCollection {
    fn spans(&self, q: &dyn SpanQuery) -> Result<Box<dyn Spans>> {
        Ok(non_null_spans(leaf_spans(
            q,
            self.searcher.as_ref().expect("setUp ran"),
            Postings::Positions,
        )?))
    }
}

/// `TestSpanCollection`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<SpanCollection>(
        "org.apache.lucene.queries.spans.TestSpanCollection",
        vec![
            tear_down(do_tear_down),
            set_up(do_set_up),
            ftest("testNestedNearQuery", test_nested_near_query),
            ftest("testOrQuery", test_or_query),
            ftest("testSpanNotQuery", test_span_not_query),
        ],
    )
}

fn do_tear_down(f: &mut SpanCollection) -> Result<()> {
    f.searcher = None;
    f.reader.take().expect("setUp ran").close()?;
    f.directory.take().expect("setUp ran").close()
}

fn do_set_up(f: &mut SpanCollection) -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::new(&*random()));
    let config = LuceneTestCase::new_index_writer_config_analyzer(Some(analyzer));
    config.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let writer = RandomIndexWriter::new_with_config(&*random(), directory.as_directory(), config)?;
    for i in 0..f.doc_fields.len() {
        let mut doc = Document::new();
        doc.add(LuceneTestCase::new_field(FIELD, f.doc_fields[i], &OFFSETS));
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

/// `TestSpanCollection.TermCollector`.
#[derive(Default)]
struct TermCollector {
    terms: HashSet<Term>,
}

impl SpanCollector for TermCollector {
    fn collect_leaf(
        &mut self,
        _postings: &dyn PostingsEnum,
        _position: i32,
        term: &Term,
    ) -> Result<()> {
        self.terms.insert(term.clone());
        Ok(())
    }

    fn reset(&mut self) {
        self.terms.clear();
    }
}

/// `checkCollectedTerms(Spans spans, TermCollector collector, Term...
/// expectedTerms)`.
fn check_collected_terms(
    spans: &mut dyn Spans,
    collector: &mut TermCollector,
    expected_terms: &[Term],
) -> Result<()> {
    collector.reset();
    spans.collect(collector)?;
    for t in expected_terms {
        Assert::assert_true_msg(&format!("Missing term {t}"), collector.terms.contains(t));
    }
    Assert::assert_equals_msg(
        Some("Unexpected terms found"),
        expected_terms.len() as i64,
        collector.terms.len() as i64,
    );
    Ok(())
}

fn term(text: &str) -> Term {
    Term::from_text(FIELD, text)
}

fn stq(text: &str) -> Arc<dyn SpanQuery> {
    Arc::new(SpanTermQuery::new(term(text)))
}

fn test_nested_near_query(f: &mut SpanCollection) -> Result<()> {
    // near(w1, near(w2, or(w3, w4)))

    let q1 = stq("w1");
    let q2 = stq("w2");
    let q3 = stq("w3");
    let q4 = stq("w4");

    let q5: Arc<dyn SpanQuery> = Arc::new(SpanOrQuery::new(vec![q4, q3])?);
    let q6: Arc<dyn SpanQuery> = Arc::new(SpanNearQuery::new(vec![q2, q5], 1, true)?);
    let q7: Arc<dyn SpanQuery> = Arc::new(SpanNearQuery::new(vec![q1, q6], 1, true)?);

    let mut collector = TermCollector::default();
    let mut spans = f.spans(&*q7)?;
    Assert::assert_equals(0i64, i64::from(spans.advance(0)?));
    spans.next_start_position()?;
    check_collected_terms(
        &mut *spans,
        &mut collector,
        &[term("w1"), term("w2"), term("w3")],
    )?;

    Assert::assert_equals(3i64, i64::from(spans.advance(3)?));
    spans.next_start_position()?;
    check_collected_terms(
        &mut *spans,
        &mut collector,
        &[term("w1"), term("w2"), term("w4")],
    )?;
    spans.next_start_position()?;
    check_collected_terms(
        &mut *spans,
        &mut collector,
        &[term("w1"), term("w2"), term("w3")],
    )
}

fn test_or_query(f: &mut SpanCollection) -> Result<()> {
    let q2 = stq("w2");
    let q3 = stq("w3");
    let or_query: Arc<dyn SpanQuery> = Arc::new(SpanOrQuery::new(vec![q2, q3])?);

    let mut collector = TermCollector::default();
    let mut spans = f.spans(&*or_query)?;

    Assert::assert_equals(1i64, i64::from(spans.advance(1)?));
    spans.next_start_position()?;
    check_collected_terms(&mut *spans, &mut collector, &[term("w3")])?;
    spans.next_start_position()?;
    check_collected_terms(&mut *spans, &mut collector, &[term("w2")])?;
    spans.next_start_position()?;
    check_collected_terms(&mut *spans, &mut collector, &[term("w3")])?;

    Assert::assert_equals(3i64, i64::from(spans.advance(3)?));
    spans.next_start_position()?;
    check_collected_terms(&mut *spans, &mut collector, &[term("w2")])?;
    spans.next_start_position()?;
    check_collected_terms(&mut *spans, &mut collector, &[term("w2")])?;
    spans.next_start_position()?;
    check_collected_terms(&mut *spans, &mut collector, &[term("w3")])
}

fn test_span_not_query(f: &mut SpanCollection) -> Result<()> {
    let q1 = stq("w1");
    let q2 = stq("w2");
    let q3 = stq("w3");

    let nq: Arc<dyn SpanQuery> = Arc::new(SpanNearQuery::new(vec![q1, q2], 2, true)?);
    let notq: Arc<dyn SpanQuery> = Arc::new(SpanNotQuery::new(nq, q3)?);

    let mut collector = TermCollector::default();
    let mut spans = f.spans(&*notq)?;

    Assert::assert_equals(2i64, i64::from(spans.advance(2)?));
    spans.next_start_position()?;
    check_collected_terms(&mut *spans, &mut collector, &[term("w1"), term("w2")])
}
