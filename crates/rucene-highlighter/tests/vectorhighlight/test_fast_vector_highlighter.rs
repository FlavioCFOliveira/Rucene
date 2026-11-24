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

//! Port of `TestFastVectorHighlighter.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::{
    Analyzer, DelegatingAnalyzerWrapper, DelegatingAnalyzerWrapperLogic, PerFieldReuseStrategy,
    TokenStream,
};
use rucene::document::{Document, Field, StoredField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::{IndexReader, Term};
use rucene::search::boolean_clause::{BooleanClause, Occur};
use rucene::search::boolean_query::Builder as BooleanQueryBuilder;
use rucene::search::boost_query::BoostQuery;
use rucene::search::double_values_source;
use rucene::search::phrase_query::PhraseQuery;
use rucene::search::query::Query;
use rucene::search::synonym_query;
use rucene::search::TermQuery;
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::automaton::{CharacterRunAutomaton, RegExp};
use rucene_highlighter::highlight::default_encoder::DefaultEncoder;
use rucene_highlighter::highlight::encoder::Encoder;
use rucene_highlighter::vectorhighlight::fast_vector_highlighter::FastVectorHighlighter;
use rucene_highlighter::vectorhighlight::field_query::FieldQuery;
use rucene_highlighter::vectorhighlight::frag_list_builder::FragListBuilder;
use rucene_highlighter::vectorhighlight::fragments_builder::FragmentsBuilder;
use rucene_highlighter::vectorhighlight::score_order_fragments_builder::ScoreOrderFragmentsBuilder;
use rucene_highlighter::vectorhighlight::simple_frag_list_builder::SimpleFragListBuilder;
use rucene_highlighter::vectorhighlight::simple_fragments_builder::SimpleFragmentsBuilder;
use rucene_highlighter::vectorhighlight::weighted_frag_list_builder::WeightedFragListBuilder;
use rucene_queries::common_terms_query::CommonTermsQuery;
use rucene_queries::function::function_score_query::FunctionScoreQuery;
use rucene_test_framework::analysis::canned_token_stream::CannedTokenStream;
use rucene_test_framework::analysis::mock_token_filter::ENGLISH_STOPSET;
use rucene_test_framework::analysis::mock_tokenizer::{SIMPLE, WHITESPACE};
use rucene_test_framework::analysis::token::Token;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::LuceneTestCase;

use super::abstract_test_case::tv_type;
use crate::highlighter_support::core_support::{class, random, test, JavaHashSet};

/// `TestFastVectorHighlighter`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.vectorhighlight.TestFastVectorHighlighter",
        vec![
            test("testSimpleHighlightTest", test_simple_highlight_test),
            test(
                "testFunctionScoreQueryHighlight",
                test_function_score_query_highlight,
            ),
            test(
                "testPhraseHighlightLongTextTest",
                test_phrase_highlight_long_text_test,
            ),
            test("testPhraseHighlightTest", test_phrase_highlight_test),
            test(
                "testBoostedPhraseHighlightTest",
                test_boosted_phrase_highlight_test,
            ),
            test(
                "testCommonTermsQueryHighlight",
                test_common_terms_query_highlight,
            ),
            test("testMatchedFields", test_matched_fields),
            test(
                "testMultiValuedSortByScore",
                test_multi_valued_sort_by_score,
            ),
            test("testWithSynonym", test_with_synonym),
            test(
                "testBooleanPhraseWithSynonym",
                test_boolean_phrase_with_synonym,
            ),
            test(
                "testPhrasesSpanningFieldValues",
                test_phrases_spanning_field_values,
            ),
        ],
    )
}

fn term_query(field: &str, text: &str) -> Arc<dyn Query> {
    Arc::new(TermQuery::new(Term::from_text(field.to_string(), text)))
}

fn phrase(slop: i32, field: &str, terms: &[&str]) -> Arc<dyn Query> {
    Arc::new(PhraseQuery::with_slop(slop, field, terms).expect("a valid phrase"))
}

fn strings(tags: &[&str]) -> Vec<String> {
    tags.iter().map(|s| s.to_string()).collect()
}

/// `new IndexWriter(dir, newIndexWriterConfig(analyzer))`.
fn writer_with(
    dir: &rucene_test_framework::util::lucene_test_case::TestDirectory,
    analyzer: Arc<dyn Analyzer>,
) -> Result<Arc<IndexWriter>> {
    IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(Some(analyzer)),
    )
}

fn mock_writer(
    dir: &rucene_test_framework::util::lucene_test_case::TestDirectory,
) -> Result<Arc<IndexWriter>> {
    writer_with(dir, Arc::new(MockAnalyzer::new(&*random())))
}

fn open(writer: &Arc<IndexWriter>) -> Result<Arc<dyn IndexReader>> {
    let reader: Arc<dyn IndexReader> = directory_reader::open_with_writer(&**writer)?;
    Ok(reader)
}

fn field_query(
    highlighter: &FastVectorHighlighter,
    query: Arc<dyn Query>,
    reader: &Arc<dyn IndexReader>,
) -> Result<FieldQuery> {
    highlighter.get_field_query_with_reader(&query, Some(reader))
}

fn best(
    highlighter: &FastVectorHighlighter,
    fq: &FieldQuery,
    reader: &Arc<dyn IndexReader>,
    doc_id: i32,
    field: &str,
    frag_char_size: i32,
    max: i32,
) -> Result<Vec<String>> {
    Ok(highlighter
        .get_best_fragments(fq, &**reader, doc_id, field, frag_char_size, max)?
        .unwrap_or_default())
}

