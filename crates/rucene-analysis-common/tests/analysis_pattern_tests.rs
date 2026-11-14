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
//! that rmp task #458 lists, each run as a suite by the ported runner: the
//! `org.apache.lucene.analysis.pattern` classes
//! `TestPatternCaptureGroupTokenFilter`, `TestPatternReplaceCharFilter`,
//! `TestPatternReplaceCharFilterFactory`, `TestPatternReplaceFilter`,
//! `TestPatternReplaceFilterFactory`, `TestPatternTokenizer`,
//! `TestPatternTokenizerFactory`, `TestPatternTypingFilter`,
//! `TestPatternTypingFilterFactory`, `TestSimplePatternSplitTokenizer` and
//! `TestSimplePatternTokenizer`.
//!
//! Not ported, as Lucene does not run it either:
//! `TestPatternReplaceCharFilter.testNastyPattern` (`@Ignore`).
//! `TestPatternTokenizer.testHeapFreedAfterClose` is `@Nightly`: Lucene runs
//! it only with `tests.nightly=true`. The port runs it in every run, in its
//! own suite of the class without the annotation
//! ([`test_pattern_tokenizer_nightly`]): setting `tests.nightly` would change
//! process-wide state (`TEST_NIGHTLY` is read once), which a test run in
//! parallel with others must not do.

mod support;

use std::sync::Arc;

use randomizedtesting::jdk::random::JdkRandom;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::java_regex::JavaPattern;
use rucene::analysis::tokenattributes::{CharTermAttribute, OffsetAttribute};
use rucene::analysis::{Analyzer, CharFilter, CharReader, StringCharReader, TokenStream};
use rucene::error::Result;
use rucene::util::automaton::Automaton;
use rucene::util::extra::Version;
use rucene_analysis_common::analysis::charfilter::mapping_char_filter::MappingCharFilter;
use rucene_analysis_common::analysis::charfilter::normalize_char_map::Builder as NormalizeCharMapBuilder;
use rucene_analysis_common::analysis::pattern::pattern_capture_group_token_filter::new_pattern_capture_group_token_filter;
use rucene_analysis_common::analysis::pattern::pattern_replace_char_filter::PatternReplaceCharFilter;
use rucene_analysis_common::analysis::pattern::pattern_replace_filter::new_pattern_replace_filter;
use rucene_analysis_common::analysis::pattern::pattern_tokenizer::{
    new_pattern_tokenizer, new_pattern_tokenizer_with_factory,
};
use rucene_analysis_common::analysis::pattern::pattern_typing_filter::{
    new_pattern_typing_filter, PatternTypingRule,
};
use rucene_analysis_common::analysis::pattern::simple_pattern_split_tokenizer::{
    new_simple_pattern_split_tokenizer, new_simple_pattern_split_tokenizer_from_automaton,
};
use rucene_analysis_common::analysis::pattern::simple_pattern_tokenizer::{
    new_simple_pattern_tokenizer, new_simple_pattern_tokenizer_from_automaton,
};
use rucene_test_framework::analysis::mock_tokenizer::{MockTokenizer, WHITESPACE};
use rucene_test_framework::analysis::token::Token;
use rucene_test_framework::analysis::{
    tokenizer_components, BaseTokenStreamFactoryTestCase, BaseTokenStreamTestCase,
    CannedTokenStream,
};
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::RANDOM_MULTIPLIER;
use rucene_test_framework::util::{LuceneTestCase, StringMockResourceLoader, TestUtil};
use support::{
    boxed, btstc_class, factory_class, iae_message, keyword_tokenizer_analyzer, own, run, t,
    whitespace_analyzer, FnAnalyzer,
};

type B = BaseTokenStreamTestCase;
type F = BaseTokenStreamFactoryTestCase;

fn random() -> Arc<dyn randomizedtesting::jdk::random::Random> {
    LuceneTestCase::random()
}

fn mult() -> i32 {
    *RANDOM_MULTIPLIER
}

/// `Pattern.compile(p)`.
fn pattern(p: &str) -> JavaPattern {
    JavaPattern::compile(p, 0).expect("a valid pattern")
}

fn reader(s: &str) -> Box<dyn CharReader> {
    Box::new(StringCharReader::new(s))
}

/// `termAtt.toString()` of a stream.
fn term_of(ts: &dyn TokenStream) -> String {
    support::term(ts)
}

/// `tsToString(in)`: the terms, space-separated, with bogus values assigned
/// between tokens.
fn ts_to_string(input: &mut dyn TokenStream) -> Result<String> {
    let mut out = String::new();
    input
        .attribute_source_mut()
        .add_attribute_interface::<dyn CharTermAttribute>()?;
    let set_bogus = |input: &dyn TokenStream| {
        // extra safety to enforce, that the state is not preserved and also
        // assign bogus values
        input.attribute_source().clear_attributes();
        let mut term = input
            .attribute_source()
            .get_attribute_interface_mut::<dyn CharTermAttribute>()
            .expect("termAtt");
        term.set_empty();
        term.append_string(Some("bogusTerm"));
    };
    set_bogus(input);
    input.reset()?;
    while input.increment_token()? {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&term_of(input));
        set_bogus(input);
    }
    input.close()?;
    Ok(out)
}

/// `"G&uuml;nther G&uuml;nther is here"` through `MappingCharFilter("&uuml;" => "ü")`.
const GUNTHER: &str = "G&uuml;nther G&uuml;nther is here";

fn gunther_char_stream() -> Result<Box<dyn CharReader>> {
    let mut builder = NormalizeCharMapBuilder::new();
    builder.add("&uuml;", "\u{FC}")?;
    let norm_map = Arc::new(builder.build()?);
    Ok(Box::new(MappingCharFilter::new(norm_map, reader(GUNTHER))))
}

// -----------------------------------------------------------------------------
// TestPatternCaptureGroupTokenFilter
// -----------------------------------------------------------------------------

/// `testPatterns(input, regexes, tokens, startOffsets, endOffsets, positions,
/// preserveOriginal)`.
fn test_patterns(
    input: &str,
    regexes: &[&str],
    tokens: &[&str],
    start_offsets: &[i32],
    end_offsets: &[i32],
    positions: &[i32],
    preserve_original: bool,
) -> Result<()> {
    let patterns: Vec<JavaPattern> = regexes.iter().map(|r| pattern(r)).collect();
    let mut tokenizer = MockTokenizer::new(Arc::clone(&WHITESPACE), false)?;
    tokenizer.set_reader(reader(input))?;
    let mut ts =
        new_pattern_capture_group_token_filter(boxed(tokenizer), preserve_original, &patterns)?;
    B::assert_token_stream_contents_offsets_pos_increments(
        &mut ts,
        tokens,
        start_offsets,
        end_offsets,
        positions,
    )
}

fn pcg_test_no_pattern() -> Result<()> {
    test_patterns("foobarbaz", &[], &["foobarbaz"], &[0], &[9], &[1], false)?;
    test_patterns("foobarbaz", &[], &["foobarbaz"], &[0], &[9], &[1], true)?;
    test_patterns(
        "foo bar baz",
        &[],
        &["foo", "bar", "baz"],
        &[0, 4, 8],
        &[3, 7, 11],
        &[1, 1, 1],
        false,
    )?;
    test_patterns(
        "foo bar baz",
        &[],
        &["foo", "bar", "baz"],
        &[0, 4, 8],
        &[3, 7, 11],
        &[1, 1, 1],
        true,
    )?;
    Ok(())
}

fn pcg_test_no_match() -> Result<()> {
    test_patterns(
        "foobarbaz",
        &["xx"],
        &["foobarbaz"],
        &[0],
        &[9],
        &[1],
        false,
    )?;
    test_patterns("foobarbaz", &["xx"], &["foobarbaz"], &[0], &[9], &[1], true)?;
    test_patterns(
        "foo bar baz",
        &["xx"],
        &["foo", "bar", "baz"],
        &[0, 4, 8],
        &[3, 7, 11],
        &[1, 1, 1],
        false,
    )?;
    test_patterns(
        "foo bar baz",
        &["xx"],
        &["foo", "bar", "baz"],
        &[0, 4, 8],
        &[3, 7, 11],
        &[1, 1, 1],
        true,
    )?;
    Ok(())
}

