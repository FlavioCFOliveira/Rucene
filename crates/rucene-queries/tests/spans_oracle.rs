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

//! The Java-oracle portability test of `queries.spans` (rmp #480): every
//! span query of the package — `SpanTermQuery`, `SpanOrQuery`,
//! `SpanNearQuery` (ordered, unordered, and its builder with gaps),
//! `SpanNotQuery` (its three constructors), `SpanFirstQuery`,
//! `SpanPositionRangeQuery`, `SpanContainingQuery`, `SpanWithinQuery`,
//! `FieldMaskingSpanQuery` and `SpanMultiTermQueryWrapper` (scoring and
//! top-terms rewrites) — fixed and seeded, nested, inside boolean and boost
//! queries, over a seeded index Rucene writes (`OracleIndex::standard`:
//! three segments, deleted documents, a field without positions).
//!
//! For each query Lucene 10.5.0 and Rucene must give the same `toString()`
//! and class tree before and after rewrite, count, hits with bit-exact
//! `float` scores, `explain` of every document, and the same spans (per leaf,
//! every document, every span's start, end and width, and every term,
//! position and payload `collect` reports); edge cases (no positions,
//! mismatched fields, empty and absent clauses, negative and extreme slops
//! and windows) give the same exception class and message.
//!
//! The comparison is sharded over several tests so nextest runs them in
//! parallel; each writes its own index and runs its own JVM in its own
//! directory under `TMPDIR`.

mod query_oracle;
mod span_oracle;

use query_oracle::{compare, expr, OracleIndex, SearchOracle, VOCABULARY};
use randomizedtesting::jdk::random::{JdkRandom, Random};
use span_oracle::{
    boolean, boost, fuzzy, fuzzy_full, perturb, prefix, regexp, registry, scontaining, sfirst,
    smask, smtq, snear, snear_builder, snot, snot_dist, snot_pre_post, sor, spans_dumper, srange,
    st, swithin, wildcard, NearStep,
};

/// The seed of the index and of the generated queries.
const SEED: i64 = 0x0480_5eed;

/// The number of documents of the index.
const NUM_DOCS: usize = 60;

fn b(text: &str) -> String {
    st("body", text)
}

/// The oracle over the standard index with the span bindings and dumper.
fn oracle(index: &OracleIndex) -> SearchOracle {
    let mut oracle = SearchOracle::new(index, registry());
    oracle.spans_dumper(spans_dumper());
    oracle
}

/// A query record for each query, and a spans record (`postings`) for each
/// span query (the `boost-` and `bool-` queries wrap span queries in core
/// queries, which have no spans).
fn add_all(oracle: &mut SearchOracle, queries: &[(String, String)], postings: &str) {
    for (id, e) in queries {
        oracle.query(id, e);
        if !id.starts_with("boost-") && !id.starts_with("bool-") {
            oracle.spans(id, e, postings);
        }
    }
}

/// The records of a dump that start with `kind`.
fn count_records(dump: &str, kind: &str) -> usize {
    dump.lines()
        .filter(|l| l.starts_with(&format!("{kind}\t")))
        .count()
}

/// Runs both sides, prints the dump's size and panics with the first
/// difference; `min_spans` guards against a vacuous comparison.
fn assert_parity(oracle: &SearchOracle, min_spans: usize) -> (String, String) {
    let java = oracle.java_dump();
    let rust = oracle.rust_dump();
    let spans = count_records(&java, "span");
    println!(
        "{} queries, {} spans records, {} hits, {} spans, {} dump lines",
        count_records(&java, "query"),
        count_records(&java, "spans"),
        count_records(&java, "hit"),
        spans,
        java.lines().count()
    );
    assert!(
        spans >= min_spans,
        "the dump has {spans} spans, too few to prove parity"
    );
    if let Err(diff) = compare(&java, &rust) {
        panic!("span parity with Lucene 10.5.0 fails:\n{diff}");
    }
    (java, rust)
}

fn q(id: &str, e: String) -> (String, String) {
    (id.to_string(), e)
}

