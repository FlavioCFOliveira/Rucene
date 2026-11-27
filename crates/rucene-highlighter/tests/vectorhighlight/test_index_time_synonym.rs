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

//! Port of `TestIndexTimeSynonym.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::tokenattributes::CharTermAttribute;
use rucene::analysis::{
    Analyzer, AnalyzerState, CharReader, GlobalReuseStrategy, TokenStream, TokenStreamComponents,
    Tokenizer, TokenizerLogic,
};
use rucene::error::Result;
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query::Builder as BooleanQueryBuilder;
use rucene::search::query::Query;
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::attribute::AttributeSource;
use rucene_highlighter::vectorhighlight::field_phrase_list::FieldPhraseList;
use rucene_highlighter::vectorhighlight::field_query::FieldQuery;
use rucene_highlighter::vectorhighlight::field_term_stack::FieldTermStack;
use rucene_test_framework::analysis::attributes::{char_term_id, offset_id, pos_inc_id, Att};
use rucene_test_framework::analysis::token::Token;
use rucene_test_framework::analysis::tokenizer_components;

use super::abstract_test_case::{make1dmf_index_with, pq_f, tq, vh_class, Fixture, F};
use crate::highlighter_support::search_support::ftest;

/// `TestIndexTimeSynonym`.
pub fn class_def() -> Arc<Class> {
    vh_class(
        "org.apache.lucene.search.vectorhighlight.TestIndexTimeSynonym",
        vec![
            ftest(
                "testFieldTermStackIndex1wSearch1term",
                test_fts_1w_search_1term,
            ),
            ftest(
                "testFieldTermStackIndex1wSearch2terms",
                test_fts_1w_search_2terms,
            ),
            ftest(
                "testFieldTermStackIndex1w2wSearch1term",
                test_fts_1w2w_search_1term,
            ),
            ftest(
                "testFieldTermStackIndex1w2wSearch1phrase",
                test_fts_1w2w_search_1phrase,
            ),
            ftest(
                "testFieldTermStackIndex1w2wSearch1partial",
                test_fts_1w2w_search_1partial,
            ),
            ftest(
                "testFieldTermStackIndex1w2wSearch1term1phrase",
                test_fts_1w2w_search_1term1phrase,
            ),
            ftest(
                "testFieldTermStackIndex2w1wSearch1term",
                test_fts_2w1w_search_1term,
            ),
            ftest(
                "testFieldTermStackIndex2w1wSearch1phrase",
                test_fts_2w1w_search_1phrase,
            ),
            ftest(
                "testFieldTermStackIndex2w1wSearch1partial",
                test_fts_2w1w_search_1partial,
            ),
            ftest(
                "testFieldTermStackIndex2w1wSearch1term1phrase",
                test_fts_2w1w_search_1term1phrase,
            ),
            ftest(
                "testFieldPhraseListIndex1w2wSearch1phrase",
                test_fpl_1w2w_search_1phrase,
            ),
            ftest(
                "testFieldPhraseListIndex1w2wSearch1partial",
                test_fpl_1w2w_search_1partial,
            ),
            ftest(
                "testFieldPhraseListIndex1w2wSearch1term1phrase",
                test_fpl_1w2w_search_1term1phrase,
            ),
            ftest(
                "testFieldPhraseListIndex2w1wSearch1term",
                test_fpl_2w1w_search_1term,
            ),
            ftest(
                "testFieldPhraseListIndex2w1wSearch1phrase",
                test_fpl_2w1w_search_1phrase,
            ),
            ftest(
                "testFieldPhraseListIndex2w1wSearch1partial",
                test_fpl_2w1w_search_1partial,
            ),
            ftest(
                "testFieldPhraseListIndex2w1wSearch1term1phrase",
                test_fpl_2w1w_search_1term1phrase,
            ),
        ],
    )
}

fn fq(query: Arc<dyn Query>) -> Result<FieldQuery> {
    FieldQuery::without_reader(&query, true, true)
}

fn stack(f: &Fixture, query: Arc<dyn Query>) -> Result<FieldTermStack> {
    let fq = fq(query)?;
    FieldTermStack::new(f.reader(), 0, F, &fq)
}

