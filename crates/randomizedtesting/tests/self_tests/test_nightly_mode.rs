//! Port of `com.carrotsearch.randomizedtesting.TestNightlyMode`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::sync::{Arc, LazyLock};

use randomizedtesting::annotations::{Nightly, TestGroupUtilities, NIGHTLY};
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::system;
use randomizedtesting::junit::{After, Before};
use randomizedtesting::RandomizedTest;

use crate::support::{assert_true, cls, hook, nested, test, unit, RANDOMIZED_TEST};
use crate::with_nested_test_class::{
    assume_running_nested, check_tests_output, CLASS as WITH_NESTED,
};

static NESTED: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested("TestNightlyMode", "Nested")
        .extends(&RANDOMIZED_TEST)
        .constructor(unit())
        .method(test("passOnNightlyOnly", || {
            assume_running_nested();
            assert_true(RandomizedTest::is_nightly());
        }))
        .method(test("nightlyOnly", || {}).annotate(Nightly {
            value: "Nightly only test case.".into(),
        }))
        .build()
});

fn nightly_property() -> String {
    TestGroupUtilities::get_sys_property(&NIGHTLY)
}

fn daily_default() {
    check_tests_output(2, 0, 1, 1, &NESTED);
}

fn cleanup_after() {
    system::clear_property(&nightly_property());
}

/// `TestNightlyMode extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls("TestNightlyMode")
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(test("invalidValueNightly", || {
            system::set_property(&nightly_property(), "invalid-value");
            daily_default();
        }))
        .method(test("nightly", || {
            system::set_property(&nightly_property(), "yes");
            check_tests_output(2, 0, 0, 0, &NESTED);
        }))
        .method(test("dailyDefault", daily_default))
        .method(hook("cleanupBefore", Before, cleanup_after))
        .method(hook("cleanupAfter", After, cleanup_after))
        .build()]
}