fn simple_highlight(fq_of: impl Fn() -> Arc<dyn Query>) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = mock_writer(&dir)?;
    let mut doc = Document::new();
    let type_ = tv_type(true)?;
    let field = Field::new(
        "field",
        "This is a test where foo is highlighed and should be highlighted".to_string(),
        type_,
    )?;
    doc.add(field);
    writer.add_document(&doc)?;
    let highlighter = FastVectorHighlighter::new();
    let reader = open(&writer)?;
    let doc_id = 0;
    let fq = field_query(&highlighter, fq_of(), &reader)?;
    let best_fragments = best(&highlighter, &fq, &reader, doc_id, "field", 54, 1)?;
    // highlighted results are centered
    Assert::assert_equals(
        "This is a test where <b>foo</b> is highlighed and should be highlighted".to_string(),
        best_fragments[0].clone(),
    );
    let best_fragments = best(&highlighter, &fq, &reader, doc_id, "field", 52, 1)?;
    Assert::assert_equals(
        "This is a test where <b>foo</b> is highlighed and should be".to_string(),
        best_fragments[0].clone(),
    );
    let best_fragments = best(&highlighter, &fq, &reader, doc_id, "field", 30, 1)?;
    Assert::assert_equals(
        "a test where <b>foo</b> is highlighed".to_string(),
        best_fragments[0].clone(),
    );
    reader.close()?;
    writer.close()?;
    dir.close()?;
    Ok(())
}

fn test_simple_highlight_test() -> Result<()> {
    simple_highlight(|| term_query("field", "foo"))
}

fn test_function_score_query_highlight() -> Result<()> {
    simple_highlight(|| {
        Arc::new(FunctionScoreQuery::new(
            term_query("field", "foo"),
            double_values_source::constant(1.0),
        ))
    })
}

fn test_phrase_highlight_long_text_test() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = mock_writer(&dir)?;
    let mut doc = Document::new();
    let type_ = tv_type(true)?;
    let text = Field::new(
        "text",
        "Netscape was the general name for a series of web browsers originally produced by Netscape Communications Corporation, now a subsidiary of AOL The original browser was once the dominant browser in terms of usage share, but as a result of the first browser war it lost virtually all of its share to Internet Explorer Netscape was discontinued and support for all Netscape browsers and client products was terminated on March 1, 2008 Netscape Navigator was the name of Netscape\u{0027}s web browser from versions 1.0 through 4.8 The first beta release versions of the browser were released in 1994 and known as Mosaic and then Mosaic Netscape until a legal challenge from the National Center for Supercomputing Applications (makers of NCSA Mosaic, which many of Netscape\u{0027}s founders used to develop), led to the name change to Netscape Navigator The company\u{0027}s name also changed from Mosaic Communications Corporation to Netscape Communications Corporation The browser was easily the most advanced...".to_string(),
        type_,
    )?;
    doc.add(text);
    writer.add_document(&doc)?;
    let highlighter = FastVectorHighlighter::new();
    let reader = open(&writer)?;
    let doc_id = 0;
    let field = "text";
    {
        let mut query = BooleanQueryBuilder::new();
        query.add(term_query(field, "internet"), Occur::MUST)?;
        query.add(term_query(field, "explorer"), Occur::MUST)?;
        let fq = field_query(&highlighter, Arc::new(query.build()), &reader)?;
        let best_fragments = best(&highlighter, &fq, &reader, doc_id, field, 128, 1)?;
        // highlighted results are centered
        Assert::assert_equals(1, best_fragments.len());
        Assert::assert_equals(
            "first browser war it lost virtually all of its share to <b>Internet</b> <b>Explorer</b> Netscape was discontinued and support for all Netscape browsers".to_string(),
            best_fragments[0].clone(),
        );
    }
    {
        let query = phrase(0, field, &["internet", "explorer"]);
        let fq = field_query(&highlighter, query, &reader)?;
        let best_fragments = best(&highlighter, &fq, &reader, doc_id, field, 128, 1)?;
        // highlighted results are centered
        Assert::assert_equals(1, best_fragments.len());
        Assert::assert_equals(
            "first browser war it lost virtually all of its share to <b>Internet Explorer</b> Netscape was discontinued and support for all Netscape browsers".to_string(),
            best_fragments[0].clone(),
        );
    }
    reader.close()?;
    writer.close()?;
    dir.close()?;
    Ok(())
}

