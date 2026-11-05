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

//! Port of `org.apache.lucene.tests.util.FailureMarker`.

use std::sync::atomic::{AtomicI32, Ordering};

use randomizedtesting::junit::{Failure, RunListener};

/// `static final AtomicInteger failures`: the failures of every suite run in
/// this process.
static FAILURES: AtomicI32 = AtomicI32::new(0);

/// Port of `FailureMarker`: "a `RunListener` that detects suite/ test
/// failures. We need it because failures due to thread leaks happen outside
/// of any rule contexts."
#[derive(Debug, Default, Clone, Copy)]
pub struct FailureMarker;

impl FailureMarker {
    /// `FailureMarker.class.getName()`.
    pub const CLASS_NAME: &'static str = "org.apache.lucene.tests.util.FailureMarker";

    /// `hadFailures()`.
    pub fn had_failures() -> bool {
        FAILURES.load(Ordering::SeqCst) > 0
    }

    /// `getFailures()`.
    pub fn get_failures() -> i32 {
        FAILURES.load(Ordering::SeqCst)
    }

    /// `resetFailures()`.
    pub fn reset_failures() {
        FAILURES.store(0, Ordering::SeqCst);
    }
}

impl RunListener for FailureMarker {
    /// `testFailure(failure)`: `failures.incrementAndGet()`.
    fn test_failure(&self, _failure: &Failure) {
        FAILURES.fetch_add(1, Ordering::SeqCst);
    }
}
