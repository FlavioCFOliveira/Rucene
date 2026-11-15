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

//! Port of `TestIndexReaderFunctions.java`.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{AfterClass, Assert, BeforeClass};
use rucene::analysis::Analyzer;
use rucene::document::{Document, SortedDocValuesField, Store, StringField, TextField};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::{IndexReader, Term};
use rucene::search::double_values_source::{self, DoubleValuesSource};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::long_values_source::{self, LongValuesSource};
use rucene::search::match_all_docs_query::MatchAllDocsQuery;
use rucene::search::query::Query;
use rucene::search::score_mode::ScoreMode;
use rucene::search::sort::{Sort, SortField, SortFieldType};
use rucene::search::ScoreDoc;
use rucene::util::BytesRef;
use rucene_queries::function::function_score_query::FunctionScoreQuery;
use rucene_queries::function::index_reader_functions::IndexReaderFunctions;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::check_hits::CheckHits;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::queries_support::core_support::{class, random, test};
use crate::queries_support::search_support::static_hook;

/// `documents`: id, double, float, int, long, string, text, double MV (x3),
/// int MV (x3).
const DOCUMENTS: [[&str; 13]; 2] = [
    [
        "0",
        "3.63",
        "5.2",
        "35",
        "4343",
        "test",
        "this is a test test test",
        "2.13",
        "3.69",
        "-0.11",
        "1",
        "7",
        "5",
    ],
    [
        "1",
        "5.65",
        "9.3",
        "54",
        "1954",
        "bar",
        "second test",
        "12.79",
        "123.456",
        "0.01",
        "12",
        "900",
        "-1",
    ],
];

/// The static fields `dir`, `analyzer`, `reader` and `searcher`.
struct Statics {
    dir: TestDirectory,
    analyzer: Arc<dyn Analyzer>,
    reader: Arc<dyn DirectoryReader>,
    searcher: Arc<IndexSearcher>,
}

static STATICS: Mutex<Option<Statics>> = Mutex::new(None);

fn searcher() -> Arc<IndexSearcher> {
    let g = STATICS.lock().unwrap_or_else(|e| e.into_inner());
    Arc::clone(&g.as_ref().expect("beforeClass ran").searcher)
}

fn reader() -> Arc<dyn IndexReader> {
    let g = STATICS.lock().unwrap_or_else(|e| e.into_inner());
    Arc::clone(&g.as_ref().expect("beforeClass ran").reader) as Arc<dyn IndexReader>
}

/// `TestIndexReaderFunctions`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.queries.function.TestIndexReaderFunctions",
        vec![
            static_hook("beforeClass", BeforeClass, before_class),
            static_hook("afterClass", AfterClass, after_class),
            test("testDocFreq", test_doc_freq),
            test("testMaxDoc", test_max_doc),
            test("testNumDocs", test_num_docs),
            test("testSumTotalTermFreq", test_sum_total_term_freq),
            test("testTermFreq", test_term_freq),
            test("testTotalTermFreq", test_total_term_freq),
            test("testNumDeletedDocs", test_num_deleted_docs),
            test("testSumDocFreq", test_sum_doc_freq),
            test("testDocCount", test_doc_count),
        ],
    )
}

fn before_class() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::new(&*random()));
    let iw_config = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::clone(&analyzer)));
    iw_config.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let iw = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iw_config)?;
    for doc in DOCUMENTS {
        let mut document = Document::new();
        document.add(Box::new(StringField::new(
            "id",
            doc[0].to_string(),
            Store::NO,
        )?));
        document.add(Box::new(SortedDocValuesField::new(
            "id",
            BytesRef::from_char_sequence(doc[0]),
        )));
        document.add(Box::new(StringField::new(
            "string",
            doc[5].to_string(),
            Store::NO,
        )?));
        document.add(Box::new(SortedDocValuesField::new(
            "string",
            BytesRef::from_char_sequence(doc[5]),
        )));
        document.add(Box::new(TextField::new(
            "text",
            doc[6].to_string(),
            Store::NO,
        )?));
        iw.add_document(&document)?;
    }
    let reader = iw.get_reader()?;
    let searcher = Arc::new(LuceneTestCase::new_searcher(Arc::clone(&reader)));
    iw.close()?;
    *STATICS.lock().unwrap_or_else(|e| e.into_inner()) = Some(Statics {
        dir,
        analyzer,
        reader,
        searcher,
    });
    Ok(())
}

