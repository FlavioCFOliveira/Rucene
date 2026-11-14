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

//! Port of `SpanPositionQueue.java`, of the Java package
//! `org.apache.lucene.queries.spans`.

use rucene::error::Result;
use rucene::util::extra::{PriorityQueue, PriorityQueueComparator};

use crate::spans::spans::SharedSpans;

/// The ordering of [`SpanPositionQueue`]: its `lessThan`.
pub(crate) struct SpanPositionOrder;

impl PriorityQueueComparator<SharedSpans> for SpanPositionOrder {
    fn less_than(&self, s1: &SharedSpans, s2: &SharedSpans) -> bool {
        SpanPositionQueue::less_than(s1, s2)
    }
}

/// A priority queue of spans ordered by start position, then by end position.
///
/// Equivalent to the package-private
/// `org.apache.lucene.queries.spans.SpanPositionQueue`, which extends
/// `PriorityQueue<Spans>`.
pub(crate) struct SpanPositionQueue {
    queue: PriorityQueue<SharedSpans, SpanPositionOrder>,
}

impl SpanPositionQueue {
    /// Equivalent to `SpanPositionQueue(int)`, which does not prepopulate.
    ///
    /// # Errors
    ///
    /// Propagates the `IllegalArgumentException` of `PriorityQueue(int)` for an
    /// oversized queue.
    pub(crate) fn new(max_size: usize) -> Result<Self> {
        Ok(Self {
            queue: PriorityQueue::new(max_size, SpanPositionOrder)?,
        })
    }

    /// Equivalent to `SpanPositionQueue.lessThan(Spans, Spans)`.
    pub(crate) fn less_than(s1: &SharedSpans, s2: &SharedSpans) -> bool {
        let start1 = s1.start_position();
        let start2 = s2.start_position();
        if start1 < start2 {
            true
        } else if start1 == start2 {
            s1.end_position() < s2.end_position()
        } else {
            false
        }
    }

    /// `PriorityQueue.add(T)`.
    pub(crate) fn add(&mut self, spans: SharedSpans) {
        self.queue.add(spans);
    }

    /// `PriorityQueue.top()`.
    pub(crate) fn top(&self) -> Option<&SharedSpans> {
        self.queue.top()
    }

    /// `PriorityQueue.updateTop()`.
    pub(crate) fn update_top(&mut self) -> Option<&SharedSpans> {
        self.queue.update_top()
    }

    /// `PriorityQueue.clear()`.
    pub(crate) fn clear(&mut self) {
        self.queue.clear();
    }

    /// `PriorityQueue.size()`.
    pub(crate) fn size(&self) -> usize {
        self.queue.size()
    }
}