fn assert_stack(f: &Fixture, query: Arc<dyn Query>, expected: &[&str]) -> Result<()> {
    let mut stack = stack(f, query)?;
    Assert::assert_equals(expected.len(), stack.term_list().len());
    for e in expected {
        Assert::assert_equals(e.to_string(), stack.pop().expect("a term info").to_string());
    }
    Ok(())
}

/// `pc OR "personal computer"`.
fn pc_or_phrase() -> Result<Arc<dyn Query>> {
    let mut bq = BooleanQueryBuilder::new();
    bq.add(tq("pc"), Occur::SHOULD)?;
    bq.add(pq_f(&["personal", "computer"]), Occur::SHOULD)?;
    Ok(Arc::new(bq.build()))
}

/// The stack of `pc OR "personal computer"`: the first term info chains
/// `first` and `next`, the second is `last`.
fn assert_chained(f: &Fixture, first: &str, next: &str, last: &str) -> Result<()> {
    let mut stack = stack(f, pc_or_phrase()?)?;
    Assert::assert_equals(2, stack.term_list().len());
    let ti = stack.pop().expect("a term info");
    Assert::assert_equals(first.to_string(), ti.to_string());
    Assert::assert_equals(next.to_string(), ti.get_next().to_string());
    Assert::assert_true(ti.same(&ti.get_next().get_next()));
    Assert::assert_equals(
        last.to_string(),
        stack.pop().expect("a term info").to_string(),
    );
    Ok(())
}

fn test_fts_1w_search_1term(f: &mut Fixture) -> Result<()> {
    make_index_1w(f)?;
    assert_stack(f, tq("Mac"), &["Mac(11,20,3)"])
}

fn test_fts_1w_search_2terms(f: &mut Fixture) -> Result<()> {
    make_index_1w(f)?;

    let mut bq = BooleanQueryBuilder::new();
    bq.add(tq("Mac"), Occur::SHOULD)?;
    bq.add(tq("MacBook"), Occur::SHOULD)?;
    let mut stack = stack(f, Arc::new(bq.build()))?;
    Assert::assert_equals(1, stack.term_list().len());
    let ti = stack.pop().expect("a term info");
    Assert::assert_equals("Mac(11,20,3)".to_string(), ti.to_string());
    Assert::assert_equals("MacBook(11,20,3)".to_string(), ti.get_next().to_string());
    Assert::assert_true(ti.same(&ti.get_next().get_next()));
    Ok(())
}

fn test_fts_1w2w_search_1term(f: &mut Fixture) -> Result<()> {
    make_index_1w2w(f)?;
    assert_stack(f, tq("pc"), &["pc(3,5,1)"])
}

fn test_fts_1w2w_search_1phrase(f: &mut Fixture) -> Result<()> {
    make_index_1w2w(f)?;
    assert_stack(
        f,
        pq_f(&["personal", "computer"]),
        &["personal(3,5,1)", "computer(3,5,2)"],
    )
}

fn test_fts_1w2w_search_1partial(f: &mut Fixture) -> Result<()> {
    make_index_1w2w(f)?;
    assert_stack(f, tq("computer"), &["computer(3,5,2)"])
}

fn test_fts_1w2w_search_1term1phrase(f: &mut Fixture) -> Result<()> {
    make_index_1w2w(f)?;
    assert_chained(f, "pc(3,5,1)", "personal(3,5,1)", "computer(3,5,2)")
}

fn test_fts_2w1w_search_1term(f: &mut Fixture) -> Result<()> {
    make_index_2w1w(f)?;
    assert_stack(f, tq("pc"), &["pc(3,20,1)"])
}

fn test_fts_2w1w_search_1phrase(f: &mut Fixture) -> Result<()> {
    make_index_2w1w(f)?;
    assert_stack(
        f,
        pq_f(&["personal", "computer"]),
        &["personal(3,20,1)", "computer(3,20,2)"],
    )
}

fn test_fts_2w1w_search_1partial(f: &mut Fixture) -> Result<()> {
    make_index_2w1w(f)?;
    assert_stack(f, tq("computer"), &["computer(3,20,2)"])
}

