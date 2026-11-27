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

//! Port of `TestUnifiedHighlighterMTQ.java`: some tests that highlight
//! wildcard, fuzzy, etc queries.

use std::any::Any;
use std::sync::Arc;

use randomizedtesting::generators::RandomStrings;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::{
    default_token_attribute_factory, Analyzer, AnalyzerState, CharReader, GlobalReuseStrategy,
    StandardAnalyzer, TokenStreamComponents, Tokenizer, TokenizerLogic,
};
use rucene::document::{Document, Store};
use rucene::error::{LuceneError, Result};
use rucene::index::DirectoryReader;
use rucene::search::boolean_clause::Occur;
use rucene::search::boost_query::BoostQuery;
use rucene::search::constant_score_query::ConstantScoreQuery;
use rucene::search::disjunction_max_query::DisjunctionMaxQuery;
use rucene::search::fuzzy_query::FuzzyQuery;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::match_all_docs_query::MatchAllDocsQuery;
use rucene::search::prefix_query::PrefixQuery;
use rucene::search::query::Query;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::regexp_query::RegexpQuery;
use rucene::search::score_mode::ScoreMode;
use rucene::search::term_range_query::TermRangeQuery;
use rucene::search::weight::Weight;
use rucene::search::wildcard_query::{WildcardQuery, WILDCARD_CHAR, WILDCARD_ESCAPE};
use rucene::search::TopDocs;
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::attribute::AttributeSource;
use rucene::util::BytesRef;
use rucene_analysis_common::analysis::core::keyword_analyzer::KeywordAnalyzer;
use rucene_highlighter::uhighlight::passage::Passage;
use rucene_highlighter::uhighlight::passage_formatter::{FormattedPassages, PassageFormatter};
use rucene_highlighter::uhighlight::unified_highlighter::{
    Builder, HighlightFlag, UnifiedHighlighter, UnifiedHighlighterOverrides, MULTIVAL_SEP_CHAR,
};
use rucene_queries::spans::span_first_query::SpanFirstQuery;
use rucene_queries::spans::span_multi_term_query_wrapper::SpanMultiTermQueryWrapper;
use rucene_queries::spans::span_near_query::SpanNearQuery;
use rucene_queries::spans::span_not_query::SpanNotQuery;
use rucene_queries::spans::span_or_query::SpanOrQuery;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_queries::spans::span_term_query::SpanTermQuery;
use rucene_queries::spans::span_weight::SpanWeight;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::LuceneTestCase;

use crate::highlighter_support::core_support::random;
use crate::highlighter_support::search_support::term;
use crate::highlighter_support::ti;
use crate::uhighlight::test_unified_highlighter::{bq, builder, new_searcher, pq_builder};
use crate::uhighlight::unified_highlighter_test_base::{
    add, assert_snippet, assert_snippets, get, parameterized_constructor, parameters_method,
    random_uh_mut, random_unified_highlighter_flags, search_index_order, set, tq, uh_class, UHBase,
    UHTest,
};

type T = UHTest;

/// `TestUnifiedHighlighterMTQ`.
pub fn class_def() -> Arc<Class> {
    uh_class::<T>(
        "org.apache.lucene.search.uhighlight.TestUnifiedHighlighterMTQ",
        parameterized_constructor(|ft| UHTest { b: UHBase::new(ft) }),
        vec![
            parameters_method(),
            ti::<T>("testWildcards", test_wildcards),
            ti::<T>("testOnePrefix", test_one_prefix),
            ti::<T>("testOneRegexp", test_one_regexp),
            ti::<T>("testFuzzy", test_fuzzy),
            ti::<T>("testRanges", test_ranges),
            ti::<T>("testWildcardInBoolean", test_wildcard_in_boolean),
            ti::<T>("testWildcardInFiltered", test_wildcard_in_filtered),
            ti::<T>(
                "testWildcardInConstantScore",
                test_wildcard_in_constant_score,
            ),
            ti::<T>(
                "testWildcardInDisjunctionMax",
                test_wildcard_in_disjunction_max,
            ),
            ti::<T>("testSpanWildcard", test_span_wildcard),
            ti::<T>("testSpanOr", test_span_or),
            ti::<T>("testSpanNear", test_span_near),
            ti::<T>("testSpanNot", test_span_not),
            ti::<T>("testSpanPositionCheck", test_span_position_check),
            ti::<T>("testWhichMTQMatched", test_which_mtq_matched),
            ti::<T>("testWithMaxLen", test_with_max_len),
            ti::<T>(
                "testWithMaxLenAndMultipleWildcardMatches",
                test_with_max_len_and_multiple_wildcard_matches,
            ),
            ti::<T>("testTokenStreamIsClosed", test_token_stream_is_closed),
            ti::<T>("testNothingAnalyzes", test_nothing_analyzes),
            ti::<T>("testMultiSegment", test_multi_segment),
            ti::<T>(
                "testPositionSensitiveWithWildcardDoesNotHighlight",
                test_position_sensitive_with_wildcard_does_not_highlight,
            ),
            ti::<T>(
                "testCustomSpanQueryHighlighting",
                test_custom_span_query_highlighting,
            ),
            ti::<T>("testRussianPrefixQuery", test_russian_prefix_query),
            ti::<T>("testMultiByteMTQ", test_multi_byte_mtq),
        ],
    )
}

