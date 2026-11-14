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
//! that rmp task #464 lists, each run as a suite by the ported runner:
//! `org.apache.lucene.analysis.classic.TestClassicAnalyzer` and
//! `TestClassicFactories`, `org.apache.lucene.analysis.custom.TestCustomAnalyzer`
//! (its resources `teststop.txt`, `mapping1.txt` and `mapping2.txt` copied byte
//! for byte), and `org.apache.lucene.analysis.wikipedia.TestWikipediaTokenizer`
//! and `TestWikipediaTokenizerFactory`.
//!
//! `TestWikipediaTokenizer.checkLinkPhrases(WikipediaTokenizer)` is a helper.
//!
//! **Not expressible in Rust (not ported).** `TestCustomAnalyzer`'s
//! `testNullTokenizer`, `testNullTokenizerFactory`, `testNullParamKey` and
//! `testNullMatchVersion` pass Java `null` for a `String`, a `Class`, a
//! parameter key and a `Version`, expecting `NullPointerException`; the
//! port's `&str`, `&AnalysisFactoryClass`, `&[&str]` and `Version` parameters
//! cannot be null (the `NullPointerException` "has no counterpart", as
//! `CustomAnalyzer`'s docs declare). The last assertion of
//! `testStopWordsFromClasspathWithMap` passes an unmodifiable map, expecting
//! `UnsupportedOperationException`: the port's `&mut HashMap` cannot be
//! immutable, so that assertion is a compile-time guarantee; the rest of the
//! method is ported.
//!
//! `CustomAnalyzer.builder()` reads resources through the class loader of
//! `CustomAnalyzer`; on Java's test class path that loader sees the test
//! resources, so the port builds it with
//! `CustomAnalyzer::builder_with_loader` over the test class loader
//! ([`builder`]).

