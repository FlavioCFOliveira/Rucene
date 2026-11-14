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
//! that rmp task #460 lists, each run as a suite by the ported runner:
//! `org.apache.lucene.analysis.boost.TestDelimitedBoostTokenFilter`,
//! `org.apache.lucene.analysis.payloads.TestDelimitedPayloadTokenFilter`,
//! `TestDelimitedPayloadTokenFilterFactory`, `TestNumericPayloadTokenFilter`,
//! `TestTokenOffsetPayloadTokenFilter` and `TestTypeAsPayloadTokenFilter`,
//! `org.apache.lucene.analysis.ngram.TestNGramFilters`,
//! `org.apache.lucene.analysis.path.TestPathHierarchyTokenizer` and
//! `TestReversePathHierarchyTokenizer`,
//! `org.apache.lucene.analysis.query.TestQueryAutoStopWordAnalyzer`,
//! `org.apache.lucene.analysis.reverse.TestReverseStringFilter` and
//! `TestReverseStringFilterFactory`, and
//! `org.apache.lucene.analysis.sinks.TestTeeSinkTokenFilter`.
//!
//! Not ported, as Lucene does not run it either:
//! `TestTeeSinkTokenFilter.performance()` ("Not an explicit test, just useful
//! to print out some info on performance": not a `test*` method).

mod support;

use std::cell::RefCell;
use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::caching_token_filter::CachingTokenFilter;
use rucene::analysis::tokenattributes::{
    CharTermAttribute, OffsetAttribute, PayloadAttribute, TypeAttribute,
};
use rucene::analysis::{
    Analyzer, CharReader, StringCharReader, TokenFilter, TokenFilterLogic, TokenStream,
};
use rucene::document::{Document, Field, FieldType, Store, TextField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::postings_enum::POSTINGS_ENUM_ALL;
use rucene::index::IndexReader;
use rucene::search::NO_MORE_DOCS;
use rucene::search::{BoostAttribute, BoostAttributeImpl};
use rucene::store::{ByteBuffersDirectory, Directory};
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::AttributeSource;
use rucene_analysis_common::analysis::boost::delimited_boost_token_filter::new_delimited_boost_token_filter;
use rucene_analysis_common::analysis::boost::delimited_boost_token_filter_factory::DelimitedBoostTokenFilterFactory;
use rucene_analysis_common::analysis::charfilter::mapping_char_filter::MappingCharFilter;
use rucene_analysis_common::analysis::charfilter::normalize_char_map::Builder as NormalizeCharMapBuilder;
use rucene_analysis_common::analysis::core::lower_case_filter::LowerCaseFilter;
use rucene_analysis_common::analysis::path::path_hierarchy_tokenizer::{
    new_path_hierarchy_tokenizer, new_path_hierarchy_tokenizer_factory, DEFAULT_DELIMITER,
    DEFAULT_SKIP,
};
use rucene_analysis_common::analysis::path::reverse_path_hierarchy_tokenizer::{
    new_reverse_path_hierarchy_tokenizer, new_reverse_path_hierarchy_tokenizer_factory,
};
use rucene_analysis_common::analysis::payloads::delimited_payload_token_filter::{
    new_delimited_payload_token_filter, DEFAULT_DELIMITER as PAYLOAD_DEFAULT_DELIMITER,
};
use rucene_analysis_common::analysis::payloads::float_encoder::FloatEncoder;
use rucene_analysis_common::analysis::payloads::identity_encoder::IdentityEncoder;
use rucene_analysis_common::analysis::payloads::integer_encoder::IntegerEncoder;
use rucene_analysis_common::analysis::payloads::numeric_payload_token_filter::new_numeric_payload_token_filter;
use rucene_analysis_common::analysis::payloads::payload_helper::PayloadHelper;
use rucene_analysis_common::analysis::payloads::token_offset_payload_token_filter::new_token_offset_payload_token_filter;
use rucene_analysis_common::analysis::payloads::type_as_payload_token_filter::new_type_as_payload_token_filter;
use rucene_analysis_common::analysis::query::query_auto_stop_word_analyzer::QueryAutoStopWordAnalyzer;
use rucene_analysis_common::analysis::reverse::reverse_string_filter::{
    new_reverse_string_filter, new_reverse_string_filter_with_marker, reverse, reverse_range,
};
use rucene_analysis_common::analysis::sinks::tee_sink_token_filter::TeeSinkTokenFilter;
use rucene_test_framework::analysis::base_token_stream_test_case::{
    CheckClearAttributesAttribute, CheckClearAttributesAttributeImpl,
};
use rucene_test_framework::analysis::mock_analyzer::MockAnalyzer;
use rucene_test_framework::analysis::mock_tokenizer::{MockTokenizer, WHITESPACE};
use rucene_test_framework::analysis::{
    tokenizer_components, BaseTokenStreamFactoryTestCase, BaseTokenStreamTestCase,
};
use rucene_test_framework::util::lucene_test_case::RANDOM_MULTIPLIER;
use rucene_test_framework::util::LuceneTestCase;
use support::{
    boxed, btstc_class, factory_class, hook, iae_message, keyword_tokenizer_analyzer, own, run, t,
    whitespace_analyzer, FnAnalyzer,
};

type B = BaseTokenStreamTestCase;
type F = BaseTokenStreamFactoryTestCase;

fn random() -> Arc<dyn Random> {
    LuceneTestCase::random()
}

fn mult() -> i32 {
    *RANDOM_MULTIPLIER
}

/// `new StringReader(s)`.
fn string_reader(s: &str) -> Box<dyn CharReader> {
    Box::new(StringCharReader::new(s))
}

/// `tokenFilterFactory(name, keysAndValues...)`.
fn fac(
    name: &str,
    keys_and_values: &[&str],
) -> Result<Arc<dyn rucene::analysis::factories::TokenFilterFactory>> {
    F::token_filter_factory(name, keys_and_values)
}

/// `termAtt.toString()`.
fn term_of(ts: &dyn TokenStream) -> String {
    support::term(ts)
}

/// The bytes of `payloadAtt.getPayload()`, `None` for a null payload.
fn payload_of(ts: &dyn TokenStream) -> Option<Vec<u8>> {
    ts.attribute_source()
        .get_attribute_interface::<dyn PayloadAttribute>()
        .expect("payloadAtt")
        .get_payload()
        .map(|p| p.bytes[p.offset..p.offset + p.length].to_vec())
}

fn type_of(ts: &dyn TokenStream) -> String {
    ts.attribute_source()
        .get_attribute_interface::<dyn TypeAttribute>()
        .expect("typeAtt")
        .type_value()
        .to_string()
}

// -----------------------------------------------------------------------------
// TestDelimitedBoostTokenFilter
// -----------------------------------------------------------------------------

/// `assertTermEquals(expected, stream, [termAtt, boostAtt,] expectedBoost)`.
fn assert_boost_term_equals(
    expected: &str,
    stream: &mut dyn TokenStream,
    expected_boost: f32,
) -> Result<()> {
    // `stream.addAttribute(BoostAttribute.class)`
    stream
        .attribute_source_mut()
        .add_attribute::<BoostAttributeImpl>()?;
    Assert::assert_true(stream.increment_token()?);
    Assert::assert_equals(expected, term_of(stream).as_str());
    let actual_boost = stream
        .attribute_source()
        .get_attribute::<BoostAttributeImpl>()
        .expect("boostAtt")
        .get_boost();
    Assert::assert_true_msg(
        &format!("{actual_boost:?} does not equal: {expected_boost:?}"),
        actual_boost == expected_boost,
    );
    Ok(())
}

fn boost_filter(test: &str) -> Result<rucene_analysis_common::analysis::boost::delimited_boost_token_filter::DelimitedBoostTokenFilter>{
    new_delimited_boost_token_filter(
        boxed(B::whitespace_mock_tokenizer_str(test)?),
        DelimitedBoostTokenFilterFactory::DEFAULT_DELIMITER,
    )
}

fn dbtf_test_boosts() -> Result<()> {
    let test = "The quick|0.4 red|0.5 fox|0.2 jumped|0.1 over the lazy|0.8 brown|0.9 dogs|0.9";
    let mut filter = boost_filter(test)?;
    filter
        .attribute_source_mut()
        .add_attribute::<BoostAttributeImpl>()?;
    filter.reset()?;
    for (term, boost) in [
        ("The", 1.0f32),
        ("quick", 0.4),
        ("red", 0.5),
        ("fox", 0.2),
        ("jumped", 0.1),
        ("over", 1.0),
        ("the", 1.0),
        ("lazy", 0.8),
        ("brown", 0.9),
        ("dogs", 0.9),
    ] {
        assert_boost_term_equals(term, &mut filter, boost)?;
    }
    Assert::assert_false(filter.increment_token()?);
    filter.end()?;
    filter.close()?;
    Ok(())
}

fn dbtf_test_next() -> Result<()> {
    let test = "The quick|0.1 red|0.2 fox|0.3 jumped|0.4 over the lazy|0.5 brown|0.6 dogs|0.6";
    let mut filter = boost_filter(test)?;
    filter.reset()?;
    for (term, boost) in [
        ("The", 1.0f32),
        ("quick", 0.1),
        ("red", 0.2),
        ("fox", 0.3),
        ("jumped", 0.4),
        ("over", 1.0),
        ("the", 1.0),
        ("lazy", 0.5),
        ("brown", 0.6),
        ("dogs", 0.6),
    ] {
        assert_boost_term_equals(term, &mut filter, boost)?;
    }
    Assert::assert_false(filter.increment_token()?);
    filter.end()?;
    filter.close()?;
    Ok(())
}

fn delimited_boost_token_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.boost.TestDelimitedBoostTokenFilter")
        .method(t("testBoosts", dbtf_test_boosts))
        .method(t("testNext", dbtf_test_next))
        .build()
}

// -----------------------------------------------------------------------------
// TestDelimitedPayloadTokenFilter
// -----------------------------------------------------------------------------

