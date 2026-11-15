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

//! Port of `TestDisjunctionRewrites.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::error::Result;

use crate::intervals::support::{
    assert_source_equals, assert_source_not_equals, contained_by, containing, fix_field, maxgaps,
    maxwidth, not_contained_by, not_containing, or, or_rewrite, ordered, phrase, phrase_s, term,
};
use crate::queries_support::core_support::{class, test};

/// `TestDisjunctionRewrites`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.queries.intervals.TestDisjunctionRewrites",
        vec![
            test("testDisjunctionSuffix", test_disjunction_suffix),
            test(
                "testPhraseDisjunctionWithDifferentLengthClauses",
                test_phrase_disjunction_with_different_length_clauses,
            ),
            test(
                "testPhraseDisjunctionWithNestedDifferentLengthClauses",
                test_phrase_disjunction_with_nested_different_length_clauses,
            ),
            test(
                "testDisjunctionRewritePreservesFilters",
                test_disjunction_rewrite_preserves_filters,
            ),
            test("testNestedMaxGaps", test_nested_max_gaps),
            test("testNestedMaxWidth", test_nested_max_width),
            test("testNestedFixField", test_nested_fix_field),
            test("testContainedBy", test_contained_by),
            test("testContaining", test_containing),
            test("testNotContainedBy", test_not_contained_by),
            test("testNotContaining", test_not_containing),
            test("testBlockedRewrites", test_blocked_rewrites),
        ],
    )
}

fn test_disjunction_suffix() -> Result<()> {
    // BLOCK(a,or(b, BLOCK(b, c))) => or(BLOCK(a, b), BLOCK(a, b, c))
    let actual = phrase(vec![term("a"), or(vec![term("b"), phrase_s(&["b", "c"])])]);
    let expected = or(vec![phrase_s(&["a", "b"]), phrase_s(&["a", "b", "c"])]);
    assert_source_equals(&expected, &actual);
    Ok(())
}

fn test_phrase_disjunction_with_different_length_clauses() -> Result<()> {
    // BLOCK(a, or(b, BLOCK(b, c)), d) => or(BLOCK(a, b, d), BLOCK(a, b, c, d))
    let actual = phrase(vec![
        term("a"),
        or(vec![term("b"), phrase(vec![term("b"), term("c")])]),
        term("d"),
    ]);
    let expected = or(vec![
        phrase_s(&["a", "b", "d"]),
        phrase_s(&["a", "b", "c", "d"]),
    ]);
    assert_source_equals(&expected, &actual);
    Ok(())
}

fn test_phrase_disjunction_with_nested_different_length_clauses() -> Result<()> {
    // BLOCK(a, or(ORDERED(or(b, c), d), b, p, q), f, g)
    //  => or(BLOCK(a, or(b, p, q), f, g), BLOCK(a, ORDERED(or(b, c), d), f, g))
    let expected = or(vec![
        phrase(vec![
            term("a"),
            or(vec![term("b"), term("p"), term("q")]),
            term("f"),
            term("g"),
        ]),
        phrase(vec![
            term("a"),
            ordered(vec![or(vec![term("b"), term("c")]), term("d")]),
            term("f"),
            term("g"),
        ]),
    ]);
    let actual = phrase(vec![
        term("a"),
        or(vec![
            ordered(vec![or(vec![term("b"), term("c")]), term("d")]),
            term("b"),
            term("p"),
            term("q"),
        ]),
        term("f"),
        term("g"),
    ]);
    assert_source_equals(&expected, &actual);
    Ok(())
}

fn test_disjunction_rewrite_preserves_filters() -> Result<()> {
    // BLOCK(a, MAXGAPS/3(OR(BLOCK(a, b), BLOCK(c, d))), c)
    // => or(BLOCK(a, MAXGAPS/3(BLOCK(a, b)), c), BLOCK(a, MAXGAPS/3(BLOCK(c, d)), c))
    let actual = phrase(vec![
        term("a"),
        maxgaps(3, or(vec![phrase_s(&["a", "b"]), phrase_s(&["c", "d"])])),
        term("c"),
    ]);
    let expected = or(vec![
        phrase(vec![
            term("a"),
            maxgaps(3, phrase_s(&["a", "b"])),
            term("c"),
        ]),
        phrase(vec![
            term("a"),
            maxgaps(3, phrase_s(&["c", "d"])),
            term("c"),
        ]),
    ]);
    assert_source_equals(&expected, &actual);
    Ok(())
}

