//! Port of `com.carrotsearch.randomizedtesting.timeouts.Test021SuiteTimeoutHanging`
//! (randomized-runner 2.8.1 test sources, Apache-2.0): a suite whose test
//! cannot be stopped is abandoned at the suite timeout, not joined forever.
//!
//! **Divergence.** `test001` holds the monitor of its own `Thread` object
//! (`synchronized (Thread.currentThread())`, `Test021SuiteTimeoutHanging.java:31`),
//! so that a `Thread.join` on it, which synchronizes on that monitor, would
//! block. Rust's threads have no monitor; the port's `test001` keeps the
//! rest (it ignores interrupts until told to stop), which is what makes the
//! runner abandon it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::annotations::{
    Action, Consequence, Scope, TestCaseOrdering, ThreadLeakAction, ThreadLeakLingering,
    ThreadLeakScope, ThreadLeakZombies, TimeoutSuite,
};
use randomizedtesting::jdk::exception::EXCEPTION;
use randomizedtesting::jdk::reflect::{modifier, Class, Method};
use randomizedtesting::jdk::system;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::BeforeClass;
use randomizedtesting::{DeadlineClock, SysGlobals};

use super::common::{cls, nested};
use crate::support::{
    assert_that_bool, assert_that_num, assert_that_str, test, throw_no_message, unit,
    RANDOMIZED_TEST,
};
use crate::with_nested_test_class::{
    assume_running_nested, run_tests, FullResult, CLASS as WITH_NESTED,
};

const O: &str = "Test021SuiteTimeoutHanging";

/// `private static AtomicBoolean stop`.
static STOP: AtomicBool = AtomicBool::new(false);
/// `private static ArrayList<Thread> waitQueue`.
static WAIT_QUEUE: Mutex<Vec<Thread>> = Mutex::new(Vec::new());

static NESTED1: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Nested1")
        .extends(&RANDOMIZED_TEST)
        .annotate(ThreadLeakScope {
            value: Scope::Suite,
        })
        .annotate(ThreadLeakLingering { linger: 0 })
        .annotate(ThreadLeakAction {
            value: vec![Action::Warn, Action::Interrupt],
        })
        .annotate(ThreadLeakZombies {
            value: Consequence::IgnoreRemainingTests,
        })
        .annotate(TimeoutSuite { millis: 1000 })
        .annotate(TestCaseOrdering::alphabetic_order())
        .constructor(unit())
        .method(test("test001", || {
            WAIT_QUEUE
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(Thread::current());
            while !STOP.load(Ordering::SeqCst) {
                if Thread::try_sleep(250).is_err() {
                    // If interrupted, just continue.
                    system::out_println("Interrupted.");
                }
                system::out_println("Still running.");
            }
        }))
        .method(test("test002", || {
            // Should not be executed.
            throw_no_message(EXCEPTION)
        }))
        .method(
            Method::new("setup")
                .modifiers(modifier::PRIVATE)
                .annotate(BeforeClass)
                .static_body(assume_running_nested),
        )
        .build()
});

/// `Test021SuiteTimeoutHanging extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls(O)
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(test("testThreadLeakInterruptsIsNotHangingOnJoin", || {
            STOP.store(false, Ordering::SeqCst);
            system::set_property(&SysGlobals::sysprop_killattempts(), "1");
            let result: Arc<Mutex<Option<FullResult>>> = Arc::new(Mutex::new(None));
            let r2 = Arc::clone(&result);
            let tester = Thread::unnamed(move || {
                WAIT_QUEUE
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(Thread::current());
                *r2.lock().unwrap_or_else(|e| e.into_inner()) =
                    Some(run_tests(std::slice::from_ref(&*NESTED1)));
            });
            tester.start();
            let deadline = DeadlineClock::millis(10_000);
            while deadline.is_before_deadline() && tester.is_alive() {
                Thread::sleep(250);
            }
            let tester_alive = tester.is_alive();
            // Wait for all threads to die.
            STOP.store(true, Ordering::SeqCst);
            let queue: Vec<Thread> = WAIT_QUEUE
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .drain(..)
                .collect();
            for t in queue {
                let _ = t.join();
            }
            system::clear_property(&SysGlobals::sysprop_killattempts());
            // The framework abandoned the thread it couldn't interrupt.
            assert_that_bool(tester_alive).is_false();
            let result = result
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take()
                .expect("the tester's result");
            // Only one test executed.
            assert_that_num(result.get_run_count() as i64).is_equal_to(1);
            // Make sure suite timeouts have been reported.
            for failure in result.get_failures() {
                assert_that_str(failure.message().unwrap_or("null").to_lowercase())
                    .contains("suite timeout");
            }
        }))
        .build()]
}