mod support;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::factories::{
    AbstractAnalysisFactory, AnalysisFactoryClass, CharFilterFactory, FactoryTokenizer,
    SPINameField, TokenFilterFactory, TokenizerFactory,
};
use rucene::analysis::standard::StandardTokenizerFactory;
use rucene::analysis::tokenattributes::FlagsAttribute;
use rucene::analysis::{Analyzer, CharFilter, CharReader, StringCharReader, TokenStream};
use rucene::document::{Document, Store, TextField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::{IndexWriter, MAX_TERM_LENGTH};
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::multi_fields::MultiTerms;
use rucene::index::{IndexReader, Term};
use rucene::search::NO_MORE_DOCS;
use rucene::store::{ByteBuffersDirectory, Directory};
use rucene::util::attribute::AttributeFactory;
use rucene::util::extra::Version;
use rucene::util::resource_loader::ClasspathResourceLoader;
use rucene::util::BytesRef;
use rucene_analysis_common::analysis::charfilter::html_strip_char_filter_factory::HTMLStripCharFilterFactory;
use rucene_analysis_common::analysis::charfilter::mapping_char_filter_factory::MappingCharFilterFactory;
use rucene_analysis_common::analysis::classic::classic_analyzer::ClassicAnalyzer;
use rucene_analysis_common::analysis::classic::classic_tokenizer_factory::ClassicTokenizerFactory;
use rucene_analysis_common::analysis::core::letter_tokenizer::LetterTokenizerLogic;
use rucene_analysis_common::analysis::core::lower_case_filter::LowerCaseFilter;
use rucene_analysis_common::analysis::core::lower_case_filter_factory::LowerCaseFilterFactory;
use rucene_analysis_common::analysis::core::stop_filter_factory::StopFilterFactory;
use rucene_analysis_common::analysis::core::whitespace_tokenizer_factory::WhitespaceTokenizerFactory;
use rucene_analysis_common::analysis::custom::custom_analyzer::{Builder, CustomAnalyzer};
use rucene_analysis_common::analysis::miscellaneous::ascii_folding_filter_factory::ASCIIFoldingFilterFactory;
use rucene_analysis_common::analysis::reverse::reverse_string_filter_factory::ReverseStringFilterFactory;
use rucene_analysis_common::analysis::util::char_tokenizer::{
    char_tokenizer_source, CharTokenizerAdapter,
};
use rucene_analysis_common::analysis::wikipedia::wikipedia_tokenizer::*;
use rucene_test_framework::analysis::{
    tokenizer_components, BaseTokenStreamFactoryTestCase, BaseTokenStreamTestCase,
};
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::RANDOM_MULTIPLIER;
use rucene_test_framework::util::LuceneTestCase;
use support::{btstc_class, factory_class, hook, iae_message, own, run, t, FnAnalyzer};

type B = BaseTokenStreamTestCase;
type F = BaseTokenStreamFactoryTestCase;

fn reader(s: &str) -> Box<dyn CharReader> {
    Box::new(StringCharReader::new(s))
}

// -----------------------------------------------------------------------------
// TestClassicAnalyzer
// -----------------------------------------------------------------------------

thread_local! {
    /// The analyzer field `a` that `setUp` sets.
    static A: std::cell::RefCell<Option<Arc<dyn Analyzer>>> = const { std::cell::RefCell::new(None) };
}

fn field_a() -> Arc<dyn Analyzer> {
    A.with(|a| Arc::clone(a.borrow().as_ref().expect("setUp")))
}

fn ca_set_up() -> Result<()> {
    LuceneTestCase::super_set_up();
    let a: Arc<dyn Analyzer> = Arc::new(ClassicAnalyzer::new());
    A.with(|f| *f.borrow_mut() = Some(a));
    Ok(())
}

fn ca_tear_down() -> Result<()> {
    if let Some(a) = A.with(|a| a.borrow_mut().take()) {
        a.close();
    }
    LuceneTestCase::super_tear_down();
    Ok(())
}

fn ca_test_max_term_length() -> Result<()> {
    let mut sa = ClassicAnalyzer::new();
    sa.set_max_token_length(5);
    B::assert_analyzes_to(&sa, "ab cd toolong xy z", &["ab", "cd", "xy", "z"])?;
    sa.close();
    Ok(())
}

fn ca_test_max_term_length2() -> Result<()> {
    let mut sa = ClassicAnalyzer::new();
    B::assert_analyzes_to(
        &sa,
        "ab cd toolong xy z",
        &["ab", "cd", "toolong", "xy", "z"],
    )?;
    sa.set_max_token_length(5);
    B::assert_analyzes_to_pos_increments(
        &sa,
        "ab cd toolong xy z",
        &["ab", "cd", "xy", "z"],
        &[1, 1, 2, 1],
    )?;
    sa.close();
    Ok(())
}

fn ca_test_max_term_length3() -> Result<()> {
    let a = field_a();
    let long_term = "a".repeat(255);
    B::assert_analyzes_to(
        &*a,
        &format!("ab cd {long_term} xy z"),
        &["ab", "cd", &long_term, "xy", "z"],
    )?;
    B::assert_analyzes_to(
        &*a,
        &format!("ab cd {long_term}a xy z"),
        &["ab", "cd", "xy", "z"],
    )?;
    Ok(())
}

fn ca_test_alphanumeric() -> Result<()> {
    let a = field_a();
    // alphanumeric tokens
    B::assert_analyzes_to(&*a, "B2B", &["b2b"])?;
    B::assert_analyzes_to(&*a, "2B", &["2b"])?;
    Ok(())
}

fn ca_test_underscores() -> Result<()> {
    let a = field_a();
    // underscores are delimiters, but not in email addresses (below)
    B::assert_analyzes_to(
        &*a,
        "word_having_underscore",
        &["word", "having", "underscore"],
    )?;
    B::assert_analyzes_to(
        &*a,
        "word_with_underscore_and_stopwords",
        &["word", "underscore", "stopwords"],
    )?;
    Ok(())
}

fn ca_test_delimiters() -> Result<()> {
    let a = field_a();
    // other delimiters: "-", "/", ","
    B::assert_analyzes_to(&*a, "some-dashed-phrase", &["some", "dashed", "phrase"])?;
    B::assert_analyzes_to(&*a, "dogs,chase,cats", &["dogs", "chase", "cats"])?;
    B::assert_analyzes_to(&*a, "ac/dc", &["ac", "dc"])?;
    Ok(())
}

fn ca_test_apostrophes() -> Result<()> {
    let a = field_a();
    // internal apostrophes: O'Reilly, you're, O'Reilly's
    // possessives are actually removed by StardardFilter, not the tokenizer
    B::assert_analyzes_to(&*a, "O'Reilly", &["o'reilly"])?;
    B::assert_analyzes_to(&*a, "you're", &["you're"])?;
    B::assert_analyzes_to(&*a, "she's", &["she"])?;
    B::assert_analyzes_to(&*a, "Jim's", &["jim"])?;
    B::assert_analyzes_to(&*a, "don't", &["don't"])?;
    B::assert_analyzes_to(&*a, "O'Reilly's", &["o'reilly"])?;
    Ok(())
}

fn ca_test_tsadash() -> Result<()> {
    let a = field_a();
    // t and s had been stopwords in Lucene <= 2.0, which made it impossible
    // to correctly search for these terms:
    B::assert_analyzes_to(&*a, "s-class", &["s", "class"])?;
    B::assert_analyzes_to(&*a, "t-com", &["t", "com"])?;
    // 'a' is still a stopword:
    B::assert_analyzes_to(&*a, "a-class", &["class"])?;
    Ok(())
}

fn ca_test_company_names() -> Result<()> {
    let a = field_a();
    // company names
    B::assert_analyzes_to(&*a, "AT&T", &["at&t"])?;
    B::assert_analyzes_to(&*a, "Excite@Home", &["excite@home"])?;
    Ok(())
}

fn ca_test_lucene1140() -> Result<()> {
    // should not throw NPE
    let analyzer = ClassicAnalyzer::new();
    B::assert_analyzes_to_types_only(&analyzer, "www.nutch.org.", &["www.nutch.org"], &["<HOST>"])?;
    analyzer.close();
    Ok(())
}

fn ca_test_domain_names() -> Result<()> {
    // Current lucene should not show the bug
    let a2 = ClassicAnalyzer::new();
    // domain names
    B::assert_analyzes_to(&a2, "www.nutch.org", &["www.nutch.org"])?;
    // Notice the trailing .  See https://issues.apache.org/jira/browse/LUCENE-1068.
    // the following should be recognized as HOST:
    B::assert_analyzes_to_types_only(&a2, "www.nutch.org.", &["www.nutch.org"], &["<HOST>"])?;
    // 2.3 should show the bug. But, alas, it's obsolete, we don't support it.
    // 2.4 should not show the bug. But, alas, it's also obsolete,
    // so we check latest released (Robert's gonna break this on 4.0 soon :) )
    a2.close();
    let a2 = ClassicAnalyzer::new();
    B::assert_analyzes_to_types_only(&a2, "www.nutch.org.", &["www.nutch.org"], &["<HOST>"])?;
    a2.close();
    Ok(())
}

fn ca_test_email_addresses() -> Result<()> {
    let a = field_a();
    // email addresses, possibly with underscores, periods, etc
    B::assert_analyzes_to(&*a, "test@example.com", &["test@example.com"])?;
    B::assert_analyzes_to(
        &*a,
        "first.lastname@example.com",
        &["first.lastname@example.com"],
    )?;
    B::assert_analyzes_to(
        &*a,
        "first_lastname@example.com",
        &["first_lastname@example.com"],
    )?;
    Ok(())
}

fn ca_test_numeric() -> Result<()> {
    let a = field_a();
    // floating point, serial, model numbers, ip addresses, etc.
    // every other segment must have at least one digit
    B::assert_analyzes_to(&*a, "21.35", &["21.35"])?;
    B::assert_analyzes_to(&*a, "R2D2 C3PO", &["r2d2", "c3po"])?;
    B::assert_analyzes_to(&*a, "216.239.63.104", &["216.239.63.104"])?;
    B::assert_analyzes_to(&*a, "1-2-3", &["1-2-3"])?;
    B::assert_analyzes_to(&*a, "a1-b2-c3", &["a1-b2-c3"])?;
    B::assert_analyzes_to(&*a, "a1-b-c3", &["a1-b-c3"])?;
    Ok(())
}

fn ca_test_text_with_numbers() -> Result<()> {
    let a = field_a();
    // numbers
    B::assert_analyzes_to(
        &*a,
        "David has 5000 bones",
        &["david", "has", "5000", "bones"],
    )?;
    Ok(())
}

fn ca_test_various_text() -> Result<()> {
    let a = field_a();
    // various
    B::assert_analyzes_to(
        &*a,
        "C embedded developers wanted",
        &["c", "embedded", "developers", "wanted"],
    )?;
    B::assert_analyzes_to(&*a, "foo bar FOO BAR", &["foo", "bar", "foo", "bar"])?;
    B::assert_analyzes_to(
        &*a,
        "foo      bar .  FOO <> BAR",
        &["foo", "bar", "foo", "bar"],
    )?;
    B::assert_analyzes_to(&*a, "\"QUOTED\" word", &["quoted", "word"])?;
    Ok(())
}

fn ca_test_acronyms() -> Result<()> {
    let a = field_a();
    // acronyms have their dots stripped
    B::assert_analyzes_to(&*a, "U.S.A.", &["usa"])?;
    Ok(())
}

fn ca_test_cplus_plus_hash() -> Result<()> {
    let a = field_a();
    // It would be nice to change the grammar in StandardTokenizer.jj to make "C#" and "C++" end up
    // as tokens.
    B::assert_analyzes_to(&*a, "C++", &["c"])?;
    B::assert_analyzes_to(&*a, "C#", &["c"])?;
    Ok(())
}

fn ca_test_korean() -> Result<()> {
    let a = field_a();
    // Korean words
    B::assert_analyzes_to(
        &*a,
        "\u{C548}\u{B155}\u{D558}\u{C138}\u{C694} \u{D55C}\u{AE00}\u{C785}\u{B2C8}\u{B2E4}",
        &[
            "\u{C548}\u{B155}\u{D558}\u{C138}\u{C694}",
            "\u{D55C}\u{AE00}\u{C785}\u{B2C8}\u{B2E4}",
        ],
    )?;
    Ok(())
}

fn ca_test_compliance_file_name() -> Result<()> {
    let a = field_a();
    B::assert_analyzes_to_types_only(&*a, "2004.jpg", &["2004.jpg"], &["<HOST>"])?;
    Ok(())
}

fn ca_test_compliance_numeric_incorrect() -> Result<()> {
    let a = field_a();
    B::assert_analyzes_to_types_only(&*a, "62.46", &["62.46"], &["<HOST>"])?;
    Ok(())
}

fn ca_test_compliance_numeric_long() -> Result<()> {
    let a = field_a();
    B::assert_analyzes_to_types_only(&*a, "978-0-94045043-1", &["978-0-94045043-1"], &["<NUM>"])?;
    Ok(())
}

fn ca_test_compliance_numeric_file() -> Result<()> {
    let a = field_a();
    B::assert_analyzes_to_types_only(
        &*a,
        "78academyawards/rules/rule02.html",
        &["78academyawards/rules/rule02.html"],
        &["<NUM>"],
    )?;
    Ok(())
}

fn ca_test_compliance_numeric_with_underscores() -> Result<()> {
    let a = field_a();
    B::assert_analyzes_to_types_only(
        &*a,
        "2006-03-11t082958z_01_ban130523_rtridst_0_ozabs",
        &["2006-03-11t082958z_01_ban130523_rtridst_0_ozabs"],
        &["<NUM>"],
    )?;
    Ok(())
}

fn ca_test_compliance_numeric_with_dash() -> Result<()> {
    let a = field_a();
    B::assert_analyzes_to_types_only(&*a, "mid-20th", &["mid-20th"], &["<NUM>"])?;
    Ok(())
}

fn ca_test_compliance_many_tokens() -> Result<()> {
    let a = field_a();
    B::assert_analyzes_to_types_only(&*a, "/money.cnn.com/magazines/fortune/fortune_archive/2007/03/19/8402357/index.htm safari-0-sheikh-zayed-grand-mosque.jpg", &["money.cnn.com", "magazines", "fortune", "fortune", "archive/2007/03/19/8402357", "index.htm", "safari-0-sheikh", "zayed", "grand", "mosque.jpg"], &["<HOST>", "<ALPHANUM>", "<ALPHANUM>", "<ALPHANUM>", "<NUM>", "<HOST>", "<NUM>", "<ALPHANUM>", "<ALPHANUM>", "<HOST>"])?;
    Ok(())
}

fn ca_test_java14_bwcompatibility() -> Result<()> {
    let sa = ClassicAnalyzer::new();
    B::assert_analyzes_to(&sa, "test\u{2C6}test", &["test", "test"])?;
    sa.close();
    Ok(())
}

fn ca_test_wicked_long_term() -> Result<()> {
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let analyzer: Arc<dyn Analyzer> = Arc::new(ClassicAnalyzer::new());
    let writer = IndexWriter::new(
        Arc::clone(&dir),
        IndexWriterConfig::with_analyzer(Some(Arc::clone(&analyzer))),
    )?;
    let big_term = "x".repeat(MAX_TERM_LENGTH);
    // This produces a too-long term:
    let contents = format!("abc xyz x{big_term} another term");
    let mut doc = Document::new();
    doc.add(Box::new(TextField::new("content", contents, Store::NO)?));
    writer.add_document(&doc)?;
    // Make sure we can add another normal document
    let mut doc = Document::new();
    doc.add(Box::new(TextField::new(
        "content",
        "abc bbb ccc".to_string(),
        Store::NO,
    )?));
    writer.add_document(&doc)?;
    writer.close()?;

    let reader: Arc<dyn IndexReader> = directory_reader::open(Arc::clone(&dir))?;
    // Make sure all terms < max size were indexed
    Assert::assert_equals(2, reader.doc_freq(&Term::from_text("content", "abc"))?);
    Assert::assert_equals(1, reader.doc_freq(&Term::from_text("content", "bbb"))?);
    Assert::assert_equals(1, reader.doc_freq(&Term::from_text("content", "term"))?);
    Assert::assert_equals(1, reader.doc_freq(&Term::from_text("content", "another"))?);

    // Make sure position is still incremented when massive term is skipped:
    let mut tps = MultiTerms::get_term_postings_enum_all(
        &reader,
        "content",
        &BytesRef::from_char_sequence("another"),
    )?
    .expect("postings");
    Assert::assert_true(tps.next_doc()? != NO_MORE_DOCS);
    Assert::assert_equals(1, tps.freq()?);
    Assert::assert_equals(3, tps.next_position()?);

    // Make sure the doc that has the massive term is in the index:
    Assert::assert_equals_msg(
        Some("document with wicked long term should is not in the index!"),
        2,
        reader.num_docs(),
    );
    reader.close()?;

    // Make sure we can add a document with exactly the maximum length term,
    // and search on that term:
    let mut doc = Document::new();
    doc.add(Box::new(TextField::new(
        "content",
        big_term.clone(),
        Store::NO,
    )?));
    let mut sa = ClassicAnalyzer::new();
    sa.set_max_token_length(100000);
    let sa: Arc<dyn Analyzer> = Arc::new(sa);
    let writer = IndexWriter::new(
        Arc::clone(&dir),
        IndexWriterConfig::with_analyzer(Some(Arc::clone(&sa))),
    )?;
    writer.add_document(&doc)?;
    writer.close()?;
    let reader: Arc<dyn IndexReader> = directory_reader::open(Arc::clone(&dir))?;
    Assert::assert_equals(1, reader.doc_freq(&Term::from_text("content", &big_term))?);
    reader.close()?;

    dir.close()?;
    analyzer.close();
    sa.close();
    Ok(())
}

fn ca_test_random_strings() -> Result<()> {
    let analyzer: Arc<dyn Analyzer> = Arc::new(ClassicAnalyzer::new());
    B::check_random_data(
        &*LuceneTestCase::random(),
        &analyzer,
        200 * *RANDOM_MULTIPLIER,
    )?;
    analyzer.close();
    Ok(())
}

fn ca_test_random_huge_strings() -> Result<()> {
    let analyzer: Arc<dyn Analyzer> = Arc::new(ClassicAnalyzer::new());
    B::check_random_data_max_word_length(
        &*LuceneTestCase::random(),
        &analyzer,
        10 * *RANDOM_MULTIPLIER,
        8192,
    )?;
    analyzer.close();
    Ok(())
}
fn classic_analyzer_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.classic.TestClassicAnalyzer")
        .method(hook("setUp", ca_set_up))
        .method(hook("tearDown", ca_tear_down))
        .method(t("testMaxTermLength", ca_test_max_term_length))
        .method(t("testMaxTermLength2", ca_test_max_term_length2))
        .method(t("testMaxTermLength3", ca_test_max_term_length3))
        .method(t("testAlphanumeric", ca_test_alphanumeric))
        .method(t("testUnderscores", ca_test_underscores))
        .method(t("testDelimiters", ca_test_delimiters))
        .method(t("testApostrophes", ca_test_apostrophes))
        .method(t("testTSADash", ca_test_tsadash))
        .method(t("testCompanyNames", ca_test_company_names))
        .method(t("testLucene1140", ca_test_lucene1140))
        .method(t("testDomainNames", ca_test_domain_names))
        .method(t("testEMailAddresses", ca_test_email_addresses))
        .method(t("testNumeric", ca_test_numeric))
        .method(t("testTextWithNumbers", ca_test_text_with_numbers))
        .method(t("testVariousText", ca_test_various_text))
        .method(t("testAcronyms", ca_test_acronyms))
        .method(t("testCPlusPlusHash", ca_test_cplus_plus_hash))
        .method(t("testKorean", ca_test_korean))
        .method(t("testComplianceFileName", ca_test_compliance_file_name))
        .method(t(
            "testComplianceNumericIncorrect",
            ca_test_compliance_numeric_incorrect,
        ))
        .method(t(
            "testComplianceNumericLong",
            ca_test_compliance_numeric_long,
        ))
        .method(t(
            "testComplianceNumericFile",
            ca_test_compliance_numeric_file,
        ))
        .method(t(
            "testComplianceNumericWithUnderscores",
            ca_test_compliance_numeric_with_underscores,
        ))
        .method(t(
            "testComplianceNumericWithDash",
            ca_test_compliance_numeric_with_dash,
        ))
        .method(t(
            "testComplianceManyTokens",
            ca_test_compliance_many_tokens,
        ))
        .method(t(
            "testJava14BWCompatibility",
            ca_test_java14_bwcompatibility,
        ))
        .method(t("testWickedLongTerm", ca_test_wicked_long_term))
        .method(t("testRandomStrings", ca_test_random_strings))
        .method(t("testRandomHugeStrings", ca_test_random_huge_strings))
        .build()
}

