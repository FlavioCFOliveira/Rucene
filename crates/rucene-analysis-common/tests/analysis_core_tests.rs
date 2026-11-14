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
//! that rmp task #449 lists, each run as a suite by the ported runner:
//! `org.apache.lucene.analysis.util.TestCharTokenizers` and the
//! `org.apache.lucene.analysis.core` classes `TestCoreFactories`,
//! `TestDecimalDigitFilter`, `TestDecimalDigitFilterFactory`,
//! `TestDuelingAnalyzers`, `TestKeywordAnalyzer`, `TestKeywordTokenizer`,
//! `TestTypeTokenFilter`, `TestTypeTokenFilterFactory`,
//! `TestUnicodeWhitespaceTokenizer` and `TestWhitespaceAnalyzer`.

mod support;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};

use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::factories::TokenizerFactory;
use rucene::analysis::tokenattributes::{
    CharTermAttribute, OffsetAttribute, PositionIncrementAttribute, TypeAttribute,
};
use rucene::analysis::{
    new_standard_tokenizer, new_standard_tokenizer_with_factory, Analyzer, CharReader,
    StringCharReader, TokenStream,
};
use rucene::document::{Document, Store, StringField, TextField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::IndexReader;
use rucene::search::NO_MORE_DOCS;
use rucene::store::{ByteBuffersDirectory, Directory};
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::automaton::{Automaton, CharacterRunAutomaton, Operations};
use rucene::util::bit_sets::SparseFixedBitSet;
use rucene::util::extra::Version;
use rucene::util::resource_loader::ResourceLoaderAware;
use rucene::util::BytesRef;
use rucene_analysis_common::analysis::core::decimal_digit_filter::new_decimal_digit_filter;
use rucene_analysis_common::analysis::core::keyword_analyzer::KeywordAnalyzer;
use rucene_analysis_common::analysis::core::keyword_tokenizer::{
    new_keyword_tokenizer, new_keyword_tokenizer_with_factory,
};
use rucene_analysis_common::analysis::core::keyword_tokenizer_factory::KeywordTokenizerFactory;
use rucene_analysis_common::analysis::core::letter_tokenizer::{
    new_letter_tokenizer_with_factory, new_letter_tokenizer_with_factory_and_max_token_len,
};
use rucene_analysis_common::analysis::core::lower_case_filter::LowerCaseFilter;
use rucene_analysis_common::analysis::core::simple_analyzer::SimpleAnalyzer;
use rucene_analysis_common::analysis::core::type_token_filter::new_type_token_filter;
use rucene_analysis_common::analysis::core::type_token_filter_factory::TypeTokenFilterFactory;
use rucene_analysis_common::analysis::core::unicode_whitespace_tokenizer::new_unicode_whitespace_tokenizer;
use rucene_analysis_common::analysis::core::whitespace_analyzer::WhitespaceAnalyzer;
use rucene_analysis_common::analysis::core::whitespace_tokenizer::new_whitespace_tokenizer_with_factory_and_max_token_len;
use rucene_analysis_common::analysis::core::whitespace_tokenizer_factory::WhitespaceTokenizerFactory;
use rucene_analysis_common::analysis::util::char_tokenizer::{
    from_separator_char_predicate, from_token_char_predicate,
};
use rucene_analysis_common::jdk::character::{
    get_numeric_value, is_digit, is_letter, is_whitespace,
};
use rucene_test_framework::analysis::mock_tokenizer::{MockTokenizer, WHITESPACE};
use rucene_test_framework::analysis::{
    components, shared, sink, tokenizer_components, BaseTokenStreamFactoryTestCase,
    BaseTokenStreamTestCase, CannedTokenStream, MockAnalyzer, MockReaderWrapper,
};
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::{RANDOM_MULTIPLIER, VERBOSE};
use rucene_test_framework::util::{English, LuceneTestCase, TestUtil};
use support::{
    boxed, btstc_class, factory_class, hook, iae_message, make_args, run, t, test_class_loader,
    token_filter_factory, FnAnalyzer,
};

type F = BaseTokenStreamFactoryTestCase;
type B = BaseTokenStreamTestCase;

/// `random()`.
fn random() -> Arc<dyn Random> {
    LuceneTestCase::random()
}

/// `s.toLowerCase(Locale.ROOT)` over UTF-16 (a supplementary letter maps by
/// code point, as Java's does).
fn lower_root(s: &str) -> String {
    s.chars().flat_map(|c| c.to_lowercase()).collect()
}

/// `tokenizerFactory(name, keysAndValues...)` of a test class of `package`.
fn tokenizer_factory(
    package: &str,
    name: &str,
    keys_and_values: &[&str],
) -> Result<Arc<dyn TokenizerFactory>> {
    F::tokenizer_factory_with_loader(
        name,
        Some(Version::LATEST),
        &test_class_loader(package),
        keys_and_values,
    )
}

/// `tokenizer.getClass()` names `class`: the tokenizer's body is the
/// logic of that class (Rust has no runtime class; the body's `Debug` names
/// its type).
fn assert_tokenizer_class(class: &str, tokenizer: &rucene::analysis::factories::FactoryTokenizer) {
    let body = format!("{:?}", tokenizer.logic());
    Assert::assert_true_msg(
        &format!("expected {class}, got {body}"),
        body.starts_with(&format!("{class}Logic"))
            || body.contains(&format!("logic: {class}Logic")),
    );
}

// -----------------------------------------------------------------------------
// TestCharTokenizers
// -----------------------------------------------------------------------------

/// test to read surrogate pairs without loosing the pairing if the
/// surrogate pair is at the border of the internal IO buffer
fn ct_test_read_supplementary_chars() -> Result<()> {
    // create random input
    let mut num = 1024 + random().next_int_bound(1024);
    num *= *RANDOM_MULTIPLIER;
    let mut builder: Vec<u16> = Vec::new();
    for i in 1..num {
        builder.extend("\u{1041c}abc".encode_utf16());
        if (i % 10) == 0 {
            builder.push(b' ' as u16);
        }
    }
    // internal buffer size is 1024 make sure we have a surrogate pair right at the border
    let pair: Vec<u16> = "\u{1041c}".encode_utf16().collect();
    builder.splice(1023..1023, pair);
    let text = String::from_utf16(&builder).expect("valid UTF-16");
    let mut tokenizer = new_letter_tokenizer_with_factory(B::new_attribute_factory())?;
    tokenizer.set_reader(Box::new(StringCharReader::new(text.clone())))?;
    let lower = lower_root(&text);
    // `String.split(" ")` drops trailing empty strings (the text ends with a
    // space when `num - 1` is a multiple of 10).
    let mut expected: Vec<&str> = lower.split(' ').collect();
    while expected.last() == Some(&"") {
        expected.pop();
    }
    let mut stream = LowerCaseFilter::new(boxed(tokenizer))?;
    B::assert_token_stream_contents(&mut stream, &expected)
}

/// test to extend the buffer TermAttribute buffer internally. If the
/// internal alg that extends the size of the char array only extends by 1
/// char and the next char to be filled in is a supplementary codepoint
/// (using 2 chars) an index out of bound exception is triggered.
fn ct_test_extend_char_buffer() -> Result<()> {
    for i in 0..40 {
        let mut builder = "a".repeat(1 + i);
        builder.push_str("\u{1041c}abc");
        let mut tokenizer = new_letter_tokenizer_with_factory(B::new_attribute_factory())?;
        tokenizer.set_reader(Box::new(StringCharReader::new(builder.clone())))?;
        let mut stream = LowerCaseFilter::new(boxed(tokenizer))?;
        B::assert_token_stream_contents(&mut stream, &[lower_root(&builder).as_str()])?;
    }
    Ok(())
}

/// tests the max word length of 255 - tokenizer will split at the 255 char
/// no matter what happens
fn ct_test_max_word_length() -> Result<()> {
    let builder = "A".repeat(255);
    let mut tokenizer = new_letter_tokenizer_with_factory(B::new_attribute_factory())?;
    tokenizer.set_reader(Box::new(StringCharReader::new(format!(
        "{builder}{builder}"
    ))))?;
    let mut stream = LowerCaseFilter::new(boxed(tokenizer))?;
    let lower = lower_root(&builder);
    B::assert_token_stream_contents(&mut stream, &[lower.as_str(), lower.as_str()])
}

/// tests the max word length passed as parameter - tokenizer will split at
/// the passed position char no matter what happens
fn ct_test_custom_max_token_length() -> Result<()> {
    let mut builder = "A".repeat(100);
    let mut tokenizer =
        new_letter_tokenizer_with_factory_and_max_token_len(B::new_attribute_factory(), 100)?;
    // Tricky, passing two copies of the string to the reader....
    tokenizer.set_reader(Box::new(StringCharReader::new(format!(
        "{builder}{builder}"
    ))))?;
    let mut stream = LowerCaseFilter::new(boxed(tokenizer))?;
    let lower = lower_root(&builder);
    B::assert_token_stream_contents(&mut stream, &[lower.as_str(), lower.as_str()])?;

    let e = iae_message(|| {
        new_letter_tokenizer_with_factory_and_max_token_len(B::new_attribute_factory(), -1)
    });
    Assert::assert_equals(
        "maxTokenLen must be greater than 0 and less than 1048576 passed: -1",
        e.as_str(),
    );

    let mut tokenizer =
        new_letter_tokenizer_with_factory_and_max_token_len(B::new_attribute_factory(), 100)?;
    tokenizer.set_reader(Box::new(StringCharReader::new(format!(
        "{builder}{builder}"
    ))))?;
    B::assert_token_stream_contents(&mut tokenizer, &[builder.as_str(), builder.as_str()])?;

    // Let's test that we can get a token longer than 255 through.
    builder = "Z".repeat(500);
    let mut tokenizer =
        new_letter_tokenizer_with_factory_and_max_token_len(B::new_attribute_factory(), 500)?;
    tokenizer.set_reader(Box::new(StringCharReader::new(builder.clone())))?;
    B::assert_token_stream_contents(&mut tokenizer, &[builder.as_str()])?;

    // Just to be sure what is happening here, token lengths of zero make no sense,
    // Let's try the edge cases, token > I/O buffer (4096)
    builder = "aUrOkIjq".repeat(600); // 600 * 8 = 4800 chars.
    let e = iae_message(|| {
        new_letter_tokenizer_with_factory_and_max_token_len(B::new_attribute_factory(), 0)
    });
    Assert::assert_equals(
        "maxTokenLen must be greater than 0 and less than 1048576 passed: 0",
        e.as_str(),
    );
    let e = iae_message(|| {
        new_letter_tokenizer_with_factory_and_max_token_len(B::new_attribute_factory(), 10_000_000)
    });
    Assert::assert_equals(
        "maxTokenLen must be greater than 0 and less than 1048576 passed: 10000000",
        e.as_str(),
    );
    let mut tokenizer =
        new_letter_tokenizer_with_factory_and_max_token_len(B::new_attribute_factory(), 4800)?;
    tokenizer.set_reader(Box::new(StringCharReader::new(builder.clone())))?;
    let mut stream = LowerCaseFilter::new(boxed(tokenizer))?;
    B::assert_token_stream_contents(&mut stream, &[lower_root(&builder).as_str()])?;

    let e = iae_message(|| new_keyword_tokenizer_with_factory(B::new_attribute_factory(), 0));
    Assert::assert_equals(
        "maxTokenLen must be greater than 0 and less than 1048576 passed: 0",
        e.as_str(),
    );
    let e =
        iae_message(|| new_keyword_tokenizer_with_factory(B::new_attribute_factory(), 10_000_000));
    Assert::assert_equals(
        "maxTokenLen must be greater than 0 and less than 1048576 passed: 10000000",
        e.as_str(),
    );
    let mut tokenizer = new_keyword_tokenizer_with_factory(B::new_attribute_factory(), 4800)?;
    tokenizer.set_reader(Box::new(StringCharReader::new(builder.clone())))?;
    B::assert_token_stream_contents(&mut tokenizer, &[builder.as_str()])?;

    let e = iae_message(|| {
        new_letter_tokenizer_with_factory_and_max_token_len(B::new_attribute_factory(), 0)
    });
    Assert::assert_equals(
        "maxTokenLen must be greater than 0 and less than 1048576 passed: 0",
        e.as_str(),
    );
    let e = iae_message(|| {
        new_letter_tokenizer_with_factory_and_max_token_len(B::new_attribute_factory(), 2_000_000)
    });
    Assert::assert_equals(
        "maxTokenLen must be greater than 0 and less than 1048576 passed: 2000000",
        e.as_str(),
    );
    let mut tokenizer =
        new_letter_tokenizer_with_factory_and_max_token_len(B::new_attribute_factory(), 4800)?;
    tokenizer.set_reader(Box::new(StringCharReader::new(builder.clone())))?;
    B::assert_token_stream_contents(&mut tokenizer, &[builder.as_str()])?;

    let e = iae_message(|| {
        new_whitespace_tokenizer_with_factory_and_max_token_len(B::new_attribute_factory(), 0)
    });
    Assert::assert_equals(
        "maxTokenLen must be greater than 0 and less than 1048576 passed: 0",
        e.as_str(),
    );
    let e = iae_message(|| {
        new_whitespace_tokenizer_with_factory_and_max_token_len(
            B::new_attribute_factory(),
            3_000_000,
        )
    });
    Assert::assert_equals(
        "maxTokenLen must be greater than 0 and less than 1048576 passed: 3000000",
        e.as_str(),
    );
    let mut tokenizer =
        new_whitespace_tokenizer_with_factory_and_max_token_len(B::new_attribute_factory(), 4800)?;
    tokenizer.set_reader(Box::new(StringCharReader::new(builder.clone())))?;
    B::assert_token_stream_contents(&mut tokenizer, &[builder.as_str()])
}

/// tests the max word length of 255 with a surrogate pair at position 255
fn ct_test_max_word_length_with_supplementary() -> Result<()> {
    let mut builder = "A".repeat(254);
    builder.push('\u{1041c}');
    let mut tokenizer = new_letter_tokenizer_with_factory(B::new_attribute_factory())?;
    tokenizer.set_reader(Box::new(StringCharReader::new(format!(
        "{builder}{builder}"
    ))))?;
    let mut stream = LowerCaseFilter::new(boxed(tokenizer))?;
    let lower = lower_root(&builder);
    B::assert_token_stream_contents(&mut stream, &[lower.as_str(), lower.as_str()])
}

fn ct_test_definition_using_method_reference1() -> Result<()> {
    let mut tokenizer = from_separator_char_predicate(is_whitespace);
    tokenizer.set_reader(Box::new(StringCharReader::new("Tokenizer Test")))?;
    B::assert_token_stream_contents(&mut tokenizer, &["Tokenizer", "Test"])
}

fn ct_test_definition_using_method_reference2() -> Result<()> {
    let mut tokenizer = from_token_char_predicate(is_letter);
    tokenizer.set_reader(Box::new(StringCharReader::new("Tokenizer(Test)")))?;
    B::assert_token_stream_contents(&mut tokenizer, &["Tokenizer", "Test"])
}

fn ct_test_definition_using_lambda() -> Result<()> {
    let mut tokenizer = from_separator_char_predicate(|c| c == 0x00A0 || is_whitespace(c));
    tokenizer.set_reader(Box::new(StringCharReader::new("Tokenizer\u{00A0}Test Foo")))?;
    B::assert_token_stream_contents(&mut tokenizer, &["Tokenizer", "Test", "Foo"])
}

fn char_tokenizers_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.util.TestCharTokenizers")
        .method(t(
            "testReadSupplementaryChars",
            ct_test_read_supplementary_chars,
        ))
        .method(t("testExtendCharBuffer", ct_test_extend_char_buffer))
        .method(t("testMaxWordLength", ct_test_max_word_length))
        .method(t(
            "testCustomMaxTokenLength",
            ct_test_custom_max_token_length,
        ))
        .method(t(
            "testMaxWordLengthWithSupplementary",
            ct_test_max_word_length_with_supplementary,
        ))
        .method(t(
            "testDefinitionUsingMethodReference1",
            ct_test_definition_using_method_reference1,
        ))
        .method(t(
            "testDefinitionUsingMethodReference2",
            ct_test_definition_using_method_reference2,
        ))
        .method(t(
            "testDefinitionUsingLambda",
            ct_test_definition_using_lambda,
        ))
        .build()
}

