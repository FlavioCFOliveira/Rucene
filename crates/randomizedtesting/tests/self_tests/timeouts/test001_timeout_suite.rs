//! Port of `com.carrotsearch.randomizedtesting.timeouts.Test001TimeoutSuite`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::sync::{Arc, LazyLock};

use randomizedtesting::annotations::{Timeout, TimeoutSuite};
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::RandomizedTest;

use super::common::{apply_at_place_suite, cls, place_tests, set_place, ALL_PLACES};
use crate::support::{assert_that_str, unit};
use crate::utils::{assert_failure_with_message, assert_failures_contain_seeds};
use crate::with_nested_test_class::{get_logging_messages, run_tests, Place, CLASS as WITH_NESTED};

const O: &str = "Test001TimeoutSuite";

/// `@TimeoutSuite(millis = 500) @Timeout(millis = 5000) Nested extends ApplyAtPlace`.
static NESTED: LazyLock<Arc<Class>> = LazyLock::new(|| {
    apply_at_place_suite(O, |b| {
        b.annotate(TimeoutSuite { millis: 500 })
            .annotate(Timeout { millis: 5000 })
    })
});

fn check(p: Place) {
    set_place(p, || loop {
        RandomizedTest::sleep(10000)
    });
    let r = run_tests(std::slice::from_ref(&*NESTED));
    assert_failure_with_message(&r, "Suite timeout exceeded");
    assert_failures_contain_seeds(&r);
    assert_that_str(get_logging_messages()).does_not_contain("Test execution timed out");
}

/// `Test001TimeoutSuite extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![place_tests(
        cls(O).extends(&WITH_NESTED).constructor(unit()),
        &ALL_PLACES,
        check,
    )
    .build()]
}
