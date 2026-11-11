//! Port of `com.carrotsearch.randomizedtesting.rules.TestNoClassHooksShadowingRule`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::sync::{Arc, LazyLock};

use randomizedtesting::jdk::reflect::{modifier, Class, Field, Method, Value};
use randomizedtesting::junit::{BeforeClass, ClassRule, RuleChain};
use randomizedtesting::rules::NoClassHooksShadowingRule;

use super::{assume_not_nested_field, cls, nested, AssumeNotNestedRule};
use crate::support::{assert_that_list, assert_that_str, static_hook, test, unit, RANDOMIZED_TEST};
use crate::with_nested_test_class::{run_tests, CLASS as WITH_NESTED};

const O: &str = "TestNoClassHooksShadowingRule";

static SUPER: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Super")
        .extends(&RANDOMIZED_TEST)
        .constructor(unit())
        .field(assume_not_nested_field())
        .field(
            Field::new_static("classRules", "org.junit.rules.TestRule", || {
                Value::TestRule(Arc::new(
                    RuleChain::outer_rule(Arc::new(AssumeNotNestedRule))
                        .around(Arc::new(NoClassHooksShadowingRule::new())),
                ))
            })
            .annotate(ClassRule),
        )
        .method(static_hook("before", BeforeClass, || {}))
        .method(
            Method::new("privateBefore")
                .modifiers(modifier::PRIVATE)
                .annotate(BeforeClass)
                .static_body(|| {}),
        )
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

/// `TestNoClassHooksShadowingRule extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls(O)
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(test("testShadowingNoAnnotation", || {
            let t = failure_traces(sub("Sub1", Method::new("before").static_body(|| {})));
            assert_that_bool_not_empty(&t);
            assert_that_str(t[0].clone()).contains("shadow or override each other");
        }))
        .method(test("testShadowingWithAnnotation", || {
            let t = failure_traces(sub("Sub2", static_hook("before", BeforeClass, || {})));
            assert_that_bool_not_empty(&t);
            assert_that_str(t[0].clone()).contains("shadow or override each other");
        }))
        .method(test("testIndependentChains", || {
            let t = failure_traces(sub(
                "Sub3",
                Method::new("privateBefore")
                    .modifiers(modifier::PRIVATE)
                    .annotate(BeforeClass)
                    .static_body(|| {}),
            ));
            assert_that_list(t).is_empty();
        }))
        .build()]
}

/// `assertThat(failures).isNotEmpty()`.
fn assert_that_bool_not_empty(t: &[String]) {
    if t.is_empty() {
        crate::support::assertj_fail_msg("\nExpecting actual not to be empty");
    }
}
