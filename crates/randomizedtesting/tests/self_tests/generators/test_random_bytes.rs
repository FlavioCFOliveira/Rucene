//! Port of `com.carrotsearch.randomizedtesting.generators.TestRandomBytes`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::sync::Arc;

use randomizedtesting::annotations::Repeat;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::RandomizedTest;

use super::cls;
use crate::support::{assert_that_bool, assert_that_list, test, unit, RANDOMIZED_TEST};

/// `@Repeat(iterations = 100) TestRandomBytes extends RandomizedTest`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls("TestRandomBytes")
        .extends(&RANDOMIZED_TEST)
        .annotate(Repeat {
            iterations: 100,
            use_constant_seed: false,
        })
        .constructor(unit())
        .method(test("testRandomBytes", || {
            let len = RandomizedTest::random_int_between(0, 100);
            assert_that_list(RandomizedTest::random_bytes_of_length(len)).has_size(len as usize);
        }))
        .method(test("testRandomBytesOfLength", || {
            let min = RandomizedTest::random_int_between(0, 100);
            let max = min + RandomizedTest::random_int_between(0, 10);
            let bytes = RandomizedTest::random_bytes_of_length_between(min, max);
            assert_that_bool(bytes.len() as i32 >= min).is_true();
            assert_that_bool(bytes.len() as i32 <= max).is_true();
        }))
        .build()]
}
