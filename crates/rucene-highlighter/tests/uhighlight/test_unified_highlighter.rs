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

//! Port of `TestUnifiedHighlighter.java`.

use std::any::Any;
use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::{
    Analyzer, DelegatingAnalyzerWrapper, DelegatingAnalyzerWrapperLogic, PerFieldReuseStrategy,
};
use rucene::document::{Document, FieldType, Store};
use rucene::error::Result;
use rucene::index::index_writer::IndexWriter;
use rucene::index::{directory_reader, DirectoryReader, IndexOptions};
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query::Builder as BooleanQueryBuilder;
use rucene::search::fuzzy_query::FuzzyQuery;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::phrase_query::{Builder as PhraseQueryBuilder, PhraseQuery};
use rucene::search::prefix_query::PrefixQuery;
use rucene::search::query::Query;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::score_mode::ScoreMode;
use rucene::search::weight::Weight;
use rucene::search::TermQuery;
use rucene::util::automaton::automata::MAX_STRING_UNION_TERM_LENGTH;
use rucene::util::automaton::{CharacterRunAutomaton, RegExp};
use rucene::util::query_builder::{QueryBuilder, QueryBuilderOps};
use rucene_analysis_common::analysis::core::whitespace_analyzer::WhitespaceAnalyzer;
use rucene_analysis_common::analysis::en::english_analyzer::EnglishAnalyzer;
use rucene_analysis_common::jdk::break_iterator::BreakIterator;
use rucene_highlighter::uhighlight::default_passage_formatter::DefaultPassageFormatter;
use rucene_highlighter::uhighlight::passage::Passage;
use rucene_highlighter::uhighlight::passage_formatter::{FormattedPassages, PassageFormatter};
use rucene_highlighter::uhighlight::uh_components::FieldMatcher;
use rucene_highlighter::uhighlight::unified_highlighter::{
    Builder, HighlightFlag, SortedDocIdIterator, UnifiedHighlighter, UnifiedHighlighterOverrides,
};
use rucene_highlighter::uhighlight::whole_break_iterator::WholeBreakIterator;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::LuceneTestCase;

use crate::highlighter_support::core_support::random;
use crate::highlighter_support::search_support::term;
use crate::highlighter_support::{ti, FnAnalyzer};
use crate::uhighlight::unified_highlighter_test_base::{
    add, assert_null_snippet, assert_snippet, assert_snippets, assertion_error, field, get,
    override_field_matcher_for_tests, parameterized_constructor, parameters_method,
    random_field_type, random_uh_from_builder, random_uh_from_builder_flags,
    random_unified_highlighter, random_unified_highlighter_flags, s, search_index_order, set, tq,
    uh_class, units, UHBase, UHTest,
};

type T = UHTest;

/// `TestUnifiedHighlighter`.
pub fn class_def() -> Arc<Class> {
    uh_class::<T>(
        "org.apache.lucene.search.uhighlight.TestUnifiedHighlighter",
        parameterized_constructor(|ft| UHTest { b: UHBase::new(ft) }),
        vec![
            parameters_method(),
            ti::<T>("testBasics", test_basics),
            ti::<T>(
                "testFormatWithMatchExceedingContentLength2",
                test_format_with_match_exceeding_content_length2,
            ),
            ti::<T>(
                "testFormatWithMatchExceedingContentLength3",
                test_format_with_match_exceeding_content_length3,
            ),
            ti::<T>(
                "testFormatWithMatchExceedingContentLength",
                test_format_with_match_exceeding_content_length,
            ),
            ti::<T>("testHighlightLastWord", test_highlight_last_word),
            ti::<T>("testOneSentence", test_one_sentence),
            ti::<T>("testMaxLengthWithMultivalue", test_max_length_with_multivalue),
            ti::<T>("testMultipleFields", test_multiple_fields),
            ti::<T>("testMultipleTerms", test_multiple_terms),
            ti::<T>("testMultiplePassages", test_multiple_passages),
            ti::<T>("testBuddhism", test_buddhism),
            ti::<T>("testHighlighterDefaultFlags", test_highlighter_default_flags),
            ti::<T>("testCuriousGeorge", test_curious_george),
            ti::<T>("testCambridgeMA", test_cambridge_ma),
            ti::<T>("testPassageRanking", test_passage_ranking),
            ti::<T>("testBooleanMustNot", test_boolean_must_not),
            ti::<T>("testHighlightAllText", test_highlight_all_text),
            ti::<T>("testSpecificDocIDs", test_specific_doc_ids),
            ti::<T>("testCustomFieldValueSource", test_custom_field_value_source),
            ti::<T>("testEmptyHighlights", test_empty_highlights),
            ti::<T>("testNothingAnalyzes", test_nothing_analyzes),
            ti::<T>("testCustomEmptyHighlights", test_custom_empty_highlights),
            ti::<T>("testEmptyHighlightsWhole", test_empty_highlights_whole),
            ti::<T>("testFieldIsMissing", test_field_is_missing),
            ti::<T>("testFieldIsJustSpace", test_field_is_just_space),
            ti::<T>("testFieldIsEmptyString", test_field_is_empty_string),
            ti::<T>("testMultipleDocs", test_multiple_docs),
            ti::<T>("testMultipleSnippetSizes", test_multiple_snippet_sizes),
            ti::<T>("testEncode", test_encode),
            ti::<T>("testObjectFormatter", test_object_formatter),
            ti::<T>("testFieldMatcherTermQuery", test_field_matcher_term_query),
            ti::<T>(
                "testFieldMatcherMultiTermQuery",
                test_field_matcher_multi_term_query,
            ),
            ti::<T>("testMaskedFields", test_masked_fields),
            ti::<T>("testMatchesSlopBug", test_matches_slop_bug),
            ti::<T>("testFieldMatcherPhraseQuery", test_field_matcher_phrase_query),
            ti::<T>(
                "testNestedBooleanQueryAccuracy",
                test_nested_boolean_query_accuracy,
            ),
            ti::<T>("testNotReanalyzed", test_not_reanalyzed),
            ti::<T>(
                "testUnknownQueryWithWeightMatches",
                test_unknown_query_with_weight_matches,
            ),
            ti::<T>("testQueryWithLongTerm", test_query_with_long_term),
        ],
    )
}

/// `new BooleanQuery.Builder().add(…)….build()`.
pub fn bq(clauses: Vec<(Arc<dyn Query>, Occur)>) -> Result<Arc<dyn Query>> {
    let mut builder = BooleanQueryBuilder::new();
    for (query, occur) in clauses {
        builder.add(query, occur)?;
    }
    Ok(Arc::new(builder.build()))
}

/// `new PhraseQuery(slop, field, terms...)`.
pub fn pq(slop: i32, field: &str, terms: &[&str]) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(PhraseQuery::with_slop(slop, field, terms)?))
}

/// `new PhraseQuery.Builder().add(new Term(field, t))….build()`.
pub fn pq_builder(field: &str, terms: &[&str]) -> Result<Arc<dyn Query>> {
    let mut builder = PhraseQueryBuilder::new();
    for t in terms {
        builder.add(term(field, t))?;
    }
    Ok(Arc::new(builder.build()?))
}

/// `newSearcher(ir)`.
pub fn new_searcher(ir: &Arc<dyn DirectoryReader>) -> Arc<IndexSearcher> {
    Arc::new(LuceneTestCase::new_searcher(Arc::clone(ir)))
}

/// `new UnifiedHighlighter.Builder(searcher, indexAnalyzer)`.
pub fn builder(searcher: &Arc<IndexSearcher>, analyzer: Arc<dyn Analyzer>) -> Builder {
    Builder::new(Some(Arc::clone(searcher)), analyzer)
}

/// The common prologue: one document per `values` entry, the `body` field
/// (of `fieldType`) of one reused `Document` changed by `setStringValue`.
fn index_bodies(t: &UHBase, values: &[&str]) -> Result<Arc<dyn DirectoryReader>> {
    let iw = t.writer()?;
    let body = t.field("body", "")?;
    let mut doc = Document::new();
    add(&mut doc, &body);
    for value in values {
        set(&body, value)?;
        iw.add_document(&doc)?;
    }
    let ir = iw.get_reader()?;
    iw.close()?;
    Ok(ir)
}

fn whole_break_iterator() -> Arc<dyn Fn() -> Box<dyn BreakIterator> + Send + Sync> {
    Arc::new(|| Box::new(WholeBreakIterator::new()) as Box<dyn BreakIterator>)
}

fn flags_contain_all(flags: &HashSet<HighlightFlag>, all: &[HighlightFlag]) -> bool {
    all.iter().all(|f| flags.contains(f))
}

