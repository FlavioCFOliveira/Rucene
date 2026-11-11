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

//! Port of `org.apache.lucene.tests.util.TestRuleIgnoreAfterMaxFailures`.

use std::sync::Arc;

use randomizedtesting::junit::{Assert, Description, Statement, TestRule};
use randomizedtesting::RandomizedTest;

use crate::util::failure_marker::FailureMarker;

/// Port of `TestRuleIgnoreAfterMaxFailures`: "this rule keeps a count of
/// failed tests (suites) and will result in an `AssumptionViolatedException`
/// after a given number of failures for all tests following this condition."
#[derive(Debug, Clone, Copy)]
pub struct TestRuleIgnoreAfterMaxFailures {
    /// `maxFailures`: "maximum failures. Package scope for tests."
    pub max_failures: i32,
}

impl TestRuleIgnoreAfterMaxFailures {
    /// `new TestRuleIgnoreAfterMaxFailures(maxFailures)`.
    ///
    /// # Panics
    ///
    /// `AssertionError` ("maxFailures must be >= 1: …") for `max_failures < 1`.
    pub fn new(max_failures: i32) -> TestRuleIgnoreAfterMaxFailures {
        Assert::assert_true_msg(
            &format!("maxFailures must be >= 1: {max_failures}"),
            max_failures >= 1,
        );
        TestRuleIgnoreAfterMaxFailures { max_failures }
    }
}

impl TestRule for TestRuleIgnoreAfterMaxFailures {
    /// `apply(s, d)`: an assumption failure once `FailureMarker.getFailures()`
    /// reached `maxFailures`.
    fn apply(&self, s: Statement, _d: &Arc<Description>) -> Statement {
        let max_failures = self.max_failures;
        Statement::new(move || {
            let failures_so_far = FailureMarker::get_failures();
            if failures_so_far >= max_failures {
                RandomizedTest::assume_true_msg(
                    &format!(
                        "Ignored, failures limit reached ({failures_so_far} >= {max_failures})."
                    ),
                    false,
                );
            }
            s.evaluate();
        })
    }
}
