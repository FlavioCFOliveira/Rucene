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

//! The parser parity of the XML query parser (rmp #475, #493): Lucene
//! 10.5.0's and Rucene's `CoreParser`, `CorePlusQueriesParser` and
//! `CorePlusExtensionsParser` parse
//!
//! * every test XML of Lucene's `queryparser.xml` tests, read from the same
//!   file by both sides (`parse` and `parseAsSpanQuery`);
//! * a seeded set of generated XML queries covering every builder (with
//!   valid, missing, empty and malformed attributes, unknown elements,
//!   nesting, and malformed documents),
//!
//! and give the same `toString()` and class tree, or the same exception
//! class and message. The resources' SHA-1s are checked against Lucene's
//! files (`xml_test_resources_are_byte_identical`).

#[path = "../../rucene-queries/tests/query_oracle/mod.rs"]
mod query_oracle;

use std::path::PathBuf;
use std::sync::Arc;

use query_oracle::{compare, expr, lit, sha1, ParseOracle, Registry, Value, VOCABULARY};
use randomizedtesting::jdk::random::{JdkRandom, Random};
use rucene::analysis::Analyzer;
use rucene::search::Query;
use rucene_queryparser::xml::core_parser::{CoreParser, CoreParserOps};
use rucene_queryparser::xml::core_plus_extensions_parser::CorePlusExtensionsParser;
use rucene_queryparser::xml::core_plus_queries_parser::CorePlusQueriesParser;

const SEED: i64 = 0x0493_584D;

const CP: &str = "org.apache.lucene.queryparser.xml.CoreParser";
const CPQ: &str = "org.apache.lucene.queryparser.xml.CorePlusQueriesParser";
const CPE: &str = "org.apache.lucene.queryparser.xml.CorePlusExtensionsParser";
const FIS: &str = "java.io.FileInputStream";
const BAIS: &str = "java.io.ByteArrayInputStream";
const ORACLE: &str = "org.apache.lucene.rucene.codec.QueryOracle";

/// `$p.parse(stream)` or `QueryOracle.parseAsSpanQuery($p, stream)`.
fn parse_call(method: &str, parser: &str, stream: &str) -> String {
    if method == "parse" {
        expr("call", &["parse", parser, stream])
    } else {
        expr("static", &[ORACLE, "parseAsSpanQuery", parser, stream])
    }
}

/// The resources and the SHA-1 of each in Lucene 10.5.0's
/// `lucene/queryparser/src/test/org/apache/lucene/queryparser/xml/`.
const RESOURCES: [(&str, &str); 25] = [
    (
        "BooleanQuery.xml",
        "da30bba9d29db39c04154c73b5c0dfd0b2ad58e9",
    ),
    (
        "BoostingQuery.xml",
        "69a88bf368491176dc7abdd4dd213c56bbbc2c2f",
    ),
    (
        "BoostingTermQuery.xml",
        "c56338175fe991446aa296c6ae4b0e77832c9b38",
    ),
    (
        "ConstantScoreQuery.xml",
        "b7a4318086fc47817aa09eecbdeb4f26158de11c",
    ),
    (
        "DisjunctionMaxQuery.xml",
        "fa973134fae5b8a56fda65864b0347c492e3bcbe",
    ),
    (
        "DOCTYPE_TermQuery.xml",
        "06477c2982fb51b4591eb82e936cf615cd599cfb",
    ),
    (
        "ENTITY_TermQuery.xml",
        "67e1cf13c8cb189b56d00d05253429d4cbd2b691",
    ),
    (
        "FuzzyLikeThisQuery.xml",
        "c224140c4a0d0a8a493841eda84fe2b95f0b822a",
    ),
    (
        "LikeThisQuery.xml",
        "328661676b3b932b384330d5b32db91fe06332ec",
    ),
    (
        "MatchAllDocsQuery.xml",
        "cf4d041f93bc4e61223c0a2cac8c97aa9c601dde",
    ),
    (
        "NestedBooleanQuery.xml",
        "4a4e1da23656880890cada11b3d7b780fc1b0a56",
    ),
    (
        "PointRangeQueryWithoutLowerTerm.xml",
        "80ef0fa6223077219aeac653c7c0d6c8a730adc3",
    ),
    (
        "PointRangeQueryWithoutRange.xml",
        "f694963398766f9323a6c2cb02c392e73b2536de",
    ),
    (
        "PointRangeQueryWithoutUpperTerm.xml",
        "6d089462edb985e730a6eb98a58a64fb400b659a",
    ),
    (
        "PointRangeQuery.xml",
        "3794cf82fc4a0213f1a7f82e05081ffd9fc5c236",
    ),
    ("RangeQuery.xml", "2a516f4fc86eecd84444a4e233ef6a8e53d79a3a"),
    (
        "SpanNearQueryWithoutSlop.xml",
        "a33c91cf0a59152c4d0f85817c5081efda3a6f30",
    ),
    (
        "SpanPositionRangeQuery.xml",
        "b5401ce3a3835ff1af890a1e2bc62ab1f6b1668f",
    ),
    ("SpanQuery.xml", "b8ed144d74740d0940e27ba76a74618a8da7c775"),
    (
        "TermQueryEmpty.xml",
        "5a969de60bd8cc1aa3d0d6e59256493eb0c85374",
    ),
    ("TermQuery.xml", "1d3a7ba715d2740c146e9fcdcb4ec629e05d21df"),
    ("TermsQuery.xml", "68e689c7e63cc8bd8c610ea880d7676d2c5c8a0a"),
    (
        "UserInputQueryCustomField.xml",
        "7cfd4be4617b420672ba7009964eaadbeca79ef8",
    ),
    (
        "UserInputQuery.xml",
        "0a98df3cfc0b84a9a94d3924d372d167f2dfac68",
    ),
    (
        "reuters21578.txt",
        "1a5946c0dd6ed1792ca158d9cf577de845cb9cf2",
    ),
];

