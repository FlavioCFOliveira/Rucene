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

//! Port of `TestSpanFirstQuery.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::Analyzer;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::search::query::Query;
use rucene::util::automaton::{CharacterRunAutomaton, RegExp};
use rucene_test_framework::analysis::mock_tokenizer::SIMPLE;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::LuceneTestCase;

use crate::queries_support::core_support::{class, random, test};
use crate::queries_support::span_test_util::{span_first_query, span_not_query, span_term_query};

/// `TestSpanFirstQuery`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.queries.spans.TestSpanFirstQuery",
        vec![test("testStartPositions", test_start_positions)],
    )
}

fn test_start_positions() -> Result<()> {
    let dir = LuceneTestCase::new_directory();

    // mimic StopAnalyzer
    let stop_set = Arc::new(CharacterRunAutomaton::new(
        RegExp::new("the|a|of")?.to_automaton()?,
    )?);
    let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::with_filter(
        &*random(),
        Arc::clone(&SIMPLE),
        true,
        stop_set,
    ));

    let writer = RandomIndexWriter::new(&*random(), dir.as_directory(), analyzer)?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "field",
        "the quick brown fox",
        Store::NO,
    )));
    writer.add_document(&doc)?;
    let mut doc2 = Document::new();
    doc2.add(Box::new(LuceneTestCase::new_text_field(
        "field",
        "quick brown fox",
        Store::NO,
    )));
    writer.add_document(&doc2)?;

    let reader = writer.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    // user queries on "starts-with quick"
    let sfq = span_first_query(span_term_query("field", "quick"), 1);
    Assert::assert_equals(
        1i64,
        searcher
            .search(Arc::clone(&sfq) as Arc<dyn Query>, 10)?
            .total_hits
            .value(),
    );

    // user queries on "starts-with the quick"
    let include = span_first_query(span_term_query("field", "quick"), 2);
    let sfq = span_not_query(include, sfq)?;
    Assert::assert_equals(
        1i64,
        searcher
            .search(sfq as Arc<dyn Query>, 10)?
            .total_hits
            .value(),
    );

    writer.close()?;
    reader.close()?;
    dir.close()
}
