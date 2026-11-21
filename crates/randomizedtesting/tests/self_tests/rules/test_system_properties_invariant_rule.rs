//! Port of `com.carrotsearch.randomizedtesting.rules.TestSystemPropertiesInvariantRule`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).
//!
//! **Divergence.** `NonStringProperties` puts a non-`String` value into
//! `System.getProperties()` (`TestSystemPropertiesInvariantRule.java:79`),
//! which the invariant rule must ignore. The port's system properties hold
//! strings only (`jdk::system`), so the non-`String` entries live in this
//! module's own table, invisible to the rule as Java's rule ignores them;
//! the test's control flow (the second method sees the first one's entry)
//! is unchanged.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::jdk::exception::RUNTIME_EXCEPTION;
use randomizedtesting::jdk::reflect::{Class, Field, Value};
use randomizedtesting::jdk::system;
use randomizedtesting::junit::{AfterClass, BeforeClass, ClassRule, Rule, RuleChain};
use randomizedtesting::rules::SystemPropertiesInvariantRule;

use super::{assume_not_nested_field, cls, nested, AssumeNotNestedRule};
use crate::support::{assert_equals, assert_true, static_hook, test, throw, unit};
use crate::with_nested_test_class::{run_tests, CLASS as WITH_NESTED};

const O: &str = "TestSystemPropertiesInvariantRule";

/// `PROP_KEY1`.
pub const PROP_KEY1: &str = "new-property-1";
/// `VALUE1`.
pub const VALUE1: &str = "new-value-1";

/// The non-`String` entries of `System.getProperties()` (see the divergence).
static OBJECT_PROPERTIES: LazyLock<Mutex<HashMap<String, ()>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

static BASE: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Base")
        .constructor(unit())
        .field(assume_not_nested_field())
        .field(
            Field::new_static("classRules", "org.junit.rules.TestRule", || {
                Value::TestRule(Arc::new(
                    RuleChain::outer_rule(Arc::new(AssumeNotNestedRule))
                        .around(Arc::new(SystemPropertiesInvariantRule::new())),
                ))
            })
            .annotate(ClassRule),
        )
        .field(
            Field::new_instance::<()>("testRules", "org.junit.rules.TestRule", |_| {
                Value::TestRule(Arc::new(RuleChain::outer_rule(Arc::new(
                    SystemPropertiesInvariantRule::new(),
                ))))
            })
            .annotate(Rule),
        )
        .method(test("testEmpty", || {}))
        .build()
});

fn set_prop() {
    system::set_property(PROP_KEY1, VALUE1);
}

static IN_BEFORE_CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "InBeforeClass")
        .extends(&BASE)
        .constructor(unit())
        .method(static_hook("beforeClass", BeforeClass, set_prop))
        .build()
});

static IN_AFTER_CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "InAfterClass")
        .extends(&BASE)
        .constructor(unit())
        .method(static_hook("afterClass", AfterClass, set_prop))
        .build()
});

fn in_test_method1() {
    if system::get_property(PROP_KEY1).is_some() {
        throw(RUNTIME_EXCEPTION, "Shouldn't be here.");
    }
    set_prop();
}

static IN_TEST_METHOD: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "InTestMethod")
        .extends(&BASE)
        .constructor(unit())
        .method(test("testMethod1", in_test_method1))
        .method(test("testMethod2", in_test_method1))
        .build()
});

fn non_string_method1() {
    if OBJECT_PROPERTIES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains_key(PROP_KEY1)
    {
        throw(RUNTIME_EXCEPTION, "Will pass.");
    }
    OBJECT_PROPERTIES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(PROP_KEY1.into(), ());
    assert_true(
        OBJECT_PROPERTIES
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(PROP_KEY1),
    );
}

static NON_STRING_PROPERTIES: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "NonStringProperties")
        .extends(&BASE)
        .constructor(unit())
        .method(test("testMethod1", non_string_method1))
        .method(test("testMethod2", non_string_method1))
        .method(static_hook("cleanup", AfterClass, || {
            OBJECT_PROPERTIES
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(PROP_KEY1);
        }))
        .build()
});

fn assert_single_failure_mentions_key(c: &Arc<Class>) {
    let r = run_tests(std::slice::from_ref(c));
    assert_equals(1, r.get_failure_count());
    assert_true(
        r.get_failures()[0]
            .message()
            .unwrap_or("")
            .contains(PROP_KEY1),
    );
    assert_true(system::get_property(PROP_KEY1).is_none());
}

/// `TestSystemPropertiesInvariantRule extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls(O)
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(test("testRuleInvariantBeforeClass", || {
            assert_single_failure_mentions_key(&IN_BEFORE_CLASS)
        }))
        .method(test("testRuleInvariantAfterClass", || {
            assert_single_failure_mentions_key(&IN_AFTER_CLASS)
        }))
        .method(test("testRuleInvariantInTestMethod", || {
            let r = run_tests(std::slice::from_ref(&*IN_TEST_METHOD));
            assert_equals(2, r.get_failure_count());
            for f in r.get_failures() {
                assert_true(f.message().unwrap_or("").contains(PROP_KEY1));
            }
            assert_true(system::get_property(PROP_KEY1).is_none());
        }))
        .method(test("testNonStringProperties", || {
            let r = run_tests(std::slice::from_ref(&*NON_STRING_PROPERTIES));
            assert_equals(1, r.get_failure_count());
            assert_true(
                r.get_failures()[0]
                    .message()
                    .unwrap_or("")
                    .contains("Will pass"),
            );
            assert_equals(3, r.get_run_count());
        }))
        .build()]
}
