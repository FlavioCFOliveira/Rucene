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

//! Port of the `org.apache.lucene.analysis.util` test classes of Lucene
//! 10.5.0 `lucene/analysis/common/src/test` that rmp task #448 lists, each
//! run as a suite by the ported runner: `TestCSVUtil`,
//! `TestCharArrayIterator`, `TestElisionFilterFactory`,
//! `TestFilesystemResourceLoader`, `TestRollingCharBuffer` and
//! `TestSegmentingTokenizerBase`. (`TestCharTokenizers`, of the same
//! package, belongs to task #449 and runs in `analysis_core_tests`.)

mod support;

use std::cell::RefCell;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::tokenattributes::{
    CharTermAttribute, OffsetAttribute, PositionIncrementAttribute,
};
use rucene::analysis::wordlist_loader::{get_lines, get_snowball_word_set_reader};
use rucene::analysis::{Analyzer, CharReader, StringCharReader, TokenStream};
use rucene::error::Result;
use rucene::util::extra::Version;
use rucene::util::resource_loader::ResourceLoader;
use rucene_analysis_common::analysis::util::char_array_iterator::CharArrayIterator;
use rucene_analysis_common::analysis::util::csv_util::CSVUtil;
use rucene_analysis_common::analysis::util::filesystem_resource_loader::FilesystemResourceLoader;
use rucene_analysis_common::analysis::util::rolling_char_buffer::RollingCharBuffer;
use rucene_analysis_common::analysis::util::segmenting_tokenizer_base::{
    new_segmenting_tokenizer_base, segmenting_tokenizer_base_source, SegmentingTokenizerBaseLogic,
};
use rucene_analysis_common::jdk::break_iterator::{
    get_sentence_instance, get_word_instance, BreakIterator, DONE as BI_DONE,
};
use rucene_analysis_common::jdk::character::{is_digit, is_letter};
use rucene_analysis_common::jdk::character_iterator::{CharacterIterator, DONE};
use rucene_analysis_common::jdk::locale::Locale;
use rucene_analysis_common::resources::class_loader_resources;
use rucene_test_framework::analysis::mock_tokenizer::{MockTokenizer, WHITESPACE};
use rucene_test_framework::analysis::{
    tokenizer_components, BaseTokenStreamFactoryTestCase, BaseTokenStreamTestCase,
};
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::{RANDOM_MULTIPLIER, VERBOSE};
use rucene_test_framework::util::string_mock_resource_loader::StringMockResourceLoader;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};
use support::{
    btstc_class, factory_class, hook, lucene_class, run, t, test_class_loader, FnAnalyzer,
};

type F = BaseTokenStreamFactoryTestCase;
type B = BaseTokenStreamTestCase;

