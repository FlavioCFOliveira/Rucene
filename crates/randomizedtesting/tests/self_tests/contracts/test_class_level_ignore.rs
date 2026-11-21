//! Port of `com.carrotsearch.randomizedtesting.contracts.TestClassLevelIgnore`
//! (randomized-runner 2.8.1 test sources, Apache-2.0): class-level `@Ignore`.

use std::sync::{Arc, LazyLock};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Ignore;

use super::{cls, nested};
use crate::support::{test, unit, RANDOMIZED_TEST};
use crate::with_nested_test_class::{check_tests_output, CLASS as WITH_NESTED};

static NESTED: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested("TestClassLevelIgnore", "Nested")
        .extends(&RANDOMIZED_TEST)
        .annotate(Ignore::new())
        .constructor(unit())
        .method(test("ignored", || {}))
        .build()
});

/// `TestClassLevelIgnore extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls("TestClassLevelIgnore")
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(test("allIgnored", || {
            check_tests_output(0, 1, 0, 0, &NESTED);
        }))
        .build()]
}
