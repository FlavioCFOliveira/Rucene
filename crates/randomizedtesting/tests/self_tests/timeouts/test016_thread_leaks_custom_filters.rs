//! Port of `com.carrotsearch.randomizedtesting.timeouts.Test016ThreadLeaksCustomFilters`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::sync::{Arc, LazyLock};

use randomizedtesting::annotations::{ClassRef, Scope, ThreadLeakFilters, ThreadLeakScope};
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::ThreadFilter;

use super::common::{cls, nested};
use crate::support::{assert_that_list, assertj_fail_msg, test, unit, RANDOMIZED_TEST};
use crate::with_nested_test_class::{
    assume_running_nested, get_logging_messages, get_sysouts, run_tests, start_zombie_thread,
    CLASS as WITH_NESTED,
};

const O: &str = "Test016ThreadLeaksCustomFilters";

/// `FooBarFilter implements ThreadFilter`.
struct FooBarFilter;

impl ThreadFilter for FooBarFilter {
    fn reject(&self, t: &Thread) -> bool {
        t.name().contains("foobar")
    }
}

static NESTED1: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Nested1")
        .extends(&RANDOMIZED_TEST)
        .annotate(ThreadLeakScope { value: Scope::Test })
        .annotate(ThreadLeakFilters {
            default_filters: true,
            filters: vec![ClassRef::new(
                "com.carrotsearch.randomizedtesting.timeouts.Test016ThreadLeaksCustomFilters$FooBarFilter",
                || Arc::new(FooBarFilter) as Arc<dyn ThreadFilter>,
            )],
        })
        .constructor(unit())
        .method(test("testFooBars", || {
            assume_running_nested();
            for i in 0..10 {
                start_zombie_thread(&format!("foobar-{i}"));
            }
        }))
        .build()
});

/// `assertThat(text).isEmpty()`.
pub fn is_empty(s: &str) {
    if !s.is_empty() {
        assertj_fail_msg(&format!("\nExpecting empty but was:<\"{s}\">"));
    }
}

/// `Test016ThreadLeaksCustomFilters extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls(O)
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(test("testFilteredOnly", || {
            assert_that_list(run_tests(std::slice::from_ref(&*NESTED1)).get_failures()).is_empty();
            is_empty(&get_logging_messages());
            is_empty(&get_sysouts());
        }))
        .build()]
}
