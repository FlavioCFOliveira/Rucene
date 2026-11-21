//! Port of `com.carrotsearch.randomizedtesting.timeouts.Test005ThreadLeaksSystemThreads`
//! (randomized-runner 2.8.1 test sources, Apache-2.0): daemon threads known
//! JVM library calls spawn are not leaks.
//!
//! **Divergence.** The nested tests call `MessageDigest.getInstance("MD5")`
//! (the PKCS#11 token poller thread) and `sun.misc.GC.requestLatency` (the
//! `GC Daemon` thread) by reflection (`Test005ThreadLeaksSystemThreads.java:24`,
//! `:36`), so that the main-group leak check meets JVM system threads. Rust
//! has neither; the tests keep their assumption and their `randomByte()`
//! draw, and the check runs over a group without such threads.

use std::sync::{Arc, LazyLock};

use randomizedtesting::annotations::{Group, ThreadLeakGroup};
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::RandomizedTest;

use super::common::{cls, nested};
use crate::support::{assert_that_list, assert_that_str, test, unit, RANDOMIZED_TEST};
use crate::with_nested_test_class::{
    assume_running_nested, get_logging_messages, run_tests, CLASS as WITH_NESTED,
};

const O: &str = "Test005ThreadLeaksSystemThreads";

static NESTED: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Nested")
        .extends(&RANDOMIZED_TEST)
        .annotate(ThreadLeakGroup { value: Group::Main })
        .constructor(unit())
        .method(test("tokenPoller", || {
            assume_running_nested();
            RandomizedTest::random_byte();
        }))
        .method(test("gcDaemon", assume_running_nested))
        .build()
});

/// `Test005ThreadLeaksSystemThreads extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls(O)
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(test("leftOverThread", || {
            let r = run_tests(std::slice::from_ref(&*NESTED));
            assert_that_list(r.get_failures()).is_empty();
            assert_that_str(get_logging_messages())
                .does_not_contain("java.lang.Thread.sleep")
                .does_not_contain("Uncaught exception");
        }))
        .build()]
}
