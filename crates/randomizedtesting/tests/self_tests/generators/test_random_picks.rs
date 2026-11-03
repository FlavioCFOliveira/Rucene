//! Port of `com.carrotsearch.randomizedtesting.generators.TestRandomPicks`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::collections::HashSet;
use std::sync::Arc;

use randomizedtesting::generators::RandomPicks;
use randomizedtesting::jdk::exception::ILLEGAL_ARGUMENT_EXCEPTION;
use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::junit::Test;

use super::cls;
use crate::support::{assert_true, get_random, test, unit, RANDOMIZED_TEST};

fn expects_iae(name: &str, body: fn()) -> Method {
    Method::new(name)
        .annotate(Test::expected(ILLEGAL_ARGUMENT_EXCEPTION))
        .body_any(body)
}

/// `TestRandomPicks extends RandomizedTest`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls("TestRandomPicks")
        .extends(&RANDOMIZED_TEST)
        .constructor(unit())
        .method(expects_iae("testRandomFromEmptyCollection", || {
            RandomPicks::random_from_collection(&*get_random(), HashSet::<i32>::new());
        }))
        .method(test("testRandomFromCollection", || {
            // assertSame(r, t): the one element of the set, by identity.
            let t = Arc::new(());
            let set: Vec<Arc<()>> = vec![Arc::clone(&t)];
            let r = RandomPicks::random_from_collection(&*get_random(), set);
            assert_true(Arc::ptr_eq(&r, &t));
        }))
        .method(expects_iae("testRandomFromList", || {
            RandomPicks::random_from_list(&*get_random(), &Vec::<i32>::new());
        }))
        .method(expects_iae("testRandomFromArray", || {
            RandomPicks::random_from(&*get_random(), &[0u8; 0]);
        }))
        .build()]
}
