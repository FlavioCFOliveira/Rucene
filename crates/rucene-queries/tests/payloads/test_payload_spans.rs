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

//! Port of `TestPayloadSpans.java`.

use std::collections::HashSet;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::{
    Analyzer, AnalyzerState, GlobalReuseStrategy, StringCharReader, TokenFilter, TokenFilterLogic,
    TokenStream, TokenStreamComponents,
};
use rucene::document::{Document, Store, TextField};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::{IndexReader, PostingsEnum, Term};
use rucene::search::doc_id_set_iterator::NO_MORE_DOCS;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::Query;
use rucene::search::similarities::{ClassicSimilarity, Similarity};
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::attribute::AttributeSource;
use rucene::util::BytesRef;
use rucene_queries::spans::span_collector::SpanCollector;
use rucene_queries::spans::span_first_query::SpanFirstQuery;
use rucene_queries::spans::span_near_query::SpanNearQuery;
use rucene_queries::spans::span_not_query::SpanNotQuery;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_queries::spans::span_term_query::SpanTermQuery;
use rucene_queries::spans::span_weight::Postings;
use rucene_queries::spans::spans::{Spans, NO_MORE_POSITIONS};
use rucene_test_framework::analysis::attributes::{char_term_id, payload_id, pos_inc_id, Att};
use rucene_test_framework::analysis::mock_tokenizer::{MockTokenizer, SIMPLE};
use rucene_test_framework::analysis::{components, shared, sink};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::queries_support::core_support::{random, verbose};
use crate::queries_support::payload_helper::{self, PayloadHelper};
use crate::queries_support::search_support::{fixture_class, ftest, set_up, tear_down};
use crate::queries_support::{first_leaf, leaf_spans, non_null_spans};

/// The instance of `TestPayloadSpans`.
pub struct PayloadSpans {
    searcher: Option<IndexSearcher>,
    similarity: Arc<dyn Similarity>,
    index_reader: Option<Arc<dyn IndexReader>>,
    close_index_reader: Option<Arc<dyn DirectoryReader>>,
    directory: Option<TestDirectory>,
    helper: Option<PayloadHelper>,
}

impl Default for PayloadSpans {
    fn default() -> Self {
        Self {
            searcher: None,
            similarity: Arc::new(ClassicSimilarity::new()),
            index_reader: None,
            close_index_reader: None,
            directory: None,
            helper: None,
        }
    }
}

impl PayloadSpans {
    fn searcher(&self) -> &IndexSearcher {
        self.searcher.as_ref().expect("setUp ran")
    }

    /// `getSearcher()`.
    fn get_searcher(&mut self) -> Result<IndexSearcher> {
        let directory = LuceneTestCase::new_directory();
        let docs = [
            "xx rr yy mm  pp",
            "xx yy mm rr pp",
            "nopayload qq ss pp np",
            "one two three four five six seven eight nine ten eleven",
            "nine one two three four five six seven eight eleven ten",
        ];
        let config =
            LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(PayloadAnalyzer::new(
                "org.apache.lucene.queries.payloads.TestPayloadSpans$PayloadAnalyzer",
            ))));
        config.set_similarity(Some(Arc::clone(&self.similarity)))?;
        let writer =
            RandomIndexWriter::new_with_config(&*random(), directory.as_directory(), config)?;

        for i in 0..docs.len() {
            let mut doc = Document::new();
            let doc_text = docs[i];
            doc.add(LuceneTestCase::new_text_field(
                payload_helper::FIELD,
                doc_text,
                Store::YES,
            ));
            writer.add_document(&doc)?;
        }

        writer.force_merge(1)?;
        let close_index_reader = writer.get_reader()?;
        writer.close()?;

        let searcher = LuceneTestCase::new_searcher_wrap(Arc::clone(&close_index_reader), false);
        self.close_index_reader = Some(close_index_reader);
        self.directory = Some(directory);
        Ok(searcher)
    }

    /// `closeIndexReader.close(); directory.close();`.
    fn close_get_searcher(&mut self) -> Result<()> {
        self.close_index_reader
            .take()
            .expect("getSearcher ran")
            .close()?;
        self.directory.take().expect("getSearcher ran").close()
    }
}

