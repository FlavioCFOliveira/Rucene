//! Port of `com.carrotsearch.randomizedtesting.timeouts.Test013ThreadLeaksScopeNone`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::sync::{Arc, LazyLock};

use randomizedtesting::annotations::{Scope, ThreadLeakScope};
use randomizedtesting::jdk::reflect::Class;

use super::common::{apply_at_place_suite, cls, place_tests, set_place, ALL_PLACES};
use crate::support::{assert_equals, unit};
use crate::with_nested_test_class::{
    get_logging_messages, get_sysouts, run_tests, start_zombie_thread, Place, CLASS as WITH_NESTED,
};

const O: &str = "Test013ThreadLeaksScopeNone";

static NESTED: LazyLock<Arc<Class>> = LazyLock::new(|| {
    apply_at_place_suite(O, |b| b.annotate(ThreadLeakScope { value: Scope::None }))
});

fn test_leak(p: Place) {
    set_place(p, || {
        start_zombie_thread("foobar");
    });
    let r = run_tests(std::slice::from_ref(&*NESTED));
    assert_equals(0, r.get_failure_count());
    is_empty(&get_logging_messages());
    is_empty(&get_sysouts());
}

/// `assertThat(text).isEmpty()`.
fn is_empty(s: &str) {
    if !s.is_empty() {
        crate::support::assertj_fail_msg(&format!("\nExpecting empty but was:<\"{s}\">"));
    }
}

/// `Test013ThreadLeaksScopeNone extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![place_tests(
        cls(O).extends(&WITH_NESTED).constructor(unit()),
        &ALL_PLACES,
        test_leak,
    )
    .build()]
}
