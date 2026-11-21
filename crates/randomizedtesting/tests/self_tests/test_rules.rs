//! Port of `com.carrotsearch.randomizedtesting.TestRules`
//! (randomized-runner 2.8.1 test sources, Apache-2.0): `@Rule` with
//! `MethodRule` and `TestRule`, as under JUnit.

use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::jdk::exception::RUNTIME_EXCEPTION;
use randomizedtesting::jdk::reflect::{Class, Field, Instance, Value};
use randomizedtesting::junit::{
    Description, FrameworkMethod, Ignore, MethodRule, Rule, RuleChain, Statement, TestRule,
};

use crate::support::{
    assert_equals, cls, nested, run_randomized_runner, test, throw_no_message, unit,
};
use crate::with_nested_test_class::{assume_running_nested, run_tests, CLASS as WITH_NESTED};

const O: &str = "TestRules";

/// `static List<String> order` (`None` for `null`).
static ORDER: Mutex<Option<Vec<String>>> = Mutex::new(None);

fn add(s: String) {
    match ORDER.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        Some(v) => v.push(s),
        None => randomizedtesting::jdk::exception::throw_without_message(
            randomizedtesting::jdk::exception::NULL_POINTER_EXCEPTION,
        ),
    }
}

/// The anonymous `MethodRule` of `MethodRuleSupport.rule1`.
struct Rule1;

impl MethodRule for Rule1 {
    fn apply(&self, base: Statement, _m: &FrameworkMethod, _target: &Instance) -> Statement {
        Statement::new(move || {
            assume_running_nested();
            add("rule1-before".into());
            base.evaluate();
            add("rule1-after".into());
        })
    }
}

static METHOD_RULE_SUPPORT: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "MethodRuleSupport")
        .constructor(unit())
        .field(
            Field::new_instance::<()>("rule1", "org.junit.rules.MethodRule", |_| {
                Value::MethodRule(Arc::new(Rule1))
            })
            .annotate(Rule),
        )
        .method(test("passing", || add("passing".into())))
        .build()
});

/// `SimpleRule implements TestRule`.
struct SimpleRule(&'static str);

impl TestRule for SimpleRule {
    fn apply(&self, base: Statement, _d: &Arc<Description>) -> Statement {
        let msg = self.0;
        Statement::new(move || {
            assume_running_nested();
            add(format!("{msg}-before"));
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| base.evaluate()));
            add(format!("{msg}-after"));
            if let Err(p) = r {
                std::panic::resume_unwind(p);
            }
        })
    }
}

fn chain_field() -> Field {
    Field::new_instance::<()>("rule1", "org.junit.rules.TestRule", |_| {
        Value::TestRule(Arc::new(
            RuleChain::outer_rule(Arc::new(SimpleRule("outer")))
                .around(Arc::new(SimpleRule("inner"))),
        ))
    })
    .annotate(Rule)
}

static NEW_RULE_SUPPORT_PASSING: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "NewRuleSupportPassing")
        .constructor(unit())
        .field(chain_field())
        .method(test("passing", || add("passing".into())))
        .build()
});

static NEW_RULE_SUPPORT_FAILING: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "NewRuleSupportFailing")
        .constructor(unit())
        .field(chain_field())
        .method(test("failing", || {
            add("failing".into());
            throw_no_message(RUNTIME_EXCEPTION)
        }))
        .build()
});

static NEW_RULE_SUPPORT_IGNORED: LazyLock<Arc<Class>> = LazyLock::new(|| {
    nested(O, "NewRuleSupportIgnored")
        .constructor(unit())
        .field(chain_field())
        .method(
            test("ignored", || {
                add("ignored".into());
                throw_no_message(RUNTIME_EXCEPTION)
            })
            .annotate(Ignore::new()),
        )
        .build()
});

/// `assertSameExecution(clazz)`.
fn assert_same_execution(clazz: &Arc<Class>) {
    *ORDER.lock().unwrap_or_else(|e| e.into_inner()) = Some(Vec::new());
    run_tests(std::slice::from_ref(clazz));
    let order1 = ORDER
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .replace(Vec::new());
    run_randomized_runner(clazz);
    let order2 = ORDER.lock().unwrap_or_else(|e| e.into_inner()).take();
    let f = |v: Option<Vec<String>>| format!("[{}]", v.unwrap_or_default().join(", "));
    assert_equals(f(order1), f(order2));
}

/// `TestRules extends WithNestedTestClass`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls(O)
        .extends(&WITH_NESTED)
        .constructor(unit())
        .method(test("checkOldMethodRules", || {
            assert_same_execution(&METHOD_RULE_SUPPORT)
        }))
        .method(test("checkNewTestRules", || {
            assert_same_execution(&NEW_RULE_SUPPORT_PASSING);
            assert_same_execution(&NEW_RULE_SUPPORT_FAILING);
            assert_same_execution(&NEW_RULE_SUPPORT_IGNORED);
        }))
        .build()]
}
