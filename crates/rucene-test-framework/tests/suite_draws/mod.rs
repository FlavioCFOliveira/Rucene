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
pub fn assert_draws_match(fixture: &str, class: fn() -> Arc<Class>, draws: &Mutex<Vec<String>>) {
    let path = format!("{}/tests/fixtures/{fixture}", env!("CARGO_MANIFEST_DIR"));
    let java: Vec<String> = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{path}: {e}"))
        .lines()
        .filter(|l| !l.starts_with('#'))
        .map(str::to_string)
        .collect();
    let mut seeds: Vec<String> = Vec::new();
    for row in &java {
        let seed = row.split('\t').next().unwrap().to_string();
        if !seeds.contains(&seed) {
            seeds.push(seed);
        }
    }
    let mut rust: Vec<String> = Vec::new();
    for seed in &seeds {
        rust.extend(draw_rows(seed, class, draws));
    }
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
