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

//! Port of `TestSimplifications.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;

use crate::intervals::support::{
    assert_source_equals, at_least, or, ordered, phrase, phrase_s, term, unordered,
    unordered_no_overlaps,
};
use crate::queries_support::core_support::{class, test};

/// `TestSimplifications`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.queries.intervals.TestSimplifications",
        vec![
            test("testStringPhrases", test_string_phrases),
            test("testSourcePhrases", test_source_phrases),
            test("testOrdered", test_ordered),
            test("testOrderedWithDuplicates", test_ordered_with_duplicates),
            test("testUnordered", test_unordered),
            test(
                "testUnorderedWithDuplicates",
                test_unordered_with_duplicates,
            ),
            test("testUnorderedOverlaps", test_unordered_overlaps),
            test("testDisjunctionSingleton", test_disjunction_singleton),
            test(
                "testDisjunctionRemovesDuplicates",
                test_disjunction_removes_duplicates,
            ),
            test("testPhraseSimplification", test_phrase_simplification),
            test(
                "testDisjunctionSimplification",
                test_disjunction_simplification,
            ),
            test(
                "testMinShouldMatchSimplifications",
                test_min_should_match_simplifications,
            ),
        ],
    )
}

fn test_string_phrases() -> Result<()> {
    // BLOCK(term) => term
    let actual = phrase_s(&["term"]);
    assert_source_equals(&term("term"), &actual);
    Ok(())
}

fn test_source_phrases() -> Result<()> {
    let actual = phrase(vec![term("term")]);
    assert_source_equals(&term("term"), &actual);
    Ok(())
}

fn test_ordered() -> Result<()> {
    // ORDERED(term) => term
    let actual = ordered(vec![term("term")]);
    assert_source_equals(&term("term"), &actual);
    Ok(())
}

fn test_ordered_with_duplicates() -> Result<()> {
    let actual = ordered(vec![term("term"), term("term")]);
    Assert::assert_equals_str(
        None,
        Some("ORDERED(term,term)"),
        Some(&actual.to_source_string()),
    );
    let actual = ordered(vec![term("term"), term("term"), term("bar")]);
    Assert::assert_equals_str(
        None,
        Some("ORDERED(term,term,bar)"),
        Some(&actual.to_source_string()),
    );
    Ok(())
}

fn test_unordered() -> Result<()> {
    // UNORDERED(term) => term
    let actual = unordered(vec![term("term")]);
    assert_source_equals(&term("term"), &actual);
    Ok(())
}

fn test_unordered_with_duplicates() -> Result<()> {
    let actual = unordered(vec![term("term"), term("term")]);
    Assert::assert_equals_str(
        None,
        Some("UNORDERED(term,term)"),
        Some(&actual.to_source_string()),
    );
    let actual = unordered(vec![term("term"), term("term"), term("bar")]);
    Assert::assert_equals_str(
        None,
        Some("UNORDERED(term,term,bar)"),
        Some(&actual.to_source_string()),
    );
    Ok(())
}

fn test_unordered_overlaps() -> Result<()> {
    // UNORDERED_NO_OVERLAPS(term, term) => ORDERED(term, term)
    let actual = unordered_no_overlaps(term("term"), term("term"));
    assert_source_equals(&ordered(vec![term("term"), term("term")]), &actual);
    Ok(())
}

fn test_disjunction_singleton() -> Result<()> {
    let actual = or(vec![term("a")]);
    assert_source_equals(&term("a"), &actual);
    Ok(())
}

fn test_disjunction_removes_duplicates() -> Result<()> {
    // or(a, b, a) => or(a, b)
    let actual = or(vec![term("a"), term("b"), term("a")]);
    assert_source_equals(&or(vec![term("a"), term("b")]), &actual);
    Ok(())
}

fn test_phrase_simplification() -> Result<()> {
    // BLOCK(BLOCK(a, b), c) => BLOCK(a, b, c)
    let actual = phrase(vec![phrase(vec![term("a"), term("b")]), term("c")]);
    assert_source_equals(&phrase(vec![term("a"), term("b"), term("c")]), &actual);
    // BLOCK(a, BLOCK(b, BLOCK(c, d))) => BLOCK(a, b, c, d)
    let actual = phrase(vec![
        term("a"),
        phrase(vec![term("b"), phrase(vec![term("c"), term("d")])]),
    ]);
    assert_source_equals(
        &phrase(vec![term("a"), term("b"), term("c"), term("d")]),
        &actual,
    );
    Ok(())
}

fn test_disjunction_simplification() -> Result<()> {
    // or(a, or(b, or(c, d))) => or(a, b, c, d)
    let actual = or(vec![
        term("a"),
        or(vec![term("b"), or(vec![term("c"), term("d")])]),
    ]);
    assert_source_equals(
        &or(vec![term("a"), term("b"), term("c"), term("d")]),
        &actual,
    );
    Ok(())
}

fn test_min_should_match_simplifications() -> Result<()> {
    let expected = unordered(vec![term("a"), term("b")]);
    assert_source_equals(&expected, &at_least(2, vec![term("a"), term("b")]));
    Assert::assert_equals_str(
        None,
        Some("NOMATCH(Too few sources to match minimum of [3]: [a, b])"),
        Some(&at_least(3, vec![term("a"), term("b")]).to_source_string()),
    );
    Ok(())
}