/// `assertTermEquals(expected, stream, [termAtt, payAtt,] expectPay)`.
fn assert_payload_term_equals(
    expected: &str,
    stream: &mut dyn TokenStream,
    expect_pay: Option<&[u8]>,
) -> Result<()> {
    Assert::assert_true(stream.increment_token()?);
    Assert::assert_equals(expected, term_of(stream).as_str());
    match payload_of(stream) {
        Some(payload) => {
            let expect_pay = expect_pay.expect("a NullPointerException in Java");
            Assert::assert_true_msg(
                &format!("{} does not equal: {}", payload.len(), expect_pay.len()),
                payload.len() == expect_pay.len(),
            );
            for (e, a) in expect_pay.iter().zip(&payload) {
                Assert::assert_true_msg(
                    &format!("{} does not equal: {}", *e as i8, *a as i8),
                    e == a,
                );
            }
        }
        None => Assert::assert_true_msg(
            "expectPay is not null and it should be",
            expect_pay.is_none(),
        ),
    }
    Ok(())
}

const PAYLOAD_TEST: &str = "The quick|JJ red|JJ fox|NN jumped|VB over the lazy|JJ brown|JJ dogs|NN";

fn pos_tags(filter: &mut dyn TokenStream) -> Result<()> {
    for (term, pay) in [
        ("The", None),
        ("quick", Some("JJ")),
        ("red", Some("JJ")),
        ("fox", Some("NN")),
        ("jumped", Some("VB")),
        ("over", None),
        ("the", None),
        ("lazy", Some("JJ")),
        ("brown", Some("JJ")),
        ("dogs", Some("NN")),
    ] {
        assert_payload_term_equals(term, filter, pay.map(str::as_bytes))?;
    }
    Ok(())
}

fn dptf_test_payloads() -> Result<()> {
    let mut filter = new_delimited_payload_token_filter(
        boxed(B::whitespace_mock_tokenizer_str(PAYLOAD_TEST)?),
        PAYLOAD_DEFAULT_DELIMITER,
        Arc::new(IdentityEncoder::new()),
    )?;
    filter.reset()?;
    pos_tags(&mut filter)?;
    Assert::assert_false(filter.increment_token()?);
    filter.end()?;
    filter.close()?;
    Ok(())
}

fn dptf_test_next() -> Result<()> {
    dptf_test_payloads()
}

fn dptf_test_float_encoding() -> Result<()> {
    let test = "The quick|1.0 red|2.0 fox|3.5 jumped|0.5 over the lazy|5 brown|99.3 dogs|83.7";
    let mut filter = new_delimited_payload_token_filter(
        boxed(B::whitespace_mock_tokenizer_str(test)?),
        '|' as u16,
        Arc::new(FloatEncoder::new()),
    )?;
    filter.reset()?;
    for (term, pay) in [
        ("The", None),
        ("quick", Some(1.0f32)),
        ("red", Some(2.0)),
        ("fox", Some(3.5)),
        ("jumped", Some(0.5)),
        ("over", None),
        ("the", None),
        ("lazy", Some(5.0)),
        ("brown", Some(99.3)),
        ("dogs", Some(83.7)),
    ] {
        let bytes = pay.map(PayloadHelper::encode_float);
        assert_payload_term_equals(term, &mut filter, bytes.as_deref())?;
    }
    Assert::assert_false(filter.increment_token()?);
    filter.end()?;
    filter.close()?;
    Ok(())
}

fn dptf_test_int_encoding() -> Result<()> {
    let test = "The quick|1 red|2 fox|3 jumped over the lazy|5 brown|99 dogs|83";
    let mut filter = new_delimited_payload_token_filter(
        boxed(B::whitespace_mock_tokenizer_str(test)?),
        '|' as u16,
        Arc::new(IntegerEncoder::new()),
    )?;
    filter.reset()?;
    for (term, pay) in [
        ("The", None),
        ("quick", Some(1)),
        ("red", Some(2)),
        ("fox", Some(3)),
        ("jumped", None),
        ("over", None),
        ("the", None),
        ("lazy", Some(5)),
        ("brown", Some(99)),
        ("dogs", Some(83)),
    ] {
        let bytes = pay.map(PayloadHelper::encode_int);
        assert_payload_term_equals(term, &mut filter, bytes.as_deref())?;
    }
    Assert::assert_false(filter.increment_token()?);
    filter.end()?;
    filter.close()?;
    Ok(())
}

fn delimited_payload_token_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.payloads.TestDelimitedPayloadTokenFilter")
        .method(t("testPayloads", dptf_test_payloads))
        .method(t("testNext", dptf_test_next))
        .method(t("testFloatEncoding", dptf_test_float_encoding))
        .method(t("testIntEncoding", dptf_test_int_encoding))
        .build()
}

// -----------------------------------------------------------------------------
// TestDelimitedPayloadTokenFilterFactory
// -----------------------------------------------------------------------------

/// The `testEncoder`/`testDelim` loop: every payload decodes to 0.1f.
fn check_float_payloads(text: &str, params: &[&str]) -> Result<()> {
    let mut tokenizer = MockTokenizer::new(Arc::clone(&WHITESPACE), false)?;
    tokenizer.set_reader(string_reader(text))?;
    let mut stream = fac("DelimitedPayload", params)?.create(boxed(tokenizer))?;
    stream.reset()?;
    while stream.increment_token()? {
        Assert::assert_true(
            stream
                .attribute_source()
                .has_attribute_interface::<dyn PayloadAttribute>(),
        );
        let pay_data = stream
            .attribute_source()
            .get_attribute_interface::<dyn PayloadAttribute>()
            .expect("payAttr")
            .get_payload()
            .map(|p| p.bytes.clone());
        Assert::assert_true(pay_data.is_some());
        let pay_float = PayloadHelper::decode_float(&pay_data.expect("payData"));
        Assert::assert_equals_float_msg(None, 0.1, pay_float, 0.0);
    }
    stream.end()?;
    stream.close()?;
    Ok(())
}

fn dptff_test_encoder() -> Result<()> {
    check_float_payloads("the|0.1 quick|0.1 red|0.1", &["encoder", "float"])
}

fn dptff_test_delim() -> Result<()> {
    check_float_payloads(
        "the*0.1 quick*0.1 red*0.1",
        &["encoder", "float", "delimiter", "*"],
    )
}

/// Test that bogus arguments result in exception
fn dptff_test_bogus_arguments() -> Result<()> {
    let expected = iae_message(|| {
        fac(
            "DelimitedPayload",
            &["encoder", "float", "bogusArg", "bogusValue"],
        )
    });
    Assert::assert_true(expected.contains("Unknown parameters"));
    Ok(())
}

