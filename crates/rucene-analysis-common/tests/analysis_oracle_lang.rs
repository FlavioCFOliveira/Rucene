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

//! Java-oracle portability tests of the per-language packages (rmp #468
//! `ca da nl fr gl de it no pt es sv`, #469 `br el id ga ro tr`, #470 `hy eu
//! bg cz et fi hu lv lt ru sr`, #471 `ar bn hi in ne fa ckb ta te`): every
//! analyzer, filter, char filter and factory of the 43 packages analyses
//! the same inputs on Lucene 10.5.0 and Rucene through the harness of #446
//! ([`analysis_oracle`]), and every token attribute is compared.
//!
//! **Specs.** Each analyzer through its three public constructors (no
//! arguments; a stop word set; a stop word set and a stem exclusion set,
//! `DutchAnalyzer`'s stem exclusion table); each of the 49 SPI factories
//! after a `whitespace` tokenizer, after `standard` + `lowercase`, and after
//! a `keywordMarker` (so that the keyword attribute is honoured), plus every
//! documented parameter value (`variant` `nb`/`nn`/`no` of the Norwegian
//! stemmers, `stemDerivational`, `haircut`) and the `persian` char filter.
//!
//! **Corpora.** One launch per task over the harness's standard corpus
//! (fixed edge cases and 200 seeded random inputs), every string literal of
//! the task's ported Lucene tests (the language texts their authors chose),
//! and words built from the suffix and replacement literals of each
//! stemmer and normalizer of the task after stems of the language, so that
//! the rules are reached. Then one launch per language with Lucene test
//! vocabulary (`*testdata.zip`, the Norwegian and Swedish lists, the German
//! `data.txt`): its analyzer and its filters over every word.

mod analysis_oracle;
mod support;

use std::collections::BTreeSet;
use std::path::PathBuf;

use analysis_oracle::{corpus, Arg, Chain, Oracle, Spec};
use randomizedtesting::jdk::random::{JdkRandom, Random};
use rucene::analysis::{Analyzer, CharArraySet};
use rucene_analysis_common::analysis as lang;
use rucene_test_framework::util::TestUtil;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// UTF-16 inputs of `texts`.
fn inputs<S: AsRef<str>>(texts: &[S]) -> Vec<Vec<u16>> {
    texts
        .iter()
        .map(|s| s.as_ref().encode_utf16().collect())
        .collect()
}

/// `words` as inputs of 50 words each.
fn word_inputs(words: &[String]) -> Vec<Vec<u16>> {
    words
        .chunks(50)
        .map(|c| c.join(" ").encode_utf16().collect())
        .collect()
}

/// Runs the oracle, checks that Lucene built every spec, and compares the
/// dumps.
fn assert_oracle(o: &Oracle) {
    let java = o.java_dump();
    o.assert_covers(&java);
    let mut spec_errors: Vec<String> = Vec::new();
    let mut spec = "";
    for line in java.lines() {
        if let Some(id) = line.strip_prefix("spec ") {
            spec = id;
        } else if line.starts_with("spec-error ") {
            spec_errors.push(format!("{spec}: {line}"));
        }
    }
    assert!(
        spec_errors.is_empty(),
        "Lucene refused a spec: {spec_errors:?}"
    );
    if let Err(report) = analysis_oracle::compare(&java, &o.rust_dump(), o) {
        panic!("{report}");
    }
}

/// The value of a Rust string literal body (`\u{..}`, `\n`, `\t`, `\\`,
/// `\"` escapes).
fn rust_unescape(lit: &str) -> String {
    let mut out = String::new();
    let mut it = lit.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('u') => {
                assert_eq!(it.next(), Some('{'));
                let hex: String = it.by_ref().take_while(|&c| c != '}').collect();
                out.push(char::from_u32(u32::from_str_radix(&hex, 16).unwrap()).unwrap());
            }
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('0') => out.push('\0'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

/// The string literals of Rust source `source`, outside comments.
fn rust_literals(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in source.lines() {
        let code = line.trim_start();
        if code.starts_with("//") {
            continue;
        }
        let bytes = line.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'/' if bytes.get(i + 1) == Some(&b'/') => break,
                b'\'' => {
                    // a char literal ('"' among them) or a lifetime
                    if bytes.get(i + 2) == Some(&b'\'') {
                        i += 3;
                    } else if bytes.get(i + 1) == Some(&b'\\') {
                        i += 2;
                        while i < bytes.len() && bytes[i] != b'\'' {
                            i += 1;
                        }
                        i += 1;
                    } else {
                        i += 1;
                    }
                }
                b'"' => {
                    let start = i + 1;
                    let mut j = start;
                    while j < bytes.len() && bytes[j] != b'"' {
                        j += if bytes[j] == b'\\' { 2 } else { 1 };
                    }
                    out.push(rust_unescape(&line[start..j.min(bytes.len())]));
                    i = j + 1;
                }
                _ => i += 1,
            }
        }
    }
    out
}

