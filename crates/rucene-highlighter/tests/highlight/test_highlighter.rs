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

//! Port of `TestHighlighter.java` (with its supporting classes
//! `SynonymAnalyzer`, `SynonymTokenizer` and
//! `SynonymTokenizer.TestHighlightRunner`).
//!
//! The test class implements `Formatter`: its `highlightTerm` counts the
//! highlights in `numHighlights`, which [`HitFormatter`] shares with the
//! fixture.

use std::any::Any;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::junit::Assert;
use rucene::analysis::caching_token_filter::CachingTokenFilter;
use rucene::analysis::{default_token_attribute_factory, Analyzer, SharedTokenStream, TokenStream};
use rucene::document::{
    Document, Field, FieldType, IntPoint, NumericValue, SortedNumericDocValuesField, StoredField,
    TextField,
};
use rucene::error::Result;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::IndexReader;
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query::Builder as BooleanQueryBuilder;
use rucene::search::constant_score_query::ConstantScoreQuery;
use rucene::search::double_values_source;
use rucene::search::field_exists_query::FieldExistsQuery;
use rucene::search::fuzzy_query::{
    FuzzyQuery, DEFAULT_MAX_EXPANSIONS, DEFAULT_PREFIX_LENGTH, DEFAULT_TRANSPOSITIONS,
};
use rucene::search::index_or_doc_values_query::IndexOrDocValuesQuery;
use rucene::search::multi_phrase_query::Builder as MultiPhraseQueryBuilder;
use rucene::search::multi_term_query::{constant_score_blended_rewrite, scoring_boolean_rewrite};
use rucene::search::phrase_query::{Builder as PhraseQueryBuilder, PhraseQuery};
use rucene::search::prefix_query::PrefixQuery;
use rucene::search::query::Query;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::regexp_query::RegexpQuery;
use rucene::search::sort::{Sort, SortField};
use rucene::search::synonym_query;
use rucene::search::term_range_query::TermRangeQuery;
use rucene::search::top_docs::TopDocs;
use rucene::search::wildcard_query::WildcardQuery;
use rucene::search::{IndexSearcher, TermQuery};
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::attribute::AttributeSource;
use rucene::util::automaton::operations::DEFAULT_DETERMINIZE_WORK_LIMIT;
use rucene::util::automaton::{Automata, CharacterRunAutomaton, RegExp};
use rucene::util::BytesRef;
use rucene_analysis_common::analysis::ngram::ngram_tokenizer::new_ngram_tokenizer;
use rucene_analysis_common::jdk::sax::{Attributes, DefaultHandler, InputSource, XmlReader};
use rucene_highlighter::highlight::formatter::Formatter;
use rucene_highlighter::highlight::highlighter::Highlighter;
use rucene_highlighter::highlight::null_fragmenter::NullFragmenter;
use rucene_highlighter::highlight::position_span::PositionSpan;
use rucene_highlighter::highlight::query_scorer::QueryScorer;
use rucene_highlighter::highlight::query_term_scorer::QueryTermScorer;
use rucene_highlighter::highlight::scorer::Scorer;
use rucene_highlighter::highlight::simple_fragmenter::SimpleFragmenter;
use rucene_highlighter::highlight::simple_html_encoder::SimpleHTMLEncoder;
use rucene_highlighter::highlight::simple_html_formatter::SimpleHTMLFormatter;
use rucene_highlighter::highlight::simple_span_fragmenter::SimpleSpanFragmenter;
use rucene_highlighter::highlight::text_fragment::TextFragment;
use rucene_highlighter::highlight::token_group::TokenGroup;
use rucene_highlighter::highlight::token_sources::TokenSources;
use rucene_highlighter::highlight::weighted_span_term::WeightedSpanTerm;
use rucene_highlighter::highlight::weighted_span_term_extractor::{
    PositionCheckingMap, WeightedSpanTermExtractor, WeightedSpanTermExtractorOverrides,
};
use rucene_highlighter::highlight::weighted_term::WeightedTerm;
use rucene_queries::common_terms_query::CommonTermsQuery;
use rucene_queries::function::function_score_query::FunctionScoreQuery;
use rucene_queries::payloads::span_payload_check_query::SpanPayloadCheckQuery;
use rucene_queries::spans::span_multi_term_query_wrapper::SpanMultiTermQueryWrapper;
use rucene_queries::spans::span_near_query::SpanNearQuery;
use rucene_queries::spans::span_not_query::SpanNotQuery;
use rucene_queries::spans::span_or_query::SpanOrQuery;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_queries::spans::span_term_query::SpanTermQuery;
use rucene_test_framework::analysis::attributes::{char_term_id, offset_id, pos_inc_id, Att};
use rucene_test_framework::analysis::mock_analyzer::BoxedStream;
use rucene_test_framework::analysis::mock_payload_analyzer::MockPayloadAnalyzer;
use rucene_test_framework::analysis::mock_token_filter::ENGLISH_STOPSET;
use rucene_test_framework::analysis::mock_tokenizer::{SIMPLE, WHITESPACE};
use rucene_test_framework::analysis::token::Token;
use rucene_test_framework::analysis::{
    components, shared, sink, tokenizer_components, BaseTokenStreamTestCase, CannedTokenStream,
    MockAnalyzer, MockTokenizer,
};
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use super::test_highlighter_phrase::FixedTokenStream;
use crate::highlighter_support::attempt;
use crate::highlighter_support::core_support::{random, verbose};
use crate::highlighter_support::search_support::{ftest, set_up as su, tear_down as td, term};
use crate::highlighter_support::FnAnalyzer;

pub(crate) const FIELD_NAME: &str = "contents";
const NUMERIC_FIELD_NAME: &str = "nfield";

const TEXTS: [&str; 9] = [
    "Hello this is a piece of text that is very long and contains too much preamble and the meat is really here which says kennedy has been shot",
    "This piece of text refers to Kennedy at the beginning then has a longer piece of text that is very long in the middle and finally ends with another reference to Kennedy",
    "JFK has been shot",
    "John Kennedy has been shot",
    "This text has a typo in referring to Keneddy",
    "wordx wordy wordz wordx wordy wordx worda wordb wordy wordc",
    "y z x y z a b",
    "lets is a the lets is a the lets is a the lets",
    "Attribute instances are reused for all tokens of a document. Thus, a TokenStream/-Filter needs to update the appropriate Attribute(s) in incrementToken(). The consumer, commonly the Lucene indexer, consumes the data in the Attributes and then calls incrementToken() again until it retuns false, which indicates that the end of the stream was reached. This means that in each call of incrementToken() a TokenStream/-Filter can safely overwrite the data in the Attribute instances. ",
];

/// The instance fields of `TestHighlighter`.
#[derive(Default)]
pub struct Th {
    reader: Option<Arc<dyn DirectoryReader>>,
    query: Option<Arc<dyn Query>>,
    dir1: Option<TestDirectory>,
    searcher: Option<IndexSearcher>,
    num_highlights: Arc<AtomicI32>,
    analyzer: Option<Arc<MockAnalyzer>>,
    hits: Option<TopDocs>,
    /// see doc()
    field_type: Option<Arc<FieldType>>,
    dir2: Option<TestDirectory>,
    a: Option<Arc<MockAnalyzer>>,
}

/// `TestHighlighter.this` as a `Formatter`: `highlightTerm` counts in
/// `numHighlights`.
#[derive(Debug, Clone)]
struct HitFormatter {
    num_highlights: Arc<AtomicI32>,
}

impl Formatter for HitFormatter {
    fn highlight_term(&self, original_text: &str, group: &TokenGroup) -> String {
        if group.get_total_score() <= 0.0 {
            return original_text.to_string();
        }
        self.num_highlights.fetch_add(1, Ordering::SeqCst); // update stats used in assertions
        format!("<B>{original_text}</B>")
    }
}

impl Th {
    fn this(&self) -> Box<dyn Formatter> {
        Box::new(HitFormatter {
            num_highlights: Arc::clone(&self.num_highlights),
        })
    }

    fn nh(&self) -> i32 {
        self.num_highlights.load(Ordering::SeqCst)
    }

    fn reset_nh(&self) {
        self.num_highlights.store(0, Ordering::SeqCst);
    }

    fn reader(&self) -> Arc<dyn DirectoryReader> {
        Arc::clone(self.reader.as_ref().expect("setUp ran"))
    }

    fn searcher(&self) -> &IndexSearcher {
        self.searcher.as_ref().expect("a searcher")
    }

    fn analyzer(&self) -> Arc<MockAnalyzer> {
        Arc::clone(self.analyzer.as_ref().expect("setUp ran"))
    }

    fn query(&self) -> Arc<dyn Query> {
        Arc::clone(self.query.as_ref().expect("a query"))
    }

    fn hits(&self) -> &TopDocs {
        self.hits.as_ref().expect("hits")
    }

    /// `hits.totalHits.value()`.
    fn total_hits(&self) -> i64 {
        self.hits().total_hits.value()
    }

    /// `hits.scoreDocs[i].doc`.
    fn hit_doc(&self, i: i64) -> i32 {
        self.hits().score_docs[i as usize].doc
    }

    /// `searcher.storedFields().document(doc_id).get(field)`.
    fn stored(&self, doc_id: i32, field: &str) -> Result<String> {
        Ok(self
            .searcher()
            .stored_fields()?
            .document(doc_id)?
            .get(field)
            .expect("a stored value"))
    }

    /// `analyzer.tokenStream(field, text)`.
    fn token_stream(&self, field: &str, text: &str) -> Result<Box<dyn TokenStream>> {
        Ok(Box::new(SharedTokenStream::new(
            self.analyzer().token_stream_from_str(field, text)?,
        )))
    }

    /// `newSearcher(reader)`.
    fn new_searcher(&mut self) {
        self.searcher = Some(LuceneTestCase::new_searcher(self.reader()));
    }

    // Convenience method for succinct tests; doesn't represent "best practice"
    fn get_any_token_stream(&self, field_name: &str, doc_id: i32) -> Result<Box<dyn TokenStream>> {
        let tv = self
            .searcher()
            .get_index_reader()
            .term_vectors()?
            .get(doc_id)?;
        let text = self.stored(doc_id, field_name)?;
        TokenSources::get_token_stream(field_name, tv.as_deref(), &text, &*self.analyzer(), -1)
    }

    fn assert_num_highlights(&self, expected: i32) {
        let n = self.nh();
        Assert::assert_true_msg(
            &format!("Failed to find correct number of highlights {n} found"),
            n == expected,
        );
    }

    fn do_searching(&mut self, un_rewritten_query: Arc<dyn Query>) -> Result<()> {
        self.new_searcher();
        // for any multi-term queries to work (prefix, wildcard, range,fuzzy etc)
        // you must use a rewritten query!
        let query = un_rewritten_query
            .rewrite(self.searcher())?
            .unwrap_or(un_rewritten_query);
        if verbose() {
            println!("Searching for: {}", query.to_query_string(FIELD_NAME));
        }
        self.hits = Some(self.searcher().search(Arc::clone(&query), 1000)?);
        self.query = Some(query);
        Ok(())
    }

    fn assert_expected_highlight_count(
        &self,
        max_num_fragments_required: i32,
        expected_highlights: i32,
    ) -> Result<()> {
        for i in 0..self.total_hits() {
            let text = self.stored(self.hit_doc(i), FIELD_NAME)?;
            let token_stream = self.token_stream(FIELD_NAME, &text)?;
            let scorer = QueryScorer::with_field(self.query(), Some(FIELD_NAME));
            let mut highlighter = Highlighter::with_formatter(self.this(), Box::new(scorer));

            highlighter.set_text_fragmenter(Box::new(SimpleFragmenter::with_fragment_size(40)));

            let result = highlighter.get_best_fragments_with_separator(
                token_stream,
                &text,
                max_num_fragments_required,
                "...",
            )?;
            if verbose() {
                println!("\t{result}");
            }

            self.assert_num_highlights(expected_highlights);
        }
        Ok(())
    }

    /// `doc(name, value)`: fieldType is randomly chosen for term vectors in
    /// setUp.
    fn doc(&self, name: &str, value: &str) -> Result<Document> {
        let mut d = Document::new();
        d.add(Field::new(
            name.to_string(),
            value.to_string(),
            Arc::clone(self.field_type.as_ref().expect("setUp ran")),
        )?);
        Ok(d)
    }
}