fn test_fts_2w1w_search_1term1phrase(f: &mut Fixture) -> Result<()> {
    make_index_2w1w(f)?;
    assert_chained(f, "pc(3,20,1)", "personal(3,20,1)", "computer(3,20,2)")
}

fn fpl(f: &Fixture, query: Arc<dyn Query>) -> Result<FieldPhraseList> {
    let fq = fq(query)?;
    let mut stack = FieldTermStack::new(f.reader(), 0, F, &fq)?;
    Ok(FieldPhraseList::new(&mut stack, &fq))
}

/// One phrase: its `toString()` (`exact`) or a substring of it past index 0
/// (`contains`), and its offsets.
fn assert_one_phrase(fpl: &FieldPhraseList, text: &str, exact: bool, start: i32, end: i32) {
    Assert::assert_equals(1, fpl.get_phrase_list().len());
    let s = fpl.get_phrase_list()[0].to_string();
    if exact {
        Assert::assert_equals(text.to_string(), s);
    } else {
        Assert::assert_true(s.find(text).is_some_and(|i| i > 0));
    }
    Assert::assert_equals(start, fpl.get_phrase_list()[0].get_start_offset());
    Assert::assert_equals(end, fpl.get_phrase_list()[0].get_end_offset());
}

fn test_fpl_1w2w_search_1phrase(f: &mut Fixture) -> Result<()> {
    make_index_1w2w(f)?;
    let fpl = fpl(f, pq_f(&["personal", "computer"]))?;
    assert_one_phrase(&fpl, "personalcomputer(1.0)((3,5))", true, 3, 5);
    Ok(())
}

fn test_fpl_1w2w_search_1partial(f: &mut Fixture) -> Result<()> {
    make_index_1w2w(f)?;
    let fpl = fpl(f, tq("computer"))?;
    assert_one_phrase(&fpl, "computer(1.0)((3,5))", true, 3, 5);
    Ok(())
}

fn test_fpl_1w2w_search_1term1phrase(f: &mut Fixture) -> Result<()> {
    make_index_1w2w(f)?;
    let fpl = fpl(f, pc_or_phrase()?)?;
    assert_one_phrase(&fpl, "(1.0)((3,5))", false, 3, 5);
    Ok(())
}

fn test_fpl_2w1w_search_1term(f: &mut Fixture) -> Result<()> {
    make_index_2w1w(f)?;
    let fpl = fpl(f, tq("pc"))?;
    assert_one_phrase(&fpl, "pc(1.0)((3,20))", true, 3, 20);
    Ok(())
}

fn test_fpl_2w1w_search_1phrase(f: &mut Fixture) -> Result<()> {
    make_index_2w1w(f)?;
    let fpl = fpl(f, pq_f(&["personal", "computer"]))?;
    assert_one_phrase(&fpl, "personalcomputer(1.0)((3,20))", true, 3, 20);
    Ok(())
}

fn test_fpl_2w1w_search_1partial(f: &mut Fixture) -> Result<()> {
    make_index_2w1w(f)?;
    let fpl = fpl(f, tq("computer"))?;
    assert_one_phrase(&fpl, "computer(1.0)((3,20))", true, 3, 20);
    Ok(())
}

fn test_fpl_2w1w_search_1term1phrase(f: &mut Fixture) -> Result<()> {
    make_index_2w1w(f)?;
    let fpl = fpl(f, pc_or_phrase()?)?;
    assert_one_phrase(&fpl, "(1.0)((3,20))", false, 3, 20);
    Ok(())
}

fn make_index_1w(f: &mut Fixture) -> Result<()> {
    //           11111111112
    // 012345678901234567890
    // I'll buy a Macintosh
    //            Mac
    //            MacBook
    // 0    1   2 3
    make_synonym_index(
        f,
        "I'll buy a Macintosh",
        vec![
            t("I'll", 0, 4),
            t("buy", 5, 8),
            t("a", 9, 10),
            t("Macintosh", 11, 20),
            t_inc("Mac", 11, 20, 0),
            t_inc("MacBook", 11, 20, 0),
        ],
    )
}

