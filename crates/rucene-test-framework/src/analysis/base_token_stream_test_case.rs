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

//! Port of `org.apache.lucene.tests.analysis.BaseTokenStreamTestCase`.

use std::any::{Any, TypeId};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use randomizedtesting::jdk::exception::{Throwable, ASSERTION_ERROR};
use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::analysis::automaton_conversion::{TokenStreamToAutomaton, POS_SEP};
use rucene::analysis::{
    default_attribute_factory, default_token_attribute_factory, Analyzer, CharReader,
    StringCharReader, TokenStream,
};
use rucene::document::{Document, Field, FieldType, SharedReader, TextField};
use rucene::error::{LuceneError, Result};
use rucene::index::{IndexOptions, IndexableFieldType};
use rucene::util::attribute::{Attribute, AttributeFactory, AttributeImpl, AttributeReflector};
use rucene::util::fst::Util;
use rucene::util::{BytesRef, BytesRefBuilder, IntsRef};

use super::attributes::{
    boost_id, char_term_id, flags_id, keyword_id, offset_id, payload_id, pos_inc_id, pos_len_id,
    term_to_bytes_ref_id, type_id, Att,
};
use super::mock_char_filter::MockCharFilter;
use super::mock_reader_wrapper::MockReaderWrapper;
use super::mock_tokenizer::{MockTokenizer, KEYWORD, WHITESPACE};
use super::token::Token;
use super::token_stream_to_dot::TokenStreamToDot;
use crate::index::RandomIndexWriter;
use crate::util::automaton::AutomatonTestUtil;
use crate::util::lucene_test_case::{TestDirectory, VERBOSE};
use crate::util::{LuceneTestCase, TestUtil};

/// Attribute that records if it was cleared or not. This is used for testing
/// that clearAttributes() was called correctly.
///
/// Equivalent to `BaseTokenStreamTestCase.CheckClearAttributesAttribute`.
pub trait CheckClearAttributesAttribute: Attribute {
    /// `getAndResetClearCalled()`.
    fn get_and_reset_clear_called(&mut self) -> bool;
}

/// Attribute that records if it was cleared or not. This is used for testing
/// that clearAttributes() was called correctly.
///
/// Equivalent to `BaseTokenStreamTestCase.CheckClearAttributesAttributeImpl`.
#[derive(Clone, Debug, Default)]
pub struct CheckClearAttributesAttributeImpl {
    clear_called: bool,
}

impl Attribute for CheckClearAttributesAttributeImpl {}

impl CheckClearAttributesAttribute for CheckClearAttributesAttributeImpl {
    fn get_and_reset_clear_called(&mut self) -> bool {
        std::mem::replace(&mut self.clear_called, false)
    }
}

impl AttributeImpl for CheckClearAttributesAttributeImpl {
    fn clear(&mut self) {
        self.clear_called = true;
    }

    fn copy_to(&self, target: &mut dyn AttributeImpl) {
        target.clear();
    }

    fn reflect_with(&self, reflector: &mut dyn AttributeReflector) {
        reflector.reflect(
            TypeId::of::<dyn CheckClearAttributesAttribute>(),
            std::any::type_name::<CheckClearAttributesAttributeImpl>(),
            "clearCalled",
            &self.clear_called,
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
                TypeId::of::<CheckClearAttributesAttributeImpl>(),
                TypeId::of::<dyn CheckClearAttributesAttribute>(),
            ]
        })
    }

    fn equals(&self, other: &dyn AttributeImpl) -> bool {
        other
            .as_any()
            .downcast_ref::<CheckClearAttributesAttributeImpl>()
            .is_some_and(|o| o.clear_called == self.clear_called)
    }

    fn hash_code(&self) -> i32 {
        76137213 ^ if self.clear_called { 1231 } else { 1237 }
    }
}

/// `Throwable.toString()` of an error, as Java's `"" + exception` prints it.
pub fn java_to_string(e: &LuceneError) -> String {
    match e {
        LuceneError::IllegalState { message, .. } => {
            format!("java.lang.IllegalStateException: {message}")
        }
        LuceneError::IllegalArgument(message) => {
            format!("java.lang.IllegalArgumentException: {message}")
        }
        LuceneError::Io(io) => rucene::store::java_nio_file::java_to_string(io),
        LuceneError::Unchecked { .. } => e.to_string(),
        other => format!("{}: {}", other.java_simple_name(), other),
    }
}

/// The binary name of the Java class an error stands for, and its message.
pub fn java_class_and_message(e: &LuceneError) -> (String, String) {
    match e {
        LuceneError::IllegalState { message, .. } => (
            "java.lang.IllegalStateException".to_string(),
            message.clone(),
        ),
        LuceneError::IllegalArgument(message) => (
            "java.lang.IllegalArgumentException".to_string(),
            message.clone(),
        ),
        LuceneError::Io(io) => (
            rucene::store::java_nio_file::java_class_of(io).to_string(),
            rucene::store::java_nio_file::java_message_of(io)
                .unwrap_or_else(|| "<null>".to_string()),
        ),
        LuceneError::Unchecked { class, message, .. } => (
            class.to_string(),
            message.clone().unwrap_or_else(|| "<null>".to_string()),
        ),
        other => (other.java_simple_name().to_string(), other.to_string()),
    }
}

/// Whether Java's `catch (RuntimeException e)` catches the error.
fn is_runtime_exception(e: &LuceneError) -> bool {
    !e.is_io_exception()
}

/// Whether Java's `catch (IllegalStateException e)` catches the error.
fn is_illegal_state(e: &LuceneError) -> bool {
    matches!(e, LuceneError::IllegalState { .. })
}

/// Rethrows an error inside a thread, as `Rethrow.rethrow(e)` does: the
/// panic payload is the Java throwable.
fn rethrow(e: &LuceneError) -> ! {
    let (class, message) = java_class_and_message(e);
    Throwable::new(class, message).throw()
}

/// Base class for all Lucene unit tests that use TokenStreams.
///
/// When writing unit tests for analysis components, it's highly recommended
/// to use the helper methods here (especially in conjunction with
/// [`MockAnalyzer`](super::MockAnalyzer) or [`MockTokenizer`]), as they
/// contain many assertions and checks to catch bugs.
///
/// Equivalent to `org.apache.lucene.tests.analysis.BaseTokenStreamTestCase`.
/// Java's overloads are named after their parameters here; the nullable
/// arrays and boxed integers are `Option`s. Assertion failures panic with
/// JUnit's `AssertionError`/`ComparisonFailure` and Java's message, and the
/// errors of the stream are returned.
pub struct BaseTokenStreamTestCase;

impl BaseTokenStreamTestCase {
    /// The declaration of `org.apache.lucene.tests.analysis.BaseTokenStreamTestCase`
    /// (`public abstract class BaseTokenStreamTestCase extends LuceneTestCase`,
    /// which adds no rule, hook or annotation) for suites that extend it.
    pub fn class() -> Arc<randomizedtesting::jdk::reflect::Class> {
        use randomizedtesting::jdk::reflect::{modifier, Class};
        static CLASS: std::sync::LazyLock<Arc<Class>> = std::sync::LazyLock::new(|| {
            Class::builder("org.apache.lucene.tests.analysis.BaseTokenStreamTestCase")
                .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
                .extends(&LuceneTestCase::class())
                .build()
        });
        Arc::clone(&CLASS)
    }
}

