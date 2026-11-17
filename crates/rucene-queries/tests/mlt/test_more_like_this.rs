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

//! Port of `TestMoreLikeThis.java`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::{
    Analyzer, AnalyzerState, GlobalReuseStrategy, StringCharReader, TokenFilter, TokenFilterLogic,
    TokenStream, TokenStreamComponents,
};
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::{IndexReader, Term};
use rucene::search::boolean_clause::{BooleanClause, Occur};
use rucene::search::boolean_query::BooleanQuery;
use rucene::search::boost_query::BoostQuery;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::Query;
use rucene::search::term_query::TermQuery;
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::attribute::AttributeSource;
use rucene::util::ArrayUtil;
use rucene_queries::mlt::more_like_this::MoreLikeThis;
use rucene_queries::mlt::more_like_this_query::MoreLikeThisQuery;
use rucene_test_framework::analysis::attributes::{char_term_id, term_freq_id, Att};
use rucene_test_framework::analysis::mock_token_filter::{MockTokenFilter, EMPTY_STOPSET};
use rucene_test_framework::analysis::mock_tokenizer::{MockTokenizer, KEYWORD, WHITESPACE};
use rucene_test_framework::analysis::{components, shared, sink, MockAnalyzer};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::query_utils::QueryUtils;
use rucene_test_framework::util::lucene_test_case::{AwaitsFix, TestDirectory};
use rucene_test_framework::util::LuceneTestCase;

use crate::queries_support::core_support::random;
use crate::queries_support::search_support::{fixture_class, ftest, set_up, tear_down};

const SHOP_TYPE: &str = "type";
const FOR_SALE: &str = "weSell";
const NOT_FOR_SALE: &str = "weDontSell";

const MIN_DOC_FREQ: i32 = 0;
const MIN_WORD_LEN: i32 = 1;
const MIN_TERM_FREQ: i32 = 1;

/// The instance of `TestMoreLikeThis`.
#[derive(Default)]
pub struct MoreLikeThisTest {
    directory: Option<TestDirectory>,
    reader: Option<Arc<dyn IndexReader>>,
    searcher: Option<IndexSearcher>,
    mlt: Option<MoreLikeThis>,
    analyzer: Option<Arc<dyn Analyzer>>,
}

impl MoreLikeThisTest {
    fn mlt(&mut self) -> &mut MoreLikeThis {
        self.mlt.as_mut().expect("setUp ran")
    }
}

/// `TestMoreLikeThis`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<MoreLikeThisTest>(
        "org.apache.lucene.queries.mlt.TestMoreLikeThis",
        vec![
            set_up(do_set_up),
            tear_down(do_tear_down),
            ftest("testSmallSampleFromCorpus", test_small_sample_from_corpus),
            ftest("testBoostFactor", test_boost_factor),
            ftest("testMultiFields", test_multi_fields),
            ftest("testMultiValues", test_multi_values),
            ftest(
                "testSeedDocumentMap_minTermFrequencySet_shouldBuildQueryAccordingToCorrectTermFrequencies",
                test_seed_document_map_min_term_frequency_set,
            ),
            ftest(
                "testSeedDocumentMap_minTermFrequencySetMltFieldSet_shouldBuildQueryAccordingToCorrectTermFrequenciesAndField",
                test_seed_document_map_min_term_frequency_set_mlt_field_set,
            ),
            ftest(
                "testSeedDocumentMap_queryFieldsSet_shouldBuildQueryFromSpecifiedFieldnamesOnly",
                test_seed_document_map_query_fields_set,
            ),
            ftest("testMoreLikeThisQuery", test_more_like_this_query),
            ftest("testTopN", test_top_n),
            ftest(
                "testMultiFieldShouldReturnPerFieldBooleanQuery",
                test_multi_field_should_return_per_field_boolean_query,
            )
            .annotate(AwaitsFix {
                bug_url: "https://issues.apache.org/jira/browse/LUCENE-7161".to_string(),
            }),
            ftest("testCustomFrequency", test_custom_frequency),
        ],
    )
}

fn whitespace_analyzer() -> Arc<dyn Analyzer> {
    Arc::new(MockAnalyzer::with_automaton(
        &*random(),
        Arc::clone(&WHITESPACE),
        false,
    ))
}