// -----------------------------------------------------------------------------
// TestClassicFactories
// -----------------------------------------------------------------------------

/// `tokenizerFactory(name, params).create(newAttributeFactory())` over `text`.
fn factory_tokenizer(name: &str, params: &[&str], text: &str) -> Result<FactoryTokenizer> {
    let mut stream =
        F::tokenizer_factory(name, params)?.create_with_factory(B::new_attribute_factory())?;
    stream.set_reader(reader(text))?;
    Ok(stream)
}

fn cf_test_classic_tokenizer() -> Result<()> {
    let mut stream = factory_tokenizer("Classic", &[], "What's this thing do?")?;
    B::assert_token_stream_contents(&mut stream, &["What's", "this", "thing", "do"])
}

fn cf_test_classic_tokenizer_max_token_length() -> Result<()> {
    let long_word = "abcdefg".repeat(100); // 7 * 100 = 700 char "word"
    let content = format!("one two three {long_word} four five six");
    let mut stream = factory_tokenizer("Classic", &["maxTokenLength", "1000"], &content)?;
    B::assert_token_stream_contents(
        &mut stream,
        &["one", "two", "three", &long_word, "four", "five", "six"],
    )
}

fn cf_test_classic_filter() -> Result<()> {
    let tokenizer = factory_tokenizer("Classic", &[], "What's this thing do?")?;
    let mut stream = F::token_filter_factory("Classic", &[])?.create(Box::new(tokenizer))?;
    B::assert_token_stream_contents(&mut *stream, &["What", "this", "thing", "do"])
}

