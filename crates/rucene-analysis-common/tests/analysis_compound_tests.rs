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
//! that rmp task #461 lists, each run as a suite by the ported runner:
//! `org.apache.lucene.analysis.compound.TestCompoundWordTokenFilter`,
//! `TestDictionaryCompoundWordTokenFilterFactory`,
//! `TestHyphenationCompoundWordTokenFilterFactory` and
//! `org.apache.lucene.analysis.compound.hyphenation.TestPatternParser`. The
//! dictionaries and hyphenation grammars are copied byte for byte to
//! `tests/resources/org/apache/lucene/analysis/compound/`; a test reads a
//! grammar through the `file:` URL of its resource
//! (`getClass().getResource(name).toExternalForm()`).

mod support;

use std::any::{Any, TypeId};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::tokenattributes::CharTermAttribute;
use rucene::analysis::{
    CharArraySet, StringCharReader, TokenFilter, TokenFilterLogic, TokenStream,
};
use rucene::error::{LuceneErrorKind, Result};
use rucene::store::java_nio_file::java_cause_of;
use rucene::util::attribute::{Attribute, AttributeImpl, AttributeReflector};
use rucene::util::AttributeSource;
use rucene_analysis_common::analysis::charfilter::mapping_char_filter::MappingCharFilter;
use rucene_analysis_common::analysis::charfilter::normalize_char_map::Builder as NormalizeCharMapBuilder;
use rucene_analysis_common::analysis::compound::compound_word_token_filter_base::{
    DEFAULT_MAX_SUBWORD_SIZE, DEFAULT_MIN_SUBWORD_SIZE, DEFAULT_MIN_WORD_SIZE,
};
use rucene_analysis_common::analysis::compound::dictionary_compound_word_token_filter::{
    new_dictionary_compound_word_token_filter,
    new_dictionary_compound_word_token_filter_with_only_longest_match,
};
use rucene_analysis_common::analysis::compound::hyphenation::hyphenation_tree::HyphenationTree;
use rucene_analysis_common::analysis::compound::hyphenation::pattern_parser::PatternParser;
use rucene_analysis_common::analysis::compound::hyphenation_compound_word_token_filter::{
    get_hyphenation_tree_from_source, new_hyphenation_compound_word_token_filter,
    new_hyphenation_compound_word_token_filter_with_flags,
    new_hyphenation_compound_word_token_filter_with_hyphenator,
    new_hyphenation_compound_word_token_filter_with_sizes,
    new_hyphenation_compound_word_token_filter_without_dictionary,
};
use rucene_analysis_common::jdk::sax::{InputSource, SaxException};
use rucene_test_framework::analysis::mock_tokenizer::{MockTokenizer, WHITESPACE};
use rucene_test_framework::analysis::{shared, BaseTokenStreamTestCase};
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::RANDOM_MULTIPLIER;
use rucene_test_framework::util::LuceneTestCase;
use support::{
    boxed, btstc_class, factory_class, keyword_tokenizer_analyzer, lucene_class, own, run, t, term,
    token_filter_factory, FnAnalyzer,
};

type B = BaseTokenStreamTestCase;

/// The package of the compound test classes, where their resources are.
const PACKAGE: &str = "org.apache.lucene.analysis.compound";

/// The file of the test resource `name` of the compound package.
fn resource_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/resources")
        .join(PACKAGE.replace('.', "/"))
        .join(name)
}

/// `getClass().getResource(name).toExternalForm()`: a `file:` URL.
fn resource_url(name: &str) -> String {
    format!("file:{}", resource_path(name).display())
}

/// `HyphenationCompoundWordTokenFilter.getHyphenationTree(new
/// InputSource(getClass().getResource(name).toExternalForm()))`.
fn hyphenator(name: &str) -> Result<Arc<HyphenationTree>> {
    Ok(Arc::new(get_hyphenation_tree_from_source(
        InputSource::from_system_id(resource_url(name)),
    )?))
}

/// `makeDictionary(dictionary...)`: an ignore-case set.
fn make_dictionary(dictionary: &[&str]) -> CharArraySet {
    CharArraySet::from_iter(dictionary.iter().copied(), true)
}

/// `whitespaceMockTokenizer(input)`.
fn ws(input: &str) -> Result<Box<dyn TokenStream>> {
    Ok(boxed(B::whitespace_mock_tokenizer_str(input)?))
}

/// `new MockTokenizer(MockTokenizer.WHITESPACE, false)` reading `input`.
fn mock_ws(input: &str) -> Result<MockTokenizer> {
    let mut tokenizer = MockTokenizer::new(Arc::clone(&WHITESPACE), false)?;
    tokenizer.set_reader(Box::new(StringCharReader::new(input)))?;
    Ok(tokenizer)
}

// -----------------------------------------------------------------------------
// TestCompoundWordTokenFilter
// -----------------------------------------------------------------------------

