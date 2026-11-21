//! Port of `com.carrotsearch.randomizedtesting.timeouts.Test003ThreadLeaksMethod`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::sync::{Arc, LazyLock};

use randomizedtesting::annotations::{
    Action, Scope, ThreadLeakAction, ThreadLeakLingering, ThreadLeakScope,
};
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::LifecycleScope;

use super::common::{apply_at_place_suite, cls, place_tests, set_place, ALL_PLACES};
use crate::support::{assert_equals, assert_that_str, unit};
use crate::utils::{
    assert_failure_with_message, assert_failures_contain_seeds, assert_no_live_threads_containing,
};
use crate::with_nested_test_class::{
    get_logging_messages, run_tests, start_thread, Place, CLASS as WITH_NESTED,
};

const O: &str = "Test003ThreadLeaksMethod";

static NESTED: LazyLock<Arc<Class>> = LazyLock::new(|| {
    apply_at_place_suite(O, |b| {
        b.annotate(ThreadLeakScope { value: Scope::Test })
            .annotate(ThreadLeakLingering { linger: 0 })
            .annotate(ThreadLeakAction {
                value: vec![Action::Warn, Action::Interrupt],
            })
    })
});

fn check(p: Place) {
    let scope = match p {
        Place::ClassRule | Place::BeforeClass | Place::Constructor | Place::AfterClass => {
            LifecycleScope::Suite
        }
        _ => LifecycleScope::Test,
    };
    set_place(p, || {
        start_thread("foobar");
    });
    let r = run_tests(std::slice::from_ref(&*NESTED));
    assert_failure_with_message(&r, &format!("1 thread leaked from {scope} scope at"));
    assert_equals(1, r.get_failure_count());
    assert_failures_contain_seeds(&r);
    assert_no_live_threads_containing("foobar");
    assert_that_str(get_logging_messages()).does_not_contain("Uncaught exception");
}

/// `Test003ThreadLeaksMethod extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![place_tests(
        cls(O).extends(&WITH_NESTED).constructor(unit()),
        &ALL_PLACES,
        check,
    )
    .build()]
}