// see LUCENE-4899
fn test_phrase_highlight_test() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = mock_writer(&dir)?;
    let mut doc = Document::new();
    let type_ = tv_type(true)?;
    let long_term_field = Field::new(
        "long_term",
        "This is a test thisisaverylongwordandmakessurethisfails where foo is highlighed and should be highlighted".to_string(),
        Arc::clone(&type_),
    )?;
    let no_long_term_field = Field::new(
        "no_long_term",
        "This is a test where foo is highlighed and should be highlighted".to_string(),
        type_,
    )?;
    doc.add(long_term_field);
    doc.add(no_long_term_field);
    writer.add_document(&doc)?;
    let highlighter = FastVectorHighlighter::new();
    let reader = open(&writer)?;
    let doc_id = 0;
    let mut field = "no_long_term";
    {
        let mut query = BooleanQueryBuilder::new();
        query.add(term_query(field, "test"), Occur::MUST)?;
        query.add(term_query(field, "foo"), Occur::MUST)?;
        query.add(term_query(field, "highlighed"), Occur::MUST)?;
        let fq = field_query(&highlighter, Arc::new(query.build()), &reader)?;
        let best_fragments = best(&highlighter, &fq, &reader, doc_id, field, 18, 1)?;
        // highlighted results are centered
        Assert::assert_equals(1, best_fragments.len());
        Assert::assert_equals(
            "<b>foo</b> is <b>highlighed</b> and".to_string(),
            best_fragments[0].clone(),
        );
    }
    {
        let mut query = BooleanQueryBuilder::new();
        let pq = phrase(5, field, &["test", "foo", "highlighed"]);
        query.add(term_query(field, "foo"), Occur::MUST)?;
        query.add(pq, Occur::MUST)?;
        query.add(term_query(field, "highlighed"), Occur::MUST)?;
        let fq = field_query(&highlighter, Arc::new(query.build()), &reader)?;
        let best_fragments = best(&highlighter, &fq, &reader, doc_id, field, 18, 1)?;
        // highlighted results are centered
        Assert::assert_equals(0, best_fragments.len());
        let best_fragments = best(&highlighter, &fq, &reader, doc_id, field, 30, 1)?;
        // highlighted results are centered
        Assert::assert_equals(1, best_fragments.len());
        Assert::assert_equals(
            "a <b>test</b> where <b>foo</b> is <b>highlighed</b> and".to_string(),
            best_fragments[0].clone(),
        );
    }
    {
        let query = phrase(3, field, &["test", "foo", "highlighed"]);
        let fq = field_query(&highlighter, query, &reader)?;
        let best_fragments = best(&highlighter, &fq, &reader, doc_id, field, 18, 1)?;
        // highlighted results are centered
        Assert::assert_equals(0, best_fragments.len());
        let best_fragments = best(&highlighter, &fq, &reader, doc_id, field, 30, 1)?;
        // highlighted results are centered
        Assert::assert_equals(1, best_fragments.len());
        Assert::assert_equals(
            "a <b>test</b> where <b>foo</b> is <b>highlighed</b> and".to_string(),
            best_fragments[0].clone(),
        );
    }
    {
        let query = phrase(30, field, &["test", "foo", "highlighed"]);
        let fq = field_query(&highlighter, query, &reader)?;
        let best_fragments = best(&highlighter, &fq, &reader, doc_id, field, 18, 1)?;
        Assert::assert_equals(0, best_fragments.len());
    }
    {
        let mut query = BooleanQueryBuilder::new();
        let pq = phrase(5, field, &["test", "foo", "highlighed"]);
        let mut inner = BooleanQueryBuilder::new();
        inner.add(Arc::clone(&pq), Occur::MUST)?;
        inner.add(term_query(field, "foo"), Occur::MUST)?;
        query.add(Arc::new(inner.build()), Occur::MUST)?;
        query.add(pq, Occur::MUST)?;
        query.add(term_query(field, "highlighed"), Occur::MUST)?;
        let fq = field_query(&highlighter, Arc::new(query.build()), &reader)?;
        let best_fragments = best(&highlighter, &fq, &reader, doc_id, field, 18, 1)?;
        Assert::assert_equals(0, best_fragments.len());
        let best_fragments = best(&highlighter, &fq, &reader, doc_id, field, 30, 1)?;
        // highlighted results are centered
        Assert::assert_equals(1, best_fragments.len());
        Assert::assert_equals(
            "a <b>test</b> where <b>foo</b> is <b>highlighed</b> and".to_string(),
            best_fragments[0].clone(),
        );
    }

    field = "long_term";
    {
        let mut query = BooleanQueryBuilder::new();
        query.add(
            term_query(field, "thisisaverylongwordandmakessurethisfails"),
            Occur::MUST,
        )?;
        query.add(term_query(field, "foo"), Occur::MUST)?;
        query.add(term_query(field, "highlighed"), Occur::MUST)?;
        let fq = field_query(&highlighter, Arc::new(query.build()), &reader)?;
        let best_fragments = best(&highlighter, &fq, &reader, doc_id, field, 18, 1)?;
        // highlighted results are centered
        Assert::assert_equals(1, best_fragments.len());
        Assert::assert_equals(
            "<b>thisisaverylongwordandmakessurethisfails</b>".to_string(),
            best_fragments[0].clone(),
        );
    }
    reader.close()?;
    writer.close()?;
    dir.close()?;
    Ok(())
}

fn test_boosted_phrase_highlight_test() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = mock_writer(&dir)?;
    let mut doc = Document::new();
    let type_ = tv_type(true)?;
    let mut text = String::new();
    text.push_str("words words junk junk junk junk junk junk junk junk highlight junk junk junk junk together junk ");
    for _ in 0..10 {
        text.push_str("junk junk junk junk junk junk junk junk junk junk junk junk junk junk junk junk junk junk junk junk ");
    }
    text.push_str("highlight words together ");
    for _ in 0..10 {
        text.push_str("junk junk junk junk junk junk junk junk junk junk junk junk junk junk junk junk junk junk junk junk ");
    }
    doc.add(Field::new("text", text.trim().to_string(), type_)?);
    writer.add_document(&doc)?;
    let highlighter = FastVectorHighlighter::new();
    let reader = open(&writer)?;

    // This mimics what some query parsers do to <highlight words together>
    let mut terms = BooleanQueryBuilder::new();
    terms.add(clause("text", &["highlight"]), Occur::MUST)?;
    terms.add(clause("text", &["words"]), Occur::MUST)?;
    terms.add(clause("text", &["together"]), Occur::MUST)?;

    // This mimics what some query parsers do to <"highlight words together">
    let mut phrase_b = BooleanQueryBuilder::new();
    phrase_b.add(
        clause("text", &["highlight", "words", "together"]),
        Occur::MUST,
    )?;
    let phrase: Arc<dyn Query> = Arc::new(phrase_b.build());
    let phrase: Arc<dyn Query> = Arc::new(BoostQuery::new(phrase, 100.0)?);

    // Now combine those results in a boolean query which should pull the phrases to the front of
    // the list of fragments
    let mut query = BooleanQueryBuilder::new();
    query.add(Arc::clone(&phrase), Occur::MUST)?;
    query.add(phrase, Occur::SHOULD)?;
    let q: Arc<dyn Query> = Arc::new(query.build());
    let fq = FieldQuery::new(&q, Some(&reader), true, false)?;
    let fragment = highlighter.get_best_fragment(&fq, &*reader, 0, "text", 100)?;
    Assert::assert_equals_str(
        None,
        Some("junk junk junk junk junk junk junk junk <b>highlight words together</b> junk junk junk junk junk junk junk junk"),
        fragment.as_deref(),
    );
    reader.close()?;
    writer.close()?;
    dir.close()?;
    Ok(())
}

