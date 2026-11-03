//! `org.junit.runner.notification.RunListener`, `RunNotifier`, `Failure`
//! and `org.junit.runner.Result`, rebuilt from their documented behaviour.

use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::description::Description;
use crate::jdk::exception::Throwable;

/// The result of a test (`Failure`): the description of the failed test
/// and the throwable.
#[derive(Clone)]
pub struct Failure {
    description: Arc<Description>,
    exception: Throwable,
}

impl Failure {
    /// `new Failure(description, thrownException)`.
    pub fn new(description: Arc<Description>, exception: Throwable) -> Failure {
        Failure {
            description,
            exception,
        }
    }

    /// `getDescription()`.
    pub fn description(&self) -> &Arc<Description> {
        &self.description
    }

    /// `getException()`.
    pub fn exception(&self) -> &Throwable {
        &self.exception
    }

    /// `getTestHeader()`: the description's display name.
    pub fn test_header(&self) -> &str {
        self.description.display_name()
    }

    /// `getMessage()`: the exception's message.
    pub fn message(&self) -> Option<&str> {
        self.exception.message()
    }

    /// `getTrace()`: the exception's stack trace text, as
    /// `printStackTrace` writes it (every line, the last included, ends with
    /// a line separator).
    pub fn trace(&self) -> String {
        let mut t = crate::jdk::system::format_stack_trace(&self.exception);
        t.push('\n');
        t
    }
}

impl fmt::Display for Failure {
    /// `toString()`: `header: message`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: {}",
            self.test_header(),
            self.message().unwrap_or("null")
        )
    }
}

impl fmt::Debug for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// `RunListener`: receives the events of a test run. Every method does
/// nothing by default.
pub trait RunListener: Send + Sync {
    /// `testRunStarted(description)`.
    fn test_run_started(&self, _description: &Arc<Description>) {}
    /// `testRunFinished(result)`.
    fn test_run_finished(&self, _result: &JUnitResult) {}
    /// `testStarted(description)`.
    fn test_started(&self, _description: &Arc<Description>) {}
    /// `testFinished(description)`.
    fn test_finished(&self, _description: &Arc<Description>) {}
    /// `testFailure(failure)`.
    fn test_failure(&self, _failure: &Failure) {}
    /// `testAssumptionFailure(failure)`.
    fn test_assumption_failure(&self, _failure: &Failure) {}
    /// `testIgnored(description)`.
    fn test_ignored(&self, _description: &Arc<Description>) {}
}

/// A shared listener; identity is the `Arc`.
pub type ListenerRef = Arc<dyn RunListener>;

fn same_listener(a: &ListenerRef, b: &ListenerRef) -> bool {
    std::ptr::eq(Arc::as_ptr(a) as *const u8, Arc::as_ptr(b) as *const u8)
}

/// Notifies listeners of a run's events (`RunNotifier`).
///
/// A listener that throws is removed and the failure is reported to the
/// remaining listeners against `Description.TEST_MECHANISM`, as JUnit
/// documents.
#[derive(Default)]
pub struct RunNotifier {
    listeners: Mutex<Vec<ListenerRef>>,
    please_stop: AtomicBool,
}

/// `org.junit.runner.notification.StoppedByUserException`.
pub const STOPPED_BY_USER: &str = "org.junit.runner.notification.StoppedByUserException";

impl RunNotifier {
    /// `new RunNotifier()`.
    pub fn new() -> RunNotifier {
        RunNotifier::default()
    }

    fn snapshot(&self) -> Vec<ListenerRef> {
        self.listeners
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// `addListener(listener)`.
    pub fn add_listener(&self, listener: ListenerRef) {
        self.listeners
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(listener);
    }

    /// `addFirstListener(listener)`.
    pub fn add_first_listener(&self, listener: ListenerRef) {
        self.listeners
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(0, listener);
    }

    /// `removeListener(listener)`.
    pub fn remove_listener(&self, listener: &ListenerRef) {
        self.listeners
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|l| !same_listener(l, listener));
    }

    fn fire(&self, f: &dyn Fn(&ListenerRef)) {
        let mut failed = Vec::new();
        for l in self.snapshot() {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(&l)));
            if let Err(p) = r {
                failed.push((l, Throwable::from_panic(p)));
            }
        }
        if !failed.is_empty() {
            for (l, _) in &failed {
                self.remove_listener(l);
            }
            let mechanism = Description::create_suite_description("Test mechanism", Vec::new());
            for (_, t) in failed {
                let failure = Failure::new(Arc::clone(&mechanism), t);
                self.fire(&|l| l.test_failure(&failure));
            }
        }
    }

    /// `fireTestRunStarted(description)`.
    pub fn fire_test_run_started(&self, description: &Arc<Description>) {
        self.fire(&|l| l.test_run_started(description));
    }

    /// `fireTestRunFinished(result)`.
    pub fn fire_test_run_finished(&self, result: &JUnitResult) {
        self.fire(&|l| l.test_run_finished(result));
    }

    /// `pleaseStop()`: the next `fireTestStarted` throws.
    pub fn please_stop(&self) {
        self.please_stop.store(true, Ordering::Release);
    }
}