/// `TestPayloadSpans`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<PayloadSpans>(
        "org.apache.lucene.queries.payloads.TestPayloadSpans",
        vec![
            set_up(do_set_up),
            tear_down(do_tear_down),
            ftest("testSpanTermQuery", test_span_term_query),
            ftest("testSpanFirst", test_span_first),
            ftest("testSpanNot", test_span_not),
            ftest("testNestedSpans", test_nested_spans),
            ftest(
                "testFirstClauseWithoutPayload",
                test_first_clause_without_payload,
            ),
            ftest("testHeavilyNestedSpanQuery", test_heavily_nested_span_query),
            ftest(
                "testShrinkToAfterShortestMatch",
                test_shrink_to_after_shortest_match,
            ),
            ftest(
                "testShrinkToAfterShortestMatch2",
                test_shrink_to_after_shortest_match2,
            ),
            ftest(
                "testShrinkToAfterShortestMatch3",
                test_shrink_to_after_shortest_match3,
            ),
        ],
    )
}

fn do_set_up(f: &mut PayloadSpans) -> Result<()> {
    let mut helper = PayloadHelper::new();
    let searcher = helper.set_up(&*random(), Arc::clone(&f.similarity), 1000)?;
    f.index_reader = Some(Arc::clone(searcher.get_index_reader()));
    f.searcher = Some(searcher);
    f.helper = Some(helper);
    Ok(())
}

fn do_tear_down(f: &mut PayloadSpans) -> Result<()> {
    f.helper.as_mut().expect("setUp ran").tear_down()
}

fn stq(text: &str) -> Arc<dyn SpanQuery> {
    Arc::new(SpanTermQuery::new(Term::from_text(
        payload_helper::FIELD,
        text,
    )))
}

fn stq_field(field: &str, text: &str) -> Arc<dyn SpanQuery> {
    Arc::new(SpanTermQuery::new(Term::from_text(field, text)))
}

fn test_span_term_query(f: &mut PayloadSpans) -> Result<()> {
    let q = stq_field(payload_helper::FIELD, "seventy");
    let spans = leaf_spans(&*q, f.searcher(), Postings::Payloads)?;
    Assert::assert_true_msg("spans is null and it shouldn't be", spans.is_some());
    check_spans(spans, 100, 1, 1, 1)?;

    let q = stq_field(payload_helper::NO_PAYLOAD_FIELD, "seventy");
    let spans = leaf_spans(&*q, f.searcher(), Postings::Payloads)?;
    Assert::assert_true_msg("spans is null and it shouldn't be", spans.is_some());
    check_spans(spans, 100, 0, 0, 0)
}

fn test_span_first(f: &mut PayloadSpans) -> Result<()> {
    let m = stq("one");
    let sfq: Arc<dyn SpanQuery> = Arc::new(SpanFirstQuery::new(m, 2));
    let spans = leaf_spans(&*sfq, f.searcher(), Postings::Payloads)?;
    check_spans(spans, 109, 1, 1, 1)?;
    // Test more complicated subclause
    let clauses: Vec<Arc<dyn SpanQuery>> = vec![stq("one"), stq("hundred")];
    let m: Arc<dyn SpanQuery> = Arc::new(SpanNearQuery::new(clauses.clone(), 0, true)?);
    let sfq: Arc<dyn SpanQuery> = Arc::new(SpanFirstQuery::new(m, 2));
    check_spans(
        leaf_spans(&*sfq, f.searcher(), Postings::Payloads)?,
        100,
        2,
        1,
        1,
    )?;

    let m: Arc<dyn SpanQuery> = Arc::new(SpanNearQuery::new(clauses, 0, false)?);
    let sfq: Arc<dyn SpanQuery> = Arc::new(SpanFirstQuery::new(m, 2));
    check_spans(
        leaf_spans(&*sfq, f.searcher(), Postings::Payloads)?,
        100,
        2,
        1,
        1,
    )
}

