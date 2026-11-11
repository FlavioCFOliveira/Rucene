//! Port of `com.carrotsearch.randomizedtesting.TestIgnoredRunCount`
//! (randomized-runner 2.8.1 test sources, Apache-2.0): `Result`'s run count
//! for ignored tests.

use std::sync::{Arc, LazyLock};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Ignore;

use crate::support::{cls, nested, test, unit};
use crate::with_nested_test_class::{check_tests_output, CLASS as WITH_NESTED};

static NESTED1: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested("TestIgnoredRunCount", "Nested1")
        .constructor(unit())
        .method(test("ignored", || {}).annotate(Ignore::new()))
        .build()
});

/// `@RunWith(RandomizedRunner.class) Nested2`.
static NESTED2: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested("TestIgnoredRunCount", "Nested2")
        .constructor(unit())
        .method(test("ignored", || {}).annotate(Ignore::new()))
        .build()
});

/// `TestIgnoredRunCount extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls("TestIgnoredRunCount")
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(test("checkIgnoredCount", || {
            check_tests_output(0, 1, 0, 0, &NESTED1);
            check_tests_output(0, 1, 0, 0, &NESTED2);
        }))
        .build()]
}