fn resource_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/resources/org/apache/lucene/queryparser/xml")
}

#[test]
fn xml_test_resources_are_byte_identical() {
    for (name, expected) in RESOURCES {
        let data = std::fs::read(resource_dir().join(name))
            .unwrap_or_else(|e| panic!("read the resource {name}: {e}"));
        assert_eq!(sha1(&data), expected, "SHA-1 of {name}");
    }
}

/// The bytes an `InputStream` value of the specs holds.
#[derive(Clone)]
struct Bytes(Vec<u8>);

fn opt_analyzer(v: &Value) -> Option<Arc<dyn Analyzer>> {
    match v {
        Value::Null => None,
        v => Some(v.get::<Arc<dyn Analyzer>>()),
    }
}

/// A constructor `(String defaultField, Analyzer analyzer)` of a parser.
type ParserCtor = fn(Option<String>, Option<Arc<dyn Analyzer>>) -> Arc<dyn CoreParserOps>;

fn register_parser(r: &mut Registry, class: &'static str, ctor: ParserCtor) {
    r.new_fn(class, 2, move |a| {
        Ok(Value::object(
            class,
            ctor(a[0].as_opt_str().map(str::to_string), opt_analyzer(&a[1])),
        ))
    });
    r.call_fn(class, "parse", 1, |p, a| {
        let bytes = a[0].get::<Bytes>().0;
        let parser = p.get::<Arc<dyn CoreParserOps>>();
        Ok(Value::query(parser.parse(&mut &bytes[..])?))
    });
}

fn registry() -> Registry {
    let mut r = Registry::core();
    register_parser(&mut r, CP, |f, a| Arc::new(CoreParser::new(f, a)));
    register_parser(&mut r, CPQ, |f, a| {
        Arc::new(CorePlusQueriesParser::new(f, a))
    });
    register_parser(&mut r, CPE, |f, a| {
        Arc::new(CorePlusExtensionsParser::new(f, a))
    });
    // `QueryOracle.parseAsSpanQuery(parser, in)`: the package-private
    // `CoreParser.parseAsSpanQuery(InputStream)`.
    r.static_fn(ORACLE, "parseAsSpanQuery", 2, |a| {
        let parser = a[0].get::<Arc<dyn CoreParserOps>>();
        let bytes = a[1].get::<Bytes>().0;
        let q: Arc<dyn Query> = parser.parse_as_span_query(&mut &bytes[..])?;
        Ok(Value::query(q))
    });
    r.new_fn(FIS, 1, |a| {
        let data = std::fs::read(a[0].as_str()).expect("read a resource");
        Ok(Value::object(FIS, Bytes(data)))
    });
    r.field_fn("java.nio.charset.StandardCharsets", "UTF_8", || {
        Value::object("sun.nio.cs.UTF_8", ())
    });
    r.call_fn("java.lang.String", "getBytes", 1, |s, _| {
        Ok(Value::object("[B", Bytes(s.as_str().as_bytes().to_vec())))
    });
    r.new_fn(BAIS, 1, |a| Ok(Value::object(BAIS, a[0].get::<Bytes>())));
    r
}

