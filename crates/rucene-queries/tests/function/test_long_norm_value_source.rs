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

//! Port of `TestLongNormValueSource.java`.

use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{AfterClass, BeforeClass};
use rucene::analysis::Analyzer;
use rucene::document::{Document, Store, TextField};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::Query;
use rucene::search::similarities::{ClassicSimilarity, Similarity};
use rucene::search::ScoreDoc;
use rucene_queries::function::function_query::FunctionQuery;
use rucene_queries::function::valuesource::norm_value_source::NormValueSource;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::check_hits::CheckHits;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::function::test_index_reader_functions::id_sort;
use crate::queries_support::core_support::{class, random, test};
use crate::queries_support::search_support::static_hook;

/// The static fields `dir`, `reader`, `searcher` and `analyzer`.
struct Statics {
    dir: TestDirectory,
    reader: Arc<dyn DirectoryReader>,
    searcher: IndexSearcher,
    analyzer: Arc<dyn Analyzer>,
}

static STATICS: Mutex<Option<Statics>> = Mutex::new(None);

/// `sim`.
static SIM: LazyLock<Arc<dyn Similarity>> =
    LazyLock::new(|| Arc::new(ClassicSimilarity::new()) as Arc<dyn Similarity>);

/// `TestLongNormValueSource`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.queries.function.TestLongNormValueSource",
        vec![
            static_hook("beforeClass", BeforeClass, before_class),
            static_hook("afterClass", AfterClass, after_class),
            test("testNorm", test_norm),
        ],
    )
}

fn before_class() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::new(&*random()));
    let iw_config = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::clone(&analyzer)));
    iw_config.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    iw_config.set_similarity(Some(Arc::clone(&SIM)))?;
    let iw = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iw_config)?;

    let mut doc = Document::new();
    doc.add(Box::new(TextField::new(
        "text",
        "this is a test test test".to_string(),
        Store::NO,
    )?));
    iw.add_document(&doc)?;

    let mut doc = Document::new();
    doc.add(Box::new(TextField::new(
        "text",
        "second test".to_string(),
        Store::NO,
    )?));
    iw.add_document(&doc)?;

    let reader = iw.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    iw.close()?;
    *STATICS.lock().unwrap_or_else(|e| e.into_inner()) = Some(Statics {
        dir,
        reader,
        searcher,
        analyzer,
    });
    Ok(())
}

fn after_class() -> Result<()> {
    if let Some(Statics {
        dir,
        reader,
        searcher,
        analyzer,
    }) = STATICS.lock().unwrap_or_else(|e| e.into_inner()).take()
    {
        drop(searcher);
        reader.close()?;
        dir.close()?;
        analyzer.close();
    }
    Ok(())
}

fn test_norm() -> Result<()> {
    let mut guard = STATICS.lock().unwrap_or_else(|e| e.into_inner());
    let searcher = &mut guard.as_mut().expect("beforeClass ran").searcher;
    let saved = Arc::clone(searcher.get_similarity());
    // no norm field (so agnostic to indexed similarity)
    searcher.set_similarity(Arc::clone(&SIM));
    let result = assert_hits(
        searcher,
        Arc::new(FunctionQuery::new(Arc::new(NormValueSource::new("text")))),
        &[0.0, 0.0],
    );
    searcher.set_similarity(saved);
    result
}

fn assert_hits(searcher: &IndexSearcher, q: Arc<dyn Query>, scores: &[f32]) -> Result<()> {
    let mut expected: Vec<ScoreDoc> = Vec::with_capacity(scores.len());
    let mut expected_docs = vec![0i32; scores.len()];
    for i in 0..scores.len() {
        expected_docs[i] = i as i32;
        expected.push(ScoreDoc::new(i as i32, scores[i]));
    }
    let docs = searcher.search_with_sort(Arc::clone(&q), 2, &id_sort()?)?;
    let score_docs: Vec<ScoreDoc> = docs.score_docs.iter().map(|fd| fd.score_doc).collect();
    /*
    for (int i=0;i<docs.scoreDocs.length;i++) {
      System.out.println(searcher.explain(q, docs.scoreDocs[i].doc));
    }
    */
    CheckHits::check_hits(&*random(), &q, Some(""), searcher, &expected_docs)?;
    CheckHits::check_hits_query(&*q, &expected, &score_docs, &expected_docs);
    CheckHits::check_explanations(&q, Some(""), searcher)
}