fn cf_test_bogus_arguments() -> Result<()> {
    let expected = iae_message(|| F::tokenizer_factory("Classic", &["bogusArg", "bogusValue"]));
    Assert::assert_true(expected.contains("Unknown parameters"));
    let expected = iae_message(|| F::token_filter_factory("Classic", &["bogusArg", "bogusValue"]));
    Assert::assert_true(expected.contains("Unknown parameters"));
    Ok(())
}
fn classic_factories_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.classic.TestClassicFactories")
        .method(t("testClassicTokenizer", cf_test_classic_tokenizer))
        .method(t(
            "testClassicTokenizerMaxTokenLength",
            cf_test_classic_tokenizer_max_token_length,
        ))
        .method(t("testClassicFilter", cf_test_classic_filter))
        .method(t("testBogusArguments", cf_test_bogus_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// TestWikipediaTokenizer
// -----------------------------------------------------------------------------

const LINK_PHRASES: &str =
    "click [[link here again]] click [http://lucene.apache.org here again] [[Category:a b c d]]";

/// `new WikipediaTokenizer(newAttributeFactory(), tokenOutput, untoks)` over `text`.
fn wikipedia(token_output: i32, untoks: &[&str], text: &str) -> Result<WikipediaTokenizer> {
    let mut tf = new_wikipedia_tokenizer_with_factory(
        B::new_attribute_factory(),
        token_output,
        untoks
            .iter()
            .map(|s| s.to_string())
            .collect::<HashSet<String>>(),
    )?;
    tf.set_reader(reader(text))?;
    Ok(tf)
}

/// `checkLinkPhrases(tf)`.
fn check_link_phrases(tf: &mut WikipediaTokenizer) -> Result<()> {
    B::assert_token_stream_contents_pos_increments(
        tf,
        &[
            "click",
            "link",
            "here",
            "again",
            "click",
            "http://lucene.apache.org",
            "here",
            "again",
            "a",
            "b",
            "c",
            "d",
        ],
        &[1, 1, 1, 1, 1, 1, 0, 1, 1, 1, 1, 1],
    )
}

/// `new Analyzer() { WikipediaTokenizer(newAttributeFactory(), TOKENS_ONLY, emptySet()) }`.
fn wikipedia_analyzer(name: &'static str) -> Arc<dyn Analyzer> {
    FnAnalyzer::new(name, |_| {
        Ok(tokenizer_components(own(
            new_wikipedia_tokenizer_with_factory(
                B::new_attribute_factory(),
                TOKENS_ONLY,
                HashSet::new(),
            )?,
        )))
    })
    .arc()
}

fn wt_test_simple() -> Result<()> {
    let text = "This is a [[Category:foo]]";
    let mut tf = wikipedia(TOKENS_ONLY, &[], text)?;
    B::assert_token_stream_contents_types_final_offset(
        &mut tf,
        &["This", "is", "a", "foo"],
        Some(&[0, 5, 8, 21]),
        Some(&[4, 7, 9, 24]),
        Some(&["<ALPHANUM>", "<ALPHANUM>", "<ALPHANUM>", CATEGORY]),
        Some(&[1, 1, 1, 1]),
        Some(text.len() as i32),
    )
}

fn wt_test_handwritten() -> Result<()> {
    // make sure all tokens are in only one type
    let test = "[[link]] This is a [[Category:foo]] Category  This is a linked [[:Category:bar none withstanding]] Category This is (parens) This is a [[link]]  This is an external URL [http://lucene.apache.org] Here is ''italics'' and ''more italics'', '''bold''' and '''''five quotes'''''  This is a [[link|display info]]  This is a period.  Here is $3.25 and here is 3.50.  Here's Johnny.  ==heading== ===sub head=== followed by some text  [[Category:blah| ]] ''[[Category:ital_cat]]''  here is some that is ''italics [[Category:foo]] but is never closed.'''same [[Category:foo]] goes for this '''''and2 [[Category:foo]] and this [http://foo.boo.com/test/test/ Test Test] [http://foo.boo.com/test/test/test.html Test Test] [http://foo.boo.com/test/test/test.html?g=b&c=d Test Test] <ref>Citation</ref> <sup>martian</sup> <span class=\"glue\">code</span>";
    let mut tf = wikipedia(TOKENS_ONLY, &[], test)?;
    B::assert_token_stream_contents_types_only(
        &mut tf,
        &[
            "link",
            "This",
            "is",
            "a",
            "foo",
            "Category",
            "This",
            "is",
            "a",
            "linked",
            "bar",
            "none",
            "withstanding",
            "Category",
            "This",
            "is",
            "parens",
            "This",
            "is",
            "a",
            "link",
            "This",
            "is",
            "an",
            "external",
            "URL",
            "http://lucene.apache.org",
            "Here",
            "is",
            "italics",
            "and",
            "more",
            "italics",
            "bold",
            "and",
            "five",
            "quotes",
            "This",
            "is",
            "a",
            "link",
            "display",
            "info",
            "This",
            "is",
            "a",
            "period",
            "Here",
            "is",
            "3.25",
            "and",
            "here",
            "is",
            "3.50",
            "Here's",
            "Johnny",
            "heading",
            "sub",
            "head",
            "followed",
            "by",
            "some",
            "text",
            "blah",
            "ital",
            "cat",
            "here",
            "is",
            "some",
            "that",
            "is",
            "italics",
            "foo",
            "but",
            "is",
            "never",
            "closed",
            "same",
            "foo",
            "goes",
            "for",
            "this",
            "and2",
            "foo",
            "and",
            "this",
            "http://foo.boo.com/test/test/",
            "Test",
            "Test",
            "http://foo.boo.com/test/test/test.html",
            "Test",
            "Test",
            "http://foo.boo.com/test/test/test.html?g=b&c=d",
            "Test",
            "Test",
            "Citation",
            "martian",
            "code",
        ],
        &[
            INTERNAL_LINK,
            "<ALPHANUM>",
            "<ALPHANUM>",
            "<ALPHANUM>",
            CATEGORY,
            "<ALPHANUM>",
            "<ALPHANUM>",
            "<ALPHANUM>",
            "<ALPHANUM>",
            "<ALPHANUM>",
            CATEGORY,
            CATEGORY,
            CATEGORY,
            "<ALPHANUM>",
            "<ALPHANUM>",
            "<ALPHANUM>",
            "<ALPHANUM>",
            "<ALPHANUM>",
            "<ALPHANUM>",
            "<ALPHANUM>",
            INTERNAL_LINK,
            "<ALPHANUM>",
            "<ALPHANUM>",
            "<ALPHANUM>",
            "<ALPHANUM>",
            "<ALPHANUM>",
            EXTERNAL_LINK_URL,
            "<ALPHANUM>",
            "<ALPHANUM>",
            ITALICS,
            "<ALPHANUM>",
            ITALICS,
            ITALICS,
            BOLD,
            "<ALPHANUM>",
            BOLD_ITALICS,
            BOLD_ITALICS,
            "<ALPHANUM>",
            "<ALPHANUM>",
            "<ALPHANUM>",
            INTERNAL_LINK,
            INTERNAL_LINK,
            INTERNAL_LINK,
            "<ALPHANUM>",
            "<ALPHANUM>",
            "<ALPHANUM>",
            "<ALPHANUM>",
            "<ALPHANUM>",
            "<ALPHANUM>",
            "<NUM>",
            "<ALPHANUM>",
            "<ALPHANUM>",
            "<ALPHANUM>",
            "<NUM>",
            "<APOSTROPHE>",
            "<ALPHANUM>",
            HEADING,
            SUB_HEADING,
            SUB_HEADING,
            "<ALPHANUM>",
            "<ALPHANUM>",
            "<ALPHANUM>",
            "<ALPHANUM>",
            CATEGORY,
            CATEGORY,
            CATEGORY,
            "<ALPHANUM>",
            "<ALPHANUM>",
            "<ALPHANUM>",
            "<ALPHANUM>",
            "<ALPHANUM>",
            ITALICS,
            CATEGORY,
            "<ALPHANUM>",
            "<ALPHANUM>",
            "<ALPHANUM>",
            "<ALPHANUM>",
            BOLD,
            CATEGORY,
            "<ALPHANUM>",
            "<ALPHANUM>",
            "<ALPHANUM>",
            BOLD_ITALICS,
            CATEGORY,
            "<ALPHANUM>",
            "<ALPHANUM>",
            EXTERNAL_LINK_URL,
            EXTERNAL_LINK,
            EXTERNAL_LINK,
            EXTERNAL_LINK_URL,
            EXTERNAL_LINK,
            EXTERNAL_LINK,
            EXTERNAL_LINK_URL,
            EXTERNAL_LINK,
            EXTERNAL_LINK,
            CITATION,
            "<ALPHANUM>",
            "<ALPHANUM>",
        ],
    )?;
    Ok(())
}

fn wt_test_link_phrases() -> Result<()> {
    let mut tf = wikipedia(TOKENS_ONLY, &[], LINK_PHRASES)?;
    check_link_phrases(&mut tf)
}

fn wt_test_links() -> Result<()> {
    let test = "[http://lucene.apache.org/java/docs/index.html#news here] [http://lucene.apache.org/java/docs/index.html?b=c here] [https://lucene.apache.org/java/docs/index.html?b=c here]";
    let mut tf = wikipedia(TOKENS_ONLY, &[], test)?;
    B::assert_token_stream_contents_types_only(
        &mut tf,
        &[
            "http://lucene.apache.org/java/docs/index.html#news",
            "here",
            "http://lucene.apache.org/java/docs/index.html?b=c",
            "here",
            "https://lucene.apache.org/java/docs/index.html?b=c",
            "here",
        ],
        &[
            EXTERNAL_LINK_URL,
            EXTERNAL_LINK,
            EXTERNAL_LINK_URL,
            EXTERNAL_LINK,
            EXTERNAL_LINK_URL,
            EXTERNAL_LINK,
        ],
    )?;
    Ok(())
}

fn wt_test_lucene1133() -> Result<()> {
    let untoks = [CATEGORY, ITALICS];
    // should be exactly the same, regardless of untoks
    let mut tf = wikipedia(TOKENS_ONLY, &untoks, LINK_PHRASES)?;
    check_link_phrases(&mut tf)?;
    let test = "[[Category:a b c d]] [[Category:e f g]] [[link here]] [[link there]] ''italics here'' something ''more italics'' [[Category:h   i   j]]";
    let mut tf = new_wikipedia_tokenizer_with_output(
        UNTOKENIZED_ONLY,
        untoks
            .iter()
            .map(|s| s.to_string())
            .collect::<HashSet<String>>(),
    )?;
    tf.set_reader(reader(test))?;
    B::assert_token_stream_contents_offsets_pos_increments(
        &mut tf,
        &[
            "a b c d",
            "e f g",
            "link",
            "here",
            "link",
            "there",
            "italics here",
            "something",
            "more italics",
            "h   i   j",
        ],
        &[11, 32, 42, 47, 56, 61, 71, 86, 98, 124],
        &[18, 37, 46, 51, 60, 66, 83, 95, 110, 133],
        &[1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
    )?;
    Ok(())
}

fn wt_test_both() -> Result<()> {
    let untoks = [CATEGORY, ITALICS];
    let test = "[[Category:a b c d]] [[Category:e f g]] [[link here]] [[link there]] ''italics here'' something ''more italics'' [[Category:h   i   j]]";
    // should output all the indivual tokens plus the untokenized tokens as well.  Untokenized
    // tokens
    let mut tf = wikipedia(BOTH, &untoks, test)?;
    B::assert_token_stream_contents_offsets_pos_increments(
        &mut tf,
        &[
            "a b c d",
            "a",
            "b",
            "c",
            "d",
            "e f g",
            "e",
            "f",
            "g",
            "link",
            "here",
            "link",
            "there",
            "italics here",
            "italics",
            "here",
            "something",
            "more italics",
            "more",
            "italics",
            "h   i   j",
            "h",
            "i",
            "j",
        ],
        &[
            11, 11, 13, 15, 17, 32, 32, 34, 36, 42, 47, 56, 61, 71, 71, 79, 86, 98, 98, 103, 124,
            124, 128, 132,
        ],
        &[
            18, 12, 14, 16, 18, 37, 33, 35, 37, 46, 51, 60, 66, 83, 78, 83, 95, 110, 102, 110, 133,
            125, 129, 133,
        ],
        &[
            1, 0, 1, 1, 1, 1, 0, 1, 1, 1, 1, 1, 1, 1, 0, 1, 1, 1, 0, 1, 1, 0, 1, 1,
        ],
    )?;

    // now check the flags, TODO: add way to check flags from BaseTokenStreamTestCase?
    let mut tf = wikipedia(BOTH, &untoks, test)?;
    let expected_flags = [
        UNTOKENIZED_TOKEN_FLAG,
        0,
        0,
        0,
        0,
        UNTOKENIZED_TOKEN_FLAG,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        UNTOKENIZED_TOKEN_FLAG,
        0,
        0,
        0,
        UNTOKENIZED_TOKEN_FLAG,
        0,
        0,
        UNTOKENIZED_TOKEN_FLAG,
        0,
        0,
        0,
    ];
    tf.attribute_source_mut()
        .add_attribute_interface::<dyn FlagsAttribute>()?;
    tf.reset()?;
    for (i, expected) in expected_flags.iter().enumerate() {
        Assert::assert_true(tf.increment_token()?);
        let flags = tf
            .attribute_source()
            .get_attribute_interface::<dyn FlagsAttribute>()
            .expect("flagsAtt")
            .get_flags();
        Assert::assert_equals_msg(Some(&format!("flags {i}")), *expected, flags);
    }
    Assert::assert_false(tf.increment_token()?);
    tf.close()?;
    Ok(())
}

fn wt_test_random_strings() -> Result<()> {
    let a = wikipedia_analyzer("org.apache.lucene.analysis.wikipedia.TestWikipediaTokenizer$1");
    // TODO: properly support positionLengthAttribute
    B::check_random_data_graph(
        &*LuceneTestCase::random(),
        &a,
        200 * *RANDOM_MULTIPLIER,
        20,
        false,
        false,
    )?;
    a.close();
    Ok(())
}

fn wt_test_random_huge_strings() -> Result<()> {
    let random = LuceneTestCase::random();
    let a = wikipedia_analyzer("org.apache.lucene.analysis.wikipedia.TestWikipediaTokenizer$2");
    // TODO: properly support positionLengthAttribute
    B::check_random_data_graph(&*random, &a, 10 * *RANDOM_MULTIPLIER, 8192, false, false)?;
    a.close();
    Ok(())
}
fn wikipedia_tokenizer_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.wikipedia.TestWikipediaTokenizer")
        .method(t("testSimple", wt_test_simple))
        .method(t("testHandwritten", wt_test_handwritten))
        .method(t("testLinkPhrases", wt_test_link_phrases))
        .method(t("testLinks", wt_test_links))
        .method(t("testLucene1133", wt_test_lucene1133))
        .method(t("testBoth", wt_test_both))
        .method(t("testRandomStrings", wt_test_random_strings))
        .method(t("testRandomHugeStrings", wt_test_random_huge_strings))
        .build()
}

// -----------------------------------------------------------------------------
// TestWikipediaTokenizerFactory
// -----------------------------------------------------------------------------

const WIKIPEDIA: &str = "Wikipedia";
const TOKEN_OUTPUT: &str = "tokenOutput";
const UNTOKENIZED_TYPES: &str = "untokenizedTypes";

fn wf_test_tokenizer() -> Result<()> {
    let text = "This is a [[Category:foo]]";
    let mut tf = factory_tokenizer(WIKIPEDIA, &[], text)?;
    B::assert_token_stream_contents_types_final_offset(
        &mut tf,
        &["This", "is", "a", "foo"],
        Some(&[0, 5, 8, 21]),
        Some(&[4, 7, 9, 24]),
        Some(&["<ALPHANUM>", "<ALPHANUM>", "<ALPHANUM>", CATEGORY]),
        Some(&[1, 1, 1, 1]),
        Some(text.len() as i32),
    )
}

fn wf_test_tokenizer_tokens_only() -> Result<()> {
    let text = "This is a [[Category:foo]]";
    let mut tf = factory_tokenizer(WIKIPEDIA, &[TOKEN_OUTPUT, &TOKENS_ONLY.to_string()], text)?;
    B::assert_token_stream_contents_types_final_offset(
        &mut tf,
        &["This", "is", "a", "foo"],
        Some(&[0, 5, 8, 21]),
        Some(&[4, 7, 9, 24]),
        Some(&["<ALPHANUM>", "<ALPHANUM>", "<ALPHANUM>", CATEGORY]),
        Some(&[1, 1, 1, 1]),
        Some(text.len() as i32),
    )
}

fn wf_test_tokenizer_untokenized_only() -> Result<()> {
    let test = "[[Category:a b c d]] [[Category:e f g]] [[link here]] [[link there]] ''italics here'' something ''more italics'' [[Category:h   i   j]]";
    let untoks = format!("{CATEGORY}, {ITALICS}");
    let mut tf = factory_tokenizer(
        WIKIPEDIA,
        &[
            TOKEN_OUTPUT,
            &UNTOKENIZED_ONLY.to_string(),
            UNTOKENIZED_TYPES,
            &untoks,
        ],
        test,
    )?;
    B::assert_token_stream_contents_offsets_pos_increments(
        &mut tf,
        &[
            "a b c d",
            "e f g",
            "link",
            "here",
            "link",
            "there",
            "italics here",
            "something",
            "more italics",
            "h   i   j",
        ],
        &[11, 32, 42, 47, 56, 61, 71, 86, 98, 124],
        &[18, 37, 46, 51, 60, 66, 83, 95, 110, 133],
        &[1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
    )?;
    Ok(())
}

fn wf_test_tokenizer_both() -> Result<()> {
    let test = "[[Category:a b c d]] [[Category:e f g]] [[link here]] [[link there]] ''italics here'' something ''more italics'' [[Category:h   i   j]]";
    let untoks = format!("{CATEGORY}, {ITALICS}");
    let mut tf = factory_tokenizer(
        WIKIPEDIA,
        &[TOKEN_OUTPUT, &BOTH.to_string(), UNTOKENIZED_TYPES, &untoks],
        test,
    )?;
    B::assert_token_stream_contents_offsets_pos_increments(
        &mut tf,
        &[
            "a b c d",
            "a",
            "b",
            "c",
            "d",
            "e f g",
            "e",
            "f",
            "g",
            "link",
            "here",
            "link",
            "there",
            "italics here",
            "italics",
            "here",
            "something",
            "more italics",
            "more",
            "italics",
            "h   i   j",
            "h",
            "i",
            "j",
        ],
        &[
            11, 11, 13, 15, 17, 32, 32, 34, 36, 42, 47, 56, 61, 71, 71, 79, 86, 98, 98, 103, 124,
            124, 128, 132,
        ],
        &[
            18, 12, 14, 16, 18, 37, 33, 35, 37, 46, 51, 60, 66, 83, 78, 83, 95, 110, 102, 110, 133,
            125, 129, 133,
        ],
        &[
            1, 0, 1, 1, 1, 1, 0, 1, 1, 1, 1, 1, 1, 1, 0, 1, 1, 1, 0, 1, 1, 0, 1, 1,
        ],
    )?;
    Ok(())
}

fn wf_test_bogus_arguments() -> Result<()> {
    let expected = iae_message(|| {
        F::tokenizer_factory(WIKIPEDIA, &["bogusArg", "bogusValue"])?
            .create_with_factory(B::new_attribute_factory())
    });
    Assert::assert_true(expected.contains("Unknown parameters"));
    Ok(())
}

fn wf_test_illegal_arguments() -> Result<()> {
    let expected = iae_message(|| {
        F::tokenizer_factory(WIKIPEDIA, &[TOKEN_OUTPUT, "-1"])?
            .create_with_factory(B::new_attribute_factory())
    });
    Assert::assert_true(
        expected.contains("tokenOutput must be TOKENS_ONLY, UNTOKENIZED_ONLY or BOTH"),
    );
    Ok(())
}
fn wikipedia_tokenizer_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.wikipedia.TestWikipediaTokenizerFactory")
        .method(t("testTokenizer", wf_test_tokenizer))
        .method(t("testTokenizerTokensOnly", wf_test_tokenizer_tokens_only))
        .method(t(
            "testTokenizerUntokenizedOnly",
            wf_test_tokenizer_untokenized_only,
        ))
        .method(t("testTokenizerBoth", wf_test_tokenizer_both))
        .method(t("testBogusArguments", wf_test_bogus_arguments))
        .method(t("testIllegalArguments", wf_test_illegal_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// TestCustomAnalyzer
// -----------------------------------------------------------------------------

#[allow(deprecated)]
const LUCENE_9_0_0: Version = Version::LUCENE_9_0_0;

/// `CustomAnalyzer.builder()` on the test class path (see the module docs).
fn builder() -> Builder {
    CustomAnalyzer::builder_with_loader(Box::new(ClasspathResourceLoader::new(
        support::test_class_loader_resources(),
    )))
}

/// `getDataPath(name)` of `TestCustomAnalyzer`: its resources are copied
/// byte for byte under `tests/resources`.
fn custom_data_path(name: &str) -> Result<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/resources/org/apache/lucene/analysis/custom");
    LuceneTestCase::get_data_path(&dir, name)
}

/// `assertSame(expected.class, factory.getClass())`.
fn assert_class(expected: &str, actual: &str) {
    Assert::assert_equals(expected, actual);
}

fn names_of<T: ?Sized>(
    factories: &[Arc<T>],
    name: impl Fn(&T) -> &'static str,
) -> Vec<&'static str> {
    factories.iter().map(|f| name(f)).collect()
}

/// The `getClass()` checks of the whitespace/folding/lowercase analyzers.
fn assert_whitespace_folding_lowercase(a: &CustomAnalyzer) {
    assert_class(
        "org.apache.lucene.analysis.core.WhitespaceTokenizerFactory",
        a.get_tokenizer_factory().java_class_name(),
    );
    Assert::assert_true(a.get_char_filter_factories().is_empty());
    let token_filters = names_of(a.get_token_filter_factories(), |f| f.java_class_name());
    Assert::assert_equals(2, token_filters.len());
    assert_class(
        "org.apache.lucene.analysis.miscellaneous.ASCIIFoldingFilterFactory",
        token_filters[0],
    );
    assert_class(
        "org.apache.lucene.analysis.core.LowerCaseFilterFactory",
        token_filters[1],
    );
    Assert::assert_equals(0, a.get_position_increment_gap("dummy"));
    Assert::assert_equals(1, a.get_offset_gap("dummy"));
}

fn cu_test_whitespace_factory_with_folding() -> Result<()> {
    let a = builder()
        .with_tokenizer_class(&WhitespaceTokenizerFactory::class(), &[])?
        .add_token_filter_class(
            &ASCIIFoldingFilterFactory::class(),
            &["preserveOriginal", "true"],
        )?
        .add_token_filter_class(&LowerCaseFilterFactory::class(), &[])?
        .build()?;
    assert_whitespace_folding_lowercase(&a);
    B::assert_analyzes_to_pos_increments(
        &a,
        "foo bar FOO BAR",
        &["foo", "bar", "foo", "bar"],
        &[1, 1, 1, 1],
    )?;
    B::assert_analyzes_to_pos_increments(
        &a,
        "f\u{F6}\u{F3} b\u{E4}r F\u{D6}\u{D6} BAR",
        &[
            "foo",
            "f\u{F6}\u{F3}",
            "bar",
            "b\u{E4}r",
            "foo",
            "f\u{F6}\u{F6}",
            "bar",
        ],
        &[1, 0, 1, 0, 1, 0, 1],
    )?;
    a.close();
    Ok(())
}

fn cu_test_whitespace_with_folding() -> Result<()> {
    let a = builder()
        .with_tokenizer("whitespace", &[])?
        .add_token_filter("asciifolding", &["preserveOriginal", "true"])?
        .add_token_filter("lowercase", &[])?
        .build()?;
    assert_whitespace_folding_lowercase(&a);
    B::assert_analyzes_to_pos_increments(
        &a,
        "foo bar FOO BAR",
        &["foo", "bar", "foo", "bar"],
        &[1, 1, 1, 1],
    )?;
    B::assert_analyzes_to_pos_increments(
        &a,
        "f\u{F6}\u{F3} b\u{E4}r F\u{D6}\u{D6} BAR",
        &[
            "foo",
            "f\u{F6}\u{F3}",
            "bar",
            "b\u{E4}r",
            "foo",
            "f\u{F6}\u{F6}",
            "bar",
        ],
        &[1, 0, 1, 0, 1, 0, 1],
    )?;
    a.close();
    Ok(())
}

fn cu_test_version_aware_filter() -> Result<()> {
    let a = builder()
        .with_default_match_version(LUCENE_9_0_0)?
        .with_tokenizer_class(&StandardTokenizerFactory::class(), &[])?
        .add_token_filter_class(&DUMMY_VERSION_AWARE_TOKEN_FILTER_FACTORY, &[])?
        .build()?;
    B::assert_analyzes_to(&a, "HELLO WORLD", &["HELLO", "WORLD"])?;

    let b = builder()
        .with_tokenizer_class(&StandardTokenizerFactory::class(), &[])?
        .add_token_filter_class(&DUMMY_VERSION_AWARE_TOKEN_FILTER_FACTORY, &[])?
        .build()?;
    B::assert_analyzes_to(&b, "HELLO WORLD", &["hello", "world"])?;
    Ok(())
}

/// The `getClass()` checks of the html-strip/classic/folding analyzers.
fn assert_html_strip_classic_folding(a: &CustomAnalyzer) {
    assert_class(
        "org.apache.lucene.analysis.classic.ClassicTokenizerFactory",
        a.get_tokenizer_factory().java_class_name(),
    );
    let char_filters = names_of(a.get_char_filter_factories(), |f| f.java_class_name());
    Assert::assert_equals(1, char_filters.len());
    assert_class(
        "org.apache.lucene.analysis.charfilter.HTMLStripCharFilterFactory",
        char_filters[0],
    );
    let token_filters = names_of(a.get_token_filter_factories(), |f| f.java_class_name());
    Assert::assert_equals(2, token_filters.len());
    assert_class(
        "org.apache.lucene.analysis.miscellaneous.ASCIIFoldingFilterFactory",
        token_filters[0],
    );
    assert_class(
        "org.apache.lucene.analysis.core.LowerCaseFilterFactory",
        token_filters[1],
    );
    Assert::assert_equals(100, a.get_position_increment_gap("dummy"));
    Assert::assert_equals(1000, a.get_offset_gap("dummy"));
}

fn assert_html_strip_classic_folding_analysis(a: &CustomAnalyzer) -> Result<()> {
    B::assert_analyzes_to_pos_increments(
        a,
        "<p>foo bar</p> FOO BAR",
        &["foo", "bar", "foo", "bar"],
        &[1, 1, 1, 1],
    )?;
    B::assert_analyzes_to_pos_increments(
        a,
        "<p><b>f\u{F6}\u{F3}</b> b\u{E4}r     F\u{D6}\u{D6} BAR</p>",
        &[
            "foo",
            "f\u{F6}\u{F3}",
            "bar",
            "b\u{E4}r",
            "foo",
            "f\u{F6}\u{F6}",
            "bar",
        ],
        &[1, 0, 1, 0, 1, 0, 1],
    )
}

fn cu_test_factory_html_strip_classic_folding() -> Result<()> {
    let a = builder()
        .with_default_match_version(LUCENE_9_0_0)?
        .add_char_filter_class(&HTMLStripCharFilterFactory::class(), &[])?
        .with_tokenizer_class(&ClassicTokenizerFactory::class(), &[])?
        .add_token_filter_class(
            &ASCIIFoldingFilterFactory::class(),
            &["preserveOriginal", "true"],
        )?
        .add_token_filter_class(&LowerCaseFilterFactory::class(), &[])?
        .with_position_increment_gap(100)?
        .with_offset_gap(1000)?
        .build()?;
    assert_html_strip_classic_folding(&a);
    assert_html_strip_classic_folding_analysis(&a)?;
    a.close();
    Ok(())
}

fn cu_test_html_strip_classic_folding() -> Result<()> {
    let a = builder()
        .with_default_match_version(LUCENE_9_0_0)?
        .add_char_filter("htmlstrip", &[])?
        .with_tokenizer("classic", &[])?
        .add_token_filter("asciifolding", &["preserveOriginal", "true"])?
        .add_token_filter("lowercase", &[])?
        .with_position_increment_gap(100)?
        .with_offset_gap(1000)?
        .build()?;
    assert_html_strip_classic_folding(&a);
    assert_html_strip_classic_folding_analysis(&a)?;
    a.close();
    Ok(())
}

const STOP_PARAMS: [&str; 6] = [
    "ignoreCase",
    "true",
    "words",
    "org/apache/lucene/analysis/custom/teststop.txt",
    "format",
    "wordset",
];

fn cu_test_stop_words_from_classpath() -> Result<()> {
    let a = builder()
        .with_tokenizer_class(&WhitespaceTokenizerFactory::class(), &[])?
        .add_token_filter("stop", &STOP_PARAMS)?
        .build()?;
    assert_class(
        "org.apache.lucene.analysis.core.WhitespaceTokenizerFactory",
        a.get_tokenizer_factory().java_class_name(),
    );
    Assert::assert_true(a.get_char_filter_factories().is_empty());
    let token_filters = names_of(a.get_token_filter_factories(), |f| f.java_class_name());
    Assert::assert_equals(1, token_filters.len());
    assert_class(
        "org.apache.lucene.analysis.core.StopFilterFactory",
        token_filters[0],
    );
    Assert::assert_equals(0, a.get_position_increment_gap("dummy"));
    Assert::assert_equals(1, a.get_offset_gap("dummy"));
    B::assert_analyzes_to(&a, "foo Foo Bar", &[])?;
    a.close();
    Ok(())
}

fn cu_test_stop_words_from_classpath_with_map() -> Result<()> {
    let mut stop_config1: HashMap<String, String> = HashMap::new();
    stop_config1.insert("ignoreCase".into(), "true".into());
    stop_config1.insert(
        "words".into(),
        "org/apache/lucene/analysis/custom/teststop.txt".into(),
    );
    stop_config1.insert("format".into(), "wordset".into());
    let mut stop_config2 = stop_config1.clone();

    let a = builder()
        .with_tokenizer("whitespace", &[])?
        .add_token_filter_map("stop", &mut stop_config1)?
        .build()?;
    Assert::assert_true(stop_config1.is_empty());
    B::assert_analyzes_to(&a, "foo Foo Bar", &[])?;

    let a = builder()
        .with_tokenizer_class(&WhitespaceTokenizerFactory::class(), &[])?
        .add_token_filter_class_map(&StopFilterFactory::class(), &mut stop_config2)?
        .build()?;
    Assert::assert_true(stop_config2.is_empty());
    B::assert_analyzes_to(&a, "foo Foo Bar", &[])?;

    // try with unmodifiableMap, should fail: not expressible, an immutable map
    // cannot be passed as `&mut HashMap` (see the module docs)
    a.close();
    Ok(())
}

fn cu_test_stop_words_from_file() -> Result<()> {
    let words = custom_data_path("teststop.txt")?;
    let a = CustomAnalyzer::builder_with_config_dir(&custom_data_path("")?)?
        .with_tokenizer("whitespace", &[])?
        .add_token_filter(
            "stop",
            &[
                "ignoreCase",
                "true",
                "words",
                &words.to_string_lossy(),
                "format",
                "wordset",
            ],
        )?
        .build()?;
    B::assert_analyzes_to(&a, "foo Foo Bar", &[])?;
    a.close();
    Ok(())
}

fn cu_test_stop_words_from_file_absolute() -> Result<()> {
    let words = custom_data_path("teststop.txt")?;
    let a = CustomAnalyzer::builder_with_config_dir(Path::new("."))?
        .with_tokenizer("whitespace", &[])?
        .add_token_filter(
            "stop",
            &[
                "ignoreCase",
                "true",
                "words",
                &words.to_string_lossy(),
                "format",
                "wordset",
            ],
        )?
        .build()?;
    B::assert_analyzes_to(&a, "foo Foo Bar", &[])?;
    a.close();
    Ok(())
}

// Now test misconfigurations:

fn cu_test_incorrect_order() -> Result<()> {
    expect_throws("java.lang.IllegalStateException", || {
        builder()
            .add_char_filter("htmlstrip", &[])?
            .with_default_match_version(Version::LATEST)?
            .with_tokenizer("whitespace", &[])?
            .build()
    });
    Ok(())
}

fn cu_test_missing_spi() -> Result<()> {
    let expected = iae_message(|| builder().with_tokenizer("foobar_nonexistent", &[])?.build());
    Assert::assert_true(expected.contains("SPI"));
    Assert::assert_true(expected.contains("does not exist"));
    Ok(())
}

const ALREADY_SET: &str = "org.apache.lucene.util.SetOnce$AlreadySetException";

fn cu_test_set_tokenizer_twice() -> Result<()> {
    expect_throws(ALREADY_SET, || {
        builder()
            .with_tokenizer("whitespace", &[])?
            .with_tokenizer_class(&StandardTokenizerFactory::class(), &[])?
            .build()
    });
    Ok(())
}

fn cu_test_set_match_version_twice() -> Result<()> {
    expect_throws(ALREADY_SET, || {
        builder()
            .with_default_match_version(Version::LATEST)?
            .with_default_match_version(Version::LATEST)?
            .with_tokenizer("standard", &[])?
            .build()
    });
    Ok(())
}

fn cu_test_set_pos_inc_twice() -> Result<()> {
    expect_throws(ALREADY_SET, || {
        builder()
            .with_position_increment_gap(2)?
            .with_position_increment_gap(3)?
            .with_tokenizer("standard", &[])?
            .build()
    });
    Ok(())
}

fn cu_test_set_ofs_gap_twice() -> Result<()> {
    expect_throws(ALREADY_SET, || {
        builder()
            .with_offset_gap(2)?
            .with_offset_gap(3)?
            .with_tokenizer("standard", &[])?
            .build()
    });
    Ok(())
}

fn cu_test_no_tokenizer() -> Result<()> {
    expect_throws("java.lang.IllegalStateException", || builder().build());
    Ok(())
}

/// `DummyCharFilter`: replaces `match` with `repl`, no offset correction.
#[derive(Debug)]
struct DummyCharFilter {
    input: Box<dyn CharReader>,
    match_: u16,
    repl: u16,
}

impl CharReader for DummyCharFilter {
    fn read(&mut self, buf: &mut [u16]) -> Result<usize> {
        let read = self.input.read(buf)?;
        for c in &mut buf[..read] {
            if *c == self.match_ {
                *c = self.repl;
            }
        }
        Ok(read)
    }

    fn close(&mut self) -> Result<()> {
        self.input.close()
    }

    fn as_char_filter(&self) -> Option<&dyn CharFilter> {
        Some(self)
    }
}

impl CharFilter for DummyCharFilter {
    fn correct(&self, current_off: i32) -> i32 {
        current_off
    }

    fn input(&self) -> &dyn CharReader {
        &*self.input
    }

    fn input_mut(&mut self) -> &mut dyn CharReader {
        &mut *self.input
    }
}

/// `DummyCharFilterFactory` (`multi_term_aware` false) and
/// `DummyMultiTermAwareCharFilterFactory` (true, `normalize` = `create`).
#[derive(Debug)]
struct DummyCharFilterFactory {
    multi_term_aware: bool,
}

impl CharFilterFactory for DummyCharFilterFactory {
    fn java_class_name(&self) -> &'static str {
        if self.multi_term_aware {
            "org.apache.lucene.analysis.custom.TestCustomAnalyzer$DummyMultiTermAwareCharFilterFactory"
        } else {
            "org.apache.lucene.analysis.custom.TestCustomAnalyzer$DummyCharFilterFactory"
        }
    }

    fn create(&self, input: Box<dyn CharReader>) -> Result<Box<dyn CharReader>> {
        Ok(Box::new(DummyCharFilter {
            input,
            match_: '0' as u16,
            repl: '1' as u16,
        }))
    }

    fn normalize(&self, input: Box<dyn CharReader>) -> Result<Box<dyn CharReader>> {
        if self.multi_term_aware {
            self.create(input)
        } else {
            Ok(input)
        }
    }
}

fn new_dummy_char_filter_factory(
    args: &mut HashMap<String, String>,
) -> Result<Arc<dyn CharFilterFactory>> {
    AbstractAnalysisFactory::new(args)?;
    Ok(Arc::new(DummyCharFilterFactory {
        multi_term_aware: false,
    }))
}

fn new_dummy_multi_term_aware_char_filter_factory(
    args: &mut HashMap<String, String>,
) -> Result<Arc<dyn CharFilterFactory>> {
    AbstractAnalysisFactory::new(args)?;
    Ok(Arc::new(DummyCharFilterFactory {
        multi_term_aware: true,
    }))
}

const DUMMY_CHAR_FILTER_FACTORY: AnalysisFactoryClass<dyn CharFilterFactory> =
    AnalysisFactoryClass::new(
        "org.apache.lucene.analysis.custom.TestCustomAnalyzer$DummyCharFilterFactory",
        SPINameField::Missing,
        Some(new_dummy_char_filter_factory),
    );

const DUMMY_MULTI_TERM_AWARE_CHAR_FILTER_FACTORY: AnalysisFactoryClass<dyn CharFilterFactory> =
    AnalysisFactoryClass::new(
        "org.apache.lucene.analysis.custom.TestCustomAnalyzer$DummyMultiTermAwareCharFilterFactory",
        SPINameField::Missing,
        Some(new_dummy_multi_term_aware_char_filter_factory),
    );

/// `DummyTokenizerFactory`: a `LetterTokenizer`.
#[derive(Debug)]
struct DummyTokenizerFactory;

impl TokenizerFactory for DummyTokenizerFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.custom.TestCustomAnalyzer$DummyTokenizerFactory"
    }

    fn create_with_factory(&self, factory: Arc<dyn AttributeFactory>) -> Result<FactoryTokenizer> {
        // `new LetterTokenizer(factory)`, through its `Tokenizer` supertype
        // (built as `new_letter_tokenizer_with_factory` builds it)
        let source = char_tokenizer_source(factory)?;
        Ok(rucene::analysis::Tokenizer::with_source(
            source,
            Box::new(CharTokenizerAdapter::new(LetterTokenizerLogic)),
        ))
    }
}

