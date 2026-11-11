//! Port of `com.carrotsearch.randomizedtesting.timeouts.Test020SuiteTimeoutStopsTests`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, LazyLock};

use randomizedtesting::annotations::{Scope, ThreadLeakLingering, ThreadLeakScope, TimeoutSuite};
use randomizedtesting::jdk::reflect::{modifier, Class, Method};
use randomizedtesting::jdk::system;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::BeforeClass;

use super::common::{cls, nested};
use crate::support::{assert_that_str, test, unit, RANDOMIZED_TEST};
use crate::with_nested_test_class::{
    assume_running_nested, get_logging_messages, get_sysouts, run_tests, sysout_println,
    CLASS as WITH_NESTED,
};

const O: &str = "Test020SuiteTimeoutStopsTests";

/// `private static AtomicInteger executedTests`.
static EXECUTED_TESTS: AtomicI32 = AtomicI32::new(0);

/// `idle(m)`: the first test idles until the suite timeout interrupts it.
fn idle(m: &str) {
    if EXECUTED_TESTS.fetch_add(1, Ordering::SeqCst) == 0 {
        system::out_println(&format!("before timeout({m})"));
        loop {
            if Thread::try_sleep(1000).is_err() {
                system::out_println(&format!("suite timeout({m})"));
                // Suite timeout.
                return;
            }
        }
    } else {
        system::out_println(&format!("after timeout({m})"));
    }
}

static NESTED1: LazyLock<Arc<Class>> = LazyLock::new(|| {
    let mut b = nested(O, "Nested1")
        .extends(&RANDOMIZED_TEST)
        .annotate(ThreadLeakScope {
            value: Scope::Suite,
        })
        .annotate(TimeoutSuite { millis: 500 })
        .annotate(ThreadLeakLingering { linger: 0 })
        .constructor(unit());
    for (name, m) in [
        ("test001", "1"),
        ("test002", "2"),
        ("test003", "3"),
        ("test004", "4"),
        ("test005", "5"),
        ("test006", "6"),
    ] {
        b = b.method(test(name, move || idle(m)));
    }
    b.method(
        Method::new("setup")
            .modifiers(modifier::PRIVATE)
            .annotate(BeforeClass)
            .static_body(|| {
                assume_running_nested();
                EXECUTED_TESTS.store(0, Ordering::SeqCst);
            }),
    )
    .build()
});

/// `Test020SuiteTimeoutStopsTests extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls(O)
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(test("testExceptionInFilter", || {
            run_tests(std::slice::from_ref(&*NESTED1));
            assert_that_str(get_sysouts()).does_not_contain("after timeout");
            sysout_println(&get_logging_messages());
        }))
        .build()]
}
