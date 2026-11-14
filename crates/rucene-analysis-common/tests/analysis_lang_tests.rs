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

//! Port of the test classes of the per-language packages of Lucene 10.5.0
//! `lucene/analysis/common/src/test` that rmp tasks #468 (`ca`, `da`, `nl`,
//! `fr`, `gl`, `de`, `it`, `no`, `pt`, `es`, `sv`), #469 (`br`, `el`, `id`,
//! `ga`, `ro`, `tr`), #470 (`hy`, `eu`, `bg`, `cz`, `et`, `fi`, `hu`, `lv`,
//! `lt`, `ru`, `sr`) and #471 (`ar`, `bn`, `hi`, `in`, `ne`, `fa`, `ckb`,
//! `ta`, `te`, and `analysis.miscellaneous.TestConditionalTokenFilter`)
//! list, each run as a suite by the ported runner; #468 also lists
//! `analysis.util.TestElision`, ported here.
//!
//! The regular classes are translated statement for statement by
//! `tools/translate/lang_tests.py` into [`analysis_lang_generated`]; this
//! file holds the classes it rejects, ported by hand, and the helpers both
//! share ([`lang_support`]). `lt.TestLithuanianStemming` belongs to rmp
//! #455 and is ported in `analysis_snowball_tests.rs`.

mod analysis_lang_generated;
mod support;

/// Helpers of the generated and the hand-ported suites.
pub mod lang_support {
    use std::path::PathBuf;
    use std::sync::Arc;

    use randomizedtesting::jdk::random::Random;
    use rucene::analysis::factories::CharFilterFactory;
    use rucene::analysis::CharArraySet;
    use rucene::error::Result;
    use rucene_test_framework::analysis::BaseTokenStreamFactoryTestCase;
    use rucene_test_framework::util::lucene_test_case::RANDOM_MULTIPLIER;
    use rucene_test_framework::util::LuceneTestCase;

    use crate::support::test_class_loader;

    /// `random()`.
    pub fn random() -> Arc<dyn Random> {
        LuceneTestCase::random()
    }

    /// `RANDOM_MULTIPLIER`.
    pub fn mult() -> i32 {
        *RANDOM_MULTIPLIER
    }

    /// `new CharArraySet(asSet(words...), ignoreCase)`.
    pub fn char_set(words: &[&str], ignore_case: bool) -> CharArraySet {
        CharArraySet::from_iter(words.iter().copied(), ignore_case)
    }

    /// The directory of the test resources of package `package`, copied
    /// byte for byte from `lucene/analysis/common/src/test`.
    fn resource_dir(package: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/resources")
            .join(package.replace('.', "/"))
    }

    /// `getDataPath(name)` of a test class of `package`.
    pub fn data_path(package: &str, name: &str) -> Result<PathBuf> {
        LuceneTestCase::get_data_path(&resource_dir(package), name)
    }

    /// `getClass().getResourceAsStream(name)` of a test class of `package`,
    /// for a resource the test class has (Java returns `null` otherwise,
    /// which no ported test reads).
    pub fn resource_stream(package: &str, name: &str) -> Result<std::fs::File> {
        Ok(std::fs::File::open(resource_dir(package).join(name))?)
    }

    /// `charFilterFactory(name, keysAndValues...)` of a test class of
    /// `package`.
    pub fn char_filter_factory(
        package: &str,
        name: &str,
        keys_and_values: &[&str],
    ) -> Result<Arc<dyn CharFilterFactory>> {
        BaseTokenStreamFactoryTestCase::char_filter_factory_with_loader(
            name,
            Some(rucene::util::extra::Version::LATEST),
            &test_class_loader(package),
            keys_and_values,
        )
    }
}

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::character_utils::to_lower_case;
use rucene::analysis::filtering_token_filter::{
    FilteringTokenFilterAdapter, FilteringTokenFilterLogic,
};
use rucene::analysis::java_regex::JavaPattern;
use rucene::analysis::standard::new_standard_tokenizer;
use rucene::analysis::tokenattributes::{
    CharTermAttribute, OffsetAttribute, PositionIncrementAttribute,
};
use rucene::analysis::{
    Analyzer, CharArraySet, CharReader, TokenFilter, TokenFilterLogic, TokenStream,
};
use rucene::error::Result;
use rucene::util::attribute::{AttributeSource, CapturedState};
use rucene_analysis_common::analysis::bn::bengali_normalization_filter::new_bengali_normalization_filter;
use rucene_analysis_common::analysis::bn::bengali_normalizer::BengaliNormalizer;
use rucene_analysis_common::analysis::classic::classic_tokenizer::new_classic_tokenizer;
use rucene_analysis_common::analysis::core::keyword_tokenizer::new_keyword_tokenizer;
use rucene_analysis_common::analysis::core::type_token_filter::new_type_token_filter;
use rucene_analysis_common::analysis::de::german_stem_filter::new_german_stem_filter;
use rucene_analysis_common::analysis::fa::persian_char_filter::PersianCharFilter;
use rucene_analysis_common::analysis::miscellaneous::conditional_token_filter::{
    new_conditional_token_filter, ConditionalTokenFilter, ConditionalTokenFilterLogic,
};
use rucene_analysis_common::analysis::miscellaneous::keyword_repeat_filter::new_keyword_repeat_filter;
use rucene_analysis_common::analysis::miscellaneous::length_filter::new_length_filter;
use rucene_analysis_common::analysis::miscellaneous::protected_term_filter::new_protected_term_filter;
use rucene_analysis_common::analysis::miscellaneous::truncate_token_filter::truncate_after_code_points;
use rucene_analysis_common::analysis::ngram::ngram_tokenizer::new_ngram_tokenizer_default;
use rucene_analysis_common::analysis::r#in::indic_normalization_filter::new_indic_normalization_filter;
use rucene_analysis_common::analysis::shingle::fixed_shingle_filter::FixedShingleFilter;
use rucene_analysis_common::analysis::shingle::shingle_filter::new_shingle_filter_with_max;
use rucene_analysis_common::analysis::synonym::solr_synonym_parser::SolrSynonymParser;
use rucene_analysis_common::analysis::synonym::synonym_graph_filter::new_synonym_graph_filter;
use rucene_analysis_common::analysis::synonym::synonym_map::Parser as SynonymMapParser;
use rucene_test_framework::analysis::mock_analyzer::{BoxedStream, MockAnalyzer};
use rucene_test_framework::analysis::mock_tokenizer::{MockTokenizer, WHITESPACE};
use rucene_test_framework::analysis::token::Token;
use rucene_test_framework::analysis::validating_token_filter::ValidatingTokenFilter;
use rucene_test_framework::analysis::{
    components, shared, sink, BaseTokenStreamTestCase, CannedTokenStream,
};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use lang_support::{mult, random};
use support::{boxed, btstc_class, hook, own, run, t, FnAnalyzer};

