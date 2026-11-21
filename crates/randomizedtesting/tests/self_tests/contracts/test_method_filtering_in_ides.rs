//! Port of `com.carrotsearch.randomizedtesting.contracts.TestMethodFilteringInIDEs`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::sync::Arc;

use randomizedtesting::annotations::{Repeat, Seed};
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{Assume, Ignore};

use super::cls;
use crate::support::{test, unit, RANDOMIZED_TEST};

/// `@Repeat(iterations = 2, useConstantSeed = false) @Seed("deadbeef") TestMethodFilteringInIDEs extends RandomizedTest`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls("TestMethodFilteringInIDEs")
        .extends(&RANDOMIZED_TEST)
        .annotate(Repeat {
            iterations: 2,
            use_constant_seed: false,
        })
        .annotate(Seed::new("deadbeef"))
        .constructor(unit())
        .method(test("testExecuted1", || {}))
        .method(test("testExecuted2", || {}))
        .method(test("testIgnoredByAssumption", || {
            Assume::assume_true(false)
        }))
        .method(test("testIgnoredByAnnotation", || {}).annotate(Ignore::new()))
        .build()]
}