fn test_nested_max_gaps() -> Result<()> {
    // MAXGAPS/3(ORDERED(MAXGAPS/4(ORDERED(a, or(b, BLOCK(c, d)))), e)
    // => or(MAXGAPS/3(ORDERED(MAXGAPS/4(ORDERED(a, b)), e)), MAXGAPS/3(ORDERED(MAXGAPS/4(ORDERED(a,
    // BLOCK(c, d))), e)))
    let actual = maxgaps(
        3,
        ordered(vec![
            maxgaps(
                4,
                ordered(vec![term("a"), or(vec![term("b"), phrase_s(&["c", "d"])])]),
            ),
            term("e"),
        ]),
    );
    let expected = or(vec![
        maxgaps(
            3,
            ordered(vec![
                maxgaps(4, ordered(vec![term("a"), term("b")])),
                term("e"),
            ]),
        ),
        maxgaps(
            3,
            ordered(vec![
                maxgaps(4, ordered(vec![term("a"), phrase_s(&["c", "d"])])),
                term("e"),
            ]),
        ),
    ]);
    assert_source_equals(&expected, &actual);
    Ok(())
}

fn test_nested_max_width() -> Result<()> {
    // maxwidth does not automatically pull up disjunctions at construction time, so we need to
    // check
    // that it does the right thing if wrapped by something that does need exact internal accounting
    // PHRASE(a, MAXWIDTH(4, OR(ORDERED(b, c), ORDERED(d, e))), f)
    // => or(PHRASE(a, MAXWIDTH(4, ORDERED(b, c)), f), PHRASE(a, MAXWIDTH(4, ORDERED(d, e)), f))
    let actual = phrase(vec![
        term("a"),
        maxwidth(
            4,
            or(vec![
                ordered(vec![term("b"), term("c")]),
                ordered(vec![term("d"), term("e")]),
            ]),
        ),
        term("f"),
    ]);
    let expected = or(vec![
        phrase(vec![
            term("a"),
            maxwidth(4, ordered(vec![term("b"), term("c")])),
            term("f"),
        ]),
        phrase(vec![
            term("a"),
            maxwidth(4, ordered(vec![term("d"), term("e")])),
            term("f"),
        ]),
    ]);
    assert_source_equals(&expected, &actual);
    Ok(())
}

fn test_nested_fix_field() -> Result<()> {
    // PHRASE(a, FIXFIELD(field, or(PHRASE(a, b), b)), c)
    // => or(PHRASE(a, FIXFIELD(PHRASE(a, b)), c), PHRASE(a, FIXFIELD(b), c))
    let actual = phrase(vec![
        term("a"),
        fix_field("field", or(vec![phrase_s(&["a", "b"]), term("b")])),
        term("c"),
    ]);
    let expected = or(vec![
        phrase(vec![
            term("a"),
            fix_field("field", phrase_s(&["a", "b"])),
            term("c"),
        ]),
        phrase(vec![term("a"), fix_field("field", term("b")), term("c")]),
    ]);
    assert_source_equals(&expected, &actual);
    Ok(())
}

fn test_contained_by() -> Result<()> {
    // the 'big' interval should not be minimized, the 'small' one should be
    // CONTAINED_BY(or("s", BLOCK("s", "t")), MAXGAPS/4(or(ORDERED("a", "b"), ORDERED("c", "d"))))
    // => or(CONTAINED_BY(or("s", BLOCK("s", "t")), MAXGAPS/4(ORDERED("a", "b"))),
    //       CONTAINED_BY(or("s", BLOCK("s", "t")), MAXGAPS/4(ORDERED("c", "d"))))
    let actual = contained_by(
        or(vec![term("s"), phrase_s(&["s", "t"])]),
        maxgaps(
            4,
            or(vec![
                ordered(vec![term("a"), term("b")]),
                ordered(vec![term("c"), term("d")]),
            ]),
        ),
    );
    let expected = or(vec![
        contained_by(
            or(vec![term("s"), phrase_s(&["s", "t"])]),
            maxgaps(4, ordered(vec![term("a"), term("b")])),
        ),
        contained_by(
            or(vec![term("s"), phrase_s(&["s", "t"])]),
            maxgaps(4, ordered(vec![term("c"), term("d")])),
        ),
    ]);
    assert_source_equals(&expected, &actual);
    Ok(())
}