fn test_common_terms_query_highlight() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let config = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
        MockAnalyzer::with_filter(
            &*random(),
            Arc::clone(&SIMPLE),
            true,
            Arc::clone(&ENGLISH_STOPSET),
        ),
    )));
    // don't reorder doc ids
    config.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let writer = IndexWriter::new(dir.as_directory(), config)?;
    let type_ = tv_type(true)?;
    let texts = [
        "Hello this is a piece of text that is very long and contains too much preamble and the meat is really here which says kennedy has been shot",
        "This piece of text refers to Kennedy at the beginning then has a longer piece of text that is very long in the middle and finally ends with another reference to Kennedy",
        "JFK has been shot",
        "John Kennedy has been shot",
        "This text has a typo in referring to Keneddy",
        "wordx wordy wordz wordx wordy wordx worda wordb wordy wordc",
        "y z x y z a b",
        "lets is a the lets is a the lets is a the lets",
    ];
    for t in texts {
        let mut doc = Document::new();
        let field = Field::new("field", t.to_string(), Arc::clone(&type_))?;
        doc.add(field);
        writer.add_document(&doc)?;
    }
    let mut query = CommonTermsQuery::new(Occur::MUST, Occur::SHOULD, 2.0)?;
    query.add(Term::from_text("field", "text"));
    query.add(Term::from_text("field", "long"));
    query.add(Term::from_text("field", "very"));
    let query: Arc<dyn Query> = Arc::new(query);

    let highlighter = FastVectorHighlighter::new();
    let directory_reader = directory_reader::open_with_writer(&*writer)?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&directory_reader));
    let reader: Arc<dyn IndexReader> = directory_reader;
    let hits = searcher.search(Arc::clone(&query), 10)?;
    Assert::assert_equals(2, hits.total_hits.value());
    let fq = field_query(&highlighter, Arc::clone(&query), &reader)?;
    let best_fragments = best(&highlighter, &fq, &reader, 1, "field", 1000, 1)?;
    Assert::assert_equals(
        "This piece of <b>text</b> refers to Kennedy at the beginning then has a longer piece of <b>text</b> that is <b>very</b> <b>long</b> in the middle and finally ends with another reference to Kennedy".to_string(),
        best_fragments[0].clone(),
    );

    let fq = field_query(&highlighter, query, &reader)?;
    let best_fragments = best(&highlighter, &fq, &reader, 0, "field", 1000, 1)?;
    Assert::assert_equals(
        "Hello this is a piece of <b>text</b> that is <b>very</b> <b>long</b> and contains too much preamble and the meat is really here which says kennedy has been shot".to_string(),
        best_fragments[0].clone(),
    );

    drop(searcher);
    reader.close()?;
    writer.close()?;
    dir.close()?;
    Ok(())
}