fn bind_parsers(oracle: &mut ParseOracle) {
    let analyzer = expr(
        "new",
        &["org.apache.lucene.analysis.standard.StandardAnalyzer"],
    );
    oracle.bind("cp", &expr("new", &[CP, &lit("contents"), &analyzer]));
    oracle.bind("cpq", &expr("new", &[CPQ, &lit("contents"), &analyzer]));
    oracle.bind("cpe", &expr("new", &[CPE, &lit("contents"), &analyzer]));
    oracle.bind("nofield", &expr("new", &[CPE, "null", &analyzer]));
}

const PARSERS: [&str; 4] = ["cp", "cpq", "cpe", "nofield"];

fn assert_parity(oracle: &ParseOracle, what: &str) {
    let java = oracle.java_dump();
    let rust = oracle.rust_dump();
    let records = java.lines().filter(|l| l.starts_with("parse\t")).count();
    let errors = java.lines().filter(|l| l.starts_with("error\t")).count();
    println!(
        "{what}: {records} parses, {errors} exceptions, {} dump lines",
        java.lines().count()
    );
    assert!(
        errors > 0 && errors < records,
        "{what}: the corpus must parse and fail"
    );
    // Diagnostic aid: `RUCENE_ORACLE_DUMP_DIR=<dir>` keeps both dumps.
    if let Ok(dir) = std::env::var("RUCENE_ORACLE_DUMP_DIR") {
        let name: String = what
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '_' })
            .collect();
        std::fs::write(format!("{dir}/{name}.java"), &java).expect("write the Java dump");
        std::fs::write(format!("{dir}/{name}.rust"), &rust).expect("write the Rust dump");
    }
    if let Err(diff) = compare(&java, &rust) {
        panic!("{what}: XML parser parity with Lucene 10.5.0 fails:\n{diff}");
    }
}

#[test]
fn xml_test_resources_parse_like_lucene() {
    let mut oracle = ParseOracle::new(registry());
    bind_parsers(&mut oracle);
    for (name, _) in RESOURCES.iter().filter(|(n, _)| n.ends_with(".xml")) {
        let path = resource_dir().join(name);
        let path = path.to_string_lossy();
        let units: Vec<u16> = name.encode_utf16().collect();
        for p in PARSERS {
            for method in ["parse", "parseAsSpanQuery"] {
                oracle.parse(
                    &format!("{p}.{method}.{name}"),
                    &parse_call(method, &format!("${p}"), &expr("new", &[FIS, &lit(&path)])),
                    &units,
                    Some("contents"),
                );
            }
        }
    }
    assert_parity(&oracle, "XML test resources");
}

// ---------------------------------------------------------------------------
// Generated queries
// ---------------------------------------------------------------------------

fn pick<'a>(random: &mut JdkRandom, items: &[&'a str]) -> &'a str {
    items[random.next_int_bound(items.len() as i32) as usize]
}

/// An optional attribute: absent, empty, a valid value or a malformed one.
fn attr(random: &mut JdkRandom, name: &str, valid: &[&str], bad: &[&str]) -> String {
    match random.next_int_bound(8) {
        0 => String::new(),
        1 => format!(" {name}=\"\""),
        2 if !bad.is_empty() => format!(" {name}=\"{}\"", pick(random, bad)),
        _ => format!(" {name}=\"{}\"", pick(random, valid)),
    }
}

fn words(random: &mut JdkRandom, max: i32) -> String {
    let n = random.next_int_bound(max + 1);
    let w: Vec<&str> = (0..n).map(|_| pick(random, &VOCABULARY)).collect();
    w.join(" ")
}

const FIELDS: [&str; 4] = ["contents", "body", "title", "date3"];
const NUMS: [&str; 5] = ["0", "1", "2", "3.5", "10"];
const BAD_NUMS: [&str; 3] = ["x", "-", "1e"];
const BOOLS: [&str; 3] = ["true", "false", "TRUE"];

