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

//! Port of `TestPayloadTermQuery.java`.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{AfterClass, Assert, BeforeClass};
use rucene::analysis::{
    Analyzer, AnalyzerState, PerFieldReuseStrategy, TokenFilter, TokenFilterLogic, TokenStream,
    TokenStreamComponents,
};
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::{IndexReader, Term};
use rucene::search::boolean_clause::{BooleanClause, Occur};
use rucene::search::boolean_query::Builder as BooleanQueryBuilder;
use rucene::search::doc_id_set_iterator::NO_MORE_DOCS;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::Query;
use rucene::search::score_mode::ScoreMode;
use rucene::search::similarities::{
    tfidf_scorer, CollectionStatistics, Explanation, SimScorer, Similarity, TFIDFSimilarity,
    TermStatistics,
};
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::attribute::AttributeSource;
use rucene::util::BytesRef;
use rucene_queries::payloads::average_payload_function::AveragePayloadFunction;
use rucene_queries::payloads::max_payload_function::MaxPayloadFunction;
use rucene_queries::payloads::payload_decoder::float_decoder;
use rucene_queries::payloads::payload_score_query::PayloadScoreQuery;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_queries::spans::span_term_query::SpanTermQuery;
use rucene_queries::spans::span_weight::Postings;
use rucene_queries::spans::spans::NO_MORE_POSITIONS;
use rucene_test_framework::analysis::attributes::{payload_id, Att};
use rucene_test_framework::analysis::mock_tokenizer::{MockTokenizer, SIMPLE};
use rucene_test_framework::analysis::{components, shared, sink};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::check_hits::CheckHits;
use rucene_test_framework::search::query_utils::QueryUtils;
use rucene_test_framework::util::english::English;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::queries_support::core_support::{class, random, test};
use crate::queries_support::payload_helper;
use crate::queries_support::search_support::static_hook;
use crate::queries_support::{first_leaf, non_null_spans};

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

/// `payloadField`.
const PAYLOAD_FIELD: [u8; 1] = [1];
/// `payloadMultiField1`.
const PAYLOAD_MULTI_FIELD1: [u8; 1] = [2];
/// `payloadMultiField2`.
const PAYLOAD_MULTI_FIELD2: [u8; 1] = [4];

/// `TestPayloadTermQuery`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.queries.payloads.TestPayloadTermQuery",
        vec![
            static_hook("beforeClass", BeforeClass, before_class),
            static_hook("afterClass", AfterClass, after_class),
            test("test", test_test),
            test("testQuery", test_query),
            test("testMultipleMatchesPerDoc", test_multiple_matches_per_doc),
            test("testNoMatch", test_no_match),
            test("testNoPayload", test_no_payload),
        ],
    )
}

/// `TestPayloadTermQuery.PayloadAnalyzer`.
#[derive(Debug)]
struct PayloadAnalyzer {
    state: AnalyzerState,
}

impl PayloadAnalyzer {
    /// `super(PER_FIELD_REUSE_STRATEGY)`.
    fn new() -> Self {
        Self {
            state: AnalyzerState::new(Box::new(PerFieldReuseStrategy)),
        }
    }
}

impl Analyzer for PayloadAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.queries.payloads.TestPayloadTermQuery$PayloadAnalyzer"
    }

    fn create_components(&self, field_name: &str) -> Result<TokenStreamComponents> {
        let result = Arc::new(AtomicRefCell::new(MockTokenizer::new(
            Arc::clone(&SIMPLE),
            true,
        )?));
        let mut input = shared(&result);
        let payload_att = Att::add(input.attribute_source_mut(), payload_id())?;
        let filter = TokenFilter::new(
            input,
            PayloadFilter {
                field_name: field_name.to_string(),
                num_seen: 0,
                payload_att,
            },
        );
        Ok(components(result, sink(filter)))
    }
}

/// `TestPayloadTermQuery.PayloadFilter`.
#[derive(Debug)]
struct PayloadFilter {
    field_name: String,
    num_seen: i32,
    payload_att: Att,
}