fn test_span_not(f: &mut PayloadSpans) -> Result<()> {
    let clauses: Vec<Arc<dyn SpanQuery>> = vec![stq("one"), stq("three")];
    let spq: Arc<dyn SpanQuery> = Arc::new(SpanNearQuery::new(clauses, 5, true)?);
    let snq = SpanNotQuery::new(spq, stq("two"))?;

    let directory = LuceneTestCase::new_directory();
    let config = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
        PayloadAnalyzer::new("org.apache.lucene.queries.payloads.TestPayloadSpans$PayloadAnalyzer"),
    )));
    config.set_similarity(Some(Arc::clone(&f.similarity)))?;
    let writer = RandomIndexWriter::new_with_config(&*random(), directory.as_directory(), config)?;

    let mut doc = Document::new();
    doc.add(LuceneTestCase::new_text_field(
        payload_helper::FIELD,
        "one two three one four three",
        Store::YES,
    ));
    writer.add_document(&doc)?;
    let reader =
        LuceneTestCase::get_only_leaf_reader(writer.get_reader()? as Arc<dyn IndexReader>)?;
    writer.close()?;

    // `snq.createWeight(newSearcher(reader, false), ...).getSpans(reader.leaves().get(0), ...)`:
    // with `maybeWrap` false the searcher's leaves are `reader.leaves()`
    let s = LuceneTestCase::new_searcher_wrap(Arc::clone(&reader), false);
    check_spans_num(leaf_spans(&snq, &s, Postings::Payloads)?, 1, &[2])?;
    drop(s);
    Arc::clone(&reader).as_index_reader().close()?;
    directory.close()
}

fn test_nested_spans(f: &mut PayloadSpans) -> Result<()> {
    let searcher = f.get_searcher()?;

    let q = stq("mark");
    let spans = leaf_spans(&*q, &searcher, Postings::Payloads)?;
    if let Some(s) = &spans {
        Assert::fail(&format!("expected null, but was:<{}>", s.java_to_string()));
    }

    let clauses: Vec<Arc<dyn SpanQuery>> = vec![stq("rr"), stq("yy"), stq("xx")];
    let span_near_query = SpanNearQuery::new(clauses, 12, false)?;

    let spans = leaf_spans(&span_near_query, &searcher, Postings::Payloads)?;
    Assert::assert_true_msg("spans is null and it shouldn't be", spans.is_some());
    check_spans_num(spans, 2, &[3, 3])?;

    let clauses: Vec<Arc<dyn SpanQuery>> = vec![stq("xx"), stq("rr"), stq("yy")];
    let span_near_query = SpanNearQuery::new(clauses, 6, true)?;

    let spans = leaf_spans(&span_near_query, &searcher, Postings::Payloads)?;

    Assert::assert_true_msg("spans is null and it shouldn't be", spans.is_some());
    check_spans_num(spans, 1, &[3])?;

    let clauses: Vec<Arc<dyn SpanQuery>> = vec![stq("xx"), stq("rr")];

    let span_near_query: Arc<dyn SpanQuery> = Arc::new(SpanNearQuery::new(clauses, 6, true)?);

    // xx within 6 of rr

    let clauses2: Vec<Arc<dyn SpanQuery>> = vec![stq("yy"), span_near_query];

    let nested_span_near_query = SpanNearQuery::new(clauses2, 6, false)?;

    // yy within 6 of xx within 6 of rr

    let spans = leaf_spans(&nested_span_near_query, &searcher, Postings::Payloads)?;
    Assert::assert_true_msg("spans is null and it shouldn't be", spans.is_some());
    check_spans_num(spans, 2, &[3, 3])?;
    drop(searcher);
    f.close_get_searcher()
}

