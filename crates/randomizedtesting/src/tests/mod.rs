//! Oracle parity tests: every ported member against the values JDK 25 and
//! randomizedtesting 2.8.1 produce for the same inputs.
//!
//! The fixtures under `tests/fixtures/` are written by
//! `tests/fixtures/RandomizedTestingOracle.java`. Each sequence of draws is
//! an FNV-1a 64-bit digest over the little-endian bytes of its values
//! (floats and doubles by raw bits, strings as a 4-byte UTF-16 length then
//! their UTF-16 code units) plus the first value, computed the same way on
//! both sides.
//!
//! The tests live inside the crate because several ported members are
//! package-private in Java (`pub(crate)` here) and the seed chain and context
//! tests drive them as the runner does.

mod context;
mod generators;
mod helpers;
mod java_lang;
mod random_draws;
mod reflect_dispatch;
mod runner_parts;
mod seed_chain;
mod to_string;
mod ziggurat;

use std::collections::BTreeMap;
use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::jdk::exception::panic_message;

/// The text of a fixture file.
pub(crate) fn fixture(name: &str) -> String {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

/// The tab-separated rows of a fixture, without `#` comment lines.
pub(crate) fn rows(name: &str) -> Vec<Vec<String>> {
    fixture(name)
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .map(|l| l.split('\t').map(str::to_string).collect())
        .collect()
}

/// The FNV-1a digest of a sequence of values, as the oracle's `Rec`.
#[derive(Debug)]
pub(crate) struct Rec {
    h: u64,
    first: Vec<String>,
}

impl Default for Rec {
    fn default() -> Self {
        Rec {
            h: 0xcbf2_9ce4_8422_2325,
            first: Vec::new(),
        }
    }
}

impl Rec {
    fn b(&mut self, v: u8) {
        self.h ^= u64::from(v);
        self.h = self.h.wrapping_mul(0x100_0000_01b3);
    }

    fn first(&mut self, s: String) {
        if self.first.is_empty() {
            self.first.push(s);
        }
    }

    fn raw_i(&mut self, v: i32) {
        for b in v.to_le_bytes() {
            self.b(b);
        }
    }

    pub(crate) fn i(&mut self, v: i32) {
        self.raw_i(v);
        self.first(v.to_string());
    }

    pub(crate) fn l(&mut self, v: i64) {
        for b in v.to_le_bytes() {
            self.b(b);
        }
        self.first(v.to_string());
    }

    pub(crate) fn f(&mut self, v: f32) {
        let bits = v.to_bits();
        for b in bits.to_le_bytes() {
            self.b(b);
        }
        self.first(format!("{bits:08x}"));
    }

    pub(crate) fn d(&mut self, v: f64) {
        let bits = v.to_bits();
        for b in bits.to_le_bytes() {
            self.b(b);
        }
        self.first(format!("{bits:016x}"));
    }

    pub(crate) fn z(&mut self, v: bool) {
        self.b(u8::from(v));
        self.first(v.to_string());
    }

    pub(crate) fn bytes(&mut self, a: &[u8]) {
        self.raw_i(a.len() as i32);
        let mut hex = String::new();
        for &x in a {
            self.b(x);
            hex.push_str(&format!("{x:02x}"));
        }
        self.first(hex);
    }

    pub(crate) fn s(&mut self, v: &str) {
        let units: Vec<u16> = v.encode_utf16().collect();
        self.raw_i(units.len() as i32);
        for &u in &units {
            self.b(u as u8);
            self.b((u >> 8) as u8);
        }
        self.first(units_of(v));
    }

    /// `fnv64\tfirst`, the oracle's `Rec.line()`.
    pub(crate) fn line(&self) -> String {
        format!("{:016x}\t{}", self.h, self.first.join(","))
    }
}

/// A string as the oracle prints it: hex UTF-16 code units joined by `.`,
/// `-` when empty.
pub(crate) fn units_of(v: &str) -> String {
    let parts: Vec<String> = v.encode_utf16().map(|u| format!("{u:04x}")).collect();
    if parts.is_empty() {
        "-".to_string()
    } else {
        parts.join(".")
    }
}

/// The string the oracle printed as code units.
pub(crate) fn from_units(units: &str) -> String {
    if units == "-" {
        return String::new();
    }
    let u: Vec<u16> = units
        .split('.')
        .map(|x| u16::from_str_radix(x, 16).expect("hex unit"))
        .collect();
    String::from_utf16(&u).expect("valid UTF-16 in the oracle's inputs")
}

/// The oracle's 20 seeds: the edge values, then ten from
/// `new SplittableRandom(20261103L).nextLong()`, read from the fixture's
/// `formatSeed` rows (the oracle prints them all).
pub(crate) fn seeds() -> Vec<i64> {
    rows("seed_utils.tsv")
        .into_iter()
        .filter(|r| r[0] == "formatSeed")
        .map(|r| r[1].parse().expect("seed"))
        .collect()
}

/// Runs `f` and returns `Ok(value)` or `Err(panic message)`.
pub(crate) fn catch<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    catch_unwind(AssertUnwindSafe(f)).map_err(|p| panic_message(p.as_ref()))
}

/// The oracle's `class\tmessage` of a thrown exception (`<null>` for a null
/// message), from a panic message `class: message` (first line only).
pub(crate) fn java_error(panic: &str) -> String {
    let first = panic.lines().next().unwrap_or("");
    match first.split_once(": ") {
        Some((class, message)) if !class.contains(' ') => format!("{class}\t{message}"),
        _ => format!("{first}\t<null>"),
    }
}

/// Compares two maps of `label -> line`, reporting every difference.
pub(crate) fn assert_same_lines(
    what: &str,
    expected: &BTreeMap<String, String>,
    actual: &BTreeMap<String, String>,
) {
    let mut diffs = Vec::new();
    for (k, v) in expected {
        match actual.get(k) {
            None => diffs.push(format!("missing {k}")),
            Some(a) if a != v => diffs.push(format!("{k}: java={v} rust={a}")),
            _ => {}
        }
    }
    for k in actual.keys() {
        if !expected.contains_key(k) {
            diffs.push(format!("extra {k}"));
        }
    }
    assert!(
        diffs.is_empty(),
        "{what}: {} of {} rows differ:\n{}",
        diffs.len(),
        expected.len(),
        diffs
            .iter()
            .take(40)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// Replaces every run of decimal digits that follows `marker` with `N`, to
/// compare messages that print thread identifiers.
pub(crate) fn mask_after(s: &str, marker: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(pos) = rest.find(marker) {
        out.push_str(&rest[..pos + marker.len()]);
        rest = &rest[pos + marker.len()..];
        let digits = rest
            .find(|c: char| !c.is_ascii_hexdigit())
            .unwrap_or(rest.len());
        out.push('N');
        rest = &rest[digits..];
    }
    out.push_str(rest);
    out
}