/// `TestHighlighter`.
pub fn class_def() -> Arc<Class> {
    let methods: Vec<Method> = vec![
        su(set_up),
        td(tear_down),
        ftest("testFunctionScoreQuery", test_function_score_query),
        ftest("testQueryScorerHits", test_query_scorer_hits),
        ftest(
            "testHighlightingCommonTermsQuery",
            test_highlighting_common_terms_query,
        ),
        ftest(
            "testHighlightingSynonymQuery",
            test_highlighting_synonym_query,
        ),
        ftest(
            "testHighlightingIndexOrDocValuesQuery",
            test_highlighting_index_or_doc_values_query,
        ),
        ftest(
            "testHighlightUnknownQueryAfterRewrite",
            test_highlight_unknown_query_after_rewrite,
        ),
        ftest(
            "testHighlightingWithDefaultField",
            test_highlighting_with_default_field,
        ),
        ftest("testSimpleSpanHighlighter", test_simple_span_highlighter),
        ftest(
            "testSimpleSpanHighlighterWithStopWordsStraddlingFragmentBoundaries",
            test_simple_span_highlighter_with_stop_words_straddling_fragment_boundaries,
        ),
        ftest(
            "testRepeatingTermsInMultBooleans",
            test_repeating_terms_in_mult_booleans,
        ),
        ftest(
            "testSimpleQueryScorerPhraseHighlighting",
            test_simple_query_scorer_phrase_highlighting,
        ),
        ftest("testSpanRegexQuery", test_span_regex_query),
        ftest("testRegexQuery", test_regex_query),
        ftest("testExternalReader", test_external_reader),
        ftest("testDimensionalRangeQuery", test_dimensional_range_query),
        ftest(
            "testSimpleQueryScorerPhraseHighlighting2",
            test_simple_query_scorer_phrase_highlighting2,
        ),
        ftest(
            "testSimpleQueryScorerPhraseHighlighting3",
            test_simple_query_scorer_phrase_highlighting3,
        ),
        ftest("testSimpleSpanFragmenter", test_simple_span_fragmenter),
        ftest("testPosTermStdTerm", test_pos_term_std_term),
        ftest(
            "testQueryScorerMultiPhraseQueryHighlighting",
            test_query_scorer_multi_phrase_query_highlighting,
        ),
        ftest(
            "testQueryScorerMultiPhraseQueryHighlightingWithGap",
            test_query_scorer_multi_phrase_query_highlighting_with_gap,
        ),
        ftest("testNearSpanSimpleQuery", test_near_span_simple_query),
        ftest(
            "testSimpleQueryTermScorerHighlighter",
            test_simple_query_term_scorer_highlighter,
        ),
        ftest("testSpanHighlighting", test_span_highlighting),
        ftest("testNotSpanSimpleQuery", test_not_span_simple_query),
        ftest(
            "testGetBestFragmentsSimpleQuery",
            test_get_best_fragments_simple_query,
        ),
        ftest(
            "testGetBestFragmentsConstantScore",
            test_get_best_fragments_constant_score,
        ),
        ftest("testGetFuzzyFragments", test_get_fuzzy_fragments),
        ftest("testGetWildCardFragments", test_get_wild_card_fragments),
        ftest(
            "testGetMidWildCardFragments",
            test_get_mid_wild_card_fragments,
        ),
        ftest("testGetRangeFragments", test_get_range_fragments),
        ftest(
            "testConstantScoreMultiTermQuery",
            test_constant_score_multi_term_query,
        ),
        ftest("testGetBestFragmentsPhrase", test_get_best_fragments_phrase),
        ftest(
            "testGetBestFragmentsQueryScorer",
            test_get_best_fragments_query_scorer,
        ),
        ftest("testOffByOne", test_off_by_one),
        ftest(
            "testGetBestFragmentsFilteredQuery",
            test_get_best_fragments_filtered_query,
        ),
        ftest(
            "testGetBestFragmentsFilteredPhraseQuery",
            test_get_best_fragments_filtered_phrase_query,
        ),
        ftest(
            "testGetBestFragmentsMultiTerm",
            test_get_best_fragments_multi_term,
        ),
        ftest(
            "testGetBestFragmentsWithOr",
            test_get_best_fragments_with_or,
        ),
        ftest("testGetBestSingleFragment", test_get_best_single_fragment),
        ftest(
            "testNotRewriteMultiTermQuery",
            test_not_rewrite_multi_term_query,
        ),
        ftest("testFieldExistsQuery", test_field_exists_query),
        ftest(
            "testGetBestSingleFragmentWithWeights",
            test_get_best_single_fragment_with_weights,
        ),
        ftest("testOverlapAnalyzer", test_overlap_analyzer),
        ftest("testGetSimpleHighlight", test_get_simple_highlight),
        ftest("testGetTextFragments", test_get_text_fragments),
        ftest("testMaxSizeHighlight", test_max_size_highlight),
        ftest(
            "testMaxSizeHighlightTruncates",
            test_max_size_highlight_truncates,
        ),
        ftest("testMaxSizeEndHighlight", test_max_size_end_highlight),
        ftest(
            "testHighlighterWithPhraseQuery",
            test_highlighter_with_phrase_query,
        ),
        ftest(
            "testHighlighterWithMultiPhraseQuery",
            test_highlighter_with_multi_phrase_query,
        ),
        ftest("testUnRewrittenQuery", test_un_rewritten_query),
        ftest("testNoFragments", test_no_fragments),
        ftest("testEncoding", test_encoding),
        ftest(
            "testFieldSpecificHighlighting",
            test_field_specific_highlighting,
        ),
        ftest("testOverlapAnalyzer2", test_overlap_analyzer2),
        ftest(
            "testWeightedTermsWithDeletes",
            test_weighted_terms_with_deletes,
        ),
        ftest("testPayloadQuery", test_payload_query),
        ftest(
            "testSortedNumericDocValuesRangeQuery",
            test_sorted_numeric_doc_values_range_query,
        ),
    ];
    let mut builder = Class::builder("org.apache.lucene.search.highlight.TestHighlighter")
        .extends(&BaseTokenStreamTestCase::class())
        .constructor(Constructor::no_args(Th::default));
    for m in methods {
        builder = builder.method(m);
    }
    builder.build()
}

fn tq(field: &str, text: &str) -> Arc<dyn Query> {
    Arc::new(TermQuery::new(term(field, text)))
}

fn phrase(slop: i32, field: &str, terms: &[&str]) -> Result<Arc<dyn Query>> {
    Ok(Arc::new(PhraseQuery::with_slop(slop, field, terms)?))
}

fn span_near(field: &str, terms: &[&str], slop: i32, in_order: bool) -> Result<Arc<dyn SpanQuery>> {
    let clauses: Vec<Arc<dyn SpanQuery>> = terms
        .iter()
        .map(|t| Arc::new(SpanTermQuery::new(term(field, t))) as Arc<dyn SpanQuery>)
        .collect();
    Ok(Arc::new(SpanNearQuery::new(clauses, slop, in_order)?))
}

fn as_query(q: Arc<dyn SpanQuery>) -> Arc<dyn Query> {
    q
}

/// `new Sort(SortField.FIELD_DOC, SortField.FIELD_SCORE)`.
fn doc_then_score() -> Result<Sort> {
    Sort::new_fields(vec![
        SortField::FIELD_DOC.clone(),
        SortField::FIELD_SCORE.clone(),
    ])
}

/// The common body of `testFunctionScoreQuery`,
/// `testHighlightingCommonTermsQuery` and
/// `testHighlightUnknownQueryAfterRewrite`: two hits sorted by doc, and the
/// best fragment of the first (and, with `second`, of the second).
fn sorted_two_hits(
    f: &mut Th,
    query: Arc<dyn Query>,
    first: &str,
    second: Option<&str>,
) -> Result<()> {
    f.new_searcher();
    let hits = f
        .searcher()
        .search_with_sort(Arc::clone(&query), 10, &doc_then_score()?)?;
    Assert::assert_equals(2, hits.total_hits.value());
    let scorer = QueryScorer::with_field(query, Some(FIELD_NAME));
    let mut highlighter = Highlighter::new(Box::new(scorer.clone()));

    let doc_id0 = hits.score_docs[0].doc();
    let stored_field = f.stored(doc_id0, FIELD_NAME)?;
    let stream = f.get_any_token_stream(FIELD_NAME, doc_id0)?;
    highlighter.set_text_fragmenter(Box::new(SimpleSpanFragmenter::new(scorer.clone())));
    let fragment = highlighter.get_best_fragment_from_stream(stream, &stored_field)?;
    Assert::assert_equals_str(None, Some(first), fragment.as_deref());

    if let Some(second) = second {
        let doc_id1 = hits.score_docs[1].doc();
        let stored_field = f.stored(doc_id1, FIELD_NAME)?;
        let stream = f.get_any_token_stream(FIELD_NAME, doc_id1)?;
        highlighter.set_text_fragmenter(Box::new(SimpleSpanFragmenter::new(scorer)));
        let fragment = highlighter.get_best_fragment_from_stream(stream, &stored_field)?;
        Assert::assert_equals_str(None, Some(second), fragment.as_deref());
    }
    Ok(())
}

fn test_function_score_query(f: &mut Th) -> Result<()> {
    let term_query = tq(FIELD_NAME, "very");
    let query: Arc<dyn Query> = Arc::new(FunctionScoreQuery::new(
        term_query,
        double_values_source::constant(1.0),
    ));
    sorted_two_hits(
        f,
        query,
        "Hello this is a piece of text that is <B>very</B> long and contains too much preamble and the meat is really here which says kennedy has been shot",
        None,
    )
}

fn test_query_scorer_hits(f: &mut Th) -> Result<()> {
    let phrase_query = phrase(0, FIELD_NAME, &["very", "long"])?;

    f.query = Some(phrase_query);
    f.new_searcher();
    let hits = f.searcher().search(f.query(), 10)?;

    let scorer = QueryScorer::with_field(f.query(), Some(FIELD_NAME));
    let mut highlighter = Highlighter::new(Box::new(scorer.clone()));

    for score_doc in &hits.score_docs {
        let doc_id = score_doc.doc;
        let stored_field = f.stored(doc_id, FIELD_NAME)?;

        let stream = f.get_any_token_stream(FIELD_NAME, doc_id)?;

        highlighter.set_text_fragmenter(Box::new(SimpleSpanFragmenter::new(scorer.clone())));

        let fragment = highlighter.get_best_fragment_from_stream(stream, &stored_field)?;

        if verbose() {
            println!("{fragment:?}");
        }
    }
    Ok(())
}

fn common_terms_query() -> Result<Arc<dyn Query>> {
    let mut query = CommonTermsQuery::new(Occur::MUST, Occur::SHOULD, 3.0)?;
    query.add(term(FIELD_NAME, "this")); // stop-word
    query.add(term(FIELD_NAME, "long"));
    query.add(term(FIELD_NAME, "very"));
    Ok(Arc::new(query))
}

fn test_highlighting_common_terms_query(f: &mut Th) -> Result<()> {
    sorted_two_hits(
        f,
        common_terms_query()?,
        "Hello this is a piece of text that is <B>very</B> <B>long</B> and contains too much preamble and the meat is really here which says kennedy has been shot",
        Some("This piece of text refers to Kennedy at the beginning then has a longer piece of text that is <B>very</B>"),
    )
}

/// The common checks of `testHighlightingSynonymQuery` and
/// `testHighlightingIndexOrDocValuesQuery` over documents 2 and 3.
fn jfk_kennedy(f: &Th, query: Arc<dyn Query>, span_fragmenter: bool) -> Result<()> {
    let scorer = QueryScorer::with_field(query, Some(FIELD_NAME));
    let mut highlighter = Highlighter::new(Box::new(scorer.clone()));
    let stream = f.get_any_token_stream(FIELD_NAME, 2)?;
    if span_fragmenter {
        highlighter.set_text_fragmenter(Box::new(SimpleSpanFragmenter::new(scorer)));
    }
    let stored_field = f.stored(2, FIELD_NAME)?;
    let fragment = highlighter.get_best_fragment_from_stream(stream, &stored_field)?;
    Assert::assert_equals_str(None, Some("<B>JFK</B> has been shot"), fragment.as_deref());

    let stream = f.get_any_token_stream(FIELD_NAME, 3)?;
    let stored_field = f.stored(3, FIELD_NAME)?;
    let fragment = highlighter.get_best_fragment_from_stream(stream, &stored_field)?;
    Assert::assert_equals_str(
        None,
        Some("John <B>Kennedy</B> has been shot"),
        fragment.as_deref(),
    );
    Ok(())
}

fn test_highlighting_synonym_query(f: &mut Th) -> Result<()> {
    f.new_searcher();
    let mut builder = synonym_query::Builder::new(Some(FIELD_NAME));
    builder.add_term(&term(FIELD_NAME, "jfk"))?;
    builder.add_term(&term(FIELD_NAME, "kennedy"))?;
    let query: Arc<dyn Query> = Arc::new(builder.build());
    jfk_kennedy(f, query, true)
}

