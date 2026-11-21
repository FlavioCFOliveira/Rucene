//! Port of `com.carrotsearch.randomizedtesting.timeouts.Test009TimeoutOrNotIdenticalSequence`
//! (randomized-runner 2.8.1 test sources, Apache-2.0): the random sequence
//! does not depend on `@Timeout`.

use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::annotations::{Repeat, Seed, Timeout};
use randomizedtesting::jdk::reflect::{Class, Method};
use randomizedtesting::RandomizedTest;

use super::common::{cls, nested};
use crate::support::{assert_that_bool, assert_that_list, test, unit, RANDOMIZED_TEST};
use crate::with_nested_test_class::{assume_running_nested, run_tests, CLASS as WITH_NESTED};

const O: &str = "Test009TimeoutOrNotIdenticalSequence";

/// `final static ArrayList<String> seeds`.
static SEEDS: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn test_no_timeout() {
    assume_running_nested();
    SEEDS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(RandomizedTest::random_ascii_letters_of_length(20));
}

const REPEAT: Repeat = Repeat {
    iterations: 2,
    use_constant_seed: false,
};

fn method() -> Method {
    test("testNoTimeout", test_no_timeout)
        .annotate(Seed::new("deadbeef"))
        .annotate(REPEAT)
}

static NESTED1: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Nested1")
        .extends(&RANDOMIZED_TEST)
        .constructor(unit())
        .method(method())
        .build()
});

static NESTED2: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Nested2")
        .extends(&NESTED1)
        .constructor(unit())
        .method(method().annotate(Timeout { millis: 30 * 1000 }))
        .build()
});

static NESTED3: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Nested3")
        .extends(&NESTED1)
        .annotate(Timeout { millis: 30 * 1000 })
        .constructor(unit())
        .method(method().annotate(Timeout { millis: 30 * 1000 }))
        .build()
});

/// `Test009TimeoutOrNotIdenticalSequence extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls(O)
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(test("checkAllRunsIdentical", || {
            let mut previous: Option<Vec<String>> = None;
            for c in [NESTED1.clone(), NESTED2.clone(), NESTED3.clone()] {
                SEEDS.lock().unwrap_or_else(|e| e.into_inner()).clear();
                assert_that_bool(run_tests(std::slice::from_ref(&c)).was_successful()).is_true();
                let seeds = SEEDS.lock().unwrap_or_else(|e| e.into_inner()).clone();
                match &previous {
                    Some(p) => {
                        assert_that_list(seeds)
                            .as_(format!("Class {}", c.simple_name()))
                            .is_equal_to(p);
                    }
                    None => previous = Some(seeds),
                }
            }
        }))
        .build()]
}
