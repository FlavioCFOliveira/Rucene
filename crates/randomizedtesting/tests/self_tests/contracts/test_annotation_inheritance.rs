//! Port of `com.carrotsearch.randomizedtesting.contracts.TestAnnotationInheritance`
//! (randomized-runner 2.8.1 test sources, Apache-2.0): annotations on
//! overridden and shadowed hooks.

use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::jdk::reflect::{Class, Field, Method, Value};
use randomizedtesting::junit::{After, AfterClass, Before, BeforeClass, Rule};

use super::{cls, nested, AppendRule};
use crate::support::{assert_that_list, hook, run_randomized_runner, static_hook, test, unit};
use crate::with_nested_test_class::{run_tests, CLASS as WITH_NESTED};

const O: &str = "TestAnnotationInheritance";

/// `final static List<String> order`.
static ORDER: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn add(s: String) {
    ORDER.lock().unwrap_or_else(|e| e.into_inner()).push(s);
}

fn step(s: &'static str) -> impl Fn() + Send + Sync + 'static {
    move || add(s.into())
}

static NESTED1: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Nested1")
        .constructor(unit())
        .field(
            Field::new_instance::<()>("rules", "org.junit.rules.TestRule", |_| {
                Value::TestRule(Arc::new(AppendRule {
                    text: "rule".into(),
                    sink: add,
                }))
            })
            .annotate(Rule),
        )
        .method(static_hook(
            "beforeClass",
            BeforeClass,
            step("before-class"),
        ))
        .method(hook("before", Before, step("before-test")))
        .method(test("testMethod1", step("testMethod1")))
        .method(hook("after", After, step("after-test")))
        .method(static_hook("afterClass", AfterClass, step("after-class")))
        .build()
});

static NESTED2: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Nested2")
        .extends(&NESTED1)
        .constructor(unit())
        .method(Method::new("beforeClass").static_body(step("shadowed-before-class")))
        .method(Method::new("before").body_any(step("inherited before-test")))
        .method(Method::new("after").body_any(step("inherited after-test")))
        .method(Method::new("testMethod1").body_any(step("inherited testMethod")))
        .method(Method::new("afterClass").static_body(step("shadowed-after-class")))
        .build()
});

fn take() -> Vec<String> {
    std::mem::take(&mut *ORDER.lock().unwrap_or_else(|e| e.into_inner()))
}

/// `TestAnnotationInheritance extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls(O)
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(test("checkOldMethodRules", || {
            take();
            run_tests(std::slice::from_ref(&*NESTED2));
            let order1 = take();
            run_randomized_runner(&NESTED2);
            let order2 = take();
            let msg = format!(
                "# JUnit order:\n[{}]\n# RR order:\n[{}]",
                order1.join(", "),
                order2.join(", ")
            );
            assert_that_list(order2).as_(msg).is_equal_to(&order1);
        }))
        .build()]
}
