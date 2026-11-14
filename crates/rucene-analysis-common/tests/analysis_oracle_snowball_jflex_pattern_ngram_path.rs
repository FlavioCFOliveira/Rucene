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

//! Java-oracle portability tests of the Snowball stemmers and package
//! `analysis.snowball` (rmp #454, #455, #456), the JFlex packages
//! `analysis.charfilter` (#457), `analysis.classic` and `analysis.wikipedia`
//! (#464) and `analysis.email` (#465), `analysis.pattern` (#458), the
//! packages `ngram`, `shingle`, `commongrams` and `minhash` (#459), and
//! `path`, `reverse`, `boost`, `payloads`, `query`, `sinks` and `custom`
//! (#460): every analyzer, tokenizer, filter, char filter and factory of each
//! task, with parameterisations of the factories' documented parameters,
//! analyses the same corpus on Lucene 10.5.0 and Rucene through the harness
//! of #446 ([`analysis_oracle`]); every token attribute is compared (offsets
//! through char filters are the corrected ones the tokenizers report).
//!
//! **Corpora.** Each task's specs run over the harness's standard corpus
//! (fixed edge cases and 200 seeded random inputs) plus inputs that reach the
//! task's code: HTML, entities, CDATA, scripts and Lucene's two HTML test
//! files (#457); wiki markup and the classic grammar's hosts, e-mails,
//! acronyms and numbers (#464); every input of `WordBreakTestUnicode_12_1_0`
//! (the UAX#29 test data), every sequence of `EmojiTokenizationTestUnicode_12_1`,
//! Lucene's URL and e-mail test texts and `example.<tld>` for every TLD of
//! `TLDs.txt` (#465); and so on.
//!
//! **Stemmers.** Each of the 30 stemmers runs, through the `snowballPorter`
//! factory, over a vocabulary of its own: Lucene's test vocabulary of the
//! language where `lucene/analysis/common/src/test` has one (the English
//! Porter data, the German, Spanish, Finnish, French, Hungarian, Italian,
//! Portuguese, Russian and Swedish light-stemmer data, the Norwegian lists,
//! the Sorani data for Arabic), every word of every stop word list of the
//! module, and words built from the stemmer's own `Among` suffix tables
//! (each suffix after seeded stems of the language, and pairs of suffixes),
//! so that every rule of every stemmer is reached. One JVM launch per
//! stemmer: a launch runs every spec over every input, and the union of the
//! 30 vocabularies under every stemmer would be a dump of millions of lines.
//!
//! **Declared divergence (MinHash).** `MinHashFilter` writes arbitrary UTF-16
//! units into the term; an unpaired surrogate among them is U+FFFD in
//! Rucene's `char` term buffer (declared on `MinHashFilterLogic`, rmp #459).
//! The min-hash specs compare Lucene's dump with its unpaired surrogates
//! replaced by U+FFFD ([`fffd_lone_surrogates`]); every other attribute,
//! the term bytes included (Lucene's UTF-8 conversion already writes U+FFFD
//! for them), is compared as is.

mod analysis_oracle;
mod support;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use analysis_oracle::{corpus, Arg, Chain, Oracle, Spec};
use randomizedtesting::jdk::random::{JdkRandom, Random};
use rucene::analysis::{Analyzer, CharArraySet, TokenStream, Tokenizer, TokenizerLogic};
use rucene::document::{Document, Store, TextField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::IndexReader;
use rucene::store::{ByteBuffersDirectory, Directory};
use rucene::util::attribute::AttributeSource;
use rucene::util::extra::Version;
use rucene_analysis_common::analysis::charfilter::html_strip_char_filter_factory::HTMLStripCharFilterFactory;
use rucene_analysis_common::analysis::classic::classic_analyzer::ClassicAnalyzer;
use rucene_analysis_common::analysis::core::lower_case_filter::LowerCaseFilter;
use rucene_analysis_common::analysis::core::lower_case_filter_factory::LowerCaseFilterFactory;
use rucene_analysis_common::analysis::core::whitespace_analyzer::WhitespaceAnalyzer;
use rucene_analysis_common::analysis::core::whitespace_tokenizer::new_whitespace_tokenizer;
use rucene_analysis_common::analysis::custom::custom_analyzer::CustomAnalyzer;
use rucene_analysis_common::analysis::email::uax29_url_email_analyzer::UAX29URLEmailAnalyzer;
use rucene_analysis_common::analysis::miscellaneous::ascii_folding_filter_factory::ASCIIFoldingFilterFactory;
use rucene_analysis_common::analysis::query::query_auto_stop_word_analyzer::QueryAutoStopWordAnalyzer;
use rucene_analysis_common::analysis::reverse::reverse_string_filter_factory::ReverseStringFilterFactory;
use rucene_analysis_common::analysis::shingle::shingle_analyzer_wrapper::ShingleAnalyzerWrapper;
use rucene_analysis_common::analysis::sinks::tee_sink_token_filter::{
    SinkTokenStream, TeeSinkTokenFilter,
};
use rucene_test_framework::analysis::mock_analyzer::BoxedStream;
use rucene_test_framework::analysis::{components, shared, sink};
use rucene_test_framework::util::TestUtil;
use support::{own, FnAnalyzer};

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// UTF-16 inputs of `texts`.
fn inputs<S: AsRef<str>>(texts: &[S]) -> Vec<Vec<u16>> {
    texts
        .iter()
        .map(|s| s.as_ref().encode_utf16().collect())
        .collect()
}

/// The standard corpus of the harness (seed `seed`, 200 random inputs) and
/// `extra`.
fn task_corpus<S: AsRef<str>>(seed: i64, extra: &[S]) -> Vec<Vec<u16>> {
    let mut c = corpus::standard(seed, 200);
    c.extend(inputs(extra));
    c
}

/// A directory holding `files` (name, content), for the specs' resources.
fn config_dir(name: &str, files: &[(&str, &str)]) -> PathBuf {
    // per test and per call: tests run in parallel, in one process or many
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "rucene-oracle-t4-{}-{}-{}",
        name,
        std::process::id(),
        N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("create the config dir");
    for (file, content) in files {
        std::fs::write(dir.join(file), content).expect("write a config file");
    }
    dir
}

/// A factories spec: `tokenizer(tparams)` and `filters`, over the char
/// filters `char_filters`, resources from `dir`.
fn chain(
    id: &str,
    dir: &Path,
    char_filters: &[(&str, &[(&str, &str)])],
    tokenizer: (&str, &[(&str, &str)]),
    filters: &[(&str, &[(&str, &str)])],
) -> Spec {
    let mut c = Chain::new().config_dir(dir);
    for (name, params) in char_filters {
        c = c.char_filter(name, params);
    }
    c = c.tokenizer(tokenizer.0, tokenizer.1);
    for (name, params) in filters {
        c = c.filter(name, params);
    }
    Spec::factories(id, c)
}

/// Runs the oracle, checks that Lucene built every spec, and compares the
/// dumps after `java_transform`.
fn assert_oracle_with(o: &Oracle, java_transform: impl Fn(&str) -> String) {
    let java = java_transform(&o.java_dump());
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

fn assert_oracle(o: &Oracle) {
    assert_oracle_with(o, str::to_string);
}

/// Lucene's dump with every unpaired surrogate escape (`\uD800`-`\uDFFF`
/// not forming a pair) replaced by `�` (see the module docs).
fn fffd_lone_surrogates(dump: &str) -> String {
    fn unit_at(s: &[u8], i: usize) -> Option<u16> {
        if i + 6 <= s.len() && s[i] == b'\\' && s[i + 1] == b'u' {
            let hex = std::str::from_utf8(&s[i + 2..i + 6]).ok()?;
            u16::from_str_radix(hex, 16).ok()
        } else {
            None
        }
    }
    let b = dump.as_bytes();
    let mut out = String::with_capacity(dump.len());
    let mut i = 0;
    while i < b.len() {
        if let Some(u) = unit_at(b, i) {
            if (0xD800..0xDC00).contains(&u) {
                if let Some(low) = unit_at(b, i + 6) {
                    if (0xDC00..0xE000).contains(&low) {
                        out.push_str(&dump[i..i + 12]);
                        i += 12;
                        continue;
                    }
                }
                out.push_str("\\uFFFD");
                i += 6;
                continue;
            }
            if (0xDC00..0xE000).contains(&u) {
                out.push_str("\\uFFFD");
                i += 6;
                continue;
            }
            out.push_str(&dump[i..i + 6]);
            i += 6;
            continue;
        }
        let ch = dump[i..].chars().next().expect("a char");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Unescapes a Rust string literal body (`\u{..}`, `\n`, `\t`, `\r`, `\\`,
/// `\"`, `\'`, `\0`).
fn rust_unescape(lit: &str) -> String {
    let mut out = String::new();
    let mut chars = lit.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('u') => {
                assert_eq!(chars.next(), Some('{'));
                let hex: String = chars.by_ref().take_while(|&c| c != '}').collect();
                out.push(
                    char::from_u32(u32::from_str_radix(&hex, 16).expect("hex")).expect("a char"),
                );
            }
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('0') => out.push('\0'),
            Some(c) => out.push(c),
            None => panic!("a dangling escape"),
        }
    }
    out
}

/// The string literals that follow `marker` in `source` (Rust syntax).
fn literals_after(source: &str, marker: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = source;
    while let Some(i) = rest.find(marker) {
        rest = &rest[i + marker.len()..];
        let bytes = rest.as_bytes();
        let mut j = 0;
        let mut lit = String::new();
        while bytes[j] != b'"' {
            if bytes[j] == b'\\' {
                lit.push_str(&rest[j..j + 2]);
                j += 2;
            } else {
                let ch = rest[j..].chars().next().expect("a char");
                lit.push(ch);
                j += ch.len_utf8();
            }
        }
        out.push(rust_unescape(&lit));
        rest = &rest[j + 1..];
    }
    out
}

/// A test resource's bytes, read from `tests/resources`.
fn resource(path: &str) -> Vec<u8> {
    let full = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/resources")
        .join(path);
    std::fs::read(&full).unwrap_or_else(|e| panic!("read {}: {e}", full.display()))
}

fn resource_text(path: &str) -> String {
    String::from_utf8(resource(path)).expect("UTF-8")
}

// ---------------------------------------------------------------------------
// #454, #455, #456 Snowball
// ---------------------------------------------------------------------------

/// The Rust source of a stemmer (its `Among` tables are read from it).
fn stemmer_source(language: &str) -> &'static str {
    match language {
        "Arabic" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/tartarus/snowball/ext/arabic_stemmer.rs"
        )),
        "Armenian" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/tartarus/snowball/ext/armenian_stemmer.rs"
        )),
        "Basque" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/tartarus/snowball/ext/basque_stemmer.rs"
        )),
        "Catalan" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/tartarus/snowball/ext/catalan_stemmer.rs"
        )),
        "Danish" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/tartarus/snowball/ext/danish_stemmer.rs"
        )),
        "Dutch" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/tartarus/snowball/ext/dutch_stemmer.rs"
        )),
        "English" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/tartarus/snowball/ext/english_stemmer.rs"
        )),
        "Estonian" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/tartarus/snowball/ext/estonian_stemmer.rs"
        )),
        "Finnish" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/tartarus/snowball/ext/finnish_stemmer.rs"
        )),
        "French" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/tartarus/snowball/ext/french_stemmer.rs"
        )),
        "German" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/tartarus/snowball/ext/german_stemmer.rs"
        )),
        "Greek" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/tartarus/snowball/ext/greek_stemmer.rs"
        )),
        "Hindi" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/tartarus/snowball/ext/hindi_stemmer.rs"
        )),
        "Hungarian" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/tartarus/snowball/ext/hungarian_stemmer.rs"
        )),
        "Indonesian" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/tartarus/snowball/ext/indonesian_stemmer.rs"
        )),
        "Irish" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/tartarus/snowball/ext/irish_stemmer.rs"
        )),
        "Italian" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/tartarus/snowball/ext/italian_stemmer.rs"
        )),
        "Lithuanian" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/tartarus/snowball/ext/lithuanian_stemmer.rs"
        )),
        "Nepali" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/tartarus/snowball/ext/nepali_stemmer.rs"
        )),
        "Norwegian" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/tartarus/snowball/ext/norwegian_stemmer.rs"
        )),
        "Porter" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/tartarus/snowball/ext/porter_stemmer.rs"
        )),
        "Portuguese" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/tartarus/snowball/ext/portuguese_stemmer.rs"
        )),
        "Romanian" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/tartarus/snowball/ext/romanian_stemmer.rs"
        )),
        "Russian" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/tartarus/snowball/ext/russian_stemmer.rs"
        )),
        "Serbian" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/tartarus/snowball/ext/serbian_stemmer.rs"
        )),
        "Spanish" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/tartarus/snowball/ext/spanish_stemmer.rs"
        )),
        "Swedish" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/tartarus/snowball/ext/swedish_stemmer.rs"
        )),
        "Tamil" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/tartarus/snowball/ext/tamil_stemmer.rs"
        )),
        "Turkish" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/tartarus/snowball/ext/turkish_stemmer.rs"
        )),
        "Yiddish" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/tartarus/snowball/ext/yiddish_stemmer.rs"
        )),
        _ => panic!("no stemmer {language}"),
    }
}

