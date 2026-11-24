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

//! Port of `TestFieldPhraseList.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query::Builder as BooleanQueryBuilder;
use rucene::search::query::Query;
use rucene_highlighter::vectorhighlight::field_phrase_list::{
    FieldPhraseList, Toffs, WeightedPhraseInfo,
};
use rucene_highlighter::vectorhighlight::field_query::FieldQuery;
use rucene_highlighter::vectorhighlight::field_term_stack::{FieldTermStack, TermInfo};
use rucene_test_framework::util::test_util::TestUtil;

use super::abstract_test_case::{
    make1d1f_index, make1d1f_index_b, make_index_long_mv, make_index_long_mvb, make_index_short_mv,
    pq_f, pq_f_bs, tq, vh_class, Fixture, F,
};
use crate::highlighter_support::core_support::random;
use crate::highlighter_support::search_support::ftest;

/// `TestFieldPhraseList`.
pub fn class_def() -> Arc<Class> {
    vh_class(
        "org.apache.lucene.search.vectorhighlight.TestFieldPhraseList",
        vec![
            ftest("test1TermIndex", test_1_term_index),
            ftest("test2TermsIndex", test_2_terms_index),
            ftest("test1PhraseIndex", test_1_phrase_index),
            ftest("test1PhraseIndexB", test_1_phrase_index_b),
            ftest("test2ConcatTermsIndexB", test_2_concat_terms_index_b),
            ftest("test2Terms1PhraseIndex", test_2_terms_1_phrase_index),
            ftest("testPhraseSlop", test_phrase_slop),
            ftest("test2PhrasesOverlap", test_2_phrases_overlap),
            ftest("test3TermsPhrase", test_3_terms_phrase),
            ftest("testSearchLongestPhrase", test_search_longest_phrase),
            ftest("test1PhraseShortMV", test_1_phrase_short_mv),
            ftest("test1PhraseLongMV", test_1_phrase_long_mv),
            ftest("test1PhraseLongMVB", test_1_phrase_long_mvb),
            ftest(
                "testWeightedPhraseInfoComparisonConsistency",
                test_weighted_phrase_info_comparison_consistency,
            ),
            ftest(
                "testToffsComparisonConsistency",
                test_toffs_comparison_consistency,
            ),
            ftest(
                "testMergeOverlappingWeightedPhraseInfoAccumulateBoost",
                test_merge_overlapping_weighted_phrase_info_accumulate_boost,
            ),
        ],
    )
}

/// `new FieldPhraseList(new FieldTermStack(reader, 0, F, fq), fq)` with
/// `fq = new FieldQuery(query, phraseHighlight, true)`.
fn fpl(f: &Fixture, query: Arc<dyn Query>, phrase_highlight: bool) -> Result<FieldPhraseList> {
    let fq = FieldQuery::without_reader(&query, phrase_highlight, true)?;
    let mut stack = FieldTermStack::new(f.reader(), 0, F, &fq)?;
    Ok(FieldPhraseList::new(&mut stack, &fq))
}

fn assert_phrases(fpl: &FieldPhraseList, expected: &[&str]) {
    Assert::assert_equals(expected.len(), fpl.get_phrase_list().len());
    for (i, e) in expected.iter().enumerate() {
        Assert::assert_equals(e.to_string(), fpl.get_phrase_list()[i].to_string());
    }
}

fn test_1_term_index(f: &mut Fixture) -> Result<()> {
    make1d1f_index(f, "a")?;
    assert_phrases(&fpl(f, tq("a"), true)?, &["a(1.0)((0,1))"]);
    assert_phrases(&fpl(f, tq("b"), true)?, &[]);
    Ok(())
}

fn test_2_terms_index(f: &mut Fixture) -> Result<()> {
    make1d1f_index(f, "a a")?;
    assert_phrases(&fpl(f, tq("a"), true)?, &["a(1.0)((0,1))", "a(1.0)((2,3))"]);
    Ok(())
}

fn test_1_phrase_index(f: &mut Fixture) -> Result<()> {
    make1d1f_index(f, "a b")?;
    assert_phrases(&fpl(f, pq_f(&["a", "b"]), true)?, &["ab(1.0)((0,3))"]);
    assert_phrases(&fpl(f, tq("b"), true)?, &["b(1.0)((2,3))"]);
    Ok(())
}