//
//  Tests below were ported from the PostingsHighlighter. Possibly augmented.  Far below are newer
// tests.
//

fn test_basics(t: &mut T) -> Result<()> {
    let ir = index_bodies(
        t,
        &[
            "This is a test. Just a test highlighting from postings. Feel free to ignore.",
            "Highlighting the first term. Hope it works.",
        ],
    )?;

    let searcher = new_searcher(&ir);
    let highlighter = random_unified_highlighter(&searcher, t.analyzer());
    let query = tq("body", "highlighting");
    let top_docs = search_index_order(&searcher, &query, 10)?;
    Assert::assert_equals(2, top_docs.total_hits.value());
    let snippets = highlighter.highlight("body", &query, &top_docs)?;
    Assert::assert_equals(2, snippets.len());
    assert_snippet("Just a test <b>highlighting</b> from postings. ", &snippets[0]);
    assert_snippet("<b>Highlighting</b> the first term. ", &snippets[1]);
    ir.close()
}

fn test_format_with_match_exceeding_content_length2(t: &mut T) -> Result<()> {
    let body_text = "123 TEST 01234 TEST";

    let snippets = format_with_match_exceeding_content_length(t, body_text)?;

    Assert::assert_equals(1, snippets.len());
    assert_snippet("123 <b>TEST</b> 01234 TE", &snippets[0]);
    Ok(())
}

fn test_format_with_match_exceeding_content_length3(t: &mut T) -> Result<()> {
    let body_text = "123 5678 01234 TEST TEST";

    let snippets = format_with_match_exceeding_content_length(t, body_text)?;

    Assert::assert_equals(1, snippets.len());
    assert_snippet("123 5678 01234 TE", &snippets[0]);
    Ok(())
}

fn test_format_with_match_exceeding_content_length(t: &mut T) -> Result<()> {
    let body_text = "123 5678 01234 TEST";

    let snippets = format_with_match_exceeding_content_length(t, body_text)?;

    Assert::assert_equals(1, snippets.len());
    // LUCENE-5166: no snippet
    assert_snippet("123 5678 01234 TE", &snippets[0]);
    Ok(())
}

fn format_with_match_exceeding_content_length(
    t: &UHTest,
    body_text: &str,
) -> Result<Vec<Option<String>>> {
    let max_length = 17;
    let ir = index_bodies(t, &[body_text])?;

    let searcher = new_searcher(&ir);

    let query = tq("body", "test");

    let top_docs = search_index_order(&searcher, &query, 10)?;
    Assert::assert_equals(1, top_docs.total_hits.value());

    let uh_builder = builder(&searcher, t.analyzer()).with_max_length(max_length)?;
    let highlighter = random_uh_from_builder(uh_builder);
    let snippets = highlighter.highlight("body", &query, &top_docs)?;

    ir.close()?;
    Ok(snippets)
}

// simple test highlighting last word.
fn test_highlight_last_word(t: &mut T) -> Result<()> {
    let ir = index_bodies(t, &["This is a test"])?;

    let searcher = new_searcher(&ir);
    let highlighter = random_unified_highlighter(&searcher, t.analyzer());
    let query = tq("body", "test");
    let top_docs = search_index_order(&searcher, &query, 10)?;
    Assert::assert_equals(1, top_docs.total_hits.value());
    let snippets = highlighter.highlight("body", &query, &top_docs)?;
    Assert::assert_equals(1, snippets.len());
    assert_snippet("This is a <b>test</b>", &snippets[0]);
    ir.close()
}

// simple test with one sentence documents.
fn test_one_sentence(t: &mut T) -> Result<()> {
    let ir = index_bodies(t, &["This is a test.", "Test a one sentence document."])?;

    let searcher = new_searcher(&ir);
    let highlighter = random_unified_highlighter(&searcher, t.analyzer());
    let query = tq("body", "test");
    let top_docs = search_index_order(&searcher, &query, 10)?;
    Assert::assert_equals(2, top_docs.total_hits.value());
    let snippets = highlighter.highlight("body", &query, &top_docs)?;
    Assert::assert_equals(2, snippets.len());
    assert_snippet("This is a <b>test</b>.", &snippets[0]);
    assert_snippet("<b>Test</b> a one sentence document.", &snippets[1]);
    ir.close()
}

// simple test with multiple values that make a result longer than maxLength.
fn test_max_length_with_multivalue(t: &mut T) -> Result<()> {
    let iw = t.writer()?;

    let mut doc = Document::new();

    let value = "This is a multivalued field. Sentencetwo field.";
    add(&mut doc, &t.field("body", value)?);
    add(&mut doc, &t.field("body", value)?);
    add(&mut doc, &t.field("body", value)?);

    iw.add_document(&doc)?;

    let ir = iw.get_reader()?;
    iw.close()?;

    let searcher = new_searcher(&ir);
    let uh_builder =
        builder(&searcher, t.analyzer()).with_max_length(value.len() as i32 * 2 + 1)?;
    let highlighter = random_uh_from_builder(uh_builder);
    let query = tq("body", "field");
    let top_docs = search_index_order(&searcher, &query, 10)?;
    Assert::assert_equals(1, top_docs.total_hits.value());
    let snippets = highlighter.highlight_max_passages("body", &query, &top_docs, 10)?;
    Assert::assert_equals(1, snippets.len());
    let highlighted_value = "This is a multivalued <b>field</b>. Sentencetwo <b>field</b>.";
    assert_snippet(
        &format!("{highlighted_value}... {highlighted_value}"),
        &snippets[0],
    );

    ir.close()
}

fn test_multiple_fields(t: &mut T) -> Result<()> {
    let iw = t.writer()?;

    let body = t.field("body", "")?;
    let title = field("title", "", &random_field_type(&*random(), &[]))?;
    let mut doc = Document::new();
    add(&mut doc, &body);
    add(&mut doc, &title);

    set(
        &body,
        "This is a test. Just a test highlighting from postings. Feel free to ignore.",
    )?;
    set(&title, "I am hoping for the best.")?;
    iw.add_document(&doc)?;
    set(&body, "Highlighting the first term. Hope it works.")?;
    set(&title, "But best may not be good enough.")?;
    iw.add_document(&doc)?;

    let ir = iw.get_reader()?;
    iw.close()?;

    let searcher = new_searcher(&ir);
    let highlighter = random_unified_highlighter(&searcher, t.analyzer());
    let query = bq(vec![
        (tq("body", "highlighting"), Occur::SHOULD),
        (tq("title", "best"), Occur::SHOULD),
    ])?;
    let top_docs = search_index_order(&searcher, &query, 10)?;
    Assert::assert_equals(2, top_docs.total_hits.value());
    let snippets = highlighter.highlight_fields(
        &["body".to_string(), "title".to_string()],
        &query,
        &top_docs,
    )?;
    Assert::assert_equals(2, snippets.len());
    assert_snippet(
        "Just a test <b>highlighting</b> from postings. ",
        &get(&snippets, "body")[0],
    );
    assert_snippet(
        "<b>Highlighting</b> the first term. ",
        &get(&snippets, "body")[1],
    );
    assert_snippet(
        "I am hoping for the <b>best</b>.",
        &get(&snippets, "title")[0],
    );
    assert_snippet(
        "But <b>best</b> may not be good enough.",
        &get(&snippets, "title")[1],
    );
    ir.close()
}

fn test_multiple_terms(t: &mut T) -> Result<()> {
    let ir = index_bodies(
        t,
        &[
            "This is a test. Just a test highlighting from postings. Feel free to ignore.",
            "Highlighting the first term. Hope it works.",
        ],
    )?;

    let searcher = new_searcher(&ir);
    let highlighter = random_unified_highlighter(&searcher, t.analyzer());
    let query = bq(vec![
        (tq("body", "highlighting"), Occur::SHOULD),
        (tq("body", "just"), Occur::SHOULD),
        (tq("body", "first"), Occur::SHOULD),
    ])?;
    let top_docs = search_index_order(&searcher, &query, 10)?;
    Assert::assert_equals(2, top_docs.total_hits.value());
    let snippets = highlighter.highlight("body", &query, &top_docs)?;
    Assert::assert_equals(2, snippets.len());
    assert_snippet(
        "<b>Just</b> a test <b>highlighting</b> from postings. ",
        &snippets[0],
    );
    assert_snippet("<b>Highlighting</b> the <b>first</b> term. ", &snippets[1]);
    ir.close()
}

