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

//! Java-oracle portability tests of the packages `analysis.hunspell` (rmp
//! #462, #463), `analysis.compound` and `analysis.compound.hyphenation`
//! (#461), `analysis.morph` (#466) and `collation` (#472): Lucene 10.5.0 on
//! JDK 25 and Rucene compute the same things and the dumps are compared line
//! by line.
//!
//! * **Token streams** ([`token_stream_oracle`], one `AnalysisOracle.java`
//!   launch): the `hunspellStem` factory over 30 bundled test dictionaries
//!   (with `ignoreCase`, `longestOnly` and two dictionaries at once), the
//!   `dictionaryCompoundWord` and `hyphenationCompoundWord` factories over
//!   the compound test dictionaries and grammars with their parameters, and
//!   `CollationKeyAnalyzer` (scenario `collation:`) for 6 locales at every
//!   strength, over the harness's standard corpus plus the words of the
//!   dictionaries; every token attribute is compared.
//! * **Hunspell** ([`hunspell_api_oracle`]): `HunspellOracle.java` loads every
//!   bundled `.aff` (with its `.dic`, or the one Lucene's tests pair it with)
//!   and 16 synthetic dictionaries in the JDK's multi-byte charsets (Big5,
//!   HKSCS, EUC-JP, Shift_JIS, windows-31j, ISO-2022-JP, EUC-KR, MS949,
//!   GB2312, GBK, GB18030, EUC-TW, UTF-16, UTF-16LE, IBM942, IBM970), which it
//!   writes with the JDK's encoders; for each word of the dictionary, its
//!   `.good`/`.wrong`/`.sug` files and derived misspellings it prints
//!   `spell`, `Stemmer.stem` (with and without `ignoreCase`), `getRoots`,
//!   `analyzeSimpleWord`, suggestions, `getAllWordForms` and `compress`, or
//!   the load error. The words come from the Java dump; Rucene computes the
//!   rest.
//! * **Hyphenation** ([`hyphenation_oracle`]): `HyphenationOracle.java`
//!   records the `PatternConsumer` calls `PatternParser` makes through the
//!   JDK's SAX parser for the three bundled grammars and for synthetic ones
//!   (entities, character references, CDATA, comments, processing
//!   instructions, CRLF, ISO-8859-1, UTF-16, no DTD, malformed and
//!   external-entity documents), then the size of the `HyphenationTree` and
//!   the hyphenation points of a word corpus for four `(remain, push)` pairs,
//!   and `findPattern` of every pattern.
//! * **Collation** ([`collation_oracle`]): `CollationOracle.java` prints
//!   `Collator.getInstance(locale)` keys (`CollationKey.toByteArray()`) and
//!   comparison signs for 11 locales at every strength and decomposition over
//!   a seeded corpus, and the order of documents sorted by a
//!   `CollationDocValuesField` for 4 collators.
//! * **Morph** ([`morph_oracle`]): `analysis.morph` is abstract (its concrete
//!   dictionaries and tokenizers are the kuromoji and nori modules); both
//!   sides build the same minimal concrete world (`MorphOracle.java`
//!   documents it): the files the writers produce are compared byte for
//!   byte, then what the readers read back and the tokens a `Viterbi`
//!   subclass produces for a seeded corpus.

mod analysis_oracle;
mod support;

use std::collections::BTreeMap;
use std::fmt::Display;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use analysis_oracle::{
    corpus, escape_str, escape_units, java_errors, run_java_file, Chain, Oracle, Spec,
};
use randomizedtesting::jdk::random::{JdkRandom, Random};
use rucene::analysis::{Analyzer, CharReader, StringCharReader};
use rucene::document::StoredField;
use rucene::error::{LuceneError, Result};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::{directory_reader, IndexReader, IndexableField, IndexableFieldIterable};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::match_all_docs_query::MatchAllDocsQuery;
use rucene::search::{Sort, SortField, SortFieldType};
use rucene::store::java_nio_file::{IntoNioPath, Path as NioPath};
use rucene::store::{ByteBuffersDirectory, Directory};
use rucene::util::fst::{Builder as FSTCompilerBuilder, InputType, PositiveIntOutputs, FST};
use rucene::util::IntsRef;
use rucene_analysis_common::analysis::compound::hyphenation::hyphenation_tree::HyphenationTree;
use rucene_analysis_common::analysis::compound::hyphenation::pattern_consumer::{
    HyphenatedWordPart, PatternConsumer,
};
use rucene_analysis_common::analysis::compound::hyphenation::pattern_parser::PatternParser;
use rucene_analysis_common::analysis::hunspell::dictionary::{Dictionary, DictionarySubclass};
use rucene_analysis_common::analysis::hunspell::hunspell::{no_check_canceled, Hunspell};
use rucene_analysis_common::analysis::hunspell::sorting_strategy::SortingStrategy;
use rucene_analysis_common::analysis::hunspell::stemmer::Stemmer;
use rucene_analysis_common::analysis::hunspell::suggester::Suggester;
use rucene_analysis_common::analysis::hunspell::timeout_policy::TimeoutPolicy;
use rucene_analysis_common::analysis::morph::binary_dictionary::{
    BinaryDictionary, BinaryDictionarySubclass,
};
use rucene_analysis_common::analysis::morph::binary_dictionary_writer::BinaryDictionaryWriter;
use rucene_analysis_common::analysis::morph::character_definition::{
    CharacterDefinition, LookupCharacterClass,
};
use rucene_analysis_common::analysis::morph::character_definition_writer::CharacterDefinitionWriter;
use rucene_analysis_common::analysis::morph::connection_costs::ConnectionCosts;
use rucene_analysis_common::analysis::morph::connection_costs_writer::ConnectionCostsWriter;
use rucene_analysis_common::analysis::morph::dictionary::Dictionary as MorphDictionary;
use rucene_analysis_common::analysis::morph::dictionary_entry_writer::{
    DictionaryEntryWriter, DictionaryEntryWriterSubclass, DictionaryFileOutput,
};
use rucene_analysis_common::analysis::morph::morph_data::MorphData;
use rucene_analysis_common::analysis::morph::token::{Token, TokenSubclass};
use rucene_analysis_common::analysis::morph::token_info_fst::TokenInfoFST;
use rucene_analysis_common::analysis::morph::token_type::TokenType;
use rucene_analysis_common::analysis::morph::viterbi::{
    Position, Viterbi, ViterbiPosition, ViterbiSubclass,
};
use rucene_analysis_common::collation::collation_doc_values_field::CollationDocValuesField;
use rucene_analysis_common::collation::collation_key_analyzer::CollationKeyAnalyzer;
use rucene_analysis_common::jdk::collator::RuleBasedCollator;
use rucene_analysis_common::jdk::locale::Locale;
use rucene_analysis_common::jdk::sax::InputSource;
use rucene_analysis_common::jdk::string::to_rust_string;
use rucene_test_framework::util::java_exceptions::{java_class_name, java_throwable};

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// A new directory of its own under the temporary directory (tests run in
/// parallel, in one process or many).
fn temp_dir(name: &str) -> PathBuf {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "rucene-oracle-t5-{name}-{}-{}",
        std::process::id(),
        N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("create a temporary directory");
    dir
}

/// A Lucene 10.5.0 jar of the local Maven repository.
fn lucene_jar(artifact: &str) -> PathBuf {
    let home = std::env::var("HOME").expect("HOME");
    Path::new(&home).join(format!(
        ".m2/repository/org/apache/lucene/{artifact}/10.5.0/{artifact}-10.5.0.jar"
    ))
}

/// The fixture directory of the Java oracles.
fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene")
}

/// Runs a single-file Java oracle, panicking with its errors on failure.
fn java_single(source: &Path, args: &[&str]) -> String {
    let (ok, stdout, stderr) = run_java_file(source, args);
    assert!(ok, "{} failed:\n{}", source.display(), java_errors(&stderr));
    stdout
}