#[allow(clippy::too_many_arguments)]
impl BaseTokenStreamTestCase {
    // graphOffsetsAreCorrect validates:
    //   - graph offsets are correct (all tokens leaving from
    //     pos X have the same startOffset; all tokens
    //     arriving to pos Y have the same endOffset)
    /// The full `assertTokenStreamContents`, every other overload's body.
    ///
    /// # Errors
    ///
    /// What the stream throws.
    ///
    /// # Panics
    ///
    /// The `AssertionError` (or `ComparisonFailure`) of the first unmet
    /// expectation, with Java's message.
    pub fn assert_token_stream_contents_full(
        ts: &mut dyn TokenStream,
        output: &[&str],
        start_offsets: Option<&[i32]>,
        end_offsets: Option<&[i32]>,
        types: Option<&[&str]>,
        pos_increments: Option<&[i32]>,
        pos_lengths: Option<&[i32]>,
        final_offset: Option<i32>,
        final_pos_inc: Option<i32>,
        keyword_atts: Option<&[bool]>,
        graph_offsets_are_correct: bool,
        payloads: Option<&[Option<&[u8]>]>,
        flags: Option<&[i32]>,
        boost: Option<&[f32]>,
    ) -> Result<()> {
        let check_clear_att = {
            let source = ts.attribute_source_mut();
            if !source.has_attribute_by_id(TypeId::of::<dyn CheckClearAttributesAttribute>()) {
                source.add_attribute_impl_instance(Box::new(
                    CheckClearAttributesAttributeImpl::default(),
                ));
            }
            Att::get(source, TypeId::of::<dyn CheckClearAttributesAttribute>())
                .expect("INVARIANT: added just above")
        };

        let source = ts.attribute_source();
        let mut term_att: Option<Att> = None;
        if !output.is_empty() {
            Assert::assert_true_msg(
                "has no CharTermAttribute",
                source.has_attribute_by_id(char_term_id()),
            );
            let t = Att::get(source, char_term_id()).unwrap();
            // every UTF-16 character-based TokenStream MUST provide a TermToBytesRefAttribute,
            // implemented by same instance like the CharTermAttribute:
            Assert::assert_true_msg(
                "has no TermToBytesRefAttribute",
                source.has_attribute_by_id(term_to_bytes_ref_id()),
            );
            let bytes_att = Att::get(source, term_to_bytes_ref_id()).unwrap();

            // ConcatenateGraphFilter has some tricky logic violating this. We have an extra assert there:
            let simple_name = format!("{:?}", bytes_att.implementation());
            if !simple_name.starts_with("BytesRefBuilderTermAttributeImpl") && !t.same(&bytes_att) {
                Assert::fail(&format!(
                    "TermToBytesRefAttribute must be implemented by same instance expected same:<{:?}> was not:<{:?}>",
                    t.implementation(),
                    bytes_att.implementation()
                ));
            }
            term_att = Some(t);
        }

        let mut offset_att: Option<Att> = None;
        if start_offsets.is_some() || end_offsets.is_some() || final_offset.is_some() {
            Assert::assert_true_msg(
                "has no OffsetAttribute",
                source.has_attribute_by_id(offset_id()),
            );
            offset_att = Att::get(source, offset_id());
        }

        let mut type_att: Option<Att> = None;
        if types.is_some() {
            Assert::assert_true_msg(
                "has no TypeAttribute",
                source.has_attribute_by_id(type_id()),
            );
            type_att = Att::get(source, type_id());
        }

        let mut pos_incr_att: Option<Att> = None;
        if pos_increments.is_some() || final_pos_inc.is_some() {
            Assert::assert_true_msg(
                "has no PositionIncrementAttribute",
                source.has_attribute_by_id(pos_inc_id()),
            );
            pos_incr_att = Att::get(source, pos_inc_id());
        }

        let mut pos_length_att: Option<Att> = None;
        if pos_lengths.is_some() {
            Assert::assert_true_msg(
                "has no PositionLengthAttribute",
                source.has_attribute_by_id(pos_len_id()),
            );
            pos_length_att = Att::get(source, pos_len_id());
        }

        let mut keyword_att: Option<Att> = None;
        if keyword_atts.is_some() {
            Assert::assert_true_msg(
                "has no KeywordAttribute",
                source.has_attribute_by_id(keyword_id()),
            );
            keyword_att = Att::get(source, keyword_id());
        }

        let mut payload_att: Option<Att> = None;
        if payloads.is_some() {
            Assert::assert_true_msg(
                "has no PayloadAttribute",
                source.has_attribute_by_id(payload_id()),
            );
            payload_att = Att::get(source, payload_id());
        }

        let mut flags_att: Option<Att> = None;
        if flags.is_some() {
            Assert::assert_true_msg(
                "has no FlagsAttribute",
                source.has_attribute_by_id(flags_id()),
            );
            flags_att = Att::get(source, flags_id());
        }

        let mut boost_att: Option<Att> = None;
        if boost.is_some() {
            Assert::assert_true_msg(
                "has no BoostAttribute",
                source.has_attribute_by_id(boost_id()),
            );
            boost_att = Att::get(source, boost_id());
        }

        // Maps position to the start/end offset:
        let mut pos_to_start_offset: HashMap<i32, i32> = HashMap::new();
        let mut pos_to_end_offset: HashMap<i32, i32> = HashMap::new();

        // TODO: would be nice to be able to assert silly duplicated tokens are not created, but a
        // number of cases do this "legitimately": LUCENE-7622

        let term = |t: &Option<Att>| t.as_ref().map(Att::term_string).unwrap_or_default();
        let bogus_payload = || BytesRef::new(vec![0x00, 0xdf, 0x12, 0xbd, 0x24]);

        ts.reset()?;
        let mut pos = -1;
        let mut last_start_offset = 0;
        for (i, expected_term) in output.iter().enumerate() {
            // extra safety to enforce, that the state is not preserved and also assign bogus values
            ts.attribute_source().clear_attributes();
            if let Some(t) = &term_att {
                t.set_term("bogusTerm");
            }
            if let Some(a) = &offset_att {
                a.offset_mut().set_offset(14584724, 24683243);
            }
            if let Some(a) = &type_att {
                a.type_att_mut().set_type("bogusType".to_string());
            }
            if let Some(a) = &pos_incr_att {
                a.pos_inc_mut().set_position_increment(45987657);
            }
            if let Some(a) = &pos_length_att {
                a.pos_len_mut().set_position_length(45987653);
            }
            if let Some(a) = &keyword_att {
                a.keyword_mut().set_keyword((i & 1) == 0);
            }
            if let Some(a) = &payload_att {
                a.payload_mut().set_payload(Some(bogus_payload()));
            }
            if let Some(a) = &flags_att {
                a.flags_mut().set_flags(!0); // all 1's
            }
            if let Some(a) = &boost_att {
                a.boost_mut().set_boost(-1.0);
            }

            check_clear_att
                .check_clear_mut()
                .get_and_reset_clear_called(); // reset it, because we called clearAttribute() before
            Assert::assert_true_msg(
                &format!("token {} does not exist", i),
                ts.increment_token()?,
            );
            Assert::assert_true_msg(
                &format!(
                    "clearAttributes() was not called correctly in TokenStream chain at token {}",
                    i
                ),
                check_clear_att
                    .check_clear_mut()
                    .get_and_reset_clear_called(),
            );

            let actual_term = term(&term_att);
            Assert::assert_equals_str(
                Some(&format!("term {}", i)),
                Some(expected_term),
                Some(&actual_term),
            );
            if let Some(start_offsets) = start_offsets {
                let actual = offset_att.as_ref().unwrap().offset().start_offset();
                Assert::assert_equals_msg(
                    Some(&format!("startOffset {} term={}", i, actual_term)),
                    i64::from(start_offsets[i]),
                    i64::from(actual),
                );
            }
            if let Some(end_offsets) = end_offsets {
                let actual = offset_att.as_ref().unwrap().offset().end_offset();
                Assert::assert_equals_msg(
                    Some(&format!("endOffset {} term={}", i, actual_term)),
                    i64::from(end_offsets[i]),
                    i64::from(actual),
                );
            }
            if let Some(types) = types {
                let actual = type_att
                    .as_ref()
                    .unwrap()
                    .type_att()
                    .type_value()
                    .to_string();
                Assert::assert_equals_str(
                    Some(&format!("type {} term={}", i, actual_term)),
                    Some(types[i]),
                    Some(&actual),
                );
            }
            if let Some(pos_increments) = pos_increments {
                let actual = pos_incr_att
                    .as_ref()
                    .unwrap()
                    .pos_inc()
                    .get_position_increment();
                Assert::assert_equals_msg(
                    Some(&format!("posIncrement {} term={}", i, actual_term)),
                    i64::from(pos_increments[i]),
                    i64::from(actual),
                );
            }
            if let Some(pos_lengths) = pos_lengths {
                let actual = pos_length_att
                    .as_ref()
                    .unwrap()
                    .pos_len()
                    .get_position_length();
                Assert::assert_equals_msg(
                    Some(&format!("posLength {} term={}", i, actual_term)),
                    i64::from(pos_lengths[i]),
                    i64::from(actual),
                );
            }
            if let Some(keyword_atts) = keyword_atts {
                let actual = keyword_att.as_ref().unwrap().keyword().is_keyword();
                Assert::assert_equals_msg(
                    Some(&format!("keywordAtt {} term={}", i, actual_term)),
                    keyword_atts[i],
                    actual,
                );
            }
            if let Some(a) = &flags_att {
                let actual = a.flags().get_flags();
                Assert::assert_equals_msg(
                    Some(&format!("flagsAtt {} term={}", i, actual_term)),
                    i64::from(flags.unwrap()[i]),
                    i64::from(actual),
                );
            }
            if let Some(a) = &boost_att {
                let actual = a.boost().get_boost();
                Assert::assert_equals_float_msg(
                    Some(&format!("boostAtt {} term={}", i, actual_term)),
                    boost.unwrap()[i],
                    actual,
                    0.001,
                );
            }
            if let Some(payloads) = payloads {
                if let Some(expected) = payloads[i] {
                    let expected = BytesRef::new(expected.to_vec());
                    let actual = payload_att
                        .as_ref()
                        .unwrap()
                        .payload()
                        .get_payload()
                        .cloned();
                    if actual.as_ref() != Some(&expected) {
                        Assert::fail(&format!(
                            "payloads {} expected:<{}> but was:<{}>",
                            i,
                            expected,
                            actual.map_or_else(|| "null".to_string(), |a| a.to_string())
                        ));
                    }
                } else {
                    // assertNull("payloads " + i, payloads[i]): always true
                }
            }
            if let Some(a) = &pos_incr_att {
                let inc = a.pos_inc().get_position_increment();
                if i == 0 {
                    Assert::assert_true_msg("first posIncrement must be >= 1", inc >= 1);
                } else {
                    Assert::assert_true_msg("posIncrement must be >= 0", inc >= 0);
                }
            }
            if let Some(a) = &pos_length_att {
                let len = a.pos_len().get_position_length();
                Assert::assert_true_msg(&format!("posLength must be >= 1; got: {}", len), len >= 1);
            }
            // we can enforce some basic things about a few attributes even if the caller doesn't check:
            if let Some(a) = &offset_att {
                let (start_offset, end_offset) = {
                    let o = a.offset();
                    (o.start_offset(), o.end_offset())
                };
                if let Some(final_offset) = final_offset {
                    Assert::assert_true_msg(
                        &format!(
                            "startOffset (= {}) must be <= finalOffset (= {}) term={}",
                            start_offset, final_offset, actual_term
                        ),
                        start_offset <= final_offset,
                    );
                    Assert::assert_true_msg(
                        &format!(
                            "endOffset must be <= finalOffset: got endOffset={} vs finalOffset={} term={}",
                            end_offset, final_offset, actual_term
                        ),
                        end_offset <= final_offset,
                    );
                }

                Assert::assert_true_msg(
                    &format!(
                        "offsets must not go backwards startOffset={} is < lastStartOffset={} term={}",
                        start_offset, last_start_offset, actual_term
                    ),
                    start_offset >= last_start_offset,
                );
                last_start_offset = start_offset;

                if graph_offsets_are_correct {
                    if let (Some(pl), Some(pi)) = (&pos_length_att, &pos_incr_att) {
                        // Validate offset consistency in the graph, ie
                        // all tokens leaving from a certain pos have the
                        // same startOffset, and all tokens arriving to a
                        // certain pos have the same endOffset:
                        let pos_inc = pi.pos_inc().get_position_increment();
                        pos += pos_inc;

                        let pos_length = pl.pos_len().get_position_length();

                        match pos_to_start_offset.get(&pos) {
                            None => {
                                // First time we've seen a token leaving from this position:
                                pos_to_start_offset.insert(pos, start_offset);
                            }
                            Some(&expected) => {
                                // We've seen a token leaving from this position
                                // before; verify the startOffset is the same:
                                Assert::assert_equals_msg(
                                    Some(&format!(
                                        "{} inconsistent startOffset: pos={} posLen={} token={}",
                                        i, pos, pos_length, actual_term
                                    )),
                                    i64::from(expected),
                                    i64::from(start_offset),
                                );
                            }
                        }

                        let end_pos = pos + pos_length;

                        match pos_to_end_offset.get(&end_pos) {
                            None => {
                                // First time we've seen a token arriving to this position:
                                pos_to_end_offset.insert(end_pos, end_offset);
                            }
                            Some(&expected) => {
                                // We've seen a token arriving to this position
                                // before; verify the endOffset is the same:
                                Assert::assert_equals_msg(
                                    Some(&format!(
                                        "inconsistent endOffset {} pos={} posLen={} token={}",
                                        i, pos, pos_length, actual_term
                                    )),
                                    i64::from(expected),
                                    i64::from(end_offset),
                                );
                            }
                        }
                    }
                }
            }
        }

        if ts.increment_token()? {
            let extra = Att::get(ts.attribute_source(), char_term_id())
                .map(|t| t.term_string())
                .unwrap_or_else(|| "null".to_string());
            Assert::fail(&format!(
                "TokenStream has more tokens than expected (expected count={}); extra token={}",
                output.len(),
                extra
            ));
        }

        // repeat our extra safety checks for end()
        ts.attribute_source().clear_attributes();
        if let Some(t) = &term_att {
            t.set_term("bogusTerm");
        }
        if let Some(a) = &offset_att {
            a.offset_mut().set_offset(14584724, 24683243);
        }
        if let Some(a) = &type_att {
            a.type_att_mut().set_type("bogusType".to_string());
        }
        if let Some(a) = &pos_incr_att {
            a.pos_inc_mut().set_position_increment(45987657);
        }
        if let Some(a) = &pos_length_att {
            a.pos_len_mut().set_position_length(45987653);
        }
        if let Some(a) = &keyword_att {
            a.keyword_mut().set_keyword(true);
        }
        if let Some(a) = &payload_att {
            a.payload_mut().set_payload(Some(bogus_payload()));
        }
        if let Some(a) = &flags_att {
            a.flags_mut().set_flags(!0); // all 1's
        }
        if let Some(a) = &boost_att {
            a.boost_mut().set_boost(-1.0);
        }

        check_clear_att
            .check_clear_mut()
            .get_and_reset_clear_called(); // reset it, because we called clearAttribute() before

        ts.end()?;
        Assert::assert_true_msg(
            "super.end()/clearAttributes() was not called correctly in end()",
            check_clear_att
                .check_clear_mut()
                .get_and_reset_clear_called(),
        );

        if let Some(final_offset) = final_offset {
            let actual = offset_att.as_ref().unwrap().offset().end_offset();
            Assert::assert_equals_msg(
                Some("finalOffset"),
                i64::from(final_offset),
                i64::from(actual),
            );
        }

        if let Some(a) = &offset_att {
            Assert::assert_true_msg("finalOffset must be >= 0", a.offset().end_offset() >= 0);
        }

        if let Some(final_pos_inc) = final_pos_inc {
            let actual = pos_incr_att
                .as_ref()
                .unwrap()
                .pos_inc()
                .get_position_increment();
            Assert::assert_equals_msg(
                Some("finalPosInc"),
                i64::from(final_pos_inc),
                i64::from(actual),
            );
        }

        ts.close()
    }