fn delimited_payload_token_filter_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.payloads.TestDelimitedPayloadTokenFilterFactory")
        .method(t("testEncoder", dptff_test_encoder))
        .method(t("testDelim", dptff_test_delim))
        .method(t("testBogusArguments", dptff_test_bogus_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// TestNumericPayloadTokenFilter, TestTokenOffsetPayloadTokenFilter and
// TestTypeAsPayloadTokenFilter
// -----------------------------------------------------------------------------

const TEN_WORDS: &str = "The quick red fox jumped over the lazy brown dogs";

/// The two `WordTokenFilter`s: `dogs_only` sets type "D" on "dogs"
/// (`TestNumericPayloadTokenFilter`), otherwise the upper-cased first
/// character becomes the type (`TestTypeAsPayloadTokenFilter`).
#[derive(Debug)]
struct WordTokenFilterLogic {
    dogs_only: bool,
}

impl TokenFilterLogic for WordTokenFilterLogic {
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if input.increment_token()? {
            let (term, first) = {
                let t = source
                    .get_attribute_interface::<dyn CharTermAttribute>()
                    .expect("termAtt");
                let term: String = t.buffer()[..t.length()].iter().collect();
                (term, t.buffer()[0])
            };
            let mut ty = source
                .get_attribute_interface_mut::<dyn TypeAttribute>()
                .expect("typeAtt");
            if self.dogs_only {
                if term == "dogs" {
                    ty.set_type("D".into());
                }
            } else {
                // `String.valueOf(Character.toUpperCase(termAtt.buffer()[0]))`
                ty.set_type(first.to_uppercase().collect());
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

fn word_token_filter(
    input: Box<dyn TokenStream>,
    dogs_only: bool,
) -> Result<TokenFilter<WordTokenFilterLogic>> {
    let mut filter = TokenFilter::new(input, WordTokenFilterLogic { dogs_only });
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn CharTermAttribute>()?;
    filter
        .attribute_source_mut()
        .add_attribute_interface::<dyn TypeAttribute>()?;
    Ok(filter)
}

fn nptf_test() -> Result<()> {
    let mut input = MockTokenizer::new(Arc::clone(&WHITESPACE), false)?;
    input.set_reader(string_reader(TEN_WORDS))?;
    let mut nptf =
        new_numeric_payload_token_filter(boxed(word_token_filter(boxed(input), true)?), 3.0, "D")?;
    let mut seen_dogs = false;
    nptf.reset()?;
    while nptf.increment_token()? {
        if term_of(&nptf) == "dogs" {
            seen_dogs = true;
            let ty = type_of(&nptf);
            Assert::assert_true_msg(&format!("{ty} is not equal to D"), ty == "D");
            let payload = nptf
                .attribute_source()
                .get_attribute_interface::<dyn PayloadAttribute>()
                .expect("payloadAtt")
                .get_payload()
                .cloned();
            Assert::assert_true_msg(
                "payloadAtt.getPayload() is null and it shouldn't be",
                payload.is_some(),
            );
            let payload = payload.expect("payload");
            // safe here to just use the bytes, otherwise we should use offset, length
            let bytes = &payload.bytes;
            Assert::assert_true_msg(
                &format!("{} does not equal: {}", bytes.len(), payload.length),
                bytes.len() == payload.length,
            );
            Assert::assert_true_msg(
                &format!("{} does not equal: 0", payload.offset),
                payload.offset == 0,
            );
            let pay = PayloadHelper::decode_float(bytes);
            Assert::assert_true_msg(&format!("{pay:?} does not equal: 3"), pay == 3.0);
        } else {
            let ty = type_of(&nptf);
            Assert::assert_true_msg(&format!("{ty} is not null and it should be"), ty == "word");
        }
    }
    Assert::assert_true_msg(&format!("{seen_dogs} does not equal: true"), seen_dogs);
    Ok(())
}

fn numeric_payload_token_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.payloads.TestNumericPayloadTokenFilter")
        .method(t("test", nptf_test))
        .build()
}

fn toptf_test() -> Result<()> {
    let mut nptf =
        new_token_offset_payload_token_filter(boxed(B::whitespace_mock_tokenizer_str(TEN_WORDS)?))?;
    let mut count = 0;
    nptf.reset()?;
    while nptf.increment_token()? {
        let pay = nptf
            .attribute_source()
            .get_attribute_interface::<dyn PayloadAttribute>()
            .expect("payloadAtt")
            .get_payload()
            .cloned();
        Assert::assert_true_msg("pay is null and it shouldn't be", pay.is_some());
        let data = pay.expect("pay").bytes;
        let (so, eo) = {
            let o = nptf
                .attribute_source()
                .get_attribute_interface::<dyn OffsetAttribute>()
                .expect("offsetAtt");
            (o.start_offset(), o.end_offset())
        };
        let start = PayloadHelper::decode_int(&data, 0);
        Assert::assert_true_msg(&format!("{start} does not equal: {so}"), start == so);
        let end = PayloadHelper::decode_int(&data, 4);
        Assert::assert_true_msg(&format!("{end} does not equal: {eo}"), end == eo);
        count += 1;
    }
    Assert::assert_true_msg(&format!("{count} does not equal: 10"), count == 10);
    Ok(())
}

fn token_offset_payload_token_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.payloads.TestTokenOffsetPayloadTokenFilter")
        .method(t("test", toptf_test))
        .build()
}

fn taptf_test() -> Result<()> {
    let mut nptf = new_type_as_payload_token_filter(boxed(word_token_filter(
        boxed(B::whitespace_mock_tokenizer_str(TEN_WORDS)?),
        false,
    )?))?;
    let mut count = 0;
    nptf.reset()?;
    while nptf.increment_token()? {
        let ty = type_of(&nptf);
        let first: String = term_of(&nptf)
            .chars()
            .next()
            .expect("a term")
            .to_uppercase()
            .collect();
        Assert::assert_true_msg(&format!("{ty} is not null and it should be"), ty == first);
        let payload = nptf
            .attribute_source()
            .get_attribute_interface::<dyn PayloadAttribute>()
            .expect("payloadAtt")
            .get_payload()
            .cloned();
        Assert::assert_true_msg(
            "nextToken.getPayload() is null and it shouldn't be",
            payload.is_some(),
        );
        let ty_payload = payload.expect("payload").utf8_to_string()?;
        Assert::assert_true_msg(
            &format!("{ty_payload} is not equal to {ty}"),
            ty_payload == ty,
        );
        count += 1;
    }
    Assert::assert_true_msg(&format!("{count} does not equal: 10"), count == 10);
    Ok(())
}

fn type_as_payload_token_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.payloads.TestTypeAsPayloadTokenFilter")
        .method(t("test", taptf_test))
        .build()
}

// -----------------------------------------------------------------------------
// TestNGramFilters
// -----------------------------------------------------------------------------

fn ngf_test_ngram_tokenizer() -> Result<()> {
    let reader = string_reader("test");
    let mut stream = F::tokenizer_factory("NGram", &[])?.create()?;
    stream.set_reader(reader)?;
    B::assert_token_stream_contents(&mut stream, &["t", "te", "e", "es", "s", "st", "t"])?;
    Ok(())
}

fn ngf_test_ngram_tokenizer2() -> Result<()> {
    let reader = string_reader("test");
    let mut stream =
        F::tokenizer_factory("NGram", &["minGramSize", "2", "maxGramSize", "3"])?.create()?;
    stream.set_reader(reader)?;
    B::assert_token_stream_contents(&mut stream, &["te", "tes", "es", "est", "st"])?;
    Ok(())
}

fn ngf_test_ngram_filter() -> Result<()> {
    let reader = string_reader("test");
    let mut stream: Box<dyn TokenStream> = boxed(B::whitespace_mock_tokenizer(reader)?);
    stream = fac("NGram", &["minGramSize", "1", "maxGramSize", "2"])?.create(stream)?;
    B::assert_token_stream_contents(&mut *stream, &["t", "te", "e", "es", "s", "st", "t"])?;
    Ok(())
}

fn ngf_test_ngram_filter2() -> Result<()> {
    let reader = string_reader("test");
    let mut stream: Box<dyn TokenStream> = boxed(B::whitespace_mock_tokenizer(reader)?);
    stream = fac("NGram", &["minGramSize", "2", "maxGramSize", "3"])?.create(stream)?;
    B::assert_token_stream_contents(&mut *stream, &["te", "tes", "es", "est", "st"])?;
    Ok(())
}

fn ngf_test_ngram_filter3() -> Result<()> {
    let reader = string_reader("test");
    let mut stream: Box<dyn TokenStream> = boxed(B::whitespace_mock_tokenizer(reader)?);
    stream = fac(
        "NGram",
        &[
            "minGramSize",
            "2",
            "maxGramSize",
            "3",
            "preserveOriginal",
            "true",
        ],
    )?
    .create(stream)?;
    B::assert_token_stream_contents(&mut *stream, &["te", "tes", "es", "est", "st", "test"])?;
    Ok(())
}

fn ngf_test_ngram_filter_payload() -> Result<()> {
    let reader = string_reader("test|0.1");
    let mut stream: Box<dyn TokenStream> = boxed(B::whitespace_mock_tokenizer(reader)?);
    stream = fac("DelimitedPayload", &["encoder", "float"])?.create(stream)?;
    stream = fac("NGram", &["minGramSize", "1", "maxGramSize", "2"])?.create(stream)?;
    stream.reset()?;
    while stream.increment_token()? {
        let pay_data = payload_of(&*stream);
        Assert::assert_true(pay_data.is_some());
        let pay_float = PayloadHelper::decode_float(&pay_data.expect("payload"));
        Assert::assert_equals_float_msg(None, 0.1, pay_float, 0.0);
    }
    stream.end()?;
    stream.close()?;
    Ok(())
}

fn ngf_test_edge_ngram_tokenizer() -> Result<()> {
    let reader = string_reader("test");
    let mut stream = F::tokenizer_factory("EdgeNGram", &[])?.create()?;
    stream.set_reader(reader)?;
    B::assert_token_stream_contents(&mut stream, &["t"])?;
    Ok(())
}

fn ngf_test_edge_ngram_tokenizer2() -> Result<()> {
    let reader = string_reader("test");
    let mut stream =
        F::tokenizer_factory("EdgeNGram", &["minGramSize", "1", "maxGramSize", "2"])?.create()?;
    stream.set_reader(reader)?;
    B::assert_token_stream_contents(&mut stream, &["t", "te"])?;
    Ok(())
}

fn ngf_test_edge_ngram_filter() -> Result<()> {
    let reader = string_reader("test");
    let mut stream: Box<dyn TokenStream> = boxed(B::whitespace_mock_tokenizer(reader)?);
    stream = fac("EdgeNGram", &["minGramSize", "1", "maxGramSize", "1"])?.create(stream)?;
    B::assert_token_stream_contents(&mut *stream, &["t"])?;
    Ok(())
}

fn ngf_test_edge_ngram_filter2() -> Result<()> {
    let reader = string_reader("test");
    let mut stream: Box<dyn TokenStream> = boxed(B::whitespace_mock_tokenizer(reader)?);
    stream = fac("EdgeNGram", &["minGramSize", "1", "maxGramSize", "2"])?.create(stream)?;
    B::assert_token_stream_contents(&mut *stream, &["t", "te"])?;
    Ok(())
}

fn ngf_test_edge_ngram_filter3() -> Result<()> {
    let reader = string_reader("test");
    let mut stream: Box<dyn TokenStream> = boxed(B::whitespace_mock_tokenizer(reader)?);
    stream = fac(
        "EdgeNGram",
        &[
            "minGramSize",
            "1",
            "maxGramSize",
            "2",
            "preserveOriginal",
            "true",
        ],
    )?
    .create(stream)?;
    B::assert_token_stream_contents(&mut *stream, &["t", "te", "test"])?;
    Ok(())
}

fn ngf_test_edge_ngram_filter_payload() -> Result<()> {
    let reader = string_reader("test|0.1");
    let mut stream: Box<dyn TokenStream> = boxed(B::whitespace_mock_tokenizer(reader)?);
    stream = fac("DelimitedPayload", &["encoder", "float"])?.create(stream)?;
    stream = fac("EdgeNGram", &["minGramSize", "1", "maxGramSize", "2"])?.create(stream)?;
    stream.reset()?;
    while stream.increment_token()? {
        let pay_data = payload_of(&*stream);
        Assert::assert_true(pay_data.is_some());
        let pay_float = PayloadHelper::decode_float(&pay_data.expect("payload"));
        Assert::assert_equals_float_msg(None, 0.1, pay_float, 0.0);
    }
    stream.end()?;
    stream.close()?;
    Ok(())
}

fn ngf_test_bogus_arguments() -> Result<()> {
    let expected = iae_message(|| F::tokenizer_factory("NGram", &["bogusArg", "bogusValue"]));
    Assert::assert_true(expected.contains("Unknown parameters"));
    let expected = iae_message(|| F::tokenizer_factory("EdgeNGram", &["bogusArg", "bogusValue"]));
    Assert::assert_true(expected.contains("Unknown parameters"));
    let expected = iae_message(|| {
        fac(
            "NGram",
            &[
                "minGramSize",
                "2",
                "maxGramSize",
                "5",
                "bogusArg",
                "bogusValue",
            ],
        )
    });
    Assert::assert_true(expected.contains("Unknown parameters"));
    let expected = iae_message(|| {
        fac(
            "EdgeNGram",
            &[
                "minGramSize",
                "2",
                "maxGramSize",
                "5",
                "bogusArg",
                "bogusValue",
            ],
        )
    });
    Assert::assert_true(expected.contains("Unknown parameters"));
    Ok(())
}
fn ngram_filters_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.ngram.TestNGramFilters")
        .method(t("testNGramTokenizer", ngf_test_ngram_tokenizer))
        .method(t("testNGramTokenizer2", ngf_test_ngram_tokenizer2))
        .method(t("testNGramFilter", ngf_test_ngram_filter))
        .method(t("testNGramFilter2", ngf_test_ngram_filter2))
        .method(t("testNGramFilter3", ngf_test_ngram_filter3))
        .method(t("testNGramFilterPayload", ngf_test_ngram_filter_payload))
        .method(t("testEdgeNGramTokenizer", ngf_test_edge_ngram_tokenizer))
        .method(t("testEdgeNGramTokenizer2", ngf_test_edge_ngram_tokenizer2))
        .method(t("testEdgeNGramFilter", ngf_test_edge_ngram_filter))
        .method(t("testEdgeNGramFilter2", ngf_test_edge_ngram_filter2))
        .method(t("testEdgeNGramFilter3", ngf_test_edge_ngram_filter3))
        .method(t(
            "testEdgeNGramFilterPayload",
            ngf_test_edge_ngram_filter_payload,
        ))
        .method(t("testBogusArguments", ngf_test_bogus_arguments))
        .build()
}

// -----------------------------------------------------------------------------
// TestPathHierarchyTokenizer and TestReversePathHierarchyTokenizer
// -----------------------------------------------------------------------------

/// `new Analyzer() { tokenizer = make(); return new TokenStreamComponents(tokenizer[, tokenizer]); }`.
fn path_analyzer<T: rucene_test_framework::analysis::ReaderTokenizer + 'static>(
    name: &'static str,
    make: impl Fn() -> Result<T> + Send + Sync + 'static,
) -> Arc<dyn Analyzer> {
    FnAnalyzer::new(name, move |_| Ok(tokenizer_components(own(make()?)))).arc()
}