fn pcg_test_no_capture() -> Result<()> {
    test_patterns(
        "foobarbaz",
        &[".."],
        &["foobarbaz"],
        &[0],
        &[9],
        &[1],
        false,
    )?;
    test_patterns("foobarbaz", &[".."], &["foobarbaz"], &[0], &[9], &[1], true)?;
    test_patterns(
        "foo bar baz",
        &[".."],
        &["foo", "bar", "baz"],
        &[0, 4, 8],
        &[3, 7, 11],
        &[1, 1, 1],
        false,
    )?;
    test_patterns(
        "foo bar baz",
        &[".."],
        &["foo", "bar", "baz"],
        &[0, 4, 8],
        &[3, 7, 11],
        &[1, 1, 1],
        true,
    )?;
    Ok(())
}

fn pcg_test_empty_capture() -> Result<()> {
    test_patterns(
        "foobarbaz",
        &[".(y*)"],
        &["foobarbaz"],
        &[0],
        &[9],
        &[1],
        false,
    )?;
    test_patterns(
        "foobarbaz",
        &[".(y*)"],
        &["foobarbaz"],
        &[0],
        &[9],
        &[1],
        true,
    )?;
    test_patterns(
        "foo bar baz",
        &[".(y*)"],
        &["foo", "bar", "baz"],
        &[0, 4, 8],
        &[3, 7, 11],
        &[1, 1, 1],
        false,
    )?;
    test_patterns(
        "foo bar baz",
        &[".(y*)"],
        &["foo", "bar", "baz"],
        &[0, 4, 8],
        &[3, 7, 11],
        &[1, 1, 1],
        true,
    )?;
    Ok(())
}

fn pcg_test_capture_all() -> Result<()> {
    test_patterns(
        "foobarbaz",
        &["(.+)"],
        &["foobarbaz"],
        &[0],
        &[9],
        &[1],
        false,
    )?;
    test_patterns(
        "foobarbaz",
        &["(.+)"],
        &["foobarbaz"],
        &[0],
        &[9],
        &[1],
        true,
    )?;
    test_patterns(
        "foo bar baz",
        &["(.+)"],
        &["foo", "bar", "baz"],
        &[0, 4, 8],
        &[3, 7, 11],
        &[1, 1, 1],
        false,
    )?;
    test_patterns(
        "foo bar baz",
        &["(.+)"],
        &["foo", "bar", "baz"],
        &[0, 4, 8],
        &[3, 7, 11],
        &[1, 1, 1],
        true,
    )?;
    Ok(())
}

fn pcg_test_capture_start() -> Result<()> {
    test_patterns("foobarbaz", &["^(.)"], &["f"], &[0], &[9], &[1], false)?;
    test_patterns(
        "foobarbaz",
        &["^(.)"],
        &["foobarbaz", "f"],
        &[0, 0],
        &[9, 9],
        &[1, 0],
        true,
    )?;
    test_patterns(
        "foo bar baz",
        &["^(.)"],
        &["f", "b", "b"],
        &[0, 4, 8],
        &[3, 7, 11],
        &[1, 1, 1],
        false,
    )?;
    test_patterns(
        "foo bar baz",
        &["^(.)"],
        &["foo", "f", "bar", "b", "baz", "b"],
        &[0, 0, 4, 4, 8, 8],
        &[3, 3, 7, 7, 11, 11],
        &[1, 0, 1, 0, 1, 0],
        true,
    )?;
    Ok(())
}

fn pcg_test_capture_middle() -> Result<()> {
    test_patterns("foobarbaz", &["^.(.)."], &["o"], &[0], &[9], &[1], false)?;
    test_patterns(
        "foobarbaz",
        &["^.(.)."],
        &["foobarbaz", "o"],
        &[0, 0],
        &[9, 9],
        &[1, 0],
        true,
    )?;
    test_patterns(
        "foo bar baz",
        &["^.(.)."],
        &["o", "a", "a"],
        &[0, 4, 8],
        &[3, 7, 11],
        &[1, 1, 1],
        false,
    )?;
    test_patterns(
        "foo bar baz",
        &["^.(.)."],
        &["foo", "o", "bar", "a", "baz", "a"],
        &[0, 0, 4, 4, 8, 8],
        &[3, 3, 7, 7, 11, 11],
        &[1, 0, 1, 0, 1, 0],
        true,
    )?;
    Ok(())
}

fn pcg_test_capture_end() -> Result<()> {
    test_patterns("foobarbaz", &["(.)$"], &["z"], &[0], &[9], &[1], false)?;
    test_patterns(
        "foobarbaz",
        &["(.)$"],
        &["foobarbaz", "z"],
        &[0, 0],
        &[9, 9],
        &[1, 0],
        true,
    )?;
    test_patterns(
        "foo bar baz",
        &["(.)$"],
        &["o", "r", "z"],
        &[0, 4, 8],
        &[3, 7, 11],
        &[1, 1, 1],
        false,
    )?;
    test_patterns(
        "foo bar baz",
        &["(.)$"],
        &["foo", "o", "bar", "r", "baz", "z"],
        &[0, 0, 4, 4, 8, 8],
        &[3, 3, 7, 7, 11, 11],
        &[1, 0, 1, 0, 1, 0],
        true,
    )?;
    Ok(())
}

fn pcg_test_capture_start_middle() -> Result<()> {
    test_patterns(
        "foobarbaz",
        &["^(.)(.)"],
        &["f", "o"],
        &[0, 0],
        &[9, 9],
        &[1, 0],
        false,
    )?;
    test_patterns(
        "foobarbaz",
        &["^(.)(.)"],
        &["foobarbaz", "f", "o"],
        &[0, 0, 0],
        &[9, 9, 9],
        &[1, 0, 0],
        true,
    )?;
    test_patterns(
        "foo bar baz",
        &["^(.)(.)"],
        &["f", "o", "b", "a", "b", "a"],
        &[0, 0, 4, 4, 8, 8],
        &[3, 3, 7, 7, 11, 11],
        &[1, 0, 1, 0, 1, 0],
        false,
    )?;
    test_patterns(
        "foo bar baz",
        &["^(.)(.)"],
        &["foo", "f", "o", "bar", "b", "a", "baz", "b", "a"],
        &[0, 0, 0, 4, 4, 4, 8, 8, 8],
        &[3, 3, 3, 7, 7, 7, 11, 11, 11],
        &[1, 0, 0, 1, 0, 0, 1, 0, 0],
        true,
    )?;
    Ok(())
}

fn pcg_test_capture_start_end() -> Result<()> {
    test_patterns(
        "foobarbaz",
        &["^(.).+(.)$"],
        &["f", "z"],
        &[0, 0],
        &[9, 9],
        &[1, 0],
        false,
    )?;
    test_patterns(
        "foobarbaz",
        &["^(.).+(.)$"],
        &["foobarbaz", "f", "z"],
        &[0, 0, 0],
        &[9, 9, 9],
        &[1, 0, 0],
        true,
    )?;
    test_patterns(
        "foo bar baz",
        &["^(.).+(.)$"],
        &["f", "o", "b", "r", "b", "z"],
        &[0, 0, 4, 4, 8, 8],
        &[3, 3, 7, 7, 11, 11],
        &[1, 0, 1, 0, 1, 0],
        false,
    )?;
    test_patterns(
        "foo bar baz",
        &["^(.).+(.)$"],
        &["foo", "f", "o", "bar", "b", "r", "baz", "b", "z"],
        &[0, 0, 0, 4, 4, 4, 8, 8, 8],
        &[3, 3, 3, 7, 7, 7, 11, 11, 11],
        &[1, 0, 0, 1, 0, 0, 1, 0, 0],
        true,
    )?;
    Ok(())
}

fn pcg_test_capture_middle_end() -> Result<()> {
    test_patterns(
        "foobarbaz",
        &["(.)(.)$"],
        &["a", "z"],
        &[0, 0],
        &[9, 9],
        &[1, 0],
        false,
    )?;
    test_patterns(
        "foobarbaz",
        &["(.)(.)$"],
        &["foobarbaz", "a", "z"],
        &[0, 0, 0],
        &[9, 9, 9],
        &[1, 0, 0],
        true,
    )?;
    test_patterns(
        "foo bar baz",
        &["(.)(.)$"],
        &["o", "o", "a", "r", "a", "z"],
        &[0, 0, 4, 4, 8, 8],
        &[3, 3, 7, 7, 11, 11],
        &[1, 0, 1, 0, 1, 0],
        false,
    )?;
    test_patterns(
        "foo bar baz",
        &["(.)(.)$"],
        &["foo", "o", "o", "bar", "a", "r", "baz", "a", "z"],
        &[0, 0, 0, 4, 4, 4, 8, 8, 8],
        &[3, 3, 3, 7, 7, 7, 11, 11, 11],
        &[1, 0, 0, 1, 0, 0, 1, 0, 0],
        true,
    )?;
    Ok(())
}