type B = BaseTokenStreamTestCase;

/// `termAtt.toString()` of `source`.
fn term_of(source: &AttributeSource) -> String {
    let a = source
        .get_attribute_interface::<dyn CharTermAttribute>()
        .expect("termAtt");
    a.buffer()[..a.length()].iter().collect()
}

// -----------------------------------------------------------------------------
// TestBengaliNormalizer (the translator rejects its `for` loop)
// -----------------------------------------------------------------------------

mod test_bengali_normalizer {
    use super::*;

    /// Test some basic normalization, with an example from the paper.
    fn test_chndrobindu() -> Result<()> {
        check("\u{99A}\u{9BE}\u{981}\u{9A6}", "\u{99A}\u{9BE}\u{9A6}")?;
        Ok(())
    }

    fn test_rossho_i_kar() -> Result<()> {
        check(
            "\u{9AC}\u{9BE}\u{9DC}\u{9C0}",
            "\u{9AC}\u{9BE}\u{9B0}\u{9BF}",
        )?;
        check("\u{9A4}\u{9C0}\u{9B0}", "\u{9A4}\u{9BF}\u{9B0}")?;
        Ok(())
    }

    fn test_rossho_u_kar() -> Result<()> {
        check("\u{9AD}\u{9C2}\u{9B2}", "\u{9AD}\u{9C1}\u{9B2}")?;
        check(
            "\u{985}\u{9A8}\u{9C2}\u{9AA}",
            "\u{985}\u{9A8}\u{9C1}\u{9AA}",
        )?;
        Ok(())
    }

    fn test_nga() -> Result<()> {
        check(
            "\u{9AC}\u{9BE}\u{999}\u{9B2}\u{9BE}",
            "\u{9AC}\u{9BE}\u{982}\u{9B2}\u{9BE}",
        )?;
        Ok(())
    }

    fn test_ja_phaala() -> Result<()> {
        check(
            "\u{9AC}\u{9CD}\u{9AF}\u{9BE}\u{995}\u{9CD}\u{9A4}\u{9BF}",
            "\u{9AC}\u{9C7}\u{995}\u{9CD}\u{9A4}\u{9BF}",
        )?;
        check(
            "\u{9B8}\u{9A8}\u{9CD}\u{9A7}\u{9CD}\u{9AF}\u{9BE}",
            "\u{9B8}\u{9A8}\u{9CD}\u{9A7}\u{9BE}",
        )?;
        Ok(())
    }

    fn test_ba_phalaa() -> Result<()> {
        check(
            "\u{9B8}\u{9CD}\u{9AC}\u{9A6}\u{9C7}\u{9B6}",
            "\u{9B8}\u{9A6}\u{9C7}\u{9B8}",
        )?;
        check(
            "\u{9A4}\u{9A4}\u{9CD}\u{9A4}\u{9CD}\u{9AC}",
            "\u{9A4}\u{9A4}\u{9CD}\u{9A4}",
        )?;
        check(
            "\u{9AC}\u{9BF}\u{9B6}\u{9CD}\u{9AC}",
            "\u{9AC}\u{9BF}\u{9B8}\u{9B8}",
        )?;
        Ok(())
    }

    fn test_visarga() -> Result<()> {
        check(
            "\u{9A6}\u{9C1}\u{983}\u{996}",
            "\u{9A6}\u{9C1}\u{996}\u{996}",
        )?;
        check("\u{989}\u{983}", "\u{989}\u{9B9}")?;
        check("\u{9AA}\u{9C1}\u{9A8}\u{983}", "\u{9AA}\u{9C1}\u{9A8}")?;
        Ok(())
    }

    fn test_basics() -> Result<()> {
        check("\u{995}\u{9A3}\u{9BE}", "\u{995}\u{9A8}\u{9BE}")?;
        check(
            "\u{9B6}\u{9B0}\u{9C0}\u{9B0}",
            "\u{9B8}\u{9B0}\u{9BF}\u{9B0}",
        )?;
        check(
            "\u{9AC}\u{9BE}\u{9DC}\u{9BF}",
            "\u{9AC}\u{9BE}\u{9B0}\u{9BF}",
        )?;
        Ok(())
    }