fn field_attr(random: &mut JdkRandom) -> String {
    attr(random, "fieldName", &FIELDS, &[])
}

fn span(random: &mut JdkRandom, depth: u32) -> String {
    let leaf = depth >= 2;
    match random.next_int_bound(if leaf { 2 } else { 7 }) {
        0 => format!(
            "<SpanTerm{}{}>{}</SpanTerm>",
            field_attr(random),
            attr(random, "boost", &NUMS, &BAD_NUMS),
            pick(random, &VOCABULARY)
        ),
        1 => format!(
            "<SpanOrTerms{}>{}</SpanOrTerms>",
            field_attr(random),
            words(random, 3)
        ),
        2 => format!(
            "<SpanNear{}{}{}>{}{}</SpanNear>",
            attr(random, "slop", &["0", "1", "5"], &BAD_NUMS),
            attr(random, "inOrder", &BOOLS, &[]),
            field_attr(random),
            span(random, depth + 1),
            span(random, depth + 1)
        ),
        3 => format!(
            "<SpanOr{}>{}{}</SpanOr>",
            field_attr(random),
            span(random, depth + 1),
            span(random, depth + 1)
        ),
        4 => format!(
            "<SpanFirst{}{}>{}</SpanFirst>",
            attr(random, "end", &["1", "3"], &BAD_NUMS),
            field_attr(random),
            span(random, depth + 1)
        ),
        5 => format!(
            "<SpanNot{}><Include>{}</Include><Exclude>{}</Exclude></SpanNot>",
            field_attr(random),
            span(random, depth + 1),
            span(random, depth + 1)
        ),
        _ => format!(
            "<SpanPositionRange{}{}{}>{}</SpanPositionRange>",
            attr(random, "start", &["0", "1"], &BAD_NUMS),
            attr(random, "end", &["2", "4"], &BAD_NUMS),
            field_attr(random),
            span(random, depth + 1)
        ),
    }
}

