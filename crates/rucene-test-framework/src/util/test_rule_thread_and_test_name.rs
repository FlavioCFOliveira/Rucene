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

//! Port of `org.apache.lucene.tests.util.TestRuleThreadAndTestName`.

use std::sync::{Arc, RwLock};

use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::{Description, Statement, TestRule};

/// The `volatile` fields of the rule.
#[derive(Debug)]
pub struct ThreadAndTestName {
    /// `testCaseThread`: "the thread executing the current test case."
    pub test_case_thread: Option<Thread>,
    /// `testMethodName`: "the name of the test method currently executing."
    pub test_method_name: Option<String>,
}

/// Port of `TestRuleThreadAndTestName`: "saves the executing thread and
/// method name of the test case."
#[derive(Debug)]
pub struct TestRuleThreadAndTestName {
    state: Arc<RwLock<ThreadAndTestName>>,
}

impl Default for TestRuleThreadAndTestName {
    fn default() -> Self {
        TestRuleThreadAndTestName {
            state: Arc::new(RwLock::new(ThreadAndTestName {
                test_case_thread: None,
                test_method_name: Some("<unknown>".to_string()),
            })),
        }
    }
}

impl TestRuleThreadAndTestName {
    /// `testCaseThread`.
    pub fn test_case_thread(&self) -> Option<Thread> {
        self.read().test_case_thread.clone()
    }

    /// `testMethodName`.
    pub fn test_method_name(&self) -> Option<String> {
        self.read().test_method_name.clone()
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, ThreadAndTestName> {
        self.state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl TestRule for TestRuleThreadAndTestName {
    /// `apply(base, description)`: sets both fields around the statement and
    /// clears them (to `null`) after it.
    fn apply(&self, base: Statement, description: &Arc<Description>) -> Statement {
        let state = Arc::clone(&self.state);
        let description = Arc::clone(description);
        Statement::new(move || {
            struct Clear<'a>(&'a RwLock<ThreadAndTestName>);
            impl Drop for Clear<'_> {
                fn drop(&mut self) {
                    let mut s = self
                        .0
                        .write()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    s.test_case_thread = None;
                    s.test_method_name = None;
                }
            }
            let _clear = Clear(&state);
            {
                let mut s = state
                    .write()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                s.test_case_thread = Some(Thread::current());
                s.test_method_name = description.method_name();
            }
            base.evaluate();
        })
    }
}
