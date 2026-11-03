//! Port of `com.carrotsearch.randomizedtesting.WithNestedTestClass`
//! (randomized-runner 2.8.1, `src/test/java/.../WithNestedTestClass.java`,
//! Apache-2.0): the superclass of the suites that run nested
//! `RandomizedRunner` suites and check their outcome.
//!
//! Java's static members are the module's statics; the class declaration is
//! [`CLASS`], which the outer suites extend.
//!
//! **Divergences.**
//! * `setupNested` tees `System.out`/`System.err` into a buffer through the
//!   port's `System.setOut` sinks, and replaces the root logger's handlers
//!   with one that records each record as `LEVEL: message` (Java's
//!   `SimpleFormatter` adds a timestamp and source line first,
//!   `WithNestedTestClass.java:146`); the suites only search that text.
//! * `zombieToken` is an `Object` compared by identity
//!   (`WithNestedTestClass.java:121`); here it is a generation number, `0`
//!   for `null`.

#![allow(dead_code)]

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::jdk::logging::{self, Level};
use randomizedtesting::jdk::reflect::{modifier, Class, Field, Method, Value};
use randomizedtesting::jdk::system::{self, PrintSink};
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::{
    After, AfterClass, Assume, Before, BeforeClass, ClassRule, Description, Failure, JUnitCore,
    JUnitResult, Rule, RuleChain, RunListener, Statement, TestRule,
};
use randomizedtesting::rules::{
    StatementAdapter, SystemPropertiesInvariantRule, TestRuleAdapter, TestRuleAdapterRule,
};
use randomizedtesting::RandomizedTest;

use crate::print_event_listener::PrintEventListener;
use crate::support::{self, cls, nested, test, unit, RANDOMIZED_TEST};

/// `runningNested`.
static RUNNING_NESTED: AtomicBool = AtomicBool::new(false);

/// `WithNestedTestClass.Place`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// `CLASS_RULE`.
    ClassRule,
    /// `BEFORE_CLASS`.
    BeforeClass,
    /// `CONSTRUCTOR`.
    Constructor,
    /// `TEST_RULE`.
    TestRule,
    /// `BEFORE`.
    Before,
    /// `TEST`.
    Test,
    /// `AFTER`.
    After,
    /// `AFTER_CLASS`.
    AfterClass,
}

/// `ApplyAtPlace.place`.
pub static APPLY_PLACE: Mutex<Option<Place>> = Mutex::new(None);
/// `ApplyAtPlace.runnable`.
pub static APPLY_RUNNABLE: Mutex<Option<Arc<dyn Fn() + Send + Sync>>> = Mutex::new(None);

/// `ApplyAtPlace.apply(p)`.
fn apply(p: Place) {
    if *APPLY_PLACE.lock().unwrap_or_else(|e| e.into_inner()) == Some(p) {
        assume_running_nested();
        let r = APPLY_RUNNABLE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(r) = r {
            r();
        }
    }
}

struct ApplyClassRule;
impl TestRuleAdapter for ApplyClassRule {
    fn before(&self) {
        apply(Place::ClassRule);
    }
}

struct ApplyTestRule;
impl TestRuleAdapter for ApplyTestRule {
    fn before(&self) {
        apply(Place::TestRule);
    }
}

/// `WithNestedTestClass.ApplyAtPlace extends RandomizedTest`: runs
/// `runnable` at `place` of a suite's lifecycle.
pub static APPLY_AT_PLACE: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested("WithNestedTestClass", "ApplyAtPlace")
        .extends(&RANDOMIZED_TEST)
        .constructor(randomizedtesting::jdk::reflect::Constructor::no_args(
            || apply(Place::Constructor),
        ))
        .field(
            Field::new_static("classRule", "org.junit.rules.TestRule", || {
                Value::TestRule(Arc::new(TestRuleAdapterRule(Arc::new(ApplyClassRule))))
            })
            .annotate(ClassRule),
        )
        .field(
            Field::new_instance::<()>("testRule", "org.junit.rules.TestRule", |_| {
                Value::TestRule(Arc::new(TestRuleAdapterRule(Arc::new(ApplyTestRule))))
            })
            .annotate(Rule),
        )
        .method(support::static_hook("beforeClass", BeforeClass, || {
            apply(Place::BeforeClass)
        }))
        .method(support::hook("before", Before, || apply(Place::Before)))
        .method(test("testMethod", || apply(Place::Test)))
        .method(support::hook("after", After, || apply(Place::After)))
        .method(support::static_hook("afterClass", AfterClass, || {
            apply(Place::AfterClass)
        }))
        .build()
});