/// Compiles `source` with the JDK's compiler module (the JRE has no `javac`
/// launcher) onto the Lucene class path, then runs `main` with `args`: a
/// class of a Lucene package must share the jars' class loader to reach its
/// package-private members, which a source-launched class does not.
fn java_compiled(source: &Path, main: &str, args: &[&str]) -> String {
    let classes = temp_dir("classes");
    let class_path = std::env::join_paths([
        lucene_jar("lucene-core"),
        lucene_jar("lucene-analysis-common"),
    ])
    .expect("a class path");
    let out = Command::new("java")
        .args(["-m", "jdk.compiler/com.sun.tools.javac.Main", "-d"])
        .arg(&classes)
        .arg("-cp")
        .arg(&class_path)
        .arg(source)
        .output()
        .expect("a JDK is required to run the Lucene oracle");
    assert!(
        out.status.success(),
        "javac failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let run_path = std::env::join_paths([
        classes.clone(),
        lucene_jar("lucene-core"),
        lucene_jar("lucene-analysis-common"),
    ])
    .expect("a class path");
    let out = Command::new("java")
        .arg("-cp")
        .arg(run_path)
        .arg(main)
        .args(args)
        .output()
        .expect("a JDK is required to run the Lucene oracle");
    assert!(
        out.status.success(),
        "{main} failed:\n{}",
        java_errors(&String::from_utf8_lossy(&out.stderr))
    );
    String::from_utf8(out.stdout).expect("the oracle prints UTF-8")
}

/// Compares two dumps line by line; panics with the first differences.
fn compare(what: &str, java: &str, rust: &str) {
    if java == rust {
        return;
    }
    let j: Vec<&str> = java.lines().collect();
    let r: Vec<&str> = rust.lines().collect();
    let mut report = Vec::new();
    let mut context = "";
    for i in 0..j.len().max(r.len()) {
        let (a, b) = (j.get(i).copied(), r.get(i).copied());
        if let Some(a) = a {
            if !a.starts_with(' ')
                && !a.starts_with("h ")
                && (a != b.unwrap_or("") || report.is_empty())
            {
                context = a;
            }
        }
        if a != b {
            report.push(format!(
                "line {}: (after {context:?})\n  java: {a:?}\n  rust: {b:?}",
                i + 1
            ));
            if report.len() == 25 {
                break;
            }
        }
    }
    panic!(
        "{what}: the dumps differ ({} java lines, {} rust lines):\n{}",
        j.len(),
        r.len(),
        report.join("\n")
    );
}

/// The inverse of `escape`.
fn unescape(s: &str) -> Vec<u16> {
    let mut out = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' {
            out.push(u16::from_str_radix(&s[i + 2..i + 6], 16).expect("\\uXXXX"));
            i += 6;
        } else {
            let c = s[i..].chars().next().expect("a char");
            let mut buf = [0u16; 2];
            out.extend_from_slice(c.encode_utf16(&mut buf));
            i += c.len_utf8();
        }
    }
    out
}

/// The simple name of the Java exception `e` stands for.
fn simple_name(e: &LuceneError) -> String {
    let class = java_class_name(e);
    class
        .rsplit(['.', '$'])
        .next()
        .unwrap_or(&class)
        .to_string()
}

/// `HunspellOracle.list`: each item escaped, joined by `, `, in brackets.
fn list<T: Display>(items: &[T]) -> String {
    let items: Vec<String> = items.iter().map(|i| escape_str(&i.to_string())).collect();
    format!("[{}]", items.join(", "))
}

// ---------------------------------------------------------------------------
// Hunspell
// ---------------------------------------------------------------------------

/// The hunspell test resources.
fn hunspell_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/resources/org/apache/lucene/analysis/hunspell")
}

/// The compound test resources.
fn compound_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/resources/org/apache/lucene/analysis/compound")
}

/// Every bundled `.aff` with its `.dic` (or the one Lucene's tests pair it
/// with) and the load mode: `(name, aff, dic, mode)`.
fn hunspell_dicts() -> Vec<(String, PathBuf, PathBuf, &'static str)> {
    let dir = hunspell_dir();
    let mut affs: Vec<String> = std::fs::read_dir(&dir)
        .expect("the hunspell resources")
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.strip_suffix(".aff").map(str::to_string)
        })
        .collect();
    affs.sort();
    let mut out = Vec::new();
    for name in affs {
        let dic = match name.as_str() {
            "compressed-before-set" | "compressed-empty-alias" => "compressed",
            "optional-condition" => "condition",
            "twofold" => "morph",
            "forgivable-errors-long" | "forgivable-errors-num" => "single-word",
            n if n.starts_with("broken") || n == "timelimit" => "simple",
            n => n,
        };
        let mode = if name.starts_with("forgivable-errors") || name == "common-errors" {
            "tolerant"
        } else {
            "plain"
        };
        out.push((
            name.clone(),
            dir.join(format!("{name}.aff")),
            dir.join(format!("{dic}.dic")),
            mode,
        ));
    }
    out.push((
        "simple+mixedcase".to_string(),
        dir.join("simple.aff"),
        dir.join("mixedcase.dic"),
        "plain",
    ));
    out
}

/// The tolerances of `TestDictionary.loadForgivingDictionary`.
struct Tolerant;

impl DictionarySubclass for Tolerant {
    fn tolerate_affix_rule_count_mismatches(&self) -> bool {
        true
    }

    fn tolerate_rep_rule_count_mismatches(&self) -> bool {
        true
    }

    fn tolerate_duplicate_conversion_mappings(&self) -> bool {
        true
    }
}

/// `HunspellOracle.load`.
fn load_dictionary(aff: &Path, dic: &Path, mode: &str, ignore_case: bool) -> Result<Dictionary> {
    let mut a = std::fs::File::open(aff)?;
    let mut d = std::fs::File::open(dic)?;
    if mode == "tolerant" {
        Dictionary::with_subclass(
            &mut a,
            vec![&mut d],
            ignore_case,
            SortingStrategy::in_memory(),
            Arc::new(Tolerant),
        )
    } else {
        Dictionary::with_sorting_strategy(
            &mut a,
            vec![&mut d],
            ignore_case,
            SortingStrategy::in_memory(),
        )
    }
}

/// `HunspellOracle.safe`.
fn safe<T>(r: Result<T>, show: impl FnOnce(T) -> String) -> String {
    match r {
        Ok(v) => show(v),
        Err(e) => format!("!{}", simple_name(&e)),
    }
}

/// Rucene's lines for one `dict` record and the words the Java dump lists.
fn hunspell_rust_section(header: &str, words: &[(String, String)], out: &mut Vec<String>) {
    out.push(header.to_string());
    let p: Vec<&str> = header["dict ".len()..].split('\t').collect();
    let (aff, dic, mode) = (Path::new(p[1]), Path::new(p[2]), p[3]);
    let loaded = load_dictionary(aff, dic, mode, false)
        .and_then(|d| Ok((d, load_dictionary(aff, dic, mode, true)?)));
    let (dictionary, ignore_case) = match loaded {
        Ok(d) => d,
        Err(e) => {
            let message = java_throwable(&e).message().unwrap_or("null").to_string();
            out.push(format!(
                "load-error {} {}",
                simple_name(&e),
                escape_str(&message)
            ));
            return;
        }
    };
    let dictionary = Arc::new(dictionary);
    out.push(format!("charset {}", dictionary.decoder.charset().name()));
    let stemmer = Stemmer::new(Arc::clone(&dictionary));
    let stemmer_ic = Stemmer::new(Arc::new(ignore_case));
    let hunspell = Hunspell::with_policy(
        Arc::clone(&dictionary),
        TimeoutPolicy::NoTimeout,
        no_check_canceled(),
    );
    let suggester = Suggester::new(Arc::clone(&dictionary));
    for (flags, word) in words {
        let units: Vec<u16> = word.encode_utf16().collect();
        out.push(format!("w {flags} {}", escape_str(word)));
        out.push(format!(
            " spell {}",
            safe(hunspell.spell(word), |b| b.to_string())
        ));
        let stems = |s: &Stemmer| {
            safe(s.stem(&units, units.len()), |v| {
                list(&v.iter().map(ToString::to_string).collect::<Vec<_>>())
            })
        };
        out.push(format!(" stem {}", stems(&stemmer)));
        out.push(format!(" stemIC {}", stems(&stemmer_ic)));
        out.push(format!(
            " roots {}",
            safe(hunspell.get_roots(word), |v| list(&v))
        ));
        out.push(format!(
            " analyze {}",
            safe(hunspell.analyze_simple_word(word), |v| list(&v))
        ));
        if flags.contains('S') {
            out.push(format!(
                " sug {}",
                safe(
                    suggester.suggest_no_timeout(word, no_check_canceled()),
                    |v| list(&v)
                )
            ));
        }
        if flags.contains('F') {
            out.push(format!(
                " forms {}",
                safe(hunspell.get_all_word_forms(word), |v| list(&v))
            ));
            let compressed = hunspell.get_all_word_forms(word).and_then(|forms| {
                let words: Vec<String> = forms.iter().map(|f| f.get_word()).collect();
                let refs: Vec<&str> = words.iter().map(String::as_str).collect();
                hunspell.compress(&refs)
            });
            out.push(format!(
                " compress {}",
                safe(compressed, |s| match s {
                    Some(s) => escape_str(&s.internals_to_string()),
                    None => "null".to_string(),
                })
            ));
        }
    }
}

