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

//! JDK 25 oracle of the locale-sensitive formats the query parsers call
//! (rmp #476; `src/jdk/`), against `tools/translate/JdkFormatOracle.java`
//! run on the local JDK 25.
//!
//! For every locale of `Locale.getAvailableLocales()` (1158 on JDK 25.0.5),
//! sharded over twelve tests so that nextest runs them in parallel, and a
//! seeded set of dates, numbers and strings per locale:
//!
//! * `DateFormat.getDateInstance(SHORT, l)`: its pattern; `format` of
//!   fixed and seeded dates in a seeded time zone; `parse(String,
//!   ParsePosition)` lenient and strict of the printed dates, their
//!   variants (truncated, trailing text, replaced fields, upper case, other
//!   digit sets) and fixed strings — index, error index and result;
//! * `NumberFormat.getNumberInstance(l)` and `getIntegerInstance(l)`:
//!   `format` of fixed and seeded `Long`, `Integer`, `Double` and `Float`
//!   values; `parse(String, ParsePosition)` of the printed numbers, their
//!   variants and fixed strings — index, error index, and the result's class
//!   and bits;
//! * `String.toLowerCase(l)` of fixed and seeded strings (Turkish and
//!   Azeri dotted/dotless `I`, Lithuanian dots, final sigma, supplementary
//!   characters);
//!
//! must equal JDK 25's. The locale itself must give the JDK's language tag.
//! A date input holding a lone surrogate (a truncation inside a pair) has no
//! Rust `&str` and is counted and skipped, as a query string carrying one
//! cannot reach the parser; number inputs are parsed as UTF-16 units.

use std::path::{Path, PathBuf};
use std::process::Command;

use rucene_queryparser::jdk::date::Date;
use rucene_queryparser::jdk::decimal_format::DecimalFormat;
use rucene_queryparser::jdk::format::{JavaNumber, ParsePosition};
use rucene_queryparser::jdk::locale::Locale;
use rucene_queryparser::jdk::number_format::{
    get_integer_instance, get_number_instance, NumberFormat,
};
use rucene_queryparser::jdk::simple_date_format::{SimpleDateFormat, DF_SHORT};
use rucene_queryparser::jdk::string::to_lower_case;
use rucene_queryparser::jdk::time_zone::TimeZone;
use rucene_test_framework::util::java_exceptions::java_class_name;

/// The seed of the generated dates, numbers and strings.
const SEED: i64 = 0x0476_f0a7;

/// The number of shards (test functions) the locales are spread over.
const SHARDS: usize = 12;

/// `tools/translate/JdkFormatOracle.java`.
fn oracle_source() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/translate/JdkFormatOracle.java")
}

/// The oracle's output for `shard`.
fn run_oracle(shard: usize) -> String {
    let work = std::env::temp_dir().join(format!(
        "rucene-jdk-format-oracle-{}-{shard}",
        std::process::id()
    ));
    std::fs::create_dir_all(&work).expect("create the work directory");
    let output = Command::new("java")
        .current_dir(&work)
        .arg(format!("-Djava.io.tmpdir={}", work.display()))
        .arg("-Xmx1g")
        .arg(oracle_source())
        .arg(SEED.to_string())
        .arg(shard.to_string())
        .arg(SHARDS.to_string())
        .output()
        .expect("a JDK 25 is required to run the format oracle");
    let _ = std::fs::remove_dir_all(&work);
    assert!(
        output.status.success(),
        "JdkFormatOracle failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("the oracle prints UTF-8")
}

/// The oracle's escaping (`\uXXXX`) undone, as UTF-16 units.
fn unescape(s: &str) -> Vec<u16> {
    let mut out = Vec::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            let u: String = chars.by_ref().skip(1).take(4).collect();
            out.push(u16::from_str_radix(&u, 16).expect("a \\u escape"));
        } else {
            out.push(c as u16);
        }
    }
    out
}

fn unescape_str(s: &str) -> String {
    String::from_utf16(&unescape(s)).expect("valid UTF-16")
}

