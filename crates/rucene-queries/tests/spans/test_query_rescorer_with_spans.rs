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

//! Port of `TestQueryRescorerWithSpans.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::Term;
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query::Builder as BooleanQueryBuilder;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::Query;
use rucene::search::query_rescorer;
use rucene::search::similarities::ClassicSimilarity;
use rucene::search::term_query::TermQuery;
use rucene_queries::spans::span_near_query::SpanNearQuery;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_queries::spans::span_term_query::SpanTermQuery;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::LuceneTestCase;

use crate::queries_support::core_support::{assert_equals, class, random, test};

/// `TestQueryRescorerWithSpans`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.queries.spans.TestQueryRescorerWithSpans",
        vec![
            test("testBasic", test_basic),
            test("testMissingSecondPassScore", test_missing_second_pass_score),
        ],
    )
}

/// `getSearcher(IndexReader r)`.
fn get_searcher(r: Arc<dyn DirectoryReader>) -> IndexSearcher {
    let mut searcher = LuceneTestCase::new_searcher(r);

    // We rely on more tokens = lower score:
    searcher.set_similarity(Arc::new(ClassicSimilarity::new()));

    searcher
}

/// `searcher.storedFields().document(doc).get("id")`.
fn stored_id(searcher: &IndexSearcher, doc: i32) -> Result<Option<String>> {
    Ok(searcher.stored_fields()?.document(doc)?.get("id"))
}

/// The body both test methods share (Java repeats it verbatim).
fn rescore_with_span_near() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_config(
        &*random(),
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )?;

    let mut doc = Document::new();
    doc.add(LuceneTestCase::new_string_field("id", "0", Store::YES));
    doc.add(LuceneTestCase::new_text_field(
        "field",
        "wizard the the the the the oz",
        Store::NO,
    ));
    w.add_document(&doc)?;
    let mut doc = Document::new();
    doc.add(LuceneTestCase::new_string_field("id", "1", Store::YES));
    // 1 extra token, but wizard and oz are close;
    doc.add(LuceneTestCase::new_text_field(
        "field",
        "wizard oz the the the the the the",
        Store::NO,
    ));
    w.add_document(&doc)?;
    let r = w.get_reader()?;
    w.close()?;

    // Do ordinary BooleanQuery:
    let mut bq = BooleanQueryBuilder::new();
    bq.add(
        Arc::new(TermQuery::new(Term::from_text("field", "wizard"))),
        Occur::SHOULD,
    )?;
    bq.add(
        Arc::new(TermQuery::new(Term::from_text("field", "oz"))),
        Occur::SHOULD,
    )?;
    let searcher = get_searcher(Arc::clone(&r));

    let hits = searcher.search(Arc::new(bq.build()), 10)?;
    Assert::assert_equals(2i64, hits.total_hits.value());
    assert_equals(
        &Some("0".to_string()),
        &stored_id(&searcher, hits.score_docs[0].doc)?,
    );
    assert_equals(
        &Some("1".to_string()),
        &stored_id(&searcher, hits.score_docs[1].doc)?,
    );

    // Resort using SpanNearQuery:
    let t1: Arc<dyn SpanQuery> = Arc::new(SpanTermQuery::new(Term::from_text("field", "wizard")));
    let t2: Arc<dyn SpanQuery> = Arc::new(SpanTermQuery::new(Term::from_text("field", "oz")));
    let snq: Arc<dyn Query> = Arc::new(SpanNearQuery::new(vec![t1, t2], 0, true)?);

    let hits3 = query_rescorer::rescore(&searcher, &hits, snq, 2.0, 10)?;

    // Resorting changed the order:
    Assert::assert_equals(2i64, hits3.total_hits.value());
    assert_equals(
        &Some("1".to_string()),
        &stored_id(&searcher, hits3.score_docs[0].doc)?,
    );
    assert_equals(
        &Some("0".to_string()),
        &stored_id(&searcher, hits3.score_docs[1].doc)?,
    );

    r.close()?;
    dir.close()
}

fn test_basic() -> Result<()> {
    rescore_with_span_near()
}

fn test_missing_second_pass_score() -> Result<()> {
    rescore_with_span_near()
}
