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

//! JDK 25 oracle of the `SimpleDateFormat` parsing emulation that
//! `DateRecognizerFilter` and its factory parse with (rmp #451;
//! `jdk/simple_date_format.rs`, `jdk/locale_data.rs`, `jdk/time_zone.rs`),
//! against `tools/translate/JdkDateParseOracle.java` run on the local JDK 25:
//!
//! * [`locale_class_keys_equal_the_jdk_dumpers`]: for every locale of
//!   `DateFormat.getAvailableLocales()`, alone and with eight Unicode
//!   extensions (`ca`, `nu`, `rg`, `tz`), and for edge tags (legacy
//!   variants, aliases, unknown subtags, the root locale), the class key and
//!   the zone key `LocaleData` computes equal those of the generator
//!   (`JdkDateParseData.key`/`zoneKey`), and the data holds both classes
//!   (`class_of`/`zone_class_of`, which panic on a missing key);
//! * [`parse_results_equal_jdk_25`]: for a seeded sample of locales (with
//!   `-u-ca-`, `-u-nu-`, `-u-rg-` and `-u-tz-` cases) and fifteen patterns
//!   (`DEFAULT`, numeric, text, era, zone names `z`/`zzzz`, RFC 822 `Z`, ISO
//!   8601 `XXX`, week dates, an illegal letter), the compiled pattern and
//!   time zone of each format, and for each input — dates the JDK format
//!   printed, their case, spacing and truncation variants, the same with a
//!   random zone name of the locale, and fixed strings — the parse
//!   position, error index, success and the format's time zone after the
//!   parse, equal JDK 25's.
//!
//! An input holding a lone surrogate (a truncation inside a pair) has no
//! Rust `&str`; such inputs are counted and skipped, as a term carrying one
//! cannot reach the parser through `rucene`'s `CharTermAttribute` (its
//! declared divergence).

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::Command;

use rucene_analysis_common::jdk::locale::Locale;
use rucene_analysis_common::jdk::locale_data::data;
use rucene_analysis_common::jdk::simple_date_format::{ParsePosition, SimpleDateFormat};
use rucene_test_framework::util::java_exceptions::java_class_name;

/// The seed of the sampled locales and generated dates.
const SEED: i64 = 0x0451_d47e;

/// The number of sampled available locales, beyond the fixed ones.
const SAMPLED_LOCALES: usize = 120;

/// `tools/translate/JdkDateParseOracle.java`.
fn oracle_source() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/translate/JdkDateParseOracle.java")
}

/// Runs the oracle with `args` (with the JDK-internal access the generator
/// needs) and returns its standard output.
fn run_oracle(args: &[&str]) -> String {
    let output = Command::new("java")
        .args([
            "-Xmx2g",
            "--add-exports",
            "java.base/sun.util.locale.provider=ALL-UNNAMED",
            "--add-exports",
            "java.base/sun.util.resources=ALL-UNNAMED",
            "--add-opens",
            "java.base/sun.util.cldr=ALL-UNNAMED",
            "--add-opens",
            "java.base/sun.util.locale.provider=ALL-UNNAMED",
            "--add-opens",
            "java.base/sun.util.calendar=ALL-UNNAMED",
            "--add-opens",
            "java.base/java.text=ALL-UNNAMED",
        ])
        .arg(oracle_source())
        .args(args)
        .output()
        .expect("a JDK 25 is required to run the date-parsing oracle");
    assert!(
        output.status.success(),
        "JdkDateParseOracle failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("the oracle prints UTF-8")
}

/// The oracle's escaping (`\\` and `\uXXXX`) undone, as UTF-16 units.
fn unescape(s: &str) -> Vec<u16> {
    let mut out = Vec::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('\\') => out.push(b'\\' as u16),
                Some('u') => {
                    let hex: String = chars.by_ref().take(4).collect();
                    out.push(u16::from_str_radix(&hex, 16).expect("a \\u escape"));
                }
                other => panic!("bad escape \\{other:?} in {s}"),
            }
        } else {
            let mut buf = [0u16; 2];
            out.extend_from_slice(c.encode_utf16(&mut buf));
        }
    }
    out
}

/// The simple name of the Java class of an error.
fn simple_class(e: &rucene::error::LuceneError) -> String {
    let class = java_class_name(e);
    class.rsplit('.').next().unwrap_or(&class).to_string()
}

