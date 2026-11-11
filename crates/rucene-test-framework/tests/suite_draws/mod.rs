//! The seeded-draw oracle of the `Base…FormatTestCase` suites (rmp #351,
//! #352, #353): runs a `Test…SuiteDraws` class (a suite whose `tearDown()`
//! records the next `random().nextLong()` of each test's randomness, and
//! whose `@AfterClass` records that of the class randomness) on the seeds
//! of a `BaseFormatSuitesOracle` fixture, and compares every row with
//! Java's.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::system;
use randomizedtesting::junit::{Failure, JUnitCore, Request, RunListener};

/// The failures and assumption failures of a run, as
/// `BaseFormatSuitesOracle` writes them.
struct Events(Mutex<Vec<String>>);

impl RunListener for Events {
    fn test_failure(&self, f: &Failure) {
        let m = f
            .description()
            .method_name()
            .unwrap_or_else(|| "null".into());
        let t = f.exception();
        let first = t
            .message()
            .unwrap_or("null")
            .lines()
            .next()
            .unwrap_or("")
            .replace('\t', "\\t")
            .replace('\r', "");
        self.0
            .lock()
            .unwrap()
            .push(format!("{m}\tF\t{}: {first}", t.class_name()));
    }
    fn test_assumption_failure(&self, f: &Failure) {
        let m = f
            .description()
            .method_name()
            .unwrap_or_else(|| "null".into());
        let first = f
            .message()
            .unwrap_or("null")
            .lines()
            .next()
            .unwrap_or("")
            .replace('\t', "\\t")
            .replace('\r', "");
        self.0.lock().unwrap().push(format!("{m}\tA\t{first}"));
    }
}

/// The rows of one seed of the draws class `class` builds, as
/// `BaseFormatSuitesOracle` writes them (sorted); `draws` is the list the
/// class records into.
pub fn draw_rows(seed: &str, class: fn() -> Arc<Class>, draws: &Mutex<Vec<String>>) -> Vec<String> {
    system::set_property("tests.seed", seed);
    draws.lock().unwrap().clear();
    let events = Arc::new(Events(Mutex::new(Vec::new())));
    let mut core = JUnitCore::new();
    core.add_listener(Arc::clone(&events) as Arc<dyn RunListener>);
    let r = core.run(&Request::a_class(&class()));
    system::clear_property("tests.seed");
    let mut rows: Vec<String> = draws.lock().unwrap().clone();
    rows.extend(events.0.lock().unwrap().iter().cloned());
    rows.push(format!(
        "#counts\trun={} fail={} assume={}\t-",
        r.run_count(),
        r.failure_count(),
        r.assumption_failure_count()
    ));
    rows.sort();
    rows.into_iter()
        .map(|row| format!("{seed}\t{row}"))
        .collect()
}

/// Asserts that, on the seeds of the fixture `fixture` (a file of
/// `tests/fixtures/`), every row of the draws class equals Java's.
// Each test binary includes this module and calls one of the two entry points.
#[allow(dead_code)]
pub fn assert_draws_match(fixture: &str, class: fn() -> Arc<Class>, draws: &Mutex<Vec<String>>) {
    assert_draws_match_excluding(fixture, class, draws, &[]);
}

/// The JVM system properties a fixture records (`# property\t<key>\t<value>`
/// lines, written by `BaseFormatSuitesOracle`), set as the process
/// environment `rucene::util::Constants` reads them from while the guard
/// lives, and restored when it drops.
///
/// They reach the draws through `IndexWriter.setDiagnostics`: the `.si` of
/// every segment records `os.arch`, `os.version`, `java.vendor` and
/// `java.runtime.version`, so its length, the segment's size and the order
/// `TieredMergePolicy` merges segments in depend on them.
struct OracleProperties(Vec<(String, Option<std::ffi::OsString>)>);

impl OracleProperties {
    fn set(lines: &[String]) -> OracleProperties {
        let mut saved = Vec::new();
        for line in lines {
            let mut parts = line.splitn(3, '\t');
            if parts.next() != Some("# property") {
                continue;
            }
            let (Some(key), Some(value)) = (parts.next(), parts.next()) else {
                panic!("malformed property line: {line}");
            };
            saved.push((key.to_string(), std::env::var_os(key)));
            std::env::set_var(key, value);
        }
        OracleProperties(saved)
    }
}

impl Drop for OracleProperties {
    fn drop(&mut self) {
        for (key, previous) in self.0.drain(..).rev() {
            match previous {
                Some(value) => std::env::set_var(&key, value),
                None => std::env::remove_var(&key),
            }
        }
    }
}

/// [`assert_draws_match`], leaving out of the comparison the `next` row of
/// each `(seed, method)` of `excluded` (on both sides): rows Java itself
/// does not reproduce from run to run. The method still runs, and its
/// failure and `#counts` rows are still compared.
pub fn assert_draws_match_excluding(
    fixture: &str,
    class: fn() -> Arc<Class>,
    draws: &Mutex<Vec<String>>,
    excluded: &[(&str, &str)],
) {
    let path = format!("{}/tests/fixtures/{fixture}", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let lines: Vec<String> = text.lines().map(str::to_string).collect();
    let is_excluded = |row: &String| {
        let mut f = row.split('\t');
        let (seed, method, kind) = (f.next(), f.next(), f.next());
        kind == Some("next")
            && excluded
                .iter()
                .any(|(s, m)| seed == Some(*s) && method == Some(*m))
    };
    let java: Vec<String> = lines
        .iter()
        .filter(|l| !l.starts_with('#'))
        .filter(|l| !is_excluded(l))
        .cloned()
        .collect();
    let mut seeds: Vec<String> = Vec::new();
    for row in &java {
        let seed = row.split('\t').next().unwrap().to_string();
        if !seeds.contains(&seed) {
            seeds.push(seed);
        }
    }
    let properties = OracleProperties::set(&lines);
    let mut rust: Vec<String> = Vec::new();
    for seed in &seeds {
        rust.extend(draw_rows(seed, class, draws));
    }
    drop(properties);
    rust.retain(|r| !is_excluded(r));
    let only_java: Vec<&String> = java.iter().filter(|r| !rust.contains(r)).collect();
    let only_rust: Vec<&String> = rust.iter().filter(|r| !java.contains(r)).collect();
    println!(
        "draw rows: java={} rust={} equal={}",
        java.len(),
        rust.len(),
        java.len() - only_java.len()
    );
    for r in &only_java {
        println!("  java only: {r}");
    }
    for r in &only_rust {
        println!("  rust only: {r}");
    }
    assert!(
        only_java.is_empty() && only_rust.is_empty(),
        "rows differ from Lucene's: java only {only_java:?}, rust only {only_rust:?}"
    );
}
