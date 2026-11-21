//! Port of `com.carrotsearch.randomizedtesting.contracts.TestBeforeAfterMethodOrder`
//! (randomized-runner 2.8.1 test sources, Apache-2.0): hook and rule order in
//! class hierarchies.
//!
//! `checkOrder` compares the order of a nested run (JUnit's runner in Java,
//! `RandomizedRunner` in the port; see the target's divergences) with the
//! order of a direct `RandomizedRunner` run.

use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::annotations::{Repeat, Seed};
use randomizedtesting::jdk::exception::RUNTIME_EXCEPTION;
use randomizedtesting::jdk::reflect::{modifier, Class, Field, Instance, Method, Value};
use randomizedtesting::jdk::system;
use randomizedtesting::junit::{
    After, AfterClass, Before, BeforeClass, FrameworkMethod, MethodRule, Rule, RuleChain, Statement,
};
use randomizedtesting::RandomizedContext;

use super::{cls, nested, AppendRule};
use crate::support::{
    assert_equals, fail_msg, hook, run_randomized_runner, static_hook, test, throw, unit,
};
use crate::with_nested_test_class::{
    assume_running_nested, check_tests_output, CLASS as WITH_NESTED,
};

const O: &str = "TestBeforeAfterMethodOrder";

/// `static final List<String> callOrder`.
static CALL_ORDER: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn add(s: String) {
    CALL_ORDER.lock().unwrap_or_else(|e| e.into_inner()).push(s);
}

fn step(s: &'static str) -> impl Fn() + Send + Sync + 'static {
    move || add(s.into())
}

/// `AppendMethodRule implements MethodRule`.
struct AppendMethodRule(&'static str);

impl MethodRule for AppendMethodRule {
    fn apply(&self, base: Statement, _m: &FrameworkMethod, _target: &Instance) -> Statement {
        let text = self.0;
        Statement::new(move || {
            add(format!("{text}-before"));
            base.evaluate();
            add(format!("{text}-after"));
        })
    }
}

fn append_rule(text: &str) -> Arc<AppendRule> {
    Arc::new(AppendRule {
        text: text.into(),
        sink: add,
    })
}

fn chain(outer: &str, middle: &str, inner: &str) -> Value {
    Value::TestRule(Arc::new(
        RuleChain::outer_rule(append_rule(outer))
            .around(append_rule(middle))
            .around(append_rule(inner)),
    ))
}

static SUPER: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "Super")
        .constructor(unit())
        .method(static_hook(
            "beforeClassSuper",
            BeforeClass,
            step("beforeClassSuper"),
        ))
        .field(
            Field::new_instance::<()>("superRule", "org.junit.rules.TestRule", |_| {
                chain(
                    "superOuterTestRule",
                    "superMiddleTestRule",
                    "superInnerTestRule",
                )
            })
            .annotate(Rule),
        )
        .field(
            Field::new_instance::<()>("superMethodRule", "org.junit.rules.MethodRule", |_| {
                Value::MethodRule(Arc::new(AppendMethodRule("superMethodRule")))
            })
            .annotate(Rule),
        )
        .method(
            Method::new("beforeTest")
                .modifiers(modifier::PUBLIC | modifier::FINAL)
                .annotate(Before)
                .body_any(step("beforeTestSuper")),
        )
        .method(
            Method::new("testMethod")
                .modifiers(modifier::PROTECTED)
                .body_any(|| throw(RUNTIME_EXCEPTION, "Should be overridden and public.")),
        )
        .method(
            Method::new("afterTest")
                .modifiers(modifier::PUBLIC | modifier::FINAL)
                .annotate(After)
                .body_any(step("afterTestSuper")),
        )
        .method(static_hook(
            "afterClassSuper",
            AfterClass,
            step("afterClassSuper"),
        ))
        .build()
});

static SUB_SUB: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "SubSub")
        .extends(&SUPER)
        .constructor(unit())
        .field(
            Field::new_instance::<()>("rule", "org.junit.rules.TestRule", |_| {
                chain(
                    "  subOuterTestRule",
                    "  subMiddleTestRule",
                    "  subInnerTestRule",
                )
            })
            .annotate(Rule),
        )
        .field(
            Field::new_instance::<()>("methodRule", "org.junit.rules.MethodRule", |_| {
                Value::MethodRule(Arc::new(AppendMethodRule("  subMethodRule")))
            })
            .annotate(Rule),
        )
        .method(static_hook(
            "beforeClass",
            BeforeClass,
            step("  beforeClassSub"),
        ))
        .method(hook("beforeTestSub", Before, step("  beforeTestSub")))
        .method(test("testMethod", step("    testMethodSub")))
        .method(hook("afterTestSub", After, step("  afterTestSub")))
        .method(static_hook(
            "afterClass",
            AfterClass,
            step("  afterClassSub"),
        ))
        .build()
});

fn draw_step(prefix: &'static str) -> impl Fn() + Send + Sync + 'static {
    move || {
        add(format!(
            "{prefix} {}",
            RandomizedContext::current().get_random().next_int()
        ))
    }
}

const TEN: Repeat = Repeat {
    iterations: 10,
    use_constant_seed: false,
};

static SUB_SUB_FIXED_SEED: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "SubSubFixedSeed")
        .extends(&SUPER)
        .annotate(Seed::new("deadbeef"))
        .constructor(unit())
        .method(static_hook(
            "beforeClass",
            BeforeClass,
            step("beforeClassSubFS"),
        ))
        .method(hook("beforeTestSub", Before, || {
            assume_running_nested();
            add("beforeTestSubFS".into());
        }))
        .method(test("testMethod1", draw_step("testMethodSubFS1")).annotate(TEN))
        .method(test("testMethod2", draw_step("testMethodSubFS2")).annotate(TEN))
        .method(hook("afterTestSub", After, step("afterTestSubFS")))
        .method(static_hook(
            "afterClass",
            AfterClass,
            step("afterClassSubFS"),
        ))
        .build()
});

fn take() -> Vec<String> {
    std::mem::take(&mut *CALL_ORDER.lock().unwrap_or_else(|e| e.into_inner()))
}

/// `TestBeforeAfterMethodOrder extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls(O)
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(hook("cleanup", Before, || {
            take();
        }))
        .method(test("checkOrder", || {
            // Normal JUnit.
            check_tests_output(1, 0, 0, 0, &SUB_SUB);
            let junit_order = take();
            run_randomized_runner(&SUB_SUB);
            let call_order = take();
            if call_order != junit_order {
                let (i, j) = (junit_order.len(), call_order.len());
                system::out_println(&format!("{:<30} | {:<30}", "JUnit4", "RR"));
                for k in 0..i.max(j) {
                    system::out_println(&format!(
                        "{:<30} | {:<30}",
                        junit_order.get(k).map(String::as_str).unwrap_or("--"),
                        call_order.get(k).map(String::as_str).unwrap_or("--")
                    ));
                }
                fail_msg("JUnit4 and RandomizedRunner differed.");
            }
        }))
        .method(test("checkOrderFixedSeed", || {
            run_randomized_runner(&SUB_SUB_FIXED_SEED);
            let order = take();
            run_randomized_runner(&SUB_SUB_FIXED_SEED);
            let again = take();
            assert_equals(format!("{order:?}"), format!("{again:?}"));
        }))
        .build()]
}
