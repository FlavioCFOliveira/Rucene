//! Port of `com.carrotsearch.randomizedtesting.PrintEventListener`
//! (randomized-runner 2.8.1 test sources, Apache-2.0): prints a run's events
//! to the standard output `WithNestedTestClass.setupNested` saved.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use randomizedtesting::junit::{Description, Failure, JUnitResult, RunListener};

use crate::with_nested_test_class::sysout_println;

/// `PrintEventListener extends RunListener` over `sysout`.
#[derive(Default)]
pub struct PrintEventListener {
    assumptions: AtomicUsize,
}

impl PrintEventListener {
    /// `new PrintEventListener(sysout)`.
    pub fn new() -> PrintEventListener {
        PrintEventListener::default()
    }
}

impl RunListener for PrintEventListener {
    fn test_run_started(&self, _d: &Arc<Description>) {
        sysout_println("Run started.");
    }
    fn test_run_finished(&self, r: &JUnitResult) {
        sysout_println(&format!(
            "Run finished: run={}, ignored={}, failures={}, assumptions={}\n",
            r.run_count(),
            r.ignore_count(),
            r.failure_count(),
            self.assumptions.load(Ordering::SeqCst)
        ));
    }
    fn test_started(&self, d: &Arc<Description>) {
        sysout_println(&format!(
            "Started : {}",
            d.method_name().unwrap_or_else(|| "null".into())
        ));
    }
    fn test_finished(&self, d: &Arc<Description>) {
        sysout_println(&format!(
            "Finished: {}",
            d.method_name().unwrap_or_else(|| "null".into())
        ));
    }
    fn test_failure(&self, f: &Failure) {
        sysout_println(&format!("Failure : {f}"));
    }
    fn test_assumption_failure(&self, f: &Failure) {
        sysout_println(&format!("Assumpt.: {f}"));
        self.assumptions.fetch_add(1, Ordering::SeqCst);
    }
    fn test_ignored(&self, d: &Arc<Description>) {
        match d.method_name() {
            // Ignored due to class-level @Ignore or some other reason.
            None => sysout_println(&format!("Ignored : {}", d.display_name())),
            Some(m) => sysout_println(&format!("Ignored : {m}")),
        }
    }
}
