/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `org.apache.lucene.tests.util.AbstractBeforeAfterRule`.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::junit::{Description, Statement, TestRule};

/// Port of `AbstractBeforeAfterRule`: "a `TestRule` that guarantees the
/// execution of `after()` even if an exception has been thrown from
/// delegate `Statement`. This is much like `AfterClass` or `After`
/// annotations but can be used with `RuleChain` to guarantee the order of
/// execution."
///
/// Java subclasses override `before()` and `after()`; a Rust type implements
/// them and [`BeforeAfterRule`] is its `TestRule`.
pub trait AbstractBeforeAfterRule: Send + Sync + 'static {
    /// `before()`: runs before the statement; a panic is a thrown exception.
    fn before(&self, _description: &Arc<Description>) {}
    /// `after()`: always runs after the statement.
    fn after(&self) {}
}

/// The `TestRule` of an [`AbstractBeforeAfterRule`]
/// (`AbstractBeforeAfterRule.apply`).
pub struct BeforeAfterRule<T: AbstractBeforeAfterRule>(pub Arc<T>);

impl<T: AbstractBeforeAfterRule> TestRule for BeforeAfterRule<T> {
    /// `apply(s, d)`: `before()` then the statement, collecting the error;
    /// `after()`, collecting its error; then
    /// `MultipleFailureException.assertEmpty(errors)`.
    fn apply(&self, s: Statement, d: &Arc<Description>) -> Statement {
        let rule = Arc::clone(&self.0);
        let d = Arc::clone(d);
        Statement::new(move || {
            let mut errors = Vec::new();
            if let Err(t) = try_run(|| {
                rule.before(&d);
                s.evaluate();
            }) {
                errors.push(t);
            }
            if let Err(t) = try_run(|| rule.after()) {
                errors.push(t);
            }
            assert_empty(errors);
        })
    }
}

/// Runs `f`, converting a panic into its [`Throwable`] (Java's
/// `catch (Throwable t)`).
pub(crate) fn try_run<R>(f: impl FnOnce() -> R) -> Result<R, Throwable> {
    catch_unwind(AssertUnwindSafe(f)).map_err(Throwable::from_panic)
}

/// `MultipleFailureException.assertEmpty(errors)`: nothing for no error,
/// the error itself for one, a `MultipleFailureException` for more.
pub(crate) fn assert_empty(mut errors: Vec<Throwable>) {
    match errors.len() {
        0 => {}
        1 => errors.remove(0).throw(),
        _ => Throwable::multiple_failures(errors).throw(),
    }
}
