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

//! Port of `TestSimpleFragListBuilder.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::Term;
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query::Builder as BooleanQueryBuilder;
use rucene::search::phrase_query::PhraseQuery;
use rucene::search::query::Query;
use rucene::search::TermQuery;
use rucene_highlighter::vectorhighlight::field_frag_list::FieldFragList;
use rucene_highlighter::vectorhighlight::field_phrase_list::FieldPhraseList;
use rucene_highlighter::vectorhighlight::field_query::FieldQuery;
use rucene_highlighter::vectorhighlight::field_term_stack::FieldTermStack;
use rucene_highlighter::vectorhighlight::frag_list_builder::FragListBuilder;
use rucene_highlighter::vectorhighlight::simple_frag_list_builder::SimpleFragListBuilder;
use rucene_test_framework::util::java_exceptions::expect_throws;

use super::abstract_test_case::{
    make1d1f_index, make_index_long_mv, make_index_long_mvb, make_index_short_mv, pq_f, tq,
    vh_class, Fixture, F,
};
use crate::highlighter_support::core_support::verbose;
use crate::highlighter_support::search_support::ftest;

/// `TestSimpleFragListBuilder`.
pub fn class_def() -> Arc<Class> {
    vh_class(
        "org.apache.lucene.search.vectorhighlight.TestSimpleFragListBuilder",
        vec![
            ftest("testNullFieldFragList", test_null_field_frag_list),
            ftest("testTooSmallFragSize", test_too_small_frag_size),
            ftest(
                "testSmallerFragSizeThanTermQuery",
                test_smaller_frag_size_than_term_query,
            ),
            ftest(
                "testSmallerFragSizeThanPhraseQuery",
                test_smaller_frag_size_than_phrase_query,
            ),
            ftest("test1TermIndex", test_1_term_index),
            ftest("test2TermsIndex1Frag", test_2_terms_index_1_frag),
            ftest("test2TermsIndex2Frags", test_2_terms_index_2_frags),
            ftest("test2TermsQuery", test_2_terms_query),
            ftest("testPhraseQuery", test_phrase_query),
            ftest("testPhraseQuerySlop", test_phrase_query_slop),
            ftest("test1PhraseShortMV", test_1_phrase_short_mv),
            ftest("test1PhraseLongMV", test_1_phrase_long_mv),
            ftest("test1PhraseLongMVB", test_1_phrase_long_mvb),
        ],
    )
}

fn tqf(text: &str) -> Arc<dyn Query> {
    Arc::new(TermQuery::new(Term::from_text(F, text)))
}

fn assert_infos(ffl: &dyn FieldFragList, expected: &[&str]) {
    Assert::assert_equals(expected.len(), ffl.get_frag_infos().len());
    for (i, e) in expected.iter().enumerate() {
        Assert::assert_equals(e.to_string(), ffl.get_frag_infos()[i].to_string());
    }
}

fn test_null_field_frag_list(f: &mut Fixture) -> Result<()> {
    let sflb = SimpleFragListBuilder::new();
    let ffl = sflb.create_field_frag_list(&fpl(f, tqf("a"), "b c d")?, 100)?;
    Assert::assert_equals(0, ffl.get_frag_infos().len());
    Ok(())
}

fn test_too_small_frag_size(f: &mut Fixture) -> Result<()> {
    expect_throws("java.lang.IllegalArgumentException", || {
        let sflb = SimpleFragListBuilder::new();
        sflb.create_field_frag_list(&fpl(f, tqf("a"), "b c d")?, sflb.min_frag_char_size() - 1)
    });
    Ok(())
}

fn test_smaller_frag_size_than_term_query(f: &mut Fixture) -> Result<()> {
    let sflb = SimpleFragListBuilder::new();
    let ffl = sflb.create_field_frag_list(
        &fpl(f, tqf("abcdefghijklmnopqrs"), "abcdefghijklmnopqrs")?,
        sflb.min_frag_char_size(),
    )?;
    assert_infos(&*ffl, &["subInfos=(abcdefghijklmnopqrs((0,19)))/1.0(0,19)"]);
    Ok(())
}