fn test_containing() -> Result<()> {
    // the 'big' interval should not be minimized, the 'small' one should be
    // CONTAINING(MAXGAPS/4(or(ORDERED("a", "b"), ORDERED("c", "d"))), or("s", BLOCK("s", "t")))
    // => or(CONTAINING(MAXGAPS/4(ORDERED("a", "b")), or("s", BLOCK("s", "t"))),
    //       CONTAINING(MAXGAPS/4(ORDERED("c", "d")), or("s", BLOCK("s", "t"))))
    let actual = containing(
        maxgaps(
            4,
            or(vec![
                ordered(vec![term("a"), term("b")]),
                ordered(vec![term("c"), term("d")]),
            ]),
        ),
        or(vec![term("s"), phrase_s(&["s", "t"])]),
    );
    let expected = or(vec![
        containing(
            maxgaps(4, ordered(vec![term("a"), term("b")])),
            or(vec![term("s"), phrase_s(&["s", "t"])]),
        ),
        containing(
            maxgaps(4, ordered(vec![term("c"), term("d")])),
            or(vec![term("s"), phrase_s(&["s", "t"])]),
        ),
    ]);
    assert_source_equals(&expected, &actual);
    Ok(())
}

fn test_not_contained_by() -> Result<()> {
    // the 'big' interval should not be minimized, the 'small' one should be
    // NOT_CONTAINED_BY(or(BLOCK("a", "b"), "a"), or(BLOCK("c", "d"), "d"))
    // => or(NOT_CONTAINED_BY(or(BLOCK("a", "b"), "a"), BLOCK("c", "d"))),
    // NOT_CONTAINED_BY(or(BLOCK("a", "b"), "a"), "d"))
    let actual = not_contained_by(
        or(vec![phrase_s(&["a", "b"]), term("a")]),
        or(vec![phrase_s(&["c", "d"]), term("d")]),
    );
    let expected = or(vec![
        not_contained_by(
            or(vec![phrase_s(&["a", "b"]), term("a")]),
            phrase_s(&["c", "d"]),
        ),
        not_contained_by(or(vec![phrase_s(&["a", "b"]), term("a")]), term("d")),
    ]);
    assert_source_equals(&expected, &actual);
    Ok(())
}

fn test_not_containing() -> Result<()> {
    // the 'big' interval should not be minimized, the 'small' one should be
    // NOT_CONTAINING(or(BLOCK("a", "b"), "a"), or(BLOCK("c", "d"), "d"))
    // => or(NOT_CONTAINING(BLOCK("a", "b"), or(BLOCK("c", "d"), "d")), NOT_CONTAINING("a",
    // or(BLOCK("c", "d"), "d")))
    let actual = not_containing(
        or(vec![phrase_s(&["a", "b"]), term("a")]),
        or(vec![phrase_s(&["c", "d"]), term("d")]),
    );
    let expected = or(vec![
        not_containing(
            phrase_s(&["a", "b"]),
            or(vec![phrase_s(&["c", "d"]), term("d")]),
        ),
        not_containing(term("a"), or(vec![phrase_s(&["c", "d"]), term("d")])),
    ]);
    assert_source_equals(&expected, &actual);
    Ok(())
}

fn test_blocked_rewrites() -> Result<()> {
    let actual = phrase(vec![
        term("a"),
        or_rewrite(false, vec![phrase_s(&["b", "c"]), term("c")]),
    ]);
    let if_rewritten = or(vec![phrase_s(&["a", "b", "c"]), phrase_s(&["a", "c"])]);
    assert_source_not_equals(&if_rewritten, &actual);
    Ok(())
}
