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

//! The Java-oracle portability test of `queries.function.valuesource` (rmp
//! #479): a `FunctionQuery` over every concrete value source of the
//! package (field sources of every doc-values type, single- and
//! multi-valued, `EnumFieldSource`, constants, the float functions, `If`,
//! `Def`, `Vector`, `QueryValueSource`, the term-statistics sources under
//! `ClassicSimilarity` and under the default `BM25Similarity`, and the
//! vector sources and similarity functions), over a seeded index Rucene
//! writes (`function_oracle::function_index`: three segments, deleted
//! documents, documents without a value).
//!
//! For each source Lucene 10.5.0 and Rucene must give the same query
//! `toString()`, class tree, count, hits with bit-exact `float` scores and
//! `explain` of every document, and the same `FunctionValues` of every
//! document (`values` records: every getter's value, floats as bits, or the
//! exception it throws). The abstract classes of the package
//! (`BoolFunction`, `ComparisonBoolFunction`, `DualFloatFunction`,
//! `FieldCacheSource`, `MultiBoolFunction`, `MultiFloatFunction`,
//! `MultiFunction`, `MultiValueSource`, `SimpleBoolFunction`,
//! `SimpleFloatFunction`, `SingleFunction`, `ConstNumberSource`,
//! `VectorSimilarityFunction`, `VectorFieldFunction`) are reached through
//! their concrete subclasses; `ComparisonBoolFunction` has none in Lucene.
//!
//! Sharded over several tests so nextest runs them in parallel; each writes
//! its own index and runs its own JVM in its own directory under `TMPDIR`.

mod function_oracle;
mod query_oracle;

use function_oracle::{
    add_sources, assert_parity, cvs, enum_source, field, function_index, mv, oracle, similarity,
    stat, term_query, vs, vs_array, vs_list,
};
use query_oracle::{expr, lit};

/// The seed of the index.
const SEED: i64 = 0x0479_5eed;

/// The number of documents of the index.
const NUM_DOCS: usize = 40;

#[test]
fn valuesource_field_sources() {
    let index = function_index(SEED, NUM_DOCS);
    let mut o = oracle(&index);
    let sources = vec![
        ("int", field("IntFieldSource", "i")),
        ("long", field("LongFieldSource", "l")),
        ("float", field("FloatFieldSource", "f")),
        ("double", field("DoubleFieldSource", "d")),
        ("int-bogus", field("IntFieldSource", "bogus")),
        ("double-bogus", field("DoubleFieldSource", "bogus")),
        ("int-mv-min", mv("MultiValuedIntFieldSource", "imv", "MIN")),
        ("int-mv-max", mv("MultiValuedIntFieldSource", "imv", "MAX")),
        (
            "long-mv-min",
            mv("MultiValuedLongFieldSource", "lmv", "MIN"),
        ),
        (
            "long-mv-max",
            mv("MultiValuedLongFieldSource", "lmv", "MAX"),
        ),
        (
            "float-mv-min",
            mv("MultiValuedFloatFieldSource", "fmv", "MIN"),
        ),
        (
            "float-mv-max",
            mv("MultiValuedFloatFieldSource", "fmv", "MAX"),
        ),
        (
            "double-mv-min",
            mv("MultiValuedDoubleFieldSource", "dmv", "MIN"),
        ),
        (
            "double-mv-max",
            mv("MultiValuedDoubleFieldSource", "dmv", "MAX"),
        ),
        ("bytes-ref", field("BytesRefFieldSource", "s")),
        ("bytes-ref-bogus", field("BytesRefFieldSource", "bogus")),
        ("sorted-set", field("SortedSetFieldSource", "ss")),
        ("enum", enum_source("e")),
        ("literal", field("LiteralValueSource", "a literal")),
        ("const", cvs(0.3)),
        ("double-const", vs("DoubleConstValueSource", &["d:0.3"])),
    ];
    add_sources(&mut o, &sources);
    assert_parity(&o, 5000);
}

