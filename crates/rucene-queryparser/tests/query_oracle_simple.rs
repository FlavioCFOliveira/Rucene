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

//! The parser parity of `SimpleQueryParser` (rmp #475, #485): Lucene 10.5.0
//! and Rucene parse a seeded corpus of query strings (the edge cases of
//! Lucene's `TestSimpleQueryParser`, the classic corpus, and strings drawn
//! from the simple syntax's operator characters) under **every one of the
//! 1024 operator-flag combinations**, with a random default operator and
//! one or two weighted fields, and give the same `toString()`, class tree,
//! or exception class and message.
//!
//! The flag space is sharded over four `#[test]` functions (256
//! combinations each) so nextest runs them in parallel.

#[path = "../../rucene-queries/tests/query_oracle/mod.rs"]
mod query_oracle;

use std::sync::Arc;

use query_oracle::{compare, corpus, expr, lit, ParseOracle, Registry, Value};
use randomizedtesting::jdk::random::{JdkRandom, Random};
use rucene::analysis::Analyzer;
use rucene::search::boolean_clause::Occur;
use rucene_queryparser::simple::simple_query_parser::{SimpleQueryParser, SimpleQueryParserOps};

const SEED: i64 = 0x0485_5149;

const SQP: &str = "org.apache.lucene.queryparser.simple.SimpleQueryParser";
const LINKED_HASH_MAP: &str = "java.util.LinkedHashMap";
const OCCUR: &str = "org.apache.lucene.search.BooleanClause$Occur";

/// An ordered `Map<String, Float>`: Java's `LinkedHashMap`.
#[derive(Clone, Default)]
struct Weights(Vec<(String, f32)>);

fn registry() -> Registry {
    let mut r = Registry::core();
    r.new_fn(LINKED_HASH_MAP, 0, |_| {
        Ok(Value::object(LINKED_HASH_MAP, Weights::default()))
    });
    r.call_fn(LINKED_HASH_MAP, "put", 2, |m, a| {
        let k = a[0].as_str().to_string();
        let v = a[1].as_float();
        let old = m.with_mut(
            |m: &mut Weights| match m.0.iter_mut().find(|(key, _)| *key == k) {
                Some(entry) => Some(std::mem::replace(&mut entry.1, v)),
                None => {
                    m.0.push((k, v));
                    None
                }
            },
        );
        Ok(old.map_or(Value::Null, Value::Float))
    });
    r.new_fn(SQP, 3, |a| {
        let analyzer = match &a[0] {
            Value::Null => None,
            v => Some(v.get::<Arc<dyn Analyzer>>()),
        };
        let weights = a[1].get::<Weights>().0;
        Ok(Value::object(
            SQP,
            SimpleQueryParser::with_flags(analyzer, weights, a[2].as_int()),
        ))
    });
    r.call_fn(SQP, "setDefaultOperator", 1, |p, a| {
        let op = a[0].get::<Occur>();
        p.with_mut(|p: &mut SimpleQueryParser| p.set_default_operator(op))?;
        Ok(Value::Null)
    });
    r.call_fn(SQP, "parse", 1, |p, a| {
        let input = a[0].as_str().to_string();
        let q = p.with_mut(|p: &mut SimpleQueryParser| p.parse(&input))?;
        Ok(Value::query(q))
    });
    r
}