fn do_set_up(f: &mut MoreLikeThisTest) -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), directory.as_directory())?;

    // Add series of docs with specific information for MoreLikeThis
    add_doc(&writer, "text", "lucene")?;
    add_doc(&writer, "text", "lucene release")?;
    add_doc(&writer, "text", "apache")?;
    add_doc(&writer, "text", "apache lucene")?;

    // one more time to increase the doc frequencies
    add_doc(&writer, "text", "lucene2")?;
    add_doc(&writer, "text", "lucene2 release2")?;
    add_doc(&writer, "text", "apache2")?;
    add_doc(&writer, "text", "apache2 lucene2")?;

    add_doc(&writer, "text2", "lucene2")?;
    add_doc(&writer, "text2", "lucene2 release2")?;
    add_doc(&writer, "text2", "apache2")?;
    add_doc(&writer, "text2", "apache2 lucene2")?;

    let directory_reader = writer.get_reader()?;
    writer.close()?;
    f.searcher = Some(LuceneTestCase::new_searcher(Arc::clone(&directory_reader)));
    let reader: Arc<dyn IndexReader> = directory_reader;
    f.analyzer = Some(whitespace_analyzer());
    f.mlt = Some(get_default_more_like_this(Arc::clone(&reader)));
    f.reader = Some(reader);
    f.directory = Some(directory);
    Ok(())
}

/// `getDefaultMoreLikeThis(IndexReader)`.
fn get_default_more_like_this(reader: Arc<dyn IndexReader>) -> MoreLikeThis {
    let mut mlt = MoreLikeThis::new(reader);
    let analyzer = whitespace_analyzer();
    mlt.set_analyzer(Some(analyzer));
    mlt.set_min_doc_freq(MIN_DOC_FREQ);
    mlt.set_min_term_freq(MIN_TERM_FREQ);
    mlt.set_min_word_len(MIN_WORD_LEN);
    mlt
}

fn do_tear_down(f: &mut MoreLikeThisTest) -> Result<()> {
    f.analyzer.take().expect("setUp ran").close();
    f.mlt = None;
    f.searcher = None;
    f.reader.take().expect("setUp ran").close()?;
    f.directory.take().expect("setUp ran").close()
}

fn add_doc(writer: &RandomIndexWriter, field_name: &str, text: &str) -> Result<()> {
    let mut doc = Document::new();
    doc.add(LuceneTestCase::new_text_field(field_name, text, Store::YES));
    writer.add_document(&doc)?;
    Ok(())
}

fn add_doc_texts(writer: &RandomIndexWriter, field_name: &str, texts: &[String]) -> Result<()> {
    let mut doc = Document::new();
    for text in texts {
        doc.add(LuceneTestCase::new_text_field(field_name, text, Store::YES));
    }
    writer.add_document(&doc)?;
    Ok(())
}

/// `new StringReader(text)`.
fn reader_of(text: &str) -> Box<dyn rucene::analysis::CharReader> {
    Box::new(StringCharReader::new(text))
}

/// `(BooleanQuery) query`.
fn as_boolean(query: &Arc<dyn Query>) -> &BooleanQuery {
    query
        .as_any()
        .downcast_ref::<BooleanQuery>()
        .unwrap_or_else(|| class_cast("BooleanQuery"))
}

/// `(TermQuery) query`.
fn as_term_query(query: &Arc<dyn Query>) -> &TermQuery {
    query
        .as_any()
        .downcast_ref::<TermQuery>()
        .unwrap_or_else(|| class_cast("TermQuery"))
}

/// `(BoostQuery) query`.
fn as_boost_query(query: &Arc<dyn Query>) -> &BoostQuery {
    query
        .as_any()
        .downcast_ref::<BoostQuery>()
        .unwrap_or_else(|| class_cast("BoostQuery"))
}

fn class_cast(what: &str) -> ! {
    randomizedtesting::jdk::exception::Throwable::new(
        "java.lang.ClassCastException",
        format!("not a {what}"),
    )
    .throw()
}

fn names(fields: &[&str]) -> Option<Vec<String>> {
    Some(fields.iter().map(|s| s.to_string()).collect())
}

