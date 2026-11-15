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

//! The search parity of the query modules' Java-oracle harness (rmp #475),
//! on ported core queries: `TermQuery`, `BooleanQuery` and `PhraseQuery`
//! (with `BoostQuery`, `ConstantScoreQuery`, `DisjunctionMaxQuery`,
//! `MatchAllDocsQuery` and `MatchNoDocsQuery` around them), fixed and seeded,
//! over a seeded index Rucene writes, under `BM25Similarity` and
//! `ClassicSimilarity`. Hits, bit-exact scores, explanations and
//! `toString()` before and after rewrite must equal Lucene 10.5.0's; a
//! perturbed Rust dump must fail at the perturbed query (positive control).

mod query_oracle;

use query_oracle::{compare, expr, lit, OracleIndex, Registry, SearchOracle, VOCABULARY};
use randomizedtesting::jdk::random::{JdkRandom, Random};

/// The seed of the index and of the generated queries.
const SEED: i64 = 0x0475_5eed;

fn term(field: &str, text: &str) -> String {
    expr(
        "new",
        &["org.apache.lucene.index.Term", &lit(field), &lit(text)],
    )
}

fn term_query(field: &str, text: &str) -> String {
    expr(
        "new",
        &["org.apache.lucene.search.TermQuery", &term(field, text)],
    )
}

fn occur(name: &str) -> String {
    expr(
        "field",
        &["org.apache.lucene.search.BooleanClause$Occur", name],
    )
}

/// `new BooleanQuery.Builder()`, an `add` per clause, `setMinimumNumberShouldMatch`
/// when `min > 0`, `build()`.
fn boolean(clauses: &[(String, &str)], min: i32) -> String {
    let mut b = expr("new", &["org.apache.lucene.search.BooleanQuery$Builder"]);
    for (q, o) in clauses {
        b = expr("call", &["add", &b, q, &occur(o)]);
    }
    if min > 0 {
        b = expr(
            "call",
            &["setMinimumNumberShouldMatch", &b, &format!("i:{min}")],
        );
    }
    expr("call", &["build", &b])
}

fn phrase(slop: i32, field: &str, words: &[&str]) -> String {
    let words: Vec<String> = words.iter().map(|w| lit(w)).collect();
    let mut array = vec!["java.lang.String"];
    array.extend(words.iter().map(String::as_str));
    expr(
        "new",
        &[
            "org.apache.lucene.search.PhraseQuery",
            &format!("i:{slop}"),
            &lit(field),
            &expr("array", &array),
        ],
    )
}