fn test_multiple_passages(t: &mut T) -> Result<()> {
    let ir = index_bodies(
        t,
        &[
            "This is a test. Just a test highlighting from postings. Feel free to ignore.",
            "This test is another test. Not a good sentence. Test test test test.",
        ],
    )?;

    let searcher = new_searcher(&ir);
    let highlighter = random_unified_highlighter(&searcher, t.analyzer());
    let query = tq("body", "test");
    let top_docs = search_index_order(&searcher, &query, 10)?;
    Assert::assert_equals(2, top_docs.total_hits.value());
    let snippets = highlighter.highlight_max_passages("body", &query, &top_docs, 2)?;
    Assert::assert_equals(2, snippets.len());
    assert_snippet(
        "This is a <b>test</b>. Just a <b>test</b> highlighting from postings. ",
        &snippets[0],
    );
    assert_snippet(
        "This <b>test</b> is another <b>test</b>. ... <b>Test</b> <b>test</b> <b>test</b> <b>test</b>.",
        &snippets[1],
    );
    ir.close()
}

fn test_buddhism(t: &mut T) -> Result<()> {
    let text = concat!(
        "This eight-volume set brings together seminal papers in Buddhist studies from a vast ",
        "range of academic disciplines published over the last forty years. With a new introduction ",
        "by the editor, this collection is a unique and unrivalled research resource for both ",
        "student and scholar. Coverage includes: - Buddhist origins; early history of Buddhism in ",
        "South and Southeast Asia - early Buddhist Schools and Doctrinal History; Theravada Doctrine ",
        "- the Origins and nature of Mahayana Buddhism; some Mahayana religious topics - Abhidharma ",
        "and Madhyamaka - Yogacara, the Epistemological tradition, and Tathagatagarbha - Tantric ",
        "Buddhism (Including China and Japan); Buddhism in Nepal and Tibet - Buddhism in South and ",
        "Southeast Asia, and - Buddhism in China, East Asia, and Japan."
    );
    let ir = index_bodies(t, &[text])?;
    let searcher = new_searcher(&ir);
    let query = pq_builder("body", &["buddhist", "origins"])?;
    let top_docs = searcher.search(Arc::clone(&query), 10)?;
    Assert::assert_equals(1, top_docs.total_hits.value());
    let uh_builder = builder(&searcher, t.analyzer()).with_highlight_phrases_strictly(false);
    let highlighter = random_uh_from_builder(uh_builder);
    let snippets = highlighter.highlight_max_passages("body", &query, &top_docs, 2)?;
    Assert::assert_equals(1, snippets.len());
    if flags_contain_all(
        &highlighter.get_flags("body"),
        &[HighlightFlag::WEIGHT_MATCHES, HighlightFlag::PHRASES],
    ) {
        Assert::assert_true_msg(
            s(&snippets[0]),
            s(&snippets[0]).contains("<b>Buddhist origins</b>"),
        );
    } else {
        Assert::assert_true_msg(
            s(&snippets[0]),
            s(&snippets[0]).contains("<b>Buddhist</b> <b>origins</b>"),
        );
    }
    ir.close()
}

fn test_highlighter_default_flags(t: &mut T) -> Result<()> {
    let iw = t.writer()?;
    let mut document = Document::new();
    add(&mut document, &t.field("body", "test body")?);
    iw.add_document(&document)?;
    let ir = iw.get_reader()?;
    iw.close()?;
    let searcher = new_searcher(&ir);
    let highlighter = UnifiedHighlighter::builder(Arc::clone(&searcher), t.analyzer()).build();
    let flags = highlighter.get_flags("body");
    Assert::assert_true(flags.contains(&HighlightFlag::PHRASES));
    Assert::assert_true(flags.contains(&HighlightFlag::MULTI_TERM_QUERY));
    Assert::assert_true(flags.contains(&HighlightFlag::PASSAGE_RELEVANCY_OVER_SPEED));
    Assert::assert_true(flags.contains(&HighlightFlag::WEIGHT_MATCHES));
    // if more flags are added, bump the number below and add an assertTrue or assertFalse above
    Assert::assert_equals(
        4,
        [
            HighlightFlag::PHRASES,
            HighlightFlag::MULTI_TERM_QUERY,
            HighlightFlag::PASSAGE_RELEVANCY_OVER_SPEED,
            HighlightFlag::WEIGHT_MATCHES,
        ]
        .len(),
    );
    ir.close()
}

fn test_curious_george(t: &mut T) -> Result<()> {
    let text = concat!(
        "It\u{2019}s the formula for success for preschoolers\u{2014}Curious George and fire trucks! ",
        "Curious George and the Firefighters is a story based on H. A. and Margret Rey\u{2019}s ",
        "popular primate and painted in the original watercolor and charcoal style. ",
        "Firefighters are a famously brave lot, but can they withstand a visit from one curious monkey?"
    );
    let ir = index_bodies(t, &[text])?;
    let searcher = new_searcher(&ir);
    let query = pq_builder("body", &["curious", "george"])?;
    let top_docs = searcher.search(Arc::clone(&query), 10)?;
    Assert::assert_equals(1, top_docs.total_hits.value());
    let uh_builder = builder(&searcher, t.analyzer()).with_highlight_phrases_strictly(false);
    let highlighter = random_uh_from_builder(uh_builder);
    let snippets = highlighter.highlight_max_passages("body", &query, &top_docs, 2)?;
    Assert::assert_equals(1, snippets.len());
    Assert::assert_false(s(&snippets[0]).contains("<b>Curious</b>Curious"));
    ir.close()
}

/// The resource `CambridgeMA.utf8` of the test package.
const CAMBRIDGE_MA: &str = include_str!("CambridgeMA.utf8");

fn test_cambridge_ma(t: &mut T) -> Result<()> {
    // BufferedReader.readLine(): the first line, without its terminator.
    let text = CAMBRIDGE_MA
        .split(['\n', '\r'])
        .next()
        .expect("a first line");
    let ir = index_bodies(t, &[text])?;
    let searcher = new_searcher(&ir);
    let query = bq(vec![
        (tq("body", "porter"), Occur::SHOULD),
        (tq("body", "square"), Occur::SHOULD),
        (tq("body", "massachusetts"), Occur::SHOULD),
    ])?;
    let top_docs = searcher.search(Arc::clone(&query), 10)?;
    Assert::assert_equals(1, top_docs.total_hits.value());
    let uh_builder = builder(&searcher, t.analyzer()).with_max_length(i32::MAX - 1)?;
    let highlighter = random_uh_from_builder(uh_builder);
    let snippets = highlighter.highlight_max_passages("body", &query, &top_docs, 2)?;
    Assert::assert_equals(1, snippets.len());
    Assert::assert_true(s(&snippets[0]).contains("<b>Square</b>"));
    Assert::assert_true(s(&snippets[0]).contains("<b>Porter</b>"));
    ir.close()
}

fn test_passage_ranking(t: &mut T) -> Result<()> {
    let ir = index_bodies(t, &["This is a test.  Just highlighting from postings. This is also a much sillier test.  Feel free to test test test test test test test."])?;

    let searcher = new_searcher(&ir);
    let highlighter = random_unified_highlighter(&searcher, t.analyzer());
    let query = tq("body", "test");
    let top_docs = search_index_order(&searcher, &query, 10)?;
    Assert::assert_equals(1, top_docs.total_hits.value());
    let snippets = highlighter.highlight_max_passages("body", &query, &top_docs, 2)?;
    Assert::assert_equals(1, snippets.len());
    assert_snippet(
        "This is a <b>test</b>.  ... Feel free to <b>test</b> <b>test</b> <b>test</b> <b>test</b> <b>test</b> <b>test</b> <b>test</b>.",
        &snippets[0],
    );
    ir.close()
}

fn test_boolean_must_not(t: &mut T) -> Result<()> {
    let ir = index_bodies(
        t,
        &["This sentence has both terms.  This sentence has only terms."],
    )?;

    let searcher = new_searcher(&ir);

    let query2 = bq(vec![(tq("body", "both"), Occur::MUST_NOT)])?;

    let query = bq(vec![
        (tq("body", "terms"), Occur::SHOULD),
        (query2, Occur::SHOULD),
    ])?;

    let top_docs = searcher.search(Arc::clone(&query), 10)?;
    Assert::assert_equals(1, top_docs.total_hits.value());
    let uh_builder = builder(&searcher, t.analyzer()).with_max_length(i32::MAX - 1)?;
    let highlighter = random_uh_from_builder(uh_builder);
    let snippets = highlighter.highlight_max_passages("body", &query, &top_docs, 2)?;
    Assert::assert_equals(1, snippets.len());
    Assert::assert_false(s(&snippets[0]).contains("<b>both</b>"));
    ir.close()
}