fn pht_test_basic() -> Result<()> {
    let path = "/a/b/c";
    let mut t = new_path_hierarchy_tokenizer_factory(
        B::new_attribute_factory(),
        DEFAULT_DELIMITER,
        DEFAULT_DELIMITER,
        DEFAULT_SKIP,
    )?;
    t.set_reader(string_reader(path))?;
    B::assert_token_stream_contents_offsets_pos_increments_final_offset(
        &mut t,
        &["/a", "/a/b", "/a/b/c"],
        &[0, 0, 0],
        &[2, 4, 6],
        &[1, 1, 1],
        Some(path.len() as i32),
    )?;
    Ok(())
}

fn pht_test_end_of_delimiter() -> Result<()> {
    let path = "/a/b/c/";
    let mut t = new_path_hierarchy_tokenizer_factory(
        B::new_attribute_factory(),
        DEFAULT_DELIMITER,
        DEFAULT_DELIMITER,
        DEFAULT_SKIP,
    )?;
    t.set_reader(string_reader(path))?;
    B::assert_token_stream_contents_offsets_pos_increments_final_offset(
        &mut t,
        &["/a", "/a/b", "/a/b/c", "/a/b/c/"],
        &[0, 0, 0, 0],
        &[2, 4, 6, 7],
        &[1, 1, 1, 1],
        Some(path.len() as i32),
    )?;
    Ok(())
}

fn pht_test_start_of_char() -> Result<()> {
    let path = "a/b/c";
    let mut t = new_path_hierarchy_tokenizer_factory(
        B::new_attribute_factory(),
        DEFAULT_DELIMITER,
        DEFAULT_DELIMITER,
        DEFAULT_SKIP,
    )?;
    t.set_reader(string_reader(path))?;
    B::assert_token_stream_contents_offsets_pos_increments_final_offset(
        &mut t,
        &["a", "a/b", "a/b/c"],
        &[0, 0, 0],
        &[1, 3, 5],
        &[1, 1, 1],
        Some(path.len() as i32),
    )?;
    Ok(())
}

fn pht_test_start_of_char_end_of_delimiter() -> Result<()> {
    let path = "a/b/c/";
    let mut t = new_path_hierarchy_tokenizer_factory(
        B::new_attribute_factory(),
        DEFAULT_DELIMITER,
        DEFAULT_DELIMITER,
        DEFAULT_SKIP,
    )?;
    t.set_reader(string_reader(path))?;
    B::assert_token_stream_contents_offsets_pos_increments_final_offset(
        &mut t,
        &["a", "a/b", "a/b/c", "a/b/c/"],
        &[0, 0, 0, 0],
        &[1, 3, 5, 6],
        &[1, 1, 1, 1],
        Some(path.len() as i32),
    )?;
    Ok(())
}

fn pht_test_only_delimiter() -> Result<()> {
    let path = "/";
    let mut t = new_path_hierarchy_tokenizer_factory(
        B::new_attribute_factory(),
        DEFAULT_DELIMITER,
        DEFAULT_DELIMITER,
        DEFAULT_SKIP,
    )?;
    t.set_reader(string_reader(path))?;
    B::assert_token_stream_contents_offsets_pos_increments_final_offset(
        &mut t,
        &["/"],
        &[0],
        &[1],
        &[1],
        Some(path.len() as i32),
    )?;
    Ok(())
}

fn pht_test_only_delimiters() -> Result<()> {
    let path = "//";
    let mut t = new_path_hierarchy_tokenizer_factory(
        B::new_attribute_factory(),
        DEFAULT_DELIMITER,
        DEFAULT_DELIMITER,
        DEFAULT_SKIP,
    )?;
    t.set_reader(string_reader(path))?;
    B::assert_token_stream_contents_offsets_pos_increments_final_offset(
        &mut t,
        &["/", "//"],
        &[0, 0],
        &[1, 2],
        &[1, 1],
        Some(path.len() as i32),
    )?;
    Ok(())
}

fn pht_test_replace() -> Result<()> {
    let path = "/a/b/c";
    let mut t = new_path_hierarchy_tokenizer_factory(
        B::new_attribute_factory(),
        '/' as u16,
        '\\' as u16,
        DEFAULT_SKIP,
    )?;
    t.set_reader(string_reader(path))?;
    B::assert_token_stream_contents_offsets_pos_increments_final_offset(
        &mut t,
        &["\\a", "\\a\\b", "\\a\\b\\c"],
        &[0, 0, 0],
        &[2, 4, 6],
        &[1, 1, 1],
        Some(path.len() as i32),
    )?;
    Ok(())
}

fn pht_test_windows_path() -> Result<()> {
    let path = "c:\\a\\b\\c";
    let mut t = new_path_hierarchy_tokenizer_factory(
        B::new_attribute_factory(),
        '\\' as u16,
        '\\' as u16,
        DEFAULT_SKIP,
    )?;
    t.set_reader(string_reader(path))?;
    B::assert_token_stream_contents_offsets_pos_increments_final_offset(
        &mut t,
        &["c:", "c:\\a", "c:\\a\\b", "c:\\a\\b\\c"],
        &[0, 0, 0, 0],
        &[2, 4, 6, 8],
        &[1, 1, 1, 1],
        Some(path.len() as i32),
    )?;
    Ok(())
}

fn pht_test_normalize_win_delim_to_linux_delim() -> Result<()> {
    let mut builder = NormalizeCharMapBuilder::new();
    builder.add("\\", "/")?;
    let norm_map = Arc::new(builder.build()?);
    let path = "c:\\a\\b\\c";
    let cs: Box<dyn CharReader> = Box::new(MappingCharFilter::new(norm_map, string_reader(path)));
    let mut t = new_path_hierarchy_tokenizer_factory(
        B::new_attribute_factory(),
        DEFAULT_DELIMITER,
        DEFAULT_DELIMITER,
        DEFAULT_SKIP,
    )?;
    t.set_reader(cs)?;
    B::assert_token_stream_contents_offsets_pos_increments_final_offset(
        &mut t,
        &["c:", "c:/a", "c:/a/b", "c:/a/b/c"],
        &[0, 0, 0, 0],
        &[2, 4, 6, 8],
        &[1, 1, 1, 1],
        Some(path.len() as i32),
    )
}

fn pht_test_basic_skip() -> Result<()> {
    let path = "/a/b/c";
    let mut t = new_path_hierarchy_tokenizer_factory(
        B::new_attribute_factory(),
        DEFAULT_DELIMITER,
        DEFAULT_DELIMITER,
        1,
    )?;
    t.set_reader(string_reader(path))?;
    B::assert_token_stream_contents_offsets_pos_increments_final_offset(
        &mut t,
        &["/b", "/b/c"],
        &[2, 2],
        &[4, 6],
        &[1, 1],
        Some(path.len() as i32),
    )?;
    Ok(())
}

fn pht_test_end_of_delimiter_skip() -> Result<()> {
    let path = "/a/b/c/";
    let mut t = new_path_hierarchy_tokenizer_factory(
        B::new_attribute_factory(),
        DEFAULT_DELIMITER,
        DEFAULT_DELIMITER,
        1,
    )?;
    t.set_reader(string_reader(path))?;
    B::assert_token_stream_contents_offsets_pos_increments_final_offset(
        &mut t,
        &["/b", "/b/c", "/b/c/"],
        &[2, 2, 2],
        &[4, 6, 7],
        &[1, 1, 1],
        Some(path.len() as i32),
    )?;
    Ok(())
}

fn pht_test_start_of_char_skip() -> Result<()> {
    let path = "a/b/c";
    let mut t = new_path_hierarchy_tokenizer_factory(
        B::new_attribute_factory(),
        DEFAULT_DELIMITER,
        DEFAULT_DELIMITER,
        1,
    )?;
    t.set_reader(string_reader(path))?;
    B::assert_token_stream_contents_offsets_pos_increments_final_offset(
        &mut t,
        &["/b", "/b/c"],
        &[1, 1],
        &[3, 5],
        &[1, 1],
        Some(path.len() as i32),
    )?;
    Ok(())
}

