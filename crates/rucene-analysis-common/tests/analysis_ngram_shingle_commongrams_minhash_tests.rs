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

//! Port of the test classes of Lucene 10.5.0 `lucene/analysis/common/src/test`
//! that rmp task #459 lists, each run as a suite by the ported runner:
//! `org.apache.lucene.analysis.commongrams.TestCommonGramsFilter`,
//! `TestCommonGramsFilterFactory` and `TestCommonGramsQueryFilterFactory`
//! (with `common-1.txt`, `common-2.txt` and `common-snowball.txt` copied byte
//! for byte), `org.apache.lucene.analysis.ngram.TestEdgeNGramTokenFilter`,
//! `TestEdgeNGramTokenizer`, `TestNGramTokenFilter` and `TestNGramTokenizer`,
//! `org.apache.lucene.analysis.minhash.TestMinHashFilter` and
//! `org.apache.lucene.analysis.shingle.TestFixedShingleFilter`,
//! `TestShingleAnalyzerWrapper`, `TestShingleFilter` and
//! `TestShingleFilterFactory`.
//!
//! The helpers of the classes (`testNGrams`, `shingleFilterTest`,
//! `createTokenStream`, `compareRanks`, …) take arguments and are not test
//! methods.
//!
//! **Declared divergence met (`TestMinHashFilter.testTokenStream1`).** The
//! second expected min-hash term holds the unit `0xDE0B`, an unpaired low
//! surrogate. Rucene's `CharTermAttribute` holds `char`s, so the port's
//! `MinHashFilterLogic` emits U+FFFD for it (the divergence declared on
//! `MinHashFilterLogic`, rmp #459); the assertion compares against that
//! replacement and is marked at the point.

mod support;

use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::Arc;

use randomizedtesting::generators::RandomStrings;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::stop_filter::{make_stop_set, new_stop_filter};
use rucene::analysis::tokenattributes::{
    CharTermAttribute, OffsetAttribute, PositionIncrementAttribute, PositionLengthAttribute,
    DEFAULT_TYPE,
};
use rucene::analysis::{
    Analyzer, CharArraySet, CharReader, StringCharReader, TokenFilter, TokenFilterLogic,
    TokenStream,
};
use rucene::document::{Document, Store, TextField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::{IndexWriter, MAX_TERM_LENGTH};
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::Term;
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query::Builder as BooleanQueryBuilder;
use rucene::search::phrase_query::Builder as PhraseQueryBuilder;
use rucene::search::term_query::TermQuery;
use rucene::search::ScoreDoc;
use rucene::util::attribute::AttributeSource;
use rucene::util::automaton::{
    CharacterRunAutomaton, Operations, RegExp, DEFAULT_DETERMINIZE_WORK_LIMIT,
};
use rucene_analysis_common::analysis::commongrams::common_grams_filter::new_common_grams_filter;
use rucene_analysis_common::analysis::commongrams::common_grams_filter_factory::CommonGramsFilterFactory;
use rucene_analysis_common::analysis::commongrams::common_grams_query_filter::new_common_grams_query_filter;
use rucene_analysis_common::analysis::commongrams::common_grams_query_filter_factory::CommonGramsQueryFilterFactory;
use rucene_analysis_common::analysis::core::keyword_tokenizer::new_keyword_tokenizer;
use rucene_analysis_common::analysis::core::letter_tokenizer::new_letter_tokenizer;
use rucene_analysis_common::analysis::core::whitespace_tokenizer::new_whitespace_tokenizer;
use rucene_analysis_common::analysis::minhash::min_hash_filter::{
    get_bytes, is_less_than_unsigned, murmurhash3_x64_128, FixedSizeTreeSet, LongPair,
    MIN_HASH_TYPE,
};
use rucene_analysis_common::analysis::minhash::min_hash_filter_factory::MinHashFilterFactory;
use rucene_analysis_common::analysis::miscellaneous::ascii_folding_filter::new_ascii_folding_filter;
use rucene_analysis_common::analysis::ngram::edge_ngram_token_filter::new_edge_ngram_token_filter;
use rucene_analysis_common::analysis::ngram::edge_ngram_tokenizer::new_edge_ngram_tokenizer;
use rucene_analysis_common::analysis::ngram::ngram_token_filter::new_ngram_token_filter;
use rucene_analysis_common::analysis::ngram::ngram_tokenizer::{
    new_ngram_tokenizer, new_ngram_tokenizer_with_token_char,
};
use rucene_analysis_common::analysis::shingle::fixed_shingle_filter::FixedShingleFilter;
use rucene_analysis_common::analysis::shingle::shingle_analyzer_wrapper::ShingleAnalyzerWrapper;
use rucene_analysis_common::analysis::shingle::shingle_filter::{
    new_shingle_filter, new_shingle_filter_default, new_shingle_filter_with_max, ShingleFilter,
    DEFAULT_FILLER_TOKEN, DEFAULT_MAX_SHINGLE_SIZE, DEFAULT_MIN_SHINGLE_SIZE,
    DEFAULT_TOKEN_SEPARATOR,
};
use rucene_test_framework::analysis::mock_analyzer::MockAnalyzer;
use rucene_test_framework::analysis::mock_tokenizer::{MockTokenizer, WHITESPACE};
use rucene_test_framework::analysis::token::Token;
use rucene_test_framework::analysis::{
    shared, tokenizer_components, BaseTokenStreamFactoryTestCase, BaseTokenStreamTestCase,
    CannedTokenStream,
};
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::{RANDOM_MULTIPLIER, TEST_NIGHTLY};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};
use support::{
    boxed, btstc_class, factory_class, hook, iae_message, keyword_tokenizer_analyzer, own, run, t,
    whitespace_analyzer, FnAnalyzer,
};

type B = BaseTokenStreamTestCase;
type F = BaseTokenStreamFactoryTestCase;

/// `tokenFilterFactory(name, keysAndValues...)` (resources from the test
/// class loader).
fn fac(
    name: &str,
    keys_and_values: &[&str],
) -> Result<Arc<dyn rucene::analysis::factories::TokenFilterFactory>> {
    F::token_filter_factory(name, keys_and_values)
}

/// `new StringReader(s)`.
fn string_reader(s: &str) -> Box<dyn CharReader> {
    Box::new(StringCharReader::new(s))
}

// -----------------------------------------------------------------------------
// Shared helpers
// -----------------------------------------------------------------------------

fn random() -> Arc<dyn Random> {
    LuceneTestCase::random()
}

fn mult() -> i32 {
    *RANDOM_MULTIPLIER
}

fn reader(s: &str) -> Box<dyn CharReader> {
    Box::new(StringCharReader::new(s))
}

/// `whitespaceMockTokenizer(s)` as a filter input.
fn ws(s: &str) -> Result<Box<dyn TokenStream>> {
    Ok(boxed(B::whitespace_mock_tokenizer_str(s)?))
}

/// `termAtt.toString()`.
fn term_of(ts: &dyn TokenStream) -> String {
    support::term(ts)
}

fn offsets_of(ts: &dyn TokenStream) -> (i32, i32) {
    let o = ts
        .attribute_source()
        .get_attribute_interface::<dyn OffsetAttribute>()
        .expect("offsetAtt");
    (o.start_offset(), o.end_offset())
}

fn pos_inc_of(ts: &dyn TokenStream) -> i32 {
    ts.attribute_source()
        .get_attribute_interface::<dyn PositionIncrementAttribute>()
        .expect("posIncAtt")
        .get_position_increment()
}

fn pos_len_of(ts: &dyn TokenStream) -> i32 {
    ts.attribute_source()
        .get_attribute_interface::<dyn PositionLengthAttribute>()
        .expect("posLenAtt")
        .get_position_length()
}

/// `s.length()`: UTF-16 units.
fn jlen(s: &str) -> i32 {
    s.encode_utf16().count() as i32
}

