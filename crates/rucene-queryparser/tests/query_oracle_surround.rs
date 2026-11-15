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

//! The parity of the surround query parser (rmp #475, #492) against Lucene
//! 10.5.0:
//!
//! * **Search parity** (`surround_queries_search_like_lucene_<n>`): each
//!   query string of a seeded corpus (valid and invalid: terms,
//!   truncations, `AND`/`OR`/`NOT` in infix and prefix form, `W`/`N`
//!   distances, nesting, fields, weights, and syntax errors) is parsed with
//!   the surround `QueryParser`, made into a Lucene query with
//!   `makeLuceneQueryField("body", new BasicQueryFactory(n))` and searched
//!   over the harness's standard index: the same `toString()`, class tree,
//!   rewritten query, count, hits with the bits of their scores, and
//!   explanations — or the same exception class and message of the stage
//!   that throws (`ParseException`, `TooManyBasicQueries`, ...).
//! * **Parse parity** (`surround_parser_matches_lucene`): the `SrndQuery`
//!   itself, `toString()` of the parsed tree or the exception.

#[path = "../../rucene-queries/tests/query_oracle/mod.rs"]
mod query_oracle;

use std::sync::Arc;

use query_oracle::{
    compare, expr, lit, OracleIndex, ParseOracle, Registry, SearchOracle, Value, VOCABULARY,
};
use randomizedtesting::jdk::random::{JdkRandom, Random};
use rucene_queryparser::surround::parser::query_parser::QueryParser;
use rucene_queryparser::surround::query::basic_query_factory::BasicQueryFactory;
use rucene_queryparser::surround::query::distance_query::DistanceQuery;
use rucene_queryparser::surround::query::or_query::OrQuery;
use rucene_queryparser::surround::query::srnd_query::SrndQuery;

const SEED: i64 = 0x0492_5352;

const SQP: &str = "org.apache.lucene.queryparser.surround.parser.QueryParser";
const BQF: &str = "org.apache.lucene.queryparser.surround.query.BasicQueryFactory";
const OR_QUERY: &str = "org.apache.lucene.queryparser.surround.query.OrQuery";
const DISTANCE_QUERY: &str = "org.apache.lucene.queryparser.surround.query.DistanceQuery";

/// The concrete `SrndQuery` classes the parser builds.
const SRND_CLASSES: [&str; 8] = [
    "org.apache.lucene.queryparser.surround.query.AndQuery",
    "org.apache.lucene.queryparser.surround.query.OrQuery",
    "org.apache.lucene.queryparser.surround.query.NotQuery",
    "org.apache.lucene.queryparser.surround.query.DistanceQuery",
    "org.apache.lucene.queryparser.surround.query.FieldsQuery",
    "org.apache.lucene.queryparser.surround.query.SrndTermQuery",
    "org.apache.lucene.queryparser.surround.query.SrndPrefixQuery",
    "org.apache.lucene.queryparser.surround.query.SrndTruncQuery",
];

fn registry() -> Registry {
    let mut r = Registry::core();
    r.static_fn(SQP, "parse", 1, |a| {
        let q = QueryParser::parse(a[0].as_str())?;
        Ok(Value::object(q.java_class_name(), q))
    });
    r.new_fn(BQF, 1, |a| {
        Ok(Value::object(
            BQF,
            Arc::new(BasicQueryFactory::with_max_basic_queries(a[0].as_int())),
        ))
    });
    // The public constructors, for trees the parser never builds.
    r.new_fn(OR_QUERY, 3, |a| {
        let q: Arc<dyn SrndQuery> = Arc::new(OrQuery::new(
            srnd_queries(&a[0]),
            a[1].as_bool(),
            a[2].as_str(),
        )?);
        Ok(Value::object(OR_QUERY, q))
    });
    r.new_fn(DISTANCE_QUERY, 5, |a| {
        let q: Arc<dyn SrndQuery> = Arc::new(DistanceQuery::new(
            srnd_queries(&a[0]),
            a[1].as_bool(),
            a[2].as_int(),
            a[3].as_str(),
            a[4].as_bool(),
        )?);
        Ok(Value::object(DISTANCE_QUERY, q))
    });
    for class in SRND_CLASSES {
        r.call_fn(class, "makeLuceneQueryField", 2, |q, a| {
            let q = q.get::<Arc<dyn SrndQuery>>();
            let qf = a[1].get::<Arc<BasicQueryFactory>>();
            Ok(Value::query(
                q.make_lucene_query_field(a[0].as_opt_str(), &qf)?,
            ))
        });
        r.call_fn(class, "toString", 0, |q, _| {
            Ok(Value::Str(q.get::<Arc<dyn SrndQuery>>().srnd_to_string()))
        });
    }
    r
}