fn test_smaller_frag_size_than_phrase_query(f: &mut Fixture) -> Result<()> {
    let sflb = SimpleFragListBuilder::new();

    let phrase_query: Arc<dyn Query> =
        Arc::new(PhraseQuery::from_strings(F, &["abcdefgh", "jklmnopqrs"])?);

    let ffl = sflb.create_field_frag_list(
        &fpl(f, phrase_query, "abcdefgh   jklmnopqrs")?,
        sflb.min_frag_char_size(),
    )?;
    Assert::assert_equals(1, ffl.get_frag_infos().len());
    if verbose() {
        println!("{}", ffl.get_frag_infos()[0]);
    }
    assert_infos(&*ffl, &["subInfos=(abcdefghjklmnopqrs((0,21)))/1.0(0,21)"]);
    Ok(())
}

fn test_1_term_index(f: &mut Fixture) -> Result<()> {
    let sflb = SimpleFragListBuilder::new();
    let ffl = sflb.create_field_frag_list(&fpl(f, tqf("a"), "a")?, 100)?;
    assert_infos(&*ffl, &["subInfos=(a((0,1)))/1.0(0,100)"]);
    Ok(())
}

fn test_2_terms_index_1_frag(f: &mut Fixture) -> Result<()> {
    let sflb = SimpleFragListBuilder::new();
    let ffl = sflb.create_field_frag_list(&fpl(f, tqf("a"), "a a")?, 100)?;
    assert_infos(&*ffl, &["subInfos=(a((0,1))a((2,3)))/2.0(0,100)"]);

    let ffl = sflb.create_field_frag_list(&fpl(f, tqf("a"), "a b b b b b b b b a")?, 20)?;
    assert_infos(&*ffl, &["subInfos=(a((0,1))a((18,19)))/2.0(0,20)"]);

    let ffl = sflb.create_field_frag_list(&fpl(f, tqf("a"), "b b b b a b b b b a")?, 20)?;
    assert_infos(&*ffl, &["subInfos=(a((8,9))a((18,19)))/2.0(4,24)"]);
    Ok(())
}

fn test_2_terms_index_2_frags(f: &mut Fixture) -> Result<()> {
    let sflb = SimpleFragListBuilder::new();
    let ffl =
        sflb.create_field_frag_list(&fpl(f, tqf("a"), "a b b b b b b b b b b b b b a")?, 20)?;
    assert_infos(
        &*ffl,
        &[
            "subInfos=(a((0,1)))/1.0(0,20)",
            "subInfos=(a((28,29)))/1.0(20,40)",
        ],
    );

    let ffl = sflb.create_field_frag_list(&fpl(f, tqf("a"), "a b b b b b b b b b b b b a")?, 20)?;
    assert_infos(
        &*ffl,
        &[
            "subInfos=(a((0,1)))/1.0(0,20)",
            "subInfos=(a((26,27)))/1.0(20,40)",
        ],
    );

    let ffl = sflb.create_field_frag_list(&fpl(f, tqf("a"), "a b b b b b b b b b a")?, 20)?;
    assert_infos(
        &*ffl,
        &[
            "subInfos=(a((0,1)))/1.0(0,20)",
            "subInfos=(a((20,21)))/1.0(20,40)",
        ],
    );
    Ok(())
}

fn test_2_terms_query(f: &mut Fixture) -> Result<()> {
    let sflb = SimpleFragListBuilder::new();

    let mut boolean_query = BooleanQueryBuilder::new();
    boolean_query.add(tqf("a"), Occur::SHOULD)?;
    boolean_query.add(tqf("b"), Occur::SHOULD)?;

    let ffl =
        sflb.create_field_frag_list(&fpl(f, Arc::new(boolean_query.build()), "c d e")?, 20)?;
    Assert::assert_equals(0, ffl.get_frag_infos().len());

    let ffl =
        sflb.create_field_frag_list(&fpl(f, Arc::new(boolean_query.build()), "d b c")?, 20)?;
    assert_infos(&*ffl, &["subInfos=(b((2,3)))/1.0(0,20)"]);

    let ffl =
        sflb.create_field_frag_list(&fpl(f, Arc::new(boolean_query.build()), "a b c")?, 20)?;
    assert_infos(&*ffl, &["subInfos=(a((0,1))b((2,3)))/2.0(0,20)"]);
    Ok(())
}

