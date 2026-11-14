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

//! Port of the `org.apache.lucene.analysis.miscellaneous` test classes of
//! Lucene 10.5.0 `lucene/analysis/common/src/test` that rmp task #450
//! lists, each run as a suite by the ported runner.
//!
//! `TestFixBrokenOffsetsFilter` is `@Deprecated` and exercises the
//! deprecated `FixBrokenOffsetsFilter`, and `TestTruncateTokenFilterFactory`
//! the deprecated `PREFIX_LENGTH_KEY`, as in Lucene: deprecation warnings are
//! allowed here.

#![allow(deprecated)]

mod support;

#[path = "analysis_miscellaneous/ascii_foldings.rs"]
mod ascii_foldings;

use std::cell::RefCell;
use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{Assert, Test};
use rucene::analysis::java_regex::JavaPattern;
use rucene::analysis::tokenattributes::{
    CharTermAttribute, FlagsAttribute, KeywordAttribute, OffsetAttribute,
    PositionIncrementAttribute, TermFrequencyAttribute, TermToBytesRefAttribute, TypeAttribute,
};
use rucene::analysis::{
    Analyzer, AnalyzerState, AnalyzerWrapper, AnalyzerWrapperLogic, CharArraySet, CharReader,
    ReuseStrategy, SharedTokenStream, StringCharReader, TokenFilter, TokenFilterLogic, TokenStream,
    TokenStreamComponents,
};
use rucene::document::{Document, Field, FieldType, Store, StringField, TextField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::Term;
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::extra::Version;
use rucene::util::AttributeSource;
use rucene_analysis_common::analysis::core::keyword_tokenizer::new_keyword_tokenizer;
use rucene_analysis_common::analysis::core::simple_analyzer::SimpleAnalyzer;
use rucene_analysis_common::analysis::core::whitespace_analyzer::WhitespaceAnalyzer;
use rucene_analysis_common::analysis::miscellaneous::ascii_folding_filter::{
    new_ascii_folding_filter, new_ascii_folding_filter_preserving, ASCIIFoldingFilter,
};
use rucene_analysis_common::analysis::miscellaneous::capitalization_filter::{
    new_capitalization_filter, new_capitalization_filter_with, DEFAULT_MAX_TOKEN_LENGTH,
    DEFAULT_MAX_WORD_COUNT,
};
use rucene_analysis_common::analysis::miscellaneous::codepoint_count_filter::new_codepoint_count_filter;
use rucene_analysis_common::analysis::miscellaneous::codepoint_count_filter_factory::CodepointCountFilterFactory;
use rucene_analysis_common::analysis::miscellaneous::delimited_term_frequency_token_filter::new_delimited_term_frequency_token_filter;
use rucene_analysis_common::analysis::miscellaneous::drop_if_flagged_filter::new_drop_if_flagged_filter;
use rucene_analysis_common::analysis::miscellaneous::empty_token_stream::EmptyTokenStream;
use rucene_analysis_common::analysis::miscellaneous::fix_broken_offsets_filter::new_fix_broken_offsets_filter;
use rucene_analysis_common::analysis::miscellaneous::hyphenated_words_filter::new_hyphenated_words_filter;
use rucene_analysis_common::analysis::miscellaneous::keep_word_filter::new_keep_word_filter;
use rucene_analysis_common::analysis::miscellaneous::keep_word_filter_factory::KeepWordFilterFactory;
use rucene_analysis_common::analysis::miscellaneous::length_filter::new_length_filter;
use rucene_analysis_common::analysis::miscellaneous::length_filter_factory::LengthFilterFactory;
use rucene_analysis_common::analysis::miscellaneous::limit_token_count_analyzer::LimitTokenCountAnalyzer;
use rucene_analysis_common::analysis::miscellaneous::limit_token_count_filter::{
    new_limit_token_count_filter, new_limit_token_count_filter_consuming,
};
use rucene_analysis_common::analysis::miscellaneous::limit_token_count_filter_factory::LimitTokenCountFilterFactory;
use rucene_analysis_common::analysis::miscellaneous::limit_token_offset_filter::{
    new_limit_token_offset_filter, new_limit_token_offset_filter_consuming,
};
use rucene_analysis_common::analysis::miscellaneous::limit_token_offset_filter_factory::LimitTokenOffsetFilterFactory;
use rucene_analysis_common::analysis::miscellaneous::limit_token_position_filter_factory::LimitTokenPositionFilterFactory;
use rucene_analysis_common::analysis::miscellaneous::pattern_keyword_marker_filter::new_pattern_keyword_marker_filter;
use rucene_analysis_common::analysis::miscellaneous::per_field_analyzer_wrapper::PerFieldAnalyzerWrapper;
use rucene_analysis_common::analysis::miscellaneous::scandinavian_folding_filter::new_scandinavian_folding_filter;
use rucene_analysis_common::analysis::miscellaneous::scandinavian_normalization_filter::new_scandinavian_normalization_filter;
use rucene_analysis_common::analysis::miscellaneous::scandinavian_normalizer::{
    Foldings, ScandinavianNormalizer,
};
use rucene_analysis_common::analysis::miscellaneous::set_keyword_marker_filter::new_set_keyword_marker_filter;
use rucene_analysis_common::analysis::miscellaneous::trim_filter::new_trim_filter;
use rucene_analysis_common::analysis::miscellaneous::truncate_token_filter::{
    truncate_after_chars, truncate_after_code_points,
};
use rucene_analysis_common::analysis::miscellaneous::truncate_token_filter_factory::TruncateTokenFilterFactory;
use rucene_analysis_common::analysis::miscellaneous::type_as_synonym_filter::{
    new_type_as_synonym_filter, new_type_as_synonym_filter_with,
};
use rucene_test_framework::analysis::mock_tokenizer::{MockTokenizer, KEYWORD, WHITESPACE};
use rucene_test_framework::analysis::{
    sink, tokenizer_components, BaseTokenStreamTestCase, CannedTokenStream, MockAnalyzer,
    MockCharFilter, Token,
};
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::RANDOM_MULTIPLIER;
use rucene_test_framework::util::string_mock_resource_loader::StringMockResourceLoader;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};
use support::{
    boxed, btstc_class, concrete_factory, factory_class, hook, iae_message,
    keyword_tokenizer_analyzer, mock_tokenizer_analyzer, run, t, term, token_filter_factory,
    token_filter_factory_version, whitespace_analyzer, FnAnalyzer,
};

type B = BaseTokenStreamTestCase;

/// The package of the test classes, which resolves their resources.
const MISC: &str = "org.apache.lucene.analysis.miscellaneous";

/// `random()`.
fn random() -> Arc<dyn Random> {
    LuceneTestCase::random()
}

/// `whitespaceMockTokenizer(String)`.
fn ws(input: &str) -> Result<MockTokenizer> {
    B::whitespace_mock_tokenizer_str(input)
}

/// `keywordMockTokenizer(String)`.
fn kw(input: &str) -> Result<MockTokenizer> {
    B::keyword_mock_tokenizer_str(input)
}

/// `tokenFilterFactory(name, keysAndValues...)` of this package.
fn fac(
    name: &str,
    keys_and_values: &[&str],
) -> Result<Arc<dyn rucene::analysis::factories::TokenFilterFactory>> {
    token_filter_factory(MISC, name, keys_and_values)
}

/// `assertTrue(expected.getMessage().contains("Unknown parameters"))` over
/// `tokenFilterFactory(name, keysAndValues...)`.
fn assert_bogus(name: &str, keys_and_values: &[&str]) -> Result<()> {
    let message = iae_message(|| fac(name, keys_and_values));
    Assert::assert_true(message.contains("Unknown parameters"));
    Ok(())
}

/// `new Token(text, start, end)`.
fn tok(text: &str, start: i32, end: i32) -> Token {
    Token::with_offsets(text, start, end)
}

/// `CannedTokenStream(Token... tokens)` as a boxed stream.
fn canned(tokens: Vec<Token>) -> Box<dyn TokenStream> {
    Box::new(CannedTokenStream::new(tokens))
}

/// An `@Test(expected = IllegalArgumentException.class)` method.
fn t_iae(name: &str, body: fn() -> Result<()>) -> randomizedtesting::jdk::reflect::Method {
    t(name, body).annotate(Test::expected("java.lang.IllegalArgumentException"))
}

// -----------------------------------------------------------------------------
// TestASCIIFoldingFilter
// -----------------------------------------------------------------------------

/// Pop one input token's worth of tokens off the filter and verify that they
/// are as expected.
fn assert_next_terms(
    expected_unfolded: &str,
    expected_folded: &str,
    filter: &mut ASCIIFoldingFilter,
) -> Result<()> {
    Assert::assert_true(filter.increment_token()?);
    Assert::assert_equals(expected_folded, term(filter).as_str());
    if filter.logic().is_preserve_original() && expected_unfolded != expected_folded {
        Assert::assert_true(filter.increment_token()?);
        Assert::assert_equals(expected_unfolded, term(filter).as_str());
    }
    Ok(())
}

// testLain1Accents() is a copy of TestLatin1AccentFilter.testU().
fn af_test_latin1_accents() -> Result<()> {
    let stream = ws(concat!(
        "Des mot clés À LA CHAÎNE À Á Â Ã Ä Å Æ Ç È É Ê Ë Ì Í Î Ï Ĳ Ð Ñ",
        " Ò Ó Ô Õ Ö Ø Œ Þ Ù Ú Û Ü Ý Ÿ à á â ã ä å æ ç è é ê ë ì í î ï ĳ",
        " ð ñ ò ó ô õ ö ø œ ß þ ù ú û ü ý ÿ ﬁ ﬂ"
    ))?;
    let mut filter = new_ascii_folding_filter_preserving(boxed(stream), random().next_boolean())?;
    filter.reset()?;
    for (unfolded, folded) in [
        ("Des", "Des"),
        ("mot", "mot"),
        ("clés", "cles"),
        ("À", "A"),
        ("LA", "LA"),
        ("CHAÎNE", "CHAINE"),
        ("À", "A"),
        ("Á", "A"),
        ("Â", "A"),
        ("Ã", "A"),
        ("Ä", "A"),
        ("Å", "A"),
        ("Æ", "AE"),
        ("Ç", "C"),
        ("È", "E"),
        ("É", "E"),
        ("Ê", "E"),
        ("Ë", "E"),
        ("Ì", "I"),
        ("Í", "I"),
        ("Î", "I"),
        ("Ï", "I"),
        ("Ĳ", "IJ"),
        ("Ð", "D"),
        ("Ñ", "N"),
        ("Ò", "O"),
        ("Ó", "O"),
        ("Ô", "O"),
        ("Õ", "O"),
        ("Ö", "O"),
        ("Ø", "O"),
        ("Œ", "OE"),
        ("Þ", "TH"),
        ("Ù", "U"),
        ("Ú", "U"),
        ("Û", "U"),
        ("Ü", "U"),
        ("Ý", "Y"),
        ("Ÿ", "Y"),
        ("à", "a"),
        ("á", "a"),
        ("â", "a"),
        ("ã", "a"),
        ("ä", "a"),
        ("å", "a"),
        ("æ", "ae"),
        ("ç", "c"),
        ("è", "e"),
        ("é", "e"),
        ("ê", "e"),
        ("ë", "e"),
        ("ì", "i"),
        ("í", "i"),
        ("î", "i"),
        ("ï", "i"),
        ("ĳ", "ij"),
        ("ð", "d"),
        ("ñ", "n"),
        ("ò", "o"),
        ("ó", "o"),
        ("ô", "o"),
        ("õ", "o"),
        ("ö", "o"),
        ("ø", "o"),
        ("œ", "oe"),
        ("ß", "ss"),
        ("þ", "th"),
        ("ù", "u"),
        ("ú", "u"),
        ("û", "u"),
        ("ü", "u"),
        ("ý", "y"),
        ("ÿ", "y"),
        ("ﬁ", "fi"),
        ("ﬂ", "fl"),
    ] {
        assert_next_terms(unfolded, folded, &mut filter)?;
    }
    Assert::assert_false(filter.increment_token()?);
    Ok(())
}

// Test that we do not emit duplicated tokens when preserve original is on
fn af_test_unmodified_letters() -> Result<()> {
    let stream = ws("§ ¦ ¤ END")?;
    let mut filter = new_ascii_folding_filter_preserving(boxed(stream), true)?;
    filter.reset()?;
    assert_next_terms("§", "§", &mut filter)?;
    assert_next_terms("¦", "¦", &mut filter)?;
    assert_next_terms("¤", "¤", &mut filter)?;
    assert_next_terms("END", "END", &mut filter)?;
    Assert::assert_false(filter.increment_token()?);
    Ok(())
}

fn af_test_all_foldings() -> Result<()> {
    let foldings = ascii_foldings::FOLDINGS;

    // Construct input text and expected output tokens
    let mut expected_unfolded_tokens: Vec<String> = Vec::new();
    let mut expected_folded_tokens: Vec<String> = Vec::new();
    let mut input_text = String::new();
    let mut n = 0;
    while n < foldings.len() {
        if n > 0 {
            input_text.push(' '); // Space between tokens
        }
        input_text.push_str(foldings[n]);

        // Construct the expected output tokens: both the unfolded and folded string,
        // with the folded duplicated as many times as the number of characters in
        // the input text.
        let mut expected = String::new();
        let num_chars = foldings[n].encode_utf16().count();
        for _ in 0..num_chars {
            expected.push_str(foldings[n + 1]);
        }
        expected_unfolded_tokens.push(foldings[n].to_string());
        expected_folded_tokens.push(expected);
        n += 2;
    }

    let stream = ws(&input_text)?;
    let mut filter = new_ascii_folding_filter_preserving(boxed(stream), random().next_boolean())?;
    filter.reset()?;
    for (unfolded, folded) in expected_unfolded_tokens
        .iter()
        .zip(expected_folded_tokens.iter())
    {
        assert_next_terms(unfolded, folded, &mut filter)?;
    }
    Assert::assert_false(filter.increment_token()?);
    Ok(())
}

/// blast some random strings through the analyzer
fn af_test_random_strings() -> Result<()> {
    let a = whitespace_analyzer(
        "org.apache.lucene.analysis.miscellaneous.TestASCIIFoldingFilter$1",
        |tokenizer| {
            Ok(boxed(new_ascii_folding_filter_preserving(
                tokenizer,
                random().next_boolean(),
            )?))
        },
    );
    B::check_random_data(&*random(), &a, 200 * *RANDOM_MULTIPLIER)?;
    a.close();
    Ok(())
}

fn af_test_empty_term() -> Result<()> {
    let a = keyword_tokenizer_analyzer(
        "org.apache.lucene.analysis.miscellaneous.TestASCIIFoldingFilter$2",
        |tokenizer| {
            Ok(boxed(new_ascii_folding_filter_preserving(
                tokenizer,
                random().next_boolean(),
            )?))
        },
    );
    B::check_one_term(&*a, "", "")?;
    a.close();
    Ok(())
}

fn ascii_folding_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.miscellaneous.TestASCIIFoldingFilter")
        .method(t("testLatin1Accents", af_test_latin1_accents))
        .method(t("testUnmodifiedLetters", af_test_unmodified_letters))
        .method(t("testAllFoldings", af_test_all_foldings))
        .method(t("testRandomStrings", af_test_random_strings))
        .method(t("testEmptyTerm", af_test_empty_term))
        .build()
}

// -----------------------------------------------------------------------------
// TestAsciiFoldingFilterFactory
// -----------------------------------------------------------------------------

