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

//! Port of `org.apache.lucene.tests.util.TestRuleIgnoreTestSuites`.

use std::sync::Arc;

use randomizedtesting::jdk::exception;
use randomizedtesting::jdk::system;
use randomizedtesting::junit::{Description, Statement, TestRule};

use crate::util::lucene_test_case::LuceneTestCase;

/// The name of the marker interface `TestRuleIgnoreTestSuites.NestedTestSuite`:
/// "marker interface for nested suites that should be ignored if executed in
/// stand-alone mode." A suite implements it by declaring the marker
/// annotation [`NESTED_TEST_SUITE`].
pub const NESTED_TEST_SUITE_NAME: &str =
    "org.apache.lucene.tests.util.TestRuleIgnoreTestSuites$NestedTestSuite";

/// `NestedTestSuite`, the interface a class implements, as an inherited
/// marker annotation of the declared class (a declared class has no
/// interfaces): `NestedTestSuite.class.isAssignableFrom(c)` is
/// `c.isAnnotationPresent(NESTED_TEST_SUITE)`.
pub static NESTED_TEST_SUITE: randomizedtesting::annotations::AnnotationType =
    randomizedtesting::annotations::AnnotationType::plain(
        NESTED_TEST_SUITE_NAME,
        "NestedTestSuite",
        true,
    );

/// Port of `TestRuleIgnoreTestSuites`: "this rule will cause the suite to be
/// assumption-ignored if the test class implements a given marker interface
/// and a special property is not set."
#[derive(Debug, Default, Clone, Copy)]
pub struct TestRuleIgnoreTestSuites;

impl TestRuleIgnoreTestSuites {
    /// `PROPERTY_RUN_NESTED`: "a boolean system property indicating nested
    /// suites should be executed normally."
    pub const PROPERTY_RUN_NESTED: &'static str = "tests.runnested";

    /// `isRunningNested()`: `Boolean.getBoolean(PROPERTY_RUN_NESTED)`, true
    /// only for the property's value `true`, ignoring case.
    pub fn is_running_nested() -> bool {
        system::get_property(Self::PROPERTY_RUN_NESTED)
            .is_some_and(|v| v.eq_ignore_ascii_case("true"))
    }
}

impl TestRule for TestRuleIgnoreTestSuites {
    /// `apply(s, d)`.
    fn apply(&self, s: Statement, d: &Arc<Description>) -> Statement {
        let d = Arc::clone(d);
        Statement::new(move || {
            let Some(class) = d.test_class() else {
                exception::throw_without_message(exception::NULL_POINTER_EXCEPTION)
            };
            if class.is_annotation_present(&NESTED_TEST_SUITE) {
                LuceneTestCase::assume_true(
                    "Nested suite class ignored (started as stand-alone).",
                    TestRuleIgnoreTestSuites::is_running_nested(),
                );
            }
            s.evaluate();
        })
    }
}