/// The UTF-16 units of `s` (`String.toCharArray()`).
fn units(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

// -----------------------------------------------------------------------------
// TestCSVUtil
// -----------------------------------------------------------------------------

fn csv_test_quote_escape_quotes() -> Result<()> {
    let input = "\"Let It Be\" is a song and album by the The Beatles.";
    let expected_output = input.replace('"', "\"\"");
    csv_impl_test_quote_escape(input, &expected_output)
}

fn csv_test_quote_escape_comma() -> Result<()> {
    let input = "To be, or not to be ...";
    let expected_output = format!("\"{input}\"");
    csv_impl_test_quote_escape(input, &expected_output)
}

fn csv_test_quote_escape_quotes_and_comma() -> Result<()> {
    let input = "\"To be, or not to be ...\" is a well-known phrase from Shakespeare's Hamlet.";
    let expected_output = format!("\"{}\"", input.replace('"', "\"\""));
    csv_impl_test_quote_escape(input, &expected_output)
}

fn csv_impl_test_quote_escape(input: &str, expected_output: &str) -> Result<()> {
    let actual_output = CSVUtil::quote_escape(input);
    Assert::assert_equals(expected_output, actual_output.as_str());
    Ok(())
}

fn csv_util_class() -> Arc<Class> {
    lucene_class("org.apache.lucene.analysis.util.TestCSVUtil")
        .method(t("testQuoteEscapeQuotes", csv_test_quote_escape_quotes))
        .method(t("testQuoteEscapeComma", csv_test_quote_escape_comma))
        .method(t(
            "testQuoteEscapeQuotesAndComma",
            csv_test_quote_escape_quotes_and_comma,
        ))
        .build()
}

// -----------------------------------------------------------------------------
// TestCharArrayIterator
// -----------------------------------------------------------------------------

/// `Locale.getDefault()`, which `LuceneTestCase` randomizes, as this
/// module's `Locale`.
fn default_locale() -> Locale {
    match rucene_test_framework::jdk::locale::get_default() {
        Some(l) => Locale::of_parts(
            l.get_language(),
            l.get_script(),
            l.get_country(),
            l.get_variant(),
        ),
        None => Locale::of_parts("", "", "", ""),
    }
}

fn cai_test_word_instance() -> Result<()> {
    cai_do_tests(CharArrayIterator::new_word_instance())
}

fn cai_test_consume_word_instance() -> Result<()> {
    // we use the default locale, as it's randomized by LuceneTestCase
    let mut bi = get_word_instance(&default_locale());
    let mut ci = CharArrayIterator::new_word_instance();
    let random = LuceneTestCase::random();
    for _ in 0..10000 {
        let text = units(&TestUtil::random_unicode_string(&*random));
        ci.set_text(&text, 0, text.len() as i32);
        cai_consume(&mut *bi, &ci);
    }
    Ok(())
}

fn cai_test_sentence_instance() -> Result<()> {
    cai_do_tests(CharArrayIterator::new_sentence_instance())
}

fn cai_test_consume_sentence_instance() -> Result<()> {
    // we use the default locale, as it's randomized by LuceneTestCase
    let mut bi = get_sentence_instance(&default_locale());
    let mut ci = CharArrayIterator::new_sentence_instance();
    let random = LuceneTestCase::random();
    for _ in 0..10000 {
        let text = units(&TestUtil::random_unicode_string(&*random));
        ci.set_text(&text, 0, text.len() as i32);
        cai_consume(&mut *bi, &ci);
    }
    Ok(())
}

fn cai_do_tests(mut ci: CharArrayIterator) -> Result<()> {
    let c = |ch: char| ch as u16;
    // basics
    ci.set_text(&units("testing"), 0, "testing".len() as i32);
    Assert::assert_equals(0, ci.get_begin_index());
    Assert::assert_equals(7, ci.get_end_index());
    Assert::assert_equals(0, ci.get_index());
    Assert::assert_equals(c('t'), ci.current());
    Assert::assert_equals(c('e'), ci.next());
    Assert::assert_equals(c('g'), ci.last());
    Assert::assert_equals(c('n'), ci.previous());
    Assert::assert_equals(c('t'), ci.first());
    Assert::assert_equals(DONE, ci.previous());

    // first()
    ci.set_text(&units("testing"), 0, "testing".len() as i32);
    ci.next();
    // Sets the position to getBeginIndex() and returns the character at that position.
    Assert::assert_equals(c('t'), ci.first());
    Assert::assert_equals(ci.get_begin_index(), ci.get_index());
    // or DONE if the text is empty
    ci.set_text(&[], 0, 0);
    Assert::assert_equals(DONE, ci.first());

    // last()
    ci.set_text(&units("testing"), 0, "testing".len() as i32);
    // Sets the position to getEndIndex()-1 (getEndIndex() if the text is empty)
    // and returns the character at that position.
    Assert::assert_equals(c('g'), ci.last());
    Assert::assert_equals(ci.get_index(), ci.get_end_index() - 1);
    // or DONE if the text is empty
    ci.set_text(&[], 0, 0);
    Assert::assert_equals(DONE, ci.last());
    Assert::assert_equals(ci.get_end_index(), ci.get_index());

    // current()
    // Gets the character at the current position (as returned by getIndex()).
    ci.set_text(&units("testing"), 0, "testing".len() as i32);
    Assert::assert_equals(c('t'), ci.current());
    ci.last();
    ci.next();
    // or DONE if the current position is off the end of the text.
    Assert::assert_equals(DONE, ci.current());

    // next()
    ci.set_text(&units("te"), 0, 2);
    // Increments the iterator's index by one and returns the character at the new index.
    Assert::assert_equals(c('e'), ci.next());
    Assert::assert_equals(1, ci.get_index());
    // or DONE if the new position is off the end of the text range.
    Assert::assert_equals(DONE, ci.next());
    Assert::assert_equals(ci.get_end_index(), ci.get_index());

    // setIndex()
    ci.set_text(&units("test"), 0, "test".len() as i32);
    expect_throws("java.lang.IllegalArgumentException", || ci.set_index(5));

    // clone()
    let text = units("testing");
    ci.set_text(&text, 0, text.len() as i32);
    ci.next();
    let mut ci2 = ci.clone();
    Assert::assert_equals(ci.get_index(), ci2.get_index());
    Assert::assert_equals(ci.next(), ci2.next());
    Assert::assert_equals(ci.last(), ci2.last());
    Ok(())
}

/// `consume(BreakIterator bi, CharacterIterator ci)`.
fn cai_consume(bi: &mut dyn BreakIterator, ci: &CharArrayIterator) {
    bi.set_text(Box::new(ci.clone()));
    while bi.next() != BI_DONE {}
}

fn char_array_iterator_class() -> Arc<Class> {
    lucene_class("org.apache.lucene.analysis.util.TestCharArrayIterator")
        .method(t("testWordInstance", cai_test_word_instance))
        .method(t("testConsumeWordInstance", cai_test_consume_word_instance))
        .method(t("testSentenceInstance", cai_test_sentence_instance))
        .method(t(
            "testConsumeSentenceInstance",
            cai_test_consume_sentence_instance,
        ))
        .build()
}

// -----------------------------------------------------------------------------
// TestElisionFilterFactory
// -----------------------------------------------------------------------------

/// `tokenFilterFactory(name, keysAndValues...)` of a test class of this
/// package: `Version.LATEST`, `new ClasspathResourceLoader(getClass())`.
fn token_filter_factory(
    name: &str,
    keys_and_values: &[&str],
) -> Result<Arc<dyn rucene::analysis::factories::TokenFilterFactory>> {
    F::token_filter_factory_with_loader(
        name,
        Some(Version::LATEST),
        &test_class_loader("org.apache.lucene.analysis.util"),
        keys_and_values,
    )
}

/// `new MockTokenizer(MockTokenizer.WHITESPACE, false)` over `text`.
fn whitespace_mock(text: &str) -> Result<Box<dyn TokenStream>> {
    let mut stream = MockTokenizer::new(Arc::clone(&WHITESPACE), false)?;
    stream.set_reader(Box::new(StringCharReader::new(text)))?;
    Ok(Box::new(stream))
}

/// Ensure the filter actually normalizes text.
fn elision_test_elision() -> Result<()> {
    let stream = whitespace_mock("l'avion")?;
    let mut stream =
        token_filter_factory("Elision", &["articles", "frenchArticles.txt"])?.create(stream)?;
    B::assert_token_stream_contents(&mut *stream, &["avion"])
}

/// Test creating an elision filter without specifying any articles
fn elision_test_default_articles() -> Result<()> {
    let stream = whitespace_mock("l'avion")?;
    let mut stream = token_filter_factory("Elision", &[])?.create(stream)?;
    B::assert_token_stream_contents(&mut *stream, &["avion"])
}

/// Test setting ignoreCase=true
fn elision_test_case_insensitive() -> Result<()> {
    let stream = whitespace_mock("L'avion")?;
    let mut stream = token_filter_factory(
        "Elision",
        &["articles", "frenchArticles.txt", "ignoreCase", "true"],
    )?
    .create(stream)?;
    B::assert_token_stream_contents(&mut *stream, &["avion"])
}

/// Test that bogus arguments result in exception
fn elision_test_bogus_arguments() -> Result<()> {
    let expected = expect_throws("java.lang.IllegalArgumentException", || {
        token_filter_factory("Elision", &["bogusArg", "bogusValue"])
    });
    Assert::assert_true(
        expected
            .message()
            .unwrap_or_default()
            .contains("Unknown parameters"),
    );
    Ok(())
}

fn elision_filter_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.util.TestElisionFilterFactory")
        .method(t("testElision", elision_test_elision))
        .method(t("testDefaultArticles", elision_test_default_articles))
        .method(t("testCaseInsensitive", elision_test_case_insensitive))
        .method(t("testBogusArguments", elision_test_bogus_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// TestFilesystemResourceLoader
// -----------------------------------------------------------------------------

/// `assertNotFound(ResourceLoader rl)`.
///
/// Java's second check, `rl.newInstance("org.apache.lucene.analysis.FooBarFilterFactory",
/// TokenFilterFactory.class)` throwing a `RuntimeException`, has no
/// counterpart: `rucene`'s `ResourceLoader` has no class half (the
/// divergence declared on `rucene::util::resource_loader`).
fn fsrl_assert_not_found(rl: &dyn ResourceLoader) -> Result<()> {
    // the resource does not exist, should fail!
    expect_throws("java.io.IOException", || {
        let stream =
            rl.open_resource("this-directory-really-really-really-should-not-exist/foo/bar.txt");
        // `IOUtils.closeWhileHandlingException(...)`: dropping closes it.
        stream.map(drop)
    });
    Ok(())
}

/// `assertClasspathDelegation(ResourceLoader rl)`.
///
/// Java's second check, `rl.newInstance("org.apache.lucene.analysis.util.RollingCharBuffer",
/// Object.class)`, has no counterpart (see [`fsrl_assert_not_found`]).
fn fsrl_assert_classpath_delegation(rl: &dyn ResourceLoader) -> Result<()> {
    // try a stopwords file from classpath
    let mut stream = rl.open_resource("org/apache/lucene/analysis/snowball/english_stop.txt")?;
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut stream, &mut bytes)
        .map_err(rucene::error::LuceneError::from)?;
    let text = String::from_utf8(bytes).expect("UTF-8");
    let mut reader = StringCharReader::new(text);
    let set = get_snowball_word_set_reader(&mut reader)?;
    Assert::assert_true(set.contains(Some("you")));
    Ok(())
}

fn fsrl_test_base_dir() -> Result<()> {
    let base = LuceneTestCase::create_temp_dir_prefix("fsResourceLoaderBase");
    let base = base.as_std_path().to_path_buf();
    std::fs::write(base.join("template.txt"), "foobar\n")
        .map_err(rucene::error::LuceneError::from)?;

    // `new FilesystemResourceLoader(base, TestFilesystemResourceLoader.class.getClassLoader())`
    let rl = FilesystemResourceLoader::with_class_loader(&base, class_loader_resources())?;
    Assert::assert_equals(
        "foobar",
        get_lines(rl.open_resource("template.txt")?, "UTF-8")?[0].as_str(),
    );
    // Same with full path name:
    let full_path = std::path::absolute(base.join("template.txt"))
        .map_err(rucene::error::LuceneError::from)?
        .to_string_lossy()
        .into_owned();
    Assert::assert_equals(
        "foobar",
        get_lines(rl.open_resource(&full_path)?, "UTF-8")?[0].as_str(),
    );
    fsrl_assert_classpath_delegation(&rl)?;
    fsrl_assert_not_found(&rl)?;
    Ok(())
}

fn fsrl_test_delegation() -> Result<()> {
    let empty = LuceneTestCase::create_temp_dir_prefix("empty");
    let rl = FilesystemResourceLoader::with_delegate(
        empty.as_std_path(),
        Box::new(StringMockResourceLoader::new("foobar\n")),
    )?;
    Assert::assert_equals(
        "foobar",
        get_lines(rl.open_resource("template.txt")?, "UTF-8")?[0].as_str(),
    );
    Ok(())
}

fn filesystem_resource_loader_class() -> Arc<Class> {
    lucene_class("org.apache.lucene.analysis.util.TestFilesystemResourceLoader")
        .method(t("testBaseDir", fsrl_test_base_dir))
        .method(t("testDelegation", fsrl_test_delegation))
        .build()
}

// -----------------------------------------------------------------------------
// TestRollingCharBuffer
// -----------------------------------------------------------------------------

fn rcb_test() -> Result<()> {
    let iters = LuceneTestCase::at_least(100);
    let mut buffer = RollingCharBuffer::new();
    let random = LuceneTestCase::random();
    for iter in 0..iters {
        let string_len = if random.next_boolean() {
            random.next_int_bound(50)
        } else {
            random.next_int_bound(20000)
        };
        let s: Vec<u16> = if string_len == 0 {
            Vec::new()
        } else {
            units(&TestUtil::random_unicode_string_max(&*random, string_len))
        };
        if *VERBOSE {
            println!("\nTEST: iter={iter} s.length()={}", s.len());
        }
        buffer.reset();
        let mut reader: Box<dyn CharReader> = Box::new(StringCharReader::from_utf16(s.clone()));
        let mut next_read: i32 = 0;
        let mut avail_count: i32 = 0;
        while (next_read as usize) < s.len() {
            if *VERBOSE {
                println!("  cycle nextRead={next_read} avail={avail_count}");
            }
            if avail_count == 0 || random.next_boolean() {
                // Read next char
                if *VERBOSE {
                    println!("    new char");
                }
                Assert::assert_equals(
                    s[next_read as usize] as i32,
                    buffer.get(next_read, &mut *reader)?,
                );
                next_read += 1;
                avail_count += 1;
            } else if random.next_boolean() {
                // Read previous char
                let pos = TestUtil::next_int(&*random, next_read - avail_count, next_read - 1);
                if *VERBOSE {
                    println!("    old char pos={pos}");
                }
                Assert::assert_equals(s[pos as usize] as i32, buffer.get(pos, &mut *reader)?);
            } else {
                // Read slice
                let length = if avail_count == 1 {
                    1
                } else {
                    TestUtil::next_int(&*random, 1, avail_count)
                };
                let start = if length == avail_count {
                    next_read - avail_count
                } else {
                    next_read - avail_count + random.next_int_bound(avail_count - length)
                };
                if *VERBOSE {
                    println!("    slice start={start} length={length}");
                }
                let expected = &s[start as usize..(start + length) as usize];
                let actual = buffer.get_range(start, length as usize);
                Assert::assert_equals(
                    String::from_utf16_lossy(expected),
                    String::from_utf16_lossy(&actual),
                );
                Assert::assert_true(expected == actual.as_slice());
            }
            if avail_count > 0 && random.next_int_bound(20) == 17 {
                let to_free = random.next_int_bound(avail_count);
                if *VERBOSE {
                    println!("    free {to_free} (avail={})", avail_count - to_free);
                }
                buffer.free_before(next_read - (avail_count - to_free));
                avail_count -= to_free;
            }
        }
    }
    Ok(())
}

fn rolling_char_buffer_class() -> Arc<Class> {
    lucene_class("org.apache.lucene.analysis.util.TestRollingCharBuffer")
        .method(t("test", rcb_test))
        .build()
}

// -----------------------------------------------------------------------------
// TestSegmentingTokenizerBase
// -----------------------------------------------------------------------------

/// `Character.isLetterOrDigit(char)`.
fn is_letter_or_digit(ch: u16) -> bool {
    is_letter(ch as i32) || is_digit(ch as i32)
}

/// silly tokenizer that just returns whole sentences as tokens
/// (`WholeSentenceTokenizer`).
#[derive(Debug, Default)]
struct WholeSentenceTokenizer {
    sentence_start: i32,
    sentence_end: i32,
    has_sentence: bool,
}

impl SegmentingTokenizerBaseLogic for WholeSentenceTokenizer {
    fn set_next_sentence(&mut self, _buffer: &[u16], sentence_start: i32, sentence_end: i32) {
        self.sentence_start = sentence_start;
        self.sentence_end = sentence_end;
        self.has_sentence = true;
    }

    fn increment_word(
        &mut self,
        source: &mut rucene::util::AttributeSource,
        buffer: &[u16],
        offset: i32,
        input: &dyn CharReader,
    ) -> bool {
        if self.has_sentence {
            self.has_sentence = false;
            source.clear_attributes();
            let chars = rucene_analysis_common::jdk::character::utf16_to_term_chars(
                &buffer[self.sentence_start as usize..self.sentence_end as usize],
            );
            source
                .get_attribute_interface_mut::<dyn CharTermAttribute>()
                .expect("termAtt")
                .copy_buffer(&chars, 0, chars.len());
            let start = correct_offset(Some(input), offset + self.sentence_start);
            let end = correct_offset(Some(input), offset + self.sentence_end);
            source
                .get_attribute_interface_mut::<dyn OffsetAttribute>()
                .expect("offsetAtt")
                .set_offset(start, end);
            true
        } else {
            false
        }
    }
}

/// simple tokenizer, that bumps posinc + 1 for tokens after a sentence
/// boundary to inhibit phrase queries without slop
/// (`SentenceAndWordTokenizer`).
#[derive(Debug)]
struct SentenceAndWordTokenizer {
    sentence_start: i32,
    sentence_end: i32,
    word_start: i32,
    word_end: i32,
    /// initially set to -1 so the first word in the document doesn't get a
    /// pos boost
    pos_boost: i32,
}

impl SegmentingTokenizerBaseLogic for SentenceAndWordTokenizer {
    fn set_next_sentence(&mut self, _buffer: &[u16], sentence_start: i32, sentence_end: i32) {
        self.word_start = sentence_start;
        self.word_end = sentence_start;
        self.sentence_start = sentence_start;
        self.sentence_end = sentence_end;
        self.pos_boost += 1;
    }

    fn reset(&mut self, _source: &mut rucene::util::AttributeSource) -> Result<()> {
        self.pos_boost = -1;
        Ok(())
    }

    fn increment_word(
        &mut self,
        source: &mut rucene::util::AttributeSource,
        buffer: &[u16],
        offset: i32,
        input: &dyn CharReader,
    ) -> bool {
        self.word_start = self.word_end;
        while self.word_start < self.sentence_end {
            if is_letter_or_digit(buffer[self.word_start as usize]) {
                break;
            }
            self.word_start += 1;
        }
        if self.word_start == self.sentence_end {
            return false;
        }
        self.word_end = self.word_start + 1;
        while self.word_end < self.sentence_end
            && is_letter_or_digit(buffer[self.word_end as usize])
        {
            self.word_end += 1;
        }
        source.clear_attributes();
        let chars = rucene_analysis_common::jdk::character::utf16_to_term_chars(
            &buffer[self.word_start as usize..self.word_end as usize],
        );
        source
            .get_attribute_interface_mut::<dyn CharTermAttribute>()
            .expect("termAtt")
            .copy_buffer(&chars, 0, chars.len());
        let start = correct_offset(Some(input), offset + self.word_start);
        let end = correct_offset(Some(input), offset + self.word_end);
        source
            .get_attribute_interface_mut::<dyn OffsetAttribute>()
            .expect("offsetAtt")
            .set_offset(start, end);
        let mut pos_inc = source
            .get_attribute_interface_mut::<dyn PositionIncrementAttribute>()
            .expect("posIncAtt");
        let increment = pos_inc.get_position_increment() + self.pos_boost;
        pos_inc.set_position_increment(increment);
        self.pos_boost = 0;
        true
    }
}

/// `Tokenizer.correctOffset(int)` of the tokenizer whose input is `input`.
fn correct_offset(input: Option<&dyn CharReader>, current_off: i32) -> i32 {
    rucene::analysis::Tokenizer::<
        rucene_analysis_common::analysis::util::segmenting_tokenizer_base::SegmentingTokenizerBaseAdapter<
            WholeSentenceTokenizer,
        >,
    >::correct_offset_of(input, current_off)
}

/// `new WholeSentenceTokenizer()`: `super(newAttributeFactory(),
/// BreakIterator.getSentenceInstance(Locale.ROOT))`, then its fields
/// `termAtt` and `offsetAtt`.
fn whole_sentence_tokenizer() -> Result<
    rucene_analysis_common::analysis::util::segmenting_tokenizer_base::SegmentingTokenizerBase<
        WholeSentenceTokenizer,
    >,
> {
    let mut source = segmenting_tokenizer_base_source(B::new_attribute_factory())?;
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_interface::<dyn OffsetAttribute>()?;
    Ok(new_segmenting_tokenizer_base(
        source,
        get_sentence_instance(&Locale::of_parts("", "", "", "")),
        WholeSentenceTokenizer::default(),
    ))
}

/// `new SentenceAndWordTokenizer()`.
fn sentence_and_word_tokenizer() -> Result<
    rucene_analysis_common::analysis::util::segmenting_tokenizer_base::SegmentingTokenizerBase<
        SentenceAndWordTokenizer,
    >,
> {
    let mut source = segmenting_tokenizer_base_source(B::new_attribute_factory())?;
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_interface::<dyn OffsetAttribute>()?;
    source.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
    Ok(new_segmenting_tokenizer_base(
        source,
        get_sentence_instance(&Locale::of_parts("", "", "", "")),
        SentenceAndWordTokenizer {
            sentence_start: 0,
            sentence_end: 0,
            word_start: 0,
            word_end: 0,
            pos_boost: -1,
        },
    ))
}

/// The two analyzers `setUp` creates and `tearDown` closes.
type AnalyzerPair = (Arc<dyn Analyzer>, Arc<dyn Analyzer>);

thread_local! {
    /// The instance fields `sentence` and `sentenceAndWord` `setUp` sets.
    static STB: RefCell<Option<AnalyzerPair>> = const { RefCell::new(None) };
}

fn stb_set_up() -> Result<()> {
    LuceneTestCase::super_set_up();
    let sentence = FnAnalyzer::new(
        "org.apache.lucene.analysis.util.TestSegmentingTokenizerBase$1",
        |_| {
            Ok(tokenizer_components(Arc::new(
                rucene::util::atomic_ref_cell::AtomicRefCell::new(whole_sentence_tokenizer()?),
            )))
        },
    )
    .arc();
    let sentence_and_word = FnAnalyzer::new(
        "org.apache.lucene.analysis.util.TestSegmentingTokenizerBase$2",
        |_| {
            Ok(tokenizer_components(Arc::new(
                rucene::util::atomic_ref_cell::AtomicRefCell::new(sentence_and_word_tokenizer()?),
            )))
        },
    )
    .arc();
    STB.with(|s| *s.borrow_mut() = Some((sentence, sentence_and_word)));
    Ok(())
}

fn stb_tear_down() -> Result<()> {
    // `IOUtils.close(sentence, sentenceAndWord)`
    if let Some((a, b)) = STB.with(|s| s.borrow_mut().take()) {
        a.close();
        b.close();
    }
    LuceneTestCase::super_tear_down();
    Ok(())
}

fn sentence() -> Arc<dyn Analyzer> {
    STB.with(|s| Arc::clone(&s.borrow().as_ref().expect("setUp").0))
}

fn sentence_and_word() -> Arc<dyn Analyzer> {
    STB.with(|s| Arc::clone(&s.borrow().as_ref().expect("setUp").1))
}

/// Some simple examples, just outputting the whole sentence boundaries as "terms"
fn stb_test_basics() -> Result<()> {
    B::assert_analyzes_to(
        &*sentence(),
        "The acronym for United States is U.S. but this doesn't end a sentence",
        &["The acronym for United States is U.S. but this doesn't end a sentence"],
    )?;
    B::assert_analyzes_to(
        &*sentence(),
        "He said, \"Are you going?\" John shook his head.",
        &["He said, \"Are you going?\" ", "John shook his head."],
    )
}

/// Test a subclass that sets some custom attribute values
fn stb_test_custom_attributes() -> Result<()> {
    B::assert_analyzes_to_offsets_pos_increments(
        &*sentence_and_word(),
        "He said, \"Are you going?\" John shook his head.",
        &[
            "He", "said", "Are", "you", "going", "John", "shook", "his", "head",
        ],
        &[0, 3, 10, 14, 18, 26, 31, 37, 41],
        &[2, 7, 13, 17, 23, 30, 36, 40, 45],
        &[1, 1, 1, 1, 1, 2, 1, 1, 1],
    )
}

/// Tests tokenstream reuse
fn stb_test_reuse() -> Result<()> {
    B::assert_analyzes_to_offsets_pos_increments(
        &*sentence_and_word(),
        "He said, \"Are you going?\"",
        &["He", "said", "Are", "you", "going"],
        &[0, 3, 10, 14, 18],
        &[2, 7, 13, 17, 23],
        &[1, 1, 1, 1, 1],
    )?;
    B::assert_analyzes_to_offsets_pos_increments(
        &*sentence_and_word(),
        "John shook his head.",
        &["John", "shook", "his", "head"],
        &[0, 5, 11, 15],
        &[4, 10, 14, 19],
        &[1, 1, 1, 1],
    )
}

/// Tests TokenStream.end()
fn stb_test_end() -> Result<()> {
    // BaseTokenStreamTestCase asserts that end() is set to our StringReader's length for us here.
    // we add some junk whitespace to the end just to test it.
    B::assert_analyzes_to(
        &*sentence_and_word(),
        "John shook his head          ",
        &["John", "shook", "his", "head"],
    )?;
    B::assert_analyzes_to(
        &*sentence_and_word(),
        "John shook his head.          ",
        &["John", "shook", "his", "head"],
    )
}

/// Tests terms which span across boundaries
fn stb_test_huge_doc() -> Result<()> {
    let mut sb = "\n".repeat(4094);
    sb.push_str("testing 1234");
    B::assert_analyzes_to(&*sentence_and_word(), &sb, &["testing", "1234"])
}

/// Tests the handling of binary/malformed data
fn stb_test_huge_term() -> Result<()> {
    let input = "a".repeat(10240);
    let expected_token = "a".repeat(1024);
    let expected: Vec<&str> = vec![expected_token.as_str(); 10];
    B::assert_analyzes_to(&*sentence(), &input, &expected)
}

/// blast some random strings through the analyzer
fn stb_test_random_strings() -> Result<()> {
    let random = LuceneTestCase::random();
    B::check_random_data(&*random, &sentence(), 200 * *RANDOM_MULTIPLIER)?;
    B::check_random_data(&*random, &sentence_and_word(), 200 * *RANDOM_MULTIPLIER)
}

fn segmenting_tokenizer_base_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.util.TestSegmentingTokenizerBase")
        .method(hook("setUp", stb_set_up))
        .method(hook("tearDown", stb_tear_down))
        .method(t("testBasics", stb_test_basics))
        .method(t("testCustomAttributes", stb_test_custom_attributes))
        .method(t("testReuse", stb_test_reuse))
        .method(t("testEnd", stb_test_end))
        .method(t("testHugeDoc", stb_test_huge_doc))
        .method(t("testHugeTerm", stb_test_huge_term))
        .method(t("testRandomStrings", stb_test_random_strings))
        .build()
}

// -----------------------------------------------------------------------------
// The suites
// -----------------------------------------------------------------------------

#[test]
fn test_csv_util() {
    run(csv_util_class);
}

#[test]
fn test_char_array_iterator() {
    run(char_array_iterator_class);
}

#[test]
fn test_elision_filter_factory() {
    run(elision_filter_factory_class);
}

#[test]
fn test_filesystem_resource_loader() {
    run(filesystem_resource_loader_class);
}

#[test]
fn test_rolling_char_buffer() {
    run(rolling_char_buffer_class);
}

#[test]
fn test_segmenting_tokenizer_base() {
    run(segmenting_tokenizer_base_class);
}