fn test_highlight_all_text(t: &mut T) -> Result<()> {
    let ir = index_bodies(t, &["This is a test.  Just highlighting from postings. This is also a much sillier test.  Feel free to test test test test test test test."])?;

    let searcher = new_searcher(&ir);
    let uh_builder = builder(&searcher, t.analyzer())
        .with_break_iterator(whole_break_iterator())
        .with_max_length(10000)?;
    let highlighter = random_uh_from_builder(uh_builder);
    let query = tq("body", "test");
    let top_docs = search_index_order(&searcher, &query, 10)?;
    Assert::assert_equals(1, top_docs.total_hits.value());
    let snippets = highlighter.highlight_max_passages("body", &query, &top_docs, 2)?;
    Assert::assert_equals(1, snippets.len());
    assert_snippet(
        "This is a <b>test</b>.  Just highlighting from postings. This is also a much sillier <b>test</b>.  Feel free to <b>test</b> <b>test</b> <b>test</b> <b>test</b> <b>test</b> <b>test</b> <b>test</b>.",
        &snippets[0],
    );
    ir.close()
}

fn test_specific_doc_ids(t: &mut T) -> Result<()> {
    let ir = index_bodies(
        t,
        &[
            "This is a test. Just a test highlighting from postings. Feel free to ignore.",
            "Highlighting the first term. Hope it works.",
        ],
    )?;

    let searcher = new_searcher(&ir);
    let highlighter = random_unified_highlighter(&searcher, t.analyzer());
    let query = tq("body", "highlighting");
    let top_docs = search_index_order(&searcher, &query, 10)?;
    Assert::assert_equals(2, top_docs.total_hits.value());
    let hits = &top_docs.score_docs;
    let doc_ids = [hits[0].doc, hits[1].doc];
    let snippets = highlighter.highlight_fields_doc_ids(&["body".to_string()], &query, &doc_ids, &[1])?;
    let snippets = get(&snippets, "body");
    Assert::assert_equals(2, snippets.len());
    assert_snippet("Just a test <b>highlighting</b> from postings. ", &snippets[0]);
    assert_snippet("<b>Highlighting</b> the first term. ", &snippets[1]);
    ir.close()
}

/// The overrides of the anonymous `UnifiedHighlighter` of
/// `testCustomFieldValueSource`.
#[derive(Debug)]
struct CustomFieldValueSource {
    text: String,
}

impl UnifiedHighlighterOverrides for CustomFieldValueSource {
    fn load_field_values(
        &self,
        _this: &UnifiedHighlighter,
        fields: &[String],
        doc_iter: &mut SortedDocIdIterator<'_>,
        _cache_chars_threshold: i32,
    ) -> Result<Vec<Vec<Option<Vec<u16>>>>> {
        assert!(fields.len() == 1);
        assert!(doc_iter.cost() == 1);
        doc_iter.next_doc();
        Ok(vec![vec![Some(units(&self.text))]])
    }

    fn get_break_iterator(&self, _this: &UnifiedHighlighter, _field: &str) -> Box<dyn BreakIterator> {
        Box::new(WholeBreakIterator::new())
    }
}

fn test_custom_field_value_source(t: &mut T) -> Result<()> {
    let text = "This is a test.  Just highlighting from postings. This is also a much sillier test.  Feel free to test test test test test test test.";
    let ir = index_bodies(t, &[text])?;

    let searcher = new_searcher(&ir);

    let uh_builder = builder(&searcher, t.analyzer());
    let mut highlighter = UnifiedHighlighter::from_builder(uh_builder);
    highlighter.set_overrides(Arc::new(CustomFieldValueSource {
        text: text.to_string(),
    }));

    let query = tq("body", "test");
    let top_docs = search_index_order(&searcher, &query, 10)?;
    Assert::assert_equals(1, top_docs.total_hits.value());
    let snippets = highlighter.highlight_max_passages("body", &query, &top_docs, 2)?;
    Assert::assert_equals(1, snippets.len());
    assert_snippet(
        "This is a <b>test</b>.  Just highlighting from postings. This is also a much sillier <b>test</b>.  Feel free to <b>test</b> <b>test</b> <b>test</b> <b>test</b> <b>test</b> <b>test</b> <b>test</b>.",
        &snippets[0],
    );
    ir.close()
}

/// Make sure highlighter returns first N sentences if there were no hits.
fn test_empty_highlights(t: &mut T) -> Result<()> {
    let ir = index_bodies(
        t,
        &["test this is.  another sentence this test has.  far away is that planet."],
    )?;

    let searcher = new_searcher(&ir);
    let highlighter = random_unified_highlighter(&searcher, t.analyzer());
    let query = tq("body", "highlighting");
    let doc_ids = [0];
    let snippets = highlighter.highlight_fields_doc_ids(&["body".to_string()], &query, &doc_ids, &[2])?;
    let snippets = get(&snippets, "body");
    Assert::assert_equals(1, snippets.len());
    assert_snippet("test this is.  another sentence this test has.  ", &snippets[0]);
    ir.close()
}

/// The two documents of `testNothingAnalyzes` and its kin: `body` of the
/// first is `first_body`, with an `id` field.
fn index_id_and_something(t: &UHBase, first_body: &str) -> Result<Arc<dyn DirectoryReader>> {
    let iw = t.writer()?;

    let mut doc = Document::new();
    add(&mut doc, &t.field("body", first_body)?);
    doc.add(LuceneTestCase::new_text_field("id", "id", Store::YES));
    iw.add_document(&doc)?;

    doc = Document::new();
    add(&mut doc, &t.field("body", "something")?);
    iw.add_document(&doc)?;

    let ir = iw.get_reader()?;
    iw.close()?;
    Ok(ir)
}

/// The highlights of `body` for the `id:id` document, with 2 passages.
fn highlight_id_doc(t: &UHBase, ir: &Arc<dyn DirectoryReader>) -> Result<Vec<Option<String>>> {
    let searcher = new_searcher(ir);
    let highlighter = random_unified_highlighter(&searcher, t.analyzer());
    let doc_id = searcher.search(tq("id", "id"), 1)?.score_docs[0].doc;

    let query = tq("body", "highlighting");
    let doc_ids = [doc_id];
    let mut snippets = highlighter.highlight_fields_doc_ids(&["body".to_string()], &query, &doc_ids, &[2])?;
    Ok(snippets.remove("body").expect("NullPointerException: no body snippets"))
}

/// Not empty but nothing analyzes. Ensures we address null term-vectors.
fn test_nothing_analyzes(t: &mut T) -> Result<()> {
    let ir = index_id_and_something(t, " ")?; // just a space! (thus not empty)
    let snippets = highlight_id_doc(t, &ir)?;
    Assert::assert_equals(1, snippets.len());
    assert_snippet(" ", &snippets[0]);
    ir.close()
}

/// The single document `body` of the "no highlights" tests.
const PLANET: &str = "test this is.  another sentence this test has.  far away is that planet.";

/// Make sure highlighter we can customize how emtpy highlight is returned.
fn test_custom_empty_highlights(t: &mut T) -> Result<()> {
    t.mock().set_position_increment_gap(10);
    let ir = index_bodies(t, &[PLANET])?;

    let searcher = new_searcher(&ir);
    let uh_builder = builder(&searcher, t.analyzer()).with_max_no_highlight_passages(0); // don't want any default summary
    let highlighter = random_uh_from_builder(uh_builder);
    let query = tq("body", "highlighting");
    let doc_ids = [0];
    let snippets = highlighter.highlight_fields_doc_ids(&["body".to_string()], &query, &doc_ids, &[2])?;
    let snippets = get(&snippets, "body");
    Assert::assert_equals(1, snippets.len());
    assert_null_snippet(&snippets[0]);
    ir.close()
}

/// Make sure highlighter returns whole text when there are no hits and
/// BreakIterator is null.
fn test_empty_highlights_whole(t: &mut T) -> Result<()> {
    let ir = index_bodies(t, &[PLANET])?;

    let searcher = new_searcher(&ir);
    let highlighter = UnifiedHighlighter::builder(Arc::clone(&searcher), t.analyzer())
        .with_break_iterator(whole_break_iterator())
        .build();
    let query = tq("body", "highlighting");
    let doc_ids = [0];
    let snippets = highlighter.highlight_fields_doc_ids(&["body".to_string()], &query, &doc_ids, &[2])?;
    let snippets = get(&snippets, "body");
    Assert::assert_equals(1, snippets.len());
    assert_snippet(PLANET, &snippets[0]);
    ir.close()
}