fn aff_test_multi_term_analysis() -> Result<()> {
    use rucene::analysis::factories::TokenFilterFactory;
    use rucene_analysis_common::analysis::miscellaneous::ascii_folding_filter_factory::ASCIIFoldingFilterFactory;

    let factory = ASCIIFoldingFilterFactory::new(&mut std::collections::HashMap::new())?;
    let stream = canned(vec![tok("Été", 0, 3)]);
    let mut stream = factory.create(stream)?;
    B::assert_token_stream_contents(&mut *stream, &["Ete"])?;

    let stream = canned(vec![tok("Été", 0, 3)]);
    let mut stream = factory.normalize(stream)?;
    B::assert_token_stream_contents(&mut *stream, &["Ete"])?;

    let factory =
        ASCIIFoldingFilterFactory::new(&mut support::make_args(&["preserveOriginal", "true"]))?;
    let stream = canned(vec![tok("Été", 0, 3)]);
    let mut stream = factory.create(stream)?;
    B::assert_token_stream_contents(&mut *stream, &["Ete", "Été"])?;

    let stream = canned(vec![tok("Été", 0, 3)]);
    let mut stream = factory.normalize(stream)?;
    B::assert_token_stream_contents(&mut *stream, &["Ete"])
}

fn ascii_folding_filter_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.miscellaneous.TestAsciiFoldingFilterFactory")
        .method(t("testMultiTermAnalysis", aff_test_multi_term_analysis))
        .build()
}

// -----------------------------------------------------------------------------
// TestCapitalizationFilter
// -----------------------------------------------------------------------------

/// `assertCapitalizesTo(Tokenizer tokenizer, String[] expected, ...)`.
#[allow(clippy::too_many_arguments)]
fn assert_capitalizes_to_stream(
    tokenizer: MockTokenizer,
    expected: &[&str],
    only_first_word: bool,
    keep: Option<&CharArraySet>,
    force_first_letter: bool,
    ok_prefix: Option<Vec<Vec<u16>>>,
    min_word_length: i32,
    max_word_count: i32,
    max_token_length: i32,
) -> Result<()> {
    let mut filter = new_capitalization_filter_with(
        boxed(tokenizer),
        only_first_word,
        keep.cloned(),
        force_first_letter,
        ok_prefix,
        min_word_length,
        max_word_count,
        max_token_length,
    )?;
    B::assert_token_stream_contents(&mut filter, expected)
}

/// `assertCapitalizesTo(String input, String[] expected, ...)`.
#[allow(clippy::too_many_arguments)]
fn assert_capitalizes_to(
    input: &str,
    expected: &[&str],
    only_first_word: bool,
    keep: Option<&CharArraySet>,
    force_first_letter: bool,
    ok_prefix: Option<Vec<Vec<u16>>>,
    min_word_length: i32,
    max_word_count: i32,
    max_token_length: i32,
) -> Result<()> {
    let mut tokenizer = MockTokenizer::new(Arc::clone(&WHITESPACE), false)?;
    tokenizer.set_reader(Box::new(StringCharReader::new(input)))?;
    assert_capitalizes_to_stream(
        tokenizer,
        expected,
        only_first_word,
        keep,
        force_first_letter,
        ok_prefix,
        min_word_length,
        max_word_count,
        max_token_length,
    )
}

/// `assertCapitalizesToKeyword(String input, String expected, ...)`.
#[allow(clippy::too_many_arguments)]
fn assert_capitalizes_to_keyword(
    input: &str,
    expected: &str,
    only_first_word: bool,
    keep: Option<&CharArraySet>,
    force_first_letter: bool,
    ok_prefix: Option<Vec<Vec<u16>>>,
    min_word_length: i32,
    max_word_count: i32,
    max_token_length: i32,
) -> Result<()> {
    let mut tokenizer = MockTokenizer::new(Arc::clone(&KEYWORD), false)?;
    tokenizer.set_reader(Box::new(StringCharReader::new(input)))?;
    assert_capitalizes_to_stream(
        tokenizer,
        &[expected],
        only_first_word,
        keep,
        force_first_letter,
        ok_prefix,
        min_word_length,
        max_word_count,
        max_token_length,
    )
}

fn cap_test_capitalization() -> Result<()> {
    let keep = CharArraySet::from_iter(["and", "the", "it", "BIG"], false);
    let k = Some(&keep);
    let (mwc, mtl) = (DEFAULT_MAX_WORD_COUNT, DEFAULT_MAX_TOKEN_LENGTH);

    assert_capitalizes_to("kiTTEN", &["Kitten"], true, k, true, None, 0, mwc, mtl)?;
    assert_capitalizes_to("and", &["And"], true, k, true, None, 0, mwc, mtl)?;
    assert_capitalizes_to("AnD", &["And"], true, k, true, None, 0, mwc, mtl)?;
    // first is not forced, but it's not a keep word, either
    assert_capitalizes_to("AnD", &["And"], true, k, false, None, 0, mwc, mtl)?;
    assert_capitalizes_to("big", &["Big"], true, k, true, None, 0, mwc, mtl)?;
    assert_capitalizes_to("BIG", &["BIG"], true, k, true, None, 0, mwc, mtl)?;
    assert_capitalizes_to_keyword(
        "Hello thEre my Name is Ryan",
        "Hello there my name is ryan",
        true,
        k,
        true,
        None,
        0,
        mwc,
        mtl,
    )?;
    // now each token
    assert_capitalizes_to(
        "Hello thEre my Name is Ryan",
        &["Hello", "There", "My", "Name", "Is", "Ryan"],
        false,
        k,
        true,
        None,
        0,
        mwc,
        mtl,
    )?;
    // now only the long words
    assert_capitalizes_to(
        "Hello thEre my Name is Ryan",
        &["Hello", "There", "my", "Name", "is", "Ryan"],
        false,
        k,
        true,
        None,
        3,
        mwc,
        mtl,
    )?;
    // without prefix
    assert_capitalizes_to("McKinley", &["Mckinley"], true, k, true, None, 0, mwc, mtl)?;
    // Now try some prefixes
    let ok_prefix: Vec<Vec<u16>> = vec!["McK".encode_utf16().collect()];
    assert_capitalizes_to(
        "McKinley",
        &["McKinley"],
        true,
        k,
        true,
        Some(ok_prefix),
        0,
        mwc,
        mtl,
    )?;
    // now try some stuff with numbers
    assert_capitalizes_to(
        "1st 2nd third",
        &["1st", "2nd", "Third"],
        false,
        k,
        false,
        None,
        0,
        mwc,
        mtl,
    )?;
    assert_capitalizes_to_keyword(
        "the The the",
        "The The the",
        false,
        k,
        true,
        None,
        0,
        mwc,
        mtl,
    )
}

/// blast some random strings through the analyzer
fn cap_test_random_string() -> Result<()> {
    let a = whitespace_analyzer(
        "org.apache.lucene.analysis.miscellaneous.TestCapitalizationFilter$1",
        |tokenizer| Ok(boxed(new_capitalization_filter(tokenizer)?)),
    );
    B::check_random_data(&*random(), &a, 200 * *RANDOM_MULTIPLIER)?;
    a.close();
    Ok(())
}

fn cap_test_empty_term() -> Result<()> {
    let a = keyword_tokenizer_analyzer(
        "org.apache.lucene.analysis.miscellaneous.TestCapitalizationFilter$2",
        |tokenizer| Ok(boxed(new_capitalization_filter(tokenizer)?)),
    );
    B::check_one_term(&*a, "", "")?;
    a.close();
    Ok(())
}

/// checking the validity of constructor arguments
fn cap_test_illegal_arguments() -> Result<()> {
    new_capitalization_filter_with(
        boxed(ws("accept only valid arguments")?),
        true,
        None,
        true,
        None,
        -1,
        DEFAULT_MAX_WORD_COUNT,
        DEFAULT_MAX_TOKEN_LENGTH,
    )?;
    Ok(())
}

fn cap_test_illegal_arguments1() -> Result<()> {
    new_capitalization_filter_with(
        boxed(ws("accept only valid arguments")?),
        true,
        None,
        true,
        None,
        0,
        -10,
        DEFAULT_MAX_TOKEN_LENGTH,
    )?;
    Ok(())
}

fn cap_test_illegal_arguments2() -> Result<()> {
    new_capitalization_filter_with(
        boxed(ws("accept only valid arguments")?),
        true,
        None,
        true,
        None,
        0,
        DEFAULT_MAX_WORD_COUNT,
        -50,
    )?;
    Ok(())
}

fn capitalization_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.miscellaneous.TestCapitalizationFilter")
        .method(t("testCapitalization", cap_test_capitalization))
        .method(t("testRandomString", cap_test_random_string))
        .method(t("testEmptyTerm", cap_test_empty_term))
        .method(t_iae("testIllegalArguments", cap_test_illegal_arguments))
        .method(t_iae("testIllegalArguments1", cap_test_illegal_arguments1))
        .method(t_iae("testIllegalArguments2", cap_test_illegal_arguments2))
        .build()
}

// -----------------------------------------------------------------------------
// TestCapitalizationFilterFactory
// -----------------------------------------------------------------------------

/// The body every `testCapitalization*` method shares: the input through
/// a whitespace (`keyword` false) or keyword mock tokenizer, then the
/// `Capitalization` factory with `keys_and_values`.
fn capf_check(
    input: &str,
    keyword: bool,
    keys_and_values: &[&str],
    expected: &[&str],
) -> Result<()> {
    let stream: Box<dyn TokenStream> = if keyword {
        boxed(kw(input)?)
    } else {
        boxed(ws(input)?)
    };
    let mut stream = fac("Capitalization", keys_and_values)?.create(stream)?;
    B::assert_token_stream_contents(&mut *stream, expected)
}

fn capf_test_capitalization() -> Result<()> {
    capf_check(
        "kiTTEN",
        false,
        &["keep", "and the it BIG", "onlyFirstWord", "true"],
        &["Kitten"],
    )
}

fn capf_test_capitalization2() -> Result<()> {
    capf_check(
        "and",
        false,
        &[
            "keep",
            "and the it BIG",
            "onlyFirstWord",
            "true",
            "forceFirstLetter",
            "true",
        ],
        &["And"],
    )
}

/// first is forced, but it's not a keep word, either
fn capf_test_capitalization3() -> Result<()> {
    capf_check(
        "AnD",
        false,
        &[
            "keep",
            "and the it BIG",
            "onlyFirstWord",
            "true",
            "forceFirstLetter",
            "true",
        ],
        &["And"],
    )
}

fn capf_test_capitalization4() -> Result<()> {
    capf_check(
        "AnD",
        false,
        &[
            "keep",
            "and the it BIG",
            "onlyFirstWord",
            "true",
            "forceFirstLetter",
            "false",
        ],
        &["And"],
    )
}

fn capf_test_capitalization5() -> Result<()> {
    capf_check(
        "big",
        false,
        &[
            "keep",
            "and the it BIG",
            "onlyFirstWord",
            "true",
            "forceFirstLetter",
            "true",
        ],
        &["Big"],
    )
}

fn capf_test_capitalization6() -> Result<()> {
    capf_check(
        "BIG",
        false,
        &[
            "keep",
            "and the it BIG",
            "onlyFirstWord",
            "true",
            "forceFirstLetter",
            "true",
        ],
        &["BIG"],
    )
}

fn capf_test_capitalization7() -> Result<()> {
    capf_check(
        "Hello thEre my Name is Ryan",
        true,
        &[
            "keep",
            "and the it BIG",
            "onlyFirstWord",
            "true",
            "forceFirstLetter",
            "true",
        ],
        &["Hello there my name is ryan"],
    )
}

fn capf_test_capitalization8() -> Result<()> {
    capf_check(
        "Hello thEre my Name is Ryan",
        false,
        &[
            "keep",
            "and the it BIG",
            "onlyFirstWord",
            "false",
            "forceFirstLetter",
            "true",
        ],
        &["Hello", "There", "My", "Name", "Is", "Ryan"],
    )
}

fn capf_test_capitalization9() -> Result<()> {
    capf_check(
        "Hello thEre my Name is Ryan",
        false,
        &[
            "keep",
            "and the it BIG",
            "onlyFirstWord",
            "false",
            "minWordLength",
            "3",
            "forceFirstLetter",
            "true",
        ],
        &["Hello", "There", "my", "Name", "is", "Ryan"],
    )
}

fn capf_test_capitalization10() -> Result<()> {
    capf_check(
        "McKinley",
        false,
        &[
            "keep",
            "and the it BIG",
            "onlyFirstWord",
            "false",
            "minWordLength",
            "3",
            "forceFirstLetter",
            "true",
        ],
        &["Mckinley"],
    )
}

/// using "McK" as okPrefix
fn capf_test_capitalization11() -> Result<()> {
    capf_check(
        "McKinley",
        false,
        &[
            "keep",
            "and the it BIG",
            "onlyFirstWord",
            "false",
            "minWordLength",
            "3",
            "okPrefix",
            "McK",
            "forceFirstLetter",
            "true",
        ],
        &["McKinley"],
    )
}

/// test with numbers
fn capf_test_capitalization12() -> Result<()> {
    capf_check(
        "1st 2nd third",
        false,
        &[
            "keep",
            "and the it BIG",
            "onlyFirstWord",
            "false",
            "minWordLength",
            "3",
            "okPrefix",
            "McK",
            "forceFirstLetter",
            "false",
        ],
        &["1st", "2nd", "Third"],
    )
}

fn capf_test_capitalization13() -> Result<()> {
    capf_check(
        "the The the",
        true,
        &[
            "keep",
            "and the it BIG",
            "onlyFirstWord",
            "false",
            "minWordLength",
            "3",
            "okPrefix",
            "McK",
            "forceFirstLetter",
            "true",
        ],
        &["The The the"],
    )
}

fn capf_test_keep_ignore_case() -> Result<()> {
    capf_check(
        "kiTTEN",
        true,
        &[
            "keep",
            "kitten",
            "keepIgnoreCase",
            "true",
            "onlyFirstWord",
            "true",
            "forceFirstLetter",
            "true",
        ],
        &["KiTTEN"],
    )
}

fn capf_test_keep_ignore_case2() -> Result<()> {
    capf_check(
        "kiTTEN",
        true,
        &[
            "keep",
            "kitten",
            "keepIgnoreCase",
            "true",
            "onlyFirstWord",
            "true",
            "forceFirstLetter",
            "false",
        ],
        &["kiTTEN"],
    )
}

fn capf_test_keep_ignore_case3() -> Result<()> {
    capf_check(
        "kiTTEN",
        true,
        &[
            "keepIgnoreCase",
            "true",
            "onlyFirstWord",
            "true",
            "forceFirstLetter",
            "false",
        ],
        &["Kitten"],
    )
}

/// Test CapitalizationFilterFactory's minWordLength option.
///
/// This is very weird when combined with ONLY_FIRST_WORD!!!
fn capf_test_min_word_length() -> Result<()> {
    capf_check(
        "helo testing",
        false,
        &["onlyFirstWord", "true", "minWordLength", "5"],
        &["helo", "Testing"],
    )
}

/// Test CapitalizationFilterFactory's maxWordCount option with only words of
/// 1 in each token (it should do nothing)
fn capf_test_max_word_count() -> Result<()> {
    capf_check(
        "one two three four",
        false,
        &["maxWordCount", "2"],
        &["One", "Two", "Three", "Four"],
    )
}

/// Test CapitalizationFilterFactory's maxWordCount option when exceeded
fn capf_test_max_word_count2() -> Result<()> {
    capf_check(
        "one two three four",
        true,
        &["maxWordCount", "2"],
        &["one two three four"],
    )
}

/// Test CapitalizationFilterFactory's maxTokenLength option when exceeded
///
/// This is weird, it is not really a max, but inclusive (look at 'is')
fn capf_test_max_token_length() -> Result<()> {
    capf_check(
        "this is a test",
        false,
        &["maxTokenLength", "2"],
        &["this", "is", "A", "test"],
    )
}