fn test_highlighting_index_or_doc_values_query(f: &mut Th) -> Result<()> {
    f.new_searcher();
    let mut boolean_query_builder = BooleanQueryBuilder::new();
    boolean_query_builder.add(tq(FIELD_NAME, "jfk"), Occur::SHOULD)?;
    boolean_query_builder.add(tq(FIELD_NAME, "kennedy"), Occur::SHOULD)?;
    let index_query: Arc<dyn Query> = Arc::new(boolean_query_builder.build());
    let dv_query: Arc<dyn Query> = Arc::new(TermRangeQuery::new_string_range(
        FIELD_NAME,
        Some("a"),
        Some("z"),
        true,
        true,
    )?);
    let query: Arc<dyn Query> = Arc::new(IndexOrDocValuesQuery::new(index_query, dv_query));
    jfk_kennedy(f, query, false)
}

/// The anonymous `Query` of `testHighlightUnknownQueryAfterRewrite`.
#[derive(Debug)]
struct UnknownQuery;

impl Query for UnknownQuery {
    fn rewrite(&self, _index_searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        Ok(Some(common_terms_query()?))
    }

    fn visit(&self, _visitor: &mut dyn QueryVisitor) {}

    fn to_query_string(&self, _field: &str) -> String {
        // Java returns null
        "null".to_string()
    }

    fn query_hash(&self) -> i32 {
        // System.identityHashCode(this)
        (self as *const Self as usize) as i32
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        std::ptr::addr_eq(self as *const Self, other.as_any() as *const dyn Any)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.search.highlight.TestHighlighter$1"
    }
}

fn test_highlight_unknown_query_after_rewrite(f: &mut Th) -> Result<()> {
    sorted_two_hits(
        f,
        Arc::new(UnknownQuery),
        "Hello this is a piece of text that is <B>very</B> <B>long</B> and contains too much preamble and the meat is really here which says kennedy has been shot",
        Some("This piece of text refers to Kennedy at the beginning then has a longer piece of text that is <B>very</B>"),
    )
}

fn test_highlighting_with_default_field(f: &mut Th) -> Result<()> {
    let s1 = "I call our world Flatland, not because we call it so,";

    // Verify that a query against the default field results in text being
    // highlighted
    // regardless of the field name.

    let q = phrase(3, FIELD_NAME, &["world", "flatland"])?;

    let expected = "I call our <B>world</B> <B>Flatland</B>, not because we call it so,";
    let observed = highlight_field(f, q, "SOME_FIELD_NAME", s1)?;
    if verbose() {
        println!("Expected: \"{expected}\nObserved: \"{observed}");
    }
    Assert::assert_equals_str(
        Some("Query in the default field results in text for *ANY* field being highlighted"),
        Some(expected),
        Some(observed.as_str()),
    );

    // Verify that a query against a named field does not result in any
    // highlighting
    // when the query field name differs from the name of the field being
    // highlighted,
    // which in this example happens to be the default field name.
    let q = phrase(3, "text", &["world", "flatland"])?;

    let expected = s1;
    let observed = highlight_field(f, Arc::clone(&q), FIELD_NAME, s1)?;
    if verbose() {
        println!("Expected: \"{expected}\nObserved: \"{observed}");
    }
    Assert::assert_equals_str(
        Some(
            "Query in a named field does not result in highlighting when that field isn't in the query",
        ),
        Some(s1),
        Some(highlight_field(f, q, FIELD_NAME, s1)?.as_str()),
    );
    Ok(())
}

/// This method intended for use with `testHighlightingWithDefaultField()`
fn highlight_field(f: &Th, query: Arc<dyn Query>, field_name: &str, text: &str) -> Result<String> {
    let token_stream = f.token_stream(field_name, text)?;
    // Assuming "<B>", "</B>" used to highlight
    let formatter = SimpleHTMLFormatter::new();
    let scorer =
        QueryScorer::with_field_and_default_field(query, Some(field_name), Some(FIELD_NAME));
    let mut highlighter = Highlighter::with_formatter(Box::new(formatter), Box::new(scorer));
    highlighter.set_text_fragmenter(Box::new(SimpleFragmenter::with_fragment_size(i32::MAX)));

    let rv = highlighter.get_best_fragments_with_separator(
        token_stream,
        text,
        1,
        "(FIELD TEXT TRUNCATED)",
    )?;
    Ok(if rv.is_empty() { text.to_string() } else { rv })
}

/// How a hit loop builds each hit's token stream.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stream {
    /// `getAnyTokenStream(FIELD_NAME, docId)`.
    Any,
    /// `analyzer.tokenStream(FIELD_NAME, text)`.
    Analyzer,
}

/// The loop most tests run over `hits`: each hit's stored text, its token
/// stream, `make_fragmenter()` as the text fragmenter, and
/// `getBestFragments(tokenStream, text, maxNumFragmentsRequired, "...")`;
/// `after_each` runs after each hit.
fn highlight_hits(
    f: &Th,
    highlighter: &mut Highlighter,
    stream: Stream,
    max_num_fragments_required: i32,
    make_fragmenter: &dyn Fn() -> Box<dyn rucene_highlighter::highlight::fragmenter::Fragmenter>,
    after_each: &dyn Fn(&str),
) -> Result<()> {
    for i in 0..f.total_hits() {
        let doc_id = f.hit_doc(i);
        let text = f.stored(doc_id, FIELD_NAME)?;
        let token_stream = match stream {
            Stream::Any => f.get_any_token_stream(FIELD_NAME, doc_id)?,
            Stream::Analyzer => f.token_stream(FIELD_NAME, &text)?,
        };
        highlighter.set_text_fragmenter(make_fragmenter());
        let result = highlighter.get_best_fragments_with_separator(
            token_stream,
            &text,
            max_num_fragments_required,
            "...",
        )?;
        if verbose() {
            println!("\t{result}");
        }
        after_each(&result);
    }
    Ok(())
}

fn simple40() -> Box<dyn rucene_highlighter::highlight::fragmenter::Fragmenter> {
    Box::new(SimpleFragmenter::with_fragment_size(40))
}

fn test_simple_span_highlighter(f: &mut Th) -> Result<()> {
    f.do_searching(tq(FIELD_NAME, "kennedy"))?;

    let max_num_fragments_required = 2;

    let scorer = QueryScorer::with_field(f.query(), Some(FIELD_NAME));
    let mut highlighter = Highlighter::new(Box::new(scorer));

    highlight_hits(
        f,
        &mut highlighter,
        Stream::Analyzer,
        max_num_fragments_required,
        &simple40,
        &|_| {},
    )
    // Not sure we can assert anything here - just running to check we dont
    // throw any exceptions
}

// LUCENE-2229
fn test_simple_span_highlighter_with_stop_words_straddling_fragment_boundaries(
    f: &mut Th,
) -> Result<()> {
    f.do_searching(phrase(0, FIELD_NAME, &["all", "tokens"])?)?;

    let max_num_fragments_required = 1;

    let scorer = QueryScorer::with_field(f.query(), Some(FIELD_NAME));
    let mut highlighter = Highlighter::new(Box::new(scorer.clone()));

    Assert::assert_equals_msg(Some("Must have one hit"), 1, f.total_hits());
    highlight_hits(
        f,
        &mut highlighter,
        Stream::Analyzer,
        max_num_fragments_required,
        &|| Box::new(SimpleSpanFragmenter::with_fragment_size(scorer.clone(), 36)),
        &|result| {
            Assert::assert_true_msg(
                "Fragment must be less than 60 characters long",
                result.encode_utf16().count() < 60,
            )
        },
    )
}

// LUCENE-1752
fn test_repeating_terms_in_mult_booleans(f: &mut Th) -> Result<()> {
    let content = "x y z a b c d e f g b c g";
    let f1 = "f1";
    let f2 = "f2";

    let f1ph1 = phrase(0, f1, &["a", "b", "c", "d"])?;
    let f2ph1 = phrase(0, f2, &["a", "b", "c", "d"])?;
    let f1ph2 = phrase(0, f1, &["b", "c", "g"])?;
    let f2ph2 = phrase(0, f2, &["b", "c", "g"])?;

    let mut boolean_query = BooleanQueryBuilder::new();
    let mut left_child = BooleanQueryBuilder::new();
    left_child.add(f1ph1, Occur::SHOULD)?;
    left_child.add(f2ph1, Occur::SHOULD)?;
    boolean_query.add(Arc::new(left_child.build()), Occur::MUST)?;

    let mut right_child = BooleanQueryBuilder::new();
    right_child.add(f1ph2, Occur::SHOULD)?;
    right_child.add(f2ph2, Occur::SHOULD)?;
    boolean_query.add(Arc::new(right_child.build()), Occur::MUST)?;

    let mut scorer = QueryScorer::with_field(Arc::new(boolean_query.build()), Some(f1));
    scorer.set_expand_multi_term_query(false);

    let mut h = Highlighter::with_formatter(f.this(), Box::new(scorer));

    let analyzer = MockAnalyzer::with_automaton(&*random(), Arc::clone(&WHITESPACE), false);

    h.get_best_fragment(&analyzer, f1, content)?;

    f.assert_num_highlights(7);
    Ok(())
}

/// The block of `testSimpleQueryScorerPhraseHighlighting` repeated for each
/// phrase: search `phrase_query`, highlight every hit, expect `expected`
/// highlights.
fn phrase_highlights(f: &mut Th, phrase_query: Arc<dyn Query>, expected: i32) -> Result<()> {
    f.do_searching(phrase_query)?;

    let max_num_fragments_required = 2;
    let scorer = QueryScorer::with_field(f.query(), Some(FIELD_NAME));
    let mut highlighter = Highlighter::with_formatter(f.this(), Box::new(scorer));

    highlight_hits(
        f,
        &mut highlighter,
        Stream::Any,
        max_num_fragments_required,
        &simple40,
        &|_| {},
    )?;

    f.assert_num_highlights(expected);
    Ok(())
}

fn phrase_at(terms: &[(&str, i32)]) -> Result<Arc<dyn Query>> {
    let mut builder = PhraseQueryBuilder::new();
    for (text, position) in terms {
        builder.add_at(term(FIELD_NAME, text), *position)?;
    }
    Ok(Arc::new(builder.build()?))
}

fn test_simple_query_scorer_phrase_highlighting(f: &mut Th) -> Result<()> {
    phrase_highlights(
        f,
        phrase_at(&[("very", 0), ("long", 1), ("contains", 3)])?,
        3,
    )?;

    f.reset_nh();

    phrase_highlights(
        f,
        phrase_at(&[("piece", 1), ("text", 3), ("refers", 4), ("kennedy", 6)])?,
        4,
    )?;

    f.reset_nh();

    phrase_highlights(
        f,
        phrase_at(&[("lets", 0), ("lets", 4), ("lets", 8), ("lets", 12)])?,
        4,
    )
}

/// The common body of `testSpanRegexQuery`, `testRegexQuery` and
/// `testExternalReader`: search `f.query` (100), highlight every hit with
/// `scorer`, expect 5 highlights.
fn regex_highlights(f: &mut Th, scorer: QueryScorer) -> Result<()> {
    let max_num_fragments_required = 2;
    let mut highlighter = Highlighter::with_formatter(f.this(), Box::new(scorer));
    highlight_hits(
        f,
        &mut highlighter,
        Stream::Any,
        max_num_fragments_required,
        &simple40,
        &|_| {},
    )
}

fn ken_regexp() -> Result<Arc<RegexpQuery>> {
    Ok(Arc::new(RegexpQuery::new(term(FIELD_NAME, "ken.*"))?))
}

fn test_span_regex_query(f: &mut Th) -> Result<()> {
    let wrapper: Arc<dyn SpanQuery> = Arc::new(SpanMultiTermQueryWrapper::new(ken_regexp()?));
    f.query = Some(Arc::new(SpanOrQuery::new(vec![wrapper])?));
    f.new_searcher();
    f.hits = Some(f.searcher().search(f.query(), 100)?);
    let scorer = QueryScorer::with_field(f.query(), Some(FIELD_NAME));
    regex_highlights(f, scorer)?;
    f.assert_num_highlights(5);
    Ok(())
}

fn test_regex_query(f: &mut Th) -> Result<()> {
    f.query = Some(ken_regexp()?);
    f.new_searcher();
    f.hits = Some(f.searcher().search(f.query(), 100)?);
    let scorer = QueryScorer::with_field(f.query(), Some(FIELD_NAME));
    regex_highlights(f, scorer)?;
    f.assert_num_highlights(5);
    Ok(())
}

fn test_external_reader(f: &mut Th) -> Result<()> {
    f.query = Some(ken_regexp()?);
    f.new_searcher();
    f.hits = Some(f.searcher().search(f.query(), 100)?);
    let reader: Arc<dyn IndexReader> = f.reader();
    let scorer = QueryScorer::with_reader(f.query(), Some(Arc::clone(&reader)), Some(FIELD_NAME));
    regex_highlights(f, scorer)?;

    Assert::assert_true(reader.doc_freq(&term(FIELD_NAME, "hello"))? > 0);
    f.assert_num_highlights(5);
    Ok(())
}

