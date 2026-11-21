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

//! The Java-oracle portability test of `CommonTermsQuery` and
//! `queries.mlt` (rmp #477): `CommonTermsQuery` (occurs, relative and
//! absolute maximum term frequency, low and high minimum-should-match, only
//! high or only low frequency terms, absent terms, a single term),
//! `MoreLikeThis.like` term selection (from a document of the index with
//! the indexed fields or named fields, and from text readers, under minimum
//! term and document frequency, word length, maximum query terms and boost
//! settings) and `MoreLikeThisQuery`, over a seeded index Rucene writes
//! (`OracleIndex::standard`: three segments, deleted documents).
//!
//! For each query Lucene 10.5.0 and Rucene must give the same `toString()`
//! (for `MoreLikeThis.like` it lists the selected terms and boosts in queue
//! order) and class tree before and after rewrite, count, hits with
//! bit-exact `float` scores and `explain` of every document.
//!
//! Sharded over several tests so nextest runs them in parallel; each writes
//! its own index and runs its own JVM in its own directory under `TMPDIR`.

mod query_oracle;

use std::sync::Arc;

use query_oracle::{compare, expr, lit, OracleIndex, Registry, SearchOracle, Value};
use rucene::analysis::{Analyzer, CharReader, StringCharReader};
use rucene::index::{IndexReader, Term};
use rucene::search::boolean_clause::Occur;
use rucene::search::query::Query;
use rucene_queries::common_terms_query::CommonTermsQuery;
use rucene_queries::mlt::more_like_this::MoreLikeThis;
use rucene_queries::mlt::more_like_this_query::MoreLikeThisQuery;

/// The seed of the index.
const SEED: i64 = 0x0477_5eed;

/// The number of documents of the index.
const NUM_DOCS: usize = 60;

const CTQ: &str = "org.apache.lucene.queries.CommonTermsQuery";
const MLT: &str = "org.apache.lucene.queries.mlt.MoreLikeThis";
const MLTQ: &str = "org.apache.lucene.queries.mlt.MoreLikeThisQuery";
const READER: &str = "java.io.StringReader";

/// Replaces the query a value holds by `f` of a clone of it, as a setter
/// changes the Java object in place.
fn mutate<T: Query + Clone + 'static>(v: &Value, f: impl FnOnce(&mut T)) {
    v.with_mut(|q: &mut Arc<dyn Query>| {
        let mut c = q
            .as_any()
            .downcast_ref::<T>()
            .expect("the query class the binding names")
            .clone();
        f(&mut c);
        *q = Arc::new(c);
    });
}

