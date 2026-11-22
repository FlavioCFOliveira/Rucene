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

//! Port of `TopOrdAndFloatQueue.java`, of the Java package
//! `org.apache.lucene.facet`.

use std::ops::{Deref, DerefMut};

use rucene::document::NumericValue;
use rucene::error::Result;

use crate::top_ord_and_number_queue::{OrdAndValue, TopOrdAndNumberQueue};

/// Keeps highest results, first by largest float value, then tie-break by
/// smallest ord.
///
/// Equivalent to `org.apache.lucene.facet.TopOrdAndFloatQueue`.
pub struct TopOrdAndFloatQueue {
    inner: TopOrdAndNumberQueue,
}

impl TopOrdAndFloatQueue {
    /// Sole constructor.
    ///
    /// Equivalent to `TopOrdAndFloatQueue(int)`.
    pub fn new(top_n: i32) -> Result<Self> {
        Ok(Self {
            inner: TopOrdAndNumberQueue::new(top_n, || Box::new(OrdAndFloat::new()))?,
        })
    }

    /// Unwraps the base queue.
    pub fn into_inner(self) -> TopOrdAndNumberQueue {
        self.inner
    }
}

impl Deref for TopOrdAndFloatQueue {
    type Target = TopOrdAndNumberQueue;

    fn deref(&self) -> &TopOrdAndNumberQueue {
        &self.inner
    }
}

impl DerefMut for TopOrdAndFloatQueue {
    fn deref_mut(&mut self) -> &mut TopOrdAndNumberQueue {
        &mut self.inner
    }
}

/// Holds a single entry.
///
/// Equivalent to `org.apache.lucene.facet.TopOrdAndFloatQueue.OrdAndFloat`.
#[derive(Debug, Clone, Default)]
pub struct OrdAndFloat {
    /// Ordinal of the entry.
    pub ord: i32,
    /// The value corresponding to the ord.
    pub value: f32,
}

impl OrdAndFloat {
    /// Default constructor.
    ///
    /// Equivalent to `OrdAndFloat()`.
    pub fn new() -> Self {
        Self::default()
    }
}

impl OrdAndValue for OrdAndFloat {
    fn ord(&self) -> i32 {
        self.ord
    }

    fn set_ord(&mut self, ord: i32) {
        self.ord = ord;
    }

    fn less_than(&self, other: &dyn OrdAndValue) -> bool {
        let other = other
            .as_any()
            .downcast_ref::<OrdAndFloat>()
            .expect("ClassCastException: not an OrdAndFloat");
        if self.value < other.value {
            return true;
        }
        if self.value > other.value {
            return false;
        }
        self.ord > other.ord
    }

    fn get_value(&self) -> NumericValue {
        NumericValue::Float(self.value)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}