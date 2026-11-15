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

//! Port of `TestPayloadFilteredInterval.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::{Analyzer, AnalyzerState, GlobalReuseStrategy, TokenStreamComponents};
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::IndexReader;
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene_queries::intervals::interval_iterator::NO_MORE_INTERVALS;
use rucene_queries::intervals::intervals::Intervals;
use rucene_queries::intervals::intervals_source::IntervalsSource;
use rucene_test_framework::analysis::mock_tokenizer::{MockTokenizer, SIMPLE};
use rucene_test_framework::analysis::simple_payload_filter::SimplePayloadFilter;
use rucene_test_framework::analysis::{components, shared, sink};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::queries_support::core_support::{class, random, test};

/// `TestPayloadFilteredInterval`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.queries.intervals.TestPayloadFilteredInterval",
        vec![
            test(
                "testPayloadFilteredInterval",
                test_payload_filtered_interval,
            ),
            test(
                "testPayloadFilteredTermIntervalsSourceEquals",
                test_payload_filtered_term_intervals_source_equals,
            ),
        ],
    )
}

/// The anonymous `Analyzer` of `testPayloadFilteredInterval`.
#[derive(Debug)]
struct PayloadAnalyzer {
    state: AnalyzerState,
}

impl Analyzer for PayloadAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.queries.intervals.TestPayloadFilteredInterval$1"
    }

    fn create_components(&self, _field_name: Option<&str>) -> Result<TokenStreamComponents> {
        let tok = Arc::new(AtomicRefCell::new(MockTokenizer::new(
            Arc::clone(&SIMPLE),
            true,
        )?));
        let filter = SimplePayloadFilter::new(shared(&tok))?;
        Ok(components(tok, sink(filter)))
    }
}

fn test_payload_filtered_interval() -> Result<()> {
    let analyzer: Arc<dyn Analyzer> = Arc::new(PayloadAnalyzer {
        state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
    });
    let directory = LuceneTestCase::new_directory();
    let config = LuceneTestCase::new_index_writer_config_analyzer(Some(analyzer));
    config.set_max_buffered_docs(TestUtil::next_int(&*random(), 100, 1000))?;
    config.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let writer = RandomIndexWriter::new_with_config(&*random(), directory.as_directory(), config)?;

    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "field",
        "a sentence with words repeated words words quite often words",
        Store::NO,
    )));
    writer.add_document(&doc)?;
    let reader = writer.get_reader()?;
    writer.close()?;

    // SimplePayloadFilter stores a payload for each term at position n containing
    // the bytes 'pos:n'
    let source = Intervals::term_str_with_payload_filter(
        "words",
        Arc::new(|b: Option<&[u8]>| {
            // `b.utf8ToString()`: a null payload is a NullPointerException
            let b = b.unwrap_or_else(|| {
                randomizedtesting::jdk::exception::throw_without_message(
                    "java.lang.NullPointerException",
                )
            });
            !String::from_utf8_lossy(b).ends_with('5')
        }),
    );
    Assert::assert_equals_str(
        None,
        Some("PAYLOAD_FILTERED(words)"),
        Some(&source.to_source_string()),
    );

    let leaf = Arc::clone(&(Arc::clone(&reader) as Arc<dyn IndexReader>).leaves()?[0]);
    let mut it = source
        .intervals("field", &leaf)?
        .expect("a non-null IntervalIterator");
    Assert::assert_equals(0, it.next_doc()?);
    Assert::assert_equals(3, it.next_interval()?);
    Assert::assert_equals(6, it.next_interval()?);
    Assert::assert_equals(9, it.next_interval()?);
    Assert::assert_equals(NO_MORE_INTERVALS, it.next_interval()?);

    let mi = source.matches("field", &leaf, 0)?;
    Assert::assert_true(mi.is_some());
    let mut mi = mi.expect("assertNotNull(mi)");
    Assert::assert_true(mi.next()?);
    Assert::assert_equals(3, mi.start_position());
    Assert::assert_true(mi.next()?);
    Assert::assert_equals(6, mi.start_position());
    Assert::assert_true(mi.next()?);
    Assert::assert_equals(9, mi.start_position());
    Assert::assert_false(mi.next()?);

    reader.close()?;
    directory.close()
}

/// `Object.equals(Object)` with a possibly `null` argument: `false` for
/// `null`, as `PayloadFilteredTermIntervalsSource.equals` answers it.
fn equals_nullable(
    source: &Arc<dyn IntervalsSource>,
    other: Option<&Arc<dyn IntervalsSource>>,
) -> bool {
    other.is_some_and(|o| source.source_eq(&**o))
}

fn test_payload_filtered_term_intervals_source_equals() -> Result<()> {
    let interval = Intervals::term_str_with_payload_filter("test", Arc::new(|_payload| true));
    let same_interval = Intervals::term_str_with_payload_filter("test", Arc::new(|_payload| true));
    let different_interval = Intervals::term_str("test");

    Assert::assert_true(interval.source_eq(&*same_interval));
    Assert::assert_false(interval.source_eq(&*different_interval));
    Assert::assert_false(equals_nullable(&interval, None));
    Ok(())
}
