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

//! The Java-oracle portability test of `queries.intervals` (rmp #482): an
//! `IntervalQuery` over every `Intervals` factory source (`term` with and
//! without a payload filter, `phrase`, `or` with and without rewriting,
//! `prefix`, `wildcard`, `regexp`, `range`, `fuzzyTerm`, `multiterm` with
//! their expansion limits, `maxwidth`, `maxgaps`, `extend`, `ordered`,
//! `unordered`, `unorderedNoOverlaps`, `fixField`, `nonOverlapping`,
//! `overlapping`, `notWithin`, `within`, `notContaining`, `containing`,
//! `notContainedBy`, `containedBy`, `atLeast`, `before`, `after`,
//! `noIntervals`, `analyzedText`) and the scoring variants of
//! `IntervalQuery`, over a seeded index Rucene writes
//! (`OracleIndex::standard`: three segments, deleted documents, a field
//! without positions).
//!
//! For each query Lucene 10.5.0 and Rucene must give the same `toString()`
//! and class tree before and after rewrite, count, hits with bit-exact
//! `float` scores and `explain` of every document; for each source the same
//! intervals and matches (`intervals` records, see `interval_oracle`). Edge
//! cases (absent terms and fields, a field without positions, expansion
//! limits, too few sources, invalid pivots) give the same exception class and
//! message.
//!
//! The comparison is sharded over several tests so nextest runs them in
//! parallel; each writes its own index and runs its own JVM in its own
//! directory under `TMPDIR`.

mod interval_oracle;
mod query_oracle;

use interval_oracle::{
    at_least, bref, call, compiled, intervals_dumper, or_no_rewrite, phrase_terms, query,
    query_pivot, query_pivot_exp, regexp_string, registry, term, term_bytes, term_payload, vararg,
};
use query_oracle::{compare, expr, lit, OracleIndex, SearchOracle};

/// The seed of the index.
const SEED: i64 = 0x0482_5eed;

/// The number of documents of the index.
const NUM_DOCS: usize = 60;

fn oracle(index: &OracleIndex) -> SearchOracle {
    let mut oracle = SearchOracle::new(index, registry());
    oracle.custom_dumper("intervals", intervals_dumper());
    oracle
}

/// A query record (`IntervalQuery` over `body`) and an intervals record
/// (field `body`) for each source.
fn add_sources(oracle: &mut SearchOracle, sources: &[(&str, String)]) {
    for (id, source) in sources {
        oracle.query(&format!("iq-{id}"), &query("body", source));
        oracle.custom("intervals", &format!("iv-{id}"), source, "body");
    }
}

fn count_records(dump: &str, kind: &str) -> usize {
    dump.lines()
        .filter(|l| l.starts_with(&format!("{kind}\t")))
        .count()
}

/// Runs both sides and panics with the first difference; `min_intervals`
/// guards against a vacuous comparison.
fn assert_parity(oracle: &SearchOracle, min_intervals: usize) {
    let java = oracle.java_dump();
    let rust = oracle.rust_dump();
    let intervals = count_records(&java, "iv");
    println!(
        "{} queries, {} hits, {} intervals, {} matches, {} dump lines",
        count_records(&java, "query"),
        count_records(&java, "hit"),
        intervals,
        count_records(&java, "m"),
        java.lines().count()
    );
    assert!(
        intervals >= min_intervals,
        "the dump has {intervals} intervals, too few to prove parity"
    );
    if let Err(diff) = compare(&java, &rust) {
        panic!("interval parity with Lucene 10.5.0 fails:\n{diff}");
    }
    // negative control: a changed interval must make the comparison fail
    let tampered = rust.replacen("\niv\t", "\niv\t-", 1);
    assert!(
        compare(&java, &tampered).is_err(),
        "the comparison does not see a changed interval"
    );
}

fn t(s: &str) -> String {
    term(s)
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| term(x)).collect()
}

#[test]
fn intervals_terms_phrases_disjunctions() {
    let index = OracleIndex::standard(SEED, NUM_DOCS);
    let mut o = oracle(&index);
    let sources = vec![
        ("term-common", t("the")),
        ("term-rare", t("zulu")),
        ("term-absent", t("nonexistent")),
        ("term-bytes", term_bytes("alpha")),
        ("term-payload", term_payload("of")),
        ("phrase-terms", phrase_terms(&["the", "of"])),
        ("phrase-one", phrase_terms(&["the"])),
        (
            "phrase-sources",
            vararg("phrase", &[t("the"), vararg("or", &s(&["alpha", "bravo"]))]),
        ),
        (
            "phrase-nested",
            vararg("phrase", &[phrase_terms(&["the", "of"]), t("and")]),
        ),
        ("or-3", vararg("or", &s(&["alpha", "bravo", "charlie"]))),
        ("or-dup", vararg("or", &s(&["alpha", "alpha", "bravo"]))),
        ("or-one", vararg("or", &s(&["delta"]))),
        (
            "or-nested",
            vararg("or", &[t("echo"), vararg("or", &s(&["golf", "hotel"]))]),
        ),
        (
            "or-no-rewrite",
            vararg(
                "phrase",
                &[
                    t("the"),
                    or_no_rewrite(&[phrase_terms(&["of", "and"]), t("of")]),
                ],
            ),
        ),
        (
            "or-rewrite",
            vararg(
                "phrase",
                &[
                    t("the"),
                    vararg("or", &[phrase_terms(&["of", "and"]), t("of")]),
                ],
            ),
        ),
        ("or-absent", vararg("or", &s(&["nonexistent", "missing"]))),
    ];
    add_sources(&mut o, &sources);
    o.custom("intervals", "iv-title", &t("alpha"), "title");
    o.query("iq-title", &query("title", &t("alpha")));
    o.custom("intervals", "iv-nofield", &t("the"), "nofield");
    o.custom("intervals", "iv-nopositions", &t("red"), "tag");
    o.query("iq-nopositions", &query("tag", &t("red")));
    assert_parity(&o, 200);
}