/// The `List<SrndQuery>` argument of a surround constructor.
fn srnd_queries(list: &Value) -> Vec<Arc<dyn SrndQuery>> {
    list.as_values()
        .iter()
        .map(|v| v.get::<Arc<dyn SrndQuery>>())
        .collect()
}

/// Fixed edge cases: the queries of Lucene's surround tests, operators
/// alone, malformed distances and groups.
fn surround_edge_cases() -> Vec<String> {
    [
        "alpha", "alpha*", "al?ha", "a*", "ab*", "alp*a", "w?rd?", "al*?a", "\"alpha\"",
        "\"alpha bravo\"", "alpha AND bravo", "alpha and bravo", "and(alpha, bravo)",
        "AND(alpha, bravo, charlie)", "alpha OR bravo", "OR(alpha, bravo)", "alpha NOT bravo",
        "alpha* not b*", "(alpha or the) and or(bravo, xyz, defg)",
        "or( alpha* not b*, and(xyz,def))", "alpha w bravo", "alpha n bravo", "bravo w alpha",
        "alpha 2W bravo", "alpha 2N bravo", "alpha 3w bravo", "alpha 3n bravo", "alpha 99n b*",
        "W(alpha, bravo, charlie)", "N(alpha, bravo, charlie)", "2W(alpha,bravo,charlie)",
        "2N(alpha, bravo ,charlie)", "alpha W bravo W charlie", "alpha 2N bravo 2N charlie",
        "(alpha OR bravo) W charlie*", "(alpha OR bravo) 2W (charlie OR delta)",
        "50n((alpha w bravo*) or charlie*, 5n(delta*, (echo* or (foxtrot* 3n golf*))), hotel* or india*)",
        "body:alpha", "title:alpha", "body:(alpha OR bravo)", "body, title: alpha",
        "body:alpha W bravo", "alpha^2", "(alpha OR bravo)^0.5", "alpha W bravo^3", "*", "?",
        "a?", "a???b", "a*b?", "word1 word2", "alpha AND", "alpha OR", "AND(alpha)",
        "AND(alpha,)", "OR(alpha , bravo ,)", "xx NOT", "xx (a AND b)", "(a AND b", "a OR b)",
        "or(alpha+ not bravo+, and xyz,def)", "", " ", "(aa and bb) w cc",
        "(aa or bb) w (cc and dd)", "(aa not bb) w cc", "(aa or bb) w (body:cc)",
        "(aa or body:bb) w cc", "100W(alpha, bravo)", "0N(alpha, bravo)", "1W alpha",
        "alpha 1W bravo", "3W(alpha* , b?avo, \"charlie\")", "alpha^", "alpha^x", "body:",
        ":alpha", "\"unterminated", "über", "ΟΔΟΣ",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn pick<'a>(random: &mut JdkRandom, items: &[&'a str]) -> &'a str {
    items[random.next_int_bound(items.len() as i32) as usize]
}

fn random_term(random: &mut JdkRandom) -> String {
    let w = pick(random, &VOCABULARY);
    match random.next_int_bound(8) {
        0 => format!("{}*", &w[..1.max(w.len().min(3))]),
        1 => w.replacen(&w[1..2], "?", 1),
        2 => format!("{}*{}", &w[..1], &w[w.len() - 1..]),
        _ => w.to_string(),
    }
}

fn random_query(random: &mut JdkRandom, depth: u32) -> String {
    match random.next_int_bound(if depth < 2 { 7 } else { 3 }) {
        0 | 1 => random_term(random),
        2 => format!("\"{}\"", pick(random, &VOCABULARY)),
        3 => {
            let op = pick(
                random,
                &[" AND ", " OR ", " NOT ", " W ", " N ", " 2W ", " 3N "],
            );
            format!(
                "{}{op}{}",
                random_query(random, depth + 1),
                random_query(random, depth + 1)
            )
        }
        4 => {
            let op = pick(random, &["AND", "OR", "W", "N", "2N", "3W"]);
            let n = 2 + random.next_int_bound(2);
            let args: Vec<String> = (0..n).map(|_| random_query(random, depth + 1)).collect();
            format!("{op}({})", args.join(", "))
        }
        5 => format!("({})", random_query(random, depth + 1)),
        _ => format!(
            "{}:{}",
            pick(random, &["body", "title", "body, title"]),
            random_query(random, depth + 1)
        ),
    }
}

fn surround_random(seed: i64, n: usize) -> Vec<String> {
    let mut random = JdkRandom::new(seed);
    (0..n)
        .map(|_| {
            let q = random_query(&mut random, 0);
            if random.next_int_bound(6) == 0 {
                format!("({q})^{}", 1 + random.next_int_bound(3))
            } else {
                q
            }
        })
        .collect()
}