fn new_dummy_tokenizer_factory(
    args: &mut HashMap<String, String>,
) -> Result<Arc<dyn TokenizerFactory>> {
    AbstractAnalysisFactory::new(args)?;
    Ok(Arc::new(DummyTokenizerFactory))
}

const DUMMY_TOKENIZER_FACTORY: AnalysisFactoryClass<dyn TokenizerFactory> =
    AnalysisFactoryClass::new(
        "org.apache.lucene.analysis.custom.TestCustomAnalyzer$DummyTokenizerFactory",
        SPINameField::Missing,
        Some(new_dummy_tokenizer_factory),
    );

/// `DummyTokenFilterFactory` (`multi_term_aware` false: `create` returns the
/// input) and `DummyMultiTermAwareTokenFilterFactory` (its `normalize` is
/// `new ASCIIFoldingFilterFactory(emptyMap()).normalize(input)`).
#[derive(Debug)]
struct DummyTokenFilterFactory {
    multi_term_aware: bool,
}

impl TokenFilterFactory for DummyTokenFilterFactory {
    fn java_class_name(&self) -> &'static str {
        if self.multi_term_aware {
            "org.apache.lucene.analysis.custom.TestCustomAnalyzer$DummyMultiTermAwareTokenFilterFactory"
        } else {
            "org.apache.lucene.analysis.custom.TestCustomAnalyzer$DummyTokenFilterFactory"
        }
    }

    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        Ok(input)
    }

    fn normalize(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        if self.multi_term_aware {
            ASCIIFoldingFilterFactory::new(&mut HashMap::new())?.normalize(input)
        } else {
            Ok(input)
        }
    }
}