fn test_dimensional_range_query(f: &mut Th) -> Result<()> {
    // doesn't currently highlight, but make sure it doesn't cause exception either
    f.query = Some(IntPoint::new_range_query(NUMERIC_FIELD_NAME, &[2], &[6])?);
    f.new_searcher();
    f.hits = Some(f.searcher().search(f.query(), 100)?);
    let max_num_fragments_required = 2;

    let scorer = QueryScorer::with_field(f.query(), Some(FIELD_NAME));
    let mut highlighter = Highlighter::with_formatter(f.this(), Box::new(scorer));

    for i in 0..f.total_hits() {
        let doc = f.searcher().stored_fields()?.document(f.hit_doc(i))?;
        let text = doc
            .get_field(NUMERIC_FIELD_NAME)
            .and_then(|field| field.numeric_value())
            .map(|v: NumericValue| v.to_string())
            .expect("NullPointerException: no numeric value");
        let token_stream = f.token_stream(FIELD_NAME, &text)?;

        highlighter.set_text_fragmenter(simple40());
        highlighter.get_best_fragments_with_separator(
            token_stream,
            &text,
            max_num_fragments_required,
            "...",
        )?;
    }
    Ok(())
}

fn test_simple_query_scorer_phrase_highlighting2(f: &mut Th) -> Result<()> {
    f.do_searching(phrase(5, FIELD_NAME, &["text", "piece", "long"])?)?;

    let max_num_fragments_required = 2;
    let scorer = QueryScorer::with_field(f.query(), Some(FIELD_NAME));
    let mut highlighter = Highlighter::with_formatter(f.this(), Box::new(scorer));
    highlight_hits(
        f,
        &mut highlighter,
        Stream::Any,
        max_num_fragments_required,
        &simple40,
        &|_| {},
    )?;

    f.assert_num_highlights(6);
    Ok(())
}

fn test_simple_query_scorer_phrase_highlighting3(f: &mut Th) -> Result<()> {
    f.do_searching(phrase(0, FIELD_NAME, &["x", "y", "z"])?)?;

    let max_num_fragments_required = 2;

    for i in 0..f.total_hits() {
        let doc_id = f.hit_doc(i);
        let text = f.stored(doc_id, FIELD_NAME)?;
        let token_stream = f.get_any_token_stream(FIELD_NAME, doc_id)?;
        let scorer = QueryScorer::with_field(f.query(), Some(FIELD_NAME));
        let mut highlighter = Highlighter::with_formatter(f.this(), Box::new(scorer));

        highlighter.set_text_fragmenter(simple40());

        let result = highlighter.get_best_fragments_with_separator(
            token_stream,
            &text,
            max_num_fragments_required,
            "...",
        )?;
        if verbose() {
            println!("\t{result}");
        }

        f.assert_num_highlights(3);
    }
    Ok(())
}

fn test_simple_span_fragmenter(f: &mut Th) -> Result<()> {
    let phrase_query = phrase_at(&[("piece", 0), ("text", 2), ("very", 5), ("long", 6)])?;
    f.do_searching(phrase_query)?;

    let max_num_fragments_required = 2;

    let scorer = QueryScorer::with_field(f.query(), Some(FIELD_NAME));
    let mut highlighter = Highlighter::with_formatter(f.this(), Box::new(scorer.clone()));

    highlight_hits(
        f,
        &mut highlighter,
        Stream::Any,
        max_num_fragments_required,
        &|| Box::new(SimpleSpanFragmenter::with_fragment_size(scorer.clone(), 5)),
        &|_| {},
    )?;

    let _phrase_query = phrase(0, FIELD_NAME, &["been", "shot"])?;

    f.do_searching(f.query())?;

    let max_num_fragments_required = 2;

    let scorer = QueryScorer::with_field(f.query(), Some(FIELD_NAME));
    let mut highlighter = Highlighter::with_formatter(f.this(), Box::new(scorer.clone()));

    highlight_hits(
        f,
        &mut highlighter,
        Stream::Analyzer,
        max_num_fragments_required,
        &|| Box::new(SimpleSpanFragmenter::with_fragment_size(scorer.clone(), 20)),
        &|_| {},
    )
}

// position sensitive query added after position insensitive query
fn test_pos_term_std_term(f: &mut Th) -> Result<()> {
    let mut boolean_query = BooleanQueryBuilder::new();
    boolean_query.add(tq(FIELD_NAME, "y"), Occur::SHOULD)?;

    let phrase_query = phrase(0, FIELD_NAME, &["x", "y", "z"])?;
    boolean_query.add(phrase_query, Occur::SHOULD)?;

    f.do_searching(Arc::new(boolean_query.build()))?;

    let max_num_fragments_required = 2;

    let scorer = QueryScorer::with_field(f.query(), Some(FIELD_NAME));
    let mut highlighter = Highlighter::with_formatter(f.this(), Box::new(scorer));
    highlight_hits(
        f,
        &mut highlighter,
        Stream::Any,
        max_num_fragments_required,
        &simple40,
        &|_| f.assert_num_highlights(4),
    )
}

fn test_query_scorer_multi_phrase_query_highlighting(f: &mut Th) -> Result<()> {
    let mut mpqb = MultiPhraseQueryBuilder::new();

    mpqb.add_terms(vec![term(FIELD_NAME, "wordx"), term(FIELD_NAME, "wordb")])?;
    mpqb.add(term(FIELD_NAME, "wordy"))?;

    f.do_searching(Arc::new(mpqb.build()))?;

    let max_num_fragments_required = 2;
    f.assert_expected_highlight_count(max_num_fragments_required, 6)
}

fn test_query_scorer_multi_phrase_query_highlighting_with_gap(f: &mut Th) -> Result<()> {
    let mut mpqb = MultiPhraseQueryBuilder::new();

    // The toString of MultiPhraseQuery doesn't work so well with these
    // out-of-order additions, but the Query itself seems to match accurately.

    mpqb.add_at(vec![term(FIELD_NAME, "wordz")], 2)?;
    mpqb.add_at(vec![term(FIELD_NAME, "wordx")], 0)?;

    f.do_searching(Arc::new(mpqb.build()))?;

    let max_num_fragments_required = 1;
    let expected_highlights = 2;

    f.assert_expected_highlight_count(max_num_fragments_required, expected_highlights)
}

/// `TestHighlightRunner.QUERY`.
const QUERY: i32 = 0;
/// `TestHighlightRunner.QUERY_TERM`.
const QUERY_TERM: i32 = 1;

/// `SynonymTokenizer.TestHighlightRunner`: the `mode`, and `frag` (a
/// `new SimpleFragmenter(20)`, made afresh for each highlighter; the
/// fragmenter restarts on every text).
struct TestHighlightRunner {
    mode: i32,
}

impl TestHighlightRunner {
    fn frag() -> Box<dyn rucene_highlighter::highlight::fragmenter::Fragmenter> {
        Box::new(SimpleFragmenter::with_fragment_size(20))
    }

    fn get_highlighter(
        &self,
        query: Arc<dyn Query>,
        field_name: Option<&str>,
        formatter: Box<dyn Formatter>,
    ) -> Highlighter {
        self.get_highlighter_expand(query, field_name, formatter, true)
    }

    fn get_highlighter_expand(
        &self,
        query: Arc<dyn Query>,
        field_name: Option<&str>,
        formatter: Box<dyn Formatter>,
        expan_multi_term: bool,
    ) -> Highlighter {
        let scorer: Box<dyn Scorer> = if self.mode == QUERY {
            let mut scorer = QueryScorer::with_field(query, field_name);
            if !expan_multi_term {
                scorer.set_expand_multi_term_query(false);
            }
            Box::new(scorer)
        } else if self.mode == QUERY_TERM {
            Box::new(QueryTermScorer::new(&*query))
        } else {
            panic!("RuntimeException: Unknown highlight mode");
        };
        Highlighter::with_formatter(formatter, scorer)
    }

    fn get_highlighter_weighted(
        &self,
        weighted_terms: &[WeightedSpanTerm],
        formatter: Box<dyn Formatter>,
    ) -> Highlighter {
        if self.mode == QUERY {
            Highlighter::with_formatter(
                formatter,
                Box::new(QueryScorer::from_weighted_span_terms(
                    weighted_terms.to_vec(),
                )),
            )
        } else if self.mode == QUERY_TERM {
            let terms: Vec<WeightedTerm> = weighted_terms.iter().map(|w| (**w).clone()).collect();
            Highlighter::with_formatter(
                formatter,
                Box::new(QueryTermScorer::from_weighted_terms(terms)),
            )
        } else {
            panic!("RuntimeException: Unknown highlight mode");
        }
    }

    /// `doStandardHighlights(analyzer, searcher, hits, query, formatter[,
    /// expandMT])` over the fixture's `analyzer`, `searcher`, `hits` and
    /// `query`, with `TestHighlighter.this` as the formatter.
    fn do_standard_highlights(&self, f: &Th) -> Result<()> {
        self.do_standard_highlights_with(f, f.query())
    }

    fn do_standard_highlights_with(&self, f: &Th, query: Arc<dyn Query>) -> Result<()> {
        let term_vectors = f.searcher().get_index_reader().term_vectors()?;
        for i in 0..f.total_hits() {
            let doc_id = f.hit_doc(i);
            let doc = f.searcher().stored_fields()?.document(doc_id)?;
            let text = doc.get(FIELD_NAME).expect("a stored value");
            let max_num_fragments_required = 2;
            let fragment_separator = "...";
            let token_stream = TokenSources::get_token_stream(
                FIELD_NAME,
                term_vectors.get(doc_id)?.as_deref(),
                &text,
                &*f.analyzer(),
                -1,
            )?;
            let scorer: Box<dyn Scorer> = if self.mode == QUERY {
                Box::new(QueryScorer::new(Arc::clone(&query)))
            } else {
                Box::new(QueryTermScorer::new(&*query))
            };
            let mut highlighter = Highlighter::with_formatter(f.this(), scorer);

            highlighter.set_text_fragmenter(Self::frag());

            let result = highlighter.get_best_fragments_with_separator(
                token_stream,
                &text,
                max_num_fragments_required,
                fragment_separator,
            )?;
            if verbose() {
                println!("\t{result}");
            }
        }
        Ok(())
    }
}

/// `helper.start()`: `run` with `QUERY`, then with `QUERY_TERM`.
fn start(f: &mut Th, run: fn(&mut Th, &TestHighlightRunner) -> Result<()>) -> Result<()> {
    let mut helper = TestHighlightRunner { mode: QUERY };
    if verbose() {
        println!("Run QueryScorer");
    }
    run(f, &helper)?;
    if verbose() {
        println!("Run QueryTermScorer");
    }
    helper.mode = QUERY_TERM;
    run(f, &helper)
}

/// `helper.run()` of a runner whose `run` sets `mode = QUERY` and does the
/// standard highlights.
fn run_query_mode(f: &Th) -> Result<()> {
    let helper = TestHighlightRunner { mode: QUERY };
    helper.do_standard_highlights(f)
}

fn test_near_span_simple_query(f: &mut Th) -> Result<()> {
    f.do_searching(as_query(span_near(
        FIELD_NAME,
        &["beginning", "kennedy"],
        3,
        false,
    )?))?;
    run_query_mode(f)?;
    f.assert_num_highlights(2);
    Ok(())
}

fn test_simple_query_term_scorer_highlighter(f: &mut Th) -> Result<()> {
    f.do_searching(tq(FIELD_NAME, "kennedy"))?;
    let mut highlighter = Highlighter::new(Box::new(QueryTermScorer::new(&*f.query())));
    highlighter.set_text_fragmenter(simple40());
    let max_num_fragments_required = 2;
    for i in 0..f.total_hits() {
        let doc_id = f.hit_doc(i);
        let text = f.stored(doc_id, FIELD_NAME)?;
        let token_stream = f.get_any_token_stream(FIELD_NAME, doc_id)?;

        let result = highlighter.get_best_fragments_with_separator(
            token_stream,
            &text,
            max_num_fragments_required,
            "...",
        )?;
        if verbose() {
            println!("\t{result}");
        }
    }
    // Not sure we can assert anything here - just running to check we dont
    // throw any exceptions
    Ok(())
}

fn test_span_highlighting(f: &mut Th) -> Result<()> {
    let query1 = as_query(span_near(FIELD_NAME, &["wordx", "wordy"], 1, false)?);
    let query2 = as_query(span_near(FIELD_NAME, &["wordy", "wordc"], 1, false)?);
    let mut bquery = BooleanQueryBuilder::new();
    bquery.add(query1, Occur::SHOULD)?;
    bquery.add(query2, Occur::SHOULD)?;
    f.do_searching(Arc::new(bquery.build()))?;
    run_query_mode(f)?;
    f.assert_num_highlights(7);
    Ok(())
}

