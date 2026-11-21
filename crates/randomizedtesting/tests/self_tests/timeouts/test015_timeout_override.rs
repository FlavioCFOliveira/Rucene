//! Port of `com.carrotsearch.randomizedtesting.timeouts.Test015TimeoutOverride`
//! (randomized-runner 2.8.1 test sources, Apache-2.0): the global timeout
//! override (`-Dtests.timeout=1000!`).

use std::sync::{Arc, LazyLock};

use randomizedtesting::annotations::Timeout;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::system;
use randomizedtesting::junit::After;
use randomizedtesting::{RandomizedTest, SysGlobals};

use super::common::{cls, nested};
use crate::support::{assert_equals, assert_true, hook, test, unit, RANDOMIZED_TEST};
use crate::with_nested_test_class::{assume_running_nested, run_tests, CLASS as WITH_NESTED};

const O: &str = "Test015TimeoutOverride";

static NESTED: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Nested")
        .extends(&RANDOMIZED_TEST)
        .constructor(unit())
        .method(
            test("testMethod1", || {
                assume_running_nested();
                RandomizedTest::sleep(10000);
            })
            .annotate(Timeout { millis: 5000 }),
        )
        .build()
});

static NESTED2: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Nested2")
        .extends(&RANDOMIZED_TEST)
        .constructor(unit())
        .method(
            test("testMethod1", || {
                assume_running_nested();
                RandomizedTest::sleep(1000);
            })
            .annotate(Timeout { millis: 100 }),
        )
        .build()
});

fn elapsed_ms(f: impl FnOnce()) -> i64 {
    let start = system::nano_time();
    f();
    (system::nano_time() - start) / 1_000_000
}

/// `Test015TimeoutOverride extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls(O)
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(test("testTimeoutOverride", || {
            system::set_property(&SysGlobals::sysprop_timeout(), "200!");
            let mut failures = 0;
            let ms = elapsed_ms(|| {
                failures = run_tests(std::slice::from_ref(&*NESTED)).get_failure_count()
            });
            assert_equals(1, failures);
            assert_true(ms < 3000);
        }))
        .method(test("testDisableTimeout", || {
            system::set_property(&SysGlobals::sysprop_timeout(), "0!");
            let mut failures = 1;
            let ms = elapsed_ms(|| {
                failures = run_tests(std::slice::from_ref(&*NESTED2)).get_failure_count()
            });
            assert_equals(0, failures);
            assert_true(ms > 900);
        }))
        .method(hook("cleanup", After, || {
            system::clear_property(&SysGlobals::sysprop_timeout());
        }))
        .build()]
}