fn test_small_sample_from_corpus(_f: &mut MoreLikeThisTest) -> Result<()> {
    // add series of docs with terms of decreasing df
    let dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    for _ in 0..1980 {
        add_doc(&writer, "text", "filler")?;
    }
    for _ in 0..18 {
        add_doc(&writer, "one_percent", "all")?;
    }
    for _ in 0..2 {
        let mut doc = Document::new();
        doc.add(LuceneTestCase::new_text_field(
            "one_percent",
            "all",
            Store::YES,
        ));
        doc.add(LuceneTestCase::new_text_field(
            "one_percent",
            "tenth",
            Store::YES,
        ));
        writer.add_document(&doc)?;
    }
    let reader: Arc<dyn IndexReader> = writer.get_reader()?;
    writer.close()?;

    // setup MLT query
    let mut mlt = MoreLikeThis::new(Arc::clone(&reader));
    let analyzer = whitespace_analyzer();
    mlt.set_analyzer(Some(Arc::clone(&analyzer)));
    mlt.set_max_query_terms(3);
    mlt.set_min_doc_freq(1);
    mlt.set_min_term_freq(1);
    mlt.set_min_word_len(1);
    mlt.set_field_names(names(&["one_percent"]));

    let q = mlt.like_readers("one_percent", vec![reader_of("tenth tenth all")])?;
    let clauses = as_boolean(&q).clauses();

    Assert::assert_true(clauses.len() == 2);
    let term = as_term_query(clauses[0].query()).get_term();
    Assert::assert_true(term.text() == "all");
    let term = as_term_query(clauses[1].query()).get_term();
    Assert::assert_true(term.text() == "tenth");

    let q = mlt.like_readers("one_percent", vec![reader_of("tenth all all")])?;
    let clauses = as_boolean(&q).clauses();

    Assert::assert_true(clauses.len() == 2);
    let term = as_term_query(clauses[0].query()).get_term();
    Assert::assert_true(term.text() == "all");
    let term = as_term_query(clauses[1].query()).get_term();
    Assert::assert_true(term.text() == "tenth");

    // clean up
    reader.close()?;
    dir.close()?;
    analyzer.close();
    Ok(())
}

fn test_boost_factor(f: &mut MoreLikeThisTest) -> Result<()> {
    let original_values = get_original_values(f)?;
    f.mlt().set_field_names(names(&["text"]));
    f.mlt().set_boost(true);

    // this mean that every term boost factor will be multiplied by this
    // number
    let boost_factor = 5.0f32;
    f.mlt().set_boost_factor(boost_factor);

    let q = f
        .mlt()
        .like_readers("text", vec![reader_of("lucene release")])?;
    let clauses = as_boolean(&q).clauses();

    Assert::assert_equals_msg(
        Some(&format!("Expected {} clauses.", original_values.len())),
        original_values.len(),
        clauses.len(),
    );

    for clause in clauses {
        let bq = as_boost_query(clause.query());
        let inner = bq.get_query();
        let tq = as_term_query(&inner);
        let text = tq.get_term().text();
        let term_boost = original_values.get(&text).copied();
        Assert::assert_true_msg(&format!("Expected term {text}"), term_boost.is_some());

        let total_boost = term_boost.expect("assertNotNull") * boost_factor;
        Assert::assert_equals_double_msg(
            Some(&format!(
                "Expected boost of {} for term '{text}' got {}",
                rucene::util::java_fmt::float_to_string(total_boost),
                rucene::util::java_fmt::float_to_string(bq.get_boost())
            )),
            f64::from(total_boost),
            f64::from(bq.get_boost()),
            0.0001,
        );
    }
    Ok(())
}

/// `getOriginalValues()`.
fn get_original_values(f: &mut MoreLikeThisTest) -> Result<HashMap<String, f32>> {
    let mut original_values = HashMap::new();
    f.mlt().set_field_names(names(&["text"]));
    f.mlt().set_boost(true);
    let q = f
        .mlt()
        .like_readers("text", vec![reader_of("lucene release")])?;
    for clause in as_boolean(&q).clauses() {
        let bq = as_boost_query(clause.query());
        let inner = bq.get_query();
        let tq = as_term_query(&inner);
        original_values.insert(tq.get_term().text(), bq.get_boost());
    }
    Ok(original_values)
}

