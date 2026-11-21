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

//! Port of `TestPayloadSpanPositions.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::StringCharReader;
use rucene::document::{Document, TextField};
use rucene::error::Result;
use rucene::index::postings_enum::POSTINGS_ENUM_ALL;
use rucene::index::{IndexReader, PostingsEnum, Term};
use rucene::search::doc_id_set_iterator::NO_MORE_DOCS;
use rucene::util::BytesRef;
use rucene_queries::spans::span_collector::SpanCollector;
use rucene_queries::spans::span_near_query::SpanNearQuery;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_queries::spans::span_term_query::SpanTermQuery;
use rucene_queries::spans::span_weight::Postings;
use rucene_queries::spans::spans::NO_MORE_POSITIONS;
use rucene_test_framework::analysis::mock_payload_analyzer::MockPayloadAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::LuceneTestCase;

use crate::queries_support::core_support::{class, random, test, verbose};
use crate::queries_support::{leaf_spans, non_null_spans};

/// `TestPayloadSpanPositions`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.queries.payloads.TestPayloadSpanPositions",
        vec![test("testPayloadsPos0", test_payloads_pos0)],
    )
}

/// `TestPayloadSpanPositions.PayloadSpanCollector`.
#[derive(Default)]
struct PayloadSpanCollector {
    payloads: Vec<BytesRef>,
}

impl SpanCollector for PayloadSpanCollector {
    fn collect_leaf(
        &mut self,
        postings: &dyn PostingsEnum,
        _position: i32,
        _term: &Term,
    ) -> Result<()> {
        if let Some(payload) = postings.get_payload()? {
            self.payloads.push(BytesRef::new(payload.to_vec()));
        }
        Ok(())
    }

    fn reset(&mut self) {
        self.payloads.clear();
    }
}

fn assert_long(expected: i32, actual: i32) {
    Assert::assert_equals(i64::from(expected), i64::from(actual));
}

fn test_payloads_pos0() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new(
        &*random(),
        dir.as_directory(),
        Arc::new(MockPayloadAnalyzer::new()),
    )?;
    let mut doc = Document::new();
    doc.add(TextField::new_with_reader(
        "content",
        Box::new(StringCharReader::new("a a b c d e a f g h i j a b k k")),
    )?);
    writer.add_document(&doc)?;

    let reader_from_writer = writer.get_reader()?;
    let r = LuceneTestCase::get_only_leaf_reader(
        Arc::clone(&reader_from_writer) as Arc<dyn IndexReader>
    )?;

    let mut tp = r
        .postings(&Term::from_text("content", "a"), POSTINGS_ENUM_ALL)?
        .unwrap_or_else(|| {
            randomizedtesting::jdk::exception::throw_without_message(
                "java.lang.NullPointerException",
            )
        });

    Assert::assert_true(tp.next_doc()? != NO_MORE_DOCS);
    // "a" occurs 4 times
    assert_long(4, tp.freq()?);
    assert_long(0, tp.next_position()?);
    assert_long(1, tp.next_position()?);
    assert_long(3, tp.next_position()?);
    assert_long(6, tp.next_position()?);

    // only one doc has "a"
    assert_long(NO_MORE_DOCS, tp.next_doc()?);

    let is = LuceneTestCase::new_searcher(LuceneTestCase::get_only_leaf_reader(Arc::clone(
        &reader_from_writer,
    )
        as Arc<dyn IndexReader>)?);

    let stq1: Arc<dyn SpanQuery> = Arc::new(SpanTermQuery::new(Term::from_text("content", "a")));
    let stq2: Arc<dyn SpanQuery> = Arc::new(SpanTermQuery::new(Term::from_text("content", "k")));
    let sqs = vec![stq1, stq2];
    let snq = SpanNearQuery::new(sqs, 30, false)?;

    let mut count = 0;
    let mut saw_zero = false;
    if verbose() {
        println!("\ngetPayloadSpans test");
    }
    let mut collector = PayloadSpanCollector::default();
    let mut pspans = non_null_spans(leaf_spans(&snq, &is, Postings::Payloads)?);
    while pspans.next_doc()? != NO_MORE_DOCS {
        while pspans.next_start_position()? != NO_MORE_POSITIONS {
            if verbose() {
                println!(
                    "doc {}: span {} to {}",
                    pspans.doc_id(),
                    pspans.start_position(),
                    pspans.end_position()
                );
            }
            collector.reset();
            pspans.collect(&mut collector)?;
            saw_zero |= pspans.start_position() == 0;
            for payload in &collector.payloads {
                count += 1;
                if verbose() {
                    println!("  payload: {}", Term::to_string_bytes_ref(payload));
                }
            }
        }
    }
    Assert::assert_true(saw_zero);
    assert_long(8, count);

    // System.out.println("\ngetSpans test");
    let mut spans = non_null_spans(leaf_spans(&snq, &is, Postings::Positions)?);
    count = 0;
    saw_zero = false;
    while spans.next_doc()? != NO_MORE_DOCS {
        while spans.next_start_position()? != NO_MORE_POSITIONS {
            count += 1;
            saw_zero |= spans.start_position() == 0;
            // System.out.println(spans.storedFields().document() + " - " + spans.start() + " - " +
            // spans.end());
        }
    }
    assert_long(4, count);
    Assert::assert_true(saw_zero);

    writer.close()?;
    is.get_index_reader().close()?;
    dir.close()
}