/// Every suffix of the stemmer's `Among` tables.
fn among_suffixes(language: &str) -> BTreeSet<String> {
    let source = stemmer_source(language);
    let mut out = BTreeSet::new();
    for marker in ["Among::new(utf16!(\"", "Among::with_method(utf16!(\""] {
        out.extend(literals_after(source, marker));
    }
    out.retain(|s| !s.is_empty());
    out
}

/// The first tab-separated column of the non-comment lines of `text`.
fn first_column(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .filter_map(|l| l.split('\t').next())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// The first column of the files of a zip test resource.
fn zip_words(path: &str) -> Vec<String> {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "rucene-oracle-t4-zip-{}-{}-{}",
        path.replace('/', "_"),
        std::process::id(),
        N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("a temp dir");
    TestUtil::unzip(&resource(path), &dir).expect("unzip");
    let mut words = Vec::new();
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("the unzipped files")
        .map(|e| e.expect("an entry").path())
        .filter(|p| p.is_file())
        .collect();
    entries.sort();
    for p in entries {
        let bytes = std::fs::read(&p).expect("an unzipped file");
        words.extend(first_column(&String::from_utf8_lossy(&bytes)));
    }
    let _ = std::fs::remove_dir_all(&dir);
    words
}

/// Lucene's test vocabulary of `language` (`lucene/analysis/common/src/test`).
fn test_vocabulary(language: &str) -> Vec<String> {
    const A: &str = "org/apache/lucene/analysis/";
    let zips: &[&str] = match language {
        "English" | "Porter" => &["en/porterTestData.zip", "en/kstemTestData.zip"],
        "German" => &["de/delighttestdata.zip"],
        "Spanish" => &["es/eslighttestdata.zip"],
        "Finnish" => &["fi/filighttestdata.zip"],
        "French" => &["fr/frlighttestdata.zip"],
        "Hungarian" => &["hu/hulighttestdata.zip"],
        "Italian" => &["it/itlighttestdata.zip"],
        "Portuguese" => &["pt/ptlighttestdata.zip"],
        "Russian" => &["ru/rulighttestdata.zip"],
        "Swedish" => &["sv/svlighttestdata.zip"],
        "Arabic" => &["ckb/ckbtestdata.zip"],
        _ => &[],
    };
    let mut words: Vec<String> = zips
        .iter()
        .flat_map(|z| zip_words(&format!("{A}{z}")))
        .collect();
    if language == "Norwegian" {
        for f in [
            "nb_light.txt",
            "nn_light.txt",
            "nb_minimal.txt",
            "nn_minimal.txt",
        ] {
            words.extend(first_column(&resource_text(&format!("{A}no/{f}"))));
        }
    }
    // the porter data is "voc.txt" and "output.txt": the second is stems
    words
}

/// The words of a stop word resource (snowball format: `|` comments).
fn stop_words(bytes: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(|l| l.split('|').next().unwrap_or(""))
        .flat_map(|l| l.split_whitespace().map(str::to_string).collect::<Vec<_>>())
        .filter(|w| !w.starts_with('#'))
        .collect()
}

/// Every stop word list of the module, by resource name.
fn module_stop_words() -> Vec<(String, Vec<String>)> {
    let mut lists: Vec<(String, Vec<String>)> =
        rucene_analysis_common::resources::class_loader_resources()
            .into_iter()
            .filter(|(name, _)| name.ends_with("stopwords.txt") || name.ends_with("_stop.txt"))
            .map(|(name, bytes)| {
                let words = stop_words(&bytes);
                (name, words)
            })
            .collect();
    lists.sort();
    lists
}

/// The stop word resource of `language`, if the module has one.
fn language_stop_list(language: &str) -> Option<&'static str> {
    Some(match language {
        "Arabic" => "org/apache/lucene/analysis/ar/stopwords.txt",
        "Armenian" => "org/apache/lucene/analysis/hy/stopwords.txt",
        "Basque" => "org/apache/lucene/analysis/eu/stopwords.txt",
        "Catalan" => "org/apache/lucene/analysis/ca/stopwords.txt",
        "Danish" => "org/apache/lucene/analysis/snowball/danish_stop.txt",
        "Dutch" => "org/apache/lucene/analysis/snowball/dutch_stop.txt",
        "English" | "Porter" => "org/apache/lucene/analysis/snowball/english_stop.txt",
        "Estonian" => "org/apache/lucene/analysis/et/stopwords.txt",
        "Finnish" => "org/apache/lucene/analysis/snowball/finnish_stop.txt",
        "French" => "org/apache/lucene/analysis/snowball/french_stop.txt",
        "German" => "org/apache/lucene/analysis/snowball/german_stop.txt",
        "Greek" => "org/apache/lucene/analysis/el/stopwords.txt",
        "Hindi" => "org/apache/lucene/analysis/hi/stopwords.txt",
        "Hungarian" => "org/apache/lucene/analysis/snowball/hungarian_stop.txt",
        "Indonesian" => "org/apache/lucene/analysis/id/stopwords.txt",
        "Irish" => "org/apache/lucene/analysis/snowball/irish_stop.txt",
        "Italian" => "org/apache/lucene/analysis/snowball/italian_stop.txt",
        "Lithuanian" => "org/apache/lucene/analysis/lt/stopwords.txt",
        "Nepali" => "org/apache/lucene/analysis/ne/stopwords.txt",
        "Norwegian" => "org/apache/lucene/analysis/snowball/norwegian_stop.txt",
        "Portuguese" => "org/apache/lucene/analysis/snowball/portuguese_stop.txt",
        "Romanian" => "org/apache/lucene/analysis/ro/stopwords.txt",
        "Russian" => "org/apache/lucene/analysis/snowball/russian_stop.txt",
        "Serbian" => "org/apache/lucene/analysis/sr/stopwords.txt",
        "Spanish" => "org/apache/lucene/analysis/snowball/spanish_stop.txt",
        "Swedish" => "org/apache/lucene/analysis/snowball/swedish_stop.txt",
        "Tamil" => "org/apache/lucene/analysis/ta/stopwords.txt",
        "Turkish" => "org/apache/lucene/analysis/tr/stopwords.txt",
        _ => return None,
    })
}

