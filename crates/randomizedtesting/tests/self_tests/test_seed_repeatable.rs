//! Port of `com.carrotsearch.randomizedtesting.TestSeedRepeatable`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::annotations::Seed;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::BeforeClass;

use crate::support::{assert_equals, cls, current_seed, nested, static_hook, test, unit};
use crate::with_nested_test_class::{
    assume_running_nested, check_tests_output, CLASS as WITH_NESTED,
};

/// `static Map<String, Object> seeds`.
static SEEDS: LazyLock<Mutex<HashMap<String, i64>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// `checkSeed(key)`.
fn check_seed(key: &str) {
    let seed = current_seed();
    let prev = SEEDS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(key)
        .copied();
    match prev {
        Some(s) => assert_equals(s, seed),
        None => {
            SEEDS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(key.to_string(), seed);
        }
    }
}

/// `@Seed("deadbeef") @RunWith(RandomizedRunner.class) Nested`.
static NESTED: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested("TestSeedRepeatable", "Nested")
        .annotate(Seed::new("deadbeef"))
        .constructor(unit())
        .method(static_hook("nested", BeforeClass, assume_running_nested))
        .method(test("testMethod1", || check_seed("method1")))
        .method(test("testMethod2", || check_seed("method2")))
        .build()
});

/// `TestSeedRepeatable extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls("TestSeedRepeatable")
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(test("testSameMethodRandomnessWithFixedRunner", || {
            SEEDS.lock().unwrap_or_else(|e| e.into_inner()).clear();
            check_tests_output(2, 0, 0, 0, &NESTED);
            check_tests_output(2, 0, 0, 0, &NESTED);
            check_tests_output(2, 0, 0, 0, &NESTED);
        }))
        .build()]
}
