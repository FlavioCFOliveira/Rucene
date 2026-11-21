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
//! that rmp task #452 lists, each run as a suite by the ported runner: the
//! `org.apache.lucene.analysis.en` classes `TestEnglishAnalyzer`,
//! `TestEnglishMinimalStemFilter`, `TestEnglishMinimalStemFilterFactory`,
//! `TestKStemFilterFactory`, `TestKStemmer`, `TestPorterStemFilter` and
//! `TestPorterStemFilterFactory`, the `org.apache.lucene.analysis.core`
//! classes `TestAnalyzers`, `TestStopAnalyzer` and `TestStopFilterFactory`,
//! and the `org.apache.lucene.analysis.miscellaneous` classes
//! `TestStemmerOverrideFilter`, `TestWordDelimiterFilter` and
//! `TestWordDelimiterGraphFilter` (all use `analysis.en` types).
//!
//! Not ported, as Lucene does not run them either: `TestKStemmer.testCreateMap`
//! (`@Ignore`, empty body) and `TestAnalyzers._testStandardConstants` (not a
//! `test*` method). The assertions of `TestAnalyzers.testLowerCaseFilter` and
//! `testUpperCaseFilter` on unpaired surrogates, and
//! `testLowerCaseFilterLowSurrogateLeftover` (whose inputs end in unpaired
//! surrogates), cannot be expressed: a Rust string holds no lone surrogate
//! and Rucene's `CharTermAttribute` holds `char`s (the divergence declared on
//! `rucene::analysis::CharTermAttribute`).
//!
//! `TestWordDelimiterFilter` is `@Deprecated` and exercises the deprecated
//! `WordDelimiterFilter`, as in Lucene: deprecation warnings are allowed here.

#![allow(deprecated)]

mod support;

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::character_utils::to_lower_case;
use rucene::analysis::stop_filter::new_stop_filter;
use rucene::analysis::tokenattributes::{
    CharTermAttribute, PayloadAttribute, PositionIncrementAttribute,
};
use rucene::analysis::{
    Analyzer, CharArraySet, StringCharReader, TokenFilter, TokenFilterLogic, TokenStream,
};
use rucene::error::Result;
use rucene::index::index_writer::MAX_TERM_LENGTH;
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::java_collections::JavaHashMap;
use rucene::util::{AttributeSource, BytesRef};
use rucene_analysis_common::analysis::core::keyword_tokenizer::new_keyword_tokenizer;
use rucene_analysis_common::analysis::core::lower_case_filter::LowerCaseFilter;
use rucene_analysis_common::analysis::core::simple_analyzer::SimpleAnalyzer;
use rucene_analysis_common::analysis::core::stop_analyzer::StopAnalyzer;
use rucene_analysis_common::analysis::core::stop_filter_factory::StopFilterFactory;
use rucene_analysis_common::analysis::core::unicode_whitespace_analyzer::UnicodeWhitespaceAnalyzer;
use rucene_analysis_common::analysis::core::unicode_whitespace_tokenizer::new_unicode_whitespace_tokenizer;
use rucene_analysis_common::analysis::core::upper_case_filter::new_upper_case_filter;
use rucene_analysis_common::analysis::core::whitespace_analyzer::WhitespaceAnalyzer;
use rucene_analysis_common::analysis::core::whitespace_tokenizer::new_whitespace_tokenizer;
use rucene_analysis_common::analysis::en::english_analyzer::{
    EnglishAnalyzer, ENGLISH_STOP_WORDS_SET,
};
use rucene_analysis_common::analysis::en::english_minimal_stem_filter::new_english_minimal_stem_filter;
use rucene_analysis_common::analysis::en::kstem_filter::new_kstem_filter;
use rucene_analysis_common::analysis::en::porter_stem_filter::new_porter_stem_filter;
use rucene_analysis_common::analysis::miscellaneous::keyword_marker_filter::{
    new_keyword_marker_filter, KeywordMarkerFilterLogic,
};
use rucene_analysis_common::analysis::miscellaneous::set_keyword_marker_filter::new_set_keyword_marker_filter;
use rucene_analysis_common::analysis::miscellaneous::stemmer_override_filter::{
    new_stemmer_override_filter, Builder as StemmerOverrideBuilder,
};
use rucene_analysis_common::analysis::miscellaneous::word_delimiter_filter as wdf;
use rucene_analysis_common::analysis::miscellaneous::word_delimiter_graph_filter as wdgf;
use rucene_analysis_common::analysis::miscellaneous::word_delimiter_iterator::DEFAULT_WORD_DELIM_TABLE;
use rucene_analysis_common::jdk::character;
use rucene_test_framework::analysis::mock_analyzer::BoxedStream;
use rucene_test_framework::analysis::mock_tokenizer::{MockTokenizer, KEYWORD, WHITESPACE};
use rucene_test_framework::analysis::token::Token;
use rucene_test_framework::analysis::vocabulary_assert::VocabularyAssert;
use rucene_test_framework::analysis::{
    BaseTokenStreamFactoryTestCase, BaseTokenStreamTestCase, CannedTokenStream,
};
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::{RANDOM_MULTIPLIER, VERBOSE};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};
use support::{
    boxed, btstc_class, concrete_factory, factory_class, hook, iae_message,
    keyword_tokenizer_analyzer, mock_tokenizer_analyzer, run, t, token_filter_factory,
    whitespace_analyzer, FnAnalyzer,
};

type B = BaseTokenStreamTestCase;
#[allow(dead_code)]
type F = BaseTokenStreamFactoryTestCase;

/// The test package of the English classes.
const EN: &str = "org.apache.lucene.analysis.en";

/// The test package of `TestStopFilterFactory`.
const CORE: &str = "org.apache.lucene.analysis.core";

/// `random()`.
fn random() -> Arc<dyn Random> {
    LuceneTestCase::random()
}

/// `RANDOM_MULTIPLIER`.
fn mult() -> i32 {
    *RANDOM_MULTIPLIER
}

/// `new CharArraySet(asSet(words...), ignoreCase)`.
fn char_set(words: &[&str], ignore_case: bool) -> CharArraySet {
    CharArraySet::from_iter(words.iter().copied(), ignore_case)
}

/// `getDataPath(name)` of a test class of the `en` package: its resources
/// are copied byte for byte under `tests/resources`.
fn en_data_path(name: &str) -> Result<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/resources/org/apache/lucene/analysis/en");
    LuceneTestCase::get_data_path(&dir, name)
}

/// `termAtt.toString()`.
fn term_of(source: &AttributeSource) -> String {
    let a = source
        .get_attribute_interface::<dyn CharTermAttribute>()
        .expect("termAtt");
    a.buffer()[..a.length()].iter().collect()
}

thread_local! {
    /// The analyzer field a test class's `setUp` sets.
    static ANALYZER: RefCell<Option<Arc<dyn Analyzer>>> = const { RefCell::new(None) };
}

fn analyzer() -> Arc<dyn Analyzer> {
    ANALYZER.with(|a| Arc::clone(a.borrow().as_ref().expect("setUp")))
}

fn set_analyzer(a: Arc<dyn Analyzer>) {
    ANALYZER.with(|f| *f.borrow_mut() = Some(a));
}

/// The common `tearDown`: closes the analyzer `setUp` made.
fn close_analyzer_tear_down() -> Result<()> {
    if let Some(a) = ANALYZER.with(|a| a.borrow_mut().take()) {
        a.close();
    }
    LuceneTestCase::super_tear_down();
    Ok(())
}

// -----------------------------------------------------------------------------
// TestEnglishAnalyzer
// -----------------------------------------------------------------------------

/// This test fails with NPE when the stopwords file is missing in classpath
fn ea_test_resources_available() -> Result<()> {
    EnglishAnalyzer::new().close();
    Ok(())
}

/// test stopwords and stemming
fn ea_test_basics() -> Result<()> {
    let a = EnglishAnalyzer::new();
    // stemming
    B::check_one_term(&a, "books", "book")?;
    B::check_one_term(&a, "book", "book")?;
    // stopword
    B::assert_analyzes_to(&a, "the", &[])?;
    // possessive removal
    B::check_one_term(&a, "steven's", "steven")?;
    B::check_one_term(&a, "steven\u{2019}s", "steven")?;
    B::check_one_term(&a, "steven\u{FF07}s", "steven")?;
    a.close();
    Ok(())
}

/// test use of exclusion set
fn ea_test_exclude() -> Result<()> {
    let exclusion_set = char_set(&["books"], false);
    let a = EnglishAnalyzer::with_stem_exclusion_set(
        EnglishAnalyzer::get_default_stop_set().clone(),
        &exclusion_set,
    );
    B::check_one_term(&a, "books", "books")?;
    B::check_one_term(&a, "book", "book")?;
    a.close();
    Ok(())
}

/// blast some random strings through the analyzer
fn ea_test_random_strings() -> Result<()> {
    let a: Arc<dyn Analyzer> = Arc::new(EnglishAnalyzer::new());
    B::check_random_data(&*random(), &a, 200 * mult())?;
    a.close();
    Ok(())
}

fn english_analyzer_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.en.TestEnglishAnalyzer")
        .method(t("testResourcesAvailable", ea_test_resources_available))
        .method(t("testBasics", ea_test_basics))
        .method(t("testExclude", ea_test_exclude))
        .method(t("testRandomStrings", ea_test_random_strings))
        .build()
}

// -----------------------------------------------------------------------------
// TestEnglishMinimalStemFilter
// -----------------------------------------------------------------------------

fn ems_set_up() -> Result<()> {
    LuceneTestCase::super_set_up();
    set_analyzer(whitespace_analyzer(
        "org.apache.lucene.analysis.en.TestEnglishMinimalStemFilter$1",
        |t| Ok(boxed(new_english_minimal_stem_filter(t)?)),
    ));
    Ok(())
}

/// Test some examples from various papers about this technique
fn ems_test_examples() -> Result<()> {
    let a = analyzer();
    B::check_one_term(&*a, "queries", "query")?;
    B::check_one_term(&*a, "phrases", "phrase")?;
    B::check_one_term(&*a, "corpus", "corpus")?;
    B::check_one_term(&*a, "stress", "stress")?;
    B::check_one_term(&*a, "kings", "king")?;
    B::check_one_term(&*a, "panels", "panel")?;
    B::check_one_term(&*a, "aerodynamics", "aerodynamic")?;
    B::check_one_term(&*a, "congress", "congress")?;
    B::check_one_term(&*a, "serious", "serious")
}

/// blast some random strings through the analyzer
fn ems_test_random_strings() -> Result<()> {
    B::check_random_data(&*random(), &analyzer(), 200 * mult())
}

fn ems_test_empty_term() -> Result<()> {
    let a = keyword_tokenizer_analyzer(
        "org.apache.lucene.analysis.en.TestEnglishMinimalStemFilter$2",
        |t| Ok(boxed(new_english_minimal_stem_filter(t)?)),
    );
    B::check_one_term(&*a, "", "")?;
    a.close();
    Ok(())
}

fn english_minimal_stem_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.en.TestEnglishMinimalStemFilter")
        .method(hook("setUp", ems_set_up))
        .method(hook("tearDown", close_analyzer_tear_down))
        .method(t("testExamples", ems_test_examples))
        .method(t("testRandomStrings", ems_test_random_strings))
        .method(t("testEmptyTerm", ems_test_empty_term))
        .build()
}

// -----------------------------------------------------------------------------
// Stemming factory tests (TestEnglishMinimalStemFilterFactory,
// TestKStemFilterFactory, TestPorterStemFilterFactory)
// -----------------------------------------------------------------------------

/// `whitespaceMockTokenizer(new StringReader(text))` through the filter
/// factory `name`, asserted to produce `expected`.
fn stem_factory(name: &str, text: &str, expected: &[&str]) -> Result<()> {
    let stream = B::whitespace_mock_tokenizer(Box::new(StringCharReader::new(text)))?;
    let mut stream = token_filter_factory(EN, name, &[])?.create(boxed(stream))?;
    B::assert_token_stream_contents(&mut *stream, expected)
}

/// Test that bogus arguments result in exception
fn bogus_arguments(name: &str) -> Result<()> {
    let expected = iae_message(|| token_filter_factory(EN, name, &["bogusArg", "bogusValue"]));
    Assert::assert_true(expected.contains("Unknown parameters"));
    Ok(())
}

fn emsf_test_stemming() -> Result<()> {
    stem_factory("EnglishMinimalStem", "bricks", &["brick"])
}

fn emsf_test_bogus_arguments() -> Result<()> {
    bogus_arguments("EnglishMinimalStem")
}

fn english_minimal_stem_filter_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.en.TestEnglishMinimalStemFilterFactory")
        .method(t("testStemming", emsf_test_stemming))
        .method(t("testBogusArguments", emsf_test_bogus_arguments))
        .build()
}

fn ksf_test_stemming() -> Result<()> {
    stem_factory("KStem", "bricks", &["brick"])
}

