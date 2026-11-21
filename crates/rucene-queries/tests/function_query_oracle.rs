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

//! The Java-oracle portability test of `queries.function` and
//! `queries.function.docvalues` (rmp #478): `FunctionQuery`,
//! `FunctionRangeQuery` (number, string and open bounds, inclusive and
//! exclusive, inverted), `FunctionScoreQuery` (`new`, `boostByValue`,
//! `boostByQuery`, the score source) and `FunctionMatchQuery` (default and
//! explicit match cost) over doc-values sources and over every
//! `IndexReaderFunctions` source, over a seeded index Rucene writes
//! (`function_oracle::function_index`: three segments, deleted documents,
//! documents without a value).
//!
//! For each query Lucene 10.5.0 and Rucene must give the same `toString()`,
//! class tree before and after rewrite, count, hits with bit-exact `float`
//! scores and `explain` of every document; each `IndexReaderFunctions`
//! source is also dumped as `FunctionValues` (`values` records, through
//! `ValueSource.fromDoubleValuesSource`), which exercises the
//! `function.docvalues` classes.
//!
//! Sharded over several tests so nextest runs them in parallel; each writes
//! its own index and runs its own JVM in its own directory under `TMPDIR`.

mod function_oracle;
mod query_oracle;

use function_oracle::{
    assert_parity, between, cvs, field, frange, from_dvs, function_index, function_query,
    match_all, oracle, static_call, term, term_query, DVS,
};
use query_oracle::{expr, lit};

/// The seed of the index.
const SEED: i64 = 0x0478_5eed;

/// The number of documents of the index.
const NUM_DOCS: usize = 40;

const F: &str = "org.apache.lucene.queries.function";
const IRF: &str = "org.apache.lucene.queries.function.IndexReaderFunctions";

fn fsq(q: &str, source: &str) -> String {
    expr("new", &[&format!("{F}.FunctionScoreQuery"), q, source])
}

fn dvs_field(method: &str, f: &str) -> String {
    static_call(DVS, method, &[&lit(f)])
}

fn null_int() -> String {
    expr("cast", &["java.lang.Integer", "null"])
}

#[test]
fn function_and_range_queries() {
    let index = function_index(SEED, NUM_DOCS);
    let mut o = oracle(&index);
    let i = || field("IntFieldSource", "i");
    let f = || field("FloatFieldSource", "f");
    o.query("fq-int", &function_query(&i()));
    o.query("fq-const", &function_query(&cvs(5.0)));
    o.query(
        "fq-boost",
        &expr(
            "new",
            &[
                "org.apache.lucene.search.BoostQuery",
                &function_query(&i()),
                "f:2.5",
            ],
        ),
    );
    o.query("fr-int", &frange(&i(), "i:2", "i:40", true, false));
    o.query("fr-int-incl", &frange(&i(), "i:2", "i:40", false, true));
    o.query(
        "fr-open-lower",
        &frange(&i(), &null_int(), "i:10", true, true),
    );
    o.query(
        "fr-open-upper",
        &frange(&i(), "i:10", &null_int(), true, true),
    );
    o.query(
        "fr-open-both",
        &frange(&i(), &null_int(), &null_int(), false, false),
    );
    o.query(
        "fr-strings",
        &frange(&i(), &lit("-10"), &lit("20"), true, true),
    );
    o.query("fr-float", &frange(&f(), "f:-1.5", "f:2.25", false, true));
    o.query(
        "fr-long",
        &frange(
            &field("LongFieldSource", "l"),
            "l:0",
            "l:100000000",
            true,
            true,
        ),
    );
    o.query(
        "fr-double",
        &frange(
            &field("DoubleFieldSource", "d"),
            "d:-3.5",
            "d:7.25",
            true,
            false,
        ),
    );
    o.query("fr-inverted", &frange(&i(), "i:50", "i:-50", true, true));
    o.query(
        "fr-mv",
        &frange(
            &function_oracle::mv("MultiValuedIntFieldSource", "imv", "MAX"),
            "i:0",
            "i:500",
            true,
            true,
        ),
    );
    o.query(
        "fr-bogus",
        &frange(&field("IntFieldSource", "bogus"), "i:0", "i:0", true, true),
    );
    o.bind(
        "bq",
        &expr("new", &["org.apache.lucene.search.BooleanQuery$Builder"]),
    );
    o.exec(&expr(
        "call",
        &[
            "add",
            "$bq",
            &frange(&i(), "i:0", "i:20", true, true),
            &expr(
                "field",
                &["org.apache.lucene.search.BooleanClause$Occur", "SHOULD"],
            ),
        ],
    ));
    o.exec(&expr(
        "call",
        &[
            "add",
            "$bq",
            &frange(&i(), "i:60", "i:100", true, true),
            &expr(
                "field",
                &["org.apache.lucene.search.BooleanClause$Occur", "SHOULD"],
            ),
        ],
    ));
    o.query("fr-two", &expr("call", &["build", "$bq"]));
    assert_parity(&o, 0);
}