fn test_first_clause_without_payload(f: &mut PayloadSpans) -> Result<()> {
    let searcher = f.get_searcher()?;

    let clauses: Vec<Arc<dyn SpanQuery>> = vec![stq("nopayload"), stq("qq"), stq("ss")];

    let span_near_query: Arc<dyn SpanQuery> = Arc::new(SpanNearQuery::new(clauses, 6, true)?);

    let clauses2: Vec<Arc<dyn SpanQuery>> = vec![stq("pp"), span_near_query];

    let snq: Arc<dyn SpanQuery> = Arc::new(SpanNearQuery::new(clauses2, 6, false)?);

    let clauses3: Vec<Arc<dyn SpanQuery>> = vec![stq("np"), snq];

    let nested_span_near_query = SpanNearQuery::new(clauses3, 6, false)?;
    let spans = leaf_spans(&nested_span_near_query, &searcher, Postings::Payloads)?;

    Assert::assert_true_msg("spans is null and it shouldn't be", spans.is_some());
    check_spans_num(spans, 1, &[3])?;
    drop(searcher);
    f.close_get_searcher()
}

fn test_heavily_nested_span_query(f: &mut PayloadSpans) -> Result<()> {
    let searcher = f.get_searcher()?;

    let clauses: Vec<Arc<dyn SpanQuery>> = vec![stq("one"), stq("two"), stq("three")];

    let span_near_query: Arc<dyn SpanQuery> = Arc::new(SpanNearQuery::new(clauses, 5, true)?);

    let clauses: Vec<Arc<dyn SpanQuery>> = vec![span_near_query, stq("five"), stq("six")];

    let span_near_query2: Arc<dyn SpanQuery> = Arc::new(SpanNearQuery::new(clauses, 6, true)?);

    let clauses2: Vec<Arc<dyn SpanQuery>> = vec![stq("eleven"), stq("ten")];
    let span_near_query3: Arc<dyn SpanQuery> = Arc::new(SpanNearQuery::new(clauses2, 2, false)?);

    let clauses3: Vec<Arc<dyn SpanQuery>> = vec![stq("nine"), span_near_query2, span_near_query3];

    let nested_span_near_query = SpanNearQuery::new(clauses3, 6, false)?;

    let spans = leaf_spans(&nested_span_near_query, &searcher, Postings::Payloads)?;
    Assert::assert_true_msg("spans is null and it shouldn't be", spans.is_some());
    check_spans_num(spans, 2, &[8, 8])?;
    drop(searcher);
    f.close_get_searcher()
}

/// The body shared by the three `testShrinkToAfterShortestMatch` methods:
/// `text` indexed with `TestPayloadAnalyzer`, the spans of `a` near `k`
/// with `slop`, and the payloads they collect.
fn shrink_payloads(text: &str, slop: i32) -> Result<HashSet<String>> {
    let directory = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_config(
        &*random(),
        directory.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(PayloadAnalyzer::new(
            "org.apache.lucene.queries.payloads.TestPayloadSpans$TestPayloadAnalyzer",
        )))),
    )?;

    let mut doc = Document::new();
    doc.add(TextField::new_with_reader(
        "content",
        Box::new(StringCharReader::new(text)),
    )?);
    writer.add_document(&doc)?;

    let reader = writer.get_reader()?;
    let is = LuceneTestCase::new_searcher_wrap(
        LuceneTestCase::get_only_leaf_reader(Arc::clone(&reader) as Arc<dyn IndexReader>)?,
        false,
    );
    writer.close()?;

    let stq1 = stq_field("content", "a");
    let stq2 = stq_field("content", "k");
    let sqs = vec![stq1, stq2];
    let snq = SpanNearQuery::new(sqs, slop, true)?;
    let mut collector = VerifyingCollector::default();
    let mut spans = non_null_spans(
        snq.create_span_weight(
            &is,
            rucene::search::score_mode::ScoreMode::COMPLETE_NO_SCORES,
            1.0,
        )?
        .get_spans(&first_leaf(&is), Postings::Payloads)?,
    );

    let top_docs = is.search(Arc::new(snq) as Arc<dyn Query>, 1)?;
    let mut payload_set: HashSet<String> = HashSet::new();
    for _i in 0..top_docs.score_docs.len() {
        while spans.next_doc()? != NO_MORE_DOCS {
            while spans.next_start_position()? != NO_MORE_POSITIONS {
                collector.reset();
                spans.collect(&mut collector)?;
                for payload in &collector.payloads {
                    payload_set.insert(Term::to_string_bytes_ref(payload));
                }
            }
        }
    }
    reader.close()?;
    directory.close()?;
    Ok(payload_set)
}

