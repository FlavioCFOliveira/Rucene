//! Package `com.carrotsearch.randomizedtesting.rules` of the runner's own
//! tests.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{Description, Statement, TestRule};

mod test_no_class_hooks_shadowing_rule;
mod test_no_instance_hooks_overrides_rule;
mod test_static_fields_invariant_rule;
mod test_system_properties_invariant_rule;

/// `rules.<TestClass>` entries.
pub fn entries() -> Vec<crate::Entry> {
    vec![
        (
            "rules.TestNoClassHooksShadowingRule",
            test_no_class_hooks_shadowing_rule::classes as fn() -> Vec<Arc<Class>>,
        ),
        (
            "rules.TestNoInstanceHooksOverridesRule",
            test_no_instance_hooks_overrides_rule::classes,
        ),
        (
            "rules.TestStaticFieldsInvariantRule",
            test_static_fields_invariant_rule::classes,
        ),
        (
            "rules.TestSystemPropertiesInvariantRule",
            test_system_properties_invariant_rule::classes,
        ),
    ]
}

/// A builder for `com.carrotsearch.randomizedtesting.rules.<name>`.
pub fn cls(name: &str) -> randomizedtesting::jdk::reflect::ClassBuilder {
    crate::support::cls(&format!("rules.{name}"))
}

/// A builder for the nested `rules.<outer>$<inner>`.
pub fn nested(outer: &str, inner: &str) -> randomizedtesting::jdk::reflect::ClassBuilder {
    crate::support::nested(&format!("rules.{outer}"), inner)
}

/// The anonymous `assumeNotNestedRule` every suite here declares:
/// `assumeRunningNested()`, then the base.
pub struct AssumeNotNestedRule;

impl TestRule for AssumeNotNestedRule {
    fn apply(&self, base: Statement, _d: &Arc<Description>) -> Statement {
        Statement::new(move || {
            crate::with_nested_test_class::assume_running_nested();
            base.evaluate();
        })
    }
}

/// `private static TestRule assumeNotNestedRule` as a declared field.
pub fn assume_not_nested_field() -> randomizedtesting::jdk::reflect::Field {
    randomizedtesting::jdk::reflect::Field::new_static(
        "assumeNotNestedRule",
        "org.junit.rules.TestRule",
        || randomizedtesting::jdk::reflect::Value::TestRule(Arc::new(AssumeNotNestedRule)),
    )
    .modifiers(randomizedtesting::jdk::reflect::modifier::PRIVATE)
}