fn after_class() -> Result<()> {
    if let Some(Statics {
        dir,
        analyzer,
        reader,
        searcher,
    }) = STATICS.lock().unwrap_or_else(|e| e.into_inner()).take()
    {
        drop(searcher);
        // IOUtils.close(reader, dir, analyzer)
        let r = reader.close();
        let d = dir.close();
        analyzer.close();
        r?;
        d?;
    }
    Ok(())
}

fn test_doc_freq() -> Result<()> {
    let vs = IndexReaderFunctions::doc_freq(Term::from_text("text", "test"));
    assert_hits(&vs, &[2.0, 2.0])?;
    Assert::assert_equals_str(
        None,
        Some("docFreq(text:test)"),
        Some(&vs.to_source_string()),
    );
    assert_cacheable(&vs, false)
}

fn test_max_doc() -> Result<()> {
    let vs = IndexReaderFunctions::max_doc();
    assert_hits(&vs, &[2.0, 2.0])?;
    Assert::assert_equals_str(None, Some("maxDoc()"), Some(&vs.to_source_string()));
    assert_cacheable(&vs, false)
}

fn test_num_docs() -> Result<()> {
    let vs = IndexReaderFunctions::num_docs();
    assert_hits(&vs, &[2.0, 2.0])?;
    Assert::assert_equals_str(None, Some("numDocs()"), Some(&vs.to_source_string()));
    assert_cacheable(&vs, false)
}

fn test_sum_total_term_freq() -> Result<()> {
    let vs = IndexReaderFunctions::sum_total_term_freq("text");
    assert_hits(
        &long_values_source::to_double_values_source(Arc::clone(&vs)),
        &[8.0, 8.0],
    )?;
    Assert::assert_equals_str(
        None,
        Some("sumTotalTermFreq(text)"),
        Some(&vs.to_source_string()),
    );
    assert_cacheable_long(&vs, false)
}

fn test_term_freq() -> Result<()> {
    assert_hits(
        &IndexReaderFunctions::term_freq(Term::from_text("string", "bar")),
        &[0.0, 1.0],
    )?;
    assert_hits(
        &IndexReaderFunctions::term_freq(Term::from_text("text", "test")),
        &[3.0, 1.0],
    )?;
    assert_hits(
        &IndexReaderFunctions::term_freq(Term::from_text("bogus", "bogus")),
        &[0.0, 0.0],
    )?;
    Assert::assert_equals_str(
        None,
        Some("termFreq(string:bar)"),
        Some(&IndexReaderFunctions::term_freq(Term::from_text("string", "bar")).to_source_string()),
    );
    assert_cacheable(
        &IndexReaderFunctions::term_freq(Term::from_text("text", "test")),
        true,
    )
}

fn test_total_term_freq() -> Result<()> {
    let vs = IndexReaderFunctions::total_term_freq(Term::from_text("text", "test"));
    assert_hits(&vs, &[4.0, 4.0])?;
    Assert::assert_equals_str(
        None,
        Some("totalTermFreq(text:test)"),
        Some(&vs.to_source_string()),
    );
    assert_cacheable(&vs, false)
}

fn test_num_deleted_docs() -> Result<()> {
    let vs = IndexReaderFunctions::num_deleted_docs();
    assert_hits(&vs, &[0.0, 0.0])?;
    Assert::assert_equals_str(None, Some("numDeletedDocs()"), Some(&vs.to_source_string()));
    assert_cacheable(&vs, false)
}

fn test_sum_doc_freq() -> Result<()> {
    let vs = IndexReaderFunctions::sum_doc_freq("text");
    assert_hits(&vs, &[6.0, 6.0])?;
    Assert::assert_equals_str(None, Some("sumDocFreq(text)"), Some(&vs.to_source_string()));
    assert_cacheable(&vs, false)
}