fn ksf_test_bogus_arguments() -> Result<()> {
    bogus_arguments("KStem")
}

fn kstem_filter_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.en.TestKStemFilterFactory")
        .method(t("testStemming", ksf_test_stemming))
        .method(t("testBogusArguments", ksf_test_bogus_arguments))
        .build()
}

/// Ensure the filter actually stems text.
fn psf_test_stemming() -> Result<()> {
    stem_factory("PorterStem", "dogs", &["dog"])
}

fn psf_test_bogus_arguments() -> Result<()> {
    bogus_arguments("PorterStem")
}

fn porter_stem_filter_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.en.TestPorterStemFilterFactory")
        .method(t("testStemming", psf_test_stemming))
        .method(t("testBogusArguments", psf_test_bogus_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// TestKStemmer
// -----------------------------------------------------------------------------

fn ks_set_up() -> Result<()> {
    LuceneTestCase::super_set_up();
    set_analyzer(mock_tokenizer_analyzer(
        "org.apache.lucene.analysis.en.TestKStemmer$1",
        Arc::clone(&WHITESPACE),
        true,
        |t| Ok(boxed(new_kstem_filter(t)?)),
    ));
    Ok(())
}

/// blast some random strings through the analyzer
fn ks_test_random_strings() -> Result<()> {
    B::check_random_data(&*random(), &analyzer(), 200 * mult())
}

/// test the kstemmer optimizations against a bunch of words that were
/// stemmed with the original java kstemmer (generated from testCreateMap,
/// commented out below).
fn ks_test_vocabulary() -> Result<()> {
    VocabularyAssert::assert_vocabulary_zip_tsv(
        &*analyzer(),
        &en_data_path("kstemTestData.zip")?,
        "kstem_examples.txt",
    )
}

fn ks_test_empty_term() -> Result<()> {
    let a = keyword_tokenizer_analyzer("org.apache.lucene.analysis.en.TestKStemmer$2", |t| {
        Ok(boxed(new_kstem_filter(t)?))
    });
    B::check_one_term(&*a, "", "")?;
    a.close();
    Ok(())
}

fn kstemmer_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.en.TestKStemmer")
        .method(hook("setUp", ks_set_up))
        .method(hook("tearDown", close_analyzer_tear_down))
        .method(t("testRandomStrings", ks_test_random_strings))
        .method(t("testVocabulary", ks_test_vocabulary))
        .method(t("testEmptyTerm", ks_test_empty_term))
        .build()
}

// -----------------------------------------------------------------------------
// TestPorterStemFilter
// -----------------------------------------------------------------------------

fn ps_set_up() -> Result<()> {
    LuceneTestCase::super_set_up();
    set_analyzer(mock_tokenizer_analyzer(
        "org.apache.lucene.analysis.en.TestPorterStemFilter$1",
        Arc::clone(&KEYWORD),
        false,
        |t| Ok(boxed(new_porter_stem_filter(t)?)),
    ));
    Ok(())
}

/// Run the stemmer against all strings in voc.txt The output should be the
/// same as the string in output.txt
fn ps_test_porter_stem_filter() -> Result<()> {
    VocabularyAssert::assert_vocabulary_zip(
        &*analyzer(),
        &en_data_path("porterTestData.zip")?,
        "voc.txt",
        "output.txt",
    )
}

fn ps_test_with_keyword_attribute() -> Result<()> {
    let mut set = CharArraySet::new(1, true);
    set.add("yourselves");
    let mut tokenizer = MockTokenizer::new(Arc::clone(&WHITESPACE), false)?;
    tokenizer.set_reader(Box::new(StringCharReader::new("yourselves yours")))?;
    let mut filter =
        new_porter_stem_filter(boxed(new_set_keyword_marker_filter(boxed(tokenizer), set)?))?;
    B::assert_token_stream_contents(&mut filter, &["yourselves", "your"])
}

/// blast some random strings through the analyzer
fn ps_test_random_strings() -> Result<()> {
    B::check_random_data(&*random(), &analyzer(), 200 * mult())
}

fn ps_test_empty_term() -> Result<()> {
    let a = keyword_tokenizer_analyzer(
        "org.apache.lucene.analysis.en.TestPorterStemFilter$2",
        |t| Ok(boxed(new_porter_stem_filter(t)?)),
    );
    B::check_one_term(&*a, "", "")?;
    a.close();
    Ok(())
}

fn porter_stem_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.en.TestPorterStemFilter")
        .method(hook("setUp", ps_set_up))
        .method(hook("tearDown", close_analyzer_tear_down))
        .method(t("testPorterStemFilter", ps_test_porter_stem_filter))
        .method(t(
            "testWithKeywordAttribute",
            ps_test_with_keyword_attribute,
        ))
        .method(t("testRandomStrings", ps_test_random_strings))
        .method(t("testEmptyTerm", ps_test_empty_term))
        .build()
}

// -----------------------------------------------------------------------------
// TestAnalyzers
// -----------------------------------------------------------------------------

/// `assertEquals(new BytesRef(expected), actual)`.
fn assert_bytes(expected: &str, actual: BytesRef) {
    Assert::assert_equals(
        format!("{:?}", expected.as_bytes()),
        format!("{:?}", actual.slice()),
    );
}

fn an_test_simple() -> Result<()> {
    let a = SimpleAnalyzer::new();
    B::assert_analyzes_to(&a, "foo bar FOO BAR", &["foo", "bar", "foo", "bar"])?;
    B::assert_analyzes_to(
        &a,
        "foo      bar .  FOO <> BAR",
        &["foo", "bar", "foo", "bar"],
    )?;
    B::assert_analyzes_to(&a, "foo.bar.FOO.BAR", &["foo", "bar", "foo", "bar"])?;
    B::assert_analyzes_to(&a, "U.S.A.", &["u", "s", "a"])?;
    B::assert_analyzes_to(&a, "C++", &["c"])?;
    B::assert_analyzes_to(&a, "B2B", &["b", "b"])?;
    B::assert_analyzes_to(&a, "2B", &["b"])?;
    B::assert_analyzes_to(&a, "\"QUOTED\" word", &["quoted", "word"])?;
    assert_bytes("\"\\à3[]()! cz@", a.normalize("dummy", "\"\\À3[]()! Cz@")?);
    a.close();
    Ok(())
}

fn an_test_null() -> Result<()> {
    let a = WhitespaceAnalyzer::new();
    B::assert_analyzes_to(&a, "foo bar FOO BAR", &["foo", "bar", "FOO", "BAR"])?;
    B::assert_analyzes_to(
        &a,
        "foo      bar .  FOO <> BAR",
        &["foo", "bar", ".", "FOO", "<>", "BAR"],
    )?;
    B::assert_analyzes_to(&a, "foo.bar.FOO.BAR", &["foo.bar.FOO.BAR"])?;
    B::assert_analyzes_to(&a, "U.S.A.", &["U.S.A."])?;
    B::assert_analyzes_to(&a, "C++", &["C++"])?;
    B::assert_analyzes_to(&a, "B2B", &["B2B"])?;
    B::assert_analyzes_to(&a, "2B", &["2B"])?;
    B::assert_analyzes_to(&a, "\"QUOTED\" word", &["\"QUOTED\"", "word"])?;
    assert_bytes("\"\\À3[]()! Cz@", a.normalize("dummy", "\"\\À3[]()! Cz@")?);
    a.close();
    Ok(())
}

fn an_test_stop() -> Result<()> {
    let a = StopAnalyzer::new(ENGLISH_STOP_WORDS_SET.clone());
    B::assert_analyzes_to(&a, "foo bar FOO BAR", &["foo", "bar", "foo", "bar"])?;
    B::assert_analyzes_to(
        &a,
        "foo a bar such FOO THESE BAR",
        &["foo", "bar", "foo", "bar"],
    )?;
    assert_bytes("\"\\à3[]()! cz@", a.normalize("dummy", "\"\\À3[]()! Cz@")?);
    assert_bytes("the", a.normalize("dummy", "the")?);
    a.close();
    Ok(())
}

/// `PayloadSetter`: sets one reused one-byte payload on every token and then
/// increments its byte. Java shares the `byte[]` with the attribute, so the
/// consumer sees the incremented byte; the port's payload is a value, so it
/// sets the incremented byte.
#[derive(Debug)]
struct PayloadSetterLogic {
    data: u8,
}

impl TokenFilterLogic for PayloadSetterLogic {
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        let has_next = input.increment_token()?;
        if !has_next {
            return Ok(false);
        }
        // reuse the payload / byte[]
        self.data = self.data.wrapping_add(1);
        source
            .get_attribute_interface_mut::<dyn PayloadAttribute>()
            .expect("payloadAtt")
            .set_payload(Some(BytesRef::new(vec![self.data])));
        Ok(true)
    }
}

/// `new PayloadSetter(input)`.
fn payload_setter(input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
    let mut filter = TokenFilter::new(input, PayloadSetterLogic { data: 0 });
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn PayloadAttribute>()?;
    Ok(boxed(filter))
}

fn verify_payload(ts: &mut dyn TokenStream) -> Result<()> {
    ts.reset()?;
    let mut b: u8 = 1;
    loop {
        let has_next = ts.increment_token()?;
        if !has_next {
            break;
        }
        let payload = ts
            .attribute_source()
            .get_attribute_interface::<dyn PayloadAttribute>()
            .expect("payloadAtt")
            .get_payload()
            .expect("a payload")
            .slice()[0];
        Assert::assert_equals(b, payload);
        b = b.wrapping_add(1);
    }
    Ok(())
}

// Make sure old style next() calls result in a new copy of payloads
fn an_test_payload_copy() -> Result<()> {
    let s = "how now brown cow";
    for _ in 0..2 {
        let mut ts = new_whitespace_tokenizer();
        ts.set_reader(Box::new(StringCharReader::new(s)))?;
        let mut ts = payload_setter(boxed(ts))?;
        verify_payload(&mut *ts)?;
    }
    Ok(())
}

/// `LowerCaseWhitespaceAnalyzer` / `UpperCaseWhitespaceAnalyzer`: a random
/// whitespace tokenizer and the case filter `upper` selects.
fn case_whitespace_analyzer(upper: bool) -> Arc<dyn Analyzer> {
    use rucene_test_framework::analysis::{components, shared, sink};
    let name = if upper {
        "org.apache.lucene.analysis.core.TestAnalyzers$UpperCaseWhitespaceAnalyzer"
    } else {
        "org.apache.lucene.analysis.core.TestAnalyzers$LowerCaseWhitespaceAnalyzer"
    };
    let filter = move |input: Box<dyn TokenStream>| -> Result<Box<dyn TokenStream>> {
        if upper {
            Ok(boxed(new_upper_case_filter(input)?))
        } else {
            Ok(boxed(LowerCaseFilter::new(input)?))
        }
    };
    FnAnalyzer::new(name, move |_| {
        if random().next_boolean() {
            let tokenizer = Arc::new(AtomicRefCell::new(new_whitespace_tokenizer()));
            let stream = filter(shared(&tokenizer))?;
            Ok(components(tokenizer, sink(BoxedStream(stream))))
        } else {
            let tokenizer = Arc::new(AtomicRefCell::new(new_unicode_whitespace_tokenizer()));
            let stream = filter(shared(&tokenizer))?;
            Ok(components(tokenizer, sink(BoxedStream(stream))))
        }
    })
    .arc()
}

/// Test that LowercaseFilter handles entire unicode range correctly
fn an_test_lower_case_filter() -> Result<()> {
    let a = case_whitespace_analyzer(false);
    // BMP
    B::assert_analyzes_to(&*a, "AbaCaDabA", &["abacadaba"])?;
    // supplementary
    B::assert_analyzes_to(
        &*a,
        "\u{10416}\u{10416}\u{10416}\u{10416}",
        &["\u{1043e}\u{1043e}\u{1043e}\u{1043e}"],
    )?;
    B::assert_analyzes_to(&*a, "AbaCa\u{10416}DabA", &["abaca\u{1043e}daba"])?;
    // unpaired lead surrogate / unpaired trail surrogate: not expressible
    // (see the module documentation).
    a.close();
    Ok(())
}

/// Test that LowercaseFilter handles entire unicode range correctly
fn an_test_upper_case_filter() -> Result<()> {
    let a = case_whitespace_analyzer(true);
    // BMP
    B::assert_analyzes_to(&*a, "AbaCaDabA", &["ABACADABA"])?;
    // supplementary
    B::assert_analyzes_to(
        &*a,
        "\u{1043e}\u{1043e}\u{1043e}\u{1043e}",
        &["\u{10416}\u{10416}\u{10416}\u{10416}"],
    )?;
    B::assert_analyzes_to(&*a, "AbaCa\u{1043e}DabA", &["ABACA\u{10416}DABA"])?;
    // unpaired lead surrogate / unpaired trail surrogate: not expressible
    // (see the module documentation).
    a.close();
    Ok(())
}