fn test_phrase_query(f: &mut Fixture) -> Result<()> {
    let sflb = SimpleFragListBuilder::new();

    let phrase_query: Arc<dyn Query> = Arc::new(PhraseQuery::from_strings(F, &["a", "b"])?);

    let ffl = sflb.create_field_frag_list(&fpl(f, Arc::clone(&phrase_query), "c d e")?, 20)?;
    Assert::assert_equals(0, ffl.get_frag_infos().len());

    let ffl = sflb.create_field_frag_list(&fpl(f, Arc::clone(&phrase_query), "a c b")?, 20)?;
    Assert::assert_equals(0, ffl.get_frag_infos().len());

    let ffl = sflb.create_field_frag_list(&fpl(f, phrase_query, "a b c")?, 20)?;
    assert_infos(&*ffl, &["subInfos=(ab((0,3)))/1.0(0,20)"]);
    Ok(())
}

fn test_phrase_query_slop(f: &mut Fixture) -> Result<()> {
    let sflb = SimpleFragListBuilder::new();

    let phrase_query: Arc<dyn Query> = Arc::new(PhraseQuery::with_slop(1, F, &["a", "b"])?);

    let ffl = sflb.create_field_frag_list(&fpl(f, phrase_query, "a c b")?, 20)?;
    assert_infos(&*ffl, &["subInfos=(ab((0,1)(4,5)))/1.0(0,20)"]);
    Ok(())
}

fn fpl(f: &mut Fixture, query: Arc<dyn Query>, index_value: &str) -> Result<FieldPhraseList> {
    make1d1f_index(f, index_value)?;
    let fq = FieldQuery::without_reader(&query, true, true)?;
    let mut stack = FieldTermStack::new(f.reader(), 0, F, &fq)?;
    Ok(FieldPhraseList::new(&mut stack, &fq))
}

/// The `FieldPhraseList` of `query` over the index `f` holds.
fn fpl_of(f: &Fixture, query: Arc<dyn Query>) -> Result<FieldPhraseList> {
    let fq = FieldQuery::without_reader(&query, true, true)?;
    let mut stack = FieldTermStack::new(f.reader(), 0, F, &fq)?;
    Ok(FieldPhraseList::new(&mut stack, &fq))
}

fn test_1_phrase_short_mv(f: &mut Fixture) -> Result<()> {
    make_index_short_mv(f)?;
    let fpl = fpl_of(f, tq("d"))?;
    let sflb = SimpleFragListBuilder::new();
    let ffl = sflb.create_field_frag_list(&fpl, 100)?;
    assert_infos(&*ffl, &["subInfos=(d((9,10)))/1.0(0,100)"]);
    Ok(())
}

fn test_1_phrase_long_mv(f: &mut Fixture) -> Result<()> {
    make_index_long_mv(f)?;
    let fpl = fpl_of(f, pq_f(&["search", "engines"]))?;
    let sflb = SimpleFragListBuilder::new();
    let ffl = sflb.create_field_frag_list(&fpl, 100)?;
    assert_infos(
        &*ffl,
        &["subInfos=(searchengines((102,116))searchengines((157,171)))/2.0(87,187)"],
    );
    Ok(())
}

fn test_1_phrase_long_mvb(f: &mut Fixture) -> Result<()> {
    make_index_long_mvb(f)?;
    // "speed" -(2gram)-> "sp","pe","ee","ed"
    let fpl = fpl_of(f, pq_f(&["sp", "pe", "ee", "ed"]))?;
    let sflb = SimpleFragListBuilder::new();
    let ffl = sflb.create_field_frag_list(&fpl, 100)?;
    assert_infos(&*ffl, &["subInfos=(sppeeeed((88,93)))/1.0(41,141)"]);
    Ok(())
}