/// `randomUnifiedHighlighter(searcher, indexAnalyzer)` of this class: with
/// `MULTI_TERM_QUERY` mandatory.
fn random_uh(
    searcher: &Arc<IndexSearcher>,
    index_analyzer: Arc<dyn Analyzer>,
) -> UnifiedHighlighter {
    random_unified_highlighter_flags(
        searcher,
        index_analyzer,
        &[HighlightFlag::MULTI_TERM_QUERY],
        None,
    )
}

/// `randomUnifiedHighlighter(uhBuilder)` of this class.
fn random_uh_builder(uh_builder: &mut Builder) -> UnifiedHighlighter {
    random_uh_mut(uh_builder, &[HighlightFlag::MULTI_TERM_QUERY], None)
}

fn wildcard(field: &str, text: &str) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(WildcardQuery::new(term(field, text))?))
}

fn wildcard_mtq(field: &str, text: &str) -> Result<Arc<WildcardQuery>> {
    Ok(Arc::new(WildcardQuery::new(term(field, text))?))
}

fn prefix(field: &str, text: &str) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(PrefixQuery::new(term(field, text))?))
}

fn match_all() -> Arc<dyn Query> {
    Arc::new(MatchAllDocsQuery::instance())
}

fn string_range(
    field: &str,
    lower: Option<&str>,
    upper: Option<&str>,
    include_lower: bool,
    include_upper: bool,
) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(TermRangeQuery::new_string_range(
        field,
        lower,
        upper,
        include_lower,
        include_upper,
    )?))
}

/// `new SpanMultiTermQueryWrapper<>(new WildcardQuery(new Term(field, text)))`.
fn span_wildcard(field: &str, text: &str) -> Result<Arc<SpanMultiTermQueryWrapper>> {
    let mtq: Arc<dyn rucene::search::multi_term_query::MultiTermQuery> = wildcard_mtq(field, text)?;
    Ok(Arc::new(SpanMultiTermQueryWrapper::new(mtq)))
}

const DOC1: &str = "This is a test.";
const DOC2: &str = "Test a one sentence document.";

/// The two documents most tests index through one reused `body` field.
fn index_two_docs(t: &UHBase) -> Result<Arc<dyn DirectoryReader>> {
    index_values(t, &[DOC1, DOC2])
}

fn index_values(t: &UHBase, values: &[&str]) -> Result<Arc<dyn DirectoryReader>> {
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

/// The two snippets of the two documents.
fn assert_two(snippets: &[Option<String>], first: &str, second: &str) {
    Assert::assert_equals(2, snippets.len());
    assert_snippet(first, &snippets[0]);
    assert_snippet(second, &snippets[1]);
}

const HL1: &str = "This is a <b>test</b>.";
const HL2: &str = "<b>Test</b> a one sentence document.";

/// `searcher.search(query, 10, Sort.INDEXORDER)`, asserting `hits` total
/// hits.
fn search_io(searcher: &IndexSearcher, query: &Arc<dyn Query>, hits: i64) -> Result<TopDocs> {
    let top_docs = search_index_order(searcher, query, 10)?;
    Assert::assert_equals(hits, top_docs.total_hits.value());
    Ok(top_docs)
}

fn test_wildcards(t: &mut T) -> Result<()> {
    let ir = index_two_docs(t)?;

    let searcher = new_searcher(&ir);
    let mut uh_builder = builder(&searcher, t.analyzer());
    let query = wildcard("body", "te*")?;
    let mut top_docs = search_io(&searcher, &query, 2)?;
    let mut snippets = uh_builder
        .clone()
        .build()
        .highlight("body", &query, &top_docs)?;
    assert_two(&snippets, HL1, HL2);

    // disable MTQ; won't highlight
    uh_builder = uh_builder.with_handle_multi_term_query(false);
    snippets = uh_builder
        .clone()
        .build()
        .highlight("body", &query, &top_docs)?;
    assert_two(&snippets, DOC1, DOC2);

    // wrong field
    let bq = bq(vec![
        (match_all(), Occur::SHOULD),
        (wildcard("bogus", "te*")?, Occur::SHOULD),
    ])?;
    top_docs = search_io(&searcher, &bq, 2)?;
    uh_builder = uh_builder.with_handle_multi_term_query(true);
    snippets = uh_builder
        .clone()
        .build()
        .highlight("body", &bq, &top_docs)?;
    assert_two(&snippets, DOC1, DOC2);
    ir.close()
}

/// The common body of the single-MTQ tests: `query` highlights both
/// documents, and `bogus` (on the wrong field, next to a
/// `MatchAllDocsQuery`) highlights none with `withFieldMatcher(null)`.
fn single_mtq(t: &UHBase, query: Arc<dyn Query>, bogus: Arc<dyn Query>) -> Result<()> {
    let ir = index_two_docs(t)?;

    let searcher = new_searcher(&ir);
    let mut uh_builder = builder(&searcher, t.analyzer());
    let highlighter = random_uh_builder(&mut uh_builder);
    let mut top_docs = search_io(&searcher, &query, 2)?;
    let mut snippets = highlighter.highlight("body", &query, &top_docs)?;
    assert_two(&snippets, HL1, HL2);

    // wrong field
    let bq = bq(vec![(match_all(), Occur::SHOULD), (bogus, Occur::SHOULD)])?;
    top_docs = search_io(&searcher, &bq, 2)?;
    uh_builder = uh_builder.with_field_matcher(None);
    snippets = uh_builder
        .clone()
        .build()
        .highlight("body", &bq, &top_docs)?;
    assert_two(&snippets, DOC1, DOC2);
    ir.close()
}

fn test_one_prefix(t: &mut T) -> Result<()> {
    // wrap in a BoostQuery to also show we see inside it
    let query: Arc<dyn Query> = Arc::new(BoostQuery::new(prefix("body", "te")?, 2.0)?);
    single_mtq(t, query, prefix("bogus", "te")?)
}

fn test_one_regexp(t: &mut T) -> Result<()> {
    let query: Arc<dyn Query> = Arc::new(RegexpQuery::new(term("body", "te.*"))?);
    let bogus: Arc<dyn Query> = Arc::new(RegexpQuery::new(term("bogus", "te.*"))?);
    single_mtq(t, query, bogus)
}

fn fuzzy(field: &str, text: &str, max_edits: i32) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(FuzzyQuery::with_max_edits(
        term(field, text),
        max_edits,
    )?))
}