/// The edge cases of `TestSimpleQueryParser` and the simple syntax.
fn simple_edge_cases() -> Vec<String> {
    [
        "foobar~2",
        "foobar~",
        "foobar~a",
        "foobar~1a",
        "foo~3 bar",
        "\"foo bar\"~2",
        "\"foo bar\"~10",
        "\"foo bar\"~",
        "\"foo bar\"~a",
        "\"foo bar\"~-1",
        "\"foo bar\"~12 baz",
        "foobar*",
        "foo+bar",
        "\"foo bar\"+\"star wars\"",
        "foo|bar",
        "foo||bar",
        "-foo",
        "-(foo)",
        "---foo",
        "st*ar*",
        "st*ar\\\\**",
        "sT*Ar\\\\\\*",
        "star\n",
        "star(",
        "star)",
        "star\"",
        "- + \"\" - star \\",
        "\\ ",
        "\\ \\ ",
        "\" \"|\" \"",
        "(\" \" \" \")",
        "  |     star + + |   wars   \\",
        "- -star -",
        "\"star wars\\ \"",
        "     -()(star)        \n\n\r     ",
        "| + - ( + - |      star    \n      ) \n",
        " | --star wars empire \n\\",
        "star| + wars + ----empire |",
        "(star + | wars |) | ----empire | + --(strikes + | --back) \\",
        "star + (| wars | | --(--empire | + --strikes + | --back + jar\\+\\|jar) \"\" ) \"",
        "*",
        " *   ",
        "\"test\"",
        "test*",
        "+foo+bar",
        "|foo|bar",
        "(foo)",
        ")foo(",
        "foo\\bar",
        "(foo\\bar)",
        "\"foo\\bar\"",
        " foo foo\n ",
        "\t\tfoo foo foo",
        "foo~1",
        "a1|&( )\"-~",
        "~~~",
        "\"\"\"",
        "((((",
        "))))",
        "\\",
        "a\\",
        "-",
        "+",
        "|",
        "foo~0",
        "foo~-1",
        "\"a\"~2147483648",
        "foo~99999999999",
        "alpha* bravo~1 \"charlie delta\"~3",
        "über* straße",
        "日本語~1",
        "a\u{1F600}b*",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// `n` strings of the characters `testRandomQueries2` draws from, and
/// words.
fn simple_random(seed: i64, n: usize) -> Vec<String> {
    let random = JdkRandom::new(seed);
    let atoms = [
        "a", "1", "|", "&", " ", "(", ")", "\"", "-", "~", "+", "*", "\\", "foo", "bar", "~2",
        "\t", "\n",
    ];
    (0..n)
        .map(|_| {
            let len = random.next_int_bound(20);
            (0..len)
                .map(|_| atoms[random.next_int_bound(atoms.len() as i32) as usize])
                .collect::<String>()
        })
        .collect()
}

fn shard(index: i32) {
    let random = JdkRandom::new(SEED ^ i64::from(index));
    let mut oracle = ParseOracle::new(registry());
    oracle.bind(
        "an",
        &expr(
            "new",
            &["org.apache.lucene.analysis.standard.StandardAnalyzer"],
        ),
    );
    oracle.bind("w1", &expr("new", &[LINKED_HASH_MAP]));
    oracle.run(&expr("call", &["put", "$w1", &lit("body"), "f:1.0"]));
    oracle.bind("w2", &expr("new", &[LINKED_HASH_MAP]));
    oracle.run(&expr("call", &["put", "$w2", &lit("title"), "f:5.0"]));
    oracle.run(&expr("call", &["put", "$w2", &lit("body"), "f:10.0"]));
    let inputs: Vec<String> = simple_edge_cases()
        .into_iter()
        .chain(corpus::classic_edge_cases())
        .chain(simple_random(SEED + i64::from(index), 30))
        .collect();
    let mut records = 0;
    for flags in (index * 256)..((index + 1) * 256) {
        let name = format!("p{flags}");
        let weights = if random.next_boolean() { "$w1" } else { "$w2" };
        oracle.bind(
            &name,
            &expr("new", &[SQP, "$an", weights, &format!("i:{flags}")]),
        );
        if random.next_boolean() {
            oracle.run(&expr(
                "call",
                &[
                    "setDefaultOperator",
                    &format!("${name}"),
                    &expr("field", &[OCCUR, "MUST"]),
                ],
            ));
        }
        // Every flag combination parses a seeded sample of the corpus; the
        // whole corpus runs under the all-on and all-off combinations.
        let take = if flags == 0 || flags == 1023 {
            inputs.len()
        } else {
            12
        };
        for k in 0..take {
            let input = if take == inputs.len() {
                &inputs[k]
            } else {
                &inputs[random.next_int_bound(inputs.len() as i32) as usize]
            };
            let units: Vec<u16> = input.encode_utf16().collect();
            oracle.parse(
                &format!("f{flags}_{k}"),
                &expr("call", &["parse", &format!("${name}"), "$input"]),
                &units,
                Some("body"),
            );
            records += 1;
        }
    }
    let java = oracle.java_dump();
    let rust = oracle.rust_dump();
    println!(
        "simple shard {index}: {records} parses, {} dump lines",
        java.lines().count()
    );
    assert!(records >= 256 * 12, "every flag combination parses");
    if let Err(diff) = compare(&java, &rust) {
        panic!("SimpleQueryParser parity with Lucene 10.5.0 fails:\n{diff}");
    }
}

#[test]
fn simple_query_parser_flags_0_to_255_match_lucene() {
    shard(0);
}

#[test]
fn simple_query_parser_flags_256_to_511_match_lucene() {
    shard(1);
}

#[test]
fn simple_query_parser_flags_512_to_767_match_lucene() {
    shard(2);
}

#[test]
fn simple_query_parser_flags_768_to_1023_match_lucene() {
    shard(3);
}
