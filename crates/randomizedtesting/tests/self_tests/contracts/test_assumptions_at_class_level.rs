//! Port of `com.carrotsearch.randomizedtesting.contracts.TestAssumptionsAtClassLevel`
//! (randomized-runner 2.8.1 test sources, Apache-2.0): assumptions in
//! `@BeforeClass`.

use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{After, AfterClass, Assume, Before, BeforeClass};

use super::{cls, nested};
use crate::support::{assert_equals, hook, static_hook, test, unit, RANDOMIZED_TEST};
use crate::with_nested_test_class::{
    assume_running_nested, check_tests_output, CLASS as WITH_NESTED,
};

const O: &str = "TestAssumptionsAtClassLevel";

/// `static final List<String> callOrder`.
static CALL_ORDER: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn step(s: &'static str) -> impl Fn() + Send + Sync + 'static {
    move || {
        CALL_ORDER
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(s.into())
    }
}

static SUPER: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Super")
        .extends(&RANDOMIZED_TEST)
        .constructor(unit())
        .method(static_hook("beforeClassSuper", BeforeClass, || {
            assume_running_nested();
            step("beforeClassSuper")();
            Assume::assume_true(false);
        }))
        .method(static_hook(
            "afterClassSuper",
            AfterClass,
            step("afterClassSuper"),
        ))
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

/// `TestAssumptionsAtClassLevel extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls(O)
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(hook("cleanup", Before, || {
            CALL_ORDER.lock().unwrap_or_else(|e| e.into_inner()).clear()
        }))
        .method(test("checkOrder", || {
            check_tests_output(0, 1, 0, 1, &SUB_SUB);
            let expected = "[beforeClassSuper, afterClassSub, afterClassSuper]".to_string();
            let got = format!(
                "[{}]",
                CALL_ORDER
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .join(", ")
            );
            assert_equals(expected, got);
        }))
        .build()]
}