#[test]
fn locale_class_keys_equal_the_jdk_dumpers() {
    let java = run_oracle(&["--keys"]);
    let data = data();
    let mut checked = 0;
    let mut ill = 0;
    let mut failures: Vec<String> = Vec::new();
    for line in java.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        let tag = String::from_utf16(&unescape(fields[0])).expect("a tag");
        let locale = Locale::builder_from_language_tag(&tag);
        if fields[1] == "ill" {
            ill += 1;
            if locale.is_ok() {
                failures.push(format!("{tag}: the JDK refuses the tag, Rucene accepts it"));
            }
            continue;
        }
        let locale = match locale {
            Ok(l) => l,
            Err(e) => {
                failures.push(format!("{tag}: Rucene refuses the tag: {e}"));
                continue;
            }
        };
        checked += 1;
        let java_key = String::from_utf16(&unescape(fields[1])).unwrap();
        let java_zone_key = String::from_utf16(&unescape(fields[2])).unwrap();
        let key = data.class_key(&locale);
        let zone_key = data.zone_key(&locale);
        if key != java_key {
            failures.push(format!(
                "{tag}: class key\n  JDK    {java_key}\n  Rucene {key}"
            ));
        }
        if zone_key != java_zone_key {
            failures.push(format!(
                "{tag}: zone key\n  JDK    {java_zone_key}\n  Rucene {zone_key}"
            ));
        }
        if catch_unwind(AssertUnwindSafe(|| {
            data.class_of(&locale);
            data.zone_class_of(&locale);
        }))
        .is_err()
        {
            failures.push(format!("{tag}: the data holds no class for the locale"));
        }
    }
    println!("checked {checked} locales, {ill} ill-formed tags");
    assert!(checked > 9_000, "the oracle listed {checked} locales");
    assert!(
        failures.is_empty(),
        "{} of {} tags differ:\n{}",
        failures.len(),
        checked + ill,
        failures
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn parse_results_equal_jdk_25() {
    let java = run_oracle(&["--parse", &SEED.to_string(), &SAMPLED_LOCALES.to_string()]);
    let mut format: Option<SimpleDateFormat> = None;
    let mut context = String::new();
    let (mut formats, mut parses, mut successes, mut skipped) = (0, 0, 0, 0);
    let mut zone_names = 0;
    let mut failures: Vec<String> = Vec::new();
    for line in java.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        match fields[0] {
            "format" => {
                formats += 1;
                let tag = String::from_utf16(&unescape(fields[1])).unwrap();
                let pattern = String::from_utf16(&unescape(fields[2])).unwrap();
                context = format!("{tag} {pattern:?}");
                let locale = Locale::builder_from_language_tag(&tag).expect("a well-formed tag");
                let built = if pattern == "DEFAULT" {
                    SimpleDateFormat::get_date_instance_default(&locale)
                } else {
                    SimpleDateFormat::new(&pattern, &locale)
                };
                format = None;
                match (built, fields[3].strip_prefix("error ")) {
                    (Err(e), Some(class)) => {
                        if simple_class(&e) != class {
                            failures.push(format!(
                                "{context}: JDK throws {class}, Rucene {}",
                                simple_class(&e)
                            ));
                        }
                    }
                    (Err(e), None) => failures.push(format!(
                        "{context}: JDK builds the format, Rucene throws {}",
                        simple_class(&e)
                    )),
                    (Ok(_), Some(class)) => failures.push(format!(
                        "{context}: JDK throws {class}, Rucene builds the format"
                    )),
                    (Ok(f), None) => {
                        let java_pattern = String::from_utf16(&unescape(fields[3])).unwrap();
                        if f.to_pattern() != java_pattern || f.get_time_zone_id() != fields[4] {
                            failures.push(format!(
                                "{context}: format JDK ({java_pattern:?}, {}) Rucene ({:?}, {})",
                                fields[4],
                                f.to_pattern(),
                                f.get_time_zone_id()
                            ));
                        }
                        format = Some(f);
                    }
                }
            }
            "parse" => {
                let Some(f) = &format else {
                    continue;
                };
                let units = unescape(fields[1]);
                let Ok(input) = String::from_utf16(&units) else {
                    skipped += 1;
                    continue;
                };
                parses += 1;
                let (index, error_index, ok, zone) = (
                    fields[2].parse::<usize>().unwrap(),
                    fields[3].parse::<i64>().unwrap(),
                    fields[4] == "true",
                    fields[5],
                );
                if ok {
                    successes += 1;
                }
                let mut pos = ParsePosition::new(0);
                let zone_before = f.get_time_zone_id();
                let result = f.parse_position(&input, &mut pos);
                let rust_ok = match result {
                    Ok(b) => b,
                    Err(e) => {
                        failures.push(format!(
                            "{context} {input:?}: Rucene throws {}",
                            simple_class(&e)
                        ));
                        continue;
                    }
                };
                if zone != zone_before {
                    zone_names += 1;
                }
                if (
                    pos.index,
                    pos.error_index,
                    rust_ok,
                    f.get_time_zone_id().as_str(),
                ) != (index, error_index, ok, zone)
                {
                    failures.push(format!(
                        "{context} {input:?}: JDK (index {index}, error {error_index}, {ok}, {zone}) \
                         Rucene (index {}, error {}, {rust_ok}, {})",
                        pos.index,
                        pos.error_index,
                        f.get_time_zone_id()
                    ));
                }
            }
            other => panic!("unexpected oracle line kind {other}"),
        }
    }
    println!(
        "{formats} formats, {parses} parses ({successes} successful, {zone_names} changed the \
         zone), {skipped} lone-surrogate inputs skipped"
    );
    assert!(formats > 2_000 && parses > 50_000 && successes > 10_000);
    assert!(
        zone_names > 0,
        "no input matched a zone name of another zone"
    );
    assert!(
        failures.is_empty(),
        "{} differences:\n{}",
        failures.len(),
        failures
            .iter()
            .take(30)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