#[test]
fn hunspell_api_oracle() {
    let work = temp_dir("hunspell");
    let mut spec = format!("synthetic\t{}\n", work.join("synthetic").display());
    for (name, aff, dic, mode) in hunspell_dicts() {
        spec.push_str(&format!(
            "dict\t{name}\t{}\t{}\t{mode}\n",
            aff.display(),
            dic.display()
        ));
    }
    let spec_file = work.join("spec.txt");
    std::fs::write(&spec_file, spec).expect("write the spec");
    let java = java_compiled(
        &fixtures().join("analysis/hunspell/HunspellOracle.java"),
        "org.apache.lucene.analysis.hunspell.HunspellOracle",
        &[spec_file.to_str().expect("a UTF-8 path")],
    );

    // Rucene over the dictionaries and words of the Java dump
    let mut rust = Vec::new();
    let mut header: Option<String> = None;
    let mut words: Vec<(String, String)> = Vec::new();
    let mut dicts = 0;
    let mut checked = 0;
    for line in java.lines() {
        if line.starts_with("dict ") {
            if let Some(h) = header.take() {
                hunspell_rust_section(&h, &words, &mut rust);
            }
            header = Some(line.to_string());
            words.clear();
            dicts += 1;
        } else if let Some(rest) = line.strip_prefix("w ") {
            let (flags, word) = rest.split_once(' ').expect("w <flags> <word>");
            words.push((
                flags.to_string(),
                String::from_utf16(&unescape(word)).expect("no unpaired surrogate"),
            ));
            checked += 1;
        }
    }
    if let Some(h) = header.take() {
        hunspell_rust_section(&h, &words, &mut rust);
    }
    let mut rust = rust.join("\n");
    rust.push('\n');
    println!("hunspell oracle: {dicts} dictionaries, {checked} words");
    assert!(
        dicts > 100 && checked > 4000,
        "the oracle checked too little"
    );
    compare("hunspell", &java, &rust);
}

// ---------------------------------------------------------------------------
// Token streams: hunspellStem, the compound factories, CollationKeyAnalyzer
// ---------------------------------------------------------------------------

/// The bundled dictionaries the `hunspellStem` specs read (UTF-8 or
/// ISO-8859-1, loading without error).
const STEM_DICTIONARIES: &[&str] = &[
    "simple",
    "allcaps",
    "alternate-casing",
    "base",
    "base_utf",
    "casesensitive",
    "checksharps",
    "circumfix",
    "complexprefix",
    "compressed",
    "condition",
    "condition2",
    "conv",
    "dependencies",
    "double-escaped",
    "escaped",
    "flaglong",
    "flagnum",
    "fullstrip",
    "homonyms",
    "ignore",
    "keepcase",
    "morph",
    "morphalias",
    "morphdata",
    "needaffix",
    "onlyincompound",
    "spaces",
    "strange-overgeneration",
    "IJ",
];

/// Inputs from the words of `files` (decoded as UTF-8 with replacement),
/// ten per input.
fn word_inputs(dir: &Path, files: &[String]) -> Vec<String> {
    let mut words = Vec::new();
    for f in files {
        if let Ok(bytes) = std::fs::read(dir.join(f)) {
            for line in String::from_utf8_lossy(&bytes)
                .lines()
                .skip(usize::from(f.ends_with(".dic")))
            {
                let word = line.split(['/', '\t']).next().unwrap_or("").trim();
                if !word.is_empty() && !word.starts_with('#') {
                    words.push(word.to_string());
                }
            }
        }
    }
    words.chunks(10).map(|c| c.join(" ")).collect()
}