fn pcg_test_multi_capture_overlap() -> Result<()> {
    test_patterns(
        "foobarbaz",
        &["(.(.(.)))"],
        &["foo", "oo", "o", "bar", "ar", "r", "baz", "az", "z"],
        &[0, 0, 0, 0, 0, 0, 0, 0, 0],
        &[9, 9, 9, 9, 9, 9, 9, 9, 9],
        &[1, 0, 0, 0, 0, 0, 0, 0, 0],
        false,
    )?;
    test_patterns(
        "foobarbaz",
        &["(.(.(.)))"],
        &[
            "foobarbaz",
            "foo",
            "oo",
            "o",
            "bar",
            "ar",
            "r",
            "baz",
            "az",
            "z",
        ],
        &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        &[9, 9, 9, 9, 9, 9, 9, 9, 9, 9],
        &[1, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        true,
    )?;
    test_patterns(
        "foo bar baz",
        &["(.(.(.)))"],
        &["foo", "oo", "o", "bar", "ar", "r", "baz", "az", "z"],
        &[0, 0, 0, 4, 4, 4, 8, 8, 8],
        &[3, 3, 3, 7, 7, 7, 11, 11, 11],
        &[1, 0, 0, 1, 0, 0, 1, 0, 0],
        false,
    )?;
    test_patterns(
        "foo bar baz",
        &["(.(.(.)))"],
        &["foo", "oo", "o", "bar", "ar", "r", "baz", "az", "z"],
        &[0, 0, 0, 4, 4, 4, 8, 8, 8],
        &[3, 3, 3, 7, 7, 7, 11, 11, 11],
        &[1, 0, 0, 1, 0, 0, 1, 0, 0],
        true,
    )?;
    Ok(())
}

fn pcg_test_multi_pattern() -> Result<()> {
    test_patterns(
        "aaabbbaaa",
        &["(aaa)", "(bbb)", "(ccc)"],
        &["aaa", "bbb", "aaa"],
        &[0, 0, 0],
        &[9, 9, 9],
        &[1, 0, 0],
        false,
    )?;
    test_patterns(
        "aaabbbaaa",
        &["(aaa)", "(bbb)", "(ccc)"],
        &["aaabbbaaa", "aaa", "bbb", "aaa"],
        &[0, 0, 0, 0],
        &[9, 9, 9, 9],
        &[1, 0, 0, 0],
        true,
    )?;
    test_patterns(
        "aaa bbb aaa",
        &["(aaa)", "(bbb)", "(ccc)"],
        &["aaa", "bbb", "aaa"],
        &[0, 4, 8],
        &[3, 7, 11],
        &[1, 1, 1],
        false,
    )?;
    test_patterns(
        "aaa bbb aaa",
        &["(aaa)", "(bbb)", "(ccc)"],
        &["aaa", "bbb", "aaa"],
        &[0, 4, 8],
        &[3, 7, 11],
        &[1, 1, 1],
        true,
    )?;
    Ok(())
}

fn pcg_test_camel_case() -> Result<()> {
    test_patterns(
        "letsPartyLIKEits1999_dude",
        &[
            "([A-Z]{2,})",
            "(?<![A-Z])([A-Z][a-z]+)",
            "(?:^|\\b|(?<=[0-9_])|(?<=[A-Z]{2}))([a-z]+)",
            "([0-9]+)",
        ],
        &["lets", "Party", "LIKE", "its", "1999", "dude"],
        &[0, 0, 0, 0, 0, 0],
        &[25, 25, 25, 25, 25, 25],
        &[1, 0, 0, 0, 0, 0, 0],
        false,
    )?;
    test_patterns(
        "letsPartyLIKEits1999_dude",
        &[
            "([A-Z]{2,})",
            "(?<![A-Z])([A-Z][a-z]+)",
            "(?:^|\\b|(?<=[0-9_])|(?<=[A-Z]{2}))([a-z]+)",
            "([0-9]+)",
        ],
        &[
            "letsPartyLIKEits1999_dude",
            "lets",
            "Party",
            "LIKE",
            "its",
            "1999",
            "dude",
        ],
        &[0, 0, 0, 0, 0, 0, 0],
        &[25, 25, 25, 25, 25, 25, 25],
        &[1, 0, 0, 0, 0, 0, 0, 0],
        true,
    )?;
    Ok(())
}

fn pcg_test_random_string() -> Result<()> {
    let a = whitespace_analyzer(
        "org.apache.lucene.analysis.pattern.TestPatternCaptureGroupTokenFilter$1",
        |t| {
            Ok(boxed(new_pattern_capture_group_token_filter(
                t,
                false,
                &[pattern("((..)(..))")],
            )?))
        },
    );
    B::check_random_data(&*random(), &a, 1000 * mult())?;
    a.close();
    Ok(())
}
fn pattern_capture_group_token_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.pattern.TestPatternCaptureGroupTokenFilter")
        .method(t("testNoPattern", pcg_test_no_pattern))
        .method(t("testNoMatch", pcg_test_no_match))
        .method(t("testNoCapture", pcg_test_no_capture))
        .method(t("testEmptyCapture", pcg_test_empty_capture))
        .method(t("testCaptureAll", pcg_test_capture_all))
        .method(t("testCaptureStart", pcg_test_capture_start))
        .method(t("testCaptureMiddle", pcg_test_capture_middle))
        .method(t("testCaptureEnd", pcg_test_capture_end))
        .method(t("testCaptureStartMiddle", pcg_test_capture_start_middle))
        .method(t("testCaptureStartEnd", pcg_test_capture_start_end))
        .method(t("testCaptureMiddleEnd", pcg_test_capture_middle_end))
        .method(t("testMultiCaptureOverlap", pcg_test_multi_capture_overlap))
        .method(t("testMultiPattern", pcg_test_multi_pattern))
        .method(t("testCamelCase", pcg_test_camel_case))
        .method(t("testRandomString", pcg_test_random_string))
        .build()
}

// -----------------------------------------------------------------------------
// TestPatternReplaceCharFilter
// -----------------------------------------------------------------------------

/// `checkOutput(input, pattern, replacement, expectedOutput,
/// expectedIndexMatchedOutput)`.
fn check_output(
    input: &str,
    pattern_str: &str,
    replacement: &str,
    expected_output: &str,
    expected_index_matched_output: &str,
) -> Result<()> {
    let input_units: Vec<u16> = input.encode_utf16().collect();
    let mut cs = PatternReplaceCharFilter::new(pattern(pattern_str), replacement, reader(input));
    let mut output: Vec<u16> = Vec::new();
    // `for (int chr = cs.read(); chr > 0; chr = cs.read())`
    let mut one = [0u16; 1];
    while cs.read(&mut one)? == 1 && one[0] > 0 {
        output.push(one[0]);
    }
    let mut index_matched: Vec<u16> = Vec::new();
    for i in 0..output.len() {
        let off = cs.correct_offset(i as i32);
        if off < 0 {
            index_matched.push('-' as u16);
        } else {
            index_matched.push(input_units[off as usize]);
        }
    }
    let output_good = expected_output.encode_utf16().eq(output.iter().copied());
    let index_matched_good = expected_index_matched_output
        .encode_utf16()
        .eq(index_matched.iter().copied());
    if !output_good || !index_matched_good {
        println!("Pattern : {pattern_str}");
        println!("Replac. : {replacement}");
        println!("Input   : {input}");
        println!("Output  : {}", String::from_utf16_lossy(&output));
        println!("Expected: {expected_output}");
        println!("Output/i: {}", String::from_utf16_lossy(&index_matched));
        println!("Expected: {expected_index_matched_output}");
        println!();
    }
    Assert::assert_true_msg("Output doesn't match.", output_good);
    Assert::assert_true_msg("Index-matched output doesn't match.", index_matched_good);
    Ok(())
}

/// `assertTokenStreamContents(whitespaceMockTokenizer(cs), output, start, end, finalOffset)`.
fn assert_replaced(
    cs: Box<dyn CharReader>,
    output: &[&str],
    start: &[i32],
    end: &[i32],
    final_offset: i32,
) -> Result<()> {
    let mut ts = B::whitespace_mock_tokenizer(cs)?;
    B::assert_token_stream_contents_offsets_final_offset(
        &mut ts,
        output,
        start,
        end,
        Some(final_offset),
    )
}

