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

//! The parser parity of the classic family (rmp #475, #484): Lucene 10.5.0's
//! and Rucene's `classic.QueryParser`, `MultiFieldQueryParser`,
//! `ComplexPhraseQueryParser` and `ExtendableQueryParser` parse a seeded
//! corpus of query strings (fixed edge cases, date ranges, complex phrases,
//! extension fields, and generated strings) and give the same `toString()`,
//! `toString(field)` and class tree, or the same exception class and
//! message.
//!
//! * `classic_query_parser_matches_lucene`: two fixed configurations (T1's
//!   harness example), with a positive control.
//! * `classic_randomized_configurations_matches_lucene_<n>`: seeded random
//!   configurations of every `QueryParserBase`/`QueryBuilder` setter (default
//!   field, analyzer, operator, leading wildcards, whitespace splitting and
//!   auto phrases, position increments, slop, fuzzy settings, locale, time
//!   zone, date resolutions, rewrite method, graph and synonym-phrase
//!   switches, determinize work limit).
//! * `multi_field_query_parser_matches_lucene`: the instance parser with and
//!   without boosts under random configurations, and the three static
//!   `parse` forms.
//! * `complex_phrase_query_parser_matches_lucene`: in-order and unordered
//!   parsers, fuzzy prefix lengths.
//! * `extendable_query_parser_matches_lucene`: three extension delimiters,
//!   with the `ExtensionStub` of Lucene's tests registered.
//! * `parsers_without_default_field_match_lucene`: complex-phrase and
//!   extendable parsers whose default field is `null` (the NPEs of E6).
//!
//! The harness is shared with `rucene-queries` (`query_oracle`, rmp #475).

#[path = "../../rucene-queries/tests/query_oracle/mod.rs"]
mod query_oracle;

use std::collections::HashMap;
use std::sync::Arc;

use query_oracle::{compare, corpus, expr, lit, ParseOracle, Registry, Value};
use randomizedtesting::jdk::random::{JdkRandom, Random};
use rucene::analysis::standard::StandardAnalyzer;
use rucene::analysis::{Analyzer, CharArraySet};
use rucene::document::Resolution;
use rucene::error::Result;
use rucene::index::Term;
use rucene::search::boolean_clause::Occur;
use rucene::search::multi_term_query::{
    constant_score_blended_rewrite, constant_score_boolean_rewrite, constant_score_rewrite,
    scoring_boolean_rewrite, RewriteMethod,
};
use rucene::search::term_query::TermQuery;
use rucene::search::Query;
use rucene_queryparser::classic::multi_field_query_parser::MultiFieldQueryParser;
use rucene_queryparser::classic::query_parser::{Operator, QueryParser, QueryParserOps};
use rucene_queryparser::complex_phrase::complex_phrase_query_parser::ComplexPhraseQueryParser;
use rucene_queryparser::ext::extendable_query_parser::ExtendableQueryParser;
use rucene_queryparser::ext::extension_query::ExtensionQuery;
use rucene_queryparser::ext::extensions::Extensions;
use rucene_queryparser::ext::parser_extension::ParserExtension;
use rucene_queryparser::flexible::standard::common_query_parser_configuration::CommonQueryParserConfiguration;
use rucene_queryparser::jdk::locale::Locale;
use rucene_queryparser::jdk::time_zone::TimeZone;

/// The seed of the generated query strings.
const SEED: i64 = 0x0475_c1a5;

const QP: &str = "org.apache.lucene.queryparser.classic.QueryParser";
const MFQP: &str = "org.apache.lucene.queryparser.classic.MultiFieldQueryParser";
const CPQP: &str = "org.apache.lucene.queryparser.complexPhrase.ComplexPhraseQueryParser";
const EQP: &str = "org.apache.lucene.queryparser.ext.ExtendableQueryParser";
const EXTENSIONS: &str = "org.apache.lucene.queryparser.ext.Extensions";
const OPERATOR: &str = "org.apache.lucene.queryparser.classic.QueryParser$Operator";
const RESOLUTION: &str = "org.apache.lucene.document.DateTools$Resolution";
const MTQ: &str = "org.apache.lucene.search.MultiTermQuery";
const OCCUR: &str = "org.apache.lucene.search.BooleanClause$Occur";
const STANDARD: &str = "org.apache.lucene.analysis.standard.StandardAnalyzer";
const CHAR_ARRAY_SET: &str = "org.apache.lucene.analysis.CharArraySet";
const HASH_MAP: &str = "java.util.HashMap";
const ORACLE: &str = "org.apache.lucene.rucene.codec.QueryOracle";

/// `QueryOracle.termExtension()`: Lucene's test `ExtensionStub`.
#[derive(Debug)]
struct TermExtension;

