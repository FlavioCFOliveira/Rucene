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

//! Port of `DisiWrapper.java`, of the Java package
//! `org.apache.lucene.queries.intervals`.

use crate::intervals::interval_iterator::{IntervalIterator, SharedIntervalIterator};

/// Wrapper used in the intervals package's `DisiPriorityQueue`.
///
/// Equivalent to the package-private class
/// `org.apache.lucene.queries.intervals.DisiWrapper`, the package's own copy,
/// distinct from the core's.
///
/// **Divergence.** Java's three fields `iterator`, `intervals` and
/// `approximation` reference the one interval iterator; here they are the one
/// handle [`intervals`](Self::intervals). The `next` field that
/// `DisiPriorityQueue.topList()` threads its list through is replaced by the
/// vector of positions that method returns, in the same order.
pub(crate) struct DisiWrapper {
    /// The interval iterator, also Java's `iterator` and `approximation`.
    pub(crate) intervals: SharedIntervalIterator,
    /// The `cost` field.
    pub(crate) cost: i64,
    /// The match cost for two-phase iterators, `0` otherwise; the `matchCost`
    /// field.
    pub(crate) match_cost: f32,
    /// The current doc, used for comparison; the `doc` field.
    pub(crate) doc: i32,
}

impl std::fmt::Debug for DisiWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DisiWrapper")
            .field("cost", &self.cost)
            .field("match_cost", &self.match_cost)
            .field("doc", &self.doc)
            .finish_non_exhaustive()
    }
}

impl DisiWrapper {
    /// Equivalent to `DisiWrapper(IntervalIterator)`.
    pub(crate) fn new(iterator: SharedIntervalIterator) -> Self {
        let cost = rucene::search::doc_id_set_iterator::DocIdSetIterator::cost(&iterator);
        let match_cost = iterator.match_cost();
        Self {
            intervals: iterator,
            cost,
            match_cost,
            doc: -1,
        }
    }
}