// LUCENE-3326
fn test_multi_fields(f: &mut MoreLikeThisTest) -> Result<()> {
    f.mlt().set_field_names(names(&["text", "foobar"]));
    f.mlt()
        .like_readers("foobar", vec![reader_of("this is a test")])?;
    Ok(())
}

// LUCENE-5725
fn test_multi_values(f: &mut MoreLikeThisTest) -> Result<()> {
    let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::with_automaton(
        &*random(),
        Arc::clone(&KEYWORD),
        false,
    ));
    f.mlt().set_analyzer(Some(analyzer));
    f.mlt().set_field_names(names(&["text"]));

    let q = f.mlt().like_readers(
        "text",
        vec![
            reader_of("lucene"),
            reader_of("lucene release"),
            reader_of("apache"),
            reader_of("apache lucene"),
        ],
    )?;
    let clauses = as_boolean(&q).clauses();
    Assert::assert_equals_msg(Some("Expected 2 clauses only!"), 2usize, clauses.len());
    let expected = [
        Term::from_text("text", "lucene"),
        Term::from_text("text", "apache"),
    ];
    for clause in clauses {
        let term = as_term_query(clause.query()).get_term();
        Assert::assert_true(expected.contains(term));
    }
    Ok(())
}

/// `Arrays.asList(textValue)` as the values of a seed document field.
fn values(text: &str) -> Vec<Option<String>> {
    vec![Some(text.to_string())]
}

fn test_seed_document_map_min_term_frequency_set(f: &mut MoreLikeThisTest) -> Result<()> {
    f.mlt().set_min_term_freq(3);

    let mlt_field1 = "text";
    let mlt_field2 = "text2";
    f.mlt().set_field_names(names(&[mlt_field1, mlt_field2]));

    let mut seed_document: HashMap<String, Vec<Option<String>>> = HashMap::new();
    let text_value = "apache apache lucene lucene lucene";
    seed_document.insert(mlt_field1.to_string(), values(text_value));
    seed_document.insert(mlt_field2.to_string(), values(text_value));

    let q = f.mlt().like_document(&seed_document)?;
    let clauses = as_boolean(&q).clauses();
    Assert::assert_equals_msg(Some("Expected 1 clauses only!"), 1usize, clauses.len());
    for clause in clauses {
        let term = as_term_query(clause.query()).get_term();
        Assert::assert_true_msg(
            &format!("expected:<text:lucene> but was:<{term}>"),
            &Term::from_text(mlt_field1, "lucene") == term,
        );
    }
    f.analyzer.as_ref().expect("setUp ran").close();
    Ok(())
}

fn test_seed_document_map_min_term_frequency_set_mlt_field_set(
    f: &mut MoreLikeThisTest,
) -> Result<()> {
    f.mlt().set_min_term_freq(3);

    let mlt_field = "text";
    f.mlt().set_field_names(names(&[mlt_field]));

    let mut seed_document: HashMap<String, Vec<Option<String>>> = HashMap::new();
    let sample_field2 = "text2";
    let text_value1 = "apache apache lucene lucene";
    let text_value2 = "apache2 apache2 lucene2 lucene2 lucene2";
    seed_document.insert(mlt_field.to_string(), values(text_value1));
    seed_document.insert(sample_field2.to_string(), values(text_value2));

    let q = f.mlt().like_document(&seed_document)?;
    let clauses = as_boolean(&q).clauses();
    let unexpected_terms: HashSet<Term> = [
        Term::from_text(mlt_field, "apache"), // Term Frequency < Minimum Accepted Term Frequency
        Term::from_text(mlt_field, "lucene"), // Term Frequency < Minimum Accepted Term Frequency
        Term::from_text(mlt_field, "apache2"), // Term Frequency < Minimum Accepted Term Frequency
        Term::from_text(mlt_field, "lucene2"), // Wrong Field
    ]
    .into_iter()
    .collect();

    // None of the Not Expected terms is in the query
    for clause in clauses {
        let term = as_term_query(clause.query()).get_term();
        Assert::assert_false_msg(
            &format!("Unexpected term '{term}' found in query terms"),
            unexpected_terms.contains(term),
        );
    }

    Assert::assert_equals_msg(Some("Expected 0 clauses only!"), 0usize, clauses.len());
    f.analyzer.as_ref().expect("setUp ran").close();
    Ok(())
}