/// `s.substring(0, Character.offsetByCodePoints(s, 0, n))`, as `&str`.
fn first_code_points(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

/// `new Analyzer() { MockTokenizer(WHITESPACE, false) + filter }`.
fn ws_false_analyzer(
    name: &'static str,
    chain: impl Fn(Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> + Send + Sync + 'static,
) -> Arc<dyn Analyzer> {
    whitespace_analyzer(name, chain)
}

thread_local! {
    /// The `input` field of the n-gram filter tests (`setUp`).
    static INPUT: RefCell<Option<Box<dyn TokenStream>>> = const { RefCell::new(None) };
}

fn set_input(ts: Box<dyn TokenStream>) {
    INPUT.with(|i| *i.borrow_mut() = Some(ts));
}

fn input() -> Box<dyn TokenStream> {
    INPUT.with(|i| i.borrow_mut().take().expect("setUp"))
}

fn ngram_filter_set_up() -> Result<()> {
    LuceneTestCase::super_set_up();
    set_input(ws("abcde")?);
    Ok(())
}

fn ngram_filter_tear_down() -> Result<()> {
    INPUT.with(|i| *i.borrow_mut() = None);
    LuceneTestCase::super_tear_down();
    Ok(())
}

// -----------------------------------------------------------------------------
// TestNGramTokenFilter
// -----------------------------------------------------------------------------

fn ngtf_test_invalid_input() -> Result<()> {
    expect_throws("java.lang.IllegalArgumentException", || {
        new_ngram_token_filter(input(), 2, 1, false)
    });
    Ok(())
}

fn ngtf_test_invalid_input2() -> Result<()> {
    expect_throws("java.lang.IllegalArgumentException", || {
        new_ngram_token_filter(input(), 0, 1, false)
    });
    Ok(())
}

fn ngtf_test_unigrams() -> Result<()> {
    let mut filter = new_ngram_token_filter(input(), 1, 1, false)?;
    B::assert_token_stream_contents_offsets_pos_increments(
        &mut filter,
        &["a", "b", "c", "d", "e"],
        &[0, 0, 0, 0, 0],
        &[5, 5, 5, 5, 5],
        &[1, 0, 0, 0, 0],
    )
}

fn ngtf_test_bigrams() -> Result<()> {
    let mut filter = new_ngram_token_filter(input(), 2, 2, false)?;
    B::assert_token_stream_contents_offsets_pos_increments(
        &mut filter,
        &["ab", "bc", "cd", "de"],
        &[0, 0, 0, 0],
        &[5, 5, 5, 5],
        &[1, 0, 0, 0],
    )
}

fn ngtf_assert_ngrams() -> Result<()> {
    let mut filter = new_ngram_token_filter(input(), 1, 3, false)?;
    B::assert_token_stream_contents_graph(
        &mut filter,
        &[
            "a", "ab", "abc", "b", "bc", "bcd", "c", "cd", "cde", "d", "de", "e",
        ],
        Some(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
        Some(&[5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5]),
        None,
        Some(&[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
        None,
        None,
        false,
    )
}

fn ngtf_test_ngrams() -> Result<()> {
    ngtf_assert_ngrams()
}

fn ngtf_test_ngrams_no_increment() -> Result<()> {
    ngtf_assert_ngrams()
}

fn ngtf_test_oversized_ngrams() -> Result<()> {
    let mut filter = new_ngram_token_filter(input(), 6, 7, false)?;
    B::assert_token_stream_contents_offsets(&mut filter, &[], &[], &[])
}

fn ngtf_test_oversized_ngrams_preserve_original() -> Result<()> {
    let mut tokenizer = new_ngram_token_filter(input(), 6, 6, true)?;
    B::assert_token_stream_contents_offsets(&mut tokenizer, &["abcde"], &[0], &[5])
}

fn ngtf_test_small_token_in_stream() -> Result<()> {
    drop(input());
    let mut tokenizer = new_ngram_token_filter(ws("abc de fgh")?, 3, 3, false)?;
    B::assert_token_stream_contents_offsets_pos_increments(
        &mut tokenizer,
        &["abc", "fgh"],
        &[0, 7],
        &[3, 10],
        &[1, 2],
    )
}

fn ngtf_test_small_token_in_stream_preserve_original() -> Result<()> {
    drop(input());
    let mut tokenizer = new_ngram_token_filter(ws("abc de fgh")?, 3, 3, true)?;
    B::assert_token_stream_contents_offsets_pos_increments(
        &mut tokenizer,
        &["abc", "de", "fgh"],
        &[0, 4, 7],
        &[3, 6, 10],
        &[1, 1, 1],
    )
}

fn ngtf_test_reset() -> Result<()> {
    let tokenizer = own(new_whitespace_tokenizer());
    tokenizer.borrow_mut().set_reader(reader("abcde"))?;
    let mut filter = new_ngram_token_filter(shared(&tokenizer), 1, 1, false)?;
    B::assert_token_stream_contents_offsets_pos_increments(
        &mut filter,
        &["a", "b", "c", "d", "e"],
        &[0, 0, 0, 0, 0],
        &[5, 5, 5, 5, 5],
        &[1, 0, 0, 0, 0],
    )?;
    tokenizer.borrow_mut().set_reader(reader("abcde"))?;
    B::assert_token_stream_contents_offsets_pos_increments(
        &mut filter,
        &["a", "b", "c", "d", "e"],
        &[0, 0, 0, 0, 0],
        &[5, 5, 5, 5, 5],
        &[1, 0, 0, 0, 0],
    )
}

fn ngtf_test_keep_short_term_keep_long_term() -> Result<()> {
    let input_string = "a bcd efghi jk";
    {
        // preserveOriginal = false
        let mut filter = new_ngram_token_filter(ws(input_string)?, 2, 3, false)?;
        B::assert_token_stream_contents_offsets_pos_increments(
            &mut filter,
            &[
                "bc", "bcd", "cd", "ef", "efg", "fg", "fgh", "gh", "ghi", "hi", "jk",
            ],
            &[2, 2, 2, 6, 6, 6, 6, 6, 6, 6, 12],
            &[5, 5, 5, 11, 11, 11, 11, 11, 11, 11, 14],
            &[2, 0, 0, 1, 0, 0, 0, 0, 0, 0, 1],
        )?;
    }
    {
        // preserveOriginal = true
        let mut filter = new_ngram_token_filter(ws(input_string)?, 2, 3, true)?;
        B::assert_token_stream_contents_offsets_pos_increments(
            &mut filter,
            &[
                "a", "bc", "bcd", "cd", "ef", "efg", "fg", "fgh", "gh", "ghi", "hi", "efghi", "jk",
            ],
            &[0, 2, 2, 2, 6, 6, 6, 6, 6, 6, 6, 6, 12],
            &[1, 5, 5, 5, 11, 11, 11, 11, 11, 11, 11, 11, 14],
            &[1, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1],
        )?;
    }
    Ok(())
}

// LUCENE-3642
// EdgeNgram blindly adds term length to offset, but this can take things out of bounds
// wrt original text if a previous filter increases the length of the word (in this case æ -> ae)
// so in this case we behave like WDF, and preserve any modified offsets
fn ngtf_test_invalid_offsets() -> Result<()> {
    let analyzer = ws_false_analyzer(
        "org.apache.lucene.analysis.ngram.TestNGramTokenFilter$1",
        |t| {
            let filters = boxed(new_ascii_folding_filter(t)?);
            Ok(boxed(new_ngram_token_filter(filters, 2, 2, false)?))
        },
    );
    B::assert_analyzes_to_offsets_pos_increments(
        &*analyzer,
        "mosfellsb\u{E6}r",
        &[
            "mo", "os", "sf", "fe", "el", "ll", "ls", "sb", "ba", "ae", "er",
        ],
        &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        &[11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11],
        &[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    )?;
    analyzer.close();
    Ok(())
}

/// `testEndPositionIncrement` over `make(source)`.
fn end_position_increment(
    make: impl FnOnce(Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>>,
) -> Result<()> {
    let source = ws("seventeen one two three four")?;
    let mut input = make(source)?;
    input
        .attribute_source_mut()
        .add_attribute_interface::<dyn PositionIncrementAttribute>()?;
    input.reset()?;
    while input.increment_token()? {}
    input.end()?;
    Assert::assert_equals(4, pos_inc_of(&*input));
    Ok(())
}

fn ngtf_test_end_position_increment() -> Result<()> {
    end_position_increment(|s| Ok(boxed(new_ngram_token_filter(s, 8, 8, false)?)))
}

/// blast some random strings through the analyzer
fn ngtf_test_random_strings() -> Result<()> {
    for _ in 0..10 {
        let min = TestUtil::next_int(&*random(), 2, 10);
        let max = TestUtil::next_int(&*random(), min, 20);
        let preserve_original = TestUtil::next_int(&*random(), 0, 1) % 2 == 0;
        let a = ws_false_analyzer(
            "org.apache.lucene.analysis.ngram.TestNGramTokenFilter$2",
            move |t| {
                Ok(boxed(new_ngram_token_filter(
                    t,
                    min,
                    max,
                    preserve_original,
                )?))
            },
        );
        B::check_random_data_max_word_length(&*random(), &a, 10 * mult(), 20)?;
        a.close();
    }
    Ok(())
}

fn ngtf_test_empty_term() -> Result<()> {
    let random = random();
    let a = keyword_tokenizer_analyzer(
        "org.apache.lucene.analysis.ngram.TestNGramTokenFilter$3",
        |t| Ok(boxed(new_ngram_token_filter(t, 2, 15, false)?)),
    );
    let use_char_filter = random.next_boolean();
    B::check_analysis_consistency(&random, &*a, use_char_filter, "")?;
    a.close();
    Ok(())
}

/// The expectations of `testSupplementaryCharacters` before and after the
/// grams, and the stream setup: a `KeywordTokenizer` over `s` under `make`.
fn supplementary_stream(
    s: &str,
    make: impl FnOnce(Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>>,
) -> Result<Box<dyn TokenStream>> {
    let mut tk = new_keyword_tokenizer();
    tk.set_reader(reader(s))?;
    let mut tk = make(boxed(tk))?;
    tk.attribute_source_mut()
        .add_attribute_interface::<dyn CharTermAttribute>()?;
    tk.attribute_source_mut()
        .add_attribute_interface::<dyn OffsetAttribute>()?;
    tk.reset()?;
    Ok(tk)
}

/// `assertTrue(tk.incrementToken())` and the offsets/term checks.
fn assert_next_gram(tk: &mut dyn TokenStream, s: &str, expected: &str) -> Result<()> {
    Assert::assert_true(tk.increment_token()?);
    let (start, end) = offsets_of(tk);
    Assert::assert_equals(0, start);
    Assert::assert_equals(jlen(s), end);
    Assert::assert_equals(expected, term_of(tk).as_str());
    Ok(())
}

fn ngtf_test_supplementary_characters() -> Result<()> {
    for _ in 0..20 {
        let s = TestUtil::random_unicode_string_max(&*random(), 10);
        let code_point_count = s.chars().count() as i32;
        let min_gram = TestUtil::next_int(&*random(), 1, 3);
        let max_gram = TestUtil::next_int(&*random(), min_gram, 10);
        let preserve_original = TestUtil::next_int(&*random(), 0, 1) % 2 == 0;
        let mut tk = supplementary_stream(&s, |tk| {
            Ok(boxed(new_ngram_token_filter(
                tk,
                min_gram,
                max_gram,
                preserve_original,
            )?))
        })?;
        if code_point_count < min_gram && preserve_original {
            assert_next_gram(&mut *tk, &s, &s)?;
        }
        let cps: Vec<char> = s.chars().collect();
        for start in 0..code_point_count {
            let mut end = start + min_gram;
            while end <= code_point_count.min(start + max_gram) {
                let gram: String = cps[start as usize..end as usize].iter().collect();
                assert_next_gram(&mut *tk, &s, &gram)?;
                end += 1;
            }
        }
        if code_point_count > max_gram && preserve_original {
            assert_next_gram(&mut *tk, &s, &s)?;
        }
        Assert::assert_false(tk.increment_token()?);
        tk.close()?;
    }
    Ok(())
}

fn ngram_token_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.ngram.TestNGramTokenFilter")
        .method(hook("setUp", ngram_filter_set_up))
        .method(hook("tearDown", ngram_filter_tear_down))
        .method(t("testInvalidInput", ngtf_test_invalid_input))
        .method(t("testInvalidInput2", ngtf_test_invalid_input2))
        .method(t("testUnigrams", ngtf_test_unigrams))
        .method(t("testBigrams", ngtf_test_bigrams))
        .method(t("testNgrams", ngtf_test_ngrams))
        .method(t("testNgramsNoIncrement", ngtf_test_ngrams_no_increment))
        .method(t("testOversizedNgrams", ngtf_test_oversized_ngrams))
        .method(t(
            "testOversizedNgramsPreserveOriginal",
            ngtf_test_oversized_ngrams_preserve_original,
        ))
        .method(t("testSmallTokenInStream", ngtf_test_small_token_in_stream))
        .method(t(
            "testSmallTokenInStreamPreserveOriginal",
            ngtf_test_small_token_in_stream_preserve_original,
        ))
        .method(t("testReset", ngtf_test_reset))
        .method(t(
            "testKeepShortTermKeepLongTerm",
            ngtf_test_keep_short_term_keep_long_term,
        ))
        .method(t("testInvalidOffsets", ngtf_test_invalid_offsets))
        .method(t(
            "testEndPositionIncrement",
            ngtf_test_end_position_increment,
        ))
        .method(t("testRandomStrings", ngtf_test_random_strings))
        .method(t("testEmptyTerm", ngtf_test_empty_term))
        .method(t(
            "testSupplementaryCharacters",
            ngtf_test_supplementary_characters,
        ))
        .build()
}

// -----------------------------------------------------------------------------
// TestEdgeNGramTokenFilter
// -----------------------------------------------------------------------------

fn egtf_test_invalid_input() -> Result<()> {
    expect_throws("java.lang.IllegalArgumentException", || {
        new_edge_ngram_token_filter(input(), 0, 0, false)
    });
    Ok(())
}

fn egtf_test_invalid_input2() -> Result<()> {
    expect_throws("java.lang.IllegalArgumentException", || {
        new_edge_ngram_token_filter(input(), 2, 1, false)
    });
    Ok(())
}

fn egtf_test_invalid_input3() -> Result<()> {
    expect_throws("java.lang.IllegalArgumentException", || {
        new_edge_ngram_token_filter(input(), -1, 2, false)
    });
    Ok(())
}

fn egtf_test_front_unigram() -> Result<()> {
    let mut tokenizer = new_edge_ngram_token_filter(input(), 1, 1, false)?;
    B::assert_token_stream_contents_offsets(&mut tokenizer, &["a"], &[0], &[5])
}

fn egtf_test_oversized_ngrams() -> Result<()> {
    let mut tokenizer = new_edge_ngram_token_filter(input(), 6, 6, false)?;
    B::assert_token_stream_contents_offsets(&mut tokenizer, &[], &[], &[])
}

fn egtf_test_oversized_ngrams_preserve_original() -> Result<()> {
    let mut tokenizer = new_edge_ngram_token_filter(input(), 6, 6, true)?;
    B::assert_token_stream_contents_offsets(&mut tokenizer, &["abcde"], &[0], &[5])
}

fn egtf_test_preserve_original() -> Result<()> {
    let input_string = "a bcd efghi jk";
    {
        // preserveOriginal = false
        let mut filter = new_edge_ngram_token_filter(ws(input_string)?, 2, 3, false)?;
        B::assert_token_stream_contents_offsets_pos_increments(
            &mut filter,
            &["bc", "bcd", "ef", "efg", "jk"],
            &[2, 2, 6, 6, 12],
            &[5, 5, 11, 11, 14],
            &[2, 0, 1, 0, 1],
        )?;
    }
    {
        // preserveOriginal = true
        let mut filter = new_edge_ngram_token_filter(ws(input_string)?, 2, 3, true)?;
        B::assert_token_stream_contents_offsets_pos_increments(
            &mut filter,
            &["a", "bc", "bcd", "ef", "efg", "efghi", "jk"],
            &[0, 2, 2, 6, 6, 6, 12],
            &[1, 5, 5, 11, 11, 11, 14],
            &[1, 1, 0, 1, 0, 0, 1],
        )?;
    }
    Ok(())
}

fn egtf_test_front_range_of_ngrams() -> Result<()> {
    let mut tokenizer = new_edge_ngram_token_filter(input(), 1, 3, false)?;
    B::assert_token_stream_contents_offsets(
        &mut tokenizer,
        &["a", "ab", "abc"],
        &[0, 0, 0],
        &[5, 5, 5],
    )
}

fn egtf_test_filter_positions() -> Result<()> {
    let mut tokenizer = new_edge_ngram_token_filter(ws("abcde vwxyz")?, 1, 3, false)?;
    B::assert_token_stream_contents_offsets(
        &mut tokenizer,
        &["a", "ab", "abc", "v", "vw", "vwx"],
        &[0, 0, 0, 6, 6, 6],
        &[5, 5, 5, 11, 11, 11],
    )
}

/// `PositionFilter`: all but the first token get a position increment of 0.
#[derive(Debug, Default)]
struct PositionFilterLogic {
    started: bool,
}

impl TokenFilterLogic for PositionFilterLogic {
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if input.increment_token()? {
            if self.started {
                source
                    .get_attribute_interface_mut::<dyn PositionIncrementAttribute>()
                    .expect("posIncrAtt")
                    .set_position_increment(0);
            } else {
                self.started = true;
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.started = false;
        Ok(())
    }
}

fn position_filter(input: Box<dyn TokenStream>) -> Result<TokenFilter<PositionFilterLogic>> {
    let mut filter = TokenFilter::new(input, PositionFilterLogic::default());
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn PositionIncrementAttribute>()?;
    Ok(filter)
}

fn egtf_test_first_token_position_increment() -> Result<()> {
    let ts = boxed(position_filter(ws("a abc")?)?); // All but first token will get 0 position increment
    let mut filter = new_edge_ngram_token_filter(ts, 2, 3, false)?;
    // The first token "a" will not be output, since it's smaller than the mingram size of 2.
    // The second token on input to EdgeNGramTokenFilter will have position increment of 0,
    // which should be increased to 1, since this is the first output token in the stream.
    B::assert_token_stream_contents_offsets_pos_increments(
        &mut filter,
        &["ab", "abc"],
        &[2, 2],
        &[5, 5],
        &[1, 0],
    )
}

fn egtf_test_small_token_in_stream() -> Result<()> {
    drop(input());
    let mut tokenizer = new_edge_ngram_token_filter(ws("abc de fgh")?, 3, 3, false)?;
    B::assert_token_stream_contents_offsets(&mut tokenizer, &["abc", "fgh"], &[0, 7], &[3, 10])
}

fn egtf_test_reset() -> Result<()> {
    let tokenizer = own(new_whitespace_tokenizer());
    tokenizer.borrow_mut().set_reader(reader("abcde"))?;
    let mut filter = new_edge_ngram_token_filter(shared(&tokenizer), 1, 3, false)?;
    B::assert_token_stream_contents_offsets(
        &mut filter,
        &["a", "ab", "abc"],
        &[0, 0, 0],
        &[5, 5, 5],
    )?;
    tokenizer.borrow_mut().set_reader(reader("abcde"))?;
    B::assert_token_stream_contents_offsets(
        &mut filter,
        &["a", "ab", "abc"],
        &[0, 0, 0],
        &[5, 5, 5],
    )
}

/// blast some random strings through the analyzer
fn egtf_test_random_strings() -> Result<()> {
    for _ in 0..10 {
        let min = TestUtil::next_int(&*random(), 2, 10);
        let max = TestUtil::next_int(&*random(), min, 20);
        let preserve_original = TestUtil::next_int(&*random(), 0, 1) % 2 == 0;
        let a = ws_false_analyzer(
            "org.apache.lucene.analysis.ngram.TestEdgeNGramTokenFilter$1",
            move |t| {
                Ok(boxed(new_edge_ngram_token_filter(
                    t,
                    min,
                    max,
                    preserve_original,
                )?))
            },
        );
        B::check_random_data(&*random(), &a, 10 * mult())?;
        a.close();
    }
    Ok(())
}

fn egtf_test_empty_term() -> Result<()> {
    let random = random();
    let a = keyword_tokenizer_analyzer(
        "org.apache.lucene.analysis.ngram.TestEdgeNGramTokenFilter$2",
        |t| Ok(boxed(new_edge_ngram_token_filter(t, 2, 15, false)?)),
    );
    let use_char_filter = random.next_boolean();
    B::check_analysis_consistency(&random, &*a, use_char_filter, "")?;
    a.close();
    Ok(())
}

fn egtf_test_graphs() -> Result<()> {
    let mut tk = new_letter_tokenizer();
    tk.set_reader(reader("abc d efgh ij klmno p q"))?;
    let tk = boxed(new_shingle_filter_default(boxed(tk))?);
    let mut tk = new_edge_ngram_token_filter(tk, 7, 10, false)?;
    B::assert_token_stream_contents_offsets_positions_final_offset(
        &mut tk,
        &["efgh ij", "ij klmn", "ij klmno", "klmno p"],
        &[6, 11, 11, 14],
        &[13, 19, 19, 21],
        &[3, 1, 0, 1],
        &[2, 2, 2, 2],
        Some(23),
    )
}

fn egtf_test_supplementary_characters() -> Result<()> {
    for _ in 0..20 {
        let s = TestUtil::random_unicode_string_max(&*random(), 10);
        let code_point_count = s.chars().count() as i32;
        let min_gram = TestUtil::next_int(&*random(), 1, 3);
        let max_gram = TestUtil::next_int(&*random(), min_gram, 10);
        let preserve_original = TestUtil::next_int(&*random(), 0, 1) % 2 == 0;
        let mut tk = supplementary_stream(&s, |tk| {
            Ok(boxed(new_edge_ngram_token_filter(
                tk,
                min_gram,
                max_gram,
                preserve_original,
            )?))
        })?;
        if code_point_count < min_gram && preserve_original {
            assert_next_gram(&mut *tk, &s, &s)?;
        }
        let mut j = min_gram;
        while j <= code_point_count.min(max_gram) {
            assert_next_gram(&mut *tk, &s, first_code_points(&s, j as usize))?;
            j += 1;
        }
        if code_point_count > max_gram && preserve_original {
            assert_next_gram(&mut *tk, &s, &s)?;
        }
        Assert::assert_false(tk.increment_token()?);
        tk.close()?;
    }
    Ok(())
}

fn egtf_test_end_position_increment() -> Result<()> {
    end_position_increment(|s| Ok(boxed(new_edge_ngram_token_filter(s, 8, 8, false)?)))
}

fn edge_ngram_token_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.ngram.TestEdgeNGramTokenFilter")
        .method(hook("setUp", ngram_filter_set_up))
        .method(hook("tearDown", ngram_filter_tear_down))
        .method(t("testInvalidInput", egtf_test_invalid_input))
        .method(t("testInvalidInput2", egtf_test_invalid_input2))
        .method(t("testInvalidInput3", egtf_test_invalid_input3))
        .method(t("testFrontUnigram", egtf_test_front_unigram))
        .method(t("testOversizedNgrams", egtf_test_oversized_ngrams))
        .method(t(
            "testOversizedNgramsPreserveOriginal",
            egtf_test_oversized_ngrams_preserve_original,
        ))
        .method(t("testPreserveOriginal", egtf_test_preserve_original))
        .method(t("testFrontRangeOfNgrams", egtf_test_front_range_of_ngrams))
        .method(t("testFilterPositions", egtf_test_filter_positions))
        .method(t(
            "testFirstTokenPositionIncrement",
            egtf_test_first_token_position_increment,
        ))
        .method(t("testSmallTokenInStream", egtf_test_small_token_in_stream))
        .method(t("testReset", egtf_test_reset))
        .method(t("testRandomStrings", egtf_test_random_strings))
        .method(t("testEmptyTerm", egtf_test_empty_term))
        .method(t("testGraphs", egtf_test_graphs))
        .method(t(
            "testSupplementaryCharacters",
            egtf_test_supplementary_characters,
        ))
        .method(t(
            "testEndPositionIncrement",
            egtf_test_end_position_increment,
        ))
        .build()
}

// -----------------------------------------------------------------------------
// TestNGramTokenizer and TestEdgeNGramTokenizer
// -----------------------------------------------------------------------------

thread_local! {
    /// The `input` reader field of the n-gram tokenizer tests (`setUp`).
    static READER: RefCell<Option<Box<dyn CharReader>>> = const { RefCell::new(None) };
}

fn input_reader() -> Box<dyn CharReader> {
    READER.with(|r| r.borrow_mut().take().expect("setUp"))
}

fn ngram_tokenizer_set_up() -> Result<()> {
    LuceneTestCase::super_set_up();
    READER.with(|r| *r.borrow_mut() = Some(reader("abcde")));
    Ok(())
}

fn ngram_tokenizer_tear_down() -> Result<()> {
    READER.with(|r| *r.borrow_mut() = None);
    LuceneTestCase::super_tear_down();
    Ok(())
}

fn ngt_test_invalid_input() -> Result<()> {
    expect_throws("java.lang.IllegalArgumentException", || {
        new_ngram_tokenizer(2, 1)
    });
    Ok(())
}

fn ngt_test_invalid_input2() -> Result<()> {
    expect_throws("java.lang.IllegalArgumentException", || {
        let mut tok = new_ngram_tokenizer(0, 1)?;
        tok.set_reader(input_reader())
    });
    Ok(())
}

fn ngt_test_unigrams() -> Result<()> {
    let mut tokenizer = new_ngram_tokenizer(1, 1)?;
    tokenizer.set_reader(input_reader())?;
    B::assert_token_stream_contents_offsets_final_offset(
        &mut tokenizer,
        &["a", "b", "c", "d", "e"],
        &[0, 1, 2, 3, 4],
        &[1, 2, 3, 4, 5],
        Some(5), /* abcde */
    )
}

fn ngt_test_bigrams() -> Result<()> {
    let mut tokenizer = new_ngram_tokenizer(2, 2)?;
    tokenizer.set_reader(input_reader())?;
    B::assert_token_stream_contents_offsets_final_offset(
        &mut tokenizer,
        &["ab", "bc", "cd", "de"],
        &[0, 1, 2, 3],
        &[2, 3, 4, 5],
        Some(5), /* abcde */
    )
}

fn ngt_test_ngrams() -> Result<()> {
    let mut tokenizer = new_ngram_tokenizer(1, 3)?;
    tokenizer.set_reader(input_reader())?;
    B::assert_token_stream_contents_graph(
        &mut tokenizer,
        &[
            "a", "ab", "abc", "b", "bc", "bcd", "c", "cd", "cde", "d", "de", "e",
        ],
        Some(&[0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 3, 4]),
        Some(&[1, 2, 3, 2, 3, 4, 3, 4, 5, 4, 5, 5]),
        None,
        None,
        None,
        Some(5), /* abcde */
        false,
    )
}

fn ngt_test_oversized_ngrams() -> Result<()> {
    let mut tokenizer = new_ngram_tokenizer(6, 7)?;
    tokenizer.set_reader(input_reader())?;
    B::assert_token_stream_contents_offsets_final_offset(&mut tokenizer, &[], &[], &[], Some(5))
}

fn ngt_test_reset() -> Result<()> {
    let mut tokenizer = new_ngram_tokenizer(1, 1)?;
    tokenizer.set_reader(input_reader())?;
    B::assert_token_stream_contents_offsets_final_offset(
        &mut tokenizer,
        &["a", "b", "c", "d", "e"],
        &[0, 1, 2, 3, 4],
        &[1, 2, 3, 4, 5],
        Some(5), /* abcde */
    )?;
    tokenizer.set_reader(reader("abcde"))?;
    B::assert_token_stream_contents_offsets_final_offset(
        &mut tokenizer,
        &["a", "b", "c", "d", "e"],
        &[0, 1, 2, 3, 4],
        &[1, 2, 3, 4, 5],
        Some(5), /* abcde */
    )
}

/// `new Analyzer() { tokenizer = make(); return new TokenStreamComponents(tokenizer, tokenizer); }`.
fn tokenizer_only_analyzer<T: rucene_test_framework::analysis::ReaderTokenizer + 'static>(
    name: &'static str,
    make: impl Fn() -> Result<T> + Send + Sync + 'static,
) -> Arc<dyn Analyzer> {
    FnAnalyzer::new(name, move |_| Ok(tokenizer_components(own(make()?)))).arc()
}

/// blast some random strings through the analyzer
fn ngt_test_random_strings() -> Result<()> {
    let num_iters = if *TEST_NIGHTLY { 10 } else { 1 };
    for _ in 0..num_iters {
        let min = TestUtil::next_int(&*random(), 2, 10);
        let max = TestUtil::next_int(&*random(), min, 20);
        let a = tokenizer_only_analyzer(
            "org.apache.lucene.analysis.ngram.TestNGramTokenizer$1",
            move || new_ngram_tokenizer(min, max),
        );
        B::check_random_data_max_word_length(&*random(), &a, 200 * mult(), 20)?;
        B::check_random_data_max_word_length(&*random(), &a, 10 * mult(), 1027)?;
        a.close();
    }
    Ok(())
}

/// `testNGrams(minGram, maxGram, length, nonTokenChars)`: a random ASCII
/// letter string of `length`.
fn test_ngrams_len(
    min_gram: i32,
    max_gram: i32,
    length: i32,
    non_token_chars: &str,
    edges_only: bool,
) -> Result<()> {
    let s = RandomStrings::random_ascii_letters_of_length_between(&*random(), length, length);
    test_ngrams(min_gram, max_gram, &s, non_token_chars, edges_only)
}

/// `isTokenChar(nonTokenChars, codePoint)`.
fn is_token_char(non_token_chars: &str, code_point: char) -> bool {
    !non_token_chars.chars().any(|cp| cp == code_point)
}

/// `TestNGramTokenizer.testNGrams(minGram, maxGram, s, nonTokenChars, edgesOnly)`.
fn test_ngrams(
    min_gram: i32,
    max_gram: i32,
    s: &str,
    non_token_chars: &str,
    edges_only: bool,
) -> Result<()> {
    // convert the string to code points
    let code_points: Vec<char> = s.chars().collect();
    let mut offsets = vec![0i32; code_points.len() + 1];
    for (i, cp) in code_points.iter().enumerate() {
        offsets[i + 1] = offsets[i] + cp.len_utf16() as i32;
    }
    let non_token: Vec<char> = non_token_chars.chars().collect();
    // the anonymous subclass's `isTokenChar(chr)`: `nonTokenChars.indexOf(chr) < 0`
    let mut grams = new_ngram_tokenizer_with_token_char(
        rucene::analysis::default_token_attribute_factory(),
        min_gram,
        max_gram,
        edges_only,
        Box::new(move |chr: i32| !non_token.iter().any(|&c| c as i32 == chr)),
    )?;
    grams.set_reader(reader(s))?;
    {
        let src = grams.attribute_source_mut();
        src.add_attribute_interface::<dyn CharTermAttribute>()?;
        src.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
        src.add_attribute_interface::<dyn PositionLengthAttribute>()?;
        src.add_attribute_interface::<dyn OffsetAttribute>()?;
    }
    grams.reset()?;
    let n = code_points.len() as i32;
    for start in 0..n {
        'next_gram: for end in (start + min_gram)..=(start + max_gram).min(n) {
            if edges_only
                && start > 0
                && is_token_char(non_token_chars, code_points[(start - 1) as usize])
            {
                // not on an edge
                continue 'next_gram;
            }
            for j in start..end {
                if !is_token_char(non_token_chars, code_points[j as usize]) {
                    continue 'next_gram;
                }
            }
            Assert::assert_true(grams.increment_token()?);
            let expected: String = code_points[start as usize..end as usize].iter().collect();
            Assert::assert_true(expected == term_of(&grams));
            Assert::assert_equals(1, pos_inc_of(&grams));
            Assert::assert_equals(1, pos_len_of(&grams));
            let (so, eo) = offsets_of(&grams);
            Assert::assert_equals(offsets[start as usize], so);
            Assert::assert_equals(offsets[end as usize], eo);
        }
    }
    Assert::assert_false(grams.increment_token()?);
    grams.end()?;
    let (so, eo) = offsets_of(&grams);
    Assert::assert_equals(jlen(s), so);
    Assert::assert_equals(jlen(s), eo);
    Ok(())
}

fn ngram_large_input(edges_only: bool) -> Result<()> {
    // test sliding
    let min_gram = TestUtil::next_int(&*random(), 1, 100);
    let max_gram = TestUtil::next_int(&*random(), min_gram, 100);
    let length = TestUtil::next_int(&*random(), 3 * 1024, 4 * 1024);
    test_ngrams_len(min_gram, max_gram, length, "", edges_only)
}

fn ngram_large_max_gram(edges_only: bool) -> Result<()> {
    // test sliding with maxGram > 1024
    let min_gram = TestUtil::next_int(&*random(), 1290, 1300);
    let max_gram = TestUtil::next_int(&*random(), min_gram, 1300);
    let length = TestUtil::next_int(&*random(), 3 * 1024, 4 * 1024);
    test_ngrams_len(min_gram, max_gram, length, "", edges_only)
}

fn ngram_pre_tokenization(edges_only: bool, non_token_chars: &str) -> Result<()> {
    let min_gram = TestUtil::next_int(&*random(), 1, 100);
    let max_gram = TestUtil::next_int(&*random(), min_gram, 100);
    let length = TestUtil::next_int(&*random(), 0, 4 * 1024);
    test_ngrams_len(min_gram, max_gram, length, non_token_chars, edges_only)
}

fn ngram_few_token_chars(edges_only: bool) -> Result<()> {
    let n = TestUtil::next_int(&*random(), 4000, 5000) as usize;
    let mut chrs = vec![' '; n];
    for c in chrs.iter_mut() {
        if random().next_float() < 0.1 {
            *c = 'a';
        }
    }
    let min_gram = TestUtil::next_int(&*random(), 1, 2);
    let max_gram = TestUtil::next_int(&*random(), min_gram, 2);
    let s: String = chrs.into_iter().collect();
    test_ngrams(min_gram, max_gram, &s, " ", edges_only)
}

fn ngram_full_utf8_range(edges_only: bool) -> Result<()> {
    let min_gram = TestUtil::next_int(&*random(), 1, 100);
    let max_gram = TestUtil::next_int(&*random(), min_gram, 100);
    let s = TestUtil::random_unicode_string_max(&*random(), 4 * 1024);
    test_ngrams(min_gram, max_gram, &s, "", edges_only)?;
    test_ngrams(min_gram, max_gram, &s, "abcdef", edges_only)
}

fn ngt_test_large_input() -> Result<()> {
    ngram_large_input(false)
}

fn ngt_test_large_max_gram() -> Result<()> {
    ngram_large_max_gram(false)
}

fn ngt_test_pre_tokenization() -> Result<()> {
    ngram_pre_tokenization(false, "a")
}

fn ngt_test_heavy_pre_tokenization() -> Result<()> {
    ngram_pre_tokenization(false, "abcdef")
}

fn ngt_test_few_token_chars() -> Result<()> {
    ngram_few_token_chars(false)
}

fn ngt_test_full_utf8_range() -> Result<()> {
    ngram_full_utf8_range(false)
}

fn ngram_tokenizer_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.ngram.TestNGramTokenizer")
        .method(hook("setUp", ngram_tokenizer_set_up))
        .method(hook("tearDown", ngram_tokenizer_tear_down))
        .method(t("testInvalidInput", ngt_test_invalid_input))
        .method(t("testInvalidInput2", ngt_test_invalid_input2))
        .method(t("testUnigrams", ngt_test_unigrams))
        .method(t("testBigrams", ngt_test_bigrams))
        .method(t("testNgrams", ngt_test_ngrams))
        .method(t("testOversizedNgrams", ngt_test_oversized_ngrams))
        .method(t("testReset", ngt_test_reset))
        .method(t("testRandomStrings", ngt_test_random_strings))
        .method(t("testLargeInput", ngt_test_large_input))
        .method(t("testLargeMaxGram", ngt_test_large_max_gram))
        .method(t("testPreTokenization", ngt_test_pre_tokenization))
        .method(t(
            "testHeavyPreTokenization",
            ngt_test_heavy_pre_tokenization,
        ))
        .method(t("testFewTokenChars", ngt_test_few_token_chars))
        .method(t("testFullUTF8Range", ngt_test_full_utf8_range))
        .build()
}

fn egt_test_invalid_input() -> Result<()> {
    expect_throws("java.lang.IllegalArgumentException", || {
        new_edge_ngram_tokenizer(0, 0)?.set_reader(input_reader())
    });
    Ok(())
}

fn egt_test_invalid_input2() -> Result<()> {
    expect_throws("java.lang.IllegalArgumentException", || {
        new_edge_ngram_tokenizer(2, 1)?.set_reader(input_reader())
    });
    Ok(())
}

fn egt_test_invalid_input3() -> Result<()> {
    expect_throws("java.lang.IllegalArgumentException", || {
        new_edge_ngram_tokenizer(-1, 2)?.set_reader(input_reader())
    });
    Ok(())
}

fn egt_test_front_unigram() -> Result<()> {
    let mut tokenizer = new_edge_ngram_tokenizer(1, 1)?;
    tokenizer.set_reader(input_reader())?;
    B::assert_token_stream_contents_offsets_final_offset(
        &mut tokenizer,
        &["a"],
        &[0],
        &[1],
        Some(5),
    )
}

fn egt_test_oversized_ngrams() -> Result<()> {
    let mut tokenizer = new_edge_ngram_tokenizer(6, 6)?;
    tokenizer.set_reader(input_reader())?;
    B::assert_token_stream_contents_offsets_final_offset(&mut tokenizer, &[], &[], &[], Some(5))
}

fn egt_test_front_range_of_ngrams() -> Result<()> {
    let mut tokenizer = new_edge_ngram_tokenizer(1, 3)?;
    tokenizer.set_reader(input_reader())?;
    B::assert_token_stream_contents_offsets_final_offset(
        &mut tokenizer,
        &["a", "ab", "abc"],
        &[0, 0, 0],
        &[1, 2, 3],
        Some(5),
    )
}

fn egt_test_reset() -> Result<()> {
    let mut tokenizer = new_edge_ngram_tokenizer(1, 3)?;
    tokenizer.set_reader(input_reader())?;
    B::assert_token_stream_contents_offsets_final_offset(
        &mut tokenizer,
        &["a", "ab", "abc"],
        &[0, 0, 0],
        &[1, 2, 3],
        Some(5),
    )?;
    tokenizer.set_reader(reader("abcde"))?;
    B::assert_token_stream_contents_offsets_final_offset(
        &mut tokenizer,
        &["a", "ab", "abc"],
        &[0, 0, 0],
        &[1, 2, 3],
        Some(5),
    )
}

/// blast some random strings through the analyzer
fn egt_test_random_strings() -> Result<()> {
    let num_iters = if *TEST_NIGHTLY { 10 } else { 1 };
    for _ in 0..num_iters {
        let min = TestUtil::next_int(&*random(), 2, 10);
        let max = TestUtil::next_int(&*random(), min, 20);
        let a = tokenizer_only_analyzer(
            "org.apache.lucene.analysis.ngram.TestEdgeNGramTokenizer$1",
            move || new_edge_ngram_tokenizer(min, max),
        );
        B::check_random_data_max_word_length(&*random(), &a, 100 * mult(), 20)?;
        B::check_random_data_max_word_length(&*random(), &a, 10 * mult(), 8192)?;
        a.close();
    }
    Ok(())
}

fn egt_test_tokenizer_positions() -> Result<()> {
    let mut tokenizer = new_edge_ngram_tokenizer(1, 3)?;
    tokenizer.set_reader(reader("abcde"))?;
    B::assert_token_stream_contents_graph(
        &mut tokenizer,
        &["a", "ab", "abc"],
        Some(&[0, 0, 0]),
        Some(&[1, 2, 3]),
        None,
        Some(&[1, 1, 1]),
        None,
        None,
        false,
    )
}

fn egt_test_large_input() -> Result<()> {
    ngram_large_input(true)
}

fn egt_test_large_max_gram() -> Result<()> {
    ngram_large_max_gram(true)
}

fn egt_test_pre_tokenization() -> Result<()> {
    ngram_pre_tokenization(true, "a")
}

fn egt_test_heavy_pre_tokenization() -> Result<()> {
    ngram_pre_tokenization(true, "abcdef")
}

fn egt_test_few_token_chars() -> Result<()> {
    ngram_few_token_chars(true)
}

fn egt_test_full_utf8_range() -> Result<()> {
    ngram_full_utf8_range(true)
}

fn edge_ngram_tokenizer_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.ngram.TestEdgeNGramTokenizer")
        .method(hook("setUp", ngram_tokenizer_set_up))
        .method(hook("tearDown", ngram_tokenizer_tear_down))
        .method(t("testInvalidInput", egt_test_invalid_input))
        .method(t("testInvalidInput2", egt_test_invalid_input2))
        .method(t("testInvalidInput3", egt_test_invalid_input3))
        .method(t("testFrontUnigram", egt_test_front_unigram))
        .method(t("testOversizedNgrams", egt_test_oversized_ngrams))
        .method(t("testFrontRangeOfNgrams", egt_test_front_range_of_ngrams))
        .method(t("testReset", egt_test_reset))
        .method(t("testRandomStrings", egt_test_random_strings))
        .method(t("testTokenizerPositions", egt_test_tokenizer_positions))
        .method(t("testLargeInput", egt_test_large_input))
        .method(t("testLargeMaxGram", egt_test_large_max_gram))
        .method(t("testPreTokenization", egt_test_pre_tokenization))
        .method(t(
            "testHeavyPreTokenization",
            egt_test_heavy_pre_tokenization,
        ))
        .method(t("testFewTokenChars", egt_test_few_token_chars))
        .method(t("testFullUTF8Range", egt_test_full_utf8_range))
        .build()
}

// -----------------------------------------------------------------------------
// TestShingleFilter
// -----------------------------------------------------------------------------

/// `createToken(term, start, offset, positionIncrement)`.
fn create_token(term: &str, start: i32, offset: i32, position_increment: i32) -> Token {
    Token::with_pos_inc(term, position_increment, start, offset)
}

fn test_token() -> Vec<Token> {
    vec![
        create_token("please", 0, 6, 1),
        create_token("divide", 7, 13, 1),
        create_token("this", 14, 18, 1),
        create_token("sentence", 19, 27, 1),
        create_token("into", 28, 32, 1),
        create_token("shingles", 33, 39, 1),
    ]
}

const UNIGRAM_ONLY_POSITION_INCREMENTS: &[i32] = &[1, 1, 1, 1, 1, 1];

const UNIGRAM_ONLY_TYPES: &[&str] = &["word", "word", "word", "word", "word", "word"];

fn testtokenwithholes() -> Vec<Token> {
    vec![
        create_token("please", 0, 6, 1),
        create_token("divide", 7, 13, 1),
        create_token("sentence", 19, 27, 2),
        create_token("shingles", 33, 39, 2),
    ]
}

fn bi_gram_tokens() -> Vec<Token> {
    vec![
        create_token("please", 0, 6, 1),
        create_token("please divide", 0, 13, 1),
        create_token("divide", 7, 13, 1),
        create_token("divide this", 7, 18, 1),
        create_token("this", 14, 18, 1),
        create_token("this sentence", 14, 27, 1),
        create_token("sentence", 19, 27, 1),
        create_token("sentence into", 19, 32, 1),
        create_token("into", 28, 32, 1),
        create_token("into shingles", 28, 39, 1),
        create_token("shingles", 33, 39, 1),
    ]
}

const BI_GRAM_POSITION_INCREMENTS: &[i32] = &[1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1];

const BI_GRAM_TYPES: &[&str] = &[
    "word", "shingle", "word", "shingle", "word", "shingle", "word", "shingle", "word", "shingle",
    "word",
];

fn bi_gram_tokens_with_holes() -> Vec<Token> {
    vec![
        create_token("please", 0, 6, 1),
        create_token("please divide", 0, 13, 1),
        create_token("divide", 7, 13, 1),
        create_token("divide _", 7, 19, 1),
        create_token("_ sentence", 19, 27, 1),
        create_token("sentence", 19, 27, 1),
        create_token("sentence _", 19, 33, 1),
        create_token("_ shingles", 33, 39, 1),
        create_token("shingles", 33, 39, 1),
    ]
}

const BI_GRAM_POSITION_INCREMENTS_WITH_HOLES: &[i32] = &[1, 0, 1, 0, 1, 1, 0, 1, 1];

const BI_GRAM_TYPES_WITH_HOLES: &[&str] = &[
    "word", "shingle", "word", "shingle", "shingle", "word", "shingle", "shingle", "word",
];

fn bi_gram_tokens_without_unigrams() -> Vec<Token> {
    vec![
        create_token("please divide", 0, 13, 1),
        create_token("divide this", 7, 18, 1),
        create_token("this sentence", 14, 27, 1),
        create_token("sentence into", 19, 32, 1),
        create_token("into shingles", 28, 39, 1),
    ]
}

const BI_GRAM_POSITION_INCREMENTS_WITHOUT_UNIGRAMS: &[i32] = &[1, 1, 1, 1, 1];

const BI_GRAM_TYPES_WITHOUT_UNIGRAMS: &[&str] =
    &["shingle", "shingle", "shingle", "shingle", "shingle"];

fn bi_gram_tokens_with_holes_without_unigrams() -> Vec<Token> {
    vec![
        create_token("please divide", 0, 13, 1),
        create_token("divide _", 7, 19, 1),
        create_token("_ sentence", 19, 27, 1),
        create_token("sentence _", 19, 33, 1),
        create_token("_ shingles", 33, 39, 1),
    ]
}

const BI_GRAM_POSITION_INCREMENTS_WITH_HOLES_WITHOUT_UNIGRAMS: &[i32] = &[1, 1, 1, 1, 1, 1];

fn test_single_token() -> Vec<Token> {
    vec![create_token("please", 0, 6, 1)]
}

fn single_token() -> Vec<Token> {
    vec![create_token("please", 0, 6, 1)]
}

const SINGLE_TOKEN_INCREMENTS: &[i32] = &[1];

const SINGLE_TOKEN_TYPES: &[&str] = &["word"];

fn empty_token_array() -> Vec<Token> {
    vec![]
}

const EMPTY_TOKEN_INCREMENTS_ARRAY: &[i32] = &[];

const EMPTY_TOKEN_TYPES_ARRAY: &[&str] = &[];

fn tri_gram_tokens() -> Vec<Token> {
    vec![
        create_token("please", 0, 6, 1),
        create_token("please divide", 0, 13, 1),
        create_token("please divide this", 0, 18, 1),
        create_token("divide", 7, 13, 1),
        create_token("divide this", 7, 18, 1),
        create_token("divide this sentence", 7, 27, 1),
        create_token("this", 14, 18, 1),
        create_token("this sentence", 14, 27, 1),
        create_token("this sentence into", 14, 32, 1),
        create_token("sentence", 19, 27, 1),
        create_token("sentence into", 19, 32, 1),
        create_token("sentence into shingles", 19, 39, 1),
        create_token("into", 28, 32, 1),
        create_token("into shingles", 28, 39, 1),
        create_token("shingles", 33, 39, 1),
    ]
}

const TRI_GRAM_POSITION_INCREMENTS: &[i32] = &[1, 0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 1];

const TRI_GRAM_TYPES: &[&str] = &[
    "word", "shingle", "shingle", "word", "shingle", "shingle", "word", "shingle", "shingle",
    "word", "shingle", "shingle", "word", "shingle", "word",
];

fn tri_gram_tokens_without_unigrams() -> Vec<Token> {
    vec![
        create_token("please divide", 0, 13, 1),
        create_token("please divide this", 0, 18, 1),
        create_token("divide this", 7, 18, 1),
        create_token("divide this sentence", 7, 27, 1),
        create_token("this sentence", 14, 27, 1),
        create_token("this sentence into", 14, 32, 1),
        create_token("sentence into", 19, 32, 1),
        create_token("sentence into shingles", 19, 39, 1),
        create_token("into shingles", 28, 39, 1),
    ]
}

const TRI_GRAM_POSITION_INCREMENTS_WITHOUT_UNIGRAMS: &[i32] = &[1, 0, 1, 0, 1, 0, 1, 0, 1];

const TRI_GRAM_TYPES_WITHOUT_UNIGRAMS: &[&str] = &[
    "shingle", "shingle", "shingle", "shingle", "shingle", "shingle", "shingle", "shingle",
    "shingle",
];

fn four_gram_tokens() -> Vec<Token> {
    vec![
        create_token("please", 0, 6, 1),
        create_token("please divide", 0, 13, 1),
        create_token("please divide this", 0, 18, 1),
        create_token("please divide this sentence", 0, 27, 1),
        create_token("divide", 7, 13, 1),
        create_token("divide this", 7, 18, 1),
        create_token("divide this sentence", 7, 27, 1),
        create_token("divide this sentence into", 7, 32, 1),
        create_token("this", 14, 18, 1),
        create_token("this sentence", 14, 27, 1),
        create_token("this sentence into", 14, 32, 1),
        create_token("this sentence into shingles", 14, 39, 1),
        create_token("sentence", 19, 27, 1),
        create_token("sentence into", 19, 32, 1),
        create_token("sentence into shingles", 19, 39, 1),
        create_token("into", 28, 32, 1),
        create_token("into shingles", 28, 39, 1),
        create_token("shingles", 33, 39, 1),
    ]
}

const FOUR_GRAM_POSITION_INCREMENTS: &[i32] =
    &[1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 1, 0, 1];

const FOUR_GRAM_TYPES: &[&str] = &[
    "word", "shingle", "shingle", "shingle", "word", "shingle", "shingle", "shingle", "word",
    "shingle", "shingle", "shingle", "word", "shingle", "shingle", "word", "shingle", "word",
];

fn four_gram_tokens_without_unigrams() -> Vec<Token> {
    vec![
        create_token("please divide", 0, 13, 1),
        create_token("please divide this", 0, 18, 1),
        create_token("please divide this sentence", 0, 27, 1),
        create_token("divide this", 7, 18, 1),
        create_token("divide this sentence", 7, 27, 1),
        create_token("divide this sentence into", 7, 32, 1),
        create_token("this sentence", 14, 27, 1),
        create_token("this sentence into", 14, 32, 1),
        create_token("this sentence into shingles", 14, 39, 1),
        create_token("sentence into", 19, 32, 1),
        create_token("sentence into shingles", 19, 39, 1),
        create_token("into shingles", 28, 39, 1),
    ]
}

const FOUR_GRAM_POSITION_INCREMENTS_WITHOUT_UNIGRAMS: &[i32] =
    &[1, 0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 1];

const FOUR_GRAM_TYPES_WITHOUT_UNIGRAMS: &[&str] = &[
    "shingle", "shingle", "shingle", "shingle", "shingle", "shingle", "shingle", "shingle",
    "shingle", "shingle", "shingle", "shingle",
];

fn tri_gram_tokens_min_tri_gram() -> Vec<Token> {
    vec![
        create_token("please", 0, 6, 1),
        create_token("please divide this", 0, 18, 1),
        create_token("divide", 7, 13, 1),
        create_token("divide this sentence", 7, 27, 1),
        create_token("this", 14, 18, 1),
        create_token("this sentence into", 14, 32, 1),
        create_token("sentence", 19, 27, 1),
        create_token("sentence into shingles", 19, 39, 1),
        create_token("into", 28, 32, 1),
        create_token("shingles", 33, 39, 1),
    ]
}

const TRI_GRAM_POSITION_INCREMENTS_MIN_TRI_GRAM: &[i32] = &[1, 0, 1, 0, 1, 0, 1, 0, 1, 1];

const TRI_GRAM_TYPES_MIN_TRI_GRAM: &[&str] = &[
    "word", "shingle", "word", "shingle", "word", "shingle", "word", "shingle", "word", "word",
];

fn tri_gram_tokens_without_unigrams_min_tri_gram() -> Vec<Token> {
    vec![
        create_token("please divide this", 0, 18, 1),
        create_token("divide this sentence", 7, 27, 1),
        create_token("this sentence into", 14, 32, 1),
        create_token("sentence into shingles", 19, 39, 1),
    ]
}

const TRI_GRAM_POSITION_INCREMENTS_WITHOUT_UNIGRAMS_MIN_TRI_GRAM: &[i32] = &[1, 1, 1, 1];

const TRI_GRAM_TYPES_WITHOUT_UNIGRAMS_MIN_TRI_GRAM: &[&str] =
    &["shingle", "shingle", "shingle", "shingle"];

fn four_gram_tokens_min_tri_gram() -> Vec<Token> {
    vec![
        create_token("please", 0, 6, 1),
        create_token("please divide this", 0, 18, 1),
        create_token("please divide this sentence", 0, 27, 1),
        create_token("divide", 7, 13, 1),
        create_token("divide this sentence", 7, 27, 1),
        create_token("divide this sentence into", 7, 32, 1),
        create_token("this", 14, 18, 1),
        create_token("this sentence into", 14, 32, 1),
        create_token("this sentence into shingles", 14, 39, 1),
        create_token("sentence", 19, 27, 1),
        create_token("sentence into shingles", 19, 39, 1),
        create_token("into", 28, 32, 1),
        create_token("shingles", 33, 39, 1),
    ]
}

const FOUR_GRAM_POSITION_INCREMENTS_MIN_TRI_GRAM: &[i32] = &[1, 0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 1, 1];

const FOUR_GRAM_TYPES_MIN_TRI_GRAM: &[&str] = &[
    "word", "shingle", "shingle", "word", "shingle", "shingle", "word", "shingle", "shingle",
    "word", "shingle", "word", "word",
];

fn four_gram_tokens_without_unigrams_min_tri_gram() -> Vec<Token> {
    vec![
        create_token("please divide this", 0, 18, 1),
        create_token("please divide this sentence", 0, 27, 1),
        create_token("divide this sentence", 7, 27, 1),
        create_token("divide this sentence into", 7, 32, 1),
        create_token("this sentence into", 14, 32, 1),
        create_token("this sentence into shingles", 14, 39, 1),
        create_token("sentence into shingles", 19, 39, 1),
    ]
}

const FOUR_GRAM_POSITION_INCREMENTS_WITHOUT_UNIGRAMS_MIN_TRI_GRAM: &[i32] = &[1, 0, 1, 0, 1, 0, 1];

const FOUR_GRAM_TYPES_WITHOUT_UNIGRAMS_MIN_TRI_GRAM: &[&str] = &[
    "shingle", "shingle", "shingle", "shingle", "shingle", "shingle", "shingle",
];

fn four_gram_tokens_min_four_gram() -> Vec<Token> {
    vec![
        create_token("please", 0, 6, 1),
        create_token("please divide this sentence", 0, 27, 1),
        create_token("divide", 7, 13, 1),
        create_token("divide this sentence into", 7, 32, 1),
        create_token("this", 14, 18, 1),
        create_token("this sentence into shingles", 14, 39, 1),
        create_token("sentence", 19, 27, 1),
        create_token("into", 28, 32, 1),
        create_token("shingles", 33, 39, 1),
    ]
}

const FOUR_GRAM_POSITION_INCREMENTS_MIN_FOUR_GRAM: &[i32] = &[1, 0, 1, 0, 1, 0, 1, 1, 1];

const FOUR_GRAM_TYPES_MIN_FOUR_GRAM: &[&str] = &[
    "word", "shingle", "word", "shingle", "word", "shingle", "word", "word", "word",
];

fn four_gram_tokens_without_unigrams_min_four_gram() -> Vec<Token> {
    vec![
        create_token("please divide this sentence", 0, 27, 1),
        create_token("divide this sentence into", 7, 32, 1),
        create_token("this sentence into shingles", 14, 39, 1),
    ]
}

const FOUR_GRAM_POSITION_INCREMENTS_WITHOUT_UNIGRAMS_MIN_FOUR_GRAM: &[i32] = &[1, 1, 1];

const FOUR_GRAM_TYPES_WITHOUT_UNIGRAMS_MIN_FOUR_GRAM: &[&str] = &["shingle", "shingle", "shingle"];

fn bi_gram_tokens_no_separator() -> Vec<Token> {
    vec![
        create_token("please", 0, 6, 1),
        create_token("pleasedivide", 0, 13, 1),
        create_token("divide", 7, 13, 1),
        create_token("dividethis", 7, 18, 1),
        create_token("this", 14, 18, 1),
        create_token("thissentence", 14, 27, 1),
        create_token("sentence", 19, 27, 1),
        create_token("sentenceinto", 19, 32, 1),
        create_token("into", 28, 32, 1),
        create_token("intoshingles", 28, 39, 1),
        create_token("shingles", 33, 39, 1),
    ]
}

const BI_GRAM_POSITION_INCREMENTS_NO_SEPARATOR: &[i32] = &[1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1];

const BI_GRAM_TYPES_NO_SEPARATOR: &[&str] = &[
    "word", "shingle", "word", "shingle", "word", "shingle", "word", "shingle", "word", "shingle",
    "word",
];

fn bi_gram_tokens_without_unigrams_no_separator() -> Vec<Token> {
    vec![
        create_token("pleasedivide", 0, 13, 1),
        create_token("dividethis", 7, 18, 1),
        create_token("thissentence", 14, 27, 1),
        create_token("sentenceinto", 19, 32, 1),
        create_token("intoshingles", 28, 39, 1),
    ]
}

const BI_GRAM_POSITION_INCREMENTS_WITHOUT_UNIGRAMS_NO_SEPARATOR: &[i32] = &[1, 1, 1, 1, 1];

const BI_GRAM_TYPES_WITHOUT_UNIGRAMS_NO_SEPARATOR: &[&str] =
    &["shingle", "shingle", "shingle", "shingle", "shingle"];

fn tri_gram_tokens_no_separator() -> Vec<Token> {
    vec![
        create_token("please", 0, 6, 1),
        create_token("pleasedivide", 0, 13, 1),
        create_token("pleasedividethis", 0, 18, 1),
        create_token("divide", 7, 13, 1),
        create_token("dividethis", 7, 18, 1),
        create_token("dividethissentence", 7, 27, 1),
        create_token("this", 14, 18, 1),
        create_token("thissentence", 14, 27, 1),
        create_token("thissentenceinto", 14, 32, 1),
        create_token("sentence", 19, 27, 1),
        create_token("sentenceinto", 19, 32, 1),
        create_token("sentenceintoshingles", 19, 39, 1),
        create_token("into", 28, 32, 1),
        create_token("intoshingles", 28, 39, 1),
        create_token("shingles", 33, 39, 1),
    ]
}

const TRI_GRAM_POSITION_INCREMENTS_NO_SEPARATOR: &[i32] =
    &[1, 0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 1];

const TRI_GRAM_TYPES_NO_SEPARATOR: &[&str] = &[
    "word", "shingle", "shingle", "word", "shingle", "shingle", "word", "shingle", "shingle",
    "word", "shingle", "shingle", "word", "shingle", "word",
];

fn tri_gram_tokens_without_unigrams_no_separator() -> Vec<Token> {
    vec![
        create_token("pleasedivide", 0, 13, 1),
        create_token("pleasedividethis", 0, 18, 1),
        create_token("dividethis", 7, 18, 1),
        create_token("dividethissentence", 7, 27, 1),
        create_token("thissentence", 14, 27, 1),
        create_token("thissentenceinto", 14, 32, 1),
        create_token("sentenceinto", 19, 32, 1),
        create_token("sentenceintoshingles", 19, 39, 1),
        create_token("intoshingles", 28, 39, 1),
    ]
}

const TRI_GRAM_POSITION_INCREMENTS_WITHOUT_UNIGRAMS_NO_SEPARATOR: &[i32] =
    &[1, 0, 1, 0, 1, 0, 1, 0, 1];

const TRI_GRAM_TYPES_WITHOUT_UNIGRAMS_NO_SEPARATOR: &[&str] = &[
    "shingle", "shingle", "shingle", "shingle", "shingle", "shingle", "shingle", "shingle",
    "shingle",
];

fn bi_gram_tokens_alt_separator() -> Vec<Token> {
    vec![
        create_token("please", 0, 6, 1),
        create_token("please<SEP>divide", 0, 13, 1),
        create_token("divide", 7, 13, 1),
        create_token("divide<SEP>this", 7, 18, 1),
        create_token("this", 14, 18, 1),
        create_token("this<SEP>sentence", 14, 27, 1),
        create_token("sentence", 19, 27, 1),
        create_token("sentence<SEP>into", 19, 32, 1),
        create_token("into", 28, 32, 1),
        create_token("into<SEP>shingles", 28, 39, 1),
        create_token("shingles", 33, 39, 1),
    ]
}

const BI_GRAM_POSITION_INCREMENTS_ALT_SEPARATOR: &[i32] = &[1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1];

const BI_GRAM_TYPES_ALT_SEPARATOR: &[&str] = &[
    "word", "shingle", "word", "shingle", "word", "shingle", "word", "shingle", "word", "shingle",
    "word",
];

fn bi_gram_tokens_without_unigrams_alt_separator() -> Vec<Token> {
    vec![
        create_token("please<SEP>divide", 0, 13, 1),
        create_token("divide<SEP>this", 7, 18, 1),
        create_token("this<SEP>sentence", 14, 27, 1),
        create_token("sentence<SEP>into", 19, 32, 1),
        create_token("into<SEP>shingles", 28, 39, 1),
    ]
}

const BI_GRAM_POSITION_INCREMENTS_WITHOUT_UNIGRAMS_ALT_SEPARATOR: &[i32] = &[1, 1, 1, 1, 1];

const BI_GRAM_TYPES_WITHOUT_UNIGRAMS_ALT_SEPARATOR: &[&str] =
    &["shingle", "shingle", "shingle", "shingle", "shingle"];

fn tri_gram_tokens_alt_separator() -> Vec<Token> {
    vec![
        create_token("please", 0, 6, 1),
        create_token("please<SEP>divide", 0, 13, 1),
        create_token("please<SEP>divide<SEP>this", 0, 18, 1),
        create_token("divide", 7, 13, 1),
        create_token("divide<SEP>this", 7, 18, 1),
        create_token("divide<SEP>this<SEP>sentence", 7, 27, 1),
        create_token("this", 14, 18, 1),
        create_token("this<SEP>sentence", 14, 27, 1),
        create_token("this<SEP>sentence<SEP>into", 14, 32, 1),
        create_token("sentence", 19, 27, 1),
        create_token("sentence<SEP>into", 19, 32, 1),
        create_token("sentence<SEP>into<SEP>shingles", 19, 39, 1),
        create_token("into", 28, 32, 1),
        create_token("into<SEP>shingles", 28, 39, 1),
        create_token("shingles", 33, 39, 1),
    ]
}

const TRI_GRAM_POSITION_INCREMENTS_ALT_SEPARATOR: &[i32] =
    &[1, 0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 1];

const TRI_GRAM_TYPES_ALT_SEPARATOR: &[&str] = &[
    "word", "shingle", "shingle", "word", "shingle", "shingle", "word", "shingle", "shingle",
    "word", "shingle", "shingle", "word", "shingle", "word",
];

fn tri_gram_tokens_without_unigrams_alt_separator() -> Vec<Token> {
    vec![
        create_token("please<SEP>divide", 0, 13, 1),
        create_token("please<SEP>divide<SEP>this", 0, 18, 1),
        create_token("divide<SEP>this", 7, 18, 1),
        create_token("divide<SEP>this<SEP>sentence", 7, 27, 1),
        create_token("this<SEP>sentence", 14, 27, 1),
        create_token("this<SEP>sentence<SEP>into", 14, 32, 1),
        create_token("sentence<SEP>into", 19, 32, 1),
        create_token("sentence<SEP>into<SEP>shingles", 19, 39, 1),
        create_token("into<SEP>shingles", 28, 39, 1),
    ]
}

const TRI_GRAM_POSITION_INCREMENTS_WITHOUT_UNIGRAMS_ALT_SEPARATOR: &[i32] =
    &[1, 0, 1, 0, 1, 0, 1, 0, 1];

const TRI_GRAM_TYPES_WITHOUT_UNIGRAMS_ALT_SEPARATOR: &[&str] = &[
    "shingle", "shingle", "shingle", "shingle", "shingle", "shingle", "shingle", "shingle",
    "shingle",
];

fn tri_gram_tokens_null_separator() -> Vec<Token> {
    vec![
        create_token("please", 0, 6, 1),
        create_token("pleasedivide", 0, 13, 1),
        create_token("pleasedividethis", 0, 18, 1),
        create_token("divide", 7, 13, 1),
        create_token("dividethis", 7, 18, 1),
        create_token("dividethissentence", 7, 27, 1),
        create_token("this", 14, 18, 1),
        create_token("thissentence", 14, 27, 1),
        create_token("thissentenceinto", 14, 32, 1),
        create_token("sentence", 19, 27, 1),
        create_token("sentenceinto", 19, 32, 1),
        create_token("sentenceintoshingles", 19, 39, 1),
        create_token("into", 28, 32, 1),
        create_token("intoshingles", 28, 39, 1),
        create_token("shingles", 33, 39, 1),
    ]
}

const TRI_GRAM_POSITION_INCREMENTS_NULL_SEPARATOR: &[i32] =
    &[1, 0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 1];

const TRI_GRAM_TYPES_NULL_SEPARATOR: &[&str] = &[
    "word", "shingle", "shingle", "word", "shingle", "shingle", "word", "shingle", "shingle",
    "word", "shingle", "shingle", "word", "shingle", "word",
];

fn test_token_pos_incr_equal_to_n() -> Vec<Token> {
    vec![
        create_token("please", 0, 6, 1),
        create_token("divide", 7, 13, 1),
        create_token("this", 14, 18, 1),
        create_token("sentence", 29, 37, 3),
        create_token("into", 38, 42, 1),
        create_token("shingles", 43, 49, 1),
    ]
}

fn tri_gram_tokens_pos_incr_equal_to_n() -> Vec<Token> {
    vec![
        create_token("please", 0, 6, 1),
        create_token("please divide", 0, 13, 1),
        create_token("please divide this", 0, 18, 1),
        create_token("divide", 7, 13, 1),
        create_token("divide this", 7, 18, 1),
        create_token("divide this _", 7, 29, 1),
        create_token("this", 14, 18, 1),
        create_token("this _", 14, 29, 1),
        create_token("this _ _", 14, 29, 1),
        create_token("_ _ sentence", 29, 37, 1),
        create_token("_ sentence", 29, 37, 1),
        create_token("_ sentence into", 29, 42, 1),
        create_token("sentence", 29, 37, 1),
        create_token("sentence into", 29, 42, 1),
        create_token("sentence into shingles", 29, 49, 1),
        create_token("into", 38, 42, 1),
        create_token("into shingles", 38, 49, 1),
        create_token("shingles", 43, 49, 1),
    ]
}

const TRI_GRAM_POSITION_INCREMENTS_POS_INCR_EQUAL_TO_N: &[i32] =
    &[1, 0, 0, 1, 0, 0, 1, 0, 0, 1, 1, 0, 1, 0, 0, 1, 0, 1];

const TRI_GRAM_TYPES_POS_INCR_EQUAL_TO_N: &[&str] = &[
    "word", "shingle", "shingle", "word", "shingle", "shingle", "word", "shingle", "shingle",
    "shingle", "shingle", "shingle", "word", "shingle", "shingle", "word", "shingle", "word",
];

fn tri_gram_tokens_pos_incr_equal_to_n_without_unigrams() -> Vec<Token> {
    vec![
        create_token("please divide", 0, 13, 1),
        create_token("please divide this", 0, 18, 1),
        create_token("divide this", 7, 18, 1),
        create_token("divide this _", 7, 29, 1),
        create_token("this _", 14, 29, 1),
        create_token("this _ _", 14, 29, 1),
        create_token("_ _ sentence", 29, 37, 1),
        create_token("_ sentence", 29, 37, 1),
        create_token("_ sentence into", 29, 42, 1),
        create_token("sentence into", 29, 42, 1),
        create_token("sentence into shingles", 29, 49, 1),
        create_token("into shingles", 38, 49, 1),
    ]
}

const TRI_GRAM_POSITION_INCREMENTS_POS_INCR_EQUAL_TO_N_WITHOUT_UNIGRAMS: &[i32] =
    &[1, 0, 1, 0, 1, 0, 1, 1, 0, 1, 0, 1, 1];

const TRI_GRAM_TYPES_POS_INCR_EQUAL_TO_N_WITHOUT_UNIGRAMS: &[&str] = &[
    "shingle", "shingle", "shingle", "shingle", "shingle", "shingle", "shingle", "shingle",
    "shingle", "shingle", "shingle", "shingle",
];

fn test_token_pos_incr_greater_than_n() -> Vec<Token> {
    vec![
        create_token("please", 0, 6, 1),
        create_token("divide", 57, 63, 8),
        create_token("this", 64, 68, 1),
        create_token("sentence", 69, 77, 1),
        create_token("into", 78, 82, 1),
        create_token("shingles", 83, 89, 1),
    ]
}

fn tri_gram_tokens_pos_incr_greater_than_n() -> Vec<Token> {
    vec![
        create_token("please", 0, 6, 1),
        create_token("please _", 0, 57, 1),
        create_token("please _ _", 0, 57, 1),
        create_token("_ _ divide", 57, 63, 1),
        create_token("_ divide", 57, 63, 1),
        create_token("_ divide this", 57, 68, 1),
        create_token("divide", 57, 63, 1),
        create_token("divide this", 57, 68, 1),
        create_token("divide this sentence", 57, 77, 1),
        create_token("this", 64, 68, 1),
        create_token("this sentence", 64, 77, 1),
        create_token("this sentence into", 64, 82, 1),
        create_token("sentence", 69, 77, 1),
        create_token("sentence into", 69, 82, 1),
        create_token("sentence into shingles", 69, 89, 1),
        create_token("into", 78, 82, 1),
        create_token("into shingles", 78, 89, 1),
        create_token("shingles", 83, 89, 1),
    ]
}

const TRI_GRAM_POSITION_INCREMENTS_POS_INCR_GREATER_THAN_N: &[i32] =
    &[1, 0, 0, 1, 1, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 1];

const TRI_GRAM_TYPES_POS_INCR_GREATER_THAN_N: &[&str] = &[
    "word", "shingle", "shingle", "shingle", "shingle", "shingle", "word", "shingle", "shingle",
    "word", "shingle", "shingle", "word", "shingle", "shingle", "word", "shingle", "word",
];

fn tri_gram_tokens_pos_incr_greater_than_n_without_unigrams() -> Vec<Token> {
    vec![
        create_token("please _", 0, 57, 1),
        create_token("please _ _", 0, 57, 1),
        create_token("_ _ divide", 57, 63, 1),
        create_token("_ divide", 57, 63, 1),
        create_token("_ divide this", 57, 68, 1),
        create_token("divide this", 57, 68, 1),
        create_token("divide this sentence", 57, 77, 1),
        create_token("this sentence", 64, 77, 1),
        create_token("this sentence into", 64, 82, 1),
        create_token("sentence into", 69, 82, 1),
        create_token("sentence into shingles", 69, 89, 1),
        create_token("into shingles", 78, 89, 1),
    ]
}

const TRI_GRAM_POSITION_INCREMENTS_POS_INCR_GREATER_THAN_N_WITHOUT_UNIGRAMS: &[i32] =
    &[1, 0, 1, 1, 0, 1, 0, 1, 0, 1, 0, 1];

const TRI_GRAM_TYPES_POS_INCR_GREATER_THAN_N_WITHOUT_UNIGRAMS: &[&str] = &[
    "shingle", "shingle", "shingle", "shingle", "shingle", "shingle", "shingle", "shingle",
    "shingle", "shingle", "shingle", "shingle",
];

/// `shingleFilterTestCommon(filter, tokensToCompare, positionIncrements, types)`.
fn shingle_filter_test_common(
    filter: &mut ShingleFilter,
    tokens_to_compare: &[Token],
    position_increments: &[i32],
    types: &[&str],
) -> Result<()> {
    let text: Vec<String> = tokens_to_compare.iter().map(Token::term).collect();
    let text: Vec<&str> = text.iter().map(String::as_str).collect();
    let start_offsets: Vec<i32> = tokens_to_compare.iter().map(|t| t.start_offset()).collect();
    let end_offsets: Vec<i32> = tokens_to_compare.iter().map(|t| t.end_offset()).collect();
    B::assert_token_stream_contents_types(
        filter,
        &text,
        Some(&start_offsets),
        Some(&end_offsets),
        Some(types),
        Some(position_increments),
    )
}

/// `shingleFilterTest(maxSize, tokensToShingle, tokensToCompare, positionIncrements, types, outputUnigrams)`.
fn shingle_filter_test(
    max_size: i32,
    tokens_to_shingle: &[Token],
    tokens_to_compare: &[Token],
    position_increments: &[i32],
    types: &[&str],
    output_unigrams: bool,
) -> Result<()> {
    let mut filter = new_shingle_filter_with_max(
        boxed(CannedTokenStream::new(tokens_to_shingle.to_vec())),
        max_size,
    )?;
    filter.logic_mut().set_output_unigrams(output_unigrams);
    shingle_filter_test_common(&mut filter, tokens_to_compare, position_increments, types)
}

/// `shingleFilterTest(minSize, maxSize, tokensToShingle, tokensToCompare, positionIncrements, types, outputUnigrams)`.
fn shingle_filter_test_min_max(
    min_size: i32,
    max_size: i32,
    tokens_to_shingle: &[Token],
    tokens_to_compare: &[Token],
    position_increments: &[i32],
    types: &[&str],
    output_unigrams: bool,
) -> Result<()> {
    let mut filter = new_shingle_filter(
        boxed(CannedTokenStream::new(tokens_to_shingle.to_vec())),
        min_size,
        max_size,
    )?;
    filter.logic_mut().set_output_unigrams(output_unigrams);
    shingle_filter_test_common(&mut filter, tokens_to_compare, position_increments, types)
}

/// `shingleFilterTest(minSize, maxSize, tokensToShingle, tokensToCompare,
/// positionIncrements, types, outputUnigrams, outputUnigramsIfNoShingles)`.
#[allow(clippy::too_many_arguments)]
fn shingle_filter_test_if_no_shingles(
    min_size: i32,
    max_size: i32,
    tokens_to_shingle: &[Token],
    tokens_to_compare: &[Token],
    position_increments: &[i32],
    types: &[&str],
    output_unigrams: bool,
    output_unigrams_if_no_shingles: bool,
) -> Result<()> {
    let mut filter = new_shingle_filter(
        boxed(CannedTokenStream::new(tokens_to_shingle.to_vec())),
        min_size,
        max_size,
    )?;
    filter.logic_mut().set_output_unigrams(output_unigrams);
    filter
        .logic_mut()
        .set_output_unigrams_if_no_shingles(output_unigrams_if_no_shingles);
    shingle_filter_test_common(&mut filter, tokens_to_compare, position_increments, types)
}

/// `shingleFilterTest(tokenSeparator, minSize, maxSize, tokensToShingle,
/// tokensToCompare, positionIncrements, types, outputUnigrams)`.
#[allow(clippy::too_many_arguments)]
fn shingle_filter_test_sep(
    token_separator: Option<&str>,
    min_size: i32,
    max_size: i32,
    tokens_to_shingle: &[Token],
    tokens_to_compare: &[Token],
    position_increments: &[i32],
    types: &[&str],
    output_unigrams: bool,
) -> Result<()> {
    let mut filter = new_shingle_filter(
        boxed(CannedTokenStream::new(tokens_to_shingle.to_vec())),
        min_size,
        max_size,
    )?;
    filter.logic_mut().set_token_separator(token_separator);
    filter.logic_mut().set_output_unigrams(output_unigrams);
    shingle_filter_test_common(&mut filter, tokens_to_compare, position_increments, types)
}

fn sf_test_bi_gram_filter() -> Result<()> {
    shingle_filter_test(
        2,
        &test_token(),
        &bi_gram_tokens(),
        BI_GRAM_POSITION_INCREMENTS,
        BI_GRAM_TYPES,
        true,
    )
}

fn sf_test_bi_gram_filter_with_holes() -> Result<()> {
    shingle_filter_test(
        2,
        &testtokenwithholes(),
        &bi_gram_tokens_with_holes(),
        BI_GRAM_POSITION_INCREMENTS_WITH_HOLES,
        BI_GRAM_TYPES_WITH_HOLES,
        true,
    )
}

fn sf_test_bi_gram_filter_without_unigrams() -> Result<()> {
    shingle_filter_test(
        2,
        &test_token(),
        &bi_gram_tokens_without_unigrams(),
        BI_GRAM_POSITION_INCREMENTS_WITHOUT_UNIGRAMS,
        BI_GRAM_TYPES_WITHOUT_UNIGRAMS,
        false,
    )
}

fn sf_test_bi_gram_filter_with_holes_without_unigrams() -> Result<()> {
    shingle_filter_test(
        2,
        &testtokenwithholes(),
        &bi_gram_tokens_with_holes_without_unigrams(),
        BI_GRAM_POSITION_INCREMENTS_WITH_HOLES_WITHOUT_UNIGRAMS,
        BI_GRAM_TYPES_WITHOUT_UNIGRAMS,
        false,
    )
}

fn sf_test_bi_gram_filter_with_single_token() -> Result<()> {
    shingle_filter_test(
        2,
        &test_single_token(),
        &single_token(),
        SINGLE_TOKEN_INCREMENTS,
        SINGLE_TOKEN_TYPES,
        true,
    )
}

fn sf_test_bi_gram_filter_with_single_token_without_unigrams() -> Result<()> {
    shingle_filter_test(
        2,
        &test_single_token(),
        &empty_token_array(),
        EMPTY_TOKEN_INCREMENTS_ARRAY,
        EMPTY_TOKEN_TYPES_ARRAY,
        false,
    )
}

fn sf_test_bi_gram_filter_with_empty_token_stream() -> Result<()> {
    shingle_filter_test(
        2,
        &empty_token_array(),
        &empty_token_array(),
        EMPTY_TOKEN_INCREMENTS_ARRAY,
        EMPTY_TOKEN_TYPES_ARRAY,
        true,
    )
}

fn sf_test_bi_gram_filter_with_empty_token_stream_without_unigrams() -> Result<()> {
    shingle_filter_test(
        2,
        &empty_token_array(),
        &empty_token_array(),
        EMPTY_TOKEN_INCREMENTS_ARRAY,
        EMPTY_TOKEN_TYPES_ARRAY,
        false,
    )
}

fn sf_test_tri_gram_filter() -> Result<()> {
    shingle_filter_test(
        3,
        &test_token(),
        &tri_gram_tokens(),
        TRI_GRAM_POSITION_INCREMENTS,
        TRI_GRAM_TYPES,
        true,
    )
}

fn sf_test_tri_gram_filter_without_unigrams() -> Result<()> {
    shingle_filter_test(
        3,
        &test_token(),
        &tri_gram_tokens_without_unigrams(),
        TRI_GRAM_POSITION_INCREMENTS_WITHOUT_UNIGRAMS,
        TRI_GRAM_TYPES_WITHOUT_UNIGRAMS,
        false,
    )
}

fn sf_test_four_gram_filter() -> Result<()> {
    shingle_filter_test(
        4,
        &test_token(),
        &four_gram_tokens(),
        FOUR_GRAM_POSITION_INCREMENTS,
        FOUR_GRAM_TYPES,
        true,
    )
}

fn sf_test_four_gram_filter_without_unigrams() -> Result<()> {
    shingle_filter_test(
        4,
        &test_token(),
        &four_gram_tokens_without_unigrams(),
        FOUR_GRAM_POSITION_INCREMENTS_WITHOUT_UNIGRAMS,
        FOUR_GRAM_TYPES_WITHOUT_UNIGRAMS,
        false,
    )
}

fn sf_test_tri_gram_filter_min_tri_gram() -> Result<()> {
    shingle_filter_test_min_max(
        3,
        3,
        &test_token(),
        &tri_gram_tokens_min_tri_gram(),
        TRI_GRAM_POSITION_INCREMENTS_MIN_TRI_GRAM,
        TRI_GRAM_TYPES_MIN_TRI_GRAM,
        true,
    )
}

fn sf_test_tri_gram_filter_without_unigrams_min_tri_gram() -> Result<()> {
    shingle_filter_test_min_max(
        3,
        3,
        &test_token(),
        &tri_gram_tokens_without_unigrams_min_tri_gram(),
        TRI_GRAM_POSITION_INCREMENTS_WITHOUT_UNIGRAMS_MIN_TRI_GRAM,
        TRI_GRAM_TYPES_WITHOUT_UNIGRAMS_MIN_TRI_GRAM,
        false,
    )
}

fn sf_test_four_gram_filter_min_tri_gram() -> Result<()> {
    shingle_filter_test_min_max(
        3,
        4,
        &test_token(),
        &four_gram_tokens_min_tri_gram(),
        FOUR_GRAM_POSITION_INCREMENTS_MIN_TRI_GRAM,
        FOUR_GRAM_TYPES_MIN_TRI_GRAM,
        true,
    )
}

fn sf_test_four_gram_filter_without_unigrams_min_tri_gram() -> Result<()> {
    shingle_filter_test_min_max(
        3,
        4,
        &test_token(),
        &four_gram_tokens_without_unigrams_min_tri_gram(),
        FOUR_GRAM_POSITION_INCREMENTS_WITHOUT_UNIGRAMS_MIN_TRI_GRAM,
        FOUR_GRAM_TYPES_WITHOUT_UNIGRAMS_MIN_TRI_GRAM,
        false,
    )
}

fn sf_test_four_gram_filter_min_four_gram() -> Result<()> {
    shingle_filter_test_min_max(
        4,
        4,
        &test_token(),
        &four_gram_tokens_min_four_gram(),
        FOUR_GRAM_POSITION_INCREMENTS_MIN_FOUR_GRAM,
        FOUR_GRAM_TYPES_MIN_FOUR_GRAM,
        true,
    )
}

fn sf_test_four_gram_filter_without_unigrams_min_four_gram() -> Result<()> {
    shingle_filter_test_min_max(
        4,
        4,
        &test_token(),
        &four_gram_tokens_without_unigrams_min_four_gram(),
        FOUR_GRAM_POSITION_INCREMENTS_WITHOUT_UNIGRAMS_MIN_FOUR_GRAM,
        FOUR_GRAM_TYPES_WITHOUT_UNIGRAMS_MIN_FOUR_GRAM,
        false,
    )
}

fn sf_test_bi_gram_filter_no_separator() -> Result<()> {
    shingle_filter_test_sep(
        Some(""),
        2,
        2,
        &test_token(),
        &bi_gram_tokens_no_separator(),
        BI_GRAM_POSITION_INCREMENTS_NO_SEPARATOR,
        BI_GRAM_TYPES_NO_SEPARATOR,
        true,
    )
}

fn sf_test_bi_gram_filter_without_unigrams_no_separator() -> Result<()> {
    shingle_filter_test_sep(
        Some(""),
        2,
        2,
        &test_token(),
        &bi_gram_tokens_without_unigrams_no_separator(),
        BI_GRAM_POSITION_INCREMENTS_WITHOUT_UNIGRAMS_NO_SEPARATOR,
        BI_GRAM_TYPES_WITHOUT_UNIGRAMS_NO_SEPARATOR,
        false,
    )
}

fn sf_test_tri_gram_filter_no_separator() -> Result<()> {
    shingle_filter_test_sep(
        Some(""),
        2,
        3,
        &test_token(),
        &tri_gram_tokens_no_separator(),
        TRI_GRAM_POSITION_INCREMENTS_NO_SEPARATOR,
        TRI_GRAM_TYPES_NO_SEPARATOR,
        true,
    )
}

fn sf_test_tri_gram_filter_without_unigrams_no_separator() -> Result<()> {
    shingle_filter_test_sep(
        Some(""),
        2,
        3,
        &test_token(),
        &tri_gram_tokens_without_unigrams_no_separator(),
        TRI_GRAM_POSITION_INCREMENTS_WITHOUT_UNIGRAMS_NO_SEPARATOR,
        TRI_GRAM_TYPES_WITHOUT_UNIGRAMS_NO_SEPARATOR,
        false,
    )
}

fn sf_test_bi_gram_filter_alt_separator() -> Result<()> {
    shingle_filter_test_sep(
        Some("<SEP>"),
        2,
        2,
        &test_token(),
        &bi_gram_tokens_alt_separator(),
        BI_GRAM_POSITION_INCREMENTS_ALT_SEPARATOR,
        BI_GRAM_TYPES_ALT_SEPARATOR,
        true,
    )
}

fn sf_test_bi_gram_filter_without_unigrams_alt_separator() -> Result<()> {
    shingle_filter_test_sep(
        Some("<SEP>"),
        2,
        2,
        &test_token(),
        &bi_gram_tokens_without_unigrams_alt_separator(),
        BI_GRAM_POSITION_INCREMENTS_WITHOUT_UNIGRAMS_ALT_SEPARATOR,
        BI_GRAM_TYPES_WITHOUT_UNIGRAMS_ALT_SEPARATOR,
        false,
    )
}

fn sf_test_tri_gram_filter_alt_separator() -> Result<()> {
    shingle_filter_test_sep(
        Some("<SEP>"),
        2,
        3,
        &test_token(),
        &tri_gram_tokens_alt_separator(),
        TRI_GRAM_POSITION_INCREMENTS_ALT_SEPARATOR,
        TRI_GRAM_TYPES_ALT_SEPARATOR,
        true,
    )
}

fn sf_test_tri_gram_filter_without_unigrams_alt_separator() -> Result<()> {
    shingle_filter_test_sep(
        Some("<SEP>"),
        2,
        3,
        &test_token(),
        &tri_gram_tokens_without_unigrams_alt_separator(),
        TRI_GRAM_POSITION_INCREMENTS_WITHOUT_UNIGRAMS_ALT_SEPARATOR,
        TRI_GRAM_TYPES_WITHOUT_UNIGRAMS_ALT_SEPARATOR,
        false,
    )
}

fn sf_test_tri_gram_filter_null_separator() -> Result<()> {
    shingle_filter_test_sep(
        None,
        2,
        3,
        &test_token(),
        &tri_gram_tokens_null_separator(),
        TRI_GRAM_POSITION_INCREMENTS_NULL_SEPARATOR,
        TRI_GRAM_TYPES_NULL_SEPARATOR,
        true,
    )
}

fn sf_test_position_increment_equal_to_n() -> Result<()> {
    shingle_filter_test_min_max(
        2,
        3,
        &test_token_pos_incr_equal_to_n(),
        &tri_gram_tokens_pos_incr_equal_to_n(),
        TRI_GRAM_POSITION_INCREMENTS_POS_INCR_EQUAL_TO_N,
        TRI_GRAM_TYPES_POS_INCR_EQUAL_TO_N,
        true,
    )
}

fn sf_test_position_increment_equal_to_nwithout_unigrams() -> Result<()> {
    shingle_filter_test_min_max(
        2,
        3,
        &test_token_pos_incr_equal_to_n(),
        &tri_gram_tokens_pos_incr_equal_to_n_without_unigrams(),
        TRI_GRAM_POSITION_INCREMENTS_POS_INCR_EQUAL_TO_N_WITHOUT_UNIGRAMS,
        TRI_GRAM_TYPES_POS_INCR_EQUAL_TO_N_WITHOUT_UNIGRAMS,
        false,
    )
}

fn sf_test_position_increment_greater_than_n() -> Result<()> {
    shingle_filter_test_min_max(
        2,
        3,
        &test_token_pos_incr_greater_than_n(),
        &tri_gram_tokens_pos_incr_greater_than_n(),
        TRI_GRAM_POSITION_INCREMENTS_POS_INCR_GREATER_THAN_N,
        TRI_GRAM_TYPES_POS_INCR_GREATER_THAN_N,
        true,
    )
}

fn sf_test_position_increment_greater_than_nwithout_unigrams() -> Result<()> {
    shingle_filter_test_min_max(
        2,
        3,
        &test_token_pos_incr_greater_than_n(),
        &tri_gram_tokens_pos_incr_greater_than_n_without_unigrams(),
        TRI_GRAM_POSITION_INCREMENTS_POS_INCR_GREATER_THAN_N_WITHOUT_UNIGRAMS,
        TRI_GRAM_TYPES_POS_INCR_GREATER_THAN_N_WITHOUT_UNIGRAMS,
        false,
    )
}

fn sf_test_reset() -> Result<()> {
    let ws_tokenizer = own(new_whitespace_tokenizer());
    ws_tokenizer
        .borrow_mut()
        .set_reader(reader("please divide this sentence"))?;
    let mut filter = new_shingle_filter_with_max(shared(&ws_tokenizer), 2)?;
    let types = [
        DEFAULT_TYPE,
        "shingle",
        DEFAULT_TYPE,
        "shingle",
        DEFAULT_TYPE,
        "shingle",
        DEFAULT_TYPE,
    ];
    let terms = [
        "please",
        "please divide",
        "divide",
        "divide this",
        "this",
        "this sentence",
        "sentence",
    ];
    B::assert_token_stream_contents_types(
        &mut filter,
        &terms,
        Some(&[0, 0, 7, 7, 14, 14, 19]),
        Some(&[6, 13, 13, 18, 18, 27, 27]),
        Some(&types),
        Some(&[1, 0, 1, 0, 1, 0, 1]),
    )?;
    ws_tokenizer
        .borrow_mut()
        .set_reader(reader("please divide this sentence"))?;
    B::assert_token_stream_contents_types(
        &mut filter,
        &terms,
        Some(&[0, 0, 7, 7, 14, 14, 19]),
        Some(&[6, 13, 13, 18, 18, 27, 27]),
        Some(&types),
        Some(&[1, 0, 1, 0, 1, 0, 1]),
    )
}

fn sf_test_output_unigrams_if_no_shingles_single_token_case() -> Result<()> {
    // Single token input with outputUnigrams==false is the primary case where
    // enabling this option should alter program behavior.
    shingle_filter_test_if_no_shingles(
        2,
        2,
        &test_single_token(),
        &single_token(),
        SINGLE_TOKEN_INCREMENTS,
        SINGLE_TOKEN_TYPES,
        false,
        true,
    )
}

fn sf_test_output_unigrams_if_no_shingles_with_simple_bigram() -> Result<()> {
    // Here we expect the same result as with testBiGramFilter().
    shingle_filter_test_if_no_shingles(
        2,
        2,
        &test_token(),
        &bi_gram_tokens(),
        BI_GRAM_POSITION_INCREMENTS,
        BI_GRAM_TYPES,
        true,
        true,
    )
}

fn sf_test_output_unigrams_if_no_shingles_with_simple_unigramless_bigram() -> Result<()> {
    // Here we expect the same result as with testBiGramFilterWithoutUnigrams().
    shingle_filter_test_if_no_shingles(
        2,
        2,
        &test_token(),
        &bi_gram_tokens_without_unigrams(),
        BI_GRAM_POSITION_INCREMENTS_WITHOUT_UNIGRAMS,
        BI_GRAM_TYPES_WITHOUT_UNIGRAMS,
        false,
        true,
    )
}

fn sf_test_output_unigrams_if_no_shingles_with_multiple_input_tokens() -> Result<()> {
    // Test when the minimum shingle size is greater than the number of input tokens
    shingle_filter_test_if_no_shingles(
        7,
        7,
        &test_token(),
        &test_token(),
        UNIGRAM_ONLY_POSITION_INCREMENTS,
        UNIGRAM_ONLY_TYPES,
        false,
        true,
    )
}

fn sf_test_random_strings() -> Result<()> {
    let a = whitespace_analyzer(
        "org.apache.lucene.analysis.shingle.TestShingleFilter$1",
        |t| Ok(boxed(new_shingle_filter_default(t)?)),
    );
    B::check_random_data(&*random(), &a, 200 * mult())?;
    a.close();
    Ok(())
}

fn sf_test_random_huge_strings() -> Result<()> {
    let random = random();
    let a = support::mock_tokenizer_analyzer_max(
        "org.apache.lucene.analysis.shingle.TestShingleFilter$2",
        Arc::clone(&WHITESPACE),
        false,
        (MAX_TERM_LENGTH / 2) as i32,
        |t| Ok(boxed(new_shingle_filter_default(t)?)),
    );
    B::check_random_data_max_word_length(&*random, &a, 3 * mult(), 8192)?;
    a.close();
    Ok(())
}

fn sf_test_empty_term() -> Result<()> {
    let a = keyword_tokenizer_analyzer(
        "org.apache.lucene.analysis.shingle.TestShingleFilter$3",
        |t| Ok(boxed(new_shingle_filter_default(t)?)),
    );
    B::check_one_term(&*a, "", "")?;
    a.close();
    Ok(())
}

fn sf_test_trailing_hole1() -> Result<()> {
    // Analyzing "wizard of", where of is removed as a
    // stopword leaving a trailing hole:
    let input_tokens = vec![create_token("wizard", 0, 6, 1)];
    let mut filter = new_shingle_filter(
        boxed(CannedTokenStream::with_final(1, 9, input_tokens)),
        2,
        2,
    )?;
    B::assert_token_stream_contents_offsets_pos_increments_final_offset(
        &mut filter,
        &["wizard", "wizard _"],
        &[0, 0],
        &[6, 9],
        &[1, 0],
        Some(9),
    )
}

fn sf_test_trailing_hole2() -> Result<()> {
    // Analyzing "purple wizard of", where of is removed as a
    // stopword leaving a trailing hole:
    let input_tokens = vec![
        create_token("purple", 0, 6, 1),
        create_token("wizard", 7, 13, 1),
    ];
    let mut filter = new_shingle_filter(
        boxed(CannedTokenStream::with_final(1, 16, input_tokens)),
        2,
        2,
    )?;
    B::assert_token_stream_contents_offsets_pos_increments_final_offset(
        &mut filter,
        &["purple", "purple wizard", "wizard", "wizard _"],
        &[0, 0, 7, 7],
        &[6, 13, 13, 16],
        &[1, 0, 1, 0],
        Some(16),
    )
}

fn sf_test_two_trailing_holes() -> Result<()> {
    // Analyzing "purple wizard of the", where of and the are removed as a
    // stopwords, leaving two trailing holes:
    let input_tokens = vec![
        create_token("purple", 0, 6, 1),
        create_token("wizard", 7, 13, 1),
    ];
    let mut filter = new_shingle_filter(
        boxed(CannedTokenStream::with_final(2, 20, input_tokens)),
        2,
        2,
    )?;
    B::assert_token_stream_contents_offsets_pos_increments_final_offset(
        &mut filter,
        &["purple", "purple wizard", "wizard", "wizard _"],
        &[0, 0, 7, 7],
        &[6, 13, 13, 20],
        &[1, 0, 1, 0],
        Some(20),
    )
}

fn sf_test_two_trailing_holes_tri_shingle() -> Result<()> {
    // Analyzing "purple wizard of the", where of and the are removed as a
    // stopwords, leaving two trailing holes:
    let input_tokens = vec![
        create_token("purple", 0, 6, 1),
        create_token("wizard", 7, 13, 1),
    ];
    let mut filter = new_shingle_filter(
        boxed(CannedTokenStream::with_final(2, 20, input_tokens)),
        2,
        3,
    )?;
    B::assert_token_stream_contents_offsets_pos_increments_final_offset(
        &mut filter,
        &[
            "purple",
            "purple wizard",
            "purple wizard _",
            "wizard",
            "wizard _",
            "wizard _ _",
        ],
        &[0, 0, 0, 7, 7, 7],
        &[6, 13, 20, 13, 20, 20],
        &[1, 0, 0, 1, 0, 0],
        Some(20),
    )
}

fn sf_test_two_trailing_holes_tri_shingle_with_token_filler() -> Result<()> {
    // Analyzing "purple wizard of the", where of and the are removed as a
    // stopwords, leaving two trailing holes:
    let input_tokens = || {
        vec![
            create_token("purple", 0, 6, 1),
            create_token("wizard", 7, 13, 1),
        ]
    };
    let check =
        |filler: Option<&str>, separator: Option<Option<&str>>, out: &[&str]| -> Result<()> {
            let mut filter = new_shingle_filter(
                boxed(CannedTokenStream::with_final(2, 20, input_tokens())),
                2,
                3,
            )?;
            filter.logic_mut().set_filler_token(filler);
            if let Some(separator) = separator {
                filter.logic_mut().set_token_separator(separator);
            }
            B::assert_token_stream_contents_offsets_pos_increments_final_offset(
                &mut filter,
                out,
                &[0, 0, 0, 7, 7, 7],
                &[6, 13, 20, 13, 20, 20],
                &[1, 0, 0, 1, 0, 0],
                Some(20),
            )
        };
    check(
        Some("--"),
        None,
        &[
            "purple",
            "purple wizard",
            "purple wizard --",
            "wizard",
            "wizard --",
            "wizard -- --",
        ],
    )?;
    check(
        Some(""),
        None,
        &[
            "purple",
            "purple wizard",
            "purple wizard ",
            "wizard",
            "wizard ",
            "wizard  ",
        ],
    )?;
    check(
        None,
        None,
        &[
            "purple",
            "purple wizard",
            "purple wizard ",
            "wizard",
            "wizard ",
            "wizard  ",
        ],
    )?;
    check(
        None,
        Some(None),
        &[
            "purple",
            "purplewizard",
            "purplewizard",
            "wizard",
            "wizard",
            "wizard",
        ],
    )
}

fn sf_test_position_length() -> Result<()> {
    let analyzer = |min: i32, max: i32, name: &'static str| {
        whitespace_analyzer(name, move |t| {
            let mut filter = new_shingle_filter(t, min, max)?;
            filter.logic_mut().set_output_unigrams(false);
            Ok(boxed(filter))
        })
    };
    let check = |a: &Arc<dyn Analyzer>,
                 out: &[&str],
                 starts: &[i32],
                 ends: &[i32],
                 incs: &[i32],
                 lens: &[i32]|
     -> Result<()> {
        let ts = a.token_stream("", Box::new(StringCharReader::new("to be or not to be")))?;
        let mut ts = ts.borrow_mut();
        B::assert_token_stream_contents_graph(
            &mut *ts,
            out,
            Some(starts),
            Some(ends),
            None,
            Some(incs),
            Some(lens),
            Some(18),
            // offsets are correct but assertTokenStreamContents does not handle multiple terms with
            // different offsets
            // finishing at the same position
            false,
        )
    };
    let a = analyzer(
        4,
        4,
        "org.apache.lucene.analysis.shingle.TestShingleFilter$4",
    );
    check(
        &a,
        &["to be or not", "be or not to", "or not to be"],
        &[0, 3, 6],
        &[12, 15, 18],
        &[1, 1, 1],
        &[1, 1, 1],
    )?;
    let a = analyzer(
        2,
        4,
        "org.apache.lucene.analysis.shingle.TestShingleFilter$5",
    );
    check(
        &a,
        &[
            "to be",
            "to be or",
            "to be or not",
            "be or",
            "be or not",
            "be or not to",
            "or not",
            "or not to",
            "or not to be",
            "not to",
            "not to be",
            "to be",
        ],
        &[0, 0, 0, 3, 3, 3, 6, 6, 6, 9, 9, 13],
        &[5, 8, 12, 8, 12, 15, 12, 15, 18, 15, 18, 18],
        &[1, 0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 1],
        &[1, 2, 3, 1, 2, 3, 1, 2, 3, 1, 2, 1],
    )?;
    let a = analyzer(
        3,
        4,
        "org.apache.lucene.analysis.shingle.TestShingleFilter$6",
    );
    check(
        &a,
        &[
            "to be or",
            "to be or not",
            "be or not",
            "be or not to",
            "or not to",
            "or not to be",
            "not to be",
        ],
        &[0, 0, 3, 3, 6, 6, 9],
        &[8, 12, 12, 15, 15, 18, 18],
        &[1, 0, 1, 0, 1, 0, 1, 0],
        &[1, 2, 1, 2, 1, 2, 1, 2],
    )?;
    let a = analyzer(
        3,
        5,
        "org.apache.lucene.analysis.shingle.TestShingleFilter$7",
    );
    check(
        &a,
        &[
            "to be or",
            "to be or not",
            "to be or not to",
            "be or not",
            "be or not to",
            "be or not to be",
            "or not to",
            "or not to be",
            "not to be",
        ],
        &[0, 0, 0, 3, 3, 3, 6, 6, 9, 9],
        &[8, 12, 15, 12, 15, 18, 15, 18, 18],
        &[1, 0, 0, 1, 0, 0, 1, 0, 1, 0],
        &[1, 2, 3, 1, 2, 3, 1, 2, 1],
    )
}
fn shingle_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.shingle.TestShingleFilter")
        .method(t("testBiGramFilter", sf_test_bi_gram_filter))
        .method(t(
            "testBiGramFilterWithHoles",
            sf_test_bi_gram_filter_with_holes,
        ))
        .method(t(
            "testBiGramFilterWithoutUnigrams",
            sf_test_bi_gram_filter_without_unigrams,
        ))
        .method(t(
            "testBiGramFilterWithHolesWithoutUnigrams",
            sf_test_bi_gram_filter_with_holes_without_unigrams,
        ))
        .method(t(
            "testBiGramFilterWithSingleToken",
            sf_test_bi_gram_filter_with_single_token,
        ))
        .method(t(
            "testBiGramFilterWithSingleTokenWithoutUnigrams",
            sf_test_bi_gram_filter_with_single_token_without_unigrams,
        ))
        .method(t(
            "testBiGramFilterWithEmptyTokenStream",
            sf_test_bi_gram_filter_with_empty_token_stream,
        ))
        .method(t(
            "testBiGramFilterWithEmptyTokenStreamWithoutUnigrams",
            sf_test_bi_gram_filter_with_empty_token_stream_without_unigrams,
        ))
        .method(t("testTriGramFilter", sf_test_tri_gram_filter))
        .method(t(
            "testTriGramFilterWithoutUnigrams",
            sf_test_tri_gram_filter_without_unigrams,
        ))
        .method(t("testFourGramFilter", sf_test_four_gram_filter))
        .method(t(
            "testFourGramFilterWithoutUnigrams",
            sf_test_four_gram_filter_without_unigrams,
        ))
        .method(t(
            "testTriGramFilterMinTriGram",
            sf_test_tri_gram_filter_min_tri_gram,
        ))
        .method(t(
            "testTriGramFilterWithoutUnigramsMinTriGram",
            sf_test_tri_gram_filter_without_unigrams_min_tri_gram,
        ))
        .method(t(
            "testFourGramFilterMinTriGram",
            sf_test_four_gram_filter_min_tri_gram,
        ))
        .method(t(
            "testFourGramFilterWithoutUnigramsMinTriGram",
            sf_test_four_gram_filter_without_unigrams_min_tri_gram,
        ))
        .method(t(
            "testFourGramFilterMinFourGram",
            sf_test_four_gram_filter_min_four_gram,
        ))
        .method(t(
            "testFourGramFilterWithoutUnigramsMinFourGram",
            sf_test_four_gram_filter_without_unigrams_min_four_gram,
        ))
        .method(t(
            "testBiGramFilterNoSeparator",
            sf_test_bi_gram_filter_no_separator,
        ))
        .method(t(
            "testBiGramFilterWithoutUnigramsNoSeparator",
            sf_test_bi_gram_filter_without_unigrams_no_separator,
        ))
        .method(t(
            "testTriGramFilterNoSeparator",
            sf_test_tri_gram_filter_no_separator,
        ))
        .method(t(
            "testTriGramFilterWithoutUnigramsNoSeparator",
            sf_test_tri_gram_filter_without_unigrams_no_separator,
        ))
        .method(t(
            "testBiGramFilterAltSeparator",
            sf_test_bi_gram_filter_alt_separator,
        ))
        .method(t(
            "testBiGramFilterWithoutUnigramsAltSeparator",
            sf_test_bi_gram_filter_without_unigrams_alt_separator,
        ))
        .method(t(
            "testTriGramFilterAltSeparator",
            sf_test_tri_gram_filter_alt_separator,
        ))
        .method(t(
            "testTriGramFilterWithoutUnigramsAltSeparator",
            sf_test_tri_gram_filter_without_unigrams_alt_separator,
        ))
        .method(t(
            "testTriGramFilterNullSeparator",
            sf_test_tri_gram_filter_null_separator,
        ))
        .method(t(
            "testPositionIncrementEqualToN",
            sf_test_position_increment_equal_to_n,
        ))
        .method(t(
            "testPositionIncrementEqualToNWithoutUnigrams",
            sf_test_position_increment_equal_to_nwithout_unigrams,
        ))
        .method(t(
            "testPositionIncrementGreaterThanN",
            sf_test_position_increment_greater_than_n,
        ))
        .method(t(
            "testPositionIncrementGreaterThanNWithoutUnigrams",
            sf_test_position_increment_greater_than_nwithout_unigrams,
        ))
        .method(t("testReset", sf_test_reset))
        .method(t(
            "testOutputUnigramsIfNoShinglesSingleTokenCase",
            sf_test_output_unigrams_if_no_shingles_single_token_case,
        ))
        .method(t(
            "testOutputUnigramsIfNoShinglesWithSimpleBigram",
            sf_test_output_unigrams_if_no_shingles_with_simple_bigram,
        ))
        .method(t(
            "testOutputUnigramsIfNoShinglesWithSimpleUnigramlessBigram",
            sf_test_output_unigrams_if_no_shingles_with_simple_unigramless_bigram,
        ))
        .method(t(
            "testOutputUnigramsIfNoShinglesWithMultipleInputTokens",
            sf_test_output_unigrams_if_no_shingles_with_multiple_input_tokens,
        ))
        .method(t("testRandomStrings", sf_test_random_strings))
        .method(t("testRandomHugeStrings", sf_test_random_huge_strings))
        .method(t("testEmptyTerm", sf_test_empty_term))
        .method(t("testTrailingHole1", sf_test_trailing_hole1))
        .method(t("testTrailingHole2", sf_test_trailing_hole2))
        .method(t("testTwoTrailingHoles", sf_test_two_trailing_holes))
        .method(t(
            "testTwoTrailingHolesTriShingle",
            sf_test_two_trailing_holes_tri_shingle,
        ))
        .method(t(
            "testTwoTrailingHolesTriShingleWithTokenFiller",
            sf_test_two_trailing_holes_tri_shingle_with_token_filler,
        ))
        .method(t("testPositionLength", sf_test_position_length))
        .build()
}

// -----------------------------------------------------------------------------
// TestFixedShingleFilter
// -----------------------------------------------------------------------------

fn fsf_test_bi_gram_filter() -> Result<()> {
    let ts: Box<dyn TokenStream> = boxed(CannedTokenStream::new(vec![
        Token::with_offsets("please", 0, 6),
        Token::with_offsets("divide", 7, 13),
        Token::with_offsets("this", 14, 18),
        Token::with_offsets("sentence", 19, 27),
        Token::with_offsets("into", 28, 32),
        Token::with_offsets("shingles", 33, 41),
    ]));
    B::assert_token_stream_contents_types_pos_lengths(
        &mut (FixedShingleFilter::new(ts, 2)?),
        &[
            "please divide",
            "divide this",
            "this sentence",
            "sentence into",
            "into shingles",
        ],
        Some(&[0, 7, 14, 19, 28]),
        Some(&[13, 18, 27, 32, 41]),
        Some(&["shingle", "shingle", "shingle", "shingle", "shingle"]),
        Some(&[1, 1, 1, 1, 1]),
        Some(&[1, 1, 1, 1, 1]),
    )?;
    Ok(())
}

fn fsf_test_bi_gram_filter_with_alt_separator() -> Result<()> {
    let ts: Box<dyn TokenStream> = boxed(CannedTokenStream::new(vec![
        Token::with_offsets("please", 0, 6),
        Token::with_offsets("divide", 7, 13),
        Token::with_offsets("this", 14, 18),
        Token::with_offsets("sentence", 19, 27),
        Token::with_offsets("into", 28, 32),
        Token::with_offsets("shingles", 33, 41),
    ]));
    B::assert_token_stream_contents_types(
        &mut (FixedShingleFilter::with_separators(ts, 2, "<SEP>", "_")?),
        &[
            "please<SEP>divide",
            "divide<SEP>this",
            "this<SEP>sentence",
            "sentence<SEP>into",
            "into<SEP>shingles",
        ],
        Some(&[0, 7, 14, 19, 28]),
        Some(&[13, 18, 27, 32, 41]),
        Some(&["shingle", "shingle", "shingle", "shingle", "shingle"]),
        Some(&[1, 1, 1, 1, 1]),
    )?;
    Ok(())
}

fn fsf_test_tri_gram_filter() -> Result<()> {
    let ts: Box<dyn TokenStream> = boxed(CannedTokenStream::new(vec![
        Token::with_offsets("please", 0, 6),
        Token::with_offsets("divide", 7, 13),
        Token::with_offsets("this", 14, 18),
        Token::with_offsets("sentence", 19, 27),
        Token::with_offsets("into", 28, 32),
        Token::with_offsets("shingles", 33, 41),
    ]));
    B::assert_token_stream_contents(
        &mut (FixedShingleFilter::new(ts, 3)?),
        &[
            "please divide this",
            "divide this sentence",
            "this sentence into",
            "sentence into shingles",
        ],
    )?;
    Ok(())
}

fn fsf_test_shingle_size_greater_than_tokenstream_length() -> Result<()> {
    let mut ts = FixedShingleFilter::new(
        boxed(CannedTokenStream::new(vec![
            Token::with_offsets("please", 0, 6),
            Token::with_offsets("divide", 7, 13),
        ])),
        3,
    )?;
    ts.reset()?;
    Assert::assert_false(ts.increment_token()?);
    Ok(())
}

fn fsf_test_with_stopwords() -> Result<()> {
    let ts: Box<dyn TokenStream> = boxed(CannedTokenStream::new(vec![
        Token::with_offsets("please", 0, 6),
        Token::with_offsets("divide", 7, 13),
        Token::with_pos_inc("sentence", 2, 19, 27),
        Token::with_pos_inc("shingles", 2, 33, 41),
    ]));
    B::assert_token_stream_contents_types(
        &mut (FixedShingleFilter::new(ts, 3)?),
        &[
            "please divide _",
            "divide _ sentence",
            "sentence _ shingles",
        ],
        Some(&[0, 7, 19]),
        Some(&[13, 27, 41]),
        Some(&["shingle", "shingle", "shingle"]),
        Some(&[1, 1, 2]),
    )?;
    Ok(())
}

fn fsf_test_consecutive_stopwords() -> Result<()> {
    let ts: Box<dyn TokenStream> = boxed(CannedTokenStream::new(vec![
        Token::with_pos_inc("b", 2, 2, 3),
        Token::with_offsets("c", 4, 5),
        Token::with_offsets("d", 6, 7),
        Token::with_pos_inc("b", 3, 12, 13),
        Token::with_offsets("c", 14, 15),
    ]));
    B::assert_token_stream_contents_offsets_pos_increments(
        &mut (FixedShingleFilter::new(ts, 4)?),
        &["b c d _", "c d _ _", "d _ _ b"],
        &[2, 4, 6],
        &[7, 7, 13],
        &[2, 1, 1],
    )?;
    Ok(())
}

fn fsf_test_trailing_stopwords() -> Result<()> {
    let ts: Box<dyn TokenStream> = boxed(CannedTokenStream::with_final(
        1,
        7,
        vec![
            Token::with_offsets("b", 0, 1),
            Token::with_offsets("c", 2, 3),
            Token::with_offsets("d", 4, 5),
        ],
    ));
    B::assert_token_stream_contents_offsets_pos_increments(
        &mut (FixedShingleFilter::new(ts, 3)?),
        &["b c d", "c d _"],
        &[0, 2],
        &[5, 5],
        &[1, 1],
    )?;
    Ok(())
}

fn fsf_test_multiple_trailing_stopwords() -> Result<()> {
    let ts: Box<dyn TokenStream> = boxed(CannedTokenStream::with_final(
        2,
        9,
        vec![
            Token::with_offsets("b", 0, 1),
            Token::with_offsets("c", 2, 3),
            Token::with_offsets("d", 4, 5),
        ],
    ));
    B::assert_token_stream_contents_offsets_pos_increments(
        &mut (FixedShingleFilter::new(ts, 3)?),
        &["b c d", "c d _", "d _ _"],
        &[0, 2, 4],
        &[5, 5, 5],
        &[1, 1, 1],
    )?;
    Ok(())
}

fn fsf_test_incoming_graphs() -> Result<()> {
    // b/a c b/a d
    let ts: Box<dyn TokenStream> = boxed(CannedTokenStream::new(vec![
        Token::with_offsets("b", 0, 1),
        Token::with_pos_inc("a", 0, 0, 1),
        Token::with_offsets("c", 2, 3),
        Token::with_offsets("b", 4, 5),
        Token::with_pos_inc("a", 0, 4, 5),
        Token::with_offsets("d", 6, 7),
    ]));
    B::assert_token_stream_contents_offsets_pos_increments(
        &mut (FixedShingleFilter::new(ts, 2)?),
        &["b c", "a c", "c b", "c a", "b d", "a d"],
        &[0, 0, 2, 2, 4, 4],
        &[3, 3, 5, 5, 7, 7],
        &[1, 0, 1, 0, 1, 0],
    )?;
    Ok(())
}

fn fsf_test_shingles_spanning_graphs() -> Result<()> {
    let ts: Box<dyn TokenStream> = boxed(CannedTokenStream::new(vec![
        Token::with_offsets("b", 0, 1),
        Token::with_pos_inc("a", 0, 0, 1),
        Token::with_offsets("c", 2, 3),
        Token::with_offsets("b", 4, 5),
        Token::with_pos_inc("a", 0, 4, 5),
        Token::with_offsets("d", 6, 7),
    ]));
    B::assert_token_stream_contents_offsets_pos_increments(
        &mut (FixedShingleFilter::new(ts, 3)?),
        &["b c b", "b c a", "a c b", "a c a", "c b d", "c a d"],
        &[0, 0, 0, 0, 2, 2],
        &[5, 5, 5, 5, 7, 7],
        &[1, 0, 0, 0, 1, 0],
    )?;
    Ok(())
}

fn fsf_test_trailing_graphs_of_differing_lengths() -> Result<()> {
    // a b:3/c d e f
    let ts: Box<dyn TokenStream> = boxed(CannedTokenStream::new(vec![
        Token::with_offsets("a", 0, 1),
        Token::with_pos_len("b", 1, 2, 3, 3),
        Token::with_pos_inc("c", 0, 2, 3),
        Token::with_offsets("d", 2, 3),
        Token::with_offsets("e", 2, 3),
        Token::with_offsets("f", 4, 5),
    ]));
    B::assert_token_stream_contents(
        &mut (FixedShingleFilter::new(ts, 3)?),
        &["a b f", "a c d", "c d e", "d e f"],
    )?;
    Ok(())
}

fn fsf_test_parameter_limits() -> Result<()> {
    let e = expect_throws("java.lang.IllegalArgumentException", || {
        FixedShingleFilter::new(boxed(CannedTokenStream::new(vec![])), 1)
    });
    Assert::assert_equals(
        "Shingle size must be between 2 and 4, got 1",
        e.message().unwrap_or("null"),
    );
    let e2 = expect_throws("java.lang.IllegalArgumentException", || {
        FixedShingleFilter::new(boxed(CannedTokenStream::new(vec![])), 5)
    });
    Assert::assert_equals(
        "Shingle size must be between 2 and 4, got 5",
        e2.message().unwrap_or("null"),
    );
    Ok(())
}

fn fsf_test_with_graph_input() -> Result<()> {
    let mut ts = CannedTokenStream::new(vec![
        Token::with_offsets("fuz", 0, 3),
        Token::with_pos_len("foo", 1, 4, 6, 2),
        Token::with_pos_inc("bar", 0, 4, 6),
        Token::with_pos_inc("baz", 1, 4, 6),
    ]);
    let graph = rucene::util::graph::GraphTokenStreamFiniteStrings::new(&mut ts)?;
    let mut it = graph.get_finite_strings();
    let first = it.next()?.expect("a finite string");
    B::assert_token_stream_contents(&mut FixedShingleFilter::new(boxed(first), 2)?, &["fuz foo"])?;
    let second = it.next()?.expect("a finite string");
    B::assert_token_stream_contents(
        &mut FixedShingleFilter::new(boxed(second), 2)?,
        &["fuz bar", "bar baz"],
    )
}
fn fixed_shingle_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.shingle.TestFixedShingleFilter")
        .method(t("testBiGramFilter", fsf_test_bi_gram_filter))
        .method(t(
            "testBiGramFilterWithAltSeparator",
            fsf_test_bi_gram_filter_with_alt_separator,
        ))
        .method(t("testTriGramFilter", fsf_test_tri_gram_filter))
        .method(t(
            "testShingleSizeGreaterThanTokenstreamLength",
            fsf_test_shingle_size_greater_than_tokenstream_length,
        ))
        .method(t("testWithStopwords", fsf_test_with_stopwords))
        .method(t(
            "testConsecutiveStopwords",
            fsf_test_consecutive_stopwords,
        ))
        .method(t("testTrailingStopwords", fsf_test_trailing_stopwords))
        .method(t(
            "testMultipleTrailingStopwords",
            fsf_test_multiple_trailing_stopwords,
        ))
        .method(t("testIncomingGraphs", fsf_test_incoming_graphs))
        .method(t(
            "testShinglesSpanningGraphs",
            fsf_test_shingles_spanning_graphs,
        ))
        .method(t(
            "testTrailingGraphsOfDifferingLengths",
            fsf_test_trailing_graphs_of_differing_lengths,
        ))
        .method(t("testParameterLimits", fsf_test_parameter_limits))
        .method(t("testWithGraphInput", fsf_test_with_graph_input))
        .build()
}