#[test]
fn function_score_and_match_queries() {
    let index = function_index(SEED, NUM_DOCS);
    let mut o = oracle(&index);
    let the = || term_query("body", "the");
    o.query("fsq-int", &fsq(&the(), &dvs_field("fromIntField", "i")));
    o.query(
        "fsq-const",
        &fsq(&match_all(), &static_call(DVS, "constant", &["d:5"])),
    );
    o.query("fsq-scores", &fsq(&the(), &expr("field", &[DVS, "SCORES"])));
    o.query(
        "fsq-double",
        &fsq(&match_all(), &dvs_field("fromDoubleField", "d")),
    );
    o.query(
        "fsq-long",
        &fsq(&match_all(), &dvs_field("fromLongField", "l")),
    );
    o.query(
        "fsq-value-source",
        &fsq(
            &the(),
            &expr(
                "call",
                &["asDoubleValuesSource", &field("IntFieldSource", "i")],
            ),
        ),
    );
    o.query(
        "fsq-boost-by-value",
        &static_call(
            &format!("{F}.FunctionScoreQuery"),
            "boostByValue",
            &[&the(), &dvs_field("fromFloatField", "f")],
        ),
    );
    o.query(
        "fsq-boost-by-query",
        &static_call(
            &format!("{F}.FunctionScoreQuery"),
            "boostByQuery",
            &[&the(), &term_query("body", "alpha"), "f:3.5"],
        ),
    );
    o.query(
        "fsq-boost-by-query-scores",
        &static_call(
            &format!("{F}.FunctionScoreQuery"),
            "boostByValue",
            &[
                &static_call(
                    &format!("{F}.FunctionScoreQuery"),
                    "boostByQuery",
                    &[&the(), &term_query("body", "of"), "f:2"],
                ),
                &expr("field", &[DVS, "SCORES"]),
            ],
        ),
    );
    o.query(
        "fmq-int",
        &expr(
            "new",
            &[
                &format!("{F}.FunctionMatchQuery"),
                &dvs_field("fromIntField", "i"),
                &between(0.0, 50.0),
            ],
        ),
    );
    o.query(
        "fmq-double-cost",
        &expr(
            "new",
            &[
                &format!("{F}.FunctionMatchQuery"),
                &dvs_field("fromDoubleField", "d"),
                &between(-5.0, 5.0),
                "f:50",
            ],
        ),
    );
    o.query(
        "fmq-none",
        &expr(
            "new",
            &[
                &format!("{F}.FunctionMatchQuery"),
                &dvs_field("fromIntField", "bogus"),
                &between(0.0, 1.0),
            ],
        ),
    );
    assert_parity(&o, 0);
}

#[test]
fn function_index_reader_functions() {
    let index = function_index(SEED, NUM_DOCS);
    let mut o = oracle(&index);
    let irf = |method: &str, args: &[&str]| static_call(IRF, method, args);
    let sources = vec![
        ("doc-freq", irf("docFreq", &[&term("body", "the")])),
        (
            "doc-freq-absent",
            irf("docFreq", &[&term("body", "nonexistent")]),
        ),
        ("max-doc", irf("maxDoc", &[])),
        ("num-docs", irf("numDocs", &[])),
        ("num-deleted-docs", irf("numDeletedDocs", &[])),
        ("term-freq", irf("termFreq", &[&term("body", "the")])),
        ("term-freq-docs-only", irf("termFreq", &[&term("id", "d3")])),
        (
            "term-freq-bogus",
            irf("termFreq", &[&term("bogus", "bogus")]),
        ),
        (
            "total-term-freq",
            irf("totalTermFreq", &[&term("body", "of")]),
        ),
        ("sum-doc-freq", irf("sumDocFreq", &[&lit("body")])),
        ("doc-count", irf("docCount", &[&lit("body")])),
        (
            "sum-total-term-freq",
            expr(
                "call",
                &[
                    "toDoubleValuesSource",
                    &irf("sumTotalTermFreq", &[&lit("body")]),
                ],
            ),
        ),
    ];
    for (id, source) in &sources {
        o.query(&format!("irf-{id}"), &fsq(&match_all(), source));
        o.custom("values", &format!("vals-{id}"), &from_dvs(source), "-");
    }
    o.query(
        "irf-match",
        &expr(
            "new",
            &[
                &format!("{F}.FunctionMatchQuery"),
                &irf("termFreq", &[&term("body", "the")]),
                &between(2.0, 100.0),
            ],
        ),
    );
    assert_parity(&o, 3000);
}