fn test_seed_document_map_query_fields_set(f: &mut MoreLikeThisTest) -> Result<()> {
    f.mlt().set_min_term_freq(2);

    let mlt_field = "text";

    f.mlt().set_field_names(names(&[mlt_field]));

    let mut seed_document: HashMap<String, Vec<Option<String>>> = HashMap::new();
    let not_mlt_field = "text2";
    let text_value1 = "apache apache lucene lucene";
    let text_value2 = "apache2 apache2 lucene2 lucene2 lucene2";
    seed_document.insert(mlt_field.to_string(), values(text_value1));
    seed_document.insert(not_mlt_field.to_string(), values(text_value2));

    let expected_terms: Vec<Term> = vec![
        Term::from_text(mlt_field, "apache"),
        Term::from_text(mlt_field, "lucene"),
    ];

    let unexpected_terms: HashSet<Term> = [
        Term::from_text(mlt_field, "apache2"),
        Term::from_text(mlt_field, "lucene2"),
        Term::from_text(not_mlt_field, "apache2"),
        Term::from_text(not_mlt_field, "lucene2"),
    ]
    .into_iter()
    .collect();

    let q = f.mlt().like_document(&seed_document)?;
    let clauses = as_boolean(&q).clauses();
    let mut clauses_terms: HashSet<Term> = HashSet::new();
    for clause in clauses {
        let term = as_term_query(clause.query()).get_term();
        clauses_terms.insert(term.clone());
    }

    Assert::assert_equals_msg(Some("Expected 2 clauses only!"), 2usize, clauses.len());

    // None of the Not Expected terms is in the query
    for clause in clauses {
        let term = as_term_query(clause.query()).get_term();
        Assert::assert_false_msg(
            &format!("Unexpected term '{term}' found in query terms"),
            unexpected_terms.contains(term),
        );
    }

    // All of the Expected terms are in the query
    for expected_term in &expected_terms {
        Assert::assert_true_msg(
            &format!("Expected term '{expected_term}' is not found in query terms"),
            clauses_terms.contains(expected_term),
        );
    }
    Ok(())
}

// just basic equals/hashcode etc
fn test_more_like_this_query(f: &mut MoreLikeThisTest) -> Result<()> {
    let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::new(&*random()));
    let query: Arc<dyn Query> = Arc::new(MoreLikeThisQuery::new(
        "this is a test",
        vec!["text".to_string()],
        Arc::clone(&analyzer),
        "text",
    ));
    QueryUtils::check_searcher(&*random(), &query, f.searcher.as_ref());
    analyzer.close();
    Ok(())
}

fn test_top_n(f: &mut MoreLikeThisTest) -> Result<()> {
    let num_docs = 100;
    let top_n = 25;

    // add series of docs with terms of decreasing df
    let dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    for i in 0..num_docs {
        add_doc_texts(&writer, "text", &generate_str_seq(0, i + 1))?;
    }
    let reader: Arc<dyn IndexReader> = writer.get_reader()?;
    writer.close()?;

    // setup MLT query
    f.mlt = Some(get_default_more_like_this(Arc::clone(&reader)));
    f.mlt().set_max_query_terms(top_n);
    f.mlt().set_field_names(names(&["text"]));

    // perform MLT query
    let mut like_text = String::new();
    for text in generate_str_seq(0, num_docs) {
        like_text.push_str(&text);
        like_text.push(' ');
    }
    let q = f.mlt().like_readers("text", vec![reader_of(&like_text)])?;

    // check best terms are topN of highest idf
    let clauses = as_boolean(&q).clauses();
    Assert::assert_equals_msg(
        Some(&format!("Expected {top_n} clauses only!")),
        top_n as usize,
        clauses.len(),
    );

    let expected_terms: Vec<Term> = generate_str_seq(num_docs - top_n, top_n)
        .iter()
        .map(|text| Term::from_text("text", text))
        .collect();
    for clause in clauses {
        let term = as_term_query(clause.query()).get_term();
        Assert::assert_true(expected_terms.contains(term));
    }

    // clean up
    reader.close()?;
    dir.close()?;
    f.analyzer.as_ref().expect("setUp ran").close();
    Ok(())
}

