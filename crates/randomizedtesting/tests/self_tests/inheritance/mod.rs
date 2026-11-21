//! Package `com.carrotsearch.randomizedtesting.inheritance` of the runner's
//! own tests: hook and rule propagation through class hierarchies.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;

mod junit_annotation_propagation;
mod private_hooks_propagation;

/// `inheritance.<TestClass>` entries.
pub fn entries() -> Vec<crate::Entry> {
    vec![
        (
            "inheritance.JUnitAnnotationPropagation",
            junit_annotation_propagation::classes as fn() -> Vec<Arc<Class>>,
        ),
        (
            "inheritance.PrivateHooksPropagation",
            private_hooks_propagation::classes,
        ),
    ]
}

/// A builder for `com.carrotsearch.randomizedtesting.inheritance.<name>`.
pub fn cls(name: &str) -> randomizedtesting::jdk::reflect::ClassBuilder {
    crate::support::cls(&format!("inheritance.{name}"))
}

/// A builder for the nested `inheritance.<outer>$<inner>`.
pub fn nested(outer: &str, inner: &str) -> randomizedtesting::jdk::reflect::ClassBuilder {
    crate::support::nested(&format!("inheritance.{outer}"), inner)
}
