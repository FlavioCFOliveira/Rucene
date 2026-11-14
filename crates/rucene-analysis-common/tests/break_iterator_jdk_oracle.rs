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

//! JDK 25 oracle of the `java.text.BreakIterator` emulation (rmp #447):
//! `rucene_analysis_common::jdk::break_iterator` against
//! `BreakIterator.getWordInstance(Locale.ROOT)`,
//! `getSentenceInstance(Locale.ROOT)` and `getWordInstance(th)` of the local
//! JDK, through `BreakIteratorOracle.java`
//! (`tests/fixtures/java-codec-harness/.../codec/`), run once per test as a
//! single source file.
//!
//! Each case sets a text (`setText(String)` or, as `SegmentingTokenizerBase`
//! does, Lucene's `CharArrayIterator` over a sub-range) and runs a call
//! sequence: every forward boundary (`first`/`next`), then 12 random calls
//! among `first`, `last`, `next`, `next(n)`, `previous`, `following`,
//! `preceding`, `isBoundary` and `current`, with offsets that include
//! out-of-range ones (`IllegalArgumentException` on both sides). Every
//! result is compared.
//!
//! The Thai word iterator reads the installed JDK's own `thai_dict` at run
//! time (`jdk::break_dictionary`), so with a JDK on the machine it must equal
//! the JDK on every text, Thai included ([`thai_dictionary_source_is_the_jdk`]
//! asserts the source). The fallback iterator, on the embedded ICU 79.1 word
//! list (`get_icu_thai_word_instance`), must equal the JDK on texts without
//! Thai characters; on Thai text its differences are measured
//! ([`thai_dictionary_measurement`]) and reported, and must not be zero (the
//! positive control that the comparison sees a dictionary change).
//!
//! Positive controls: [`negative_control_detects_a_changed_result`] shows
//! that one wrong result fails the comparison.

mod analysis_oracle;

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use randomizedtesting::jdk::random::{JdkRandom, Random};
use rucene::error::LuceneError;
use rucene_analysis_common::analysis::util::char_array_iterator::CharArrayIterator;
use rucene_analysis_common::jdk::break_dictionary::{thai_dictionary_source, ThaiDictionarySource};
use rucene_analysis_common::jdk::break_iterator::{
    get_icu_thai_word_instance, get_sentence_instance, get_word_instance, BreakIterator,
};
use rucene_analysis_common::jdk::locale::Locale;

// ---------------------------------------------------------------------------
// The Java side
// ---------------------------------------------------------------------------

/// `BreakIteratorOracle.java`.
fn oracle_source() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../../tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
         BreakIteratorOracle.java",
    )
}

/// A Lucene 10.5.0 jar of the local Maven repository.
fn lucene_jar(artifact: &str) -> PathBuf {
    let home = std::env::var("HOME").expect("HOME is set");
    Path::new(&home).join(format!(
        ".m2/repository/org/apache/lucene/{artifact}/10.5.0/{artifact}-10.5.0.jar"
    ))
}

/// Runs `BreakIteratorOracle.java` with `args`; its stdout.
fn run_java(args: &[&str]) -> String {
    let class_path = std::env::join_paths([
        lucene_jar("lucene-core"),
        lucene_jar("lucene-analysis-common"),
    ])
    .expect("a class path");
    let output = Command::new("java")
        .arg("-cp")
        .arg(class_path)
        .arg(oracle_source())
        .args(args)
        .output()
        .expect("a JDK is required to run the BreakIterator oracle");
    assert!(
        output.status.success(),
        "BreakIteratorOracle.java failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("the oracle prints UTF-8")
}

/// The UTF-16 units of `units` as 4-digit hex (`-` when empty).
fn hex(units: &[u16]) -> String {
    if units.is_empty() {
        return "-".into();
    }
    let mut s = String::with_capacity(units.len() * 4);
    for u in units {
        write!(s, "{u:04x}").unwrap();
    }
    s
}

// ---------------------------------------------------------------------------
// Cases
// ---------------------------------------------------------------------------

/// Which iterator a case runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// `getWordInstance(Locale.ROOT)`.
    Word,
    /// `getSentenceInstance(Locale.ROOT)`.
    Sentence,
    /// `getWordInstance(th)`: the port on the process's Thai dictionary, the
    /// installed JDK's `thai_dict` on a machine with a JDK.
    Thai,
    /// The port's fallback Thai iterator, on the embedded ICU 79.1 list
    /// (`get_icu_thai_word_instance`).
    ThaiIcu,
}