fn an_test_whitespace_tokenizer() -> Result<()> {
    let mut tokenizer = new_whitespace_tokenizer();
    tokenizer.set_reader(Box::new(StringCharReader::new("Tokenizer \u{1041c}test")))?;
    B::assert_token_stream_contents(&mut tokenizer, &["Tokenizer", "\u{1041c}test"])
}

/// The analyzers `testRandomStrings`/`testRandomHugeStrings` blast.
fn random_analyzers() -> Vec<Arc<dyn Analyzer>> {
    vec![
        Arc::new(WhitespaceAnalyzer::new()),
        Arc::new(SimpleAnalyzer::new()),
        Arc::new(StopAnalyzer::new(ENGLISH_STOP_WORDS_SET.clone())),
        Arc::new(UnicodeWhitespaceAnalyzer::new()),
    ]
}

/// blast some random strings through the analyzer
fn an_test_random_strings() -> Result<()> {
    let analyzers = random_analyzers();
    for analyzer in &analyzers {
        B::check_random_data(&*random(), analyzer, 100 * mult())?;
    }
    for analyzer in analyzers {
        analyzer.close();
    }
    Ok(())
}

/// blast some random large strings through the analyzer
fn an_test_random_huge_strings() -> Result<()> {
    let analyzers = random_analyzers();
    for analyzer in &analyzers {
        B::check_random_data_max_word_length(&*random(), analyzer, 10 * mult(), 8192)?;
    }
    for analyzer in analyzers {
        analyzer.close();
    }
    Ok(())
}

fn analyzers_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.core.TestAnalyzers")
        .method(t("testSimple", an_test_simple))
        .method(t("testNull", an_test_null))
        .method(t("testStop", an_test_stop))
        .method(t("testPayloadCopy", an_test_payload_copy))
        .method(t("testLowerCaseFilter", an_test_lower_case_filter))
        .method(t("testUpperCaseFilter", an_test_upper_case_filter))
        .method(t("testWhitespaceTokenizer", an_test_whitespace_tokenizer))
        .method(t("testRandomStrings", an_test_random_strings))
        .method(t("testRandomHugeStrings", an_test_random_huge_strings))
        .build()
}

// -----------------------------------------------------------------------------
// TestStopAnalyzer
// -----------------------------------------------------------------------------

thread_local! {
    /// `stop` and `inValidTokens`, which `setUp` sets.
    static STOP: RefCell<Option<(Arc<StopAnalyzer>, HashSet<String>)>> = const { RefCell::new(None) };
}

fn sa_set_up() -> Result<()> {
    LuceneTestCase::super_set_up();
    let in_valid_tokens: HashSet<String> = ENGLISH_STOP_WORDS_SET
        .iter()
        .map(|w| String::from_utf16(w).expect("valid UTF-16"))
        .collect();
    let stop = Arc::new(StopAnalyzer::new(ENGLISH_STOP_WORDS_SET.clone()));
    STOP.with(|s| *s.borrow_mut() = Some((stop, in_valid_tokens)));
    Ok(())
}

fn sa_tear_down() -> Result<()> {
    if let Some((stop, _)) = STOP.with(|s| s.borrow_mut().take()) {
        stop.close();
    }
    LuceneTestCase::super_tear_down();
    Ok(())
}

/// Consumes `analyzer.tokenStream("test", s)`, calling `check` with each
/// term and position increment.
fn consume(analyzer: &dyn Analyzer, s: &str, mut check: impl FnMut(&str, i32)) -> Result<()> {
    let stream = analyzer.token_stream("test", Box::new(StringCharReader::new(s)))?;
    let mut stream = stream.borrow_mut();
    stream
        .attribute_source_mut()
        .add_attribute_interface::<dyn PositionIncrementAttribute>()?;
    stream.reset()?;
    while stream.increment_token()? {
        let text = term_of(stream.attribute_source());
        let pos_inc = stream
            .attribute_source()
            .get_attribute_interface::<dyn PositionIncrementAttribute>()
            .expect("posIncrAtt")
            .get_position_increment();
        check(&text, pos_inc);
    }
    stream.end()?;
    stream.close()
}

fn sa_test_defaults() -> Result<()> {
    let (stop, in_valid_tokens) = STOP.with(|s| s.borrow().clone().expect("setUp"));
    consume(
        &*stop,
        "This is a test of the english stop analyzer",
        |text, _| Assert::assert_false(in_valid_tokens.contains(text)),
    )
}

fn sa_test_stop_list() -> Result<()> {
    let stop_words_set = char_set(&["good", "test", "analyzer"], false);
    let new_stop = StopAnalyzer::new(stop_words_set.clone());
    consume(
        &new_stop,
        "This is a good test of the english stop analyzer",
        |text, _| Assert::assert_false(stop_words_set.contains(Some(text))),
    )?;
    new_stop.close();
    Ok(())
}

fn sa_test_stop_list_positions() -> Result<()> {
    let stop_words_set = char_set(&["good", "test", "analyzer"], false);
    let new_stop = StopAnalyzer::new(stop_words_set.clone());
    let s = "This is a good test of the english stop analyzer with positions";
    let expected_incr = [1, 1, 1, 3, 1, 1, 1, 2, 1];
    let mut i = 0;
    consume(&new_stop, s, |text, pos_inc| {
        Assert::assert_false(stop_words_set.contains(Some(text)));
        Assert::assert_equals(expected_incr[i], pos_inc);
        i += 1;
    })?;
    new_stop.close();
    Ok(())
}

fn stop_analyzer_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.core.TestStopAnalyzer")
        .method(hook("setUp", sa_set_up))
        .method(hook("tearDown", sa_tear_down))
        .method(t("testDefaults", sa_test_defaults))
        .method(t("testStopList", sa_test_stop_list))
        .method(t("testStopListPositions", sa_test_stop_list_positions))
        .build()
}

// -----------------------------------------------------------------------------
// TestStopFilterFactory
// -----------------------------------------------------------------------------

/// `(StopFilterFactory) tokenFilterFactory("Stop", keysAndValues...)`.
fn stop_factory(keys_and_values: &[&str]) -> Result<StopFilterFactory> {
    concrete_factory(CORE, StopFilterFactory::new, keys_and_values)
}

fn sff_test_inform() -> Result<()> {
    // `new ClasspathResourceLoader(getClass())` is a value, never null.
    let factory = stop_factory(&["words", "stop-1.txt", "ignoreCase", "true"])?;
    let words = factory.get_stop_words();
    Assert::assert_true_msg("words is null and it shouldn't be", words.is_some());
    let words = words.unwrap();
    Assert::assert_true_msg(
        &format!("words Size: {} is not: 2", words.size()),
        words.size() == 2,
    );
    Assert::assert_true_msg(
        &format!("{} does not equal: true", factory.base().is_ignore_case()),
        factory.base().is_ignore_case(),
    );
    let factory = stop_factory(&["words", "stop-1.txt, stop-2.txt", "ignoreCase", "true"])?;
    let words = factory.get_stop_words();
    Assert::assert_true_msg("words is null and it shouldn't be", words.is_some());
    let words = words.unwrap();
    Assert::assert_true_msg(
        &format!("words Size: {} is not: 4", words.size()),
        words.size() == 4,
    );
    Assert::assert_true_msg(
        &format!("{} does not equal: true", factory.base().is_ignore_case()),
        factory.base().is_ignore_case(),
    );
    let factory = stop_factory(&[
        "words",
        "stop-snowball.txt",
        "format",
        "snowball",
        "ignoreCase",
        "true",
    ])?;
    let words = factory.get_stop_words().expect("words");
    Assert::assert_equals(8, words.size());
    for w in [
        "he", "him", "his", "himself", "she", "her", "hers", "herself",
    ] {
        Assert::assert_true(words.contains(Some(w)));
    }
    // defaults
    let factory = stop_factory(&[])?;
    let words = factory.get_stop_words().expect("words");
    // `assertEquals(ENGLISH_STOP_WORDS_SET, factory.getStopWords())`:
    // `AbstractSet.equals`, same size and every element contained.
    Assert::assert_equals(ENGLISH_STOP_WORDS_SET.size(), words.size());
    for w in ENGLISH_STOP_WORDS_SET.iter() {
        let w = String::from_utf16(w).expect("valid UTF-16");
        Assert::assert_true(words.contains(Some(&w)));
    }
    Assert::assert_equals(false, factory.base().is_ignore_case());
    Ok(())
}

/// Test that bogus arguments result in exception
fn sff_test_bogus_arguments() -> Result<()> {
    let expected = iae_message(|| token_filter_factory(CORE, "Stop", &["bogusArg", "bogusValue"]));
    Assert::assert_true(expected.contains("Unknown parameters"));
    Ok(())
}

/// Test that bogus arguments result in exception
fn sff_test_bogus_formats() -> Result<()> {
    let msg = iae_message(|| {
        token_filter_factory(
            CORE,
            "Stop",
            &["words", "stop-snowball.txt", "format", "bogus"],
        )
    });
    Assert::assert_true_msg(&msg, msg.contains("Unknown"));
    Assert::assert_true_msg(&msg, msg.contains("format"));
    Assert::assert_true_msg(&msg, msg.contains("bogus"));
    // implicit default words file
    let msg = iae_message(|| token_filter_factory(CORE, "Stop", &["format", "bogus"]));
    Assert::assert_true_msg(&msg, msg.contains("can not be specified"));
    Assert::assert_true_msg(&msg, msg.contains("format"));
    Assert::assert_true_msg(&msg, msg.contains("bogus"));
    Ok(())
}

fn stop_filter_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.core.TestStopFilterFactory")
        .method(t("testInform", sff_test_inform))
        .method(t("testBogusArguments", sff_test_bogus_arguments))
        .method(t("testBogusFormats", sff_test_bogus_formats))
        .build()
}

// -----------------------------------------------------------------------------
// TestStemmerOverrideFilter
// -----------------------------------------------------------------------------

/// `keywordTokenizer(data)`.
fn keyword_tokenizer(data: &str) -> Result<Box<dyn TokenStream>> {
    let mut tokenizer = new_keyword_tokenizer();
    tokenizer.set_reader(Box::new(StringCharReader::new(data)))?;
    Ok(boxed(tokenizer))
}

/// `new PorterStemFilter(new StemmerOverrideFilter(tokenizer, map))`.
fn override_then_porter(
    tokenizer: Box<dyn TokenStream>,
    map: Arc<rucene_analysis_common::analysis::miscellaneous::stemmer_override_filter::StemmerOverrideMap>,
) -> Result<Box<dyn TokenStream>> {
    Ok(boxed(new_porter_stem_filter(boxed(
        new_stemmer_override_filter(tokenizer, map)?,
    ))?))
}

fn sof_test_override() -> Result<()> {
    // lets make booked stem to books
    // the override filter will convert "booked" to "books",
    // but also mark it with KeywordAttribute so Porter will not change it.
    let mut builder = StemmerOverrideBuilder::new();
    builder.add("booked", "books")?;
    let tokenizer = keyword_tokenizer("booked")?;
    let mut stream = override_then_porter(tokenizer, Arc::new(builder.build()?))?;
    B::assert_token_stream_contents(&mut *stream, &["books"])
}

fn sof_test_ignore_case() -> Result<()> {
    // lets make booked stem to books
    // the override filter will convert "booked" to "books",
    // but also mark it with KeywordAttribute so Porter will not change it.
    let mut builder = StemmerOverrideBuilder::with_ignore_case(true);
    builder.add("boOkEd", "books")?;
    let tokenizer = keyword_tokenizer("BooKeD")?;
    let mut stream = override_then_porter(tokenizer, Arc::new(builder.build()?))?;
    B::assert_token_stream_contents(&mut *stream, &["books"])
}

fn sof_test_no_overrides() -> Result<()> {
    let mut builder = StemmerOverrideBuilder::with_ignore_case(true);
    let tokenizer = keyword_tokenizer("book")?;
    let mut stream = override_then_porter(tokenizer, Arc::new(builder.build()?))?;
    B::assert_token_stream_contents(&mut *stream, &["book"])
}

