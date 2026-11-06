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

/// The first line of a message, tabs escaped, as `FrameworkTestsOracle.firstLine`.
fn first_line(s: Option<&str>) -> String {
    let v = s.unwrap_or("null");
    v.split('\n')
        .next()
        .unwrap_or("")
        .replace('\t', "\\t")
        .replace('\r', "")
}

/// The `RunListener` of `FrameworkTestsOracle`: one row per event.
struct OutcomeListener(std::sync::Mutex<Vec<String>>);

impl randomizedtesting::junit::RunListener for OutcomeListener {
    fn test_started(&self, d: &std::sync::Arc<randomizedtesting::junit::Description>) {
        let m = d.method_name().unwrap_or_else(|| "null".into());
        self.0.lock().unwrap().push(format!("{m}\tS\t-"));
    }
    fn test_failure(&self, f: &randomizedtesting::junit::Failure) {
        let m = f
            .description()
            .method_name()
            .unwrap_or_else(|| "null".into());
        let t = f.exception();
        self.0.lock().unwrap().push(format!(
            "{m}\tF\t{}: {}",
            t.class_name(),
            first_line(t.message())
        ));
    }
    fn test_assumption_failure(&self, f: &randomizedtesting::junit::Failure) {
        let m = f
            .description()
            .method_name()
            .unwrap_or_else(|| "null".into());
        self.0
            .lock()
            .unwrap()
            .push(format!("{m}\tA\t{}", first_line(f.message())));
    }
    fn test_ignored(&self, d: &std::sync::Arc<randomizedtesting::junit::Description>) {
        let m = d.method_name().unwrap_or_else(|| "null".into());
        self.0.lock().unwrap().push(format!("{m}\tI\t-"));
    }
}

/// Runs a suite with `JUnitCore` as `FrameworkTestsOracle` does and returns
/// the rows it records for the class: the `#counts` row, then the sorted
/// event rows, each `method\tevent\tdetail`.
pub fn framework_outcomes(
    class: &std::sync::Arc<randomizedtesting::jdk::reflect::Class>,
) -> Vec<String> {
    let listener = std::sync::Arc::new(OutcomeListener(std::sync::Mutex::new(Vec::new())));
    let mut core = randomizedtesting::junit::JUnitCore::new();
    core.add_listener(std::sync::Arc::clone(&listener)
        as std::sync::Arc<dyn randomizedtesting::junit::RunListener>);
    let r = core.run(&randomizedtesting::junit::Request::a_class(class));
    let mut events = listener.0.lock().unwrap().clone();
    events.sort();
    let mut rows = vec![format!(
        "#counts\trun={} fail={} ignore={} assume={}\t-",
        r.run_count(),
        r.failure_count(),
        r.ignore_count(),
        r.assumption_failure_count()
    )];
    rows.extend(events);
    rows
}

/// The rows `framework_tests.tsv` records for the Java test class `name`.
pub fn java_framework_outcomes(name: &str) -> Vec<String> {
    rows("framework_tests.tsv")
        .into_iter()
        .filter(|r| r[0] == name)
        .map(|r| r[1..].join("\t"))
        .collect()
}

/// Runs the suite and asserts it reports exactly what Lucene's own test
/// class reported on the JVM (`framework_tests.tsv`).
pub fn assert_framework_outcomes(class: &std::sync::Arc<randomizedtesting::jdk::reflect::Class>) {
    let name = class.name().to_string();
    let expected = java_framework_outcomes(&name);
    assert!(!expected.is_empty(), "{name} is not in framework_tests.tsv");
    let actual = framework_outcomes(class);
    assert_eq!(
        actual.join("\n"),
        expected.join("\n"),
        "{name}: rust (left) and java (right) outcomes differ"
    );
}
