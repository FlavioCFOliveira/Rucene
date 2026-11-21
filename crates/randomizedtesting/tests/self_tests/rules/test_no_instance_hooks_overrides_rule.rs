//! Port of `com.carrotsearch.randomizedtesting.rules.TestNoInstanceHooksOverridesRule`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::sync::{Arc, LazyLock};

use randomizedtesting::jdk::reflect::{modifier, Class, Field, Method, Value};
use randomizedtesting::junit::{Before, ClassRule, RuleChain};
use randomizedtesting::rules::NoInstanceHooksOverridesRule;

use super::{assume_not_nested_field, cls, nested, AssumeNotNestedRule};
use crate::support::{
    assert_that_list, assert_that_str, assertj_fail_msg, hook, test, unit, RANDOMIZED_TEST,
};
use crate::with_nested_test_class::{run_tests, CLASS as WITH_NESTED};

const O: &str = "TestNoInstanceHooksOverridesRule";

static SUPER: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Super")
        .extends(&RANDOMIZED_TEST)
        .constructor(unit())
        .field(assume_not_nested_field())
        .field(
            Field::new_static("classRules", "org.junit.rules.TestRule", || {
                Value::TestRule(Arc::new(
                    RuleChain::outer_rule(Arc::new(AssumeNotNestedRule)).around(Arc::new(
                        // protected boolean verify(Method key) { return !key.getName().equals("setup"); }
                        NoInstanceHooksOverridesRule::new()
                            .with_verify(|key| key.name() != "setup"),
                    )),
                ))
            })
            .annotate(ClassRule),
        )
        .method(hook("before", Before, || {}))
        .method(
            Method::new("privateBefore")
                .modifiers(modifier::PRIVATE)
                .annotate(Before)
                .body_any(|| {}),
        )
        .method(hook("setup", Before, || {}))
        .method(test("testEmpty", || {}))
        .build()
});

fn sub(name: &str, m: Method) -> Arc<Class> {
    nested(O, name)
        .extends(&SUPER)
        .constructor(unit())
        .method(m)
        .build()
}

fn failure_traces(c: Arc<Class>) -> Vec<String> {
    run_tests(&[c])
        .get_failures()
        .iter()
        .map(|f| f.trace())
        .collect()
}

fn not_empty(t: &[String]) {
    if t.is_empty() {
        assertj_fail_msg("\nExpecting actual not to be empty");
    }
}

/// `TestNoInstanceHooksOverridesRule extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls(O)
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(test("testOverrideNoAnnotation", || {
            let t = failure_traces(sub("Sub1", Method::new("before").body_any(|| {})));
            not_empty(&t);
            assert_that_str(t[0].clone()).contains("shadow or override each other");
        }))
        .method(test("testOverrideWithAnnotation", || {
            let t = failure_traces(sub("Sub2", hook("before", Before, || {})));
            not_empty(&t);
            assert_that_str(t[0].clone()).contains("shadow or override each other");
        }))
        .method(test("testIndependentChains", || {
            let t = failure_traces(sub(
                "Sub3",
                Method::new("privateBefore")
                    .modifiers(modifier::PRIVATE)
                    .annotate(Before)
                    .body_any(|| {}),
            ));
            assert_that_list(t).is_empty();
        }))
        .method(test("testFiltering", || {
            let t = failure_traces(sub("Sub4", Method::new("setup").body_any(|| {})));
            assert_that_list(t).is_empty();
        }))
        .build()]
}