#[test]
fn intervals_multi_term_sources() {
    let index = OracleIndex::standard(SEED, NUM_DOCS);
    let mut o = oracle(&index);
    let sources = vec![
        ("prefix", call("prefix", &[&bref("ch")])),
        ("prefix-limit", call("prefix", &[&bref("a"), "i:1"])),
        ("prefix-absent", call("prefix", &[&bref("qqq")])),
        ("wildcard", call("wildcard", &[&bref("?ravo")])),
        ("wildcard-limit", call("wildcard", &[&bref("*o"), "i:2"])),
        ("regexp", call("regexp", &[&bref("[a-c].*")])),
        ("regexp-limit", call("regexp", &[&bref(".*a"), "i:1"])),
        (
            "range",
            call(
                "range",
                &[&bref("delta"), &bref("golf"), "z:true", "z:false"],
            ),
        ),
        (
            "range-open-lower",
            call("range", &["null", &bref("bravo"), "z:false", "z:true"]),
        ),
        (
            "range-open-upper",
            call("range", &[&bref("x"), "null", "z:true", "z:true"]),
        ),
        (
            "range-limit",
            call(
                "range",
                &[&bref("alpha"), &bref("zulu"), "z:true", "z:true", "i:1"],
            ),
        ),
        ("fuzzy", call("fuzzyTerm", &[&lit("alpah"), "i:1"])),
        ("fuzzy-2", call("fuzzyTerm", &[&lit("bravp"), "i:2"])),
        (
            "fuzzy-full",
            call(
                "fuzzyTerm",
                &[&lit("charly"), "i:2", "i:1", "z:false", "i:128"],
            ),
        ),
        (
            "fuzzy-limit",
            call("fuzzyTerm", &[&lit("the"), "i:2", "i:0", "z:true", "i:1"]),
        ),
        (
            "multiterm",
            call(
                "multiterm",
                &[&compiled("[a-d].*"), &regexp_string("[a-d].*")],
            ),
        ),
        (
            "multiterm-limit",
            call(
                "multiterm",
                &[&compiled("[a-d].*"), "i:1", &regexp_string("[a-d].*")],
            ),
        ),
        (
            "ordered-prefix",
            vararg("ordered", &[call("prefix", &[&bref("th")]), t("of")]),
        ),
    ];
    add_sources(&mut o, &sources);
    assert_parity(&o, 200);
}