fn test_fuzzy(t: &mut T) -> Result<()> {
    let ir = index_two_docs(t)?;

    let searcher = new_searcher(&ir);
    let mut uh_builder = builder(&searcher, t.analyzer());
    let highlighter = random_uh_builder(&mut uh_builder);
    let mut query = fuzzy("body", "tets", 1)?;
    let mut top_docs = search_io(&searcher, &query, 2)?;
    let mut snippets = highlighter.highlight("body", &query, &top_docs)?;
    assert_two(&snippets, HL1, HL2);

    // with prefix
    query = Arc::new(FuzzyQuery::with_prefix_length(term("body", "tets"), 1, 2)?);
    top_docs = search_io(&searcher, &query, 2)?;
    snippets = highlighter.highlight("body", &query, &top_docs)?;
    assert_two(&snippets, HL1, HL2);

    // with zero max edits
    query = Arc::new(FuzzyQuery::with_prefix_length(term("body", "test"), 0, 2)?);
    top_docs = search_io(&searcher, &query, 2)?;
    snippets = highlighter.highlight("body", &query, &top_docs)?;
    assert_two(&snippets, HL1, HL2);

    // wrong field
    let bq = bq(vec![
        (match_all(), Occur::SHOULD),
        (fuzzy("bogus", "tets", 1)?, Occur::SHOULD),
    ])?;
    top_docs = search_io(&searcher, &bq, 2)?;
    uh_builder = uh_builder.with_field_matcher(None);
    snippets = uh_builder
        .clone()
        .build()
        .highlight("body", &bq, &top_docs)?;
    assert_two(&snippets, DOC1, DOC2);
    ir.close()
}

fn test_ranges(t: &mut T) -> Result<()> {
    let ir = index_two_docs(t)?;

    let searcher = new_searcher(&ir);
    let mut uh_builder = builder(&searcher, t.analyzer());
    let highlighter = random_uh_builder(&mut uh_builder);
    let mut query = string_range("body", Some("ta"), Some("tf"), true, true)?;
    let mut top_docs = search_io(&searcher, &query, 2)?;
    let mut snippets = highlighter.highlight("body", &query, &top_docs)?;
    assert_two(&snippets, HL1, HL2);

    // null start
    query = string_range("body", None, Some("tf"), true, true)?;
    top_docs = search_io(&searcher, &query, 2)?;
    snippets = highlighter.highlight("body", &query, &top_docs)?;
    assert_two(
        &snippets,
        "This <b>is</b> <b>a</b> <b>test</b>.",
        "<b>Test</b> <b>a</b> <b>one</b> <b>sentence</b> <b>document</b>.",
    );

    // null end
    query = string_range("body", Some("ta"), None, true, true)?;
    top_docs = search_io(&searcher, &query, 2)?;
    snippets = highlighter.highlight("body", &query, &top_docs)?;
    assert_two(&snippets, "<b>This</b> is a <b>test</b>.", HL2);

    // exact start inclusive
    query = string_range("body", Some("test"), Some("tf"), true, true)?;
    top_docs = search_io(&searcher, &query, 2)?;
    snippets = highlighter.highlight("body", &query, &top_docs)?;
    assert_two(&snippets, HL1, HL2);

    // exact end inclusive
    query = string_range("body", Some("ta"), Some("test"), true, true)?;
    top_docs = search_io(&searcher, &query, 2)?;
    snippets = highlighter.highlight("body", &query, &top_docs)?;
    assert_two(&snippets, HL1, HL2);

    // exact start exclusive
    let mut bq = bq(vec![
        (match_all(), Occur::SHOULD),
        (
            string_range("body", Some("test"), Some("tf"), false, true)?,
            Occur::SHOULD,
        ),
    ])?;
    top_docs = search_io(&searcher, &bq, 2)?;
    snippets = highlighter.highlight("body", &bq, &top_docs)?;
    assert_two(&snippets, DOC1, DOC2);

    // exact end exclusive
    bq = crate::uhighlight::test_unified_highlighter::bq(vec![
        (match_all(), Occur::SHOULD),
        (
            string_range("body", Some("ta"), Some("test"), true, false)?,
            Occur::SHOULD,
        ),
    ])?;
    top_docs = search_io(&searcher, &bq, 2)?;
    snippets = highlighter.highlight("body", &bq, &top_docs)?;
    assert_two(&snippets, DOC1, DOC2);

    // wrong field
    bq = crate::uhighlight::test_unified_highlighter::bq(vec![
        (match_all(), Occur::SHOULD),
        (
            string_range("bogus", Some("ta"), Some("tf"), true, true)?,
            Occur::SHOULD,
        ),
    ])?;
    top_docs = search_io(&searcher, &bq, 2)?;
    uh_builder = uh_builder.with_field_matcher(None);
    snippets = uh_builder
        .clone()
        .build()
        .highlight("body", &bq, &top_docs)?;
    assert_two(&snippets, DOC1, DOC2);
    ir.close()
}