/// `SpanTermQuery`, `SpanOrQuery` and `SpanNearQuery`, fixed.
fn term_or_near_queries() -> Vec<(String, String)> {
    vec![
        q("st-common", b("the")),
        q("st-rare", b("zulu")),
        q("st-none", b("nonexistent")),
        q("st-nofield", st("nofield", "the")),
        q("st-title", st("title", "alpha")),
        q("st-nopositions", st("tag", "red")),
        q("so-2", sor(&[b("alpha"), b("bravo")])),
        q("so-3", sor(&[b("the"), b("of"), b("and")])),
        q("so-1", sor(&[b("delta")])),
        q("so-dup", sor(&[b("the"), b("the")])),
        q("so-empty", sor(&[])),
        q("so-none", sor(&[b("nonexistent"), b("missing")])),
        q("so-mixed", sor(&[b("the"), b("nonexistent")])),
        q(
            "so-nested",
            sor(&[snear(&[b("the"), b("of")], 0, true), b("alpha")]),
        ),
        q("so-field-mismatch", sor(&[b("the"), st("title", "the")])),
        q("sn-o0", snear(&[b("the"), b("of")], 0, true)),
        q("sn-o2", snear(&[b("the"), b("and")], 2, true)),
        q("sn-u0", snear(&[b("of"), b("the")], 0, false)),
        q(
            "sn-u3",
            snear(&[b("alpha"), b("bravo"), b("charlie")], 3, false),
        ),
        q("sn-o3terms", snear(&[b("the"), b("of"), b("and")], 5, true)),
        q("sn-same-o", snear(&[b("the"), b("the")], 1, true)),
        q("sn-same-u", snear(&[b("of"), b("of")], 2, false)),
        q("sn-neg", snear(&[b("the"), b("of")], -1, false)),
        q("sn-max", snear(&[b("the"), b("zulu")], i32::MAX, false)),
        q(
            "sn-nested",
            snear(
                &[
                    snear(&[b("the"), b("of")], 1, true),
                    sor(&[b("and"), b("to")]),
                ],
                3,
                true,
            ),
        ),
        q("sn-single", snear(&[b("the")], 0, true)),
        q("sn-none", snear(&[b("the"), b("nonexistent")], 5, true)),
        q(
            "sn-title",
            snear(&[st("title", "the"), st("title", "of")], 4, false),
        ),
        q(
            "sn-nopositions",
            snear(&[st("tag", "red"), st("tag", "blue")], 2, true),
        ),
        q(
            "sn-field-mismatch",
            snear(&[b("the"), st("title", "of")], 2, true),
        ),
        q(
            "sn-builder-gap",
            snear_builder(
                "body",
                true,
                &[
                    NearStep::Clause(b("the")),
                    NearStep::Gap(1),
                    NearStep::Clause(b("of")),
                    NearStep::Slop(1),
                ],
            ),
        ),
        q(
            "sn-builder-gaps",
            snear_builder(
                "body",
                true,
                &[
                    NearStep::Clause(b("alpha")),
                    NearStep::Gap(2),
                    NearStep::Clause(b("the")),
                    NearStep::Gap(1),
                    NearStep::Clause(b("of")),
                    NearStep::Slop(2),
                ],
            ),
        ),
        q(
            "sn-builder-unordered",
            snear_builder(
                "body",
                false,
                &[
                    NearStep::Clause(b("the")),
                    NearStep::Clause(b("and")),
                    NearStep::Slop(2),
                ],
            ),
        ),
        q(
            "sn-builder-unordered-gap",
            snear_builder("body", false, &[NearStep::Gap(1)]),
        ),
        q(
            "sn-builder-field-mismatch",
            snear_builder("body", true, &[NearStep::Clause(st("title", "the"))]),
        ),
        q(
            "sn-builder-nested-gap",
            snear_builder(
                "body",
                true,
                &[
                    NearStep::Clause(sor(&[
                        b("open"),
                        snear_builder(
                            "body",
                            true,
                            &[NearStep::Clause(b("the")), NearStep::Gap(1)],
                        ),
                    ])),
                    NearStep::Clause(b("of")),
                ],
            ),
        ),
    ]
}

