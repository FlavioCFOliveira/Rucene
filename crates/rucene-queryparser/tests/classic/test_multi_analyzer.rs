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

//! Port of `TestMultiAnalyzer.java`: QueryParser's ability to deal with
//! Analyzers that return more than one token per position or that return
//! tokens with a position increment &gt; 1.

use std::any::Any;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor};
use rucene::analysis::{Analyzer, TokenFilter, TokenFilterLogic, TokenStream};
use rucene::error::Result;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::Query;
use rucene::util::attribute::AttributeSource;
use rucene_queryparser::classic::query_parser::QueryParser;
use rucene_queryparser::classic::query_parser_base::{self, QueryParserBaseOps, AND_OPERATOR};
use rucene_queryparser::flexible::standard::common_query_parser_configuration::CommonQueryParserConfiguration;
use rucene_test_framework::analysis::attributes::{
    char_term_id, offset_id, pos_inc_id, type_id, Att,
};
use rucene_test_framework::analysis::base_token_stream_test_case::BaseTokenStreamTestCase;
use rucene_test_framework::analysis::mock_tokenizer::WHITESPACE;
use rucene_test_framework::analysis::{components, shared, sink};

use super::test_query_parser::{classic_base_required, classic_subclass};
use crate::qp_support::jdk_default_hooks;
use crate::qp_support::query_parser_test_base::{mock_tokenizer, FnAnalyzer};
use crate::qp_support::search_support::ftest;
use crate::qp_support::{assert_str_eq, to_s};

/// The instance of `TestMultiAnalyzer`: `private int multiToken = 0`, which
/// the inner `TestFilter` reads and writes.
#[derive(Default)]
pub struct TestMultiAnalyzer {
    multi_token: Arc<AtomicI32>,
}

/// `TestMultiAnalyzer`.
pub fn class_def() -> Arc<Class> {
    let mut b = Class::builder("org.apache.lucene.queryparser.classic.TestMultiAnalyzer")
        .extends(&BaseTokenStreamTestCase::class())
        .constructor(Constructor::no_args(TestMultiAnalyzer::default));
    for m in jdk_default_hooks() {
        b = b.method(m);
    }
    b.method(ftest("testMultiAnalyzer", test_multi_analyzer))
        .method(ftest(
            "testMultiAnalyzerWithSubclassOfQueryParser",
            test_multi_analyzer_with_subclass_of_query_parser,
        ))
        .method(ftest(
            "testPosIncrementAnalyzer",
            test_pos_increment_analyzer,
        ))
        .build()
}

fn test_multi_analyzer(t: &mut TestMultiAnalyzer) -> Result<()> {
    let mut qp = QueryParser::new(Some(""), Some(multi_analyzer(&t.multi_token)))?;

    // trivial, no multiple tokens:
    assert_str_eq("foo", &to_s(&*qp.parse("foo")?));
    assert_str_eq("foo", &to_s(&*qp.parse("\"foo\"")?));
    assert_str_eq("foo foobar", &to_s(&*qp.parse("foo foobar")?));
    assert_str_eq("\"foo foobar\"", &to_s(&*qp.parse("\"foo foobar\"")?));
    assert_str_eq(
        "\"foo foobar blah\"",
        &to_s(&*qp.parse("\"foo foobar blah\"")?),
    );

    // two tokens at the same position:
    assert_str_eq("Synonym(multi multi2) foo", &to_s(&*qp.parse("multi foo")?));
    assert_str_eq("foo Synonym(multi multi2)", &to_s(&*qp.parse("foo multi")?));
    assert_str_eq(
        "Synonym(multi multi2) Synonym(multi multi2)",
        &to_s(&*qp.parse("multi multi")?),
    );
    assert_str_eq(
        "+(foo Synonym(multi multi2)) +(bar Synonym(multi multi2))",
        &to_s(&*qp.parse("+(foo multi) +(bar multi)")?),
    );
    assert_str_eq(
        "+(foo Synonym(multi multi2)) field:\"bar (multi multi2)\"",
        &to_s(&*qp.parse("+(foo multi) field:\"bar multi\"")?),
    );

    // phrases:
    assert_str_eq(
        "\"(multi multi2) foo\"",
        &to_s(&*qp.parse("\"multi foo\"")?),
    );
    assert_str_eq(
        "\"foo (multi multi2)\"",
        &to_s(&*qp.parse("\"foo multi\"")?),
    );
    assert_str_eq(
        "\"foo (multi multi2) foobar (multi multi2)\"",
        &to_s(&*qp.parse("\"foo multi foobar multi\"")?),
    );

    // fields:
    assert_str_eq(
        "Synonym(field:multi field:multi2) field:foo",
        &to_s(&*qp.parse("field:multi field:foo")?),
    );
    assert_str_eq(
        "field:\"(multi multi2) foo\"",
        &to_s(&*qp.parse("field:\"multi foo\"")?),
    );

    // three tokens at one position:
    assert_str_eq(
        "Synonym(multi2 multi3 triplemulti)",
        &to_s(&*qp.parse("triplemulti")?),
    );
    assert_str_eq(
        "foo Synonym(multi2 multi3 triplemulti) foobar",
        &to_s(&*qp.parse("foo triplemulti foobar")?),
    );

    // phrase with non-default slop:
    assert_str_eq(
        "\"(multi multi2) foo\"~10",
        &to_s(&*qp.parse("\"multi foo\"~10")?),
    );

    // phrase with non-default boost:
    assert_str_eq(
        "(\"(multi multi2) foo\")^2.0",
        &to_s(&*qp.parse("\"multi foo\"^2")?),
    );

    // phrase after changing default slop
    qp.set_phrase_slop(99)?;
    assert_str_eq(
        "\"(multi multi2) foo\"~99 bar",
        &to_s(&*qp.parse("\"multi foo\" bar")?),
    );
    assert_str_eq(
        "\"(multi multi2) foo\"~99 \"foo bar\"~2",
        &to_s(&*qp.parse("\"multi foo\" \"foo bar\"~2")?),
    );
    qp.set_phrase_slop(0)?;

    // non-default operator:
    qp.set_default_operator(AND_OPERATOR);
    assert_str_eq(
        "+Synonym(multi multi2) +foo",
        &to_s(&*qp.parse("multi foo")?),
    );
    Ok(())
}

