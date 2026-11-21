//! Port of `com.carrotsearch.randomizedtesting.rules.StatementAdapter`
//! (randomizedtesting 2.8.1, `rules/StatementAdapter.java`).

use std::sync::Arc;

use crate::jdk::exception::Throwable;
use crate::junit::Statement;
use crate::randomized_runner::{assert_empty, try_run};

type Hook = Arc<dyn Fn() + Send + Sync>;
type AfterAlways = Arc<dyn Fn(&mut Vec<Throwable>) + Send + Sync>;

/// A statement that runs `afterAlways` even when the delegate throws
/// (`StatementAdapter`): `before`, the delegate and `afterIfSuccessful` run
/// in turn until one throws; `afterAlways` then receives the errors so far
/// and may add more; the errors are thrown as
/// `MultipleFailureException.assertEmpty` does.
///
/// **Divergence.** Java subclasses override the three hooks
/// (`StatementAdapter.java:19`, `:54`); the port sets
/// them as closures on a builder.
#[derive(Clone)]
pub struct StatementAdapter {
    delegate: Statement,
    before: Option<Hook>,
    after_if_successful: Option<Hook>,
    after_always: Option<AfterAlways>,
}

impl StatementAdapter {
    /// `new StatementAdapter(delegate)` with no-op hooks.
    pub fn new(delegate: Statement) -> StatementAdapter {
        StatementAdapter {
            delegate,
            before: None,
            after_if_successful: None,
            after_always: None,
        }
    }

    /// Overrides `before()`.
    pub fn before(mut self, f: impl Fn() + Send + Sync + 'static) -> StatementAdapter {
        self.before = Some(Arc::new(f));
        self
    }

    /// Overrides `afterIfSuccessful()`.
    pub fn after_if_successful(mut self, f: impl Fn() + Send + Sync + 'static) -> StatementAdapter {
        self.after_if_successful = Some(Arc::new(f));
        self
    }

    /// Overrides `afterAlways(errors)`.
    pub fn after_always(
        mut self,
        f: impl Fn(&mut Vec<Throwable>) + Send + Sync + 'static,
    ) -> StatementAdapter {
        self.after_always = Some(Arc::new(f));
        self
    }

    /// The statement whose `evaluate()` is `StatementAdapter.evaluate()`.
    pub fn build(self) -> Statement {
        Statement::new(move || self.evaluate())
    }

    /// `evaluate()`.
    pub fn evaluate(&self) {
        let mut errors: Vec<Throwable> = Vec::new();
        if let Err(t) = try_run(|| {
            if let Some(b) = &self.before {
                b();
            }
            self.delegate.evaluate();
            if let Some(a) = &self.after_if_successful {
                a();
            }
        }) {
            errors.push(t);
        }
        if let Some(a) = &self.after_always {
            let mut list = std::mem::take(&mut errors);
            let r = try_run(std::panic::AssertUnwindSafe(|| a(&mut list)));
            errors = list;
            if let Err(t) = r {
                errors.push(t);
            }
        }
        assert_empty(errors);
    }
}
