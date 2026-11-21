//! Helpers shared by the `timeouts` suites: a nested suite extending
//! `WithNestedTestClass.ApplyAtPlace`, and `check(place)`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, ClassBuilder};

use crate::with_nested_test_class::{Place, APPLY_AT_PLACE, APPLY_PLACE, APPLY_RUNNABLE};

/// A builder for `com.carrotsearch.randomizedtesting.timeouts.<name>`.
pub fn cls(name: &str) -> ClassBuilder {
    crate::support::cls(&format!("timeouts.{name}"))
}

/// A builder for the nested `timeouts.<outer>$<inner>`.
pub fn nested(outer: &str, inner: &str) -> ClassBuilder {
    crate::support::nested(&format!("timeouts.{outer}"), inner)
}

/// `public static class Nested extends ApplyAtPlace {}` with the class
/// annotations `annotate` adds.
pub fn apply_at_place_suite(
    outer: &str,
    annotate: impl FnOnce(ClassBuilder) -> ClassBuilder,
) -> Arc<Class> {
    annotate(
        nested(outer, "Nested")
            .extends(&APPLY_AT_PLACE)
            .constructor(crate::with_nested_test_class::apply_at_place_constructor()),
    )
    .build()
}

/// `ApplyAtPlace.place = p; ApplyAtPlace.runnable = runnable;`.
pub fn set_place(p: Place, runnable: impl Fn() + Send + Sync + 'static) {
    *APPLY_PLACE.lock().unwrap_or_else(|e| e.into_inner()) = Some(p);
    *APPLY_RUNNABLE.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(runnable));
}

/// One `@Test` per place, each calling `check(place)`.
pub fn place_tests(b: ClassBuilder, places: &[(&str, Place)], check: fn(Place)) -> ClassBuilder {
    let mut b = b;
    for (name, p) in places {
        let p = *p;
        b = b.method(crate::support::test(name, move || check(p)));
    }
    b
}

/// Every place, with Java's test names.
pub const ALL_PLACES: [(&str, Place); 8] = [
    ("testClassRule", Place::ClassRule),
    ("testBeforeClass", Place::BeforeClass),
    ("testConstructor", Place::Constructor),
    ("testTestRule", Place::TestRule),
    ("testBefore", Place::Before),
    ("testTest", Place::Test),
    ("testAfter", Place::After),
    ("testAfterClass", Place::AfterClass),
];
