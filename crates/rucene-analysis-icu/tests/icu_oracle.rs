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

//! The Java-oracle portability tests of `lucene/analysis/icu` (rmp #642):
//! `AnalysisIcuOracle.java` (Lucene 10.5.0 over ICU4J 77.1) and Rucene, from
//! one seed, give the same token streams for every ICU tokenizer
//! configuration, filter, normalizer, char filter and transform over a
//! multilingual corpus (Latin, CJK, Thai, Lao, Khmer, Myanmar, Arabic,
//! Hebrew, Greek, Cyrillic, Hangul, emoji, …), and the same
//! `ICUCollationKeyAnalyzer` collation keys per locale and strength.

#[path = "../../../tests/sprint17_oracle/mod.rs"]
mod sprint17_oracle;

mod support;

use std::fmt::Write as _;
use std::sync::Arc;

use icu4j::text::collator::{self, Collator, IDENTICAL, PRIMARY};
use icu4j::text::normalizer2::{self, Mode, Normalizer2};
use icu4j::text::transliterator;
use icu4j::util::u_locale::ULocale;
use randomizedtesting::jdk::random::{JdkRandom, Random};
use rucene::analysis::tokenattributes::{
    CharTermAttribute, OffsetAttribute, PositionIncrementAttribute, TermToBytesRefAttribute,
    TypeAttribute,
};
use rucene::analysis::{Analyzer, StringCharReader, TokenStreamComponents};
use rucene::error::Result;
use rucene_analysis_common::analysis::cjk::cjk_bigram_filter::new_cjk_bigram_filter;
use rucene_analysis_common::analysis::core::keyword_tokenizer::new_keyword_tokenizer;
use rucene_analysis_icu::analysis::icu::icu_collation_key_analyzer::ICUCollationKeyAnalyzer;
use rucene_analysis_icu::analysis::icu::icu_folding_filter::ICUFoldingFilter;
use rucene_analysis_icu::analysis::icu::icu_normalizer2_char_filter::ICUNormalizer2CharFilter;
use rucene_analysis_icu::analysis::icu::icu_normalizer2_filter::{
    new_icu_normalizer2_filter, new_icu_normalizer2_filter_with,
};
use rucene_analysis_icu::analysis::icu::icu_transform_filter::new_icu_transform_filter;
use rucene_analysis_icu::analysis::icu::segmentation::default_icu_tokenizer_config::DefaultICUTokenizerConfig;
use rucene_analysis_icu::analysis::icu::segmentation::icu_tokenizer::{
    new_icu_tokenizer, new_icu_tokenizer_with_config,
};
use rucene_analysis_icu::analysis::icu::tokenattributes::script_attribute::ScriptAttribute;
use rucene_test_framework::analysis::mock_analyzer::BoxedStream;
use rucene_test_framework::analysis::{components, shared, sink, tokenizer_components};
use sprint17_oracle::{escape, Module};
use support::{boxed, own, FnAnalyzer};

const SEED_TOKENS: i64 = 0x0642_0001;
const SEED_COLLATION: i64 = 0x0642_0002;

/// `AnalysisIcuOracle.FRAGMENTS`.
const FRAGMENTS: [&str; 30] = [
    "The quick brown fox",
    "don't stop",
    "O'Reilly's 2.5 km",
    "résumé naïve café",
    "Ruß ELİF",
    "Ｔｅｓｔ １２３４",
    "我是中国人",
    "定義切分字串的基本單位",
    "それはまだ実験段階にあります",
    "カタカナ ﾊﾟﾅｿﾆｯｸ",
    "훈민정음 안녕하세요",
    "การที่ได้ต้องแสดงว่างานดี",
    "ພາສາລາວກວ່າດອກ",
    "ផ្ទះស្កឹមស្កៃបីបួនខ្នងនេះ",
    "သက်ဝင်လှုပ်ရှားစေပြီး",
    "الفيلم الوثائقي الأول",
    "ویکی پدیای انگلیسی ۲۵",
    "דנקנר תקף את הדו\"ח",
    "Γράφεται σε συνεργασία",
    "Российская Федерация",
    "💩 👩‍❤️‍👩 🇺🇸🇺🇸",
    "#️⃣ 3️⃣ 👨🏼‍⚕️",
    "簡化字 ヒラガナ",
    "℃ № ㈱ ㌘ ｻﾞ",
    "\u{FDFA} ﴳﴺﰧ",
    "৭০৬ ๑๒๓๔",
    "क्‍ष चि",
    "𐐖𐐾 𩬅艱鍟",
    "a-b c_d e.f",
    "  ,;  ",
];

/// `AnalysisIcuOracle.SEPARATORS`.
const SEPARATORS: [&str; 4] = [" ", "", ". ", "\n"];

/// `AnalysisIcuOracle.text(r, n)`.
fn text(r: &dyn Random, n: i32) -> String {
    let mut sb = String::new();
    for i in 0..n {
        if i > 0 {
            sb.push_str(SEPARATORS[r.next_int_bound(SEPARATORS.len() as i32) as usize]);
        }
        sb.push_str(FRAGMENTS[r.next_int_bound(FRAGMENTS.len() as i32) as usize]);
    }
    sb
}