fn cw_test_hyphenation_compound_words_da() -> Result<()> {
    let dict = make_dictionary(&["l\u{e6}se", "hest"]);

    let hyphenator = hyphenator("da_UTF8.xml")?;

    let mut tf = new_hyphenation_compound_word_token_filter_with_sizes(
        ws("min veninde som er lidt af en l\u{e6}sehest")?,
        hyphenator,
        Some(dict),
        DEFAULT_MIN_WORD_SIZE,
        DEFAULT_MIN_SUBWORD_SIZE,
        DEFAULT_MAX_SUBWORD_SIZE,
        false,
    )?;
    B::assert_token_stream_contents_pos_increments(
        &mut tf,
        &[
            "min",
            "veninde",
            "som",
            "er",
            "lidt",
            "af",
            "en",
            "l\u{e6}sehest",
            "l\u{e6}se",
            "hest",
        ],
        &[1, 1, 1, 1, 1, 1, 1, 1, 0, 0],
    )
}

fn cw_test_hyphenation_compound_words_de_longest_match() -> Result<()> {
    let dict = make_dictionary(&["basketball", "basket", "ball", "kurv"]);

    let hyphenator = hyphenator("da_UTF8.xml")?;

    // the word basket will not be added due to the longest match option
    let mut tf = new_hyphenation_compound_word_token_filter_with_sizes(
        ws("basketballkurv")?,
        hyphenator,
        Some(dict),
        DEFAULT_MIN_WORD_SIZE,
        DEFAULT_MIN_SUBWORD_SIZE,
        40,
        true,
    )?;
    B::assert_token_stream_contents_pos_increments(
        &mut tf,
        &["basketballkurv", "basketball", "ball", "kurv"],
        &[1, 0, 0, 0],
    )
}

/// With hyphenation-only, you can get a lot of nonsense tokens. This can be
/// controlled with the min/max subword size.
fn cw_test_hyphenation_only() -> Result<()> {
    let hyphenator = hyphenator("da_UTF8.xml")?;

    let mut tf = new_hyphenation_compound_word_token_filter_without_dictionary(
        ws("basketballkurv")?,
        Arc::clone(&hyphenator),
        DEFAULT_MIN_WORD_SIZE,
        2,
        4,
    )?;

    // min=2, max=4
    B::assert_token_stream_contents(
        &mut tf,
        &["basketballkurv", "ba", "sket", "ball", "bal", "kurv"],
    )?;

    let mut tf = new_hyphenation_compound_word_token_filter_without_dictionary(
        ws("basketballkurv")?,
        Arc::clone(&hyphenator),
        DEFAULT_MIN_WORD_SIZE,
        4,
        6,
    )?;

    // min=4, max=6
    B::assert_token_stream_contents(
        &mut tf,
        &["basketballkurv", "basket", "sket", "ball", "lkurv", "kurv"],
    )?;

    let mut tf = new_hyphenation_compound_word_token_filter_without_dictionary(
        ws("basketballkurv")?,
        hyphenator,
        DEFAULT_MIN_WORD_SIZE,
        4,
        10,
    )?;

    // min=4, max=10
    B::assert_token_stream_contents(
        &mut tf,
        &[
            "basketballkurv",
            "basketball",
            "basketbal",
            "basket",
            "sketball",
            "sketbal",
            "sket",
            "ballkurv",
            "ball",
            "lkurv",
            "kurv",
        ],
    )
}

fn cw_test_dumb_compound_words_se() -> Result<()> {
    let dict = make_dictionary(&[
        "Bil",
        "D\u{f6}rr",
        "Motor",
        "Tak",
        "Borr",
        "Slag",
        "Hammar",
        "Pelar",
        "Glas",
        "\u{d6}gon",
        "Fodral",
        "Bas",
        "Fiol",
        "Makare",
        "Ges\u{e4}ll",
        "Sko",
        "Vind",
        "Rute",
        "Torkare",
        "Blad",
    ]);

    let mut tf = new_dictionary_compound_word_token_filter(
        ws("Bild\u{f6}rr Bilmotor Biltak Slagborr Hammarborr Pelarborr Glas\u{f6}gonfodral Basfiolsfodral Basfiolsfodralmakareges\u{e4}ll Skomakare Vindrutetorkare Vindrutetorkarblad abba")?,
        Some(dict),
    )?;

    B::assert_token_stream_contents_offsets_pos_increments(
        &mut tf,
        &[
            "Bild\u{f6}rr",
            "Bil",
            "d\u{f6}rr",
            "Bilmotor",
            "Bil",
            "motor",
            "Biltak",
            "Bil",
            "tak",
            "Slagborr",
            "Slag",
            "borr",
            "Hammarborr",
            "Hammar",
            "borr",
            "Pelarborr",
            "Pelar",
            "borr",
            "Glas\u{f6}gonfodral",
            "Glas",
            "\u{f6}gon",
            "fodral",
            "Basfiolsfodral",
            "Bas",
            "fiol",
            "fodral",
            "Basfiolsfodralmakareges\u{e4}ll",
            "Bas",
            "fiol",
            "fodral",
            "makare",
            "ges\u{e4}ll",
            "Skomakare",
            "Sko",
            "makare",
            "Vindrutetorkare",
            "Vind",
            "rute",
            "torkare",
            "Vindrutetorkarblad",
            "Vind",
            "rute",
            "blad",
            "abba",
        ],
        &[
            0, 0, 0, 8, 8, 8, 17, 17, 17, 24, 24, 24, 33, 33, 33, 44, 44, 44, 54, 54, 54, 54, 69,
            69, 69, 69, 84, 84, 84, 84, 84, 84, 111, 111, 111, 121, 121, 121, 121, 137, 137, 137,
            137, 156,
        ],
        &[
            7, 7, 7, 16, 16, 16, 23, 23, 23, 32, 32, 32, 43, 43, 43, 53, 53, 53, 68, 68, 68, 68,
            83, 83, 83, 83, 110, 110, 110, 110, 110, 110, 120, 120, 120, 136, 136, 136, 136, 155,
            155, 155, 155, 160,
        ],
        &[
            1, 0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0,
            0, 0, 0, 1, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1,
        ],
    )
}