/// The vocabulary a stemmer is checked on (see the module docs): Lucene's
/// test vocabulary of the language, every module stop word, and words built
/// from the stemmer's `Among` suffixes after seeded stems of the language
/// (Hebrew-script random stems for Yiddish, which has neither).
fn stemmer_vocabulary(language: &str) -> Vec<String> {
    let mut words: BTreeSet<String> = BTreeSet::new();
    let own = test_vocabulary(language);
    let lists = module_stop_words();
    let own_stops: Vec<String> = language_stop_list(language)
        .and_then(|name| lists.iter().find(|(n, _)| n == name))
        .map(|(_, w)| w.clone())
        .unwrap_or_default();
    words.extend(own.iter().cloned());
    for (_, list) in &lists {
        words.extend(list.iter().cloned());
    }
    let r = JdkRandom::new(0x454_0000 + language.len() as i64);
    let mut stems: Vec<String> = if !own.is_empty() { own } else { own_stops };
    if stems.is_empty() {
        // Yiddish: random Hebrew-script stems (U+05D0..U+05EA)
        stems = (0..500)
            .map(|_| {
                let n = 2 + r.next_int_bound(5);
                (0..n)
                    .map(|_| char::from_u32(0x05D0 + r.next_int_bound(27) as u32).expect("Hebrew"))
                    .collect()
            })
            .collect();
    }
    let suffixes: Vec<String> = among_suffixes(language).into_iter().collect();
    let pick = |r: &JdkRandom, v: &[String]| v[r.next_int_bound(v.len() as i32) as usize].clone();
    for suffix in &suffixes {
        words.insert(suffix.clone());
        for _ in 0..4 {
            let stem = pick(&r, &stems);
            // a stem's own ending may be a suffix: also strip up to 2 chars first
            let cut = r.next_int_bound(3) as usize;
            let mut base: Vec<char> = stem.chars().collect();
            let keep = base.len().saturating_sub(cut).max(1.min(base.len()));
            base.truncate(keep);
            words.insert(format!("{}{suffix}", base.into_iter().collect::<String>()));
        }
    }
    for _ in 0..(suffixes.len() * 2).min(3000) {
        let stem = pick(&r, &stems);
        let s1 = pick(&r, &suffixes);
        let s2 = pick(&r, &suffixes);
        words.insert(format!("{stem}{s1}{s2}"));
    }
    words.retain(|w| !w.chars().any(char::is_whitespace));
    words.into_iter().collect()
}

/// The words as inputs of 50 space-separated words each.
fn word_inputs(words: &[String]) -> Vec<Vec<u16>> {
    words
        .chunks(50)
        .map(|c| c.join(" ").encode_utf16().collect())
        .collect()
}

/// Every stemmer of `languages` over its vocabulary, one launch each.
fn stemmer_vocabulary_oracle(languages: &[&str]) {
    let dir = config_dir(&format!("snowball-{}", languages[0]), &[]);
    for language in languages {
        let words = stemmer_vocabulary(language);
        assert!(words.len() > 1000, "{language}: {} words", words.len());
        let mut o = Oracle::new(word_inputs(&words));
        o.spec(chain(
            &format!("stem-{language}"),
            &dir,
            &[],
            ("whitespace", &[]),
            &[("snowballPorter", &[("language", language)])],
        ));
        assert_oracle(&o);
        println!("{language}: {} words equal", words.len());
    }
}

const T454: [&str; 11] = [
    "Danish",
    "Dutch",
    "English",
    "French",
    "German",
    "Italian",
    "Norwegian",
    "Porter",
    "Portuguese",
    "Spanish",
    "Swedish",
];
const T455: [&str; 14] = [
    "Arabic",
    "Armenian",
    "Basque",
    "Catalan",
    "Estonian",
    "Finnish",
    "Hindi",
    "Hungarian",
    "Indonesian",
    "Irish",
    "Lithuanian",
    "Nepali",
    "Romanian",
    "Russian",
];
const T456: [&str; 5] = ["Greek", "Serbian", "Tamil", "Turkish", "Yiddish"];

#[test]
fn oracle_454_stemmer_vocabularies() {
    stemmer_vocabulary_oracle(&T454);
}

#[test]
fn oracle_455_stemmer_vocabularies() {
    stemmer_vocabulary_oracle(&T455);
}

#[test]
fn oracle_456_stemmer_vocabularies() {
    stemmer_vocabulary_oracle(&T456);
}

/// Inputs that reach the Snowball stemmers beyond the standard corpus.
const SNOWBALL_INPUTS: [&str; 6] = [
        "The fledgling banks were counting on a big boom in banking",
        "ridding of some stemming running runs runner",
        "vaikas vaikai vaiko vaik\u{173} brolis broliai",
        "\u{434}\u{43e}\u{43c}\u{430} \u{434}\u{43e}\u{43c}\u{430}\u{43c}\u{438} katter hundarna",
        "\u{3b1}\u{3bd}\u{3b8}\u{3c1}\u{3ce}\u{3c0}\u{3bf}\u{3c5}\u{3c2} \u{39a}\u{391}\u{39b}\u{39f}\u{3a3}",
        "kitaplar\u{131}m\u{131}z evlerimizden \u{5d0}\u{5d9}\u{5df}",
];

