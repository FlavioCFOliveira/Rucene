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

//! Port of `TestPayloadScoreQuery.java`.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{AfterClass, Assert, BeforeClass};
use rucene::analysis::{
    Analyzer, AnalyzerState, GlobalReuseStrategy, TokenFilter, TokenFilterLogic, TokenStream,
    TokenStreamComponents,
};
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::Term;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::Query;
use rucene::search::similarities::{
    tfidf_scorer, ClassicSimilarity, CollectionStatistics, Explanation, SimScorer, Similarity,
    TFIDFSimilarity, TermStatistics,
};
use rucene::search::wildcard_query::WildcardQuery;
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::attribute::AttributeSource;
use rucene::util::BytesRef;
use rucene_queries::payloads::average_payload_function::AveragePayloadFunction;
use rucene_queries::payloads::max_payload_function::MaxPayloadFunction;
use rucene_queries::payloads::min_payload_function::MinPayloadFunction;
use rucene_queries::payloads::payload_decoder::float_decoder;
use rucene_queries::payloads::payload_function::PayloadFunction;
use rucene_queries::payloads::payload_score_query::PayloadScoreQuery;
use rucene_queries::spans::span_containing_query::SpanContainingQuery;
use rucene_queries::spans::span_multi_term_query_wrapper::SpanMultiTermQueryWrapper;
use rucene_queries::spans::span_near_query::SpanNearQuery;
use rucene_queries::spans::span_or_query::SpanOrQuery;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_queries::spans::span_term_query::SpanTermQuery;
use rucene_test_framework::analysis::attributes::{payload_id, Att};
use rucene_test_framework::analysis::mock_tokenizer::{MockTokenizer, SIMPLE};
use rucene_test_framework::analysis::{components, shared, sink};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::query_utils::QueryUtils;
use rucene_test_framework::util::english::English;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::queries_support::core_support::{class, random, test};
use crate::queries_support::search_support::{assert_query_equals, static_hook};

/// The static fields `searcher`, `reader` and `directory`.
struct Statics {
    searcher: IndexSearcher,
    reader: Arc<dyn DirectoryReader>,
    directory: TestDirectory,
}

static STATICS: Mutex<Option<Statics>> = Mutex::new(None);

/// Runs `f` on the static `searcher`.
fn with_searcher<R>(f: impl FnOnce(&mut IndexSearcher) -> R) -> R {
    let mut guard = STATICS.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut guard.as_mut().expect("beforeClass ran").searcher)
}

/// `payload2`.
const PAYLOAD2: [u8; 1] = [2];
/// `payload4`.
const PAYLOAD4: [u8; 1] = [4];

/// `TestPayloadScoreQuery`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.queries.payloads.TestPayloadScoreQuery",
        vec![
            static_hook("beforeClass", BeforeClass, before_class),
            static_hook("afterClass", AfterClass, after_class),
            test("testTermQuery", test_term_query),
            test("testOrQuery", test_or_query),
            test("testNearQuery", test_near_query),
            test("testNestedNearQuery", test_nested_near_query),
            test("testSpanContainingQuery", test_span_containing_query),
            test("testEquality", test_equality),
            test("testRewrite", test_rewrite),
        ],
    )
}

/// `checkQuery(SpanQuery query, PayloadFunction function, int[]
/// expectedDocs, float[] expectedScores)`.
fn check_query(
    query: &Arc<dyn SpanQuery>,
    function: Arc<dyn PayloadFunction>,
    expected_docs: &[i32],
    expected_scores: &[f32],
) -> Result<()> {
    check_query_with(query, function, true, expected_docs, expected_scores)
}