fn pht_test_start_of_char_end_of_delimiter_skip() -> Result<()> {
    let path = "a/b/c/";
    let mut t = new_path_hierarchy_tokenizer_factory(
        B::new_attribute_factory(),
        DEFAULT_DELIMITER,
        DEFAULT_DELIMITER,
        1,
    )?;
    t.set_reader(string_reader(path))?;
    B::assert_token_stream_contents_offsets_pos_increments_final_offset(
        &mut t,
        &["/b", "/b/c", "/b/c/"],
        &[1, 1, 1],
        &[3, 5, 6],
        &[1, 1, 1],
        Some(path.len() as i32),
    )?;
    Ok(())
}

fn pht_test_only_delimiter_skip() -> Result<()> {
    let path = "/";
    let mut t = new_path_hierarchy_tokenizer_factory(
        B::new_attribute_factory(),
        DEFAULT_DELIMITER,
        DEFAULT_DELIMITER,
        1,
    )?;
    t.set_reader(string_reader(path))?;
    B::assert_token_stream_contents_offsets_pos_increments_final_offset(
        &mut t,
        &[],
        &[],
        &[],
        &[],
        Some(path.len() as i32),
    )?;
    Ok(())
}

fn pht_test_only_delimiters_skip() -> Result<()> {
    let path = "//";
    let mut t = new_path_hierarchy_tokenizer_factory(
        B::new_attribute_factory(),
        DEFAULT_DELIMITER,
        DEFAULT_DELIMITER,
        1,
    )?;
    t.set_reader(string_reader(path))?;
    B::assert_token_stream_contents_offsets_pos_increments_final_offset(
        &mut t,
        &["/"],
        &[1],
        &[2],
        &[1],
        Some(path.len() as i32),
    )?;
    Ok(())
}

fn pht_test_random_strings() -> Result<()> {
    let a = path_analyzer(
        "org.apache.lucene.analysis.path.TestPathHierarchyTokenizer$1",
        || {
            new_path_hierarchy_tokenizer_factory(
                B::new_attribute_factory(),
                DEFAULT_DELIMITER,
                DEFAULT_DELIMITER,
                DEFAULT_SKIP,
            )
        },
    );
    // TODO: properly support positionLengthAttribute
    B::check_random_data_graph(&*random(), &a, 200 * mult(), 20, false, false)?;
    a.close();
    Ok(())
}

fn pht_test_random_huge_strings() -> Result<()> {
    let random = random();
    let a = path_analyzer(
        "org.apache.lucene.analysis.path.TestPathHierarchyTokenizer$2",
        || {
            new_path_hierarchy_tokenizer_factory(
                B::new_attribute_factory(),
                DEFAULT_DELIMITER,
                DEFAULT_DELIMITER,
                DEFAULT_SKIP,
            )
        },
    );
    // TODO: properly support positionLengthAttribute
    B::check_random_data_graph(&*random, &a, 100 * mult(), 1027, false, false)?;
    a.close();
    Ok(())
}

fn pht_test_tokenizer_via_analyzer_output() -> Result<()> {
    let analyzer = path_analyzer(
        "org.apache.lucene.analysis.path.TestPathHierarchyTokenizer$3",
        || Ok(new_path_hierarchy_tokenizer()),
    );
    B::assert_analyzes_to(&*analyzer, "a/b/c", &["a", "a/b", "a/b/c"])?;
    B::assert_analyzes_to(&*analyzer, "a/b/c/", &["a", "a/b", "a/b/c", "a/b/c/"])?;
    B::assert_analyzes_to(&*analyzer, "/a/b/c", &["/a", "/a/b", "/a/b/c"])?;
    B::assert_analyzes_to(&*analyzer, "/a/b/c/", &["/a", "/a/b", "/a/b/c", "/a/b/c/"])?;
    Ok(())
}
fn path_hierarchy_tokenizer_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.path.TestPathHierarchyTokenizer")
        .method(t("testBasic", pht_test_basic))
        .method(t("testEndOfDelimiter", pht_test_end_of_delimiter))
        .method(t("testStartOfChar", pht_test_start_of_char))
        .method(t(
            "testStartOfCharEndOfDelimiter",
            pht_test_start_of_char_end_of_delimiter,
        ))
        .method(t("testOnlyDelimiter", pht_test_only_delimiter))
        .method(t("testOnlyDelimiters", pht_test_only_delimiters))
        .method(t("testReplace", pht_test_replace))
        .method(t("testWindowsPath", pht_test_windows_path))
        .method(t(
            "testNormalizeWinDelimToLinuxDelim",
            pht_test_normalize_win_delim_to_linux_delim,
        ))
        .method(t("testBasicSkip", pht_test_basic_skip))
        .method(t("testEndOfDelimiterSkip", pht_test_end_of_delimiter_skip))
        .method(t("testStartOfCharSkip", pht_test_start_of_char_skip))
        .method(t(
            "testStartOfCharEndOfDelimiterSkip",
            pht_test_start_of_char_end_of_delimiter_skip,
        ))
        .method(t("testOnlyDelimiterSkip", pht_test_only_delimiter_skip))
        .method(t("testOnlyDelimitersSkip", pht_test_only_delimiters_skip))
        .method(t("testRandomStrings", pht_test_random_strings))
        .method(t("testRandomHugeStrings", pht_test_random_huge_strings))
        .method(t(
            "testTokenizerViaAnalyzerOutput",
            pht_test_tokenizer_via_analyzer_output,
        ))
        .build()
}
fn rpht_test_basic_reverse() -> Result<()> {
    let path = "/a/b/c";
    let mut t = new_reverse_path_hierarchy_tokenizer_factory(
        B::new_attribute_factory(),
        DEFAULT_DELIMITER,
        DEFAULT_DELIMITER,
        DEFAULT_SKIP,
    )?;
    t.set_reader(string_reader(path))?;
    B::assert_token_stream_contents_offsets_pos_increments_final_offset(
        &mut t,
        &["/a/b/c", "a/b/c", "b/c", "c"],
        &[0, 1, 3, 5],
        &[6, 6, 6, 6],
        &[1, 1, 1, 1],
        Some(path.len() as i32),
    )?;
    Ok(())
}

fn rpht_test_end_of_delimiter_reverse() -> Result<()> {
    let path = "/a/b/c/";
    let mut t = new_reverse_path_hierarchy_tokenizer_factory(
        B::new_attribute_factory(),
        DEFAULT_DELIMITER,
        DEFAULT_DELIMITER,
        DEFAULT_SKIP,
    )?;
    t.set_reader(string_reader(path))?;
    B::assert_token_stream_contents_offsets_pos_increments_final_offset(
        &mut t,
        &["/a/b/c/", "a/b/c/", "b/c/", "c/"],
        &[0, 1, 3, 5],
        &[7, 7, 7, 7],
        &[1, 1, 1, 1],
        Some(path.len() as i32),
    )?;
    Ok(())
}

fn rpht_test_start_of_char_reverse() -> Result<()> {
    let path = "a/b/c";
    let mut t = new_reverse_path_hierarchy_tokenizer_factory(
        B::new_attribute_factory(),
        DEFAULT_DELIMITER,
        DEFAULT_DELIMITER,
        DEFAULT_SKIP,
    )?;
    t.set_reader(string_reader(path))?;
    B::assert_token_stream_contents_offsets_pos_increments_final_offset(
        &mut t,
        &["a/b/c", "b/c", "c"],
        &[0, 2, 4],
        &[5, 5, 5],
        &[1, 1, 1],
        Some(path.len() as i32),
    )?;
    Ok(())
}

fn rpht_test_start_of_char_end_of_delimiter_reverse() -> Result<()> {
    let path = "a/b/c/";
    let mut t = new_reverse_path_hierarchy_tokenizer_factory(
        B::new_attribute_factory(),
        DEFAULT_DELIMITER,
        DEFAULT_DELIMITER,
        DEFAULT_SKIP,
    )?;
    t.set_reader(string_reader(path))?;
    B::assert_token_stream_contents_offsets_pos_increments_final_offset(
        &mut t,
        &["a/b/c/", "b/c/", "c/"],
        &[0, 2, 4],
        &[6, 6, 6],
        &[1, 1, 1],
        Some(path.len() as i32),
    )?;
    Ok(())
}

fn rpht_test_only_delimiter_reverse() -> Result<()> {
    let path = "/";
    let mut t = new_reverse_path_hierarchy_tokenizer_factory(
        B::new_attribute_factory(),
        DEFAULT_DELIMITER,
        DEFAULT_DELIMITER,
        DEFAULT_SKIP,
    )?;
    t.set_reader(string_reader(path))?;
    B::assert_token_stream_contents_offsets_pos_increments_final_offset(
        &mut t,
        &["/"],
        &[0],
        &[1],
        &[1],
        Some(path.len() as i32),
    )?;
    Ok(())
}

fn rpht_test_only_delimiters_reverse() -> Result<()> {
    let path = "//";
    let mut t = new_reverse_path_hierarchy_tokenizer_factory(
        B::new_attribute_factory(),
        DEFAULT_DELIMITER,
        DEFAULT_DELIMITER,
        DEFAULT_SKIP,
    )?;
    t.set_reader(string_reader(path))?;
    B::assert_token_stream_contents_offsets_pos_increments_final_offset(
        &mut t,
        &["//", "/"],
        &[0, 1],
        &[2, 2],
        &[1, 1],
        Some(path.len() as i32),
    )?;
    Ok(())
}

fn rpht_test_end_of_delimiter_reverse_skip() -> Result<()> {
    let path = "/a/b/c/";
    let mut t = new_reverse_path_hierarchy_tokenizer_factory(
        B::new_attribute_factory(),
        DEFAULT_DELIMITER,
        DEFAULT_DELIMITER,
        1,
    )?;
    t.set_reader(string_reader(path))?;
    drop(string_reader(path)); // the Java test's discarded `new StringReader(path);`
    B::assert_token_stream_contents_offsets_pos_increments_final_offset(
        &mut t,
        &["/a/b/", "a/b/", "b/"],
        &[0, 1, 3],
        &[5, 5, 5],
        &[1, 1, 1],
        Some(path.len() as i32),
    )?;
    Ok(())
}