/// Make sure highlighter is OK with entirely missing field.
fn test_field_is_missing(t: &mut T) -> Result<()> {
    let ir = index_bodies(t, &[PLANET])?;

    let searcher = new_searcher(&ir);
    let highlighter = random_unified_highlighter(&searcher, t.analyzer());
    let query = tq("bogus", "highlighting");
    let doc_ids = [0];
    let snippets = highlighter.highlight_fields_doc_ids(&["bogus".to_string()], &query, &doc_ids, &[2])?;
    let snippets = get(&snippets, "bogus");
    Assert::assert_equals(1, snippets.len());
    assert_null_snippet(&snippets[0]);
    ir.close()
}

fn test_field_is_just_space(t: &mut T) -> Result<()> {
    let ir = index_id_and_something(t, "   ")?;
    let snippets = highlight_id_doc(t, &ir)?;
    Assert::assert_equals(1, snippets.len());
    assert_snippet("   ", &snippets[0]);
    ir.close()
}

fn test_field_is_empty_string(t: &mut T) -> Result<()> {
    let ir = index_id_and_something(t, "")?;
    let snippets = highlight_id_doc(t, &ir)?;
    Assert::assert_equals(1, snippets.len());
    assert_null_snippet(&snippets[0]);
    ir.close()
}

fn test_multiple_docs(t: &mut T) -> Result<()> {
    let iw = t.writer()?;
    let num_docs = LuceneTestCase::at_least(100);
    for i in 0..num_docs {
        let mut doc = Document::new();
        let mut content = format!("the answer is {i}");
        if (i & 1) == 0 {
            content.push_str(" some more terms");
        }
        add(&mut doc, &t.field("body", &content)?);
        doc.add(LuceneTestCase::new_string_field("id", &i.to_string(), Store::YES));
        iw.add_document(&doc)?;

        if random().next_int_bound(10) == 2 {
            iw.commit()?;
        }
    }

    let ir = iw.get_reader()?;
    iw.close()?;

    let searcher = new_searcher(&ir);
    let uh_builder = builder(&searcher, t.analyzer())
        .with_cache_field_val_chars_threshold(random().next_int_bound(10) * 10); // 0 thru 90 intervals of 10
    let highlighter = random_uh_from_builder(uh_builder);
    let query = tq("body", "answer");
    let hits = searcher.search(Arc::clone(&query), num_docs)?;
    Assert::assert_equals(num_docs as i64, hits.total_hits.value());

    let snippets = highlighter.highlight("body", &query, &hits)?;
    Assert::assert_equals(num_docs as usize, snippets.len());
    let stored_fields = searcher.stored_fields()?;
    for hit in 0..num_docs as usize {
        let doc = stored_fields.document(hits.score_docs[hit].doc)?;
        let id: i32 = doc
            .get("id")
            .expect("NullPointerException: no id")
            .parse()
            .expect("NumberFormatException");
        let mut expected = format!("the <b>answer</b> is {id}");
        if (id & 1) == 0 {
            expected.push_str(" some more terms");
        }
        assert_snippet(&expected, &snippets[hit]);
    }
    ir.close()
}

fn test_multiple_snippet_sizes(t: &mut T) -> Result<()> {
    let iw = t.writer()?;

    let body = t.field("body", "")?;
    let title = field("title", "", &random_field_type(&*random(), &[]))?;
    let mut doc = Document::new();
    add(&mut doc, &body);
    add(&mut doc, &title);

    set(
        &body,
        "This is a test. Just a test highlighting from postings. Feel free to ignore.",
    )?;
    set(
        &title,
        "This is a test. Just a test highlighting from postings. Feel free to ignore.",
    )?;
    iw.add_document(&doc)?;

    let ir = iw.get_reader()?;
    iw.close()?;

    let searcher = new_searcher(&ir);
    let highlighter = random_unified_highlighter(&searcher, t.analyzer());
    let query = bq(vec![
        (tq("body", "test"), Occur::SHOULD),
        (tq("title", "test"), Occur::SHOULD),
    ])?;
    let snippets = highlighter.highlight_fields_doc_ids(
        &["title".to_string(), "body".to_string()],
        &query,
        &[0],
        &[1, 2],
    )?;
    let title_highlight = &get(&snippets, "title")[0];
    let body_highlight = &get(&snippets, "body")[0];
    assert_snippet("This is a <b>test</b>. ", title_highlight);
    assert_snippet(
        "This is a <b>test</b>. Just a <b>test</b> highlighting from postings. ",
        body_highlight,
    );
    ir.close()
}

fn test_encode(t: &mut T) -> Result<()> {
    let ir = index_bodies(
        t,
        &["This is a test. Just a test highlighting from <i>postings</i>. Feel free to ignore."],
    )?;

    let searcher = new_searcher(&ir);
    let highlighter = UnifiedHighlighter::builder(Arc::clone(&searcher), t.analyzer())
        .with_formatter(Arc::new(DefaultPassageFormatter::with_tags(
            "<b>", "</b>", "... ", true,
        )))
        .build();
    let query = tq("body", "highlighting");
    let top_docs = search_index_order(&searcher, &query, 10)?;
    Assert::assert_equals(1, top_docs.total_hits.value());
    let snippets = highlighter.highlight("body", &query, &top_docs)?;
    Assert::assert_equals(1, snippets.len());
    assert_snippet(
        "Just a test <b>highlighting</b> from &lt;i&gt;postings&lt;&#x2F;i&gt;. ",
        &snippets[0],
    );
    ir.close()
}

/// The `String[]` an object formatter returns.
#[derive(Debug, PartialEq)]
struct StringArray(Vec<String>);

impl fmt::Display for StringArray {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Object.toString() of a String[]: its class name and identity.
        write!(f, "[Ljava.lang.String;@{:x}", self as *const Self as usize as u32)
    }
}

/// The anonymous `PassageFormatter` of `testObjectFormatter`.
#[derive(Debug)]
struct ArrayPassageFormatter {
    default_formatter: DefaultPassageFormatter,
}

impl PassageFormatter for ArrayPassageFormatter {
    fn format(&self, passages: &[Passage], content: &str) -> Result<Box<dyn FormattedPassages>> {
        // Just turns the String snippet into a length 2
        // array of String
        Ok(Box::new(StringArray(vec![
            "blah blah".to_string(),
            self.default_formatter.format(passages, content)?.to_string(),
        ])))
    }
}

// LUCENE-4906
fn test_object_formatter(t: &mut T) -> Result<()> {
    let ir = index_bodies(
        t,
        &["This is a test. Just a test highlighting from postings. Feel free to ignore."],
    )?;

    let searcher = new_searcher(&ir);
    let passage_formatter = Arc::new(ArrayPassageFormatter {
        default_formatter: DefaultPassageFormatter::new(),
    });

    let highlighter = UnifiedHighlighter::builder(Arc::clone(&searcher), t.analyzer())
        .with_formatter(passage_formatter)
        .build();
    let query = tq("body", "highlighting");
    let top_docs = search_index_order(&searcher, &query, 10)?;
    Assert::assert_equals(1, top_docs.total_hits.value());
    let doc_ids = [top_docs.score_docs[0].doc];
    let snippets =
        highlighter.highlight_fields_as_objects(&["body".to_string()], &query, &doc_ids, &[1])?;
    let body_snippets = snippets
        .get("body")
        .expect("NullPointerException: no body snippets");
    Assert::assert_equals(1, body_snippets.len());
    let array = body_snippets[0]
        .as_ref()
        .and_then(|o| (**o).as_any().downcast_ref::<StringArray>())
        .unwrap_or_else(|| panic!("java.lang.ClassCastException: not a String[]"));
    Assert::assert_true(
        array
            == &StringArray(vec![
                "blah blah".to_string(),
                "Just a test <b>highlighting</b> from postings. ".to_string(),
            ]),
    );
    ir.close()
}

fn index_some_fields(t: &UHBase) -> Result<Arc<dyn DirectoryReader>> {
    let iw = t.writer()?;
    let ft = FieldType::new();
    ft.set_index_options(IndexOptions::NONE)?;
    ft.set_tokenized(false)?;
    ft.set_stored(true)?;
    ft.freeze();

    let title = t.field("title", "")?;
    let text = t.field("text", "")?;
    let category = t.field("category", "")?;

    let mut doc = Document::new();
    add(&mut doc, &title);
    add(&mut doc, &text);
    add(&mut doc, &category);
    set(&title, "This is the title field.")?;
    set(
        &text,
        "This is the text field. You can put some text if you want.",
    )?;
    set(&category, "This is the category field.")?;
    iw.add_document(&doc)?;

    let ir = iw.get_reader()?;
    iw.close()?;
    Ok(ir)
}

