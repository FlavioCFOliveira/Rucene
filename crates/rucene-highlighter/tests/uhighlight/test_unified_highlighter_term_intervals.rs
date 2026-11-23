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

//! Port of `TestUnifiedHighlighterTermIntervals.java`.

use std::fmt;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::java_regex::JavaPattern;
use rucene::analysis::Analyzer;
use rucene::document::{Document, FieldType, Store};
use rucene::error::Result;
use rucene::index::{DirectoryReader, IndexOptions};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::query::Query;
use rucene_analysis_common::jdk::break_iterator::BreakIterator;
use rucene_highlighter::uhighlight::default_passage_formatter::DefaultPassageFormatter;
use rucene_highlighter::uhighlight::passage::Passage;
use rucene_highlighter::uhighlight::passage_formatter::{FormattedPassages, PassageFormatter};
use rucene_highlighter::uhighlight::unified_highlighter::{
    Builder, HighlightFlag, SortedDocIdIterator, UnifiedHighlighter, UnifiedHighlighterOverrides,
};
use rucene_highlighter::uhighlight::whole_break_iterator::WholeBreakIterator;
use rucene_queries::intervals::interval_query::IntervalQuery;
use rucene_queries::intervals::intervals::Intervals;
use rucene_queries::intervals::intervals_source::IntervalsSource;
use rucene_test_framework::util::LuceneTestCase;

use crate::highlighter_support::core_support::random;
use crate::highlighter_support::{ti, FnAnalyzer};
use crate::uhighlight::test_unified_highlighter::{builder, new_searcher};
use crate::uhighlight::unified_highlighter_test_base::{
    add, assert_null_snippet, assert_snippet, assertion_error, get, parameterized_constructor,
    parameters_method, random_uh_from_builder_flags, random_unified_highlighter_flags, s,
    search_index_order, set, tq, uh_class, units, UHBase, UHTest,
};

type T = UHTest;

/// `TestUnifiedHighlighterTermIntervals`.
pub fn class_def() -> Arc<Class> {
    uh_class::<T>(
        "org.apache.lucene.search.uhighlight.TestUnifiedHighlighterTermIntervals",
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
            ti::<T>(
                "testMaxLengthWithMultivalue",
                test_max_length_with_multivalue,
            ),
            ti::<T>("testMultipleTerms", test_multiple_terms),
            ti::<T>("testMultiplePassages", test_multiple_passages),
            ti::<T>("testBuddhism", test_buddhism),
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
            ti::<T>("testEncode", test_encode),
            ti::<T>("testObjectFormatter", test_object_formatter),
            ti::<T>("testMatchesSlopBug", test_matches_slop_bug),
            ti::<T>("testNotReanalyzed", test_not_reanalyzed),
        ],
    )
}

/// `randomUnifiedHighlighter(searcher, indexAnalyzer)` of this class: no
/// mandatory flag.
fn random_uh(
    searcher: &Arc<IndexSearcher>,
    index_analyzer: Arc<dyn Analyzer>,
) -> UnifiedHighlighter {
    random_unified_highlighter_flags(searcher, index_analyzer, &[], None)
}

/// `randomUnifiedHighlighter(uhBuilder)` of this class.
fn random_uh_builder(uh_builder: Builder) -> UnifiedHighlighter {
    random_uh_from_builder_flags(uh_builder, &[], None)
}

/// `new IntervalQuery(field, source)`.
fn iq(field: &str, source: Arc<dyn IntervalsSource>) -> Arc<dyn Query> {
    Arc::new(IntervalQuery::new(field, source))
}

/// `Intervals.term(text)`.
fn it(text: &str) -> Arc<dyn IntervalsSource> {
    Intervals::term_str(text)
}

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
    let highlighter = random_uh(&searcher, t.analyzer());
    let query = iq("body", it("highlighting"));
    let top_docs = search_index_order(&searcher, &query, 10)?;
    Assert::assert_equals(2, top_docs.total_hits.value());
    let snippets = highlighter.highlight("body", &query, &top_docs)?;
    Assert::assert_equals(2, snippets.len());
    assert_snippet(
        "Just a test <b>highlighting</b> from postings. ",
        &snippets[0],
    );
    assert_snippet("<b>Highlighting</b> the first term. ", &snippets[1]);
    ir.close()
}