/// Test CapitalizationFilterFactory's forceFirstLetter option
fn capf_test_force_first_letter_with_keep() -> Result<()> {
    capf_check(
        "kitten",
        false,
        &["keep", "kitten", "forceFirstLetter", "true"],
        &["Kitten"],
    )
}

/// Test that bogus arguments result in exception
fn capf_test_bogus_arguments() -> Result<()> {
    assert_bogus("Capitalization", &["bogusArg", "bogusValue"])
}

/// Test that invalid arguments result in exception
fn capf_test_invalid_arguments() -> Result<()> {
    for arg in ["minWordLength", "maxTokenLength", "maxWordCount"] {
        let expected = iae_message(|| {
            let stream = ws("foo foobar super-duper-trooper")?;
            fac(
                "Capitalization",
                &[
                    "keep",
                    "and the it BIG",
                    "onlyFirstWord",
                    "false",
                    arg,
                    "-3",
                    "okPrefix",
                    "McK",
                    "forceFirstLetter",
                    "true",
                ],
            )?
            .create(boxed(stream))
        });
        Assert::assert_true(
            expected.contains(&format!("{arg} must be greater than or equal to zero"))
                || expected.contains(&format!("{arg} must be greater than zero")),
        );
    }
    Ok(())
}

fn capitalization_filter_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.miscellaneous.TestCapitalizationFilterFactory")
        .method(t("testCapitalization", capf_test_capitalization))
        .method(t("testCapitalization2", capf_test_capitalization2))
        .method(t("testCapitalization3", capf_test_capitalization3))
        .method(t("testCapitalization4", capf_test_capitalization4))
        .method(t("testCapitalization5", capf_test_capitalization5))
        .method(t("testCapitalization6", capf_test_capitalization6))
        .method(t("testCapitalization7", capf_test_capitalization7))
        .method(t("testCapitalization8", capf_test_capitalization8))
        .method(t("testCapitalization9", capf_test_capitalization9))
        .method(t("testCapitalization10", capf_test_capitalization10))
        .method(t("testCapitalization11", capf_test_capitalization11))
        .method(t("testCapitalization12", capf_test_capitalization12))
        .method(t("testCapitalization13", capf_test_capitalization13))
        .method(t("testKeepIgnoreCase", capf_test_keep_ignore_case))
        .method(t("testKeepIgnoreCase2", capf_test_keep_ignore_case2))
        .method(t("testKeepIgnoreCase3", capf_test_keep_ignore_case3))
        .method(t("testMinWordLength", capf_test_min_word_length))
        .method(t("testMaxWordCount", capf_test_max_word_count))
        .method(t("testMaxWordCount2", capf_test_max_word_count2))
        .method(t("testMaxTokenLength", capf_test_max_token_length))
        .method(t(
            "testForceFirstLetterWithKeep",
            capf_test_force_first_letter_with_keep,
        ))
        .method(t("testBogusArguments", capf_test_bogus_arguments))
        .method(t("testInvalidArguments", capf_test_invalid_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// TestCodepointCountFilter
// -----------------------------------------------------------------------------

fn ccf_test_filter_with_pos_incr() -> Result<()> {
    let stream = ws("short toolong evenmuchlongertext a ab toolong foo")?;
    let mut filter = new_codepoint_count_filter(boxed(stream), 2, 6)?;
    B::assert_token_stream_contents_pos_increments(&mut filter, &["short", "ab", "foo"], &[1, 4, 2])
}

fn ccf_test_empty_term() -> Result<()> {
    let a = keyword_tokenizer_analyzer(
        "org.apache.lucene.analysis.miscellaneous.TestCodepointCountFilter$1",
        |tokenizer| Ok(boxed(new_codepoint_count_filter(tokenizer, 0, 5)?)),
    );
    B::check_one_term(&*a, "", "")?;
    a.close();
    Ok(())
}

fn ccf_test_random_strings() -> Result<()> {
    for _ in 0..10000 {
        let text = TestUtil::random_unicode_string_max(&*random(), 100);
        let mut min = TestUtil::next_int(&*random(), 0, 100);
        let mut max = TestUtil::next_int(&*random(), 0, 100);
        let count = text.chars().count() as i32;
        if min > max {
            std::mem::swap(&mut min, &mut max);
        }
        let expected = count >= min && count <= max;
        let mut tokenizer = new_keyword_tokenizer();
        tokenizer.set_reader(Box::new(StringCharReader::new(text)))?;
        let mut stream = new_codepoint_count_filter(boxed(tokenizer), min, max)?;
        stream.reset()?;
        Assert::assert_equals(expected, stream.increment_token()?);
        stream.end()?;
        stream.close()?;
    }
    Ok(())
}

/// checking the validity of constructor arguments
fn ccf_test_illegal_arguments() -> Result<()> {
    new_codepoint_count_filter(boxed(ws("accept only valid arguments")?), 4, 1)?;
    Ok(())
}

fn codepoint_count_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.miscellaneous.TestCodepointCountFilter")
        .method(t("testFilterWithPosIncr", ccf_test_filter_with_pos_incr))
        .method(t("testEmptyTerm", ccf_test_empty_term))
        .method(t("testRandomStrings", ccf_test_random_strings))
        .method(t_iae("testIllegalArguments", ccf_test_illegal_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// TestCodepointCountFilterFactory
// -----------------------------------------------------------------------------

fn ccff_test_position_increments() -> Result<()> {
    let mut stream = MockTokenizer::new(Arc::clone(&WHITESPACE), false)?;
    stream.set_reader(Box::new(StringCharReader::new(
        "foo foobar super-duper-trooper",
    )))?;
    let mut stream = fac("CodepointCount", &["min", "4", "max", "10"])?.create(boxed(stream))?;
    B::assert_token_stream_contents_pos_increments(&mut *stream, &["foobar"], &[2])
}

/// Test that bogus arguments result in exception
fn ccff_test_bogus_arguments() -> Result<()> {
    assert_bogus(
        "CodepointCount",
        &["min", "4", "max", "5", "bogusArg", "bogusValue"],
    )
}

/// Test that invalid arguments result in exception
fn ccff_test_invalid_arguments() -> Result<()> {
    let expected = iae_message(|| {
        let mut stream = MockTokenizer::new(Arc::clone(&WHITESPACE), false)?;
        stream.set_reader(Box::new(StringCharReader::new(
            "foo foobar super-duper-trooper",
        )))?;
        fac(
            "CodepointCount",
            &[
                CodepointCountFilterFactory::MIN_KEY,
                "5",
                CodepointCountFilterFactory::MAX_KEY,
                "4",
            ],
        )?
        .create(boxed(stream))
    });
    Assert::assert_true(
        expected.contains("maximum length must not be greater than minimum length"),
    );
    Ok(())
}

fn codepoint_count_filter_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.miscellaneous.TestCodepointCountFilterFactory")
        .method(t("testPositionIncrements", ccff_test_position_increments))
        .method(t("testBogusArguments", ccff_test_bogus_arguments))
        .method(t("testInvalidArguments", ccff_test_invalid_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// TestDelimitedTermFrequencyTokenFilter
// -----------------------------------------------------------------------------

/// `assertTermEquals(String expected, TokenStream stream, termAtt, tfAtt, int expectedTf)`.
fn dtf_assert_term_equals(
    expected: &str,
    stream: &mut dyn TokenStream,
    expected_tf: i32,
) -> Result<()> {
    Assert::assert_true(stream.increment_token()?);
    Assert::assert_equals(expected, term(stream).as_str());
    let tf = stream
        .attribute_source()
        .get_attribute_interface::<dyn TermFrequencyAttribute>()
        .expect("getAttribute(TermFrequencyAttribute.class)")
        .get_term_frequency();
    Assert::assert_equals(expected_tf, tf);
    Ok(())
}

fn dtf_test_term_frequency() -> Result<()> {
    let test = "The quick|40 red|4 fox|06 jumped|1 over the lazy|2 brown|123 dogs|1024";
    let mut filter = new_delimited_term_frequency_token_filter(boxed(ws(test)?))?;
    filter.reset()?;
    dtf_assert_term_equals("The", &mut filter, 1)?;
    dtf_assert_term_equals("quick", &mut filter, 40)?;
    dtf_assert_term_equals("red", &mut filter, 4)?;
    dtf_assert_term_equals("fox", &mut filter, 6)?;
    dtf_assert_term_equals("jumped", &mut filter, 1)?;
    dtf_assert_term_equals("over", &mut filter, 1)?;
    dtf_assert_term_equals("the", &mut filter, 1)?;
    dtf_assert_term_equals("lazy", &mut filter, 2)?;
    dtf_assert_term_equals("brown", &mut filter, 123)?;
    dtf_assert_term_equals("dogs", &mut filter, 1024)?;
    Assert::assert_false(filter.increment_token()?);
    filter.end()?;
    filter.close()
}

fn dtf_test_invalid_negative_tf() -> Result<()> {
    let test = "foo bar|-20";
    let mut filter = new_delimited_term_frequency_token_filter(boxed(ws(test)?))?;
    filter.reset()?;
    dtf_assert_term_equals("foo", &mut filter, 1)?;
    let iae = iae_message(|| filter.increment_token());
    Assert::assert_equals("Term frequency must be 1 or greater; got -20", iae.as_str());
    Ok(())
}

fn dtf_test_invalid_float_tf() -> Result<()> {
    let test = "foo bar|1.2";
    let mut filter = new_delimited_term_frequency_token_filter(boxed(ws(test)?))?;
    filter.reset()?;
    dtf_assert_term_equals("foo", &mut filter, 1)?;
    expect_throws("java.lang.NumberFormatException", || {
        filter.increment_token()
    });
    Ok(())
}

fn delimited_term_frequency_token_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.miscellaneous.TestDelimitedTermFrequencyTokenFilter")
        .method(t("testTermFrequency", dtf_test_term_frequency))
        .method(t("testInvalidNegativeTf", dtf_test_invalid_negative_tf))
        .method(t("testInvalidFloatTf", dtf_test_invalid_float_tf))
        .build()
}

// -----------------------------------------------------------------------------
// TestDropIfFlaggedFilter
// -----------------------------------------------------------------------------

/// `new Token(text, start, end)` with `setFlags(flags)`.
fn flagged(text: &str, start: i32, end: i32, flags: i32) -> Token {
    let mut token = Token::with_offsets(text, start, end);
    token.set_flags(flags);
    token
}

/// Test the straight forward cases. When all flags match the token should be
/// dropped
fn dif_test_dropped() -> Result<()> {
    let ts = canned(vec![
        flagged("foo", 0, 2, 0),   // 000 no flags match
        flagged("bar", 4, 6, 1),   // 001 one flag matches
        flagged("baz", 8, 10, 2),  // 010 no flags match
        flagged("bam", 12, 14, 7), // 111 both flags match (drop)
    ]);
    let mut ts = new_drop_if_flagged_filter(ts, 5)?; // 101
    B::assert_token_stream_contents_offsets_pos_increments(
        &mut ts,
        &["foo", "bar", "baz"],
        &[0, 4, 8],
        &[2, 6, 10],
        &[1, 1, 1],
    )
}

/// Test where the first and last token are dropped.
fn dif_test_dropped_first() -> Result<()> {
    let ts = canned(vec![
        flagged("foo", 0, 2, 4),   // 100 flag matches (drop)
        flagged("bar", 4, 6, 1),   // 001 no flags match
        flagged("baz", 8, 10, 2),  // 010 no flags match
        flagged("bam", 12, 14, 7), // 111 flag matches (drop)
    ]);
    let mut ts = new_drop_if_flagged_filter(ts, 4)?;
    B::assert_token_stream_contents_offsets_pos_increments(
        &mut ts,
        &["bar", "baz"],
        &[4, 8],
        &[6, 10],
        &[2, 1],
    )
}

fn drop_if_flagged_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.miscellaneous.TestDropIfFlaggedFilter")
        .method(t("testDropped", dif_test_dropped))
        .method(t("testDroppedFirst", dif_test_dropped_first))
        .build()
}

// -----------------------------------------------------------------------------
// TestDropIfFlaggedFilterFactory
// -----------------------------------------------------------------------------

/// `token(String term, int flags, int soff, int eoff)`.
fn diff_token(text: &str, flags: i32, soff: i32, eoff: i32) -> Token {
    let mut token = Token::new();
    token.set_empty();
    token.append_string(Some(text));
    token.set_flags(flags);
    token.set_offset(soff, eoff);
    token
}

fn diff_test_factory() -> Result<()> {
    let stream = canned(vec![diff_token("foo", 1, 0, 2), diff_token("bar", 3, 4, 6)]);
    let token_filter_factory = fac("dropIfFlagged", &["flags", "2"])?;
    let mut stream = token_filter_factory.create(stream)?;
    B::assert_token_stream_contents_types(
        &mut *stream,
        &["foo"],
        None,
        None,
        Some(&["word"]),
        Some(&[1]),
    )
}

fn drop_if_flagged_filter_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.miscellaneous.TestDropIfFlaggedFilterFactory")
        .method(t("testFactory", diff_test_factory))
        .build()
}

// -----------------------------------------------------------------------------
// TestEmptyTokenStream
// -----------------------------------------------------------------------------

fn ets_test_consume() -> Result<()> {
    let mut ts = EmptyTokenStream::new();
    ts.reset()?;
    Assert::assert_false(ts.increment_token()?);
    ts.end()?;
    ts.close()?;
    // try again with reuse:
    ts.reset()?;
    Assert::assert_false(ts.increment_token()?);
    ts.end()?;
    ts.close()
}

fn ets_test_consume2() -> Result<()> {
    B::assert_token_stream_contents(&mut EmptyTokenStream::new(), &[])
}

fn ets_test_index_writer_lucene4656() -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(
        directory.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(None),
    )?;

    let ts = EmptyTokenStream::new();
    Assert::assert_false(
        ts.attribute_source()
            .has_attribute_by_id(std::any::TypeId::of::<dyn TermToBytesRefAttribute>()),
    );

    let mut doc = Document::new();
    doc.add(Box::new(StringField::new(
        "id",
        "0".to_string(),
        Store::YES,
    )?));
    let ts: Arc<AtomicRefCell<dyn TokenStream>> = Arc::new(AtomicRefCell::new(ts));
    doc.add(Box::new(TextField::new_with_token_stream(
        "description",
        ts,
    )?));

    // this should not fail because we have no TermToBytesRefAttribute
    writer.add_document(&doc)?;

    Assert::assert_equals(1, writer.get_doc_stats()?.num_docs);

    writer.close()?;
    directory.close()
}

fn empty_token_stream_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.miscellaneous.TestEmptyTokenStream")
        .method(t("testConsume", ets_test_consume))
        .method(t("testConsume2", ets_test_consume2))
        .method(t(
            "testIndexWriter_LUCENE4656",
            ets_test_index_writer_lucene4656,
        ))
        .build()
}

// -----------------------------------------------------------------------------
// TestFixBrokenOffsetsFilter
// -----------------------------------------------------------------------------

fn fbo_test_bogus_term_vectors() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iw = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(None),
    )?;
    let mut doc = Document::new();
    let ft = FieldType::new_from(TextField::type_not_stored());
    ft.set_store_term_vectors(true)?;
    ft.set_store_term_vector_offsets(true)?;
    let filter = new_fix_broken_offsets_filter(canned(vec![
        Token::with_offsets("bar", 5, 10),
        Token::with_offsets("bar", 1, 4),
    ]))?;
    let filter: Arc<AtomicRefCell<dyn TokenStream>> = Arc::new(AtomicRefCell::new(filter));
    let field = Field::new_with_token_stream("foo", filter, ft)?;
    doc.add(Box::new(field));
    iw.add_document(&doc)?;
    iw.close()?;
    dir.close()
}

