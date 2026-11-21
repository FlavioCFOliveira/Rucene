//! Port of `com.carrotsearch.randomizedtesting.TestHookMethodOrderWithExceptions`
//! (randomized-runner 2.8.1 test sources, Apache-2.0): hook order when hooks
//! throw.
//!
//! **Divergence.** The test compares the hook order under JUnit's
//! `BlockJUnit4ClassRunner` (`WithRegularRunner`) with the order under
//! `RandomizedRunner` (`WithRandomizedRunner`,
//! `TestHookMethodOrderWithExceptions.java:110`). The port has no JUnit
//! runner (`Request::a_class`): both run under `RandomizedRunner`, so the
//! comparison checks that the order is deterministic for a seed.

use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::annotations::Repeat;
use randomizedtesting::jdk::exception::RUNTIME_EXCEPTION;
use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::reflect::{modifier, Class, Method};
use randomizedtesting::junit::{After, AfterClass, Before, BeforeClass};

use crate::support::{
    assert_equals, assert_that_num, cls, current_seed, hook, nested, static_hook, test,
    throw_no_message, unit, RANDOMIZED_TEST,
};
use crate::with_nested_test_class::run_tests;

const O: &str = "TestHookMethodOrderWithExceptions";

/// `static final List<String> callOrder`.
static CALL_ORDER: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// `Super.rnd`.
static RND: Mutex<Option<Arc<JdkRandom>>> = Mutex::new(None);

/// `Super.maybeThrowException()`.
fn maybe_throw_exception() {
    let r = RND.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if let Some(r) = r {
        if r.next_int_bound(10) == 0 {
            throw_no_message(RUNTIME_EXCEPTION);
        }
    }
}

fn step(s: &'static str) -> impl Fn() + Send + Sync + 'static {
    move || {
        CALL_ORDER
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(s.into());
        maybe_throw_exception();
    }
}

/// `public abstract static class Super`.
static SUPER: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Super")
        .modifiers(modifier::PUBLIC | modifier::STATIC | modifier::ABSTRACT)
        .constructor(unit())
        .method(static_hook(
            "beforeClassSuper",
            BeforeClass,
            step("beforeClassSuper"),
        ))
        .method(
            Method::new("beforeTest")
                .modifiers(modifier::PUBLIC | modifier::FINAL)
                .annotate(Before)
                .body_any(step("beforeTestSuper")),
        )
        .method(
            Method::new("afterTest")
                .modifiers(modifier::PUBLIC | modifier::FINAL)
                .annotate(After)
                .body_any(step("afterTestSuper")),
        )
        .method(static_hook(
            "afterClassSuper",
            AfterClass,
            step("afterClassSuper"),
        ))
        .method(Method::new("maybeThrowException").static_body(maybe_throw_exception))
        .build()
});

static SUB_SUB: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "SubSub")
        .extends(&SUPER)
        .constructor(unit())
        .method(static_hook(
            "beforeClass",
            BeforeClass,
            step("beforeClassSub"),
        ))
        .method(hook("beforeTestSub", Before, step("beforeTestSub")))
        .method(test("testMethod", step("testMethodSub")))
        .method(hook("afterTestSub", After, step("afterTestSub")))
        .method(static_hook("afterClass", AfterClass, step("afterClassSub")))
        .build()
});

/// `@RunWith(RandomizedRunner.class) WithRandomizedRunner extends SubSub`.
static WITH_RANDOMIZED_RUNNER: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "WithRandomizedRunner")
        .extends(&SUB_SUB)
        .constructor(unit())
        .build()
});
/// `WithRegularRunner extends SubSub`.
static WITH_REGULAR_RUNNER: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "WithRegularRunner")
        .extends(&SUB_SUB)
        .constructor(unit())
        .build()
});

fn clear() {
    CALL_ORDER.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

fn order() -> String {
    format!(
        "[{}]",
        CALL_ORDER
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .join(", ")
    )
}

/// `TestHookMethodOrderWithExceptions extends RandomizedTest`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls(O)
        .extends(&RANDOMIZED_TEST)
        .constructor(unit())
        .method(hook("setup", Before, clear))
        .method(hook("cleanup", After, clear))
        .method(
            test("checkOrderSameAsJUnit", || {
                let seed = current_seed();
                clear();
                *RND.lock().unwrap_or_else(|e| e.into_inner()) =
                    Some(Arc::new(JdkRandom::new(seed)));
                let r1 = run_tests(std::slice::from_ref(&*WITH_REGULAR_RUNNER));
                let junit_order = order();
                clear();
                *RND.lock().unwrap_or_else(|e| e.into_inner()) =
                    Some(Arc::new(JdkRandom::new(seed)));
                let r2 = run_tests(std::slice::from_ref(&*WITH_RANDOMIZED_RUNNER));
                let rrunner_order = order();
                assert_equals(junit_order, rrunner_order);
                assert_that_num(r1.get_run_count() as i64).is_equal_to(r2.get_run_count() as i64);
            })
            .annotate(Repeat {
                iterations: 20,
                use_constant_seed: false,
            }),
        )
        .build()]
}
