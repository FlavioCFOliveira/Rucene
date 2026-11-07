//! Runs a `Base…TestCase` suite with `JUnitCore` on one or more master
//! seeds and asserts that no test failed.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::system;
use randomizedtesting::junit::{Description, Failure, JUnitCore, Request, RunListener};

/// The master seeds `RUCENE_SUITE_SEEDS=<n>` runs a suite on (the first `n`).
pub const SEEDS: [&str; 10] = [
    "DEADBEEF", "1", "C0FFEE", "12345678", "A5A5A5A5", "7", "FEEDFACE", "31337", "BADC0DE", "2026",
];

/// What a run reports: one line per failure or assumption failure, the
/// started tests.
#[derive(Default)]
struct Events {
    started: Vec<String>,
    failures: Vec<String>,
    assumptions: Vec<String>,
    ignored: Vec<String>,
}

struct Listener(Mutex<Events>);

impl RunListener for Listener {
    fn test_started(&self, d: &Arc<Description>) {
        self.0
            .lock()
            .unwrap()
            .started
            .push(d.method_name().unwrap_or_else(|| "null".into()));
    }
    fn test_failure(&self, f: &Failure) {
        let m = f
            .description()
            .method_name()
            .unwrap_or_else(|| "null".into());
        let t = f.exception();
        self.0.lock().unwrap().failures.push(format!(
            "{m}: {}: {}",
            t.class_name(),
            t.message().unwrap_or("null")
        ));
    }
    fn test_assumption_failure(&self, f: &Failure) {
        let m = f
            .description()
            .method_name()
            .unwrap_or_else(|| "null".into());
        self.0
            .lock()
            .unwrap()
            .assumptions
            .push(format!("{m}: {}", f.message().unwrap_or("null")));
    }
    fn test_ignored(&self, d: &Arc<Description>) {
        self.0
            .lock()
            .unwrap()
            .ignored
            .push(d.method_name().unwrap_or_else(|| "null".into()));
    }
}

/// Runs the suite `class` builds once per seed and panics with every
/// failure; prints `<class> seed=<s>: run=<r> pass=<p> assume=<a>
/// ignore=<i>` per seed.
pub fn run_suite(class: fn() -> Arc<Class>) {
    let seeds: Vec<Option<&str>> = match std::env::var("RUCENE_SUITE_SEEDS") {
        Ok(n) => SEEDS
            .iter()
            .take(n.parse().expect("RUCENE_SUITE_SEEDS is a number"))
            .map(|s| Some(*s))
            .collect(),
        Err(_) => vec![None],
    };
    let mut all_failures = Vec::new();
    for seed in seeds {
        if let Some(seed) = seed {
            system::set_property("tests.seed", seed);
        }
        let class = class();
        let listener = Arc::new(Listener(Mutex::new(Events::default())));
        let mut core = JUnitCore::new();
        core.add_listener(Arc::clone(&listener) as Arc<dyn RunListener>);
        let r = core.run(&Request::a_class(&class));
        if seed.is_some() {
            system::clear_property("tests.seed");
        }
        let events = listener.0.lock().unwrap();
        println!(
            "{} seed={}: run={} pass={} assume={} ignore={} fail={}",
            class.name(),
            seed.unwrap_or("-"),
            r.run_count(),
            // A suite-level failure (an uncaught exception reported after a
            // test) counts as a failure without a test run of its own.
            r.run_count()
                .saturating_sub(r.failure_count())
                .saturating_sub(r.assumption_failure_count()),
            r.assumption_failure_count(),
            r.ignore_count(),
            r.failure_count()
        );
        for a in &events.assumptions {
            println!("  assume {a}");
        }
        for f in &events.failures {
            println!("  FAIL {f}");
            all_failures.push(format!("seed={}: {f}", seed.unwrap_or("-")));
        }
        assert!(!events.started.is_empty() || r.run_count() == 0);
    }
    assert!(
        all_failures.is_empty(),
        "failures:\n{}",
        all_failures.join("\n")
    );
}