/// The captured `System.out`/`System.err` text (`sw`).
static SW: Mutex<String> = Mutex::new(String::new());
/// The captured logging records (`loggingMessages`).
static LOGGING_MESSAGES: Mutex<String> = Mutex::new(String::new());
/// The streams `setupNested` replaced (`sysout`, `syserr`).
static SAVED: Mutex<Option<(Option<PrintSink>, Option<PrintSink>)>> = Mutex::new(None);
/// The capture handler's key.
static HANDLER: Mutex<Option<u64>> = Mutex::new(None);
/// `zombies`.
static ZOMBIES: Mutex<Vec<Thread>> = Mutex::new(Vec::new());
/// `zombieToken` (`0` for `null`).
static ZOMBIE_TOKEN: AtomicU64 = AtomicU64::new(0);
static ZOMBIE_GENERATION: AtomicU64 = AtomicU64::new(0);

/// `dumpLoggerOutputOnFailure`: on failure, prints the description and the
/// captured logging to the saved `sysout`.
struct DumpLoggerOutputOnFailure;

impl TestRule for DumpLoggerOutputOnFailure {
    fn apply(&self, base: Statement, description: &Arc<Description>) -> Statement {
        let d = Arc::clone(description);
        StatementAdapter::new(base)
            .after_always(move |errors| {
                if !errors.is_empty() {
                    sysout_println(&format!("-- {d}"));
                    let logged = LOGGING_MESSAGES
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .clone();
                    sysout_println(&logged);
                }
            })
            .build()
    }
}

/// `ruleChain`: `RuleChain.outerRule(noLeftOverProperties).around(dumpLoggerOutputOnFailure)`.
fn rule_chain() -> Value {
    let no_left_over_properties = SystemPropertiesInvariantRule::ignoring(&["user.timezone"]);
    Value::TestRule(Arc::new(
        RuleChain::outer_rule(Arc::new(no_left_over_properties))
            .around(Arc::new(DumpLoggerOutputOnFailure)),
    ))
}

/// `setupNested()`.
fn setup_nested() {
    RUNNING_NESTED.store(true, Ordering::SeqCst);
    ZOMBIE_TOKEN.store(
        ZOMBIE_GENERATION.fetch_add(1, Ordering::SeqCst) + 1,
        Ordering::SeqCst,
    );
    SW.lock().unwrap_or_else(|e| e.into_inner()).clear();
    let out_prev = system::set_out(None);
    let err_prev = system::set_err(None);
    let tee = |prev: Option<PrintSink>, err: bool| -> PrintSink {
        Arc::new(move |text: &str| {
            match &prev {
                Some(p) => p(text),
                None => process_print(text, err),
            }
            SW.lock().unwrap_or_else(|e| e.into_inner()).push_str(text);
        })
    };
    let out_tee = tee(out_prev.clone(), false);
    let err_tee = tee(err_prev.clone(), true);
    *SAVED.lock().unwrap_or_else(|e| e.into_inner()) = Some((out_prev, err_prev));
    system::set_out(Some(out_tee));
    system::set_err(Some(err_tee));
    LOGGING_MESSAGES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
    let key = logging::add_handler(Arc::new(
        |_name: &str, level: Level, message: &str, thrown| {
            let mut b = format!("{level}: {message}");
            if let Some(t) = thrown {
                b.push('\n');
                b.push_str(&system::format_stack_trace(t));
            }
            b.push_str("\n\n");
            LOGGING_MESSAGES
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push_str(&b);
        },
    ));
    *HANDLER.lock().unwrap_or_else(|e| e.into_inner()) = Some(key);
}

/// `clearNested()`.
fn clear_nested() {
    ZOMBIE_TOKEN.store(0, Ordering::SeqCst);
    RUNNING_NESTED.store(false, Ordering::SeqCst);
    *APPLY_RUNNABLE.lock().unwrap_or_else(|e| e.into_inner()) = None;
    *APPLY_PLACE.lock().unwrap_or_else(|e| e.into_inner()) = None;
    if let Some((out, err)) = SAVED.lock().unwrap_or_else(|e| e.into_inner()).take() {
        system::set_out(out);
        system::set_err(err);
    }
    if let Some(k) = HANDLER.lock().unwrap_or_else(|e| e.into_inner()).take() {
        logging::remove_handler(k);
    }
    let zombies: Vec<Thread> =
        std::mem::take(&mut *ZOMBIES.lock().unwrap_or_else(|e| e.into_inner()));
    for t in &zombies {
        t.interrupt();
    }
    for t in &zombies {
        let _ = t.join();
    }
}