/// The fixed queries: each kind alone and combined, matching many, few and
/// no documents, and failing ones.
fn fixed_queries() -> Vec<(String, String)> {
    let mut q: Vec<(&str, String)> = vec![
        ("t-common", term_query("body", "the")),
        ("t-mid", term_query("body", "kilo")),
        ("t-rare", term_query("body", "zulu")),
        ("t-title", term_query("title", "alpha")),
        ("t-tag", term_query("tag", "red")),
        ("t-none", term_query("body", "nonexistent")),
        ("t-nofield", term_query("nofield", "the")),
        (
            "bq-or2",
            boolean(
                &[
                    (term_query("body", "alpha"), "SHOULD"),
                    (term_query("body", "bravo"), "SHOULD"),
                ],
                0,
            ),
        ),
        (
            "bq-or4",
            boolean(
                &[
                    (term_query("body", "the"), "SHOULD"),
                    (term_query("body", "golf"), "SHOULD"),
                    (term_query("body", "mike"), "SHOULD"),
                    (term_query("title", "papa"), "SHOULD"),
                ],
                0,
            ),
        ),
        (
            "bq-and",
            boolean(
                &[
                    (term_query("body", "the"), "MUST"),
                    (term_query("body", "of"), "MUST"),
                ],
                0,
            ),
        ),
        (
            "bq-req-opt",
            boolean(
                &[
                    (term_query("body", "and"), "MUST"),
                    (term_query("body", "echo"), "SHOULD"),
                ],
                0,
            ),
        ),
        (
            "bq-filter-not",
            boolean(
                &[
                    (term_query("body", "to"), "SHOULD"),
                    (term_query("tag", "blue"), "FILTER"),
                    (term_query("cat", "c1"), "MUST_NOT"),
                ],
                0,
            ),
        ),
        (
            "bq-msm",
            boolean(
                &[
                    (term_query("body", "alpha"), "SHOULD"),
                    (term_query("body", "bravo"), "SHOULD"),
                    (term_query("body", "charlie"), "SHOULD"),
                ],
                2,
            ),
        ),
        (
            "bq-nested",
            boolean(
                &[
                    (
                        boolean(
                            &[
                                (term_query("body", "the"), "SHOULD"),
                                (term_query("title", "of"), "SHOULD"),
                            ],
                            0,
                        ),
                        "MUST",
                    ),
                    (term_query("tag", "green"), "SHOULD"),
                    (term_query("body", "zulu"), "MUST_NOT"),
                ],
                0,
            ),
        ),
        (
            "bq-only-not",
            boolean(&[(term_query("body", "the"), "MUST_NOT")], 0),
        ),
        (
            "bq-single",
            boolean(&[(term_query("body", "delta"), "SHOULD")], 0),
        ),
        ("bq-empty", boolean(&[], 0)),
        ("pq-exact", phrase(0, "body", &["the", "of"])),
        ("pq-sloppy", phrase(2, "body", &["the", "and"])),
        ("pq-three", phrase(1, "body", &["of", "the", "to"])),
        ("pq-single", phrase(0, "body", &["alpha"])),
        ("pq-title", phrase(3, "title", &["alpha", "bravo"])),
        ("pq-nopositions", phrase(0, "tag", &["red", "blue"])),
        ("pq-builder-gap", {
            let mut b = expr("new", &["org.apache.lucene.search.PhraseQuery$Builder"]);
            b = expr("call", &["add", &b, &term("body", "the"), "i:0"]);
            b = expr("call", &["add", &b, &term("body", "the"), "i:2"]);
            b = expr("call", &["setSlop", &b, "i:1"]);
            expr("call", &["build", &b])
        }),
        (
            "boost",
            expr(
                "new",
                &[
                    "org.apache.lucene.search.BoostQuery",
                    &term_query("body", "india"),
                    "f:2.5",
                ],
            ),
        ),
        (
            "constant",
            expr(
                "new",
                &[
                    "org.apache.lucene.search.ConstantScoreQuery",
                    &term_query("body", "lima"),
                ],
            ),
        ),
        (
            "dismax",
            expr(
                "new",
                &[
                    "org.apache.lucene.search.DisjunctionMaxQuery",
                    &expr(
                        "list",
                        &[&term_query("body", "oscar"), &term_query("title", "oscar")],
                    ),
                    "f:0.1",
                ],
            ),
        ),
        (
            "match-all",
            expr(
                "field",
                &["org.apache.lucene.search.MatchAllDocsQuery", "INSTANCE"],
            ),
        ),
        (
            "match-none",
            expr(
                "new",
                &[
                    "org.apache.lucene.search.MatchNoDocsQuery",
                    &lit("no reason"),
                ],
            ),
        ),
        (
            "bad-boost",
            expr(
                "new",
                &[
                    "org.apache.lucene.search.BoostQuery",
                    &term_query("body", "the"),
                    "f:-1.0",
                ],
            ),
        ),
    ];
    q.drain(..).map(|(id, e)| (id.to_string(), e)).collect()
}

/// `n` random queries drawn from `seed`: terms, phrases and nested boolean
/// queries over the standard index's fields and words.
fn random_queries(seed: i64, n: usize) -> Vec<(String, String)> {
    let mut random = JdkRandom::new(seed);
    (0..n)
        .map(|i| (format!("r{i}"), random_query(&mut random, 0)))
        .collect()
}