impl Kind {
    /// The Java kind.
    fn java(self) -> &'static str {
        match self {
            Kind::Word => "w",
            Kind::Sentence => "s",
            Kind::ThaiIcu | Kind::Thai => "t",
        }
    }

    fn instance(self) -> Box<dyn BreakIterator> {
        match self {
            Kind::Word => get_word_instance(&Locale::of_parts("", "", "", "")),
            Kind::Sentence => get_sentence_instance(&Locale::of_parts("", "", "", "")),
            Kind::Thai => get_word_instance(&Locale::builder_from_language_tag("th").expect("th")),
            Kind::ThaiIcu => get_icu_thai_word_instance(),
        }
    }
}

/// One case: an iterator, a text (with a sub-range or not) and the calls.
#[derive(Debug, Clone)]
struct Case {
    kind: Kind,
    text: Vec<u16>,
    /// `Some((begin, length))`: `CharArrayIterator.setText(chars, begin,
    /// length)`; `None`: `setText(String)`.
    range: Option<(i32, i32)>,
    ops: Vec<String>,
}

impl Case {
    fn line(&self) -> String {
        let (b, l) = self.range.unwrap_or((-1, -1));
        format!(
            "{} {} {} {} {}",
            self.kind.java(),
            hex(&self.text),
            b,
            l,
            self.ops.join(",")
        )
    }
}

/// The result of `r` as the oracle prints it.
fn res<T: ToString>(r: rucene::error::Result<T>) -> String {
    match r {
        Ok(v) => v.to_string(),
        Err(e) => format!("E{}", simple_name(&e)),
    }
}

fn simple_name(e: &LuceneError) -> String {
    e.java_simple_name().to_string()
}

/// Runs `case` on `bi` (reused across cases, as the Java side reuses one
/// instance per kind).
fn run_rust(bi: &mut dyn BreakIterator, case: &Case) -> String {
    match case.range {
        None => bi.set_text_units(&case.text),
        Some((begin, length)) => {
            let mut it = CharArrayIterator::new_word_instance();
            it.set_text(&case.text, begin, length);
            bi.set_text(Box::new(it));
        }
    }
    let mut out = Vec::new();
    for op in &case.ops {
        let (c, arg) = op.split_at(1);
        let n = || arg.parse::<i32>().unwrap();
        let r = match c {
            "F" => bi.first().to_string(),
            "L" => bi.last().to_string(),
            "N" => bi.next().to_string(),
            "P" => bi.previous().to_string(),
            "C" => bi.current().to_string(),
            "X" => bi.next_n(n()).to_string(),
            "W" => res(bi.following(n())),
            "V" => res(bi.preceding(n())),
            "B" => res(bi.is_boundary(n()).map(|b| if b { 1 } else { 0 })),
            "A" => {
                let mut v = vec![bi.first()];
                loop {
                    let p = bi.next();
                    if p == -1 {
                        break;
                    }
                    v.push(p);
                }
                v.iter().map(i32::to_string).collect::<Vec<_>>().join(":")
            }
            _ => unreachable!("{op}"),
        };
        out.push(r);
    }
    out.join(",")
}

