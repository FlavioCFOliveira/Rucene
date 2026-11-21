//! Port of `com.carrotsearch.randomizedtesting.inheritance.JUnitAnnotationPropagation`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).
//!
//! **Divergence.** The test compares the order JUnit's
//! `BlockJUnit4ClassRunner` runs the hooks and tests of `Sub` in (through
//! `WithNestedTestClass.runTests`, `JUnitAnnotationPropagation.java:67`)
//! with the order `RandomizedRunner` runs them in. On JDK 25 the Java test
//! fails: JUnit runs `testMethod5`, whose `@Ignore` override in `Sub` drops
//! its `@Test`, and `RandomizedRunner` does not. The port has no JUnit
//! runner (`Request::a_class`), so both runs are `RandomizedRunner` runs and
//! the Java failure cannot be reproduced; the driver records this class as
//! the one whose outcome differs by construction.

use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::jdk::reflect::{Class, Field, Method, Value};
use randomizedtesting::junit::{
    Before, BeforeClass, Description, Ignore, Rule, Statement, Test, TestRule,
};

use super::{cls, nested};
use crate::support::{assert_that_list, run_randomized_runner, unit};
use crate::with_nested_test_class::run_tests;

const O: &str = "JUnitAnnotationPropagation";

/// `final static List<String> order`.
static ORDER: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn add(s: &str) {
    ORDER
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(s.into());
}

fn step(s: &'static str) -> impl Fn() + Send + Sync + 'static {
    move || add(s)
}

/// The anonymous `rules` of `Super`.
struct OrderRule;

impl TestRule for OrderRule {
    fn apply(&self, base: Statement, _d: &Arc<Description>) -> Statement {
        Statement::new(move || {
            add("rule-before");
            base.evaluate();
            add("rule-after");
        })
    }
}

fn plain(name: &str, s: &'static str) -> Method {
    Method::new(name).body_any(step(s))
}

fn plain_static(name: &str, s: &'static str) -> Method {
    Method::new(name).static_body(step(s))
}

static SUPER: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Super")
        .constructor(unit())
        .field(
            Field::new_instance::<()>("rules", "org.junit.rules.TestRule", |_| {
                Value::TestRule(Arc::new(OrderRule))
            })
            .annotate(Rule),
        )
        .method(plain_static("beforeClass1", "super.beforeclass1").annotate(BeforeClass))
        .method(plain_static("beforeClass2", "super.beforeclass2"))
        .method(plain_static("beforeClass3", "super.beforeclass3").annotate(BeforeClass))
        .method(plain("before1", "super.before1").annotate(Before))
        .method(plain("before2", "super.before2"))
        .method(plain("before3", "super.before3").annotate(Before))
        .method(plain("testMethod1", "super.testMethod1").annotate(Test::new()))
        .method(plain("testMethod2", "super.testMethod2"))
        .method(plain("testMethod3", "super.testMethod3").annotate(Test::new()))
        // Awkward cases of annotations and virtual methods.
        .method(plain("testMethod4", "super.testMethod4").annotate(Ignore::new()))
        .method(plain("testMethod5", "super.testMethod5").annotate(Test::new()))
        .build()
});

static SUB: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Sub")
        .extends(&SUPER)
        .constructor(unit())
        .method(plain_static("beforeClass1", "sub.beforeclass1"))
        .method(plain_static("beforeClass2", "sub.beforeclass2").annotate(BeforeClass))
        .method(plain_static("beforeClass3", "sub.beforeclass3").annotate(BeforeClass))
        .method(plain("before1", "sub.before1"))
        .method(plain("before2", "sub.before2").annotate(Before))
        .method(plain("before3", "sub.before3").annotate(Before))
        .method(plain("testMethod1", "sub.testMethod1"))
        .method(plain("testMethod2", "sub.testMethod2").annotate(Test::new()))
        .method(plain("testMethod3", "sub.testMethod3").annotate(Test::new()))
        .method(plain("testMethod4", "sub.testMethod4").annotate(Test::new()))
        .method(plain("testMethod5", "sub.testMethod5").annotate(Ignore::new()))
        .build()
});

fn take() -> Vec<String> {
    std::mem::take(&mut *ORDER.lock().unwrap_or_else(|e| e.into_inner()))
}

/// `noNumbers(in)`: each entry without its trailing digits.
fn no_numbers(v: &[String]) -> Vec<String> {
    v.iter()
        .map(|s| s.trim_end_matches(|c: char| c.is_ascii_digit()).to_string())
        .collect()
}

/// `JUnitAnnotationPropagation`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls(O)
        .constructor(unit())
        .method(
            Method::new("checkOldMethodRules")
                .annotate(Test::new())
                .body_any(|| {
                    take();
                    run_tests(std::slice::from_ref(&*SUB));
                    let mut order1 = take();
                    run_randomized_runner(&SUB);
                    let mut order2 = take();
                    let msg = format!(
                        "# JUnit order:\n[{}]\n# RR order:\n[{}]",
                        order1.join(", "),
                        order2.join(", ")
                    );
                    // Don't care about relative ordering of hook methods.
                    assert_that_list(no_numbers(&order2))
                        .as_(msg.clone())
                        .is_equal_to(&no_numbers(&order1));
                    // But do care that all methods show up.
                    order1.sort();
                    order2.sort();
                    assert_that_list(order2).as_(msg).is_equal_to(&order1);
                }),
        )
        .build()]
}
