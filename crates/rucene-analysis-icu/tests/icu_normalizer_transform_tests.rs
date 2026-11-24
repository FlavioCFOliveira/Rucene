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

//! Port of the `org.apache.lucene.analysis.icu` test classes of Lucene
//! 10.5.0 `lucene/analysis/icu/src/test` over normalization, folding and
//! transforms, each run as a suite by the ported runner:
//! `TestICUFoldingFilter`, `TestICUFoldingFilterFactory`,
//! `TestICUNormalizer2CharFilter`, `TestICUNormalizer2CharFilterFactory`,
//! `TestICUNormalizer2Filter`, `TestICUNormalizer2FilterFactory`,
//! `TestICUTransformFilter` and `TestICUTransformFilterFactory`.
//!
//! ICU4J's Java strings are UTF-16 code units in the `icu4j` port
//! (`&[u16]`/`Vec<u16>`): [`u`] and [`s`] convert at the test boundary.

mod support;

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use icu4j::text::normalizer2::{self, Mode, Normalizer2};
use icu4j::text::transliterator::{self, Transliterator, FORWARD};
use icu4j::text::unicode_set::UnicodeSet;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::factories::{CharFilterFactory, TokenFilterFactory};
use rucene::analysis::{Analyzer, CharFilter, CharReader, StringCharReader, TokenStream};
use rucene::error::Result;
use rucene::index::index_writer::MAX_TERM_LENGTH;
use rucene_analysis_common::analysis::core::keyword_tokenizer::new_keyword_tokenizer;
use rucene_analysis_common::analysis::ngram::ngram_tokenizer::new_ngram_tokenizer_with_factory;
use rucene_analysis_icu::analysis::icu::icu_folding_filter::ICUFoldingFilter;
use rucene_analysis_icu::analysis::icu::icu_folding_filter_factory::ICUFoldingFilterFactory;
use rucene_analysis_icu::analysis::icu::icu_normalizer2_char_filter::ICUNormalizer2CharFilter;
use rucene_analysis_icu::analysis::icu::icu_normalizer2_char_filter_factory::ICUNormalizer2CharFilterFactory;
use rucene_analysis_icu::analysis::icu::icu_normalizer2_filter::{
    new_icu_normalizer2_filter, new_icu_normalizer2_filter_with,
};
use rucene_analysis_icu::analysis::icu::icu_normalizer2_filter_factory::ICUNormalizer2FilterFactory;
use rucene_analysis_icu::analysis::icu::icu_transform_filter::new_icu_transform_filter;
use rucene_analysis_icu::analysis::icu::icu_transform_filter_factory::ICUTransformFilterFactory;
use rucene_test_framework::analysis::mock_tokenizer::{MockTokenizer, KEYWORD, WHITESPACE};
use rucene_test_framework::analysis::{tokenizer_components, BaseTokenStreamTestCase};
use rucene_test_framework::util::lucene_test_case::RANDOM_MULTIPLIER;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};
use support::{
    boxed, btstc_class, hook, iae_message, keyword_tokenizer_analyzer, own, run, t,
    whitespace_analyzer, FnAnalyzer,
};

type B = BaseTokenStreamTestCase;

/// `random()`.
fn random() -> Arc<dyn Random> {
    LuceneTestCase::random()
}

/// `RANDOM_MULTIPLIER`.
fn mult() -> i32 {
    *RANDOM_MULTIPLIER
}

