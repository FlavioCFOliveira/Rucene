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
//! Lucene 10.5.0 `lucene/analysis/common/src/test` that rmp task #451
//! lists, each run as a suite by the ported runner:
//! `TestConcatenateGraphFilterFactory`, `TestConcatenatingTokenStream`,
//! `TestDateRecognizerFilter`, `TestDateRecognizerFilterFactory`,
//! `TestFingerprintFilter`, `TestFingerprintFilterFactory`,
//! `TestMiscellaneousFactories`, `TestProtectedTermFilter`,
//! `TestProtectedTermFilterFactory` and `TestStemmerOverrideFilterFactory`.

mod support;

use std::any::TypeId;
use std::collections::HashMap;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::stop_filter::{make_stop_set, new_stop_filter};
use rucene::analysis::tokenattributes::{FlagsAttribute, PayloadAttribute};
use rucene::analysis::{
    default_attribute_factory, default_token_attribute_factory, CharArraySet, StringCharReader,
    TokenStream,
};
use rucene::error::Result;
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::extra::Version;
use rucene_analysis_common::analysis::core::lower_case_filter::LowerCaseFilter;
use rucene_analysis_common::analysis::miscellaneous::concatenate_graph_filter::SEP_LABEL;
use rucene_analysis_common::analysis::miscellaneous::concatenating_token_stream::ConcatenatingTokenStream;
use rucene_analysis_common::analysis::miscellaneous::date_recognizer_filter::new_date_recognizer_filter_with;
use rucene_analysis_common::analysis::miscellaneous::date_recognizer_filter_factory::DateRecognizerFilterFactory;
use rucene_analysis_common::analysis::miscellaneous::empty_token_stream::EmptyTokenStream;
use rucene_analysis_common::analysis::miscellaneous::fingerprint_filter::{
    new_fingerprint_filter, new_fingerprint_filter_with, DEFAULT_MAX_OUTPUT_TOKEN_SIZE,
};
use rucene_analysis_common::analysis::miscellaneous::fingerprint_filter_factory::FingerprintFilterFactory;
use rucene_analysis_common::analysis::miscellaneous::protected_term_filter::new_protected_term_filter;
use rucene_analysis_common::analysis::miscellaneous::protected_term_filter_factory::ProtectedTermFilterFactory;
use rucene_analysis_common::jdk::locale::Locale;
use rucene_analysis_common::jdk::simple_date_format::SimpleDateFormat;
use rucene_test_framework::analysis::mock_tokenizer::{MockTokenizer, WHITESPACE};
use rucene_test_framework::analysis::{
    shared, BaseTokenStreamFactoryTestCase, BaseTokenStreamTestCase, CannedTokenStream, Token,
};
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::string_mock_resource_loader::StringMockResourceLoader;
use support::{
    boxed, btstc_class, concrete_factory, factory_class, iae_message, run, t, token_filter_factory,
};

type B = BaseTokenStreamTestCase;

/// The package of the test classes, which resolves their resources.
const MISC: &str = "org.apache.lucene.analysis.miscellaneous";

/// `whitespaceMockTokenizer(String)`.
fn ws(input: &str) -> Result<MockTokenizer> {
    B::whitespace_mock_tokenizer_str(input)
}