/// `checkQuery(SpanQuery query, PayloadFunction function, boolean
/// includeSpanScore, int[] expectedDocs, float[] expectedScores)`.
fn check_query_with(
    query: &Arc<dyn SpanQuery>,
    function: Arc<dyn PayloadFunction>,
    include_span_score: bool,
    expected_docs: &[i32],
    expected_scores: &[f32],
) -> Result<()> {
    Assert::assert_true_msg(
        "Expected docs and scores arrays must be the same length!",
        expected_docs.len() == expected_scores.len(),
    );

    let psq: Arc<dyn Query> = Arc::new(PayloadScoreQuery::new(
        Arc::clone(query),
        function,
        float_decoder(),
        include_span_score,
    ));
    with_searcher(|searcher| {
        let hits = searcher.search(Arc::clone(&psq), expected_docs.len() as i32)?;

        for i in 0..hits.score_docs.len() {
            if i as isize > expected_docs.len() as isize - 1 {
                Assert::fail(&format!(
                    "Unexpected hit in document {}",
                    hits.score_docs[i].doc
                ));
            }
            if hits.score_docs[i].doc != expected_docs[i] {
                Assert::fail(&format!(
                    "Unexpected hit in document {}",
                    hits.score_docs[i].doc
                ));
            }
            Assert::assert_equals_double_msg(
                Some(&format!("Bad score in document {}", expected_docs[i])),
                f64::from(expected_scores[i]),
                f64::from(hits.score_docs[i].score),
                0.000001,
            );
        }

        if hits.score_docs.len() > expected_docs.len() {
            Assert::fail(&format!(
                "Unexpected hit in document {}",
                hits.score_docs[expected_docs.len()]
            ));
        }

        QueryUtils::check_searcher(&*random(), &psq, Some(searcher));
        Ok(())
    })
}

fn stq(text: &str) -> Arc<dyn SpanQuery> {
    Arc::new(SpanTermQuery::new(Term::from_text("field", text)))
}

/// `new AveragePayloadFunction()`, `new MaxPayloadFunction()`, `new
/// MinPayloadFunction()`.
fn avg() -> Arc<dyn PayloadFunction> {
    Arc::new(AveragePayloadFunction::new())
}
fn max() -> Arc<dyn PayloadFunction> {
    Arc::new(MaxPayloadFunction::new())
}
fn min() -> Arc<dyn PayloadFunction> {
    Arc::new(MinPayloadFunction::new())
}

fn test_term_query() -> Result<()> {
    let q = stq("eighteen");
    for f in [avg(), max(), min()] {
        check_query(&q, f, &[118, 218, 18], &[4.0, 4.0, 2.0])?;
    }
    Ok(())
}

fn test_or_query() -> Result<()> {
    let q: Arc<dyn SpanQuery> = Arc::new(SpanOrQuery::new(vec![stq("eighteen"), stq("nineteen")])?);
    for f in [avg(), max(), min()] {
        check_query(
            &q,
            f,
            &[118, 119, 218, 219, 18, 19],
            &[4.0, 4.0, 4.0, 4.0, 2.0, 2.0],
        )?;
    }
    Ok(())
}

fn test_near_query() -> Result<()> {
    //   2     4
    // twenty two
    //  2     4      4     4
    // one hundred twenty two

    let q: Arc<dyn SpanQuery> = Arc::new(SpanNearQuery::new(
        vec![stq("twenty"), stq("two")],
        0,
        true,
    )?);

    check_query(&q, max(), &[22, 122, 222], &[4.0, 4.0, 4.0])?;
    check_query(&q, min(), &[122, 222, 22], &[4.0, 4.0, 2.0])?;
    check_query(&q, avg(), &[122, 222, 22], &[4.0, 4.0, 3.0])
}

