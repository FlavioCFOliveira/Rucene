//! Port of `com.carrotsearch.randomizedtesting.TestSeedDecorator`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::collections::HashSet;
use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::annotations::{SeedDecoratorClass, SeedDecorators};
use randomizedtesting::generators::RandomStrings;
use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::system;
use randomizedtesting::junit::{After, Before, BeforeClass};
use randomizedtesting::{RandomizedContext, SeedUtils, SysGlobals};

use crate::support::{assert_equals, assert_that_str, cls, hook, nested, static_hook, test, unit};
use crate::with_nested_test_class::{assume_running_nested, run_tests, CLASS as WITH_NESTED};

static RUNNER_SEEDS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static STRINGS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// `@RunWith(RandomizedRunner.class) Nested1`.
static NESTED1: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested("TestSeedDecorator", "Nested1")
        .constructor(unit())
        .method(static_hook("generateSequence", BeforeClass, || {
            let s = RandomStrings::random_ascii_letters_of_length(
                &*RandomizedContext::current().get_random(),
                200,
            );
            STRINGS.lock().unwrap_or_else(|e| e.into_inner()).push(s);
        }))
        .method(test("method1", || {
            assume_running_nested();
            RUNNER_SEEDS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(RandomizedContext::current().get_runner_seed_as_string());
        }))
        .build()
});

static NESTED2: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested("TestSeedDecorator", "Nested2")
        .extends(&NESTED1)
        .constructor(unit())
        .build()
});

static NESTED3: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested("TestSeedDecorator", "Nested3")
        .extends(&NESTED1)
        .annotate(SeedDecorators {
            value: vec![SeedDecoratorClass::mix_with_suite_name()],
        })
        .constructor(unit())
        .build()
});

static NESTED4: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested("TestSeedDecorator", "Nested4")
        .extends(&NESTED3)
        .constructor(unit())
        .build()
});

fn cleanup() {
    RUNNER_SEEDS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
    STRINGS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    system::clear_property(&SysGlobals::sysprop_random_seed());
}

/// `TestSeedDecorator extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls("TestSeedDecorator")
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(hook("cleanup", Before, cleanup).annotate(After))
        .method(test("testDecoratedMainSeed", || {
            let main_seed = SeedUtils::format_seed(JdkRandom::new_unseeded().next_long());
            system::set_property(&SysGlobals::sysprop_random_seed(), &main_seed);
            // These classes should get a different main seed (perturbed by decorator).
            run_tests(&[
                NESTED1.clone(),
                NESTED2.clone(),
                NESTED3.clone(),
                NESTED4.clone(),
            ]);
            let runner_seeds = RUNNER_SEEDS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            let strings = STRINGS.lock().unwrap_or_else(|e| e.into_inner()).clone();
            // All four classes get the same initial "runner" seed.
            assert_equals(4, runner_seeds.len());
            assert_equals(1, runner_seeds.iter().collect::<HashSet<_>>().len());
            // @BeforeClass scope strings for Nested1 and Nested2 should be the same
            // because these classes share identical main seed.
            assert_that_str(strings[1].clone()).is_equal_to(&strings[0]);
            // but Nested3 and Nested4 have a seed decorator so strings there
            // should be different.
            assert_that_str(strings[2].clone()).is_not_equal_to(&strings[0]);
            assert_that_str(strings[3].clone()).is_not_equal_to(&strings[0]);
            assert_that_str(strings[2].clone()).is_not_equal_to(&strings[3]);
        }))
        .build()]
}
