//! The JUnit 4.13.1 pieces randomizedtesting's runner is written against:
//! `Description`, `Statement`, `TestRule`, `MethodRule`, `RuleChain`,
//! `RunListener`, `RunNotifier`, `Failure`, `Result`, `Filter`, `Assert`,
//! `Assume`, the annotations (`@Test`, `@Before`, `@After`, `@BeforeClass`,
//! `@AfterClass`, `@Ignore`, `@Rule`, `@ClassRule`) and `JUnitCore`.
//!
//! **Divergence.** These are not a port of JUnit: JUnit is EPL-1.0, so no
//! JUnit source is copied. Each piece is rebuilt from JUnit's documented
//! behaviour (javadoc) and checked against JUnit 4.13.1 through the runner
//! oracle (`tests/fixtures/oracle/`), to the extent randomizedtesting
//! observes it. Members randomizedtesting never calls are not provided.

mod annotations;
mod assert;
mod core;
mod description;
mod notification;
mod statement;

pub use annotations::{After, AfterClass, Before, BeforeClass, ClassRule, Ignore, Rule, Test};
pub use assert::{Assert, Assume};
pub use core::{InitializationError, JUnitCore, Request, Runner};
pub use description::Description;
pub use notification::{
    Failure, JUnitResult, ListenerRef, Notifier, RunListener, RunNotifier, TextListener,
    STOPPED_BY_USER,
};
pub use statement::{
    Filter, FrameworkMethod, MethodRule, RuleChain, Statement, TestRule, ASSUMPTION_VIOLATED,
    ASSUMPTION_VIOLATED_INTERNAL, NO_TESTS_REMAIN,
};