/// The `snowballPorter` factory (`SnowballFilter`) of every language, with
/// `protected` words and in chains (#454, #455, #456).
fn snowball_specs(o: &mut Oracle, dir: &Path) {
    let all: Vec<&str> = T454
        .iter()
        .chain(T455.iter())
        .chain(T456.iter())
        .copied()
        .collect();
    for language in &all {
        o.spec(chain(
            &format!("snowball-{language}"),
            dir,
            &[],
            ("standard", &[]),
            &[
                ("lowercase", &[]),
                ("snowballPorter", &[("language", language)]),
            ],
        ));
    }
    o.spec(chain(
        "snowball-protected",
        dir,
        &[],
        ("whitespace", &[]),
        &[(
            "snowballPorter",
            &[("language", "English"), ("protected", "protwords.txt")],
        )],
    ))
    .spec(chain(
        "snowball-keyword-marked",
        dir,
        &[],
        ("whitespace", &[]),
        &[
            ("keywordMarker", &[("protected", "protwords.txt")]),
            ("snowballPorter", &[("language", "Swedish")]),
        ],
    ))
    .spec(chain(
        "snowball-repeat-dedup",
        dir,
        &[],
        ("whitespace", &[]),
        &[
            ("keywordRepeat", &[]),
            ("snowballPorter", &[("language", "Russian")]),
            ("removeDuplicates", &[]),
        ],
    ))
    .spec(chain(
        "snowball-default-language",
        dir,
        &[],
        ("whitespace", &[]),
        &[("snowballPorter", &[])],
    ));
}

// ---------------------------------------------------------------------------
// #457 charfilter, #464 classic and wikipedia
// ---------------------------------------------------------------------------

/// Inputs that reach the HTML stripper, the mapping filter and the classic
/// and Wikipedia grammars.
fn jflex_inputs() -> Vec<String> {
    let mut v: Vec<String> = [
        "<div class=\"foo\">this is some text</div> here is a <a href=\"#bar\">link</a> and another <a href=\"http://lucene.apache.org/\">link</a>. This is an entity: &amp; plus a &lt;.  Here is an &. <!-- is a comment -->",
        "&nbsp; &lt;foo&gt; &Uuml;bermensch &#61; &Gamma; bar &#x393; &#8217; &QUOT;-&COPY;&GT;>&LT;<&REG;&AMP;",
        "aaa bbb <reserved ccc=\"ddddd\"> eeee </reserved> ffff <reserved ggg=\"hhhh\"/> <other/>",
        "one<script><!-- <!--#config comment=\"<!-- \\\"comment\\\"-->\"--> --></script>two",
        "one<style type=\"text/css\"> body,font,a { font-family:arial; } </style>two<BR />three<br>four",
        "one<![CDATA[<one><two>three<four></four></two></one>]]>two <![CDATA[<br>]]",
        "<! [CDATA[&]]> <!\u{2009}[CDATA[&]\u{2009}]\u{2009} <![endif]",
        "one<?xml:namespace prefix = o ns = \"urn:schemas-microsoft-com:office:office\" />two",
        "one<\u{29B05}\u{8271}>two<\u{701B}\u{612F}\u{29B05}>three</\u{701B}>four",
        " one two &#xD86C;&#XdC01;three &#55404;&#57999; &#Xd921; &#xdfdb<br> &#57209",
        "<a href=\"https://www.example.com/?test=\">example</a> this gets discarded <a href='next'>example2</a>",
        "X < &zz >X &# < X > < &l > &g < X hello &#x world",
        "<input type=\"text\" value=\"<search here>\"> <a href=\" http://x.org/ <a href=\" y",
        "h i j k ll cccc bbb aa empty \u{FF01} \u{1D122} aaaa ll h",
        "c:\\a\\b\\c G&uuml;nther G&uuml;nther is here",
        "AT&T Excite@Home www.nutch.org. 2004.jpg 62.46 978-0-94045043-1 mid-20th",
        "U.S.A. O'Reilly's you're C++ C# R2D2 216.239.63.104 test@example.com first_lastname@example.com",
        "78academyawards/rules/rule02.html 2006-03-11t082958z_01_ban130523_rtridst_0_ozabs test\u{2C6}test",
        "/money.cnn.com/magazines/fortune/fortune_archive/2007/03/19/8402357/index.htm safari-0-sheikh-zayed-grand-mosque.jpg",
        "[[link]] This is a [[Category:foo]] Category  This is a linked [[:Category:bar none withstanding]] Category",
        "This is an external URL [http://lucene.apache.org] Here is ''italics'' and ''more italics'', '''bold''' and '''''five quotes'''''",
        "This is a [[link|display info]]  ==heading== ===sub head=== followed by some text  [[Category:blah| ]] ''[[Category:ital_cat]]''",
        "[http://foo.boo.com/test/test/ Test Test] [http://foo.boo.com/test/test/test.html?g=b&c=d Test Test] <ref>Citation</ref> <sup>martian</sup>",
        "[[Category:a b c d]] [[Category:e f g]] [[link here]] [[link there]] ''italics here'' something ''more italics'' [[Category:h   i   j]]",
        "click [[link here again]] click [http://lucene.apache.org here again] [[Category:a b c d]]",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    // Lucene's two HTML test files
    v.push(resource_text(
        "org/apache/lucene/analysis/charfilter/htmlStripReaderTest.html",
    ));
    v.push(resource_text(
        "org/apache/lucene/analysis/charfilter/MS-Word 14 generated.htm",
    ));
    v
}

#[test]
fn oracle_457_464_charfilter_classic_wikipedia() {
    let dir = config_dir(
        "jflex",
        &[
            (
                "mapping.txt",
                "# test mapping\n\"aa\" => \"a\"\n\"bbb\" => \"b\"\n\"cccc\" => \"cc\"\n\"h\" => \"i\"\n\
                 \"j\" => \"jj\"\n\"k\" => \"kkk\"\n\"ll\" => \"llll\"\n\"empty\" => \"\"\n\
                 \"\\uFF01\" => \"!\"\n\"&uuml;\" => \"\\u00FC\"\n\"\\\\\" => \"/\"\n\"\\t\" => \" \"\n",
            ),
            ("mapping2.txt", "\"a\" => \"e\"\n\"b\" => \"f\"\n"),
            ("stopwords.txt", "is\nthe\na\nan\n"),
        ],
    );
    let mut o = Oracle::new(task_corpus(0x0457_0464, &jflex_inputs()));
    // #457
    for (id, cf) in [
        ("html", vec![("htmlStrip", vec![])]),
        (
            "html-escaped",
            vec![(
                "htmlStrip",
                vec![("escapedTags", "a, reserved, SCRIPT, style, BR")],
            )],
        ),
        (
            "html-escaped-sep",
            vec![("htmlStrip", vec![("escapedTags", ",, , ")])],
        ),
        (
            "mapping",
            vec![("mapping", vec![("mapping", "mapping.txt")])],
        ),
        (
            "mapping-two-files",
            vec![("mapping", vec![("mapping", "mapping.txt,mapping2.txt")])],
        ),
        (
            "mapping-chained",
            vec![
                ("mapping", vec![("mapping", "mapping.txt")]),
                ("mapping", vec![("mapping", "mapping.txt")]),
            ],
        ),
        (
            "html-mapping",
            vec![
                ("htmlStrip", vec![]),
                ("mapping", vec![("mapping", "mapping.txt")]),
            ],
        ),
        (
            "mapping-html",
            vec![
                ("mapping", vec![("mapping", "mapping2.txt")]),
                ("htmlStrip", vec![]),
            ],
        ),
    ] {
        let cfs: Vec<(&str, &[(&str, &str)])> =
            cf.iter().map(|(n, p)| (*n, p.as_slice())).collect();
        for tokenizer in ["whitespace", "standard", "keyword", "classic"] {
            o.spec(chain(
                &format!("{id}-{tokenizer}"),
                &dir,
                &cfs,
                (tokenizer, &[]),
                &[],
            ));
        }
    }
    o.spec(Spec::custom(
        "html-custom",
        Chain::new()
            .config_dir(&dir)
            .char_filter("htmlStrip", &[])
            .tokenizer("standard", &[])
            .filter("lowercase", &[]),
    ));
    // #464
    o.spec(Spec::analyzer(
        "classic-analyzer",
        "org.apache.lucene.analysis.classic.ClassicAnalyzer",
        || Ok(Box::new(ClassicAnalyzer::new()) as Box<dyn Analyzer>),
    ))
    .spec(
        Spec::analyzer(
            "classic-analyzer-stop",
            "org.apache.lucene.analysis.classic.ClassicAnalyzer",
            || {
                Ok(
                    Box::new(ClassicAnalyzer::with_stop_words(CharArraySet::from_iter(
                        ["is", "this", "a"],
                        false,
                    ))) as Box<dyn Analyzer>,
                )
            },
        )
        .arg(Arg::CharSet(vec!["is".into(), "this".into(), "a".into()])),
    )
    .spec(
        Spec::analyzer(
            "classic-analyzer-max5",
            "org.apache.lucene.analysis.classic.ClassicAnalyzer",
            || {
                let mut a = ClassicAnalyzer::new();
                a.set_max_token_length(5);
                Ok(Box::new(a) as Box<dyn Analyzer>)
            },
        )
        .call("setMaxTokenLength", Arg::Int(5)),
    );
    for (id, params) in [
        ("classic", vec![]),
        ("classic-max5", vec![("maxTokenLength", "5")]),
        ("classic-max1000", vec![("maxTokenLength", "1000")]),
    ] {
        o.spec(chain(
            &format!("{id}-tokenizer"),
            &dir,
            &[],
            ("classic", &params),
            &[],
        ))
        .spec(chain(
            &format!("{id}-filter"),
            &dir,
            &[],
            ("classic", &params),
            &[("classic", &[])],
        ));
    }
    o.spec(chain(
        "classic-filter-standard",
        &dir,
        &[],
        ("standard", &[]),
        &[("classic", &[])],
    ))
    .spec(chain(
        "classic-filter-whitespace",
        &dir,
        &[],
        ("whitespace", &[]),
        &[("classic", &[])],
    ));
    for (id, params) in [
        ("wikipedia", vec![]),
        ("wikipedia-tokens", vec![("tokenOutput", "0")]),
        (
            "wikipedia-untokenized",
            vec![("tokenOutput", "1"), ("untokenizedTypes", "c, i")],
        ),
        (
            "wikipedia-both",
            vec![
                ("tokenOutput", "2"),
                ("untokenizedTypes", "c, i, b, il, el"),
            ],
        ),
        ("wikipedia-both-none", vec![("tokenOutput", "2")]),
    ] {
        o.spec(chain(id, &dir, &[], ("wikipedia", &params), &[]));
    }
    o.spec(chain(
        "wikipedia-lower",
        &dir,
        &[],
        ("wikipedia", &[]),
        &[("lowercase", &[])],
    ))
    .spec(Spec::custom(
        "classic-custom",
        Chain::new()
            .config_dir(&dir)
            .char_filter("htmlStrip", &[])
            .tokenizer("classic", &[("maxTokenLength", "20")])
            .filter("classic", &[])
            .filter("lowercase", &[])
            .filter("stop", &[("words", "stopwords.txt")]),
    ));
    assert_oracle(&o);
}

// ---------------------------------------------------------------------------
// #465 email
// ---------------------------------------------------------------------------

/// Every input of `WordBreakTestUnicode_12_1_0` (the UAX#29 word break test
/// data), read from the test framework's port.
fn word_break_test_inputs() -> Vec<String> {
    let source = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../rucene-test-framework/src/analysis/standard/word_break_test_unicode_12_1_0.rs"
    ));
    literals_after(
        source,
        "BaseTokenStreamTestCase::assert_analyzes_to(analyzer, \"",
    )
}