/// The oracle's escaping of UTF-16 units.
fn escape(units: &[u16]) -> String {
    let mut out = String::new();
    for &u in units {
        if (0x20..=0x7e).contains(&u) && u != u16::from(b'\\') {
            out.push(char::from(u as u8));
        } else {
            out.push_str(&format!("\\u{u:04X}"));
        }
    }
    out
}

/// `JdkFormatOracle.number`.
fn number(n: Option<JavaNumber>) -> String {
    match n {
        None => "null".into(),
        Some(JavaNumber::Long(v)) => format!("L:{v}"),
        Some(JavaNumber::Integer(v)) => format!("I:{v}"),
        Some(JavaNumber::Double(v)) => format!("D:{:x}", v.to_bits()),
        Some(JavaNumber::Float(v)) => format!("F:{:x}", v.to_bits()),
    }
}

/// The inverse of [`number`] for a formatted value.
fn parse_number(s: &str) -> JavaNumber {
    let (kind, v) = s.split_at(2);
    match kind {
        "L:" => JavaNumber::Long(v.parse().expect("a long")),
        "I:" => JavaNumber::Integer(v.parse().expect("an int")),
        "D:" => JavaNumber::Double(f64::from_bits(u64::from_str_radix(v, 16).expect("bits"))),
        "F:" => JavaNumber::Float(f32::from_bits(u32::from_str_radix(v, 16).expect("bits"))),
        _ => panic!("bad number {s}"),
    }
}

/// The state of the locale being checked.
struct LocaleState {
    tag: String,
    locale: Locale,
    date: Option<SimpleDateFormat>,
    number: Option<DecimalFormat>,
    integer: Option<DecimalFormat>,
}

/// The counts a shard reports.
#[derive(Default)]
struct Counts {
    locales: usize,
    formats: usize,
    parses: usize,
    parsed: usize,
    numbers: usize,
    lowers: usize,
    skipped: usize,
}