#[test]
fn intervals_filters_and_conjunctions() {
    let index = OracleIndex::standard(SEED, NUM_DOCS);
    let mut o = oracle(&index);
    let sources = vec![
        ("ordered", vararg("ordered", &s(&["the", "of"]))),
        ("ordered-3", vararg("ordered", &s(&["alpha", "the", "of"]))),
        ("ordered-dup", vararg("ordered", &s(&["the", "the"]))),
        ("ordered-one", vararg("ordered", &s(&["the"]))),
        ("unordered", vararg("unordered", &s(&["the", "of", "and"]))),
        (
            "unordered-dup",
            vararg("unordered", &s(&["of", "of", "the"])),
        ),
        (
            "unordered-no-overlaps",
            call("unorderedNoOverlaps", &[&t("the"), &t("the")]),
        ),
        (
            "unordered-no-overlaps-or",
            call(
                "unorderedNoOverlaps",
                &[&vararg("or", &s(&["the", "of"])), &t("and")],
            ),
        ),
        (
            "maxwidth",
            call("maxwidth", &["i:3", &vararg("ordered", &s(&["the", "of"]))]),
        ),
        (
            "maxgaps",
            call(
                "maxgaps",
                &["i:2", &vararg("unordered", &s(&["the", "and"]))],
            ),
        ),
        (
            "maxgaps-0",
            call("maxgaps", &["i:0", &vararg("ordered", &s(&["the", "of"]))]),
        ),
        ("extend", call("extend", &[&t("alpha"), "i:1", "i:2"])),
        (
            "extend-max",
            call("extend", &[&t("of"), "i:5", &format!("i:{}", i32::MAX)]),
        ),
        (
            "phrase-extend",
            vararg(
                "phrase",
                &[t("the"), call("extend", &[&t("and"), "i:1", "i:0"])],
            ),
        ),
        (
            "fix-field",
            vararg(
                "phrase",
                &[t("the"), call("fixField", &[&lit("title"), &t("alpha")])],
            ),
        ),
        (
            "non-overlapping",
            call(
                "nonOverlapping",
                &[
                    &vararg("unordered", &s(&["the", "of"])),
                    &vararg("unordered", &s(&["and", "to"])),
                ],
            ),
        ),
        (
            "overlapping",
            call(
                "overlapping",
                &[
                    &vararg("unordered", &s(&["the", "of"])),
                    &vararg("unordered", &s(&["and", "of"])),
                ],
            ),
        ),
        (
            "not-within",
            call("notWithin", &[&t("alpha"), "i:2", &t("the")]),
        ),
        ("within", call("within", &[&t("alpha"), "i:3", &t("the")])),
        (
            "not-containing",
            call(
                "notContaining",
                &[&vararg("ordered", &s(&["the", "and"])), &t("of")],
            ),
        ),
        (
            "containing",
            call(
                "containing",
                &[
                    &call(
                        "maxwidth",
                        &["i:10", &vararg("ordered", &s(&["the", "and"]))],
                    ),
                    &t("of"),
                ],
            ),
        ),
        (
            "not-contained-by",
            call(
                "notContainedBy",
                &[&t("of"), &vararg("ordered", &s(&["the", "and"]))],
            ),
        ),
        (
            "contained-by",
            call(
                "containedBy",
                &[
                    &t("of"),
                    &call(
                        "maxwidth",
                        &["i:10", &vararg("ordered", &s(&["the", "and"]))],
                    ),
                ],
            ),
        ),
        (
            "contained-by-or",
            call(
                "containedBy",
                &[
                    &vararg("or", &[t("of"), phrase_terms(&["of", "the"])]),
                    &call(
                        "maxgaps",
                        &[
                            "i:4",
                            &vararg(
                                "or",
                                &[
                                    vararg("ordered", &s(&["the", "and"])),
                                    vararg("ordered", &s(&["to", "the"])),
                                ],
                            ),
                        ],
                    ),
                ],
            ),
        ),
        ("before", call("before", &[&t("alpha"), &t("bravo")])),
        ("after", call("after", &[&t("alpha"), &t("bravo")])),
    ];
    add_sources(&mut o, &sources);
    assert_parity(&o, 200);
}

#[test]
fn intervals_minimum_should_match_analyzed_and_scoring() {
    let index = OracleIndex::standard(SEED, NUM_DOCS);
    let mut o = oracle(&index);
    let standard = expr(
        "new",
        &["org.apache.lucene.analysis.standard.StandardAnalyzer"],
    );
    let sources = vec![
        (
            "at-least-2",
            at_least(2, &s(&["alpha", "bravo", "charlie", "the"])),
        ),
        ("at-least-3", at_least(3, &s(&["the", "of", "and", "to"]))),
        ("at-least-1", at_least(1, &s(&["delta"]))),
        ("at-least-too-few", at_least(4, &s(&["the", "of"]))),
        (
            "at-least-nested",
            at_least(
                2,
                &[vararg("unordered", &s(&["the", "of"])), t("and"), t("to")],
            ),
        ),
        (
            "ordered-at-least",
            vararg(
                "ordered",
                &[at_least(1, &s(&["alpha"])), at_least(1, &s(&["the"]))],
            ),
        ),
        ("no-intervals", call("noIntervals", &[&lit("for a test")])),
        (
            "analyzed-ordered",
            call(
                "analyzedText",
                &[
                    &lit("The alpha, of"),
                    &standard,
                    &lit("body"),
                    "i:-1",
                    "z:true",
                ],
            ),
        ),
        (
            "analyzed-phrase",
            call(
                "analyzedText",
                &[&lit("the of"), &standard, &lit("body"), "i:0", "z:true"],
            ),
        ),
        (
            "analyzed-unordered",
            call(
                "analyzedText",
                &[
                    &lit("of the and"),
                    &standard,
                    &lit("body"),
                    "i:2",
                    "z:false",
                ],
            ),
        ),
        (
            "analyzed-single",
            call(
                "analyzedText",
                &[&lit("bravo"), &standard, &lit("body"), "i:-1", "z:true"],
            ),
        ),
        (
            "analyzed-empty",
            call(
                "analyzedText",
                &[&lit(""), &standard, &lit("body"), "i:-1", "z:true"],
            ),
        ),
    ];
    add_sources(&mut o, &sources);
    let source = vararg("ordered", &[vararg("or", &s(&["the", "of"])), t("and")]);
    o.query("iq-pivot", &query_pivot("body", &source, 2.0));
    o.query("iq-pivot-exp", &query_pivot_exp("body", &source, 1.2, 2.0));
    o.query("iq-pivot-negative", &query_pivot("body", &source, -1.0));
    o.query(
        "iq-exp-negative",
        &query_pivot_exp("body", &source, 1.0, -1.0),
    );
    o.query(
        "iq-boost",
        &expr(
            "new",
            &[
                "org.apache.lucene.search.BoostQuery",
                &query("body", &source),
                "f:2",
            ],
        ),
    );
    assert_parity(&o, 100);
}
