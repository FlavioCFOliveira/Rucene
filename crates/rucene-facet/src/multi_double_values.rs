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

//! Port of `MultiDoubleValues.java`, of the Java package
//! `org.apache.lucene.facet`.

use rucene::error::Result;

/// Per-segment, per-document double values, which can be calculated at
/// search-time. Documents may produce multiple values.
///
/// Equivalent to `org.apache.lucene.facet.MultiDoubleValues`.
pub trait MultiDoubleValues {
    /// Retrieves the number of values for the current document. This must
    /// always be greater than zero.
    ///
    /// Equivalent to `getValueCount()`.
    fn get_value_count(&self) -> Result<i64>;

    /// Iterates to the next value in the current document.
    ///
    /// Equivalent to `nextValue()`.
    fn next_value(&mut self) -> Result<f64>;

    /// Advances to exactly `doc` and returns true if `doc` has a value.
    ///
    /// Equivalent to `advanceExact(int)`.
    fn advance_exact(&mut self, doc: i32) -> Result<bool>;
}
