//! Port of `com.carrotsearch.randomizedtesting.timeouts.Test004ThreadLeaksSuite`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::sync::{Arc, LazyLock};

use randomizedtesting::annotations::{
    Action, Scope, ThreadLeakAction, ThreadLeakLingering, ThreadLeakScope,
};
use randomizedtesting::jdk::reflect::Class;

use super::common::{apply_at_place_suite, cls, place_tests, set_place, ALL_PLACES};
use crate::support::{assert_equals, assert_that_str, unit};
use crate::utils::{
    assert_failure_with_message, assert_failures_contain_seeds, assert_no_live_threads_containing,
};
use crate::with_nested_test_class::{
    get_logging_messages, run_tests, start_thread, Place, CLASS as WITH_NESTED,
};

const O: &str = "Test004ThreadLeaksSuite";

static NESTED: LazyLock<Arc<Class>> = LazyLock::new(|| {
    apply_at_place_suite(O, |b| {
        b.annotate(ThreadLeakScope {
            value: Scope::Suite,
        })
        .annotate(ThreadLeakLingering { linger: 0 })
        .annotate(ThreadLeakAction {
            value: vec![Action::Warn, Action::Interrupt],
        })
    })
});

fn suite_leak(p: Place) {
    set_place(p, || {
        start_thread("foobar");
    });
    let r = run_tests(std::slice::from_ref(&*NESTED));
    assert_failure_with_message(&r, "1 thread leaked from SUITE scope at");
    assert_equals(1, r.get_failure_count());
    assert_failures_contain_seeds(&r);
    assert_no_live_threads_containing("foobar");
    assert_that_str(get_logging_messages()).does_not_contain("Uncaught exception");
}

/// `Test004ThreadLeaksSuite extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![place_tests(
        cls(O).extends(&WITH_NESTED).constructor(unit()),
        &ALL_PLACES,
        suite_leak,
    )
    .build()]
}