fn prcf(p: &str, replacement: &str, input: Box<dyn CharReader>) -> Box<dyn CharReader> {
    Box::new(PatternReplaceCharFilter::new(
        pattern(p),
        replacement,
        input,
    ))
}

fn prcf_test_failing_dot() -> Result<()> {
    check_output("A. .B.", "\\.[\\s]*", ".", "A..B.", "A..B.")?;
    Ok(())
}

fn prcf_test_longer_replacement() -> Result<()> {
    check_output(
        "XXabcZZabcYY",
        "abc",
        "abcde",
        "XXabcdeZZabcdeYY",
        "XXabcccZZabcccYY",
    )?;
    check_output(
        "XXabcabcYY",
        "abc",
        "abcde",
        "XXabcdeabcdeYY",
        "XXabcccabcccYY",
    )?;
    check_output("abcabcYY", "abc", "abcde", "abcdeabcdeYY", "abcccabcccYY")?;
    // Should be: "-----YY" but we're enforcing non-negative offsets.
    check_output("YY", "^", "abcde", "abcdeYY", "YYYYYYY")?;
    check_output("YY", "$", "abcde", "YYabcde", "YYYYYYY")?;
    check_output("XYZ", ".", "abc", "abcabcabc", "XXXYYYZZZ")?;
    check_output("XYZ", ".", "$0abc", "XabcYabcZabc", "XXXXYYYYZZZZ")?;
    Ok(())
}

fn prcf_test_shorter_replacement() -> Result<()> {
    check_output("XXabcZZabcYY", "abc", "xy", "XXxyZZxyYY", "XXabZZabYY")?;
    check_output("XXabcabcYY", "abc", "xy", "XXxyxyYY", "XXababYY")?;
    check_output("abcabcYY", "abc", "xy", "xyxyYY", "ababYY")?;
    check_output("abcabcYY", "abc", "", "YY", "YY")?;
    check_output("YYabcabc", "abc", "", "YY", "YY")?;
    Ok(())
}

fn prcf_test_nothing_change() -> Result<()> {
    let block = "this is test.";
    assert_replaced(
        prcf("(aa)\\s+(bb)\\s+(cc)", "$1$2$3", reader(block)),
        &["this", "is", "test."],
        &[0, 5, 8],
        &[4, 7, 13],
        block.len() as i32,
    )
}

fn prcf_test_replace_by_empty() -> Result<()> {
    let block = "aa bb cc";
    let mut ts = B::whitespace_mock_tokenizer(prcf("(aa)\\s+(bb)\\s+(cc)", "", reader(block)))?;
    B::assert_token_stream_contents(&mut ts, &[])
}

fn prcf_test1block1match_same_length() -> Result<()> {
    let block = "aa bb cc";
    assert_replaced(
        prcf("(aa)\\s+(bb)\\s+(cc)", "$1#$2#$3", reader(block)),
        &["aa#bb#cc"],
        &[0],
        &[8],
        block.len() as i32,
    )
}

fn prcf_test1block1match_longer() -> Result<()> {
    let block = "aa bb cc dd";
    assert_replaced(
        prcf("(aa)\\s+(bb)\\s+(cc)", "$1##$2###$3", reader(block)),
        &["aa##bb###cc", "dd"],
        &[0, 9],
        &[8, 11],
        block.len() as i32,
    )
}

fn prcf_test1block2match_longer() -> Result<()> {
    let block = " a  a";
    assert_replaced(
        prcf("a", "aa", reader(block)),
        &["aa", "aa"],
        &[1, 4],
        &[2, 5],
        block.len() as i32,
    )
}

fn prcf_test1block1match_shorter() -> Result<()> {
    let block = "aa  bb   cc dd";
    assert_replaced(
        prcf("(aa)\\s+(bb)\\s+(cc)", "$1#$2", reader(block)),
        &["aa#bb", "dd"],
        &[0, 12],
        &[11, 14],
        block.len() as i32,
    )
}

fn prcf_test1block_multi_matches() -> Result<()> {
    let block = "  aa bb cc --- aa bb aa   bb   cc";
    assert_replaced(
        prcf("(aa)\\s+(bb)\\s+(cc)", "$1  $2  $3", reader(block)),
        &["aa", "bb", "cc", "---", "aa", "bb", "aa", "bb", "cc"],
        &[2, 6, 9, 11, 15, 18, 21, 25, 29],
        &[4, 8, 10, 14, 17, 20, 23, 27, 33],
        block.len() as i32,
    )
}

fn prcf_test2blocks_multi_matches() -> Result<()> {
    let block = "  aa bb cc --- aa bb aa. bb aa   bb cc";
    assert_replaced(
        prcf("(aa)\\s+(bb)", "$1##$2", reader(block)),
        &["aa##bb", "cc", "---", "aa##bb", "aa.", "bb", "aa##bb", "cc"],
        &[2, 8, 11, 15, 21, 25, 28, 36],
        &[7, 10, 14, 20, 24, 27, 35, 38],
        block.len() as i32,
    )
}

fn prcf_test_chain() -> Result<()> {
    let block = " a bb - ccc . --- bb a . ccc ccc bb";
    let cs = prcf("a", "aa", reader(block));
    let cs = prcf("bb", "b", cs);
    let cs = prcf("ccc", "c", cs);
    assert_replaced(
        cs,
        &[
            "aa", "b", "-", "c", ".", "---", "b", "aa", ".", "c", "c", "b",
        ],
        &[1, 3, 6, 8, 12, 14, 18, 21, 23, 25, 29, 33],
        &[2, 5, 7, 11, 13, 17, 20, 22, 24, 28, 32, 35],
        block.len() as i32,
    )
}

fn prcf_test_random_strings() -> Result<()> {
    let num_patterns = 5;
    let random = JdkRandom::new(random().next_long());
    for _ in 0..num_patterns {
        let p = pattern(&TestUtil::random_pattern(&*LuceneTestCase::random()));
        let replacement = TestUtil::random_simple_string(&random);
        let a = FnAnalyzer::new(
            "org.apache.lucene.analysis.pattern.TestPatternReplaceCharFilter$1",
            |_| {
                Ok(tokenizer_components(own(MockTokenizer::new(
                    Arc::clone(&WHITESPACE),
                    false,
                )?)))
            },
        )
        .init_reader(move |_, r| {
            Box::new(PatternReplaceCharFilter::new(
                p.clone(),
                replacement.clone(),
                r,
            ))
        })
        .arc();
        // max input length. don't make it longer -- exponential processing
        // time for certain patterns.
        let max_input_length = 30;
        // ASCII only input?:
        let ascii_only = true;
        B::check_random_data_max_word_length_simple(
            &random,
            &a,
            50 * mult(),
            max_input_length,
            ascii_only,
        )?;
        a.close();
    }
    Ok(())
}
fn pattern_replace_char_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.pattern.TestPatternReplaceCharFilter")
        .method(t("testFailingDot", prcf_test_failing_dot))
        .method(t("testLongerReplacement", prcf_test_longer_replacement))
        .method(t("testShorterReplacement", prcf_test_shorter_replacement))
        .method(t("testNothingChange", prcf_test_nothing_change))
        .method(t("testReplaceByEmpty", prcf_test_replace_by_empty))
        .method(t(
            "test1block1matchSameLength",
            prcf_test1block1match_same_length,
        ))
        .method(t("test1block1matchLonger", prcf_test1block1match_longer))
        .method(t("test1block2matchLonger", prcf_test1block2match_longer))
        .method(t("test1block1matchShorter", prcf_test1block1match_shorter))
        .method(t("test1blockMultiMatches", prcf_test1block_multi_matches))
        .method(t("test2blocksMultiMatches", prcf_test2blocks_multi_matches))
        .method(t("testChain", prcf_test_chain))
        .method(t("testRandomStrings", prcf_test_random_strings))
        .build()
}

// -----------------------------------------------------------------------------
// TestPatternReplaceCharFilterFactory
// -----------------------------------------------------------------------------

/// `charFilterFactory("PatternReplace", params).create(new StringReader(text))`
/// under a whitespace `MockTokenizer`.
fn prcff_stream(params: &[&str], text: &str) -> Result<MockTokenizer> {
    let r = F::char_filter_factory("PatternReplace", params)?.create(reader(text))?;
    B::whitespace_mock_tokenizer(r)
}

