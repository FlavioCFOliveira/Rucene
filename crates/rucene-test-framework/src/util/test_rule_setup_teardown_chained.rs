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

//! Port of `org.apache.lucene.tests.util.TestRuleSetupTeardownChained`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use randomizedtesting::junit::{Assert, Description, Statement, TestRule};

/// Port of `TestRuleSetupTeardownChained`: "make sure `LuceneTestCase.setUp()`
/// and `LuceneTestCase.tearDown()` were invoked even if they have been
/// overridden. We assume nobody will call these out of non-overridden
/// methods (they have to be public by contract, unfortunately). The top-level
/// methods just set a flag that is checked upon successful execution of each
/// test case."
#[derive(Debug, Default)]
pub struct TestRuleSetupTeardownChained {
    /// `setupCalled`.
    pub setup_called: Arc<AtomicBool>,
    /// `teardownCalled`.
    pub teardown_called: Arc<AtomicBool>,
}

impl TestRule for TestRuleSetupTeardownChained {
    /// `apply(base, description)`: clears both flags, runs the statement and
    /// fails when an override did not propagate the call; "I assume we don't
    /// want to check teardown chaining if something happens in the test
    /// because this would obscure the original exception?"
    fn apply(&self, base: Statement, _description: &Arc<Description>) -> Statement {
        let setup_called = Arc::clone(&self.setup_called);
        let teardown_called = Arc::clone(&self.teardown_called);
        Statement::new(move || {
            setup_called.store(false, Ordering::SeqCst);
            teardown_called.store(false, Ordering::SeqCst);
            base.evaluate();
            if !setup_called.load(Ordering::SeqCst) {
                Assert::fail("One of the overrides of setUp does not propagate the call.");
            }
            if !teardown_called.load(Ordering::SeqCst) {
                Assert::fail("One of the overrides of tearDown does not propagate the call.");
            }
        })
    }
}