// -----------------------------------------------------------------------------
// TestCoreFactories
// -----------------------------------------------------------------------------

const CORE: &str = "org.apache.lucene.analysis.core";

/// Test KeywordTokenizerFactory
fn cf_test_keyword_tokenizer() -> Result<()> {
    let mut stream = tokenizer_factory(CORE, "Keyword", &[])?.create()?;
    stream.set_reader(Box::new(StringCharReader::new("What's this thing do?")))?;
    B::assert_token_stream_contents(&mut stream, &["What's this thing do?"])
}

/// Test WhitespaceTokenizerFactory
fn cf_test_whitespace_tokenizer() -> Result<()> {
    let mut stream = tokenizer_factory(CORE, "Whitespace", &[])?
        .create_with_factory(B::new_attribute_factory())?;
    stream.set_reader(Box::new(StringCharReader::new("What's this thing do?")))?;
    B::assert_token_stream_contents(&mut stream, &["What's", "this", "thing", "do?"])
}

/// Test LetterTokenizerFactory
fn cf_test_letter_tokenizer() -> Result<()> {
    let mut stream =
        tokenizer_factory(CORE, "Letter", &[])?.create_with_factory(B::new_attribute_factory())?;
    stream.set_reader(Box::new(StringCharReader::new("What's this thing do?")))?;
    B::assert_token_stream_contents(&mut stream, &["What", "s", "this", "thing", "do"])
}