//           1111
// 01234567890123
// this is test.
fn prcff_test_nothing_change() -> Result<()> {
    let mut ts = prcff_stream(
        &["pattern", "(aa)\\s+(bb)\\s+(cc)", "replacement", "$1$2$3"],
        "this is test.",
    )?;
    B::assert_token_stream_contents_offsets(
        &mut ts,
        &["this", "is", "test."],
        &[0, 5, 8],
        &[4, 7, 13],
    )
}

// 012345678
// aa bb cc
fn prcff_test_replace_by_empty() -> Result<()> {
    let mut ts = prcff_stream(&["pattern", "(aa)\\s+(bb)\\s+(cc)"], "aa bb cc")?;
    B::assert_token_stream_contents(&mut ts, &[])
}

// 012345678
// aa bb cc
// aa#bb#cc
fn prcff_test1block1match_same_length() -> Result<()> {
    let mut ts = prcff_stream(
        &["pattern", "(aa)\\s+(bb)\\s+(cc)", "replacement", "$1#$2#$3"],
        "aa bb cc",
    )?;
    B::assert_token_stream_contents_offsets(&mut ts, &["aa#bb#cc"], &[0], &[8])
}

/// Test that bogus arguments result in exception
fn prcff_test_bogus_arguments() -> Result<()> {
    let expected = iae_message(|| {
        F::char_filter_factory(
            "PatternReplace",
            &["pattern", "something", "bogusArg", "bogusValue"],
        )
    });
    Assert::assert_true(expected.contains("Unknown parameters"));
    Ok(())
}

/// Test with backslash unescape
fn prcff_test_unescape() -> Result<()> {
    let mut ts = prcff_stream(
        &["pattern", "\\\\(.)", "replacement", "$1"],
        "aaa\\ bbb\\-ccc",
    )?;
    B::assert_token_stream_contents_offsets(&mut ts, &["aaa", "bbb-ccc"], &[0, 5], &[3, 13])?;
    let mut ts = prcff_stream(
        &[
            "pattern",
            "\\\\([^\\p{IsAlphabetic}\\p{Digit}])",
            "replacement",
            "$1",
        ],
        "a\\b\\0\\-c\\\u{E9}\\ d",
    )?;
    B::assert_token_stream_contents_offsets(
        &mut ts,
        &["a\\b\\0-c\\\u{E9}", "d"],
        &[0, 12],
        &[10, 13],
    )
}

fn pattern_replace_char_filter_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.pattern.TestPatternReplaceCharFilterFactory")
        .method(t("testNothingChange", prcff_test_nothing_change))
        .method(t("testReplaceByEmpty", prcff_test_replace_by_empty))
        .method(t(
            "test1block1matchSameLength",
            prcff_test1block1match_same_length,
        ))
        .method(t("testBogusArguments", prcff_test_bogus_arguments))
        .method(t("testUnescape", prcff_test_unescape))
        .build()
}

// -----------------------------------------------------------------------------
// TestPatternReplaceFilter
// -----------------------------------------------------------------------------

const PRF_INPUT: &str = "aabfooaabfooabfoob ab caaaaaaaaab";

/// `new PatternReplaceFilter(whitespaceMockTokenizer(input), Pattern.compile(p), replacement, all)`.
fn prf(p: &str, replacement: Option<&str>, all: bool) -> Result<Box<dyn TokenStream>> {
    Ok(boxed(new_pattern_replace_filter(
        boxed(B::whitespace_mock_tokenizer_str(PRF_INPUT)?),
        &pattern(p),
        replacement,
        all,
    )?))
}

fn prf_test_replace_all() -> Result<()> {
    B::assert_token_stream_contents(
        &mut *prf("a*b", Some("-"), true)?,
        &["-foo-foo-foo-", "-", "c-"],
    )
}

fn prf_test_replace_first() -> Result<()> {
    B::assert_token_stream_contents(
        &mut *prf("a*b", Some("-"), false)?,
        &["-fooaabfooabfoob", "-", "c-"],
    )
}

fn prf_test_strip_first() -> Result<()> {
    B::assert_token_stream_contents(
        &mut *prf("a*b", None, false)?,
        &["fooaabfooabfoob", "", "c"],
    )
}

fn prf_test_strip_all() -> Result<()> {
    B::assert_token_stream_contents(&mut *prf("a*b", None, true)?, &["foofoofoo", "", "c"])
}

fn prf_test_replace_all_with_back_ref() -> Result<()> {
    B::assert_token_stream_contents(
        &mut *prf("(a*)b", Some("$1\\$"), true)?,
        &["aa$fooaa$fooa$foo$", "a$", "caaaaaaaaa$"],
    )
}

/// blast some random strings through the analyzer
fn prf_test_random_strings() -> Result<()> {
    let a = whitespace_analyzer(
        "org.apache.lucene.analysis.pattern.TestPatternReplaceFilter$1",
        |t| {
            Ok(boxed(new_pattern_replace_filter(
                t,
                &pattern("a"),
                Some("b"),
                false,
            )?))
        },
    );
    B::check_random_data(&*random(), &a, 200 * mult())?;
    a.close();
    let b = whitespace_analyzer(
        "org.apache.lucene.analysis.pattern.TestPatternReplaceFilter$2",
        |t| {
            Ok(boxed(new_pattern_replace_filter(
                t,
                &pattern("a"),
                Some("b"),
                true,
            )?))
        },
    );
    B::check_random_data(&*random(), &b, 200 * mult())?;
    b.close();
    Ok(())
}

fn prf_test_empty_term() -> Result<()> {
    let a = keyword_tokenizer_analyzer(
        "org.apache.lucene.analysis.pattern.TestPatternReplaceFilter$3",
        |t| {
            Ok(boxed(new_pattern_replace_filter(
                t,
                &pattern("a"),
                Some("b"),
                true,
            )?))
        },
    );
    B::check_one_term(&*a, "", "")?;
    a.close();
    Ok(())
}

fn pattern_replace_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.pattern.TestPatternReplaceFilter")
        .method(t("testReplaceAll", prf_test_replace_all))
        .method(t("testReplaceFirst", prf_test_replace_first))
        .method(t("testStripFirst", prf_test_strip_first))
        .method(t("testStripAll", prf_test_strip_all))
        .method(t(
            "testReplaceAllWithBackRef",
            prf_test_replace_all_with_back_ref,
        ))
        .method(t("testRandomStrings", prf_test_random_strings))
        .method(t("testEmptyTerm", prf_test_empty_term))
        .build()
}

// -----------------------------------------------------------------------------
// TestPatternReplaceFilterFactory
// -----------------------------------------------------------------------------

fn prff_test_replace_all() -> Result<()> {
    let stream = boxed(B::whitespace_mock_tokenizer(reader(PRF_INPUT))?);
    let mut stream =
        F::token_filter_factory("PatternReplace", &["pattern", "a*b", "replacement", "-"])?
            .create(stream)?;
    B::assert_token_stream_contents(&mut *stream, &["-foo-foo-foo-", "-", "c-"])
}

/// Test that bogus arguments result in exception
fn prff_test_bogus_arguments() -> Result<()> {
    let expected = iae_message(|| {
        F::token_filter_factory(
            "PatternReplace",
            &["pattern", "something", "bogusArg", "bogusValue"],
        )
    });
    Assert::assert_true(expected.contains("Unknown parameters"));
    Ok(())
}

