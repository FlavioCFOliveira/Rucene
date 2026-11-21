//! Port of `com.carrotsearch.randomizedtesting.TestJ9SysThreads`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).
//!
//! **Divergence.** The nested test lists the JVM's memory pool MXBeans
//! (`ManagementFactory.getMemoryPoolMXBeans()`, `TestJ9SysThreads.java:23`),
//! which on IBM J9 starts system threads the suite-scope leak check must
//! tolerate. Rust has no MXBeans: the loop has no element, and the test
//! keeps its assumption and its suite-scope `INTERRUPT` leak check.

use std::sync::{Arc, LazyLock};

use randomizedtesting::annotations::{Action, Scope, ThreadLeakAction, ThreadLeakScope};
use randomizedtesting::jdk::reflect::Class;

use crate::support::{cls, nested, test, unit, RANDOMIZED_TEST};
use crate::with_nested_test_class::{
    assume_running_nested, check_tests_output, CLASS as WITH_NESTED,
};

static NESTED: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested("TestJ9SysThreads", "Nested")
        .extends(&RANDOMIZED_TEST)
        .annotate(ThreadLeakScope {
            value: Scope::Suite,
        })
        .annotate(ThreadLeakAction {
            value: vec![Action::Interrupt],
        })
        .constructor(unit())
        .method(test("testMethod1", assume_running_nested))
        .build()
});

/// `TestJ9SysThreads extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls("TestJ9SysThreads")
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(test("testSuccessfulExceptedFailure", || {
            check_tests_output(1, 0, 0, 0, &NESTED);
        }))
        .build()]
}
