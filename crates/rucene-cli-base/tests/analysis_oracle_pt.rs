//! Portability test of the pt-PT analysis chain (rmp #535): the token
//! streams of [`PortugueseFoldingAnalyzer`] equal those of the Lucene 10.5.0
//! `CustomAnalyzer` its module documentation names, built by Lucene itself
//! from the same components in the same order:
//!
//! ```java
//! CustomAnalyzer.builder(configDir)
//!     .withTokenizer("standard")
//!     .addTokenFilter("lowercase")
//!     .addTokenFilter("asciiFolding")
//!     .addTokenFilter("stop", "words", "portuguese_stop_folded.txt")
//!     .addTokenFilter("patternReplace", "pattern", "oes$", "replacement", "ões", "replace", "first")
//!     .addTokenFilter("patternReplace", "pattern", "aes$", "replacement", "ães", "replace", "first")
//!     .addTokenFilter("patternReplace", "pattern", "ois$", "replacement", "óis", "replace", "first")
//!     .addTokenFilter("portugueseLightStem")
//!     .addTokenFilter("asciiFolding")
//!     .build();
//! ```
//!
//! The stop file is written by Lucene (`tests/oracle/PortugueseStopFolded.java`:
//! `ASCIIFoldingFilter.foldToASCII` of every word of
//! `PortugueseAnalyzer.getDefaultStopSet()`), so the Rust stop set is checked
//! against Lucene's, not against itself.
//!
//! The comparison reuses the Java-oracle harness of `rucene-analysis-common`
//! (`tests/analysis_oracle/mod.rs`): Lucene's dump of the `CustomAnalyzer`
//! and Rucene's dump of [`PortugueseFoldingAnalyzer`] are compared token by
//! token (term, bytes, type, position increment and length, offsets, flags,
//! payload, keyword flag, term frequency, attribute set, final offset).
//!
//! The corpus: the nine spelling groups of DECISION 3249, every Portuguese
//! stop word in its accented, folded and upper-case spellings, accents,
//! cedilla, tilde and case variants, every text of the crate's fixtures (real
//! contract objects, announcement descriptions, entity names), and the
//! harness's standard edge cases and seeded random inputs.

#[path = "../../rucene-analysis-common/tests/analysis_oracle/mod.rs"]
mod analysis_oracle;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use analysis_oracle::{compare, corpus, java_errors, run_java_file, Chain, Oracle, Spec};
use rucene::analysis::Analyzer;
use rucene_cli_base::analysis::{stop_set, PortugueseFoldingAnalyzer, PLURAL_RESTORATIONS};
use rucene_cli_base::json::{self, JsonValue};

/// The spec id shared by the Java and the Rust side.
const SPEC: &str = "pt-folding";

/// The seed of the random part of the corpus.
const SEED: i64 = 0x0535_1050;

/// The stop file name of the Java chain.
const STOP_FILE: &str = "portuguese_stop_folded.txt";

/// The nine spelling groups of DECISION 3249 (rmp #532).
const GROUPS: &[&[&str]] = &[
    &["ação", "acao", "AÇÃO", "ações", "acoes", "AÇÕES"],
    &["acção", "acções", "accao", "accoes"],
    &["pão", "pães", "paes"],
    &["limão", "limões", "limoes"],
    &["papéis", "papeis", "papel"],
    &["anzóis", "anzois", "anzol"],
    &["preço", "preco"],
    &["música", "musica"],
    &["português", "portugues"],
];

/// Hand-written pt-PT samples: accents, cedilla, tilde, circumflex, case,
/// stop words inside sentences, hyphens, numbers and short tokens.
const SAMPLES: &[&str] = &[
    "Aquisição de serviços de manutenção e conservação de edifícios",
    "AQUISIÇÃO DE SERVIÇOS DE MANUTENÇÃO E CONSERVAÇÃO DE EDIFÍCIOS",
    "aquisicao de servicos de manutencao e conservacao de edificios",
    "Empreitada de requalificação da Escola Básica n.º 2 de Évora",
    "Fornecimento de refeições escolares — ano letivo 2025/2026",
    "Câmara Municipal de São João da Madeira; Junta de Freguesia de Ílhavo",
    "Construção de vias férreas; reabilitação de pontes e viadutos",
    "não nao NÃO você voce VOCÊ está esta ESTÁ às as À à é e É",
    "pá fé PÓ nó dó lá cá já só",
    "Coração, emoções, canções, mãos, irmãs, alemães, capitães",
    "Hotéis, anéis, papéis, faróis, lençóis, caracóis, espanhóis",
    "Felizmente, rapidamente, provavelmente; portuguesa, portugueses",
    "Ação social escolar: acções de formação e informação",
    "Infraestruturas de Portugal, S.A. — Lote 3: Linha do Minho",
    "Consultoria jurídica (artigo 20.º, n.º 1, alínea d) do CCP)",
    "Município de Santa Maria da Feira e FEIRA VIVA - CULTURA E DESPORTO, E.M.",
    "Serviços Partilhados do Ministério da Saúde, EPE",
    "Gás natural, eletricidade em baixa tensão normal (BTN) e média tensão",
    "Ñandú, Æsir, Øresund, straße, façade, naïve, crème brûlée",
    "Shoes and goes: aes ois oes — anões, balões, leões, pães, mães",
];