fn test_format_with_match_exceeding_content_length2(t: &mut T) -> Result<()> {
    let snippets = format_with_match_exceeding_content_length(t, "123 TEST 01234 TEST")?;
    Assert::assert_equals(1, snippets.len());
    assert_snippet("123 <b>TEST</b> 01234 TE", &snippets[0]);
    Ok(())
}

fn test_format_with_match_exceeding_content_length3(t: &mut T) -> Result<()> {
    let snippets = format_with_match_exceeding_content_length(t, "123 5678 01234 TEST TEST")?;
    Assert::assert_equals(1, snippets.len());
    assert_snippet("123 5678 01234 TE", &snippets[0]);
    Ok(())
}

fn test_format_with_match_exceeding_content_length(t: &mut T) -> Result<()> {
    let snippets = format_with_match_exceeding_content_length(t, "123 5678 01234 TEST")?;
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

    let query = iq("body", it("test"));

    let top_docs = search_index_order(&searcher, &query, 10)?;
    Assert::assert_equals(1, top_docs.total_hits.value());

    let uh_builder = builder(&searcher, t.analyzer()).with_max_length(max_length)?;
    let highlighter = random_uh_builder(uh_builder);
    let snippets = highlighter.highlight("body", &query, &top_docs)?;

    ir.close()?;
    Ok(snippets)
}