/// `new MockTokenizer(MockTokenizer.WHITESPACE, false)` over `input`.
fn mock_ws(input: &str) -> Result<MockTokenizer> {
    let mut tokenizer = MockTokenizer::new(Arc::clone(&WHITESPACE), false)?;
    tokenizer.set_reader(Box::new(StringCharReader::new(input)))?;
    Ok(tokenizer)
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

// -----------------------------------------------------------------------------
// TestConcatenateGraphFilterFactory
// -----------------------------------------------------------------------------

/// `input.replace(' ', (char) ConcatenateGraphFilter.SEP_LABEL)`.
fn with_sep_label(input: &str) -> String {
    let sep = char::from_u32(SEP_LABEL as u32).expect("SEP_LABEL is a char");
    input.replace(' ', &sep.to_string())
}

fn cgff_test() -> Result<()> {
    for consume_all in [true, false] {
        let input = "A1 B2 A1 D4 C3";
        let mut tokenizer = mock_ws(input)?;
        tokenizer.set_enable_checks(consume_all);
        let mut stream =
            fac("ConcatenateGraph", &["tokenSeparator", "\u{001F}"])?.create(boxed(tokenizer))?;
        B::assert_token_stream_contents(&mut *stream, &[with_sep_label(input).as_str()])?;
    }
    Ok(())
}

fn cgff_test_empty_token_separator() -> Result<()> {
    let input = "A1 B2 A1 D4 C3";
    let output = "A1A1D4C3";
    let tokenizer = mock_ws(input)?;
    let stream = new_stop_filter(boxed(tokenizer), make_stop_set(["B2"], false))?;
    let mut stream = fac("ConcatenateGraph", &["tokenSeparator", ""])?.create(boxed(stream))?;
    B::assert_token_stream_contents(&mut *stream, &[output])
}

fn cgff_test_preserve_position_increments() -> Result<()> {
    let input = "A1 B2 A1 D4 C3";
    let output = "A1 A1 D4 C3";
    let tokenizer = mock_ws(input)?;
    let stream = new_stop_filter(boxed(tokenizer), make_stop_set(["B2"], false))?;
    let mut stream = fac(
        "ConcatenateGraph",
        &[
            "tokenSeparator",
            "\u{001F}",
            "preservePositionIncrements",
            "false",
        ],
    )?
    .create(boxed(stream))?;
    B::assert_token_stream_contents(&mut *stream, &[with_sep_label(output).as_str()])
}

fn cgff_test_required() -> Result<()> {
    // no params are required
    fac("ConcatenateGraph", &[])?;
    Ok(())
}

/// Test that bogus arguments result in exception
fn cgff_test_bogus_arguments() -> Result<()> {
    assert_bogus("ConcatenateGraph", &["bogusArg", "bogusValue"])
}

fn cgff_test_separator() -> Result<()> {
    let input = "A B C D E F J H";
    let output = "B-C-F-H";
    let tokenizer = mock_ws(input)?;
    let stream = new_stop_filter(boxed(tokenizer), make_stop_set(["A", "D", "E", "J"], false))?;
    let mut stream = fac(
        "ConcatenateGraph",
        &["tokenSeparator", "-", "preservePositionIncrements", "false"],
    )?
    .create(boxed(stream))?;
    B::assert_token_stream_contents(&mut *stream, &[output])
}

fn concatenate_graph_filter_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.miscellaneous.TestConcatenateGraphFilterFactory")
        .method(t("test", cgff_test))
        .method(t(
            "testEmptyTokenSeparator",
            cgff_test_empty_token_separator,
        ))
        .method(t(
            "testPreservePositionIncrements",
            cgff_test_preserve_position_increments,
        ))
        .method(t("testRequired", cgff_test_required))
        .method(t("testBogusArguments", cgff_test_bogus_arguments))
        .method(t("testSeparator", cgff_test_separator))
        .build()
}

// -----------------------------------------------------------------------------
// TestConcatenatingTokenStream
// -----------------------------------------------------------------------------

/// `new MockTokenizer(factory, MockTokenizer.WHITESPACE, lowerCase)`, kept
/// by the test while the concatenation reads it (Java's references).
fn shared_mock(
    factory: Arc<dyn rucene::util::attribute::AttributeFactory>,
    lower_case: bool,
) -> Result<Arc<AtomicRefCell<MockTokenizer>>> {
    Ok(Arc::new(AtomicRefCell::new(
        MockTokenizer::with_factory_default_length(factory, Arc::clone(&WHITESPACE), lower_case)?,
    )))
}

/// `tokenizer.setReader(new StringReader(text))`.
fn set_reader(tokenizer: &Arc<AtomicRefCell<MockTokenizer>>, text: &str) -> Result<()> {
    tokenizer
        .borrow_mut()
        .set_reader(Box::new(StringCharReader::new(text)))
}

fn cts_test_basic() -> Result<()> {
    let factory = B::new_attribute_factory();

    let first = shared_mock(Arc::clone(&factory), false)?;
    set_reader(&first, "first words ")?;
    let second = shared_mock(Arc::clone(&factory), false)?;
    set_reader(&second, "second words")?;
    let third = shared_mock(Arc::clone(&factory), false)?;
    set_reader(&third, " third words")?;

    let mut ts = ConcatenatingTokenStream::new(vec![
        shared(&first),
        shared(&second),
        boxed(EmptyTokenStream::new()),
        shared(&third),
    ])?;
    B::assert_token_stream_contents_offsets(
        &mut ts,
        &["first", "words", "second", "words", "third", "words"],
        &[0, 6, 12, 19, 25, 31],
        &[5, 11, 18, 24, 30, 36],
    )?;

    // test re-use
    set_reader(&first, "first words ")?;
    set_reader(&second, "second words")?;
    set_reader(&third, " third words")?;
    B::assert_token_stream_contents_offsets_pos_increments(
        &mut ts,
        &["first", "words", "second", "words", "third", "words"],
        &[0, 6, 12, 19, 25, 31],
        &[5, 11, 18, 24, 30, 36],
        &[1, 1, 1, 1, 1, 1],
    )
}