// Java's literals, kept as written (they round to the same floats).
#[allow(clippy::excessive_precision)]
fn test_nested_near_query() -> Result<()> {
    // (one OR hundred) NEAR (twenty two) ~ 1
    //  2    4        4    4
    // one hundred twenty two
    // two hundred twenty two

    let q: Arc<dyn SpanQuery> = Arc::new(SpanNearQuery::new(
        vec![
            Arc::new(SpanOrQuery::new(vec![stq("one"), stq("hundred")])?),
            Arc::new(SpanNearQuery::new(
                vec![stq("twenty"), stq("two")],
                0,
                true,
            )?),
        ],
        1,
        true,
    )?);

    // check includeSpanScore makes a difference here
    with_searcher(|s| s.set_similarity(Arc::new(ClassicSimilarity::new())));
    let result = || -> Result<()> {
        check_query(
            &q,
            max(),
            &[122, 222],
            &[20.901256561279297, 17.06580352783203],
        )?;
        check_query(
            &q,
            min(),
            &[222, 122],
            &[17.06580352783203, 10.450628280639648],
        )?;
        check_query(
            &q,
            avg(),
            &[122, 222],
            &[19.15948486328125, 17.06580352783203],
        )?;
        check_query_with(&q, max(), false, &[122, 222], &[4.0, 4.0])?;
        check_query_with(&q, min(), false, &[222, 122], &[4.0, 2.0])?;
        check_query_with(&q, avg(), false, &[222, 122], &[4.0, 3.666666])
    };
    let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(result));
    // finally
    with_searcher(|s| s.set_similarity(Arc::new(JustScorePayloadSimilarity)));
    match run {
        Ok(r) => r,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

fn test_span_containing_query() -> Result<()> {
    // twenty WITHIN ((one OR hundred) NEAR two)~2
    let q: Arc<dyn SpanQuery> = Arc::new(SpanContainingQuery::new(
        Arc::new(SpanNearQuery::new(
            vec![
                Arc::new(SpanOrQuery::new(vec![stq("one"), stq("hundred")])?),
                stq("two"),
            ],
            2,
            true,
        )?),
        stq("twenty"),
    )?);

    check_query(&q, avg(), &[222, 122], &[4.0, 3.666666])?;
    check_query(&q, max(), &[122, 222], &[4.0, 4.0])?;
    check_query(&q, min(), &[222, 122], &[4.0, 2.0])
}

fn test_equality() -> Result<()> {
    let sq1 = stq("one");
    let sq2 = stq("two");
    let min_func = min();
    let max_func = max();
    let query1: Arc<dyn Query> = Arc::new(PayloadScoreQuery::new(
        Arc::clone(&sq1),
        Arc::clone(&min_func),
        float_decoder(),
        true,
    ));
    let query2: Arc<dyn Query> = Arc::new(PayloadScoreQuery::new(
        Arc::clone(&sq2),
        Arc::clone(&min_func),
        float_decoder(),
        true,
    ));
    let query3: Arc<dyn Query> = Arc::new(PayloadScoreQuery::new(
        Arc::clone(&sq2),
        Arc::clone(&max_func),
        float_decoder(),
        true,
    ));
    let query4: Arc<dyn Query> = Arc::new(PayloadScoreQuery::new(
        Arc::clone(&sq2),
        Arc::clone(&max_func),
        float_decoder(),
        false,
    ));
    let query5: Arc<dyn Query> = Arc::new(PayloadScoreQuery::with_span_score(
        sq1,
        min_func,
        float_decoder(),
    ));

    assert_query_equals(&*query1, &*query5);
    Assert::assert_false(query1.query_eq(&*query2));
    Assert::assert_false(query1.query_eq(&*query3));
    Assert::assert_false(query1.query_eq(&*query4));
    Assert::assert_false(query2.query_eq(&*query3));
    Assert::assert_false(query2.query_eq(&*query4));
    Assert::assert_false(query3.query_eq(&*query4));
    Ok(())
}

fn test_rewrite() -> Result<()> {
    let xyz: Arc<dyn SpanQuery> = Arc::new(SpanMultiTermQueryWrapper::new(Arc::new(
        WildcardQuery::new(Term::from_text("field", "xyz*"))?,
    )));
    let psq: Arc<dyn Query> = Arc::new(PayloadScoreQuery::new(xyz, avg(), float_decoder(), false));

    // if query wasn't rewritten properly, the query would have failed with "Rewrite first!"
    with_searcher(|s| s.search(psq, 1))?;
    Ok(())
}

/// `TestPayloadScoreQuery.PayloadAnalyzer`.
#[derive(Debug)]
struct PayloadAnalyzer {
    state: AnalyzerState,
}

impl PayloadAnalyzer {
    fn new() -> Self {
        Self {
            state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
        }
    }
}

impl Analyzer for PayloadAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.queries.payloads.TestPayloadScoreQuery$PayloadAnalyzer"
    }

    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        let result = Arc::new(AtomicRefCell::new(MockTokenizer::new(
            Arc::clone(&SIMPLE),
            true,
        )?));
        let mut input = shared(&result);
        let pay_att = Att::add(input.attribute_source_mut(), payload_id())?;
        let filter = TokenFilter::new(
            input,
            PayloadFilter {
                num_seen: 0,
                pay_att,
            },
        );
        Ok(components(result, sink(filter)))
    }
}