/// `SpanNotQuery`, `SpanFirstQuery`, `SpanPositionRangeQuery`, the contain
/// queries, `FieldMaskingSpanQuery`, `SpanMultiTermQueryWrapper` and span
/// queries inside core queries, fixed.
fn not_first_contain_queries() -> Vec<(String, String)> {
    vec![
        q("snot-basic", snot(&b("the"), &b("of"))),
        q(
            "snot-near",
            snot(&snear(&[b("the"), b("and")], 3, true), &b("of")),
        ),
        q("snot-dist", snot_dist(&b("the"), &b("of"), 1)),
        q("snot-pre", snot_pre_post(&b("and"), &b("the"), 2, 0)),
        q("snot-post", snot_pre_post(&b("and"), &b("the"), 0, 2)),
        q(
            "snot-max",
            snot_pre_post(
                &snear(&[b("alpha"), b("bravo")], 4, true),
                &b("charlie"),
                i32::MAX,
                i32::MAX,
            ),
        ),
        q(
            "snot-neg",
            snot_pre_post(&snear(&[b("the"), b("of")], 2, true), &b("and"), -1, -2),
        ),
        q("snot-none-exclude", snot(&b("the"), &b("nonexistent"))),
        q("snot-none-include", snot(&b("nonexistent"), &b("the"))),
        q("snot-same", snot(&b("the"), &b("the"))),
        q("snot-field-mismatch", snot(&b("the"), &st("title", "of"))),
        q("sf-1", sfirst(&b("the"), 1)),
        q("sf-near", sfirst(&snear(&[b("the"), b("of")], 0, true), 3)),
        q("sf-0", sfirst(&b("alpha"), 0)),
        q("sf-max", sfirst(&b("of"), i32::MAX)),
        q("sr-1-2", srange(&b("the"), 1, 2)),
        q("sr-or", srange(&sor(&[b("alpha"), b("bravo")]), 0, 5)),
        q("sr-reversed", srange(&b("the"), 5, 3)),
        q(
            "sr-near",
            srange(&snear(&[b("the"), b("of")], 2, false), 2, 10),
        ),
        q(
            "sc-1",
            scontaining(&snear(&[b("the"), b("and")], 5, true), &b("of")),
        ),
        q(
            "sw-1",
            swithin(&snear(&[b("the"), b("and")], 5, true), &b("of")),
        ),
        q("sc-same", scontaining(&b("the"), &b("the"))),
        q("sw-same", swithin(&b("the"), &b("the"))),
        q(
            "sc-or",
            scontaining(
                &snear(&[b("the"), b("of")], 3, false),
                &sor(&[b("and"), b("to")]),
            ),
        ),
        q(
            "sw-nested",
            swithin(
                &snear(&[b("alpha"), b("bravo")], 10, false),
                &sor(&[b("charlie"), b("delta")]),
            ),
        ),
        q(
            "sc-field-mismatch",
            scontaining(&b("the"), &st("title", "of")),
        ),
        q(
            "fm-near",
            snear(&[b("the"), smask(&st("title", "alpha"), "body")], 3, false),
        ),
        q("fm-or", sor(&[b("the"), smask(&st("title", "of"), "body")])),
        q("fm-self", smask(&b("the"), "body")),
        q("fm-other", smask(&st("title", "alpha"), "body")),
        q("mtq-wildcard", smtq(&wildcard("body", "ech*"))),
        q("mtq-wildcard-q", smtq(&wildcard("body", "?lpha"))),
        q("mtq-prefix", smtq(&prefix("body", "t"))),
        q("mtq-regexp", smtq(&regexp("body", "[a-c].*"))),
        q("mtq-fuzzy", smtq(&fuzzy("body", "alhpa"))),
        q(
            "mtq-fuzzy-1",
            smtq(&fuzzy_full("body", "bravo", 1, 0, 1, false)),
        ),
        q("mtq-none", smtq(&prefix("body", "nosuch"))),
        q(
            "mtq-near",
            snear(
                &[smtq(&prefix("body", "the")), smtq(&wildcard("body", "o?"))],
                1,
                true,
            ),
        ),
        q("mtq-first", sfirst(&smtq(&wildcard("body", "*o")), 3)),
        q(
            "boost-span",
            boost(&snear(&[b("the"), b("of")], 1, true), 2.5),
        ),
        q(
            "bool-span",
            boolean(&[
                (snear(&[b("the"), b("of")], 1, true), "SHOULD"),
                (b("and"), "MUST"),
                (sfirst(&b("to"), 2), "SHOULD"),
            ]),
        ),
        q(
            "bool-span-not",
            boolean(&[
                (sor(&[b("alpha"), b("bravo")]), "SHOULD"),
                (snot(&b("the"), &b("of")), "MUST_NOT"),
            ]),
        ),
    ]
}

/// A word of [`VOCABULARY`], low ranks more frequent (the index's own
/// distribution).
fn word(random: &mut JdkRandom) -> &'static str {
    let r = random.next_double();
    VOCABULARY[((r * r) * VOCABULARY.len() as f64) as usize]
}

/// A random span query over `body`, nested up to `depth`.
fn random_span(random: &mut JdkRandom, depth: u32) -> String {
    let kinds = if depth < 2 { 8 } else { 1 };
    match random.next_int_bound(kinds) {
        0 => b(word(random)),
        1 => {
            let n = 1 + random.next_int_bound(3) as usize;
            let clauses: Vec<String> = (0..n).map(|_| random_span(random, depth + 1)).collect();
            sor(&clauses)
        }
        2 | 3 => {
            let n = 2 + random.next_int_bound(2) as usize;
            let clauses: Vec<String> = (0..n).map(|_| random_span(random, depth + 1)).collect();
            let slop = random.next_int_bound(6) - 1;
            let ordered = random.next_boolean();
            snear(&clauses, slop, ordered)
        }
        4 => {
            let include = random_span(random, depth + 1);
            let exclude = random_span(random, depth + 1);
            let pre = random.next_int_bound(5) - 1;
            let post = random.next_int_bound(5) - 1;
            snot_pre_post(&include, &exclude, pre, post)
        }
        5 => {
            let m = random_span(random, depth + 1);
            sfirst(&m, 1 + random.next_int_bound(10))
        }
        6 => {
            let m = random_span(random, depth + 1);
            let start = random.next_int_bound(6);
            let end = start + random.next_int_bound(9);
            srange(&m, start, end)
        }
        _ => {
            let big = snear(
                &[
                    random_span(random, depth + 1),
                    random_span(random, depth + 1),
                ],
                random.next_int_bound(8),
                random.next_boolean(),
            );
            let little = random_span(random, depth + 1);
            if random.next_boolean() {
                scontaining(&big, &little)
            } else {
                swithin(&big, &little)
            }
        }
    }
}

