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

//! The parser parity of the query modules' Java-oracle harness (rmp #475):
//! Lucene's `classic.QueryParser` and Rucene's parse the classic corpus
//! (fixed edge cases and seeded strings) under two configurations, and give
//! the same `toString()`, `toString("body")` and class tree, or the same
//! exception class and message; a perturbed Rust dump must fail at the
//! perturbed query string (positive control).
//!
//! The harness is shared with `rucene-queries` (`query_oracle`, rmp #475).

#[path = "../../rucene-queries/tests/query_oracle/mod.rs"]
mod query_oracle;

use std::sync::Arc;

use query_oracle::{compare, corpus, expr, lit, ParseOracle, Registry, Value};
use rucene::analysis::Analyzer;
use rucene_queryparser::classic::query_parser::{Operator, QueryParser};
use rucene_queryparser::classic::query_parser_base::QueryParserBaseOps;
use rucene_queryparser::flexible::standard::common_query_parser_configuration::CommonQueryParserConfiguration;
use rucene_queryparser::jdk::locale::Locale;
use rucene_queryparser::jdk::time_zone::TimeZone;

/// The seed of the generated query strings.
const SEED: i64 = 0x0475_c1a5;

const QP: &str = "org.apache.lucene.queryparser.classic.QueryParser";
const OPERATOR: &str = "org.apache.lucene.queryparser.classic.QueryParser$Operator";

/// [`Registry::core`] with the members of `classic.QueryParser` the specs
/// call, and `Locale.forLanguageTag` / `TimeZone.getTimeZone`.
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
    r.call_fn(QP, "setDefaultOperator", 1, |p, a| {
        let op = a[0].get::<Operator>();
        p.with_mut(|p: &mut QueryParser| p.set_default_operator(op));
        Ok(Value::Null)
    });
    r.call_fn(QP, "setAllowLeadingWildcard", 1, |p, a| {
        let allow = a[0].as_bool();
        p.with_mut(|p: &mut QueryParser| p.set_allow_leading_wildcard(allow))?;
        Ok(Value::Null)
    });
    r.call_fn(QP, "setLocale", 1, |p, a| {
        let locale = match &a[0] {
            Value::Null => None,
            v => Some(v.get::<Locale>()),
        };
        p.with_mut(|p: &mut QueryParser| p.set_locale(locale))?;
        Ok(Value::Null)
    });
    r.call_fn(QP, "setTimeZone", 1, |p, a| {
        let zone = match &a[0] {
            Value::Null => None,
            v => Some(v.get::<TimeZone>()),
        };
        p.with_mut(|p: &mut QueryParser| p.set_time_zone(zone))?;
        Ok(Value::Null)
    });
    r.field_fn(OPERATOR, "AND", || Value::object(OPERATOR, Operator::And));
    r.field_fn(OPERATOR, "OR", || Value::object(OPERATOR, Operator::Or));
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

/// `new QueryParser("body", new StandardAnalyzer())` with the locale
/// `en-US` and the time zone `UTC` (date ranges must not depend on the
/// machine's defaults), bound to `$name`.
fn bind_parser(oracle: &mut ParseOracle, name: &str) {
    let analyzer = expr(
        "new",
        &["org.apache.lucene.analysis.standard.StandardAnalyzer"],
    );
    oracle.bind(name, &expr("new", &[QP, &lit("body"), &analyzer]));
    let var = format!("${name}");
    oracle.run(&expr(
        "call",
        &[
            "setLocale",
            &var,
            &expr(
                "static",
                &["java.util.Locale", "forLanguageTag", &lit("en-US")],
            ),
        ],
    ));
    oracle.run(&expr(
        "call",
        &[
            "setTimeZone",
            &var,
            &expr(
                "static",
                &["java.util.TimeZone", "getTimeZone", &lit("UTC")],
            ),
        ],
    ));
}

fn oracle() -> (ParseOracle, Vec<String>) {
    let mut oracle = ParseOracle::new(registry());
    bind_parser(&mut oracle, "or");
    bind_parser(&mut oracle, "and");
    oracle.run(&expr(
        "call",
        &[
            "setDefaultOperator",
            "$and",
            &expr("field", &[OPERATOR, "AND"]),
        ],
    ));
    oracle.run(&expr(
        "call",
        &["setAllowLeadingWildcard", "$and", "z:true"],
    ));
    let inputs: Vec<String> = corpus::classic_edge_cases()
        .into_iter()
        .chain(corpus::classic_random(SEED, 200))
        .collect();
    let mut ids = Vec::new();
    for (i, input) in inputs.iter().enumerate() {
        let units: Vec<u16> = input.encode_utf16().collect();
        for parser in ["or", "and"] {
            let id = format!("{parser}{i}");
            oracle.parse(
                &id,
                &expr("call", &["parse", &format!("${parser}"), "$input"]),
                &units,
                Some("body"),
            );
            ids.push(id);
        }
    }
    (oracle, ids)
}

/// Replaces the first `tostring` line of record `id` in `dump`.
fn perturb_parse(dump: &str, id: &str) -> String {
    let mut out = String::new();
    let mut in_record = false;
    let mut done = false;
    for l in dump.lines() {
        if l.starts_with("parse\t") {
            in_record = l == format!("parse\t{id}");
        }
        if in_record && !done && l.starts_with("tostring\t") {
            out.push_str(&format!("{l}X"));
            done = true;
        } else {
            out.push_str(l);
        }
        out.push('\n');
    }
    assert!(done, "record {id} has a query to perturb");
    out
}

#[test]
fn classic_query_parser_matches_lucene() {
    let (oracle, ids) = oracle();
    let java = oracle.java_dump();
    let rust = oracle.rust_dump();
    let errors = java.lines().filter(|l| l.starts_with("error\t")).count();
    println!(
        "{} parses, {errors} exceptions, {} dump lines",
        ids.len(),
        java.lines().count()
    );
    assert!(
        ids.len() > 500 && errors > 20 && errors < ids.len() / 2,
        "the corpus must parse and fail in both directions"
    );
    if let Err(diff) = compare(&java, &rust) {
        panic!("parser parity with Lucene 10.5.0 fails:\n{diff}");
    }

    // Positive control: a changed toString() fails at its query string.
    let perturbed = perturb_parse(&rust, "and2");
    let clean = compare(&java, &perturbed);
    assert!(clean.is_err(), "a perturbed parse must fail the comparison");
    let message = clean.unwrap_err();
    assert!(
        message.contains("record `parse\tand2`"),
        "the failure names the perturbed query string: {message}"
    );
}