/// `generateStrSeq(int from, int size)`.
fn generate_str_seq(from: i32, size: i32) -> Vec<String> {
    (0..size).map(|i| (from + i).to_string()).collect()
}

/// `addShopDoc(writer, type, weSell, weDontSell)`.
fn add_shop_doc(
    writer: &RandomIndexWriter,
    type_: &str,
    we_sell: &[&str],
    we_dont_sell: &[&str],
) -> Result<i32> {
    let mut doc = Document::new();
    doc.add(LuceneTestCase::new_text_field(SHOP_TYPE, type_, Store::YES));
    for item in we_sell {
        doc.add(LuceneTestCase::new_text_field(FOR_SALE, item, Store::YES));
    }
    for item in we_dont_sell {
        doc.add(LuceneTestCase::new_text_field(
            NOT_FOR_SALE,
            item,
            Store::YES,
        ));
    }
    writer.add_document(&doc)?;
    Ok(writer.get_doc_stats()?.num_docs - 1)
}

fn test_multi_field_should_return_per_field_boolean_query(_f: &mut MoreLikeThisTest) -> Result<()> {
    let mut reader: Option<Arc<dyn IndexReader>> = None;
    let dir = LuceneTestCase::new_directory();
    let analyzer = whitespace_analyzer();
    let result = (|| -> Result<()> {
        let max_query_terms = 25;

        let it_shop_item_for_sale = [
            "watch",
            "ipod",
            "asrock",
            "imac",
            "macbookpro",
            "monitor",
            "keyboard",
            "mouse",
            "speakers",
        ];
        let it_shop_item_not_for_sale = ["tie", "trousers", "shoes", "skirt", "hat"];

        let clothes_shop_item_for_sale = ["tie", "trousers", "shoes", "skirt", "hat"];
        let clothes_shop_item_not_for_sale = [
            "watch",
            "ipod",
            "asrock",
            "imac",
            "macbookpro",
            "monitor",
            "keyboard",
            "mouse",
            "speakers",
        ];

        // add series of shop docs
        let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
        for _ in 0..300 {
            add_shop_doc(
                &writer,
                "it",
                &it_shop_item_for_sale,
                &it_shop_item_not_for_sale,
            )?;
        }
        for _ in 0..300 {
            add_shop_doc(
                &writer,
                "clothes",
                &clothes_shop_item_for_sale,
                &clothes_shop_item_not_for_sale,
            )?;
        }
        // Input Document is a clothes shop
        let input_doc_id = add_shop_doc(
            &writer,
            "clothes",
            &clothes_shop_item_for_sale,
            &clothes_shop_item_not_for_sale,
        )?;
        let r: Arc<dyn IndexReader> = writer.get_reader()?;
        reader = Some(Arc::clone(&r));
        writer.close()?;

        // setup MLT query
        let mut mlt = get_default_more_like_this(r);
        mlt.set_max_query_terms(max_query_terms);
        mlt.set_field_names(names(&[FOR_SALE, NOT_FOR_SALE]));

        // perform MLT query
        let q = mlt.like(input_doc_id)?;
        let clauses = as_boolean(&q).clauses();

        let mut expected_clothes_shop_clauses: Vec<BooleanClause> = Vec::new();
        for item_for_sale in clothes_shop_item_for_sale {
            expected_clothes_shop_clauses.push(BooleanClause::new(
                Arc::new(TermQuery::new(Term::from_text(FOR_SALE, item_for_sale))),
                Occur::SHOULD,
            ));
        }
        for item_not_for_sale in clothes_shop_item_not_for_sale {
            expected_clothes_shop_clauses.push(BooleanClause::new(
                Arc::new(TermQuery::new(Term::from_text(
                    NOT_FOR_SALE,
                    item_not_for_sale,
                ))),
                Occur::SHOULD,
            ));
        }

        // `clauses.contains(expectedClause)`: `BooleanClause.equals` compares
        // the occur and the query
        for expected_clause in &expected_clothes_shop_clauses {
            Assert::assert_true(clauses.iter().any(|c| {
                c.occur() == expected_clause.occur()
                    && c.query().query_eq(&**expected_clause.query())
            }));
        }
        Ok(())
    })();
    // finally: clean up
    if let Some(reader) = reader {
        reader.close()?;
    }
    dir.close()?;
    analyzer.close();
    result
}

