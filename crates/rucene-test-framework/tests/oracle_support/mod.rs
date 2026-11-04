//! Readers of the fixtures the Lucene oracles of the workspace's oracle
//! harness write (`crates/randomizedtesting/tests/fixtures/oracle/`, classes
//! under `src/org/apache/lucene/`; see its `README.md`): tab-separated rows,
//! `#` comment lines, the temporary directory masked as `$D` and identity hash
//! codes as `@HASH`.

#![allow(dead_code)]

use std::collections::BTreeMap;

use rucene::error::LuceneError;
use rucene::store::java_nio_file::{java_class_of, java_message_of};

/// The text of a fixture file of this crate.
pub fn fixture(name: &str) -> String {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

/// The tab-separated rows of a fixture, without `#` comment lines.
pub fn rows(name: &str) -> Vec<Vec<String>> {
    fixture(name)
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .map(|l| l.split('\t').map(str::to_string).collect())
        .collect()
}

/// The fixture as `label -> rest of the row` (the rest joined by tabs).
pub fn labelled(name: &str) -> BTreeMap<String, String> {
    rows(name)
        .into_iter()
        .map(|r| (r[0].clone(), r[1..].join("\t")))
        .collect()
}

/// Masks `dir` as `$D` (and `rel` as `$R`), identity hashes as `@HASH`, and
/// escapes line breaks and tabs, as the oracles' `m(…)` does.
pub fn mask(s: &str, dir: &str, rel: Option<&str>) -> String {
    let mut out = s.to_string();
    if let Some(rel) = rel {
        out = out.replace(rel, "$R");
    }
    out = out.replace(dir, "$D");
    let mut masked = String::with_capacity(out.len());
    let mut chars = out.chars().peekable();
    while let Some(c) = chars.next() {
        masked.push(c);
        if c == '@' {
            let mut hex = String::new();
            while let Some(&n) = chars.peek() {
                if n.is_ascii_hexdigit() && !n.is_ascii_uppercase() {
                    hex.push(n);
                    chars.next();
                } else {
                    break;
                }
            }
            if hex.is_empty() {
                continue;
            }
            masked.push_str("HASH");
        }
    }
    masked.replace('\n', "\\n").replace('\t', "\\t")
}

/// `class\tmessage` of a Rust error as the oracles print a Java exception.
pub fn lucene_error(e: &LuceneError) -> (String, String) {
    match e {
        LuceneError::Io(io) => (
            java_class_of(io).to_string(),
            java_message_of(io).unwrap_or_else(|| "<null>".to_string()),
        ),
        other => (
            format!("rucene::{}", other.java_simple_name()),
            other.to_string(),
        ),
    }
}

/// `class\tmessage` of an `io::Error`.
pub fn io_error(e: &std::io::Error) -> (String, String) {
    (
        java_class_of(e).to_string(),
        java_message_of(e).unwrap_or_else(|| "<null>".to_string()),
    )
}

/// `class\tmessage` of a panic payload (an unchecked Java exception).
pub fn panic_error(payload: Box<dyn std::any::Any + Send>) -> (String, String) {
    let t = randomizedtesting::jdk::exception::Throwable::from_panic(payload);
    (
        t.class_name().to_string(),
        t.message()
            .map_or_else(|| "<null>".to_string(), str::to_string),
    )
}

/// A fresh temporary directory, its real path, removed on drop.
pub struct TempDir(pub std::path::PathBuf);

impl TempDir {
    pub fn new(prefix: &str) -> TempDir {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!(
            "{prefix}-{}-{n}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p.canonicalize().unwrap())
    }

    pub fn str(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Compares two maps of `label -> line`, reporting every difference.
pub fn assert_same(
    what: &str,
    expected: &BTreeMap<String, String>,
    actual: &BTreeMap<String, String>,
) {
    let mut diffs = Vec::new();
    for (k, v) in expected {
        match actual.get(k) {
            None => diffs.push(format!("missing {k}")),
            Some(a) if a != v => diffs.push(format!("{k}:\n   java={v}\n   rust={a}")),
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
        "{what}: {} differences of {} rows:\n{}",
        diffs.len(),
        expected.len(),
        diffs.join("\n")
    );
}