fn test_wildcard_in_boolean(t: &mut T) -> Result<()> {
    let ir = index_two_docs(t)?;

    let searcher = new_searcher(&ir);
    let highlighter = random_uh(&searcher, t.analyzer());
    let mut query = bq(vec![(wildcard("body", "te*")?, Occur::SHOULD)])?;
    let mut top_docs = search_io(&searcher, &query, 2)?;
    let mut snippets = highlighter.highlight("body", &query, &top_docs)?;
    assert_two(&snippets, HL1, HL2);

    // must not
    query = bq(vec![
        (match_all(), Occur::SHOULD),
        (wildcard("bogus", "te*")?, Occur::MUST_NOT),
    ])?;
    top_docs = search_io(&searcher, &query, 2)?;
    snippets = highlighter.highlight("body", &query, &top_docs)?;
    assert_two(&snippets, DOC1, DOC2);
    ir.close()
}

/// The common body of the tests that highlight both documents with one
/// query and `randomUnifiedHighlighter(searcher, indexAnalyzer)`.
fn both_highlighted(t: &UHBase, query: Arc<dyn Query>) -> Result<()> {
    let ir = index_two_docs(t)?;

    let searcher = new_searcher(&ir);
    let highlighter = random_uh(&searcher, t.analyzer());
    let top_docs = search_io(&searcher, &query, 2)?;
    let snippets = highlighter.highlight("body", &query, &top_docs)?;
    assert_two(&snippets, HL1, HL2);
    ir.close()
}

fn test_wildcard_in_filtered(t: &mut T) -> Result<()> {
    let query = bq(vec![
        (wildcard("body", "te*")?, Occur::MUST),
        (tq("body", "test"), Occur::FILTER),
    ])?;
    both_highlighted(t, query)
}

fn test_wildcard_in_constant_score(t: &mut T) -> Result<()> {
    let query: Arc<dyn Query> = Arc::new(ConstantScoreQuery::new(wildcard("body", "te*")?));
    both_highlighted(t, query)
}

fn test_wildcard_in_disjunction_max(t: &mut T) -> Result<()> {
    let query: Arc<dyn Query> = Arc::new(DisjunctionMaxQuery::new(
        vec![wildcard("body", "te*")?],
        0.0,
    )?);
    both_highlighted(t, query)
}

fn test_span_wildcard(t: &mut T) -> Result<()> {
    let query: Arc<dyn Query> = span_wildcard("body", "te*")?;
    both_highlighted(t, query)
}

fn test_span_or(t: &mut T) -> Result<()> {
    let child_query: Arc<dyn SpanQuery> = span_wildcard("body", "te*")?;
    let query: Arc<dyn Query> = Arc::new(SpanOrQuery::new(vec![child_query])?);
    both_highlighted(t, query)
}

fn test_span_near(t: &mut T) -> Result<()> {
    let child_query: Arc<dyn SpanQuery> = span_wildcard("body", "te*")?;
    let query: Arc<dyn Query> = Arc::new(SpanNearQuery::new(
        vec![Arc::clone(&child_query), child_query],
        0,
        false,
    )?);
    both_highlighted(t, query)
}

fn test_span_not(t: &mut T) -> Result<()> {
    let include: Arc<dyn SpanQuery> = span_wildcard("body", "te*")?;
    let exclude: Arc<dyn SpanQuery> = Arc::new(SpanTermQuery::new(term("body", "bogus")));
    let query: Arc<dyn Query> = Arc::new(SpanNotQuery::new(include, exclude)?);
    both_highlighted(t, query)
}

fn test_span_position_check(t: &mut T) -> Result<()> {
    let child_query: Arc<dyn SpanQuery> = span_wildcard("body", "te*")?;
    let query: Arc<dyn Query> = Arc::new(SpanFirstQuery::new(child_query, 1000000));
    both_highlighted(t, query)
}

/// The anonymous `PassageFormatter` of `testWhichMTQMatched`: copied from
/// `DefaultPassageFormatter`, but tweaked to include the matched term.
#[derive(Debug)]
struct MatchedTermFormatter;

impl PassageFormatter for MatchedTermFormatter {
    fn format(&self, passages: &[Passage], content: &str) -> Result<Box<dyn FormattedPassages>> {
        let content: Vec<u16> = content.encode_utf16().collect();
        let mut sb: Vec<u16> = Vec::new();
        let append = |sb: &mut Vec<u16>, from: i32, to: i32| {
            sb.extend_from_slice(&content[from as usize..to as usize]);
        };
        let mut pos = 0;
        for passage in passages {
            // don't add ellipsis if its the first one, or if its connected.
            if passage.get_start_offset() > pos && pos > 0 {
                sb.extend("... ".encode_utf16());
            }
            pos = passage.get_start_offset();
            for i in 0..passage.get_num_matches() as usize {
                let start = passage.get_match_starts()[i];
                let end = passage.get_match_ends()[i];
                // its possible to have overlapping terms
                if start > pos {
                    append(&mut sb, pos, start);
                }
                if end > pos {
                    sb.extend("<b>".encode_utf16());
                    append(&mut sb, pos.max(start), end);
                    sb.push(b'(' as u16);
                    sb.extend(
                        passage.get_match_terms()[i]
                            .utf8_to_string()?
                            .encode_utf16(),
                    );
                    sb.push(b')' as u16);
                    sb.extend("</b>".encode_utf16());
                    pos = end;
                }
            }
            // its possible a "term" from the analyzer could span a sentence boundary.
            append(&mut sb, pos, pos.max(passage.get_end_offset()));
            pos = passage.get_end_offset();
        }
        Ok(Box::new(String::from_utf16_lossy(&sb)))
    }
}

