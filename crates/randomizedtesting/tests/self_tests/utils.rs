//! Port of `com.carrotsearch.randomizedtesting.Utils` (randomized-runner
//! 2.8.1 test sources, Apache-2.0): assertions over nested-run results.

#![allow(dead_code)]

use randomizedtesting::jdk::logging::Logger;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::{Assert, JUnitResult};
use randomizedtesting::{seed_from_throwable, RandomizedContext, Threads};

use crate::support;
use crate::with_nested_test_class::FullResult;

/// `assertFailureWithMessage(r, message)`: some failure's trace contains
/// `message`.
pub fn assert_failure_with_message(r: &FullResult, message: &str) {
    let failures = r.get_failures();
    if failures.iter().any(|f| f.trace().contains(message)) {
        return;
    }
    let mut b = format!(
        "No failure with message: '{message}' ({} failures):",
        failures.len()
    );
    for f in &failures {
        b.push_str(&format!("\n\t- {}", f.trace()));
    }
    Logger::get_logger("").severe(&b);
    Assert::fail(&b);
}

/// `assertNoFailureWithMessage(r, message)`.
pub fn assert_no_failure_with_message(r: &JUnitResult, message: &str) {
    let failures = r.failures();
    if !failures.iter().any(|f| f.trace().contains(message)) {
        return;
    }
    let mut b = format!(
        "Failure with message: '{message}' ({} failures):",
        failures.len()
    );
    for f in &failures {
        b.push_str(&format!("\n\t- {}", f.trace()));
    }
    Logger::get_logger("").severe(&b);
    Assert::fail(&b);
}

/// `assertFailuresContainSeeds(r)`: every failure was augmented with its
/// seed.
pub fn assert_failures_contain_seeds(r: &FullResult) {
    for f in r.get_failures() {
        let seed = seed_from_throwable(f.exception());
        Assert::assert_true_msg(&format!("Not augmented: {}", f.trace()), seed.is_some());
    }
}

/// `getRunnerSeed()`: `RandomizedContext.current().getRunnerSeed()`.
pub fn get_runner_seed() -> i64 {
    RandomizedContext::current().get_runner_seed()
}

/// `assertNoLiveThreadsContaining(substring)`.
pub fn assert_no_live_threads_containing(substring: &str) {
    for t in get_all_threads() {
        if t.is_alive() {
            support::assert_that_str(t.name())
                .as_("Unexpected live thread")
                .does_not_contain(substring);
        }
    }
}

/// `getAllThreads()`: `Threads.getAllThreads()`.
pub fn get_all_threads() -> Vec<Thread> {
    Threads::get_all_threads()
}

/// `getTopThreadGroup()`: `Threads.getTopThreadGroup()`.
pub fn get_top_thread_group() -> std::sync::Arc<randomizedtesting::jdk::thread::ThreadGroup> {
    Threads::get_top_thread_group()
}

/// `getSeed(randomness)`: `Randomness.getSeed()`.
pub fn get_seed(randomness: &randomizedtesting::Randomness) -> i64 {
    randomness.get_seed()
}
