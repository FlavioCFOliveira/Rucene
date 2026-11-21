//! Port of `com.carrotsearch.randomizedtesting.TestExpected`
//! (randomized-runner 2.8.1 test sources, Apache-2.0): `@Test(expected)`.

use std::sync::{Arc, LazyLock};

use randomizedtesting::jdk::exception::RUNTIME_EXCEPTION;
use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::Test;

use crate::support::{
    assert_instance_of, cls, nested, test, throw_no_message, unit, ERROR, RANDOMIZED_TEST,
};
use crate::with_nested_test_class::{
    assume_running_nested, check_tests_output, CLASS as WITH_NESTED,
};

static NESTED1: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested("TestExpected", "Nested1")
        .extends(&RANDOMIZED_TEST)
        .constructor(unit())
        .method(
            Method::new("testMethod1")
                .annotate(Test::expected(RUNTIME_EXCEPTION))
                .body_any(|| throw_no_message(RUNTIME_EXCEPTION)),
        )
        // We expect a RuntimeException but get an error: should fail.
        .method(
            Method::new("testMethod2")
                .annotate(Test::expected(RUNTIME_EXCEPTION))
                .body_any(|| {
                    assume_running_nested();
                    throw_no_message(ERROR)
                }),
        )
        .build()
});

static NESTED2: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested("TestExpected", "Nested2")
        .extends(&RANDOMIZED_TEST)
        .constructor(unit())
        .method(
            Method::new("testMethod1")
                .annotate(Test::expected(RUNTIME_EXCEPTION))
                .body_any(assume_running_nested),
        )
        .build()
});

/// `TestExpected extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls("TestExpected")
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(test("testExpectedFailureDifferentException", || {
            let f = check_tests_output(2, 0, 1, 0, &NESTED1);
            assert_instance_of(f.get_failures()[0].exception(), ERROR);
        }))
        .method(test("testExpectedFailurePassed", || {
            check_tests_output(1, 0, 1, 0, &NESTED2);
        }))
        .build()]
}