/// The overrides of the anonymous highlighter of `testWhichMTQMatched`.
#[derive(Debug)]
struct MatchedTermFormatterOverride;

impl UnifiedHighlighterOverrides for MatchedTermFormatterOverride {
    fn get_formatter(&self, _this: &UnifiedHighlighter, _field: &str) -> Arc<dyn PassageFormatter> {
        Arc::new(MatchedTermFormatter)
    }
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

/// Runs a query with two MTQs and confirms the formatter can tell which
/// query matched which hit.
fn test_which_mtq_matched(t: &mut T) -> Result<()> {
    let ir = index_values(t, &[DOC2])?;

    let searcher = new_searcher(&ir);
    let mut highlighter = random_uh(&searcher, t.analyzer());
    // use a variety of common MTQ types
    let query = bq(vec![
        (prefix("body", "te")?, Occur::SHOULD),
        (wildcard("body", "*one*")?, Occur::SHOULD),
        (
            Arc::new(FuzzyQuery::new(term("body", "zentence~"))?),
            Occur::SHOULD,
        ),
    ])?;
    let top_docs = search_io(&searcher, &query, 1)?;
    let mut snippets = highlighter.highlight("body", &query, &top_docs)?;
    Assert::assert_equals(1, snippets.len());

    // Default formatter just bolds each hit:
    assert_snippet(
        "<b>Test</b> a <b>one</b> <b>sentence</b> document.",
        &snippets[0],
    );

    let uh_builder = builder(&searcher, t.analyzer());
    // Now use our own formatter, that also stuffs the
    // matching term's text into the result:
    highlighter = UnifiedHighlighter::from_builder(uh_builder);
    highlighter.set_overrides(Arc::new(MatchedTermFormatterOverride));

    Assert::assert_equals(1, top_docs.total_hits.value());
    snippets = highlighter.highlight("body", &query, &top_docs)?;
    Assert::assert_equals(1, snippets.len());

    // All flags are enabled.
    Assert::assert_equals_msg(
        Some(&flags_string(&highlighter, "body")),
        4,
        highlighter.get_flags("body").len(),
    );

    Assert::assert_equals_str(
        Some(&flags_string(&highlighter, "title")),
        Some("<b>Test(body:te*)</b> a <b>one(body:*one*)</b> <b>sentence(sentence)</b> document."),
        snippets[0].as_deref(),
    );

    ir.close()
}

//
//  All tests below were *not* ported from the PostingsHighlighter; they are new to the U.H.
//

fn alpha_bra_query() -> Result<Arc<dyn Query>> {
    bq(vec![
        (tq("body", "alpha"), Occur::MUST),
        (prefix("body", "bra")?, Occur::MUST),
    ])
}

fn test_with_max_len(t: &mut T) -> Result<()> {
    let ir = index_values(t, &["Alpha Bravo foo foo foo. Foo foo Alpha Bravo"])?; // 44 char long, 2 sentences

    let searcher = new_searcher(&ir);
    let mut uh_builder = builder(&searcher, t.analyzer()).with_max_length(25)?; // a little past first sentence
    let highlighter = random_uh_builder(&mut uh_builder);

    let query = alpha_bra_query()?;
    let top_docs = search_index_order(&searcher, &query, 10)?;
    let snippets = highlighter.highlight_max_passages("body", &query, &top_docs, 2)?; // ask for 2 but we'll only get 1
    assert_snippets(&["<b>Alpha</b> <b>Bravo</b> foo foo foo. "], &snippets);
    ir.close()
}

fn test_with_max_len_and_multiple_wildcard_matches(t: &mut T) -> Result<()> {
    // tests interleaving of multiple wildcard matches with the CompositePostingsEnum
    // In this case the CompositePostingsEnum will have an underlying PostingsEnum that jumps form
    // pos 1 to 9 for bravo
    // and a second with position 2 for Bravado
    let ir = index_values(t, &["Alpha Bravo Bravado foo foo foo. Foo foo Alpha Bravo"])?;

    let searcher = new_searcher(&ir);
    let mut uh_builder = builder(&searcher, t.analyzer()).with_max_length(32)?; // a little past first sentence
    let highlighter = random_uh_builder(&mut uh_builder);

    let query = alpha_bra_query()?;
    let top_docs = search_index_order(&searcher, &query, 10)?;
    let snippets = highlighter.highlight_max_passages("body", &query, &top_docs, 2)?; // ask for 2 but we'll only get 1
    assert_snippets(
        &["<b>Alpha</b> <b>Bravo</b> <b>Bravado</b> foo foo foo."],
        &snippets,
    );
    ir.close()
}

/// The anonymous `Tokenizer` of `testTokenStreamIsClosed`, whose
/// `incrementToken()` throws `IOException("EXPECTED")`.
#[derive(Debug, Default)]
struct BuggyTokenizerLogic;

impl TokenizerLogic for BuggyTokenizerLogic {
    fn increment_token(
        &mut self,
        _source: &mut AttributeSource,
        _reader: &mut dyn CharReader,
    ) -> Result<bool> {
        Err(LuceneError::from(std::io::Error::other("EXPECTED")))
    }
}

/// The buggy `Analyzer` of `testTokenStreamIsClosed`.
#[derive(Debug)]
struct BuggyAnalyzer {
    state: AnalyzerState,
}

impl Analyzer for BuggyAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.search.uhighlight.TestUnifiedHighlighterMTQ$2"
    }

    fn create_components(&self, _field_name: Option<&str>) -> Result<TokenStreamComponents> {
        let buggy_tokenizer = Arc::new(AtomicRefCell::new(Tokenizer::with_factory(
            default_token_attribute_factory(),
            BuggyTokenizerLogic,
        )));
        Ok(TokenStreamComponents::from_tokenizer(buggy_tokenizer))
    }
}

