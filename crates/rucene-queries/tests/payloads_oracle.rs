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

//! The Java-oracle portability test of `queries.payloads` (rmp #481):
//! `PayloadScoreQuery` with each `PayloadFunction` (`Average`, `Max`, `Min`,
//! `Sum`), with and without the span score, over span queries of each kind,
//! and `SpanPayloadCheckQuery` with each payload type (`STRING`, `INT`,
//! `FLOAT`) and match operation (`EQ`, `GT`, `GTE`, `LT`, `LTE`), over a
//! seeded index Rucene writes whose fields carry one-byte factors, big-endian
//! `int` and `float` payloads (negative values, `NaN`) and string payloads,
//! with positions that have no payload, three segments and deleted
//! documents.
//!
//! For each query Lucene 10.5.0 and Rucene must give the same `toString()`
//! and class tree before and after rewrite, count, hits with bit-exact
//! `float` scores, `explain` of every document, and the same spans with the
//! payloads `collect` reports; edge cases (more payloads than terms, `null`
//! payloads to match, an empty list, fields without payloads) included.
//! Sharded over several tests, each with its own index and JVM.

mod query_oracle;
mod span_oracle;

use std::sync::Arc;

use query_oracle::{compare, expr, OracleIndex, SearchOracle};
use randomizedtesting::jdk::random::{JdkRandom, Random};
use rucene::analysis::standard::new_standard_tokenizer;
use rucene::analysis::{
    Analyzer, AnalyzerState, PerFieldReuseStrategy, TokenFilter, TokenFilterLogic, TokenStream,
    TokenStreamComponents,
};
use rucene::document::{Document, Store, StringField, TextField};
use rucene::error::Result;
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::attribute::AttributeSource;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::attributes::{char_term_id, payload_id, Att};
use rucene_test_framework::analysis::{components, shared, sink};
use span_oracle::{
    payload_check, payload_score, perturb, registry, scontaining, sfirst, snear, snot, sor,
    spans_dumper, st, Payload,
};

/// The seed of the index.
const SEED: i64 = 0x0481_5eed;

/// The words of the payload documents, most frequent first.
const WORDS: [&str; 8] = [
    "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel",
];

/// The fields whose tokens carry payloads, and `body` without any.
const FIELDS: [&str; 5] = ["pf", "pi", "pfl", "ps", "body"];

/// The payload of the token `term` at `pos` in `field`, `None` for no
/// payload:
///
/// * `pf`: one byte, `-5` to `5` (the factor `FLOAT_DECODER` reads);
/// * `pi`: a big-endian `int`, negative values included;
/// * `pfl`: a big-endian `float`, negative values and `NaN` included;
/// * `ps`: the string `pos: <pos>` or `t<length>`.
fn payload_of(field: &str, term: &str, pos: i32) -> Option<Vec<u8>> {
    let len = term.len() as i32;
    match field {
        "pf" if pos % 5 == 4 => None,
        "pf" => Some(vec![(((len * 7 + pos * 3) % 11) - 5) as i8 as u8]),
        "pi" if pos % 7 == 6 => None,
        "pi" => Some((pos * 37 - 100 + len * 3).to_be_bytes().to_vec()),
        "pfl" if pos % 9 == 8 => Some(f32::NAN.to_be_bytes().to_vec()),
        "pfl" if pos % 11 == 10 => None,
        "pfl" => Some(
            ((pos as f32 - 3.5) * 1.25 + len as f32 * 0.5)
                .to_be_bytes()
                .to_vec(),
        ),
        "ps" if pos % 8 == 7 => None,
        "ps" if pos % 6 == 5 => Some(format!("t{len}").into_bytes()),
        "ps" => Some(format!("pos: {pos}").into_bytes()),
        _ => None,
    }
}

/// The analyzer of the payload index: `StandardTokenizer` tokens (the text is
/// lower-case words separated by spaces, so each word is one token) with the
/// payload [`payload_of`] gives. `MockTokenizer` needs a randomized-runner
/// context, which an oracle test does not run in.
#[derive(Debug)]
struct OraclePayloadAnalyzer {
    state: AnalyzerState,
}