fn pattern_replace_filter_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.pattern.TestPatternReplaceFilterFactory")
        .method(t("testReplaceAll", prff_test_replace_all))
        .method(t("testBogusArguments", prff_test_bogus_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// TestPatternTokenizer
// -----------------------------------------------------------------------------

fn pt_test_splitting() -> Result<()> {
    let qpattern = "\\'([^\\']+)\\'"; // get stuff between "'"
    let tests: [[&str; 4]; 8] = [
        // group  pattern        input                    output
        ["-1", "--", "aaa--bbb--ccc", "aaa bbb ccc"],
        ["-1", ":", "aaa:bbb:ccc", "aaa bbb ccc"],
        ["-1", "\\p{Space}", "aaa   bbb \t\tccc  ", "aaa bbb ccc"],
        ["-1", ":", "boo:and:foo", "boo and foo"],
        ["-1", "o", "boo:and:foo", "b :and:f"],
        ["0", ":", "boo:and:foo", ": :"],
        ["0", qpattern, "aaa 'bbb' 'ccc'", "'bbb' 'ccc'"],
        ["1", qpattern, "aaa 'bbb' 'ccc'", "bbb ccc"],
    ];
    for test in tests {
        let mut stream = new_pattern_tokenizer_with_factory(
            B::new_attribute_factory(),
            &pattern(test[1]),
            test[0].parse::<i32>().expect("an int"),
        )?;
        stream.set_reader(reader(test[2]))?;
        let out = ts_to_string(&mut stream)?;
        Assert::assert_equals_msg(
            Some(&format!("pattern: {} with input: {}", test[1], test[2])),
            test[3],
            out.as_str(),
        );
    }
    Ok(())
}

fn pt_test_offset_correction() -> Result<()> {
    let mut stream =
        new_pattern_tokenizer_with_factory(B::new_attribute_factory(), &pattern("[,;/\\s]+"), -1)?;
    stream.set_reader(gunther_char_stream()?)?;
    B::assert_token_stream_contents_offsets_final_offset(
        &mut stream,
        &["G\u{FC}nther", "G\u{FC}nther", "is", "here"],
        &[0, 13, 26, 29],
        &[12, 25, 28, 33],
        Some(GUNTHER.len() as i32),
    )?;
    let mut stream = new_pattern_tokenizer_with_factory(
        B::new_attribute_factory(),
        &pattern("G\u{FC}nther"),
        0,
    )?;
    stream.set_reader(gunther_char_stream()?)?;
    B::assert_token_stream_contents_offsets_final_offset(
        &mut stream,
        &["G\u{FC}nther", "G\u{FC}nther"],
        &[0, 13],
        &[12, 25],
        Some(GUNTHER.len() as i32),
    )
}

/// `new Analyzer() { new PatternTokenizer(newAttributeFactory(), Pattern.compile(p), group) }`.
fn pattern_tokenizer_analyzer(
    name: &'static str,
    p: &'static str,
    group: i32,
) -> Arc<dyn Analyzer> {
    FnAnalyzer::new(name, move |_| {
        Ok(tokenizer_components(own(
            new_pattern_tokenizer_with_factory(B::new_attribute_factory(), &pattern(p), group)?,
        )))
    })
    .arc()
}

/// blast some random strings through the analyzer
fn pt_test_random_strings() -> Result<()> {
    let a = pattern_tokenizer_analyzer(
        "org.apache.lucene.analysis.pattern.TestPatternTokenizer$1",
        "a",
        -1,
    );
    B::check_random_data(&*random(), &a, 200 * mult())?;
    a.close();
    let b = pattern_tokenizer_analyzer(
        "org.apache.lucene.analysis.pattern.TestPatternTokenizer$2",
        "a",
        0,
    );
    B::check_random_data(&*random(), &b, 200 * mult())?;
    b.close();
    Ok(())
}

// LUCENE-6814
fn pt_test_heap_freed_after_close() -> Result<()> {
    // TODO: can we move this to BaseTSTC to catch other "hangs onto heap"ers?
    // Build a 1MB string:
    let mut b = String::with_capacity(1024 * 1024);
    for _ in 0..1024 {
        // 1023 spaces, then an x
        b.push_str(&" ".repeat(1023));
        b.push('x');
    }
    let big = b;
    let x = pattern("x");
    let mut tokenizers = Vec::new();
    for _ in 0..512 {
        let mut stream = new_pattern_tokenizer(&x, -1)?;
        stream.set_reader(reader(&big))?;
        stream.reset()?;
        for _ in 0..1024 {
            Assert::assert_true(stream.increment_token()?);
        }
        Assert::assert_false(stream.increment_token()?);
        stream.end()?;
        stream.close()?;
        tokenizers.push(stream);
    }
    Ok(())
}

fn pattern_tokenizer_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.pattern.TestPatternTokenizer")
        .method(t("testSplitting", pt_test_splitting))
        .method(t("testOffsetCorrection", pt_test_offset_correction))
        .method(t("testRandomStrings", pt_test_random_strings))
        .build()
}

/// The class with only its `@Nightly` method, `testHeapFreedAfterClose`
/// (unannotated: see the module docs).
fn pattern_tokenizer_nightly_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.pattern.TestPatternTokenizer")
        .method(t("testHeapFreedAfterClose", pt_test_heap_freed_after_close))
        .build()
}

// -----------------------------------------------------------------------------
// TestPatternTokenizerFactory
// -----------------------------------------------------------------------------

fn ptf_test_factory() -> Result<()> {
    // create PatternTokenizer
    let mut stream = F::tokenizer_factory("Pattern", &["pattern", "[,;/\\s]+"])?
        .create_with_factory(B::new_attribute_factory())?;
    stream.set_reader(reader("G\u{FC}nther G\u{FC}nther is here"))?;
    B::assert_token_stream_contents(&mut stream, &["G\u{FC}nther", "G\u{FC}nther", "is", "here"])
}

/// Test that bogus arguments result in exception
fn ptf_test_bogus_arguments() -> Result<()> {
    let expected = iae_message(|| {
        F::tokenizer_factory(
            "Pattern",
            &["pattern", "something", "bogusArg", "bogusValue"],
        )
    });
    Assert::assert_true(expected.contains("Unknown parameters"));
    Ok(())
}

fn pattern_tokenizer_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.pattern.TestPatternTokenizerFactory")
        .method(t("testFactory", ptf_test_factory))
        .method(t("testBogusArguments", ptf_test_bogus_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// TestPatternTypingFilter
// -----------------------------------------------------------------------------

/// `assertTokenStreamContents(ts, output, null, null, types, null, null,
/// null, null, null, false, null, flags)`.
fn assert_types_flags(
    ts: &mut dyn TokenStream,
    output: &[&str],
    types: &[&str],
    flags: &[i32],
) -> Result<()> {
    B::assert_token_stream_contents_flags(
        ts,
        output,
        None,
        None,
        Some(types),
        None,
        None,
        None,
        None,
        None,
        false,
        None,
        Some(flags),
    )
}

/// Test the straight forward cases. When all flags match the token should be dropped
fn ptyf_test_patterns() -> Result<()> {
    let ts = CannedTokenStream::new(vec![
        Token::with_offsets("One", 0, 2),
        Token::with_offsets("401(k)", 4, 9),
        Token::with_offsets("two", 11, 13),
        Token::with_offsets("three", 15, 19),
        Token::with_offsets("401k", 21, 24),
    ]);
    // 2 ^(\d+)\(?([a-z])\)?$ ::: legal2_$1_$2
    let mut ts = new_pattern_typing_filter(
        boxed(ts),
        &[PatternTypingRule::new(
            pattern("^(\\d+)\\(?([a-z])\\)?$"),
            2,
            "legal2_$1_$2",
        )],
    )?;
    assert_types_flags(
        &mut ts,
        &["One", "401(k)", "two", "three", "401k"],
        &["word", "legal2_401_k", "word", "word", "legal2_401_k"],
        &[0, 2, 0, 0, 2],
    )
}

fn ptyf_test_first_pattern_wins() -> Result<()> {
    let ts = CannedTokenStream::new(vec![
        Token::with_offsets("One", 0, 2),
        Token::with_offsets("forty-two", 11, 13),
        Token::with_offsets("4-2", 15, 19),
    ]);
    // 2 ^(\d+)\(?([a-z])\)?$ ::: legal2_$1_$2
    let p1 = PatternTypingRule::new(pattern("^(\\d+)-(\\d+)$"), 6, "$1_hnum_$2");
    let p2 = PatternTypingRule::new(pattern("^(\\w+)-(\\w+)$"), 2, "$1_hword_$2");
    let mut ts = new_pattern_typing_filter(boxed(ts), &[p1, p2])?; // 101
    assert_types_flags(
        &mut ts,
        &["One", "forty-two", "4-2"],
        &["word", "forty_hword_two", "4_hnum_2"],
        &[0, 2, 6],
    )
}

fn pattern_typing_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.pattern.TestPatternTypingFilter")
        .method(t("testPatterns", ptyf_test_patterns))
        .method(t("testFirstPatternWins", ptyf_test_first_pattern_wins))
        .build()
}

// -----------------------------------------------------------------------------
// TestPatternTypingFilterFactory
// -----------------------------------------------------------------------------

fn ptyff_test_factory() -> Result<()> {
    let ts = CannedTokenStream::new(vec![
        Token::with_offsets("One", 0, 2),
        Token::with_offsets("forty-two", 11, 13),
        Token::with_offsets("4-2", 15, 19),
    ]);
    let token_filter_factory = F::token_filter_factory_with_loader(
        "patternTyping",
        Some(Version::LATEST),
        &StringMockResourceLoader::new(
            "6 \\b(\\d+)-(\\d+) ::: $1_hnum_$2\n2 \\b(\\w+)-(\\w+) ::: $1_hword_$2",
        ),
        &["patternFile", "patterns.txt"],
    )?;
    let mut ts = token_filter_factory.create(boxed(ts))?;
    assert_types_flags(
        &mut *ts,
        &["One", "forty-two", "4-2"],
        &["word", "forty_hword_two", "4_hnum_2"],
        &[0, 2, 6],
    )
}

fn pattern_typing_filter_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.pattern.TestPatternTypingFilterFactory")
        .method(t("testFactory", ptyff_test_factory))
        .build()
}