/// The anonymous `TokenFilter` of `testCustomFrequency`: a term `foo|2` is
/// the term `foo` with the term frequency `2`.
#[derive(Debug)]
struct DelimitedTermFrequencyLogic {
    term_att: Att,
    tf_att: Att,
}

impl TokenFilterLogic for DelimitedTermFrequencyLogic {
    fn increment_token(
        &mut self,
        _source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if input.increment_token()? {
            let mut term = self.term_att.char_term_mut();
            let buffer: Vec<char> = term.buffer().to_vec();
            let length = term.length();
            let mut i = 0;
            while i < length {
                if buffer[i] == '|' {
                    term.set_length(i);
                    drop(term);
                    i += 1;
                    let tf = ArrayUtil::parse_int(&buffer, i, length - i)?;
                    self.tf_att.term_freq_mut().set_term_frequency(tf);
                    return Ok(true);
                }
                i += 1;
            }
            return Ok(true);
        }
        Ok(false)
    }
}

/// The anonymous `Analyzer` of `testCustomFrequency`: an analyzer with
/// delimited term frequency, e.g. "foo|2 bar|3".
#[derive(Debug)]
struct DelimitedTermFrequencyAnalyzer {
    state: AnalyzerState,
}

impl Analyzer for DelimitedTermFrequencyAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.queries.mlt.TestMoreLikeThis$1"
    }

    fn create_components(&self, _field_name: Option<&str>) -> Result<TokenStreamComponents> {
        let tokenizer = Arc::new(AtomicRefCell::new(MockTokenizer::with_max_token_length(
            Arc::clone(&WHITESPACE),
            false,
            100,
        )?));
        let filter = MockTokenFilter::new(shared(&tokenizer), Arc::clone(&EMPTY_STOPSET))?;
        Ok(components(
            tokenizer,
            sink(add_custom_token_filter(Box::new(filter))?),
        ))
    }
}

/// `addCustomTokenFilter(TokenStream)`.
fn add_custom_token_filter(
    mut input: Box<dyn TokenStream>,
) -> Result<TokenFilter<DelimitedTermFrequencyLogic>> {
    let source = input.attribute_source_mut();
    let term_att = Att::add(source, char_term_id())?;
    let tf_att = Att::add(source, term_freq_id())?;
    Ok(TokenFilter::new(
        input,
        DelimitedTermFrequencyLogic { term_att, tf_att },
    ))
}

/// The sum of the boosts of the `lucene` clauses of `mlt.like("text", ...)`.
fn lucene_boost(mlt: &MoreLikeThis, text: &str) -> Result<f64> {
    let q = mlt.like_readers("text", vec![reader_of(text)])?;
    Ok(as_boolean(&q)
        .clauses()
        .iter()
        .map(|c| as_boost_query(c.query()))
        .filter(|x| as_term_query(&x.get_query()).get_term().text() == "lucene")
        .map(|x| f64::from(x.get_boost()))
        .sum())
}

fn test_custom_frequency(f: &mut MoreLikeThisTest) -> Result<()> {
    // define an analyzer with delimited term frequency, e.g. "foo|2 bar|3"
    let analyzer: Arc<dyn Analyzer> = Arc::new(DelimitedTermFrequencyAnalyzer {
        state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
    });

    f.mlt().set_analyzer(Some(analyzer));
    f.mlt().set_field_names(names(&["text"]));
    f.mlt().set_boost(true);

    let boost10 = lucene_boost(f.mlt(), "lucene|10 release|1")?;
    let boost1 = lucene_boost(f.mlt(), "lucene|1 release|1")?;

    // mlt should use the custom frequencies provided by the analyzer so "lucene|10" should be
    // boosted more than "lucene|1"
    Assert::assert_true_msg(
        &format!(
            "{} should be grater than {}",
            rucene::util::java_fmt::double_to_string(boost10),
            rucene::util::java_fmt::double_to_string(boost1)
        ),
        boost10 > boost1,
    );
    Ok(())
}