fn cw_test_dumb_compound_words_se_longest_match() -> Result<()> {
    let dict = make_dictionary(&[
        "Bil",
        "D\u{f6}rr",
        "Motor",
        "Tak",
        "Borr",
        "Slag",
        "Hammar",
        "Pelar",
        "Glas",
        "\u{d6}gon",
        "Fodral",
        "Bas",
        "Fiols",
        "Makare",
        "Ges\u{e4}ll",
        "Sko",
        "Vind",
        "Rute",
        "Torkare",
        "Blad",
        "Fiolsfodral",
    ]);

    let mut tf = new_dictionary_compound_word_token_filter_with_only_longest_match(
        ws("Basfiolsfodralmakareges\u{e4}ll")?,
        Some(dict),
        DEFAULT_MIN_WORD_SIZE,
        DEFAULT_MIN_SUBWORD_SIZE,
        DEFAULT_MAX_SUBWORD_SIZE,
        true,
        false,
    )?;

    B::assert_token_stream_contents_offsets_pos_increments(
        &mut tf,
        &[
            "Basfiolsfodralmakareges\u{e4}ll",
            "Bas",
            "fiolsfodral",
            "fodral",
            "makare",
            "ges\u{e4}ll",
        ],
        &[0, 0, 0, 0, 0, 0],
        &[26, 26, 26, 26, 26, 26],
        &[1, 0, 0, 0, 0, 0],
    )
}

fn cw_test_token_ending_with_word_component_of_minimum_length() -> Result<()> {
    let dict = make_dictionary(&["ab", "cd", "ef"]);

    let tokenizer = mock_ws("abcdef")?;
    let mut tf = new_dictionary_compound_word_token_filter_with_only_longest_match(
        boxed(tokenizer),
        Some(dict),
        DEFAULT_MIN_WORD_SIZE,
        DEFAULT_MIN_SUBWORD_SIZE,
        DEFAULT_MAX_SUBWORD_SIZE,
        false,
        false,
    )?;

    B::assert_token_stream_contents_offsets_pos_increments(
        &mut tf,
        &["abcdef", "ab", "cd", "ef"],
        &[0, 0, 0, 0],
        &[6, 6, 6, 6],
        &[1, 0, 0, 0],
    )
}

fn cw_test_word_component_with_less_than_minimum_length() -> Result<()> {
    let dict = make_dictionary(&["abc", "d", "efg"]);

    let tokenizer = mock_ws("abcdefg")?;
    let mut tf = new_dictionary_compound_word_token_filter_with_only_longest_match(
        boxed(tokenizer),
        Some(dict),
        DEFAULT_MIN_WORD_SIZE,
        DEFAULT_MIN_SUBWORD_SIZE,
        DEFAULT_MAX_SUBWORD_SIZE,
        false,
        false,
    )?;

    // since "d" is shorter than the minimum subword size, it should not be
    // added to the token stream
    B::assert_token_stream_contents_offsets_pos_increments(
        &mut tf,
        &["abcdefg", "abc", "efg"],
        &[0, 0, 0],
        &[7, 7, 7],
        &[1, 0, 0],
    )
}

fn cw_test_reset() -> Result<()> {
    let dict = make_dictionary(&[
        "Rind",
        "Fleisch",
        "Draht",
        "Schere",
        "Gesetz",
        "Aufgabe",
        "\u{dc}berwachung",
    ]);

    let mut ws_tokenizer = MockTokenizer::new(Arc::clone(&WHITESPACE), false)?;
    ws_tokenizer.set_enable_checks(false); // we will reset in a strange place
    ws_tokenizer.set_reader(Box::new(StringCharReader::new(
        "Rindfleisch\u{fc}berwachungsgesetz",
    )))?;
    let ws_tokenizer = own(ws_tokenizer);
    let mut tf = new_dictionary_compound_word_token_filter_with_only_longest_match(
        shared(&ws_tokenizer),
        Some(dict),
        DEFAULT_MIN_WORD_SIZE,
        DEFAULT_MIN_SUBWORD_SIZE,
        DEFAULT_MAX_SUBWORD_SIZE,
        false,
        false,
    )?;

    tf.reset()?;
    Assert::assert_true(tf.increment_token()?);
    eq("Rindfleisch\u{fc}berwachungsgesetz", &term(&tf));
    Assert::assert_true(tf.increment_token()?);
    eq("Rind", &term(&tf));
    tf.end()?;
    tf.close()?;
    ws_tokenizer
        .borrow_mut()
        .set_reader(Box::new(StringCharReader::new(
            "Rindfleisch\u{fc}berwachungsgesetz",
        )))?;
    tf.reset()?;
    Assert::assert_true(tf.increment_token()?);
    eq("Rindfleisch\u{fc}berwachungsgesetz", &term(&tf));
    let _ = tf
        .attribute_source()
        .get_attribute_interface::<dyn CharTermAttribute>();
    Ok(())
}

