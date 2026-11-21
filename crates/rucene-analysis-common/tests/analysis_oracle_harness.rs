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

//! Proof of the analysis Java-oracle harness (rmp #446,
//! [`analysis_oracle`]): `StandardAnalyzer` (lucene-core) and
//! `WhitespaceAnalyzer` (this module), as analyzers, as a `CustomAnalyzer`
//! chain and as a factory chain, analyse the standard corpus (the fixed edge
//! cases and 300 seeded random inputs) identically on Lucene 10.5.0 and
//! Rucene; and the comparison fails, at the right token, on a perturbed Rust
//! dump and on a Rust spec that builds a different analyzer (positive
//! controls).

mod analysis_oracle;

use analysis_oracle::{compare, corpus, Arg, Chain, Oracle, Spec};
use rucene::analysis::{Analyzer, CharArraySet, StandardAnalyzer};
use rucene_analysis_common::analysis::core::whitespace_analyzer::WhitespaceAnalyzer;

/// The seed of the random half of the corpus.
const SEED: i64 = 0x0446_1050;

/// The number of random inputs.
const RANDOM_INPUTS: usize = 300;

/// The stop words of the stop-set specs.
const STOP_WORDS: [&str; 5] = ["the", "a", "of", "it's", "caf\u{e9}"];

/// The specs of the proof: every spec kind over the two analyzers.
fn oracle() -> Oracle {
    let mut o = Oracle::new(corpus::standard(SEED, RANDOM_INPUTS));
    o.spec(Spec::analyzer(
        "standard",
        "org.apache.lucene.analysis.standard.StandardAnalyzer",
        || Ok(Box::new(StandardAnalyzer::new()) as Box<dyn Analyzer>),
    ))
    .spec(
        Spec::analyzer(
            "standard-stop-max5",
            "org.apache.lucene.analysis.standard.StandardAnalyzer",
            || {
                let mut a =
                    StandardAnalyzer::with_stopwords(CharArraySet::from_iter(STOP_WORDS, false));
                a.set_max_token_length(5);
                Ok(Box::new(a) as Box<dyn Analyzer>)
            },
        )
        .arg(Arg::CharSet(
            STOP_WORDS.iter().map(|s| s.to_string()).collect(),
        ))
        .call("setMaxTokenLength", Arg::Int(5)),
    )
    .spec(Spec::analyzer(
        "whitespace",
        "org.apache.lucene.analysis.core.WhitespaceAnalyzer",
        || Ok(Box::new(WhitespaceAnalyzer::new()) as Box<dyn Analyzer>),
    ))
    .spec(
        Spec::analyzer(
            "whitespace-max7",
            "org.apache.lucene.analysis.core.WhitespaceAnalyzer",
            || Ok(Box::new(WhitespaceAnalyzer::with_max_token_length(7)) as Box<dyn Analyzer>),
        )
        .arg(Arg::Int(7)),
    )
    .spec(Spec::custom(
        "custom-standard",
        Chain::new().tokenizer("standard", &[("maxTokenLength", "6")]),
    ))
    .spec(Spec::factories(
        "factories-whitespace",
        Chain::new().tokenizer("whitespace", &[("rule", "unicode"), ("maxTokenLen", "9")]),
    ));
    o
}

/// The dumps agree, and Lucene's covers what the comparison claims: every
/// spec and input, and tokens of every `StandardTokenizer` type the corpus
/// is built to reach.
#[test]
fn standard_and_whitespace_analyzers_match_lucene() {
    let o = oracle();
    let java = o.java_dump();
    o.assert_covers(&java);
    // Every spec builds and analyses every input: a spec that fails on both
    // sides would otherwise compare equal without proving anything.
    assert!(
        !java
            .lines()
            .any(|l| l.starts_with("spec-error ") || l.starts_with("error ")),
        "Lucene failed a spec or an input:\n{}",
        java.lines()
            .filter(|l| l.starts_with("spec-error ") || l.starts_with("error "))
            .collect::<Vec<_>>()
            .join("\n")
    );
    for spec in o.specs() {
        let section = java
            .split(&format!("spec {}\n", spec.id()))
            .nth(1)
            .expect("the spec's section");
        let section = section.split("\nspec ").next().unwrap();
        assert!(
            section.lines().filter(|l| l.starts_with("tok ")).count() > 1_000,
            "spec {} must produce tokens",
            spec.id()
        );
    }
    for token_type in [
        "<ALPHANUM>",
        "<NUM>",
        "<SOUTHEAST_ASIAN>",
        "<IDEOGRAPHIC>",
        "<HIRAGANA>",
        "<KATAKANA>",
        "<HANGUL>",
        "<EMOJI>",
    ] {
        assert!(
            java.contains(&format!(" type={token_type} ")),
            "the corpus produces no {token_type} token"
        );
    }
    if let Err(report) = compare(&java, &o.rust_dump(), &o) {
        panic!("{report}");
    }
}

