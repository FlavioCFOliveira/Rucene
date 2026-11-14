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

//! Port of `DisjunctionDISIApproximation.java`, of the Java package
//! `org.apache.lucene.queries.intervals`.

use rucene::error::Result;
use rucene::search::doc_id_set_iterator::DocIdSetIterator;

use crate::intervals::disi_priority_queue::DisiPriorityQueue;

/// Message for the `NullPointerException` Java raises on an empty queue.
const QUEUE_TOP: &str = "INVARIANT: an interval disjunction holds at least one sub-iterator";

/// A [`DocIdSetIterator`] which is a disjunction of the approximations of the
/// provided iterators.
///
/// Equivalent to the class
/// `org.apache.lucene.queries.intervals.DisjunctionDISIApproximation`, the
/// package's own copy, distinct from the core's. The queue is owned here: the
/// interval iterators that Java lets share their queue with this
/// approximation reach it through [`sub_iterators`](Self::sub_iterators).
#[derive(Debug)]
pub(crate) struct DisjunctionDISIApproximation {
    /// The package-private final `subIterators` field.
    pub(crate) sub_iterators: DisiPriorityQueue,
    /// The package-private final `cost` field.
    cost: i64,
}

impl DisjunctionDISIApproximation {
    /// Equivalent to `DisjunctionDISIApproximation(DisiPriorityQueue)`.
    pub(crate) fn new(sub_iterators: DisiPriorityQueue) -> Self {
        let mut cost = 0i64;
        for w in sub_iterators.iter() {
            cost = cost.wrapping_add(w.cost);
        }
        Self {
            sub_iterators,
            cost,
        }
    }
}

impl DocIdSetIterator for DisjunctionDISIApproximation {
    fn java_to_string(&self) -> String {
        rucene::index::index_reader::java_object_to_string_stand_in(
            "org.apache.lucene.queries.intervals.DisjunctionDISIApproximation",
        )
    }

    fn cost(&self) -> i64 {
        self.cost
    }

    fn doc_id(&self) -> i32 {
        let top = self.sub_iterators.top().expect(QUEUE_TOP);
        self.sub_iterators.wrapper(top).doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        let mut top = self.sub_iterators.top().expect(QUEUE_TOP);
        let doc = self.sub_iterators.wrapper(top).doc;
        loop {
            let w = self.sub_iterators.wrapper_mut(top);
            w.doc = w.intervals.next_doc()?;
            top = self.sub_iterators.update_top().expect(QUEUE_TOP);
            if self.sub_iterators.wrapper(top).doc != doc {
                break;
            }
        }

        Ok(self.sub_iterators.wrapper(top).doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        let mut top = self.sub_iterators.top().expect(QUEUE_TOP);
        loop {
            let w = self.sub_iterators.wrapper_mut(top);
            w.doc = w.intervals.advance(target)?;
            top = self.sub_iterators.update_top().expect(QUEUE_TOP);
            if self.sub_iterators.wrapper(top).doc >= target {
                break;
            }
        }

        Ok(self.sub_iterators.wrapper(top).doc)
    }
}
