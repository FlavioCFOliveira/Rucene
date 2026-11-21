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

//! Port of `TestMultiAnalyzerQPHelper.java`: the `StandardQueryParser`'s
//! ability to deal with Analyzers that return more than one token per
//! position or that return tokens with a position increment &gt; 1.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::analysis::{Analyzer, TokenFilter, TokenFilterLogic, TokenStream};
use rucene::error::{LuceneError, Result};
use rucene::search::Query;
use rucene::util::attribute::AttributeSource;
use rucene_queryparser::flexible::standard::common_query_parser_configuration::CommonQueryParserConfiguration;
use rucene_queryparser::flexible::standard::config::standard_query_config_handler::Operator;
use rucene_queryparser::flexible::standard::standard_query_parser::StandardQueryParser;
use rucene_test_framework::analysis::attributes::{
    char_term_id, offset_id, pos_inc_id, type_id, Att,
};
use rucene_test_framework::analysis::mock_tokenizer::WHITESPACE;
use rucene_test_framework::analysis::{components, shared, sink};

use crate::qp_support::query_parser_test_base::{mock_tokenizer, FnAnalyzer};
use crate::qp_support::search_support::{fixture_class, ftest};
use crate::qp_support::{assert_str_eq, jdk_default_hooks, to_s};

/// The instance of `TestMultiAnalyzerQPHelper`: `private int multiToken =
/// 0`, which the inner `TestFilter` reads and writes.
#[derive(Default)]
pub struct TestMultiAnalyzerQPHelper {
    multi_token: Arc<AtomicI32>,
}

/// `TestMultiAnalyzerQPHelper`.
pub fn class_def() -> Arc<Class> {
    let mut methods = jdk_default_hooks();
    methods.extend([
        ftest("testMultiAnalyzer", test_multi_analyzer),
        ftest("testPosIncrementAnalyzer", test_pos_increment_analyzer),
    ]);
    fixture_class::<TestMultiAnalyzerQPHelper>(
        "org.apache.lucene.queryparser.flexible.standard.TestMultiAnalyzerQPHelper",
        methods,
    )
}

/// `qp.parse(query, "").toString()`.
fn parsed(qp: &StandardQueryParser, query: &str) -> Result<String> {
    let q: Arc<dyn Query> = qp
        .parse(query, Some(""))?
        .ok_or_else(LuceneError::null_pointer)?;
    Ok(to_s(&*q))
}

