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

//! Port of `org.apache.lucene.tests.search.AssertingBulkScorer`.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use randomizedtesting::generators::RandomNumbers;
use randomizedtesting::jdk::random::Random;
use rucene::search::{BulkScorer, CollectionResult, LeafCollector, NO_MORE_DOCS};
use rucene::util::Bits;

use crate::search::asserting_leaf_collector::AssertingLeafCollector;

/// Wraps a [`BulkScorer`] with additional checks: windows are scored forward,
/// collection stays within each window, and the returned estimate of the next
/// match is consistent. It also splits every `score` call into random windows.
///
/// Equivalent to `org.apache.lucene.tests.search.AssertingBulkScorer`.
pub struct AssertingBulkScorer {
    /// `random`.
    random: Arc<dyn Random>,
    /// `in`.
    inner: Box<dyn BulkScorer>,
    /// `maxDoc`.
    max_doc: i32,
    /// `max`: the upper bound of the previous `score` call.
    max: i32,
}

impl fmt::Debug for AssertingBulkScorer {
    /// `"AssertingBulkScorer(" + in + ")"`; Rust's [`BulkScorer`] has no
    /// string form, so the wrapped scorer prints as `<bulkScorer>`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AssertingBulkScorer(<bulkScorer>)")
    }
}

impl AssertingBulkScorer {
    /// `AssertingBulkScorer.wrap(random, other, maxDoc)`: `other` itself when
    /// it is `None` or already an `AssertingBulkScorer`.
    pub fn wrap(
        random: Arc<dyn Random>,
        other: Option<Box<dyn BulkScorer>>,
        max_doc: i32,
    ) -> Option<Box<dyn BulkScorer>> {
        let other = other?;
        if other
            .as_any()
            .is_some_and(|any| any.is::<AssertingBulkScorer>())
        {
            return Some(other);
        }
        Some(Box::new(AssertingBulkScorer {
            random,
            inner: other,
            max_doc,
            max: 0,
        }))
    }

    /// `getIn()`.
    pub fn get_in(&self) -> &dyn BulkScorer {
        &*self.inner
    }
}

impl BulkScorer for AssertingBulkScorer {
    fn score(
        &mut self,
        collector: &mut dyn LeafCollector,
        accept_docs: Option<&dyn Bits>,
        min: i32,
        max: i32,
    ) -> CollectionResult<i32> {
        java_assert!(
            min >= self.max,
            "Scoring backward: min={} while previous max was max={}",
            min,
            self.max
        );
        java_assert!(
            min <= max,
            "max must be greater than min, got min={}, and max={}",
            min,
            max
        );
        self.max = max;
        let mut collector = AssertingLeafCollector::new(Box::new(collector), min, max);
        let mut next = min;
        loop {
            let up_to = if self.random.next_boolean() {
                max
            } else {
                let interval: i64 = if self.random.next_int_bound(100) <= 5 {
                    1 + i64::from(self.random.next_int_bound(10))
                } else {
                    let bound = if self.random.next_boolean() {
                        100
                    } else {
                        5000
                    };
                    1 + i64::from(self.random.next_int_bound(bound))
                };
                // `Math.toIntExact(Math.min(next + interval, max))`: the minimum
                // with an int always fits.
                (i64::from(next) + interval).min(i64::from(max)) as i32
            };
            let mut window = AssertingLeafCollector::new(Box::new(&mut collector), next, up_to);
            next = self.inner.score(&mut window, accept_docs, next, up_to)?;
            if next >= max {
                break;
            }
        }
        if max >= self.max_doc || next >= self.max_doc {
            java_assert!(next == NO_MORE_DOCS);
            Ok(NO_MORE_DOCS)
        } else {
            Ok(RandomNumbers::random_int_between(&*self.random, max, next))
        }
    }

    fn cost(&self) -> i64 {
        self.inner.cost()
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
}