/// Every emoji sequence of `EmojiTokenizationTestUnicode_12_1` (its `TESTS`
/// array: a line of `emoji-test.txt`, then the sequence), read from the test
/// framework's port.
fn emoji_test_sequences() -> Vec<String> {
    let source = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../rucene-test-framework/src/analysis/standard/emoji_tokenization_test_unicode_12_1.rs"
    ));
    let start = source.find("static TESTS").expect("the TESTS array");
    let lits = literals_after(&source[start..], "\"");
    lits.chunks(2)
        .filter(|c| c.len() == 2)
        .map(|c| c[1].clone())
        .collect()
}

#[test]
fn oracle_465_email() {
    let mut extra: Vec<String> = Vec::new();
    let wb = word_break_test_inputs();
    assert!(wb.len() > 1800, "{} word break inputs", wb.len());
    extra.extend(wb);
    let emoji = emoji_test_sequences();
    assert!(emoji.len() > 1000, "{} emoji sequences", emoji.len());
    // a few sequences per input
    extra.extend(emoji.chunks(4).map(|c| c.join(" ")));
    for f in [
        "LuceneResourcesWikiPage.html",
        "random.text.with.email.addresses.txt",
        "random.text.with.urls.txt",
        "email.addresses.from.random.text.with.email.addresses.txt",
        "urls.from.random.text.with.urls.txt",
    ] {
        extra.push(resource_text(&format!(
            "org/apache/lucene/analysis/email/{f}"
        )));
    }
    let tlds = resource_text("org/apache/lucene/analysis/email/TLDs.txt");
    let urls: Vec<String> = tlds
        .split('\n')
        .filter(|l| !l.trim().starts_with('#') && !l.trim().is_empty())
        .map(|tld| format!("example.{tld}"))
        .collect();
    extra.extend(urls.chunks(20).map(|c| c.join(" ")));
    extra.extend(
        [
            "mailto:test@example.org mailto:personA@example.com,personB@example.com?cc=personC@example.com&subject=Subjectivity",
            "http://johno.jsmf.net/knowhow/ngrams/index.php?table=en-dickens-word-2gram&paragraphs=50&length=200&no-ads=on",
            "dJ8ngFi@avz13m.CC kU-l6DS@[082.015.228.189] \"%U\u{12}@?\\B\"@Fl2d.md Bvd#@tupjv.sn",
            "example.com:8080#fragment example.com/path/here?query=something#fragment 1.2_1..2 A:B_A::B",
            "Wha\u{301}t's this thing do? \u{1F4A9} \u{1F4A9}\u{1F4A9} \u{1F469}\u{200D}\u{2764}\u{FE0F}\u{200D}\u{1F469} \u{1F468}\u{1F3FC}\u{200D}\u{2695}\u{FE0F} \u{1F1FA}\u{1F1F8}",
        ]
        .iter()
        .map(|s| s.to_string()),
    );
    let dir = config_dir("email", &[]);
    let mut o = Oracle::new(task_corpus(0x0465, &extra));
    o.spec(Spec::analyzer(
        "uax29-analyzer",
        "org.apache.lucene.analysis.email.UAX29URLEmailAnalyzer",
        || Ok(Box::new(UAX29URLEmailAnalyzer::new()) as Box<dyn Analyzer>),
    ))
    .spec(
        Spec::analyzer(
            "uax29-analyzer-stop",
            "org.apache.lucene.analysis.email.UAX29URLEmailAnalyzer",
            || {
                Ok(Box::new(UAX29URLEmailAnalyzer::with_stop_words(
                    CharArraySet::from_iter(["the", "example.com", "is"], false),
                )) as Box<dyn Analyzer>)
            },
        )
        .arg(Arg::CharSet(vec![
            "the".into(),
            "example.com".into(),
            "is".into(),
        ])),
    )
    .spec(
        Spec::analyzer(
            "uax29-analyzer-max5",
            "org.apache.lucene.analysis.email.UAX29URLEmailAnalyzer",
            || {
                let mut a = UAX29URLEmailAnalyzer::new();
                a.set_max_token_length(5);
                Ok(Box::new(a) as Box<dyn Analyzer>)
            },
        )
        .call("setMaxTokenLength", Arg::Int(5)),
    );
    for (id, params) in [
        ("uax29", vec![]),
        ("uax29-max5", vec![("maxTokenLength", "5")]),
        ("uax29-max1000", vec![("maxTokenLength", "1000")]),
    ] {
        o.spec(chain(id, &dir, &[], ("uax29UrlEmail", &params), &[]));
    }
    o.spec(chain(
        "uax29-lower",
        &dir,
        &[],
        ("uax29UrlEmail", &[]),
        &[("lowercase", &[])],
    ));
    assert_oracle(&o);
}

// ---------------------------------------------------------------------------
// #458 pattern, #459 ngram/shingle/commongrams, #460 path/reverse/boost/
// payloads/query/sinks/custom (and the Snowball factory specs)
// ---------------------------------------------------------------------------