impl ParserExtension for TermExtension {
    fn parse(&self, components: ExtensionQuery<'_>) -> Result<Option<Arc<dyn Query>>> {
        Ok(Some(Arc::new(TermQuery::new(
            Term::from_text_nullable_field(components.field(), components.raw_query_string()),
        ))))
    }
}

fn opt_analyzer(v: &Value) -> Option<Arc<dyn Analyzer>> {
    match v {
        Value::Null => None,
        v => Some(v.get::<Arc<dyn Analyzer>>()),
    }
}

/// The setters of `QueryParserBase` and `QueryBuilder` (and `parse`) on
/// receivers of class `class` held as `T`.
fn register_common<T: QueryParserOps + 'static>(r: &mut Registry, class: &'static str) {
    r.call_fn(class, "parse", 1, |p, a| {
        let input = a[0].as_str().to_string();
        let q = p.with_mut(|p: &mut T| p.parse(&input))?;
        Ok(Value::query(q))
    });
    r.call_fn(class, "setDefaultOperator", 1, |p, a| {
        let op = a[0].get::<Operator>();
        p.with_mut(|p: &mut T| p.set_default_operator(op));
        Ok(Value::Null)
    });
    r.call_fn(class, "setAllowLeadingWildcard", 1, |p, a| {
        let v = a[0].as_bool();
        p.with_mut(|p: &mut T| p.set_allow_leading_wildcard(v))?;
        Ok(Value::Null)
    });
    r.call_fn(class, "setSplitOnWhitespace", 1, |p, a| {
        let v = a[0].as_bool();
        p.with_mut(|p: &mut T| p.set_split_on_whitespace(v))?;
        Ok(Value::Null)
    });
    r.call_fn(class, "setAutoGeneratePhraseQueries", 1, |p, a| {
        let v = a[0].as_bool();
        p.with_mut(|p: &mut T| p.set_auto_generate_phrase_queries(v))?;
        Ok(Value::Null)
    });
    r.call_fn(class, "setEnablePositionIncrements", 1, |p, a| {
        let v = a[0].as_bool();
        p.with_mut(|p: &mut T| {
            CommonQueryParserConfiguration::set_enable_position_increments(p, v)
        })?;
        Ok(Value::Null)
    });
    r.call_fn(class, "setPhraseSlop", 1, |p, a| {
        let v = a[0].as_int();
        p.with_mut(|p: &mut T| p.set_phrase_slop(v))?;
        Ok(Value::Null)
    });
    r.call_fn(class, "setFuzzyMinSim", 1, |p, a| {
        let v = a[0].as_float();
        p.with_mut(|p: &mut T| p.set_fuzzy_min_sim(v))?;
        Ok(Value::Null)
    });
    r.call_fn(class, "setFuzzyPrefixLength", 1, |p, a| {
        let v = a[0].as_int();
        p.with_mut(|p: &mut T| p.set_fuzzy_prefix_length(v))?;
        Ok(Value::Null)
    });
    r.call_fn(class, "setLocale", 1, |p, a| {
        let v = match &a[0] {
            Value::Null => None,
            v => Some(v.get::<Locale>()),
        };
        p.with_mut(|p: &mut T| p.set_locale(v))?;
        Ok(Value::Null)
    });
    r.call_fn(class, "setTimeZone", 1, |p, a| {
        let v = match &a[0] {
            Value::Null => None,
            v => Some(v.get::<TimeZone>()),
        };
        p.with_mut(|p: &mut T| p.set_time_zone(v))?;
        Ok(Value::Null)
    });
    r.call_fn(class, "setDateResolution", 1, |p, a| {
        let v = match &a[0] {
            Value::Null => None,
            v => Some(v.get::<Resolution>()),
        };
        p.with_mut(|p: &mut T| CommonQueryParserConfiguration::set_date_resolution(p, v))?;
        Ok(Value::Null)
    });
    r.call_fn(class, "setDateResolution", 2, |p, a| {
        let field = a[0].as_opt_str().map(str::to_string);
        let v = match &a[1] {
            Value::Null => None,
            v => Some(v.get::<Resolution>()),
        };
        p.with_mut(|p: &mut T| p.set_date_resolution_for_field(field.as_deref(), v))?;
        Ok(Value::Null)
    });
    r.call_fn(class, "setMultiTermRewriteMethod", 1, |p, a| {
        let v = match &a[0] {
            Value::Null => None,
            v => Some(v.get::<Arc<dyn RewriteMethod>>()),
        };
        p.with_mut(|p: &mut T| p.set_multi_term_rewrite_method(v))?;
        Ok(Value::Null)
    });
    r.call_fn(
        class,
        "setAutoGenerateMultiTermSynonymsPhraseQuery",
        1,
        |p, a| {
            let v = a[0].as_bool();
            p.with_mut(|p: &mut T| {
                p.query_parser_base_mut()
                    .query_builder_mut()
                    .set_auto_generate_multi_term_synonyms_phrase_query(v)
            });
            Ok(Value::Null)
        },
    );
    r.call_fn(class, "setEnableGraphQueries", 1, |p, a| {
        let v = a[0].as_bool();
        p.with_mut(|p: &mut T| {
            p.query_parser_base_mut()
                .query_builder_mut()
                .set_enable_graph_queries(v)
        });
        Ok(Value::Null)
    });
    r.call_fn(class, "setDeterminizeWorkLimit", 1, |p, a| {
        let v = a[0].as_int();
        p.with_mut(|p: &mut T| p.set_determinize_work_limit(v));
        Ok(Value::Null)
    });
}