fn rpht_test_start_of_char_reverse_skip() -> Result<()> {
    let path = "a/b/c";
    let mut t = new_reverse_path_hierarchy_tokenizer_factory(
        B::new_attribute_factory(),
        DEFAULT_DELIMITER,
        DEFAULT_DELIMITER,
        1,
    )?;
    t.set_reader(string_reader(path))?;
    B::assert_token_stream_contents_offsets_pos_increments_final_offset(
        &mut t,
        &["a/b/", "b/"],
        &[0, 2],
        &[4, 4],
        &[1, 1],
        Some(path.len() as i32),
    )?;
    Ok(())
}

fn rpht_test_start_of_char_end_of_delimiter_reverse_skip() -> Result<()> {
    let path = "a/b/c/";
    let mut t = new_reverse_path_hierarchy_tokenizer_factory(
        B::new_attribute_factory(),
        DEFAULT_DELIMITER,
        DEFAULT_DELIMITER,
        1,
    )?;
    t.set_reader(string_reader(path))?;
    B::assert_token_stream_contents_offsets_pos_increments_final_offset(
        &mut t,
        &["a/b/", "b/"],
        &[0, 2],
        &[4, 4],
        &[1, 1],
        Some(path.len() as i32),
    )?;
    Ok(())
}

fn rpht_test_only_delimiter_reverse_skip() -> Result<()> {
    let path = "/";
    let mut t = new_reverse_path_hierarchy_tokenizer_factory(
        B::new_attribute_factory(),
        DEFAULT_DELIMITER,
        DEFAULT_DELIMITER,
        1,
    )?;
    t.set_reader(string_reader(path))?;
    B::assert_token_stream_contents_offsets_pos_increments_final_offset(
        &mut t,
        &[],
        &[],
        &[],
        &[],
        Some(path.len() as i32),
    )?;
    Ok(())
}

fn rpht_test_only_delimiters_reverse_skip() -> Result<()> {
    let path = "//";
    let mut t = new_reverse_path_hierarchy_tokenizer_factory(
        B::new_attribute_factory(),
        DEFAULT_DELIMITER,
        DEFAULT_DELIMITER,
        1,
    )?;
    t.set_reader(string_reader(path))?;
    B::assert_token_stream_contents_offsets_pos_increments_final_offset(
        &mut t,
        &["/"],
        &[0],
        &[1],
        &[1],
        Some(path.len() as i32),
    )?;
    Ok(())
}

fn rpht_test_reverse_skip2() -> Result<()> {
    let path = "/a/b/c/";
    let mut t = new_reverse_path_hierarchy_tokenizer_factory(
        B::new_attribute_factory(),
        DEFAULT_DELIMITER,
        DEFAULT_DELIMITER,
        2,
    )?;
    t.set_reader(string_reader(path))?;
    B::assert_token_stream_contents_offsets_pos_increments_final_offset(
        &mut t,
        &["/a/", "a/"],
        &[0, 1],
        &[3, 3],
        &[1, 1],
        Some(path.len() as i32),
    )?;
    Ok(())
}

fn rpht_test_random_strings() -> Result<()> {
    let a = path_analyzer(
        "org.apache.lucene.analysis.path.TestReversePathHierarchyTokenizer$1",
        || {
            new_reverse_path_hierarchy_tokenizer_factory(
                B::new_attribute_factory(),
                DEFAULT_DELIMITER,
                DEFAULT_DELIMITER,
                DEFAULT_SKIP,
            )
        },
    );
    // TODO: properly support positionLengthAttribute
    B::check_random_data_graph(&*random(), &a, 200 * mult(), 20, false, false)?;
    a.close();
    Ok(())
}

fn rpht_test_random_huge_strings() -> Result<()> {
    let random = random();
    let a = path_analyzer(
        "org.apache.lucene.analysis.path.TestReversePathHierarchyTokenizer$2",
        || {
            new_reverse_path_hierarchy_tokenizer_factory(
                B::new_attribute_factory(),
                DEFAULT_DELIMITER,
                DEFAULT_DELIMITER,
                DEFAULT_SKIP,
            )
        },
    );
    // TODO: properly support positionLengthAttribute
    B::check_random_data_graph(&*random, &a, 100 * mult(), 1027, false, false)?;
    a.close();
    Ok(())
}

fn rpht_test_tokenizer_via_analyzer_output() -> Result<()> {
    let analyzer = path_analyzer(
        "org.apache.lucene.analysis.path.TestReversePathHierarchyTokenizer$3",
        || Ok(new_reverse_path_hierarchy_tokenizer()),
    );
    B::assert_analyzes_to(&*analyzer, "a/b/c", &["a/b/c", "b/c", "c"])?;
    B::assert_analyzes_to(&*analyzer, "a/b/c/", &["a/b/c/", "b/c/", "c/"])?;
    B::assert_analyzes_to(&*analyzer, "/a/b/c", &["/a/b/c", "a/b/c", "b/c", "c"])?;
    B::assert_analyzes_to(&*analyzer, "/a/b/c/", &["/a/b/c/", "a/b/c/", "b/c/", "c/"])?;
    Ok(())
}
fn reverse_path_hierarchy_tokenizer_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.path.TestReversePathHierarchyTokenizer")
        .method(t("testBasicReverse", rpht_test_basic_reverse))
        .method(t(
            "testEndOfDelimiterReverse",
            rpht_test_end_of_delimiter_reverse,
        ))
        .method(t("testStartOfCharReverse", rpht_test_start_of_char_reverse))
        .method(t(
            "testStartOfCharEndOfDelimiterReverse",
            rpht_test_start_of_char_end_of_delimiter_reverse,
        ))
        .method(t(
            "testOnlyDelimiterReverse",
            rpht_test_only_delimiter_reverse,
        ))
        .method(t(
            "testOnlyDelimitersReverse",
            rpht_test_only_delimiters_reverse,
        ))
        .method(t(
            "testEndOfDelimiterReverseSkip",
            rpht_test_end_of_delimiter_reverse_skip,
        ))
        .method(t(
            "testStartOfCharReverseSkip",
            rpht_test_start_of_char_reverse_skip,
        ))
        .method(t(
            "testStartOfCharEndOfDelimiterReverseSkip",
            rpht_test_start_of_char_end_of_delimiter_reverse_skip,
        ))
        .method(t(
            "testOnlyDelimiterReverseSkip",
            rpht_test_only_delimiter_reverse_skip,
        ))
        .method(t(
            "testOnlyDelimitersReverseSkip",
            rpht_test_only_delimiters_reverse_skip,
        ))
        .method(t("testReverseSkip2", rpht_test_reverse_skip2))
        .method(t("testRandomStrings", rpht_test_random_strings))
        .method(t("testRandomHugeStrings", rpht_test_random_huge_strings))
        .method(t(
            "testTokenizerViaAnalyzerOutput",
            rpht_test_tokenizer_via_analyzer_output,
        ))
        .build()
}

// -----------------------------------------------------------------------------
// TestQueryAutoStopWordAnalyzer
// -----------------------------------------------------------------------------

const VARIED_FIELD_VALUES: [&str; 10] = [
    "the", "quick", "brown", "fox", "jumped", "over", "the", "lazy", "boring", "dog",
];
const REPETITIVE_FIELD_VALUES: [&str; 3] = ["boring", "boring", "vaguelyboring"];

type QaswState = (Arc<dyn Directory>, Arc<dyn Analyzer>, Arc<dyn IndexReader>);

thread_local! {
    /// The fields `dir`, `appAnalyzer` and `reader`.
    static QASW: RefCell<Option<QaswState>> = const { RefCell::new(None) };
}

fn app_analyzer() -> Arc<dyn Analyzer> {
    QASW.with(|q| Arc::clone(&q.borrow().as_ref().expect("setUp").1))
}

fn qasw_reader() -> Arc<dyn IndexReader> {
    QASW.with(|q| Arc::clone(&q.borrow().as_ref().expect("setUp").2))
}

fn qasw_set_up() -> Result<()> {
    LuceneTestCase::super_set_up();
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let app_analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::with_automaton(
        &*random(),
        Arc::clone(&WHITESPACE),
        false,
    ));
    let writer = IndexWriter::new(
        Arc::clone(&dir),
        IndexWriterConfig::with_analyzer(Some(Arc::clone(&app_analyzer))),
    )?;
    let num_docs = 200;
    for i in 0..num_docs {
        let mut doc = Document::new();
        let varied_field_value = VARIED_FIELD_VALUES[i % VARIED_FIELD_VALUES.len()];
        let repetitive_field_value = REPETITIVE_FIELD_VALUES[i % REPETITIVE_FIELD_VALUES.len()];
        doc.add(Box::new(TextField::new(
            "variedField",
            varied_field_value.to_string(),
            Store::YES,
        )?));
        doc.add(Box::new(TextField::new(
            "repetitiveField",
            repetitive_field_value.to_string(),
            Store::YES,
        )?));
        writer.add_document(&doc)?;
    }
    writer.close()?;
    let reader: Arc<dyn IndexReader> = directory_reader::open(Arc::clone(&dir))?;
    QASW.with(|q| *q.borrow_mut() = Some((dir, app_analyzer, reader)));
    Ok(())
}

fn qasw_tear_down() -> Result<()> {
    if let Some((_dir, app_analyzer, reader)) = QASW.with(|q| q.borrow_mut().take()) {
        app_analyzer.close();
        reader.close()?;
    }
    LuceneTestCase::super_tear_down();
    Ok(())
}

/// `assertTokenStreamContents(protectedAnalyzer.tokenStream(field, text), output)`.
fn assert_protected(
    a: &QueryAutoStopWordAnalyzer,
    field: &str,
    text: &str,
    output: &[&str],
) -> Result<()> {
    let ts = a.token_stream(field, string_reader(text))?;
    let mut ts = ts.borrow_mut();
    B::assert_token_stream_contents(&mut *ts, output)
}

fn fields(names: &[&str]) -> Vec<String> {
    names.iter().map(|s| s.to_string()).collect()
}