/// A Java `String` literal as UTF-16 code units.
fn u(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

/// UTF-16 code units as a Rust string (the tests' strings are well formed).
fn s(units: &[u16]) -> String {
    String::from_utf16(units).expect("well-formed UTF-16")
}

/// `Normalizer2.getInstance(null, name, mode)`.
fn norm(name: &str, mode: Mode) -> Arc<dyn Normalizer2> {
    normalizer2::get_instance(None, name, mode).expect("a built-in normalizer")
}

/// `normalizer.normalize(input)`.
fn normalize(normalizer: &dyn Normalizer2, input: &[u16]) -> Vec<u16> {
    normalizer.normalize_str(input).expect("normalize")
}

/// `new HashMap<String, String>()` filled with `pairs`.
fn args(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

/// `whitespaceMockTokenizer(new StringReader(text))`.
fn whitespace_stream(text: &str) -> Result<Box<dyn TokenStream>> {
    Ok(boxed(B::whitespace_mock_tokenizer(Box::new(
        StringCharReader::new(text),
    ))?))
}

thread_local! {
    /// The `Analyzer a` field of the suites that build one in `setUp`.
    static A: RefCell<Option<Arc<dyn Analyzer>>> = const { RefCell::new(None) };
}

fn set_analyzer(a: Arc<dyn Analyzer>) {
    A.with(|cell| *cell.borrow_mut() = Some(a));
}

fn analyzer() -> Arc<dyn Analyzer> {
    A.with(|cell| Arc::clone(cell.borrow().as_ref().expect("setUp")))
}

/// `a.close(); super.tearDown();`.
fn close_analyzer_tear_down() -> Result<()> {
    if let Some(a) = A.with(|cell| cell.borrow_mut().take()) {
        a.close();
    }
    LuceneTestCase::super_tear_down();
    Ok(())
}

// -----------------------------------------------------------------------------
// TestICUFoldingFilter
// -----------------------------------------------------------------------------

fn ff_set_up() -> Result<()> {
    LuceneTestCase::super_set_up();
    set_analyzer(whitespace_analyzer(
        "org.apache.lucene.analysis.icu.TestICUFoldingFilter$1",
        |t| Ok(boxed(ICUFoldingFilter::new(t)?)),
    ));
    Ok(())
}

fn ff_test_defaults() -> Result<()> {
    let a = analyzer();
    let a = &*a;
    // case folding
    B::assert_analyzes_to(a, "This is a test", &["this", "is", "a", "test"])?;
    // case folding
    B::assert_analyzes_to(a, "Ruß", &["russ"])?;
    // case folding with accent removal
    B::assert_analyzes_to(a, "ΜΆΪΟΣ", &["μαιοσ"])?;
    B::assert_analyzes_to(a, "Μάϊος", &["μαιοσ"])?;
    // supplementary case folding
    B::assert_analyzes_to(a, "𐐖", &["𐐾"])?;
    // normalization
    B::assert_analyzes_to(a, "ﴳﴺﰧ", &["طمطمطم"])?;
    // removal of default ignorables
    B::assert_analyzes_to(a, "क्‍ष", &["कष"])?;
    // removal of latin accents (composed)
    B::assert_analyzes_to(a, "résumé", &["resume"])?;
    // removal of latin accents (decomposed)
    B::assert_analyzes_to(a, "re\u{0301}sume\u{0301}", &["resume"])?;
    // fold native digits
    B::assert_analyzes_to(a, "৭০৬", &["706"])?;
    // ascii-folding-filter type stuff
    B::assert_analyzes_to(a, "đis is cræzy", &["dis", "is", "craezy"])?;
    // proper downcasing of Turkish dotted-capital I
    // (according to default case folding rules)
    B::assert_analyzes_to(a, "ELİF", &["elif"])?;
    // handling of decomposed combining-dot-above
    B::assert_analyzes_to(a, "eli\u{0307}f", &["elif"])
}

/// blast some random strings through the analyzer
fn ff_test_random_strings() -> Result<()> {
    B::check_random_data(&*random(), &analyzer(), 200 * mult())
}

fn ff_test_empty_term() -> Result<()> {
    let a = keyword_tokenizer_analyzer(
        "org.apache.lucene.analysis.icu.TestICUFoldingFilter$2",
        |t| Ok(boxed(ICUFoldingFilter::new(t)?)),
    );
    B::check_one_term(&*a, "", "")?;
    a.close();
    Ok(())
}

fn folding_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.icu.TestICUFoldingFilter")
        .method(hook("setUp", ff_set_up))
        .method(hook("tearDown", close_analyzer_tear_down))
        .method(t("testDefaults", ff_test_defaults))
        .method(t("testRandomStrings", ff_test_random_strings))
        .method(t("testEmptyTerm", ff_test_empty_term))
        .build()
}

// -----------------------------------------------------------------------------
// TestICUFoldingFilterFactory
// -----------------------------------------------------------------------------

/// basic tests to ensure the folding is working
fn fff_test() -> Result<()> {
    let factory = ICUFoldingFilterFactory::new(&mut HashMap::new())?;
    let stream = whitespace_stream("Résumé")?;
    let mut stream = factory.create(stream)?;
    B::assert_token_stream_contents(&mut *stream, &["resume"])
}

/// test to ensure the filter parameter is working
fn fff_test_filter() -> Result<()> {
    let factory = ICUFoldingFilterFactory::new(&mut args(&[("filter", "[^ö]")]))?;
    let stream = whitespace_stream("Résumé")?;
    let mut stream = factory.create(stream)?;
    B::assert_token_stream_contents(&mut *stream, &["resume"])?;

    let stream = whitespace_stream("Fönster")?;
    let mut stream = factory.create(stream)?;
    B::assert_token_stream_contents(&mut *stream, &["fönster"])
}

/// Test that bogus arguments result in exception
fn fff_test_bogus_arguments() -> Result<()> {
    let expected =
        iae_message(|| ICUFoldingFilterFactory::new(&mut args(&[("bogusArg", "bogusValue")])));
    Assert::assert_true(expected.contains("Unknown parameters"));
    Ok(())
}

fn folding_filter_factory_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.icu.TestICUFoldingFilterFactory")
        .method(t("test", fff_test))
        .method(t("testFilter", fff_test_filter))
        .method(t("testBogusArguments", fff_test_bogus_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// TestICUNormalizer2Filter
// -----------------------------------------------------------------------------

fn nf_set_up() -> Result<()> {
    LuceneTestCase::super_set_up();
    set_analyzer(whitespace_analyzer(
        "org.apache.lucene.analysis.icu.TestICUNormalizer2Filter$1",
        |t| Ok(boxed(new_icu_normalizer2_filter(t)?)),
    ));
    Ok(())
}

/// Test the default normalization: NFKC_Casefold
fn nf_test_defaults() -> Result<()> {
    let a = analyzer();
    let a = &*a;
    // case folding
    B::assert_analyzes_to(a, "This is a test", &["this", "is", "a", "test"])?;
    // case folding
    B::assert_analyzes_to(a, "Ruß", &["russ"])?;
    // case folding
    B::assert_analyzes_to(a, "ΜΆΪΟΣ", &["μάϊοσ"])?;
    B::assert_analyzes_to(a, "Μάϊος", &["μάϊοσ"])?;
    // supplementary case folding
    B::assert_analyzes_to(a, "𐐖", &["𐐾"])?;
    // normalization
    B::assert_analyzes_to(a, "ﴳﴺﰧ", &["طمطمطم"])?;
    // removal of default ignorables
    B::assert_analyzes_to(a, "क्‍ष", &["क्ष"])
}

fn nf_test_alternate() -> Result<()> {
    let a = whitespace_analyzer(
        "org.apache.lucene.analysis.icu.TestICUNormalizer2Filter$2",
        |t| {
            Ok(boxed(new_icu_normalizer2_filter_with(
                t,
                /* specify nfc with decompose to get nfd */
                norm("nfc", Mode::Decompose),
            )?))
        },
    );
    // decompose EAcute into E + combining Acute
    B::assert_analyzes_to(&*a, "\u{00E9}", &["\u{0065}\u{0301}"])?;
    a.close();
    Ok(())
}

/// blast some random strings through the analyzer
fn nf_test_random_strings() -> Result<()> {
    B::check_random_data(&*random(), &analyzer(), 200 * mult())
}

fn nf_test_empty_term() -> Result<()> {
    let a = keyword_tokenizer_analyzer(
        "org.apache.lucene.analysis.icu.TestICUNormalizer2Filter$3",
        |t| Ok(boxed(new_icu_normalizer2_filter(t)?)),
    );
    B::check_one_term(&*a, "", "")?;
    a.close();
    Ok(())
}

fn normalizer2_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.icu.TestICUNormalizer2Filter")
        .method(hook("setUp", nf_set_up))
        .method(hook("tearDown", close_analyzer_tear_down))
        .method(t("testDefaults", nf_test_defaults))
        .method(t("testAlternate", nf_test_alternate))
        .method(t("testRandomStrings", nf_test_random_strings))
        .method(t("testEmptyTerm", nf_test_empty_term))
        .build()
}

// -----------------------------------------------------------------------------
// TestICUNormalizer2FilterFactory
// -----------------------------------------------------------------------------

/// Test nfkc_cf defaults
fn nff_test_defaults() -> Result<()> {
    let factory = ICUNormalizer2FilterFactory::new(&mut HashMap::new())?;
    let stream = whitespace_stream("This is a Ｔｅｓｔ")?;
    let mut stream = factory.create(stream)?;
    B::assert_token_stream_contents(&mut *stream, &["this", "is", "a", "test"])
}

/// Test nfkc form
fn nff_test_form_argument() -> Result<()> {
    let factory = ICUNormalizer2FilterFactory::new(&mut args(&[("form", "nfkc")]))?;
    let stream = whitespace_stream("This is a Ｔｅｓｔ")?;
    let mut stream = factory.create(stream)?;
    B::assert_token_stream_contents(&mut *stream, &["This", "is", "a", "Test"])
}

/// Test nfkc_scf form
fn nff_test_simple_case_fold() -> Result<()> {
    // example from https://www.w3.org/TR/charmod-norm/#dfn-unicode-simple
    let factory = ICUNormalizer2FilterFactory::new(&mut args(&[("form", "nfkc_scf")]))?;
    let stream = whitespace_stream("ᾛ")?;
    let mut stream = factory.create(stream)?;
    B::assert_token_stream_contents(&mut *stream, &["ᾓ"])
}

/// Test that bogus arguments result in exception
fn nff_test_bogus_arguments() -> Result<()> {
    let expected =
        iae_message(|| ICUNormalizer2FilterFactory::new(&mut args(&[("bogusArg", "bogusValue")])));
    Assert::assert_true(expected.contains("Unknown parameters"));
    Ok(())
}

fn normalizer2_filter_factory_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.icu.TestICUNormalizer2FilterFactory")
        .method(t("testDefaults", nff_test_defaults))
        .method(t("testFormArgument", nff_test_form_argument))
        .method(t("testSimpleCaseFold", nff_test_simple_case_fold))
        .method(t("testBogusArguments", nff_test_bogus_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// TestICUNormalizer2CharFilter
// -----------------------------------------------------------------------------

fn ncf_test_normalization() -> Result<()> {
    let input =
        u("ʰ㌰゙5℃№㈱㌘，バッファーの正規化のテスト．㋐㋑㋒㋓㋔ｶｷｸｹｺｻﾞｼﾞｽﾞｾﾞｿﾞg̈각/각நிเกषिchkʷक्षि");
    let normalizer = norm("nfkc_cf", Mode::Compose);
    let expected_output = normalize(&*normalizer, &input);
    let mut reader = ICUNormalizer2CharFilter::with_normalizer(
        Box::new(StringCharReader::from_utf16(input.clone())),
        Arc::clone(&normalizer),
    )?;
    let mut temp_buff = [0u16; 10];
    let mut output: Vec<u16> = Vec::new();
    loop {
        let length = reader.read(&mut temp_buff)?;
        if length == 0 {
            break;
        }
        output.extend_from_slice(&temp_buff[..length]);
        let corrected = reader.correct_offset(output.len() as i32) as usize;
        Assert::assert_equals_str(
            None,
            Some(&s(&output)),
            Some(&s(&normalize(&*normalizer, &input[..corrected]))),
        );
    }
    Assert::assert_equals_str(None, Some(&s(&expected_output)), Some(&s(&output)));
    Ok(())
}

fn ncf_test_token_stream() -> Result<()> {
    // '℃', '№', '㈱', '㌘', 'ｻ'+'<<', 'ｿ'+'<<', '㌰'+'<<'
    let input = "℃ № ㈱ ㌘ ｻﾞ ｿﾞ ㌰ﾞ";
    let reader = ICUNormalizer2CharFilter::with_normalizer(
        Box::new(StringCharReader::new(input)),
        norm("nfkc", Mode::Compose),
    )?;
    let mut token_stream = MockTokenizer::new(Arc::clone(&WHITESPACE), false)?;
    token_stream.set_reader(Box::new(reader))?;
    B::assert_token_stream_contents_offsets_final_offset(
        &mut token_stream,
        &["°C", "No", "(株)", "グラム", "ザ", "ゾ", "ピゴ"],
        &[0, 2, 4, 6, 8, 11, 14],
        &[1, 3, 5, 7, 10, 13, 16],
        Some(u(input).len() as i32),
    )
}

fn ncf_test_token_stream2() -> Result<()> {
    // '㌰', '<<'゙, '5', '℃', '№', '㈱', '㌘', 'ｻ', '<<', 'ｿ', '<<'
    let input = "㌰゙5℃№㈱㌘ｻﾞｿﾞ";
    let reader = ICUNormalizer2CharFilter::with_normalizer(
        Box::new(StringCharReader::new(input)),
        norm("nfkc_cf", Mode::Compose),
    )?;
    let mut token_stream = new_ngram_tokenizer_with_factory(B::new_attribute_factory(), 1, 1)?;
    token_stream.set_reader(Box::new(reader))?;
    B::assert_token_stream_contents_offsets_final_offset(
        &mut token_stream,
        &[
            "ピ", "ゴ", "5", "°", "c", "n", "o", "(", "株", ")", "グ", "ラ", "ム", "ザ", "ゾ",
        ],
        &[0, 1, 2, 3, 3, 4, 4, 5, 5, 5, 6, 6, 6, 7, 9],
        &[1, 2, 3, 3, 4, 4, 5, 5, 5, 6, 6, 6, 7, 9, 11],
        Some(u(input).len() as i32),
    )
}

/// The observable state of a [`TransparentReader`]: its private fields,
/// which the test reads after handing the reader to the char filter.
#[derive(Debug)]
struct TransparentState {
    cursor_position: i32,
    finished: bool,
}

/// A wrapping reader that provides transparency wrt how much of the
/// underlying stream has been consumed. This may be used to validate
/// incremental behavior in upstream reads.
#[derive(Debug)]
struct TransparentReader {
    expect_size: i32,
    backing: Box<dyn CharReader>,
    state: Arc<Mutex<TransparentState>>,
}

impl TransparentReader {
    fn new(backing: Box<dyn CharReader>, expect_size: i32) -> TransparentReader {
        TransparentReader {
            expect_size,
            backing,
            state: Arc::new(Mutex::new(TransparentState {
                cursor_position: -1,
                finished: false,
            })),
        }
    }
}

impl CharReader for TransparentReader {
    fn read(&mut self, cbuf: &mut [u16]) -> Result<usize> {
        let ret = self.backing.read(cbuf)?;
        // `read` answers 0 where Java's answers -1 (end of stream).
        let mut st = self.state.lock().unwrap();
        if st.cursor_position == -1 {
            if ret != 0 {
                st.cursor_position = ret as i32;
            } else {
                st.cursor_position = 0;
            }
        } else if ret == 0 {
            assert!(st.finished);
        } else {
            st.cursor_position += ret as i32;
        }
        if !st.finished && st.cursor_position >= self.expect_size {
            st.finished = true;
        }
        Ok(ret)
    }

    fn close(&mut self) -> Result<()> {
        {
            let st = self.state.lock().unwrap();
            assert!(st.finished && st.cursor_position != -1);
        }
        self.backing.close()
    }
}

fn ncf_test_incremental_no_inert_chars() -> Result<()> {
    let input = u("℃aa℃aa℃aa℃aa℃aa℃aa℃aa℃aa℃aa℃aa℃aa℃aa℃aa℃aa");
    let input_length = input.len() as i32;
    let expect_output = "°caa°caa°caa°caa°caa°caa°caa°caa°caa°caa°caa°caa°caa°caa";
    let norm = norm("nfkc_cf", Mode::Compose);

    // Sanity check/demonstrate that the input doesn't have any normalization-inert codepoints
    for i in (0..input.len()).rev() {
        let cp = rucene::analysis::character_utils::code_point_at(&input, i, input.len());
        Assert::assert_false(norm.is_inert(cp)?);
    }

    let outer_buffer_size = random().next_int_bound(8) + 1; // buffer size between 1 <= n <= 8
    let mut temp_buff = vec![0u16; outer_buffer_size as usize];

    /*
    Sanity check that with the default inner buffer size, the upstream input is read in its
    entirety after the first external read.
     */
    let tr = TransparentReader::new(
        Box::new(StringCharReader::from_utf16(input.clone())),
        input_length,
    );
    let tr_state = Arc::clone(&tr.state);
    let mut reader = ICUNormalizer2CharFilter::with_normalizer(Box::new(tr), Arc::clone(&norm))?;
    Assert::assert_true(reader.read(&mut temp_buff)? > 0);
    Assert::assert_equals(input_length, tr_state.lock().unwrap().cursor_position);
    Assert::assert_true(tr_state.lock().unwrap().finished);

    /*
    By way of contrast with the "control" above, we now check the actual desired behavior, ensuring that
    even for input with no inert chars, we're able to read input incrementally. To support incremental
    upstream reads with reasonable-sized input, we artificially reduce the size of the internal buffer
     */
    let inner_buffer_size = random().next_int_bound(7) + 2; // buffer size between 2 <= n <= 8
    let tr = TransparentReader::new(
        Box::new(StringCharReader::from_utf16(input.clone())),
        input_length,
    );
    let tr_state = Arc::clone(&tr.state);
    let mut reader =
        ICUNormalizer2CharFilter::with_buffer_size(Box::new(tr), norm, inner_buffer_size as usize)?;
    let mut result: Vec<u16> = Vec::new();
    let mut incremental_reads = 0;
    let mut finished_reads = 0;
    loop {
        let length = reader.read(&mut temp_buff)?;
        if length == 0 {
            Assert::assert_true(tr_state.lock().unwrap().finished);
            break;
        }
        result.extend_from_slice(&temp_buff[..length]);
        if length > 0 {
            let st = tr_state.lock().unwrap();
            if !st.finished {
                incremental_reads += 1;
                Assert::assert_true(st.cursor_position < input_length);
            } else {
                finished_reads += 1;
            }
        }
    }
    Assert::assert_true(incremental_reads > finished_reads);
    Assert::assert_equals_str(None, Some(expect_output), Some(&s(&result)));
    Ok(())
}

fn ncf_test_massive_ligature() -> Result<()> {
    let input = "\u{FDFA}";
    let reader = ICUNormalizer2CharFilter::with_normalizer(
        Box::new(StringCharReader::new(input)),
        norm("nfkc_cf", Mode::Compose),
    )?;
    let mut token_stream = MockTokenizer::new(Arc::clone(&WHITESPACE), false)?;
    token_stream.set_reader(Box::new(reader))?;
    B::assert_token_stream_contents_offsets_final_offset(
        &mut token_stream,
        &["صلى", "الله", "عليه", "وسلم"],
        &[0, 0, 0, 0],
        &[0, 0, 0, 1],
        Some(u(input).len() as i32),
    )
}

/// An anonymous analyzer over `new MockTokenizer(runAutomaton, false[,
/// maxTokenLength])` whose `initReader` wraps the reader in
/// `new ICUNormalizer2CharFilter(reader, normalizer[, bufferSize])`.
fn char_filter_analyzer(
    name: &'static str,
    keyword: bool,
    max_token_length: Option<i32>,
    normalizer: Arc<dyn Normalizer2>,
    buffer_size: Option<i32>,
) -> Arc<dyn Analyzer> {
    FnAnalyzer::new(name, move |_| {
        let automaton = Arc::clone(if keyword { &KEYWORD } else { &WHITESPACE });
        let tokenizer = match max_token_length {
            Some(max) => MockTokenizer::with_max_token_length(automaton, false, max)?,
            None => MockTokenizer::new(automaton, false)?,
        };
        Ok(tokenizer_components(own(tokenizer)))
    })
    .init_reader(move |_, reader| {
        let filter = match buffer_size {
            Some(size) => ICUNormalizer2CharFilter::with_buffer_size(
                reader,
                Arc::clone(&normalizer),
                size as usize,
            ),
            None => ICUNormalizer2CharFilter::with_normalizer(reader, Arc::clone(&normalizer)),
        };
        Box::new(filter.expect("ICUNormalizer2CharFilter"))
    })
    .arc()
}

fn do_test_mode(
    normalizer: Arc<dyn Normalizer2>,
    max_length: i32,
    iterations: i32,
    buffer_size: i32,
) -> Result<()> {
    let a = char_filter_analyzer(
        "org.apache.lucene.analysis.icu.TestICUNormalizer2CharFilter$1",
        true,
        Some((MAX_TERM_LENGTH / 2) as i32),
        Arc::clone(&normalizer),
        Some(buffer_size),
    );

    for _ in 0..iterations {
        let input = TestUtil::random_unicode_string_max(&*random(), max_length);
        if input.is_empty() {
            continue;
        }
        let normalized = s(&normalize(&*normalizer, &u(&input)));
        if normalized.is_empty() {
            continue; // MockTokenizer doesnt tokenize empty string...
        }
        B::check_one_term(&*a, &input, &normalized)?;
    }
    a.close();
    Ok(())
}

fn ncf_test_nfc() -> Result<()> {
    do_test_mode(norm("nfc", Mode::Compose), 20, mult() * 1000, 128)
}

fn ncf_test_nfc_huge() -> Result<()> {
    do_test_mode(norm("nfc", Mode::Compose), 256, mult() * 500, 16)
}

fn ncf_test_nfd() -> Result<()> {
    do_test_mode(norm("nfc", Mode::Decompose), 20, mult() * 1000, 128)
}

fn ncf_test_nfd_huge() -> Result<()> {
    do_test_mode(norm("nfc", Mode::Decompose), 256, mult() * 500, 16)
}

fn ncf_test_nfkc() -> Result<()> {
    do_test_mode(norm("nfkc", Mode::Compose), 20, mult() * 1000, 128)
}

fn ncf_test_nfkc_huge() -> Result<()> {
    do_test_mode(norm("nfkc", Mode::Compose), 256, mult() * 500, 16)
}

fn ncf_test_nfkd() -> Result<()> {
    do_test_mode(norm("nfkc", Mode::Decompose), 20, mult() * 1000, 128)
}

fn ncf_test_nfkd_huge() -> Result<()> {
    do_test_mode(norm("nfkc", Mode::Decompose), 256, mult() * 500, 16)
}

fn ncf_test_nfkc_cf() -> Result<()> {
    do_test_mode(norm("nfkc_cf", Mode::Compose), 20, mult() * 1000, 128)
}

fn ncf_test_nfkc_cf_huge() -> Result<()> {
    do_test_mode(norm("nfkc_cf", Mode::Compose), 256, mult() * 500, 16)
}

/// `testRandomStrings*`: the anonymous analyzer `$n` over `normalizer`
/// (`MockTokenizer(WHITESPACE, false, maxTokenLength)` when given).
fn random_strings(
    name: &'static str,
    max_token_length: Option<i32>,
    normalizer: Arc<dyn Normalizer2>,
) -> Result<()> {
    let a = char_filter_analyzer(name, false, max_token_length, normalizer, None);
    B::check_random_data(&*random(), &a, 200 * mult())?;
    // huge strings
    B::check_random_data_max_word_length(&*random(), &a, 25 * mult(), 8192)?;
    a.close();
    Ok(())
}

fn ncf_test_random_strings_nfc() -> Result<()> {
    random_strings(
        "org.apache.lucene.analysis.icu.TestICUNormalizer2CharFilter$2",
        Some((MAX_TERM_LENGTH / 2) as i32),
        norm("nfc", Mode::Compose),
    )
}

fn ncf_test_random_strings_nfkc() -> Result<()> {
    random_strings(
        "org.apache.lucene.analysis.icu.TestICUNormalizer2CharFilter$3",
        Some((MAX_TERM_LENGTH / 2) as i32),
        norm("nfkc", Mode::Compose),
    )
}

fn ncf_test_random_strings_nfkc_cf() -> Result<()> {
    random_strings(
        "org.apache.lucene.analysis.icu.TestICUNormalizer2CharFilter$4",
        Some((MAX_TERM_LENGTH / 2) as i32),
        norm("nfkc_cf", Mode::Compose),
    )
}

fn ncf_test_random_strings_nfd() -> Result<()> {
    random_strings(
        "org.apache.lucene.analysis.icu.TestICUNormalizer2CharFilter$5",
        Some((MAX_TERM_LENGTH / 2) as i32),
        norm("nfc", Mode::Decompose),
    )
}

fn ncf_test_random_strings_nfkd() -> Result<()> {
    random_strings(
        "org.apache.lucene.analysis.icu.TestICUNormalizer2CharFilter$6",
        None,
        norm("nfkc", Mode::Decompose),
    )
}

fn ncf_test_curious_string() -> Result<()> {
    let text = java_literal(&[
        r#"\udb40\udc3d\uf273\ue960\u06c8\ud955\udc13\ub7fc\u0692 \u2089\u207b\u2073\u2075"#,
    ]);
    let a = char_filter_analyzer(
        "org.apache.lucene.analysis.icu.TestICUNormalizer2CharFilter$7",
        false,
        None,
        norm("nfkc_cf", Mode::Compose),
        None,
    );
    for _ in 0..1000 {
        B::check_analysis_consistency(&random(), &*a, false, &text)?;
    }
    a.close();
    Ok(())
}

/// `testCuriousMassiveString`'s text: Java's string literals, decoded by
/// [`java_literal`].
const CURIOUS_MASSIVE_STRING: [&str; 172] = [
    r#"yqt \u0728\u0707\u0712\u0720\u0734 \u204c\u201d hyipy \u2667\u2619"#,
    r#"\u26ec\u267b\u26da uboyjwfbv \u2012\u205d\u2042\u200a\u2047\u2040 gyxmmz yvv %"#,
    r#"\ufb86 \n<script> hupvobbv jvsztd x rww ct{1,5} brteyi dfgyzqbm hdykd ahgeizyhv"#,
    r#" kLn c#\ud8f8\udd74 fPezd ktedq \ufcea=\ud997\uddc9\u876bJ\u0231\ud98a\udce0\uf872"#,
    r#" zquqah \ub9d6\u144e\uc686 3\ud93d\udfca\u1215\ud614 tnorask \u0348\u0334\u0334"#,
    r#"\u0300\u033d geqdeoghh foojebut \ufb52\u227ag\ud9bd\udc3a\u07efK nyantr lksxw fs"#,
    r#"zies ubzqfolksjpgk \u6fa3\ud859\udc36\u0501\ucca0\u0306\u001e\ua756\u052f \ucaf7"#,
    r#"\u0247\u0009\ufdddg \ud83c\udd02\ud83c\uddaf \u05628\u2b49\u02e3\u0718\u0769\u4f1b"#,
    r#"\u0496\u0766\uecaa\ufb44 \u001d \u0006hr\u00f0\ue649\u041a\uda6f\udfa5\uf31b\ue274"#,
    r#" ptgjf  \ud8cc\udf83M\u0013\u04c6i \u205f\u2004\u2032\u2001\u2057\u2066 \u07d0\uacdb"#,
    r#"\u06a5z  pqfxwgbwe \ud1bc\u2eba\u2d45\u02ee\u56df xnujtfs \u1b19\u1b17\u1b39\u1b20"#,
    r#"\u1b69\u1b58\u1b03\u1b6e\u1b73\u1b20 afsl zxlzziqh ahrhckhktf \ud801\udc5b\ud801\udc61"#,
    r#" bkpmeyyqobwi qnkunmpjpihezll plhhws \u37f7\u41d6\u3dca\u3e80\u4923\u36b8 \u195a\u1959"#,
    r#"\u196f\u1968\u1958\u197f </p \u0006s\u019f\uda82\udc90H_\u079d\ufd6f: idpp \u217c"#,
    r#"\u2168\u2185\u2179\u2156\u2159\u217c\u2154\u2158 ({1,5}( jkieylqzmb bfirnaj \uea71"#,
    r#"\uf17f\u0749\u054c \ud9ea\udf83\udbea\ude91j x\u3366\u09c2\ud828\udd13~\u6fda\ueeee"#,
    r#" \ud834\udcd3\ud834\udc2b\ud834\udc8b\ud834\udcd8 dl \ud802\ude3a\ud802\ude36\ud802"#,
    r#"\ude23\ud802\ude56 \u20ad\u20a0\u20a0\u20a0\u20b8\u20b4\u20ad lcql \u0547\u0156]"#,
    r#"\ue344V] \ud90c\udd7d\u000eS\uf965\uf15e\u008f qdn \udac3\ude3c buy m qni \u31a4\u31a3"#,
    r#"\u31b2\u31a9\u31a7 \u20df\u20ee\u20d3\u20d4\u20f1\u20de \u4dc1\u4dc4\u4dde\u4dec\u4df9"#,
    r#"\u4dee\u4dc5 \udb40\udc36 gstfqnfWY \u24ab\u8d69\u0301 n?]v||]i )- \udb0b\ude77\uf634"#,
    r#"\u0762 tpkkjlbcntsk eebtzirw xo hktxy \n</     vxro  xpr mtlp p|tjf|i?.- lxpfo \udbd7"#,
    r#"\udf78\udbf5\udf57 u..b jjj]p?e jtckvhqb \u20eb\u20ea\u20fa\u20ef\u20e1\u20ed\u20eb vvm"#,
    r#"uhbsvyi jx mqwxckf \u00d7 qqzs \u05ae\u022d\udb7c\udfb1\u070b vnhkz egnutyuat \udaa2"#,
    r#"\udf20\ufa45#O\u2b61\u0d0e \u09a2\u0996\u09cd\u09b4 v \udbdb\ude9bqp owsln \ua837\ua833"#,
    r#"\ua83f\ua83f\ua83f\ua831\ua83c\ua839 \u1a15\u1a1c\u1a12 \ud83c\ude20 >&pt&#x>129 \ud9f1"#,
    r#"\udf8c\uecdd \ud809\udc48\ud809\udc72 wswskop \ua70d\ua70e ohcjsboriux \u2025\u2000\u206e"#,
    r#"\u2039\u2036\u2002\u200e \u1741\u1741\u175e bsykot \u0696\ufab5W ifj cnosdrAxa qtv wvrdn"#,
    r#" dmt (){0 udqg Z5U\ufcab\ue9ed\u0378 ts zndsksqtu uxbkAbn \u0535\"\u001b\ud923\udcc3\udae7"#,
    r#"\udccf\u02ac \u0131\ua72e\u1273\u02f8\u2329\u5d83Q icakme oojxv hpec xndjzxre )e{0,5}. "#,
    r#"\u0005!\u0009\u0004\u000bt\u0006N vqxnokp cdshqof \ua654\ua681\ua667\ua676\ua698  vwwp "#,
    r#"\ufe4b\ufe3b\ufe4d\ufe42\ufe44\ufe38\ufe4b\ufe4c\ufe4b\ufe38 \u2ffb\u2ffa\u2ff6\u2ff7"#,
    r#"\u2ffc\u2ff6\u2ff5 \u3116\u312c\u3100\u3102\u310f\u3116 agyueiij \u0764\u1178\ud866\udca1"#,
    r#"\u00f2\u039d \ud802\udc12\ud802\udc1e\ud802\udc28\ud802\udc19 kygiv fxctjyj           \n"#,
    r#"  omu \ud855\udeb1\u063c\ufd54\u9dbf\uf80a\ufc60 \u76ee\u3365\uf998\u70a8\u02d2 \u05e5"#,
    r#"\u0478\u61bb \ua71c\ua708\ua70d\ua71a\ua712\ua712\ua719\ua706\ua71f\ua71c\ua71e\ua718 pgq"#,
    r#"arvtzcoduk pyok \u1779\u1766\u1778\u177b \u16e5\u16a7\u16f3\u16fe\u16c8\u16ba\u16a4 \u0527"#,
    r#"\u052f\u0513\u0500 iisgh hxd \u13dd\u13c6\u13db\u13ee\u13d7 \u0019\ufed4 \ud802\udd3c\ud802"#,
    r#"\udd30\ud802\udd3d\ud802\udd24\ud802\udd2c jm gvylt eax xsbt mvuvnw \u0246\u5514\udb16\uddcf"#,
    r#"\u1dc2\ud07b\u07af\u12e8\u8e8f\u0004 phy haduzzw \u04a1\u8334\u14b5\uf0de\udb4b\udec0\u6b69 "#,
    r#"dubdl \u0cd2\u06c7\uf0297\u45efy-\u05e9\u01a3\uf46f aiafsh   &# \u0003\ue5ab\uedcd] xhz vil"#,
    r#"wdlytsj \uda10\udf4f\u87b2 tomuca \u1fe4\u1f4c\u1fab \u035d\u0332 xgeel nzp -)r)]r([{ nbc "#,
    r#"\u01b4\ud80f\udff5\u0008\u0091 tbugdgv \ud9cc\udd57\u0776t\uda0f\udc17 rsuwxqxm \u2d71\u2d3d"#,
    r#"\u2d6e zsvuicin \udb50\ude9d\u7424\u30c7\uff73\ue11e\u0014 qxtxx dlssfvxg ud \u0c1f\ud9d9"#,
    r#"\udce4\u2317\u0482\u017b \ud83c\udd91\ud83c\uddaf\ud83c\udd84 \ud834\udf7d\ud834\udf70\ud834"#,
    r#"\udf61 \uabfc\uabe1\uabcd\uabd1\uabe8\uabf9 \u292a\u2916\u295d\u290b\u296d\u291f \uf476\u0283"#,
    r#"\u03d5\ucfe2 h)(o? lqeatt \u20c9\u20a5\u20cd \u1634d\u001a\ua052:\u00db \ud83c\udc8a\ud83c"#,
    r#"\udc41\ud83c\udc8a\ud83c\udc6e k civyjm ptz \uf20d\uea32&\ud8db\ude48\uf657s\u06dc\u9aa5\udbd7"#,
    r#"\udc74\u0111 cehuo \u2090\u209b\u2099\u208c\u209a\u2088\u2092\u207e\u207b\u2089 efpifnvdd zstz"#,
    r#"duuyb \u04af3 \u2e5f\u2e7e\u2e7c\u2e54\u2e0f\u2e68\u2e0d\u2e05 <??>  \u28d3\udbb7\udf6fJ\uf089"#,
    r#"\ub617\ufb80\u07d0\uf141 \u0820\u083b\u0800\u0801\u0832\u0823\u0820\u081c\u0835 r laxzpfbcvz "#,
    r#"iuwbmq scpeqaq nvw{1,5}s) \u234c\u231f\u231e\u23cc\u23d8\u2302\u2345\u231b\u239d\u231e 154614 "#,
    r#"wgefnmgq \udbbe\udc2a\uee8c ayaeg \u243c\u2404\u2409\u241a dd hxuawoswx jqghoznw \u0019\u70cd& "#,
    r#"\ufe0a\ufe02\ufe04\ufe0c\ufe0d \u0510\u0523\u0510\u0501\u0508\u0513\u050e ropgywv guqsrcz kmf "#,
    r#"\u0d1f\u0d5c\u0d24\u0d5f\u0d0b\u0d14 xgklnql oprw \u0365\u034e\u036a\u0310\u034f\u0316\u031f "#,
    r#"\u01b2\u55f6\uf1eeo\ue994\u07c4? wewz idjwsqwo \ufe67\ufe66\ufe52\ufe6a\ufe5b\ufe57\ufe68\ufe62"#,
    r#" \u27fd\u27fe\u27f6\u27f8 fcsqqvoy edrso \u5580\ue897 vantkitto sm \uff60\ud957\udf48\uf919w"#,
    r#"\ud889\udf3e\ud9c8\uddf6 jhas uqzmlr \ua4ce\ua4aa\ua4b3\ua4b5\ua4c2\ua4a5 kvuop ><script> "#,
    r#"\ud802\udd0f\ud802\udd15\ud802\udd11 \u16bc gjyabb mlaewr \u1175\u11cc\u1192 ri ]\u01eb\ue4ca"#,
    r#"  \uedca\ucd66\u597c\u03df\uaf8f\u0619 &#x; ]?e \ua6c2\ua6ed\ua6eb\ua6ea\ua6cd\ua6e2 gfpafsr"#,
    r#" pooo \u20cc\u20c4\u20a7\u20c8\u20a6\u20b0\u20ad \udb40\udd5b tgcdmeuau \u141f\u637d \ufba8"#,
    r#"\ufcc7\ufde1\ufc85\ufbfc\ufbed b \u319a\u3193\u3192\u3195\u319e\u319d\u3195\u3194\u3197\u319c "#,
    r#"\u0514\u0503\u051c\u050e\u0502\u0520 \u1db3\u1db5\u1d96 \ud977\udde8w\u02ec&\u01dd\u29ed\ufead"#,
    r#"y\u03e3 ukzqq {0,5}{0 \u000f\uf028$\u046f7\udb7e\uded2 <p><l \uea5e(\uf1dcX\u376b ([mi[n(a Jev"#,
    r#"xsJl \ubd09\u04c1\ua0f3\uff7b \u1cd3\u1cd9\u1cf6\u1cf0\u1cd8\u1cdd\u1cdd\u1ce7\u1ce3\u1cd9 "#,
    r#"\ud800\udf91\ud800\udf99\ud800\udf83 \"135541 \u18b3\u18c0\u18c2\u18ea\u18c4\u18fe\u18b2\u18fd"#,
    r#"\u18c3 uwykvfd lqiflow afdfctcz ol[hemp strmhxmk \ua732\ua748\ua731\ua791\ua78b\ua7ee\ua7ea"#,
    r#"\ua789\ua766\ua7e4 gmzpmrpzr dqfp  wfxwjzepdj M\udb03\udeff\u13c5 afsncdwp \ue716\u0734\ud8f9"#,
    r#"\udc986\u0017\u0211 t r vhczf (]|q.{0  \u195e\u1973\u1958\u1956\u196c\u1973\u1966\u196c\u197a"#,
    r#"  \u2595\u258e\u259a\u2591\u2583\u2595 l kgopyql wes E\u6611\ub713\u7058^ bipq dx 7507 \ua90b"#,
    r#"\ua90b ktjeqqx \u0e1d\u0e7f\u0e35 #(\u71b7'\u06e5\u03e4(\uf714\u6ff2 advbgh \u319c\u3191 \uef11"#,
    r#"% \uc8a7C\ud8ed\udf4c rjb \u02ca\uf5bd\ue379n \ud834\udd7d\ud834\udd83 jracjcd rpearfd ujzvdc"#,
    r#" ofg \u09df\u09f4\u0980\u09b3\u09bf\u09b7 \ud9cc\uddf4$\udb08\udf72 iqcnwsyjmo </scri ]-q jsit"#,
    r#"gjg naeajiix vvmq dnlihau o cgyp tqsfe uixlzmhz sixhftpr uvtbvv mphcWojZs \u190b\ud9c3\ude7c"#,
    r#"\u008b\u0606\ua8b1 a  \u000ebq# \u1e57\u0b66\uda41\ude32\ubfd6 ohph b- ze \ue2a9\u0000 zatuye"#,
    r#" \ufd26\ufdfa\ufbbf\ufdb4\ufde3\ufd14\ufc25\ufcb8 sbtpb  nxtacgjo \ud834\ude2a\ud834\ude0f"#,
    r#"\ud834\ude14\ud834\ude27 \ua835\ua835 ujpjkkyhujx \u001e\ud9a7\udc45\u0011>\u1aef\u0d0d <"#,
    r#" hcefg \u01f0\u01d3 gxlwv \ud802\udd2f\ud802\udd34 \udb9c\udcc8\udbb6\ude1e\udbaf\ude33\udbae"#,
    r#"\udc49 xyzlzynd \ud83c\udd44 vynhdwh \u315d\u3157\u314d\u3180\u317d\u318d\u317d\u3156 ?>\"  "#,
    r#"\ud800\udfdb\ud800\udfb8\ud800\udfa6\ud800\udfa7 hszn sspwldpdwjfhk vrbag \ueabd\ud9f2\udfb5"#,
    r#"\udafb\udd28\uf6a4y\ufdeb \u0798\u078f\u0789 \ue80c\uf5c1\u001a\ud84b\uddef ywdrudo odqsts nnp"#,
    r#"vfl nvotk rdwhr rquj cn \ud7d4\ud7b3\ud7c7\ud7bf\ud7bd &#x528f28 emdq pnqsbbgzs \u2c7d\u2c7e fj"#,
    r#"kwhku >&c \ud800\udf85\ud800\udf88\ud800\udf93\ud800\udf84\ud800\udf82\ud800\udf8b '\n\"<p></p>"#,
    r#" xltgxaa vqjmhiz n .m]c]tr( qerokel lc kugmimmtfxi         <?&#x524963 [g)|n|[ cij </ BwbZfg "#,
    r#"pta bmhn \uf60dz\u54ca fwdp gnkz \u030ba\ue115}\udb7d\ude86\ud813\udc50\uedb9 \u1745\u1749\u174d"#,
    r#"\u1747\u174b\u174f \ud802\udf09\ud802\udf3f\ud802\udf03\ud802\udf00 agmj \u1db7 \u60669\u000f"#,
    r#"j\u000f\u02e4e\u05f5f   p \udaac\udc9f\uc257\u6e58U zcwsy \u19a7\u19cf\u19aa\u199f\u19b7 zhll"#,
    r#" sbmv .uud \u040c\u2e78\ud9fc\udd0d\ufb7f\uf2e1\u04bf vqkyxua \ud834\udf5e\ud834\udf45\ud834"#,
    r#"\udf23 \ud834\uddbe\ud834\udd9b\ud834\uddc4 f [{0,5}t ovbsy tcj nyalqyabn yzhes mlwpw \ud835"#,
    r#"\uddd5\ud835\udfdf\ud835\uddb4\ud835\ude3e epqfkk cssusatbbq \u1424\u1413\u1652\u15f0 dtfy zN"#,
    r#" \u2417\u2435\u2407 qtz \u2fff\u2ff1\u2ff8\u2ff8\u2ff7\u2ff7 \ud802\ude43 gfpe stimmb ugPwlr "#,
    r#"\u0737\u0712\u0712\u071c \u21de \u01d8\u079e\u8215\ue5b9\u07ef\u3cff\u0478 \u05dd\u05e5 gwqset"#,
    r#"iyyh jbncv \u68ba\u52aa) kcoeiuogjsh n[nh- \uf5ff\u7ec3Z zrptjltqstzvx ts tbod &#88087 \u07fd"#,
    r#"\u07c1\u07c0\u07e9\u07fa\u07f2\u07e3\u07e8\u07cb\u07eb\u07d8 fisslh \ue40f\u012b\u02cf\u0766"#,
    r#" \u1c25\u1c4f\u1c1d\u1c20 \"--> dPhwz \ud808\udef9\ud808\udf4a\ud808\uddd3 cicqhk D\ue7d3=\u5228"#,
    r#"\udbc3\udd18\ueb0d\u0012\u0744\ufb04U\u001e\uf388c\u0306 \u2c08\u2c1e Xxzcd \u001d\u0230\u45e8"#,
    r#"\u0653 <zlx \u1e8f\u1e28\u1e3c\u1e8d\u1ee8\u1e69 zcoeork d gusesc \ud802\udd3e nyr vkfrgi \u2cde"#,
    r#" mo nklnsqweh <script gtoppwsbik vrcdbsx  pz \udb0d\ude0c|\u93d0\uf241\u28a8\u0531 \ud83c"#,
    r#"\udc2b\ud83c\udc10 \ud800\udf91\ud800\udf8e qwmmkty \u19f7\u19f7\u19e8\u19e0\u19f9\u19f6\u19e6"#,
    r#" \u7a60\u7b7b\u878c\u603c\u53c6\u6552\u6dfe \u0776\u0775 foxaqi m cdjd guyqxodsccy \ucd7d\ub5eb"#,
    r#"\ud29e\ub9ad\uba00\uac9d\ud2f0 pxjtj \ue362\u079fx\uf193 ){1,5}[{ hmpujw \u3193\u319b\u3195"#,
    r#"\u319c\u3198\u3193\u3195\u319d\u319e \udb40\udd65\udb40\udd29\udb40\udd5c\udb40\uddba \ud801"#,
    r#"\udc18\ud801\udc24\ud801\udc4f\ud801\udc15\ud801\udc04 \u1970\u196c\u1963\u196f\u1979 vjpeg "#,
    r#"\ufeb9 lhoeh &#x540b Szu \u0c21\u0c21\u0c36\u0c0e oyb \u1c7c\ue0ba\u001e gskf \ud826\udd47"#,
    r#"\u0018 ooxki \u001d\u5b0d \uf0e2\u05ba\u000e\u6200 \u2ecc\u2e8a\u2eb8\u2ee5\u2edb\u2ee4\u2ec7"#,
    r#"\u2ef9\u2e9e\u2e99 xpyijb bqvtjfjrxkjv sruyitq jobm u \u0752\u075d\u0778\u0754\u075c \ua4af"#,
    r#"\ua4a5\ua4be\ua4a6\ua4b9\ua4b9 \ua835\ua832\ua838\ua83d \ud83c\udc3a\ud83c\udc9f\ud83c\udc4e"#,
    r#" \ud7fb\ud7ce\ud7c6\ud7f8 erkzdwed ftzpcx ecbylf geptanc jxbhc ophh wqng \ue48c\u9c86Z imkj"#,
    r#"nwetjbz njivpvo  \u6d9a\ud8da\udcba \u29f4\u29fd\u29a6\u2980\u2989\u29f3\u29ec\u2991\u29e5"#,
    r#"\u29c6 \udb08\ude9d\u2ecb\u037e chmap <!--<sCrip \ud83c\udc34\ud83c\udc79 SoOq l botgy \ud83c"#,
    r#"\udc11\ud83c\udc2e\ud83c\udc10 -)h?|] \ud801\udc2e\ud801\udc47 pjlrcjij lpdft v.o| qolrd b  "#,
    r#"uefppzjyvva D\u05de\u0425\u01a3\ue1c0f\uf117\ue266\u0268\u03ec ynik  \udae4\udc38\udba0\udd4c"#,
    r#" M\ue775\u000f \u3170\u3171\u3185\u3140\u3165\u317f \u07f6\u4250vp\u001c\u07a9\uba71 myfawjt"#,
    r#"wvc cqsc o uxabwm \ua9b0\ua9d3  \u0293P\u4dde\u034e \udbe7\udd0b\udbce\udf4d  a\udb4a\ude26x"#,
    r#"\u0bc5\u0355 xtwyyxkj uiwqzvdn \u00c4\u00f4\u00b9\u00f3\u00e3 svfczoqm fx \ua65a\ua696\ua675"#,
    r#"\ua651\ua661\ua686\ua644 cohpzoprr \u000f\ud9d5\udcbd\ud8fa\udc16\ub733\ud8d9\udcf7\uefe9"#,
    r#"\u02da wssqee tikod iwdo cuoi  mvkvd \ud834\udcb7\ud834\udc52\ud834\udc37\ud834\udc30 yqmvakwq"#,
    r#" bwgy ndp \u0b53\u0b75\u0b60\u0b25\u0b1d\u0b1b\u0b19\u0b62 <pmg  cqponju tiwlftbdn jdmc <?"#,
    r#"</p waesof \u3fea\u43bd\u3524\u3d5b \uf87f\u03ab\\\u0727?\uf145 vwznkqkz \ud83c\ude6c\ud83c"#,
    r#"\udea7\ud83c\udedd powof \u94f3\u0392\ue4b5$ \u079f\u07b5\u0781\u07ba\u0786\u07ae\u0782\u0785"#,
    r#" \ud83c\udecc\ud83c\ude8e\ud83c\udea1\ud83c\ude74 \u2afb\u2a2c\u2ae8\u2ab5\u2af4 x\u4c6f hlb"#,
    r#" oirm \ud83c\udc0f\ud83c\udc19 abzbdmkzc qsvvzzc \uf14b \udb53\ude89\u04d2\u53fe\ueb79 uuexkn"#,
    r#" nyeevfme \ue210\uea3e zdwk licyc { cik o].o{1,5 \ua9d1\ua984\ua997\ua99d\ua9a2\ua9b3\ua986"#,
    r#"\ua9d7 \u13ea\u13fb\u13b8\u13b9\u13db\u13e2\u13cf\u13c3\u13c8\u13cc\u13bc \ueeae\u3c1c\uf043"#,
    r#"\u3302   \ufb791\u0009\uc0b7\u039cWG\u4f35\u000f\uf28c \ueb24\udb18\uddef\ufb2c n-jr wsmhtbq "#,
    r#"\ue76b\ud894\udec7\u37f8 box \u1726\u1720\u172b\u173c\u1727 gqcrnrhxcj \u09f8 rof \ua7fa"#,
    r#"\ua7a1 \u07ef\u07f3\u07e2\u07e0\u07d7 udhuv gadgqab({1 \u2c52\u2c30\u2c17\u2c16 P\u33f9\u06da"#,
    r#"\u284b\u0734\u0799 \u289a\u28a1\u28f0\u2853\u282a\u284b\u2859\u2881\u283c qmkopt qbvxnls \ud9c6"#,
    r#"\udc11Z\u7c39\u24ec\u0353 \u069c dfdgt \ue967\u0493\uf460 )yv-|. nl qvmu x{1,5} \\'\\'  ' \u0969"#,
    r#"\u0926\u0943\u0966\u0931\u0970\u094d\u0911\u0970 phiyfw ;\\'\\ zngsxit \u07ec&\ud914\udd55\u9ab7"#,
    r#" ?[| b \ufffc\uffff\ufffb\ufff3\ufff7\ufff8\ufff8\ufffb\ufff5\ufff9\ufffd \u2ffd\u2ff2\u2ff1"#,
    r#"\u2ff9\u2ff6\u2ff1\u2ff8\u2ff1\u2ff8 \ua73d\ua793\ua7d1\ua7cf \u258d\u2599\u259e\u258e\u258d"#,
    r#"\u259f \u001fha f \u0b2e\u0b37\u0b71\u0b44\u0b40\u0b2b \uf1909\ud033 ofkl tbuxhs \ufb06\ufb47"#,
    r#" rqcr \u043d\uf8cf\u001c \ud87e\ude05\ud87e\ude0d\ud87e\udd99\ud87e\udcc0 qqzqzntfby \u0a3f"#,
    r#"\u0a0e\u0a16 \ud7b8\ud7cd\ud7c7\ud7cc\ud7ca\ud7e8\ud7f9\ud7b3\ud7df arzxo \u0f23\u0f2b\u0f68"#,
    r#"\u0f1c\u0fe8\u0f97\u0f27\u0fbd 190854825 \ua915\ua907\ua902\ua902\ua907 \ufbbb\ufdd1\ufbdb"#,
    r#"\ufbed\ufbbb\ufd81\ufd41\ufc3a rrxnh \u0ead\u0ebb\u0e97\u0eff\u0eed\u0e94\u0e86 \ud8c0\udd29"#,
    r#"\u0016\ue050\uebf0;\u79c0\u07ba\uf8ed b \u0122\u0012\udaf5\udcfb+  mkt dluv \u18db\u18d4\u18ea"#,
    r#" \uee53\ueb89\u0679 \u24c2\u24ee\u24e5\u24ab\u24e1\u2460  \ub41eq \uf1e0Tu\u0018\ue5b5 cqf"#,
    r#"pwjzw  cadaxx \u2642\u26be\u2603\u26aa\u26b0 pybwrfqbzr wgyejg cbhzl ipagqw \ud841\udd0d"#,
    r#"\ud84a\udc42\ud84b\udf94\ud85e\udf91\ud847\udd41 fgjm lhtmoqls \u2cc1\u076af >\u034e\ud8a7"#,
    r#"\udd17U\uffcf \u42cb\u07d6\u1d08Y\u0570 o\u016c] .ed]l.u oxms :\uf3cc\u0f67\u0014\u22c6"#,
    r#"\u0720E \u1fef\u1f6f\u1f6a <scri \u63fb\u0508d\ueb2a\u001d\ue3f5\ue915\ud33d \ud800\udf43"#,
    r#"\ud800\udf43\ud800\udf4c\ud800\udf46 \ud802\udc3c\ud802\udc00 ktdajettd urkyb \u040e\uaacf"#,
    r#"\ufd7f\uf130\u048f\u80a6g\u0277\u0013\u8981\uc35d xckws icodgomr \udbf2\ude88\u9e5f o "#,
    r#"h{0,5}x cu oqtvvb ohbuuew ggh 0\u001d=\u8479\ufc33\ue941\ue518  \uff87\u0012\u0226\u743d"#,
    r#"\uef94e\ue0e2\u05cc \ue261\u0015\uf9dc\u8233\u0305/\u111e3\udbb7\udcb5 mxgbvl \uf20f\ud860"#,
    r#"\udc00\uf9f2\uecd2 fl \u03d1\u1664\u5e1d\u619b\uda19\udfe0v/ \ud863\udfa2U\ue0c1\u07f1"#,
    r#"\ue071\udb8f\udeb6 miz \u0641\udb66\udce0' >\ud9c0\udfaf\u07b3J\uf240\ud863\udff8\u01bf"#,
    r#"\u2257\u008b\u0186\u0006 \uaa90\uaa92\uaa9a\uaad6\uaaa7\uaac1\uaa9d\uaaa0\uaaab vorp \u1380"#,
    r#"\u1392\u139e\u138b\u1390\u1386 \uf626\uda58\uddb3\u0014 qrzl fzrb rllb \uc5e5\uf606\u0749"#,
    r#"\ufff8\ud88a\udec12\ud97e\udee4 zscmiqtr \u01be\n \u05f2\u05a0\u05ca\u05de\u059d\u05ac  "#,
    r#"\u2e21\u2e62\u2e72 \u0293 \ufff0\ufff3\ufff8\uffff\ufff2 grtuzvl \ua8bc\ua880\ua89a kprmh "#,
    r#"\ud802\ude51\ud802\ude2e\ud802\ude09\ud802\ude15 cwauqnjs Ou \u31c9\u31dc\u31e4\u31d1\u31e5"#,
    r#"\u31c1\u31d1\u31ce\u31c8 \u31f6\u31fd\u31f0\u31fa\u31f0\u31f2\u31f3\u31f9 wpyi  awanm "#,
    r#"irnupwe &#x7e345 vzwzc qhupnxm qbaboo gtxfdycm vnvaptcc \u0356\ud93f\udf7a {0,5})[k oxnum"#,
    r#"pqyhkg \ufc2c\u0213\ue36e\u0017\ud98b\udc43 \u27f3\u27f7\u27ff\u27ff\u27f5\u27ff\u27f1 hm"#,
    r#"kmwm j{1,5} \u0293\u0262\u2c6d\u0278\u0267\u2c64\u02a8\u0296\u0284 thjrit \u12e3 \ud808"#,
    r#"\udf7d\ud808\udca7 b prkjpdeavdooly \"\\\u06d5\ud9dc\uddb6;\ufdd6\u05bd\u077f kyql \u2d2e"#,
    r#"\u2d04\u2d2e\u2d2a\u2d03\u2d1d scjl higfadu \u3041\u306c\u3073\u305c\u308a\u308e\u3075"#,
    r#"\u3086 akfmdqrrwkw rfmqcxyekf \ud86c\udd70\ud86c\udcdc\ud86b\udea2 c< cwab t \ud800\udd13"#,
    r#"\ud800\udd23 \u0138\ud817\uddcd\uf9f2 zisakv \uea3e\u0314\u07be\ufe67b\ud38b\u0439\r "#,
    r#"\ua766\ua7c5\ua769\ua7a8\ua794 ksfyrg ({1,5}j)?wl \ua94a\ua943\ua932\ua939\ua946\ua95c"#,
    r#"\ua955\ua952\ua958\ua94c pshdyg lhxpypaug blqtrdsvc wycmf ndrzh ekvdkm bnnceq napak n Ko"#,
    r#"KomfgoU \ud83c\uded0\ud83c\udeee \n-->169 mopdhsp \uda82\udca1\\T\udb22\udea8\ufa82C\""#,
    r#"\u06d9\u0218 \u8708 \u18cd\u18c0\u18e8\u18fc\u18be\u18fd\u18c0 yldjbofabwj \u1720\u1739"#,
    r#"\u1729 ([[m{1,5} blqaoru pvsvfall  ydsz \ufd6f\ufce2\ufd4d\ufd07\ufde5\ufddc\ufb6c\ufbc9"#,
    r#"\ufd14\ufc4f\ufd05 \u216b\u218a\u2152\u2172\u217d\u2181\u2188 savpwhs {1,5}f[ha-y[) xnzz "#,
    r#"gksck \u783a\u517a\u513e\u7355\u8741 kicgsn \u3117\u311c\u3104\u310c\u312e\u3104\u3103 "#,
    r#"\u0291\u430b\uc9bfd\ue6e1\uf2d6~0 \ud802\udd38 \ub2cd\uca67\u1c0d\u034c\uf3e2 \u03a2\u0009"#,
    r#"\uda96\udfde \u0010\ufb41\u06dd\u06d0\ue4ef\u241b \ue1a3d\ub55d=\ud8fd\udd54\ueb5f\ud844"#,
    r#"\udf25 xnygolayn txnlsggei yhn \u0e5c\u0e02 \\ fornos oe epp "#,
];

/// A Java string literal's text (escapes `\uXXXX`, `\n`, `\r`, `\"`, `\\`)
/// as the string it denotes (UTF-16 code units, then a Rust string).
fn java_literal(parts: &[&str]) -> String {
    let mut units: Vec<u16> = Vec::new();
    for part in parts {
        let mut chars = part.chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                let mut buf = [0u16; 2];
                units.extend_from_slice(c.encode_utf16(&mut buf));
                continue;
            }
            match chars.next().expect("an escape") {
                'u' => {
                    let hex: String = chars.by_ref().take(4).collect();
                    units.push(u16::from_str_radix(&hex, 16).expect("hex digits"));
                }
                'n' => units.push(u16::from(b'\n')),
                'r' => units.push(u16::from(b'\r')),
                '"' => units.push(u16::from(b'"')),
                '\\' => units.push(u16::from(b'\\')),
                other => panic!("unexpected escape {other}"),
            }
        }
    }
    String::from_utf16(&units).expect("well-formed UTF-16")
}

fn ncf_test_curious_massive_string() -> Result<()> {
    let a = char_filter_analyzer(
        "org.apache.lucene.analysis.icu.TestICUNormalizer2CharFilter$8",
        false,
        None,
        norm("nfkc_cf", Mode::Compose),
        None,
    );
    let text = java_literal(&CURIOUS_MASSIVE_STRING);
    for _ in 0..25 {
        B::check_analysis_consistency(&random(), &*a, false, &text)?;
    }
    a.close();
    Ok(())
}

// https://issues.apache.org/jira/browse/LUCENE-7956
fn ncf_test_very_large_input_of_non_inert_chars() -> Result<()> {
    let text = "a".repeat(1_000_000);
    let normalizer = norm("nfkc_cf", Mode::Compose);
    let a = FnAnalyzer::new(
        "org.apache.lucene.analysis.icu.TestICUNormalizer2CharFilter$9",
        |_| Ok(tokenizer_components(own(new_keyword_tokenizer()))),
    )
    .init_reader(move |_, reader| {
        Box::new(
            ICUNormalizer2CharFilter::with_normalizer(reader, Arc::clone(&normalizer))
                .expect("ICUNormalizer2CharFilter"),
        )
    })
    .arc();
    let result = B::check_analysis_consistency(&random(), &*a, false, &text);
    a.close();
    result
}

fn normalizer2_char_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.icu.TestICUNormalizer2CharFilter")
        .method(t("testNormalization", ncf_test_normalization))
        .method(t("testTokenStream", ncf_test_token_stream))
        .method(t("testTokenStream2", ncf_test_token_stream2))
        .method(t(
            "testIncrementalNoInertChars",
            ncf_test_incremental_no_inert_chars,
        ))
        .method(t("testMassiveLigature", ncf_test_massive_ligature))
        .method(t("testNFC", ncf_test_nfc))
        .method(t("testNFCHuge", ncf_test_nfc_huge))
        .method(t("testNFD", ncf_test_nfd))
        .method(t("testNFDHuge", ncf_test_nfd_huge))
        .method(t("testNFKC", ncf_test_nfkc))
        .method(t("testNFKCHuge", ncf_test_nfkc_huge))
        .method(t("testNFKD", ncf_test_nfkd))
        .method(t("testNFKDHuge", ncf_test_nfkd_huge))
        .method(t("testNFKC_CF", ncf_test_nfkc_cf))
        .method(t("testNFKC_CFHuge", ncf_test_nfkc_cf_huge))
        .method(t("testRandomStringsNFC", ncf_test_random_strings_nfc))
        .method(t("testRandomStringsNFKC", ncf_test_random_strings_nfkc))
        .method(t(
            "testRandomStringsNFKC_CF",
            ncf_test_random_strings_nfkc_cf,
        ))
        .method(t("testRandomStringsNFD", ncf_test_random_strings_nfd))
        .method(t("testRandomStringsNFKD", ncf_test_random_strings_nfkd))
        .method(t("testCuriousString", ncf_test_curious_string))
        .method(t(
            "testCuriousMassiveString",
            ncf_test_curious_massive_string,
        ))
        .method(t(
            "testVeryLargeInputOfNonInertChars",
            ncf_test_very_large_input_of_non_inert_chars,
        ))
        .build()
}

// -----------------------------------------------------------------------------
// TestICUNormalizer2CharFilterFactory
// -----------------------------------------------------------------------------

/// Test nfkc_cf defaults
fn ncff_test_defaults() -> Result<()> {
    let factory = ICUNormalizer2CharFilterFactory::new(&mut HashMap::new())?;
    let reader = factory.create(Box::new(StringCharReader::new("This is a Ｔｅｓｔ")))?;
    let mut stream = B::whitespace_mock_tokenizer(reader)?;
    B::assert_token_stream_contents(&mut stream, &["this", "is", "a", "test"])
}

/// Test nfkc form
fn ncff_test_form_argument() -> Result<()> {
    let factory = ICUNormalizer2CharFilterFactory::new(&mut args(&[("form", "nfkc")]))?;
    let reader = factory.create(Box::new(StringCharReader::new("This is a Ｔｅｓｔ")))?;
    let mut stream = B::whitespace_mock_tokenizer(reader)?;
    B::assert_token_stream_contents(&mut stream, &["This", "is", "a", "Test"])
}

/// Test that bogus arguments result in exception
fn ncff_test_bogus_arguments() -> Result<()> {
    let expected = iae_message(|| {
        ICUNormalizer2CharFilterFactory::new(&mut args(&[("bogusArg", "bogusValue")]))
    });
    Assert::assert_true(expected.contains("Unknown parameters"));
    Ok(())
}

fn normalizer2_char_filter_factory_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.icu.TestICUNormalizer2CharFilterFactory")
        .method(t("testDefaults", ncff_test_defaults))
        .method(t("testFormArgument", ncff_test_form_argument))
        .method(t("testBogusArguments", ncff_test_bogus_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// TestICUTransformFilter
// -----------------------------------------------------------------------------

/// `Transliterator.getInstance(id)`.
fn translit(id: &str) -> Arc<dyn Transliterator> {
    transliterator::get_instance(&u(id)).expect("a registered transliterator")
}

/// `Transliterator.createFromRules("test", rules, Transliterator.FORWARD)`.
fn from_rules(rules: &str) -> Arc<dyn Transliterator> {
    transliterator::create_from_rules(&u("test"), &u(rules), FORWARD).expect("valid rules")
}

fn tf_test_basic_functionality() -> Result<()> {
    check_token(translit("Traditional-Simplified"), "簡化字", "简化字")?;
    check_token(translit("Katakana-Hiragana"), "ヒラガナ", "ひらがな")?;
    check_token(translit("Fullwidth-Halfwidth"), "アルアノリウ", "ｱﾙｱﾉﾘｳ")?;
    check_token(
        translit("Any-Latin"),
        "Αλφαβητικός Κατάλογος",
        "Alphabētikós Katálogos",
    )?;
    check_token(
        translit("NFD; [:Nonspacing Mark:] Remove"),
        "Alphabētikós Katálogos",
        "Alphabetikos Katalogos",
    )?;
    check_token(translit("Han-Latin"), "中国", "zhōng guó")
}

fn tf_test_custom_functionality() -> Result<()> {
    let rules = "a > b; b > c;"; // convert a's to b's and b's to c's
    check_token(from_rules(rules), "abacadaba", "bcbcbdbcb")
}

fn tf_test_custom_functionality2() -> Result<()> {
    let rules = "c { a > b; a > d;"; // convert a's to b's and b's to c's
    check_token(from_rules(rules), "caa", "cbd")
}

/// `final KeywordTokenizer input = new KeywordTokenizer();
/// input.setReader(new StringReader("")); new ICUTransformFilter(input, custom);`
fn construct_over_empty_keyword(custom: &Arc<dyn Transliterator>) -> Result<()> {
    let mut input = new_keyword_tokenizer();
    input.set_reader(Box::new(StringCharReader::new("")))?;
    new_icu_transform_filter(boxed(input), Arc::clone(custom))?;
    Ok(())
}

fn tf_test_optimizer() -> Result<()> {
    let rules = "a > b; b > c;"; // convert a's to b's and b's to c's
    let custom = from_rules(rules);
    Assert::assert_true(custom.get_filter().is_none());
    construct_over_empty_keyword(&custom)?;
    Assert::assert_true(
        *custom.get_filter().expect("a filter") == UnicodeSet::from_pattern(&u("[ab]"))?,
    );
    Ok(())
}

fn tf_test_optimizer2() -> Result<()> {
    check_token(
        translit("Traditional-Simplified; CaseFold"),
        "ABCDE",
        "abcde",
    )
}

fn tf_test_optimizer_surrogate() -> Result<()> {
    let rules = "\\U00020087 > x;"; // convert CJK UNIFIED IDEOGRAPH-20087 to an x
    let custom = from_rules(rules);
    Assert::assert_true(custom.get_filter().is_none());
    construct_over_empty_keyword(&custom)?;
    Assert::assert_true(
        *custom.get_filter().expect("a filter") == UnicodeSet::from_pattern(&u("[\\U00020087]"))?,
    );
    Ok(())
}

fn check_token(transform: Arc<dyn Transliterator>, input: &str, expected: &str) -> Result<()> {
    let mut input1 = new_keyword_tokenizer();
    input1.set_reader(Box::new(StringCharReader::new(input)))?;
    let mut ts = new_icu_transform_filter(boxed(input1), transform)?;
    B::assert_token_stream_contents(&mut ts, &[expected])
}

/// blast some random strings through the analyzer
fn tf_test_random_strings() -> Result<()> {
    let transform = translit("Any-Latin");
    let a = whitespace_analyzer(
        "org.apache.lucene.analysis.icu.TestICUTransformFilter$1",
        move |t| Ok(boxed(new_icu_transform_filter(t, Arc::clone(&transform))?)),
    );
    B::check_random_data(&*random(), &a, 200 * mult())?;
    a.close();
    Ok(())
}

fn tf_test_empty_term() -> Result<()> {
    let a = keyword_tokenizer_analyzer(
        "org.apache.lucene.analysis.icu.TestICUTransformFilter$2",
        |t| Ok(boxed(new_icu_transform_filter(t, translit("Any-Latin"))?)),
    );
    B::check_one_term(&*a, "", "")?;
    a.close();
    Ok(())
}

fn transform_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.icu.TestICUTransformFilter")
        .method(t("testBasicFunctionality", tf_test_basic_functionality))
        .method(t("testCustomFunctionality", tf_test_custom_functionality))
        .method(t("testCustomFunctionality2", tf_test_custom_functionality2))
        .method(t("testOptimizer", tf_test_optimizer))
        .method(t("testOptimizer2", tf_test_optimizer2))
        .method(t("testOptimizerSurrogate", tf_test_optimizer_surrogate))
        .method(t("testRandomStrings", tf_test_random_strings))
        .method(t("testEmptyTerm", tf_test_empty_term))
        .build()
}

// -----------------------------------------------------------------------------
// TestICUTransformFilterFactory
// -----------------------------------------------------------------------------

/// ensure the transform is working
fn tff_test() -> Result<()> {
    let factory = ICUTransformFilterFactory::new(&mut args(&[("id", "Traditional-Simplified")]))?;
    let stream = whitespace_stream("簡化字")?;
    let mut stream = factory.create(stream)?;
    B::assert_token_stream_contents(&mut *stream, &["简化字"])
}

/// test forward and reverse direction
fn tff_test_forward_direction() -> Result<()> {
    // forward
    let factory = ICUTransformFilterFactory::new(&mut args(&[("id", "Cyrillic-Latin")]))?;
    let stream = whitespace_stream("Российская Федерация")?;
    let mut stream = factory.create(stream)?;
    B::assert_token_stream_contents(&mut *stream, &["Rossijskaâ", "Federaciâ"])
}

fn tff_test_reverse_direction() -> Result<()> {
    // backward (invokes Latin-Cyrillic)
    let factory = ICUTransformFilterFactory::new(&mut args(&[
        ("id", "Cyrillic-Latin"),
        ("direction", "reverse"),
    ]))?;
    let stream = whitespace_stream("Rossijskaâ Federaciâ")?;
    let mut stream = factory.create(stream)?;
    B::assert_token_stream_contents(&mut *stream, &["Российская", "Федерация"])
}

/// Test that bogus arguments result in exception
fn tff_test_bogus_arguments() -> Result<()> {
    let expected = iae_message(|| {
        ICUTransformFilterFactory::new(&mut args(&[("id", "Null"), ("bogusArg", "bogusValue")]))
    });
    Assert::assert_true(expected.contains("Unknown parameters"));
    Ok(())
}

fn transform_filter_factory_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.icu.TestICUTransformFilterFactory")
        .method(t("test", tff_test))
        .method(t("testForwardDirection", tff_test_forward_direction))
        .method(t("testReverseDirection", tff_test_reverse_direction))
        .method(t("testBogusArguments", tff_test_bogus_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// Suites
// -----------------------------------------------------------------------------

#[test]
fn test_icu_folding_filter() {
    run(folding_filter_class);
}

#[test]
fn test_icu_folding_filter_factory() {
    run(folding_filter_factory_class);
}

#[test]
fn test_icu_normalizer2_filter() {
    run(normalizer2_filter_class);
}

#[test]
fn test_icu_normalizer2_filter_factory() {
    run(normalizer2_filter_factory_class);
}

#[test]
fn test_icu_normalizer2_char_filter() {
    run(normalizer2_char_filter_class);
}

#[test]
fn test_icu_normalizer2_char_filter_factory() {
    run(normalizer2_char_filter_factory_class);
}

#[test]
fn test_icu_transform_filter() {
    run(transform_filter_class);
}

#[test]
fn test_icu_transform_filter_factory() {
    run(transform_filter_factory_class);
}
