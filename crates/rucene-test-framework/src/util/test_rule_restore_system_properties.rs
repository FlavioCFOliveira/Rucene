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

//! Port of `org.apache.lucene.tests.util.TestRuleRestoreSystemProperties`.

use std::collections::HashMap;
use std::sync::Mutex;

use randomizedtesting::jdk::exception::{self, Throwable};
use randomizedtesting::jdk::system;
use randomizedtesting::rules::TestRuleAdapter;

/// Port of `TestRuleRestoreSystemProperties`: "restore a given set of system
/// properties to a snapshot taken at the beginning of the rule."
///
/// "This is semantically similar to `SystemPropertiesRestoreRule` but the
/// list of properties to restore must be provided explicitly (because the
/// security manager prevents us from accessing the whole set of properties).
/// All properties to be restored should have r/w property permission."
///
/// Java catches a `SecurityException` per property and records it; the
/// port has no security manager, so no property write can fail.
pub struct TestRuleRestoreSystemProperties {
    property_names: Vec<String>,
    restore: Mutex<HashMap<String, Option<String>>>,
}

impl TestRuleRestoreSystemProperties {
    /// `new TestRuleRestoreSystemProperties(propertyNames...)`.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` ("No properties to restore? Odd.") for no
    /// property.
    pub fn new(property_names: &[&str]) -> TestRuleRestoreSystemProperties {
        if property_names.is_empty() {
            exception::illegal_argument("No properties to restore? Odd.");
        }
        TestRuleRestoreSystemProperties {
            property_names: property_names.iter().map(|s| (*s).to_string()).collect(),
            restore: Mutex::new(HashMap::new()),
        }
    }
}

impl TestRuleAdapter for TestRuleRestoreSystemProperties {
    /// `before()`: the snapshot of each property.
    fn before(&self) {
        let mut restore = self
            .restore
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        exception::java_assert_no_message(restore.is_empty());
        for key in &self.property_names {
            restore.insert(key.clone(), system::get_property(key));
        }
    }

    /// `afterAlways(errors)`: each property set back to its snapshot, or
    /// cleared when it had none.
    fn after_always(&self, _errors: &mut Vec<Throwable>) {
        let mut restore = self
            .restore
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for key in &self.property_names {
            match restore.get(key).cloned().flatten() {
                None => {
                    system::clear_property(key);
                }
                Some(value) => {
                    system::set_property(key, &value);
                }
            }
        }
        restore.clear();
    }
}