#[test]
fn token_stream_oracle() {
    let hdir = hunspell_dir();
    let cdir = compound_dir();
    let mut files = Vec::new();
    for d in STEM_DICTIONARIES {
        for ext in [".dic", ".good", ".wrong"] {
            files.push(format!("{d}{ext}"));
        }
    }
    let mut texts = word_inputs(&hdir, &files);
    texts.extend(word_inputs(
        &cdir,
        &[
            "compoundDictionary.txt".into(),
            "da_compoundDictionary.txt".into(),
            "compoundDictionary_lucene8183.txt".into(),
        ],
    ));
    texts.extend(
        [
            "min veninde som er lidt af en l\u{e6}sehest",
            "basketballkurv Basketballkurv BASKETBALLKURV",
            "Bild\u{f6}rr Bilmotor Biltak Slagborr Hammarborr Pelarborr Glas\u{f6}gonfodral Basfiolsfodral Basfiolsfodralmakareges\u{e4}ll Skomakare Vindrutetorkare Vindrutetorkarblad abba",
            "Rindfleisch\u{fc}berwachungsgesetz Rindfleisch fu\u{df}ballpumpe fu\u{df}ball schweinefleisch nahkampfwaffen",
            "softball softballs baseball footballer handball ballpark",
            "I like to play softball",
            "a ab abc abcd abcde abcdef abcdefg",
            "\u{130}ZM\u{130}R Istanbul \u{131}stanbul M\u{fc}\u{df}ig PROZESSIONSSTRASSE",
        ]
        .iter()
        .map(|s| s.to_string()),
    );
    let mut c = corpus::standard(0x5EED_7005, 120);
    c.extend(texts.iter().map(|s| s.encode_utf16().collect::<Vec<u16>>()));
    let mut oracle = Oracle::new(c);

    // hunspellStem
    for d in STEM_DICTIONARIES {
        let dic = format!("{d}.dic");
        let aff = format!("{d}.aff");
        oracle.spec(Spec::factories(
            &format!("hunspell-{d}"),
            Chain::new()
                .config_dir(&hdir)
                .tokenizer("whitespace", &[])
                .filter("hunspellStem", &[("dictionary", &dic), ("affix", &aff)]),
        ));
    }
    for (id, params) in [
        ("simple-ic", vec![("ignoreCase", "true")]),
        ("simple-longest", vec![("longestOnly", "true")]),
        (
            "simple-ic-longest",
            vec![("ignoreCase", "true"), ("longestOnly", "true")],
        ),
        (
            "simple-legacy",
            vec![("strictAffixParsing", "false"), ("recursionCap", "1")],
        ),
    ] {
        let mut p = vec![("dictionary", "simple.dic"), ("affix", "simple.aff")];
        p.extend(params);
        oracle.spec(Spec::factories(
            &format!("hunspell-{id}"),
            Chain::new()
                .config_dir(&hdir)
                .tokenizer("whitespace", &[])
                .filter("hunspellStem", &p),
        ));
    }
    for d in [
        "casesensitive",
        "keepcase",
        "allcaps",
        "checksharps",
        "alternate-casing",
    ] {
        let dic = format!("{d}.dic");
        let aff = format!("{d}.aff");
        oracle.spec(Spec::factories(
            &format!("hunspell-{d}-ic"),
            Chain::new()
                .config_dir(&hdir)
                .tokenizer("whitespace", &[])
                .filter(
                    "hunspellStem",
                    &[
                        ("dictionary", &dic),
                        ("affix", &aff),
                        ("ignoreCase", "true"),
                    ],
                ),
        ));
    }
    oracle.spec(Spec::factories(
        "hunspell-two-dictionaries",
        Chain::new()
            .config_dir(&hdir)
            .tokenizer("whitespace", &[])
            .filter(
                "hunspellStem",
                &[
                    ("dictionary", "simple.dic,mixedcase.dic"),
                    ("affix", "simple.aff"),
                ],
            ),
    ));
    oracle.spec(Spec::factories(
        "hunspell-standard-lowercase",
        Chain::new()
            .config_dir(&hdir)
            .tokenizer("standard", &[])
            .filter("lowercase", &[])
            .filter(
                "hunspellStem",
                &[("dictionary", "base.dic"), ("affix", "base.aff")],
            ),
    ));

    // dictionaryCompoundWord
    let dict_params: Vec<(&str, Vec<(&str, &str)>)> = vec![
        ("default", vec![]),
        (
            "short",
            vec![
                ("minWordSize", "3"),
                ("minSubwordSize", "1"),
                ("maxSubwordSize", "5"),
            ],
        ),
        (
            "all-matches",
            vec![
                ("onlyLongestMatch", "false"),
                ("onlyLongestMatchIgnoreSubwords", "false"),
            ],
        ),
        (
            "longest-keep-subwords",
            vec![
                ("onlyLongestMatch", "true"),
                ("onlyLongestMatchIgnoreSubwords", "false"),
            ],
        ),
        (
            "no-longest-ignore",
            vec![
                ("onlyLongestMatch", "false"),
                ("onlyLongestMatchIgnoreSubwords", "true"),
            ],
        ),
        ("long", vec![("minWordSize", "8"), ("maxSubwordSize", "40")]),
    ];
    for dict in [
        "compoundDictionary.txt",
        "da_compoundDictionary.txt",
        "compoundDictionary_lucene8183.txt",
    ] {
        for (pid, params) in &dict_params {
            let mut p = vec![("dictionary", dict)];
            p.extend(params.iter().copied());
            oracle.spec(Spec::factories(
                &format!("dcw-{dict}-{pid}"),
                Chain::new()
                    .config_dir(&cdir)
                    .tokenizer("whitespace", &[])
                    .filter("dictionaryCompoundWord", &p),
            ));
        }
    }

    // hyphenationCompoundWord
    let hyph_params: Vec<(&str, Vec<(&str, &str)>)> = vec![
        ("default", vec![]),
        (
            "small",
            vec![("minSubwordSize", "2"), ("maxSubwordSize", "4")],
        ),
        ("longest", vec![("onlyLongestMatch", "true")]),
        ("nosub", vec![("noSubMatches", "true")]),
        ("nooverlap", vec![("noOverlappingMatches", "true")]),
        (
            "all",
            vec![
                ("onlyLongestMatch", "true"),
                ("noSubMatches", "true"),
                ("noOverlappingMatches", "true"),
                ("minWordSize", "3"),
            ],
        ),
        (
            "encoding",
            vec![("encoding", "UTF-8"), ("maxSubwordSize", "10")],
        ),
    ];
    for hyph in [
        "da_UTF8.xml",
        "hyphenation-LUCENE-8124.xml",
        "hyphenation-LUCENE-8183.xml",
    ] {
        for dict in [
            None,
            Some("da_compoundDictionary.txt"),
            Some("compoundDictionary_lucene8183.txt"),
        ] {
            for (pid, params) in &hyph_params {
                if dict.is_none() && *pid != "small" && *pid != "default" {
                    continue;
                }
                let mut p = vec![("hyphenator", hyph)];
                if let Some(d) = dict {
                    p.push(("dictionary", d));
                }
                p.extend(params.iter().copied());
                oracle.spec(Spec::factories(
                    &format!("hcw-{hyph}-{}-{pid}", dict.unwrap_or("none")),
                    Chain::new()
                        .config_dir(&cdir)
                        .tokenizer("whitespace", &[])
                        .filter("hyphenationCompoundWord", &p),
                ));
            }
        }
    }

    // CollationKeyAnalyzer
    for tag in ["en", "de", "sv", "da", "ar", "ja"] {
        for strength in 0..4 {
            let decomposition = strength % 3;
            let tag_owned = tag.to_string();
            oracle.spec(Spec::scenario(
                &format!("collation-{tag}-{strength}-{decomposition}"),
                &format!("collation:{tag}:{strength}:{decomposition}"),
                move || {
                    let mut collator = RuleBasedCollator::get_instance(
                        &Locale::builder_from_language_tag(&tag_owned)?,
                    );
                    collator.set_strength(strength)?;
                    collator.set_decomposition(decomposition)?;
                    Ok(Box::new(CollationKeyAnalyzer::new(&collator)) as Box<dyn Analyzer>)
                },
            ));
        }
    }

    oracle.assert_matches();
}

// ---------------------------------------------------------------------------
// Hyphenation
// ---------------------------------------------------------------------------

