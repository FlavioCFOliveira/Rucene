//! Port of `com.carrotsearch.randomizedtesting.TestRandomNumbers`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::sync::Arc;

use randomizedtesting::annotations::Repeat;
use randomizedtesting::generators::RandomNumbers;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;

use crate::support::{cls, fail_msg, get_random, test, unit, RANDOMIZED_TEST};

fn check_random_int(rnd: &dyn Random, min: i32, max: i32) {
    for _ in 0..100_000 {
        let v = RandomNumbers::random_int_between(rnd, min, max);
        if v < min || v > max {
            fail_msg(&format!("{min} {max}: {v}"));
        }
    }
}

fn check_random_long(rnd: &dyn Random, min: i64, max: i64) {
    for _ in 0..100_000 {
        let v = RandomNumbers::random_long_between(rnd, min, max);
        if v < min || v > max {
            fail_msg(&format!("{min} {max}: {v}"));
        }
    }
}

/// `@Repeat(iterations = 100) TestRandomNumbers extends RandomizedTest`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls("TestRandomNumbers")
        .extends(&RANDOMIZED_TEST)
        .annotate(Repeat {
            iterations: 100,
            use_constant_seed: false,
        })
        .constructor(unit())
        .method(test("testRandomIntBetween", || {
            let (max, min) = (i32::MAX, i32::MIN);
            let rnd = get_random();
            check_random_int(&*rnd, 1, 1);
            check_random_int(&*rnd, 0, 100);
            check_random_int(&*rnd, 0, max);
            check_random_int(&*rnd, min, 0);
            check_random_int(&*rnd, -1, max);
            check_random_int(&*rnd, min, 1);
            check_random_int(&*rnd, min, max);
        }))
        .method(test("testRandomLongBetween", || {
            let (max, min) = (i64::MAX, i64::MIN);
            let rnd = get_random();
            check_random_long(&*rnd, 1, 1);
            check_random_long(&*rnd, 0, 100);
            check_random_long(&*rnd, 0, max);
            check_random_long(&*rnd, min, 0);
            check_random_long(&*rnd, -1, max);
            check_random_long(&*rnd, min, 1);
            check_random_long(&*rnd, min, max);
        }))
        .build()]
}