/// `AnalysisIcuOracle.CHAINS`.
const CHAINS: [&str; 11] = [
    "icu",
    "icu-nocjk",
    "icu-myanmar-syllable",
    "icu-cjkbigram",
    "folding",
    "nfkc_cf",
    "nfkc",
    "nfd",
    "nfkd",
    "charfilter-nfkc_cf",
    "charfilter-nfd",
];

/// `AnalysisIcuOracle.TRANSFORMS`.
const TRANSFORMS: [&str; 14] = [
    "Any-Latin",
    "Traditional-Simplified",
    "Katakana-Hiragana",
    "Fullwidth-Halfwidth",
    "Cyrillic-Latin",
    "Han-Latin",
    "NFD; [:Nonspacing Mark:] Remove",
    "Latin-ASCII",
    "Greek-Latin",
    "Arabic-Latin",
    "Hebrew-Latin",
    "Thai-Latin",
    "Any-Upper",
    "Any-Title",
];

fn norm(name: &str, mode: Mode) -> Arc<dyn Normalizer2> {
    normalizer2::get_instance(None, name, mode).expect("a built-in normalizer")
}

fn u(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

/// `AnalysisIcuOracle.chain(name)`.
fn chain(name: &'static str) -> Arc<dyn Analyzer> {
    let create = move |_: Option<&str>| -> Result<TokenStreamComponents> {
        let icu_over = |filter: &dyn Fn(
            Box<dyn rucene::analysis::TokenStream>,
        )
            -> Result<Box<dyn rucene::analysis::TokenStream>>|
         -> Result<TokenStreamComponents> {
            let t = own(new_icu_tokenizer()?);
            let stream = filter(shared(&t))?;
            Ok(components(t, sink(BoxedStream(stream))))
        };
        match name {
            "icu" => Ok(tokenizer_components(own(new_icu_tokenizer()?))),
            "icu-nocjk" => Ok(tokenizer_components(own(new_icu_tokenizer_with_config(
                Arc::new(DefaultICUTokenizerConfig::new(false, true)),
            )?))),
            "icu-myanmar-syllable" => Ok(tokenizer_components(own(new_icu_tokenizer_with_config(
                Arc::new(DefaultICUTokenizerConfig::new(false, false)),
            )?))),
            "icu-cjkbigram" => {
                let t = own(new_icu_tokenizer_with_config(Arc::new(
                    DefaultICUTokenizerConfig::new(false, true),
                ))?);
                let stream = boxed(new_cjk_bigram_filter(shared(&t))?);
                Ok(components(t, sink(BoxedStream(stream))))
            }
            "folding" => icu_over(&|t| Ok(boxed(ICUFoldingFilter::new(t)?))),
            "nfkc_cf" => icu_over(&|t| Ok(boxed(new_icu_normalizer2_filter(t)?))),
            "nfkc" => icu_over(&|t| {
                Ok(boxed(new_icu_normalizer2_filter_with(
                    t,
                    norm("nfkc", Mode::Compose),
                )?))
            }),
            "nfd" => icu_over(&|t| {
                Ok(boxed(new_icu_normalizer2_filter_with(
                    t,
                    norm("nfc", Mode::Decompose),
                )?))
            }),
            "nfkd" => icu_over(&|t| {
                Ok(boxed(new_icu_normalizer2_filter_with(
                    t,
                    norm("nfkc", Mode::Decompose),
                )?))
            }),
            "charfilter-nfkc_cf" | "charfilter-nfd" => {
                Ok(tokenizer_components(own(new_icu_tokenizer()?)))
            }
            id => {
                let t = own(new_keyword_tokenizer());
                let stream = boxed(new_icu_transform_filter(
                    shared(&t),
                    transliterator::get_instance(&u(id))?,
                )?);
                Ok(components(t, sink(BoxedStream(stream))))
            }
        }
    };
    FnAnalyzer::new("AnalysisIcuOracle$1", create)
        .init_reader(move |_, reader| match name {
            "charfilter-nfkc_cf" => {
                Box::new(ICUNormalizer2CharFilter::new(reader).expect("ICUNormalizer2CharFilter"))
            }
            "charfilter-nfd" => Box::new(
                ICUNormalizer2CharFilter::with_normalizer(reader, norm("nfc", Mode::Decompose))
                    .expect("ICUNormalizer2CharFilter"),
            ),
            _ => reader,
        })
        .arc()
}

/// `AnalysisIcuOracle.dump(a, text, out)`.
fn dump(a: &dyn Analyzer, text: &str, out: &mut String) -> Result<()> {
    let ts = a.token_stream("f", Box::new(StringCharReader::new(text)))?;
    let mut ts = ts.borrow_mut();
    {
        let source = ts.attribute_source_mut();
        source.add_attribute_interface::<dyn CharTermAttribute>()?;
        source.add_attribute_interface::<dyn OffsetAttribute>()?;
        source.add_attribute_interface::<dyn TypeAttribute>()?;
        source.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
    }
    let has_script = ts
        .attribute_source()
        .get_attribute_interface::<dyn ScriptAttribute>()
        .is_some();
    ts.reset()?;
    while ts.increment_token()? {
        let source = ts.attribute_source();
        let term = source
            .get_attribute_interface::<dyn CharTermAttribute>()
            .expect("CharTermAttribute");
        let term: String = term.buffer()[..term.length()].iter().collect();
        let offset = source
            .get_attribute_interface::<dyn OffsetAttribute>()
            .expect("OffsetAttribute");
        let type_att = source
            .get_attribute_interface::<dyn TypeAttribute>()
            .expect("TypeAttribute");
        let pos_inc = source
            .get_attribute_interface::<dyn PositionIncrementAttribute>()
            .expect("PositionIncrementAttribute");
        let script = if has_script {
            let s = source
                .get_attribute_interface::<dyn ScriptAttribute>()
                .expect("ScriptAttribute");
            format!(" s{}", s.get_code())
        } else {
            String::new()
        };
        writeln!(
            out,
            "  {} {}-{} {} +{}{}",
            escape(&term),
            offset.start_offset(),
            offset.end_offset(),
            type_att.type_value(),
            pos_inc.get_position_increment(),
            script
        )
        .unwrap();
    }
    ts.end()?;
    let end = ts
        .attribute_source()
        .get_attribute_interface::<dyn OffsetAttribute>()
        .expect("OffsetAttribute")
        .end_offset();
    writeln!(out, "  end {end}").unwrap();
    ts.close()
}

/// `AnalysisIcuOracle.tokens(r, out)`.
fn rust_tokens(seed: i64) -> String {
    let r = JdkRandom::new(seed);
    let names: Vec<&'static str> = CHAINS.iter().chain(TRANSFORMS.iter()).copied().collect();
    let chains: Vec<Arc<dyn Analyzer>> = names.iter().map(|n| chain(n)).collect();
    let mut out = String::new();
    for _ in 0..40 {
        let n = 1 + r.next_int_bound(8);
        let text = text(&r, n);
        writeln!(out, "text {}", escape(&text)).unwrap();
        for (name, a) in names.iter().zip(&chains) {
            writeln!(out, " {name}").unwrap();
            dump(&**a, &text, &mut out).unwrap();
        }
    }
    for a in chains {
        a.close();
    }
    out
}

/// `AnalysisIcuOracle.LOCALES`.
const LOCALES: [&str; 17] = [
    "",
    "en",
    "de",
    "fr",
    "ja",
    "zh",
    "ar",
    "fa",
    "sv",
    "es",
    "th",
    "ko",
    "ru",
    "he",
    "el",
    "de@collation=phonebook",
    "zh@collation=stroke",
];

/// `AnalysisIcuOracle.collation(r, out)`.
fn rust_collation(seed: i64) -> String {
    let r = JdkRandom::new(seed);
    let texts: Vec<String> = (0..20)
        .map(|_| {
            let n = 1 + r.next_int_bound(3);
            text(&r, n)
        })
        .collect();
    let strengths = [-1, PRIMARY, IDENTICAL];
    let mut out = String::new();
    for locale in LOCALES {
        for strength in strengths {
            let mut collator =
                collator::get_instance_ulocale(Some(&ULocale::new(locale).expect("a locale ID")))
                    .expect("a collator");
            if strength >= 0 {
                collator.set_strength(strength).expect("setStrength");
            }
            writeln!(out, "locale {locale} strength {strength}").unwrap();
            let collator: Arc<dyn Collator> = Arc::from(collator);
            let a = ICUCollationKeyAnalyzer::new(collator);
            for text in &texts {
                let ts = a
                    .token_stream("f", Box::new(StringCharReader::new(text)))
                    .expect("tokenStream");
                let mut ts = ts.borrow_mut();
                ts.attribute_source_mut()
                    .add_attribute_interface::<dyn TermToBytesRefAttribute>()
                    .expect("addAttribute");
                ts.reset().expect("reset");
                while ts.increment_token().expect("incrementToken") {
                    let bytes = ts
                        .attribute_source()
                        .get_attribute_interface::<dyn TermToBytesRefAttribute>()
                        .expect("TermToBytesRefAttribute")
                        .get_bytes_ref();
                    let hex: String = bytes.bytes[bytes.offset..bytes.offset + bytes.length]
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect();
                    writeln!(out, "  {hex}").unwrap();
                }
                ts.end().expect("end");
                ts.close().expect("close");
            }
            a.close();
        }
    }
    out
}

#[test]
fn analysis_icu_tokens_oracle() {
    sprint17_oracle::pin_tests_seed();
    let java = sprint17_oracle::run(Module::AnalysisIcu, "tokens", SEED_TOKENS, &[]);
    sprint17_oracle::assert_same(&java, &rust_tokens(SEED_TOKENS));
}

#[test]
fn analysis_icu_collation_oracle() {
    sprint17_oracle::pin_tests_seed();
    let java = sprint17_oracle::run(Module::AnalysisIcu, "collation", SEED_COLLATION, &[]);
    sprint17_oracle::assert_same(&java, &rust_collation(SEED_COLLATION));
}
