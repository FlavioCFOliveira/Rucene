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
//! that rmp task #467 lists, each run as a suite by the ported runner: the
//! `org.apache.lucene.analysis.cjk` classes `TestCJKAnalyzer`,
//! `TestCJKBigramFilter`, `TestCJKBigramFilterFactory`,
//! `TestCJKWidthCharFilter`, `TestCJKWidthCharFilterFactory`,
//! `TestCJKWidthFilter` and `TestCJKWidthFilterFactory`, the
//! `org.apache.lucene.analysis.th` classes `TestThaiAnalyzer` and
//! `TestThaiTokenizerFactory`, and
//! `org.apache.lucene.analysis.synonym.TestSynonymFilterFactory` (it loads
//! `CJKAnalyzer` by class name).
//!
//! `TestSynonymFilterFactory` is `@Deprecated` and exercises the deprecated
//! `SynonymFilter`, as in Lucene: deprecation warnings are allowed here.

#![allow(deprecated)]

mod support;

use std::cell::RefCell;
use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::factories::TokenFilterFactory;
use rucene::analysis::standard::{IDEOGRAPHIC, TOKEN_TYPES};
use rucene::analysis::stop_filter::new_stop_filter;
use rucene::analysis::tokenattributes::{FlagsAttribute, TypeAttribute};
use rucene::analysis::{
    new_standard_tokenizer, Analyzer, CharArraySet, StringCharReader, TokenFilter,
    TokenFilterLogic, TokenStream,
};
use rucene::error::Result;
use rucene::util::extra::Version;
use rucene::util::AttributeSource;
use rucene_analysis_common::analysis::charfilter::mapping_char_filter::MappingCharFilter;
use rucene_analysis_common::analysis::charfilter::normalize_char_map::Builder as NormalizeCharMapBuilder;
use rucene_analysis_common::analysis::cjk::cjk_analyzer::CJKAnalyzer;
use rucene_analysis_common::analysis::cjk::cjk_bigram_filter::{
    new_cjk_bigram_filter, new_cjk_bigram_filter_with_flags, new_cjk_bigram_filter_with_unigrams,
    HAN,
};
use rucene_analysis_common::analysis::cjk::cjk_width_char_filter::CJKWidthCharFilter;
use rucene_analysis_common::analysis::cjk::cjk_width_filter::new_cjk_width_filter;
use rucene_analysis_common::analysis::en::english_analyzer::ENGLISH_STOP_WORDS_SET;
use rucene_analysis_common::analysis::th::thai_analyzer::ThaiAnalyzer;
use rucene_analysis_common::analysis::th::thai_tokenizer::DBBI_AVAILABLE;
use rucene_test_framework::analysis::mock_tokenizer::{MockTokenizer, WHITESPACE};
use rucene_test_framework::analysis::{
    tokenizer_components, BaseTokenStreamFactoryTestCase, BaseTokenStreamTestCase,
};
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::RANDOM_MULTIPLIER;
use rucene_test_framework::util::string_mock_resource_loader::StringMockResourceLoader;
use rucene_test_framework::util::LuceneTestCase;
use support::{
    boxed, btstc_class, factory_class, hook, iae_message, keyword_tokenizer_analyzer, run, t,
    test_class_loader, token_filter_factory, tokenizer_analyzer, whitespace_analyzer, FnAnalyzer,
};

type F = BaseTokenStreamFactoryTestCase;
type B = BaseTokenStreamTestCase;

/// The test package of the CJK classes.
const CJK: &str = "org.apache.lucene.analysis.cjk";

/// The test package of the Thai classes.
const TH: &str = "org.apache.lucene.analysis.th";

/// The test package of `TestSynonymFilterFactory`.
const SYNONYM: &str = "org.apache.lucene.analysis.synonym";

/// `random()`.
fn random() -> Arc<dyn Random> {
    LuceneTestCase::random()
}

/// `RANDOM_MULTIPLIER`.
fn mult() -> i32 {
    *RANDOM_MULTIPLIER
}

