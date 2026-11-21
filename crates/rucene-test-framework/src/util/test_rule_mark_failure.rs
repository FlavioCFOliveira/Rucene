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

//! Port of `org.apache.lucene.tests.util.TestRuleMarkFailure`, in full under
//! the sprint's cycle rule: `TestRuleTemporaryFilesCleanup.afterAlways` asks
//! its failure marker `wasSuccessful()`.

use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::exception::{panic_message, Throwable};
use randomizedtesting::junit::{Description, Statement, TestRule};

/// `org.junit.AssumptionViolatedException`'s superclass, which an assumption
/// failure is an instance of.
const ASSUMPTION_VIOLATED_EXCEPTION: &str = "org.junit.internal.AssumptionViolatedException";

/// Port of `org.apache.lucene.tests.util.TestRuleMarkFailure`: a rule that
/// marks the failures of the statements it wraps, and of its chained markers.
#[derive(Default)]
pub struct TestRuleMarkFailure {
    /// `chained`: the markers marked failed with this one.
    chained: Vec<Arc<TestRuleMarkFailure>>,
    /// `volatile boolean failures`.
    failures: AtomicBool,
}

impl TestRuleMarkFailure {
    /// `new TestRuleMarkFailure(chained...)`.
    pub fn new(chained: Vec<Arc<TestRuleMarkFailure>>) -> TestRuleMarkFailure {
        TestRuleMarkFailure {
            chained,
            failures: AtomicBool::new(false),
        }
    }

    /// `isAssumption(t)`: whether `t`, and every failure a
    /// `MultipleFailureException` holds, is an `AssumptionViolatedException`.
    pub fn is_assumption(t: &Throwable) -> bool {
        Self::expand_from_multiple(t)
            .iter()
            .all(|t2| t2.is_instance(ASSUMPTION_VIOLATED_EXCEPTION))
    }

    /// `expandFromMultiple(t)`: the failures of a `MultipleFailureException`,
    /// recursively, or `t` itself.
    fn expand_from_multiple(t: &Throwable) -> Vec<Throwable> {
        let mut list = Vec::new();
        Self::expand_into(t, &mut list);
        list
    }

    fn expand_into(t: &Throwable, list: &mut Vec<Throwable>) {
        if t.is_instance("org.junit.runners.model.MultipleFailureException") {
            for sub in t.failures() {
                Self::expand_into(sub, list);
            }
        } else {
            list.push(t.clone());
        }
    }

    /// `markFailed()`: this marker and every chained one.
    pub fn mark_failed(&self) {
        self.failures.store(true, Ordering::SeqCst);
        for next in &self.chained {
            next.mark_failed();
        }
    }

    /// `hadFailures()`.
    pub fn had_failures(&self) -> bool {
        self.failures.load(Ordering::SeqCst)
    }

    /// `wasSuccessful()`: the opposite of [`had_failures`](Self::had_failures).
    pub fn was_successful(&self) -> bool {
        !self.had_failures()
    }
}

/// The marker a rule chain holds.
pub struct TestRuleMarkFailureRule(pub Arc<TestRuleMarkFailure>);

impl TestRule for TestRuleMarkFailureRule {
    /// `apply(s, d)`: clears the status at start, and marks a failure that is
    /// not an assumption before rethrowing it.
    fn apply(&self, base: Statement, _description: &Arc<Description>) -> Statement {
        let marker = Arc::clone(&self.0);
        Statement::new(move || {
            // Clear status at start.
            marker.failures.store(false, Ordering::SeqCst);
            if let Err(payload) = catch_unwind(AssertUnwindSafe(|| base.evaluate())) {
                let t = match payload.downcast_ref::<Throwable>() {
                    Some(t) => t.clone(),
                    None => Throwable::from_text(&panic_message(payload.as_ref())),
                };
                if !TestRuleMarkFailure::is_assumption(&t) {
                    marker.mark_failed();
                }
                resume_unwind(payload);
            }
        })
    }

    fn java_class(&self) -> Option<&'static str> {
        Some("org.apache.lucene.tests.util.TestRuleMarkFailure")
    }
}