fn test_matched_fields() -> Result<()> {
    // Searching just on the stored field doesn't highlight a stopword
    matched_fields_test_case_full(
        false,
        true,
        "a match",
        "a <b>match</b>",
        &[clause("field", &["a"]), clause("field", &["match"])],
    )?;

    // Even if you add an unqueried matched field that would match it
    matched_fields_test_case(
        "a match",
        "a <b>match</b>",
        &[clause("field", &["a"]), clause("field", &["match"])],
    )?;

    // Nor if you query the field but don't add it as a matched field to the highlighter
    matched_fields_test_case_full(
        false,
        false,
        "a match",
        "a <b>match</b>",
        &[clause("field_exact", &["a"]), clause("field", &["match"])],
    )?;

    // But if you query the field and add it as a matched field to the highlighter then it is
    // highlighted
    matched_fields_test_case(
        "a match",
        "<b>a</b> <b>match</b>",
        &[clause("field_exact", &["a"]), clause("field", &["match"])],
    )?;

    // It is also ok to match just the matched field but get highlighting from the stored field
    matched_fields_test_case(
        "a match",
        "<b>a</b> <b>match</b>",
        &[
            clause("field_exact", &["a"]),
            clause("field_exact", &["match"]),
        ],
    )?;

    // Boosted matched fields work too
    matched_fields_test_case(
        "a match",
        "<b>a</b> <b>match</b>",
        &[
            clause_b("field_exact", 5.0, &["a"]),
            clause("field", &["match"]),
        ],
    )?;

    // It is also ok if both the stored and the matched field match the term
    matched_fields_test_case(
        "a match",
        "a <b>match</b>",
        &[
            clause("field_exact", &["match"]),
            clause("field", &["match"]),
        ],
    )?;

    // And the highlighter respects the boosts on matched fields when sorting fragments
    matched_fields_test_case(
        "cat cat junk junk junk junk junk junk junk a cat junk junk",
        "junk junk <b>a cat</b> junk junk",
        &[
            clause("field", &["cat"]),
            clause_b("field_exact", 5.0, &["a", "cat"]),
        ],
    )?;
    matched_fields_test_case(
        "cat cat junk junk junk junk junk junk junk a cat junk junk",
        "<b>cat</b> <b>cat</b> junk junk junk junk",
        &[
            clause("field", &["cat"]),
            clause("field_exact", &["a", "cat"]),
        ],
    )?;

    // The same thing works across three fields as well
    matched_fields_test_case(
        "cat cat CAT junk junk junk junk junk junk junk a cat junk junk",
        "junk junk <b>a cat</b> junk junk",
        &[
            clause("field", &["cat"]),
            clause_b("field_exact", 200.0, &["a", "cat"]),
            clause_b("field_super_exact", 5.0, &["CAT"]),
        ],
    )?;
    matched_fields_test_case(
        "a cat cat junk junk junk junk junk junk junk a CAT junk junk",
        "junk junk <b>a CAT</b> junk junk",
        &[
            clause("field", &["cat"]),
            clause_b("field_exact", 5.0, &["a", "cat"]),
            clause_b("field_super_exact", 200.0, &["a", "CAT"]),
        ],
    )?;

    // And across fields with different tokenizers!
    matched_fields_test_case(
        "cat cat junk junk junk junk junk junk junk a cat junk junk",
        "junk junk <b>a cat</b> junk junk",
        &[
            clause_b("field_exact", 5.0, &["a", "cat"]),
            clause("field_characters", &["c"]),
        ],
    )?;
    matched_fields_test_case(
        "cat cat junk junk junk junk junk junk junk a cat junk junk",
        "<b>c</b>at <b>c</b>at junk junk junk junk",
        &[
            clause("field_exact", &["a", "cat"]),
            clause("field_characters", &["c"]),
        ],
    )?;
    matched_fields_test_case(
        "cat cat junk junk junk junk junk junk junk a cat junk junk",
        "ca<b>t</b> ca<b>t</b> junk junk junk junk",
        &[
            clause("field_exact", &["a", "cat"]),
            clause("field_characters", &["t"]),
        ],
    )?;
    matched_fields_test_case(
        "cat cat junk junk junk junk junk junk junk a cat junk junk",
        "<b>cat</b> <b>cat</b> junk junk junk junk", // See how the phrases are joined?
        &[
            clause("field", &["cat"]),
            clause_b("field_characters", 5.0, &["c"]),
        ],
    )?;
    matched_fields_test_case(
        "cat cat junk junk junk junk junk junk junk a cat junk junk",
        "junk junk <b>a cat</b> junk junk",
        &[
            clause("field", &["cat"]),
            clause_b("field_characters", 5.0, &["a", " ", "c", "a", "t"]),
        ],
    )?;

    // Phrases and tokens inside one another are joined
    matched_fields_test_case(
        "cats wow",
        "<b>cats w</b>ow",
        &[
            clause("field", &["cats"]),
            clause("field_tripples", &["s w"]),
        ],
    )?;

    // Everything works pretty well even if you don't require a field match
    matched_fields_test_case_full(
        true,
        false,
        "cat cat junk junk junk junk junk junk junk a cat junk junk",
        "junk junk <b>a cat</b> junk junk",
        &[
            clause("field", &["cat"]),
            clause_b("field_characters", 10.0, &["a", " ", "c", "a", "t"]),
        ],
    )?;

    // Even boosts keep themselves pretty much intact
    matched_fields_test_case_full(
        true,
        false,
        "a cat cat junk junk junk junk junk junk junk a CAT junk junk",
        "junk junk <b>a CAT</b> junk junk",
        &[
            clause("field", &["cat"]),
            clause_b("field_exact", 5.0, &["a", "cat"]),
            clause_b("field_super_exact", 200.0, &["a", "CAT"]),
        ],
    )?;
    matched_fields_test_case_full(
        true,
        false,
        "cat cat CAT junk junk junk junk junk junk junk a cat junk junk",
        "junk junk <b>a cat</b> junk junk",
        &[
            clause("field", &["cat"]),
            clause_b("field_exact", 200.0, &["a", "cat"]),
            clause_b("field_super_exact", 5.0, &["CAT"]),
        ],
    )?;

    // Except that all the matched field matches apply even if they aren't mentioned in the query
    // which can make for some confusing scoring.  This isn't too big a deal, just something you
    // need to think about when you don't force a field match.
    matched_fields_test_case_full(
        true,
        false,
        "cat cat junk junk junk junk junk junk junk a cat junk junk",
        "<b>cat</b> <b>cat</b> junk junk junk junk",
        &[
            clause("field", &["cat"]),
            clause_b("field_characters", 4.0, &["a", " ", "c", "a", "t"]),
        ],
    )?;

    // It is also cool to match fields that don't have _exactly_ the same text so long as you are
    // careful.
    // In this case field_sliced is a prefix of field.
    matched_fields_test_case(
        "cat cat junk junk junk junk junk junk junk a cat junk junk",
        "<b>cat</b> <b>cat</b> junk junk junk junk",
        &[clause("field_sliced", &["cat"])],
    )?;

    // Multiple matches add to the score of the segment
    matched_fields_test_case(
        "cat cat junk junk junk junk junk junk junk a cat junk junk",
        "<b>cat</b> <b>cat</b> junk junk junk junk",
        &[
            clause("field", &["cat"]),
            clause("field_sliced", &["cat"]),
            clause_b("field_exact", 2.0, &["a", "cat"]),
        ],
    )?;
    matched_fields_test_case(
        "cat cat junk junk junk junk junk junk junk a cat junk junk",
        "junk junk <b>a cat</b> junk junk",
        &[
            clause("field", &["cat"]),
            clause("field_sliced", &["cat"]),
            clause_b("field_exact", 4.0, &["a", "cat"]),
        ],
    )?;

    // Even fields with tokens on top of one another are ok
    matched_fields_test_case(
        "cat cat junk junk junk junk junk junk junk a cat junk junk",
        "<b>cat</b> cat junk junk junk junk",
        &[
            clause_b("field_der_red", 2.0, &["der"]),
            clause("field_exact", &["a", "cat"]),
        ],
    )?;
    matched_fields_test_case(
        "cat cat junk junk junk junk junk junk junk a cat junk junk",
        "<b>cat</b> cat junk junk junk junk",
        &[
            clause_b("field_der_red", 2.0, &["red"]),
            clause("field_exact", &["a", "cat"]),
        ],
    )?;
    matched_fields_test_case(
        "cat cat junk junk junk junk junk junk junk a cat junk junk",
        "<b>cat</b> cat junk junk junk junk",
        &[
            clause("field_der_red", &["red"]),
            clause("field_der_red", &["der"]),
            clause("field_exact", &["a", "cat"]),
        ],
    )?;
    Ok(())
}