/// Test that bogus arguments result in exception
fn cf_test_bogus_arguments() -> Result<()> {
    let expected =
        iae_message(|| tokenizer_factory(CORE, "Whitespace", &["bogusArg", "bogusValue"]));
    Assert::assert_true(expected.contains("Unknown parameters"));
    let expected = iae_message(|| tokenizer_factory(CORE, "Letter", &["bogusArg", "bogusValue"]));
    Assert::assert_true(expected.contains("Unknown parameters"));
    Ok(())
}

fn core_factories_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.core.TestCoreFactories")
        .method(t("testKeywordTokenizer", cf_test_keyword_tokenizer))
        .method(t("testWhitespaceTokenizer", cf_test_whitespace_tokenizer))
        .method(t("testLetterTokenizer", cf_test_letter_tokenizer))
        .method(t("testBogusArguments", cf_test_bogus_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// TestDecimalDigitFilter
// -----------------------------------------------------------------------------

/// `DECIMAL_DIGIT_CODEPOINTS`, which `@BeforeClass` fills: every code point
/// `Character.isDigit` accepts.
static DECIMAL_DIGIT_CODEPOINTS: LazyLock<SparseFixedBitSet> = LazyLock::new(|| {
    // `Character.MAX_CODE_POINT`
    let mut set = SparseFixedBitSet::new(0x10FFFF).expect("a bit set");
    for codepoint in 0..0x10FFFF {
        if is_digit(codepoint) {
            set.set(codepoint as usize);
        }
    }
    assert!(0 < set.cardinality());
    set
});

/// Two analyzers a test class's `setUp` creates and `tearDown` closes.
type AnalyzerPair = (Arc<dyn Analyzer>, Arc<dyn Analyzer>);

thread_local! {
    /// The instance fields `tokenized` and `keyword` `setUp` sets.
    static DDF: RefCell<Option<AnalyzerPair>> = const { RefCell::new(None) };
}

fn ddf_set_up() -> Result<()> {
    LuceneTestCase::super_set_up();
    let tokenized = FnAnalyzer::new(
        "org.apache.lucene.analysis.core.TestDecimalDigitFilter$1",
        |_| {
            let tokenizer = Arc::new(AtomicRefCell::new(MockTokenizer::new(
                Arc::clone(&WHITESPACE),
                false,
            )?));
            let filter = new_decimal_digit_filter(shared(&tokenizer))?;
            Ok(components(tokenizer, sink(filter)))
        },
    )
    .arc();
    let keyword = FnAnalyzer::new(
        "org.apache.lucene.analysis.core.TestDecimalDigitFilter$2",
        |_| {
            let tokenizer = Arc::new(AtomicRefCell::new(new_keyword_tokenizer()));
            let filter = new_decimal_digit_filter(shared(&tokenizer))?;
            Ok(components(tokenizer, sink(filter)))
        },
    )
    .arc();
    DDF.with(|d| *d.borrow_mut() = Some((tokenized, keyword)));
    Ok(())
}

fn ddf_tear_down() -> Result<()> {
    if let Some((tokenized, keyword)) = DDF.with(|d| d.borrow_mut().take()) {
        tokenized.close();
        keyword.close();
    }
    LuceneTestCase::super_tear_down();
    Ok(())
}

fn tokenized() -> Arc<dyn Analyzer> {
    DDF.with(|d| Arc::clone(&d.borrow().as_ref().expect("setUp").0))
}

fn keyword() -> Arc<dyn Analyzer> {
    DDF.with(|d| Arc::clone(&d.borrow().as_ref().expect("setUp").1))
}

/// test that digits are normalized
fn ddf_test_simple() -> Result<()> {
    B::check_one_term(&*tokenized(), "١٢٣٤", "1234")
}

/// test that double struck digits are normalized
fn ddf_test_double_struck() -> Result<()> {
    // MATHEMATICAL DOUBLE-STRUCK DIGIT ... 1, 9, 8, 4
    let input = "𝟙 𝟡 𝟠 𝟜";
    let expected = "1 9 8 4";
    B::check_one_term(&*keyword(), input, expected)?;
    // `replaceAll("\\s", "")`: `\s` is `[ \t\n\x0B\f\r]`.
    let strip = |s: &str| -> String {
        s.chars()
            .filter(|c| !matches!(c, ' ' | '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r'))
            .collect()
    };
    B::check_one_term(&*keyword(), &strip(input), &strip(expected))
}

/// test sequences of digits mixed with other random simple string data
fn ddf_test_random_sequences() -> Result<()> {
    // test numIters random strings containing a sequence of numDigits codepoints
    let num_iters = LuceneTestCase::at_least(5);
    for _ in 0..num_iters {
        let num_digits = LuceneTestCase::at_least(20);
        let mut expected = String::new();
        let mut actual = String::new();
        for _ in 0..num_digits {
            // increased odds of 0 length random string prefix
            let prefix = if random().next_boolean() {
                String::new()
            } else {
                TestUtil::random_simple_string(&*random())
            };
            expected.push_str(&prefix);
            actual.push_str(&prefix);
            let codepoint = get_random_decimal_digit(&*random());
            let value = get_numeric_value(codepoint);
            assert!((0..=9).contains(&value));
            expected.push_str(&value.to_string());
            actual.push(char::from_u32(codepoint as u32).expect("a scalar value"));
        }
        // occasional suffix, increased odds of 0 length random string
        let suffix = if random().next_boolean() {
            String::new()
        } else {
            TestUtil::random_simple_string(&*random())
        };
        expected.push_str(&suffix);
        actual.push_str(&suffix);
        B::check_one_term(&*keyword(), &actual, &expected)?;
    }
    Ok(())
}

/// test each individual digit in different locations of strings.
fn ddf_test_random() -> Result<()> {
    let mut num_code_points_checked = 0; // sanity check
    let mut next = DECIMAL_DIGIT_CODEPOINTS.next_set_bit(0);
    while let Some(codepoint) = next {
        let codepoint = codepoint as i32;
        assert!(is_digit(codepoint));

        // add some a-z before/after the string
        let prefix = TestUtil::random_simple_string(&*random());
        let suffix = TestUtil::random_simple_string(&*random());

        let mut expected = prefix.clone();
        let value = get_numeric_value(codepoint);
        assert!((0..=9).contains(&value));
        expected.push_str(&value.to_string());
        expected.push_str(&suffix);

        let mut actual = prefix;
        actual.push(char::from_u32(codepoint as u32).expect("a scalar value"));
        actual.push_str(&suffix);

        B::check_one_term(&*keyword(), &actual, &expected)?;

        num_code_points_checked += 1;
        next = DECIMAL_DIGIT_CODEPOINTS.next_set_bit(codepoint as usize + 1);
    }
    assert_eq!(
        DECIMAL_DIGIT_CODEPOINTS.cardinality(),
        num_code_points_checked
    );
    Ok(())
}

/// check the filter is a no-op for the empty string term
fn ddf_test_empty_term() -> Result<()> {
    B::check_one_term(&*keyword(), "", "")
}

/// blast some random strings through the filter
fn ddf_test_random_strings() -> Result<()> {
    B::check_random_data(&*random(), &tokenized(), 200 * *RANDOM_MULTIPLIER)
}

/// returns a psuedo-random codepoint which is a Decimal Digit
fn get_random_decimal_digit(r: &dyn Random) -> i32 {
    let set = &*DECIMAL_DIGIT_CODEPOINTS;
    let aprox = TestUtil::next_int(r, 0, set.length() as i32 - 1);
    if set.get(aprox as usize) {
        // lucky guess
        assert!(is_digit(aprox));
        return aprox;
    }
    // seek up and down for closest set bit
    let lower = set.prev_set_bit(aprox as usize);
    let higher = set
        .next_set_bit(aprox as usize)
        .map_or(NO_MORE_DOCS, |h| h as i32);

    // sanity check edge cases
    if lower < 0 {
        assert!(higher != NO_MORE_DOCS);
        assert!(is_digit(higher));
        return higher;
    }
    if higher == NO_MORE_DOCS {
        assert!(0 <= lower);
        assert!(is_digit(lower));
        return lower;
    }

    // which is closer?
    let cmp = (aprox - lower).cmp(&(higher - aprox));

    if cmp == std::cmp::Ordering::Equal {
        // dead even, flip a coin
        let result = if random().next_boolean() {
            lower
        } else {
            higher
        };
        assert!(is_digit(result));
        return result;
    }

    let result = if cmp == std::cmp::Ordering::Less {
        lower
    } else {
        higher
    };
    assert!(is_digit(result));
    result
}

fn decimal_digit_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.core.TestDecimalDigitFilter")
        .static_init(|| {
            // `@BeforeClass init_DECIMAL_DIGIT_CODEPOINTS`
            LazyLock::force(&DECIMAL_DIGIT_CODEPOINTS);
        })
        .method(hook("setUp", ddf_set_up))
        .method(hook("tearDown", ddf_tear_down))
        .method(t("testSimple", ddf_test_simple))
        .method(t("testDoubleStruck", ddf_test_double_struck))
        .method(t("testRandomSequences", ddf_test_random_sequences))
        .method(t("testRandom", ddf_test_random))
        .method(t("testEmptyTerm", ddf_test_empty_term))
        .method(t("testRandomStrings", ddf_test_random_strings))
        .build()
}

// -----------------------------------------------------------------------------
// TestDecimalDigitFilterFactory
// -----------------------------------------------------------------------------

/// Ensure the filter actually normalizes digits.
fn ddff_test_normalization() -> Result<()> {
    let stream = B::whitespace_mock_tokenizer_str("١٢٣٤")?;
    let mut stream = token_filter_factory(CORE, "DecimalDigit", &[])?.create(boxed(stream))?;
    B::assert_token_stream_contents(&mut *stream, &["1234"])
}

/// Test that bogus arguments result in exception
fn ddff_test_bogus_arguments() -> Result<()> {
    let expected =
        iae_message(|| token_filter_factory(CORE, "DecimalDigit", &["bogusArg", "bogusValue"]));
    Assert::assert_true(expected.contains("Unknown parameters"));
    Ok(())
}

fn decimal_digit_filter_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.core.TestDecimalDigitFilterFactory")
        .method(t("testNormalization", ddff_test_normalization))
        .method(t("testBogusArguments", ddff_test_bogus_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// TestDuelingAnalyzers
// -----------------------------------------------------------------------------

/// `jvmLetter`, which `@BeforeClass` builds: an automaton matching this
/// JVM's letter definition, repeated.
static JVM_LETTER: LazyLock<Arc<CharacterRunAutomaton>> = LazyLock::new(|| {
    let mut single = Automaton::new();
    let initial = single.create_state();
    let accept = single.create_state();
    single.set_accept(accept, true);

    // build an automaton matching this jvm's letter definition
    for i in 0..=0x10FFFF {
        if is_letter(i) {
            single.add_transition(initial, accept, i);
        }
    }
    let repeat = Operations::repeat(&single);
    Arc::new(CharacterRunAutomaton::new(repeat).expect("a run automaton"))
});

/// The anonymous right-hand analyzer: a `LetterTokenizer(newAttributeFactory())`.
fn letter_analyzer(name: &'static str) -> Arc<dyn Analyzer> {
    FnAnalyzer::new(name, |_| {
        Ok(tokenizer_components(Arc::new(AtomicRefCell::new(
            new_letter_tokenizer_with_factory(B::new_attribute_factory())?,
        ))))
    })
    .arc()
}

/// `newStringReader(String s)`.
fn new_string_reader(s: &str) -> Box<dyn CharReader> {
    let random = random();
    let r: Box<dyn CharReader> = Box::new(StringCharReader::new(s));
    if random.next_boolean() {
        Box::new(MockReaderWrapper::new(random, r))
    } else {
        r
    }
}

/// `assertEquals(String s, TokenStream left, TokenStream right)`: we only
/// check a few core attributes here.
fn da_assert_equals(s: &str, left: &dyn Analyzer, right: &dyn Analyzer) -> Result<()> {
    let left = left.token_stream("foo", new_string_reader(s))?;
    let right = right.token_stream("foo", new_string_reader(s))?;
    let mut left = left.borrow_mut();
    let mut right = right.borrow_mut();
    left.reset()?;
    right.reset()?;
    for ts in [&mut *left, &mut *right] {
        let src = ts.attribute_source_mut();
        src.add_attribute_interface::<dyn CharTermAttribute>()?;
        src.add_attribute_interface::<dyn OffsetAttribute>()?;
        src.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
    }
    let term = |ts: &dyn TokenStream| -> String {
        let a = ts
            .attribute_source()
            .get_attribute_interface::<dyn CharTermAttribute>()
            .unwrap();
        a.buffer()[..a.length()].iter().collect()
    };
    let pos = |ts: &dyn TokenStream| -> i32 {
        ts.attribute_source()
            .get_attribute_interface::<dyn PositionIncrementAttribute>()
            .unwrap()
            .get_position_increment()
    };
    let offsets = |ts: &dyn TokenStream| -> (i32, i32) {
        let a = ts
            .attribute_source()
            .get_attribute_interface::<dyn OffsetAttribute>()
            .unwrap();
        (a.start_offset(), a.end_offset())
    };
    while left.increment_token()? {
        Assert::assert_true_msg(
            &format!("wrong number of tokens for input: {s}"),
            right.increment_token()?,
        );
        Assert::assert_equals_msg(
            Some(&format!("wrong term text for input: {s}")),
            term(&*left),
            term(&*right),
        );
        Assert::assert_equals_msg(
            Some(&format!("wrong position for input: {s}")),
            pos(&*left),
            pos(&*right),
        );
        Assert::assert_equals_msg(
            Some(&format!("wrong start offset for input: {s}")),
            offsets(&*left).0,
            offsets(&*right).0,
        );
        Assert::assert_equals_msg(
            Some(&format!("wrong end offset for input: {s}")),
            offsets(&*left).1,
            offsets(&*right).1,
        );
    }
    Assert::assert_false_msg(
        &format!("wrong number of tokens for input: {s}"),
        right.increment_token()?,
    );
    left.end()?;
    right.end()?;
    Assert::assert_equals_msg(
        Some(&format!("wrong final offset for input: {s}")),
        offsets(&*left).1,
        offsets(&*right).1,
    );
    left.close()?;
    right.close()
}

fn da_test_letter_ascii() -> Result<()> {
    let random = random();
    let left = MockAnalyzer::with_automaton(&*random, Arc::clone(&JVM_LETTER), false);
    let right = letter_analyzer("org.apache.lucene.analysis.core.TestDuelingAnalyzers$1");
    for _ in 0..200 {
        let s = TestUtil::random_simple_string(&*random);
        da_assert_equals(&s, &left, &*right)?;
    }
    left.close();
    right.close();
    Ok(())
}

// not so useful since it's all one token?!
fn da_test_letter_ascii_huge() -> Result<()> {
    let random = random();
    let max_length = 8192; // CharTokenizer.IO_BUFFER_SIZE*2
    let left = MockAnalyzer::with_automaton(&*random, Arc::clone(&JVM_LETTER), false);
    left.set_max_token_length(255); // match CharTokenizer's max token length
    let right = letter_analyzer("org.apache.lucene.analysis.core.TestDuelingAnalyzers$2");
    let num_iterations = LuceneTestCase::at_least(10);
    for _ in 0..num_iterations {
        let s = TestUtil::random_simple_string_max(&*random, max_length);
        da_assert_equals(&s, &left, &*right)?;
    }
    left.close();
    right.close();
    Ok(())
}

fn da_test_letter_htmlish() -> Result<()> {
    let random = random();
    let left = MockAnalyzer::with_automaton(&*random, Arc::clone(&JVM_LETTER), false);
    let right = letter_analyzer("org.apache.lucene.analysis.core.TestDuelingAnalyzers$3");
    for _ in 0..200 {
        let s = TestUtil::random_htmlish_string(&*random, 20);
        da_assert_equals(&s, &left, &*right)?;
    }
    left.close();
    right.close();
    Ok(())
}

fn da_test_letter_htmlish_huge() -> Result<()> {
    let random = random();
    let max_length = 1024; // this is number of elements, not chars!
    let left = MockAnalyzer::with_automaton(&*random, Arc::clone(&JVM_LETTER), false);
    left.set_max_token_length(255); // match CharTokenizer's max token length
    let right = letter_analyzer("org.apache.lucene.analysis.core.TestDuelingAnalyzers$4");
    let num_iterations = LuceneTestCase::at_least(10);
    for _ in 0..num_iterations {
        let s = TestUtil::random_htmlish_string(&*random, max_length);
        da_assert_equals(&s, &left, &*right)?;
    }
    left.close();
    right.close();
    Ok(())
}

fn da_test_letter_unicode() -> Result<()> {
    let random = random();
    let left =
        MockAnalyzer::with_automaton(&*LuceneTestCase::random(), Arc::clone(&JVM_LETTER), false);
    let right = letter_analyzer("org.apache.lucene.analysis.core.TestDuelingAnalyzers$5");
    for _ in 0..200 {
        let s = TestUtil::random_unicode_string(&*random);
        da_assert_equals(&s, &left, &*right)?;
    }
    left.close();
    right.close();
    Ok(())
}

fn da_test_letter_unicode_huge() -> Result<()> {
    let random = random();
    let max_length = 4300; // CharTokenizer.IO_BUFFER_SIZE + fudge
    let left = MockAnalyzer::with_automaton(&*random, Arc::clone(&JVM_LETTER), false);
    left.set_max_token_length(255); // match CharTokenizer's max token length
    let right = letter_analyzer("org.apache.lucene.analysis.core.TestDuelingAnalyzers$6");
    let num_iterations = LuceneTestCase::at_least(10);
    for _ in 0..num_iterations {
        let s = TestUtil::random_unicode_string_max(&*random, max_length);
        da_assert_equals(&s, &left, &*right)?;
    }
    left.close();
    right.close();
    Ok(())
}

fn dueling_analyzers_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.core.TestDuelingAnalyzers")
        .static_init(|| {
            // `@BeforeClass beforeClass()`
            LazyLock::force(&JVM_LETTER);
        })
        .method(t("testLetterAscii", da_test_letter_ascii))
        .method(t("testLetterAsciiHuge", da_test_letter_ascii_huge))
        .method(t("testLetterHtmlish", da_test_letter_htmlish))
        .method(t("testLetterHtmlishHuge", da_test_letter_htmlish_huge))
        .method(t("testLetterUnicode", da_test_letter_unicode))
        .method(t("testLetterUnicodeHuge", da_test_letter_unicode_huge))
        .build()
}

// -----------------------------------------------------------------------------
// TestKeywordAnalyzer
// -----------------------------------------------------------------------------

type KaState = (
    rucene_test_framework::util::lucene_test_case::TestDirectory,
    Arc<dyn rucene::index::DirectoryReader>,
    Arc<dyn Analyzer>,
);

thread_local! {
    /// The instance fields `directory`, `reader` and `analyzer`.
    static KA: RefCell<Option<KaState>> = const { RefCell::new(None) };
}

fn ka_set_up() -> Result<()> {
    LuceneTestCase::super_set_up();
    let directory = LuceneTestCase::new_directory();
    let analyzer: Arc<dyn Analyzer> = Arc::new(SimpleAnalyzer::new());
    let writer = IndexWriter::new(
        directory.as_directory(),
        IndexWriterConfig::with_analyzer(Some(Arc::clone(&analyzer))),
    )?;
    let mut doc = Document::new();
    doc.add(Box::new(StringField::new(
        "partnum",
        "Q36".to_string(),
        Store::YES,
    )?));
    doc.add(Box::new(TextField::new(
        "description",
        "Illidium Space Modulator".to_string(),
        Store::YES,
    )?));
    writer.add_document(&doc)?;
    writer.close()?;
    let reader = directory_reader::open(directory.as_directory())?;
    KA.with(|k| *k.borrow_mut() = Some((directory, reader, analyzer)));
    Ok(())
}

fn ka_tear_down() -> Result<()> {
    // `IOUtils.close(analyzer, reader, directory)`
    if let Some((directory, reader, analyzer)) = KA.with(|k| k.borrow_mut().take()) {
        analyzer.close();
        reader.close()?;
        directory.close()?;
    }
    LuceneTestCase::super_tear_down();
    Ok(())
}

fn ka_test_mutiple_document() -> Result<()> {
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let analyzer: Arc<dyn Analyzer> = Arc::new(KeywordAnalyzer::new());
    let writer = IndexWriter::new(
        Arc::clone(&dir),
        IndexWriterConfig::with_analyzer(Some(Arc::clone(&analyzer))),
    )?;
    let mut doc = Document::new();
    doc.add(Box::new(TextField::new(
        "partnum",
        "Q36".to_string(),
        Store::YES,
    )?));
    writer.add_document(&doc)?;
    let mut doc = Document::new();
    doc.add(Box::new(TextField::new(
        "partnum",
        "Q37".to_string(),
        Store::YES,
    )?));
    writer.add_document(&doc)?;
    writer.close()?;

    let reader: Arc<dyn IndexReader> = directory_reader::open(dir)?;
    let mut td = TestUtil::docs(
        &*random(),
        &reader,
        "partnum",
        &BytesRef::from_char_sequence("Q36"),
        None,
        0,
    )?
    .expect("postings");
    Assert::assert_true(td.next_doc()? != NO_MORE_DOCS);
    let mut td = TestUtil::docs(
        &*random(),
        &reader,
        "partnum",
        &BytesRef::from_char_sequence("Q37"),
        None,
        0,
    )?
    .expect("postings");
    Assert::assert_true(td.next_doc()? != NO_MORE_DOCS);
    analyzer.close();
    Ok(())
}

// LUCENE-1441
fn ka_test_offsets() -> Result<()> {
    let analyzer = KeywordAnalyzer::new();
    let stream = analyzer.token_stream("field", Box::new(StringCharReader::new("abcd")))?;
    let mut stream = stream.borrow_mut();
    stream
        .attribute_source_mut()
        .add_attribute_interface::<dyn OffsetAttribute>()?;
    stream.reset()?;
    Assert::assert_true(stream.increment_token()?);
    {
        let offset_att = stream
            .attribute_source()
            .get_attribute_interface::<dyn OffsetAttribute>()
            .unwrap();
        Assert::assert_equals(0, offset_att.start_offset());
        Assert::assert_equals(4, offset_att.end_offset());
    }
    Assert::assert_false(stream.increment_token()?);
    stream.end()?;
    stream.close()?;
    drop(stream);
    analyzer.close();
    Ok(())
}

/// blast some random strings through the analyzer
fn ka_test_random_strings() -> Result<()> {
    let analyzer: Arc<dyn Analyzer> = Arc::new(KeywordAnalyzer::new());
    B::check_random_data(&*random(), &analyzer, 200 * *RANDOM_MULTIPLIER)?;
    analyzer.close();
    Ok(())
}

fn keyword_analyzer_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.core.TestKeywordAnalyzer")
        .method(hook("setUp", ka_set_up))
        .method(hook("tearDown", ka_tear_down))
        .method(t("testMutipleDocument", ka_test_mutiple_document))
        .method(t("testOffsets", ka_test_offsets))
        .method(t("testRandomStrings", ka_test_random_strings))
        .build()
}

// -----------------------------------------------------------------------------
// TestKeywordTokenizer
// -----------------------------------------------------------------------------

fn kt_test_simple() -> Result<()> {
    let mut tokenizer = new_keyword_tokenizer();
    tokenizer.set_reader(Box::new(StringCharReader::new("Tokenizer \u{1041c}test")))?;
    B::assert_token_stream_contents(&mut tokenizer, &["Tokenizer \u{1041c}test"])
}

fn kt_test_factory() -> Result<()> {
    let mut args = HashMap::new();
    let factory = KeywordTokenizerFactory::new(&mut args)?;
    let attribute_factory = B::new_attribute_factory();
    let tokenizer = factory.create_with_factory(attribute_factory)?;
    assert_tokenizer_class("KeywordTokenizer", &tokenizer);
    Ok(())
}

fn kt_test_params_factory() -> Result<()> {
    // negative maxTokenLen
    let iae = iae_message(|| KeywordTokenizerFactory::new(&mut make_args(&["maxTokenLen", "-1"])));
    Assert::assert_equals(
        "maxTokenLen must be greater than 0 and less than 1048576 passed: -1",
        iae.as_str(),
    );

    // zero maxTokenLen
    let iae = iae_message(|| KeywordTokenizerFactory::new(&mut make_args(&["maxTokenLen", "0"])));
    Assert::assert_equals(
        "maxTokenLen must be greater than 0 and less than 1048576 passed: 0",
        iae.as_str(),
    );

    // Added random param, should throw illegal error
    let iae = iae_message(|| {
        KeywordTokenizerFactory::new(&mut make_args(&[
            "maxTokenLen",
            "255",
            "randomParam",
            "rValue",
        ]))
    });
    Assert::assert_equals("Unknown parameters: {randomParam=rValue}", iae.as_str());

    // tokeniser will never split, no matter what is passed,
    // but the buffer will not be more than length of the token
    let factory = KeywordTokenizerFactory::new(&mut make_args(&["maxTokenLen", "5"]))?;
    let mut tokenizer = factory.create_with_factory(B::new_attribute_factory())?;
    tokenizer.set_reader(Box::new(StringCharReader::new("Tokenizertest")))?;
    B::assert_token_stream_contents(&mut tokenizer, &["Tokenizertest"])?;

    // tokeniser will never split, no matter what is passed,
    // but the buffer will not be more than length of the token
    let factory = KeywordTokenizerFactory::new(&mut make_args(&["maxTokenLen", "2"]))?;
    let mut tokenizer = factory.create_with_factory(B::new_attribute_factory())?;
    tokenizer.set_reader(Box::new(StringCharReader::new("Tokenizer\u{00A0}test")))?;
    B::assert_token_stream_contents(&mut tokenizer, &["Tokenizer\u{00A0}test"])
}

fn keyword_tokenizer_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.core.TestKeywordTokenizer")
        .method(t("testSimple", kt_test_simple))
        .method(t("testFactory", kt_test_factory))
        .method(t("testParamsFactory", kt_test_params_factory))
        .build()
}

