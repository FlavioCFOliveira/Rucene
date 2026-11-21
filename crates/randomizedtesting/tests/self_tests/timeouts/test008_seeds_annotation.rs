//! Port of `com.carrotsearch.randomizedtesting.timeouts.Test008SeedsAnnotation`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::annotations::{Repeat, Seeds};
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::RandomizedTest;

use super::common::{cls, nested};
use crate::support::{
    assert_equals, assert_that_list, assert_true, assertj_fail_msg, test, unit, RANDOMIZED_TEST,
};
use crate::utils::get_seed;
use crate::with_nested_test_class::{assume_running_nested, run_tests, CLASS as WITH_NESTED};

const O: &str = "Test008SeedsAnnotation";

/// `final static ArrayList<String> seeds`.
static SEEDS: Mutex<Vec<String>> = Mutex::new(Vec::new());

static NESTED: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Nested")
        .extends(&RANDOMIZED_TEST)
        .constructor(unit())
        .method(
            test("testMe", || {
                assume_running_nested();
                let r = RandomizedTest::get_context()
                    .get_randomness()
                    .expect("a randomness");
                SEEDS
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(format!("{:x}", get_seed(&r)));
            })
            // @Seed adds a randomized execution too.
            .annotate(Seeds::new(&["deadbeef", "cafebabe", "random"]))
            .annotate(Repeat {
                iterations: 2,
                use_constant_seed: true,
            }),
        )
        .build()
});

/// `assertThat(counts).contains(entry(key, value))`.
fn contains_entry(counts: &HashMap<String, i64>, key: &str, value: i64) {
    if counts.get(key) != Some(&value) {
        assertj_fail_msg(&format!("\nExpecting:\n <{counts:?}>\nto contain:\n <[MapEntry[key=\"{key}\", value={value}L]]>"));
    }
}

/// `Test008SeedsAnnotation extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls(O)
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(test("checkSeeds", || {
            let mut counts: HashMap<String, i64> = HashMap::new();
            let n = 4i64;
            for _ in 0..n {
                SEEDS.lock().unwrap_or_else(|e| e.into_inner()).clear();
                let result = run_tests(std::slice::from_ref(&*NESTED));
                assert_equals(3 * 2, result.get_run_count());
                assert_that_list(result.get_failures()).is_empty();
                for s in SEEDS.lock().unwrap_or_else(|e| e.into_inner()).iter() {
                    *counts.entry(s.clone()).or_insert(0) += 1;
                }
            }
            contains_entry(&counts, "deadbeef", n * 2);
            contains_entry(&counts, "cafebabe", n * 2);
            counts.remove("deadbeef");
            counts.remove("cafebabe");
            // Allow for a single collision.
            assert_true(counts.len() as i64 >= n - 1);
        }))
        .build()]
}