fn registry() -> Registry {
    let mut r = Registry::core();
    r.new_fn(CTQ, 3, |a| {
        Ok(Value::query(Arc::new(CommonTermsQuery::new(
            a[0].get::<Occur>(),
            a[1].get::<Occur>(),
            a[2].as_float(),
        )?)))
    });
    r.call_fn(CTQ, "add", 1, |q, a| {
        let t = a[0].get::<Term>();
        mutate(q, |c: &mut CommonTermsQuery| c.add(t));
        Ok(Value::Null)
    });
    r.call_fn(CTQ, "setLowFreqMinimumNumberShouldMatch", 1, |q, a| {
        let m = a[0].as_float();
        mutate(q, |c: &mut CommonTermsQuery| {
            c.set_low_freq_minimum_number_should_match(m)
        });
        Ok(Value::Null)
    });
    r.call_fn(CTQ, "setHighFreqMinimumNumberShouldMatch", 1, |q, a| {
        let m = a[0].as_float();
        mutate(q, |c: &mut CommonTermsQuery| {
            c.set_high_freq_minimum_number_should_match(m)
        });
        Ok(Value::Null)
    });

    r.new_fn(MLT, 1, |a| {
        Ok(Value::object(
            MLT,
            MoreLikeThis::new(a[0].get::<Arc<dyn IndexReader>>()),
        ))
    });
    r.call_fn(MLT, "setAnalyzer", 1, |m, a| {
        let analyzer = a[0].get::<Arc<dyn Analyzer>>();
        m.with_mut(|m: &mut MoreLikeThis| m.set_analyzer(Some(analyzer)));
        Ok(Value::Null)
    });
    type IntSetter = fn(&mut MoreLikeThis, i32);
    let int_setters: [(&str, IntSetter); 7] = [
        ("setMinTermFreq", |m, v| m.set_min_term_freq(v)),
        ("setMinDocFreq", |m, v| m.set_min_doc_freq(v)),
        ("setMaxDocFreq", |m, v| m.set_max_doc_freq(v)),
        ("setMinWordLen", |m, v| m.set_min_word_len(v)),
        ("setMaxWordLen", |m, v| m.set_max_word_len(v)),
        ("setMaxQueryTerms", |m, v| m.set_max_query_terms(v)),
        ("setMaxNumTokensParsed", |m, v| {
            m.set_max_num_tokens_parsed(v)
        }),
    ];
    for (name, setter) in int_setters {
        r.call_fn(MLT, name, 1, move |m, a| {
            let v = a[0].as_int();
            m.with_mut(|m: &mut MoreLikeThis| setter(m, v));
            Ok(Value::Null)
        });
    }
    r.call_fn(MLT, "setBoost", 1, |m, a| {
        let v = a[0].as_bool();
        m.with_mut(|m: &mut MoreLikeThis| m.set_boost(v));
        Ok(Value::Null)
    });
    r.call_fn(MLT, "setBoostFactor", 1, |m, a| {
        let v = a[0].as_float();
        m.with_mut(|m: &mut MoreLikeThis| m.set_boost_factor(v));
        Ok(Value::Null)
    });
    r.call_fn(MLT, "setFieldNames", 1, |m, a| {
        let names = match &a[0] {
            Value::Null => None,
            other => Some(other.as_strings()),
        };
        m.with_mut(|m: &mut MoreLikeThis| m.set_field_names(names));
        Ok(Value::Null)
    });
    r.call_fn(MLT, "like", 1, |m, a| {
        let doc = a[0].as_int();
        Ok(Value::query(
            m.with_mut(|m: &mut MoreLikeThis| m.like(doc))?,
        ))
    });
    r.new_fn(READER, 1, |a| {
        Ok(Value::object(READER, a[0].as_str().to_string()))
    });
    r.call_fn(MLT, "like", 2, |m, a| {
        let field = a[0].as_str().to_string();
        let readers: Vec<Box<dyn CharReader>> = a[1]
            .as_values()
            .iter()
            .map(|r| Box::new(StringCharReader::new(r.get::<String>())) as Box<dyn CharReader>)
            .collect();
        Ok(Value::query(m.with_mut(|m: &mut MoreLikeThis| {
            m.like_readers(&field, readers)
        })?))
    });

    r.new_fn(MLTQ, 4, |a| {
        Ok(Value::query(Arc::new(MoreLikeThisQuery::new(
            a[0].as_str(),
            a[1].as_strings(),
            a[2].get::<Arc<dyn Analyzer>>(),
            a[3].as_str(),
        ))))
    });
    type QIntSetter = fn(&mut MoreLikeThisQuery, i32);
    let q_setters: [(&str, QIntSetter); 3] = [
        ("setMinTermFrequency", |q, v| q.set_min_term_frequency(v)),
        ("setMinDocFreq", |q, v| q.set_min_doc_freq(v)),
        ("setMaxQueryTerms", |q, v| q.set_max_query_terms(v)),
    ];
    for (name, setter) in q_setters {
        r.call_fn(MLTQ, name, 1, move |q, a| {
            let v = a[0].as_int();
            mutate(q, |c: &mut MoreLikeThisQuery| setter(c, v));
            Ok(Value::Null)
        });
    }
    r.call_fn(MLTQ, "setPercentTermsToMatch", 1, |q, a| {
        let v = a[0].as_float();
        mutate(q, |c: &mut MoreLikeThisQuery| {
            c.set_percent_terms_to_match(v)
        });
        Ok(Value::Null)
    });
    r
}

fn count_records(dump: &str, kind: &str) -> usize {
    dump.lines()
        .filter(|l| l.starts_with(&format!("{kind}\t")))
        .count()
}

/// Runs both sides and panics with the first difference; a tampered hit
/// must make the comparison fail (negative control).
fn assert_parity(oracle: &SearchOracle, min_hits: usize) {
    let java = oracle.java_dump();
    let rust = oracle.rust_dump();
    let hits = count_records(&java, "hit");
    println!(
        "{} queries, {} hits, {} dump lines",
        count_records(&java, "query"),
        hits,
        java.lines().count()
    );
    assert!(
        hits >= min_hits,
        "the dump has {hits} hits, too few to prove parity"
    );
    if let Err(diff) = compare(&java, &rust) {
        panic!("parity with Lucene 10.5.0 fails:\n{diff}");
    }
    let tampered = rust.replacen("\nhit\t", "\nhit\t-", 1);
    assert!(
        compare(&java, &tampered).is_err(),
        "the comparison does not see a changed hit"
    );
}