#[allow(clippy::type_complexity)]
fn multi_valued_builders() -> Vec<Box<dyn FragListBuilder>> {
    vec![
        Box::new(SimpleFragListBuilder::new()),
        Box::new(WeightedFragListBuilder::new()),
    ]
}

fn test_multi_valued_sort_by_score() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = mock_writer(&dir)?;
    let mut doc = Document::new();
    let type_ = tv_type(true)?;
    // The first two fields contain the best match
    doc.add(Field::new(
        "field",
        "zero if naught".to_string(),
        Arc::clone(&type_),
    )?);
    // but total a lower score (3) than the bottom
    doc.add(Field::new(
        "field",
        "hero of legend".to_string(),
        Arc::clone(&type_),
    )?);
    // two fields (4)
    doc.add(Field::new(
        "field",
        "naught of hero".to_string(),
        Arc::clone(&type_),
    )?);
    doc.add(Field::new("field", "naught of hero".to_string(), type_)?);
    writer.add_document(&doc)?;

    let highlighter = FastVectorHighlighter::new();

    let mut fragments_builder = ScoreOrderFragmentsBuilder::new();
    fragments_builder.set_discrete_multi_value_highlighting(true);
    let reader = open(&writer)?;
    let pre_tags = strings(&["<b>"]);
    let post_tags = strings(&["</b>"]);
    let encoder: Box<dyn Encoder> = Box::new(DefaultEncoder::new());
    let doc_id = 0;
    let mut query = BooleanQueryBuilder::new();
    query.add(clause("field", &["hero"]), Occur::SHOULD)?;
    query.add(clause("field", &["of"]), Occur::SHOULD)?;
    query.add(clause("field", &["legend"]), Occur::SHOULD)?;
    let fq = field_query(&highlighter, Arc::new(query.build()), &reader)?;

    for frag_list_builder in multi_valued_builders() {
        for frag_char_size in [20, 28, 30000] {
            let best_fragments = highlighter
                .get_best_fragments_with(
                    &fq,
                    &*reader,
                    doc_id,
                    "field",
                    frag_char_size,
                    1,
                    &*frag_list_builder,
                    &fragments_builder,
                    &pre_tags,
                    &post_tags,
                    &*encoder,
                )?
                .unwrap_or_default();
            Assert::assert_equals(
                "<b>hero</b> <b>of</b> <b>legend</b>".to_string(),
                best_fragments[0].clone(),
            );
        }
    }

    reader.close()?;
    writer.close()?;
    dir.close()?;
    Ok(())
}

fn test_with_synonym() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = mock_writer(&dir)?;
    let type_ = tv_type(true)?;
    let mut doc = Document::new();
    doc.add(Field::new(
        "field",
        "the quick brown fox".to_string(),
        type_,
    )?);
    writer.add_document(&doc)?;
    let highlighter = FastVectorHighlighter::new();
    let reader = open(&writer)?;
    let doc_id = 0;

    // query1: simple synonym query
    let mut builder = synonym_query::Builder::new(Some("field"));
    builder.add_term(&Term::from_text("field", "quick"))?;
    builder.add_term(&Term::from_text("field", "fast"))?;
    let syn_query: Arc<dyn Query> = Arc::new(builder.build());
    let fq = field_query(&highlighter, Arc::clone(&syn_query), &reader)?;
    let best_fragments = best(&highlighter, &fq, &reader, doc_id, "field", 54, 1)?;
    Assert::assert_equals(
        "the <b>quick</b> brown fox".to_string(),
        best_fragments[0].clone(),
    );

    // query2: boolean query with synonym query
    let mut bq = BooleanQueryBuilder::new();
    bq.add_clause(BooleanClause::new(syn_query, Occur::MUST))?
        .add_clause(BooleanClause::new(term_query("field", "fox"), Occur::MUST))?;
    let fq = field_query(&highlighter, Arc::new(bq.build()), &reader)?;
    let best_fragments = best(&highlighter, &fq, &reader, doc_id, "field", 54, 1)?;
    Assert::assert_equals(
        "the <b>quick</b> brown <b>fox</b>".to_string(),
        best_fragments[0].clone(),
    );

    reader.close()?;
    writer.close()?;
    dir.close()?;
    Ok(())
}

fn canned(tokens: Vec<Token>) -> Arc<AtomicRefCell<dyn TokenStream>> {
    Arc::new(AtomicRefCell::new(CannedTokenStream::new(tokens)))
}