fn test_not_span_simple_query(f: &mut Th) -> Result<()> {
    f.do_searching(Arc::new(SpanNotQuery::new(
        span_near(FIELD_NAME, &["shot", "kennedy"], 3, false)?,
        Arc::new(SpanTermQuery::new(term(FIELD_NAME, "john"))),
    )?))?;
    run_query_mode(f)?;
    f.assert_num_highlights(4);
    Ok(())
}

fn test_get_best_fragments_simple_query(f: &mut Th) -> Result<()> {
    start(f, |f, helper| {
        f.reset_nh();
        f.do_searching(tq(FIELD_NAME, "kennedy"))?;
        helper.do_standard_highlights(f)?;
        f.assert_num_highlights(4);
        Ok(())
    })
}

fn test_get_best_fragments_constant_score(f: &mut Th) -> Result<()> {
    start(f, |f, helper| {
        f.reset_nh();
        if random().next_boolean() {
            let mut bq = BooleanQueryBuilder::new();
            bq.add(
                Arc::new(ConstantScoreQuery::new(tq(FIELD_NAME, "kennedy"))),
                Occur::MUST,
            )?;
            bq.add(
                Arc::new(ConstantScoreQuery::new(tq(FIELD_NAME, "kennedy"))),
                Occur::MUST,
            )?;
            f.do_searching(Arc::new(bq.build()))?;
        } else {
            f.do_searching(Arc::new(ConstantScoreQuery::new(tq(FIELD_NAME, "kennedy"))))?;
        }
        helper.do_standard_highlights(f)?;
        f.assert_num_highlights(4);
        Ok(())
    })
}

fn test_get_fuzzy_fragments(f: &mut Th) -> Result<()> {
    start(f, |f, helper| {
        f.reset_nh();
        let fuzzy_query = FuzzyQuery::with_rewrite_method(
            term(FIELD_NAME, "kinnedy"),
            2,
            DEFAULT_PREFIX_LENGTH,
            DEFAULT_MAX_EXPANSIONS,
            DEFAULT_TRANSPOSITIONS,
            scoring_boolean_rewrite(),
        )?;
        f.do_searching(Arc::new(fuzzy_query))?;
        helper.do_standard_highlights(f)?;
        f.assert_num_highlights(4);
        Ok(())
    })
}

fn wildcard_run(
    f: &mut Th,
    helper: &TestHighlightRunner,
    pattern: &str,
    expected: i32,
) -> Result<()> {
    f.reset_nh();
    let wildcard_query = WildcardQuery::with_rewrite_method(
        term(FIELD_NAME, pattern),
        DEFAULT_DETERMINIZE_WORK_LIMIT,
        scoring_boolean_rewrite(),
    )?;
    f.do_searching(Arc::new(wildcard_query))?;
    helper.do_standard_highlights(f)?;
    f.assert_num_highlights(expected);
    Ok(())
}

fn test_get_wild_card_fragments(f: &mut Th) -> Result<()> {
    start(f, |f, helper| wildcard_run(f, helper, "k?nnedy", 4))
}

fn test_get_mid_wild_card_fragments(f: &mut Th) -> Result<()> {
    start(f, |f, helper| wildcard_run(f, helper, "k*dy", 5))
}

fn test_get_range_fragments(f: &mut Th) -> Result<()> {
    start(f, |f, helper| {
        f.reset_nh();

        // Need to explicitly set the QueryParser property to use TermRangeQuery
        // rather
        // than RangeFilters

        let query: Arc<dyn Query> = Arc::new(TermRangeQuery::with_rewrite_method(
            FIELD_NAME,
            Some(BytesRef::from_char_sequence("kannedy")),
            Some(BytesRef::from_char_sequence("kznnedy")),
            true,
            true,
            scoring_boolean_rewrite(),
        )?);
        f.query = Some(Arc::clone(&query));
        f.do_searching(query)?;
        helper.do_standard_highlights(f)?;
        f.assert_num_highlights(5);
        Ok(())
    })
}

fn test_constant_score_multi_term_query(f: &mut Th) -> Result<()> {
    f.reset_nh();

    f.query = Some(Arc::new(WildcardQuery::with_rewrite_method(
        term(FIELD_NAME, "ken*"),
        DEFAULT_DETERMINIZE_WORK_LIMIT,
        constant_score_blended_rewrite(),
    )?));
    f.new_searcher();
    // can't rewrite ConstantScore if you want to highlight it -
    // it rewrites to ConstantScoreQuery which cannot be highlighted
    // query = unReWrittenQuery.rewrite(reader);
    if verbose() {
        println!("Searching for: {}", f.query().to_query_string(FIELD_NAME));
    }

    let scorers: [&dyn Fn(&Th) -> QueryScorer; 3] = [
        &|f| QueryScorer::with_field(f.query(), Some(FIELD_NAME)),
        // try null field
        &|f| QueryScorer::with_field(f.query(), None),
        // try default field
        &|f| {
            QueryScorer::with_field_and_default_field(
                f.query(),
                Some("random_field"),
                Some(FIELD_NAME),
            )
        },
    ];
    for (round, make_scorer) in scorers.iter().enumerate() {
        f.hits = Some(f.searcher().search(f.query(), 1000)?);
        if round > 0 {
            f.reset_nh();
        }
        for i in 0..f.total_hits() {
            let doc_id = f.hit_doc(i);
            let text = f.stored(doc_id, FIELD_NAME)?;
            let token_stream = f.get_any_token_stream(FIELD_NAME, doc_id)?;
            let max_num_fragments_required = 2;
            let fragment_separator = "...";
            let scorer = make_scorer(f);

            let mut highlighter = Highlighter::with_formatter(f.this(), Box::new(scorer));

            highlighter.set_text_fragmenter(Box::new(SimpleFragmenter::with_fragment_size(20)));

            let result = highlighter.get_best_fragments_with_separator(
                token_stream,
                &text,
                max_num_fragments_required,
                fragment_separator,
            )?;
            if verbose() {
                println!("\t{result}");
            }
        }
        f.assert_num_highlights(5);
    }
    Ok(())
}

fn test_get_best_fragments_phrase(f: &mut Th) -> Result<()> {
    start(f, |f, helper| {
        f.reset_nh();
        let phrase_query = phrase(0, FIELD_NAME, &["john", "kennedy"])?;
        f.do_searching(phrase_query)?;
        helper.do_standard_highlights(f)?;
        // Currently highlights "John" and "Kennedy" separately
        f.assert_num_highlights(2);
        Ok(())
    })
}

fn test_get_best_fragments_query_scorer(f: &mut Th) -> Result<()> {
    start(f, |f, helper| {
        f.reset_nh();
        let snq = span_near("contents", &["john", "kennedy"], 1, true)?;
        f.do_searching(as_query(snq))?;
        helper.do_standard_highlights(f)?;
        // Currently highlights "John" and "Kennedy" separately
        f.assert_num_highlights(2);
        Ok(())
    })
}

fn test_off_by_one(f: &mut Th) -> Result<()> {
    start(f, |f, _helper| {
        let query = TermQuery::new(term("data", "help"));
        let mut hg = Highlighter::with_formatter(
            Box::new(SimpleHTMLFormatter::new()),
            Box::new(QueryTermScorer::new(&query)),
        );
        hg.set_text_fragmenter(Box::new(NullFragmenter::new()));

        let match_ = hg.get_best_fragment(&*f.analyzer(), "data", "help me [54-65]")?;
        Assert::assert_equals_str(None, Some("<B>help</B> me [54-65]"), match_.as_deref());
        Ok(())
    })
}

fn john_filter() -> Result<Arc<dyn Query>> {
    Ok(Arc::new(TermRangeQuery::new_string_range(
        "contents",
        Some("john"),
        Some("john"),
        true,
        true,
    )?))
}

fn test_get_best_fragments_filtered_query(f: &mut Th) -> Result<()> {
    start(f, |f, helper| {
        f.reset_nh();
        let snq = span_near("contents", &["john", "kennedy"], 1, true)?;
        let mut bq = BooleanQueryBuilder::new();
        bq.add(as_query(snq), Occur::MUST)?;
        bq.add(john_filter()?, Occur::FILTER)?;

        f.do_searching(Arc::new(bq.build()))?;
        helper.do_standard_highlights(f)?;
        // Currently highlights "John" and "Kennedy" separately
        f.assert_num_highlights(2);
        Ok(())
    })
}

fn test_get_best_fragments_filtered_phrase_query(f: &mut Th) -> Result<()> {
    start(f, |f, helper| {
        f.reset_nh();
        let pq = phrase(0, "contents", &["john", "kennedy"])?;
        let mut bq = BooleanQueryBuilder::new();
        bq.add(pq, Occur::MUST)?;
        bq.add(john_filter()?, Occur::FILTER)?;

        f.do_searching(Arc::new(bq.build()))?;
        helper.do_standard_highlights(f)?;
        // Currently highlights "John" and "Kennedy" separately
        f.assert_num_highlights(2);
        Ok(())
    })
}

fn test_get_best_fragments_multi_term(f: &mut Th) -> Result<()> {
    start(f, |f, helper| {
        f.reset_nh();
        let mut boolean_query = BooleanQueryBuilder::new();
        boolean_query.add(tq(FIELD_NAME, "john"), Occur::SHOULD)?;
        let prefix_query =
            PrefixQuery::with_rewrite_method(term(FIELD_NAME, "kenn"), scoring_boolean_rewrite())?;
        boolean_query.add(Arc::new(prefix_query), Occur::SHOULD)?;

        f.do_searching(Arc::new(boolean_query.build()))?;
        helper.do_standard_highlights(f)?;
        f.assert_num_highlights(5);
        Ok(())
    })
}

fn test_get_best_fragments_with_or(f: &mut Th) -> Result<()> {
    start(f, |f, helper| {
        f.reset_nh();

        let mut query = BooleanQueryBuilder::new();
        query.add(tq(FIELD_NAME, "jfk"), Occur::SHOULD)?;
        query.add(tq(FIELD_NAME, "kennedy"), Occur::SHOULD)?;

        f.do_searching(Arc::new(query.build()))?;
        helper.do_standard_highlights_with(f, Arc::new(query.build()))?;
        f.assert_num_highlights(5);
        Ok(())
    })
}

fn test_get_best_single_fragment(f: &mut Th) -> Result<()> {
    start(f, |f, helper| {
        f.do_searching(tq(FIELD_NAME, "kennedy"))?;
        f.reset_nh();
        for i in 0..f.total_hits() {
            let doc_id = f.hit_doc(i);
            let text = f.stored(doc_id, FIELD_NAME)?;
            let token_stream = f.get_any_token_stream(FIELD_NAME, doc_id)?;

            let mut highlighter = helper.get_highlighter(f.query(), Some(FIELD_NAME), f.this());
            highlighter.set_text_fragmenter(simple40());
            let result = highlighter.get_best_fragment_from_stream(token_stream, &text)?;
            if verbose() {
                println!("\t{result:?}");
            }
        }
        f.assert_num_highlights(4);

        f.reset_nh();
        for i in 0..f.total_hits() {
            let doc_id = f.hit_doc(i);
            let text = f.stored(doc_id, FIELD_NAME)?;
            let token_stream = f.get_any_token_stream(FIELD_NAME, doc_id)?;
            let mut highlighter = helper.get_highlighter(f.query(), Some(FIELD_NAME), f.this());
            highlighter.get_best_fragment_from_stream(token_stream, &text)?;
        }
        f.assert_num_highlights(4);

        f.reset_nh();
        for i in 0..f.total_hits() {
            let doc_id = f.hit_doc(i);
            let text = f.stored(doc_id, FIELD_NAME)?;
            let token_stream = f.get_any_token_stream(FIELD_NAME, doc_id)?;

            let mut highlighter = helper.get_highlighter(f.query(), Some(FIELD_NAME), f.this());
            highlighter.get_best_fragments_from_stream(token_stream, &text, 10)?;
        }
        f.assert_num_highlights(4);
        Ok(())
    })
}

/// The anonymous `WeightedSpanTermExtractor` of
/// `testNotRewriteMultiTermQuery`: `extract` asserts it is handed `mtq`.
#[derive(Debug)]
struct NotRewriteExtractor {
    mtq: Arc<dyn Query>,
}

impl WeightedSpanTermExtractorOverrides for NotRewriteExtractor {
    fn extract(
        &self,
        this: &mut WeightedSpanTermExtractor,
        query: &Arc<dyn Query>,
        boost: f32,
        terms: &mut PositionCheckingMap,
    ) -> Result<()> {
        Assert::assert_true_msg(
            &format!(
                "expected:<{}> but was:<{}>",
                self.mtq.to_query_string(""),
                query.to_query_string("")
            ),
            self.mtq.query_eq(&**query),
        );
        this.super_extract(query, boost, terms)
    }
}

