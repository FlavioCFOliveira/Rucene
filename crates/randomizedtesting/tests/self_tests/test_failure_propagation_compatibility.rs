//! Port of `com.carrotsearch.randomizedtesting.TestFailurePropagationCompatibility`
//! (randomized-runner 2.8.1 test sources, Apache-2.0): failures propagate as
//! under JUnit's runner.
//!
//! **Divergence.** The test runs the same randomly failing suite under
//! JUnit's `BlockJUnit4ClassRunner` (`FailRandomly2`, no `@RunWith`) and
//! under `RandomizedRunner` (`FailRandomly3`) and compares the two traces
//! (`TestFailurePropagationCompatibility.java:111`). The port has no JUnit
//! runner (`Request::a_class`): both run under `RandomizedRunner`, so the
//! comparison holds by construction and the test checks only that the
//! port's propagation is deterministic for a seed.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::jdk::exception::{Throwable, RUNTIME_EXCEPTION};
use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::reflect::{Class, Constructor, Field, Value};
use randomizedtesting::jdk::system;
use randomizedtesting::junit::{
    After, AfterClass, Before, BeforeClass, ClassRule, Description, JUnitCore, JUnitResult, Rule,
    RuleChain, Statement, TestRule,
};

use crate::support::{cls, fail, hook, nested, static_hook, test, unit};
use crate::with_nested_test_class::CLASS as WITH_NESTED;

const O: &str = "TestFailurePropagationCompatibility";

/// `static Random random`.
static RANDOM: Mutex<Option<Arc<JdkRandom>>> = Mutex::new(None);
/// `static int frequency`.
static FREQUENCY: AtomicI32 = AtomicI32::new(1);

/// `maybeFail()`.
fn maybe_fail() {
    let r = RANDOM.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if let Some(random) = r {
        if random.next_int_bound(FREQUENCY.load(Ordering::SeqCst)) == 0 {
            if random.next_boolean() {
                Throwable::new(RUNTIME_EXCEPTION, format!("State: {}", random.next_long())).throw();
            } else {
                let errors = vec![
                    Throwable::new(RUNTIME_EXCEPTION, format!("State: {}", random.next_long())),
                    Throwable::new(RUNTIME_EXCEPTION, format!("State: {}", random.next_long())),
                ];
                // Throw MultipleFailureException as if unchecked.
                Throwable::multiple_failures_of_class(
                    "org.junit.runners.model.MultipleFailureException",
                    errors,
                )
                .throw();
            }
        }
    }
}

/// `MaybeFailRule implements TestRule`.
struct MaybeFailRule;

impl TestRule for MaybeFailRule {
    fn apply(&self, base: Statement, _d: &Arc<Description>) -> Statement {
        Statement::new(move || {
            let body = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                maybe_fail();
                base.evaluate();
                maybe_fail();
            }));
            // catch (Throwable t) { maybeFail(); } finally { maybeFail(); }
            let caught = match body {
                Ok(()) => Ok(()),
                Err(_) => std::panic::catch_unwind(std::panic::AssertUnwindSafe(maybe_fail)),
            };
            maybe_fail();
            if let Err(p) = caught {
                std::panic::resume_unwind(p);
            }
        })
    }
}

fn chain() -> Value {
    Value::TestRule(Arc::new(
        RuleChain::outer_rule(Arc::new(MaybeFailRule)).around(Arc::new(MaybeFailRule)),
    ))
}

static FAIL_RANDOMLY1: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "FailRandomly1")
        .constructor(Constructor::no_args(maybe_fail))
        .field(
            Field::new_static("classRule1", "org.junit.rules.TestRule", chain).annotate(ClassRule),
        )
        .field(
            Field::new_instance::<()>("testRule1", "org.junit.rules.TestRule", |_| chain())
                .annotate(Rule),
        )
        .method(static_hook("beforeClass1", BeforeClass, maybe_fail))
        .method(hook("before", Before, maybe_fail))
        .method(test("test", maybe_fail))
        .method(hook("after", After, maybe_fail))
        .method(static_hook("afterClass", AfterClass, maybe_fail))
        .build()
});

fn fail_randomly2(name: &str, superclass: &Arc<Class>) -> Arc<Class> {
    nested(O, name)
        .extends(superclass)
        .constructor(Constructor::no_args(|| {
            maybe_fail(); // super()
            maybe_fail();
        }))
        .field(
            Field::new_static("classRule2", "org.junit.rules.TestRule", chain).annotate(ClassRule),
        )
        .field(
            Field::new_instance::<()>("testRule2", "org.junit.rules.TestRule", |_| chain())
                .annotate(Rule),
        )
        .method(static_hook("beforeClass2", BeforeClass, maybe_fail))
        .method(hook("before2", Before, maybe_fail))
        .method(hook("after2", After, maybe_fail))
        .method(static_hook("afterClass2", AfterClass, maybe_fail))
        .build()
}

static FAIL_RANDOMLY2: LazyLock<Arc<Class>> =
    LazyLock::new(|| fail_randomly2("FailRandomly2", &FAIL_RANDOMLY1));

/// `@RunWith(RandomizedRunner.class) FailRandomly3 extends FailRandomly2`.
static FAIL_RANDOMLY3: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "FailRandomly3")
        .extends(&FAIL_RANDOMLY2)
        .constructor(Constructor::no_args(|| {
            maybe_fail(); // FailRandomly1()
            maybe_fail(); // FailRandomly2()
        }))
        .build()
});

fn execution_trace(r: &JUnitResult) -> String {
    let mut sb = format!(
        "Run count: {}\nIgnore count: {}\nFailure count: {}\n",
        r.run_count(),
        r.ignore_count(),
        r.failure_count()
    );
    for f in r.failures() {
        let t = f.exception();
        sb.push_str(&format!(
            "{}: {}\n\n",
            t.class_name(),
            t.message().unwrap_or("null")
        ));
    }
    sb
}

/// `TestFailurePropagationCompatibility extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls(O)
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(test("testRunEquals", || {
            let rnd = JdkRandom::new_unseeded(); // initial.
            let mut f = 1;
            while f < 10 {
                FREQUENCY.store(f, Ordering::SeqCst);
                for i in 0..25 {
                    let seed = rnd.next_long().wrapping_add(i);
                    *RANDOM.lock().unwrap_or_else(|e| e.into_inner()) =
                        Some(Arc::new(JdkRandom::new(seed)));
                    let junit4 = JUnitCore::new().run_class(&FAIL_RANDOMLY2);
                    *RANDOM.lock().unwrap_or_else(|e| e.into_inner()) =
                        Some(Arc::new(JdkRandom::new(seed)));
                    let rr = JUnitCore::new().run_class(&FAIL_RANDOMLY3);
                    let trace_junit4 = execution_trace(&junit4);
                    let trace_rr = execution_trace(&rr);
                    if trace_junit4 != trace_rr {
                        system::out_println(&format!("=== Random({seed}), freq: {f}"));
                        system::out_println("--- JUnit4:");
                        system::out_println(&trace_junit4);
                        system::out_println("--- RandomizedRunner:");
                        system::out_println(&trace_rr);
                        fail();
                    }
                }
                f += 2;
            }
        }))
        .method(static_hook("cleanup", AfterClass, || {
            *RANDOM.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }))
        .build()]
}
