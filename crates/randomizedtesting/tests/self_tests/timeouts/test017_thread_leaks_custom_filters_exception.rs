//! Port of `com.carrotsearch.randomizedtesting.timeouts.Test017ThreadLeaksCustomFiltersException`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::sync::{Arc, LazyLock};

use randomizedtesting::annotations::{ClassRef, Scope, ThreadLeakFilters, ThreadLeakScope};
use randomizedtesting::jdk::exception::RUNTIME_EXCEPTION;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::ThreadFilter;

use super::common::{cls, nested};
use crate::support::{test, throw, unit, RANDOMIZED_TEST};
use crate::utils::assert_failure_with_message;
use crate::with_nested_test_class::{
    assume_running_nested, is_running_nested, run_tests, start_zombie_thread, CLASS as WITH_NESTED,
};

const O: &str = "Test017ThreadLeaksCustomFiltersException";

/// `ExceptionFilter implements ThreadFilter`.
struct ExceptionFilter;

impl ThreadFilter for ExceptionFilter {
    fn reject(&self, _t: &Thread) -> bool {
        if !is_running_nested() {
            return false;
        }
        throw(RUNTIME_EXCEPTION, "filter-exception")
    }
}

static NESTED1: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Nested1")
        .extends(&RANDOMIZED_TEST)
        .annotate(ThreadLeakScope { value: Scope::Test })
        .annotate(ThreadLeakFilters {
            default_filters: true,
            filters: vec![ClassRef::new(
                "com.carrotsearch.randomizedtesting.timeouts.Test017ThreadLeaksCustomFiltersException$ExceptionFilter",
                || Arc::new(ExceptionFilter) as Arc<dyn ThreadFilter>,
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

/// `Test017ThreadLeaksCustomFiltersException extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls(O)
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(test("testExceptionInFilter", || {
            let r = run_tests(std::slice::from_ref(&*NESTED1));
            assert_failure_with_message(&r, "filter-exception");
        }))
        .build()]
}