// -----------------------------------------------------------------------------
// TestShingleFilterFactory
// -----------------------------------------------------------------------------

fn sff_test_defaults() -> Result<()> {
    let reader = string_reader("this is a test");
    let mut stream: Box<dyn TokenStream> = boxed(B::whitespace_mock_tokenizer(reader)?);
    stream = fac("Shingle", &[])?.create(stream)?;
    B::assert_token_stream_contents(
        &mut *stream,
        &["this", "this is", "is", "is a", "a", "a test", "test"],
    )?;
    Ok(())
}

fn sff_test_no_unigrams() -> Result<()> {
    let reader = string_reader("this is a test");
    let mut stream: Box<dyn TokenStream> = boxed(B::whitespace_mock_tokenizer(reader)?);
    stream = fac("Shingle", &["outputUnigrams", "false"])?.create(stream)?;
    B::assert_token_stream_contents(&mut *stream, &["this is", "is a", "a test"])?;
    Ok(())
}

fn sff_test_max_shingle_size() -> Result<()> {
    let reader = string_reader("this is a test");
    let mut stream: Box<dyn TokenStream> = boxed(B::whitespace_mock_tokenizer(reader)?);
    stream = fac("Shingle", &["maxShingleSize", "3"])?.create(stream)?;
    B::assert_token_stream_contents(
        &mut *stream,
        &[
            "this",
            "this is",
            "this is a",
            "is",
            "is a",
            "is a test",
            "a",
            "a test",
            "test",
        ],
    )?;
    Ok(())
}