fn qasw_test_no_stopwords() -> Result<()> {
    // Note: an empty list of fields passed in
    let protected_analyzer = QueryAutoStopWordAnalyzer::with_fields_max_doc_freq(
        app_analyzer(),
        &qasw_reader(),
        &[],
        1,
    )?;
    assert_protected(&protected_analyzer, "variedField", "quick", &["quick"])?;
    assert_protected(
        &protected_analyzer,
        "repetitiveField",
        "boring",
        &["boring"],
    )?;
    protected_analyzer.close();
    Ok(())
}

fn qasw_test_default_stopwords_all_fields() -> Result<()> {
    let protected_analyzer = QueryAutoStopWordAnalyzer::new(app_analyzer(), &qasw_reader())?;
    // Default stop word filtering will remove boring
    assert_protected(&protected_analyzer, "repetitiveField", "boring", &[])?;
    protected_analyzer.close();
    Ok(())
}

fn qasw_test_stopwords_all_fields_max_percent_docs() -> Result<()> {
    let protected_analyzer = QueryAutoStopWordAnalyzer::with_max_percent_docs(
        app_analyzer(),
        &qasw_reader(),
        1f32 / 2f32,
    )?;
    // A filter on terms in > one half of docs remove boring
    assert_protected(&protected_analyzer, "repetitiveField", "boring", &[])?;
    // A filter on terms in > half of docs should not remove vaguelyBoring
    assert_protected(
        &protected_analyzer,
        "repetitiveField",
        "vaguelyboring",
        &["vaguelyboring"],
    )?;
    protected_analyzer.close();
    let protected_analyzer = QueryAutoStopWordAnalyzer::with_max_percent_docs(
        app_analyzer(),
        &qasw_reader(),
        1f32 / 4f32,
    )?;
    // A filter on terms in > quarter of docs should remove vaguelyBoring
    assert_protected(&protected_analyzer, "repetitiveField", "vaguelyboring", &[])?;
    protected_analyzer.close();
    Ok(())
}

fn qasw_test_stopwords_per_field_max_percent_docs() -> Result<()> {
    let protected_analyzer = QueryAutoStopWordAnalyzer::with_fields_max_percent_docs(
        app_analyzer(),
        &qasw_reader(),
        &fields(&["variedField"]),
        1f32 / 2f32,
    )?;
    // A filter on one Field should not affect queries on another
    assert_protected(
        &protected_analyzer,
        "repetitiveField",
        "boring",
        &["boring"],
    )?;
    protected_analyzer.close();
    let protected_analyzer = QueryAutoStopWordAnalyzer::with_fields_max_percent_docs(
        app_analyzer(),
        &qasw_reader(),
        &fields(&["variedField", "repetitiveField"]),
        1f32 / 2f32,
    )?;
    // A filter on the right Field should affect queries on it
    assert_protected(&protected_analyzer, "repetitiveField", "boring", &[])?;
    protected_analyzer.close();
    Ok(())
}

fn qasw_test_stopwords_per_field_max_doc_freq() -> Result<()> {
    let protected_analyzer = QueryAutoStopWordAnalyzer::with_fields_max_doc_freq(
        app_analyzer(),
        &qasw_reader(),
        &fields(&["repetitiveField"]),
        10,
    )?;
    let num_stop_words = protected_analyzer.get_stop_words("repetitiveField").len();
    Assert::assert_true_msg("Should have identified stop words", num_stop_words > 0);
    protected_analyzer.close();
    let protected_analyzer = QueryAutoStopWordAnalyzer::with_fields_max_doc_freq(
        app_analyzer(),
        &qasw_reader(),
        &fields(&["repetitiveField", "variedField"]),
        10,
    )?;
    let num_new_stop_words = protected_analyzer.get_stop_words("repetitiveField").len()
        + protected_analyzer.get_stop_words("variedField").len();
    Assert::assert_true_msg(
        "Should have identified more stop words",
        num_new_stop_words > num_stop_words,
    );
    protected_analyzer.close();
    Ok(())
}

fn qasw_test_no_field_name_pollution() -> Result<()> {
    let protected_analyzer = QueryAutoStopWordAnalyzer::with_fields_max_doc_freq(
        app_analyzer(),
        &qasw_reader(),
        &fields(&["repetitiveField"]),
        10,
    )?;
    // Check filter set up OK
    assert_protected(&protected_analyzer, "repetitiveField", "boring", &[])?;
    // Filter should not prevent stopwords in one field being used in another
    assert_protected(&protected_analyzer, "variedField", "boring", &["boring"])?;
    protected_analyzer.close();
    Ok(())
}

fn qasw_test_token_stream() -> Result<()> {
    let a = QueryAutoStopWordAnalyzer::with_max_doc_freq(
        Arc::new(MockAnalyzer::with_automaton(
            &*random(),
            Arc::clone(&WHITESPACE),
            false,
        )),
        &qasw_reader(),
        10,
    )?;
    assert_protected(&a, "repetitiveField", "this boring", &["this"])?;
    a.close();
    Ok(())
}

fn query_auto_stop_word_analyzer_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.query.TestQueryAutoStopWordAnalyzer")
        .method(hook("setUp", qasw_set_up))
        .method(hook("tearDown", qasw_tear_down))
        .method(t("testNoStopwords", qasw_test_no_stopwords))
        .method(t(
            "testDefaultStopwordsAllFields",
            qasw_test_default_stopwords_all_fields,
        ))
        .method(t(
            "testStopwordsAllFieldsMaxPercentDocs",
            qasw_test_stopwords_all_fields_max_percent_docs,
        ))
        .method(t(
            "testStopwordsPerFieldMaxPercentDocs",
            qasw_test_stopwords_per_field_max_percent_docs,
        ))
        .method(t(
            "testStopwordsPerFieldMaxDocFreq",
            qasw_test_stopwords_per_field_max_doc_freq,
        ))
        .method(t(
            "testNoFieldNamePollution",
            qasw_test_no_field_name_pollution,
        ))
        .method(t("testTokenStream", qasw_test_token_stream))
        .build()
}

// -----------------------------------------------------------------------------
// TestReverseStringFilter
// -----------------------------------------------------------------------------

fn rsf_stream() -> Result<Box<dyn TokenStream>> {
    let mut stream = MockTokenizer::new(Arc::clone(&WHITESPACE), false)?; // 1-4 length string
    stream.set_reader(string_reader("Do have a nice day"))?;
    Ok(boxed(stream))
}

fn rsf_test_filter() -> Result<()> {
    let mut filter = new_reverse_string_filter(rsf_stream()?)?;
    B::assert_token_stream_contents(&mut filter, &["oD", "evah", "a", "ecin", "yad"])
}

fn rsf_test_filter_with_mark() -> Result<()> {
    let mut filter = new_reverse_string_filter_with_marker(rsf_stream()?, 0x0001)?;
    B::assert_token_stream_contents(
        &mut filter,
        &["\u{1}oD", "\u{1}evah", "\u{1}a", "\u{1}ecin", "\u{1}yad"],
    )
}

fn rsf_test_reverse_string() -> Result<()> {
    Assert::assert_equals("A", reverse("A").as_str());
    Assert::assert_equals("BA", reverse("AB").as_str());
    Assert::assert_equals("CBA", reverse("ABC").as_str());
    Ok(())
}

/// `ReverseStringFilter.reverse(buffer, start, len)` then `new String(buffer)`.
fn reverse_chars(s: &str, start: usize, len: usize) -> String {
    let mut buffer: Vec<u16> = s.encode_utf16().collect();
    reverse_range(&mut buffer, start, len);
    String::from_utf16(&buffer).expect("no lone surrogate")
}

fn rsf_test_reverse_char() -> Result<()> {
    Assert::assert_equals("ABEDCF", reverse_chars("ABCDEF", 2, 3).as_str());
    Ok(())
}

fn rsf_test_reverse_supplementary() -> Result<()> {
    // supplementary at end
    Assert::assert_equals(
        "\u{29B05}\u{8271}\u{935F}\u{41F9}\u{612F}\u{701B}",
        reverse("\u{701B}\u{612F}\u{41F9}\u{935F}\u{8271}\u{29B05}").as_str(),
    );
    // supplementary at end - 1
    Assert::assert_equals(
        "a\u{29B05}\u{8271}\u{935F}\u{41F9}\u{612F}\u{701B}",
        reverse("\u{701B}\u{612F}\u{41F9}\u{935F}\u{8271}\u{29B05}a").as_str(),
    );
    // supplementary at start
    Assert::assert_equals("fedcba\u{29B05}", reverse("\u{29B05}abcdef").as_str());
    // supplementary at start + 1
    Assert::assert_equals("fedcba\u{29B05}z", reverse("z\u{29B05}abcdef").as_str());
    // supplementary medial
    Assert::assert_equals("gfe\u{29B05}dcba", reverse("abcd\u{29B05}efg").as_str());
    Ok(())
}

fn rsf_test_reverse_supplementary_char() -> Result<()> {
    // supplementary at end
    Assert::assert_equals(
        "abc\u{29B05}\u{8271}\u{935F}\u{41F9}\u{612F}\u{701B}",
        reverse_chars("abc\u{701B}\u{612F}\u{41F9}\u{935F}\u{8271}\u{29B05}", 3, 7).as_str(),
    );
    // supplementary at end - 1
    Assert::assert_equals(
        "abcd\u{29B05}\u{8271}\u{935F}\u{41F9}\u{612F}\u{701B}",
        reverse_chars(
            "abc\u{701B}\u{612F}\u{41F9}\u{935F}\u{8271}\u{29B05}d",
            3,
            8,
        )
        .as_str(),
    );
    // supplementary at start
    Assert::assert_equals(
        "abc\u{8271}\u{935F}\u{41F9}\u{612F}\u{701B}\u{29B05}",
        reverse_chars("abc\u{29B05}\u{701B}\u{612F}\u{41F9}\u{935F}\u{8271}", 3, 7).as_str(),
    );
    // supplementary at start + 1
    Assert::assert_equals(
        "abc\u{8271}\u{935F}\u{41F9}\u{612F}\u{701B}\u{29B05}d",
        reverse_chars(
            "abcd\u{29B05}\u{701B}\u{612F}\u{41F9}\u{935F}\u{8271}",
            3,
            8,
        )
        .as_str(),
    );
    // supplementary medial
    Assert::assert_equals(
        "abcfed\u{29B05}\u{612F}\u{701B}",
        reverse_chars("abc\u{701B}\u{612F}\u{29B05}def", 3, 7).as_str(),
    );
    Ok(())
}