/// Inputs that reach the pattern, n-gram, shingle, common-grams, path,
/// payload and boost code beyond the standard corpus.
const MISC_INPUTS: [&str; 20] = [
    "aabfooaabfooabfoob ab caaaaaaaaab foobarbaz foo bar baz",
    "aa bb cc --- aa bb aa. bb aa   bb cc  a bb - ccc . --- bb a . ccc ccc bb",
    "G\u{FC}nther G\u{FC}nther is here; one,two/three  four",
    "One 401(k) two three 401k forty-two 4-2 a1b2c3 abc123def",
    "aaa\\ bbb\\-ccc a\\b\\0\\-c\\\u{E9}\\ d",
    "please divide this sentence into shingles",
    "to be or not to be that is the question",
    "How the s a brown s cow d like A B thing? the dog dog the monster",
    "/usr/local/lib/x.so /a/b/c/ //  /  a/b/c c:\\a\\b\\c \\\\server\\share www.site.co.uk",
    "The quick|JJ red|JJ fox|NN jumped|VB over the lazy|JJ brown|JJ dogs|NN",
    "The quick|1.0 red|2.0 fox|3.5 jumped|0.5 over the lazy|5 brown|99.3 dogs|83.7 bad|x",
    "The quick|0.4 red|0.5 fox|0.2 jumped|0.1 over the lazy|0.8 brown|0.9 dogs|0.9",
    "test|0.1 the*0.1 quick*0.1 red|2 fox|",
    "abcde vwxyz mosfellsb\u{E6}r seventeen one two three four",
    "\u{29B05}\u{8271}\u{935F} \u{1D11E}\u{1D11F}x a\u{301}b\u{308}",
    "Do have a nice day \u{701B}\u{612F}\u{41F9}\u{935F}\u{8271}\u{29B05}",
    "woof woof woof woof woof woof woof woof woof puff",
    "click here please now and then again and again",
    "the quick brown fox jumped over the lazy boring dog boring vaguelyboring",
    "This is a test of the emergency broadcast system with more words",
];

/// The Rust delegate analyzer of a scenario: `whitespace` or `standard`.
fn delegate(name: &str) -> Arc<dyn Analyzer> {
    match name {
        "whitespace" => Arc::new(WhitespaceAnalyzer::new()),
        "standard" => Arc::new(rucene::analysis::standard::StandardAnalyzer::new()),
        _ => panic!("unknown delegate {name}"),
    }
}

/// The `qasw:<delegate>:<mode>:<value>[:<fields>]` scenario
/// (`AnalysisOracle.scenario`): the index of the corpus, then the analyzer.
fn qasw_scenario(name: &str, corpus: &[Vec<u16>]) -> Result<Box<dyn Analyzer>> {
    let p: Vec<&str> = name.split(':').collect();
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    {
        let w = IndexWriter::new(
            Arc::clone(&dir),
            IndexWriterConfig::with_analyzer(Some(delegate(p[1]))),
        )?;
        for text in corpus {
            let text = String::from_utf16(text).expect("valid UTF-16");
            let mut doc = Document::new();
            doc.add(Box::new(TextField::new("field", text.clone(), Store::NO)?));
            doc.add(Box::new(TextField::new("body", text, Store::NO)?));
            w.add_document(&doc)?;
        }
        w.close()?;
    }
    let reader: Arc<dyn IndexReader> = directory_reader::open(dir)?;
    let d = delegate(p[1]);
    let fields: Option<Vec<String>> = p.get(4).map(|f| f.split(',').map(str::to_string).collect());
    let a = match (p[2], &fields) {
        ("default", _) => QueryAutoStopWordAnalyzer::new(d, &reader)?,
        ("percent", None) => QueryAutoStopWordAnalyzer::with_max_percent_docs(
            d,
            &reader,
            p[3].parse().expect("a float"),
        )?,
        ("percent", Some(f)) => QueryAutoStopWordAnalyzer::with_fields_max_percent_docs(
            d,
            &reader,
            f,
            p[3].parse().expect("a float"),
        )?,
        ("docfreq", None) => {
            QueryAutoStopWordAnalyzer::with_max_doc_freq(d, &reader, p[3].parse().expect("an int"))?
        }
        ("docfreq", Some(f)) => QueryAutoStopWordAnalyzer::with_fields_max_doc_freq(
            d,
            &reader,
            f,
            p[3].parse().expect("an int"),
        )?,
        _ => panic!("unknown qasw mode {}", p[2]),
    };
    Ok(Box::new(a))
}

/// `AnalysisOracle.SinkAfterTee`: a tee's sink, after the tee is consumed.
#[derive(Debug)]
struct SinkAfterTee {
    tee: TeeSinkTokenFilter,
    sink: SinkTokenStream,
}

impl TokenStream for SinkAfterTee {
    fn increment_token(&mut self) -> Result<bool> {
        self.sink.increment_token()
    }

    fn reset(&mut self) -> Result<()> {
        self.tee.reset()?;
        self.tee.consume_all_tokens()?;
        self.sink.reset()
    }

    fn end(&mut self) -> Result<()> {
        self.sink.end()
    }

    fn close(&mut self) -> Result<()> {
        self.tee.close()?;
        self.sink.close()
    }

    fn attribute_source(&self) -> &AttributeSource {
        self.sink.attribute_source()
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        self.sink.attribute_source_mut()
    }
}

/// The `tee:<tokenizer>:<stream>` scenario over a tokenizer.
fn tee_components<L: TokenizerLogic + 'static>(
    tokenizer: Tokenizer<L>,
    stream: &str,
) -> Result<rucene::analysis::TokenStreamComponents> {
    let tokenizer = own(tokenizer);
    let tee = TeeSinkTokenFilter::new(shared(&tokenizer));
    Ok(match stream {
        "source" => components(tokenizer, sink(tee)),
        "sink" => {
            let s = tee.new_sink_token_stream();
            components(tokenizer, sink(SinkAfterTee { tee, sink: s }))
        }
        "sink-lowercase" => {
            let s = tee.new_sink_token_stream();
            let lower = LowerCaseFilter::new(Box::new(SinkAfterTee { tee, sink: s }))?;
            components(tokenizer, sink(BoxedStream(Box::new(lower))))
        }
        _ => panic!("unknown tee stream {stream}"),
    })
}

fn tee_scenario(name: &str) -> Result<Box<dyn Analyzer>> {
    let p: Vec<String> = name.split(':').map(str::to_string).collect();
    Ok(Box::new(FnAnalyzer::new(
        "org.apache.lucene.rucene.codec.AnalysisOracle$1",
        move |_| match p[1].as_str() {
            "whitespace" => tee_components(new_whitespace_tokenizer(), &p[2]),
            "standard" => {
                tee_components(rucene::analysis::standard::new_standard_tokenizer(), &p[2])
            }
            t => panic!("unknown tokenizer {t}"),
        },
    )))
}

/// The `custom:*` scenarios.
fn custom_scenario(name: &str) -> Result<Box<dyn Analyzer>> {
    let mut b = CustomAnalyzer::builder();
    match name {
        "custom:when-term" => {
            b.with_tokenizer("whitespace", &[])?
                .add_token_filter("lowercase", &[])?;
            let mut c = b.when_term(Arc::new(|t: &str| t.contains('o')));
            c.add_token_filter("uppercase", &[])?
                .add_token_filter_class(&ReverseStringFilterFactory::class(), &[])?;
            c.endwhen()?.add_token_filter("asciifolding", &[])?;
        }
        "custom:when-protected" => {
            b.with_tokenizer("standard", &[])?
                .add_token_filter("lowercase", &[])?;
            let mut c = b.when(
                "protectedterm",
                &[
                    "protected",
                    "org/apache/lucene/analysis/snowball/english_stop.txt",
                ],
            )?;
            c.add_token_filter("reversestring", &[])?;
            c.endwhen()?;
        }
        "custom:classes" => {
            #[allow(deprecated)]
            let v = Version::LUCENE_9_0_0;
            b.with_default_match_version(v)?
                .add_char_filter_class(&HTMLStripCharFilterFactory::class(), &[])?
                .with_tokenizer_class(
                    &rucene::analysis::standard::StandardTokenizerFactory::class(),
                    &[],
                )?
                .add_token_filter_class(
                    &ASCIIFoldingFilterFactory::class(),
                    &["preserveOriginal", "true"],
                )?
                .add_token_filter_class(&LowerCaseFilterFactory::class(), &[])?
                .with_position_increment_gap(100)?
                .with_offset_gap(1000)?;
        }
        _ => panic!("unknown custom scenario {name}"),
    }
    Ok(Box::new(b.build()?))
}