fn strings_of(v: &Value) -> Vec<String> {
    v.as_values()
        .iter()
        .map(|s| s.as_str().to_string())
        .collect()
}

fn occurs_of(v: &Value) -> Vec<Occur> {
    v.as_values().iter().map(|o| o.get::<Occur>()).collect()
}

/// [`Registry::core`] with the members of the classic family the specs
/// call.
fn registry() -> Registry {
    let mut r = Registry::core();
    r.new_fn(QP, 2, |a| {
        Ok(Value::object(
            QP,
            QueryParser::new(a[0].as_opt_str(), opt_analyzer(&a[1]))?,
        ))
    });
    register_common::<QueryParser>(&mut r, QP);

    r.new_fn(MFQP, 2, |a| {
        Ok(Value::object(
            MFQP,
            MultiFieldQueryParser::new(strings_of(&a[0]), opt_analyzer(&a[1]))?,
        ))
    });
    r.new_fn(MFQP, 3, |a| {
        let boosts = match &a[2] {
            Value::Null => None,
            v => Some(v.get::<HashMap<String, f32>>()),
        };
        Ok(Value::object(
            MFQP,
            MultiFieldQueryParser::with_boosts(strings_of(&a[0]), opt_analyzer(&a[1]), boosts)?,
        ))
    });
    register_common::<MultiFieldQueryParser>(&mut r, MFQP);
    r.static_fn(MFQP, "parse", 3, |a| {
        let queries = strings_of(&a[0]);
        let fields = strings_of(&a[1]);
        let q: Vec<&str> = queries.iter().map(String::as_str).collect();
        let f: Vec<&str> = fields.iter().map(String::as_str).collect();
        Ok(Value::query(MultiFieldQueryParser::parse_queries(
            &q,
            &f,
            opt_analyzer(&a[2]),
        )?))
    });
    r.static_fn(MFQP, "parse", 4, |a| {
        let fields = strings_of(&a[1]);
        let f: Vec<&str> = fields.iter().map(String::as_str).collect();
        let flags = occurs_of(&a[2]);
        let q = match &a[0] {
            Value::Str(query) => {
                MultiFieldQueryParser::parse_with_flags(query, &f, &flags, opt_analyzer(&a[3]))?
            }
            v => {
                let queries = strings_of(v);
                let q: Vec<&str> = queries.iter().map(String::as_str).collect();
                MultiFieldQueryParser::parse_queries_with_flags(
                    &q,
                    &f,
                    &flags,
                    opt_analyzer(&a[3]),
                )?
            }
        };
        Ok(Value::query(q))
    });

    r.new_fn(CPQP, 2, |a| {
        Ok(Value::object(
            CPQP,
            ComplexPhraseQueryParser::new(a[0].as_opt_str(), opt_analyzer(&a[1]))?,
        ))
    });
    register_common::<ComplexPhraseQueryParser>(&mut r, CPQP);
    r.call_fn(CPQP, "setInOrder", 1, |p, a| {
        let v = a[0].as_bool();
        p.with_mut(|p: &mut ComplexPhraseQueryParser| p.set_in_order(v));
        Ok(Value::Null)
    });

    r.new_fn(EXTENSIONS, 1, |a| {
        let c = char::from_u32(u32::from(match &a[0] {
            Value::Char(c) => *c,
            other => query_oracle::harness_fail(format!("expected a char, found {other:?}")),
        }))
        .expect("a BMP delimiter");
        Ok(Value::object(
            EXTENSIONS,
            Arc::new(Extensions::with_delimiter(c)),
        ))
    });
    r.call_fn(EXTENSIONS, "add", 2, |e, a| {
        let key = a[0].as_opt_str().map(str::to_string);
        let ext = match &a[1] {
            Value::Null => None,
            v => Some(v.get::<Arc<dyn ParserExtension>>()),
        };
        e.with_mut(|e: &mut Arc<Extensions>| e.add(key.as_deref(), ext));
        Ok(Value::Null)
    });
    r.static_fn(ORACLE, "termExtension", 0, |_| {
        Ok(Value::object(
            "org.apache.lucene.rucene.codec.QueryOracle$1",
            Arc::new(TermExtension) as Arc<dyn ParserExtension>,
        ))
    });
    r.new_fn(EQP, 2, |a| {
        Ok(Value::object(
            EQP,
            ExtendableQueryParser::new(a[0].as_opt_str(), opt_analyzer(&a[1]))?,
        ))
    });
    r.new_fn(EQP, 3, |a| {
        let ext = a[2].get::<Arc<Extensions>>();
        Ok(Value::object(
            EQP,
            ExtendableQueryParser::with_extensions(a[0].as_opt_str(), opt_analyzer(&a[1]), ext)?,
        ))
    });
    register_common::<ExtendableQueryParser>(&mut r, EQP);

    r.new_fn(HASH_MAP, 0, |_| {
        Ok(Value::object(HASH_MAP, HashMap::<String, f32>::new()))
    });
    r.call_fn(HASH_MAP, "put", 2, |m, a| {
        let k = a[0].as_str().to_string();
        let v = a[1].as_float();
        let old = m.with_mut(|m: &mut HashMap<String, f32>| m.insert(k, v));
        Ok(old.map_or(Value::Null, Value::Float))
    });
    r.new_fn(CHAR_ARRAY_SET, 2, |a| {
        Ok(Value::object(
            CHAR_ARRAY_SET,
            CharArraySet::from_iter(strings_of(&a[0]), a[1].as_bool()),
        ))
    });
    r.new_fn(STANDARD, 1, |a| {
        let set = a[0].get::<CharArraySet>();
        Ok(Value::object(
            STANDARD,
            Arc::new(StandardAnalyzer::with_stopwords(set)) as Arc<dyn Analyzer>,
        ))
    });

    r.field_fn(OPERATOR, "AND", || Value::object(OPERATOR, Operator::And));
    r.field_fn(OPERATOR, "OR", || Value::object(OPERATOR, Operator::Or));
    for (name, res) in [
        ("YEAR", Resolution::YEAR),
        ("MONTH", Resolution::MONTH),
        ("DAY", Resolution::DAY),
        ("HOUR", Resolution::HOUR),
        ("MINUTE", Resolution::MINUTE),
        ("SECOND", Resolution::SECOND),
        ("MILLISECOND", Resolution::MILLISECOND),
    ] {
        r.field_fn(RESOLUTION, name, move || Value::object(RESOLUTION, res));
    }
    type RewriteFn = fn() -> Arc<dyn RewriteMethod>;
    let rewrites: [(&str, RewriteFn); 4] = [
        ("CONSTANT_SCORE_REWRITE", constant_score_rewrite),
        (
            "CONSTANT_SCORE_BLENDED_REWRITE",
            constant_score_blended_rewrite,
        ),
        (
            "CONSTANT_SCORE_BOOLEAN_REWRITE",
            constant_score_boolean_rewrite,
        ),
        ("SCORING_BOOLEAN_REWRITE", scoring_boolean_rewrite),
    ];
    for (name, f) in rewrites {
        r.field_fn(MTQ, name, move || Value::object(MTQ, f()));
    }
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

fn standard_analyzer() -> String {
    expr("new", &[STANDARD])
}

fn stop_analyzer() -> String {
    expr(
        "new",
        &[
            STANDARD,
            &expr(
                "new",
                &[
                    CHAR_ARRAY_SET,
                    &expr("list", &[&lit("the"), &lit("of"), &lit("and"), &lit("to")]),
                    "z:true",
                ],
            ),
        ],
    )
}

fn call(method: &str, receiver: &str, args: &[&str]) -> String {
    let mut all = vec![method, receiver];
    all.extend_from_slice(args);
    expr("call", &all)
}

fn static_call(class: &str, method: &str, args: &[&str]) -> String {
    let mut all = vec![class, method];
    all.extend_from_slice(args);
    expr("static", &all)
}

/// `new QueryParser("body", new StandardAnalyzer())` with the locale
/// `en-US` and the time zone `UTC` (date ranges must not depend on the
/// machine's defaults), bound to `$name`.
fn bind_parser(oracle: &mut ParseOracle, name: &str) {
    oracle.bind(
        name,
        &expr("new", &[QP, &lit("body"), &standard_analyzer()]),
    );
    let var = format!("${name}");
    oracle.run(&call(
        "setLocale",
        &var,
        &[&static_call(
            "java.util.Locale",
            "forLanguageTag",
            &[&lit("en-US")],
        )],
    ));
    oracle.run(&call(
        "setTimeZone",
        &var,
        &[&static_call(
            "java.util.TimeZone",
            "getTimeZone",
            &[&lit("UTC")],
        )],
    ));
}

fn oracle() -> (ParseOracle, Vec<String>) {
    let mut oracle = ParseOracle::new(registry());
    bind_parser(&mut oracle, "or");
    bind_parser(&mut oracle, "and");
    oracle.run(&call(
        "setDefaultOperator",
        "$and",
        &[&expr("field", &[OPERATOR, "AND"])],
    ));
    oracle.run(&call("setAllowLeadingWildcard", "$and", &["z:true"]));
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
                &call("parse", &format!("${parser}"), &["$input"]),
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

// ---------------------------------------------------------------------------
// Randomised configurations
// ---------------------------------------------------------------------------

/// Locales whose `SHORT` date patterns, digits and calendars differ.
const LOCALES: [&str; 14] = [
    "en-US",
    "en-GB",
    "de-DE",
    "fr-FR",
    "ja-JP",
    "ar-EG",
    "th-TH",
    "hi-IN",
    "tr-TR",
    "zh-CN",
    "pt-PT",
    "ru-RU",
    "fa-IR",
    "ja-JP-u-ca-japanese",
];

/// Time zones with odd offsets and recent rule changes.
const ZONES: [&str; 7] = [
    "UTC",
    "Europe/Lisbon",
    "America/New_York",
    "Asia/Kolkata",
    "Pacific/Chatham",
    "Africa/Casablanca",
    "Australia/Lord_Howe",
];

const RESOLUTIONS: [&str; 7] = [
    "YEAR",
    "MONTH",
    "DAY",
    "HOUR",
    "MINUTE",
    "SECOND",
    "MILLISECOND",
];

const REWRITES: [&str; 4] = [
    "CONSTANT_SCORE_REWRITE",
    "CONSTANT_SCORE_BLENDED_REWRITE",
    "CONSTANT_SCORE_BOOLEAN_REWRITE",
    "SCORING_BOOLEAN_REWRITE",
];

fn pick<'a>(random: &mut JdkRandom, items: &[&'a str]) -> &'a str {
    items[random.next_int_bound(items.len() as i32) as usize]
}