/// `assertEquals(expected, actual)` of two strings.
fn eq(expected: &str, actual: &str) {
    Assert::assert_equals_str(None, Some(expected), Some(actual));
}

/// `TestCompoundWordTokenFilter.MockRetainAttribute`.
trait MockRetainAttribute: Attribute {
    fn set_retain(&mut self, attr: bool);

    fn get_retain(&self) -> bool;
}

/// `TestCompoundWordTokenFilter.MockRetainAttributeImpl`.
#[derive(Debug, Clone, Default)]
struct MockRetainAttributeImpl {
    retain: bool,
}

impl Attribute for MockRetainAttributeImpl {}

impl MockRetainAttribute for MockRetainAttributeImpl {
    fn set_retain(&mut self, retain: bool) {
        self.retain = retain;
    }

    fn get_retain(&self) -> bool {
        self.retain
    }
}

impl AttributeImpl for MockRetainAttributeImpl {
    fn clear(&mut self) {
        self.retain = false;
    }

    fn copy_to(&self, target: &mut dyn AttributeImpl) {
        let t = target
            .as_any_mut()
            .downcast_mut::<MockRetainAttributeImpl>()
            .expect("(MockRetainAttribute) target");
        t.set_retain(self.retain);
    }

    fn reflect_with(&self, reflector: &mut dyn AttributeReflector) {
        reflector.reflect(
            TypeId::of::<dyn MockRetainAttribute>(),
            "MockRetainAttribute",
            "retain",
            &self.retain,
        );
    }

    fn clone_box(&self) -> Box<dyn AttributeImpl> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn attribute_interfaces(&self) -> &'static [TypeId] {
        static IDS: OnceLock<Vec<TypeId>> = OnceLock::new();
        IDS.get_or_init(|| {
            vec![
                TypeId::of::<MockRetainAttributeImpl>(),
                TypeId::of::<dyn MockRetainAttribute>(),
            ]
        })
    }

    fn equals(&self, other: &dyn AttributeImpl) -> bool {
        other
            .as_any()
            .downcast_ref::<MockRetainAttributeImpl>()
            .is_some_and(|o| o.retain == self.retain)
    }

    fn hash_code(&self) -> i32 {
        i32::from(self.retain)
    }
}

/// `addAttribute(MockRetainAttribute.class)`: the default factory finds
/// `MockRetainAttributeImpl` by its name; the port adds the instance.
fn add_retain_attribute(source: &mut AttributeSource) {
    if !source.has_attribute_by_id(TypeId::of::<dyn MockRetainAttribute>()) {
        source.add_attribute_impl_instance(Box::new(MockRetainAttributeImpl::default()));
    }
}

/// `retainAtt.getRetain()` / `setRetain(..)` through the source.
fn with_retain<R>(
    source: &AttributeSource,
    f: impl FnOnce(&mut MockRetainAttributeImpl) -> R,
) -> R {
    let cell = source
        .get_attribute_by_id(TypeId::of::<dyn MockRetainAttribute>())
        .expect("MockRetainAttribute");
    let mut att = cell.borrow_mut();
    f(att
        .as_any_mut()
        .downcast_mut::<MockRetainAttributeImpl>()
        .expect("MockRetainAttributeImpl"))
}

/// `TestCompoundWordTokenFilter.MockRetainAttributeFilter`.
#[derive(Debug)]
struct MockRetainAttributeFilterLogic;