fn new_dummy_token_filter_factory(
    args: &mut HashMap<String, String>,
) -> Result<Arc<dyn TokenFilterFactory>> {
    AbstractAnalysisFactory::new(args)?;
    Ok(Arc::new(DummyTokenFilterFactory {
        multi_term_aware: false,
    }))
}

fn new_dummy_multi_term_aware_token_filter_factory(
    args: &mut HashMap<String, String>,
) -> Result<Arc<dyn TokenFilterFactory>> {
    AbstractAnalysisFactory::new(args)?;
    Ok(Arc::new(DummyTokenFilterFactory {
        multi_term_aware: true,
    }))
}

const DUMMY_TOKEN_FILTER_FACTORY: AnalysisFactoryClass<dyn TokenFilterFactory> =
    AnalysisFactoryClass::new(
        "org.apache.lucene.analysis.custom.TestCustomAnalyzer$DummyTokenFilterFactory",
        SPINameField::Missing,
        Some(new_dummy_token_filter_factory),
    );

const DUMMY_MULTI_TERM_AWARE_TOKEN_FILTER_FACTORY: AnalysisFactoryClass<dyn TokenFilterFactory> =
    AnalysisFactoryClass::new(
        "org.apache.lucene.analysis.custom.TestCustomAnalyzer$DummyMultiTermAwareTokenFilterFactory",
        SPINameField::Missing,
        Some(new_dummy_multi_term_aware_token_filter_factory),
    );