fn assert_shrunk(payload_set: &HashSet<String>) {
    Assert::assert_equals(2i64, payload_set.len() as i64);
    Assert::assert_true(payload_set.contains("a:Noise:10"));
    Assert::assert_true(payload_set.contains("k:Noise:11"));
}

fn test_shrink_to_after_shortest_match(_f: &mut PayloadSpans) -> Result<()> {
    let payload_set = shrink_payloads("a b c d e f g h i j a k", 1)?;
    assert_shrunk(&payload_set);
    Ok(())
}

fn test_shrink_to_after_shortest_match2(_f: &mut PayloadSpans) -> Result<()> {
    let payload_set = shrink_payloads("a b a d k f a h i k a k", 0)?;
    assert_shrunk(&payload_set);
    Ok(())
}

fn test_shrink_to_after_shortest_match3(_f: &mut PayloadSpans) -> Result<()> {
    let payload_set = shrink_payloads("j k a l f k k p a t a k l k t a", 0)?;
    Assert::assert_equals(2i64, payload_set.len() as i64);
    if verbose() {
        for payload in &payload_set {
            println!("match:{payload}");
        }
    }
    Assert::assert_true(payload_set.contains("a:Noise:10"));
    Assert::assert_true(payload_set.contains("k:Noise:11"));
    Ok(())
}

/// `TestPayloadSpans.VerifyingCollector`.
#[derive(Default)]
struct VerifyingCollector {
    payloads: Vec<BytesRef>,
}

impl SpanCollector for VerifyingCollector {
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

impl VerifyingCollector {
    /// `verify(int expectedLength, int expectedFirstByte)`.
    fn verify(&self, expected_length: i32, expected_first_byte: i32) {
        for payload in &self.payloads {
            Assert::assert_equals_msg(
                Some("Incorrect payload length"),
                i64::from(expected_length),
                payload.length as i64,
            );
            // `payload.bytes[0]` of the deep copy (offset 0), a signed byte
            Assert::assert_equals_msg(
                Some("Incorrect first byte"),
                i64::from(expected_first_byte),
                i64::from(payload.bytes[0] as i8),
            );
        }
    }
}

/// `checkSpans(Spans spans, int expectedNumSpans, int expectedNumPayloads,
/// int expectedPayloadLength, int expectedFirstByte)`.
fn check_spans(
    spans: Option<Box<dyn Spans>>,
    expected_num_spans: i32,
    expected_num_payloads: i32,
    expected_payload_length: i32,
    expected_first_byte: i32,
) -> Result<()> {
    Assert::assert_true_msg("spans is null and it shouldn't be", spans.is_some());
    let mut spans = non_null_spans(spans);
    // each position match should have a span associated with it, since there is just one underlying
    // term query, there should
    // only be one entry in the span
    let mut collector = VerifyingCollector::default();
    let mut seen = 0;
    while spans.next_doc()? != NO_MORE_DOCS {
        while spans.next_start_position()? != NO_MORE_POSITIONS {
            collector.reset();
            spans.collect(&mut collector)?;
            collector.verify(expected_payload_length, expected_first_byte);
            Assert::assert_equals_msg(
                Some("expectedNumPayloads"),
                i64::from(expected_num_payloads),
                collector.payloads.len() as i64,
            );
            seen += 1;
        }
    }
    Assert::assert_equals_msg(
        Some("expectedNumSpans"),
        i64::from(expected_num_spans),
        i64::from(seen),
    );
    Ok(())
}

/// `checkSpans(Spans spans, int numSpans, int[] numPayloads)`.
fn check_spans_num(
    spans: Option<Box<dyn Spans>>,
    num_spans: i32,
    num_payloads: &[i32],
) -> Result<()> {
    let mut spans = non_null_spans(spans);
    let mut cnt = 0;
    let mut collector = VerifyingCollector::default();
    while spans.next_doc()? != NO_MORE_DOCS {
        while spans.next_start_position()? != NO_MORE_POSITIONS {
            if verbose() {
                println!("\nSpans Dump --");
            }
            collector.reset();
            spans.collect(&mut collector)?;
            Assert::assert_equals_msg(
                Some("payload size"),
                i64::from(num_payloads[cnt]),
                collector.payloads.len() as i64,
            );

            cnt += 1;
        }
    }

    Assert::assert_equals_msg(Some("expected numSpans"), i64::from(num_spans), cnt as i64);
    Ok(())
}

/// `TestPayloadSpans.PayloadAnalyzer` and `TestPayloadSpans.TestPayloadAnalyzer`
/// (the same components; `java_class_name` tells them apart).
#[derive(Debug)]
struct PayloadAnalyzer {
    state: AnalyzerState,
    class_name: &'static str,
}

impl PayloadAnalyzer {
    fn new(class_name: &'static str) -> Self {
        Self {
            state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
            class_name,
        }
    }
}

impl Analyzer for PayloadAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        self.class_name
    }

    fn create_components(&self, _field_name: Option<&str>) -> Result<TokenStreamComponents> {
        let result = Arc::new(AtomicRefCell::new(MockTokenizer::new(
            Arc::clone(&SIMPLE),
            true,
        )?));
        let filter = PayloadFilter::new(shared(&result))?;
        Ok(components(result, sink(filter)))
    }
}

