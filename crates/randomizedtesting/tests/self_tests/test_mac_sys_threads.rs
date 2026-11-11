//! Port of `com.carrotsearch.randomizedtesting.TestMacSysThreads`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).
//!
//! **Divergence.** The nested test touches the JVM's platform MBean server
//! (`ManagementFactory.getPlatformMBeanServer().getMBeanCount()`,
//! `TestMacSysThreads.java:21`), which starts JVM system threads on macOS
//! that the suite-scope leak check must tolerate. Rust has no MBean server;
//! the test keeps the rest of its body (the 2.5 s sleep under a suite-scope
//! leak check), the faithful equivalent the port allows.

use std::sync::{Arc, LazyLock};

use randomizedtesting::annotations::{Action, Scope, ThreadLeakAction, ThreadLeakScope};
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::RandomizedTest;

use crate::support::{cls, nested, test, unit, RANDOMIZED_TEST};
use crate::with_nested_test_class::{check_tests_output, CLASS as WITH_NESTED};

static NESTED: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested("TestMacSysThreads", "Nested")
        .extends(&RANDOMIZED_TEST)
        .annotate(ThreadLeakScope {
            value: Scope::Suite,
        })
        .annotate(ThreadLeakAction {
            value: vec![Action::Warn],
        })
        .constructor(unit())
        .method(test("testMethod1", || {
            RandomizedTest::sleep(2500);
        }))
        .build()
});

/// `TestMacSysThreads extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls("TestMacSysThreads")
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(test("testSuccessful", || {
            check_tests_output(1, 0, 0, 0, &NESTED);
        }))
        .build()]
}