    /// creates random strings in the bengali block and ensures the normalizer
    /// doesn't trip up on them
    fn test_random() -> Result<()> {
        let normalizer = BengaliNormalizer::new();
        for _ in 0..100000 {
            let random_bengali =
                TestUtil::random_simple_string_range(&*random(), 0x0980, 0x09FF, 7);
            let mut chars: Vec<u16> = random_bengali.encode_utf16().collect();
            let len = chars.len();
            // a panic here is Java's exception: report the input as Java does
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                normalizer.normalize(&mut chars, len)
            }));
            match outcome {
                Ok(new_len) => {
                    // should not return negative length (usize cannot be)
                    Assert::assert_true(new_len <= len); // should not increase length of string
                }
                Err(e) => {
                    eprintln!(
                        "normalizer failed on input: '{random_bengali}' ({})",
                        B::escape(&random_bengali)
                    );
                    std::panic::resume_unwind(e);
                }
            }
        }
        Ok(())
    }

    fn check(input: &str, output: &str) -> Result<()> {
        let tokenizer = B::whitespace_mock_tokenizer_str(input)?;
        let mut tf = new_bengali_normalization_filter(boxed(tokenizer))?;
        B::assert_token_stream_contents(&mut tf, &[output])?;
        Ok(())
    }

    fn test_empty_term() -> Result<()> {
        let a = FnAnalyzer::new(
            "org.apache.lucene.analysis.bn.TestBengaliNormalizer$1",
            move |_| {
                let tokenizer = own(new_keyword_tokenizer());
                let stream = boxed(new_bengali_normalization_filter(shared(&tokenizer))?);
                Ok(components(tokenizer, sink(BoxedStream(stream))))
            },
        )
        .arc();
        B::check_one_term(&*a, "", "")?;
        a.close();
        Ok(())
    }

    pub fn class() -> Arc<Class> {
        btstc_class("org.apache.lucene.analysis.bn.TestBengaliNormalizer")
            .method(t("testChndrobindu", test_chndrobindu))
            .method(t("testRosshoIKar", test_rossho_i_kar))
            .method(t("testRosshoUKar", test_rossho_u_kar))
            .method(t("testNga", test_nga))
            .method(t("testJaPhaala", test_ja_phaala))
            .method(t("testBaPhalaa", test_ba_phalaa))
            .method(t("testVisarga", test_visarga))
            .method(t("testBasics", test_basics))
            .method(t("testRandom", test_random))
            .method(t("testEmptyTerm", test_empty_term))
            .build()
    }
}

#[test]
fn test_bengali_normalizer() {
    run(test_bengali_normalizer::class);
}

// -----------------------------------------------------------------------------
// TestPersianCharFilter (its anonymous analyzer overrides initReader)
// -----------------------------------------------------------------------------

mod test_persian_char_filter {
    use super::*;
    use std::cell::RefCell;

    thread_local! {
        /// The `analyzer` field.
        static ANALYZER: RefCell<Option<Arc<dyn Analyzer>>> = const { RefCell::new(None) };
    }

    fn analyzer() -> Arc<dyn Analyzer> {
        ANALYZER.with(|f| Arc::clone(f.borrow().as_ref().expect("setUp")))
    }

    fn set_up() -> Result<()> {
        LuceneTestCase::super_set_up();
        let a = FnAnalyzer::new(
            "org.apache.lucene.analysis.fa.TestPersianCharFilter$1",
            |_| {
                let tokenizer = own(MockTokenizer::new(Arc::clone(&WHITESPACE), true)?);
                let sink_: Arc<rucene::util::atomic_ref_cell::AtomicRefCell<dyn TokenStream>> =
                    tokenizer.clone();
                Ok(components(tokenizer, sink_))
            },
        )
        .init_reader(|_, reader| Box::new(PersianCharFilter::new(reader)) as Box<dyn CharReader>)
        .arc();
        ANALYZER.with(|f| *f.borrow_mut() = Some(a));
        Ok(())
    }

    fn tear_down() -> Result<()> {
        analyzer().close();
        LuceneTestCase::super_tear_down();
        Ok(())
    }

    fn test_basics() -> Result<()> {
        B::assert_analyzes_to(
            &*analyzer(),
            "this is a\u{200C}test",
            &["this", "is", "a", "test"],
        )
    }

    /// blast some random strings through the analyzer
    fn test_random_strings() -> Result<()> {
        B::check_random_data(&*random(), &analyzer(), 200 * mult())
    }

    pub fn class() -> Arc<Class> {
        btstc_class("org.apache.lucene.analysis.fa.TestPersianCharFilter")
            .method(hook("setUp", set_up))
            .method(hook("tearDown", tear_down))
            .method(t("testBasics", test_basics))
            .method(t("testRandomStrings", test_random_strings))
            .build()
    }
}

#[test]
fn test_persian_char_filter() {
    run(test_persian_char_filter::class);
}

// -----------------------------------------------------------------------------
// TestConditionalTokenFilter (org.apache.lucene.analysis.miscellaneous)
// -----------------------------------------------------------------------------

mod test_conditional_token_filter {
    use super::*;

    /// The test instance's `closed`, `ended` and `reset` fields, which the
    /// inner class `AssertingLowerCaseFilter` sets (JUnit makes a new instance
    /// per test method: each test makes its own).
    #[derive(Debug, Default)]
    struct Instance {
        closed: AtomicBool,
        ended: AtomicBool,
        reset: AtomicBool,
    }

