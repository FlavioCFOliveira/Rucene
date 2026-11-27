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

//! Port of `TestFieldTermStack.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::Term;
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query::Builder as BooleanQueryBuilder;
use rucene::search::query::Query;
use rucene::search::wildcard_query::WildcardQuery;
use rucene_highlighter::vectorhighlight::field_query::FieldQuery;
use rucene_highlighter::vectorhighlight::field_term_stack::{FieldTermStack, TermInfo};
use rucene_test_framework::util::test_util::TestUtil;

use super::abstract_test_case::{
    make1dmf_index, make1dmf_index_b, make_index_long_mv, make_index_long_mvb, make_index_short_mv,
    pq_f, tq, vh_class, Fixture, F,
};
use crate::highlighter_support::core_support::random;
use crate::highlighter_support::search_support::ftest;

/// `TestFieldTermStack`.
pub fn class_def() -> Arc<Class> {
    vh_class(
        "org.apache.lucene.search.vectorhighlight.TestFieldTermStack",
        vec![
            ftest("test1Term", test_1_term),
            ftest("test2Terms", test_2_terms),
            ftest("test1Phrase", test_1_phrase),
            ftest("test1TermB", test_1_term_b),
            ftest("test2TermsB", test_2_terms_b),
            ftest("test1PhraseB", test_1_phrase_b),
            ftest("test1PhraseShortMV", test_1_phrase_short_mv),
            ftest("test1PhraseLongMV", test_1_phrase_long_mv),
            ftest("test1PhraseMVB", test_1_phrase_mvb),
            ftest("testWildcard", test_wildcard),
            ftest(
                "testTermInfoComparisonConsistency",
                test_term_info_comparison_consistency,
            ),
        ],
    )
}

/// `new FieldTermStack(reader, 0, F, new FieldQuery(query, true, true))`,
/// its `termList.size()` asserted, then every `pop().toString()`.
fn assert_stack(f: &Fixture, fq: &FieldQuery, expected: &[&str]) -> Result<()> {
    let mut stack = FieldTermStack::new(f.reader(), 0, F, fq)?;
    Assert::assert_equals(expected.len(), stack.term_list().len());
    for e in expected {
        Assert::assert_equals(e.to_string(), stack.pop().expect("a term info").to_string());
    }
    Ok(())
}

fn fq(query: Arc<dyn Query>) -> Result<FieldQuery> {
    FieldQuery::without_reader(&query, true, true)
}

fn test_1_term(f: &mut Fixture) -> Result<()> {
    make_index(f)?;
    assert_stack(
        f,
        &fq(tq("a"))?,
        &[
            "a(0,1,0)",
            "a(2,3,1)",
            "a(4,5,2)",
            "a(12,13,6)",
            "a(28,29,14)",
            "a(32,33,16)",
        ],
    )
}

fn test_2_terms(f: &mut Fixture) -> Result<()> {
    make_index(f)?;
    let mut query = BooleanQueryBuilder::new();
    query.add(tq("b"), Occur::SHOULD)?;
    query.add(tq("c"), Occur::SHOULD)?;
    assert_stack(
        f,
        &fq(Arc::new(query.build()))?,
        &[
            "b(6,7,3)",
            "b(8,9,4)",
            "c(10,11,5)",
            "b(14,15,7)",
            "b(16,17,8)",
            "c(18,19,9)",
            "b(26,27,13)",
            "b(30,31,15)",
        ],
    )
}

fn test_1_phrase(f: &mut Fixture) -> Result<()> {
    make_index(f)?;
    assert_stack(
        f,
        &fq(pq_f(&["c", "d"]))?,
        &["c(10,11,5)", "c(18,19,9)", "d(20,21,10)"],
    )
}

fn make_index(f: &mut Fixture) -> Result<()> {
    //           111111111122222
    // 0123456789012345678901234 (offsets)
    // a a a b b c a b b c d e f
    // 0 1 2 3 4 5 6 7 8 9101112 (position)
    let value1 = "a a a b b c a b b c d e f";
    // 222233333
    // 678901234 (offsets)
    // b a b a f
    // 1314151617 (position)
    let value2 = "b a b a f";

    make1dmf_index(f, &[value1, value2])
}

fn test_1_term_b(f: &mut Fixture) -> Result<()> {
    make_index_b(f)?;
    assert_stack(f, &fq(tq("ab"))?, &["ab(2,4,2)", "ab(6,8,6)"])
}

