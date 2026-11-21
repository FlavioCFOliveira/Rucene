//! Package `com.carrotsearch.randomizedtesting.contracts` of the runner's
//! own tests: JUnit contracts the runner honours.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;

mod test_annotation_inheritance;
mod test_assumptions_at_class_level;
mod test_before_after_method_order;
mod test_class_level_ignore;
mod test_class_rules;
mod test_method_filtering_in_ides;

/// `contracts.<TestClass>` entries.
pub fn entries() -> Vec<crate::Entry> {
    vec![
        (
            "contracts.TestAnnotationInheritance",
            test_annotation_inheritance::classes as fn() -> Vec<Arc<Class>>,
        ),
        (
            "contracts.TestAssumptionsAtClassLevel",
            test_assumptions_at_class_level::classes,
        ),
        (
            "contracts.TestBeforeAfterMethodOrder",
            test_before_after_method_order::classes,
        ),
        (
            "contracts.TestClassLevelIgnore",
            test_class_level_ignore::classes,
        ),
        ("contracts.TestClassRules", test_class_rules::classes),
        (
            "contracts.TestMethodFilteringInIDEs",
            test_method_filtering_in_ides::classes,
        ),
    ]
}

/// A builder for `com.carrotsearch.randomizedtesting.contracts.<name>`.
pub fn cls(name: &str) -> randomizedtesting::jdk::reflect::ClassBuilder {
    crate::support::cls(&format!("contracts.{name}"))
}

/// A builder for the nested `contracts.<outer>$<inner>`.
pub fn nested(outer: &str, inner: &str) -> randomizedtesting::jdk::reflect::ClassBuilder {
    crate::support::nested(&format!("contracts.{outer}"), inner)
}

/// A recording `TestRule`: `<text>-before`, the base, `<text>-after` (the
/// base's failure skips the second entry, as Java's statement does).
pub struct AppendRule {
    pub text: String,
    pub sink: fn(String),
}

impl randomizedtesting::junit::TestRule for AppendRule {
    fn apply(
        &self,
        base: randomizedtesting::junit::Statement,
        _d: &Arc<randomizedtesting::junit::Description>,
    ) -> randomizedtesting::junit::Statement {
        let (text, sink) = (self.text.clone(), self.sink);
        randomizedtesting::junit::Statement::new(move || {
            sink(format!("{text}-before"));
            base.evaluate();
            sink(format!("{text}-after"));
        })
    }
}