/// The synthetic grammars: file name and bytes.
fn synthetic_grammars() -> Vec<(&'static str, Vec<u8>)> {
    let dtd = "<!DOCTYPE hyphenation-info SYSTEM \"hyphenation.dtd\">";
    let mut g: Vec<(&'static str, Vec<u8>)> = Vec::new();
    g.push((
        "entities.xml",
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<!DOCTYPE hyphenation-info SYSTEM \"hyphenation.dtd\" [\r\n  <!ENTITY ae \"\u{e6}\u{c6}\">\r\n  <!ENTITY oe \"&#248;&#xD8;\">\r\n]>\r\n<!-- a comment -->\r\n<?pi data?>\r\n<hyphenation-info>\r\n<hyphen-char value=\"-\"/>\r\n<hyphen-min before=\"2\" after=\"3\"/>\r\n<classes>\r\naA\r\nbB cC\tdD\r\neE fF gG hH iI jJ kK lL mM nN oO pP qQ rR sS tT uU vV wW xX yY zZ\r\n&ae; &oe; &#229;&#197;\r\n</classes>\r\n<exceptions>\r\nta-ble hy<hyphen pre=\"-\" no=\"\" post=\"\"/>phen <![CDATA[as-so-ciate]]> pro<hyphen/>ject\r\n</exceptions>\r\n<patterns>\r\n.a2b1 1ba 2bb1 b1c .ab3 1c2d 4e1 x1y <![CDATA[1yz2]]> \u{e6}1\u{f8} 2\u{e5}1\r\n<!-- inside -->\r\nst1r 1st2 ta2b1le\r\n</patterns>\r\n</hyphenation-info>\r\n"
        .to_string()
        .into_bytes(),
    ));
    let latin1 = format!(
        "<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?>\n{dtd}\n<hyphenation-info>\n<classes>aA bB cC eE lL rR sS tT uU \u{fc}\u{dc} \u{df}</classes>\n<patterns>\n\u{fc}1b 1\u{df}2 2ss1 a1ll .be2 r1st\n</patterns>\n</hyphenation-info>\n"
    );
    g.push((
        "latin1.xml",
        latin1.chars().map(|c| c as u32 as u8).collect(),
    ));
    let utf16 = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-16\"?>\n{dtd}\n<hyphenation-info>\n<classes>aA bB cC</classes>\n<patterns>a1b 2bc1 c3a</patterns>\n</hyphenation-info>\n"
    );
    let mut bytes = vec![0xFF, 0xFE];
    for u in utf16.encode_utf16() {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    g.push(("utf16.xml", bytes));
    g.push((
        "nodtd.xml",
        b"<hyphenation-info><classes>aA bB nN oO</classes><patterns>n1o 1bo 2ab</patterns><exceptions>no-bo-dy</exceptions></hyphenation-info>".to_vec(),
    ));
    g.push((
        "malformed.xml",
        format!("<?xml version=\"1.0\"?>\n{dtd}\n<hyphenation-info>\n<classes>aA</classes>\n<patterns>a1b").into_bytes(),
    ));
    g.push((
        "undeclared.xml",
        format!("<?xml version=\"1.0\"?>\n{dtd}\n<hyphenation-info>\n<classes>aA &foo;</classes>\n</hyphenation-info>\n").into_bytes(),
    ));
    g.push((
        "external.xml",
        b"<?xml version=\"1.0\"?>\n<!DOCTYPE hyphenation-info [\n  <!ENTITY xxe SYSTEM \"compoundDictionary.txt\">\n]>\n<hyphenation-info>\n<classes>aA</classes>\n<patterns>&xxe;</patterns>\n</hyphenation-info>\n".to_vec(),
    ));
    g.push((
        "whitespace.xml",
        format!("<?xml version=\"1.0\"?>\n{dtd}\n<hyphenation-info>\n<classes>   aA\n\n\n   bB   </classes>\n<patterns>\n\n  a1b\n\t\t2ba\n  </patterns>\n<exceptions>\n  a-b\tb-a\n</exceptions>\n</hyphenation-info>\n").into_bytes(),
    ));
    g
}

/// A `PatternConsumer` that logs its calls as `HyphenationOracle` does.
#[derive(Default)]
struct Logger {
    lines: Vec<String>,
    patterns: Vec<Vec<u16>>,
}

impl PatternConsumer for Logger {
    fn add_class(&mut self, chargroup: &[u16]) {
        self.lines
            .push(format!("class {}", escape_units(chargroup)));
    }

    fn add_exception(&mut self, word: &[u16], hyphenatedword: Vec<HyphenatedWordPart>) {
        let parts: Vec<String> = hyphenatedword
            .iter()
            .map(|p| match p {
                HyphenatedWordPart::String(s) => format!("s:{}", escape_units(s)),
                HyphenatedWordPart::Hyphen(h) => format!("h:{}", escape_str(&h.to_string())),
            })
            .collect();
        self.lines.push(format!(
            "exception {} [{}]",
            escape_units(word),
            parts.join(", ")
        ));
    }

    fn add_pattern(&mut self, pattern: &[u16], values: &[u16]) {
        self.patterns.push(pattern.to_vec());
        self.lines.push(format!(
            "pattern {} {}",
            escape_units(pattern),
            escape_units(values)
        ));
    }
}

/// `HyphenationOracle.PARAMS`.
const PARAMS: [(i32, i32); 4] = [(1, 1), (2, 2), (2, 3), (3, 1)];

#[test]
fn hyphenation_oracle() {
    let work = temp_dir("hyphenation");
    let mut xmls: Vec<PathBuf> = [
        "da_UTF8.xml",
        "hyphenation-LUCENE-8124.xml",
        "hyphenation-LUCENE-8183.xml",
    ]
    .iter()
    .map(|n| compound_dir().join(n))
    .collect();
    for (name, bytes) in synthetic_grammars() {
        let path = work.join(name);
        std::fs::write(&path, bytes).expect("write a grammar");
        xmls.push(path);
    }
    // the word corpus: the compound tests' words, the Danish dictionary,
    // seeded random words over the grammars' letters, case and edge cases
    let mut words: Vec<String> = [
        "l\u{e6}sehest",
        "basketballkurv",
        "Basketballkurv",
        "BASKETBALLKURV",
        "Rindfleisch",
        "fu\u{df}ballpumpe",
        "fu\u{df}ball",
        "table",
        "hyphen",
        "associate",
        "project",
        "nobody",
        "abcabc",
        "stable",
        "\u{e6}\u{f8}\u{e5}",
        "\u{fc}ber",
        "a",
        "ab",
        "",
        "a-b",
        "x1y2",
        "don't",
        "R\u{fc}be",
        "STRASSE",
        "Stra\u{df}e",
        "AbBa",
        "\u{c6}\u{d8}",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    if let Ok(text) = std::fs::read_to_string(compound_dir().join("da_compoundDictionary.txt")) {
        words.extend(
            text.lines()
                .filter(|l| !l.starts_with('#') && !l.is_empty())
                .map(str::to_string),
        );
    }
    let r = JdkRandom::new(0x7E5_4A1);
    let letters: Vec<char> =
        "abcdefghijklmnopqrstuvwxyz\u{e6}\u{f8}\u{e5}\u{fc}\u{df}ABCDEFGHIJKLMNOPQRSTUVWXYZ'-0"
            .chars()
            .collect();
    for _ in 0..400 {
        let len = 1 + r.next_int_bound(18);
        words.push(
            (0..len)
                .map(|_| letters[r.next_int_bound(letters.len() as i32) as usize])
                .collect(),
        );
    }

    let mut spec = String::new();
    for x in &xmls {
        spec.push_str(&format!("xml\t{}\n", x.display()));
    }
    for w in &words {
        spec.push_str(&format!("word\t{}\n", escape_str(w)));
    }
    let spec_file = work.join("spec.txt");
    std::fs::write(&spec_file, spec).expect("write the spec");
    let java = java_single(
        &fixtures().join("rucene/codec/HyphenationOracle.java"),
        &[spec_file.to_str().expect("a UTF-8 path")],
    );

    let mut out = Vec::new();
    for x in &xmls {
        let path = x.display().to_string();
        out.push(format!("xml {path}"));
        let mut logger = Logger::default();
        let parsed = {
            let mut parser = PatternParser::with_consumer(&mut logger);
            parser.parse_file_name(&path)
        };
        out.append(&mut logger.lines);
        if let Err(e) = parsed {
            out.push(format!("parse-error {}", simple_name(&e)));
        }
        let mut tree = HyphenationTree::new();
        if let Err(e) = tree.load_patterns(InputSource::from_system_id(path.clone())) {
            out.push(format!("tree-error {}", simple_name(&e)));
            continue;
        }
        out.push(format!("tree {}", tree.size()));
        for w in &words {
            let units: Vec<u16> = w.encode_utf16().collect();
            for (remain, push) in PARAMS {
                let points = match tree.hyphenate_word(&units, remain, push) {
                    None => "null".to_string(),
                    Some(h) => format!(
                        "[{}]",
                        h.get_hyphenation_points()
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                };
                out.push(format!("h {remain} {push} {} {points}", escape_str(w)));
            }
        }
        for p in &logger.patterns {
            out.push(format!(
                "find {} {}",
                escape_units(p),
                escape_units(&tree.find_pattern(p))
            ));
        }
    }
    let mut rust = out.join("\n");
    rust.push('\n');
    println!(
        "hyphenation oracle: {} grammars, {} words, {} lines",
        xmls.len(),
        words.len(),
        java.lines().count()
    );
    compare("hyphenation", &java, &rust);
}

// ---------------------------------------------------------------------------
// Collation
// ---------------------------------------------------------------------------

/// The documents of one sort: the same field objects for every document,
/// as `CollationOracle` reuses them.
struct ReusedDocument<'a>(Vec<&'a dyn IndexableField>);

impl IndexableFieldIterable for ReusedDocument<'_> {
    fn iterator(&self) -> Box<dyn Iterator<Item = Result<&dyn IndexableField>> + '_> {
        Box::new(self.0.iter().map(|f| Ok(*f)))
    }
}

fn collator(tag: &str, strength: i32, decomposition: i32) -> Result<RuleBasedCollator> {
    let mut c = RuleBasedCollator::get_instance(&Locale::builder_from_language_tag(tag)?);
    c.set_strength(strength)?;
    c.set_decomposition(decomposition)?;
    Ok(c)
}