// -----------------------------------------------------------------------------
// TestTypeTokenFilter
// -----------------------------------------------------------------------------

fn as_set(items: &[&str]) -> HashSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn ttf_test_type_filter() -> Result<()> {
    let stop_types = as_set(&["<NUM>"]);
    let mut input = new_standard_tokenizer_with_factory(B::new_attribute_factory())?;
    input.set_reader(Box::new(StringCharReader::new(
        "121 is palindrome, while 123 is not",
    )))?;
    let mut stream = new_type_token_filter(boxed(input), stop_types, false)?;
    B::assert_token_stream_contents(&mut stream, &["is", "palindrome", "while", "is", "not"])
}

/// Test Position increments applied by TypeTokenFilter with and without
/// enabling this option.
fn ttf_test_stop_positons() -> Result<()> {
    let mut sb = String::new();
    for i in 10..20 {
        if i % 3 != 0 {
            sb.push_str(&format!("{i} "));
        } else {
            let w = English::int_to_english(i).trim().to_string();
            sb.push_str(&format!("{w} "));
        }
    }
    ttf_log(&sb);
    let stop_set = as_set(&["<NUM>"]);
    // with increments
    let mut input = new_standard_tokenizer();
    input.set_reader(Box::new(StringCharReader::new(sb)))?;
    let mut type_token_filter = new_type_token_filter(boxed(input), stop_set, false)?;
    ttf_test_positons(&mut type_token_filter)
}