fn test_doc_count() -> Result<()> {
    let vs = IndexReaderFunctions::doc_count("text");
    assert_hits(&vs, &[2.0, 2.0])?;
    Assert::assert_equals_str(None, Some("docCount(text)"), Some(&vs.to_source_string()));
    assert_cacheable(&vs, false)
}

/// `assertCacheable(DoubleValuesSource, boolean)`.
fn assert_cacheable(vs: &Arc<dyn DoubleValuesSource>, expected: bool) -> Result<()> {
    let q: Arc<dyn Query> = Arc::new(FunctionScoreQuery::new(
        Arc::new(MatchAllDocsQuery::instance()),
        Arc::clone(vs),
    ));
    let w = searcher().create_weight(q, ScoreMode::COMPLETE, 1.0)?;
    let ctx = Arc::clone(&reader().leaves()?[0]);
    Assert::assert_equals(expected, w.is_cacheable(&ctx));
    Ok(())
}

/// `assertCacheable(LongValuesSource, boolean)`.
fn assert_cacheable_long(vs: &Arc<dyn LongValuesSource>, expected: bool) -> Result<()> {
    assert_cacheable(
        &long_values_source::to_double_values_source(Arc::clone(vs)),
        expected,
    )
}

/// `new Sort(new SortField("id", SortField.Type.STRING))`.
pub fn id_sort() -> Result<Sort> {
    Sort::new_fields(vec![SortField::new(
        Some("id".to_string()),
        SortFieldType::String,
    )?])
}

/// `assertHits(DoubleValuesSource, float[])`.
fn assert_hits(vs: &Arc<dyn DoubleValuesSource>, scores: &[f32]) -> Result<()> {
    let searcher = searcher();
    let q: Arc<dyn Query> = Arc::new(FunctionScoreQuery::new(
        Arc::new(MatchAllDocsQuery::instance()),
        Arc::clone(vs),
    ));
    let mut expected: Vec<ScoreDoc> = Vec::with_capacity(scores.len());
    let mut expected_docs = vec![0i32; scores.len()];
    for i in 0..scores.len() {
        expected_docs[i] = i as i32;
        expected.push(ScoreDoc::new(i as i32, scores[i]));
    }
    let docs = searcher.search_with_sort_and_scores(
        Arc::clone(&q),
        DOCUMENTS.len() as i32,
        &id_sort()?,
        true,
    )?;
    let score_docs: Vec<ScoreDoc> = docs.score_docs.iter().map(|fd| fd.score_doc).collect();
    CheckHits::check_hits(&*random(), &q, Some(""), &searcher, &expected_docs)?;
    CheckHits::check_hits_query(&*q, &expected, &score_docs, &expected_docs);
    CheckHits::check_explanations(&q, Some(""), &searcher)?;
    assert_sort(vs, expected)
}

/// `assertSort(DoubleValuesSource, ScoreDoc[])`.
fn assert_sort(vs: &Arc<dyn DoubleValuesSource>, mut expected: Vec<ScoreDoc>) -> Result<()> {
    let searcher = searcher();
    let reversed = random().next_boolean();
    // Arrays.sort (stable) with `(int) (b.score - a.score)` or `(int) (a.score - b.score)`
    expected.sort_by(|a, b| {
        let d = if reversed {
            b.score - a.score
        } else {
            a.score - b.score
        };
        (d as i32).cmp(&0)
    });
    let expected_docs: Vec<i32> = expected.iter().map(|sd| sd.doc).collect();
    let all: Arc<dyn Query> = Arc::new(MatchAllDocsQuery::instance());
    let sort = Sort::new_fields(vec![double_values_source::get_sort_field(
        Arc::clone(vs),
        reversed,
    )?])?;
    let docs = searcher.search_with_sort(Arc::clone(&all), expected.len() as i32, &sort)?;
    let score_docs: Vec<ScoreDoc> = docs.score_docs.iter().map(|fd| fd.score_doc).collect();
    CheckHits::check_hits_query(&*all, &expected, &score_docs, &expected_docs);
    Ok(())
}