fn test_1_phrase_index_b(f: &mut Fixture) -> Result<()> {
    // 01 12 23 34 45 56 67 78 (offsets)
    // bb|bb|ba|ac|cb|ba|ab|bc
    //  0  1  2  3  4  5  6  7 (positions)
    make1d1f_index_b(f, "bbbacbabc")?;
    assert_phrases(&fpl(f, pq_f(&["ba", "ac"]), true)?, &["baac(1.0)((2,5))"]);
    Ok(())
}

fn test_2_concat_terms_index_b(f: &mut Fixture) -> Result<()> {
    // 01 12 23 (offsets)
    // ab|ba|ab
    //  0  1  2 (positions)
    make1d1f_index_b(f, "abab")?;
    assert_phrases(
        &fpl(f, tq("ab"), true)?,
        &["ab(1.0)((0,2))", "ab(1.0)((2,4))"],
    );
    Ok(())
}

fn test_2_terms_1_phrase_index(f: &mut Fixture) -> Result<()> {
    make1d1f_index(f, "c a a b")?;

    // phraseHighlight = true
    assert_phrases(&fpl(f, pq_f(&["a", "b"]), true)?, &["ab(1.0)((4,7))"]);

    // phraseHighlight = false
    assert_phrases(
        &fpl(f, pq_f(&["a", "b"]), false)?,
        &["a(1.0)((2,3))", "ab(1.0)((4,7))"],
    );
    Ok(())
}

fn test_phrase_slop(f: &mut Fixture) -> Result<()> {
    make1d1f_index(f, "c a a b c")?;

    let fpl = fpl(f, pq_f_bs(2.0, 1, &["a", "c"]), true)?;
    assert_phrases(&fpl, &["ac(2.0)((4,5)(8,9))"]);
    Assert::assert_equals(4, fpl.get_phrase_list()[0].get_start_offset());
    Assert::assert_equals(9, fpl.get_phrase_list()[0].get_end_offset());
    Ok(())
}

fn test_2_phrases_overlap(f: &mut Fixture) -> Result<()> {
    make1d1f_index(f, "d a b c d")?;

    let mut query = BooleanQueryBuilder::new();
    query.add(pq_f(&["a", "b"]), Occur::SHOULD)?;
    query.add(pq_f(&["b", "c"]), Occur::SHOULD)?;
    assert_phrases(
        &fpl(f, Arc::new(query.build()), true)?,
        &["abc(1.0)((2,7))"],
    );
    Ok(())
}

fn test_3_terms_phrase(f: &mut Fixture) -> Result<()> {
    make1d1f_index(f, "d a b a b c d")?;
    assert_phrases(
        &fpl(f, pq_f(&["a", "b", "c"]), true)?,
        &["abc(1.0)((6,11))"],
    );
    Ok(())
}

fn test_search_longest_phrase(f: &mut Fixture) -> Result<()> {
    make1d1f_index(f, "d a b d c a b c")?;

    let mut query = BooleanQueryBuilder::new();
    query.add(pq_f(&["a", "b"]), Occur::SHOULD)?;
    query.add(pq_f(&["a", "b", "c"]), Occur::SHOULD)?;
    assert_phrases(
        &fpl(f, Arc::new(query.build()), true)?,
        &["ab(1.0)((2,5))", "abc(1.0)((10,15))"],
    );
    Ok(())
}

fn test_1_phrase_short_mv(f: &mut Fixture) -> Result<()> {
    make_index_short_mv(f)?;
    assert_phrases(&fpl(f, tq("d"), true)?, &["d(1.0)((9,10))"]);
    Ok(())
}

fn test_1_phrase_long_mv(f: &mut Fixture) -> Result<()> {
    make_index_long_mv(f)?;
    assert_phrases(
        &fpl(f, pq_f(&["search", "engines"]), true)?,
        &[
            "searchengines(1.0)((102,116))",
            "searchengines(1.0)((157,171))",
        ],
    );
    Ok(())
}

fn test_1_phrase_long_mvb(f: &mut Fixture) -> Result<()> {
    make_index_long_mvb(f)?;
    // "speed" -(2gram)-> "sp","pe","ee","ed"
    assert_phrases(
        &fpl(f, pq_f(&["sp", "pe", "ee", "ed"]), true)?,
        &["sppeeeed(1.0)((88,93))"],
    );
    Ok(())
}

