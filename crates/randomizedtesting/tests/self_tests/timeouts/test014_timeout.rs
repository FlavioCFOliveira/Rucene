//! Port of `com.carrotsearch.randomizedtesting.timeouts.Test014Timeout`
//! (randomized-runner 2.8.1 test sources, Apache-2.0): `@Test(timeout)`.

use std::sync::{Arc, LazyLock};

use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Test;
use randomizedtesting::RandomizedTest;

use super::common::{cls, nested};
use crate::support::{assert_equals, test, unit, RANDOMIZED_TEST};
use crate::with_nested_test_class::{assume_running_nested, run_tests, CLASS as WITH_NESTED};

const O: &str = "Test014Timeout";

static NESTED: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Nested")
        .extends(&RANDOMIZED_TEST)
        .constructor(unit())
        .method(
            Method::new("testMethod1")
                .annotate(Test::timeout(100))
                .body_any(|| {
                    assume_running_nested();
                    RandomizedTest::sleep(2000);
                }),
        )
        .method(
            Method::new("testMethod2")
                .annotate(Test::timeout(100))
                .body_any(|| {
                    assume_running_nested();
                    while !Thread::interrupted() {
                        // Do nothing.
                        std::hint::spin_loop();
                    }
                }),
        )
        .build()
});

/// `Test014Timeout extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls(O)
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(test("testTimeoutInTestAnnotation", || {
            let result = run_tests(std::slice::from_ref(&*NESTED));
            assert_equals(0, result.get_ignore_count());
            assert_equals(2, result.get_run_count());
            assert_equals(2, result.get_failure_count());
            assert_equals(0, result.get_assumption_ignored());
        }))
        .build()]
}
