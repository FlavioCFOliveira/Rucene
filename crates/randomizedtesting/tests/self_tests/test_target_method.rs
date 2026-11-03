//! Port of `com.carrotsearch.randomizedtesting.TestTargetMethod`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::sync::{Arc, LazyLock};

use randomizedtesting::annotations::Repeat;
use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::{AfterClass, Before, BeforeClass};
use randomizedtesting::RandomizedContext;

use crate::support::{
    assert_that_opt_str, assert_that_str, cls, hook, nested, test, unit, RANDOMIZED_TEST,
};
use crate::with_nested_test_class::{
    assume_running_nested, check_tests_output, CLASS as WITH_NESTED,
};

fn target_method() -> Option<String> {
    RandomizedContext::current().get_target_method()
}

static NESTED: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested("TestTargetMethod", "Nested")
        .extends(&RANDOMIZED_TEST)
        .constructor(unit())
        .method(hook("checkInHook", Before, assume_running_nested))
        .method(
            test("testOne", || {
                assert_that_str(target_method().unwrap_or_default()).is_equal_to("testOne");
            })
            .annotate(Repeat {
                iterations: 3,
                use_constant_seed: false,
            }),
        )
        .method(test("testTwo", || {
            assert_that_str(target_method().unwrap_or_default()).is_equal_to("testTwo");
        }))
        .method(
            Method::new("staticHooks")
                .annotate(AfterClass)
                .annotate(BeforeClass)
                .static_body(|| assert_that_opt_str(target_method()).is_null()),
        )
        .build()
});

/// `TestTargetMethod extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls("TestTargetMethod")
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(test("testTargetMethodAvailable", || {
            check_tests_output(4, 0, 0, 0, &NESTED);
        }))
        .build()]
}