/// An analyzer over `new StandardTokenizer()` and the filter chain `chain`.
fn standard_analyzer(
    name: &'static str,
    chain: impl Fn(Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> + Send + Sync + 'static,
) -> Arc<dyn Analyzer> {
    tokenizer_analyzer(name, || Ok(new_standard_tokenizer()), chain)
}

/// `tokenizerFactory(name, keysAndValues...)` of a test class of `package`.
fn tokenizer_factory(
    package: &str,
    name: &str,
    keys_and_values: &[&str],
) -> Result<Arc<dyn rucene::analysis::factories::TokenizerFactory>> {
    F::tokenizer_factory_with_loader(
        name,
        Some(Version::LATEST),
        &test_class_loader(package),
        keys_and_values,
    )
}

// -----------------------------------------------------------------------------
// TestCJKAnalyzer
// -----------------------------------------------------------------------------

thread_local! {
    /// The `analyzer` field `setUp` sets (one per test class using it).
    static ANALYZER: RefCell<Option<Arc<dyn Analyzer>>> = const { RefCell::new(None) };
    /// The `unibiAnalyzer` field of `TestCJKBigramFilter`.
    static UNIBI: RefCell<Option<Arc<dyn Analyzer>>> = const { RefCell::new(None) };
}

fn analyzer() -> Arc<dyn Analyzer> {
    ANALYZER.with(|a| Arc::clone(a.borrow().as_ref().expect("setUp")))
}

fn unibi_analyzer() -> Arc<dyn Analyzer> {
    UNIBI.with(|a| Arc::clone(a.borrow().as_ref().expect("setUp")))
}

fn close_analyzer() {
    if let Some(a) = ANALYZER.with(|a| a.borrow_mut().take()) {
        a.close();
    }
}

fn cjka_set_up() -> Result<()> {
    LuceneTestCase::super_set_up();
    ANALYZER.with(|a| *a.borrow_mut() = Some(Arc::new(CJKAnalyzer::new())));
    Ok(())
}

fn cjka_tear_down() -> Result<()> {
    close_analyzer();
    LuceneTestCase::super_tear_down();
    Ok(())
}

/// `assertAnalyzesTo(a, input, output, startOffsets, endOffsets, types,
/// posIncrements)`.
fn analyzes(
    a: &dyn Analyzer,
    input: &str,
    output: &[&str],
    start: &[i32],
    end: &[i32],
    types: &[&str],
    pos_inc: &[i32],
) -> Result<()> {
    B::assert_analyzes_to_types(
        a,
        input,
        output,
        Some(start),
        Some(end),
        Some(types),
        Some(pos_inc),
    )
}

fn cjka_test_ja1() -> Result<()> {
    analyzes(
        &*analyzer(),
        "一二三四五六七八九十",
        &[
            "一二", "二三", "三四", "四五", "五六", "六七", "七八", "八九", "九十",
        ],
        &[0, 1, 2, 3, 4, 5, 6, 7, 8],
        &[2, 3, 4, 5, 6, 7, 8, 9, 10],
        &["<DOUBLE>"; 9],
        &[1, 1, 1, 1, 1, 1, 1, 1, 1],
    )
}

fn cjka_test_ja2() -> Result<()> {
    analyzes(
        &*analyzer(),
        "一 二三四 五六七八九 十",
        &["一", "二三", "三四", "五六", "六七", "七八", "八九", "十"],
        &[0, 2, 3, 6, 7, 8, 9, 12],
        &[1, 4, 5, 8, 9, 10, 11, 13],
        &[
            "<SINGLE>", "<DOUBLE>", "<DOUBLE>", "<DOUBLE>", "<DOUBLE>", "<DOUBLE>", "<DOUBLE>",
            "<SINGLE>",
        ],
        &[1, 1, 1, 1, 1, 1, 1, 1],
    )
}

fn cjka_test_c() -> Result<()> {
    analyzes(
        &*analyzer(),
        "abc defgh ijklmn opqrstu vwxy z",
        &["abc", "defgh", "ijklmn", "opqrstu", "vwxy", "z"],
        &[0, 4, 10, 17, 25, 30],
        &[3, 9, 16, 24, 29, 31],
        &["<ALPHANUM>"; 6],
        &[1, 1, 1, 1, 1, 1],
    )
}

/// LUCENE-2207: wrong offset calculated by end()
fn cjka_test_final_offset() -> Result<()> {
    let a = analyzer();
    analyzes(&*a, "あい", &["あい"], &[0], &[2], &["<DOUBLE>"], &[1])?;
    analyzes(&*a, "あい   ", &["あい"], &[0], &[2], &["<DOUBLE>"], &[1])?;
    analyzes(&*a, "test", &["test"], &[0], &[4], &["<ALPHANUM>"], &[1])?;
    analyzes(&*a, "test   ", &["test"], &[0], &[4], &["<ALPHANUM>"], &[1])?;
    analyzes(
        &*a,
        "あいtest",
        &["あい", "test"],
        &[0, 2],
        &[2, 6],
        &["<DOUBLE>", "<ALPHANUM>"],
        &[1, 1],
    )?;
    analyzes(
        &*a,
        "testあい    ",
        &["test", "あい"],
        &[0, 4],
        &[4, 6],
        &["<ALPHANUM>", "<DOUBLE>"],
        &[1, 1],
    )
}

fn cjka_test_mix() -> Result<()> {
    analyzes(
        &*analyzer(),
        "あいうえおabcかきくけこ",
        &[
            "あい", "いう", "うえ", "えお", "abc", "かき", "きく", "くけ", "けこ",
        ],
        &[0, 1, 2, 3, 5, 8, 9, 10, 11],
        &[2, 3, 4, 5, 8, 10, 11, 12, 13],
        &[
            "<DOUBLE>",
            "<DOUBLE>",
            "<DOUBLE>",
            "<DOUBLE>",
            "<ALPHANUM>",
            "<DOUBLE>",
            "<DOUBLE>",
            "<DOUBLE>",
            "<DOUBLE>",
        ],
        &[1, 1, 1, 1, 1, 1, 1, 1, 1],
    )
}

fn cjka_test_mix2() -> Result<()> {
    analyzes(
        &*analyzer(),
        "あいうえおabんcかきくけ こ",
        &[
            "あい", "いう", "うえ", "えお", "ab", "ん", "c", "かき", "きく", "くけ", "こ",
        ],
        &[0, 1, 2, 3, 5, 7, 8, 9, 10, 11, 14],
        &[2, 3, 4, 5, 7, 8, 9, 11, 12, 13, 15],
        &[
            "<DOUBLE>",
            "<DOUBLE>",
            "<DOUBLE>",
            "<DOUBLE>",
            "<ALPHANUM>",
            "<SINGLE>",
            "<ALPHANUM>",
            "<DOUBLE>",
            "<DOUBLE>",
            "<DOUBLE>",
            "<SINGLE>",
        ],
        &[1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
    )
}

/// Non-english text (outside of CJK) is treated normally, according to unicode rules
fn cjka_test_non_ideographic() -> Result<()> {
    analyzes(
        &*analyzer(),
        "一 روبرت موير",
        &["一", "روبرت", "موير"],
        &[0, 2, 8],
        &[1, 7, 12],
        &["<SINGLE>", "<ALPHANUM>", "<ALPHANUM>"],
        &[1, 1, 1],
    )
}

/// Same as the above, except with a nonspacing mark to show correctness.
fn cjka_test_non_ideographic_non_letter() -> Result<()> {
    analyzes(
        &*analyzer(),
        "一 رُوبرت موير",
        &["一", "رُوبرت", "موير"],
        &[0, 2, 9],
        &[1, 8, 13],
        &["<SINGLE>", "<ALPHANUM>", "<ALPHANUM>"],
        &[1, 1, 1],
    )
}

fn cjka_test_surrogates() -> Result<()> {
    analyzes(
        &*analyzer(),
        "𩬅艱鍟䇹愯瀛",
        &["𩬅艱", "艱鍟", "鍟䇹", "䇹愯", "愯瀛"],
        &[0, 2, 3, 4, 5],
        &[3, 4, 5, 6, 7],
        &["<DOUBLE>"; 5],
        &[1, 1, 1, 1, 1],
    )
}

fn cjka_test_reusable_token_stream() -> Result<()> {
    cjka_test_mix()?;
    cjka_test_mix2()
}

fn cjka_test_single_char() -> Result<()> {
    analyzes(&*analyzer(), "一", &["一"], &[0], &[1], &["<SINGLE>"], &[1])
}

fn cjka_test_token_stream() -> Result<()> {
    analyzes(
        &*analyzer(),
        "一丁丂",
        &["一丁", "丁丂"],
        &[0, 1],
        &[2, 3],
        &["<DOUBLE>", "<DOUBLE>"],
        &[1, 1],
    )
}

/// test that offsets are correct when mappingcharfilter is previously applied
fn cjka_test_changed_offsets() -> Result<()> {
    let mut builder = NormalizeCharMapBuilder::new();
    builder.add("a", "一二")?;
    builder.add("b", "二三")?;
    let norm = Arc::new(builder.build()?);
    let analyzer = FnAnalyzer::new("org.apache.lucene.analysis.cjk.TestCJKAnalyzer$1", |_| {
        let tokenizer = Arc::new(rucene::util::atomic_ref_cell::AtomicRefCell::new(
            new_standard_tokenizer(),
        ));
        let filter = new_cjk_bigram_filter(rucene_test_framework::analysis::shared(&tokenizer))?;
        Ok(rucene_test_framework::analysis::components(
            tokenizer,
            rucene_test_framework::analysis::sink(filter),
        ))
    })
    .init_reader(move |_, reader| Box::new(MappingCharFilter::new(Arc::clone(&norm), reader)))
    .arc();
    B::assert_analyzes_to_offsets(
        &*analyzer,
        "ab",
        &["一二", "二二", "二三"],
        &[0, 0, 1],
        &[1, 1, 2],
    )?;
    // note: offsets are strange since this is how the charfilter maps them...
    // before bigramming, the 4 tokens look like:
    //   { 0, 0, 1, 1 },
    //   { 0, 1, 1, 2 }
    analyzer.close();
    Ok(())
}

/// `FakeStandardTokenizer`: sets every token's type to
/// `StandardTokenizer.TOKEN_TYPES[StandardTokenizer.IDEOGRAPHIC]` (the same
/// string object, which `CJKBigramFilter` compares by identity).
#[derive(Debug)]
struct FakeStandardTokenizerLogic;

impl TokenFilterLogic for FakeStandardTokenizerLogic {
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if input.increment_token()? {
            source
                .get_attribute_interface_mut::<dyn TypeAttribute>()
                .expect("typeAtt")
                .set_type_static(TOKEN_TYPES[IDEOGRAPHIC]);
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

fn cjka_test_single_char2() -> Result<()> {
    let analyzer = whitespace_analyzer(
        "org.apache.lucene.analysis.cjk.TestCJKAnalyzer$2",
        |tokenizer| {
            let mut filter = TokenFilter::new(tokenizer, FakeStandardTokenizerLogic);
            filter
                .attribute_source_mut()
                .add_attribute_interface::<dyn TypeAttribute>()?;
            let filter = new_stop_filter(boxed(filter), CharArraySet::empty_set())?;
            Ok(boxed(new_cjk_bigram_filter(boxed(filter))?))
        },
    );
    analyzes(&*analyzer, "一", &["一"], &[0], &[1], &["<SINGLE>"], &[1])?;
    analyzer.close();
    Ok(())
}

/// blast some random strings through the analyzer
fn cjka_test_random_strings() -> Result<()> {
    let a: Arc<dyn Analyzer> = Arc::new(CJKAnalyzer::new());
    B::check_random_data(&*random(), &a, 200 * mult())?;
    a.close();
    Ok(())
}

/// blast some random strings through the analyzer
fn cjka_test_random_huge_strings() -> Result<()> {
    let a: Arc<dyn Analyzer> = Arc::new(CJKAnalyzer::new());
    B::check_random_data_max_word_length(&*random(), &a, 10 * mult(), 8192)?;
    a.close();
    Ok(())
}

fn cjka_test_empty_term() -> Result<()> {
    let a = keyword_tokenizer_analyzer(
        "org.apache.lucene.analysis.cjk.TestCJKAnalyzer$3",
        |tokenizer| Ok(boxed(new_cjk_bigram_filter(tokenizer)?)),
    );
    B::check_one_term(&*a, "", "")?;
    a.close();
    Ok(())
}

fn cjk_analyzer_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.cjk.TestCJKAnalyzer")
        .method(hook("setUp", cjka_set_up))
        .method(hook("tearDown", cjka_tear_down))
        .method(t("testJa1", cjka_test_ja1))
        .method(t("testJa2", cjka_test_ja2))
        .method(t("testC", cjka_test_c))
        .method(t("testFinalOffset", cjka_test_final_offset))
        .method(t("testMix", cjka_test_mix))
        .method(t("testMix2", cjka_test_mix2))
        .method(t("testNonIdeographic", cjka_test_non_ideographic))
        .method(t(
            "testNonIdeographicNonLetter",
            cjka_test_non_ideographic_non_letter,
        ))
        .method(t("testSurrogates", cjka_test_surrogates))
        .method(t(
            "testReusableTokenStream",
            cjka_test_reusable_token_stream,
        ))
        .method(t("testSingleChar", cjka_test_single_char))
        .method(t("testTokenStream", cjka_test_token_stream))
        .method(t("testChangedOffsets", cjka_test_changed_offsets))
        .method(t("testSingleChar2", cjka_test_single_char2))
        .method(t("testRandomStrings", cjka_test_random_strings))
        .method(t("testRandomHugeStrings", cjka_test_random_huge_strings))
        .method(t("testEmptyTerm", cjka_test_empty_term))
        .build()
}

// -----------------------------------------------------------------------------
// TestCJKBigramFilter
// -----------------------------------------------------------------------------

/// The sentence `testHuge` and `testUnigramsAndBigramsHuge` repeat 11 times.
const SENTENCE: &str = "多くの学生が試験に落ちた";

fn cjkb_set_up() -> Result<()> {
    LuceneTestCase::super_set_up();
    let analyzer = standard_analyzer(
        "org.apache.lucene.analysis.cjk.TestCJKBigramFilter$1",
        |t| Ok(boxed(new_cjk_bigram_filter(t)?)),
    );
    let unibi = standard_analyzer(
        "org.apache.lucene.analysis.cjk.TestCJKBigramFilter$2",
        |t| Ok(boxed(new_cjk_bigram_filter_with_unigrams(t, 0xff, true)?)),
    );
    ANALYZER.with(|a| *a.borrow_mut() = Some(analyzer));
    UNIBI.with(|a| *a.borrow_mut() = Some(unibi));
    Ok(())
}

fn cjkb_tear_down() -> Result<()> {
    close_analyzer();
    if let Some(a) = UNIBI.with(|a| a.borrow_mut().take()) {
        a.close();
    }
    LuceneTestCase::super_tear_down();
    Ok(())
}

/// The bigrams of `text` (every pair of adjacent characters), which the
/// Java test lists literally for `SENTENCE` × 11.
fn bigrams(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    chars.windows(2).map(|w| w.iter().collect()).collect()
}

/// Each character of `text` followed by its bigram with the next one, which
/// the Java test lists literally for `SENTENCE` × 11.
fn unigrams_and_bigrams(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    for i in 0..chars.len() {
        out.push(chars[i].to_string());
        if i + 1 < chars.len() {
            out.push(chars[i..i + 2].iter().collect());
        }
    }
    out
}

fn as_strs(v: &[String]) -> Vec<&str> {
    v.iter().map(String::as_str).collect()
}

fn cjkb_test_huge() -> Result<()> {
    let input = SENTENCE.repeat(11);
    let expected = bigrams(&input);
    assert_eq!(131, expected.len());
    B::assert_analyzes_to(&*analyzer(), &input, &as_strs(&expected))
}

fn cjkb_test_han_only() -> Result<()> {
    let a = standard_analyzer(
        "org.apache.lucene.analysis.cjk.TestCJKBigramFilter$3",
        |t| Ok(boxed(new_cjk_bigram_filter_with_flags(t, HAN)?)),
    );
    B::assert_analyzes_to_pos_lengths(
        &*a,
        "多くの学生が試験に落ちた。",
        &[
            "多", "く", "の", "学生", "が", "試験", "に", "落", "ち", "た",
        ],
        Some(&[0, 1, 2, 3, 5, 6, 8, 9, 10, 11]),
        Some(&[1, 2, 3, 5, 6, 8, 9, 10, 11, 12]),
        Some(&[
            "<SINGLE>",
            "<HIRAGANA>",
            "<HIRAGANA>",
            "<DOUBLE>",
            "<HIRAGANA>",
            "<DOUBLE>",
            "<HIRAGANA>",
            "<SINGLE>",
            "<HIRAGANA>",
            "<HIRAGANA>",
            "<SINGLE>",
        ]),
        Some(&[1, 1, 1, 1, 1, 1, 1, 1, 1, 1]),
        Some(&[1, 1, 1, 1, 1, 1, 1, 1, 1, 1]),
    )?;
    a.close();
    Ok(())
}

fn cjkb_test_all_scripts() -> Result<()> {
    let a = standard_analyzer(
        "org.apache.lucene.analysis.cjk.TestCJKBigramFilter$4",
        |t| Ok(boxed(new_cjk_bigram_filter_with_unigrams(t, 0xff, false)?)),
    );
    B::assert_analyzes_to(
        &*a,
        "多くの学生が試験に落ちた。",
        &[
            "多く", "くの", "の学", "学生", "生が", "が試", "試験", "験に", "に落", "落ち", "ちた",
        ],
    )?;
    a.close();
    Ok(())
}

fn cjkb_test_unigrams_and_bigrams_all_scripts() -> Result<()> {
    B::assert_analyzes_to_pos_lengths(
        &*unibi_analyzer(),
        "多くの学生が試験に落ちた。",
        &[
            "多", "多く", "く", "くの", "の", "の学", "学", "学生", "生", "生が", "が", "が試",
            "試", "試験", "験", "験に", "に", "に落", "落", "落ち", "ち", "ちた", "た",
        ],
        Some(&[
            0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11,
        ]),
        Some(&[
            1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12,
        ]),
        Some(&[
            "<SINGLE>", "<DOUBLE>", "<SINGLE>", "<DOUBLE>", "<SINGLE>", "<DOUBLE>", "<SINGLE>",
            "<DOUBLE>", "<SINGLE>", "<DOUBLE>", "<SINGLE>", "<DOUBLE>", "<SINGLE>", "<DOUBLE>",
            "<SINGLE>", "<DOUBLE>", "<SINGLE>", "<DOUBLE>", "<SINGLE>", "<DOUBLE>", "<SINGLE>",
            "<DOUBLE>", "<SINGLE>",
        ]),
        Some(&[
            1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1,
        ]),
        Some(&[
            1, 2, 1, 2, 1, 2, 1, 2, 1, 2, 1, 2, 1, 2, 1, 2, 1, 2, 1, 2, 1, 2, 1,
        ]),
    )
}

fn cjkb_test_unigrams_and_bigrams_han_only() -> Result<()> {
    let a = standard_analyzer(
        "org.apache.lucene.analysis.cjk.TestCJKBigramFilter$5",
        |t| Ok(boxed(new_cjk_bigram_filter_with_unigrams(t, HAN, true)?)),
    );
    B::assert_analyzes_to_pos_lengths(
        &*a,
        "多くの学生が試験に落ちた。",
        &[
            "多", "く", "の", "学", "学生", "生", "が", "試", "試験", "験", "に", "落", "ち", "た",
        ],
        Some(&[0, 1, 2, 3, 3, 4, 5, 6, 6, 7, 8, 9, 10, 11]),
        Some(&[1, 2, 3, 4, 5, 5, 6, 7, 8, 8, 9, 10, 11, 12]),
        Some(&[
            "<SINGLE>",
            "<HIRAGANA>",
            "<HIRAGANA>",
            "<SINGLE>",
            "<DOUBLE>",
            "<SINGLE>",
            "<HIRAGANA>",
            "<SINGLE>",
            "<DOUBLE>",
            "<SINGLE>",
            "<HIRAGANA>",
            "<SINGLE>",
            "<HIRAGANA>",
            "<HIRAGANA>",
            "<SINGLE>",
        ]),
        Some(&[1, 1, 1, 1, 0, 1, 1, 1, 0, 1, 1, 1, 1, 1]),
        Some(&[1, 1, 1, 1, 2, 1, 1, 1, 2, 1, 1, 1, 1, 1]),
    )?;
    a.close();
    Ok(())
}

fn cjkb_test_unigrams_and_bigrams_huge() -> Result<()> {
    let input = SENTENCE.repeat(11);
    let expected = unigrams_and_bigrams(&input);
    assert_eq!(263, expected.len());
    B::assert_analyzes_to(&*unibi_analyzer(), &input, &as_strs(&expected))
}

/// blast some random strings through the analyzer
fn cjkb_test_random_unibi_strings() -> Result<()> {
    B::check_random_data(&*random(), &unibi_analyzer(), 200 * mult())
}

/// blast some random strings through the analyzer
fn cjkb_test_random_unibi_huge_strings() -> Result<()> {
    let random = random();
    B::check_random_data_max_word_length(&*random, &unibi_analyzer(), 10 * mult(), 8192)
}

fn cjk_bigram_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.cjk.TestCJKBigramFilter")
        .method(hook("setUp", cjkb_set_up))
        .method(hook("tearDown", cjkb_tear_down))
        .method(t("testHuge", cjkb_test_huge))
        .method(t("testHanOnly", cjkb_test_han_only))
        .method(t("testAllScripts", cjkb_test_all_scripts))
        .method(t(
            "testUnigramsAndBigramsAllScripts",
            cjkb_test_unigrams_and_bigrams_all_scripts,
        ))
        .method(t(
            "testUnigramsAndBigramsHanOnly",
            cjkb_test_unigrams_and_bigrams_han_only,
        ))
        .method(t(
            "testUnigramsAndBigramsHuge",
            cjkb_test_unigrams_and_bigrams_huge,
        ))
        .method(t("testRandomUnibiStrings", cjkb_test_random_unibi_strings))
        .method(t(
            "testRandomUnibiHugeStrings",
            cjkb_test_random_unibi_huge_strings,
        ))
        .build()
}

// -----------------------------------------------------------------------------
// TestCJKBigramFilterFactory
// -----------------------------------------------------------------------------

/// `tokenizerFactory("standard").create()` over `text`, then the
/// `CJKBigram` filter factory with `keys_and_values`.
fn cjkbf_stream(text: &str, keys_and_values: &[&str]) -> Result<Box<dyn TokenStream>> {
    let mut stream = tokenizer_factory(CJK, "standard", &[])?.create()?;
    stream.set_reader(Box::new(StringCharReader::new(text)))?;
    token_filter_factory(CJK, "CJKBigram", keys_and_values)?.create(boxed(stream))
}

fn cjkbf_test_defaults() -> Result<()> {
    let mut stream = cjkbf_stream("多くの学生が試験に落ちた。", &[])?;
    B::assert_token_stream_contents(
        &mut *stream,
        &[
            "多く", "くの", "の学", "学生", "生が", "が試", "試験", "験に", "に落", "落ち", "ちた",
        ],
    )
}

fn cjkbf_test_han_only() -> Result<()> {
    let mut stream = cjkbf_stream("多くの学生が試験に落ちた。", &["hiragana", "false"])?;
    B::assert_token_stream_contents(
        &mut *stream,
        &[
            "多", "く", "の", "学生", "が", "試験", "に", "落", "ち", "た",
        ],
    )
}

fn cjkbf_test_han_only_unigrams() -> Result<()> {
    let mut stream = cjkbf_stream(
        "多くの学生が試験に落ちた。",
        &["hiragana", "false", "outputUnigrams", "true"],
    )?;
    B::assert_token_stream_contents(
        &mut *stream,
        &[
            "多", "く", "の", "学", "学生", "生", "が", "試", "試験", "験", "に", "落", "ち", "た",
        ],
    )
}

/// Test that bogus arguments result in exception
fn cjkbf_test_bogus_arguments() -> Result<()> {
    expect_throws("java.lang.IllegalArgumentException", || {
        token_filter_factory(CJK, "CJKBigram", &["bogusArg", "bogusValue"])
    });
    Ok(())
}

fn cjk_bigram_filter_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.cjk.TestCJKBigramFilterFactory")
        .method(t("testDefaults", cjkbf_test_defaults))
        .method(t("testHanOnly", cjkbf_test_han_only))
        .method(t("testHanOnlyUnigrams", cjkbf_test_han_only_unigrams))
        .method(t("testBogusArguments", cjkbf_test_bogus_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// TestCJKWidthCharFilter
// -----------------------------------------------------------------------------

/// `whitespaceMockTokenizer(new CJKWidthCharFilter(new StringReader(text)))`.
fn width_char_stream(text: &str) -> Result<Box<dyn TokenStream>> {
    let reader = CJKWidthCharFilter::new(Box::new(StringCharReader::new(text)));
    Ok(boxed(B::whitespace_mock_tokenizer(Box::new(reader))?))
}

/// `assertTokenStreamContents(ts, output, startOffsets, endOffsets,
/// finalOffset)`.
fn assert_offsets_final(
    text: &str,
    output: &[&str],
    start: &[i32],
    end: &[i32],
    final_offset: i32,
) -> Result<()> {
    let mut ts = width_char_stream(text)?;
    B::assert_token_stream_contents_offsets_final_offset(
        &mut *ts,
        output,
        start,
        end,
        Some(final_offset),
    )
}

/// Full-width ASCII forms normalized to half-width (basic latin)
fn cjkwc_test_full_width_ascii() -> Result<()> {
    assert_offsets_final("Ｔｅｓｔ １２３４", &["Test", "1234"], &[0, 5], &[4, 9], 9)
}

/// Half-width katakana forms normalized to standard katakana. A bit trickier
/// in some cases, since half-width forms are decomposed and voice marks need
/// to be recombined with a preceding base form.
fn cjkwc_test_half_width_kana() -> Result<()> {
    assert_offsets_final("ｶﾀｶﾅ", &["カタカナ"], &[0], &[4], 4)?;
    assert_offsets_final("ｳﾞｨｯﾂ", &["ヴィッツ"], &[0], &[5], 5)?;
    assert_offsets_final("ﾊﾟﾅｿﾆｯｸ", &["パナソニック"], &[0], &[7], 7)?;
    assert_offsets_final(
        "ｳﾞｨｯﾂ ﾊﾟﾅｿﾆｯｸ",
        &["ヴィッツ", "パナソニック"],
        &[0, 6],
        &[5, 13],
        13,
    )
}

/// Input may contain orphan voiced marks that cannot be combined with the
/// previous character.
fn cjkwc_test_orphan_voice_mark() -> Result<()> {
    assert_offsets_final("ｱﾞｨｯﾂ", &["ア\u{3099}ィッツ"], &[0], &[5], 5)?;
    assert_offsets_final("ﾞｨｯﾂ", &["\u{3099}ィッツ"], &[0], &[4], 4)?;
    assert_offsets_final("ｱﾟﾅｿﾆｯｸ", &["ア\u{309A}ナソニック"], &[0], &[7], 7)?;
    assert_offsets_final("ﾟﾅｿﾆｯｸ", &["\u{309A}ナソニック"], &[0], &[6], 6)
}

fn cjkwc_test_complex_input() -> Result<()> {
    assert_offsets_final("Ｔｅst １２34", &["Test", "1234"], &[0, 5], &[4, 9], 9)?;
    assert_offsets_final(
        "ｶﾀカナ ｳﾞｨッツ ﾊﾟﾅｿニック",
        &["カタカナ", "ヴィッツ", "パナソニック"],
        &[0, 5, 11],
        &[4, 10, 18],
        18,
    )
}

fn cjkwc_test_empty_input() -> Result<()> {
    let mut ts = width_char_stream("")?;
    B::assert_token_stream_contents(&mut *ts, &[])
}

fn cjkwc_test_random() -> Result<()> {
    let analyzer = FnAnalyzer::new(
        "org.apache.lucene.analysis.cjk.TestCJKWidthCharFilter$1",
        |_| {
            let tokenizer = Arc::new(rucene::util::atomic_ref_cell::AtomicRefCell::new(
                MockTokenizer::new(Arc::clone(&WHITESPACE), false)?,
            ));
            Ok(tokenizer_components(tokenizer))
        },
    )
    .init_reader(|_, reader| Box::new(CJKWidthCharFilter::new(reader)))
    .arc();
    let num_rounds = mult() * 1000;
    B::check_random_data(&*random(), &analyzer, num_rounds)?;
    analyzer.close();
    Ok(())
}

fn cjk_width_char_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.cjk.TestCJKWidthCharFilter")
        .method(t("testFullWidthASCII", cjkwc_test_full_width_ascii))
        .method(t("testHalfWidthKana", cjkwc_test_half_width_kana))
        .method(t("testOrphanVoiceMark", cjkwc_test_orphan_voice_mark))
        .method(t("testComplexInput", cjkwc_test_complex_input))
        .method(t("testEmptyInput", cjkwc_test_empty_input))
        .method(t("testRandom", cjkwc_test_random))
        .build()
}

// -----------------------------------------------------------------------------
// TestCJKWidthCharFilterFactory
// -----------------------------------------------------------------------------

fn cjkwcf_test() -> Result<()> {
    let reader = F::char_filter_factory_with_loader(
        "cjkWidth",
        Some(Version::LATEST),
        &test_class_loader(CJK),
        &[],
    )?
    .create(Box::new(StringCharReader::new("Ｔｅｓｔ １２３４")))?;
    let mut stream = B::whitespace_mock_tokenizer(reader)?;
    B::assert_token_stream_contents(&mut stream, &["Test", "1234"])
}

/// Test that bogus arguments result in exception
fn cjkwcf_test_bogus_arguments() -> Result<()> {
    let expected = iae_message(|| {
        F::char_filter_factory_with_loader(
            "cjkWidth",
            Some(Version::LATEST),
            &test_class_loader(CJK),
            &["bogusArg", "bogusValue"],
        )
    });
    Assert::assert_true(expected.contains("Unknown parameters"));
    Ok(())
}

fn cjk_width_char_filter_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.cjk.TestCJKWidthCharFilterFactory")
        .method(t("test", cjkwcf_test))
        .method(t("testBogusArguments", cjkwcf_test_bogus_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// TestCJKWidthFilter
// -----------------------------------------------------------------------------

fn cjkw_set_up() -> Result<()> {
    LuceneTestCase::super_set_up();
    let analyzer =
        whitespace_analyzer("org.apache.lucene.analysis.cjk.TestCJKWidthFilter$1", |t| {
            Ok(boxed(new_cjk_width_filter(t)?))
        });
    ANALYZER.with(|a| *a.borrow_mut() = Some(analyzer));
    Ok(())
}

fn cjkw_tear_down() -> Result<()> {
    close_analyzer();
    LuceneTestCase::super_tear_down();
    Ok(())
}

/// Full-width ASCII forms normalized to half-width (basic latin)
fn cjkw_test_full_width_ascii() -> Result<()> {
    B::assert_analyzes_to_offsets(
        &*analyzer(),
        "Ｔｅｓｔ １２３４",
        &["Test", "1234"],
        &[0, 5],
        &[4, 9],
    )
}

/// Half-width katakana forms normalized to standard katakana. A bit trickier
/// in some cases, since half-width forms are decomposed and voice marks need
/// to be recombined with a preceding base form.
fn cjkw_test_half_width_kana() -> Result<()> {
    let a = analyzer();
    B::assert_analyzes_to(&*a, "ｶﾀｶﾅ", &["カタカナ"])?;
    B::assert_analyzes_to(&*a, "ｳﾞｨｯﾂ", &["ヴィッツ"])?;
    B::assert_analyzes_to(&*a, "ﾊﾟﾅｿﾆｯｸ", &["パナソニック"])
}

fn cjkw_test_random_data() -> Result<()> {
    B::check_random_data(&*random(), &analyzer(), 200 * mult())
}

fn cjkw_test_empty_term() -> Result<()> {
    let a =
        keyword_tokenizer_analyzer("org.apache.lucene.analysis.cjk.TestCJKWidthFilter$2", |t| {
            Ok(boxed(new_cjk_width_filter(t)?))
        });
    B::check_one_term(&*a, "", "")?;
    a.close();
    Ok(())
}

fn cjk_width_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.cjk.TestCJKWidthFilter")
        .method(hook("setUp", cjkw_set_up))
        .method(hook("tearDown", cjkw_tear_down))
        .method(t("testFullWidthASCII", cjkw_test_full_width_ascii))
        .method(t("testHalfWidthKana", cjkw_test_half_width_kana))
        .method(t("testRandomData", cjkw_test_random_data))
        .method(t("testEmptyTerm", cjkw_test_empty_term))
        .build()
}

// -----------------------------------------------------------------------------
// TestCJKWidthFilterFactory
// -----------------------------------------------------------------------------

fn cjkwf_test() -> Result<()> {
    let stream =
        B::whitespace_mock_tokenizer(Box::new(StringCharReader::new("Ｔｅｓｔ １２３４")))?;
    let mut stream = token_filter_factory(CJK, "CJKWidth", &[])?.create(boxed(stream))?;
    B::assert_token_stream_contents(&mut *stream, &["Test", "1234"])
}

/// Test that bogus arguments result in exception
fn cjkwf_test_bogus_arguments() -> Result<()> {
    let expected =
        iae_message(|| token_filter_factory(CJK, "CJKWidth", &["bogusArg", "bogusValue"]));
    Assert::assert_true(expected.contains("Unknown parameters"));
    Ok(())
}

fn cjk_width_filter_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.cjk.TestCJKWidthFilterFactory")
        .method(t("test", cjkwf_test))
        .method(t("testBogusArguments", cjkwf_test_bogus_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// TestThaiAnalyzer
// -----------------------------------------------------------------------------

fn tha_set_up() -> Result<()> {
    LuceneTestCase::super_set_up();
    LuceneTestCase::assume_true(
        "JRE does not support Thai dictionary-based BreakIterator",
        *DBBI_AVAILABLE,
    );
    Ok(())
}

/// testcase for offsets
fn tha_test_offsets() -> Result<()> {
    let analyzer = ThaiAnalyzer::with_stop_words(CharArraySet::empty_set());
    B::assert_analyzes_to_offsets(
        &analyzer,
        "การที่ได้ต้องแสดงว่างานดี",
        &["การ", "ที่", "ได้", "ต้อง", "แสดง", "ว่า", "งาน", "ดี"],
        &[0, 3, 6, 9, 13, 17, 20, 23],
        &[3, 6, 9, 13, 17, 20, 23, 25],
    )?;
    analyzer.close();
    Ok(())
}

fn tha_test_stop_words() -> Result<()> {
    let analyzer = ThaiAnalyzer::new();
    B::assert_analyzes_to_offsets_pos_increments(
        &analyzer,
        "การที่ได้ต้องแสดงว่างานดี",
        &["แสดง", "งาน", "ดี"],
        &[13, 20, 23],
        &[17, 23, 25],
        &[5, 2, 1],
    )?;
    analyzer.close();
    Ok(())
}

/// Test that position increments are adjusted correctly for stopwords.
// note this test uses stopfilter's stopset
fn tha_test_position_increments() -> Result<()> {
    let analyzer = ThaiAnalyzer::with_stop_words(ENGLISH_STOP_WORDS_SET.clone());
    B::assert_analyzes_to_offsets_pos_increments(
        &analyzer,
        "การที่ได้ต้อง the แสดงว่างานดี",
        &["การ", "ที่", "ได้", "ต้อง", "แสดง", "ว่า", "งาน", "ดี"],
        &[0, 3, 6, 9, 18, 22, 25, 28],
        &[3, 6, 9, 13, 22, 25, 28, 30],
        &[1, 1, 1, 1, 2, 1, 1, 1],
    )?;
    // case that a stopword is adjacent to thai text, with no whitespace
    B::assert_analyzes_to_offsets_pos_increments(
        &analyzer,
        "การที่ได้ต้องthe แสดงว่างานดี",
        &["การ", "ที่", "ได้", "ต้อง", "แสดง", "ว่า", "งาน", "ดี"],
        &[0, 3, 6, 9, 17, 21, 24, 27],
        &[3, 6, 9, 13, 21, 24, 27, 29],
        &[1, 1, 1, 1, 2, 1, 1, 1],
    )?;
    analyzer.close();
    Ok(())
}

fn tha_test_reusable_token_stream() -> Result<()> {
    let analyzer = ThaiAnalyzer::with_stop_words(CharArraySet::empty_set());
    B::assert_analyzes_to(&analyzer, "", &[])?;
    B::assert_analyzes_to(
        &analyzer,
        "การที่ได้ต้องแสดงว่างานดี",
        &["การ", "ที่", "ได้", "ต้อง", "แสดง", "ว่า", "งาน", "ดี"],
    )?;
    B::assert_analyzes_to(
        &analyzer,
        "บริษัทชื่อ XY&Z - คุยกับ xyz@demo.com",
        &["บริษัท", "ชื่อ", "xy", "z", "คุย", "กับ", "xyz", "demo.com"],
    )?;
    analyzer.close();
    Ok(())
}

/// blast some random strings through the analyzer
fn tha_test_random_strings() -> Result<()> {
    let analyzer: Arc<dyn Analyzer> = Arc::new(ThaiAnalyzer::new());
    B::check_random_data(&*random(), &analyzer, 200 * mult())?;
    analyzer.close();
    Ok(())
}

/// blast some random large strings through the analyzer
fn tha_test_random_huge_strings() -> Result<()> {
    let analyzer: Arc<dyn Analyzer> = Arc::new(ThaiAnalyzer::new());
    B::check_random_data_max_word_length(&*random(), &analyzer, 3 * mult(), 8192)?;
    analyzer.close();
    Ok(())
}

// LUCENE-3044
fn tha_test_attribute_reuse() -> Result<()> {
    let analyzer = ThaiAnalyzer::new();
    // just consume
    let ts = analyzer.token_stream("dummy", Box::new(StringCharReader::new("ภาษาไทย")))?;
    B::assert_token_stream_contents(&mut *ts.borrow_mut(), &["ภาษา", "ไทย"])?;
    // this consumer adds flagsAtt, which this analyzer does not use.
    let ts = analyzer.token_stream("dummy", Box::new(StringCharReader::new("ภาษาไทย")))?;
    ts.borrow_mut()
        .attribute_source_mut()
        .add_attribute_interface::<dyn FlagsAttribute>()?;
    B::assert_token_stream_contents(&mut *ts.borrow_mut(), &["ภาษา", "ไทย"])?;
    analyzer.close();
    Ok(())
}

/// test we fold digits to latin-1
fn tha_test_digits() -> Result<()> {
    let a = ThaiAnalyzer::new();
    B::check_one_term(&a, "๑๒๓๔", "1234")?;
    a.close();
    Ok(())
}

fn tha_test_two_sentences() -> Result<()> {
    let analyzer = ThaiAnalyzer::with_stop_words(CharArraySet::empty_set());
    B::assert_analyzes_to_offsets(
        &analyzer,
        "This is a test. การที่ได้ต้องแสดงว่างานดี",
        &[
            "this",
            "is",
            "a",
            "test",
            "การ",
            "ที่",
            "ได้",
            "ต้อง",
            "แสดง",
            "ว่า",
            "งาน",
            "ดี",
        ],
        &[0, 5, 8, 10, 16, 19, 22, 25, 29, 33, 36, 39],
        &[4, 7, 9, 14, 19, 22, 25, 29, 33, 36, 39, 41],
    )?;
    analyzer.close();
    Ok(())
}

fn thai_analyzer_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.th.TestThaiAnalyzer")
        .method(hook("setUp", tha_set_up))
        .method(t("testOffsets", tha_test_offsets))
        .method(t("testStopWords", tha_test_stop_words))
        .method(t("testPositionIncrements", tha_test_position_increments))
        .method(t("testReusableTokenStream", tha_test_reusable_token_stream))
        .method(t("testRandomStrings", tha_test_random_strings))
        .method(t("testRandomHugeStrings", tha_test_random_huge_strings))
        .method(t("testAttributeReuse", tha_test_attribute_reuse))
        .method(t("testDigits", tha_test_digits))
        .method(t("testTwoSentences", tha_test_two_sentences))
        .build()
}

// -----------------------------------------------------------------------------
// TestThaiTokenizerFactory
// -----------------------------------------------------------------------------

/// Ensure the filter actually decomposes text.
fn thf_test_word_break() -> Result<()> {
    LuceneTestCase::assume_true(
        "JRE does not support Thai dictionary-based BreakIterator",
        *DBBI_AVAILABLE,
    );
    let mut tokenizer =
        tokenizer_factory(TH, "Thai", &[])?.create_with_factory(B::new_attribute_factory())?;
    tokenizer.set_reader(Box::new(StringCharReader::new("การที่ได้ต้องแสดงว่างานดี")))?;
    B::assert_token_stream_contents(
        &mut tokenizer,
        &["การ", "ที่", "ได้", "ต้อง", "แสดง", "ว่า", "งาน", "ดี"],
    )
}

/// Test that bogus arguments result in exception
fn thf_test_bogus_arguments() -> Result<()> {
    LuceneTestCase::assume_true(
        "JRE does not support Thai dictionary-based BreakIterator",
        *DBBI_AVAILABLE,
    );
    let expected = iae_message(|| tokenizer_factory(TH, "Thai", &["bogusArg", "bogusValue"]));
    Assert::assert_true(expected.contains("Unknown parameters"));
    Ok(())
}

fn thai_tokenizer_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.th.TestThaiTokenizerFactory")
        .method(t("testWordBreak", thf_test_word_break))
        .method(t("testBogusArguments", thf_test_bogus_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// TestSynonymFilterFactory
// -----------------------------------------------------------------------------

/// `stream instanceof SynonymFilter`: the stream's `Debug` names the
/// filter's body (the port's `TokenFilter` offers no `Any`).
fn is_synonym_filter(stream: &dyn TokenStream) -> bool {
    let debug = format!("{stream:?}");
    debug.starts_with("TokenFilter") && debug.contains("logic: SynonymFilterLogic")
}

/// checks for synonyms of "GB" in synonyms.txt
fn check_solr_synonyms(factory: Arc<dyn TokenFilterFactory>) -> Result<()> {
    let stream = B::whitespace_mock_tokenizer(Box::new(StringCharReader::new("GB")))?;
    let mut stream = factory.create(boxed(stream))?;
    Assert::assert_true(is_synonym_filter(&*stream));
    B::assert_token_stream_contents_pos_increments(
        &mut *stream,
        &["GB", "gib", "gigabyte", "gigabytes"],
        &[1, 0, 0, 0],
    )
}

/// checks for synonyms of "second" in synonyms-wordnet.txt
fn check_wordnet_synonyms(factory: Arc<dyn TokenFilterFactory>) -> Result<()> {
    let stream = B::whitespace_mock_tokenizer(Box::new(StringCharReader::new("second")))?;
    let mut stream = factory.create(boxed(stream))?;
    Assert::assert_true(is_synonym_filter(&*stream));
    B::assert_token_stream_contents_pos_increments(
        &mut *stream,
        &["second", "2nd", "two"],
        &[1, 0, 0],
    )
}

/// `tokenFilterFactory("Synonym", keysAndValues...)` of the test class.
fn synonym(keys_and_values: &[&str]) -> Result<Arc<dyn TokenFilterFactory>> {
    token_filter_factory(SYNONYM, "Synonym", keys_and_values)
}

/// test that we can parse and use the solr syn file
fn sff_test_synonyms() -> Result<()> {
    check_solr_synonyms(synonym(&["synonyms", "synonyms.txt"])?)
}

/// if the synonyms are completely empty, test that we still analyze correctly
fn sff_test_empty_synonyms() -> Result<()> {
    let stream = B::whitespace_mock_tokenizer(Box::new(StringCharReader::new("GB")))?;
    let mut stream = F::token_filter_factory_with_loader(
        "Synonym",
        Some(Version::LATEST),
        &StringMockResourceLoader::new(""), // empty file!
        &["synonyms", "synonyms.txt"],
    )?
    .create(boxed(stream))?;
    B::assert_token_stream_contents(&mut *stream, &["GB"])
}

fn sff_test_format() -> Result<()> {
    check_solr_synonyms(synonym(&["synonyms", "synonyms.txt", "format", "solr"])?)?;
    check_wordnet_synonyms(synonym(&[
        "synonyms",
        "synonyms-wordnet.txt",
        "format",
        "wordnet",
    ])?)?;
    // explicit class should work the same as the "solr" alias
    check_solr_synonyms(synonym(&[
        "synonyms",
        "synonyms.txt",
        "format",
        "org.apache.lucene.analysis.synonym.SolrSynonymParser",
    ])?)
}

/// Test that bogus arguments result in exception
fn sff_test_bogus_arguments() -> Result<()> {
    let expected = iae_message(|| synonym(&["synonyms", "synonyms.txt", "bogusArg", "bogusValue"]));
    Assert::assert_true(expected.contains("Unknown parameters"));
    Ok(())
}

/// Test that analyzer and tokenizerFactory is both specified
fn sff_test_analyzer() -> Result<()> {
    let analyzer = "org.apache.lucene.analysis.cjk.CJKAnalyzer";
    let tokenizer_factory = "org.apache.lucene.analysis.pattern.PatternTokenizerFactory";
    let factory = synonym(&["synonyms", "synonyms2.txt", "analyzer", analyzer])?;
    // `assertNotNull(factory)`: a value, never null.
    let _ = factory;
    let expected = iae_message(|| {
        synonym(&[
            "synonyms",
            "synonyms.txt",
            "analyzer",
            analyzer,
            "tokenizerFactory",
            tokenizer_factory,
        ])
    });
    Assert::assert_true(expected.contains("Analyzer and TokenizerFactory can't be specified both"));
    Ok(())
}

/// Test that we can parse TokenierFactory's arguments
fn sff_test_tokenizer_factory_arguments() -> Result<()> {
    let clazz = "org.apache.lucene.analysis.pattern.PatternTokenizerFactory";
    // simple arg form
    let factory = synonym(&[
        "synonyms",
        "synonyms.txt",
        "tokenizerFactory",
        clazz,
        "pattern",
        "(.*)",
        "group",
        "0",
    ])?;
    let _ = factory;
    // prefix
    let factory = synonym(&[
        "synonyms",
        "synonyms.txt",
        "tokenizerFactory",
        clazz,
        "tokenizerFactory.pattern",
        "(.*)",
        "tokenizerFactory.group",
        "0",
    ])?;
    let _ = factory;
    // sanity check that sub-PatternTokenizerFactory fails w/o pattern
    expect_throws("java.lang.Exception", || {
        synonym(&["synonyms", "synonyms.txt", "tokenizerFactory", clazz])
    });
    // sanity check that sub-PatternTokenizerFactory fails on unexpected
    expect_throws("java.lang.Exception", || {
        synonym(&[
            "synonyms",
            "synonyms.txt",
            "tokenizerFactory",
            clazz,
            "tokenizerFactory.pattern",
            "(.*)",
            "tokenizerFactory.bogusbogusbogus",
            "bogus",
            "tokenizerFactory.group",
            "0",
        ])
    });
    Ok(())
}

fn synonym_filter_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.synonym.TestSynonymFilterFactory")
        .method(t("testSynonyms", sff_test_synonyms))
        .method(t("testEmptySynonyms", sff_test_empty_synonyms))
        .method(t("testFormat", sff_test_format))
        .method(t("testBogusArguments", sff_test_bogus_arguments))
        .method(t("testAnalyzer", sff_test_analyzer))
        .method(t(
            "testTokenizerFactoryArguments",
            sff_test_tokenizer_factory_arguments,
        ))
        .build()
}

// -----------------------------------------------------------------------------
// Suites
// -----------------------------------------------------------------------------

#[test]
fn test_cjk_analyzer() {
    run(cjk_analyzer_class);
}

#[test]
fn test_cjk_bigram_filter() {
    run(cjk_bigram_filter_class);
}

#[test]
fn test_cjk_bigram_filter_factory() {
    run(cjk_bigram_filter_factory_class);
}

#[test]
fn test_cjk_width_char_filter() {
    run(cjk_width_char_filter_class);
}

#[test]
fn test_cjk_width_char_filter_factory() {
    run(cjk_width_char_filter_factory_class);
}

#[test]
fn test_cjk_width_filter() {
    run(cjk_width_filter_class);
}

#[test]
fn test_cjk_width_filter_factory() {
    run(cjk_width_filter_factory_class);
}

#[test]
fn test_thai_analyzer() {
    run(thai_analyzer_class);
}

#[test]
fn test_thai_tokenizer_factory() {
    run(thai_tokenizer_factory_class);
}

#[test]
fn test_synonym_filter_factory() {
    run(synonym_filter_factory_class);
}
