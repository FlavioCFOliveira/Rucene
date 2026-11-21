//! Port of `com.carrotsearch.randomizedtesting.rules.TestRuleAdapter`
//! (randomizedtesting 2.8.1, `rules/TestRuleAdapter.java`).

use std::sync::Arc;

use super::StatementAdapter;
use crate::jdk::exception::Throwable;
use crate::junit::{Description, Statement, TestRule};

/// A rule whose statement runs `after_always` even when the base throws
/// (`TestRuleAdapter`). Implement the hooks; wrap the value in
/// [`TestRuleAdapterRule`] to use it as a [`TestRule`].
pub trait TestRuleAdapter: Send + Sync + 'static {
    /// `before()`: always called before the base statement.
    fn before(&self) {}
    /// `afterAlways(errors)`: always called after the base statement.
    fn after_always(&self, _errors: &mut Vec<Throwable>) {}
    /// `afterIfSuccessful()`: called only when the base statement returned.
    fn after_if_successful(&self) {}
}

/// The [`TestRule`] of a [`TestRuleAdapter`] (`TestRuleAdapter.apply`).
pub struct TestRuleAdapterRule<T: TestRuleAdapter>(pub Arc<T>);

impl<T: TestRuleAdapter> TestRule for TestRuleAdapterRule<T> {
    fn apply(&self, base: Statement, _description: &Arc<Description>) -> Statement {
        let a = Arc::clone(&self.0);
        let b = Arc::clone(&self.0);
        let c = Arc::clone(&self.0);
        StatementAdapter::new(base)
            .before(move || a.before())
            .after_always(move |errors| b.after_always(errors))
            .after_if_successful(move || c.after_if_successful())
            .build()
    }
}