fn ttf_test_positons(stpf: &mut dyn TokenStream) -> Result<()> {
    stpf.reset()?;
    while stpf.increment_token()? {
        let src = stpf.attribute_source();
        let term = src
            .get_attribute_interface::<dyn CharTermAttribute>()
            .unwrap();
        let type_att = src.get_attribute_interface::<dyn TypeAttribute>().unwrap();
        let pos_incr_att = src
            .get_attribute_interface::<dyn PositionIncrementAttribute>()
            .unwrap();
        ttf_log(&format!(
            "Token: {}: {} - {}",
            term.buffer()[..term.length()].iter().collect::<String>(),
            type_att.type_value(),
            pos_incr_att.get_position_increment()
        ));
        Assert::assert_equals_msg(
            Some(
                "if position increment is enabled the positionIncrementAttribute value should be 3, otherwise 1",
            ),
            pos_incr_att.get_position_increment(),
            3,
        );
    }
    stpf.end()?;
    stpf.close()
}

fn ttf_test_type_filter_whitelist() -> Result<()> {
    let stop_types = as_set(&["<NUM>"]);
    let mut input = new_standard_tokenizer_with_factory(B::new_attribute_factory())?;
    input.set_reader(Box::new(StringCharReader::new(
        "121 is palindrome, while 123 is not",
    )))?;
    let mut stream = new_type_token_filter(boxed(input), stop_types, true)?;
    B::assert_token_stream_contents(&mut stream, &["121", "123"])
}

