//! Port of `com.carrotsearch.randomizedtesting.TestBiasedNumbers`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::sync::Arc;

use randomizedtesting::annotations::Repeat;
use randomizedtesting::generators::BiasedNumbers;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::system;

use crate::support::{
    assert_not_nan, assert_that_bool, cls, get_random, test, unit, RANDOMIZED_TEST,
};

fn sanity_check_f(r: &dyn Random, from: f32, to: f32) {
    for _ in 0..100 {
        let v = BiasedNumbers::random_float_between(r, from, to);
        assert_not_nan(v as f64);
        assert_that_bool(v >= from && v <= to).is_true();
    }
}

fn sanity_check_d(r: &dyn Random, from: f64, to: f64) {
    for _ in 0..100 {
        let v = BiasedNumbers::random_double_between(r, from, to);
        assert_not_nan(v);
        assert_that_bool(v >= from && v <= to).is_true();
    }
}

const REPEAT: Repeat = Repeat {
    iterations: 100,
    use_constant_seed: false,
};

/// `TestBiasedNumbers extends RandomizedTest`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls("TestBiasedNumbers")
        .extends(&RANDOMIZED_TEST)
        .constructor(unit())
        .method(
            test("biasedFloats", || {
                let r = get_random();
                let r = &*r;
                sanity_check_f(r, f32::NEG_INFINITY, f32::NEG_INFINITY);
                sanity_check_f(r, f32::NEG_INFINITY, 0.0);
                sanity_check_f(r, f32::NEG_INFINITY, f32::INFINITY);
                sanity_check_f(r, 0.0, f32::INFINITY);
                sanity_check_f(r, 0.0, 0.0);
                sanity_check_f(r, 0.0, 1.0);
                sanity_check_f(r, -1.0, 1.0);
                sanity_check_f(r, 1.0, 2.0);
            })
            .annotate(REPEAT),
        )
        .method(
            test("biasedDoubles", || {
                let r = get_random();
                let r = &*r;
                sanity_check_d(r, f64::NEG_INFINITY, f64::NEG_INFINITY);
                sanity_check_d(r, f64::NEG_INFINITY, 0.0);
                sanity_check_d(r, f64::NEG_INFINITY, f64::INFINITY);
                sanity_check_d(r, 0.0, f64::INFINITY);
                sanity_check_d(r, 0.0, 0.0);
                sanity_check_d(r, 0.0, 1.0);
                sanity_check_d(r, -1.0, 1.0);
                sanity_check_d(r, 1.0, 2.0);
            })
            .annotate(REPEAT),
        )
        .method(test("histo", || {
            let rnd = get_random();
            let r = 10i32;
            let mut counts = vec![vec![0i32; (r + 1) as usize]; (r + 1) as usize];
            for _ in 0..100_000 {
                let x = (r / 2) as f32
                    + BiasedNumbers::random_float_between(&*rnd, (-r / 2) as f32, (r / 2) as f32);
                let y = (r / 2) as f32
                    + BiasedNumbers::random_float_between(&*rnd, (-r / 2) as f32, (r / 2) as f32);
                // Math.round(float): floor(x + 0.5).
                counts[(x + 0.5).floor() as usize][(y + 0.5).floor() as usize] += 1;
            }
            for row in &counts {
                let mut line = String::new();
                for c in row {
                    line.push_str(&format!("{c:>10} "));
                }
                system::out_println(&line);
            }
        }))
        .build()]
}