    /// `AssertingLowerCaseFilter`, an inner class of the test.
    #[derive(Debug)]
    struct AssertingLowerCaseFilterLogic {
        this: Arc<Instance>,
    }

    impl TokenFilterLogic for AssertingLowerCaseFilterLogic {
        fn increment_token(
            &mut self,
            source: &mut AttributeSource,
            input: &mut dyn TokenStream,
        ) -> Result<bool> {
            if input.increment_token()? {
                let mut term_att = source
                    .get_attribute_interface_mut::<dyn CharTermAttribute>()
                    .expect("termAtt");
                let length = term_att.length();
                to_lower_case(term_att.buffer_mut(), 0, length);
                Ok(true)
            } else {
                Ok(false)
            }
        }

        fn end(
            &mut self,
            _source: &mut AttributeSource,
            input: &mut dyn TokenStream,
        ) -> Result<()> {
            input.end()?;
            self.this.ended.store(true, Ordering::SeqCst);
            Ok(())
        }

        fn close(
            &mut self,
            _source: &mut AttributeSource,
            input: &mut dyn TokenStream,
        ) -> Result<()> {
            input.close()?;
            self.this.closed.store(true, Ordering::SeqCst);
            Ok(())
        }

        fn reset(
            &mut self,
            _source: &mut AttributeSource,
            input: &mut dyn TokenStream,
        ) -> Result<()> {
            input.reset()?;
            self.this.reset.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    /// `new AssertingLowerCaseFilter(in)`.
    fn asserting_lower_case_filter(
        this: &Arc<Instance>,
        input: Box<dyn TokenStream>,
    ) -> Result<Box<dyn TokenStream>> {
        let mut f = TokenFilter::new(
            input,
            AssertingLowerCaseFilterLogic {
                this: Arc::clone(this),
            },
        );
        f.attribute_source_mut()
            .add_attribute_interface::<dyn CharTermAttribute>()?;
        Ok(boxed(f))
    }

    /// `SkipMatchingFilter.shouldFilter()`.
    #[derive(Debug)]
    struct SkipMatchingLogic {
        pattern: JavaPattern,
    }

    impl ConditionalTokenFilterLogic for SkipMatchingLogic {
        fn should_filter(&mut self, source: &AttributeSource) -> Result<bool> {
            Ok(!self.pattern.matches(&term_of(source)))
        }
    }

    /// `new SkipMatchingFilter(input, inputFactory, termRegex)`.
    fn skip_matching_filter(
        input: Box<dyn TokenStream>,
        input_factory: impl FnOnce(Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>>,
        term_regex: &str,
    ) -> Result<Box<dyn TokenStream>> {
        let pattern = JavaPattern::compile(term_regex, 0).expect("a valid pattern");
        let mut f =
            new_conditional_token_filter(input, input_factory, SkipMatchingLogic { pattern })?;
        f.attribute_source_mut()
            .add_attribute_interface::<dyn CharTermAttribute>()?;
        Ok(boxed(f))
    }

    fn test_simple() -> Result<()> {
        let this = Arc::new(Instance::default());
        let stream = B::whitespace_mock_tokenizer_str("Alice Bob Clara David")?;
        let this2 = Arc::clone(&this);
        let mut t = skip_matching_filter(
            boxed(stream),
            move |input| asserting_lower_case_filter(&this2, input),
            ".*o.*",
        )?;
        B::assert_token_stream_contents(&mut *t, &["alice", "Bob", "clara", "david"])?;
        Assert::assert_true(this.closed.load(Ordering::SeqCst));
        Assert::assert_true(this.reset.load(Ordering::SeqCst));
        Assert::assert_true(this.ended.load(Ordering::SeqCst));
        Ok(())
    }

    /// `TokenSplitter`.
    #[derive(Debug, Default)]
    struct TokenSplitterLogic {
        state: Option<CapturedState>,
        half: Option<String>,
    }

    impl TokenFilterLogic for TokenSplitterLogic {
        fn increment_token(
            &mut self,
            source: &mut AttributeSource,
            input: &mut dyn TokenStream,
        ) -> Result<bool> {
            match self.half.take() {
                None => {
                    self.state = source.capture_state();
                    if !input.increment_token()? {
                        return Ok(false);
                    }
                    let term = term_of(source);
                    // `termAtt.toString().substring(4)` (ASCII terms)
                    self.half = Some(term.chars().skip(4).collect());
                    source
                        .get_attribute_interface_mut::<dyn CharTermAttribute>()
                        .expect("termAtt")
                        .set_length(4);
                    Ok(true)
                }
                Some(half) => {
                    if let Some(state) = &self.state {
                        source.restore_state(state)?;
                    }
                    let mut term_att = source
                        .get_attribute_interface_mut::<dyn CharTermAttribute>()
                        .expect("termAtt");
                    term_att.set_empty();
                    term_att.append_string(Some(&half));
                    Ok(true)
                }
            }
        }
    }

    /// `new TokenSplitter(input)`.
    fn token_splitter(input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        let mut f = TokenFilter::new(input, TokenSplitterLogic::default());
        f.attribute_source_mut()
            .add_attribute_interface::<dyn CharTermAttribute>()?;
        Ok(boxed(f))
    }

    fn test_multitoken_wrapping() -> Result<()> {
        let stream = B::whitespace_mock_tokenizer_str("tokenpos1 tokenpos2 tokenpos3 tokenpos4")?;
        let mut ts = skip_matching_filter(boxed(stream), token_splitter, ".*2.*")?;
        B::assert_token_stream_contents(
            &mut *ts,
            &[
                "toke",
                "npos1",
                "tokenpos2",
                "toke",
                "npos3",
                "toke",
                "npos4",
            ],
        )
    }

    /// `EndTrimmingFilter extends FilteringTokenFilter`: accepts every token
    /// and overrides `end()`.
    #[derive(Debug)]
    struct AcceptAll;

    impl FilteringTokenFilterLogic for AcceptAll {
        fn accept(&mut self, _source: &AttributeSource) -> Result<bool> {
            Ok(true)
        }
    }

    #[derive(Debug)]
    struct EndTrimmingLogic {
        filtering: FilteringTokenFilterAdapter<AcceptAll>,
    }

    impl TokenFilterLogic for EndTrimmingLogic {
        fn increment_token(
            &mut self,
            source: &mut AttributeSource,
            input: &mut dyn TokenStream,
        ) -> Result<bool> {
            self.filtering.increment_token(source, input)
        }

        fn end(&mut self, source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
            self.filtering.end(source, input)?;
            let mut offset_att = source
                .get_attribute_interface_mut::<dyn OffsetAttribute>()
                .expect("offsetAtt");
            let end = offset_att.end_offset();
            offset_att.set_offset(0, end - 2);
            Ok(())
        }

        fn reset(
            &mut self,
            source: &mut AttributeSource,
            input: &mut dyn TokenStream,
        ) -> Result<()> {
            self.filtering.reset(source, input)
        }
    }

    /// `new EndTrimmingFilter(in)`: `FilteringTokenFilter`'s constructor
    /// adds the `PositionIncrementAttribute`, the field initializer the
    /// `OffsetAttribute`.
    fn end_trimming_filter(input: Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>> {
        let mut f = TokenFilter::new(
            input,
            EndTrimmingLogic {
                filtering: FilteringTokenFilterAdapter::new(AcceptAll),
            },
        );
        f.attribute_source_mut()
            .add_attribute_interface::<dyn PositionIncrementAttribute>()?;
        f.attribute_source_mut()
            .add_attribute_interface::<dyn OffsetAttribute>()?;
        Ok(boxed(f))
    }

    /// An anonymous `ConditionalTokenFilter` whose `shouldFilter()` returns
    /// a constant.
    #[derive(Debug)]
    struct Constant(bool);

    impl ConditionalTokenFilterLogic for Constant {
        fn should_filter(&mut self, _source: &AttributeSource) -> Result<bool> {
            Ok(self.0)
        }
    }

    fn test_end_propagation() -> Result<()> {
        let cts2 = CannedTokenStream::with_final(
            0,
            20,
            vec![
                Token::with_offsets("alice", 0, 5),
                Token::with_offsets("bob", 6, 8),
            ],
        );
        let mut ts2 =
            new_conditional_token_filter(boxed(cts2), end_trimming_filter, Constant(true))?;
        B::assert_token_stream_contents_final_offset(
            &mut ts2,
            &["alice", "bob"],
            None,
            None,
            None,
            None,
            None,
            Some(18),
        )?;

        let cts1 = CannedTokenStream::with_final(
            0,
            20,
            vec![
                Token::with_offsets("alice", 0, 5),
                Token::with_offsets("bob", 6, 8),
            ],
        );
        let mut ts1 =
            new_conditional_token_filter(boxed(cts1), end_trimming_filter, Constant(false))?;
        B::assert_token_stream_contents_final_offset(
            &mut ts1,
            &["alice", "bob"],
            None,
            None,
            None,
            None,
            None,
            Some(20),
        )
    }

    fn test_wrap_graphs() -> Result<()> {
        let stream = B::whitespace_mock_tokenizer_str("a b c d e")?;

        let sm = {
            let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::new(&*random()));
            let mut parser = SolrSynonymParser::new(true, true, Arc::clone(&analyzer));
            let mut r = rucene::analysis::StringCharReader::new("a b, f\nc d, g");
            let parsed = parser.parse(&mut r as &mut dyn CharReader);
            let built = parsed.and_then(|()| SynonymMapParser::build(&mut parser));
            analyzer.close();
            Arc::new(built?)
        };

        let mut ts = skip_matching_filter(
            boxed(stream),
            move |input| {
                Ok(boxed(new_synonym_graph_filter(
                    input,
                    Arc::clone(&sm),
                    true,
                )?))
            },
            "c",
        )?;

        B::assert_token_stream_contents_final_offset(
            &mut *ts,
            &["f", "a", "b", "c", "d", "e"],
            None,
            None,
            None,
            Some(&[1, 0, 1, 1, 1, 1]),
            Some(&[2, 1, 1, 1, 1, 1]),
            None,
        )
    }

    /// `analyzer.tokenStream("", input)` asserted to produce `output` (with
    /// the optional offsets, increments, lengths and final offset), then
    /// closed (the `try`-with-resources).
    #[allow(clippy::too_many_arguments)]
    fn assert_analyzer_stream(
        analyzer: &dyn Analyzer,
        input: &str,
        output: &[&str],
        start_offsets: Option<&[i32]>,
        end_offsets: Option<&[i32]>,
        pos_increments: Option<&[i32]>,
        pos_lengths: Option<&[i32]>,
        final_offset: Option<i32>,
    ) -> Result<()> {
        let ts =
            analyzer.token_stream("", Box::new(rucene::analysis::StringCharReader::new(input)))?;
        let result = B::assert_token_stream_contents_final_offset(
            &mut *ts.borrow_mut(),
            output,
            start_offsets,
            end_offsets,
            None,
            pos_increments,
            pos_lengths,
            final_offset,
        );
        let closed = ts.borrow_mut().close();
        result.and(closed)
    }

    fn test_readahead_with_no_filtering() -> Result<()> {
        let analyzer = FnAnalyzer::new(
            "org.apache.lucene.analysis.miscellaneous.TestConditionalTokenFilter$1",
            |_| {
                let source = own(new_classic_tokenizer());
                let sink_ = new_conditional_token_filter(
                    shared(&source),
                    |input| Ok(boxed(new_shingle_filter_with_max(input, 2)?)),
                    Constant(true),
                )?;
                Ok(components(source, sink(BoxedStream(boxed(sink_)))))
            },
        );

        let input = "one two three four";

        assert_analyzer_stream(
            &analyzer,
            input,
            &[
                "one",
                "one two",
                "two",
                "two three",
                "three",
                "three four",
                "four",
            ],
            None,
            None,
            None,
            None,
            None,
        )
    }

    fn test_readahead_with_filtering() -> Result<()> {
        let mut protected_terms = CharArraySet::new(2, true);
        protected_terms.add("three");

        let analyzer = FnAnalyzer::new(
            "org.apache.lucene.analysis.miscellaneous.TestConditionalTokenFilter$2",
            move |_| {
                let source = own(new_classic_tokenizer());
                let sink_ = boxed(new_protected_term_filter(
                    Some(protected_terms.clone()),
                    shared(&source),
                    |input| Ok(boxed(new_shingle_filter_with_max(input, 2)?)),
                )?);
                let sink_ = boxed(ValidatingTokenFilter::new(sink_, "1"));
                Ok(components(source, sink(BoxedStream(sink_))))
            },
        );

        let input = "one two three four";

        assert_analyzer_stream(
            &analyzer,
            input,
            &["one", "one two", "two", "three", "four"],
            Some(&[0, 0, 4, 8, 14]),
            Some(&[3, 7, 7, 13, 18]),
            Some(&[1, 0, 1, 1, 1]),
            Some(&[1, 2, 1, 1, 1]),
            Some(18),
        )
    }

    /// `Collections.singleton(type)`.
    fn singleton(type_: &str) -> HashSet<String> {
        HashSet::from([type_.to_string()])
    }

    fn test_filtering_with_readahead() -> Result<()> {
        let mut protected_terms = CharArraySet::new(2, true);
        protected_terms.add("two");
        protected_terms.add("two three");

        let analyzer = FnAnalyzer::new(
            "org.apache.lucene.analysis.miscellaneous.TestConditionalTokenFilter$3",
            move |_| {
                let source = own(new_standard_tokenizer());
                let sink_ = boxed(new_shingle_filter_with_max(shared(&source), 3)?);
                let sink_ = boxed(new_protected_term_filter(
                    Some(protected_terms.clone()),
                    sink_,
                    |input| Ok(boxed(new_type_token_filter(input, singleton("ALL"), true)?)),
                )?);
                Ok(components(source, sink(BoxedStream(sink_))))
            },
        );

        let input = "one two three four";

        assert_analyzer_stream(
            &analyzer,
            input,
            &["two", "two three"],
            Some(&[4, 4]),
            Some(&[7, 13]),
            Some(&[2, 0]),
            Some(&[1, 2]),
            Some(18),
        )
    }

    fn test_multiple_conditional_filters() -> Result<()> {
        let this = Arc::new(Instance::default());
        let stream = B::whitespace_mock_tokenizer_str("Alice Bob Clara David")?;
        let this2 = Arc::clone(&this);
        let mut t = skip_matching_filter(
            boxed(stream),
            move |input| {
                let truncate_filter = truncate_after_code_points(input, 2)?;
                asserting_lower_case_filter(&this2, boxed(truncate_filter))
            },
            ".*o.*",
        )?;

        B::assert_token_stream_contents(&mut *t, &["al", "Bob", "cl", "da"])?;
        Assert::assert_true(this.closed.load(Ordering::SeqCst));
        Assert::assert_true(this.reset.load(Ordering::SeqCst));
        Assert::assert_true(this.ended.load(Ordering::SeqCst));
        Ok(())
    }

    fn test_filtered_token_filters() -> Result<()> {
        let mut protected_terms = CharArraySet::new(2, true);
        protected_terms.add("foobar");

        let ts = B::whitespace_mock_tokenizer_str("wuthering foobar abc")?;
        let mut ts =
            new_protected_term_filter(Some(protected_terms.clone()), boxed(ts), |input| {
                Ok(boxed(new_length_filter(input, 1, 4)?))
            })?;
        B::assert_token_stream_contents(&mut ts, &["foobar", "abc"])?;

        let ts = B::whitespace_mock_tokenizer_str("foobar abc")?;
        let mut ts =
            new_protected_term_filter(Some(protected_terms.clone()), boxed(ts), |input| {
                Ok(boxed(new_length_filter(input, 1, 4)?))
            })?;
        B::assert_token_stream_contents(&mut ts, &["foobar", "abc"])
    }

    fn test_consistent_offsets() -> Result<()> {
        let seed = random().next_long();
        let analyzer = FnAnalyzer::new(
            "org.apache.lucene.analysis.miscellaneous.TestConditionalTokenFilter$4",
            move |_| {
                let source = own(new_ngram_tokenizer_default());
                let sink_ = boxed(ValidatingTokenFilter::new(
                    boxed(new_keyword_repeat_filter(shared(&source))?),
                    "stage 0",
                ));
                let sink_ = boxed(ValidatingTokenFilter::new(sink_, "stage 1"));
                let sink_ = random_skipping_filter(sink_, seed, |input| {
                    Ok(boxed(new_type_token_filter(
                        input,
                        singleton("word"),
                        false,
                    )?))
                })?;
                let sink_ = boxed(ValidatingTokenFilter::new(sink_, "last stage"));
                Ok(components(source, sink(BoxedStream(sink_))))
            },
        )
        .arc();

        B::check_random_data(&*random(), &analyzer, 1)
    }

    fn test_end_with_shingles() -> Result<()> {
        let ts = B::whitespace_mock_tokenizer_str("cyk jvboq \u{092e}\u{0962}\u{093f}")?;
        let ts = boxed(new_german_stem_filter(boxed(ts))?);
        let ts = non_random_skipping_filter(
            ts,
            |input| Ok(boxed(FixedShingleFilter::new(input, 2)?)),
            &[true, false, true],
        )?;
        let mut ts = non_random_skipping_filter(
            ts,
            |input| Ok(boxed(new_indic_normalization_filter(input)?)),
            &[true],
        )?;

        B::assert_token_stream_contents(&mut *ts, &["jvboq"])
    }

    fn test_internal_position_adjustment() -> Result<()> {
        // check that the partial TokenStream sent to the condition filter begins with a posInc of 1,
        // even if the input stream has a posInc of 0 at that position, and that the filtered stream
        // has the correct posInc afterwards
        let ts = B::whitespace_mock_tokenizer_str("one two three")?;
        let ts = boxed(new_keyword_repeat_filter(boxed(ts))?);
        let mut ts = non_random_skipping_filter(
            ts,
            position_asserting_token_filter,
            &[false, true, true, true, true, false],
        )?;

        B::assert_token_stream_contents_pos_increments(
            &mut *ts,
            &["one", "one", "two", "two", "three", "three"],
            &[1, 0, 1, 0, 1, 0],
        )
    }

    /// `PositionAssertingTokenFilter`.
    #[derive(Debug, Default)]
    struct PositionAssertingLogic {
        reset: bool,
    }

    impl TokenFilterLogic for PositionAssertingLogic {
        fn reset(
            &mut self,
            _source: &mut AttributeSource,
            input: &mut dyn TokenStream,
        ) -> Result<()> {
            input.reset()?;
            self.reset = true;
            Ok(())
        }

        fn increment_token(
            &mut self,
            source: &mut AttributeSource,
            input: &mut dyn TokenStream,
        ) -> Result<bool> {
            let more = input.increment_token()?;
            if more && self.reset {
                let pos_inc = source
                    .get_attribute_interface::<dyn PositionIncrementAttribute>()
                    .expect("posIncAtt")
                    .get_position_increment();
                Assert::assert_equals(1, pos_inc);
            }
            self.reset = false;
            Ok(more)
        }
    }

    /// `PositionAssertingTokenFilter::new`.
    fn position_asserting_token_filter(
        input: Box<dyn TokenStream>,
    ) -> Result<Box<dyn TokenStream>> {
        let mut f = TokenFilter::new(input, PositionAssertingLogic::default());
        f.attribute_source_mut()
            .add_attribute_interface::<dyn PositionIncrementAttribute>()?;
        Ok(boxed(f))
    }

    /// A `ConditionalTokenFilter` subclass that overrides `reset()`: Rust's
    /// `ConditionalTokenFilterLogic` carries only `shouldFilter()`, so the
    /// subclass is the filter wrapped in a stream whose `reset()` calls the
    /// filter's (`super.reset()`) and then the subclass's own reset of the
    /// state it shares with its `shouldFilter()`.
    #[derive(Debug)]
    struct ResettingConditionalFilter<L: ConditionalTokenFilterLogic + 'static> {
        inner: ConditionalTokenFilter<L>,
        on_reset: fn(&L),
    }

    impl<L: ConditionalTokenFilterLogic + 'static> TokenStream for ResettingConditionalFilter<L> {
        fn increment_token(&mut self) -> Result<bool> {
            self.inner.increment_token()
        }
        fn end(&mut self) -> Result<()> {
            self.inner.end()
        }
        fn reset(&mut self) -> Result<()> {
            self.inner.reset()?;
            self.inner.with_logic(self.on_reset);
            Ok(())
        }
        fn close(&mut self) -> Result<()> {
            self.inner.close()
        }
        fn attribute_source(&self) -> &AttributeSource {
            self.inner.attribute_source()
        }
        fn attribute_source_mut(&mut self) -> &mut AttributeSource {
            self.inner.attribute_source_mut()
        }
    }