// print debug info depending on VERBOSE
fn ttf_log(s: &str) {
    if *VERBOSE {
        println!("{s}");
    }
}

fn type_token_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.core.TestTypeTokenFilter")
        .method(t("testTypeFilter", ttf_test_type_filter))
        .method(t("testStopPositons", ttf_test_stop_positons))
        .method(t("testTypeFilterWhitelist", ttf_test_type_filter_whitelist))
        .build()
}

// -----------------------------------------------------------------------------
// TestTypeTokenFilterFactory
// -----------------------------------------------------------------------------

/// `(TypeTokenFilterFactory) tokenFilterFactory("Type", keysAndValues...)`:
/// the factory built by its `Map` constructor with `luceneMatchVersion`
/// (`Version.LATEST`) and informed with the test class's resource loader,
/// as `analysisFactory` does; built directly, since the port's lookup
/// returns a trait object that cannot be cast back.
fn type_factory(keys_and_values: &[&str]) -> Result<TypeTokenFilterFactory> {
    let mut args = make_args(keys_and_values);
    args.insert("luceneMatchVersion".into(), Version::LATEST.to_string());
    let mut factory = TypeTokenFilterFactory::new(&mut args)?;
    factory.inform(&test_class_loader(CORE))?;
    Ok(factory)
}

