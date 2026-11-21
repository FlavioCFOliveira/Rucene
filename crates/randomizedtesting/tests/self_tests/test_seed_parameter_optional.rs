//! Port of `com.carrotsearch.randomizedtesting.TestSeedParameterOptional`
//! (randomized-runner 2.8.1 test sources, Apache-2.0): the `seed` parameter
//! of a test name is optional without repetitions.

use std::collections::HashSet;
use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::annotations::{Repeat, Seed, Seeds};
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{Description, JUnitCore, RunListener};

use crate::support::{assert_equals, assert_true, cls, nested, test, unit, RANDOMIZED_TEST};
use crate::with_nested_test_class::CLASS as WITH_NESTED;

static NESTED: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested("TestSeedParameterOptional", "Nested")
        .extends(&RANDOMIZED_TEST)
        .constructor(unit())
        .method(
            test("method1", || {})
                // @Seed adds a randomized execution too.
                .annotate(Seeds::new(&["deadbeef", "cafebabe", "random"]))
                .annotate(Repeat {
                    iterations: 2,
                    use_constant_seed: true,
                }),
        )
        .method(test("method2", || {}).annotate(Seed::new("cafebabe")))
        .method(test("method3", || {}))
        .build()
});

struct Names(Arc<Mutex<HashSet<String>>>);

impl RunListener for Names {
    fn test_started(&self, d: &Arc<Description>) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(d.method_name().unwrap_or_default());
    }
}

/// `TestSeedParameterOptional extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls("TestSeedParameterOptional")
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(test("checkNames", || {
            let tests = Arc::new(Mutex::new(HashSet::new()));
            let mut junit = JUnitCore::new();
            junit.add_listener(Arc::new(Names(Arc::clone(&tests))));
            junit.run_class(&NESTED);
            let tests = tests.lock().unwrap_or_else(|e| e.into_inner()).clone();
            // Single repetitions, no seed parameter in test name.
            assert_true(tests.contains("method2"));
            assert_true(tests.contains("method3"));
            // Method 1 has 2x3 repetitions.
            let count = tests.iter().filter(|s| s.starts_with("method1")).count();
            assert_equals(6, count);
        }))
        .build()]
}
