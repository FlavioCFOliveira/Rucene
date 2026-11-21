//! Package `com.carrotsearch.randomizedtesting.snippets` of the runner's own
//! tests: the documentation snippet `TheGoodBadAndUglySnippet`.
//!
//! The snippet is no test class (Maven's surefire and `SelfTestsOracle` skip
//! it): its nested suite runs only nested. [`check`] runs that suite both
//! ways and compares the outcome with `tests/fixtures/self_tests_snippets.tsv`,
//! which `tests/fixtures/oracle/.../SnippetsOracle.java` wrote under
//! randomizedtesting 2.8.1 and JUnit 4.13.1. Seeds are not masked: the
//! nested run is seeded (`tests.seed`), so which iterations of `bad` fail is
//! part of the outcome.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::system;
use randomizedtesting::junit::{Failure, JUnitCore, RunListener};

mod the_good_bad_and_ugly_snippet;

use the_good_bad_and_ugly_snippet::{SEED, THE_GOOD_BAD_AND_UGLY};

use crate::with_nested_test_class::{clear_nested, run_tests, setup_nested};

/// The first line of a text, `<null>` for none, escaped as the oracle does.
fn first_line(s: Option<&str>) -> String {
    match s {
        None => "<null>".to_string(),
        Some(s) => s
            .split('\n')
            .next()
            .unwrap_or("")
            .replace('\\', "\\\\")
            .replace('\t', "\\t"),
    }
}

/// The oracle's rows for one scenario.
fn rows(scenario: &str, counts: [usize; 4], failures: &[Failure]) -> Vec<String> {
    let mut lines = vec![format!(
        "C\t{scenario}\t{}\t{}\t{}\t{}",
        counts[0], counts[1], counts[2], counts[3]
    )];
    for f in failures {
        let t = f.exception();
        lines.push(format!(
            "F\t{scenario}\t{}\t{}\t{}",
            first_line(Some(f.description().display_name())),
            t.class_name(),
            first_line(t.message())
        ));
    }
    lines
}

/// Records failures and assumption failures, in order.
#[derive(Default)]
struct Recorder {
    failures: Mutex<Vec<Failure>>,
    assumptions: Mutex<usize>,
}

impl RunListener for Recorder {
    fn test_failure(&self, f: &Failure) {
        self.failures
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(f.clone());
    }
    fn test_assumption_failure(&self, _f: &Failure) {
        *self.assumptions.lock().unwrap_or_else(|e| e.into_inner()) += 1;
    }
}

/// `direct`: JUnit runs the suite as is.
fn direct(suite: &Arc<Class>) -> Vec<String> {
    let recorder = Arc::new(Recorder::default());
    let mut core = JUnitCore::new();
    core.add_listener(recorder.clone());
    let r = core.run_all(std::slice::from_ref(suite));
    let failures = recorder
        .failures
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let assumptions = *recorder
        .assumptions
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    rows(
        "direct",
        [
            r.run_count(),
            r.failure_count(),
            r.ignore_count(),
            assumptions,
        ],
        &failures,
    )
}

/// `nested <seed>`: `runTests(suite)` between `setupNested()` and
/// `clearNested()`, with `tests.seed` set.
fn nested(suite: &Arc<Class>) -> Vec<String> {
    system::set_property("tests.seed", SEED);
    setup_nested();
    let r = run_tests(std::slice::from_ref(suite));
    clear_nested();
    system::clear_property("tests.seed");
    rows(
        &format!("nested {SEED}"),
        [
            r.get_run_count(),
            r.get_failure_count(),
            r.get_ignore_count(),
            r.get_assumption_ignored(),
        ],
        &r.get_failures(),
    )
}

/// Runs the snippet's scenarios and compares them with the oracle's rows;
/// appends the report lines and returns the number of differences.
pub fn check(report: &mut Vec<String>) -> usize {
    let path = format!(
        "{}/tests/fixtures/self_tests_snippets.tsv",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let java: Vec<&str> = text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .collect();
    let suite = Arc::clone(&THE_GOOD_BAD_AND_UGLY);
    let mut rust = direct(&suite);
    rust.extend(nested(&suite));
    let name = "snippets.TheGoodBadAndUglySnippet";
    if java == rust {
        report.push(format!("ok   {name}  {} oracle rows", java.len()));
        0
    } else {
        report.push(format!("DIFF {name}"));
        for i in 0..java.len().max(rust.len()) {
            let x = java.get(i).copied().unwrap_or("<none>");
            let y = rust.get(i).map(String::as_str).unwrap_or("<none>");
            if x != y {
                report.push(format!("    java: {x}\n    rust: {y}"));
            }
        }
        1
    }
}
