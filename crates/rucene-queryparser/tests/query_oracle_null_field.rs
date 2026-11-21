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

//! Parser parity without a default field (rmp #496): Lucene's parsers build
//! terms of a `null` field (`new Term(null, text)`) when no default field is
//! configured, and Rucene's core `Term` now accepts one. Through the query
//! modules' Java-oracle harness (rmp #475), Lucene 10.5.0 and Rucene parse
//! the same strings and give the same `toString()`, `toString("body")` and
//! class tree — or the same exception, of the parse or of each of those
//! stages (a `TermQuery` of a `null` field throws a `NullPointerException`
//! from `toString`):
//!
//! * the queries the parsers build over such a term, built directly
//!   (`TermQuery`, `BoostQuery`, `BooleanQuery`, `PhraseQuery` — whose
//!   builder throws Java's helpful `NullPointerException` on a second term —
//!   and the multi-term queries, which throw `NullPointerException("field
//!   must not be null")`);
//! * `new StandardQueryParser()` (no analyzer, an empty `MULTI_FIELDS`)
//!   parsing with a `null` default field: the classic corpus (fixed edge
//!   cases and seeded strings). Lucene's `MatchAllDocsQueryNodeProcessor`
//!   dereferences the `null` field of every `FieldQueryNode`
//!   (`MatchAllDocsQueryNodeProcessor.java:45`), so these give its helpful
//!   `NullPointerException` or a syntax error;
//! * `new QueryParser(null, new StandardAnalyzer())` (leading wildcards
//!   allowed) over the strings whose `null`-field terms are built without
//!   the analyzer (pure wildcards, open ranges, `*:*`, field-qualified
//!   clauses) and over the whole classic corpus: the analyzer receives the
//!   `null` field (`normalize`, `tokenStream` through
//!   `QueryBuilder.createFieldQuery`), so an analyzed term becomes a
//!   `TermQuery(null:term)`, a phrase a `PhraseQuery` builder NPE;
//! * `new QueryBuilder(new StandardAnalyzer())` building boolean and phrase
//!   queries for a `null` field.
//!
//! A perturbed Rust dump must fail at the perturbed record (positive control).

#[path = "../../rucene-queries/tests/query_oracle/mod.rs"]
mod query_oracle;

use std::sync::{Arc, RwLock};

use query_oracle::{compare, corpus, expr, lit, ParseOracle, Registry, Value};
use rucene::analysis::Analyzer;
use rucene::index::Term;
use rucene::search::fuzzy_query::FuzzyQuery;
use rucene::search::prefix_query::PrefixQuery;
use rucene::search::regexp_query::RegexpQuery;
use rucene::search::wildcard_query::WildcardQuery;
use rucene::util::query_builder::{QueryBuilder, QueryBuilderOps};
use rucene_queryparser::classic::query_parser::QueryParser;
use rucene_queryparser::classic::query_parser_base::QueryParserBaseOps;
use rucene_queryparser::flexible::standard::common_query_parser_configuration::CommonQueryParserConfiguration;
use rucene_queryparser::flexible::standard::standard_query_parser::StandardQueryParser;
use rucene_queryparser::jdk::char_sequence::CharSequence;
use rucene_queryparser::jdk::locale::Locale;
use rucene_queryparser::jdk::time_zone::TimeZone;

/// The seed of the generated query strings.
const SEED: i64 = 0x0496_0f1e;

const QP: &str = "org.apache.lucene.queryparser.classic.QueryParser";
const SQP: &str = "org.apache.lucene.queryparser.flexible.standard.StandardQueryParser";