fn occur(name: &str) -> String {
    expr(
        "field",
        &["org.apache.lucene.search.BooleanClause$Occur", name],
    )
}

fn term(text: &str) -> String {
    expr(
        "new",
        &["org.apache.lucene.index.Term", &lit("body"), &lit(text)],
    )
}

/// Binds `$name` to a `CommonTermsQuery` over `terms` with the given
/// settings, and adds its query record.
#[allow(clippy::too_many_arguments)]
fn ctq(
    o: &mut SearchOracle,
    name: &str,
    high: &str,
    low: &str,
    max_freq: f32,
    terms: &[&str],
    low_msm: Option<f32>,
    high_msm: Option<f32>,
) {
    o.bind(
        name,
        &expr(
            "new",
            &[CTQ, &occur(high), &occur(low), &format!("f:{max_freq}")],
        ),
    );
    for t in terms {
        o.exec(&expr("call", &["add", &format!("${name}"), &term(t)]));
    }
    if let Some(m) = low_msm {
        o.exec(&expr(
            "call",
            &[
                "setLowFreqMinimumNumberShouldMatch",
                &format!("${name}"),
                &format!("f:{m}"),
            ],
        ));
    }
    if let Some(m) = high_msm {
        o.exec(&expr(
            "call",
            &[
                "setHighFreqMinimumNumberShouldMatch",
                &format!("${name}"),
                &format!("f:{m}"),
            ],
        ));
    }
    o.query(&format!("ctq-{name}"), &format!("${name}"));
}

#[test]
fn common_terms_query_parity() {
    let index = OracleIndex::standard(SEED, NUM_DOCS);
    let mut o = SearchOracle::new(&index, registry());
    let mixed = ["the", "of", "alpha", "zulu", "kilo", "and"];
    ctq(
        &mut o, "should", "SHOULD", "SHOULD", 0.3, &mixed, None, None,
    );
    ctq(
        &mut o, "low-must", "SHOULD", "MUST", 0.3, &mixed, None, None,
    );
    ctq(
        &mut o,
        "high-must",
        "MUST",
        "SHOULD",
        0.3,
        &mixed,
        None,
        None,
    );
    ctq(
        &mut o, "absolute", "SHOULD", "SHOULD", 10.0, &mixed, None, None,
    );
    ctq(
        &mut o,
        "low-msm",
        "SHOULD",
        "SHOULD",
        0.3,
        &mixed,
        Some(0.5),
        None,
    );
    ctq(
        &mut o,
        "low-msm-abs",
        "SHOULD",
        "SHOULD",
        0.3,
        &mixed,
        Some(2.0),
        None,
    );
    ctq(
        &mut o,
        "both-msm",
        "SHOULD",
        "SHOULD",
        0.3,
        &mixed,
        Some(1.0),
        Some(2.0),
    );
    ctq(
        &mut o,
        "only-high",
        "SHOULD",
        "SHOULD",
        0.1,
        &["the", "of", "and"],
        Some(1.0),
        Some(2.0),
    );
    ctq(
        &mut o,
        "only-high-must",
        "MUST",
        "SHOULD",
        0.1,
        &["the", "of", "and"],
        None,
        None,
    );
    ctq(
        &mut o,
        "only-low",
        "SHOULD",
        "MUST",
        0.9,
        &["zulu", "yankee", "xray"],
        None,
        None,
    );
    ctq(
        &mut o,
        "absent",
        "SHOULD",
        "SHOULD",
        0.3,
        &["nonexistent", "missing"],
        None,
        None,
    );
    ctq(
        &mut o,
        "single",
        "SHOULD",
        "SHOULD",
        0.3,
        &["alpha"],
        None,
        None,
    );
    ctq(&mut o, "empty", "SHOULD", "SHOULD", 0.3, &[], None, None);
    o.query(
        "ctq-must-not",
        &expr("new", &[CTQ, &occur("MUST_NOT"), &occur("SHOULD"), "f:0.3"]),
    );
    assert_parity(&o, 100);
}

fn mlt_call(o: &mut SearchOracle, name: &str, method: &str, arg: &str) {
    o.exec(&expr("call", &[method, &format!("${name}"), arg]));
}

/// Binds `$name` to a `MoreLikeThis` over the index with the standard
/// analyzer and the given minimum term and document frequencies.
fn mlt(o: &mut SearchOracle, name: &str, min_term_freq: i32, min_doc_freq: i32) {
    o.bind(name, &expr("new", &[MLT, "$reader"]));
    mlt_call(
        o,
        name,
        "setAnalyzer",
        &expr(
            "new",
            &["org.apache.lucene.analysis.standard.StandardAnalyzer"],
        ),
    );
    mlt_call(o, name, "setMinTermFreq", &format!("i:{min_term_freq}"));
    mlt_call(o, name, "setMinDocFreq", &format!("i:{min_doc_freq}"));
}