/// `ApplyAtPlace()`, the constructor a subclass's implicit `super()` runs.
pub fn apply_at_place_constructor() -> randomizedtesting::jdk::reflect::Constructor {
    randomizedtesting::jdk::reflect::Constructor::no_args(|| apply(Place::Constructor))
}

/// The `WithNestedTestClass` declaration the outer suites extend.
///
/// It carries `@ThreadLeakScope(Scope.NONE)`, which Java's class does not:
/// JUnit runs the outer suites (no `@RunWith`) under `BlockJUnit4ClassRunner`,
/// which has no thread-leak control, while the port runs them under
/// `RandomizedRunner` (the target's divergences), whose leak control would
/// otherwise see the threads the nested suites leave behind. The outer
/// suites Java runs under `RandomizedRunner` (`@RunWith`) restore the
/// default with `@ThreadLeakScope(Scope.TEST)`.
pub static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
    cls("WithNestedTestClass")
        .annotate(randomizedtesting::annotations::ThreadLeakScope {
            value: randomizedtesting::annotations::Scope::None,
        })
        .constructor(unit())
        .field(
            Field::new_static("ruleChain", "org.junit.rules.TestRule", rule_chain)
                .modifiers(modifier::PUBLIC | modifier::FINAL)
                .annotate(ClassRule),
        )
        .method(
            Method::new("setupNested")
                .modifiers(modifier::PUBLIC | modifier::FINAL)
                .throws(&["java.io.IOException"])
                .annotate(BeforeClass)
                .static_body(setup_nested),
        )
        .method(
            Method::new("clearNested")
                .modifiers(modifier::PUBLIC | modifier::FINAL)
                .throws(&["java.lang.Exception"])
                .annotate(AfterClass)
                .static_body(clear_nested),
        )
        .method(support::hook("after", After, || {
            // Reset zombie thread marker.
            randomizedtesting::RandomizedRunner::set_zombie_marker(false);
        }))
        .method(support::hook("before", Before, || {
            SW.lock().unwrap_or_else(|e| e.into_inner()).clear();
            LOGGING_MESSAGES
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clear();
        }))
        .build()
});

/// The process's own standard output (or error), which `System.out`
/// (`System.err`) is when no sink replaced it.
fn process_print(text: &str, err: bool) {
    use std::io::Write;
    if err {
        let mut e = std::io::stderr().lock();
        let _ = e.write_all(text.as_bytes());
        let _ = e.flush();
    } else {
        let mut o = std::io::stdout().lock();
        let _ = o.write_all(text.as_bytes());
        let _ = o.flush();
    }
}

/// `sysout.println(text)`: the standard output `setupNested` replaced.
pub fn sysout_println(text: &str) {
    let saved = SAVED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|(o, _)| o.clone());
    match saved {
        Some(s) => s(&format!("{text}\n")),
        None => process_print(&format!("{text}\n"), false),
    }
}