fn test_multi_analyzer_with_subclass_of_query_parser(t: &mut TestMultiAnalyzer) -> Result<()> {
    let mut qp = DumbQueryParser::new("", multi_analyzer(&t.multi_token))?;
    qp.set_phrase_slop(99)?; // modified default slop

    // direct call to (super's) getFieldQuery to demonstrate differnce
    // between phrase and multiphrase with modified default slop
    assert_str_eq(
        "\"foo bar\"~99",
        &to_s(&*non_null(qp.get_super_field_query(
            Some(""),
            "foo bar",
            true,
        )?)),
    );
    assert_str_eq(
        "\"(multi multi2) bar\"~99",
        &to_s(&*non_null(qp.get_super_field_query(
            Some(""),
            "multi bar",
            true,
        )?)),
    );

    // ask sublcass to parse phrase with modified default slop
    assert_str_eq(
        "\"(multi multi2) foo\"~99 bar",
        &to_s(&*qp.parse("\"multi foo\" bar")?),
    );
    Ok(())
}

fn test_pos_increment_analyzer(_t: &mut TestMultiAnalyzer) -> Result<()> {
    let mut qp = QueryParser::new(Some(""), Some(pos_increment_analyzer()))?;
    assert_str_eq("quick brown", &to_s(&*qp.parse("the quick brown")?));
    assert_str_eq("quick brown fox", &to_s(&*qp.parse("the quick brown fox")?));
    Ok(())
}

/// `.toString()` of a query Java dereferences: a `null` throws
/// `NullPointerException`.
fn non_null(q: Option<Arc<dyn Query>>) -> Arc<dyn Query> {
    q.unwrap_or_else(|| {
        randomizedtesting::jdk::exception::throw_without_message("java.lang.NullPointerException")
    })
}

/// Expands "multi" to "multi" and "multi2", both at the same position, and
/// expands "triplemulti" to "triplemulti", "multi3", and "multi2".
fn multi_analyzer(multi_token: &Arc<AtomicI32>) -> Arc<dyn Analyzer> {
    let multi_token = Arc::clone(multi_token);
    FnAnalyzer::new(
        "org.apache.lucene.queryparser.classic.TestMultiAnalyzer$MultiAnalyzer",
        move |_| {
            let result = mock_tokenizer(&WHITESPACE, true)?;
            let filter = test_filter(shared(&result), Arc::clone(&multi_token))?;
            Ok(components(result, sink(filter)))
        },
    )
    .arc()
}

/// The logic of `TestMultiAnalyzer.TestFilter`.
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
        "org.apache.lucene.queryparser.classic.TestMultiAnalyzer$PosIncrementAnalyzer",
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

/// The logic of `TestMultiAnalyzer.TestPosIncrementFilter`.
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

/// a very simple subclass of QueryParser
struct DumbQueryParser {
    parser: QueryParser,
}

impl DumbQueryParser {
    fn new(f: &str, a: Arc<dyn Analyzer>) -> Result<Self> {
        Ok(DumbQueryParser {
            parser: QueryParser::new(Some(f), Some(a))?,
        })
    }

    /// expose super's version
    fn get_super_field_query(
        &mut self,
        f: Option<&str>,
        t: &str,
        quoted: bool,
    ) -> Result<Option<Arc<dyn Query>>> {
        query_parser_base::get_field_query(self, f, t, quoted)
    }
}

classic_subclass!(DumbQueryParser);

impl QueryParserBaseOps for DumbQueryParser {
    classic_base_required!();

    /// wrap super's version
    fn get_field_query(
        &mut self,
        f: Option<&str>,
        t: &str,
        quoted: bool,
    ) -> Result<Option<Arc<dyn Query>>> {
        let q = non_null(self.get_super_field_query(f, t, quoted)?);
        Ok(Some(Arc::new(DumbQueryWrapper { q })))
    }
}

/// A very simple wrapper to prevent instanceof checks but uses the toString
/// of the query it wraps.
#[derive(Debug)]
struct DumbQueryWrapper {
    q: Arc<dyn Query>,
}

impl Query for DumbQueryWrapper {
    fn to_query_string(&self, f: &str) -> String {
        self.q.to_query_string(f)
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        self.q.visit(visitor);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        self.same_class_as(other)
            && other
                .as_any()
                .downcast_ref::<DumbQueryWrapper>()
                .is_some_and(|o| self.q.query_eq(&*o.q))
    }

    fn query_hash(&self) -> i32 {
        self.class_hash() & self.q.query_hash()
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.queryparser.classic.TestMultiAnalyzer$DumbQueryWrapper"
    }
}
