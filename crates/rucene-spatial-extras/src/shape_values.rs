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

//! The types declared in `ShapeValues.java`, of the Java package
//! `org.apache.lucene.spatial`.

use std::sync::Arc;

use rucene::error::Result;
use spatial4j::shape::shape::Shape;

/// Equivalent to `org.apache.lucene.spatial.ShapeValues` (`public abstract
/// class`): iterator over `Shape` objects for an index segment.
pub trait ShapeValues {
    /// `advanceExact(int doc)`: advance the iterator to the given document.
    /// Returns `true` if there is a value for this document.
    ///
    /// # Errors
    ///
    /// The I/O errors of the underlying values.
    fn advance_exact(&mut self, doc: i32) -> Result<bool>;

    /// `value()`: the `Shape` for the current document.
    ///
    /// # Errors
    ///
    /// The I/O errors of the underlying values.
    fn value(&mut self) -> Result<Arc<dyn Shape>>;
}