// simple test highlighting last word.
fn test_highlight_last_word(t: &mut T) -> Result<()> {
    let ir = index_bodies(t, &["This is a test"])?;

    let searcher = new_searcher(&ir);
    let highlighter = random_uh(&searcher, t.analyzer());
    let query = iq("body", it("test"));
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
    let highlighter = random_uh(&searcher, t.analyzer());
    let query = iq("body", it("test"));
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
    let highlighter = random_uh_builder(uh_builder);
    let query = iq("body", it("field"));
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

fn test_multiple_terms(t: &mut T) -> Result<()> {
    let ir = index_bodies(
        t,
        &[
            "This is a test. Just a test highlighting from postings. Feel free to ignore.",
            "Highlighting the first term. Hope it works.",
        ],
    )?;

    let searcher = new_searcher(&ir);
    let highlighter = random_uh(&searcher, t.analyzer());
    let query = iq(
        "body",
        Intervals::or(vec![it("highlighting"), it("just"), it("first")])?,
    );
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
    let highlighter = random_uh(&searcher, t.analyzer());
    let query = iq("body", it("test"));
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
    let query = iq("body", Intervals::phrase_terms(&["buddhist", "origins"])?);
    let top_docs = searcher.search(Arc::clone(&query), 10)?;
    Assert::assert_equals(1, top_docs.total_hits.value());
    let uh_builder = builder(&searcher, t.analyzer()).with_highlight_phrases_strictly(false);
    let highlighter = random_uh_builder(uh_builder);
    let snippets = highlighter.highlight_max_passages("body", &query, &top_docs, 2)?;
    Assert::assert_equals(1, snippets.len());
    //  highlighter.getFlags("body").containsAll(EnumSet.of(HighlightFlag.WEIGHT_MATCHES,
    // HighlightFlag.PHRASES))) {
    // assertTrue(snippets[0] + " " + query, snippets[0].contains("<b>Buddhist origins</b>"));
    Assert::assert_true_msg(
        s(&snippets[0]),
        s(&snippets[0]).contains("<b>Buddhist</b> <b>origins</b>"),
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
    let query = iq("body", Intervals::phrase_terms(&["curious", "george"])?);
    let top_docs = searcher.search(Arc::clone(&query), 10)?;
    Assert::assert_equals(1, top_docs.total_hits.value());
    let uh_builder = builder(&searcher, t.analyzer()).with_highlight_phrases_strictly(false);
    let highlighter = random_uh_builder(uh_builder);
    let snippets = highlighter.highlight_max_passages("body", &query, &top_docs, 2)?;
    Assert::assert_equals(1, snippets.len());
    Assert::assert_false(s(&snippets[0]).contains("<b>Curious</b>Curious"));
    let mut matches = 0;
    let snippet = "<b>Curious((</b> <b>)| )?George</b>";
    let pattern = JavaPattern::compile(snippet, 0)
        .unwrap_or_else(|e| panic!("java.util.regex.PatternSyntaxException: {e:?}"));
    let input = units(s(&snippets[0]));
    let mut m = pattern.matcher(&input);
    while m.find() {
        matches += 1;
    }
    Assert::assert_equals_msg(
        Some(&format!(
            "{} is looking for {snippet} at {}",
            query.to_query_string(""),
            s(&snippets[0])
        )),
        2,
        matches,
    );
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
    let iw = t.writer()?;
    let mut document = Document::new();
    add(&mut document, &t.field("body", text)?);
    iw.add_document(&document)?;
    let ir = iw.get_reader()?;
    let result = (|| -> Result<()> {
        iw.close()?;
        let searcher = new_searcher(&ir);
        let query = iq(
            "body",
            Intervals::unordered(vec![it("porter"), it("square"), it("massachusetts")]),
        );
        let top_docs = searcher.search(Arc::clone(&query), 10)?;
        Assert::assert_equals(1, top_docs.total_hits.value());
        let uh_builder = builder(&searcher, t.analyzer()).with_max_length(i32::MAX - 1)?;
        let highlighter = random_uh_builder(uh_builder);
        let snippets = highlighter.highlight_max_passages("body", &query, &top_docs, 2)?;
        Assert::assert_equals(1, snippets.len());
        Assert::assert_true(s(&snippets[0]).contains("<b>Square</b>"));
        Assert::assert_true(s(&snippets[0]).contains("<b>Porter</b>"));
        Ok(())
    })();
    let closed = ir.close();
    result?;
    closed
}

fn test_passage_ranking(t: &mut T) -> Result<()> {
    let ir = index_bodies(t, &["This is a test.  Just highlighting from postings. This is also a much sillier test.  Feel free to test test test test test test test."])?;

    let searcher = new_searcher(&ir);
    let highlighter = random_uh(&searcher, t.analyzer());
    let query = iq("body", it("test"));
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

    let query = iq("body", Intervals::not_containing(it("terms"), it("both"))?);
    let top_docs = searcher.search(Arc::clone(&query), 10)?;
    Assert::assert_equals(1, top_docs.total_hits.value());
    let uh_builder = builder(&searcher, t.analyzer()).with_max_length(i32::MAX - 1)?;
    let highlighter = random_uh_builder(uh_builder);
    let snippets = highlighter.highlight_max_passages("body", &query, &top_docs, 2)?;
    Assert::assert_equals(1, snippets.len());
    Assert::assert_false(s(&snippets[0]).contains("<b>both</b>"));
    Assert::assert_true(s(&snippets[0]).contains("<b>terms</b>"));
    ir.close()
}

const SILLIER: &str = "This is a test.  Just highlighting from postings. This is also a much sillier test.  Feel free to test test test test test test test.";
const SILLIER_ALL: &str = "This is a <b>test</b>.  Just highlighting from postings. This is also a much sillier <b>test</b>.  Feel free to <b>test</b> <b>test</b> <b>test</b> <b>test</b> <b>test</b> <b>test</b> <b>test</b>.";

fn test_highlight_all_text(t: &mut T) -> Result<()> {
    let ir = index_bodies(t, &[SILLIER])?;

    let searcher = new_searcher(&ir);
    let uh_builder = builder(&searcher, t.analyzer())
        .with_max_length(1000)?
        .with_break_iterator(whole_break_iterator());
    let highlighter = random_uh_builder(uh_builder);
    let query = iq("body", it("test"));
    let top_docs = search_index_order(&searcher, &query, 10)?;
    Assert::assert_equals(1, top_docs.total_hits.value());
    let snippets = highlighter.highlight_max_passages("body", &query, &top_docs, 2)?;
    Assert::assert_equals(1, snippets.len());
    assert_snippet(SILLIER_ALL, &snippets[0]);
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
    let highlighter = random_uh(&searcher, t.analyzer());
    let query = iq("body", it("highlighting"));
    let top_docs = search_index_order(&searcher, &query, 10)?;
    Assert::assert_equals(2, top_docs.total_hits.value());
    let hits = &top_docs.score_docs;
    let doc_ids = [hits[0].doc, hits[1].doc];
    let snippets =
        highlighter.highlight_fields_doc_ids(&["body".to_string()], &query, &doc_ids, &[1])?;
    let snippets = get(&snippets, "body");
    Assert::assert_equals(2, snippets.len());
    assert_snippet(
        "Just a test <b>highlighting</b> from postings. ",
        &snippets[0],
    );
    assert_snippet("<b>Highlighting</b> the first term. ", &snippets[1]);
    ir.close()
}

/// The overrides of the anonymous highlighter of
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

    fn get_break_iterator(
        &self,
        _this: &UnifiedHighlighter,
        _field: &str,
    ) -> Box<dyn BreakIterator> {
        Box::new(WholeBreakIterator::new())
    }
}

fn test_custom_field_value_source(t: &mut T) -> Result<()> {
    let iw = t.writer()?;
    let mut doc = Document::new();
    add(&mut doc, &t.field("body", SILLIER)?);
    iw.add_document(&doc)?;
    let ir = iw.get_reader()?;
    iw.close()?;

    let searcher = new_searcher(&ir);
    let uh_builder = builder(&searcher, t.analyzer());
    let mut highlighter = UnifiedHighlighter::from_builder(uh_builder);
    highlighter.set_overrides(Arc::new(CustomFieldValueSource {
        text: SILLIER.to_string(),
    }));

    let query = iq("body", it("test"));
    let top_docs = search_index_order(&searcher, &query, 10)?;
    Assert::assert_equals(1, top_docs.total_hits.value());
    let snippets = highlighter.highlight_max_passages("body", &query, &top_docs, 2)?;
    Assert::assert_equals(1, snippets.len());
    assert_snippet(SILLIER_ALL, &snippets[0]);
    ir.close()
}

const PLANET: &str = "test this is.  another sentence this test has.  far away is that planet.";

/// One document whose `body` is `PLANET`.
fn index_planet(t: &UHBase) -> Result<Arc<dyn DirectoryReader>> {
    let iw = t.writer()?;
    let mut doc = Document::new();
    add(&mut doc, &t.field("body", PLANET)?);
    iw.add_document(&doc)?;
    let ir = iw.get_reader()?;
    iw.close()?;
    Ok(ir)
}

/// Make sure highlighter returns first N sentences if there were no hits.
fn test_empty_highlights(t: &mut T) -> Result<()> {
    let ir = index_planet(t)?;

    let searcher = new_searcher(&ir);
    let highlighter = random_uh(&searcher, t.analyzer());
    let query = iq("body", it("highlighting"));
    let snippets =
        highlighter.highlight_fields_doc_ids(&["body".to_string()], &query, &[0], &[2])?;
    let snippets = get(&snippets, "body");
    Assert::assert_equals(1, snippets.len());
    assert_snippet(
        "test this is.  another sentence this test has.  ",
        &snippets[0],
    );
    ir.close()
}

/// The two documents of the "id" tests: `body` of the first is
/// `first_body`.
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

fn highlight_id_doc(t: &UHBase, ir: &Arc<dyn DirectoryReader>) -> Result<Vec<Option<String>>> {
    let searcher = new_searcher(ir);
    let highlighter = random_uh(&searcher, t.analyzer());
    let doc_id = searcher.search(tq("id", "id"), 1)?.score_docs[0].doc;

    let query = iq("body", it("highlighting"));
    let doc_ids = [doc_id];
    let mut snippets =
        highlighter.highlight_fields_doc_ids(&["body".to_string()], &query, &doc_ids, &[2])?;
    Ok(snippets
        .remove("body")
        .expect("NullPointerException: no body snippets"))
}

/// Not empty but nothing analyzes. Ensures we address null term-vectors.
fn test_nothing_analyzes(t: &mut T) -> Result<()> {
    let ir = index_id_and_something(t, " ")?; // just a space! (thus not empty)
    let snippets = highlight_id_doc(t, &ir)?;
    Assert::assert_equals(1, snippets.len());
    assert_snippet(" ", &snippets[0]);
    ir.close()
}

/// Make sure highlighter we can customize how emtpy highlight is returned.
fn test_custom_empty_highlights(t: &mut T) -> Result<()> {
    t.mock().set_position_increment_gap(10);
    let ir = index_planet(t)?;

    let searcher = new_searcher(&ir);
    let uh_builder = builder(&searcher, t.analyzer()).with_max_no_highlight_passages(0); // don't want any default summary
    let highlighter = random_uh_builder(uh_builder);
    let query = iq("body", it("highlighting"));
    let snippets =
        highlighter.highlight_fields_doc_ids(&["body".to_string()], &query, &[0], &[2])?;
    let snippets = get(&snippets, "body");
    Assert::assert_equals(1, snippets.len());
    assert_null_snippet(&snippets[0]);
    ir.close()
}

/// Make sure highlighter returns whole text when there are no hits and
/// BreakIterator is null.
fn test_empty_highlights_whole(t: &mut T) -> Result<()> {
    let ir = index_planet(t)?;

    let searcher = new_searcher(&ir);
    let highlighter = UnifiedHighlighter::builder(Arc::clone(&searcher), t.analyzer())
        .with_break_iterator(whole_break_iterator())
        .build();
    let query = iq("body", it("highlighting"));
    let snippets =
        highlighter.highlight_fields_doc_ids(&["body".to_string()], &query, &[0], &[2])?;
    let snippets = get(&snippets, "body");
    Assert::assert_equals(1, snippets.len());
    assert_snippet(PLANET, &snippets[0]);
    ir.close()
}

/// Make sure highlighter is OK with entirely missing field.
fn test_field_is_missing(t: &mut T) -> Result<()> {
    let ir = index_planet(t)?;

    let searcher = new_searcher(&ir);
    let highlighter = random_uh(&searcher, t.analyzer());
    let query = iq("bogus", it("highlighting"));
    let snippets =
        highlighter.highlight_fields_doc_ids(&["bogus".to_string()], &query, &[0], &[2])?;
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
        doc.add(LuceneTestCase::new_string_field(
            "id",
            &i.to_string(),
            Store::YES,
        ));
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
    let highlighter = random_uh_builder(uh_builder);
    let query = iq("body", it("answer"));
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
    let query = iq("body", it("highlighting"));
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
        write!(
            f,
            "[Ljava.lang.String;@{:x}",
            self as *const Self as usize as u32
        )
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
            self.default_formatter
                .format(passages, content)?
                .to_string(),
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
    let highlighter = UnifiedHighlighter::builder(Arc::clone(&searcher), t.analyzer())
        .with_formatter(Arc::new(ArrayPassageFormatter {
            default_formatter: DefaultPassageFormatter::new(),
        }))
        .build();
    let query = iq("body", it("highlighting"));
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

/// `"" + highlighter.getFlags(field)`: an `EnumSet`'s `toString()`, in
/// ordinal order.
fn flags_string(highlighter: &UnifiedHighlighter, field: &str) -> String {
    let mut flags: Vec<HighlightFlag> = highlighter.get_flags(field).into_iter().collect();
    flags.sort();
    format!(
        "[{}]",
        flags
            .iter()
            .map(|f| format!("{f:?}"))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn test_matches_slop_bug(t: &mut T) -> Result<()> {
    let ir = index_some_fields(t)?;
    let searcher = new_searcher(&ir);
    let highlighter = UnifiedHighlighter::builder(Arc::clone(&searcher), t.analyzer()).build();
    let query = iq(
        "title",
        Intervals::maxgaps(
            if random().next_boolean() { 1 } else { 2 },
            Intervals::ordered(vec![it("this"), it("is"), it("the"), it("field")]),
        )?,
    );
    let top_docs = search_index_order(&searcher, &query, 10)?;
    Assert::assert_equals(1, top_docs.total_hits.value());
    let snippets = highlighter.highlight_max_passages("title", &query, &top_docs, 10)?;
    Assert::assert_equals(1, snippets.len());
    // All flags are enabled.
    Assert::assert_equals_msg(
        Some(&flags_string(&highlighter, "title")),
        4,
        highlighter.get_flags("title").len(),
    );
    Assert::assert_equals_str(
        Some(&flags_string(&highlighter, "title")),
        Some("<b>This</b> <b>is</b> <b>the</b> title <b>field</b>."),
        snippets[0].as_deref(),
    );
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
    let highlighter = random_uh(
        &searcher,
        FnAnalyzer::new(
            "org.apache.lucene.search.uhighlight.TestUnifiedHighlighterTermIntervals$3",
            |_field_name| Err(assertion_error("shouldn't be called")),
        )
        .arc(),
    );
    let query = iq("body", it("highlighting"));
    let top_docs = search_index_order(&searcher, &query, 10)?;
    Assert::assert_equals(1, top_docs.total_hits.value());
    let snippets = highlighter.highlight("body", &query, &top_docs)?;
    Assert::assert_equals(1, snippets.len());
    assert_snippet(
        "Just a test <b>highlighting</b> from postings. ",
        &snippets[0],
    );

    ir.close()
}
