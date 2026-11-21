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

//! Port of `org.apache.lucene.tests.util.TestRuleSetupAndRestoreInstanceEnv`.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::junit::Description;
use rucene::search::IndexSearcher;

use crate::util::abstract_before_after_rule::AbstractBeforeAfterRule;

/// Port of `TestRuleSetupAndRestoreInstanceEnv`: "prepares and restores
/// `LuceneTestCase` at instance level (fine grained junk that doesn't fit
/// anywhere else)."
#[derive(Debug, Default)]
pub struct TestRuleSetupAndRestoreInstanceEnv {
    saved_bool_max_clause_count: AtomicI32,
}

impl AbstractBeforeAfterRule for TestRuleSetupAndRestoreInstanceEnv {
    /// `before()`: saves `IndexSearcher.getMaxClauseCount()`.
    fn before(&self, _description: &Arc<Description>) {
        self.saved_bool_max_clause_count
            .store(IndexSearcher::get_max_clause_count(), Ordering::SeqCst);
    }

    /// `after()`: `IndexSearcher.setMaxClauseCount(saved)`.
    fn after(&self) {
        let saved = self.saved_bool_max_clause_count.load(Ordering::SeqCst);
        if let Err(e) = IndexSearcher::set_max_clause_count(saved) {
            // Java's `IllegalArgumentException`, for a count the setter rejects.
            randomizedtesting::jdk::exception::illegal_argument(e)
        }
    }
}