fn search_shard(shard: i64) {
    let index = OracleIndex::standard(SEED, 60);
    let mut oracle = SearchOracle::new(&index, registry());
    let inputs: Vec<String> = if shard == 0 {
        surround_edge_cases()
    } else {
        surround_random(SEED + shard, 60)
    };
    let mut random = JdkRandom::new(SEED ^ shard);
    for (i, input) in inputs.iter().enumerate() {
        // Small factories exercise `TooManyBasicQueries`.
        let max = pick(&mut random, &["i:1024", "i:16", "i:4"]);
        let field = if random.next_int_bound(4) == 0 {
            "title"
        } else {
            "body"
        };
        let q = expr(
            "call",
            &[
                "makeLuceneQueryField",
                &expr("static", &[SQP, "parse", &lit(input)]),
                &lit(field),
                &expr("new", &[BQF, max]),
            ],
        );
        oracle.query(&format!("q{i}"), &q);
    }
    let java = oracle.java_dump();
    let rust = oracle.rust_dump();
    let errors = java.lines().filter(|l| l.starts_with("error\t")).count();
    let hits = java.lines().filter(|l| l.starts_with("hit\t")).count();
    println!(
        "surround shard {shard}: {} queries, {errors} exceptions, {hits} hits, {} dump lines",
        inputs.len(),
        java.lines().count()
    );
    assert!(hits > 0, "the queries must match documents");
    if let Err(diff) = compare(&java, &rust) {
        panic!("surround search parity with Lucene 10.5.0 fails:\n{diff}");
    }
}

#[test]
fn surround_queries_search_like_lucene_0() {
    search_shard(0);
}

#[test]
fn surround_queries_search_like_lucene_1() {
    search_shard(1);
}

#[test]
fn surround_queries_search_like_lucene_2() {
    search_shard(2);
}

/// The `ClassCastException` of the casts `(DistanceSubQuery) q` in
/// `OrQuery.addSpanQueries` (`OrQuery.java:60`) and
/// `DistanceQuery.getSpanNearQuery` (`DistanceQuery.java:81,86`). The parser
/// rejects such trees (`QueryParser.checkDistanceSubQueries`), so they are
/// built with the public constructors: a distance query whose subquery is an
/// `OrQuery` holding a `NotQuery`, and one whose subquery is the `NotQuery`
/// itself. Both throw when the rewrite builds the span query, with JDK 25's
/// message and its module suffix.
#[test]
fn surround_distance_cast_failures_match_lucene() {
    let index = OracleIndex::standard(SEED, 60);
    let mut oracle = SearchOracle::new(&index, registry());
    let parse = |q: &str| expr("static", &[SQP, "parse", &lit(q)]);
    let or_with_not = expr(
        "new",
        &[
            OR_QUERY,
            &expr("list", &[&parse("alpha NOT bravo"), &parse("charlie")]),
            "z:true",
            &lit("OR"),
        ],
    );
    let subqueries = [
        expr("list", &[&or_with_not, &parse("delta")]),
        expr("list", &[&parse("alpha NOT bravo"), &parse("delta")]),
        expr("list", &[&parse("delta"), &parse("alpha NOT bravo")]),
    ];
    for (i, list) in subqueries.iter().enumerate() {
        let distance = expr(
            "new",
            &[DISTANCE_QUERY, list, "z:true", "i:1", &lit("W"), "z:true"],
        );
        let q = expr(
            "call",
            &[
                "makeLuceneQueryField",
                &distance,
                &lit("body"),
                &expr("new", &[BQF, "i:1024"]),
            ],
        );
        oracle.query(&format!("c{i}"), &q);
    }
    let java = oracle.java_dump();
    let rust = oracle.rust_dump();
    let casts = java
        .lines()
        .filter(|l| l.starts_with("error\t") && l.contains("java.lang.ClassCastException"))
        .count();
    assert_eq!(casts, 3, "each tree must throw the cast exception:\n{java}");
    if let Err(diff) = compare(&java, &rust) {
        panic!("surround cast-failure parity with Lucene 10.5.0 fails:\n{diff}");
    }
}

#[test]
fn surround_parser_matches_lucene() {
    let mut oracle = ParseOracle::new(registry());
    let inputs: Vec<String> = surround_edge_cases()
        .into_iter()
        .chain(surround_random(SEED + 99, 200))
        .collect();
    for (i, input) in inputs.iter().enumerate() {
        let units: Vec<u16> = input.encode_utf16().collect();
        oracle.parse(
            &format!("p{i}"),
            &expr("static", &[SQP, "parse", "$input"]),
            &units,
            None,
        );
    }
    let java = oracle.java_dump();
    let rust = oracle.rust_dump();
    let errors = java.lines().filter(|l| l.starts_with("error\t")).count();
    println!(
        "surround parse: {} strings, {errors} exceptions",
        inputs.len()
    );
    assert!(
        errors > 20 && errors < inputs.len(),
        "the corpus must parse and fail"
    );
    if let Err(diff) = compare(&java, &rust) {
        panic!("surround parse parity with Lucene 10.5.0 fails:\n{diff}");
    }
}