fn fields(names: &[&str]) -> String {
    let lits: Vec<String> = names.iter().map(|n| lit(n)).collect();
    let mut args: Vec<&str> = vec!["java.lang.String"];
    args.extend(lits.iter().map(String::as_str));
    expr("array", &args)
}

fn readers(texts: &[&str]) -> String {
    let rs: Vec<String> = texts
        .iter()
        .map(|t| expr("new", &[READER, &lit(t)]))
        .collect();
    let mut args: Vec<&str> = vec!["java.io.Reader"];
    args.extend(rs.iter().map(String::as_str));
    expr("array", &args)
}

#[test]
fn more_like_this_parity() {
    let index = OracleIndex::standard(SEED, NUM_DOCS);
    let mut o = SearchOracle::new(&index, registry());
    // the indexed fields of the index (`setFieldNames(null)`)
    mlt(&mut o, "all", 1, 1);
    for doc in [0, 3, 17, 42] {
        o.query(
            &format!("like-all-{doc}"),
            &expr("call", &["like", "$all", &format!("i:{doc}")]),
        );
    }
    mlt(&mut o, "body", 1, 2);
    mlt_call(&mut o, "body", "setFieldNames", &fields(&["body"]));
    for doc in [1, 5, 33] {
        o.query(
            &format!("like-body-{doc}"),
            &expr("call", &["like", "$body", &format!("i:{doc}")]),
        );
    }
    mlt(&mut o, "boost", 2, 1);
    mlt_call(
        &mut o,
        "boost",
        "setFieldNames",
        &fields(&["body", "title"]),
    );
    mlt_call(&mut o, "boost", "setBoost", "z:true");
    mlt_call(&mut o, "boost", "setBoostFactor", "f:2.5");
    mlt_call(&mut o, "boost", "setMaxQueryTerms", "i:4");
    mlt_call(&mut o, "boost", "setMinWordLen", "i:4");
    mlt_call(&mut o, "boost", "setMaxWordLen", "i:7");
    for doc in [2, 9, 51] {
        o.query(
            &format!("like-boost-{doc}"),
            &expr("call", &["like", "$boost", &format!("i:{doc}")]),
        );
    }
    mlt(&mut o, "text", 1, 1);
    mlt_call(&mut o, "text", "setFieldNames", &fields(&["body"]));
    mlt_call(&mut o, "text", "setMaxDocFreq", "i:30");
    o.query(
        "like-text",
        &expr(
            "call",
            &[
                "like",
                "$text",
                &lit("body"),
                &readers(&["the alpha bravo alpha charlie", "delta alpha the of zulu"]),
            ],
        ),
    );
    o.query(
        "like-text-absent-field",
        &expr(
            "call",
            &["like", "$text", &lit("nofield"), &readers(&["alpha bravo"])],
        ),
    );
    mlt(&mut o, "tokens", 1, 1);
    mlt_call(&mut o, "tokens", "setFieldNames", &fields(&["body"]));
    mlt_call(&mut o, "tokens", "setMaxNumTokensParsed", "i:3");
    o.query(
        "like-tokens",
        &expr(
            "call",
            &[
                "like",
                "$tokens",
                &lit("body"),
                &readers(&["kilo lima mike november oscar"]),
            ],
        ),
    );

    let standard = expr(
        "new",
        &["org.apache.lucene.analysis.standard.StandardAnalyzer"],
    );
    o.bind(
        "mltq",
        &expr(
            "new",
            &[
                MLTQ,
                &lit("alpha bravo charlie the of delta"),
                &fields(&["body", "title"]),
                &standard,
                &lit("body"),
            ],
        ),
    );
    o.query("mltq-default", "$mltq");
    o.bind(
        "mltq2",
        &expr(
            "new",
            &[
                MLTQ,
                &lit("alpha alpha bravo kilo lima"),
                &fields(&["body"]),
                &standard,
                &lit("body"),
            ],
        ),
    );
    o.exec(&expr("call", &["setMinDocFreq", "$mltq2", "i:1"]));
    o.exec(&expr("call", &["setMaxQueryTerms", "$mltq2", "i:3"]));
    o.exec(&expr(
        "call",
        &["setPercentTermsToMatch", "$mltq2", "f:0.5"],
    ));
    o.query("mltq-settings", "$mltq2");
    assert_parity(&o, 50);
}