fn fix_broken_offsets_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.miscellaneous.TestFixBrokenOffsetsFilter")
        .method(t("testBogusTermVectors", fbo_test_bogus_term_vectors))
        .build()
}

// -----------------------------------------------------------------------------
// TestHyphenatedWordsFilter
// -----------------------------------------------------------------------------

fn hw_test_hyphenated_words() -> Result<()> {
    let input =
        "ecologi-\r\ncal devel-\r\n\r\nop compre-\u{0009}hensive-hands-on and ecologi-\ncal";
    // first test
    let mut ts = MockTokenizer::new(Arc::clone(&WHITESPACE), false)?;
    ts.set_reader(Box::new(StringCharReader::new(input)))?;
    let mut ts = new_hyphenated_words_filter(boxed(ts))?;
    B::assert_token_stream_contents(
        &mut ts,
        &[
            "ecological",
            "develop",
            "comprehensive-hands-on",
            "and",
            "ecological",
        ],
    )
}

/// Test that HyphenatedWordsFilter behaves correctly with a final hyphen
fn hw_test_hyphen_at_end() -> Result<()> {
    let input = "ecologi-\r\ncal devel-\r\n\r\nop compre-\u{0009}hensive-hands-on and ecology-";
    // first test
    let mut ts = MockTokenizer::new(Arc::clone(&WHITESPACE), false)?;
    ts.set_reader(Box::new(StringCharReader::new(input)))?;
    let mut ts = new_hyphenated_words_filter(boxed(ts))?;
    B::assert_token_stream_contents(
        &mut ts,
        &[
            "ecological",
            "develop",
            "comprehensive-hands-on",
            "and",
            "ecology-",
        ],
    )
}

fn hw_test_offsets() -> Result<()> {
    let input = "abc- def geh 1234- 5678-";
    let mut ts = MockTokenizer::new(Arc::clone(&WHITESPACE), false)?;
    ts.set_reader(Box::new(StringCharReader::new(input)))?;
    let mut ts = new_hyphenated_words_filter(boxed(ts))?;
    B::assert_token_stream_contents_offsets(
        &mut ts,
        &["abcdef", "geh", "12345678-"],
        &[0, 9, 13],
        &[8, 12, 24],
    )
}

/// blast some random strings through the analyzer
fn hw_test_random_string() -> Result<()> {
    let a = whitespace_analyzer(
        "org.apache.lucene.analysis.miscellaneous.TestHyphenatedWordsFilter$1",
        |tokenizer| Ok(boxed(new_hyphenated_words_filter(tokenizer)?)),
    );
    B::check_random_data(&*random(), &a, 200 * *RANDOM_MULTIPLIER)?;
    a.close();
    Ok(())
}

fn hw_test_empty_term() -> Result<()> {
    let a = keyword_tokenizer_analyzer(
        "org.apache.lucene.analysis.miscellaneous.TestHyphenatedWordsFilter$2",
        |tokenizer| Ok(boxed(new_hyphenated_words_filter(tokenizer)?)),
    );
    B::check_one_term(&*a, "", "")?;
    a.close();
    Ok(())
}

fn hyphenated_words_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.miscellaneous.TestHyphenatedWordsFilter")
        .method(t("testHyphenatedWords", hw_test_hyphenated_words))
        .method(t("testHyphenAtEnd", hw_test_hyphen_at_end))
        .method(t("testOffsets", hw_test_offsets))
        .method(t("testRandomString", hw_test_random_string))
        .method(t("testEmptyTerm", hw_test_empty_term))
        .build()
}

// -----------------------------------------------------------------------------
// TestKeepFilterFactory
// -----------------------------------------------------------------------------

fn kff_test_inform() -> Result<()> {
    // `new ClasspathResourceLoader(getClass())`
    let _loader = support::test_class_loader(MISC);

    let factory = concrete_factory(
        MISC,
        KeepWordFilterFactory::new,
        &["words", "keep-1.txt", "ignoreCase", "true"],
    )?;
    let words = factory.base().get_words();
    Assert::assert_true_msg("words is null and it shouldn't be", words.is_some());
    let size = words.map_or(0, |w| w.size());
    Assert::assert_true_msg(&format!("words Size: {size} is not: 2"), size == 2);

    let factory = concrete_factory(
        MISC,
        KeepWordFilterFactory::new,
        &["words", "keep-1.txt, keep-2.txt", "ignoreCase", "true"],
    )?;
    let words = factory.base().get_words();
    Assert::assert_true_msg("words is null and it shouldn't be", words.is_some());
    let size = words.map_or(0, |w| w.size());
    Assert::assert_true_msg(&format!("words Size: {size} is not: 4"), size == 4);

    let factory = concrete_factory(
        MISC,
        KeepWordFilterFactory::new,
        &[
            "words",
            "keep-snowball.txt",
            "format",
            "snowball",
            "ignoreCase",
            "true",
        ],
    )?;
    let words = factory.base().get_words().expect("words");
    Assert::assert_equals(8, words.size());
    for w in [
        "he", "him", "his", "himself", "she", "her", "hers", "herself",
    ] {
        Assert::assert_true(words.contains(Some(w)));
    }

    // defaults
    let factory = concrete_factory(MISC, KeepWordFilterFactory::new, &[])?;
    Assert::assert_true(factory.base().get_words().is_none());
    Assert::assert_equals(false, factory.base().is_ignore_case());
    Ok(())
}

/// Test that bogus arguments result in exception
fn kff_test_bogus_arguments() -> Result<()> {
    assert_bogus("KeepWord", &["bogusArg", "bogusValue"])
}

fn keep_filter_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.miscellaneous.TestKeepFilterFactory")
        .method(t("testInform", kff_test_inform))
        .method(t("testBogusArguments", kff_test_bogus_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// TestKeepWordFilter
// -----------------------------------------------------------------------------

fn kwf_test_stop_and_go() -> Result<()> {
    let words = ["aaa", "bbb"];
    let input = "xxx yyy aaa zzz BBB ccc ddd EEE";

    // Test Stopwords
    let stream = ws(input)?;
    let mut stream = new_keep_word_filter(boxed(stream), CharArraySet::from_iter(words, true))?;
    B::assert_token_stream_contents_pos_increments(&mut stream, &["aaa", "BBB"], &[3, 2])?;

    // Now force case
    let stream = ws(input)?;
    let mut stream = new_keep_word_filter(boxed(stream), CharArraySet::from_iter(words, false))?;
    B::assert_token_stream_contents_pos_increments(&mut stream, &["aaa"], &[3])
}

/// blast some random strings through the analyzer
fn kwf_test_random_strings() -> Result<()> {
    let a = whitespace_analyzer(
        "org.apache.lucene.analysis.miscellaneous.TestKeepWordFilter$1",
        |tokenizer| {
            Ok(boxed(new_keep_word_filter(
                tokenizer,
                CharArraySet::from_iter(["a", "b"], true),
            )?))
        },
    );
    B::check_random_data(&*random(), &a, 200 * *RANDOM_MULTIPLIER)?;
    a.close();
    Ok(())
}

fn keep_word_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.miscellaneous.TestKeepWordFilter")
        .method(t("testStopAndGo", kwf_test_stop_and_go))
        .method(t("testRandomStrings", kwf_test_random_strings))
        .build()
}

// -----------------------------------------------------------------------------
// TestKeywordMarkerFilter
// -----------------------------------------------------------------------------

/// `LowerCaseFilterMock`: lower-cases every term not marked as a keyword.
#[derive(Debug)]
struct LowerCaseFilterMockLogic;

impl TokenFilterLogic for LowerCaseFilterMockLogic {
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if input.increment_token()? {
            let keyword = source
                .get_attribute_interface::<dyn KeywordAttribute>()
                .expect("keywordAttr")
                .is_keyword();
            if !keyword {
                let mut term_att = source
                    .get_attribute_interface_mut::<dyn CharTermAttribute>()
                    .expect("termAtt");
                // `termAtt.toString().toLowerCase(Locale.ROOT)`
                let term: String = term_att.buffer()[..term_att.length()]
                    .iter()
                    .collect::<String>()
                    .to_lowercase();
                term_att.set_empty();
                term_att.append_string(Some(&term));
            }
            return Ok(true);
        }
        Ok(false)
    }
}

/// `new LowerCaseFilterMock(in)`.
fn lower_case_filter_mock(
    input: Box<dyn TokenStream>,
) -> Result<TokenFilter<LowerCaseFilterMockLogic>> {
    let mut filter = TokenFilter::new(input, LowerCaseFilterMockLogic);
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn CharTermAttribute>()?;
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn KeywordAttribute>()?;
    Ok(filter)
}

/// `Pattern.compile(regex)`.
fn pattern(regex: &str) -> JavaPattern {
    JavaPattern::compile(regex, 0).expect("a valid pattern")
}

fn kmf_test_set_filter_increment_token() -> Result<()> {
    let mut set = CharArraySet::new(5, true);
    set.add("lucenefox");
    let output = ["the", "quick", "brown", "LuceneFox", "jumps"];
    B::assert_token_stream_contents(
        &mut lower_case_filter_mock(boxed(new_set_keyword_marker_filter(
            boxed(ws("The quIck browN LuceneFox Jumps")?),
            set.clone(),
        )?))?,
        &output,
    )?;
    let mixed_case_set = CharArraySet::from_iter(["LuceneFox"], false);
    B::assert_token_stream_contents(
        &mut lower_case_filter_mock(boxed(new_set_keyword_marker_filter(
            boxed(ws("The quIck browN LuceneFox Jumps")?),
            mixed_case_set,
        )?))?,
        &output,
    )?;
    let set2 = set;
    B::assert_token_stream_contents(
        &mut lower_case_filter_mock(boxed(new_set_keyword_marker_filter(
            boxed(ws("The quIck browN LuceneFox Jumps")?),
            set2,
        )?))?,
        &output,
    )
}

fn kmf_test_pattern_filter_increment_token() -> Result<()> {
    let output = ["the", "quick", "brown", "LuceneFox", "jumps"];
    B::assert_token_stream_contents(
        &mut lower_case_filter_mock(boxed(new_pattern_keyword_marker_filter(
            boxed(ws("The quIck browN LuceneFox Jumps")?),
            pattern("[a-zA-Z]+[fF]ox"),
        )?))?,
        &output,
    )?;

    let output = ["the", "quick", "brown", "lucenefox", "jumps"];
    B::assert_token_stream_contents(
        &mut lower_case_filter_mock(boxed(new_pattern_keyword_marker_filter(
            boxed(ws("The quIck browN LuceneFox Jumps")?),
            pattern("[a-zA-Z]+[f]ox"),
        )?))?,
        &output,
    )
}

// LUCENE-2901
fn kmf_test_composition() -> Result<()> {
    let mut ts = lower_case_filter_mock(boxed(new_set_keyword_marker_filter(
        boxed(new_set_keyword_marker_filter(
            boxed(ws("Dogs Trees Birds Houses")?),
            CharArraySet::from_iter(["Birds", "Houses"], false),
        )?),
        CharArraySet::from_iter(["Dogs", "Trees"], false),
    )?))?;
    B::assert_token_stream_contents(&mut ts, &["Dogs", "Trees", "Birds", "Houses"])?;

    let mut ts = lower_case_filter_mock(boxed(new_pattern_keyword_marker_filter(
        boxed(new_pattern_keyword_marker_filter(
            boxed(ws("Dogs Trees Birds Houses")?),
            pattern("Birds|Houses"),
        )?),
        pattern("Dogs|Trees"),
    )?))?;
    B::assert_token_stream_contents(&mut ts, &["Dogs", "Trees", "Birds", "Houses"])?;

    let mut ts = lower_case_filter_mock(boxed(new_set_keyword_marker_filter(
        boxed(new_pattern_keyword_marker_filter(
            boxed(ws("Dogs Trees Birds Houses")?),
            pattern("Birds|Houses"),
        )?),
        CharArraySet::from_iter(["Dogs", "Trees"], false),
    )?))?;
    B::assert_token_stream_contents(&mut ts, &["Dogs", "Trees", "Birds", "Houses"])
}

fn keyword_marker_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.miscellaneous.TestKeywordMarkerFilter")
        .method(t(
            "testSetFilterIncrementToken",
            kmf_test_set_filter_increment_token,
        ))
        .method(t(
            "testPatternFilterIncrementToken",
            kmf_test_pattern_filter_increment_token,
        ))
        .method(t("testComposition", kmf_test_composition))
        .build()
}

// -----------------------------------------------------------------------------
// TestKeywordMarkerFilterFactory
// -----------------------------------------------------------------------------

/// `tokenFilterFactory("KeywordMarker", Version.LATEST, new
/// StringMockResourceLoader("cats"), keysAndValues...)`.
fn kmff_with_cats(
    keys_and_values: &[&str],
) -> Result<Arc<dyn rucene::analysis::factories::TokenFilterFactory>> {
    rucene_test_framework::analysis::BaseTokenStreamFactoryTestCase::token_filter_factory_with_loader(
        "KeywordMarker",
        Some(Version::LATEST),
        &StringMockResourceLoader::new("cats"),
        keys_and_values,
    )
}

fn kmff_test_keywords() -> Result<()> {
    let stream = ws("dogs cats")?;
    let stream = kmff_with_cats(&["protected", "protwords.txt"])?.create(boxed(stream))?;
    let mut stream = fac("PorterStem", &[])?.create(stream)?;
    B::assert_token_stream_contents(&mut *stream, &["dog", "cats"])
}

fn kmff_test_keywords2() -> Result<()> {
    let stream = ws("dogs cats")?;
    let stream = fac("KeywordMarker", &["pattern", "cats|Dogs"])?.create(boxed(stream))?;
    let mut stream = fac("PorterStem", &[])?.create(stream)?;
    B::assert_token_stream_contents(&mut *stream, &["dog", "cats"])
}

fn kmff_test_keywords_mixed() -> Result<()> {
    let stream = ws("dogs cats birds")?;
    let stream = kmff_with_cats(&["protected", "protwords.txt", "pattern", "birds|Dogs"])?
        .create(boxed(stream))?;
    let mut stream = fac("PorterStem", &[])?.create(stream)?;
    B::assert_token_stream_contents(&mut *stream, &["dog", "cats", "birds"])
}

fn kmff_test_keywords_case_insensitive() -> Result<()> {
    let stream = ws("dogs cats Cats")?;
    let stream = kmff_with_cats(&["protected", "protwords.txt", "ignoreCase", "true"])?
        .create(boxed(stream))?;
    let mut stream = fac("PorterStem", &[])?.create(stream)?;
    B::assert_token_stream_contents(&mut *stream, &["dog", "cats", "Cats"])
}

fn kmff_test_keywords_case_insensitive2() -> Result<()> {
    let stream = ws("dogs cats Cats")?;
    let stream =
        fac("KeywordMarker", &["pattern", "Cats", "ignoreCase", "true"])?.create(boxed(stream))?;
    let mut stream = fac("PorterStem", &[])?.create(stream)?;
    B::assert_token_stream_contents(&mut *stream, &["dog", "cats", "Cats"])
}

fn kmff_test_keywords_case_insensitive_mixed() -> Result<()> {
    let stream = ws("dogs cats Cats Birds birds")?;
    let stream = kmff_with_cats(&[
        "protected",
        "protwords.txt",
        "pattern",
        "birds",
        "ignoreCase",
        "true",
    ])?
    .create(boxed(stream))?;
    let mut stream = fac("PorterStem", &[])?.create(stream)?;
    B::assert_token_stream_contents(&mut *stream, &["dog", "cats", "Cats", "Birds", "birds"])
}

/// Test that bogus arguments result in exception
fn kmff_test_bogus_arguments() -> Result<()> {
    assert_bogus("KeywordMarker", &["bogusArg", "bogusValue"])
}