fn random_word(random: &mut JdkRandom) -> &'static str {
    let r = random.next_double();
    VOCABULARY[((r * r) * VOCABULARY.len() as f64) as usize]
}

fn random_query(random: &mut JdkRandom, depth: u32) -> String {
    let field = ["body", "body", "body", "title"][random.next_int_bound(4) as usize];
    match random.next_int_bound(if depth < 2 { 4 } else { 2 }) {
        0 => term_query(field, random_word(random)),
        1 => {
            let n = 1 + random.next_int_bound(3) as usize;
            let words: Vec<&str> = (0..n).map(|_| random_word(random)).collect();
            phrase(random.next_int_bound(3), field, &words)
        }
        _ => {
            let n = 1 + random.next_int_bound(4) as usize;
            let clauses: Vec<(String, &str)> = (0..n)
                .map(|_| {
                    let o = ["SHOULD", "SHOULD", "MUST", "FILTER", "MUST_NOT"]
                        [random.next_int_bound(5) as usize];
                    (random_query(random, depth + 1), o)
                })
                .collect();
            let min = if random.next_int_bound(4) == 0 { 1 } else { 0 };
            boolean(&clauses, min)
        }
    }
}

fn oracle(index: &OracleIndex) -> SearchOracle {
    let mut oracle = SearchOracle::new(index, Registry::core());
    for (id, e) in fixed_queries().into_iter().chain(random_queries(SEED, 60)) {
        oracle.query(&id, &e);
    }
    oracle
}

/// Changes the score bits of the first hit of record `id` in `dump`.
fn perturb_score(dump: &str, id: &str) -> String {
    let mut out = String::new();
    let mut in_record = false;
    let mut done = false;
    for l in dump.lines() {
        if l.starts_with("query\t") {
            in_record = l == format!("query\t{id}");
        }
        if in_record && !done && l.starts_with("hit\t") {
            let mut f: Vec<String> = l.split('\t').map(str::to_string).collect();
            let bits = u32::from_str_radix(&f[2], 16).expect("score bits") ^ 1;
            f[2] = format!("{bits:08x}");
            out.push_str(&f.join("\t"));
            done = true;
        } else {
            out.push_str(l);
        }
        out.push('\n');
    }
    assert!(done, "record {id} has a hit to perturb");
    out
}

#[test]
fn core_queries_match_lucene_under_bm25() {
    let index = OracleIndex::standard(SEED, 60);
    let oracle = oracle(&index);
    let java = oracle.java_dump();
    let rust = oracle.rust_dump();
    let records = java.lines().filter(|l| l.starts_with("query\t")).count();
    let hits = java.lines().filter(|l| l.starts_with("hit\t")).count();
    println!(
        "{records} queries, {hits} hits, {} dump lines",
        java.lines().count()
    );
    assert!(
        records >= 90 && hits > 500,
        "the dump is too small to prove parity"
    );
    if let Err(diff) = compare(&java, &rust) {
        panic!("search parity with Lucene 10.5.0 fails:\n{diff}");
    }

    // Positive control: one flipped score bit fails at its query.
    let perturbed = perturb_score(&rust, "bq-or4");
    let clean = compare(&java, &perturbed);
    assert!(clean.is_err(), "a perturbed score must fail the comparison");
    let message = clean.unwrap_err();
    assert!(
        message.contains("record `query\tbq-or4`"),
        "the failure names the perturbed query: {message}"
    );
}

#[test]
fn core_queries_match_lucene_under_classic_similarity() {
    let index = OracleIndex::standard(SEED ^ 0x5a5a, 45);
    let mut oracle = oracle(&index);
    oracle.similarity(&expr(
        "new",
        &["org.apache.lucene.search.similarities.ClassicSimilarity"],
    ));
    oracle.assert_matches();
}