fn cts_test_offset_gaps() -> Result<()> {
    let cts1 = CannedTokenStream::with_final(
        2,
        10,
        vec![
            Token::with_offsets("a", 0, 1),
            Token::with_offsets("b", 2, 3),
        ],
    );
    let cts2 = CannedTokenStream::with_final(
        2,
        10,
        vec![
            Token::with_offsets("c", 0, 1),
            Token::with_offsets("d", 2, 3),
        ],
    );
    let mut ts = ConcatenatingTokenStream::new(vec![boxed(cts1), boxed(cts2)])?;
    B::assert_token_stream_contents_full(
        &mut ts,
        &["a", "b", "c", "d"],
        Some(&[0, 2, 10, 12]),
        Some(&[1, 3, 11, 13]),
        None,
        Some(&[1, 1, 3, 1]),
        None,
        Some(20),
        Some(2),
        None,
        false,
        None,
        None,
        None,
    )
}

fn cts_test_inconsistent_attributes() -> Result<()> {
    let factory = B::new_attribute_factory();

    let first = shared_mock(Arc::clone(&factory), false)?;
    set_reader(&first, "first words ")?;
    first
        .borrow_mut()
        .attribute_source_mut()
        .add_attribute_interface::<dyn PayloadAttribute>()?;
    let second = shared_mock(Arc::clone(&factory), false)?;
    set_reader(&second, "second words")?;
    second
        .borrow_mut()
        .attribute_source_mut()
        .add_attribute_interface::<dyn FlagsAttribute>()?;

    let mut ts = ConcatenatingTokenStream::new(vec![shared(&first), shared(&second)])?;
    Assert::assert_true(
        ts.attribute_source()
            .has_attribute_by_id(TypeId::of::<dyn FlagsAttribute>()),
    );
    Assert::assert_true(
        ts.attribute_source()
            .has_attribute_by_id(TypeId::of::<dyn PayloadAttribute>()),
    );

    B::assert_token_stream_contents_offsets(
        &mut ts,
        &["first", "words", "second", "words"],
        &[0, 6, 12, 19],
        &[5, 11, 18, 24],
    )
}

fn cts_test_inconsistent_attribute_factories() -> Result<()> {
    let first = shared_mock(default_attribute_factory(), true)?;
    let second = shared_mock(default_token_attribute_factory(), true)?;
    expect_throws("java.lang.IllegalArgumentException", || {
        ConcatenatingTokenStream::new(vec![shared(&first), shared(&second)])
    });
    Ok(())
}

fn concatenating_token_stream_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.miscellaneous.TestConcatenatingTokenStream")
        .method(t("testBasic", cts_test_basic))
        .method(t("testOffsetGaps", cts_test_offset_gaps))
        .method(t(
            "testInconsistentAttributes",
            cts_test_inconsistent_attributes,
        ))
        .method(t(
            "testInconsistentAttributeFactories",
            cts_test_inconsistent_attribute_factories,
        ))
        .build()
}

// -----------------------------------------------------------------------------
// TestDateRecognizerFilter
// -----------------------------------------------------------------------------

fn drf_test() -> Result<()> {
    let test =
        "The red fox jumped over the lazy dogs on 7/11/2006 The dogs finally reacted on 7/12/2006";
    let date_format = Arc::new(SimpleDateFormat::new("MM/dd/yyyy", &Locale::english())?);
    let mut ts = new_date_recognizer_filter_with(boxed(ws(test)?), Some(date_format))?;
    B::assert_stream_has_number_of_tokens(&mut ts, 2)?;
    ts.close()
}

fn date_recognizer_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.miscellaneous.TestDateRecognizerFilter")
        .method(t("test", drf_test))
        .build()
}

// -----------------------------------------------------------------------------
// TestDateRecognizerFilterFactory
// -----------------------------------------------------------------------------