/// The events a runner fires: implemented by [`RunNotifier`] and by the
/// notifiers that wrap it (`ThreadLeakControl`'s sub-notifier).
pub trait Notifier: Send + Sync {
    /// `fireTestStarted(description)`.
    ///
    /// # Panics
    ///
    /// `StoppedByUserException` after `pleaseStop()`.
    fn fire_test_started(&self, description: &Arc<Description>);
    /// `fireTestFailure(failure)`.
    fn fire_test_failure(&self, failure: Failure);
    /// `fireTestAssumptionFailed(failure)`.
    fn fire_test_assumption_failed(&self, failure: Failure);
    /// `fireTestIgnored(description)`.
    fn fire_test_ignored(&self, description: &Arc<Description>);
    /// `fireTestFinished(description)`.
    fn fire_test_finished(&self, description: &Arc<Description>);
}

impl Notifier for RunNotifier {
    fn fire_test_started(&self, description: &Arc<Description>) {
        if self.please_stop.load(Ordering::Acquire) {
            Throwable::with_message(STOPPED_BY_USER, None).throw();
        }
        self.fire(&|l| l.test_started(description));
    }

    fn fire_test_failure(&self, failure: Failure) {
        self.fire(&|l| l.test_failure(&failure));
    }

    fn fire_test_assumption_failed(&self, failure: Failure) {
        self.fire(&|l| l.test_assumption_failure(&failure));
    }

    fn fire_test_ignored(&self, description: &Arc<Description>) {
        self.fire(&|l| l.test_ignored(description));
    }

    fn fire_test_finished(&self, description: &Arc<Description>) {
        self.fire(&|l| l.test_finished(description));
    }
}

/// The summary of a run (`org.junit.runner.Result`).
#[derive(Default)]
pub struct JUnitResult {
    run_count: AtomicUsize,
    ignore_count: AtomicUsize,
    assumption_failure_count: AtomicUsize,
    failures: Mutex<Vec<Failure>>,
    run_time: AtomicI64,
    start_time: AtomicI64,
}

impl JUnitResult {
    /// `new Result()`.
    pub fn new() -> Arc<JUnitResult> {
        Arc::new(JUnitResult::default())
    }

    /// `getRunCount()`: the finished tests.
    pub fn run_count(&self) -> usize {
        self.run_count.load(Ordering::Acquire)
    }

    /// `getFailureCount()`.
    pub fn failure_count(&self) -> usize {
        self.failures
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .len()
    }

    /// `getFailures()`.
    pub fn failures(&self) -> Vec<Failure> {
        self.failures
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// `getIgnoreCount()`.
    pub fn ignore_count(&self) -> usize {
        self.ignore_count.load(Ordering::Acquire)
    }

    /// `getAssumptionFailureCount()`.
    pub fn assumption_failure_count(&self) -> usize {
        self.assumption_failure_count.load(Ordering::Acquire)
    }

    /// `getRunTime()` in milliseconds.
    pub fn run_time(&self) -> i64 {
        self.run_time.load(Ordering::Acquire)
    }

    /// `wasSuccessful()`: no failures.
    pub fn was_successful(&self) -> bool {
        self.failure_count() == 0
    }

    /// `createListener()`: the listener that fills this result.
    pub fn create_listener(self: &Arc<Self>) -> ListenerRef {
        Arc::new(ResultListener(Arc::clone(self)))
    }
}

struct ResultListener(Arc<JUnitResult>);

impl RunListener for ResultListener {
    fn test_run_started(&self, _d: &Arc<Description>) {
        self.0
            .start_time
            .store(crate::jdk::system::current_time_millis(), Ordering::Release);
    }
    fn test_run_finished(&self, _r: &JUnitResult) {
        let start = self.0.start_time.load(Ordering::Acquire);
        self.0.run_time.fetch_add(
            crate::jdk::system::current_time_millis() - start,
            Ordering::AcqRel,
        );
    }
    fn test_finished(&self, _d: &Arc<Description>) {
        self.0.run_count.fetch_add(1, Ordering::AcqRel);
    }
    fn test_failure(&self, failure: &Failure) {
        self.0
            .failures
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(failure.clone());
    }
    fn test_ignored(&self, _d: &Arc<Description>) {
        self.0.ignore_count.fetch_add(1, Ordering::AcqRel);
    }
    fn test_assumption_failure(&self, _f: &Failure) {
        self.0
            .assumption_failure_count
            .fetch_add(1, Ordering::AcqRel);
    }
}

/// A listener that prints every event to `System.out`, one line each, for
/// diagnostics (`TextListener` prints a summary instead; this is its
/// verbose counterpart used by the harness).
pub struct TextListener;

impl RunListener for TextListener {
    fn test_started(&self, d: &Arc<Description>) {
        crate::jdk::system::out_println(&format!("STARTED  {d}"));
    }
    fn test_failure(&self, f: &Failure) {
        crate::jdk::system::out_print(&format!("FAILURE  {}\n{}", f.description(), f.trace()));
    }
    fn test_assumption_failure(&self, f: &Failure) {
        crate::jdk::system::out_println(&format!("ASSUMED  {} {}", f.description(), f.exception()));
    }
    fn test_ignored(&self, d: &Arc<Description>) {
        crate::jdk::system::out_println(&format!("IGNORED  {d}"));
    }
    fn test_finished(&self, d: &Arc<Description>) {
        crate::jdk::system::out_println(&format!("FINISHED {d}"));
    }
}
