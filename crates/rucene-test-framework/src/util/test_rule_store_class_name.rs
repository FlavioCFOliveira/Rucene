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

//! Port of `org.apache.lucene.tests.util.TestRuleStoreClassName`.
//!
//! Ported in full under the sprint's cycle rule: `LuceneTestCase.getTestClass()`
//! (task #338) returns `classNameRule.getTestClass()`. The rule is installed
//! in `LuceneTestCase`'s class rules by task #348.

use std::sync::{Arc, RwLock};

use randomizedtesting::jdk::exception::{self, ILLEGAL_ARGUMENT_EXCEPTION, RUNTIME_EXCEPTION};
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{Description, Statement, TestRule};

/// Port of `org.apache.lucene.tests.util.TestRuleStoreClassName`: stores the
/// suite name so you can retrieve it from [`get_test_class`](Self::get_test_class).
#[derive(Default)]
pub struct TestRuleStoreClassName {
    /// `volatile Description description`.
    description: Arc<RwLock<Option<Arc<Description>>>>,
}

impl TestRuleStoreClassName {
    /// `new TestRuleStoreClassName()`.
    pub fn new() -> TestRuleStoreClassName {
        TestRuleStoreClassName::default()
    }

    /// `getTestClass()`: the test class currently executing in this rule.
    ///
    /// # Panics
    ///
    /// `RuntimeException` ("The rule is not currently executing.") outside
    /// the rule.
    pub fn get_test_class(&self) -> Arc<Class> {
        let local = self
            .description
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let Some(description) = local else {
            exception::throw(RUNTIME_EXCEPTION, "The rule is not currently executing.")
        };
        match description.test_class() {
            Some(class) => Arc::clone(class),
            None => exception::throw_without_message(exception::NULL_POINTER_EXCEPTION),
        }
    }
}

impl TestRule for TestRuleStoreClassName {
    /// `apply(s, d)`: a class rule only; the description is stored while the
    /// suite runs.
    fn apply(&self, base: Statement, description: &Arc<Description>) -> Statement {
        if !description.is_suite() {
            exception::throw(
                ILLEGAL_ARGUMENT_EXCEPTION,
                "This is a @ClassRule (applies to suites only).",
            )
        }
        let slot = Arc::clone(&self.description);
        let d = Arc::clone(description);
        Statement::new(move || {
            struct Reset<'a>(&'a RwLock<Option<Arc<Description>>>);
            impl Drop for Reset<'_> {
                fn drop(&mut self) {
                    *self
                        .0
                        .write()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
                }
            }
            *slot
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::clone(&d));
            let _reset = Reset(&slot);
            base.evaluate();
        })
    }
}
