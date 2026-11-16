//! Port of `com.carrotsearch.randomizedtesting.snippets.TheGoodBadAndUglySnippet`
//! (randomized-runner 2.8.1 test sources, Apache-2.0): the documentation
//! snippet of a good, a bad (randomly failing) and an ugly (thread-leaking)
//! test.
//!
//! The outer class is no test class: it has no test method, and Java's
//! `BlockJUnit4ClassRunner` would reject it ("No runnable methods"); it is
//! declared, as Java declares it, and never run (see the crate's
//! `self_tests` divergences: the port has only `RandomizedRunner`).

#![allow(dead_code)]

use std::sync::{Arc, LazyLock};

use randomizedtesting::annotations::{
    Action, Repeat, Scope, ThreadLeakAction, ThreadLeakLingering, ThreadLeakScope,
};
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::BeforeClass;
use randomizedtesting::RandomizedTest;

use crate::support::{assert_false, cls, nested, static_hook, test, unit, RANDOMIZED_TEST};
use crate::with_nested_test_class::{assume_running_nested, CLASS as WITH_NESTED};

/// The outer class's Java name, relative to `com.carrotsearch.randomizedtesting.`.
const O: &str = "snippets.TheGoodBadAndUglySnippet";

/// The seed the oracle runs the nested suite with (`tests.seed`).
pub const SEED: &str = "DEADBEEF";

/// `TheGoodBadAndUglySnippet extends WithNestedTestClass`.
pub static THE_GOOD_BAD_AND_UGLY_SNIPPET: LazyLock<Arc<Class>> =
    LazyLock::new(|| cls(O).extends(&WITH_NESTED).constructor(unit()).build());

/// `TheGoodBadAndUglySnippet.RandomizedTest extends
/// com.carrotsearch.randomizedtesting.RandomizedTest`: "This is cheating so
/// that snippets appear to be extending RandomizedTest but at the same time
/// snippety tests won't execute in Eclipse."
pub static SNIPPET_RANDOMIZED_TEST: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "RandomizedTest")
        .extends(&RANDOMIZED_TEST)
        .constructor(unit())
        .method(static_hook("runAsTest", BeforeClass, assume_running_nested))
        .build()
});

/// `@ThreadLeakScope(Scope.TEST) @ThreadLeakAction({Action.WARN,
/// Action.INTERRUPT}) @ThreadLeakLingering(linger = 1000)
/// TheGoodBadAndUgly extends RandomizedTest`.
pub static THE_GOOD_BAD_AND_UGLY: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "TheGoodBadAndUgly")
        .annotate(ThreadLeakScope { value: Scope::Test })
        .annotate(ThreadLeakAction {
            value: vec![Action::Warn, Action::Interrupt],
        })
        .annotate(ThreadLeakLingering { linger: 1000 })
        .extends(&SNIPPET_RANDOMIZED_TEST)
        .constructor(unit())
        .method(test("good", || {
            // I do nothing and I'm good.
        }))
        .method(
            test("bad", || {
                // I fail randomly, about 20% of the time.
                assert_false(RandomizedTest::random_int_between(1, 100) <= 20);
            })
            .annotate(Repeat {
                iterations: 10,
                use_constant_seed: false,
            }),
        )
        .method(test("ugly", || {
            // I start and abandon a thread which falls
            // outside the test's scope. The test will fail.
            Thread::unnamed(|| RandomizedTest::sleep(5000)).start();
        }))
        .build()
});
