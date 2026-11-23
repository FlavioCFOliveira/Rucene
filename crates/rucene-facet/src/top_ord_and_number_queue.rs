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

//! Port of `TopOrdAndNumberQueue.java`, of the Java package
//! `org.apache.lucene.facet`.

use rucene::document::NumericValue;
use rucene::error::Result;
use rucene::util::extra::{PriorityQueue, PriorityQueueComparator};

/// Holds a single entry.
///
/// Equivalent to `org.apache.lucene.facet.TopOrdAndNumberQueue.OrdAndValue`;
/// its subclasses `OrdAndInt` and `OrdAndFloat` are this trait's
/// implementations.
pub trait OrdAndValue: Send {
    /// Ordinal of the entry.
    fn ord(&self) -> i32;

    /// Sets the ordinal of the entry.
    fn set_ord(&mut self, ord: i32);

    /// Compare with another `OrdAndValue`.
    ///
    /// Equivalent to `lessThan(OrdAndValue)`.
    fn less_than(&self, other: &dyn OrdAndValue) -> bool;

    /// Get the value stored in this `OrdAndValue`.
    ///
    /// Equivalent to `getValue()`.
    fn get_value(&self) -> NumericValue;

    /// This entry as [`std::any::Any`], for Java's casts in `lessThan`.
    fn as_any(&self) -> &dyn std::any::Any;

    /// This entry as mutable [`std::any::Any`], for Java's casts in
    /// `setIncomingValue`.
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
}

/// Orders by [`OrdAndValue::less_than`].
#[derive(Debug, Clone, Copy, Default)]
pub struct OrdAndValueLessThan;

impl PriorityQueueComparator<Box<dyn OrdAndValue>> for OrdAndValueLessThan {
    fn less_than(&self, a: &Box<dyn OrdAndValue>, b: &Box<dyn OrdAndValue>) -> bool {
        a.less_than(&**b)
    }
}

/// Keeps highest results, first by largest value, then tie-break by smallest
/// ord.
///
/// Equivalent to `org.apache.lucene.facet.TopOrdAndNumberQueue`, a
/// `PriorityQueue<OrdAndValue>`; [`new_ord_and_value`](Self::new_ord_and_value)
/// is its abstract factory.
pub struct TopOrdAndNumberQueue {
    queue: PriorityQueue<Box<dyn OrdAndValue>, OrdAndValueLessThan>,
    factory: fn() -> Box<dyn OrdAndValue>,
}

impl TopOrdAndNumberQueue {
    /// Sole constructor.
    ///
    /// Equivalent to `TopOrdAndNumberQueue(int)`.
    pub fn new(top_n: i32, factory: fn() -> Box<dyn OrdAndValue>) -> Result<Self> {
        Ok(Self {
            queue: PriorityQueue::new(top_n.max(0) as usize, OrdAndValueLessThan)?,
            factory,
        })
    }

    /// Create a new `OrdAndValue` of the appropriate type.
    ///
    /// Equivalent to `newOrdAndValue()`.
    pub fn new_ord_and_value(&self) -> Box<dyn OrdAndValue> {
        (self.factory)()
    }

    /// Equivalent to `PriorityQueue.insertWithOverflow(T)`.
    pub fn insert_with_overflow(
        &mut self,
        element: Box<dyn OrdAndValue>,
    ) -> Option<Box<dyn OrdAndValue>> {
        self.queue.insert_with_overflow(element)
    }

    /// Equivalent to `PriorityQueue.add(T)`.
    pub fn add(&mut self, element: Box<dyn OrdAndValue>) {
        self.queue.add(element);
    }

    /// Equivalent to `PriorityQueue.pop()`.
    pub fn pop(&mut self) -> Option<Box<dyn OrdAndValue>> {
        self.queue.pop()
    }

    /// Equivalent to `PriorityQueue.top()`.
    pub fn top(&self) -> Option<&dyn OrdAndValue> {
        self.queue.top().map(|b| &**b)
    }

    /// Equivalent to `PriorityQueue.size()`.
    pub fn size(&self) -> usize {
        self.queue.size()
    }
}

/// Equivalent to `TopOrdAndNumberQueue.lessThan(OrdAndValue, OrdAndValue)`.
pub fn less_than(a: &dyn OrdAndValue, b: &dyn OrdAndValue) -> bool {
    a.less_than(b)
}