impl Analyzer for OraclePayloadAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.rucene.tests.OraclePayloadAnalyzer"
    }

    fn create_components(&self, field_name: Option<&str>) -> Result<TokenStreamComponents> {
        let tokenizer = Arc::new(AtomicRefCell::new(new_standard_tokenizer()));
        let mut input = shared(&tokenizer);
        let source = input.attribute_source_mut();
        let term_att = Att::add(source, char_term_id())?;
        let payload_att = Att::add(source, payload_id())?;
        let filter = TokenFilter::new(
            input,
            PayloadRule {
                // Java switches on the field name, which a `null` one
                // dereferences; the oracle analyses named fields only.
                field: field_name
                    .unwrap_or_else(|| panic!("java.lang.NullPointerException"))
                    .to_string(),
                pos: -1,
                term_att,
                payload_att,
            },
        );
        Ok(components(tokenizer, sink(filter)))
    }
}

/// The filter of [`OraclePayloadAnalyzer`].
#[derive(Debug)]
struct PayloadRule {
    field: String,
    pos: i32,
    term_att: Att,
    payload_att: Att,
}

impl TokenFilterLogic for PayloadRule {
    fn increment_token(
        &mut self,
        _source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if !input.increment_token()? {
            return Ok(false);
        }
        self.pos += 1;
        let term = self.term_att.term_string();
        let payload = payload_of(&self.field, &term, self.pos).map(BytesRef::new);
        self.payload_att.payload_mut().set_payload(payload);
        Ok(true)
    }

    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.pos = -1;
        Ok(())
    }
}

/// A word of [`WORDS`], low ranks more frequent.
fn word(random: &mut JdkRandom) -> &'static str {
    let r = random.next_double();
    WORDS[((r * r) * WORDS.len() as f64) as usize]
}

/// The payload index: 40 documents of 1 to 25 words, the same text in every
/// field of [`FIELDS`], an `id` field, three segments, about one in twenty
/// deleted.
fn payload_index() -> OracleIndex {
    let mut random = JdkRandom::new(SEED);
    let num_docs: i32 = 40;
    let docs: Vec<Document> = (0..num_docs)
        .map(|i| {
            let mut doc = Document::new();
            doc.add(StringField::new("id", format!("d{i}"), Store::YES).expect("id"));
            let n = 1 + random.next_int_bound(25);
            let text: Vec<&str> = (0..n).map(|_| word(&mut random)).collect();
            let text = text.join(" ");
            for field in FIELDS {
                doc.add(TextField::new(field, text.clone(), Store::NO).expect("a text field"));
            }
            doc
        })
        .collect();
    let deletes: Vec<usize> = (0..num_docs / 20)
        .map(|_| random.next_int_bound(num_docs) as usize)
        .collect();
    OracleIndex::write_with_analyzer(
        &docs,
        &deletes,
        Arc::new(OraclePayloadAnalyzer {
            state: AnalyzerState::new(Box::new(PerFieldReuseStrategy)),
        }),
    )
}

fn oracle(index: &OracleIndex) -> SearchOracle {
    let mut oracle = SearchOracle::new(index, registry());
    oracle.spans_dumper(spans_dumper());
    oracle
}

/// A query record and a `PAYLOADS` spans record for each query.
fn add_all(oracle: &mut SearchOracle, queries: &[(String, String)]) {
    for (id, e) in queries {
        oracle.query(id, e);
        oracle.spans(id, e, "PAYLOADS");
    }
}

fn count_records(dump: &str, kind: &str) -> usize {
    dump.lines()
        .filter(|l| l.starts_with(&format!("{kind}\t")))
        .count()
}

/// Runs both sides and panics with the first difference; `min_hits` and
/// `min_payloads` guard against a vacuous comparison.
fn assert_parity(oracle: &SearchOracle, min_hits: usize, min_payloads: usize) -> (String, String) {
    let java = oracle.java_dump();
    let rust = oracle.rust_dump();
    let hits = count_records(&java, "hit");
    let payloads = java
        .lines()
        .filter(|l| l.starts_with("coll\t") && !l.ends_with("\tnull"))
        .count();
    println!(
        "{} queries, {hits} hits, {} spans, {payloads} collected payloads, {} dump lines",
        count_records(&java, "query"),
        count_records(&java, "span"),
        java.lines().count()
    );
    assert!(
        hits >= min_hits && payloads >= min_payloads,
        "the dump is too small to prove parity ({hits} hits, {payloads} payloads)"
    );
    if let Err(diff) = compare(&java, &rust) {
        panic!("payload parity with Lucene 10.5.0 fails:\n{diff}");
    }
    (java, rust)
}