// LUCENE-2874
fn test_not_rewrite_multi_term_query(_f: &mut Th) -> Result<()> {
    // field "bar": (not the field we ultimately want to extract)
    let mtq: Arc<dyn Query> = Arc::new(TermRangeQuery::new(
        "bar",
        Some(BytesRef::from_char_sequence("aa")),
        Some(BytesRef::from_char_sequence("zz")),
        true,
        true,
    )?);
    let mut extractor = WeightedSpanTermExtractor::new();
    extractor.set_overrides(Arc::new(NotRewriteExtractor {
        mtq: Arc::clone(&mtq),
    }));
    extractor.set_expand_multi_term_query(true);
    extractor.set_max_doc_chars_to_analyze(51200);
    extractor.get_weighted_span_terms_for_field(
        &mtq,
        3.0,
        Box::new(CannedTokenStream::new(vec![
            Token::with_offsets("aa", 0, 2),
            Token::with_offsets("bb", 2, 4),
        ])),
        Some("foo"),
    )?; // field "foo"
    Ok(())
}

fn test_field_exists_query(_f: &mut Th) -> Result<()> {
    let mut builder = BooleanQueryBuilder::new();
    builder.add(tq("field", "term"), Occur::MUST)?;
    builder.add(Arc::new(FieldExistsQuery::new("field")), Occur::MUST)?;
    let q: Arc<dyn Query> = Arc::new(builder.build());
    let mut extractor = WeightedSpanTermExtractor::new();
    extractor.get_weighted_span_terms_for_field(
        &q,
        1.0,
        Box::new(CannedTokenStream::new(vec![Token::with_offsets(
            "term", 0, 4,
        )])),
        Some("field"),
    )?;
    Ok(())
}

/// `String.trim()`: strips the code units `<= ' '` at both ends.
fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

fn test_get_best_single_fragment_with_weights(f: &mut Th) -> Result<()> {
    start(f, |f, helper| {
        let mut w_terms = vec![
            WeightedSpanTerm::new(10.0, "hello"),
            WeightedSpanTerm::new(1.0, "kennedy"),
        ];

        w_terms[0].add_position_spans(&[PositionSpan::new(0, 0)]);
        w_terms[1].add_position_spans(&[PositionSpan::new(14, 14)]);

        let mut highlighter = helper.get_highlighter_weighted(&w_terms, f.this()); // new
                                                                                   // Highlighter(new
                                                                                   // QueryTermScorer(wTerms));
        let token_stream = f.token_stream(FIELD_NAME, TEXTS[0])?;
        highlighter.set_text_fragmenter(Box::new(SimpleFragmenter::with_fragment_size(2)));

        let result = highlighter
            .get_best_fragment_from_stream(token_stream, TEXTS[0])?
            .expect("NullPointerException: no fragment");
        let result = java_trim(&result);
        Assert::assert_true_msg(
            &format!("Failed to find best section using weighted terms. Found: [{result}]"),
            "<B>Hello</B>" == result,
        );

        w_terms[1].set_weight(50.0);
        let token_stream = f.token_stream(FIELD_NAME, TEXTS[0])?;
        let mut highlighter = helper.get_highlighter_weighted(&w_terms, f.this());
        highlighter.set_text_fragmenter(Box::new(SimpleFragmenter::with_fragment_size(2)));

        let result = highlighter
            .get_best_fragment_from_stream(token_stream, TEXTS[0])?
            .expect("NullPointerException: no fragment");
        let result = java_trim(&result);
        Assert::assert_true_msg(
            &format!("Failed to find best section using weighted terms. Found: {result}"),
            "<B>kennedy</B>" == result,
        );
        Ok(())
    })
}

// tests a "complex" analyzer that produces multiple
// overlapping tokens
fn test_overlap_analyzer(f: &mut Th) -> Result<()> {
    start(f, |f, helper| {
        let mut synonyms = HashMap::new();
        synonyms.insert("football".to_string(), "soccer,footie".to_string());
        let analyzer = synonym_analyzer(synonyms);

        let s = "football-soccer in the euro 2004 footie competition";

        let mut query = BooleanQueryBuilder::new();
        query.add(tq("bookid", "football"), Occur::SHOULD)?;
        query.add(tq("bookid", "soccer"), Occur::SHOULD)?;
        query.add(tq("bookid", "footie"), Occur::SHOULD)?;

        let mut highlighter = helper.get_highlighter(Arc::new(query.build()), None, f.this());

        // Get 3 best fragments and separate with a "..."
        let token_stream = Box::new(SharedTokenStream::new(
            analyzer.token_stream_from_str_nullable_field(None, s)?,
        ));

        let result = highlighter.get_best_fragments_with_separator(token_stream, s, 3, "...")?;
        let expected_result =
            "<B>football</B>-<B>soccer</B> in the euro 2004 <B>footie</B> competition";
        Assert::assert_true_msg(
            &format!(
                "overlapping analyzer should handle highlights OK, expected:{expected_result} actual:{result}"
            ),
            expected_result == result,
        );
        analyzer.close();
        Ok(())
    })
}

fn test_get_simple_highlight(f: &mut Th) -> Result<()> {
    start(f, |f, helper| {
        f.reset_nh();

        f.do_searching(tq(FIELD_NAME, "kennedy"))?;
        // new Highlighter(TestHighlighter.this, new QueryTermScorer(query));

        for i in 0..f.total_hits() {
            let text = f.stored(f.hit_doc(i), FIELD_NAME)?;
            let token_stream = f.token_stream(FIELD_NAME, &text)?;
            let mut highlighter = helper.get_highlighter(f.query(), Some(FIELD_NAME), f.this());
            let result = highlighter.get_best_fragment_from_stream(token_stream, &text)?;
            if verbose() {
                println!("\t{result:?}");
            }
        }
        f.assert_num_highlights(4);
        Ok(())
    })
}

fn test_get_text_fragments(f: &mut Th) -> Result<()> {
    start(f, |f, helper| {
        f.do_searching(tq(FIELD_NAME, "kennedy"))?;

        for i in 0..f.total_hits() {
            let doc_id = f.hit_doc(i);
            let text = f.stored(doc_id, FIELD_NAME)?;
            let token_stream = f.get_any_token_stream(FIELD_NAME, doc_id)?;

            let mut highlighter = helper.get_highlighter(f.query(), Some(FIELD_NAME), f.this()); // new Highlighter(this, new
                                                                                                 // QueryTermScorer(query));
            highlighter.set_text_fragmenter(Box::new(SimpleFragmenter::with_fragment_size(20)));
            let string_results =
                highlighter.get_best_fragments_from_stream(token_stream, &text, 10)?;

            let token_stream = f.token_stream(FIELD_NAME, &text)?;
            let fragment_results: Vec<TextFragment> =
                highlighter.get_best_text_fragments(token_stream, &text, true, 10)?;

            Assert::assert_true_msg(
                &format!(
                    "Failed to find correct number of text Fragments: {} vs {}",
                    fragment_results.len(),
                    string_results.len()
                ),
                fragment_results.len() == string_results.len(),
            );
            for j in 0..string_results.len() {
                if verbose() {
                    println!("{}", fragment_results[j]);
                }
                Assert::assert_true_msg(
                    &format!(
                        "Failed to find same text Fragments: {} found",
                        fragment_results[j]
                    ),
                    fragment_results[j].to_string() == string_results[j],
                );
            }
        }
        Ok(())
    })
}

/// `analyzer.getReuseStrategy().setReusableComponents(analyzer, field,
/// null)`: the per-field strategy of `MockAnalyzer` forgets the components
/// of `field`.
fn forget_reusable_components(analyzer: &MockAnalyzer, field: &str) {
    type PerField =
        HashMap<Option<String>, Arc<AtomicRefCell<rucene::analysis::TokenStreamComponents>>>;
    let stored = analyzer.analyzer_state().stored_value().get();
    let mut borrow = stored.borrow_mut();
    if let Some(map) = borrow
        .as_mut()
        .and_then(|any| any.downcast_mut::<PerField>())
    {
        map.remove(&Some(field.to_string()));
    }
}

fn test_max_size_highlight(f: &mut Th) -> Result<()> {
    // we disable MockTokenizer checks because we will forcefully limit the
    // tokenstream and call end() before incrementToken() returns false.
    // But we first need to clear the re-used tokenstream components that have enableChecks
    // enabled.
    forget_reusable_components(&f.analyzer(), FIELD_NAME);
    f.analyzer().set_enable_checks(false);
    start(f, |f, helper| {
        f.reset_nh();
        f.do_searching(tq(FIELD_NAME, "meat"))?;
        let token_stream = f.token_stream(FIELD_NAME, TEXTS[0])?;
        let mut highlighter = helper.get_highlighter(f.query(), Some(FIELD_NAME), f.this()); // new Highlighter(this, new
                                                                                             // QueryTermScorer(query));
        highlighter.set_max_doc_chars_to_analyze(30);

        highlighter.get_best_fragment_from_stream(token_stream, TEXTS[0])?;
        Assert::assert_true_msg(
            &format!(
                "Setting MaxDocBytesToAnalyze should have prevented us from finding matches for this record: {} found",
                f.nh()
            ),
            f.nh() == 0,
        );
        Ok(())
    })
}

fn test_max_size_highlight_truncates(f: &mut Th) -> Result<()> {
    start(f, |_f, helper| {
        let good_word = "goodtoken";
        let stop_words = Arc::new(CharacterRunAutomaton::new(Automata::make_string(
            "stoppedtoken",
        ))?);
        // we disable MockTokenizer checks because we will forcefully limit the
        // tokenstream and call end() before incrementToken() returns false.
        let analyzer = MockAnalyzer::with_filter(&*random(), Arc::clone(&SIMPLE), true, stop_words);
        analyzer.set_enable_checks(false);
        let query: Arc<dyn Query> = tq("data", good_word);

        let mut sb = String::new();
        sb.push_str(good_word);
        for _ in 0..10000 {
            sb.push(' ');
            // only one stopword
            sb.push_str("stoppedtoken");
        }
        let fm = SimpleHTMLFormatter::new();
        let mut hg = helper.get_highlighter(query, Some("data"), Box::new(fm)); // new Highlighter(fm,
                                                                                // new
                                                                                // QueryTermScorer(query));
        hg.set_text_fragmenter(Box::new(NullFragmenter::new()));
        hg.set_max_doc_chars_to_analyze(100);
        let match_ = hg
            .get_best_fragment(&analyzer, "data", &sb)?
            .expect("NullPointerException: no fragment");
        Assert::assert_true_msg(
            "Matched text should be no more than 100 chars in length ",
            (match_.encode_utf16().count() as i32) < hg.get_max_doc_chars_to_analyze(),
        );

        // add another tokenized word to the overrall length - but set way
        // beyond
        // the length of text under consideration (after a large slug of stop
        // words
        // + whitespace)
        sb.push(' ');
        sb.push_str(good_word);
        let match_ = hg
            .get_best_fragment(&analyzer, "data", &sb)?
            .expect("NullPointerException: no fragment");
        Assert::assert_true_msg(
            "Matched text should be no more than 100 chars in length ",
            (match_.encode_utf16().count() as i32) < hg.get_max_doc_chars_to_analyze(),
        );
        Ok(())
    })
}

fn test_max_size_end_highlight(f: &mut Th) -> Result<()> {
    start(f, |_f, helper| {
        let stop_words = Arc::new(CharacterRunAutomaton::new(
            RegExp::new("i[nt]")?.to_automaton()?,
        )?);
        let query = tq("text", "searchterm");

        let text = "this is a text with searchterm in it";
        let fm = SimpleHTMLFormatter::new();
        let mut hg = helper.get_highlighter(query, Some("text"), Box::new(fm));
        hg.set_text_fragmenter(Box::new(NullFragmenter::new()));
        hg.set_max_doc_chars_to_analyze(36);
        let match_ = hg
            .get_best_fragment(
                &MockAnalyzer::with_filter(&*random(), Arc::clone(&SIMPLE), true, stop_words),
                "text",
                text,
            )?
            .expect("NullPointerException: no fragment");
        Assert::assert_true_msg(
            "Matched text should contain remainder of text after highlighted query ",
            match_.ends_with("in it"),
        );
        Ok(())
    })
}

fn test_highlighter_with_phrase_query(_f: &mut Th) -> Result<()> {
    let field_name = "substring";

    let query: Arc<dyn Query> = Arc::new(PhraseQuery::with_slop_bytes(
        0,
        field_name,
        &[BytesRef::from_char_sequence("uchu")],
    )?);

    assert_highlighting(
        query,
        Box::new(SimpleHTMLFormatter::with_tags("<b>", "</b>")),
        "Buchung",
        "B<b>uchu</b>ng",
        field_name,
    )
}

