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
//! that rmp tasks #454 and #455 list, each run as a suite by the ported
//! runner: `org.apache.lucene.analysis.snowball.TestSnowball` and
//! `TestSnowballPorterFilterFactory`,
//! `org.apache.lucene.analysis.miscellaneous.TestKeywordRepeatFilter` (#454)
//! and `org.apache.lucene.analysis.lt.TestLithuanianStemming` (#455).
//!
//! `TestSnowball.checkRandomStrings(String)` is a helper (it takes an
//! argument), not a test method; `testRandomStrings` calls it for each
//! language of `languages.txt`, copied byte for byte.

mod support;

use std::cell::RefCell;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::tokenattributes::{
    CharTermAttribute, FlagsAttribute, OffsetAttribute, PayloadAttribute,
    PositionIncrementAttribute, TypeAttribute,
};
use rucene::analysis::wordlist_loader::get_lines;
use rucene::analysis::{Analyzer, StringCharReader, TokenStream};
use rucene::error::Result;
use rucene::util::{AttributeSource, BytesRef};
use rucene_analysis_common::analysis::miscellaneous::keyword_repeat_filter::new_keyword_repeat_filter;
use rucene_analysis_common::analysis::miscellaneous::remove_duplicates_token_filter::new_remove_duplicates_token_filter;
use rucene_analysis_common::analysis::snowball::snowball_filter::{
    new_snowball_filter, new_snowball_filter_by_name,
};
use rucene_analysis_common::tartarus::snowball::ext::english_stemmer::EnglishStemmer;
use rucene_analysis_common::tartarus::snowball::ext::lithuanian_stemmer::LithuanianStemmer;
use rucene_analysis_common::tartarus::snowball::snowball_stemmer::SnowballStemmer;
use rucene_test_framework::analysis::mock_tokenizer::MockTokenizer;
use rucene_test_framework::analysis::{BaseTokenStreamFactoryTestCase, BaseTokenStreamTestCase};
use rucene_test_framework::util::lucene_test_case::RANDOM_MULTIPLIER;
use rucene_test_framework::util::{LuceneTestCase, StringMockResourceLoader};
use support::{
    boxed, btstc_class, factory_class, hook, iae_message, keyword_tokenizer_analyzer, run, t,
    tokenizer_analyzer, whitespace_analyzer,
};

type B = BaseTokenStreamTestCase;
type F = BaseTokenStreamFactoryTestCase;

// -----------------------------------------------------------------------------
// TestSnowball
// -----------------------------------------------------------------------------

/// `new Analyzer() { MockTokenizer(); SnowballFilter(tokenizer, lang) }`.
fn mock_snowball_analyzer(name: &'static str, lang: String) -> Arc<dyn Analyzer> {
    tokenizer_analyzer(name, MockTokenizer::whitespace_lower_case, move |t| {
        Ok(boxed(new_snowball_filter_by_name(t, &lang)?))
    })
}

fn sb_test_english() -> Result<()> {
    let a = mock_snowball_analyzer(
        "org.apache.lucene.analysis.snowball.TestSnowball$1",
        "English".into(),
    );
    B::assert_analyzes_to(&*a, "he abhorred accents", &["he", "abhor", "accent"])?;
    a.close();
    Ok(())
}

/// `TestSnowball.TestTokenStream`: one token, forever.
#[derive(Debug)]
struct TestTokenStream {
    source: AttributeSource,
}

impl TestTokenStream {
    fn new() -> Result<TestTokenStream> {
        let mut source = AttributeSource::new();
        source.add_attribute_interface::<dyn CharTermAttribute>()?;
        source.add_attribute_interface::<dyn OffsetAttribute>()?;
        source.add_attribute_interface::<dyn TypeAttribute>()?;
        source.add_attribute_interface::<dyn PayloadAttribute>()?;
        source.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
        source.add_attribute_interface::<dyn FlagsAttribute>()?;
        Ok(TestTokenStream { source })
    }
}

impl TokenStream for TestTokenStream {
    fn increment_token(&mut self) -> Result<bool> {
        let s = &self.source;
        s.clear_attributes();
        {
            let mut term = s
                .get_attribute_interface_mut::<dyn CharTermAttribute>()
                .expect("termAtt");
            term.set_empty();
            term.append_string(Some("accents"));
        }
        s.get_attribute_interface_mut::<dyn OffsetAttribute>()
            .expect("offsetAtt")
            .set_offset(2, 7);
        s.get_attribute_interface_mut::<dyn TypeAttribute>()
            .expect("typeAtt")
            .set_type("wrd".into());
        s.get_attribute_interface_mut::<dyn PositionIncrementAttribute>()
            .expect("posIncAtt")
            .set_position_increment(3);
        s.get_attribute_interface_mut::<dyn PayloadAttribute>()
            .expect("payloadAtt")
            .set_payload(Some(BytesRef::new(vec![0u8, 1, 2, 3])));
        s.get_attribute_interface_mut::<dyn FlagsAttribute>()
            .expect("flagsAtt")
            .set_flags(77);
        Ok(true)
    }

    fn attribute_source(&self) -> &AttributeSource {
        &self.source
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        &mut self.source
    }
}