/// The pattern specs (#458).
fn pattern_specs(o: &mut Oracle, dir: &Path) {
    for (id, params) in [
        ("pattern-split", vec![("pattern", "[,;/\\s]+")]),
        (
            "pattern-group0",
            vec![("pattern", "[a-z]+"), ("group", "0")],
        ),
        (
            "pattern-group1",
            vec![("pattern", "'([^']+)'|([a-z]+)(\\d+)"), ("group", "1")],
        ),
        ("pattern-ws", vec![("pattern", "\\p{Space}")]),
    ] {
        o.spec(chain(id, dir, &[], ("pattern", &params), &[]));
    }
    for (id, params) in [
        ("simple-pattern", vec![("pattern", "[a-z]+")]),
        ("simple-pattern-foo", vec![("pattern", "(foo)+")]),
        ("simple-pattern-any", vec![("pattern", ".*")]),
    ] {
        o.spec(chain(id, dir, &[], ("simplePattern", &params), &[]));
    }
    for (id, params) in [
        ("simple-split", vec![("pattern", "[ \t\r\n]+")]),
        ("simple-split-a", vec![("pattern", "a*")]),
        ("simple-split-sep", vec![("pattern", "[,;/ ]")]),
    ] {
        o.spec(chain(id, dir, &[], ("simplePatternSplit", &params), &[]));
    }
    for (id, params) in [
        (
            "replace-all",
            vec![("pattern", "a*b"), ("replacement", "-")],
        ),
        (
            "replace-first",
            vec![
                ("pattern", "a*b"),
                ("replacement", "-"),
                ("replace", "first"),
            ],
        ),
        ("replace-strip", vec![("pattern", "[aeiou]")]),
        (
            "replace-backref",
            vec![("pattern", "(a*)b"), ("replacement", "$1\\$")],
        ),
    ] {
        o.spec(chain(
            id,
            dir,
            &[],
            ("whitespace", &[]),
            &[("patternReplace", &params)],
        ));
    }
    for (id, params) in [
        ("capture", vec![("pattern", "((..)(..))")]),
        (
            "capture-preserve",
            vec![("pattern", "(\\w+?)(\\d+)"), ("preserve_original", "true")],
        ),
        (
            "capture-nopreserve",
            vec![
                ("pattern", "([a-z]+)|(\\d+)"),
                ("preserve_original", "false"),
            ],
        ),
    ] {
        o.spec(chain(
            id,
            dir,
            &[],
            ("whitespace", &[]),
            &[("patternCaptureGroup", &params)],
        ));
    }
    o.spec(chain(
        "typing",
        dir,
        &[],
        ("whitespace", &[]),
        &[("patternTyping", &[("patternFile", "typing.txt")])],
    ));
    for (id, params) in [
        (
            "replace-cf",
            vec![
                ("pattern", "(aa)\\s+(bb)\\s+(cc)"),
                ("replacement", "$1#$2#$3"),
            ],
        ),
        (
            "replace-cf-longer",
            vec![("pattern", "a"), ("replacement", "aa")],
        ),
        ("replace-cf-empty", vec![("pattern", "(aa)\\s+(bb)")]),
        (
            "replace-cf-unescape",
            vec![("pattern", "\\\\(.)"), ("replacement", "$1")],
        ),
    ] {
        o.spec(chain(
            id,
            dir,
            &[("patternReplace", &params)],
            ("whitespace", &[]),
            &[],
        ));
    }
}

/// The n-gram, shingle and common-grams specs (#459).
fn ngram_shingle_specs(o: &mut Oracle, dir: &Path) {
    for (id, params) in [
        ("ngram-tok", vec![]),
        (
            "ngram-tok-2-3",
            vec![("minGramSize", "2"), ("maxGramSize", "3")],
        ),
        (
            "ngram-tok-1-1",
            vec![("minGramSize", "1"), ("maxGramSize", "1")],
        ),
    ] {
        o.spec(chain(id, dir, &[], ("nGram", &params), &[]));
    }
    for (id, params) in [
        ("edge-tok", vec![]),
        (
            "edge-tok-1-3",
            vec![("minGramSize", "1"), ("maxGramSize", "3")],
        ),
    ] {
        o.spec(chain(id, dir, &[], ("edgeNGram", &params), &[]));
    }
    for (filter, id, params) in [
        (
            "nGram",
            "ngram-f-1-2",
            vec![("minGramSize", "1"), ("maxGramSize", "2")],
        ),
        (
            "nGram",
            "ngram-f-2-3-keep",
            vec![
                ("minGramSize", "2"),
                ("maxGramSize", "3"),
                ("preserveOriginal", "true"),
            ],
        ),
        (
            "edgeNGram",
            "edge-f-1-3",
            vec![("minGramSize", "1"), ("maxGramSize", "3")],
        ),
        (
            "edgeNGram",
            "edge-f-2-3-keep",
            vec![
                ("minGramSize", "2"),
                ("maxGramSize", "3"),
                ("preserveOriginal", "true"),
            ],
        ),
    ] {
        o.spec(chain(
            id,
            dir,
            &[],
            ("whitespace", &[]),
            &[(filter, &params)],
        ))
        .spec(chain(
            &format!("{id}-folded"),
            dir,
            &[],
            ("standard", &[]),
            &[("asciiFolding", &[]), (filter, &params)],
        ));
    }
    for (id, params) in [
        ("shingle", vec![]),
        ("shingle-nouni", vec![("outputUnigrams", "false")]),
        (
            "shingle-3-4",
            vec![("minShingleSize", "3"), ("maxShingleSize", "4")],
        ),
        (
            "shingle-sep",
            vec![("tokenSeparator", "=BLAH="), ("outputUnigrams", "false")],
        ),
        ("shingle-empty-sep", vec![("tokenSeparator", "")]),
        (
            "shingle-ifno",
            vec![
                ("minShingleSize", "3"),
                ("maxShingleSize", "3"),
                ("outputUnigrams", "false"),
                ("outputUnigramsIfNoShingles", "true"),
            ],
        ),
        (
            "shingle-filler",
            vec![("fillerToken", "--"), ("maxShingleSize", "3")],
        ),
    ] {
        o.spec(chain(
            id,
            dir,
            &[],
            ("whitespace", &[]),
            &[("shingle", &params)],
        ))
        .spec(chain(
            &format!("{id}-stop"),
            dir,
            &[],
            ("standard", &[]),
            &[
                ("stop", &[("words", "stopwords.txt")]),
                ("shingle", &params),
            ],
        ));
    }
    for (id, params) in [
        ("fixed-2", vec![("shingleSize", "2")]),
        (
            "fixed-3-sep",
            vec![("shingleSize", "3"), ("tokenSeparator", "<SEP>")],
        ),
        (
            "fixed-4-filler",
            vec![("shingleSize", "4"), ("fillerToken", "*")],
        ),
    ] {
        o.spec(chain(
            id,
            dir,
            &[],
            ("whitespace", &[]),
            &[("fixedShingle", &params)],
        ))
        .spec(chain(
            &format!("{id}-stop"),
            dir,
            &[],
            ("standard", &[]),
            &[
                ("stop", &[("words", "stopwords.txt")]),
                ("fixedShingle", &params),
            ],
        ));
    }
    for (id, params) in [
        ("common-default", vec![]),
        ("common-words", vec![("words", "common.txt")]),
        (
            "common-words-ic",
            vec![("words", "common.txt"), ("ignoreCase", "true")],
        ),
    ] {
        o.spec(chain(
            id,
            dir,
            &[],
            ("whitespace", &[]),
            &[("commonGrams", &params)],
        ))
        .spec(chain(
            &format!("{id}-query"),
            dir,
            &[],
            ("whitespace", &[]),
            &[("commonGramsQuery", &params)],
        ));
    }
    o.spec(
        Spec::analyzer(
            "shingle-wrapper",
            "org.apache.lucene.analysis.shingle.ShingleAnalyzerWrapper",
            || {
                Ok(Box::new(ShingleAnalyzerWrapper::new(Arc::new(
                    WhitespaceAnalyzer::new(),
                ))?) as Box<dyn Analyzer>)
            },
        )
        .arg(Arg::Analyzer(
            "org.apache.lucene.analysis.core.WhitespaceAnalyzer".into(),
        )),
    )
    .spec(
        Spec::analyzer(
            "shingle-wrapper-3",
            "org.apache.lucene.analysis.shingle.ShingleAnalyzerWrapper",
            || {
                Ok(Box::new(ShingleAnalyzerWrapper::with_max_shingle_size(
                    Arc::new(WhitespaceAnalyzer::new()),
                    3,
                )?) as Box<dyn Analyzer>)
            },
        )
        .arg(Arg::Analyzer(
            "org.apache.lucene.analysis.core.WhitespaceAnalyzer".into(),
        ))
        .arg(Arg::Int(3)),
    )
    .spec(
        Spec::analyzer(
            "shingle-wrapper-3-4",
            "org.apache.lucene.analysis.shingle.ShingleAnalyzerWrapper",
            || {
                Ok(Box::new(ShingleAnalyzerWrapper::with_shingle_sizes(
                    Arc::new(rucene::analysis::standard::StandardAnalyzer::new()),
                    3,
                    4,
                )?) as Box<dyn Analyzer>)
            },
        )
        .arg(Arg::Analyzer(
            "org.apache.lucene.analysis.standard.StandardAnalyzer".into(),
        ))
        .arg(Arg::Int(3))
        .arg(Arg::Int(4)),
    );
}