/// The span queries `PayloadScoreQuery` wraps, over `field`: each kind.
fn score_shapes(field: &str) -> Vec<(String, String)> {
    let t = |w: &str| st(field, w);
    vec![
        ("term".into(), t("alpha")),
        ("or".into(), sor(&[t("alpha"), t("bravo")])),
        ("near-o".into(), snear(&[t("alpha"), t("bravo")], 2, true)),
        (
            "near-u".into(),
            snear(&[t("charlie"), t("alpha")], 3, false),
        ),
        ("not".into(), snot(&t("alpha"), &t("bravo"))),
        ("first".into(), sfirst(&t("delta"), 4)),
        (
            "containing".into(),
            scontaining(&snear(&[t("alpha"), t("bravo")], 4, false), &t("charlie")),
        ),
    ]
}

/// `PayloadScoreQuery` over each shape with `functions`, with the default
/// and with `includeSpanScore` false.
fn score_queries(field: &str, functions: &[&str]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (shape, sq) in score_shapes(field) {
        for f in functions {
            out.push((
                format!("psq-{field}-{shape}-{f}"),
                payload_score(&sq, f, None),
            ));
            out.push((
                format!("psq-{field}-{shape}-{f}-nospan"),
                payload_score(&sq, f, Some(false)),
            ));
        }
    }
    out
}

#[test]
fn payload_score_average_and_max_match_lucene_under_bm25() {
    let index = payload_index();
    let mut oracle = oracle(&index);
    add_all(
        &mut oracle,
        &score_queries("pf", &["AveragePayloadFunction", "MaxPayloadFunction"]),
    );
    assert_parity(&oracle, 200, 200);
}

#[test]
fn payload_score_min_and_sum_match_lucene_under_bm25() {
    let index = payload_index();
    let mut oracle = oracle(&index);
    add_all(
        &mut oracle,
        &score_queries("pf", &["MinPayloadFunction", "SumPayloadFunction"]),
    );
    assert_parity(&oracle, 200, 200);
}

#[test]
fn payload_score_match_lucene_under_classic_similarity_and_edge_cases() {
    let index = payload_index();
    let mut oracle = oracle(&index);
    oracle.similarity(&expr(
        "new",
        &["org.apache.lucene.search.similarities.ClassicSimilarity"],
    ));
    let all = [
        "AveragePayloadFunction",
        "MaxPayloadFunction",
        "MinPayloadFunction",
        "SumPayloadFunction",
    ];
    let mut queries: Vec<(String, String)> = score_queries("pf", &all)
        .into_iter()
        .filter(|(id, _)| id.contains("-term-") || id.contains("-near-o-") || id.contains("-or-"))
        .collect();
    // string payloads (first byte as the factor), no payloads, absent field
    for f in all {
        queries.push((
            format!("psq-ps-{f}"),
            payload_score(&st("ps", "bravo"), f, None),
        ));
        queries.push((
            format!("psq-body-{f}"),
            payload_score(&st("body", "alpha"), f, None),
        ));
    }
    queries.push((
        "psq-nofield".into(),
        payload_score(&st("nofield", "alpha"), "MaxPayloadFunction", None),
    ));
    queries.push((
        "psq-none".into(),
        payload_score(&st("pf", "nonexistent"), "MaxPayloadFunction", None),
    ));
    // a PayloadScoreQuery nested in a span query
    queries.push((
        "psq-nested".into(),
        snear(
            &[
                payload_score(&st("pf", "alpha"), "SumPayloadFunction", Some(true)),
                st("pf", "bravo"),
            ],
            3,
            false,
        ),
    ));
    add_all(&mut oracle, &queries);
    assert_parity(&oracle, 200, 200);
}

fn text(s: &str) -> Payload {
    Payload::Text(s.to_string())
}

fn int(v: i32) -> Payload {
    Payload::Bytes(v.to_be_bytes().to_vec())
}

fn float(v: f32) -> Payload {
    Payload::Bytes(v.to_be_bytes().to_vec())
}

const OPS: [&str; 5] = ["EQ", "GT", "GTE", "LT", "LTE"];

/// `SpanPayloadCheckQuery` of payload type `t` over `field` with each match
/// operation, each `(shape, payloads)` case.
fn check_queries(
    field: &str,
    t: &str,
    cases: Vec<(&str, String, Vec<Payload>)>,
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (name, m, payloads) in &cases {
        for op in OPS {
            out.push((
                format!("check-{field}-{t}-{op}-{name}"),
                payload_check(m, payloads, Some((t, op))),
            ));
        }
    }
    out
}