fn ttff_test_inform() -> Result<()> {
    let factory = type_factory(&["types", "stoptypes-1.txt"])?;
    let types = factory.get_stop_types();
    Assert::assert_true_msg("types is null and it shouldn't be", types.is_some());
    let size = types.map_or(0, |t| t.len());
    Assert::assert_true_msg(&format!("types Size: {size} is not: 2"), size == 2);

    let factory = type_factory(&[
        "types",
        "stoptypes-1.txt, stoptypes-2.txt",
        "useWhitelist",
        "true",
    ])?;
    let types = factory.get_stop_types();
    Assert::assert_true_msg("types is null and it shouldn't be", types.is_some());
    let size = types.map_or(0, |t| t.len());
    Assert::assert_true_msg(&format!("types Size: {size} is not: 4"), size == 4);
    Ok(())
}

fn ttff_test_creation_with_black_list() -> Result<()> {
    let factory =
        token_filter_factory(CORE, "Type", &["types", "stoptypes-1.txt, stoptypes-2.txt"])?;
    let input = CannedTokenStream::new(Vec::new());
    factory.create(boxed(input))?;
    Ok(())
}

fn ttff_test_creation_with_white_list() -> Result<()> {
    let factory = token_filter_factory(
        CORE,
        "Type",
        &[
            "types",
            "stoptypes-1.txt, stoptypes-2.txt",
            "useWhitelist",
            "true",
        ],
    )?;
    let input = CannedTokenStream::new(Vec::new());
    factory.create(boxed(input))?;
    Ok(())
}

fn ttff_test_missing_types_parameter() -> Result<()> {
    // not supplying 'types' parameter should cause an IllegalArgumentException
    expect_throws("java.lang.IllegalArgumentException", || {
        token_filter_factory(CORE, "Type", &[])
    });
    Ok(())
}

/// Test that bogus arguments result in exception
fn ttff_test_bogus_arguments() -> Result<()> {
    let expected = iae_message(|| {
        token_filter_factory(
            CORE,
            "Type",
            &["types", "stoptypes-1.txt", "bogusArg", "bogusValue"],
        )
    });
    Assert::assert_true(expected.contains("Unknown parameters"));
    Ok(())
}

