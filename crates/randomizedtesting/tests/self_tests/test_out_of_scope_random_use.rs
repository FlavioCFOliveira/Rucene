//! Port of `com.carrotsearch.randomizedtesting.TestOutOfScopeRandomUse`
//! (randomized-runner 2.8.1 test sources, Apache-2.0): a context's `Random`
//! cannot be used outside its scope.

use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::annotations::Timeout;
use randomizedtesting::jdk::exception::ILLEGAL_STATE_EXCEPTION;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::{AfterClass, Assert, Before, BeforeClass};
use randomizedtesting::{AssertingRandom, RandomizedTest};

use crate::support::{
    assert_that_list, cls, fail_msg, hook, nested, static_hook, test, try_catch, unit,
    RANDOMIZED_TEST,
};
use crate::with_nested_test_class::{
    assume_running_nested, is_running_nested, run_tests, CLASS as WITH_NESTED,
};

type Rnd = Mutex<Option<Arc<dyn Random>>>;

static INSTANCE_RANDOM: Rnd = Mutex::new(None);
static BEFORE_HOOK_RANDOM: Rnd = Mutex::new(None);
static STATIC_CONTEXT_RANDOM: Rnd = Mutex::new(None);
static OTHER_THREAD_RANDOM: Rnd = Mutex::new(None);

fn get(r: &Rnd) -> Option<Arc<dyn Random>> {
    r.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn set(r: &Rnd, v: Option<Arc<dyn Random>>) {
    *r.lock().unwrap_or_else(|e| e.into_inner()) = v;
}

/// `try { r.nextBoolean(); fail(message); } catch (IllegalStateException e) {}`.
fn expect_illegal_state(r: Option<Arc<dyn Random>>, message: &str) {
    match try_catch(|| {
        r.expect("a random").next_boolean();
        fail_msg(message);
    }) {
        Err(e) if e.is_instance(ILLEGAL_STATE_EXCEPTION) => {} // Expected.
        Err(e) => e.throw(),
        Ok(()) => {}
    }
}

/// `touchRandom()`.
fn touch_random() {
    assume_running_nested();
    // We shouldn't be able to reach to some other thread's random for which
    // the context is still valid.
    expect_illegal_state(
        get(&OTHER_THREAD_RANDOM),
        "Shouldn't be able to use another thread's Random.",
    );
    // We should always be able to reach to @Before hook initialized Random.
    get(&BEFORE_HOOK_RANDOM)
        .expect("before hook random")
        .next_boolean();
    // Check if we're the first method or the latter methods.
    match get(&INSTANCE_RANDOM) {
        None => set(&INSTANCE_RANDOM, Some(RandomizedTest::get_random())),
        // For anything not-first, we shouldn't be able to reuse first random anymore.
        Some(r) => expect_illegal_state(Some(r), "Shouldn't be able to use another test's Random."),
    }
}

static NESTED: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested("TestOutOfScopeRandomUse", "Nested")
        .extends(&RANDOMIZED_TEST)
        .constructor(unit())
        .method(static_hook("beforeClass", BeforeClass, || {
            assume_running_nested();
            set(&INSTANCE_RANDOM, None);
            let r = RandomizedTest::get_random();
            // Should be able to use the random we've acquired for the static context.
            r.next_boolean();
            set(&STATIC_CONTEXT_RANDOM, Some(r));
            let t =
                Thread::unnamed(|| set(&OTHER_THREAD_RANDOM, Some(RandomizedTest::get_random())));
            t.start();
            let _ = t.join();
        }))
        .method(static_hook("afterClass", AfterClass, || {
            if !is_running_nested() {
                return;
            }
            // Again should be able to use the random we've acquired for the static context.
            get(&STATIC_CONTEXT_RANDOM)
                .expect("static random")
                .next_boolean();
        }))
        .method(hook("before", Before, || {
            set(&BEFORE_HOOK_RANDOM, Some(RandomizedTest::get_random()))
        }))
        .method(test("method1", touch_random))
        .method(
            test("method2", || {
                touch_random();
                // Timeouting tests run in their own thread, and the class hooks
                // in theirs, so the static random is out of reach.
                expect_illegal_state(
                    get(&STATIC_CONTEXT_RANDOM),
                    "Shouldn't be able to use static context thread's Random.",
                );
            })
            .annotate(Timeout { millis: 2000 }),
        )
        .build()
});

/// `TestOutOfScopeRandomUse extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls("TestOutOfScopeRandomUse")
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(hook("checkRunningWithAssertions", Before, || {
            // Sharing Random is only checked with -ea
            RandomizedTest::assume_true_msg(
                "AssertionRandom not verifying sharing.",
                AssertingRandom::is_verifying(),
            );
        }))
        .method(test("testCrossTestCaseIsolation", || {
            let failures = run_tests(std::slice::from_ref(&*NESTED)).get_failures();
            assert_that_list(failures).is_empty();
        }))
        .method(test("testCrossTestSuiteIsolation", || {
            run_tests(std::slice::from_ref(&*NESTED));
            match try_catch(|| {
                get(&STATIC_CONTEXT_RANDOM)
                    .expect("static random")
                    .next_boolean();
                Assert::fail("Shouldn't be able to use another suite's Random.");
            }) {
                Err(e) if e.is_instance(ILLEGAL_STATE_EXCEPTION) => {}
                Err(e) => e.throw(),
                Ok(()) => {}
            }
        }))
        .build()]
}
