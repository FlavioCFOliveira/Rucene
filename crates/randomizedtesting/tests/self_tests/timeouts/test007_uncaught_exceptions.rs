//! Port of `com.carrotsearch.randomizedtesting.timeouts.Test007UncaughtExceptions`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::jdk::exception::RUNTIME_EXCEPTION;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::{Thread, ThreadGroup};

use super::common::{cls, nested};
use crate::support::{assert_that_str, test, throw, unit, RANDOMIZED_TEST};
use crate::utils::{
    assert_failure_with_message, assert_failures_contain_seeds, assert_no_live_threads_containing,
};
use crate::with_nested_test_class::{
    assume_running_nested, get_logging_messages, run_tests, CLASS as WITH_NESTED,
};

const O: &str = "Test007UncaughtExceptions";

/// `static ThreadGroup parentGroup` (`None` for `null`).
static PARENT_GROUP: Mutex<Option<Arc<ThreadGroup>>> = Mutex::new(None);

static NESTED: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Nested")
        .extends(&RANDOMIZED_TEST)
        .constructor(unit())
        .method(test("checkGlobal", || {
            assume_running_nested();
            let body = || throw(RUNTIME_EXCEPTION, "Yoda died.");
            // new Thread(parentGroup, "XYZ"): a null group is the current thread's.
            let t = match PARENT_GROUP
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone()
            {
                Some(g) => Thread::with_group(&g, "XYZ", body),
                None => Thread::new("XYZ", body),
            };
            t.start();
            let _ = t.join();
        }))
        .build()
});

fn check() {
    let r = run_tests(std::slice::from_ref(&*NESTED));
    assert_failure_with_message(&r, "Captured an uncaught exception in thread: ");
    assert_failure_with_message(&r, "Yoda died.");
    assert_failures_contain_seeds(&r);
    assert_no_live_threads_containing("XYZ");
    assert_that_str(get_logging_messages())
        .contains("Uncaught exception")
        .contains("Yoda died.");
}

/// `Test007UncaughtExceptions extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls(O)
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(test("testUncaughtExceptionsAtMainGroup", || {
            let mut parent_tg = Thread::current().thread_group().expect("a thread group");
            while let Some(p) = parent_tg.parent().cloned() {
                parent_tg = p;
            }
            *PARENT_GROUP.lock().unwrap_or_else(|e| e.into_inner()) = Some(parent_tg);
            check();
        }))
        .method(test("testUncaughtExceptionsAtThreadGroup", || {
            *PARENT_GROUP.lock().unwrap_or_else(|e| e.into_inner()) = None;
            check();
        }))
        .build()]
}