fn sof_test_random_realistic_white_space() -> Result<()> {
    let mut map: JavaHashMap<String> = JavaHashMap::new();
    let mut seen: HashSet<String> = HashSet::new();
    let num_terms = LuceneTestCase::at_least(50);
    let ignore_case = random().next_boolean();
    for _ in 0..num_terms {
        let random_realistic_unicode_string = TestUtil::random_realistic_unicode_string(&*random());
        let mut builder = String::new();
        for c in random_realistic_unicode_string.chars() {
            if !character::is_whitespace(c as i32) {
                builder.push(c);
            }
        }
        if !builder.is_empty() {
            let input_value = builder;
            // Make sure we don't try to add two inputs that vary only by case:
            let seen_input_value = if ignore_case {
                // TODO: can we simply use inputValue.toLowerCase(Locale.ROOT)???
                let mut buffer: Vec<char> = input_value.chars().collect();
                let len = buffer.len();
                to_lower_case(&mut buffer, 0, len);
                buffer.into_iter().collect()
            } else {
                input_value.clone()
            };
            if !seen.contains(&seen_input_value) {
                seen.insert(seen_input_value);
                let value = TestUtil::random_simple_string(&*random());
                map.insert(
                    input_value,
                    if value.is_empty() {
                        "a".to_string()
                    } else {
                        value
                    },
                );
            }
        }
    }
    if map.is_empty() {
        map.insert("booked".into(), "books".into());
    }
    let mut builder = StemmerOverrideBuilder::with_ignore_case(ignore_case);
    let mut input = String::new();
    let mut output: Vec<String> = Vec::new();
    for (key, value) in map.iter() {
        builder.add(key, value)?;
        if random().next_boolean() || output.is_empty() {
            input.push_str(key);
            input.push(' ');
            output.push(value.clone());
        }
    }
    let mut tokenizer = new_whitespace_tokenizer();
    tokenizer.set_reader(Box::new(StringCharReader::new(input)))?;
    let mut stream = override_then_porter(boxed(tokenizer), Arc::new(builder.build()?))?;
    let expected: Vec<&str> = output.iter().map(String::as_str).collect();
    B::assert_token_stream_contents(&mut *stream, &expected)
}

fn sof_test_random_realistic_keyword() -> Result<()> {
    let mut map: JavaHashMap<String> = JavaHashMap::new();
    let num_terms = LuceneTestCase::at_least(50);
    for _ in 0..num_terms {
        let random_realistic_unicode_string = TestUtil::random_realistic_unicode_string(&*random());
        if !random_realistic_unicode_string.is_empty() {
            let value = TestUtil::random_simple_string(&*random());
            map.insert(
                random_realistic_unicode_string,
                if value.is_empty() {
                    "a".to_string()
                } else {
                    value
                },
            );
        }
    }
    if map.is_empty() {
        map.insert("booked".into(), "books".into());
    }
    // This test might fail if ignoreCase is true since the map might have twice the same key, once
    // lowercased and once uppercased
    let mut builder = StemmerOverrideBuilder::with_ignore_case(false);
    for (key, value) in map.iter() {
        builder.add(key, value)?;
    }
    let build = Arc::new(builder.build()?);
    for (key, value) in map.iter() {
        if random().next_boolean() {
            let mut tokenizer = new_keyword_tokenizer();
            tokenizer.set_reader(Box::new(StringCharReader::new(key.clone())))?;
            let mut stream = override_then_porter(boxed(tokenizer), Arc::clone(&build))?;
            B::assert_token_stream_contents(&mut *stream, &[value.as_str()])?;
        }
    }
    Ok(())
}

fn stemmer_override_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.miscellaneous.TestStemmerOverrideFilter")
        .method(t("testOverride", sof_test_override))
        .method(t("testIgnoreCase", sof_test_ignore_case))
        .method(t("testNoOverrides", sof_test_no_overrides))
        .method(t(
            "testRandomRealisticWhiteSpace",
            sof_test_random_realistic_white_space,
        ))
        .method(t(
            "testRandomRealisticKeyword",
            sof_test_random_realistic_keyword,
        ))
        .build()
}

// -----------------------------------------------------------------------------
// TestWordDelimiterFilter
// -----------------------------------------------------------------------------

/// Set a large position increment gap of 10 if the token is "largegap" or "/"
#[derive(Debug)]
struct LargePosIncTokenFilterLogic;