/// Date ranges in several locales' `SHORT` forms, and strings that are no
/// date at all.
fn date_inputs() -> Vec<String> {
    [
        "[1/2/2002 TO 4/5/2003]",
        "{12/31/2020 TO 1/1/2021}",
        "title:[2/3/04 TO 5/6/07]",
        "[02.01.2002 TO 04.01.2002]",
        "{02.01.2002 TO 04.01.2002]",
        "[2002-01-02 TO 2002-01-04]",
        "[2002/01/02 TO 2002/01/04]",
        "[02/01/2002 TO 04/01/2002}",
        "[1/1/1500 TO 1/1/1600]",
        "[\"1/2/2002\" TO \"4/5/2003\"]",
        "[1/2/2002 TO *]",
        "[* TO 4/5/2003]",
        "body:{3/3/33 TO 4/4/44}",
        "[13/45/2002 TO 99/99/99]",
        "[٠١/٠٢/٢٠٠٢ TO ٠٤/٠٥/٢٠٠٣]",
        "[2002年1月2日 TO 2002年1月4日]",
        "[H14/01/02 TO H14/01/04]",
        "[2.1.2002 TO 4.1.2002]",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Binds `$name` to a parser of `class` built by `ctor` and applies a
/// seeded random configuration; returns its default field.
fn configure(oracle: &mut ParseOracle, random: &mut JdkRandom, name: &str, ctor: &str) {
    oracle.bind(name, ctor);
    let var = format!("${name}");
    let set = |oracle: &mut ParseOracle, method: &str, arg: &str| {
        oracle.run(&call(method, &var, &[arg]));
    };
    if random.next_boolean() {
        set(
            oracle,
            "setDefaultOperator",
            &expr("field", &[OPERATOR, "AND"]),
        );
    }
    set(
        oracle,
        "setAllowLeadingWildcard",
        &format!("z:{}", random.next_boolean()),
    );
    let split = random.next_boolean();
    set(oracle, "setSplitOnWhitespace", &format!("z:{split}"));
    if split && random.next_boolean() {
        set(oracle, "setAutoGeneratePhraseQueries", "z:true");
    }
    set(
        oracle,
        "setEnablePositionIncrements",
        &format!("z:{}", random.next_boolean()),
    );
    set(
        oracle,
        "setPhraseSlop",
        &format!("i:{}", random.next_int_bound(4)),
    );
    set(
        oracle,
        "setFuzzyMinSim",
        pick(random, &["f:2.0", "f:1.0", "f:0.5", "f:0.75"]),
    );
    set(
        oracle,
        "setFuzzyPrefixLength",
        &format!("i:{}", random.next_int_bound(3)),
    );
    set(
        oracle,
        "setLocale",
        &static_call(
            "java.util.Locale",
            "forLanguageTag",
            &[&lit(pick(random, &LOCALES))],
        ),
    );
    set(
        oracle,
        "setTimeZone",
        &static_call(
            "java.util.TimeZone",
            "getTimeZone",
            &[&lit(pick(random, &ZONES))],
        ),
    );
    if random.next_boolean() {
        set(
            oracle,
            "setDateResolution",
            &expr("field", &[RESOLUTION, pick(random, &RESOLUTIONS)]),
        );
    }
    if random.next_boolean() {
        oracle.run(&call(
            "setDateResolution",
            &var,
            &[
                &lit("title"),
                &expr("field", &[RESOLUTION, pick(random, &RESOLUTIONS)]),
            ],
        ));
    }
    if random.next_boolean() {
        set(
            oracle,
            "setMultiTermRewriteMethod",
            &expr("field", &[MTQ, pick(random, &REWRITES)]),
        );
    }
    set(
        oracle,
        "setAutoGenerateMultiTermSynonymsPhraseQuery",
        &format!("z:{}", random.next_boolean()),
    );
    set(
        oracle,
        "setEnableGraphQueries",
        &format!("z:{}", random.next_boolean()),
    );
    if random.next_int_bound(4) == 0 {
        set(
            oracle,
            "setDeterminizeWorkLimit",
            pick(random, &["i:1", "i:40", "i:200"]),
        );
    }
}

fn analyzer_of(random: &mut JdkRandom) -> String {
    if random.next_boolean() {
        standard_analyzer()
    } else {
        stop_analyzer()
    }
}

/// Parses every input with each parser variable of `parsers`, ids
/// `<prefix><parser><i>`.
fn parse_all(
    oracle: &mut ParseOracle,
    parsers: &[&str],
    inputs: &[String],
    field: Option<&str>,
) -> usize {
    let mut n = 0;
    for (i, input) in inputs.iter().enumerate() {
        let units: Vec<u16> = input.encode_utf16().collect();
        for parser in parsers {
            oracle.parse(
                &format!("{parser}{i}"),
                &call("parse", &format!("${parser}"), &["$input"]),
                &units,
                field,
            );
            n += 1;
        }
    }
    n
}

/// Runs both sides and fails with the first difference; checks the corpus
/// exercises both outcomes.
fn assert_parity(oracle: &ParseOracle, what: &str, min_records: usize) {
    let java = oracle.java_dump();
    let rust = oracle.rust_dump();
    let records = java.lines().filter(|l| l.starts_with("parse\t")).count();
    let errors = java.lines().filter(|l| l.starts_with("error\t")).count();
    println!(
        "{what}: {records} parses, {errors} exceptions, {} dump lines",
        java.lines().count()
    );
    assert!(
        records >= min_records && errors > 0,
        "{what}: the corpus must parse and fail"
    );
    if let Err(diff) = compare(&java, &rust) {
        panic!("{what}: parser parity with Lucene 10.5.0 fails:\n{diff}");
    }
}

fn randomized_shard(shard: i64) {
    let mut random = JdkRandom::new(SEED ^ (shard * 0x9E37_79B9));
    let mut oracle = ParseOracle::new(registry());
    let mut names = Vec::new();
    for c in 0..3 {
        let name = format!("c{c}");
        let field = pick(&mut random, &["body", "title"]);
        let analyzer = analyzer_of(&mut random);
        configure(
            &mut oracle,
            &mut random,
            &name,
            &expr("new", &[QP, &lit(field), &analyzer]),
        );
        names.push(name);
    }
    let inputs: Vec<String> = corpus::classic_edge_cases()
        .into_iter()
        .chain(date_inputs())
        .chain(corpus::classic_random(SEED + shard, 120))
        .collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let n = parse_all(&mut oracle, &refs, &inputs, Some("body"));
    assert_parity(&oracle, &format!("classic shard {shard}"), n);
}

#[test]
fn classic_randomized_configurations_match_lucene_0() {
    randomized_shard(0);
}

#[test]
fn classic_randomized_configurations_match_lucene_1() {
    randomized_shard(1);
}

#[test]
fn classic_randomized_configurations_match_lucene_2() {
    randomized_shard(2);
}

#[test]
fn classic_randomized_configurations_match_lucene_3() {
    randomized_shard(3);
}

// ---------------------------------------------------------------------------
// MultiFieldQueryParser
// ---------------------------------------------------------------------------

fn string_array(items: &[&str]) -> String {
    let lits: Vec<String> = items.iter().map(|s| lit(s)).collect();
    let mut args: Vec<&str> = vec!["java.lang.String"];
    args.extend(lits.iter().map(String::as_str));
    expr("array", &args)
}

fn occur_array(items: &[&str]) -> String {
    let fields: Vec<String> = items.iter().map(|o| expr("field", &[OCCUR, o])).collect();
    let mut args: Vec<&str> = vec![OCCUR];
    args.extend(fields.iter().map(String::as_str));
    expr("array", &args)
}

#[test]
fn multi_field_query_parser_matches_lucene() {
    let mut random = JdkRandom::new(SEED ^ 0x4D46);
    let mut oracle = ParseOracle::new(registry());
    oracle.bind("an", &stop_analyzer());
    oracle.bind("boosts", &expr("new", &[HASH_MAP]));
    oracle.run(&call("put", "$boosts", &[&lit("title"), "f:5.0"]));
    oracle.run(&call("put", "$boosts", &[&lit("body"), "f:10.0"]));
    let fields = string_array(&["title", "body"]);
    configure(
        &mut oracle,
        &mut random,
        "m0",
        &expr("new", &[MFQP, &fields, "$an"]),
    );
    configure(
        &mut oracle,
        &mut random,
        "m1",
        &expr("new", &[MFQP, &fields, &standard_analyzer(), "$boosts"]),
    );
    configure(
        &mut oracle,
        &mut random,
        "m2",
        &expr(
            "new",
            &[
                MFQP,
                &string_array(&["title", "body", "tag"]),
                &standard_analyzer(),
            ],
        ),
    );
    let inputs: Vec<String> = corpus::classic_edge_cases()
        .into_iter()
        .chain(corpus::classic_random(SEED + 7, 120))
        .collect();
    let mut n = parse_all(&mut oracle, &["m0", "m1", "m2"], &inputs, Some("body"));
    // The static forms: queries per field, flags per field, mismatched lengths.
    let flags_ok = occur_array(&["MUST", "MUST_NOT"]);
    let flags_short = occur_array(&["SHOULD"]);
    for (i, input) in inputs.iter().take(80).enumerate() {
        let units: Vec<u16> = input.encode_utf16().collect();
        let queries = expr(
            "array",
            &["java.lang.String", "$input", &lit("alpha bravo")],
        );
        let specs = [
            static_call(MFQP, "parse", &[&queries, &fields, "$an"]),
            static_call(MFQP, "parse", &["$input", &fields, &flags_ok, "$an"]),
            static_call(MFQP, "parse", &[&queries, &fields, &flags_ok, "$an"]),
            static_call(MFQP, "parse", &["$input", &fields, &flags_short, "$an"]),
            static_call(MFQP, "parse", &[&queries, &string_array(&["title"]), "$an"]),
        ];
        for (k, spec) in specs.iter().enumerate() {
            oracle.parse(&format!("s{k}_{i}"), spec, &units, Some("body"));
            n += 1;
        }
    }
    assert_parity(&oracle, "MultiFieldQueryParser", n);
}

// ---------------------------------------------------------------------------
// ComplexPhraseQueryParser
// ---------------------------------------------------------------------------

/// The syntax `ComplexPhraseQueryParser` adds inside quotes.
fn complex_phrase_inputs() -> Vec<String> {
    [
        "\"john smith\"",
        "\"j*   smyth~\"",
        "\"(jo* -john)  smith\"",
        "\"jo*  smith\"~2",
        "\"jo* [sma TO smZ]\" ",
        "\"john\"",
        "\"(john OR johathon)  smith\"",
        "\"(john OR nosuchword*)  smith\"",
        "\"(jo* -john) smyth~\"",
        "\"john  nosuchword*\"",
        "\"jo*  id:1 smith\"",
        "\"jo* \"smith\" \"",
        "\"joh*\" \"tom\"",
        "+\"j*\" +\"tom\"",
        "\"jo*\" \"[sma TO smZ]\" ",
        "+\"j*hn\" +\"sm*h\"",
        "\"smith jo*\"~2",
        "name:\"john smith\"",
        "role:\"p* manager\" AND name:jack*",
        "name:\"j* smyth~\"~5",
        "(\"john^3 smit*\"~4)^2",
        "\"aaa* bbb*\"",
        "\"alpha bravo\"~1 \"charlie*\"",
        "\"alp* (bravo OR charlie)\"",
        "\"alp* -bravo\"",
        "\"alp* NOT bravo\"",
        "\"a? b??\"",
        "\"alpha~1 bravo~2\"",
        "\"/al.*a/ bravo\"",
        "\"alpha \"",
        "\" alpha\"~3^2",
        "\"\"",
        "\"*\"",
        "\"alpha body:bravo\"",
        "title:\"alpha body:bravo\"",
        "\"[a TO c} {d TO f]\"",
        "\"alpha (bravo (charlie delta))\"",
        "\"alpha\"~0",
        "\"alpha bravo\" \"charlie delta\" \"echo* f*\"",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

#[test]
fn complex_phrase_query_parser_matches_lucene() {
    let mut random = JdkRandom::new(SEED ^ 0x4350);
    let mut oracle = ParseOracle::new(registry());
    for (c, in_order) in [(0, true), (1, false), (2, true)] {
        let name = format!("p{c}");
        let analyzer = analyzer_of(&mut random);
        configure(
            &mut oracle,
            &mut random,
            &name,
            &expr("new", &[CPQP, &lit("name"), &analyzer]),
        );
        oracle.run(&call(
            "setInOrder",
            &format!("${name}"),
            &[&format!("z:{in_order}")],
        ));
    }
    let inputs: Vec<String> = complex_phrase_inputs()
        .into_iter()
        .chain(corpus::classic_edge_cases())
        .chain(corpus::classic_random(SEED + 11, 100))
        .collect();
    let n = parse_all(&mut oracle, &["p0", "p1", "p2"], &inputs, Some("name"));
    assert_parity(&oracle, "ComplexPhraseQueryParser", n);
}

// ---------------------------------------------------------------------------
// ExtendableQueryParser
// ---------------------------------------------------------------------------

/// Extension fields for the delimiters `:`, `-` and `|`, escaped and not.
fn extension_inputs() -> Vec<String> {
    [
        "aField\\:testExt:foo bar",
        "aField\\:testExt:\"foo \\& bar\"",
        "\\:testExt:\"foo \\& bar\"",
        "aField:testExt:\"foo \\& bar\"",
        "afield\\:testExt:\"foo \\& bar\"",
        "aField\\-testExt:foo bar",
        "\\-testExt:\"foo \\& bar\"",
        "afield\\-testExt:\"foo \\& bar\"",
        "aField\\|testExt:foo bar",
        "\\|testExt:\"foo \\& bar\"",
        "afield\\|testExt:\"foo \\& bar\"",
        "aField\\:otherExt:foo",
        "aField\\:\\:testExt:foo",
        "testExt:foo",
        "\\:testExt:foo* bar~",
        "\\:testExt:[a TO b]",
        "a-b:foo",
        "a|b:foo",
        "aField\\:testExt:(foo bar)",
        "+aField\\:testExt:foo -\\:testExt:bar",
        "aField\\:testExt:\"foo bar\"~2^3",
        "\\:testExt:",
        "\\::foo",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

#[test]
fn extendable_query_parser_matches_lucene() {
    let mut random = JdkRandom::new(SEED ^ 0x4551);
    let mut oracle = ParseOracle::new(registry());
    let ext = static_call(ORACLE, "termExtension", &[]);
    let mut names = Vec::new();
    for (k, delimiter) in ["c:003a", "c:002d", "c:007c"].iter().enumerate() {
        let ext_var = format!("ext{k}");
        oracle.bind(&ext_var, &expr("new", &[EXTENSIONS, delimiter]));
        oracle.run(&call(
            "add",
            &format!("${ext_var}"),
            &[&lit("testExt"), &ext],
        ));
        let name = format!("e{k}");
        let analyzer = analyzer_of(&mut random);
        configure(
            &mut oracle,
            &mut random,
            &name,
            &expr(
                "new",
                &[EQP, &lit("body"), &analyzer, &format!("${ext_var}")],
            ),
        );
        names.push(name);
    }
    // The default `Extensions` (no extension registered).
    configure(
        &mut oracle,
        &mut random,
        "e3",
        &expr("new", &[EQP, &lit("body"), &standard_analyzer()]),
    );
    names.push("e3".to_string());
    let inputs: Vec<String> = extension_inputs()
        .into_iter()
        .chain(corpus::classic_edge_cases())
        .chain(corpus::classic_random(SEED + 13, 100))
        .collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let n = parse_all(&mut oracle, &refs, &inputs, Some("body"));
    assert_parity(&oracle, "ExtendableQueryParser", n);
}

// ---------------------------------------------------------------------------
// No default field
// ---------------------------------------------------------------------------

#[test]
fn parsers_without_default_field_match_lucene() {
    let mut oracle = ParseOracle::new(registry());
    oracle.bind("cp", &expr("new", &[CPQP, "null", &standard_analyzer()]));
    oracle.bind("ex", &expr("new", &[EQP, "null", &standard_analyzer()]));
    oracle.bind("ext", &expr("new", &[EXTENSIONS, "c:003a"]));
    oracle.run(&call(
        "add",
        "$ext",
        &[&lit("testExt"), &static_call(ORACLE, "termExtension", &[])],
    ));
    oracle.bind(
        "ex2",
        &expr("new", &[EQP, "null", &standard_analyzer(), "$ext"]),
    );
    for p in ["cp", "ex", "ex2"] {
        oracle.run(&call(
            "setAllowLeadingWildcard",
            &format!("${p}"),
            &["z:true"],
        ));
    }
    let inputs: Vec<String> = complex_phrase_inputs()
        .into_iter()
        .chain(extension_inputs())
        .chain(corpus::classic_edge_cases())
        .collect();
    let n = parse_all(&mut oracle, &["cp", "ex", "ex2"], &inputs, None);
    assert_parity(&oracle, "parsers without a default field", n);
}