/// Every text of the crate's fixtures that the pt-PT chain analyses: the
/// strings (scalar or list elements) of every member of every record.
fn fixture_texts() -> Vec<String> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
        .collect();
    paths.sort();
    let mut texts = BTreeSet::new();
    for path in paths {
        for line in std::fs::read_to_string(&path).unwrap().lines() {
            if line.trim().is_empty() {
                continue;
            }
            let JsonValue::Object(members) = json::parse(line).unwrap() else {
                panic!("{}: not an object", path.display());
            };
            for (_, value) in members {
                let values = match value {
                    JsonValue::Array(items) => items,
                    other => vec![other],
                };
                for value in values {
                    if let JsonValue::String(text) = value {
                        // Words only: codes, dates and URLs add nothing.
                        if text.chars().filter(|c| c.is_alphabetic()).count() >= 3
                            && !text.starts_with("http")
                        {
                            texts.insert(text);
                        }
                    }
                }
            }
        }
    }
    texts.into_iter().collect()
}

/// The stop words of Lucene's `PortugueseAnalyzer`, ASCII-folded by Lucene
/// (`tests/oracle/PortugueseStopFolded.java`), in sorted order.
fn lucene_folded_stop_words() -> Vec<String> {
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/oracle/PortugueseStopFolded.java");
    let (ok, stdout, stderr) = run_java_file(&source, &[]);
    assert!(
        ok,
        "PortugueseStopFolded.java failed:\n{}",
        java_errors(&stderr)
    );
    let words: Vec<String> = stdout.lines().map(str::to_owned).collect();
    assert!(words.len() > 100, "Lucene's Portuguese stop set: {words:?}");
    words
}

/// The whole corpus, UTF-16.
fn corpus_inputs(stop_words: &[String]) -> Vec<Vec<u16>> {
    let mut inputs: Vec<String> = Vec::new();
    for group in GROUPS {
        inputs.extend(group.iter().map(|form| (*form).to_owned()));
        inputs.push(group.join(" "));
    }
    inputs.extend(SAMPLES.iter().map(|sample| (*sample).to_owned()));
    // Every stop word as Lucene lists it, folded, upper-case, and inside a
    // sentence (positions after a removed word).
    let accented: Vec<String> = analysis_oracle_stop_words();
    inputs.push(accented.join(" "));
    inputs.push(stop_words.join(" "));
    inputs.push(accented.join(" ").to_uppercase());
    for word in &accented {
        inputs.push(format!("contrato {word} fornecimento"));
    }
    inputs.extend(fixture_texts());
    let mut units: Vec<Vec<u16>> = inputs
        .iter()
        .map(|text| text.encode_utf16().collect())
        .collect();
    units.extend(corpus::standard(SEED, 100));
    units
}

/// Lucene's Portuguese stop words as Rucene's `PortugueseAnalyzer` holds them
/// (accented): the port of `snowball/portuguese_stop.txt`.
fn analysis_oracle_stop_words() -> Vec<String> {
    let set = rucene_analysis_common::analysis::pt::portuguese_analyzer::PortugueseAnalyzer::
        get_default_stop_set();
    let mut words: Vec<String> = set.iter().map(String::from_utf16_lossy).collect();
    words.sort();
    words
}

/// The corpus index of the input joining every form of group `g` (see
/// [`corpus_inputs`]: each group's forms, then their join).
fn group_input_index(g: usize) -> usize {
    GROUPS[..=g]
        .iter()
        .map(|group| group.len() + 1)
        .sum::<usize>()
        - 1
}

/// The terms of input `input` in a dump, in order.
fn java_terms(dump: &str, input: usize) -> Vec<String> {
    let header = format!("input {input}");
    dump.lines()
        .skip_while(|line| *line != header)
        .skip(1)
        .take_while(|line| !line.starts_with("input "))
        .filter_map(|line| line.strip_prefix("tok term="))
        .map(|rest| rest.split(' ').next().unwrap().to_owned())
        .collect()
}