/// blast some random strings through the analyzer
fn rsf_test_random_strings() -> Result<()> {
    let a = whitespace_analyzer(
        "org.apache.lucene.analysis.reverse.TestReverseStringFilter$1",
        |t| Ok(boxed(new_reverse_string_filter(t)?)),
    );
    B::check_random_data(&*random(), &a, 200 * mult())?;
    a.close();
    Ok(())
}

fn rsf_test_empty_term() -> Result<()> {
    let a = keyword_tokenizer_analyzer(
        "org.apache.lucene.analysis.reverse.TestReverseStringFilter$2",
        |t| Ok(boxed(new_reverse_string_filter(t)?)),
    );
    B::check_one_term(&*a, "", "")?;
    a.close();
    Ok(())
}

fn reverse_string_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.reverse.TestReverseStringFilter")
        .method(t("testFilter", rsf_test_filter))
        .method(t("testFilterWithMark", rsf_test_filter_with_mark))
        .method(t("testReverseString", rsf_test_reverse_string))
        .method(t("testReverseChar", rsf_test_reverse_char))
        .method(t(
            "testReverseSupplementary",
            rsf_test_reverse_supplementary,
        ))
        .method(t(
            "testReverseSupplementaryChar",
            rsf_test_reverse_supplementary_char,
        ))
        .method(t("testRandomStrings", rsf_test_random_strings))
        .method(t("testEmptyTerm", rsf_test_empty_term))
        .build()
}

// -----------------------------------------------------------------------------
// TestTeeSinkTokenFilter
// -----------------------------------------------------------------------------

const TOKENS1: [&str; 10] = [
    "The", "quick", "Burgundy", "Fox", "jumped", "over", "the", "lazy", "Red", "Dogs",
];
const TOKENS2: [&str; 8] = [
    "The", "Lazy", "Dogs", "should", "stay", "on", "the", "porch",
];

/// `addAttribute(CheckClearAttributesAttribute.class)`.
fn add_check_clear(source: &mut AttributeSource) {
    if !source.has_attribute_by_id(std::any::TypeId::of::<dyn CheckClearAttributesAttribute>()) {
        source.add_attribute_impl_instance(Box::new(CheckClearAttributesAttributeImpl::default()));
    }
}

/// `buffer1`/`buffer2` of `setUp`: the tokens, each followed by a space.
fn buffer_of(tokens: &[&str]) -> String {
    tokens.iter().map(|t| format!("{t} ")).collect()
}

// LUCENE-1448
// TODO: instead of testing it this way, we can test
// with BaseTokenStreamTestCase now...
fn tstf_test_end_offset_position_with_tee_sink_token_filter() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let analyzer: Arc<dyn Analyzer> = Arc::new(MockAnalyzer::with_automaton(
        &*random(),
        Arc::clone(&WHITESPACE),
        false,
    ));
    let w = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::clone(&analyzer))),
    )?;
    let mut doc = Document::new();
    let token_stream = analyzer.token_stream("field", string_reader("abcd   "))?;
    let tee = TeeSinkTokenFilter::new(Box::new(rucene::analysis::SharedTokenStream::new(
        token_stream,
    )));
    let sink = tee.new_sink_token_stream();
    let ft = FieldType::new_from(TextField::type_not_stored());
    ft.set_store_term_vectors(true)?;
    ft.set_store_term_vector_offsets(true)?;
    ft.set_store_term_vector_positions(true)?;
    let ft = Arc::new(ft);
    let tee: Arc<AtomicRefCell<dyn TokenStream>> = Arc::new(AtomicRefCell::new(tee));
    let sink: Arc<AtomicRefCell<dyn TokenStream>> = Arc::new(AtomicRefCell::new(sink));
    let f1 = Field::new_with_token_stream("field", tee, Arc::clone(&ft))?;
    let f2 = Field::new_with_token_stream("field", sink, Arc::clone(&ft))?;
    doc.add(Box::new(f1));
    doc.add(Box::new(f2));
    w.add_document(&doc)?;
    w.close()?;

    let r: Arc<dyn IndexReader> = directory_reader::open(dir.as_directory())?;
    let vector = r
        .term_vectors()?
        .get(0)?
        .expect("term vectors")
        .terms("field")?
        .expect("the field's vector");
    Assert::assert_equals(1, vector.size()?);
    let mut terms_enum = vector.iterator()?;
    terms_enum.next()?;
    Assert::assert_equals(2, terms_enum.total_term_freq()?);
    let mut positions = terms_enum
        .postings(None, POSTINGS_ENUM_ALL)?
        .expect("postings");
    Assert::assert_true(positions.next_doc()? != NO_MORE_DOCS);
    Assert::assert_equals(2, positions.freq()?);
    positions.next_position()?;
    Assert::assert_equals(0, positions.start_offset());
    Assert::assert_equals(4, positions.end_offset());
    positions.next_position()?;
    Assert::assert_equals(8, positions.start_offset());
    Assert::assert_equals(12, positions.end_offset());
    Assert::assert_equals(NO_MORE_DOCS, positions.next_doc()?);
    r.close()?;
    dir.close()?;
    analyzer.close();
    Ok(())
}

fn tstf_test_general() -> Result<()> {
    let mut source = TeeSinkTokenFilter::new(boxed(B::whitespace_mock_tokenizer_str(&buffer_of(
        &TOKENS1,
    ))?));
    let mut sink = source.new_sink_token_stream();
    add_check_clear(source.attribute_source_mut());
    add_check_clear(sink.attribute_source_mut());
    B::assert_token_stream_contents(&mut source, &TOKENS1)?;
    B::assert_token_stream_contents(&mut sink, &TOKENS1)
}

fn tstf_test_multiple_sources() -> Result<()> {
    let tee1 = own(TeeSinkTokenFilter::new(boxed(
        B::whitespace_mock_tokenizer_str(&buffer_of(&TOKENS1))?,
    )));
    let source1 = own(CachingTokenFilter::new(
        rucene_test_framework::analysis::shared(&tee1),
    ));
    add_check_clear(tee1.borrow_mut().attribute_source_mut());
    let factory = tee1.borrow().attribute_source().attribute_factory();
    let mut tokenizer =
        MockTokenizer::with_factory_default_length(factory, Arc::clone(&WHITESPACE), false)?;
    tokenizer.set_reader(string_reader(&buffer_of(&TOKENS2)))?;
    let mut source2 = TeeSinkTokenFilter::new(boxed(tokenizer));
    B::assert_token_stream_contents(&mut *source1.borrow_mut(), &TOKENS1)?;
    B::assert_token_stream_contents(&mut source2, &TOKENS2)?;
    let mut lower_casing = LowerCaseFilter::new(rucene_test_framework::analysis::shared(&source1))?;
    let lower_case_tokens: Vec<String> = TOKENS1.iter().map(|t| t.to_lowercase()).collect();
    let lower_case_tokens: Vec<&str> = lower_case_tokens.iter().map(String::as_str).collect();
    B::assert_token_stream_contents(&mut lower_casing, &lower_case_tokens)
}

fn tee_sink_token_filter_class() -> Arc<Class> {
    btstc_class("org.apache.lucene.analysis.sinks.TestTeeSinkTokenFilter")
        .method(t(
            "testEndOffsetPositionWithTeeSinkTokenFilter",
            tstf_test_end_offset_position_with_tee_sink_token_filter,
        ))
        .method(t("testGeneral", tstf_test_general))
        .method(t("testMultipleSources", tstf_test_multiple_sources))
        .build()
}

// -----------------------------------------------------------------------------
// TestReverseStringFilterFactory
// -----------------------------------------------------------------------------

fn rsff_test_reversing() -> Result<()> {
    let reader = string_reader("simple test");
    let mut stream: Box<dyn TokenStream> = boxed(B::whitespace_mock_tokenizer(reader)?);
    stream = fac("ReverseString", &[])?.create(stream)?;
    B::assert_token_stream_contents(&mut *stream, &["elpmis", "tset"])?;
    Ok(())
}

fn rsff_test_bogus_arguments() -> Result<()> {
    let expected = iae_message(|| fac("ReverseString", &["bogusArg", "bogusValue"]));
    Assert::assert_true(expected.contains("Unknown parameters"));
    Ok(())
}
fn reverse_string_filter_factory_class() -> Arc<Class> {
    factory_class("org.apache.lucene.analysis.reverse.TestReverseStringFilterFactory")
        .method(t("testReversing", rsff_test_reversing))
        .method(t("testBogusArguments", rsff_test_bogus_arguments))
        .build()
}

#[test]
fn test_delimited_boost_token_filter() {
    run(delimited_boost_token_filter_class);
}

#[test]
fn test_delimited_payload_token_filter() {
    run(delimited_payload_token_filter_class);
}

#[test]
fn test_delimited_payload_token_filter_factory() {
    run(delimited_payload_token_filter_factory_class);
}

#[test]
fn test_ngram_filters() {
    run(ngram_filters_class);
}

#[test]
fn test_numeric_payload_token_filter() {
    run(numeric_payload_token_filter_class);
}

#[test]
fn test_path_hierarchy_tokenizer() {
    run(path_hierarchy_tokenizer_class);
}

#[test]
fn test_query_auto_stop_word_analyzer() {
    run(query_auto_stop_word_analyzer_class);
}

#[test]
fn test_reverse_path_hierarchy_tokenizer() {
    run(reverse_path_hierarchy_tokenizer_class);
}

#[test]
fn test_reverse_string_filter() {
    run(reverse_string_filter_class);
}

#[test]
fn test_reverse_string_filter_factory() {
    run(reverse_string_filter_factory_class);
}

#[test]
fn test_tee_sink_token_filter() {
    run(tee_sink_token_filter_class);
}

#[test]
fn test_token_offset_payload_token_filter() {
    run(token_offset_payload_token_filter_class);
}

#[test]
fn test_type_as_payload_token_filter() {
    run(type_as_payload_token_filter_class);
}