fn sb_test_filter_tokens() -> Result<()> {
    let mut filter = new_snowball_filter_by_name(boxed(TestTokenStream::new()?), "English")?;
    filter.increment_token()?;
    let s = filter.attribute_source();
    let term: String = {
        let a = s
            .get_attribute_interface::<dyn CharTermAttribute>()
            .unwrap();
        a.buffer()[..a.length()].iter().collect()
    };
    Assert::assert_equals("accent", term.as_str());
    let (start, end) = {
        let o = s.get_attribute_interface::<dyn OffsetAttribute>().unwrap();
        (o.start_offset(), o.end_offset())
    };
    Assert::assert_equals(2, start);
    Assert::assert_equals(7, end);
    let ty = s
        .get_attribute_interface::<dyn TypeAttribute>()
        .unwrap()
        .type_value()
        .to_string();
    Assert::assert_equals("wrd", ty.as_str());
    let pos_inc = s
        .get_attribute_interface::<dyn PositionIncrementAttribute>()
        .unwrap()
        .get_position_increment();
    Assert::assert_equals(3, pos_inc);
    let flags = s
        .get_attribute_interface::<dyn FlagsAttribute>()
        .unwrap()
        .get_flags();
    Assert::assert_equals(77, flags);
    let payload = s
        .get_attribute_interface::<dyn PayloadAttribute>()
        .unwrap()
        .get_payload()
        .cloned();
    Assert::assert_true(payload == Some(BytesRef::new(vec![0u8, 1, 2, 3])));
    Ok(())
}

/// `SNOWBALL_LANGS`: the lines of `languages.txt`.
fn snowball_langs() -> Result<Vec<String>> {
    let bytes = support::TEST_RESOURCES
        .iter()
        .find(|(name, _)| *name == "org/apache/lucene/analysis/snowball/languages.txt")
        .expect("languages.txt")
        .1;
    get_lines(bytes, "UTF-8")
}

fn sb_test_empty_term() -> Result<()> {
    for lang in snowball_langs()? {
        let a = keyword_tokenizer_analyzer(
            "org.apache.lucene.analysis.snowball.TestSnowball$2",
            move |t| Ok(boxed(new_snowball_filter_by_name(t, &lang)?)),
        );
        B::check_one_term(&*a, "", "")?;
        a.close();
    }
    Ok(())
}

/// `checkRandomStrings(snowballLanguage)`.
fn check_random_strings(snowball_language: String) -> Result<()> {
    let a = mock_snowball_analyzer(
        "org.apache.lucene.analysis.snowball.TestSnowball$3",
        snowball_language,
    );
    B::check_random_data(&*LuceneTestCase::random(), &a, 20 * *RANDOM_MULTIPLIER)?;
    a.close();
    Ok(())
}

fn sb_test_random_strings() -> Result<()> {
    for lang in snowball_langs()? {
        check_random_strings(lang)?;
    }
    Ok(())
}

fn snowball_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.snowball.TestSnowball")
        .method(t("testEnglish", sb_test_english))
        .method(t("testFilterTokens", sb_test_filter_tokens))
        .method(t("testEmptyTerm", sb_test_empty_term))
        .method(t("testRandomStrings", sb_test_random_strings))
        .build()
}

// -----------------------------------------------------------------------------
// TestSnowballPorterFilterFactory
// -----------------------------------------------------------------------------

fn spf_test() -> Result<()> {
    let text = "The fledgling banks were counting on a big boom in banking";
    let mut stemmer = EnglishStemmer::new();
    // `text.split("\\s")`: single spaces, no trailing empty string
    let test: Vec<&str> = text.split(' ').collect();
    let mut gold: Vec<String> = Vec::with_capacity(test.len());
    for word in &test {
        stemmer.as_mut().set_current(word);
        stemmer.stem();
        gold.push(stemmer.as_ref().get_current());
    }
    let mut stream: Box<dyn TokenStream> = boxed(B::whitespace_mock_tokenizer(Box::new(
        StringCharReader::new(text),
    ))?);
    stream = F::token_filter_factory("SnowballPorter", &["language", "English"])?.create(stream)?;
    let gold: Vec<&str> = gold.iter().map(String::as_str).collect();
    B::assert_token_stream_contents(&mut *stream, &gold)
}

/// Test the protected words mechanism of SnowballPorterFilterFactory
fn spf_test_protected() -> Result<()> {
    let mut stream: Box<dyn TokenStream> = boxed(B::whitespace_mock_tokenizer(Box::new(
        StringCharReader::new("ridding of some stemming"),
    ))?);
    stream = F::token_filter_factory_with_loader(
        "SnowballPorter",
        Some(rucene::util::extra::Version::LATEST),
        &StringMockResourceLoader::new("ridding"),
        &["protected", "protwords.txt", "language", "English"],
    )?
    .create(stream)?;
    B::assert_token_stream_contents(&mut *stream, &["ridding", "of", "some", "stem"])
}

/// Test that bogus arguments result in exception
fn spf_test_bogus_arguments() -> Result<()> {
    let expected =
        iae_message(|| F::token_filter_factory("SnowballPorter", &["bogusArg", "bogusValue"]));
    Assert::assert_true(expected.contains("Unknown parameters"));
    Ok(())
}

