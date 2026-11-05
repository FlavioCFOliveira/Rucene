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

//! Port of `org.apache.lucene.tests.search.QueryUtils`.
//!
//! The JUnit assertions of Java (`assertEquals`, `assertFalse`, …) fail by
//! throwing an unchecked `AssertionError`; here they panic with that
//! `Throwable`, as [`randomizedtesting::junit::Assert`] does.

use std::any::Any;
use std::sync::Arc;

use randomizedtesting::junit::Assert;
use rucene::search::{query_to_string, Query, QueryVisitor};

/// Utility methods for checking queries.
///
/// Equivalent to `org.apache.lucene.tests.search.QueryUtils`.
pub struct QueryUtils;

/// The anonymous "whacky" query of `checkHashEquals`: equal only to itself.
#[derive(Debug)]
struct WhackyQuery;

impl Query for WhackyQuery {
    fn to_query_string(&self, _field: &str) -> String {
        "My Whacky Query".to_string()
    }

    fn visit(&self, _visitor: &mut dyn QueryVisitor) {}

    fn as_any(&self) -> &dyn Any {
        self
    }

    /// `o == this`.
    fn query_eq(&self, other: &dyn Query) -> bool {
        std::ptr::addr_eq(self as *const Self, other as *const dyn Query)
    }

    /// `System.identityHashCode(this)`.
    fn query_hash(&self) -> u64 {
        self as *const Self as usize as u64
    }
}

impl QueryUtils {
    /// `check(q)`: checks the basic contract of `hashCode`/`equals`.
    ///
    /// # Panics
    ///
    /// With Java's `AssertionError` when the contract does not hold.
    pub fn check(q: &dyn Query) {
        Self::check_hash_equals(q);
    }

    /// `checkHashEquals(q)`: `q` equals itself with the same hash, and an
    /// unrelated query compares unequal both ways without failing.
    ///
    /// **Divergence.** Java also asserts `q.equals(null)` is false; a
    /// `&dyn Query` cannot be null, so that check has no counterpart.
    ///
    /// # Panics
    ///
    /// With Java's `AssertionError` when the contract does not hold.
    pub fn check_hash_equals(q: &dyn Query) {
        Self::check_equal(q, q);
        // test that a class check is done so that no exception is thrown in
        // the implementation of equals()
        let whacky: Arc<dyn Query> = Arc::new(WhackyQuery);
        Self::check_unequal(q, &*whacky);
    }

    /// `checkEqual(q1, q2)`: `assertEquals(q1, q2)` and equal hash codes.
    ///
    /// # Panics
    ///
    /// With Java's `AssertionError` (`expected:<q1> but was:<q2>`).
    pub fn check_equal(q1: &dyn Query, q2: &dyn Query) {
        if !q1.query_eq(q2) {
            Assert::fail(&format!(
                "expected:<{}> but was:<{}>",
                query_to_string(q1),
                query_to_string(q2)
            ));
        }
        Assert::assert_equals(q1.query_hash(), q2.query_hash());
    }

    /// `checkUnequal(q1, q2)`: neither query equals the other.
    ///
    /// # Panics
    ///
    /// With Java's `AssertionError` (`<q1> equal to <q2>`).
    pub fn check_unequal(q1: &dyn Query, q2: &dyn Query) {
        Assert::assert_true_msg(
            &format!("{} equal to {}", query_to_string(q1), query_to_string(q2)),
            !q1.query_eq(q2),
        );
        Assert::assert_true_msg(
            &format!("{} equal to {}", query_to_string(q2), query_to_string(q1)),
            !q2.query_eq(q1),
        );
    }
}