    /// `assertTokenStreamContents(ts, output, startOffsets, endOffsets,
    /// types, posIncrements, posLengths, finalOffset, finalPosInc,
    /// keywordAtts, graphOffsetsAreCorrect, payloads, flags)`.
    ///
    /// # Errors
    ///
    /// As [`assert_token_stream_contents_full`](Self::assert_token_stream_contents_full).
    pub fn assert_token_stream_contents_flags(
        ts: &mut dyn TokenStream,
        output: &[&str],
        start_offsets: Option<&[i32]>,
        end_offsets: Option<&[i32]>,
        types: Option<&[&str]>,
        pos_increments: Option<&[i32]>,
        pos_lengths: Option<&[i32]>,
        final_offset: Option<i32>,
        final_pos_inc: Option<i32>,
        keyword_atts: Option<&[bool]>,
        graph_offsets_are_correct: bool,
        payloads: Option<&[Option<&[u8]>]>,
        flags: Option<&[i32]>,
    ) -> Result<()> {
        Self::assert_token_stream_contents_full(
            ts,
            output,
            start_offsets,
            end_offsets,
            types,
            pos_increments,
            pos_lengths,
            final_offset,
            final_pos_inc,
            keyword_atts,
            graph_offsets_are_correct,
            payloads,
            flags,
            None,
        )
    }

    /// `assertTokenStreamContents(ts, output, startOffsets, endOffsets,
    /// types, posIncrements, posLengths, finalOffset, keywordAtts,
    /// graphOffsetsAreCorrect)`.
    ///
    /// # Errors
    ///
    /// As [`assert_token_stream_contents_full`](Self::assert_token_stream_contents_full).
    pub fn assert_token_stream_contents_keywords(
        ts: &mut dyn TokenStream,
        output: &[&str],
        start_offsets: Option<&[i32]>,
        end_offsets: Option<&[i32]>,
        types: Option<&[&str]>,
        pos_increments: Option<&[i32]>,
        pos_lengths: Option<&[i32]>,
        final_offset: Option<i32>,
        keyword_atts: Option<&[bool]>,
        graph_offsets_are_correct: bool,
    ) -> Result<()> {
        Self::assert_token_stream_contents_keywords_boost(
            ts,
            output,
            start_offsets,
            end_offsets,
            types,
            pos_increments,
            pos_lengths,
            final_offset,
            keyword_atts,
            graph_offsets_are_correct,
            None,
        )
    }

    /// `assertTokenStreamContents(ts, output, startOffsets, endOffsets,
    /// types, posIncrements, posLengths, finalOffset, keywordAtts,
    /// graphOffsetsAreCorrect, boost)`.
    ///
    /// # Errors
    ///
    /// As [`assert_token_stream_contents_full`](Self::assert_token_stream_contents_full).
    pub fn assert_token_stream_contents_keywords_boost(
        ts: &mut dyn TokenStream,
        output: &[&str],
        start_offsets: Option<&[i32]>,
        end_offsets: Option<&[i32]>,
        types: Option<&[&str]>,
        pos_increments: Option<&[i32]>,
        pos_lengths: Option<&[i32]>,
        final_offset: Option<i32>,
        keyword_atts: Option<&[bool]>,
        graph_offsets_are_correct: bool,
        boost: Option<&[f32]>,
    ) -> Result<()> {
        Self::assert_token_stream_contents_full(
            ts,
            output,
            start_offsets,
            end_offsets,
            types,
            pos_increments,
            pos_lengths,
            final_offset,
            None,
            keyword_atts,
            graph_offsets_are_correct,
            None,
            None,
            boost,
        )
    }

    /// `assertTokenStreamContents(ts, output, startOffsets, endOffsets,
    /// types, posIncrements, posLengths, finalOffset, finalPosInc,
    /// keywordAtts, graphOffsetsAreCorrect, payloads)`.
    ///
    /// # Errors
    ///
    /// As [`assert_token_stream_contents_full`](Self::assert_token_stream_contents_full).
    pub fn assert_token_stream_contents_payloads(
        ts: &mut dyn TokenStream,
        output: &[&str],
        start_offsets: Option<&[i32]>,
        end_offsets: Option<&[i32]>,
        types: Option<&[&str]>,
        pos_increments: Option<&[i32]>,
        pos_lengths: Option<&[i32]>,
        final_offset: Option<i32>,
        final_pos_inc: Option<i32>,
        keyword_atts: Option<&[bool]>,
        graph_offsets_are_correct: bool,
        payloads: Option<&[Option<&[u8]>]>,
    ) -> Result<()> {
        Self::assert_token_stream_contents_full(
            ts,
            output,
            start_offsets,
            end_offsets,
            types,
            pos_increments,
            pos_lengths,
            final_offset,
            final_pos_inc,
            keyword_atts,
            graph_offsets_are_correct,
            payloads,
            None,
            None,
        )
    }

    /// `assertTokenStreamContents(ts, output, startOffsets, endOffsets,
    /// types, posIncrements, posLengths, finalOffset,
    /// graphOffsetsAreCorrect, boost)`.
    ///
    /// # Errors
    ///
    /// As [`assert_token_stream_contents_full`](Self::assert_token_stream_contents_full).
    pub fn assert_token_stream_contents_graph_boost(
        ts: &mut dyn TokenStream,
        output: &[&str],
        start_offsets: Option<&[i32]>,
        end_offsets: Option<&[i32]>,
        types: Option<&[&str]>,
        pos_increments: Option<&[i32]>,
        pos_lengths: Option<&[i32]>,
        final_offset: Option<i32>,
        graph_offsets_are_correct: bool,
        boost: Option<&[f32]>,
    ) -> Result<()> {
        Self::assert_token_stream_contents_keywords_boost(
            ts,
            output,
            start_offsets,
            end_offsets,
            types,
            pos_increments,
            pos_lengths,
            final_offset,
            None,
            graph_offsets_are_correct,
            boost,
        )
    }

    /// `assertTokenStreamContents(ts, output, startOffsets, endOffsets,
    /// types, posIncrements, posLengths, finalOffset,
    /// graphOffsetsAreCorrect)`.
    ///
    /// # Errors
    ///
    /// As [`assert_token_stream_contents_full`](Self::assert_token_stream_contents_full).
    pub fn assert_token_stream_contents_graph(
        ts: &mut dyn TokenStream,
        output: &[&str],
        start_offsets: Option<&[i32]>,
        end_offsets: Option<&[i32]>,
        types: Option<&[&str]>,
        pos_increments: Option<&[i32]>,
        pos_lengths: Option<&[i32]>,
        final_offset: Option<i32>,
        graph_offsets_are_correct: bool,
    ) -> Result<()> {
        Self::assert_token_stream_contents_keywords_boost(
            ts,
            output,
            start_offsets,
            end_offsets,
            types,
            pos_increments,
            pos_lengths,
            final_offset,
            None,
            graph_offsets_are_correct,
            None,
        )
    }

    /// `assertTokenStreamContents(ts, output, startOffsets, endOffsets,
    /// types, posIncrements, posLengths, finalOffset)`: graph offsets
    /// checked.
    ///
    /// # Errors
    ///
    /// As [`assert_token_stream_contents_full`](Self::assert_token_stream_contents_full).
    pub fn assert_token_stream_contents_final_offset(
        ts: &mut dyn TokenStream,
        output: &[&str],
        start_offsets: Option<&[i32]>,
        end_offsets: Option<&[i32]>,
        types: Option<&[&str]>,
        pos_increments: Option<&[i32]>,
        pos_lengths: Option<&[i32]>,
        final_offset: Option<i32>,
    ) -> Result<()> {
        Self::assert_token_stream_contents_graph(
            ts,
            output,
            start_offsets,
            end_offsets,
            types,
            pos_increments,
            pos_lengths,
            final_offset,
            true,
        )
    }