/// 12 random calls over a text whose iterator spans `[begin, end]`.
fn random_ops(r: &JdkRandom, begin: i32, end: i32) -> Vec<String> {
    let mut ops = vec!["A".to_string()];
    for _ in 0..12 {
        // offsets one past each end on purpose (IllegalArgumentException)
        let k = begin - 1 + r.next_int_bound(end - begin + 3);
        let op = match r.next_int_bound(9) {
            0 => "F".to_string(),
            1 => "L".to_string(),
            2 => "N".to_string(),
            3 => "P".to_string(),
            4 => "C".to_string(),
            5 => format!("X{}", r.next_int_bound(7) - 3),
            6 => format!("W{k}"),
            7 => format!("V{k}"),
            _ => format!("B{k}"),
        };
        ops.push(op);
    }
    ops
}

/// A case of `kind` over `text`: a sub-range one time in three.
fn make_case(r: &JdkRandom, kind: Kind, text: Vec<u16>) -> Case {
    let len = text.len() as i32;
    let range = if len > 0 && r.next_int_bound(3) == 0 {
        let begin = r.next_int_bound(len);
        let length = r.next_int_bound(len - begin + 1);
        Some((begin, length))
    } else {
        None
    };
    // `CharArrayIterator` indexes its range from 0.
    let (b, e) = match range {
        None => (0, len),
        Some((_, length)) => (0, length),
    };
    let ops = random_ops(r, b, e);
    Case {
        kind,
        text,
        range,
        ops,
    }
}

/// The JDK's results of `cases`, one line each.
fn java_results(cases: &[Case], name: &str) -> Vec<String> {
    let dir = std::env::temp_dir();
    let path = dir.join(format!(
        "rucene-brk-oracle-{name}-{}.txt",
        std::process::id()
    ));
    let mut spec = String::new();
    for c in cases {
        spec.push_str(&c.line());
        spec.push('\n');
    }
    std::fs::write(&path, spec).expect("write the spec file");
    let out = run_java(&["ops", path.to_str().unwrap()]);
    let _ = std::fs::remove_file(&path);
    let lines: Vec<String> = out.lines().map(str::to_string).collect();
    assert_eq!(lines.len(), cases.len(), "one result line per case");
    lines
}

/// The cases whose Rust results differ from the JDK's: `(case, jdk, rust)`.
fn compare(cases: &[Case], java: &[String]) -> Vec<(Case, String, String)> {
    let mut instances: Vec<(Kind, Box<dyn BreakIterator>)> = Vec::new();
    let mut diffs = Vec::new();
    for (case, j) in cases.iter().zip(java) {
        if !instances.iter().any(|(k, _)| *k == case.kind) {
            instances.push((case.kind, case.kind.instance()));
        }
        let bi = &mut instances
            .iter_mut()
            .find(|(k, _)| *k == case.kind)
            .unwrap()
            .1;
        let r = run_rust(&mut **bi, case);
        if &r != j {
            diffs.push((case.clone(), j.clone(), r));
        }
    }
    diffs
}

/// Panics with the first differences.
fn assert_no_diffs(what: &str, cases: usize, diffs: &[(Case, String, String)]) {
    println!("{what}: {cases} cases, {} differences", diffs.len());
    if !diffs.is_empty() {
        let mut report = format!("{what}: {} of {cases} cases differ\n", diffs.len());
        for (case, j, r) in diffs.iter().take(10) {
            writeln!(
                report,
                "  {:?} text={:?} range={:?} ops={}\n    jdk ={j}\n    rust={r}",
                case.kind,
                String::from_utf16_lossy(&case.text),
                case.range,
                case.ops.join(",")
            )
            .unwrap();
        }
        panic!("{report}");
    }
}

// ---------------------------------------------------------------------------
// Corpora
// ---------------------------------------------------------------------------