// -----------------------------------------------------------------------------
// TestSimplePatternSplitTokenizer and TestSimplePatternTokenizer
// -----------------------------------------------------------------------------

/// The automaton of `testNotDeterminized`.
fn not_determinized() -> Automaton {
    let mut a = Automaton::new();
    let start = a.create_state();
    let mid1 = a.create_state();
    let mid2 = a.create_state();
    let end = a.create_state();
    a.set_accept(end, true);
    a.add_transition_range(start, mid1, 'a' as i32, 'z' as i32);
    a.add_transition_range(start, mid2, 'a' as i32, 'z' as i32);
    a.add_transition(mid1, end, 'b' as i32);
    a.add_transition(mid2, end, 'b' as i32);
    a
}

fn split(regexp: &str, text: &str) -> Result<rucene_analysis_common::analysis::pattern::simple_pattern_split_tokenizer::SimplePatternSplitTokenizer>{
    let mut t = new_simple_pattern_split_tokenizer(regexp)?;
    t.set_reader(reader(text))?;
    Ok(t)
}

fn simple(
    regexp: &str,
    text: &str,
) -> Result<
    rucene_analysis_common::analysis::pattern::simple_pattern_tokenizer::SimplePatternTokenizer,
> {
    let mut t = new_simple_pattern_tokenizer(regexp)?;
    t.set_reader(reader(text))?;
    Ok(t)
}

fn spst_test_greedy() -> Result<()> {
    let mut t = split("(foo)+", "bar foofoo baz")?;
    B::assert_token_stream_contents_offsets(&mut t, &["bar ", " baz"], &[0, 10], &[4, 14])
}

fn spst_test_back_to_back() -> Result<()> {
    let mut t = split("foo", "bar foofoo baz")?;
    B::assert_token_stream_contents_offsets(&mut t, &["bar ", " baz"], &[0, 10], &[4, 14])
}

fn spst_test_big_lookahead() -> Result<()> {
    let mut t = new_simple_pattern_split_tokenizer(&format!("{}b", "a".repeat(100)))?;
    let b = "a".repeat(200);
    t.set_reader(reader(&b))?;
    t.reset()?;
    Assert::assert_true(t.increment_token()?);
    Assert::assert_equals(b.as_str(), term_of(&t).as_str());
    Assert::assert_false(t.increment_token()?);
    Ok(())
}

/// A non-empty `TestUtil.randomUnicodeString(random())`.
fn non_empty_random_unicode_string() -> String {
    loop {
        let s = TestUtil::random_unicode_string(&*random());
        if !s.is_empty() {
            return s;
        }
    }
}

fn spst_test_no_tokens() -> Result<()> {
    let s = non_empty_random_unicode_string();
    let mut t = split(".*", &s)?;
    t.reset()?;
    Assert::assert_false(t.increment_token()?);
    Ok(())
}

fn spst_test_empty_string_pattern_no_match() -> Result<()> {
    let mut t = split("a*", "bbb")?;
    t.reset()?;
    Assert::assert_true(t.increment_token()?);
    Assert::assert_equals("bbb", term_of(&t).as_str());
    Assert::assert_false(t.increment_token()?);
    Ok(())
}

fn spst_test_split_single_char_whitespace() -> Result<()> {
    let mut t = split("[ \t\r\n]", "a \tb   c")?;
    B::assert_token_stream_contents_offsets(&mut t, &["a", "b", "c"], &[0, 3, 7], &[1, 4, 8])
}

fn spst_test_split_multi_char_whitespace() -> Result<()> {
    let mut t = split("[ \t\r\n]*", "a \tb   c")?;
    B::assert_token_stream_contents_offsets(&mut t, &["a", "b", "c"], &[0, 3, 7], &[1, 4, 8])
}

fn spst_test_leading_non_token() -> Result<()> {
    let mut t = split("[ \t\r\n]*", "    a c")?;
    B::assert_token_stream_contents_offsets(&mut t, &["a", "c"], &[4, 6], &[5, 7])
}

fn spst_test_trailing_non_token() -> Result<()> {
    let mut t = split("[ \t\r\n]*", "a c   ")?;
    B::assert_token_stream_contents_offsets(&mut t, &["a", "c"], &[0, 2], &[1, 3])
}

fn spst_test_empty_string_pattern_one_match() -> Result<()> {
    let mut t = split("a*", "bbab")?;
    B::assert_token_stream_contents_offsets(&mut t, &["bb", "b"], &[0, 3], &[2, 4])
}

/// `offsetAtt.endOffset()`.
fn end_offset(ts: &dyn TokenStream) -> i32 {
    ts.attribute_source()
        .get_attribute_interface::<dyn OffsetAttribute>()
        .expect("offsetAtt")
        .end_offset()
}

fn spst_test_end_offset() -> Result<()> {
    let mut t = split("a+", "aaabbb")?;
    t.reset()?;
    Assert::assert_true(t.increment_token()?);
    Assert::assert_equals("bbb", term_of(&t).as_str());
    Assert::assert_false(t.increment_token()?);
    t.end()?;
    Assert::assert_equals(6, end_offset(&t));
    Ok(())
}

fn spst_test_fixed_token() -> Result<()> {
    let mut t = split("aaaa", "aaaaaaaaaaaaaaa")?;
    B::assert_token_stream_contents_offsets(&mut t, &["aaa"], &[12], &[15])
}

fn spst_test_basic() -> Result<()> {
    let tests: [[&str; 3]; 4] = [
        // pattern        input                    output
        ["--", "aaa--bbb--ccc", "aaa bbb ccc"],
        [":", "aaa:bbb:ccc", "aaa bbb ccc"],
        [":", "boo:and:foo", "boo and foo"],
        ["o", "boo:and:foo", "b :and:f"],
    ];
    for test in tests {
        let mut stream = split(test[0], test[1])?;
        let out = ts_to_string(&mut stream)?;
        Assert::assert_equals_msg(
            Some(&format!("pattern: {} with input: {}", test[0], test[1])),
            test[2],
            out.as_str(),
        );
    }
    Ok(())
}

fn spst_test_not_determinized() -> Result<()> {
    let a = not_determinized();
    expect_throws("java.lang.IllegalArgumentException", || {
        new_simple_pattern_split_tokenizer_from_automaton(a)
    });
    Ok(())
}

fn spst_test_offset_correction() -> Result<()> {
    // create SimplePatternSplitTokenizer
    let mut stream = new_simple_pattern_split_tokenizer("G\u{FC}nther")?;
    stream.set_reader(gunther_char_stream()?)?;
    B::assert_token_stream_contents_offsets_final_offset(
        &mut stream,
        &[" ", " is here"],
        &[12, 25],
        &[13, 33],
        Some(GUNTHER.len() as i32),
    )
}

/// `new Analyzer() { tokenizer = make(); return new TokenStreamComponents(tokenizer); }`.
fn simple_tokenizer_analyzer<T: rucene_test_framework::analysis::ReaderTokenizer + 'static>(
    name: &'static str,
    make: fn() -> Result<T>,
) -> Arc<dyn Analyzer> {
    FnAnalyzer::new(name, move |_| Ok(tokenizer_components(own(make()?)))).arc()
}

/// blast some random strings through the analyzer
fn spst_test_random_strings() -> Result<()> {
    let a = simple_tokenizer_analyzer(
        "org.apache.lucene.analysis.pattern.TestSimplePatternSplitTokenizer$1",
        || new_simple_pattern_split_tokenizer("a"),
    );
    B::check_random_data(&*random(), &a, 200 * mult())?;
    a.close();
    let b = simple_tokenizer_analyzer(
        "org.apache.lucene.analysis.pattern.TestSimplePatternSplitTokenizer$2",
        || new_simple_pattern_split_tokenizer("a"),
    );
    B::check_random_data(&*random(), &b, 200 * mult())?;
    b.close();
    Ok(())
}