fn test_boolean_phrase_with_synonym() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = mock_writer(&dir)?;
    let mut doc = Document::new();
    let type_ = tv_type(false)?;
    let syn = Token::with_pos_inc("httpwwwfacebookcom", 0, 6, 29);
    let ts = canned(vec![
        Token::with_offsets("test", 0, 4),
        Token::with_offsets("http", 6, 10),
        syn,
        Token::with_offsets("www", 13, 16),
        Token::with_offsets("facebook", 17, 25),
        Token::with_offsets("com", 26, 29),
    ]);
    let field = Field::new_with_token_stream("field", ts, type_)?;
    doc.add(field);
    doc.add(StoredField::new_string(
        "field",
        "Test: http://www.facebook.com".to_string(),
    )?);
    writer.add_document(&doc)?;
    let highlighter = FastVectorHighlighter::new();

    let reader = open(&writer)?;
    let doc_id = 0;

    // query1: match
    let pq = phrase(0, "field", &["test", "http", "www", "facebook", "com"]);
    let fq = field_query(&highlighter, Arc::clone(&pq), &reader)?;
    let best_fragments = best(&highlighter, &fq, &reader, doc_id, "field", 54, 1)?;
    Assert::assert_equals(
        "<b>Test: http://www.facebook.com</b>".to_string(),
        best_fragments[0].clone(),
    );

    // query2: match
    let pq2 = phrase(
        0,
        "field",
        &["test", "httpwwwfacebookcom", "www", "facebook", "com"],
    );
    let fq = field_query(&highlighter, Arc::clone(&pq2), &reader)?;
    let best_fragments = best(&highlighter, &fq, &reader, doc_id, "field", 54, 1)?;
    Assert::assert_equals(
        "<b>Test: http://www.facebook.com</b>".to_string(),
        best_fragments[0].clone(),
    );

    // query3: OR query1 and query2 together
    let mut bq = BooleanQueryBuilder::new();
    bq.add(pq, Occur::SHOULD)?;
    bq.add(pq2, Occur::SHOULD)?;
    let fq = field_query(&highlighter, Arc::new(bq.build()), &reader)?;
    let best_fragments = best(&highlighter, &fq, &reader, doc_id, "field", 54, 1)?;
    Assert::assert_equals(
        "<b>Test: http://www.facebook.com</b>".to_string(),
        best_fragments[0].clone(),
    );

    reader.close()?;
    writer.close()?;
    dir.close()?;
    Ok(())
}

fn test_phrases_spanning_field_values() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    // positionIncrementGap is 0 so the pharse is found across multiple field
    // values.
    let writer = mock_writer(&dir)?;
    let type_ = tv_type(true)?;

    let mut doc = Document::new();
    for value in [
        "one two three five",
        "two three four",
        "five six five",
        "six seven eight nine eight nine eight nine eight nine eight nine eight nine",
        "eight nine",
        "ten eleven",
        "twelve thirteen",
    ] {
        doc.add(Field::new("field", value.to_string(), Arc::clone(&type_))?);
    }
    writer.add_document(&doc)?;

    let frag_list_builder = SimpleFragListBuilder::new();
    let mut fragments_builder = SimpleFragmentsBuilder::new();
    fragments_builder.set_discrete_multi_value_highlighting(true);
    let highlighter = FastVectorHighlighter::with_builders(
        true,
        true,
        Box::new(frag_list_builder),
        Box::new(fragments_builder),
    );
    let reader = open(&writer)?;
    let doc_id = 0;

    // Phrase that spans a field value
    let q = phrase(0, "field", &["four", "five"]);
    let fq = field_query(&highlighter, q, &reader)?;
    let best_fragments = best(&highlighter, &fq, &reader, doc_id, "field", 1000, 1000)?;
    Assert::assert_equals(
        "two three <b>four</b>".to_string(),
        best_fragments[0].clone(),
    );
    Assert::assert_equals(
        "<b>five</b> six five".to_string(),
        best_fragments[1].clone(),
    );
    Assert::assert_equals(2, best_fragments.len());

    // Phrase that ends at a field value
    let q = phrase(0, "field", &["three", "five"]);
    let fq = field_query(&highlighter, q, &reader)?;
    let best_fragments = best(&highlighter, &fq, &reader, doc_id, "field", 1000, 1000)?;
    Assert::assert_equals(
        "one two <b>three five</b>".to_string(),
        best_fragments[0].clone(),
    );
    Assert::assert_equals(1, best_fragments.len());

    // Phrase that spans across three values
    let q = phrase(0, "field", &["nine", "ten", "eleven", "twelve"]);
    let fq = field_query(&highlighter, q, &reader)?;
    let best_fragments = best(&highlighter, &fq, &reader, doc_id, "field", 1000, 1000)?;
    Assert::assert_equals("eight <b>nine</b>".to_string(), best_fragments[0].clone());
    Assert::assert_equals("<b>ten eleven</b>".to_string(), best_fragments[1].clone());
    Assert::assert_equals(
        "<b>twelve</b> thirteen".to_string(),
        best_fragments[2].clone(),
    );
    Assert::assert_equals(3, best_fragments.len());

    // Term query that appears in multiple values
    let q = term_query("field", "two");
    let fq = field_query(&highlighter, q, &reader)?;
    let best_fragments = best(&highlighter, &fq, &reader, doc_id, "field", 1000, 1000)?;
    Assert::assert_equals(
        "one <b>two</b> three five".to_string(),
        best_fragments[0].clone(),
    );
    Assert::assert_equals(
        "<b>two</b> three four".to_string(),
        best_fragments[1].clone(),
    );
    Assert::assert_equals(2, best_fragments.len());

    reader.close()?;
    writer.close()?;
    dir.close()?;
    Ok(())
}

fn matched_fields_test_case(
    field_value: &str,
    expected: &str,
    query_clauses: &[Arc<dyn Query>],
) -> Result<()> {
    matched_fields_test_case_full(true, true, field_value, expected, query_clauses)
}

/// The anonymous `DelegatingAnalyzerWrapper` of `matchedFieldsTestCase`.
#[derive(Debug)]
struct FieldAnalyzers {
    field: Arc<dyn Analyzer>,
    field_exact: Arc<dyn Analyzer>,
    field_super_exact: Arc<dyn Analyzer>,
    field_characters: Arc<dyn Analyzer>,
    field_tripples: Arc<dyn Analyzer>,
}

impl DelegatingAnalyzerWrapperLogic for FieldAnalyzers {
    fn get_wrapped_analyzer(&self, field_name: Option<&str>) -> &dyn Analyzer {
        // fieldAnalyzers.get(fieldName) of the TreeMap.
        match field_name {
            Some("field") | Some("field_sliced") | Some("field_der_red") => &*self.field,
            Some("field_exact") => &*self.field_exact,
            Some("field_super_exact") => &*self.field_super_exact,
            Some("field_characters") => &*self.field_characters,
            Some("field_tripples") => &*self.field_tripples,
            other => panic!("NullPointerException: no analyzer for {other:?}"),
        }
    }
}