impl TokenFilterLogic for LargePosIncTokenFilterLogic {
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if input.increment_token()? {
            let term = term_of(source);
            if term == "largegap" || term == "/" {
                source
                    .get_attribute_interface_mut::<dyn PositionIncrementAttribute>()
                    .expect("posIncAtt")
                    .set_position_increment(10);
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// `new LargePosIncTokenFilter(input)`.
fn large_pos_inc(input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
    let mut filter = TokenFilter::new(input, LargePosIncTokenFilterLogic);
    let source = filter.attribute_source_mut();
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
    Ok(boxed(filter))
}

/// The anonymous `KeywordMarkerFilter` of `keywordTestAnalyzer`: marks terms
/// starting with the letter 'k' as keywords.
#[derive(Debug)]
struct StartsWithK;

impl KeywordMarkerFilterLogic for StartsWithK {
    fn is_keyword(&mut self, source: &AttributeSource) -> Result<bool> {
        // Marks terms starting with the letter 'k' as keywords
        Ok(term_of(source).starts_with('k'))
    }
}

/// `new KeywordMarkerFilter(tokenizer) { isKeyword: startsWith('k') }`, which
/// adds the `CharTermAttribute`.
fn k_filter(input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
    let mut filter = new_keyword_marker_filter(input, StartsWithK)?;
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn CharTermAttribute>()?;
    Ok(boxed(filter))
}

/// `assertAnalyzesTo(a, input, output, startOffsets, endOffsets, null,
/// posIncrements, null, false)`.
fn analyzes_graph(
    a: &dyn Analyzer,
    input: &str,
    output: &[&str],
    start: &[i32],
    end: &[i32],
    pos_inc: &[i32],
) -> Result<()> {
    B::assert_analyzes_to_graph(
        a,
        input,
        output,
        Some(start),
        Some(end),
        None,
        Some(pos_inc),
        None,
        false,
    )
}

/// The flags most `TestWordDelimiter*Filter` tests use.
const WDF_FLAGS: i32 = wdf::GENERATE_WORD_PARTS
    | wdf::GENERATE_NUMBER_PARTS
    | wdf::CATENATE_ALL
    | wdf::SPLIT_ON_CASE_CHANGE
    | wdf::SPLIT_ON_NUMERICS
    | wdf::STEM_ENGLISH_POSSESSIVE;

/// `new WordDelimiterFilter(new CannedTokenStream(new Token(text, start,
/// end)), DEFAULT_WORD_DELIM_TABLE, flags, null)`.
fn wdf_canned(text: &str, start: i32, end: i32, flags: i32) -> Result<Box<dyn TokenStream>> {
    Ok(boxed(wdf::new_word_delimiter_filter_with_table(
        boxed(CannedTokenStream::new(vec![Token::with_offsets(
            text, start, end,
        )])),
        Arc::clone(&DEFAULT_WORD_DELIM_TABLE),
        flags,
        None,
    )?))
}

fn wd_test_offsets() -> Result<()> {
    // test that subwords and catenated subwords have
    // the correct offsets.
    let mut f = wdf_canned("foo-bar", 5, 12, WDF_FLAGS)?;
    B::assert_token_stream_contents_offsets(
        &mut *f,
        &["foo", "foobar", "bar"],
        &[5, 5, 9],
        &[8, 12, 12],
    )?;
    let mut f = wdf_canned("foo-bar", 5, 6, WDF_FLAGS)?;
    B::assert_token_stream_contents_offsets(
        &mut *f,
        &["foo", "bar", "foobar"],
        &[5, 5, 5],
        &[6, 6, 6],
    )
}

fn wd_test_offset_change() -> Result<()> {
    let mut f = wdf_canned("übelkeit)", 7, 16, WDF_FLAGS)?;
    B::assert_token_stream_contents_offsets(&mut *f, &["übelkeit"], &[7], &[15])
}

fn wd_test_offset_change2() -> Result<()> {
    let mut f = wdf_canned("(übelkeit", 7, 17, WDF_FLAGS)?;
    B::assert_token_stream_contents_offsets(&mut *f, &["übelkeit"], &[8], &[17])
}

fn wd_test_offset_change3() -> Result<()> {
    let mut f = wdf_canned("(übelkeit", 7, 16, WDF_FLAGS)?;
    B::assert_token_stream_contents_offsets(&mut *f, &["übelkeit"], &[8], &[16])
}

fn wd_test_offset_change4() -> Result<()> {
    let mut f = wdf_canned("(foo,bar)", 7, 16, WDF_FLAGS)?;
    B::assert_token_stream_contents_offsets(
        &mut *f,
        &["foo", "foobar", "bar"],
        &[8, 8, 12],
        &[11, 15, 15],
    )
}

/// The `doSplit` cases `testSplits` of both word delimiter tests runs.
const SPLITS: &[(&str, &[&str])] = &[
    ("basic-split", &["basic", "split"]),
    ("camelCase", &["camel", "Case"]),
    // non-space marking symbol shouldn't cause split
    // this is an example in Thai
    (
        "\u{0e1a}\u{0e49}\u{0e32}\u{0e19}",
        &["\u{0e1a}\u{0e49}\u{0e32}\u{0e19}"],
    ),
    // possessive followed by delimiter
    ("test's'", &["test"]),
    // some russian upper and lowercase
    ("Роберт", &["Роберт"]),
    // now cause a split (russian camelCase)
    ("РобЕрт", &["Роб", "Ерт"]),
    // a composed titlecase character, don't split
    ("aǅungla", &["aǅungla"]),
    // a modifier letter, don't split
    ("ســـــــــــــــــلام", &["ســـــــــــــــــلام"]),
    // enclosing mark, don't split
    ("test⃝", &["test⃝"]),
    // combining spacing mark (the virama), don't split
    ("हिन्दी", &["हिन्दी"]),
    // don't split non-ascii digits
    ("١٢٣٤", &["١٢٣٤"]),
    // don't split supplementaries into unpaired surrogates
    ("𠀀𠀀", &["𠀀𠀀"]),
];

/// The flags of `doSplit`.
const SPLIT_FLAGS: i32 = wdf::GENERATE_WORD_PARTS
    | wdf::GENERATE_NUMBER_PARTS
    | wdf::SPLIT_ON_CASE_CHANGE
    | wdf::SPLIT_ON_NUMERICS
    | wdf::STEM_ENGLISH_POSSESSIVE;

fn wd_do_split(input: &str, output: &[&str]) -> Result<()> {
    let mut f = wdf::new_word_delimiter_filter_with_table(
        boxed(B::keyword_mock_tokenizer_str(input)?),
        Arc::clone(&DEFAULT_WORD_DELIM_TABLE),
        SPLIT_FLAGS,
        None,
    )?;
    B::assert_token_stream_contents(&mut f, output)
}

fn wd_test_splits() -> Result<()> {
    for (input, output) in SPLITS {
        wd_do_split(input, output)?;
    }
    Ok(())
}

fn wd_do_split_possessive(stem_possessive: i32, input: &str, output: &[&str]) -> Result<()> {
    let mut flags = wdf::GENERATE_WORD_PARTS
        | wdf::GENERATE_NUMBER_PARTS
        | wdf::SPLIT_ON_CASE_CHANGE
        | wdf::SPLIT_ON_NUMERICS;
    flags |= if stem_possessive == 1 {
        wdf::STEM_ENGLISH_POSSESSIVE
    } else {
        0
    };
    let mut f =
        wdf::new_word_delimiter_filter(boxed(B::keyword_mock_tokenizer_str(input)?), flags, None)?;
    B::assert_token_stream_contents(&mut f, output)
}

/// Test option that allows disabling the special "'s" stemming, instead
/// treating the single quote like other delimiters.
fn wd_test_possessives() -> Result<()> {
    wd_do_split_possessive(1, "ra's", &["ra"])?;
    wd_do_split_possessive(0, "ra's", &["ra", "s"])
}

fn wd_test_position_increments() -> Result<()> {
    let flags = WDF_FLAGS;
    let prot_words = char_set(&["NUTCH"], false);
    // analyzer that uses whitespace + wdf
    let pw = prot_words.clone();
    let a = whitespace_analyzer(
        "org.apache.lucene.analysis.miscellaneous.TestWordDelimiterFilter$1",
        move |t| {
            Ok(boxed(wdf::new_word_delimiter_filter(
                t,
                flags,
                Some(pw.clone()),
            )?))
        },
    );
    // in this case, works as expected.
    analyzes_graph(
        &*a,
        "LUCENE / SOLR",
        &["LUCENE", "SOLR"],
        &[0, 9],
        &[6, 13],
        &[1, 1],
    )?;
    // only in this case, posInc of 2 ?!
    analyzes_graph(
        &*a,
        "LUCENE / solR",
        &["LUCENE", "sol", "solR", "R"],
        &[0, 9, 9, 12],
        &[6, 12, 13, 13],
        &[1, 1, 0, 1],
    )?;
    analyzes_graph(
        &*a,
        "LUCENE / NUTCH SOLR",
        &["LUCENE", "NUTCH", "SOLR"],
        &[0, 9, 15],
        &[6, 14, 19],
        &[1, 1, 1],
    )?;
    // analyzer that will consume tokens with large position increments
    let pw = prot_words.clone();
    let a2 = whitespace_analyzer(
        "org.apache.lucene.analysis.miscellaneous.TestWordDelimiterFilter$2",
        move |t| {
            Ok(boxed(wdf::new_word_delimiter_filter(
                large_pos_inc(t)?,
                flags,
                Some(pw.clone()),
            )?))
        },
    );
    // increment of "largegap" is preserved
    analyzes_graph(
        &*a2,
        "LUCENE largegap SOLR",
        &["LUCENE", "largegap", "SOLR"],
        &[0, 7, 16],
        &[6, 15, 20],
        &[1, 10, 1],
    )?;
    // the "/" had a position increment of 10, where did it go?!?!!
    analyzes_graph(
        &*a2,
        "LUCENE / SOLR",
        &["LUCENE", "SOLR"],
        &[0, 9],
        &[6, 13],
        &[1, 11],
    )?;
    // in this case, the increment of 10 from the "/" is carried over
    analyzes_graph(
        &*a2,
        "LUCENE / solR",
        &["LUCENE", "sol", "solR", "R"],
        &[0, 9, 9, 12],
        &[6, 12, 13, 13],
        &[1, 11, 0, 1],
    )?;
    analyzes_graph(
        &*a2,
        "LUCENE / NUTCH SOLR",
        &["LUCENE", "NUTCH", "SOLR"],
        &[0, 9, 15],
        &[6, 14, 19],
        &[1, 11, 1],
    )?;
    let pw = prot_words.clone();
    let a3 = whitespace_analyzer(
        "org.apache.lucene.analysis.miscellaneous.TestWordDelimiterFilter$3",
        move |t| {
            let filter = new_stop_filter(t, ENGLISH_STOP_WORDS_SET.clone())?;
            Ok(boxed(wdf::new_word_delimiter_filter(
                boxed(filter),
                flags,
                Some(pw.clone()),
            )?))
        },
    );
    analyzes_graph(
        &*a3,
        "lucene.solr",
        &["lucene", "lucenesolr", "solr"],
        &[0, 0, 7],
        &[6, 11, 11],
        &[1, 0, 1],
    )?;
    // the stopword should add a gap here
    analyzes_graph(
        &*a3,
        "the lucene.solr",
        &["lucene", "lucenesolr", "solr"],
        &[4, 4, 11],
        &[10, 15, 15],
        &[2, 0, 1],
    )?;
    a.close();
    a2.close();
    a3.close();
    Ok(())
}

/// `keywordTestAnalyzer(flags)` of `TestWordDelimiterFilter`.
fn wd_keyword_test_analyzer(flags: i32) -> Arc<dyn Analyzer> {
    whitespace_analyzer(
        "org.apache.lucene.analysis.miscellaneous.TestWordDelimiterFilter$4",
        move |t| {
            Ok(boxed(wdf::new_word_delimiter_filter(
                k_filter(t)?,
                flags,
                None,
            )?))
        },
    )
}

fn wd_test_keyword_filter() -> Result<()> {
    B::assert_analyzes_to(
        &*wd_keyword_test_analyzer(wdf::GENERATE_WORD_PARTS),
        "abc-def klm-nop kpop",
        &["abc", "def", "klm", "nop", "kpop"],
    )?;
    analyzes_graph(
        &*wd_keyword_test_analyzer(wdf::GENERATE_WORD_PARTS | wdf::IGNORE_KEYWORDS),
        "abc-def klm-nop kpop",
        &["abc", "def", "klm-nop", "kpop"],
        &[0, 4, 8, 16],
        &[3, 7, 15, 20],
        &[1, 1, 1, 1],
    )
}

/// concat numbers + words + all
fn wd_test_lots_of_concatenating() -> Result<()> {
    let flags = wdf::GENERATE_WORD_PARTS
        | wdf::GENERATE_NUMBER_PARTS
        | wdf::CATENATE_WORDS
        | wdf::CATENATE_NUMBERS
        | wdf::CATENATE_ALL
        | wdf::SPLIT_ON_CASE_CHANGE
        | wdf::SPLIT_ON_NUMERICS
        | wdf::STEM_ENGLISH_POSSESSIVE;
    // analyzer that uses whitespace + wdf
    let a = whitespace_analyzer(
        "org.apache.lucene.analysis.miscellaneous.TestWordDelimiterFilter$5",
        move |t| Ok(boxed(wdf::new_word_delimiter_filter(t, flags, None)?)),
    );
    analyzes_graph(
        &*a,
        "abc-def-123-456",
        &[
            "abc",
            "abcdef",
            "abcdef123456",
            "def",
            "123",
            "123456",
            "456",
        ],
        &[0, 0, 0, 4, 8, 8, 12],
        &[3, 7, 15, 7, 11, 15, 15],
        &[1, 0, 0, 1, 1, 0, 1],
    )?;
    a.close();
    Ok(())
}

/// concat numbers + words + all + preserve original
fn wd_test_lots_of_concatenating2() -> Result<()> {
    let flags = wdf::PRESERVE_ORIGINAL
        | wdf::GENERATE_WORD_PARTS
        | wdf::GENERATE_NUMBER_PARTS
        | wdf::CATENATE_WORDS
        | wdf::CATENATE_NUMBERS
        | wdf::CATENATE_ALL
        | wdf::SPLIT_ON_CASE_CHANGE
        | wdf::SPLIT_ON_NUMERICS
        | wdf::STEM_ENGLISH_POSSESSIVE;
    // analyzer that uses whitespace + wdf
    let a = whitespace_analyzer(
        "org.apache.lucene.analysis.miscellaneous.TestWordDelimiterFilter$6",
        move |t| Ok(boxed(wdf::new_word_delimiter_filter(t, flags, None)?)),
    );
    analyzes_graph(
        &*a,
        "abc-def-123-456",
        &[
            "abc-def-123-456",
            "abc",
            "abcdef",
            "abcdef123456",
            "def",
            "123",
            "123456",
            "456",
        ],
        &[0, 0, 0, 0, 4, 8, 8, 12],
        &[15, 3, 7, 15, 7, 11, 15, 15],
        &[1, 0, 0, 0, 1, 1, 0, 1],
    )?;
    a.close();
    Ok(())
}

/// The protected words of the random tests: `{"a", "b", "cd"}` or `null`.
fn random_protected_words(random: &dyn Random) -> Option<CharArraySet> {
    if random.next_boolean() {
        Some(char_set(&["a", "b", "cd"], false))
    } else {
        None
    }
}

/// blast some random strings through the analyzer
fn wd_test_random_strings() -> Result<()> {
    let num_iterations = LuceneTestCase::at_least(3);
    for _ in 0..num_iterations {
        let flags = random().next_int_bound(512);
        let protected_words = random_protected_words(&*random());
        let a = whitespace_analyzer(
            "org.apache.lucene.analysis.miscellaneous.TestWordDelimiterFilter$7",
            move |t| {
                Ok(boxed(wdf::new_word_delimiter_filter(
                    t,
                    flags,
                    protected_words.clone(),
                )?))
            },
        );
        // TODO: properly support positionLengthAttribute
        B::check_random_data_graph(&*random(), &a, 100 * mult(), 20, false, false)?;
        a.close();
    }
    Ok(())
}

/// blast some enormous random strings through the analyzer
fn wd_test_random_huge_strings() -> Result<()> {
    let num_iterations = LuceneTestCase::at_least(1);
    for _ in 0..num_iterations {
        let flags = random().next_int_bound(512);
        let protected_words = random_protected_words(&*random());
        let a = FnAnalyzer::new(
            "org.apache.lucene.analysis.miscellaneous.TestWordDelimiterFilter$8",
            move |_| {
                let tokenizer = Arc::new(AtomicRefCell::new(MockTokenizer::with_max_token_length(
                    Arc::clone(&WHITESPACE),
                    false,
                    (MAX_TERM_LENGTH / 2) as i32,
                )?));
                let filter = wdf::new_word_delimiter_filter(
                    rucene_test_framework::analysis::shared(&tokenizer),
                    flags,
                    protected_words.clone(),
                )?;
                Ok(rucene_test_framework::analysis::components(
                    tokenizer,
                    rucene_test_framework::analysis::sink(filter),
                ))
            },
        )
        .arc();
        // TODO: properly support positionLengthAttribute
        B::check_random_data_graph(&*random(), &a, 10 * mult(), 8192, false, false)?;
        a.close();
    }
    Ok(())
}

fn wd_test_empty_term() -> Result<()> {
    let random = random();
    for i in 0..512 {
        let flags = i;
        let protected_words = random_protected_words(&*random);
        let a = keyword_tokenizer_analyzer(
            "org.apache.lucene.analysis.miscellaneous.TestWordDelimiterFilter$9",
            move |t| {
                Ok(boxed(wdf::new_word_delimiter_filter(
                    t,
                    flags,
                    protected_words.clone(),
                )?))
            },
        );
        // depending upon options, this thing may or may not preserve the empty term
        let use_char_filter = random.next_boolean();
        B::check_analysis_consistency(&random, &*a, use_char_filter, "")?;
        a.close();
    }
    Ok(())
}

fn wd_test_only_numbers() -> Result<()> {
    let flags = wdf::GENERATE_WORD_PARTS | wdf::SPLIT_ON_CASE_CHANGE | wdf::SPLIT_ON_NUMERICS;
    let a = whitespace_analyzer(
        "org.apache.lucene.analysis.miscellaneous.TestWordDelimiterFilter$10",
        move |t| Ok(boxed(wdf::new_word_delimiter_filter(t, flags, None)?)),
    );
    analyzes_graph(&*a, "7-586", &[], &[], &[], &[])
}

fn wd_test_number_punct() -> Result<()> {
    let flags = wdf::GENERATE_WORD_PARTS | wdf::SPLIT_ON_CASE_CHANGE | wdf::SPLIT_ON_NUMERICS;
    let a = whitespace_analyzer(
        "org.apache.lucene.analysis.miscellaneous.TestWordDelimiterFilter$11",
        move |t| Ok(boxed(wdf::new_word_delimiter_filter(t, flags, None)?)),
    );
    analyzes_graph(&*a, "6-", &["6"], &[0], &[1], &[1])
}

fn word_delimiter_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.miscellaneous.TestWordDelimiterFilter")
        .method(t("testOffsets", wd_test_offsets))
        .method(t("testOffsetChange", wd_test_offset_change))
        .method(t("testOffsetChange2", wd_test_offset_change2))
        .method(t("testOffsetChange3", wd_test_offset_change3))
        .method(t("testOffsetChange4", wd_test_offset_change4))
        .method(t("testSplits", wd_test_splits))
        .method(t("testPossessives", wd_test_possessives))
        .method(t("testPositionIncrements", wd_test_position_increments))
        .method(t("testKeywordFilter", wd_test_keyword_filter))
        .method(t("testLotsOfConcatenating", wd_test_lots_of_concatenating))
        .method(t(
            "testLotsOfConcatenating2",
            wd_test_lots_of_concatenating2,
        ))
        .method(t("testRandomStrings", wd_test_random_strings))
        .method(t("testRandomHugeStrings", wd_test_random_huge_strings))
        .method(t("testEmptyTerm", wd_test_empty_term))
        .method(t("testOnlyNumbers", wd_test_only_numbers))
        .method(t("testNumberPunct", wd_test_number_punct))
        .build()
}

// -----------------------------------------------------------------------------
// TestWordDelimiterGraphFilter
// -----------------------------------------------------------------------------

/// `new WordDelimiterGraphFilter(new CannedTokenStream(new Token(text,
/// start, end)), true, DEFAULT_WORD_DELIM_TABLE, flags, null)`.
fn wdgf_canned(text: &str, start: i32, end: i32, flags: i32) -> Result<Box<dyn TokenStream>> {
    Ok(boxed(wdgf::new_word_delimiter_graph_filter_with(
        boxed(CannedTokenStream::new(vec![Token::with_offsets(
            text, start, end,
        )])),
        true,
        Arc::clone(&DEFAULT_WORD_DELIM_TABLE),
        flags,
        None,
    )?))
}

fn wdg_test_offsets() -> Result<()> {
    // test that subwords and catenated subwords have
    // the correct offsets.
    let mut f = wdgf_canned("foo-bar", 5, 12, WDF_FLAGS)?;
    B::assert_token_stream_contents_offsets(
        &mut *f,
        &["foobar", "foo", "bar"],
        &[5, 5, 9],
        &[12, 8, 12],
    )?;
    // with illegal offsets:
    let mut f = wdgf_canned("foo-bar", 5, 6, WDF_FLAGS)?;
    B::assert_token_stream_contents_offsets(
        &mut *f,
        &["foobar", "foo", "bar"],
        &[5, 5, 5],
        &[6, 6, 6],
    )
}

fn wdg_test_offset_change() -> Result<()> {
    let mut f = wdgf_canned("übelkeit)", 7, 16, WDF_FLAGS)?;
    B::assert_token_stream_contents_offsets(&mut *f, &["übelkeit"], &[7], &[15])
}

fn wdg_test_offset_change2() -> Result<()> {
    let mut f = wdgf_canned("(übelkeit", 7, 17, WDF_FLAGS)?;
    // illegal offsets:
    B::assert_token_stream_contents_offsets(&mut *f, &["übelkeit"], &[7], &[17])
}

fn wdg_test_offset_change3() -> Result<()> {
    let mut f = wdgf_canned("(übelkeit", 7, 16, WDF_FLAGS)?;
    B::assert_token_stream_contents_offsets(&mut *f, &["übelkeit"], &[8], &[16])
}

fn wdg_test_offset_change4() -> Result<()> {
    let mut f = wdgf_canned("(foo,bar)", 7, 16, WDF_FLAGS)?;
    B::assert_token_stream_contents_offsets(
        &mut *f,
        &["foobar", "foo", "bar"],
        &[8, 8, 12],
        &[15, 11, 15],
    )
}

fn wdg_do_split(input: &str, output: &[&str]) -> Result<()> {
    let mut f = wdgf::new_word_delimiter_graph_filter_with(
        boxed(B::keyword_mock_tokenizer_str(input)?),
        false,
        Arc::clone(&DEFAULT_WORD_DELIM_TABLE),
        SPLIT_FLAGS,
        None,
    )?;
    B::assert_token_stream_contents(&mut f, output)
}

fn wdg_test_splits() -> Result<()> {
    for (input, output) in SPLITS {
        wdg_do_split(input, output)?;
    }
    Ok(())
}

fn wdg_do_split_possessive(stem_possessive: i32, input: &str, output: &[&str]) -> Result<()> {
    let mut flags = wdgf::GENERATE_WORD_PARTS
        | wdgf::GENERATE_NUMBER_PARTS
        | wdgf::SPLIT_ON_CASE_CHANGE
        | wdgf::SPLIT_ON_NUMERICS;
    flags |= if stem_possessive == 1 {
        wdgf::STEM_ENGLISH_POSSESSIVE
    } else {
        0
    };
    let mut f = wdgf::new_word_delimiter_graph_filter(
        boxed(B::keyword_mock_tokenizer_str(input)?),
        flags,
        None,
    )?;
    B::assert_token_stream_contents(&mut f, output)
}

/// Test option that allows disabling the special "'s" stemming, instead
/// treating the single quote like other delimiters.
fn wdg_test_possessives() -> Result<()> {
    wdg_do_split_possessive(1, "ra's", &["ra"])?;
    wdg_do_split_possessive(0, "ra's", &["ra", "s"])
}

fn wdg_test_token_type() -> Result<()> {
    // test that subwords and catenated subwords have
    // the correct offsets.
    let mut token = Token::with_offsets("foo-bar", 5, 12);
    rucene::analysis::tokenattributes::TypeAttribute::set_type(&mut token, "mytype".into());
    let mut f = wdgf::new_word_delimiter_graph_filter(
        boxed(CannedTokenStream::new(vec![token])),
        WDF_FLAGS,
        None,
    )?;
    B::assert_token_stream_contents_types_only(
        &mut f,
        &["foobar", "foo", "bar"],
        &["mytype", "mytype", "mytype"],
    )
}

fn wdg_test_position_increments() -> Result<()> {
    let a4 = whitespace_analyzer(
        "org.apache.lucene.analysis.miscellaneous.TestWordDelimiterGraphFilter$1",
        |t| {
            let flags = wdgf::SPLIT_ON_NUMERICS
                | wdgf::GENERATE_WORD_PARTS
                | wdgf::PRESERVE_ORIGINAL
                | wdgf::GENERATE_NUMBER_PARTS
                | wdgf::SPLIT_ON_CASE_CHANGE;
            Ok(boxed(wdgf::new_word_delimiter_graph_filter(
                t,
                flags,
                Some(CharArraySet::empty_set()),
            )?))
        },
    );
    B::assert_analyzes_to_pos_increments(
        &*a4,
        "SAL_S8371 - SAL",
        &["SAL_S8371", "SAL", "S", "8371", "-", "SAL"],
        &[1, 0, 1, 1, 1, 1],
    )?;
    let flags = WDF_FLAGS;
    let prot_words = char_set(&["NUTCH"], false);
    // analyzer that uses whitespace + wdf
    let pw = prot_words.clone();
    let a = whitespace_analyzer(
        "org.apache.lucene.analysis.miscellaneous.TestWordDelimiterGraphFilter$2",
        move |t| {
            Ok(boxed(wdgf::new_word_delimiter_graph_filter_with(
                t,
                true,
                Arc::clone(&DEFAULT_WORD_DELIM_TABLE),
                flags,
                Some(pw.clone()),
            )?))
        },
    );
    // in this case, works as expected.
    analyzes_graph(
        &*a,
        "LUCENE / SOLR",
        &["LUCENE", "SOLR"],
        &[0, 9],
        &[6, 13],
        &[1, 2],
    )?;
    // only in this case, posInc of 2 ?!
    analyzes_graph(
        &*a,
        "LUCENE / solR",
        &["LUCENE", "solR", "sol", "R"],
        &[0, 9, 9, 12],
        &[6, 13, 12, 13],
        &[1, 2, 0, 1],
    )?;
    analyzes_graph(
        &*a,
        "LUCENE / NUTCH SOLR",
        &["LUCENE", "NUTCH", "SOLR"],
        &[0, 9, 15],
        &[6, 14, 19],
        &[1, 2, 1],
    )?;
    // analyzer that will consume tokens with large position increments
    let pw = prot_words.clone();
    let a2 = whitespace_analyzer(
        "org.apache.lucene.analysis.miscellaneous.TestWordDelimiterGraphFilter$3",
        move |t| {
            Ok(boxed(wdgf::new_word_delimiter_graph_filter_with(
                large_pos_inc(t)?,
                true,
                Arc::clone(&DEFAULT_WORD_DELIM_TABLE),
                flags,
                Some(pw.clone()),
            )?))
        },
    );
    // increment of "largegap" is preserved
    analyzes_graph(
        &*a2,
        "LUCENE largegap SOLR",
        &["LUCENE", "largegap", "SOLR"],
        &[0, 7, 16],
        &[6, 15, 20],
        &[1, 10, 1],
    )?;
    // the "/" had a position increment of 10, where did it go?!?!!
    analyzes_graph(
        &*a2,
        "LUCENE / SOLR",
        &["LUCENE", "SOLR"],
        &[0, 9],
        &[6, 13],
        &[1, 11],
    )?;
    // in this case, the increment of 10 from the "/" is carried over
    analyzes_graph(
        &*a2,
        "LUCENE / solR",
        &["LUCENE", "solR", "sol", "R"],
        &[0, 9, 9, 12],
        &[6, 13, 12, 13],
        &[1, 11, 0, 1],
    )?;
    analyzes_graph(
        &*a2,
        "LUCENE / NUTCH SOLR",
        &["LUCENE", "NUTCH", "SOLR"],
        &[0, 9, 15],
        &[6, 14, 19],
        &[1, 11, 1],
    )?;
    let pw = prot_words.clone();
    let a3 = whitespace_analyzer(
        "org.apache.lucene.analysis.miscellaneous.TestWordDelimiterGraphFilter$4",
        move |t| {
            let filter = new_stop_filter(t, ENGLISH_STOP_WORDS_SET.clone())?;
            Ok(boxed(wdgf::new_word_delimiter_graph_filter_with(
                boxed(filter),
                true,
                Arc::clone(&DEFAULT_WORD_DELIM_TABLE),
                flags,
                Some(pw.clone()),
            )?))
        },
    );
    analyzes_graph(
        &*a3,
        "lucene.solr",
        &["lucenesolr", "lucene", "solr"],
        &[0, 0, 7],
        &[11, 6, 11],
        &[1, 0, 1],
    )?;
    // the stopword should add a gap here
    analyzes_graph(
        &*a3,
        "the lucene.solr",
        &["lucenesolr", "lucene", "solr"],
        &[4, 4, 11],
        &[15, 10, 15],
        &[2, 0, 1],
    )?;
    a.close();
    a2.close();
    a3.close();
    a4.close();
    Ok(())
}

/// `keywordTestAnalyzer(flags)` of `TestWordDelimiterGraphFilter`.
fn wdg_keyword_test_analyzer(flags: i32) -> Arc<dyn Analyzer> {
    whitespace_analyzer(
        "org.apache.lucene.analysis.miscellaneous.TestWordDelimiterGraphFilter$5",
        move |t| {
            Ok(boxed(wdgf::new_word_delimiter_graph_filter(
                k_filter(t)?,
                flags,
                None,
            )?))
        },
    )
}

fn wdg_test_keyword_filter() -> Result<()> {
    B::assert_analyzes_to(
        &*wdg_keyword_test_analyzer(wdgf::GENERATE_WORD_PARTS),
        "abc-def klm-nop kpop",
        &["abc", "def", "klm", "nop", "kpop"],
    )?;
    analyzes_graph(
        &*wdg_keyword_test_analyzer(wdgf::GENERATE_WORD_PARTS | wdgf::IGNORE_KEYWORDS),
        "abc-def klm-nop kpop",
        &["abc", "def", "klm-nop", "kpop"],
        &[0, 0, 8, 16],
        &[7, 7, 15, 20],
        &[1, 1, 1, 1],
    )
}

fn wdg_test_original_token_emitted_first() -> Result<()> {
    let flags = wdgf::PRESERVE_ORIGINAL
        | wdgf::GENERATE_WORD_PARTS
        | wdgf::GENERATE_NUMBER_PARTS
        | wdgf::CATENATE_WORDS
        | wdgf::CATENATE_NUMBERS
        | wdgf::CATENATE_ALL
        | wdgf::SPLIT_ON_CASE_CHANGE
        | wdgf::SPLIT_ON_NUMERICS
        | wdgf::STEM_ENGLISH_POSSESSIVE;
    // analyzer that uses whitespace + wdf
    let a = whitespace_analyzer(
        "org.apache.lucene.analysis.miscellaneous.TestWordDelimiterGraphFilter$6",
        move |t| {
            Ok(boxed(wdgf::new_word_delimiter_graph_filter_with(
                t,
                true,
                Arc::clone(&DEFAULT_WORD_DELIM_TABLE),
                flags,
                None,
            )?))
        },
    );
    B::assert_analyzes_to(
        &*a,
        "abc-def abcDEF abc123",
        &[
            "abc-def", "abcdef", "abc", "def", "abcDEF", "abcDEF", "abc", "DEF", "abc123",
            "abc123", "abc", "123",
        ],
    )?;
    a.close();
    Ok(())
}

// https://issues.apache.org/jira/browse/LUCENE-9006
fn wdg_test_catenate_all_emitted_before_parts() -> Result<()> {
    // no number parts
    let flags = wdgf::PRESERVE_ORIGINAL | wdgf::GENERATE_WORD_PARTS | wdgf::CATENATE_ALL;
    let use_char_filter = true;
    let graph_offsets_are_correct = false; // note: could solve via always incrementing wordPos on first word ('8')
                                           // not using getAnalyzer because we want adjustInternalOffsets=true
    let a = whitespace_analyzer(
        "org.apache.lucene.analysis.miscellaneous.TestWordDelimiterGraphFilter$7",
        move |t| {
            Ok(boxed(wdgf::new_word_delimiter_graph_filter_with(
                t,
                true,
                Arc::clone(&DEFAULT_WORD_DELIM_TABLE),
                flags,
                None,
            )?))
        },
    );
    // input starts with a number, but we don't generate numbers.
    //   Nonetheless preserve-original and concatenate-all show up first.
    let ts = a.token_stream("dummy", Box::new(StringCharReader::new("8-other")))?;
    B::assert_token_stream_contents_offsets_pos_increments(
        &mut *ts.borrow_mut(),
        &["8-other", "8other", "other"],
        &[0, 0, 2],
        &[7, 7, 7],
        &[1, 0, 0],
    )?;
    B::check_analysis_consistency_graph(
        &random(),
        &*a,
        use_char_filter,
        "8-other",
        graph_offsets_are_correct,
    )?;
    wdg_verify("8-other", flags)?; // uses getAnalyzer which uses adjustInternalOffsets=false which works
                                   // input ends with a number, but we don't generate numbers
    let ts = a.token_stream("dummy", Box::new(StringCharReader::new("other-9")))?;
    B::assert_token_stream_contents_offsets_pos_increments(
        &mut *ts.borrow_mut(),
        &["other-9", "other9", "other"],
        &[0, 0, 0],
        &[7, 7, 5],
        &[1, 0, 0],
    )?;
    B::check_analysis_consistency_graph(
        &random(),
        &*a,
        use_char_filter,
        "other-9",
        graph_offsets_are_correct,
    )?;
    wdg_verify("9-other", flags)?; // uses getAnalyzer which uses adjustInternalOffsets=false which works
    a.close();
    Ok(())
}

/// concat numbers + words + all
fn wdg_test_lots_of_concatenating() -> Result<()> {
    let flags = wdgf::GENERATE_WORD_PARTS
        | wdgf::GENERATE_NUMBER_PARTS
        | wdgf::CATENATE_WORDS
        | wdgf::CATENATE_NUMBERS
        | wdgf::CATENATE_ALL
        | wdgf::SPLIT_ON_CASE_CHANGE
        | wdgf::SPLIT_ON_NUMERICS
        | wdgf::STEM_ENGLISH_POSSESSIVE;
    // analyzer that uses whitespace + wdf
    let a = whitespace_analyzer(
        "org.apache.lucene.analysis.miscellaneous.TestWordDelimiterGraphFilter$8",
        move |t| {
            Ok(boxed(wdgf::new_word_delimiter_graph_filter_with(
                t,
                true,
                Arc::clone(&DEFAULT_WORD_DELIM_TABLE),
                flags,
                None,
            )?))
        },
    );
    analyzes_graph(
        &*a,
        "abc-def-123-456",
        &[
            "abcdef123456",
            "abcdef",
            "abc",
            "def",
            "123456",
            "123",
            "456",
        ],
        &[0, 0, 0, 4, 8, 8, 12],
        &[15, 7, 3, 7, 15, 11, 15],
        &[1, 0, 0, 1, 1, 0, 1],
    )?;
    a.close();
    Ok(())
}

/// concat numbers + words + all + preserve original
fn wdg_test_lots_of_concatenating2() -> Result<()> {
    let flags = wdgf::PRESERVE_ORIGINAL
        | wdgf::GENERATE_WORD_PARTS
        | wdgf::GENERATE_NUMBER_PARTS
        | wdgf::CATENATE_WORDS
        | wdgf::CATENATE_NUMBERS
        | wdgf::CATENATE_ALL
        | wdgf::SPLIT_ON_CASE_CHANGE
        | wdgf::SPLIT_ON_NUMERICS
        | wdgf::STEM_ENGLISH_POSSESSIVE;
    // analyzer that uses whitespace + wdf
    let a = whitespace_analyzer(
        "org.apache.lucene.analysis.miscellaneous.TestWordDelimiterGraphFilter$9",
        move |t| {
            Ok(boxed(wdgf::new_word_delimiter_graph_filter(
                t, flags, None,
            )?))
        },
    );
    analyzes_graph(
        &*a,
        "abc-def-123-456",
        &[
            "abc-def-123-456",
            "abcdef123456",
            "abcdef",
            "abc",
            "def",
            "123456",
            "123",
            "456",
        ],
        &[0, 0, 0, 0, 0, 0, 0, 0],
        &[15, 15, 15, 15, 15, 15, 15, 15],
        &[1, 0, 0, 0, 1, 1, 0, 1],
    )?;
    a.close();
    Ok(())
}

/// blast some random strings through the analyzer
fn wdg_test_random_strings() -> Result<()> {
    let num_iterations = LuceneTestCase::at_least(3);
    for _ in 0..num_iterations {
        let flags = random().next_int_bound(512);
        let protected_words = random_protected_words(&*random());
        let a = whitespace_analyzer(
            "org.apache.lucene.analysis.miscellaneous.TestWordDelimiterGraphFilter$10",
            move |t| {
                Ok(boxed(wdgf::new_word_delimiter_graph_filter(
                    t,
                    flags,
                    protected_words.clone(),
                )?))
            },
        );
        // TODO: properly support positionLengthAttribute
        B::check_random_data_graph(&*random(), &a, 100 * mult(), 20, false, false)?;
        a.close();
    }
    Ok(())
}

/// blast some enormous random strings through the analyzer
fn wdg_test_random_huge_strings() -> Result<()> {
    let num_iterations = LuceneTestCase::at_least(1);
    for _ in 0..num_iterations {
        let flags = random().next_int_bound(512);
        let protected_words = random_protected_words(&*random());
        let a = whitespace_analyzer(
            "org.apache.lucene.analysis.miscellaneous.TestWordDelimiterGraphFilter$11",
            move |t| {
                Ok(boxed(wdgf::new_word_delimiter_graph_filter(
                    t,
                    flags,
                    protected_words.clone(),
                )?))
            },
        );
        // TODO: properly support positionLengthAttribute
        B::check_random_data_graph(&*random(), &a, 10 * mult(), 8192, false, false)?;
        a.close();
    }
    Ok(())
}

fn wdg_test_empty_term() -> Result<()> {
    let random = random();
    for i in 0..512 {
        let flags = i;
        let protected_words = random_protected_words(&*random);
        let a = keyword_tokenizer_analyzer(
            "org.apache.lucene.analysis.miscellaneous.TestWordDelimiterGraphFilter$12",
            move |t| {
                Ok(boxed(wdgf::new_word_delimiter_graph_filter(
                    t,
                    flags,
                    protected_words.clone(),
                )?))
            },
        );
        // depending upon options, this thing may or may not preserve the empty term
        let use_char_filter = random.next_boolean();
        B::check_analysis_consistency(&random, &*a, use_char_filter, "")?;
        a.close();
    }
    Ok(())
}

/// `getAnalyzer(flags)`: the keyword tokenizer and the graph filter.
fn get_analyzer(flags: i32) -> Arc<dyn Analyzer> {
    get_analyzer_protected(flags, None)
}

/// `getAnalyzer(flags, protectedWords)`.
fn get_analyzer_protected(flags: i32, protected_words: Option<CharArraySet>) -> Arc<dyn Analyzer> {
    keyword_tokenizer_analyzer(
        "org.apache.lucene.analysis.miscellaneous.TestWordDelimiterGraphFilter$13",
        move |t| {
            Ok(boxed(wdgf::new_word_delimiter_graph_filter(
                t,
                flags,
                protected_words.clone(),
            )?))
        },
    )
}

fn has(flags: i32, flag: i32) -> bool {
    (flags & flag) != 0
}

/// `WordPart`.
#[derive(Debug, Clone)]
struct WordPart {
    part: Vec<u16>,
    #[allow(dead_code)]
    start_offset: usize,
    #[allow(dead_code)]
    end_offset: usize,
    type_: i32,
}

impl WordPart {
    fn new(text: &[u16], start_offset: usize, end_offset: usize) -> WordPart {
        let part = text[start_offset..end_offset].to_vec();
        let type_ = to_type(part[0]);
        WordPart {
            part,
            start_offset,
            end_offset,
            type_,
        }
    }
}

const NUMBER: i32 = 0;
const LETTER: i32 = 1;
const DELIM: i32 = 2;

fn to_type(ch: u16) -> i32 {
    if character::is_digit(i32::from(ch)) {
        // numbers
        NUMBER
    } else if character::is_letter(i32::from(ch)) {
        // letters
        LETTER
    } else {
        // delimiter
        DELIM
    }
}

/// Does (hopefully) the same thing as WordDelimiterGraphFilter, according to
/// the flags, but more slowly, returning all string paths combinations.
fn slow_wdf(text: &str, flags: i32) -> HashSet<String> {
    let text: Vec<u16> = text.encode_utf16().collect();
    // first make word parts:
    let mut word_parts: Vec<WordPart> = Vec::new();
    let mut last_ch: i32 = -1;
    let mut word_part_start = 0;
    let mut in_token = false;
    let mut i = 0;
    while i < text.len() {
        let ch = text[i];
        if to_type(ch) == DELIM {
            // delimiter
            if in_token {
                // end current token
                word_parts.push(WordPart::new(&text, word_part_start, i));
                in_token = false;
            }
            // strip english possessive at the end of this token?:
            if has(flags, wdgf::STEM_ENGLISH_POSSESSIVE)
                && ch == u16::from(b'\'')
                && i > 0
                && i < text.len() - 1
                && (text[i + 1] == u16::from(b's') || text[i + 1] == u16::from(b'S'))
                && to_type(text[i - 1]) == LETTER
                && (i + 2 == text.len() || to_type(text[i + 2]) == DELIM)
            {
                i += 2;
            }
        } else if !in_token {
            // start new token
            in_token = true;
            word_part_start = i;
        } else {
            let mut new_token = false;
            if character::is_letter(last_ch) {
                if character::is_letter(i32::from(ch)) {
                    if has(flags, wdgf::SPLIT_ON_CASE_CHANGE)
                        && character::is_lower_case(last_ch)
                        && !character::is_lower_case(i32::from(ch))
                    {
                        // start new token on lower -> UPPER case change (but not vice versa!)
                        new_token = true;
                    }
                } else if has(flags, wdgf::SPLIT_ON_NUMERICS) && character::is_digit(i32::from(ch))
                {
                    // start new token on letter -> number change
                    new_token = true;
                }
            } else {
                assert!(character::is_digit(last_ch));
                if character::is_letter(i32::from(ch)) && has(flags, wdgf::SPLIT_ON_NUMERICS) {
                    // start new token on number -> letter change
                    new_token = true;
                }
            }
            if new_token {
                word_parts.push(WordPart::new(&text, word_part_start, i));
                word_part_start = i;
            }
        }
        last_ch = i32::from(ch);
        i += 1;
    }
    if in_token {
        // add last token
        word_parts.push(WordPart::new(&text, word_part_start, text.len()));
    }
    let mut paths: HashSet<String> = HashSet::new();
    if !word_parts.is_empty() {
        enumerate(flags, 0, &word_parts, &mut paths, &mut Vec::new());
    }
    if has(flags, wdgf::PRESERVE_ORIGINAL) {
        paths.insert(String::from_utf16(&text).expect("valid UTF-16"));
    }
    if has(flags, wdgf::CATENATE_ALL) && !word_parts.is_empty() {
        let mut b: Vec<u16> = Vec::new();
        for word_part in &word_parts {
            b.extend_from_slice(&word_part.part);
        }
        paths.insert(String::from_utf16(&b).expect("valid UTF-16"));
    }
    paths
}

fn add_part(path: &mut Vec<u16>, part: &[u16]) {
    if !path.is_empty() {
        path.push(u16::from(b' '));
    }
    path.extend_from_slice(part);
}

fn add_range(path: &mut Vec<u16>, word_parts: &[WordPart], from: usize, to: usize) {
    if !path.is_empty() {
        path.push(u16::from(b' '));
    }
    // no spaces:
    for wp in &word_parts[from..to] {
        path.extend_from_slice(&wp.part);
    }
}

fn add_with_spaces(path: &mut Vec<u16>, word_parts: &[WordPart], from: usize, to: usize) {
    for wp in &word_parts[from..to] {
        add_part(path, &wp.part);
    }
}

/// Finds the end (exclusive) of the series of part with the same type
fn end_of_run(word_parts: &[WordPart], start: usize) -> usize {
    let mut upto = start + 1;
    while upto < word_parts.len() && word_parts[upto].type_ == word_parts[start].type_ {
        upto += 1;
    }
    upto
}

/// Recursively enumerates all paths through the word parts
fn enumerate(
    flags: i32,
    upto: usize,
    word_parts: &[WordPart],
    paths: &mut HashSet<String>,
    path: &mut Vec<u16>,
) {
    if upto == word_parts.len() {
        if !path.is_empty() {
            paths.insert(String::from_utf16(path).expect("valid UTF-16"));
        }
    } else {
        let sav_length = path.len();
        let end = end_of_run(word_parts, upto);
        let (generate, catenate) = if word_parts[upto].type_ == NUMBER {
            (wdgf::GENERATE_NUMBER_PARTS, wdgf::CATENATE_NUMBERS)
        } else {
            assert_eq!(LETTER, word_parts[upto].type_);
            (wdgf::GENERATE_WORD_PARTS, wdgf::CATENATE_WORDS)
        };
        // always output single word, optionally surrounded by delims:
        if has(flags, generate) || word_parts.len() == 1 {
            add_with_spaces(path, word_parts, upto, end);
            if has(flags, catenate) {
                // recurse first with the parts
                enumerate(flags, end, word_parts, paths, path);
                path.truncate(sav_length);
                // .. and second with the concat
                add_range(path, word_parts, upto, end);
            }
        } else if has(flags, catenate) {
            add_range(path, word_parts, upto, end);
        }
        enumerate(flags, end, word_parts, paths, path);
        path.truncate(sav_length);
    }
}

fn wdg_test_basic_graph_splits() -> Result<()> {
    use wdgf::{
        CATENATE_NUMBERS, CATENATE_WORDS, GENERATE_NUMBER_PARTS, GENERATE_WORD_PARTS,
        PRESERVE_ORIGINAL, SPLIT_ON_CASE_CHANGE,
    };
    let g = |flags: i32, text: &str, expected: &[&str]| -> Result<()> {
        B::assert_graph_strings(&*get_analyzer(flags), text, expected)
    };
    g(0, "PowerShotPlus", &["PowerShotPlus"])?;
    g(GENERATE_WORD_PARTS, "PowerShotPlus", &["PowerShotPlus"])?;
    g(
        GENERATE_WORD_PARTS | SPLIT_ON_CASE_CHANGE,
        "PowerShotPlus",
        &["Power Shot Plus"],
    )?;
    g(
        GENERATE_WORD_PARTS | SPLIT_ON_CASE_CHANGE | PRESERVE_ORIGINAL,
        "PowerShotPlus",
        &["PowerShotPlus", "Power Shot Plus"],
    )?;
    g(GENERATE_WORD_PARTS, "Power-Shot-Plus", &["Power Shot Plus"])?;
    g(
        GENERATE_WORD_PARTS | SPLIT_ON_CASE_CHANGE,
        "Power-Shot-Plus",
        &["Power Shot Plus"],
    )?;
    g(
        GENERATE_WORD_PARTS | SPLIT_ON_CASE_CHANGE | PRESERVE_ORIGINAL,
        "Power-Shot-Plus",
        &["Power-Shot-Plus", "Power Shot Plus"],
    )?;
    g(
        GENERATE_WORD_PARTS | SPLIT_ON_CASE_CHANGE,
        "PowerShotPlus",
        &["Power Shot Plus"],
    )?;
    g(
        GENERATE_WORD_PARTS | SPLIT_ON_CASE_CHANGE,
        "PowerShot1000Plus",
        &["Power Shot1000Plus"],
    )?;
    g(
        GENERATE_WORD_PARTS | SPLIT_ON_CASE_CHANGE,
        "Power-Shot-Plus",
        &["Power Shot Plus"],
    )?;
    g(
        GENERATE_WORD_PARTS | SPLIT_ON_CASE_CHANGE | CATENATE_WORDS,
        "PowerShotPlus",
        &["Power Shot Plus", "PowerShotPlus"],
    )?;
    g(
        GENERATE_WORD_PARTS | SPLIT_ON_CASE_CHANGE | CATENATE_WORDS,
        "PowerShot1000Plus",
        &["Power Shot1000Plus", "PowerShot1000Plus"],
    )?;
    g(
        GENERATE_WORD_PARTS
            | GENERATE_NUMBER_PARTS
            | SPLIT_ON_CASE_CHANGE
            | CATENATE_WORDS
            | CATENATE_NUMBERS,
        "Power-Shot-1000-17-Plus",
        &[
            "Power Shot 1000 17 Plus",
            "Power Shot 100017 Plus",
            "PowerShot 1000 17 Plus",
            "PowerShot 100017 Plus",
        ],
    )?;
    g(
        GENERATE_WORD_PARTS
            | GENERATE_NUMBER_PARTS
            | SPLIT_ON_CASE_CHANGE
            | CATENATE_WORDS
            | CATENATE_NUMBERS
            | PRESERVE_ORIGINAL,
        "Power-Shot-1000-17-Plus",
        &[
            "Power-Shot-1000-17-Plus",
            "Power Shot 1000 17 Plus",
            "Power Shot 100017 Plus",
            "PowerShot 1000 17 Plus",
            "PowerShot 100017 Plus",
        ],
    )
}

fn random_wdf_text() -> String {
    let mut b = String::new();
    let length = TestUtil::next_int(&*random(), 1, 50);
    for _ in 0..length {
        let surprise_me = random().next_int_bound(37);
        let (lower, upper) = if surprise_me < 10 {
            // lowercase letter
            (b'a' as i32, b'z' as i32)
        } else if surprise_me < 20 {
            // uppercase letter
            (b'A' as i32, b'Z' as i32)
        } else if surprise_me < 30 {
            // digit
            (b'0' as i32, b'9' as i32)
        } else if surprise_me < 35 {
            // punct
            (b'-' as i32, b'-' as i32)
        } else {
            b.push_str("'s");
            (-1, -1)
        };
        if lower != -1 {
            b.push(char::from(
                TestUtil::next_int(&*random(), lower, upper) as u8
            ));
        }
    }
    b
}

fn wdg_test_invalid_flag() -> Result<()> {
    expect_throws("java.lang.IllegalArgumentException", || {
        wdgf::new_word_delimiter_graph_filter(boxed(CannedTokenStream::new(vec![])), 1 << 31, None)
    });
    Ok(())
}

fn wdg_test_random_paths() -> Result<()> {
    let iters = LuceneTestCase::at_least(10);
    for _ in 0..iters {
        let text = random_wdf_text();
        if *VERBOSE {
            println!("\nTEST: text={} len={}", text, text.len());
        }
        let mut flags = 0;
        for flag in [
            wdgf::GENERATE_WORD_PARTS,
            wdgf::GENERATE_NUMBER_PARTS,
            wdgf::CATENATE_WORDS,
            wdgf::CATENATE_NUMBERS,
            wdgf::CATENATE_ALL,
            wdgf::PRESERVE_ORIGINAL,
            wdgf::SPLIT_ON_CASE_CHANGE,
            wdgf::SPLIT_ON_NUMERICS,
            wdgf::STEM_ENGLISH_POSSESSIVE,
        ] {
            if random().next_boolean() {
                flags |= flag;
            }
        }
        wdg_verify(&text, flags)?;
    }
    Ok(())
}

/// Runs normal and slow WDGF and compares results
fn wdg_verify(text: &str, flags: i32) -> Result<()> {
    let expected = slow_wdf(text, flags);
    if *VERBOSE {
        for path in &expected {
            println!("  {path}");
        }
    }
    let actual = B::get_graph_strings(&*get_analyzer(flags), text)?;
    if actual != expected {
        let mut b = String::new();
        b.push_str("\n\nFAIL: text=");
        b.push_str(text);
        b.push_str(" flags=");
        b.push_str(&wdgf::flags_to_string(flags));
        b.push('\n');
        b.push_str("  expected paths:\n");
        for s in &expected {
            b.push_str("    ");
            b.push_str(s);
            if !actual.contains(s) {
                b.push_str(" [missing!]");
            }
            b.push('\n');
        }
        b.push_str("  actual paths:\n");
        for s in &actual {
            b.push_str("    ");
            b.push_str(s);
            if !expected.contains(s) {
                b.push_str(" [unexpected!]");
            }
            b.push('\n');
        }
        Assert::fail(&b);
    }
    let use_char_filter = true;
    B::check_analysis_consistency(&random(), &*get_analyzer(flags), use_char_filter, text)
}

fn wdg_test_only_numbers() -> Result<()> {
    // no token should be produced
    B::assert_graph_strings(
        &*get_analyzer(
            wdgf::GENERATE_WORD_PARTS | wdgf::SPLIT_ON_CASE_CHANGE | wdgf::SPLIT_ON_NUMERICS,
        ),
        "7-586",
        &[],
    )
}

fn wdg_test_no_catenate() -> Result<()> {
    // no token should be produced
    B::assert_graph_strings(
        &*get_analyzer(
            wdgf::GENERATE_WORD_PARTS
                | wdgf::GENERATE_NUMBER_PARTS
                | wdgf::SPLIT_ON_CASE_CHANGE
                | wdgf::SPLIT_ON_NUMERICS,
        ),
        "a-b-c-9-d",
        &["a b c 9 d"],
    )
}

fn wdg_test_curious_case1() -> Result<()> {
    wdg_verify(
        "u-0L-4836-ip4Gw--13--q7--L07E1",
        wdgf::CATENATE_WORDS
            | wdgf::CATENATE_ALL
            | wdgf::SPLIT_ON_CASE_CHANGE
            | wdgf::SPLIT_ON_NUMERICS
            | wdgf::STEM_ENGLISH_POSSESSIVE,
    )
}

fn wdg_test_curious_case2() -> Result<()> {
    wdg_verify("u-l-p", wdgf::CATENATE_ALL)
}

fn wdg_test_original_pos_length() -> Result<()> {
    wdg_verify(
        "Foo-Bar-Baz",
        wdgf::CATENATE_WORDS | wdgf::SPLIT_ON_CASE_CHANGE | wdgf::PRESERVE_ORIGINAL,
    )
}

fn wdg_test_curious_case3() -> Result<()> {
    wdg_verify(
        "cQzk4-GL0izl0mKM-J8--4m-'s",
        wdgf::GENERATE_NUMBER_PARTS
            | wdgf::CATENATE_NUMBERS
            | wdgf::SPLIT_ON_CASE_CHANGE
            | wdgf::SPLIT_ON_NUMERICS,
    )
}

fn wdg_test_empty_string() -> Result<()> {
    let mut wdf = wdgf::new_word_delimiter_graph_filter(
        boxed(CannedTokenStream::new(vec![Token::with_offsets("", 0, 0)])),
        wdgf::GENERATE_WORD_PARTS | wdgf::CATENATE_ALL | wdgf::PRESERVE_ORIGINAL,
        None,
    )?;
    wdf.reset()?;
    Assert::assert_true(wdf.increment_token()?);
    Assert::assert_false(wdf.increment_token()?);
    wdf.end()?;
    wdf.close()
}

fn wdg_test_protected_words() -> Result<()> {
    let tokens = CannedTokenStream::new(vec![
        Token::with_offsets("foo17-bar", 0, 9),
        Token::with_offsets("foo-bar", 0, 7),
    ]);
    let protected_words = char_set(&["foo17-BAR"], true);
    let mut wdf = wdgf::new_word_delimiter_graph_filter(
        boxed(tokens),
        wdgf::GENERATE_WORD_PARTS | wdgf::PRESERVE_ORIGINAL | wdgf::CATENATE_ALL,
        Some(protected_words),
    )?;
    B::assert_graph_strings_of(
        &mut wdf,
        &["foo17-bar foo bar", "foo17-bar foo-bar", "foo17-bar foobar"],
    )
}

fn word_delimiter_graph_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.miscellaneous.TestWordDelimiterGraphFilter")
        .method(t("testOffsets", wdg_test_offsets))
        .method(t("testOffsetChange", wdg_test_offset_change))
        .method(t("testOffsetChange2", wdg_test_offset_change2))
        .method(t("testOffsetChange3", wdg_test_offset_change3))
        .method(t("testOffsetChange4", wdg_test_offset_change4))
        .method(t("testSplits", wdg_test_splits))
        .method(t("testPossessives", wdg_test_possessives))
        .method(t("testTokenType", wdg_test_token_type))
        .method(t("testPositionIncrements", wdg_test_position_increments))
        .method(t("testKeywordFilter", wdg_test_keyword_filter))
        .method(t(
            "testOriginalTokenEmittedFirst",
            wdg_test_original_token_emitted_first,
        ))
        .method(t(
            "testCatenateAllEmittedBeforeParts",
            wdg_test_catenate_all_emitted_before_parts,
        ))
        .method(t("testLotsOfConcatenating", wdg_test_lots_of_concatenating))
        .method(t(
            "testLotsOfConcatenating2",
            wdg_test_lots_of_concatenating2,
        ))
        .method(t("testRandomStrings", wdg_test_random_strings))
        .method(t("testRandomHugeStrings", wdg_test_random_huge_strings))
        .method(t("testEmptyTerm", wdg_test_empty_term))
        .method(t("testBasicGraphSplits", wdg_test_basic_graph_splits))
        .method(t("testInvalidFlag", wdg_test_invalid_flag))
        .method(t("testRandomPaths", wdg_test_random_paths))
        .method(t("testOnlyNumbers", wdg_test_only_numbers))
        .method(t("testNoCatenate", wdg_test_no_catenate))
        .method(t("testCuriousCase1", wdg_test_curious_case1))
        .method(t("testCuriousCase2", wdg_test_curious_case2))
        .method(t("testOriginalPosLength", wdg_test_original_pos_length))
        .method(t("testCuriousCase3", wdg_test_curious_case3))
        .method(t("testEmptyString", wdg_test_empty_string))
        .method(t("testProtectedWords", wdg_test_protected_words))
        .build()
}

// -----------------------------------------------------------------------------
// Suites
// -----------------------------------------------------------------------------

#[test]
fn test_english_analyzer() {
    run(english_analyzer_class);
}

#[test]
fn test_english_minimal_stem_filter() {
    run(english_minimal_stem_filter_class);
}

#[test]
fn test_english_minimal_stem_filter_factory() {
    run(english_minimal_stem_filter_factory_class);
}

#[test]
fn test_kstem_filter_factory() {
    run(kstem_filter_factory_class);
}

#[test]
fn test_kstemmer() {
    run(kstemmer_class);
}

#[test]
fn test_porter_stem_filter() {
    run(porter_stem_filter_class);
}

#[test]
fn test_porter_stem_filter_factory() {
    run(porter_stem_filter_factory_class);
}

#[test]
fn test_analyzers() {
    run(analyzers_class);
}

#[test]
fn test_stop_analyzer() {
    run(stop_analyzer_class);
}

#[test]
fn test_stop_filter_factory() {
    run(stop_filter_factory_class);
}

#[test]
fn test_stemmer_override_filter() {
    run(stemmer_override_filter_class);
}

#[test]
fn test_word_delimiter_filter() {
    run(word_delimiter_filter_class);
}

#[test]
fn test_word_delimiter_graph_filter() {
    run(word_delimiter_graph_filter_class);
}