fn drff_test_bad_language_tag_throws_exception() -> Result<()> {
    expect_throws("java.lang.Exception", || {
        let mut args = HashMap::new();
        args.insert(
            DateRecognizerFilterFactory::LOCALE.to_string(),
            "en_US".to_string(),
        );
        DateRecognizerFilterFactory::new(&mut args)
    });
    Ok(())
}

fn drff_test_good_locale_parses_well() -> Result<()> {
    let mut args = HashMap::new();
    args.insert(
        DateRecognizerFilterFactory::LOCALE.to_string(),
        "en-US".to_string(),
    );
    DateRecognizerFilterFactory::new(&mut args)?;
    Ok(())
}

fn date_recognizer_filter_factory_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.miscellaneous.TestDateRecognizerFilterFactory")
        .method(t(
            "testBadLanguageTagThrowsException",
            drff_test_bad_language_tag_throws_exception,
        ))
        .method(t(
            "testGoodLocaleParsesWell",
            drff_test_good_locale_parses_well,
        ))
        .build()
}

// -----------------------------------------------------------------------------
// TestFingerprintFilter
// -----------------------------------------------------------------------------

/// The body every `TestFingerprintFilter` method shares: `input` through a
/// whitespace mock tokenizer whose checks follow `consumeAll`, then the
/// filter `make` builds, for both values of `consumeAll`.
fn fp_check(
    input: &str,
    make: impl Fn(Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>>,
    expected: &[&str],
) -> Result<()> {
    for consume_all in [true, false] {
        let mut tokenizer = ws(input)?;
        tokenizer.set_enable_checks(consume_all);
        let mut stream = make(boxed(tokenizer))?;
        B::assert_token_stream_contents(&mut *stream, expected)?;
    }
    Ok(())
}

fn fp_test_dups_and_sorting() -> Result<()> {
    fp_check(
        "B A B E",
        |s| Ok(boxed(new_fingerprint_filter(s)?)),
        &["A B E"],
    )
}

fn fp_test_all_dup_values() -> Result<()> {
    fp_check("B2 B2", |s| Ok(boxed(new_fingerprint_filter(s)?)), &["B2"])
}

fn fp_test_max_fingerprint_size() -> Result<()> {
    fp_check(
        "B2 A1 C3 D4 E5 F6 G7 H1",
        |s| Ok(boxed(new_fingerprint_filter_with(s, 4, ' ')?)),
        &[],
    )
}

fn fp_test_custom_separator() -> Result<()> {
    fp_check(
        "B2 A1 C3 B2",
        |s| {
            Ok(boxed(new_fingerprint_filter_with(
                s,
                DEFAULT_MAX_OUTPUT_TOKEN_SIZE,
                '_',
            )?))
        },
        &["A1_B2_C3"],
    )
}

fn fp_test_single_token() -> Result<()> {
    fp_check("A1", |s| Ok(boxed(new_fingerprint_filter(s)?)), &["A1"])
}

fn fp_test_empty() -> Result<()> {
    fp_check("", |s| Ok(boxed(new_fingerprint_filter(s)?)), &[])
}

fn fingerprint_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.miscellaneous.TestFingerprintFilter")
        .method(t("testDupsAndSorting", fp_test_dups_and_sorting))
        .method(t("testAllDupValues", fp_test_all_dup_values))
        .method(t("testMaxFingerprintSize", fp_test_max_fingerprint_size))
        .method(t("testCustomSeparator", fp_test_custom_separator))
        .method(t("testSingleToken", fp_test_single_token))
        .method(t("testEmpty", fp_test_empty))
        .build()
}

// -----------------------------------------------------------------------------
// TestFingerprintFilterFactory
// -----------------------------------------------------------------------------

fn fpf_test() -> Result<()> {
    for consume_all in [true, false] {
        let mut tokenizer = mock_ws("A1 B2 A1 D4 C3")?;
        tokenizer.set_enable_checks(consume_all);
        let mut stream = fac(
            "Fingerprint",
            &[
                FingerprintFilterFactory::MAX_OUTPUT_TOKEN_SIZE_KEY,
                "256",
                FingerprintFilterFactory::SEPARATOR_KEY,
                "_",
            ],
        )?
        .create(boxed(tokenizer))?;
        B::assert_token_stream_contents(&mut *stream, &["A1_B2_C3_D4"])?;
    }
    Ok(())
}

