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

//! Port of `org.apache.lucene.tests.util.TestRuleAssertionsRequired`.

use std::sync::Arc;

use randomizedtesting::jdk::exception::{self, Throwable};
use randomizedtesting::jdk::system;
use randomizedtesting::junit::{Description, Statement, TestRule};

use crate::util::lucene_test_case::{LuceneTestCase, TEST_ASSERTS_ENABLED};

/// Port of `TestRuleAssertionsRequired`: "require assertions for
/// Lucene/Solr packages."
#[derive(Debug, Default, Clone, Copy)]
pub struct TestRuleAssertionsRequired;

impl TestRuleAssertionsRequired {
    /// The check of `evaluate()` for the given state of `-ea`
    /// (`assertsAreEnabled`) and `-Dtests.asserts`: "make sure -ea matches
    /// -Dtests.asserts, to catch accidental mis-use". `Some(msg)` is the
    /// mismatch Java prints to `System.err` and throws as an `Exception`.
    pub fn mismatch(asserts_are_enabled: bool, test_asserts_enabled: bool) -> Option<String> {
        if asserts_are_enabled == test_asserts_enabled {
            return None;
        }
        let mut msg = String::from("Assertions mismatch: ");
        if asserts_are_enabled {
            msg.push_str("-ea was specified");
        } else {
            msg.push_str("-ea was not specified");
        }
        if test_asserts_enabled {
            msg.push_str(" but -Dtests.asserts=true");
        } else {
            msg.push_str(" but -Dtests.asserts=false");
        }
        Some(msg)
    }
}

impl TestRule for TestRuleAssertionsRequired {
    /// `apply(base, description)`.
    ///
    /// Java's `catch (AssertionError e)` ("Ok, enabled.") catches nothing the
    /// check itself throws (it throws an `Exception`); it is not reproduced.
    fn apply(&self, base: Statement, _description: &Arc<Description>) -> Statement {
        Statement::new(move || {
            if let Some(msg) =
                Self::mismatch(LuceneTestCase::ASSERTS_ARE_ENABLED, *TEST_ASSERTS_ENABLED)
            {
                system::err_println(&msg);
                Throwable::new(exception::EXCEPTION, msg).throw();
            }
            base.evaluate();
        })
    }
}
