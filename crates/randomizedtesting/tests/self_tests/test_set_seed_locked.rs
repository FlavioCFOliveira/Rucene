//! Port of `com.carrotsearch.randomizedtesting.TestSetSeedLocked`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::sync::Arc;

use randomizedtesting::jdk::exception::RUNTIME_EXCEPTION;
use randomizedtesting::jdk::reflect::Class;

use crate::support::{cls, fail, get_random, test, try_catch, unit, RANDOMIZED_TEST};

/// `TestSetSeedLocked extends RandomizedTest`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls("TestSetSeedLocked")
        .extends(&RANDOMIZED_TEST)
        .constructor(unit())
        .method(test("testMethod", || {
            match try_catch(|| {
                get_random().set_seed(0);
                fail();
            }) {
                Err(e) if e.is_instance(RUNTIME_EXCEPTION) => {} // Ok, expected.
                Err(e) => e.throw(),
                Ok(()) => {}
            }
        }))
        .build()]
}