/// The characters of the rule classes (the alphabet of the model checks in
/// `tools/translate/BrkCheck.java`): letters, marks, digits, spaces and
/// controls, sentence and word punctuation, quotes, CJK and supplementary.
const ALPHA: &str = "aAzZ\u{e9}\u{df}\u{3a9}\u{3c9}\u{436}\u{416}\u{5d0}\u{628}\u{915}\u{903}\u{301}\u{20dd}\u{200b}\u{ad}\u{1} \t\n\r\u{c}\u{b}\u{a0}\u{2028}\u{3000}\u{2029}\
                     .. .  ..\"\"'!? 0959\u{663}\u{b2}\u{216b}.,'\"!?-_()[]\u{ab}\u{bb}\u{2018}\u{2019}\u{201c}\u{201d}#$%&\u{a2}\u{20ac}\u{66a}\u{2030}\u{2031}\u{66b}\u{2027}\u{964}\u{965}\u{3002}\u{ff01}\u{ff1f}\u{ff0e}:;/@+=\u{a9}\
                     \u{4e00}\u{3005}\u{30a2}\u{30a1}\u{30fd}\u{30fc}\u{30fb}\u{3042}\u{309d}\u{309b}\u{3099}\u{6f22}\u{3131}\u{ac00}\u{1d400}\u{1f600}\u{10000}";

/// Thai characters of the Thai rules (consonants, vowels, marks, digits,
/// `ฯ`, `ๆ`).
const THAI: &str = "\u{e01}\u{e02}\u{e2f}\u{e25}\u{e46}\u{e30}\u{e31}\u{e47}\u{e51}\u{e44}\u{e17}\u{e22}\u{e20}\u{e32}\u{e29}\u{e32}";

/// `n` random strings of 1 to `max_len` code points over `chars`.
fn random_strings(r: &JdkRandom, chars: &[char], n: usize, max_len: i32) -> Vec<Vec<u16>> {
    (0..n)
        .map(|_| {
            let len = 1 + r.next_int_bound(max_len);
            let s: String = (0..len)
                .map(|_| chars[r.next_int_bound(chars.len() as i32) as usize])
                .collect();
            s.encode_utf16().collect()
        })
        .collect()
}

/// Seeded multilingual sentences: words of several scripts joined by
/// spaces, sentence-ending punctuation (`.`, `?`, `!`, `。`, `।`) followed by
/// zero to two spaces, quotes and brackets around sentences, abbreviations,
/// numbers with decimal points and lower-case sentence starts.
fn multilingual_sentences(r: &JdkRandom, n: usize) -> Vec<Vec<u16>> {
    const WORDS: &[&str] = &[
        "the",
        "Quick",
        "brown",
        "fox",
        "U.S.A.",
        "Mr.",
        "e.g.",
        "3.14",
        "1,000",
        "caf\u{e9}",
        "Stra\u{df}e",
        "\u{3c3}\u{3bf}\u{3c6}\u{3af}\u{3b1}",
        "\u{41c}\u{43e}\u{441}\u{43a}\u{432}\u{430}",
        "\u{627}\u{644}\u{643}\u{62a}\u{627}\u{628}",
        "\u{5e9}\u{5dc}\u{5d5}\u{5dd}",
        "\u{928}\u{92e}\u{938}\u{94d}\u{924}\u{947}",
        "\u{4e2d}\u{6587}",
        "\u{65e5}\u{672c}\u{8a9e}",
        "\u{d55c}\u{ad6d}\u{c5b4}",
        "\u{30ab}\u{30bf}\u{30ab}\u{30ca}",
        "it's",
        "e-mail",
        "x\u{2019}y",
        "\u{1f600}",
        "\u{20000}",
        "a\u{301}",
        "\u{e20}\u{e32}\u{e29}\u{e32}\u{e44}\u{e17}\u{e22}",
        "AT&T",
        "www.example.com",
        "foo@bar.org",
        "(see",
        "below)",
        "\u{ab}quoted\u{bb}",
    ];
    const ENDS: &[&str] = &[
        ".", "?", "!", "\u{3002}", "\u{964}", "...", "?!", ".\"", ".)", ";", "",
    ];
    (0..n)
        .map(|_| {
            let mut s = String::new();
            let sentences = 1 + r.next_int_bound(4);
            for i in 0..sentences {
                if i > 0 {
                    s.push_str(&" ".repeat(r.next_int_bound(3) as usize));
                }
                if r.next_int_bound(6) == 0 {
                    s.push(['"', '(', '\u{201c}', '\''][r.next_int_bound(4) as usize]);
                }
                let words = 1 + r.next_int_bound(8);
                for w in 0..words {
                    if w > 0 {
                        s.push(if r.next_int_bound(10) == 0 { '\t' } else { ' ' });
                    }
                    s.push_str(WORDS[r.next_int_bound(WORDS.len() as i32) as usize]);
                }
                s.push_str(ENDS[r.next_int_bound(ENDS.len() as i32) as usize]);
            }
            s.encode_utf16().collect()
        })
        .collect()
}