/// `TestPayloadSpans.PayloadFilter`.
#[derive(Debug)]
struct PayloadFilterLogic {
    entities: HashSet<String>,
    nopayload: HashSet<String>,
    pos: i32,
    payload_att: Att,
    term_att: Att,
    pos_incr_att: Att,
}

struct PayloadFilter;

impl PayloadFilter {
    /// `PayloadFilter(TokenStream input)`.
    #[allow(clippy::new_ret_no_self)]
    fn new(mut input: Box<dyn TokenStream>) -> Result<TokenFilter<PayloadFilterLogic>> {
        let mut entities = HashSet::new();
        entities.insert("xx".to_string());
        entities.insert("one".to_string());
        let mut nopayload = HashSet::new();
        nopayload.insert("nopayload".to_string());
        nopayload.insert("np".to_string());
        let source = input.attribute_source_mut();
        let term_att = Att::add(source, char_term_id())?;
        let pos_incr_att = Att::add(source, pos_inc_id())?;
        let payload_att = Att::add(source, payload_id())?;
        Ok(TokenFilter::new(
            input,
            PayloadFilterLogic {
                entities,
                nopayload,
                pos: 0,
                payload_att,
                term_att,
                pos_incr_att,
            },
        ))
    }
}

impl TokenFilterLogic for PayloadFilterLogic {
    fn increment_token(
        &mut self,
        _source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if input.increment_token()? {
            let token = self.term_att.term_string();
            if !self.nopayload.contains(&token) {
                if self.entities.contains(&token) {
                    self.payload_att
                        .payload_mut()
                        .set_payload(Some(BytesRef::from_char_sequence(&format!(
                            "{token}:Entity:{}",
                            self.pos
                        ))));
                } else {
                    self.payload_att
                        .payload_mut()
                        .set_payload(Some(BytesRef::from_char_sequence(&format!(
                            "{token}:Noise:{}",
                            self.pos
                        ))));
                }
            }
            self.pos += self.pos_incr_att.pos_inc().get_position_increment();
            return Ok(true);
        }
        Ok(false)
    }

    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.pos = 0;
        Ok(())
    }
}