fn always() -> Option<FieldMatcher> {
    Some(Arc::new(|_qf: &str| true))
}

fn equals(name: &'static str) -> Option<FieldMatcher> {
    Some(Arc::new(move |f: &str| f == name))
}

/// `highlighter.highlight(field, query, topDocs, 10)`, asserting one
/// snippet, which is returned.
fn one_snippet(
    highlighter: &UnifiedHighlighter,
    field: &str,
    query: &Arc<dyn Query>,
    top_docs: &rucene::search::TopDocs,
) -> Result<Option<String>> {
    let mut snippets = highlighter.highlight_max_passages(field, query, top_docs, 10)?;
    Assert::assert_equals(1, snippets.len());
    Ok(snippets.remove(0))
}

/// The checks of `testFieldMatcherTermQuery` and
/// `testFieldMatcherMultiTermQuery`, whose expectations are the same.
fn field_matcher_checks(
    searcher: &Arc<IndexSearcher>,
    query: &Arc<dyn Query>,
    highlighter_no_field_match: &UnifiedHighlighter,
    mut highlighter_field_match: UnifiedHighlighter,
) -> Result<()> {
    // title
    {
        let top_docs = search_index_order(searcher, query, 10)?;
        Assert::assert_equals(1, top_docs.total_hits.value());
        let mut snippet = one_snippet(highlighter_no_field_match, "title", query, &top_docs)?;
        assert_snippet("<b>This</b> <b>is</b> the title <b>field</b>.", &snippet);

        snippet = one_snippet(&highlighter_field_match, "title", query, &top_docs)?;
        assert_snippet("<b>This</b> <b>is</b> the title field.", &snippet);

        highlighter_field_match =
            override_field_matcher_for_tests(&highlighter_field_match, equals("text"), "text");
        snippet = one_snippet(&highlighter_field_match, "title", query, &top_docs)?;
        assert_snippet("<b>This</b> is the title <b>field</b>.", &snippet);
        highlighter_field_match =
            override_field_matcher_for_tests(&highlighter_field_match, None, "text");
    }

    // text
    {
        let top_docs = search_index_order(searcher, query, 10)?;
        Assert::assert_equals(1, top_docs.total_hits.value());
        let mut snippet = one_snippet(highlighter_no_field_match, "text", query, &top_docs)?;
        assert_snippet(
            "<b>This</b> <b>is</b> the text <b>field</b>. You can put <b>some</b> text if you want.",
            &snippet,
        );

        snippet = one_snippet(&highlighter_field_match, "text", query, &top_docs)?;
        assert_snippet(
            "<b>This</b> is the text <b>field</b>. You can put <b>some</b> text if you want.",
            &snippet,
        );

        highlighter_field_match =
            override_field_matcher_for_tests(&highlighter_field_match, equals("title"), "title");
        snippet = one_snippet(&highlighter_field_match, "text", query, &top_docs)?;
        assert_snippet("<b>This</b> <b>is</b> the text field. ", &snippet);
        highlighter_field_match =
            override_field_matcher_for_tests(&highlighter_field_match, None, "title");
    }

    // category
    {
        let top_docs = search_index_order(searcher, query, 10)?;
        Assert::assert_equals(1, top_docs.total_hits.value());
        let mut snippet = one_snippet(highlighter_no_field_match, "category", query, &top_docs)?;
        assert_snippet(
            "<b>This</b> <b>is</b> the <b>category</b> <b>field</b>.",
            &snippet,
        );

        snippet = one_snippet(&highlighter_field_match, "category", query, &top_docs)?;
        assert_snippet("<b>This</b> is the <b>category</b> field.", &snippet);

        highlighter_field_match =
            override_field_matcher_for_tests(&highlighter_field_match, equals("title"), "title");
        snippet = one_snippet(&highlighter_field_match, "category", query, &top_docs)?;
        assert_snippet("<b>This</b> <b>is</b> the category field.", &snippet);
        let _ = override_field_matcher_for_tests(&highlighter_field_match, None, "title");
    }
    Ok(())
}

fn test_field_matcher_term_query(t: &mut T) -> Result<()> {
    let ir = index_some_fields(t)?;
    let searcher = new_searcher(&ir);
    let highlighter_no_field_match = UnifiedHighlighter::builder(Arc::clone(&searcher), t.analyzer())
        .with_field_matcher(always())
        .build();
    let uh_builder = builder(&searcher, t.analyzer());
    let highlighter_field_match =
        override_field_matcher_for_tests(&random_uh_from_builder(uh_builder), None, "text");
    let query = bq(vec![
        (tq("text", "some"), Occur::SHOULD),
        (tq("text", "field"), Occur::SHOULD),
        (tq("text", "this"), Occur::SHOULD),
        (tq("title", "is"), Occur::SHOULD),
        (tq("title", "this"), Occur::SHOULD),
        (tq("category", "this"), Occur::SHOULD),
        (tq("category", "some"), Occur::SHOULD),
        (tq("category", "category"), Occur::SHOULD),
    ])?;
    field_matcher_checks(
        &searcher,
        &query,
        &highlighter_no_field_match,
        highlighter_field_match,
    )?;
    ir.close()
}

fn fuzzy(field: &str, text: &str, max_edits: i32) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(FuzzyQuery::with_max_edits(term(field, text), max_edits)?))
}

fn prefix(field: &str, text: &str) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(PrefixQuery::new(term(field, text))?))
}

fn test_field_matcher_multi_term_query(t: &mut T) -> Result<()> {
    let ir = index_some_fields(t)?;
    let searcher = new_searcher(&ir);
    let highlighter_no_field_match = UnifiedHighlighter::builder(Arc::clone(&searcher), t.analyzer())
        .with_field_matcher(always())
        .build();
    let uh_builder = builder(&searcher, t.analyzer());
    let highlighter_field_match = override_field_matcher_for_tests(
        &random_uh_from_builder_flags(uh_builder, &[HighlightFlag::MULTI_TERM_QUERY], None),
        None,
        "text",
    );
    let query = bq(vec![
        (fuzzy("text", "sime", 1)?, Occur::SHOULD),
        (prefix("text", "fie")?, Occur::SHOULD),
        (prefix("text", "thi")?, Occur::SHOULD),
        (tq("title", "is"), Occur::SHOULD),
        (prefix("title", "thi")?, Occur::SHOULD),
        (prefix("category", "thi")?, Occur::SHOULD),
        (fuzzy("category", "sime", 1)?, Occur::SHOULD),
        (prefix("category", "categ")?, Occur::SHOULD),
    ])?;
    field_matcher_checks(
        &searcher,
        &query,
        &highlighter_no_field_match,
        highlighter_field_match,
    )?;
    ir.close()
}

/// The anonymous `DelegatingAnalyzerWrapper` of `testMaskedFields` over its
/// `TreeMap` of field analyzers.
#[derive(Debug)]
struct MaskedFieldAnalyzers {
    field: Arc<dyn Analyzer>,
    field_english: Arc<dyn Analyzer>,
    field_characters: Arc<dyn Analyzer>,
    field_tripples: Arc<dyn Analyzer>,
}

