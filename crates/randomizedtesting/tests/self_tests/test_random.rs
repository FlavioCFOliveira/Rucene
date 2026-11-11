//! Port of `com.carrotsearch.randomizedtesting.TestRandom`
//! (randomized-runner 2.8.1 test sources, Apache-2.0): the context random's
//! uniformity.

use std::sync::Arc;

use randomizedtesting::annotations::Repeat;
use randomizedtesting::jdk::lang::{double_to_string, float_to_string};
use randomizedtesting::jdk::reflect::Class;

use crate::support::{cls, fail_msg, get_random, test, unit, RANDOMIZED_TEST};

/// The bucket check shared by both tests: every bucket within ±10% of the
/// average.
fn check_buckets(buckets: &[i32], reps: i32) {
    let expected_average = reps / buckets.len() as i32;
    let min = expected_average - expected_average / 10;
    let max = expected_average + expected_average / 10;
    for b in buckets {
        if *b < min || *b > max {
            let list: Vec<String> = buckets.iter().map(|x| x.to_string()).collect();
            fail_msg(&format!("oops: {b} biased? [{}]", list.join(", ")));
        }
    }
}

/// `@Repeat(iterations = 100) TestRandom extends RandomizedTest`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls("TestRandom")
        .extends(&RANDOMIZED_TEST)
        .annotate(Repeat {
            iterations: 100,
            use_constant_seed: false,
        })
        .constructor(unit())
        .method(test("testDoubleDistribution", || {
            let rnd = get_random();
            let mut buckets = [0i32; 100];
            let reps = 1_000_000;
            for _ in 0..reps {
                let d = rnd.next_double();
                if !(0.0..1.0).contains(&d) {
                    fail_msg(&format!("Oops: {}", double_to_string(d)));
                }
                buckets[(d * buckets.len() as f64) as usize] += 1;
            }
            check_buckets(&buckets, reps);
        }))
        .method(test("testFloatDistribution", || {
            let rnd = get_random();
            let mut buckets = [0i32; 100];
            let reps = 1_000_000;
            for _ in 0..reps {
                let d = rnd.next_float();
                if !(0.0..1.0).contains(&d) {
                    fail_msg(&format!("Oops: {}", float_to_string(d)));
                }
                buckets[(d * buckets.len() as f32) as usize] += 1;
            }
            check_buckets(&buckets, reps);
        }))
        .build()]
}