fn keyword_marker_filter_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.miscellaneous.TestKeywordMarkerFilterFactory")
        .method(t("testKeywords", kmff_test_keywords))
        .method(t("testKeywords2", kmff_test_keywords2))
        .method(t("testKeywordsMixed", kmff_test_keywords_mixed))
        .method(t(
            "testKeywordsCaseInsensitive",
            kmff_test_keywords_case_insensitive,
        ))
        .method(t(
            "testKeywordsCaseInsensitive2",
            kmff_test_keywords_case_insensitive2,
        ))
        .method(t(
            "testKeywordsCaseInsensitiveMixed",
            kmff_test_keywords_case_insensitive_mixed,
        ))
        .method(t("testBogusArguments", kmff_test_bogus_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// TestLengthFilter
// -----------------------------------------------------------------------------

fn lf_test_filter_with_pos_incr() -> Result<()> {
    let stream = ws("short toolong evenmuchlongertext a ab toolong foo")?;
    let mut filter = new_length_filter(boxed(stream), 2, 6)?;
    B::assert_token_stream_contents_pos_increments(&mut filter, &["short", "ab", "foo"], &[1, 4, 2])
}

fn lf_test_empty_term() -> Result<()> {
    let a = keyword_tokenizer_analyzer(
        "org.apache.lucene.analysis.miscellaneous.TestLengthFilter$1",
        |tokenizer| Ok(boxed(new_length_filter(tokenizer, 0, 5)?)),
    );
    B::check_one_term(&*a, "", "")?;
    a.close();
    Ok(())
}

/// checking the validity of constructor arguments
fn lf_test_illegal_arguments() -> Result<()> {
    new_length_filter(boxed(ws("accept only valid arguments")?), -4, -1)?;
    Ok(())
}

fn length_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.miscellaneous.TestLengthFilter")
        .method(t("testFilterWithPosIncr", lf_test_filter_with_pos_incr))
        .method(t("testEmptyTerm", lf_test_empty_term))
        .method(t_iae("testIllegalArguments", lf_test_illegal_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// TestLengthFilterFactory
// -----------------------------------------------------------------------------

fn lff_test_position_increments() -> Result<()> {
    let mut stream = MockTokenizer::new(Arc::clone(&WHITESPACE), false)?;
    stream.set_reader(Box::new(StringCharReader::new(
        "foo foobar super-duper-trooper",
    )))?;
    let mut stream = fac(
        "Length",
        &[
            LengthFilterFactory::MIN_KEY,
            "4",
            LengthFilterFactory::MAX_KEY,
            "10",
        ],
    )?
    .create(boxed(stream))?;
    B::assert_token_stream_contents_pos_increments(&mut *stream, &["foobar"], &[2])
}

/// Test that bogus arguments result in exception
fn lff_test_bogus_arguments() -> Result<()> {
    assert_bogus(
        "Length",
        &[
            LengthFilterFactory::MIN_KEY,
            "4",
            LengthFilterFactory::MAX_KEY,
            "5",
            "bogusArg",
            "bogusValue",
        ],
    )
}

/// Test that invalid arguments result in exception
fn lff_test_invalid_arguments() -> Result<()> {
    let expected = iae_message(|| {
        let mut stream = MockTokenizer::new(Arc::clone(&WHITESPACE), false)?;
        stream.set_reader(Box::new(StringCharReader::new(
            "foo foobar super-duper-trooper",
        )))?;
        fac(
            "Length",
            &[
                LengthFilterFactory::MIN_KEY,
                "5",
                LengthFilterFactory::MAX_KEY,
                "4",
            ],
        )?
        .create(boxed(stream))
    });
    Assert::assert_true(
        expected.contains("maximum length must not be greater than minimum length"),
    );
    Ok(())
}

fn length_filter_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.miscellaneous.TestLengthFilterFactory")
        .method(t("testPositionIncrements", lff_test_position_increments))
        .method(t("testBogusArguments", lff_test_bogus_arguments))
        .method(t("testInvalidArguments", lff_test_invalid_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// TestLimitTokenCountAnalyzer
// -----------------------------------------------------------------------------

fn ltca_test_limit_token_count_analyzer() -> Result<()> {
    for consume_all in [true, false] {
        let mock = MockAnalyzer::new(&*random());
        // if we are consuming all tokens, we can use the checks,
        // otherwise we can't
        mock.set_enable_checks(consume_all);
        let a = LimitTokenCountAnalyzer::new_consuming(Arc::new(mock), 2, consume_all);

        // dont use assertAnalyzesTo here, as the end offset is not the end of the string (unless
        // consumeAll is true, in which case it's correct)!
        for (text, out, starts, ends, final_offset) in [
            (
                "1  2     3  4  5",
                &["1", "2"][..],
                &[0, 3][..],
                &[1, 4][..],
                16,
            ),
            ("1 2 3 4 5", &["1", "2"][..], &[0, 2][..], &[1, 3][..], 9),
            // less than the limit, ensure we behave correctly
            ("1  ", &["1"][..], &[0][..], &[1][..], 3),
            // equal to limit
            ("1  2  ", &["1", "2"][..], &[0, 3][..], &[1, 4][..], 6),
        ] {
            let ts = a.token_stream_from_str("dummy", text)?;
            let mut ts = ts.borrow_mut();
            B::assert_token_stream_contents_offsets_final_offset(
                &mut *ts,
                out,
                starts,
                ends,
                if consume_all {
                    Some(final_offset)
                } else {
                    None
                },
            )?;
        }

        a.close();
    }
    Ok(())
}

fn ltca_test_limit_token_count_index_writer() -> Result<()> {
    for consume_all in [true, false] {
        let dir = LuceneTestCase::new_directory();
        let limit = TestUtil::next_int(&*random(), 50, 101000);
        let mock = MockAnalyzer::new(&*random());

        // if we are consuming all tokens, we can use the checks,
        // otherwise we can't
        mock.set_enable_checks(consume_all);
        let a: Arc<dyn Analyzer> = Arc::new(LimitTokenCountAnalyzer::new_consuming(
            Arc::new(mock),
            limit,
            consume_all,
        ));

        let writer = IndexWriter::new(
            dir.as_directory(),
            IndexWriterConfig::with_analyzer(Some(Arc::clone(&a))),
        )?;

        let mut doc = Document::new();
        let mut b = String::new();
        for _ in 1..limit {
            b.push_str(" a");
        }
        b.push_str(" x");
        b.push_str(" z");
        doc.add(Box::new(LuceneTestCase::new_text_field(
            "field",
            &b,
            Store::NO,
        )));
        writer.add_document(&doc)?;
        writer.close()?;

        let reader = directory_reader::open(dir.as_directory())?;
        let t = Term::from_text("field", "x");
        Assert::assert_equals(1, reader.doc_freq(&t)?);
        let t = Term::from_text("field", "z");
        Assert::assert_equals(0, reader.doc_freq(&t)?);
        reader.close()?;
        dir.close()?;
        a.close();
    }
    Ok(())
}

fn limit_token_count_analyzer_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.miscellaneous.TestLimitTokenCountAnalyzer")
        .method(t(
            "testLimitTokenCountAnalyzer",
            ltca_test_limit_token_count_analyzer,
        ))
        .method(t(
            "testLimitTokenCountIndexWriter",
            ltca_test_limit_token_count_index_writer,
        ))
        .build()
}

// -----------------------------------------------------------------------------
// TestLimitTokenCountFilter
// -----------------------------------------------------------------------------

fn ltcf_test() -> Result<()> {
    for consume_all in [true, false] {
        let mut tokenizer = ws("A1 B2 C3 D4 E5 F6")?;
        tokenizer.set_enable_checks(consume_all);
        let mut stream = new_limit_token_count_filter_consuming(boxed(tokenizer), 3, consume_all)?;
        B::assert_token_stream_contents(&mut stream, &["A1", "B2", "C3"])?;
    }
    Ok(())
}

fn ltcf_test_illegal_arguments() -> Result<()> {
    new_limit_token_count_filter(boxed(ws("A1 B2 C3 D4 E5 F6")?), -1)?;
    Ok(())
}

fn limit_token_count_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.miscellaneous.TestLimitTokenCountFilter")
        .method(t("test", ltcf_test))
        .method(t_iae("testIllegalArguments", ltcf_test_illegal_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// TestLimitTokenCountFilterFactory
// -----------------------------------------------------------------------------

fn ltcff_test() -> Result<()> {
    for consume_all in [true, false] {
        let mut tokenizer = MockTokenizer::new(Arc::clone(&WHITESPACE), false)?;
        tokenizer.set_reader(Box::new(StringCharReader::new("A1 B2 C3 D4 E5 F6")))?;
        tokenizer.set_enable_checks(consume_all);
        let consume = consume_all.to_string();
        let mut stream = fac(
            "LimitTokenCount",
            &[
                LimitTokenCountFilterFactory::MAX_TOKEN_COUNT_KEY,
                "3",
                LimitTokenCountFilterFactory::CONSUME_ALL_TOKENS_KEY,
                &consume,
            ],
        )?
        .create(boxed(tokenizer))?;
        B::assert_token_stream_contents(&mut *stream, &["A1", "B2", "C3"])?;
    }
    Ok(())
}

fn ltcff_test_required() -> Result<()> {
    // param is required
    let expected = iae_message(|| fac("LimitTokenCount", &[]));
    Assert::assert_true_msg(
        &format!("exception doesn't mention param: {expected}"),
        expected
            .find(LimitTokenCountFilterFactory::MAX_TOKEN_COUNT_KEY)
            .is_some_and(|i| 0 < i),
    );
    Ok(())
}

/// Test that bogus arguments result in exception
fn ltcff_test_bogus_arguments() -> Result<()> {
    assert_bogus(
        "LimitTokenCount",
        &[
            LimitTokenCountFilterFactory::MAX_TOKEN_COUNT_KEY,
            "3",
            "bogusArg",
            "bogusValue",
        ],
    )
}

fn limit_token_count_filter_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.miscellaneous.TestLimitTokenCountFilterFactory")
        .method(t("test", ltcff_test))
        .method(t("testRequired", ltcff_test_required))
        .method(t("testBogusArguments", ltcff_test_bogus_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// TestLimitTokenOffsetFilter
// -----------------------------------------------------------------------------

fn ltof_test() -> Result<()> {
    for consume_all in [true, false] {
        let mut tokenizer = ws("A1 B2 C3 D4 E5 F6")?;
        tokenizer.set_enable_checks(consume_all);
        // note with '3', this test would fail if erroneously the filter used endOffset instead
        let mut stream = new_limit_token_offset_filter_consuming(boxed(tokenizer), 3, consume_all)?;
        B::assert_token_stream_contents(&mut stream, &["A1", "B2"])?;
    }
    Ok(())
}

fn ltof_test_illegal_arguments() -> Result<()> {
    new_limit_token_offset_filter(boxed(ws("A1 B2 C3 D4 E5 F6")?), -1)?;
    Ok(())
}

fn limit_token_offset_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.miscellaneous.TestLimitTokenOffsetFilter")
        .method(t("test", ltof_test))
        .method(t_iae("testIllegalArguments", ltof_test_illegal_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// TestLimitTokenOffsetFilterFactory
// -----------------------------------------------------------------------------

fn ltoff_test() -> Result<()> {
    for consume_all in [true, false] {
        let mut tokenizer = MockTokenizer::new(Arc::clone(&WHITESPACE), false)?;
        tokenizer.set_reader(Box::new(StringCharReader::new("A1 B2 C3 D4 E5 F6")))?;
        tokenizer.set_enable_checks(consume_all);
        let consume = consume_all.to_string();
        let mut stream = fac(
            "LimitTokenOffset",
            &[
                LimitTokenOffsetFilterFactory::MAX_START_OFFSET,
                "3",
                LimitTokenOffsetFilterFactory::CONSUME_ALL_TOKENS_KEY,
                &consume,
            ],
        )?
        .create(boxed(tokenizer))?;
        B::assert_token_stream_contents(&mut *stream, &["A1", "B2"])?;
    }
    Ok(())
}

fn ltoff_test_required() -> Result<()> {
    // param is required
    let expected = iae_message(|| fac("LimitTokenOffset", &[]));
    Assert::assert_true_msg(
        &format!("exception doesn't mention param: {expected}"),
        expected
            .find(LimitTokenOffsetFilterFactory::MAX_START_OFFSET)
            .is_some_and(|i| 0 < i),
    );
    Ok(())
}

/// Test that bogus arguments result in exception
fn ltoff_test_bogus_arguments() -> Result<()> {
    assert_bogus(
        "LimitTokenOffset",
        &[
            LimitTokenOffsetFilterFactory::MAX_START_OFFSET,
            "3",
            "bogusArg",
            "bogusValue",
        ],
    )
}

fn limit_token_offset_filter_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.miscellaneous.TestLimitTokenOffsetFilterFactory")
        .method(t("test", ltoff_test))
        .method(t("testRequired", ltoff_test_required))
        .method(t("testBogusArguments", ltoff_test_bogus_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// TestLimitTokenPositionFilterFactory
// -----------------------------------------------------------------------------

fn ltpff_test_max_position1() -> Result<()> {
    for consume_all in [true, false] {
        let mut tokenizer = ws("A1 B2 C3 D4 E5 F6")?;
        // if we are consuming all tokens, we can use the checks, otherwise we can't
        tokenizer.set_enable_checks(consume_all);
        let consume = consume_all.to_string();
        let mut stream = fac(
            "LimitTokenPosition",
            &[
                LimitTokenPositionFilterFactory::MAX_TOKEN_POSITION_KEY,
                "1",
                LimitTokenPositionFilterFactory::CONSUME_ALL_TOKENS_KEY,
                &consume,
            ],
        )?
        .create(boxed(tokenizer))?;
        B::assert_token_stream_contents(&mut *stream, &["A1"])?;
    }
    Ok(())
}

fn ltpff_test_missing_param() -> Result<()> {
    let expected = iae_message(|| fac("LimitTokenPosition", &[]));
    Assert::assert_true_msg(
        &format!("exception doesn't mention param: {expected}"),
        expected
            .find(LimitTokenPositionFilterFactory::MAX_TOKEN_POSITION_KEY)
            .is_some_and(|i| 0 < i),
    );
    Ok(())
}

fn ltpff_test_max_position1_with_shingles() -> Result<()> {
    for consume_all in [true, false] {
        let mut tokenizer = ws("one two three four five")?;
        // if we are consuming all tokens, we can use the checks, otherwise we can't
        tokenizer.set_enable_checks(consume_all);
        let stream = fac(
            "Shingle",
            &[
                "minShingleSize",
                "2",
                "maxShingleSize",
                "3",
                "outputUnigrams",
                "true",
            ],
        )?
        .create(boxed(tokenizer))?;
        let consume = consume_all.to_string();
        let mut stream = fac(
            "LimitTokenPosition",
            &[
                LimitTokenPositionFilterFactory::MAX_TOKEN_POSITION_KEY,
                "1",
                LimitTokenPositionFilterFactory::CONSUME_ALL_TOKENS_KEY,
                &consume,
            ],
        )?
        .create(stream)?;
        B::assert_token_stream_contents(&mut *stream, &["one", "one two", "one two three"])?;
    }
    Ok(())
}

/// Test that bogus arguments result in exception
fn ltpff_test_bogus_arguments() -> Result<()> {
    assert_bogus(
        "LimitTokenPosition",
        &["maxTokenPosition", "3", "bogusArg", "bogusValue"],
    )
}

fn limit_token_position_filter_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.miscellaneous.TestLimitTokenPositionFilterFactory")
        .method(t("testMaxPosition1", ltpff_test_max_position1))
        .method(t("testMissingParam", ltpff_test_missing_param))
        .method(t(
            "testMaxPosition1WithShingles",
            ltpff_test_max_position1_with_shingles,
        ))
        .method(t("testBogusArguments", ltpff_test_bogus_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// TestPerFieldAnalyzerWrapper
// -----------------------------------------------------------------------------

/// `Collections.singletonMap(field, analyzer)` as the wrapper's field map.
fn singleton_map(
    field: &str,
    analyzer: Arc<dyn Analyzer>,
) -> rucene::util::java_collections::JavaHashMap<Arc<dyn Analyzer>> {
    let mut map = rucene::util::java_collections::JavaHashMap::new();
    map.insert(field.to_string(), analyzer);
    map
}

fn pfaw_test_per_field() -> Result<()> {
    let text = "Qwerty";

    let special: Arc<dyn Analyzer> = Arc::new(SimpleAnalyzer::new());
    let default_analyzer: Arc<dyn Analyzer> = Arc::new(WhitespaceAnalyzer::new());

    let analyzer = PerFieldAnalyzerWrapper::new_with_fields(
        Arc::clone(&default_analyzer),
        Some(singleton_map("special", Arc::clone(&special))),
    );

    {
        let token_stream = analyzer.token_stream_from_str("field", text)?;
        let mut token_stream = token_stream.borrow_mut();
        token_stream.reset()?;

        Assert::assert_true(token_stream.increment_token()?);
        Assert::assert_equals_msg(
            Some("WhitespaceAnalyzer does not lowercase"),
            "Qwerty",
            term(&*token_stream).as_str(),
        );
        Assert::assert_false(token_stream.increment_token()?);
        token_stream.end()?;
        token_stream.close()?;
    }

    {
        let token_stream = analyzer.token_stream_from_str("special", text)?;
        let mut token_stream = token_stream.borrow_mut();
        token_stream.reset()?;

        Assert::assert_true(token_stream.increment_token()?);
        Assert::assert_equals_msg(
            Some("SimpleAnalyzer lowercases"),
            "qwerty",
            term(&*token_stream).as_str(),
        );
        Assert::assert_false(token_stream.increment_token()?);
        token_stream.end()?;
        token_stream.close()?;
    }
    // TODO: fix this about PFAW, this is crazy
    analyzer.close();
    default_analyzer.close();
    special.close();
    Ok(())
}

/// The reuse strategy object `wrapper1.getReuseStrategy()` returns, shared
/// with `wrapper2`: every call is forwarded to the strategy `wrapper1`
/// holds, with the calling analyzer's state (`ReuseStrategy` objects are
/// owned by their analyzer in the port).
#[derive(Debug)]
struct SharedStrategy(Arc<PerFieldAnalyzerWrapper>);

impl ReuseStrategy for SharedStrategy {
    fn get_reusable_components(
        &self,
        state: &AnalyzerState,
        field_name: &str,
    ) -> Option<Arc<AtomicRefCell<TokenStreamComponents>>> {
        self.0
            .analyzer_state()
            .reuse_strategy()
            .get_reusable_components(state, field_name)
    }

    fn set_reusable_components(
        &self,
        state: &AnalyzerState,
        field_name: &str,
        components: Arc<AtomicRefCell<TokenStreamComponents>>,
    ) {
        self.0
            .analyzer_state()
            .reuse_strategy()
            .set_reusable_components(state, field_name, components)
    }
}

/// The anonymous `AnalyzerWrapper` of `testReuseWrapped`, which does *not*
/// extend `DelegatingAnalyzerWrapper`.
#[derive(Debug)]
struct Wrapper2Logic {
    wrapper1: Arc<PerFieldAnalyzerWrapper>,
    special_analyzer: Arc<dyn Analyzer>,
    text: &'static str,
}

impl AnalyzerWrapperLogic for Wrapper2Logic {
    fn get_wrapped_analyzer(&self, _field_name: &str) -> &dyn Analyzer {
        &*self.wrapper1
    }

    fn wrap_components(
        &self,
        _field_name: &str,
        components: TokenStreamComponents,
    ) -> TokenStreamComponents {
        let special = self
            .special_analyzer
            .token_stream_from_str("special", self.text)
            .expect("specialAnalyzer.tokenStream");
        Assert::assert_false_msg(
            "expected not same",
            Arc::ptr_eq(&special, &components.token_stream()),
        );
        let filter =
            new_ascii_folding_filter(Box::new(SharedTokenStream::new(components.token_stream())))
                .expect("new ASCIIFoldingFilter");
        components.with_sink(sink(filter))
    }
}

/// `ts instanceof ASCIIFoldingFilter`: the stream's `Debug` names the
/// filter's body (the port's `TokenFilter` offers no `Any`).
fn is_ascii_folding_filter(ts: &Arc<AtomicRefCell<dyn TokenStream>>) -> bool {
    format!("{:?}", ts.borrow()).contains("ASCIIFoldingFilterLogic")
}

fn pfaw_test_reuse_wrapped() -> Result<()> {
    let text = "Qwerty";

    let special_analyzer: Arc<dyn Analyzer> = Arc::new(SimpleAnalyzer::new());
    let default_analyzer: Arc<dyn Analyzer> = Arc::new(WhitespaceAnalyzer::new());

    let wrapper1 = Arc::new(PerFieldAnalyzerWrapper::new_with_fields(
        Arc::clone(&default_analyzer),
        Some(singleton_map("special", Arc::clone(&special_analyzer))),
    ));

    // test that the PerFieldWrapper returns the same instance as original Analyzer:
    let ts1 = default_analyzer.token_stream_from_str("something", text)?;
    let ts2 = wrapper1.token_stream_from_str("something", text)?;
    Assert::assert_true_msg("assertSame(ts1, ts2)", Arc::ptr_eq(&ts1, &ts2));

    let ts1 = special_analyzer.token_stream_from_str("special", text)?;
    let ts2 = wrapper1.token_stream_from_str("special", text)?;
    Assert::assert_true_msg("assertSame(ts1, ts2)", Arc::ptr_eq(&ts1, &ts2));

    // Wrap with another wrapper, which does *not* extend DelegatingAnalyzerWrapper:
    let wrapper2 = AnalyzerWrapper::new(
        Box::new(Wrapper2Logic {
            wrapper1: Arc::clone(&wrapper1),
            special_analyzer: Arc::clone(&special_analyzer),
            text,
        }),
        Box::new(SharedStrategy(Arc::clone(&wrapper1))),
    );
    let ts3 = wrapper2.token_stream_from_str("special", text)?;
    Assert::assert_false_msg("assertNotSame(ts1, ts3)", Arc::ptr_eq(&ts1, &ts3));
    Assert::assert_true(is_ascii_folding_filter(&ts3));
    // check that cache did not get corrumpted:
    let ts2 = wrapper1.token_stream_from_str("special", text)?;
    Assert::assert_true_msg("assertSame(ts1, ts2)", Arc::ptr_eq(&ts1, &ts2));

    // Wrap PerField with another PerField. In that case all TokenStreams returned must be the same:
    let wrapper3 = PerFieldAnalyzerWrapper::new_with_fields(
        Arc::clone(&wrapper1) as Arc<dyn Analyzer>,
        Some(singleton_map("moreSpecial", Arc::clone(&special_analyzer))),
    );
    let ts1 = special_analyzer.token_stream_from_str("special", text)?;
    let ts2 = wrapper3.token_stream_from_str("special", text)?;
    Assert::assert_true_msg("assertSame(ts1, ts2)", Arc::ptr_eq(&ts1, &ts2));
    let ts3 = special_analyzer.token_stream_from_str("moreSpecial", text)?;
    let ts4 = wrapper3.token_stream_from_str("moreSpecial", text)?;
    Assert::assert_true_msg("assertSame(ts3, ts4)", Arc::ptr_eq(&ts3, &ts4));
    Assert::assert_true_msg("assertSame(ts2, ts3)", Arc::ptr_eq(&ts2, &ts3));

    // `IOUtils.close(wrapper3, wrapper2, wrapper1, specialAnalyzer, defaultAnalyzer)`
    wrapper3.close();
    wrapper2.close();
    wrapper1.close();
    special_analyzer.close();
    default_analyzer.close();
    Ok(())
}

fn pfaw_test_char_filters() -> Result<()> {
    let a = FnAnalyzer::new(
        "org.apache.lucene.analysis.miscellaneous.TestPerFieldAnalyzerWrapper$2",
        |_| {
            Ok(tokenizer_components(Arc::new(AtomicRefCell::new(
                MockTokenizer::whitespace_lower_case()?,
            ))))
        },
    )
    .init_reader(|_, reader: Box<dyn CharReader>| {
        Box::new(MockCharFilter::new(reader, 7).expect("new MockCharFilter")) as Box<dyn CharReader>
    })
    .arc();
    B::assert_analyzes_to_offsets(&*a, "ab", &["aab"], &[0], &[2])?;

    // now wrap in PFAW
    let p = PerFieldAnalyzerWrapper::new_with_fields(
        Arc::clone(&a),
        Some(rucene::util::java_collections::JavaHashMap::new()),
    );

    B::assert_analyzes_to_offsets(&p, "ab", &["aab"], &[0], &[2])?;
    p.close();
    a.close(); // TODO: fix this about PFAW, its a trap
    Ok(())
}

fn per_field_analyzer_wrapper_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.miscellaneous.TestPerFieldAnalyzerWrapper")
        .method(t("testPerField", pfaw_test_per_field))
        .method(t("testReuseWrapped", pfaw_test_reuse_wrapped))
        .method(t("testCharFilters", pfaw_test_char_filters))
        .build()
}

// -----------------------------------------------------------------------------
// TestRemoveDuplicatesTokenFilterFactory
// -----------------------------------------------------------------------------

/// `tok(int pos, String t, int start, int end)`.
fn rd_tok(pos: i32, text: &str, start: i32, end: i32) -> Token {
    let mut tok = Token::with_offsets(text, start, end);
    tok.set_position_increment(pos);
    tok
}

/// `testDups(String expected, Token... tokens)`: a helper with parameters,
/// which the runner (`LuceneJUnit3MethodProvider`: public, no-arg `test*`
/// methods only) does not run as a test.
fn rd_test_dups(expected: &str, tokens: Vec<Token>) -> Result<()> {
    let stream = canned(tokens);
    let mut stream = fac("RemoveDuplicates", &[])?.create(stream)?;
    // `expected.split("\\s")`
    let expected: Vec<&str> = expected
        .split([' ', '\t', '\n', '\u{0B}', '\u{0C}', '\r'])
        .collect();
    B::assert_token_stream_contents(&mut *stream, &expected)
}

fn rd_test_simple_dups() -> Result<()> {
    rd_test_dups(
        "A B C D E",
        vec![
            rd_tok(1, "A", 0, 4),
            rd_tok(1, "B", 5, 10),
            rd_tok(0, "B", 11, 15),
            rd_tok(1, "C", 16, 20),
            rd_tok(0, "D", 16, 20),
            rd_tok(1, "E", 21, 25),
        ],
    )
}

/// Test that bogus arguments result in exception
fn rd_test_bogus_arguments() -> Result<()> {
    assert_bogus("RemoveDuplicates", &["bogusArg", "bogusValue"])
}

fn remove_duplicates_token_filter_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.miscellaneous.TestRemoveDuplicatesTokenFilterFactory")
        .method(t("testSimpleDups", rd_test_simple_dups))
        .method(t("testBogusArguments", rd_test_bogus_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// TestScandinavianFoldingFilter
// -----------------------------------------------------------------------------

thread_local! {
    /// The instance field `analyzer` `setUp` sets.
    static SFF: RefCell<Option<Arc<dyn Analyzer>>> = const { RefCell::new(None) };
}

fn sff_set_up() -> Result<()> {
    LuceneTestCase::super_set_up();
    let analyzer = whitespace_analyzer(
        "org.apache.lucene.analysis.miscellaneous.TestScandinavianFoldingFilter$1",
        |tokenizer| Ok(boxed(new_scandinavian_folding_filter(tokenizer)?)),
    );
    SFF.with(|a| *a.borrow_mut() = Some(analyzer));
    Ok(())
}

fn sff_tear_down() -> Result<()> {
    if let Some(analyzer) = SFF.with(|a| a.borrow_mut().take()) {
        analyzer.close();
    }
    LuceneTestCase::super_tear_down();
    Ok(())
}

fn sff_analyzer() -> Arc<dyn Analyzer> {
    SFF.with(|a| Arc::clone(a.borrow().as_ref().expect("setUp")))
}

fn sff_test() -> Result<()> {
    let analyzer = sff_analyzer();
    for (input, expected) in [
        ("aeäaeeea", "aaaeea"), // should not cause ArrayOutOfBoundsException
        ("aeäaeeeae", "aaaeea"),
        ("aeaeeeae", "aaeea"),
        ("bøen", "boen"),
        ("åene", "aene"),
        ("blåbærsyltetøj", "blabarsyltetoj"),
        ("blaabaarsyltetoej", "blabarsyltetoj"),
        ("blåbärsyltetöj", "blabarsyltetoj"),
        ("raksmorgas", "raksmorgas"),
        ("räksmörgås", "raksmorgas"),
        ("ræksmørgås", "raksmorgas"),
        ("raeksmoergaas", "raksmorgas"),
        ("ræksmörgaos", "raksmorgas"),
        ("ab", "ab"),
        ("ob", "ob"),
        ("Ab", "Ab"),
        ("Ob", "Ob"),
        ("å", "a"),
        ("aa", "a"),
        ("aA", "a"),
        ("ao", "a"),
        ("aO", "a"),
        ("AA", "A"),
        ("Aa", "A"),
        ("Ao", "A"),
        ("AO", "A"),
        ("æ", "a"),
        ("ä", "a"),
        ("Æ", "A"),
        ("Ä", "A"),
        ("ae", "a"),
        ("aE", "a"),
        ("Ae", "A"),
        ("AE", "A"),
        ("ö", "o"),
        ("ø", "o"),
        ("Ö", "O"),
        ("Ø", "O"),
        ("oo", "o"),
        ("oe", "o"),
        ("oO", "o"),
        ("oE", "o"),
        ("Oo", "O"),
        ("Oe", "O"),
        ("OO", "O"),
        ("OE", "O"),
    ] {
        B::check_one_term(&*analyzer, input, expected)?;
    }
    Ok(())
}

/// check that the empty string doesn't cause issues
fn sff_test_empty_term() -> Result<()> {
    let a = keyword_tokenizer_analyzer(
        "org.apache.lucene.analysis.miscellaneous.TestScandinavianFoldingFilter$2",
        |tokenizer| Ok(boxed(new_scandinavian_folding_filter(tokenizer)?)),
    );
    B::check_one_term(&*a, "", "")?;
    a.close();
    Ok(())
}

/// blast some random strings through the analyzer
fn sff_test_random_data() -> Result<()> {
    B::check_random_data(&*random(), &sff_analyzer(), 200 * *RANDOM_MULTIPLIER)
}

fn scandinavian_folding_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.miscellaneous.TestScandinavianFoldingFilter")
        .method(hook("setUp", sff_set_up))
        .method(hook("tearDown", sff_tear_down))
        .method(t("test", sff_test))
        .method(t("testEmptyTerm", sff_test_empty_term))
        .method(t("testRandomData", sff_test_random_data))
        .build()
}

// -----------------------------------------------------------------------------
// TestScandinavianFoldingFilterFactory
// -----------------------------------------------------------------------------

fn sfff_test_stemming() -> Result<()> {
    let stream = ws("räksmörgås")?;
    let mut stream = fac("ScandinavianFolding", &[])?.create(boxed(stream))?;
    B::assert_token_stream_contents(&mut *stream, &["raksmorgas"])
}

/// Test that bogus arguments result in exception
fn sfff_test_bogus_arguments() -> Result<()> {
    assert_bogus("ScandinavianFolding", &["bogusArg", "bogusValue"])
}

fn scandinavian_folding_filter_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.miscellaneous.TestScandinavianFoldingFilterFactory")
        .method(t("testStemming", sfff_test_stemming))
        .method(t("testBogusArguments", sfff_test_bogus_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// TestScandinavianNormalizationFilter
// -----------------------------------------------------------------------------

/// `createAnalyzer()`.
fn snf_create_analyzer() -> Arc<dyn Analyzer> {
    whitespace_analyzer(
        "org.apache.lucene.analysis.miscellaneous.TestScandinavianNormalizationFilter$2",
        |tokenizer| Ok(boxed(new_scandinavian_normalization_filter(tokenizer)?)),
    )
}

fn snf_test_default() -> Result<()> {
    let analyzer = snf_create_analyzer();
    for (input, expected) in [
        ("aeäaeeea", "æææeea"), // should not cause ArrayIndexOutOfBoundsException
        ("aeäaeeeae", "æææeeæ"),
        ("aeaeeeae", "ææeeæ"),
        ("bøen", "bøen"),
        ("bOEen", "bØen"),
        ("åene", "åene"),
        ("blåbærsyltetøj", "blåbærsyltetøj"),
        ("blaabaersyltetöj", "blåbærsyltetøj"),
        ("räksmörgås", "ræksmørgås"),
        ("raeksmörgaos", "ræksmørgås"),
        ("raeksmörgaas", "ræksmørgås"),
        ("raeksmoergås", "ræksmørgås"),
        ("ab", "ab"),
        ("ob", "ob"),
        ("Ab", "Ab"),
        ("Ob", "Ob"),
        ("å", "å"),
        ("aa", "å"),
        ("aA", "å"),
        ("ao", "å"),
        ("aO", "å"),
        ("AA", "Å"),
        ("Aa", "Å"),
        ("Ao", "Å"),
        ("AO", "Å"),
        ("æ", "æ"),
        ("ä", "æ"),
        ("Æ", "Æ"),
        ("Ä", "Æ"),
        ("ae", "æ"),
        ("aE", "æ"),
        ("Ae", "Æ"),
        ("AE", "Æ"),
        ("ö", "ø"),
        ("ø", "ø"),
        ("Ö", "Ø"),
        ("Ø", "Ø"),
        ("oo", "ø"),
        ("oe", "ø"),
        ("oO", "ø"),
        ("oE", "ø"),
        ("Oo", "Ø"),
        ("Oe", "Ø"),
        ("OO", "Ø"),
        ("OE", "Ø"),
    ] {
        B::check_one_term(&*analyzer, input, expected)?;
    }
    analyzer.close();
    Ok(())
}

/// check that the empty string doesn't cause issues
fn snf_test_empty_term() -> Result<()> {
    let a = keyword_tokenizer_analyzer(
        "org.apache.lucene.analysis.miscellaneous.TestScandinavianNormalizationFilter$1",
        |tokenizer| Ok(boxed(new_scandinavian_normalization_filter(tokenizer)?)),
    );
    B::check_one_term(&*a, "", "")?;
    a.close();
    Ok(())
}

/// blast some random strings through the analyzer
fn snf_test_random_data() -> Result<()> {
    let analyzer = snf_create_analyzer();
    B::check_random_data(&*random(), &analyzer, 200 * *RANDOM_MULTIPLIER)?;
    analyzer.close();
    Ok(())
}

fn scandinavian_normalization_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.miscellaneous.TestScandinavianNormalizationFilter")
        .method(t("testDefault", snf_test_default))
        .method(t("testEmptyTerm", snf_test_empty_term))
        .method(t("testRandomData", snf_test_random_data))
        .build()
}

// -----------------------------------------------------------------------------
// TestScandinavianNormalizationFilterFactory
// -----------------------------------------------------------------------------

fn snff_test_default() -> Result<()> {
    let stream = ws("räksmörgås_ae_oe_aa_oo_ao_AE_OE_AA_OO_AO")?;
    let mut stream = fac("ScandinavianNormalization", &[])?.create(boxed(stream))?;
    B::assert_token_stream_contents(&mut *stream, &["ræksmørgås_æ_ø_å_ø_å_Æ_Ø_Å_Ø_Å"])
}

/// Test that bogus arguments result in exception
fn snff_test_bogus_arguments() -> Result<()> {
    let expected = iae_message(|| fac("ScandinavianNormalization", &["bogusArg", "bogusValue"]));
    Assert::assert_true_msg(
        &format!("Got {expected}"),
        expected.contains("Unknown parameters"),
    );
    Ok(())
}

fn scandinavian_normalization_filter_factory_class() -> Arc<Class> {
    factory_class(
        "org.apache.lucene.analysis.miscellaneous.TestScandinavianNormalizationFilterFactory",
    )
    .method(t("testDefault", snff_test_default))
    .method(t("testBogusArguments", snff_test_bogus_arguments))
    .build()
}

// -----------------------------------------------------------------------------
// TestScandinavianNormalizer
// -----------------------------------------------------------------------------

/// The anonymous `TokenFilter` of `createAnalyzer`: the term through a
/// `ScandinavianNormalizer`.
#[derive(Debug)]
struct NormalizerFilterLogic {
    normalizer: ScandinavianNormalizer,
}

impl TokenFilterLogic for NormalizerFilterLogic {
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if !input.increment_token()? {
            return Ok(false);
        }
        let mut char_term_attribute = source
            .get_attribute_interface_mut::<dyn CharTermAttribute>()
            .expect("charTermAttribute");
        let length = char_term_attribute.length();
        let new_length = self
            .normalizer
            .process_token(char_term_attribute.buffer_mut(), length);
        char_term_attribute.set_length(new_length);
        Ok(true)
    }
}

/// `createAnalyzer(Set<Foldings> foldings)`.
fn sn_create_analyzer(foldings: &[Foldings]) -> Arc<dyn Analyzer> {
    let foldings: BTreeSet<Foldings> = foldings.iter().copied().collect();
    whitespace_analyzer(
        "org.apache.lucene.analysis.miscellaneous.TestScandinavianNormalizer$1",
        move |tokenizer| {
            let mut stream = TokenFilter::new(
                tokenizer,
                NormalizerFilterLogic {
                    normalizer: ScandinavianNormalizer::new(foldings.clone()),
                },
            );
            stream
                .attribute_source_mut()
                .add_attribute_interface::<dyn CharTermAttribute>()?;
            Ok(boxed(stream))
        },
    )
}

/// `checkOneTerm` over every pair.
fn sn_check(analyzer: &dyn Analyzer, pairs: &[(&str, &str)]) -> Result<()> {
    for (input, expected) in pairs {
        B::check_one_term(analyzer, input, expected)?;
    }
    Ok(())
}

fn sn_test_no_foldings() -> Result<()> {
    let analyzer = sn_create_analyzer(&[]);
    sn_check(
        &*analyzer,
        &[
            ("aa", "aa"),
            ("ao", "ao"),
            ("ae", "ae"),
            ("oo", "oo"),
            ("oe", "oe"),
        ],
    )?;
    analyzer.close();
    Ok(())
}

fn sn_test_ae_folding() -> Result<()> {
    let analyzer = sn_create_analyzer(&[Foldings::AE]);
    sn_check(
        &*analyzer,
        &[
            ("aa", "aa"),
            ("ao", "ao"),
            ("ae", "æ"),
            ("aE", "æ"),
            ("Ae", "Æ"),
            ("AE", "Æ"),
            ("oo", "oo"),
            ("oe", "oe"),
        ],
    )?;
    analyzer.close();
    Ok(())
}

fn sn_test_aa_folding() -> Result<()> {
    let analyzer = sn_create_analyzer(&[Foldings::AA]);
    sn_check(
        &*analyzer,
        &[
            ("aa", "å"),
            ("aA", "å"),
            ("Aa", "Å"),
            ("AA", "Å"),
            ("ao", "ao"),
            ("ae", "ae"),
            ("oo", "oo"),
            ("oe", "oe"),
        ],
    )?;
    analyzer.close();
    Ok(())
}

fn sn_test_oe_folding() -> Result<()> {
    let analyzer = sn_create_analyzer(&[Foldings::OE]);
    sn_check(
        &*analyzer,
        &[
            ("aa", "aa"),
            ("ao", "ao"),
            ("ae", "ae"),
            ("oo", "oo"),
            ("oe", "ø"),
            ("oE", "ø"),
            ("Oe", "Ø"),
            ("OE", "Ø"),
        ],
    )?;
    analyzer.close();
    Ok(())
}

fn sn_test_oo_folding() -> Result<()> {
    let analyzer = sn_create_analyzer(&[Foldings::OO]);
    sn_check(
        &*analyzer,
        &[
            ("aa", "aa"),
            ("ao", "ao"),
            ("ae", "ae"),
            ("oo", "ø"),
            ("oO", "ø"),
            ("Oo", "Ø"),
            ("OO", "Ø"),
            ("oe", "oe"),
        ],
    )?;
    analyzer.close();
    Ok(())
}

fn sn_test_ao_folding() -> Result<()> {
    let analyzer = sn_create_analyzer(&[Foldings::AO]);
    sn_check(
        &*analyzer,
        &[
            ("aa", "aa"),
            ("ao", "å"),
            ("aO", "å"),
            ("Ao", "Å"),
            ("AO", "Å"),
            ("ae", "ae"),
            ("oo", "oo"),
            ("oe", "oe"),
        ],
    )?;
    analyzer.close();
    Ok(())
}

fn scandinavian_normalizer_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.miscellaneous.TestScandinavianNormalizer")
        .method(t("testNoFoldings", sn_test_no_foldings))
        .method(t("testAeFolding", sn_test_ae_folding))
        .method(t("testAaFolding", sn_test_aa_folding))
        .method(t("testOeFolding", sn_test_oe_folding))
        .method(t("testOoFolding", sn_test_oo_folding))
        .method(t("testAoFolding", sn_test_ao_folding))
        .build()
}

// -----------------------------------------------------------------------------
// TestTrimFilter
// -----------------------------------------------------------------------------

fn tf_test_trim() -> Result<()> {
    let ts = canned(vec![
        tok(" a ", 1, 5),
        tok("b   ", 6, 10),
        tok("cCc", 11, 15),
        tok("   ", 16, 20),
        tok("", 21, 21),
    ]);
    let mut ts = new_trim_filter(ts)?;
    B::assert_token_stream_contents(&mut ts, &["a", "b", "cCc", "", ""])
}

/// blast some random strings through the analyzer
fn tf_test_random_strings() -> Result<()> {
    let a = mock_tokenizer_analyzer(
        "org.apache.lucene.analysis.miscellaneous.TestTrimFilter$1",
        Arc::clone(&KEYWORD),
        false,
        |tokenizer| Ok(boxed(new_trim_filter(tokenizer)?)),
    );
    B::check_random_data(&*random(), &a, 200 * *RANDOM_MULTIPLIER)?;
    a.close();
    Ok(())
}

fn tf_test_empty_term() -> Result<()> {
    let a = keyword_tokenizer_analyzer(
        "org.apache.lucene.analysis.miscellaneous.TestTrimFilter$2",
        |tokenizer| Ok(boxed(new_trim_filter(tokenizer)?)),
    );
    B::check_one_term(&*a, "", "")?;
    a.close();
    Ok(())
}

fn trim_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.miscellaneous.TestTrimFilter")
        .method(t("testTrim", tf_test_trim))
        .method(t("testRandomStrings", tf_test_random_strings))
        .method(t("testEmptyTerm", tf_test_empty_term))
        .build()
}

// -----------------------------------------------------------------------------
// TestTrimFilterFactory
// -----------------------------------------------------------------------------

fn tff_test_trimming() -> Result<()> {
    let stream = kw("trim me    ")?;
    let mut stream = fac("Trim", &[])?.create(boxed(stream))?;
    B::assert_token_stream_contents(&mut *stream, &["trim me"])
}

/// Test that bogus arguments result in exception
fn tff_test_bogus_arguments() -> Result<()> {
    assert_bogus("Trim", &["bogusArg", "bogusValue"])
}

fn trim_filter_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.miscellaneous.TestTrimFilterFactory")
        .method(t("testTrimming", tff_test_trimming))
        .method(t("testBogusArguments", tff_test_bogus_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// TestTruncateTokenFilter
// -----------------------------------------------------------------------------

/// The last expected term of the char-truncating tests:
/// `"1234" + "😃".charAt(0)`, a lone high surrogate, which `rucene`'s
/// `CharTermAttribute` (it holds `char`s) carries as U+FFFD — the
/// divergence declared on `TruncateTokenFilter` and on
/// `rucene::analysis::CharTermAttribute`.
const CUT_PAIR: &str = "1234\u{FFFD}";

fn ttf_test_legacy_truncating() -> Result<()> {
    let stream = ws("abcdefg 1234567 ABCDEFG abcde abc 12345 123 1234567 1234😃5")?;
    let mut stream = truncate_after_chars(boxed(stream), 5)?;
    B::assert_token_stream_contents(
        &mut stream,
        &[
            "abcde", "12345", "ABCDE", "abcde", "abc", "12345", "123", "12345", CUT_PAIR,
        ],
    )
}

/// The input of the code-point truncating tests.
const CODE_POINT_INPUT: &str =
    "abcdefg 1234567 ABCDEFG abcde abc 12345 123 1234😃5 1 😃 😃12345 😃😃 😃😃😃 😃😃😃😃 😃😃😃😃😃 😃😃😃😃😃😃";

/// The output of the code-point truncating tests.
const CODE_POINT_OUTPUT: [&str; 16] = [
    "abcde",
    "12345",
    "ABCDE",
    "abcde",
    "abc",
    "12345",
    "123",
    "1234😃",
    "1",
    "😃",
    "😃1234",
    "😃😃",
    "😃😃😃",
    "😃😃😃😃",
    "😃😃😃😃😃",
    "😃😃😃😃😃",
];

fn ttf_test_code_point_truncating() -> Result<()> {
    let stream = ws(CODE_POINT_INPUT)?;
    let mut stream = truncate_after_code_points(boxed(stream), 5)?;
    B::assert_token_stream_contents(&mut stream, &CODE_POINT_OUTPUT)
}

/// `Character.codePointCount(termAtt, 0, termAtt.length())`.
fn code_point_count(ts: &dyn TokenStream) -> i32 {
    term(ts).chars().count() as i32
}

fn ttf_test_random() -> Result<()> {
    let rnd = random();
    for _ in 0..50 * *RANDOM_MULTIPLIER {
        let truncate_length = rnd.next_int_bound(5) + 1;
        let text = TestUtil::random_analysis_string(&*rnd, 200, false);
        let mut ts1 = ws(&text)?;
        ts1.attribute_source_mut()
            .add_attribute_interface::<dyn CharTermAttribute>()?;
        let mut ts2 = truncate_after_code_points(boxed(ws(&text)?), truncate_length)?;
        ts2.attribute_source_mut()
            .add_attribute_interface::<dyn CharTermAttribute>()?;
        ts1.reset()?;
        ts2.reset()?;
        while ts2.increment_token()? {
            Assert::assert_true(ts1.increment_token()?);
            let len1 = code_point_count(&ts1);
            let len2 = code_point_count(&ts2);
            if len1 <= truncate_length {
                Assert::assert_equals(len1, len2);
                Assert::assert_equals(term(&ts1), term(&ts2));
            } else {
                Assert::assert_equals(truncate_length, len2);
                Assert::assert_true(term(&ts1).starts_with(&term(&ts2)));
            }
        }
        Assert::assert_false(ts1.increment_token()?);
        ts1.end()?;
        ts2.end()?;
        ts1.close()?;
        ts2.close()?;
    }
    Ok(())
}

fn ttf_test_stress_random() -> Result<()> {
    let rnd = random();
    let truncate_length = rnd.next_int_bound(5) + 1;
    let a = whitespace_analyzer(
        "org.apache.lucene.analysis.miscellaneous.TestTruncateTokenFilter$1",
        move |tokenizer| {
            Ok(boxed(truncate_after_code_points(
                tokenizer,
                truncate_length,
            )?))
        },
    );
    B::check_random_data_max_word_length(&*rnd, &a, 20 * *RANDOM_MULTIPLIER, truncate_length * 2)
}

fn ttf_test_legacy_non_positive_length() -> Result<()> {
    truncate_after_chars(boxed(ws("param must be a positive number")?), -48)?;
    Ok(())
}

fn ttf_test_non_positive_length() -> Result<()> {
    truncate_after_code_points(boxed(ws("param must be a positive number")?), -48)?;
    Ok(())
}

fn truncate_token_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.miscellaneous.TestTruncateTokenFilter")
        .method(t("testLegacyTruncating", ttf_test_legacy_truncating))
        .method(t("testCodePointTruncating", ttf_test_code_point_truncating))
        .method(t("testRandom", ttf_test_random))
        .method(t("testStressRandom", ttf_test_stress_random))
        .method(t_iae(
            "testLegacyNonPositiveLength",
            ttf_test_legacy_non_positive_length,
        ))
        .method(t_iae("testNonPositiveLength", ttf_test_non_positive_length))
        .build()
}

// -----------------------------------------------------------------------------
// TestTruncateTokenFilterFactory
// -----------------------------------------------------------------------------

/// `testCharTruncating(String param)`.
fn ttff_char_truncating(param: Option<&str>) -> Result<()> {
    let mut tokenizer = MockTokenizer::new(Arc::clone(&WHITESPACE), false)?;
    tokenizer.set_reader(Box::new(StringCharReader::new(
        "abcdefg 1234567 ABCDEFG abcde abc 12345 123 1234567 1234😃5",
    )))?;
    let factory = match param {
        None => token_filter_factory_version(MISC, "truncate", Version::LUCENE_10_0_0, &[])?,
        Some(param) => {
            token_filter_factory_version(MISC, "Truncate", Version::LUCENE_10_0_0, &[param, "5"])?
        }
    };
    let mut stream = factory.create(boxed(tokenizer))?;
    B::assert_token_stream_contents(
        &mut *stream,
        &[
            "abcde", "12345", "ABCDE", "abcde", "abc", "12345", "123", "12345", CUT_PAIR,
        ],
    )
}

fn ttff_test_char_truncating() -> Result<()> {
    ttff_char_truncating(Some(TruncateTokenFilterFactory::TRUNCATE_AFTER_CHARS_KEY))?;
    ttff_char_truncating(Some(TruncateTokenFilterFactory::PREFIX_LENGTH_KEY))?;
    ttff_char_truncating(None)
}

/// `testCodePointTruncating(String param)`: a public method with a
/// parameter, which the runner does not run as a test.
fn ttff_code_point_truncating(param: Option<&str>) -> Result<()> {
    let mut tokenizer = MockTokenizer::new(Arc::clone(&WHITESPACE), false)?;
    tokenizer.set_reader(Box::new(StringCharReader::new(CODE_POINT_INPUT)))?;
    let factory = match param {
        None => token_filter_factory_version(MISC, "truncate", Version::LATEST, &[])?,
        Some(param) => {
            token_filter_factory_version(MISC, "Truncate", Version::LATEST, &[param, "5"])?
        }
    };
    let mut stream = factory.create(boxed(tokenizer))?;
    B::assert_token_stream_contents(&mut *stream, &CODE_POINT_OUTPUT)
}

fn ttff_test_code_point_truncating() -> Result<()> {
    ttff_code_point_truncating(Some(
        TruncateTokenFilterFactory::TRUNCATE_AFTER_CODEPOINTS_KEY,
    ))?;
    ttff_code_point_truncating(Some(TruncateTokenFilterFactory::PREFIX_LENGTH_KEY))?;
    ttff_code_point_truncating(None)
}

/// Test that bogus arguments result in exception
fn ttff_test_bogus_arguments() -> Result<()> {
    let expected = iae_message(|| {
        fac(
            "Truncate",
            &[
                TruncateTokenFilterFactory::TRUNCATE_AFTER_CODEPOINTS_KEY,
                "5",
                "bogusArg",
                "bogusValue",
            ],
        )
    });
    Assert::assert_true(expected.contains("Unknown parameter(s):"));

    let expected = iae_message(|| {
        fac(
            "Truncate",
            &[
                TruncateTokenFilterFactory::TRUNCATE_AFTER_CODEPOINTS_KEY,
                "5",
                TruncateTokenFilterFactory::TRUNCATE_AFTER_CHARS_KEY,
                "6",
            ],
        )
    });
    Assert::assert_true(expected.contains("Can only give one of the following parameters:"));
    Ok(())
}

/// Test that negative prefix length result in exception
fn ttff_test_non_positive_prefix_length_argument() -> Result<()> {
    let expected = iae_message(|| {
        fac(
            "Truncate",
            &[
                TruncateTokenFilterFactory::TRUNCATE_AFTER_CODEPOINTS_KEY,
                "-5",
            ],
        )
    });
    Assert::assert_true(expected.contains(&format!(
        "{} parameter must be a positive number: -5",
        TruncateTokenFilterFactory::TRUNCATE_AFTER_CODEPOINTS_KEY
    )));
    Ok(())
}

fn truncate_token_filter_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.miscellaneous.TestTruncateTokenFilterFactory")
        .method(t("testCharTruncating", ttff_test_char_truncating))
        .method(t(
            "testCodePointTruncating",
            ttff_test_code_point_truncating,
        ))
        .method(t("testBogusArguments", ttff_test_bogus_arguments))
        .method(t(
            "testNonPositivePrefixLengthArgument",
            ttff_test_non_positive_prefix_length_argument,
        ))
        .build()
}

// -----------------------------------------------------------------------------
// TestTypeAsSynonymFilter
// -----------------------------------------------------------------------------

/// Test the straight forward case with the simplest constructor. Simply
/// converts every type to a synonym. Typically one wants to also set an
/// ignore list containing "word" unless that default value is removed by
/// prior analysis.
fn tas_test_simple() -> Result<()> {
    let mut token = Token::with_offsets("foo", 0, 2);
    token.set_type("bar".to_string());
    let mut token2 = Token::with_offsets("foo", 4, 6);
    token2.set_flags(5);

    let ts = canned(vec![token, token2]);
    let mut ts = new_type_as_synonym_filter(ts)?;

    // "word" is the default type!
    B::assert_token_stream_contents_flags(
        &mut ts,
        &["foo", "bar", "foo", "word"],
        Some(&[0, 0, 4, 4]),
        Some(&[2, 2, 6, 6]),
        None, // not testing types
        Some(&[1, 0, 1, 0]),
        None, // positions
        // final values, keywords, graph, payloads not tested here
        None,
        None,
        None,
        false,
        None,
        // ensure basic case continues to copy flags to synonym by default for back compat
        Some(&[0, 0, 5, 5]),
    )
}

/// Tests that we can add a prefix to the synonym (for example, to keep it
/// from ever matching user input directly), and test that we can ignore a
/// list of type values we don't wish to turn into synonyms.
fn tas_test_with_prefix_and_ignore() -> Result<()> {
    let mut tokens = vec![tok("foo", 1, 3), tok("foo", 5, 7), tok("foo", 9, 11)];
    tokens[0].set_type("bar".to_string());
    tokens[2].set_type("ignoreme".to_string());

    let ts = canned(tokens);
    let ignore: HashSet<String> = ["word", "ignoreme"].iter().map(|s| s.to_string()).collect();
    let mut ts = new_type_as_synonym_filter_with(ts, Some("pfx_".to_string()), Some(ignore), 0)?;

    B::assert_token_stream_contents_offsets_pos_increments(
        &mut ts,
        &["foo", "pfx_bar", "foo", "foo"],
        &[1, 1, 5, 9],
        &[3, 3, 7, 11],
        &[1, 0, 1, 1],
    )
}

/// Analysis chains that make use of flags may or may not want flags
/// transferred to the synonym to be created. This tests the mask that can be
/// used to control which flag bits are transferred.
fn tas_test_flag_mask() -> Result<()> {
    let mut token = Token::with_offsets("foo", 0, 2);
    token.set_type("bar".to_string());
    token.set_flags(7);
    let token2 = Token::with_offsets("foo", 4, 6);

    let ts = canned(vec![token, token2]);
    let mut ts = new_type_as_synonym_filter_with(ts, Some(String::new()), Some(HashSet::new()), 5)?;

    // "word" is the default type!
    B::assert_token_stream_contents_flags(
        &mut ts,
        &["foo", "bar", "foo", "word"],
        Some(&[0, 0, 4, 4]),
        Some(&[2, 2, 6, 6]),
        None, // not testing types
        None,
        None, // positions tested above
        // final values, keywords, graph, payloads not tested here
        None,
        None,
        None,
        false,
        None,
        Some(&[7, 5, 0, 0]),
    )
}

fn type_as_synonym_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.miscellaneous.TestTypeAsSynonymFilter")
        .method(t("testSimple", tas_test_simple))
        .method(t(
            "testWithPrefixAndIgnore",
            tas_test_with_prefix_and_ignore,
        ))
        .method(t("testFlagMask", tas_test_flag_mask))
        .build()
}

// -----------------------------------------------------------------------------
// TestTypeAsSynonymFilterFactory
// -----------------------------------------------------------------------------

/// `token(String term, String type)`.
fn tasf_token(term: &str, type_value: &str) -> Token {
    let mut token = Token::new();
    token.set_empty();
    token.append_string(Some(term));
    token.set_type(type_value.to_string());
    token
}

/// `TOKENS`.
fn tasf_tokens() -> Vec<Token> {
    vec![
        tasf_token("Visit", "<ALPHANUM>"),
        tasf_token("example.com", "<URL>"),
    ]
}

fn tasf_test_basic() -> Result<()> {
    let stream = canned(tasf_tokens());
    let mut stream = fac("TypeAsSynonym", &[])?.create(stream)?;
    B::assert_token_stream_contents_types(
        &mut *stream,
        &["Visit", "<ALPHANUM>", "example.com", "<URL>"],
        None,
        None,
        Some(&["<ALPHANUM>", "<ALPHANUM>", "<URL>", "<URL>"]),
        Some(&[1, 0, 1, 0]),
    )
}

fn tasf_test_prefix() -> Result<()> {
    let stream = canned(tasf_tokens());
    let mut stream = fac("TypeAsSynonym", &["prefix", "_type_"])?.create(stream)?;
    B::assert_token_stream_contents_types(
        &mut *stream,
        &["Visit", "_type_<ALPHANUM>", "example.com", "_type_<URL>"],
        None,
        None,
        Some(&["<ALPHANUM>", "<ALPHANUM>", "<URL>", "<URL>"]),
        Some(&[1, 0, 1, 0]),
    )
}

fn tasf_test_ignore() -> Result<()> {
    let stream = canned(tasf_tokens());
    let mut stream = fac(
        "typeAsSynonym",
        &["prefix", "_type_", "ignore", "<ALPHANUM>"],
    )?
    .create(stream)?;
    B::assert_token_stream_contents_types(
        &mut *stream,
        &["Visit", "example.com", "_type_<URL>"],
        None,
        None,
        Some(&["<ALPHANUM>", "<URL>", "<URL>"]),
        Some(&[1, 1, 0]),
    )
}

fn type_as_synonym_filter_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.miscellaneous.TestTypeAsSynonymFilterFactory")
        .method(t("testBasic", tasf_test_basic))
        .method(t("testPrefix", tasf_test_prefix))
        .method(t("testIgnore", tasf_test_ignore))
        .build()
}

// -----------------------------------------------------------------------------
// The suites
// -----------------------------------------------------------------------------

#[test]
fn test_ascii_folding_filter() {
    run(ascii_folding_filter_class);
}

#[test]
fn test_ascii_folding_filter_factory() {
    run(ascii_folding_filter_factory_class);
}

#[test]
fn test_capitalization_filter() {
    run(capitalization_filter_class);
}

#[test]
fn test_capitalization_filter_factory() {
    run(capitalization_filter_factory_class);
}

#[test]
fn test_codepoint_count_filter() {
    run(codepoint_count_filter_class);
}

#[test]
fn test_codepoint_count_filter_factory() {
    run(codepoint_count_filter_factory_class);
}

#[test]
fn test_delimited_term_frequency_token_filter() {
    run(delimited_term_frequency_token_filter_class);
}

#[test]
fn test_drop_if_flagged_filter() {
    run(drop_if_flagged_filter_class);
}

#[test]
fn test_drop_if_flagged_filter_factory() {
    run(drop_if_flagged_filter_factory_class);
}

#[test]
fn test_empty_token_stream() {
    run(empty_token_stream_class);
}

#[test]
fn test_fix_broken_offsets_filter() {
    run(fix_broken_offsets_filter_class);
}

#[test]
fn test_hyphenated_words_filter() {
    run(hyphenated_words_filter_class);
}

#[test]
fn test_keep_filter_factory() {
    run(keep_filter_factory_class);
}

#[test]
fn test_keep_word_filter() {
    run(keep_word_filter_class);
}

#[test]
fn test_keyword_marker_filter() {
    run(keyword_marker_filter_class);
}

#[test]
fn test_keyword_marker_filter_factory() {
    run(keyword_marker_filter_factory_class);
}

#[test]
fn test_length_filter() {
    run(length_filter_class);
}

#[test]
fn test_length_filter_factory() {
    run(length_filter_factory_class);
}

#[test]
fn test_limit_token_count_analyzer() {
    run(limit_token_count_analyzer_class);
}

#[test]
fn test_limit_token_count_filter() {
    run(limit_token_count_filter_class);
}

#[test]
fn test_limit_token_count_filter_factory() {
    run(limit_token_count_filter_factory_class);
}

#[test]
fn test_limit_token_offset_filter() {
    run(limit_token_offset_filter_class);
}

#[test]
fn test_limit_token_offset_filter_factory() {
    run(limit_token_offset_filter_factory_class);
}

#[test]
fn test_limit_token_position_filter_factory() {
    run(limit_token_position_filter_factory_class);
}

#[test]
fn test_per_field_analyzer_wrapper() {
    run(per_field_analyzer_wrapper_class);
}

#[test]
fn test_remove_duplicates_token_filter_factory() {
    run(remove_duplicates_token_filter_factory_class);
}

#[test]
fn test_scandinavian_folding_filter() {
    run(scandinavian_folding_filter_class);
}

#[test]
fn test_scandinavian_folding_filter_factory() {
    run(scandinavian_folding_filter_factory_class);
}

#[test]
fn test_scandinavian_normalization_filter() {
    run(scandinavian_normalization_filter_class);
}

#[test]
fn test_scandinavian_normalization_filter_factory() {
    run(scandinavian_normalization_filter_factory_class);
}

#[test]
fn test_scandinavian_normalizer() {
    run(scandinavian_normalizer_class);
}

#[test]
fn test_trim_filter() {
    run(trim_filter_class);
}

#[test]
fn test_trim_filter_factory() {
    run(trim_filter_factory_class);
}

#[test]
fn test_truncate_token_filter() {
    run(truncate_token_filter_class);
}

#[test]
fn test_truncate_token_filter_factory() {
    run(truncate_token_filter_factory_class);
}

#[test]
fn test_type_as_synonym_filter() {
    run(type_as_synonym_filter_class);
}

#[test]
fn test_type_as_synonym_filter_factory() {
    run(type_as_synonym_filter_factory_class);
}