    /// `assertTokenStreamContents(ts, output, startOffsets, endOffsets,
    /// types, posIncrements, posLengths, finalOffset, boost)`.
    ///
    /// # Errors
    ///
    /// As [`assert_token_stream_contents_full`](Self::assert_token_stream_contents_full).
    pub fn assert_token_stream_contents_final_offset_boost(
        ts: &mut dyn TokenStream,
        output: &[&str],
        start_offsets: Option<&[i32]>,
        end_offsets: Option<&[i32]>,
        types: Option<&[&str]>,
        pos_increments: Option<&[i32]>,
        pos_lengths: Option<&[i32]>,
        final_offset: Option<i32>,
        boost: Option<&[f32]>,
    ) -> Result<()> {
        Self::assert_token_stream_contents_graph_boost(
            ts,
            output,
            start_offsets,
            end_offsets,
            types,
            pos_increments,
            pos_lengths,
            final_offset,
            true,
            boost,
        )
    }

    /// `assertTokenStreamContents(ts, output, startOffsets, endOffsets,
    /// types, posIncrements, finalOffset)`.
    ///
    /// # Errors
    ///
    /// As [`assert_token_stream_contents_full`](Self::assert_token_stream_contents_full).
    pub fn assert_token_stream_contents_types_final_offset(
        ts: &mut dyn TokenStream,
        output: &[&str],
        start_offsets: Option<&[i32]>,
        end_offsets: Option<&[i32]>,
        types: Option<&[&str]>,
        pos_increments: Option<&[i32]>,
        final_offset: Option<i32>,
    ) -> Result<()> {
        Self::assert_token_stream_contents_final_offset(
            ts,
            output,
            start_offsets,
            end_offsets,
            types,
            pos_increments,
            None,
            final_offset,
        )
    }

    /// `assertTokenStreamContents(ts, output, startOffsets, endOffsets,
    /// types, posIncrements)`.
    ///
    /// # Errors
    ///
    /// As [`assert_token_stream_contents_full`](Self::assert_token_stream_contents_full).
    pub fn assert_token_stream_contents_types(
        ts: &mut dyn TokenStream,
        output: &[&str],
        start_offsets: Option<&[i32]>,
        end_offsets: Option<&[i32]>,
        types: Option<&[&str]>,
        pos_increments: Option<&[i32]>,
    ) -> Result<()> {
        Self::assert_token_stream_contents_final_offset(
            ts,
            output,
            start_offsets,
            end_offsets,
            types,
            pos_increments,
            None,
            None,
        )
    }

    /// `assertTokenStreamContents(ts, output, startOffsets, endOffsets,
    /// types, posIncrements, posLengths)`.
    ///
    /// # Errors
    ///
    /// As [`assert_token_stream_contents_full`](Self::assert_token_stream_contents_full).
    pub fn assert_token_stream_contents_types_pos_lengths(
        ts: &mut dyn TokenStream,
        output: &[&str],
        start_offsets: Option<&[i32]>,
        end_offsets: Option<&[i32]>,
        types: Option<&[&str]>,
        pos_increments: Option<&[i32]>,
        pos_lengths: Option<&[i32]>,
    ) -> Result<()> {
        Self::assert_token_stream_contents_final_offset(
            ts,
            output,
            start_offsets,
            end_offsets,
            types,
            pos_increments,
            pos_lengths,
            None,
        )
    }

    /// `assertTokenStreamContents(ts, output)`.
    ///
    /// # Errors
    ///
    /// As [`assert_token_stream_contents_full`](Self::assert_token_stream_contents_full).
    pub fn assert_token_stream_contents(ts: &mut dyn TokenStream, output: &[&str]) -> Result<()> {
        Self::assert_token_stream_contents_final_offset(
            ts, output, None, None, None, None, None, None,
        )
    }

    /// `assertTokenStreamContents(ts, output, types)`.
    ///
    /// # Errors
    ///
    /// As [`assert_token_stream_contents_full`](Self::assert_token_stream_contents_full).
    pub fn assert_token_stream_contents_types_only(
        ts: &mut dyn TokenStream,
        output: &[&str],
        types: &[&str],
    ) -> Result<()> {
        Self::assert_token_stream_contents_final_offset(
            ts,
            output,
            None,
            None,
            Some(types),
            None,
            None,
            None,
        )
    }

    /// `assertTokenStreamContents(ts, output, posIncrements)`.
    ///
    /// # Errors
    ///
    /// As [`assert_token_stream_contents_full`](Self::assert_token_stream_contents_full).
    pub fn assert_token_stream_contents_pos_increments(
        ts: &mut dyn TokenStream,
        output: &[&str],
        pos_increments: &[i32],
    ) -> Result<()> {
        Self::assert_token_stream_contents_final_offset(
            ts,
            output,
            None,
            None,
            None,
            Some(pos_increments),
            None,
            None,
        )
    }

    /// `assertTokenStreamContents(ts, output, startOffsets, endOffsets)`.
    ///
    /// # Errors
    ///
    /// As [`assert_token_stream_contents_full`](Self::assert_token_stream_contents_full).
    pub fn assert_token_stream_contents_offsets(
        ts: &mut dyn TokenStream,
        output: &[&str],
        start_offsets: &[i32],
        end_offsets: &[i32],
    ) -> Result<()> {
        Self::assert_token_stream_contents_final_offset(
            ts,
            output,
            Some(start_offsets),
            Some(end_offsets),
            None,
            None,
            None,
            None,
        )
    }

    /// `assertTokenStreamContents(ts, output, startOffsets, endOffsets,
    /// finalOffset)`.
    ///
    /// # Errors
    ///
    /// As [`assert_token_stream_contents_full`](Self::assert_token_stream_contents_full).
    pub fn assert_token_stream_contents_offsets_final_offset(
        ts: &mut dyn TokenStream,
        output: &[&str],
        start_offsets: &[i32],
        end_offsets: &[i32],
        final_offset: Option<i32>,
    ) -> Result<()> {
        Self::assert_token_stream_contents_final_offset(
            ts,
            output,
            Some(start_offsets),
            Some(end_offsets),
            None,
            None,
            None,
            final_offset,
        )
    }

    /// `assertTokenStreamContents(ts, output, startOffsets, endOffsets,
    /// posIncrements)`.
    ///
    /// # Errors
    ///
    /// As [`assert_token_stream_contents_full`](Self::assert_token_stream_contents_full).
    pub fn assert_token_stream_contents_offsets_pos_increments(
        ts: &mut dyn TokenStream,
        output: &[&str],
        start_offsets: &[i32],
        end_offsets: &[i32],
        pos_increments: &[i32],
    ) -> Result<()> {
        Self::assert_token_stream_contents_final_offset(
            ts,
            output,
            Some(start_offsets),
            Some(end_offsets),
            None,
            Some(pos_increments),
            None,
            None,
        )
    }

    /// `assertTokenStreamContents(ts, output, startOffsets, endOffsets,
    /// posIncrements, finalOffset)`.
    ///
    /// # Errors
    ///
    /// As [`assert_token_stream_contents_full`](Self::assert_token_stream_contents_full).
    pub fn assert_token_stream_contents_offsets_pos_increments_final_offset(
        ts: &mut dyn TokenStream,
        output: &[&str],
        start_offsets: &[i32],
        end_offsets: &[i32],
        pos_increments: &[i32],
        final_offset: Option<i32>,
    ) -> Result<()> {
        Self::assert_token_stream_contents_final_offset(
            ts,
            output,
            Some(start_offsets),
            Some(end_offsets),
            None,
            Some(pos_increments),
            None,
            final_offset,
        )
    }

    /// `assertTokenStreamContents(ts, output, startOffsets, endOffsets,
    /// posIncrements, posLengths, finalOffset)`.
    ///
    /// # Errors
    ///
    /// As [`assert_token_stream_contents_full`](Self::assert_token_stream_contents_full).
    pub fn assert_token_stream_contents_offsets_positions_final_offset(
        ts: &mut dyn TokenStream,
        output: &[&str],
        start_offsets: &[i32],
        end_offsets: &[i32],
        pos_increments: &[i32],
        pos_lengths: &[i32],
        final_offset: Option<i32>,
    ) -> Result<()> {
        Self::assert_token_stream_contents_final_offset(
            ts,
            output,
            Some(start_offsets),
            Some(end_offsets),
            None,
            Some(pos_increments),
            Some(pos_lengths),
            final_offset,
        )
    }

    /// The UTF-16 length of `input`, Java's `input.length()`.
    fn length16(input: &str) -> i32 {
        input.encode_utf16().count() as i32
    }

    /// `a.tokenStream(field, text)`, then `body` over the stream.
    fn with_stream<T>(
        a: &dyn Analyzer,
        field: &str,
        text: &str,
        body: impl FnOnce(&mut dyn TokenStream) -> Result<T>,
    ) -> Result<T> {
        let ts = a.token_stream_from_str(field, text)?;
        let mut ts = ts.borrow_mut();
        body(&mut *ts)
    }

    /// `assertAnalyzesTo(a, input, output, startOffsets, endOffsets, types,
    /// posIncrements)`.
    ///
    /// # Errors
    ///
    /// What the analyzer throws.
    pub fn assert_analyzes_to_types(
        a: &dyn Analyzer,
        input: &str,
        output: &[&str],
        start_offsets: Option<&[i32]>,
        end_offsets: Option<&[i32]>,
        types: Option<&[&str]>,
        pos_increments: Option<&[i32]>,
    ) -> Result<()> {
        Self::with_stream(a, "dummy", input, |ts| {
            Self::assert_token_stream_contents_final_offset(
                ts,
                output,
                start_offsets,
                end_offsets,
                types,
                pos_increments,
                None,
                Some(Self::length16(input)),
            )
        })?;
        Self::check_reset_exception(a, input)?;
        Self::check_analysis_consistency(&LuceneTestCase::random(), a, true, input)
    }

    /// `assertAnalyzesTo(a, input, output, startOffsets, endOffsets, types,
    /// posIncrements, posLengths)`.
    ///
    /// # Errors
    ///
    /// What the analyzer throws.
    pub fn assert_analyzes_to_pos_lengths(
        a: &dyn Analyzer,
        input: &str,
        output: &[&str],
        start_offsets: Option<&[i32]>,
        end_offsets: Option<&[i32]>,
        types: Option<&[&str]>,
        pos_increments: Option<&[i32]>,
        pos_lengths: Option<&[i32]>,
    ) -> Result<()> {
        Self::assert_analyzes_to_boost(
            a,
            input,
            output,
            start_offsets,
            end_offsets,
            types,
            pos_increments,
            pos_lengths,
            None,
        )
    }