fn sff_test_min_shingle_size() -> Result<()> {
    let reader = string_reader("this is a test");
    let mut stream: Box<dyn TokenStream> = boxed(B::whitespace_mock_tokenizer(reader)?);
    stream = fac("Shingle", &["minShingleSize", "3", "maxShingleSize", "4"])?.create(stream)?;
    B::assert_token_stream_contents(
        &mut *stream,
        &[
            "this",
            "this is a",
            "this is a test",
            "is",
            "is a test",
            "a",
            "test",
        ],
    )?;
    Ok(())
}

fn sff_test_min_shingle_size_no_unigrams() -> Result<()> {
    let reader = string_reader("this is a test");
    let mut stream: Box<dyn TokenStream> = boxed(B::whitespace_mock_tokenizer(reader)?);
    stream = fac(
        "Shingle",
        &[
            "minShingleSize",
            "3",
            "maxShingleSize",
            "4",
            "outputUnigrams",
            "false",
        ],
    )?
    .create(stream)?;
    B::assert_token_stream_contents(&mut *stream, &["this is a", "this is a test", "is a test"])?;
    Ok(())
}

fn sff_test_equal_min_and_max_shingle_size() -> Result<()> {
    let reader = string_reader("this is a test");
    let mut stream: Box<dyn TokenStream> = boxed(B::whitespace_mock_tokenizer(reader)?);
    stream = fac("Shingle", &["minShingleSize", "3", "maxShingleSize", "3"])?.create(stream)?;
    B::assert_token_stream_contents(
        &mut *stream,
        &["this", "this is a", "is", "is a test", "a", "test"],
    )?;
    Ok(())
}