fn test_2_terms_b(f: &mut Fixture) -> Result<()> {
    make_index_b(f)?;
    let mut query = BooleanQueryBuilder::new();
    query.add(tq("bc"), Occur::SHOULD)?;
    query.add(tq("ef"), Occur::SHOULD)?;
    assert_stack(
        f,
        &fq(Arc::new(query.build()))?,
        &["bc(4,6,4)", "bc(8,10,8)", "ef(11,13,11)"],
    )
}

fn test_1_phrase_b(f: &mut Fixture) -> Result<()> {
    make_index_b(f)?;
    assert_stack(
        f,
        &fq(pq_f(&["ab", "bb"]))?,
        &["ab(2,4,2)", "bb(3,5,3)", "ab(6,8,6)", "bb(7,9,7)"],
    )
}

fn make_index_b(f: &mut Fixture) -> Result<()> {
    //                             1 11 11
    // 01 12 23 34 45 56 67 78 89 90 01 12 (offsets)
    // aa|aa|ab|bb|bc|ca|ab|bb|bc|cd|de|ef
    //  0  1  2  3  4  5  6  7  8  9 10 11 (position)
    let value = "aaabbcabbcdef";

    make1dmf_index_b(f, &[value])
}

fn test_1_phrase_short_mv(f: &mut Fixture) -> Result<()> {
    make_index_short_mv(f)?;
    assert_stack(f, &fq(tq("d"))?, &["d(9,10,3)"])
}

fn test_1_phrase_long_mv(f: &mut Fixture) -> Result<()> {
    make_index_long_mv(f)?;
    assert_stack(
        f,
        &fq(pq_f(&["search", "engines"]))?,
        &[
            "search(102,108,14)",
            "engines(109,116,15)",
            "search(157,163,24)",
            "engines(164,171,25)",
        ],
    )
}

fn test_1_phrase_mvb(f: &mut Fixture) -> Result<()> {
    make_index_long_mvb(f)?;
    // "speed" -(2gram)-> "sp","pe","ee","ed"
    assert_stack(
        f,
        &fq(pq_f(&["sp", "pe", "ee", "ed"]))?,
        &[
            "sp(88,90,61)",
            "pe(89,91,62)",
            "ee(90,92,63)",
            "ed(91,93,64)",
        ],
    )
}

fn test_wildcard(f: &mut Fixture) -> Result<()> {
    make_index_long_mv(f)?;
    let query: Arc<dyn Query> = Arc::new(WildcardQuery::new(Term::from_text(F, "th*e"))?);
    let fq = FieldQuery::new(&query, f.reader.as_ref(), true, true)?;
    assert_stack(
        f,
        &fq,
        &[
            "the(15,18,2)",
            "these(133,138,20)",
            "the(153,156,23)",
            "the(195,198,31)",
        ],
    )
}

fn test_term_info_comparison_consistency(_f: &mut Fixture) -> Result<()> {
    let a = TermInfo::new(&TestUtil::random_unicode_string(&*random()), 0, 0, 0, 1.0);
    let b = TermInfo::new(&TestUtil::random_unicode_string(&*random()), 0, 0, 1, 1.0);
    let c = TermInfo::new(&TestUtil::random_unicode_string(&*random()), 0, 0, 2, 1.0);
    let d = TermInfo::new(&TestUtil::random_unicode_string(&*random()), 0, 0, 0, 1.0);

    assert_consistent_equals(&a, &a);
    assert_consistent_equals(&b, &b);
    assert_consistent_equals(&c, &c);
    assert_consistent_equals(&d, &d);
    assert_consistent_equals(&a, &d);
    assert_consistent_less_than(&a, &b);
    assert_consistent_less_than(&b, &c);
    assert_consistent_less_than(&a, &c);
    assert_consistent_less_than(&d, &b);
    assert_consistent_less_than(&d, &c);
    Ok(())
}

fn assert_consistent_equals(a: &TermInfo, b: &TermInfo) {
    Assert::assert_true(a == b);
    Assert::assert_true(b == a);
    Assert::assert_equals(a.hash_code(), b.hash_code());
    Assert::assert_equals(0, a.compare_to(b));
    Assert::assert_equals(0, b.compare_to(a));
}

fn assert_consistent_less_than(a: &TermInfo, b: &TermInfo) {
    Assert::assert_false(a == b);
    Assert::assert_false(b == a);
    Assert::assert_false(a.hash_code() == b.hash_code());
    Assert::assert_true(a.compare_to(b) < 0);
    Assert::assert_true(b.compare_to(a) > 0);
}