/// `n` random span queries drawn from `seed`.
fn random_queries(seed: i64, n: usize) -> Vec<(String, String)> {
    let mut random = JdkRandom::new(seed);
    (0..n)
        .map(|i| (format!("r{i}"), random_span(&mut random, 0)))
        .collect()
}

#[test]
fn span_term_or_near_match_lucene() {
    let index = OracleIndex::standard(SEED, NUM_DOCS);
    let mut oracle = oracle(&index);
    add_all(&mut oracle, &term_or_near_queries(), "POSITIONS");
    assert_parity(&oracle, 300);
}

#[test]
fn span_not_first_contain_mask_multiterm_match_lucene() {
    let index = OracleIndex::standard(SEED, NUM_DOCS);
    let mut oracle = oracle(&index);
    add_all(&mut oracle, &not_first_contain_queries(), "POSITIONS");
    assert_parity(&oracle, 300);
}

#[test]
fn span_random_queries_match_lucene_under_bm25() {
    let index = OracleIndex::standard(SEED, NUM_DOCS);
    let mut oracle = oracle(&index);
    add_all(&mut oracle, &random_queries(SEED, 50), "POSITIONS");
    assert_parity(&oracle, 200);
}

#[test]
fn span_random_queries_match_lucene_under_classic_similarity() {
    let index = OracleIndex::standard(SEED ^ 0x5a5a, 45);
    let mut oracle = oracle(&index);
    oracle.similarity(&expr(
        "new",
        &["org.apache.lucene.search.similarities.ClassicSimilarity"],
    ));
    let mut queries = random_queries(SEED ^ 0x77, 40);
    queries.extend(
        term_or_near_queries()
            .into_iter()
            .filter(|(id, _)| id.starts_with("sn-") || id.starts_with("so-")),
    );
    add_all(&mut oracle, &queries, "POSITIONS");
    assert_parity(&oracle, 200);
}

/// The spans with the other postings constants: `PAYLOADS` (no payload
/// indexed: every `collect` reports `null`) and `OFFSETS`.
#[test]
fn span_postings_payloads_and_offsets_match_lucene() {
    let index = OracleIndex::standard(SEED, NUM_DOCS);
    let mut oracle = oracle(&index);
    let queries: Vec<(String, String)> = term_or_near_queries()
        .into_iter()
        .chain(not_first_contain_queries())
        .filter(|(id, _)| {
            [
                "st-common",
                "so-3",
                "sn-o2",
                "sn-u3",
                "snot-near",
                "sf-near",
                "sc-1",
                "sw-1",
            ]
            .contains(&id.as_str())
        })
        .collect();
    for (id, e) in &queries {
        oracle.spans(&format!("{id}-payloads"), e, "PAYLOADS");
        oracle.spans(&format!("{id}-offsets"), e, "OFFSETS");
    }
    assert_parity(&oracle, 100);
}

/// Positive control: one changed span line, or one changed score bit, fails
/// the comparison at its record.
#[test]
fn span_oracle_detects_a_perturbed_dump() {
    let index = OracleIndex::standard(SEED, NUM_DOCS);
    let mut oracle = oracle(&index);
    let queries = vec![
        q("sn-o2", snear(&[b("the"), b("and")], 2, true)),
        q(
            "sw-1",
            swithin(&snear(&[b("the"), b("and")], 5, true), &b("of")),
        ),
    ];
    add_all(&mut oracle, &queries, "POSITIONS");
    let (java, rust) = assert_parity(&oracle, 5);

    let perturbed = perturb(&rust, "spans\tsn-o2", "span\t");
    let diff = compare(&java, &perturbed).expect_err("a perturbed span must fail");
    assert!(
        diff.contains("record `spans\tsn-o2`"),
        "the failure names the perturbed record: {diff}"
    );
    let perturbed = perturb(&rust, "query\tsw-1", "hit\t");
    let diff = compare(&java, &perturbed).expect_err("a perturbed hit must fail");
    assert!(
        diff.contains("record `query\tsw-1`"),
        "the failure names the perturbed record: {diff}"
    );
}