fn test_token_stream_is_closed(t: &mut T) -> Result<()> {
    // note: test is a derivative of testWithMaxLen()
    let iw = t.writer()?;

    let body = t.field("body", "")?;
    let mut doc = Document::new();
    add(&mut doc, &body);
    set(&body, "Alpha Bravo foo foo foo. Foo foo Alpha Bravo")?;
    if random().next_boolean() {
        // sometimes add a 2nd value (maybe matters?)
        add(&mut doc, &t.field("body", "2nd value Alpha Bravo")?);
    }
    iw.add_document(&doc)?;

    let ir = iw.get_reader()?;
    iw.close()?;

    // use this buggy Analyzer at highlight time
    let buggy_analyzer: Arc<dyn Analyzer> = Arc::new(BuggyAnalyzer {
        state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
    });

    let searcher = new_searcher(&ir);
    let mut uh_builder =
        builder(&searcher, Arc::clone(&buggy_analyzer)).with_handle_multi_term_query(true);
    if LuceneTestCase::rarely() {
        uh_builder = uh_builder.with_max_length(25)?; // a little past first sentence
    }
    let highlighter = random_uh_builder(&mut uh_builder);

    let mut has_clauses = false;
    let mut clauses: Vec<(Arc<dyn Query>, Occur)> = Vec::new();
    if random().next_boolean() {
        has_clauses = true;
        clauses.push((tq("body", "alpha"), Occur::MUST));
    }
    if !has_clauses || random().next_boolean() {
        clauses.push((prefix("body", "bra")?, Occur::MUST));
    }
    let query = bq(clauses)?;
    let top_docs = search_index_order(&searcher, &query, 10)?;
    match highlighter.highlight_max_passages("body", &query, &top_docs, 2) {
        Ok(_) => {
            // don't even care what the results are; just want to test exception behavior
            if t.is_reanalysis() {
                Assert::fail("Expecting EXPECTED IOException");
            }
        }
        Err(e) => {
            if !e.to_string().contains("EXPECTED") {
                return Err(e);
            }
        }
    }
    ir.close()?;

    // Now test we can get the tokenStream without it puking due to IllegalStateException for not
    // calling close()
    let ts = buggy_analyzer.token_stream_from_str("body", "anything")?;
    let reset = ts.borrow_mut().reset(); // hopefully doesn't throw
                                         // don't call incrementToken; we know it's buggy ;-)
    let closed = ts.borrow_mut().close();
    reset?;
    closed
}

/// Not empty but nothing analyzes. Ensures we address null term-vectors.
fn test_nothing_analyzes(t: &mut T) -> Result<()> {
    let iw = t.writer()?;

    let mut doc = Document::new();
    add(&mut doc, &t.field("body", " ")?); // just a space! (thus not empty)
    doc.add(LuceneTestCase::new_text_field("id", "id", Store::YES));
    iw.add_document(&doc)?;

    doc = Document::new();
    add(&mut doc, &t.field("body", "something")?);
    iw.add_document(&doc)?;

    let ir = iw.get_reader()?;
    iw.close()?;

    let searcher = new_searcher(&ir);
    let highlighter = random_uh(&searcher, t.analyzer());

    let doc_id = searcher.search(tq("id", "id"), 1)?.score_docs[0].doc;

    let query = prefix("body", "nonexistent")?;
    let doc_ids = [doc_id];
    let snippets =
        highlighter.highlight_fields_doc_ids(&["body".to_string()], &query, &doc_ids, &[2])?;
    let snippets = get(&snippets, "body");
    Assert::assert_equals(1, snippets.len());
    assert_snippet(" ", &snippets[0]);

    ir.close()
}