/// `getSysouts()`.
pub fn get_sysouts() -> String {
    SW.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// `getLoggingMessages()`.
pub fn get_logging_messages() -> String {
    LOGGING_MESSAGES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// `isRunningNested()`.
pub fn is_running_nested() -> bool {
    RUNNING_NESTED.load(Ordering::SeqCst)
}

/// `assumeRunningNested()`: `Assume.assumeTrue(runningNested)`.
pub fn assume_running_nested() {
    Assume::assume_true(RUNNING_NESTED.load(Ordering::SeqCst));
}

/// `startZombieThread(name)`: a thread that ignores interrupts until
/// `clearNested` clears the token it captured.
pub fn start_zombie_thread(name: &str) -> Thread {
    let latch = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
    let l2 = Arc::clone(&latch);
    let token = ZOMBIE_TOKEN.load(Ordering::SeqCst);
    let t = Thread::new(name, move || {
        {
            let (m, c) = &*l2;
            *m.lock().unwrap_or_else(|e| e.into_inner()) = true;
            c.notify_all();
        }
        while ZOMBIE_TOKEN.load(Ordering::SeqCst) == token {
            let _ = Thread::try_sleep(1000);
        }
    });
    t.start();
    ZOMBIES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(t.clone());
    let (m, c) = &*latch;
    let mut g = m.lock().unwrap_or_else(|e| e.into_inner());
    while !*g {
        g = c.wait(g).unwrap_or_else(|e| e.into_inner());
    }
    t
}

/// `startThread(name)`: a thread that sleeps forever
/// (`RandomizedTest.sleep(1000)` in a loop, so an interrupt ends it with
/// the `InterruptedException` that `sleep` rethrows).
pub fn start_thread(name: &str) -> Thread {
    let latch = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
    let l2 = Arc::clone(&latch);
    let t = Thread::new(name, move || {
        {
            let (m, c) = &*l2;
            *m.lock().unwrap_or_else(|e| e.into_inner()) = true;
            c.notify_all();
        }
        loop {
            RandomizedTest::sleep(1000);
        }
    });
    t.start();
    let (m, c) = &*latch;
    let mut g = m.lock().unwrap_or_else(|e| e.into_inner());
    while !*g {
        g = c.wait(g).unwrap_or_else(|e| e.into_inner());
    }
    t
}

/// `WithNestedTestClass.FullResult`.
pub struct FullResult {
    assumption_ignored: Arc<AtomicUsize>,
    result: Arc<JUnitResult>,
}

impl FullResult {
    /// `getRunCount()`.
    pub fn get_run_count(&self) -> usize {
        self.result.run_count()
    }
    /// `getIgnoreCount()`.
    pub fn get_ignore_count(&self) -> usize {
        self.result.ignore_count()
    }
    /// `getFailureCount()`.
    pub fn get_failure_count(&self) -> usize {
        self.result.failure_count()
    }
    /// `getAssumptionIgnored()`.
    pub fn get_assumption_ignored(&self) -> usize {
        self.assumption_ignored.load(Ordering::SeqCst)
    }
    /// `getFailures()`.
    pub fn get_failures(&self) -> Vec<Failure> {
        self.result.failures()
    }
    /// `wasSuccessful()`.
    pub fn was_successful(&self) -> bool {
        self.result.was_successful()
    }
}

struct AssumptionCounter(Arc<AtomicUsize>);

impl RunListener for AssumptionCounter {
    fn test_assumption_failure(&self, _failure: &Failure) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

/// `runTests(classes…)`: runs the classes in one JUnit run on a separate
/// thread ("so that it appears as we're not running in an IDE").
pub fn run_tests(classes: &[Arc<Class>]) -> FullResult {
    let classes: Vec<Arc<Class>> = classes.to_vec();
    let counter = Arc::new(AtomicUsize::new(0));
    let c2 = Arc::clone(&counter);
    let out: Arc<Mutex<Option<Arc<JUnitResult>>>> = Arc::new(Mutex::new(None));
    let o2 = Arc::clone(&out);
    let t = Thread::unnamed(move || {
        let mut core = JUnitCore::new();
        core.add_listener(Arc::new(PrintEventListener::new()));
        core.add_listener(Arc::new(AssumptionCounter(c2)));
        let r = core.run_all(&classes);
        *o2.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
    });
    t.start();
    if let Err(p) = t.join() {
        std::panic::resume_unwind(p);
    }
    let result = out
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
        .expect("the run thread stores its result before it ends");
    FullResult {
        assumption_ignored: counter,
        result,
    }
}

/// `checkTestsOutput(run, ignored, failures, assumptions, classes)`.
pub fn check_tests_output(
    run: usize,
    ignored: usize,
    failures: usize,
    assumptions: usize,
    class: &Arc<Class>,
) -> FullResult {
    let result = run_tests(std::slice::from_ref(class));
    if result.get_run_count() != run
        || result.get_ignore_count() != ignored
        || result.get_failure_count() != failures
        || result.get_assumption_ignored() != assumptions
    {
        support::assertj_fail_msg(&format!(
            "Different result. [run,ign,fail,ass] Expected: {run},{ignored},{failures},{assumptions}, Actual: {},{},{},{}",
            result.get_run_count(),
            result.get_ignore_count(),
            result.get_failure_count(),
            result.get_assumption_ignored()
        ));
    }
    result
}
