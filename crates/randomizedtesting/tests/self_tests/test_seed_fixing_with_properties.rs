//! Port of `com.carrotsearch.randomizedtesting.TestSeedFixingWithProperties`
//! (randomized-runner 2.8.1 test sources, Apache-2.0): seed fixing for static
//! fixtures and methods with system properties.

use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::system;
use randomizedtesting::junit::{After, Before, BeforeClass};
use randomizedtesting::SysGlobals;

use crate::support::{
    assert_equals, assert_false, cls, current_seed, hook, nested, static_hook, test, unit,
};
use crate::with_nested_test_class::{check_tests_output, run_tests, CLASS as WITH_NESTED};

/// `static List<Long> seeds`.
static SEEDS: Mutex<Vec<i64>> = Mutex::new(Vec::new());

fn push_seed() {
    let s = current_seed();
    SEEDS.lock().unwrap_or_else(|e| e.into_inner()).push(s);
}

fn seeds() -> Vec<i64> {
    SEEDS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// `@RunWith(RandomizedRunner.class) Nested`.
static NESTED: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested("TestSeedFixingWithProperties", "Nested")
        .constructor(unit())
        .method(static_hook("staticFixture", BeforeClass, push_seed))
        .method(test("testMethod", push_seed))
        .build()
});

fn cleanup_after() {
    system::clear_property(&SysGlobals::sysprop_iterations());
    system::clear_property(&SysGlobals::sysprop_random_seed());
    SEEDS.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

/// `assertEquals(List, List)`.
fn assert_list_equals(expected: &[i64], actual: &[i64]) {
    let f = |v: &[i64]| {
        format!(
            "[{}]",
            v.iter()
                .map(|x| x.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    assert_equals(f(expected), f(actual));
}

/// `TestSeedFixingWithProperties extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls("TestSeedFixingWithProperties")
        .extends(&WITH_NESTED)
        .constructor(unit())
        // Combined seed: fixing everything: the runner, method and any repetitions.
        .method(test("testRunnerAndMethodProperty", || {
            system::set_property(&SysGlobals::sysprop_random_seed(), "deadbeef:cafebabe");
            system::set_property(&SysGlobals::sysprop_iterations(), "3");
            run_tests(std::slice::from_ref(&*NESTED));
            assert_list_equals(&[0xdeadbeef, 0xcafebabe, 0xcafebabe, 0xcafebabe], &seeds());
        }))
        // Runner seed fixing only (methods have predictable pseudo-random seeds derived from the runner).
        .method(test("testFixedRunnerPropertyOnly", || {
            system::set_property(&SysGlobals::sysprop_random_seed(), "deadbeef");
            system::set_property(&SysGlobals::sysprop_iterations(), "3");
            check_tests_output(3, 0, 0, 0, &NESTED);
            let s = seeds();
            assert_equals(0xdeadbeef_i64, s[0]);
            // _very_ slim chances of this actually being true...
            assert_false(s[0] == s[1] && s[1] == s[2]);
            // Same randomized method seeds for the same runner seed.
            let copy = s.clone();
            SEEDS.lock().unwrap_or_else(|e| e.into_inner()).clear();
            run_tests(std::slice::from_ref(&*NESTED));
            assert_list_equals(&copy, &seeds());
        }))
        .method(hook("cleanupBefore", Before, cleanup_after))
        .method(hook("cleanupAfter", After, cleanup_after))
        .build()]
}