fn fpf_test_required() -> Result<()> {
    // no params are required
    fac("Fingerprint", &[])?;
    Ok(())
}

/// Test that bogus arguments result in exception
fn fpf_test_bogus_arguments() -> Result<()> {
    assert_bogus(
        "Fingerprint",
        &[
            FingerprintFilterFactory::MAX_OUTPUT_TOKEN_SIZE_KEY,
            "3",
            "bogusArg",
            "bogusValue",
        ],
    )
}

fn fingerprint_filter_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.miscellaneous.TestFingerprintFilterFactory")
        .method(t("test", fpf_test))
        .method(t("testRequired", fpf_test_required))
        .method(t("testBogusArguments", fpf_test_bogus_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// TestMiscellaneousFactories
// -----------------------------------------------------------------------------

/// Ensure the ASCIIFoldingFilterFactory works
fn mf_test_ascii_folding() -> Result<()> {
    let stream = ws("Česká")?;
    let mut stream = fac("ASCIIFolding", &[])?.create(boxed(stream))?;
    B::assert_token_stream_contents(&mut *stream, &["Ceska"])
}

/// Test that bogus arguments result in exception
fn mf_test_bogus_arguments() -> Result<()> {
    assert_bogus("ASCIIFolding", &["bogusArg", "bogusValue"])
}

fn miscellaneous_factories_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.miscellaneous.TestMiscellaneousFactories")
        .method(t("testASCIIFolding", mf_test_ascii_folding))
        .method(t("testBogusArguments", mf_test_bogus_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// TestProtectedTermFilter
// -----------------------------------------------------------------------------

fn ptf_test_basic() -> Result<()> {
    let cts = CannedTokenStream::new(vec![
        Token::with_pos_inc("Alice", 1, 0, 5),
        Token::with_pos_inc("Bob", 1, 6, 9),
        Token::with_pos_inc("Clara", 1, 10, 15),
        Token::with_pos_inc("David", 1, 16, 21),
    ]);

    let mut protected_terms = CharArraySet::new(5, true);
    protected_terms.add("bob");

    let mut ts = new_protected_term_filter(Some(protected_terms), boxed(cts), |input| {
        Ok(boxed(LowerCaseFilter::new(input)?))
    })?;
    B::assert_token_stream_contents(&mut ts, &["alice", "Bob", "clara", "david"])
}

fn protected_term_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.miscellaneous.TestProtectedTermFilter")
        .method(t("testBasic", ptf_test_basic))
        .build()
}

// -----------------------------------------------------------------------------
// TestProtectedTermFilterFactory
// -----------------------------------------------------------------------------

fn ptff_test_inform() -> Result<()> {
    let factory = concrete_factory(
        MISC,
        ProtectedTermFilterFactory::new,
        &[
            "protected",
            "protected-1.txt",
            "ignoreCase",
            "true",
            "wrappedFilters",
            "lowercase",
        ],
    )?;
    let protected_terms = factory.get_protected_terms();
    Assert::assert_true_msg(
        "protectedTerms is null and it shouldn't be",
        protected_terms.is_some(),
    );
    let size = protected_terms.map_or(0, |p| p.size());
    Assert::assert_true_msg(&format!("protectedTerms Size: {size} is not: 2"), size == 2);
    Assert::assert_true_msg(
        &format!("{} does not equal: true", factory.is_ignore_case()),
        factory.is_ignore_case(),
    );

    let factory = concrete_factory(
        MISC,
        ProtectedTermFilterFactory::new,
        &[
            "protected",
            "protected-1.txt, protected-2.txt",
            "ignoreCase",
            "true",
            "wrappedFilters",
            "lowercase",
        ],
    )?;
    let protected_terms = factory.get_protected_terms();
    Assert::assert_true_msg(
        "protectedTerms is null and it shouldn't be",
        protected_terms.is_some(),
    );
    let size = protected_terms.map_or(0, |p| p.size());
    Assert::assert_true_msg(&format!("protectedTerms Size: {size} is not: 4"), size == 4);
    Assert::assert_true_msg(
        &format!("{} does not equal: true", factory.is_ignore_case()),
        factory.is_ignore_case(),
    );

    // defaults
    let factory = concrete_factory(
        MISC,
        ProtectedTermFilterFactory::new,
        &["protected", "protected-1.txt"],
    )?;
    Assert::assert_equals(false, factory.is_ignore_case());
    Ok(())
}

fn ptff_test_basic() -> Result<()> {
    let str = "Foo Clara Bar David";
    let stream = ws(str)?;
    let mut stream = fac(
        "ProtectedTerm",
        &[
            "ignoreCase",
            "true",
            "protected",
            "protected-1.txt",
            "wrappedFilters",
            "lowercase",
        ],
    )?
    .create(boxed(stream))?;
    B::assert_token_stream_contents(&mut *stream, &["Foo", "clara", "Bar", "david"])
}

fn ptff_test_multiple_wrapped_filters_with_params() -> Result<()> {
    let str = "Foo Clara Bar David";
    let stream = ws(str)?;
    let mut stream = fac(
        "ProtectedTerm",
        &[
            "ignoreCase",
            "true",
            "protected",
            "protected-1.txt",
            "wrappedFilters",
            "lowercase, truncate",
            "truncate.prefixLength",
            "2",
        ],
    )?
    .create(boxed(stream))?;
    B::assert_token_stream_contents(&mut *stream, &["Foo", "cl", "Bar", "da"])
}

fn ptff_test_multiple_same_named_filters_with_params() -> Result<()> {
    let str = "Foo Clara Bar David";
    let stream = ws(str)?;
    let mut stream = fac(
        "ProtectedTerm",
        &[
            "ignoreCase",
            "true",
            "protected",
            "protected-1.txt",
            "wrappedFilters",
            "truncate-A, reversestring, truncate-B",
            "truncate-A.prefixLength",
            "3",
            "truncate-B.prefixLength",
            "2",
        ],
    )?
    .create(boxed(stream))?;
    B::assert_token_stream_contents(&mut *stream, &["Foo", "al", "Bar", "va"])?;

    // same-named wrapped filters, one with an ID and another without
    let stream = ws(str)?;
    let mut stream = fac(
        "ProtectedTerm",
        &[
            "ignoreCase",
            "true",
            "protected",
            "protected-1.txt",
            "wrappedFilters",
            "truncate, reversestring, truncate-A",
            "truncate.prefixLength",
            "3",
            "truncate-A.prefixLength",
            "2",
        ],
    )?
    .create(boxed(stream))?;
    B::assert_token_stream_contents(&mut *stream, &["Foo", "al", "Bar", "va"])?;

    // Case-insensitive wrapped "filter-id"
    let stream = ws(str)?;
    let mut stream = fac(
        "ProtectedTerm",
        &[
            "ignoreCase",
            "true",
            "protected",
            "protected-1.txt",
            "wrappedFilters",
            "TRUNCATE-a, reversestring, truncate-b",
            "truncate-A.prefixLength",
            "3",
            "TRUNCATE-B.prefixLength",
            "2",
        ],
    )?
    .create(boxed(stream))?;
    B::assert_token_stream_contents(&mut *stream, &["Foo", "al", "Bar", "va"])
}

/// Test that bogus arguments result in exception
fn ptff_test_bogus_arguments() -> Result<()> {
    let exception = iae_message(|| {
        fac(
            "ProtectedTerm",
            &["protected", "protected-1.txt", "bogusArg", "bogusValue"],
        )
    });
    Assert::assert_true(exception.contains("Unknown parameters"));

    // same-named wrapped filters
    let exception = iae_message(|| {
        fac(
            "ProtectedTerm",
            &[
                "protected",
                "protected-1.txt",
                "wrappedFilters",
                "truncate, truncate",
            ],
        )
    });
    Assert::assert_true(exception.contains("wrappedFilters contains duplicate"));

    // case-insensitive same-named wrapped filters
    let exception = iae_message(|| {
        fac(
            "ProtectedTerm",
            &[
                "protected",
                "protected-1.txt",
                "wrappedFilters",
                "TRUNCATE, truncate",
            ],
        )
    });
    Assert::assert_true(exception.contains("wrappedFilters contains duplicate"));

    // case-insensitive same-named wrapped filter IDs
    let exception = iae_message(|| {
        fac(
            "ProtectedTerm",
            &[
                "protected",
                "protected-1.txt",
                "wrappedFilters",
                "truncate-ABC, truncate-abc",
            ],
        )
    });
    Assert::assert_true(exception.contains("wrappedFilters contains duplicate"));

    // mismatched wrapped filter and associated args
    let exception = iae_message(|| {
        fac(
            "ProtectedTerm",
            &[
                "protected",
                "protected-1.txt",
                "wrappedFilters",
                "truncate-A, reversestring, truncate-B",
                "truncate.prefixLength",
                "3",
                "truncate-A.prefixLength",
                "2",
            ],
        )
    });
    Assert::assert_true(exception.contains("Unknown parameters: {truncate.prefixLength=3}"));

    // missing required arg(s) for wrapped filter
    let str = "Foo Clara Bar David";
    let stream = ws(str)?;
    let exception = iae_message(|| {
        fac(
            "ProtectedTerm",
            &["protected", "protected-1.txt", "wrappedFilters", "length"],
        )?
        .create(boxed(stream))
    });
    Assert::assert_true(exception.contains("Configuration Error: missing parameter"));
    Ok(())
}

fn protected_term_filter_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.miscellaneous.TestProtectedTermFilterFactory")
        .method(t("testInform", ptff_test_inform))
        .method(t("testBasic", ptff_test_basic))
        .method(t(
            "testMultipleWrappedFiltersWithParams",
            ptff_test_multiple_wrapped_filters_with_params,
        ))
        .method(t(
            "testMultipleSameNamedFiltersWithParams",
            ptff_test_multiple_same_named_filters_with_params,
        ))
        .method(t("testBogusArguments", ptff_test_bogus_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// TestStemmerOverrideFilterFactory
// -----------------------------------------------------------------------------

/// `tokenFilterFactory("StemmerOverride", Version.LATEST, new
/// StringMockResourceLoader("dogs\tcat"), keysAndValues...)`.
fn sof_with_dict(
    keys_and_values: &[&str],
) -> Result<Arc<dyn rucene::analysis::factories::TokenFilterFactory>> {
    BaseTokenStreamFactoryTestCase::token_filter_factory_with_loader(
        "StemmerOverride",
        Some(Version::LATEST),
        &StringMockResourceLoader::new("dogs\tcat"),
        keys_and_values,
    )
}

fn sof_test_keywords() -> Result<()> {
    // our stemdict stems dogs to 'cat'
    let stream = ws("testing dogs")?;
    let stream = sof_with_dict(&["dictionary", "stemdict.txt"])?.create(boxed(stream))?;
    let mut stream = fac("PorterStem", &[])?.create(stream)?;
    B::assert_token_stream_contents(&mut *stream, &["test", "cat"])
}

fn sof_test_keywords_case_insensitive() -> Result<()> {
    let stream = ws("testing DoGs")?;
    let stream = sof_with_dict(&["dictionary", "stemdict.txt", "ignoreCase", "true"])?
        .create(boxed(stream))?;
    let mut stream = fac("PorterStem", &[])?.create(stream)?;
    B::assert_token_stream_contents(&mut *stream, &["test", "cat"])
}

/// Test that bogus arguments result in exception
fn sof_test_bogus_arguments() -> Result<()> {
    assert_bogus("StemmerOverride", &["bogusArg", "bogusValue"])
}

fn stemmer_override_filter_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.miscellaneous.TestStemmerOverrideFilterFactory")
        .method(t("testKeywords", sof_test_keywords))
        .method(t(
            "testKeywordsCaseInsensitive",
            sof_test_keywords_case_insensitive,
        ))
        .method(t("testBogusArguments", sof_test_bogus_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// The suites
// -----------------------------------------------------------------------------

#[test]
fn test_concatenate_graph_filter_factory() {
    run(concatenate_graph_filter_factory_class);
}

#[test]
fn test_concatenating_token_stream() {
    run(concatenating_token_stream_class);
}

#[test]
fn test_date_recognizer_filter() {
    run(date_recognizer_filter_class);
}

#[test]
fn test_date_recognizer_filter_factory() {
    run(date_recognizer_filter_factory_class);
}

#[test]
fn test_fingerprint_filter() {
    run(fingerprint_filter_class);
}

#[test]
fn test_fingerprint_filter_factory() {
    run(fingerprint_filter_factory_class);
}

#[test]
fn test_miscellaneous_factories() {
    run(miscellaneous_factories_class);
}

#[test]
fn test_protected_term_filter() {
    run(protected_term_filter_class);
}

#[test]
fn test_protected_term_filter_factory() {
    run(protected_term_filter_factory_class);
}

#[test]
fn test_stemmer_override_filter_factory() {
    run(stemmer_override_filter_factory_class);
}