/// `TestPayloadScoreQuery.PayloadFilter`.
#[derive(Debug)]
struct PayloadFilter {
    num_seen: i32,
    pay_att: Att,
}

impl TokenFilterLogic for PayloadFilter {
    fn increment_token(
        &mut self,
        _source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        let mut result = false;
        if input.increment_token()? {
            if self.num_seen % 4 == 0 {
                self.pay_att
                    .payload_mut()
                    .set_payload(Some(BytesRef::new(PAYLOAD2.to_vec())));
            } else {
                self.pay_att
                    .payload_mut()
                    .set_payload(Some(BytesRef::new(PAYLOAD4.to_vec())));
            }
            self.num_seen += 1;
            result = true;
        }
        Ok(result)
    }

    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.num_seen = 0;
        Ok(())
    }
}

fn before_class() -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    let config =
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(PayloadAnalyzer::new())));
    config.set_merge_policy(Some(NoMergePolicy::instance()))?;
    let writer = RandomIndexWriter::new_with_config(&*random(), directory.as_directory(), config)?;
    // writer.infoStream = System.out;
    for i in 0..300 {
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_text_field(
            "field",
            &English::int_to_english(i),
            Store::YES,
        )));
        let txt = format!(
            "{} {}",
            English::int_to_english(i),
            English::int_to_english(i + 1)
        );
        doc.add(Box::new(LuceneTestCase::new_text_field(
            "field2",
            &txt,
            Store::YES,
        )));
        writer.add_document(&doc)?;
    }
    let reader = writer.get_reader()?;
    writer.close()?;

    let mut searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    searcher.set_similarity(Arc::new(JustScorePayloadSimilarity));
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
        drop(searcher);
        reader.close()?;
        directory.close()?;
    }
    Ok(())
}

/// `TestPayloadScoreQuery.JustScorePayloadSimilarity extends
/// ClassicSimilarity`: makes everything else 1 so we see the effect of the
/// payload.
#[derive(Debug, Clone, Copy, Default)]
pub struct JustScorePayloadSimilarity;

impl Similarity for JustScorePayloadSimilarity {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.queries.payloads.TestPayloadScoreQuery$JustScorePayloadSimilarity"
    }

    /// Inherited `ClassicSimilarity.toString()`.
    fn to_java_string(&self) -> String {
        "ClassicSimilarity".to_string()
    }

    fn as_tfidf_similarity(&self) -> Option<&dyn TFIDFSimilarity> {
        Some(self)
    }

    fn scorer<'a>(
        &'a self,
        boost: f32,
        collection_stats: &CollectionStatistics,
        term_stats: &[TermStatistics],
    ) -> Box<dyn SimScorer + 'a> {
        tfidf_scorer(self, boost, collection_stats, term_stats)
    }
}

impl TFIDFSimilarity for JustScorePayloadSimilarity {
    // !!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!
    // Make everything else 1 so we see the effect of the payload
    // !!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!
    fn length_norm(&self, _length: i32) -> f32 {
        1.0
    }

    fn tf(&self, _freq: f32) -> f32 {
        1.0
    }

    /// Inherited `ClassicSimilarity.idf`.
    fn idf(&self, doc_freq: i64, doc_count: i64) -> f32 {
        ClassicSimilarity::new().idf(doc_freq, doc_count)
    }

    // idf used for phrase queries
    fn idf_explain_phrase(
        &self,
        _collection_stats: &CollectionStatistics,
        _term_stats: &[TermStatistics],
    ) -> Explanation {
        Explanation::matched(1.0f32, "Inexplicable", vec![])
    }

    fn idf_explain(
        &self,
        _collection_stats: &CollectionStatistics,
        _term_stats: &TermStatistics,
    ) -> Explanation {
        Explanation::matched(1.0f32, "Inexplicable", vec![])
    }
}