fn type_token_filter_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.core.TestTypeTokenFilterFactory")
        .method(t("testInform", ttff_test_inform))
        .method(t(
            "testCreationWithBlackList",
            ttff_test_creation_with_black_list,
        ))
        .method(t(
            "testCreationWithWhiteList",
            ttff_test_creation_with_white_list,
        ))
        .method(t(
            "testMissingTypesParameter",
            ttff_test_missing_types_parameter,
        ))
        .method(t("testBogusArguments", ttff_test_bogus_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// TestUnicodeWhitespaceTokenizer
// -----------------------------------------------------------------------------

// clone of test from WhitespaceTokenizer
fn uwt_test_simple() -> Result<()> {
    let mut tokenizer = new_unicode_whitespace_tokenizer();
    tokenizer.set_reader(Box::new(StringCharReader::new("Tokenizer \u{1041c}test")))?;
    B::assert_token_stream_contents(&mut tokenizer, &["Tokenizer", "\u{1041c}test"])
}

fn uwt_test_nbsp() -> Result<()> {
    let mut tokenizer = new_unicode_whitespace_tokenizer();
    tokenizer.set_reader(Box::new(StringCharReader::new("Tokenizer\u{00A0}test")))?;
    B::assert_token_stream_contents(&mut tokenizer, &["Tokenizer", "test"])
}

fn uwt_test_factory() -> Result<()> {
    let mut args = HashMap::new();
    args.insert("rule".to_string(), "unicode".to_string());
    let factory = WhitespaceTokenizerFactory::new(&mut args)?;
    let tokenizer = factory.create_with_factory(B::new_attribute_factory())?;
    assert_tokenizer_class("UnicodeWhitespaceTokenizer", &tokenizer);
    Ok(())
}

fn uwt_test_params_factory() -> Result<()> {
    // negative maxTokenLen
    let iae = iae_message(|| {
        WhitespaceTokenizerFactory::new(&mut make_args(&["rule", "unicode", "maxTokenLen", "-1"]))
    });
    Assert::assert_equals(
        "maxTokenLen must be greater than 0 and less than 1048576 passed: -1",
        iae.as_str(),
    );

    // zero maxTokenLen
    let iae = iae_message(|| {
        WhitespaceTokenizerFactory::new(&mut make_args(&["rule", "unicode", "maxTokenLen", "0"]))
    });
    Assert::assert_equals(
        "maxTokenLen must be greater than 0 and less than 1048576 passed: 0",
        iae.as_str(),
    );

    // Added random param, should throw illegal error
    let iae = iae_message(|| {
        WhitespaceTokenizerFactory::new(&mut make_args(&[
            "rule",
            "unicode",
            "maxTokenLen",
            "255",
            "randomParam",
            "rValue",
        ]))
    });
    Assert::assert_equals("Unknown parameters: {randomParam=rValue}", iae.as_str());

    // tokeniser will split at 5, Token | izer, no matter what happens
    let factory =
        WhitespaceTokenizerFactory::new(&mut make_args(&["rule", "unicode", "maxTokenLen", "5"]))?;
    let mut tokenizer = factory.create_with_factory(B::new_attribute_factory())?;
    tokenizer.set_reader(Box::new(StringCharReader::new("Tokenizer \u{1041c}test")))?;
    B::assert_token_stream_contents(&mut tokenizer, &["Token", "izer", "\u{1041c}tes", "t"])?;

    // tokeniser will split at 2, To | ke | ni | ze | r, no matter what happens
    let factory =
        WhitespaceTokenizerFactory::new(&mut make_args(&["rule", "unicode", "maxTokenLen", "2"]))?;
    let mut tokenizer = factory.create_with_factory(B::new_attribute_factory())?;
    tokenizer.set_reader(Box::new(StringCharReader::new("Tokenizer\u{00A0}test")))?;
    B::assert_token_stream_contents(&mut tokenizer, &["To", "ke", "ni", "ze", "r", "te", "st"])?;

    // tokeniser will split at 10, no matter what happens,
    // but tokens' length are less than that
    let factory =
        WhitespaceTokenizerFactory::new(&mut make_args(&["rule", "unicode", "maxTokenLen", "10"]))?;
    let mut tokenizer = factory.create_with_factory(B::new_attribute_factory())?;
    tokenizer.set_reader(Box::new(StringCharReader::new("Tokenizer\u{00A0}test")))?;
    B::assert_token_stream_contents(&mut tokenizer, &["Tokenizer", "test"])
}

fn unicode_whitespace_tokenizer_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.core.TestUnicodeWhitespaceTokenizer")
        .method(t("testSimple", uwt_test_simple))
        .method(t("testNBSP", uwt_test_nbsp))
        .method(t("testFactory", uwt_test_factory))
        .method(t("testParamsFactory", uwt_test_params_factory))
        .build()
}

// -----------------------------------------------------------------------------
// TestWhitespaceAnalyzer
// -----------------------------------------------------------------------------

const ALPHABET: &str = "abcdefghijklmnopqrstuvwxyz";

/// `LONGTOKEN`: the alphabet, 12 times (312 chars).
fn longtoken() -> String {
    ALPHABET.repeat(12)
}

fn wa_test_default_maximum_token_length() -> Result<()> {
    let a = WhitespaceAnalyzer::new();
    let first = format!("{}abcdefghijklmnopqrstu", ALPHABET.repeat(9));
    B::assert_analyzes_to(
        &a,
        &format!("{} extra", longtoken()),
        &[
            first.as_str(),
            "vwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz",
            "extra",
        ],
    )?;
    a.close();
    Ok(())
}

fn wa_test_custom_maximum_token_length() -> Result<()> {
    let a = WhitespaceAnalyzer::with_max_token_length(1024);
    B::assert_analyzes_to(
        &a,
        &format!("{} extra", longtoken()),
        &[longtoken().as_str(), "extra"],
    )?;
    a.close();
    Ok(())
}

fn whitespace_analyzer_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.core.TestWhitespaceAnalyzer")
        .method(t(
            "testDefaultMaximumTokenLength",
            wa_test_default_maximum_token_length,
        ))
        .method(t(
            "testCustomMaximumTokenLength",
            wa_test_custom_maximum_token_length,
        ))
        .build()
}

// -----------------------------------------------------------------------------
// The suites
// -----------------------------------------------------------------------------

#[test]
fn test_char_tokenizers() {
    run(char_tokenizers_class);
}

#[test]
fn test_core_factories() {
    run(core_factories_class);
}

#[test]
fn test_decimal_digit_filter() {
    run(decimal_digit_filter_class);
}

#[test]
fn test_decimal_digit_filter_factory() {
    run(decimal_digit_filter_factory_class);
}

#[test]
fn test_dueling_analyzers() {
    run(dueling_analyzers_class);
}

#[test]
fn test_keyword_analyzer() {
    run(keyword_analyzer_class);
}

#[test]
fn test_keyword_tokenizer() {
    run(keyword_tokenizer_class);
}

#[test]
fn test_type_token_filter() {
    run(type_token_filter_class);
}

#[test]
fn test_type_token_filter_factory() {
    run(type_token_filter_factory_class);
}

#[test]
fn test_unicode_whitespace_tokenizer() {
    run(unicode_whitespace_tokenizer_class);
}

#[test]
fn test_whitespace_analyzer() {
    run(whitespace_analyzer_class);
}