fn sorted_docs(c: &RuleBasedCollator, inputs: &[String]) -> Result<Vec<String>> {
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    {
        let w = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new())?;
        let collation_field = CollationDocValuesField::new("collated", c)?;
        for input in inputs {
            let field = StoredField::new_string("field", input.clone())?;
            collation_field.set_string_value(input)?;
            w.add_document(&ReusedDocument(vec![&field, &collation_field]))?;
        }
        w.close()?;
    }
    let r = directory_reader::open(Arc::clone(&dir))?;
    let reader: Arc<dyn IndexReader> = r.clone();
    let s = IndexSearcher::new(Arc::clone(&reader))?;
    let td = s.search_with_sort(
        Arc::new(MatchAllDocsQuery::instance()),
        inputs.len() as i32,
        &Sort::new_fields(vec![SortField::new(
            Some("collated".to_string()),
            SortFieldType::String,
        )?])?,
    )?;
    let stored = reader.stored_fields()?;
    let mut out = Vec::new();
    for sd in &td.score_docs {
        let v = stored.document(sd.doc())?.get("field").unwrap_or_default();
        let ok = if v == inputs[sd.doc() as usize] {
            "ok"
        } else {
            "MISMATCH"
        };
        out.push(format!("d {} {ok}", sd.doc()));
    }
    drop(s);
    r.close()?;
    Ok(out)
}

#[test]
fn collation_oracle() {
    // the corpus: the harness's edge cases and seeded random strings, and
    // strings that reach the tailorings (accents, case, expansions,
    // contractions, Thai prevowels, ignorables, Hangul and Kana)
    let mut inputs: Vec<String> = corpus::edge_cases();
    inputs.extend(corpus::random(0xC0_11A7E, 160));
    inputs.extend(
        [
            "a",
            "A",
            "\u{e1}",
            "\u{c1}",
            "a\u{301}",
            "ae",
            "\u{e6}",
            "AE",
            "\u{c6}",
            "\u{e5}",
            "aa",
            "Aa",
            "\u{f6}",
            "o\u{308}",
            "\u{f8}",
            "oe",
            "\u{df}",
            "ss",
            "SS",
            "\u{1e9e}",
            "ch",
            "Ch",
            "cz",
            "\u{10d}",
            "ll",
            "\u{f1}",
            "n\u{303}",
            "\u{131}",
            "I",
            "\u{130}",
            "i",
            "\u{e0}\u{e8}",
            "\u{e40}\u{e01}",
            "\u{e01}\u{e40}",
            "\u{ac00}",
            "\u{1100}\u{1161}",
            "\u{304b}",
            "\u{30ab}",
            "\u{ff76}",
            "\u{304c}",
            "\u{f900}",
            "\u{623}",
            "\u{627}",
            "\u{622}",
            "x\u{ad}y",
            "xy",
            "x-y",
            "-",
            "\u{2010}",
            "1",
            "\u{661}",
            "\u{ff11}",
            "\u{2460}",
            "",
            " ",
            "\t",
        ]
        .iter()
        .map(|s| s.to_string()),
    );
    let tags = [
        "en",
        "de",
        "fr",
        "sv",
        "sv-u-co-trad",
        "da",
        "es",
        "tr",
        "ar",
        "ja",
        "th",
    ];
    let mut spec = String::new();
    let mut specs = Vec::new();
    for tag in tags {
        for strength in 0..4 {
            for decomposition in 0..3 {
                specs.push(("collator", tag, strength, decomposition));
            }
        }
    }
    for (tag, strength) in [("en", 2), ("de", 0), ("sv", 1), ("ja", 3)] {
        specs.push(("sort", tag, strength, 1));
    }
    for (kind, tag, s, d) in &specs {
        spec.push_str(&format!("{kind}\t{tag}\t{s}\t{d}\n"));
    }
    for i in &inputs {
        spec.push_str(&format!("input\t{}\n", escape_str(i)));
    }
    let work = temp_dir("collation");
    let spec_file = work.join("spec.txt");
    std::fs::write(&spec_file, spec).expect("write the spec");
    let java = java_single(
        &fixtures().join("rucene/codec/CollationOracle.java"),
        &[spec_file.to_str().expect("a UTF-8 path")],
    );

    let mut out = Vec::new();
    for (kind, tag, strength, decomposition) in &specs {
        out.push(format!("{kind} {tag} {strength} {decomposition}"));
        let c = collator(tag, *strength, *decomposition).expect("a collator");
        if *kind == "collator" {
            for i in 0..inputs.len() {
                let key: String = c
                    .get_collation_key(&inputs[i])
                    .to_byte_array()
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect();
                out.push(format!("k {key}"));
                if i + 1 < inputs.len() {
                    out.push(format!(
                        "c {}",
                        c.compare(&inputs[i], &inputs[i + 1]).signum()
                    ));
                }
            }
        } else {
            // `CollationOracle`'s documents: the inputs whose keys fit a doc value
            let docs: Vec<String> = inputs
                .iter()
                .filter(|s| s.encode_utf16().count() <= 1000)
                .cloned()
                .collect();
            out.extend(sorted_docs(&c, &docs).expect("the sorted documents"));
        }
    }
    let mut rust = out.join("\n");
    rust.push('\n');
    println!(
        "collation oracle: {} collators/sorts, {} inputs",
        specs.len(),
        inputs.len()
    );
    compare("collation", &java, &rust);
}

// ---------------------------------------------------------------------------
// Morph
// ---------------------------------------------------------------------------

/// `MorphOracle.FORWARD_SIZE`, `BACKWARD_SIZE` and `CLASSES`.
const FORWARD_SIZE: i32 = 4;
const BACKWARD_SIZE: i32 = 5;
const CLASSES: [&str; 4] = ["DEFAULT", "ALPHA", "NUMERIC", "SYMBOL"];

/// The binary names `MorphOracle`'s classes have (`Class.getName()`).
const CC_CLASS: &str = "org.apache.lucene.rucene.codec.MorphOracle$MiniConnectionCosts";
const CD_CLASS: &str = "org.apache.lucene.rucene.codec.MorphOracle$MiniCharacterDefinition";
const BD_CLASS: &str = "org.apache.lucene.rucene.codec.MorphOracle$MiniBinaryDictionary";

/// `MorphOracle.lookup`.
struct Lookup;

impl LookupCharacterClass for Lookup {
    fn lookup_character_class(&self, name: &str) -> i8 {
        CLASSES
            .iter()
            .position(|c| *c == name)
            .unwrap_or_else(|| panic!("unknown class {name}")) as i8
    }
}

/// `MorphOracle.MiniEntryWriter`.
struct MiniEntryWriter {
    base: DictionaryEntryWriter,
}

impl DictionaryEntryWriterSubclass for MiniEntryWriter {
    fn base(&self) -> &DictionaryEntryWriter {
        &self.base
    }

    fn base_mut(&mut self) -> &mut DictionaryEntryWriter {
        &mut self.base
    }

    fn put_entry(&mut self, entry: &[String]) -> Result<i32> {
        let start = self.base.buffer.position() as i32;
        for field in &entry[1..4] {
            let v: i16 = field.parse().expect("Short.parseShort");
            self.base.buffer.put_short(v)?;
        }
        self.base.pos_dict.push(Some(entry[4].clone()));
        Ok(start)
    }

    fn write_pos_dict_to(&mut self, out: &mut DictionaryFileOutput) -> Result<()> {
        use rucene::store::DataOutput;
        out.write_v_int(self.base.pos_dict.len() as i32)?;
        for s in &self.base.pos_dict {
            out.write_string(s.as_deref().unwrap_or(""))?;
        }
        Ok(())
    }
}

/// `MorphOracle.MiniBinaryDictionary` and its `MiniMorphData`.
struct MiniBinaryDictionary {
    base: BinaryDictionary,
}

fn open_supplier(path: PathBuf) -> impl FnMut() -> Result<Box<dyn Read + Send>> {
    move || Ok(Box::new(std::fs::File::open(&path)?) as Box<dyn Read + Send>)
}

fn class_file(dir: &Path, class: &str, suffix: &str) -> PathBuf {
    dir.join(format!("{}{suffix}", class.replace('.', "/")))
}