/// `DummyVersionAwareTokenFilterFactory`: the input itself under
/// `LUCENE_9_0_0`, a `LowerCaseFilter` otherwise.
#[derive(Debug)]
struct DummyVersionAwareTokenFilterFactory {
    lucene_match_version: Version,
}

impl TokenFilterFactory for DummyVersionAwareTokenFilterFactory {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.analysis.custom.TestCustomAnalyzer$DummyVersionAwareTokenFilterFactory"
    }

    fn create(&self, input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        if self.lucene_match_version == LUCENE_9_0_0 {
            return Ok(input);
        }
        Ok(Box::new(LowerCaseFilter::new(input)?))
    }
}

fn new_dummy_version_aware_token_filter_factory(
    args: &mut HashMap<String, String>,
) -> Result<Arc<dyn TokenFilterFactory>> {
    let base = AbstractAnalysisFactory::new(args)?;
    Ok(Arc::new(DummyVersionAwareTokenFilterFactory {
        lucene_match_version: base.get_lucene_match_version(),
    }))
}

const DUMMY_VERSION_AWARE_TOKEN_FILTER_FACTORY: AnalysisFactoryClass<dyn TokenFilterFactory> =
    AnalysisFactoryClass::new(
        "org.apache.lucene.analysis.custom.TestCustomAnalyzer$DummyVersionAwareTokenFilterFactory",
        SPINameField::Missing,
        Some(new_dummy_version_aware_token_filter_factory),
    );