/// `BoostAttribute`'s reflected property, as `AnalysisOracle.attributes`
/// prints it (`String.valueOf(float)`).
fn boost_extra(src: &AttributeSource) -> Vec<String> {
    use rucene::search::BoostAttribute as _;
    let boost = src
        .get_attribute::<rucene::search::BoostAttributeImpl>()
        .expect("the BoostAttribute")
        .get_boost();
    vec![format!(
        "BoostAttribute#boost={}",
        rucene::util::java_fmt::float_to_string(boost)
    )]
}

/// The path, reverse, boost, payload, query, sink and custom specs (#460).
fn path_payload_specs(o: &mut Oracle, dir: &Path, corpus: &[Vec<u16>]) {
    o.extra_attribute(
        std::any::TypeId::of::<dyn rucene::search::BoostAttribute>(),
        "BoostAttribute",
        Some(boost_extra),
    );
    for (id, params) in [
        ("path", vec![]),
        ("path-replace", vec![("delimiter", "/"), ("replace", "\\")]),
        ("path-win", vec![("delimiter", "\\"), ("replace", "/")]),
        ("path-skip", vec![("skip", "1")]),
        ("path-reverse", vec![("reverse", "true")]),
        (
            "path-reverse-dot",
            vec![("reverse", "true"), ("delimiter", "."), ("skip", "1")],
        ),
    ] {
        o.spec(chain(id, dir, &[], ("pathHierarchy", &params), &[]));
    }
    o.spec(chain(
        "reverse",
        dir,
        &[],
        ("whitespace", &[]),
        &[("reverseString", &[])],
    ));
    for (id, params) in [
        ("payload-float", vec![("encoder", "float")]),
        ("payload-int", vec![("encoder", "integer")]),
        ("payload-identity", vec![("encoder", "identity")]),
        (
            "payload-float-star",
            vec![("encoder", "float"), ("delimiter", "*")],
        ),
        (
            "payload-class",
            vec![(
                "encoder",
                "org.apache.lucene.analysis.payloads.FloatEncoder",
            )],
        ),
    ] {
        o.spec(chain(
            id,
            dir,
            &[],
            ("whitespace", &[]),
            &[("delimitedPayload", &params)],
        ));
    }
    o.spec(chain(
        "payload-ngram",
        dir,
        &[],
        ("whitespace", &[]),
        &[
            ("delimitedPayload", &[("encoder", "float")]),
            ("nGram", &[("minGramSize", "1"), ("maxGramSize", "2")]),
        ],
    ))
    .spec(chain(
        "numeric-payload",
        dir,
        &[],
        ("standard", &[]),
        &[(
            "numericPayload",
            &[("payload", "0.75"), ("typeMatch", "<NUM>")],
        )],
    ))
    .spec(chain(
        "offset-payload",
        dir,
        &[],
        ("whitespace", &[]),
        &[("tokenOffsetPayload", &[])],
    ))
    .spec(chain(
        "type-payload",
        dir,
        &[],
        ("standard", &[]),
        &[("typeAsPayload", &[])],
    ))
    .spec(chain(
        "boost",
        dir,
        &[],
        ("whitespace", &[]),
        &[("delimitedBoost", &[])],
    ))
    .spec(chain(
        "boost-star",
        dir,
        &[],
        ("whitespace", &[]),
        &[("delimitedBoost", &[("delimiter", "*")])],
    ));
    for name in [
        "qasw:whitespace:default",
        "qasw:whitespace:percent:0.1",
        "qasw:whitespace:docfreq:2",
        "qasw:whitespace:percent:0.05:field",
        "qasw:whitespace:docfreq:3:body",
        "qasw:standard:default",
        "qasw:standard:docfreq:5:field,body",
    ] {
        let c = corpus.to_vec();
        let n = name.to_string();
        o.spec(Spec::scenario(name, name, move || qasw_scenario(&n, &c)));
    }
    for name in [
        "tee:whitespace:source",
        "tee:whitespace:sink",
        "tee:whitespace:sink-lowercase",
        "tee:standard:source",
        "tee:standard:sink",
    ] {
        let n = name.to_string();
        o.spec(Spec::scenario(name, name, move || tee_scenario(&n)));
    }
    for name in [
        "custom:when-term",
        "custom:when-protected",
        "custom:classes",
    ] {
        let n = name.to_string();
        o.spec(Spec::scenario(name, name, move || custom_scenario(&n)));
    }
    o.spec(Spec::custom(
        "custom-chain",
        Chain::new()
            .config_dir(dir)
            .char_filter(
                "patternReplace",
                &[("pattern", "&uuml;"), ("replacement", "\u{FC}")],
            )
            .tokenizer("pathHierarchy", &[("delimiter", " ")])
            .filter("reverseString", &[])
            .filter("lowercase", &[]),
    ));
}

#[test]
fn oracle_454_to_460_factories_and_scenarios() {
    let dir = config_dir(
        "misc",
        &[
            ("protwords.txt", "running\nbanks\nkatter\n\u{434}\u{43e}\u{43c}\u{430}\n"),
            ("typing.txt", "6 \\b(\\d+)-(\\d+) ::: $1_hnum_$2\n2 \\b(\\w+)-(\\w+) ::: $1_hword_$2\n2 ^(\\d+)\\(?([a-z])\\)?$ ::: legal2_$1_$2\n"),
            ("stopwords.txt", "is\nthe\na\nof\nto\n"),
            ("common.txt", "s\na\nb\nc\nd\nthe\nof\nThe\n"),
        ],
    );
    let mut extra: Vec<&str> = MISC_INPUTS.to_vec();
    extra.extend(SNOWBALL_INPUTS);
    let corpus = task_corpus(0x0458_0460, &extra);
    let mut o = Oracle::new(corpus.clone());
    snowball_specs(&mut o, &dir);
    pattern_specs(&mut o, &dir);
    ngram_shingle_specs(&mut o, &dir);
    path_payload_specs(&mut o, &dir, &corpus);
    assert_oracle(&o);
}

/// The min-hash specs (#459), compared with Lucene's unpaired surrogates as
/// U+FFFD (see the module docs).
#[test]
fn oracle_459_min_hash() {
    let dir = config_dir("minhash", &[]);
    let mut o = Oracle::new(task_corpus(0x0459, &MISC_INPUTS));
    for (id, shingle, params) in [
        ("minhash-default", "5", vec![]),
        (
            "minhash-2-1-1",
            "2",
            vec![
                ("hashCount", "2"),
                ("bucketCount", "1"),
                ("hashSetSize", "1"),
            ],
        ),
        (
            "minhash-1-100-1",
            "3",
            vec![
                ("hashCount", "1"),
                ("bucketCount", "100"),
                ("hashSetSize", "1"),
            ],
        ),
        (
            "minhash-rotation",
            "3",
            vec![
                ("hashCount", "1"),
                ("bucketCount", "10"),
                ("hashSetSize", "1"),
                ("withRotation", "true"),
            ],
        ),
        (
            "minhash-10-1-10",
            "2",
            vec![
                ("hashCount", "10"),
                ("bucketCount", "1"),
                ("hashSetSize", "10"),
            ],
        ),
    ] {
        o.spec(chain(
            id,
            &dir,
            &[],
            ("whitespace", &[]),
            &[
                (
                    "shingle",
                    &[
                        ("minShingleSize", shingle),
                        ("maxShingleSize", shingle),
                        ("outputUnigrams", "false"),
                    ],
                ),
                ("minHash", &params),
            ],
        ));
    }
    o.spec(chain(
        "minhash-words",
        &dir,
        &[],
        ("standard", &[]),
        &[("minHash", &[])],
    ));
    assert_oracle_with(&o, fffd_lone_surrogates);
}