fn query(random: &mut JdkRandom, depth: u32) -> String {
    let leaf = depth >= 2;
    match random.next_int_bound(if leaf { 9 } else { 14 }) {
        0 => format!(
            "<TermQuery{}{}>{}</TermQuery>",
            field_attr(random),
            attr(random, "boost", &NUMS, &BAD_NUMS),
            if random.next_int_bound(5) == 0 {
                String::new()
            } else {
                pick(random, &VOCABULARY).to_string()
            }
        ),
        1 => format!(
            "<TermsQuery{}{}{}>{}</TermsQuery>",
            field_attr(random),
            attr(
                random,
                "minimumNumberShouldMatch",
                &["0", "1", "2"],
                &BAD_NUMS
            ),
            attr(random, "boost", &NUMS, &[]),
            words(random, 4)
        ),
        2 => format!(
            "<MatchAllDocsQuery{}/>",
            attr(random, "boost", &NUMS, &BAD_NUMS)
        ),
        3 => format!(
            "<RangeQuery{}{}{}{}{}/>",
            field_attr(random),
            attr(random, "lowerTerm", &["alpha", "c", "19870301"], &[]),
            attr(random, "upperTerm", &["mike", "zulu", "19870310"], &[]),
            attr(random, "includeLower", &BOOLS, &[]),
            attr(random, "includeUpper", &BOOLS, &[])
        ),
        4 => format!(
            "<PointRangeQuery{}{}{}{}/>",
            field_attr(random),
            attr(
                random,
                "type",
                &["int", "long", "float", "double"],
                &["byte", "INT"]
            ),
            attr(random, "lowerTerm", &["19870301", "-5", "1.5"], &["x"]),
            attr(random, "upperTerm", &["19870310", "99", "2.5"], &["y"])
        ),
        5 => format!(
            "<UserQuery{}{}>{}</UserQuery>",
            field_attr(random),
            attr(random, "boost", &NUMS, &[]),
            pick(
                random,
                &[
                    "alpha AND bravo",
                    "\"charlie delta\"~2",
                    "echo*",
                    "foxtrot~1",
                    "(golf",
                    "title:hotel^2",
                    "[a TO c]",
                    ""
                ]
            )
        ),
        6 => format!(
            "<BoostingTermQuery{}{}>{}</BoostingTermQuery>",
            field_attr(random),
            attr(random, "boost", &NUMS, &BAD_NUMS),
            pick(random, &VOCABULARY)
        ),
        7 | 8 => span(random, depth),
        9 => {
            let n = random.next_int_bound(4);
            let clauses: String = (0..n)
                .map(|_| {
                    format!(
                        "<Clause{}>{}</Clause>",
                        attr(
                            random,
                            "occurs",
                            &["must", "should", "mustNot", "filter"],
                            &["never"]
                        ),
                        query(random, depth + 1)
                    )
                })
                .collect();
            format!(
                "<BooleanQuery{}{}{}>{clauses}</BooleanQuery>",
                field_attr(random),
                attr(random, "minimumNumberShouldMatch", &["0", "1"], &BAD_NUMS),
                attr(random, "boost", &NUMS, &[])
            )
        }
        10 => format!(
            "<DisjunctionMaxQuery{}{}>{}{}</DisjunctionMaxQuery>",
            attr(random, "tieBreaker", &["0.0", "0.3", "1"], &BAD_NUMS),
            attr(random, "boost", &NUMS, &[]),
            query(random, depth + 1),
            query(random, depth + 1)
        ),
        11 => format!(
            "<ConstantScoreQuery{}>{}</ConstantScoreQuery>",
            attr(random, "boost", &NUMS, &BAD_NUMS),
            query(random, depth + 1)
        ),
        12 => format!(
            "<LikeThisQuery{}{}{}{}{}>{}</LikeThisQuery>",
            attr(
                random,
                "fieldNames",
                &["contents", "contents,title", "body title"],
                &[]
            ),
            attr(random, "stopWords", &["the of", "alpha"], &[]),
            attr(random, "maxQueryTerms", &["5", "20"], &BAD_NUMS),
            attr(random, "minTermFrequency", &["1", "2"], &BAD_NUMS),
            attr(random, "percentTermsToMatch", &["30", "50.5"], &BAD_NUMS),
            words(random, 6)
        ),
        _ => format!(
            "<FuzzyLikeThisQuery{}{}><Field{}{}{}>{}</Field></FuzzyLikeThisQuery>",
            attr(random, "maxNumTerms", &["5", "50"], &BAD_NUMS),
            attr(random, "ignoreTF", &BOOLS, &[]),
            field_attr(random),
            attr(random, "minSimilarity", &["0.5", "2"], &BAD_NUMS),
            attr(random, "prefixLength", &["0", "1"], &BAD_NUMS),
            words(random, 3)
        ),
    }
}

/// A generated document: well-formed most of the time, else malformed or
/// with an unknown element.
fn document(random: &mut JdkRandom) -> String {
    let q = query(random, 0);
    match random.next_int_bound(12) {
        0 => format!("<Unknown>{q}</Unknown>"),
        1 => q.replacen('>', "", 1),
        2 => format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n{q}"),
        3 => format!("{q}<TermQuery>extra</TermQuery>"),
        _ => q,
    }
}

fn generated_shard(shard: i64) {
    let mut random = JdkRandom::new(SEED ^ shard);
    let mut oracle = ParseOracle::new(registry());
    bind_parsers(&mut oracle);
    for i in 0..100 {
        let doc = document(&mut random);
        let units: Vec<u16> = doc.encode_utf16().collect();
        let stream = expr(
            "new",
            &[
                BAIS,
                &expr(
                    "call",
                    &[
                        "getBytes",
                        "$input",
                        &expr("field", &["java.nio.charset.StandardCharsets", "UTF_8"]),
                    ],
                ),
            ],
        );
        let p = PARSERS[i % PARSERS.len()];
        let method = if random.next_int_bound(4) == 0 {
            "parseAsSpanQuery"
        } else {
            "parse"
        };
        oracle.parse(
            &format!("g{i}"),
            &parse_call(method, &format!("${p}"), &stream),
            &units,
            Some("contents"),
        );
    }
    assert_parity(&oracle, &format!("generated XML shard {shard}"));
}

#[test]
fn generated_xml_queries_parse_like_lucene_0() {
    generated_shard(0);
}

#[test]
fn generated_xml_queries_parse_like_lucene_1() {
    generated_shard(1);
}

#[test]
fn generated_xml_queries_parse_like_lucene_2() {
    generated_shard(2);
}
