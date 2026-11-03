//! `org.junit.runners.model.Statement`, `org.junit.rules.TestRule`,
//! `org.junit.rules.MethodRule`, `org.junit.rules.RuleChain`,
//! `org.junit.runners.model.FrameworkMethod` and
//! `org.junit.runner.manipulation.Filter`, rebuilt from their documented
//! behaviour.

use std::fmt;
use std::sync::Arc;

use super::description::Description;
use crate::jdk::reflect::{Instance, Method};

/// `org.junit.AssumptionViolatedException`.
pub const ASSUMPTION_VIOLATED: &str = "org.junit.AssumptionViolatedException";
/// `org.junit.internal.AssumptionViolatedException`.
pub const ASSUMPTION_VIOLATED_INTERNAL: &str = "org.junit.internal.AssumptionViolatedException";
/// `org.junit.runner.manipulation.NoTestsRemainException`.
pub const NO_TESTS_REMAIN: &str = "org.junit.runner.manipulation.NoTestsRemainException";

/// One or more actions to run (`Statement`): `evaluate()` runs them and
/// throws (panics with a [`Throwable`](crate::jdk::exception::Throwable))
/// on failure. A statement may be evaluated more than once and from any
/// thread.
#[derive(Clone)]
pub struct Statement(Arc<dyn Fn() + Send + Sync>);

impl Statement {
    /// A statement running `body`.
    pub fn new(body: impl Fn() + Send + Sync + 'static) -> Statement {
        Statement(Arc::new(body))
    }

    /// `evaluate()`.
    pub fn evaluate(&self) {
        (self.0)()
    }
}

impl fmt::Debug for Statement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Statement")
    }
}

/// `TestRule`: alters how a test (or a suite, as a class rule) runs.
pub trait TestRule: Send + Sync {
    /// `apply(Statement base, Description description)`: the statement to
    /// run in place of `base`.
    fn apply(&self, base: Statement, description: &Arc<Description>) -> Statement;

    /// `getClass().getName()` when the rule is a JUnit class whose fields the
    /// memory estimator walks (`RuleChain`); `None` for any other rule.
    fn java_class(&self) -> Option<&'static str> {
        None
    }
}

/// The method a `MethodRule` wraps (`FrameworkMethod`).
#[derive(Clone)]
pub struct FrameworkMethod {
    method: Arc<Method>,
}

impl FrameworkMethod {
    /// `new FrameworkMethod(method)`.
    pub fn new(method: Arc<Method>) -> FrameworkMethod {
        FrameworkMethod { method }
    }

    /// `getMethod()`.
    pub fn method(&self) -> &Arc<Method> {
        &self.method
    }

    /// `getName()`.
    pub fn name(&self) -> &str {
        self.method.name()
    }
}

/// The deprecated `MethodRule`: like [`TestRule`], with the method and the
/// test instance.
pub trait MethodRule: Send + Sync {
    /// `apply(Statement base, FrameworkMethod method, Object target)`.
    fn apply(&self, base: Statement, method: &FrameworkMethod, target: &Instance) -> Statement;
}

/// `RuleChain`: orders rules, the first (outer) one wrapping the rest.
#[derive(Clone, Default)]
pub struct RuleChain {
    rules_starting_with_inner_most: Vec<Arc<dyn TestRule>>,
}

impl RuleChain {
    /// `RuleChain.emptyRuleChain()`.
    pub fn empty_rule_chain() -> RuleChain {
        RuleChain::default()
    }

    /// `RuleChain.outerRule(rule)`.
    pub fn outer_rule(rule: Arc<dyn TestRule>) -> RuleChain {
        RuleChain::empty_rule_chain().around(rule)
    }

    /// `around(enclosedRule)`: a chain with `rule` inside the current ones.
    pub fn around(&self, rule: Arc<dyn TestRule>) -> RuleChain {
        let mut rules = vec![rule];
        rules.extend(self.rules_starting_with_inner_most.iter().cloned());
        RuleChain {
            rules_starting_with_inner_most: rules,
        }
    }
}

impl TestRule for RuleChain {
    /// Applies the innermost rule first, so the outer rule runs outermost.
    fn apply(&self, base: Statement, description: &Arc<Description>) -> Statement {
        let mut s = base;
        for rule in &self.rules_starting_with_inner_most {
            s = rule.apply(s, description);
        }
        s
    }

    /// `org.junit.rules.RuleChain`, whose `rulesStartingWithInnerMost` is a
    /// `java.util.ArrayList`.
    fn java_class(&self) -> Option<&'static str> {
        Some("org.junit.rules.RuleChain")
    }
}

/// `org.junit.runner.manipulation.Filter`.
pub trait Filter: Send + Sync {
    /// `shouldRun(description)`.
    fn should_run(&self, description: &Description) -> bool;

    /// `describe()`.
    fn describe(&self) -> String;
}