fn snowball_porter_filter_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.snowball.TestSnowballPorterFilterFactory")
        .method(t("test", spf_test))
        .method(t("testProtected", spf_test_protected))
        .method(t("testBogusArguments", spf_test_bogus_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// TestKeywordRepeatFilter
// -----------------------------------------------------------------------------

fn krf_test_basic() -> Result<()> {
    let mut ts = new_remove_duplicates_token_filter(boxed(new_snowball_filter_by_name(
        boxed(new_keyword_repeat_filter(boxed(
            B::whitespace_mock_tokenizer_str("the birds are flying")?,
        ))?),
        "English",
    )?))?;
    B::assert_token_stream_contents_pos_increments(
        &mut ts,
        &["the", "birds", "bird", "are", "flying", "fli"],
        &[1, 1, 0, 1, 1, 0],
    )
}

fn krf_test_composition() -> Result<()> {
    let mut ts = new_remove_duplicates_token_filter(boxed(new_snowball_filter_by_name(
        boxed(new_keyword_repeat_filter(boxed(
            new_keyword_repeat_filter(boxed(B::whitespace_mock_tokenizer_str(
                "the birds are flying",
            )?))?,
        ))?),
        "English",
    )?))?;
    B::assert_token_stream_contents_pos_increments(
        &mut ts,
        &["the", "birds", "bird", "are", "flying", "fli"],
        &[1, 1, 0, 1, 1, 0],
    )
}

fn keyword_repeat_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.miscellaneous.TestKeywordRepeatFilter")
        .method(t("testBasic", krf_test_basic))
        .method(t("testComposition", krf_test_composition))
        .build()
}

// -----------------------------------------------------------------------------
// TestLithuanianStemming
// -----------------------------------------------------------------------------

thread_local! {
    /// The analyzer field `setUp` sets.
    static ANALYZER: RefCell<Option<Arc<dyn Analyzer>>> = const { RefCell::new(None) };
}

fn analyzer() -> Arc<dyn Analyzer> {
    ANALYZER.with(|a| Arc::clone(a.borrow().as_ref().expect("setUp")))
}

fn lt_set_up() -> Result<()> {
    LuceneTestCase::super_set_up();
    let a = whitespace_analyzer(
        "org.apache.lucene.analysis.lt.TestLithuanianStemming$1",
        |t| {
            Ok(boxed(new_snowball_filter(
                t,
                Box::new(LithuanianStemmer::new()) as Box<dyn SnowballStemmer>,
            )?))
        },
    );
    ANALYZER.with(|f| *f.borrow_mut() = Some(a));
    Ok(())
}

fn lt_tear_down() -> Result<()> {
    if let Some(a) = ANALYZER.with(|a| a.borrow_mut().take()) {
        a.close();
    }
    LuceneTestCase::super_tear_down();
    Ok(())
}

fn lt_test_nouns_i() -> Result<()> {
    let a = analyzer();
    // n. decl. I (-as)
    B::check_one_term(&*a, "vaikas", "vaik")?; // nom. sing.
    B::check_one_term(&*a, "vaikai", "vaik")?; // nom. pl.
    B::check_one_term(&*a, "vaiko", "vaik")?; // gen. sg.
    B::check_one_term(&*a, "vaik\u{173}", "vaik")?; // gen. pl.
    B::check_one_term(&*a, "vaikui", "vaik")?; // dat. sg.
    B::check_one_term(&*a, "vaikams", "vaik")?; // dat. pl.
    B::check_one_term(&*a, "vaik\u{105}", "vaik")?; // acc. sg.
    B::check_one_term(&*a, "vaikus", "vaik")?; // acc. pl.
    B::check_one_term(&*a, "vaiku", "vaik")?; // ins. sg.
    B::check_one_term(&*a, "vaikais", "vaik")?; // ins. pl.
    B::check_one_term(&*a, "vaike", "vaik")?; // loc. sg.
    B::check_one_term(&*a, "vaikuose", "vaik")?; // loc. pl.
    B::check_one_term(&*a, "vaike", "vaik")?; // voc. sg.
    B::check_one_term(&*a, "vaikai", "vaik")?; // voc. pl.
                                               // n. decl. I (-is)
    B::check_one_term(&*a, "brolis", "brol")?; // nom. sing.
    B::check_one_term(&*a, "broliai", "brol")?; // nom. pl.
    B::check_one_term(&*a, "brolio", "brol")?; // gen. sg.
    B::check_one_term(&*a, "broli\u{173}", "brol")?; // gen. pl.
    B::check_one_term(&*a, "broliui", "brol")?; // dat. sg.
    B::check_one_term(&*a, "broliams", "brol")?; // dat. pl.
    B::check_one_term(&*a, "brol\u{12F}", "brol")?; // acc. sg.
    B::check_one_term(&*a, "brolius", "brol")?; // acc. pl.
    B::check_one_term(&*a, "broliu", "brol")?; // ins. sg.
    B::check_one_term(&*a, "broliais", "brol")?; // ins. pl.
    B::check_one_term(&*a, "brolyje", "brol")?; // loc. sg.
    B::check_one_term(&*a, "broliuose", "brol")?; // loc. pl.
    B::check_one_term(&*a, "broli", "brol")?; // voc. sg.
    B::check_one_term(&*a, "broliai", "brol")?; // voc. pl.
                                                // n. decl. I (-ys)
                                                // note: some forms don't conflate
    B::check_one_term(&*a, "arklys", "arkl")?; // nom. sing.
    B::check_one_term(&*a, "arkliai", "arkliai")?; // nom. pl.
    B::check_one_term(&*a, "arklio", "arkl")?; // gen. sg.
    B::check_one_term(&*a, "arkli\u{173}", "arkl")?; // gen. pl.
    B::check_one_term(&*a, "arkliui", "arkliui")?; // dat. sg.
    B::check_one_term(&*a, "arkliams", "arkliam")?; // dat. pl.
    B::check_one_term(&*a, "arkl\u{12F}", "arkl")?; // acc. sg.
    B::check_one_term(&*a, "arklius", "arklius")?; // acc. pl.
    B::check_one_term(&*a, "arkliu", "arkl")?; // ins. sg.
    B::check_one_term(&*a, "arkliais", "arkliais")?; // ins. pl.
    B::check_one_term(&*a, "arklyje", "arklyj")?; // loc. sg.
    B::check_one_term(&*a, "arkliuose", "arkliuos")?; // loc. pl.
    B::check_one_term(&*a, "arkly", "arkl")?; // voc. sg.
    B::check_one_term(&*a, "arkliai", "arkliai")?; // voc. pl.
    Ok(())
}

fn lt_test_nouns_ii() -> Result<()> {
    let a = analyzer();
    // n. decl II (-a)
    B::check_one_term(&*a, "motina", "motin")?; // nom. sing.
    B::check_one_term(&*a, "motinos", "motin")?; // nom. pl.
    B::check_one_term(&*a, "motinos", "motin")?; // gen. sg.
    B::check_one_term(&*a, "motin\u{173}", "motin")?; // gen. pl.
    B::check_one_term(&*a, "motinai", "motin")?; // dat. sg.
    B::check_one_term(&*a, "motinoms", "motin")?; // dat. pl.
    B::check_one_term(&*a, "motin\u{105}", "motin")?; // acc. sg.
    B::check_one_term(&*a, "motinas", "motin")?; // acc. pl.
    B::check_one_term(&*a, "motina", "motin")?; // ins. sg.
    B::check_one_term(&*a, "motinomis", "motin")?; // ins. pl.
    B::check_one_term(&*a, "motinoje", "motin")?; // loc. sg.
    B::check_one_term(&*a, "motinose", "motin")?; // loc. pl.
    B::check_one_term(&*a, "motina", "motin")?; // voc. sg.
    B::check_one_term(&*a, "motinos", "motin")?; // voc. pl.
                                                 // n. decl II (-ė)
    B::check_one_term(&*a, "kat\u{117}", "kat")?; // nom. sing.
    B::check_one_term(&*a, "kat\u{117}s", "kat")?; // nom. pl.
    B::check_one_term(&*a, "kat\u{117}s", "kat")?; // gen. sg.
    B::check_one_term(&*a, "ka\u{10D}i\u{173}", "kat")?; // gen. pl.
    B::check_one_term(&*a, "katei", "kat")?; // dat. sg.
    B::check_one_term(&*a, "kat\u{117}ms", "kat")?; // dat. pl.
    B::check_one_term(&*a, "kat\u{119}", "kat")?; // acc. sg.
    B::check_one_term(&*a, "kates", "kat")?; // acc. pl.
    B::check_one_term(&*a, "kate", "kat")?; // ins. sg.
    B::check_one_term(&*a, "kat\u{117}mis", "kat")?; // ins. pl.
    B::check_one_term(&*a, "kat\u{117}je", "kat")?; // loc. sg.
    B::check_one_term(&*a, "kat\u{117}se", "kat")?; // loc. pl.
    B::check_one_term(&*a, "kate", "kat")?; // voc. sg.
    B::check_one_term(&*a, "kat\u{117}s", "kat")?; // voc. pl.
                                                   // n. decl II (-ti)
    B::check_one_term(&*a, "pati", "pat")?; // nom. sing.
    B::check_one_term(&*a, "pa\u{10D}ios", "pat")?; // nom. pl.
    B::check_one_term(&*a, "pa\u{10D}ios", "pat")?; // gen. sg.
    B::check_one_term(&*a, "pa\u{10D}i\u{173}", "pat")?; // gen. pl.
    B::check_one_term(&*a, "pa\u{10D}iai", "pat")?; // dat. sg.
    B::check_one_term(&*a, "pa\u{10D}ioms", "pat")?; // dat. pl.
    B::check_one_term(&*a, "pa\u{10D}i\u{105}", "pat")?; // acc. sg.
    B::check_one_term(&*a, "pa\u{10D}ias", "pat")?; // acc. pl.
    B::check_one_term(&*a, "pa\u{10D}ia", "pat")?; // ins. sg.
    B::check_one_term(&*a, "pa\u{10D}iomis", "pat")?; // ins. pl.
    B::check_one_term(&*a, "pa\u{10D}ioje", "pat")?; // loc. sg.
    B::check_one_term(&*a, "pa\u{10D}iose", "pat")?; // loc. pl.
    B::check_one_term(&*a, "pati", "pat")?; // voc. sg.
    B::check_one_term(&*a, "pa\u{10D}ios", "pat")?; // voc. pl.
    Ok(())
}

fn lt_test_nouns_iii() -> Result<()> {
    let a = analyzer();
    // n. decl III-m
    B::check_one_term(&*a, "vagis", "vag")?; // nom. sing.
    B::check_one_term(&*a, "vagys", "vag")?; // nom. pl.
    B::check_one_term(&*a, "vagies", "vag")?; // gen. sg.
    B::check_one_term(&*a, "vagi\u{173}", "vag")?; // gen. pl.
    B::check_one_term(&*a, "vagiui", "vag")?; // dat. sg.
    B::check_one_term(&*a, "vagims", "vag")?; // dat. pl.
    B::check_one_term(&*a, "vag\u{12F}", "vag")?; // acc. sg.
    B::check_one_term(&*a, "vagis", "vag")?; // acc. pl.
    B::check_one_term(&*a, "vagimi", "vag")?; // ins. sg.
    B::check_one_term(&*a, "vagimis", "vag")?; // ins. pl.
    B::check_one_term(&*a, "vagyje", "vag")?; // loc. sg.
    B::check_one_term(&*a, "vagyse", "vag")?; // loc. pl.
    B::check_one_term(&*a, "vagie", "vag")?; // voc. sg.
    B::check_one_term(&*a, "vagys", "vag")?; // voc. pl.
                                             // n. decl III-f
    B::check_one_term(&*a, "akis", "ak")?; // nom. sing.
    B::check_one_term(&*a, "akys", "ak")?; // nom. pl.
    B::check_one_term(&*a, "akies", "ak")?; // gen. sg.
    B::check_one_term(&*a, "aki\u{173}", "ak")?; // gen. pl.
    B::check_one_term(&*a, "akiai", "ak")?; // dat. sg.
    B::check_one_term(&*a, "akims", "ak")?; // dat. pl.
    B::check_one_term(&*a, "ak\u{12F}", "ak")?; // acc. sg.
    B::check_one_term(&*a, "akis", "ak")?; // acc. pl.
    B::check_one_term(&*a, "akimi", "ak")?; // ins. sg.
    B::check_one_term(&*a, "akimis", "ak")?; // ins. pl.
    B::check_one_term(&*a, "akyje", "ak")?; // loc. sg.
    B::check_one_term(&*a, "akyse", "ak")?; // loc. pl.
    B::check_one_term(&*a, "akie", "ak")?; // voc. sg.
    B::check_one_term(&*a, "akys", "ak")?; // voc. pl.
    Ok(())
}

fn lt_test_nouns_iv() -> Result<()> {
    let a = analyzer();
    // n. decl IV (-us)
    B::check_one_term(&*a, "s\u{16B}nus", "s\u{16B}n")?; // nom. sing.
    B::check_one_term(&*a, "s\u{16B}n\u{16B}s", "s\u{16B}n")?; // nom. pl.
    B::check_one_term(&*a, "s\u{16B}naus", "s\u{16B}n")?; // gen. sg.
    B::check_one_term(&*a, "s\u{16B}n\u{173}", "s\u{16B}n")?; // gen. pl.
    B::check_one_term(&*a, "s\u{16B}nui", "s\u{16B}n")?; // dat. sg.
    B::check_one_term(&*a, "s\u{16B}nums", "s\u{16B}n")?; // dat. pl.
    B::check_one_term(&*a, "s\u{16B}n\u{173}", "s\u{16B}n")?; // acc. sg.
    B::check_one_term(&*a, "s\u{16B}nus", "s\u{16B}n")?; // acc. pl.
    B::check_one_term(&*a, "s\u{16B}numi", "s\u{16B}n")?; // ins. sg.
    B::check_one_term(&*a, "s\u{16B}numis", "s\u{16B}n")?; // ins. pl.
    B::check_one_term(&*a, "s\u{16B}nuje", "s\u{16B}n")?; // loc. sg.
    B::check_one_term(&*a, "s\u{16B}nuose", "s\u{16B}n")?; // loc. pl.
    B::check_one_term(&*a, "s\u{16B}nau", "s\u{16B}n")?; // voc. sg.
    B::check_one_term(&*a, "s\u{16B}n\u{16B}s", "s\u{16B}n")?; // voc. pl.
                                                               // n. decl IV (-ius)
    B::check_one_term(&*a, "profesorius", "profesor")?; // nom. sing.
    B::check_one_term(&*a, "profesoriai", "profesor")?; // nom. pl.
    B::check_one_term(&*a, "profesoriaus", "profesor")?; // gen. sg.
    B::check_one_term(&*a, "profesori\u{173}", "profesor")?; // gen. pl.
    B::check_one_term(&*a, "profesoriui", "profesor")?; // dat. sg.
    B::check_one_term(&*a, "profesoriams", "profesor")?; // dat. pl.
    B::check_one_term(&*a, "profesori\u{173}", "profesor")?; // acc. sg.
    B::check_one_term(&*a, "profesorius", "profesor")?; // acc. pl.
    B::check_one_term(&*a, "profesoriumi", "profesor")?; // ins. sg.
    B::check_one_term(&*a, "profesoriais", "profesor")?; // ins. pl.
    B::check_one_term(&*a, "profesoriuje", "profesor")?; // loc. sg.
    B::check_one_term(&*a, "profesoriuose", "profesor")?; // loc. pl.
    B::check_one_term(&*a, "profesoriau", "profesor")?; // voc. sg.
    B::check_one_term(&*a, "profesoriai", "profesor")?; // voc. pl.
    Ok(())
}

fn lt_test_nouns_v() -> Result<()> {
    let a = analyzer();
    // n. decl V
    // note: gen.pl. doesn't conflate
    B::check_one_term(&*a, "vanduo", "vand")?; // nom. sing.
    B::check_one_term(&*a, "vandenys", "vand")?; // nom. pl.
    B::check_one_term(&*a, "vandens", "vand")?; // gen. sg.
    B::check_one_term(&*a, "vanden\u{173}", "vanden")?; // gen. pl.
    B::check_one_term(&*a, "vandeniui", "vand")?; // dat. sg.
    B::check_one_term(&*a, "vandenims", "vand")?; // dat. pl.
    B::check_one_term(&*a, "vanden\u{12F}", "vand")?; // acc. sg.
    B::check_one_term(&*a, "vandenis", "vand")?; // acc. pl.
    B::check_one_term(&*a, "vandeniu", "vand")?; // ins. sg.
    B::check_one_term(&*a, "vandenimis", "vand")?; // ins. pl.
    B::check_one_term(&*a, "vandenyje", "vand")?; // loc. sg.
    B::check_one_term(&*a, "vandenyse", "vand")?; // loc. pl.
    B::check_one_term(&*a, "vandenie", "vand")?; // voc. sg.
    B::check_one_term(&*a, "vandenys", "vand")?; // voc. pl.
    Ok(())
}

fn lt_test_adj_i() -> Result<()> {
    let a = analyzer();
    // adj. decl I
    B::check_one_term(&*a, "geras", "ger")?; // m. nom. sing.
    B::check_one_term(&*a, "geri", "ger")?; // m. nom. pl.
    B::check_one_term(&*a, "gero", "ger")?; // m. gen. sg.
    B::check_one_term(&*a, "ger\u{173}", "ger")?; // m. gen. pl.
    B::check_one_term(&*a, "geram", "ger")?; // m. dat. sg.
    B::check_one_term(&*a, "geriems", "ger")?; // m. dat. pl.
    B::check_one_term(&*a, "ger\u{105}", "ger")?; // m. acc. sg.
    B::check_one_term(&*a, "gerus", "ger")?; // m. acc. pl.
    B::check_one_term(&*a, "geru", "ger")?; // m. ins. sg.
    B::check_one_term(&*a, "gerais", "ger")?; // m. ins. pl.
    B::check_one_term(&*a, "gerame", "ger")?; // m. loc. sg.
    B::check_one_term(&*a, "geruose", "ger")?; // m. loc. pl.
    B::check_one_term(&*a, "gera", "ger")?; // f. nom. sing.
    B::check_one_term(&*a, "geros", "ger")?; // f. nom. pl.
    B::check_one_term(&*a, "geros", "ger")?; // f. gen. sg.
    B::check_one_term(&*a, "ger\u{173}", "ger")?; // f. gen. pl.
    B::check_one_term(&*a, "gerai", "ger")?; // f. dat. sg.
    B::check_one_term(&*a, "geroms", "ger")?; // f. dat. pl.
    B::check_one_term(&*a, "ger\u{105}", "ger")?; // f. acc. sg.
    B::check_one_term(&*a, "geras", "ger")?; // f. acc. pl.
    B::check_one_term(&*a, "gera", "ger")?; // f. ins. sg.
    B::check_one_term(&*a, "geromis", "ger")?; // f. ins. pl.
    B::check_one_term(&*a, "geroje", "ger")?; // f. loc. sg.
    B::check_one_term(&*a, "gerose", "ger")?; // f. loc. pl.
    Ok(())
}

fn lt_test_adj_ii() -> Result<()> {
    let a = analyzer();
    // adj. decl II
    B::check_one_term(&*a, "gra\u{17E}us", "gra\u{17E}")?; // m. nom. sing.
    B::check_one_term(&*a, "gra\u{17E}\u{16B}s", "gra\u{17E}")?; // m. nom. pl.
    B::check_one_term(&*a, "gra\u{17E}aus", "gra\u{17E}")?; // m. gen. sg.
    B::check_one_term(&*a, "gra\u{17E}i\u{173}", "gra\u{17E}")?; // m. gen. pl.
    B::check_one_term(&*a, "gra\u{17E}iam", "gra\u{17E}")?; // m. dat. sg.
    B::check_one_term(&*a, "gra\u{17E}iems", "gra\u{17E}")?; // m. dat. pl.
    B::check_one_term(&*a, "gra\u{17E}\u{173}", "gra\u{17E}")?; // m. acc. sg.
    B::check_one_term(&*a, "gra\u{17E}ius", "gra\u{17E}")?; // m. acc. pl.
    B::check_one_term(&*a, "gra\u{17E}iu", "gra\u{17E}")?; // m. ins. sg.
    B::check_one_term(&*a, "gra\u{17E}iais", "gra\u{17E}")?; // m. ins. pl.
    B::check_one_term(&*a, "gra\u{17E}iame", "gra\u{17E}")?; // m. loc. sg.
    B::check_one_term(&*a, "gra\u{17E}iuose", "gra\u{17E}")?; // m. loc. pl.
    B::check_one_term(&*a, "gra\u{17E}i", "gra\u{17E}")?; // f. nom. sing.
    B::check_one_term(&*a, "gra\u{17E}ios", "gra\u{17E}")?; // f. nom. pl.
    B::check_one_term(&*a, "gra\u{17E}ios", "gra\u{17E}")?; // f. gen. sg.
    B::check_one_term(&*a, "gra\u{17E}i\u{173}", "gra\u{17E}")?; // f. gen. pl.
    B::check_one_term(&*a, "gra\u{17E}iai", "gra\u{17E}")?; // f. dat. sg.
    B::check_one_term(&*a, "gra\u{17E}ioms", "gra\u{17E}")?; // f. dat. pl.
    B::check_one_term(&*a, "gra\u{17E}i\u{105}", "gra\u{17E}")?; // f. acc. sg.
    B::check_one_term(&*a, "gra\u{17E}ias", "gra\u{17E}")?; // f. acc. pl.
    B::check_one_term(&*a, "gra\u{17E}ia", "gra\u{17E}")?; // f. ins. sg.
    B::check_one_term(&*a, "gra\u{17E}iomis", "gra\u{17E}")?; // f. ins. pl.
    B::check_one_term(&*a, "gra\u{17E}ioje", "gra\u{17E}")?; // f. loc. sg.
    B::check_one_term(&*a, "gra\u{17E}iose", "gra\u{17E}")?; // f. loc. pl.
    Ok(())
}

fn lt_test_adj_iii() -> Result<()> {
    let a = analyzer();
    // adj. decl III
    B::check_one_term(&*a, "vidutinis", "vidutin")?; // m. nom. sing.
    B::check_one_term(&*a, "vidutiniai", "vidutin")?; // m. nom. pl.
    B::check_one_term(&*a, "vidutinio", "vidutin")?; // m. gen. sg.
    B::check_one_term(&*a, "vidutini\u{173}", "vidutin")?; // m. gen. pl.
    B::check_one_term(&*a, "vidutiniam", "vidutin")?; // m. dat. sg.
    B::check_one_term(&*a, "vidutiniams", "vidutin")?; // m. dat. pl.
    B::check_one_term(&*a, "vidutin\u{12F}", "vidutin")?; // m. acc. sg.
    B::check_one_term(&*a, "vidutinius", "vidutin")?; // m. acc. pl.
    B::check_one_term(&*a, "vidutiniu", "vidutin")?; // m. ins. sg.
    B::check_one_term(&*a, "vidutiniais", "vidutin")?; // m. ins. pl.
    B::check_one_term(&*a, "vidutiniame", "vidutin")?; // m. loc. sg.
    B::check_one_term(&*a, "vidutiniuose", "vidutin")?; // m. loc. pl.
    B::check_one_term(&*a, "vidutin\u{117}", "vidutin")?; // f. nom. sing.
    B::check_one_term(&*a, "vidutin\u{117}s", "vidutin")?; // f. nom. pl.
    B::check_one_term(&*a, "vidutin\u{117}s", "vidutin")?; // f. gen. sg.
    B::check_one_term(&*a, "vidutini\u{173}", "vidutin")?; // f. gen. pl.
    B::check_one_term(&*a, "vidutinei", "vidutin")?; // f. dat. sg.
    B::check_one_term(&*a, "vidutin\u{117}ms", "vidutin")?; // f. dat. pl.
    B::check_one_term(&*a, "vidutin\u{119}", "vidutin")?; // f. acc. sg.
    B::check_one_term(&*a, "vidutines", "vidutin")?; // f. acc. pl.
    B::check_one_term(&*a, "vidutine", "vidutin")?; // f. ins. sg.
    B::check_one_term(&*a, "vidutin\u{117}mis", "vidutin")?; // f. ins. pl.
    B::check_one_term(&*a, "vidutin\u{117}je", "vidutin")?; // f. loc. sg.
    B::check_one_term(&*a, "vidutin\u{117}se", "vidutin")?; // f. loc. pl.
    Ok(())
}

fn lt_test_high_frequency_terms() -> Result<()> {
    let a = analyzer();
    B::check_one_term(&*a, "ir", "ir")?;
    B::check_one_term(&*a, "kad", "kad")?;
    B::check_one_term(&*a, "\u{12F}", "\u{12F}")?;
    B::check_one_term(&*a, "tai", "tai")?;
    B::check_one_term(&*a, "su", "su")?;
    B::check_one_term(&*a, "o", "o")?;
    B::check_one_term(&*a, "i\u{161}", "i\u{161}")?;
    B::check_one_term(&*a, "kaip", "kaip")?;
    B::check_one_term(&*a, "bet", "bet")?;
    B::check_one_term(&*a, "yra", "yr")?;
    B::check_one_term(&*a, "buvo", "buv")?;
    B::check_one_term(&*a, "tik", "tik")?;
    B::check_one_term(&*a, "ne", "ne")?;
    B::check_one_term(&*a, "taip", "taip")?;
    B::check_one_term(&*a, "ar", "ar")?;
    B::check_one_term(&*a, "dar", "dar")?;
    B::check_one_term(&*a, "jau", "jau")?;
    B::check_one_term(&*a, "savo", "sav")?;
    B::check_one_term(&*a, "apie", "ap")?;
    B::check_one_term(&*a, "kai", "kai")?;
    B::check_one_term(&*a, "a\u{161}", "a\u{161}")?;
    B::check_one_term(&*a, "per", "per")?;
    B::check_one_term(&*a, "nuo", "nuo")?;
    B::check_one_term(&*a, "po", "po")?;
    B::check_one_term(&*a, "jis", "jis")?;
    B::check_one_term(&*a, "kas", "kas")?;
    B::check_one_term(&*a, "d", "d")?;
    B::check_one_term(&*a, "labai", "lab")?;
    B::check_one_term(&*a, "man", "man")?;
    B::check_one_term(&*a, "d\u{117}l", "d\u{117}l")?;
    B::check_one_term(&*a, "ta\u{10D}iau", "tat")?;
    B::check_one_term(&*a, "nes", "nes")?;
    B::check_one_term(&*a, "u\u{17E}", "u\u{17E}")?;
    B::check_one_term(&*a, "to", "to")?;
    B::check_one_term(&*a, "jo", "jo")?;
    B::check_one_term(&*a, "iki", "ik")?;
    B::check_one_term(&*a, "k\u{105}", "k\u{105}")?;
    B::check_one_term(&*a, "mano", "man")?;
    B::check_one_term(&*a, "met\u{173}", "met")?;
    B::check_one_term(&*a, "nors", "nor")?;
    B::check_one_term(&*a, "jei", "jei")?;
    B::check_one_term(&*a, "bus", "bus")?;
    B::check_one_term(&*a, "j\u{173}", "j\u{173}")?;
    B::check_one_term(&*a, "\u{10D}ia", "\u{10D}ia")?;
    B::check_one_term(&*a, "dabar", "dabar")?;
    B::check_one_term(&*a, "Lietuvos", "Lietuv")?;
    B::check_one_term(&*a, "net", "net")?;
    B::check_one_term(&*a, "nei", "nei")?;
    B::check_one_term(&*a, "gali", "gal")?;
    B::check_one_term(&*a, "daug", "daug")?;
    B::check_one_term(&*a, "prie", "prie")?;
    B::check_one_term(&*a, "ji", "ji")?;
    B::check_one_term(&*a, "jos", "jos")?;
    B::check_one_term(&*a, "pat", "pat")?;
    B::check_one_term(&*a, "jie", "jie")?;
    B::check_one_term(&*a, "kur", "kur")?;
    B::check_one_term(&*a, "gal", "gal")?;
    B::check_one_term(&*a, "ant", "ant")?;
    B::check_one_term(&*a, "tiek", "tiek")?;
    B::check_one_term(&*a, "be", "be")?;
    B::check_one_term(&*a, "b\u{16B}ti", "b\u{16B}t")?;
    B::check_one_term(&*a, "bei", "bei")?;
    B::check_one_term(&*a, "daugiau", "daug")?;
    B::check_one_term(&*a, "turi", "tur")?;
    B::check_one_term(&*a, "prie\u{161}", "prie\u{161}")?;
    B::check_one_term(&*a, "vis", "vis")?;
    B::check_one_term(&*a, "b\u{16B}t\u{173}", "b\u{16B}t")?;
    B::check_one_term(&*a, "jog", "jog")?;
    B::check_one_term(&*a, "reikia", "reik")?;
    B::check_one_term(&*a, "m\u{16B}s\u{173}", "m\u{16B}s")?;
    B::check_one_term(&*a, "metu", "met")?;
    B::check_one_term(&*a, "galima", "galim")?;
    B::check_one_term(&*a, "n\u{117}ra", "n\u{117}r")?;
    B::check_one_term(&*a, "arba", "arb")?;
    B::check_one_term(&*a, "mes", "mes")?;
    B::check_one_term(&*a, "kurie", "kur")?;
    B::check_one_term(&*a, "tikrai", "tikr")?;
    B::check_one_term(&*a, "tod\u{117}l", "tod")?;
    B::check_one_term(&*a, "ten", "ten")?;
    B::check_one_term(&*a, "\u{161}iandien", "\u{161}iandien")?;
    B::check_one_term(&*a, "vienas", "vien")?;
    B::check_one_term(&*a, "visi", "vis")?;
    B::check_one_term(&*a, "kuris", "kur")?;
    B::check_one_term(&*a, "tada", "tad")?;
    B::check_one_term(&*a, "kiek", "kiek")?;
    B::check_one_term(&*a, "tuo", "tuo")?;
    B::check_one_term(&*a, "gerai", "ger")?;
    B::check_one_term(&*a, "nieko", "niek")?;
    B::check_one_term(&*a, "j\u{12F}", "j\u{12F}")?;
    B::check_one_term(&*a, "kol", "kol")?;
    B::check_one_term(&*a, "viskas", "visk")?;
    B::check_one_term(&*a, "mane", "man")?;
    B::check_one_term(&*a, "kart\u{105}", "kart")?;
    B::check_one_term(&*a, "m", "m")?;
    B::check_one_term(&*a, "tas", "tas")?;
    B::check_one_term(&*a, "sak\u{117}", "sak")?;
    B::check_one_term(&*a, "\u{17E}moni\u{173}", "\u{17E}mon")?;
    B::check_one_term(&*a, "tu", "tu")?;
    B::check_one_term(&*a, "dien\u{105}", "dien")?;
    B::check_one_term(&*a, "\u{17E}mon\u{117}s", "\u{17E}mon")?;
    B::check_one_term(&*a, "metais", "met")?;
    B::check_one_term(&*a, "vien\u{105}", "vien")?;
    B::check_one_term(&*a, "v\u{117}l", "v\u{117}l")?;
    B::check_one_term(&*a, "na", "na")?;
    B::check_one_term(&*a, "t\u{105}", "t\u{105}")?;
    B::check_one_term(&*a, "tiesiog", "tiesiog")?;
    B::check_one_term(&*a, "toks", "tok")?;
    B::check_one_term(&*a, "pats", "pat")?;
    B::check_one_term(&*a, "ko", "ko")?;
    B::check_one_term(&*a, "Lietuvoje", "Lietuv")?;
    B::check_one_term(&*a, "pagal", "pagal")?;
    B::check_one_term(&*a, "jeigu", "jeig")?;
    B::check_one_term(&*a, "visai", "vis")?;
    B::check_one_term(&*a, "viena", "vien")?;
    B::check_one_term(&*a, "\u{161}\u{12F}", "\u{161}\u{12F}")?;
    B::check_one_term(&*a, "metus", "met")?;
    B::check_one_term(&*a, "jam", "jam")?;
    B::check_one_term(&*a, "kod\u{117}l", "kod")?;
    B::check_one_term(&*a, "lit\u{173}", "lit")?;
    B::check_one_term(&*a, "j\u{105}", "j\u{105}")?;
    B::check_one_term(&*a, "kuri", "kur")?;
    B::check_one_term(&*a, "darbo", "darb")?;
    B::check_one_term(&*a, "tarp", "tarp")?;
    B::check_one_term(&*a, "juk", "juk")?;
    B::check_one_term(&*a, "laiko", "laik")?;
    B::check_one_term(&*a, "juos", "juos")?;
    B::check_one_term(&*a, "vis\u{105}", "vis")?;
    B::check_one_term(&*a, "kurios", "kur")?;
    B::check_one_term(&*a, "tam", "tam")?;
    B::check_one_term(&*a, "pas", "pas")?;
    B::check_one_term(&*a, "visk\u{105}", "visk")?;
    B::check_one_term(&*a, "Europos", "Eur")?;
    B::check_one_term(&*a, "atrodo", "atrod")?;
    B::check_one_term(&*a, "tad", "tad")?;
    B::check_one_term(&*a, "bent", "bent")?;
    B::check_one_term(&*a, "kit\u{173}", "kit")?;
    B::check_one_term(&*a, "\u{161}is", "\u{161}is")?;
    B::check_one_term(&*a, "Vilniaus", "Viln")?;
    B::check_one_term(&*a, "beveik", "bevei")?;
    B::check_one_term(&*a, "proc", "proc")?;
    B::check_one_term(&*a, "tokia", "tok")?;
    B::check_one_term(&*a, "\u{161}iuo", "\u{161}iuo")?;
    B::check_one_term(&*a, "du", "du")?;
    B::check_one_term(&*a, "kartu", "kart")?;
    B::check_one_term(&*a, "visada", "visad")?;
    B::check_one_term(&*a, "kuo", "kuo")?;
    Ok(())
}

fn lithuanian_stemming_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.lt.TestLithuanianStemming")
        .method(hook("setUp", lt_set_up))
        .method(hook("tearDown", lt_tear_down))
        .method(t("testNounsI", lt_test_nouns_i))
        .method(t("testNounsII", lt_test_nouns_ii))
        .method(t("testNounsIII", lt_test_nouns_iii))
        .method(t("testNounsIV", lt_test_nouns_iv))
        .method(t("testNounsV", lt_test_nouns_v))
        .method(t("testAdjI", lt_test_adj_i))
        .method(t("testAdjII", lt_test_adj_ii))
        .method(t("testAdjIII", lt_test_adj_iii))
        .method(t("testHighFrequencyTerms", lt_test_high_frequency_terms))
        .build()
}

#[test]
fn test_snowball() {
    run(snowball_class);
}

#[test]
fn test_snowball_porter_filter_factory() {
    run(snowball_porter_filter_factory_class);
}

#[test]
fn test_keyword_repeat_filter() {
    run(keyword_repeat_filter_class);
}

#[test]
fn test_lithuanian_stemming() {
    run(lithuanian_stemming_class);
}