fn test_highlighter_with_multi_phrase_query(_f: &mut Th) -> Result<()> {
    let field_name = "substring";

    let mut builder = MultiPhraseQueryBuilder::new();
    builder.add(term(field_name, "uchu"))?;
    let mpq: Arc<dyn Query> = Arc::new(builder.build());

    assert_highlighting(
        mpq,
        Box::new(SimpleHTMLFormatter::with_tags("<b>", "</b>")),
        "Buchung",
        "B<b>uchu</b>ng",
        field_name,
    )
}

fn assert_highlighting(
    query: Arc<dyn Query>,
    formatter: Box<dyn Formatter>,
    text: &str,
    expected: &str,
    field_name: &str,
) -> Result<()> {
    let analyzer = FnAnalyzer::new(
        "org.apache.lucene.search.highlight.TestHighlighter$34",
        |_| {
            Ok(tokenizer_components(Arc::new(AtomicRefCell::new(
                new_ngram_tokenizer(4, 4)?,
            ))))
        },
    );

    let fragment_scorer = QueryScorer::with_field(query, Some(field_name));

    let mut highlighter = Highlighter::with_formatter(formatter, Box::new(fragment_scorer));
    highlighter.set_text_fragmenter(Box::new(SimpleFragmenter::with_fragment_size(100)));
    let fragment = highlighter.get_best_fragment(&analyzer, field_name, text)?;

    Assert::assert_equals_str(None, Some(expected), fragment.as_deref());
    Ok(())
}

fn test_un_rewritten_query(f: &mut Th) -> Result<()> {
    start(f, |f, helper| {
        f.reset_nh();
        // test to show how rewritten query can still be used
        f.new_searcher();

        let mut query = BooleanQueryBuilder::new();
        query.add(
            Arc::new(WildcardQuery::new(term(FIELD_NAME, "jf?"))?),
            Occur::SHOULD,
        )?;
        query.add(
            Arc::new(WildcardQuery::new(term(FIELD_NAME, "kenned*"))?),
            Occur::SHOULD,
        )?;

        if verbose() {
            println!("Searching with primitive query");
        }
        // forget to set this and...
        // query=query.rewrite(reader);
        let hits = f.searcher().search(Arc::new(query.build()), 1000)?;

        // create an instance of the highlighter with the tags used to surround
        // highlighted text
        // QueryHighlightExtractor highlighter = new
        // QueryHighlightExtractor(this,
        // query, new StandardAnalyzer(TEST_VERSION));

        let max_num_fragments_required = 3;

        for i in 0..hits.total_hits.value() {
            let doc_id = hits.score_docs[i as usize].doc;
            let text = f.stored(doc_id, FIELD_NAME)?;
            let token_stream = f.get_any_token_stream(FIELD_NAME, doc_id)?;
            let mut highlighter = helper.get_highlighter_expand(
                Arc::new(query.build()),
                Some(FIELD_NAME),
                f.this(),
                false,
            );

            highlighter.set_text_fragmenter(simple40());

            let highlighted_text = highlighter.get_best_fragments_with_separator(
                token_stream,
                &text,
                max_num_fragments_required,
                "...",
            )?;

            if verbose() {
                println!("{highlighted_text}");
            }
        }
        // We expect to have zero highlights if the query is multi-terms and is
        // not
        // rewritten!
        f.assert_num_highlights(0);
        Ok(())
    })
}

fn test_no_fragments(f: &mut Th) -> Result<()> {
    start(f, |f, helper| {
        f.do_searching(tq(FIELD_NAME, "aninvalidquerywhichshouldyieldnoresults"))?;

        for text in TEXTS {
            let token_stream = f.token_stream(FIELD_NAME, text)?;
            let mut highlighter = helper.get_highlighter(f.query(), Some(FIELD_NAME), f.this());
            let result = highlighter.get_best_fragment_from_stream(token_stream, text)?;
            Assert::assert_true_msg(
                "The highlight result should be null for text with no query terms",
                result.is_none(),
            );
        }
        Ok(())
    })
}

/// The anonymous `Scorer` of `testEncoding`: every token scores 0, every
/// fragment 1.
#[derive(Debug)]
struct ConstantFragmentScorer;

impl Scorer for ConstantFragmentScorer {
    fn start_fragment(&mut self, _new_fragment: &TextFragment) {}

    fn get_token_score(&mut self) -> Result<f32> {
        Ok(0.0)
    }

    fn get_fragment_score(&self) -> f32 {
        1.0
    }

    fn init(&mut self, token_stream: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        // Java returns null: the stream is read as it is
        Ok(token_stream)
    }
}

/// The parts of the DOM `testEncoding` reads: the first child (a text node)
/// of the first `h2` of the first `body`.
#[derive(Default)]
struct FirstH2Text {
    in_body: bool,
    in_h2: bool,
    done: bool,
    child_seen: bool,
    text: Vec<u16>,
}

impl DefaultHandler for FirstH2Text {
    fn start_element(
        &mut self,
        _uri: &str,
        _local: &str,
        raw: &str,
        _attrs: &Attributes,
    ) -> Result<()> {
        if self.in_h2 {
            self.child_seen = true;
        } else if raw == "body" {
            self.in_body = true;
        } else if raw == "h2" && self.in_body && !self.done {
            self.in_h2 = true;
        }
        Ok(())
    }

    fn end_element(&mut self, _uri: &str, _local: &str, raw: &str) -> Result<()> {
        if raw == "h2" && self.in_h2 {
            self.in_h2 = false;
            self.done = true;
        }
        Ok(())
    }

    fn characters(&mut self, ch: &[u16], start: usize, length: usize) -> Result<()> {
        if self.in_h2 && !self.child_seen {
            self.text.extend_from_slice(&ch[start..start + length]);
        }
        Ok(())
    }
}

/// Demonstrates creation of an XHTML compliant doc using new encoding
/// facilities.
fn test_encoding(f: &mut Th) -> Result<()> {
    let raw_doc_content = "\"Smith & sons' prices < 3 and >4\" claims article";
    // run the highlighter on the raw content (scorer does not score any tokens
    // for
    // highlighting but scores a single fragment for selection
    let mut highlighter = Highlighter::with_formatter_and_encoder(
        f.this(),
        Box::new(SimpleHTMLEncoder::new()),
        Box::new(ConstantFragmentScorer),
    );
    highlighter.set_text_fragmenter(Box::new(SimpleFragmenter::with_fragment_size(2000)));
    let token_stream = f.token_stream(FIELD_NAME, raw_doc_content)?;

    let encoded_snippet =
        highlighter.get_best_fragments_with_separator(token_stream, raw_doc_content, 1, "")?;
    // An ugly bit of XML creation:
    let xhtml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <html xmlns=\"http://www.w3.org/1999/xhtml\" xml:lang=\"en\" lang=\"en\">\n\
         <head>\n\
         <title>My Test HTML Document</title>\n\
         </head>\n\
         <body>\n\
         <h2>{encoded_snippet}</h2>\n\
         </body>\n\
         </html>"
    );
    // now an ugly built of XML parsing to test the snippet is encoded OK
    let mut handler = FirstH2Text::default();
    XmlReader::new().parse(
        InputSource::from_byte_stream(Box::new(std::io::Cursor::new(xhtml.into_bytes()))),
        &mut handler,
    )?;
    let decoded_snippet = String::from_utf16_lossy(&handler.text);
    Assert::assert_equals_str(
        Some("XHTML Encoding should have worked:"),
        Some(raw_doc_content),
        Some(decoded_snippet.as_str()),
    );
    Ok(())
}

fn test_field_specific_highlighting(f: &mut Th) -> Result<()> {
    start(f, |f, helper| {
        let doc_main_text = "fred is one of the people";

        let mut query = BooleanQueryBuilder::new();
        query.add(tq(FIELD_NAME, "fred"), Occur::SHOULD)?;
        query.add(tq("category", "people"), Occur::SHOULD)?;
        let query: Arc<dyn Query> = Arc::new(query.build());

        // highlighting respects fieldnames used in query

        let field_specific_scorer: Box<dyn Scorer> = if helper.mode == QUERY {
            Box::new(QueryScorer::with_field(
                Arc::clone(&query),
                Some(FIELD_NAME),
            ))
        } else {
            Box::new(QueryTermScorer::with_field(&*query, "contents"))
        };
        let mut field_specific_highlighter = Highlighter::with_formatter(
            Box::new(SimpleHTMLFormatter::new()),
            field_specific_scorer,
        );
        field_specific_highlighter.set_text_fragmenter(Box::new(NullFragmenter::new()));
        let result = field_specific_highlighter.get_best_fragment(
            &*f.analyzer(),
            FIELD_NAME,
            doc_main_text,
        )?;
        Assert::assert_equals_str(
            Some("Should match"),
            result.as_deref(),
            Some("<B>fred</B> is one of the people"),
        );

        // highlighting does not respect fieldnames used in query
        let field_in_specific_scorer: Box<dyn Scorer> = if helper.mode == QUERY {
            Box::new(QueryScorer::with_field(Arc::clone(&query), None))
        } else {
            Box::new(QueryTermScorer::new(&*query))
        };

        let mut field_in_specific_highlighter = Highlighter::with_formatter(
            Box::new(SimpleHTMLFormatter::new()),
            field_in_specific_scorer,
        );
        field_in_specific_highlighter.set_text_fragmenter(Box::new(NullFragmenter::new()));
        let result = field_in_specific_highlighter.get_best_fragment(
            &*f.analyzer(),
            FIELD_NAME,
            doc_main_text,
        )?;
        Assert::assert_equals_str(
            Some("Should match"),
            result.as_deref(),
            Some("<B>fred</B> is one of the <B>people</B>"),
        );

        f.reader().close()?;
        Ok(())
    })
}

/// `getTS2()`.
fn get_ts2() -> Box<dyn TokenStream> {
    // String s = "Hi-Speed10 foo";
    Box::new(FixedTokenStream::new(&[
        ("hi", 0, 2, 1),
        ("hispeed", 0, 8, 1),
        ("speed", 3, 8, 0),
        ("10", 8, 10, 1),
        ("foo", 11, 14, 1),
    ]))
}

/// `getTS2a()`: same token-stream as above, but the bigger token comes first
/// this time.
fn get_ts2a() -> Box<dyn TokenStream> {
    // String s = "Hi-Speed10 foo";
    Box::new(FixedTokenStream::new(&[
        ("hispeed", 0, 8, 1),
        ("hi", 0, 2, 0),
        ("speed", 3, 8, 1),
        ("10", 8, 10, 1),
        ("foo", 11, 14, 1),
    ]))
}

fn test_overlap_analyzer2(f: &mut Th) -> Result<()> {
    start(f, |f, helper| {
        let s = "Hi-Speed10 foo";

        let mut boolean_query = BooleanQueryBuilder::new();
        boolean_query.add(tq("text", "hi"), Occur::SHOULD)?;
        boolean_query.add(tq("text", "speed"), Occur::SHOULD)?;

        let cases: [(Arc<dyn Query>, &str); 6] = [
            (tq("text", "foo"), "Hi-Speed10 <B>foo</B>"),
            (tq("text", "10"), "Hi-Speed<B>10</B> foo"),
            (tq("text", "hi"), "<B>Hi</B>-Speed10 foo"),
            (tq("text", "speed"), "Hi-<B>Speed</B>10 foo"),
            (tq("text", "hispeed"), "<B>Hi-Speed</B>10 foo"),
            (Arc::new(boolean_query.build()), "<B>Hi-Speed</B>10 foo"),
        ];
        // ///////////////// same tests, just put the bigger overlapping token
        // first
        for ts in [get_ts2 as fn() -> Box<dyn TokenStream>, get_ts2a] {
            for (query, expected) in &cases {
                let mut highlighter =
                    helper.get_highlighter(Arc::clone(query), Some("text"), f.this());
                let result = highlighter.get_best_fragments_with_separator(ts(), s, 3, "...")?;
                Assert::assert_equals_str(None, Some(expected), Some(result.as_str()));
            }
        }
        Ok(())
    })
}

fn test_weighted_terms_with_deletes(f: &mut Th) -> Result<()> {
    make_index(f)?;
    delete_document(f)?;
    search_index(f)
}

fn dir1(f: &Th) -> Arc<dyn rucene::store::Directory> {
    f.dir1.as_ref().expect("setUp ran").as_directory()
}

fn whitespace_config() -> IndexWriterConfig {
    IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::with_automaton(
        &*random(),
        Arc::clone(&WHITESPACE),
        false,
    ))))
}

fn make_index(f: &Th) -> Result<()> {
    let writer = IndexWriter::new(dir1(f), whitespace_config())?;
    writer.add_document(&f.doc("t_text1", "random words for highlighting tests del")?)?;
    writer.add_document(&f.doc("t_text1", "more random words for second field del")?)?;
    writer.add_document(&f.doc("t_text1", "random words for highlighting tests del")?)?;
    writer.add_document(&f.doc("t_text1", "more random words for second field")?)?;
    writer.force_merge(1)?;
    writer.close()
}