    /// `assertAnalyzesTo(a, input, output, startOffsets, endOffsets, types,
    /// posIncrements, posLengths, boost)`.
    ///
    /// # Errors
    ///
    /// What the analyzer throws.
    pub fn assert_analyzes_to_boost(
        a: &dyn Analyzer,
        input: &str,
        output: &[&str],
        start_offsets: Option<&[i32]>,
        end_offsets: Option<&[i32]>,
        types: Option<&[&str]>,
        pos_increments: Option<&[i32]>,
        pos_lengths: Option<&[i32]>,
        boost: Option<&[f32]>,
    ) -> Result<()> {
        Self::with_stream(a, "dummy", input, |ts| {
            Self::assert_token_stream_contents_final_offset_boost(
                ts,
                output,
                start_offsets,
                end_offsets,
                types,
                pos_increments,
                pos_lengths,
                Some(Self::length16(input)),
                boost,
            )
        })?;
        Self::check_reset_exception(a, input)?;
        Self::check_analysis_consistency(&LuceneTestCase::random(), a, true, input)
    }

    /// `assertAnalyzesTo(a, input, output, startOffsets, endOffsets, types,
    /// posIncrements, posLengths, graphOffsetsAreCorrect)`.
    ///
    /// # Errors
    ///
    /// What the analyzer throws.
    pub fn assert_analyzes_to_graph(
        a: &dyn Analyzer,
        input: &str,
        output: &[&str],
        start_offsets: Option<&[i32]>,
        end_offsets: Option<&[i32]>,
        types: Option<&[&str]>,
        pos_increments: Option<&[i32]>,
        pos_lengths: Option<&[i32]>,
        graph_offsets_are_correct: bool,
    ) -> Result<()> {
        Self::with_stream(a, "dummy", input, |ts| {
            Self::assert_token_stream_contents_graph(
                ts,
                output,
                start_offsets,
                end_offsets,
                types,
                pos_increments,
                pos_lengths,
                Some(Self::length16(input)),
                graph_offsets_are_correct,
            )
        })?;
        Self::check_reset_exception(a, input)?;
        Self::check_analysis_consistency_graph(
            &LuceneTestCase::random(),
            a,
            true,
            input,
            graph_offsets_are_correct,
        )
    }

    /// `assertAnalyzesTo(a, input, output, startOffsets, endOffsets, types,
    /// posIncrements, posLengths, graphOffsetsAreCorrect, payloads)`.
    ///
    /// # Errors
    ///
    /// What the analyzer throws.
    pub fn assert_analyzes_to_payloads(
        a: &dyn Analyzer,
        input: &str,
        output: &[&str],
        start_offsets: Option<&[i32]>,
        end_offsets: Option<&[i32]>,
        types: Option<&[&str]>,
        pos_increments: Option<&[i32]>,
        pos_lengths: Option<&[i32]>,
        graph_offsets_are_correct: bool,
        payloads: Option<&[Option<&[u8]>]>,
    ) -> Result<()> {
        Self::with_stream(a, "dummy", input, |ts| {
            Self::assert_token_stream_contents_payloads(
                ts,
                output,
                start_offsets,
                end_offsets,
                types,
                pos_increments,
                pos_lengths,
                Some(Self::length16(input)),
                None,
                None,
                graph_offsets_are_correct,
                payloads,
            )
        })?;
        Self::check_reset_exception(a, input)?;
        Self::check_analysis_consistency_graph(
            &LuceneTestCase::random(),
            a,
            true,
            input,
            graph_offsets_are_correct,
        )
    }

    /// `assertAnalyzesTo(a, input, output)`.
    ///
    /// # Errors
    ///
    /// What the analyzer throws.
    pub fn assert_analyzes_to(a: &dyn Analyzer, input: &str, output: &[&str]) -> Result<()> {
        Self::assert_analyzes_to_pos_lengths(a, input, output, None, None, None, None, None)
    }

    /// `assertAnalyzesTo(a, input, output, types)`.
    ///
    /// # Errors
    ///
    /// What the analyzer throws.
    pub fn assert_analyzes_to_types_only(
        a: &dyn Analyzer,
        input: &str,
        output: &[&str],
        types: &[&str],
    ) -> Result<()> {
        Self::assert_analyzes_to_pos_lengths(a, input, output, None, None, Some(types), None, None)
    }

    /// `assertAnalyzesTo(a, input, output, posIncrements)`.
    ///
    /// # Errors
    ///
    /// What the analyzer throws.
    pub fn assert_analyzes_to_pos_increments(
        a: &dyn Analyzer,
        input: &str,
        output: &[&str],
        pos_increments: &[i32],
    ) -> Result<()> {
        Self::assert_analyzes_to_pos_lengths(
            a,
            input,
            output,
            None,
            None,
            None,
            Some(pos_increments),
            None,
        )
    }

    /// `assertAnalyzesToPositions(a, input, output, posIncrements,
    /// posLengths)`.
    ///
    /// # Errors
    ///
    /// What the analyzer throws.
    pub fn assert_analyzes_to_positions(
        a: &dyn Analyzer,
        input: &str,
        output: &[&str],
        pos_increments: &[i32],
        pos_lengths: &[i32],
    ) -> Result<()> {
        Self::assert_analyzes_to_pos_lengths(
            a,
            input,
            output,
            None,
            None,
            None,
            Some(pos_increments),
            Some(pos_lengths),
        )
    }

    /// `assertAnalyzesToPositions(a, input, output, types, posIncrements,
    /// posLengths)`.
    ///
    /// # Errors
    ///
    /// What the analyzer throws.
    pub fn assert_analyzes_to_positions_types(
        a: &dyn Analyzer,
        input: &str,
        output: &[&str],
        types: &[&str],
        pos_increments: &[i32],
        pos_lengths: &[i32],
    ) -> Result<()> {
        Self::assert_analyzes_to_pos_lengths(
            a,
            input,
            output,
            None,
            None,
            Some(types),
            Some(pos_increments),
            Some(pos_lengths),
        )
    }

    /// `assertAnalyzesTo(a, input, output, startOffsets, endOffsets)`.
    ///
    /// # Errors
    ///
    /// What the analyzer throws.
    pub fn assert_analyzes_to_offsets(
        a: &dyn Analyzer,
        input: &str,
        output: &[&str],
        start_offsets: &[i32],
        end_offsets: &[i32],
    ) -> Result<()> {
        Self::assert_analyzes_to_pos_lengths(
            a,
            input,
            output,
            Some(start_offsets),
            Some(end_offsets),
            None,
            None,
            None,
        )
    }

    /// `assertAnalyzesTo(a, input, output, startOffsets, endOffsets,
    /// posIncrements)`.
    ///
    /// # Errors
    ///
    /// What the analyzer throws.
    pub fn assert_analyzes_to_offsets_pos_increments(
        a: &dyn Analyzer,
        input: &str,
        output: &[&str],
        start_offsets: &[i32],
        end_offsets: &[i32],
        pos_increments: &[i32],
    ) -> Result<()> {
        Self::assert_analyzes_to_pos_lengths(
            a,
            input,
            output,
            Some(start_offsets),
            Some(end_offsets),
            None,
            Some(pos_increments),
            None,
        )
    }

    /// `checkResetException(a, input)`: a stream consumed without `reset()`
    /// must throw `IllegalStateException`, and asking for a new stream before
    /// closing the previous one must too.
    ///
    /// # Errors
    ///
    /// What the analyzer throws while consuming correctly.
    pub fn check_reset_exception(a: &dyn Analyzer, input: &str) -> Result<()> {
        let ts = a.token_stream_from_str("bogus", input)?;
        let outcome = match ts.borrow_mut().increment_token() {
            Ok(true) => Some("didn't get expected exception when reset() not called".to_string()),
            Ok(false) => None,
            Err(e) if is_illegal_state(&e) => None, // ok
            Err(unexpected) => {
                eprintln!("{}", java_to_string(&unexpected));
                Some(format!(
                    "got wrong exception when reset() not called: {}",
                    java_to_string(&unexpected)
                ))
            }
        };
        {
            // consume correctly
            let mut ts = ts.borrow_mut();
            ts.reset()?;
            while ts.increment_token()? {}
            ts.end()?;
            ts.close()?;
        }
        if let Some(message) = outcome {
            Assert::fail(&message);
        }

        // check for a missing close()
        let ts = a.token_stream_from_str("bogus", input)?;
        {
            let mut ts = ts.borrow_mut();
            ts.reset()?;
            while ts.increment_token()? {}
            ts.end()?;
        }
        match a.token_stream_from_str("bogus", input) {
            Ok(new_ts) => {
                new_ts.borrow_mut().close()?;
                Assert::fail("didn't get expected exception when close() not called");
            }
            Err(e) if is_illegal_state(&e) => {
                // ok
                ts.borrow_mut().close()?;
            }
            Err(e) => {
                ts.borrow_mut().close()?;
                return Err(e);
            }
        }
        Ok(())
    }

    /// Simple utility method for testing stemmers.
    ///
    /// # Errors
    ///
    /// What the analyzer throws.
    pub fn check_one_term(a: &dyn Analyzer, input: &str, expected: &str) -> Result<()> {
        Self::assert_analyzes_to(a, input, &[expected])
    }

    /// Utility method for blasting tokenstreams with data to make sure they
    /// don't do anything crazy.
    ///
    /// # Errors
    ///
    /// What the analyzer throws.
    pub fn check_random_data(
        random: &dyn Random,
        a: &Arc<dyn Analyzer>,
        iterations: i32,
    ) -> Result<()> {
        Self::check_random_data_graph(random, a, iterations, 20, false, true)
    }

    /// `checkRandomData(random, a, iterations, maxWordLength)`.
    ///
    /// # Errors
    ///
    /// What the analyzer throws.
    pub fn check_random_data_max_word_length(
        random: &dyn Random,
        a: &Arc<dyn Analyzer>,
        iterations: i32,
        max_word_length: i32,
    ) -> Result<()> {
        Self::check_random_data_graph(random, a, iterations, max_word_length, false, true)
    }

    /// `checkRandomData(random, a, iterations, simple)`.
    ///
    /// # Errors
    ///
    /// What the analyzer throws.
    pub fn check_random_data_simple(
        random: &dyn Random,
        a: &Arc<dyn Analyzer>,
        iterations: i32,
        simple: bool,
    ) -> Result<()> {
        Self::check_random_data_graph(random, a, iterations, 20, simple, true)
    }

    /// Asserts that the given stream has expected number of tokens.
    ///
    /// # Errors
    ///
    /// What the stream throws.
    pub fn assert_stream_has_number_of_tokens(
        ts: &mut dyn TokenStream,
        expected_count: i32,
    ) -> Result<()> {
        ts.reset()?;
        let mut count = 0;
        while ts.increment_token()? {
            count += 1;
        }
        ts.end()?;
        Assert::assert_equals_msg(
            Some("wrong number of tokens"),
            i64::from(expected_count),
            i64::from(count),
        );
        Ok(())
    }

