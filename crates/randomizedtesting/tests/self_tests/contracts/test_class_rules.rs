//! Port of `com.carrotsearch.randomizedtesting.contracts.TestClassRules`
//! (randomized-runner 2.8.1 test sources, Apache-2.0): `@ClassRule`.

use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::jdk::reflect::{Class, Field, Value};
use randomizedtesting::junit::{AfterClass, BeforeClass, ClassRule, RuleChain};

use super::{cls, nested, AppendRule};
use crate::support::{assert_equals, run_randomized_runner, static_hook, test, unit};
use crate::with_nested_test_class::{run_tests, CLASS as WITH_NESTED};

const O: &str = "TestClassRules";

/// `final static List<String> order`.
static ORDER: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn add(s: String) {
    ORDER.lock().unwrap_or_else(|e| e.into_inner()).push(s);
}

static CLASS_RULE_SUPPORT: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "ClassRuleSupport")
        .constructor(unit())
        .field(
            Field::new_static("rules", "org.junit.rules.TestRule", || {
                Value::TestRule(Arc::new(RuleChain::outer_rule(Arc::new(AppendRule {
                    text: "rule".into(),
                    sink: add,
                }))))
            })
            .annotate(ClassRule),
        )
        .method(static_hook("beforeClass", BeforeClass, || {
            add("before-class".into())
        }))
        .method(static_hook("afterClass", AfterClass, || {
            add("after-class".into())
        }))
        .method(test("passing", || add("passing".into())))
        .build()
});

fn take() -> Vec<String> {
    std::mem::take(&mut *ORDER.lock().unwrap_or_else(|e| e.into_inner()))
}

/// `TestClassRules extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls(O)
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(test("checkOldMethodRules", || {
            take();
            run_tests(std::slice::from_ref(&*CLASS_RULE_SUPPORT));
            let order1 = take();
            run_randomized_runner(&CLASS_RULE_SUPPORT);
            let order2 = take();
            assert_equals(format!("{order1:?}"), format!("{order2:?}"));
        }))
        .build()]
}