/// Positive control: one attribute of one token changed in Rucene's dump is
/// reported at that spec, input and token.
#[test]
fn a_perturbed_rust_dump_fails() {
    let o = oracle();
    let java = o.java_dump();
    let rust = o.rust_dump();
    compare(&java, &rust, &o).expect("the unperturbed dumps agree");

    // The second token of input 7 of spec `whitespace`: its position length.
    let mut perturbed = String::new();
    let (mut spec, mut input, mut token) = ("", "", 0);
    for line in rust.lines() {
        if let Some(id) = line.strip_prefix("spec ") {
            spec = id;
        } else if let Some(i) = line.strip_prefix("input ") {
            input = i;
            token = 0;
        } else if line.starts_with("tok ") {
            token += 1;
        }
        if spec == "whitespace" && input == "7" && token == 2 && line.starts_with("tok ") {
            assert!(line.contains(" posLen=1 "), "{line}");
            perturbed.push_str(&line.replace(" posLen=1 ", " posLen=2 "));
        } else {
            perturbed.push_str(line);
        }
        perturbed.push('\n');
    }
    assert_ne!(perturbed, rust, "the perturbation must change the dump");
    let report = compare(&java, &perturbed, &o).expect_err("a perturbed dump must fail");
    assert!(
        report.contains("in 1 spec(s)")
            && report.contains(
                "spec `whitespace` (new org.apache.lucene.analysis.core.WhitespaceAnalyzer())"
            )
            && report.contains(", input 7 [")
            && report.contains("], token 1:")
            && report.contains("Rucene: tok term=")
            && report.contains(" posLen=2 "),
        "the report must locate the perturbed token:\n{report}"
    );
}

/// Positive control: a Rust spec that builds another analyzer than its Java
/// side (`setMaxTokenLength(5)` missing) fails at the first token longer than
/// five characters.
#[test]
fn a_mismatched_rust_spec_fails() {
    let mut o = Oracle::new(corpus::standard(SEED, 20));
    o.spec(
        Spec::analyzer(
            "standard-max5",
            "org.apache.lucene.analysis.standard.StandardAnalyzer",
            || Ok(Box::new(StandardAnalyzer::new()) as Box<dyn Analyzer>),
        )
        .call("setMaxTokenLength", Arg::Int(5)),
    );
    let java = o.java_dump();
    o.assert_covers(&java);
    let report = compare(&java, &o.rust_dump(), &o).expect_err("a mismatched spec must fail");
    assert!(
        report.contains("spec `standard-max5`") && report.contains("Lucene: tok term="),
        "{report}"
    );
}

/// The corpus is reproducible from its seed and holds what it promises:
/// supplementary characters, HTML, URLs and e-mails, non-ASCII digits, a token
/// longer than 255 characters and an input longer than 4096 UTF-16 units.
#[test]
fn the_corpus_is_seeded_and_covers_its_classes() {
    assert_eq!(corpus::random(SEED, 50), corpus::random(SEED, 50));
    assert_ne!(corpus::random(SEED, 50), corpus::random(SEED + 1, 50));
    let all = corpus::standard(SEED, RANDOM_INPUTS);
    assert_eq!(all.len(), corpus::edge_cases().len() + RANDOM_INPUTS);
    let text: Vec<String> = all.iter().map(|u| String::from_utf16(u).unwrap()).collect();
    let any = |p: &dyn Fn(&str) -> bool| text.iter().any(|t| p(t));
    assert!(any(&|t| t.chars().any(|c| c as u32 > 0xffff)));
    assert!(any(&|t| t.contains("<p class=")));
    assert!(any(&|t| t.contains("https://")));
    assert!(any(&|t| t.contains("@example.com")));
    assert!(any(&|t| t.contains('\u{661}')));
    assert!(any(&|t| t.contains(&"a".repeat(300))));
    assert!(all.iter().any(|u| u.len() > 4096));
    assert!(all.iter().any(|u| u.is_empty()));
}