fn make_index_1w2w(f: &mut Fixture) -> Result<()> {
    //           1111111
    // 01234567890123456
    // My pc was broken
    //    personal computer
    // 0  1  2   3
    make_synonym_index(
        f,
        "My pc was broken",
        vec![
            t("My", 0, 2),
            t("pc", 3, 5),
            t_inc("personal", 3, 5, 0),
            t("computer", 3, 5),
            t("was", 6, 9),
            t("broken", 10, 16),
        ],
    )
}

fn make_index_2w1w(f: &mut Fixture) -> Result<()> {
    //           1111111111222222222233
    // 01234567890123456789012345678901
    // My personal computer was broken
    //    pc
    // 0  1        2        3   4
    make_synonym_index(
        f,
        "My personal computer was broken",
        vec![
            t("My", 0, 2),
            t("personal", 3, 20),
            t_inc("pc", 3, 20, 0),
            t("computer", 3, 20),
            t("was", 21, 24),
            t("broken", 25, 31),
        ],
    )
}

fn make_synonym_index(f: &mut Fixture, value: &str, tokens: Vec<Tok>) -> Result<()> {
    let analyzer: Arc<dyn Analyzer> = Arc::new(TokenArrayAnalyzer {
        state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
        tokens: Arc::new(tokens),
    });
    make1dmf_index_with(f, analyzer, &[value])
}

/// A `Token` of `t(...)`: its term, offsets and position increment (every
/// other field keeps its default).
#[derive(Debug, Clone)]
struct Tok {
    text: String,
    start_offset: i32,
    end_offset: i32,
    position_increment: i32,
}

/// `t(text, startOffset, endOffset)`.
fn t(text: &str, start_offset: i32, end_offset: i32) -> Tok {
    t_inc(text, start_offset, end_offset, 1)
}

/// `t(text, startOffset, endOffset, positionIncrement)`.
fn t_inc(text: &str, start_offset: i32, end_offset: i32, position_increment: i32) -> Tok {
    Tok {
        text: text.to_string(),
        start_offset,
        end_offset,
        position_increment,
    }
}

/// `public static final class TokenArrayAnalyzer extends Analyzer`.
#[derive(Debug)]
struct TokenArrayAnalyzer {
    state: AnalyzerState,
    tokens: Arc<Vec<Tok>>,
}

impl Analyzer for TokenArrayAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.search.vectorhighlight.TestIndexTimeSynonym$TokenArrayAnalyzer"
    }

    fn create_components(&self, _field_name: Option<&str>) -> Result<TokenStreamComponents> {
        let mut ts = Tokenizer::with_factory(
            Token::token_attribute_factory(),
            TokenArrayLogic {
                tokens: Arc::clone(&self.tokens),
                p: 0,
            },
        );
        // `final AttributeImpl reusableToken = (AttributeImpl)
        // addAttribute(CharTermAttribute.class);`
        ts.attribute_source_mut()
            .add_attribute_interface::<dyn CharTermAttribute>()?;
        Ok(tokenizer_components(Arc::new(AtomicRefCell::new(ts))))
    }
}

/// The anonymous `Tokenizer` of `TokenArrayAnalyzer.createComponents`:
/// `tokens[p++].copyTo(reusableToken)`.
#[derive(Debug)]
struct TokenArrayLogic {
    tokens: Arc<Vec<Tok>>,
    p: usize,
}

impl TokenizerLogic for TokenArrayLogic {
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        _reader: &mut dyn CharReader,
    ) -> Result<bool> {
        if self.p >= self.tokens.len() {
            return Ok(false);
        }
        source.clear_attributes();
        let token = &self.tokens[self.p];
        self.p += 1;
        // `copyTo` into the `Token` the factory made: the copied token's term,
        // offsets and position increment (its other fields are the defaults
        // `clearAttributes` restored)
        Att::add(source, char_term_id())?.set_term(&token.text);
        Att::add(source, offset_id())?
            .offset_mut()
            .set_offset(token.start_offset, token.end_offset);
        Att::add(source, pos_inc_id())?
            .pos_inc_mut()
            .set_position_increment(token.position_increment);
        Ok(true)
    }

    fn reset(&mut self, _source: &mut AttributeSource, _reader: &mut dyn CharReader) -> Result<()> {
        self.p = 0;
        Ok(())
    }
}