#[test]
fn span_payload_check_string_matches_lucene() {
    let index = payload_index();
    let mut oracle = oracle(&index);
    let s = |w: &str| st("ps", w);
    let cases = vec![
        ("term-3", s("alpha"), vec![text("pos: 3")]),
        ("term-10", s("bravo"), vec![text("pos: 10")]),
        ("term-t5", s("alpha"), vec![text("t5")]),
        (
            "near-o",
            snear(&[s("alpha"), s("bravo")], 0, true),
            vec![text("pos: 1"), text("pos: 2")],
        ),
        (
            "near-u",
            snear(&[s("bravo"), s("alpha")], 1, false),
            vec![text("pos: 4"), text("pos: 3")],
        ),
        ("null", s("alpha"), vec![Payload::Null]),
        (
            "more-payloads",
            s("alpha"),
            vec![text("pos: 1"), text("pos: 2")],
        ),
        (
            "fewer-payloads",
            snear(&[s("alpha"), s("bravo")], 2, true),
            vec![text("pos: 9")],
        ),
        ("empty", s("charlie"), vec![]),
    ];
    let mut queries = check_queries("ps", "STRING", cases);
    // the two-argument constructor: STRING, EQ
    queries.push((
        "check-default".into(),
        payload_check(&s("alpha"), &[text("pos: 0")], None),
    ));
    queries.push((
        "check-default-near".into(),
        payload_check(
            &snear(&[s("alpha"), s("alpha")], 3, true),
            &[text("pos: 0"), text("pos: 1")],
            None,
        ),
    ));
    add_all(&mut oracle, &queries);
    assert_parity(&oracle, 100, 200);
}

#[test]
fn span_payload_check_int_and_float_match_lucene() {
    let index = payload_index();
    let mut oracle = oracle(&index);
    let i = |w: &str| st("pi", w);
    let f = |w: &str| st("pfl", w);
    let int_cases = vec![
        ("neg", i("alpha"), vec![int(-10)]),
        ("zero", i("bravo"), vec![int(0)]),
        ("mid", i("alpha"), vec![int(50)]),
        (
            "near",
            snear(&[i("alpha"), i("bravo")], 1, false),
            vec![int(-50), int(100)],
        ),
        ("null", i("charlie"), vec![Payload::Null]),
    ];
    let float_cases = vec![
        ("neg", f("alpha"), vec![float(-1.0)]),
        ("zero", f("bravo"), vec![float(0.0)]),
        ("mid", f("alpha"), vec![float(2.5)]),
        ("nan", f("bravo"), vec![float(f32::NAN)]),
        (
            "near",
            snear(&[f("alpha"), f("bravo")], 1, true),
            vec![float(-3.0), float(6.75)],
        ),
    ];
    let mut queries = check_queries("pi", "INT", int_cases);
    queries.extend(check_queries("pfl", "FLOAT", float_cases));
    add_all(&mut oracle, &queries);
    assert_parity(&oracle, 100, 200);
}

/// Positive control: one changed collected payload, or one changed score
/// bit, fails the comparison at its record.
#[test]
fn payload_oracle_detects_a_perturbed_dump() {
    let index = payload_index();
    let mut oracle = oracle(&index);
    let queries = vec![
        (
            "psq-max".to_string(),
            payload_score(&st("pf", "alpha"), "MaxPayloadFunction", None),
        ),
        (
            "check-int".to_string(),
            payload_check(&st("pi", "alpha"), &[int(0)], Some(("INT", "GT"))),
        ),
    ];
    add_all(&mut oracle, &queries);
    let (java, rust) = assert_parity(&oracle, 5, 5);

    let perturbed = perturb(&rust, "spans\tcheck-int", "coll\t");
    let diff = compare(&java, &perturbed).expect_err("a perturbed payload must fail");
    assert!(
        diff.contains("record `spans\tcheck-int`"),
        "the failure names the perturbed record: {diff}"
    );
    let perturbed = perturb(&rust, "query\tpsq-max", "hit\t");
    let diff = compare(&java, &perturbed).expect_err("a perturbed hit must fail");
    assert!(
        diff.contains("record `query\tpsq-max`"),
        "the failure names the perturbed record: {diff}"
    );
}
