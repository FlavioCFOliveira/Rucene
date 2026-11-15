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

//! The Java-oracle portability test of `FuzzyLikeThisQuery` (rmp #483):
//! the rewritten query and its scores for single and multiple words, several
//! fields, every `minSimilarity` (0, 1, 2) and prefix length, a small
//! `maxNumTerms`, `ignoreTF`, an absent field, a `null` query string, a
//! `null` field, a `null` analyzer (with and without terms to analyze), no
//! terms at all and an invalid `minSimilarity`, over a seeded index Rucene
//! writes (`OracleIndex::standard`: three segments, deleted documents).
//!
//! For each query Lucene 10.5.0 and Rucene must give the same `toString()`
//! (`null`, printed as `null`), class tree, rewritten query and its class
//! tree, count, hits with bit-exact `float` scores and `explain` of every
//! document, or the same exception class and message.

#[path = "../../rucene-queries/tests/query_oracle/mod.rs"]
mod query_oracle;

use std::sync::Arc;

use query_oracle::{compare, expr, lit, OracleIndex, Registry, SearchOracle, Value};
use rucene::analysis::Analyzer;
use rucene::search::query::Query;
use rucene_sandbox::queries::fuzzy_like_this_query::FuzzyLikeThisQuery;

/// The seed of the index.
const SEED: i64 = 0x0483_5eed;

/// The number of documents of the index.
const NUM_DOCS: usize = 60;

const FLT: &str = "org.apache.lucene.sandbox.queries.FuzzyLikeThisQuery";

/// Replaces the query a value holds by `f` of a clone of it, as a setter
/// changes the Java object in place.
fn mutate(
    v: &Value,
    f: impl FnOnce(&mut FuzzyLikeThisQuery) -> rucene::error::Result<()>,
) -> rucene::error::Result<()> {
    v.with_mut(|q: &mut Arc<dyn Query>| {
        let mut c = q
            .as_any()
            .downcast_ref::<FuzzyLikeThisQuery>()
            .expect("a FuzzyLikeThisQuery")
            .clone();
        f(&mut c)?;
        *q = Arc::new(c);
        Ok(())
    })
}

fn registry() -> Registry {
    let mut r = Registry::core();
    r.new_fn(FLT, 2, |a| {
        let analyzer = match &a[1] {
            Value::Null => None,
            other => Some(other.get::<Arc<dyn Analyzer>>()),
        };
        Ok(Value::query(Arc::new(FuzzyLikeThisQuery::new(
            a[0].as_int(),
            analyzer,
        ))))
    });
    r.call_fn(FLT, "addTerms", 4, |q, a| {
        let query_string = a[0].as_opt_str().map(str::to_string);
        let field = a[1].as_opt_str().map(str::to_string);
        let (min_similarity, prefix_length) = (a[2].as_float(), a[3].as_int());
        mutate(q, |c| {
            c.add_terms(query_string, field, min_similarity, prefix_length)
        })?;
        Ok(Value::Null)
    });
    r.call_fn(FLT, "setIgnoreTF", 1, |q, a| {
        let v = a[0].as_bool();
        mutate(q, |c| {
            c.set_ignore_tf(v);
            Ok(())
        })?;
        Ok(Value::Null)
    });
    r
}

fn analyzer() -> String {
    expr(
        "new",
        &["org.apache.lucene.analysis.standard.StandardAnalyzer"],
    )
}

fn opt_lit(s: Option<&str>) -> String {
    s.map_or_else(|| "null".to_string(), lit)
}

/// Binds `$name` to a `FuzzyLikeThisQuery` with `additions` (query string,
/// field, minSimilarity, prefix length), and adds its query record.
fn flt(
    o: &mut SearchOracle,
    name: &str,
    max_num_terms: i32,
    analyzer: &str,
    additions: &[(Option<&str>, Option<&str>, f32, i32)],
    ignore_tf: bool,
) {
    o.bind(
        name,
        &expr("new", &[FLT, &format!("i:{max_num_terms}"), analyzer]),
    );
    for (text, field, min_similarity, prefix_length) in additions {
        o.exec(&expr(
            "call",
            &[
                "addTerms",
                &format!("${name}"),
                &opt_lit(*text),
                &opt_lit(*field),
                &format!("f:{min_similarity}"),
                &format!("i:{prefix_length}"),
            ],
        ));
    }
    if ignore_tf {
        o.exec(&expr(
            "call",
            &["setIgnoreTF", &format!("${name}"), "z:true"],
        ));
    }
    o.query(&format!("flt-{name}"), &format!("${name}"));
}