fn test_weighted_phrase_info_comparison_consistency(_f: &mut Fixture) -> Result<()> {
    let a = new_info(0, 0, 1.0);
    let b = new_info(1, 2, 1.0);
    let c = new_info(2, 3, 1.0);
    let d = new_info(0, 0, 1.0);
    let e = new_info(0, 0, 2.0);

    let eq = |x: &WeightedPhraseInfo, y: &WeightedPhraseInfo| {
        Assert::assert_true(x == y);
        Assert::assert_true(y == x);
        Assert::assert_equals(x.hash_code(), y.hash_code());
        Assert::assert_equals(0, x.compare_to(y));
        Assert::assert_equals(0, y.compare_to(x));
    };
    let lt = |x: &WeightedPhraseInfo, y: &WeightedPhraseInfo| {
        Assert::assert_false(x == y);
        Assert::assert_false(y == x);
        Assert::assert_false(x.hash_code() == y.hash_code());
        Assert::assert_true(x.compare_to(y) < 0);
        Assert::assert_true(y.compare_to(x) > 0);
    };
    eq(&a, &a);
    eq(&b, &b);
    eq(&c, &c);
    eq(&d, &d);
    eq(&e, &e);
    eq(&a, &d);
    lt(&a, &b);
    lt(&b, &c);
    lt(&a, &c);
    lt(&a, &e);
    lt(&e, &b);
    lt(&e, &c);
    lt(&d, &b);
    lt(&d, &c);
    lt(&d, &e);
    Ok(())
}

fn test_toffs_comparison_consistency(_f: &mut Fixture) -> Result<()> {
    let a = Toffs::new(0, 0);
    let b = Toffs::new(1, 2);
    let c = Toffs::new(2, 3);
    let d = Toffs::new(0, 0);

    let eq = |x: &Toffs, y: &Toffs| {
        Assert::assert_true(x == y);
        Assert::assert_true(y == x);
        Assert::assert_equals(x.hash_code(), y.hash_code());
        Assert::assert_equals(0, x.compare_to(y));
        Assert::assert_equals(0, y.compare_to(x));
    };
    let lt = |x: &Toffs, y: &Toffs| {
        Assert::assert_false(x == y);
        Assert::assert_false(y == x);
        Assert::assert_false(x.hash_code() == y.hash_code());
        Assert::assert_true(x.compare_to(y) < 0);
        Assert::assert_true(y.compare_to(x) > 0);
    };
    eq(&a, &a);
    eq(&b, &b);
    eq(&c, &c);
    eq(&d, &d);
    eq(&a, &d);
    lt(&a, &b);
    lt(&b, &c);
    lt(&a, &c);
    lt(&d, &b);
    lt(&d, &c);
    Ok(())
}

fn new_info(start_offset: i32, end_offset: i32, boost: f32) -> WeightedPhraseInfo {
    let infos = vec![TermInfo::new(
        &TestUtil::random_unicode_string(&*random()),
        start_offset,
        end_offset,
        0,
        0.0,
    )];
    WeightedPhraseInfo::new(&infos, boost)
}

fn test_merge_overlapping_weighted_phrase_info_accumulate_boost(_f: &mut Fixture) -> Result<()> {
    let infos1 = vec![
        TermInfo::new("中国", 0, 2, 0, 0.0),
        TermInfo::new("经济", 3, 5, 1, 1.0),
    ];
    let wpi1 = WeightedPhraseInfo::new(&infos1, 2.0);

    let infos2 = vec![
        TermInfo::new("经济", 3, 5, 1, 1.0),
        TermInfo::new("发展", 6, 8, 2, 2.0),
    ];
    let wpi2 = WeightedPhraseInfo::new(&infos2, 3.0);

    let mut fpl = FieldPhraseList::from_merge(&[]);
    fpl.get_phrase_list_mut().push(wpi1);

    // This will execute: existWpi.boost += wpi.getBoost();
    fpl.add_if_no_overlap(wpi2);

    Assert::assert_equals(1, fpl.get_phrase_list().len());

    let merged = &fpl.get_phrase_list()[0];

    Assert::assert_equals_float_msg(None, 5.0, merged.get_boost(), 0.0001);
    Ok(())
}