fn delete_document(f: &Th) -> Result<()> {
    let config = whitespace_config();
    config.set_open_mode(OpenMode::APPEND)?;
    let writer = IndexWriter::new(dir1(f), config)?;
    writer.delete_documents_by_terms(vec![term("t_text1", "del")])?;
    // To see negative idf, keep comment the following line
    // writer.forceMerge(1);
    writer.close()
}

fn search_index(f: &Th) -> Result<()> {
    let query = tq("t_text1", "random");
    let reader = directory_reader::open(dir1(f))?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    // This scorer can return negative idf -> null fragment
    let scorer = QueryTermScorer::with_reader(&*query, &**searcher.get_index_reader(), "t_text1")?;
    // This scorer doesn't use idf (patch version)
    // Scorer scorer = new QueryTermScorer( query, "t_text1" );
    let mut h = Highlighter::new(Box::new(scorer));

    let hits = searcher.search(query, 10)?;
    let a = Arc::clone(f.a.as_ref().expect("setUp ran"));
    for i in 0..hits.total_hits.value() {
        let doc = searcher
            .stored_fields()?
            .document(hits.score_docs[i as usize].doc)?;
        let result = h.get_best_fragment(&*a, "t_text1", &doc.get("t_text1").expect("stored"))?;
        if verbose() {
            println!("result:{result:?}");
        }
        Assert::assert_equals_str(
            None,
            Some("more <B>random</B> words for second field"),
            result.as_deref(),
        );
    }
    reader.close()
}

/// We can highlight based on payloads. It's supported both via term vectors
/// and MemoryIndex since Lucene 5.
fn test_payload_query(f: &mut Th) -> Result<()> {
    let text = "random words and words"; // "words" at positions 1 & 4

    let analyzer: Arc<dyn Analyzer> = Arc::new(MockPayloadAnalyzer::new()); // sets payload to "pos: X" (where X is position #)
    {
        let writer = IndexWriter::new(
            dir1(f),
            IndexWriterConfig::with_analyzer(Some(Arc::clone(&analyzer))),
        )?;
        let result = attempt(|| {
            writer.delete_all()?;
            let mut doc = Document::new();

            doc.add(Field::new(
                FIELD_NAME,
                text.to_string(),
                Arc::clone(f.field_type.as_ref().expect("setUp ran")),
            )?);
            writer.add_document(&doc)?;
            writer.commit()
        });
        writer.close()?;
        result?;
    }
    let reader = directory_reader::open(dir1(f))?;
    let result = attempt(|| {
        let span_term: Arc<dyn SpanQuery> = Arc::new(SpanTermQuery::new(term(FIELD_NAME, "words")));
        let query: Arc<dyn Query> = Arc::new(SpanPayloadCheckQuery::new(
            span_term,
            vec![Some(BytesRef::from_char_sequence("pos: 1"))],
        )); // just match the first "word" occurrence
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
        let mut scorer = QueryScorer::with_reader(
            Arc::clone(&query),
            Some(Arc::clone(searcher.get_index_reader())),
            Some(FIELD_NAME),
        );
        scorer.set_use_payloads(true);
        let mut h = Highlighter::new(Box::new(scorer));

        let hits = searcher.search(query, 10)?;
        Assert::assert_equals(1, hits.score_docs.len());
        #[allow(deprecated)]
        let mut stream = TokenSources::get_any_token_stream(
            &**searcher.get_index_reader(),
            0,
            FIELD_NAME,
            &*analyzer,
        )?;
        if random().next_boolean() {
            stream = Box::new(CachingTokenFilter::new(stream)); // conceals detection of TokenStreamFromTermVector
        }
        let result = h.get_best_fragment_from_stream(stream, text)?;
        Assert::assert_equals_str(
            None,
            Some("random <B>words</B> and words"),
            result.as_deref(),
        ); // only highlight first "word"
        Ok(())
    });
    reader.close()?;
    result
}

fn test_sorted_numeric_doc_values_range_query(_f: &mut Th) -> Result<()> {
    let mut builder = BooleanQueryBuilder::new();
    builder.add(tq("text", "compensation"), Occur::MUST)?;
    builder.add(
        SortedNumericDocValuesField::new_slow_range_query("date_of_review", 20190210, 20260210),
        Occur::FILTER,
    )?;
    let query: Arc<dyn Query> = Arc::new(builder.build());
    let mut extractor = WeightedSpanTermExtractor::new();
    let terms = extractor.get_weighted_span_terms_for_field(
        &query,
        1.0,
        Box::new(CannedTokenStream::new(vec![Token::with_offsets(
            "compensation",
            0,
            12,
        )])),
        Some("text"),
    )?;
    Assert::assert_true_msg(
        "Term 'compensation' should be extracted",
        terms.contains_key("compensation"),
    );
    Ok(())
}

fn set_up(f: &mut Th) -> Result<()> {
    // Not many use this setup:
    f.a = Some(Arc::new(MockAnalyzer::with_automaton(
        &*random(),
        Arc::clone(&WHITESPACE),
        false,
    )));
    f.dir1 = Some(LuceneTestCase::new_directory());

    // Most tests use this setup:
    let analyzer = Arc::new(MockAnalyzer::with_filter(
        &*random(),
        Arc::clone(&SIMPLE),
        true,
        Arc::clone(&ENGLISH_STOPSET),
    ));
    f.analyzer = Some(Arc::clone(&analyzer));
    let dir2 = LuceneTestCase::new_directory();
    let field_type_tv = {
        let field_type = FieldType::new_from(TextField::type_stored());
        field_type.set_store_term_vectors(true)?;
        field_type.set_store_term_vector_positions(true)?;
        field_type.set_store_term_vector_payloads(true)?;
        field_type.set_store_term_vector_offsets(true)?;
        field_type.freeze();
        field_type
    };
    f.field_type = Some(Arc::new(if random().next_boolean() {
        field_type_tv
    } else {
        FieldType::new_from(TextField::type_stored())
    }));
    let config =
        LuceneTestCase::new_index_writer_config_analyzer(Some(analyzer as Arc<dyn Analyzer>));
    config.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let writer = IndexWriter::new(dir2.as_directory(), config)?;

    for text in TEXTS {
        writer.add_document(&f.doc(FIELD_NAME, text)?)?;
    }

    // a few tests need other docs...:
    for value in [1, 3, 5, 7] {
        let mut doc = Document::new();
        doc.add(IntPoint::new(NUMERIC_FIELD_NAME, &[value])?);
        doc.add(StoredField::new_number(
            NUMERIC_FIELD_NAME,
            NumericValue::Int(value),
        )?);
        writer.add_document(&doc)?;
    }

    let child_doc = f.doc(FIELD_NAME, "child document")?;
    let parent_doc = f.doc(FIELD_NAME, "parent document")?;
    writer.add_documents(&[child_doc, parent_doc])?;

    writer.force_merge(1)?;
    writer.close()?;
    f.reader = Some(directory_reader::open(dir2.as_directory())?);
    f.dir2 = Some(dir2);

    // Misc:
    f.reset_nh();
    Ok(())
}

fn tear_down(f: &mut Th) -> Result<()> {
    f.searcher = None;
    if let Some(reader) = f.reader.take() {
        reader.close()?;
    }
    if let Some(dir) = f.dir1.take() {
        dir.close()?;
    }
    if let Some(dir) = f.dir2.take() {
        dir.close()?;
    }
    Ok(())
}

// ===================================================================
// ========== BEGIN TEST SUPPORTING CLASSES
// ========== THESE LOOK LIKE, WITH SOME MORE EFFORT THESE COULD BE
// ========== MADE MORE GENERALLY USEFUL.
// TODO - make synonyms all interchangeable with each other and produce
// a version that does hyponyms - the "is a specialised type of ...."
// so that car = audi, bmw and volkswagen but bmw != audi so different
// behaviour to synonyms
// ===================================================================

/// `new SynonymAnalyzer(synonyms)`.
fn synonym_analyzer(synonyms: HashMap<String, String>) -> FnAnalyzer {
    let synonyms = Arc::new(synonyms);
    FnAnalyzer::new(
        "org.apache.lucene.search.highlight.SynonymAnalyzer",
        move |_| {
            let mut stream = MockTokenizer::new(Arc::clone(&SIMPLE), true)?;
            Att::add(stream.attribute_source_mut(), char_term_id())?;
            Att::add(stream.attribute_source_mut(), pos_inc_id())?;
            Att::add(stream.attribute_source_mut(), offset_id())?;
            let stream = Arc::new(AtomicRefCell::new(stream));
            let synonym_tokenizer = SynonymTokenizer::new(shared(&stream), Arc::clone(&synonyms))?;
            Ok(components(
                stream,
                sink(BoxedStream(Box::new(synonym_tokenizer))),
            ))
        },
    )
}

/// Expands a token stream with synonyms (TODO - make the synonyms analyzed
/// by choice of analyzer)
///
/// Equivalent to `SynonymTokenizer`; `st` (a `StringTokenizer` over the
/// expansions, separated by `,`) is the queue of its remaining tokens.
#[derive(Debug)]
struct SynonymTokenizer {
    source: AttributeSource,
    real_stream: Box<dyn TokenStream>,
    /// `currentRealToken`: its offsets.
    current_real_token: Option<(i32, i32)>,
    synonyms: Arc<HashMap<String, String>>,
    st: Option<VecDeque<String>>,
    real_term_att: Att,
    real_pos_incr_att: Att,
    real_offset_att: Att,
    term_att: Att,
    pos_incr_att: Att,
    offset_att: Att,
}

impl SynonymTokenizer {
    fn new(
        mut real_stream: Box<dyn TokenStream>,
        synonyms: Arc<HashMap<String, String>>,
    ) -> Result<Self> {
        let real_term_att = Att::add(real_stream.attribute_source_mut(), char_term_id())?;
        let real_pos_incr_att = Att::add(real_stream.attribute_source_mut(), pos_inc_id())?;
        let real_offset_att = Att::add(real_stream.attribute_source_mut(), offset_id())?;

        let mut source = AttributeSource::new_with_factory(default_token_attribute_factory());
        let term_att = Att::add(&mut source, char_term_id())?;
        let pos_incr_att = Att::add(&mut source, pos_inc_id())?;
        let offset_att = Att::add(&mut source, offset_id())?;
        Ok(Self {
            source,
            real_stream,
            current_real_token: None,
            synonyms,
            st: None,
            real_term_att,
            real_pos_incr_att,
            real_offset_att,
            term_att,
            pos_incr_att,
            offset_att,
        })
    }
}

/// `new StringTokenizer(s, delimiters)`'s tokens: the maximal runs of
/// non-delimiter characters.
fn string_tokenizer(s: &str, delimiters: &str) -> VecDeque<String> {
    s.split(|c| delimiters.contains(c))
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

impl TokenStream for SynonymTokenizer {
    fn increment_token(&mut self) -> Result<bool> {
        if self.current_real_token.is_none() {
            let next = self.real_stream.increment_token()?;
            if !next {
                return Ok(false);
            }
            // Token nextRealToken = new Token(, offsetAtt.startOffset(), offsetAtt.endOffset());
            self.source.clear_attributes();
            let real_term = self.real_term_att.term_string();
            self.term_att.set_term(&real_term);
            let (start, end) = {
                let offset = self.real_offset_att.offset();
                (offset.start_offset(), offset.end_offset())
            };
            self.offset_att.offset_mut().set_offset(start, end);
            let pos_inc = self.real_pos_incr_att.pos_inc().get_position_increment();
            self.pos_incr_att
                .pos_inc_mut()
                .set_position_increment(pos_inc);

            let Some(expansions) = self.synonyms.get(&real_term) else {
                return Ok(true);
            };
            let st = string_tokenizer(expansions, ",");
            if !st.is_empty() {
                self.current_real_token = Some((start, end));
            }
            self.st = Some(st);
            Ok(true)
        } else {
            let st = self.st.as_mut().expect("INVARIANT: expansions pending");
            let tok = st
                .pop_front()
                .expect("NoSuchElementException: no more tokens");
            self.source.clear_attributes();
            self.term_att.set_term(&tok);
            let (start, end) = self.current_real_token.expect("INVARIANT: a real token");
            self.offset_att.offset_mut().set_offset(start, end);
            self.pos_incr_att.pos_inc_mut().set_position_increment(0);
            if st.is_empty() {
                self.current_real_token = None;
                self.st = None;
            }
            Ok(true)
        }
    }

    fn reset(&mut self) -> Result<()> {
        self.real_stream.reset()?;
        self.current_real_token = None;
        self.st = None;
        Ok(())
    }

    fn end(&mut self) -> Result<()> {
        self.source.end_attributes();
        self.real_stream.end()
    }

    fn close(&mut self) -> Result<()> {
        self.real_stream.close()
    }

    fn attribute_source(&self) -> &AttributeSource {
        &self.source
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        &mut self.source
    }
}