    /// `RandomSkippingFilter`: `random` (a `java.util.Random`) is replaced by
    /// a new `Random(seed)` on `reset()`.
    #[derive(Debug)]
    struct RandomSkippingLogic {
        random: Mutex<JdkRandom>,
        seed: i64,
    }

    impl ConditionalTokenFilterLogic for RandomSkippingLogic {
        fn should_filter(&mut self, _source: &AttributeSource) -> Result<bool> {
            Ok(self.random.lock().unwrap().next_boolean())
        }
    }

    fn random_skipping_filter(
        input: Box<dyn TokenStream>,
        seed: i64,
        input_factory: impl FnOnce(Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>>,
    ) -> Result<Box<dyn TokenStream>> {
        let inner = new_conditional_token_filter(
            input,
            input_factory,
            RandomSkippingLogic {
                random: Mutex::new(JdkRandom::new(seed)),
                seed,
            },
        )?;
        Ok(boxed(ResettingConditionalFilter {
            inner,
            on_reset: |l: &RandomSkippingLogic| {
                *l.random.lock().unwrap() = JdkRandom::new(l.seed);
            },
        }))
    }

    /// `NonRandomSkippingFilter`: `pos` returns to 0 on `reset()`.
    #[derive(Debug)]
    struct NonRandomSkippingLogic {
        should_filters: Vec<bool>,
        pos: Mutex<usize>,
    }

    impl ConditionalTokenFilterLogic for NonRandomSkippingLogic {
        fn should_filter(&mut self, _source: &AttributeSource) -> Result<bool> {
            let mut pos = self.pos.lock().unwrap();
            let r = self.should_filters[*pos % self.should_filters.len()];
            *pos += 1;
            Ok(r)
        }
    }

    fn non_random_skipping_filter(
        input: Box<dyn TokenStream>,
        input_factory: impl FnOnce(Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>>,
        should_filters: &[bool],
    ) -> Result<Box<dyn TokenStream>> {
        let inner = new_conditional_token_filter(
            input,
            input_factory,
            NonRandomSkippingLogic {
                should_filters: should_filters.to_vec(),
                pos: Mutex::new(0),
            },
        )?;
        Ok(boxed(ResettingConditionalFilter {
            inner,
            on_reset: |l: &NonRandomSkippingLogic| *l.pos.lock().unwrap() = 0,
        }))
    }

    pub fn class() -> Arc<Class> {
        btstc_class("org.apache.lucene.analysis.miscellaneous.TestConditionalTokenFilter")
            .method(t("testSimple", test_simple))
            .method(t("testMultitokenWrapping", test_multitoken_wrapping))
            .method(t("testEndPropagation", test_end_propagation))
            .method(t("testWrapGraphs", test_wrap_graphs))
            .method(t(
                "testReadaheadWithNoFiltering",
                test_readahead_with_no_filtering,
            ))
            .method(t(
                "testReadaheadWithFiltering",
                test_readahead_with_filtering,
            ))
            .method(t(
                "testFilteringWithReadahead",
                test_filtering_with_readahead,
            ))
            .method(t(
                "testMultipleConditionalFilters",
                test_multiple_conditional_filters,
            ))
            .method(t("testFilteredTokenFilters", test_filtered_token_filters))
            .method(t("testConsistentOffsets", test_consistent_offsets))
            .method(t("testEndWithShingles", test_end_with_shingles))
            .method(t(
                "testInternalPositionAdjustment",
                test_internal_position_adjustment,
            ))
            .build()
    }
}

#[test]
fn test_conditional_token_filter() {
    run(test_conditional_token_filter::class);
}

// -----------------------------------------------------------------------------
// TestElision (org.apache.lucene.analysis.util; listed by rmp #468)
// -----------------------------------------------------------------------------

mod test_elision {
    use super::*;
    use rucene::analysis::standard::new_standard_tokenizer_with_factory;
    use rucene::analysis::StringCharReader;
    use rucene_analysis_common::analysis::fr::french_analyzer::DEFAULT_ARTICLES;
    use rucene_analysis_common::analysis::util::elision_filter::new_elision_filter;

    fn test_elision() -> Result<()> {
        let test = "Plop, juste pour voir l'embrouille avec O'brian. M'enfin.";
        let mut tokenizer = new_standard_tokenizer_with_factory(B::new_attribute_factory())?;
        tokenizer.set_reader(Box::new(StringCharReader::new(test)))?;
        let articles = lang_support::char_set(&["l", "M"], false);
        let mut filter = new_elision_filter(boxed(tokenizer), articles)?;
        let tas = filter_terms(&mut filter)?;
        Assert::assert_equals("embrouille", tas[4].as_str());
        Assert::assert_equals("O'brian", tas[6].as_str());
        Assert::assert_equals("enfin", tas[7].as_str());
        Ok(())
    }

    /// `filter(TokenFilter)`.
    fn filter_terms(filter: &mut dyn TokenStream) -> Result<Vec<String>> {
        let mut tas = Vec::new();
        filter.reset()?;
        while filter.increment_token()? {
            tas.push(term_of(filter.attribute_source()));
        }
        filter.end()?;
        filter.close()?;
        Ok(tas)
    }

    fn test_empty_term() -> Result<()> {
        let a = FnAnalyzer::new("org.apache.lucene.analysis.util.TestElision$1", |_| {
            let tokenizer = own(new_keyword_tokenizer());
            let stream = boxed(new_elision_filter(
                shared(&tokenizer),
                DEFAULT_ARTICLES.clone(),
            )?);
            Ok(components(tokenizer, sink(BoxedStream(stream))))
        })
        .arc();
        B::check_one_term(&*a, "", "")?;
        a.close();
        Ok(())
    }

    pub fn class() -> Arc<Class> {
        btstc_class("org.apache.lucene.analysis.util.TestElision")
            .method(t("testElision", test_elision))
            .method(t("testEmptyTerm", test_empty_term))
            .build()
    }
}

#[test]
fn test_elision() {
    run(test_elision::class);
}