impl TokenFilterLogic for PayloadFilter {
    fn increment_token(
        &mut self,
        _source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        let has_next = input.increment_token()?;
        if has_next {
            if self.field_name == "field" {
                self.payload_att
                    .payload_mut()
                    .set_payload(Some(BytesRef::new(PAYLOAD_FIELD.to_vec())));
            } else if self.field_name == "multiField" {
                if self.num_seen % 2 == 0 {
                    self.payload_att
                        .payload_mut()
                        .set_payload(Some(BytesRef::new(PAYLOAD_MULTI_FIELD1.to_vec())));
                } else {
                    self.payload_att
                        .payload_mut()
                        .set_payload(Some(BytesRef::new(PAYLOAD_MULTI_FIELD2.to_vec())));
                }
                self.num_seen += 1;
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.num_seen = 0;
        Ok(())
    }
}

/// `similarity` (`private static final`).
fn similarity() -> Arc<dyn Similarity> {
    Arc::new(BoostingSimilarity)
}

fn before_class() -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    let config =
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(PayloadAnalyzer::new())));
    config.set_similarity(Some(similarity()))?;
    config.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let writer = RandomIndexWriter::new_with_config(&*random(), directory.as_directory(), config)?;
    // writer.infoStream = System.out;
    for i in 0..1000 {
        let mut doc = Document::new();
        let no_payload_field = LuceneTestCase::new_text_field(
            payload_helper::NO_PAYLOAD_FIELD,
            &English::int_to_english(i),
            Store::YES,
        );
        // noPayloadField.setBoost(0);
        doc.add(Box::new(no_payload_field));
        doc.add(Box::new(LuceneTestCase::new_text_field(
            "field",
            &English::int_to_english(i),
            Store::YES,
        )));
        doc.add(Box::new(LuceneTestCase::new_text_field(
            "multiField",
            &format!(
                "{}  {}",
                English::int_to_english(i),
                English::int_to_english(i)
            ),
            Store::YES,
        )));
        writer.add_document(&doc)?;
    }
    writer.force_merge(1)?;
    let reader = writer.get_reader()?;
    writer.close()?;

    let mut searcher = LuceneTestCase::new_searcher(LuceneTestCase::get_only_leaf_reader(
        Arc::clone(&reader) as Arc<dyn IndexReader>,
    )?);
    searcher.set_similarity(similarity());
    *STATICS.lock().unwrap_or_else(|e| e.into_inner()) = Some(Statics {
        searcher: Arc::new(searcher),
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

fn psq_max(field: &str, text: &str) -> Arc<dyn SpanQuery> {
    Arc::new(PayloadScoreQuery::with_span_score(
        Arc::new(SpanTermQuery::new(Term::from_text(field, text))),
        Arc::new(MaxPayloadFunction::new()),
        float_decoder(),
    ))
}

/// Java's `test()`.
fn test_test() -> Result<()> {
    let query = psq_max("field", "seventy");
    let searcher = searcher();
    let hits = searcher.search(Arc::clone(&query) as Arc<dyn Query>, 100)?;
    Assert::assert_true_msg("hits is null and it shouldn't be", true);
    Assert::assert_true_msg(
        &format!("hits Size: {} is not: 100", hits.total_hits.value()),
        hits.total_hits.value() == 100,
    );

    // they should all have the exact same score, because they all contain seventy once, and we set
    // all the other similarity factors to be 1

    for i in 0..hits.score_docs.len() {
        let doc = &hits.score_docs[i];
        Assert::assert_true_msg(
            &format!(
                "{} does not equal: 1",
                rucene::util::java_fmt::float_to_string(doc.score)
            ),
            doc.score == 1.0,
        );
    }
    CheckHits::check_explanations_deep(
        &(Arc::clone(&query) as Arc<dyn Query>),
        Some(payload_helper::FIELD),
        &searcher,
        true,
    )?;
    let spans = query
        .create_span_weight(&searcher, ScoreMode::COMPLETE_NO_SCORES, 1.0)?
        .get_spans(&first_leaf(&searcher), Postings::Positions)?;
    Assert::assert_true_msg("spans is null and it shouldn't be", spans.is_some());
    /*float score = hits.score(0);
    for (int i =1; i < hits.length(); i++)
    {
      assertTrue("scores are not equal and they should be", score == hits.score(i));
    }*/
    Ok(())
}

fn test_query() -> Result<()> {
    let boosting_func_term_query = psq_max(payload_helper::MULTI_FIELD, "seventy");
    QueryUtils::check(&*boosting_func_term_query);

    let span_term_query: Arc<dyn Query> = Arc::new(SpanTermQuery::new(Term::from_text(
        payload_helper::MULTI_FIELD,
        "seventy",
    )));

    Assert::assert_true(
        boosting_func_term_query.query_eq(&*span_term_query)
            == span_term_query.query_eq(&*boosting_func_term_query as &dyn Query),
    );

    let boosting_func_term_query2: Arc<dyn SpanQuery> =
        Arc::new(PayloadScoreQuery::with_span_score(
            Arc::new(SpanTermQuery::new(Term::from_text(
                payload_helper::MULTI_FIELD,
                "seventy",
            ))),
            Arc::new(AveragePayloadFunction::new()),
            float_decoder(),
        ));

    QueryUtils::check_unequal(&*boosting_func_term_query, &*boosting_func_term_query2);
    Ok(())
}

fn test_multiple_matches_per_doc() -> Result<()> {
    let query = psq_max(payload_helper::MULTI_FIELD, "seventy");
    let searcher = searcher();
    let hits = searcher.search(Arc::clone(&query) as Arc<dyn Query>, 100)?;
    Assert::assert_true_msg("hits is null and it shouldn't be", true);
    Assert::assert_true_msg(
        &format!("hits Size: {} is not: 100", hits.total_hits.value()),
        hits.total_hits.value() == 100,
    );

    // they should all have the exact same score, because they all contain seventy once, and we set
    // all the other similarity factors to be 1

    // System.out.println("Hash: " + seventyHash + " Twice Hash: " + 2*seventyHash);
    // there should be exactly 10 items that score a 4, all the rest should score a 2
    // The 10 items are: 70 + i*100 where i in [0-9]
    let mut num_tens = 0;
    for i in 0..hits.score_docs.len() {
        let doc = &hits.score_docs[i];
        let score = rucene::util::java_fmt::float_to_string(doc.score);
        if doc.doc % 10 == 0 {
            num_tens += 1;
            Assert::assert_true_msg(
                &format!("{score} does not equal: 4.0"),
                f64::from(doc.score) == 4.0,
            );
        } else {
            Assert::assert_true_msg(&format!("{score} does not equal: 2"), doc.score == 2.0);
        }
    }
    Assert::assert_true_msg(&format!("{num_tens} does not equal: 10"), num_tens == 10);
    CheckHits::check_explanations_deep(
        &(Arc::clone(&query) as Arc<dyn Query>),
        Some("field"),
        &searcher,
        true,
    )?;
    let spans = query
        .create_span_weight(&searcher, ScoreMode::COMPLETE_NO_SCORES, 1.0)?
        .get_spans(&first_leaf(&searcher), Postings::Positions)?;
    Assert::assert_true_msg("spans is null and it shouldn't be", spans.is_some());
    let mut spans = non_null_spans(spans);
    // should be two matches per document
    let mut count = 0;
    // 100 hits times 2 matches per hit, we should have 200 in count
    while spans.next_doc()? != NO_MORE_DOCS {
        while spans.next_start_position()? != NO_MORE_POSITIONS {
            count += 1;
        }
    }
    Assert::assert_true_msg(&format!("{count} does not equal: 200"), count == 200);
    Ok(())
}

fn test_no_match() -> Result<()> {
    let query = psq_max(payload_helper::FIELD, "junk");
    let hits = searcher().search(query as Arc<dyn Query>, 100)?;
    Assert::assert_true_msg("hits is null and it shouldn't be", true);
    Assert::assert_true_msg(
        &format!("hits Size: {} is not: 0", hits.total_hits.value()),
        hits.total_hits.value() == 0,
    );
    Ok(())
}

fn test_no_payload() -> Result<()> {
    let q1 = psq_max(payload_helper::NO_PAYLOAD_FIELD, "zero");
    let q2 = psq_max(payload_helper::NO_PAYLOAD_FIELD, "foo");
    let c1 = BooleanClause::new(q1 as Arc<dyn Query>, Occur::MUST);
    let c2 = BooleanClause::new(q2 as Arc<dyn Query>, Occur::MUST_NOT);
    let mut query = BooleanQueryBuilder::new();
    query.add_clause(c1)?;
    query.add_clause(c2)?;
    let searcher = searcher();
    let hits = searcher.search(Arc::new(query.build()), 100)?;
    Assert::assert_true_msg("hits is null and it shouldn't be", true);
    Assert::assert_true_msg(
        &format!("hits Size: {} is not: 1", hits.total_hits.value()),
        hits.total_hits.value() == 1,
    );
    let results = [0]; // hits.scoreDocs[0].doc;
    CheckHits::check_hit_collector(
        &*random(),
        &(Arc::new(query.build()) as Arc<dyn Query>),
        Some(payload_helper::NO_PAYLOAD_FIELD),
        &searcher,
        &results,
    )
}

/// `TestPayloadTermQuery.BoostingSimilarity extends ClassicSimilarity`:
/// makes everything else 1 so we see the effect of the payload.
#[derive(Debug, Clone, Copy, Default)]
pub struct BoostingSimilarity;

impl Similarity for BoostingSimilarity {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.queries.payloads.TestPayloadTermQuery$BoostingSimilarity"
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

impl TFIDFSimilarity for BoostingSimilarity {
    // !!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!
    // Make everything else 1 so we see the effect of the payload
    // !!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!
    fn length_norm(&self, _length: i32) -> f32 {
        1.0
    }

    fn idf(&self, _doc_freq: i64, _doc_count: i64) -> f32 {
        1.0
    }

    fn tf(&self, freq: f32) -> f32 {
        if freq == 0.0 {
            0.0
        } else {
            1.0
        }
    }

    /// Inherited `ClassicSimilarity.idfExplain(CollectionStatistics,
    /// TermStatistics)`, over this class's `idf`.
    fn idf_explain(
        &self,
        collection_stats: &CollectionStatistics,
        term_stats: &TermStatistics,
    ) -> Explanation {
        let df = term_stats.doc_freq();
        let doc_count = collection_stats.doc_count();
        let idf = self.idf(df, doc_count);
        Explanation::matched(
            idf,
            "idf, computed as log((docCount+1)/(docFreq+1)) + 1 from:",
            vec![
                Explanation::matched(df, "docFreq, number of documents containing term", vec![]),
                Explanation::matched(
                    doc_count,
                    "docCount, total number of documents with field",
                    vec![],
                ),
            ],
        )
    }
}