/// The crate's root directory.
fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The string literals of each test class module of the generated suites
/// and of the hand-ported ones, by Java package (the language texts of the
/// ported Lucene tests).
fn test_literals(package: &str) -> Vec<String> {
    let mut out = Vec::new();
    for file in [
        "tests/analysis_lang_generated/mod.rs",
        "tests/analysis_lang_tests.rs",
    ] {
        let source = std::fs::read_to_string(crate_dir().join(file)).expect("a test source");
        // each class's suite names its class: `"org.apache.lucene.analysis.<pkg>.Test..."`
        for module in source.split("\nmod test_").skip(1) {
            let marker = format!("\"org.apache.lucene.analysis.{package}.Test");
            if module.contains(&marker) {
                out.extend(
                    rust_literals(module)
                        .into_iter()
                        .filter(|s| !s.starts_with("org.apache.lucene.") && !s.contains('$')),
                );
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The string literals of the stemmers, normalizers and filters of package
/// `package` (`src/analysis/<package>/`): their suffixes and replacements.
fn rule_literals(package: &str) -> Vec<String> {
    let dir = crate_dir().join("src/analysis").join(package);
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("the package sources")
        .map(|e| e.expect("an entry").path())
        .filter(|p| {
            let n = p.file_name().unwrap().to_string_lossy().into_owned();
            n.ends_with("stemmer.rs")
                || n.ends_with("normalizer.rs")
                || (n.ends_with("_filter.rs") && !n.ends_with("_factory.rs"))
        })
        .collect();
    files.sort();
    let mut out = BTreeSet::new();
    for f in files {
        let source = std::fs::read_to_string(&f).expect("a source");
        for lit in rust_literals(&source) {
            if !lit.is_empty() && lit.chars().count() <= 12 && !lit.contains(char::is_whitespace) {
                out.insert(lit);
            }
        }
    }
    out.into_iter().collect()
}

/// Words that reach the rules of package `package`: every rule literal
/// alone, after three seeded stems taken from the package's test texts, and
/// 300 seeded pairs of literals after a stem.
fn rule_words(package: &str, seed: i64) -> Vec<String> {
    let rules = rule_literals(package);
    let mut stems: Vec<String> = test_literals(package)
        .iter()
        .flat_map(|t| t.split_whitespace().map(str::to_string).collect::<Vec<_>>())
        .filter(|w| w.chars().count() >= 3 && w.chars().all(char::is_alphabetic))
        .map(|w| w.chars().take(4).collect())
        .collect();
    stems.sort();
    stems.dedup();
    if stems.is_empty() || rules.is_empty() {
        return rules;
    }
    let random = JdkRandom::new(seed);
    let mut words = Vec::new();
    for r in &rules {
        words.push(r.clone());
        for _ in 0..3 {
            let stem = &stems[random.next_int_bound(stems.len() as i32) as usize];
            words.push(format!("{stem}{r}"));
        }
    }
    for _ in 0..300 {
        let stem = &stems[random.next_int_bound(stems.len() as i32) as usize];
        let a = &rules[random.next_int_bound(rules.len() as i32) as usize];
        let b = &rules[random.next_int_bound(rules.len() as i32) as usize];
        words.push(format!("{stem}{a}{b}"));
    }
    words
}

/// A test resource's bytes, read from `tests/resources`.
fn resource(path: &str) -> Vec<u8> {
    let full = crate_dir().join("tests/resources").join(path);
    std::fs::read(&full).unwrap_or_else(|e| panic!("read {}: {e}", full.display()))
}

/// The tab-separated columns of the non-comment lines of `text`.
fn columns(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .flat_map(|l| {
            l.split('\t')
                .map(|s| s.trim().to_string())
                .collect::<Vec<_>>()
        })
        .filter(|s| !s.is_empty())
        .collect()
}

/// Every column of the files of a zip test resource (words and their
/// expected stems).
fn zip_words(path: &str) -> Vec<String> {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "rucene-oracle-t6-zip-{}-{}-{}",
        path.replace('/', "_"),
        std::process::id(),
        N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("a temp dir");
    TestUtil::unzip(&resource(path), &dir).expect("unzip");
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("the unzipped files")
        .map(|e| e.expect("an entry").path())
        .filter(|p| p.is_file())
        .collect();
    entries.sort();
    let mut words = Vec::new();
    for p in entries {
        let bytes = std::fs::read(&p).expect("an unzipped file");
        words.extend(columns(
            &String::from_utf8(bytes).expect("UTF-8 vocabulary"),
        ));
    }
    let _ = std::fs::remove_dir_all(&dir);
    words
}

/// A `CharArraySet` of `words`, case-sensitive.
fn set(words: &[String]) -> CharArraySet {
    CharArraySet::from_iter(words.iter().map(String::as_str), false)
}

// ---------------------------------------------------------------------------
// Specs
// ---------------------------------------------------------------------------

/// The analyzers of the four tasks: package, Java class, and the Rust
/// builders of the no-argument, stop-set and stop-set-plus-exclusion-set
/// constructors.
type Build0 = fn() -> Box<dyn Analyzer>;
type Build1 = fn(CharArraySet) -> Box<dyn Analyzer>;
type Build2 = fn(CharArraySet, &CharArraySet) -> Box<dyn Analyzer>;

macro_rules! analyzer_row {
    ($pkg:literal, $module:ident :: $file:ident :: $ty:ident) => {
        (
            $pkg,
            concat!("org.apache.lucene.analysis.", $pkg, ".", stringify!($ty)),
            (|| Box::new(lang::$module::$file::$ty::new())) as Build0,
            (|s| Box::new(lang::$module::$file::$ty::with_stop_words(s))) as Build1,
            Some(
                (|s, e| Box::new(lang::$module::$file::$ty::with_stem_exclusion_set(s, e)))
                    as Build2,
            ),
        )
    };
}

type Row = (&'static str, &'static str, Build0, Build1, Option<Build2>);

fn analyzers() -> Vec<Row> {
    vec![
        analyzer_row!("ca", ca::catalan_analyzer::CatalanAnalyzer),
        analyzer_row!("da", da::danish_analyzer::DanishAnalyzer),
        (
            "nl",
            "org.apache.lucene.analysis.nl.DutchAnalyzer",
            (|| Box::new(lang::nl::dutch_analyzer::DutchAnalyzer::new())) as Build0,
            (|s| Box::new(lang::nl::dutch_analyzer::DutchAnalyzer::with_stop_words(s))) as Build1,
            Some(
                (|s, e| {
                    Box::new(
                        lang::nl::dutch_analyzer::DutchAnalyzer::with_stem_exclusion_table(s, e),
                    )
                }) as Build2,
            ),
        ),
        analyzer_row!("fr", fr::french_analyzer::FrenchAnalyzer),
        analyzer_row!("gl", gl::galician_analyzer::GalicianAnalyzer),
        analyzer_row!("de", de::german_analyzer::GermanAnalyzer),
        analyzer_row!("it", it::italian_analyzer::ItalianAnalyzer),
        analyzer_row!("no", no::norwegian_analyzer::NorwegianAnalyzer),
        analyzer_row!("pt", pt::portuguese_analyzer::PortugueseAnalyzer),
        analyzer_row!("es", es::spanish_analyzer::SpanishAnalyzer),
        analyzer_row!("sv", sv::swedish_analyzer::SwedishAnalyzer),
        analyzer_row!("br", br::brazilian_analyzer::BrazilianAnalyzer),
        (
            "el",
            "org.apache.lucene.analysis.el.GreekAnalyzer",
            (|| Box::new(lang::el::greek_analyzer::GreekAnalyzer::new())) as Build0,
            (|s| Box::new(lang::el::greek_analyzer::GreekAnalyzer::with_stop_words(s))) as Build1,
            // GreekAnalyzer has no (CharArraySet, CharArraySet) constructor
            None,
        ),
        analyzer_row!("id", id::indonesian_analyzer::IndonesianAnalyzer),
        analyzer_row!("ga", ga::irish_analyzer::IrishAnalyzer),
        analyzer_row!("ro", ro::romanian_analyzer::RomanianAnalyzer),
        analyzer_row!("tr", tr::turkish_analyzer::TurkishAnalyzer),
        analyzer_row!("hy", hy::armenian_analyzer::ArmenianAnalyzer),
        analyzer_row!("eu", eu::basque_analyzer::BasqueAnalyzer),
        analyzer_row!("bg", bg::bulgarian_analyzer::BulgarianAnalyzer),
        analyzer_row!("cz", cz::czech_analyzer::CzechAnalyzer),
        analyzer_row!("et", et::estonian_analyzer::EstonianAnalyzer),
        analyzer_row!("fi", fi::finnish_analyzer::FinnishAnalyzer),
        analyzer_row!("hu", hu::hungarian_analyzer::HungarianAnalyzer),
        analyzer_row!("lv", lv::latvian_analyzer::LatvianAnalyzer),
        analyzer_row!("lt", lt::lithuanian_analyzer::LithuanianAnalyzer),
        analyzer_row!("ru", ru::russian_analyzer::RussianAnalyzer),
        analyzer_row!("sr", sr::serbian_analyzer::SerbianAnalyzer),
        analyzer_row!("ar", ar::arabic_analyzer::ArabicAnalyzer),
        analyzer_row!("bn", bn::bengali_analyzer::BengaliAnalyzer),
        analyzer_row!("hi", hi::hindi_analyzer::HindiAnalyzer),
        analyzer_row!("ne", ne::nepali_analyzer::NepaliAnalyzer),
        analyzer_row!("fa", fa::persian_analyzer::PersianAnalyzer),
        analyzer_row!("ckb", ckb::sorani_analyzer::SoraniAnalyzer),
        analyzer_row!("ta", ta::tamil_analyzer::TamilAnalyzer),
        analyzer_row!("te", te::telugu_analyzer::TeluguAnalyzer),
    ]
}

/// The SPI token filter factories of each package, with the parameter
/// values each documents beyond the default.
fn factories(package: &str) -> Vec<(&'static str, Vec<(&'static str, &'static str)>)> {
    let plain = |names: &[&'static str]| -> Vec<(&'static str, Vec<(&'static str, &'static str)>)> {
        names.iter().map(|n| (*n, Vec::new())).collect()
    };
    match package {
        "fr" => plain(&["frenchLightStem", "frenchMinimalStem"]),
        "gl" => plain(&["galicianMinimalStem", "galicianStem"]),
        "de" => plain(&[
            "germanLightStem",
            "germanMinimalStem",
            "germanNormalization",
            "germanStem",
        ]),
        "it" => plain(&["italianLightStem"]),
        "no" => {
            let mut v = Vec::new();
            for f in ["norwegianLightStem", "norwegianMinimalStem"] {
                v.push((f, Vec::new()));
                for variant in ["nb", "nn", "no"] {
                    v.push((f, vec![("variant", variant)]));
                }
            }
            v.push(("norwegianNormalization", Vec::new()));
            v
        }
        "pt" => plain(&[
            "portugueseLightStem",
            "portugueseMinimalStem",
            "portugueseStem",
        ]),
        "es" => plain(&[
            "spanishLightStem",
            "spanishMinimalStem",
            "spanishPluralStem",
        ]),
        "sv" => plain(&["swedishLightStem", "swedishMinimalStem"]),
        "br" => plain(&["brazilianStem"]),
        "el" => plain(&["greekLowercase", "greekStem"]),
        "id" => vec![
            ("indonesianStem", Vec::new()),
            ("indonesianStem", vec![("stemDerivational", "false")]),
            ("indonesianStem", vec![("stemDerivational", "true")]),
        ],
        "ga" => plain(&["irishLowercase"]),
        "ro" => plain(&["romanianNormalization"]),
        "tr" => plain(&["apostrophe", "turkishLowercase"]),
        "bg" => plain(&["bulgarianStem"]),
        "cz" => plain(&["czechStem"]),
        "fi" => plain(&["finnishLightStem"]),
        "hu" => plain(&["hungarianLightStem"]),
        "lv" => plain(&["latvianStem"]),
        "ru" => plain(&["russianLightStem"]),
        "sr" => vec![
            ("serbianNormalization", Vec::new()),
            ("serbianNormalization", vec![("haircut", "bald")]),
            ("serbianNormalization", vec![("haircut", "regular")]),
        ],
        "ar" => plain(&["arabicNormalization", "arabicStem"]),
        "bn" => plain(&["bengaliNormalization", "bengaliStem"]),
        "hi" => plain(&["hindiNormalization", "hindiStem"]),
        "in" => plain(&["indicNormalization"]),
        "fa" => plain(&["persianNormalization", "persianStem"]),
        "ckb" => plain(&["soraniNormalization", "soraniStem"]),
        "te" => plain(&["teluguNormalization", "teluguStem"]),
        _ => Vec::new(),
    }
}

/// A spec id suffix of factory parameters.
fn params_id(params: &[(&str, &str)]) -> String {
    params
        .iter()
        .map(|(k, v)| format!("-{k}={v}"))
        .collect::<String>()
}

/// The three constructor specs of `package`'s analyzer, if it has one:
/// stop words and exclusions are words of the package's test texts.
fn analyzer_specs(o: &mut Oracle, package: &str, texts: &[String]) {
    let Some((_, class, b0, b1, b2)) = analyzers().into_iter().find(|r| r.0 == package) else {
        return;
    };
    let mut words: Vec<String> = texts
        .iter()
        .flat_map(|t| {
            t.split_whitespace()
                .map(str::to_lowercase)
                .collect::<Vec<_>>()
        })
        .collect();
    words.sort();
    words.dedup();
    let stop: Vec<String> = words.iter().step_by(7).take(12).cloned().collect();
    let excl: Vec<String> = words.iter().skip(3).step_by(5).take(12).cloned().collect();
    let (s1, s2, e2) = (stop.clone(), stop.clone(), excl.clone());
    o.spec(Spec::analyzer(
        &format!("{package}-analyzer"),
        class,
        move || Ok(b0()),
    ))
    .spec(
        Spec::analyzer(&format!("{package}-analyzer-stop"), class, move || {
            Ok(b1(set(&s1)))
        })
        .arg(Arg::CharSet(stop.clone())),
    );
    if let Some(b2) = b2 {
        o.spec(
            Spec::analyzer(&format!("{package}-analyzer-stop-excl"), class, move || {
                Ok(b2(set(&s2), &set(&e2)))
            })
            .arg(Arg::CharSet(stop))
            .arg(Arg::CharSet(excl)),
        );
    }
}

/// The factory specs of `package`: each factory after `whitespace`, after
/// `standard` + `lowercase`, and after a `keywordMarker`.
fn factory_specs(o: &mut Oracle, package: &str) {
    for (name, params) in factories(package) {
        let id = format!("{package}-{name}{}", params_id(&params));
        o.spec(Spec::factories(
            &format!("{id}-ws"),
            Chain::new()
                .tokenizer("whitespace", &[])
                .filter(name, &params),
        ))
        .spec(Spec::factories(
            &format!("{id}-lc"),
            Chain::new()
                .tokenizer("standard", &[])
                .filter("lowercase", &[])
                .filter(name, &params),
        ))
        .spec(Spec::factories(
            &format!("{id}-kw"),
            Chain::new()
                .tokenizer("whitespace", &[])
                .filter("keywordMarker", &[("pattern", ".{2,5}")])
                .filter(name, &params),
        ));
    }
    if package == "fa" {
        o.spec(Spec::factories(
            "fa-persian-charfilter",
            Chain::new()
                .char_filter("persian", &[])
                .tokenizer("standard", &[]),
        ))
        .spec(Spec::factories(
            "fa-persian-charfilter-ws",
            Chain::new()
                .char_filter("persian", &[])
                .tokenizer("whitespace", &[])
                .filter("persianNormalization", &[]),
        ));
    }
}

/// The packages of each task.
const T468: [&str; 11] = [
    "ca", "da", "nl", "fr", "gl", "de", "it", "no", "pt", "es", "sv",
];
const T469: [&str; 6] = ["br", "el", "id", "ga", "ro", "tr"];
const T470: [&str; 11] = [
    "hy", "eu", "bg", "cz", "et", "fi", "hu", "lv", "lt", "ru", "sr",
];
const T471: [&str; 9] = ["ar", "bn", "hi", "in", "ne", "fa", "ckb", "ta", "te"];

/// One launch for the packages of a task: every analyzer and factory spec
/// over the standard corpus, the task's test texts and its rule words.
fn task_oracle(packages: &[&str], seed: i64) {
    let mut corpus = corpus::standard(seed, 200);
    let mut texts_all = Vec::new();
    let mut words_all = Vec::new();
    for package in packages {
        let texts = test_literals(package);
        assert!(
            !texts.is_empty() || *package == "in",
            "{package}: no test texts"
        );
        texts_all.extend(texts);
        words_all.extend(rule_words(package, seed ^ package.len() as i64));
    }
    // short texts joined ten to an input
    let joined: Vec<String> = texts_all.chunks(10).map(|c| c.join(" ")).collect();
    corpus.extend(inputs(&joined));
    corpus.extend(inputs(
        &texts_all
            .iter()
            .filter(|t| t.chars().count() > 40)
            .collect::<Vec<_>>(),
    ));
    corpus.extend(word_inputs(&words_all));
    let mut o = Oracle::new(corpus);
    for package in packages {
        analyzer_specs(&mut o, package, &test_literals(package));
        factory_specs(&mut o, package);
    }
    let n_specs = o.specs().len();
    assert_oracle(&o);
    println!(
        "{}: {n_specs} specs x {} inputs ({} test texts, {} rule words) equal",
        packages.join(","),
        o.corpus().len(),
        texts_all.len(),
        words_all.len()
    );
}

#[test]
fn oracle_468_analyzers_and_factories() {
    task_oracle(&T468, 0x468);
}

#[test]
fn oracle_469_analyzers_and_factories() {
    task_oracle(&T469, 0x469);
}

#[test]
fn oracle_470_analyzers_and_factories() {
    task_oracle(&T470, 0x470);
}

#[test]
fn oracle_471_analyzers_and_factories() {
    task_oracle(&T471, 0x471);
}

// ---------------------------------------------------------------------------
// Vocabularies
// ---------------------------------------------------------------------------

/// Lucene's test vocabularies of `package` (words and expected stems).
fn vocabulary(package: &str) -> Vec<String> {
    const A: &str = "org/apache/lucene/analysis/";
    let zips: &[&str] = match package {
        "fr" => &["fr/frlighttestdata.zip", "fr/frminimaltestdata.zip"],
        "gl" => &["gl/gltestdata.zip"],
        "de" => &["de/delighttestdata.zip", "de/deminimaltestdata.zip"],
        "it" => &["it/itlighttestdata.zip"],
        "pt" => &[
            "pt/ptlighttestdata.zip",
            "pt/ptminimaltestdata.zip",
            "pt/ptrslptestdata.zip",
        ],
        "es" => &["es/eslighttestdata.zip", "es/espluraltestdata.zip"],
        "sv" => &["sv/svlighttestdata.zip"],
        "fi" => &["fi/filighttestdata.zip"],
        "hu" => &["hu/hulighttestdata.zip"],
        "ru" => &["ru/rulighttestdata.zip"],
        "ckb" => &["ckb/ckbtestdata.zip"],
        _ => &[],
    };
    let texts: &[&str] = match package {
        "no" => &[
            "no/nb_light.txt",
            "no/nn_light.txt",
            "no/nb_minimal.txt",
            "no/nn_minimal.txt",
        ],
        "sv" => &["sv/minimal.txt"],
        "de" => &["de/data.txt"],
        _ => &[],
    };
    let mut words: Vec<String> = zips
        .iter()
        .flat_map(|z| zip_words(&format!("{A}{z}")))
        .collect();
    for t in texts {
        let text = String::from_utf8(resource(&format!("{A}{t}"))).expect("UTF-8");
        words.extend(columns(&text));
    }
    let mut seen = BTreeSet::new();
    words.retain(|w| seen.insert(w.clone()));
    words
}

/// One launch per language with Lucene test vocabulary: its analyzer and
/// its filters after `whitespace` over every word.
fn vocabulary_oracle(packages: &[&str]) {
    for package in packages {
        let words = vocabulary(package);
        assert!(words.len() > 100, "{package}: {} words", words.len());
        let mut o = Oracle::new(word_inputs(&words));
        if let Some((_, class, b0, _, _)) = analyzers().into_iter().find(|r| r.0 == *package) {
            o.spec(Spec::analyzer(
                &format!("{package}-analyzer"),
                class,
                move || Ok(b0()),
            ));
        }
        for (name, params) in factories(package) {
            o.spec(Spec::factories(
                &format!("{package}-{name}{}", params_id(&params)),
                Chain::new()
                    .tokenizer("whitespace", &[])
                    .filter(name, &params),
            ));
        }
        assert_oracle(&o);
        println!("{package}: {} vocabulary words equal", words.len());
    }
}

#[test]
fn oracle_468_vocabularies() {
    vocabulary_oracle(&["fr", "gl", "de", "it", "no", "pt", "es", "sv"]);
}

#[test]
fn oracle_470_vocabularies() {
    vocabulary_oracle(&["fi", "hu", "ru"]);
}

#[test]
fn oracle_471_vocabularies() {
    vocabulary_oracle(&["ckb"]);
}