/// A config directory holding the stop file Lucene derived.
fn config_dir(label: &str, stop_words: &[String]) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "rucene-cli-base-pt-oracle-{label}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut text = String::new();
    for word in stop_words {
        text.push_str(word);
        text.push('\n');
    }
    std::fs::write(dir.join(STOP_FILE), text).unwrap();
    dir
}

/// The Java `CustomAnalyzer` chain; `restore_plurals` false drops the three
/// `PatternReplaceFilter`s (a positive control).
fn java_chain(dir: &Path, restore_plurals: bool) -> Chain {
    let mut chain = Chain::new()
        .config_dir(dir)
        .tokenizer("standard", &[])
        .filter("lowercase", &[])
        .filter("asciiFolding", &[])
        .filter("stop", &[("words", STOP_FILE)]);
    if restore_plurals {
        for (pattern, replacement) in PLURAL_RESTORATIONS {
            chain = chain.filter(
                "patternReplace",
                &[
                    ("pattern", pattern),
                    ("replacement", replacement),
                    ("replace", "first"),
                ],
            );
        }
    }
    chain
        .filter("portugueseLightStem", &[])
        .filter("asciiFolding", &[])
}

/// Lucene's dump of `chain` and Rucene's dump of
/// [`PortugueseFoldingAnalyzer`] over `inputs`, with the Java oracle (for the
/// report).
fn dumps(inputs: Vec<Vec<u16>>, chain: Chain) -> (String, String, Oracle) {
    let mut java = Oracle::new(inputs.clone());
    java.spec(Spec::custom(SPEC, chain));
    let java_dump = java.java_dump();
    java.assert_covers(&java_dump);
    let mut rust = Oracle::new(inputs);
    rust.spec(Spec::analyzer(
        SPEC,
        "rucene_cli_base.analysis.PortugueseFoldingAnalyzer",
        || Ok(Box::new(PortugueseFoldingAnalyzer::new()) as Box<dyn Analyzer>),
    ));
    let rust_dump = rust.rust_dump();
    (java_dump, rust_dump, java)
}

#[test]
fn the_stop_set_is_lucenes_portuguese_set_folded_by_lucene() {
    let lucene: BTreeSet<String> = lucene_folded_stop_words().into_iter().collect();
    let rust: BTreeSet<String> = stop_set().iter().map(String::from_utf16_lossy).collect();
    assert_eq!(rust, lucene);
}

#[test]
fn pt_chain_matches_lucene_custom_analyzer() {
    let stop_words = lucene_folded_stop_words();
    let inputs = corpus_inputs(&stop_words);
    assert!(inputs.len() > 400, "corpus size {}", inputs.len());
    let dir = config_dir("match", &stop_words);
    let (java, rust, oracle) = dumps(inputs, java_chain(&dir, true));
    assert!(
        !java
            .lines()
            .any(|l| l.starts_with("spec-error ") || l.starts_with("error ")),
        "Lucene failed the spec or an input:\n{}",
        java.lines()
            .filter(|l| l.starts_with("spec-error ") || l.starts_with("error "))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let tokens = java.lines().filter(|l| l.starts_with("tok ")).count();
    assert!(tokens > 4_000, "Lucene produced {tokens} tokens");
    // On Lucene's side, every form of each spelling group analyses to one
    // and the same term (the group's input joining all its forms).
    for (g, group) in GROUPS.iter().enumerate() {
        let terms = java_terms(&java, group_input_index(g));
        assert_eq!(terms.len(), group.len(), "group {group:?}: {terms:?}");
        assert!(
            terms.iter().all(|term| term == &terms[0]),
            "group {group:?} has several terms on Lucene's side: {terms:?}"
        );
    }
    if let Err(report) = compare(&java, &rust, &oracle) {
        panic!("{report}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Positive control: Lucene's chain without the three plural restorations
/// differs from Rucene's analyzer, at the first unaccented `-oes` plural.
#[test]
fn a_chain_without_the_plural_restorations_fails() {
    let stop_words = lucene_folded_stop_words();
    let dir = config_dir("control", &stop_words);
    let inputs: Vec<Vec<u16>> = ["contrato", "acoes"]
        .iter()
        .map(|text| text.encode_utf16().collect())
        .collect();
    let (java, rust, oracle) = dumps(inputs, java_chain(&dir, false));
    let report = compare(&java, &rust, &oracle).expect_err("the control chain must differ");
    assert!(
        report.contains(", input 1 [") && report.contains("acoe"),
        "the report must locate `acoes`:\n{report}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