fn sff_test_equal_min_and_max_shingle_size_no_unigrams() -> Result<()> {
    let reader = string_reader("this is a test");
    let mut stream: Box<dyn TokenStream> = boxed(B::whitespace_mock_tokenizer(reader)?);
    stream = fac(
        "Shingle",
        &[
            "minShingleSize",
            "3",
            "maxShingleSize",
            "3",
            "outputUnigrams",
            "false",
        ],
    )?
    .create(stream)?;
    B::assert_token_stream_contents(&mut *stream, &["this is a", "is a test"])?;
    Ok(())
}

fn sff_test_token_separator() -> Result<()> {
    let reader = string_reader("this is a test");
    let mut stream: Box<dyn TokenStream> = boxed(B::whitespace_mock_tokenizer(reader)?);
    stream = fac("Shingle", &["tokenSeparator", "=BLAH="])?.create(stream)?;
    B::assert_token_stream_contents(
        &mut *stream,
        &[
            "this",
            "this=BLAH=is",
            "is",
            "is=BLAH=a",
            "a",
            "a=BLAH=test",
            "test",
        ],
    )?;
    Ok(())
}

fn sff_test_token_separator_no_unigrams() -> Result<()> {
    let reader = string_reader("this is a test");
    let mut stream: Box<dyn TokenStream> = boxed(B::whitespace_mock_tokenizer(reader)?);
    stream = fac(
        "Shingle",
        &["tokenSeparator", "=BLAH=", "outputUnigrams", "false"],
    )?
    .create(stream)?;
    B::assert_token_stream_contents(&mut *stream, &["this=BLAH=is", "is=BLAH=a", "a=BLAH=test"])?;
    Ok(())
}