impl DelegatingAnalyzerWrapperLogic for MaskedFieldAnalyzers {
    fn get_wrapped_analyzer(&self, field_name: Option<&str>) -> &dyn Analyzer {
        match field_name {
            Some("field") => &*self.field,
            Some("field_english") => &*self.field_english,
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

fn test_masked_fields(t: &mut T) -> Result<()> {
    let field_characters: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::with_automaton(
        &*random(),
        run_automaton(".")?,
        true,
    )); // Each letter is a token
    let field_tripples: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::with_automaton(
        &*random(),
        run_automaton("...")?,
        true,
    )); // Every three letters is a token
    let analyzer: Arc<dyn Analyzer> = Arc::new(DelegatingAnalyzerWrapper::new(
        Box::new(PerFieldReuseStrategy),
        Box::new(MaskedFieldAnalyzers {
            field: Arc::new(WhitespaceAnalyzer::new()),
            field_english: Arc::new(EnglishAnalyzer::new()), // English stemming and stopwords
            field_characters,
            field_tripples,
        }),
    ));
    let field_type_matched = FieldType::new_from(&*t.field_type);
    field_type_matched.set_stored(false)?; // matched fields don't need to be stored
    field_type_matched.freeze();
    let field_type_matched = Arc::new(field_type_matched);

    let dir = LuceneTestCase::new_directory();
    let result = (|| -> Result<()> {
        {
            let writer = IndexWriter::new(
                dir.as_directory(),
                LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::clone(&analyzer))),
            )?;
            let added = (|| -> Result<()> {
                let mut doc = Document::new();
                add(&mut doc, &t.field("field", "dance with star")?);
                add(&mut doc, &field("field_english", "dance with star", &field_type_matched)?);
                add(
                    &mut doc,
                    &field("field_characters", "dance with star", &field_type_matched)?,
                );
                add(
                    &mut doc,
                    &field("field_tripples", "dance with star", &field_type_matched)?,
                );
                writer.add_document(&doc)?;
                Ok(())
            })();
            let closed = writer.close();
            added?;
            closed?;
        }
        let reader = directory_reader::open(dir.as_directory())?;
        let checked = (|| -> Result<()> {
            let searcher = Arc::new(LuceneTestCase::new_searcher(Arc::clone(&reader)));
            // field is highlighted based on the matches from the "field_english"
            masked_fields_test_case(
                &analyzer,
                &searcher,
                "field",
                &["field_english"],
                "dancing with the stars",
                "<b>dance with star</b>",
                "<b>dance</b> with <b>star</b>",
            )?;

            // field is highlighted based on the matches from the "field_characters"
            masked_fields_test_case(
                &analyzer,
                &searcher,
                "field",
                &["field_characters"],
                "danc",
                "<b>danc</b>e with star",
                "<b>d</b><b>a</b><b>n</b><b>c</b>e with star",
            )?;

            // field is highlighted based on the matches from the "field_tripples"
            masked_fields_test_case(
                &analyzer,
                &searcher,
                "field",
                &["field_tripples"],
                "danc",
                "<b>dan</b>ce with star",
                "<b>dan</b>ce with star",
            )?;

            // field is highlighted based on the matches from the "field_characters" and
            // "field_tripples"
            masked_fields_test_case(
                &analyzer,
                &searcher,
                "field",
                &["field_tripples", "field_characters"],
                "danc",
                "<b>danc</b>e with star",
                "<b>dan</b><b>c</b>e with star",
            )
        })();
        let closed = reader.close();
        checked?;
        closed
    })();
    let closed = dir.close();
    result?;
    closed
}