impl MiniBinaryDictionary {
    fn new(dir: &Path) -> Result<Self> {
        Ok(Self {
            base: BinaryDictionary::new(
                &mut open_supplier(class_file(
                    dir,
                    BD_CLASS,
                    BinaryDictionary::TARGETMAP_FILENAME_SUFFIX,
                )),
                &mut open_supplier(class_file(
                    dir,
                    BD_CLASS,
                    BinaryDictionary::DICT_FILENAME_SUFFIX,
                )),
                "mini_tm",
                "mini_dict",
                1,
            )?,
        })
    }

    fn short_at(&self, i: i32) -> i32 {
        i32::from(self.base.buffer.get_short_at(i).expect("buffer.getShort"))
    }
}

impl MorphData for MiniBinaryDictionary {
    fn get_left_id(&self, morph_id: i32) -> i32 {
        self.short_at(morph_id)
    }

    fn get_right_id(&self, morph_id: i32) -> i32 {
        self.short_at(morph_id + 2)
    }

    fn get_word_cost(&self, morph_id: i32) -> i32 {
        self.short_at(morph_id + 4)
    }
}

impl MorphDictionary<dyn MorphData> for MiniBinaryDictionary {
    fn get_morph_attributes(&self) -> &(dyn MorphData + 'static) {
        self
    }
}

impl BinaryDictionarySubclass<dyn MorphData> for MiniBinaryDictionary {
    fn binary_dictionary(&self) -> &BinaryDictionary {
        &self.base
    }
}

/// `MorphOracle.UnknownMorphData`.
struct UnknownMorphData;

impl MorphData for UnknownMorphData {
    fn get_left_id(&self, morph_id: i32) -> i32 {
        morph_id + 1
    }

    fn get_right_id(&self, morph_id: i32) -> i32 {
        morph_id
    }

    fn get_word_cost(&self, morph_id: i32) -> i32 {
        1000 + 100 * morph_id
    }
}

/// `MorphOracle.MiniToken`.
struct MiniToken {
    token: Token,
    word_id: i32,
}

impl TokenSubclass for MiniToken {
    fn token(&self) -> &Token {
        &self.token
    }

    fn token_mut(&mut self) -> &mut Token {
        &mut self.token
    }
}

/// `MorphOracle.MiniViterbi`.
struct MiniViterbi {
    viterbi: Viterbi<MiniToken, Position>,
    char_def: Arc<CharacterDefinition>,
}

impl ViterbiSubclass<MiniToken, Position> for MiniViterbi {
    fn viterbi(&self) -> &Viterbi<MiniToken, Position> {
        &self.viterbi
    }

    fn viterbi_mut(&mut self) -> &mut Viterbi<MiniToken, Position> {
        &mut self.viterbi
    }

    fn process_unknown_word(
        &mut self,
        reader: &mut dyn CharReader,
        any_matches: bool,
        pos_data: i32,
    ) -> Result<i32> {
        let pos = self.viterbi.pos;
        let first_character = self.viterbi.buffer.get(pos, reader)? as u16;
        if !any_matches || self.char_def.is_invoke(first_character) {
            let character_class = self.char_def.get_character_class(first_character);
            let mut unknown_word_length = 1;
            if self.char_def.is_group(first_character) {
                let mut pos_ahead = pos + 1;
                while unknown_word_length < 1024 {
                    let ch = self.viterbi.buffer.get(pos_ahead, reader)?;
                    if ch == -1 || character_class != self.char_def.get_character_class(ch as u16) {
                        break;
                    }
                    unknown_word_length += 1;
                    pos_ahead += 1;
                }
            }
            self.add(
                reader,
                &UnknownMorphData,
                pos_data,
                pos,
                pos + unknown_word_length,
                i32::from(character_class),
                TokenType::Unknown,
                false,
            )?;
            return Ok(unknown_word_length);
        }
        Ok(0)
    }

    fn backtrace(
        &mut self,
        _reader: &mut dyn CharReader,
        end_pos_data: i32,
        from_idx: i32,
    ) -> Result<()> {
        let v = &mut self.viterbi;
        let end_pos = end_pos_data;
        let mut pos = end_pos;
        let mut best_idx = from_idx;
        while pos > v.last_back_trace_pos {
            let (back_pos, back_word_pos, back_type, back_id, next_best_idx) = {
                let pos_data = v.positions.get(pos).position();
                (
                    pos_data.get_back_pos(best_idx),
                    pos_data.get_back_word_pos(best_idx),
                    pos_data.get_back_type(best_idx),
                    pos_data.get_back_id(best_idx),
                    pos_data.get_back_index(best_idx),
                )
            };
            let length = pos - back_word_pos;
            let fragment: Arc<[u16]> = v.buffer.get_range(back_word_pos, length as usize).into();
            v.pending.push(MiniToken {
                token: Token::new(
                    fragment,
                    0,
                    length,
                    back_word_pos,
                    back_word_pos + length,
                    back_type.expect("a token type"),
                ),
                word_id: back_id,
            });
            pos = back_pos;
            best_idx = next_best_idx;
        }
        v.last_back_trace_pos = end_pos;
        v.buffer.free_before(end_pos);
        v.positions.free_before(end_pos);
        Ok(())
    }
}

