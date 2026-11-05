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

//! Port of `org.apache.lucene.tests.util.CloseableDirectory`: attempts to
//! close a [`BaseDirectoryWrapper`].

use std::sync::Arc;

use crate::store::BaseDirectoryWrapper;
use crate::util::TestRuleMarkFailure;

/// Port of `org.apache.lucene.tests.util.CloseableDirectory` (package-private
/// in Java).
pub struct CloseableDirectory {
    dir: Arc<BaseDirectoryWrapper>,
    failure_marker: Arc<TestRuleMarkFailure>,
}

impl CloseableDirectory {
    /// `new CloseableDirectory(dir, failureMarker)`.
    pub fn new(dir: Arc<BaseDirectoryWrapper>, failure_marker: Arc<TestRuleMarkFailure>) -> Self {
        CloseableDirectory {
            dir,
            failure_marker,
        }
    }

    /// `close()`: fails the test (`Directory not closed: …`) when no other
    /// test failure happened and the directory is still open. The delegate
    /// is not closed (LUCENE-4058).
    ///
    /// # Panics
    ///
    /// The `AssertionError` of `Assert.fail`.
    pub fn close(&self) {
        // We only attempt to check open/closed state if there were no other test
        // failures.
        if self.failure_marker.was_successful() && self.dir.is_open() {
            randomizedtesting::junit::Assert::fail(&format!("Directory not closed: {}", self.dir));
        }
        // TODO: perform real close of the delegate: LUCENE-4058
        // dir.close();
    }
}