fn count_records(dump: &str, kind: &str) -> usize {
    dump.lines()
        .filter(|l| l.starts_with(&format!("{kind}\t")))
        .count()
}

#[test]
fn fuzzy_like_this_query_parity() {
    let index = OracleIndex::standard(SEED, NUM_DOCS);
    let mut o = SearchOracle::new(&index, registry());
    let a = analyzer();
    flt(
        &mut o,
        "one-word",
        10,
        &a,
        &[(Some("alpah"), Some("body"), 2.0, 1)],
        false,
    );
    flt(
        &mut o,
        "two-words",
        10,
        &a,
        &[(Some("alpah bravp"), Some("body"), 2.0, 1)],
        false,
    );
    flt(
        &mut o,
        "exact",
        10,
        &a,
        &[(Some("charlie delta"), Some("body"), 0.0, 0)],
        false,
    );
    flt(
        &mut o,
        "edits-1",
        10,
        &a,
        &[(Some("echp foxtrit"), Some("body"), 1.0, 0)],
        false,
    );
    flt(
        &mut o,
        "prefix",
        10,
        &a,
        &[(Some("gplf hotel"), Some("body"), 2.0, 2)],
        false,
    );
    flt(
        &mut o,
        "max-terms",
        2,
        &a,
        &[(Some("india juliet kilo lima mike"), Some("body"), 2.0, 0)],
        false,
    );
    flt(
        &mut o,
        "two-fields",
        10,
        &a,
        &[
            (Some("alpha"), Some("body"), 1.0, 0),
            (Some("brovo"), Some("title"), 2.0, 0),
        ],
        false,
    );
    flt(
        &mut o,
        "ignore-tf",
        10,
        &a,
        &[(Some("the of andd"), Some("body"), 1.0, 0)],
        true,
    );
    flt(
        &mut o,
        "absent-field",
        10,
        &a,
        &[(Some("alpha"), Some("nofield"), 2.0, 1)],
        false,
    );
    flt(
        &mut o,
        "absent-and-present",
        10,
        &a,
        &[
            (Some("alpha"), Some("nofield"), 2.0, 1),
            (Some("alpah"), Some("body"), 2.0, 1),
        ],
        false,
    );
    flt(
        &mut o,
        "absent-term",
        10,
        &a,
        &[(Some("qqqqqqq"), Some("body"), 2.0, 0)],
        false,
    );
    flt(
        &mut o,
        "null-query-string",
        10,
        &a,
        &[(None, Some("body"), 1.0, 0)],
        false,
    );
    flt(
        &mut o,
        "null-field",
        10,
        &a,
        &[(Some("alpha"), None, 1.0, 0)],
        false,
    );
    flt(
        &mut o,
        "null-analyzer",
        10,
        "null",
        &[(Some("alpha"), Some("body"), 1.0, 0)],
        false,
    );
    flt(
        &mut o,
        "null-analyzer-null-field",
        10,
        "null",
        &[(Some("alpha"), None, 1.0, 0)],
        false,
    );
    flt(
        &mut o,
        "null-analyzer-absent-field",
        10,
        "null",
        &[(Some("alpha"), Some("nofield"), 1.0, 0)],
        false,
    );
    flt(&mut o, "empty", 10, &a, &[], false);
    o.bind("bad", &expr("new", &[FLT, "i:10", &a]));
    o.query(
        "flt-bad-similarity",
        &expr(
            "call",
            &[
                "addTerms",
                "$bad",
                &lit("alpha"),
                &lit("body"),
                "f:1.5",
                "i:0",
            ],
        ),
    );
    o.query(
        "flt-boost",
        &expr(
            "new",
            &["org.apache.lucene.search.BoostQuery", "$two-words", "f:3"],
        ),
    );

    let java = o.java_dump();
    let rust = o.rust_dump();
    let hits = count_records(&java, "hit");
    println!(
        "{} queries, {} hits, {} errors, {} dump lines",
        count_records(&java, "query"),
        hits,
        count_records(&java, "error"),
        java.lines().count()
    );
    assert!(
        hits >= 50,
        "the dump has {hits} hits, too few to prove parity"
    );
    if let Err(diff) = compare(&java, &rust) {
        panic!("FuzzyLikeThisQuery parity with Lucene 10.5.0 fails:\n{diff}");
    }
    // negative control: a changed hit must make the comparison fail
    let tampered = rust.replacen("\nhit\t", "\nhit\t-", 1);
    assert!(
        compare(&java, &tampered).is_err(),
        "the comparison does not see a changed hit"
    );
}