/// Lucene 10.5.0's Thai test texts (`TestThaiAnalyzer`,
/// `TestThaiTokenizerFactory`).
const LUCENE_THAI_TEXTS: &[&str] = &[
    "การที่ได้ต้องแสดงว่างานดี",
    "การที่ได้ต้อง the แสดงว่างานดี",
    "การที่ได้ต้องthe แสดงว่างานดี",
    "บริษัทชื่อ XY&Z - คุยกับ xyz@demo.com",
    "ภาษาไทย",
    "๑๒๓๔",
    "This is a test. การที่ได้ต้องแสดงว่างานดี",
];

/// The words of Lucene's Thai test texts, as Lucene's tests expect them.
const LUCENE_THAI_WORDS: &[&str] = &[
    "การ",
    "ที่",
    "ได้",
    "ต้อง",
    "แสดง",
    "ว่า",
    "งาน",
    "ดี",
    "บริษัท",
    "ชื่อ",
    "คุย",
    "กับ",
    "ภาษา",
    "ไทย",
];

/// The ICU 79.1 Thai word list embedded in the crate.
fn icu_thai_words() -> Vec<String> {
    let text = include_str!("../resources/icu/thaidict.txt");
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    text.lines()
        .map(str::trim)
        .filter(|w| !w.is_empty() && !w.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// The seeded Thai corpus of the measurement: `n` sentences of 2 to 12
/// words, each word drawn from the ICU 79.1 word list (9 times in 10) or
/// from the words of Lucene's Thai test texts (1 time in 10), written
/// without spaces as Thai is, with a space between words 1 time in 8, and,
/// 1 time in 20 each, a Latin word, Thai digits, `ฯ` or `ๆ` after a word.
fn thai_corpus(seed: i64, n: usize) -> Vec<Vec<u16>> {
    let icu = icu_thai_words();
    let r = JdkRandom::new(seed);
    (0..n)
        .map(|_| {
            let mut s = String::new();
            let words = 2 + r.next_int_bound(11);
            for w in 0..words {
                if w > 0 && r.next_int_bound(8) == 0 {
                    s.push(' ');
                }
                if r.next_int_bound(10) == 0 {
                    s.push_str(
                        LUCENE_THAI_WORDS
                            [r.next_int_bound(LUCENE_THAI_WORDS.len() as i32) as usize],
                    );
                } else {
                    s.push_str(&icu[r.next_int_bound(icu.len() as i32) as usize]);
                }
                match r.next_int_bound(80) {
                    0 => s.push_str(" Lucene "),
                    1 => s.push_str("\u{e51}\u{e52}\u{e53}"),
                    2 => s.push('\u{e2f}'),
                    3 => s.push('\u{e46}'),
                    _ => {}
                }
            }
            s.encode_utf16().collect()
        })
        .collect()
}

fn has_thai(units: &[u16]) -> bool {
    units.iter().any(|&u| (0x0E00..=0x0E7F).contains(&u))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Word and sentence instances of the root locale, every method, on the
/// harness's standard corpus, seeded strings over the rules' classes and
/// seeded multilingual sentences.
#[test]
fn word_and_sentence_instances_equal_jdk() {
    let r = JdkRandom::new(0x447);
    let alpha: Vec<char> = ALPHA.chars().collect();
    let mut texts: Vec<Vec<u16>> = analysis_oracle::corpus::standard(0x447, 300);
    texts.extend(random_strings(&r, &alpha, 15_000, 12));
    texts.extend(random_strings(&r, &alpha, 5_000, 40));
    texts.extend(multilingual_sentences(&r, 5_000));
    let mut cases = Vec::new();
    for text in &texts {
        cases.push(make_case(&r, Kind::Word, text.clone()));
        cases.push(make_case(&r, Kind::Sentence, text.clone()));
    }
    let java = java_results(&cases, "ws");
    let diffs = compare(&cases, &java);
    assert_no_diffs("word+sentence (root)", cases.len(), &diffs);
}

/// The Thai word instance (the installed JDK's `thai_dict`): equal to the
/// JDK on Lucene's Thai test texts, a seeded Thai corpus, seeded strings over
/// the Thai and rule classes, and the standard corpus; and the fallback
/// (ICU-list) instance equal to the JDK on every text without Thai
/// characters.
#[test]
fn thai_word_instance_equals_jdk() {
    let r = JdkRandom::new(0x0447_7411);
    let mut chars: Vec<char> = ALPHA.chars().collect();
    for _ in 0..3 {
        chars.extend(THAI.chars());
    }
    let mut texts: Vec<Vec<u16>> = LUCENE_THAI_TEXTS
        .iter()
        .map(|s| s.encode_utf16().collect())
        .collect();
    texts.extend(thai_corpus(0x447, 3_000));
    texts.extend(random_strings(&r, &chars, 10_000, 16));
    texts.extend(analysis_oracle::corpus::standard(0x7411, 300));
    let alpha: Vec<char> = ALPHA.chars().collect();
    let non_thai = random_strings(&r, &alpha, 5_000, 20);
    let mut cases = Vec::new();
    for text in &texts {
        cases.push(make_case(&r, Kind::Thai, text.clone()));
    }
    for text in texts.iter().chain(&non_thai).filter(|t| !has_thai(t)) {
        cases.push(make_case(&r, Kind::ThaiIcu, text.clone()));
    }
    let java = java_results(&cases, "th");
    let diffs = compare(&cases, &java);
    let algorithm: Vec<_> = diffs
        .iter()
        .filter(|(c, _, _)| c.kind == Kind::Thai)
        .cloned()
        .collect();
    let non_thai_diffs: Vec<_> = diffs
        .iter()
        .filter(|(c, _, _)| c.kind == Kind::ThaiIcu)
        .cloned()
        .collect();
    let n_jdk = cases.iter().filter(|c| c.kind == Kind::Thai).count();
    assert_no_diffs("thai (JDK thai_dict read at run time)", n_jdk, &algorithm);
    assert_no_diffs(
        "thai fallback (ICU list, no Thai text)",
        cases.len() - n_jdk,
        &non_thai_diffs,
    );
}

/// The Thai dictionary source is the JDK's `thai_dict`, read from the image
/// of the JDK the lookup order finds first (`JAVA_HOME`, then `java` on
/// `PATH`): on this machine, the JDK whose `java` the oracle runs.
#[test]
fn thai_dictionary_source_is_the_jdk() {
    let expected = match std::env::var_os("JAVA_HOME").filter(|h| !h.is_empty()) {
        Some(home) => PathBuf::from(home).join("lib/modules"),
        None => {
            let java = std::env::split_paths(&std::env::var_os("PATH").expect("PATH"))
                .map(|d| d.join("java"))
                .find(|j| j.is_file())
                .expect("java on PATH");
            let java = std::fs::canonicalize(java).expect("canonical java");
            java.parent().unwrap().parent().unwrap().join("lib/modules")
        }
    };
    assert_eq!(
        ThaiDictionarySource::Jdk { image: expected },
        thai_dictionary_source()
    );
}

/// Regression test of the supplementary categories of the JDK's compiled
/// tables (`supplementary_table` and `type_segments` in
/// `break_iterator_rules.rs`): code points where `Character.getType` and
/// the JDK's break iterators disagree, in letter, digit, Thai and sentence
/// contexts, for the three iterators. Before the port reproduced the JDK's
/// table construction these differed (observed with a per-code-point
/// comparison of every code point; `U+2FA24` and `U+110BD`/`U+110CD` in
/// random strings): gaps before a `UnicodeData.txt` `First>` range
/// (`U+2A6E0`, `U+2FFFF`, `U+3134B`) join words, the last code point of a
/// supplementary ignore range (`U+1343F`, `U+1D17A`, `U+E007F`) and a
/// single-code-point ignore range (`U+11001`, `U+110BD`, `U+E0001`) are not
/// ignored.
#[test]
fn supplementary_category_quirks_equal_jdk() {
    let r = JdkRandom::new(0x10000);
    let points = [
        0x2A6DF, 0x2A6E0, 0x2A6FF, 0x2A700, 0x2B73A, 0x2EBE1, 0x2FA1D, 0x2FA1E, 0x2FA24, 0x2FFFF,
        0x30000, 0x3134B, 0x323AF, 0x323B0, 0x13430, 0x1343E, 0x1343F, 0x13440, 0x1BCA3, 0x1D173,
        0x1D17A, 0x11000, 0x11001, 0x11002, 0x110BD, 0x110CD, 0x10376, 0x1037A, 0x10A03, 0xE0001,
        0xE0020, 0xE007E, 0xE007F, 0xE0100, 0xE01EF, 0xFFFFE, 0x10FFFD,
    ];
    let contexts = [
        "a{}a",
        "1{}1",
        "{}{}",
        " {}.",
        "\u{e01}{}\u{e01}",
        "\u{4e00}{}\u{4e00}",
        "a.{}A",
        "x{}. y",
    ];
    let mut cases = Vec::new();
    for &p in &points {
        let x = char::from_u32(p).unwrap().to_string();
        for c in contexts {
            let text: Vec<u16> = c.replace("{}", &x).encode_utf16().collect();
            for kind in [Kind::Word, Kind::Sentence, Kind::Thai] {
                cases.push(make_case(&r, kind, text.clone()));
            }
        }
    }
    let java = java_results(&cases, "supp");
    let diffs = compare(&cases, &java);
    assert_no_diffs("supplementary categories", cases.len(), &diffs);
}

/// Positive control of the comparison: a JDK result changed by one fails.
#[test]
fn negative_control_detects_a_changed_result() {
    let r = JdkRandom::new(7);
    let cases: Vec<Case> = LUCENE_THAI_TEXTS
        .iter()
        .map(|s| make_case(&r, Kind::Sentence, s.encode_utf16().collect()))
        .collect();
    let mut java = java_results(&cases, "neg");
    assert!(
        compare(&cases, &java).is_empty(),
        "the untouched results match"
    );
    // first boundary list "0:..." becomes "1:..."
    java[0] = java[0].replacen('0', "1", 1);
    let diffs = compare(&cases, &java);
    assert_eq!(1, diffs.len(), "the changed result is detected");
}

/// The boundaries `bi` finds in `text` (`first`/`next`).
fn boundaries(bi: &mut dyn BreakIterator, text: &[u16]) -> Vec<i32> {
    bi.set_text_units(text);
    let mut v = vec![bi.first()];
    loop {
        let p = bi.next();
        if p == -1 {
            return v;
        }
        v.push(p);
    }
}

/// `text` with `|` at each boundary of `b`.
fn segmented(text: &[u16], b: &[i32]) -> String {
    let mut out = String::new();
    let mut last = 0usize;
    for &p in b.iter().skip(1) {
        out.push_str(&String::from_utf16_lossy(&text[last..p as usize]));
        out.push('|');
        last = p as usize;
    }
    out.pop();
    out
}

/// The Thai word breaks of the port against JDK 25 on Lucene's Thai test
/// texts and a seeded Thai corpus of 10,000 sentences ([`thai_corpus`]): the
/// port's Thai iterator (the installed JDK's `thai_dict`) must equal the JDK
/// on every text. The fallback (ICU 79.1 list) is measured on the same texts
/// (the share of texts with identical breaks, the share of break positions
/// that differ, the first 10 differing texts), and must differ somewhere:
/// the positive control that the comparison sees the dictionary. Written to
/// `$TMPDIR/rucene-thai-measurement.txt` as well.
#[test]
fn thai_dictionary_measurement() {
    let mut texts: Vec<Vec<u16>> = LUCENE_THAI_TEXTS
        .iter()
        .map(|s| s.encode_utf16().collect())
        .collect();
    let lucene_count = texts.len();
    texts.extend(thai_corpus(0x0447_0447, 10_000));
    let cases: Vec<Case> = texts
        .iter()
        .map(|t| Case {
            kind: Kind::ThaiIcu,
            text: t.clone(),
            range: None,
            ops: vec!["A".into()],
        })
        .collect();
    let java = java_results(&cases, "measure");
    let mut icu = Kind::ThaiIcu.instance();
    let mut jdk_words = Kind::Thai.instance();
    assert!(matches!(
        thai_dictionary_source(),
        ThaiDictionarySource::Jdk { .. }
    ));
    let mut report = String::new();
    let mut summary = |name: &str, range: std::ops::Range<usize>, report: &mut String| {
        let (mut same, mut diff_pos, mut union_pos, mut jdk_pos, mut algorithm) = (0, 0, 0, 0, 0);
        let mut examples = Vec::new();
        for i in range.clone() {
            let j: Vec<i32> = java[i].split(':').map(|p| p.parse().unwrap()).collect();
            let r = boundaries(&mut *icu, &texts[i]);
            let w = boundaries(&mut *jdk_words, &texts[i]);
            if w != j {
                algorithm += 1;
            }
            let js: BTreeSet<i32> = j.iter().copied().collect();
            let rs: BTreeSet<i32> = r.iter().copied().collect();
            union_pos += js.union(&rs).count();
            jdk_pos += js.len();
            let d = js.symmetric_difference(&rs).count();
            diff_pos += d;
            if d == 0 {
                same += 1;
            } else if examples.len() < 10 {
                examples.push(format!(
                    "  text: {}\n    JDK : {}\n    ICU : {}",
                    String::from_utf16_lossy(&texts[i]),
                    segmented(&texts[i], &j),
                    segmented(&texts[i], &r)
                ));
            }
        }
        let n = range.len();
        writeln!(
            report,
            "{name}: texts={n} identical={same} ({:.2}%); break positions: JDK={jdk_pos} \
             union={union_pos} differing={diff_pos} ({:.2}% of the union, {:.2}% of JDK's); \
             port (JDK thai_dict) differs on {algorithm} texts",
            100.0 * same as f64 / n as f64,
            100.0 * diff_pos as f64 / union_pos as f64,
            100.0 * diff_pos as f64 / jdk_pos as f64,
        )
        .unwrap();
        for e in examples {
            writeln!(report, "{e}").unwrap();
        }
        (algorithm, diff_pos)
    };
    let (a1, _) = summary("Lucene Thai test texts", 0..lucene_count, &mut report);
    let (a2, icu_diffs) = summary("seeded Thai corpus", lucene_count..texts.len(), &mut report);
    println!("{report}");
    let _ = std::fs::write(
        std::env::temp_dir().join("rucene-thai-measurement.txt"),
        &report,
    );
    assert_eq!(0, a1 + a2, "the port (JDK thai_dict) equals the JDK");
    assert!(
        icu_diffs > 0,
        "positive control: the ICU fallback differs from the JDK on the corpus"
    );
}