fn check_shard(shard: usize) {
    let java = run_oracle(shard);
    let mut counts = Counts::default();
    let mut failures: Vec<String> = Vec::new();
    let mut state: Option<LocaleState> = None;
    for line in java.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if f[0] == "locale" {
            counts.locales += 1;
            let tag = unescape_str(f[1]);
            let locale = if f[2] == "true" {
                match Locale::builder_from_language_tag(&tag) {
                    Ok(l) => l,
                    Err(e) => {
                        failures.push(format!("{tag}: Rucene refuses the tag: {e}"));
                        state = None;
                        continue;
                    }
                }
            } else {
                Locale::of_parts(
                    &unescape_str(f[3]),
                    &unescape_str(f[4]),
                    &unescape_str(f[5]),
                    &unescape_str(f[6]),
                )
            };
            if locale.to_language_tag() != tag {
                failures.push(format!(
                    "{tag}: Rucene's locale gives the tag {}",
                    locale.to_language_tag()
                ));
            }
            state = Some(LocaleState {
                tag,
                locale,
                date: None,
                number: None,
                integer: None,
            });
            continue;
        }
        let Some(st) = state.as_mut() else {
            continue;
        };
        let tag = st.tag.clone();
        match f[0] {
            "dpattern" | "dpattern-error" => {
                let built = SimpleDateFormat::get_date_instance(DF_SHORT, &st.locale);
                match (built, f[0]) {
                    (Ok(df), "dpattern") => {
                        let pattern = unescape_str(f[1]);
                        if df.to_pattern() != pattern {
                            failures.push(format!(
                                "{tag}: SHORT pattern JDK {pattern:?} Rucene {:?}",
                                df.to_pattern()
                            ));
                        }
                        st.date = Some(df);
                    }
                    (Ok(_), _) => failures.push(format!(
                        "{tag}: JDK throws {} building the date format, Rucene builds it",
                        f[1]
                    )),
                    (Err(e), "dpattern") => {
                        failures.push(format!(
                            "{tag}: Rucene throws {} building the date format",
                            java_class_name(&e)
                        ));
                        st.date = None;
                    }
                    (Err(e), _) => {
                        if java_class_name(&e) != f[1] {
                            failures.push(format!(
                                "{tag}: JDK throws {}, Rucene {}",
                                f[1],
                                java_class_name(&e)
                            ));
                        }
                    }
                }
            }
            "dzone" => {
                if let Some(df) = &st.date {
                    df.set_time_zone(&TimeZone::get_time_zone(f[1]));
                }
            }
            "dformat" => {
                let Some(df) = &st.date else { continue };
                counts.formats += 1;
                let millis: i64 = f[1].parse().expect("millis");
                let expected = unescape(f[2]);
                match df.format(Date::new(millis)) {
                    Ok(s) => {
                        let got: Vec<u16> = s.encode_utf16().collect();
                        if got != expected {
                            failures.push(format!(
                                "{tag}: format({millis}) JDK {} Rucene {}",
                                f[2],
                                escape(&got)
                            ));
                        }
                    }
                    Err(e) => failures.push(format!(
                        "{tag}: format({millis}) Rucene throws {}",
                        java_class_name(&e)
                    )),
                }
            }
            "dparse" => {
                let Some(df) = &st.date else { continue };
                let lenient = f[1] == "true";
                let Ok(input) = String::from_utf16(&unescape(f[2])) else {
                    counts.skipped += 1;
                    continue;
                };
                counts.parses += 1;
                df.set_lenient(lenient);
                let mut pos = ParsePosition::new(0);
                let got = match df.parse_position(&input, &mut pos) {
                    Ok(d) => d.map_or_else(|| "null".to_string(), |d| d.get_time().to_string()),
                    Err(e) => format!("throws {}", java_class_name(&e)),
                };
                if got != "null" {
                    counts.parsed += 1;
                }
                let expected = (f[3], f[4], f[5]);
                let actual = (
                    pos.get_index().to_string(),
                    pos.get_error_index().to_string(),
                    got,
                );
                if (actual.0.as_str(), actual.1.as_str(), actual.2.as_str()) != expected {
                    failures.push(format!(
                        "{tag}: parse lenient={lenient} {:?}: JDK (index {}, error {}, {}) \
                         Rucene (index {}, error {}, {})",
                        f[2], expected.0, expected.1, expected.2, actual.0, actual.1, actual.2
                    ));
                }
            }
            "nformat" | "nparse" | "nformat-error" | "nparse-error" => {
                let kind = f[1];
                let slot = if kind == "number" {
                    &mut st.number
                } else {
                    &mut st.integer
                };
                if slot.is_none() {
                    let built = if kind == "number" {
                        get_number_instance(&st.locale)
                    } else {
                        get_integer_instance(&st.locale)
                    };
                    match built {
                        Ok(nf) => *slot = Some(nf),
                        Err(e) => {
                            failures.push(format!(
                                "{tag}: {kind} instance: Rucene throws {}",
                                java_class_name(&e)
                            ));
                            continue;
                        }
                    }
                }
                let nf = slot.as_ref().expect("built above");
                counts.numbers += 1;
                match f[0] {
                    "nformat" => {
                        let value = parse_number(f[2]);
                        match NumberFormat::format(nf, value) {
                            Ok(s) => {
                                let got: Vec<u16> = s.encode_utf16().collect();
                                if got != unescape(f[3]) {
                                    failures.push(format!(
                                        "{tag}: {kind} format({}) JDK {} Rucene {}",
                                        f[2],
                                        f[3],
                                        escape(&got)
                                    ));
                                }
                            }
                            Err(e) => failures.push(format!(
                                "{tag}: {kind} format({}) Rucene throws {}",
                                f[2],
                                java_class_name(&e)
                            )),
                        }
                    }
                    "nformat-error" => {
                        if f[2] != "-" {
                            match NumberFormat::format(nf, parse_number(f[2])) {
                                Ok(s) => failures.push(format!(
                                    "{tag}: {kind} format({}) JDK throws {}, Rucene gives {s:?}",
                                    f[2], f[3]
                                )),
                                Err(e) => {
                                    if java_class_name(&e) != f[3] {
                                        failures.push(format!(
                                            "{tag}: {kind} format({}) JDK throws {}, Rucene {}",
                                            f[2],
                                            f[3],
                                            java_class_name(&e)
                                        ));
                                    }
                                }
                            }
                        }
                    }
                    "nparse" => {
                        let input = unescape(f[2]);
                        let mut pos = ParsePosition::new(0);
                        let got = number(nf.parse_units(&input, &mut pos));
                        let actual = (
                            pos.get_index().to_string(),
                            pos.get_error_index().to_string(),
                            got,
                        );
                        if (actual.0.as_str(), actual.1.as_str(), actual.2.as_str())
                            != (f[3], f[4], f[5])
                        {
                            failures.push(format!(
                                "{tag}: {kind} parse {:?}: JDK (index {}, error {}, {}) \
                                 Rucene (index {}, error {}, {})",
                                f[2], f[3], f[4], f[5], actual.0, actual.1, actual.2
                            ));
                        }
                    }
                    _ => failures.push(format!(
                        "{tag}: {kind} parse {:?}: JDK throws {}",
                        f[2], f[3]
                    )),
                }
            }
            "lower" => {
                counts.lowers += 1;
                let input = unescape(f[1]);
                let got = to_lower_case(&input, &st.locale);
                if got != unescape(f[2]) {
                    failures.push(format!(
                        "{tag}: toLowerCase({}) JDK {} Rucene {}",
                        f[1],
                        f[2],
                        escape(&got)
                    ));
                }
            }
            other => panic!("unexpected oracle line kind {other}"),
        }
    }
    println!(
        "shard {shard}: {} locales, {} date formats, {} date parses ({} with a date), {} number \
         checks, {} lower-cases, {} lone-surrogate inputs skipped",
        counts.locales,
        counts.formats,
        counts.parses,
        counts.parsed,
        counts.numbers,
        counts.lowers,
        counts.skipped
    );
    assert!(
        counts.locales > 1100 / SHARDS && counts.parses > counts.locales * 100,
        "the oracle listed too little"
    );
    if !failures.is_empty() {
        // Every difference, and their count per kind (the text after the
        // locale up to the first `(` or `:`), on the captured output.
        let mut kinds: std::collections::BTreeMap<String, usize> = Default::default();
        for f in &failures {
            let rest = f.split_once(": ").map_or(f.as_str(), |(_, r)| r);
            let kind: String = rest
                .split(['(', ':'])
                .next()
                .unwrap_or("")
                .chars()
                .filter(|c| !c.is_ascii_digit())
                .collect();
            *kinds.entry(kind).or_default() += 1;
        }
        for (kind, n) in &kinds {
            println!("KIND {n:6} {kind}");
        }
        for f in failures.iter().take(3000) {
            println!("DIFF {f}");
        }
    }
    assert!(
        failures.is_empty(),
        "shard {shard}: {} differences from JDK 25:\n{}",
        failures.len(),
        failures
            .iter()
            .take(40)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn jdk_formats_shard_00() {
    check_shard(0);
}

#[test]
fn jdk_formats_shard_01() {
    check_shard(1);
}

#[test]
fn jdk_formats_shard_02() {
    check_shard(2);
}

#[test]
fn jdk_formats_shard_03() {
    check_shard(3);
}

#[test]
fn jdk_formats_shard_04() {
    check_shard(4);
}

#[test]
fn jdk_formats_shard_05() {
    check_shard(5);
}

#[test]
fn jdk_formats_shard_06() {
    check_shard(6);
}

#[test]
fn jdk_formats_shard_07() {
    check_shard(7);
}

#[test]
fn jdk_formats_shard_08() {
    check_shard(8);
}

#[test]
fn jdk_formats_shard_09() {
    check_shard(9);
}

#[test]
fn jdk_formats_shard_10() {
    check_shard(10);
}

#[test]
fn jdk_formats_shard_11() {
    check_shard(11);
}
