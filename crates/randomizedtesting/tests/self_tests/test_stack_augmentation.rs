//! Port of `com.carrotsearch.randomizedtesting.TestStackAugmentation`
//! (randomized-runner 2.8.1 test sources, Apache-2.0): failures carry their
//! seed in an augmented stack trace.

use std::sync::{Arc, LazyLock};

use randomizedtesting::annotations::Seed;
use randomizedtesting::jdk::exception::{Throwable, RUNTIME_EXCEPTION};
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{AfterClass, BeforeClass};
use randomizedtesting::seed_from_throwable;

use crate::support::{
    assert_equals, assert_not_null, assert_true, assert_true_msg, cls, nested, static_hook, test,
    throw, unit, ERROR,
};
use crate::with_nested_test_class::{
    assume_running_nested, check_tests_output, CLASS as WITH_NESTED,
};

const O: &str = "TestStackAugmentation";

/// `@RunWith(RandomizedRunner.class) @Seed("deadbeef") Nested`.
static NESTED: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Nested")
        .annotate(Seed::new("deadbeef"))
        .constructor(unit())
        .method(
            test("testMethod1", || {
                assume_running_nested();
                // Throws a chained exception.
                Throwable::with_cause(
                    ERROR,
                    Some("Outer.".into()),
                    Throwable::new(RUNTIME_EXCEPTION, "Inner."),
                )
                .throw()
            })
            .annotate(Seed::new("cafebabe")),
        )
        .build()
});

static NESTED2: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Nested2")
        .annotate(Seed::new("deadbeef"))
        .constructor(unit())
        .method(static_hook("beforeClass", BeforeClass, || {
            assume_running_nested();
            throw(ERROR, "beforeclass.");
        }))
        .method(test("testMethod1", || {}).annotate(Seed::new("cafebabe")))
        .build()
});

static NESTED3: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Nested3")
        .annotate(Seed::new("deadbeef"))
        .constructor(unit())
        .method(static_hook("afterClass", AfterClass, || {
            assume_running_nested();
            throw(ERROR, "afterclass.");
        }))
        .method(test("testMethod1", || {}).annotate(Seed::new("cafebabe")))
        .build()
});

/// `String.compareToIgnoreCase(other) == 0`.
fn equals_ignore_case(a: &str, b: &str) -> bool {
    a.to_uppercase().to_lowercase() == b.to_uppercase().to_lowercase()
}

/// `TestStackAugmentation extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls(O)
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(test("testMethodLevel", || {
            let result = check_tests_output(1, 0, 1, 0, &NESTED);
            let f = &result.get_failures()[0];
            let seed = assert_not_null(seed_from_throwable(f.exception()));
            assert_true(equals_ignore_case("[DEADBEEF:CAFEBABE]", &seed));
        }))
        .method(test("testBeforeClass", || {
            let result = check_tests_output(0, 0, 1, 0, &NESTED2);
            assert_equals(1, result.get_failure_count());
            let f = &result.get_failures()[0];
            let seed = assert_not_null(seed_from_throwable(f.exception()));
            assert_true_msg(&f.trace(), equals_ignore_case("[DEADBEEF]", &seed));
        }))
        .method(test("testAfterClass", || {
            let result = check_tests_output(1, 0, 1, 0, &NESTED3);
            let f = &result.get_failures()[0];
            let seed = assert_not_null(seed_from_throwable(f.exception()));
            assert_true_msg(&f.trace(), equals_ignore_case("[DEADBEEF]", &seed));
        }))
        .build()]
}
