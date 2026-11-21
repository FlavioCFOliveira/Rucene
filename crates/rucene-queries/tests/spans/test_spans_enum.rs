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

//! Port of `TestSpansEnum.java`: tests Spans (v2).

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{AfterClass, BeforeClass};
use rucene::analysis::Analyzer;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::Query;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::check_hits::CheckHits;
use rucene_test_framework::util::english::English;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::queries_support::core_support::{class, random, test};
use crate::queries_support::search_support::static_hook;
use crate::queries_support::span_test_util::{
    span_not_query, span_not_query_pre_post, span_or_query_terms, span_term_query,
};

/// The static fields `searcher`, `reader` and `directory`.
struct Statics {
    searcher: Arc<IndexSearcher>,
    reader: Arc<dyn DirectoryReader>,
    directory: TestDirectory,
}

static STATICS: Mutex<Option<Statics>> = Mutex::new(None);

fn searcher() -> Arc<IndexSearcher> {
    let guard = STATICS.lock().unwrap_or_else(|e| e.into_inner());
    Arc::clone(&guard.as_ref().expect("beforeClass ran").searcher)
}

/// `TestSpansEnum`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.queries.spans.TestSpansEnum",
        vec![
            static_hook("beforeClass", BeforeClass, before_class),
            static_hook("afterClass", AfterClass, after_class),
            test("testSpansEnumOr1", test_spans_enum_or1),
            test("testSpansEnumOr2", test_spans_enum_or2),
            test("testSpansEnumOr3", test_spans_enum_or3),
            test("testSpansEnumOrNot1", test_spans_enum_or_not1),
            test(
                "testSpansEnumNotBeforeAfter1",
                test_spans_enum_not_before_after1,
            ),
            test(
                "testSpansEnumNotBeforeAfter2",
                test_spans_enum_not_before_after2,
            ),
            test(
                "testSpansEnumNotBeforeAfter3",
                test_spans_enum_not_before_after3,
            ),
        ],
    )
}

fn before_class() -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::new(&*random()));
    let config = LuceneTestCase::new_index_writer_config_analyzer(Some(analyzer));
    config.set_max_buffered_docs(TestUtil::next_int(&*random(), 100, 1000))?;
    config.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let writer = RandomIndexWriter::new_with_config(&*random(), directory.as_directory(), config)?;
    // writer.infoStream = System.out;
    for i in 0..10 {
        let mut doc = Document::new();
        doc.add(LuceneTestCase::new_text_field(
            "field",
            &English::int_to_english(i),
            Store::YES,
        ));
        writer.add_document(&doc)?;
    }
    for i in 100..110 {
        let mut doc = Document::new(); // doc id 10-19 have 100-109
        doc.add(LuceneTestCase::new_text_field(
            "field",
            &English::int_to_english(i),
            Store::YES,
        ));
        writer.add_document(&doc)?;
    }
    let reader = writer.get_reader()?;
    let searcher = Arc::new(LuceneTestCase::new_searcher(Arc::clone(&reader)));
    writer.close()?;
    *STATICS.lock().unwrap_or_else(|e| e.into_inner()) = Some(Statics {
        searcher,
        reader,
        directory,
    });
    Ok(())
}

fn after_class() -> Result<()> {
    let taken = STATICS.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(Statics {
        searcher,
        reader,
        directory,
    }) = taken
    {
        reader.close()?;
        directory.close()?;
        drop(searcher);
    }
    Ok(())
}

fn check_hits(query: Arc<dyn SpanQuery>, results: &[i32]) -> Result<()> {
    CheckHits::check_hits(
        &*random(),
        &(query as Arc<dyn Query>),
        Some("field"),
        &searcher(),
        results,
    )
}

fn test_spans_enum_or1() -> Result<()> {
    check_hits(
        span_or_query_terms("field", &["one", "two"])?,
        &[1, 2, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19],
    )
}

fn test_spans_enum_or2() -> Result<()> {
    check_hits(
        span_or_query_terms("field", &["one", "eleven"])?,
        &[1, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19],
    )
}

fn test_spans_enum_or3() -> Result<()> {
    check_hits(span_or_query_terms("field", &["twelve", "eleven"])?, &[])
}

/// `spanTQ(String s)`.
#[allow(dead_code)]
pub fn span_tq(s: &str) -> Arc<dyn SpanQuery> {
    span_term_query("field", s)
}

fn test_spans_enum_or_not1() -> Result<()> {
    check_hits(
        span_not_query(
            span_or_query_terms("field", &["one", "two"])?,
            span_term_query("field", "one"),
        )?,
        &[2, 12],
    )
}

fn test_spans_enum_not_before_after1() -> Result<()> {
    check_hits(
        span_not_query(
            span_term_query("field", "hundred"),
            span_term_query("field", "one"),
        )?,
        &[10, 11, 12, 13, 14, 15, 16, 17, 18, 19],
    ) // include all "one hundred ..."
}

fn test_spans_enum_not_before_after2() -> Result<()> {
    check_hits(
        span_not_query_pre_post(
            span_term_query("field", "hundred"),
            span_term_query("field", "one"),
            1,
            0,
        )?,
        &[],
    ) // exclude all "one hundred ..."
}

fn test_spans_enum_not_before_after3() -> Result<()> {
    check_hits(
        span_not_query_pre_post(
            span_term_query("field", "hundred"),
            span_term_query("field", "one"),
            0,
            1,
        )?,
        &[10, 12, 13, 14, 15, 16, 17, 18, 19],
    ) // exclude "one hundred one"
}