    /// `checkRandomData(random, a, iterations, maxWordLength, simple)`.
    ///
    /// # Errors
    ///
    /// What the analyzer throws.
    pub fn check_random_data_max_word_length_simple(
        random: &dyn Random,
        a: &Arc<dyn Analyzer>,
        iterations: i32,
        max_word_length: i32,
        simple: bool,
    ) -> Result<()> {
        Self::check_random_data_graph(random, a, iterations, max_word_length, simple, true)
    }

    /// `checkRandomData(random, a, iterations, maxWordLength, simple,
    /// graphOffsetsAreCorrect)`: the checks over random text, first on the
    /// calling thread, then the very same on 2 to 4 threads at once.
    ///
    /// # Errors
    ///
    /// What the analyzer throws; `RuntimeException` (`some thread(s)
    /// failed`) when a thread failed.
    pub fn check_random_data_graph(
        random: &dyn Random,
        a: &Arc<dyn Analyzer>,
        iterations: i32,
        max_word_length: i32,
        simple: bool,
        graph_offsets_are_correct: bool,
    ) -> Result<()> {
        Self::check_reset_exception(&**a, "best effort")?;
        let seed = random.next_long();
        let use_char_filter = random.next_boolean();
        let mut dir: Option<TestDirectory> = None;
        let mut iw: Option<Arc<RandomIndexWriter>> = None;
        let postings_format = TestUtil::get_postings_format("dummy");
        let codec_ok =
            iterations.wrapping_mul(max_word_length) < 100000 && postings_format != "SimpleText";
        if LuceneTestCase::rarely_with(random) && codec_ok {
            let d =
                LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir_prefix("bttc"));
            iw = Some(Arc::new(RandomIndexWriter::new(
                &JdkRandom::new(seed),
                d.as_directory(),
                Arc::clone(a),
            )?));
            dir = Some(d);
        }
        let result = Self::check_random_data_threads(
            seed,
            a,
            iterations,
            max_word_length,
            use_char_filter,
            simple,
            graph_offsets_are_correct,
            random,
            &mut iw,
        );
        // The writer's handle on the directory goes with it: a
        // `MockDirectoryWrapper` closes its delegate only once no copy of it
        // is alive.
        drop(iw);
        match result {
            Ok(()) => {
                // `IOUtils.close(dir)`
                if let Some(mut dir) = dir {
                    dir.close()?;
                }
                Ok(())
            }
            Err(e) => {
                // `IOUtils.closeWhileHandlingException(dir); // checkindex`
                if let Some(mut dir) = dir {
                    let _ = dir.close();
                }
                Err(e)
            }
        }
    }

    /// The `try` block of `checkRandomData(random, a, iterations,
    /// maxWordLength, simple, graphOffsetsAreCorrect)`: the checks on the
    /// calling thread, then on 2 to 4 threads, then `iw.close()`.
    fn check_random_data_threads(
        seed: i64,
        a: &Arc<dyn Analyzer>,
        iterations: i32,
        max_word_length: i32,
        use_char_filter: bool,
        simple: bool,
        graph_offsets_are_correct: bool,
        random: &dyn Random,
        iw: &mut Option<Arc<RandomIndexWriter>>,
    ) -> Result<()> {
        Self::check_random_data_inner(
            &(Arc::new(JdkRandom::new(seed)) as Arc<dyn Random>),
            &**a,
            iterations,
            max_word_length,
            use_char_filter,
            simple,
            graph_offsets_are_correct,
            iw.as_deref(),
        )?;
        // now test with multiple threads: note we do the EXACT same thing we did before in each
        // thread,
        // so this should only really fail from another thread if it's an actual thread problem
        let num_threads = TestUtil::next_int(random, 2, 4);
        let starting_gun = Arc::new((Mutex::new(false), Condvar::new()));
        let mut threads = Vec::with_capacity(num_threads as usize);
        for _ in 0..num_threads {
            let failed = Arc::new(AtomicBool::new(false));
            let thread = AnalysisThread {
                seed,
                latch: Arc::clone(&starting_gun),
                a: Arc::clone(a),
                iterations,
                max_word_length,
                use_char_filter,
                simple,
                graph_offsets_are_correct,
                iw: iw.clone(),
                failed: Arc::clone(&failed),
            };
            threads.push((Thread::unnamed(move || thread.run()), failed));
        }
        for (thread, _) in &threads {
            thread.start();
        }
        {
            let (lock, cvar) = &*starting_gun;
            *lock.lock().unwrap_or_else(|e| e.into_inner()) = true;
            cvar.notify_all();
        }
        for (thread, _) in &threads {
            let _ = thread.join();
        }
        for (_, failed) in &threads {
            if failed.load(Ordering::Relaxed) {
                return Err(LuceneError::unchecked(
                    "java.lang.RuntimeException",
                    "some thread(s) failed",
                ));
            }
        }
        drop(threads);
        if let Some(iw) = iw.as_ref() {
            iw.close()?;
        }
        Ok(())
    }

    /// The private `checkRandomData(random, a, iterations, maxWordLength,
    /// useCharFilter, simple, graphOffsetsAreCorrect, iw)`: the checks over
    /// `iterations` random texts, each also indexed through `iw` when there is
    /// one.
    #[allow(clippy::too_many_arguments)]
    fn check_random_data_inner(
        random: &Arc<dyn Random>,
        a: &dyn Analyzer,
        iterations: i32,
        max_word_length: i32,
        use_char_filter: bool,
        simple: bool,
        graph_offsets_are_correct: bool,
        iw: Option<&RandomIndexWriter>,
    ) -> Result<()> {
        // `doc`, the field type of `field`, and the reader cell of
        // `currentField`: `field.setReaderValue(r)` replaces what the cell
        // holds. Java's `field` is put back into the document after a
        // multivalued pile-up; its reader is always replaced before the next
        // `addDocument`, so a new field of the same type stands for it.
        let mut indexing: Option<(Document, FieldType, SharedReader)> = None;
        if iw.is_some() {
            let mut doc = Document::new();
            let mut ft = FieldType::new_from(TextField::type_not_stored());
            if random.next_boolean() {
                ft.set_store_term_vectors(true)?;
                ft.set_store_term_vector_offsets(random.next_boolean())?;
                ft.set_store_term_vector_positions(random.next_boolean())?;
                if ft.store_term_vector_positions() {
                    ft.set_store_term_vector_payloads(random.next_boolean())?;
                }
            }
            if random.next_boolean() {
                ft.set_omit_norms(true)?;
            }
            match random.next_int_bound(4) {
                0 => ft.set_index_options(IndexOptions::DOCS)?,
                1 => ft.set_index_options(IndexOptions::DOCS_AND_FREQS)?,
                2 => ft.set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)?,
                _ => {
                    ft.set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS)?
                }
            }
            let current_field = Self::new_dummy_field(&mut doc, &ft)?;
            indexing = Some((doc, ft, current_field));
        }

        for _ in 0..iterations {
            let text = TestUtil::random_analysis_string(&**random, max_word_length, simple);

            let report = |text: &str| {
                // TODO: really we should pass a random seed to
                // checkAnalysisConsistency then print it here too:
                eprintln!(
                    "TEST FAIL: useCharFilter={} text='{}'",
                    use_char_filter,
                    Self::escape(text)
                );
            };
            match catch_unwind(AssertUnwindSafe(|| {
                Self::check_analysis_consistency_field(
                    random,
                    a,
                    use_char_filter,
                    &text,
                    graph_offsets_are_correct,
                    indexing.as_ref().map(|(_, _, current_field)| current_field),
                )?;
                if let (Some(iw), Some((doc, ft, current_field))) = (iw, indexing.as_mut()) {
                    if random.next_int_bound(7) == 0 {
                        // pile up a multivalued field
                        *current_field = Self::new_dummy_field(doc, ft)?;
                    } else {
                        iw.add_document(&*doc)?;
                        if doc.get_fields().len() > 1 {
                            // back to 1 field
                            doc.remove_fields("dummy");
                            *current_field = Self::new_dummy_field(doc, ft)?;
                        }
                    }
                }
                Ok(())
            })) {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    report(&text);
                    return Err(e);
                }
                Err(payload) => {
                    report(&text);
                    resume_unwind(payload);
                }
            }
        }
        Ok(())
    }

    /// `new Field("dummy", bogus, ft)` added to `doc`: the reader cell of the
    /// new field (Java's fields share the empty `StringReader` `bogus`, which
    /// is replaced before every use).
    fn new_dummy_field(doc: &mut Document, ft: &FieldType) -> Result<SharedReader> {
        let field =
            Field::new_with_reader("dummy", Box::new(StringCharReader::new("")), ft.clone())?;
        let reader = field
            .reader_value()
            .expect("INVARIANT: a field built with a reader has a reader value");
        doc.add(Box::new(field));
        Ok(reader)
    }

    /// `escape(String)`: the string with `\n`, `\r`, `"` and `\` escaped and
    /// every UTF-16 unit outside printable ASCII as `\uXXXX`.
    pub fn escape(s: &str) -> String {
        let mut sb = String::new();
        for c in s.encode_utf16() {
            if c == 0xa {
                // Strangely, you cannot put \ u000A into Java
                // sources (not in a comment nor a string
                // constant)...:
                sb.push_str("\\n");
            } else if c == 0xd {
                // ... nor \ u000D:
                sb.push_str("\\r");
            } else if c == u16::from(b'"') {
                sb.push_str("\\\"");
            } else if c == u16::from(b'\\') {
                sb.push_str("\\\\");
            } else if (0x20..0x80).contains(&c) {
                sb.push(char::from(c as u8));
            } else {
                // TODO: we can make ascii easier to read if we
                // don't escape...
                sb.push_str(&format!("\\u{:04x}", c));
            }
        }
        sb
    }

    /// `checkAnalysisConsistency(random, a, useCharFilter, text)`.
    ///
    /// # Errors
    ///
    /// What the analyzer throws.
    pub fn check_analysis_consistency(
        random: &Arc<dyn Random>,
        a: &dyn Analyzer,
        use_char_filter: bool,
        text: &str,
    ) -> Result<()> {
        Self::check_analysis_consistency_graph(random, a, use_char_filter, text, true)
    }

    fn reader(
        text: &str,
        use_char_filter: bool,
        remainder: i32,
        wrap: Option<Arc<dyn Random>>,
    ) -> Result<Box<dyn CharReader>> {
        let mut reader: Box<dyn CharReader> = Box::new(StringCharReader::new(text));
        if let Some(random) = wrap {
            reader = Box::new(MockReaderWrapper::new(random, reader));
        }
        Ok(if use_char_filter {
            Box::new(MockCharFilter::new(reader, remainder)?)
        } else {
            reader
        })
    }

    /// `checkAnalysisConsistency(random, a, useCharFilter, text,
    /// graphOffsetsAreCorrect)`: analyzes `text` once to record its tokens,
    /// maybe re-runs it with an exception from the reader or a partial
    /// consumption, and checks that a final clean pass yields the same
    /// tokens.
    ///
    /// # Errors
    ///
    /// What the analyzer throws.
    pub fn check_analysis_consistency_graph(
        random: &Arc<dyn Random>,
        a: &dyn Analyzer,
        use_char_filter: bool,
        text: &str,
        graph_offsets_are_correct: bool,
    ) -> Result<()> {
        Self::check_analysis_consistency_field(
            random,
            a,
            use_char_filter,
            text,
            graph_offsets_are_correct,
            None,
        )
    }

    /// The private `checkAnalysisConsistency(random, a, useCharFilter, text,
    /// graphOffsetsAreCorrect, field)`: the checks, then, when there is a
    /// field, its reader is set to `text` (through the same char filter and,
    /// one time in 30, a spoon-feeding reader).
    fn check_analysis_consistency_field(
        random: &Arc<dyn Random>,
        a: &dyn Analyzer,
        use_char_filter: bool,
        text: &str,
        graph_offsets_are_correct: bool,
        field: Option<&SharedReader>,
    ) -> Result<()> {
        if *VERBOSE {
            println!(
                "{:?}: NOTE: BaseTokenStreamTestCase: get first token stream now text={}",
                std::thread::current().name(),
                text
            );
        }

        let remainder = random.next_int_bound(10);
        let ts = a.token_stream(
            "dummy",
            Self::reader(text, use_char_filter, remainder, None)?,
        )?;
        let (term_att, offset_att, pos_inc_att, pos_length_att, type_att) = {
            let ts = ts.borrow();
            let source = ts.attribute_source();
            (
                Att::get(source, char_term_id()),
                Att::get(source, offset_id()),
                Att::get(source, pos_inc_id()),
                Att::get(source, pos_len_id()),
                Att::get(source, type_id()),
            )
        };

        let mut tokens: Vec<String> = Vec::new();
        let mut types: Vec<String> = Vec::new();
        let mut positions: Vec<i32> = Vec::new();
        let mut position_lengths: Vec<i32> = Vec::new();
        let mut start_offsets: Vec<i32> = Vec::new();
        let mut end_offsets: Vec<i32> = Vec::new();
        {
            let mut ts = ts.borrow_mut();
            ts.reset()?;

            // First pass: save away "correct" tokens
            while ts.increment_token()? {
                Assert::assert_true_msg("has no CharTermAttribute", term_att.is_some());
                tokens.push(term_att.as_ref().unwrap().term_string());
                if let Some(a) = &type_att {
                    types.push(a.type_att().type_value().to_string());
                }
                if let Some(a) = &pos_inc_att {
                    positions.push(a.pos_inc().get_position_increment());
                }
                if let Some(a) = &pos_length_att {
                    position_lengths.push(a.pos_len().get_position_length());
                }
                if let Some(a) = &offset_att {
                    let o = a.offset();
                    start_offsets.push(o.start_offset());
                    end_offsets.push(o.end_offset());
                }
            }
            ts.end()?;
            ts.close()?;
        }

        // verify reusing is "reproducable" and also get the normal tokenstream sanity checks
        if !tokens.is_empty() {
            // KWTokenizer (for example) can produce a token
            // even when input is length 0:
            if !text.is_empty() {
                // (Optional) second pass: do something evil:
                let evilness = random.next_int_bound(50);
                if evilness == 17 {
                    if *VERBOSE {
                        println!(
                            "{:?}: NOTE: BaseTokenStreamTestCase: re-run analysis w/ exception",
                            std::thread::current().name()
                        );
                    }
                    // Throw an errant exception from the Reader:

                    let mut evil_reader = MockReaderWrapper::new(
                        Arc::clone(random),
                        Box::new(StringCharReader::new(text)),
                    );
                    evil_reader
                        .throw_exc_after_char(random.next_int_bound(Self::length16(text) + 1));
                    let reader: Box<dyn CharReader> = if use_char_filter {
                        Box::new(MockCharFilter::new(Box::new(evil_reader), remainder)?)
                    } else {
                        Box::new(evil_reader)
                    };

                    // NOTE: some Tokenizers go and read characters
                    // when you call .setReader(Reader), eg
                    // PatternTokenizer.  This is a bit
                    // iffy... (really, they should only
                    // pull from the Reader when you call
                    // .incremenToken(), I think?), but we
                    // currently allow it, so, we must call
                    // a.tokenStream inside the try since we may
                    // hit the exc on init:
                    let mut current = Rc::clone(&ts);
                    let attempt: Result<()> = (|| {
                        current = a.token_stream("dummy", reader)?;
                        let mut ts = current.borrow_mut();
                        ts.reset()?;
                        while ts.increment_token()? {}
                        Ok(())
                    })();
                    match attempt {
                        Ok(()) => Assert::fail("did not hit exception"),
                        Err(re) if is_runtime_exception(&re) => {
                            Assert::assert_true(MockReaderWrapper::is_my_evil_exception(&re));
                        }
                        Err(e) => return Err(e),
                    }
                    let mut ts = current.borrow_mut();
                    if let Err(ise) = ts.end() {
                        // Catch & ignore MockTokenizer's
                        // anger...
                        match &ise {
                            LuceneError::IllegalState { message, .. }
                                if message.contains("end() called in wrong state=") =>
                            {
                                // OK
                            }
                            _ => return Err(ise),
                        }
                    }
                    ts.close()?;
                } else if evilness == 7 {
                    // Only consume a subset of the tokens:
                    let num_tokens_to_read = random.next_int_bound(tokens.len() as i32);
                    if *VERBOSE {
                        println!(
                            "{:?}: NOTE: BaseTokenStreamTestCase: re-run analysis, only consuming {} of {} tokens",
                            std::thread::current().name(),
                            num_tokens_to_read,
                            tokens.len()
                        );
                    }
                    let ts = a.token_stream(
                        "dummy",
                        Self::reader(text, use_char_filter, remainder, None)?,
                    )?;
                    let mut ts = ts.borrow_mut();
                    ts.reset()?;
                    for _ in 0..num_tokens_to_read {
                        Assert::assert_true(ts.increment_token()?);
                    }
                    if let Err(ise) = ts.end() {
                        // Catch & ignore MockTokenizer's
                        // anger...
                        match &ise {
                            LuceneError::IllegalState { message, .. }
                                if message.contains("end() called in wrong state=") =>
                            {
                                // OK
                            }
                            _ => return Err(ise),
                        }
                    }
                    ts.close()?;
                }
            }
        }

        // Final pass: verify clean tokenization matches
        // results from first pass:

        if *VERBOSE {
            println!(
                "{:?}: NOTE: BaseTokenStreamTestCase: re-run analysis; {} tokens",
                std::thread::current().name(),
                tokens.len()
            );
        }
        let seed = random.next_long();
        let random: Arc<dyn Random> = Arc::new(JdkRandom::new(seed));
        let wrap = if random.next_int_bound(30) == 7 {
            if *VERBOSE {
                println!(
                    "{:?}: NOTE: BaseTokenStreamTestCase: using spoon-feed reader",
                    std::thread::current().name()
                );
            }
            Some(Arc::clone(&random))
        } else {
            None
        };

        let ts = a.token_stream(
            "dummy",
            Self::reader(text, use_char_filter, remainder, wrap)?,
        )?;
        let tokens: Vec<&str> = tokens.iter().map(String::as_str).collect();
        let types: Vec<&str> = types.iter().map(String::as_str).collect();
        let final_offset = Some(Self::length16(text));
        {
            let mut ts = ts.borrow_mut();
            let ts = &mut *ts;
            if type_att.is_some()
                && pos_inc_att.is_some()
                && pos_length_att.is_some()
                && offset_att.is_some()
            {
                // offset + pos + posLength + type
                Self::assert_token_stream_contents_graph(
                    ts,
                    &tokens,
                    Some(&start_offsets),
                    Some(&end_offsets),
                    Some(&types),
                    Some(&positions),
                    Some(&position_lengths),
                    final_offset,
                    graph_offsets_are_correct,
                )?;
            } else if type_att.is_some() && pos_inc_att.is_some() && offset_att.is_some() {
                // offset + pos + type
                Self::assert_token_stream_contents_graph(
                    ts,
                    &tokens,
                    Some(&start_offsets),
                    Some(&end_offsets),
                    Some(&types),
                    Some(&positions),
                    None,
                    final_offset,
                    graph_offsets_are_correct,
                )?;
            } else if pos_inc_att.is_some() && pos_length_att.is_some() && offset_att.is_some() {
                // offset + pos + posLength
                Self::assert_token_stream_contents_graph(
                    ts,
                    &tokens,
                    Some(&start_offsets),
                    Some(&end_offsets),
                    None,
                    Some(&positions),
                    Some(&position_lengths),
                    final_offset,
                    graph_offsets_are_correct,
                )?;
            } else if pos_inc_att.is_some() && offset_att.is_some() {
                // offset + pos
                Self::assert_token_stream_contents_graph(
                    ts,
                    &tokens,
                    Some(&start_offsets),
                    Some(&end_offsets),
                    None,
                    Some(&positions),
                    None,
                    final_offset,
                    graph_offsets_are_correct,
                )?;
            } else if offset_att.is_some() {
                // offset
                Self::assert_token_stream_contents_graph(
                    ts,
                    &tokens,
                    Some(&start_offsets),
                    Some(&end_offsets),
                    None,
                    None,
                    None,
                    final_offset,
                    graph_offsets_are_correct,
                )?;
            } else {
                // terms only
                Self::assert_token_stream_contents(ts, &tokens)?;
            }
        }

        a.normalize("dummy", text)?;
        // TODO: what can we do besides testing that the above method does not throw?

        if let Some(field) = field {
            let mut reader: Box<dyn CharReader> = Box::new(StringCharReader::new(text));
            let random = JdkRandom::new(seed);
            if random.next_int_bound(30) == 7 {
                if *VERBOSE {
                    println!(
                        "{:?}: NOTE: BaseTokenStreamTestCase: indexing using spoon-feed reader",
                        std::thread::current().name()
                    );
                }

                reader = Box::new(MockReaderWrapper::new(Arc::new(random), reader));
            }

            // `field.setReaderValue(...)`
            *field.borrow_mut() = if use_char_filter {
                Box::new(MockCharFilter::new(reader, remainder)?)
            } else {
                reader
            };
        }
        Ok(())
    }

    /// `toDot(Analyzer a, String inputText)`: the dot graph of the analysis
    /// of `input_text`.
    ///
    /// # Errors
    ///
    /// What the analyzer throws.
    pub fn to_dot(a: &dyn Analyzer, input_text: &str) -> Result<String> {
        Self::with_stream(a, "field", input_text, |ts| {
            ts.reset()?;
            let mut dot = TokenStreamToDot::new(Some(input_text), ts)?;
            dot.to_dot()?;
            Ok(dot.out)
        })
    }

    /// `toDotFile(Analyzer a, String inputText, String localFileName)`.
    ///
    /// # Errors
    ///
    /// What the analyzer throws, and the I/O errors of writing the file.
    pub fn to_dot_file(a: &dyn Analyzer, input_text: &str, local_file_name: &str) -> Result<()> {
        let dot = Self::to_dot(a, input_text)?;
        std::fs::write(local_file_name, dot)?;
        Ok(())
    }

    /// `whitespaceMockTokenizer(Reader input)`.
    ///
    /// # Errors
    ///
    /// As [`MockTokenizer::new`].
    pub fn whitespace_mock_tokenizer(input: Box<dyn CharReader>) -> Result<MockTokenizer> {
        let mut mock_tokenizer = MockTokenizer::new(Arc::clone(&WHITESPACE), false)?;
        mock_tokenizer.set_reader(input)?;
        Ok(mock_tokenizer)
    }

    /// `whitespaceMockTokenizer(String input)`.
    ///
    /// # Errors
    ///
    /// As [`MockTokenizer::new`].
    pub fn whitespace_mock_tokenizer_str(input: &str) -> Result<MockTokenizer> {
        Self::whitespace_mock_tokenizer(Box::new(StringCharReader::new(input)))
    }

    /// `keywordMockTokenizer(Reader input)`.
    ///
    /// # Errors
    ///
    /// As [`MockTokenizer::new`].
    pub fn keyword_mock_tokenizer(input: Box<dyn CharReader>) -> Result<MockTokenizer> {
        let mut mock_tokenizer = MockTokenizer::new(Arc::clone(&KEYWORD), false)?;
        mock_tokenizer.set_reader(input)?;
        Ok(mock_tokenizer)
    }

    /// `keywordMockTokenizer(String input)`.
    ///
    /// # Errors
    ///
    /// As [`MockTokenizer::new`].
    pub fn keyword_mock_tokenizer_str(input: &str) -> Result<MockTokenizer> {
        Self::keyword_mock_tokenizer(Box::new(StringCharReader::new(input)))
    }

    /// Returns a random AttributeFactory impl.
    pub fn new_attribute_factory_with(random: &dyn Random) -> Arc<dyn AttributeFactory> {
        match random.next_int_bound(3) {
            0 => default_token_attribute_factory(),
            1 => Token::token_attribute_factory(),
            2 => default_attribute_factory(),
            _ => Throwable::new(
                ASSERTION_ERROR,
                "Please fix the Random.nextInt() call above",
            )
            .throw(),
        }
    }

    /// Returns a random AttributeFactory impl, drawn from the context's
    /// random.
    pub fn new_attribute_factory() -> Arc<dyn AttributeFactory> {
        Self::new_attribute_factory_with(&*LuceneTestCase::random())
    }

    fn to_string_set(strings: &HashSet<String>) -> String {
        let strings_list: BTreeSet<&String> = strings.iter().collect();
        let mut b = String::new();
        for s in strings_list {
            b.push_str("  ");
            b.push_str(s);
            b.push('\n');
        }
        b
    }

    /// Enumerates all accepted strings in the token graph created by the
    /// analyzer on the provided text, and then asserts that it's equal to
    /// the expected strings. Uses [`TokenStreamToAutomaton`] to create an
    /// automaton. Asserts the finite strings of the automaton are all and
    /// only the given valid strings.
    ///
    /// # Errors
    ///
    /// What the analyzer throws.
    pub fn assert_graph_strings(
        analyzer: &dyn Analyzer,
        text: &str,
        expected_strings: &[&str],
    ) -> Result<()> {
        Self::check_analysis_consistency_graph(
            &LuceneTestCase::random(),
            analyzer,
            true,
            text,
            true,
        )?;
        Self::with_stream(analyzer, "dummy", text, |ts| {
            let result = Self::assert_graph_strings_of(ts, expected_strings);
            ts.close()?;
            result
        })
    }

    /// Enumerates all accepted strings in the token graph created by the
    /// already initialized [`TokenStream`].
    ///
    /// # Errors
    ///
    /// What the stream throws.
    pub fn assert_graph_strings_of(
        token_stream: &mut dyn TokenStream,
        expected_strings: &[&str],
    ) -> Result<()> {
        let actual_strings = Self::get_graph_strings_of(token_stream)?;
        let expected_strings_set: HashSet<String> =
            expected_strings.iter().map(|s| s.to_string()).collect();
        let message = |s: &str| {
            format!(
                "Analyzer created unexpected string path: {}\nexpected:\n{}\nactual:\n{}",
                s,
                Self::to_string_set(&expected_strings_set),
                Self::to_string_set(&actual_strings)
            )
        };
        for s in &actual_strings {
            Assert::assert_true_msg(&message(s), expected_strings_set.contains(s));
        }
        for s in expected_strings {
            Assert::assert_true_msg(&message(s), actual_strings.contains(*s));
        }
        Ok(())
    }

    /// Returns all paths accepted by the token stream graph produced by
    /// analyzing text with the provided analyzer. The tokens CharTermAttribute
    /// values are concatenated, and separated with space.
    ///
    /// # Errors
    ///
    /// What the analyzer throws.
    pub fn get_graph_strings(analyzer: &dyn Analyzer, text: &str) -> Result<HashSet<String>> {
        Self::with_stream(analyzer, "dummy", text, |ts| {
            let result = Self::get_graph_strings_of(ts);
            ts.close()?;
            result
        })
    }

    /// Returns all paths accepted by the token stream graph produced by the
    /// already initialized [`TokenStream`].
    ///
    /// # Errors
    ///
    /// What the stream throws.
    pub fn get_graph_strings_of(token_stream: &mut dyn TokenStream) -> Result<HashSet<String>> {
        let automaton = TokenStreamToAutomaton::new().to_automaton(token_stream)?;
        let actual_string_paths = AutomatonTestUtil::get_finite_strings_recursive(&automaton, -1);
        let mut scratch = BytesRefBuilder::new();
        let mut paths = HashSet::new();
        for ir in actual_string_paths {
            let ints = IntsRef::new(ir);
            let bytes = Util::to_bytes_ref(&ints, &mut scratch)?;
            let s = bytes.utf8_to_string()?;
            let sep = char::from_u32(POS_SEP as u32).unwrap();
            paths.insert(s.replace(sep, " "));
        }
        Ok(paths)
    }

    /// Returns a `String` summary of the tokens this analyzer produces on
    /// this text.
    ///
    /// # Errors
    ///
    /// What the analyzer throws.
    pub fn to_string(analyzer: &dyn Analyzer, text: &str) -> Result<String> {
        Self::with_stream(analyzer, "field", text, |ts| {
            let mut b = String::new();
            let source = ts.attribute_source();
            let term_att = Att::get(source, char_term_id());
            let pos_inc_att = Att::get(source, pos_inc_id());
            let pos_length_att = Att::get(source, pos_len_id());
            let offset_att = Att::get(source, offset_id());
            Assert::assert_true(offset_att.is_some());
            let offset_att = offset_att.unwrap();
            ts.reset()?;
            let mut pos = -1;
            while ts.increment_token()? {
                pos += pos_inc_att
                    .as_ref()
                    .expect("java.lang.NullPointerException")
                    .pos_inc()
                    .get_position_increment();
                b.push_str(
                    &term_att
                        .as_ref()
                        .map_or_else(|| "null".to_string(), Att::term_string),
                );
                b.push_str(" at pos=");
                b.push_str(&pos.to_string());
                if let Some(pl) = &pos_length_att {
                    b.push_str(" to pos=");
                    b.push_str(&(pos + pl.pos_len().get_position_length()).to_string());
                }
                b.push_str(" offsets=");
                let (s, e) = {
                    let o = offset_att.offset();
                    (o.start_offset(), o.end_offset())
                };
                b.push_str(&s.to_string());
                b.push('-');
                b.push_str(&e.to_string());
                b.push('\n');
            }
            ts.end()?;
            ts.close()?;
            Ok(b)
        })
    }
}