fn sff_test_empty_token_separator() -> Result<()> {
    let reader = string_reader("this is a test");
    let mut stream: Box<dyn TokenStream> = boxed(B::whitespace_mock_tokenizer(reader)?);
    stream = fac("Shingle", &["tokenSeparator", ""])?.create(stream)?;
    B::assert_token_stream_contents(
        &mut *stream,
        &["this", "thisis", "is", "isa", "a", "atest", "test"],
    )?;
    Ok(())
}

fn sff_test_min_shingle_size_and_token_separator() -> Result<()> {
    let reader = string_reader("this is a test");
    let mut stream: Box<dyn TokenStream> = boxed(B::whitespace_mock_tokenizer(reader)?);
    stream = fac(
        "Shingle",
        &[
            "minShingleSize",
            "3",
            "maxShingleSize",
            "4",
            "tokenSeparator",
            "=BLAH=",
        ],
    )?
    .create(stream)?;
    B::assert_token_stream_contents(
        &mut *stream,
        &[
            "this",
            "this=BLAH=is=BLAH=a",
            "this=BLAH=is=BLAH=a=BLAH=test",
            "is",
            "is=BLAH=a=BLAH=test",
            "a",
            "test",
        ],
    )?;
    Ok(())
}

fn sff_test_min_shingle_size_and_token_separator_no_unigrams() -> Result<()> {
    let reader = string_reader("this is a test");
    let mut stream: Box<dyn TokenStream> = boxed(B::whitespace_mock_tokenizer(reader)?);
    stream = fac(
        "Shingle",
        &[
            "minShingleSize",
            "3",
            "maxShingleSize",
            "4",
            "tokenSeparator",
            "=BLAH=",
            "outputUnigrams",
            "false",
        ],
    )?
    .create(stream)?;
    B::assert_token_stream_contents(
        &mut *stream,
        &[
            "this=BLAH=is=BLAH=a",
            "this=BLAH=is=BLAH=a=BLAH=test",
            "is=BLAH=a=BLAH=test",
        ],
    )?;
    Ok(())
}

fn sff_test_output_unigrams_if_no_shingles() -> Result<()> {
    let reader = string_reader("test");
    let mut stream: Box<dyn TokenStream> = boxed(B::whitespace_mock_tokenizer(reader)?);
    stream = fac(
        "Shingle",
        &[
            "outputUnigrams",
            "false",
            "outputUnigramsIfNoShingles",
            "true",
        ],
    )?
    .create(stream)?;
    B::assert_token_stream_contents(&mut *stream, &["test"])?;
    Ok(())
}