fn run_automaton(regexp: &str) -> Result<Arc<CharacterRunAutomaton>> {
    Ok(Arc::new(CharacterRunAutomaton::new(
        RegExp::new(regexp)?.to_automaton()?,
    )?))
}

fn matched_fields_test_case_full(
    use_matched_fields: bool,
    field_match: bool,
    field_value: &str,
    expected: &str,
    query_clauses: &[Arc<dyn Query>],
) -> Result<()> {
    let mut doc = Document::new();
    let stored = tv_type(true)?;
    let matched = tv_type(false)?;
    // Whitespace tokenized with English stop words
    doc.add(Field::new("field", field_value.to_string(), stored)?);
    // Whitespace tokenized without stop words
    doc.add(Field::new(
        "field_exact",
        field_value.to_string(),
        Arc::clone(&matched),
    )?);
    // Whitespace tokenized without toLower
    doc.add(Field::new(
        "field_super_exact",
        field_value.to_string(),
        Arc::clone(&matched),
    )?);
    // Each letter is a token
    doc.add(Field::new(
        "field_characters",
        field_value.to_string(),
        Arc::clone(&matched),
    )?);
    // Every three letters is a token
    doc.add(Field::new(
        "field_tripples",
        field_value.to_string(),
        Arc::clone(&matched),
    )?);
    // Sliced at 10 chars then analyzed just like field
    let units: Vec<u16> = field_value.encode_utf16().collect();
    let end = (units.len() as i32 - 1).clamp(0, 10) as usize;
    doc.add(Field::new(
        "field_sliced",
        String::from_utf16_lossy(&units[..end]),
        Arc::clone(&matched),
    )?);
    // Hacky field containing "der" and "red" at pos = 0
    doc.add(Field::new_with_token_stream(
        "field_der_red",
        canned(vec![token("der", 1, 0, 3), token("red", 0, 0, 3)]),
        matched,
    )?);

    let field: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::with_filter(
        &*random(),
        Arc::clone(&WHITESPACE),
        true,
        Arc::clone(&ENGLISH_STOPSET),
    ));
    let field_exact: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::new(&*random()));
    let field_super_exact: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::with_automaton(
        &*random(),
        Arc::clone(&WHITESPACE),
        false,
    ));
    let field_characters: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::with_automaton(
        &*random(),
        run_automaton(".")?,
        true,
    ));
    let field_tripples: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::with_automaton(
        &*random(),
        run_automaton("...")?,
        true,
    ));
    let analyzer: Arc<dyn Analyzer> = Arc::new(DelegatingAnalyzerWrapper::new(
        Box::new(PerFieldReuseStrategy),
        Box::new(FieldAnalyzers {
            field,
            field_exact,
            field_super_exact,
            field_characters,
            field_tripples,
        }),
    ));

    let dir = LuceneTestCase::new_directory();
    let writer = writer_with(&dir, analyzer)?;
    writer.add_document(&doc)?;

    let highlighter = FastVectorHighlighter::new();
    let frag_list_builder: Box<dyn FragListBuilder> = Box::new(SimpleFragListBuilder::new());
    let fragments_builder: Box<dyn FragmentsBuilder> = Box::new(ScoreOrderFragmentsBuilder::new());
    let reader = open(&writer)?;
    let pre_tags = strings(&["<b>"]);
    let post_tags = strings(&["</b>"]);
    let encoder = DefaultEncoder::new();
    let doc_id = 0;
    let mut query = BooleanQueryBuilder::new();
    for clause in query_clauses {
        query.add(Arc::clone(clause), Occur::MUST)?;
    }
    let q: Arc<dyn Query> = Arc::new(query.build());
    let fq = FieldQuery::new(&q, Some(&reader), true, field_match)?;
    let best_fragments = if use_matched_fields {
        let mut matched_fields: JavaHashSet<String> = JavaHashSet::new();
        for name in [
            "field",
            "field_exact",
            "field_super_exact",
            "field_characters",
            "field_tripples",
            "field_sliced",
            "field_der_red",
        ] {
            matched_fields.add(name.to_string());
        }
        let matched_fields: Vec<&str> = matched_fields.iter().map(String::as_str).collect();
        highlighter.get_best_fragments_matched_fields(
            &fq,
            &*reader,
            doc_id,
            "field",
            &matched_fields,
            25,
            1,
            &*frag_list_builder,
            &*fragments_builder,
            &pre_tags,
            &post_tags,
            &encoder,
        )?
    } else {
        highlighter.get_best_fragments_with(
            &fq,
            &*reader,
            doc_id,
            "field",
            25,
            1,
            &*frag_list_builder,
            &*fragments_builder,
            &pre_tags,
            &post_tags,
            &encoder,
        )?
    }
    .unwrap_or_default();
    Assert::assert_equals(expected.to_string(), best_fragments[0].clone());

    reader.close()?;
    writer.close()?;
    dir.close()?;
    Ok(())
}

/// `clause(field, terms...)`.
fn clause(field: &str, terms: &[&str]) -> Arc<dyn Query> {
    clause_b(field, 1.0, terms)
}

/// `clause(field, boost, terms...)`.
fn clause_b(field: &str, boost: f32, terms: &[&str]) -> Arc<dyn Query> {
    let q: Arc<dyn Query> = if terms.len() == 1 {
        term_query(field, terms[0])
    } else {
        phrase(0, field, terms)
    };
    Arc::new(BoostQuery::new(q, boost).expect("a valid boost"))
}

fn token(term: &str, pos_inc: i32, start_offset: i32, end_offset: i32) -> Token {
    Token::with_pos_inc(term, pos_inc, start_offset, end_offset)
}