fn test_multi_analyzer(t: &mut TestMultiAnalyzerQPHelper) -> Result<()> {
    let mut qp = StandardQueryParser::new();
    qp.set_analyzer(Some(multi_analyzer(&t.multi_token)))?;

    // trivial, no multiple tokens:
    assert_str_eq("foo", &parsed(&qp, "foo")?);
    assert_str_eq("foo", &parsed(&qp, "\"foo\"")?);
    assert_str_eq("foo foobar", &parsed(&qp, "foo foobar")?);
    assert_str_eq("\"foo foobar\"", &parsed(&qp, "\"foo foobar\"")?);
    assert_str_eq("\"foo foobar blah\"", &parsed(&qp, "\"foo foobar blah\"")?);

    // two tokens at the same position:
    assert_str_eq("(multi multi2) foo", &parsed(&qp, "multi foo")?);
    assert_str_eq("foo (multi multi2)", &parsed(&qp, "foo multi")?);
    assert_str_eq(
        "(multi multi2) (multi multi2)",
        &parsed(&qp, "multi multi")?,
    );
    assert_str_eq(
        "+(foo (multi multi2)) +(bar (multi multi2))",
        &parsed(&qp, "+(foo multi) +(bar multi)")?,
    );
    assert_str_eq(
        "+(foo (multi multi2)) field:\"bar (multi multi2)\"",
        &parsed(&qp, "+(foo multi) field:\"bar multi\"")?,
    );

    // phrases:
    assert_str_eq("\"(multi multi2) foo\"", &parsed(&qp, "\"multi foo\"")?);
    assert_str_eq("\"foo (multi multi2)\"", &parsed(&qp, "\"foo multi\"")?);
    assert_str_eq(
        "\"foo (multi multi2) foobar (multi multi2)\"",
        &parsed(&qp, "\"foo multi foobar multi\"")?,
    );

    // fields:
    assert_str_eq(
        "(field:multi field:multi2) field:foo",
        &parsed(&qp, "field:multi field:foo")?,
    );
    assert_str_eq(
        "field:\"(multi multi2) foo\"",
        &parsed(&qp, "field:\"multi foo\"")?,
    );

    // three tokens at one position:
    assert_str_eq("triplemulti multi3 multi2", &parsed(&qp, "triplemulti")?);
    assert_str_eq(
        "foo (triplemulti multi3 multi2) foobar",
        &parsed(&qp, "foo triplemulti foobar")?,
    );

    // phrase with non-default slop:
    assert_str_eq(
        "\"(multi multi2) foo\"~10",
        &parsed(&qp, "\"multi foo\"~10")?,
    );

    // phrase with non-default boost:
    assert_str_eq(
        "(\"(multi multi2) foo\")^2.0",
        &parsed(&qp, "\"multi foo\"^2")?,
    );

    // phrase after changing default slop
    qp.set_phrase_slop(99)?;
    assert_str_eq(
        "\"(multi multi2) foo\"~99 bar",
        &parsed(&qp, "\"multi foo\" bar")?,
    );
    assert_str_eq(
        "\"(multi multi2) foo\"~99 \"foo bar\"~2",
        &parsed(&qp, "\"multi foo\" \"foo bar\"~2")?,
    );
    qp.set_phrase_slop(0)?;

    // non-default operator:
    qp.set_default_operator(Some(Operator::And))?;
    assert_str_eq("+(multi multi2) +foo", &parsed(&qp, "multi foo")?);
    Ok(())
}

// public void testMultiAnalyzerWithSubclassOfQueryParser() throws
// ParseException {
// this test doesn't make sense when using the new QueryParser API
// (the Java method is commented out)

fn test_pos_increment_analyzer(_t: &mut TestMultiAnalyzerQPHelper) -> Result<()> {
    let mut qp = StandardQueryParser::new();
    qp.set_analyzer(Some(pos_increment_analyzer()))?;

    assert_str_eq("quick brown", &parsed(&qp, "the quick brown")?);
    assert_str_eq("\"? quick brown\"", &parsed(&qp, "\"the quick brown\"")?);
    assert_str_eq("quick brown fox", &parsed(&qp, "the quick brown fox")?);
    assert_str_eq(
        "\"? quick brown fox\"",
        &parsed(&qp, "\"the quick brown fox\"")?,
    );
    Ok(())
}

/// Expands "multi" to "multi" and "multi2", both at the same position, and
/// expands "triplemulti" to "triplemulti", "multi3", and "multi2".
fn multi_analyzer(multi_token: &Arc<AtomicI32>) -> Arc<dyn Analyzer> {
    let multi_token = Arc::clone(multi_token);
    FnAnalyzer::new(
        "org.apache.lucene.queryparser.flexible.standard.TestMultiAnalyzerQPHelper$MultiAnalyzer",
        move |_| {
            let result = mock_tokenizer(&WHITESPACE, true)?;
            let filter = test_filter(shared(&result), Arc::clone(&multi_token))?;
            Ok(components(result, sink(filter)))
        },
    )
    .arc()
}

/// The logic of `TestMultiAnalyzerQPHelper.TestFilter`.
#[derive(Debug)]
struct TestFilterLogic {
    multi_token: Arc<AtomicI32>,
    prev_type: Option<String>,
    prev_start_offset: i32,
    prev_end_offset: i32,
    term_att: Att,
    pos_incr_att: Att,
    offset_att: Att,
    type_att: Att,
}