/// The iteration order of `Set.of(e…)` (an `ImmutableCollections.SetN`,
/// probed from the per-JVM `SALT32L`) is unspecified: the boolean query
/// adds one SHOULD clause per masked field, which commute.
fn masked_fields_test_case(
    analyzer: &Arc<dyn Analyzer>,
    searcher: &Arc<IndexSearcher>,
    field: &'static str,
    masked_fields: &[&'static str],
    query_text: &str,
    expected_snippet_with_weight_matches: &str,
    expected_snippet_without_weight_matches: &str,
) -> Result<()> {
    let query_builder = QueryBuilder::new(Some(Arc::clone(analyzer)));
    let mut bool_query_builder = BooleanQueryBuilder::new();
    let field_phrase_query = query_builder
        .create_phrase_query_with_slop(Some(field), query_text, 2)?
        .expect("a phrase query");
    bool_query_builder.add(field_phrase_query, Occur::SHOULD)?;
    for masked_field in masked_fields {
        let field_phrase_query = query_builder
            .create_phrase_query_with_slop(Some(masked_field), query_text, 2)?
            .expect("a phrase query");
        bool_query_builder.add(field_phrase_query, Occur::SHOULD)?;
    }
    let query: Arc<dyn Query> = Arc::new(bool_query_builder.build());
    let top_docs = searcher.search(Arc::clone(&query), 10)?;
    Assert::assert_equals(1, top_docs.total_hits.value());

    let masked: Vec<String> = masked_fields.iter().map(|f| f.to_string()).collect();
    let masked_fields_func = Arc::new(move |field_name: &str| {
        Some(if field_name == field {
            masked.clone()
        } else {
            Vec::new()
        })
    });
    let uh_builder = builder(searcher, Arc::clone(analyzer))
        .with_masked_fields_func(Some(masked_fields_func));
    let highlighter = random_uh_from_builder_flags(
        uh_builder,
        &[HighlightFlag::PHRASES],
        Some(random().next_boolean()),
    );
    let snippets = highlighter.highlight_max_passages(field, &query, &top_docs, 10)?;
    let expected_snippet = if highlighter
        .get_flags(field)
        .contains(&HighlightFlag::WEIGHT_MATCHES)
    {
        expected_snippet_with_weight_matches
    } else {
        expected_snippet_without_weight_matches
    };
    Assert::assert_equals(1, snippets.len());
    assert_snippet(expected_snippet, &snippets[0]);
    Ok(())
}

fn test_matches_slop_bug(t: &mut T) -> Result<()> {
    let ir = index_some_fields(t)?;
    let searcher = new_searcher(&ir);
    let highlighter = UnifiedHighlighter::builder(Arc::clone(&searcher), t.analyzer()).build();
    let query = pq(2, "title", &["this", "is", "the", "field"])?;
    let top_docs = search_index_order(&searcher, &query, 10)?;
    Assert::assert_equals(1, top_docs.total_hits.value());
    let snippets = highlighter.highlight_max_passages("title", &query, &top_docs, 10)?;
    Assert::assert_equals(1, snippets.len());
    if highlighter
        .get_flags("title")
        .contains(&HighlightFlag::WEIGHT_MATCHES)
    {
        assert_snippet("<b>This is the title field</b>.", &snippets[0]);
    } else {
        assert_snippet(
            "<b>This</b> <b>is</b> <b>the</b> title <b>field</b>.",
            &snippets[0],
        );
    }
    ir.close()
}

/// `assertEquals(weightMatches ? with : without, snippet)`.
fn assert_by_flags(highlighter: &UnifiedHighlighter, field: &str, with: &str, without: &str, snippet: &Option<String>) {
    if highlighter
        .get_flags(field)
        .contains(&HighlightFlag::WEIGHT_MATCHES)
    {
        assert_snippet(with, snippet);
    } else {
        assert_snippet(without, snippet);
    }
}

fn test_field_matcher_phrase_query(t: &mut T) -> Result<()> {
    let ir = index_some_fields(t)?;
    let searcher = new_searcher(&ir);
    let highlighter_no_field_match = UnifiedHighlighter::builder(Arc::clone(&searcher), t.analyzer())
        // requireFieldMatch=false
        .with_field_matcher(always())
        .build();
    let uh_builder = builder(&searcher, t.analyzer());
    let mut highlighter_field_match = override_field_matcher_for_tests(
        &random_uh_from_builder_flags(
            uh_builder,
            &[HighlightFlag::PHRASES, HighlightFlag::MULTI_TERM_QUERY],
            None,
        ),
        None,
        "text",
    );
    let query = bq(vec![
        (pq(0, "title", &["this", "is", "the", "title"])?, Occur::SHOULD),
        (pq(2, "category", &["this", "is", "the", "field"])?, Occur::SHOULD),
        (pq(0, "text", &["this", "is"])?, Occur::SHOULD),
        (pq(0, "category", &["this", "is"])?, Occur::SHOULD),
        (pq(1, "text", &["you", "can", "put", "text"])?, Occur::SHOULD),
    ])?;

    // title
    {
        let top_docs = search_index_order(&searcher, &query, 10)?;
        Assert::assert_equals(1, top_docs.total_hits.value());
        let mut snippet = one_snippet(&highlighter_no_field_match, "title", &query, &top_docs)?;
        assert_by_flags(
            &highlighter_no_field_match,
            "title",
            "<b>This is the title field</b>.",
            "<b>This</b> <b>is</b> <b>the</b> <b>title</b> <b>field</b>.",
            &snippet,
        );

        snippet = one_snippet(&highlighter_field_match, "title", &query, &top_docs)?;
        assert_by_flags(
            &highlighter_field_match,
            "title",
            "<b>This is the title</b> field.",
            "<b>This</b> <b>is</b> <b>the</b> <b>title</b> field.",
            &snippet,
        );

        highlighter_field_match =
            override_field_matcher_for_tests(&highlighter_field_match, equals("text"), "text");
        snippet = one_snippet(&highlighter_field_match, "title", &query, &top_docs)?;
        assert_by_flags(
            &highlighter_field_match,
            "title",
            "<b>This is</b> the title field.",
            "<b>This</b> <b>is</b> the title field.",
            &snippet,
        );
        highlighter_field_match =
            override_field_matcher_for_tests(&highlighter_field_match, None, "text");
    }

    // text
    {
        let top_docs = search_index_order(&searcher, &query, 10)?;
        Assert::assert_equals(1, top_docs.total_hits.value());
        let mut snippet = one_snippet(&highlighter_no_field_match, "text", &query, &top_docs)?;
        assert_by_flags(
            &highlighter_no_field_match,
            "text",
            "<b>This is the text field</b>. <b>You can put some text</b> if you want.",
            "<b>This</b> <b>is</b> <b>the</b> <b>text</b> <b>field</b>. <b>You</b> <b>can</b> <b>put</b> some <b>text</b> if you want.",
            &snippet,
        );

        snippet = one_snippet(&highlighter_field_match, "text", &query, &top_docs)?;
        // note: odd that the first "text" is highlighted.  Apparently WSTE converts PhraseQuery to
        // a SpanNearQuery with
        //   with inorder=false when non-0 slop.  Probably a bug.
        assert_by_flags(
            &highlighter_field_match,
            "text",
            "<b>This is</b> the text field. <b>You can put some text</b> if you want.",
            "<b>This</b> <b>is</b> the <b>text</b> field. <b>You</b> <b>can</b> <b>put</b> some <b>text</b> if you want.",
            &snippet,
        );

        highlighter_field_match =
            override_field_matcher_for_tests(&highlighter_field_match, equals("title"), "title");
        snippet = one_snippet(&highlighter_field_match, "text", &query, &top_docs)?;
        assert_snippet(
            "This is the text field. You can put some text if you want.",
            &snippet,
        );
        highlighter_field_match =
            override_field_matcher_for_tests(&highlighter_field_match, None, "title");
    }

    // category
    {
        let top_docs = search_index_order(&searcher, &query, 10)?;
        Assert::assert_equals(1, top_docs.total_hits.value());
        let mut snippet = one_snippet(&highlighter_no_field_match, "category", &query, &top_docs)?;
        assert_by_flags(
            &highlighter_no_field_match,
            "category",
            "<b>This is the category field</b>.",
            "<b>This</b> <b>is</b> <b>the</b> category <b>field</b>.",
            &snippet,
        );

        snippet = one_snippet(&highlighter_field_match, "category", &query, &top_docs)?;
        assert_by_flags(
            &highlighter_field_match,
            "category",
            "<b>This is the category field</b>.",
            "<b>This</b> <b>is</b> <b>the</b> category <b>field</b>.",
            &snippet,
        );

        highlighter_field_match =
            override_field_matcher_for_tests(&highlighter_field_match, equals("text"), "text");
        snippet = one_snippet(&highlighter_field_match, "category", &query, &top_docs)?;
        assert_by_flags(
            &highlighter_field_match,
            "category",
            "<b>This is</b> the category field.",
            "<b>This</b> <b>is</b> the category field.",
            &snippet,
        );
        let _ = override_field_matcher_for_tests(&highlighter_field_match, None, "text");
    }
    ir.close()
}

// LUCENE-7909
fn test_nested_boolean_query_accuracy(t: &mut T) -> Result<()> {
    let ir = index_some_fields(t)?;
    let searcher = new_searcher(&ir);
    let highlighter = random_unified_highlighter_flags(
        &searcher,
        t.analyzer(),
        &[HighlightFlag::WEIGHT_MATCHES],
        Some(true),
    );

    // This query contains an inner Boolean of two MUST clauses (== "AND").  Only one of them is
    //  actually in the data, the other is not.  We should highlight neither.  We can highlight the
    // outer
    //  SHOULD clauses though.
    let query = bq(vec![
        (pq(0, "title", &["title", "field"])?, Occur::SHOULD),
        (
            bq(vec![
                (tq("category", "category"), Occur::MUST),
                (tq("category", "nonexistent"), Occur::MUST),
            ])?,
            Occur::SHOULD,
        ),
    ])?;

    let top_docs = search_index_order(&searcher, &query, 10)?;

    let mut snippets = highlighter.highlight("title", &query, &top_docs)?;
    assert_snippets(&["This is the <b>title field</b>."], &snippets);

    // no highlights, not of "category" since "nonexistent" wasn't there
    snippets = highlighter.highlight("category", &query, &top_docs)?;
    assert_snippets(&["This is the category field."], &snippets);

    ir.close()
}

fn test_not_reanalyzed(t: &mut T) -> Result<()> {
    if t.is_reanalysis() {
        return Ok(()); // we're testing the *other* cases
    }

    let ir = index_bodies(
        t,
        &["This is a test. Just a test highlighting from postings. Feel free to ignore."],
    )?;

    let searcher = new_searcher(&ir);
    let highlighter = random_unified_highlighter(
        &searcher,
        FnAnalyzer::new(
            "org.apache.lucene.search.uhighlight.TestUnifiedHighlighter$5",
            |_field_name| Err(assertion_error("shouldn't be called")),
        )
        .arc(),
    );
    let query = tq("body", "highlighting");
    let top_docs = search_index_order(&searcher, &query, 10)?;
    Assert::assert_equals(1, top_docs.total_hits.value());
    let snippets = highlighter.highlight("body", &query, &top_docs)?;
    Assert::assert_equals(1, snippets.len());
    assert_snippet("Just a test <b>highlighting</b> from postings. ", &snippets[0]);

    ir.close()
}

/// The anonymous leaf `Query` of `testUnknownQueryWithWeightMatches`: it
/// visits no term and exposes no automaton, so it appears as some unknown
/// leaf, while its weight matches `body:sentence`.
#[derive(Debug)]
struct BogusQuery {
    body_name: String,
}

impl Query for BogusQuery {
    fn to_query_string(&self, _field: &str) -> String {
        "bogus".to_string()
    }

    fn rewrite(&self, _index_searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        Ok(None)
    }

    // we don't visit terms, and we don't expose an automata.  Thus this appears as
    // some unknown leaf.
    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        if visitor.accept_field(&self.body_name) {
            visitor.visit_leaf(self);
        }
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        std::ptr::eq(
            self as *const Self as *const u8,
            other.as_any() as *const dyn Any as *const u8,
        )
    }

    fn query_hash(&self) -> i32 {
        // System.identityHashCode(this)
        self as *const Self as usize as i32
    }

    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        // TODO maybe should loop through index terms to show we can see other terms
        TermQuery::new(term(&self.body_name, "sentence")).create_weight(searcher, score_mode, boost)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.search.uhighlight.TestUnifiedHighlighter$6"
    }
}

fn test_unknown_query_with_weight_matches(t: &mut T) -> Result<()> {
    let ir = index_bodies(t, &["Test a one sentence document."])?;

    let searcher = new_searcher(&ir);
    let highlighter = random_unified_highlighter_flags(
        &searcher,
        t.analyzer(),
        &[HighlightFlag::WEIGHT_MATCHES],
        None,
    );
    let query = bq(vec![
        // simple term query body:one
        (tq("body", "one"), Occur::MUST),
        // a custom query, a leaf, that which matches body:sentence
        //    Note this isn't even an MTQ.  What matters is that Weight.matches works.
        (
            Arc::new(BogusQuery {
                body_name: "body".to_string(),
            }),
            Occur::MUST,
        ),
    ])?;
    let top_docs = search_index_order(&searcher, &query, 10)?;
    Assert::assert_equals(1, top_docs.total_hits.value());
    let snippets = highlighter.highlight("body", &query, &top_docs)?;
    Assert::assert_equals(1, snippets.len());
    assert_snippet("Test a <b>one</b> <b>sentence</b> document.", &snippets[0]);

    ir.close()
}

fn test_query_with_long_term(t: &mut T) -> Result<()> {
    let ir = index_some_fields(t)?;
    let searcher = new_searcher(&ir);
    let highlighter = random_unified_highlighter_flags(
        &searcher,
        t.analyzer(),
        &[HighlightFlag::WEIGHT_MATCHES],
        Some(true),
    );

    let query = bq(vec![
        (
            tq("title", &"a".repeat(MAX_STRING_UNION_TERM_LENGTH)),
            Occur::SHOULD,
        ),
        (
            tq("title", &"a".repeat(MAX_STRING_UNION_TERM_LENGTH + 1)),
            Occur::SHOULD,
        ),
        (tq("title", "title"), Occur::SHOULD),
    ])?;

    let top_docs = search_index_order(&searcher, &query, 10)?;

    let snippets = highlighter.highlight("title", &query, &top_docs)?;
    assert_snippets(&["This is the <b>title</b> field."], &snippets);
    ir.close()
}