/// [`Registry::core`] with the members of `classic.QueryParser` and
/// `StandardQueryParser` the specs call, and `Locale.forLanguageTag` /
/// `TimeZone.getTimeZone`.
fn registry() -> Registry {
    let mut r = Registry::core();
    r.new_fn(QP, 2, |a| {
        let analyzer = match &a[1] {
            Value::Null => None,
            v => Some(v.get::<Arc<dyn Analyzer>>()),
        };
        Ok(Value::object(
            QP,
            QueryParser::new(a[0].as_opt_str(), analyzer)?,
        ))
    });
    r.call_fn(QP, "parse", 1, |p, a| {
        let input = a[0].as_str().to_string();
        let q = p.with_mut(|p: &mut QueryParser| p.parse(&input))?;
        Ok(Value::query(q))
    });
    r.call_fn(QP, "setAllowLeadingWildcard", 1, |p, a| {
        let allow = a[0].as_bool();
        p.with_mut(|p: &mut QueryParser| p.set_allow_leading_wildcard(allow))?;
        Ok(Value::Null)
    });
    r.call_fn(QP, "setLocale", 1, |p, a| {
        let locale = Some(a[0].get::<Locale>());
        p.with_mut(|p: &mut QueryParser| p.set_locale(locale))?;
        Ok(Value::Null)
    });
    r.call_fn(QP, "setTimeZone", 1, |p, a| {
        let zone = Some(a[0].get::<TimeZone>());
        p.with_mut(|p: &mut QueryParser| p.set_time_zone(zone))?;
        Ok(Value::Null)
    });
    r.new_fn(SQP, 0, |_| {
        Ok(Value::object(SQP, StandardQueryParser::new()))
    });
    r.call_fn(SQP, "parse", 2, |p, a| {
        let input = a[0].as_str().to_string();
        let field = a[1].as_opt_str().map(str::to_string);
        let q = p.with_mut(|p: &mut StandardQueryParser| p.parse(&input, field.as_deref()))?;
        Ok(match q {
            Some(q) => Value::query(q),
            None => Value::Null,
        })
    });
    r.call_fn(SQP, "setMultiFields", 1, |p, a| {
        let fields: Vec<Option<CharSequence>> = a[0]
            .as_values()
            .iter()
            .map(|v| v.as_opt_str().map(CharSequence::from))
            .collect();
        p.with_mut(|p: &mut StandardQueryParser| {
            p.set_multi_fields(Some(Arc::new(RwLock::new(fields))))
        })?;
        Ok(Value::Null)
    });
    const QB: &str = "org.apache.lucene.util.QueryBuilder";
    r.new_fn(QB, 1, |a| {
        Ok(Value::object(
            QB,
            QueryBuilder::new(Some(a[0].get::<Arc<dyn Analyzer>>())),
        ))
    });
    r.call_fn(QB, "createBooleanQuery", 2, |b, a| {
        let (field, text) = (
            a[0].as_opt_str().map(str::to_string),
            a[1].as_str().to_string(),
        );
        let q =
            b.with_mut(|b: &mut QueryBuilder| b.create_boolean_query(field.as_deref(), &text))?;
        Ok(q.map_or(Value::Null, Value::query))
    });
    r.call_fn(QB, "createPhraseQuery", 2, |b, a| {
        let (field, text) = (
            a[0].as_opt_str().map(str::to_string),
            a[1].as_str().to_string(),
        );
        let q =
            b.with_mut(|b: &mut QueryBuilder| b.create_phrase_query(field.as_deref(), &text))?;
        Ok(q.map_or(Value::Null, Value::query))
    });
    r.new_fn("org.apache.lucene.search.WildcardQuery", 1, |a| {
        Ok(Value::query(Arc::new(WildcardQuery::new(
            a[0].get::<Term>(),
        )?)))
    });
    r.new_fn("org.apache.lucene.search.PrefixQuery", 1, |a| {
        Ok(Value::query(Arc::new(PrefixQuery::new(
            a[0].get::<Term>(),
        )?)))
    });
    r.new_fn("org.apache.lucene.search.RegexpQuery", 1, |a| {
        Ok(Value::query(Arc::new(RegexpQuery::new(
            a[0].get::<Term>(),
        )?)))
    });
    r.new_fn("org.apache.lucene.search.FuzzyQuery", 1, |a| {
        Ok(Value::query(Arc::new(FuzzyQuery::new(a[0].get::<Term>())?)))
    });
    r.static_fn("java.util.Locale", "forLanguageTag", 1, |a| {
        Ok(Value::object(
            "java.util.Locale",
            Locale::builder_from_language_tag(a[0].as_str())?,
        ))
    });
    r.static_fn("java.util.TimeZone", "getTimeZone", 1, |a| {
        Ok(Value::object(
            "sun.util.calendar.ZoneInfo",
            TimeZone::get_time_zone(a[0].as_str()),
        ))
    });
    r
}

