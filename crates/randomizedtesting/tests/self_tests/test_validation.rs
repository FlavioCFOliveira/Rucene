//! Port of `com.carrotsearch.randomizedtesting.TestValidation`
//! (randomized-runner 2.8.1 test sources, Apache-2.0): the runner rejects
//! malformed suite classes and hooks.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{modifier, Class, Constructor, Method};
use randomizedtesting::junit::{After, AfterClass, Before, BeforeClass};
use randomizedtesting::RandomizedRunner;

use crate::support::{cls, fail_msg, nested, test, unit};
use crate::with_nested_test_class::CLASS as WITH_NESTED;

const O: &str = "TestValidation";

fn with_hook(name: &str, m: Method) -> Arc<Class> {
    nested(O, name).constructor(unit()).method(m).build()
}

fn suites() -> Vec<Arc<Class>> {
    vec![
        // static class SuiteClassNotPublic {}
        Class::builder(format!("{}{O}$SuiteClassNotPublic", crate::support::P))
            .modifiers(modifier::STATIC)
            .constructor(unit().modifiers(0))
            .build(),
        // public class SuiteClassNotStatic {} (an inner class: its constructor takes the outer instance)
        Class::builder(format!("{}{O}$SuiteClassNotStatic", crate::support::P))
            .modifiers(modifier::PUBLIC)
            .constructor(Constructor::with_params(
                &[("com.carrotsearch.randomizedtesting.TestValidation", None)],
                |_| (),
            ))
            .build(),
        with_hook(
            "BeforeClassNotStatic",
            Method::new("beforeClass")
                .annotate(BeforeClass)
                .body_any(|| {}),
        ),
        with_hook(
            "BeforeClassWithArgs",
            Method::new("beforeClass")
                .annotate(BeforeClass)
                .params(&["int"])
                .static_body(|| {}),
        ),
        with_hook(
            "AfterClassNotStatic",
            Method::new("afterClass")
                .annotate(AfterClass)
                .body_any(|| {}),
        ),
        with_hook(
            "AfterClassWithArgs",
            Method::new("afterClass")
                .annotate(AfterClass)
                .params(&["int"])
                .static_body(|| {}),
        ),
        with_hook(
            "BeforeStatic",
            Method::new("before").annotate(Before).static_body(|| {}),
        ),
        with_hook(
            "BeforeWithArgs",
            Method::new("before")
                .annotate(Before)
                .params(&["int"])
                .body_any(|| {}),
        ),
        with_hook(
            "AfterStatic",
            Method::new("after").annotate(After).static_body(|| {}),
        ),
        with_hook(
            "AfterWithArgs",
            Method::new("after")
                .annotate(After)
                .params(&["int"])
                .body_any(|| {}),
        ),
    ]
}

/// `TestValidation extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls(O)
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(test("checkBeforeClass", || {
            for c in suites() {
                if RandomizedRunner::new(&c).is_ok() {
                    fail_msg(&format!("Expected validation failure on: {}", c.name()));
                }
                // Ok, expected.
            }
        }))
        .build()]
}