#[test]
fn valuesource_functions() {
    let index = function_index(SEED, NUM_DOCS);
    let mut o = oracle(&index);
    let i = || field("IntFieldSource", "i");
    let f = || field("FloatFieldSource", "f");
    let bogus = || field("FloatFieldSource", "bogus");
    let sources = vec![
        ("div", vs("DivFloatFunction", &[&i(), &cvs(3.0)])),
        ("div-bogus", vs("DivFloatFunction", &[&bogus(), &cvs(3.0)])),
        ("pow", vs("PowFloatFunction", &[&f(), &cvs(2.0)])),
        ("linear", vs("LinearFloatFunction", &[&i(), "f:2", "f:1"])),
        (
            "reciprocal",
            vs("ReciprocalFloatFunction", &[&i(), "f:1", "f:2", "f:3"]),
        ),
        ("scale", vs("ScaleFloatFunction", &[&i(), "f:0", "f:1"])),
        (
            "scale-bogus",
            vs("ScaleFloatFunction", &[&bogus(), "f:0", "f:1"]),
        ),
        (
            "range-map",
            vs(
                "RangeMapFloatFunction",
                &[&f(), "f:-1", "f:1", "f:5", "f:0"],
            ),
        ),
        (
            "range-map-null",
            vs(
                "RangeMapFloatFunction",
                &[&f(), "f:-1", "f:1", "f:5", "null"],
            ),
        ),
        (
            "range-map-sources",
            vs(
                "RangeMapFloatFunction",
                &[
                    &i(),
                    "f:0",
                    "f:50",
                    &vs("SumFloatFunction", &[&vs_array(&[cvs(1.0), cvs(2.0)])]),
                    &cvs(11.0),
                ],
            ),
        ),
        (
            "max",
            vs("MaxFloatFunction", &[&vs_array(&[i(), f(), bogus()])]),
        ),
        (
            "min",
            vs("MinFloatFunction", &[&vs_array(&[i(), f(), bogus()])]),
        ),
        ("sum", vs("SumFloatFunction", &[&vs_array(&[i(), f()])])),
        (
            "sum-bogus",
            vs("SumFloatFunction", &[&vs_array(&[bogus(), cvs(42.0)])]),
        ),
        (
            "product",
            vs("ProductFloatFunction", &[&vs_array(&[i(), f()])]),
        ),
        (
            "def",
            vs("DefFunction", &[&vs_list(&[bogus(), i(), cvs(-1.0)])]),
        ),
        (
            "if",
            vs(
                "IfFunction",
                &[&field("BytesRefFieldSource", "s"), &cvs(1.0), &cvs(2.0)],
            ),
        ),
        (
            "if-bogus",
            vs("IfFunction", &[&bogus(), &cvs(1.0), &cvs(2.0)]),
        ),
        (
            "if-literal",
            vs(
                "IfFunction",
                &[&field("LiteralValueSource", "false"), &i(), &f()],
            ),
        ),
        ("vector", vs("VectorValueSource", &[&vs_list(&[i(), f()])])),
        (
            "query",
            vs("QueryValueSource", &[&term_query("body", "the"), "f:7"]),
        ),
        (
            "query-none",
            vs(
                "QueryValueSource",
                &[&term_query("body", "nonexistent"), "f:5"],
            ),
        ),
        (
            "query-function",
            vs(
                "QueryValueSource",
                &[&function_oracle::function_query(&i()), "f:0"],
            ),
        ),
    ];
    add_sources(&mut o, &sources);
    assert_parity(&o, 5000);
}

/// The term-statistics sources over `body`.
fn statistics_sources() -> Vec<(&'static str, String)> {
    vec![
        ("doc-freq", stat("DocFreqValueSource", "body", "the")),
        (
            "doc-freq-absent",
            stat("DocFreqValueSource", "body", "nonexistent"),
        ),
        ("idf", stat("IDFValueSource", "body", "alpha")),
        ("idf-absent", stat("IDFValueSource", "body", "nonexistent")),
        ("tf", stat("TFValueSource", "body", "the")),
        ("tf-bogus", stat("TFValueSource", "bogus", "bogus")),
        ("term-freq", stat("TermFreqValueSource", "body", "of")),
        (
            "term-freq-bogus",
            stat("TermFreqValueSource", "bogus", "bogus"),
        ),
        (
            "total-term-freq",
            stat("TotalTermFreqValueSource", "body", "the"),
        ),
        (
            "sum-total-term-freq",
            field("SumTotalTermFreqValueSource", "body"),
        ),
        ("norm", field("NormValueSource", "body")),
        ("norm-bogus", field("NormValueSource", "bogus")),
        ("max-doc", vs("MaxDocValueSource", &[])),
        ("num-docs", vs("NumDocsValueSource", &[])),
        (
            "join-doc-freq",
            vs("JoinDocFreqValueSource", &[&lit("s"), &lit("body")]),
        ),
    ]
}