/// Query strings whose `null`-field terms a classic parser builds without
/// handing the analyzer a `null` field.
fn classic_inputs() -> Vec<String> {
    [
        "*",
        "?",
        "**",
        "*?",
        "?*?",
        "*:*",
        "[* TO *]",
        "{* TO *}",
        "title:alpha",
        "title:(alpha bravo)",
        "title:\"alpha bravo\"",
        "title:alp*",
        "title:alpha~1",
        "title:[alpha TO bravo]",
        "title:/br.vo/",
        "* title:alpha",
        "+* -title:alpha",
        "title:alpha OR *",
        "(* ?)^2",
        "*^3",
        "title:*",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn oracle() -> (ParseOracle, Vec<String>) {
    let mut oracle = ParseOracle::new(registry());
    // `new StandardQueryParser()`: no analyzer, no default field, and an
    // empty `MULTI_FIELDS` (`MultiFieldQueryNodeProcessor` rejects an unset
    // one), so a node without a field keeps its `null` field.
    oracle.bind("sqp", &expr("new", &[SQP]));
    oracle.run(&expr(
        "call",
        &[
            "setMultiFields",
            "$sqp",
            &expr("array", &["java.lang.CharSequence"]),
        ],
    ));
    // `new QueryParser(null, new StandardAnalyzer())`, with the locale
    // `en-US` and the time zone `UTC` (date ranges must not depend on the
    // machine's defaults).
    let analyzer = expr(
        "new",
        &["org.apache.lucene.analysis.standard.StandardAnalyzer"],
    );
    oracle.bind("qp", &expr("new", &[QP, "null", &analyzer]));
    oracle.run(&expr(
        "call",
        &[
            "setLocale",
            "$qp",
            &expr(
                "static",
                &["java.util.Locale", "forLanguageTag", &lit("en-US")],
            ),
        ],
    ));
    // Leading wildcards allowed, so that a pure wildcard reaches the term of
    // the `null` field.
    oracle.run(&expr("call", &["setAllowLeadingWildcard", "$qp", "z:true"]));
    oracle.run(&expr(
        "call",
        &[
            "setTimeZone",
            "$qp",
            &expr(
                "static",
                &["java.util.TimeZone", "getTimeZone", &lit("UTC")],
            ),
        ],
    ));

    let mut ids = Vec::new();
    // Queries over a term of a `null` field, built directly: what the
    // parsers hand on when they build such a term.
    let null_term = || {
        expr(
            "new",
            &["org.apache.lucene.index.Term", "null", &lit("foo")],
        )
    };
    let body_term = |t: &str| {
        expr(
            "new",
            &["org.apache.lucene.index.Term", &lit("body"), &lit(t)],
        )
    };
    let term_query = |t: &str| expr("new", &["org.apache.lucene.search.TermQuery", t]);
    const BQB: &str = "org.apache.lucene.search.BooleanQuery$Builder";
    const PQB: &str = "org.apache.lucene.search.PhraseQuery$Builder";
    let occur = |o: &str| {
        expr(
            "field",
            &["org.apache.lucene.search.BooleanClause$Occur", o],
        )
    };
    let add_clause = |b: &str, q: &str, o: &str| expr("call", &["add", b, q, &occur(o)]);
    let add_term = |b: &str, t: &str| expr("call", &["add", b, t]);
    let direct = vec![
        term_query(&null_term()),
        expr(
            "new",
            &[
                "org.apache.lucene.search.BoostQuery",
                &term_query(&null_term()),
                "f:2.0",
            ],
        ),
        expr(
            "call",
            &[
                "build",
                &add_clause(
                    &add_clause(&expr("new", &[BQB]), &term_query(&null_term()), "SHOULD"),
                    &term_query(&body_term("bar")),
                    "MUST",
                ),
            ],
        ),
        expr(
            "call",
            &["build", &add_term(&expr("new", &[PQB]), &null_term())],
        ),
        expr(
            "call",
            &[
                "build",
                &add_term(&add_term(&expr("new", &[PQB]), &null_term()), &null_term()),
            ],
        ),
        expr(
            "call",
            &[
                "build",
                &add_term(
                    &add_term(&expr("new", &[PQB]), &null_term()),
                    &body_term("a"),
                ),
            ],
        ),
        expr(
            "call",
            &[
                "build",
                &add_term(
                    &add_term(&expr("new", &[PQB]), &body_term("a")),
                    &null_term(),
                ),
            ],
        ),
        expr(
            "new",
            &["org.apache.lucene.search.WildcardQuery", &null_term()],
        ),
        expr(
            "new",
            &["org.apache.lucene.search.PrefixQuery", &null_term()],
        ),
        expr(
            "new",
            &["org.apache.lucene.search.RegexpQuery", &null_term()],
        ),
        expr(
            "new",
            &["org.apache.lucene.search.FuzzyQuery", &null_term()],
        ),
    ];
    const QB: &str = "org.apache.lucene.util.QueryBuilder";
    let query_builder = || {
        expr(
            "new",
            &[
                QB,
                &expr(
                    "new",
                    &["org.apache.lucene.analysis.standard.StandardAnalyzer"],
                ),
            ],
        )
    };
    let direct: Vec<String> = direct
        .into_iter()
        .chain(
            ["Foo", "Foo bar", "The quick brown fox"]
                .iter()
                .flat_map(|text| {
                    [
                        expr(
                            "call",
                            &["createBooleanQuery", &query_builder(), "null", &lit(text)],
                        ),
                        expr(
                            "call",
                            &["createPhraseQuery", &query_builder(), "null", &lit(text)],
                        ),
                    ]
                }),
        )
        .collect();
    for (i, e) in direct.iter().enumerate() {
        let id = format!("direct{i}");
        oracle.parse(&id, e, &[], Some("body"));
        ids.push(id);
    }
    let standard: Vec<String> = corpus::classic_edge_cases()
        .into_iter()
        .chain(corpus::classic_random(SEED, 200))
        .collect();
    for (i, input) in standard.iter().enumerate() {
        let id = format!("sqp{i}");
        let units: Vec<u16> = input.encode_utf16().collect();
        oracle.parse(
            &id,
            &expr("call", &["parse", "$sqp", "$input", "null"]),
            &units,
            Some("body"),
        );
        ids.push(id);
    }
    let classic: Vec<String> = classic_inputs()
        .into_iter()
        .chain(corpus::classic_edge_cases())
        .chain(corpus::classic_random(SEED + 1, 200))
        .collect();
    for (i, input) in classic.iter().enumerate() {
        let id = format!("qp{i}");
        let units: Vec<u16> = input.encode_utf16().collect();
        oracle.parse(
            &id,
            &expr("call", &["parse", "$qp", "$input"]),
            &units,
            Some("body"),
        );
        ids.push(id);
    }
    (oracle, ids)
}

/// Appends `X` to the first `tostring` or `error` line of record `id`.
fn perturb(dump: &str, id: &str) -> String {
    let mut out = String::new();
    let mut in_record = false;
    let mut done = false;
    for l in dump.lines() {
        if l.starts_with("parse\t") {
            in_record = l == format!("parse\t{id}");
        }
        if in_record && !done && (l.starts_with("tostring\t") || l.starts_with("error\t")) {
            out.push_str(&format!("{l}X"));
            done = true;
        } else {
            out.push_str(l);
        }
        out.push('\n');
    }
    assert!(done, "record {id} has a line to perturb");
    out
}

#[test]
fn parsers_without_a_default_field_match_lucene() {
    let (oracle, ids) = oracle();
    let java = oracle.java_dump();
    let rust = oracle.rust_dump();
    let count = |pred: &dyn Fn(&str) -> bool| java.lines().filter(|l| pred(l)).count();
    let to_string_npes =
        count(&|l| l.starts_with("error\ttostring\tjava.lang.NullPointerException\t"));
    let field_npes =
        count(&|l| l == "error\tjava.lang.NullPointerException\tfield must not be null");
    let null_field_trees = count(&|l| l.starts_with("classtree\t") && l.contains(":[null:"));
    let flexible_npes = count(&|l| {
        l.starts_with("error\tjava.lang.NullPointerException\t")
            && l.contains("FieldQueryNode.getField()")
    });
    println!(
        "{} parses, {to_string_npes} toString NPEs, {field_npes} multi-term NPEs, \
         {null_field_trees} class trees with null-field terms, {flexible_npes} flexible \
         null-field NPEs, {} dump lines",
        ids.len(),
        java.lines().count()
    );
    assert!(
        to_string_npes >= 30 && field_npes >= 4 && null_field_trees >= 30 && flexible_npes > 20,
        "the corpus must reach terms of a null field; Lucene's dump begins:\n{}",
        java.lines().take(60).collect::<Vec<_>>().join("\n")
    );
    if let Err(diff) = compare(&java, &rust) {
        panic!("parser parity with Lucene 10.5.0 without a default field fails:\n{diff}");
    }

    // Positive control: a changed line fails at its query string.
    let perturbed = perturb(&rust, "sqp2");
    let clean = compare(&java, &perturbed);
    assert!(clean.is_err(), "a perturbed parse must fail the comparison");
    let message = clean.unwrap_err();
    assert!(
        message.contains("record `parse\tsqp2`"),
        "the failure names the perturbed query string: {message}"
    );
}