/// `Arrays.toString(snippets)` of `String[]` snippets.
fn arrays_to_string(snippets: &[Option<String>]) -> String {
    format!(
        "[{}]",
        snippets
            .iter()
            .map(|s| s.as_deref().unwrap_or("null").to_string())
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn test_multi_segment(t: &mut T) -> Result<()> {
    // If we incorrectly got the term vector from mis-matched global/leaf doc ID, this test may fail
    let iw = t.writer()?;

    let mut doc = Document::new();
    add(&mut doc, &t.field("body", "word aberration")?);
    iw.add_document(&doc)?;

    iw.commit()?; // make segment

    doc = Document::new();
    add(&mut doc, &t.field("body", "word absolve")?);
    iw.add_document(&doc)?;

    let ir = iw.get_reader()?;
    iw.close()?;

    let searcher = new_searcher(&ir);
    let highlighter = random_uh(&searcher, t.analyzer());
    let query = prefix("body", "ab")?;
    let top_docs = searcher.search(Arc::clone(&query), 10)?;

    let snippets = highlighter.highlight_fields(&["body".to_string()], &query, &top_docs)?;
    let mut snippets = get(&snippets, "body").to_vec();
    // Arrays.sort(Object[]): natural (String) order; no snippet is null here
    snippets.sort_by(|a, b| {
        let a: Vec<u16> = a.as_deref().unwrap_or_default().encode_utf16().collect();
        let b: Vec<u16> = b.as_deref().unwrap_or_default().encode_utf16().collect();
        a.cmp(&b)
    });
    Assert::assert_equals(
        "[word <b>aberration</b>, word <b>absolve</b>]".to_string(),
        arrays_to_string(&snippets),
    );

    ir.close()
}

fn test_position_sensitive_with_wildcard_does_not_highlight(t: &mut T) -> Result<()> {
    let iw = t.writer()?;
    let mut doc = Document::new();
    add(
        &mut doc,
        &t.field("body", "iterate insect ipswitch illinois indirect")?,
    );
    doc.add(LuceneTestCase::new_text_field("id", "id", Store::YES));

    iw.add_document(&doc)?;
    let ir = iw.get_reader()?;
    iw.close()?;

    let searcher = new_searcher(&ir);
    let highlighter = random_uh(&searcher, t.analyzer());
    let doc_id = searcher.search(tq("id", "id"), 1)?.score_docs[0].doc;

    let pq = pq_builder("body", &["consent", "order"])?;

    let query = bq(vec![
        (wildcard("body", "enforc*")?, Occur::MUST),
        (pq, Occur::MUST),
    ])?;

    let doc_ids = [doc_id];

    let snippets =
        highlighter.highlight_fields_doc_ids(&["body".to_string()], &query, &doc_ids, &[2])?;
    let snippets = get(&snippets, "body");
    Assert::assert_equals(1, snippets.len());
    assert_snippet("iterate insect ipswitch illinois indirect", &snippets[0]);
    ir.close()
}

fn test_custom_span_query_highlighting(t: &mut T) -> Result<()> {
    let iw = t.writer()?;
    let mut doc = Document::new();
    add(
        &mut doc,
        &t.field(
            "body",
            "alpha bravo charlie delta echo foxtrot golf hotel india juliet",
        )?,
    );
    doc.add(LuceneTestCase::new_text_field("id", "id", Store::YES));

    iw.add_document(&doc)?;
    let ir = iw.get_reader()?;
    iw.close()?;

    let searcher = new_searcher(&ir);
    let highlighter = UnifiedHighlighter::builder(Arc::clone(&searcher), t.analyzer()).build();

    let doc_id = searcher.search(tq("id", "id"), 1)?.score_docs[0].doc;

    let wildcard_query_wrapper: Arc<dyn SpanQuery> = span_wildcard("body", "foxtr*")?;
    let wrapped_query: Arc<dyn Query> = Arc::new(MyWrapperSpanQuery::new(wildcard_query_wrapper));

    let query = bq(vec![(wrapped_query, Occur::SHOULD)])?;

    let doc_ids = [doc_id];

    let snippets =
        highlighter.highlight_fields_doc_ids(&["body".to_string()], &query, &doc_ids, &[2])?;
    let snippets = get(&snippets, "body");
    Assert::assert_equals(1, snippets.len());
    assert_snippet(
        "alpha bravo charlie delta echo <b>foxtrot</b> golf hotel india juliet",
        &snippets[0],
    );
    ir.close()
}

/// `private static class MyWrapperSpanQuery extends SpanQuery`.
#[derive(Debug)]
struct MyWrapperSpanQuery {
    original_query: Arc<dyn SpanQuery>,
}

impl MyWrapperSpanQuery {
    fn new(original_query: Arc<dyn SpanQuery>) -> Self {
        Self { original_query }
    }
}

impl Query for MyWrapperSpanQuery {
    fn to_query_string(&self, field: &str) -> String {
        format!("(Wrapper[{}])", self.original_query.to_query_string(field))
    }

    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        self.original_query
            .create_weight(searcher, score_mode, boost)
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        let mut sub = visitor.get_sub_visitor(Occur::MUST, self);
        self.original_query.visit(&mut *sub);
    }

    fn rewrite(&self, index_searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        match self.original_query.rewrite_span(index_searcher)? {
            Some(new_original_query) => {
                Ok(Some(Arc::new(MyWrapperSpanQuery::new(new_original_query))))
            }
            None => Ok(None),
        }
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
            && other
                .as_any()
                .downcast_ref::<MyWrapperSpanQuery>()
                .is_some_and(|o| {
                    self.original_query
                        .query_eq(&*o.original_query as &dyn Query)
                })
    }

    fn query_hash(&self) -> i32 {
        self.original_query.query_hash()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.search.uhighlight.TestUnifiedHighlighterMTQ$MyWrapperSpanQuery"
    }
}

impl SpanQuery for MyWrapperSpanQuery {
    fn get_field(&self) -> Option<&str> {
        self.original_query.get_field()
    }

    fn create_span_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn SpanWeight>> {
        self.original_query
            .create_span_weight(searcher, score_mode, boost)
    }

    fn rewrite_span(&self, index_searcher: &IndexSearcher) -> Result<Option<Arc<dyn SpanQuery>>> {
        match self.original_query.rewrite_span(index_searcher)? {
            Some(new_original_query) => {
                Ok(Some(Arc::new(MyWrapperSpanQuery::new(new_original_query))))
            }
            None => Ok(None),
        }
    }
}

// LUCENE-7717 bug, ordering of MTQ AutomatonQuery detection
fn test_russian_prefix_query(t: &mut T) -> Result<()> {
    let analyzer: Arc<dyn Analyzer> = Arc::new(StandardAnalyzer::new());
    let iw = RandomIndexWriter::new(&*random(), t.dir().as_directory(), Arc::clone(&analyzer))?;
    let field = "title";
    let mut doc = Document::new();
    add(&mut doc, &t.field(field, "\u{044f}")?); // Russian char; uses 2 UTF8 bytes
    iw.add_document(&doc)?;
    let ir = iw.get_reader()?;
    iw.close()?;

    let searcher = new_searcher(&ir);
    let query = prefix(field, "\u{044f}")?;
    let top_docs = searcher.search(Arc::clone(&query), 1)?;
    let highlighter = random_uh(&searcher, analyzer);
    let snippets = highlighter.highlight(field, &query, &top_docs)?;
    Assert::assert_equals("[<b>\u{044f}</b>]".to_string(), arrays_to_string(&snippets));
    ir.close()
}

// LUCENE-7719
fn test_multi_byte_mtq(t: &mut T) -> Result<()> {
    let analyzer: Arc<dyn Analyzer> = Arc::new(KeywordAnalyzer::new());
    let iw = RandomIndexWriter::new(&*random(), t.dir().as_directory(), Arc::clone(&analyzer))?;
    let result = (|| -> Result<()> {
        for _attempt in 0..20 {
            iw.delete_all()?;
            let field = "title";
            let value = RandomStrings::random_unicode_of_length(&*random(), 3);
            if value.encode_utf16().any(|u| u == MULTIVAL_SEP_CHAR) {
                // will throw things off
                continue;
            }
            let value_points: Vec<i32> = value.chars().map(|c| c as i32).collect();

            let mut doc = Document::new();
            add(&mut doc, &t.field(field, &value)?);
            iw.add_document(&doc)?;
            iw.commit()?;
            let ir = iw.get_reader()?;
            let checked = (|| -> Result<()> {
                let searcher = new_searcher(&ir);
                let mut uh_builder = builder(&searcher, Arc::clone(&analyzer))
                    .with_break_iterator(Arc::new(|| {
                        Box::new(rucene_highlighter::uhighlight::whole_break_iterator::WholeBreakIterator::new())
                            as Box<dyn rucene_analysis_common::jdk::break_iterator::BreakIterator>
                    }));
                let highlighter = random_uh_builder(&mut uh_builder);

                // Test PrefixQuery
                let mut query = prefix(
                    field,
                    &rucene::util::unicode_util::UnicodeUtil::new_string(&value_points, 0, 1)?,
                )?;
                highlight_and_assert_match(&searcher, &highlighter, &query, field, &value)?;

                // Test TermRangeQuery
                query = Arc::new(TermRangeQuery::new(
                    field,
                    Some(BytesRef::from_char_sequence(&value)),
                    Some(BytesRef::from_char_sequence(&value)),
                    true,
                    true,
                )?);
                highlight_and_assert_match(&searcher, &highlighter, &query, field, &value)?;

                // Test FuzzyQuery
                query = fuzzy(field, &format!("{value}Z"), 1)?;
                highlight_and_assert_match(&searcher, &highlighter, &query, field, &value)?;

                if value_points.len() != 3 {
                    return Ok(()); // even though we ask RandomStrings for a String with 3 code points, it seems
                                   // sometimes it's less
                }

                // Test WildcardQuery
                let mut pattern = String::new();
                pattern.push(WILDCARD_ESCAPE);
                pattern.push(char::from_u32(value_points[0] as u32).expect("a code point"));
                pattern.push(WILDCARD_CHAR);
                pattern.push(WILDCARD_ESCAPE);
                pattern.push(char::from_u32(value_points[2] as u32).expect("a code point"));
                query = wildcard(field, &pattern)?;
                highlight_and_assert_match(&searcher, &highlighter, &query, field, &value)?;

                // TODO hmmm; how to randomly generate RegexpQuery? Low priority; we've covered the others
                // well.
                Ok(())
            })();
            let closed = ir.close();
            checked?;
            closed?;
        }
        Ok(())
    })();
    let closed = iw.close();
    result?;
    closed
}

fn highlight_and_assert_match(
    searcher: &Arc<IndexSearcher>,
    highlighter: &UnifiedHighlighter,
    query: &Arc<dyn Query>,
    field: &str,
    field_val: &str,
) -> Result<()> {
    let top_docs = searcher.search(Arc::clone(query), 1)?;
    Assert::assert_equals(1, top_docs.total_hits.value());
    let snippets = highlighter.highlight(field, query, &top_docs)?;
    Assert::assert_equals(format!("[<b>{field_val}</b>]"), arrays_to_string(&snippets));
    Ok(())
}
