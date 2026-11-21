//! Port of `com.carrotsearch.randomizedtesting.TestRandomizedContext`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::{RandomizedContext, RandomizedTest};

use crate::support::{assert_true_msg, cls, test, unit, RANDOMIZED_TEST};

fn sequence() -> Vec<i32> {
    let size = RandomizedTest::random_int_between(10, 1000);
    (0..size).map(|_| RandomizedTest::random_int()).collect()
}

/// `TestRandomizedContext extends RandomizedTest`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls("TestRandomizedContext")
        .extends(&RANDOMIZED_TEST)
        .constructor(unit())
        .method(test("testRunWithPrivateRandomness", || {
            let iters = RandomizedTest::random_int_between(1, 10);
            for _ in 0..iters {
                let seed = RandomizedTest::random_long();
                let first =
                    RandomizedContext::current().run_with_private_randomness_seed(seed, sequence);
                let second =
                    RandomizedContext::current().run_with_private_randomness_seed(seed, sequence);
                // assertNotNull, assertNotSame: two distinct arrays.
                assert_true_msg(
                    "first and second sequence must be identical",
                    first == second,
                );
            }
        }))
        .build()]
}