impl TokenFilterLogic for MockRetainAttributeFilterLogic {
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if input.increment_token()? {
            with_retain(source, |a| a.set_retain(true));
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

fn mock_retain_attribute_filter(
    input: Box<dyn TokenStream>,
) -> TokenFilter<MockRetainAttributeFilterLogic> {
    let mut filter = TokenFilter::new(input, MockRetainAttributeFilterLogic);
    add_retain_attribute(filter.attribute_source_mut());
    filter
}

fn cw_test_retain_mock_attribute() -> Result<()> {
    let dict = make_dictionary(&["abc", "d", "efg"]);
    let tokenizer = mock_ws("abcdefg")?;
    let stream = mock_retain_attribute_filter(boxed(tokenizer));
    let mut stream = new_dictionary_compound_word_token_filter_with_only_longest_match(
        boxed(stream),
        Some(dict),
        DEFAULT_MIN_WORD_SIZE,
        DEFAULT_MIN_SUBWORD_SIZE,
        DEFAULT_MAX_SUBWORD_SIZE,
        false,
        false,
    )?;
    add_retain_attribute(stream.attribute_source_mut());
    stream.reset()?;
    while stream.increment_token()? {
        Assert::assert_true_msg(
            "Custom attribute value was lost",
            with_retain(stream.attribute_source(), |a| a.get_retain()),
        );
    }
    Ok(())
}

fn cw_test_lucene8124() -> Result<()> {
    let hyphenator = hyphenator("hyphenation-LUCENE-8124.xml")?;

    let mut tf =
        new_hyphenation_compound_word_token_filter_with_hyphenator(ws("Rindfleisch")?, hyphenator)?;

    B::assert_token_stream_contents(&mut tf, &["Rindfleisch", "Rind", "fleisch"])
}

/// `new HyphenationCompoundWordTokenFilter(ws(input), hyphenator,
/// dictionary, DEFAULT_MIN_WORD_SIZE, DEFAULT_MIN_SUBWORD_SIZE,
/// DEFAULT_MAX_SUBWORD_SIZE, onlyLongestMatch, noSub, noOverlap)`.
fn flags_filter(
    input: &str,
    hyphenator: &Arc<HyphenationTree>,
    dictionary: &CharArraySet,
    only_longest_match: bool,
    no_sub: bool,
    no_overlap: bool,
) -> Result<impl TokenStream> {
    new_hyphenation_compound_word_token_filter_with_flags(
        ws(input)?,
        Arc::clone(hyphenator),
        Some(dictionary.clone()),
        DEFAULT_MIN_WORD_SIZE,
        DEFAULT_MIN_SUBWORD_SIZE,
        DEFAULT_MAX_SUBWORD_SIZE,
        only_longest_match,
        no_sub,
        no_overlap,
    )
}

// LUCENE-8183
fn cw_test_no_sub_and_no_overlap() -> Result<()> {
    let input = "fu\u{df}ballpumpe";
    let hyphenator = hyphenator("hyphenation-LUCENE-8183.xml")?;
    let dictionary = make_dictionary(&["fu\u{df}", "fu\u{df}ball", "ballpumpe", "ball", "pumpe"]);

    // test the default configuration
    let mut tf1 = new_hyphenation_compound_word_token_filter(
        ws(input)?,
        Arc::clone(&hyphenator),
        Some(dictionary.clone()),
    )?;
    B::assert_token_stream_contents(
        &mut tf1,
        &[
            "fu\u{df}ballpumpe",
            "fu\u{df}ball",
            "fu\u{df}",
            "ballpumpe",
            "ball",
            "pumpe",
        ],
    )?;

    // test with onlyLongestMatch
    let mut tf2 = new_hyphenation_compound_word_token_filter_with_sizes(
        ws(input)?,
        Arc::clone(&hyphenator),
        Some(dictionary.clone()),
        DEFAULT_MIN_WORD_SIZE,
        DEFAULT_MIN_SUBWORD_SIZE,
        DEFAULT_MAX_SUBWORD_SIZE,
        true,
    )?;
    B::assert_token_stream_contents(
        &mut tf2,
        &["fu\u{df}ballpumpe", "fu\u{df}ball", "ballpumpe", "pumpe"],
    )?;

    // test with noSub enabled and noOverlap disabled
    let mut tf3 = flags_filter(input, &hyphenator, &dictionary, true, true, false)?;
    B::assert_token_stream_contents(
        &mut tf3,
        &["fu\u{df}ballpumpe", "fu\u{df}ball", "ballpumpe"],
    )?;
    // assert that the onlyLongestMatch state does not matter if noSub is active
    let mut tf3b = flags_filter(input, &hyphenator, &dictionary, false, true, false)?;
    B::assert_token_stream_contents(
        &mut tf3b,
        &["fu\u{df}ballpumpe", "fu\u{df}ball", "ballpumpe"],
    )?;

    // test with noOverlap enabled
    let mut tf4 = flags_filter(input, &hyphenator, &dictionary, true, true, true)?;
    // NOTE: 'fußball' consumes 'ball' as possible start so 'ballpumpe' is not
    // considered and 'pumpe' is added
    B::assert_token_stream_contents(&mut tf4, &["fu\u{df}ballpumpe", "fu\u{df}ball", "pumpe"])?;

    // assert that the noSub and onlyLongestMatch states do not matter
    let mut tf4b = flags_filter(input, &hyphenator, &dictionary, false, false, true)?;
    B::assert_token_stream_contents(&mut tf4b, &["fu\u{df}ballpumpe", "fu\u{df}ball", "pumpe"])?;

    let mut tf4c = flags_filter(input, &hyphenator, &dictionary, true, false, true)?;
    B::assert_token_stream_contents(&mut tf4c, &["fu\u{df}ballpumpe", "fu\u{df}ball", "pumpe"])
}

// LUCENE-8183
fn cw_test_no_sub_and_token_in_dictionary() -> Result<()> {
    // test that no subwords are added if the token is part of the dictionary
    // and onlyLongestMatch or noSub is present
    let input = "fu\u{df}ball";
    let hyphenator = hyphenator("hyphenation-LUCENE-8183.xml")?;
    let dictionary = make_dictionary(&["fu\u{df}ball", "fu\u{df}", "ball"]);

    // test the default configuration as baseline
    let mut tf5 = new_hyphenation_compound_word_token_filter(
        ws(input)?,
        Arc::clone(&hyphenator),
        Some(dictionary.clone()),
    )?;
    B::assert_token_stream_contents(&mut tf5, &["fu\u{df}ball", "fu\u{df}", "ball"])?;

    // when onlyLongestMatch is enabled fußball matches dictionary. So even so
    // fußball is not added as token it MUST prevent shorter matches to be added
    let mut tf6 = flags_filter(input, &hyphenator, &dictionary, true, false, false)?;
    B::assert_token_stream_contents(&mut tf6, &["fu\u{df}ball"])?;

    // when noSub is enabled fuß and ball MUST NOT be added as subwords as
    // fußball is in the dictionary
    let mut tf7 = flags_filter(input, &hyphenator, &dictionary, false, true, false)?;
    B::assert_token_stream_contents(&mut tf7, &["fu\u{df}ball"])?;

    // when noOverlap is enabled fuß and ball MUST NOT be added as subwords as
    // fußball is in the dictionary
    let mut tf8 = flags_filter(input, &hyphenator, &dictionary, false, false, true)?;
    B::assert_token_stream_contents(&mut tf8, &["fu\u{df}ball"])
}

// SOLR-2891
// *CompoundWordTokenFilter blindly adds term length to offset, but this can
// take things out of bounds wrt original text if a previous filter increases
// the length of the word (in this case ü -> ue) so in this case we behave
// like WDF, and preserve any modified offsets
fn cw_test_invalid_offsets() -> Result<()> {
    let dict = make_dictionary(&["fall"]);
    let mut builder = NormalizeCharMapBuilder::new();
    builder.add("\u{fc}", "ue")?;
    let norm_map = Arc::new(builder.build()?);

    let analyzer = FnAnalyzer::new(
        "org.apache.lucene.analysis.compound.TestCompoundWordTokenFilter$1",
        move |_| {
            let tokenizer = own(MockTokenizer::new(Arc::clone(&WHITESPACE), false)?);
            let filter =
                new_dictionary_compound_word_token_filter(shared(&tokenizer), Some(dict.clone()))?;
            Ok(rucene_test_framework::analysis::components(
                tokenizer,
                rucene_test_framework::analysis::sink(filter),
            ))
        },
    )
    .init_reader(move |_, reader| Box::new(MappingCharFilter::new(Arc::clone(&norm_map), reader)))
    .arc();

    B::assert_analyzes_to_offsets(
        &*analyzer,
        "bank\u{fc}berfall",
        &["bankueberfall", "fall"],
        &[0, 0],
        &[12, 12],
    )?;
    analyzer.close();
    Ok(())
}

/// blast some random strings through the analyzer
fn cw_test_random_strings() -> Result<()> {
    let dict = make_dictionary(&["a", "e", "i", "o", "u", "y", "bc", "def"]);
    let a = support::whitespace_analyzer(
        "org.apache.lucene.analysis.compound.TestCompoundWordTokenFilter$2",
        move |t| {
            Ok(boxed(new_dictionary_compound_word_token_filter(
                t,
                Some(dict.clone()),
            )?))
        },
    );
    B::check_random_data(&*LuceneTestCase::random(), &a, 200 * *RANDOM_MULTIPLIER)?;
    a.close();

    let hyphenator = hyphenator("da_UTF8.xml")?;
    let b = support::whitespace_analyzer(
        "org.apache.lucene.analysis.compound.TestCompoundWordTokenFilter$3",
        move |t| {
            Ok(boxed(
                new_hyphenation_compound_word_token_filter_with_hyphenator(
                    t,
                    Arc::clone(&hyphenator),
                )?,
            ))
        },
    );
    B::check_random_data(&*LuceneTestCase::random(), &b, 200 * *RANDOM_MULTIPLIER)?;
    b.close();
    Ok(())
}

fn cw_test_empty_term() -> Result<()> {
    let dict = make_dictionary(&["a", "e", "i", "o", "u", "y", "bc", "def"]);
    let a = keyword_tokenizer_analyzer(
        "org.apache.lucene.analysis.compound.TestCompoundWordTokenFilter$4",
        move |t| {
            Ok(boxed(new_dictionary_compound_word_token_filter(
                t,
                Some(dict.clone()),
            )?))
        },
    );
    B::check_one_term(&*a, "", "")?;
    a.close();

    let hyphenator = hyphenator("da_UTF8.xml")?;
    let b = keyword_tokenizer_analyzer(
        "org.apache.lucene.analysis.compound.TestCompoundWordTokenFilter$5",
        move |t| {
            Ok(boxed(
                new_hyphenation_compound_word_token_filter_with_hyphenator(
                    t,
                    Arc::clone(&hyphenator),
                )?,
            ))
        },
    );
    B::check_one_term(&*b, "", "")?;
    b.close();
    Ok(())
}

/// `getDictionaryCompoundWordTokenFilter(tokenizer, searchTerm, dict,
/// onlyLongestMatch)`.
fn get_dictionary_compound_word_token_filter(
    search_term: &str,
    dict: CharArraySet,
    only_longest_match: bool,
) -> Result<impl TokenStream> {
    let tokenizer = mock_ws(search_term)?;
    new_dictionary_compound_word_token_filter_with_only_longest_match(
        boxed(tokenizer),
        Some(dict),
        DEFAULT_MIN_WORD_SIZE,
        DEFAULT_MIN_SUBWORD_SIZE,
        DEFAULT_MAX_SUBWORD_SIZE,
        only_longest_match,
        true,
    )
}

fn cw_test_decompounding_with_consuming_chars() -> Result<()> {
    let dict = make_dictionary(&["wein", "schwein", "fleisch"]);

    let search_term = "schweinefleisch";
    let mut tf = get_dictionary_compound_word_token_filter(search_term, dict, true)?;

    B::assert_token_stream_contents(&mut tf, &[search_term, "schwein", "fleisch"])
}

fn cw_test_decompounding_with_consuming_chars2() -> Result<()> {
    let dict = make_dictionary(&["waffe", "affe", "kampf"]);

    let search_term = "nahkampfwaffen";

    let mut tf = get_dictionary_compound_word_token_filter(search_term, dict, true)?;

    B::assert_token_stream_contents(&mut tf, &[search_term, "kampf", "waffe"])
}

fn compound_word_token_filter_class() -> Arc<Class> {
    btstc_class(&format!("{PACKAGE}.TestCompoundWordTokenFilter"))
        .method(t(
            "testHyphenationCompoundWordsDA",
            cw_test_hyphenation_compound_words_da,
        ))
        .method(t(
            "testHyphenationCompoundWordsDELongestMatch",
            cw_test_hyphenation_compound_words_de_longest_match,
        ))
        .method(t("testHyphenationOnly", cw_test_hyphenation_only))
        .method(t("testDumbCompoundWordsSE", cw_test_dumb_compound_words_se))
        .method(t(
            "testDumbCompoundWordsSELongestMatch",
            cw_test_dumb_compound_words_se_longest_match,
        ))
        .method(t(
            "testTokenEndingWithWordComponentOfMinimumLength",
            cw_test_token_ending_with_word_component_of_minimum_length,
        ))
        .method(t(
            "testWordComponentWithLessThanMinimumLength",
            cw_test_word_component_with_less_than_minimum_length,
        ))
        .method(t("testReset", cw_test_reset))
        .method(t("testRetainMockAttribute", cw_test_retain_mock_attribute))
        .method(t("testLucene8124", cw_test_lucene8124))
        .method(t("testNoSubAndNoOverlap", cw_test_no_sub_and_no_overlap))
        .method(t(
            "testNoSubAndTokenInDictionary",
            cw_test_no_sub_and_token_in_dictionary,
        ))
        .method(t("testInvalidOffsets", cw_test_invalid_offsets))
        .method(t("testRandomStrings", cw_test_random_strings))
        .method(t("testEmptyTerm", cw_test_empty_term))
        .method(t(
            "testDecompoundingWithConsumingChars",
            cw_test_decompounding_with_consuming_chars,
        ))
        .method(t(
            "testDecompoundingWithConsumingChars2",
            cw_test_decompounding_with_consuming_chars2,
        ))
        .build()
}

// -----------------------------------------------------------------------------
// TestDictionaryCompoundWordTokenFilterFactory
// -----------------------------------------------------------------------------

/// Ensure the filter actually decompounds text.
fn dcwf_test_decompounding() -> Result<()> {
    let stream = mock_ws("I like to play softball")?;
    let mut stream = token_filter_factory(
        PACKAGE,
        "DictionaryCompoundWord",
        &["dictionary", "compoundDictionary.txt"],
    )?
    .create(boxed(stream))?;
    B::assert_token_stream_contents(
        &mut *stream,
        &["I", "like", "to", "play", "softball", "soft", "ball"],
    )
}

/// Test that bogus arguments result in exception
fn dcwf_test_bogus_arguments() -> Result<()> {
    let expected = expect_throws("java.lang.IllegalArgumentException", || {
        token_filter_factory(
            PACKAGE,
            "DictionaryCompoundWord",
            &[
                "dictionary",
                "compoundDictionary.txt",
                "bogusArg",
                "bogusValue",
            ],
        )
    });
    Assert::assert_true(
        expected
            .message()
            .unwrap_or("")
            .contains("Unknown parameters"),
    );
    Ok(())
}

fn dictionary_compound_word_token_filter_factory_class() -> Arc<Class> {
    factory_class(&format!(
        "{PACKAGE}.TestDictionaryCompoundWordTokenFilterFactory"
    ))
    .method(t("testDecompounding", dcwf_test_decompounding))
    .method(t("testBogusArguments", dcwf_test_bogus_arguments))
    .build()
}

// -----------------------------------------------------------------------------
// TestHyphenationCompoundWordTokenFilterFactory
// -----------------------------------------------------------------------------

/// Ensure the factory works with hyphenation grammar+dictionary: using
/// default options.
fn hcwf_test_hyphenation_with_dictionary() -> Result<()> {
    let stream = mock_ws("min veninde som er lidt af en l\u{e6}sehest")?;
    let mut stream = token_filter_factory(
        PACKAGE,
        "HyphenationCompoundWord",
        &[
            "hyphenator",
            "da_UTF8.xml",
            "dictionary",
            "da_compoundDictionary.txt",
        ],
    )?
    .create(boxed(stream))?;

    B::assert_token_stream_contents_pos_increments(
        &mut *stream,
        &[
            "min",
            "veninde",
            "som",
            "er",
            "lidt",
            "af",
            "en",
            "l\u{e6}sehest",
            "l\u{e6}se",
            "hest",
        ],
        &[1, 1, 1, 1, 1, 1, 1, 1, 0, 0],
    )
}

/// just tests that the two no configuration options are correctly processed
/// tests for the functionality are part of TestCompoundWordTokenFilter
fn hcwf_test_lucene8183() -> Result<()> {
    let stream = mock_ws("basketballkurv")?;
    let mut stream = token_filter_factory(
        PACKAGE,
        "HyphenationCompoundWord",
        &[
            "hyphenator",
            "da_UTF8.xml",
            "dictionary",
            "compoundDictionary_lucene8183.txt",
            "onlyLongestMatch",
            "false",
            "noSubMatches",
            "true",
            "noOverlappingMatches",
            "false",
        ],
    )?
    .create(boxed(stream))?;

    B::assert_token_stream_contents_pos_increments(
        &mut *stream,
        &["basketballkurv", "basketball", "kurv"],
        &[1, 0, 0],
    )
}

/// Ensure the factory works with no dictionary: using hyphenation grammar
/// only. Also change the min/max subword sizes from the default. When using
/// no dictionary, it's generally necessary to tweak these, or you get lots
/// of expansions.
fn hcwf_test_hyphenation_only() -> Result<()> {
    let stream = mock_ws("basketballkurv")?;
    let mut stream = token_filter_factory(
        PACKAGE,
        "HyphenationCompoundWord",
        &[
            "hyphenator",
            "da_UTF8.xml",
            "minSubwordSize",
            "2",
            "maxSubwordSize",
            "4",
        ],
    )?
    .create(boxed(stream))?;

    B::assert_token_stream_contents(
        &mut *stream,
        &["basketballkurv", "ba", "sket", "ball", "bal", "kurv"],
    )
}

/// Test that bogus arguments result in exception
fn hcwf_test_bogus_arguments() -> Result<()> {
    let expected = expect_throws("java.lang.IllegalArgumentException", || {
        token_filter_factory(
            PACKAGE,
            "HyphenationCompoundWord",
            &["hyphenator", "da_UTF8.xml", "bogusArg", "bogusValue"],
        )
    });
    Assert::assert_true(
        expected
            .message()
            .unwrap_or("")
            .contains("Unknown parameters"),
    );
    Ok(())
}

fn hyphenation_compound_word_token_filter_factory_class() -> Arc<Class> {
    factory_class(&format!(
        "{PACKAGE}.TestHyphenationCompoundWordTokenFilterFactory"
    ))
    .method(t(
        "testHyphenationWithDictionary",
        hcwf_test_hyphenation_with_dictionary,
    ))
    .method(t("testLucene8183", hcwf_test_lucene8183))
    .method(t("testHyphenationOnly", hcwf_test_hyphenation_only))
    .method(t("testBogusArguments", hcwf_test_bogus_arguments))
    .build()
}

// -----------------------------------------------------------------------------
// TestPatternParser
// -----------------------------------------------------------------------------

fn pp_test_external_entity_is_not_expanded() -> Result<()> {
    // `this.getClass().getResource("../compoundDictionary.txt")`
    let external_ref = resource_url("compoundDictionary.txt");
    let evil = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE hyphenation-info [
  <!ENTITY xxe SYSTEM "{external_ref}">
]>
<hyphenation-info>
  <classes>aA</classes>
  <patterns>&xxe;</patterns>
</hyphenation-info>
"#
    );

    let mut tree = HyphenationTree::new();
    let mut parser = PatternParser::with_consumer(&mut tree);
    let e = match parser.parse(InputSource::from_character_stream(Box::new(
        StringCharReader::new(&evil),
    ))) {
        Ok(()) => Assert::fail("Expected exception IOException but no exception was thrown"),
        Err(e) => e,
    };
    let LuceneErrorKind::Io(io) = e.kind() else {
        Assert::fail(&format!("expected an IOException, got {e:?}"))
    };
    // `e.getCause() instanceof SAXException`
    let cause = java_cause_of(io);
    Assert::assert_true(cause.is_some_and(|c| SaxException::of(c).is_some()));
    let message = io.to_string();
    Assert::assert_true(message.contains("External Entity resolving unsupported"));
    Assert::assert_true(message.contains(&external_ref));
    Ok(())
}

fn pattern_parser_class() -> Arc<Class> {
    lucene_class(&format!("{PACKAGE}.hyphenation.TestPatternParser"))
        .method(t(
            "testExternalEntityIsNotExpanded",
            pp_test_external_entity_is_not_expanded,
        ))
        .build()
}

// -----------------------------------------------------------------------------
// Suites
// -----------------------------------------------------------------------------

#[test]
fn test_compound_word_token_filter() {
    run(compound_word_token_filter_class);
}

#[test]
fn test_dictionary_compound_word_token_filter_factory() {
    run(dictionary_compound_word_token_filter_factory_class);
}

#[test]
fn test_hyphenation_compound_word_token_filter_factory() {
    run(hyphenation_compound_word_token_filter_factory_class);
}

#[test]
fn test_pattern_parser() {
    run(pattern_parser_class);
}
