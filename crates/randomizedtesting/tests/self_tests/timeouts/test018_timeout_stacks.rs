//! Port of `com.carrotsearch.randomizedtesting.timeouts.Test018TimeoutStacks`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).
//!
//! **Divergence.** The test asserts that the suite-timeout dump in the log
//! shows the stack of each leaked thread, through the frame
//! `sleepForever(` of `WithNestedTestClass.startThread`
//! (`Test018TimeoutStacks.java:30`). The port has no Java stack frames
//! (`Threads` divergence: the dump is the stack-less thread list), so the
//! faithful equivalent asserts that the dump lists the leaked threads.

use std::sync::{Arc, LazyLock};

use randomizedtesting::annotations::{Scope, ThreadLeakScope, TimeoutSuite};
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;

use super::common::{cls, nested};
use crate::support::{assert_that_str, run_randomized_runner, test, unit};
use crate::with_nested_test_class::{
    assume_running_nested, get_logging_messages, start_thread, CLASS as WITH_NESTED,
};

const O: &str = "Test018TimeoutStacks";

static NESTED1: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Nested1")
        .annotate(ThreadLeakScope { value: Scope::Test })
        .annotate(TimeoutSuite { millis: 1000 })
        .constructor(unit())
        .method(test("testFooBars", || {
            assume_running_nested();
            for i in 0..5 {
                start_thread(&format!("foobar-{i}"));
            }
            Thread::sleep(5000);
        }))
        .build()
});

/// `Test018TimeoutStacks extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls(O)
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(test("testExceptionInFilter", || {
            run_randomized_runner(&NESTED1);
            assert_that_str(get_logging_messages()).contains("foobar-");
        }))
        .build()]
}