/// `BaseTokenStreamTestCase.AnalysisThread`: runs the same
/// `checkRandomData` as the calling thread once the starting gun fires.
struct AnalysisThread {
    seed: i64,
    latch: Arc<(Mutex<bool>, Condvar)>,
    a: Arc<dyn Analyzer>,
    iterations: i32,
    max_word_length: i32,
    use_char_filter: bool,
    simple: bool,
    graph_offsets_are_correct: bool,
    iw: Option<Arc<RandomIndexWriter>>,
    // NOTE: not volatile because we don't want the tests to
    // add memory barriers (ie alter how threads
    // interact)... so this is just "best effort":
    failed: Arc<AtomicBool>,
}

impl AnalysisThread {
    fn run(self) {
        struct Finally<'a>(&'a AtomicBool, bool);
        impl Drop for Finally<'_> {
            fn drop(&mut self) {
                self.0.store(!self.1, Ordering::Relaxed);
            }
        }
        let mut success = Finally(&self.failed, false);
        {
            let (lock, cvar) = &*self.latch;
            let mut fired = lock.lock().unwrap_or_else(|e| e.into_inner());
            while !*fired {
                fired = cvar.wait(fired).unwrap_or_else(|e| e.into_inner());
            }
        }
        // see the part in checkRandomData where it replays the same text again
        // to verify reproducability/reuse: hopefully this would catch thread hazards.
        let result = BaseTokenStreamTestCase::check_random_data_inner(
            &(Arc::new(JdkRandom::new(self.seed)) as Arc<dyn Random>),
            &*self.a,
            self.iterations,
            self.max_word_length,
            self.use_char_filter,
            self.simple,
            self.graph_offsets_are_correct,
            self.iw.as_deref(),
        );
        match result {
            Ok(()) => success.1 = true,
            Err(e) => rethrow(&e),
        }
    }
}