fn cu_test_normalization() -> Result<()> {
    let analyzer1 = builder()
        // none of these components are multi-term aware so they should not be applied
        .with_tokenizer_class_map(&DUMMY_TOKENIZER_FACTORY, &mut HashMap::new())?
        .add_char_filter_class_map(&DUMMY_CHAR_FILTER_FACTORY, &mut HashMap::new())?
        .add_token_filter_class_map(&DUMMY_TOKEN_FILTER_FACTORY, &mut HashMap::new())?
        .build()?;
    Assert::assert_true(
        BytesRef::from_char_sequence("0\u{C0}") == analyzer1.normalize("dummy", "0\u{C0}")?,
    );

    let analyzer2 = builder()
        // this component in not multi-term aware so it should not be applied
        .with_tokenizer_class_map(&DUMMY_TOKENIZER_FACTORY, &mut HashMap::new())?
        // these components are multi-term aware so they should be applied
        .add_char_filter_class_map(
            &DUMMY_MULTI_TERM_AWARE_CHAR_FILTER_FACTORY,
            &mut HashMap::new(),
        )?
        .add_token_filter_class_map(
            &DUMMY_MULTI_TERM_AWARE_TOKEN_FILTER_FACTORY,
            &mut HashMap::new(),
        )?
        .build()?;
    Assert::assert_true(
        BytesRef::from_char_sequence("1A") == analyzer2.normalize("dummy", "0\u{C0}")?,
    );
    Ok(())
}

fn cu_test_normalization_with_multiple_token_filters() -> Result<()> {
    let analyzer = builder()
        // none of these components are multi-term aware so they should not be applied
        .with_tokenizer_class_map(&WhitespaceTokenizerFactory::class(), &mut HashMap::new())?
        .add_token_filter_class_map(&LowerCaseFilterFactory::class(), &mut HashMap::new())?
        .add_token_filter_class_map(&ASCIIFoldingFilterFactory::class(), &mut HashMap::new())?
        .build()?;
    Assert::assert_true(
        BytesRef::from_char_sequence("a b e") == analyzer.normalize("dummy", "\u{C0} B \u{E9}")?,
    );
    Ok(())
}

fn cu_test_normalization_with_multipl_char_filters() -> Result<()> {
    let mut m1: HashMap<String, String> = HashMap::new();
    m1.insert(
        "mapping".into(),
        "org/apache/lucene/analysis/custom/mapping1.txt".into(),
    );
    let mut m2: HashMap<String, String> = HashMap::new();
    m2.insert(
        "mapping".into(),
        "org/apache/lucene/analysis/custom/mapping2.txt".into(),
    );
    let analyzer = builder()
        // none of these components are multi-term aware so they should not be applied
        .with_tokenizer_class_map(&WhitespaceTokenizerFactory::class(), &mut HashMap::new())?
        .add_char_filter_class_map(&MappingCharFilterFactory::class(), &mut m1)?
        .add_char_filter_class_map(&MappingCharFilterFactory::class(), &mut m2)?
        .build()?;
    Assert::assert_true(
        BytesRef::from_char_sequence("e f c") == analyzer.normalize("dummy", "a b c")?,
    );
    Ok(())
}

fn cu_test_conditions() -> Result<()> {
    let mut b = builder();
    b.with_tokenizer("whitespace", &[])?
        .add_token_filter("lowercase", &[])?;
    let mut c = b.when_term(Arc::new(|t: &str| t.contains('o')));
    c.add_token_filter("uppercase", &[])?
        .add_token_filter_class(&ReverseStringFilterFactory::class(), &[])?;
    c.endwhen()?.add_token_filter("asciifolding", &[])?;
    let analyzer = b.build()?;
    B::assert_analyzes_to(
        &analyzer,
        "H\u{E9}llo world whaT's h\u{E3}ppening",
        &["OLLEH", "DLROW", "what's", "happening"],
    )
}

fn cu_test_conditions_with_resource_loader() -> Result<()> {
    let mut b = builder();
    b.with_tokenizer("whitespace", &[])?
        .add_token_filter("lowercase", &[])?;
    let mut c = b.when(
        "protectedterm",
        &[
            "protected",
            "org/apache/lucene/analysis/custom/teststop.txt",
        ],
    )?;
    c.add_token_filter("reversestring", &[])?;
    c.endwhen()?;
    let analyzer = b.build()?;
    B::assert_analyzes_to(&analyzer, "FOO BAR BAZ", &["foo", "bar", "zab"])
}

fn cu_test_conditions_with_wrapped_resource_loader() -> Result<()> {
    let mut b = builder();
    b.with_tokenizer("whitespace", &[])?
        .add_token_filter("lowercase", &[])?;
    let mut c = b.when_term(Arc::new(|t: &str| !t.contains('o')));
    c.add_token_filter("stop", &STOP_PARAMS)?;
    c.endwhen()?;
    let analyzer = b.build()?;
    B::assert_analyzes_to(&analyzer, "foo bar baz", &["foo", "baz"])
}

fn custom_analyzer_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.custom.TestCustomAnalyzer")
        .method(t(
            "testWhitespaceFactoryWithFolding",
            cu_test_whitespace_factory_with_folding,
        ))
        .method(t(
            "testWhitespaceWithFolding",
            cu_test_whitespace_with_folding,
        ))
        .method(t("testVersionAwareFilter", cu_test_version_aware_filter))
        .method(t(
            "testFactoryHtmlStripClassicFolding",
            cu_test_factory_html_strip_classic_folding,
        ))
        .method(t(
            "testHtmlStripClassicFolding",
            cu_test_html_strip_classic_folding,
        ))
        .method(t(
            "testStopWordsFromClasspath",
            cu_test_stop_words_from_classpath,
        ))
        .method(t(
            "testStopWordsFromClasspathWithMap",
            cu_test_stop_words_from_classpath_with_map,
        ))
        .method(t("testStopWordsFromFile", cu_test_stop_words_from_file))
        .method(t(
            "testStopWordsFromFileAbsolute",
            cu_test_stop_words_from_file_absolute,
        ))
        .method(t("testIncorrectOrder", cu_test_incorrect_order))
        .method(t("testMissingSPI", cu_test_missing_spi))
        .method(t("testSetTokenizerTwice", cu_test_set_tokenizer_twice))
        .method(t(
            "testSetMatchVersionTwice",
            cu_test_set_match_version_twice,
        ))
        .method(t("testSetPosIncTwice", cu_test_set_pos_inc_twice))
        .method(t("testSetOfsGapTwice", cu_test_set_ofs_gap_twice))
        .method(t("testNoTokenizer", cu_test_no_tokenizer))
        .method(t("testNormalization", cu_test_normalization))
        .method(t(
            "testNormalizationWithMultipleTokenFilters",
            cu_test_normalization_with_multiple_token_filters,
        ))
        .method(t(
            "testNormalizationWithMultiplCharFilters",
            cu_test_normalization_with_multipl_char_filters,
        ))
        .method(t("testConditions", cu_test_conditions))
        .method(t(
            "testConditionsWithResourceLoader",
            cu_test_conditions_with_resource_loader,
        ))
        .method(t(
            "testConditionsWithWrappedResourceLoader",
            cu_test_conditions_with_wrapped_resource_loader,
        ))
        .build()
}

#[test]
fn test_classic_analyzer() {
    run(classic_analyzer_class);
}

#[test]
fn test_classic_factories() {
    run(classic_factories_class);
}

#[test]
fn test_custom_analyzer() {
    run(custom_analyzer_class);
}

#[test]
fn test_wikipedia_tokenizer() {
    run(wikipedia_tokenizer_class);
}

#[test]
fn test_wikipedia_tokenizer_factory() {
    run(wikipedia_tokenizer_factory_class);
}

/// Regression test (T4): `ClassicFilter` recognises `ClassicTokenizer`'s
/// `<APOSTROPHE>` and `<ACRONYM>` types over every attribute factory
/// `newAttributeFactory()` draws. Over the test framework's `Token`
/// factory the type used to lose its identity (`Token` did not forward
/// `set_type_static`, so the constant was copied), and `"What's"` kept its
/// `'s`; Java's `Token` inherits `PackedTokenAttributeImpl.setType`, which
/// keeps the reference.
#[test]
fn regression_classic_filter_over_every_attribute_factory() {
    use rucene::analysis::default_attribute_factory;
    use rucene_analysis_common::analysis::classic::classic_filter::new_classic_filter;
    use rucene_analysis_common::analysis::classic::classic_tokenizer::new_classic_tokenizer_with_factory;
    use rucene_test_framework::analysis::token::Token;
    for (i, factory) in [
        rucene::analysis::default_token_attribute_factory(),
        Token::token_attribute_factory(),
        default_attribute_factory(),
    ]
    .into_iter()
    .enumerate()
    {
        let mut tokenizer = new_classic_tokenizer_with_factory(factory).expect("a tokenizer");
        tokenizer
            .set_reader(reader("What's this thing do? U.S.A."))
            .expect("a reader");
        let mut stream = new_classic_filter(Box::new(tokenizer)).expect("a filter");
        let mut terms = Vec::new();
        stream.reset().expect("reset");
        while stream.increment_token().expect("a token") {
            terms.push(support::term(&stream));
        }
        stream.end().expect("end");
        stream.close().expect("close");
        assert_eq!(
            terms,
            ["What", "this", "thing", "do", "USA"],
            "attribute factory #{i} (0 default token, 1 Token, 2 default)"
        );
    }
}