fn sff_test_bogus_arguments() -> Result<()> {
    let expected = iae_message(|| fac("Shingle", &["bogusArg", "bogusValue"]));
    Assert::assert_true(expected.contains("Unknown parameters"));
    Ok(())
}
fn shingle_filter_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.shingle.TestShingleFilterFactory")
        .method(t("testDefaults", sff_test_defaults))
        .method(t("testNoUnigrams", sff_test_no_unigrams))
        .method(t("testMaxShingleSize", sff_test_max_shingle_size))
        .method(t("testMinShingleSize", sff_test_min_shingle_size))
        .method(t(
            "testMinShingleSizeNoUnigrams",
            sff_test_min_shingle_size_no_unigrams,
        ))
        .method(t(
            "testEqualMinAndMaxShingleSize",
            sff_test_equal_min_and_max_shingle_size,
        ))
        .method(t(
            "testEqualMinAndMaxShingleSizeNoUnigrams",
            sff_test_equal_min_and_max_shingle_size_no_unigrams,
        ))
        .method(t("testTokenSeparator", sff_test_token_separator))
        .method(t(
            "testTokenSeparatorNoUnigrams",
            sff_test_token_separator_no_unigrams,
        ))
        .method(t("testEmptyTokenSeparator", sff_test_empty_token_separator))
        .method(t(
            "testMinShingleSizeAndTokenSeparator",
            sff_test_min_shingle_size_and_token_separator,
        ))
        .method(t(
            "testMinShingleSizeAndTokenSeparatorNoUnigrams",
            sff_test_min_shingle_size_and_token_separator_no_unigrams,
        ))
        .method(t(
            "testOutputUnigramsIfNoShingles",
            sff_test_output_unigrams_if_no_shingles,
        ))
        .method(t("testBogusArguments", sff_test_bogus_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// TestShingleAnalyzerWrapper
// -----------------------------------------------------------------------------

type SawState = (
    Arc<dyn Analyzer>,
    rucene::search::index_searcher::IndexSearcher,
    Arc<dyn rucene::index::DirectoryReader>,
    rucene_test_framework::util::lucene_test_case::TestDirectory,
);

thread_local! {
    /// The fields `analyzer`, `searcher`, `reader` and `directory`.
    static SAW: RefCell<Option<SawState>> = const { RefCell::new(None) };
}

fn field_analyzer() -> Arc<dyn Analyzer> {
    SAW.with(|s| Arc::clone(&s.borrow().as_ref().expect("setUp").0))
}

fn field_searcher() -> rucene::search::index_searcher::IndexSearcher {
    SAW.with(|s| s.borrow().as_ref().expect("setUp").1.clone())
}

/// `new MockAnalyzer(random(), MockTokenizer.WHITESPACE, false)`.
fn mock_ws() -> Arc<dyn Analyzer> {
    Arc::new(MockAnalyzer::with_automaton(
        &*random(),
        Arc::clone(&WHITESPACE),
        false,
    ))
}

/// The anonymous delegate of `testAltFillerToken`: a whitespace
/// `MockTokenizer` and a `StopFilter` of "into".
fn stop_into_delegate() -> Arc<dyn Analyzer> {
    whitespace_analyzer(
        "org.apache.lucene.analysis.shingle.TestShingleAnalyzerWrapper$1",
        |tokenizer| {
            let stop_set = make_stop_set(["into"], false);
            Ok(boxed(new_stop_filter(tokenizer, stop_set)?))
        },
    )
}

/// Set up a new index in RAM with three test phrases and the supplied Analyzer.
fn saw_set_up() -> Result<()> {
    LuceneTestCase::super_set_up();
    let analyzer: Arc<dyn Analyzer> =
        Arc::new(ShingleAnalyzerWrapper::with_max_shingle_size(mock_ws(), 2)?);
    let directory = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(
        directory.as_directory(),
        IndexWriterConfig::with_analyzer(Some(Arc::clone(&analyzer))),
    )?;
    for text in [
        "please divide this sentence into shingles",
        "just another test sentence",
        "a sentence which contains no test",
    ] {
        let mut doc = Document::new();
        doc.add(Box::new(TextField::new(
            "content",
            text.to_string(),
            Store::YES,
        )?));
        writer.add_document(&doc)?;
    }
    writer.close()?;
    let reader = directory_reader::open(directory.as_directory())?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    SAW.with(|s| *s.borrow_mut() = Some((analyzer, searcher, reader, directory)));
    Ok(())
}

fn saw_tear_down() -> Result<()> {
    if let Some((analyzer, searcher, reader, directory)) = SAW.with(|s| s.borrow_mut().take()) {
        drop(searcher);
        reader.close()?;
        directory.close()?;
        analyzer.close();
    }
    LuceneTestCase::super_tear_down();
    Ok(())
}

/// `compareRanks(hits, ranks)`.
fn compare_ranks(hits: &[ScoreDoc], ranks: &[i32]) -> Result<()> {
    Assert::assert_equals(ranks.len(), hits.len());
    for (rank, hit) in ranks.iter().zip(hits) {
        Assert::assert_equals(*rank, hit.doc);
    }
    Ok(())
}

fn saw_test_shingle_analyzer_wrapper_phrase_query() -> Result<()> {
    let mut builder = PhraseQueryBuilder::new();
    {
        let ts = field_analyzer().token_stream("content", string_reader("this sentence"))?;
        let mut ts = ts.borrow_mut();
        let mut j = -1;
        ts.attribute_source_mut()
            .add_attribute_interface::<dyn PositionIncrementAttribute>()?;
        ts.attribute_source_mut()
            .add_attribute_interface::<dyn CharTermAttribute>()?;
        ts.reset()?;
        while ts.increment_token()? {
            j += pos_inc_of(&*ts);
            let term_text = term_of(&*ts);
            builder.add_at(Term::from_text("content", &term_text), j)?;
        }
        ts.end()?;
        ts.close()?;
    }
    let q = builder.build()?;
    let hits = field_searcher().search(Arc::new(q), 1000)?.score_docs;
    compare_ranks(&hits, &[0])
}

fn saw_test_shingle_analyzer_wrapper_boolean_query() -> Result<()> {
    let mut q = BooleanQueryBuilder::new();
    {
        let ts = field_analyzer().token_stream("content", string_reader("test sentence"))?;
        let mut ts = ts.borrow_mut();
        ts.attribute_source_mut()
            .add_attribute_interface::<dyn CharTermAttribute>()?;
        ts.reset()?;
        while ts.increment_token()? {
            let term_text = term_of(&*ts);
            q.add(
                Arc::new(TermQuery::new(Term::from_text("content", &term_text))),
                Occur::SHOULD,
            )?;
        }
        ts.end()?;
        ts.close()?;
    }
    let hits = field_searcher()
        .search(Arc::new(q.build()), 1000)?
        .score_docs;
    compare_ranks(&hits, &[1, 2, 0])
}

fn saw_test_reusable_token_stream() -> Result<()> {
    let a: Arc<dyn Analyzer> =
        Arc::new(ShingleAnalyzerWrapper::with_max_shingle_size(mock_ws(), 2)?);
    B::assert_analyzes_to_offsets_pos_increments(
        &*a,
        "please divide into shingles",
        &[
            "please",
            "please divide",
            "divide",
            "divide into",
            "into",
            "into shingles",
            "shingles",
        ],
        &[0, 0, 7, 7, 14, 14, 19],
        &[6, 13, 13, 18, 18, 27, 27],
        &[1, 0, 1, 0, 1, 0, 1],
    )?;
    B::assert_analyzes_to_offsets_pos_increments(
        &*a,
        "divide me up again",
        &[
            "divide",
            "divide me",
            "me",
            "me up",
            "up",
            "up again",
            "again",
        ],
        &[0, 0, 7, 7, 10, 10, 13],
        &[6, 9, 9, 12, 12, 18, 18],
        &[1, 0, 1, 0, 1, 0, 1],
    )?;
    a.close();
    Ok(())
}

fn saw_test_non_default_min_shingle_size() -> Result<()> {
    let analyzer: Arc<dyn Analyzer> =
        Arc::new(ShingleAnalyzerWrapper::with_shingle_sizes(mock_ws(), 3, 4)?);
    B::assert_analyzes_to_offsets_pos_increments(
        &*analyzer,
        "please divide this sentence into shingles",
        &[
            "please",
            "please divide this",
            "please divide this sentence",
            "divide",
            "divide this sentence",
            "divide this sentence into",
            "this",
            "this sentence into",
            "this sentence into shingles",
            "sentence",
            "sentence into shingles",
            "into",
            "shingles",
        ],
        &[0, 0, 0, 7, 7, 7, 14, 14, 14, 19, 19, 28, 33],
        &[6, 18, 27, 13, 27, 32, 18, 32, 41, 27, 41, 32, 41],
        &[1, 0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 1, 1],
    )?;
    analyzer.close();
    let analyzer: Arc<dyn Analyzer> = Arc::new(ShingleAnalyzerWrapper::with_options(
        mock_ws(),
        3,
        4,
        Some(DEFAULT_TOKEN_SEPARATOR),
        false,
        false,
        Some(DEFAULT_FILLER_TOKEN),
    )?);
    B::assert_analyzes_to_offsets_pos_increments(
        &*analyzer,
        "please divide this sentence into shingles",
        &[
            "please divide this",
            "please divide this sentence",
            "divide this sentence",
            "divide this sentence into",
            "this sentence into",
            "this sentence into shingles",
            "sentence into shingles",
        ],
        &[0, 0, 7, 7, 14, 14, 19],
        &[18, 27, 27, 32, 32, 41, 41],
        &[1, 0, 1, 0, 1, 0, 1],
    )?;
    analyzer.close();
    Ok(())
}

fn saw_test_non_default_min_and_same_max_shingle_size() -> Result<()> {
    let analyzer: Arc<dyn Analyzer> =
        Arc::new(ShingleAnalyzerWrapper::with_shingle_sizes(mock_ws(), 3, 3)?);
    B::assert_analyzes_to_offsets_pos_increments(
        &*analyzer,
        "please divide this sentence into shingles",
        &[
            "please",
            "please divide this",
            "divide",
            "divide this sentence",
            "this",
            "this sentence into",
            "sentence",
            "sentence into shingles",
            "into",
            "shingles",
        ],
        &[0, 0, 7, 7, 14, 14, 19, 19, 28, 33],
        &[6, 18, 13, 27, 18, 32, 27, 41, 32, 41],
        &[1, 0, 1, 0, 1, 0, 1, 0, 1, 1],
    )?;
    analyzer.close();
    let analyzer: Arc<dyn Analyzer> = Arc::new(ShingleAnalyzerWrapper::with_options(
        mock_ws(),
        3,
        3,
        Some(DEFAULT_TOKEN_SEPARATOR),
        false,
        false,
        Some(DEFAULT_FILLER_TOKEN),
    )?);
    B::assert_analyzes_to_offsets_pos_increments(
        &*analyzer,
        "please divide this sentence into shingles",
        &[
            "please divide this",
            "divide this sentence",
            "this sentence into",
            "sentence into shingles",
        ],
        &[0, 7, 14, 19],
        &[18, 27, 32, 41],
        &[1, 1, 1, 1],
    )?;
    analyzer.close();
    Ok(())
}

fn saw_test_no_token_separator() -> Result<()> {
    let analyzer: Arc<dyn Analyzer> = Arc::new(ShingleAnalyzerWrapper::with_options(
        mock_ws(),
        DEFAULT_MIN_SHINGLE_SIZE,
        DEFAULT_MAX_SHINGLE_SIZE,
        Some(""),
        true,
        false,
        Some(DEFAULT_FILLER_TOKEN),
    )?);
    B::assert_analyzes_to_offsets_pos_increments(
        &*analyzer,
        "please divide into shingles",
        &[
            "please",
            "pleasedivide",
            "divide",
            "divideinto",
            "into",
            "intoshingles",
            "shingles",
        ],
        &[0, 0, 7, 7, 14, 14, 19],
        &[6, 13, 13, 18, 18, 27, 27],
        &[1, 0, 1, 0, 1, 0, 1],
    )?;
    analyzer.close();
    let analyzer: Arc<dyn Analyzer> = Arc::new(ShingleAnalyzerWrapper::with_options(
        mock_ws(),
        DEFAULT_MIN_SHINGLE_SIZE,
        DEFAULT_MAX_SHINGLE_SIZE,
        Some(""),
        false,
        false,
        Some(DEFAULT_FILLER_TOKEN),
    )?);
    B::assert_analyzes_to_offsets_pos_increments(
        &*analyzer,
        "please divide into shingles",
        &["pleasedivide", "divideinto", "intoshingles"],
        &[0, 7, 14],
        &[13, 18, 27],
        &[1, 1, 1],
    )?;
    analyzer.close();
    Ok(())
}

fn saw_test_null_token_separator() -> Result<()> {
    let analyzer: Arc<dyn Analyzer> = Arc::new(ShingleAnalyzerWrapper::with_options(
        mock_ws(),
        DEFAULT_MIN_SHINGLE_SIZE,
        DEFAULT_MAX_SHINGLE_SIZE,
        None,
        true,
        false,
        Some(DEFAULT_FILLER_TOKEN),
    )?);
    B::assert_analyzes_to_offsets_pos_increments(
        &*analyzer,
        "please divide into shingles",
        &[
            "please",
            "pleasedivide",
            "divide",
            "divideinto",
            "into",
            "intoshingles",
            "shingles",
        ],
        &[0, 0, 7, 7, 14, 14, 19],
        &[6, 13, 13, 18, 18, 27, 27],
        &[1, 0, 1, 0, 1, 0, 1],
    )?;
    analyzer.close();
    let analyzer: Arc<dyn Analyzer> = Arc::new(ShingleAnalyzerWrapper::with_options(
        mock_ws(),
        DEFAULT_MIN_SHINGLE_SIZE,
        DEFAULT_MAX_SHINGLE_SIZE,
        Some(""),
        false,
        false,
        Some(DEFAULT_FILLER_TOKEN),
    )?);
    B::assert_analyzes_to_offsets_pos_increments(
        &*analyzer,
        "please divide into shingles",
        &["pleasedivide", "divideinto", "intoshingles"],
        &[0, 7, 14],
        &[13, 18, 27],
        &[1, 1, 1],
    )?;
    analyzer.close();
    Ok(())
}

fn saw_test_alt_token_separator() -> Result<()> {
    let analyzer: Arc<dyn Analyzer> = Arc::new(ShingleAnalyzerWrapper::with_options(
        mock_ws(),
        DEFAULT_MIN_SHINGLE_SIZE,
        DEFAULT_MAX_SHINGLE_SIZE,
        Some("<SEP>"),
        true,
        false,
        Some(DEFAULT_FILLER_TOKEN),
    )?);
    B::assert_analyzes_to_offsets_pos_increments(
        &*analyzer,
        "please divide into shingles",
        &[
            "please",
            "please<SEP>divide",
            "divide",
            "divide<SEP>into",
            "into",
            "into<SEP>shingles",
            "shingles",
        ],
        &[0, 0, 7, 7, 14, 14, 19],
        &[6, 13, 13, 18, 18, 27, 27],
        &[1, 0, 1, 0, 1, 0, 1],
    )?;
    analyzer.close();
    let analyzer: Arc<dyn Analyzer> = Arc::new(ShingleAnalyzerWrapper::with_options(
        mock_ws(),
        DEFAULT_MIN_SHINGLE_SIZE,
        DEFAULT_MAX_SHINGLE_SIZE,
        Some("<SEP>"),
        false,
        false,
        Some(DEFAULT_FILLER_TOKEN),
    )?);
    B::assert_analyzes_to_offsets_pos_increments(
        &*analyzer,
        "please divide into shingles",
        &["please<SEP>divide", "divide<SEP>into", "into<SEP>shingles"],
        &[0, 7, 14],
        &[13, 18, 27],
        &[1, 1, 1],
    )?;
    analyzer.close();
    Ok(())
}

fn saw_test_alt_filler_token() -> Result<()> {
    let delegate = stop_into_delegate();
    let analyzer: Arc<dyn Analyzer> = Arc::new(ShingleAnalyzerWrapper::with_options(
        Arc::clone(&delegate),
        DEFAULT_MIN_SHINGLE_SIZE,
        DEFAULT_MAX_SHINGLE_SIZE,
        Some(DEFAULT_TOKEN_SEPARATOR),
        true,
        false,
        Some("--"),
    )?);
    B::assert_analyzes_to_offsets_pos_increments(
        &*analyzer,
        "please divide into shingles",
        &[
            "please",
            "please divide",
            "divide",
            "divide --",
            "-- shingles",
            "shingles",
        ],
        &[0, 0, 7, 7, 19, 19],
        &[6, 13, 13, 19, 27, 27],
        &[1, 0, 1, 0, 1, 1],
    )?;
    analyzer.close();
    let delegate = stop_into_delegate();
    let analyzer: Arc<dyn Analyzer> = Arc::new(ShingleAnalyzerWrapper::with_options(
        Arc::clone(&delegate),
        DEFAULT_MIN_SHINGLE_SIZE,
        DEFAULT_MAX_SHINGLE_SIZE,
        Some(DEFAULT_TOKEN_SEPARATOR),
        false,
        false,
        None,
    )?);
    B::assert_analyzes_to_offsets_pos_increments(
        &*analyzer,
        "please divide into shingles",
        &["please divide", "divide ", " shingles"],
        &[0, 7, 19],
        &[13, 19, 27],
        &[1, 1, 1],
    )?;
    analyzer.close();
    let delegate = stop_into_delegate();
    let analyzer: Arc<dyn Analyzer> = Arc::new(ShingleAnalyzerWrapper::with_options(
        Arc::clone(&delegate),
        DEFAULT_MIN_SHINGLE_SIZE,
        DEFAULT_MAX_SHINGLE_SIZE,
        Some(DEFAULT_TOKEN_SEPARATOR),
        false,
        false,
        Some(""),
    )?);
    B::assert_analyzes_to_offsets_pos_increments(
        &*analyzer,
        "please divide into shingles",
        &["please divide", "divide ", " shingles"],
        &[0, 7, 19],
        &[13, 19, 27],
        &[1, 1, 1],
    )?;
    analyzer.close();
    Ok(())
}

fn saw_test_output_unigrams_if_no_shingles_single_token() -> Result<()> {
    let analyzer: Arc<dyn Analyzer> = Arc::new(ShingleAnalyzerWrapper::with_options(
        mock_ws(),
        DEFAULT_MIN_SHINGLE_SIZE,
        DEFAULT_MAX_SHINGLE_SIZE,
        Some(""),
        false,
        true,
        Some(DEFAULT_FILLER_TOKEN),
    )?);
    B::assert_analyzes_to_offsets_pos_increments(
        &*analyzer,
        "please",
        &["please"],
        &[0],
        &[6],
        &[1],
    )?;
    analyzer.close();
    Ok(())
}
fn shingle_analyzer_wrapper_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.shingle.TestShingleAnalyzerWrapper")
        .method(hook("setUp", saw_set_up))
        .method(hook("tearDown", saw_tear_down))
        .method(t(
            "testShingleAnalyzerWrapperPhraseQuery",
            saw_test_shingle_analyzer_wrapper_phrase_query,
        ))
        .method(t(
            "testShingleAnalyzerWrapperBooleanQuery",
            saw_test_shingle_analyzer_wrapper_boolean_query,
        ))
        .method(t("testReusableTokenStream", saw_test_reusable_token_stream))
        .method(t(
            "testNonDefaultMinShingleSize",
            saw_test_non_default_min_shingle_size,
        ))
        .method(t(
            "testNonDefaultMinAndSameMaxShingleSize",
            saw_test_non_default_min_and_same_max_shingle_size,
        ))
        .method(t("testNoTokenSeparator", saw_test_no_token_separator))
        .method(t("testNullTokenSeparator", saw_test_null_token_separator))
        .method(t("testAltTokenSeparator", saw_test_alt_token_separator))
        .method(t("testAltFillerToken", saw_test_alt_filler_token))
        .method(t(
            "testOutputUnigramsIfNoShinglesSingleToken",
            saw_test_output_unigrams_if_no_shingles_single_token,
        ))
        .build()
}

// -----------------------------------------------------------------------------
// TestCommonGramsFilter
// -----------------------------------------------------------------------------

/// `commonWords`.
fn common_words() -> CharArraySet {
    CharArraySet::from_iter(["s", "a", "b", "c", "d", "the", "of"], false)
}

/// The query-filter checks of `testLastWordisStopWord` & co.: a whitespace
/// `MockTokenizer` over `input`, a `CommonGramsFilter` and a
/// `CommonGramsQueryFilter`.
fn query_filter_of(input: &str, expected: &[&str]) -> Result<()> {
    let mut wt = MockTokenizer::new(Arc::clone(&WHITESPACE), false)?;
    wt.set_reader(string_reader(input))?;
    let cgf = new_common_grams_filter(boxed(wt), Some(common_words()))?;
    let mut nsf = new_common_grams_query_filter(cgf)?;
    B::assert_token_stream_contents(&mut nsf, expected)
}

// -----------------------------------------------------------------------------
// TestCommonGramsFilterFactory and TestCommonGramsQueryFilterFactory
// -----------------------------------------------------------------------------

/// `(CommonGramsFilterFactory) tokenFilterFactory("CommonGrams", Version.LATEST,
/// loader, keysAndValues...)` over the resources of `package`.
fn common_grams_factory(
    package: &str,
    keys_and_values: &[&str],
) -> Result<CommonGramsFilterFactory> {
    support::concrete_factory(package, CommonGramsFilterFactory::new, keys_and_values)
}

fn common_grams_query_factory(
    package: &str,
    keys_and_values: &[&str],
) -> Result<CommonGramsQueryFilterFactory> {
    support::concrete_factory(package, CommonGramsQueryFilterFactory::new, keys_and_values)
}

const COMMONGRAMS: &str = "org.apache.lucene.analysis.commongrams";
const CORE: &str = "org.apache.lucene.analysis.core";

/// The `testInform` body over `words(factory)` and `is_ignore_case(factory)`.
fn check_inform<T>(
    make: impl Fn(&[&str]) -> Result<T>,
    words: impl Fn(&T) -> Option<CharArraySet>,
    ignore_case: impl Fn(&T) -> bool,
    files: [&str; 3],
) -> Result<()> {
    let loader_is_null = false; // `new ClasspathResourceLoader(...)`
    Assert::assert_true_msg("loader is null and it shouldn't be", !loader_is_null);
    let factory = make(&["words", files[0], "ignoreCase", "true"])?;
    let w = words(&factory);
    Assert::assert_true_msg("words is null and it shouldn't be", w.is_some());
    let w = w.expect("words");
    Assert::assert_true_msg(
        &format!("words Size: {} is not: 2", w.size()),
        w.size() == 2,
    );
    Assert::assert_true_msg(
        &format!("{} does not equal: true", ignore_case(&factory)),
        ignore_case(&factory),
    );

    let factory = make(&["words", files[1], "ignoreCase", "true"])?;
    let w = words(&factory);
    Assert::assert_true_msg("words is null and it shouldn't be", w.is_some());
    let w = w.expect("words");
    Assert::assert_true_msg(
        &format!("words Size: {} is not: 4", w.size()),
        w.size() == 4,
    );
    Assert::assert_true_msg(
        &format!("{} does not equal: true", ignore_case(&factory)),
        ignore_case(&factory),
    );

    let factory = make(&[
        "words",
        files[2],
        "format",
        "snowball",
        "ignoreCase",
        "true",
    ])?;
    let w = words(&factory).expect("words");
    Assert::assert_equals(8, w.size());
    for word in [
        "he", "him", "his", "himself", "she", "her", "hers", "herself",
    ] {
        Assert::assert_true(w.contains(Some(word)));
    }
    Ok(())
}

fn cgff_test_inform() -> Result<()> {
    check_inform(
        |kv| common_grams_factory(COMMONGRAMS, kv),
        |f| f.get_common_words().cloned(),
        |f| f.base().is_ignore_case(),
        [
            "common-1.txt",
            "common-1.txt, common-2.txt",
            "common-snowball.txt",
        ],
    )
}

/// If no words are provided, then a set of english default stopwords is used.
fn cgff_test_defaults() -> Result<()> {
    let factory = common_grams_factory(COMMONGRAMS, &[])?;
    let words = factory.get_common_words().cloned();
    Assert::assert_true_msg("words is null and it shouldn't be", words.is_some());
    Assert::assert_true(words.expect("words").contains(Some("the")));
    let mut tokenizer = MockTokenizer::new(Arc::clone(&WHITESPACE), false)?;
    tokenizer.set_reader(string_reader("testing the factory"))?;
    let mut stream =
        rucene::analysis::factories::TokenFilterFactory::create(&factory, boxed(tokenizer))?;
    B::assert_token_stream_contents(
        &mut *stream,
        &["testing", "testing_the", "the", "the_factory", "factory"],
    )
}

/// Test that ignoreCase flag is honored when no words are provided and default stopwords are used.
fn cgff_test_ignore_case() -> Result<()> {
    let factory = common_grams_factory(COMMONGRAMS, &["ignoreCase", "true"])?;
    let words = factory.get_common_words().cloned();
    Assert::assert_true_msg("words is null and it shouldn't be", words.is_some());
    let words = words.expect("words");
    Assert::assert_true(words.contains(Some("the")));
    Assert::assert_true(words.contains(Some("The")));
    let mut tokenizer = MockTokenizer::new(Arc::clone(&WHITESPACE), false)?;
    tokenizer.set_reader(string_reader("testing The factory"))?;
    let mut stream =
        rucene::analysis::factories::TokenFilterFactory::create(&factory, boxed(tokenizer))?;
    B::assert_token_stream_contents(
        &mut *stream,
        &["testing", "testing_The", "The", "The_factory", "factory"],
    )
}

/// Test that bogus arguments result in exception
fn cgff_test_bogus_arguments() -> Result<()> {
    let expected = iae_message(|| fac("CommonGrams", &["bogusArg", "bogusValue"]));
    Assert::assert_true(expected.contains("Unknown parameters"));
    Ok(())
}

fn common_grams_filter_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.commongrams.TestCommonGramsFilterFactory")
        .method(t("testInform", cgff_test_inform))
        .method(t("testDefaults", cgff_test_defaults))
        .method(t("testIgnoreCase", cgff_test_ignore_case))
        .method(t("testBogusArguments", cgff_test_bogus_arguments))
        .build()
}

fn cgqff_test_inform() -> Result<()> {
    // `new ClasspathResourceLoader(TestStopFilterFactory.class)`: the core package
    check_inform(
        |kv| common_grams_query_factory(CORE, kv),
        |f| f.base().get_common_words().cloned(),
        |f| f.base().base().is_ignore_case(),
        ["stop-1.txt", "stop-1.txt, stop-2.txt", "stop-snowball.txt"],
    )
}

/// If no words are provided, then a set of english default stopwords is used.
fn cgqff_test_defaults() -> Result<()> {
    let factory = common_grams_query_factory(COMMONGRAMS, &[])?;
    let words = factory.base().get_common_words().cloned();
    Assert::assert_true_msg("words is null and it shouldn't be", words.is_some());
    Assert::assert_true(words.expect("words").contains(Some("the")));
    let tokenizer = B::whitespace_mock_tokenizer_str("testing the factory")?;
    let mut stream =
        rucene::analysis::factories::TokenFilterFactory::create(&factory, boxed(tokenizer))?;
    B::assert_token_stream_contents(&mut *stream, &["testing_the", "the_factory"])
}

/// Test that bogus arguments result in exception
fn cgqff_test_bogus_arguments() -> Result<()> {
    let expected = iae_message(|| fac("CommonGramsQuery", &["bogusArg", "bogusValue"]));
    Assert::assert_true(expected.contains("Unknown parameters"));
    Ok(())
}

fn cgqff_test_complete_graph() -> Result<()> {
    let factory = common_grams_query_factory(COMMONGRAMS, &[])?;
    let words = factory.base().get_common_words().cloned();
    Assert::assert_true_msg("words is null and it shouldn't be", words.is_some());
    Assert::assert_true(words.expect("words").contains(Some("the")));
    let tokenizer = B::whitespace_mock_tokenizer_str("testing the factory works")?;
    let mut stream =
        rucene::analysis::factories::TokenFilterFactory::create(&factory, boxed(tokenizer))?;
    B::assert_graph_strings_of(&mut *stream, &["testing_the the_factory factory works"])
}

fn common_grams_query_filter_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.commongrams.TestCommonGramsQueryFilterFactory")
        .method(t("testInform", cgqff_test_inform))
        .method(t("testDefaults", cgqff_test_defaults))
        .method(t("testBogusArguments", cgqff_test_bogus_arguments))
        .method(t("testCompleteGraph", cgqff_test_complete_graph))
        .build()
}

// -----------------------------------------------------------------------------
// TestMinHashFilter
// -----------------------------------------------------------------------------

fn mh_test_int_hash() -> Result<()> {
    let hash = murmurhash3_x64_128(&get_bytes(0), 0, 4, 0);
    Assert::assert_equals(-3485513579396041028i64, hash.val1);
    Assert::assert_equals(6383328099726337777i64, hash.val2);
    Ok(())
}

/// `s.getBytes(StandardCharsets.UTF_16LE)`.
fn utf16le(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
}

fn mh_test_string_hash() -> Result<()> {
    let bytes = utf16le("woof woof woof woof woof");
    let hash = murmurhash3_x64_128(&bytes, 0, bytes.len() as i32, 0);
    Assert::assert_equals(7638079586852243959i64, hash.val1);
    Assert::assert_equals(4378804943379391304i64, hash.val2);
    Ok(())
}

fn mh_test_simple_order() -> Result<()> {
    let hash1 = LongPair { val1: 1, val2: 2 };
    let hash2 = LongPair { val1: 2, val2: 1 };
    // Java's `assert` (tests run with -ea)
    assert!(hash1.cmp(&hash2) == std::cmp::Ordering::Greater);
    Ok(())
}

/// The fill loop of `testHashOrder`, `testCollisions` and
/// `testHashNotRepeated`: `num_elements` hashes into a 500-element set.
fn fill_min_set(
    num_elements: i32,
    mut on_hash: impl FnMut(LongPair),
) -> (FixedSizeTreeSet<LongPair>, HashSet<LongPair>) {
    let mut min_set: FixedSizeTreeSet<LongPair> = FixedSizeTreeSet::with_capacity(500);
    let mut unadded: HashSet<LongPair> = HashSet::new();
    for i in 0..num_elements {
        let hash = murmurhash3_x64_128(&get_bytes(i), 0, 4, 0);
        on_hash(hash);
        let peek = if min_set.size() > 0 {
            min_set.last()
        } else {
            None
        };
        if !min_set.add(hash) {
            unadded.insert(hash);
        } else if let Some(peek) = peek {
            if min_set.size() == 500 && Some(peek) != min_set.last() {
                unadded.insert(peek);
            }
        }
    }
    (min_set, unadded)
}

/// `isLessThan(hash1, hash2)`.
fn is_less_than(hash1: &LongPair, hash2: &LongPair) -> bool {
    is_less_than_unsigned(hash1.val2, hash2.val2)
        || hash1.val2 == hash2.val2 && is_less_than_unsigned(hash1.val1, hash2.val1)
}

/// The `pollLast` order check.
fn check_descending(min_set: &mut FixedSizeTreeSet<LongPair>) {
    let mut last: Option<LongPair> = None;
    while let Some(current) = min_set.poll_last() {
        if let Some(last) = last {
            Assert::assert_true(is_less_than(&current, &last));
        }
        last = Some(current);
    }
}