impl TokenFilterLogic for TestFilterLogic {
    fn increment_token(
        &mut self,
        _source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        let multi_token = self.multi_token.load(Ordering::Relaxed);
        if multi_token > 0 {
            self.term_att.set_term(&format!("multi{}", multi_token + 1));
            self.offset_att
                .offset_mut()
                .set_offset(self.prev_start_offset, self.prev_end_offset);
            // `typeAtt.setType(prevType)`: `prevType` was read from the
            // stream before any synonym is injected, so it is never `null`
            // here.
            self.type_att
                .type_att_mut()
                .set_type(self.prev_type.clone().unwrap_or_default());
            self.pos_incr_att.pos_inc_mut().set_position_increment(0);
            self.multi_token.store(multi_token - 1, Ordering::Relaxed);
            Ok(true)
        } else {
            let next = input.increment_token()?;
            if !next {
                return Ok(false);
            }
            self.prev_type = Some(self.type_att.type_att().type_value().to_string());
            self.prev_start_offset = self.offset_att.offset().start_offset();
            self.prev_end_offset = self.offset_att.offset().end_offset();
            let text = self.term_att.term_string();
            if text == "triplemulti" {
                self.multi_token.store(2, Ordering::Relaxed);
                Ok(true)
            } else if text == "multi" {
                self.multi_token.store(1, Ordering::Relaxed);
                Ok(true)
            } else {
                Ok(true)
            }
        }
    }

    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.prev_type = None;
        self.prev_start_offset = 0;
        self.prev_end_offset = 0;
        Ok(())
    }
}

fn test_filter(
    mut input: Box<dyn TokenStream>,
    multi_token: Arc<AtomicI32>,
) -> Result<TokenFilter<TestFilterLogic>> {
    let source = input.attribute_source_mut();
    let term_att = Att::add(source, char_term_id())?;
    let pos_incr_att = Att::add(source, pos_inc_id())?;
    let offset_att = Att::add(source, offset_id())?;
    let type_att = Att::add(source, type_id())?;
    Ok(TokenFilter::new(
        input,
        TestFilterLogic {
            multi_token,
            prev_type: None,
            prev_start_offset: 0,
            prev_end_offset: 0,
            term_att,
            pos_incr_att,
            offset_att,
            type_att,
        },
    ))
}

/// Analyzes "the quick brown" as: quick(incr=2) brown(incr=1). Does not
/// work correctly for input other than "the quick brown ...".
fn pos_increment_analyzer() -> Arc<dyn Analyzer> {
    FnAnalyzer::new(
        "org.apache.lucene.queryparser.flexible.standard.TestMultiAnalyzerQPHelper$PosIncrementAnalyzer",
        |_| {
            let result = mock_tokenizer(&WHITESPACE, true)?;
            let mut input = shared(&result);
            let term_att = Att::add(input.attribute_source_mut(), char_term_id())?;
            let pos_incr_att = Att::add(input.attribute_source_mut(), pos_inc_id())?;
            let filter = TokenFilter::new(
                input,
                TestPosIncrementFilterLogic {
                    term_att,
                    pos_incr_att,
                },
            );
            Ok(components(result, sink(filter)))
        },
    )
    .arc()
}

/// The logic of `TestMultiAnalyzerQPHelper.TestPosIncrementFilter`.
#[derive(Debug)]
struct TestPosIncrementFilterLogic {
    term_att: Att,
    pos_incr_att: Att,
}

impl TokenFilterLogic for TestPosIncrementFilterLogic {
    fn increment_token(
        &mut self,
        _source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        while input.increment_token()? {
            let term = self.term_att.term_string();
            if term == "the" {
                // stopword, do nothing
            } else if term == "quick" {
                self.pos_incr_att.pos_inc_mut().set_position_increment(2);
                return Ok(true);
            } else {
                self.pos_incr_att.pos_inc_mut().set_position_increment(1);
                return Ok(true);
            }
        }
        Ok(false)
    }
}