fn spst_test_end_lookahead() -> Result<()> {
    let mut t = split("(ab)+", "aba")?;
    B::assert_token_stream_contents_offsets_final_offset(&mut t, &["a"], &[2], &[3], Some(3))
}

fn simple_pattern_split_tokenizer_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.pattern.TestSimplePatternSplitTokenizer")
        .method(t("testGreedy", spst_test_greedy))
        .method(t("testBackToBack", spst_test_back_to_back))
        .method(t("testBigLookahead", spst_test_big_lookahead))
        .method(t("testNoTokens", spst_test_no_tokens))
        .method(t(
            "testEmptyStringPatternNoMatch",
            spst_test_empty_string_pattern_no_match,
        ))
        .method(t(
            "testSplitSingleCharWhitespace",
            spst_test_split_single_char_whitespace,
        ))
        .method(t(
            "testSplitMultiCharWhitespace",
            spst_test_split_multi_char_whitespace,
        ))
        .method(t("testLeadingNonToken", spst_test_leading_non_token))
        .method(t("testTrailingNonToken", spst_test_trailing_non_token))
        .method(t(
            "testEmptyStringPatternOneMatch",
            spst_test_empty_string_pattern_one_match,
        ))
        .method(t("testEndOffset", spst_test_end_offset))
        .method(t("testFixedToken", spst_test_fixed_token))
        .method(t("testBasic", spst_test_basic))
        .method(t("testNotDeterminized", spst_test_not_determinized))
        .method(t("testOffsetCorrection", spst_test_offset_correction))
        .method(t("testRandomStrings", spst_test_random_strings))
        .method(t("testEndLookahead", spst_test_end_lookahead))
        .build()
}

fn spt_test_greedy() -> Result<()> {
    let mut t = simple("(foo)+", "bar foofoo baz")?;
    B::assert_token_stream_contents_offsets(&mut t, &["foofoo"], &[4], &[10])
}

fn spt_test_big_lookahead() -> Result<()> {
    let mut t = new_simple_pattern_tokenizer(&format!("{}b", "a".repeat(100)))?;
    let b = "a".repeat(200);
    t.set_reader(reader(&b))?;
    t.reset()?;
    Assert::assert_false(t.increment_token()?);
    Ok(())
}

fn spt_test_one_token() -> Result<()> {
    let s = non_empty_random_unicode_string();
    let mut t = simple(".*", &s)?;
    t.reset()?;
    Assert::assert_true(t.increment_token()?);
    Assert::assert_equals(s.as_str(), term_of(&t).as_str());
    Ok(())
}

fn spt_test_empty_string_pattern_no_match() -> Result<()> {
    let mut t = simple("a*", "bbb")?;
    t.reset()?;
    Assert::assert_false(t.increment_token()?);
    Ok(())
}

fn spt_test_empty_string_pattern_one_match() -> Result<()> {
    let mut t = simple("a*", "bbab")?;
    t.reset()?;
    Assert::assert_true(t.increment_token()?);
    Assert::assert_equals("a", term_of(&t).as_str());
    Assert::assert_false(t.increment_token()?);
    Ok(())
}

fn spt_test_end_offset() -> Result<()> {
    let mut t = simple("a+", "aaabbb")?;
    t.reset()?;
    Assert::assert_true(t.increment_token()?);
    Assert::assert_equals("aaa", term_of(&t).as_str());
    Assert::assert_false(t.increment_token()?);
    t.end()?;
    Assert::assert_equals(6, end_offset(&t));
    Ok(())
}

fn spt_test_fixed_token() -> Result<()> {
    let mut t = simple("aaaa", "aaaaaaaaaaaaaaa")?;
    B::assert_token_stream_contents_offsets(
        &mut t,
        &["aaaa", "aaaa", "aaaa"],
        &[0, 4, 8],
        &[4, 8, 12],
    )
}

fn spt_test_basic() -> Result<()> {
    let qpattern = "\\'([^\\']+)\\'"; // get stuff between "'"
    let tests: [[&str; 3]; 2] = [
        // pattern        input                    output
        [":", "boo:and:foo", ": :"],
        [qpattern, "aaa 'bbb' 'ccc'", "'bbb' 'ccc'"],
    ];
    for test in tests {
        let mut stream = simple(test[0], test[1])?;
        let out = ts_to_string(&mut stream)?;
        Assert::assert_equals_msg(
            Some(&format!("pattern: {} with input: {}", test[0], test[1])),
            test[2],
            out.as_str(),
        );
    }
    Ok(())
}

fn spt_test_not_determinized() -> Result<()> {
    let a = not_determinized();
    expect_throws("java.lang.IllegalArgumentException", || {
        new_simple_pattern_tokenizer_from_automaton(a)
    });
    Ok(())
}

fn spt_test_offset_correction() -> Result<()> {
    // create SimplePatternTokenizer
    let mut stream = new_simple_pattern_tokenizer("G\u{FC}nther")?;
    stream.set_reader(gunther_char_stream()?)?;
    B::assert_token_stream_contents_offsets_final_offset(
        &mut stream,
        &["G\u{FC}nther", "G\u{FC}nther"],
        &[0, 13],
        &[12, 25],
        Some(GUNTHER.len() as i32),
    )
}

/// blast some random strings through the analyzer
fn spt_test_random_strings() -> Result<()> {
    let a = simple_tokenizer_analyzer(
        "org.apache.lucene.analysis.pattern.TestSimplePatternTokenizer$1",
        || new_simple_pattern_tokenizer("a"),
    );
    B::check_random_data(&*random(), &a, 200 * mult())?;
    a.close();
    let b = simple_tokenizer_analyzer(
        "org.apache.lucene.analysis.pattern.TestSimplePatternTokenizer$2",
        || new_simple_pattern_tokenizer("a"),
    );
    B::check_random_data(&*random(), &b, 200 * mult())?;
    b.close();
    Ok(())
}

fn spt_test_end_lookahead() -> Result<()> {
    let mut t = simple("(ab)+", "aba")?;
    B::assert_token_stream_contents_offsets_final_offset(&mut t, &["ab"], &[0], &[2], Some(3))
}

fn simple_pattern_tokenizer_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.pattern.TestSimplePatternTokenizer")
        .method(t("testGreedy", spt_test_greedy))
        .method(t("testBigLookahead", spt_test_big_lookahead))
        .method(t("testOneToken", spt_test_one_token))
        .method(t(
            "testEmptyStringPatternNoMatch",
            spt_test_empty_string_pattern_no_match,
        ))
        .method(t(
            "testEmptyStringPatternOneMatch",
            spt_test_empty_string_pattern_one_match,
        ))
        .method(t("testEndOffset", spt_test_end_offset))
        .method(t("testFixedToken", spt_test_fixed_token))
        .method(t("testBasic", spt_test_basic))
        .method(t("testNotDeterminized", spt_test_not_determinized))
        .method(t("testOffsetCorrection", spt_test_offset_correction))
        .method(t("testRandomStrings", spt_test_random_strings))
        .method(t("testEndLookahead", spt_test_end_lookahead))
        .build()
}

#[test]
fn test_pattern_capture_group_token_filter() {
    run(pattern_capture_group_token_filter_class);
}

#[test]
fn test_pattern_replace_char_filter() {
    run(pattern_replace_char_filter_class);
}

#[test]
fn test_pattern_replace_char_filter_factory() {
    run(pattern_replace_char_filter_factory_class);
}

#[test]
fn test_pattern_replace_filter() {
    run(pattern_replace_filter_class);
}

#[test]
fn test_pattern_replace_filter_factory() {
    run(pattern_replace_filter_factory_class);
}

#[test]
fn test_pattern_tokenizer() {
    run(pattern_tokenizer_class);
}

/// `testHeapFreedAfterClose` (`@Nightly` in Lucene; see the module docs).
#[test]
fn test_pattern_tokenizer_nightly() {
    run(pattern_tokenizer_nightly_class);
}

#[test]
fn test_pattern_tokenizer_factory() {
    run(pattern_tokenizer_factory_class);
}

#[test]
fn test_pattern_typing_filter() {
    run(pattern_typing_filter_class);
}

#[test]
fn test_pattern_typing_filter_factory() {
    run(pattern_typing_filter_factory_class);
}

#[test]
fn test_simple_pattern_split_tokenizer() {
    run(simple_pattern_split_tokenizer_class);
}

#[test]
fn test_simple_pattern_tokenizer() {
    run(simple_pattern_tokenizer_class);
}