/// `MorphOracle`'s seeded corpus: entries sorted by surface, and inputs.
fn morph_corpus() -> (Vec<[String; 5]>, Vec<String>) {
    let r = JdkRandom::new(0x30_4F);
    let syllables = [
        "a",
        "b",
        "c",
        "ab",
        "ba",
        "abc",
        "ca",
        "x",
        "xy",
        "1",
        "12",
        "\u{65e5}",
        "\u{65e5}\u{672c}",
        "\u{672c}",
        "\u{30fb}",
    ];
    let mut surfaces: BTreeMap<Vec<u16>, ()> = BTreeMap::new();
    for s in syllables {
        surfaces.insert(s.encode_utf16().collect(), ());
    }
    for _ in 0..40 {
        let n = 1 + r.next_int_bound(3);
        let s: String = (0..n)
            .map(|_| syllables[r.next_int_bound(syllables.len() as i32) as usize])
            .collect();
        surfaces.insert(s.encode_utf16().collect(), ());
    }
    let mut entries = Vec::new();
    for s in surfaces.keys() {
        let homographs = 1 + r.next_int_bound(3);
        for _ in 0..homographs {
            entries.push([
                to_rust_string(s),
                r.next_int_bound(BACKWARD_SIZE).to_string(),
                r.next_int_bound(FORWARD_SIZE).to_string(),
                (r.next_int_bound(2000) - 500).to_string(),
                ["n", "v", "adj", "num", ""][r.next_int_bound(5) as usize].to_string(),
            ]);
        }
    }
    let alphabet = [
        "a", "b", "c", "x", "y", "z", "Q", "1", "2", "9", "!", "?", ".", " ", "\u{65e5}",
        "\u{672c}", "\u{30fb}", "\u{3042}", "\u{0}",
    ];
    let mut inputs: Vec<String> = [
        "",
        "a",
        "abc",
        "abcabc",
        "ab12x!?",
        "\u{65e5}\u{672c}\u{672c}",
        "zzz",
        "  ",
        "a b c",
        "QQQ999",
        "\u{3042}\u{3042}",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    for _ in 0..200 {
        let n = r.next_int_bound(24);
        inputs.push(
            (0..n)
                .map(|_| alphabet[r.next_int_bound(alphabet.len() as i32) as usize])
                .collect(),
        );
    }
    // a long input crossing the rolling buffer's growth
    inputs.push("abcxy12".repeat(400));
    (entries, inputs)
}

/// Every file under `dir`, by relative path.
fn files_of(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for e in std::fs::read_dir(dir).expect("read_dir").flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(base, &p, out);
            } else {
                out.insert(
                    p.strip_prefix(base)
                        .expect("under base")
                        .display()
                        .to_string(),
                    std::fs::read(&p).expect("read"),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

fn nio(p: &Path) -> NioPath {
    p.into_nio_path()
}

/// Rucene's half of `MorphOracle.main`.
fn morph_rust(entries: &[[String; 5]], inputs: &[String], dir: &Path) -> Result<Vec<String>> {
    let mut out = Vec::new();

    let mut ccw = ConnectionCostsWriter::new(CC_CLASS, FORWARD_SIZE, BACKWARD_SIZE)?;
    for f in 0..FORWARD_SIZE {
        for b in 0..BACKWARD_SIZE {
            if (f + b) % 5 != 3 {
                ccw.add(f, b, ((f * 7 + b * 13) % 41) * 37 - 600)?;
            }
        }
    }
    match ccw.write(&nio(dir), "mini_cc", 1) {
        Ok(()) => out.push("ccw written".to_string()),
        Err(e) => out.push(format!("ccw-error {}", simple_name(&e))),
    }
    std::fs::create_dir_all(class_file(dir, CC_CLASS, "").parent().expect("a parent"))?;
    ccw.write(&nio(dir), "mini_cc", 1)?;

    let mut cdw = CharacterDefinitionWriter::new(CD_CLASS, 0, CLASSES.len(), Box::new(Lookup));
    for c in 'a'..='z' {
        cdw.put_character_category(c as i32, "ALPHA");
    }
    for c in 'A'..='Z' {
        cdw.put_character_category(c as i32, "ALPHA EXTRA");
    }
    for c in '0'..='9' {
        cdw.put_character_category(c as i32, "NUMERIC");
    }
    for c in "!?.,;:-".chars() {
        cdw.put_character_category(c as i32, "SYMBOL");
    }
    cdw.put_character_category(0x30FB, "ALPHA");
    cdw.put_character_category(0x65E5, "ALPHA");
    cdw.put_invoke_definition("ALPHA", 1, 1, 0);
    cdw.put_invoke_definition("NUMERIC", 1, 1, 2);
    cdw.put_invoke_definition("SYMBOL", 0, 1, 0);
    cdw.put_invoke_definition("DEFAULT", 0, 0, 0);
    cdw.write(&nio(dir), "mini_cd", 1)?;

    let mut dw = BinaryDictionaryWriter::new(
        BD_CLASS,
        Box::new(MiniEntryWriter {
            base: DictionaryEntryWriter::new(1 << 12),
        }),
    );
    let mut compiler =
        FSTCompilerBuilder::new(InputType::Byte2, PositiveIntOutputs::get_singleton()).build()?;
    let mut last: Option<&str> = None;
    let mut source_id = -1;
    for e in entries {
        let word_id = dw.put(e)?;
        if last != Some(e[0].as_str()) {
            source_id += 1;
            let ints: Vec<i32> = e[0].encode_utf16().map(i32::from).collect();
            compiler.add(&IntsRef::new(ints), source_id as i64)?;
            last = Some(e[0].as_str());
        }
        dw.add_mapping(source_id, word_id)?;
        out.push(format!("put {} {word_id}", escape_str(&e[0])));
    }
    dw.write_files(&nio(dir), "mini_tm", "mini_pos", "mini_dict", 1)?;
    let metadata = compiler.compile()?;
    let fst = FST::from_fst_reader(metadata, compiler.into_fst_reader()?).expect("a non-empty FST");

    let dict = Arc::new(MiniBinaryDictionary::new(dir)?);
    for s in 0..=source_id {
        let mut r = IntsRef::default();
        dict.lookup_word_ids(s, &mut r);
        let mut line = format!("ids {s}:");
        for i in 0..r.length {
            let id = r.ints[r.offset + i];
            line.push_str(&format!(
                " {id}/{}/{}/{}",
                MorphDictionary::get_left_id(&*dict, id),
                MorphDictionary::get_right_id(&*dict, id),
                MorphDictionary::get_word_cost(&*dict, id)
            ));
        }
        out.push(line);
    }
    let costs = Arc::new(ConnectionCosts::new(
        &mut open_supplier(class_file(dir, CC_CLASS, ".dat")),
        "mini_cc",
        1,
    )?);
    for f in 0..FORWARD_SIZE {
        let mut line = format!("cost {f}:");
        for b in 0..BACKWARD_SIZE {
            line.push_str(&format!(" {}", costs.get(f, b)));
        }
        out.push(line);
    }
    let char_def = Arc::new(CharacterDefinition::new(
        &mut open_supplier(class_file(dir, CD_CLASS, ".dat")),
        "mini_cd",
        1,
        CLASSES.len(),
    )?);
    for c in "aZ09!?x\u{30fb}\u{65e5}\u{0}\u{ffff} ".encode_utf16() {
        out.push(format!(
            "char {} {} {} {}",
            escape_units(&[c]),
            char_def.get_character_class(c),
            char_def.is_invoke(c),
            char_def.is_group(c)
        ));
    }

    let token_fst = Arc::new(TokenInfoFST::new(Arc::new(fst), 0x7a, 0x61)?);
    let mut viterbi = MiniViterbi {
        viterbi: Viterbi::new(
            token_fst,
            dict as Arc<dyn BinaryDictionarySubclass<dyn MorphData>>,
            None,
            None,
            Arc::clone(&costs),
        ),
        char_def,
    };
    for input in inputs {
        out.push(format!("input {}", escape_str(input)));
        let mut reader: Box<dyn CharReader> = Box::new(StringCharReader::new(input));
        viterbi.viterbi.reset_buffer();
        viterbi.viterbi.reset_state();
        loop {
            if viterbi.viterbi.get_pending().is_empty() {
                if viterbi.viterbi.is_end() {
                    break;
                }
                viterbi.forward(reader.as_mut())?;
                continue;
            }
            let t = viterbi
                .viterbi
                .get_pending()
                .pop()
                .expect("a pending token");
            let tok = &t.token;
            let surface = &tok.get_surface_form()
                [tok.get_offset() as usize..(tok.get_offset() + tok.get_length()) as usize];
            out.push(format!(
                "tok {} {}-{} {} {} {} {}",
                escape_units(surface),
                tok.get_start_offset(),
                tok.get_end_offset(),
                tok.get_type(),
                t.word_id,
                tok.get_position_increment(),
                tok.get_position_length()
            ));
        }
    }
    Ok(out)
}

#[test]
fn morph_oracle() {
    let (entries, inputs) = morph_corpus();
    let work = temp_dir("morph");
    let mut spec = String::new();
    for e in &entries {
        spec.push_str(&format!(
            "entry\t{}\t{}\t{}\t{}\t{}\n",
            escape_str(&e[0]),
            e[1],
            e[2],
            e[3],
            e[4]
        ));
    }
    for i in &inputs {
        spec.push_str(&format!("input\t{}\n", escape_str(i)));
    }
    let spec_file = work.join("spec.txt");
    std::fs::write(&spec_file, spec).expect("write the spec");
    let java_dir = work.join("java");
    let rust_dir = work.join("rust");
    let java = java_single(
        &fixtures().join("rucene/codec/MorphOracle.java"),
        &[
            spec_file.to_str().expect("a UTF-8 path"),
            java_dir.to_str().expect("a UTF-8 path"),
        ],
    );
    let mut rust = morph_rust(&entries, &inputs, &rust_dir)
        .unwrap_or_else(|e| panic!("the Rust side failed: {e:?}"))
        .join("\n");
    rust.push('\n');

    // the files the writers wrote, byte for byte
    let java_files = files_of(&java_dir);
    let rust_files = files_of(&rust_dir);
    assert_eq!(
        java_files.keys().collect::<Vec<_>>(),
        rust_files.keys().collect::<Vec<_>>(),
        "the writers wrote different files"
    );
    assert_eq!(java_files.len(), 5, "five dictionary files");
    for (name, bytes) in &java_files {
        assert!(
            bytes == &rust_files[name],
            "{name} differs: java {} bytes, rust {} bytes",
            bytes.len(),
            rust_files[name].len()
        );
    }
    println!(
        "morph oracle: {} entries, {} inputs, {} files byte-identical, {} dump lines",
        entries.len(),
        inputs.len(),
        java_files.len(),
        java.lines().count()
    );
    compare("morph", &java, &rust);
}
