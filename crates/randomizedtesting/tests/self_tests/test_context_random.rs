//! Port of `com.carrotsearch.randomizedtesting.TestContextRandom`
//! (randomized-runner 2.8.1 test sources, Apache-2.0): the context's random
//! is repeatable.

use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::annotations::Seed;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Ignore;
use randomizedtesting::RandomizedTest;

use crate::support::{assert_equals, assert_false, cls, nested, test, unit, RANDOMIZED_TEST};
use crate::with_nested_test_class::{run_tests, CLASS as WITH_NESTED};

/// `static ArrayList<Integer> numbers`.
static NUMBERS: Mutex<Vec<i32>> = Mutex::new(Vec::new());

fn fill() {
    let mut n = NUMBERS.lock().unwrap_or_else(|e| e.into_inner());
    n.clear();
    for _ in 0..10 {
        n.push(RandomizedTest::random_int());
    }
}

fn numbers() -> String {
    format!(
        "{:?}",
        NUMBERS.lock().unwrap_or_else(|e| e.into_inner()).clone()
    )
}

static NESTED1: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested("TestContextRandom", "Nested1")
        .extends(&RANDOMIZED_TEST)
        .constructor(unit())
        // Fix the seed to get a repeatable result.
        .method(test("testMethod", fill).annotate(Seed::new("deadbeef")))
        .build()
});

static NESTED2: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested("TestContextRandom", "Nested2")
        .extends(&RANDOMIZED_TEST)
        .constructor(unit())
        .method(test("testMethod", fill))
        .build()
});

static NESTED3: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested("TestContextRandom", "Nested3")
        .extends(&RANDOMIZED_TEST)
        .constructor(unit())
        // Fix the seed to get a repeatable result even if subthreads use randomness.
        .method(
            test("testMethod", || {
                let t = Thread::unnamed(fill);
                t.start();
                let _ = t.join();
            })
            .annotate(Seed::new("deadbeef")),
        )
        .build()
});

fn twice(c: &Arc<Class>) -> (String, String) {
    run_tests(std::slice::from_ref(c));
    let run1 = numbers();
    run_tests(std::slice::from_ref(c));
    (run1, numbers())
}

/// `TestContextRandom extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls("TestContextRandom")
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(
            test("testFixedSeedSubthreads", || {
                let (a, b) = twice(&NESTED3);
                assert_equals(a, b);
            })
            .annotate(Ignore::because(
                "Forked threads get the main seed (by-design).",
            )),
        )
        .method(test("testFixedSeed", || {
            let (a, b) = twice(&NESTED1);
            assert_equals(a, b);
        }))
        .method(test("testRandomSeed", || {
            let (a, b) = twice(&NESTED2);
            assert_false(a == b);
        }))
        .build()]
}