fn mh_test_hash_order() -> Result<()> {
    Assert::assert_true(!is_less_than_unsigned(0, 0));
    Assert::assert_true(is_less_than_unsigned(0, -1));
    Assert::assert_true(is_less_than_unsigned(1, -1));
    Assert::assert_true(is_less_than_unsigned(-2, -1));
    Assert::assert_true(is_less_than_unsigned(1, 2));
    Assert::assert_true(is_less_than_unsigned(i64::MAX, i64::MIN));
    let (min_set, unadded) = fill_min_set(100, |_| {});
    Assert::assert_equals(100, min_set.size());
    Assert::assert_equals(0, unadded.len());
    Ok(())
}

fn mh_test_collisions() -> Result<()> {
    let mut collision_detection: HashSet<LongPair> = HashSet::new();
    let num_elements = if *TEST_NIGHTLY { 1000000 } else { 10000 };
    let (mut min_set, unadded) = fill_min_set(num_elements, |hash| {
        collision_detection.insert(hash);
    });
    Assert::assert_equals(num_elements as usize, collision_detection.len());
    Assert::assert_equals(500, min_set.size());
    Assert::assert_equals(num_elements as usize - 500, unadded.len());
    check_descending(&mut min_set);
    Ok(())
}

fn mh_test_hash_not_repeated() -> Result<()> {
    let (mut min_set, _unadded) = fill_min_set(10000, |_| {});
    Assert::assert_equals(500, min_set.size());
    check_descending(&mut min_set);
    Ok(())
}

/// `createMockShingleTokenizer(shingleSize, shingles)`.
fn create_mock_shingle_tokenizer(
    shingle_size: i32,
    shingles: Option<&str>,
) -> Result<MockTokenizer> {
    let regexp = RegExp::new(&format!(
        "[^ \t\r\n]+([ \t\r\n]+[^ \t\r\n]+){{{}}}",
        shingle_size - 1
    ))?;
    let a = Operations::determinize(&regexp.to_automaton()?, DEFAULT_DETERMINIZE_WORK_LIMIT)
        .map_err(rucene::error::LuceneError::from)?;
    let mut tokenizer = MockTokenizer::new(Arc::new(CharacterRunAutomaton::new(a)?), true)?;
    tokenizer.set_enable_checks(true);
    if let Some(shingles) = shingles {
        tokenizer.set_reader(string_reader(shingles))?;
    }
    Ok(tokenizer)
}

/// `createTokenStream(shingleSize, shingles, hashCount, bucketCount,
/// hashSetSize, withRotation)`.
fn create_token_stream(
    shingle_size: i32,
    shingles: &str,
    hash_count: i32,
    bucket_count: i32,
    hash_set_size: i32,
    with_rotation: bool,
) -> Result<Box<dyn TokenStream>> {
    let tokenizer = create_mock_shingle_tokenizer(shingle_size, Some(shingles))?;
    let mut lshffargs = std::collections::HashMap::new();
    lshffargs.insert("hashCount".to_string(), hash_count.to_string());
    lshffargs.insert("bucketCount".to_string(), bucket_count.to_string());
    lshffargs.insert("hashSetSize".to_string(), hash_set_size.to_string());
    lshffargs.insert("withRotation".to_string(), with_rotation.to_string());
    let lshff = MinHashFilterFactory::new(&mut lshffargs)?;
    rucene::analysis::factories::TokenFilterFactory::create(&lshff, boxed(tokenizer))
}

/// `new String(new char[] {...})` of BMP units that are not surrogates.
fn units(u: &[u16]) -> String {
    String::from_utf16(u).expect("no surrogate")
}

const WOOF_PUFF: &str = "woof woof woof woof woof woof woof woof woof puff";

fn mh_test_mock_shingle_tokenizer() -> Result<()> {
    let mut mock_shingle_tokenizer = create_mock_shingle_tokenizer(5, Some(WOOF_PUFF))?;
    B::assert_token_stream_contents(
        &mut mock_shingle_tokenizer,
        &["woof woof woof woof woof", "woof woof woof woof puff"],
    )
}

/// `assertTokenStreamContents(ts, output, start, end, types, posInc, posLen,
/// finalOffset, 0, null, true, null)`.
#[allow(clippy::too_many_arguments)]
fn assert_min_hashes(
    ts: &mut dyn TokenStream,
    output: &[&str],
    start: &[i32],
    end: &[i32],
    types: &[&str],
    pos_inc: &[i32],
    pos_len: &[i32],
    final_offset: i32,
) -> Result<()> {
    B::assert_token_stream_contents_payloads(
        ts,
        output,
        Some(start),
        Some(end),
        Some(types),
        Some(pos_inc),
        Some(pos_len),
        Some(final_offset),
        Some(0),
        None,
        true,
        None,
    )
}

fn mh_test_token_stream_single_input() -> Result<()> {
    let hashes = ["\u{2101}\u{D33D}\u{FA85}\u{8059}\u{219D}\u{A1C1}\u{D671}\u{676F}"];
    let mut ts = create_token_stream(5, "woof woof woof woof woof", 1, 1, 100, false)?;
    assert_min_hashes(
        &mut *ts,
        &hashes,
        &[0],
        &[24],
        &[MIN_HASH_TYPE],
        &[1],
        &[1],
        24,
    )?;

    let mut ts = create_token_stream(5, "woof woof woof woof woof", 2, 1, 1, false)?;
    let first = units(&[0, 0, 8449, 54077, 64133, 32857, 8605, 41409]);
    let second = units(&[0, 1, 16887, 58164, 39536, 14926, 6529, 17276]);
    assert_min_hashes(
        &mut *ts,
        &[&first, &second],
        &[0, 0],
        &[24, 24],
        &[MIN_HASH_TYPE, MIN_HASH_TYPE],
        &[1, 0],
        &[1, 1],
        24,
    )
}

fn mh_test_token_stream1() -> Result<()> {
    // Java's second hash is `new String(new char[] {36347, 63457, 43013,
    // 56843, 52284, 34231, 57934, 42302})`: 56843 (0xDE0B) is an unpaired low
    // surrogate, which Rucene's `char` term buffer holds as U+FFFD (the
    // divergence declared on `MinHashFilterLogic`, rmp #459).
    let second: String = [36347u16, 63457, 43013]
        .iter()
        .map(|&u| char::from_u32(u32::from(u)).expect("BMP"))
        .chain(std::iter::once('\u{FFFD}'))
        .chain(
            [52284u16, 34231, 57934, 42302]
                .iter()
                .map(|&u| char::from_u32(u32::from(u)).expect("BMP")),
        )
        .collect();
    let hashes = [
        "\u{2101}\u{D33D}\u{FA85}\u{8059}\u{219D}\u{A1C1}\u{D671}\u{676F}",
        second.as_str(),
    ]; // String is degenerate as characters!
    let mut ts = create_token_stream(5, WOOF_PUFF, 1, 1, 100, false)?;
    assert_min_hashes(
        &mut *ts,
        &hashes,
        &[0, 0],
        &[49, 49],
        &[MIN_HASH_TYPE, MIN_HASH_TYPE],
        &[1, 0],
        &[1, 1],
        49,
    )
}

/// `getTokens(ts)`.
fn get_tokens(ts: &mut dyn TokenStream) -> Result<Vec<String>> {
    let mut tokens = Vec::new();
    ts.reset()?;
    while ts.increment_token()? {
        tokens.push(term_of(ts));
    }
    ts.end()?;
    ts.close()?;
    Ok(tokens)
}

fn mh_test_token_stream2() -> Result<()> {
    let mut ts = create_token_stream(5, WOOF_PUFF, 100, 1, 1, false)?;
    let tokens = get_tokens(&mut *ts)?;
    ts.close()?;
    Assert::assert_equals(100, tokens.len());
    Ok(())
}

fn mh_test_token_stream3() -> Result<()> {
    let mut ts = create_token_stream(5, WOOF_PUFF, 10, 1, 10, false)?;
    let tokens = get_tokens(&mut *ts)?;
    ts.close()?;
    Assert::assert_equals(20, tokens.len());
    Ok(())
}

fn mh_test_token_stream4() -> Result<()> {
    let mut ts = create_token_stream(5, WOOF_PUFF, 10, 10, 1, false)?;
    let tokens = get_tokens(&mut *ts)?;
    ts.close()?;
    Assert::assert_equals(20, tokens.len());
    let mut ts = create_token_stream(5, WOOF_PUFF, 10, 10, 1, true)?;
    let tokens = get_tokens(&mut *ts)?;
    ts.close()?;
    Assert::assert_equals(100, tokens.len());
    Ok(())
}

/// `String.compareTo`: UTF-16 unit order.
fn java_compare(a: &str, b: &str) -> std::cmp::Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

fn mh_test_token_stream5() -> Result<()> {
    let mut ts = create_token_stream(5, WOOF_PUFF, 1, 100, 1, false)?;
    let tokens = get_tokens(&mut *ts)?;
    ts.close()?;
    Assert::assert_equals(2, tokens.len());
    let mut ts = create_token_stream(5, WOOF_PUFF, 1, 100, 1, true)?;
    let tokens = get_tokens(&mut *ts)?;
    ts.close()?;
    Assert::assert_equals(100, tokens.len());
    let set: HashSet<&String> = tokens.iter().collect();
    Assert::assert_equals(2, set.len());
    let mut rolled = false;
    let mut first: Option<&String> = None;
    let mut last: Option<&String> = None;
    for current in &tokens {
        if first.is_none() {
            first = Some(current);
        }
        if let Some(l) = last {
            if !rolled {
                if java_compare(current, l) != std::cmp::Ordering::Less {
                    // fine
                } else if Some(current) == first {
                    rolled = true;
                } else {
                    Assert::fail("Incorrect hash order");
                }
            } else if Some(current) != first {
                Assert::fail("Incorrect hash order");
            }
        }
        last = Some(current);
    }
    Ok(())
}

fn min_hash_filter_class() -> Arc<Class> {
    use randomizedtesting::junit::Test;
    let test = |name: &str, body: fn() -> Result<()>| t(name, body).annotate(Test::new());
    btstc_class("org.apache.lucene.analysis.minhash.TestMinHashFilter")
        .method(test("testIntHash", mh_test_int_hash))
        .method(test("testStringHash", mh_test_string_hash))
        .method(test("testSimpleOrder", mh_test_simple_order))
        .method(test("testHashOrder", mh_test_hash_order))
        .method(t("testCollisions", mh_test_collisions))
        .method(test("testHashNotRepeated", mh_test_hash_not_repeated))
        .method(test(
            "testMockShingleTokenizer",
            mh_test_mock_shingle_tokenizer,
        ))
        .method(test(
            "testTokenStreamSingleInput",
            mh_test_token_stream_single_input,
        ))
        .method(test("testTokenStream1", mh_test_token_stream1))
        .method(test("testTokenStream2", mh_test_token_stream2))
        .method(test("testTokenStream3", mh_test_token_stream3))
        .method(test("testTokenStream4", mh_test_token_stream4))
        .method(test("testTokenStream5", mh_test_token_stream5))
        .build()
}
fn cgf_test_reset() -> Result<()> {
    let input = "How the s a brown s cow d like A B thing?";
    let wt = own(new_whitespace_tokenizer());
    wt.borrow_mut().set_reader(string_reader(input))?;
    let mut cgf = new_common_grams_filter(shared(&wt), Some(common_words()))?;
    cgf.attribute_source_mut()
        .add_attribute_interface::<dyn CharTermAttribute>()?;
    cgf.reset()?;
    for expected in ["How", "How_the", "the", "the_s"] {
        Assert::assert_true(cgf.increment_token()?);
        Assert::assert_equals(expected, term_of(&cgf).as_str());
    }
    cgf.close()?;
    wt.borrow_mut().set_reader(string_reader(input))?;
    cgf.reset()?;
    Assert::assert_true(cgf.increment_token()?);
    Assert::assert_equals("How", term_of(&cgf).as_str());
    Ok(())
}

fn cgf_test_query_reset() -> Result<()> {
    let input = "How the s a brown s cow d like A B thing?";
    let wt = own(new_whitespace_tokenizer());
    wt.borrow_mut().set_reader(string_reader(input))?;
    let cgf = new_common_grams_filter(shared(&wt), Some(common_words()))?;
    let mut nsf = new_common_grams_query_filter(cgf)?;
    // `wt.addAttribute(CharTermAttribute.class)`: the term the tokenizer and
    // its filters share
    wt.borrow_mut()
        .attribute_source_mut()
        .add_attribute_interface::<dyn CharTermAttribute>()?;
    nsf.reset()?;
    for expected in ["How_the", "the_s"] {
        Assert::assert_true(nsf.increment_token()?);
        Assert::assert_equals(expected, term_of(&*wt.borrow()).as_str());
    }
    nsf.close()?;
    wt.borrow_mut().set_reader(string_reader(input))?;
    nsf.reset()?;
    Assert::assert_true(nsf.increment_token()?);
    Assert::assert_equals("How_the", term_of(&*wt.borrow()).as_str());
    Ok(())
}

fn cgf_test_common_grams_query_filter() -> Result<()> {
    let a = whitespace_analyzer(
        "org.apache.lucene.analysis.commongrams.TestCommonGramsFilter$1",
        |tokenizer| {
            Ok(boxed(new_common_grams_query_filter(
                new_common_grams_filter(tokenizer, Some(common_words()))?,
            )?))
        },
    );
    // Stop words used below are "of" "the" and "s"
    // two word queries
    B::assert_analyzes_to(&*a, "brown fox", &["brown", "fox"])?;
    B::assert_analyzes_to(&*a, "the fox", &["the_fox"])?;
    B::assert_analyzes_to(&*a, "fox of", &["fox_of"])?;
    B::assert_analyzes_to(&*a, "of the", &["of_the"])?;
    // one word queries
    B::assert_analyzes_to(&*a, "the", &["the"])?;
    B::assert_analyzes_to(&*a, "foo", &["foo"])?;
    // 3 word combinations s=stopword/common word n=not a stop word
    B::assert_analyzes_to(&*a, "n n n", &["n", "n", "n"])?;
    B::assert_analyzes_to(&*a, "quick brown fox", &["quick", "brown", "fox"])?;
    B::assert_analyzes_to(&*a, "n n s", &["n", "n_s"])?;
    B::assert_analyzes_to(&*a, "quick brown the", &["quick", "brown_the"])?;
    B::assert_analyzes_to(&*a, "n s n", &["n_s", "s_n"])?;
    B::assert_analyzes_to(&*a, "quick the brown", &["quick_the", "the_brown"])?;
    B::assert_analyzes_to(&*a, "n s s", &["n_s", "s_s"])?;
    B::assert_analyzes_to(&*a, "fox of the", &["fox_of", "of_the"])?;
    B::assert_analyzes_to(&*a, "s n n", &["s_n", "n", "n"])?;
    B::assert_analyzes_to(&*a, "the quick brown", &["the_quick", "quick", "brown"])?;
    B::assert_analyzes_to(&*a, "s n s", &["s_n", "n_s"])?;
    B::assert_analyzes_to(&*a, "the fox of", &["the_fox", "fox_of"])?;
    B::assert_analyzes_to(&*a, "s s n", &["s_s", "s_n"])?;
    B::assert_analyzes_to(&*a, "of the fox", &["of_the", "the_fox"])?;
    B::assert_analyzes_to(&*a, "s s s", &["s_s", "s_s"])?;
    B::assert_analyzes_to(&*a, "of the of", &["of_the", "the_of"])?;
    a.close();
    Ok(())
}

fn cgf_test_common_grams_filter() -> Result<()> {
    let a = whitespace_analyzer(
        "org.apache.lucene.analysis.commongrams.TestCommonGramsFilter$2",
        |tokenizer| {
            Ok(boxed(new_common_grams_filter(
                tokenizer,
                Some(common_words()),
            )?))
        },
    );
    // Stop words used below are "of" "the" and "s"
    // one word queries
    B::assert_analyzes_to(&*a, "the", &["the"])?;
    B::assert_analyzes_to(&*a, "foo", &["foo"])?;
    // two word queries
    B::assert_analyzes_to_pos_increments(&*a, "brown fox", &["brown", "fox"], &[1, 1])?;
    B::assert_analyzes_to_pos_increments(&*a, "the fox", &["the", "the_fox", "fox"], &[1, 0, 1])?;
    B::assert_analyzes_to_pos_increments(&*a, "fox of", &["fox", "fox_of", "of"], &[1, 0, 1])?;
    B::assert_analyzes_to_pos_increments(&*a, "of the", &["of", "of_the", "the"], &[1, 0, 1])?;
    // 3 word combinations s=stopword/common word n=not a stop word
    B::assert_analyzes_to_pos_increments(&*a, "n n n", &["n", "n", "n"], &[1, 1, 1])?;
    B::assert_analyzes_to_pos_increments(
        &*a,
        "quick brown fox",
        &["quick", "brown", "fox"],
        &[1, 1, 1],
    )?;
    B::assert_analyzes_to_pos_increments(&*a, "n n s", &["n", "n", "n_s", "s"], &[1, 1, 0, 1])?;
    B::assert_analyzes_to_pos_increments(
        &*a,
        "quick brown the",
        &["quick", "brown", "brown_the", "the"],
        &[1, 1, 0, 1],
    )?;
    B::assert_analyzes_to_pos_increments(
        &*a,
        "n s n",
        &["n", "n_s", "s", "s_n", "n"],
        &[1, 0, 1, 0, 1],
    )?;
    B::assert_analyzes_to_pos_increments(
        &*a,
        "quick the fox",
        &["quick", "quick_the", "the", "the_fox", "fox"],
        &[1, 0, 1, 0, 1],
    )?;
    B::assert_analyzes_to_pos_increments(
        &*a,
        "n s s",
        &["n", "n_s", "s", "s_s", "s"],
        &[1, 0, 1, 0, 1],
    )?;
    B::assert_analyzes_to_pos_increments(
        &*a,
        "fox of the",
        &["fox", "fox_of", "of", "of_the", "the"],
        &[1, 0, 1, 0, 1],
    )?;
    B::assert_analyzes_to_pos_increments(&*a, "s n n", &["s", "s_n", "n", "n"], &[1, 0, 1, 1])?;
    B::assert_analyzes_to_pos_increments(
        &*a,
        "the quick brown",
        &["the", "the_quick", "quick", "brown"],
        &[1, 0, 1, 1],
    )?;
    B::assert_analyzes_to_pos_increments(
        &*a,
        "s n s",
        &["s", "s_n", "n", "n_s", "s"],
        &[1, 0, 1, 0, 1],
    )?;
    B::assert_analyzes_to_pos_increments(
        &*a,
        "the fox of",
        &["the", "the_fox", "fox", "fox_of", "of"],
        &[1, 0, 1, 0, 1],
    )?;
    B::assert_analyzes_to_pos_increments(
        &*a,
        "s s n",
        &["s", "s_s", "s", "s_n", "n"],
        &[1, 0, 1, 0, 1],
    )?;
    B::assert_analyzes_to_pos_increments(
        &*a,
        "of the fox",
        &["of", "of_the", "the", "the_fox", "fox"],
        &[1, 0, 1, 0, 1],
    )?;
    B::assert_analyzes_to_pos_increments(
        &*a,
        "s s s",
        &["s", "s_s", "s", "s_s", "s"],
        &[1, 0, 1, 0, 1],
    )?;
    B::assert_analyzes_to_pos_increments(
        &*a,
        "of the of",
        &["of", "of_the", "the", "the_of", "of"],
        &[1, 0, 1, 0, 1],
    )?;
    a.close();
    Ok(())
}

fn cgf_test_case_sensitive() -> Result<()> {
    let input = "How The s a brown s cow d like A B thing?";
    let mut wt = MockTokenizer::new(Arc::clone(&WHITESPACE), false)?;
    wt.set_reader(string_reader(input))?;
    let mut cgf = new_common_grams_filter(boxed(wt), Some(common_words()))?;
    B::assert_token_stream_contents(
        &mut cgf,
        &[
            "How", "The", "The_s", "s", "s_a", "a", "a_brown", "brown", "brown_s", "s", "s_cow",
            "cow", "cow_d", "d", "d_like", "like", "A", "B", "thing?",
        ],
    )
}

fn cgf_test_last_wordis_stop_word() -> Result<()> {
    query_filter_of("dog the", &["dog_the"])
}

fn cgf_test_first_wordis_stop_word() -> Result<()> {
    query_filter_of("the dog", &["the_dog"])
}

fn cgf_test_one_word_query_stop_word() -> Result<()> {
    query_filter_of("the", &["the"])
}

fn cgf_test_one_word_query() -> Result<()> {
    query_filter_of("monster", &["monster"])
}

fn cgf_test_random_strings() -> Result<()> {
    let a = whitespace_analyzer(
        "org.apache.lucene.analysis.commongrams.TestCommonGramsFilter$4",
        |t| Ok(boxed(new_common_grams_filter(t, Some(common_words()))?)),
    );
    B::check_random_data(&*random(), &a, 200 * mult())?;
    a.close();
    let b = whitespace_analyzer(
        "org.apache.lucene.analysis.commongrams.TestCommonGramsFilter$5",
        |t| {
            let cgf = new_common_grams_filter(t, Some(common_words()))?;
            Ok(boxed(new_common_grams_query_filter(cgf)?))
        },
    );
    B::check_random_data(&*random(), &b, 200 * mult())?;
    b.close();
    Ok(())
}
fn common_grams_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.commongrams.TestCommonGramsFilter")
        .method(t("testReset", cgf_test_reset))
        .method(t("testQueryReset", cgf_test_query_reset))
        .method(t(
            "testCommonGramsQueryFilter",
            cgf_test_common_grams_query_filter,
        ))
        .method(t("testCommonGramsFilter", cgf_test_common_grams_filter))
        .method(t("testCaseSensitive", cgf_test_case_sensitive))
        .method(t("testLastWordisStopWord", cgf_test_last_wordis_stop_word))
        .method(t(
            "testFirstWordisStopWord",
            cgf_test_first_wordis_stop_word,
        ))
        .method(t(
            "testOneWordQueryStopWord",
            cgf_test_one_word_query_stop_word,
        ))
        .method(t("testOneWordQuery", cgf_test_one_word_query))
        .method(t("testRandomStrings", cgf_test_random_strings))
        .build()
}

#[test]
fn test_common_grams_filter() {
    run(common_grams_filter_class);
}

#[test]
fn test_common_grams_filter_factory() {
    run(common_grams_filter_factory_class);
}

#[test]
fn test_common_grams_query_filter_factory() {
    run(common_grams_query_filter_factory_class);
}

#[test]
fn test_edge_ngram_token_filter() {
    run(edge_ngram_token_filter_class);
}

#[test]
fn test_edge_ngram_tokenizer() {
    run(edge_ngram_tokenizer_class);
}

#[test]
fn test_fixed_shingle_filter() {
    run(fixed_shingle_filter_class);
}

#[test]
fn test_min_hash_filter() {
    run(min_hash_filter_class);
}

#[test]
fn test_ngram_token_filter() {
    run(ngram_token_filter_class);
}

#[test]
fn test_ngram_tokenizer() {
    run(ngram_tokenizer_class);
}

#[test]
fn test_shingle_analyzer_wrapper() {
    run(shingle_analyzer_wrapper_class);
}

#[test]
fn test_shingle_filter() {
    run(shingle_filter_class);
}

#[test]
fn test_shingle_filter_factory() {
    run(shingle_filter_factory_class);
}