#[test]
fn valuesource_term_statistics_classic() {
    let index = function_index(SEED, NUM_DOCS);
    let mut o = oracle(&index);
    o.similarity(&expr(
        "new",
        &["org.apache.lucene.search.similarities.ClassicSimilarity"],
    ));
    add_sources(&mut o, &statistics_sources());
    assert_parity(&o, 5000);
}

#[test]
fn valuesource_term_statistics_bm25() {
    let index = function_index(SEED, NUM_DOCS);
    let mut o = oracle(&index);
    add_sources(&mut o, &statistics_sources());
    assert_parity(&o, 1000);
}

#[test]
fn valuesource_vectors() {
    let index = function_index(SEED, NUM_DOCS);
    let mut o = oracle(&index);
    let fv = || field("FloatKnnVectorFieldSource", "fv");
    let bv = || field("ByteKnnVectorFieldSource", "bv");
    let cf = |v: &[&str]| {
        let mut args: Vec<&str> = vec!["float"];
        args.extend_from_slice(v);
        vs("ConstKnnFloatValueSource", &[&expr("array", &args)])
    };
    let cb = |v: &[&str]| {
        let mut args: Vec<&str> = vec!["byte"];
        args.extend_from_slice(v);
        vs("ConstKnnByteVectorValueSource", &[&expr("array", &args)])
    };
    let fsim = |s: &str, a: String, b: String| {
        vs("FloatVectorSimilarityFunction", &[&similarity(s), &a, &b])
    };
    let bsim = |s: &str, a: String, b: String| {
        vs("ByteVectorSimilarityFunction", &[&similarity(s), &a, &b])
    };
    let consts = || cf(&["f:0.1", "f:-0.2", "f:0.3"]);
    let constb = || cb(&["b:1", "b:-2", "b:3"]);
    let sources = vec![
        ("float-field", fv()),
        ("byte-field", bv()),
        ("float-const", consts()),
        ("byte-const", constb()),
        ("float-euclidean", fsim("EUCLIDEAN", fv(), consts())),
        ("float-dot-product", fsim("DOT_PRODUCT", consts(), fv())),
        ("float-cosine", fsim("COSINE", fv(), consts())),
        ("float-mip", fsim("MAXIMUM_INNER_PRODUCT", fv(), fv())),
        ("byte-euclidean", bsim("EUCLIDEAN", bv(), constb())),
        ("byte-dot-product", bsim("DOT_PRODUCT", constb(), bv())),
        ("byte-cosine", bsim("COSINE", bv(), constb())),
        ("byte-mip", bsim("MAXIMUM_INNER_PRODUCT", bv(), bv())),
        ("float-dims", fsim("EUCLIDEAN", cf(&["f:1", "f:2"]), fv())),
        (
            "byte-dims",
            bsim("EUCLIDEAN", cb(&["b:1", "b:2", "b:3", "b:4"]), bv()),
        ),
        ("float-on-byte", fsim("EUCLIDEAN", constb(), bv())),
        ("byte-on-float", bsim("EUCLIDEAN", constb(), fv())),
        (
            "byte-wrong-field",
            bsim("EUCLIDEAN", bv(), field("ByteKnnVectorFieldSource", "fv")),
        ),
        (
            "float-wrong-field",
            fsim("EUCLIDEAN", field("FloatKnnVectorFieldSource", "bv"), fv()),
        ),
        (
            "float-no-vectors",
            fsim("EUCLIDEAN", field("FloatKnnVectorFieldSource", "i"), fv()),
        ),
    ];
    add_sources(&mut o, &sources);
    assert_parity(&o, 3000);
}
